-- ============================================================================
-- menu_runtime.lua — Built-in QuarterMaster Menu & Settings Framework
-- Standardized UI engine for Mod Operations (85), Mod Options (86), and Briefing Pages (100+)
-- ============================================================================

_G.QM = _G.QM or {}
local menu = {}
_G.QM.Menu = menu
_G.QM.settings = _G.QM.settings or {}
_G.modSettings = _G.modSettings or {}

-- Screen IDs
menu.PAGE_MOD_MODES = 85
menu.PAGE_MOD_OPTIONS = 86

-- Dynamic Page Allocator & Registry (Assigns collision-free IDs 100+)
menu.nextDynamicPage = 100
menu.customPages = {}      -- pageNumber -> { number, id, modId, draw, registerButtons }
menu.pageLookup = {}       -- "modId:pageId" -> pageNumber

-- Storage tables
menu.menuButtons = {}     -- Array of custom game mode cards
menu.optionsByMod = {}    -- modId -> { id, name, options }
menu.modList = {}         -- Ordered list of modIds
menu.activeModIndex = 1   -- Selected tab on Screen 86
menu.savedSettings = {}   -- Persistent table saved to saveModSettings.lua

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

-- Clean text drawing with 1px drop shadow and dynamic font downscaling
local function drawCleanShadowText(text, x, y, limit, align, font, textColor)
  local f = font or cardFont or bigFont
  local fade = uiFade or 255
  local tc = textColor or { 255, 255, 255, fade }

  if limit and f and type(f.getWidth) == "function" then
    if f:getWidth(text) > limit then
      if f == bigFont and cardFont and cardFont:getWidth(text) <= limit then
        f = cardFont
      elseif nonoFont then
        f = nonoFont
      end
    end
  end

  if f then love.graphics.setFont(f) end
  love.graphics.printf({{love.math.colorFromBytes(0, 0, 0, math.min(180, fade))}, text}, x + 1, y + 1, limit or 200, align or "left")
  love.graphics.printf({{love.math.colorFromBytes(tc[1], tc[2], tc[3], tc[4] or fade)}, text}, x, y, limit or 200, align or "left")
end

-- ============================================================================
-- PERSISTENCE: saveModSettings.lua
-- ============================================================================

function menu.loadSavedSettings()
  if love and love.filesystem and love.filesystem.getInfo and love.filesystem.getInfo("saveModSettings.lua") then
    local ok_read, raw = pcall(love.filesystem.read, "saveModSettings.lua")
    if ok_read and type(raw) == "string" then
      if raw:sub(1, 14) == "return local _" then
        raw = raw:sub(8)
      end
      local loader = loadstring or load
      local ok, chunk = pcall(loader, raw, "saveModSettings.lua")
      if ok and type(chunk) == "function" then
        local ok2, data = pcall(chunk)
        if ok2 and type(data) == "table" then
          menu.savedSettings = data
        end
      end
    end
  end

  -- Mirror saved settings into live QM.settings and modSettings
  for modId, modTbl in pairs(menu.savedSettings) do
    if type(modTbl) == "table" then
      _G.QM.settings[modId] = _G.QM.settings[modId] or {}
      for optId, val in pairs(modTbl) do
        _G.QM.settings[modId][optId] = val
        _G.modSettings[optId] = val
      end
    end
  end
  if player and _G.QM and _G.QM.syncToPlayer then
    _G.QM.syncToPlayer(player)
  end
end

function menu.saveCurrentSettings()
  if love and love.filesystem then
    local ser = require "includes/ser"
    if ser then
      local ok, str = pcall(ser, menu.savedSettings)
      if ok and str then
        if not str:match("^local _=") and not str:match("^return ") then
          str = "return " .. str
        end
        love.filesystem.write("saveModSettings.lua", str)
      end
    end
  end
end

-- Global delegation bridges for legacy call sites
_G.loadModSettings = function()
  if menu and menu.loadSavedSettings then
    menu.loadSavedSettings()
  end
  if player and _G.QM and _G.QM.syncToPlayer then
    _G.QM.syncToPlayer(player)
  end
end

_G.saveModSettings = function()
  if menu and menu.saveCurrentSettings then
    menu.saveCurrentSettings()
  end
end

-- ============================================================================
-- DYNAMIC PAGE ALLOCATION
-- ============================================================================

function menu.allocatePage(modId, pageId, drawFn, registerButtonsFn)
  local key = (modId or "global") .. ":" .. (pageId or tostring(menu.nextDynamicPage))
  if menu.pageLookup[key] then
    local existingNum = menu.pageLookup[key]
    if drawFn then menu.customPages[existingNum].draw = drawFn end
    if registerButtonsFn then menu.customPages[existingNum].registerButtons = registerButtonsFn end
    return existingNum
  end

  local pageNum = menu.nextDynamicPage
  menu.nextDynamicPage = menu.nextDynamicPage + 1

  local entry = {
    number = pageNum,
    id = pageId,
    modId = modId,
    draw = drawFn,
    registerButtons = registerButtonsFn
  }
  menu.customPages[pageNum] = entry
  menu.pageLookup[key] = pageNum
  return pageNum
end

function menu.getPage(modId, pageId)
  local key = (modId or "global") .. ":" .. (pageId or "")
  return menu.pageLookup[key]
end

