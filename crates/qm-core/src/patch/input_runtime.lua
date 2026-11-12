-- ============================================================================
-- input_runtime.lua — Built-in QuarterMaster Keybinding & Controls Framework
-- Reconfigurable Dual-Bind Controls, Combo Engine, LÖVE Engine Shim, and Screen 87
-- ============================================================================

_G.QM = _G.QM or {}
local input = {}
_G.QM.Input = input

input.PAGE_CONTROLS = 87

-- State Storage
input.actions = {}           -- Array of registered actions
input.actionMap = {}         -- actionId -> action definition
input.tabs = { "vanilla", "uncategorized" }
input.tabNames = {
  vanilla = "Vanilla Controls",
  uncategorized = "Uncategorized",
  modded = "Uncategorized",
}
input.activeTab = "vanilla"
input.tabPageIndex = 1
input.tabsPerPage = 5

-- Capture State
input.capturingAction = nil  -- actionId currently awaiting input
input.capturingSlot = 1      -- 1 = Primary, 2 = Secondary
input.heldKeys = {}          -- map of keys currently held down { [key] = true }
input.capturedCombo = {}     -- ordered list of unique keys/buttons pressed in sequence
input.waitingForRelease = false

-- Persistence & Conflicts
input.pendingConflict = nil  -- { actionId, slot, newKey, conflictingId, conflictingSlot }
input.savedKeybinds = {}     -- Persistent table saved to saveKeybinds.lua
input.previousPage = 86

-- Row Pagination
input.pageIndex = 1
input.visibleRows = 11

-- Sound helper
local function playClickSound()
  local sfx = (weaponInspect1 and _G["weaponInspect" .. randNbr(5)]) or clickSound
  if sfx and type(sfx.clone) == "function" then
    local ok, c = pcall(sfx.clone, sfx)
    if ok and c and type(c.play) == "function" then
      pcall(c.play, c)
    end
  end
end

-- Text drawing helper with 1px drop shadow
local function drawCleanShadowText(text, x, y, limit, align, font, textColor)
  local f = font or cardFont or bigFont
  local fade = uiFade or 255
  local tc = textColor or { 255, 255, 255, fade }

  if f then love.graphics.setFont(f) end
  love.graphics.printf({{love.math.colorFromBytes(0, 0, 0, math.min(180, fade))}, text}, x + 1, y + 1, limit or 200, align or "left")
  love.graphics.printf({{love.math.colorFromBytes(tc[1], tc[2], tc[3], tc[4] or fade)}, text}, x, y, limit or 200, align or "left")
end

-- Key display name formatting
local KEY_LABELS = {
  lctrl = "L-CTRL", rctrl = "R-CTRL", ctrl = "CTRL",
  lshift = "L-SHIFT", rshift = "R-SHIFT", shift = "SHIFT",
  lalt = "L-ALT", ralt = "R-ALT", alt = "ALT",
  escape = "ESC", backspace = "BACKSPACE",
  space = "SPACE", tab = "TAB", ["return"] = "ENTER",
  left = "LEFT", right = "RIGHT", up = "UP", down = "DOWN",
  kp0 = "NUM 0", kp1 = "NUM 1", kp2 = "NUM 2", kp3 = "NUM 3",
  kp4 = "NUM 4", kp5 = "NUM 5", kp6 = "NUM 6", kp7 = "NUM 7",
  kp8 = "NUM 8", kp9 = "NUM 9",
  mouse1 = "M1",
  mouse2 = "M2",
  mouse3 = "M3",
  mouse4 = "M4",
  mouse5 = "M5",
  mouse6 = "M6", mouse7 = "M7", mouse8 = "M8",
}

function input.formatSingleKey(k)
  if not k or k == "" then return "" end
  local lk = tostring(k):lower()
  return KEY_LABELS[lk] or lk:upper()
end

function input.formatKey(combo)
  if not combo or combo == "" or combo == "[NONE]" then
    return "[NONE]"
  end
  local parts = {}
  for p in combo:gmatch("[^+]+") do
    table.insert(parts, input.formatSingleKey(p))
  end
  if #parts == 0 then return "[NONE]" end
  return table.concat(parts, "+")
end

-- Build canonical combo string from an array of pressed key identifiers
function input.buildComboString(keysList)
  if not keysList or #keysList == 0 then return nil end
  local hasCtrl = false
  local hasShift = false
  local hasAlt = false
  local baseKeys = {}

  for _, k in ipairs(keysList) do
    local lk = tostring(k):lower()
    if lk == "lctrl" or lk == "rctrl" or lk == "ctrl" then
      hasCtrl = true
    elseif lk == "lshift" or lk == "rshift" or lk == "shift" then
      hasShift = true
    elseif lk == "lalt" or lk == "ralt" or lk == "alt" then
      hasAlt = true
    else
      local present = false
      for _, bk in ipairs(baseKeys) do
        if bk == lk then present = true; break end
      end
      if not present then
        table.insert(baseKeys, lk)
      end
    end
  end

  local parts = {}
  if hasCtrl then table.insert(parts, "ctrl") end
  if hasShift then table.insert(parts, "shift") end
  if hasAlt then table.insert(parts, "alt") end

  if #baseKeys > 0 then
    for _, bk in ipairs(baseKeys) do
      table.insert(parts, bk)
    end
  elseif #parts == 0 then
    return nil
  end

  return table.concat(parts, "+")
end

-- Parse a combo string into components for evaluation
function input.parseCombo(bindingStr)
  if not bindingStr or bindingStr == "" or bindingStr == "[NONE]" then
    return nil
  end
  local parts = {}
  for p in bindingStr:gmatch("[^+]+") do
    table.insert(parts, p:lower())
  end
  local reqCtrl, reqShift, reqAlt = false, false, false
  local baseKey = nil
  for _, p in ipairs(parts) do
    if p == "ctrl" or p == "lctrl" or p == "rctrl" then
      reqCtrl = true
    elseif p == "shift" or p == "lshift" or p == "rshift" then
      reqShift = true
    elseif p == "alt" or p == "lalt" or p == "ralt" then
      reqAlt = true
    else
      baseKey = p
    end
  end

  if not baseKey and (reqCtrl or reqShift or reqAlt) then
    baseKey = reqCtrl and "ctrl" or (reqShift and "shift" or "alt")
    if baseKey == "ctrl" then reqCtrl = false end
    if baseKey == "shift" then reqShift = false end
    if baseKey == "alt" then reqAlt = false end
  end

  return {
    baseKey = baseKey,
    reqCtrl = reqCtrl,
    reqShift = reqShift,
    reqAlt = reqAlt,
  }
end

-- Match an incoming key/button event against a binding combo string
function input.matchesInput(bindingStr, inputKey, isCtrl, isShift, isAlt)
  local parsed = input.parseCombo(bindingStr)
  if not parsed or not parsed.baseKey then return false end
  local ik = tostring(inputKey):lower()
  if ik == "lctrl" or ik == "rctrl" then ik = "ctrl" end
  if ik == "lshift" or ik == "rshift" then ik = "shift" end
  if ik == "lalt" or ik == "ralt" then ik = "alt" end

  if parsed.baseKey ~= ik then return false end
  if parsed.reqCtrl ~= (isCtrl == true) then return false end
  if parsed.reqShift ~= (isShift == true) then return false end
  if parsed.reqAlt ~= (isAlt == true) then return false end
  return true
end

