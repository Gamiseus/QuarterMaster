//! Tests of the DLL entry (`QuarterMaster::load_buffer_guarded`, Phase C D34). The Lua side runs
//! against stub functions and `loadbuffer` records the text it is given, so the tests see
//! exactly what the game would load.
use super::*;
use std::ffi::{c_char, c_void, CString};
use std::sync::Mutex;

use crate::patch::loader::STEAM_SAFETY_PAYLOAD;

const EMPTY: &[u8] = b"\0";

unsafe extern "C" fn s_call(_: *mut LuaState, _: c_int, _: c_int) {}
unsafe extern "C" fn s_pcall(_: *mut LuaState, _: c_int, _: c_int, _: c_int) -> c_int {
    0
}
unsafe extern "C" fn s_field(_: *mut LuaState, _: c_int, _: *const c_char) {}
unsafe extern "C" fn s_gettop(_: *mut LuaState) -> c_int {
    0
}
unsafe extern "C" fn s_index(_: *mut LuaState, _: c_int) {}
unsafe extern "C" fn s_pushcclosure(_: *mut LuaState, _: LuaFunc, _: c_int) {}
unsafe extern "C" fn s_tolstring(_: *mut LuaState, _: c_int, len: *mut usize) -> *const c_char {
    if !len.is_null() {
        *len = 0;
    }
    EMPTY.as_ptr() as _
}
unsafe extern "C" fn s_type(_: *mut LuaState, _: c_int) -> c_int {
    0
}
unsafe extern "C" fn s_pushstring(_: *mut LuaState, _: *const char) {}
unsafe extern "C" fn s_pushnumber(_: *mut LuaState, _: f64) {}
unsafe extern "C" fn s_pushboolean(_: *mut LuaState, _: c_int) {}
unsafe extern "C" fn s_toboolean(_: *mut LuaState, _: c_int) -> c_int {
    0
}
unsafe extern "C" fn s_createtable(_: *mut LuaState, _: c_int, _: c_int) {}
unsafe extern "C" fn s_error(_: *mut LuaState) -> c_int {
    0
}
unsafe extern "C" fn s_register(_: *mut LuaState, _: *const char, _: *const c_void) {}
unsafe extern "C" fn s_checklstring(_: *mut LuaState, _: c_int, len: *mut usize) -> *const char {
    if !len.is_null() {
        *len = 0;
    }
    EMPTY.as_ptr() as _
}

fn stub_lua() {
    let _ = LUA.set(LuaLib {
        lua_call: s_call,
        lua_pcall: s_pcall,
        lua_getfield: s_field,
        lua_setfield: s_field,
        lua_gettop: s_gettop,
        lua_settop: s_index,
        lua_pushvalue: s_index,
        lua_pushcclosure: s_pushcclosure,
        lua_tolstring: s_tolstring,
        lua_type: s_type,
        lua_pushstring: s_pushstring,
        lua_pushnumber: s_pushnumber,
        lua_pushboolean: s_pushboolean,
        lua_toboolean: s_toboolean,
        lua_settable: s_index,
        lua_createtable: s_createtable,
        lua_error: s_error,
        lual_register: s_register,
        lual_checklstring: s_checklstring,
    });
}

const MOD_TOML: &str = r#"
[manifest]
version = "1.0.0"
priority = 50

[[patches]]
[patches.pattern]
id = "marker_main"
target = "main.lua"
pattern = "local x = 1"
position = "after"
payload = "-- MOD_MARK main"

[[patches]]
[patches.pattern]
id = "marker_conf"
target = "conf.lua"
pattern = "t.window = nil"
position = "after"
payload = "-- MOD_MARK conf"

[[patches]]
[patches.pattern]
id = "marker_external"
target = "external.lua"
pattern = "send()"
position = "after"
payload = "-- MOD_MARK external"

[[patches]]
[patches.pattern]
id = "marker_other"
target = "includes/other.lua"
pattern = "local y = 2"
position = "after"
payload = "-- MOD_MARK other"
"#;

const MAIN: &str = "local x = 1\nfunction love.load()\nend\n";
const CONF: &str = "function love.conf(t)\n  t.identity = nil\n  t.window = nil\nend\n";
const EXTERNAL: &str = "function uploadScoresHelper(handle, scorez)\n  send()\nend\n";
const OTHER: &str = "local y = 2\n";

struct Rig {
    _dir: tempfile::TempDir,
    mods: PathBuf,
    qm: QuarterMaster,
    loaded: Arc<Mutex<Vec<String>>>,
}

fn rig() -> Rig {
    rig_with_plan(None)
}

/// The rig with `Mods/qm/load-order.json` written first. The report starts as init writes it.
fn rig_with_plan(plan: Option<&str>) -> Rig {
    rig_with(MOD_TOML, plan)
}

/// The rig with ModA's qm.toml `toml`.
fn rig_with(toml: &str, plan: Option<&str>) -> Rig {
    stub_lua();
    let dir = tempfile::TempDir::new().unwrap();
    let mods = dir.path().join("Mods");
    std::fs::create_dir_all(mods.join("qm")).unwrap();
    std::fs::create_dir_all(mods.join("ModA")).unwrap();
    std::fs::write(mods.join("ModA").join("qm.toml"), toml).unwrap();
    if let Some(plan) = plan {
        std::fs::write(mods.join("qm").join("load-order.json"), plan).unwrap();
    }
    let table = PatchTable::load(&mods).unwrap();
    assert!(table.load_errors.is_empty());
    let report = report_fields_of(table.prepare().report());
    let (loadbuffer, loaded) = recording_loadbuffer();
    let qm = QuarterMaster {
        mod_dir: table.mod_dir.clone(),
        is_vanilla: false,
        launch_config: LaunchConfig {
            save_identity_override: Some("qm_test".to_string()),
            ..Default::default()
        },
        loadbuffer,
        lua_vars: Arc::new(RwLock::new(table.vars.clone())),
        runtime: Arc::new(RwLock::new(Runtime::new(table))),
        dump_all: AtomicBool::new(false),
        debug: false,
        dev: false,
        patch_report: Arc::new(RwLock::new(report)),
        debug_by_file: Arc::new(RwLock::new(HashMap::new())),
        degraded: AtomicBool::new(false),
        degraded_notice_shown: AtomicBool::new(false),
        load_errors_shown: AtomicBool::new(false),
    };
    Rig {
        _dir: dir,
        mods,
        qm,
        loaded,
    }
}

