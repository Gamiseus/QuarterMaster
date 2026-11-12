//! The patch tools' commands (plan 2d Task 1): patch detail, what-if previews, anchor
//! candidates, glue patches and profile patch order. They read the patch table the cached
//! simulation ran; writes go through the active profile like every other profile edit.
use crate::cache::VanillaFiles;
use crate::dto::{
    ASite, AnchorCandidate, FactionChange, FactionPick, FallbackNotice, FixChangeDraft, FixDraft,
    FixStatus, FixStatusChange, LineSource, PatchChange, PatchDetail, PatchFilePosition,
    PreviewChanges, PreviewImpact, PreviewResult, RegionOption, RegionOptions, RegionRef,
    RegionSite, StackChange,
};
use crate::error::{ErrorKind, QmError, QmResult};
use crate::jobs::JobKind;
use crate::service::Service;
use qm_core::handshake::{GluePatchEntry, LoadOrderPlan, PatchOutcomeStatus, RunAs, StackMode};
use qm_core::patch::edits::ApplyAs;
use qm_core::patch::faction::LoadedFaction;
use qm_core::patch::linemap::Side;
use qm_core::patch::order::{matching_patches, OrderRule};
use qm_core::patch::plan::{self, FpMode, PatchExtra, RawLoad};
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, OnFailurePolicy, Patch, PatternPatch, Target};
use qm_manager::fixes::{self, FixChange, FixSource, Provenance, FIXES_VERSION};
use qm_manager::region::{self, RegionLine};
use qm_manager::runs::TrialChanges;
use qm_manager::suggestions::PLACEMENT_KINDS;
use qm_manager::{ModProfile, SharedStack, SimulationReport, NOTICE_LINE_RULES, NOTICE_STACKING};
use qm_schema::faction::Side as FactionSide;
use qm_schema::ModManifest;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub(crate) const MSG_NOT_READY: &str =
    "The patch simulation is still running. Try again in a moment.";

/// The longest a preview waits for a simulation that is being computed.
const SIMULATION_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

impl Service {
    /// The table the cached simulation ran, or an error while it is being computed.
    fn sim_table(&self) -> QmResult<Arc<PatchTable>> {
        self.inner
            .caches
            .patch_table
            .get()
            .ok_or_else(|| QmError::new(ErrorKind::Conflict, MSG_NOT_READY))
    }

    /// While the simulation is not cached but a job is computing it (a staging just cleared
    /// it, as when a test run ends), waits for that job, at most `SIMULATION_WAIT` in all.
    /// A preview asked for right then gets the new simulation instead of `MSG_NOT_READY`.
    /// Called outside the preview lock.
    fn wait_for_pending_simulation(&self) {
        let deadline = std::time::Instant::now() + SIMULATION_WAIT;
        loop {
            let caches = &self.inner.caches;
            if caches.patch_table.get().is_some() && caches.simulation.get().is_some() {
                return;
            }
            let job = self
                .jobs()
                .list()
                .into_iter()
                .find(|j| matches!(j.kind, JobKind::Simulation | JobKind::WarmVanilla));
            let now = std::time::Instant::now();
            match job {
                Some(j) if now < deadline => {
                    let left = deadline - now;
                    self.jobs()
                        .wait_for(j.id, left.min(std::time::Duration::from_millis(500)));
                }
                _ => return,
            }
        }
    }

