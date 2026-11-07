-- WoW Chat Translator: pixel strip.
-- Each incoming chat message is sent to the companion app as a small grid of colored
-- cells at the top-left of the screen, which the app decodes exactly (no OCR).
-- The format must match app/src/core/strip.js (see that file for the layout); its
-- design is informed by Parley's documented protocol (MIT) and LibSerpix (MIT).
-- Besides player chat it also sends, at lower priority, the rest of the chat (loot,
-- money, system, XP, reputation, achievements, NPC speech, emotes); the app shows those
-- only when its "Registro del chat" option is on.

local ADDON = ...

local DEFAULTS = {
  shown = true,
  cell = 2, -- physical pixels per cell side
  hold = 0.1, -- seconds each message stays on screen
  skipOwn = true,
}

local COLS, ROWS = 64, 6
local CELLS = COLS * ROWS
local MARKER = { { 15, 0, 8 }, { 0, 15, 15 }, { 15, 8, 0 }, { 3, 0, 15 } }
local CAL, SEQ, LEN, SUM, DATA = 4, 20, 21, 22, 24
local MAX_PAYLOAD = math.floor((CELLS - DATA) * 3 / 2)
local SEP = "\31" -- unit separator
local VERSION = "v1"

local EVENT_CHANNEL = {
  CHAT_MSG_SAY = "Say",
  CHAT_MSG_YELL = "Yell",
  CHAT_MSG_WHISPER = "Whisper",
  CHAT_MSG_PARTY = "Party",
  CHAT_MSG_PARTY_LEADER = "Party",
  CHAT_MSG_RAID = "Raid",
  CHAT_MSG_RAID_LEADER = "Raid",
  CHAT_MSG_RAID_WARNING = "Raid",
  CHAT_MSG_GUILD = "Guild",
  CHAT_MSG_OFFICER = "Officer",
  CHAT_MSG_INSTANCE_CHAT = "Instance",
  CHAT_MSG_INSTANCE_CHAT_LEADER = "Instance",
  CHAT_MSG_CHANNEL = false, -- resolved from the channel name
}

