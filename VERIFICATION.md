# QuarterMaster (QM) — How the DLL Works & How to Verify It

Dropping a custom `.dll` file into a game folder is something you should always be skeptical of. This document explains why QuarterMaster uses `winmm.dll`, what it actually does on your machine, and how you can build it from source or inspect the binary yourself so you don't have to take anyone's word for it.

---

## 1. Why QuarterMaster Uses a Proxy DLL

*Mud and Blood* runs on **LÖVE 2D (v11.4)** and ships as a single fused executable (`mnb.exe`) with all Lua scripts and assets zipped inside the `.exe` itself. When the game runs, `mnb.exe` unpacks those Lua files in memory and passes them to `lua51.dll`.

Without a runtime hook, installing a mod means unpacking `mnb.exe`, editing the game's Lua files directly on disk, and either running unpacked files or repacking the executable. That breaks every time the game updates and makes it impossible to toggle mods on and off cleanly.

By loading as a DLL alongside `mnb.exe`, QuarterMaster can intercept Lua scripts in memory right as `lua51.dll` loads them, apply patches from your `Mods/` folder, and leave `mnb.exe` untouched.

---

## 2. How `winmm.dll` Loads Without Breaking Audio or Timing

When Windows starts `mnb.exe`, it loads the DLLs the executable asks for by checking the game's own folder first before checking `C:\Windows\System32`.

`mnb.exe` imports `winmm.dll` (the standard Windows Multimedia library). When you place QuarterMaster's `winmm.dll` in the game folder:
1. Windows loads QuarterMaster's `winmm.dll` into the `mnb.exe` process.
2. QuarterMaster uses compile-time export forwarding (`forward-dll`, configured in `crates/qm-win/build.rs`) to forward every Windows Multimedia function (`timeGetTime`, `PlaySoundW`, `waveOutOpen`, `joyGetPosEx`, etc.) straight to the real `C:\Windows\System32\winmm.dll`.
3. Windows handles those forwarded calls natively, while QuarterMaster's `DllMain` sets up the Lua hook in the background.

---

## 3. What Happens When the Game Runs

1. **Finding `lua51.dll`**: On startup (`DLL_PROCESS_ATTACH`), QuarterMaster looks up `lua51.dll` inside the current `mnb.exe` process and finds `luaL_loadbuffer` / `luaL_loadbufferx`.
2. **Hooking script loads**: QuarterMaster places an inline detour (`retour`) on `luaL_loadbuffer`.
3. **Applying patches in RAM**: Whenever the game loads a script (like `main.lua` or `includes/urb_engine.lua`), QuarterMaster checks your active `Mods/` folder for matching `qm.toml` patches or `assets/` files, edits the script buffer in memory, and hands the patched text to the original `luaL_loadbuffer`.
4. **Blocking Steam stats/leaderboards**: QuarterMaster injects a small Lua snippet at the top of `main.lua` and `includes/external.lua` that stubs out Steam leaderboard uploads and achievement/stat writes (`Steam.userStats.uploadLeaderboardScore`, `setAchievement`, `setStatInt`, etc.) so modded runs stay off the public leaderboards.

---

## 4. What QuarterMaster Touches (and What It Doesn't)

Here is the exact scope of what the proxy DLL (`qm-win` + `qm-core`) does at runtime:

| Area | What `winmm.dll` Does | How to Check |
| :--- | :--- | :--- |
| **Network Access** | **None.** The proxy DLL has no networking code, sockets, HTTP clients, or telemetry. *(Note: the optional desktop manager `qm-launcher.exe` can check GitHub for mod updates if you use it, but `winmm.dll` itself never touches the network.)* | Run `dumpbin /IMPORTS winmm.dll` — `ws2_32.dll`, `winhttp.dll`, and `wininet.dll` are not imported. |
| **Files Read** | Reads `Mods/*/qm.toml`, mod `.lua` files, and mod asset files (`Mods/*/assets/*`). | See `crates/qm-core/src/patch/loader.rs`. |
| **Files Written** | Writes only inside `Mods/qm/`: logs (`Mods/qm/log/`), patched Lua script dumps for debugging (`Mods/qm/dump/`), optional vanilla script dumps (`Mods/qm/game-dump/`), and `Mods/qm/patch-report.json`. Never modifies `mnb.exe` or vanilla files. | See `crates/qm-core/src/dump.rs` and `crates/qm-core/src/logger.rs`. |
| **Process Scope** | Runs strictly inside `mnb.exe`. Does not touch other processes, the Windows registry, or global keyboard hooks. | See `crates/qm-win/src/lib.rs`. |

---

## 5. Building and Inspecting the DLL Yourself

If you don't want to run a precompiled DLL from a release zip, you can compile `winmm.dll` from source in a couple of minutes.

### Prerequisites
1. **Rust**: Install from [rustup.rs](https://rustup.rs). The workspace includes a `rust-toolchain.toml` that automatically uses `nightly` (needed by the `retour` hooking crate).
2. **Visual Studio C++ Build Tools**: Install Visual Studio 2019/2022 or the standalone "Build Tools for Visual Studio" with the **Desktop development with C++** workload.

### Compiling from Source
Open PowerShell in the `Quartermaster` folder and run:

```powershell
# Run tests
cargo test -p qm-schema -p qm-core -p qm-manager -p qm-service

# Build the release DLL
cargo build --release -p qm-win
```

If `cargo` complains that `link.exe` is not found (meaning the Visual Studio C++ tools aren't in your default `PATH`), run the build through a Visual Studio Developer Command Prompt, or initialize the MSVC environment in PowerShell first:

```powershell
# Locate vcvars64.bat from your Visual Studio installation and load its environment into PowerShell
$vcvars = (Get-ChildItem "C:\Program Files*\Microsoft Visual Studio" -Recurse -Filter "vcvars64.bat" -ErrorAction SilentlyContinue | Select-Object -First 1).FullName
cmd /c "`"$vcvars`" && cargo build --release -p qm-win"
```

Once finished, grab `target\release\qm_winmm.dll`, copy it into your `MudandBlood` game folder next to `mnb.exe`, and rename it to `winmm.dll`.

### Inspecting a Prebuilt `winmm.dll`
> **Note on file hashes (`Get-FileHash`)**: Because the Windows MSVC linker embeds compile timestamps, machine-specific paths, and exact compiler/Windows SDK versions into PE headers, a DLL you compile locally on your PC will usually **not** have the exact same SHA-256 hash as a release DLL built on another machine—even from the exact same commit. If you want 100% certainty, simply use the `qm_winmm.dll` you just compiled above.

You can also inspect any `winmm.dll` directly using Visual Studio's `dumpbin.exe` in PowerShell to verify its forwarded exports and imported system libraries:

```powershell
# Find dumpbin.exe if it isn't already in your PATH
$dumpbin = (Get-ChildItem "C:\Program Files*\Microsoft Visual Studio" -Recurse -Filter "dumpbin.exe" -ErrorAction SilentlyContinue | Select-Object -First 1).FullName

# 1. Check all exported functions (you should see the winmm multimedia calls forwarded to C:\Windows\System32\winmm.dll)
& $dumpbin /EXPORTS .\target\release\qm_winmm.dll

# 2. Check all imported DLLs and APIs (confirms no networking or unexpected system DLLs are linked)
& $dumpbin /IMPORTS .\target\release\qm_winmm.dll
```
