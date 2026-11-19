# QuarterMaster changelog

Versions are `Major.Minor.Patch`.

## 2.0.2

- a mod's options can be grouped in a submenu: an option with `type = "submenu"` is one row that opens a page of the options whose `submenu` key names it. In the launcher it is a group that opens in place. Submenus can sit inside submenus
- added feature name `config_submenus`; `qm lint` warns when a submenu is used without it
- launcher: the Options tab hides an option while its `show_if` does not match, as the game does
- in the game and the launcher, `show_if` with `value = true` also matches a checkbox saved as `1` (and `false` matches `0`)
- Safe reset drops a value saved under a submenu's id, since a submenu holds none
- fixed: the game's Mod Settings screen drew a mod's options past the 14th off the screen; they are now on pages of 14, turned with page buttons or the mouse wheel

## 2.0.1

- launcher: updating QuarterMaster no longer leaves a console window open with an error in it; the update itself was already fine

## 2.0.0

- a patch can be written in a short form: `[[pattern]]`, `[[regex]]`, `[[copy]]` or `[[module]]` in place of `[[patches]]` and `[patches.<type>]`. Both forms can be mixed in one `qm.toml`, and patches keep the order they are written in
- added feature name `short_patches`
- a patch can have a `group`; the launcher lists a mod's patches under their groups on its Patches tab
- launcher: the first-run setup, the import and move offers and Report a problem no longer close on a click outside them or on Escape
- a QuarterMaster update also replaces `qm.exe` where it is installed
- `qm new` has six templates that all pass `qm lint`: `empty`, `patch` (the default), `module`, `settings`, `assets` and `faction`. Old template names give the nearest one; seven gameplay templates that no longer matched the game were removed
- a new mod from `qm new` requires no other mod and points its editor schemas at the site
- `qm --help` describes every command in plain words and lists the templates; `qm --version` prints the version
- `qm new` and `qm pack` refuse an option they do not know instead of ignoring it; `qm schema` writes into `schemas` by default
- `qm lint` and `qm test-patches` print clearer results
- launcher: the window's place and the launcher's browser data are kept in `qm-data` for an installed or portable copy, not in AppData
- launcher: a mod with no update source is offered an update found by its id in the mod list, and never installs it without asking unless you chose that for the mod
- `qm lint` warns when `qm.toml` uses the short patch form and `mod.json` does not list `short_patches`
- launcher: a patch's group shows on its chip in the Patches view
- security scan: a file a mod's Lua names (`dofile("extra.dat")`) is scanned whatever its extension, and so are the sources of short-form `[[module]]` and `[[copy]]` patches
- fixed: a downloaded mod with two dots inside a file name (`v1..v2.txt`) was refused as unsafe
- fixed: a `.zip` mod took its options from any `config.toml` or `qm.toml` inside it, not only the one at the mod's root
- fixed: a sound or image the game reloads through a faction's replacement lost what the loader knew about it, so a later replacement or size rule could miss it
- there is an installer, `QuarterMaster_<version>_x64-setup.exe`: it installs to Program Files, can install the `qm` command-line tool and add it to PATH, and offers a Start menu entry and a desktop shortcut
- the uninstaller takes QuarterMaster's files out of the game folder and, when asked, deletes mods, profiles, save slots and settings; a mod workspace folder is never deleted
- launcher: Settings is in three groups (Game, Mods and saves, QuarterMaster) on one grid, so cards in a row line up; copies of old Mods folders are a row of the Backups card, shown while there are some
- an installed or unzipped launcher keeps its data in `qm-data` beside itself instead of `%APPDATA%\QuarterMaster`; at its first start it offers to move earlier data over
- launcher: a manual install in the game folder (an old `winmm.dll` and a real `Mods` folder) is found and can be imported: the mods go into the library, the folder moves to the backups and the old `winmm.dll` is deleted
- a mod that states no version is installed as 0.0.0 instead of 1.0.0
- launcher: the first start has Auto-detect as the main button for the game and the save folder, and a step that explains the workspace folder and suggests one beside the launcher
- launcher: new themes Graphite, Black, Midnight, Paper and Desert; Dark and Light are now named Field dark and Field light
- launcher: a switch that is on follows the colour vision mode instead of staying green
- launcher: Report a problem (Diagnostics, Settings > About) opens a filled-in issue and makes a report file to attach
- launcher: a choice from the Assets view can be saved as a compatibility patch
- added `[[asset_choices]]` in `qm.toml`: a compatibility patch names whose sound, image or font is used where two mods it requires replace the same one
- a player's own choice for the file or its group wins over a compatibility patch's; the Assets view shows which mod chose a file
- added feature name `asset_choices`
- launcher: Settings fills the window: its cards flow into one to three columns by the window's width and the text size, and each setting keeps its control beside its text; colour vision is one row with a list instead of a tall block, and text size is a row like the others
- launcher: on the Patches tab the view switcher (Changes, Order, Fixes, Assets) leads the tab, and Fixes and Assets use the whole width
- launcher: the Saves tabs (Dossier, Ribbons, Soldiers, Backups) use the panel's whole width
- launcher: the Dossier shows its figures in three columns in a very wide window
- launcher: Manage profiles shows the profiles beside the picked profile's settings, so the dialog fits the window
- launcher: the "mod update is ready" note closes by itself after 15 seconds; the update stays on the Mods screen
- launcher: a mod's Patches tab explains each conflict in plain words, with the mods' and patches' names
- launcher: Compare profiles shows each profile's own mods side by side, and a kind of difference with nothing in it takes one line
- launcher: Diagnostics lists health problems first, then things to check, with the overwrite files inside their own row; the card is taller in a tall window
- launcher: a mod's Load order tab names other mods by name and writes a version range in words
- launcher: in a narrow mod list the category tag gives way so the mod's name stays readable
- launcher: the patch details explain each conflict in plain words, with the mods' and patches' names
- launcher: a file's changes start in the unified layout when there is no room for two columns of code, as at a large text size
- launcher: an order preview names adjusted copies "Your adjustments" instead of an internal name
- launcher: every mod row has an uninstall button; Select all (or Ctrl and Shift click) shows a bar that enables, disables or uninstalls the picked mods together, after one confirmation that lists them
- launcher: fixed uninstalled mods coming back: with a workspace mods folder set, an emptied library was filled from it again at the next start. The folder now fills an empty library once; Sync now in Settings copies it again
- launcher: fixed a message with a long address pushing its Dismiss button out of the window, so the message could not be closed
- launcher: a mod's details use the panel's width: patches, options and sections sit in columns, and the readme sits beside the mod's facts
- launcher: on the Patches tab, notes and conflicts that belong to no file are one line until opened; opened, each conflict has Resolve when a fix exists and a button to open the mod it names
- launcher: the patch details column shows only once a patch is picked, and has a close button; the file uses the room until then
- launcher: the patches above a file are larger, show name, mod and state on two lines, and picking one scrolls the file to its change
- launcher: a change longer than 60 lines shows its first 24 and folds the rest behind "Show N more lines of this change"
- launcher: Diagnostics shows Health and Crashes at the same height
- launcher: fixed a mod's Patches tab never loading when one of its patches targets several files
- launcher: checking a profile is faster with many patches on one long file: a mod is run alone once per file instead of once per patch, and an order swap is tested once per file instead of once per broken patch
- launcher: the launcher log no longer records every patch of every check, which wrote megabytes per session; warnings and errors stay
- launcher: on the Patches tab, "Show files changed by" also dims the other mods' changes in the file; the separate mod and patch lists above the file are gone, and so is the caption line above the patches
- launcher: long lines in a file's changes scroll sideways instead of being cut off, and the mod's whole name stays at the right edge of each change
- launcher: the row of patches above a file scrolls sideways with the mouse wheel, and a picked patch scrolls fully into view
- launcher: a mod with more than one installed version has a version picker in the mod list; each profile chooses the version it uses
- launcher: with a workspace mods folder set, a Library and Workspace switch beside the profile picker swaps between your installed mods and the workspace folder's mods. Workspace runs the folders as they are, with its own profiles; switching restarts the launcher
- launcher: a workspace mods folder that does not exist yet is made; Settings can open it and write every file of the game into `_game_dump` inside it
- a workspace subfolder whose name starts with `_` is never read as a mod
- launcher: Settings lists the copies the launcher made of real `Mods` folders it found in the game folder, with their size and contents, and deletes them. Before, they were kept without any notice
- launcher: a conflict that belongs to no line can be dismissed per profile; it stays listed under "Dismissed", stops counting, and can be shown again. Its game file can be viewed
- launcher: the Assets view shows a replaced file from the game and from each mod: images are shown, sounds play, text is read
- launcher: a file in a mod's file list opens in the same viewer
- launcher: Settings > Game window sets window size, fullscreen and its type, borderless, resizable, VSync and monitor. The values go into a mod the launcher writes, Game window settings, which patches `conf.lua`
- launcher: a file's changes can wrap long lines (Wrap), as well as scroll sideways; a file whose lines are too long for two columns side by side starts in the unified layout
- launcher: a mod has the same colour in every file on the Patches tab, and its Overview tab can pick the colour
- a `qm.toml` with a key written twice in one patch now says so in plain words, and that the lines starting the next patch are probably missing; before, the error read like two patches sharing an id
- fixed: when a damaged save had no older copy to fall back on, the fresh profile the game got was empty, and the profile in play was reset in memory
- fixed: a save whose last write never reached the disk is loaded from its `.prev` copy instead of starting over
- the game no longer does extra work each time it reads an undefined global while no mod adds numbered sounds
- a module or copy `source` must be inside its mod's folder; a path with `..` or an absolute path fails the mod
- security scan: a file a patch names as a `source` is read whatever its extension
- each `qm.toml` is read once per launch instead of three times, so a warning about it appears once
- `blacklist.txt`: spaces around a name are ignored
- a `.zip` mod's assets are unpacked again only when they changed, not on every launch
- fixed: the size limit for a mod archive counts what is really unpacked, not what the archive claims
- fixed: the mod list could mark a mod as installed because another mod's update key contained its repository name
- fixed: a crafted download address could place the temporary mod archive outside the launcher's download folder
- fixed: restoring a save backup reports a file it could not put back instead of saying it worked
- restoring a save backup makes the slot's saves exactly the backup's: a save file made after the backup is removed (it is kept in the backup made before the restore, when automatic backups are on)
- fixed: a shared profile code or `.qmpack` opened with `qm` could create its profile outside the profiles folder, or replace a profile with the same name; the new profile now gets a plain, unused id
- a `qm://import-profile` link shows what it would add and asks before it makes the profile
- `qm://install` links no longer install from a file or network path; use `qm import <path>` for a file on this computer
- fixed: a mod setting whose id is a Lua reserved word (such as `end`) no longer breaks the saved settings file
- fixed: a settings file that cannot be read is kept as `settings.json.unreadable` instead of being replaced by the defaults without a trace
- fixed: a copy of the game's Lua files that was cut short while being made is made again, instead of being used with files missing
- fixed: a launcher update that fails half way puts the old launcher back, so one is always there to start
- fixed: `qm verify-library` can repair an installed mod file that was edited in place; the library keeps its own copy of each file, which takes more disk space
- launcher: a mod's Overview lists the files it comes with, by folder and with sizes; a file opens in the viewer, and a long list can be filtered
- launcher: installing a newer version of a mod removes the version it takes the place of; before, every old version stayed in the library for good. This covers an archive or folder, the Browse tab, a mod update and the workspace mods folder. A version another profile is set to use stays
- launcher: Add mod offers two choices for a mod that is already installed, Upgrade and Install alongside; Replace is gone, since Upgrade now does it
- launcher: with several versions of a mod installed, its Overview lists them and each can be removed
- launcher: fixed file copies of removed mods and versions staying in the library's file store; they are deleted with the mod
- launcher: fixed a profile staying set to a version that a replace had removed
- launcher: the file viewer wraps the lines of `.md` and `.txt` files
- launcher: fixed dropping several archives or folders on the Mods screen adding only the first; Add mod lists the mods of each, and Browse zip and Browse folder can pick several
- launcher: QuarterMaster updates itself: Settings > Updates shows the installed version, offers a newer release with Update and restart, and replaces the launcher and the loader together; Go back to the previous version puts the replaced pair back
- launcher: the check for updates covers QuarterMaster as well as the mods
- launcher: added the setting Keep the old version when a mod updates. Off, an update takes the old version's place in every profile; on, both stay and other profiles stay on the old one. Installing an installed mod again from the Browse tab follows it too

