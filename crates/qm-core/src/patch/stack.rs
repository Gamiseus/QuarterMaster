//! The slot rule for code that several patches add at one line (plan 2e §5.3).
//!
//! A chain is the list of top-level items stacked on one side of an anchor line, top to
//! bottom (`LineMap::chain`). A new insert goes into a slot `p` in `0..=k`: above item `p`,
//! or below all of them at `k`. Legacy is today's rule: after a line the newest goes on top,
//! before a line it goes right above the line. Dependencies moves it the least needed to put
//! it below the mods it depends on and above the mods that depend on it. LoadOrder puts it
//! last, so the code reads in application order on both sides. Same-mod items always keep
//! the legacy order (D5). A copy's legacy slot is the far end of the chain: a prepend on top
//! of the code before a line, an append below the code after it (D53). Stack rules (D55) act
//! after all of that, in every mode, Legacy included: a profile rule first, then compat rules
//! (D56). An author hint (`stack = "first"` or `"last"`, D57) acts between the dependency guard
//! and the rules, also in every mode: the block goes to the top or the bottom of what the guard
//! allows, and stays there for later inserts.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::borrow::Cow;

use crop::Rope;
use serde::{Deserialize, Serialize};

use crate::handshake::{StackMode, StackMove, StackReason};
use crate::patch::anchor::SiteKey;
use crate::patch::fingerprint::site_context;
use crate::patch::linemap::{Side, NO_INDEX, NO_LINE};
use crate::patch::ApplyCtx;

/// Where a kind of insert goes when stacking does not move it (D53).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacySlot {
    /// Nearest the line: on top of the code after it, right above the line before it
    /// (patterns and regex).
    Near,
    /// Farthest from the line: below the code after it, on top of the code before it (copies).
    Far,
}

/// An author's `stack` key (D57): the block goes to the top (`first`, its code runs first) or
/// the bottom (`last`) of the code at its line, as far as the dependency guard allows.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StackHint {
    First,
    Last,
}

/// The author hints a slot reads (D57): this insert's own, and each chain item's (top to
/// bottom, as the owners).
#[derive(Clone, Copy, Debug, Default)]
pub struct Hints<'a> {
    pub me: Option<StackHint>,
    pub items: &'a [Option<StackHint>],
}

/// Why a slot differs from the legacy one. Item indexes are into the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotReason {
    /// Placed below item `i`, a mod this one depends on.
    BelowDependency(usize),
    /// Placed above item `i`, a mod that depends on this one.
    AboveDependent(usize),
    LoadOrder,
    /// No slot is below every dependency and above every dependent; the dependency wins.
    Unsatisfiable { below: usize, above: usize },
    /// Placed, or kept, by rule `i` of the rules given (D55).
    Rule(usize),
    /// Placed by an author hint (D57): this insert's own (`item` none) or item `item`'s.
    Hint {
        item: Option<usize>,
        hint: StackHint,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub slot: usize,
    pub legacy: usize,
    /// `None` when the slot is the legacy one (Unsatisfiable is always given).
    pub reason: Option<SlotReason>,
}

/// Who made a stack rule. A profile rule beats a compat one (D56).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleSource {
    Profile { fix_id: String },
    Compat { mod_id: String },
}

impl RuleSource {
    fn rank(&self) -> u8 {
        match self {
            RuleSource::Profile { .. } => 0,
            RuleSource::Compat { .. } => 1,
        }
    }

    fn reason(&self) -> StackReason {
        match self {
            RuleSource::Profile { fix_id } => StackReason::UserRule {
                fix_id: fix_id.clone(),
            },
            RuleSource::Compat { mod_id } => StackReason::CompatRule {
                mod_id: mod_id.clone(),
            },
        }
    }

    fn label(&self) -> String {
        match self {
            RuleSource::Profile { fix_id } => format!("your fix {fix_id}"),
            RuleSource::Compat { mod_id } => format!("{mod_id}'s rule"),
        }
    }
}

/// A stack rule on one patch as the plan layers give it (D55): at line `site` (any line when
/// none) on `side` (either side when none), this patch's block goes below `below`'s block or
/// above `above`'s. Names are order rule names (`order::name_matches`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StackRuleSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<SiteKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub below: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub above: Option<String>,
    pub source: RuleSource,
}

/// A stack rule bound in `prepare`, held by both patches it names, so it holds whichever runs
/// first (D55): this patch goes below patch `other` (`me_below`) or above it.
#[derive(Clone, Debug, PartialEq)]
pub struct StackRuleBound {
    pub other: u32,
    pub me_below: bool,
    pub site: Option<SiteKey>,
    pub side: Option<Side>,
    pub source: RuleSource,
}

