#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
mod lualib;

use std::env;
use std::ffi::c_void;
use std::panic;
use std::sync::{LazyLock, OnceLock};
use anyhow::Context;
use itertools::Itertools;
use qm_core::handshake::{launch_flag, LaunchConfig};
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

/// What the panic hook does with a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanicAction {
    /// Inside the guarded detour (D34): log only; the detour catches the panic, turns mods off
    /// and loads the builtin-only text.
    LogOnly,
    /// Anywhere else: crash report, message box and abort, as before.
    ReportAndAbort,
}

fn panic_action(guarded: bool) -> PanicAction {
    if guarded {
        PanicAction::LogOnly
    } else {
        PanicAction::ReportAndAbort
    }
}

/// Whether DllMain returns before installing any hook (vanilla). A launch-config.json that
/// cannot be read or parsed never does: its `vanilla_mode` is unknown, so QuarterMaster starts,
/// loads no mods and keeps its safety edits (D65). The command line and the environment
/// still turn mods off.
fn takes_vanilla_return(
    launch: &Result<Option<LaunchConfig>, String>,
    args: &[String],
    env_disables_mods: bool,
) -> bool {
    let config_vanilla = matches!(launch, Ok(Some(c)) if c.vanilla_mode);
    config_vanilla
        || env_disables_mods
        || args
            .iter()
            .any(|a| matches!(a.as_str(), "--vanilla" | "-v" | "--disable-mods" | "-d"))
}

unsafe extern "C" fn lua_loadbufferx_detour(
    state: *mut LuaState,
    buf_ptr: *const u8,
    size: usize,
    name_ptr: *const u8,
    mode_ptr: *const u8,
) -> u32 {
    match RUNTIME.get() {
        Some(rt) => rt.load_buffer_guarded(state, buf_ptr, size, name_ptr, mode_ptr),
        // Not reached: RUNTIME is set before the hooks are enabled.
        None => LuaLoadbufferx_Detour.call(state, buf_ptr, size, name_ptr, mode_ptr),
    }
}

/// Shows `message` in a message box.
unsafe fn message_box(message: &str) {
    let u16_msg = U16CString::from_str_truncate(message);
    MessageBoxW(
        HWND(0),
        PCWSTR(u16_msg.as_ptr()),
        PCWSTR(WIN_TITLE.as_ptr()),
        MESSAGEBOX_STYLE(0),
    );
}

/// Writes the crash report, shows `message` and stops the game.
unsafe fn report_and_abort(message: &str, detail: &str) -> ! {
    if let Some(rt) = RUNTIME.get() {
        let _ = rt.build_and_save_crash_report("rust_panic", message, message);
    } else {
        qm_core::log::record_crash(detail);
    }
    message_box(message);
    std::process::abort();
}

/// A start-up failure: report it, show it and stop the game, exactly as the panic hook does
/// for a panic outside the guard.
unsafe fn fatal(detail: &str) -> ! {
    let message = format!("QuarterMaster (QM) has encountered an unexpected crash:\n{detail}");
    error!("{message}");
    report_and_abort(&message, detail)
}

/// The DLL's panic hook. Inside the guard it only logs and hands its message (it has the
/// location) to the guarded caller, which recovers (D34). Anywhere else, including a Lua
/// callback (`guard::unguarded`), it reports, shows the message box and aborts.
fn qm_panic_hook(info: &panic::PanicHookInfo<'_>) {
    let message = format!("QuarterMaster (QM) has encountered an unexpected crash:\n{info}");
    error!("{message}");

    if panic_action(qm_core::guard::is_guarded()) == PanicAction::LogOnly {
        qm_core::guard::record_panic(message);
        return;
    }

    unsafe { report_and_abort(&message, &format!("{info}")) }
}

