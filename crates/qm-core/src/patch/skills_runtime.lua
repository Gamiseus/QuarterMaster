-- QuarterMaster: class skills (QM.Skills).
-- A mod changes any skill of any class, removes it, or adds new ones above the game's three,
-- with skill levels past 3 through promotion. The game keeps each class's skill icons in
-- <class>_skill_<n> and their mouse-over text in <class>_skillText_<n>; QuarterMaster sets those
-- for what it changes, and two hooks in urb_engine (added with the "skills" or "ordnance"
-- feature) call in:
--   the skill buttons            -> QM.Skills.addButton(peeps)
--   the promotion's skill choice -> QM.Skills.addSelect(peeps)
-- After love.load it wraps skillClick (what a click does), displayAbilityMenuz (the icons),
-- displaySkillMenu (the promotion's icon) and skillTimerUpdate (cooldowns past the third).
do
  _G.QM = _G.QM or {}
  if not _G.QM.Skills then
    local S = { slots = {}, orig = {}, watching = {} }
    _G.QM.Skills = S

    local function warn(msg)
      print("[QM][Skills] " .. msg)
    end

    local function sound(name, dude, scale)
      local s = _G[name]
      if not (s and s.clone) then return end
      local c = s:clone()
      if type(stepScale) == "function" and dude then c:setVolume(stepScale(dude) / (scale or 1)) end
      c:setPitch((90 + randNbr(10)) / 100)
      c:play()
    end

    local function no(dude)
      sound("neg" .. randNbr(15), dude, 3)
    end

    local function timer(dude, n)
      return tonumber(dude["skill" .. n .. "Timer"]) or 0
    end

    local function level(dude)
      return tonumber(dude.skillUpgrade) or 0
    end

    -- game = 2, "2", "light:3" or { class = "light", slot = 3 }: the game's own skill.
    local function parseGame(class, game)
      local c, m = class, nil
      if type(game) == "number" then
        m = game
      elseif type(game) == "string" then
        local a, b = game:match("^([%w_]+):(%d+)$")
        if a then c, m = a, tonumber(b) else m = tonumber(game) end
      elseif type(game) == "table" then
        c, m = game.class or class, tonumber(game.slot)
      end
      if not m or m < 1 or m > 3 or m % 1 ~= 0 then return nil end
      return { class = c, slot = m }
    end

    -- A skill: { class, slot (1 or more), name, description, icon (an image's name), cooldown
    -- (seconds), unlock (the skill level it comes with), and what it does: use =
    -- function(soldier, index), ordnance (QM.Ordnance, an entry or its id), game (a class's own
    -- skill: 2 or "light:3") or remove = true. On slots 1 to 3 with none of them, the game's
    -- own skill stays, with the changes given.
    function S.set(def)
      if type(def) ~= "table" or type(def.class) ~= "string" or def.class == "" then
        warn("a skill needs a class")
        return nil
      end
      local n = tonumber(def.slot)
      if not n or n < 1 or n % 1 ~= 0 then
        warn("the " .. def.class .. " skill needs a slot: 1, 2, 3 for the game's, 4 or more for new ones")
        return nil
      end
      def.slot = n
      if type(def.ordnance) == "string" then
        local O = _G.QM.Ordnance
        local g = O and O.byId[def.ordnance]
        if not g then
          warn("the " .. def.class .. " skill " .. n .. ": there is no ordnance " .. def.ordnance)
          return nil
        end
        def.ordnance = g
      end
      if def.game ~= nil then
        local game = parseGame(def.class, def.game)
        if not game then
          warn("the " .. def.class .. " skill " .. n .. ": game is a skill 1 to 3, such as 2 or \"light:3\"")
          return nil
        end
        def.game = game
        if game.class == "comms" and def.class ~= "comms" then
          warn("the " .. def.class .. " skill " .. n .. ": the radio operator's calls go through only for radio operators")
        end
      end
      local actions = 0
      if type(def.use) == "function" then actions = actions + 1 else def.use = nil end
      if def.ordnance then actions = actions + 1 end
      if def.game then actions = actions + 1 end
      if def.remove then actions = actions + 1 else def.remove = nil end
      if actions > 1 then
        warn("the " .. def.class .. " skill " .. n .. ": use, ordnance, game and remove are one or the other")
        return nil
      end
      if n > 3 and actions == 0 then
        warn("the " .. def.class .. " skill " .. n .. " is a new skill: it needs use, ordnance or game")
        return nil
      end
      -- A new skill comes with skill level 3, after the game's third, unless set.
      def.unlock = math.max(1, math.floor(tonumber(def.unlock) or (n <= 3 and n or 3)))
      def.cooldown = tonumber(def.cooldown)
      if not def.cooldown then
        if def.use then def.cooldown = 60 elseif def.ordnance then def.cooldown = def.ordnance.cooldown end
      end
      if n > 3 and def.use and not def.icon then
        warn("the " .. def.class .. " skill " .. n .. " has no icon; it shows the game's plain circle")
      end
      -- QuarterMaster draws a slot's icon and button itself when it is new, removed, or comes
      -- with another level than the game's.
      def.own = n > 3 or def.remove == true or def.unlock ~= n
      local list = S.slots[def.class] or {}
      S.slots[def.class] = list
      if list[n] then
        warn("the " .. def.class .. " skill " .. n .. " is set twice; the last one holds")
      end
      list[n] = def
      if S.ready then
        S.apply(def.class)
        S.install()
      end
      return def
    end

    -- A new skill on the class's next free slot above the game's three.
    function S.add(def)
      if type(def) ~= "table" or type(def.class) ~= "string" then
        warn("a skill needs a class")
        return nil
      end
      if not def.slot then
        local list = S.slots[def.class] or {}
        local n = 4
        while list[n] do n = n + 1 end
        def.slot = n
      end
      return S.set(def)
    end

    function S.remove(class, slot)
      return S.set({ class = class, slot = slot, remove = true })
    end

    function S.get(class, slot)
      return S.slots[class] and S.slots[class][tonumber(slot)]
    end

    -- The highest skill level a class has: 3, or a later skill's unlock.
    function S.maxLevel(class)
      local top = 3
      for _, def in pairs(S.slots[class] or {}) do
        if not def.remove and def.unlock > top then top = def.unlock end
      end
      return top
    end

    local function orig(class)
      local o = S.orig[class]
      if not o then
        o = { icons = {}, texts = {} }
        for n = 1, 3 do
          o.icons[n] = _G[class .. "_skill_" .. n]
          o.texts[n] = _G[class .. "_skillText_" .. n]
        end
        S.orig[class] = o
      end
      return o
    end

    -- A slot's mouse-over text when it is ready.
    function S.text(class, n)
      local def = S.get(class, n)
      if def and (def.name or def.description) then
        if def.name and def.description then return def.name .. ": " .. def.description end
        return def.name or def.description
      end
      local g = def and def.ordnance
      if g and (g.name or g.description) then
        return (g.name or g.id) .. (g.description and (": " .. g.description) or "")
      end
      if def and def.game then
        local o = orig(def.game.class)
        return o.texts[def.game.slot] or ""
      end
      if n <= 3 then return orig(class).texts[n] or "" end
      return ""
    end

    local function imageOf(name)
      local img = type(name) == "string" and _G[name] or name
      if type(img) == "userdata" or (type(img) == "table" and img.getWidth) then return img end
      return nil
    end

    -- A slot's icon.
    function S.icon(class, n)
      local def = S.get(class, n)
      local img = def and imageOf(def.icon)
      if img then return img end
      if def and def.game then return orig(def.game.class).icons[def.game.slot] end
      local g = def and def.ordnance
      if g then
        img = imageOf(g.icon) or imageOf(g.sprite)
        if img then return img end
        local fallback = ({ mortar = "rifle_skill_3", artillery = "comms_skill_3", airstrike = "comms_skill_2",
          drop = "comms_skill_1" })[g.kind]
        if fallback then return _G[fallback] end
      end
      if n <= 3 then return orig(class).icons[n] end
      -- A new skill with no picture of its own: the game's plain circle, so its button shows.
      return _G.whiteCircle
    end

    -- What a skill level brings, for the promotion: its skills' texts.
    function S.levelText(class, lvl)
      local parts = {}
      local list = S.slots[class] or {}
      local top = 3
      for n in pairs(list) do if n > top then top = n end end
      for n = 1, top do
        local def = list[n]
        local unlock = def and def.unlock or (n <= 3 and n or nil)
        if unlock == lvl and not (def and def.remove) and (def or n <= 3) then
          local t = S.text(class, n)
          if t ~= "" then parts[#parts + 1] = t end
        end
      end
      return table.concat(parts, " / ")
    end

    -- The icon a skill level shows in the promotion: its first skill's.
    function S.levelIcon(class, lvl)
      local list = S.slots[class] or {}
      local top = 3
      for n in pairs(list) do if n > top then top = n end end
      for n = 1, top do
        local def = list[n]
        local unlock = def and def.unlock or (n <= 3 and n or nil)
        if unlock == lvl and not (def and def.remove) and (def or n <= 3) then return S.icon(class, n) end
      end
      return nil
    end

    -- The game's globals for a class: the icons of the game's three slots, and each skill
    -- level's text (the slot buttons' own text is set as they are made).
    function S.apply(class)
      local o = orig(class)
      local list = S.slots[class] or {}
      for n = 1, 3 do
        local def = list[n]
        local img = def and imageOf(def.icon)
        if def and def.icon and not img then
          warn("the " .. class .. " skill " .. n .. ": there is no image " .. tostring(def.icon))
        end
        -- A slot that does another class's skill shows that skill's icon unless it has its own.
        if not img and def and def.game then img = orig(def.game.class).icons[def.game.slot] end
        if img then _G[class .. "_skill_" .. n] = img end
      end
      -- A level that brings no skill any more has no text.
      for lvl = 1, S.maxLevel(class) do
        _G[class .. "_skillText_" .. lvl] = S.levelText(class, lvl)
      end
    end

    -- The selected soldier.
    local function selected(id)
      if type(convertedId) ~= "function" then return nil end
      local idx = convertedId(id or specID)
      if idx == -1 then return nil end
      local dude = player.deployedTroops[idx]
      if not dude then return nil end
      return dude, idx
    end

    local function peepsOf(idx)
      local peeps = 0
      for v = 1, idx do
        if player.deployedTroops[v].playerControlled == 1 then peeps = peeps + 1 end
      end
      return peeps
    end

    -- Where a slot's button goes, from the soldier's portrait: the game's three at 610, 510
    -- and 400; new ones above, 100 apart, seven to a column, then a column to the right.
    function S.place(n)
      if n == 1 then return 0, 610 elseif n == 2 then return 0, 510 elseif n == 3 then return 0, 400 end
      if n <= 7 then return 0, 300 - (n - 4) * 100 end
      local k = n - 8
      return 55 * (1 + math.floor(k / 7)), 610 - (k % 7) * 100
    end

    -- Where a slot's icon is centred, below its button's top: the game's 632, 532 and 432.
    function S.iconDrop(n)
      if n == 1 or n == 2 then return 22 end
      return 32
    end

    -- The game's own skill m, as another slot's: its timer is set to 0 for the call and put
    -- back, and the cooldown the game then sets is moved to the slot's own timer.
    function S.watch(dude, from, to, cooldown, saved)
      S.watching[dude] = { from = from, to = to, cooldown = cooldown, saved = saved, last = timer(dude, from), age = 0 }
    end

    function S.check(dude)
      local w = S.watching[dude]
      if not w then return end
      local now = timer(dude, w.from)
      if now > w.last then
        dude["skill" .. w.to .. "Timer"] = w.cooldown or now
        if w.from ~= w.to then dude["skill" .. w.from .. "Timer"] = w.saved or 0 end
        S.watching[dude] = nil
      else
        w.last = now
      end
    end

    function S.borrow(dude, def, n, game)
      if timer(dude, n) > 0 then
        no(dude)
        return true
      end
      local key = "skill" .. game.slot .. "Timer"
      local saved = dude[key]
      dude[key] = 0
      S.watch(dude, game.slot, n, def.cooldown, saved)
      local spec = dude.spec
      dude.spec = game.class
      local ok, err = pcall(S.gameClick, game.slot)
      dude.spec = spec
      if not ok then warn("the " .. tostring(spec) .. " skill " .. n .. " failed: " .. tostring(err)) end
      S.check(dude)
      -- The game arms a skill that waits for a click: its timer comes later.
      if S.watching[dude] and game.slot ~= n then dude[key] = saved; S.watching[dude].last = tonumber(saved) or 0 end
      return true
    end

    -- skillClick(n): a skill QuarterMaster changed; anything else is the game's.
    function S.click(n)
      n = tonumber(n)
      local dude, idx = selected()
      if not dude or not n then return false end
      local def = S.get(tostring(dude.spec), n)
      if not def then return n > 3 end
      -- A slot QuarterMaster draws waits for its level; the game's own show only when unlocked.
      if def.remove or (def.own and level(dude) < def.unlock) then return true end
      if def.ordnance then
        local O = _G.QM.Ordnance
        return O and O.arm and O.arm(dude, def.ordnance, n) or true
      end
      if def.use then
        if dude.health > 10 then
          if timer(dude, n) <= 0 then
            local ok, err = pcall(def.use, dude, idx)
            if not ok then warn("the " .. def.class .. " skill " .. n .. " failed: " .. tostring(err)) end
            dude["skill" .. n .. "Timer"] = def.cooldown
          else
            no(dude)
          end
        end
        return true
      end
      local game = def.game or (def.own and { class = tostring(dude.spec), slot = n }) or nil
      if game then return S.borrow(dude, def, n, game) end
      -- A changed game skill: the game's own, with its cooldown watched.
      if def.cooldown then
        S.watch(dude, n, n, def.cooldown)
        S.clicked = dude
      end
      return false
    end

    -- The hook where the game makes the selected soldier's skill buttons: the game's buttons
    -- for slots QuarterMaster draws go, the others get their slot's text, and the new ones come.
    function S.addButton(peeps)
      local dude, idx = selected()
      if not dude then return end
      local class = tostring(dude.spec)
      local list = S.slots[class]
      if not list then return end
      peeps = peeps or peepsOf(idx)
      for i = #buttons, math.max(1, #buttons - 5), -1 do
        local b = buttons[i]
        local n = b and type(b.label) == "string" and tonumber(b.label:match("^skill_([123])$"))
        if n then
          local def = list[n]
          if def and def.own then
            table.remove(buttons, i)
          elseif timer(dude, n) <= 0 then
            -- The game's button text is its level's; the button says its own skill.
            b.mouseOverText = S.text(class, n)
          end
        end
      end
      local lvl = level(dude)
      for n, def in pairs(list) do
        if def.own and not def.remove and lvl >= def.unlock then
          local dx, by = S.place(n)
          local t = timer(dude, n)
          local text = t > 0 and type(tellTime) == "function" and tellTime(t) or S.text(class, n)
          buttons[#buttons + 1] = Button.new(player.screenx + (peeps * 60) + dx, by + player.screeny, 45, 50,
            "skill_" .. n, "active", true, text)
        end
      end
    end

    -- The hook in the promotion's choices: a skill level past the game's third.
    function S.addSelect(peeps)
      local dude, idx = selected()
      if not dude then return end
      local class = tostring(dude.spec)
      local lvl = level(dude)
      if lvl < 3 or lvl >= S.maxLevel(class) then return end
      peeps = peeps or peepsOf(idx)
      buttons[#buttons + 1] = Button.new(player.screenx + (peeps * 60), 610 + player.screeny, 45, 45,
        "skillSelect", "active", true, S.levelText(class, lvl + 1))
    end

    local function blank()
      if not S.blank and love and love.image and love.graphics then
        S.blank = love.graphics.newImage(love.image.newImageData(1, 1))
      end
      return S.blank
    end

    local function drawIcon(img, x, y, dim)
      local w, h = img:getWidth(), img:getHeight()
      local scale = math.min(1, 46 / math.max(w, h))
      love.graphics.setColor(love.math.colorFromBytes(255, 255, 255, dim and 20 or 255))
      love.graphics.draw(img, x, y, 0, scale, scale, w / 2, h / 2)
      love.graphics.setColor(love.math.colorFromBytes(255, 255, 255))
    end

    -- The icons: the game draws its three (a slot QuarterMaster draws is blank for the game),
    -- then QuarterMaster draws its own.
    function S.drawAbilities(game, id, ...)
      local dude, idx = selected(id)
      local list = dude and S.slots[tostring(dude.spec)]
      if not list then return game(id, ...) end
      local class = tostring(dude.spec)
      local hidden = {}
      for n = 1, 3 do
        if list[n] and list[n].own then
          hidden[n] = _G[class .. "_skill_" .. n]
          _G[class .. "_skill_" .. n] = blank() or hidden[n]
        end
      end
      local r = { pcall(game, id, ...) }
      for n, img in pairs(hidden) do _G[class .. "_skill_" .. n] = img end
      if not r[1] then error(r[2], 0) end
      local ok, err = pcall(S.draw, dude, idx, list)
      if not ok then warn("drawing skills: " .. tostring(err)) end
      return (table.unpack or unpack)(r, 2)
    end

    function S.draw(dude, idx, list)
      local class = tostring(dude.spec)
      local peeps = peepsOf(idx)
      local lvl = level(dude)
      for n, def in pairs(list) do
        if def.own and not def.remove and lvl >= def.unlock then
          local img = S.icon(class, n)
          if img then
            local dx, by = S.place(n)
            drawIcon(img, (peeps * 60) - 3 + 25 + player.screenx + dx, by + S.iconDrop(n) + player.screeny,
              timer(dude, n) > 0)
          end
        end
      end
    end

    -- The promotion: the game draws the next skill's icon below level 3; QuarterMaster gives
    -- it the icon of what that level now brings, and draws the levels past 3.
    function S.drawSelect(game, id, ...)
      local dude, idx = selected(id)
      local list = dude and S.slots[tostring(dude.spec)]
      if not list then return game(id, ...) end
      local class = tostring(dude.spec)
      local lvl = level(dude)
      local key = class .. "_skill_" .. (lvl + 1)
      local held = _G[key]
      if lvl < 3 then _G[key] = S.levelIcon(class, lvl + 1) or blank() or held end
      local r = { pcall(game, id, ...) }
      if lvl < 3 then _G[key] = held end
      if not r[1] then error(r[2], 0) end
      if lvl >= 3 and lvl < S.maxLevel(class) then
        local img = S.levelIcon(class, lvl + 1)
        if img then drawIcon(img, (peepsOf(idx) * 60) - 3 + 25 + player.screenx, 632 + player.screeny, false) end
      end
      return (table.unpack or unpack)(r, 2)
    end

    -- Once a game second: the cooldowns past the third, and the game cooldowns watched.
    function S.tick()
      local top = 3
      for _, list in pairs(S.slots) do
        for n in pairs(list) do if n > top then top = n end end
      end
      for _, dude in ipairs(player.deployedTroops or {}) do
        if dude.playerControlled == 1 then
          for n = 4, top do
            local t = timer(dude, n)
            if t > 0 then dude["skill" .. n .. "Timer"] = t - 1 end
          end
        end
      end
      for dude, w in pairs(S.watching) do
        S.check(dude)
        w.age = w.age + 1
        if S.watching[dude] == w and w.age > 60 then
          if w.from ~= w.to and w.saved ~= nil then dude["skill" .. w.from .. "Timer"] = w.saved end
          S.watching[dude] = nil
        end
      end
    end

    -- After love.load (and again when a skill is set later).
    function S.install()
      S.ready = true
      if next(S.slots) == nil then return end
      for class in pairs(S.slots) do S.apply(class) end
      S.wrapped = S.wrapped or {}
      local W = S.wrapped
      if not W.skillClick and type(skillClick) == "function" then
        W.skillClick = true
        local game = skillClick
        S.gameClick = game
        skillClick = function(skill, ...)
          S.clicked = nil
          local ok, handled = pcall(S.click, skill)
          if ok and handled then return end
          if not ok then warn("skill " .. tostring(skill) .. ": " .. tostring(handled)) end
          local r = { game(skill, ...) }
          -- A changed cooldown, when the game set its own at once.
          if S.clicked then pcall(S.check, S.clicked) end
          return (table.unpack or unpack)(r)
        end
      end
      if not W.displayAbilityMenuz and type(displayAbilityMenuz) == "function" then
        W.displayAbilityMenuz = true
        local game = displayAbilityMenuz
        displayAbilityMenuz = function(id, ...) return S.drawAbilities(game, id, ...) end
      end
      if not W.displaySkillMenu and type(displaySkillMenu) == "function" then
        W.displaySkillMenu = true
        local game = displaySkillMenu
        displaySkillMenu = function(id, ...) return S.drawSelect(game, id, ...) end
      end
      if not W.skillTimerUpdate and type(skillTimerUpdate) == "function" then
        W.skillTimerUpdate = true
        local game = skillTimerUpdate
        skillTimerUpdate = function(...)
          local r = { game(...) }
          local ok, err = pcall(S.tick)
          if not ok then warn("skill cooldowns: " .. tostring(err)) end
          return (table.unpack or unpack)(r)
        end
      end
    end
  end
end