/// The key of 0-based line `u` of a buffer for a stack rule: the context of the line alone,
/// and its place (1-based) among the earlier lines with the same text and context.
/// QuarterMaster's lines are skipped, as for site keys.
pub fn line_key(lines: &[&str], is_builtin: impl Fn(usize) -> bool, u: usize) -> SiteKey {
    let context = site_context(lines, &is_builtin, u, 1);
    let text = lines.get(u).map(|l| l.trim());
    let earlier = (0..u)
        .filter(|&i| !is_builtin(i) && lines.get(i).map(|l| l.trim()) == text)
        .filter(|&i| site_context(lines, &is_builtin, i, 1) == context)
        .count();
    SiteKey {
        context,
        ordinal: u32::try_from(earlier + 1).ok(),
    }
}

/// Whether `key` names line `u` (see `line_key`); with no ordinal, any line with its context.
pub fn line_matches(
    key: &SiteKey,
    lines: &[&str],
    is_builtin: impl Fn(usize) -> bool,
    u: usize,
) -> bool {
    if site_context(lines, &is_builtin, u, 1) != key.context {
        return false;
    }
    key.ordinal
        .is_none_or(|n| line_key(lines, &is_builtin, u).ordinal == Some(n))
}

/// One stack rule as the slot rule reads it: this insert goes below chain item `item`
/// (`below`) or above it. `rank` 0 is the strongest (a profile rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotRule {
    pub item: usize,
    pub below: bool,
    pub rank: u8,
}

/// `stack_slot` with stack rules (D55, D56). The rules act last and win over the mode rule
/// and the dependency guard; the slot moves the least it must. Of two rules that cannot both
/// hold, the stronger (lower rank, then earlier) stays and the other is returned as
/// overridden (indexes into `rules`). A rule that holds names the reason, `Rule(i)`.
pub fn stack_slot_ruled(
    mode: StackMode,
    side: Side,
    kind: LegacySlot,
    me: u32,
    owners: &[u32],
    depends_on: impl Fn(u32, u32) -> bool,
    rules: &[SlotRule],
) -> (Slot, Vec<usize>) {
    stack_slot_with(
        mode,
        side,
        kind,
        me,
        owners,
        depends_on,
        Hints::default(),
        rules,
    )
}

/// The full slot rule (D56): the mode rule, the dependency guard, the author hints
/// (`stack_slot_hinted`), then the stack rules (`stack_slot_ruled`).
#[allow(clippy::too_many_arguments)]
pub fn stack_slot_with(
    mode: StackMode,
    side: Side,
    kind: LegacySlot,
    me: u32,
    owners: &[u32],
    depends_on: impl Fn(u32, u32) -> bool,
    hints: Hints,
    rules: &[SlotRule],
) -> (Slot, Vec<usize>) {
    let base = stack_slot_hinted(mode, side, kind, me, owners, depends_on, hints);
    let k = owners.len();
    let mut order: Vec<usize> = (0..rules.len()).collect();
    order.sort_by_key(|&i| rules.get(i).map_or(u8::MAX, |r| r.rank));
    let (mut lo, mut hi) = (0usize, k);
    let mut held: Vec<(usize, usize, usize)> = Vec::new();
    let mut overridden = Vec::new();
    for i in order {
        let Some(r) = rules.get(i) else { continue };
        let (rlo, rhi) = if r.below {
            (r.item.saturating_add(1), k)
        } else {
            (0, r.item)
        };
        let (nlo, nhi) = (lo.max(rlo), hi.min(rhi));
        if nlo <= nhi && r.item < k {
            (lo, hi) = (nlo, nhi);
            held.push((i, rlo, rhi));
        } else {
            overridden.push(i);
        }
    }
    let Some(&(first, _, _)) = held.first() else {
        return (base, overridden);
    };
    let slot = base.slot.clamp(lo, hi);
    let reason = if slot != base.slot {
        // The rule that bounds the slot.
        let bound = held
            .iter()
            .find(|&&(_, rlo, rhi)| (slot == lo && rlo == lo) || (slot == hi && rhi == hi))
            .map_or(first, |&(i, _, _)| i);
        Some(SlotReason::Rule(bound))
    } else {
        base.reason.or(Some(SlotReason::Rule(first)))
    };
    (
        Slot {
            slot,
            legacy: base.legacy,
            reason,
        },
        overridden,
    )
}

