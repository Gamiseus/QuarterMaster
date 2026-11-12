//! The one patch ordering function. The game DLL and the launcher's simulation both apply
//! patches in the order computed here (through `PatchTable::ordered`); nothing else sorts patches.
//!
//! Constraints, weakest first:
//! 1. the tie-break: priority, then the order the loader produced (mod order, then file order),
//!    which is exactly the old stable sort by priority;
//! 2. author `before` / `after` lists on pattern, regex and copy patches;
//! 3. compat rules from compatibility mods (Phase C D35);
//! 4. profile rules from `load-order.json` `patch_order`;
//! 5. required edges: a patch anchored on another patch's code runs after it (§4.5), and a
//!    patch runs after the patches of its own file that its `requires` names (D64).
//!
//! An edge is dropped when the reverse edge is strictly stronger, and the drop is reported.
//!
//! A patch is named in a rule by its id (`gcore:hud_lua:1`, or an author id like `hud_overlay`),
//! by `mod_id:patch_id`, or by `mod_id:*` for every patch of that mod.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use crate::patch::{Patch, Priority};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::path::PathBuf;

/// A profile rule: `patch` runs before every patch in `before` and after every patch in `after`.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRule {
    pub patch: String,
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderResult {
    /// Indexes into the patch list, in application order.
    pub order: Vec<usize>,
    /// Each cycle that had to be broken, as patch names in cycle order.
    pub cycles: Vec<Vec<String>>,
    /// Rules a stronger rule contradicted and replaced: `"<a> before <b>"` for an author or
    /// compat rule a profile rule beat, `"<a> before <b> replaced by <compat mod>"`, and
    /// `"profile rule <a> before <b> ignored: <a> is anchored on <b>"`.
    pub overridden_rules: Vec<String>,
    /// Names in rules that matched no loaded patch (the mod may be disabled).
    pub unknown_ids: Vec<String>,
    /// The sort hit a state that should be impossible; the patches not yet placed were
    /// appended by priority, then loader order.
    pub degraded: bool,
    /// Rungs whose required edge had to be dropped to break a cycle: (patch A, rung index).
    /// A falls back to its own anchor at those rungs.
    pub suspended_rungs: Vec<(usize, u32)>,
    /// The required edges that were added, as (B, A): B runs before A.
    pub implicit_edges: Vec<(usize, usize)>,
    /// The requirement edges that were added, as (B, A): A's `requires` names B (D64).
    pub requires_edges: Vec<(usize, usize)>,
    /// The edges left after contradictions and cycles, with their provenance (D54).
    pub edges: Vec<OrderEdge>,
    /// What the sort has to say, for the log. Ordering itself logs nothing, so a table that
    /// is ordered many times logs once (`log_messages`).
    pub messages: Vec<(log::Level, String)>,
}

impl OrderResult {
    /// Writes `messages` to the log.
    pub fn log_messages(&self) {
        for (level, msg) in &self.messages {
            log::log!(*level, "{msg}");
        }
    }
}

/// How strong an order edge is. The derived `Ord` follows the variant order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    Author,
    Compat,
    Profile,
    /// A rung: B runs before the patch anchored on it.
    Required,
}

/// An order edge that survived the sort: `from` runs before `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderEdge {
    pub from: usize,
    pub to: usize,
    /// The strongest rule that named `to` itself, by id or `mod_id:patch_id` (an author list
    /// on `to` names it too). `None` when only `mod_id:*` names gave the edge (D54).
    pub explicit: Option<Strength>,
}

/// A pattern or regex that explicit edges put before a copy, directly or through other
/// explicit edges (D54).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyPred {
    pub patch: usize,
    /// The strongest explicit edge into the copy on a path from `patch`.
    pub via: Strength,
}

/// Whether a rule name names one patch rather than `mod_id:*`.
fn names_itself(name: &str) -> bool {
    !name.trim().ends_with(":*")
}

/// The name a patch goes by in reports: `mod_id:id`, or just the id when it already starts with
/// its mod (the loader's fallback ids do), or `#<index>` for a patch with no id.
pub fn patch_name(patch: &Patch, index: usize) -> String {
    match (patch.mod_id(), patch.id()) {
        (Some(m), Some(id)) if !id.starts_with(&format!("{m}:")) => format!("{m}:{id}"),
        (_, Some(id)) => id.to_string(),
        _ => format!("#{index}"),
    }
}

