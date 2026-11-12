# QuarterMaster changelog

Versions are `Major.Minor.Patch`.

## 1.2.1

- security scan: `os.rename` is found
- security scan: a module asked for with `pcall(require, "ffi")` is found
- launcher: a mod index entry from another repository than the installed mod's own is not offered as its update
- fixed: a save that used a faction no longer has the weapons of a loaded weapon mod turned into K98s when it loads
- fixed: a mod with only a `faction.toml` is found in a zip or folder; it needed "Import as unmanaged" before
- fixed: QuarterMaster's own new-profile settings patch no longer reports a count warning on every launch
- fixed: the game's log no longer warns about the `[publish]` table of a `qm.toml`

## 1.2.0

- launcher: Assets view on the Patches tab lists sounds, images and fonts that two sources replace
- launcher: pick the winner for a group of files or one file; stored as a fix, undo under Fixes
- launcher: pick the faction for a side when two factions replace it
- factions leave alone files a player gave to another mod or to the game
- faction images and sounds stay after the game reloads a file
- faction weapon pools keep weapons another mod added
- the game reports which mod's file each replaced asset ended up being
- mods page: "replaces a game file" marks use the game's full file list
- plans: `asset_choices`, `asset_exclusions`, `factions` (plan schema 4, written only when used)
- saves that load with mods are marked with the profile, the mods and the values each mod writes
- launcher: Saves screen shows values left by mods no profile on the slot loads
- launcher: Clean up removes such values on request, after a backup; values two mods wrote stay
- added `save_content` in `mod.json`: a mod's weapons become their base weapon while the mod is off
- a test launch without mods does not mark or change saves
- QuarterMaster's wrappers (save check, faction functions) are put back when a mod replaces the function during loading
- a wrapper that was put back passes straight through, so its part runs once per call
- launcher: Diagnostics has "Trace globals..." for a diagnostic test run
- a diagnostic test run notes which mod sets each watched global, and counts who calls each watched function
- the trace is kept with the run in the profile's run log; normal launches are not traced
- images are shown at a size separate from their resolution: a replacement takes the original's size
- added `[sprites]` in `qm.toml`: `scale`, `like`, `size` per image
- added `QM.Sprites` (`setSize`, `setScale`, `matchSize`, `reset`, `rawSize`, `scaleOf`)
- added feature name `sprite_sizes`
- removed the "dimension mismatch" warning; a replacement of another shape is reported instead
- Steam not running: message box, then the game closes, instead of a silent crash
- fixed crash on a save file that does not compile (zero bytes, cut off): the guard now catches LÖVE's error
- the save guard is in every modded game, not only when a mod has settings
- a damaged save is kept as `<name>.damaged`; the game gets `backUp.lua`, then `<name>.prev`, then a fresh profile
- each write of a save keeps the copy it replaces as `<name>.prev`; save slots carry the `.prev` files
- added `image` in `[publish]`: a picture for the mod's card in the mod list
- editor schema for `qm.toml` knows the `[publish]` table

## 1.1.0

- added debug mode (`-debug`, `QM_DEBUG=1`, launcher switch)
- console window only in debug mode
- dump commands (`qm.dump_game_files`) only in debug mode
- added `qm.dump_patched()`
- added dump keys: Ctrl+Shift+D, Ctrl+Shift+Alt+D, Ctrl+Shift+P
- debug mode dumps every loaded file; otherwise only patched files
- added developer commands (`-dev`, `QM_DEV=1`, launcher switch): AI overlay (F1), dev blast (F5)
- added `qm.debug`, `qm.dev`
- launcher: Debug mode and Developer commands switches replace the console switch
- fixed `on_click` ignored on `config.toml` button options
- removed `disable_console` launch setting (use `--disable-console`)

## 1.0.0

- baseline: patch engine, launcher, asset autoloader, menus, keybinds, faction mods, anchors, compatibility patches