-- Non-player chat (the app's optional "Registro" tab). Sent on a low-priority queue,
-- never translated; the sender may be empty and the player's own lines are kept.
local LOG_CHANNEL = {
  CHAT_MSG_LOOT = "Loot",
  CHAT_MSG_MONEY = "Money",
  CHAT_MSG_CURRENCY = "Currency",
  CHAT_MSG_SYSTEM = "System",
  CHAT_MSG_COMBAT_XP_GAIN = "XP",
  CHAT_MSG_COMBAT_FACTION_CHANGE = "Reputation",
  CHAT_MSG_SKILL = "Skill",
  CHAT_MSG_ACHIEVEMENT = "Achievement",
  CHAT_MSG_GUILD_ACHIEVEMENT = "Achievement",
  CHAT_MSG_MONSTER_SAY = "NPC",
  CHAT_MSG_MONSTER_YELL = "NPC",
  CHAT_MSG_MONSTER_WHISPER = "NPC",
  CHAT_MSG_MONSTER_EMOTE = "NPC",
  CHAT_MSG_RAID_BOSS_EMOTE = "BossEmote",
  CHAT_MSG_RAID_BOSS_WHISPER = "BossEmote",
  CHAT_MSG_EMOTE = "Emote",
  CHAT_MSG_TEXT_EMOTE = "Emote",
}
-- Log events whose text does not name the speaker: the sender goes in the player field.
local LOG_SPEAKER = {
  CHAT_MSG_MONSTER_SAY = true,
  CHAT_MSG_MONSTER_YELL = true,
  CHAT_MSG_MONSTER_WHISPER = true,
  CHAT_MSG_RAID_BOSS_WHISPER = true,
  CHAT_MSG_EMOTE = true,
}
local MAX_LOG_QUEUE = 200 -- a raid burst drops the oldest log lines, never chat

local db

-- Text cleanup ----------------------------------------------------------------
-- Removes color codes, textures and raid icons. Real links (|H...|h[Name]|h) are
-- wrapped in \2 ... \3, characters players cannot type, so the app keeps only real
-- link names untranslated; brackets a player types by hand are ordinary text.
-- A link with a numeric id also carries its type and that id after \4, as in
-- \2[Arcanite Bar]\4item:12360\3 (never shown by the app; used for item data).
local LINK_OPEN, LINK_REF, LINK_CLOSE = "\2", "\4", "\3"
local function markLink(link, name)
  local kind, id = link:match("^(%a+):(%d+)")
  if kind then
    return LINK_OPEN .. name .. LINK_REF .. kind .. ":" .. id .. LINK_CLOSE
  end
  return LINK_OPEN .. name .. LINK_CLOSE
end

local function sanitize(text)
  text = text:gsub("[\2\3\4]", "")
  text = text:gsub("|c%x%x%x%x%x%x%x%x", "")
  text = text:gsub("|cn[^:]*:", "") -- named colors, e.g. item quality "|cnIQ1:"
  text = text:gsub("|r", "")
  text = text:gsub("|H(.-)|h(.-)|h", markLink)
  text = text:gsub("|T.-|t", "")
  text = text:gsub("|A.-|a", "")
  text = text:gsub("{.-}", "")
  text = text:gsub("|", "/")
  text = text:gsub(SEP, " ")
  text = text:gsub("%s+", " ")
  return strtrim(text)
end

local function shortName(name)
  if Ambiguate then
    return Ambiguate(name, "short")
  end
  return (name:gsub("%-.*$", ""))
end

local function channelLabel(event, channelBaseName)
  local label = EVENT_CHANNEL[event]
  if label then
    return label
  end
  local base = channelBaseName or "Channel"
  base = base:gsub("%s+%-.*$", "") -- "Trade - City" -> "Trade"
  base = base:gsub("%s+", "")
  return base ~= "" and base or "Channel"
end

local function classOf(guid)
  if type(guid) ~= "string" or guid == "" or not GetPlayerInfoByGUID then
    return ""
  end
  local ok, _, englishClass = pcall(GetPlayerInfoByGUID, guid)
  if ok and type(englishClass) == "string" then
    return englishClass
  end
  return ""
end

-- Encoding ---------------------------------------------------------------------

-- Cuts a byte string to max bytes without leaving a broken UTF-8 sequence at the end.
local function cutUtf8(s, max)
  if #s <= max then
    return s
  end
  s = s:sub(1, max)
  for i = #s, math.max(1, #s - 3), -1 do
    local b = s:byte(i)
    if b < 128 then
      break
    elseif b >= 192 then
      local need = (b >= 240 and 4) or (b >= 224 and 3) or 2
      if #s - i + 1 < need then
        s = s:sub(1, i - 1)
      end
      break
    end
  end
  return s
end

local function to12(v)
  return { math.floor(v / 256) % 16, math.floor(v / 16) % 16, v % 16 }
end

local function encodeFrame(seq, payload)
  local cells = {}
  for i = 0, CELLS - 1 do
    cells[i] = { 0, 0, 0 }
  end
  for i = 1, #MARKER do
    cells[i - 1] = MARKER[i]
  end
  for i = 0, 15 do
    cells[CAL + i] = { i, i, i }
  end
  cells[SEQ] = to12(seq % 4096)
  cells[LEN] = to12(#payload)
  local s1, s2 = 0, 0
  for i = 1, #payload do
    s1 = (s1 + payload:byte(i)) % 4095
    s2 = (s2 + s1) % 4095
  end
  cells[SUM] = to12(s1)
  cells[SUM + 1] = to12(s2)
  local nibbles = {}
  for i = 1, #payload do
    local b = payload:byte(i)
    nibbles[#nibbles + 1] = math.floor(b / 16)
    nibbles[#nibbles + 1] = b % 16
  end
  for i = 1, #nibbles, 3 do
    cells[DATA + (i - 1) / 3] = { nibbles[i], nibbles[i + 1] or 0, nibbles[i + 2] or 0 }
  end
  return cells
end

-- Strip ------------------------------------------------------------------------

local strip = CreateFrame("Frame", "WoWChatTranslatorStrip", UIParent)
strip:SetFrameStrata("TOOLTIP")
-- Size the cells in physical pixels, whatever the UI scale is.
if strip.SetIgnoreParentScale then
  strip:SetIgnoreParentScale(true)
end
strip:SetScale(1)

local textures = {}
for i = 0, CELLS - 1 do
  local t = strip:CreateTexture(nil, "ARTWORK")
  if t.SetSnapToPixelGrid then
    t:SetSnapToPixelGrid(false)
    t:SetTexelSnappingBias(0)
  end
  textures[i] = t
end

local function layout()
  local _, physicalHeight = GetPhysicalScreenSize()
  local unit = 768 / physicalHeight -- one physical pixel in UI units at scale 1
  local size = db.cell * unit
  strip:ClearAllPoints()
  strip:SetPoint("TOPLEFT", UIParent, "TOPLEFT", 0, 0)
  strip:SetSize(COLS * size, ROWS * size)
  for i = 0, CELLS - 1 do
    local t = textures[i]
    t:ClearAllPoints()
    t:SetPoint("TOPLEFT", strip, "TOPLEFT", (i % COLS) * size, -math.floor(i / COLS) * size)
    t:SetSize(size, size)
  end
  strip:SetShown(db.shown)
end

local function draw(cells)
  for i = 0, CELLS - 1 do
    local c = cells[i]
    textures[i]:SetColorTexture(c[1] / 15, c[2] / 15, c[3] / 15, 1)
  end
end

-- Queue: each message is drawn for db.hold seconds, then the next one. With nothing
-- queued the last frame stays (the app ignores a sequence number it already read).
-- An empty payload is the idle frame: it tells the app the addon is running.
-- Player chat goes first: the log queue is only drawn while the chat queue is empty,
-- so a log line delays chat by one hold at most.
local queue = {}
local logQueue = {}
local seq = math.random(0, 4095)
local shownAt = 0

local function showPayload(payload)
  seq = (seq + 1) % 4096
  draw(encodeFrame(seq, payload))
  shownAt = GetTime()
end

local function showNext(from)
  showPayload(table.remove(from, 1))
end

-- Item tooltips ------------------------------------------------------------------
-- Each item linked in a queued message (chat or log) gets ONE tooltip frame per game
-- session, also at low priority: channel "*Item", empty class, player = the item id,
-- text = "q<quality>\n" (when the game knows it) and then the game's tooltip lines,
-- joined with "\n"; a right column follows its line after "\t". The first line is the
-- item name. The tooltip is read from a hidden GameTooltip with the full link of the
-- message (bonus ids kept). Items not cached yet are retried a few times.
local ITEM_CHANNEL = "*Item"
local MAX_TIP_QUEUE = 50
local MAX_TIP_TRIES = 5
local TIP_RETRY = 1 -- seconds between tries of an item not cached yet
local tipPending = {} -- { link, id, tries, at }, oldest first
local tipSeen = {} -- item id -> true once queued this session
local getItemInfo = (C_Item and C_Item.GetItemInfo) or GetItemInfo
local scanner

-- Tooltip text without colors, textures or control characters (\n, \t, SEP...).
local function tipText(s)
  s = s:gsub("|c%x%x%x%x%x%x%x%x", "")
  s = s:gsub("|cn[^:]*:", "")
  s = s:gsub("|r", "")
  s = s:gsub("|T.-|t", "")
  s = s:gsub("|A.-|a", "")
  s = s:gsub("%c", " ")
  s = s:gsub("%s+", " ")
  return strtrim(s)
end

local function noteItems(message)
  for link, digits in message:gmatch("|H(item:(%d+)[^|]*)|h") do
    local id = tonumber(digits)
    if id and not tipSeen[id] then
      tipSeen[id] = true
      tipPending[#tipPending + 1] = { link = link, id = id, tries = 0, at = 0 }
      if #tipPending > MAX_TIP_QUEUE then
        tipSeen[table.remove(tipPending, 1).id] = nil -- may be queued again later
      end
    end
  end
end

-- The tooltip lines of a link, or nil when the game has none (yet).
local function scanTooltip(link)
  if not scanner then
    scanner = CreateFrame("GameTooltip", "WCTScanTooltip", nil, "GameTooltipTemplate")
  end
  scanner:SetOwner(WorldFrame, "ANCHOR_NONE")
  scanner:ClearLines()
  if not pcall(scanner.SetHyperlink, scanner, link) then
    scanner:Hide()
    return nil
  end
  local lines = {}
  for i = 1, scanner:NumLines() do
    local left = _G["WCTScanTooltipTextLeft" .. i]
    local right = _G["WCTScanTooltipTextRight" .. i]
    local l = left and left:GetText()
    local r = right and right:IsShown() and right:GetText()
    if issecretvalue and ((l and issecretvalue(l)) or (r and issecretvalue(r))) then
      scanner:Hide()
      return nil
    end
    l = type(l) == "string" and tipText(l) or ""
    r = type(r) == "string" and tipText(r) or ""
    if r ~= "" then
      lines[#lines + 1] = l .. "\t" .. r
    elseif l ~= "" then
      lines[#lines + 1] = l
    end
  end
  scanner:Hide()
  if #lines == 0 or (RETRIEVING_ITEM_INFO and lines[1] == RETRIEVING_ITEM_INFO) then
    return nil
  end
  return table.concat(lines, "\n")
end

local function tipPayload(t)
  local quality
  if getItemInfo then
    local ok, name, _, q = pcall(getItemInfo, t.link)
    if not ok or type(name) ~= "string" or (issecretvalue and issecretvalue(name)) then
      if C_Item and C_Item.RequestLoadItemDataByID then
        pcall(C_Item.RequestLoadItemDataByID, t.id)
      end
      return nil
    end
    quality = q
  end
  local text = scanTooltip(t.link)
  if not text then
    return nil
  end
  if type(quality) == "number" and quality >= 0 and quality <= 8 then
    text = "q" .. math.floor(quality) .. "\n" .. text
  end
  return cutUtf8(table.concat({ VERSION, ITEM_CHANNEL, "", tostring(t.id), text }, SEP), MAX_PAYLOAD)
end

-- The next ready tooltip payload; scans one item at most per call.
local function nextTip(now)
  for _ = 1, #tipPending do
    local t = table.remove(tipPending, 1)
    if t.at <= now then
      local payload = tipPayload(t)
      if payload then
        return payload
      end
      t.tries = t.tries + 1
      if t.tries < MAX_TIP_TRIES then
        t.at = now + TIP_RETRY
        tipPending[#tipPending + 1] = t
      else
        tipSeen[t.id] = nil -- given up; a later link may try again
      end
      return nil
    end
    tipPending[#tipPending + 1] = t
  end
  return nil
end

-- Chat first; then log lines and tooltips take turns.
local tipTurn = false
strip:SetScript("OnUpdate", function()
  if not db then
    return -- settings load on ADDON_LOADED
  end
  local now = GetTime()
  if now - shownAt < db.hold then
    return
  end
  if #queue > 0 then
    showNext(queue)
    return
  end
  tipTurn = not tipTurn
  if (tipTurn or #logQueue == 0) and #tipPending > 0 then
    local payload = nextTip(now)
    if payload then
      showPayload(payload)
      return
    end
  end
  if #logQueue > 0 then
    showNext(logQueue)
  end
end)

local function payloadOf(channel, class, player, message)
  local clean = sanitize(message)
  if clean == "" then
    return nil
  end
  local payload = table.concat({ VERSION, channel, class, player, clean }, SEP)
  return cutUtf8(payload, MAX_PAYLOAD)
end

local function push(channel, class, player, message)
  local payload = payloadOf(channel, class, player, message)
  if payload then
    queue[#queue + 1] = payload
    noteItems(message)
  end
end

-- Log labels go with a "*" prefix ("*Loot"): channel names cannot contain it, so the
-- app never mistakes a player channel called "Loot" for the log.
local function pushLog(channel, player, message)
  local payload = payloadOf("*" .. channel, "", player, message)
  if not payload then
    return
  end
  logQueue[#logQueue + 1] = payload
  noteItems(message)
  if #logQueue > MAX_LOG_QUEUE then
    table.remove(logQueue, 1)
  end
end

-- Events -------------------------------------------------------------------------

local events = CreateFrame("Frame")
events:RegisterEvent("ADDON_LOADED")
events:RegisterEvent("DISPLAY_SIZE_CHANGED")
events:RegisterEvent("UI_SCALE_CHANGED")

events:SetScript("OnEvent", function(_, event, ...)
  if event == "ADDON_LOADED" then
    if ... ~= ADDON then
      return
    end
    WCT_DB = WCT_DB or {}
    for k, v in pairs(DEFAULTS) do
      if WCT_DB[k] == nil then
        WCT_DB[k] = v
      end
    end
    db = WCT_DB
    layout()
    queue[1] = "" -- idle frame
    showNext(queue)
    -- Some events do not exist on every client; ignore those.
    for chatEvent in pairs(EVENT_CHANNEL) do
      pcall(events.RegisterEvent, events, chatEvent)
    end
    for logEvent in pairs(LOG_CHANNEL) do
      pcall(events.RegisterEvent, events, logEvent)
    end
    return
  end
  if event == "DISPLAY_SIZE_CHANGED" or event == "UI_SCALE_CHANGED" then
    if db then
      layout()
    end
    return
  end

  local text, sender = ...
  if issecretvalue and (issecretvalue(text) or issecretvalue(sender)) then
    return
  end
  local logChannel = LOG_CHANNEL[event]
  if logChannel then
    if type(text) ~= "string" then
      return
    end
    local name = (type(sender) == "string" and sender ~= "") and shortName(sender) or ""
    if text:find("%s", 1, true) then
      -- NPC/boss emotes: "%s goes into a frenzy!" names the speaker in the text.
      text = text:gsub("%%s", (name:gsub("%%", "%%%%")))
      name = ""
    elseif not LOG_SPEAKER[event] then
      name = "" -- loot, system... already name whoever it is about
    end
    pushLog(logChannel, name, text)
    return
  end
  if type(text) ~= "string" or type(sender) ~= "string" or sender == "" then
    return
  end
  local player = shortName(sender)
  if db.skipOwn and player == UnitName("player") then
    return
  end
  local channelBaseName = select(9, ...)
  local guid = select(12, ...)
  push(channelLabel(event, channelBaseName), classOf(guid), player, text)
end)

-- Slash commands -------------------------------------------------------------------

local SAMPLES = {
  { "Trade", "MAGE", "Kaelthas", "WTS |cffa335ee|Hitem:12360::::::::|h[Arcanite Bar]|h|r 40g pst" },
  { "General", "DRUID", "Lunafae", "alguien para Deadmines?" },
  { "Say", "WARRIOR", "Brunhilde", "anyone know where the trainer is?" },
  { "Guild", "SHAMAN", "Pedrinho", "bora fazer raid hoje à noite" },
  { "Party", "WARLOCK", "Schatten", "ich brauche noch Mana, kurz warten {rt1}" },
  { "General", "", "Volkov", "lol LFM ZG need heal" },
}

local LOG_SAMPLES = {
  { "Loot", "", "You receive loot: |cff9d9d9d|Hitem:2589::::::::|h[Linen Cloth]|h|rx2." },
  { "Money", "", "You loot 1 Silver, 25 Copper" },
  { "Reputation", "", "Reputation with Stormwind increased by 25." },
  { "NPC", "Stormwind Guard", "Stay alert, citizen!" },
  { "System", "", "You are no longer Away." },
}

local function say(msg)
  DEFAULT_CHAT_FRAME:AddMessage("|cff33ff99WCT|r " .. msg)
end

SLASH_WOWCHATTRANSLATOR1 = "/wct"
SlashCmdList.WOWCHATTRANSLATOR = function(input)
  local cmd, arg = strsplit(" ", strtrim(input or ""):lower(), 2)
  if cmd == "" then
    db.shown = not db.shown
    say(db.shown and "strip shown" or "strip hidden (the app pauses)")
  elseif cmd == "cell" and tonumber(arg) then
    db.cell = math.max(1, math.min(4, math.floor(tonumber(arg))))
    say("cell size: " .. db.cell .. " px")
  elseif cmd == "hold" and tonumber(arg) then
    db.hold = math.max(0.05, math.min(1, tonumber(arg)))
    say("hold: " .. db.hold .. " s")
  elseif cmd == "reset" then
    for k, v in pairs(DEFAULTS) do
      db[k] = v
    end
  elseif cmd == "test" then
    for _, s in ipairs(SAMPLES) do
      push(s[1], s[2], s[3], s[4])
    end
    say("6 test messages queued")
  elseif cmd == "testlog" then
    for _, s in ipairs(LOG_SAMPLES) do
      pushLog(s[1], s[2], s[3])
    end
    say(#LOG_SAMPLES .. " test log lines queued (shown when the app's Registro is on)")
  else
    say("/wct (show/hide) | cell <1-4> | hold <0.05-1> | reset | test | testlog")
    return
  end
  layout()
end
