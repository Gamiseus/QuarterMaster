-- Example Mod: answers the keybind in config.toml.
-- Wiki: Part 6, Keybinds.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

function ExampleMod.onPing()
  local x = love.mouse.getX() + player.screenx    -- screen position to map position
  local y = love.mouse.getY() + player.screeny
  print("[Example Mod] ping at", x, y)
end

return ExampleMod
