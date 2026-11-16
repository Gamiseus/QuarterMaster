#![allow(non_upper_case_globals)]
// The DLL-path items below carry
// `#[cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing))]`
// one by one: an inner attribute here would cover every module of the crate.

use core::slice;
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{c_int, CStr};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, PoisonError, RwLock, TryLockError};
use std::time::Instant;

use log::*;

use getargs::{Arg, Options};
use itertools::Itertools;
use patch::{ModulePatch, Patch};
use regex_lite::Regex;

use sys::{check_lua_string, LuaFunc, LuaLib, LuaState, LuaStateTrait, LUA};

use crate::dump::write_dump;
use crate::handshake::{
    CrashReport, CrashSnippetLine, LaunchConfig, LoadError, PatchOutcomeStatus, PatchReport,
};
use crate::patch::table::{PatchTable, Prepared};
use crate::patch::Target;
use crate::prepare::DllOutcome;

pub mod chunk_vec_cursor;
pub mod compile;
pub mod dump;
pub mod features;
pub use features::QM_FEATURES;
pub mod guard;
pub mod handshake;
pub mod lenient;
pub mod log;
pub mod lua_scan;
pub mod patch;
pub mod prepare;
pub mod sys;

#[cfg(test)]
mod dll_tests;

pub const QM_VERSION: &str = env!("CARGO_PKG_VERSION");

pub static RUNTIME: OnceLock<QuarterMaster> = OnceLock::new();

pub const QM_RUNTIME_FOOTER_PAYLOAD: &str = r#"
-- Quartermaster: Crash Attribution, Save Fingerprinting & Hot-Reload Hooks
do
  local ok_qm, qm_mod = pcall(require, "qm")
  if ok_qm and qm_mod and love then
    local qm_unpack = unpack or table.unpack
    local function qm_pack(...)
      return { n = select('#', ...), ... }
    end
    local orig_errhand = love.errorhandler or love.errhand
    -- The handler a mod's love.load handler replaced; a re-entered wrapper goes on to it.
    local inner_errhand = nil
    local in_errhand = false
    local function qm_crash_wrapper(msg)
      if in_errhand then
        -- A mod's handler called the wrapper it saw in love.load.
        if type(inner_errhand) == "function" then
          return inner_errhand(msg)
        end
        return
      end
      in_errhand = true
      local trace = debug and debug.traceback and debug.traceback(tostring(msg), 2) or tostring(msg)
      local ok_attr, attribution = pcall(qm_mod.record_lua_crash, tostring(msg), tostring(trace))
      local enriched_msg = tostring(msg)
      if ok_attr and type(attribution) == "string" and #attribution > 0 then
        enriched_msg = enriched_msg .. "\n\n[QuarterMaster Crash Attribution]\n" .. attribution
      end
      local r = { n = 0 }
      if type(orig_errhand) == "function" then
        r = qm_pack(orig_errhand(enriched_msg))
      end
      in_errhand = false
      return qm_unpack(r, 1, r.n)
    end
    love.errorhandler = qm_crash_wrapper
    love.errhand = qm_crash_wrapper
    -- After love.load: chain to a handler a mod installed there, then put the wrapper back.
    local function qm_reassert_errhand()
      local now = love.errorhandler
      if now == qm_crash_wrapper then
        now = love.errhand
      end
      if now ~= qm_crash_wrapper and type(now) == "function" then
        inner_errhand = orig_errhand
        orig_errhand = now
      end
      love.errorhandler = qm_crash_wrapper
      love.errhand = qm_crash_wrapper
    end
    -- The game's love.quit runs first, unprotected, so its errors reach the crash handler.
    -- Only a quit that goes ahead (a falsy result) is a clean exit.
    local orig_quit = love.quit
    -- Set in a diagnostic test run: writes what was traced so far.
    local qm_diag_report = nil
    -- The love.quit a mod's love.load replaced; a re-entered wrapper goes on to it.
    local inner_quit = nil
    local in_quit = false
    local function qm_quit_wrapper(...)
      if in_quit then
        -- A mod's love.quit called the wrapper it saw in love.load.
        if type(inner_quit) == "function" then
          return inner_quit(...)
        end
        return
      end
      in_quit = true
      local r = { n = 0 }
      if type(orig_quit) == "function" then
        r = qm_pack(orig_quit(...))
      end
      in_quit = false
      if type(qm_diag_report) == "function" then
        pcall(qm_diag_report)
      end
      if not r[1] then
        pcall(qm_mod.record_clean_exit)
      end
      return qm_unpack(r, 1, r.n)
    end
    love.quit = qm_quit_wrapper
    local orig_load = love.load
    if type(orig_load) == "function" then
      love.load = function(...)
        local r = qm_pack(orig_load(...))
        qm_reassert_errhand()
        -- A mod may have replaced a game function QuarterMaster wraps (updateProfile, the
        -- faction's functions) after it was wrapped: put QuarterMaster's part back around it.
        local reassert = rawget(_G, "__qm_reassert_wrappers")
        if type(reassert) == "function" then
          pcall(reassert)
        end
        -- A diagnostic test run: report what the watched globals did while the game loaded,
        -- count who calls the watched functions, and report again as the game runs.
        local diag = rawget(_G, "__qm_diag")
        if type(diag) == "table" and type(qm_mod.record_diagnostics) == "function" then
          local function report()
            local ok_t, text = pcall(diag.text)
            if ok_t and type(text) == "string" then
              pcall(qm_mod.record_diagnostics, text)
            end
          end
          qm_diag_report = report
          pcall(diag.checkpoint)
          pcall(diag.wrap_hooks)
          report()
          local orig_update = love.update
          if type(orig_update) == "function" then
            local frames = 0
            love.update = function(...)
              local u = qm_pack(orig_update(...))
              frames = frames + 1
              if frames == 1 then
                -- The first frame is done: the line hook comes off.
                pcall(diag.stop)
                report()
              elseif frames % 300 == 0 and diag.take_dirty() then
                report()
              end
              return qm_unpack(u, 1, u.n)
            end
          end
        end
        -- Whose file each replaced asset ended up being, for the launcher's asset list.
        local winner_text = rawget(_G, "__qm_asset_winner_text")
        if type(qm_mod.record_asset_winners) == "function" and type(winner_text) == "function" then
          local ok_w, text = pcall(winner_text)
          if ok_w and type(text) == "string" and #text > 0 then
            pcall(qm_mod.record_asset_winners, text)
          end
        end
        -- Every image is loaded and replaced by now: apply the [sprites] sizes of the mods.
        local sprite_rules = rawget(_G, "__qm_apply_sprite_rules")
        if type(sprite_rules) == "function" then
          pcall(sprite_rules)
        end
        if love.quit ~= qm_quit_wrapper then
          inner_quit = orig_quit
          orig_quit = love.quit
          love.quit = qm_quit_wrapper
        end
        return qm_unpack(r, 1, r.n)
      end
    end
  end
end

-- QuarterMaster: the debug dump keys (debug mode) and the game's own developer commands
do
  local ok_qm, qm_mod = pcall(require, "qm")
  if ok_qm and qm_mod and love and (qm_mod.debug or qm_mod.dev) then
    local function cue(text)
      print("[QuarterMaster] " .. text)
      local p = rawget(_G, "player")
      if type(p) == "table" and type(p.textCue) == "table" then
        p.textCue[#p.textCue + 1] = { name = text, x = (p.screenx or 0) + 600, y = 300, alpha = 255 }
      end
    end
    local function dump(label, fn, ...)
      local ok, done, count = pcall(fn, ...)
      if ok and done then
        cue("Dumped " .. tostring(count) .. " " .. label)
      else
        cue("Dump failed: " .. tostring(ok and count or done))
      end
    end
    local actions = {}
    local function add(id, name, category, key, mods, fn)
      actions[#actions + 1] = { id = id, name = name, category = category, key = key, mods = mods, fn = fn }
    end
    -- The GDumper mod binds the same keys; it keeps them when it is loaded.
    if qm_mod.debug and not rawget(_G, "GDumper") then
      add("qm_dump_game_lua", "Dump Game Lua Files", "Debug", "d", { "ctrl", "shift" }, function()
        dump("game Lua files to Mods/qm/game-dump", qm_mod.dump_game_files)
      end)
      add("qm_dump_game_all", "Dump Full Game Archive", "Debug", "d", { "ctrl", "shift", "alt" }, function()
        dump("game files to Mods/qm/game-dump-full", qm_mod.dump_game_files, "qm/game-dump-full", "*")
      end)
      add("qm_dump_patched", "Dump Patched Lua Files", "Debug", "p", { "ctrl", "shift" }, function()
        dump("patched Lua files to Mods/qm/dump", qm_mod.dump_patched)
      end)
    end
    if qm_mod.dev then
      add("qm_dev_ai", "Toggle AI Debug Overlay", "Developer", "f1", nil, function()
        showAI = (showAI == 1) and 0 or 1
        cue(showAI == 1 and "AI debug ON" or "AI debug OFF")
      end)
      add("qm_dev_blast", "Toggle Dev Blast", "Developer", "f5", nil, function()
        devBlast = (devBlast == 1) and 0 or 1
        cue(devBlast == 1 and "Dev blast ON (right-click: air strike)" or "Dev blast OFF")
      end)
    end

    local qm_table = rawget(_G, "QM")
    local input = type(qm_table) == "table" and qm_table.Input or nil
    local has_input = type(input) == "table" and type(input.registerAction) == "function"
    if has_input then
      -- Listed and rebindable on the Controls screen.
      for _, a in ipairs(actions) do
        input.registerAction({
          id = a.id, name = a.name, category = a.category, tab = "quartermaster",
          tabName = "QuarterMaster", default = a.key, modifiers = a.mods, onTrigger = a.fn,
        })
      end
    elseif #actions > 0 then
      local orig_keypressed = love.keypressed
      love.keypressed = function(key, ...)
        local down = love.keyboard.isDown
        local held = {
          ctrl = down("lctrl") or down("rctrl"),
          shift = down("lshift") or down("rshift"),
          alt = down("lalt") or down("ralt"),
        }
        for _, a in ipairs(actions) do
          if key == a.key then
            local want = { ctrl = false, shift = false, alt = false }
            for _, m in ipairs(a.mods or {}) do want[m] = true end
            if want.ctrl == held.ctrl and want.shift == held.shift and want.alt == held.alt then
              local ok, err = pcall(a.fn)
              if not ok then print("[QuarterMaster] " .. a.id .. " failed: " .. tostring(err)) end
              return
            end
          end
        end
        if type(orig_keypressed) == "function" then
          return orig_keypressed(key, ...)
        end
      end
    end

    if qm_mod.dev then
      -- The game's dev blast: with it on, a right-click calls an air strike on the spot. The
      -- GConsole mod has its own right-click handling and keeps it.
      local function blast(x, y, button)
        local p = rawget(_G, "player")
        if button == 2 and devBlast == 1 and not rawget(_G, "GConsole")
          and type(p38Strike) == "function" and type(p) == "table" and p.gameState == 0 then
          pcall(p38Strike, x + (p.screenx or 0), y + (p.screeny or 0), 6)
        end
      end
      if has_input then
        -- The input shim owns love.mousereleased and calls this one.
        local orig_released = input.origMouseReleased
        input.origMouseReleased = function(x, y, button, ...)
          if type(orig_released) == "function" then orig_released(x, y, button, ...) end
          blast(x, y, button)
        end
      else
        local orig_released = love.mousereleased
        love.mousereleased = function(x, y, button, ...)
          if type(orig_released) == "function" then orig_released(x, y, button, ...) end
          blast(x, y, button)
        end
      end
    end
  end
end
"#;

/// Printed (to the game console and log) by the first chunk after mods were turned off.
pub const DEGRADED_NOTICE: &str =
    "QuarterMaster turned mods off after an internal error; see patch-report.json";

/// `luaL_loadbuffer`'s syntax error code.
const LUA_ERRSYNTAX: u32 = 3;

type LoadBuffer =
    dyn Fn(*mut LuaState, *const u8, usize, *const u8, *const u8) -> u32 + Send + Sync + 'static;

// Every Lua callback body runs in `guard::unguarded`: a panic in it cannot be caught, so the
// panic hook must report it and abort (see guard.rs).
#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
unsafe extern "C" fn reload_patches(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        let Some(qm) = RUNTIME.get() else {
            state.push(false);
            state.push("QuarterMaster runtime not initialized");
            return 2;
        };
        match qm.reload() {
            Ok(()) => {
                state.push(true);
                1
            }
            Err(e) => {
                state.push(false);
                state.push(e);
                2
            }
        }
    })
}

#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
unsafe extern "C" fn getvar(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        let key = check_lua_string(state, 1);
        let Some(qm) = RUNTIME.get() else {
            return 0;
        };
        let vars = qm.lua_vars.read().unwrap_or_else(PoisonError::into_inner);
        let val = vars.get(&key);
        if let Some(val) = val {
            state.push(val);
            return 1;
        }
        0
    })
}

