-- Example Mod: your own code. qm.toml loads this file with the game and calls update every frame.
-- Wiki: Part 5, Module patches: https://gamiseus.github.io/wiki/adding-your-own-lua-files/#module-patches
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod      -- make it reachable from patches, keybinds and other mods

function ExampleMod.update(dt)
  -- runs every frame once hooked up below
end

return ExampleMod