## 1.8.1

- security scan: `os.rename` is found
- security scan: a module asked for with `pcall(require, "ffi")` is found
- launcher: a mod index entry from another repository than the installed mod's own is not offered as its update
- `qm pack`: the zip holds one folder named after the mod (`ExampleMod.zip` has `ExampleMod/mod.json`), so unpacking it into the Mods folder gives `Mods/ExampleMod/`
- `qm validate` lists the warnings from reading the mods
- factions: a weapon's `mag_sprite` can be any image, or a name from `[sprites]` shared by several weapons
- `qm faction check` warns about a `mag_sprite` name that is neither one of the game's magazines nor in `[sprites]`
- fixed: a faction weapon whose `mag_sprite` is not one of the game's own magazines no longer crashes the game on its first reload
- fixed: a mod folder and a `.zip` (or a second folder) with the same mod id no longer both load and apply every patch twice; the folder loads, and the other copy is skipped with a warning
- fixed: a save that used a faction no longer has the weapons of a loaded weapon mod turned into K98s when it loads
- fixed: a mod with only a `faction.toml` is found in a zip or folder; it needed "Import as unmanaged" before
- fixed: QuarterMaster's own new-profile settings patch no longer reports a count warning on every launch
- fixed: the game's log no longer warns about the `[publish]` table of a `qm.toml`
- fixed: the launcher builds from a fresh clone; its build script needed the local test folder
- fixed: a Windows checkout keeps Lua and Rust sources LF (`.gitattributes`), so the start-up Lua QuarterMaster embeds no longer changes with `core.autocrlf`

## 1.8.0

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

## 1.7.0

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

## 1.6.0

- baseline: patch engine, launcher, asset autoloader, menus, keybinds, faction mods, anchors, compatibility patches