/// A `loadbuffer` that records the text the game would load.
fn recording_loadbuffer() -> (&'static LoadBuffer, Arc<Mutex<Vec<String>>>) {
    let loaded = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&loaded);
    let loadbuffer: &'static LoadBuffer = Box::leak(Box::new(
        move |_s: *mut LuaState, ptr: *const u8, len: usize, _n: *const u8, _m: *const u8| {
            let bytes = unsafe { slice::from_raw_parts(ptr, len) };
            sink.lock().unwrap().push(String::from_utf8_lossy(bytes).to_string());
            0
        },
    ));
    (loadbuffer, loaded)
}

/// The rig built the way `QuarterMaster::init` builds the runtime, after `launch` (when
/// given) was written to `Mods/qm/launch-config.json`.
fn rig_from_init(launch: Option<&str>) -> Rig {
    stub_lua();
    let dir = tempfile::TempDir::new().unwrap();
    let mods = dir.path().join("Mods");
    std::fs::create_dir_all(mods.join("qm")).unwrap();
    std::fs::create_dir_all(mods.join("ModA")).unwrap();
    std::fs::write(mods.join("ModA").join("qm.toml"), MOD_TOML).unwrap();
    if let Some(launch) = launch {
        std::fs::write(mods.join("qm").join("launch-config.json"), launch).unwrap();
    }
    let (config, error) = read_launch_config(&mods);
    let (loadbuffer, loaded) = recording_loadbuffer();
    let (qm, panic) = QuarterMaster::start_runtime(mods.clone(), false, config, error, false, loadbuffer);
    assert!(panic.is_none());
    Rig {
        _dir: dir,
        mods,
        qm,
        loaded,
    }
}

fn printed() -> Vec<String> {
    PRINTED.with(|p| p.borrow().clone())
}

impl Rig {
    /// Loads one chunk through the DLL entry: the return code and the text the game got.
    fn load(&self, name: &str, buf: &str) -> (u32, Option<String>) {
        self.load_bytes(name, buf.as_bytes())
    }

    fn load_bytes(&self, name: &str, buf: &[u8]) -> (u32, Option<String>) {
        let cname = CString::new(name).unwrap();
        let before = self.loaded.lock().unwrap().len();
        let code = self.qm.load_buffer_guarded(
            std::ptr::null_mut(),
            buf.as_ptr(),
            buf.len(),
            cname.as_ptr() as *const u8,
            std::ptr::null(),
        );
        let loaded = self.loaded.lock().unwrap();
        assert!(loaded.len() <= before + 1);
        (code, loaded.get(before).cloned())
    }

    fn saved_report(&self) -> PatchReport {
        PatchReport::load_from_mod_dir(&self.qm.mod_dir)
            .unwrap()
            .expect("patch-report.json written")
    }
}

fn panic_in(name: Option<&str>) {
    PANIC_IN.with(|p| *p.borrow_mut() = name.map(str::to_string));
}

fn fallback_panics(on: bool) {
    PANIC_IN_FALLBACK.with(|p| p.set(on));
}

/// Resets the injectors even when an assertion fails (tests share threads).
struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        panic_in(None);
        fallback_panics(false);
    }
}

#[test]
fn normal_load_applies_mods_and_builtins() {
    let r = rig();
    let (code, text) = r.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    let text = text.unwrap();
    assert!(text.starts_with(STEAM_SAFETY_PAYLOAD));
    assert!(text.contains("-- MOD_MARK main"));
    assert!(!r.qm.is_degraded());
    let (_, other) = r.load("@includes/other.lua", OTHER);
    assert!(other.unwrap().contains("-- MOD_MARK other"));
}

#[test]
fn forced_panic_on_main_lua_keeps_steam_safety() {
    let _reset = Reset;
    let r = rig();
    panic_in(Some("main.lua"));
    let (code, text) = r.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    let text = text.expect("main.lua was loaded");
    let builtin = prepare::prepare_buffer("@main.lua", MAIN, &r.qm.launch_config).text;
    assert_eq!(text, builtin);
    assert!(text.contains(STEAM_SAFETY_PAYLOAD));
    assert!(text.contains(QM_RUNTIME_FOOTER_PAYLOAD));
    assert!(!text.contains("MOD_MARK"));
    assert!(r.qm.is_degraded());

    let report = r.saved_report();
    let degraded = report.dll_degraded.expect("dll_degraded is written");
    assert!(degraded.contains("injected test panic in @main.lua"), "{degraded}");
    let crashes = r.mods.join("qm").join("log").join("crashes");
    assert_eq!(std::fs::read_dir(crashes).unwrap().count(), 1);
}

#[test]
fn forced_panic_on_external_lua_keeps_defuse() {
    let _reset = Reset;
    let r = rig();
    panic_in(Some("external.lua"));
    let (code, text) = r.load("@includes/external.lua", EXTERNAL);
    assert_eq!(code, 0);
    let text = text.expect("external.lua was loaded");
    assert!(text.contains("function uploadScoresHelper(handle, scorez) return false end"));
    assert!(!text.contains("MOD_MARK"));
    assert!(r.qm.is_degraded());
}