    pub fn patch_detail(&self, patch_id: &str) -> QmResult<PatchDetail> {
        let table = self.sim_table()?;
        let i = find_patch(&table, patch_id).ok_or_else(|| {
            QmError::new(
                ErrorKind::NotFound,
                format!("No loaded patch is called '{patch_id}'."),
            )
        })?;
        let (patch, prio, path) = &table.patches[i];
        let prep = table.prepare();
        let order = &prep.order().order;
        let mut positions = Vec::new();
        for target in table.targets.iter().collect::<BTreeSet<_>>() {
            let t = target.trim_start_matches('@');
            if !patch.can_apply_to_target(t) {
                continue;
            }
            let in_file: Vec<usize> = order
                .iter()
                .copied()
                .filter(|&k| table.patches[k].0.can_apply_to_target(t))
                .collect();
            if let Some(index) = in_file.iter().position(|&k| k == i) {
                positions.push(PatchFilePosition {
                    target_file: t.to_string(),
                    index: to_u32(index),
                    count: to_u32(in_file.len()),
                });
            }
        }
        let (declared_before, declared_after) = match patch {
            Patch::Pattern(p) => (p.before.clone(), p.after.clone()),
            Patch::Regex(p) => (p.before.clone(), p.after.clone()),
            Patch::Copy(p) => (p.before.clone(), p.after.clone()),
            Patch::Module(_) => (Vec::new(), Vec::new()),
        };
        let names_this = |name: &String| refers_to(&table, name, i);
        let profile_rules = self
            .active_profile()
            .map(|p| p.patch_order)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| names_this(&r.patch) || r.before.iter().chain(&r.after).any(names_this))
            .collect();
        // As the engine resolves `requires` (D64), so the disable cascade lists the patches
        // a disable would skip.
        let required_by = table
            .patches
            .iter()
            .enumerate()
            .filter(|&(k, _)| {
                k != i
                    && prep
                        .requires(k)
                        .iter()
                        .any(|r| r.hits.iter().any(|&h| h as usize == i))
            })
            .map(|(k, (q, _, _))| q.id().map_or_else(|| format!("#{k}"), str::to_string))
            .collect();
        let glue = self.active_profile().ok().and_then(|p| {
            p.glue_patches
                .into_iter()
                .find(|g| Some(g.id.as_str()) == patch.id())
        });
        Ok(PatchDetail {
            patch_id: patch.id().unwrap_or_default().to_string(),
            name: patch.name().map(str::to_string),
            mod_id: patch.mod_id().unwrap_or_default().to_string(),
            source_file: path.display().to_string().replace('\\', "/"),
            priority: *prio,
            definition: patch.clone(),
            positions,
            declared_before,
            declared_after,
            profile_rules,
            requires: patch.requires().to_vec(),
            required_by,
            disabled: patch.is_disabled(),
            replaces_patch_id: glue.as_ref().and_then(|g| g.replaces_patch_id.clone()),
            base_mod_id: glue.and_then(|g| g.base_mod_id),
            runs_as: qm_manager::conflict::runs_as_verdict(&table, &prep, i),
        })
    }

    /// What the cached simulation read from the staged Mods folder, read again (and cached)
    /// when it is missing.
    pub(crate) fn preview_raw(
        &self,
        table: &PatchTable,
        plan: Option<&LoadOrderPlan>,
    ) -> QmResult<Arc<RawLoad>> {
        if let Some(raw) = self.inner.caches.raw_load.get() {
            return Ok(raw);
        }
        let gen = self.generations().sim;
        let dir = if table.mod_dir.as_os_str().is_empty() {
            self.staged_mods_dir()
        } else {
            table.mod_dir.clone()
        };
        if !dir.is_dir() {
            // Never staged: no mods, as in the simulation.
            return Ok(Arc::new(RawLoad::default()));
        }
        let raw = plan::load_raw(&dir, plan).map(Arc::new).map_err(|e| {
            QmError::new(ErrorKind::Io, "Could not read the profile's mods.")
                .with_details(format!("{e:#}"))
        })?;
        self.with_gens(|g| {
            if g.sim == gen {
                self.inner.caches.raw_load.set(raw.clone());
            }
        });
        Ok(raw)
    }

    /// Runs the simulation with `changes` applied to a copy of the active profile, compiled
    /// the way activation compiles it (`build_plan`) and laid over the cached mods
    /// (`apply_plan`), so the preview is what committing the changes would give. Nothing is
    /// stored. One preview runs at a time; a request that a newer one overtook while it waited
    /// returns `Cancelled`.
    pub fn simulation_preview(&self, changes: PreviewChanges) -> QmResult<PreviewResult> {
        let mut result = self.preview_with(
            changes.focus_file.clone(),
            &|svc, table, raw, mut profile| {
                let ledger = !changes.add_fixes.is_empty() || !changes.remove_fixes.is_empty();
                if ledger || profile.fixes_version >= FIXES_VERSION {
                    // In memory only (D38): nothing is saved.
                    profile = svc
                        .migrated_in_memory(profile, Some(raw))
                        .map_err(|why| {
                            QmError::new(
                                ErrorKind::InvalidInput,
                                format!("Your existing fixes could not be imported yet: {why}"),
                            )
                        })?
                        .0;
                }
                apply_preview_changes(svc, &mut profile, table, raw, &changes)?;
                let mut rules = changes.order_rules.clone();
                rules.extend(changes.add_fixes.iter().filter_map(|d| match &d.change {
                    FixChangeDraft::Order { rule } => Some(rule.clone()),
                    _ => None,
                }));
                Ok((profile, rules))
            },
        )?;
        // A stack rule that puts a mod's block above the block of a mod it depends on (D56).
        if changes
            .add_fixes
            .iter()
            .any(|d| matches!(d.change, FixChangeDraft::StackRule { .. }))
        {
            let table = self.sim_table()?;
            let moved = result.impact.stack_changes.clone();
            for w in self.stack_relation_warnings(&table, &changes.add_fixes, &moved) {
                if !result.impact.relation_warnings.contains(&w) {
                    result.impact.relation_warnings.push(w);
                }
            }
        }
        Ok(result)
    }

    /// The simulation of a trial run (D68): the active profile with `changes` applied the way
    /// the trial stages them (`TrialChanges::apply`, the mods left out of the load). The same
    /// result shows what making the changes for good would do. Nothing is stored.
    pub fn trial_preview(&self, changes: TrialChanges) -> QmResult<PreviewResult> {
        self.trial_run(&changes, true)
    }

    /// `trial_preview` for the check before a trial launch: never cancelled by a newer preview.
    pub(crate) fn trial_check(&self, changes: &TrialChanges) -> QmResult<PreviewResult> {
        self.trial_run(changes, false)
    }

    fn trial_run(&self, changes: &TrialChanges, cancellable: bool) -> QmResult<PreviewResult> {
        self.preview_run(
            None,
            &changes.disable_mods,
            &|_, _, _, mut profile| {
                changes.apply(&mut profile);
                Ok((profile, Vec::new()))
            },
            cancellable,
        )
    }

    /// Runs the simulation with the active profile as `edit` changes it, compiled the way
    /// activation compiles it and laid over the cached mods. `edit` also returns the order
    /// rules to check against the mods' relations (D39). Nothing is stored.
    #[allow(clippy::type_complexity)]
    pub(crate) fn preview_with(
        &self,
        focus_file: Option<String>,
        edit: &dyn Fn(
            &Service,
            &PatchTable,
            &RawLoad,
            ModProfile,
        ) -> QmResult<(ModProfile, Vec<OrderRule>)>,
    ) -> QmResult<PreviewResult> {
        self.preview_run(focus_file, &[], edit, true)
    }

    /// `preview_with` with the mods in `drop_mods` turned off, as a write that turns them off
    /// stages it (D71: undoing an export turns its mod off).
    #[allow(clippy::type_complexity)]
    pub(crate) fn preview_without(
        &self,
        drop_mods: &[String],
        edit: &dyn Fn(
            &Service,
            &PatchTable,
            &RawLoad,
            ModProfile,
        ) -> QmResult<(ModProfile, Vec<OrderRule>)>,
    ) -> QmResult<PreviewResult> {
        self.preview_run(None, drop_mods, edit, true)
    }

    /// `preview_with` for a check before a write: it waits its turn but a newer preview never
    /// cancels it, and it cancels none.
    #[allow(clippy::type_complexity)]
    pub(crate) fn preview_check(
        &self,
        edit: &dyn Fn(
            &Service,
            &PatchTable,
            &RawLoad,
            ModProfile,
        ) -> QmResult<(ModProfile, Vec<OrderRule>)>,
    ) -> QmResult<PreviewResult> {
        self.preview_run(None, &[], edit, false)
    }

    #[allow(clippy::type_complexity)]
    fn preview_run(
        &self,
        focus_file: Option<String>,
        drop_mods: &[String],
        edit: &dyn Fn(
            &Service,
            &PatchTable,
            &RawLoad,
            ModProfile,
        ) -> QmResult<(ModProfile, Vec<OrderRule>)>,
        cancellable: bool,
    ) -> QmResult<PreviewResult> {
        let seq = cancellable.then(|| self.inner.preview_seq.fetch_add(1, Ordering::SeqCst) + 1);
        self.wait_for_pending_simulation();
        let _one = self.inner.preview.lock().unwrap_or_else(|e| e.into_inner());
        if seq.is_some_and(|seq| self.inner.preview_seq.load(Ordering::SeqCst) != seq) {
            return Err(QmError::new(
                ErrorKind::Cancelled,
                "A newer preview replaced this one.",
            ));
        }
        let table = self.sim_table()?;
        let current = self
            .inner
            .caches
            .simulation
            .get()
            .ok_or_else(|| QmError::new(ErrorKind::Conflict, MSG_NOT_READY))?;
        let vanilla = self
            .inner
            .caches
            .vanilla
            .get()
            .ok_or_else(|| QmError::new(ErrorKind::Conflict, MSG_NOT_READY))?;

        let staged = LoadOrderPlan::read(&self.staged_mods_dir());
        if let Some(e) = &staged.error {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "The profile's load order could not be read. Activate the profile again.",
            )
            .with_details(e.to_string()));
        }
        let full = self.preview_raw(&table, staged.plan.as_ref())?;
        let raw = if drop_mods.is_empty() {
            full.clone()
        } else {
            Arc::new(raw_without(&full, drop_mods))
        };
        let active = self.active_profile()?;
        let (profile, rules) = edit(self, &table, &raw, active.clone())?;
        let dropped = |m: &String| drop_mods.iter().any(|d| d.eq_ignore_ascii_case(m));
        let (enabled, mod_priorities) = match &staged.plan {
            Some(plan) => (
                plan.enabled_mods
                    .iter()
                    .filter(|m| !dropped(m))
                    .cloned()
                    .collect(),
                plan.mod_priorities
                    .iter()
                    .filter_map(|(k, v)| v.ok().map(|v| (k.clone(), *v)))
                    .collect(),
            ),
            None => (profile.enabled_mods.clone(), HashMap::new()),
        };
        // Through JSON, as the game reads the plan activation writes.
        let compiled = qm_manager::build_plan(&profile, enabled, mod_priorities);
        let json = serde_json::to_string(&compiled).map_err(|e| {
            QmError::new(ErrorKind::Internal, "Could not compile the preview.")
                .with_details(e.to_string())
        })?;
        let applied = plan::apply_plan_with(&raw, &LoadOrderPlan::parse(&json), FpMode::All);
        let t = PatchTable::from_applied(applied, raw.mod_dir.clone());
        let mut report = self.simulate_table(&t, &vanilla);
        // Code an anchor fix moves into another function or scope (D69, Task 37c): warnings
        // for this preview only.
        let moved =
            qm_manager::placement::moved_code(&table, &current, &t, &report, &vanilla.files);
        report.conflicts.extend(moved);

        // Info notes (`multi_match`) are not conflicts.
        let ids = |r: &qm_manager::SimulationReport| -> BTreeSet<String> {
            r.conflicts
                .iter()
                .filter(|c| c.severity != "info")
                .map(|c| c.id.clone())
                .collect()
        };
        let (before, after) = (ids(&current), ids(&report));
        let mut impact = preview_impact(&current, &report);
        impact.relation_warnings = self.relation_warnings(&t, &rules);
        impact.faction_changes = self.faction_changes(&full, &raw, &active, &profile);
        if active.fixes_version >= FIXES_VERSION {
            let (was, _) = self
                .migrated_in_memory(active.clone(), Some(&full))
                .unwrap_or((active.clone(), false));
            let now = self.fix_statuses(&was, Some(&full), &table, &current);
            let next = self.fix_statuses(&profile, Some(&raw), &t, &report);
            impact.fix_status_changes = status_changes(&now, &next);
        }
        // A change resolves nothing when it makes another patch use its own line (§10).
        let own = anchored_by_change(&active, &profile);
        let falls_back = impact
            .fallbacks_new
            .iter()
            .any(|f| !own.contains(&f.patch_id));
        Ok(PreviewResult {
            resolved: if impact.breaks_file || falls_back {
                Vec::new()
            } else {
                before.difference(&after).cloned().collect()
            },
            new_conflicts: after.difference(&before).cloned().collect(),
            impact,
            file: focus_file.and_then(|f| {
                report
                    .files_touched
                    .iter()
                    .find(|r| r.target_file == f)
                    .cloned()
            }),
            conflicts: report.conflicts,
        })
    }

    /// The sides whose active faction the change from `active` to `profile` would switch
    /// (D73): the factions of `full` against those of `raw` (the mods the change leaves on).
    /// The staged plan gives every faction its mod's priority, so when the change moves mods
    /// (a load order fix) the priorities the solver gives the changed profile are used.
    fn faction_changes(
        &self,
        full: &RawLoad,
        raw: &RawLoad,
        active: &ModProfile,
        profile: &ModProfile,
    ) -> Vec<FactionChange> {
        if full.factions.is_empty() {
            return Vec::new();
        }
        let mut after = raw.factions.clone();
        let moved = active.mod_priorities != profile.mod_priorities
            || active.user_load_after != profile.user_load_after;
        if moved {
            let manifests: Vec<ModManifest> = self
                .installed_mods()
                .iter()
                .map(|m| m.manifest.clone())
                .collect();
            let order = |p: &ModProfile| -> HashMap<String, i32> {
                qm_manager::resolve_profile_order(p, &manifests)
                    .ordered_mods
                    .into_iter()
                    .map(|e| (e.mod_id.to_lowercase(), e.effective_priority))
                    .collect()
            };
            let (was, will) = (order(active), order(profile));
            for lf in &mut after {
                let id = lf.mod_id.to_lowercase();
                if let Some(&p) = will.get(&id) {
                    if was.get(&id) != Some(&p) {
                        lf.priority = p;
                    }
                }
            }
        }
        // A picked side keeps its faction whatever the priorities become (D82).
        faction_diff(
            full.factions.clone(),
            after,
            &active.faction_picks,
            &profile.faction_picks,
        )
    }

    /// Lines within `radius` of `line` (1-based) in the patched `target_file`, with how many lines
    /// of the patched file each would match as a pattern anchor and who wrote it. Blank lines
    /// are left out. With `region`, the lines of that place of another patch's change instead
    /// (the picker's region mode, §11.3); `line` and `radius` are then not used.
    pub fn anchor_candidates(
        &self,
        target_file: &str,
        line: u32,
        radius: u32,
        region: Option<RegionRef>,
    ) -> QmResult<Vec<AnchorCandidate>> {
        let report = self
            .inner
            .caches
            .simulation
            .get()
            .ok_or_else(|| QmError::new(ErrorKind::Conflict, MSG_NOT_READY))?;
        // The full patched text: attributed lines are trimmed for files over 1,200 lines.
        let trace = report.traces.get(target_file).ok_or_else(|| {
            QmError::new(
                ErrorKind::NotFound,
                format!("No patches touch '{target_file}'."),
            )
        })?;
        // Patterns match the text before `[vars]` values are filled in.
        let raw = trace.raw_lines();
        let raw_text = raw.join("\n");
        let count = |l: &str| to_u32(count_matches(target_file, &raw_text, l.trim()));
        if let Some(r) = region {
            let (ix, rec) = region::find_record(&report, target_file, &r.patch_id, None)
                .ok_or_else(|| {
                    QmError::new(
                        ErrorKind::NotFound,
                        format!("'{}' made no change in '{target_file}'.", r.patch_id),
                    )
                })?;
            let site = region::b_sites(trace, ix, &rec.mod_id)
                .into_iter()
                .find(|s| s.site == r.site)
                .ok_or_else(|| {
                    QmError::new(
                        ErrorKind::NotFound,
                        format!("'{}' has no such place in '{target_file}'.", r.patch_id),
                    )
                })?;
            return Ok(site
                .lines
                .iter()
                .filter(|l| !l.text.trim().is_empty())
                .map(|l| candidate(l, count(&l.raw_text)))
                .collect());
        }
        let lines: Vec<&str> = trace.text.lines().collect();
        let lo = line.saturating_sub(radius).max(1) as usize;
        let hi = (line.saturating_add(radius) as usize).min(lines.len());
        Ok((lo..=hi)
            .filter_map(|n| {
                let text = lines.get(n - 1)?;
                if text.trim().is_empty() {
                    return None;
                }
                let raw_line = raw.get(n - 1).copied().unwrap_or(text);
                Some(AnchorCandidate {
                    line: to_u32(n),
                    text: text.to_string(),
                    matches_in_file: count(raw_line),
                    origin: region::line_source(trace, n).map(source_dto),
                    matches_in_region: None,
                    raw_text: raw_line.to_string(),
                })
            })
            .collect())
    }

    /// Where `patch_id`'s code can go in `target_file` when another patch took its line
    /// (§9.2), each option previewed on that file. `conflict_id` names the other patch when
    /// the simulation found no consumer. Never writes.
    pub fn region_options(
        &self,
        target_file: &str,
        patch_id: &str,
        conflict_id: Option<&str>,
    ) -> QmResult<RegionOptions> {
        let report = self
            .inner
            .caches
            .simulation
            .get()
            .ok_or_else(|| QmError::new(ErrorKind::Conflict, MSG_NOT_READY))?;
        let table = self.sim_table()?;
        let conflict_id = conflict_id.map(str::trim).filter(|c| !c.is_empty());
        let b_hint = conflict_id.and_then(|id| {
            report
                .conflicts
                .iter()
                .find(|c| c.id == id)
                .and_then(|c| c.patch_b_id.clone())
        });
        let plan = region::plan(&report, &table, target_file, patch_id, b_hint.as_deref())
            .ok_or_else(|| {
                QmError::new(
                    ErrorKind::NotFound,
                    format!("No loaded patch is called '{patch_id}'."),
                )
            })?;
        // The patch's own places, for a move of one of them (D25); a regex's are its matches
        // (D52).
        let a_sites = match (
            table.patches.iter().find_map(|(p, _, _)| match p {
                Patch::Pattern(_) | Patch::Regex(_) if p.id() == Some(patch_id) => Some(p),
                _ => None,
            }),
            self.vanilla_files()
                .and_then(|v| v.files.get(target_file).cloned()),
        ) {
            (Some(a), Some(src)) => {
                let buf = region::prepared_original(&table, target_file, &src);
                region::patch_sites(a, &buf)
                    .into_iter()
                    .map(|s| ASite {
                        key: s.key,
                        line: s.line,
                        anchor_line: s.anchor_line,
                        text: s.text,
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        let drafts: Vec<Option<FixDraft>> = plan
            .offers
            .iter()
            .map(|o| {
                (!o.pick).then(|| FixDraft {
                    change: if o.apply_as {
                        FixChangeDraft::ApplyAs {
                            patch_id: o.patch_id.clone(),
                            mode: ApplyAs::Edits,
                        }
                    } else {
                        FixChangeDraft::Anchor {
                            patch_id: o.patch_id.clone(),
                            anchors: o.anchors.clone(),
                            sites: o.sites.clone(),
                        }
                    },
                    label: o.label.clone(),
                    source: FixSource::Resolver,
                    conflict_id: conflict_id.map(str::to_string),
                    option_key: Some(o.key.clone()),
                    approved: None,
                })
            })
            .collect();
        // Options whose edge cannot be used are greyed without a preview.
        let to_check: Vec<(usize, &FixDraft)> = drafts
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                plan.offers
                    .get(*i)
                    .and_then(|o| o.edge.as_ref())
                    .is_none_or(|e| e.allowed)
            })
            .filter_map(|(i, d)| d.as_ref().map(|d| (i, d)))
            .collect();
        let verdicts = self.check_region_drafts(target_file, &report, &table, &to_check);
        let options = plan
            .offers
            .iter()
            .zip(drafts)
            .enumerate()
            .map(|(i, (o, draft))| {
                let reason = match o.edge.as_ref().filter(|e| !e.allowed) {
                    Some(e) => e
                        .reason
                        .clone()
                        .or_else(|| Some("cannot be used".to_string())),
                    None => verdicts.get(&i).cloned().flatten(),
                };
                RegionOption {
                    key: o.key.clone(),
                    label: o.label.clone(),
                    kind: region::kind_name(o.kind).to_string(),
                    draft,
                    placement_ok: reason.is_none(),
                    placement_reason: reason,
                    suggested: o.suggested,
                    note: o.note.clone(),
                    old_text: o.old_text.clone(),
                    new_text: o.new_text.clone(),
                }
            })
            .collect();
        let raw_text = report
            .traces
            .get(target_file)
            .map(|t| t.raw_lines().join("\n"))
            .unwrap_or_default();
        let sites = plan
            .sites
            .iter()
            .map(|s| RegionSite {
                site: s.site,
                first_line: s.first_line,
                last_line: s.last_line,
                function: s.function.clone(),
                lines: s
                    .lines
                    .iter()
                    .filter(|l| !l.text.trim().is_empty())
                    .map(|l| {
                        let n = count_matches(target_file, &raw_text, l.raw_text.trim());
                        candidate(l, to_u32(n))
                    })
                    .collect(),
                edge_start: s.edge_start.clone(),
                edge_end: s.edge_end.clone(),
            })
            .collect();
        Ok(RegionOptions {
            target_file: target_file.to_string(),
            patch_id: patch_id.to_string(),
            consumer: plan.consumer,
            a_is_at: plan.a_is_at,
            move_by_copy: plan.move_by_copy,
            b_patch_id: plan.b.as_ref().map(|b| b.0.clone()),
            b_mod_id: plan.b.map(|b| b.1),
            options,
            sites,
            a_sites,
        })
    }

    /// Previews each draft on `target` alone and says why it cannot be offered: it stops the
    /// file from compiling, moves code into another block, or its patch would still use its own
    /// line. None for a draft that passes. Runs outside the preview lock, as the suggestions do.
    fn check_region_drafts(
        &self,
        target: &str,
        current: &SimulationReport,
        table: &PatchTable,
        drafts: &[(usize, &FixDraft)],
    ) -> HashMap<usize, Option<String>> {
        let mut out = HashMap::new();
        if drafts.is_empty() {
            return out;
        }
        let raw = self.current_raw();
        let vanilla = self.vanilla_files();
        let profile = self
            .active_profile()
            .map_err(|e| e.message)
            .and_then(|p| self.migrated_in_memory(p, raw.as_deref()));
        let (Some(raw), Some(vanilla), Ok((p, _))) = (raw, vanilla, profile) else {
            let why = "The preview is not ready yet.".to_string();
            for (i, _) in drafts {
                out.insert(*i, Some(why.clone()));
            }
            return out;
        };
        let only = VanillaFiles {
            game_version: vanilla.game_version.clone(),
            info: vanilla.info.clone(),
            files: vanilla
                .files
                .iter()
                .filter(|(k, _)| k.as_str() == target)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            assets: vanilla.assets.clone(),
            listing: vanilla.listing.clone(),
        };
        let (enabled, prios) = self.staged_order(&p);
        let in_file = |r: &SimulationReport, kinds: &[&str]| -> BTreeSet<String> {
            r.conflicts
                .iter()
                .filter(|c| c.target_file == target && kinds.contains(&c.kind.as_str()))
                .map(|c| c.id.clone())
                .collect()
        };
        let broken_now = in_file(current, &["compile_error"]);
        let placed_now = in_file(current, &PLACEMENT_KINDS);
        for (i, draft) in drafts {
            let mut q = p.clone();
            let verdict = match self.apply_draft(&mut q, Some(&raw), Some(table), draft) {
                Err(e) => Some(e.message),
                Ok(_) => {
                    let t = qm_manager::suggestions::table_for(
                        &q,
                        &raw,
                        enabled.clone(),
                        prios.clone(),
                    );
                    let next = self.simulate_table(&t, &only);
                    let anchored = match &draft.change {
                        FixChangeDraft::Anchor { patch_id, .. } => patch_id.as_str(),
                        _ => "",
                    };
                    let edits = match &draft.change {
                        FixChangeDraft::ApplyAs { patch_id, .. } => Some(patch_id.as_str()),
                        _ => None,
                    };
                    region_verdict(target, anchored, &next, &broken_now, &placed_now)
                        .or_else(|| edits.and_then(|id| edits_verdict(target, id, &next)))
                }
            };
            out.insert(*i, verdict);
        }
        out
    }

    /// The shared lines of the cached simulation, of one file or of all of them.
    pub fn stacks_get(&self, target_file: Option<&str>) -> QmResult<Vec<SharedStack>> {
        let report = self
            .inner
            .caches
            .simulation
            .get()
            .ok_or_else(|| QmError::new(ErrorKind::Conflict, MSG_NOT_READY))?;
        Ok(report
            .files_touched
            .iter()
            .filter(|f| match target_file {
                Some(t) => f.target_file == t,
                None => true,
            })
            .flat_map(|f| f.stacks.iter().cloned())
            .collect())
    }

    /// Sets how the active profile orders code at shared lines; the stacking notice counts as
    /// seen (D3). The UI previews it first (`PreviewChanges.stacking`).
    pub fn stacking_set(&self, mode: StackMode) -> QmResult<()> {
        self.ensure_not_running()?;
        let mut p = self.active_profile()?;
        p.stacking = Some(mode);
        p.mark_notice_seen(NOTICE_STACKING);
        self.commit_active_profile(p).map(|_| ())
    }

    /// The `stacking_v2` answer (D51): `true` uses the newer line rules ("Use the new order"),
    /// `false` keeps the older ones. The notice counts as seen.
    pub fn line_rules_set(&self, on: bool) -> QmResult<()> {
        self.ensure_not_running()?;
        let mut p = self.active_profile()?;
        p.line_rules_v2 = Some(on);
        p.mark_notice_seen(NOTICE_LINE_RULES);
        self.commit_active_profile(p).map(|_| ())
    }

    /// Saves an `Adjusted copy of <name>` of `base_patch_id` anchored on `pattern`, and disables
    /// the base patch for the active profile.
    pub fn glue_patch_create(
        &self,
        base_patch_id: &str,
        pattern: &str,
        position: InsertPosition,
    ) -> QmResult<GluePatchEntry> {
        self.glue_patch_create_for(base_patch_id, pattern, position, None)
    }

    /// `glue_patch_create` from the resolver: the copy records `conflict_id`, so "Resolved
    /// by" finds it after a restart.
    pub fn glue_patch_create_for(
        &self,
        base_patch_id: &str,
        pattern: &str,
        position: InsertPosition,
        conflict_id: Option<&str>,
    ) -> QmResult<GluePatchEntry> {
        self.ensure_not_running()?;
        if pattern.trim().is_empty() {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "Pick a line to anchor on.",
            ));
        }
        let table = self.sim_table()?;
        let i = find_patch(&table, base_patch_id).ok_or_else(|| {
            QmError::new(
                ErrorKind::NotFound,
                format!("No loaded patch is called '{base_patch_id}'."),
            )
        })?;
        let (base, prio, _) = &table.patches[i];
        let base_id = base.id().unwrap_or(base_patch_id).to_string();
        let base_name = base.name().unwrap_or(&base_id).to_string();
        let (mut p, raw) = self.writable_profile()?;
        // A copy of a copy names the older copy as its base: the engine stops the older copy
        // and binds the newer one to the first copy's base, as that base's mod (D59). The older
        // copy's fix stays, so undoing the newer copy brings it back.
        let older = base
            .extra()
            .adjusted_copy
            .then(|| glue_entry(&p, &base_id))
            .flatten();
        // A copy keeps the places of the patch it stands in for: the first copy's base.
        let times = match &older {
            Some(g) => self
                .root_base_times(&table, &p, g)
                .or_else(|| self.copy_times(&table, base)),
            None => self.copy_times(&table, base),
        };
        let copy = retargeted(base, pattern.trim(), position, times)?;
        let (stem, name, base_mod_id) = match &older {
            Some(g) => (
                g.replaces_patch_id.clone(),
                g.name.clone(),
                g.base_mod_id.clone(),
            ),
            None => (
                Some(base_id.clone()),
                format!("Adjusted copy of {base_name}"),
                base.mod_id().map(str::to_string),
            ),
        };
        let replaces = match &older {
            Some(g) => Some(g.id.clone()),
            None => stem.clone(),
        };
        let stem = stem.unwrap_or_else(|| base_patch_id.to_string());
        // Not the id of any copy, a paused one included.
        let mut id = format!("glue:{stem}");
        let mut n = 2;
        while fixes::glue_id_taken(&p, &id) {
            id = format!("glue:{stem}:{n}");
            n += 1;
        }
        // New copies run as their base mod and match their anchor exactly (D58, D59).
        let entry = GluePatchEntry {
            id,
            name,
            replaces_patch_id: replaces,
            priority: *prio,
            patch: copy,
            base_mod_id,
            run_as: Some(RunAs::Base),
            exact: true,
        };
        let mut provenance = Provenance::new(FixSource::Resolver);
        if let Some(id) = conflict_id.map(str::trim).filter(|id| !id.is_empty()) {
            provenance.conflict_kind = self.conflict_kind(id);
            provenance.conflict_id = Some(id.to_string());
        }
        fixes::op_glue_add(&mut p, raw.as_deref(), entry.clone(), true, provenance);
        self.commit_active_profile(p)?;
        Ok(entry)
    }

    /// How many times a copy of `base` applies (D59): the base's `times`, else the most
    /// places it has in one game file when it runs alone, at least 1.
    fn copy_times(&self, table: &PatchTable, base: &Patch) -> Option<usize> {
        let own = match base {
            Patch::Pattern(p) => p.times,
            Patch::Regex(r) => r.times,
            _ => None,
        };
        if own.is_some() {
            return own;
        }
        let solo = self.vanilla_files().map_or(0, |v| {
            v.files
                .iter()
                .filter(|(k, _)| base.can_apply_to_target(k))
                .map(|(k, src)| {
                    let buf = region::prepared_original(table, k, src);
                    region::patch_sites(base, &buf).len()
                })
                .max()
                .unwrap_or(0)
        });
        Some(solo.max(1))
    }

    /// A new copy from the UI's preview with no `times` gets the count `glue_patch_create`
    /// gives it, so the preview equals the Apply (D59).
    fn fill_copy_times(&self, table: &PatchTable, p: &ModProfile, glue: &mut GluePatchEntry) {
        if !glue.runs_as_base() {
            return;
        }
        let times = self.root_base_times(table, p, glue);
        if let Patch::Pattern(x) = &mut glue.patch {
            if x.times.is_none() {
                x.times = times;
            }
        }
    }

    /// `copy_times` of the patch `glue` stands in for: its base, or for a copy of copies the
    /// first copy's base (D59). None when that base is not loaded.
    fn root_base_times(
        &self,
        table: &PatchTable,
        p: &ModProfile,
        glue: &GluePatchEntry,
    ) -> Option<usize> {
        let mut id = glue.replaces_patch_id.clone()?;
        // Bounded: a loop of copies ends where it started.
        for _ in 0..32 {
            match glue_entry(p, &id).and_then(|g| g.replaces_patch_id) {
                Some(next) if next != id => id = next,
                _ => break,
            }
        }
        if let Some((base, _, _)) = find_patch(table, &id).and_then(|i| table.patches.get(i)) {
            return self.copy_times(table, base);
        }
        // The copies keep their base off, so it is only in the raw load.
        let raw = self.current_raw()?;
        let base = raw
            .patches
            .iter()
            .find(|rp| qm_core::patch::plan::plan_keys(&rp.patch).contains(&id))?;
        self.copy_times(table, &base.patch)
    }

    /// Removes a glue patch and re-enables its base patch unless another glue still replaces it.
    pub fn glue_patch_remove(&self, glue_id: &str) -> QmResult<()> {
        self.ensure_not_running()?;
        let (mut p, _) = self.writable_profile()?;
        if !fixes::op_glue_remove(&mut p, glue_id) {
            return Err(QmError::new(
                ErrorKind::NotFound,
                "That adjusted patch is already gone.",
            ));
        }
        self.commit_active_profile(p).map(|_| ())
    }

    /// Sets the active profile's order rule for `rule.patch`, replacing any earlier one.
    pub fn patch_order_set(&self, rule: OrderRule) -> QmResult<()> {
        self.ensure_not_running()?;
        if rule.patch.trim().is_empty() || (rule.before.is_empty() && rule.after.is_empty()) {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "An order rule needs a patch and at least one patch to run before or after.",
            ));
        }
        let (mut p, raw) = self.writable_profile()?;
        fixes::op_order_set(
            &mut p,
            raw.as_deref(),
            rule,
            Provenance::new(FixSource::OrderView),
        );
        self.commit_active_profile(p).map(|_| ())
    }

    /// Removes the active profile's order rule for `patch_id`.
    pub fn patch_order_reset(&self, patch_id: &str) -> QmResult<()> {
        self.ensure_not_running()?;
        let (mut p, _) = self.writable_profile()?;
        fixes::op_order_reset(&mut p, patch_id);
        self.commit_active_profile(p).map(|_| ())
    }

    /// Enables or disables several patches with one profile write (cascades).
    /// The profile entries that turn `id` on when removed: the names `patch_aliases` gives,
    /// and through `table` each named patch's id, `mod:name` and D29 legacy id. The preview
    /// and the commit use this one list.
    pub(crate) fn enable_aliases(&self, table: Option<&PatchTable>, id: &str) -> HashSet<String> {
        let mut aliases: HashSet<String> = self.patch_aliases(id).into_iter().collect();
        if let Some(table) = table {
            for k in patch_and_aliases(table, id) {
                let q = &table.patches[k].0;
                aliases.extend(q.id().map(str::to_string));
                aliases.extend(named(q));
                // D29 legacy id and D62 zip stem names.
                let mods: Vec<&str> = q
                    .mod_id()
                    .into_iter()
                    .chain(q.extra().legacy_mod.as_deref())
                    .collect();
                if let (Some(stem), Some(id)) = (&q.extra().legacy_mod, q.id()) {
                    aliases.insert(format!("{stem}:{id}"));
                }
                for old in q.extra().legacy_names() {
                    aliases.insert(old.to_string());
                    aliases.extend(mods.iter().map(|m| format!("{m}:{old}")));
                }
            }
        }
        aliases
    }

    pub fn patch_set_enabled_many(&self, ids: &[String], enabled: bool) -> QmResult<()> {
        self.ensure_not_running()?;
        let (mut p, raw) = self.writable_profile()?;
        let table = self.sim_table().ok();
        for id in ids {
            self.set_patch_enabled_in(&mut p, raw.as_deref(), table.as_deref(), id, enabled);
        }
        self.commit_active_profile(p).map(|_| ())
    }

    /// `patch_set_enabled` on a profile in memory: enabling removes every entry that names
    /// the patch; disabling adds one unless one is there.
    pub(crate) fn set_patch_enabled_in(
        &self,
        p: &mut ModProfile,
        raw: Option<&RawLoad>,
        table: Option<&PatchTable>,
        id: &str,
        enabled: bool,
    ) {
        if enabled {
            let aliases = self.enable_aliases(table, id);
            fixes::op_enable(p, &aliases);
        } else {
            let aliases: HashSet<String> = self.patch_aliases(id).into_iter().collect();
            fixes::op_disable(
                p,
                raw,
                id,
                &aliases,
                Provenance::new(FixSource::PatchToggle),
            );
        }
    }
}