/// `stack_slot` with author hints (D57). The dependency guard's range stays (a hint never
/// lifts a block above a mod it depends on); inside it, the slot is never above another mod's
/// `first` item nor below another mod's `last` item, nor across this mod's own items (D5).
/// Then this insert's own hint takes the top (`first`) or the bottom (`last`) of what is left;
/// with no hint of its own, the slot moves the least it must. A bound that would leave no slot
/// is dropped. When nothing is hinted, or no slot fits the dependencies, it is `stack_slot`.
pub fn stack_slot_hinted(
    mode: StackMode,
    side: Side,
    kind: LegacySlot,
    me: u32,
    owners: &[u32],
    depends_on: impl Fn(u32, u32) -> bool,
    hints: Hints,
) -> Slot {
    let base = stack_slot(mode, side, kind, me, owners, &depends_on);
    let k = owners.len();
    let own = |o: u32| o == me && me != NO_INDEX;
    let hinted = |i: usize| -> Option<StackHint> {
        let o = *owners.get(i)?;
        if own(o) {
            return None;
        }
        hints.items.get(i).copied().flatten()
    };
    if hints.me.is_none() && !(0..k).any(|i| hinted(i).is_some()) {
        return base;
    }
    if matches!(base.reason, Some(SlotReason::Unsatisfiable { .. })) {
        return base;
    }
    // The dependency guard's range; the hint never leaves it (D56).
    let (mut lo, mut hi) = match mode {
        StackMode::Dependencies => dependency_range(me, owners, &depends_on),
        _ => (0, k),
    };
    let (mut lo_by, mut hi_by) = (None, None);
    for i in 0..k {
        match hinted(i) {
            Some(StackHint::First) if i + 1 > lo && i < hi => {
                lo = i + 1;
                lo_by = Some(i);
            }
            Some(StackHint::Last) if i < hi && i >= lo => {
                hi = i;
                hi_by = Some(i);
            }
            _ => {}
        }
    }
    // This mod's own items keep their legacy order (D5).
    if legacy_on_top(side, kind) {
        if let Some(f) = owners.iter().position(|&o| own(o)) {
            if f >= lo {
                hi = hi.min(f);
            }
        }
    } else if let Some(l) = owners.iter().rposition(|&o| own(o)) {
        if l < hi {
            lo = lo.max(l + 1);
        }
    }
    let slot = match hints.me {
        Some(StackHint::First) => lo,
        Some(StackHint::Last) => hi,
        None => base.slot.clamp(lo, hi),
    };
    if slot == base.slot {
        return base;
    }
    let reason = match hints.me {
        Some(hint) => Some(SlotReason::Hint { item: None, hint }),
        None => {
            let by = if slot > base.slot { lo_by } else { hi_by };
            match by.and_then(|i| hinted(i).map(|h| (i, h))) {
                Some((i, hint)) => Some(SlotReason::Hint {
                    item: Some(i),
                    hint,
                }),
                None => base.reason,
            }
        }
    };
    Slot {
        slot,
        legacy: base.legacy,
        reason: if slot == base.legacy { None } else { reason },
    }
}

/// The slots the dependency guard allows (Dependencies): below every mod this one depends on
/// and above every mod that depends on it, not counting this mod's own items. Only called when
/// such a slot exists.
fn dependency_range(
    me: u32,
    owners: &[u32],
    depends_on: impl Fn(u32, u32) -> bool,
) -> (usize, usize) {
    let own = |o: u32| o == me && me != NO_INDEX;
    let lo = owners
        .iter()
        .enumerate()
        .rev()
        .find(|&(_, &o)| !own(o) && depends_on(me, o))
        .map_or(0, |(i, _)| i + 1);
    let hi = owners
        .iter()
        .enumerate()
        .find(|&(_, &o)| !own(o) && depends_on(o, me))
        .map_or(owners.len(), |(i, _)| i);
    (lo, hi)
}

/// Whether `kind`'s legacy slot on `side` is the top of the chain.
fn legacy_on_top(side: Side, kind: LegacySlot) -> bool {
    matches!(
        (side, kind),
        (Side::After, LegacySlot::Near) | (Side::Before, LegacySlot::Far)
    )
}

/// The legacy slot of an insert of `kind` in a chain of `k` items.
pub fn legacy_slot(side: Side, k: usize, kind: LegacySlot) -> usize {
    if legacy_on_top(side, kind) {
        0
    } else {
        k
    }
}

