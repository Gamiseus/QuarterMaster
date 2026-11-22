-- Example Mod: a fourth class skill. Loaded by qm.toml.
-- In a Campaign battle (classic mode has no class skills), click the portrait of a rifleman
-- with all three skills: a fourth button sits above the third.
-- A click gives the soldier the light infantry's charge buff; the button then shows its cooldown.
-- Wiki: Part 5, Class skills.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

QM.Skills.add({
  class = "rifle",                    -- the class that gets the skill
  name = "Rally",
  description = "charge forward, as the light infantry do",
  icon = "example_mod_rally",         -- assets/sprites/example_mod_rally.png
  cooldown = 90,                      -- seconds before it is ready again
  use = function(soldier)
    buffer(soldier, "charge")         -- the game's own charge buff
    buffSound:clone():play()
  end,
})

return ExampleMod
