use super::*;
use StackMode::{Dependencies, Legacy, LoadOrder};

const GCORE: u32 = 0;
const GMENU: u32 = 1;
const GSE: u32 = 2;
const TO: u32 = 3;
const GDUMPER: u32 = 4;
const OTHER: u32 = 5;

/// §2 D4 for the user's mods: everything depends on gcore, everything but gcore on gmenu.
fn deps(a: u32, b: u32) -> bool {
    match (a, b) {
        (GMENU, GCORE) => true,
        (GSE | TO | GDUMPER, GCORE | GMENU) => true,
        _ => false,
    }
}

struct Row {
    mode: StackMode,
    side: Side,
    me: u32,
    chain: &'static [u32],
    slot: usize,
    reason: Option<SlotReason>,
}

#[test]
fn stack_slot_table() {
    use SlotReason as R;
    let rows = [
        // §5.4 main.lua:16, after, Dependencies.
        Row { mode: Dependencies, side: Side::After, me: GCORE, chain: &[], slot: 0, reason: None },
        Row { mode: Dependencies, side: Side::After, me: GMENU, chain: &[GCORE], slot: 1, reason: Some(R::BelowDependency(0)) },
        Row { mode: Dependencies, side: Side::After, me: GSE, chain: &[GCORE, GMENU], slot: 2, reason: Some(R::BelowDependency(1)) },
        Row { mode: Dependencies, side: Side::After, me: TO, chain: &[GCORE, GMENU, GSE], slot: 2, reason: Some(R::BelowDependency(1)) },
        Row { mode: Dependencies, side: Side::After, me: GDUMPER, chain: &[GCORE, GMENU, TO, GSE], slot: 2, reason: Some(R::BelowDependency(1)) },
        // The same chain in Legacy and LoadOrder.
        Row { mode: Legacy, side: Side::After, me: GDUMPER, chain: &[GCORE, GMENU, TO, GSE], slot: 0, reason: None },
        Row { mode: LoadOrder, side: Side::After, me: GDUMPER, chain: &[GCORE, GMENU, TO, GSE], slot: 4, reason: Some(R::LoadOrder) },
        Row { mode: LoadOrder, side: Side::After, me: GCORE, chain: &[], slot: 0, reason: None },
        // Before: legacy is right above the line; the rule is the same.
        Row { mode: Legacy, side: Side::Before, me: TO, chain: &[GCORE], slot: 1, reason: None },
        Row { mode: Dependencies, side: Side::Before, me: TO, chain: &[GCORE], slot: 1, reason: None },
        Row { mode: LoadOrder, side: Side::Before, me: TO, chain: &[GCORE], slot: 1, reason: None },
        // Before-stack inversion: TO applied first, GCore goes above it.
        Row { mode: Dependencies, side: Side::Before, me: GCORE, chain: &[TO], slot: 0, reason: Some(R::AboveDependent(0)) },
        Row { mode: Legacy, side: Side::Before, me: GCORE, chain: &[TO], slot: 1, reason: None },
        Row { mode: LoadOrder, side: Side::Before, me: GCORE, chain: &[TO], slot: 1, reason: None },
        // After-stack inversion: GCore applied after TO goes on top, as in Legacy.
        Row { mode: Dependencies, side: Side::After, me: GCORE, chain: &[TO], slot: 0, reason: None },
        // Unrelated mods keep the legacy slot.
        Row { mode: Dependencies, side: Side::After, me: OTHER, chain: &[GCORE, TO], slot: 0, reason: None },
        Row { mode: Dependencies, side: Side::Before, me: OTHER, chain: &[GCORE, TO], slot: 2, reason: None },
        // The minimal move: below the last dependency, not below everything.
        Row { mode: Dependencies, side: Side::After, me: TO, chain: &[GCORE, OTHER, OTHER], slot: 1, reason: Some(R::BelowDependency(0)) },
        Row { mode: Dependencies, side: Side::Before, me: GCORE, chain: &[OTHER, TO, OTHER], slot: 1, reason: Some(R::AboveDependent(1)) },
        // Same-mod items keep the legacy order (D5); a dependency can still move across one.
        Row { mode: Dependencies, side: Side::After, me: GCORE, chain: &[GCORE], slot: 0, reason: None },
        Row { mode: Dependencies, side: Side::Before, me: GCORE, chain: &[GCORE], slot: 1, reason: None },
        Row { mode: LoadOrder, side: Side::After, me: GCORE, chain: &[GCORE], slot: 0, reason: None },
        Row { mode: LoadOrder, side: Side::After, me: GCORE, chain: &[TO, GCORE, GMENU], slot: 1, reason: Some(R::LoadOrder) },
        Row { mode: Dependencies, side: Side::After, me: TO, chain: &[TO, GCORE], slot: 2, reason: Some(R::BelowDependency(1)) },
        // A dependent above a dependency (a priority inversion): the dependency wins.
        Row { mode: Dependencies, side: Side::After, me: GMENU, chain: &[TO, GCORE], slot: 2, reason: Some(R::Unsatisfiable { below: 1, above: 0 }) },
    ];
    for (n, r) in rows.iter().enumerate() {
        let got = stack_slot(r.mode, r.side, LegacySlot::Near, r.me, r.chain, deps);
        assert_eq!(
            (got.slot, got.reason),
            (r.slot, r.reason),
            "row {n}: {:?} {:?} me {} chain {:?}",
            r.mode,
            r.side,
            r.me,
            r.chain
        );
        assert_eq!(
            got.legacy,
            legacy_slot(r.side, r.chain.len(), LegacySlot::Near),
            "row {n}"
        );
    }
}

