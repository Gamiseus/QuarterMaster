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

1. Download `qm-launcher.exe` from the [releases](https://github.com/Gamiseus/QuarterMaster/releases) and run it.
2. On first start it finds the game, or asks where `mnb.exe` is.
3. Add mods, then press **Launch**.

QuarterMaster works by placing a small library, `winmm.dll`, next to `mnb.exe` when it launches the game. [VERIFICATION.md](VERIFICATION.md) explains what that library does and does not touch, and how to build or inspect it yourself. Windows only.

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

[TESTING.md](TESTING.md) has the test commands.

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