#[test]
fn after_degrade_every_chunk_is_builtin_only() {
    let _reset = Reset;
    let r = rig();
    panic_in(Some("main.lua"));
    r.load("@main.lua", MAIN);
    assert!(r.qm.is_degraded());
    panic_in(None);

    let (code, conf) = r.load("@conf.lua", CONF);
    assert_eq!(code, 0);
    let conf = conf.unwrap();
    assert_eq!(conf, prepare::prepare_buffer("@conf.lua", CONF, &r.qm.launch_config).text);
    assert!(conf.contains("t.identity = \"qm_test\""));
    assert!(!conf.contains("MOD_MARK"));

    let (_, other) = r.load("@includes/other.lua", OTHER);
    assert_eq!(other.unwrap(), OTHER);

    let (_, main) = r.load("@main.lua", MAIN);
    let main = main.unwrap();
    assert!(main.starts_with(STEAM_SAFETY_PAYLOAD));
    assert!(!main.contains("MOD_MARK"));
    assert!(r.qm.degraded_notice_shown.load(Ordering::SeqCst));
}

#[test]
fn fallback_panic_never_loads_a_safety_chunk_raw() {
    let _reset = Reset;
    let r = rig();
    panic_in(Some("main.lua"));
    fallback_panics(true);
    let (code, text) = r.load("@main.lua", MAIN);
    assert_eq!(code, LUA_ERRSYNTAX);
    assert_eq!(text, None, "main.lua must not load without its safety edits");

    let (code, text) = r.load("@includes/external.lua", EXTERNAL);
    assert_eq!(code, LUA_ERRSYNTAX);
    assert_eq!(text, None);

    // Any other chunk loads raw.
    let (code, text) = r.load("@includes/other.lua", OTHER);
    assert_eq!(code, 0);
    assert_eq!(text.unwrap(), OTHER);
}

/// Init loads the mods and prepares them inside DllMain, under the panic guard: a panic there
/// turns mods off for the session instead of aborting the game (D34).
#[test]
fn a_panic_while_init_loads_the_mods_turns_them_off() {
    let _reset = Reset;
    let r = rig();
    let ok = load_for_init(&r.mods, false, Some("p".to_string()), None);
    assert!(ok.panic.is_none());
    assert!(ok.prepared.is_some(), "init keeps the Prepared for the report");
    assert!(!ok.table.patches.is_empty());
    assert!(ok.report.load_errors.is_empty());

    panic_in(Some(INIT_PANIC_NAME));
    let caught = load_for_init(&r.mods, false, Some("p".to_string()), Some("broken".to_string()));
    let message = caught.panic.expect("the panic is caught");
    assert!(message.contains("injected test panic in @qm-init"), "{message}");
    assert!(caught.table.patches.is_empty() && caught.prepared.is_none());
    assert!(caught.lua_vars.is_empty());
    assert_eq!(caught.report.profile_id.as_deref(), Some("p"));
    assert_eq!(caught.report.qm_version, QM_VERSION);
    assert_eq!(caught.report.launch_config_error.as_deref(), Some("broken"));
}

/// D65: a launch-config.json that does not parse runs without mods, never vanilla, and keeps
/// every builtin edit. Its `vanilla_mode` is not honoured.
#[test]
fn a_broken_launch_config_loads_no_mods_and_keeps_the_builtins() {
    PRINTED.with(|p| p.borrow_mut().clear());
    let r = rig_from_init(Some("{\"vanilla_mode\": true, \"save_identity_override\": \"mnb_x\""));
    assert!(!r.qm.is_vanilla);
    assert!(!r.qm.is_degraded());
    assert_eq!(r.qm.launch_config.save_identity_override, None);
    let error = r.qm.patch_report.read().unwrap().launch_config_error.clone();
    let error = error.expect("the report carries the error");
    assert!(error.starts_with("launch-config.json is not valid: "), "{error}");
    assert_eq!(r.saved_report().launch_config_error.as_deref(), Some(error.as_str()));
    assert!(r.qm.runtime.read().unwrap().table.patches.is_empty());

    let (code, main) = r.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    let main = main.expect("main.lua loads");
    assert!(main.starts_with(STEAM_SAFETY_PAYLOAD));
    assert!(main.ends_with(QM_RUNTIME_FOOTER_PAYLOAD));
    assert!(!main.contains("MOD_MARK"));

    let (_, external) = r.load("@includes/external.lua", EXTERNAL);
    let external = external.expect("external.lua loads");
    assert!(external.contains("function uploadScoresHelper(handle, scorez) return false end"));
    assert!(!external.contains("MOD_MARK"));

    // The game's default save identity: conf.lua is not changed.
    let (_, conf) = r.load("@conf.lua", CONF);
    assert_eq!(conf.as_deref(), Some(CONF));
    let (_, other) = r.load("@includes/other.lua", OTHER);
    assert_eq!(other.as_deref(), Some(OTHER));

    // Printed once, on the first main.lua load.
    r.load("@main.lua", MAIN);
    let shown: Vec<_> = printed()
        .into_iter()
        .filter(|l| l.contains("launch-config.json is not valid"))
        .collect();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].starts_with("QuarterMaster loaded no mods: "), "{}", shown[0]);
    assert!(shown[0].contains("default save folder"), "{}", shown[0]);
}

#[test]
fn a_missing_launch_config_uses_the_defaults() {
    PRINTED.with(|p| p.borrow_mut().clear());
    let r = rig_from_init(None);
    assert!(!r.qm.is_vanilla);
    assert!(r.qm.patch_report.read().unwrap().launch_config_error.is_none());
    assert!(r.saved_report().launch_config_error.is_none());
    let (_, main) = r.load("@main.lua", MAIN);
    let main = main.unwrap();
    assert!(main.starts_with(STEAM_SAFETY_PAYLOAD));
    assert!(main.contains("-- MOD_MARK main"));
    assert!(printed().is_empty(), "{:?}", printed());
}

