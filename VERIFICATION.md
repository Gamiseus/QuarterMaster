# QuarterMaster (QM) — Security Audit & Safety Guarantees

---

## 1. Why QuarterMaster Uses a Proxy DLL

*Mud and Blood* is built using the **LÖVE 2D engine (v11.4)** and packaged as a standalone Windows PE executable (`mnb.exe`). In this architecture:
- Game logic is written in Lua.
- The Lua scripts and game assets are bundled into the game archive.
- The executable links to `lua51.dll` to execute Lua bytecode at runtime.

To allow users to install mods without permanently unpacking, modifying, and repackaging `mnb.exe` (which breaks digital signatures, complicates game updates, and risks save corruption), QuarterMaster intercepts Lua scripts as they are loaded into memory.

---

## 2. Proxy DLL Mechanism (`winmm.dll`)

### Windows DLL Search Order
When an executable launches on Microsoft Windows, the operating system's dynamic loader searches for imported dynamic-link libraries according to a strict search hierarchy:
1. **The directory from which the application loaded** (the game folder containing `mnb.exe`).
2. The system directory (`C:\Windows\System32`).
3. The 16-bit system directory.
4. The Windows directory (`C:\Windows`).
5. The current working directory.
6. Directories listed in the system `PATH` environment variable.

Because `mnb.exe` imports `winmm.dll` (the standard Windows Multimedia library used for timing and sound), placing a custom `winmm.dll` directly inside the game folder causes Windows to load QuarterMaster upon launch.

### Export Forwarding
To ensure Windows Multimedia functionality is never disrupted, QuarterMaster automatically forwards every function export to the authentic Microsoft `C:\Windows\System32\winmm.dll`.

Using the `forward-dll` crate at compile time, QuarterMaster inspects the system `winmm.dll` exports and generates direct module definition directives. Functions such as:
- `timeGetTime`
- `timeBeginPeriod` / `timeEndPeriod`
- `PlaySoundA` / `PlaySoundW`
- `waveOutOpen` / `waveOutWrite`
- `joyGetPosEx`

are forwarded directly to the operating system's native DLL. The game runs identically to vanilla with zero latency penalty or missing multimedia functions.

---

## 3. Lua Hook & Runtime Patching

### Locating `lua51.dll`
Upon process initialization (`DllMain` with `DLL_PROCESS_ATTACH`), QuarterMaster:
1. Retrieves the module handle for `lua51.dll` (already mapped into `mnb.exe`'s virtual address space).
2. Obtains the memory address of the exported function `luaL_loadbuffer` (and `luaL_loadbufferx` if present).

### In-Memory Detour (Retour / MinHook)
QuarterMaster installs an inline detour on `luaL_loadbuffer`:
```c
int hooked_luaL_loadbuffer(lua_State *L, const char *buff, size_t size, const char *name);
```

When `mnb.exe` loads a file (for example, `main.lua` or `includes/ai.lua`):
1. `luaL_loadbuffer` is invoked by the game engine with the vanilla file buffer.
2. The detour receives the buffer and checks whether any active mod in `Mods/` specifies patches for that file target (via `qm.toml` or `lovely.toml`).
3. If patches exist, QuarterMaster applies pattern, regex, or module injections in memory.
4. The patched memory buffer is passed to the original `luaL_loadbuffer`.
5. The game continues execution seamlessly.

**Crucially, no files on disk are modified.** The vanilla game archive and executables remain 100% pristine.

---

## 4. Safety & Security Guarantees

| Security Concern | Injector Status & Guarantee | Verification |
| :--- | :--- | :--- |
| **Network & Telemetry** | **NONE.** The DLL contains zero networking code, socket APIs, HTTP clients, or telemetry tracking. | Inspected imports: `ws2_32.dll`, `wininet.dll`, `winhttp.dll` are not linked. |
| **Disk & File Modification** | **READ-ONLY on Game Files.** QuarterMaster reads mod definitions from `Mods/` and writes diagnostics only to mod logs (`Mods/qm/log/`). Vanilla game files are never overwritten. | Code audited in `crates/qm-core/src/dump.rs` and `crates/qm-win/src/lib.rs`. |
| **System Integrity** | **NO SYSTEM INJECTIONS.** The DLL only operates inside the memory space of `mnb.exe`. It does not inject into `explorer.exe`, other games, or system processes. | Verified via Windows API scopes (`GetCurrentProcess`). |
| **Credential / Keyloggers** | **NONE.** The DLL does not hook keyboard messages (`WH_KEYBOARD`), read registry credentials, or inspect external memory. | Clean audit across all source files. |
| **Open Source** | **100% Open Source.** Complete source code, build scripts, and dependencies are provided in this repository. | MIT licensed. |

---

## 5. How to Build and Audit From Source

### Prerequisites
- Install the Rust toolchain: [https://rustup.rs](https://rustup.rs)
- Ensure MSVC build tools are installed (Visual Studio 2022 C++ build tools or Windows SDK). They may not be part of your path by default.
- If the MSVC build tools aren't installed in your system path, call them directly with the included alternate command as a format - mine is the most likely scenario, but yours may be installed differently.

### Build Steps
```bash
# 1. Open powershell or Git Bash in the root directory

# 2. Run unit tests
cargo test

# 3. Compile the release DLL
cargo build --release --package qm-win

# 4. Locate the output binary
# target/release/winmm.dll
```

### Verification
You can compare the SHA-256 hash or export table of your freshly compiled binary against the distributed binary using PowerShell:
```powershell
# 1. Get the hash of the dll file you just built
Get-FileHash -Algorithm SHA256 .\target\release\winmm.dll

# 2. Do it again for the one you downloaded to verify it's identical. Ie "C:\Program Files (x86)\Steam\steamapps\common\MudandBlood\winmm.dll"
Get-FileHash -Algorithm SHA256 PATH\TO\DOWNLOADED\winmm.dll

# 3. If you feel like going further
dumpbin /EXPORTS .\target\release\winmm.dll
# Alternate command if dumpbin isn't found
$dumpbin = (Get-ChildItem "C:\Program Files*\Microsoft Visual Studio" -Recurse -Filter "dumpbin.exe" -ErrorAction SilentlyContinue | Select-Object -First 1).FullName
& $dumpbin /EXPORTS .\target\release\winmm.dll
```
The exports will list the forwarded Windows Multimedia routines (`timeGetTime`, etc.) confirming export integrity.