/// D53: a copy goes to the far end of the chain today (a prepend on top of the code before
/// its line, an append below the code after it), and the mode rules move it from there.
#[test]
fn far_slot_table() {
    use SlotReason as R;
    let rows = [
        // Legacy: prepends newest on top, appends newest at the bottom.
        Row { mode: Legacy, side: Side::Before, me: TO, chain: &[GCORE], slot: 0, reason: None },
        Row { mode: Legacy, side: Side::After, me: TO, chain: &[GCORE], slot: 1, reason: None },
        // A copy that depends on the one before goes below it on both sides.
        Row { mode: Dependencies, side: Side::Before, me: TO, chain: &[GCORE], slot: 1, reason: Some(R::BelowDependency(0)) },
        Row { mode: Dependencies, side: Side::After, me: TO, chain: &[GCORE], slot: 1, reason: None },
        // The reverse dependency puts it above.
        Row { mode: Dependencies, side: Side::Before, me: GCORE, chain: &[TO], slot: 0, reason: None },
        Row { mode: Dependencies, side: Side::After, me: GCORE, chain: &[TO], slot: 0, reason: Some(R::AboveDependent(0)) },
        // Unrelated and same-mod items keep the legacy slot.
        Row { mode: Dependencies, side: Side::Before, me: OTHER, chain: &[GCORE, TO], slot: 0, reason: None },
        Row { mode: Dependencies, side: Side::After, me: OTHER, chain: &[GCORE, TO], slot: 2, reason: None },
        Row { mode: Dependencies, side: Side::Before, me: GCORE, chain: &[GCORE], slot: 0, reason: None },
        // LoadOrder: last, in application order, but never across the mod's own items (D5).
        Row { mode: LoadOrder, side: Side::Before, me: TO, chain: &[GCORE], slot: 1, reason: Some(R::LoadOrder) },
        Row { mode: LoadOrder, side: Side::After, me: TO, chain: &[GCORE], slot: 1, reason: None },
        Row { mode: LoadOrder, side: Side::Before, me: GCORE, chain: &[GCORE], slot: 0, reason: None },
        Row { mode: LoadOrder, side: Side::Before, me: GCORE, chain: &[TO, GCORE, GMENU], slot: 1, reason: Some(R::LoadOrder) },
    ];
    for (n, r) in rows.iter().enumerate() {
        let got = stack_slot(r.mode, r.side, LegacySlot::Far, r.me, r.chain, deps);
        assert_eq!(
            (got.slot, got.reason),
            (r.slot, r.reason),
            "row {n}: {:?} {:?} me {} chain {:?}",
            r.mode,
            r.side,
            r.me,
            r.chain
        );
        assert_eq!(got.legacy, legacy_slot(r.side, r.chain.len(), LegacySlot::Far), "row {n}");
    }
}

#[test]
fn mutual_dependencies_count_as_unrelated() {
    // ModGraph drops mutual pairs, so the slot rule sees no relation either way.
    let graph = crate::patch::plan::ModGraph::build(&[
        crate::patch::plan::ModMeta {
            id: "a".into(),
            requires: ["b".to_string()].into(),
            ..Default::default()
        },
        crate::patch::plan::ModMeta {
            id: "b".into(),
            load_after: ["a".to_string()].into(),
            loader_index: 1,
            ..Default::default()
        },
    ]);
    let ids = ["a", "b"];
    let dep = |x: u32, y: u32| graph.depends_on(ids[x as usize], ids[y as usize]);
    for kind in [LegacySlot::Near, LegacySlot::Far] {
        for side in [Side::After, Side::Before] {
            let got = stack_slot(Dependencies, side, kind, 0, &[1], dep);
            assert_eq!(got.slot, legacy_slot(side, 1, kind), "{side:?} {kind:?}");
            assert_eq!(got.reason, None);
        }
    }
}