#[test]
fn a_valid_launch_config_is_used() {
    let r = rig_from_init(Some(r#"{"profile_id": "p", "save_identity_override": "mnb_x"}"#));
    assert!(!r.qm.is_vanilla);
    assert_eq!(r.saved_report().profile_id.as_deref(), Some("p"));
    let (_, conf) = r.load("@conf.lua", CONF);
    assert!(conf.unwrap().contains("t.identity = \"mnb_x\""));

    let r = rig_from_init(Some(r#"{"profile_id": "p", "vanilla_mode": true}"#));
    assert!(r.qm.is_vanilla);
    let (_, main) = r.load("@main.lua", MAIN);
    assert_eq!(main.as_deref(), Some(MAIN));
}

#[test]
fn degraded_non_utf8_safety_chunk_is_not_loaded_raw() {
    let r = rig();
    r.qm.degraded.store(true, Ordering::SeqCst);
    let mut main = MAIN.as_bytes().to_vec();
    main.push(0xff);
    let (code, text) = r.load_bytes("@main.lua", &main);
    assert_eq!(code, LUA_ERRSYNTAX);
    assert_eq!(text, None, "main.lua must not load without its safety edits");

    let mut external = EXTERNAL.as_bytes().to_vec();
    external.push(0xff);
    let (code, text) = r.load_bytes("@includes/external.lua", &external);
    assert_eq!(code, LUA_ERRSYNTAX);
    assert_eq!(text, None);

    // Any other chunk loads raw, as on the normal path.
    let mut other = OTHER.as_bytes().to_vec();
    other.push(0xff);
    let (code, text) = r.load_bytes("@includes/other.lua", &other);
    assert_eq!(code, 0);
    assert_eq!(text.unwrap(), String::from_utf8_lossy(&other));
}

#[test]
fn poisoned_locks_recover() {
    let r = rig();
    std::thread::scope(|s| {
        let _ = s
            .spawn(|| {
                let _g = r.qm.debug_by_file.write().unwrap();
                panic!("poison debug_by_file");
            })
            .join();
        let _ = s
            .spawn(|| {
                let _g = r.qm.patch_report.write().unwrap();
                panic!("poison patch_report");
            })
            .join();
        let _ = s
            .spawn(|| {
                let _g = r.qm.runtime.write().unwrap();
                panic!("poison runtime");
            })
            .join();
    });
    assert!(r.qm.debug_by_file.is_poisoned());
    assert!(r.qm.patch_report.is_poisoned());
    assert!(r.qm.runtime.is_poisoned());

    let report = r
        .qm
        .build_and_save_crash_report("rust_panic", "boom at main.lua:3", "boom at main.lua:3");
    assert_eq!(report.crashing_file.as_deref(), Some("main.lua"));

    let (code, text) = r.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    let text = text.unwrap();
    let mark_line = text.lines().position(|l| l.contains("-- MOD_MARK main")).unwrap() + 1;
    assert!(!r.qm.is_degraded());
    let dbg = r.qm.debug_by_file.read().unwrap_or_else(PoisonError::into_inner);
    assert!(dbg.contains_key("main.lua"));
    drop(dbg);
    // The crash report now attributes the line to the mod.
    let msg = format!("boom at main.lua:{mark_line}");
    let report = r.qm.build_and_save_crash_report("rust_panic", &msg, "");
    assert_eq!(report.suspected_patch_id.as_deref(), Some("marker_main"));
}

#[test]
fn hard_plan_error_loads_vanilla_with_the_safety_edits() {
    let rig = rig_with_plan(Some("{\"profile_id\": \"p\""));
    assert!(rig.qm.patch_report.read().unwrap().plan_error.is_some());
    let (code, text) = rig.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    let text = text.expect("main.lua loads");
    assert!(text.contains(STEAM_SAFETY_PAYLOAD));
    assert!(!text.contains("MOD_MARK"));
    let (_, other) = rig.load("@includes/other.lua", OTHER);
    assert_eq!(other.as_deref(), Some(OTHER));
    assert!(rig.qm.load_errors_shown.load(Ordering::SeqCst));
}

#[test]
fn a_bad_plan_entry_keeps_the_rest_of_the_plan() {
    let plan = r#"{"profile_id": "p", "generated_at": "now", "enabled_mods": ["moda"],
        "disabled_patches": ["marker_other", 5]}"#;
    let rig = rig_with_plan(Some(plan));
    let report = rig.qm.patch_report.read().unwrap().clone();
    assert!(report.plan_error.is_none());
    assert_eq!(report.plan_warnings.len(), 1, "{:?}", report.plan_warnings);
    let (_, main) = rig.load("@main.lua", MAIN);
    assert!(main.unwrap().contains("-- MOD_MARK main"));
    let (_, other) = rig.load("@includes/other.lua", OTHER);
    assert!(!other.unwrap_or_default().contains("MOD_MARK"));
}

#[test]
fn glue_waits_for_its_base_mod_in_the_dll() {
    // One copy replaces ModA's patch (on), one replaces a patch of ModB (off).
    let plan = r#"{"profile_id": "p", "generated_at": "now", "enabled_mods": ["moda"],
        "disabled_patches": ["marker_other", "modb:main_lua:1"],
        "glue_patches": [
          {"id": "glue:on", "name": "On copy", "replaces_patch_id": "marker_other",
           "base_mod_id": "moda", "priority": 50, "patch": {"pattern": {"target": "includes/other.lua",
           "pattern": "local y = 2", "position": "before", "payload": "-- GLUE on"}}},
          {"id": "glue:off", "name": "Off copy", "replaces_patch_id": "modb:main_lua:1",
           "priority": 50, "patch": {"pattern": {"target": "main.lua",
           "pattern": "local x = 1", "position": "before", "payload": "-- GLUE off"}}}]}"#;
    let rig = rig_with_plan(Some(plan));
    let report = rig.qm.patch_report.read().unwrap().clone();
    assert_eq!(report.fix_bindings.len(), 1, "{:?}", report.fix_bindings);
    let b = &report.fix_bindings[0];
    assert_eq!(b.key, "glue:glue:off");
    assert_eq!(
        b.status,
        crate::handshake::BindingStatus::Waiting {
            mods: vec!["modb".to_string()]
        }
    );
    let (_, main) = rig.load("@main.lua", MAIN);
    let main = main.unwrap();
    assert!(main.contains("-- MOD_MARK main"));
    assert!(!main.contains("GLUE off"), "the copy waits for its mod");
    let (_, other) = rig.load("@includes/other.lua", OTHER);
    let other = other.unwrap();
    assert!(other.contains("-- GLUE on"), "the copy of an enabled mod runs");
    assert!(!other.contains("MOD_MARK other"), "and replaces its base");
    assert!(rig.qm.load_errors_shown.load(Ordering::SeqCst));
}

#[test]
fn prepared_is_built_once_at_the_first_buffer() {
    let r = rig();
    assert!(r.qm.runtime.read().unwrap().prepared.get().is_none(), "not built at init");
    r.load("@main.lua", MAIN);
    let first = r.qm.runtime.read().unwrap().prepared.get().map(|p| p as *const Prepared);
    assert!(first.is_some());
    r.load("@includes/other.lua", OTHER);
    let again = r.qm.runtime.read().unwrap().prepared.get().map(|p| p as *const Prepared);
    assert_eq!(first, again, "the same Prepared serves every buffer");
}

#[test]
fn reload_replaces_prepared() {
    use crate::handshake::StackMode;
    let plan = |mode: &str| {
        format!(
            r#"{{"profile_id": "p", "generated_at": "now", "enabled_mods": ["moda"], "stacking": "{mode}"}}"#
        )
    };
    let r = rig_with_plan(Some(&plan("legacy")));
    assert_eq!(r.qm.runtime.read().unwrap().table.stack_mode, StackMode::Legacy);
    let (_, main) = r.load("@main.lua", MAIN);
    assert!(main.unwrap().contains("-- MOD_MARK main"));
    let first = r.qm.runtime.read().unwrap().prepared.get().map(Prepared::stamp).unwrap();

    // Only the stack mode changes: same mods, same patch ids.
    std::fs::write(r.mods.join("qm").join("load-order.json"), plan("dependencies")).unwrap();
    r.qm.reload().unwrap();
    // The reload keeps the Prepared its load built (built once, as `init` does): the new
    // table's, not the old one's.
    let second = {
        let rt = r.qm.runtime.read().unwrap();
        assert_eq!(rt.table.stack_mode, StackMode::Dependencies);
        let second = rt.prepared.get().map(Prepared::stamp);
        assert_eq!(second, Some(rt.table.stamp()), "built with the new table");
        second.unwrap()
    };
    assert_ne!(first, second);
    let (_, other) = r.load("@includes/other.lua", OTHER);
    assert!(other.unwrap().contains("-- MOD_MARK other"));
    let rt = r.qm.runtime.read().unwrap();
    assert_eq!(rt.prepared.get().map(Prepared::stamp), Some(second), "and used as it is");
}

#[test]
fn failed_reload_keeps_the_old_runtime() {
    let r = rig();
    r.load("@main.lua", MAIN);
    let before = r.qm.runtime.read().unwrap().prepared.get().map(Prepared::stamp);
    std::fs::write(r.mods.join("qm").join("load-order.json"), "{").unwrap();
    assert!(r.qm.reload().is_err());
    let rt = r.qm.runtime.read().unwrap();
    assert_eq!(rt.prepared.get().map(Prepared::stamp), before);
    assert!(!rt.table.patches.is_empty());
}

#[test]
fn a_match_on_startup_code_is_counted_in_the_report() {
    let r = rig();
    let hook = r#"
[manifest]
version = "1.0.0"
priority = 60

[[patches]]
[patches.pattern]
id = "hook_upload"
target = "external.lua"
pattern = "function uploadScoresHelper*"
position = "before"
payload = "-- HOOK upload"
"#;
    std::fs::create_dir_all(r.mods.join("ModB")).unwrap();
    std::fs::write(r.mods.join("ModB").join("qm.toml"), hook).unwrap();
    r.qm.reload().unwrap();

    let (code, text) = r.load("@includes/external.lua", EXTERNAL);
    assert_eq!(code, 0);
    let text = text.unwrap();
    assert!(text.contains("-- HOOK upload\nfunction uploadScoresHelper(handle, scorez) return false end"));
    assert!(text.contains("-- MOD_MARK external"));

    let report = r.saved_report();
    assert_eq!(report.builtin_matches, 1);
    let hook = report.records.iter().find(|x| x.patch_id == "hook_upload").unwrap();
    assert!(prepare::is_builtin_match(hook), "{:?}", hook.warnings);
    let mark = report.records.iter().find(|x| x.patch_id == "marker_external").unwrap();
    assert!(!prepare::is_builtin_match(mark));
}

/// D42 in the DLL: an `at` on the defused upload line would undo the defuse, so it is refused
/// and the defuse stays; the record says why and counts as a builtin match.
#[test]
fn an_at_on_the_defused_line_is_refused() {
    let r = rig();
    let undo = r#"
[manifest]
version = "1.0.0"
priority = 60

[[patches]]
[patches.pattern]
id = "undo_defuse"
target = "external.lua"
pattern = "function uploadScoresHelper*"
position = "at"
payload = "function uploadScoresHelper(handle, scorez)"
"#;
    std::fs::create_dir_all(r.mods.join("ModB")).unwrap();
    std::fs::write(r.mods.join("ModB").join("qm.toml"), undo).unwrap();
    r.qm.reload().unwrap();

    let (code, text) = r.load("@includes/external.lua", EXTERNAL);
    assert_eq!(code, 0);
    let text = text.unwrap();
    assert!(text.contains("function uploadScoresHelper(handle, scorez) return false end"));
    assert!(text.contains("-- MOD_MARK external"));

    let report = r.saved_report();
    assert_eq!(report.builtin_matches, 1);
    let undo = report.records.iter().find(|x| x.patch_id == "undo_defuse").unwrap();
    assert_eq!(undo.status, PatchOutcomeStatus::ZeroMatches);
    assert!(
        undo.warnings.iter().any(|w| w.starts_with(prepare::BUILTIN_MATCH_SKIPPED)),
        "{:?}",
        undo.warnings
    );
    assert!(prepare::is_builtin_match(undo));
}

#[test]
fn crash_lines_are_attributed_by_the_record_of_each_line() {
    use crate::patch::linemap::NO_RECORD;
    let owner = |id: &str| RecordOwner {
        mod_id: format!("mod_{id}"),
        patch_id: id.to_string(),
        patch_name: format!("name {id}"),
        ..RecordOwner::default()
    };
    let fd = FileDebug {
        text: "a\nx\nY\nb\n".to_string(),
        line_records: vec![NO_RECORD, 0, 1, NO_RECORD],
        owners: vec![owner("p0"), owner("p1")],
    };
    assert_eq!(fd.owner_of_line(0), None);
    assert_eq!(fd.owner_of_line(1), None);
    assert_eq!(fd.owner_of_line(2).map(|o| o.patch_id.as_str()), Some("p0"));
    assert_eq!(fd.owner_of_line(3).map(|o| o.patch_id.as_str()), Some("p1"));
    assert_eq!(fd.owner_of_line(4), None);
    assert_eq!(fd.owner_of_line(99), None);

    // Through the DLL: the crash line of a patched chunk names the patch that wrote it.
    let r = rig();
    let (code, text) = r.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    let text = text.unwrap();
    let first_vanilla = MAIN.lines().next().unwrap();
    let vanilla_line = text.lines().position(|l| l == first_vanilla).unwrap() + 1;
    let report = r.qm.build_and_save_crash_report("rust_panic", &format!("boom at main.lua:{vanilla_line}"), "");
    assert_eq!(report.suspected_patch_id, None, "a game line has no patch");
}

/// D58: a crash in a `Base` copy's line names the copy and its fix next to its base mod, so
/// the suspect is the fix, not the mod.
#[test]
fn a_crash_in_a_base_copy_names_the_copy_and_its_fix() {
    let toml = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"base_a\"\ntarget = \"main.lua\"\npattern = \"local x = 99\"\nposition = \"after\"\npayload = \"-- gone\"\n";
    let plan = serde_json::json!({
        "profile_id": "p", "generated_at": "now", "enabled_mods": ["moda"],
        "glue_patches": [{
            "id": "glue:base_a", "name": "Adjusted copy of base_a", "replaces_patch_id": "base_a",
            "priority": 50, "base_mod_id": "moda", "run_as": "base", "exact": true,
            "patch": {"pattern": {"target": "main.lua", "pattern": "local x = 1",
                "position": "after", "payload": "boom()"}},
        }],
        "fix_of": {"glue:glue:base_a": "fx_copy"},
    });
    let r = rig_with(toml, Some(&plan.to_string()));
    let (code, text) = r.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    let text = text.unwrap();
    let line = text.lines().position(|l| l.trim() == "boom()").unwrap() + 1;
    let report = r.qm.build_and_save_crash_report("lua_runtime", &format!("main.lua:{line}: boom"), "");
    assert_eq!(report.suspected_mod_id.as_deref(), Some("moda"));
    assert_eq!(report.suspected_patch_id.as_deref(), Some("glue:base_a"));
    assert_eq!(report.suspected_fix_ids, ["fx_copy"]);
    assert!(report.suspected_adjusted_copy);
    // The crash file carries both.
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["suspected_adjusted_copy"], true);
}

/// R1 row 3 in the DLL: a profile rule against a rung is ignored, and patch-report.json says so.
#[test]
fn an_ignored_order_rule_reaches_the_patch_report() {
    let toml = r#"
[manifest]
version = "1.0.0"
priority = 50

[[patches]]
[patches.pattern]
id = "b"
target = "main.lua"
pattern = "local x = 1"
position = "after"
payload = "-- RUNG_B"

[[patches]]
[patches.pattern]
id = "a"
target = "main.lua"
pattern = "local x = 1"
position = "after"
payload = "-- RUNG_A"
[[patches.pattern.anchors]]
within = "moda:b"
pattern = "-- RUNG_B"
exact = true
"#;
    let plan = r#"{"profile_id": "p", "generated_at": "now", "enabled_mods": ["moda"],
        "patch_order": [{"patch": "moda:a", "before": ["moda:b"]}]}"#;
    let rig = rig_with(toml, Some(plan));
    let want = "profile rule moda:a before moda:b ignored: moda:a is anchored on moda:b";
    let report = rig.qm.patch_report.read().unwrap().clone();
    assert_eq!(report.overridden_rules, [want]);
    let (_, main) = rig.load("@main.lua", MAIN);
    let main = main.unwrap();
    let b = main.find("-- RUNG_B").unwrap();
    assert!(main.find("-- RUNG_A").unwrap() > b, "A goes below B's line");
    assert_eq!(rig.saved_report().overridden_rules, [want]);
    let rec = rig.saved_report().records.into_iter().find(|r| r.patch_id == "a").unwrap();
    assert_eq!(rec.anchoring.unwrap().sites[0].anchored_on.as_deref(), Some("moda:b"));
}