pub(crate) fn author_before(p: &Patch) -> &[String] {
    match p {
        Patch::Pattern(x) => &x.before,
        Patch::Regex(x) => &x.before,
        Patch::Copy(x) => &x.before,
        // A module's `before` names a file, not a patch.
        Patch::Module(_) => &[],
    }
}

pub(crate) fn author_after(p: &Patch) -> &[String] {
    match p {
        Patch::Pattern(x) => &x.after,
        Patch::Regex(x) => &x.after,
        Patch::Copy(x) => &x.after,
        Patch::Module(_) => &[],
    }
}

/// Every patch a rule name refers to (an id, `mod_id:patch_id` or `mod_id:*`). A patch whose
/// duplicate fallback id got a `#<toml>` suffix still answers to its old id (D29).
pub fn matching_patches(name: &str, patches: &[(Patch, Priority, PathBuf)]) -> Vec<usize> {
    patches
        .iter()
        .enumerate()
        .filter(|(_, (p, _, _))| name_matches(name, p))
        .map(|(i, _)| i)
        .collect()
}

/// Marks a rule name the plan layer bound to one patch: it matches that exact id only, not a
/// D29 duplicate's old id (`plan::bind`).
pub(crate) const EXACT: char = '=';

/// Whether a rule name refers to `p` (see `matching_patches`).
pub fn name_matches(name: &str, p: &Patch) -> bool {
    let name = name.trim();
    if let Some(id) = name.strip_prefix(EXACT) {
        return p.id() == Some(id);
    }
    // The D62 stem id counts as the mod too.
    let legacy_mod = p.extra().legacy_mod.as_deref();
    let mods = || p.mod_id().into_iter().chain(legacy_mod);
    if let Some(m) = name.strip_suffix(":*") {
        return mods().any(|x| x.eq_ignore_ascii_case(m));
    }
    p.id()
        .into_iter()
        .chain(p.extra().legacy_names())
        .any(|id| id == name || mods().any(|m| format!("{m}:{id}") == name))
}

/// Orders `patches` (as the loader produced them) under the author rules they carry and the
/// profile `rules`. Deterministic: the same input always gives the same order and cycles.
pub fn order_patches(patches: &[(Patch, Priority, PathBuf)], rules: &[OrderRule]) -> OrderResult {
    order_patches_with(patches, rules, &[], &[])
}

/// `order_patches` with compat rules (each with its compat mod id) and required edges
/// `(b, a, rung)`: patch `b` runs before patch `a`, because rung `rung` of `a` anchors on `b`.
pub fn order_patches_with(
    patches: &[(Patch, Priority, PathBuf)],
    rules: &[OrderRule],
    compat_rules: &[(OrderRule, String)],
    required: &[(usize, usize, u32)],
) -> OrderResult {
    order_patches_full(patches, rules, compat_rules, required, &[])
}

