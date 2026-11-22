-- Example Mod: a page for the mod's author, only in debug mode.
-- Wiki: Part 10, Debug mode; Part 6, Mod Operations.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

function ExampleMod.report()
  local qm = require "qm"
  if qm.debug then
    print("[ExampleMod] squad size is " .. tostring(#player.deployedTroops))
  end
end

local qm = require "qm"
if qm.debug and QM and QM.Menu then
  QM.Menu.registerPage({
    id = "example_mod_debug",
    modId = "example_mod",
    title = "Example Mod (debug)",
    content = "Only shown in debug mode. The button prints the squad size to the log.",
    action_label = "Report",
    action_on_click = ExampleMod.report,
  })
  QM.Menu.registerMenuButton({
    id = "example_mod_debug",
    modId = "example_mod",               -- so "page:" finds the page above
    title = "Example Mod debug",
    subtitle = "Debug mode only",
    priority = 90,
    on_click = "page:example_mod_debug",
  })
end

return ExampleMod