/// The slot for a new item of mod `me` in a chain whose items belong to `owners` (mod
/// indexes, top to bottom; `NO_INDEX` for an unknown mod). `kind` is where the insert goes
/// today. `depends_on(a, b)` says whether mod `a` depends on mod `b`. Pure; never panics.
pub fn stack_slot(
    mode: StackMode,
    side: Side,
    kind: LegacySlot,
    me: u32,
    owners: &[u32],
    depends_on: impl Fn(u32, u32) -> bool,
) -> Slot {
    let k = owners.len();
    // An unknown mod (`NO_INDEX`) is never this patch's own.
    let own = |o: u32| o == me && me != NO_INDEX;
    let legacy = legacy_slot(side, k, kind);
    let at = |slot: usize, reason: Option<SlotReason>| Slot {
        slot,
        legacy,
        reason: if slot == legacy { None } else { reason },
    };
    match mode {
        StackMode::Legacy => at(legacy, None),
        StackMode::LoadOrder => {
            // Last, but never across this mod's own items (D5): when the legacy slot is the
            // top, a mod's newer items sit above its older ones.
            let own_first = owners.iter().position(|&o| own(o));
            let slot = if legacy_on_top(side, kind) {
                own_first.unwrap_or(k)
            } else {
                k
            };
            at(slot, Some(SlotReason::LoadOrder))
        }
        StackMode::Dependencies => {
            let lo_item = owners
                .iter()
                .enumerate()
                .rev()
                .find(|&(_, &o)| !own(o) && depends_on(me, o))
                .map(|(i, _)| i);
            let hi_item = owners
                .iter()
                .enumerate()
                .find(|&(_, &o)| !own(o) && depends_on(o, me))
                .map(|(i, _)| i);
            let lo = lo_item.map_or(0, |i| i + 1);
            let hi = hi_item.unwrap_or(k);
            if lo <= hi {
                let slot = legacy.clamp(lo, hi);
                let reason = if slot > legacy {
                    lo_item.map(SlotReason::BelowDependency)
                } else {
                    hi_item.map(SlotReason::AboveDependent)
                };
                at(slot, reason)
            } else {
                Slot {
                    slot: lo,
                    legacy,
                    reason: Some(SlotReason::Unsatisfiable {
                        below: lo.saturating_sub(1),
                        above: hi,
                    }),
                }
            }
        }
    }
}

/// The text of whole lines that stacking moved to byte `at`. At the end of a text whose last
/// line has no newline, the lines start with that newline instead of joining the line.
pub(crate) fn stacked_text<'a>(rope: &Rope, at: usize, lines: &'a str) -> Cow<'a, str> {
    let len = rope.byte_len();
    if at == len && len > 0 && rope.byte(len - 1) != b'\n' {
        Cow::Owned(format!("\n{}", lines.strip_suffix('\n').unwrap_or(lines)))
    } else {
        Cow::Borrowed(lines)
    }
}

/// Where stacking puts one insert: the pre-patch line, when it is not the legacy one, and
/// what to report.
pub(crate) struct Placement {
    pub line: Option<usize>,
    pub report: StackMove,
}

