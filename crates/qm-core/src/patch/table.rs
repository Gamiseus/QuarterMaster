#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use anyhow::{Context, Result};
use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::dump::{ByteDebugEntry, PatchDebug};
use crate::handshake::{
    LoadError, LoadOrderPlan, PatchExecutionRecord, PatchOutcomeStatus, StackMode, SuspendedRung,
};
use crate::patch::anchor::{self, Ladder, OriginalLines};
use crate::patch::linemap::{CommitInfo, Edit, LineMap, Removed, NO_INDEX};
use crate::patch::origin::{self, FileTrace, JournalEdit};
use crate::patch::vars;
use crate::patch::copy::CopyPatch;
use crate::patch::edits::{Keeper, Normalized, SkipAt};
use crate::patch::order::{
    self, author_after, author_before, order_patches_full, patch_name, CopyPred, OrderResult,
    OrderRule, Strength,
};
use crate::patch::plan::{
    self, Applied, ModGraph, ModMeta, PatchExtra, PlanReport, RawLoad, GLUE_MOD,
};
use crate::patch::stack::{RuleSource, StackHint, StackRuleBound};
use crate::patch::{ApplyCtx, OnFailurePolicy, Patch, Priority, StackCtx, Target};
use crate::prepare::{canonical_path, PreparedBuffer, MATCHED_QM_STARTUP_CODE};
use crate::sys::{preload_module, LuaFunc, LuaState, LuaTable};
use crop::Rope;
use log::*;

/// Warning on records of patches skipped because an earlier patch in their mod failed under
/// `on_failure = "skip_mod"`. Consumers match it exactly to tell these from genuine failures.
pub const SKIPPED_WITH_MOD_WARNING: &str =
    "Skipped because an earlier patch in this mod failed with skip_mod";

/// Warning code on a copy an explicit order rule runs after a pattern or regex of the same
/// file, instead of with the copies that run first (D54).
pub const COPY_PASS_ORDERED: &str = "copy_pass_ordered";

/// Warning code on a patch whose `requires` names a patch that did not apply in its file, is
/// turned off in another file, or matches nothing (D64): `needs_patch:<name>: ...`.
pub const NEEDS_PATCH: &str = "needs_patch";

/// How the warning of a patch skipped for its `requires` ends (D64).
const DID_NOT_APPLY: &str = ", which did not apply";

/// Whether `r` was skipped because a patch its `requires` names did not apply first in the
/// same file (D64). Such a patch did not fail; it waits for the patch it needs.
pub fn skipped_for_requirement(r: &PatchExecutionRecord) -> bool {
    let code = format!("{NEEDS_PATCH}:");
    r.status == PatchOutcomeStatus::SkippedByPolicy
        && r
            .warnings
            .iter()
            .any(|w| w.starts_with(&code) && w.ends_with(DID_NOT_APPLY))
}

/// One name in a patch's `requires`, resolved at prepare time (D64).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requirement {
    /// The name as the author wrote it, trimmed.
    pub name: String,
    /// The patches it names, the requiring patch left out. Empty: it names no loaded patch.
    pub hits: Vec<u32>,
}

/// The names a patch's target gives (none for a module, whose `before` is not a target).
fn target_names(p: &Patch) -> Vec<&str> {
    let target = match p {
        Patch::Pattern(x) => &x.target,
        Patch::Regex(x) => &x.target,
        Patch::Copy(x) => &x.target,
        Patch::Module(_) => return Vec::new(),
    };
    match target {
        Target::Single(t) => vec![t.as_str()],
        Target::Multi(ts) => ts.iter().map(String::as_str).collect(),
    }
}

/// Whether `a` and `b` can patch one file: one's target names a file the other applies to.
fn share_a_file(a: &Patch, b: &Patch) -> bool {
    target_names(a).iter().any(|t| b.can_apply_to_target(t))
        || target_names(b).iter().any(|t| a.can_apply_to_target(t))
}

/// Whether `p` is an adjusted copy of the patch `name` names (section 4.5, the glue alias).
fn copy_of(p: &Patch, name: &str) -> bool {
    let Some(base) = p.extra().replaces.as_deref() else {
        return false;
    };
    base == name
        || p.mod_id().is_some_and(|m| format!("{m}:{base}") == name)
        || (p.mod_id() == Some(GLUE_MOD) && name.ends_with(&format!(":{base}")))
}

#[cfg(test)]
thread_local! {
    /// The stale-state warnings `fitting` logged on this thread, for tests.
    static STALE_WARNINGS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Structure to manage patch tables for QuarterMaster runtime and offline simulation
#[derive(Clone, Debug)]
pub struct PatchTable {
    pub mod_dir: PathBuf,
    pub targets: HashSet<String>,
    // Unsorted
    pub patches: Vec<(Patch, Priority, PathBuf)>,
    pub vars: HashMap<String, String>,
    /// `[vars]` names two mods set to different values (G12); `vars` keeps the last.
    pub var_collisions: Vec<crate::patch::loader::VarCollision>,
    /// The profile's patch order rules (`load-order.json` `patch_order`).
    pub order_rules: Vec<OrderRule>,
    /// Mods skipped because they failed to load, or require one that did (D15).
    pub load_errors: Vec<LoadError>,
    /// What reading load-order.json found: a hard error (no mods, D16) or skipped entries.
    pub plan_report: PlanReport,
    /// The loaded mods' mod.json data, in loader order.
    pub mods: Vec<ModMeta>,
    /// From the plan's `stacking`; Legacy when absent (D2).
    pub stack_mode: StackMode,
    /// From the plan's `line_rules_v2` (D50, D51); false when absent.
    pub line_rules_v2: bool,
    /// Order rules from compatibility mods, with the compat mod id.
    pub compat_rules: Vec<(OrderRule, String)>,
}

/// Derived state of a table: built once by `PatchTable::prepare`, then only read, so the DLL
/// can share it behind a read lock. Never stored in `PatchTable`, so a change to the table
/// cannot leave it stale: `apply_prepared` checks the stamp and rebuilds on a mismatch.
#[derive(Clone, Debug)]
pub struct Prepared {
    stamp: u64,
    order: OrderResult,
    graph: ModGraph,
    /// Per patch, the index of its mod in `PatchTable::mods` (`NO_INDEX` when not loaded).
    mod_ix: Vec<u32>,
    /// Per patch, its anchor ladder (suspended rungs already set).
    ladders: Vec<Option<Ladder>>,
    /// Per copy patch, the patterns and regexes explicit order edges put before it (D54).
    copy_preds: Vec<Vec<CopyPred>>,
    /// Per `at` pattern patch, its normalized form (D47); `None` for other patches.
    hunks: Vec<Option<Normalized>>,
    /// Per patch, its skipped edits with the kept patch's index (D49).
    skips: Vec<Vec<SkipAt>>,
    /// Per patch, the stack rules that name it, with the other patch's index (D55).
    stack_rules: Vec<Vec<StackRuleBound>>,
    /// (patch index, author hint) of every patch with a `stack` key (D57).
    hints: Vec<(u32, StackHint)>,
    /// Per patch, its `requires` names resolved to patches (D64).
    requires: Vec<Vec<Requirement>>,
    /// The table's plan report plus the order results (§3.9).
    report: PlanReport,
}

impl Prepared {
    /// The order patches are applied in.
    pub fn order(&self) -> &OrderResult {
        &self.order
    }

    /// Which loaded mods depend on which (D4).
    pub fn graph(&self) -> &ModGraph {
        &self.graph
    }

    /// The `PatchTable::stamp` of the table this was built from.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// The index in `PatchTable::mods` of patch `patch_ix`'s mod, or `NO_INDEX`.
    pub fn mod_ix(&self, patch_ix: usize) -> u32 {
        self.mod_ix.get(patch_ix).copied().unwrap_or(NO_INDEX)
    }

    /// Patch `patch_ix`'s anchor ladder, when it has rungs.
    pub fn ladder(&self, patch_ix: usize) -> Option<&Ladder> {
        self.ladders.get(patch_ix).and_then(Option::as_ref)
    }

    /// For a copy patch, the patterns and regexes that explicit order edges put before it
    /// (D54). Each buffer runs the copy after the ones that patch it; empty for other patches.
    pub fn copy_preds(&self, patch_ix: usize) -> &[CopyPred] {
        self.copy_preds.get(patch_ix).map_or(&[], Vec::as_slice)
    }

    /// Patch `patch_ix`'s normalized form when it is an `at` pattern patch (D47). Edits mode
    /// applies it; the simulation reports it for every `at` patch (D46).
    pub fn hunks(&self, patch_ix: usize) -> Option<&Normalized> {
        self.hunks.get(patch_ix).and_then(Option::as_ref)
    }

    /// Patch `patch_ix`'s skipped edits (D49).
    pub(crate) fn skips(&self, patch_ix: usize) -> &[SkipAt] {
        self.skips.get(patch_ix).map_or(&[], Vec::as_slice)
    }

    /// The stack rules that name patch `patch_ix`, from either side (D55).
    pub fn stack_rules(&self, patch_ix: usize) -> &[StackRuleBound] {
        self.stack_rules.get(patch_ix).map_or(&[], Vec::as_slice)
    }

    /// (patch index, hint) of every patch with an author `stack` key (D57).
    pub fn hints(&self) -> &[(u32, StackHint)] {
        &self.hints
    }

    /// Patch `patch_ix`'s `requires` names, resolved (D64).
    pub fn requires(&self, patch_ix: usize) -> &[Requirement] {
        self.requires.get(patch_ix).map_or(&[], Vec::as_slice)
    }

    /// The plan report with the order results: overridden rules, unknown ids, suspended and
    /// invalid rungs. The DLL and the simulation both report these.
    pub fn report(&self) -> &PlanReport {
        &self.report
    }
}

/// `apply_prepared_traced`'s result.
#[derive(Debug)]
pub struct TracedApply {
    pub text: String,
    pub debug: PatchDebug,
    pub records: Vec<PatchExecutionRecord>,
    pub trace: FileTrace,
    /// The final line map (game numbering, before variable interpolation).
    pub lines: LineMap,
}

/// `apply_prepared_buffer`'s result: what the DLL keeps of an apply.
#[derive(Debug)]
pub struct BufferApply {
    pub text: String,
    pub debug: PatchDebug,
    pub records: Vec<PatchExecutionRecord>,
    /// Per line of `text`, the index in `records` of the record that wrote it, or
    /// `linemap::NO_RECORD` (crash attribution).
    pub line_records: Vec<u32>,
}

/// The text one apply starts from, with QuarterMaster's own lines (0-based).
#[derive(Clone, Copy)]
struct BufferIn<'a> {
    text: &'a str,
    builtin: &'a [Range<usize>],
    builtin_edited: &'a [usize],
}

