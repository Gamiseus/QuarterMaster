use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::dump::{ByteDebugEntry, PatchDebug};
use crate::handshake::{PatchExecutionRecord, PatchOutcomeStatus};
use crate::patch::{loader, vars};
use crate::patch::{OnFailurePolicy, Patch, Priority};
use crate::sys::{preload_module, LuaFunc, LuaState, LuaTable};
use crop::Rope;
use itertools::Itertools;
use log::*;

/// Structure to manage patch tables for QuarterMaster runtime and offline simulation
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

        let sorted = self.patches.iter().sorted_by_key(|(_, prio, _)| *prio);

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
        self.targets.iter().any(|t| {
            let clean_t = t.trim_start_matches('@');
            t == target
                || clean_t == target
                || (!clean_t.is_empty() && target.ends_with(clean_t))
        })
    }

    /// Inject QuarterMaster metadata into the game's Lua runtime.
    /// # Safety
    /// Unsafe due to raw Lua state operations.
    pub unsafe fn inject_metadata(&self, state: *mut LuaState) {
        let mod_dir = self.mod_dir.to_str().unwrap_or_default().replace('\\', "/");
        let repo = "https://github.com/mnb-modding/QuarterMaster";

        use crate::{
            apply_patches, dump_vanilla, get_log_path, getvar, is_dump_all, record_lua_crash,
            reload_patches, removevar, set_dump_all, setvar,
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
                .add_var("record_lua_crash", record_lua_crash as LuaFunc)
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

    /// Offline patch execution engine (runs without a `*mut LuaState`).
    /// Used by both the offline validator/conflict resolver and the live `winmm.dll` runtime.
    pub fn apply_patches_offline(
        &self,
        target: &str,
        buffer: &str,
    ) -> Result<(String, PatchDebug, Vec<PatchExecutionRecord>), String> {
        let target = target.strip_prefix('@').unwrap_or(target);

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

        let mut patch_count = 0usize;
        let mut rope = Rope::from(buffer);
        let mut byte_entries: Vec<ByteDebugEntry> = Vec::new();
        let mut raw_records: Vec<(usize, PatchExecutionRecord)> = Vec::new();
        let mut skipped_mods: HashSet<String> = HashSet::new();

        // 1. Apply copy patches
        for (patch, prio, path) in copy_patches {
            let mod_id = patch
                .mod_id
                .clone()
                .unwrap_or_else(|| infer_mod_id_from_path(path));
            if skipped_mods.contains(&mod_id) {
                continue;
            }
            if patch.disabled {
                if patch.target.can_apply(target) {
                    raw_records.push((
                        usize::MAX,
                        PatchExecutionRecord {
                            patch_id: patch
                                .id
                                .clone()
                                .unwrap_or_else(|| format!("{mod_id}:copy:disabled")),
                            patch_name: patch
                                .name
                                .clone()
                                .unwrap_or_else(|| "Copy Patch".to_string()),
                            mod_id,
                            source_file: path.display().to_string(),
                            target_file: target.to_string(),
                            patch_type: "copy".to_string(),
                            priority: prio,
                            matched_count: 0,
                            expected_count: Some(1),
                            on_failure: patch.on_failure,
                            status: PatchOutcomeStatus::SkippedDisabled,
                            warnings: vec!["Disabled by user in profile".to_string()],
                            regions: Vec::new(),
                        },
                    ));
                }
                continue;
            }
            if let Some(entry) = patch.apply(target, &mut rope, path) {
                info!(
                    "Applied [Copy] from {} (Priority: {}) to '{}'",
                    path.display(),
                    prio,
                    target
                );
                for region in &entry.regions {
                    for prev_entry in &mut byte_entries {
                        prev_entry.adjust(region.start, region.delta);
                    }
                }
                let entry_idx = byte_entries.len();
                patch_count += 1;
                raw_records.push((
                    entry_idx,
                    PatchExecutionRecord {
                        patch_id: patch
                            .id
                            .clone()
                            .unwrap_or_else(|| format!("{mod_id}:copy:{entry_idx}")),
                        patch_name: patch
                            .name
                            .clone()
                            .unwrap_or_else(|| "Copy Patch".to_string()),
                        mod_id,
                        source_file: path.display().to_string(),
                        target_file: target.to_string(),
                        patch_type: "copy".to_string(),
                        priority: prio,
                        matched_count: entry.matched_count,
                        expected_count: entry.expected_count,
                        on_failure: patch.on_failure,
                        status: PatchOutcomeStatus::Applied,
                        warnings: entry.warnings.clone().unwrap_or_default(),
                        regions: Vec::new(),
                    },
                ));
                byte_entries.push(entry);
            }
        }

        // 2. Apply pattern and regex patches
        for (patch, prio, path) in pattern_and_regex {
            let mod_id = patch
                .mod_id()
                .map(|s| s.to_string())
                .unwrap_or_else(|| infer_mod_id_from_path(path));
            let patch_id = patch
                .id()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{mod_id}:{target}"));
            let patch_name = patch
                .name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| patch_id.clone());
            let policy = patch.on_failure();

            if patch.is_disabled() {
                if patch.can_apply_to_target(target) {
                    raw_records.push((
                        usize::MAX,
                        PatchExecutionRecord {
                            patch_id,
                            patch_name,
                            mod_id,
                            source_file: path.display().to_string(),
                            target_file: target.to_string(),
                            patch_type: patch.type_name().to_lowercase(),
                            priority: prio,
                            matched_count: 0,
                            expected_count: patch.expected_count(),
                            on_failure: policy,
                            status: PatchOutcomeStatus::SkippedDisabled,
                            warnings: vec!["Disabled by user in profile".to_string()],
                            regions: Vec::new(),
                        },
                    ));
                }
                continue;
            }

            if skipped_mods.contains(&mod_id) {
                raw_records.push((
                    usize::MAX,
                    PatchExecutionRecord {
                        patch_id,
                        patch_name,
                        mod_id,
                        source_file: path.display().to_string(),
                        target_file: target.to_string(),
                        patch_type: patch.type_name().to_lowercase(),
                        priority: prio,
                        matched_count: 0,
                        expected_count: None,
                        on_failure: policy,
                        status: PatchOutcomeStatus::SkippedByPolicy,
                        warnings: vec![
                            "Skipped because an earlier patch in this mod failed with skip_mod"
                                .to_string(),
                        ],
                        regions: Vec::new(),
                    },
                ));
                continue;
            }

            let mut candidate_rope = rope.clone();
            let result = match patch {
                Patch::Pattern(x) => x.apply(target, &mut candidate_rope, path),
                Patch::Regex(x) => x.apply(target, &mut candidate_rope, path),
                _ => unreachable!(),
            };

            if let Some(entry) = result {
                let is_zero = entry.matched_count == 0;
                let is_mismatch = entry
                    .expected_count
                    .map(|exp| exp != entry.matched_count)
                    .unwrap_or(false);
                let failed = is_zero || is_mismatch;

                let status = if !failed {
                    PatchOutcomeStatus::Applied
                } else {
                    match policy {
                        OnFailurePolicy::Warn => {
                            if is_zero {
                                PatchOutcomeStatus::ZeroMatches
                            } else {
                                PatchOutcomeStatus::CountMismatch
                            }
                        }
                        OnFailurePolicy::SkipPatch => PatchOutcomeStatus::SkippedByPolicy,
                        OnFailurePolicy::SkipMod => {
                            skipped_mods.insert(mod_id.clone());
                            PatchOutcomeStatus::SkippedByPolicy
                        }
                        OnFailurePolicy::Abort => {
                            let err_msg = format!(
                                "Patch '{}' ({}) from mod '{}' failed on target '{}' with on_failure = abort",
                                patch_name, patch_id, mod_id, target
                            );
                            return Err(err_msg);
                        }
                    }
                };

                // Commit rope changes unless skipped by policy
                let should_commit = !matches!(status, PatchOutcomeStatus::SkippedByPolicy)
                    && !entry.regions.is_empty();
                let entry_idx = if should_commit {
                    rope = candidate_rope;
                    info!(
                        "Applied [{}] '{}' from {} (Priority: {}) to '{}'",
                        patch.type_name(),
                        patch_name,
                        path.display(),
                        prio,
                        target
                    );
                    for region in &entry.regions {
                        for prev_entry in &mut byte_entries {
                            prev_entry.adjust(region.start, region.delta);
                        }
                    }
                    patch_count += 1;
                    let idx = byte_entries.len();
                    byte_entries.push(entry.clone());
                    idx
                } else {
                    let idx = byte_entries.len();
                    byte_entries.push(entry.clone());
                    idx
                };

                raw_records.push((
                    entry_idx,
                    PatchExecutionRecord {
                        patch_id,
                        patch_name,
                        mod_id,
                        source_file: path.display().to_string(),
                        target_file: target.to_string(),
                        patch_type: patch.type_name().to_lowercase(),
                        priority: prio,
                        matched_count: entry.matched_count,
                        expected_count: entry.expected_count,
                        on_failure: policy,
                        status,
                        warnings: entry.warnings.unwrap_or_default(),
                        regions: Vec::new(),
                    },
                ));
            }
        }

        let debug = PatchDebug::from_byte_entries(target, byte_entries, &rope);
        let mut records = Vec::with_capacity(raw_records.len());
        for (entry_idx, mut rec) in raw_records {
            if let Some(dbg_entry) = debug.entries.get(entry_idx) {
                rec.regions = dbg_entry.regions.clone();
            }
            records.push(rec);
        }

        let mut patched = rope.to_string();

        if !self.vars.is_empty() {
            vars::apply_var_interp(&mut patched, &self.vars);
        }

        if patch_count == 1 {
            info!("Applied 1 patch to '{target}'");
        } else {
            info!("Applied {patch_count} patches to '{target}'");
        }

        Ok((patched, debug, records))
    }

    /// Apply one or more patches onto the target's buffer at runtime (including `load_now` modules).
    /// Returns the patched content, debug info, and execution records.
    /// # Safety
    /// Unsafe due to internal unchecked usages of raw lua state.
    pub unsafe fn apply_patches(
        &self,
        target: &str,
        buffer: &str,
        lua_state: *mut LuaState,
    ) -> Result<(String, PatchDebug, Vec<PatchExecutionRecord>), String> {
        let target = target.strip_prefix('@').unwrap_or(target);

        let module_patches = self
            .patches
            .iter()
            .filter_map(|(x, prio, path)| match x {
                Patch::Module(patch) => Some((patch, *prio, path)),
                _ => None,
            })
            .filter(|(x, _, _)| x.load_now && !x.disabled)
            .sorted_by_key(|(_, prio, _)| *prio);

        for (patch, prio, path) in module_patches {
            let result = patch.apply(target, lua_state, path);
            if result? {
                info!(
                    "Applied [Module: load_now] '{}' from {} (Priority: {}) to '{}'",
                    patch.name,
                    path.display(),
                    prio,
                    target
                );
            }
        }

        self.apply_patches_offline(target, buffer)
    }
}

