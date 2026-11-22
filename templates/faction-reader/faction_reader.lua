-- Example Mod: reads the active faction. See qm.toml.
-- Wiki: Part 3, Reading the active faction from another mod.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

local army = QMFactions and QMFactions.get("allied")
if army then
  print(army.name)           -- "Example Army"; also army.id, army.side_name, army.replaces, army.mod_id
end

local w = QMFactions and QMFactions.weapon("example_smg")   -- { tag, mode, based_on, faction } or nil
local n = QMFactions and QMFactions.vehicle_name("sherman") -- "Example Tank" or nil
print("[Example Mod] example_smg:", w and w.faction or "not in play", "sherman:", n or "the game's own")

return ExampleMod
