-- QuarterMaster: ordnance and explosions (QM.Ordnance, QM.Explosions): grenades, rifle
-- grenades, mortars, artillery, air strikes and supply drops
-- A mod registers an explosion and its ordnance from Lua; QuarterMaster hands out the game's
-- numbers for them (grenadeType, mousePrompt) so two mods never pick the same one, and short
-- hooks in the game's code (added when a mod lists "ordnance" in qm_features) call in:
--   main.lua           the click after a skill is armed     -> QM.Ordnance.click(x, y)
--   ai.lua             a soldier's throw (state fragThrow3) -> QM.Ordnance.throwFor(i)
--   particlesSystem    the grenade's blast                  -> QM.Ordnance.blast(particle, x, y)
--   particlesSystem    a rifle grenade's impact (two places) -> QM.Ordnance.blast(particle, x, y)
--   ai.lua             a mortar round fired (state mortar3) -> QM.Ordnance.mortarRound(soldier, action)
--   urb_engine         createExplosion(tag)                 -> QM.Explosions.make(tag, x, y, dad)
-- A class skill that uses ordnance is a QM.Skills skill (skills_runtime.lua). After
-- love.load, rifling
-- (the game's one function for firing a rifle grenade, the player's and the AI's) for rifle
-- grenades, and for what is called in (artillery, air strikes, supply drops) delayUpdate
-- (QuarterMaster's own queue keeps time with the game's delayed actions), mapReset (a new
-- battle drops it), and for a side's swap the game's alliedArtyIncoming and germanArtyIncoming
-- (barrages), alliedBomb and stukaStrafe (bombing runs), alliedStrafe, p47Strafing and
-- axisStrafe (strafing runs), and cargoCratezButton and cargoCrateOpening (a crate's
-- contents). A thrown grenade flies as the game's frag does (particle type fragGrenade), a
-- rifle grenade as the game's rifle grenade (rifleGrenade), each with its own sprite; a
-- mortar round, a shell or a bomb, as the game's, is only its explosion; a supply drop falls
-- as the game's parachute (parachute) and lands as its supply crate (cargoCrate).
do
  _G.QM = _G.QM or {}
  if not _G.QM.Ordnance then
    local X = { defs = {} }
    local G = { byId = {}, byType = {}, byPrompt = {}, skills = {}, swaps = {}, rifleSwaps = {},
      mortarSwaps = {}, artySwaps = {}, bombSwaps = {}, strafeSwaps = {}, dropSwaps = {}, queue = {}, landing = {},
      nextNumber = 9000 }
    _G.QM.Explosions = X
    _G.QM.Ordnance = G

    local function warn(msg)
      print("[QM][Ordnance] " .. msg)
    end
    local KINDS
    -- Each swap: the kind it is for, and its list (side -> grenade).
    local SWAP_KEYS = {
      replaces_frag = { "thrown", "swaps" },
      replaces_rifle_grenade = { "rifle", "rifleSwaps" },
      replaces_mortar = { "mortar", "mortarSwaps" },
      replaces_artillery = { "artillery", "artySwaps" },
      replaces_bombing = { "airstrike", "bombSwaps" },
      replaces_strafing = { "airstrike", "strafeSwaps" },
      replaces_drop = { "drop", "dropSwaps" },
    }

    -- The game's own explosions (Mud and Blood 0.5.3b, createExplosion): a preset by name or
    -- by the game's tag. A test checks these against the game's code.
    X.presets = {
      frag = { tag = "mineExplosion", size = 1, atRadius = 25, atDamage = 100, blastRadius = 75, blastDamage = 50,
        demoRadius = 10, fragRadius = 100, fragDamage = 50, moraleHit = 15, stunRange = 100, stunTime = 5,
        crater = 5, sound = "fragSound", sounds = 3 },
      contact = { tag = "contactExplosion", size = 3, atRadius = 50, atDamage = 50, blastRadius = 0, blastDamage = 0,
        demoRadius = 25, fragRadius = 50, fragDamage = 35, moraleHit = 20, stunRange = 50, stunTime = 5,
        crater = 3, sound = "contactFragSound" },
      at_grenade = { tag = "atGrenadeExplosion", size = 4, atRadius = 100, atDamage = 150, blastRadius = 25, blastDamage = 25,
        demoRadius = 50, fragRadius = 50, fragDamage = 10, moraleHit = 15, stunRange = 100, stunTime = 5,
        crater = 6, sound = "atGrenadeSound" },
      rifle_grenade = { tag = "rifleGrenadeExplosion", size = 0.4, atRadius = 8, atDamage = 200, blastRadius = 25, blastDamage = 100,
        demoRadius = 1, fragRadius = 100, fragDamage = 50, moraleHit = 10, stunRange = 50, stunTime = 4,
        crater = 4, sound = "fragSound", sounds = 3 },
      mortar = { tag = "mortarExplosion", size = 1, atRadius = 25, atDamage = 100, blastRadius = 25, blastDamage = 100,
        demoRadius = 25, fragRadius = 50, fragDamage = 25, moraleHit = 10, stunRange = 25, stunTime = 3,
        crater = 4, sound = "fragSound", sounds = 3 },
      faust = { tag = "faustExplosion", size = 1, atRadius = 50, atDamage = 350, blastRadius = 50, blastDamage = 20,
        demoRadius = 25, fragRadius = 50, fragDamage = 25, moraleHit = 10, stunRange = 50, stunTime = 5,
        crater = 4, sound = "fragSound", sounds = 3 },
      artillery = { tag = "artyExplosion", size = 2, atRadius = 30, atDamage = 200, blastRadius = 75, blastDamage = 50,
        demoRadius = 100, fragRadius = 100, fragDamage = 100, moraleHit = 25, stunRange = 100, stunTime = 8,
        crater = 10, sound = "artySound", sounds = 4 },
      heavy_mortar = { tag = "hvyExplosion", size = 0.6, atRadius = 75, atDamage = 100, blastRadius = 25, blastDamage = 15,
        demoRadius = 25, fragRadius = 100, fragDamage = 50, moraleHit = 10, stunRange = 200, stunTime = 10,
        crater = 5, sound = "b17Explo", sounds = 3 },
      bomb = { tag = "bombExplosion", size = 4, atRadius = 100, atDamage = 100, blastRadius = 100, blastDamage = 100,
        demoRadius = 100, fragRadius = 100, fragDamage = 100, moraleHit = 30, stunRange = 100, stunTime = 8,
        crater = 15, sound = "bombSound" },
      strafe = { tag = "flakExplosion", size = 0.3, atRadius = 1, atDamage = 10, blastRadius = 10, blastDamage = 25,
        demoRadius = 1, fragRadius = 50, fragDamage = 25, moraleHit = 5, stunRange = 25, stunTime = 3,
        crater = 2, sound = "flak", sounds = 5 },
    }
    local FIELDS = { "size", "atRadius", "atDamage", "blastRadius", "blastDamage", "demoRadius",
      "fragRadius", "fragDamage", "moraleHit", "stunRange", "stunTime" }

    local function preset(name)
      if X.presets[name] then return X.presets[name] end
      for _, p in pairs(X.presets) do
        if p.tag == name then return p end
      end
      return nil
    end

    -- An explosion: a preset's numbers (based_on, frag when left out), then the ones given.
    function X.register(def)
      if type(def) ~= "table" or type(def.id) ~= "string" or def.id == "" then
        warn("an explosion needs an id")
        return nil
      end
      local base = preset(def.based_on or "frag")
      if not base then
        warn("explosion " .. def.id .. ": there is no preset " .. tostring(def.based_on) .. "; it is based on frag")
        base = X.presets.frag
      end
      local e = {}
      for k, v in pairs(base) do e[k] = v end
      for k, v in pairs(def) do e[k] = v end
      e.tag = def.id
      X.defs[def.id] = e
      return e
    end

    -- Inside the game's createExplosion, for a registered tag: the explosion as the game's own
    -- branches make it, and the sound it plays.
    function X.make(tag, x, y, dad)
      local e = X.defs[tag]
      local list = player.explosions
      local ex = { tag = tag, x = x, y = y, currentFrame = 1, dad = dad }
      for _, f in ipairs(FIELDS) do ex[f] = e[f] end
      list[#list + 1] = ex
      if (tonumber(e.crater) or 0) > 0 and type(createCrater) == "function" then
        createCrater(ex, e.crater)
      end
      local name = e.sound
      if type(name) == "string" and (tonumber(e.sounds) or 1) > 1 then
        name = name .. randNbr(e.sounds)
      end
      local sound = type(name) == "string" and _G[name] or nil
      if not (sound and sound.clone) then sound = _G["fragSound" .. randNbr(3)] end
      local events = _G.QM.Events
      if events and events.emit then events.emit("explosion", tag, x, y) end
      return sound:clone()
    end

    local function number()
      G.nextNumber = G.nextNumber + 1
      return G.nextNumber
    end

    -- Each kind's defaults: the game's own for that weapon.
    KINDS = {
      thrown = { sprite = true, range = 1000, cooldown = 120, tension = 40, sound = "tossFrag",
        explosion = "mineExplosion" },
      rifle = { sprite = true, range = 1000, cooldown = 180, sound = "rifleGrenadeEquip",
        explosion = "rifleGrenadeExplosion" },
      mortar = { cooldown = 180, sound = "deployMortar", explosion = "mortarExplosion", shots = 6, spread = 150 },
      artillery = { cooldown = 300, sound = "barrage", sounds = 5, explosion = "artyExplosion", shots = 5,
        spread = 250, called = true },
      airstrike = { cooldown = 240, sound = "airStrike", sounds = 5, explosion = "bombExplosion", shots = 1,
        spread = 100, plane = "alliedPlaneSprite", called = true },
      drop = { cooldown = 300, sound = "callAccepted", shots = 3, plane = "alliedCargoSprite", called = true,
        noExplosion = true },
    }

    -- A grenade. Returns its table: id, type (its grenadeType), prompt (its mousePrompt).
    function G.register(def)
      if type(def) ~= "table" or type(def.id) ~= "string" or def.id == "" then
        warn("ordnance needs an id")
        return nil
      end
      if G.byId[def.id] then
        warn("ordnance " .. def.id .. " is registered twice; the second one is ignored")
        return G.byId[def.id]
      end
      -- kind: "thrown" (a hand grenade, as the frag), "rifle" (fired from a rifle that can take
      -- rifle grenades, as the game's rifle grenade), "mortar" (a soldier's mortar rounds, as
      -- the game's mortar) or "artillery" (a barrage called on a spot).
      -- "airstrike" (a plane's bombs or strafing on a spot) and "drop" (a supply crate by
      -- parachute) are called in too.
      local kind = def.kind or "thrown"
      local base = KINDS[kind]
      if not base then
        warn("ordnance " .. def.id .. ": kind must be \"thrown\", \"rifle\", \"mortar\", \"artillery\", \"airstrike\" or \"drop\"; it is thrown")
        kind, base = "thrown", KINDS.thrown
      end
      if base.sprite and type(def.sprite) ~= "string" then
        warn("ordnance " .. def.id .. " needs a sprite (the name of an image)")
        return nil
      end
      local g = {
        id = def.id,
        kind = kind,
        sprite = def.sprite,
        explosion = def.explosion,
        on_blast = base.sprite and def.on_blast or nil,
        range = tonumber(def.range) or base.range,
        cooldown = tonumber(def.cooldown) or base.cooldown,
        tension = tonumber(def.tension) or base.tension,
        -- A thrown grenade's turn rate in the air (the frag's own, 0.1 to 1 at random, unless
        -- set); 0 flies pointing the way it was thrown.
        spin = kind == "thrown" and tonumber(def.spin) and math.max(0, tonumber(def.spin)) or nil,
        -- A thrown grenade that goes off where it lands, as an impact fuse, instead of bouncing.
        impact = kind == "thrown" and def.impact == true or nil,
        sound = def.sound or base.sound,
        sounds = tonumber(def.sounds) or (def.sound and 1 or base.sounds),
        -- What a throw uses up (frags unless set). The game's own code counts a rifle
        -- grenade or a mortar round.
        uses = kind == "thrown" and (def.uses == nil and "frags" or def.uses) or nil,
        shots = math.max(1, math.floor(tonumber(def.shots) or base.shots or 1)),
        spread = math.max(0, tonumber(def.spread) or base.spread or 0),
        delay = tonumber(def.delay),
        interval = tonumber(def.interval),
        -- The plane that flies over (an image's name), for an air strike or a drop.
        plane = def.plane or base.plane,
        -- A drop: the game's loot names its crate holds (the game's own mix unless set), and
        -- function(x, y, soldier) when it is opened.
        contents = type(def.contents) == "table" and #def.contents > 0 and def.contents or nil,
        on_open = type(def.on_open) == "function" and def.on_open or nil,
        name = def.name,
        description = def.description,
        icon = def.icon,
      }
      if def.on_blast and not base.sprite then
        warn("ordnance " .. def.id .. ": on_blast is for thrown and rifle grenades; a " .. kind .. " goes off with its explosion")
      end
      if def.impact and kind ~= "thrown" then
        warn("ordnance " .. def.id .. ": impact is for a thrown grenade; a " .. kind .. " goes off as the game's own")
      end
      if def.spin ~= nil and kind ~= "thrown" then
        warn("ordnance " .. def.id .. ": spin is for a thrown grenade; a " .. kind .. " flies as the game's own")
      end
      if not g.explosion and type(g.on_blast) ~= "function" then
        g.explosion = base.explosion
      end
      if base.noExplosion then g.explosion = nil end
      g.type = number()
      g.prompt = number()
      G.byId[g.id] = g
      G.byType[g.type] = g
      G.byPrompt[g.prompt] = g
      if type(def.skill) == "table" then G.giveSkill(def.skill.class, def.skill.slot, g, def.skill) end
      -- Each kind stands in for the game's own of that kind.
      for key, spec in pairs(SWAP_KEYS) do
        if def[key] then
          if kind == spec[1] then
            G.swapWith(key, def[key], g)
          else
            warn("ordnance " .. def.id .. " is " .. kind .. "; " .. key .. " is for kind = \"" .. spec[1] .. "\"")
          end
        end
      end
      if G.ready then G.install() end
      return g
    end

    -- A class skill uses the ordnance: class is the soldier's spec ("light", "rifle", ...),
    -- slot its skill (1 to 3, the game's; 4 or more, new ones). opts: unlock (the skill level
    -- it comes with), icon (an image's name), name and description (its mouse-over text).
    function G.giveSkill(class, slot, g, opts)
      if type(g) == "string" then g = G.byId[g] end
      local S = _G.QM.Skills
      if type(class) ~= "string" or not tonumber(slot) or not g or not S then
        warn("giveSkill needs a class, a skill slot and an ordnance entry")
        return
      end
      opts = type(opts) == "table" and opts or {}
      G.skills[class .. ":" .. tonumber(slot)] = g
      S.set({ class = class, slot = tonumber(slot), ordnance = g, unlock = opts.unlock, icon = opts.icon,
        name = opts.name, description = opts.description, cooldown = g.cooldown })
    end

    -- AI soldiers of a side ("allied", "axis" or "both") who would throw a frag throw this.
    function G.swapFrag(side, g)
      if type(g) == "string" then g = G.byId[g] end
      if not g then return end
      if side == "both" then
        G.swaps.allied, G.swaps.axis = g, g
      else
        G.swaps[side] = g
      end
    end

    -- A side ("allied", "axis" or "both") uses this one wherever it would use the game's own:
    -- key is the register key (replaces_rifle_grenade, replaces_mortar, ...).
    function G.swapWith(key, side, g)
      if type(g) == "string" then g = G.byId[g] end
      local spec = SWAP_KEYS[key]
      local list = spec and G[spec[2]]
      if not g or not spec or g.kind ~= spec[1] or not list then return end
      if side == "both" then
        list.allied, list.axis = g, g
      else
        list[side] = g
      end
      if G.ready then G.install() end
    end
    function G.swapRifle(side, g) G.swapWith("replaces_rifle_grenade", side, g) end
    function G.swapMortar(side, g) G.swapWith("replaces_mortar", side, g) end
    function G.swapArtillery(side, g) G.swapWith("replaces_artillery", side, g) end
    function G.swapBombing(side, g) G.swapWith("replaces_bombing", side, g) end
    function G.swapStrafing(side, g) G.swapWith("replaces_strafing", side, g) end
    function G.swapDrop(side, g) G.swapWith("replaces_drop", side, g) end

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

    -- A grenade's own sound, one of its variants (name1, name2, ...) when it has several.
    local function own(g, dude)
      local n = tonumber(g.sounds) or 1
      sound(n > 1 and (g.sound .. randNbr(n)) or g.sound, dude)
    end

    -- A class skill that uses ordnance (QM.Skills calls it): armed, the next click on the map
    -- throws, fires or calls it in.
    function G.arm(dude, g, skill)
      if dude.health > 10 then
        if (dude["skill" .. skill .. "Timer"] or 0) <= 0 then
          mousePrompt = g.prompt
          mousePrompter = specID
          mousePromptLock = 1
          G.armedSlot = skill
          if g.kind == "rifle" then
            -- As the game's rifle grenade skill: the soldier raises his rifle.
            dude.state, dude.aim = "aim", 1
            own(g, dude)
          end
        else
          no(dude)
        end
      end
      return true
    end

    local function hasKind(kind)
      for _, g in pairs(G.byId) do
        if g.kind == kind then return true end
      end
      return false
    end
    local function hasRifle() return hasKind("rifle") end

    -- After love.load (and again when something is registered later): the wraps the
    -- registered ordnance and skills need, each once.
    function G.install()
      G.ready = true
      if not G.queueInstalled and (hasKind("artillery") or hasKind("airstrike") or hasKind("drop")) then
        G.queueInstalled = true
        if type(delayUpdate) == "function" then
          local game = delayUpdate
          delayUpdate = function(dt, ...)
            pcall(G.tick, dt)
            return game(dt, ...)
          end
        else
          warn("the game has no delayUpdate; what is called in never comes")
        end
        if type(mapReset) == "function" then
          local game = mapReset
          mapReset = function(...)
            G.queue, G.landing = {}, {}
            return game(...)
          end
        end
      end
      if not G.airInstalled and hasKind("airstrike") then
        G.airInstalled = true
        -- The side's bombing runs (the Allies' bomber, the Axis Stukas) and strafing runs (the
        -- Allies' fighters and P-47s, the Axis fighters): every hit, queued or set off at once,
        -- is the swap's explosion.
        local runs = {
          alliedBomb = { "allied", "bombSwaps" }, stukaStrafe = { "axis", "bombSwaps" },
          alliedStrafe = { "allied", "strafeSwaps" }, p47Strafing = { "allied", "strafeSwaps" },
          axisStrafe = { "axis", "strafeSwaps" },
        }
        for name, run in pairs(runs) do
          local game = _G[name]
          if type(game) == "function" then
            _G[name] = function(...)
              local g = G[run[2]][run[1]]
              if not g or type(createExplosion) ~= "function" then return game(...) end
              local list = player.delayedAction
              local n = #list
              local real = createExplosion
              createExplosion = function(tag, ...) return real(g.explosion, ...) end
              local r = { pcall(game, ...) }
              createExplosion = real
              for i = n + 1, #list do
                local a = list[i]
                if a and a.action == "createExplosion" then a.arg1 = g.explosion end
              end
              if not r[1] then error(r[2], 0) end
              return (table.unpack or unpack)(r, 2)
            end
          end
        end
      end
      if not G.dropInstalled and hasKind("drop") then
        G.dropInstalled = true
        for _, name in ipairs({ "cargoCratezButton", "cargoCrateOpening" }) do
          local game = _G[name]
          if type(game) == "function" then
            _G[name] = function(...)
              return G.openWith(game, ...)
            end
          end
        end
      end
      if not G.artyInstalled and hasKind("artillery") then
        G.artyInstalled = true
        -- The game's barrages queue their shells' createExplosion("artyExplosion") in these;
        -- a side's swap changes the explosion.
        for name, side in pairs({ alliedArtyIncoming = "allied", germanArtyIncoming = "axis" }) do
          local game = _G[name]
          if type(game) == "function" then
            _G[name] = function(...)
              local list = player.delayedAction
              local n = #list
              local r = { game(...) }
              local g = G.artySwaps[side]
              local a = list[#list]
              if g and #list > n and a and a.action == "createExplosion" and a.arg1 == "artyExplosion" then
                a.arg1 = g.explosion
              end
              return (table.unpack or unpack)(r)
            end
          end
        end
      end
      if not G.rifleInstalled and hasRifle() and type(rifling) == "function" then
        local game = rifling
        G.gameRifling = game
        -- The game's delayed actions call _G["rifling"], the player's click rifling(...): both
        -- come here. A side's swap tags the rifle grenade the game just fired.
        rifling = function(dude, x, y, ...)
          local g = type(dude) == "table" and G.rifleSwaps[dude.faction] or nil
          return G.fireWith(game, g, dude, x, y, ...)
        end
        G.rifleInstalled = true
      end
      local S = _G.QM.Skills
      if S and S.install then S.install() end
    end

    -- The game's rifling fires a rifle grenade (when the soldier still aims); g, when given,
    -- is the one it becomes: its sprite, and its explosion where it lands.
    function G.fireWith(game, g, dude, x, y, ...)
      local list = player.particles
      local n = #list
      local r = { game(dude, x, y, ...) }
      local part = list[#list]
      if g and #list > n and part and part.typez == "rifleGrenade" then
        local img = _G[g.sprite]
        if img and img.getWidth then
          part.sprite = g.sprite
          part.xoffset, part.yoffset = img:getWidth() / 2, img:getHeight() / 2
        elseif not g.warned then
          warn("ordnance " .. g.id .. ": there is no image " .. g.sprite .. "; it flies as the game's rifle grenade")
          g.warned = true
        end
        part.qmOrdnance = g.id
        local events = _G.QM.Events
        if events and events.emit then events.emit("ordnance_fired", g.id, dude) end
      end
      return (table.unpack or unpack)(r)
    end

    -- A class skill's rifle grenade, at the spot clicked: as the game's rifle grenade skill
    -- does it, through the game's rifling (which also starts skill 2's cooldown: put back).
    function G.fire(dude, g, x, y)
      local game = G.gameRifling or rifling
      if type(game) ~= "function" then
        warn("ordnance " .. g.id .. ": the game has no rifling; the rifle grenade is not fired")
        return false
      end
      local keep = dude.skill2Timer
      dude.state, dude.aim = "aim", 1
      G.fireWith(game, g, dude, x, y)
      dude.skill2Timer = keep
      return true
    end

    -- The click after a rifle grenade skill is armed, as the game's (mousePrompt 3) does it.
    local function clickRifle(g, dude, x, y)
      local shot = distanceCheck(dude.x, dude.y, x + player.screenx, y + player.screeny)
      local capable = dude.weapon and (tonumber(dude.weapon.rgCapable) or 0) > 0
      if dude.death ~= 1 and dude.health > 10 and shot < g.range and capable then
        local fudge = math.max(1, 100 - (tonumber(dude.xp) or 0))
        local fx, fy = randNbr(fudge * 2) - fudge, randNbr(fudge * 2) - fudge
        if G.fire(dude, g, x + player.screenx + fx, y + player.screeny + fy) then
          dude["skill" .. (G.armedSlot or 2) .. "Timer"] = g.cooldown
          dude.actionTimer = 3
        end
      else
        dude.state, dude.aim = "idle", 0
        no(dude)
        if shot >= g.range and player.textCue then
          player.textCue[#player.textCue + 1] = { name = tooFarTxt, x = x + player.screenx, y = y + player.screeny, alpha = 255 }
        end
      end
    end

    local function blocked(fn, dude)
      return type(_G[fn]) == "function" and _G[fn](dude) == true
    end

    -- The click after a mortar skill is armed, as the game's (mousePrompt 4) does it: not
    -- indoors, under a camouflage net or under a tree; then the soldier sets up and fires.
    local function clickMortar(g, dude, x, y)
      local wx, wy = x + player.screenx, y + player.screeny
      local why = blocked("insideBuilding", dude) and "noIndoorTxt" or blocked("underCamNet", dude) and "noCamTxt"
        or blocked("underTree", dude) and "noUnderTreeTxt" or nil
      local far = g.range and distanceCheck(dude.x, dude.y, wx, wy) >= g.range
      if why or far or dude.death == 1 or dude.health <= 10 then
        no(dude)
        if why and type(floatyText) == "function" then floatyText(dude, _G[why]) end
        if far and player.textCue then
          player.textCue[#player.textCue + 1] = { name = tooFarTxt, x = wx, y = wy, alpha = 255 }
        end
        return
      end
      local temp = distanceCheck(dude.x, dude.y, wx, wy) / 8
      local fudge = temp - randNbr(math.max(1, temp * 2))
      dude.targetx, dude.targety = dude.x, dude.y
      dude.state = "mortar1"
      dude.fragx, dude.fragy = wx + fudge, wy + fudge
      dude.stateTimer = randNbr(200) / 100
      own(g, dude)
      sound("mortarTalk" .. randNbr(5), dude)
      dude["skill" .. (G.armedSlot or 3) .. "Timer"] = g.cooldown
      dude.mortar = g.shots
      dude.actionTimer = 3
      dude.qmMortar = g.id
    end

    -- A mortar round leaves (a hook in the game's mortar state machine, after it queued the
    -- round's createExplosion): the soldier's own mortar, or his side's swap, changes it.
    function G.mortarRound(dude, action)
      if type(dude) ~= "table" or type(action) ~= "table" then return false end
      local g = dude.qmMortar and G.byId[dude.qmMortar] or G.mortarSwaps[dude.faction]
      if not g then return false end
      action.arg1 = g.explosion
      if g.spread ~= 150 and dude.fragx and dude.fragy then
        local s = math.floor(g.spread)
        action.arg2 = dude.fragx + (s > 0 and (randNbr(s * 2) - s) or 0)
        action.arg3 = dude.fragy + (s > 0 and (randNbr(s * 2) - s) or 0)
      end
      -- The game counts the round down after this: the last one ends the soldier's own.
      if dude.qmMortar and (tonumber(dude.mortar) or 0) <= 1 then dude.qmMortar = nil end
      local events = _G.QM.Events
      if events and events.emit then events.emit("ordnance_fired", g.id, dude) end
      return true
    end

    -- QuarterMaster's own delayed actions, run with the game's (delayUpdate).
    function G.after(seconds, fn)
      G.queue[#G.queue + 1] = { t = seconds, fn = fn }
    end

    function G.tick(dt)
      local due = {}
      for i = #G.queue, 1, -1 do
        local q = G.queue[i]
        q.t = q.t - (tonumber(dt) or 0)
        if q.t <= 0 then
          table.remove(G.queue, i)
          due[#due + 1] = q
        end
      end
      for i = #due, 1, -1 do
        local ok, err = pcall(due[i].fn)
        if not ok then warn("something called in failed: " .. tostring(err)) end
      end
      G.land()
    end

    -- A plane flies over, as the game's: kind is the game's plane it starts as; g.plane, when
    -- the game has that image, is what it looks like.
    local function flyOver(g, x, kind)
      if type(addPlane) ~= "function" or not player.planes then return end
      local part = {}
      addPlane(part, kind, x - 1800, 0, 0)
      if g.plane and g.plane ~= kind then
        local img = _G[g.plane]
        if img and img.getWidth then
          part.sprite = g.plane
        elseif not g.warnedPlane then
          warn("ordnance " .. g.id .. ": there is no image " .. g.plane .. "; the game's plane flies over")
          g.warnedPlane = true
        end
      end
      table.insert(player.planes, part)
    end

    -- An air strike on (x, y): the plane's sound, the plane, then its shots, each up to
    -- spread from the spot, interval seconds apart (one bomb, as the game's bomber, unless
    -- set otherwise).
    local function airstrike(g, x, y)
      local delay = g.delay or (randNbr(20) + 5)
      local s = math.floor(g.spread)
      local interval = g.interval or (1 / 15)
      G.after(delay, function() sound("alliedPlane" .. randNbr(2)) end)
      G.after(delay + 1, function() flyOver(g, x, "alliedPlaneSprite") end)
      for i = 1, g.shots do
        local hx = x + (s > 0 and (randNbr(s * 2) - s) or 0)
        local hy = y + (s > 0 and (randNbr(s * 2) - s) or 0)
        if hy > 720 or hy < 0 then hy = randNbr(720) end
        G.after(delay + 2 + (i - 1) * interval, function() createExplosion(g.explosion, hx, hy, nil) end)
      end
    end

    -- A supply drop on (x, y): the cargo plane, then a parachute that lands as the game's
    -- supply crate; the crate is the drop's from then on.
    local function drop(g, x, y)
      local delay = g.delay or (randNbr(10) + 5)
      G.after(delay, function() flyOver(g, x, "alliedCargoSprite") end)
      G.after(delay + 2, function()
        local part = {}
        addParticle(part, "parachute", x, y, 0)
        table.insert(player.topParticles, part)
        -- The game's crate takes the parachute's faction as its item count.
        part.faction = g.shots
        part.qmDrop = g.id
        local img = g.sprite and _G[g.sprite]
        if img and img.getWidth then
          part.sprite = g.sprite
          part.xoffset, part.yoffset = img:getWidth() / 2, img:getHeight() / 2
        end
        G.landing[#G.landing + 1] = part
        sound("paraDrop")
      end)
    end

    -- A parachute that landed: the game put its crate where it was.
    function G.land()
      if #G.landing == 0 then return end
      local falling = {}
      for _, p in ipairs(player.topParticles or {}) do falling[p] = true end
      for i = #G.landing, 1, -1 do
        local part = G.landing[i]
        if not falling[part] then
          table.remove(G.landing, i)
          for _, l in ipairs(player.lootables or {}) do
            if l.name == "cargoCrate" and not l.qmDrop and l.x == part.x and l.y == part.y then
              l.qmDrop = part.qmDrop
              break
            end
          end
        end
      end
    end

    -- A supply crate is opened (the player's button, or a soldier's own): a drop's crate, or
    -- the Allies' own when a drop stands in for them, holds what the drop says.
    function G.openWith(game, ...)
      -- One opening at a time: a crate function calling the other is the same opening.
      if G.opening then return game(...) end
      local held = {}
      for _, l in ipairs(player.lootables or {}) do
        if l.name == "cargoCrate" then
          local g = (l.qmDrop and G.byId[l.qmDrop]) or G.dropSwaps.allied
          if g and (g.contents or g.on_open) then
            held[l] = { g = g, argument = l.argument }
            if g.contents then l.argument = 0 end
          end
        end
      end
      if next(held) == nil then return game(...) end
      G.opening = true
      local r = { pcall(game, ...) }
      G.opening = false
      local still = {}
      for _, l in ipairs(player.lootables or {}) do still[l] = true end
      local soldier = (...)
      for l, h in pairs(held) do
        if still[l] then
          l.argument = h.argument
        else
          local ok, err = pcall(G.opened, h.g, l, h.argument, soldier)
          if not ok then warn("ordnance " .. h.g.id .. ": its crate failed: " .. tostring(err)) end
        end
      end
      if not r[1] then error(r[2], 0) end
      return (table.unpack or unpack)(r, 2)
    end

    -- What a crate holds, laid out around it as the game lays out its own.
    function G.opened(g, crate, amount, soldier)
      if g.contents then
        local n = crate.qmDrop and g.shots or (tonumber(amount) or g.shots)
        for _ = 1, n do
          local name = g.contents[randNbr(#g.contents)]
          -- As the game does: "loot_ammo" is one of its special ammo kinds.
          if name == "loot_ammo" and type(ammoLoot) == "table" and #ammoLoot > 0 then
            name = ammoLoot[randNbr(#ammoLoot)]
          end
          local item = { name = name, x = crate.x + (randNbr(100) - 50), y = crate.y + (randNbr(100) - 50),
            flagged = 1, faction = "allied", argument = "", size = 10, life = 60,
            rotation = math.rad(randNbr(360)) }
          if name == "loot_boysRifle" then item.argument = 10 end
          player.lootables[#player.lootables + 1] = item
          if type(screenSanity) == "function" then screenSanity(item) end
        end
      end
      if g.on_open then
        local ok, err = pcall(g.on_open, crate.x, crate.y, soldier)
        if not ok then warn("ordnance " .. g.id .. ": on_open failed: " .. tostring(err)) end
      end
    end

    -- Something called in on (x, y): an artillery barrage, an air strike or a supply drop.
    -- dude, when given, is who called it.
    function G.callIn(g, x, y, dude)
      if type(g) == "string" then g = G.byId[g] end
      if not g or not (KINDS[g.kind] or {}).called then
        warn("callIn needs artillery, an air strike or a drop")
        return false
      end
      if not G.queueInstalled then G.install() end
      if g.kind == "artillery" then
        G.barrage(g, x, y)
      elseif g.kind == "airstrike" then
        airstrike(g, x, y)
      else
        drop(g, x, y)
      end
      local events = _G.QM.Events
      if events and events.emit then events.emit("ordnance_fired", g.id, dude) end
      return true
    end

    -- An artillery barrage on (x, y), as the game's: the shells walk from the spot by up to
    -- spread each, each one a far shot, its whistle 3 seconds later and its explosion 2
    -- seconds after that.
    function G.barrage(g, x, y)
      local delay = g.delay or (randNbr(10) + 5)
      local s = math.floor(g.spread)
      local bx, by = x, y
      for i = 1, g.shots do
        if s > 0 then
          bx = bx + randNbr(s * 2) - s
          by = by + randNbr(s * 2) - s
        end
        if by > 720 or by < 0 then by = randNbr(720) end
        local t = delay + i * (g.interval or randNbr(10))
        local sx, sy = bx, by
        G.after(t, function() sound("farShot" .. randNbr(3)) end)
        G.after(t + 3, function() sound("incoming" .. randNbr(8)) end)
        G.after(t + 5, function() createExplosion(g.explosion, sx, sy, nil) end)
      end
    end

    -- The click after a called-in skill is armed: the radio call, and what it calls in.
    local function clickCalled(g, dude, x, y)
      local wx, wy = x + player.screenx, y + player.screeny
      if dude.death == 1 or dude.health <= 10 or (g.range and distanceCheck(dude.x, dude.y, wx, wy) >= g.range) then
        no(dude)
        return
      end
      own(g, dude)
      if G.callIn(g, wx, wy, dude) then
        dude["skill" .. (G.armedSlot or 3) .. "Timer"] = g.cooldown
      end
    end

    -- The click after the skill is armed: the toss, as the game's frag toss does it.
    function G.click(x, y)
      local g = G.byPrompt[mousePrompt]
      if not g then return false end
      local idx = convertedId(mousePrompter)
      local dude = idx ~= -1 and player.deployedTroops[idx] or nil
      if dude and g.kind == "rifle" then
        clickRifle(g, dude, x, y)
      elseif dude and g.kind == "mortar" then
        clickMortar(g, dude, x, y)
      elseif dude and (KINDS[g.kind] or {}).called then
        clickCalled(g, dude, x, y)
      elseif dude then
        sound("fragReady" .. randNbr(14), dude, 2.3)
        dude.targetx, dude.targety = dude.x, dude.y
        dude.actionTimer, dude.stateTimer, dude.state = 3, 3, "idle"
        if dude.death ~= 1 and dude.health > 10 then
          local shot = distanceCheck(dude.x, dude.y, x + player.screenx, y + player.screeny)
          local range = g.range
          if type(hasPerk) == "function" and hasPerk(dude, 1) then range = range * 1.15 end
          if shot < range and dude.pinned ~= 1 then
            shot = shot / 8
            dude.state = "fragThrow1"
            dude.fragx = x + player.screenx + (randNbr(shot * 2) - shot)
            dude.fragy = y + player.screeny + (randNbr(shot * 2) - shot)
            dude.grenadeType = g.type
            dude.stateTimer, dude.actionTimer, dude.targetPointer = 0.1, 5, 0
            dude["skill" .. (G.armedSlot or 2) .. "Timer"] = g.cooldown
          else
            no(dude)
            if shot >= range and player.textCue then
              player.textCue[#player.textCue + 1] = { name = tooFarTxt, x = x, y = y, alpha = 255 }
            end
          end
        end
      end
      buttoned = 1
      abilityDisplay = 0
      mousePrompter = 0
      specID = -99
      mousePrompt = 0
      G.armedSlot = nil
      return true
    end

    -- A soldier's throw: one of ours by grenadeType, or a frag a swap stands in for.
    function G.throwFor(i)
      local dude = player.deployedTroops[i]
      if not dude then return false end
      local g = G.byType[dude.grenadeType]
      if not g and dude.grenadeType == 1 then
        g = G.swaps[dude.faction]
        if g then dude.grenadeType = g.type end
      end
      if not g then return false end
      G.throw(i, g, dude.fragx, dude.fragy)
      return true
    end

    -- The throw itself, as the game's throwFrag does it, with the grenade's sprite.
    function G.throw(i, g, x, y)
      local dude = player.deployedTroops[i]
      dude.rotation = math.atan2(dude.y - y, dude.x - x) + 1.5
      local part = {}
      addParticle(part, "fragGrenade", dude.x, dude.y, dude.rotation)
      local img = _G[g.sprite]
      if img and img.getWidth then
        part.sprite = g.sprite
        part.xoffset, part.yoffset = img:getWidth() / 2, img:getHeight() / 2
      else
        if not g.warned then
          warn("ordnance " .. g.id .. ": there is no image " .. g.sprite .. "; it flies as a frag")
          g.warned = true
        end
        part.sprite = "frag"
      end
      if g.spin then
        part.spin = g.spin
        -- No spin: it keeps the soldier's facing, the top of its image forward.
        if g.spin == 0 then part.innerRotation = part.rotationz end
      end
      if g.impact then part.qmImpact = true end
      part.qmOrdnance = g.id
      table.insert(player.particles, part)
      part.targetx, part.targety = x, y
      part.lob = distanceCheck(part.x, part.y, x, y)
      if (type(insideBunker) == "function" and insideBunker(dude))
        or (type(coveredBuilding) == "function" and coveredBuilding(dude)) then
        part.wallImmune = 0.3
      end
      dude.state, dude.aim, dude.stateTimer, dude.actionTimer = "idle", 0, 0.5, 1
      dude.targetPointer, dude.temporaryStop = 0, 1
      if g.uses then dude[g.uses] = (tonumber(dude[g.uses]) or 0) - 1 end
      own(g, dude)
      local events = _G.QM.Events
      if events and events.emit then events.emit("ordnance_fired", g.id, dude) end
    end

    -- The grenade went off: its explosion, or the mod's own code.
    function G.blast(particle, x, y)
      local g = particle and particle.qmOrdnance and G.byId[particle.qmOrdnance]
      if not g then return false end
      if type(g.on_blast) == "function" then
        local ok, err = pcall(g.on_blast, x, y, particle)
        if not ok then warn("ordnance " .. g.id .. ": on_blast failed: " .. tostring(err)) end
      else
        createExplosion(g.explosion, x, y)
      end
      if g.tension then player.tension = g.tension end
      local events = _G.QM.Events
      if events and events.emit then events.emit("ordnance_blast", g.id, x, y) end
      return true
    end
  end
end
