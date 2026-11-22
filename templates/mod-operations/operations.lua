-- Example Mod: the function the briefing page's button calls.
-- Wiki: Part 6, Mod Operations.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

function exampleStartOperation()        -- a plain global function
  print("[Example Mod] operation started")
end

return ExampleMod
