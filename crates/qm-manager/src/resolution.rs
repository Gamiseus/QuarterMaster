//! The check a conflict resolution goes through before it is applied (D63): the profile is
//! simulated with the change and compared with the simulation without it. Shared by the
//! service and the legacy app, which depends on qm-manager but not on qm-service.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use qm_core::compile::LuaCompiler;
use qm_core::handshake::LoadOrderPlan;
use qm_core::patch::edits::ApplyAs;
use qm_core::patch::plan::{self, FpMode, RawLoad};
use qm_core::patch::table::PatchTable;
use qm_schema::ModManifest;

use crate::conflict::{
    simulate_and_analyze_patches_with, ConflictResolutionSuggestion, SimulationOptions,
    SimulationReport,
};
use crate::fixes::{self, FixSource, Provenance};
use crate::profiles::{build_plan_for, ModProfile};
use crate::storage::QuarterMasterPaths;

/// What a resolution would change besides the conflict it targets.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolutionImpact {
    /// Game files whose patched text would change.
    pub files_changed: Vec<String>,
    /// Ids of conflicts the change would remove.
    pub resolved: Vec<String>,
    /// Files that compile now and would not after the change.
    pub compile_errors_new: Vec<String>,
}

/// What `check_resolution` found.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolutionCheck {
    pub impact: ResolutionImpact,
    /// The change would stop a file from compiling.
    pub breaks_file: bool,
    /// Ids of conflicts the change would add.
    pub new_conflicts: Vec<String>,
}

impl ResolutionCheck {
    /// Why the change must not be applied: it breaks a file or adds a conflict. None when it
    /// may be applied.
    pub fn refusal(&self) -> Option<String> {
        if self.breaks_file {
            return Some(format!(
                "The fix was not applied: it would stop {} from compiling.",
                self.impact.compile_errors_new.join(", ")
            ));
        }
        match self.new_conflicts.len() {
            0 => None,
            1 => Some(format!(
                "The fix was not applied: it would add a new conflict ({}).",
                self.new_conflicts[0]
            )),
            n => Some(format!(
                "The fix was not applied: it would add {n} new conflicts ({}).",
                self.new_conflicts.join(", ")
            )),
        }
    }
}

/// Compares the simulation without the change (`current`) with the one with it (`next`).
pub fn compare_resolution(current: &SimulationReport, next: &SimulationReport) -> ResolutionCheck {
    // Info notes (`multi_match`) are not conflicts. Semantic findings (D69) are approximate
    // warnings: they never refuse a fix.
    let ids = |r: &SimulationReport| -> BTreeSet<String> {
        r.conflicts
            .iter()
            .filter(|c| c.severity != "info" && !crate::semantic::is_semantic(c))
            .map(|c| c.id.clone())
            .collect()
    };
    let broken = |r: &SimulationReport| -> BTreeSet<String> {
        r.conflicts
            .iter()
            .filter(|c| c.kind == "compile_error")
            .map(|c| c.target_file.clone())
            .collect()
    };
    let (was, will) = (ids(current), ids(next));
    let compile_errors_new: Vec<String> =
        broken(next).difference(&broken(current)).cloned().collect();
    let files: BTreeSet<&String> = current.traces.keys().chain(next.traces.keys()).collect();
    let files_changed = files
        .into_iter()
        .filter(|f| current.traces.get(*f).map(|t| &t.text) != next.traces.get(*f).map(|t| &t.text))
        .cloned()
        .collect();
    ResolutionCheck {
        breaks_file: !compile_errors_new.is_empty(),
        new_conflicts: will.difference(&was).cloned().collect(),
        impact: ResolutionImpact {
            files_changed,
            resolved: was.difference(&will).cloned().collect(),
            compile_errors_new,
        },
    }
}

/// Writes `suggestion` into the profile's own fields, as the legacy app always has; the
/// launcher adopts them into the ledger. `ApplyAs` and `SkipHunk` have no such field: they
/// are made as fixes (D46, D49), which needs a migrated profile; `SkipHunk` also needs `raw`
/// (the profile's mods) to key the edit. `winner` is the patch a `SelectWinner` keeps (0 for
/// A, 1 for B).
pub fn apply_suggestion(
    profile: &mut ModProfile,
    suggestion: &ConflictResolutionSuggestion,
    winner: usize,
    raw: Option<&RawLoad>,
) -> Result<(), String> {
    match suggestion {
        ConflictResolutionSuggestion::SwapModOrder {
            move_mod_id,
            before_mod_id,
            suggested_priority,
            ..
        } => {
            profile
                .mod_priorities
                .insert(move_mod_id.clone(), *suggested_priority);
            profile
                .user_load_after
                .entry(before_mod_id.clone())
                .or_default()
                .push(move_mod_id.clone());
        }
        ConflictResolutionSuggestion::GenerateGluePatch { glue_patch, .. } => {
            profile.glue_patches.push(glue_patch.clone());
        }
        ConflictResolutionSuggestion::SelectWinner {
            patch_a_id,
            patch_b_id,
            ..
        } => {
            let loser = if winner == 0 { patch_b_id } else { patch_a_id };
            if !profile.disabled_patches.contains(loser) {
                profile.disabled_patches.push(loser.clone());
            }
        }
        // The profile fields have no place for these: they are fixes.
        ConflictResolutionSuggestion::ApplyAs { patch_id, .. } => {
            let provenance = Provenance::new(FixSource::Resolver);
            fixes::op_apply_as_set(profile, raw, patch_id, ApplyAs::Edits, None, provenance)
                .ok_or_else(|| NOT_IMPORTED.to_string())?;
        }
        ConflictResolutionSuggestion::SkipHunk {
            patch_id,
            ordinal,
            keep_patch_id,
            ..
        } => {
            if profile.fixes_version < fixes::FIXES_VERSION {
                return Err(NOT_IMPORTED.to_string());
            }
            let raw = raw.ok_or_else(|| "The profile's mods could not be read.".to_string())?;
            let hunk = fixes::hunk_key(raw, patch_id, *ordinal, Some(keep_patch_id.clone()))?;
            let provenance = Provenance::new(FixSource::Resolver);
            fixes::op_hunk_skip(profile, Some(raw), patch_id, hunk, None, provenance)
                .ok_or_else(|| NOT_IMPORTED.to_string())?;
        }
    }
    Ok(())
}

