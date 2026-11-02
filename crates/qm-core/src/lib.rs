#![allow(non_upper_case_globals)]

use core::slice;
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{c_int, CStr};
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Instant;

use log::*;

use getargs::{Arg, Options};
use itertools::Itertools;
use patch::{ModulePatch, Patch};
use regex_lite::Regex;

use sys::{check_lua_string, LuaFunc, LuaLib, LuaState, LuaStateTrait, LUA};

use crate::dump::{write_dump, PatchDebug};
use crate::handshake::{
    CrashReport, CrashSnippetLine, LaunchConfig, PatchOutcomeStatus, PatchReport,
};
use crate::patch::table::PatchTable;
use crate::patch::Target;

pub mod chunk_vec_cursor;
pub mod dump;
pub mod handshake;
pub mod log;
pub mod patch;
pub mod sys;

pub const QM_VERSION: &str = env!("CARGO_PKG_VERSION");

pub static RUNTIME: OnceLock<QuarterMaster> = OnceLock::new();

use crate::patch::loader::STEAM_SAFETY_PAYLOAD;

const QM_RUNTIME_FOOTER_PAYLOAD: &str = r#"
-- Quartermaster: Crash Attribution, Save Fingerprinting & Hot-Reload Hooks
do
  local ok_qm, qm_mod = pcall(require, "qm")
  if ok_qm and qm_mod and love then
    local orig_errhand = love.errorhandler or love.errhand
    local function qm_crash_wrapper(msg)
      local trace = debug and debug.traceback and debug.traceback(tostring(msg), 2) or tostring(msg)
      local ok_attr, attribution = pcall(qm_mod.record_lua_crash, tostring(msg), tostring(trace))
      local enriched_msg = tostring(msg)
      if ok_attr and type(attribution) == "string" and #attribution > 0 then
        enriched_msg = enriched_msg .. "\n\n[QuarterMaster Crash Attribution]\n" .. attribution
      end
      if type(orig_errhand) == "function" then
        return orig_errhand(enriched_msg)
      end
    end
    love.errorhandler = qm_crash_wrapper
    love.errhand = qm_crash_wrapper
  end
end
"#;

type LoadBuffer =
    dyn Fn(*mut LuaState, *const u8, usize, *const u8, *const u8) -> u32 + Send + Sync + 'static;

unsafe extern "C" fn reload_patches(state: *mut LuaState) -> c_int {
    let qm = RUNTIME.get().unwrap();
    let result = PatchTable::load(&qm.mod_dir);
    let new_table = match result {
        Ok(t) => t,
        Err(e) => {
            state.push(false);
            state.push(format!("{:?}", e));
            return 2;
        }
    };
    let binding = Arc::clone(&qm.patch_table);
    let mut patch_table = binding.write().unwrap();
    *patch_table = new_table;
    state.push(true);
    1
}

unsafe extern "C" fn getvar(state: *mut LuaState) -> c_int {
    let key = check_lua_string(state, 1);
    let qm = RUNTIME.get().unwrap();
    let vars = qm.lua_vars.read().unwrap();
    let val = vars.get(&key);
    if let Some(val) = val {
        state.push(val);
        return 1;
    }
    0
}

unsafe extern "C" fn setvar(state: *mut LuaState) -> c_int {
    let key = check_lua_string(state, 1);
    let val = check_lua_string(state, 2);
    let qm = RUNTIME.get().unwrap();
    let mut vars = qm.lua_vars.write().unwrap();
    vars.insert(key, val);
    0
}

unsafe extern "C" fn removevar(state: *mut LuaState) -> c_int {
    let key = check_lua_string(state, 1);
    let qm = RUNTIME.get().unwrap();
    let mut vars = qm.lua_vars.write().unwrap();
    let val = vars.remove(&key);
    if let Some(val) = val {
        state.push(val);
        return 1;
    }
    0
}

