use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;
use std::sync::Once;
use std::thread;
use std::time::Duration;

extern "system" {
    fn GetModuleHandleA(lpModuleName: *const c_char) -> *mut c_void;
    fn GetProcAddress(hModule: *mut c_void, lpProcName: *const c_char) -> *mut c_void;
}

type FnLoadBuffer = unsafe extern "C" fn(
    state: *mut c_void,
    buff: *const c_char,
    sz: usize,
    name: *const c_char,
) -> c_int;

static mut ORIG_LOADBUFFER: Option<FnLoadBuffer> = None;
static INIT_ONCE: Once = Once::new();

fn log(msg: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("modloader.log") {
        let _ = writeln!(f, "{}", msg);
    }
}

unsafe extern "C" fn hooked_loadbuffer(
    state: *mut c_void,
    buff: *const c_char,
    sz: usize,
    name: *const c_char,
) -> c_int {
    let chunk_name = if !name.is_null() {
        CStr::from_ptr(name).to_string_lossy().to_string()
    } else {
        String::new()
    };

    let clean_name = chunk_name.trim_start_matches('@').trim_start_matches('=');

    // 1. Check for direct file override in "mods/<clean_name>"
    let override_path = Path::new("mods").join(clean_name);
    if override_path.is_file() {
        if let Ok(modded_bytes) = fs::read(&override_path) {
            log(&format!(
                "[ModLoader] OVERRIDE: {} (replaced {} vanilla bytes with {} bytes from {})",
                clean_name, sz, modded_bytes.len(), override_path.display()
            ));
            let c_name = CString::new(chunk_name.clone()).unwrap_or_default();
            return ORIG_LOADBUFFER.unwrap()(
                state,
                modded_bytes.as_ptr() as *const c_char,
                modded_bytes.len(),
                c_name.as_ptr(),
            );
        }
    }

    // 2. If loading main.lua, inject package.path expansion and mod bootstrap at the top
    if clean_name == "main.lua" {
        let orig_slice = if !buff.is_null() && sz > 0 {
            std::slice::from_raw_parts(buff as *const u8, sz)
        } else {
            &[]
        };

        let bootstrap = b"\
-- [Mud and Blood ModLoader Injected Bootstrap]
print('[ModLoader] Runtime Lua injection active!')
package.path = 'mods/?.lua;mods/?/init.lua;' .. package.path
if love and love.filesystem and love.filesystem.mount then
    local exeDir = love.filesystem.getSourceBaseDirectory and love.filesystem.getSourceBaseDirectory() or '.'
    pcall(love.filesystem.mount, exeDir .. '/mods', 'mods')
end
-- [End ModLoader Bootstrap]
";

        let mut combined = Vec::with_capacity(bootstrap.len() + orig_slice.len());
        combined.extend_from_slice(bootstrap);
        combined.extend_from_slice(orig_slice);

        log(&format!(
            "[ModLoader] INJECTED BOOTSTRAP into main.lua (total size: {} bytes)",
            combined.len()
        ));

        let c_name = CString::new(chunk_name.clone()).unwrap_or_default();
        return ORIG_LOADBUFFER.unwrap()(
            state,
            combined.as_ptr() as *const c_char,
            combined.len(),
            c_name.as_ptr(),
        );
    }

    log(&format!("[ModLoader] Vanilla: {} ({} bytes)", clean_name, sz));
    ORIG_LOADBUFFER.unwrap()(state, buff, sz, name)
}

fn init_modloader() {
    log("==================================================");
    log("[ModLoader] M&B Runtime ModLoader initialized (version.dll)");
    log("==================================================");

    thread::spawn(|| {
        let lua_name = CString::new("lua51.dll").unwrap();
        let fn_name = CString::new("luaL_loadbuffer").unwrap();

        let mut h_module = std::ptr::null_mut();
        for _ in 0..100 {
            unsafe {
                h_module = GetModuleHandleA(lua_name.as_ptr());
            }
            if !h_module.is_null() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        if h_module.is_null() {
            log("[ModLoader] ERROR: Could not find lua51.dll in process memory!");
            return;
        }

        let proc_addr = unsafe { GetProcAddress(h_module, fn_name.as_ptr()) };
        if proc_addr.is_null() {
            log("[ModLoader] ERROR: Could not find luaL_loadbuffer in lua51.dll!");
            return;
        }

        unsafe {
            match minhook::MinHook::create_hook(proc_addr, hooked_loadbuffer as *mut c_void) {
                Ok(orig) => {
                    ORIG_LOADBUFFER = Some(std::mem::transmute(orig));
                    if let Err(e) = minhook::MinHook::enable_all_hooks() {
                        log(&format!("[ModLoader] ERROR enabling hook: {:?}", e));
                    } else {
                        log("[ModLoader] Hook successfully installed on luaL_loadbuffer!");
                    }
                }
                Err(e) => {
                    log(&format!("[ModLoader] ERROR creating hook: {:?}", e));
                }
            }
        }
    });
}

#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn DllMain(
    _hinst: *mut c_void,
    reason: u32,
    _reserved: *mut c_void,
) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        INIT_ONCE.call_once(|| {
            init_modloader();
        });
    }
    1
}