-- Check if a specific binding combo is currently held down
function input.isBindingDown(bindingStr)
  local parsed = input.parseCombo(bindingStr)
  if not parsed or not parsed.baseKey then return false end
  local origIsDown = input.origIsDown or (love and love.keyboard and love.keyboard.isDown)
  local origMDown = (love and love.mouse and love.mouse.isDown)

  local isCtrl = origIsDown and (origIsDown("lctrl") or origIsDown("rctrl"))
  local isShift = origIsDown and (origIsDown("lshift") or origIsDown("rshift"))
  local isAlt = origIsDown and (origIsDown("lalt") or origIsDown("ralt"))

  if parsed.reqCtrl and not isCtrl then return false end
  if parsed.reqShift and not isShift then return false end
  if parsed.reqAlt and not isAlt then return false end

  local bk = parsed.baseKey
  if bk:match("^mouse(%d+)$") then
    local btnNum = tonumber(bk:match("^mouse(%d+)$"))
    if origMDown and btnNum then
      local ok, res = pcall(origMDown, btnNum)
      if ok and res then return true end
    end
    return false
  end

  if bk == "ctrl" then return isCtrl end
  if bk == "shift" then return isShift end
  if bk == "alt" then return isAlt end

  if origIsDown then
    local ok, res = pcall(origIsDown, bk)
    return ok and res or false
  end
  return false
end

-- Check if an action is currently held down via primary or secondary binding
function input.isActionDown(actionId)
  local act = input.actionMap[actionId]
  if not act then return false end
  if act.primaryKey and input.isBindingDown(act.primaryKey) then return true end
  if act.secondaryKey and input.isBindingDown(act.secondaryKey) then return true end
  return false
end

-- ============================================================================
-- PERSISTENCE: saveKeybinds.lua
-- ============================================================================

function input.loadSavedKeybinds()
  if love and love.filesystem and love.filesystem.getInfo and love.filesystem.getInfo("saveKeybinds.lua") then
    local ok_read, raw = pcall(love.filesystem.read, "saveKeybinds.lua")
    if ok_read and type(raw) == "string" then
      if raw:sub(1, 14) == "return local _" then
        raw = raw:sub(8)
      end
      local loader = loadstring or load
      local ok, chunk = pcall(loader, raw, "saveKeybinds.lua")
      if ok and type(chunk) == "function" then
        local ok2, data = pcall(chunk)
        if ok2 and type(data) == "table" then
          input.savedKeybinds = data
        end
      end
    end
  end

  -- Apply saved binds to registered actions
  for actId, saved in pairs(input.savedKeybinds) do
    local act = input.actionMap[actId]
    if act and type(saved) == "table" then
      if saved.primary ~= nil then
        act.primaryKey = (saved.primary ~= "" and saved.primary ~= "[NONE]") and saved.primary or nil
      elseif saved.keys and saved.keys[1] then
        act.primaryKey = saved.keys[1]
      end

      if saved.secondary ~= nil then
        act.secondaryKey = (saved.secondary ~= "" and saved.secondary ~= "[NONE]") and saved.secondary or nil
      elseif saved.keys and saved.keys[2] then
        act.secondaryKey = saved.keys[2]
      end

      if saved.overrideConflict ~= nil then
        act.overrideConflict = saved.overrideConflict
      end

      act.currentKeys = {}
      if act.primaryKey then table.insert(act.currentKeys, act.primaryKey) end
      if act.secondaryKey then table.insert(act.currentKeys, act.secondaryKey) end
    end
  end
end

function input.saveCurrentKeybinds()
  if not love or not love.filesystem then return end
  local ser = require "includes/ser"
  if not ser then return end

  local dataToSave = {}
  for _, act in ipairs(input.actions) do
    local isDifferent = false
    if act.primaryKey ~= act.defaultPrimary or act.secondaryKey ~= act.defaultSecondary then
      isDifferent = true
    end

    if isDifferent or act.overrideConflict then
      dataToSave[act.id] = {
        primary = act.primaryKey or "",
        secondary = act.secondaryKey or "",
        keys = { act.primaryKey or "", act.secondaryKey or "" },
        overrideConflict = act.overrideConflict or false,
      }
    end
  end

  local ok, str = pcall(ser, dataToSave)
  if ok and str then
    if not str:match("^local _=") and not str:match("^return ") then
      str = "return " .. str
    end
    love.filesystem.write("saveKeybinds.lua", str)
  end
end

-- ============================================================================
-- ACTION REGISTRATION
-- ============================================================================

function input.registerAction(def)
  if not def or not def.id or not def.name then return end

  local canonicalTab = def.tab
  if not canonicalTab or canonicalTab == "modded" then
    canonicalTab = "uncategorized"
  end

  local function combineModifiers(baseKey, mods)
    if not baseKey or baseKey == "" or baseKey == "[NONE]" then return nil end
    if baseKey:find("+") then return baseKey end
    if not mods then return baseKey end
    local comboList = {}
    if type(mods) == "table" then
      for _, m in ipairs(mods) do
        if m and m ~= "" then table.insert(comboList, m) end
      end
    elseif type(mods) == "string" and mods ~= "" then
      table.insert(comboList, mods)
    end
    if #comboList == 0 then return baseKey end
    table.insert(comboList, baseKey)
    return input.buildComboString(comboList) or baseKey
  end

  local defaultPri = nil
  local defaultSec = nil
  if def.defaultKeys and #def.defaultKeys > 0 then
    defaultPri = def.defaultKeys[1]
    defaultSec = def.defaultKeys[2]
  else
    defaultPri = def.default
    defaultSec = def.secondary or def.default_secondary
  end

  defaultPri = combineModifiers(defaultPri, def.modifiers or def.modifier)
  defaultSec = combineModifiers(defaultSec, def.secondary_modifiers or def.secondaryModifiers or def.secondary_modifier)

  if input.actionMap[def.id] then
    local existing = input.actionMap[def.id]
    existing.name = def.name
    existing.category = def.category or existing.category
    existing.type = def.type or existing.type
    existing.tab = canonicalTab or existing.tab
    existing.description = def.description or existing.description
    existing.onTrigger = def.onTrigger or def.on_trigger or existing.onTrigger
    existing.vanillaKey = def.vanillaKey or existing.vanillaKey
    if defaultPri and (not existing.defaultPrimary or existing.defaultPrimary == def.default) then
      existing.defaultPrimary = defaultPri
      if not existing.primaryKey or existing.primaryKey == def.default then
        existing.primaryKey = defaultPri
      end
    end
    if defaultSec and (not existing.defaultSecondary or existing.defaultSecondary == (def.secondary or def.default_secondary)) then
      existing.defaultSecondary = defaultSec
      if not existing.secondaryKey or existing.secondaryKey == (def.secondary or def.default_secondary) then
        existing.secondaryKey = defaultSec
      end
    end
    existing.defaultKeys = { existing.defaultPrimary, existing.defaultSecondary }
    existing.currentKeys = {}
    if existing.primaryKey then table.insert(existing.currentKeys, existing.primaryKey) end
    if existing.secondaryKey then table.insert(existing.currentKeys, existing.secondaryKey) end
    return existing
  end

  local act = {
    id = def.id,
    name = def.name,
    category = def.category or "General",
    tab = canonicalTab,
    defaultPrimary = defaultPri,
    defaultSecondary = defaultSec,
    defaultKeys = { defaultPri, defaultSec },
    primaryKey = defaultPri,
    secondaryKey = defaultSec,
    currentKeys = {},
    type = def.type or "trigger",
    description = def.description,
    onTrigger = def.onTrigger or def.on_trigger,
    vanillaKey = def.vanillaKey,
    overrideConflict = false,
  }

  if act.primaryKey then table.insert(act.currentKeys, act.primaryKey) end
  if act.secondaryKey then table.insert(act.currentKeys, act.secondaryKey) end

  -- Apply saved overrides if already loaded
  if input.savedKeybinds[act.id] then
    local saved = input.savedKeybinds[act.id]
    if saved.primary ~= nil then
      act.primaryKey = (saved.primary ~= "" and saved.primary ~= "[NONE]") and saved.primary or nil
    elseif saved.keys and saved.keys[1] then
      act.primaryKey = saved.keys[1]
    end

    if saved.secondary ~= nil then
      act.secondaryKey = (saved.secondary ~= "" and saved.secondary ~= "[NONE]") and saved.secondary or nil
    elseif saved.keys and saved.keys[2] then
      act.secondaryKey = saved.keys[2]
    end

    if saved.overrideConflict ~= nil then
      act.overrideConflict = saved.overrideConflict
    end

    act.currentKeys = {}
    if act.primaryKey then table.insert(act.currentKeys, act.primaryKey) end
    if act.secondaryKey then table.insert(act.currentKeys, act.secondaryKey) end
  end

  -- Tab registration
  if act.tab and not input.tabNames[act.tab] then
    table.insert(input.tabs, act.tab)
    input.tabNames[act.tab] = def.tabName or act.tab
  end

  table.insert(input.actions, act)
  input.actionMap[act.id] = act
  return act