#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
unsafe extern "C" fn setvar(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        let key = check_lua_string(state, 1);
        let val = check_lua_string(state, 2);
        let Some(qm) = RUNTIME.get() else {
            return 0;
        };
        let mut vars = qm.lua_vars.write().unwrap_or_else(PoisonError::into_inner);
        vars.insert(key, val);
        0
    })
}

#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
unsafe extern "C" fn removevar(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        let key = check_lua_string(state, 1);
        let Some(qm) = RUNTIME.get() else {
            return 0;
        };
        let mut vars = qm.lua_vars.write().unwrap_or_else(PoisonError::into_inner);
        let val = vars.remove(&key);
        if let Some(val) = val {
            state.push(val);
            return 1;
        }
        0
    })
}

/// `qm.dump_vanilla(target, filter)` (also `dump_game_files`): copies the game's files out.
///
/// # Safety
/// Lua calls it with a valid state.
#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
pub unsafe extern "C" fn dump_vanilla(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        let qm = match RUNTIME.get() {
            Some(q) => q,
            None => {
                state.push(false);
                state.push("QuarterMaster runtime not initialized");
                return 2;
            }
        };

        if !qm.debug {
            state.push(false);
            state.push(DUMP_NEEDS_DEBUG);
            return 2;
        }

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
    })
}

/// What a dump command returns outside debug mode.
pub const DUMP_NEEDS_DEBUG: &str =
    "the dump commands need debug mode: start the game with -debug or turn on Debug mode in the launcher";

