-- Example Mod: a rifle grenade. Loaded by qm.toml.
-- In a Campaign battle (classic mode has no class skills), click a rifleman's portrait, then
-- the second skill, then a spot on the map: the soldier fires it from a rifle that takes rifle
-- grenades. Axis soldiers fire it too, in place of their own.
-- Wiki: Part 5, Rifle grenades.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

QM.Explosions.register({
  id = "example_mod:rifle_blast",
  based_on = "rifle_grenade",             -- the game's rifle grenade numbers, then these
  fragRadius = 140,
})

ExampleMod.rifleGrenade = QM.Ordnance.register({
  id = "example_mod:rifle_grenade",
  kind = "rifle",                         -- fired from a rifle, as the game's rifle grenade
  sprite = "example_mod_rifle_grenade",   -- assets/sprites/example_mod_rifle_grenade.png
  explosion = "example_mod:rifle_blast",
  skill = { class = "rifle", slot = 2 },  -- the riflemen's rifle grenade skill fires it
  replaces_rifle_grenade = "axis",        -- and Axis soldiers fire it instead of theirs
})

return ExampleMod