-- ============================================================================
-- OPTION & MODE REGISTRATION & AUTOMATIC SETTINGS DISPATCH
-- ============================================================================

local function snakeToCamel(s)
  if type(s) ~= "string" then return s end
  return (s:gsub("_(%a)", function(c) return c:upper() end))
end

local function getOptionAliases(opt)
  local names = {}
  local seen = {}
  local function add(n)
    if n and type(n) == "string" and #n > 0 and not seen[n] then
      seen[n] = true
      table.insert(names, n)
    end
  end

  add(opt.id)
  local camel = snakeToCamel(opt.id)
  add(camel)
  add(camel .. "Enabled")
  add(opt.id .. "Enabled")

  if opt.aliases and type(opt.aliases) == "table" then
    for _, a in ipairs(opt.aliases) do
      add(a)
    end
  end

  return names
end

local function applyOptionToTable(tbl, prefix, opt, val)
  if not tbl or val == nil then return end
  prefix = prefix or ""
  local aliases = getOptionAliases(opt)
  for _, name in ipairs(aliases) do
    local key = prefix .. name
    tbl[key] = val

    if type(val) == "boolean" then
      tbl[key] = val and 1 or 0
    elseif val == "OFF" or val == "off" then
      if name:find("Enabled$") then
        tbl[key] = 0
      elseif name:find("Threshold$") or name:find("Limit$") then
        tbl[key] = 0
      end
    elseif type(val) == "number" then
      if name:find("Enabled$") then
        tbl[key] = 1
      elseif name:find("Threshold$") or name:find("Limit$") then
        tbl[key] = val
      end
    end
  end
end

function _G.QM.syncToPlayer(p)
  if not p or type(p) ~= "table" then return end
  for modId, modData in pairs(menu.optionsByMod) do
    if modData and modData.options then
      for _, opt in ipairs(modData.options) do
        local val = menu.getOptionValue(modId, opt)
        if val ~= nil then
          applyOptionToTable(p, "option_", opt, val)
        end
      end
    end
  end
end

function _G.QM.syncBattleGlobals()
  for modId, modData in pairs(menu.optionsByMod) do
    if modData and modData.options then
      for _, opt in ipairs(modData.options) do
        local val = menu.getOptionValue(modId, opt)
        if val ~= nil then
          applyOptionToTable(_G, "mnb3Tomnb2_", opt, val)
        end
      end
    end
  end
end