/// The slot of an insert on `side` of pre-patch line `anchor` (§5.3), or `None` for the
/// legacy slot with nothing to report. `kind` is where the insert goes today. `windows` are
/// the patch's places before it ran, top to bottom, and `j` is this insert's place (a copy has
/// none). A slot past the next place (after) or at or above the previous one (before) is
/// kept at the legacy slot (`GuardClamped`), so the edits stay in order and never land
/// inside a place.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_in_stack(
    ctx: &ApplyCtx,
    side: Side,
    anchor: usize,
    windows: &[(usize, usize)],
    j: usize,
    site: u32,
    kind: LegacySlot,
) -> Option<Placement> {
    let st = ctx.stack.as_ref()?;
    // In Legacy only a stack rule or an author hint moves code (D55, D57).
    if st.mode == StackMode::Legacy
        && st.rules.is_empty()
        && st.hint.is_none()
        && !ctx.lines.has_hinted_blocks()
    {
        return None;
    }
    let chain = ctx.lines.chain(anchor, side);
    if chain.is_empty() {
        return None;
    }
    let owners: Vec<u32> = chain.iter().map(|c| c.mod_ix).collect();
    let item_hints: Vec<Option<StackHint>> = chain.iter().map(|c| c.hint).collect();
    let (rules, sources) = rules_here(ctx, st.rules, anchor, side, &chain);
    let (s, overridden) = stack_slot_with(
        st.mode,
        side,
        kind,
        st.mod_ix,
        &owners,
        |a, b| st.depends_on(a, b),
        Hints {
            me: st.hint,
            items: &item_hints,
        },
        &rules,
    );
    let line_of = |slot: usize| -> usize {
        match side {
            Side::After => match slot.checked_sub(1).and_then(|i| chain.get(i)) {
                Some(c) => c.last + 1,
                None => anchor + 1,
            },
            Side::Before => chain.get(slot).map_or(anchor, |c| c.first),
        }
    };
    let item_names = |i: usize| -> Option<(String, String)> {
        let c = chain.get(i)?;
        let b = ctx.lines.block(c.block)?;
        Some((st.names)(b.patch_ix))
    };
    let mod_of = |i: usize| item_names(i).map(|n| n.1).unwrap_or_default();
    let overridden: Vec<String> = overridden
        .iter()
        .filter_map(|&i| {
            let (rule, src) = (rules.get(i)?, sources.get(i)?);
            let other = item_names(rule.item).map(|n| n.0).unwrap_or_default();
            let place = if rule.below { "below" } else { "above" };
            Some(format!(
                "{} to put this block {place} {other}'s is overridden by a stronger rule",
                upper_first(&src.label())
            ))
        })
        .collect();
    for o in &overridden {
        log::warn!("Stacking at line {}: {o}", anchor + 1);
    }
    // A rule is overridden only by a rule that holds, so there is a reason then too.
    let reason = s.reason?;
    let mut slot = s.slot;
    let mut line = line_of(slot);
    let crosses = match side {
        Side::After => windows.get(j + 1).is_some_and(|&(next, _)| line > next),
        Side::Before => j
            .checked_sub(1)
            .and_then(|p| windows.get(p))
            .is_some_and(|&(prev, plen)| line < prev + plen.max(1)),
    };
    let reason = if crosses {
        slot = s.legacy;
        line = line_of(slot);
        StackReason::GuardClamped
    } else {
        match reason {
            SlotReason::BelowDependency(i) => StackReason::BelowDependency { of: mod_of(i) },
            SlotReason::AboveDependent(i) => StackReason::AboveDependent { of: mod_of(i) },
            SlotReason::LoadOrder => StackReason::LoadOrder,
            SlotReason::Unsatisfiable { below, above } => {
                let (below, above) = (mod_of(below), mod_of(above));
                log::warn!(
                    "Stacking: no place fits at line {} ({below} must be above, {above} below); keeping it below {below}",
                    anchor + 1
                );
                StackReason::Unsatisfiable { below, above }
            }
            SlotReason::Rule(i) => sources
                .get(i)
                .map_or(StackReason::GuardClamped, RuleSource::reason),
            SlotReason::Hint { item, hint } => StackReason::AuthorHint {
                hint,
                of: match item {
                    Some(i) => mod_of(i),
                    None => (st.names)(ctx.me).1,
                },
            },
        }
    };
    let to_u32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Some(Placement {
        line: (slot != s.legacy).then_some(line),
        report: StackMove {
            site,
            anchor_line: to_u32(anchor + 1),
            side,
            legacy_slot: to_u32(s.legacy),
            slot: to_u32(slot),
            reason,
            below: slot.checked_sub(1).and_then(item_names).map(|n| n.0),
            above: item_names(slot).map(|n| n.0),
            overridden,
        },
    })
}

/// The stack rules of this patch that act at `anchor` on `side`, as slot rules on `chain`,
/// with their sources: the other patch's block is in the chain, and the rule's line and side
/// are these (D55). A rule whose other patch has no block here does nothing.
fn rules_here(
    ctx: &ApplyCtx,
    rules: &[StackRuleBound],
    anchor: usize,
    side: Side,
    chain: &[crate::patch::linemap::ChainItem],
) -> (Vec<SlotRule>, Vec<RuleSource>) {
    let mut out = Vec::new();
    let mut sources = Vec::new();
    if rules.is_empty() {
        return (out, sources);
    }
    // The game's line the anchor stands for, read once.
    let vanilla = ctx
        .lines
        .get(anchor)
        .map(|t| t.vanilla)
        .filter(|&v| v != NO_LINE)
        .map(|v| v as usize);
    let mut original: Option<Vec<&str>> = None;
    for r in rules {
        if r.side.is_some_and(|s| s != side) {
            continue;
        }
        let Some(item) = chain
            .iter()
            .position(|c| ctx.lines.block(c.block).is_some_and(|b| b.patch_ix == r.other))
        else {
            continue;
        };
        if let Some(key) = &r.site {
            let (Some(u), Some(orig)) = (vanilla, ctx.original) else {
                continue;
            };
            let lines = original.get_or_insert_with(|| orig.get());
            let builtin = |i: usize| ctx.lines.uid_is_builtin(u32::try_from(i).unwrap_or(u32::MAX));
            if !line_matches(key, lines, builtin, u) {
                continue;
            }
        }
        out.push(SlotRule {
            item,
            below: r.me_below,
            rank: r.source.rank(),
        });
        sources.push(r.source.clone());
    }
    (out, sources)
}

/// `s` with its first letter upper case.
fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests;
