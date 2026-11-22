-- Example Mod: class skills. Loaded by qm.toml.
-- In a Campaign battle (classic mode has no class skills), click a rifleman's portrait: the
-- first skill is tungsten rounds, the second a steadying buff, the third the light infantry's
-- charge. Gunners lose their third skill, snipers have head shots from level 1, and medics get
-- two new skills at levels 4 and 5.
-- Wiki: Part 5, Class skills.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

-- Change a game skill: the riflemen's armour-piercing rounds, with a new name, icon and cooldown.
QM.Skills.set({ class = "rifle", slot = 1, name = "Tungsten rounds",
  description = "armour-piercing rounds, ready sooner", icon = "example_mod_tungsten", cooldown = 120 })

-- Give a game skill your own code: the riflemen's rifle grenade becomes a steadying buff.
QM.Skills.set({ class = "rifle", slot = 2, name = "Hold fast", description = "steady under fire",
  cooldown = 90, use = function(soldier)
    buffer(soldier, "hold fast")          -- the game's own buff
    buffSound:clone():play()
  end })

-- Another class's game skill: the riflemen's third is the light infantry's charge.
QM.Skills.set({ class = "rifle", slot = 3, game = "light:3" })

-- Remove a skill, and give another an earlier level.
QM.Skills.remove("gunner", 3)                               -- no incendiary rounds
QM.Skills.set({ class = "sniper", slot = 3, unlock = 1 })   -- head shots from the start

-- New skills, as many as you like, at skill levels past 3 too.
QM.Skills.add({ class = "medic", game = "officer:1", unlock = 4 })   -- the officer's rally
QM.Skills.add({ class = "medic", name = "Second wind", description = "on their feet again",
  unlock = 5, cooldown = 60, use = function(soldier)
    buffer(soldier, "charge")
  end })

return ExampleMod