/// A compat override in the DLL: the game gets the change, patch-report.json says so and the
/// record names the mod that made it (§8.1).
#[test]
fn a_compat_override_reaches_the_game_and_the_patch_report() {
    let toml = format!(
        "{MOD_TOML}\n[[overrides]]\npatch = \"moda:marker_main\"\ndisable = true\n\n[[overrides]]\npatch = \"gone:x\"\ndisable = true\n"
    );
    let rig = rig_with(&toml, None);
    let (_, main) = rig.load("@main.lua", MAIN);
    assert!(!main.unwrap().contains("-- MOD_MARK main"));
    let (_, conf) = rig.load("@conf.lua", CONF);
    assert!(conf.unwrap().contains("-- MOD_MARK conf"));
    let report = rig.saved_report();
    let got: Vec<(&str, &str)> = report
        .overrides
        .iter()
        .map(|o| (o.target.as_str(), o.status.as_str()))
        .collect();
    assert_eq!(got, [("moda:marker_main", "applied"), ("gone:x", "rejected_not_required")]);
    let rec = report.records.iter().find(|r| r.patch_id == "marker_main").unwrap();
    assert_eq!(rec.status, PatchOutcomeStatus::SkippedDisabled);
    assert_eq!(rec.changed_by, ["moda"]);
}

