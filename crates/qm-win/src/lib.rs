mod lualib;

use std::env;
use std::ffi::c_void;
use std::panic;
use std::sync::{LazyLock, OnceLock};
use anyhow::Context;
use itertools::Itertools;
use qm_core::log::*;
use qm_core::sys::LuaState;
use qm_core::QuarterMaster;
use qm_core::QM_VERSION;

use retour::static_detour;
use widestring::U16CString;
use windows::core::{s, w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::System::Console::{AllocConsole, SetConsoleTitleW};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MESSAGEBOX_STYLE};

static RUNTIME: OnceLock<&QuarterMaster> = OnceLock::new();

static_detour! {
    pub static LuaLoadbufferx_Detour: unsafe extern "C" fn(*mut LuaState, *const u8, usize, *const u8, *const u8) -> u32;
    pub static LuaLoadbuffer_Detour: unsafe extern "C" fn(*mut LuaState, *const u8, usize, *const u8) -> u32;
}

static WIN_TITLE: LazyLock<U16CString> = LazyLock::new(|| {
    U16CString::from_str_truncate(format!("QuarterMaster (QM) v{QM_VERSION} - Mud and Blood"))
});

unsafe extern "C" fn lua_loadbufferx_detour(
    state: *mut LuaState,
    buf_ptr: *const u8,
    size: usize,
    name_ptr: *const u8,
    mode_ptr: *const u8,
) -> u32 {
    let rt = RUNTIME.get().unwrap_unchecked();
    rt.apply_buffer_patches(state, buf_ptr, size, name_ptr, mode_ptr)
}

#[no_mangle]
#[allow(non_snake_case)]
unsafe extern "system" fn DllMain(_: HINSTANCE, reason: u32, _: *const c_void) -> u8 {
    if reason != 1 {
        return 1;
    }

    // Install safe crash/panic hook with null-safe UTF-16 conversion
    panic::set_hook(Box::new(|x| unsafe {
        let message = format!("QuarterMaster (QM) has encountered an unexpected crash:\n{x}");
        error!("{message}");

        qm_core::log::record_crash(&format!("{x}"));

        let u16_msg = U16CString::from_str_truncate(&message);
        MessageBoxW(
            HWND(0),
            PCWSTR(u16_msg.as_ptr()),
            PCWSTR(WIN_TITLE.as_ptr()),
            MESSAGEBOX_STYLE(0),
        );

        std::process::abort();
    }));

    let args = env::args().collect_vec();

    // Check for vanilla bypass flags
    if args.contains(&"--vanilla".to_string())
        || args.contains(&"-v".to_string())
        || args.contains(&"--disable-mods".to_string())
        || args.contains(&"-d".to_string())
        || env::var("QM_DISABLE_MODS").map(|v| v == "1").unwrap_or(false)
    {
        return 1;
    }

    // Console attachment logic: attach console unless explicitly disabled
    let console_disabled = args.contains(&"--disable-console".to_string())
        || env::var("QM_DISABLE_CONSOLE").map(|v| v == "1").unwrap_or(false);

    if !console_disabled {
        let _ = AllocConsole();
        let _ = SetConsoleTitleW(PCWSTR(WIN_TITLE.as_ptr()));
    }

    let dump_all = args.contains(&"--dump-all".to_string())
        || env::var("QM_DUMP_ALL").map(|v| v == "1").unwrap_or(false);

    // Initialize the QuarterMaster runtime
    let rt = QuarterMaster::init(
        &|a, b, c, d, e| LuaLoadbufferx_Detour.call(a, b, c, d, e),
        lualib::get_lualib().with_context(|| {
            format!(
                "\n\n[QuarterMaster] Could not bind Lua runtime library.\n\
                 Ensure QuarterMaster (winmm.dll) is running alongside Mud and Blood (mnb.exe).\n\
                 Executable path: {}",
                args.first().map(String::as_str).unwrap_or("unknown")
            )
        }).unwrap(),
        dump_all,
    );
    RUNTIME
        .set(rt)
        .unwrap_or_else(|_| panic!("Failed to instantiate QuarterMaster runtime."));

    // Locate lua51.dll module in memory, or load it
    let mut handle = GetModuleHandleW(w!("lua51.dll")).unwrap_or_default();
    if handle.is_invalid() {
        handle = LoadLibraryW(w!("lua51.dll")).unwrap();
    }

    // Hook luaL_loadbufferx if present
    if let Some(proc) = GetProcAddress(handle, s!("luaL_loadbufferx")) {
        let fn_target = std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "C" fn(*mut std::ffi::c_void, *const u8, usize, *const u8, *const u8) -> u32,
        >(proc);

        LuaLoadbufferx_Detour
            .initialize(fn_target, |a, b, c, d, e| {
                lua_loadbufferx_detour(a, b, c, d, e)
            })
            .unwrap()
            .enable()
            .unwrap();
    }

    // Hook standard luaL_loadbuffer
    let proc = GetProcAddress(handle, s!("luaL_loadbuffer")).expect("Could not locate luaL_loadbuffer in lua51.dll");
    let fn_target = std::mem::transmute::<
        unsafe extern "system" fn() -> isize,
        unsafe extern "C" fn(*mut std::ffi::c_void, *const u8, usize, *const u8) -> u32,
    >(proc);

    LuaLoadbuffer_Detour
        .initialize(fn_target, |a, b, c, d| {
            lua_loadbufferx_detour(a, b, c, d, std::ptr::null())
        })
        .unwrap()
        .enable()
        .unwrap();

    1
}
