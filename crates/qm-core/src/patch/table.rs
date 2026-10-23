use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::dump::{ByteDebugEntry, PatchDebug};
use crate::patch::{loader, vars};
use crate::patch::{Patch, Priority};
use crate::sys::{preload_module, LuaFunc, LuaState, LuaTable};
use crop::Rope;
use itertools::Itertools;
use log::*;

/// Structure to manage patch tables for QuarterMaster runtime
pub struct PatchTable {
    pub mod_dir: PathBuf,
    pub targets: HashSet<String>,
    // Unsorted
    pub patches: Vec<(Patch, Priority, PathBuf)>,
    pub vars: HashMap<String, String>,
}

impl Default for PatchTable {
    fn default() -> Self {
        Self {
            mod_dir: PathBuf::new(),
            targets: HashSet::new(),
            patches: Vec::new(),
            vars: HashMap::new(),
        }
    }
}

impl PatchTable {
    /// Load patches from the provided mod directory.
    pub fn load(mod_dir: &Path) -> Result<PatchTable> {
        let new_dir = dunce::canonicalize(mod_dir)
            .with_context(|| format!("Could not resolve mod dir {:?}", mod_dir))?;
        let raw_patches = loader::load_patches_new(&new_dir)?;
        let (patches, targets, vars) = loader::process_patches(raw_patches);

        let table = PatchTable {
            mod_dir: new_dir,
            targets,
            patches,
            vars,
        };

        // Output the full patch execution plan ordered by priority
        table.dump_load_order();

        Ok(table)
    }

    /// Prints all loaded patches in the exact order they will be applied (by priority)
    pub fn dump_load_order(&self) {
        info!("================================================================================");
        info!("QuarterMaster: Patch Execution Plan (Sorted by Priority)");
        info!("================================================================================");

        // If you want higher priority to execute first, use `std::cmp::Reverse(*prio)`
        let sorted = self
            .patches
            .iter()
            .sorted_by_key(|(_, prio, _)| *prio);

        for (order_idx, (patch, prio, path)) in sorted.enumerate() {
            info!(
                "[PLAN] #{:<3} [Priority {:>3}] Mod: {:<20} | {}",
                order_idx + 1,
                prio,
                path.display(),
                patch.summary()
            );
        }
        info!("================================================================================");
    }

    /// Determine if the provided target file / name requires patching.
    pub fn needs_patching(&self, target: &str) -> bool {
        let target = target.strip_prefix('@').unwrap_or(target);
        self.targets.contains(target)
    }

    /// Inject QuarterMaster metadata into the game's Lua runtime.
    /// # Safety
    /// Unsafe due to raw Lua state operations.
    pub unsafe fn inject_metadata(&self, state: *mut LuaState) {
        let mod_dir = self.mod_dir.to_str().unwrap().replace('\\', "/");
        let repo = "https://github.com/mnb-modding/QuarterMaster";

        // Import the functions needed for injection
        use crate::{
            apply_patches, dump_vanilla, get_log_path, getvar, is_dump_all, reload_patches,
            removevar, set_dump_all, setvar,
        };

        let log_path_str = get_log_path().unwrap_or_default();

        let create_meta_table = || {
            LuaTable::new()
                .add_var("name", "QuarterMaster")
                .add_var("short_name", "QM")
                .add_var("repo", repo)
                .add_var("version", env!("CARGO_PKG_VERSION"))
                .add_var("mod_dir", mod_dir.clone())
                .add_var("reload_patches", reload_patches as LuaFunc)
                .add_var("apply_patches", apply_patches as LuaFunc)
                .add_var("set_var", setvar as LuaFunc)
                .add_var("get_var", getvar as LuaFunc)
                .add_var("remove_var", removevar as LuaFunc)
                .add_var("log_path", log_path_str.clone())
                .add_var("dump_vanilla", dump_vanilla as LuaFunc)
                .add_var("dump_game_files", dump_vanilla as LuaFunc)
                .add_var("set_dump_all", set_dump_all as LuaFunc)
                .add_var("is_dump_all", is_dump_all as LuaFunc)
        };

        // 1. Preload primary "qm" module if not already registered
        if !crate::sys::is_module_preloaded(state, "qm") {
            preload_module(state, "qm", create_meta_table());
        }

        // 2. Preload "quartermaster" alias if not already registered
        if !crate::sys::is_module_preloaded(state, "quartermaster") {
            preload_module(state, "quartermaster", create_meta_table());
        }
    }