end

-- Visible tabs list generator (Uncategorized tab only appears if it has actions!)
function input.getVisibleTabs()
  local tabCounts = {}
  for _, act in ipairs(input.actions) do
    local t = act.tab or "uncategorized"
    if t == "modded" then t = "uncategorized" end
    tabCounts[t] = (tabCounts[t] or 0) + 1
  end

  local visible = {}
  if (tabCounts["vanilla"] or 0) > 0 then
    table.insert(visible, "vanilla")
  end

  for _, t in ipairs(input.tabs) do
    if t ~= "vanilla" and t ~= "modded" and t ~= "uncategorized" then
      if (tabCounts[t] or 0) > 0 then
        local found = false
        for _, v in ipairs(visible) do if v == t then found = true; break end end
        if not found then table.insert(visible, t) end
      end
    end
  end

  for t, count in pairs(tabCounts) do
    if t ~= "vanilla" and t ~= "modded" and t ~= "uncategorized" and count > 0 then
      local found = false
      for _, v in ipairs(visible) do if v == t then found = true; break end end
      if not found then table.insert(visible, t) end
    end
  end

  -- Uncategorized only appears if populated!
  if (tabCounts["uncategorized"] or 0) > 0 then
    table.insert(visible, "uncategorized")
  end

  return visible
end

-- Initialize complete vanilla controls inventory
local function initVanillaActions()
  -- Camera Movement
  input.registerAction({
    id = "camera_left",
    name = "Pan Camera Left",
    category = "Camera",
    tab = "vanilla",
    default = "a",
    secondary = "left",
    type = "hold",
    description = "Pans tactical battlefield view left",
  })
  input.registerAction({
    id = "camera_right",
    name = "Pan Camera Right",
    category = "Camera",
    tab = "vanilla",
    default = "d",
    secondary = "right",
    type = "hold",
    description = "Pans tactical battlefield view right",
  })

  -- Squad Orders & Controls
  input.registerAction({
    id = "squad_halt",
    name = "Halt All Troops",
    category = "Squad Orders",
    tab = "vanilla",
    default = "h",
    type = "trigger",
    vanillaKey = "h",
    description = "Orders all player-controlled soldiers to stop moving",
  })
  input.registerAction({
    id = "grouping",
    name = "Squad Multi-Select / Grouping",
    category = "Squad Orders",
    tab = "vanilla",
    default = "lctrl",
    secondary = "rctrl",
    type = "hold",
    description = "Hold to drag-select or assign control groups",
  })
  input.registerAction({
    id = "alt_move",
    name = "Directional Target / Alt Move",
    category = "Squad Orders",
    tab = "vanilla",
    default = "lalt",
    secondary = "ralt",
    type = "hold",
    description = "Hold to order directional movement and facing",
  })
  input.registerAction({
    id = "selection_mod",
    name = "Selection Range Modifier",
    category = "Squad Orders",
    tab = "vanilla",
    default = "lshift",
    secondary = "rshift",
    type = "hold",
    description = "Hold shift for modifier selection in trenches and squads",
  })
  input.registerAction({
    id = "deselect",
    name = "Deselect All / Cancel Support",
    category = "Squad Orders",
    tab = "vanilla",
    default = "backspace",
    type = "trigger",
    vanillaKey = "backspace",
    description = "Clears active soldier selections and support calls",
  })

  -- Display & View
  input.registerAction({
    id = "pause",
    name = "Pause Game / Menu",
    category = "Display & View",
    tab = "vanilla",
    default = "space",
    secondary = "escape",
    type = "trigger",
    vanillaKey = "space",
    description = "Toggles tactical pause or menu screens",
  })
  input.registerAction({
    id = "toggle_faces",
    name = "Soldier Cards / Camo HUD",
    category = "Display & View",
    tab = "vanilla",
    default = "tab",
    type = "trigger",
    vanillaKey = "tab",
    description = "Toggles soldier portrait heads-up and camouflage display",
  })
  input.registerAction({
    id = "fade_roofs",
    name = "Fade Roofs / Reset View",
    category = "Display & View",
    tab = "vanilla",
    default = "r",
    type = "trigger",
    vanillaKey = "r",
    description = "Toggles building roof opacity and resets camera angle",
  })
  input.registerAction({
    id = "tactical_view",
    name = "Tactical View",
    category = "Display & View",
    tab = "vanilla",
    default = "f9",
    type = "trigger",
    vanillaKey = "f9",
    description = "Toggles terrain nav grid and line of sight overlays",
  })
  input.registerAction({
    id = "windowed",
    name = "Windowed Display Mode",
    category = "Display & View",
    tab = "vanilla",
    default = "f3",
    type = "trigger",
    vanillaKey = "f3",
    description = "Switches display to windowed mode",
  })
  input.registerAction({
    id = "fullscreen",
    name = "Fullscreen Display Mode",
    category = "Display & View",
    tab = "vanilla",
    default = "f4",
    type = "trigger",
    vanillaKey = "f4",
    description = "Switches display to fullscreen mode",
  })

  -- Control Groups
  for i = 1, 9 do
    input.registerAction({
      id = "group_" .. i,
      name = "Control Group " .. i,
      category = "Control Groups",
      tab = "vanilla",
      default = tostring(i),
      secondary = "kp" .. i,
      type = "trigger",
      vanillaKey = tostring(i),
      description = "Select or assign squad control group " .. i,
    })
  end
  input.registerAction({
    id = "group_0",
    name = "Control Group 0",
    category = "Control Groups",
    tab = "vanilla",
    default = "0",
    secondary = "kp0",
    type = "trigger",
    vanillaKey = "0",
    description = "Select or assign squad control group 0",
  })
end

initVanillaActions()

-- ============================================================================
-- CONFLICT CHECKING & KEY BINDING
-- ============================================================================

function input.findConflict(actionId, testCombo, slot)
  local targetAct = input.actionMap[actionId]
  if not targetAct or not testCombo then return nil end

  for _, other in ipairs(input.actions) do
    if other.id ~= actionId and other.tab == targetAct.tab then
      if other.primaryKey == testCombo then
        return { act = other, conflictingSlot = 1 }
      end
      if other.secondaryKey == testCombo then
        return { act = other, conflictingSlot = 2 }
      end
    end
  end
  return nil
end

function input.setBindingSlot(actionId, slot, combo, overrideConflict)
  local act = input.actionMap[actionId]
  if not act then return end
  if slot == 1 then
    act.primaryKey = combo
  elseif slot == 2 then
    act.secondaryKey = combo
  end
  act.currentKeys = {}
  if act.primaryKey then table.insert(act.currentKeys, act.primaryKey) end
  if act.secondaryKey then table.insert(act.currentKeys, act.secondaryKey) end
  act.overrideConflict = overrideConflict or false
  input.saveCurrentKeybinds()
