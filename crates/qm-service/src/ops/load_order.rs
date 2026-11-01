//! Load order, simulation and conflict operations, plus the `WarmVanilla` and `Simulation` jobs.
use crate::cache::{Loadable, VanillaFiles};
use crate::dto::{FileDiff, LoadOrderUpdate, SimulationOverview};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::jobs::{JobCtx, JobId, JobKind};
use crate::service::Service;
use qm_core::patch::table::PatchTable;
use qm_manager::{
    resolve_load_order, simulate_and_analyze_patches, ConflictResolutionSuggestion,
    LoadOrderResolution, ModProfile, SimulationReport,
};
use std::collections::HashMap;
use std::sync::Arc;

impl Service {
    pub fn load_order_get(&self) -> Loadable<LoadOrderResolution> {
        if self.inner.caches.mods.get().is_none() {
            return Loadable::Loading;
        }
        match self.active_profile() {
            Ok(p) => Loadable::Ready(self.resolve_for(&p)),
            Err(_) => Loadable::Ready(LoadOrderResolution::default()),
        }
    }

    pub fn simulation_get(&self) -> Loadable<SimulationOverview> {
        let Some(r) = self.inner.caches.simulation.get() else {
            return Loadable::Loading;
        };
        Loadable::Ready(SimulationOverview {
            total_patches: to_u32(r.total_patches),
            applied_patches: to_u32(r.applied_patches),
            broken_anchors: to_u32(r.broken_anchors),
            conflicts: r.conflicts.clone(),
            files: r
                .files_touched
                .iter()
                .map(|f| f.target_file.clone())
                .collect(),
        })
    }

    pub fn file_diff_get(&self, target_file: &str) -> QmResult<Loadable<FileDiff>> {
        let Some(r) = self.inner.caches.simulation.get() else {
            return Ok(Loadable::Loading);
        };
        let result = r
            .files_touched
            .iter()
            .find(|f| f.target_file == target_file)
            .cloned()
            .ok_or_else(|| {
                QmError::new(
                    ErrorKind::NotFound,
                    format!("No patches touch '{target_file}'."),
                )
            })?;
        let original_lines = self
            .inner
            .caches
            .vanilla
            .get()
            .and_then(|v| {
                v.files
                    .get(target_file)
                    .map(|s| s.lines().map(String::from).collect())
            })
            .unwrap_or_default();
        Ok(Loadable::Ready(FileDiff {
            result,
            original_lines,
        }))
    }

    pub fn load_order_set(&self, update: LoadOrderUpdate) -> QmResult<LoadOrderResolution> {
        self.ensure_not_running()?;
        let mut p = self.active_profile()?;
        p.mod_priorities = update.mod_priorities;
        p.user_load_after = update.user_load_after;
        p.locked_mods = update.locked_mods;
        p.separators = update.separators;
        let p = self.commit_active_profile(p)?;
        Ok(self.resolve_for(&p))
    }

    pub fn patch_set_enabled(&self, patch_id: &str, enabled: bool) -> QmResult<()> {
        self.ensure_not_running()?;
        let mut p = self.active_profile()?;
        // A patch can be listed by its record id or its `<mod>:<name>` id; treat both as one.
        let aliases = self.patch_aliases(patch_id);
        if enabled {
            p.disabled_patches.retain(|x| !aliases.contains(x));
        } else if !p.disabled_patches.iter().any(|x| aliases.contains(x)) {
            p.disabled_patches.push(patch_id.to_string());
        }
        self.commit_active_profile(p).map(|_| ())
    }

    /// `patch_id` plus the other id form of every simulated record it names.
    fn patch_aliases(&self, patch_id: &str) -> Vec<String> {
        let mut out = vec![patch_id.to_string()];
        let Some(report) = self.inner.caches.simulation.get() else {
            return out;
        };
        for r in report.files_touched.iter().flat_map(|f| &f.records) {
            let named = format!("{}:{}", r.mod_id, r.patch_name);
            let other = if r.patch_id == patch_id {
                named
            } else if named == patch_id {
                r.patch_id.clone()
            } else {
                continue;
            };
            if !out.contains(&other) {
                out.push(other);
            }
        }
        out
    }