pub unsafe extern "C" fn dump_vanilla(state: *mut LuaState) -> c_int {
    let qm = match RUNTIME.get() {
        Some(q) => q,
        None => {
            state.push(false);
            state.push("QuarterMaster runtime not initialized");
            return 2;
        }
    };

    let target_dir = sys::opt_lua_string(state, 1);
    let filter = sys::opt_lua_string(state, 2);

    let target_path = target_dir.as_deref().map(Path::new);
    let filter_str = filter.as_deref();

    match dump::dump_vanilla_files(&qm.mod_dir, target_path, filter_str) {
        Ok((count, path)) => {
            state.push(true);
            state.push(count as isize);
            state.push(path.to_string_lossy().to_string());
            3
        }
        Err(e) => {
            state.push(false);
            state.push(e);
            2
        }
    }
}

pub unsafe extern "C" fn set_dump_all(state: *mut LuaState) -> c_int {
    let enabled = if sys::lua_gettop(state) >= 1 {
        sys::lua_toboolean(state, 1) != 0
    } else {
        true
    };
    if let Some(qm) = RUNTIME.get() {
        qm.dump_all
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }
    state.push(true);
    1
}

pub unsafe extern "C" fn is_dump_all(state: *mut LuaState) -> c_int {
    if let Some(qm) = RUNTIME.get() {
        state.push(qm.dump_all.load(std::sync::atomic::Ordering::Relaxed));
        1
    } else {
        state.push(false);
        1
    }
}

pub unsafe extern "C" fn record_lua_crash(state: *mut LuaState) -> c_int {
    let msg = sys::opt_lua_string(state, 1).unwrap_or_else(|| "Unknown Lua error".to_string());
    let trace = sys::opt_lua_string(state, 2).unwrap_or_else(|| msg.clone());

    if let Some(qm) = RUNTIME.get() {
        let report = qm.build_and_save_crash_report("lua_runtime", &msg, &trace);
        let summary = match (&report.suspected_mod_id, &report.suspected_patch_name) {
            (Some(mod_id), Some(patch_name)) => {
                format!(
                    "Suspected Mod: {} | Patch: {} ({}:{})",
                    mod_id,
                    patch_name,
                    report.crashing_file.as_deref().unwrap_or("unknown"),
                    report.crashing_line.unwrap_or(0)
                )
            }
            (Some(mod_id), None) => format!("Suspected Mod: {}", mod_id),
            _ => format!(
                "Location: {}:{} (no mod patch directly on this line)",
                report.crashing_file.as_deref().unwrap_or("unknown"),
                report.crashing_line.unwrap_or(0)
            ),
        };
        state.push(summary);
        return 1;
    }

    state.push("");
    1
}

pub struct QuarterMaster {
    pub mod_dir: PathBuf,
    pub is_vanilla: bool,
    pub launch_config: LaunchConfig,
    loadbuffer: &'static LoadBuffer,
    patch_table: Arc<RwLock<PatchTable>>,
    pub dump_all: std::sync::atomic::AtomicBool,
    lua_vars: Arc<RwLock<HashMap<String, String>>>,
    patch_report: Arc<RwLock<PatchReport>>,
    debug_by_file: Arc<RwLock<HashMap<String, (String, PatchDebug)>>>,
}

pub type QM = QuarterMaster;

impl QuarterMaster {
    /// Locate the active `Mods` directory prior to full initialization.
    pub fn resolve_default_mod_dir() -> PathBuf {
        if let Some(env_path) = env::var_os("QM_MOD_DIR") {
            return PathBuf::from(env_path);
        }
        let cur_exe =
            env::current_exe().expect("Failed to get the path of the current executable.");
        let exe_dir = cur_exe.parent().unwrap_or(Path::new("."));
        let local_mods = exe_dir.join("Mods");

        if local_mods.is_dir() {
            local_mods
        } else {
            let game_name = cur_exe
                .file_stem()
                .expect("Failed to get file_stem component of current executable path.")
                .to_string_lossy()
                .replace('.', "_");
            dirs::data_dir().unwrap().join(game_name).join("Mods")
        }
    }

