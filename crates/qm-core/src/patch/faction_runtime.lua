-- QuarterMaster faction runtime.
-- Applies the factions QuarterMaster compiled from faction.toml files. It runs inside
-- love.load, after vanilla assets, the auto-asset loader and the save have loaded.
-- Everything it changes falls back to vanilla for anything a faction doesn't cover.
-- Lua 5.1 / LuaJIT. All state lives in one table (R) to stay clear of local and upvalue limits.

local R = {
  factions = {},      -- side -> compiled faction
  weapons = {},       -- weapon tag -> definition (reskins and new weapons)
  swaps = {},         -- side -> { vanilla tag -> faction tag }
  pools = {},         -- pool name -> { add = {...}, remove = { tag = true } }
  classes = {},       -- spec -> class definition (allied side)
  hooks = {},         -- list of hook tables from hooks.lua files
  vehicle_names = {}, -- vehicle tag -> display name
  counts = {},
}

-- Weapon lists of the vanilla random pools (v0.5.3b, urb_engine.lua getRandom* functions).
R.VANILLA_POOLS = {
  allied = { fn = "getRandomAlliedGun", list = {"shotgun", "m3", "bar", "sten", "chauchat", "mas36", "stenMKVI", "tommy", "vicker", "m1919", "bren", "m2Carbine", "m2CarbineA1", "m1CarbinePara", "scopedEnfield", "scopedGarand", "scopedSpringfield", "1917", "t22", "t20", "m1941", "matchGarand", "m1Carbine", "m1", "springfield"} },
  allied_sniper = { fn = "getRandomAlliedSniper", list = {"scopedEnfield", "scopedGarand", "scopedSpringfield", "1917"} },
  grunt = { fn = "getRandomGruntGun", list = {"shotgun", "m3", "bar", "tommy", "m1919", "m1CarbinePara", "matchGarand", "m1Carbine", "m1", "springfield"} },
  para = { fn = "getRandomParaGun", list = {"shotgun", "m3", "sten", "stenMKVI", "tommy", "bar", "m1CarbinePara", "scopedGarand", "m1Carbine", "m1"} },
  resistance = { fn = "getRandomResistanceGun", list = {"shotgun", "sten", "bren", "1917", "mas36", "chauchat", "m1Carbine", "scopedEnfield", "mp40", "k98", "g98"} },
  smg = { fn = "getRandomSMG", list = {"m3", "sten", "stenMKVI", "tommy"} },
  axis = { fn = "getRandomGermGun", list = {"k98", "scopedK98", "g43", "mg42", "mg34", "mp40", "stg44", "fg42", "ppsh41", "m38", "g33", "svt40", "vg15", "irstg44", "zb26", "g98"} },
  axis_crew = { fn = "getRandomGermCrewGun", list = {"k98", "mp40", "stg44", "ppsh41", "m38", "vg15"} },
}

-- Vanilla unit tag -> schema variant name, per side.
R.VARIANTS = {
  allied = { allied = "base", sarge = "sarge", para = "para", french = "french" },
  axis = { axis = "base", officer = "officer", volks = "volks" },
}

R.STAT_KEYS = { "action", "class", "ammo", "maxAmmo", "range", "damage", "recoil", "bulk", "rof", "rofTime", "jam", "rgCapable" }

function R.log(msg)
  print("[QM][Faction] " .. tostring(msg))
end

function R.try(what, fn, ...)
  local ok, err = pcall(fn, ...)
  if not ok then
    R.log("ERROR in " .. what .. ": " .. tostring(err))
  end
  return ok
end

function R.rand(n)
  if n < 1 then return 1 end
  if type(randNbr) == "function" then
    local ok, v = pcall(randNbr, n)
    if ok and type(v) == "number" and v >= 1 and v <= n then return v end
  end
  return math.random(1, n)
end