    /// Apply one resolver suggestion to the active profile. `SelectWinner` needs `winner_index`
    /// (0 keeps patch A, 1 keeps patch B).
    pub fn conflict_resolve(
        &self,
        suggestion: ConflictResolutionSuggestion,
        winner_index: Option<u32>,
    ) -> QmResult<()> {
        if let ConflictResolutionSuggestion::SelectWinner { .. } = suggestion {
            match winner_index {
                Some(0 | 1) => {}
                Some(_) => {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        "winner_index must be 0 or 1.",
                    ))
                }
                None => {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        "Choose which patch wins.",
                    ))
                }
            }
        }
        self.ensure_not_running()?;
        let mut p = self.active_profile()?;
        match suggestion {
            ConflictResolutionSuggestion::SwapModOrder {
                move_mod_id,
                before_mod_id,
                suggested_priority,
                ..
            } => {
                p.mod_priorities
                    .insert(move_mod_id.clone(), suggested_priority);
                let after = p.user_load_after.entry(before_mod_id).or_default();
                if !after.contains(&move_mod_id) {
                    after.push(move_mod_id);
                }
            }
            ConflictResolutionSuggestion::GenerateGluePatch { glue_patch, .. } => {
                p.glue_patches.retain(|g| g.id != glue_patch.id);
                p.glue_patches.push(glue_patch);
            }
            ConflictResolutionSuggestion::SelectWinner {
                patch_a_id,
                patch_b_id,
                ..
            } => {
                let loser = if winner_index == Some(0) {
                    patch_b_id
                } else {
                    patch_a_id
                };
                if !p.disabled_patches.contains(&loser) {
                    p.disabled_patches.push(loser);
                }
            }
        }
        self.commit_active_profile(p).map(|_| ())
    }

    /// Request the `WarmVanilla` job: extract and load the vanilla Lua files, then simulate.
    pub fn warm_vanilla(&self) -> JobId {
        self.refresh(JobKind::WarmVanilla)
    }

    /// One `WarmVanilla` pass. Reads the game path fresh, so a pass requested after a settings
    /// change always sees the new path.
    pub(crate) fn warm_vanilla_pass(&self, ctx: &JobCtx) -> QmResult<Option<String>> {
        let gen = self.generations().vanilla;
        let settings = self.paths().load_settings();
        // Never fall back to a `decompile/` folder next to the working directory.
        let loaded: QmResult<VanillaFiles> = match settings.game_exe_path.as_deref() {
            None => Ok(empty_vanilla()),
            Some(exe) => (|| {
                ctx.progress(0.2, "Reading game files");
                let info = self
                    .paths()
                    .ensure_vanilla_cache(exe)
                    .or_kind(ErrorKind::Io)?;
                let (game_version, files) = self
                    .paths()
                    .load_vanilla_lua_files(Some(exe))
                    .or_kind(ErrorKind::Io)?;
                Ok(VanillaFiles {
                    game_version,
                    info: Some(info),
                    files,
                })
            })(),
        };
        // Store something either way so `app_info` stops reporting Loading.
        let (vanilla, result) = match loaded {
            Ok(v) => (v, Ok(None)),
            Err(e) => (empty_vanilla(), Err(e)),
        };
        let stored = self.with_gens(|g| {
            if g.vanilla != gen {
                return false;
            }
            self.inner.caches.vanilla.set(Arc::new(vanilla));
            // Any simulation computed before this point used other vanilla files.
            self.inner.caches.simulation.clear();
            g.sim = g.sim.wrapping_add(1);
            true
        });
        if stored {
            self.notify(Scope::Settings);
            self.schedule_simulation();
        }
        // Not stored: settings changed mid-pass and a newer pass is already requested.
        result
    }

    /// Request the `Simulation` job for the active profile's staged `Mods` folder.
    pub(crate) fn schedule_simulation(&self) {
        self.refresh(JobKind::Simulation);
    }

    /// One `Simulation` pass; emits `Invalidated { LoadOrder }` once the report is cached.
    pub(crate) fn simulation_pass(&self, ctx: &JobCtx) -> QmResult<Option<String>> {
        let gen = self.generations().sim;
        let Some(vanilla) = self.inner.caches.vanilla.get() else {
            // WarmVanilla requests a simulation once it has stored the files.
            return Ok(None);
        };
        ctx.progress(0.1, "Simulating patches");
        let (report, err) = self.simulate(&vanilla);
        let stored = self.with_gens(|g| {
            if g.sim != gen {
                return false;
            }
            self.inner.caches.simulation.set(Arc::new(report));
            true
        });
        if stored {
            self.notify(Scope::LoadOrder);
        }
        // Not stored: inputs changed mid-pass and a newer pass is already requested.
        err.map_or(Ok(None), Err)
    }

    /// Report for the active profile. Patch load errors yield an empty report plus the error.
    fn simulate(&self, vanilla: &VanillaFiles) -> (SimulationReport, Option<QmError>) {
        let active = self.paths().load_settings().active_profile_id;
        let mods_dir = self.paths().profiles_dir.join(active).join("Mods");
        if !mods_dir.is_dir() {
            // Never staged: nothing to simulate.
            return (empty_report(), None);
        }
        match PatchTable::load(&mods_dir) {
            Ok(table) => (simulate_and_analyze_patches(&table, &vanilla.files), None),
            Err(e) => (
                empty_report(),
                Some(
                    QmError::new(
                        ErrorKind::InvalidInput,
                        "Could not load the profile's patches.",
                    )
                    .with_details(format!("{e:#}")),
                ),
            ),
        }
    }

    fn resolve_for(&self, p: &ModProfile) -> LoadOrderResolution {
        let mods = self.installed_mods();
        let manifests: Vec<_> = mods
            .iter()
            .filter(|m| p.enabled_mods.contains(&m.manifest.id))
            .map(|m| m.manifest.clone())
            .collect();
        let game_version = self
            .inner
            .caches
            .vanilla
            .get()
            .and_then(|v| v.info.as_ref().map(|i| i.game_version.clone()));
        resolve_load_order(
            &manifests,
            &p.mod_priorities,
            &p.user_load_after,
            game_version.as_deref(),
        )
    }
}

fn empty_vanilla() -> VanillaFiles {
    VanillaFiles {
        game_version: String::new(),
        info: None,
        files: HashMap::new(),
    }
}

fn empty_report() -> SimulationReport {
    SimulationReport {
        files_touched: Vec::new(),
        conflicts: Vec::new(),
        module_registrations: Default::default(),
        total_patches: 0,
        applied_patches: 0,
        broken_anchors: 0,
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