/// `order_patches_with` plus requirement edges `(b, a)`: `a`'s `requires` names `b`, a patch
/// of the same file, so `b` runs first (D64). They are as strong as required edges.
pub fn order_patches_full(
    patches: &[(Patch, Priority, PathBuf)],
    rules: &[OrderRule],
    compat_rules: &[(OrderRule, String)],
    required: &[(usize, usize, u32)],
    requires: &[(usize, usize)],
) -> OrderResult {
    let n = patches.len();
    let key = |i: usize| (patches.get(i).map_or(Priority::MAX, |p| p.1), i);
    let name_of = |i: usize| {
        patches
            .get(i)
            .map_or_else(|| format!("#{i}"), |p| patch_name(&p.0, i))
    };
    let mut result = OrderResult::default();
    let mut unknown = BTreeSet::new();
    let mut lookup = |name: &str| {
        let hits = matching_patches(name, patches);
        if hits.is_empty() {
            unknown.insert(name.trim().trim_start_matches(EXACT).to_string());
        }
        hits
    };

    // (from, to) -> strongest strength; `from` runs before `to`.
    let mut edges: BTreeMap<(usize, usize), Strength> = BTreeMap::new();
    // The same, only for rules that named `to` itself (D54).
    let mut explicit: BTreeMap<(usize, usize), Strength> = BTreeMap::new();
    let add = |edges: &mut BTreeMap<(usize, usize), Strength>,
               explicit: &mut BTreeMap<(usize, usize), Strength>,
               from: usize,
               to: usize,
               s: Strength,
               named: bool| {
        if from != to {
            let e = edges.entry((from, to)).or_insert(s);
            *e = (*e).max(s);
            if named {
                let e = explicit.entry((from, to)).or_insert(s);
                *e = (*e).max(s);
            }
        }
    };
    for (i, (p, _, _)) in patches.iter().enumerate() {
        for name in author_before(p) {
            let named = names_itself(name);
            for j in lookup(name) {
                add(&mut edges, &mut explicit, i, j, Strength::Author, named);
            }
        }
        // The list is on `i`, so it names `i` itself.
        for name in author_after(p) {
            for j in lookup(name) {
                add(&mut edges, &mut explicit, j, i, Strength::Author, true);
            }
        }
    }
    // The compat mod of each compat edge, for reports.
    let mut compat_of: BTreeMap<(usize, usize), String> = BTreeMap::new();
    let ruled = compat_rules
        .iter()
        .map(|(r, m)| (r, Strength::Compat, Some(m)))
        .chain(rules.iter().map(|r| (r, Strength::Profile, None)));
    let is_copy = |i: usize| {
        patches
            .get(i)
            .is_some_and(|(p, _, _)| p.extra().adjusted_copy)
    };
    for (rule, strength, compat) in ruled {
        // A compat rule never names an adjusted copy, `mod:*` included: the copy belongs to
        // the profile, whichever mod it runs as (D58).
        let mut find = |name: &str| -> Vec<usize> {
            let mut hits = lookup(name);
            if compat.is_some() {
                hits.retain(|&i| !is_copy(i));
            }
            hits
        };
        let subjects = find(&rule.patch);
        let subject_named = names_itself(&rule.patch);
        let mut pairs = Vec::new();
        for name in &rule.before {
            let named = names_itself(name);
            for j in find(name) {
                pairs.extend(subjects.iter().map(|&i| (i, j, named)));
            }
        }
        for name in &rule.after {
            for j in find(name) {
                pairs.extend(subjects.iter().map(|&i| (j, i, subject_named)));
            }
        }
        for (from, to, named) in pairs {
            add(&mut edges, &mut explicit, from, to, strength, named);
            if let Some(m) = compat {
                compat_of.entry((from, to)).or_insert_with(|| m.clone());
            }
        }
    }
    // Required edges, with the rungs behind each.
    let mut rungs_of: BTreeMap<(usize, usize), Vec<(usize, u32)>> = BTreeMap::new();
    for &(b, a, rung) in required {
        if b == a || b >= n || a >= n {
            continue;
        }
        add(&mut edges, &mut explicit, b, a, Strength::Required, true);
        rungs_of.entry((b, a)).or_default().push((a, rung));
    }
    result.implicit_edges = rungs_of.keys().copied().collect();
    // Requirement edges: the list is on `a`, so it names `a` itself.
    let mut needs: BTreeSet<(usize, usize)> = BTreeSet::new();
    for &(b, a) in requires {
        if b == a || b >= n || a >= n {
            continue;
        }
        add(&mut edges, &mut explicit, b, a, Strength::Required, true);
        needs.insert((b, a));
    }
    result.requires_edges = needs.iter().copied().collect();
    result.unknown_ids = unknown.into_iter().collect();
    for name in &result.unknown_ids {
        result.messages.push((
            log::Level::Warn,
            format!("Patch order rule names '{name}', which matches no loaded patch; ignored."),
        ));
    }

    // An edge is dropped when the reverse edge is strictly stronger.
    let contradicted: Vec<((usize, usize), Strength, Strength)> = edges
        .iter()
        .filter_map(|(&(a, b), &s)| match edges.get(&(b, a)) {
            Some(&r) if r > s => Some(((a, b), s, r)),
            _ => None,
        })
        .collect();
    for ((a, b), s, r) in contradicted {
        edges.remove(&(a, b));
        let (na, nb) = (name_of(a), name_of(b));
        // `b` runs before `a` because `a` is anchored on `b`, or `a` requires `b`.
        let why = if !rungs_of.contains_key(&(b, a)) && needs.contains(&(b, a)) {
            format!("{na} requires {nb}")
        } else {
            format!("{na} is anchored on {nb}")
        };
        let text = match (s, r) {
            (_, Strength::Required) if s == Strength::Profile => {
                format!("profile rule {na} before {nb} ignored: {why}")
            }
            (_, Strength::Required) => format!("{na} before {nb} ignored: {why}"),
            (_, Strength::Compat) => {
                let by = compat_of.get(&(b, a)).cloned().unwrap_or_default();
                format!("{na} before {nb} replaced by {by}")
            }
            _ => format!("{na} before {nb}"),
        };
        result.overridden_rules.push(text);
    }

    // Kahn's algorithm, always taking the ready patch with the smallest tie-break key.
    // Edges only name loaded patches, so every index below is in range. A miss is the
    // impossible state that `place_rest` handles.
    let mut indeg = vec![0usize; n];
    for &(_, to) in edges.keys() {
        if let Some(d) = indeg.get_mut(to) {
            *d += 1;
        }
    }
    let mut placed = vec![false; n];
    let mut ready: BinaryHeap<Reverse<(Priority, usize)>> = (0..n)
        .filter(|&i| indeg.get(i) == Some(&0))
        .map(|i| Reverse(key(i)))
        .collect();
    while result.order.len() < n {
        if let Some(Reverse((_, i))) = ready.pop() {
            match placed.get_mut(i) {
                Some(p) if !*p => *p = true,
                // Out of range, or ready twice.
                _ => {
                    place_rest(&mut result, n, key);
                    break;
                }
            }
            result.order.push(i);
            let outs: Vec<usize> = edges
                .range((i, 0)..(i + 1, 0))
                .map(|(&(_, to), _)| to)
                .collect();
            for to in outs {
                if let Some(d) = indeg.get_mut(to) {
                    *d = d.saturating_sub(1);
                    if *d == 0 {
                        ready.push(Reverse(key(to)));
                    }
                }
            }
            continue;
        }
        // Everything left waits on something left: find a cycle and drop its weakest edge.
        let Some(mut cycle) = find_cycle(&edges, &placed, key) else {
            place_rest(&mut result, n, key);
            break;
        };
        // Report it from the patch that would normally run first.
        if let Some((first, _)) = cycle.iter().enumerate().min_by_key(|&(_, &c)| key(c)) {
            cycle.rotate_left(first);
        }
        let weakest = cycle_edges(&cycle)
            .filter_map(|(from, to)| edges.get(&(from, to)).map(|&s| (from, to, s)))
            .min_by_key(|&(from, _, s)| (s, key(from).0, name_of(from)));
        let Some((wf, wt, ws)) = weakest else {
            place_rest(&mut result, n, key);
            break;
        };
        let weakest = (wf, wt);
        edges.remove(&weakest);
        if ws == Strength::Required {
            // Only required edges were left: that rung is suspended and A falls back.
            if let Some(rungs) = rungs_of.get(&weakest) {
                result.suspended_rungs.extend(rungs.iter().copied());
            }
        }
        if let Some(d) = indeg.get_mut(weakest.1) {
            *d = d.saturating_sub(1);
            if *d == 0 {
                ready.push(Reverse(key(weakest.1)));
            }
        }
        let names: Vec<String> = cycle.iter().map(|&i| name_of(i)).collect();
        result.messages.push((
            log::Level::Warn,
            format!(
                "Patch order cycle: {}. Dropped the rule that {} runs before {}.",
                names.join(" -> "),
                name_of(weakest.0),
                name_of(weakest.1)
            ),
        ));
        result.cycles.push(names);
    }
    // A dropped edge is gone from `edges`, so a copy it named stays with the first copies.
    result.edges = edges
        .keys()
        .map(|&(from, to)| OrderEdge {
            from,
            to,
            explicit: explicit.get(&(from, to)).copied(),
        })
        .collect();
    result
}