impl<'a> BufferIn<'a> {
    fn plain(text: &'a str) -> Self {
        BufferIn {
            text,
            builtin: &[],
            builtin_edited: &[],
        }
    }

    fn prepared(buf: &'a PreparedBuffer) -> Self {
        BufferIn {
            text: &buf.text,
            builtin: &buf.builtin,
            builtin_edited: &buf.builtin_edited,
        }
    }
}

/// Everything one apply produced.
struct InnerApply {
    text: String,
    /// The text before variable interpolation, when a journal was kept.
    raw_text: Option<String>,
    debug: PatchDebug,
    records: Vec<PatchExecutionRecord>,
    lines: LineMap,
    /// Per journal edit, the lines it removed (only with a journal).
    removed: Vec<Vec<Removed>>,
}

impl Default for PatchTable {
    fn default() -> Self {
        Self {
            mod_dir: PathBuf::new(),
            targets: HashSet::new(),
            patches: Vec::new(),
            vars: HashMap::new(),
            var_collisions: Vec::new(),
            order_rules: Vec::new(),
            load_errors: Vec::new(),
            plan_report: PlanReport::default(),
            mods: Vec::new(),
            stack_mode: StackMode::default(),
            line_rules_v2: false,
            compat_rules: Vec::new(),
        }
    }
}

impl PatchTable {
    /// Load patches from the provided mod directory. The plan (`qm/load-order.json`) is read
    /// once here; `plan::load_raw` reads the mods and `plan::apply_plan` lays the plan over
    /// them. A mod that fails to load is skipped together with the enabled mods that require
    /// it, and listed in `load_errors`. A plan that cannot be used loads no mods and sets
    /// `plan_report.plan_error` (D16); a bad plan entry is skipped and listed in
    /// `plan_report.plan_warnings` (D17). `Err` only when the Mods folder itself cannot be read.
    pub fn load(mod_dir: &Path) -> Result<PatchTable> {
        Self::load_with_raw(mod_dir).map(|(table, _)| table)
    }

    /// `load`, plus what was read from disk, for previews that lay another plan over the same
    /// mods. No `RawLoad` on a plan error: the mods were not read.
    pub fn load_with_raw(mod_dir: &Path) -> Result<(PatchTable, Option<RawLoad>)> {
        Self::load_with_raw_fp(mod_dir, plan::FpMode::Lazy)
    }

    /// `load_with_raw` with a choice of how many fingerprints binding computes. The launcher's
    /// simulation uses `FpMode::All` so drift is always reported; the patches are the same.
    pub fn load_with_raw_fp(
        mod_dir: &Path,
        mode: plan::FpMode,
    ) -> Result<(PatchTable, Option<RawLoad>)> {
        Self::load_full(mod_dir, mode).map(|(table, raw, _)| (table, raw))
    }

    /// `load`, with the `Prepared` built for the order dump, so a caller that keeps one (the
    /// DLL) builds it once.
    pub fn load_prepared(mod_dir: &Path) -> Result<(PatchTable, Prepared)> {
        Self::load_full(mod_dir, plan::FpMode::Lazy).map(|(table, _, prep)| (table, prep))
    }

    fn load_full(
        mod_dir: &Path,
        mode: plan::FpMode,
    ) -> Result<(PatchTable, Option<RawLoad>, Prepared)> {
        let new_dir = dunce::canonicalize(mod_dir)
            .with_context(|| format!("Could not resolve mod dir {:?}", mod_dir))?;
        let read = LoadOrderPlan::read(&new_dir);
        for w in &read.warnings {
            warn!("{w}");
        }
        if let Some(e) = &read.error {
            error!("{e}. No mods are loaded.");
            let applied = plan::apply_plan(&RawLoad::default(), &read);
            let table = PatchTable::from_applied(applied, new_dir);
            let prep = table.prepare();
            return Ok((table, None, prep));
        }
        let raw = plan::load_raw(&new_dir, read.plan.as_ref())?;
        let applied = plan::apply_plan_with(&raw, &read, mode);
        // load_raw already logged its own warnings.
        for w in applied.report.warnings.iter().skip(raw.warnings.len()) {
            warn!("{w}");
        }
        let table = PatchTable::from_applied(applied, new_dir);

        // The full execution plan and the order warnings, once per loaded table.
        let prep = table.prepare();
        table.dump_order(prep.order());

        Ok((table, Some(raw), prep))
    }

    /// The table `apply_plan` describes.
    pub fn from_applied(a: Applied, mod_dir: PathBuf) -> PatchTable {
        PatchTable {
            mod_dir,
            targets: a.targets,
            patches: a.patches,
            vars: a.vars,
            var_collisions: a.var_collisions,
            order_rules: a.order_rules,
            load_errors: a.load_errors,
            plan_report: a.report,
            mods: a.mods,
            stack_mode: a.stack_mode,
            line_rules_v2: a.line_rules_v2,
            compat_rules: a.compat_rules,
        }
    }

    /// Like `load`, but a plan error or any mod load error is an `Err`, with the first error's
    /// message (the behaviour before D15). For tools that must not run with a mod skipped.
    pub fn load_strict(mod_dir: &Path) -> Result<PatchTable> {
        Self::load_strict_prepared(mod_dir).map(|(table, _)| table)
    }

    /// `load_strict`, with the `Prepared` built for the order dump (as `load_prepared`), so a
    /// hot reload builds it once.
    pub fn load_strict_prepared(mod_dir: &Path) -> Result<(PatchTable, Prepared)> {
        let (table, prep) = Self::load_prepared(mod_dir)?;
        if let Some(e) = &table.plan_report.plan_error {
            return Err(anyhow::anyhow!("{e}"));
        }
        match table.load_errors.first() {
            Some(e) => Err(anyhow::anyhow!("{}", e.message)),
            None => Ok((table, prep)),
        }
    }

    /// The order patches are applied in. The DLL and the launcher's simulation both go through
    /// this, so they always agree. Logs nothing; `dump_load_order` logs the order warnings.
    pub fn ordered(&self) -> OrderResult {
        self.prepare().order
    }

    /// Builds the derived state once, for any number of `apply_prepared` calls: the mod
    /// graph, the ladders, the order with their required edges, and the suspended rungs.
    pub fn prepare(&self) -> Prepared {
        let mod_ix = self
            .patches
            .iter()
            .map(|(patch, _, _)| {
                patch
                    .mod_id()
                    .and_then(|id| self.mods.iter().position(|m| m.id.eq_ignore_ascii_case(id)))
                    .and_then(|i| u32::try_from(i).ok())
                    .unwrap_or(NO_INDEX)
            })
            .collect();
        let graph = ModGraph::build(&self.mods);
        let (mut ladders, invalid) = anchor::build_ladders(&self.patches, &self.mods, &graph);
        let required = anchor::required_edges(&ladders);
        let requires = self.requirements();
        // A required patch of the same file runs first (D64).
        let requires_edges: Vec<(usize, usize)> = requires
            .iter()
            .enumerate()
            .flat_map(|(a, reqs)| {
                reqs.iter()
                    .flat_map(|r| r.hits.iter())
                    .map(move |&b| (b as usize, a))
            })
            .filter(|&(b, a)| match (self.patches.get(b), self.patches.get(a)) {
                (Some((pb, _, _)), Some((pa, _, _))) => share_a_file(pb, pa),
                _ => false,
            })
            .collect();
        let order = order_patches_full(
            &self.patches,
            &self.order_rules,
            &self.compat_rules,
            &required,
            &requires_edges,
        );
        let mut report = self.plan_report.clone();
        report.overridden_rules = order.overridden_rules.clone();
        report.unknown_ids = order.unknown_ids.clone();
        report.invalid_rungs = invalid;
        for &(a, r) in &order.suspended_rungs {
            let Some((patch, _, _)) = self.patches.get(a) else {
                continue;
            };
            let a_name = patch_name(patch, a);
            let ri = r as usize;
            let Some(Some(ladder)) = ladders.get_mut(a) else {
                continue;
            };
            let b = ladder.b_label.get(ri).cloned().unwrap_or_default();
            let why = format!(
                "{a_name} and {b} would each have to run first, so {a_name} uses its own line"
            );
            if let Some(s) = ladder.suspended.get_mut(ri) {
                *s = true;
            }
            if let Some(slot) = ladder.suspended_reason.get_mut(ri) {
                *slot = Some(why.clone());
            }
            report.suspended_rungs.push(SuspendedRung {
                patch_id: a_name,
                rung: r,
                reason: why,
                source: ladder.source(ri),
            });
        }
        let copy_preds = order::copy_preds(&self.patches, &order.edges);
        let hunks = self
            .patches
            .iter()
            .map(|(patch, _, _)| match patch {
                Patch::Pattern(x) if x.position == crate::patch::InsertPosition::At => {
                    Some(x.normalized())
                }
                _ => None,
            })
            .collect();
        let skips = self
            .patches
            .iter()
            .map(|(patch, _, _)| self.skips_of(patch))
            .collect();
        let stack_rules = self.stack_rules_bound();
        let hints = self
            .patches
            .iter()
            .enumerate()
            .filter_map(|(ix, (patch, _, _))| {
                Some((u32::try_from(ix).ok()?, patch.stack_hint()?))
            })
            .collect();
        Prepared {
            stamp: self.stamp(),
            order,
            graph,
            mod_ix,
            ladders,
            copy_preds,
            hunks,
            skips,
            stack_rules,
            hints,
            requires,
            report,
        }
    }