#[test]
fn every_slot_is_in_range() {
    let chains: [&[u32]; 5] = [&[], &[GCORE], &[TO, GCORE], &[GSE, GMENU, GCORE, TO], &[OTHER; 6]];
    for mode in [Legacy, Dependencies, LoadOrder] {
        for side in [Side::After, Side::Before] {
            for kind in [LegacySlot::Near, LegacySlot::Far] {
                for me in 0..=OTHER {
                    for chain in chains {
                        let s = stack_slot(mode, side, kind, me, chain, deps);
                        assert!(s.slot <= chain.len() && s.legacy <= chain.len());
                        if s.reason.is_none() {
                            assert_eq!(s.slot, s.legacy);
                        }
                    }
                }
            }
        }
    }
}

struct RuledRow {
    mode: StackMode,
    side: Side,
    me: u32,
    chain: &'static [u32],
    rules: Vec<SlotRule>,
    slot: usize,
    reason: Option<SlotReason>,
    overridden: &'static [usize],
}

const fn below(item: usize, rank: u8) -> SlotRule {
    SlotRule { item, below: true, rank }
}

const fn above(item: usize, rank: u8) -> SlotRule {
    SlotRule { item, below: false, rank }
}

/// D55, D56: rules act last, in every mode, and the stronger of two that cannot both hold.
#[test]
fn stack_slot_ruled_table() {
    use SlotReason as R;
    let rows = [
        // Below: GDumper goes below TO in Legacy (its legacy slot is the top).
        RuledRow { mode: Legacy, side: Side::After, me: GDUMPER, chain: &[TO, GSE, GMENU, GCORE], rules: vec![below(0, 0)], slot: 1, reason: Some(R::Rule(0)), overridden: &[] },
        // Above: GDumper goes above GSE, which it already is in Legacy: the rule holds.
        RuledRow { mode: Legacy, side: Side::After, me: GDUMPER, chain: &[TO, GSE, GMENU, GCORE], rules: vec![above(1, 0)], slot: 0, reason: Some(R::Rule(0)), overridden: &[] },
        // Above in a Before chain, whose legacy slot is the bottom.
        RuledRow { mode: Legacy, side: Side::Before, me: OTHER, chain: &[GCORE, TO], rules: vec![above(0, 0)], slot: 0, reason: Some(R::Rule(0)), overridden: &[] },
        // Symmetric: the rule held by the later patch moves it the least it must.
        RuledRow { mode: Legacy, side: Side::After, me: TO, chain: &[GDUMPER, GSE], rules: vec![below(0, 0)], slot: 1, reason: Some(R::Rule(0)), overridden: &[] },
        // Rule against the dependency guard: the rule wins.
        RuledRow { mode: Dependencies, side: Side::After, me: GMENU, chain: &[GCORE], rules: vec![above(0, 0)], slot: 0, reason: Some(R::Rule(0)), overridden: &[] },
        // Two rules that contradict: the profile rule (rank 0) stays, compat is overridden.
        RuledRow { mode: Legacy, side: Side::After, me: GDUMPER, chain: &[TO, GSE], rules: vec![above(0, 1), below(0, 0)], slot: 1, reason: Some(R::Rule(1)), overridden: &[0] },
        // Two that both hold: the tighter bound decides.
        RuledRow { mode: Legacy, side: Side::After, me: GDUMPER, chain: &[TO, GSE, GMENU], rules: vec![below(0, 0), below(1, 1)], slot: 2, reason: Some(R::Rule(1)), overridden: &[] },
        // LoadOrder puts it last; a rule above the last item pulls it up.
        RuledRow { mode: LoadOrder, side: Side::After, me: GDUMPER, chain: &[TO, GSE], rules: vec![above(1, 0)], slot: 1, reason: Some(R::Rule(0)), overridden: &[] },
        // No rules: the slot rule as before.
        RuledRow { mode: Dependencies, side: Side::After, me: TO, chain: &[GCORE, GMENU, GSE], rules: vec![], slot: 2, reason: Some(R::BelowDependency(1)), overridden: &[] },
        RuledRow { mode: Legacy, side: Side::After, me: TO, chain: &[GCORE], rules: vec![], slot: 0, reason: None, overridden: &[] },
    ];
    for (n, r) in rows.iter().enumerate() {
        let (got, over) =
            stack_slot_ruled(r.mode, r.side, LegacySlot::Near, r.me, r.chain, deps, &r.rules);
        assert_eq!(
            (got.slot, got.reason, over.as_slice()),
            (r.slot, r.reason, r.overridden),
            "row {n}: {:?} {:?} me {} chain {:?} rules {:?}",
            r.mode,
            r.side,
            r.me,
            r.chain,
            r.rules
        );
        assert_eq!(got.legacy, legacy_slot(r.side, r.chain.len(), LegacySlot::Near), "row {n}");
    }
}

