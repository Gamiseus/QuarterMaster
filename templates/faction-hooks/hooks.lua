-- Example Mod: faction code hooks. See faction.toml.
-- Wiki: Part 3, Code: https://gamiseus.github.io/wiki/faction-mods/#code-hooks

-- hooks.lua returns a table with any of these three functions.
local H = {}

function H.on_enable(faction)                    -- once, after the faction is applied
  print(faction.name .. " is active")
end

function H.on_soldier_created(dude, unit, side)  -- unit is "base", "sarge", ...
  if unit == "base" then dude.speed = dude.speed + 5 end
end

function H.on_weapon_created(weapon, tag)        -- after every weapon is built
end

return H