    /// Apply one or more patches onto the target's buffer.
    /// Returns the patched content and debug info.
    /// # Safety
    /// Unsafe due to internal unchecked usages of raw lua state.
    pub unsafe fn apply_patches(
        &self,
        target: &str,
        buffer: &str,
        lua_state: *mut LuaState,
    ) -> Result<(String, PatchDebug), String> {
        let target = target.strip_prefix('@').unwrap_or(target);

        let module_patches = self
            .patches
            .iter()
            .filter_map(|(x, prio, path)| match x {
                Patch::Module(patch) => Some((patch, *prio, path)),
                _ => None,
            })
            .filter(|(x, _, _)| x.load_now)
            .sorted_by_key(|(_, prio, _)| *prio);

        let copy_patches = self
            .patches
            .iter()
            .filter_map(|(x, prio, path)| match x {
                Patch::Copy(patch) => Some((patch, *prio, path)),
                _ => None,
            })
            .sorted_by_key(|(_, prio, _)| *prio);

        let pattern_and_regex = self
            .patches
            .iter()
            .filter(|(patch, _, _)| matches!(patch, Patch::Pattern(..) | Patch::Regex(..)))
            .map(|(patch, prio, path)| (patch, *prio, path))
            .sorted_by_key(|(_, prio, _)| *prio)
            .collect_vec();

        let mut patch_count = 0;
        let mut rope = Rope::from(buffer);
        let mut byte_entries: Vec<ByteDebugEntry> = Vec::new();

        // Apply module injection patches
        for (patch, prio, path) in module_patches {
            let result = patch.apply(target, lua_state, path);
            if result? {
                info!("Applied [Module: load_now] '{}' from {} (Priority: {}) to '{}'", patch.name, path.display(), prio, target);
                patch_count += 1;
            }
        }

        // Apply copy patches
        for (patch, prio, path) in copy_patches {
            let result = patch.apply(target, &mut rope, path);
            if let Some(entry) = result {
                info!("Applied [Copy] from {} (Priority: {}) to '{}'", path.display(), prio, target);
                for region in &entry.regions {
                    for prev_entry in &mut byte_entries {
                        prev_entry.adjust(region.start, region.delta);
                    }
                }
                patch_count += 1;
                byte_entries.push(entry);
            }
        }

        // Apply pattern and regex patches
        for (patch, prio, path) in pattern_and_regex {
            let result = match patch {
                Patch::Pattern(x) => x.apply(target, &mut rope, path),
                Patch::Regex(x) => x.apply(target, &mut rope, path),
                _ => unreachable!(),
            };

            // ONLY LOG HERE: When the patch actually matched and was applied
            if let Some(entry) = result {
                info!("Applied [{}] from {} (Priority: {}) to '{}'", patch.type_name(), path.display(), prio, target);
                for region in &entry.regions {
                    for prev_entry in &mut byte_entries {
                        prev_entry.adjust(region.start, region.delta);
                    }
                }
                patch_count += 1;
                byte_entries.push(entry);
            }
        }

        let debug = PatchDebug::from_byte_entries(target, byte_entries, &rope);
        let mut patched = rope.to_string();

        if !self.vars.is_empty() {
            vars::apply_var_interp(&mut patched, &self.vars);
        }

        if patch_count == 1 {
            info!("Applied 1 patch to '{target}'");
        } else {
            info!("Applied {patch_count} patches to '{target}'");
        }

        Ok((patched, debug))
    }
}