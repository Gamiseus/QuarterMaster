#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
//! Compatibility mod overrides (Phase C §3.10, §8.1).
//!
//! A mod's qm.toml can carry `[[overrides]]` entries that change another mod's patch: turn it
//! off, give it a priority, order it, anchor it on other code, or order its block at a shared
//! line (D55). An entry applies only when
//! the compat mod `requires` every mod it refers to (D12). A profile fix on the same patch
//! wins over it, and it wins over the author (D13). Among compat mods the later one in loader
//! order wins each change. Every entry is reported, applied or not.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::handshake::OverrideReport;
use crate::lenient::{lenient_list, Lenient};
use crate::patch::anchor::{mod_label, Rung, SiteKey};
use crate::patch::edits::{ApplyAs, HunkKey};
use crate::patch::fingerprint::PatchFingerprintLite;
use crate::patch::linemap::Side;
use crate::patch::order::{self, OrderRule};
use crate::patch::plan::{
    check_hunk, order_name, Fingerprints, FpMode, HunkSkip, ModGraph, RawLoad,
};
use crate::patch::stack::{RuleSource, StackRuleSpec};
use crate::patch::Priority;

/// One `[[overrides]]` entry. Unknown keys make the entry invalid, so a typo never applies
/// half a change.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Override {
    /// The patch to change: "mod_id:patch_id".
    pub patch: String,
    /// The patch's content when the compat mod was made; a changed patch is left alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<PatchFingerprintLite>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "lenient_list"
    )]
    pub anchors: Vec<Lenient<Rung>>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "lenient_list"
    )]
    pub sites: Vec<Lenient<SiteKey>>,
    /// False: the anchors come before the author's. True: the author's are dropped.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replace_anchors: bool,
    /// How the `at` patch runs: `"edits"` or `"literal"` (D46).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_as: Option<ApplyAs>,
    /// Edits of the `at` patch that do not run, so another patch's change to the same lines
    /// stays (D49). The patch then runs as separate edits.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "lenient_list"
    )]
    pub skip_hunks: Vec<Lenient<HunkKey>>,
    /// At a shared line, put this patch's block below this patch's block ("mod:patch", D55).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_below: Option<String>,
    /// At a shared line, put this patch's block above this patch's block (D55).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_above: Option<String>,
    /// The game's line the stack rule is for; every line where both blocks meet when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_site: Option<SiteKey>,
    /// The side of that line; both sides when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_side: Option<Side>,
}

const DISABLE: &str = "disable";
const PRIORITY: &str = "priority";
const ORDER: &str = "order";
const ANCHORS: &str = "anchors";
const APPLY_AS: &str = "apply_as";
const SKIP_HUNKS: &str = "skip_hunks";
const STACK: &str = "stack";

impl Override {
    /// The changes the entry makes, in report order.
    fn actions(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.disable {
            out.push(DISABLE);
        }
        if self.priority.is_some() {
            out.push(PRIORITY);
        }
        if !self.before.is_empty() || !self.after.is_empty() {
            out.push(ORDER);
        }
        if !self.anchors.is_empty() || !self.sites.is_empty() || self.replace_anchors {
            out.push(ANCHORS);
        }
        if self.apply_as.is_some() {
            out.push(APPLY_AS);
        }
        if !self.skip_hunks.is_empty() {
            out.push(SKIP_HUNKS);
        }
        if self.stack_below.is_some() || self.stack_above.is_some() {
            out.push(STACK);
        }
        out
    }
}

/// `entries` as `[[overrides]]` qm.toml text, each after its comment lines (`# ` added).
pub fn overrides_toml(entries: &[(String, Override)]) -> Result<String, String> {
    #[derive(Serialize)]
    struct One<'a> {
        overrides: [&'a Override; 1],
    }
    let mut out = String::new();
    for (comment, o) in entries {
        for line in comment.lines().map(str::trim) {
            out.push('#');
            if !line.is_empty() {
                out.push(' ');
                out.push_str(line);
            }
            out.push('\n');
        }
        let text = toml::to_string(&One { overrides: [o] }).map_err(|e| e.to_string())?;
        out.push_str(&text);
        out.push('\n');
    }
    Ok(out)
}

