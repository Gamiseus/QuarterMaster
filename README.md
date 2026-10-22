# QuarterMaster (QM) — Mod Loader & Runtime Hook for Mud and Blood

**QuarterMaster (QM)** is the dedicated, high-performance runtime mod loader and in-memory patch engine built specifically for **Mud and Blood** (running on the LÖVE 2D engine).

Named after the military logistics officer responsible for provisioning supplies, rations, and equipment to troops on the front line, QuarterMaster equips *Mud and Blood* with dynamic mod support without ever altering the original game files or executable on disk.

---


1. **Self-Contained Portable Mod Discovery**:
   - Checks local `<game_dir>/Mods` first before falling back to system `%APPDATA%/mnb/Mods`. The game and mods run 100% portably from any directory, external drive, or custom installation.
2. **Dedicated Mod Manifests & Variables**:
   - Uses native `qm.toml` (or `quartermaster.toml`) patch manifests.
   - Interpolates `{{qm:VAR}}` and `{{quartermaster:VAR}}` tokens.
3. **Auto-Rotating Log Architecture**:
   - **`latest.log` & `prev.log`**: Automatically rotates the previous log to `prev.log` upon each launch, preventing file clutter.
   - **`err.log` & `prev_err.log`**: Errors and unhandled warnings are mirrored into a dedicated `err.log` for immediate visibility.
   - **`crashes/` Directory**: Captures full crash dumps and panics into `Mods/qm/log/crashes/crash-{timestamp}.log`, capped at a maximum of 10 logs (oldest pruned automatically).
4. **Hardened Safety & Export Forwarding**:
   - Safe UTF-16 panic hooks prevent secondary crashes during unhandled errors.
   - Windows Multimedia API exports (`timeGetTime`, etc.) forward directly to `C:\Windows\System32\winmm.dll`.
5. **Disabled Steam Achievements**:
   - Disables tracking for achievements and leaderboards

See **[`VERIFICATION.md`](VERIFICATION.md)** for info on how to build and verify the dll contents on your own.

---

## Directory Structure

```
Quartermaster/
├── Cargo.toml                       # Root Cargo workspace manifest
├── Cargo.lock                       # Locked dependency tree for deterministic builds
├── VERIFICATION.md              # Security audit, guarantees, and verification steps
├── README.md                        # This documentation
├── crates/
│   ├── qm-win/                      # Windows DLL proxy implementation (compiles winmm.dll)
│   │   ├── Cargo.toml
│   │   ├── build.rs                 # Forward-dll build script referencing system32\winmm.dll
│   │   └── src/
│   │       ├── lib.rs               # DllMain, detour registration, and console integration
│   │       └── lualib.rs            # Safe FFI bindings for lua51.dll functions
│   ├── qm-core/                     # Core patch engine and TOML/regex processor
│   │   ├── Cargo.toml
│   │   └── src/                     # In-memory buffer patcher, rope edits, and module loader
│   └── minimal-minhook-reference/   # Standalone minimal reference hook (~120 lines of Rust)
│       ├── Cargo.toml
│       ├── build.rs
│       └── src/lib.rs
```

---

## How It Works

1. **Proxy Loading**:
   Windows launches `mnb.exe`. Because `mnb.exe` imports `winmm.dll`, Windows loads the `winmm.dll` located in the game directory first (Windows DLL Search Order).
2. **Export Forwarding**:
   All Windows Multimedia calls (`timeGetTime`, `PlaySound`, `waveOutWrite`, etc.) pass through with zero delay to `C:\Windows\System32\winmm.dll`.
3. **Lua Interception**:
   QuarterMaster detects `lua51.dll` in memory and installs an in-memory detour on `luaL_loadbuffer` / `luaL_loadbufferx`.
4. **Mod Injection**:
   When the game loads scripts (`main.lua`, `includes/ai.lua`, `includes/urb_engine.lua`, etc.), QuarterMaster applies patches from active mods in `Mods/` directly in RAM before passing the bytecode to the LÖVE Lua interpreter.
5. **Clean Disk State**:
   Vanilla game files remain 100% unaltered.

---

## Building from Source

### Requirements
- **Rust Toolchain**: 1.75+ (`stable-x86_64-pc-windows-msvc` or `nightly-x86_64-pc-windows-msvc`)
- **Visual Studio Build Tools**: 2019 or 2022 with C++ / Windows SDK

### Build Commands
```cmd
# Navigate to the Quartermaster workspace
cd Quartermaster

# Run test suite
cargo test

# Compile release DLL
cargo build --release --package qm-win
```

The compiled proxy DLL will be output to:
```
target\release\winmm.dll
```
Copy `winmm.dll` to the `MudandBlood` game folder next to `mnb.exe` to deploy.

---

## Attribution & Credits

- **Original Architecture & Concept**: QuarterMaster's in-memory Lua hooking architecture was inspired by the **Lovely** injector created by **Ethan Green (@ethangreen-dev)** for *Balatro*.