struct HintRow {
    mode: StackMode,
    side: Side,
    me: u32,
    hint: Option<StackHint>,
    chain: &'static [u32],
    hints: &'static [Option<StackHint>],
    slot: usize,
    reason: Option<SlotReason>,
}

const F: Option<StackHint> = Some(StackHint::First);
const L: Option<StackHint> = Some(StackHint::Last);
const N: Option<StackHint> = None;

/// D57: the author hint acts in every mode, inside the dependency guard's range, and stays
/// with the block for later inserts.
#[test]
fn stack_slot_hinted_table() {
    use SlotReason as R;
    let own = |item, hint| Some(R::Hint { item, hint });
    let rows = [
        // First and last with no dependency, in every mode.
        HintRow { mode: Legacy, side: Side::Before, me: OTHER, hint: F, chain: &[GCORE, TO], hints: &[N, N], slot: 0, reason: own(None, StackHint::First) },
        HintRow { mode: Legacy, side: Side::After, me: OTHER, hint: L, chain: &[GCORE, TO], hints: &[N, N], slot: 2, reason: own(None, StackHint::Last) },
        HintRow { mode: Dependencies, side: Side::Before, me: OTHER, hint: F, chain: &[GCORE, TO], hints: &[N, N], slot: 0, reason: own(None, StackHint::First) },
        // LoadOrder would put it last; `first` takes the top, the legacy slot (nothing to report).
        HintRow { mode: LoadOrder, side: Side::After, me: OTHER, hint: F, chain: &[GCORE, TO], hints: &[N, N], slot: 0, reason: None },
        HintRow { mode: LoadOrder, side: Side::Before, me: OTHER, hint: F, chain: &[GCORE, TO], hints: &[N, N], slot: 0, reason: own(None, StackHint::First) },
        // A hint that asks for the legacy slot reports nothing.
        HintRow { mode: Legacy, side: Side::After, me: OTHER, hint: F, chain: &[GCORE, TO], hints: &[N, N], slot: 0, reason: None },
        // A hint never lifts a block above its dependency: TO asks first, stays below GMenu.
        HintRow { mode: Dependencies, side: Side::After, me: TO, hint: F, chain: &[GCORE, GMENU, OTHER], hints: &[N, N, N], slot: 2, reason: Some(R::BelowDependency(1)) },
        // ...nor below a mod that depends on it: GCore asks last, stays above TO.
        HintRow { mode: Dependencies, side: Side::After, me: GCORE, hint: L, chain: &[OTHER, TO, OTHER], hints: &[N, N, N], slot: 1, reason: own(None, StackHint::Last) },
        // Without the guard (Legacy) the hint goes all the way.
        HintRow { mode: Legacy, side: Side::After, me: TO, hint: L, chain: &[GCORE, GMENU, OTHER], hints: &[N, N, N], slot: 3, reason: own(None, StackHint::Last) },
        // Sticky: a later LoadOrder insert does not land below a `last` block.
        HintRow { mode: LoadOrder, side: Side::After, me: GDUMPER, hint: N, chain: &[GCORE, OTHER], hints: &[N, L], slot: 1, reason: own(Some(1), StackHint::Last) },
        HintRow { mode: LoadOrder, side: Side::After, me: GDUMPER, hint: N, chain: &[OTHER, GCORE], hints: &[L, N], slot: 0, reason: None },
        // ...and a later Legacy insert does not go above a `first` block.
        HintRow { mode: Legacy, side: Side::After, me: GDUMPER, hint: N, chain: &[OTHER, GCORE], hints: &[F, N], slot: 1, reason: own(Some(0), StackHint::First) },
        HintRow { mode: Legacy, side: Side::Before, me: GDUMPER, hint: N, chain: &[GCORE, OTHER], hints: &[N, L], slot: 1, reason: own(Some(1), StackHint::Last) },
        // Two firsts: the newer goes right below the older one.
        HintRow { mode: Legacy, side: Side::Before, me: GDUMPER, hint: F, chain: &[OTHER, GCORE], hints: &[F, N], slot: 1, reason: own(None, StackHint::First) },
        // A `first` block of a mod that depends on this one does not hold it below: the
        // dependency guard wins.
        HintRow { mode: Dependencies, side: Side::After, me: GCORE, hint: N, chain: &[TO], hints: &[F], slot: 0, reason: None },
        // Same-mod items keep the legacy order (D5), and their hints do not bind their mod.
        HintRow { mode: Legacy, side: Side::Before, me: OTHER, hint: F, chain: &[GCORE, OTHER], hints: &[N, F], slot: 2, reason: None },
        HintRow { mode: Legacy, side: Side::After, me: OTHER, hint: L, chain: &[OTHER, GCORE], hints: &[L, N], slot: 0, reason: None },
        HintRow { mode: Legacy, side: Side::After, me: OTHER, hint: L, chain: &[GCORE, OTHER, TO], hints: &[N, L, N], slot: 1, reason: own(None, StackHint::Last) },
        // No hints anywhere: `stack_slot`.
        HintRow { mode: Legacy, side: Side::After, me: TO, hint: N, chain: &[GCORE], hints: &[N], slot: 0, reason: None },
        HintRow { mode: Dependencies, side: Side::After, me: TO, hint: N, chain: &[GCORE, GMENU, GSE], hints: &[N, N, N], slot: 2, reason: Some(R::BelowDependency(1)) },
    ];
    for (n, r) in rows.iter().enumerate() {
        let hints = Hints { me: r.hint, items: r.hints };
        let got = stack_slot_hinted(r.mode, r.side, LegacySlot::Near, r.me, r.chain, deps, hints);
        assert_eq!(
            (got.slot, got.reason),
            (r.slot, r.reason),
            "row {n}: {:?} {:?} me {} {:?} chain {:?} {:?}",
            r.mode,
            r.side,
            r.me,
            r.hint,
            r.chain,
            r.hints
        );
        assert_eq!(got.legacy, legacy_slot(r.side, r.chain.len(), LegacySlot::Near), "row {n}");
    }
    // A copy's legacy slot is the far end; a `last` block still bounds it.
    let s = stack_slot_hinted(Legacy, Side::After, LegacySlot::Far, GDUMPER, &[OTHER, GCORE], deps, Hints { me: None, items: &[L, N] });
    assert_eq!((s.slot, s.legacy), (0, 2));
}