function R.pick(list)
  return list[R.rand(#list)]
end

function R.copy(v, seen)
  if type(v) ~= "table" then return v end
  seen = seen or {}
  if seen[v] then return seen[v] end
  local out = {}
  seen[v] = out
  for k, x in pairs(v) do out[R.copy(k, seen)] = R.copy(x, seen) end
  return out
end

function R.read(path)
  local f = io.open(path, "rb")
  if not f then return nil end
  local bytes = f:read("*a")
  f:close()
  return bytes
end

function R.file_data(root, rel)
  local bytes = R.read(root .. "/" .. rel)
  if not bytes then
    R.log("missing file " .. root .. "/" .. rel)
    return nil
  end
  local ok, fd = pcall(love.filesystem.newFileData, bytes, rel:match("[^/]+$") or rel)
  if ok then return fd end
  R.log("can't read " .. rel .. ": " .. tostring(fd))
  return nil
end

function R.image(root, rel)
  local fd = R.file_data(root, rel)
  if not fd then return nil end
  local ok, img = pcall(love.graphics.newImage, fd)
  if ok and img then return img end
  R.log("can't load image " .. rel .. ": " .. tostring(img))
  return nil
end

function R.sound(root, rel, mode)
  local fd = R.file_data(root, rel)
  if not fd then return nil end
  local ok, src = pcall(love.audio.newSource, fd, mode or "static")
  if ok and src then return src end
  R.log("can't load sound " .. rel .. ": " .. tostring(src))
  return nil
end

function R.ext(rel)
  return (rel:match("%.([^%./]+)$") or ""):lower()
end

-- A choice in the launcher can give a game file to another mod or back to the game; the
-- faction then leaves it alone. `key` is the global's name, or "folder/stem" for a voice.
function R.left_out(f, key)
  return type(f.excluded) == "table" and f.excluded[key] == true
end

-- Sets the image or sound global `name` to the faction's file `rel`. The file is also
-- registered with the asset loader, so a later load of the game's own path gives it too.
function R.set_asset(f, name, value, rel, folder)
  if type(_G.__qm_asset_override) == "function" then
    pcall(_G.__qm_asset_override, name, value, f.root .. "/" .. rel, "faction:" .. f.id,
      (folder or "sprites") .. "/" .. name:lower())
  end
  rawset(_G, name, value)
end

-- Replaces a sound global, keeping the old one's stream/static mode, looping and volume.
-- With `f`, the faction's file is registered as an asset override too.
function R.replace_sound(name, root, rel, f)
  local old = rawget(_G, name)
  local mode, looping, volume = "static", false, nil
  if type(old) == "userdata" then
    pcall(function() mode = old:getType() or mode end)
    pcall(function() looping = old:isLooping() end)
    pcall(function() volume = old:getVolume() end)
  end
  local src = R.sound(root, rel, mode)
  if not src then return false end
  if looping then pcall(function() src:setLooping(true) end) end
  if volume then pcall(function() src:setVolume(volume) end) end
  if f then
    R.set_asset(f, name, src, rel, "sounds")
  else
    rawset(_G, name, src)
  end
  return true
end

-- A global that returns a random member of `list` every time it is read.
function R.sound_pool(name, list)
  if #list == 1 then
    rawset(_G, name, list[1])
  elseif type(_G.__qm_register_pool) == "function" then
    _G.__qm_register_pool(name, list)
  else
    rawset(_G, name, list[1])
  end
end

---------------------------------------------------------------------------------------------
-- Loading each faction's content
---------------------------------------------------------------------------------------------

function R.load_frames(f)
  local n = 0
  for tag, frames in pairs(f.frames or {}) do
    for frame, rel in pairs(frames) do
      local name = tag .. "_" .. frame
      local img = not R.left_out(f, name) and R.image(f.root, rel)
      if img then
        R.set_asset(f, name, img, rel)
        n = n + 1
      end
    end
  end
  R.counts.frames = (R.counts.frames or 0) + n
end

function R.load_sprites_and_sounds(f)
  for name, rel in pairs(f.sprites or {}) do
    local img = not R.left_out(f, name) and R.image(f.root, rel)
    if img then
      if rawget(_G, name) == nil then
        R.log("sprite " .. name .. " is not a vanilla sprite; it is set anyway")
      end
      R.set_asset(f, name, img, rel)
    end
  end
  for name, rel in pairs(f.sounds or {}) do
    if not R.left_out(f, name) then
      R.replace_sound(name, f.root, rel, f)
    end
  end
end

function R.apply_text(f)
  local text = f.text or {}
  -- Partial class strings first, so a whole string from [text] wins.
  for key, value in pairs(text) do
    local base, part = key:match("^(.-)::(%a+)$")
    if base then
      local cur = rawget(_G, base)
      if type(cur) ~= "string" then cur = "" end
      local name, rest = cur:match("^([^,]*),%s*(.*)$")
      if not name then name, rest = cur, "" end
      if part == "name" then
        rawset(_G, base, rest ~= "" and (value .. ", " .. rest) or value)
      elseif part == "description" then
        rawset(_G, base, name ~= "" and (name .. ", " .. value) or value)
      end
    end
  end
  for key, value in pairs(text) do
    if not key:find("::", 1, true) then
      rawset(_G, key, value)
    end
  end
end

function R.load_weapons(f)
  for _, w in ipairs(f.weapons or {}) do
    local tag = w.tag
    local def = { mode = w.mode, based_on = w.based_on, stats = w.stats or {}, faction = f.id }
    if w.top and not R.left_out(f, "top_" .. tag) then
      local img = R.image(f.root, w.top)
      if img then R.set_asset(f, "top_" .. tag, img, w.top) end
    end
    if w.side and not R.left_out(f, "side_" .. tag) then
      local img = R.image(f.root, w.side)
      if img then R.set_asset(f, "side_" .. tag, img, w.side) end
    end
    if w.mode == "new" then
      -- Fall back to the base weapon's look.
      if rawget(_G, "top_" .. tag) == nil then rawset(_G, "top_" .. tag, rawget(_G, "top_" .. w.based_on)) end
      if rawget(_G, "side_" .. tag) == nil then rawset(_G, "side_" .. tag, rawget(_G, "side_" .. w.based_on)) end
      if rawget(_G, "weaponText_" .. tag) == nil then rawset(_G, "weaponText_" .. tag, tag) end
    end
    if w.sounds and #w.sounds > 0 then
      local list = {}
      for _, rel in ipairs(w.sounds) do
        local src = R.sound(f.root, rel, "static")
        if src then list[#list + 1] = src end
      end
      if #list > 0 then
        def.sound = "__qmf_" .. f.id .. "_" .. tag .. "_fire"
        R.sound_pool(def.sound, list)
      end
    end
    if w.sound_name and not def.sound then
      if rawget(_G, w.sound_name) == nil then
        R.log("weapon " .. tag .. ": sound " .. w.sound_name .. " is not a game sound")
      end
      def.sound = w.sound_name
    end
    if w.mode == "reskin" and def.sound then
      -- Read by the tag-sound patch for weapons whose sound vanilla picks by tag.
      _G.__qm_sound_by_tag = _G.__qm_sound_by_tag or {}
      _G.__qm_sound_by_tag[tag] = true
    end
    if w.mag_file then
      local img = R.image(f.root, w.mag_file)
      if img then
        def.mag = "__qmf_" .. f.id .. "_" .. tag .. "_mag"
        rawset(_G, def.mag, img)
      end
    elseif w.mag_name then
      def.mag = w.mag_name
    end
    R.weapons[tag] = def
  end
  for from, to in pairs(f.spawn_swap or {}) do
    R.swaps[f.replaces] = R.swaps[f.replaces] or {}
    R.swaps[f.replaces][from] = to
    R.swap_any = true
  end
  for name, edit in pairs(f.pools or {}) do
    local p = R.pools[name] or { add = {}, remove = {} }
    for _, t in ipairs(edit.add or {}) do p.add[#p.add + 1] = t end
    for _, t in ipairs(edit.remove or {}) do p.remove[t] = true end
    R.pools[name] = p
  end
end

function R.load_classes(f)
  for spec, cls in pairs(f.classes or {}) do
    R.classes[spec] = cls
    if cls.icon then
      local img = R.image(f.root, cls.icon)
      if img then
        -- An icon of another resolution is shown at the size of the one it replaces.
        if type(_G.__qm_sprite_inherit) == "function" then
          _G.__qm_sprite_inherit(rawget(_G, "spec_" .. spec), img, "spec_" .. spec)
        end
        rawset(_G, "spec_" .. spec, img)
      end
    end
  end
end

-- Spreads the faction's voice files over the vanilla voice files they replace.
function R.load_voices(f)
  if type(_G.__qm_auto_asset) ~= "function" then
    R.log("voice replacement needs the QuarterMaster asset loader; voices skipped")
    return
  end
  local n = 0
  for _, dir in ipairs(f.voices or {}) do
    local items = love.filesystem.getDirectoryItems(dir.vanilla_dir) or {}
    for _, item in ipairs(items) do
      local stem, ext = item:match("^(.+)%.([^%.]+)$")
      ext = ext and ext:lower() or ""
      if stem and (ext == "wav" or ext == "ogg" or ext == "mp3") then
        local rel = dir.exact[stem]
        if not rel and dir.fill_missing then
          local prefix, num = stem:match("^(.-)(%d+)$")
          local pool = prefix and prefix ~= "" and dir.pools[prefix]
          if pool and #pool > 0 then
            rel = pool[((tonumber(num) - 1) % #pool) + 1]
          end
        end
        if rel and not R.left_out(f, dir.vanilla_dir .. "/" .. stem) then
          if _G.__qm_auto_asset(f.root, rel, dir.vanilla_dir, stem, R.ext(rel), "stream", "faction:" .. f.id) then
            n = n + 1
          end
        end
      end
    end
  end
  R.counts.voices = (R.counts.voices or 0) + n
end

function R.load_vehicles(f)
  for _, v in ipairs(f.vehicles or {}) do
    for global, rel in pairs(v.parts or {}) do
      local img = not R.left_out(f, global) and R.image(f.root, rel)
      if img then
        if rawget(_G, global) == nil then
          R.log("vehicle sprite " .. global .. " is not a vanilla sprite; it is set anyway")
        end
        R.set_asset(f, global, img, rel)
      end
    end
    if v.debris and #v.debris > 0 then
      local imgs, rels = {}, {}
      for _, rel in ipairs(v.debris) do
        local img = R.image(f.root, rel)
        if img then
          imgs[#imgs + 1] = img
          rels[#rels + 1] = rel
        end
      end
      if #imgs > 0 then
        local i = 1
        while rawget(_G, v.tag .. "_debris" .. i) ~= nil do
          local name, k = v.tag .. "_debris" .. i, ((i - 1) % #imgs) + 1
          if not R.left_out(f, name) then R.set_asset(f, name, imgs[k], rels[k]) end
          i = i + 1
        end
        if i == 1 then
          for k, img in ipairs(imgs) do
            local name = v.tag .. "_debris" .. k
            if not R.left_out(f, name) then R.set_asset(f, name, img, rels[k]) end
          end
        end
      end
    end
    if v.name then R.vehicle_names[v.tag] = v.name end
    if v.spawner and v.call_signs and #v.call_signs > 0 then
      R.wrap_spawner(v.spawner, v.tag, v.call_signs)
    end
  end
end

-- Puts `make(orig)` in place of the game function `name`. QuarterMaster's startup code
-- keeps the wrapper, so it is put back when another mod replaces the function later.
function R.wrap(name, make)
  local wrap = rawget(_G, "__qm_wrap")
  if type(wrap) == "function" then return wrap(name, make) end
  rawset(_G, name, make(rawget(_G, name)))
end

-- Gives a freshly spawned vehicle one of the faction's call signs.
function R.wrap_spawner(fn_name, tag, call_signs)
  if type(rawget(_G, fn_name)) ~= "function" then
    R.log(fn_name .. " not found; call signs for " .. tag .. " skipped")
    return
  end
  R.wrap(fn_name, function(orig) return function(...)
    local vehicles = type(player) == "table" and player.deployedVehicles
    local before = type(vehicles) == "table" and #vehicles or 0
    local result = orig(...)
    vehicles = type(player) == "table" and player.deployedVehicles
    if type(vehicles) == "table" then
      for i = #vehicles, before + 1, -1 do
        local v = vehicles[i]
        if type(v) == "table" and v.tag == tag then v.callSign = R.pick(call_signs) end
      end
    end
    return result
  end end)
end

function R.load_hooks(f)
  if not f.hooks then return end
  local path = f.root .. "/" .. f.hooks
  local src = R.read(path)
  if not src then
    R.log("missing hooks file " .. path)
    return
  end
  local compile = loadstring or load
  local chunk, err = compile(src, "@" .. path)
  if not chunk then
    R.log("hooks.lua of " .. f.id .. " doesn't compile: " .. tostring(err))
    return
  end
  local ok, tbl = pcall(chunk)
  if not ok then
    R.log("hooks.lua of " .. f.id .. " failed: " .. tostring(tbl))
    return
  end
  if type(tbl) ~= "table" then
    R.log("hooks.lua of " .. f.id .. " must return a table of functions")
    return
  end
  tbl.__faction = f
  R.hooks[#R.hooks + 1] = tbl
end

function R.call_hooks(name, ...)
  for _, h in ipairs(R.hooks) do
    local fn = h[name]
    if type(fn) == "function" then
      R.try(h.__faction.id .. " hooks." .. name, fn, ...)
    end
  end
end

---------------------------------------------------------------------------------------------
-- Wrapping vanilla functions
---------------------------------------------------------------------------------------------

-- The soldier carrying a weapon table, if it is already on the field.
function R.owner_of(weapon)
  local troops = type(player) == "table" and player.deployedTroops
  if type(troops) ~= "table" then return nil end
  for i = #troops, 1, -1 do
    local t = troops[i]
    if type(t) == "table" and t.weapon == weapon then return t end
  end
  return nil
end

function R.swap_for(weapon, tag)
  local owner = nil
  local looked = false
  for side, map in pairs(R.swaps) do
    local to = map[tag]
    if to then
      if not looked then
        owner = R.owner_of(weapon)
        looked = true
      end
      if not (owner and owner.faction and owner.faction ~= side) then
        return to
      end
    end
  end
  return nil
end

function R.apply_weapon(weapon, def)
  local stats = def.stats
  for _, k in ipairs(R.STAT_KEYS) do
    if stats[k] ~= nil then weapon[k] = stats[k] end
  end
  if stats.maxAmmo ~= nil and stats.ammo == nil then weapon.ammo = stats.maxAmmo end
  if def.sound then weapon.sound = def.sound end
  if def.mag then weapon.magSprite = def.mag end
end

function R.wrap_weapon_creator()
  if type(rawget(_G, "weaponCreator")) ~= "function" then
    R.log("weaponCreator not found; weapon changes skipped")
    return
  end
  R.wrap("weaponCreator", function(orig) return function(weapon, tag, ...)
    if type(weapon) ~= "table" then return orig(weapon, tag, ...) end
    local t = tag
    if R.swap_any and type(t) == "string" then
      t = R.swap_for(weapon, t) or t
    end
    local def = R.weapons[t]
    local result
    if def and def.mode == "new" then
      result = orig(weapon, def.based_on, ...)
      weapon.tag = t
    else
      result = orig(weapon, t, ...)
    end
    if def then R.apply_weapon(weapon, def) end
    if #R.hooks > 0 then R.call_hooks("on_weapon_created", weapon, t) end
    return result
  end end)
end

function R.wrap_pools()
  for name, edit in pairs(R.pools) do
    local vp = R.VANILLA_POOLS[name]
    if vp and type(rawget(_G, vp.fn)) == "function" then
      -- The faction's pool: the game's weapons it keeps, then the ones it adds.
      local list, kept = {}, 0
      for _, t in ipairs(vp.list) do
        if not edit.remove[t] then
          list[#list + 1] = t
          kept = kept + 1
        end
      end
      for _, t in ipairs(edit.add) do list[#list + 1] = t end
      local added = #edit.add
      R.wrap(vp.fn, function(orig) return function(...)
        if #list == 0 then return orig(...) end
        -- An added weapon comes up as often as it would in the faction's own list.
        if added > 0 and R.rand(added + kept) <= added then
          return edit.add[R.rand(added)]
        end
        -- Otherwise ask the function that was there: the game's, or another mod's version
        -- of it, so that mod's weapons stay in the pool. A removed weapon is drawn again.
        for _ = 1, 8 do
          local ok, tag = pcall(orig, ...)
          if not ok then break end
          if tag ~= nil and not edit.remove[tag] then return tag end
        end
        return R.pick(list)
      end end)
    end
  end
end

function R.wrap_spec_click()
  if type(rawget(_G, "specClick")) ~= "function" then
    R.log("specClick not found; class weapons and bonuses skipped")
    return
  end
  R.wrap("specClick", function(orig) return function(spec, ...)
    local cls = R.classes[spec]
    if not cls or (not cls.bonus and not cls.start_weapon) then
      return orig(spec, ...)
    end
    local dude = nil
    if type(convertedId) == "function" and type(player) == "table" and player.deployedTroops then
      local ok, idx = pcall(convertedId, specID)
      if ok and idx and idx ~= -1 then dude = player.deployedTroops[idx] end
    end
    local snap = nil
    if dude and cls.bonus then
      snap = {
        speed = dude.speed, skill = dude.skill, maxHealth = dude.maxHealth, health = dude.health,
        hthDamage = dude.hthDamage, detectRange = dude.detectRange, mineDetectRange = dude.mineDetectRange,
      }
    end
    local result = orig(spec, ...)
    if dude and dude.spec == spec then
      if snap then
        for k, v in pairs(snap) do dude[k] = v end
        local b = cls.bonus
        if b.speed and dude.speed then dude.speed = dude.speed + b.speed end
        if b.skill and dude.skill then dude.skill = dude.skill + b.skill end
        if b.max_health and dude.maxHealth then
          dude.maxHealth = dude.maxHealth + b.max_health
          if dude.health then dude.health = dude.health + b.max_health end
        end
        if b.hth_damage and dude.hthDamage then dude.hthDamage = dude.hthDamage + b.hth_damage end
        if b.detect_range and dude.detectRange then dude.detectRange = dude.detectRange * b.detect_range end
        if b.mine_detect_range and dude.mineDetectRange then dude.mineDetectRange = dude.mineDetectRange * b.mine_detect_range end
      end
      if cls.start_weapon and type(dude.weapon) == "table" then
        if dude.primarySwap and dude.primarySwap ~= "" then
          -- Carrying an anti-tank rifle: the class weapon waits in primarySwap.
          dude.primarySwap = cls.start_weapon
        else
          _G.weaponCreator(dude.weapon, cls.start_weapon)
        end
      end
    end
    return result
  end end)
end

function R.make_name(pool)
  local last = R.pick(pool.last)
  if pool.format == "full" and pool.first and #pool.first > 0 then
    return R.pick(pool.first) .. " " .. last
  end
  return string.char(64 + R.rand(26)) .. ". " .. last
end

function R.wrap_names(f)
  local names = f.names
  if not names then return end
  if f.replaces == "axis" then
    -- German soldiers are nameless in vanilla; only the enemy commander has a name.
    if names.soldiers then
      local pool = names.soldiers
      R.wrap("germNamer", function() return function() return R.make_name(pool) end end)
    end
    return
  end
  if names.soldiers then
    local pool = names.soldiers
    for _, fn_name in ipairs({ "firstNamer", "batiseur", "batiseurAirborne" }) do
      R.wrap(fn_name, function() return function() return R.make_name(pool) end end)
    end
  end
  if names.french then
    local pool = names.french
    R.wrap("frenchNamer", function() return function() return R.make_name(pool) end end)
  end
  if names.vehicles and #names.vehicles > 0 then
    local list = names.vehicles
    R.wrap("vehicleNamer", function() return function() return R.pick(list) end end)
  end
end

function R.variant_of(side, dude)
  return type(dude) == "table" and R.VARIANTS[side][dude.tag] or nil
end

function R.wrap_soldiers()
  local wants = false
  for _, h in ipairs(R.hooks) do
    if type(h.on_soldier_created) == "function" then wants = true end
  end
  if not wants then return end
  if R.factions.allied then
    if type(rawget(_G, "createAlliedSoldier")) == "function" then
      R.wrap("createAlliedSoldier", function(orig) return function(dude, ...)
        local result = orig(dude, ...)
        R.call_hooks("on_soldier_created", dude, R.variant_of("allied", dude) or "base", "allied")
        return result
      end end)
    end
  end
  if R.factions.axis then
    if type(rawget(_G, "spawnAxisTroops")) == "function" then
      R.wrap("spawnAxisTroops", function(orig) return function(...)
        local result = orig(...)
        local troops = type(player) == "table" and player.deployedTroops
        local dude = troops and troops[#troops]
        local variant = R.variant_of("axis", dude)
        if variant then R.call_hooks("on_soldier_created", dude, variant, "axis") end
        return result
      end end)
    end
  end
end

---------------------------------------------------------------------------------------------
-- The save: class unlocks and what is needed to undo the faction later
---------------------------------------------------------------------------------------------

function R.apply_profile(p)
  if type(p) ~= "table" then return end
  p.qmFaction = type(p.qmFaction) == "table" and p.qmFaction or {}
  p.qmFactionVanilla = type(p.qmFactionVanilla) == "table" and p.qmFactionVanilla or {}
  p.qmFactionWeaponBases = type(p.qmFactionWeaponBases) == "table" and p.qmFactionWeaponBases or {}
  for tag, def in pairs(R.weapons) do
    if def.mode == "new" then p.qmFactionWeaponBases[tag] = def.based_on end
  end
  for side, f in pairs(R.factions) do
    if p.qmFaction[side] ~= f.id or type(p.qmFactionVanilla[side]) ~= "table" then
      p.qmFactionVanilla[side] = { unlock = R.copy(p.unlock) }
      p.qmFaction[side] = f.id
    end
  end
  local allied = R.factions.allied
  if allied and type(p.unlock) == "table" then
    local backup = p.qmFactionVanilla.allied and p.qmFactionVanilla.allied.unlock
    if type(backup) == "table" then
      for spec, list in pairs(backup) do p.unlock[spec] = R.copy(list) end
    end
    for spec, cls in pairs(R.classes) do
      if cls.unlocks then p.unlock[spec] = R.copy(cls.unlocks) end
    end
  end
end

function R.wrap_new_profile()
  if type(rawget(_G, "newProfile")) ~= "function" then return end
  R.wrap("newProfile", function(orig) return function(...)
    local result = orig(...)
    R.try("profile reset", R.apply_profile, player)
    return result
  end end)
end

---------------------------------------------------------------------------------------------
-- Entry point
---------------------------------------------------------------------------------------------

function R.summary(f)
  local parts = {}
  local frames = 0
  for _, list in pairs(f.frames or {}) do for _ in pairs(list) do frames = frames + 1 end end
  local classes = 0
  for _ in pairs(f.classes or {}) do classes = classes + 1 end
  if frames > 0 then parts[#parts + 1] = frames .. " frames" end
  if f.weapons and #f.weapons > 0 then parts[#parts + 1] = #f.weapons .. " weapons" end
  if classes > 0 then parts[#parts + 1] = classes .. " classes" end
  if f.vehicles and #f.vehicles > 0 then parts[#parts + 1] = #f.vehicles .. " vehicles" end
  if #parts == 0 then parts[1] = "text and sounds only" end
  return table.concat(parts, ", ")
end

function R.apply(data)
  if type(data) ~= "table" or type(data.factions) ~= "table" then return end
  -- Wrapping twice would chain the wrappers and run every hook twice.
  if R.applied then
    R.log("factions already applied; ignoring a second apply")
    return
  end
  R.applied = true
  for _, f in ipairs(data.factions) do
    R.factions[f.replaces] = f
  end
  for side, f in pairs(R.factions) do
    local label = f.name .. " (" .. f.id .. ")"
    R.try(label .. " unit art", R.load_frames, f)
    R.try(label .. " sprites and sounds", R.load_sprites_and_sounds, f)
    R.try(label .. " text", R.apply_text, f)
    R.try(label .. " weapons", R.load_weapons, f)
    R.try(label .. " classes", R.load_classes, f)
    R.try(label .. " voices", R.load_voices, f)
    R.try(label .. " vehicles", R.load_vehicles, f)
    R.try(label .. " names", R.wrap_names, f)
    R.try(label .. " hooks", R.load_hooks, f)
    R.log(f.name .. " replaces the " .. (side == "allied" and "Allies" or "Germans") .. ": " .. R.summary(f))
  end
  R.try("weapons", R.wrap_weapon_creator)
  R.try("weapon pools", R.wrap_pools)
  if R.factions.allied then R.try("classes", R.wrap_spec_click) end
  R.try("soldier hooks", R.wrap_soldiers)
  R.try("profile", R.apply_profile, player)
  R.try("new profile", R.wrap_new_profile)
  if R.counts.voices then R.log(R.counts.voices .. " voice lines replaced") end

  -- Public, read-only view for other mods.
  local api = { active = {}, version = 1 }
  for side, f in pairs(R.factions) do
    api.active[side] = { id = f.id, name = f.name, side_name = f.side_name, replaces = f.replaces, mod_id = f.mod_id, root = f.root }
  end
  function api.get(side) return api.active[side] end
  function api.weapon(tag)
    local d = R.weapons[tag]
    return d and { tag = tag, mode = d.mode, based_on = d.based_on, faction = d.faction } or nil
  end
  function api.vehicle_name(tag) return R.vehicle_names[tag] end
  _G.QMFactions = api
  _G.__qm_vehicle_label = function(v)
    if type(v) ~= "table" then return "" end
    return R.vehicle_names[v.tag] or v.tag
  end

  for _, h in ipairs(R.hooks) do
    if type(h.on_enable) == "function" then
      R.try(h.__faction.id .. " hooks.on_enable", h.on_enable, api.active[h.__faction.replaces])
    end
  end
end

_G.__qm_factions_apply = R.apply
_G.__qm_factions_runtime = R
