#![allow(non_upper_case_globals)]

use core::slice;
use std::collections::{HashMap, HashSet};
use std::ffi::{c_int, CStr};
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Instant;
use std::env;

use log::*;

use getargs::{Arg, Options};
use itertools::Itertools;
use patch::{ModulePatch, Patch};
use regex_lite::Regex;

use sys::{check_lua_string, LuaFunc, LuaLib, LuaState, LuaStateTrait, LUA};

use crate::patch::Target;
use crate::dump::write_dump;

pub mod chunk_vec_cursor;
pub mod dump;
pub mod log;
pub mod patch;
pub mod sys;

pub const QM_VERSION: &str = env!("CARGO_PKG_VERSION");

pub static RUNTIME: OnceLock<QuarterMaster> = OnceLock::new();

const STEAM_SAFETY_PAYLOAD: &str = r#"
-- Quartermaster: Steam Safety Lockdown
print("[QM] *** STEAM SAFETY HOOK INITIALIZED ***")

local function qm_neuter_steam()
  if Steam and Steam.userStats then
    Steam.userStats.uploadLeaderboardScore = function(h, m, s, d, cb)
      print("[QM] BLOCKED: Steam.userStats.uploadLeaderboardScore")
      if cb then cb({ success = false, globalRankNew = 0 }, "Quartermaster: Disabled") end
      return false
    end
    Steam.userStats.findLeaderboard = function(n, cb)
      print("[QM] BLOCKED: Steam.userStats.findLeaderboard (" .. tostring(n) .. ")")
      if cb then cb({ leaderboardFound = false, steamLeaderboard = 0 }, "Quartermaster: Disabled") end
      return false
    end
    Steam.userStats.setAchievement = function() return true end
    Steam.userStats.clearAchievement = function() return true end
    Steam.userStats.getAchievement = function() return false, false end
    Steam.userStats.setStatInt = function() return true end
    Steam.userStats.setStatFloat = function() return true end
    Steam.userStats.storeStats = function() return true end
    Steam.userStats.requestCurrentStats = function() return false end
    Steam.userStats.getStatInt = function() return false, 0 end
    Steam.userStats.getStatFloat = function() return false, 0.0 end
  end
  if Steam and Steam.apps then
    Steam.apps.isDlcInstalled = function() return false end
  end
end

qm_neuter_steam()

local blocked_globals = {
  uploadScoresHelper = true,
  steam_campaignTime = true,
  steam_classicWaves = true,
  steam_branderburging = true,
  steam_increaseClassicWins = true,
  steam_increaseCampaignWins = true,
  steam_campaignGG = true,
  steam_keener = true,
  steam_counterSnipe = true,
  steam_sink = true,
  steam_marquis = true,
  steam_mudBleed = true,
  steam_chunk = true,
  steam_comChallenge = true,
  steam_officerClub = true,
  steam_tigerHunter = true,
  steam_increaseBranderburg = true,
  steam_victoryDay = true,
}

for name, _ in pairs(blocked_globals) do
  _G[name] = function(...)
    print("[QM] BLOCKED CALL: " .. tostring(name))
    return false
  end
end

local prev_mt = getmetatable(_G) or {}
local prev_newindex = prev_mt.__newindex

setmetatable(_G, {
  __index = prev_mt.__index,
  __newindex = function(t, k, v)
    if blocked_globals[k] then
      rawset(t, k, function(...)
        print("[QM] BLOCKED CALL: " .. tostring(k))
        return false
      end)
    elseif k == "Steam" then
      rawset(t, k, v)
      qm_neuter_steam()
    else
      if type(prev_newindex) == "function" then
        prev_newindex(t, k, v)
      else
        rawset(t, k, v)
      end
    end
  end
})
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
        qm.dump_all.store(enabled, std::sync::atomic::Ordering::Relaxed);
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

pub struct QuarterMaster {
    pub mod_dir: PathBuf,
    pub is_vanilla: bool,
    loadbuffer: &'static LoadBuffer,
    patch_table: Arc<RwLock<PatchTable>>,
    pub dump_all: std::sync::atomic::AtomicBool,
    lua_vars: Arc<RwLock<HashMap<String, String>>>,
}

pub type QM = QuarterMaster;