fn is_pattern_or_regex(patches: &[(Patch, Priority, PathBuf)], i: usize) -> bool {
    matches!(
        patches.get(i),
        Some((Patch::Pattern(_) | Patch::Regex(_), _, _))
    )
}

/// Per patch: for a copy, the patterns and regexes that explicit edges put before it, directly
/// or through other explicit edges, by patch index (D54). Empty for every other patch. A
/// `mod_id:*` name and the priority tie-break never count.
pub fn copy_preds(
    patches: &[(Patch, Priority, PathBuf)],
    edges: &[OrderEdge],
) -> Vec<Vec<CopyPred>> {
    let mut into: Vec<Vec<(usize, Strength)>> = vec![Vec::new(); patches.len()];
    for e in edges {
        if let (Some(s), Some(v)) = (e.explicit, into.get_mut(e.to)) {
            v.push((e.from, s));
        }
    }
    let none: &[(usize, Strength)] = &[];
    patches
        .iter()
        .enumerate()
        .map(|(c, (p, _, _))| {
            if !matches!(p, Patch::Copy(_)) {
                return Vec::new();
            }
            let mut direct = into.get(c).map_or(none, Vec::as_slice).to_vec();
            direct.sort_by_key(|&(from, s)| (Reverse(s), from));
            // Each patch found keeps the edge into the copy it was first reached through.
            let mut via: BTreeMap<usize, Strength> = BTreeMap::new();
            let mut queue = std::collections::VecDeque::new();
            for (from, s) in direct {
                if from != c && !via.contains_key(&from) {
                    via.insert(from, s);
                    queue.push_back((from, s));
                }
            }
            while let Some((x, s)) = queue.pop_front() {
                for &(from, _) in into.get(x).map_or(none, Vec::as_slice) {
                    if from != c && !via.contains_key(&from) {
                        via.insert(from, s);
                        queue.push_back((from, s));
                    }
                }
            }
            via.into_iter()
                .filter(|&(i, _)| is_pattern_or_regex(patches, i))
                .map(|(patch, via)| CopyPred { patch, via })
                .collect()
        })
        .collect()
}

