-- Example Mod: wraps a game function. qm.toml loads this file and calls install once.
-- Wiki: Part 5, Wrapping a game function.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

function ExampleMod.install()
  local original = floatyText            -- a game function: floatyText(dude, text)
  function floatyText(dude, text)
    print("[Example Mod] " .. tostring(text))
    return original(dude, text)          -- always call the original
  end
end

return ExampleMod
