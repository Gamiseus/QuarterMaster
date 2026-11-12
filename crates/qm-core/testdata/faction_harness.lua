-- Runs QuarterMaster's faction runtime outside the game, against vanilla functions copied
-- from the decompiled sources. Called by qm-core's faction tests:
--   lua faction_harness.lua <apply|cleanup|excluded|pools> <work dir> <path to urb_engine.lua>
-- The work dir holds payload.lua (loader runtime), expected.lua and factions.lua (the two
-- faction patches). Mod files contain their own relative path, so images and sounds can be
-- told apart by their bytes.

local scenario, work, engine_path = arg[1], arg[2], arg[3]
loadstring = loadstring or load
unpack = unpack or table.unpack

local failures = 0
local function check(cond, msg)
  if not cond then
    failures = failures + 1
    print("FAIL: " .. msg)
  end
end
local function eq(got, want, msg)
  check(got == want, msg .. " (got " .. tostring(got) .. ", want " .. tostring(want) .. ")")
end

---------------------------------------------------------------------------------------------
-- A tiny LÖVE
---------------------------------------------------------------------------------------------

local function bytes_of(a)
  if type(a) == "table" then return a.bytes end
  return "vanilla:" .. tostring(a)
end

local function new_source(a, mode)
  local s = { type = "source", kind = "source", bytes = bytes_of(a), mode = mode or "static", looping = false, volume = 1 }
  function s:getType() return self.mode end
  function s:isLooping() return self.looping end
  function s:getVolume() return self.volume end
  function s:setLooping(v) self.looping = v end
  function s:setVolume(v) self.volume = v end
  function s:stop() end
  function s:play() end
  function s:clone() return self end
  return s
end

local function new_image(a)
  local img = { type = "image", kind = "image", bytes = bytes_of(a) }
  function img:getWidth() return 10 end
  function img:getHeight() return 10 end
  return img
end

local VO = {}

love = {
  filesystem = {
    newFileData = function(bytes, name) return { bytes = bytes, name = name } end,
    getDirectoryItems = function(dir) return VO[dir] or {} end,
  },
  graphics = { newImage = new_image },
  audio = { newSource = new_source },
  math = { random = math.random },
}

local function run(file)
  local chunk = assert(loadstring(assert(io.open(work .. "/" .. file, "rb")):read("*a"), "@" .. file))
  chunk()
end

local engine_src = assert(io.open(engine_path, "rb")):read("*a")
local function vanilla(name)
  local s = engine_src:find("\nfunction " .. name .. "%(")
  assert(s, "vanilla function " .. name .. " not found")
  local e = engine_src:find("\nend[ \t]*\r?\n", s + 1)
  local src = engine_src:sub(s + 1, e + 4)
  assert(loadstring(src, "@vanilla:" .. name))()
end

local function vanilla_unlock()
  return {
    rifle = {"matchGarand", "m1941", "t20"}, light = {"m1CarbinePara", "m2Carbine", "m2CarbineA1"},
    gunner = {"bren", "m1919", "vicker"}, medic = {"sten", "m3", "m1"}, comms = {"sten", "m3", "tommy"},
    officer = {"m1Carbine", "tommy", "m1"}, engy = {"m3", "m1Carbine", "m1"}, scout = {"sten", "m3", "tommy"},
    sniper = {"scopedGarand", "scopedSpringfield", "scopedEnfield"}, commando = {"m3", "tommy", "stenMKVI"},
    sapper = {"sten", "m3", "tommy"}, intel = {"m3", "mp40", "stg44"},
  }
end

-- The loader runtime goes first, exactly as at the top of main.lua.
run("payload.lua")

randNbr = function(n) return math.random(1, n) end
vanilla("weaponCreator")
updateProfile = function(p) end -- wrapped by the loader runtime as it is defined