    /// Per patch, its `requires` names resolved to the patches they name (D64). An adjusted
    /// copy answers for the patch it replaces. A name that only names the patch itself is
    /// dropped.
    fn requirements(&self) -> Vec<Vec<Requirement>> {
        self.patches
            .iter()
            .enumerate()
            .map(|(me, (patch, _, _))| {
                patch
                    .requires()
                    .iter()
                    .map(|name| name.trim())
                    .filter(|name| !name.is_empty())
                    .filter_map(|name| {
                        let mut hits = order::matching_patches(name, &self.patches);
                        if hits.is_empty() {
                            hits = (0..self.patches.len())
                                .filter(|&i| {
                                    self.patches
                                        .get(i)
                                        .is_some_and(|(p, _, _)| copy_of(p, name))
                                })
                                .collect();
                        }
                        if !hits.is_empty() && hits.iter().all(|&h| h == me) {
                            return None;
                        }
                        let hits = hits
                            .into_iter()
                            .filter(|&h| h != me)
                            .filter_map(|h| u32::try_from(h).ok())
                            .collect();
                        Some(Requirement {
                            name: name.to_string(),
                            hits,
                        })
                    })
                    .collect()
            })
            .collect()
    }

    /// Per patch, the stack rules that name it (D55). A rule is put on both patches, so it
    /// holds whichever of them runs first. The other patch is found by its name; ids are in
    /// the stamp.
    fn stack_rules_bound(&self) -> Vec<Vec<StackRuleBound>> {
        let mut out: Vec<Vec<StackRuleBound>> = vec![Vec::new(); self.patches.len()];
        for (i, (patch, _, _)) in self.patches.iter().enumerate() {
            for spec in &patch.extra().stack_rules {
                let named = [(spec.below.as_deref(), true), (spec.above.as_deref(), false)];
                for (name, me_below) in named {
                    let Some(name) = name else { continue };
                    for o in order::matching_patches(name, &self.patches) {
                        let (Ok(me), Ok(other)) = (u32::try_from(i), u32::try_from(o)) else {
                            continue;
                        };
                        // A compat rule never names an adjusted copy (D58).
                        let copy = self
                            .patches
                            .get(o)
                            .is_some_and(|(p, _, _)| p.extra().adjusted_copy);
                        if o == i || (copy && matches!(spec.source, RuleSource::Compat { .. })) {
                            continue;
                        }
                        let bound = |other: u32, me_below: bool| StackRuleBound {
                            other,
                            me_below,
                            site: spec.site.clone(),
                            side: spec.side,
                            source: spec.source.clone(),
                        };
                        if let Some(v) = out.get_mut(i) {
                            v.push(bound(other, me_below));
                        }
                        if let Some(v) = out.get_mut(o) {
                            v.push(bound(me, !me_below));
                        }
                    }
                }
            }
        }
        out
    }

    /// A patch's skipped edits, each kept patch found by its name (D49). Ids are in the stamp.
    fn skips_of(&self, patch: &Patch) -> Vec<SkipAt> {
        patch
            .extra()
            .skip_hunks
            .iter()
            .map(|s| {
                let keep = match s.keep.as_deref() {
                    None => Keeper::Any,
                    Some(name) => self
                        .patches
                        .iter()
                        .position(|(p, _, _)| order::name_matches(name, p))
                        .and_then(|i| u32::try_from(i).ok())
                        .map_or(Keeper::Gone, Keeper::Patch),
                };
                SkipAt {
                    ordinal: s.ordinal,
                    vanilla: s.vanilla.clone(),
                    keep,
                    keep_name: s.keep.clone(),
                }
            })
            .collect()
    }

    /// A hash of everything `prepare` reads: per patch its kind, id, mod, disabled flag,
    /// priority, path, author order lists, `requires` and `PatchExtra`; for an `at` pattern
    /// patch its pattern, payload, `match_indent` and `apply_as` (D47); each mod's relations;
    /// the stack mode; the order and compat rules. Equal stamps mean a `Prepared` still fits
    /// the table.
    pub fn stamp(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.patches.len().hash(&mut h);
        for (patch, prio, path) in &self.patches {
            patch.type_name().hash(&mut h);
            patch.id().hash(&mut h);
            patch.mod_id().hash(&mut h);
            patch.is_disabled().hash(&mut h);
            prio.hash(&mut h);
            path.hash(&mut h);
            author_before(patch).hash(&mut h);
            author_after(patch).hash(&mut h);
            // Only when set, so other patches keep their stamp (D64).
            if !patch.requires().is_empty() {
                ("requires", patch.requires()).hash(&mut h);
            }
            let author = match patch {
                Patch::Pattern(x) => Some((&x.anchors, &x.sites)),
                Patch::Regex(x) => Some((&x.anchors, &x.sites)),
                _ => None,
            };
            if let Some((anchors, sites)) = author {
                if !anchors.is_empty() || !sites.is_empty() {
                    serde_json::to_string(&(anchors, sites))
                        .unwrap_or_else(|e| e.to_string())
                        .hash(&mut h);
                }
            }
            // The edits-mode hunks are made from these (D47).
            if let Patch::Pattern(x) = patch {
                if x.position == crate::patch::InsertPosition::At {
                    (&x.pattern, &x.payload, x.match_indent).hash(&mut h);
                    serde_json::to_string(&x.apply_as)
                        .unwrap_or_else(|e| e.to_string())
                        .hash(&mut h);
                }
            }
            // The author hint (D57), only when set, so other patches keep their stamp.
            if let Some(stack) = match patch {
                Patch::Pattern(x) => x.stack.as_ref(),
                Patch::Regex(x) => x.stack.as_ref(),
                _ => None,
            } {
                serde_json::to_string(stack)
                    .unwrap_or_else(|e| e.to_string())
                    .hash(&mut h);
            }
            // A regex ladder reads the position and whether the payload uses a capture (D52).
            if let Patch::Regex(x) = patch {
                if !x.anchors.is_empty() || !x.qm.rungs.is_empty() {
                    use crate::patch::anchor::RungHost;
                    format!("{:?} {:?}", x.position, x.kind()).hash(&mut h);
                }
            }
            let extra = patch.extra();
            if *extra == PatchExtra::default() {
                0u8.hash(&mut h);
            } else {
                1u8.hash(&mut h);
                serde_json::to_string(extra)
                    .unwrap_or_else(|e| e.to_string())
                    .hash(&mut h);
            }
        }
        self.mods.len().hash(&mut h);
        for m in &self.mods {
            (&m.id, &m.requires, &m.load_after, &m.load_before, &m.provides).hash(&mut h);
        }
        std::mem::discriminant(&self.stack_mode).hash(&mut h);
        // Only when on, so a table on today's rules keeps its stamp.
        if self.line_rules_v2 {
            "line_rules_v2".hash(&mut h);
        }
        let rule = |r: &OrderRule, h: &mut DefaultHasher| (&r.patch, &r.before, &r.after).hash(h);
        self.order_rules.len().hash(&mut h);
        for r in &self.order_rules {
            rule(r, &mut h);
        }
        self.compat_rules.len().hash(&mut h);
        for (r, compat) in &self.compat_rules {
            rule(r, &mut h);
            compat.hash(&mut h);
        }
        h.finish()
    }

    /// Why `prep` does not fit this table, or `None` when it does.
    fn stale_reason(&self, prep: &Prepared) -> Option<String> {
        let now = self.stamp();
        (prep.stamp != now).then(|| {
            format!(
                "The prepared patch state is out of date (stamp {:016x}, table {now:016x}); rebuilt it for this call.",
                prep.stamp
            )
        })
    }