/// D56: a stack rule beats an author hint.
#[test]
fn a_rule_beats_a_hint() {
    let hints = Hints { me: Some(StackHint::First), items: &[N, N] };
    let (s, over) = stack_slot_with(Legacy, Side::Before, LegacySlot::Near, OTHER, &[GCORE, TO], deps, hints, &[below(1, 0)]);
    assert_eq!((s.slot, s.reason, over.len()), (2, Some(SlotReason::Rule(0)), 0));
    // With no rule the hint puts it on top.
    let (s, _) = stack_slot_with(Legacy, Side::Before, LegacySlot::Near, OTHER, &[GCORE, TO], deps, hints, &[]);
    assert_eq!(s.slot, 0);
}

#[test]
fn line_keys_tell_equal_lines_apart() {
    let lines = ["a", "x", "b", "c", "x", "b", "a", "x", "b"];
    let k1 = line_key(&lines, |_| false, 1);
    let k4 = line_key(&lines, |_| false, 4);
    let k7 = line_key(&lines, |_| false, 7);
    assert_eq!(k1.ordinal, Some(1));
    assert_ne!(k1.context, k4.context);
    // Lines 1 and 7 read the same around them: the ordinal tells them apart.
    let wide = ["q", "q", "a", "x", "b", "q", "q", "a", "x", "b", "q", "q"];
    let a = line_key(&wide, |_| false, 3);
    let b = line_key(&wide, |_| false, 8);
    assert_eq!(a.context, b.context);
    assert_eq!((a.ordinal, b.ordinal), (Some(1), Some(2)));
    assert!(line_matches(&b, &wide, |_| false, 8));
    assert!(!line_matches(&b, &wide, |_| false, 3));
    let any = SiteKey { context: a.context.clone(), ordinal: None };
    assert!(line_matches(&any, &wide, |_| false, 3) && line_matches(&any, &wide, |_| false, 8));
    assert!(line_matches(&k7, &lines, |_| false, 7));
}