end

function input.clearBindingSlot(actionId, slot)
  local act = input.actionMap[actionId]
  if not act then return end
  if slot == 1 then
    act.primaryKey = nil
  elseif slot == 2 then
    act.secondaryKey = nil
  end
  act.currentKeys = {}
  if act.primaryKey then table.insert(act.currentKeys, act.primaryKey) end
  if act.secondaryKey then table.insert(act.currentKeys, act.secondaryKey) end
  act.overrideConflict = false
  input.saveCurrentKeybinds()
end

function input.clearBinding(actionId)
  local act = input.actionMap[actionId]
  if not act then return end
  act.primaryKey = nil
  act.secondaryKey = nil
  act.currentKeys = {}
  act.overrideConflict = false
  input.saveCurrentKeybinds()
end

function input.resetAction(actionId)
  local act = input.actionMap[actionId]
  if not act then return end
  act.primaryKey = act.defaultPrimary
  act.secondaryKey = act.defaultSecondary
  act.currentKeys = {}
  if act.primaryKey then table.insert(act.currentKeys, act.primaryKey) end
  if act.secondaryKey then table.insert(act.currentKeys, act.secondaryKey) end
  act.overrideConflict = false
  input.saveCurrentKeybinds()
end

function input.resetTab(tabKey)
  for _, act in ipairs(input.actions) do
    if act.tab == tabKey then
      act.primaryKey = act.defaultPrimary
      act.secondaryKey = act.defaultSecondary
      act.currentKeys = {}
      if act.primaryKey then table.insert(act.currentKeys, act.primaryKey) end
      if act.secondaryKey then table.insert(act.currentKeys, act.secondaryKey) end
      act.overrideConflict = false
    end
  end
  input.saveCurrentKeybinds()
end

-- ============================================================================
-- LÖVE ENGINE SHIM: KEYBOARD & MOUSE WRAPPERS
-- ============================================================================

input.origIsDown = input.origIsDown or (love and love.keyboard and love.keyboard.isDown)
input.origKeyPressed = input.origKeyPressed or (love and love.keypressed)
input.origKeyReleased = input.origKeyReleased or (love and love.keyreleased)
input.origMousePressed = input.origMousePressed or (love and love.mousepressed)
input.origMouseReleased = input.origMouseReleased or (love and love.mousereleased)
input.origWheelMoved = input.origWheelMoved or (love and love.wheelmoved)

function input.handleIsDown(...)
  if input.capturingAction ~= nil then
    return false
  end

  local args = { ... }
  if #args == 0 then return false end
  local origIsDown = input.origIsDown

  for _, arg in ipairs(args) do
    if arg == "a" or arg == "left" then
      if input.isActionDown("camera_left") then return true end
    elseif arg == "d" or arg == "right" then
      if input.isActionDown("camera_right") then return true end
    elseif arg == "lctrl" or arg == "rctrl" or arg == "ctrl" then
      if input.isActionDown("grouping") then return true end
    elseif arg == "lalt" or arg == "ralt" or arg == "alt" then
      if input.isActionDown("alt_move") then return true end
    elseif arg == "lshift" or arg == "rshift" or arg == "shift" then
      if input.isActionDown("selection_mod") then return true end
    end
  end

  if origIsDown then
    return origIsDown(...)
  end
  return false
end

function input.resolveTrigger(trigger)
  if type(trigger) == "function" then
    return trigger
  elseif type(trigger) == "string" then
    local cur = _G
    for part in trigger:gmatch("[^%.]+") do
      if type(cur) ~= "table" then return nil end
      cur = cur[part]
    end
    if type(cur) == "function" then
      return cur
    end
  end
  return nil
end

-- Finalize captured combo when all keys/mouse buttons are released
local function finalizeCapture()
  local actId = input.capturingAction
  local slot = input.capturingSlot or 1
  local combo = input.buildComboString(input.capturedCombo)

  input.capturingAction = nil
  input.capturingSlot = 1
  input.heldKeys = {}
  input.capturedCombo = {}
  input.waitingForRelease = false

  if not combo then return end

  local conflict = input.findConflict(actId, combo, slot)
  if conflict and not conflict.act.overrideConflict then
    input.pendingConflict = {
      actionId = actId,
      slot = slot,
      newKey = combo,
      conflictingId = conflict.act.id,
      conflictingSlot = conflict.conflictingSlot,
    }
  else
    input.setBindingSlot(actId, slot, combo, false)
  end
  playClickSound()
end

function input.handleKeyPressed(key)
  local origIsDown = input.origIsDown or (love and love.keyboard and love.keyboard.isDown)
  local origKeyPressed = input.origKeyPressed

  -- 1. INPUT CAPTURE: Wait for key release before locking in binding
  if input.capturingAction ~= nil then
    local actId = input.capturingAction
    local slot = input.capturingSlot or 1

    if key == "escape" then
      -- ESCAPE CLEARS ACTIVE SLOT TO [NONE] IMMEDIATELY
      input.clearBindingSlot(actId, slot)
      input.capturingAction = nil
      input.capturingSlot = 1
      input.heldKeys = {}
      input.capturedCombo = {}
      input.waitingForRelease = false
      playClickSound()
      return
    end

    input.heldKeys[key] = true
    input.waitingForRelease = true

    local already = false
    for _, k in ipairs(input.capturedCombo) do
      if k == key then already = true; break end
    end
    if not already then
      table.insert(input.capturedCombo, key)
    end
    return
  end

  -- 2. DISPATCH MOD ACTIONS
  local isShift = origIsDown and (origIsDown("lshift") or origIsDown("rshift"))
  local isCtrl = origIsDown and (origIsDown("lctrl") or origIsDown("rctrl"))
  local isAlt = origIsDown and (origIsDown("lalt") or origIsDown("ralt"))
  local consumed = false

  for _, act in ipairs(input.actions) do
    if act.tab ~= "vanilla" or act.onTrigger then
      local m1 = act.primaryKey and input.matchesInput(act.primaryKey, key, isCtrl, isShift, isAlt)
      local m2 = act.secondaryKey and input.matchesInput(act.secondaryKey, key, isCtrl, isShift, isAlt)

      if (m1 or m2) and act.onTrigger then
        local fn = input.resolveTrigger(act.onTrigger)
        if fn then
          consumed = true
          local ok, err = pcall(fn)
          if not ok then
            print("[QM.Input] Error executing action '" .. tostring(act.id) .. "': " .. tostring(err))
          end
        end
      end
    end
  end

  if consumed then return end

  -- 3. TRANSLATE REBOUND VANILLA ACTIONS
  local translatedKey = nil
  for _, act in ipairs(input.actions) do
    local fwdKey = act.vanillaKey or (act.tab ~= "vanilla" and not act.onTrigger and act.defaultPrimary)
    if fwdKey then
      local m1 = act.primaryKey and input.matchesInput(act.primaryKey, key, isCtrl, isShift, isAlt)
      local m2 = act.secondaryKey and input.matchesInput(act.secondaryKey, key, isCtrl, isShift, isAlt)
      if m1 or m2 then
        translatedKey = fwdKey
        break
      end
    end
  end

  if translatedKey then
    if origKeyPressed then
      origKeyPressed(translatedKey)
    end
    return
  end

  -- 4. SUPPRESS DEFAULT KEYS THAT WERE REBOUND TO ANOTHER KEY
  local wasReboundAway = false
  for _, act in ipairs(input.actions) do
    local defMatch1 = act.defaultPrimary and input.matchesInput(act.defaultPrimary, key, isCtrl, isShift, isAlt)
    local defMatch2 = act.defaultSecondary and input.matchesInput(act.defaultSecondary, key, isCtrl, isShift, isAlt)
    if defMatch1 or defMatch2 then
      local curMatch1 = act.primaryKey and input.matchesInput(act.primaryKey, key, isCtrl, isShift, isAlt)
      local curMatch2 = act.secondaryKey and input.matchesInput(act.secondaryKey, key, isCtrl, isShift, isAlt)
      if not curMatch1 and not curMatch2 then
        wasReboundAway = true
        break
      end
    end
  end

  if not wasReboundAway and origKeyPressed then
    origKeyPressed(key)
  end
