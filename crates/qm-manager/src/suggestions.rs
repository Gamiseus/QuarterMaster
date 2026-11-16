//! Suggested fixes (Phase C §9.3). Each suggestion is checked with a preview of the profile
//! as Apply would leave it, and listed only when the preview passes its rule:
//! - stale: a Disable fix that is no longer needed (turning the patch back on adds no conflict
//!   and breaks no file);
//! - follow (D11): put a patch back next to the copy of its line another mod wrote back, when
//!   that mod is one it depends on;
//! - combined (D26): turn a patch back on and anchor the patch it would break next to its copy
//!   of the line, as one change;
//! - rebind: a fix whose patch moved or became ambiguous asks the user to check it;
//! - retire: an Anchor fix that no longer places anything.
//!
//! Nothing here writes; suggestions are never applied automatically.

use std::collections::{BTreeSet, HashMap};

use qm_core::handshake::{BindingStatus, FixBinding, LoadOrderPlan, PatchOutcomeStatus};
use qm_core::patch::anchor::{Rung, SiteKey};
use qm_core::patch::plan::{self, FpMode, ModGraph, RawLoad};
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, Priority};

use crate::conflict::{ConsumerKind, RungHint, SimulationReport};
use crate::fixes::{self, FixChange, FixSource, Provenance};
use crate::profiles::ModProfile;

/// Conflict kinds that say code moved, was wrapped or now sits in a comment.
pub const PLACEMENT_KINDS: [&str; 3] = [
    "placement_changed",
    "placement_in_comment",
    "anchor_in_comment",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    Stale,
    Follow,
    Combined,
    Rebind,
    Retire,
}

impl SuggestionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SuggestionKind::Stale => "stale",
            SuggestionKind::Follow => "follow",
            SuggestionKind::Combined => "combined",
            SuggestionKind::Rebind => "rebind",
            SuggestionKind::Retire => "retire",
        }
    }
}

/// An Anchor fix a suggestion adds.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorDraft {
    pub patch_id: String,
    pub anchors: Vec<Rung>,
    pub sites: Vec<SiteKey>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub key: String,
    pub kind: SuggestionKind,
    pub label: String,
    pub adds: Vec<AnchorDraft>,
    /// Ids of the fixes it removes.
    pub removes: Vec<String>,
    /// The fix it is about (rebind, retire, stale).
    pub fix_id: Option<String>,
    /// The conflict it resolves, when there is one.
    pub conflict_id: Option<String>,
    /// The preview shows no new conflict, broken file or moved code.
    pub safe: bool,
}

/// What suggestions are made from.
pub struct SuggestCtx<'a> {
    /// A migrated profile.
    pub profile: &'a ModProfile,
    pub raw: Option<&'a RawLoad>,
    /// The profile's current simulation and the table it ran.
    pub report: &'a SimulationReport,
    pub table: &'a PatchTable,
    /// The bindings of the profile's plan.
    pub bindings: &'a [FixBinding],
    /// The most previews to run.
    pub budget: usize,
}

/// `profile` as Apply leaves it with `s`: its fixes undone, its Anchor fixes made.
pub fn apply(profile: &ModProfile, raw: Option<&RawLoad>, s: &Suggestion) -> ModProfile {
    let mut p = profile.clone();
    for id in &s.removes {
        fixes::undo(&mut p, raw, id);
    }
    for a in &s.adds {
        let prov = Provenance {
            option_key: Some(s.key.clone()),
            conflict_id: s.conflict_id.clone(),
            ..Provenance::new(FixSource::Suggestion)
        };
        fixes::op_anchor_set(
            &mut p,
            raw,
            &a.patch_id,
            a.anchors.clone(),
            a.sites.clone(),
            Some(a.label.clone()),
            prov,
        );
    }
    p
}

/// The table `profile` gives over `raw`, compiled as activation compiles it (through JSON, as
/// the game reads the plan).
pub fn table_for(
    profile: &ModProfile,
    raw: &RawLoad,
    enabled_mods: Vec<String>,
    mod_priorities: HashMap<String, Priority>,
) -> PatchTable {
    let compiled = crate::profiles::build_plan(profile, enabled_mods, mod_priorities);
    let json = serde_json::to_string(&compiled).unwrap_or_default();
    let applied = plan::apply_plan_with(raw, &LoadOrderPlan::parse(&json), FpMode::All);
    PatchTable::from_applied(applied, raw.mod_dir.clone())
}

/// What a preview changes against the current simulation.
struct Delta {
    new_conflicts: BTreeSet<String>,
    resolved: BTreeSet<String>,
    breaks_file: bool,
    new_placement: bool,
    files_changed: Vec<String>,
}