---------------------------------------------------------------------------------------------
if scenario == "cleanup" then
  local function roster_player()
    return {
      unlock = (function() local u = vanilla_unlock(); u.rifle = {"svt40", "ppsh41", "pps43"}; return u end)(),
      qmFaction = { allied = "red_army" },
      qmFactionVanilla = { allied = { unlock = vanilla_unlock() } },
      qmFactionWeaponBases = { pps43 = "m3" },
      roster = { { name = "A. Ivanov", primarySwap = "pps43", weapon = { tag = "pps43", maxAmmo = 35, ammo = 35, magSprite = "genericMag" } } },
    }
  end

  -- Faction still enabled: nothing changes.
  __qm_factions_expected = { allied = "red_army" }
  __qm_factions_expected_weapons = { pps43 = true }
  local p = roster_player()
  updateProfile(p)
  eq(p.unlock.rifle[3], "pps43", "faction unlocks kept while enabled")
  eq(p.roster[1].weapon.tag, "pps43", "faction weapon kept while enabled")
  eq(p.qmFaction.allied, "red_army", "faction id kept while enabled")

  -- Faction removed: vanilla unlocks back, weapons turned into their base.
  __qm_factions_expected = nil
  __qm_factions_expected_weapons = nil
  p = roster_player()
  updateProfile(p)
  eq(p.unlock.rifle[1], "matchGarand", "vanilla unlocks restored")
  eq(p.roster[1].weapon.tag, "m3", "weapon rebuilt as its base")
  eq(p.roster[1].weapon.maxAmmo, 30, "base weapon stats")
  eq(p.roster[1].primarySwap, "m3", "swap slot repaired")
  eq(p.qmFaction, nil, "faction marker cleared")
  eq(p.qmFactionWeaponBases, nil, "weapon bases cleared")

  -- Another faction on the same side: the old one is undone too.
  __qm_factions_expected = { allied = "polish_army" }
  p = roster_player()
  updateProfile(p)
  eq(p.unlock.rifle[1], "matchGarand", "switching factions restores vanilla first")
  eq(p.roster[1].weapon.tag, "m3", "unknown weapon repaired when switching")

  -- An axis faction removed: only allied factions change class unlocks, so the player's
  -- unlocks (progress made since) stay as they are.
  __qm_factions_expected = nil
  p = {
    unlock = (function() local u = vanilla_unlock(); u.rifle = {"matchGarand", "m1941", "earned"}; return u end)(),
    qmFaction = { axis = "volkssturm" },
    qmFactionVanilla = { axis = { unlock = vanilla_unlock() } },
  }
  updateProfile(p)
  eq(p.unlock.rifle[3], "earned", "axis faction removal keeps class unlocks")
  eq(p.qmFaction, nil, "axis faction marker cleared")

  if failures == 0 then print("HARNESS OK") else os.exit(1) end
  return
end

---------------------------------------------------------------------------------------------
-- scenario "apply"
---------------------------------------------------------------------------------------------

run("expected.lua")
eq(__qm_factions_expected.allied, "red_army", "expected faction set")

-- Vanilla assets, loaded the way main.lua loads them.
allied_idle = love.graphics.newImage("sprites/allied_idle.png")
allied_aim = love.graphics.newImage("sprites/allied_aim.png")
allied_fire = love.graphics.newImage("sprites/allied_fire.png")
sarge_idle = love.graphics.newImage("sprites/sarge_idle.png")
top_m1 = love.graphics.newImage("sprites/top_m1.png")
side_m1 = love.graphics.newImage("sprites/side_m1.png")
top_m3 = love.graphics.newImage("sprites/top_m3.png")
side_m3 = love.graphics.newImage("sprites/side_m3.png")
shermanBody = love.graphics.newImage("sprites/shermanBody.png")
springfieldSound = love.audio.newSource("sounds/springfield.wav", "static")
for z = 1, 3 do _G["sherman_debris" .. z] = love.graphics.newImage("sprites/sherman_debris" .. z .. ".png") end
VO["sounds/vo/allied"] = {}
for z = 1, 5 do
  _G["contact" .. z] = love.audio.newSource("sounds/vo/allied/contact" .. z .. ".wav", "stream")
  table.insert(VO["sounds/vo/allied"], "contact" .. z .. ".wav")
end
for z = 1, 3 do
  _G["alliedHurt" .. z] = love.audio.newSource("sounds/vo/allied/alliedHurt" .. z .. ".wav", "stream")
  table.insert(VO["sounds/vo/allied"], "alliedHurt" .. z .. ".wav")
end
mouseOverRifleSpec = "Heavy Infantry, Medium to long range"
mouseOverGunnerSpec = "Gunner, Lays suppressive fire"
mouseOverLightSpec = "Light Infantry, fast"

for _, name in ipairs({ "specClick", "getRandomAlliedGun", "getRandomGruntGun", "getRandomSMG", "firstNamer", "batiseur", "batiseurAirborne", "vehicleNamer" }) do
  vanilla(name)