/// The raw patches the profile layer changes, so its fixes win (D13).
#[derive(Clone, Debug, Default)]
pub(crate) struct ProfileTouch {
    pub priority: HashSet<usize>,
    pub anchors: HashSet<usize>,
    /// Replaced by an adjusted copy.
    pub replaced: HashSet<usize>,
    pub apply_as: HashSet<usize>,
    pub skip_hunks: HashSet<usize>,
    /// The ids of the profile's adjusted copies, lowercase. They belong to the profile, so
    /// no compat entry may name one (D58).
    pub copies: HashSet<String>,
}

/// Compat rungs for one patch.
#[derive(Clone, Debug, Default)]
pub(crate) struct CompatAnchors {
    pub mod_id: String,
    pub rungs: Vec<Rung>,
    pub sites: Vec<SiteKey>,
    pub replace: bool,
}

/// What the compat layer does to one raw patch.
#[derive(Clone, Debug, Default)]
pub(crate) struct CompatEdit {
    pub disable: bool,
    pub priority: Option<Priority>,
    pub anchors: Option<CompatAnchors>,
    pub apply_as: Option<ApplyAs>,
    /// The edits that do not run, from every compat mod that skips one (D49).
    pub skip_hunks: Vec<HunkSkip>,
    /// Stack rules from every compat mod that has one for the patch (D55).
    pub stack_rules: Vec<StackRuleSpec>,
    /// The compat mods with a change that applies.
    pub changed_by: Vec<String>,
}

/// The compat layer: per raw patch index its edits, the order rules (`Strength::Compat`)
/// and one or more reports per entry.
#[derive(Clone, Debug, Default)]
pub(crate) struct CompatLayer {
    pub edits: HashMap<usize, CompatEdit>,
    pub rules: Vec<(OrderRule, String)>,
    pub reports: Vec<OverrideReport>,
}

/// What became of one change of an entry that passed the gate.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Fate {
    Applied,
    /// A later compat mod (its id) changed the same thing.
    Superseded(String),
    Profile,
}

/// An entry that passed the gate and found its patch.
struct Live<'a> {
    /// The entry's index in `raw.overrides`.
    n: usize,
    from: String,
    o: &'a Override,
    targets: Vec<usize>,
    fates: Vec<(&'static str, Fate)>,
    /// "<old id> is now <new id>" when the patch was found by its content.
    moved: Option<String>,
    /// Per target, the edits the entry skips there (D49).
    skips: Vec<(usize, Vec<HunkSkip>)>,
}

impl Live<'_> {
    fn set(&mut self, action: &str, fate: Fate) {
        for (a, f) in &mut self.fates {
            if *a == action {
                *f = fate.clone();
            }
        }
    }
}

/// How an entry's `patch` met the target mod's patches.
enum Found {
    Bound(Vec<usize>),
    /// Not found by name; one patch has the fingerprint's content.
    Rebound(usize),
    Missing,
    Drifted,
    /// Several patches have the content and their place does not pick one.
    Ambiguous(usize),
}

