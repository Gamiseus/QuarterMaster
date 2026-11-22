-- Example Mod: reads the options in config.toml.
-- Wiki: Part 6, Reading an option in Lua.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

-- Give this the experience a soldier would earn; it doubles it when Double XP is on.
function ExampleMod.xpFor(xpGain)
  if QM and QM.settings.example_mod and QM.settings.example_mod.double_xp then
    xpGain = xpGain * 2
  end
  return xpGain
end

-- Called by the "Reset Statistics" button.
function ExampleMod.resetStats()
  print("[Example Mod] statistics reset")
end

return ExampleMod