/// `raw` as the loader reads it with the mods in `drop` turned off: their patches, overrides
/// and factions go, and the other overrides' mod indices follow.
fn raw_without(raw: &RawLoad, drop: &[String]) -> RawLoad {
    let gone = |id: &str| drop.iter().any(|d| d.eq_ignore_ascii_case(id));
    let mut new_ix: Vec<Option<u32>> = Vec::with_capacity(raw.mods.len());
    let mut mods = Vec::new();
    for m in &raw.mods {
        if gone(&m.id) {
            new_ix.push(None);
        } else {
            new_ix.push(u32::try_from(mods.len()).ok());
            mods.push(m.clone());
        }
    }
    let remap = |ix: u32| new_ix.get(ix as usize).copied().flatten();
    RawLoad {
        mod_dir: raw.mod_dir.clone(),
        mods,
        patches: raw
            .patches
            .iter()
            .filter_map(|p| {
                let mod_ix = remap(p.mod_ix)?;
                let mut p = p.clone();
                p.mod_ix = mod_ix;
                Some(p)
            })
            .collect(),
        overrides: raw
            .overrides
            .iter()
            .filter_map(|(ix, o)| Some((remap(*ix)?, o.clone())))
            .collect(),
        factions: raw
            .factions
            .iter()
            .filter(|f| !gone(&f.mod_id))
            .cloned()
            .collect(),
        configs: raw
            .configs
            .iter()
            .filter(|c| !gone(&c.mod_id))
            .cloned()
            .collect(),
        load_errors: raw.load_errors.clone(),
        warnings: raw.warnings.clone(),
    }
}

