-- Example Mod: air support. Loaded by qm.toml.
-- In a Campaign battle (classic mode has no class skills), a radio operator with all three
-- skills has a fourth: three heavy bombs where you click. A medic's fourth skill drops a
-- crate of med packs and rations; a soldier next to it opens it.
-- Wiki: Part 5, Air strikes and supply drops.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

QM.Explosions.register({
  id = "example_mod:big_bomb",
  based_on = "bomb",                      -- the game's bomber's bomb, then these
  blastRadius = 150,
})

-- A bomber run the radio operators call in where you click, as their fourth skill.
ExampleMod.bombers = QM.Ordnance.register({
  id = "example_mod:bombers",
  kind = "airstrike",
  name = "Bomber run",
  description = "three heavy bombs where you click",
  explosion = "example_mod:big_bomb",
  shots = 3,                              -- bombs
  spread = 150,                           -- how far each lands from the spot
  interval = 0.5,                         -- seconds between them
  skill = { class = "comms", slot = 4 },
})

-- A supply drop the medics call in, as their fourth skill.
ExampleMod.medicalDrop = QM.Ordnance.register({
  id = "example_mod:medical_drop",
  kind = "drop",
  name = "Medical drop",
  description = "a crate of med packs and rations where you click",
  contents = { "loot_medPack", "loot_ration" },
  shots = 4,                              -- items in the crate
  skill = { class = "medic", slot = 4 },
})

return ExampleMod