end
convertedId = function(id) return id end
specked = new_source("specked", "static")
specSoundSpam = 0
newProfile = function() player.unlock = vanilla_unlock() end
spawnSherman = function()
  player.deployedVehicles[#player.deployedVehicles + 1] = { tag = "sherman", callSign = vehicleNamer() }
end

-- scenario "pools": another mod changed two pool functions before the faction applies
-- (Phase E D83). One adds its own weapon; the other fails.
if scenario == "pools" then
  local game_grunt = getRandomGruntGun
  getRandomGruntGun = function()
    if math.random(1, 2) == 1 then return "modgun" end
    return game_grunt()
  end
  getRandomAlliedGun = function() error("the other mod's pool is broken") end
end

player = { unlock = vanilla_unlock(), deployedTroops = {}, deployedVehicles = {} }
updateProfile(player)
run("factions.lua")

if scenario == "pools" then
  local grunt, allied, smg = {}, {}, {}
  for _ = 1, 2000 do
    local g = getRandomGruntGun()
    grunt[g] = (grunt[g] or 0) + 1
    allied[getRandomAlliedGun()] = true
    smg[getRandomSMG()] = true
  end
  check(grunt.modgun, "the other mod's weapon stays in the pool")
  check(grunt.pps43, "the faction's added weapon is drawn")
  check(not grunt.springfield, "the faction's removed weapon is never drawn")
  check(grunt.m1 and grunt.tommy, "the game's other weapons are drawn")
  -- pps43 is 1 of the faction's 10 grunt weapons: about 200 of 2000 draws.
  check(grunt.pps43 > 120 and grunt.pps43 < 300, "the added weapon keeps its share (got " .. tostring(grunt.pps43) .. ")")
  -- A pool function that fails falls back to the faction's own list.
  check(allied.pps43 and allied.m1, "a failing pool function falls back to the faction's list")
  check(not allied.modgun, "nothing from a function that fails")
  -- A pool the faction does not edit is not wrapped.
  check(smg.tommy and not smg.pps43, "an unedited pool is the game's")
  if failures == 0 then print("HARNESS OK") else os.exit(1) end
  return
end

local function winner_of(slot)
  for line in (__qm_asset_winner_text() .. "\n"):gmatch("([^\n]*)\n") do
    local s, source = line:match("^(.-)\t(.*)$")
    if s == slot then return source end
  end
  return nil
end

---------------------------------------------------------------------------------------------
-- scenario "excluded": a choice gave some files to another source (Phase E D78). The
-- factions payload was compiled with those names in `excluded`.
---------------------------------------------------------------------------------------------
if scenario == "excluded" then
  eq(allied_idle.bytes, "vanilla:sprites/allied_idle.png", "an excluded frame stays as it was")
  eq(allied_aim.bytes, "units/base/aim.png", "the other frames are still replaced")
  eq(sarge_idle.bytes, "units/base/idle.png", "a copied unit is its own name")
  eq(top_m1.bytes, "vanilla:sprites/top_m1.png", "an excluded weapon sprite stays")
  eq(shermanBody.bytes, "vanilla:sprites/shermanBody.png", "an excluded vehicle part stays")
  eq(sherman_debris1.bytes, "vehicles/t34/debris1.png", "debris 1 is replaced")
  eq(sherman_debris2.bytes, "vanilla:sprites/sherman_debris2.png", "excluded debris stays")
  eq(sherman_debris3.bytes, "vehicles/t34/debris1.png", "debris 3 wraps around as before")
  eq(contact1.bytes, "vanilla:sounds/vo/allied/contact1.wav", "an excluded voice line stays")
  eq(contact2.bytes, "voices/contact2.wav", "the other voice lines are replaced")
  -- Nothing is registered for an excluded file: the game's path still gives the game's file.
  eq(love.graphics.newImage("sprites/allied_idle.png").bytes, "vanilla:sprites/allied_idle.png",
    "no override for an excluded file")
  eq(winner_of("sprites/allied_idle"), nil, "the faction did not win an excluded slot")
  eq(winner_of("sprites/allied_aim"), "faction:red_army", "it won the others")
  -- The rest of the faction is untouched by exclusions.
  local w = {}
  weaponCreator(w, "m1")
  eq(w.damage, 110, "weapon stats still apply")
  if failures == 0 then print("HARNESS OK") else os.exit(1) end
  return
end

-- One override channel (Phase E D81): the faction's files are registered with the asset
-- loader, so a later load of the game's own path (a map reload) gives the faction's file.
eq(love.graphics.newImage("sprites/allied_idle.png").bytes, "units/base/idle.png",
  "a later load of the game's path gives the faction's frame")
eq(love.graphics.newImage("sprites/top_m1.png").bytes, "weapons/mosin_top.png",
  "and the faction's weapon sprite")
eq(love.graphics.newImage("sprites/shermanBody.png").bytes, "vehicles/t34/body.png",
  "and the faction's vehicle part")
eq(love.graphics.newImage("sprites/sherman_debris2.png").bytes, "vehicles/t34/debris2.png",
  "and its debris")
eq(love.graphics.newImage("sprites/allied_fire.png").bytes, "vanilla:sprites/allied_fire.png",
  "a file the faction does not replace still loads from the game")
eq(winner_of("sprites/allied_idle"), "faction:red_army", "the faction is the winner of its frames")
eq(winner_of("sounds/vo/allied/contact1"), "faction:red_army", "and of its voice lines")
eq(winner_of("sprites/allied_fire"), nil, "nobody replaced this one")

-- Unit art, with copy_from filling the sergeant from the base unit.
eq(allied_idle.bytes, "units/base/idle.png", "base idle frame")
eq(allied_aim.bytes, "units/base/aim.png", "base aim frame")
eq(allied_fire.bytes, "vanilla:sprites/allied_fire.png", "missing frame stays vanilla")
eq(sarge_idle.bytes, "units/base/idle.png", "sarge copies base")

-- Weapons.
eq(weaponText_m1, 'Mosin-Nagant "M91/30".\nBolt action.', "weapon text with quotes and newline")
local w = {}
weaponCreator(w, "m1")
eq(w.tag, "m1", "reskin keeps its tag")
eq(w.action, "bolt", "reskin action")
eq(w.ammo, 5, "reskin ammo")
eq(w.damage, 110, "reskin damage")
eq(w.range, 350, "unchanged stat keeps the vanilla value")
eq(w.sound, "__qmf_red_army_m1_fire", "reskin sound pool name")
local seen = {}
for _ = 1, 60 do seen[_G[w.sound].bytes] = true end
check(seen["weapons/mosin1.wav"] and seen["weapons/mosin2.wav"], "sound pool plays both files")
eq(top_m1.bytes, "weapons/mosin_top.png", "reskin top sprite")
eq(side_m1.bytes, "vanilla:sprites/side_m1.png", "reskin without side sprite keeps vanilla")

local w2 = {}
weaponCreator(w2, "pps43")
eq(w2.tag, "pps43", "new weapon tag")
eq(w2.maxAmmo, 35, "new weapon maxAmmo")
eq(w2.ammo, 35, "new weapon ammo follows maxAmmo")
eq(w2.action, "auto", "new weapon copies base stats")
eq(_G[w2.sound].bytes, "weapons/pps43.wav", "single sound file")
eq(top_pps43, top_m3, "new weapon falls back to base sprite")
eq(weaponText_pps43, "PPS-43", "new weapon text from its name")

local w3 = {}
weaponCreator(w3, "svt40")
eq(w3.sound, "springfieldSound", "weapon sound by game name")
check(__qm_sound_by_tag and __qm_sound_by_tag.m1, "m1 reskin sound is used over the tag-picked Garand sound")
check(__qm_sound_by_tag and not __qm_sound_by_tag.pps43, "new weapons don't need the tag-sound check")

-- spawn_swap only for the replaced side.
player.deployedTroops[1] = { faction = "allied", weapon = {} }
player.deployedTroops[2] = { faction = "axis", weapon = {} }
weaponCreator(player.deployedTroops[1].weapon, "tommy")
weaponCreator(player.deployedTroops[2].weapon, "tommy")
local loose = {}
weaponCreator(loose, "tommy")
eq(player.deployedTroops[1].weapon.tag, "ppsh41", "allied soldier gets the swap")
eq(player.deployedTroops[2].weapon.tag, "tommy", "german soldier keeps vanilla")
eq(loose.tag, "ppsh41", "weapon without a soldier yet gets the swap")

-- Pools.
local grunt, allied = {}, {}
for _ = 1, 600 do
  grunt[getRandomGruntGun()] = true
  allied[getRandomAlliedGun()] = true
end
check(not grunt.springfield, "removed weapon never drawn")
check(grunt.pps43, "added weapon drawn from grunt pool")
check(allied.pps43, "loot_side adds to the allied pool")
check(allied.springfield, "allied pool otherwise vanilla")

-- Classes.
local function recruit()
  return { faction = "allied", health = 100, maxHealth = 100, spec = "", promo = 1, primarySwap = "", weapon = {},
           speed = 100, skill = 50, hthDamage = 100, detectRange = 1000, mineDetectRange = 200, selected = true }
end
player.deployedTroops[3] = recruit()
specID = 3
specClick("rifle")
local r = player.deployedTroops[3]
eq(r.spec, "rifle", "class assigned")
eq(r.weapon.tag, "m1", "class start weapon")
eq(r.weapon.action, "bolt", "start weapon is the reskin")
eq(r.promo, 0, "vanilla bookkeeping still runs")

player.deployedTroops[4] = recruit()
specID = 4
specClick("gunner")
local g = player.deployedTroops[4]
eq(g.speed, 95, "bonus replaces vanilla speed change")
eq(g.detectRange, 2000, "bonus detect multiplier")
eq(g.maxHealth, 110, "bonus max health")
eq(g.health, 110, "bonus heals by the same amount")
eq(g.weapon.tag, "pps43", "class start weapon can be a new weapon")

player.deployedTroops[5] = recruit()
specID = 5
specClick("light")
local l = player.deployedTroops[5]
eq(l.speed, 110, "class without faction data stays vanilla")
eq(l.weapon.tag, "m1Carbine", "vanilla class weapon")

-- The save.
eq(player.unlock.rifle[1], "svt40", "faction unlock tiers")
eq(player.unlock.rifle[3], "pps43", "unlocks can name new weapons")
eq(player.unlock.light[1], "m1CarbinePara", "other classes keep vanilla unlocks")
eq(player.qmFaction.allied, "red_army", "save remembers the faction")
eq(player.qmFactionVanilla.allied.unlock.rifle[1], "matchGarand", "save keeps vanilla unlocks")
eq(player.qmFactionWeaponBases.pps43, "m3", "save keeps weapon bases")
newProfile()
eq(player.unlock.rifle[1], "svt40", "profile reset re-applies the faction")

-- Names.
for _ = 1, 20 do
  local n = firstNamer()
  check(n:match("^%u%. Ivanov$") or n:match("^%u%. Petrov$"), "soldier names: " .. n)
  check(batiseur():match("^%u%. [IP]"), "recruit names")
  check(batiseurAirborne():match("^%u%. [IP]"), "airborne names")
end
eq(vehicleNamer(), "Za Rodinu", "vehicle names")
spawnSherman()
eq(player.deployedVehicles[#player.deployedVehicles].callSign, "Za Stalina", "per-vehicle call signs")

-- Voices: exact files, numbered files spread over vanilla's pool, renamed files.
eq(contact1.bytes, "voices/contact1.wav", "voice 1")
eq(contact2.bytes, "voices/contact2.wav", "voice 2")
eq(contact3.bytes, "voices/contact1.wav", "voice 3 reuses 1")
eq(contact4.bytes, "voices/contact2.wav", "voice 4 reuses 2")
eq(contact5.bytes, "voices/contact1.wav", "voice 5 reuses 1")
eq(contact1.mode, "stream", "voice keeps stream mode")
eq(alliedHurt1.bytes, "voices/hurt1.wav", "renamed voice")
eq(alliedHurt3.bytes, "voices/hurt1.wav", "renamed voice fills the pool")

-- Text.
eq(mouseOverRifleSpec, "Rifleman, Strelok.", "class text")
eq(mouseOverGunnerSpec, "Machine Gunner, Lays suppressive fire", "class name keeps vanilla description")
eq(mouseOverLightSpec, "Scout, fast.", "[text] override")

-- Vehicles.
eq(shermanBody.bytes, "vehicles/t34/body.png", "vehicle body")
eq(sherman_debris1.bytes, "vehicles/t34/debris1.png", "debris 1")
eq(sherman_debris2.bytes, "vehicles/t34/debris2.png", "debris 2")
eq(sherman_debris3.bytes, "vehicles/t34/debris1.png", "debris 3 reuses 1")
eq(__qm_vehicle_label({ tag = "sherman" }), "T-34/76", "vehicle tooltip name")
eq(__qm_vehicle_label({ tag = "jeep" }), "jeep", "other vehicles keep their tag")

-- Hooks and the public view.
eq(TEST_HOOK_ENABLED, "red_army", "hooks on_enable")
check((TEST_WEAPONS_CREATED or 0) > 0, "hooks on_weapon_created")
eq(QMFactions.active.allied.name, "Red Army", "QMFactions.active")
eq(QMFactions.weapon("pps43").based_on, "m3", "QMFactions.weapon")
eq(QMFactions.get("axis"), nil, "no axis faction")

-- Running the patch again (it is in love.load) must not wrap the wrappers or double the hooks.
local before = TEST_WEAPONS_CREATED or 0
weaponCreator({}, "m1")
local per_call = (TEST_WEAPONS_CREATED or 0) - before
run("factions.lua")
before = TEST_WEAPONS_CREATED or 0
weaponCreator({}, "m1")
eq((TEST_WEAPONS_CREATED or 0) - before, per_call, "a second apply does not double the hooks")

if failures == 0 then print("HARNESS OK") else os.exit(1) end
