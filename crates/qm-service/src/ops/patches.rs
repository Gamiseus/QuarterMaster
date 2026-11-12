//! Per-mod patch status from the cached simulation.
use crate::cache::Loadable;
use crate::dto::{ModPatchStatus, PatchState};
use crate::error::QmResult;
use crate::service::Service;
use qm_core::handshake::{PatchExecutionRecord, PatchOutcomeStatus};
use qm_core::patch::table::{skipped_for_requirement, SKIPPED_WITH_MOD_WARNING};
use qm_manager::diagnostics_extra::is_shadowed;
use std::collections::HashSet;

impl Service {
    /// Patches of `mod_id` with their simulated outcome. `Loading` until the simulation is cached.
    pub fn mod_patches_get(&self, mod_id: &str) -> QmResult<Loadable<Vec<ModPatchStatus>>> {
        self.installed_mod(mod_id)?;
        let Some(report) = self.inner.caches.simulation.get() else {
            return Ok(Loadable::Loading);
        };
        let disabled: HashSet<String> = self
            .active_profile()
            .map(|p| p.disabled_patches.into_iter().collect())
            .unwrap_or_default();

        let mut out: Vec<ModPatchStatus> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for file in &report.files_touched {
            for r in file.records.iter().filter(|r| r.mod_id == mod_id) {
                let named = named_id(mod_id, &r.patch_name);
                seen.insert(r.patch_id.clone());
                seen.insert(named.clone());
                let state = if disabled.contains(&r.patch_id) || disabled.contains(&named) {
                    PatchState::Disabled
                } else {
                    state_of(r)
                };
                out.push(ModPatchStatus {
                    patch_id: r.patch_id.clone(),
                    name: Some(r.patch_name.clone()).filter(|n| !n.is_empty()),
                    target_file: r.target_file.clone(),
                    state,
                    conflicts: report
                        .conflicts
                        .iter()
                        .filter(|c| {
                            c.patch_a_id == r.patch_id
                                || c.patch_b_id.as_deref() == Some(r.patch_id.as_str())
                        })
                        .cloned()
                        .collect(),
                    changed_by: r.changed_by.clone(),
                });
            }
        }
        // Disabled by `mod:name` id but absent from the (possibly stale) simulation.
        let prefix = format!("{mod_id}:");
        for id in &disabled {
            if let Some(name) = id.strip_prefix(&prefix) {
                if !seen.contains(id) {
                    out.push(ModPatchStatus {
                        patch_id: id.clone(),
                        name: Some(name.to_string()).filter(|n| !n.is_empty()),
                        target_file: String::new(),
                        state: PatchState::Disabled,
                        conflicts: Vec::new(),
                        changed_by: Vec::new(),
                    });
                }
            }
        }
        out.sort_by(|a, b| {
            a.target_file
                .cmp(&b.target_file)
                .then_with(|| a.patch_id.cmp(&b.patch_id))
        });
        Ok(Loadable::Ready(out))
    }
}

fn named_id(mod_id: &str, name: &str) -> String {
    format!("{mod_id}:{name}")
}

pub(crate) fn state_of(r: &PatchExecutionRecord) -> PatchState {
    if r.status != PatchOutcomeStatus::SkippedDisabled && is_shadowed(r) {
        return PatchState::NotRun;
    }
    match r.status {
        PatchOutcomeStatus::Applied => PatchState::Applied,
        PatchOutcomeStatus::ZeroMatches
        | PatchOutcomeStatus::CountMismatch
        | PatchOutcomeStatus::Aborted => PatchState::Broken,
        PatchOutcomeStatus::SkippedDisabled => PatchState::Disabled,
        // A failed match under a skip policy is broken; a patch skipped only because its mod
        // was skipped (the simulator's exact marker warning), or because a patch it requires
        // did not apply (D64), stays skipped.
        PatchOutcomeStatus::SkippedByPolicy
            if skipped_with_mod(r) || skipped_for_requirement(r) =>
        {
            PatchState::Skipped
        }
        PatchOutcomeStatus::SkippedByPolicy => PatchState::Broken,
    }
}

fn skipped_with_mod(r: &PatchExecutionRecord) -> bool {
    r.warnings.iter().any(|w| w == SKIPPED_WITH_MOD_WARNING)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qm_core::patch::OnFailurePolicy;

    fn record(status: PatchOutcomeStatus, warnings: Vec<String>) -> PatchExecutionRecord {
        PatchExecutionRecord {
            patch_id: "a:1".into(),
            patch_name: "One".into(),
            mod_id: "a".into(),
            source_file: String::new(),
            target_file: "includes/ser.lua".into(),
            patch_type: "pattern".into(),
            priority: 0,
            matched_count: 1,
            expected_count: Some(1),
            on_failure: OnFailurePolicy::Warn,
            status,
            warnings,
            regions: Vec::new(),
            stack_moves: Vec::new(),
            anchoring: None,
            fix_ids: Vec::new(),
            changed_by: Vec::new(),
            suspended: None,
        }
    }

    /// A patch on a file a module replaces does not run in the game (G1); a disabled one
    /// stays disabled.
    #[test]
    fn a_shadowed_patch_does_not_run() {
        let note = vec![format!(
            "{}:gcore: 'includes/ser' from 'gcore' replaces this file, so this patch does not run in the game",
            qm_manager::diagnostics_extra::SHADOWED_BY_MODULE
        )];
        assert_eq!(
            state_of(&record(PatchOutcomeStatus::Applied, note.clone())),
            PatchState::NotRun
        );
        assert_eq!(
            state_of(&record(PatchOutcomeStatus::ZeroMatches, note.clone())),
            PatchState::NotRun
        );
        assert_eq!(
            state_of(&record(PatchOutcomeStatus::SkippedDisabled, note)),
            PatchState::Disabled
        );
        assert_eq!(
            state_of(&record(PatchOutcomeStatus::Applied, Vec::new())),
            PatchState::Applied
        );
    }

    /// A patch skipped because a patch it requires did not apply is skipped, not broken
    /// (D64); a cross-file warning on a failed patch does not make it skipped.
    #[test]
    fn a_patch_waiting_for_its_requirement_is_skipped() {
        let skip = vec!["needs_patch:core:base: needs 'core:base', which did not apply".into()];
        assert_eq!(
            state_of(&record(PatchOutcomeStatus::SkippedByPolicy, skip)),
            PatchState::Skipped
        );
        let warn = vec!["needs_patch:core:base: needs 'core:base', which is turned off; it patches another file, so this patch still runs".into()];
        assert_eq!(
            state_of(&record(PatchOutcomeStatus::SkippedByPolicy, warn)),
            PatchState::Broken
        );
    }
}
