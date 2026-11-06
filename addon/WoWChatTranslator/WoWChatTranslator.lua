-- WoW Chat Translator: pixel strip.
-- Each incoming chat message is sent to the companion app as a small grid of colored
-- cells at the top-left of the screen, which the app decodes exactly (no OCR).
-- The format must match app/src/core/strip.js (see that file for the layout); its
-- design is informed by Parley's documented protocol (MIT) and LibSerpix (MIT).

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

local db

-- Text cleanup ----------------------------------------------------------------
-- Removes color codes, textures and raid icons. Real links (|H...|h[Name]|h) are
-- wrapped in \2 ... \3, characters players cannot type, so the app keeps only real
-- link names untranslated; brackets a player types by hand are ordinary text.
local LINK_OPEN, LINK_CLOSE = "\2", "\3"
local function sanitize(text)
  text = text:gsub("[\2\3]", "")
  text = text:gsub("|c%x%x%x%x%x%x%x%x", "")
  text = text:gsub("|cn[^:]*:", "") -- named colors, e.g. item quality "|cnIQ1:"
  text = text:gsub("|r", "")
  text = text:gsub("|H.-|h(.-)|h", LINK_OPEN .. "%1" .. LINK_CLOSE)
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
local queue = {}
local seq = math.random(0, 4095)
local shownAt = 0

local function showNext()
  local payload = table.remove(queue, 1)
  seq = (seq + 1) % 4096
  draw(encodeFrame(seq, payload))
  shownAt = GetTime()
end

strip:SetScript("OnUpdate", function()
  if #queue > 0 and GetTime() - shownAt >= db.hold then
    showNext()
  end
end)

local function push(channel, class, player, message)
  local clean = sanitize(message)
  if clean == "" then
    return
  end
  local payload = table.concat({ VERSION, channel, class, player, clean }, SEP)
  queue[#queue + 1] = cutUtf8(payload, MAX_PAYLOAD)
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
    showNext()
    for chatEvent in pairs(EVENT_CHANNEL) do
      -- Some events do not exist on every client; ignore those.
      pcall(events.RegisterEvent, events, chatEvent)
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
  else
    say("/wct (show/hide) | cell <1-4> | hold <0.05-1> | reset | test")
    return
  end
  layout()
end