/// A patch by record id, `mod_id:name`, or any rule name form.
fn find_patch(table: &PatchTable, id: &str) -> Option<usize> {
    let ps = &table.patches;
    ps.iter()
        .position(|(p, _, _)| p.id() == Some(id))
        .or_else(|| {
            ps.iter()
                .position(|(p, _, _)| named(p).as_deref() == Some(id))
        })
        .or_else(|| matching_patches(id, ps).first().copied())
}

fn named(p: &Patch) -> Option<String> {
    Some(format!("{}:{}", p.mod_id()?, p.name()?))
}

fn refers_to(table: &PatchTable, name: &str, i: usize) -> bool {
    named(&table.patches[i].0).as_deref() == Some(name)
        || matching_patches(name, &table.patches).contains(&i)
}

/// The patches a preview name refers to: its id, `mod:name` or any rule name form
/// (`mod:*` included).
fn patch_and_aliases(table: &PatchTable, id: &str) -> Vec<usize> {
    (0..table.patches.len())
        .filter(|&k| table.patches[k].0.id() == Some(id) || refers_to(table, id, k))
        .collect()
}

/// `changes` applied to `p` as the profile commands apply them: `patch_set_enabled_many`,
/// `patch_order_set`, `glue_patch_create`, `stacking_set`, `fix_create` and fix removal.
/// Names in `disable` are expanded to the ids of the patches they refer to (`mod:*`
/// included), since the plan matches ids and `mod:name` only.
fn apply_preview_changes(
    svc: &Service,
    p: &mut ModProfile,
    table: &PatchTable,
    raw: &RawLoad,
    changes: &PreviewChanges,
) -> QmResult<()> {
    for id in &changes.disable {
        let hits = patch_and_aliases(table, id);
        let ids: Vec<String> = if hits.is_empty() {
            vec![id.clone()]
        } else {
            hits.iter()
                .filter_map(|&k| table.patches[k].0.id().map(str::to_string))
                .collect()
        };
        for id in ids {
            let aliases = HashSet::from([id.clone()]);
            let prov = Provenance::new(FixSource::PatchToggle);
            fixes::op_disable(p, Some(raw), &id, &aliases, prov);
        }
    }
    for id in &changes.enable {
        let aliases = svc.enable_aliases(Some(table), id);
        fixes::op_enable(p, &aliases);
    }
    for rule in &changes.order_rules {
        let prov = Provenance::new(FixSource::OrderView);
        fixes::op_order_set(p, Some(raw), rule.clone(), prov);
    }
    for glue in &changes.glue_patches {
        let prov = Provenance::new(FixSource::Resolver);
        let mut glue = glue.clone();
        svc.fill_copy_times(table, p, &mut glue);
        fixes::op_glue_add(p, Some(raw), glue, true, prov);
    }
    if let Some(mode) = changes.stacking {
        p.stacking = Some(mode);
    }
    // As `fix_suggestion_apply` undoes them: each alone, later fixes stay.
    for id in &changes.remove_fixes {
        fixes::undo(p, Some(raw), id);
    }
    fixes::sync_legacy(p);
    for draft in &changes.add_fixes {
        svc.apply_draft(p, Some(raw), Some(table), draft)?;
    }
    Ok(())
}