    /// `prep` when it fits this table, else a fresh one (with a warning). Never panics.
    fn fitting<'a>(&self, prep: &'a Prepared) -> Cow<'a, Prepared> {
        match self.stale_reason(prep) {
            None => Cow::Borrowed(prep),
            Some(why) => {
                warn!("{why}");
                #[cfg(test)]
                STALE_WARNINGS.with(|w| w.borrow_mut().push(why));
                Cow::Owned(self.prepare())
            }
        }
    }

    /// Patches in application order.
    fn in_order<'a>(
        &'a self,
        order: &'a OrderResult,
    ) -> impl Iterator<Item = &'a (Patch, Priority, PathBuf)> + 'a {
        order.order.iter().filter_map(move |&i| self.patches.get(i))
    }

    /// Prints all loaded patches in the exact order they will be applied, and the order
    /// warnings.
    pub fn dump_load_order(&self) {
        self.dump_order(&self.ordered());
    }

    /// `dump_load_order` for an order already computed.
    fn dump_order(&self, order: &OrderResult) {
        order.log_messages();
        info!("================================================================================");
        info!("QuarterMaster: Patch Execution Plan");
        info!("================================================================================");

        let sorted = self.in_order(order);

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

    /// Determine if the provided target file / name requires patching. Both sides are
    /// compared in canonical form, with the legacy suffix rule (D44, D45).
    pub fn needs_patching(&self, target: &str) -> bool {
        let target = canonical_path(target);
        self.targets.iter().any(|t| {
            let t = canonical_path(t);
            t == target || (!t.is_empty() && target.ends_with(t.as_str()))
        })
    }

    /// The owner of the module a `=[qm <name> "<source>"]` chunk runs (crash attribution,
    /// §6.2): of the enabled modules named `name`, the last in `order` (a later preload
    /// replaces an earlier one), one with that `source` first. Table order without `order`.
    pub fn module_owner(
        &self,
        order: Option<&OrderResult>,
        name: &str,
        source: &str,
    ) -> Option<crate::RecordOwner> {
        let table_order: Vec<usize>;
        let indices = match order {
            Some(o) => o.order.as_slice(),
            None => {
                table_order = (0..self.patches.len()).collect();
                table_order.as_slice()
            }
        };
        let modules: Vec<(&Patch, &PathBuf, &str)> = indices
            .iter()
            .filter_map(|&i| self.patches.get(i))
            .filter_map(|(p, _, path)| match p {
                Patch::Module(m) if !m.disabled && m.name == name => {
                    Some((p, path, m.display_source.as_str()))
                }
                _ => None,
            })
            .collect();
        let (patch, path, _) = modules
            .iter()
            .rev()
            .find(|(_, _, s)| *s == source)
            .or_else(|| modules.last())?;
        let (patch_id, mod_id) = record_names(patch, path, name);
        let patch_name = patch.name().map_or_else(|| patch_id.clone(), str::to_string);
        Some(crate::RecordOwner {
            mod_id,
            patch_id,
            patch_name,
            ..crate::RecordOwner::default()
        })
    }

    /// Whether the patch a record names (by mod and id) is an adjusted copy (D58).
    pub fn is_adjusted_copy(&self, mod_id: &str, patch_id: &str) -> bool {
        self.patches.iter().any(|(p, _, _)| {
            p.extra().adjusted_copy && p.id() == Some(patch_id) && p.mod_id() == Some(mod_id)
        })
    }

    /// Inject QuarterMaster metadata into the game's Lua runtime.
    /// # Safety
    /// Unsafe due to raw Lua state operations.
    pub unsafe fn inject_metadata(&self, state: *mut LuaState) {
        let mod_dir = self.mod_dir.to_str().unwrap_or_default().replace('\\', "/");
        let repo = "https://github.com/mnb-modding/QuarterMaster";

        use crate::{
            apply_patches, dump_patched, dump_vanilla, get_log_path, getvar, is_dump_all,
            record_asset_winners, record_clean_exit, record_diagnostics, record_lua_crash,
            reload_patches, removevar,
            set_dump_all, setvar,
        };

        let log_path_str = get_log_path().unwrap_or_default();
        let (debug, dev) = crate::RUNTIME.get().map_or((false, false), |q| (q.debug, q.dev));
        // What a diagnostic test run watches (Phase E D89); empty for a normal launch.
        let diagnostics = crate::RUNTIME
            .get()
            .map(|q| q.launch_config.diagnostics_lua())
            .unwrap_or_default();
        // What the startup code's save check reads (Phase E D85).
        let save_info = crate::RUNTIME
            .get()
            .map(|q| {
                q.launch_config
                    .save_info_lua(&|id| self.mods.iter().any(|m| m.id.eq_ignore_ascii_case(id)))
            })
            .unwrap_or_default();

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
                .add_var("dump_patched", dump_patched as LuaFunc)
                .add_var("debug", debug)
                .add_var("dev", dev)
                .add_var("set_dump_all", set_dump_all as LuaFunc)
                .add_var("is_dump_all", is_dump_all as LuaFunc)
                .add_var("record_lua_crash", record_lua_crash as LuaFunc)
                .add_var("record_clean_exit", record_clean_exit as LuaFunc)
                .add_var("record_asset_winners", record_asset_winners as LuaFunc)
                .add_var("save_info", save_info.clone())
                .add_var("diagnostics", diagnostics.clone())
                .add_var("record_diagnostics", record_diagnostics as LuaFunc)
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

    /// Offline patch execution engine (runs without a `*mut LuaState`). Builds a `Prepared`
    /// per call; to apply many buffers, `prepare` once and call `apply_prepared`.
    pub fn apply_patches_offline(
        &self,
        target: &str,
        buffer: &str,
    ) -> Result<(String, PatchDebug, Vec<PatchExecutionRecord>), String> {
        self.apply_patches_inner(&self.prepare(), target, BufferIn::plain(buffer), None)
            .map(InnerApply::into_tuple)
    }

    /// Like `apply_patches_offline`, plus where every line of the result came from (see
    /// `patch::origin`). Builds a `Prepared` per call.
    pub fn apply_patches_traced(
        &self,
        target: &str,
        buffer: &str,
    ) -> Result<(String, PatchDebug, Vec<PatchExecutionRecord>, FileTrace), String> {
        let t = self.traced_inner(&self.prepare(), target, BufferIn::plain(buffer))?;
        Ok((t.text, t.debug, t.records, t.trace))
    }

    /// Applies the patches to one buffer with state from `prepare`. A `prep` built from a
    /// different table is rebuilt for the call, with a warning.
    pub fn apply_prepared(
        &self,
        prep: &Prepared,
        target: &str,
        buffer: &str,
    ) -> Result<(String, PatchDebug, Vec<PatchExecutionRecord>), String> {
        self.apply_patches_inner(&self.fitting(prep), target, BufferIn::plain(buffer), None)
            .map(InnerApply::into_tuple)
    }

    /// Like `apply_prepared`, plus where every line of the result came from (see
    /// `patch::origin`). The patched text is identical; only the simulation uses this.
    pub fn apply_prepared_traced(
        &self,
        prep: &Prepared,
        target: &str,
        buffer: &str,
    ) -> Result<TracedApply, String> {
        self.traced_inner(&self.fitting(prep), target, BufferIn::plain(buffer))
    }

    /// Applies the patches to a buffer `prepare_buffer` made (the DLL's path). Records whose
    /// match touched QuarterMaster's own lines get the `matched_qm_startup_code` warning.
    pub fn apply_prepared_buffer(
        &self,
        prep: &Prepared,
        target: &str,
        buf: &PreparedBuffer,
    ) -> Result<BufferApply, String> {
        let inner =
            self.apply_patches_inner(&self.fitting(prep), target, BufferIn::prepared(buf), None)?;
        Ok(BufferApply {
            line_records: inner.lines.line_records(),
            text: inner.text,
            debug: inner.debug,
            records: inner.records,
        })
    }

    /// `apply_prepared_buffer` with a trace (the simulation's path). Text and records equal
    /// the DLL's.
    pub fn apply_prepared_buffer_traced(
        &self,
        prep: &Prepared,
        target: &str,
        buf: &PreparedBuffer,
    ) -> Result<TracedApply, String> {
        self.traced_inner(&self.fitting(prep), target, BufferIn::prepared(buf))
    }

    fn traced_inner(
        &self,
        prep: &Prepared,
        target: &str,
        input: BufferIn,
    ) -> Result<TracedApply, String> {
        let mut journal = Vec::new();
        let inner = self.apply_patches_inner(prep, target, input, Some(&mut journal))?;
        let raw = inner.raw_text.unwrap_or_else(|| inner.text.clone());
        let mut trace = FileTrace::from_linemap(raw, &inner.lines, journal, inner.removed);
        if trace.text != inner.text {
            // Variable interpolation runs on the whole text after the patches; it changes text
            // inside lines only unless a value holds a newline.
            let total = inner.text.lines().count();
            if trace.lines.len() != total {
                warn!("variable interpolation changed the line count of '{target}'; origins padded");
                trace.lines.resize(total, origin::LineOrigin::Vanilla(0));
            }
            // The pre-vars text stays for the anchor picker: patterns match it.
            trace.raw_text = Some(std::mem::replace(&mut trace.text, inner.text.clone()));
        }
        Ok(TracedApply {
            text: inner.text,
            debug: inner.debug,
            records: inner.records,
            trace,
            lines: inner.lines,
        })
    }

    /// What patch `patch_ix` reads besides the rope in the pattern pass.
    fn apply_ctx<'a>(
        &'a self,
        prep: &'a Prepared,
        lines: &'a LineMap,
        committed: &'a [Vec<u32>],
        original: &'a OriginalLines<'a>,
        names: &'a dyn Fn(u32) -> (String, String),
        patch_ix: usize,
    ) -> ApplyCtx<'a> {
        let rules = prep.stack_rules(patch_ix);
        let hint = self.patches.get(patch_ix).and_then(|(p, _, _)| p.stack_hint());
        // In Legacy only stack rules and author hints move code (D55, D57).
        let stacks = self.stack_mode != StackMode::Legacy
            || !rules.is_empty()
            || !prep.hints().is_empty();
        ApplyCtx {
            lines,
            stack: stacks.then(|| StackCtx {
                mode: self.stack_mode,
                rules,
                hint,
                graph: &prep.graph,
                mods: &self.mods,
                mod_ix: prep.mod_ix(patch_ix),
                names,
            }),
            me: u32::try_from(patch_ix).unwrap_or(NO_INDEX),
            ladder: prep.ladder(patch_ix),
            committed,
            original: Some(original),
            line_rules: self.line_rules_v2,
            hunks: prep.hunks(patch_ix),
            skips: prep.skips(patch_ix),
        }
    }

    /// `prep` must fit this table (callers check the stamp first).
    fn apply_patches_inner(
        &self,
        prep: &Prepared,
        target: &str,
        input: BufferIn,
        mut journal: Option<&mut Vec<JournalEdit>>,
    ) -> Result<InnerApply, String> {
        let target = canonical_path(target);
        let target = target.as_str();

        let mut patch_count = 0usize;
        let mut rope = Rope::from(input.text);
        let mut lines = LineMap::new(rope.line_len(), input.builtin, input.builtin_edited);
        // The line rules (D50, D51); implicit anchors place code only outside Legacy.
        lines.set_line_rules(self.line_rules_v2);
        lines.set_implicit_chains(self.line_rules_v2 && self.stack_mode != StackMode::Legacy);
        lines.set_hints(prep.hints());
        let mut removed_log: Vec<Vec<Removed>> = Vec::new();
        // Debug entries with the index of their record.
        let mut entries: Vec<(ByteDebugEntry, u32)> = Vec::new();
        let mut raw_records: Vec<(usize, PatchExecutionRecord)> = Vec::new();
        // Records whose match touched QuarterMaster's own lines.
        let mut builtin_hits: Vec<usize> = Vec::new();
        let mut skipped_mods: HashSet<String> = HashSet::new();
        // Per patch index, the status of its record in this buffer (D64 reads it).
        let mut status_of: Vec<Option<PatchOutcomeStatus>> = vec![None; self.patches.len()];
        // Per patch index, the records it committed in this buffer (rungs read them).
        let mut committed: Vec<Vec<u32>> = vec![Vec::new(); self.patches.len()];
        let original = OriginalLines::new(input.text);
        let to_u32 = |n: usize| u32::try_from(n).unwrap_or(NO_INDEX);
        // Record id and mod id of a patch, for stack reports.
        let names = |ix: u32| -> (String, String) {
            self.patches
                .get(ix as usize)
                .map(|(p, _, path)| record_names(p, path, target))
                .unwrap_or_default()
        };

        // Copies run first, in order, except a copy that an explicit rule puts after a pattern
        // or regex of this file: it runs in the pattern pass, after that patch (D54).
        let applies = |ix: usize| {
            self.patches
                .get(ix)
                .is_some_and(|(p, _, _)| p.can_apply_to_target(target))
        };
        let position = |ix: usize| prep.order.order.iter().position(|&x| x == ix);
        let ordered: Vec<(usize, CopyPred)> = prep
            .order
            .order
            .iter()
            .filter(|&&ix| matches!(self.patches.get(ix), Some((Patch::Copy(_), _, _))))
            .filter(|&&ix| applies(ix))
            .filter_map(|&ix| {
                prep.copy_preds(ix)
                    .iter()
                    .filter(|p| applies(p.patch))
                    .max_by_key(|p| position(p.patch))
                    .map(|p| (ix, *p))
            })
            .collect();
        let ordered_ix: Vec<usize> = ordered.iter().map(|&(ix, _)| ix).collect();
        let mut steps: Vec<usize> = prep
            .order
            .order
            .iter()
            .copied()
            .filter(|ix| matches!(self.patches.get(*ix), Some((Patch::Copy(_), _, _))))
            .filter(|ix| !ordered_ix.contains(ix))
            .collect();
        steps.extend(order::pattern_pass(&self.patches, &prep.order, &ordered_ix));
        let step_of: HashMap<usize, usize> =
            steps.iter().enumerate().map(|(n, &ix)| (ix, n)).collect();

        for (step, &patch_ix) in steps.iter().enumerate() {
            let Some((patch, prio, path)) = self.patches.get(patch_ix) else {
                continue;
            };
            if let Patch::Copy(copy) = patch {
                let pred = ordered
                    .iter()
                    .find(|&&(ix, _)| ix == patch_ix)
                    .map(|&(_, p)| p);
                let (_, mod_id) = record_names(patch, path, target);
                let skipped = |id_tail: &str, status, warning: &str| {
                    copy_record(
                        copy,
                        path,
                        target,
                        *prio,
                        copy.id.clone().unwrap_or_else(|| format!("{mod_id}:copy:{id_tail}")),
                        mod_id.clone(),
                        status,
                        (0, Some(1)),
                        vec![warning.to_string()],
                    )
                };
                if copy.disabled {
                    if copy.target.can_apply(target) {
                        raw_records.push((
                            usize::MAX,
                            skipped(
                                "disabled",
                                PatchOutcomeStatus::SkippedDisabled,
                                "Disabled by user in profile",
                            ),
                        ));
                        note(&mut status_of, patch_ix, &raw_records);
                    }
                    continue;
                }
                // Only a copy in the pattern pass can follow a failed patch of its mod; the
                // copies that run first ran before every pattern.
                if skipped_mods.contains(&mod_id) {
                    if copy.target.can_apply(target) {
                        raw_records.push((
                            usize::MAX,
                            skipped(
                                "skipped",
                                PatchOutcomeStatus::SkippedByPolicy,
                                SKIPPED_WITH_MOD_WARNING,
                            ),
                        ));
                        note(&mut status_of, patch_ix, &raw_records);
                    }
                    continue;
                }
                // Every copy reads the stack and the map, so it stacks at its line (D53).
                let mut candidate_rope = rope.clone();
                let ctx = self.apply_ctx(prep, &lines, &committed, &original, &names, patch_ix);
                let outcome = copy.apply_in(target, &mut candidate_rope, path, &ctx);
                let Some(mut out) = outcome else {
                    continue;
                };
                rope = candidate_rope;
                info!(
                    "Applied [Copy] from {} (Priority: {}) to '{}'",
                    path.display(),
                    prio,
                    target
                );
                let record = raw_records.len();
                let entry_idx = entries.len();
                // An anchorless copy insert gets an implicit anchor under the line rules (D51).
                let info = lines.commit_with(
                    to_u32(record),
                    to_u32(patch_ix),
                    prep.mod_ix(patch_ix),
                    &out.edits,
                    rope.line_len(),
                    true,
                );
                lines.mark_stack_only(to_u32(record));
                if let Some(c) = committed.get_mut(patch_ix) {
                    c.push(to_u32(record));
                }
                if out.touches_builtin {
                    builtin_hits.push(record);
                }
                let patch_id = copy
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("{mod_id}:copy:{entry_idx}"));
                let patch_name = copy
                    .name
                    .clone()
                    .unwrap_or_else(|| "Copy Patch".to_string());
                if let Some(j) = journal.as_deref_mut() {
                    record_edits(j, record, &rope, &out.entry, &out.edits, &info, &patch_id, &patch_name, &mod_id);
                    removed_log.extend(info.removed);
                }
                patch_count += 1;
                let mut warnings = out.entry.warnings.clone().unwrap_or_default();
                if let Some(p) = pred {
                    warnings.push(copy_pass_warning(&names(to_u32(p.patch)).0, p.via));
                }
                let mut rec = copy_record(
                    copy,
                    path,
                    target,
                    *prio,
                    patch_id,
                    mod_id,
                    PatchOutcomeStatus::Applied,
                    (out.entry.matched_count, out.entry.expected_count),
                    warnings,
                );
                rec.stack_moves = std::mem::take(&mut out.stack_moves);
                raw_records.push((entry_idx, rec));
                note(&mut status_of, patch_ix, &raw_records);
                entries.push((out.entry, to_u32(record)));
                continue;
            }
            if !matches!(patch, Patch::Pattern(..) | Patch::Regex(..)) {
                continue;
            }
            let prio = *prio;
            let (patch_id, mod_id) = record_names(patch, path, target);
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
                            stack_moves: Vec::new(),
                            anchoring: None,
                            fix_ids: Vec::new(),
                            changed_by: patch.extra().changed_by.clone(),
                            suspended: None,
                        },
                    ));
                    note(&mut status_of, patch_ix, &raw_records);
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
                        warnings: vec![SKIPPED_WITH_MOD_WARNING.to_string()],
                        regions: Vec::new(),
                        stack_moves: Vec::new(),
                        anchoring: None,
                        fix_ids: Vec::new(),
                        changed_by: patch.extra().changed_by.clone(),
                        suspended: None,
                    },
                ));
                note(&mut status_of, patch_ix, &raw_records);
                continue;
            }

            // Patch-level `requires` (D64): a required patch of this file must have applied
            // before this one here. One of another file is judged by the table only, so the
            // DLL, which patches each file as it loads, and the simulation agree.
            let mut needs: Vec<String> = Vec::new();
            let mut missing = false;
            let reqs = if patch.can_apply_to_target(target) {
                prep.requires(patch_ix)
            } else {
                &[]
            };
            for req in reqs {
                let name = &req.name;
                if req.hits.is_empty() {
                    needs.push(format!(
                        "{NEEDS_PATCH}:{name}: needs '{name}', which matches no loaded patch"
                    ));
                    continue;
                }
                for &hit in &req.hits {
                    let Some((other, _, _)) = self.patches.get(hit as usize) else {
                        continue;
                    };
                    let other_id = order::patch_name(other, hit as usize);
                    let here =
                        !matches!(other, Patch::Module(_)) && other.can_apply_to_target(target);
                    if !here {
                        if other.is_disabled() {
                            needs.push(format!(
                                "{NEEDS_PATCH}:{name}: needs '{other_id}', which is turned off; it patches another file, so this patch still runs"
                            ));
                        }
                    } else if step_of
                        .get(&(hit as usize))
                        .is_some_and(|&later| later > step)
                    {
                        needs.push(format!(
                            "{NEEDS_PATCH}:{name}: needs '{other_id}', which runs after it in '{target}', so it was not checked"
                        ));
                    } else if status_of.get(hit as usize).copied().flatten()
                        != Some(PatchOutcomeStatus::Applied)
                    {
                        missing = true;
                        needs.push(format!(
                            "{NEEDS_PATCH}:{name}: needs '{other_id}'{DID_NOT_APPLY}"
                        ));
                    }
                }
            }
            for w in &needs {
                warn!("Patch '{patch_id}' on '{target}': {w}");
            }
            if missing {
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
                        status: PatchOutcomeStatus::SkippedByPolicy,
                        warnings: needs,
                        regions: Vec::new(),
                        stack_moves: Vec::new(),
                        anchoring: None,
                        fix_ids: Vec::new(),
                        changed_by: patch.extra().changed_by.clone(),
                        suspended: None,
                    },
                ));
                note(&mut status_of, patch_ix, &raw_records);
                continue;
            }

            let mut candidate_rope = rope.clone();
            let ladder = prep.ladder(patch_ix);
            let ctx = self.apply_ctx(prep, &lines, &committed, &original, &names, patch_ix);
            let result = match patch {
                Patch::Pattern(x) => x.apply_in(target, &mut candidate_rope, path, &ctx),
                Patch::Regex(x) => x.apply_in(target, &mut candidate_rope, path, &ctx),
                // Filtered out above; never reached.
                _ => {
                    warn!("Skipping {} patch '{patch_id}' in the pattern pass", patch.type_name());
                    continue;
                }
            };

            if let Some(mut out) = result {
                let entry = &out.entry;
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
                    && !out.entry.regions.is_empty();
                let record = raw_records.len();
                let entry_idx = entries.len();
                if should_commit {
                    rope = candidate_rope;
                    // Anchorless inserts get an implicit anchor under the line rules (D51, D53).
                    let info = lines.commit_with(
                        to_u32(record),
                        to_u32(patch_ix),
                        prep.mod_ix(patch_ix),
                        &out.edits,
                        rope.line_len(),
                        true,
                    );
                    if matches!(patch, Patch::Regex(..)) {
                        lines.mark_stack_only(to_u32(record));
                    }
                    if let Some(c) = committed.get_mut(patch_ix) {
                        c.push(to_u32(record));
                    }
                    if let Some(j) = journal.as_deref_mut() {
                        record_edits(j, record, &rope, &out.entry, &out.edits, &info, &patch_id, &patch_name, &mod_id);
                        removed_log.extend(info.removed);
                    }
                    if out.touches_builtin {
                        builtin_hits.push(record);
                    }
                    info!(
                        "Applied [{}] '{}' from {} (Priority: {}) to '{}'",
                        patch.type_name(),
                        patch_name,
                        path.display(),
                        prio,
                        target
                    );
                    patch_count += 1;
                } else {
                    // Nothing of it is in the text, so it has no regions.
                    out.entry.regions.clear();
                    out.stack_moves.clear();
                }
                let stack_moves = std::mem::take(&mut out.stack_moves);
                let extra = patch.extra();

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
                        matched_count: out.entry.matched_count,
                        expected_count: out.entry.expected_count,
                        on_failure: policy,
                        status,
                        warnings: {
                            let mut w = out.entry.warnings.clone().unwrap_or_default();
                            w.extend(needs);
                            w
                        },
                        regions: Vec::new(),
                        stack_moves,
                        anchoring: out.anchoring.take(),
                        fix_ids: extra.fix_ids.clone(),
                        changed_by: extra.changed_by.clone(),
                        suspended: ladder.and_then(Ladder::suspension).map(str::to_string),
                    },
                ));
                note(&mut status_of, patch_ix, &raw_records);
                entries.push((out.entry, to_u32(record)));
            }
        }

        let debug = PatchDebug::from_linemap(target, &lines, entries);
        let mut records = Vec::with_capacity(raw_records.len());
        for (entry_idx, mut rec) in raw_records {
            if let Some(dbg_entry) = debug.entries.get(entry_idx) {
                rec.regions = dbg_entry.regions.clone();
            }
            records.push(rec);
        }
        for i in builtin_hits {
            if let Some(rec) = records.get_mut(i) {
                rec.warnings.push(format!(
                    "{MATCHED_QM_STARTUP_CODE}: this patch matched a line QuarterMaster changes in '{}' at startup; the line stays as QuarterMaster wrote it",
                    rec.target_file
                ));
            }
        }

        let mut patched = rope.to_string();
        let raw_text = journal.is_some().then(|| patched.clone());

        if !self.vars.is_empty() {
            vars::apply_var_interp(&mut patched, &self.vars);
        }

        if patch_count == 1 {
            info!("Applied 1 patch to '{target}'");
        } else {
            info!("Applied {patch_count} patches to '{target}'");
        }

        Ok(InnerApply {
            text: patched,
            raw_text,
            debug,
            records,
            lines,
            removed: removed_log,
        })
    }

    /// Apply one or more patches onto the target's buffer at runtime (including `load_now` modules).
    /// Returns the patched content, debug info, and execution records. Builds a `Prepared`
    /// per call.
    /// # Safety
    /// Unsafe due to internal unchecked usages of raw lua state.
    pub unsafe fn apply_patches(
        &self,
        target: &str,
        buffer: &str,
        lua_state: *mut LuaState,
    ) -> Result<(String, PatchDebug, Vec<PatchExecutionRecord>), String> {
        self.apply_prepared_lua(&self.prepare(), target, buffer, lua_state)
    }

    /// `apply_patches` with state from `prepare` (the DLL builds it once per table).
    /// # Safety
    /// Unsafe due to internal unchecked usages of raw lua state.
    pub unsafe fn apply_prepared_lua(
        &self,
        prep: &Prepared,
        target: &str,
        buffer: &str,
        lua_state: *mut LuaState,
    ) -> Result<(String, PatchDebug, Vec<PatchExecutionRecord>), String> {
        let prep = self.fitting(prep);
        self.apply_load_now_modules(&prep, target, lua_state)?;
        self.apply_patches_inner(&prep, target, BufferIn::plain(buffer), None)
            .map(InnerApply::into_tuple)
    }

    /// Runs the `load_now` module patches for `target` (the DLL runs them before it loads a
    /// patched chunk). Stops at the first module that fails.
    /// # Safety
    /// Unsafe due to internal unchecked usages of raw lua state.
    pub unsafe fn apply_load_now_modules(
        &self,
        prep: &Prepared,
        target: &str,
        lua_state: *mut LuaState,
    ) -> Result<(), String> {
        let target = canonical_path(target);
        let target = target.as_str();
        let prep = self.fitting(prep);

        let module_patches = self
            .in_order(&prep.order)
            .filter_map(|(x, prio, path)| match x {
                Patch::Module(patch) => Some((patch, *prio, path)),
                _ => None,
            })
            .filter(|(x, _, _)| x.load_now && !x.disabled);

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
        Ok(())
    }
}


