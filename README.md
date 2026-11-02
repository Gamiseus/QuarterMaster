# QuarterMaster (QM) — Mod Loader & Manager for Mud and Blood

QuarterMaster is a runtime mod loader, patch engine, and desktop mod manager for **Mud and Blood** (LÖVE 11.4).

Because *Mud and Blood* packs all of its Lua scripts and assets inside `mnb.exe`, traditional modding requires unpacking the executable, editing the decompiled scripts by hand, and repacking it—which breaks whenever the game updates and makes combining multiple mods a mess. QuarterMaster hooks into the game in memory at launch and applies mod patches on the fly, leaving `mnb.exe` and your install folder untouched on disk.

For details on how the `winmm.dll` proxy works under the hood, what it touches on your system, and how to compile or inspect it yourself, see **[`VERIFICATION.md`](VERIFICATION.md)**.

---

## Features

- **In-Memory Lua Patching (`qm.toml`)**:
  Applies pattern patches, regex replacements, file appends/prepends, and new Lua modules in RAM as `mnb.exe` loads its scripts (`main.lua`, `includes/ai.lua`, `includes/urb_engine.lua`, etc.).
- **Zero-Code Asset Autopatcher (`assets/`)**:
  Mods can drop replacement or new `.wav`, `.mp3`, `.ogg`, `.png`, and `.txt` files directly into `Mods/<ModName>/assets/` (`sounds/`, `music/`, `sprites/`, `ui/`, `cursors/`, `weapons/`) without writing Lua or `qm.toml` patches. Numbered audio variations (like adding `garand5.wav` to vanilla's `garand1`–`garand4`) are pooled automatically.
- **Portable Mod Discovery**:
  Loads mods from `<game_dir>/Mods` first, falling back to `%APPDATA%/mnb/Mods`. Works from any install drive or portable folder.
- **Steam Leaderboard & Achievement Lockdown**:
  Automatically blocks Steam leaderboard uploads (`uploadLeaderboardScore`) and achievement/stat writes in memory while running modded so modded runs never pollute vanilla leaderboards.
- **Desktop Mod Manager & Patch Studio (`qm-launcher`)**:
  Includes a Tauri/Svelte desktop app for managing mod profiles, load order, save backups, conflict simulation, and authoring `qm.toml` patches against decompiled vanilla code.
- **Diagnostics & Dumps (`Mods/qm/`)**:
  - `Mods/qm/log/latest.log` & `err.log` (rotated to `prev.log` / `prev_err.log` on each launch)
  - `Mods/qm/log/crashes/` (stores up to 10 crash logs if the game panics)
  - `Mods/qm/dump/` (writes out the final patched Lua scripts so you can inspect the exact code that ran)
  - `Mods/qm/patch-report.json` (summary of every applied, warned, or failed patch)

---

## Workspace Layout

```text
Quartermaster/
├── Cargo.toml                       # Workspace manifest
├── rust-toolchain.toml              # Pinned nightly toolchain (required by retour in qm-win)
├── README.md                        # Project overview and build instructions
├── VERIFICATION.md                  # Proxy DLL explanation, security scope, and verification steps
├── launcher-ui/                     # Svelte 5 frontend for the desktop mod manager & Patch Studio
└── crates/
    ├── qm-win/                      # Windows proxy DLL (compiles to qm_winmm.dll -> winmm.dll)
    ├── qm-core/                     # Core patch engine, TOML parser, and asset autopatcher
    ├── qm-schema/                   # Shared data types and qm.toml schema definitions
    ├── qm-manager/                  # Profile staging, CAS library, save manager, and dev tools
    ├── qm-service/                  # IPC/service layer backing the desktop app and CLI
    ├── qm-launcher/                 # Tauri desktop application host
    ├── qm-cli/                      # Command-line interface (compiles to qm.exe)
    ├── qm-app/                      # Legacy standalone GUI
    └── minimal-minhook-reference/   # Minimal ~120-line Rust hook reference
```

---

## How the Loader Works

1. **DLL Loading**: When Windows launches `mnb.exe`, it looks for `winmm.dll` in the game folder before `C:\Windows\System32`. Placing QuarterMaster's `winmm.dll` next to `mnb.exe` loads QuarterMaster with the game.
2. **Multimedia Forwarding**: All real Windows Multimedia calls (`timeGetTime`, `PlaySound`, `waveOutWrite`, etc.) are forwarded straight to `C:\Windows\System32\winmm.dll`.
3. **Lua Hooking**: QuarterMaster locates `lua51.dll` in the process and hooks `luaL_loadbuffer` / `luaL_loadbufferx`.
4. **Patch Execution**: Each time LÖVE loads a Lua file from `mnb.exe`, QuarterMaster intercepts the text buffer, runs all matching patches from enabled mods in priority order, writes the patched copy to `Mods/qm/dump/` for debugging, and hands the modified buffer back to Lua.

---

## Building from Source

### Prerequisites
- **Rust**: Install via [rustup.rs](https://rustup.rs). `qm-win` uses the `retour` crate for function hooking, which requires the `nightly` compiler (automatically selected by `rust-toolchain.toml`) or `RUSTC_BOOTSTRAP=1` on stable.
- **Visual Studio C++ Build Tools**: Visual Studio 2019 or 2022 with the "Desktop development with C++" workload (MSVC linker and Windows SDK).
- **Node.js 20+** *(only needed if building the `qm-launcher` desktop UI)*.

### Build the Loader DLL (`winmm.dll`)
```powershell
cd Quartermaster

# Run the core test suite
cargo test -p qm-schema -p qm-core -p qm-manager -p qm-service

# Build the proxy DLL
cargo build --release -p qm-win
```
The compiled DLL is output to `target\release\qm_winmm.dll`. Copy it into your *Mud and Blood* folder next to `mnb.exe` and rename it to `winmm.dll`.

### Build the Desktop Launcher (`qm-launcher.exe`)
```powershell
cd Quartermaster\launcher-ui
npm install
npm run build

cd ..\crates\qm-launcher
cargo tauri build
```
The standalone executable is output to `Quartermaster\target\release\qm-launcher.exe`.

---

## Credits

- QuarterMaster's in-memory Lua hooking approach and TOML patch model were inspired by [**Lovely**](https://github.com/ethangreen-dev/lovely-injector) by Ethan Green (`@ethangreen-dev`) for *Balatro*.
