//! Per-mod patch status from the cached simulation.
use crate::cache::Loadable;
use crate::dto::{ModPatchStatus, PatchState};
use crate::error::QmResult;
use crate::service::Service;
use qm_core::handshake::{PatchExecutionRecord, PatchOutcomeStatus};
use qm_core::patch::table::SKIPPED_WITH_MOD_WARNING;
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

fn state_of(r: &PatchExecutionRecord) -> PatchState {
    match r.status {
        PatchOutcomeStatus::Applied => PatchState::Applied,
        PatchOutcomeStatus::ZeroMatches
        | PatchOutcomeStatus::CountMismatch
        | PatchOutcomeStatus::Aborted => PatchState::Broken,
        PatchOutcomeStatus::SkippedDisabled => PatchState::Disabled,
        // A failed match under a skip policy is broken; a patch skipped only because its mod
        // was skipped (the simulator's exact marker warning) stays skipped.
        PatchOutcomeStatus::SkippedByPolicy if skipped_with_mod(r) => PatchState::Skipped,
        PatchOutcomeStatus::SkippedByPolicy => PatchState::Broken,
    }
}

fn skipped_with_mod(r: &PatchExecutionRecord) -> bool {
    r.warnings.iter().any(|w| w == SKIPPED_WITH_MOD_WARNING)
}