impl InnerApply {
    fn into_tuple(self) -> (String, PatchDebug, Vec<PatchExecutionRecord>) {
        (self.text, self.debug, self.records)
    }
}

/// Notes the status of the record just pushed for patch `patch_ix` (D64).
fn note(
    status_of: &mut [Option<PatchOutcomeStatus>],
    patch_ix: usize,
    records: &[(usize, PatchExecutionRecord)],
) {
    if let (Some(slot), Some((_, rec))) = (status_of.get_mut(patch_ix), records.last()) {
        *slot = Some(rec.status);
    }
}

/// Journals one committed entry, one edit per site. Its byte regions are in application order,
/// each in the coordinates of the text right after it was made, and later regions of the same
/// entry never move earlier ones, so slicing the committed rope gives each region's inserted
/// text. `edits` and `info` are the same edits in lines and what the `LineMap` did with them.
#[allow(clippy::too_many_arguments)]
fn record_edits(
    journal: &mut Vec<JournalEdit>,
    record: usize,
    rope: &Rope,
    entry: &ByteDebugEntry,
    edits: &[Edit],
    info: &CommitInfo,
    patch_id: &str,
    patch_name: &str,
    mod_id: &str,
) {
    if entry.regions.len() != edits.len() {
        warn!(
            "patch '{patch_id}' has {} byte regions for {} line edits",
            entry.regions.len(),
            edits.len()
        );
    }
    let len = rope.byte_len();
    for (i, r) in entry.regions.iter().enumerate() {
        let inserted_len = r.end.saturating_sub(r.start);
        let removed = (inserted_len as isize - r.delta).max(0) as usize;
        let start = r.start.min(len);
        let end = r.end.clamp(start, len);
        journal.push(JournalEdit {
            patch_id: patch_id.to_string(),
            patch_name: patch_name.to_string(),
            mod_id: mod_id.to_string(),
            record,
            start: r.start,
            removed,
            inserted: rope.byte_slice(start..end).to_string(),
            site: edits.get(i).map_or(i as u32, Edit::site),
            block: info
                .blocks
                .get(i)
                .copied()
                .unwrap_or(crate::patch::linemap::NO_BLOCK),
        });
    }
}