/// `base` with a new pattern anchor and position, applied at most `times` times (D59). A
/// regex patch becomes a pattern patch with the text it inserts; a regex that reads its match
/// cannot be copied (a rung places it instead), and copy and module patches have no anchor to
/// move.
fn retargeted(
    base: &Patch,
    pattern: &str,
    position: InsertPosition,
    times: Option<usize>,
) -> QmResult<Patch> {
    let (target, payload, requires, on_failure, mod_id) = match base {
        Patch::Pattern(p) => {
            let mut p = p.clone();
            p.pattern = pattern.to_string();
            p.position = position;
            p.times = times;
            p.id = None;
            p.name = None;
            p.disabled = false;
            p.qm = Default::default();
            return Ok(Patch::Pattern(p));
        }
        Patch::Regex(r) => {
            let payload = r.plain_payload().ok_or_else(|| {
                QmError::new(
                    ErrorKind::InvalidInput,
                    "This regex patch uses parts of the text it matches, so a copy on another line would not work. Place it inside the other mod's change instead.",
                )
            })?;
            (
                r.target.clone(),
                payload,
                r.requires.clone(),
                r.on_failure,
                r.mod_id.clone(),
            )
        }
        _ => {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "Only pattern and regex patches can be moved to a new line.",
            ))
        }
    };
    Ok(Patch::Pattern(PatternPatch {
        pattern: pattern.to_string(),
        position,
        target,
        payload,
        match_indent: true,
        times,
        overwrite: false,
        id: None,
        name: None,
        description: None,
        requires,
        priority: None,
        before: Vec::new(),
        after: Vec::new(),
        on_failure,
        anchors: Vec::new(),
        sites: Vec::new(),
        stack: None,
        apply_as: None,
        mod_id,
        disabled: false,
        qm: Default::default(),
    }))
}

