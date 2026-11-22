-- Example Mod: added to the end of the game's includes/urb_engine.lua by qm.toml.
-- Define functions here; avoid code that runs on its own, since it runs as the file loads.

function exampleExtraFunction(dude)
  -- a helper the game's code or your patches can call
  return dude ~= nil and dude.health ~= nil and dude.health > 0
end
