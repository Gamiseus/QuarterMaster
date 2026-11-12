//! The fix ledger commands (Phase C §10, Task 12): the Fixes view, creating, pausing,
//! accepting, re-binding and undoing fixes. Every write migrates an unmigrated profile first
//! (D38) and goes through `commit_active_profile`.
use crate::cache::{PlanView, SuggestionsState, VanillaFiles};
use crate::dto::{
    DriftDiff, FixChangeDraft, FixDraft, FixStatus, FixSuggestion, FixView, FixesView,
    JournalClash, JournalUndoChoice, JournalUndoPreview, JournalView, PatchLabel, PreviewImpact,
    RelationWarning, StackChange, UndoPreview,
};
use crate::error::{ErrorKind, QmError, QmResult};
use crate::events::Scope;
use crate::jobs::{JobCtx, JobKind};
use crate::ops::patch_tools::MSG_NOT_READY;
use crate::service::Service;
use qm_core::handshake::{BindingStatus, FixBinding, LoadOrderPlan, RunAs, StackReason};
use qm_core::lenient::Lenient;
use qm_core::patch::anchor::Rung;
use qm_core::patch::edits::{self, ApplyAs};
use qm_core::patch::fingerprint::PatchSnapshot;
use qm_core::patch::order::{matching_patches, patch_name, OrderRule};
use qm_core::patch::plan::{self, FpMode, ModGraph, ModMeta, PlanReport, RawLoad};
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, Patch};
use qm_manager::fixes::{
    self, game_stamp::GameHash, FixChange, FixEventKind, FixSource, GameFileDrift, JournalEntry,
    ProfileFix, Provenance, FIXES_VERSION,
};
use qm_manager::suggestions::{self, SuggestCtx};
use qm_manager::{ConflictResolutionSuggestion, ModProfile, SimulationReport};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub(super) const MSG_IMPORT: &str = "Your existing fixes could not be imported yet";
const MSG_GONE: &str = "That fix is already gone.";
const MSG_PLAN_NOTE: &str =
    "The game's load order file could not be read, so this view uses the profile's mod order.";
/// Why the Fixes view has no suggestions (D70).
pub(crate) const MSG_SUGGEST_NO_GAME: &str =
    "No suggested fixes: the game files could not be read. Check the game folder in Settings.";
pub(crate) const MSG_SUGGEST_NO_MODS: &str =
    "No suggested fixes: the profile's mods could not be read.";
pub(crate) const MSG_SUGGEST_FAILED: &str =
    "No suggested fixes: the check failed. The log has the details.";

fn gone() -> QmError {
    QmError::new(ErrorKind::NotFound, MSG_GONE)
}

/// Journal step `entry_index` of `p`, or why it cannot be undone.
fn journal_entry(p: &ModProfile, entry_index: u32) -> QmResult<JournalEntry> {
    p.fix_journal
        .get(entry_index as usize)
        .cloned()
        .ok_or_else(|| QmError::new(ErrorKind::NotFound, "That step is already undone."))?
        .into_ok()
        .ok_or_else(|| {
            QmError::new(
                ErrorKind::InvalidInput,
                "A newer QuarterMaster made this step; undo it there.",
            )
        })
}

/// The running fixes of `p` that each fix `entry` brings back would clash with (§7.5), as
/// (restored fix, newer fix id). `p` no longer holds the fixes the step added.
fn journal_clashes(p: &ModProfile, entry: &JournalEntry) -> Vec<(ProfileFix, String)> {
    entry
        .removed
        .iter()
        .filter_map(|f| f.ok())
        .filter(|f| !f.paused)
        .flat_map(|f| {
            fixes::running_clashes(p, f)
                .into_iter()
                .map(move |id| (f.clone(), id))
        })
        .collect()
}

impl Service {
    /// The mods as read for the active profile: the cached copy, else read from the staged
    /// folder (and cached). None when the staged plan cannot be read.
    pub(crate) fn current_raw(&self) -> Option<Arc<RawLoad>> {
        if let Some(raw) = self.inner.caches.raw_load.get() {
            return Some(raw);
        }
        let staged = LoadOrderPlan::read(&self.staged_mods_dir());
        if staged.error.is_some() {
            return None;
        }
        let table = self
            .inner
            .caches
            .patch_table
            .get()
            .unwrap_or_else(|| Arc::new(PatchTable::default()));
        self.preview_raw(&table, staged.plan.as_ref()).ok()
    }

    /// The cached vanilla files, when there are any.
    pub(crate) fn vanilla_files(&self) -> Option<Arc<VanillaFiles>> {
        self.inner
            .caches
            .vanilla
            .get()
            .filter(|v| !v.files.is_empty())
    }

    /// The hash of game file `file` in the cached vanilla files (D74).
    pub(crate) fn game_hash(&self, vanilla: &Arc<VanillaFiles>, file: &str) -> Option<String> {
        self.inner.caches.vanilla_hashes.get(vanilla, file)
    }

    /// Stamps the fixes of `p` made or accepted since it was read, and those still waiting
    /// for a stamp, with the hashes of the game files their patches target (D74). Without
    /// the game files or the mod load they are marked to wait.
    pub(crate) fn stamp_fresh_fixes(&self, p: &mut ModProfile) {
        if !fixes::fixes(p).any(|f| f.fresh || f.stamp_pending) {
            return;
        }
        let raw = self.current_raw();
        let Some(vanilla) = self.vanilla_files() else {
            fixes::game_stamp::stamp_fresh(p, raw.as_deref(), None);
            return;
        };
        let files: Vec<String> = vanilla.files.keys().cloned().collect();
        let hash = |f: &str| self.game_hash(&vanilla, f);
        fixes::game_stamp::stamp_fresh(p, raw.as_deref(), Some((&files, &hash)));
    }

    /// Stamps the active profile's fixes that wait for a stamp, once the game files and the
    /// mod load are there (after a simulation). Saves only the profile file: a stamp does not
    /// change the plan. Left for a later pass while the game runs.
    ///
    /// `gen` and `writes` are the simulation generation and the profile write count the
    /// simulation pass started with. The read, stamp and save run under the profile-write
    /// lock, and only while neither changed: a user commit in between is never overwritten,
    /// and a newer profile is never stamped with an older pass's mod load (its own commit or
    /// the next stored simulation stamps it).
    pub(crate) fn stamp_pending_fixes(&self, gen: u32, writes: u64) {
        if self
            .inner
            .game
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_busy()
        {
            return;
        }
        let mut n = self
            .inner
            .profile_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if *n != writes || self.generations().sim != gen {
            return;
        }
        let Ok(mut p) = self.active_profile() else {
            return;
        };
        if p.id == qm_manager::VANILLA_ID || !fixes::game_stamp::has_pending(&p) {
            return;
        }
        let Some(vanilla) = self.vanilla_files() else {
            return;
        };
        let Some(raw) = self.current_raw() else {
            return;
        };
        let files: Vec<String> = vanilla.files.keys().cloned().collect();
        let hash = |f: &str| self.game_hash(&vanilla, f);
        if fixes::game_stamp::stamp_fresh(&mut p, Some(&raw), Some((&files, &hash))) == 0 {
            return;
        }
        *n = n.wrapping_add(1);
        let saved = self.paths().save_profile(&p);
        drop(n);
        match saved {
            Ok(()) => self.notify(Scope::Profiles),
            Err(e) => tracing::warn!("could not save the game stamps of {}: {e}", p.id),
        }
    }

    /// `p` migrated in memory when it is not yet (§7.4), or why that failed. A migrated
    /// profile gets hand-edited legacy entries adopted as fixes.
    pub(crate) fn migrated_in_memory(
        &self,
        mut p: ModProfile,
        raw: Option<&RawLoad>,
    ) -> Result<(ModProfile, bool), String> {
        if p.fixes_version >= FIXES_VERSION {
            fixes::reconcile(&mut p, raw);
            return Ok((p, false));
        }
        let empty = RawLoad::default();
        let raw = match raw {
            Some(raw) => raw,
            None if !fixes::has_legacy_entries(&p) => &empty,
            None => return Err("the profile's mods could not be read".to_string()),
        };
        let vanilla = self.vanilla_files();
        fixes::migrate(&p, raw, vanilla.as_ref().map(|v| &v.files)).map(|m| (m, true))
    }

    /// The active profile for a ledger write: migrated (with the backup made) or refused.
    pub(crate) fn ledger_profile(&self) -> QmResult<(ModProfile, Option<Arc<RawLoad>>)> {
        self.ledger_profile_of(self.active_profile()?)
    }

    /// `ledger_profile` for `p`, the active profile as the caller changed it.
    fn ledger_profile_of(&self, p: ModProfile) -> QmResult<(ModProfile, Option<Arc<RawLoad>>)> {
        let raw = self.current_raw();
        match self.migrated_in_memory(p, raw.as_deref()) {
            Ok((m, now)) => {
                if now && fixes::has_legacy_entries(&m) {
                    self.paths()
                        .back_up_before_fixes(&m.id)
                        .map_err(|e| QmError::new(ErrorKind::Io, e))?;
                }
                Ok((m, raw))
            }
            Err(why) => Err(QmError::new(
                ErrorKind::InvalidInput,
                format!("{MSG_IMPORT}: {why}"),
            )),
        }
    }

    /// The active profile for a legacy command: migrated when it can be, else as it is, so
    /// the legacy commands keep working on the legacy fields (D38).
    pub(crate) fn writable_profile(&self) -> QmResult<(ModProfile, Option<Arc<RawLoad>>)> {
        let p = self.active_profile()?;
        let raw = self.current_raw();
        match self.migrated_in_memory(p.clone(), raw.as_deref()) {
            Ok((m, now)) => {
                if now && fixes::has_legacy_entries(&m) {
                    self.paths()
                        .back_up_before_fixes(&m.id)
                        .map_err(|e| QmError::new(ErrorKind::Io, e))?;
                }
                Ok((m, raw))
            }
            Err(why) => {
                tracing::warn!("{MSG_IMPORT}: {why}");
                Ok((p, raw))
            }
        }
    }