/// Through the DLL entry, every name of the game's main.lua loads the same text, with the
/// Steam block, the footer and the mod patch, and adds the same records (D44).
#[test]
fn every_main_lua_name_loads_with_the_builtins() {
    let names = [
        "@main.lua",
        "main.lua",
        "@./main.lua",
        ".\\main.lua",
        "@C:/Games/MnB/main.lua",
    ];
    let mut want = None;
    for name in names {
        let r = rig();
        let (code, text) = r.load(name, MAIN);
        assert_eq!(code, 0, "{name}");
        let text = text.unwrap();
        assert!(text.starts_with(STEAM_SAFETY_PAYLOAD), "{name}");
        assert!(text.ends_with(QM_RUNTIME_FOOTER_PAYLOAD), "{name}");
        assert!(text.contains("-- MOD_MARK main"), "{name}");
        assert!(r.qm.load_errors_shown.load(Ordering::SeqCst), "{name}: main.lua setup ran");
        let records: Vec<(String, PatchOutcomeStatus)> = r
            .saved_report()
            .records
            .iter()
            .map(|x| (x.patch_id.clone(), x.status))
            .collect();
        let got = (text, records);
        match &want {
            None => want = Some(got),
            Some(w) => assert_eq!(&got, w, "{name}"),
        }
    }
}

