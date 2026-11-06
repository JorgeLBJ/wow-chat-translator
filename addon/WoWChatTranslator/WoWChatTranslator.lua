-- WoW Chat Translator: OCR feed panel.
-- Each incoming chat message is rendered as one plain line:
--   NNN Channel CLASS Player Name: message
-- CLASS is the English class file name (MAGE, WARLOCK...) or NOCLASS when unknown.
-- Channel is always one word; brackets were dropped because OCR misreads them.
-- NNN is a sequence number (000-999) the companion app uses to detect new lines.

local ADDON = ...

local DEFAULTS = {
  shown = true,
  locked = false,
  point = { "BOTTOMRIGHT", "UIParent", "BOTTOMRIGHT", -20, 120 },
  width = 520,
  height = 200,
  fontSize = 18,
  skipOwn = true,
}

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

local seq = 0
local db

-- Removes color codes, hyperlinks, textures and raid icons so OCR sees plain text.
-- Links are written as "< Name >" with spaces, and two links in a row are separated
-- by a space: in OCR tests "<" and ">" were read correctly every time, while "]" at a
-- line end or between two links was misread or dropped. The app uses these markers
-- to keep link names untranslated and shows them as "[Name]".
local function sanitize(text)
  text = text:gsub("|c%x%x%x%x%x%x%x%x", "")
  text = text:gsub("|cn[^:]*:", "") -- named colors, e.g. item quality "|cnIQ1:"
  text = text:gsub("|r", "")
  text = text:gsub("|H.-|h(.-)|h", "%1")
  text = text:gsub("|T.-|t", "")
  text = text:gsub("|A.-|a", "")
  text = text:gsub("{.-}", "")
  text = text:gsub("|", "/")
  text = text:gsub("%[%s*([^%[%]]-)%s*%]", "< %1 >")
  text = text:gsub(">%s*<", "> <")
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

-- Panel ---------------------------------------------------------------

local panel = CreateFrame("Frame", "WoWChatTranslatorPanel", UIParent)
panel:SetFrameStrata("MEDIUM")
panel:SetClampedToScreen(true)
panel:Hide()

local bg = panel:CreateTexture(nil, "BACKGROUND")
bg:SetAllPoints()
bg:SetColorTexture(0, 0, 0, 1)

local feed = CreateFrame("ScrollingMessageFrame", nil, panel)
feed:SetPoint("TOPLEFT", 8, -6)
feed:SetPoint("BOTTOMRIGHT", -8, 6)
feed:SetFading(false)
feed:SetMaxLines(50)
feed:SetInsertMode("BOTTOM")
feed:SetJustifyH("LEFT")
feed:SetShadowOffset(0, 0)

-- Resize grip (bottom-right). Hidden while locked so OCR never sees it.
local grip = CreateFrame("Button", nil, panel)
grip:SetSize(16, 16)
grip:SetPoint("BOTTOMRIGHT", -2, 2)
grip:SetNormalTexture("Interface\\ChatFrame\\UI-ChatIM-SizeGrabber-Up")
grip:SetHighlightTexture("Interface\\ChatFrame\\UI-ChatIM-SizeGrabber-Highlight")
grip:SetPushedTexture("Interface\\ChatFrame\\UI-ChatIM-SizeGrabber-Down")

panel:SetResizable(true)
if panel.SetResizeBounds then
  panel:SetResizeBounds(200, 60, 1600, 900)
elseif panel.SetMinResize then
  panel:SetMinResize(200, 60)
end

local function savePosition()
  local point, _, relPoint, x, y = panel:GetPoint(1)
  db.point = { point, "UIParent", relPoint, x, y }
  db.width, db.height = panel:GetSize()
end

grip:SetScript("OnMouseDown", function()
  if not db.locked then
    panel:StartSizing("BOTTOMRIGHT")
  end
end)
grip:SetScript("OnMouseUp", function()
  panel:StopMovingOrSizing()
  savePosition()
end)

local function applyLayout()
  panel:ClearAllPoints()
  local p = db.point
  panel:SetPoint(p[1], UIParent, p[3], p[4], p[5])
  panel:SetSize(db.width, db.height)
  feed:SetFont("Fonts\\ARIALN.TTF", db.fontSize, "")
  panel:EnableMouse(not db.locked)
  grip:SetShown(not db.locked)
  panel:SetShown(db.shown)
end

panel:SetMovable(true)
panel:RegisterForDrag("LeftButton")
panel:SetScript("OnDragStart", function(self)
  if not db.locked then
    self:StartMoving()
  end
end)
panel:SetScript("OnDragStop", function(self)
  self:StopMovingOrSizing()
  savePosition()
end)

local function classOf(guid)
  if type(guid) ~= "string" or guid == "" or not GetPlayerInfoByGUID then
    return "NOCLASS"
  end
  local ok, _, englishClass = pcall(GetPlayerInfoByGUID, guid)
  if ok and type(englishClass) == "string" and englishClass ~= "" then
    return englishClass
  end
  return "NOCLASS"
end

local function push(channel, class, player, message)
  local clean = sanitize(message)
  if clean == "" then
    return
  end
  seq = (seq + 1) % 1000
  feed:AddMessage(string.format("%03d %s %s %s: %s", seq, channel, class, player, clean), 1, 1, 1)
end

-- Events --------------------------------------------------------------

local events = CreateFrame("Frame")
events:RegisterEvent("ADDON_LOADED")

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
    applyLayout()
    for chatEvent in pairs(EVENT_CHANNEL) do
      -- Some events do not exist on every client; ignore those.
      pcall(events.RegisterEvent, events, chatEvent)
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

-- Slash commands ------------------------------------------------------

local SAMPLES = {
  { "Trade", "MAGE", "Kaelthas", "WTS |cffa335ee|Hitem:12360::::::::|h[Arcanite Bar]|h|r 40g pst" },
  { "General", "DRUID", "Lunafae", "alguien para Deadmines?" },
  { "Say", "WARRIOR", "Brunhilde", "anyone know where the trainer is?" },
  { "Guild", "SHAMAN", "Pedrinho", "bora fazer raid hoje à noite" },
  { "Party", "WARLOCK", "Schatten", "ich brauche noch Mana, kurz warten {rt1}" },
  { "General", "NOCLASS", "Volkov", "LFM ZG need heal" },
}

local function say(msg)
  DEFAULT_CHAT_FRAME:AddMessage("|cff33ff99WCT|r " .. msg)
end

SLASH_WOWCHATTRANSLATOR1 = "/wct"
SlashCmdList.WOWCHATTRANSLATOR = function(input)
  local cmd, arg = strsplit(" ", strtrim(input or ""):lower(), 2)
  if cmd == "" then
    db.shown = not db.shown
  elseif cmd == "lock" then
    db.locked = not db.locked
    say(db.locked and "panel locked" or "panel unlocked (drag to move, bottom-right corner to resize)")
  elseif cmd == "size" and tonumber(arg) then
    db.fontSize = math.max(10, math.min(24, tonumber(arg)))
  elseif cmd == "reset" then
    for k, v in pairs(DEFAULTS) do
      db[k] = v
    end
  elseif cmd == "test" then
    for _, s in ipairs(SAMPLES) do
      push(s[1], s[2], s[3], s[4])
    end
  elseif cmd == "clear" then
    feed:Clear()
  else
    say("/wct (show/hide) | lock | size <10-24> | reset | test | clear")
    return
  end
  applyLayout()
end
