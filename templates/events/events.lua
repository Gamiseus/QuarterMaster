-- Example Mod: answering the game's events. Loaded by qm.toml.
-- Each handler prints to the log (the Mod Engine's Test run panel shows it). The third value
-- of QM.Events.on names the handler in the log if it ever fails.
-- Wiki: Part 5, Game events.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

QM.Events.on("battle_start", function()
  print("[Example Mod] a battle started")
end, "example_mod")

QM.Events.on("wave_start", function(wave)
  print("[Example Mod] wave " .. wave)
end, "example_mod")

QM.Events.on("game_saved", function(file)
  print("[Example Mod] the game saved " .. file)
end, "example_mod")

QM.Events.on("game_loaded", function(file)
  print("[Example Mod] the game loaded " .. file)
end, "example_mod")

return ExampleMod
