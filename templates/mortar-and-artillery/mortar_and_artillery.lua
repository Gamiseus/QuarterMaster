-- Example Mod: a mortar and an artillery barrage. Loaded by qm.toml.
-- In a Campaign battle (classic mode has no class skills), click a rifleman's portrait, then
-- the third skill, then a spot: four heavy mortar rounds land there. An officer with all
-- three skills has a fourth: a barrage of eight heavy shells where you click.
-- Wiki: Part 5, Mortars and artillery.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

QM.Explosions.register({
  id = "example_mod:heavy_shell",
  based_on = "artillery",                 -- the game's artillery shell, then these
  fragRadius = 150,
})

-- A mortar: the riflemen fire it with their third skill (the game's mortar), and the Axis
-- mortar men fire it too.
ExampleMod.mortar = QM.Ordnance.register({
  id = "example_mod:heavy_mortar",
  kind = "mortar",
  explosion = "hvyExplosion",             -- the game's heavy mortar
  shots = 4,                              -- rounds each time the skill fires it
  skill = { class = "rifle", slot = 3 },
  replaces_mortar = "axis",
})

-- A barrage the officers call in where you click, as their fourth skill.
ExampleMod.barrage = QM.Ordnance.register({
  id = "example_mod:barrage",
  kind = "artillery",
  name = "Heavy barrage",
  description = "eight heavy shells where you click",
  icon = "example_mod_barrage",           -- assets/sprites/example_mod_barrage.png
  explosion = "example_mod:heavy_shell",
  shots = 8,
  skill = { class = "officer", slot = 4 },
})

return ExampleMod