/// Chunks that are not the game's main.lua get no Steam block or footer; chunks that are not
/// game files load as they are (D44).
#[test]
fn other_chunk_names_load_without_the_builtins() {
    let r = rig();
    let code = "local x = 1\n-- main.lua";
    for (name, buf) in [
        ("=stdin", MAIN),
        ("=(load)", MAIN),
        (code, code),
        ("=[love \"main.lua\"]", MAIN),
    ] {
        let (rc, text) = r.load(name, buf);
        assert_eq!(rc, 0, "{name:?}");
        assert_eq!(text.as_deref(), Some(buf), "{name:?}");
    }
    assert!(!r.qm.load_errors_shown.load(Ordering::SeqCst), "no main.lua setup ran");
    assert!(r.qm.patch_report.read().unwrap().records.is_empty());
    assert!(r.qm.debug_by_file.read().unwrap().is_empty());
    // The love form leaves no dump.
    assert!(!r.mods.join("qm").join("game-dump").join("love").exists());

    // Game files that are not the root main.lua: no builtins, and the main.lua setup does
    // not run for them.
    for name in ["@domain.lua", "@x/main.lua"] {
        let (rc, text) = r.load(name, MAIN);
        assert_eq!(rc, 0, "{name}");
        let text = text.unwrap();
        assert!(!text.contains(STEAM_SAFETY_PAYLOAD), "{name}");
        assert!(!text.contains(QM_RUNTIME_FOOTER_PAYLOAD), "{name}");
    }
    assert!(!r.qm.load_errors_shown.load(Ordering::SeqCst));
}

/// Only the root main.lua and includes/external.lua never load without their safety edits.
#[test]
fn safety_chunks_are_root_files() {
    for name in ["@main.lua", ".\\main.lua", "@C:/MnB/main.lua", "@includes/external.lua"] {
        assert!(is_safety_chunk(name.as_bytes()), "{name}");
    }
    for name in ["@domain.lua", "@x/main.lua", "@external.lua", "=[love \"main.lua\"]", "=stdin"] {
        assert!(!is_safety_chunk(name.as_bytes()), "{name}");
    }
}

/// A crash in a module chunk names the mod that owns the module, not the module name (§6.2).
#[test]
fn a_module_crash_names_the_owning_mod() {
    let r = rig();
    let modb = r.mods.join("ModB");
    std::fs::create_dir_all(&modb).unwrap();
    std::fs::write(modb.join("ser.lua"), "return {}\n").unwrap();
    let toml = r#"
[manifest]
version = "1.0.0"
priority = 60

[[patches]]
[patches.module]
id = "modb_ser"
source = "ser.lua"
before = "main.lua"
name = "includes/ser"
"#;
    std::fs::write(modb.join("qm.toml"), toml).unwrap();
    let modc = r.mods.join("ModC");
    std::fs::create_dir_all(&modc).unwrap();
    std::fs::write(modc.join("other_ser.lua"), "return {}\n").unwrap();
    let toml = r#"
[manifest]
version = "1.0.0"
priority = 70

[[patches]]
[patches.module]
id = "modc_ser"
source = "other_ser.lua"
before = "main.lua"
name = "includes/ser"
"#;
    std::fs::write(modc.join("qm.toml"), toml).unwrap();
    r.qm.reload().unwrap();
    assert!(r.qm.runtime.read().unwrap().table.load_errors.is_empty());

    let trace = "stack traceback:\n\t[qm includes/ser \"ser.lua\"]:3: in function 'f'";
    let report = r.qm.build_and_save_crash_report("lua_runtime", "attempt to index nil", trace);
    assert_eq!(report.suspected_mod_id.as_deref(), Some("modb"));
    assert_eq!(report.suspected_patch_id.as_deref(), Some("modb_ser"));
    assert_eq!(
        report.suspected_patch_name.as_deref(),
        Some("Module includes/ser (ser.lua)")
    );

    // The source picks between two modules of one name.
    let msg = "[qm includes/ser \"other_ser.lua\"]:1: boom";
    let report = r.qm.build_and_save_crash_report("lua_runtime", msg, "");
    assert_eq!(report.suspected_mod_id.as_deref(), Some("modc"));

    // A module no loaded mod has names no mod.
    let msg = "[qm gone/mod \"x.lua\"]:1: boom";
    let report = r.qm.build_and_save_crash_report("lua_runtime", msg, "");
    assert_eq!(report.suspected_mod_id, None);
    assert_eq!(report.suspected_patch_name.as_deref(), Some("Module gone/mod (x.lua)"));

    // Also while this thread reads the runtime (a crash report from inside a load).
    let _held = r.qm.runtime.read().unwrap();
    let msg = "[qm includes/ser \"ser.lua\"]:1: boom";
    let report = r.qm.build_and_save_crash_report("lua_syntax", msg, msg);
    assert_eq!(report.suspected_mod_id.as_deref(), Some("modb"));
}