/// `qm.dump_patched()`: writes every game Lua file as the game loads it, patches applied, to
/// `Mods/qm/dump`, also the files the game has not loaded yet. Debug mode only.
///
/// # Safety
/// Lua calls it with a valid state.
#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
pub unsafe extern "C" fn dump_patched(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        let result = match RUNTIME.get() {
            None => Err("QuarterMaster runtime not initialized".to_string()),
            Some(qm) if !qm.debug => Err(DUMP_NEEDS_DEBUG.to_string()),
            Some(qm) => qm.dump_patched_files(),
        };
        match result {
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
    })
}

/// `qm.set_dump_all(enabled)`: dump every file the game loads (default true).
///
/// # Safety
/// Lua calls it with a valid state.
#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
pub unsafe extern "C" fn set_dump_all(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
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
    })
}

/// `qm.is_dump_all()`: whether every loaded file is dumped.
///
/// # Safety
/// Lua calls it with a valid state.
#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
pub unsafe extern "C" fn is_dump_all(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        if let Some(qm) = RUNTIME.get() {
            state.push(qm.dump_all.load(std::sync::atomic::Ordering::Relaxed));
            1
        } else {
            state.push(false);
            1
        }
    })
}

/// `qm.record_lua_crash(msg, trace)`: saves a crash report and returns its summary.
///
/// # Safety
/// Lua calls it with a valid state.
#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
pub unsafe extern "C" fn record_lua_crash(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
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
    })
}

/// `qm.record_clean_exit()`: the footer's love.quit wrapper calls it when the quit goes ahead
/// (D66). Returns whether patch-report.json was saved.
///
/// # Safety
/// Lua calls it with a valid state.
#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
pub unsafe extern "C" fn record_clean_exit(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        let saved = match RUNTIME.get() {
            Some(qm) => qm.record_clean_exit(),
            None => Err("QuarterMaster runtime not initialized".to_string()),
        };
        if let Err(e) = &saved {
            error!("Could not record the clean exit: {e}");
        }
        state.push(saved.is_ok());
        1
    })
}

/// `qm.record_diagnostics(text)`: the footer of a diagnostic test run calls it with the lines
/// of `__qm_diag.text()` (Phase E D89, D90). Returns whether patch-report.json was saved.
///
/// # Safety
/// Lua calls it with a valid state.
#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
pub unsafe extern "C" fn record_diagnostics(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        let text = sys::opt_lua_string(state, 1).unwrap_or_default();
        let saved = match RUNTIME.get() {
            Some(qm) => qm.record_diagnostics(&text),
            None => Err("QuarterMaster runtime not initialized".to_string()),
        };
        if let Err(e) = &saved {
            error!("Could not record the diagnostic trace: {e}");
        }
        state.push(saved.is_ok());
        1
    })
}

/// `qm.record_asset_winners(text)`: the footer calls it after love.load with the
/// "slot<TAB>source" lines of `__qm_asset_winner_text` (Phase E D77). Returns whether
/// patch-report.json was saved.
///
/// # Safety
/// Lua calls it with a valid state.
#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
pub unsafe extern "C" fn record_asset_winners(state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        let text = sys::opt_lua_string(state, 1).unwrap_or_default();
        let saved = match RUNTIME.get() {
            Some(qm) => qm.record_asset_winners(&text),
            None => Err("QuarterMaster runtime not initialized".to_string()),
        };
        if let Err(e) = &saved {
            error!("Could not record the asset winners: {e}");
        }
        state.push(saved.is_ok());
        1
    })
}

/// The most asset winners one report keeps (the game has under 7,000 asset files).
const MAX_ASSET_WINNERS: usize = 20_000;
/// The most lines of a diagnostic trace that are read.
const MAX_DIAGNOSTIC_LINES: usize = 50_000;

/// The plan and order parts of patch-report.json (§3.9).
fn report_fields_of(r: &crate::patch::plan::PlanReport) -> PatchReport {
    let mut report = PatchReport {
        plan_error: r.plan_error.clone(),
        plan_warnings: r.plan_warnings.clone(),
        fix_bindings: r.bindings.clone(),
        ..Default::default()
    };
    set_order_fields(&mut report, r);
    report
}

/// Copies the order results (overridden rules, unknown ids, suspended and invalid rungs) and
/// the compat override reports.
fn set_order_fields(report: &mut PatchReport, r: &crate::patch::plan::PlanReport) {
    report.overrides = r.overrides.clone();
    report.overridden_rules = r.overridden_rules.clone();
    report.unknown_ids = r.unknown_ids.clone();
    report.suspended_rungs = r.suspended_rungs.clone();
    report.invalid_rungs = r.invalid_rungs.clone();
}

/// Chunks whose builtin edits are safety edits: they never load without them (D34). Root
/// files by canonical path (D44); a name that is not UTF-8 is read lossily.
fn is_safety_chunk(name: &[u8]) -> bool {
    let file = prepare::chunk_file(&String::from_utf8_lossy(name));
    file.is_root(prepare::MAIN_LUA) || file.is_root(prepare::EXTERNAL_LUA)
}

/// What init loads: the table, its `Prepared`, the Lua vars and the first patch report.
struct InitLoad {
    table: PatchTable,
    lua_vars: HashMap<String, String>,
    prepared: Option<Prepared>,
    report: PatchReport,
    /// A panic caught while loading: mods are off for the session (D34).
    panic: Option<String>,
}

/// Loads the Mods folder for init and builds the first report, under the panic guard (D34).
/// Init runs inside DllMain. The plan, the fingerprint binding and `prepare` (ladders, drift,
/// order) run here and not at the first buffer as D32 has it, because the load report needs
/// the order; the guard keeps a panic here from aborting the game, as at the first buffer.
#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
fn load_for_init(
    mod_dir: &Path,
    is_vanilla: bool,
    profile_id: Option<String>,
    launch_config_error: Option<String>,
) -> InitLoad {
    let loaded = guard::guarded(|| {
        #[cfg(test)]
        inject_test_panic(INIT_PANIC_NAME);
        // A mod that fails to load is skipped with its requirers (D15). Only an unreadable
        // Mods folder fails here: run without mods, and say so (no message box, D32). A
        // broken launch-config.json runs without mods too, never vanilla (D65).
        let mut mods_folder_error = None;
        let (table, lua_vars, prep) = if !is_vanilla && launch_config_error.is_none() {
            match PatchTable::load_prepared(mod_dir) {
                Ok((table, prep)) => {
                    let lua_vars = table.vars.clone();
                    (table, lua_vars, Some(prep))
                }
                Err(e) => {
                    error!("Failed to load patches from mod directory, running without mods: {e:#}");
                    mods_folder_error = Some(LoadError {
                        mod_id: String::new(),
                        folder: mod_dir.display().to_string(),
                        message: format!("Could not read the Mods folder: {e:#}"),
                        skipped_dependents: Vec::new(),
                    });
                    (PatchTable::default(), HashMap::new(), None)
                }
            }
        } else {
            (PatchTable::default(), HashMap::new(), None)
        };

        let prep = prep.unwrap_or_else(|| table.prepare());
        let ordered = prep.order();
        let mut load_errors = table.load_errors.clone();
        load_errors.extend(mods_folder_error);
        let report = PatchReport {
            qm_version: QM_VERSION.to_string(),
            timestamp: jiff::Zoned::now().to_string(),
            profile_id: profile_id.clone(),
            order_cycles: ordered.cycles.clone(),
            load_errors,
            order_degraded: ordered.degraded,
            launch_config_error: launch_config_error.clone(),
            ..report_fields_of(prep.report())
        };
        InitLoad {
            table,
            lua_vars,
            prepared: Some(prep),
            report,
            panic: None,
        }
    });
    loaded.unwrap_or_else(|message| InitLoad {
        table: PatchTable::default(),
        lua_vars: HashMap::new(),
        prepared: None,
        report: PatchReport {
            qm_version: QM_VERSION.to_string(),
            timestamp: jiff::Zoned::now().to_string(),
            profile_id,
            launch_config_error,
            ..Default::default()
        },
        panic: Some(message),
    })
}