    /// The conflict a resolver suggestion came from, from the cached simulation.
    pub(crate) fn conflict_of(
        &self,
        suggestion: &ConflictResolutionSuggestion,
    ) -> Option<(String, String)> {
        let report = self.inner.caches.simulation.get()?;
        let want = serde_json::to_value(suggestion).ok()?;
        report
            .conflicts
            .iter()
            .find(|c| {
                c.suggestions
                    .iter()
                    .any(|s| serde_json::to_value(s).ok().as_ref() == Some(&want))
            })
            .map(|c| (c.id.clone(), c.kind.clone()))
    }

    /// The kind of the cached simulation's conflict `id`.
    pub(crate) fn conflict_kind(&self, id: &str) -> Option<String> {
        let report = self.inner.caches.simulation.get()?;
        report
            .conflicts
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.kind.clone())
    }

    /// Lowercase mod id -> display name, from the installed mods.
    pub(crate) fn mod_names(&self) -> HashMap<String, String> {
        self.installed_mods()
            .iter()
            .map(|m| (m.manifest.id.to_lowercase(), m.manifest.name.clone()))
            .collect()
    }

    /// The relations of the enabled mods, from their mod.json files (D4).
    pub(crate) fn enabled_graph(&self, p: &ModProfile) -> (ModGraph, Vec<ModMeta>) {
        let lower = |v: &mut dyn Iterator<Item = &String>| -> BTreeSet<String> {
            v.map(|s| s.to_lowercase()).collect()
        };
        let metas: Vec<ModMeta> = self
            .installed_mods()
            .iter()
            .filter(|m| p.enabled_mods.contains(&m.manifest.id))
            .map(|m| {
                let r = &m.manifest.relationships;
                ModMeta {
                    id: m.manifest.id.to_lowercase(),
                    folder: m.manifest.id.clone(),
                    version: Some(m.manifest.version.clone()),
                    requires: lower(&mut r.requires.keys()),
                    load_after: lower(&mut r.load_after.iter()),
                    load_before: lower(&mut r.load_before.iter()),
                    provides: lower(&mut r.provides.iter()),
                    manifest_ok: true,
                    ..ModMeta::default()
                }
            })
            .collect();
        (ModGraph::build(&metas), metas)
    }

    /// D39: rules that run a patch of mod M before a patch of a mod M depends on.
    pub(crate) fn relation_warnings(
        &self,
        table: &PatchTable,
        rules: &[OrderRule],
    ) -> Vec<RelationWarning> {
        let graph = ModGraph::build(&table.mods);
        let names = self.mod_names();
        let name = |m: &str| names.get(m).cloned().unwrap_or_else(|| m.to_string());
        let mods_of = |n: &str| -> BTreeSet<String> {
            matching_patches(n, &table.patches)
                .iter()
                .filter_map(|&i| table.patches.get(i))
                .filter_map(|(p, _, _)| p.mod_id().map(str::to_lowercase))
                .collect()
        };
        let requires = |a: &str, b: &str| {
            table
                .mods
                .iter()
                .find(|m| m.id.eq_ignore_ascii_case(a))
                .is_some_and(|m| graph.requires_resolved(&m.id).contains(&b.to_lowercase()))
        };
        let mut out: Vec<RelationWarning> = Vec::new();
        let mut warn = |rule: &OrderRule, first: &str, then: &str| {
            // `first` runs before `then`; bad when `first` depends on `then`.
            if first == then || !graph.depends_on(first, then) {
                return;
            }
            let verb = if requires(first, then) {
                "requires"
            } else {
                "loads after"
            };
            let w = RelationWarning {
                rule: rule.patch.clone(),
                mod_id: first.to_string(),
                depends_on: then.to_string(),
                text: format!(
                    "{a} {verb} {b}; this runs {a}'s patch before {b}'s",
                    a = name(first),
                    b = name(then)
                ),
            };
            if !out.contains(&w) {
                out.push(w);
            }
        };
        for rule in rules {
            let subject = mods_of(&rule.patch);
            for b in &rule.before {
                for then in mods_of(b) {
                    for first in &subject {
                        warn(rule, first, &then);
                    }
                }
            }
            for a in &rule.after {
                for first in mods_of(a) {
                    for then in &subject {
                        warn(rule, &first, then);
                    }
                }
            }
        }
        out
    }

    /// Adds `draft` to `p` (migrated); the new or existing fix's id.
    pub(crate) fn apply_draft(
        &self,
        p: &mut ModProfile,
        raw: Option<&RawLoad>,
        table: Option<&PatchTable>,
        draft: &FixDraft,
    ) -> QmResult<String> {
        let provenance = Provenance {
            conflict_id: draft.conflict_id.clone(),
            option_key: draft.option_key.clone(),
            approved: draft.approved.clone(),
            ..Provenance::new(draft.source)
        };
        let before: HashSet<String> = fixes::fixes(p).map(|f| f.id.clone()).collect();
        let id = match &draft.change {
            FixChangeDraft::Disable { patch_id } => {
                let patch_id = patch_id.trim();
                if patch_id.is_empty() {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        "Pick a patch to turn off.",
                    ));
                }
                let mut aliases: HashSet<String> =
                    self.patch_aliases(patch_id).into_iter().collect();
                aliases.extend(self.enable_aliases(table, patch_id));
                fixes::op_disable(p, raw, patch_id, &aliases, provenance)
            }
            FixChangeDraft::Order { rule } => {
                if rule.patch.trim().is_empty() || (rule.before.is_empty() && rule.after.is_empty())
                {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        "An order rule needs a patch and at least one patch to run before or after.",
                    ));
                }
                fixes::op_order_set(p, raw, rule.clone(), provenance)
            }
            FixChangeDraft::Priority { patch_id, priority } => {
                let patch_id = patch_id.trim();
                if patch_id.is_empty() {
                    return Err(QmError::new(ErrorKind::InvalidInput, "Pick a patch."));
                }
                fixes::op_priority_set(p, raw, patch_id, *priority, provenance)
            }
            FixChangeDraft::Anchor {
                patch_id,
                anchors,
                sites,
            } => {
                let patch_id = patch_id.trim();
                if patch_id.is_empty() || anchors.is_empty() {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        "Pick a patch and a line to put its code next to.",
                    ));
                }
                if anchors.iter().any(|r| {
                    r.within.is_none() && r.pattern.as_deref().is_none_or(|t| t.trim().is_empty())
                }) {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        "Each place needs a patch or a line to go next to.",
                    ));
                }
                self.check_anchor_draft(table, patch_id, anchors, sites)?;
                fixes::op_anchor_set(
                    p,
                    raw,
                    patch_id,
                    anchors.clone(),
                    sites.clone(),
                    None,
                    provenance,
                )
            }
            FixChangeDraft::ApplyAs { patch_id, mode } => {
                let patch_id = patch_id.trim();
                if patch_id.is_empty() {
                    return Err(QmError::new(ErrorKind::InvalidInput, "Pick a patch."));
                }
                check_apply_as_draft(table, patch_id)?;
                fixes::op_apply_as_set(p, raw, patch_id, *mode, None, provenance)
            }
            FixChangeDraft::SkipHunk {
                patch_id,
                ordinal,
                keep_patch_id,
            } => {
                let patch_id = patch_id.trim();
                if patch_id.is_empty() {
                    return Err(QmError::new(ErrorKind::InvalidInput, "Pick a patch."));
                }
                let raw = raw.ok_or_else(|| {
                    QmError::new(ErrorKind::Conflict, "The profile's mods could not be read.")
                })?;
                let keep = keep_patch_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|k| !k.is_empty())
                    .map(str::to_string);
                let hunk = fixes::hunk_key(raw, patch_id, *ordinal, keep)
                    .map_err(|e| QmError::new(ErrorKind::InvalidInput, e))?;
                fixes::op_hunk_skip(p, Some(raw), patch_id, hunk, None, provenance)
            }
            FixChangeDraft::StackRule {
                patch_id,
                site,
                side,
                below_patch_id,
                above_patch_id,
            } => {
                let patch_id = patch_id.trim();
                let clean = |x: &Option<String>| {
                    x.as_deref()
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                        .map(str::to_string)
                };
                let (below, above) = (clean(below_patch_id), clean(above_patch_id));
                let other = match (&below, &above) {
                    (Some(o), None) | (None, Some(o)) => o.clone(),
                    _ => {
                        return Err(QmError::new(
                            ErrorKind::InvalidInput,
                            "Pick one block to put it below or above.",
                        ))
                    }
                };
                if patch_id.is_empty() || other == patch_id {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        "Pick two different blocks.",
                    ));
                }
                if site.context.trim().is_empty() {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        "Code can only be ordered at a line of the game's file.",
                    ));
                }
                let line = self.stack_line_text(site, *side);
                let label = fixes::stack_label(
                    raw,
                    patch_id,
                    below.as_deref().map(|b| fixes::pin(raw, b, false)).as_ref(),
                    above.as_deref().map(|a| fixes::pin(raw, a, false)).as_ref(),
                    line.as_deref(),
                );
                fixes::op_stack_set(
                    p,
                    raw,
                    patch_id,
                    site.clone(),
                    *side,
                    below.as_deref(),
                    above.as_deref(),
                    Some(label),
                    provenance,
                )
            }
            FixChangeDraft::AssetChoice { key, winner } => {
                let (key, winner) = (key.trim(), winner.trim());
                let Some(raw) = raw else {
                    return Err(QmError::new(
                        ErrorKind::Conflict,
                        "The patch simulation is still running. Try again in a moment.",
                    ));
                };
                // What the choice leaves out, from the ledger as it is now (D78).
                let (_, _, excludes) = self.ledger_for_choice(p, raw, key, winner)?;
                let label = Some(draft.label.clone()).filter(|l| !l.trim().is_empty());
                fixes::op_asset_choice_set(p, key, winner, excludes, label, provenance)
            }
            FixChangeDraft::AdjustedCopyRunAs { glue_id, run_as } => {
                let glue_id = glue_id.trim();
                let Some(copy) = fixes::copy_fix(p, glue_id) else {
                    return Err(QmError::new(
                        ErrorKind::NotFound,
                        "That adjusted copy is already gone.",
                    ));
                };
                if copy.paused {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        "The copy is paused. Resume it first.",
                    ));
                }
                let base_gone = match (&copy.change, raw) {
                    (FixChange::AdjustedCopy { glue, .. }, Some(raw)) => {
                        *run_as == RunAs::Base && fixes::copy_base_mod(p, raw, glue).is_none()
                    }
                    _ => false,
                };
                if base_gone {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        "The patch this copy stands in for is not loaded, so the copy cannot run like its mod.",
                    ));
                }
                fixes::op_glue_run_as(p, raw, glue_id, *run_as)
            }
        };
        let id = id.ok_or_else(|| {
            QmError::new(
                ErrorKind::Internal,
                "The profile's fixes are not imported yet.",
            )
        })?;
        let label = draft.label.trim();
        if !label.is_empty() && !before.contains(&id) {
            if let Some(f) = fixes::find_mut(p, &id) {
                f.label = label.to_string();
            }
        }
        Ok(id)
    }

    /// The text of the shared line `site` names on `side`, from the current simulation.
    fn stack_line_text(
        &self,
        site: &qm_core::patch::anchor::SiteKey,
        side: qm_core::patch::linemap::Side,
    ) -> Option<String> {
        let report = self.inner.caches.simulation.get()?;
        report
            .files_touched
            .iter()
            .flat_map(|f| &f.stacks)
            .find(|s| s.site == *site && s.side == side)
            .map(|s| s.anchor_text.clone())
    }

    /// D56: stack rules that put a mod's block above the block of a mod it depends on. They
    /// warn; they do not block. Besides the pair a draft names, every pair the preview's
    /// `stack_changes` turn around counts: a move can pass over several blocks, and a
    /// dependency it passes over ends below its dependent too.
    pub(crate) fn stack_relation_warnings(
        &self,
        table: &PatchTable,
        drafts: &[FixDraft],
        stack_changes: &[StackChange],
    ) -> Vec<RelationWarning> {
        let graph = ModGraph::build(&table.mods);
        let names = self.mod_names();
        let name = |m: &str| names.get(m).cloned().unwrap_or_else(|| m.to_string());
        let mod_of = |n: &str| -> Option<String> {
            matching_patches(n, &table.patches)
                .first()
                .and_then(|&i| table.patches.get(i))
                .and_then(|(p, _, _)| p.mod_id().map(str::to_lowercase))
        };
        let requires = |a: &str, b: &str| {
            table
                .mods
                .iter()
                .find(|m| m.id.eq_ignore_ascii_case(a))
                .is_some_and(|m| graph.requires_resolved(&m.id).contains(&b.to_lowercase()))
        };
        let mut out: Vec<RelationWarning> = Vec::new();
        let mut warn = |rule: &str, upper: &str, lower: &str| {
            if upper == lower || !graph.depends_on(upper, lower) {
                return;
            }
            if out
                .iter()
                .any(|w| w.mod_id == upper && w.depends_on == lower)
            {
                return;
            }
            let verb = if requires(upper, lower) {
                "requires"
            } else {
                "loads after"
            };
            out.push(RelationWarning {
                rule: rule.to_string(),
                mod_id: upper.to_string(),
                depends_on: lower.to_string(),
                text: format!(
                    "{a} {verb} {b}; putting {a}'s block above {b}'s may break it",
                    a = name(upper),
                    b = name(lower)
                ),
            });
        };
        let rules: Vec<&str> = drafts
            .iter()
            .filter_map(|d| match &d.change {
                FixChangeDraft::StackRule { patch_id, .. } => Some(patch_id.as_str()),
                _ => None,
            })
            .collect();
        for d in drafts {
            let FixChangeDraft::StackRule {
                patch_id,
                below_patch_id,
                above_patch_id,
                ..
            } = &d.change
            else {
                continue;
            };
            let Some(me) = mod_of(patch_id) else { continue };
            // (upper, lower): the upper block's code runs first.
            let pairs = below_patch_id
                .as_deref()
                .and_then(mod_of)
                .map(|o| (o, me.clone()))
                .into_iter()
                .chain(
                    above_patch_id
                        .as_deref()
                        .and_then(mod_of)
                        .map(|o| (me.clone(), o)),
                );
            for (upper, lower) in pairs {
                warn(patch_id, &upper, &lower);
            }
        }
        // Each pair of blocks the change turns around at a shared line: the upper block now
        // runs first, and did not before.
        for c in stack_changes {
            let at =
                |v: &[qm_manager::StackBlock], id: &str| v.iter().position(|b| b.patch_id == id);
            for (i, upper) in c.after.iter().enumerate() {
                for lower in &c.after[i + 1..] {
                    let (Some(bu), Some(bl)) = (
                        at(&c.before, &upper.patch_id),
                        at(&c.before, &lower.patch_id),
                    ) else {
                        continue;
                    };
                    if bu < bl {
                        continue;
                    }
                    let rule = rules
                        .iter()
                        .find(|r| **r == upper.patch_id || **r == lower.patch_id)
                        .or(rules.first())
                        .copied()
                        .unwrap_or(upper.patch_id.as_str());
                    warn(
                        rule,
                        &upper.mod_id.to_lowercase(),
                        &lower.mod_id.to_lowercase(),
                    );
                }
            }
        }
        out
    }

    /// Refuses an Anchor draft the engine would never use (§3.6, D6, D25, D52): a patch that
    /// is neither a pattern patch adding code before or after its line nor a regex patch, an
    /// edge for a regex that reads or replaces its match, a rung at `at`, an edge with a
    /// pattern or without a patch, a `mod:*` patch, or a move of one place of a patch with
    /// several without saying which.
    fn check_anchor_draft(
        &self,
        table: Option<&PatchTable>,
        patch_id: &str,
        anchors: &[Rung],
        sites: &[qm_core::patch::anchor::SiteKey],
    ) -> QmResult<()> {
        let bad = |m: String| Err(QmError::new(ErrorKind::InvalidInput, m));
        for r in anchors {
            if r.position == Some(InsertPosition::At) {
                return bad("Code can only go before or after a line, not replace it.".to_string());
            }
            if r.edge.is_some() && (r.pattern.is_some() || r.within.is_none()) {
                return bad(
                    "Pick either the start or end of a change, or a line inside it.".to_string(),
                );
            }
            if r.within
                .as_deref()
                .is_some_and(|w| w.trim().ends_with(":*"))
            {
                return bad("Pick one patch to put the code next to.".to_string());
            }
        }
        let Some(table) = table else {
            return Ok(());
        };
        let Some(i) = table
            .patches
            .iter()
            .position(|(p, _, _)| p.id() == Some(patch_id))
            .or_else(|| matching_patches(patch_id, &table.patches).first().copied())
        else {
            return Ok(());
        };
        let Some((patch, _, _)) = table.patches.get(i) else {
            return Ok(());
        };
        let name = patch.name().unwrap_or(patch_id);
        let target = match patch {
            Patch::Pattern(a) if a.position != InsertPosition::At => a.target.primary(),
            Patch::Pattern(_) => {
                return bad(format!(
                    "{name} replaces lines, so it cannot be moved this way. Make an adjusted copy instead."
                ))
            }
            // A regex takes rungs (D52); an edge only when it inserts text it does not match.
            Patch::Regex(r) => {
                if anchors.iter().any(|x| x.edge.is_some()) && !r.takes_edge_rungs() {
                    return bad(format!(
                        "{name} uses or replaces the text it matches, so it cannot go at the start or end of a change. Pick a line inside it."
                    ));
                }
                let moved = |x: &Rung| x.edge.is_none() && x.position.is_some_and(|p| p != r.position);
                if anchors.iter().any(moved) {
                    return bad(format!(
                        "{name} places its code by its own position, so a line can only be picked for it as it is."
                    ));
                }
                r.target.primary()
            }
            _ => {
                return bad(format!(
                    "{name} is not a pattern or regex patch, so it cannot be moved this way. Make an adjusted copy instead."
                ))
            }
        };
        let text_rung = anchors.iter().any(|r| r.within.is_none());
        if text_rung && sites.len() != 1 {
            let target = target.to_string();
            if let Some(buf) = self
                .vanilla_files()
                .and_then(|v| v.files.get(&target).cloned())
                .map(|src| qm_manager::region::prepared_original(table, &target, &src))
            {
                if qm_manager::region::patch_sites(patch, &buf).len() > 1 {
                    return bad(format!("Pick which of {name}'s places to move."));
                }
            }
        }
        Ok(())
    }

    /// Each fix's status in `p` under `table` (its plan bindings and order results) and the
    /// simulation `report` of it.
    pub(crate) fn fix_statuses(
        &self,
        p: &ModProfile,
        raw: Option<&RawLoad>,
        table: &PatchTable,
        report: &SimulationReport,
    ) -> BTreeMap<String, FixStatus> {
        let names = self.mod_names();
        let plan = table.prepare().report().clone();
        let vanilla = self.vanilla_files();
        let hash = |f: &str| vanilla.as_ref().and_then(|v| self.game_hash(v, f));
        let ctx = ViewCtx {
            p,
            raw,
            bindings: &plan.bindings,
            order: Some((&plan, table)),
            records: Some(&report.files_touched[..]),
            names: &names,
            legacy: false,
            game: vanilla.is_some().then_some(&hash as GameHash),
        };
        fixes::fixes(p)
            .map(|f| (f.id.clone(), ctx.status(f)))
            .collect()
    }

    /// The bindings the game would report for `p` (every fingerprint checked), the order
    /// results, and the table they come from, in the order of the staged plan `staged`.
    fn plan_view_for(
        &self,
        p: &ModProfile,
        raw: &RawLoad,
        staged: Option<&LoadOrderPlan>,
    ) -> Option<(PlanReport, PatchTable)> {
        let (enabled, prios) = Self::order_of(p, staged);
        let compiled = qm_manager::build_plan(p, enabled, prios);
        let json = serde_json::to_string(&compiled).ok()?;
        self.inner
            .caches
            .plan_views_built
            .fetch_add(1, Ordering::Relaxed);
        let applied = plan::apply_plan_with(raw, &LoadOrderPlan::parse(&json), FpMode::All);
        let table = PatchTable::from_applied(applied, raw.mod_dir.clone());
        let report = table.prepare().report().clone();
        Some((report, table))
    }

    /// The staged load-order.json, and why it cannot be used when it is there but broken
    /// (`FixesView.plan_note`).
    fn staged_plan(&self) -> (Option<LoadOrderPlan>, Option<String>) {
        let (plan, note) = self.staged_plan_quiet();
        if let Some(note) = &note {
            tracing::warn!("the staged load-order.json cannot be used: {note}");
        }
        (plan, note)
    }

    /// `staged_plan` without the log line.
    fn staged_plan_quiet(&self) -> (Option<LoadOrderPlan>, Option<String>) {
        let read = LoadOrderPlan::read(&self.staged_mods_dir());
        let note = read.error.map(|e| format!("{MSG_PLAN_NOTE} {e}."));
        (read.plan, note)
    }

    /// The enabled mods and mod priorities of the staged plan, else the profile's mods. A
    /// broken staged plan is logged here and noted in the Fixes view.
    pub(crate) fn staged_order(&self, p: &ModProfile) -> (Vec<String>, HashMap<String, i32>) {
        Self::order_of(p, self.staged_plan().0.as_ref())
    }

    /// The enabled mods and mod priorities of `staged`, else the profile's mods.
    fn order_of(
        p: &ModProfile,
        staged: Option<&LoadOrderPlan>,
    ) -> (Vec<String>, HashMap<String, i32>) {
        match staged {
            Some(plan) => (
                plan.enabled_mods.clone(),
                plan.mod_priorities
                    .iter()
                    .filter_map(|(k, v)| v.ok().map(|v| (k.clone(), *v)))
                    .collect(),
            ),
            None => (p.enabled_mods.clone(), HashMap::new()),
        }
    }

    /// The plan view of `p` (`plan_view_for`) and the staged plan's note it was built with,
    /// cached under the simulation generation, the profile key `key` (like the suggestions,
    /// D70) and that note, so a staged plan that breaks or heals builds it again. `gen` is
    /// the generation read before `raw`, so a view of an older raw is never kept under a
    /// newer generation.
    fn plan_view(
        &self,
        p: &ModProfile,
        key: u64,
        raw: &RawLoad,
        gen: u32,
    ) -> (Option<Arc<PlanView>>, Option<String>) {
        let (staged, note) = self.staged_plan_quiet();
        if let Some((g, k, view, n)) = self.inner.caches.plan_view.get() {
            if g == gen && k == key && n == note {
                return (view, note);
            }
        }
        if let Some(note) = &note {
            tracing::warn!("the staged load-order.json cannot be used: {note}");
        }
        let view = self.plan_view_for(p, raw, staged.as_ref()).map(Arc::new);
        self.with_gens(|g| {
            if g.sim == gen {
                let entry = (gen, key, view.clone(), note.clone());
                self.inner.caches.plan_view.set(entry);
            }
        });
        (view, note)
    }

    /// The key the suggestions and the plan view of profile `read` (as read, before
    /// `detach_missing`) are cached under. Canonical: maps are sorted, so equal profiles always
    /// give the same key.
    fn profile_key(read: &ModProfile) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_value(read)
            .map(|v| v.to_string())
            .unwrap_or_default()
            .hash(&mut h);
        h.finish()
    }

    /// The cached suggestions, when they were made for this simulation and profile key.
    fn cached_suggestions(&self, key: u64) -> Option<Arc<SuggestionsState>> {
        let gen = self.generations().sim;
        let (g, k, state) = self.inner.caches.suggestions.get()?;
        (g == gen && k == key).then_some(state)
    }

    /// A simulation is running or will run: a `Simulation` job, or a `WarmVanilla` job (it
    /// asks for one when it is done).
    fn simulation_pending(&self) -> bool {
        self.jobs()
            .list()
            .iter()
            .any(|j| matches!(j.kind, JobKind::Simulation | JobKind::WarmVanilla))
    }

    /// One `FixSuggestions` pass (§9.3, D70): makes the active profile's suggestions off the
    /// `fixes_list` path, caches the result, and always ends with `Invalidated { Fixes }`, so
    /// a view that showed them as pending reads again. When they cannot be made, the cached
    /// result says why. Waits for a running simulation first, so it never works from the
    /// report that simulation replaces. When the simulation or the profile changed meanwhile,
    /// it runs once more before the event, so a read that joined it is not left pending.
    /// Never writes.
    pub(crate) fn suggestions_pass(&self, ctx: &JobCtx) -> QmResult<Option<String>> {
        self.wait_for_simulation(ctx);
        let gen = self.generations().sim;
        if let Ok(read) = self.active_profile() {
            let key = Self::profile_key(&read);
            // Test seam (`testing::on_next_suggestions_pass`).
            let hook = self
                .inner
                .on_suggestions_pass
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(hook) = hook {
                hook();
            }
            if read.fixes_version >= FIXES_VERSION && self.cached_suggestions(key).is_none() {
                let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.make_suggestions(ctx, read, key, gen)
                }))
                .unwrap_or_else(|p| {
                    tracing::error!("suggested fixes failed: {}", panic_text(p.as_ref()));
                    Some(SuggestionsState::Unavailable(MSG_SUGGEST_FAILED.into()))
                });
                if let Some(state) = made {
                    self.with_gens(|g| {
                        if g.sim == gen {
                            let state = Arc::new(state);
                            self.inner.caches.suggestions.set((gen, key, state));
                        }
                    });
                }
            }
        }
        if !ctx.is_cancelled() && self.suggestions_missing() {
            self.refresh(JobKind::FixSuggestions);
        }
        self.notify(Scope::Fixes);
        Ok(None)
    }

    /// Waits while a `Simulation` or `WarmVanilla` job is listed (until cancelled).
    fn wait_for_simulation(&self, ctx: &JobCtx) {
        loop {
            let running = self
                .jobs()
                .list()
                .into_iter()
                .find(|j| matches!(j.kind, JobKind::Simulation | JobKind::WarmVanilla));
            match running {
                Some(j) if !ctx.is_cancelled() => {
                    self.jobs()
                        .wait_for(j.id, std::time::Duration::from_millis(500));
                }
                _ => break,
            }
        }
    }

    /// The suggestions a view reads now are not cached, though they could be made: the
    /// active profile is migrated and the simulation and game files are there. After a pass
    /// this means the simulation or the profile changed while it ran.
    fn suggestions_missing(&self) -> bool {
        let Ok(read) = self.active_profile() else {
            return false;
        };
        read.fixes_version >= FIXES_VERSION
            && self.cached_suggestions(Self::profile_key(&read)).is_none()
            && self.inner.caches.simulation.get().is_some()
            && self.inner.caches.vanilla.get().is_some()
    }

    /// The suggestions for the profile `read` under simulation generation `gen`, or why there
    /// are none. None when the simulation is not there yet or changed meanwhile.
    fn make_suggestions(
        &self,
        ctx: &JobCtx,
        read: ModProfile,
        key: u64,
        gen: u32,
    ) -> Option<SuggestionsState> {
        // Test seam (`testing::fail_next_suggestions`).
        if self
            .inner
            .fail_next_suggestions
            .swap(false, Ordering::SeqCst)
        {
            panic!("suggested fixes failed (test)");
        }
        // Not loaded yet: the simulation that follows the load asks again.
        let vanilla = self.inner.caches.vanilla.get()?;
        if vanilla.files.is_empty() {
            return Some(SuggestionsState::Unavailable(MSG_SUGGEST_NO_GAME.into()));
        }
        let Some(raw) = self.current_raw() else {
            return Some(SuggestionsState::Unavailable(MSG_SUGGEST_NO_MODS.into()));
        };
        // The profile as `fixes_list` shows it.
        let mut p = read;
        fixes::detach_missing(&mut p);
        ctx.progress(0.1, "Checking suggested fixes");
        let (view, _) = self.plan_view(&p, key, &raw, gen);
        let bindings = view.as_deref().map_or(&[][..], |(r, _)| &r.bindings);
        self.suggestions_for(&p, gen, &raw, &vanilla, bindings)
            .map(SuggestionsState::Ready)
    }

    /// Makes the suggestions for the migrated profile `p` (§9.3) from the cached simulation of
    /// generation `gen`, each checked with a preview on the game files it touches. Stops early
    /// once the simulation is stale. None when the simulation is not there or changed
    /// meanwhile. Never writes.
    fn suggestions_for(
        &self,
        p: &ModProfile,
        gen: u32,
        raw: &RawLoad,
        vanilla: &VanillaFiles,
        bindings: &[FixBinding],
    ) -> Option<Vec<FixSuggestion>> {
        let caches = &self.inner.caches;
        let report = caches.simulation.get()?;
        let table = caches.patch_table.get()?;
        let (enabled, prios) = self.staged_order(p);
        let ctx = SuggestCtx {
            profile: p,
            raw: Some(raw),
            report: &report,
            table: &table,
            bindings,
            budget: SUGGESTION_PREVIEWS,
        };
        let found = suggestions::suggest(&ctx, &mut |q, files| {
            // A stale pass skips its remaining previews; its result is not kept anyway.
            if self.generations().sim != gen {
                return None;
            }
            let t = suggestions::table_for(q, raw, enabled.clone(), prios.clone());
            let only = VanillaFiles {
                game_version: vanilla.game_version.clone(),
                info: vanilla.info.clone(),
                files: vanilla
                    .files
                    .iter()
                    .filter(|(k, _)| files.contains(k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                assets: vanilla.assets.clone(),
                listing: vanilla.listing.clone(),
            };
            Some(self.simulate_table(&t, &only))
        });
        if self.generations().sim != gen {
            return None;
        }
        Some(found.into_iter().map(suggestion_view).collect())
    }

    /// The Fixes view of the active profile. Never writes; an unmigrated profile shows its
    /// legacy fields as read-only rows (D38).
    pub fn fixes_list(&self) -> QmResult<FixesView> {
        let mut p = self.active_profile()?;
        let key = Self::profile_key(&p);
        // Read before `raw`, so the plan view is never cached under a newer generation.
        let gen = self.generations().sim;
        let raw = self.current_raw();
        let migrated = p.fixes_version >= FIXES_VERSION;
        // A fix whose entry was taken out by hand shows as paused, as the next write saves it.
        fixes::detach_missing(&mut p);
        let rows: Vec<ProfileFix> = if migrated {
            fixes::fixes(&p).cloned().collect()
        } else {
            fixes::legacy_fixes(&p, raw.as_deref())
        };
        let (view, plan_note) = match (&raw, migrated) {
            (Some(raw), true) => self.plan_view(&p, key, raw, gen),
            _ => (None, self.staged_plan_quiet().1),
        };
        let names = self.mod_names();
        let report = self.inner.caches.simulation.get();
        let vanilla = self.vanilla_files();
        let hash = |f: &str| vanilla.as_ref().and_then(|v| self.game_hash(v, f));
        let ctx = ViewCtx {
            p: &p,
            raw: raw.as_deref(),
            bindings: view.as_deref().map_or(&[][..], |(r, _)| &r.bindings),
            order: view.as_deref().map(|(r, t)| (r, t)),
            records: report.as_deref().map(|r| &r.files_touched[..]),
            names: &names,
            legacy: !migrated,
            game: vanilla.is_some().then_some(&hash as GameHash),
        };
        // Made by the `FixSuggestions` job, never here: the rows do not wait for them. Pending
        // while a simulation is coming, as they are made from it.
        let sim_pending = self.simulation_pending();
        let (suggestions, suggestions_note, suggestions_pending) = match (&raw, migrated) {
            (Some(_), true) => match self.cached_suggestions(key).as_deref() {
                Some(SuggestionsState::Ready(list)) => (list.clone(), None, sim_pending),
                Some(SuggestionsState::Unavailable(why)) => {
                    (Vec::new(), Some(why.clone()), sim_pending)
                }
                None if report.is_some() => {
                    // Joins a running pass: repeated reads do not queue more. Queued even
                    // while a simulation runs (this read may come from its own event); the
                    // pass waits for it.
                    self.refresh_if_idle(JobKind::FixSuggestions);
                    (Vec::new(), None, true)
                }
                None => (Vec::new(), None, sim_pending),
            },
            _ => (Vec::new(), None, false),
        };
        Ok(FixesView {
            profile_id: p.id.clone(),
            fixes: rows.iter().map(|f| ctx.view(f)).collect(),
            suggestions,
            suggestions_pending,
            suggestions_note,
            stacking: p.stack_mode(),
            stacking_acknowledged: p.stacking_acknowledged(),
            plan_error: report.as_ref().and_then(|r| r.plan_error.clone()),
            load_errors: report.map(|r| r.load_errors.clone()).unwrap_or_default(),
            plan_note,
            unreadable_fixes: to_u32(p.fixes.iter().filter(|f| f.error().is_some()).count()),
            // A step a newer QuarterMaster wrote is kept as it is, not listed.
            journal: p
                .fix_journal
                .iter()
                .enumerate()
                .filter_map(|(i, e)| e.ok().map(|e| (i, e)))
                .map(|(i, e)| JournalView {
                    index: to_u32(i),
                    at: e.at.clone(),
                    op: e.op.clone(),
                    label: e.label.clone(),
                    removed: to_u32(e.removed.len()),
                    added: to_u32(e.added.len()),
                })
                .collect(),
            migrated,
            migration_error: p.migration_error.clone(),
        })
    }

    /// Imports the legacy fields into the ledger now (the Fixes view's "Retry import").
    /// Refuses with the reason when the import still cannot run (D38).
    pub fn fixes_import(&self) -> QmResult<FixesView> {
        self.ensure_not_running()?;
        // Cleared first: an import the user undid runs again only from here.
        let mut p = self.active_profile()?;
        p.migration_error = None;
        let (p, _) = self.ledger_profile_of(p)?;
        self.commit_active_profile(p)?;
        self.fixes_list()
    }

    /// The view of one fix of the active profile.
    fn fix_view(&self, id: &str) -> QmResult<FixView> {
        self.fixes_list()?
            .fixes
            .into_iter()
            .find(|f| f.id == id)
            .ok_or_else(gone)
    }

    /// Makes a fix (§10): Disable, Order, Priority, Anchor (a rung on another patch, or a
    /// Text rung that moves the patch to another line, D25) or ApplyAs (D46).
    pub fn fix_create(&self, draft: FixDraft) -> QmResult<FixView> {
        self.ensure_not_running()?;
        let (mut p, raw) = self.ledger_profile()?;
        let table = self.inner.caches.patch_table.get();
        let id = self.apply_draft(&mut p, raw.as_deref(), table.as_deref(), &draft)?;
        self.commit_active_profile(p)?;
        self.fix_view(&id)
    }

    /// Makes several fixes in one write: all of them or, on any refusal, none.
    pub fn fix_create_many(&self, drafts: Vec<FixDraft>) -> QmResult<Vec<FixView>> {
        self.ensure_not_running()?;
        if drafts.is_empty() {
            return Ok(Vec::new());
        }
        let (mut p, raw) = self.ledger_profile()?;
        let table = self.inner.caches.patch_table.get();
        let mut ids = Vec::new();
        for draft in &drafts {
            let id = self.apply_draft(&mut p, raw.as_deref(), table.as_deref(), draft)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        self.commit_active_profile(p)?;
        let view = self.fixes_list()?;
        Ok(ids
            .iter()
            .filter_map(|id| view.fixes.iter().find(|f| f.id == *id).cloned())
            .collect())
    }

    /// Applies a suggestion (§9.3) in one write: undoes each fix in `removes` alone (later
    /// fixes on the same patch stay), then makes `drafts`. All or nothing, as its preview
    /// (`remove_fixes`, `add_fixes`) checked it.
    pub fn fix_suggestion_apply(
        &self,
        drafts: Vec<FixDraft>,
        removes: Vec<String>,
    ) -> QmResult<Vec<FixView>> {
        self.ensure_not_running()?;
        let (mut p, raw) = self.ledger_profile()?;
        let table = self.inner.caches.patch_table.get();
        for id in &removes {
            fixes::undo(&mut p, raw.as_deref(), id).ok_or_else(gone)?;
        }
        let mut ids = Vec::new();
        for draft in &drafts {
            let id = self.apply_draft(&mut p, raw.as_deref(), table.as_deref(), draft)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        self.commit_active_profile(p)?;
        let view = self.fixes_list()?;
        Ok(ids
            .iter()
            .filter_map(|id| view.fixes.iter().find(|f| f.id == *id).cloned())
            .collect())
    }

    /// What undoing `fix_id` would do, with the later fixes on the same patch it takes along.
    /// Nothing is saved.
    pub fn fix_undo_preview(&self, fix_id: &str) -> QmResult<UndoPreview> {
        let raw = self.current_raw();
        let (p, _) = self
            .migrated_in_memory(self.active_profile()?, raw.as_deref())
            .map_err(|why| QmError::new(ErrorKind::InvalidInput, format!("{MSG_IMPORT}: {why}")))?;
        let fix = fixes::find(&p, fix_id).ok_or_else(gone)?;
        let also = fixes::later_on_same_patch(&p, fix_id);
        let asks = fixes::undo_asks(&p, &fix);
        let ids: Vec<String> = also
            .iter()
            .rev()
            .cloned()
            .chain([fix_id.to_string()])
            .collect();
        // The profile as migrated above, so the fix ids match on an unmigrated profile too.
        let result = self.preview_with(None, &|_, _, raw, _| {
            let mut q = p.clone();
            for id in &ids {
                fixes::undo(&mut q, Some(raw), id);
            }
            Ok((q, Vec::new()))
        })?;
        Ok(UndoPreview {
            impact: result.impact,
            also_undoes: also,
            asks,
        })
    }

    /// Undoes a fix (§7.5). When later fixes on the same patch would go too, or the user
    /// changed what the fix wrote since, `confirmed` must be true (the user saw the preview).
    pub fn fix_undo(&self, fix_id: &str, confirmed: bool) -> QmResult<()> {
        self.ensure_not_running()?;
        let (mut p, raw) = self.ledger_profile()?;
        let fix = fixes::find(&p, fix_id).ok_or_else(gone)?;
        let also = fixes::later_on_same_patch(&p, fix_id);
        let asks = fixes::undo_asks(&p, &fix);
        if !confirmed && (!also.is_empty() || !asks.is_empty()) {
            let mut why = Vec::new();
            if !also.is_empty() {
                why.push(format!(
                    "{} later fix(es) on the same patch would be undone too",
                    also.len()
                ));
            }
            why.extend(asks);
            return Err(QmError::new(
                ErrorKind::Conflict,
                format!("Confirm first: {}.", why.join("; ")),
            ));
        }
        for id in also.iter().rev().chain([&fix_id.to_string()]) {
            fixes::undo(&mut p, raw.as_deref(), id);
        }
        self.commit_active_profile(p).map(|_| ())
    }

    /// What undoing batch step `entry_index` would do (D71): the fixes it brings back, the mods
    /// it turns off, the newer running fixes they would clash with, and the game code with each
    /// choice. Nothing is saved.
    pub fn fix_journal_undo_preview(&self, entry_index: u32) -> QmResult<JournalUndoPreview> {
        let raw = self.current_raw();
        let (p, _) = self
            .migrated_in_memory(self.active_profile()?, raw.as_deref())
            .map_err(|why| QmError::new(ErrorKind::InvalidInput, format!("{MSG_IMPORT}: {why}")))?;
        let entry = journal_entry(&p, entry_index)?;
        let clashes = if entry.op == "migrated" {
            Vec::new()
        } else {
            let mut q = p.clone();
            q.fixes
                .retain(|f| f.ok().is_none_or(|f| !entry.added.contains(&f.id)));
            journal_clashes(&q, &entry)
                .into_iter()
                .map(|(old, newer)| JournalClash {
                    restored_fix_id: old.id.clone(),
                    restored_label: old.label.clone(),
                    newer_label: fixes::find(&q, &newer).map(|f| f.label).unwrap_or_default(),
                    newer_fix_id: newer,
                    slot: old.kind().to_string(),
                    patch_id: old.subject().unwrap_or_default().to_string(),
                })
                .collect()
        };
        let turns_off_mods = entry
            .mods_enabled
            .iter()
            .map(|m| {
                raw.as_deref()
                    .and_then(|r| r.mods.iter().find(|x| x.id.eq_ignore_ascii_case(m)))
                    .and_then(|x| x.name.clone())
                    .unwrap_or_else(|| m.clone())
            })
            .collect();
        // The profile as migrated above, so the fix ids match on an unmigrated profile too.
        let impact = |choice: Option<JournalUndoChoice>| -> QmResult<Option<PreviewImpact>> {
            let run = self.preview_without(&entry.mods_enabled, &|svc, _, _, _| {
                let mut q = p.clone();
                svc.undo_journal(&mut q, entry_index, choice)?;
                Ok((q, Vec::new()))
            });
            match run {
                Ok(r) => Ok(Some(r.impact)),
                // The lists still show; the dialog says the check is not ready.
                Err(e) if e.kind == ErrorKind::Conflict && e.message == MSG_NOT_READY => Ok(None),
                Err(e) => Err(e),
            }
        };
        let (impact_keep, impact_pause) = if clashes.is_empty() {
            (impact(None)?, None)
        } else {
            (
                impact(Some(JournalUndoChoice::RestorePaused))?,
                impact(Some(JournalUndoChoice::PauseNewer))?,
            )
        };
        Ok(JournalUndoPreview {
            op: entry.op.clone(),
            label: entry.label.clone(),
            restores: entry
                .removed
                .iter()
                .filter_map(|f| f.ok())
                .map(|f| f.label.clone())
                .collect(),
            turns_off_mods,
            clashes,
            impact_keep,
            impact_pause,
        })
    }

    /// Reverts one batch operation (§7.5). A migration goes back to the plan fields it
    /// imported and stays unimported until "Retry import"; it is refused while fixes made or
    /// changed since would be lost. The load order is left as it is. An export brings its
    /// fixes back and turns its mod off. When a fix it brings back would clash with a newer
    /// running fix, `choice` says how to go on (D71); without one it is refused.
    pub fn fix_journal_undo(
        &self,
        entry_index: u32,
        choice: Option<JournalUndoChoice>,
    ) -> QmResult<()> {
        self.ensure_not_running()?;
        let (mut p, _) = self.ledger_profile()?;
        self.undo_journal(&mut p, entry_index, choice)?;
        self.commit_active_profile(p).map(|_| ())
    }

    /// `fix_journal_undo` on `p`; nothing is saved.
    fn undo_journal(
        &self,
        p: &mut ModProfile,
        entry_index: u32,
        choice: Option<JournalUndoChoice>,
    ) -> QmResult<()> {
        let at = entry_index as usize;
        let entry = journal_entry(p, entry_index)?;
        if entry.op == "migrated" {
            let bak = self.paths().fixes_backup_file(&p.id);
            let raw = std::fs::read_to_string(&bak).map_err(|e| {
                QmError::new(
                    ErrorKind::NotFound,
                    "The copy from before the import is gone.",
                )
                .with_details(e.to_string())
            })?;
            let old: ModProfile = serde_json::from_str(&raw).map_err(|e| {
                QmError::new(
                    ErrorKind::Io,
                    "The copy from before the import could not be read.",
                )
                .with_details(e.to_string())
            })?;
            let later = p
                .fixes
                .iter()
                .filter(|f| f.ok().is_none_or(|f| !entry.added.contains(&f.id)))
                .count();
            if later > 0 {
                return Err(QmError::new(
                    ErrorKind::Conflict,
                    format!(
                        "{later} fix(es) were made after the import. Undo them first; undoing the import would lose them."
                    ),
                ));
            }
            if fixes::same_plan_fields(&old, p).is_err() {
                return Err(QmError::new(
                    ErrorKind::Conflict,
                    "Imported fixes were changed after the import. Undo or resume them first; undoing the import would lose the changes.",
                ));
            }
            // `user_load_after` and `mod_priorities` are the load order (D36): kept as they are.
            p.disabled_patches = old.disabled_patches;
            p.patch_priorities = old.patch_priorities;
            p.glue_patches = old.glue_patches;
            p.patch_order = old.patch_order;
            p.fixes.clear();
            p.fix_journal.clear();
            p.fixes_version = 0;
            p.migration_error = Some(fixes::IMPORT_UNDONE.to_string());
            return Ok(());
        }
        p.fixes
            .retain(|f| f.ok().is_none_or(|f| !entry.added.contains(&f.id)));
        // §7.5: never override a fix made since without the user's choice. A restored
        // priority, order rule, anchor or copy would win over (or sit next to) a running one
        // on the same patch.
        let clashes = journal_clashes(p, &entry);
        let mut newer: Vec<String> = Vec::new();
        for (_, id) in &clashes {
            if !newer.contains(id) {
                newer.push(id.clone());
            }
        }
        let mut restored = entry.removed;
        match choice {
            _ if clashes.is_empty() => {}
            None => {
                let labels: Vec<String> = newer
                    .iter()
                    .filter_map(|id| fixes::find(p, id))
                    .map(|f| format!("'{}'", f.label))
                    .collect();
                return Err(QmError::new(
                    ErrorKind::Conflict,
                    format!(
                        "{} fix(es) made since change the same patches: {}. Undo or pause them first; undoing the export would override them.",
                        newer.len(),
                        labels.join(", ")
                    ),
                ));
            }
            Some(JournalUndoChoice::PauseNewer) => {
                for id in &newer {
                    if let Some(f) = fixes::find_mut(p, id) {
                        f.paused = true;
                        f.log(
                            FixEventKind::Paused,
                            format!("to bring back fixes of {}", entry.label),
                        );
                    }
                }
            }
            Some(JournalUndoChoice::RestorePaused) => {
                for f in restored.iter_mut() {
                    let Lenient::Ok(f) = f else {
                        continue;
                    };
                    let Some((_, id)) = clashes.iter().find(|(old, _)| old.id == f.id) else {
                        continue;
                    };
                    let by = fixes::find(p, id).map(|n| n.label).unwrap_or_default();
                    f.paused = true;
                    f.log(
                        FixEventKind::Paused,
                        format!("brought back paused: '{by}' is on the same patch"),
                    );
                }
            }
        }
        p.fixes.extend(restored);
        p.enabled_mods.retain(|m| !entry.mods_enabled.contains(m));
        p.fix_journal.remove(at);
        fixes::sync_legacy(p);
        Ok(())
    }

    /// Pauses or resumes a fix; a paused fix is not compiled into the plan.
    pub fn fix_set_paused(&self, fix_id: &str, paused: bool) -> QmResult<FixView> {
        self.ensure_not_running()?;
        let (mut p, _) = self.ledger_profile()?;
        let f = fixes::find(&p, fix_id).ok_or_else(gone)?;
        // One running priority, order rule or anchor per patch, one copy per id: a fix made
        // since on the same patch would be overridden, or leave two rules for one patch.
        if !paused && f.paused {
            let labels: Vec<String> = fixes::running_clashes(&p, &f)
                .iter()
                .filter_map(|id| fixes::find(&p, id))
                .map(|f| format!("'{}'", f.label))
                .collect();
            if !labels.is_empty() {
                return Err(QmError::new(
                    ErrorKind::Conflict,
                    format!(
                        "{} is on the same patch now. Undo or pause it first.",
                        labels.join(", ")
                    ),
                ));
            }
        }
        let f = fixes::find_mut(&mut p, fix_id).ok_or_else(gone)?;
        // A load order change is the load order itself (D36); pausing would not undo it.
        if paused && matches!(f.change, FixChange::ModOrder { .. }) {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "A load order change cannot be paused. Undo it, or change it in Load order.",
            ));
        }
        if f.paused != paused {
            f.paused = paused;
            let what = if paused {
                FixEventKind::Paused
            } else {
                FixEventKind::Resumed
            };
            f.log(what, "");
            fixes::sync_legacy(&mut p);
            self.commit_active_profile(p)?;
        }
        self.fix_view(fix_id)
    }

    /// Accepts a changed or moved patch: the fix is pinned to it as it is now.
    pub fn fix_accept(&self, fix_id: &str) -> QmResult<FixView> {
        self.ensure_not_running()?;
        let (mut p, raw) = self.ledger_profile()?;
        let raw = raw.ok_or_else(|| {
            QmError::new(ErrorKind::Conflict, "The profile's mods could not be read.")
        })?;
        if !fixes::accept(&mut p, &raw, fix_id) {
            return Err(gone());
        }
        self.commit_active_profile(p)?;
        self.fix_view(fix_id)
    }

    /// Points a fix at `patch_id` (the user picked one of several candidates).
    pub fn fix_rebind(&self, fix_id: &str, patch_id: &str) -> QmResult<FixView> {
        self.ensure_not_running()?;
        let (mut p, raw) = self.ledger_profile()?;
        let raw = raw.ok_or_else(|| {
            QmError::new(ErrorKind::Conflict, "The profile's mods could not be read.")
        })?;
        if fixes::find(&p, fix_id).is_none() {
            return Err(gone());
        }
        fixes::rebind(&mut p, &raw, fix_id, patch_id)
            .map_err(|e| QmError::new(ErrorKind::InvalidInput, e))?;
        self.commit_active_profile(p)?;
        self.fix_view(fix_id)
    }
}

/// Refuses an ApplyAs draft for a patch with no separate edits: only an `at` pattern patch
/// has them (D46), and a before/after pattern patch on several lines uses edits mode to find
/// them around another patch's code (D48).
pub(crate) fn check_apply_as_draft(table: Option<&PatchTable>, patch_id: &str) -> QmResult<()> {
    let Some(table) = table else {
        return Ok(());
    };
    let found = table
        .patches
        .iter()
        .position(|(p, _, _)| p.id() == Some(patch_id))
        .or_else(|| matching_patches(patch_id, &table.patches).first().copied());
    let Some((patch, _, _)) = found.and_then(|i| table.patches.get(i)) else {
        return Ok(());
    };
    match patch {
        Patch::Pattern(a) if a.position == InsertPosition::At => Ok(()),
        // A before/after patch on several lines finds them again around another patch's code
        // (D48).
        Patch::Pattern(a) if a.pattern.lines().count() > 1 => Ok(()),
        _ => Err(QmError::new(
            ErrorKind::InvalidInput,
            format!(
                "{} does not replace lines, so it has no separate edits.",
                patch.name().unwrap_or(patch_id)
            ),
        )),
    }
}

/// `s` with its first letter upper case.
fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The text of a caught panic.
fn panic_text(p: &(dyn std::any::Any + Send)) -> &str {
    p.downcast_ref::<&str>()
        .copied()
        .or_else(|| p.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("panic")
}

/// The most previews one Fixes view runs for its suggestions.
const SUGGESTION_PREVIEWS: usize = 8;

/// A suggestion as the Fixes view shows it: its Anchor fixes as drafts for `fix_create`.
fn suggestion_view(s: suggestions::Suggestion) -> FixSuggestion {
    let drafts = s
        .adds
        .iter()
        .map(|a| FixDraft {
            change: FixChangeDraft::Anchor {
                patch_id: a.patch_id.clone(),
                anchors: a.anchors.clone(),
                sites: a.sites.clone(),
            },
            label: a.label.clone(),
            source: FixSource::Suggestion,
            conflict_id: s.conflict_id.clone(),
            option_key: Some(s.key.clone()),
            approved: None,
        })
        .collect();
    FixSuggestion {
        key: s.key,
        kind: s.kind.as_str().to_string(),
        label: s.label,
        drafts,
        removes: s.removes,
        safe: s.safe,
        fix_id: s.fix_id,
    }
}

/// What a fix view is built from.
struct ViewCtx<'a> {
    p: &'a ModProfile,
    raw: Option<&'a RawLoad>,
    bindings: &'a [FixBinding],
    /// The order results of the profile's plan, with the table they come from (§3.9).
    order: Option<(&'a PlanReport, &'a PatchTable)>,
    /// The current simulation's files, for how anchor fixes placed their code.
    records: Option<&'a [qm_manager::SimulatedFileResult]>,
    names: &'a HashMap<String, String>,
    legacy: bool,
    /// The hash of a game file now, when the game files are loaded (D74).
    game: Option<GameHash<'a>>,
}

impl ViewCtx<'_> {
    /// The stamped game files of `f` that changed since (D74).
    fn game_drift(&self, f: &ProfileFix) -> Vec<GameFileDrift> {
        match self.game {
            Some(hash) if !self.legacy => fixes::game_stamp::game_drift(f, hash),
            _ => Vec::new(),
        }
    }

    /// The fix's status, and Needs review when the game changed since it was stamped (D74).
    /// A paused, waiting or suspended fix keeps its status: nothing is suspended for it.
    fn status(&self, f: &ProfileFix) -> FixStatus {
        let s = self.base_status(f);
        if matches!(
            s,
            FixStatus::Paused | FixStatus::Waiting { .. } | FixStatus::Suspended { .. }
        ) || self.game_drift(f).is_empty()
        {
            return s;
        }
        let mut reasons = match s {
            FixStatus::NeedsReview { reasons } => reasons,
            FixStatus::FellBack { reason, .. } => vec![reason],
            _ => Vec::new(),
        };
        reasons.push(fixes::GAME_CHANGED.to_string());
        FixStatus::NeedsReview { reasons }
    }

    fn one_hit(&self, name: &str, rule_name: bool) -> Option<usize> {
        let raw = self.raw?;
        match fixes::hits(raw, name, rule_name).as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    }

    fn label(&self, id: &str, mod_hint: &str, rule_name: bool) -> PatchLabel {
        let hit = self
            .one_hit(id, rule_name)
            .and_then(|i| self.raw.and_then(|r| r.patches.get(i)));
        let mod_id = hit
            .and_then(|rp| rp.patch.mod_id().map(str::to_string))
            .unwrap_or_else(|| mod_hint.to_string());
        PatchLabel {
            patch_id: id.to_string(),
            name: hit.and_then(|rp| rp.patch.name().map(str::to_string)),
            mod_name: self.names.get(&mod_id.to_lowercase()).cloned(),
            mod_id,
        }
    }

    fn base_status(&self, f: &ProfileFix) -> FixStatus {
        if f.paused {
            return FixStatus::Paused;
        }
        if let FixChange::ModOrder { .. } = f.change {
            return if fixes::mod_order_intact(self.p, f) {
                FixStatus::Active
            } else {
                FixStatus::NeedsReview {
                    reasons: vec!["Changed in Load order".to_string()],
                }
            };
        }
        if let Some(s) = self.order_status(f) {
            return s;
        }
        let mine: Vec<&FixBinding> = self.bindings.iter().filter(|b| b.fix_id == f.id).collect();
        if mine.is_empty() {
            return self.presence(f);
        }
        let mut waiting = Vec::new();
        let mut review = Vec::new();
        let mut rebound = None;
        for b in mine {
            match &b.status {
                BindingStatus::Suspended { reason } => {
                    return FixStatus::Suspended {
                        reason: reason.clone(),
                    }
                }
                BindingStatus::Waiting { mods } => waiting.extend(mods.iter().cloned()),
                BindingStatus::Drifted { .. }
                | BindingStatus::Ambiguous { .. }
                | BindingStatus::Missing => review.push(b.detail.clone()),
                BindingStatus::Rebound { from, to } => {
                    rebound = Some(FixStatus::Rebound {
                        from: from.clone(),
                        to: to.clone(),
                    })
                }
                BindingStatus::Bound => {}
            }
        }
        if !waiting.is_empty() {
            waiting.dedup();
            return FixStatus::Waiting { mods: waiting };
        }
        if !review.is_empty() {
            return FixStatus::NeedsReview { reasons: review };
        }
        rebound
            .or_else(|| self.anchoring_status(f))
            .or_else(|| self.edits_status(f))
            .or_else(|| self.stack_status(f))
            .unwrap_or(FixStatus::Active)
    }

    /// A StackRule fix whose two blocks never meet at its line in the current simulation does
    /// nothing yet: it waits for the other patch there (D55). One a stronger rule beat is
    /// suspended (D56), and so is one kept in place because the move would cross the patch's
    /// next place.
    fn stack_status(&self, f: &ProfileFix) -> Option<FixStatus> {
        let FixChange::StackRule {
            patch,
            side,
            below,
            above,
            ..
        } = &f.change
        else {
            return None;
        };
        let records = self.records?;
        let moves = || {
            records
                .iter()
                .flat_map(|file| &file.records)
                .flat_map(|r| r.stack_moves.iter().map(move |m| (r, m)))
        };
        let held = moves()
            .any(|(_, m)| matches!(&m.reason, StackReason::UserRule { fix_id } if *fix_id == f.id));
        if held {
            return None;
        }
        let other = below.as_ref().or(above.as_ref())?;
        let mine = format!("Your fix {} to put", f.id);
        if let Some(o) = moves()
            .flat_map(|(_, m)| &m.overridden)
            .find(|o| o.starts_with(&mine))
        {
            return Some(FixStatus::Suspended { reason: o.clone() });
        }
        let clamped = moves().any(|(r, m)| {
            m.reason == StackReason::GuardClamped
                && m.side == *side
                && (r.patch_id == patch.id || r.patch_id == other.id)
        });
        if clamped {
            return Some(FixStatus::Suspended {
                reason:
                    "The block stays in place here: moving it would cross the patch's next place"
                        .to_string(),
            });
        }
        let label = self.label(&other.id, &other.mod_id, false);
        let name = match (label.name, label.mod_name) {
            (Some(n), Some(m)) => format!("{m} › {n}"),
            (Some(n), None) => n,
            _ => other.id.clone(),
        };
        Some(FixStatus::Waiting {
            mods: vec![format!("{name} at this line")],
        })
    }

    /// What the order results say about a fix (§3.9): an Order fix whose rule a rung
    /// overrides needs review; an Anchor fix with a suspended rung is suspended.
    fn order_status(&self, f: &ProfileFix) -> Option<FixStatus> {
        let (report, table) = self.order?;
        match &f.change {
            FixChange::Order { rule, .. } => {
                let names = |n: &str| -> Vec<String> {
                    matching_patches(n, &table.patches)
                        .into_iter()
                        .filter_map(|i| table.patches.get(i).map(|(p, _, _)| patch_name(p, i)))
                        .collect()
                };
                let subjects = names(&rule.patch);
                let mut pairs = Vec::new();
                for b in rule.before.iter().flat_map(|n| names(n)) {
                    pairs.extend(subjects.iter().map(|a| (a.clone(), b.clone())));
                }
                for b in rule.after.iter().flat_map(|n| names(n)) {
                    pairs.extend(subjects.iter().map(|a| (b.clone(), a.clone())));
                }
                let reasons: Vec<String> = pairs
                    .iter()
                    .filter_map(|(a, b)| {
                        let head = format!("profile rule {a} before {b} ignored: ");
                        report
                            .overridden_rules
                            .iter()
                            .find_map(|r| r.strip_prefix(&head))
                            .map(|why| format!("ignored: {why}"))
                    })
                    .collect();
                (!reasons.is_empty()).then_some(FixStatus::NeedsReview { reasons })
            }
            FixChange::Anchor { .. } => {
                let source = format!("profile:{}", f.id);
                report
                    .suspended_rungs
                    .iter()
                    .find(|r| r.source == source)
                    .map(|r| FixStatus::Suspended {
                        reason: r.reason.clone(),
                    })
            }
            _ => None,
        }
    }

    /// How an Anchor fix placed its code in the current simulation: Fell back when a site
    /// used the patch's own line, Followed when a rung found its text in a changed patch.
    fn anchoring_status(&self, f: &ProfileFix) -> Option<FixStatus> {
        if !matches!(f.change, FixChange::Anchor { .. }) {
            return None;
        }
        let mut fell = 0u32;
        let mut reason = None;
        let mut followed = false;
        let records = self.records?.iter().flat_map(|file| &file.records);
        for rec in records.filter(|r| r.fix_ids.contains(&f.id)) {
            for site in rec.anchoring.iter().flat_map(|a| &a.sites) {
                if let Some(why) = &site.fallback {
                    fell += 1;
                    reason.get_or_insert_with(|| why.clone());
                }
                followed |= site.followed;
            }
        }
        if let Some(reason) = reason {
            return Some(FixStatus::FellBack {
                sites: fell,
                reason,
            });
        }
        followed.then_some(FixStatus::Followed)
    }

    /// How an ApplyAs or SkipHunk fix ran in the current simulation: Fell back when the
    /// patch, or one of its places, still ran as one replacement, with why (D47, D49).
    fn edits_status(&self, f: &ProfileFix) -> Option<FixStatus> {
        if !matches!(
            f.change,
            FixChange::ApplyAs {
                mode: ApplyAs::Edits,
                ..
            } | FixChange::SkipHunk { .. }
        ) {
            return None;
        }
        let head = format!("{}:", edits::EDITS_REFUSED);
        let mut fell = 0u32;
        let mut reason = None;
        let records = self.records?.iter().flat_map(|file| &file.records);
        for rec in records.filter(|r| r.fix_ids.contains(&f.id)) {
            for w in rec.warnings.iter().filter(|w| w.starts_with(&head)) {
                fell += 1;
                // "edits_refused:<code>: <text>"
                let text = w
                    .get(head.len()..)
                    .and_then(|rest| rest.split_once(": "))
                    .map_or(w.as_str(), |(_, t)| t);
                reason.get_or_insert_with(|| upper_first(text));
            }
        }
        reason.map(|reason| FixStatus::FellBack {
            sites: fell,
            reason,
        })
    }

    /// The status of a fix the plan has no binding for: by whether its patch is loaded.
    fn presence(&self, f: &ProfileFix) -> FixStatus {
        let (Some(raw), Some(subject)) = (self.raw, f.subject()) else {
            return FixStatus::Active;
        };
        let rule_name = matches!(f.change, FixChange::Order { .. });
        if !fixes::hits(raw, subject, rule_name).is_empty() {
            return FixStatus::Active;
        }
        let m = f
            .refs()
            .first()
            .map(|r| r.mod_id.clone())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| subject.split(':').next().unwrap_or_default().to_string());
        let loaded = raw.mods.iter().any(|x| x.id.eq_ignore_ascii_case(&m));
        if m.is_empty() || loaded {
            let copy = matches!(f.change, FixChange::AdjustedCopy { .. });
            let tail = if copy { "; your copy still runs" } else { "" };
            FixStatus::NeedsReview {
                reasons: vec![format!("{subject} is no longer in {m}{tail}")],
            }
        } else {
            FixStatus::Waiting { mods: vec![m] }
        }
    }

    /// Why a mod a fix waits for did not load, when it is enabled (D60): it failed to load, a
    /// mod it needs did (D15), or it is missing. Empty when it is off or loaded.
    fn absence(&self, m: &str) -> &'static str {
        let Some(raw) = self.raw else {
            return "";
        };
        let is = |x: &str| x.eq_ignore_ascii_case(m);
        if raw.mods.iter().any(|x| is(&x.id) || is(&x.folder)) {
            ""
        } else if raw
            .load_errors
            .iter()
            .any(|e| is(&e.mod_id) || is(&e.folder))
        {
            " (failed to load)"
        } else if raw
            .load_errors
            .iter()
            .any(|e| e.skipped_dependents.iter().any(|d| is(d)))
        {
            " (a mod it needs failed to load)"
        } else if self.p.enabled_mods.iter().any(|x| is(x)) {
            " (missing)"
        } else {
            ""
        }
    }

    fn status_text(&self, s: &FixStatus) -> String {
        let name = |m: &String| {
            self.names
                .get(&m.to_lowercase())
                .cloned()
                .unwrap_or_else(|| m.clone())
        };
        match s {
            FixStatus::Active => "Working".to_string(),
            FixStatus::Rebound { to, .. } => format!("Follows {to} now"),
            FixStatus::Followed => "Follows the changed code".to_string(),
            FixStatus::FellBack { reason, .. } => reason.clone(),
            FixStatus::Waiting { mods } => format!(
                "Waiting for {}",
                mods.iter()
                    .map(|m| format!("{}{}", name(m), self.absence(m)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            FixStatus::NeedsReview { reasons } => reasons.join("; "),
            FixStatus::Suspended { reason } => reason.clone(),
            FixStatus::Paused => "Paused".to_string(),
        }
    }

    /// Stored snapshots against the patches as they are now (D41).
    fn drift(&self, f: &ProfileFix) -> Option<Vec<DriftDiff>> {
        let raw = self.raw?;
        let rule_name = matches!(f.change, FixChange::Order { .. });
        let moved: Vec<&String> = self
            .bindings
            .iter()
            .filter(|b| b.fix_id == f.id)
            .filter_map(|b| b.new_id.as_ref())
            .collect();
        let same = |a: &PatchSnapshot, b: &PatchSnapshot| {
            (&a.kind, &a.pattern, &a.payload, a.position, a.times)
                == (&b.kind, &b.pattern, &b.payload, b.position, b.times)
        };
        let mut out = Vec::new();
        for (id, before) in &f.snapshots {
            let now = self
                .one_hit(id, rule_name)
                .or_else(|| moved.iter().find_map(|m| self.one_hit(m, false)))
                .and_then(|i| fixes::snapshot_at(raw, i));
            let Some(after) = now else {
                continue;
            };
            if !same(before, &after) {
                out.push(DriftDiff {
                    patch_id: id.clone(),
                    role: if f.subject() == Some(id.as_str()) {
                        "fix"
                    } else {
                        "anchor_target"
                    }
                    .to_string(),
                    before: before.clone(),
                    after,
                });
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn view(&self, f: &ProfileFix) -> FixView {
        let status = self.status(f);
        let rule_name = matches!(f.change, FixChange::Order { .. });
        let hint = f
            .refs()
            .first()
            .map(|r| r.mod_id.clone())
            .unwrap_or_default();
        let patch = f.subject().map(|s| self.label(s, &hint, rule_name));
        let related = match &f.change {
            FixChange::Order { rule, .. } => rule
                .before
                .iter()
                .chain(&rule.after)
                .map(|n| self.label(n, "", true))
                .collect(),
            FixChange::StackRule { below, above, .. } => below
                .iter()
                .chain(above)
                .map(|r| self.label(&r.id, &r.mod_id, false))
                .collect(),
            _ => Vec::new(),
        };
        let target_file = f
            .subject()
            .and_then(|s| self.one_hit(s, rule_name))
            .and_then(|i| self.raw.and_then(|r| r.patches.get(i)))
            .map(|rp| rp.patch.primary_target().to_string())
            .or_else(|| match &f.change {
                FixChange::AdjustedCopy { glue, .. } => {
                    Some(glue.patch.primary_target().to_string())
                }
                _ => None,
            });
        let (can_export, export_blocked_reason) = if self.legacy {
            (false, Some("Import your earlier fixes first.".to_string()))
        } else {
            match (&f.change, &status) {
                (FixChange::AdjustedCopy { .. }, _) => (
                    false,
                    Some("An adjusted copy cannot be saved as a compatibility patch.".to_string()),
                ),
                (FixChange::ModOrder { .. }, _) => (
                    false,
                    Some(
                        "A load order change cannot be saved as a compatibility patch.".to_string(),
                    ),
                ),
                (FixChange::AssetChoice { .. }, _) => (
                    false,
                    Some("An asset choice cannot be saved as a compatibility patch.".to_string()),
                ),
                (_, FixStatus::Waiting { .. }) => (
                    false,
                    Some("Turn on the mods this fix changes first.".to_string()),
                ),
                _ if f.paused => (false, Some("It is paused. Resume it first.".to_string())),
                (_, FixStatus::Suspended { .. } | FixStatus::NeedsReview { .. }) => {
                    (false, Some("Review this fix first.".to_string()))
                }
                _ => (true, None),
            }
        };
        FixView {
            id: f.id.clone(),
            label: f.label.clone(),
            kind: f.kind().to_string(),
            status_text: self.status_text(&status),
            status,
            target_file,
            patch,
            related,
            source: f.provenance.source,
            provenance: f.provenance.clone(),
            paused: f.paused,
            fallback_text: None,
            drift: self.drift(f),
            game_drift: self.game_drift(f),
            can_export,
            export_blocked_reason,
            legacy: self.legacy,
            candidates: self.candidates(f, &hint),
            glue_id: match &f.change {
                FixChange::AdjustedCopy { glue, .. } => Some(glue.id.clone()),
                _ => None,
            },
            run_like_mod: self.run_like_mod(f),
            can_run_last: self.can_run_last(f),
        }
    }

    /// Whether a running copy "Run it like <mod>" switched can go back to running after every
    /// mod patch (D58). Not for a paused or legacy row, or a copy a newer copy stands in for.
    fn can_run_last(&self, f: &ProfileFix) -> bool {
        let FixChange::AdjustedCopy { glue, .. } = &f.change else {
            return false;
        };
        !self.legacy
            && !f.paused
            && fixes::copy_switched_to_base(f)
            && !fixes::copy_superseded(self.p, &glue.id)
    }

    /// The mod a running legacy copy can run as instead (D58): none for a copy that runs as
    /// its base mod, a paused or legacy row, a copy a newer copy stands in for, or one whose
    /// base patch is not loaded.
    fn run_like_mod(&self, f: &ProfileFix) -> Option<String> {
        let FixChange::AdjustedCopy { glue, .. } = &f.change else {
            return None;
        };
        if self.legacy || f.paused || glue.runs_as_base() {
            return None;
        }
        if fixes::copy_superseded(self.p, &glue.id) {
            return None;
        }
        fixes::copy_base_mod(self.p, self.raw?, glue)
    }

    /// The patches an Ambiguous binding of `f` could mean, for the user to pick (§9.3).
    fn candidates(&self, f: &ProfileFix, hint: &str) -> Vec<PatchLabel> {
        let mut out: Vec<PatchLabel> = Vec::new();
        for b in self.bindings.iter().filter(|b| b.fix_id == f.id) {
            if let BindingStatus::Ambiguous { candidates } = &b.status {
                for c in candidates {
                    if !out.iter().any(|l| &l.patch_id == c) {
                        out.push(self.label(c, hint, false));
                    }
                }
            }
        }
        out
    }
}