/// The profile's adjusted copy `id`, running or paused.
fn glue_entry(p: &ModProfile, id: &str) -> Option<GluePatchEntry> {
    fixes::fixes(p)
        .find_map(|f| match &f.change {
            FixChange::AdjustedCopy { glue, .. } if glue.id == id => Some(glue.clone()),
            _ => None,
        })
        .or_else(|| p.glue_patches.iter().find(|g| g.id == id).cloned())
}

/// How many places `anchor` matches in `text`, by the engine's own pattern matcher. The line
/// matches only its own text, as in a new copy's anchor and a text rung (D59).
fn count_matches(target: &str, text: &str, anchor: &str) -> usize {
    let mut t = PatchTable::default();
    t.targets.insert(target.to_string());
    let probe = Patch::Pattern(PatternPatch {
        pattern: anchor.to_string(),
        position: InsertPosition::At,
        target: Target::Single(target.to_string()),
        payload: anchor.to_string(),
        match_indent: true,
        times: None,
        overwrite: false,
        id: Some("anchor-probe".to_string()),
        name: None,
        description: None,
        requires: Vec::new(),
        priority: None,
        before: Vec::new(),
        after: Vec::new(),
        on_failure: OnFailurePolicy::Warn,
        anchors: Vec::new(),
        sites: Vec::new(),
        stack: None,
        apply_as: None,
        mod_id: Some("qm".to_string()),
        disabled: false,
        qm: PatchExtra {
            exact_lines: Some(anchor.lines().map(|l| l.trim().to_string()).collect()),
            ..PatchExtra::default()
        },
    });
    t.patches.push((probe, 0, PathBuf::new()));
    t.apply_patches_offline(target, text)
        .ok()
        .and_then(|(_, _, recs)| recs.first().map(|r| r.matched_count))
        .unwrap_or(0)
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn source_dto(s: region::LineSource) -> LineSource {
    LineSource {
        kind: s.kind.to_string(),
        patch_id: s.patch_id,
        mod_id: s.mod_id,
    }
}

/// A line of another patch's place as the picker lists it.
fn candidate(l: &RegionLine, matches_in_file: u32) -> AnchorCandidate {
    AnchorCandidate {
        line: l.line,
        text: l.text.clone(),
        matches_in_file,
        origin: l.source.clone().map(source_dto),
        matches_in_region: l.matches_in_region,
        raw_text: l.raw_text.clone(),
    }
}

/// Why running `patch_id` as separate edits does not resolve its loss in `target`: it still
/// misses a place, or a place still runs as one replacement. None when it applies in full.
fn edits_verdict(target: &str, patch_id: &str, next: &SimulationReport) -> Option<String> {
    let rec = next
        .files_touched
        .iter()
        .filter(|f| f.target_file == target)
        .flat_map(|f| &f.records)
        .find(|r| r.patch_id == patch_id)?;
    let refused = rec
        .warnings
        .iter()
        .any(|w| w.starts_with(qm_core::patch::edits::EDITS_REFUSED));
    (rec.status != PatchOutcomeStatus::Applied || refused)
        .then(|| "would still not find all of its lines".to_string())
}

/// Why a previewed region option cannot be offered, or None when it can: the file stops
/// compiling, a placement warning is added, or `anchored` still uses its own line.
fn region_verdict(
    target: &str,
    anchored: &str,
    next: &SimulationReport,
    broken_now: &BTreeSet<String>,
    placed_now: &BTreeSet<String>,
) -> Option<String> {
    let mine = |c: &&qm_manager::DetectedConflict| c.target_file == target;
    if next
        .conflicts
        .iter()
        .filter(mine)
        .any(|c| c.kind == "compile_error" && !broken_now.contains(&c.id))
    {
        return Some(format!("would stop {target} from compiling"));
    }
    if let Some(c) = next
        .conflicts
        .iter()
        .filter(mine)
        .find(|c| PLACEMENT_KINDS.contains(&c.kind.as_str()) && !placed_now.contains(&c.id))
    {
        let who = next
            .files_touched
            .iter()
            .flat_map(|f| &f.records)
            .find(|r| r.patch_id == c.patch_a_id)
            .map_or_else(|| c.patch_a_id.clone(), |r| r.patch_name.clone());
        return Some(format!("would move where {who} runs"));
    }
    let rec = next
        .files_touched
        .iter()
        .filter(|f| f.target_file == target)
        .flat_map(|f| &f.records)
        .find(|r| r.patch_id == anchored)?;
    let sites = rec
        .anchoring
        .as_ref()
        .map(|a| a.sites.as_slice())
        .unwrap_or(&[]);
    if sites.is_empty() {
        return Some("its code would not be placed there".to_string());
    }
    sites.iter().find(|s| s.rung.is_none()).map(|s| {
        s.fallback
            .clone()
            .unwrap_or_else(|| "it would use its own line".to_string())
    })
}

/// Where anchored patches use their own line, by file, patch and place.
fn fallbacks(r: &SimulationReport) -> BTreeMap<(String, String, u32, u32), String> {
    let mut out = BTreeMap::new();
    for f in &r.files_touched {
        for rec in &f.records {
            for s in rec.anchoring.iter().flat_map(|a| &a.sites) {
                if let (None, Some(why)) = (s.rung, &s.fallback) {
                    out.insert(
                        (
                            f.target_file.clone(),
                            rec.patch_id.clone(),
                            s.vanilla_line.unwrap_or(0),
                            s.site,
                        ),
                        why.clone(),
                    );
                }
            }
        }
    }
    out
}

fn fallback_diff(
    a: &BTreeMap<(String, String, u32, u32), String>,
    b: &BTreeMap<(String, String, u32, u32), String>,
) -> Vec<FallbackNotice> {
    b.iter()
        .filter(|(k, _)| !a.contains_key(*k))
        .map(|((file, patch_id, line, _), why)| FallbackNotice {
            file: file.clone(),
            patch_id: patch_id.clone(),
            site_line: *line,
            reason: why.clone(),
        })
        .collect()
}

/// Fixes whose status differs between two runs, by id.
fn status_changes(
    now: &BTreeMap<String, FixStatus>,
    next: &BTreeMap<String, FixStatus>,
) -> Vec<FixStatusChange> {
    let ids: BTreeSet<&String> = now.keys().chain(next.keys()).collect();
    ids.into_iter()
        .filter_map(|id| {
            let (a, b) = (now.get(id), next.get(id));
            (a != b).then(|| FixStatusChange {
                fix_id: id.clone(),
                before: a.cloned(),
                after: b.cloned(),
            })
        })
        .collect()
}

/// The patches whose Anchor fixes the change adds, changes or removes.
fn anchored_by_change(was: &ModProfile, now: &ModProfile) -> BTreeSet<String> {
    let anchors = |p: &ModProfile| -> Vec<(String, String)> {
        fixes::fixes(p)
            .filter_map(|f| match &f.change {
                FixChange::Anchor { patch, .. } => Some((
                    patch.id.clone(),
                    serde_json::to_string(&f.change).unwrap_or_default(),
                )),
                _ => None,
            })
            .collect()
    };
    let (a, b) = (anchors(was), anchors(now));
    a.iter()
        .filter(|x| !b.contains(x))
        .chain(b.iter().filter(|x| !a.contains(x)))
        .map(|(id, _)| id.clone())
        .collect()
}

/// Compares the current simulation with a previewed one.
fn preview_impact(current: &SimulationReport, next: &SimulationReport) -> PreviewImpact {
    let broken = |r: &SimulationReport| -> BTreeSet<String> {
        r.conflicts
            .iter()
            .filter(|c| c.kind == "compile_error")
            .map(|c| c.target_file.clone())
            .collect()
    };
    let (was, will) = (broken(current), broken(next));

    let files: BTreeSet<&String> = current.traces.keys().chain(next.traces.keys()).collect();
    let files_changed = files
        .into_iter()
        .filter(|f| current.traces.get(*f).map(|t| &t.text) != next.traces.get(*f).map(|t| &t.text))
        .cloned()
        .collect();

    type Key = (String, String);
    let outcomes = |r: &SimulationReport| -> BTreeMap<Key, (String, PatchOutcomeStatus, u32)> {
        r.files_touched
            .iter()
            .flat_map(|f| {
                f.records.iter().map(move |rec| {
                    (
                        (f.target_file.clone(), rec.patch_id.clone()),
                        (rec.mod_id.clone(), rec.status, to_u32(rec.matched_count)),
                    )
                })
            })
            .collect()
    };
    let (a, b) = (outcomes(current), outcomes(next));
    let keys: BTreeSet<&Key> = a.keys().chain(b.keys()).collect();
    let patch_changes = keys
        .into_iter()
        .filter_map(|k| {
            let (x, y) = (a.get(k), b.get(k));
            if x.map(|v| (v.1, v.2)) == y.map(|v| (v.1, v.2)) {
                return None;
            }
            Some(PatchChange {
                patch_id: k.1.clone(),
                mod_id: x.or(y).map(|v| v.0.clone()).unwrap_or_default(),
                target_file: k.0.clone(),
                before: x.map(|v| v.1),
                after: y.map(|v| v.1),
                before_matches: x.map(|v| v.2).unwrap_or(0),
                after_matches: y.map(|v| v.2).unwrap_or(0),
            })
        })
        .collect();

    let placement = |r: &SimulationReport| -> BTreeSet<String> {
        r.conflicts
            .iter()
            .filter(|c| PLACEMENT_KINDS.contains(&c.kind.as_str()))
            .map(|c| c.id.clone())
            .collect()
    };
    let compile_errors_new: Vec<String> = will.difference(&was).cloned().collect();
    PreviewImpact {
        files_changed,
        patch_changes,
        breaks_file: !compile_errors_new.is_empty(),
        compile_errors_new,
        compile_errors_existing: will.intersection(&was).cloned().collect(),
        compile_errors_fixed: was.difference(&will).cloned().collect(),
        placement_new: placement(next)
            .difference(&placement(current))
            .cloned()
            .collect(),
        stack_changes: stack_changes(current, next),
        relation_warnings: Vec::new(),
        fallbacks_new: fallback_diff(&fallbacks(current), &fallbacks(next)),
        fallbacks_fixed: fallback_diff(&fallbacks(next), &fallbacks(current)),
        fix_status_changes: Vec::new(),
        faction_changes: Vec::new(),
    }
}

/// The sides whose active faction differs between the candidates `before` and `after`, one
/// faction per side as the loader picks it (`choose_active`, D73).
fn faction_diff(
    before: Vec<LoadedFaction>,
    after: Vec<LoadedFaction>,
    picks_before: &BTreeMap<String, String>,
    picks_after: &BTreeMap<String, String>,
) -> Vec<FactionChange> {
    let by_side = |c: Vec<LoadedFaction>,
                   picks: &BTreeMap<String, String>|
     -> BTreeMap<FactionSide, FactionPick> {
        crate::ops::assets::active_factions(c, picks)
            .into_iter()
            .map(|lf| {
                let pick = FactionPick {
                    mod_id: lf.mod_id,
                    faction_id: lf.faction.id,
                    name: lf.faction.name,
                };
                (lf.faction.replaces, pick)
            })
            .collect()
    };
    let (was, will) = (by_side(before, picks_before), by_side(after, picks_after));
    [FactionSide::Allied, FactionSide::Axis]
        .into_iter()
        .filter_map(|side| {
            let (x, y) = (was.get(&side), will.get(&side));
            (x != y).then(|| FactionChange {
                side: side.as_str().to_string(),
                before: x.cloned(),
                after: y.cloned(),
            })
        })
        .collect()
}

/// Stacks found in both runs whose patches run in a different order. A stack is matched by
/// file, side, line text and its place among stacks with the same text, so line shifts from
/// other changes do not matter.
fn stack_changes(current: &SimulationReport, next: &SimulationReport) -> Vec<StackChange> {
    type Key = (String, u8, String, usize);
    let index = |r: &SimulationReport| -> BTreeMap<Key, SharedStack> {
        let mut seen: HashMap<(String, u8, String), usize> = HashMap::new();
        let mut out = BTreeMap::new();
        for s in r.files_touched.iter().flat_map(|f| &f.stacks) {
            let side = match s.side {
                Side::Before => 0,
                Side::After => 1,
            };
            let n = seen
                .entry((s.target_file.clone(), side, s.anchor_text.clone()))
                .or_default();
            out.insert(
                (s.target_file.clone(), side, s.anchor_text.clone(), *n),
                s.clone(),
            );
            *n += 1;
        }
        out
    };
    let (was, will) = (index(current), index(next));
    let mut changes = Vec::new();
    for (key, a) in &was {
        let Some(b) = will.get(key) else {
            continue;
        };
        // Only the order of patches in both runs counts; added or removed code is a patch change.
        let ids = |s: &SharedStack| -> Vec<String> {
            s.blocks.iter().map(|x| x.patch_id.clone()).collect()
        };
        let (ia, ib) = (ids(a), ids(b));
        let common = |v: &[String], other: &[String]| -> Vec<String> {
            v.iter().filter(|id| other.contains(id)).cloned().collect()
        };
        if common(&ia, &ib) != common(&ib, &ia) {
            changes.push(StackChange {
                file: b.target_file.clone(),
                line: b.file_line,
                side: b.side,
                anchor_text: b.anchor_text.clone(),
                before: a.blocks.clone(),
                after: b.blocks.clone(),
            });
        }
    }
    changes.sort_by(|x, y| (&x.file, x.line).cmp(&(&y.file, y.line)));
    changes
}