/// launch-config.json as init uses it: the defaults when it is missing, and the defaults with
/// the error when it cannot be read or parsed (D65).
fn read_launch_config(mod_dir: &Path) -> (LaunchConfig, Option<String>) {
    match LaunchConfig::load_from_mod_dir(mod_dir) {
        Ok(config) => (config.unwrap_or_default(), None),
        Err(e) => (LaunchConfig::default(), Some(e)),
    }
}

/// The name `load_for_init` passes to the test panic injector.
#[cfg(test)]
const INIT_PANIC_NAME: &str = "@qm-init";

#[cfg(test)]
thread_local! {
    /// Test-only panic injector: the guarded apply panics on a chunk name ending with this.
    static PANIC_IN: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// Test-only: the builtin-only fallback panics too.
    static PANIC_IN_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    /// Test-only: the lines `show_load_errors_once` printed on this thread.
    static PRINTED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The log level of a load notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notice {
    Error,
    Warn,
    Info,
}

#[cfg(test)]
fn inject_test_panic(name: &str) {
    let hit = PANIC_IN.with(|p| p.borrow().as_deref().is_some_and(|s| name.ends_with(s)));
    if hit {
        panic!("injected test panic in {name}");
    }
}

/// The DLL's patch state: the table and what `PatchTable::prepare` built from it. Init keeps
/// the `Prepared` it built for the load report (`load_for_init`); after a reload, or a panic
/// caught at init, it is built at the first buffer. A reload replaces the whole `Runtime`, so it never outlives its table.
struct Runtime {
    table: PatchTable,
    prepared: OnceLock<Prepared>,
}

#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
impl Runtime {
    fn new(table: PatchTable) -> Self {
        Self {
            table,
            prepared: OnceLock::new(),
        }
    }

    /// With the `Prepared` already built from `table`.
    fn with_prepared(table: PatchTable, prepared: Prepared) -> Self {
        Self {
            table,
            prepared: OnceLock::from(prepared),
        }
    }

    /// Built on first use, then shared by every buffer.
    fn prepared(&self) -> &Prepared {
        self.prepared.get_or_init(|| self.table.prepare())
    }
}

pub struct QuarterMaster {
    pub mod_dir: PathBuf,
    pub is_vanilla: bool,
    pub launch_config: LaunchConfig,
    loadbuffer: &'static LoadBuffer,
    runtime: Arc<RwLock<Runtime>>,
    pub dump_all: std::sync::atomic::AtomicBool,
    /// Debug mode (`LaunchConfig::debug`, `-debug`, `QM_DEBUG=1`): Lua sees it as `qm.debug`.
    pub debug: bool,
    /// The game's developer commands are unlocked (`dev_commands`, `-dev`, `QM_DEV=1`):
    /// `qm.dev` in Lua.
    pub dev: bool,
    lua_vars: Arc<RwLock<HashMap<String, String>>>,
    patch_report: Arc<RwLock<PatchReport>>,
    debug_by_file: Arc<RwLock<HashMap<String, FileDebug>>>,
    /// Set by the first caught panic: from then on every chunk gets only the builtin edits
    /// (D34).
    degraded: AtomicBool,
    /// The degraded notice was printed.
    degraded_notice_shown: AtomicBool,
    /// The load errors were printed at the first main.lua load.
    load_errors_shown: AtomicBool,
}

pub type QM = QuarterMaster;

/// What the DLL keeps of a patched chunk for crash attribution.
#[derive(Debug, Clone, Default)]
pub struct FileDebug {
    /// The text the game loaded.
    pub text: String,
    /// Per line, the index in `owners` of the record that wrote it, or `linemap::NO_RECORD`.
    pub line_records: Vec<u32>,
    pub owners: Vec<RecordOwner>,
}

/// The patch of one record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordOwner {
    pub mod_id: String,
    pub patch_id: String,
    pub patch_name: String,
    /// The profile fixes that changed the patch.
    pub fix_ids: Vec<String>,
    /// The patch is an adjusted copy: its fix, not its mod, is the suspect (D58).
    pub adjusted_copy: bool,
}

impl FileDebug {
    /// The patch that wrote 1-based `line`, if any.
    pub fn owner_of_line(&self, line: usize) -> Option<&RecordOwner> {
        let record = *self.line_records.get(line.checked_sub(1)?)?;
        self.owners.get(record as usize)
    }
}

#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
impl QuarterMaster {
    /// Locate the active `Mods` directory prior to full initialization.
    pub fn resolve_default_mod_dir() -> PathBuf {
        if let Some(env_path) = env::var_os("QM_MOD_DIR") {
            return PathBuf::from(env_path);
        }
        // Without the game's own path, use `Mods` in the working directory.
        let Ok(cur_exe) = env::current_exe() else {
            return PathBuf::from("Mods");
        };
        let exe_dir = cur_exe.parent().unwrap_or(Path::new("."));
        let local_mods = exe_dir.join("Mods");

        if local_mods.is_dir() {
            local_mods
        } else {
            let game_name = cur_exe
                .file_stem()
                .map(|s| s.to_string_lossy().replace('.', "_"))
                .unwrap_or_else(|| "mnb".to_string());
            match dirs::data_dir() {
                Some(data) => data.join(game_name).join("Mods"),
                None => local_mods,
            }
        }
    }