fn infer_mod_id_from_path(path: &Path) -> String {
    path.components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or("unknown")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump::PatchRegion;
    use crate::patch::{InsertPosition, PatternPatch, Target};

    #[test]
    fn simulates_all_mnb_mods_against_decompiled_vanilla_files() {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let mods_dir = workspace_root.join("MnB-Mods");
        let decompile_dir = workspace_root.join("decompile");
        if !mods_dir.is_dir() || !decompile_dir.is_dir() {
            return;
        }

        let table = PatchTable::load(&mods_dir).expect("Failed to load MnB-Mods");
        assert!(
            table.patches.len() >= 50,
            "Expected at least 50 patches across MnB-Mods, got {}",
            table.patches.len()
        );
        let mut total_applied = 0usize;
        let mut zero_match_records = Vec::new();

        for target in &table.targets {
            if target.is_empty() {
                continue;
            }
            let clean = target.trim_start_matches('@');
            let file_path = decompile_dir.join(clean);
            if !file_path.is_file() {
                continue;
            }
            let raw = std::fs::read_to_string(&file_path).unwrap();
            let (_patched, _debug, records) = table
                .apply_patches_offline(clean, &raw)
                .expect("Offline simulation should not abort");

            for rec in records {
                if rec.status == PatchOutcomeStatus::Applied {
                    total_applied += 1;
                } else if rec.status == PatchOutcomeStatus::ZeroMatches {
                    zero_match_records.push(format!(
                        "{} ({}) on {} [prio {}]",
                        rec.patch_id, rec.patch_name, rec.target_file, rec.priority
                    ));
                }
            }
        }

        assert!(
            zero_match_records.is_empty(),
            "Expected 0 broken anchors across MnB-Mods with new priority order, found: {:?}",
            zero_match_records
        );
        // 52 patches loaded, minus 9 module patches that the offline simulation does not record.
        assert!(total_applied >= 43);
    }

    #[test]
    fn offline_simulation_tracks_matches_and_regions() {
        let mut table = PatchTable::default();
        table.targets.insert("main.lua".to_string());

        let p1 = Patch::Pattern(PatternPatch {
            pattern: "local x = 1".to_string(),
            position: InsertPosition::After,
            target: Target::Single("main.lua".to_string()),
            payload: "local y = 2".to_string(),
            match_indent: true,
            times: Some(1),
            overwrite: false,
            id: Some("mod_a:main_lua:1".to_string()),
            name: Some("Insert y".to_string()),
            description: None,
            requires: Vec::new(),
            priority: Some(10),
            before: Vec::new(),
            after: Vec::new(),
            on_failure: OnFailurePolicy::Warn,
            mod_id: Some("mod_a".to_string()),
            disabled: false,
        });
        table.patches.push((p1, 10, PathBuf::from("ModA/qm.toml")));

        let src = "function test()\n  local x = 1\n  return x\nend\n";
        let (patched, debug, records) = table.apply_patches_offline("main.lua", src).unwrap();

        assert!(patched.contains("  local y = 2\n"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, PatchOutcomeStatus::Applied);
        assert_eq!(records[0].matched_count, 1);
        assert_eq!(records[0].regions[0], PatchRegion { start_line: 3, end_line: 3 });
        assert!(debug.find_entry_at_line(3).is_some());
    }

    #[test]
    fn offline_simulation_records_disabled_patch_as_skipped_disabled() {
        let mut table = PatchTable::default();
        table.targets.insert("main.lua".to_string());

        let p1 = Patch::Pattern(PatternPatch {
            pattern: "local x = 1".to_string(),
            position: InsertPosition::After,
            target: Target::Single("main.lua".to_string()),
            payload: "local y = 2".to_string(),
            match_indent: true,
            times: Some(1),
            overwrite: false,
            id: Some("mod_a:main_lua:disabled_test".to_string()),
            name: Some("Disabled Insert y".to_string()),
            description: None,
            requires: Vec::new(),
            priority: Some(10),
            before: Vec::new(),
            after: Vec::new(),
            on_failure: OnFailurePolicy::Warn,
            mod_id: Some("mod_a".to_string()),
            disabled: true,
        });
        table.patches.push((p1, 10, PathBuf::from("ModA/qm.toml")));

        let src = "function test()\n  local x = 1\n  return x\nend\n";
        let (patched, debug, records) = table.apply_patches_offline("main.lua", src).unwrap();

        // The buffer must remain completely unmodified
        assert_eq!(patched, src);
        // The disabled patch must be recorded with SkippedDisabled status so UI cards stay visible
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, PatchOutcomeStatus::SkippedDisabled);
        assert_eq!(records[0].matched_count, 0);
        assert!(debug.entries.is_empty());
    }
}