/// Why an `ApplyAs` or `SkipHunk` suggestion cannot be applied to a profile whose earlier
/// fixes are not imported yet (activation imports them).
pub const NOT_IMPORTED: &str =
    "This profile's fixes are not imported yet. Activate the profile, then try again.";

/// Simulates `profile` with `suggestion` applied and compares it with `profile` as it is. Both
/// are compiled the way activation compiles them, over the profile's staged Mods folder, and
/// compiled with the game's Lua when its lua51.dll is found (D63).
pub fn check_resolution(
    paths: &QuarterMasterPaths,
    profile: &ModProfile,
    suggestion: &ConflictResolutionSuggestion,
    winner: usize,
) -> Result<ResolutionCheck, String> {
    check_change(paths, profile, suggestion, winner).map(|(check, _)| check)
}

/// `check_resolution`, with the profile the change makes.
fn check_change(
    paths: &QuarterMasterPaths,
    profile: &ModProfile,
    suggestion: &ConflictResolutionSuggestion,
    winner: usize,
) -> Result<(ResolutionCheck, ModProfile), String> {
    let settings = paths.load_settings();
    let (_, vanilla) = paths
        .load_vanilla_lua_files(settings.game_exe_path.as_deref())
        .map_err(|e| format!("The fix could not be checked: {e}"))?;
    let manifests: Vec<ModManifest> = paths
        .list_installed_mods()
        .into_iter()
        .map(|m| m.manifest)
        .collect();
    let mod_dir = paths.profiles_dir.join(&profile.id).join("Mods");
    let compiler = settings
        .game_exe_path
        .as_deref()
        .and_then(Path::parent)
        .and_then(LuaCompiler::find)
        .and_then(|dll| LuaCompiler::load(&dll).ok());
    let options = SimulationOptions {
        compiler: compiler.as_ref(),
        launch: None,
    };
    let raw = raw_for(profile, &manifests, &mod_dir)?;
    let mut changed = profile.clone();
    apply_suggestion(&mut changed, suggestion, winner, Some(&raw))?;
    let current = simulate_profile(profile, &raw, &manifests, &vanilla, options)?;
    let next_raw = raw_for(&changed, &manifests, &mod_dir)?;
    let next = simulate_profile(&changed, &next_raw, &manifests, &vanilla, options)?;
    Ok((compare_resolution(&current, &next), changed))
}

/// The legacy app's resolve: checks `suggestion` on the active profile, then saves and stages
/// the profile with it. When the check refuses, nothing is written.
pub fn resolve_conflict(
    paths: &QuarterMasterPaths,
    suggestion: &ConflictResolutionSuggestion,
    winner: usize,
) -> Result<ResolutionCheck, String> {
    let settings = paths.load_settings();
    let profile = paths.get_profile(&settings.active_profile_id)?;
    // The checked profile is the one saved, so a fix keeps the id the check gave it.
    let (check, profile) = check_change(paths, &profile, suggestion, winner)?;
    if let Some(why) = check.refusal() {
        return Err(why);
    }
    paths.save_profile(&profile)?;
    paths.activate_profile(&profile.id)?;
    Ok(check)
}

/// The mods of `mod_dir` as activation would read them for `profile`: their priorities come
/// from its plan.
fn raw_for(profile: &ModProfile, manifests: &[ModManifest], mod_dir: &Path) -> Result<RawLoad, String> {
    if !mod_dir.is_dir() {
        return Ok(RawLoad::default());
    }
    let plan = build_plan_for(profile, manifests);
    plan::load_raw(mod_dir, Some(&plan))
        .map_err(|e| format!("The fix could not be checked: the profile's mods could not be read: {e:#}"))
}

/// The simulation of `profile` as activation would stage it, over its mods `raw`.
fn simulate_profile(
    profile: &ModProfile,
    raw: &RawLoad,
    manifests: &[ModManifest],
    vanilla: &HashMap<String, String>,
    options: SimulationOptions<'_>,
) -> Result<SimulationReport, String> {
    let plan = build_plan_for(profile, manifests);
    // Through JSON, as the game reads the plan.
    let json = serde_json::to_string(&plan).map_err(|e| format!("The fix could not be checked: {e}"))?;
    let applied = plan::apply_plan_with(raw, &LoadOrderPlan::parse(&json), FpMode::All);
    let table = PatchTable::from_applied(applied, raw.mod_dir.clone());
    Ok(simulate_and_analyze_patches_with(&table, vanilla, options))
}

#[cfg(test)]
#[path = "resolution_tests.rs"]
mod tests;