#[no_mangle]
#[allow(non_snake_case)]
unsafe extern "system" fn DllMain(_: HINSTANCE, reason: u32, _: *const c_void) -> u8 {
    if reason != 1 {
        return 1;
    }

    // Install safe crash/panic hook with null-safe UTF-16 conversion
    panic::set_hook(Box::new(qm_panic_hook));

    let args = env::args().collect_vec();
    let default_mod_dir = QuarterMaster::resolve_default_mod_dir();
    let loaded_cfg = LaunchConfig::load_from_mod_dir(&default_mod_dir);

    // Check for vanilla bypass flags (CLI args, env var, or Mods/qm/launch-config.json)
    let env_disables_mods = env::var("QM_DISABLE_MODS").map(|v| v == "1").unwrap_or(false);
    if takes_vanilla_return(&loaded_cfg, &args, env_disables_mods) {
        return 1;
    }
    // A broken file gives the defaults here; qm-core reports it and loads no mods (D65).
    let launch_cfg = loaded_cfg.ok().flatten().unwrap_or_default();

    // The console window opens only in debug mode; the log file is written either way. The
    // older `disable_console` launch setting is no longer read.
    let debug = launch_cfg.debug || launch_flag(&args, "debug", "QM_DEBUG");
    let console_disabled = args.contains(&"--disable-console".to_string())
        || env::var("QM_DISABLE_CONSOLE").map(|v| v == "1").unwrap_or(false);

    if debug && !console_disabled {
        let _ = AllocConsole();
        let _ = SetConsoleTitleW(PCWSTR(WIN_TITLE.as_ptr()));
    }

    let dump_all = launch_cfg.dump_all
        || args.contains(&"--dump-all".to_string())
        || env::var("QM_DUMP_ALL").map(|v| v == "1").unwrap_or(false);

    // Initialize the QuarterMaster runtime
    let lualib = lualib::get_lualib().with_context(|| {
        format!(
            "\n\n[QuarterMaster] Could not bind Lua runtime library.\n\
             Ensure QuarterMaster (winmm.dll) is running alongside Mud and Blood (mnb.exe).\n\
             Executable path: {}",
            args.first().map(String::as_str).unwrap_or("unknown")
        )
    });
    let lualib = match lualib {
        Ok(lib) => lib,
        Err(e) => fatal(&format!("{e:?}")),
    };
    let rt = QuarterMaster::init(
        &|a, b, c, d, e| LuaLoadbufferx_Detour.call(a, b, c, d, e),
        lualib,
        dump_all,
    );
    if RUNTIME.set(rt).is_err() {
        fatal("Failed to instantiate QuarterMaster runtime.");
    }

    // Locate lua51.dll module in memory, or load it
    let mut handle = GetModuleHandleW(w!("lua51.dll")).unwrap_or_default();
    if handle.is_invalid() {
        handle = match LoadLibraryW(w!("lua51.dll")) {
            Ok(h) => h,
            Err(e) => fatal(&format!("Could not load lua51.dll: {e:?}")),
        };
    }

    // Hook luaL_loadbufferx if present
    if let Some(proc) = GetProcAddress(handle, s!("luaL_loadbufferx")) {
        let fn_target = std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "C" fn(*mut std::ffi::c_void, *const u8, usize, *const u8, *const u8) -> u32,
        >(proc);

        let hooked = LuaLoadbufferx_Detour
            .initialize(fn_target, |a, b, c, d, e| {
                lua_loadbufferx_detour(a, b, c, d, e)
            })
            .and_then(|detour| detour.enable());
        if let Err(e) = hooked {
            fatal(&format!("Could not hook luaL_loadbufferx: {e:?}"));
        }
    }

    // Hook standard luaL_loadbuffer
    let Some(proc) = GetProcAddress(handle, s!("luaL_loadbuffer")) else {
        fatal("Could not locate luaL_loadbuffer in lua51.dll");
    };
    let fn_target = std::mem::transmute::<
        unsafe extern "system" fn() -> isize,
        unsafe extern "C" fn(*mut std::ffi::c_void, *const u8, usize, *const u8) -> u32,
    >(proc);

    let hooked = LuaLoadbuffer_Detour
        .initialize(fn_target, |a, b, c, d| {
            lua_loadbufferx_detour(a, b, c, d, std::ptr::null())
        })
        .and_then(|detour| detour.enable());
    if let Err(e) = hooked {
        fatal(&format!("Could not hook luaL_loadbuffer: {e:?}"));
    }

    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use qm_core::guard;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// Every test takes this lock: one of them installs the real hook, which shows a message
    /// box and aborts on an unguarded panic (a failing assertion elsewhere).
    static HOOK: Mutex<()> = Mutex::new(());

    fn hook_lock() -> MutexGuard<'static, ()> {
        HOOK.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[test]
    fn guard_flag_decides_the_panic_hook() {
        let _lock = hook_lock();
        assert_eq!(panic_action(guard::is_guarded()), PanicAction::ReportAndAbort);
        assert_eq!(
            guard::guarded(|| panic_action(guard::is_guarded())),
            Ok(PanicAction::LogOnly)
        );
        assert_eq!(panic_action(guard::is_guarded()), PanicAction::ReportAndAbort);
    }

    #[test]
    fn a_lua_callback_inside_the_guard_reports_and_aborts() {
        let _lock = hook_lock();
        let action = guard::guarded(|| guard::unguarded(|| panic_action(guard::is_guarded())));
        assert_eq!(action, Ok(PanicAction::ReportAndAbort));
    }

    #[test]
    fn a_broken_launch_config_does_not_take_the_vanilla_return() {
        let _lock = hook_lock();
        let none: Vec<String> = Vec::new();
        let broken = Err("launch-config.json is not valid: EOF".to_string());
        assert!(!takes_vanilla_return(&broken, &none, false));
        assert!(!takes_vanilla_return(&Ok(None), &none, false));
        assert!(!takes_vanilla_return(&Ok(Some(LaunchConfig::default())), &none, false));
        let vanilla = Ok(Some(LaunchConfig {
            vanilla_mode: true,
            ..LaunchConfig::default()
        }));
        assert!(takes_vanilla_return(&vanilla, &none, false));
        // The command line and the environment still turn mods off.
        let args = vec!["mnb.exe".to_string(), "--vanilla".to_string()];
        assert!(takes_vanilla_return(&broken, &args, false));
        assert!(takes_vanilla_return(&broken, &none, true));
    }

    #[test]
    fn guarded_panic_is_caught_and_the_flag_cleared() {
        let _lock = hook_lock();
        let caught: Result<(), String> = guard::guarded(|| panic!("detour panic"));
        assert_eq!(caught, Err("detour panic".to_string()));
        assert!(!guard::is_guarded());
    }

    #[test]
    fn the_installed_hook_hands_its_message_to_the_guarded_caller() {
        let _lock = hook_lock();
        panic::set_hook(Box::new(qm_panic_hook));
        let caught: Result<(), String> = guard::guarded(|| panic!("payload only"));
        // Back to the default hook before any assertion can panic unguarded.
        let _ = panic::take_hook();

        let message = caught.expect_err("the guard catches the panic");
        assert!(
            message.starts_with("QuarterMaster (QM) has encountered an unexpected crash:\n"),
            "{message}"
        );
        assert!(message.contains("payload only"), "{message}");
        assert!(message.contains("lib.rs"), "the hook adds the location: {message}");
        assert!(!guard::is_guarded());
    }
}
