# QuarterMaster

A mod manager and mod loader for **Mud and Blood**.

The game packs its scripts and art inside `mnb.exe`, so changing them by hand means unpacking and repacking the game, and redoing it after every update. QuarterMaster applies mods in memory while the game starts instead. The game's files are never changed on disk: turn your mods off and the game is vanilla again.

- **[Download](https://github.com/Gamiseus/QuarterMaster/releases)**
- **[Mods](https://gamiseus.github.io/mods/)**
- **[Modding wiki](https://gamiseus.github.io/wiki/)**: how to make a mod, from replacing a sound to patching game code
- **[Changelog](CHANGELOG.md)**

## What it does

- **Installs and updates mods.** Browse the mod list in the launcher, or add a `.zip` or a folder. Every download is checked against the hash the site recorded.
- **Profiles.** Each profile has its own mods, load order and save slot. The built-in Vanilla profile runs the game without mods.
- **Shows what your mods change.** The Patches tab shows every changed game file line by line, which mod changed it, and where two mods collide, with fixes you can apply.
- **Looks after saves.** Save slots, automatic backups before a switch or a restore, and a dossier of each save.
- **Finds the cause of a crash.** Crash reports name the mod that was running, and a test launch can narrow a crash down to one mod.
- **Keeps modded runs off the leaderboards.** Steam leaderboard uploads, achievements and stats are blocked while mods are on.

## Installing

1. Download `QuarterMaster_<version>_x64-setup.exe` from the [latest release](https://github.com/Gamiseus/QuarterMaster/releases/latest) and run it. It installs to `C:\Program Files\QuarterMaster`. Setup can also install `qm`, the command-line tool for mod makers, and add it to PATH; the last page offers a Start menu entry and a desktop shortcut. Windows asks for permission once, for the install.
2. Start QuarterMaster. The first start finds the game and your saves (press **Auto-detect**, or **Browse** to `mnb.exe`), suggests a workspace folder and asks how to start the game. All of it can be changed later in Settings.
3. Add mods on the **Browse** tab, or drop `.zip` files and folders onto the window. Then press **Launch**.

Windows only. QuarterMaster works by placing a small library, `winmm.dll`, next to `mnb.exe` when it launches the game. [VERIFICATION.md](VERIFICATION.md) explains what that library does and does not touch, and how to build or inspect it yourself.

### "Windows protected your PC"

The first time you run the installer, Windows may show a blue box that says **Windows protected your PC**. Click **More info**, then **Run anyway**.

That box is SmartScreen. It appears for any program that is not signed with a paid code-signing certificate and that few people have downloaded yet; it does not mean something was found in the file. QuarterMaster is a free project and is not signed. If you would rather check than trust:

- download only from this repository's [releases](https://github.com/Gamiseus/QuarterMaster/releases);
- every release lists a `.sha256` file for its zip, and the source is here to read and build yourself ([Building from source](#building-from-source));
- [VERIFICATION.md](VERIFICATION.md) says what the game library does and how to inspect it.

An antivirus program may also question `winmm.dll`, because loading with a game is what it is for. VERIFICATION.md explains exactly what it does.

### Without the installer

`QuarterMaster_<version>.zip` in the same release is the portable copy: unpack it into any folder you can write to and run `qm-launcher.exe`. Keep `qm_winmm.dll` and `portable.txt` beside it; `qm.exe` is the command-line tool.

### Where your data is

Either way, QuarterMaster keeps everything (mods, profiles, save slots, backups, settings) in `qm-data` beside the program, and nothing in the game folder but its `winmm.dll` and a `Mods` link while a modded profile is in use. An earlier copy kept its data in `%APPDATA%\QuarterMaster`; the first start offers to move that over.

### Coming from a manual install

If you installed an older QuarterMaster by hand (`winmm.dll` and a `Mods` folder in the game folder), leave them where they are and start the launcher. It finds them and offers **Import**: your mods go into its library, the old `Mods` folder is moved to its backups folder, and the old `winmm.dll` is deleted. A mod that states no version is imported as 0.0.0.

### Updating and removing

QuarterMaster checks for its own updates and for mod updates (Settings > Updates) and installs them when you say so. To remove it, use **Apps > Installed apps** in Windows: the uninstaller also takes `winmm.dll` and the `Mods` link out of the game folder, so the game runs without mods again. Tick **Also delete my mods, profiles, save slots, backups and settings** to remove `qm-data` as well; left unticked, it stays for a later install. Your mod workspace folder is never deleted.

## Reporting a problem

In the launcher, open **Diagnostics** and press **Report a problem** (it is also under Settings > About). Say what it is about and what happened; the launcher opens a new issue with your text filled in and makes a report file for you to drag onto it. The file holds your mod list, load order, QuarterMaster settings, the last lines of the logs and recent crash reports, with your Windows user name and Steam id removed. Nothing is sent until you press **Create** on GitHub.

If the launcher will not start, [open an issue by hand](https://github.com/Gamiseus/QuarterMaster/issues/new/choose). A good report has:

- the QuarterMaster version (Settings > About) and the game version;
- what you did, what you expected and what happened instead;
- for a crash or a mod that does not load: which mods were on, and the log. The launcher's log is `qm-data\logs\launcher.log`; the game's is under `qm-data\profiles\<profile>\Mods\qm\log`.

Search the [open issues](https://github.com/Gamiseus/QuarterMaster/issues) first. A problem with one mod usually belongs to that mod's author. A GitHub account is free.

## Making mods

A mod is a folder with any of: `mod.json` (its name and version), `qm.toml` (changes to game code), `config.toml` (settings and keybinds), `faction.toml` (a faction) and `assets/` (replacement sounds, music, sprites and fonts). The [modding wiki](https://gamiseus.github.io/wiki/) documents all of them, and the `qm` command-line tool checks and packs a mod.

## Building from source

You need [Rust](https://rustup.rs) (the pinned nightly toolchain is selected automatically), the Visual Studio C++ build tools with "Desktop development with C++", and Node.js 20 or newer for the launcher.

```powershell
# The loader library: target\release\qm_winmm.dll
cargo build --release -p qm-win

# The launcher: target\release\qm-launcher.exe
cd launcher-ui
npm install
cd ..\crates\qm-launcher
cargo tauri build
```

`tools\build-release.ps1` builds the installer, the portable zip and its checksum into `target\release\dist`. [TESTING.md](TESTING.md) has the test commands.

| Folder | What it is |
|---|---|
| `crates/qm-win` | The library loaded with the game |
| `crates/qm-core` | The patch engine and asset loader |
| `crates/qm-schema` | The mod file formats |
| `crates/qm-manager`, `crates/qm-service` | The mod library, profiles, saves and the launcher's back end |
| `crates/qm-launcher`, `launcher-ui` | The launcher |
| `crates/qm-cli` | The `qm` command-line tool |
| `schemas` | JSON schemas for `mod.json`, `qm.toml` and `faction.toml` |

## Credits

The in-memory Lua hooking and the TOML patch model were inspired by [Lovely](https://github.com/ethangreen-dev/lovely-injector) by Ethan Green, made for *Balatro*.