    /// Initialize the QuarterMaster patch runtime.
    pub fn init(loadbuffer: &'static LoadBuffer, lualib: LuaLib, dump_all: bool) -> &'static Self {
        assert!(RUNTIME.get().is_none());

        LUA.set(lualib)
            .unwrap_or_else(|_| panic!("LUA static var has already been set."));

        let start = Instant::now();

        let args = std::env::args().skip(1).collect_vec();
        let mut opts = Options::new(args.iter().map(String::as_str));

        let mut mod_dir = Self::resolve_default_mod_dir();
        let mut is_vanilla = false;

        while let Some(opt) = opts.next_arg().expect("Failed to parse argument.") {
            match opt {
                Arg::Long("mod-dir") => {
                    mod_dir = opts.value().map(PathBuf::from).unwrap_or(mod_dir)
                }
                Arg::Long("vanilla") => is_vanilla = true,
                _ => (),
            }
        }

        let launch_config = LaunchConfig::load_from_mod_dir(&mod_dir).unwrap_or_default();
        if launch_config.vanilla_mode {
            is_vanilla = true;
        }
        let effective_dump_all = dump_all || launch_config.dump_all;

        let log_dir = mod_dir.join("qm").join("log");

        log::init(&log_dir).unwrap_or_else(|e| panic!("Failed to initialize logger: {e:?}"));

        info!("========================================================");
        info!(
            "QuarterMaster (QM) v{} — Mod Loader for Mud and Blood",
            QM_VERSION
        );
        info!("Active Mod Directory: {}", mod_dir.display());
        if let Some(ref prof) = launch_config.profile_name {
            info!("Active Profile: {}", prof);
        }
        info!("========================================================");

        if is_vanilla {
            info!("Vanilla mode enabled, ignoring patches.");
        }

        let (patch_table, lua_vars) = if !is_vanilla {
            let patch_table = PatchTable::load(&mod_dir)
                .unwrap_or_else(|e| panic!("Failed to load patches from mod directory: {e:?}"));
            let lua_vars = patch_table.vars.clone();
            (patch_table, lua_vars)
        } else {
            (PatchTable::default(), HashMap::new())
        };

        let initial_report = PatchReport {
            qm_version: QM_VERSION.to_string(),
            timestamp: jiff::Zoned::now().to_string(),
            profile_id: launch_config.profile_id.clone(),
            ..Default::default()
        };
        let _ = initial_report.save_to_mod_dir(&mod_dir);

        let qm = QuarterMaster {
            mod_dir,
            is_vanilla,
            launch_config,
            loadbuffer,
            patch_table: Arc::new(RwLock::new(patch_table)),
            dump_all: std::sync::atomic::AtomicBool::new(effective_dump_all),
            lua_vars: Arc::new(RwLock::new(lua_vars)),
            patch_report: Arc::new(RwLock::new(initial_report)),
            debug_by_file: Arc::new(RwLock::new(HashMap::new())),
        };

        let qm = RUNTIME.get_or_init(|| qm);

        info!(
            "QuarterMaster initialized in {:.2}ms",
            start.elapsed().as_secs_f64() * 1000.0
        );

        qm
    }

    /// Build an attributed `CrashReport` from an error message and traceback, and write it to `Mods/qm/log/crashes/`.
    pub fn build_and_save_crash_report(
        &self,
        crash_kind: &str,
        error_message: &str,
        traceback: &str,
    ) -> CrashReport {
        let now = jiff::Zoned::now();
        let ts_slug = now.strftime("%Y-%m-%d_%H-%M-%S").to_string();
        let crash_id = format!("crash-{ts_slug}");

        // Parse the first `file.lua:line:` occurrence from error_message or traceback
        let loc_re = Regex::new(r#"([A-Za-z0-9_./\\-]+\.lua):(\d+)"#).unwrap();
        let (crashing_file, crashing_line) = loc_re
            .captures(error_message)
            .or_else(|| loc_re.captures(traceback))
            .map(|c| {
                (
                    Some(c[1].trim_start_matches('@').to_string()),
                    c[2].parse::<usize>().ok(),
                )
            })
            .unwrap_or((None, None));

        let mut suspected_mod_id = None;
        let mut suspected_patch_id = None;
        let mut suspected_patch_name = None;
        let mut code_snippet = Vec::new();

        if let (Some(ref file_key), Some(line_no)) = (&crashing_file, crashing_line) {
            let dbg_map = self.debug_by_file.read().unwrap();
            let found = dbg_map.iter().find(|(k, _)| {
                *k == file_key
                    || k.ends_with(file_key.as_str())
                    || file_key.ends_with(k.as_str())
            });

            if let Some((_, (patched_src, patch_dbg))) = found {
                if let Some(entry) = patch_dbg.find_entry_at_line(line_no) {
                    suspected_mod_id = entry.patch_source.mod_id.clone();
                    suspected_patch_id = entry.patch_source.patch_id.clone();
                    suspected_patch_name = entry.patch_source.patch_name.clone();
                }

                let lines: Vec<&str> = patched_src.lines().collect();
                let start_l = line_no.saturating_sub(4).max(1);
                let end_l = (line_no + 4).min(lines.len());
                for l in start_l..=end_l {
                    let line_entry = patch_dbg.find_entry_at_line(l);
                    code_snippet.push(CrashSnippetLine {
                        line_number: l,
                        text: lines.get(l - 1).copied().unwrap_or("").to_string(),
                        is_crash_line: l == line_no,
                        attributed_mod_id: line_entry
                            .and_then(|e| e.patch_source.mod_id.clone()),
                        attributed_patch_id: line_entry
                            .and_then(|e| e.patch_source.patch_id.clone()),
                    });
                }
            }
        }

        // Also check if `[qm mod_name "source.lua"]:line` appeared in traceback
        if suspected_mod_id.is_none() {
            let mod_chunk_re = Regex::new(r#"\[qm\s+([^\s"]+)\s+"([^"]+)"\]:(\d+)"#).unwrap();
            if let Some(cap) = mod_chunk_re
                .captures(error_message)
                .or_else(|| mod_chunk_re.captures(traceback))
            {
                suspected_mod_id = Some(cap[1].to_string());
                suspected_patch_name = Some(format!("Module {} ({})", &cap[1], &cap[2]));
            }
        }

        let report = CrashReport {
            id: crash_id.clone(),
            timestamp: now.to_string(),
            crash_kind: crash_kind.to_string(),
            error_message: error_message.to_string(),
            traceback: traceback.to_string(),
            crashing_file,
            crashing_line,
            suspected_mod_id,
            suspected_patch_id,
            suspected_patch_name,
            code_snippet,
            active_profile_id: self.launch_config.profile_id.clone(),
            active_mods: self.launch_config.active_mods.clone(),
        };

        let crash_dir = self.mod_dir.join("qm").join("log").join("crashes");
        let _ = std::fs::create_dir_all(&crash_dir);
        if let Ok(json) = serde_json::to_string_pretty(&report) {
            let _ = std::fs::write(crash_dir.join(format!("{crash_id}.json")), json);
        }
        let _ = log::record_crash(&format!("{error_message}\n\n{traceback}"));

        report
    }

    /// Apply buffer patches. Intercepts Lua chunks loaded by luaL_loadbuffer / luaL_loadbufferx.
    pub fn apply_buffer_patches(
        &self,
        state: *mut LuaState,
        buf_ptr: *const u8,
        size: usize,
        name_ptr: *const u8,
        mode_ptr: *const u8,
    ) -> u32 {
        if self.is_vanilla {
            return (self.loadbuffer)(state, buf_ptr, size, name_ptr, mode_ptr);
        }

        let patch_table = self.patch_table.read().unwrap();

        let c_str = unsafe { CStr::from_ptr(name_ptr as _) };
        let name = match c_str.to_str() {
            Ok(x) => x,
            Err(e) => {
                warn!("The chunk name at {name_ptr:?} contains invalid UTF-8, skipping: {e}");
                return (self.loadbuffer)(state, buf_ptr, size, name_ptr, mode_ptr);
            }
        };

        // Check if main.lua is loading, inject QuarterMaster APIs and module patches
        if name.ends_with("main.lua") {
            unsafe {
                let closure: LuaFunc = sys::override_print;
                state.push(closure);
                sys::lua_setfield(state, sys::LUA_GLOBALSINDEX, c"print".as_ptr());

                patch_table.inject_metadata(state);

                let module_patches: Vec<_> = patch_table
                    .patches
                    .iter()
                    .filter_map(|(x, prio, path)| match x {
                        Patch::Module(patch) => Some((patch, *prio, path)),
                        _ => None,
                    })
                    .filter(|(x, _, _)| !x.load_now && !x.disabled)
                    .sorted_by_key(|(_, prio, _)| *prio)
                    .collect();

                for (patch, prio, path) in module_patches {
                    info!(
                        "Preloading module '{}' from {} (Priority: {})",
                        patch.name,
                        path.display(),
                        prio
                    );
                    let patch: &ModulePatch = patch;
                    let _ = patch.apply("", state, path);
                }
            }
        }

        // Prepare buffer for inspection and patching
        let buf = unsafe { slice::from_raw_parts(buf_ptr, size) };
        let buf_str = match std::str::from_utf8(buf) {
            Ok(s) => s,
            Err(e) => {
                warn!("The byte buffer for target {name} contains invalid UTF-8: {e:?}");
                return (self.loadbuffer)(state, buf_ptr, size, name_ptr, mode_ptr);
            }
        };

        let regex = Regex::new(r#"=\[(\w+)(?: (\S+))? "([^"]+)"\]"#).unwrap();
        let pretty_name = if let Some(capture) = regex.captures(name) {
            let f1 = capture.get(1).map_or("", |x| x.as_str());
            let f2 = capture.get(2).map_or("", |x| x.as_str()).replace('.', "/");
            let f3 = capture.get(3).map_or("", |x| x.as_str());
            format!("{f1}/{f2}/{f3}")
        } else {
            name.replace('@', "")
        };

        // --- BUILT-IN STEAM SAFETY & LAUNCH CONFIG INJECTION ---
        let mut current_buf = buf_str.to_string();
        let mut was_safety_patched = false;

        if name.ends_with("conf.lua") {
            if let Some(ref custom_identity) = self.launch_config.save_identity_override {
                if !custom_identity.trim().is_empty() {
                    info!(
                        "Overriding LÖVE save identity in conf.lua to '{}'",
                        custom_identity
                    );
                    current_buf = current_buf.replace(
                        "t.identity = nil",
                        &format!("t.identity = {:?}", custom_identity),
                    );
                    was_safety_patched = true;
                }
            }
        }

        if name.ends_with("main.lua") {
            info!("Injecting Steam safety hook into {}", name);
            current_buf.insert_str(0, STEAM_SAFETY_PAYLOAD);
            current_buf.push('\n');
            current_buf.push_str(QM_RUNTIME_FOOTER_PAYLOAD);
            was_safety_patched = true;
        }

        if name.ends_with("external.lua") {
            info!("Neutralizing uploadScoresHelper in {}", name);
            if current_buf.contains("function uploadScoresHelper") {
                current_buf = current_buf.replace(
                    "function uploadScoresHelper(handle, scorez)",
                    "function uploadScoresHelper(handle, scorez) return false end function _disabled_uploadScoresHelper(handle, scorez)",
                );
                was_safety_patched = true;
            }
        }
        // ---------------------------------------

        let needs_mod_patching =
            patch_table.needs_patching(name) || patch_table.needs_patching(&pretty_name);

        if !needs_mod_patching
            && !was_safety_patched
            && !self.dump_all.load(std::sync::atomic::Ordering::Relaxed)
        {
            return (self.loadbuffer)(state, buf_ptr, size, name_ptr, mode_ptr);
        }

        write_dump(&self.mod_dir, "game-dump", &pretty_name, buf_str, None);

        // Apply any user/mod patches on top of the buffer
        let (patched, debug) = if needs_mod_patching {
            let res = unsafe { patch_table.apply_patches(name, &current_buf, state) };
            if let Err(err_msg) = res {
                let _ = self.build_and_save_crash_report("lua_syntax", &err_msg, &err_msg);
                unsafe { state.push(err_msg) };
                return 3; // LUA_ERRSYNTAX
            }
            let (p, d, records) = res.unwrap();

            // Record debug regions for runtime crash attribution
            {
                let mut dbg_map = self.debug_by_file.write().unwrap();
                dbg_map.insert(pretty_name.clone(), (p.clone(), d.clone()));
            }

            // Update and persist Mods/qm/patch-report.json
            {
                let mut report = self.patch_report.write().unwrap();
                for rec in records {
                    match rec.status {
                        PatchOutcomeStatus::Applied => report.total_patches_applied += 1,
                        PatchOutcomeStatus::ZeroMatches | PatchOutcomeStatus::CountMismatch => {
                            report.total_warnings += 1;
                        }
                        PatchOutcomeStatus::SkippedByPolicy | PatchOutcomeStatus::Aborted => {
                            report.total_failures += 1;
                        }
                        PatchOutcomeStatus::SkippedDisabled => {}
                    }
                    report.records.push(rec);
                }
                let _ = report.save_to_mod_dir(&self.mod_dir);
            }

            (p, Some(d))
        } else {
            (current_buf, None)
        };

        write_dump(
            &self.mod_dir,
            "dump",
            &pretty_name,
            &patched,
            debug.as_ref(),
        );

        (self.loadbuffer)(
            state,
            patched.as_ptr(),
            patched.len(),
            name_ptr,
            mode_ptr,
        )
    }
}

unsafe extern "C" fn apply_patches(lua_state: *mut LuaState) -> c_int {
    let buf_name = check_lua_string(lua_state, 1);
    let buf = check_lua_string(lua_state, 2);
    let mut num = 1;
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let binding = RUNTIME.get().unwrap().patch_table.read().unwrap();
        if binding.needs_patching(&buf_name) {
            let res = binding.apply_patches(&buf_name, &buf, lua_state);
            if res.is_err() {
                lua_state.push(false);
                lua_state.push(res.unwrap_err());
                num = 2;
                return;
            }
            let (patched, _debug, _records) = res.unwrap();
            lua_state.push(patched);
        } else {
            lua_state.push(buf)
        }
    }));
    if result.is_ok() {
        num
    } else {
        lua_state.push(false);
        lua_state.push("Internal QuarterMaster error: Failed to acquire the QM runtime");
        2
    }
}

impl Target {
    pub fn can_apply(&self, target: &str) -> bool {
        let clean_target = target.trim_start_matches('@');
        match self {
            Self::Single(str) => {
                let clean_str = str.trim_start_matches('@');
                str == target
                    || clean_str == clean_target
                    || target.ends_with(str.as_str())
                    || clean_target.ends_with(clean_str)
            }
            Self::Multi(strs) => strs.iter().any(|x| {
                let clean_str = x.trim_start_matches('@');
                x == target
                    || clean_str == clean_target
                    || target.ends_with(x.as_str())
                    || clean_target.ends_with(clean_str)
            }),
        }
    }

    pub fn insert_into(&self, targets: &mut HashSet<String>) {
        match self {
            Self::Single(str) => {
                targets.insert(str.clone());
            }
            Self::Multi(strs) => {
                for target in strs.iter() {
                    targets.insert(target.clone());
                }
            }
        }
    }
}