/// The record id and mod id of a pattern or regex patch on `target`.
fn record_names(patch: &Patch, path: &Path, target: &str) -> (String, String) {
    let mod_id = patch
        .mod_id()
        .map(|s| s.to_string())
        .unwrap_or_else(|| infer_mod_id_from_path(path));
    let patch_id = patch
        .id()
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{mod_id}:{target}"));
    (patch_id, mod_id)
}

/// A copy patch's record; `counts` is (matched, expected).
#[allow(clippy::too_many_arguments)]
fn copy_record(
    copy: &CopyPatch,
    path: &Path,
    target: &str,
    priority: Priority,
    patch_id: String,
    mod_id: String,
    status: PatchOutcomeStatus,
    counts: (usize, Option<usize>),
    warnings: Vec<String>,
) -> PatchExecutionRecord {
    PatchExecutionRecord {
        patch_id,
        patch_name: copy
            .name
            .clone()
            .unwrap_or_else(|| "Copy Patch".to_string()),
        mod_id,
        source_file: path.display().to_string(),
        target_file: target.to_string(),
        patch_type: "copy".to_string(),
        priority,
        matched_count: counts.0,
        expected_count: counts.1,
        on_failure: copy.on_failure,
        status,
        warnings,
        regions: Vec::new(),
        stack_moves: Vec::new(),
        anchoring: None,
        fix_ids: Vec::new(),
        changed_by: copy.qm.changed_by.clone(),
        suspended: None,
    }
}

/// The `copy_pass_ordered` warning of a copy that runs after `pred` (record id), with the
/// kind of rule that put it there (D54).
fn copy_pass_warning(pred: &str, via: Strength) -> String {
    let (code, why) = match via {
        Strength::Author => ("author", "the mod's own order"),
        Strength::Compat => ("compat", "a compatibility mod's order rule"),
        Strength::Profile => ("profile", "a profile order rule"),
        Strength::Required => ("required", "an anchor"),
    };
    format!("{COPY_PASS_ORDERED}:{code}: runs after '{pred}' because of {why}")
}

fn infer_mod_id_from_path(path: &Path) -> String {
    path.components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or("unknown")
        .to_lowercase()
}

#[cfg(test)]
#[path = "copy_order_tests.rs"]
mod copy_order_tests;