/// `next` simulated only `files`; the rest of `cur` is left out of the comparison.
fn delta(cur: &SimulationReport, next: &SimulationReport, files: &[String]) -> Delta {
    let inside = |f: &String| files.contains(f);
    // Info notes (`multi_match`) are not conflicts, and semantic findings (D69) are
    // approximate warnings, as in `compare_resolution`. The preview also simulates only
    // `files`, so the whole-game scans of the semantic checks see less than `cur` did.
    let ids = |r: &SimulationReport| -> BTreeSet<String> {
        r.conflicts
            .iter()
            .filter(|c| {
                c.severity != "info"
                    && !crate::semantic::is_semantic(c)
                    && inside(&c.target_file)
            })
            .map(|c| c.id.clone())
            .collect()
    };
    let broken = |r: &SimulationReport| -> BTreeSet<String> {
        r.conflicts
            .iter()
            .filter(|c| c.kind == "compile_error" && inside(&c.target_file))
            .map(|c| c.target_file.clone())
            .collect()
    };
    let placement = |r: &SimulationReport| -> BTreeSet<String> {
        r.conflicts
            .iter()
            .filter(|c| PLACEMENT_KINDS.contains(&c.kind.as_str()) && inside(&c.target_file))
            .map(|c| c.id.clone())
            .collect()
    };
    let (a, b) = (ids(cur), ids(next));
    let files: BTreeSet<&String> = cur
        .traces
        .keys()
        .chain(next.traces.keys())
        .filter(|f| inside(f))
        .collect();
    Delta {
        new_conflicts: b.difference(&a).cloned().collect(),
        resolved: a.difference(&b).cloned().collect(),
        breaks_file: broken(next).difference(&broken(cur)).next().is_some(),
        new_placement: placement(next).difference(&placement(cur)).next().is_some(),
        files_changed: files
            .into_iter()
            .filter(|f| cur.traces.get(*f).map(|t| &t.text) != next.traces.get(*f).map(|t| &t.text))
            .cloned()
            .collect(),
    }
}

/// Display names for labels.
struct Names<'a> {
    report: &'a SimulationReport,
    table: &'a PatchTable,
}

impl Names<'_> {
    fn mod_name(&self, id: &str) -> String {
        self.table
            .mods
            .iter()
            .find(|m| m.id.eq_ignore_ascii_case(id))
            .and_then(|m| m.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| id.to_string())
    }

    fn patch_name(&self, id: &str) -> String {
        self.report
            .files_touched
            .iter()
            .flat_map(|f| &f.records)
            .find(|r| r.patch_id == id)
            .map(|r| r.patch_name.clone())
            .or_else(|| {
                self.table
                    .patches
                    .iter()
                    .find(|(p, _, _)| p.id() == Some(id))
                    .and_then(|(p, _, _)| p.name().map(str::to_string))
            })
            .unwrap_or_else(|| id.to_string())
    }

    /// "Mod › patch".
    fn both(&self, mod_id: &str, patch_id: &str) -> String {
        format!("{} › {}", self.mod_name(mod_id), self.patch_name(patch_id))
    }
}

/// The Anchor fix a rung hint stands for.
fn anchor_draft(h: &RungHint, names: &Names) -> AnchorDraft {
    AnchorDraft {
        patch_id: h.patch_id.clone(),
        anchors: vec![Rung {
            within: Some(h.within.clone()),
            pattern: Some(h.pattern.clone()),
            exact: true,
            ..Rung::default()
        }],
        sites: vec![SiteKey {
            context: h.context.clone(),
            ordinal: None,
        }],
        label: format!(
            "Keep {}'s {} next to {}'s copy of its line",
            names.mod_name(&h.mod_id),
            names.patch_name(&h.patch_id),
            names.mod_name(&h.within_mod)
        ),
    }
}

/// The conflict ids of patch `id` in `file` that a fix on it would clear.
fn conflict_ids_of(file: &str, id: &str) -> [String; 2] {
    [
        format!("broken-anchor:{file}:{id}"),
        format!("sites-lost:{file}:{id}"),
    ]
}

/// The patch ids a plan name of a fix refers to.
fn ids_of(raw: Option<&RawLoad>, name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    if let Some(raw) = raw {
        for i in fixes::hits(raw, name, false) {
            if let Some(id) = raw.patches.get(i).and_then(|rp| rp.patch.id()) {
                if !out.iter().any(|x| x == id) {
                    out.push(id.to_string());
                }
            }
        }
    }
    out
}

