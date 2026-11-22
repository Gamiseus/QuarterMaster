-- Example Mod: called after the game makes a new profile.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

function ExampleMod.onNewProfile(player)
  print("[Example Mod] a new profile was made")
end

return ExampleMod