local function registerOptionButtonHandlers(modId, opt)
  local function trigger()
    local cur = menu.getOptionValue(modId, opt)
    local optType = opt.option_type or opt.type
    if optType == "checkbox" or (not optType and (type(opt.default) == "boolean" or opt.default == 0 or opt.default == 1)) then
      local nVal
      if type(cur) == "boolean" then
        nVal = not cur
      elseif cur == 1 then
        nVal = 0
      else
        nVal = 1
      end
      menu.setOptionValue(modId, opt, nVal)
    elseif optType == "cycle" and opt.options and #opt.options > 0 then
      local curIdx = 1
      for i, choice in ipairs(opt.options) do
        if choice == cur then curIdx = i; break end
      end
      local nextIdx = (curIdx % #opt.options) + 1
      menu.setOptionValue(modId, opt, opt.options[nextIdx])
    elseif optType == "button" and opt.on_click then
      if type(opt.on_click) == "function" then
        pcall(opt.on_click)
      elseif type(opt.on_click) == "string" and _G[opt.on_click] then
        pcall(_G[opt.on_click])
      end
    end
  end

  for _, name in ipairs(getOptionAliases(opt)) do
    local fnName = "op_" .. name .. "Button"
    if not rawget(_G, fnName) then
      _G[fnName] = trigger
    end
  end
end

function menu.registerOption(modId, modName, def)
  if not modId or not def or not def.id or not def.label then return end

  if not menu.optionsByMod[modId] then
    menu.optionsByMod[modId] = {
      id = modId,
      name = modName or modId,
      options = {}
    }
    table.insert(menu.modList, modId)
  end

  local modEntry = menu.optionsByMod[modId]
  local found = false
  for i, opt in ipairs(modEntry.options) do
    if opt.id == def.id then
      modEntry.options[i] = def
      found = true
      break
    end
  end
  if not found then
    table.insert(modEntry.options, def)
  end

  _G.QM.settings[modId] = _G.QM.settings[modId] or {}
  menu.savedSettings[modId] = menu.savedSettings[modId] or {}

  -- Determine active setting value: saved > default
  local initialVal = def.default
  if menu.savedSettings[modId][def.id] ~= nil then
    initialVal = menu.savedSettings[modId][def.id]
  else
    if def.default ~= nil then
      menu.savedSettings[modId][def.id] = def.default
    end
  end

  _G.QM.settings[modId][def.id] = initialVal
  applyOptionToTable(_G.modSettings, "", def, initialVal)

  -- Auto-generate button callbacks in _G
  registerOptionButtonHandlers(modId, def)

  if player and initialVal ~= nil then
    applyOptionToTable(player, "option_", def, initialVal)
  end

  if def.set and initialVal ~= nil then
    pcall(def.set, initialVal)
  end
end

function menu.getOptionValue(modId, opt)
  if _G.QM.settings[modId] and _G.QM.settings[modId][opt.id] ~= nil then
    return _G.QM.settings[modId][opt.id]
  end
  if opt.get then
    local ok, val = pcall(opt.get)
    if ok and val ~= nil then return val end
  end
  return opt.default
end

function menu.setOptionValue(modId, opt, newVal)
  _G.QM.settings[modId] = _G.QM.settings[modId] or {}
  _G.QM.settings[modId][opt.id] = newVal
  applyOptionToTable(_G.modSettings, "", opt, newVal)

  if player then
    applyOptionToTable(player, "option_", opt, newVal)
  end

  menu.savedSettings[modId] = menu.savedSettings[modId] or {}
  menu.savedSettings[modId][opt.id] = newVal
  menu.saveCurrentSettings()

  if opt.set then
    pcall(opt.set, newVal)
  end
end

function menu.getVisibleOptions(modId)
  local modData = menu.optionsByMod[modId]
  if not modData or not modData.options then return {} end
  local list = {}
  for _, opt in ipairs(modData.options) do
    local isVis = true
    if opt.show_if then
      local rule = opt.show_if
      local targetId = rule.option_id or rule.id
      if targetId then
        local depVal = _G.QM.settings[modId] and _G.QM.settings[modId][targetId]
        if rule.equals and type(rule.equals) == "table" and #rule.equals > 0 then
          isVis = false
          for _, allowed in ipairs(rule.equals) do
            if depVal == allowed then isVis = true; break end
          end
        elseif rule.value ~= nil then
          isVis = (depVal == rule.value)
        end
      end
    end
    if isVis then
      table.insert(list, opt)
    end
  end
  return list
end

function menu.registerMenuButton(def)
  if not def or not def.id or not def.title then return end

  for i, btn in ipairs(menu.menuButtons) do
    if btn.id == def.id then
      menu.menuButtons[i] = def
      return
    end
  end

  def.priority = def.priority or 50

  -- Load image if provided
  if type(def.icon) == "string" then
    local candidates = {
      def.icon,
      (def.modDir and (def.modDir .. "/" .. def.icon)),
      ("Mods/" .. (def.modId or "") .. "/" .. def.icon)
    }
    local img = nil
    for _, p in ipairs(candidates) do
      if p and not img then
        if love.filesystem and love.filesystem.getInfo and love.filesystem.getInfo(p) then
          local ok, data = pcall(love.filesystem.newFileData, p)
          if ok and data then
            local imgOk, loaded = pcall(love.graphics.newImage, data)
            if imgOk then img = loaded; break end
          end
        end
        local f = io.open(p, "rb")
        if f then
          local bytes = f:read("*a")
          f:close()
          if bytes and #bytes > 0 then
            local ok, data = pcall(love.filesystem.newFileData, bytes, def.id .. "_icon.png")
            if ok and data then
              local imgOk, loaded = pcall(love.graphics.newImage, data)
              if imgOk then img = loaded; break end
            end
          end
        end
      end
    end
    def.iconImage = img
  elseif type(def.icon) == "userdata" then
    def.iconImage = def.icon
  end

  table.insert(menu.menuButtons, def)
  table.sort(menu.menuButtons, function(a, b) return (a.priority or 50) > (b.priority or 50) end)
end

function menu.registerPage(def)
  if not def or not def.id or not def.title then return end
  local modId = def.modId or "global"
  local pageNum

  local function drawPage()
    local fade = uiFade or 255
    local mx, my = love.mouse.getPosition()

    -- Title
    drawCleanShadowText(def.title:upper(), 30, 130, 800, "left", bigFont)

    -- Back Button
    local backHovered = (mx >= 30 and mx <= 100 and my >= 680 and my <= 715)
    local backColor = backHovered and { 101, 24, 24, fade } or { 255, 255, 255, fade }
    drawCleanShadowText("Back", 35, 685, 100, "left", bigFont, backColor)

    -- Action Button (if defined)
    if def.action_label then
      local actHovered = (mx >= 980 and mx <= 1240 and my >= 680 and my <= 715)
      local actColor = actHovered and { 101, 24, 24, fade } or { 211, 211, 211, fade }
      drawCleanShadowText(def.action_label:upper(), 980, 685, 260, "right", bigFont, actColor)
    end

    -- Content card
    love.graphics.setColor(love.math.colorFromBytes(255, 255, 255, 120))
    love.graphics.rectangle("fill", 50, 185, 1180, 470)
    love.graphics.setColor(love.math.colorFromBytes(0, 0, 0, fade))
    love.graphics.rectangle("line", 50, 185, 1180, 470)

    if def.content and cardFont then
      love.graphics.setFont(cardFont)
      love.graphics.setColor(love.math.colorFromBytes(0, 0, 0, fade))
      love.graphics.printf(def.content, 75, 210, 1130, "left")
    end
  end

  local function registerButtons()
    -- Back button -> Screen 85
    buttons[#buttons + 1] = Button.new(32, 686, 60, 30, "menu_custom_back", "active", true, mouseOverBack)
    _G.menu_custom_backButton = function()
      playClickSound()
      menuPage = menu.PAGE_MOD_MODES
      mouseOverText = ""
      contextualText = ""
    end

    -- Action button
    if def.action_label and def.action_on_click then
      buttons[#buttons + 1] = Button.new(980, 686, 260, 30, "menu_custom_action", "active", true, def.action_label)
      _G.menu_custom_actionButton = function()
        playClickSound()
        if type(def.action_on_click) == "function" then
          pcall(def.action_on_click)
        elseif type(def.action_on_click) == "string" and _G[def.action_on_click] then
          pcall(_G[def.action_on_click])
        end
      end
    end
  end

  pageNum = menu.allocatePage(modId, def.id, drawPage, registerButtons)
  return pageNum
end

-- ============================================================================
-- COMPILED CONFIG APPLICATION
-- ============================================================================

function menu.applyConfigs(compiledData)
  if not compiledData then return end

  -- Load persisted settings first
  menu.loadSavedSettings()

  -- Register mod configs
  for modId, cfg in pairs(compiledData) do
    local modName = cfg.name or modId

    -- Options
    if cfg.options then
      for _, opt in ipairs(cfg.options) do
        menu.registerOption(modId, modName, opt)
      end
    end

    -- Modes
    if cfg.modes then
      for _, mode in ipairs(cfg.modes) do
        mode.modId = modId
        if mode.on_click and type(mode.on_click) == "string" and mode.on_click:sub(1, 5) == "page:" then
          local targetPageId = mode.on_click:sub(6)
          mode.onClick = function()
            local pNum = menu.getPage(modId, targetPageId)
            if pNum then
              menuPage = pNum
            end
          end
        elseif mode.on_click and type(mode.on_click) == "string" and _G[mode.on_click] then
          mode.onClick = _G[mode.on_click]
        end
        menu.registerMenuButton(mode)
      end
    end

    -- Pages
    if cfg.pages then
      for _, page in ipairs(cfg.pages) do
        page.modId = modId
        menu.registerPage(page)
      end
    end
  end

  if player and _G.QM and _G.QM.syncToPlayer then
    _G.QM.syncToPlayer(player)
  end
end

-- ============================================================================
-- MAIN MENU INTEGRATION (Page 1)
-- ============================================================================

function menu.registerMainMenuButton()
  local curX = (player and player.bloop == 1 and lazarus == 1) and 310 or 120
  local hasModes = menu.menuButtons and #menu.menuButtons > 0
  local hasOptions = menu.modList and #menu.modList > 0

  if hasModes then
    buttons[#buttons + 1] = Button.new(curX, 686, 150, 30, "menu_modOps", "active", true, "Browse and launch custom mod game modes")
    curX = curX + 150 + 20
  end

  if hasOptions then
    buttons[#buttons + 1] = Button.new(curX, 686, 135, 30, "menu_modOptions", "active", true, "Configure installed mod options and settings")
    curX = curX + 135 + 20
  end

  -- Keybinds is a dedicated separate button at the bottom
  buttons[#buttons + 1] = Button.new(curX, 686, 100, 30, "menu_controls", "active", true, "Configure keyboard shortcuts and controls")
end

function _G.menu_modOpsButton()
  playClickSound()
  menuPage = menu.PAGE_MOD_MODES
  mouseOverText = ""
  contextualText = ""
end

function _G.menu_modOptionsButton()
  playClickSound()
  menuPage = menu.PAGE_MOD_OPTIONS
  mouseOverText = ""
  contextualText = ""
end

function _G.menu_controlsButton()
  playClickSound()
  if _G.QM and _G.QM.Input then
    _G.QM.Input.previousPage = menuPage or 1
    _G.QM.Input.pageIndex = 1
    _G.QM.Input.capturingAction = nil
    _G.QM.Input.capturingSlot = 1
    _G.QM.Input.heldKeys = {}
    _G.QM.Input.capturedCombo = {}
    _G.QM.Input.waitingForRelease = false
    _G.QM.Input.pendingConflict = nil
  end
  menuPage = (_G.QM and _G.QM.Input and _G.QM.Input.PAGE_CONTROLS) or 87
  mouseOverText = ""
  contextualText = ""
end

function menu.drawMainMenuButton()
  local mx, my = love.mouse.getPosition()
  local curX = (player and player.bloop == 1 and lazarus == 1) and 310 or 120
  local by = 685
  local bh = 30
  local gap = 20
  local fade = uiFade or 255

  local hasModes = menu.menuButtons and #menu.menuButtons > 0
  local hasOptions = menu.modList and #menu.modList > 0

  if hasModes then
    local bw = 150
    local hovered = (mx >= curX and mx <= curX + bw and my >= by - 5 and my <= by + bh)
    local textColor = hovered and { 101, 24, 24, fade } or { 211, 211, 211, fade }
    drawCleanShadowText("Mod Operations", curX, by, bw, "left", bigFont, textColor)
    curX = curX + bw + gap
  end

  if hasOptions then
    local bw = 135
    local hovered = (mx >= curX and mx <= curX + bw and my >= by - 5 and my <= by + bh)
    local textColor = hovered and { 101, 24, 24, fade } or { 211, 211, 211, fade }
    drawCleanShadowText("Mod Settings", curX, by, bw, "left", bigFont, textColor)
    curX = curX + bw + gap
  end

  -- Keybinds is a dedicated separate button at the bottom
  local bw = 100
  local hovered = (mx >= curX and mx <= curX + bw and my >= by - 5 and my <= by + bh)
  local textColor = hovered and { 101, 24, 24, fade } or { 211, 211, 211, fade }
  drawCleanShadowText("Keybinds", curX, by, bw, "left", bigFont, textColor)
end

-- ============================================================================
-- SCREEN 85: MOD OPERATIONS HUB
-- ============================================================================

local function getModeCardLayout()
  local buttonsList = menu.menuButtons or {}
  local count = #buttonsList
  local cardW = 200
  local cardH = 465
  local startY = 185
  local gap = 25

  local totalWidth = count * cardW + math.max(0, count - 1) * gap
  local startX = 75
  if totalWidth < 1100 then
    startX = math.floor((1280 - totalWidth) / 2)
  end

  return buttonsList, count, cardW, cardH, startY, gap, startX
end

function menu.registerModModesButtons()
  buttons[#buttons + 1] = Button.new(32, 686, 60, 30, "menu_back", "active", true, mouseOverBack)

  local curX = 120
  if menu.modList and #menu.modList > 0 then
    buttons[#buttons + 1] = Button.new(curX, 686, 135, 30, "menu_modOptions", "active", true, "Configure settings for installed mods")
    curX = curX + 135 + 20
  end
  buttons[#buttons + 1] = Button.new(curX, 686, 100, 30, "menu_controls", "active", true, "Configure keyboard shortcuts and controls")

  local buttonsList, count, cardW, cardH, startY, gap, startX = getModeCardLayout()
  for i, btn in ipairs(buttonsList) do
    local cardX = startX + (i - 1) * (cardW + gap)
    local btnLabel = "qm_mode_" .. i
    buttons[#buttons + 1] = Button.new(cardX, startY, cardW, cardH, btnLabel, "active", true, btn.description or btn.title)

    _G[btnLabel .. "Button"] = function()
      playClickSound()
      mouseOverText = ""
      contextualText = ""
      if btn.onClick then
        pcall(btn.onClick)
      end
    end
  end
end

function menu.drawModModes()
  local fade = uiFade or 255
  local mx, my = love.mouse.getPosition()

  drawCleanShadowText("MOD OPERATIONS", 30, 130, 600, "left", bigFont)
  if nonoFont then
    love.graphics.setFont(nonoFont)
    love.graphics.setColor(love.math.colorFromBytes(180, 180, 180, fade))
    love.graphics.print("COMMUNITY EXPANSIONS & CUSTOM THEATERS", 32, 160)
  end

  -- Back button
  local backHovered = (mx >= 30 and mx <= 100 and my >= 680 and my <= 715)
  local backColor = backHovered and { 101, 24, 24, fade } or { 255, 255, 255, fade }
  drawCleanShadowText("Back", 35, 685, 100, "left", bigFont, backColor)

  local curX = 120
  if menu.modList and #menu.modList > 0 then
    local optHovered = (mx >= curX and mx <= curX + 135 and my >= 680 and my <= 715)
    local optColor = optHovered and { 101, 24, 24, fade } or { 211, 211, 211, fade }
    drawCleanShadowText("Mod Settings", curX, 685, 135, "left", bigFont, optColor)
    curX = curX + 135 + 20
  end

  local ctrlHovered = (mx >= curX and mx <= curX + 100 and my >= 680 and my <= 715)
  local ctrlColor = ctrlHovered and { 101, 24, 24, fade } or { 211, 211, 211, fade }
  drawCleanShadowText("Keybinds", curX, 685, 100, "left", bigFont, ctrlColor)

  local buttonsList, count, cardW, cardH, startY, gap, startX = getModeCardLayout()
  if count == 0 then
    drawCleanShadowText("No custom game modes installed.", 50, 220, 600, "left", bigFont)
    return
  end

  for i, btn in ipairs(buttonsList) do
    local cardX = startX + (i - 1) * (cardW + gap)
    local hovered = (mx >= cardX and mx <= cardX + cardW and my >= startY and my <= startY + cardH)

    love.graphics.setColor(love.math.colorFromBytes(255, 255, 255, hovered and 175 or 120))
    love.graphics.rectangle("fill", cardX, startY, cardW, cardH)

    local frameX = cardX + 25
    local frameY = startY + 25
    local frameW = 150
    local frameH = 340

    love.graphics.setColor(love.math.colorFromBytes(0, 0, 0, fade))
    love.graphics.rectangle("line", frameX, frameY, frameW, frameH)

    if btn.iconImage then
      love.graphics.setColor(love.math.colorFromBytes(101, 24, 24, fade))
      local imgW = btn.iconImage:getWidth()
      local imgH = btn.iconImage:getHeight()
      local sx = frameW / imgW
      local sy = frameH / imgH
      love.graphics.draw(btn.iconImage, frameX, frameY, 0, sx, sy)
    else
      love.graphics.setColor(love.math.colorFromBytes(60, 60, 60, fade))
      love.graphics.rectangle("fill", frameX + 2, frameY + 2, frameW - 4, frameH - 4)
      love.graphics.setColor(love.math.colorFromBytes(140, 140, 140, fade))
      if nonoFont then love.graphics.setFont(nonoFont) end
      love.graphics.printf("CUSTOM OP", frameX, frameY + (frameH / 2) - 10, frameW, "center")
    end

    if hovered then
      love.graphics.setColor(love.math.colorFromBytes(101, 24, 24, fade))
      love.graphics.setLineWidth(2)
      love.graphics.rectangle("line", cardX, startY, cardW, cardH)
      love.graphics.setLineWidth(1)
    end

    local titleColor = hovered and { 101, 24, 24, fade } or { 255, 255, 255, fade }
    drawCleanShadowText(btn.title:upper(), cardX, startY + 380, cardW, "center", bigFont, titleColor)

    if btn.subtitle and nonoFont then
      love.graphics.setFont(nonoFont)
      love.graphics.setColor(love.math.colorFromBytes(0, 0, 0, fade))
      love.graphics.printf(btn.subtitle, cardX + 10, startY + 415, cardW - 20, "center")
    end
  end
end

-- ============================================================================
-- SCREEN 86: MOD OPTIONS PANEL
-- ============================================================================

local function getActiveModAndOptions()
  local modList = menu.modList or {}
  if #modList == 0 then return nil, nil end
  local activeModId = modList[menu.activeModIndex] or modList[1]
  local modData = menu.optionsByMod[activeModId]
  return activeModId, modData
end

function menu.registerModOptionsButtons()
  buttons[#buttons + 1] = Button.new(32, 686, 60, 30, "qm_opt_back", "active", true, mouseOverBack)
  _G.qm_opt_backButton = function()
    playClickSound()
    local hasModes = menu.menuButtons and #menu.menuButtons > 0
    menuPage = hasModes and menu.PAGE_MOD_MODES or 1
    mouseOverText = ""
    contextualText = ""
  end

  local modList = menu.modList or {}
  if #modList == 0 then return end

  -- Tab Buttons
  if #modList > 1 then
    menu.tabsPerPage = menu.tabsPerPage or 4
    menu.tabPageIndex = menu.tabPageIndex or 1
    local totalTabPages = math.max(1, math.ceil(#modList / menu.tabsPerPage))
    if menu.tabPageIndex > totalTabPages then menu.tabPageIndex = totalTabPages end
    if menu.tabPageIndex < 1 then menu.tabPageIndex = 1 end

    local tabStartX = 280
    local tabY = 125
    local tabW = 160
    local tabH = 32

    -- Tab pagination buttons
    if #modList > menu.tabsPerPage then
      if menu.tabPageIndex > 1 then
        buttons[#buttons + 1] = Button.new(240, tabY, 32, tabH, "qm_tab_prev", "active", true, "Previous mod options page")
        _G.qm_tab_prevButton = function()
          playClickSound()
          menu.tabPageIndex = math.max(1, menu.tabPageIndex - 1)
        end
      end

      if menu.tabPageIndex < totalTabPages then
        local nextX = tabStartX + menu.tabsPerPage * (tabW + 10) + 2
        buttons[#buttons + 1] = Button.new(nextX, tabY, 32, tabH, "qm_tab_next", "active", true, "Next mod options page")
        _G.qm_tab_nextButton = function()
          playClickSound()
          menu.tabPageIndex = math.min(totalTabPages, menu.tabPageIndex + 1)
        end
      end
    end

    local startIdx = (menu.tabPageIndex - 1) * menu.tabsPerPage + 1
    local endIdx = math.min(#modList, startIdx + menu.tabsPerPage - 1)

    for mIdx = startIdx, endIdx do
      local mId = modList[mIdx]
      local mData = menu.optionsByMod[mId]
      local relIdx = mIdx - startIdx
      local tabX = tabStartX + relIdx * (tabW + 10)
      local tabLabel = "qm_tab_" .. mIdx
      buttons[#buttons + 1] = Button.new(tabX, tabY, tabW, tabH, tabLabel, "active", true, "Select " .. (mData and mData.name or mId))

      _G[tabLabel .. "Button"] = function()
        playClickSound()
        menu.activeModIndex = mIdx
      end
    end
  end

  -- Option Card Buttons
  local activeModId, modData = getActiveModAndOptions()
  if not modData or not modData.options then return end

  local options = menu.getVisibleOptions(activeModId)
  if #options == 0 then return end

  local cardW = 500
  local cardH = 50
  local rowGap = 12

  for idx, opt in ipairs(options) do
    local col = math.floor((idx - 1) / 7)
    local row = (idx - 1) % 7
    local cardX = 50 + col * 525
    local cardY = 185 + row * (cardH + rowGap)

    local optLabel = "qm_opt_" .. idx
    buttons[#buttons + 1] = Button.new(cardX, cardY, cardW, cardH, optLabel, "active", true, opt.description or opt.label)

    _G[optLabel .. "Button"] = function()
      local optType = opt.type or opt.option_type or "checkbox"
      local curVal = menu.getOptionValue(activeModId, opt)

      if optType == "checkbox" or optType == "bool" then
        local nextVal
        if type(curVal) == "boolean" then
          nextVal = not curVal
        else
          nextVal = (curVal == 0 or curVal == false or curVal == nil) and 1 or 0
        end
        playClickSound()
        menu.setOptionValue(activeModId, opt, nextVal)

      elseif optType == "cycle" or optType == "stepper" then
        if opt.options and #opt.options > 0 then
          local curIdx = 1
          for oIdx, val in ipairs(opt.options) do
            if val == curVal then curIdx = oIdx; break end
          end

          local clickMx, _ = love.mouse.getPosition()
          local btnW = 175
          local btnX = cardX + cardW - btnW - 15
          local midX = btnX + (btnW * 0.5)

          local nextIdx
          if clickMx < midX then
            nextIdx = (curIdx - 2 + #opt.options) % #opt.options + 1
          else
            nextIdx = (curIdx % #opt.options) + 1
          end

          local nextVal = opt.options[nextIdx]
          playClickSound()
          menu.setOptionValue(activeModId, opt, nextVal)
        end

      elseif optType == "button" then
        playClickSound()
        local clickHandler = opt.onClick or opt.on_click
        if type(clickHandler) == "function" then
          pcall(clickHandler)
        elseif type(clickHandler) == "string" then
          if _G[clickHandler] then
            pcall(_G[clickHandler])
          elseif clickHandler:match("%.") then
            local modName, fnName = clickHandler:match("^([^%.]+)%.([^%.]+)$")
            if _G[modName] and _G[modName][fnName] then
              pcall(_G[modName][fnName])
            end
          end
        end
      end
    end
  end
end

function menu.drawModOptions()
  local fade = uiFade or 255
  local mx, my = love.mouse.getPosition()
  local currentTooltip = ""

  drawCleanShadowText("MOD OPTIONS", 30, 130, 400, "left", bigFont)

  -- Back Button Text
  local backHovered = (mx >= 30 and mx <= 100 and my >= 680 and my <= 715)
  local backColor = backHovered and { 101, 24, 24, fade } or { 255, 255, 255, fade }
  drawCleanShadowText("Back", 35, 685, 100, "left", bigFont, backColor)

  local modList = menu.modList or {}
  if #modList == 0 then
    drawCleanShadowText("No configurable mod options found.", 50, 220, 600, "left", bigFont)
    return
  end

  -- Tab Bar
  if #modList > 1 then
    menu.tabsPerPage = menu.tabsPerPage or 4
    menu.tabPageIndex = menu.tabPageIndex or 1
    local totalTabPages = math.max(1, math.ceil(#modList / menu.tabsPerPage))
    if menu.tabPageIndex > totalTabPages then menu.tabPageIndex = totalTabPages end
    if menu.tabPageIndex < 1 then menu.tabPageIndex = 1 end

    local tabStartX = 280
    local tabY = 125
    local tabW = 160
    local tabH = 32

    -- Tab pagination arrows
    if #modList > menu.tabsPerPage then
      if menu.tabPageIndex > 1 then
        local prevHovered = (mx >= 240 and mx <= 272 and my >= tabY and my <= tabY + tabH)
        love.graphics.setColor(love.math.colorFromBytes(prevHovered and 101 or 35, prevHovered and 24 or 35, prevHovered and 24 or 35, fade))
        love.graphics.rectangle("fill", 240, tabY, 32, tabH)
        love.graphics.setColor(love.math.colorFromBytes(180, 180, 180, fade))
        love.graphics.rectangle("line", 240, tabY, 32, tabH)
        drawCleanShadowText("<", 240, tabY + 5, 32, "center", cardFont or bigFont, { 255, 255, 255, fade })
      end

      if menu.tabPageIndex < totalTabPages then
        local nextX = tabStartX + menu.tabsPerPage * (tabW + 10) + 2
        local nextHovered = (mx >= nextX and mx <= nextX + 32 and my >= tabY and my <= tabY + tabH)
        love.graphics.setColor(love.math.colorFromBytes(nextHovered and 101 or 35, nextHovered and 24 or 35, nextHovered and 24 or 35, fade))
        love.graphics.rectangle("fill", nextX, tabY, 32, tabH)
        love.graphics.setColor(love.math.colorFromBytes(180, 180, 180, fade))
        love.graphics.rectangle("line", nextX, tabY, 32, tabH)
        drawCleanShadowText(">", nextX, tabY + 5, 32, "center", cardFont or bigFont, { 255, 255, 255, fade })
      end
    end

    local startIdx = (menu.tabPageIndex - 1) * menu.tabsPerPage + 1
    local endIdx = math.min(#modList, startIdx + menu.tabsPerPage - 1)

    for mIdx = startIdx, endIdx do
      local mId = modList[mIdx]
      local mData = menu.optionsByMod[mId]
      local relIdx = mIdx - startIdx
      local tabX = tabStartX + relIdx * (tabW + 10)
      local isSelected = (mIdx == menu.activeModIndex)
      local tabHovered = (mx >= tabX and mx <= tabX + tabW and my >= tabY and my <= tabY + tabH)

      love.graphics.setColor(love.math.colorFromBytes(255, 255, 255, (isSelected or tabHovered) and 180 or 100))
      love.graphics.rectangle("fill", tabX, tabY, tabW, tabH)

      if isSelected then
        love.graphics.setColor(love.math.colorFromBytes(101, 24, 24, fade))
        love.graphics.rectangle("fill", tabX, tabY + tabH - 4, tabW, 4)
      end

      local tabColor = isSelected and { 101, 24, 24, fade } or { 255, 255, 255, fade }
      drawCleanShadowText(mData.name, tabX, tabY + 4, tabW, "center", cardFont or bigFont, tabColor)
    end
  end

  local activeModId, modData = getActiveModAndOptions()
  if not modData or not modData.options or #modData.options == 0 then
    drawCleanShadowText("No configurable options for this mod.", 50, 220, 600, "left", bigFont)
    return
  end

  local options = menu.getVisibleOptions(activeModId)
  if #options == 0 then
    drawCleanShadowText("No options visible in this view.", 50, 220, 600, "left", bigFont)
    return
  end

  local cardW = 500
  local cardH = 50
  local rowGap = 12

  for idx, opt in ipairs(options) do
    local col = math.floor((idx - 1) / 7)
    local row = (idx - 1) % 7
    local cardX = 50 + col * 525
    local cardY = 185 + row * (cardH + rowGap)

    local hovered = (mx >= cardX and mx <= cardX + cardW and my >= cardY and my <= cardY + cardH)
    if hovered and opt.description then
      currentTooltip = opt.description
    end

    love.graphics.setColor(love.math.colorFromBytes(255, 255, 255, hovered and 165 or 120))
    love.graphics.rectangle("fill", cardX, cardY, cardW, cardH)

    local labelFont = cardFont or bigFont
    drawCleanShadowText(opt.label:upper(), cardX + 20, cardY + 14, 280, "left", labelFont)

    local optType = opt.type or opt.option_type or "checkbox"
    local curVal = menu.getOptionValue(activeModId, opt)

    if optType == "checkbox" or optType == "bool" then
      local boxX = cardX + cardW - 50
      local boxY = cardY + 15
      love.graphics.setColor(love.math.colorFromBytes(35, 35, 38, 220))
      love.graphics.rectangle("fill", boxX, boxY, 20, 20)
      love.graphics.setColor(love.math.colorFromBytes(80, 85, 95, 255))
      love.graphics.rectangle("line", boxX, boxY, 20, 20)

      if curVal == 1 or curVal == true then
        love.graphics.setColor(love.math.colorFromBytes(180, 35, 35, fade))
        love.graphics.rectangle("fill", boxX + 3, boxY + 3, 14, 14)
      end

    elseif optType == "cycle" or optType == "stepper" then
      local btnW = 175
      local btnX = cardX + cardW - btnW - 15
      local btnY = cardY + 10
      local btnH = 30
      local btnHovered = (mx >= btnX and mx <= btnX + btnW and my >= btnY and my <= btnY + btnH)
      local leftHovered = btnHovered and (mx < btnX + (btnW * 0.5))
      local rightHovered = btnHovered and (mx >= btnX + (btnW * 0.5))

      love.graphics.setColor(love.math.colorFromBytes(28, 30, 34, 240))
      love.graphics.rectangle("fill", btnX, btnY, btnW, btnH)

      if leftHovered then
        love.graphics.setColor(love.math.colorFromBytes(101, 24, 24, 210))
        love.graphics.rectangle("fill", btnX + 1, btnY + 1, math.floor(btnW * 0.5) - 1, btnH - 2)
      elseif rightHovered then
        love.graphics.setColor(love.math.colorFromBytes(101, 24, 24, 210))
        love.graphics.rectangle("fill", btnX + math.floor(btnW * 0.5), btnY + 1, math.ceil(btnW * 0.5) - 1, btnH - 2)
      end

      love.graphics.setColor(love.math.colorFromBytes(75, 80, 90, 255))
      love.graphics.rectangle("line", btnX, btnY, btnW, btnH)

      local displayLabel = tostring(curVal)
      if opt.labels and type(opt.labels) == "table" and opt.options then
        for optIdx, oVal in ipairs(opt.options) do
          if oVal == curVal and opt.labels[optIdx] then
            displayLabel = opt.labels[optIdx]
            break
          end
        end
      end

      local fontToUse = (#displayLabel > 12) and (nonoFont or cardFont or bigFont) or (cardFont or bigFont)
      local textOffsetY = (fontToUse == nonoFont) and 6 or 4
      local valColor = { 255, 255, 255, fade }

      drawCleanShadowText("< " .. displayLabel .. " >", btnX, btnY + textOffsetY, btnW, "center", fontToUse, valColor)

    elseif optType == "button" then
      local btnX = cardX + cardW - 130
      local btnY = cardY + 10
      local btnW = 110
      local btnH = 30
      local btnHovered = (mx >= btnX and mx <= btnX + btnW and my >= btnY and my <= btnY + btnH)

      love.graphics.setColor(love.math.colorFromBytes(28, 30, 34, 240))
      love.graphics.rectangle("fill", btnX, btnY, btnW, btnH)

      if btnHovered then
        love.graphics.setColor(love.math.colorFromBytes(101, 24, 24, 210))
        love.graphics.rectangle("fill", btnX + 1, btnY + 1, btnW - 2, btnH - 2)
      end

      love.graphics.setColor(love.math.colorFromBytes(75, 80, 90, 255))
      love.graphics.rectangle("line", btnX, btnY, btnW, btnH)

      local btnColor = { 255, 255, 255, fade }
      drawCleanShadowText(opt.button_text or opt.buttonText or "EXECUTE", btnX, btnY + 4, btnW, "center", cardFont or bigFont, btnColor)
    end
  end

  if currentTooltip ~= "" and nonoFont then
    love.graphics.setColor(love.math.colorFromBytes(0, 0, 0, 160))
    love.graphics.rectangle("fill", 50, 640, 1025, 30)
    love.graphics.setFont(nonoFont)
    love.graphics.setColor(love.math.colorFromBytes(255, 255, 255, fade))
    love.graphics.printf(currentTooltip, 60, 646, 1005, "left")
  end
end

return menu