end

function input.handleKeyReleased(key)
  local origKeyReleased = input.origKeyReleased

  -- 1. INPUT CAPTURE RELEASE CHECK
  if input.capturingAction ~= nil then
    input.heldKeys[key] = nil
    if input.waitingForRelease then
      local anyHeld = false
      for _, v in pairs(input.heldKeys) do
        if v then anyHeld = true; break end
      end
      if not anyHeld then
        finalizeCapture()
      end
    end
    return
  end

  -- 2. Synthesize vanilla release triggers
  local actAlt = input.actionMap["alt_move"]
  if actAlt and (actAlt.primaryKey == key or actAlt.secondaryKey == key) and origKeyReleased then
    origKeyReleased("lalt")
    return
  end

  local actGrp = input.actionMap["grouping"]
  if actGrp and (actGrp.primaryKey == key or actGrp.secondaryKey == key) and origKeyReleased then
    origKeyReleased("lctrl")
    return
  end

  if origKeyReleased then
    origKeyReleased(key)
  end
end

function input.handleMousePressed(x, y, button, ...)
  local origMousePressed = input.origMousePressed

  -- 1. INPUT CAPTURE FOR MOUSE BUTTONS
  if input.capturingAction ~= nil then
    if button == 1 then
      -- Left click outside cancels capture
      return
    elseif button >= 2 then
      local btnKey = "mouse" .. button
      input.heldKeys[btnKey] = true
      input.waitingForRelease = true

      local already = false
      for _, k in ipairs(input.capturedCombo) do
        if k == btnKey then already = true; break end
      end
      if not already then
        table.insert(input.capturedCombo, btnKey)
      end
      return
    end
  end

  -- 2. DISPATCH ACTIONS BOUND TO MOUSE BUTTONS
  if button >= 2 then
    local btnKey = "mouse" .. button
    local origIsDown = input.origIsDown or (love and love.keyboard and love.keyboard.isDown)
    local isShift = origIsDown and (origIsDown("lshift") or origIsDown("rshift"))
    local isCtrl = origIsDown and (origIsDown("lctrl") or origIsDown("rctrl"))
    local isAlt = origIsDown and (origIsDown("lalt") or origIsDown("ralt"))
    local consumed = false

    for _, act in ipairs(input.actions) do
      local m1 = act.primaryKey and input.matchesInput(act.primaryKey, btnKey, isCtrl, isShift, isAlt)
      local m2 = act.secondaryKey and input.matchesInput(act.secondaryKey, btnKey, isCtrl, isShift, isAlt)
      if m1 or m2 then
        if act.onTrigger then
          local fn = input.resolveTrigger(act.onTrigger)
          if fn then
            consumed = true
            local ok, err = pcall(fn)
            if not ok then
              print("[QM.Input] Error executing action '" .. tostring(act.id) .. "': " .. tostring(err))
            end
          end
        elseif act.vanillaKey and input.origKeyPressed then
          consumed = true
          input.origKeyPressed(act.vanillaKey)
        end
      end
    end

    if consumed then return end
  end

  if origMousePressed then
    origMousePressed(x, y, button, ...)
  end
end

function input.handleMouseReleased(x, y, button, ...)
  local origMouseReleased = input.origMouseReleased

  if input.capturingAction ~= nil and button >= 2 then
    local btnKey = "mouse" .. button
    input.heldKeys[btnKey] = nil
    if input.waitingForRelease then
      local anyHeld = false
      for _, v in pairs(input.heldKeys) do
        if v then anyHeld = true; break end
      end
      if not anyHeld then
        finalizeCapture()
      end
    end
    return
  end

  if origMouseReleased then
    origMouseReleased(x, y, button, ...)
  end
end