/// Builds the compat layer from every loaded mod's overrides. Pure; fingerprints are
/// computed only for the patches an entry with a fingerprint names, or for its target mod's
/// patches when the name finds none.
pub(crate) fn compat_layer(raw: &RawLoad, touch: &ProfileTouch) -> CompatLayer {
    let mut out = CompatLayer::default();
    if raw.overrides.is_empty() {
        return out;
    }
    let graph = ModGraph::build(&raw.mods);
    let mut fps = Fingerprints::new(raw, FpMode::Lazy);
    let mut live: Vec<Live> = Vec::new();
    let mut priority_by: HashMap<usize, usize> = HashMap::new();
    let mut anchors_by: HashMap<usize, usize> = HashMap::new();
    let mut apply_as_by: HashMap<usize, usize> = HashMap::new();

    // Per report, the entry it is about, so reports keep the entries' order.
    let mut ranks: Vec<usize> = Vec::new();
    for (n, (mod_ix, entry)) in raw.overrides.iter().enumerate() {
        ranks.resize(out.reports.len(), n.saturating_sub(1));
        let mod_ix = *mod_ix as usize;
        let from = raw
            .mods
            .get(mod_ix)
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let c = mod_label(&raw.mods, &from);
        let o = match entry {
            Lenient::Ok(o) => o,
            Lenient::Invalid { raw: v, error } => {
                let target = v.get("patch").and_then(|p| p.as_str()).unwrap_or_default();
                out.reports.push(report(
                    &from,
                    target,
                    "",
                    "invalid",
                    format!("{c}: an override could not be read and is skipped: {error}"),
                ));
                continue;
            }
        };
        let target = o.patch.trim();
        let actions = o.actions();
        let all = actions.join(", ");
        let fail = |status: &str, detail: String| report(&from, target, &all, status, detail);
        let Some((seg, _)) = target
            .split_once(':')
            .filter(|(m, r)| !m.is_empty() && !r.is_empty())
        else {
            let detail = format!(
                "{c}: the override of '{target}' must name a patch as <mod id>:<patch id>; it is skipped"
            );
            out.reports.push(fail("invalid", detail));
            continue;
        };
        if actions.is_empty() {
            let detail = format!("{c}: the override of {target} changes nothing; it is skipped");
            out.reports.push(fail("invalid", detail));
            continue;
        }
        // An adjusted copy belongs to the profile, whichever mod it runs as (D58).
        let named_copy = std::iter::once(target)
            .chain(o.before.iter().chain(&o.after).map(String::as_str))
            .chain(o.stack_below.iter().chain(&o.stack_above).map(String::as_str))
            .find(|n| names_copy(touch, n));
        if let Some(n) = named_copy {
            let detail = if n == target {
                format!("{c}: {target} is an adjusted copy, which belongs to the profile; the change is skipped")
            } else {
                format!("{c}'s change to {target} names {n}, an adjusted copy, which belongs to the profile; it is skipped")
            };
            out.reports.push(fail("invalid", detail));
            continue;
        }
        for (what, err) in o
            .anchors
            .iter()
            .filter_map(|r| r.error().map(|e| ("A rung", e)))
            .chain(
                o.sites
                    .iter()
                    .filter_map(|s| s.error().map(|e| ("A site", e))),
            )
            .chain(
                o.skip_hunks
                    .iter()
                    .filter_map(|h| h.error().map(|e| ("A skipped change", e))),
            )
        {
            out.reports.push(report(
                &from,
                target,
                ANCHORS,
                "invalid",
                format!(
                    "{what} in {c}'s change to {target} could not be read and is skipped: {err}"
                ),
            ));
        }

        // The gate (D12): every mod the entry refers to must be required.
        let target_mod = graph.resolve(seg);
        let mut allowed: BTreeSet<String> = graph.requires_resolved(&from);
        if let Some(m) = raw.mods.get(mod_ix) {
            allowed.extend(m.requires.iter().map(|r| graph.resolve(r)));
        }
        allowed.insert(from.clone());
        let mut refs = vec![target_mod.clone()];
        for rung in o.anchors.iter().filter_map(Lenient::ok) {
            if let Some(w) = rung.within.as_deref() {
                refs.extend(name_mods(raw, &graph, w));
            }
        }
        for name in o.before.iter().chain(&o.after) {
            refs.extend(name_mods(raw, &graph, name));
        }
        // A skipped change names the patch whose change stays.
        for keep in o.skip_hunks.iter().filter_map(Lenient::ok) {
            if let Some(k) = keep.keep.as_deref() {
                refs.extend(name_mods(raw, &graph, k));
            }
        }
        // A stack rule names the other patch at the line.
        for name in o.stack_below.iter().chain(&o.stack_above) {
            refs.extend(name_mods(raw, &graph, name));
        }
        if let Some(m) = refs.iter().find(|m| !allowed.contains(*m)) {
            let ml = mod_label(&raw.mods, m);
            let detail = if *m == target_mod {
                format!("{c} lists a change to {ml}'s patch but does not require {ml}; QuarterMaster ignores it")
            } else {
                format!("{c} lists a change to {target} that refers to {ml} but does not require {ml}; QuarterMaster ignores it")
            };
            out.reports.push(fail("rejected_not_required", detail));
            continue;
        }

        let Some(t_mod_ix) = raw.mods.iter().position(|m| m.id == target_mod) else {
            let ml = mod_label(&raw.mods, &target_mod);
            let detail = format!(
                "Waiting for {ml}: {c}'s change to {target} does nothing while {ml} is off"
            );
            out.reports.push(fail("waiting", detail));
            continue;
        };
        let ml = mod_label(&raw.mods, &target_mod);
        let (targets, moved) = match find(raw, t_mod_ix, o, &mut fps) {
            Found::Bound(v) => (v, None),
            Found::Rebound(i) => {
                let to = raw
                    .patches
                    .get(i)
                    .and_then(|rp| rp.patch.id())
                    .unwrap_or_default();
                (vec![i], Some(format!("{target} is now {to}")))
            }
            Found::Missing => {
                let detail = format!("{target} is not in {ml}; {c}'s change to it does nothing");
                out.reports.push(fail("target_missing", detail));
                continue;
            }
            Found::Drifted => {
                let detail =
                    format!("{target} changed since {c} was made; {c}'s change to it is off");
                out.reports.push(fail("suspended_drift", detail));
                continue;
            }
            Found::Ambiguous(n) => {
                let detail =
                    format!("{n} patches of {ml} could be {target}; {c}'s change to it is off");
                out.reports.push(fail("suspended_drift", detail));
                continue;
            }
        };

        // Each skipped change must still name one of the patch's edits (D49): one that does
        // not skips nothing.
        let mut skips: Vec<(usize, Vec<HunkSkip>)> = Vec::new();
        for &t in &targets {
            let mut list = Vec::new();
            for key in o.skip_hunks.iter().filter_map(Lenient::ok) {
                match check_hunk(raw, t, key, &mut fps) {
                    Ok(skip) => HunkSkip::add_to(&mut list, skip),
                    Err(why) => out.reports.push(report(
                        &from,
                        target,
                        SKIP_HUNKS,
                        "suspended_drift",
                        format!("{c}'s skipped change to {target} does nothing: {why}"),
                    )),
                }
            }
            if !list.is_empty() {
                skips.push((t, list));
            }
        }
        let actions: Vec<&'static str> = actions
            .into_iter()
            .filter(|a| *a != SKIP_HUNKS || !skips.is_empty())
            .collect();

        let k = live.len();
        let replaced = targets.iter().any(|t| touch.replaced.contains(t));
        let mut fates: Vec<(&'static str, Fate)> = Vec::new();
        for a in actions {
            let by_profile = replaced
                || (a == PRIORITY && targets.iter().any(|t| touch.priority.contains(t)))
                || (a == ANCHORS && targets.iter().any(|t| touch.anchors.contains(t)))
                || (a == APPLY_AS && targets.iter().any(|t| touch.apply_as.contains(t)))
                || (a == SKIP_HUNKS && targets.iter().any(|t| touch.skip_hunks.contains(t)));
            fates.push((
                a,
                if by_profile {
                    Fate::Profile
                } else {
                    Fate::Applied
                },
            ));
        }
        let entry = Live {
            n,
            from: from.clone(),
            o,
            targets,
            fates,
            moved,
            skips,
        };
        // Later compat mods win each change; the earlier one is superseded.
        for (action, by) in [
            (PRIORITY, &mut priority_by),
            (ANCHORS, &mut anchors_by),
            (APPLY_AS, &mut apply_as_by),
        ] {
            if !entry
                .fates
                .iter()
                .any(|(a, f)| *a == action && *f == Fate::Applied)
            {
                continue;
            }
            for &t in &entry.targets {
                if let Some(prev) = by.insert(t, k) {
                    if let Some(p) = live.get_mut(prev) {
                        p.set(action, Fate::Superseded(from.clone()));
                    }
                }
            }
        }
        live.push(entry);
    }

    ranks.resize(out.reports.len(), raw.overrides.len().saturating_sub(1));
    let winners = Winners {
        priority: priority_by,
        anchors: anchors_by,
        apply_as: apply_as_by,
    };
    for (k, l) in live.iter().enumerate() {
        finish(raw, k, l, &winners, &mut out);
        ranks.resize(out.reports.len(), l.n);
    }
    let mut tagged: Vec<(usize, OverrideReport)> = ranks.into_iter().zip(out.reports).collect();
    tagged.sort_by_key(|(n, _)| *n);
    out.reports = tagged.into_iter().map(|(_, r)| r).collect();
    out
}

/// Per raw patch, the live entry whose priority, anchors and `apply_as` apply (the last one).
struct Winners {
    priority: HashMap<usize, usize>,
    anchors: HashMap<usize, usize>,
    apply_as: HashMap<usize, usize>,
}

/// Turns live entry `k`'s applied changes into edits and rules, and reports each fate.
fn finish(raw: &RawLoad, k: usize, l: &Live, winners: &Winners, out: &mut CompatLayer) {
    let o = l.o;
    let target = o.patch.trim();
    let c = mod_label(&raw.mods, &l.from);
    let applied = |a: &str| l.fates.iter().any(|(x, f)| *x == a && *f == Fate::Applied);
    for &t in &l.targets {
        let edit = out.edits.entry(t).or_default();
        if applied(DISABLE) {
            edit.disable = true;
        }
        // Per patch: a later entry may have won on some of this entry's patches only.
        if winners.priority.get(&t) == Some(&k) {
            edit.priority = o.priority;
        }
        if winners.apply_as.get(&t) == Some(&k) {
            edit.apply_as = o.apply_as;
        }
        // Stack rules add up across compat mods, as disables do (D55).
        if applied(STACK) {
            edit.stack_rules.push(StackRuleSpec {
                site: o.stack_site.clone(),
                side: o.stack_side,
                below: o.stack_below.clone(),
                above: o.stack_above.clone(),
                source: RuleSource::Compat {
                    mod_id: l.from.clone(),
                },
            });
        }
        // Skipped changes add up across compat mods, as disables do.
        if applied(SKIP_HUNKS) {
            for (_, list) in l.skips.iter().filter(|(x, _)| *x == t) {
                for skip in list {
                    HunkSkip::add_to(&mut edit.skip_hunks, skip.clone());
                }
            }
        }
        if winners.anchors.get(&t) == Some(&k) {
            edit.anchors = Some(CompatAnchors {
                mod_id: l.from.clone(),
                rungs: o.anchors.iter().filter_map(Lenient::ok).cloned().collect(),
                sites: o.sites.iter().filter_map(Lenient::ok).cloned().collect(),
                replace: o.replace_anchors,
            });
        }
        if l.fates.iter().any(|(_, f)| *f == Fate::Applied) && !edit.changed_by.contains(&l.from) {
            edit.changed_by.push(l.from.clone());
        }
        if applied(ORDER) {
            let id = raw
                .patches
                .get(t)
                .and_then(|rp| rp.patch.id())
                .unwrap_or(target);
            let rule = OrderRule {
                patch: order_name(raw, id, t),
                before: o.before.clone(),
                after: o.after.clone(),
            };
            out.rules.push((rule, l.from.clone()));
        }
    }

    // One report per fate, in action order.
    let mut groups: Vec<(&Fate, Vec<&str>)> = Vec::new();
    for (a, f) in &l.fates {
        match groups.iter_mut().find(|(g, _)| *g == f) {
            Some((_, v)) => v.push(a),
            None => groups.push((f, vec![a])),
        }
    }
    for (fate, actions) in groups {
        let list = actions.join(", ");
        let (status, detail) = match fate {
            Fate::Applied => {
                let moved = l
                    .moved
                    .as_deref()
                    .map(|m| format!(" ({m})"))
                    .unwrap_or_default();
                ("applied", format!("{c} changes {target}{moved}: {list}"))
            }
            // Two entries of one compat mod on the same patch: its later entry wins.
            Fate::Superseded(by) if *by == l.from => (
                "superseded",
                format!("A later entry in {c} changes {target} too ({list})"),
            ),
            Fate::Superseded(by) => (
                "superseded",
                format!(
                    "{}'s change to {target} replaces {c}'s ({list})",
                    mod_label(&raw.mods, by)
                ),
            ),
            Fate::Profile => (
                "superseded_by_profile",
                format!("Your fix on {target} replaces {c}'s change ({list})"),
            ),
        };
        out.reports
            .push(report(&l.from, target, &list, status, detail));
    }
}

fn report(from: &str, target: &str, action: &str, status: &str, detail: String) -> OverrideReport {
    OverrideReport {
        from_mod: from.to_string(),
        target: target.to_string(),
        action: action.to_string(),
        status: status.to_string(),
        detail,
    }
}

/// Whether `name` (an id, `mod:id` or `=id`) names one of the profile's adjusted copies.
fn names_copy(touch: &ProfileTouch, name: &str) -> bool {
    let bare = name.trim().trim_start_matches(order::EXACT).to_lowercase();
    touch.copies.contains(&bare)
        || bare
            .split_once(':')
            .is_some_and(|(_, rest)| touch.copies.contains(rest))
}

/// The mods a patch name refers to: the mods of the patches it matches, else the mod part of
/// the name, resolved through `provides`. A bare name that matches nothing refers to none.
fn name_mods(raw: &RawLoad, graph: &ModGraph, name: &str) -> Vec<String> {
    let name = name.trim();
    let bare = name.strip_prefix(order::EXACT).unwrap_or(name);
    if let Some(m) = bare.strip_suffix(":*") {
        return vec![graph.resolve(m)];
    }
    let hits: BTreeSet<String> = raw
        .patches
        .iter()
        .filter(|rp| order::name_matches(name, &rp.patch))
        .filter_map(|rp| raw.mods.get(rp.mod_ix as usize))
        .map(|m| m.id.clone())
        .collect();
    if !hits.is_empty() {
        return hits.into_iter().collect();
    }
    match bare.split_once(':') {
        Some((m, _)) if !m.is_empty() => vec![graph.resolve(m)],
        _ => Vec::new(),
    }
}

/// Whether raw patch `rp` answers to `target` ("mod:patch"), ignoring case. The mod part may
/// be a `provides` key, so `<its mod>:<patch part>` counts too.
fn named(rp: &crate::patch::plan::RawPatch, target: &str) -> bool {
    let rest = target.split_once(':').map_or(target, |(_, r)| r);
    // The D62 stem id counts as the mod too.
    let mods: Vec<&str> = std::iter::once(rp.patch.mod_id().unwrap_or_default())
        .chain(rp.patch.extra().legacy_mod.as_deref())
        .collect();
    let in_mod = |x: &str| {
        mods.iter()
            .any(|m| format!("{m}:{x}").eq_ignore_ascii_case(target))
    };
    let eq = |k: &str| {
        k.eq_ignore_ascii_case(target)
            || k.eq_ignore_ascii_case(rest)
            || mods
                .iter()
                .any(|m| k.eq_ignore_ascii_case(&format!("{m}:{rest}")))
    };
    let id_hit = |id: &str| eq(id) || in_mod(id);
    rp.patch.id().is_some_and(id_hit)
        || rp.patch.extra().legacy_names().any(id_hit)
        || rp.patch.name().is_some_and(in_mod)
}

/// Finds an entry's patches among mod `mod_ix`'s (§7.2, compat row). Without a fingerprint
/// every patch the name matches. With one, the named patch with that content; a named patch
/// whose content changed is Drifted; when the name finds none, the mod's one patch with that
/// content (narrowed by place).
fn find(raw: &RawLoad, mod_ix: usize, o: &Override, fps: &mut Fingerprints) -> Found {
    let target = o.patch.trim();
    let in_mod: Vec<usize> = raw
        .patches
        .iter()
        .enumerate()
        .filter(|(_, rp)| rp.mod_ix as usize == mod_ix)
        .map(|(i, _)| i)
        .collect();
    let hits: Vec<usize> = in_mod
        .iter()
        .copied()
        .filter(|&i| raw.patches.get(i).is_some_and(|rp| named(rp, target)))
        .collect();
    let Some(fp) = o.fingerprint.as_ref() else {
        return if hits.is_empty() {
            Found::Missing
        } else {
            Found::Bound(hits)
        };
    };
    let mut same = |v: &[usize]| -> Vec<usize> {
        v.iter()
            .copied()
            .filter(|&i| {
                fps.get(i)
                    .is_some_and(|f| f.anchor == fp.anchor && f.payload == fp.payload)
            })
            .collect()
    };
    let placed = |v: &[usize]| -> Vec<usize> {
        if fp.source.is_none() && fp.ordinal.is_none() {
            return v.to_vec();
        }
        v.iter()
            .copied()
            .filter(|&i| {
                raw.patches.get(i).is_some_and(|rp| {
                    fp.source
                        .as_deref()
                        .is_none_or(|s| s.replace('\\', "/").eq_ignore_ascii_case(&rp.source))
                        && fp.ordinal.is_none_or(|n| n == rp.ordinal)
                })
            })
            .collect()
    };
    let named_same = same(&hits);
    match named_same.as_slice() {
        [one] => return Found::Bound(vec![*one]),
        [_, _, ..] => {
            return match placed(&named_same).as_slice() {
                [one] => Found::Bound(vec![*one]),
                _ => Found::Ambiguous(named_same.len()),
            }
        }
        [] if !hits.is_empty() => return Found::Drifted,
        [] => {}
    }
    let moved = same(&in_mod);
    match moved.as_slice() {
        [] => Found::Missing,
        [one] => Found::Rebound(*one),
        _ => match placed(&moved).as_slice() {
            [one] => Found::Rebound(*one),
            _ => Found::Ambiguous(moved.len()),
        },
    }
}