impl QuarterMaster {
    /// Initialize the QuarterMaster patch runtime.
    pub fn init(loadbuffer: &'static LoadBuffer, lualib: LuaLib, dump_all: bool) -> &'static Self {
        assert!(RUNTIME.get().is_none());

        LUA.set(lualib)
            .unwrap_or_else(|_| panic!("LUA static var has already been set."));

        let start = Instant::now();

        let args = std::env::args().skip(1).collect_vec();
        let mut opts = Options::new(args.iter().map(String::as_str));

        let mut mod_dir = if let Some(env_path) = env::var_os("QM_MOD_DIR") {
            PathBuf::from(env_path)
        } else {
            let cur_exe =
                env::current_exe().expect("Failed to get the path of the current executable.");
            let exe_dir = cur_exe.parent().unwrap_or(Path::new("."));
            let local_mods = exe_dir.join("Mods");
            
            // Prioritize local Mods directory alongside mnb.exe if it exists
            if local_mods.is_dir() {
                local_mods
            } else {
                let game_name = cur_exe
                    .file_stem()
                    .expect("Failed to get file_stem component of current executable path.")
                    .to_string_lossy()
                    .replace(".", "_");
                dirs::data_dir().unwrap().join(game_name).join("Mods")
            }
        };

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

        let log_dir = mod_dir.join("qm").join("log");

        log::init(&log_dir).unwrap_or_else(|e| panic!("Failed to initialize logger: {e:?}"));

        info!("========================================================");
        info!("QuarterMaster (QM) v{} — Mod Loader for Mud and Blood", QM_VERSION);
        info!("Active Mod Directory: {}", mod_dir.display());
        info!("========================================================");

        if is_vanilla {
            info!("Vanilla mode enabled via command line flag, ignoring patches.");
        }

        let (patch_table, lua_vars) = if !is_vanilla {
            let patch_table = PatchTable::load(&mod_dir)
                .unwrap_or_else(|e| panic!("Failed to load patches from mod directory: {e:?}"));
            let lua_vars = patch_table.vars.clone();
            (patch_table, lua_vars)
        } else {
            (PatchTable::default(), HashMap::new())
        };

        let qm = QuarterMaster {
            mod_dir,
            is_vanilla,
            loadbuffer,
            patch_table: Arc::new(RwLock::new(patch_table)),
            dump_all: std::sync::atomic::AtomicBool::new(dump_all),
            lua_vars: Arc::new(RwLock::new(lua_vars)),
        };

        let qm = RUNTIME.get_or_init(|| qm);

        info!("QuarterMaster initialized in {:.2}ms", start.elapsed().as_secs_f64() * 1000.0);

        qm
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
                // Hook print so Lua prints output to the QM logger
                let closure: LuaFunc = sys::override_print;
                state.push(closure);
                sys::lua_setfield(state, sys::LUA_GLOBALSINDEX, c"print".as_ptr());

                // Inject QuarterMaster metadata into Lua runtime
                patch_table.inject_metadata(state);

                // Inject registered mod modules into runtime
                let module_patches: Vec<_> = patch_table
                    .patches
                    .iter()
                    .filter_map(|(x, prio, path)| match x {
                        Patch::Module(patch) => Some((patch, prio, path)),
                        _ => None,
                    })
                    .filter(|(x, _, _)| !x.load_now)
                    .sorted_by_key(|(_, &prio, _)| prio)
                    .map(|(x, _, path)| (x, path))
                    .collect();

                for (patch, path) in module_patches {
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
            let f2 = capture.get(2).map_or("", |x| x.as_str()).replace(".", "/");
            let f3 = capture.get(3).map_or("", |x| x.as_str());
            format!("{f1}/{f2}/{f3}")
        } else {
            name.replace("@", "")
        };

        // --- BUILT-IN STEAM SAFETY INJECTION ---
        let mut current_buf = buf_str.to_string();
        let mut was_safety_patched = false;

        if name.ends_with("main.lua") {
            info!("[QM] Injecting Steam safety hook into {}", name);
            current_buf.insert_str(0, STEAM_SAFETY_PAYLOAD);
            was_safety_patched = true;
        }

        if name.ends_with("external.lua") {
            info!("[QM] Neutralizing uploadScoresHelper in {}", name);
            if current_buf.contains("function uploadScoresHelper") {
                current_buf = current_buf.replace(
                    "function uploadScoresHelper(handle, scorez)",
                    "function uploadScoresHelper(handle, scorez) return false end function _disabled_uploadScoresHelper(handle, scorez)",
                );
                was_safety_patched = true;
            }
        }
        // ---------------------------------------

        let needs_mod_patching = patch_table.needs_patching(name) || patch_table.needs_patching(&pretty_name);

        // Stop here if no valid patch exists and no safety patch was applied
        if !needs_mod_patching && !was_safety_patched && !self.dump_all.load(std::sync::atomic::Ordering::Relaxed) {
            return (self.loadbuffer)(state, buf_ptr, size, name_ptr, mode_ptr);
        }

        write_dump(&self.mod_dir, "game-dump", &pretty_name, buf_str, None);

        // Apply any user/mod patches on top of the buffer
        let (patched, debug) = if needs_mod_patching {
            let res = unsafe { patch_table.apply_patches(name, &current_buf, state) };
            if res.is_err() {
                unsafe { state.push(res.unwrap_err()) };
                return 3; // LUA_ERRSYNTAX
            }
            let (p, d) = res.unwrap();
            (p, Some(d))
        } else {
            (current_buf, None)
        };

        write_dump(&self.mod_dir, "dump", &pretty_name, &patched, debug.as_ref());

        (self.loadbuffer)(state, patched.as_ptr(), patched.len(), name_ptr, mode_ptr)
    }
}

// Import PatchTable
use crate::patch::table::PatchTable;

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
            let (patched, _debug) = res.unwrap();
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
                str == target || clean_str == clean_target || target.ends_with(str.as_str()) || clean_target.ends_with(clean_str)
            }
            Self::Multi(strs) => strs.iter().any(|x| {
                let clean_str = x.trim_start_matches('@');
                x == target || clean_str == clean_target || target.ends_with(x.as_str()) || clean_target.ends_with(clean_str)
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
