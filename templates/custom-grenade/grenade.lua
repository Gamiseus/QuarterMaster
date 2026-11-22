-- Example Mod: a custom grenade. Loaded by qm.toml.
-- In a Campaign battle (classic mode has no class skills), click a light infantry soldier's
-- portrait, then the second skill, then a spot on the map.
-- The grenade flies as a frag with its own sprite and goes off with its own explosion.
-- Wiki: Part 5, Ordnance.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

QM.Explosions.register({
  id = "example_mod:blast",
  based_on = "frag",                 -- the frag grenade's numbers, then these
  fragRadius = 150,
  fragDamage = 75,
})

ExampleMod.grenade = QM.Ordnance.register({
  id = "example_mod:grenade",
  sprite = "example_mod_grenade",    -- assets/sprites/example_mod_grenade.png
  explosion = "example_mod:blast",
  range = 1000,                      -- the farthest a soldier throws it
  cooldown = 120,                    -- seconds before the skill is ready again
  -- spin = 0,                       -- no turning: it flies pointing the way it was thrown
  skill = { class = "light", slot = 2 },   -- the light infantry's second skill throws it
})

return ExampleMod