    /// Initialize the QuarterMaster patch runtime.
    pub fn init(loadbuffer: &'static LoadBuffer, lualib: LuaLib, dump_all: bool) -> &'static Self {
        if let Some(qm) = RUNTIME.get() {
            warn!("QuarterMaster was initialized twice; keeping the first runtime.");
            return qm;
        }

        let lua_already_set = LUA.set(lualib).is_err();

        let start = Instant::now();

        let args = std::env::args().skip(1).collect_vec();
        let mut opts = Options::new(args.iter().map(String::as_str));

        let mut mod_dir = Self::resolve_default_mod_dir();
        let mut cli_vanilla = false;
        let mut arg_error = None;

        loop {
            match opts.next_arg() {
                Ok(Some(opt)) => match opt {
                    Arg::Long("mod-dir") => {
                        mod_dir = opts.value().map(PathBuf::from).unwrap_or(mod_dir)
                    }
                    Arg::Long("vanilla") => cli_vanilla = true,
                    _ => (),
                },
                Ok(None) => break,
                Err(e) => {
                    arg_error = Some(e.to_string());
                    break;
                }
            }
        }

        let (launch_config, launch_config_error) = read_launch_config(&mod_dir);
        let all_args = std::env::args().collect_vec();
        let debug = launch_config.debug || handshake::launch_flag(&all_args, "debug", "QM_DEBUG");
        let dev = launch_config.dev_commands || handshake::launch_flag(&all_args, "dev", "QM_DEV");
        let effective_dump_all = dump_all || launch_config.dump_all || debug;

        let log_dir = mod_dir.join("qm").join("log");

        if let Err(e) = log::init(&log_dir) {
            let _ = writeln!(std::io::stderr(), "QuarterMaster: could not start the logger: {e:?}");
        }

        info!("========================================================");
        info!(
            "QuarterMaster (QM) v{} — Mod Loader for Mud and Blood",
            QM_VERSION
        );
        info!("Active Mod Directory: {}", mod_dir.display());
        if let Some(ref prof) = launch_config.profile_name {
            info!("Active Profile: {}", prof);
        }
        if debug {
            info!("Debug mode: on (console, full dumps, dump commands)");
        }
        if dev {
            info!("Developer commands: on");
        }
        info!("========================================================");
        if lua_already_set {
            warn!("The Lua library bindings were already set; keeping them.");
        }
        if let Some(e) = arg_error {
            warn!("Stopped reading the command line at an invalid argument: {e}");
        }

        let (qm, panic) = Self::start_runtime(
            mod_dir,
            cli_vanilla,
            launch_config,
            launch_config_error,
            effective_dump_all,
            loadbuffer,
        );
        let qm = QuarterMaster { debug, dev, ..qm };
        let qm = RUNTIME.get_or_init(|| qm);
        if let Some(message) = &panic {
            let _ = guard::guarded(|| qm.record_degrade(message));
        }

        info!(
            "QuarterMaster initialized in {:.2}ms",
            start.elapsed().as_secs_f64() * 1000.0
        );

        qm
    }

    /// The init steps after the logger: loads the mods (none when `launch_config_error` is
    /// set, D65), writes the first report and builds the runtime. Also returns the panic
    /// caught while loading, if any (D34).
    fn start_runtime(
        mod_dir: PathBuf,
        cli_vanilla: bool,
        launch_config: LaunchConfig,
        launch_config_error: Option<String>,
        dump_all: bool,
        loadbuffer: &'static LoadBuffer,
    ) -> (Self, Option<String>) {
        // A broken launch-config.json is never read as vanilla: it runs without mods and
        // keeps the builtins (D65).
        let is_vanilla = cli_vanilla || launch_config.vanilla_mode;
        if let Some(e) = &launch_config_error {
            error!("Running without mods: {e}");
        }
        if is_vanilla {
            info!("Vanilla mode enabled, ignoring patches.");
        }
        let mut loaded = load_for_init(
            &mod_dir,
            is_vanilla,
            launch_config.profile_id.clone(),
            launch_config_error,
        );
        loaded.report.run_id = launch_config.run_id.clone();
        let _ = loaded.report.save_to_mod_dir(&mod_dir);
        let runtime = match loaded.prepared {
            Some(prep) => Runtime::with_prepared(loaded.table, prep),
            None => Runtime::new(loaded.table),
        };
        let qm = QuarterMaster {
            mod_dir,
            is_vanilla,
            launch_config,
            loadbuffer,
            runtime: Arc::new(RwLock::new(runtime)),
            dump_all: std::sync::atomic::AtomicBool::new(dump_all),
            debug: false,
            dev: false,
            lua_vars: Arc::new(RwLock::new(loaded.lua_vars)),
            patch_report: Arc::new(RwLock::new(loaded.report)),
            debug_by_file: Arc::new(RwLock::new(HashMap::new())),
            degraded: AtomicBool::new(loaded.panic.is_some()),
            degraded_notice_shown: AtomicBool::new(false),
            load_errors_shown: AtomicBool::new(false),
        };
        (qm, loaded.panic)
    }

    /// Writes the patched text of every game Lua file to `qm/dump`, loaded yet or not, with
    /// the same sidecar the load-time dump writes. Returns the file count and the folder.
    pub fn dump_patched_files(&self) -> Result<(usize, PathBuf), String> {
        let runtime = self.runtime.read().unwrap_or_else(PoisonError::into_inner);
        let mut count = 0usize;
        dump::for_each_game_lua(|name, text| {
            let outcome = prepare::dll_transform(
                &runtime.table,
                runtime.prepared(),
                &self.launch_config,
                name,
                text,
                true,
            );
            let (patched, debug) = match outcome {
                DllOutcome::Passthrough => (text.to_string(), None),
                DllOutcome::Prepared(t) => (t, None),
                DllOutcome::Patched { text: t, debug, .. } => (t, Some(debug)),
                DllOutcome::SyntaxError(e) => {
                    warn!("Not dumping {name}: {e}");
                    return;
                }
            };
            write_dump(&self.mod_dir, "dump", name, &patched, debug.as_ref());
            count += 1;
        })?;
        let dir = self.mod_dir.join("qm").join("dump");
        info!("Dumped {count} patched Lua files to {}", dir.display());
        Ok((count, dir))
    }

    /// True once a caught panic turned mods off for this session (D34).
    pub fn is_degraded(&self) -> bool {
        self.degraded.load(Ordering::SeqCst)
    }

    /// Marks the run as a clean exit in patch-report.json, with the run id (D66, D67). The
    /// save is atomic.
    pub fn record_clean_exit(&self) -> Result<(), String> {
        let mut report = self
            .patch_report
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        report.clean_exit = true;
        report.run_id = self.launch_config.run_id.clone();
        report.save_to_mod_dir(&self.mod_dir)
    }

    /// Stores the source that ended up owning each asset slot in patch-report.json (D77).
    /// `text` is one "slot<TAB>source" line per slot; other lines are ignored. The save is
    /// atomic.
    pub fn record_asset_winners(&self, text: &str) -> Result<(), String> {
        let winners: Vec<crate::handshake::AssetWinner> = text
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .filter(|(slot, source)| !slot.trim().is_empty() && !source.trim().is_empty())
            .take(MAX_ASSET_WINNERS)
            .map(|(slot, source)| crate::handshake::AssetWinner {
                slot: slot.trim().to_string(),
                source: source.trim().to_string(),
            })
            .collect();
        let mut report = self
            .patch_report
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        report.asset_winners = winners;
        report.save_to_mod_dir(&self.mod_dir)
    }

    /// The patch that wrote `line` of chunk `source` (`@main.lua`, or a module's
    /// `=[qm <name> "<file>"]`); None for the game's own code.
    fn owner_of_chunk_line(&self, source: &str, line: usize) -> Option<RecordOwner> {
        let name = source.trim_start_matches(['@', '=']);
        if let Some(rest) = name.strip_prefix("[qm ") {
            let (module, file) = rest.split_once(' ')?;
            let file = file.trim_end_matches(']').trim_matches('"');
            return self.module_owner(module, file);
        }
        let map = self
            .debug_by_file
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        map.iter()
            .find(|(k, _)| *k == name || k.ends_with(name) || name.ends_with(k.as_str()))
            .and_then(|(_, dbg)| dbg.owner_of_line(line).cloned())
    }

    /// Reads the lines a diagnostic test run reports and keeps them in patch-report.json:
    /// `G<TAB>name<TAB>type<TAB>value<TAB>source<TAB>line` for a change of a watched global,
    /// `H<TAB>name<TAB>total` for a watched function (-1 when it was not found) and
    /// `C<TAB>name<TAB>source<TAB>line<TAB>calls` for one place that calls it. Each line is
    /// mapped to the patch that wrote it. A later call replaces the earlier result.
    pub fn record_diagnostics(&self, text: &str) -> Result<(), String> {
        use crate::handshake::{GlobalChange, HookCaller, HookCalls};
        let mut trace: Vec<GlobalChange> = Vec::new();
        let mut hooks: Vec<HookCalls> = Vec::new();
        for row in text.lines().take(MAX_DIAGNOSTIC_LINES) {
            let cols: Vec<&str> = row.split('\t').collect();
            match cols.as_slice() {
                ["G", name, value_type, identity, source, line] => {
                    let line: u32 = line.parse().unwrap_or(0);
                    let owner = self.owner_of_chunk_line(source, line as usize);
                    trace.push(GlobalChange {
                        name: (*name).to_string(),
                        patch_id: owner.as_ref().map(|o| o.patch_id.clone()).unwrap_or_default(),
                        mod_id: owner.map(|o| o.mod_id).unwrap_or_default(),
                        source: (*source).to_string(),
                        line,
                        value_type: (*value_type).to_string(),
                        identity: (*identity).to_string(),
                        seq: u32::try_from(trace.len() + 1).unwrap_or(u32::MAX),
                    });
                }
                ["H", name, total] => hooks.push(HookCalls {
                    name: (*name).to_string(),
                    total: total.parse().unwrap_or(0),
                    missing: *total == "-1",
                    callers: Vec::new(),
                }),
                ["C", name, source, line, calls] => {
                    let line: u32 = line.parse().unwrap_or(0);
                    let owner = self.owner_of_chunk_line(source, line as usize);
                    if let Some(h) = hooks.iter_mut().find(|h| h.name == *name) {
                        h.callers.push(HookCaller {
                            source: (*source).to_string(),
                            line,
                            calls: calls.parse().unwrap_or(0),
                            patch_id: owner.as_ref().map(|o| o.patch_id.clone()),
                            mod_id: owner.map(|o| o.mod_id),
                        });
                    }
                }
                _ => {}
            }
        }
        let mut report = self
            .patch_report
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        report.globals_trace = trace;
        report.hook_calls = hooks;
        report.save_to_mod_dir(&self.mod_dir)
    }

    /// Hot reload: loads the Mods folder again and replaces the table and its prepared state
    /// under one write lock. Keeps the old table when any mod fails, as before.
    pub fn reload(&self) -> Result<(), String> {
        // The `Prepared` built for the order dump is kept, as `init` does.
        let (table, prep) =
            PatchTable::load_strict_prepared(&self.mod_dir).map_err(|e| format!("{e:?}"))?;
        let mut runtime = self.runtime.write().unwrap_or_else(PoisonError::into_inner);
        *runtime = Runtime::with_prepared(table, prep);
        Ok(())
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
        let loc_re = Regex::new(r#"([A-Za-z0-9_./\\-]+\.lua):(\d+)"#).ok();
        let (crashing_file, crashing_line) = loc_re
            .as_ref()
            .and_then(|re| re.captures(error_message).or_else(|| re.captures(traceback)))
            .map(|c| {
                (
                    c.get(1)
                        .map(|m| m.as_str().trim_start_matches('@').to_string()),
                    c.get(2).and_then(|m| m.as_str().parse::<usize>().ok()),
                )
            })
            .unwrap_or((None, None));

        let mut suspected_mod_id = None;
        let mut suspected_patch_id = None;
        let mut suspected_patch_name = None;
        let mut suspected_fix_ids = Vec::new();
        let mut suspected_adjusted_copy = false;
        let mut code_snippet = Vec::new();

        if let (Some(ref file_key), Some(line_no)) = (&crashing_file, crashing_line) {
            let dbg_map = self
                .debug_by_file
                .read()
                .unwrap_or_else(PoisonError::into_inner);
            let found = dbg_map.iter().find(|(k, _)| {
                *k == file_key
                    || k.ends_with(file_key.as_str())
                    || file_key.ends_with(k.as_str())
            });

            if let Some((_, file_dbg)) = found {
                if let Some(owner) = file_dbg.owner_of_line(line_no) {
                    suspected_mod_id = Some(owner.mod_id.clone());
                    suspected_patch_id = Some(owner.patch_id.clone());
                    suspected_patch_name = Some(owner.patch_name.clone());
                    // An adjusted copy keeps its own id and fix next to its base mod (D58).
                    suspected_fix_ids = owner.fix_ids.clone();
                    suspected_adjusted_copy = owner.adjusted_copy;
                }

                let lines: Vec<&str> = file_dbg.text.lines().collect();
                let start_l = line_no.saturating_sub(4).max(1);
                let end_l = (line_no + 4).min(lines.len());
                for l in start_l..=end_l {
                    let owner = file_dbg.owner_of_line(l);
                    code_snippet.push(CrashSnippetLine {
                        line_number: l,
                        text: lines
                            .get(l.saturating_sub(1))
                            .copied()
                            .unwrap_or("")
                            .to_string(),
                        is_crash_line: l == line_no,
                        attributed_mod_id: owner.map(|o| o.mod_id.clone()),
                        attributed_patch_id: owner.map(|o| o.patch_id.clone()),
                    });
                }
            }
        }

        // Also check if `[qm mod_name "source.lua"]:line` appeared in traceback
        if suspected_mod_id.is_none() {
            let mod_chunk_re = Regex::new(r#"\[qm\s+([^\s"]+)\s+"([^"]+)"\]:(\d+)"#).ok();
            if let Some(cap) = mod_chunk_re
                .as_ref()
                .and_then(|re| re.captures(error_message).or_else(|| re.captures(traceback)))
            {
                let group = |i: usize| cap.get(i).map_or("", |m| m.as_str());
                // The chunk names the module, not its mod: look the mod up (§6.2).
                if let Some(owner) = self.module_owner(group(1), group(2)) {
                    suspected_mod_id = Some(owner.mod_id);
                    suspected_patch_id = Some(owner.patch_id);
                }
                suspected_patch_name = Some(format!("Module {} ({})", group(1), group(2)));
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
            run_id: self.launch_config.run_id.clone(),
            suspected_fix_ids,
            suspected_adjusted_copy,
        };

        let crash_dir = self.mod_dir.join("qm").join("log").join("crashes");
        let _ = std::fs::create_dir_all(&crash_dir);
        if let Ok(json) = serde_json::to_string_pretty(&report) {
            let _ = std::fs::write(crash_dir.join(format!("{crash_id}.json")), json);
        }
        log::record_crash(&format!("{error_message}\n\n{traceback}"));

        report
    }

    /// The mod and patch of the module a `=[qm <name> "<source>"]` chunk runs (§6.2). None
    /// when the runtime is locked for writing (a crash during a reload) or has no such module.
    /// `try_read`, because a crash report can be written while this thread reads it.
    fn module_owner(&self, name: &str, source: &str) -> Option<RecordOwner> {
        let runtime = match self.runtime.try_read() {
            Ok(r) => r,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        let order = runtime.prepared.get().map(Prepared::order);
        runtime.table.module_owner(order, name, source)
    }

    /// The detour's entry (D34): `apply_buffer_patches` under the panic guard. On a caught
    /// panic, in this order: mods go off for the session (`degraded`), the crash report is
    /// written with no lock held, and the chunk loads with the builtin edits only, computed
    /// in a second guarded call. If that call panics too, or the chunk is not UTF-8, main.lua
    /// and external.lua fail with a syntax error and other chunks load raw.
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // the game's pointers, as for apply_buffer_patches
    pub fn load_buffer_guarded(
        &self,
        state: *mut LuaState,
        buf_ptr: *const u8,
        size: usize,
        name_ptr: *const u8,
        mode_ptr: *const u8,
    ) -> u32 {
        let first =
            guard::guarded(|| self.apply_buffer_patches(state, buf_ptr, size, name_ptr, mode_ptr));
        let message = match first {
            Ok(code) => return code,
            Err(message) => message,
        };

        self.degraded.store(true, Ordering::SeqCst);
        let _ = guard::guarded(|| self.record_degrade(&message));

        let prepared = guard::guarded(|| self.builtin_text(buf_ptr, size, name_ptr))
            .and_then(|text| text);
        self.load_prepared(state, prepared, buf_ptr, size, name_ptr, mode_ptr)
    }

    /// The builtin-only text of a chunk: `Ok(None)` when no builtin edit applies (load it
    /// raw), `Err` when a chunk name or buffer is not UTF-8.
    fn builtin_text(
        &self,
        buf_ptr: *const u8,
        size: usize,
        name_ptr: *const u8,
    ) -> Result<Option<String>, String> {
        #[cfg(test)]
        if PANIC_IN_FALLBACK.with(std::cell::Cell::get) {
            panic!("injected test panic in the builtin-only fallback");
        }
        let name = unsafe { CStr::from_ptr(name_ptr as _) }
            .to_str()
            .map_err(|e| format!("the chunk name is not UTF-8: {e}"))?;
        let buf = unsafe { slice::from_raw_parts(buf_ptr, size) };
        let buf_str =
            std::str::from_utf8(buf).map_err(|e| format!("the chunk is not UTF-8: {e}"))?;
        let prepared = prepare::prepare_buffer(name, buf_str, &self.launch_config);
        Ok(prepared.touched.then_some(prepared.text))
    }

    /// Loads the builtin-only text (D34). Without it, main.lua and external.lua fail with a
    /// syntax error (they never load without their safety edits) and other chunks load raw.
    fn load_prepared(
        &self,
        state: *mut LuaState,
        prepared: Result<Option<String>, String>,
        buf_ptr: *const u8,
        size: usize,
        name_ptr: *const u8,
        mode_ptr: *const u8,
    ) -> u32 {
        match prepared {
            Ok(Some(text)) => (self.loadbuffer)(state, text.as_ptr(), text.len(), name_ptr, mode_ptr),
            Ok(None) => (self.loadbuffer)(state, buf_ptr, size, name_ptr, mode_ptr),
            Err(why) => {
                let name = unsafe { CStr::from_ptr(name_ptr as _) };
                if is_safety_chunk(name.to_bytes()) {
                    let file = name.to_string_lossy().replace('@', "");
                    error!("QuarterMaster could not prepare {file} safely: {why}");
                    unsafe { state.push(format!("QuarterMaster could not prepare {file} safely")) };
                    return LUA_ERRSYNTAX;
                }
                (self.loadbuffer)(state, buf_ptr, size, name_ptr, mode_ptr)
            }
        }
    }

    /// Logs the caught panic and writes the crash report and `dll_degraded` (D34).
    fn record_degrade(&self, message: &str) {
        error!("QuarterMaster caught an internal error and turned mods off for this session: {message}");
        let _ = self.build_and_save_crash_report("rust_panic", message, message);
        let mut report = self
            .patch_report
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if report.dll_degraded.is_none() {
            report.dll_degraded = Some(message.to_string());
        }
        let _ = report.save_to_mod_dir(&self.mod_dir);
    }

    /// Prints a plan error, skipped plan entries, fixes waiting for a mod and the mods
    /// skipped at load to the game console, once (D15, D16, D17, D19, D32).
    fn show_load_errors_once(&self) {
        if self.load_errors_shown.swap(true, Ordering::SeqCst) {
            return;
        }
        for (level, line) in self.load_notices() {
            #[cfg(test)]
            PRINTED.with(|p| p.borrow_mut().push(line.clone()));
            match level {
                Notice::Error => error!("{line}"),
                Notice::Warn => warn!("{line}"),
                Notice::Info => info!("{line}"),
            }
        }
    }

    /// The lines `show_load_errors_once` prints, from the patch report.
    fn load_notices(&self) -> Vec<(Notice, String)> {
        let report = self
            .patch_report
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let mut out = Vec::new();
        if let Some(e) = &report.launch_config_error {
            out.push((
                Notice::Error,
                format!(
                    "QuarterMaster loaded no mods: {e}. The game uses its default save folder. See patch-report.json."
                ),
            ));
        }
        if let Some(e) = &report.plan_error {
            out.push((Notice::Error, format!("QuarterMaster loaded no mods: {e}. See patch-report.json.")));
        }
        for w in &report.plan_warnings {
            out.push((Notice::Warn, format!("QuarterMaster: {w}.")));
        }
        for b in &report.fix_bindings {
            out.push((Notice::Info, format!("QuarterMaster: {}.", b.detail)));
        }
        for r in &report.overridden_rules {
            out.push((Notice::Info, format!("QuarterMaster: patch order: {r}.")));
        }
        for r in &report.suspended_rungs {
            out.push((Notice::Warn, format!("QuarterMaster: {}.", r.reason)));
        }
        for r in &report.invalid_rungs {
            out.push((Notice::Warn, format!("QuarterMaster: {} ({}).", r.detail, r.patch_id)));
        }
        for o in &report.overrides {
            let level = if o.status == "applied" { Notice::Info } else { Notice::Warn };
            out.push((level, format!("QuarterMaster: {}.", o.detail)));
        }
        for e in &report.load_errors {
            // The Mods-folder error (init) has no mod.
            if e.mod_id.is_empty() && e.folder == self.mod_dir.display().to_string() {
                out.push((
                    Notice::Error,
                    format!(
                        "QuarterMaster loaded no mods: {} ({}). See patch-report.json.",
                        e.message, e.folder
                    ),
                ));
                continue;
            }
            let also = if e.skipped_dependents.is_empty() {
                String::new()
            } else {
                format!(" Also skipped, because they require it: {}.", e.skipped_dependents.join(", "))
            };
            out.push((
                Notice::Error,
                format!(
                    "QuarterMaster skipped mod '{}' ({}): {}.{also} See patch-report.json.",
                    e.mod_id, e.folder, e.message
                ),
            ));
        }
        out
    }

    /// The Lua side of the main.lua load: the print override and the `qm` module.
    unsafe fn inject_builtins(&self, state: *mut LuaState, patch_table: &PatchTable) {
        let closure: LuaFunc = sys::override_print;
        state.push(closure);
        sys::lua_setfield(state, sys::LUA_GLOBALSINDEX, c"print".as_ptr());

        patch_table.inject_metadata(state);
    }

    /// Degraded mode (D34): the builtin edits only, no mod patches and no mod modules.
    fn load_builtin_only(
        &self,
        state: *mut LuaState,
        buf_ptr: *const u8,
        size: usize,
        name_ptr: *const u8,
        mode_ptr: *const u8,
    ) -> u32 {
        if !self.degraded_notice_shown.swap(true, Ordering::SeqCst) {
            error!("{DEGRADED_NOTICE}");
        }
        let name = unsafe { CStr::from_ptr(name_ptr as _) }.to_str().unwrap_or_default();
        if prepare::chunk_file(name).is_root(prepare::MAIN_LUA) {
            self.show_load_errors_once();
            let runtime = self.runtime.read().unwrap_or_else(PoisonError::into_inner);
            unsafe { self.inject_builtins(state, &runtime.table) };
        }
        let prepared = self.builtin_text(buf_ptr, size, name_ptr);
        self.load_prepared(state, prepared, buf_ptr, size, name_ptr, mode_ptr)
    }

    /// Apply buffer patches. Intercepts Lua chunks loaded by luaL_loadbuffer / luaL_loadbufferx.
    // Legacy signature: only the detour calls it, with the game's pointers. The text work is
    // in the safe `prepare::dll_transform`; this keeps only the Lua side effects and the reports.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
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

        if self.is_degraded() {
            return self.load_builtin_only(state, buf_ptr, size, name_ptr, mode_ptr);
        }

        let runtime = self.runtime.read().unwrap_or_else(PoisonError::into_inner);
        let patch_table = &runtime.table;

        let c_str = unsafe { CStr::from_ptr(name_ptr as _) };
        let name = match c_str.to_str() {
            Ok(x) => x,
            Err(e) => {
                warn!("The chunk name at {name_ptr:?} contains invalid UTF-8, skipping: {e}");
                return (self.loadbuffer)(state, buf_ptr, size, name_ptr, mode_ptr);
            }
        };

        // Check if main.lua is loading, inject QuarterMaster APIs and module patches
        let file = prepare::chunk_file(name);
        if file.is_root(prepare::MAIN_LUA) {
            self.show_load_errors_once();
            unsafe {
                self.inject_builtins(state, patch_table);

                let module_patches: Vec<_> = runtime
                    .prepared()
                    .order()
                    .order
                    .iter()
                    .filter_map(|&i| patch_table.patches.get(i))
                    .filter_map(|(x, prio, path)| match x {
                        Patch::Module(patch) => Some((patch, *prio, path)),
                        _ => None,
                    })
                    .filter(|(x, _, _)| !x.load_now && !x.disabled)
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

        // The text decision: QuarterMaster's edits, then the mod patches (D1).
        let outcome = prepare::dll_transform(
            patch_table,
            runtime.prepared(),
            &self.launch_config,
            name,
            buf_str,
            self.dump_all.load(std::sync::atomic::Ordering::Relaxed),
        );
        let pretty_name = prepare::pretty_name(name);

        let (patched, debug) = match outcome {
            DllOutcome::Passthrough => {
                return (self.loadbuffer)(state, buf_ptr, size, name_ptr, mode_ptr);
            }
            DllOutcome::Prepared(text) => {
                write_dump(&self.mod_dir, "game-dump", &pretty_name, buf_str, None);
                (text, None)
            }
            DllOutcome::SyntaxError(err_msg) => {
                write_dump(&self.mod_dir, "game-dump", &pretty_name, buf_str, None);
                // The load_now modules run before the text, as they always have.
                let err_msg = match unsafe {
                    patch_table.apply_load_now_modules(runtime.prepared(), name, state)
                } {
                    Ok(()) => err_msg,
                    Err(module_err) => module_err,
                };
                let _ = self.build_and_save_crash_report("lua_syntax", &err_msg, &err_msg);
                unsafe { state.push(err_msg) };
                return LUA_ERRSYNTAX;
            }
            DllOutcome::Patched {
                text: p,
                debug: d,
                records,
                line_records,
            } => {
                write_dump(&self.mod_dir, "game-dump", &pretty_name, buf_str, None);
                if let Err(err_msg) =
                    unsafe { patch_table.apply_load_now_modules(runtime.prepared(), name, state) }
                {
                    let _ = self.build_and_save_crash_report("lua_syntax", &err_msg, &err_msg);
                    unsafe { state.push(err_msg) };
                    return LUA_ERRSYNTAX;
                }

                // Record debug regions for runtime crash attribution
                {
                    let mut dbg_map = self
                        .debug_by_file
                        .write()
                        .unwrap_or_else(PoisonError::into_inner);
                    let owners = records
                        .iter()
                        .map(|r| RecordOwner {
                            mod_id: r.mod_id.clone(),
                            patch_id: r.patch_id.clone(),
                            patch_name: r.patch_name.clone(),
                            fix_ids: r.fix_ids.clone(),
                            adjusted_copy: patch_table.is_adjusted_copy(&r.mod_id, &r.patch_id),
                        })
                        .collect();
                    // Keyed by the canonical path the crash lookup compares.
                    let key = file.game_path().map_or_else(|| pretty_name.clone(), str::to_string);
                    dbg_map.insert(
                        key,
                        FileDebug {
                            text: p.clone(),
                            line_records,
                            owners,
                        },
                    );
                }

                // Update and persist Mods/qm/patch-report.json
                {
                    let mut report = self
                        .patch_report
                        .write()
                        .unwrap_or_else(PoisonError::into_inner);
                    set_order_fields(&mut report, runtime.prepared().report());
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
                        if prepare::is_builtin_match(&rec) {
                            report.builtin_matches += 1;
                        }
                        report.records.push(rec);
                    }
                    let _ = report.save_to_mod_dir(&self.mod_dir);
                }

                (p, Some(d))
            }
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

#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
unsafe extern "C" fn apply_patches(lua_state: *mut LuaState) -> c_int {
    guard::unguarded(|| {
        let buf_name = check_lua_string(lua_state, 1);
        let buf = check_lua_string(lua_state, 2);
        let Some(qm) = RUNTIME.get() else {
            lua_state.push(false);
            lua_state.push("Internal QuarterMaster error: Failed to acquire the QM runtime");
            return 2;
        };
        // Mods are off for the session (D34): the buffer comes back as it is.
        if qm.is_degraded() {
            lua_state.push(buf);
            return 1;
        }
        let result = guard::guarded(|| {
            let runtime = qm.runtime.read().unwrap_or_else(PoisonError::into_inner);
            if runtime.table.needs_patching(&buf_name) {
                runtime
                    .table
                    .apply_prepared_lua(runtime.prepared(), &buf_name, &buf, lua_state)
                    .map(|(patched, _debug, _records)| Some(patched))
            } else {
                Ok(None)
            }
        });
        match result {
            Ok(Ok(Some(patched))) => {
                lua_state.push(patched);
                1
            }
            Ok(Ok(None)) => {
                lua_state.push(buf);
                1
            }
            Ok(Err(err)) => {
                lua_state.push(false);
                lua_state.push(err);
                2
            }
            Err(message) => {
                qm.degraded.store(true, Ordering::SeqCst);
                let _ = guard::guarded(|| qm.record_degrade(&message));
                lua_state.push(false);
                lua_state.push("Internal QuarterMaster error: Failed to acquire the QM runtime");
                2
            }
        }
    })
}

#[cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
impl Target {
    /// Whether this target names chunk `target`: the legacy suffix rule on canonical forms
    /// (D44, D45), so `./main.lua` and `@main.lua` name `main.lua`.
    pub fn can_apply(&self, target: &str) -> bool {
        let target = prepare::canonical_path(target);
        let names = |t: &String| target.ends_with(prepare::canonical_path(t).as_str());
        match self {
            Self::Single(t) => names(t),
            Self::Multi(ts) => ts.iter().any(names),
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
