//! Load order, simulation and conflict operations, plus the `WarmVanilla` and `Simulation` jobs.
use crate::cache::{Loadable, VanillaFiles};
use crate::dto::{FileDiff, FileStat, LoadOrderUpdate, PatchState, SimulationOverview};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::jobs::{JobCtx, JobId, JobKind};
use crate::service::Service;
use qm_core::compile::LuaCompiler;
use qm_core::patch::edits::ApplyAs;
use qm_core::patch::plan::{FpMode, RawLoad};
use qm_core::patch::table::PatchTable;
use qm_manager::conflict::{ResolutionCheck, ResolutionImpact};
use qm_manager::fixes::{self, FixSource, Provenance};
use qm_manager::{
    resolve_load_order, simulate_and_analyze_patches_with, ConflictResolutionSuggestion,
    LoadOrderResolution, ModProfile, SimulationOptions, SimulationReport,
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
            compile_check: r.compile_check.clone(),
            plan_error: r.plan_error.clone(),
            plan_warnings: r.plan_warnings.clone(),
            load_errors: r.load_errors.clone(),
            fix_bindings: r.fix_bindings.clone(),
            overridden_rules: r.overridden_rules.clone(),
            unknown_ids: r.unknown_ids.clone(),
            suspended_rungs: r.suspended_rungs.clone(),
            invalid_rungs: r.invalid_rungs.clone(),
            overrides: r.overrides.clone(),
            line_rules_change: self.line_rules_change(),
            failure_groups: r.failure_groups.clone(),
            inferred_edges: r.inferred_edges.clone(),
            files: r
                .files_touched
                .iter()
                .map(|f| f.target_file.clone())
                .collect(),
            file_stats: r
                .files_touched
                .iter()
                .map(|f| FileStat {
                    target_file: f.target_file.clone(),
                    original_lines: to_u32(f.original_line_count),
                    patched_lines: to_u32(f.patched_line_count),
                    patches: to_u32(
                        f.records
                            .iter()
                            .map(|x| x.patch_id.as_str())
                            .collect::<std::collections::BTreeSet<_>>()
                            .len(),
                    ),
                    mods: f.mods_touching.clone(),
                    broken: to_u32(
                        f.records
                            .iter()
                            .filter(|x| crate::ops::patches::state_of(x) == PatchState::Broken)
                            .count(),
                    ),
                    // Info notes (`multi_match`) are not conflicts.
                    conflicts: to_u32(
                        r.conflicts
                            .iter()
                            .filter(|c| c.target_file == f.target_file && c.severity != "info")
                            .count(),
                    ),
                })
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
        let (mut p, raw) = self.writable_profile()?;
        // A patch can be listed by its record id or its `<mod>:<name>` id; treat both as one.
        let table = self.inner.caches.patch_table.get();
        self.set_patch_enabled_in(&mut p, raw.as_deref(), table.as_deref(), patch_id, enabled);
        self.commit_active_profile(p).map(|_| ())
    }

    /// `patch_id` plus the other id form of every simulated record it names.
    pub(crate) fn patch_aliases(&self, patch_id: &str) -> Vec<String> {
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
        // Edits-mode and skipped-edit fixes live only in the fix ledger, so a profile whose
        // fixes could not be imported is refused with why.
        let ledger_only = matches!(
            suggestion,
            ConflictResolutionSuggestion::ApplyAs { .. }
                | ConflictResolutionSuggestion::SkipHunk { .. }
        );
        let (mut p, raw) = if ledger_only {
            self.ledger_profile()?
        } else {
            self.writable_profile()?
        };
        let not_imported = || {
            QmError::new(
                ErrorKind::InvalidInput,
                format!("{}.", super::fixes::MSG_IMPORT),
            )
        };
        // Every conflict-driven fix records its conflict, so "Resolved by" finds it.
        let mut provenance = Provenance::new(FixSource::Resolver);
        if let Some((id, kind)) = self.conflict_of(&suggestion) {
            provenance.conflict_id = Some(id);
            provenance.conflict_kind = Some(kind);
        }
        match suggestion {
            ConflictResolutionSuggestion::SwapModOrder {
                move_mod_id,
                before_mod_id,
                suggested_priority,
                ..
            } => {
                let (graph, _) = self.enabled_graph(&p);
                if graph.depends_on(&move_mod_id, &before_mod_id) {
                    let names = self.mod_names();
                    let name = |m: &str| {
                        names
                            .get(&m.to_lowercase())
                            .cloned()
                            .unwrap_or_else(|| m.to_string())
                    };
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "{} needs {} to load first, so it cannot load before it.",
                            name(&move_mod_id),
                            name(&before_mod_id)
                        ),
                    ));
                }
                fixes::op_mod_order(
                    &mut p,
                    raw.as_deref(),
                    &move_mod_id,
                    &before_mod_id,
                    suggested_priority,
                    provenance,
                );
            }
            ConflictResolutionSuggestion::GenerateGluePatch { glue_patch, .. } => {
                fixes::op_glue_add(&mut p, raw.as_deref(), glue_patch, false, provenance);
                // A copy is checked against the simulation first (D63).
                let check = self.check_profile_change(&p)?;
                if let Some(why) = check.refusal() {
                    return Err(QmError::new(ErrorKind::Conflict, why));
                }
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
                let aliases = std::collections::HashSet::from([loser.clone()]);
                fixes::op_disable(&mut p, raw.as_deref(), &loser, &aliases, provenance);
            }
            ConflictResolutionSuggestion::ApplyAs { patch_id, .. } => {
                // As the resolver's ApplyAs draft (D46).
                let table = self.inner.caches.patch_table.get();
                super::fixes::check_apply_as_draft(table.as_deref(), &patch_id)?;
                fixes::op_apply_as_set(
                    &mut p,
                    raw.as_deref(),
                    &patch_id,
                    ApplyAs::Edits,
                    None,
                    provenance,
                )
                .ok_or_else(not_imported)?;
                // Checked against the simulation first, as a copy is (D63).
                let check = self.check_profile_change(&p)?;
                if let Some(why) = check.refusal() {
                    return Err(QmError::new(ErrorKind::Conflict, why));
                }
            }
            ConflictResolutionSuggestion::SkipHunk {
                patch_id,
                ordinal,
                keep_patch_id,
                ..
            } => {
                let raw = raw.as_deref().ok_or_else(|| {
                    QmError::new(ErrorKind::Conflict, "The profile's mods could not be read.")
                })?;
                let hunk = fixes::hunk_key(raw, &patch_id, ordinal, Some(keep_patch_id))
                    .map_err(|e| QmError::new(ErrorKind::InvalidInput, e))?;
                fixes::op_hunk_skip(&mut p, Some(raw), &patch_id, hunk, None, provenance)
                    .ok_or_else(not_imported)?;
                // Checked against the simulation first, as a copy is (D63).
                let check = self.check_profile_change(&p)?;
                if let Some(why) = check.refusal() {
                    return Err(QmError::new(ErrorKind::Conflict, why));
                }
            }
        }
        self.commit_active_profile(p).map(|_| ())
    }

    /// `changed` (the active profile with a resolution in it) through the preview, as
    /// `qm_manager::conflict::check_resolution` judges it: it may not break a file or add a
    /// conflict (D63). Fails when the simulation is not ready.
    fn check_profile_change(&self, changed: &ModProfile) -> QmResult<ResolutionCheck> {
        // A UI preview started meanwhile must not cancel this check.
        let r = self.preview_check(&|_, _, _, _| Ok((changed.clone(), Vec::new())))?;
        Ok(ResolutionCheck {
            impact: ResolutionImpact {
                files_changed: r.impact.files_changed,
                resolved: r.resolved,
                compile_errors_new: r.impact.compile_errors_new,
            },
            breaks_file: r.impact.breaks_file,
            new_conflicts: r.new_conflicts,
        })
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
                // The archive lists every asset; without one, only what the sources name.
                let listing = qm_manager::vanilla_assets::VanillaAssetListing::load_or_build(&info);
                let sources = files.values().map(String::as_str);
                let assets = match &listing {
                    Some(l) => qm_core::patch::loader::VanillaAssetIndex::from_archive_paths(
                        l.paths(),
                        sources,
                    ),
                    None => qm_core::patch::loader::VanillaAssetIndex::from_lua_sources(sources),
                };
                Ok(VanillaFiles {
                    game_version,
                    info: Some(info),
                    files,
                    assets: Some(Arc::new(assets)),
                    listing: listing.map(Arc::new),
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
            self.inner.caches.patch_table.clear();
            self.inner.caches.raw_load.clear();
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
        let writes = self.profile_writes();
        // Test seam (`testing::on_next_simulation_pass`).
        let hook = self
            .inner
            .on_simulation_pass
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(hook) = hook {
            hook();
        }
        let Some(vanilla) = self.inner.caches.vanilla.get() else {
            // WarmVanilla requests a simulation once it has stored the files.
            return Ok(None);
        };
        ctx.progress(0.1, "Simulating patches");
        let (report, table, raw, err) = self.simulate(&vanilla);
        let line_rules = self.line_rules_pass(&table, &vanilla);
        let report = Arc::new(report);
        let stored = self.with_gens(|g| {
            if g.sim != gen {
                return false;
            }
            self.inner.caches.simulation.set(report.clone());
            self.inner
                .caches
                .line_rules
                .set((report.clone(), line_rules.clone()));
            self.inner.caches.patch_table.set(Arc::new(table));
            match raw {
                Some(raw) => self.inner.caches.raw_load.set(Arc::new(raw)),
                None => self.inner.caches.raw_load.clear(),
            }
            true
        });
        if stored {
            self.notify(Scope::LoadOrder);
            // The game files and the mod load are both there now (D74).
            self.stamp_pending_fixes(gen, writes);
            self.refresh_asset_choices(gen, writes);
        }
        // Not stored: inputs changed mid-pass and a newer pass is already requested.
        err.map_or(Ok(None), Err)
    }

    /// The line rules comparison (D51) for the active profile, once per simulation: only when
    /// the profile has no answer and the rules can change its output (never in the pre-launch
    /// or Fixes read paths).
    fn line_rules_pass(
        &self,
        table: &PatchTable,
        vanilla: &VanillaFiles,
    ) -> Option<Arc<qm_manager::line_rules::LineRulesDiff>> {
        let answer = self.active_profile().ok()?.line_rules_v2;
        if !qm_manager::line_rules::needs_comparison(answer, table) {
            return None;
        }
        self.inner
            .caches
            .line_rules_passes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let launch = qm_core::handshake::LaunchConfig::default();
        Some(Arc::new(qm_manager::line_rules::compare(
            table,
            &vanilla.files,
            &launch,
        )))
    }

    /// What the line rules would change in the active profile (D51), when the cached
    /// simulation compared them and the game output differs.
    pub(crate) fn line_rules_change(&self) -> Option<qm_manager::line_rules::LineRulesDiff> {
        let report = self.inner.caches.simulation.get()?;
        let (of, diff) = self.inner.caches.line_rules.get()?;
        if !Arc::ptr_eq(&report, &of) {
            return None;
        }
        diff.filter(|d| d.changes_output()).map(|d| (*d).clone())
    }

    /// Report for the active profile. Like the game, a mod that fails to load is skipped with
    /// its requirers and a bad plan loads no mods; both are in the report (D15, D16). Only an
    /// unreadable Mods folder yields an empty report plus the error.
    #[allow(clippy::type_complexity)]
    fn simulate(
        &self,
        vanilla: &VanillaFiles,
    ) -> (
        SimulationReport,
        PatchTable,
        Option<RawLoad>,
        Option<QmError>,
    ) {
        let mods_dir = self.staged_mods_dir();
        if !mods_dir.is_dir() {
            // Never staged: nothing to simulate.
            return (empty_report(), PatchTable::default(), None, None);
        }
        // Every fingerprint is checked, so drift shows; the patches are the DLL's.
        match PatchTable::load_with_raw_fp(&mods_dir, FpMode::All) {
            Ok((table, raw)) => (self.simulate_table(&table, vanilla), table, raw, None),
            Err(e) => (
                empty_report(),
                PatchTable::default(),
                None,
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

    /// The active profile's staged `Mods` folder.
    pub(crate) fn staged_mods_dir(&self) -> std::path::PathBuf {
        let active = self.paths().load_settings().active_profile_id;
        self.paths().profiles_dir.join(active).join("Mods")
    }

    /// Runs the simulation on `table`, with the compile check when the game's Lua is available.
    pub(crate) fn simulate_table(
        &self,
        table: &PatchTable,
        vanilla: &VanillaFiles,
    ) -> SimulationReport {
        let lua = self.lua_compiler();
        let mut report = simulate_and_analyze_patches_with(
            table,
            &vanilla.files,
            SimulationOptions {
                compiler: lua.as_ref().ok().map(|c| c.as_ref()),
                // The staged launch-config.json, read once per run.
                launch: None,
            },
        );
        if let Err(why) = lua {
            report.compile_check.note = Some(why);
        }
        report
    }

    /// The game's `lua51.dll`, loaded once per path, or why the compile check cannot run.
    pub(crate) fn lua_compiler(&self) -> Result<Arc<LuaCompiler>, String> {
        let dir = self
            .paths()
            .load_settings()
            .game_exe_path
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .ok_or("The game folder is not set, so patched files were not compiled.")?;
        let dll = LuaCompiler::find(&dir)
            .ok_or("The game's lua51.dll was not found, so patched files were not compiled.")?;
        let mut slot = self
            .inner
            .caches
            .lua
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((p, c)) = slot.as_ref() {
            if *p == dll {
                return c.clone();
            }
        }
        let loaded = LuaCompiler::load(&dll)
            .map(Arc::new)
            .map_err(|e| format!("Patched files were not compiled: {e}."));
        *slot = Some((dll, loaded.clone()));
        loaded
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
        assets: None,
        listing: None,
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
        order_diagnostics: Vec::new(),
        traces: Default::default(),
        compile_check: Default::default(),
        plan_error: None,
        plan_warnings: Vec::new(),
        load_errors: Vec::new(),
        fix_bindings: Vec::new(),
        overridden_rules: Vec::new(),
        unknown_ids: Vec::new(),
        suspended_rungs: Vec::new(),
        invalid_rungs: Vec::new(),
        overrides: Vec::new(),
        losses: Vec::new(),
        kept_anchors: Vec::new(),
        runs_as: Vec::new(),
        failure_groups: Vec::new(),
        inferred_edges: Vec::new(),
        save_fields: Default::default(),
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