/// The pattern pass of one buffer (D54): the patterns and regexes of `order`, plus the copies
/// in `ordered`. Each ordered copy runs as early as the edges allow: before it only what an
/// edge path puts before it, then the copy, then the rest, so a pattern no edge orders before
/// the copy sees its text. Without ordered copies this is `order`'s patterns and regexes.
pub fn pattern_pass(
    patches: &[(Patch, Priority, PathBuf)],
    order: &OrderResult,
    ordered: &[usize],
) -> Vec<usize> {
    let is_pattern = |i: usize| is_pattern_or_regex(patches, i);
    if ordered.is_empty() {
        return order.order.iter().copied().filter(|&i| is_pattern(i)).collect();
    }
    let n = patches.len();
    let mut pos = vec![usize::MAX; n];
    for (k, &i) in order.order.iter().enumerate() {
        if let Some(p) = pos.get_mut(i) {
            *p = k;
        }
    }
    let mut is_copy = vec![false; n];
    for &c in ordered {
        if let Some(x) = is_copy.get_mut(c) {
            *x = true;
        }
    }
    let mut outs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut ins: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for e in &order.edges {
        if e.from >= n || e.to >= n {
            continue;
        }
        if let (Some(o), Some(i), Some(d)) = (
            outs.get_mut(e.from),
            ins.get_mut(e.to),
            indeg.get_mut(e.to),
        ) {
            o.push(e.to);
            i.push(e.from);
            *d += 1;
        }
    }
    // What each ordered copy waits for, and how many waiting copies want each patch.
    let none: &[usize] = &[];
    let mut wanted = vec![0usize; n];
    let mut ancestors: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &c in ordered {
        let mut seen = BTreeSet::new();
        let mut stack = vec![c];
        while let Some(x) = stack.pop() {
            for &from in ins.get(x).map_or(none, Vec::as_slice) {
                if from != c && seen.insert(from) {
                    stack.push(from);
                }
            }
        }
        for &a in &seen {
            if let Some(w) = wanted.get_mut(a) {
                *w += 1;
            }
        }
        ancestors.insert(c, seen.into_iter().collect());
    }

    // Kahn's algorithm over every patch. Patches outside the pass (the first copies, modules)
    // take no turn but still pass their edges on.
    let flag = |v: &[bool], i: usize| v.get(i).copied().unwrap_or(false);
    let in_pass = |i: usize| is_pattern(i) || flag(&is_copy, i);
    let key = |i: usize| (pos.get(i).copied().unwrap_or(usize::MAX), i);
    let mut copies: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut wanted_ready: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut rest: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut passive: Vec<usize> = Vec::new();
    let mut done = vec![false; n];
    let mut out = Vec::new();
    let ready = |i: usize,
                 wanted: &[usize],
                 copies: &mut BTreeSet<(usize, usize)>,
                 wanted_ready: &mut BTreeSet<(usize, usize)>,
                 rest: &mut BTreeSet<(usize, usize)>,
                 passive: &mut Vec<usize>| {
        if !in_pass(i) {
            passive.push(i);
        } else if flag(&is_copy, i) {
            copies.insert(key(i));
        } else if wanted.get(i).copied().unwrap_or(0) > 0 {
            wanted_ready.insert(key(i));
        } else {
            rest.insert(key(i));
        }
    };
    for &i in &order.order {
        if indeg.get(i) == Some(&0) {
            ready(i, &wanted, &mut copies, &mut wanted_ready, &mut rest, &mut passive);
        }
    }
    loop {
        // Copies first, then what a waiting copy needs, then the rest, each in order.
        let (next, takes_turn) = if let Some(i) = passive.pop() {
            (i, false)
        } else if let Some((_, i)) = copies
            .pop_first()
            .or_else(|| wanted_ready.pop_first())
            .or_else(|| rest.pop_first())
        {
            (i, true)
        } else {
            break;
        };
        match done.get_mut(next) {
            Some(d) if !*d => *d = true,
            _ => continue,
        }
        if takes_turn {
            out.push(next);
        }
        if let Some(anc) = ancestors.get(&next) {
            for &a in anc {
                if let Some(w) = wanted.get_mut(a) {
                    *w = w.saturating_sub(1);
                    if *w == 0 && wanted_ready.remove(&key(a)) {
                        rest.insert(key(a));
                    }
                }
            }
        }
        for &to in outs.get(next).map_or(none, Vec::as_slice) {
            if let Some(d) = indeg.get_mut(to) {
                *d = d.saturating_sub(1);
                if *d == 0 {
                    ready(to, &wanted, &mut copies, &mut wanted_ready, &mut rest, &mut passive);
                }
            }
        }
    }
    // Only a degraded order (a cycle left in the edges) gets here: the rest in order.
    for &i in &order.order {
        if in_pass(i) && !flag(&done, i) {
            out.push(i);
        }
    }
    out
}