/// The game files a suggestion can change: the target files of the patches it names (the
/// patches of the fixes it removes, and each anchored patch and the patch it goes next to).
pub fn files_of(ctx: &SuggestCtx, s: &Suggestion) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for id in &s.removes {
        if let Some(f) = fixes::find(ctx.profile, id) {
            names.extend(f.subject().map(str::to_string));
            names.extend(f.refs().iter().map(|r| r.id.clone()));
        }
    }
    for a in &s.adds {
        names.push(a.patch_id.clone());
        names.extend(a.anchors.iter().filter_map(|r| r.within.clone()));
    }
    let mut files: BTreeSet<String> = BTreeSet::new();
    for name in &names {
        let named = qm_core::patch::order::matching_patches(name, &ctx.table.patches);
        let hits = ctx
            .table
            .patches
            .iter()
            .enumerate()
            .filter(|(i, (p, _, _))| p.id() == Some(name.as_str()) || named.contains(i));
        for (_, (p, _, _)) in hits {
            for t in &ctx.table.targets {
                let t = t.trim_start_matches('@');
                if !t.is_empty() && p.can_apply_to_target(t) {
                    files.insert(t.to_string());
                }
            }
        }
    }
    files.into_iter().collect()
}

/// The suggestions for `ctx.profile`. `preview` simulates a profile as Apply would leave it,
/// on the given game files only (None when it cannot). At most `ctx.budget` previews run; the
/// rest is left out.
pub fn suggest(
    ctx: &SuggestCtx,
    preview: &mut dyn FnMut(&ModProfile, &[String]) -> Option<SimulationReport>,
) -> Vec<Suggestion> {
    let names = Names {
        report: ctx.report,
        table: ctx.table,
    };
    let mut out: Vec<Suggestion> = Vec::new();
    let mut budget = ctx.budget;
    let mut run = |s: &Suggestion, budget: &mut usize| -> Option<(SimulationReport, Delta)> {
        if *budget == 0 {
            return None;
        }
        let files = files_of(ctx, s);
        if files.is_empty() {
            return None;
        }
        *budget -= 1;
        let p = apply(ctx.profile, ctx.raw, s);
        let next = preview(&p, &files)?;
        let d = delta(ctx.report, &next, &files);
        Some((next, d))
    };
    let live: Vec<&fixes::ProfileFix> = fixes::fixes(ctx.profile).filter(|f| !f.paused).collect();

    // Rebind: a fix whose patch moved, or that names several patches now.
    let mut asked: BTreeSet<&str> = BTreeSet::new();
    for b in ctx.bindings {
        let Some(f) = live.iter().find(|f| f.id == b.fix_id) else {
            continue;
        };
        let label = match &b.status {
            BindingStatus::Rebound { from, to } => {
                format!("Check \"{}\": {from} is now {to}", f.label)
            }
            BindingStatus::Ambiguous { .. } => {
                format!("Pick which patch \"{}\" is for", f.label)
            }
            _ => continue,
        };
        if !asked.insert(f.id.as_str()) {
            continue;
        }
        out.push(Suggestion {
            key: format!("rebind:{}", f.id),
            kind: SuggestionKind::Rebind,
            label,
            adds: Vec::new(),
            removes: Vec::new(),
            fix_id: Some(f.id.clone()),
            conflict_id: None,
            safe: false,
        });
    }

    // Retire: an Anchor fix whose code uses its own line at every place while the patch
    // applies as written.
    for f in &live {
        if !matches!(f.change, FixChange::Anchor { .. }) {
            continue;
        }
        let recs: Vec<_> = ctx
            .report
            .files_touched
            .iter()
            .flat_map(|file| &file.records)
            .filter(|r| r.fix_ids.contains(&f.id))
            .collect();
        let idle = !recs.is_empty()
            && recs.iter().all(|r| {
                r.status == PatchOutcomeStatus::Applied
                    && r.anchoring.as_ref().is_some_and(|a| {
                        !a.sites.is_empty()
                            && a.sites
                                .iter()
                                .all(|s| s.rung.is_none() && s.fallback.is_some())
                    })
            });
        if !idle {
            continue;
        }
        let mut s = Suggestion {
            key: format!("retire:{}", f.id),
            kind: SuggestionKind::Retire,
            label: format!("No longer needed: {}", f.label),
            adds: Vec::new(),
            removes: vec![f.id.clone()],
            fix_id: Some(f.id.clone()),
            conflict_id: None,
            safe: false,
        };
        if let Some((_, d)) = run(&s, &mut budget) {
            if d.files_changed.is_empty() && d.new_conflicts.is_empty() {
                s.safe = true;
                out.push(s);
            }
        }
    }

    // Follow (D11): the Reemitted case, one place, a mod A depends on.
    let graph = ModGraph::build(&ctx.table.mods);
    let anchored: BTreeSet<&str> = live
        .iter()
        .filter_map(|f| match &f.change {
            FixChange::Anchor { patch, .. } => Some(patch.id.as_str()),
            _ => None,
        })
        .collect();
    let mut followed: BTreeSet<(String, String)> = BTreeSet::new();
    for loss in &ctx.report.losses {
        let (Some(c), Some(h)) = (&loss.consumer, &loss.rung) else {
            continue;
        };
        if c.kind != ConsumerKind::Reemitted
            || h.patch_id != loss.patch_id
            || !h.single_site
            || !matches!(h.position, InsertPosition::Before | InsertPosition::After)
            || anchored.contains(h.patch_id.as_str())
            || !graph.depends_on(&loss.mod_id, &h.within_mod)
            || !followed.insert((loss.target_file.clone(), loss.patch_id.clone()))
        {
            continue;
        }
        let ids = conflict_ids_of(&loss.target_file, &loss.patch_id);
        let conflict_id = ctx
            .report
            .conflicts
            .iter()
            .find(|x| ids.contains(&x.id))
            .map(|x| x.id.clone());
        let draft = anchor_draft(h, &names);
        let mut s = Suggestion {
            key: format!("follow:{}:{}", loss.target_file, loss.patch_id),
            kind: SuggestionKind::Follow,
            label: draft.label.clone(),
            adds: vec![draft],
            removes: Vec::new(),
            fix_id: None,
            conflict_id: conflict_id.clone(),
            safe: false,
        };
        let Some((_, d)) = run(&s, &mut budget) else {
            continue;
        };
        let resolves = conflict_id
            .as_ref()
            .is_none_or(|id| d.resolved.contains(id));
        if d.new_conflicts.is_empty()
            && !d.breaks_file
            && !d.new_placement
            && d.files_changed == [loss.target_file.clone()]
            && resolves
        {
            s.safe = true;
            out.push(s);
        }
    }

    // Stale, else combined (D26): turn a disabled patch back on, alone or with an Anchor fix
    // for the patch it would break.
    for f in &live {
        let FixChange::Disable { patch } = &f.change else {
            continue;
        };
        // A patch the user turned off by hand is a choice, not a fix to retire.
        if matches!(
            f.provenance.source,
            FixSource::PatchToggle | FixSource::OrderView
        ) {
            continue;
        }
        let off = patch.mod_id.clone();
        let mut stale = Suggestion {
            key: format!("stale:{}", f.id),
            kind: SuggestionKind::Stale,
            label: format!(
                "Turn {} back on",
                names.both(if off.is_empty() { &patch.id } else { &off }, &patch.id)
            ),
            adds: Vec::new(),
            removes: vec![f.id.clone()],
            fix_id: Some(f.id.clone()),
            conflict_id: None,
            safe: false,
        };
        let Some((next, d)) = run(&stale, &mut budget) else {
            continue;
        };
        let stale_ok = d.new_conflicts.is_empty() && !d.breaks_file;
        stale.safe = stale_ok && !d.new_placement;
        // On again, the patch takes another patch's line, or another patch's code hangs on
        // its copy of a line: tie that patch to the copy, as one change (D26).
        let ids = ids_of(ctx.raw, &patch.id);
        let mut hints: Vec<&RungHint> = next
            .losses
            .iter()
            .filter_map(|l| {
                let h = l.rung.as_ref()?;
                let c = l.consumer.as_ref()?;
                let involved = ids.contains(&c.patch_id) || ids.contains(&l.patch_id);
                (involved && ids.contains(&h.within)).then_some(h)
            })
            .collect();
        hints.extend(
            next.kept_anchors
                .iter()
                .map(|(_, h)| h)
                .filter(|h| ids.contains(&h.within)),
        );
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        hints.retain(|h| !anchored.contains(h.patch_id.as_str()) && seen.insert(&h.patch_id));
        if let Some(first) = hints.first() {
            let b_mod = if off.is_empty() {
                first.within_mod.clone()
            } else {
                off.clone()
            };
            let more = match hints.len() - 1 {
                0 => String::new(),
                1 => " (and 1 other patch)".to_string(),
                n => format!(" (and {n} other patches)"),
            };
            let mut s = Suggestion {
                key: format!("combined:{}:{}", f.id, first.patch_id),
                kind: SuggestionKind::Combined,
                label: format!(
                    "Turn {} back on, and keep {}'s {}{more} next to {}'s copy of its line",
                    names.both(&b_mod, &patch.id),
                    names.mod_name(&first.mod_id),
                    names.patch_name(&first.patch_id),
                    names.mod_name(&b_mod)
                ),
                adds: hints.iter().map(|h| anchor_draft(h, &names)).collect(),
                removes: vec![f.id.clone()],
                fix_id: Some(f.id.clone()),
                conflict_id: None,
                safe: false,
            };
            if let Some((_, d)) = run(&s, &mut budget) {
                if d.new_conflicts.is_empty() && !d.breaks_file {
                    s.safe = !d.new_placement;
                    out.push(s);
                    continue;
                }
            }
        }
        if stale_ok {
            out.push(stale);
        }
    }
    out
}

#[cfg(test)]
#[path = "suggestions_tests.rs"]
mod tests;