#[test]
fn run_id_is_copied_into_the_patch_report_and_crash_reports() {
    let r = rig_from_init(Some(r#"{"profile_id": "p", "run_id": "run-1"}"#));
    let report = r.saved_report();
    assert_eq!(report.run_id.as_deref(), Some("run-1"));
    assert!(!report.clean_exit);
    let (code, _) = r.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    assert_eq!(r.saved_report().run_id.as_deref(), Some("run-1"), "kept by later saves");

    let crash = r.qm.build_and_save_crash_report("lua_runtime", "main.lua:1: boom", "trace");
    assert_eq!(crash.run_id.as_deref(), Some("run-1"));
    let crashes = r.mods.join("qm").join("log").join("crashes");
    let file = std::fs::read_dir(crashes).unwrap().next().unwrap().unwrap().path();
    let saved: CrashReport = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    assert_eq!(saved.run_id.as_deref(), Some("run-1"));

    // Without a run id (an older launcher) neither file has one.
    let r = rig_from_init(None);
    assert_eq!(r.saved_report().run_id, None);
    assert_eq!(r.qm.build_and_save_crash_report("lua_runtime", "x", "x").run_id, None);
}

#[test]
fn record_asset_winners_keeps_the_runs_report() {
    let r = rig_from_init(Some(r#"{"run_id": "run-3"}"#));
    let (code, _) = r.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    let before = r.saved_report();
    assert!(before.asset_winners.is_empty());

    r.qm.record_asset_winners(
        "sounds/garand1\taxissw\nnot a winner line\n\tno slot\nsprites/allied_idle\tfaction:eastern_front\n",
    )
    .unwrap();
    let report = r.saved_report();
    let got: Vec<_> = report
        .asset_winners
        .iter()
        .map(|w| (w.slot.as_str(), w.source.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("sounds/garand1", "axissw"),
            ("sprites/allied_idle", "faction:eastern_front")
        ]
    );
    assert_eq!(report.records.len(), before.records.len(), "the run's records are kept");
    // A later call replaces the list.
    r.qm.record_asset_winners("").unwrap();
    assert!(r.saved_report().asset_winners.is_empty());
}

#[test]
fn record_clean_exit_marks_the_report_atomically() {
    let r = rig_from_init(Some(r#"{"run_id": "run-2"}"#));
    let (code, _) = r.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    let qm_dir = r.mods.join("qm");
    let tmp = qm_dir.join("patch-report.json.tmp");

    // With the temp path blocked the save fails, and the old report stays whole.
    std::fs::create_dir_all(&tmp).unwrap();
    let err = r.qm.record_clean_exit().unwrap_err();
    assert!(err.contains("patch-report.json.tmp"), "{err}");
    let kept = r.saved_report();
    assert!(!kept.clean_exit);
    assert!(!kept.records.is_empty());

    std::fs::remove_dir(&tmp).unwrap();
    r.qm.record_clean_exit().unwrap();
    assert!(!tmp.exists(), "the temp file was renamed");
    let report = r.saved_report();
    assert!(report.clean_exit);
    assert_eq!(report.run_id.as_deref(), Some("run-2"));
    assert_eq!(report.records.len(), kept.records.len(), "the run's records are kept");
}

#[test]
fn record_diagnostics_keeps_the_trace_and_the_hook_callers() {
    let r = rig_from_init(Some(r#"{"run_id": "run-4"}"#));
    let (code, _) = r.load("@main.lua", MAIN);
    assert_eq!(code, 0);
    let text = [
        "G\tloadModSettings\tfunction\tfunction: 0x01\t@main.lua\t3",
        "G\tcounter\tnumber\t5\t@other.lua\t9",
        "not a line",
        "H\tonDeath\t3",
        "C\tonDeath\t@main.lua\t2\t2",
        "C\tonDeath\t=[qm nomodule \"x.lua\"]\t7\t1",
        "C\tunknown\t@main.lua\t2\t1",
        "H\tmissing\t-1",
    ]
    .join("\n");
    r.qm.record_diagnostics(&text).unwrap();
    let report = r.saved_report();
    assert_eq!(report.run_id.as_deref(), Some("run-4"));
    assert_eq!(report.globals_trace.len(), 2);
    let first = &report.globals_trace[0];
    assert_eq!((first.name.as_str(), first.seq, first.line), ("loadModSettings", 1, 3));
    assert_eq!(first.source, "@main.lua");
    assert_eq!(first.value_type, "function");
    // The game's own line: no patch.
    assert_eq!(first.patch_id, "");
    assert_eq!(report.globals_trace[1].identity, "5");
    assert_eq!(report.hook_calls.len(), 2);
    let on_death = &report.hook_calls[0];
    assert_eq!((on_death.name.as_str(), on_death.total, on_death.callers.len()), ("onDeath", 3, 2));
    assert_eq!(on_death.callers[0].calls, 2);
    assert_eq!(on_death.callers[1].patch_id, None);
    assert!(report.hook_calls[1].missing && !on_death.missing, "a function that was not found");
    // A later call replaces the result.
    r.qm.record_diagnostics("").unwrap();
    assert!(r.saved_report().globals_trace.is_empty());
}