/// The impossible-state exit: appends every patch not yet placed, by its tie-break key, and
/// marks the result degraded.
fn place_rest(result: &mut OrderResult, n: usize, key: impl Fn(usize) -> (Priority, usize)) {
    let done: BTreeSet<usize> = result.order.iter().copied().collect();
    let mut rest: Vec<usize> = (0..n).filter(|i| !done.contains(i)).collect();
    rest.sort_by_key(|&i| key(i));
    result.messages.push((
        log::Level::Error,
        format!(
            "Patch order reached an impossible state; {} patch(es) placed by priority instead.",
            rest.len()
        ),
    ));
    result.order.extend(rest);
    result.degraded = true;
}

/// A cycle among the unplaced patches, in edge order (each runs before the next, the last
/// before the first). Walks backwards along incoming edges from the smallest key, always taking
/// the smallest-key predecessor, so the result is deterministic.
/// `None` only in a state that should be impossible: nothing unplaced, or an unplaced patch
/// with no unplaced predecessor.
fn find_cycle(
    edges: &BTreeMap<(usize, usize), Strength>,
    placed: &[bool],
    key: impl Fn(usize) -> (Priority, usize),
) -> Option<Vec<usize>> {
    let unplaced = |i: usize| placed.get(i) == Some(&false);
    let start = (0..placed.len())
        .filter(|&i| unplaced(i))
        .min_by_key(|&i| key(i))?;
    let mut path = vec![start];
    let mut seen = BTreeMap::from([(start, 0usize)]);
    let mut cur = start;
    // Each step adds a new patch to `path`, so a cycle shows within `placed.len()` steps.
    for _ in 0..=placed.len() {
        let pred = edges
            .keys()
            .filter(|&&(from, to)| to == cur && unplaced(from))
            .map(|&(from, _)| from)
            .min_by_key(|&i| key(i))?;
        if let Some(&at) = seen.get(&pred) {
            let mut cycle: Vec<usize> = path.get(at..)?.to_vec();
            cycle.reverse();
            return Some(cycle);
        }
        seen.insert(pred, path.len());
        path.push(pred);
        cur = pred;
    }
    None
}

fn cycle_edges(cycle: &[usize]) -> impl Iterator<Item = (usize, usize)> + '_ {
    cycle
        .iter()
        .zip(cycle.iter().cycle().skip(1))
        .map(|(&a, &b)| (a, b))
}

#[cfg(test)]
#[path = "order_tests.rs"]
mod tests;
