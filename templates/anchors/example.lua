-- Example Mod: loaded after other_mod's code when other_mod is on.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod
print("[Example Mod] loaded")
return ExampleMod