function input.handleWheelMoved(x, y)
  if menuPage == input.PAGE_CONTROLS then
    local filtered = {}
    for _, act in ipairs(input.actions) do
      if act.tab == input.activeTab then
        table.insert(filtered, act)
      end
    end
    local totalPages = math.max(1, math.ceil(#filtered / input.visibleRows))

    if y > 0 then
      input.pageIndex = math.max(1, input.pageIndex - 1)
      playClickSound()
    elseif y < 0 then
      input.pageIndex = math.min(totalPages, input.pageIndex + 1)
      playClickSound()
    end
  end

  if input.origWheelMoved then
    input.origWheelMoved(x, y)
  end
end

function input.installHooks()
  if love and love.keyboard then
    if love.keyboard.isDown ~= input.handleIsDown then
      input.origIsDown = love.keyboard.isDown
      love.keyboard.isDown = input.handleIsDown
    end
  end

  if love then
    if love.keypressed ~= input.handleKeyPressed then
      input.origKeyPressed = love.keypressed
      love.keypressed = input.handleKeyPressed
    end

    if love.keyreleased ~= input.handleKeyReleased then
      input.origKeyReleased = love.keyreleased
      love.keyreleased = input.handleKeyReleased
    end

    if love.mousepressed ~= input.handleMousePressed then
      input.origMousePressed = love.mousepressed
      love.mousepressed = input.handleMousePressed
    end

    if love.mousereleased ~= input.handleMouseReleased then
      input.origMouseReleased = love.mousereleased
      love.mousereleased = input.handleMouseReleased
    end

    if love.wheelmoved ~= input.handleWheelMoved then
      input.origWheelMoved = love.wheelmoved
      love.wheelmoved = input.handleWheelMoved
    end
  end
end

-- ============================================================================
-- COMPILED CONFIG APPLICATION
-- ============================================================================

function input.isDown(actionId)
  return input.isActionDown(actionId)
end

function input.onTrigger(actionId, callback)
  local act = input.actionMap[actionId]
  if act then
    act.onTrigger = callback
  else
    input.pendingTriggers = input.pendingTriggers or {}
    input.pendingTriggers[actionId] = callback
  end
end

function input.trigger(actionId)
  local act = input.actionMap[actionId]
  if not act then return false end
  local fn = input.resolveTrigger(act.onTrigger)
  if fn then
    local ok, err = pcall(fn)
    if not ok then
      print("[QM.Input] Error executing action '" .. tostring(actionId) .. "': " .. tostring(err))
    end
    return ok
  end
  return false
end

function input.applyConfigs(compiledData)
  input.installHooks()
  if not compiledData then return end
  input.loadSavedKeybinds()

  for modId, cfg in pairs(compiledData) do
    if cfg.keybinds then
      for _, kb in ipairs(cfg.keybinds) do
        kb.modId = modId
        if not kb.tab or kb.tab == "modded" then
          kb.tab = "uncategorized"
        end
        if kb.on_trigger and not kb.onTrigger then
          kb.onTrigger = kb.on_trigger
        end
        if input.pendingTriggers and input.pendingTriggers[kb.id] then
          kb.onTrigger = input.pendingTriggers[kb.id]
        end
        input.registerAction(kb)
      end
    end
  end
end

-- ============================================================================
-- SCREEN 87: CONTROLS CONFIGURATION UI
-- ============================================================================

function _G.menu_controlsButton()
  playClickSound()
  input.previousPage = menuPage or 86
  menuPage = input.PAGE_CONTROLS
  input.pageIndex = 1
  input.capturingAction = nil
  input.capturingSlot = 1
  input.heldKeys = {}
  input.capturedCombo = {}
  input.waitingForRelease = false
  input.pendingConflict = nil
  mouseOverText = ""
  contextualText = ""
end

function input.registerControlsButtons()
  local visibleTabs = input.getVisibleTabs()
  local foundActive = false
  for _, t in ipairs(visibleTabs) do
    if t == input.activeTab then foundActive = true; break end
  end
  if not foundActive then
    input.activeTab = visibleTabs[1] or "vanilla"
  end

  buttons[#buttons + 1] = Button.new(32, 686, 60, 30, "qm_ctrl_back", "active", true, mouseOverBack)
  _G.qm_ctrl_backButton = function()
    playClickSound()
    menuPage = input.previousPage or 1
    mouseOverText = ""
    contextualText = ""
  end
  buttons[#buttons + 1] = Button.new(110, 686, 210, 30, "qm_reset_tab", "active", true, "Reset current tab to default keybindings")

  _G.qm_reset_tabButton = function()
    playClickSound()
    input.resetTab(input.activeTab)
  end

  -- Tab Bar Pagination Buttons
  local totalTabPages = math.max(1, math.ceil(#visibleTabs / input.tabsPerPage))
  if input.tabPageIndex > totalTabPages then input.tabPageIndex = totalTabPages end
  if input.tabPageIndex < 1 then input.tabPageIndex = 1 end

  local tabStartX = 30
  local tabY = 175
  local tabW = 145
  local tabH = 26
  local tabGap = 8

  if #visibleTabs > input.tabsPerPage then
    tabStartX = 65
    if input.tabPageIndex > 1 then
      buttons[#buttons + 1] = Button.new(30, tabY, 28, tabH, "qm_kbtab_prev", "active", true, "Previous controls tab page")
      _G.qm_kbtab_prevButton = function()
        playClickSound()
        input.tabPageIndex = math.max(1, input.tabPageIndex - 1)
      end
    end

    if input.tabPageIndex < totalTabPages then
      local nextX = tabStartX + input.tabsPerPage * (tabW + tabGap)
      buttons[#buttons + 1] = Button.new(nextX, tabY, 28, tabH, "qm_kbtab_next", "active", true, "Next controls tab page")
      _G.qm_kbtab_nextButton = function()
        playClickSound()
        input.tabPageIndex = math.min(totalTabPages, input.tabPageIndex + 1)
      end
    end
  end

  local tStart = (input.tabPageIndex - 1) * input.tabsPerPage + 1
  local tEnd = math.min(#visibleTabs, tStart + input.tabsPerPage - 1)

  for tIdx = tStart, tEnd do
    local tabKey = visibleTabs[tIdx]
    local tabName = input.tabNames[tabKey] or tabKey:upper()
    local relIdx = tIdx - tStart
    local tabX = tabStartX + relIdx * (tabW + tabGap)
    local btnLabel = "qm_tab_" .. tabKey

    buttons[#buttons + 1] = Button.new(tabX, tabY, tabW, tabH, btnLabel, "active", true, "Switch to " .. tabName)
    _G[btnLabel .. "Button"] = function()
      playClickSound()
      input.activeTab = tabKey
      input.pageIndex = 1
      input.capturingAction = nil
      input.capturingSlot = 1
      input.heldKeys = {}
      input.capturedCombo = {}
      input.waitingForRelease = false
      input.pendingConflict = nil
    end
  end

  -- Filter actions for active tab
  local filtered = {}
  for _, act in ipairs(input.actions) do
    if act.tab == input.activeTab then
      table.insert(filtered, act)
    end
  end

  local totalPages = math.max(1, math.ceil(#filtered / input.visibleRows))
  if input.pageIndex > totalPages then input.pageIndex = totalPages end
  if input.pageIndex < 1 then input.pageIndex = 1 end

  -- Action Rows
  local startY = 230
  local rowH = 34
  local startIndex = (input.pageIndex - 1) * input.visibleRows + 1
  local endIndex = math.min(#filtered, startIndex + input.visibleRows - 1)

  for i = startIndex, endIndex do
    local act = filtered[i]
    local relIndex = i - startIndex
    local rowY = startY + relIndex * rowH

    -- Primary Bind Slot Button (x = 460, width = 175)
    local slot1Label = "qm_bind1_" .. act.id
    buttons[#buttons + 1] = Button.new(460, rowY - 4, 175, 26, slot1Label, "active", true, "Click to reassign primary key (Esc to clear)")
    _G[slot1Label .. "Button"] = function()
      playClickSound()
      if input.capturingAction == act.id and input.capturingSlot == 1 then
        input.capturingAction = nil
        input.heldKeys = {}
        input.capturedCombo = {}
        input.waitingForRelease = false
      else
        input.capturingAction = act.id
        input.capturingSlot = 1
        input.heldKeys = {}
        input.capturedCombo = {}
        input.waitingForRelease = false
        input.pendingConflict = nil
      end
    end

    -- Secondary Bind Slot Button (x = 645, width = 175)
    local slot2Label = "qm_bind2_" .. act.id
    buttons[#buttons + 1] = Button.new(645, rowY - 4, 175, 26, slot2Label, "active", true, "Click to reassign secondary key (Esc to clear)")
    _G[slot2Label .. "Button"] = function()
      playClickSound()
      if input.capturingAction == act.id and input.capturingSlot == 2 then
        input.capturingAction = nil
        input.heldKeys = {}
        input.capturedCombo = {}
        input.waitingForRelease = false
      else
        input.capturingAction = act.id
        input.capturingSlot = 2
        input.heldKeys = {}
        input.capturedCombo = {}
        input.waitingForRelease = false
        input.pendingConflict = nil
      end
    end

    -- Reset Button (x = 830, width = 60)
    local resetLabel = "qm_rst_" .. act.id
    buttons[#buttons + 1] = Button.new(830, rowY - 4, 60, 26, resetLabel, "active", true, "Reset to default keys")
    _G[resetLabel .. "Button"] = function()
      playClickSound()
      input.resetAction(act.id)
    end
  end

  -- Action Rows Page Navigation Buttons (x = 460 to 730)
  if totalPages > 1 then
    if input.pageIndex > 1 then
      buttons[#buttons + 1] = Button.new(460, 686, 80, 30, "qm_page_prev", "active", true, "Previous controls page")
      _G.qm_page_prevButton = function()
        playClickSound()
        input.pageIndex = math.max(1, input.pageIndex - 1)
      end
    end

    if input.pageIndex < totalPages then
      buttons[#buttons + 1] = Button.new(650, 686, 80, 30, "qm_page_next", "active", true, "Next controls page")
      _G.qm_page_nextButton = function()
        playClickSound()
        input.pageIndex = math.min(totalPages, input.pageIndex + 1)
      end
    end
  end

  -- Conflict resolution buttons
  if input.pendingConflict ~= nil then
    buttons[#buttons + 1] = Button.new(410, 390, 130, 30, "qm_conf_replace", "active", true, "Rebind and remove from previous action")
    buttons[#buttons + 1] = Button.new(550, 390, 160, 30, "qm_conf_both", "active", true, "Keep both actions bound to this key")
    buttons[#buttons + 1] = Button.new(720, 390, 110, 30, "qm_conf_cancel", "active", true, "Cancel key assignment")

    _G.qm_conf_replaceButton = function()
      playClickSound()
      local p = input.pendingConflict
      if p then
        input.clearBindingSlot(p.conflictingId, p.conflictingSlot)
        input.setBindingSlot(p.actionId, p.slot, p.newKey, false)
      end
      input.pendingConflict = nil
    end

    _G.qm_conf_bothButton = function()
      playClickSound()
      local p = input.pendingConflict
      if p then
        input.setBindingSlot(p.actionId, p.slot, p.newKey, true)
      end
      input.pendingConflict = nil
    end

    _G.qm_conf_cancelButton = function()
      playClickSound()
      input.pendingConflict = nil
    end
  end
end

function input.drawControlsMenu()
  local fade = uiFade or 255
  local mx, my = love.mouse.getPosition()
  local visibleTabs = input.getVisibleTabs()

  drawCleanShadowText("CONTROLS CONFIGURATION", 30, 115, 600, "left", bigFont)
  if nonoFont then
    love.graphics.setFont(nonoFont)
    love.graphics.setColor(love.math.colorFromBytes(180, 180, 180, fade))
    love.graphics.print("CUSTOMIZE KEYBOARD SHORTCUTS & INPUT BINDINGS", 32, 145)
  end

  -- Tab Bar
  local totalTabPages = math.max(1, math.ceil(#visibleTabs / input.tabsPerPage))
  local tabStartX = 30
  local tabY = 175
  local tabW = 145
  local tabH = 26
  local tabGap = 8

  if #visibleTabs > input.tabsPerPage then
    tabStartX = 65
    -- Left tab arrow
    if input.tabPageIndex > 1 then
      local prevHov = (mx >= 30 and mx <= 58 and my >= tabY and my <= tabY + tabH)
      love.graphics.setColor(love.math.colorFromBytes(prevHov and 101 or 35, prevHov and 24 or 35, prevHov and 24 or 35, fade))
      love.graphics.rectangle("fill", 30, tabY, 28, tabH)
      love.graphics.setColor(love.math.colorFromBytes(180, 180, 180, fade))
      love.graphics.rectangle("line", 30, tabY, 28, tabH)
      drawCleanShadowText("<", 30, tabY + 4, 28, "center", cardFont or bigFont, { 255, 255, 255, fade })
    end

    -- Right tab arrow
    if input.tabPageIndex < totalTabPages then
      local nextX = tabStartX + input.tabsPerPage * (tabW + tabGap)
      local nextHov = (mx >= nextX and mx <= nextX + 28 and my >= tabY and my <= tabY + tabH)
      love.graphics.setColor(love.math.colorFromBytes(nextHov and 101 or 35, nextHov and 24 or 35, nextHov and 24 or 35, fade))
      love.graphics.rectangle("fill", nextX, tabY, 28, tabH)
      love.graphics.setColor(love.math.colorFromBytes(180, 180, 180, fade))
      love.graphics.rectangle("line", nextX, tabY, 28, tabH)
      drawCleanShadowText(">", nextX, tabY + 4, 28, "center", cardFont or bigFont, { 255, 255, 255, fade })
    end
  end

  local tStart = (input.tabPageIndex - 1) * input.tabsPerPage + 1
  local tEnd = math.min(#visibleTabs, tStart + input.tabsPerPage - 1)

  for tIdx = tStart, tEnd do
    local tabKey = visibleTabs[tIdx]
    local tabName = input.tabNames[tabKey] or tabKey:upper()
    local relIdx = tIdx - tStart
    local tabX = tabStartX + relIdx * (tabW + tabGap)
    local isSelected = (input.activeTab == tabKey)
    local hovered = (mx >= tabX and mx <= tabX + tabW and my >= tabY and my <= tabY + tabH)

    if isSelected then
      love.graphics.setColor(love.math.colorFromBytes(101, 24, 24, fade))
      love.graphics.rectangle("fill", tabX, tabY, tabW, tabH)
      love.graphics.setColor(love.math.colorFromBytes(255, 255, 255, fade))
    elseif hovered then
      love.graphics.setColor(love.math.colorFromBytes(40, 40, 40, fade))
      love.graphics.rectangle("fill", tabX, tabY, tabW, tabH)
      love.graphics.setColor(love.math.colorFromBytes(220, 220, 220, fade))
    else
      love.graphics.setColor(love.math.colorFromBytes(20, 20, 20, fade))
      love.graphics.rectangle("fill", tabX, tabY, tabW, tabH)
      love.graphics.setColor(love.math.colorFromBytes(160, 160, 160, fade))
    end
    love.graphics.rectangle("line", tabX, tabY, tabW, tabH)
    drawCleanShadowText(tabName, tabX + 6, tabY + 4, tabW - 12, "center", cardFont)
  end

  -- Table Column Headers
  local hdrY = 208
  if nonoFont then
    love.graphics.setFont(nonoFont)
    love.graphics.setColor(love.math.colorFromBytes(150, 150, 150, fade))
    love.graphics.print("CATEGORY", 35, hdrY)
    love.graphics.print("ACTION", 145, hdrY)
    love.graphics.print("PRIMARY BIND", 465, hdrY)
    love.graphics.print("SECONDARY BIND", 650, hdrY)
    love.graphics.print("RESET", 835, hdrY)
  end

  -- Action Rows
  local filtered = {}
  for _, act in ipairs(input.actions) do
    if act.tab == input.activeTab then
      table.insert(filtered, act)
    end
  end

  local totalPages = math.max(1, math.ceil(#filtered / input.visibleRows))
  local startY = 230
  local rowH = 34
  local startIndex = (input.pageIndex - 1) * input.visibleRows + 1
  local endIndex = math.min(#filtered, startIndex + input.visibleRows - 1)

  for i = startIndex, endIndex do
    local act = filtered[i]
    local relIndex = i - startIndex
    local rowY = startY + relIndex * rowH

    -- Category badge
    local catText = "[" .. (act.category or "General"):upper() .. "]"
    if nonoFont then
      love.graphics.setFont(nonoFont)
      love.graphics.setColor(love.math.colorFromBytes(140, 140, 140, fade))
      love.graphics.print(catText, 35, rowY + 3)
    end

    -- Action Name
    drawCleanShadowText(act.name, 145, rowY + 1, 305, "left", cardFont)

    -- Primary Bind Slot (x = 460, w = 175)
    local isCapturing1 = (input.capturingAction == act.id and input.capturingSlot == 1)
    local slot1Hovered = (mx >= 460 and mx <= 635 and my >= rowY - 4 and my <= rowY + 22)
    local key1Text
    if isCapturing1 then
      if input.waitingForRelease and #input.capturedCombo > 0 then
        key1Text = "REL: " .. input.formatKey(input.buildComboString(input.capturedCombo))
      else
        key1Text = "[PRESS KEY]"
      end
    else
      key1Text = input.formatKey(act.primaryKey)
    end

    if isCapturing1 then
      love.graphics.setColor(love.math.colorFromBytes(101, 24, 24, fade))
    elseif slot1Hovered then
      love.graphics.setColor(love.math.colorFromBytes(60, 60, 60, fade))
    else
      love.graphics.setColor(love.math.colorFromBytes(30, 30, 30, fade))
    end
    love.graphics.rectangle("fill", 460, rowY - 4, 175, 26)
    love.graphics.setColor(love.math.colorFromBytes(180, 180, 180, fade))
    love.graphics.rectangle("line", 460, rowY - 4, 175, 26)
    local text1Color = isCapturing1 and { 255, 255, 100, fade } or { 255, 255, 255, fade }
    drawCleanShadowText(key1Text, 465, rowY + 1, 165, "center", cardFont, text1Color)

    -- Secondary Bind Slot (x = 645, w = 175)
    local isCapturing2 = (input.capturingAction == act.id and input.capturingSlot == 2)
    local slot2Hovered = (mx >= 645 and mx <= 820 and my >= rowY - 4 and my <= rowY + 22)
    local key2Text
    if isCapturing2 then
      if input.waitingForRelease and #input.capturedCombo > 0 then
        key2Text = "REL: " .. input.formatKey(input.buildComboString(input.capturedCombo))
      else
        key2Text = "[PRESS KEY]"
      end
    else
      key2Text = input.formatKey(act.secondaryKey)
    end

    if isCapturing2 then
      love.graphics.setColor(love.math.colorFromBytes(101, 24, 24, fade))
    elseif slot2Hovered then
      love.graphics.setColor(love.math.colorFromBytes(60, 60, 60, fade))
    else
      love.graphics.setColor(love.math.colorFromBytes(30, 30, 30, fade))
    end
    love.graphics.rectangle("fill", 645, rowY - 4, 175, 26)
    love.graphics.setColor(love.math.colorFromBytes(180, 180, 180, fade))
    love.graphics.rectangle("line", 645, rowY - 4, 175, 26)
    local text2Color = isCapturing2 and { 255, 255, 100, fade } or { 255, 255, 255, fade }
    drawCleanShadowText(key2Text, 650, rowY + 1, 165, "center", cardFont, text2Color)

    -- Reset Button (x = 830, w = 60)
    local resetHovered = (mx >= 830 and mx <= 890 and my >= rowY - 4 and my <= rowY + 22)
    love.graphics.setColor(love.math.colorFromBytes(30, 30, 30, fade))
    love.graphics.rectangle("fill", 830, rowY - 4, 60, 26)
    love.graphics.setColor(love.math.colorFromBytes(120, 120, 120, fade))
    love.graphics.rectangle("line", 830, rowY - 4, 60, 26)
    local rstColor = resetHovered and { 255, 255, 255, fade } or { 160, 160, 160, fade }
    drawCleanShadowText("RESET", 830, rowY + 1, 60, "center", nonoFont or cardFont, rstColor)

    -- Conflict / Shared badge
    if act.overrideConflict then
      if nonoFont then
        love.graphics.setFont(nonoFont)
        love.graphics.setColor(love.math.colorFromBytes(220, 180, 50, fade))
        love.graphics.print("[SHARED]", 905, rowY + 3)
      end
    end
  end

  -- Bottom Navigation Bar
  local backHovered = (mx >= 30 and mx <= 100 and my >= 680 and my <= 715)
  local backColor = backHovered and { 101, 24, 24, fade } or { 255, 255, 255, fade }
  drawCleanShadowText("Back", 35, 685, 100, "left", bigFont, backColor)

  local rstTabHovered = (mx >= 110 and mx <= 320 and my >= 680 and my <= 715)
  local rstTabColor = rstTabHovered and { 101, 24, 24, fade } or { 211, 211, 211, fade }
  drawCleanShadowText("Reset Tab to Defaults", 110, 685, 220, "left", cardFont, rstTabColor)

  -- Action row pagination (x = 460 to 730)
  if totalPages > 1 then
    local prevHov = (mx >= 460 and mx <= 540 and my >= 686 and my <= 716 and input.pageIndex > 1)
    love.graphics.setColor(love.math.colorFromBytes(prevHov and 101 or 35, prevHov and 24 or 35, prevHov and 24 or 35, fade))
    love.graphics.rectangle("fill", 460, 686, 80, 26)
    love.graphics.setColor(love.math.colorFromBytes(180, 180, 180, fade))
    love.graphics.rectangle("line", 460, 686, 80, 26)
    local prevTextColor = input.pageIndex > 1 and { 255, 255, 255, fade } or { 120, 120, 120, fade }
    drawCleanShadowText("< PREV", 460, 690, 80, "center", cardFont, prevTextColor)

    local pageLabel = string.format("PAGE %d / %d", input.pageIndex, totalPages)
    drawCleanShadowText(pageLabel, 545, 690, 100, "center", cardFont, { 211, 211, 211, fade })

    local nextHov = (mx >= 650 and mx <= 730 and my >= 686 and my <= 716 and input.pageIndex < totalPages)
    love.graphics.setColor(love.math.colorFromBytes(nextHov and 101 or 35, nextHov and 24 or 35, nextHov and 24 or 35, fade))
    love.graphics.rectangle("fill", 650, 686, 80, 26)
    love.graphics.setColor(love.math.colorFromBytes(180, 180, 180, fade))
    love.graphics.rectangle("line", 650, 686, 80, 26)
    local nextTextColor = input.pageIndex < totalPages and { 255, 255, 255, fade } or { 120, 120, 120, fade }
    drawCleanShadowText("NEXT >", 650, 690, 80, "center", cardFont, nextTextColor)
  end

  if nonoFont then
    love.graphics.setFont(nonoFont)
    love.graphics.setColor(love.math.colorFromBytes(150, 150, 150, fade))
    love.graphics.print("CLICK BOX TO BIND. ESC CLEARS. RELEASE ALL KEYS TO CONFIRM.", 750, 692)
  end

  -- Conflict Modal Dialog
  if input.pendingConflict ~= nil then
    local p = input.pendingConflict
    local act = input.actionMap[p.actionId]
    local confAct = input.actionMap[p.conflictingId]
    local keyName = input.formatKey(p.newKey)

    love.graphics.setColor(love.math.colorFromBytes(0, 0, 0, 220))
    love.graphics.rectangle("fill", 0, 0, 1280, 720)

    love.graphics.setColor(love.math.colorFromBytes(25, 25, 25, 255))
    love.graphics.rectangle("fill", 370, 250, 540, 200)
    love.graphics.setColor(love.math.colorFromBytes(101, 24, 24, 255))
    love.graphics.rectangle("line", 370, 250, 540, 200)

    drawCleanShadowText("KEY CONFLICT DETECTED", 390, 270, 500, "left", bigFont, { 255, 100, 100, 255 })
    local msg = string.format("Key '%s' is already assigned to '%s'.\nWould you like to replace the existing binding or keep both?", keyName, confAct and confAct.name or "Another Action")
    drawCleanShadowText(msg, 390, 310, 500, "left", cardFont)

    local rHover = (mx >= 410 and mx <= 540 and my >= 390 and my <= 420)
    love.graphics.setColor(love.math.colorFromBytes(rHover and 101 or 40, rHover and 24 or 40, rHover and 24 or 40, 255))
    love.graphics.rectangle("fill", 410, 390, 130, 30)
    love.graphics.setColor(love.math.colorFromBytes(200, 200, 200, 255))
    love.graphics.rectangle("line", 410, 390, 130, 30)
    drawCleanShadowText("REPLACE", 410, 396, 130, "center", cardFont)

    local bHover = (mx >= 550 and mx <= 710 and my >= 390 and my <= 420)
    love.graphics.setColor(love.math.colorFromBytes(bHover and 101 or 40, bHover and 24 or 40, bHover and 24 or 40, 255))
    love.graphics.rectangle("fill", 550, 390, 160, 30)
    love.graphics.setColor(love.math.colorFromBytes(200, 200, 200, 255))
    love.graphics.rectangle("line", 550, 390, 160, 30)
    drawCleanShadowText("KEEP BOTH", 550, 396, 160, "center", cardFont)

    local cHover = (mx >= 720 and mx <= 830 and my >= 390 and my <= 420)
    love.graphics.setColor(love.math.colorFromBytes(cHover and 101 or 40, cHover and 24 or 40, cHover and 24 or 40, 255))
    love.graphics.rectangle("fill", 720, 390, 110, 30)
    love.graphics.setColor(love.math.colorFromBytes(200, 200, 200, 255))
    love.graphics.rectangle("line", 720, 390, 110, 30)
    drawCleanShadowText("CANCEL", 720, 396, 110, "center", cardFont)
  end
end

-- Initialize hooks immediately upon module load
input.installHooks()