#[cfg(test)]
#[path = "requires_tests.rs"]
mod requires_tests;

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
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            apply_as: None,
            mod_id: Some("mod_a".to_string()),
            disabled: false,
            qm: Default::default(),
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
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            apply_as: None,
            mod_id: Some("mod_a".to_string()),
            disabled: true,
            qm: Default::default(),
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

    #[test]
    fn auto_assets_multi_mod_priority_and_custom_patch_ordering() {
        let temp = tempfile::TempDir::new().unwrap();
        let mods = temp.path();
        std::fs::create_dir_all(mods.join("qm")).unwrap();

        // ModEarly (priority = 10) provides garand1.wav and m60_1.wav
        let mod_early = mods.join("ModEarly");
        std::fs::create_dir_all(mod_early.join("assets/sounds")).unwrap();
        std::fs::write(mod_early.join("assets/sounds/garand1.wav"), b"EARLY_GARAND").unwrap();
        std::fs::write(mod_early.join("assets/sounds/m60_1.wav"), b"EARLY_M60").unwrap();
        std::fs::write(
            mod_early.join("qm.toml"),
            r#"[manifest]
version = "1.0.0"
priority = 10
"#,
        )
        .unwrap();

        // ModLate (priority = 50) overrides m60_1.wav, adds top_m60.png, and uses them in a manual patch after setOptionVolume()
        let mod_late = mods.join("ModLate");
        std::fs::create_dir_all(mod_late.join("assets/sounds")).unwrap();
        std::fs::create_dir_all(mod_late.join("assets/sprites")).unwrap();
        std::fs::write(mod_late.join("assets/sounds/m60_1.wav"), b"LATE_M60").unwrap();
        std::fs::write(mod_late.join("assets/sprites/top_m60.png"), b"PNG_M60").unwrap();
        std::fs::write(
            mod_late.join("qm.toml"),
            r#"[manifest]
version = "1.0.0"
priority = 50

[[patches]]
[patches.pattern]
id = "modlate_use_m60"
name = "Use M60 Assets"
target = "main.lua"
pattern = '''
setOptionVolume()
'''
position = "after"
payload = '''
  m60_ready = (m60_1 ~= nil and top_m60 ~= nil)
'''
match_indent = true
"#,
        )
        .unwrap();

        let table = PatchTable::load(mods).unwrap();
        assert!(table.needs_patching("main.lua"));

        let sample_main = "function love.load()\n  repairSound = love.audio.newSource(\"sounds/repairSound.wav\", \"static\")\n  setOptionVolume()\n  tensionLoop:setLooping(true)\nend\n";
        let (patched, _debug, records) = table
            .apply_patches_offline("main.lua", sample_main)
            .unwrap();

        assert_eq!(records.len(), 3);
        for rec in &records {
            assert_eq!(rec.status, PatchOutcomeStatus::Applied);
            assert!(!rec.regions.is_empty());
        }
        // Verify non-overlapping line regions between ModEarly and ModLate auto-asset patches
        assert!(records[0].regions[0].end_line < records[1].regions[0].start_line);

        // Verify exact execution order in patched buffer:
        // 1. ModEarly auto-assets (priority 10)
        // 2. ModLate auto-assets (priority 50)
        // 3. setOptionVolume()
        // 4. ModLate manual patch after setOptionVolume()
        let idx_early = patched.find("Auto-Asset Patch (modearly").unwrap();
        let idx_late = patched.find("Auto-Asset Patch (modlate").unwrap();
        let idx_vol = patched.find("setOptionVolume()").unwrap();
        let idx_manual = patched.find("m60_ready =").unwrap();

        assert!(idx_early < idx_late, "ModEarly must run before ModLate");
        assert!(idx_late < idx_vol, "ModLate auto-assets must run before setOptionVolume()");
        assert!(idx_vol < idx_manual, "Manual patch must run after setOptionVolume()");
    }

    fn stamp_patch(id: &str, payload: &str) -> Patch {
        Patch::Pattern(PatternPatch {
            pattern: "local x = 1".to_string(),
            position: InsertPosition::After,
            target: Target::Single("main.lua".to_string()),
            payload: payload.to_string(),
            match_indent: false,
            times: None,
            overwrite: false,
            id: Some(id.to_string()),
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
            mod_id: id.split(':').next().map(str::to_string),
            disabled: false,
            qm: Default::default(),
        })
    }

    /// Two patches on one anchor: the one applied last ends up right below it.
    fn stamp_table() -> PatchTable {
        let mut t = PatchTable::default();
        t.targets.insert("main.lua".to_string());
        t.patches
            .push((stamp_patch("a:one", "-- A"), 10, PathBuf::from("A/qm.toml")));
        t.patches
            .push((stamp_patch("b:two", "-- B"), 10, PathBuf::from("B/qm.toml")));
        t.mods = vec![
            ModMeta {
                id: "a".to_string(),
                folder: "A".to_string(),
                manifest_ok: true,
                ..ModMeta::default()
            },
            ModMeta {
                id: "b".to_string(),
                folder: "B".to_string(),
                manifest_ok: true,
                loader_index: 1,
                ..ModMeta::default()
            },
        ];
        t
    }

    #[test]
    fn stamp_changes_when_only_what_prepare_reads_changes() {
        use crate::patch::anchor::{AnchorSource, Rung};
        let base = stamp_table().stamp();
        assert_eq!(stamp_table().stamp(), base, "deterministic");

        // Text the order never reads keeps the stamp.
        let mut t = stamp_table();
        if let Patch::Pattern(p) = &mut t.patches[0].0 {
            p.payload = "-- other".to_string();
        }
        assert_eq!(t.stamp(), base);

        let changed: Vec<(&str, Box<dyn Fn(&mut PatchTable)>)> = vec![
            ("mod.json requires", Box::new(|t| {
                t.mods[0].requires.insert("b".to_string());
            })),
            ("mod.json load_after", Box::new(|t| {
                t.mods[0].load_after.insert("b".to_string());
            })),
            ("mod.json load_before", Box::new(|t| {
                t.mods[1].load_before.insert("a".to_string());
            })),
            ("mod.json provides", Box::new(|t| {
                t.mods[1].provides.insert("lib".to_string());
            })),
            ("a rung", Box::new(|t| {
                t.patches[0].0.extra_mut().rungs.push((
                    Rung {
                        within: Some("b:two".to_string()),
                        ..Rung::default()
                    },
                    AnchorSource::Profile {
                        fix_id: "f1".to_string(),
                    },
                ));
            })),
            ("PatchExtra fix ids", Box::new(|t| {
                t.patches[1].0.extra_mut().fix_ids.push("f2".to_string());
            })),
            ("PatchExtra changed_by", Box::new(|t| {
                t.patches[1].0.extra_mut().changed_by.push("compat".to_string());
            })),
            ("PatchExtra apply_as", Box::new(|t| {
                t.patches[1].0.extra_mut().apply_as =
                    Some(crate::patch::edits::ApplyAs::Edits);
            })),
            ("PatchExtra skip_hunks", Box::new(|t| {
                t.patches[1].0.extra_mut().skip_hunks.push(plan::HunkSkip {
                    ordinal: 0,
                    vanilla: "x".to_string(),
                    keep: Some("a:one".to_string()),
                });
            })),
            ("stack hint", Box::new(|t| {
                if let Patch::Pattern(p) = &mut t.patches[0].0 {
                    p.stack = Some(StackHint::First.into());
                }
            })),
            ("patch requires", Box::new(|t| {
                if let Patch::Pattern(p) = &mut t.patches[1].0 {
                    p.requires.push("a:one".to_string());
                }
            })),
            ("stack_mode", Box::new(|t| t.stack_mode = StackMode::Dependencies)),
            ("line_rules_v2", Box::new(|t| t.line_rules_v2 = true)),
            ("disabled", Box::new(|t| t.patches[0].0.set_disabled(true))),
            ("priority", Box::new(|t| t.patches[0].1 = 20)),
            ("order rule", Box::new(|t| {
                t.order_rules.push(OrderRule {
                    patch: "b:two".to_string(),
                    before: vec!["a:one".to_string()],
                    after: Vec::new(),
                });
            })),
            ("compat rule", Box::new(|t| {
                t.compat_rules.push((
                    OrderRule {
                        patch: "b:two".to_string(),
                        before: vec!["a:one".to_string()],
                        after: Vec::new(),
                    },
                    "compat".to_string(),
                ));
            })),
        ];
        for (what, change) in changed {
            let mut t = stamp_table();
            change(&mut t);
            let ids: Vec<_> = t.patches.iter().map(|(p, _, _)| p.id()).collect();
            assert_eq!(ids, [Some("a:one"), Some("b:two")], "{what}: same ids");
            assert_ne!(t.stamp(), base, "{what} must change the stamp");
        }
    }

    #[test]
    fn stamp_changes_with_a_regex_ladder_input() {
        use crate::lenient::Lenient;
        use crate::patch::anchor::{AnchorSource, Edge, Rung};
        use crate::patch::RegexPatch;
        let regex = |payload: &str| {
            Patch::Regex(RegexPatch {
                target: Target::Single("main.lua".to_string()),
                pattern: "x".to_string(),
                position: InsertPosition::After,
                root_capture: None,
                payload: payload.to_string(),
                line_prepend: String::new(),
                times: None,
                verbose: false,
                id: Some("a:r".to_string()),
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
                mod_id: Some("a".to_string()),
                disabled: false,
                qm: Default::default(),
            })
        };
        let with = |p: Patch| {
            let mut t = stamp_table();
            t.patches[0].0 = p;
            t.stamp()
        };
        let rung = Rung {
            within: Some("b:two".to_string()),
            edge: Some(Edge::End),
            ..Rung::default()
        };
        let authored = |payload: &str, rung: &Rung| {
            let mut p = regex(payload);
            if let Patch::Regex(x) = &mut p {
                x.anchors = vec![Lenient::Ok(rung.clone())];
            }
            p
        };
        let base = with(regex("-- r"));
        // A payload change on a regex with no rungs keeps the stamp.
        assert_eq!(with(regex("-- $1")), base);
        // Author rungs on a regex are hashed (D52).
        let a = with(authored("-- r", &rung));
        assert_ne!(a, base);
        let other = Rung {
            edge: Some(Edge::Start),
            ..rung.clone()
        };
        assert_ne!(with(authored("-- r", &other)), a);
        // With rungs, whether the payload reads a capture decides if an edge rung is valid.
        assert_ne!(with(authored("-- $1", &rung)), a);
        assert_eq!(with(authored("-- s", &rung)), a);
        let mut profile = regex("-- r");
        profile.extra_mut().rungs.push((
            rung,
            AnchorSource::Profile {
                fix_id: "f".to_string(),
            },
        ));
        let p = with(profile.clone());
        if let Patch::Regex(x) = &mut profile {
            x.payload = "-- $1".to_string();
        }
        assert_ne!(with(profile), p);
    }

    /// `stamp_table` with patch 0 an `at` patch: the hunks are made from its pattern and
    /// payload (D47).
    fn at_table() -> PatchTable {
        let mut t = stamp_table();
        if let Patch::Pattern(p) = &mut t.patches[0].0 {
            p.position = InsertPosition::At;
            p.payload = "local x = 1\n-- A".to_string();
        }
        t
    }

    #[test]
    fn stamp_covers_what_the_at_hunks_are_made_from() {
        use crate::lenient::Lenient;
        use crate::patch::edits::ApplyAs;
        let base = at_table().stamp();
        assert_eq!(at_table().stamp(), base, "deterministic");
        let changed: Vec<(&str, Box<dyn Fn(&mut PatternPatch)>)> = vec![
            ("payload", Box::new(|p| p.payload = "local x = 1\n-- B".to_string())),
            ("pattern", Box::new(|p| p.pattern = "local x = *".to_string())),
            ("match_indent", Box::new(|p| p.match_indent = true)),
            ("apply_as", Box::new(|p| p.apply_as = Some(Lenient::Ok(ApplyAs::Edits)))),
        ];
        for (what, change) in changed {
            let mut t = at_table();
            if let Patch::Pattern(p) = &mut t.patches[0].0 {
                change(p);
            }
            assert_ne!(t.stamp(), base, "{what} of an `at` patch must change the stamp");
        }
        // A before/after payload is still not read by `prepare`.
        let mut t = at_table();
        if let Patch::Pattern(p) = &mut t.patches[1].0 {
            p.payload = "-- other".to_string();
        }
        assert_eq!(t.stamp(), base);
    }

    /// A `Prepared` made before an `at` patch's payload changed (same id) is stale, so its old
    /// hunks are never applied.
    #[test]
    fn a_prepared_from_before_an_at_change_is_rebuilt() {
        use crate::lenient::Lenient;
        use crate::patch::edits::ApplyAs;
        let src = "local x = 1\n";
        let mut t = at_table();
        if let Patch::Pattern(p) = &mut t.patches[0].0 {
            p.apply_as = Some(Lenient::Ok(ApplyAs::Edits));
        }
        let prep = t.prepare();
        take_stale_warnings();
        let (before, _, _) = t.apply_prepared(&prep, "main.lua", src).unwrap();
        assert!(take_stale_warnings().is_empty());
        assert!(before.contains("-- A\n"), "{before}");
        if let Patch::Pattern(p) = &mut t.patches[0].0 {
            p.payload = "local x = 1\n-- C\n-- D".to_string();
        }
        assert!(t.stale_reason(&prep).is_some());
        let (after, _, _) = t.apply_prepared(&prep, "main.lua", src).unwrap();
        assert!(after.contains("-- C\n-- D\n") && !after.contains("-- A"), "{after}");
        assert_eq!(take_stale_warnings().len(), 1);
        let fresh = t.prepare();
        assert_eq!(fresh.hunks(0).map(|h| h.as_ref().map(Vec::len)), Some(Ok(2)));
        assert!(fresh.hunks(1).is_none(), "only `at` patches have hunks");
    }

    fn take_stale_warnings() -> Vec<String> {
        STALE_WARNINGS.with(|w| std::mem::take(&mut *w.borrow_mut()))
    }

    #[test]
    fn stale_prepared_is_rebuilt_with_a_warning() {
        let src = "local x = 1\n";
        let mut t = stamp_table();
        let prep = t.prepare();
        assert!(t.stale_reason(&prep).is_none());
        take_stale_warnings();
        let (before, _, _) = t.apply_prepared(&prep, "main.lua", src).unwrap();
        assert!(take_stale_warnings().is_empty());
        assert_eq!(before, "local x = 1\n-- B\n-- A\n");

        // A mutation after prepare (as patch tools and tests do): B now runs first.
        t.order_rules.push(OrderRule {
            patch: "b:two".to_string(),
            before: vec!["a:one".to_string()],
            after: Vec::new(),
        });
        let why = t.stale_reason(&prep).expect("the stamp no longer fits");
        assert!(why.contains("out of date"), "{why}");
        let (after, _, records) = t.apply_prepared(&prep, "main.lua", src).unwrap();
        assert_eq!(after, "local x = 1\n-- A\n-- B\n", "rebuilt, not the stale order");
        assert_eq!(records[0].patch_id, "b:two");
        let traced = t.apply_prepared_traced(&prep, "main.lua", src).unwrap();
        assert_eq!(traced.text, after);
        assert_eq!(traced.trace.text, after);
        // Each call that rebuilt warned, with the reason.
        assert_eq!(take_stale_warnings(), [why.clone(), why]);
        assert!(t.stale_reason(&t.prepare()).is_none());
    }

    #[test]
    fn prepared_order_matches_ordered() {
        let mut t = stamp_table();
        t.order_rules.push(OrderRule {
            patch: "b:two".to_string(),
            before: vec!["a:one".to_string()],
            after: Vec::new(),
        });
        let prep = t.prepare();
        assert_eq!(prep.order(), &t.ordered());
        assert_eq!(prep.stamp(), t.stamp());
    }

    fn line_patch(
        id: &str,
        pattern: &str,
        position: InsertPosition,
        payload: &str,
        times: Option<usize>,
        on_failure: OnFailurePolicy,
    ) -> Patch {
        Patch::Pattern(PatternPatch {
            pattern: pattern.to_string(),
            position,
            target: Target::Single("main.lua".to_string()),
            payload: payload.to_string(),
            match_indent: false,
            times,
            overwrite: false,
            id: Some(id.to_string()),
            name: Some(format!("name of {id}")),
            description: None,
            requires: Vec::new(),
            priority: None,
            before: Vec::new(),
            after: Vec::new(),
            on_failure,
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            apply_as: None,
            mod_id: id.split(':').next().map(str::to_string),
            disabled: false,
            qm: Default::default(),
        })
    }

    fn line_table(patches: Vec<Patch>) -> PatchTable {
        let mut t = PatchTable::default();
        t.targets.insert("main.lua".to_string());
        for (i, p) in patches.into_iter().enumerate() {
            t.patches.push((p, 10 + i as i32, PathBuf::from("M/qm.toml")));
        }
        t
    }

    fn regions(rec: &PatchExecutionRecord) -> Vec<(usize, usize)> {
        rec.regions.iter().map(|r| (r.start_line, r.end_line)).collect()
    }

    #[test]
    fn regions_are_exact_when_a_later_patch_writes_inside_another() {
        // b inserts inside a's block: a's region is split, not stretched over b's line.
        let t = line_table(vec![
            line_patch("a:1", "first", InsertPosition::After, "x\ny", None, OnFailurePolicy::Warn),
            line_patch("b:1", "x", InsertPosition::After, "z", None, OnFailurePolicy::Warn),
        ]);
        let (text, debug, records) = t.apply_patches_offline("main.lua", "first\nlast\n").unwrap();
        assert_eq!(text, "first\nx\nz\ny\nlast\n");
        assert_eq!(regions(&records[0]), vec![(2, 2), (4, 4)]);
        assert_eq!(regions(&records[1]), vec![(3, 3)]);
        assert_eq!(debug.find_entry_at_line(3).unwrap().patch_source.patch_id.as_deref(), Some("b:1"));
        assert_eq!(debug.find_entry_at_line(4).unwrap().patch_source.patch_id.as_deref(), Some("a:1"));

        // An `at` that deletes another patch's line before a third patch's output: the
        // regions after it move up exactly.
        let t = line_table(vec![
            line_patch("a:1", "first", InsertPosition::After, "x\ny", None, OnFailurePolicy::Warn),
            line_patch("c:1", "last", InsertPosition::Before, "w", None, OnFailurePolicy::Warn),
            line_patch("b:1", "x", InsertPosition::At, "", None, OnFailurePolicy::Warn),
        ]);
        let (text, _, records) = t.apply_patches_offline("main.lua", "first\nlast\n").unwrap();
        assert_eq!(text, "first\n\ny\nw\nlast\n");
        let by_id: HashMap<&str, Vec<(usize, usize)>> =
            records.iter().map(|r| (r.patch_id.as_str(), regions(r))).collect();
        assert_eq!(by_id["a:1"], vec![(3, 3)]);
        assert_eq!(by_id["b:1"], vec![(2, 2)]);
        assert_eq!(by_id["c:1"], vec![(4, 4)]);
    }

    #[test]
    fn uncommitted_records_get_no_regions() {
        // Two matches where one is wanted, skipped by policy: nothing is written.
        let t = line_table(vec![line_patch(
            "a:1",
            "dup",
            InsertPosition::After,
            "x",
            Some(1),
            OnFailurePolicy::SkipPatch,
        )]);
        let (text, debug, records) = t.apply_patches_offline("main.lua", "dup\ndup\n").unwrap();
        assert_eq!(text, "dup\ndup\n");
        assert_eq!(records[0].status, PatchOutcomeStatus::SkippedByPolicy);
        assert!(records[0].regions.is_empty());
        assert!(debug.entries[0].regions.is_empty());
        assert_eq!(debug.entries.len(), 1, "the entry stays, so later entries keep their index");
    }

    #[test]
    fn line_records_attribute_each_line_to_its_last_writer() {
        let t = line_table(vec![
            line_patch("a:1", "first", InsertPosition::After, "x\ny", None, OnFailurePolicy::Warn),
            line_patch("b:1", "y", InsertPosition::At, "Y", None, OnFailurePolicy::Warn),
        ]);
        let prep = t.prepare();
        let buf = crate::prepare::PreparedBuffer {
            text: "first\nlast\n".to_string(),
            ..Default::default()
        };
        let a = t.apply_prepared_buffer(&prep, "@main.lua", &buf).unwrap();
        assert_eq!(a.text, "first\nx\nY\nlast\n");
        use crate::patch::linemap::NO_RECORD;
        assert_eq!(a.line_records, vec![NO_RECORD, 0, 1, NO_RECORD]);
        assert_eq!(a.records[1].patch_id, "b:1");
        // The traced path gives the same map.
        let traced = t.apply_prepared_buffer_traced(&prep, "main.lua", &buf).unwrap();
        assert_eq!(traced.lines.line_records(), a.line_records);
        assert_eq!(traced.text, a.text);
    }

    #[test]
    fn an_unterminated_last_line_is_touched_not_anchored() {
        let t = line_table(vec![
            line_patch("a:1", "end", InsertPosition::After, "tail", None, OnFailurePolicy::Warn),
            line_patch("b:1", "start", InsertPosition::Before, "head", None, OnFailurePolicy::Warn),
        ]);
        let traced = t
            .apply_prepared_traced(&t.prepare(), "main.lua", "start\nend")
            .unwrap();
        // Today's bytes: the payload joins the unterminated line.
        assert_eq!(traced.text, "head\nstart\nendtail\n");
        use crate::patch::linemap::{LineKind, Side};
        let kinds: Vec<_> = traced.lines.tags().iter().map(|t| t.kind).collect();
        assert_eq!(kinds, vec![LineKind::Inserted, LineKind::Vanilla, LineKind::Modified]);
        assert!(traced.lines.chain(1, Side::After).is_empty());
        assert_eq!(traced.lines.chain(1, Side::Before).len(), 1);
        assert_eq!(traced.trace, origin::trace("start\nend", traced.trace.edits.clone()));
    }
}
