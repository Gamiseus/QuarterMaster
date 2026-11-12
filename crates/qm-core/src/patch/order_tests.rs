//! Patch ordering tests (plan 2d Task 0). They live here rather than in `crates/qm-core/tests`,
//! which is git-ignored on purpose for the user's stress test.
use super::*;
use crate::handshake::PatchOutcomeStatus;
use crate::patch::table::PatchTable;
use crate::patch::{InsertPosition, OnFailurePolicy, PatternPatch, Target};

fn pat(mod_id: &str, id: &str, before: &[&str], after: &[&str]) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: "-- anchor".to_string(),
        position: InsertPosition::After,
        target: Target::Single("main.lua".to_string()),
        payload: format!("-- {mod_id}:{id}"),
        match_indent: false,
        times: Some(1),
        overwrite: false,
        id: Some(id.to_string()),
        name: Some(id.to_string()),
        description: None,
        requires: Vec::new(),
        priority: None,
        before: before.iter().map(|s| s.to_string()).collect(),
        after: after.iter().map(|s| s.to_string()).collect(),
        on_failure: OnFailurePolicy::Warn,
        anchors: Vec::new(),
        sites: Vec::new(),
        stack: None,
        apply_as: None,
        mod_id: Some(mod_id.to_string()),
        disabled: false,
        qm: Default::default(),
    })
}

fn entry(p: Patch, prio: Priority) -> (Patch, Priority, PathBuf) {
    let m = p.mod_id().unwrap_or("x").to_string();
    (p, prio, PathBuf::from(format!("{m}/qm.toml")))
}

/// Names in application order.
fn names(patches: &[(Patch, Priority, PathBuf)], r: &OrderResult) -> Vec<String> {
    r.order
        .iter()
        .map(|&i| patches[i].0.id().unwrap_or_default().to_string())
        .collect()
}

fn rule(patch: &str, before: &[&str], after: &[&str]) -> OrderRule {
    OrderRule {
        patch: patch.into(),
        before: before.iter().map(|s| s.to_string()).collect(),
        after: after.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn no_rules_keeps_order_golden() {
    // 40 patches across 6 mods in loader order (mod order, then file order), mixed priorities.
    let mod_prio = [50, 10, 50, 30, 90, 10];
    let mut patches = Vec::new();
    for k in 0..40 {
        let m = k % 6;
        let prio = if k % 7 == 0 {
            20
        } else if k % 11 == 0 {
            90
        } else {
            mod_prio[m]
        };
        patches.push(entry(
            pat(&format!("mod{m}"), &format!("p{k}"), &[], &[]),
            prio,
        ));
    }
    // Today's order: a stable sort by priority over the loader order.
    let mut expected: Vec<usize> = (0..patches.len()).collect();
    expected.sort_by_key(|&i| patches[i].1);
    let r = order_patches(&patches, &[]);
    assert_eq!(r.order, expected);
    assert!(r.cycles.is_empty() && r.overridden_rules.is_empty() && r.unknown_ids.is_empty());
}

#[test]
fn after_across_mods() {
    let patches = vec![
        entry(pat("hud", "overlay", &[], &["ai:targeting"]), 10),
        entry(pat("ai", "targeting", &[], &[]), 50),
        entry(pat("ai", "other", &[], &[]), 60),
    ];
    let r = order_patches(&patches, &[]);
    assert_eq!(names(&patches, &r), ["targeting", "overlay", "other"]);
}

#[test]
fn before_mod_star() {
    let patches = vec![
        entry(pat("core", "a", &[], &[]), 10),
        entry(pat("core", "b", &[], &[]), 20),
        entry(pat("late", "first", &["core:*"], &[]), 90),
    ];
    let r = order_patches(&patches, &[]);
    assert_eq!(names(&patches, &r), ["first", "a", "b"]);
}

#[test]
fn profile_rule_overrides_author_rule() {
    let patches = vec![
        entry(pat("hud", "overlay", &[], &["ai:targeting"]), 10),
        entry(pat("ai", "targeting", &[], &[]), 50),
    ];
    let r = order_patches(&patches, &[rule("hud:overlay", &["ai:targeting"], &[])]);
    assert_eq!(names(&patches, &r), ["overlay", "targeting"]);
    assert_eq!(r.overridden_rules, ["ai:targeting before hud:overlay"]);
    assert!(r.cycles.is_empty());
}

#[test]
fn unknown_ids_ignored_with_warning() {
    let patches = vec![
        entry(pat("hud", "overlay", &[], &["gone:patch"]), 10),
        entry(pat("ai", "t", &[], &[]), 5),
    ];
    let r = order_patches(&patches, &[rule("missing:*", &["hud:overlay"], &[])]);
    assert_eq!(names(&patches, &r), ["t", "overlay"]);
    assert_eq!(r.unknown_ids, ["gone:patch", "missing:*"]);
    // Ordering logs nothing itself; the warnings wait in `messages` for `log_messages`.
    let warned: Vec<&str> = r.messages.iter().map(|(_, m)| m.as_str()).collect();
    assert_eq!(
        warned,
        [
            "Patch order rule names 'gone:patch', which matches no loaded patch; ignored.",
            "Patch order rule names 'missing:*', which matches no loaded patch; ignored."
        ]
    );
    assert!(r.messages.iter().all(|(l, _)| *l == log::Level::Warn));
}

#[test]
fn cycle_reported_and_broken_deterministically() {
    // a -> b -> c -> a by author rules, plus a profile rule c before a (same direction as the
    // author edge c -> a, which it strengthens). The weakest edge is an author edge; of a -> b and
    // b -> c, the one from the lower-priority patch (a, priority 10) is dropped.
    let make = || {
        vec![
            entry(pat("m1", "a", &["m2:b"], &[]), 10),
            entry(pat("m2", "b", &["m3:c"], &[]), 20),
            entry(pat("m3", "c", &["m1:a"], &[]), 30),
            entry(pat("m4", "free", &[], &[]), 15),
        ]
    };
    let rules = [rule("m3:c", &["m1:a"], &[])];
    let patches = make();
    let r = order_patches(&patches, &rules);
    assert_eq!(
        r.cycles,
        [vec![
            "m1:a".to_string(),
            "m2:b".to_string(),
            "m3:c".to_string()
        ]]
    );
    // With a -> b dropped: b -> c -> a remain.
    assert_eq!(names(&patches, &r), ["free", "b", "c", "a"]);
    assert_eq!(r.messages.len(), 1);
    assert!(r.messages[0].1.starts_with("Patch order cycle: m1:a -> m2:b -> m3:c."), "{:?}", r.messages);
    for _ in 0..5 {
        assert_eq!(order_patches(&make(), &rules), r);
    }
}

#[test]
fn dll_and_simulation_share_order() {
    // `apply_patches_offline` is what the DLL runs (through `apply_patches`) and what the
    // launcher's simulation runs; both must apply in `ordered()` order.
    let mut table = PatchTable::default();
    table.targets.insert("main.lua".into());
    table.patches = vec![
        entry(pat("hud", "overlay", &[], &["ai:targeting"]), 10),
        entry(pat("ai", "targeting", &[], &[]), 50),
        entry(pat("ai", "other", &[], &[]), 60),
    ];
    table.order_rules = vec![rule("ai:other", &["hud:overlay"], &[])];
    let expected: Vec<String> = table
        .ordered()
        .order
        .iter()
        .map(|&i| table.patches[i].0.id().unwrap().to_string())
        .collect();
    assert_eq!(expected, ["targeting", "other", "overlay"]);

    let (patched, _, records) = table
        .apply_patches_offline("main.lua", "-- anchor\n")
        .unwrap();
    let applied: Vec<&str> = records
        .iter()
        .filter(|r| r.status == PatchOutcomeStatus::Applied)
        .map(|r| r.patch_id.as_str())
        .collect();
    assert_eq!(applied, expected);
    // Each inserts right after the anchor, so the last applied ends up first.
    assert_eq!(
        patched,
        "-- anchor\n-- hud:overlay\n-- ai:other\n-- ai:targeting\n"
    );
}

#[test]
fn shared_order_under_each_stack_mode() {
    use crate::handshake::StackMode;
    use crate::patch::plan::ModMeta;
    let mut table = PatchTable::default();
    table.targets.insert("main.lua".into());
    table.patches = vec![
        entry(pat("hud", "overlay", &[], &["ai:targeting"]), 10),
        entry(pat("ai", "targeting", &[], &[]), 50),
        entry(pat("ai", "other", &[], &[]), 60),
    ];
    table.order_rules = vec![rule("ai:other", &["hud:overlay"], &[])];
    table.mods = ["hud", "ai"]
        .iter()
        .enumerate()
        .map(|(i, id)| ModMeta {
            id: id.to_string(),
            folder: id.to_string(),
            manifest_ok: true,
            loader_index: i as u32,
            ..ModMeta::default()
        })
        .collect();
    let run = |t: &PatchTable| t.apply_patches_offline("main.lua", "-- anchor\n").unwrap().0;
    // Legacy, and Dependencies with unrelated mods: today's text.
    for mode in [StackMode::Legacy, StackMode::Dependencies] {
        table.stack_mode = mode;
        assert_eq!(
            run(&table),
            "-- anchor\n-- hud:overlay\n-- ai:other\n-- ai:targeting\n",
            "{mode:?}"
        );
    }
    // LoadOrder: application order, but ai's own two keep their legacy order (D5).
    table.stack_mode = StackMode::LoadOrder;
    assert_eq!(
        run(&table),
        "-- anchor\n-- ai:other\n-- ai:targeting\n-- hud:overlay\n"
    );
}

/// A small deterministic generator, so the fuzz needs no extra crate.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

#[test]
fn order_fuzz_never_panics_and_places_every_patch() {
    let mut rng = Lcg(0x5eed);
    let mods = ["a", "b", "c"];
    for round in 0..300 {
        let n = rng.next(14) + 1;
        let ids: Vec<String> = (0..n).map(|k| format!("p{k}")).collect();
        let pick = |rng: &mut Lcg| -> String {
            match rng.next(5) {
                // A name that matches nothing, a whole mod, or one patch.
                0 => "missing".to_string(),
                1 => format!("{}:*", mods[rng.next(mods.len())]),
                _ => ids[rng.next(n)].clone(),
            }
        };
        let mut patches = Vec::new();
        for id in &ids {
            let before: Vec<String> = (0..rng.next(3)).map(|_| pick(&mut rng)).collect();
            let after: Vec<String> = (0..rng.next(3)).map(|_| pick(&mut rng)).collect();
            let b: Vec<&str> = before.iter().map(String::as_str).collect();
            let a: Vec<&str> = after.iter().map(String::as_str).collect();
            let m = mods[rng.next(mods.len())];
            let prio = [10, 50, 50, 90][rng.next(4)];
            patches.push(entry(pat(m, id, &b, &a), prio));
        }
        let rules: Vec<OrderRule> = (0..rng.next(4))
            .map(|_| {
                let subject = pick(&mut rng);
                let b = pick(&mut rng);
                let a = pick(&mut rng);
                rule(&subject, &[b.as_str()], &[a.as_str()])
            })
            .collect();
        let r = order_patches(&patches, &rules);
        let mut seen = r.order.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..n).collect::<Vec<_>>(), "round {round}: {:?}", r.order);
        assert!(!r.degraded, "round {round}");
        assert_eq!(order_patches(&patches, &rules), r, "round {round}: deterministic");
    }
}

#[test]
fn place_rest_appends_by_key_and_marks_degraded() {
    let prio = [50, 10, 50, 10];
    let key = |i: usize| (prio[i], i);
    let mut r = OrderResult {
        order: vec![2],
        ..Default::default()
    };
    place_rest(&mut r, 4, key);
    assert_eq!(r.order, [2, 1, 3, 0]);
    assert!(r.degraded);
}

#[test]
fn find_cycle_reports_none_without_an_unplaced_predecessor() {
    let edges = BTreeMap::new();
    let key = |i: usize| (0, i);
    assert_eq!(find_cycle(&edges, &[true, true], key), None);
    assert_eq!(find_cycle(&edges, &[false], key), None);
}

#[test]
fn compat_rules_sit_between_author_and_profile() {
    let patches = vec![
        entry(pat("hud", "overlay", &[], &["ai:targeting"]), 10),
        entry(pat("ai", "targeting", &[], &[]), 50),
    ];
    // A compat rule beats the author rule and is reported with its mod.
    let compat = [(rule("hud:overlay", &["ai:targeting"], &[]), "fixpack".to_string())];
    let r = order_patches_with(&patches, &[], &compat, &[]);
    assert_eq!(names(&patches, &r), ["overlay", "targeting"]);
    assert_eq!(
        r.overridden_rules,
        ["ai:targeting before hud:overlay replaced by fixpack"]
    );
    // A profile rule beats the compat rule, with today's text.
    let profile = [rule("ai:targeting", &["hud:overlay"], &[])];
    let r = order_patches_with(&patches, &profile, &compat, &[]);
    assert_eq!(names(&patches, &r), ["targeting", "overlay"]);
    assert_eq!(r.overridden_rules, ["hud:overlay before ai:targeting"]);
}

#[test]
fn required_edges_beat_profile_rules() {
    let patches = vec![
        entry(pat("to", "six", &[], &[]), 10),
        entry(pat("gcore", "sixteen", &[], &[]), 50),
    ];
    // Patch 0 is anchored on patch 1 (rung 0): 1 runs first, the profile rule is ignored.
    let r = order_patches_with(&patches, &[rule("to:six", &["gcore:sixteen"], &[])], &[], &[(1, 0, 0)]);
    assert_eq!(names(&patches, &r), ["sixteen", "six"]);
    assert_eq!(
        r.overridden_rules,
        ["profile rule to:six before gcore:sixteen ignored: to:six is anchored on gcore:sixteen"]
    );
    assert_eq!(r.implicit_edges, [(1, 0)]);
    assert!(r.suspended_rungs.is_empty() && r.cycles.is_empty());
}

#[test]
fn a_cycle_of_required_edges_suspends_a_rung() {
    let patches = vec![
        entry(pat("m", "a", &[], &[]), 10),
        entry(pat("m", "b", &[], &[]), 20),
    ];
    let r = order_patches_with(&patches, &[], &[], &[(1, 0, 0), (0, 1, 2)]);
    assert_eq!(r.cycles.len(), 1);
    // The edge from the patch that runs first by priority is dropped: b's rung 2.
    assert_eq!(r.suspended_rungs, [(1, 2)]);
    assert_eq!(names(&patches, &r), ["b", "a"]);
    // A cycle with a weaker edge drops that edge instead.
    let patches = vec![
        entry(pat("m", "a", &[], &["m:b"]), 10),
        entry(pat("m", "b", &[], &[]), 20),
    ];
    let r = order_patches_with(&patches, &[], &[], &[(0, 1, 0)]);
    assert!(r.suspended_rungs.is_empty());
    assert_eq!(names(&patches, &r), ["a", "b"]);
}

/// D64: a requirement edge runs the required patch first, beats a profile rule, and in a
/// cycle of requirements is dropped without suspending a rung.
#[test]
fn requirement_edges_run_the_required_patch_first() {
    let patches = vec![
        entry(pat("addon", "follow", &[], &[]), 10),
        entry(pat("core", "base", &[], &[]), 50),
    ];
    let rules = [rule("addon:follow", &["core:base"], &[])];
    let r = order_patches_full(&patches, &rules, &[], &[], &[(1, 0)]);
    assert_eq!(names(&patches, &r), ["base", "follow"]);
    assert_eq!(r.requires_edges, [(1, 0)]);
    assert!(r.implicit_edges.is_empty());
    assert_eq!(
        r.overridden_rules,
        ["profile rule addon:follow before core:base ignored: addon:follow requires core:base"]
    );
    assert!(r
        .edges
        .iter()
        .any(|e| (e.from, e.to, e.explicit) == (1, 0, Some(Strength::Required))));
    let r = order_patches_full(&patches, &[], &[], &[], &[(1, 0), (0, 1)]);
    assert_eq!(r.cycles.len(), 1);
    assert!(r.suspended_rungs.is_empty());
    assert_eq!(r.order.len(), 2);
}

fn copy(mod_id: &str, id: &str, after: &[&str]) -> Patch {
    Patch::Copy(crate::patch::copy::CopyPatch {
        position: crate::patch::copy::CopyPosition::Prepend,
        target: Target::Single("main.lua".to_string()),
        sources: None,
        payload: Some(format!("-- copy {id}")),
        id: Some(id.to_string()),
        name: None,
        priority: None,
        before: Vec::new(),
        after: after.iter().map(|s| s.to_string()).collect(),
        on_failure: OnFailurePolicy::Warn,
        mod_id: Some(mod_id.to_string()),
        disabled: false,
        qm: Default::default(),
        contents: Vec::new(),
    })
}

/// The copy preds of patch `c` as (id, via).
fn preds_of(
    patches: &[(Patch, Priority, PathBuf)],
    r: &OrderResult,
    c: usize,
) -> Vec<(String, Strength)> {
    copy_preds(patches, &r.edges)[c]
        .iter()
        .map(|p| (patches[p.patch].0.id().unwrap_or_default().to_string(), p.via))
        .collect()
}

/// D54: only edges that name the copy itself count, by id or `mod_id:patch_id`, from author,
/// compat, profile or required edges; a `mod_id:*` name never does.
#[test]
fn copy_preds_come_from_edges_that_name_the_copy() {
    let patches = vec![
        entry(pat("y", "p1", &[], &[]), 50),
        entry(pat("y", "p2", &[], &[]), 60),
        entry(copy("x", "c", &[]), 10),
        entry(pat("x", "p3", &[], &[]), 10),
    ];
    // No rule: no preds.
    let r = order_patches(&patches, &[]);
    assert!(r.edges.is_empty());
    assert!(preds_of(&patches, &r, 2).is_empty());

    // Wildcards on the copy's side never count.
    for rules in [
        vec![rule("y:*", &["x:*"], &[])],
        vec![rule("x:*", &[], &["y:p1"])],
        vec![rule("y:p1", &["x:*"], &[])],
    ] {
        let r = order_patches(&patches, &rules);
        assert!(!r.edges.is_empty(), "{rules:?}");
        assert!(preds_of(&patches, &r, 2).is_empty(), "{rules:?}");
        let r = order_patches_with(&patches, &[], &[(rules[0].clone(), "fixpack".into())], &[]);
        assert!(preds_of(&patches, &r, 2).is_empty(), "compat {rules:?}");
    }

    // The copy named by id or mod_id:patch_id counts, whatever names the pattern.
    let r = order_patches(&patches, &[rule("y:*", &["x:c"], &[])]);
    assert_eq!(
        preds_of(&patches, &r, 2),
        [("p1".to_string(), Strength::Profile), ("p2".to_string(), Strength::Profile)]
    );
    let r = order_patches(&patches, &[rule("x:c", &[], &["y:p2"])]);
    assert_eq!(preds_of(&patches, &r, 2), [("p2".to_string(), Strength::Profile)]);
    let r = order_patches_with(&patches, &[], &[(rule("y:p1", &["c"], &[]), "fixpack".into())], &[]);
    assert_eq!(preds_of(&patches, &r, 2), [("p1".to_string(), Strength::Compat)]);

    // Author lists: on the copy, any name; on the pattern, the copy by name.
    let mut authored = patches.clone();
    authored[2] = entry(copy("x", "c", &["y:*"]), 10);
    let r = order_patches(&authored, &[]);
    assert_eq!(
        preds_of(&authored, &r, 2),
        [("p1".to_string(), Strength::Author), ("p2".to_string(), Strength::Author)]
    );
    let mut authored = patches.clone();
    authored[0] = entry(pat("y", "p1", &["x:c"], &[]), 50);
    let r = order_patches(&authored, &[]);
    assert_eq!(preds_of(&authored, &r, 2), [("p1".to_string(), Strength::Author)]);
    authored[0] = entry(pat("y", "p1", &["x:*"], &[]), 50);
    let r = order_patches(&authored, &[]);
    assert!(preds_of(&authored, &r, 2).is_empty());

    // Through other explicit edges, required ones too: p2 before p1 (a rung), p1 before c.
    let r = order_patches_with(&patches, &[rule("y:p1", &["x:c"], &[])], &[], &[(1, 0, 0)]);
    assert_eq!(
        preds_of(&patches, &r, 2),
        [("p1".to_string(), Strength::Profile), ("p2".to_string(), Strength::Profile)]
    );
    // Only copies have preds; a pattern after another pattern has none.
    let r = order_patches(&patches, &[rule("y:p1", &["x:p3"], &[])]);
    assert!(copy_preds(&patches, &r.edges)[3].is_empty());
}

/// D54: an explicit edge dropped by a stronger rule or a cycle gives no pred, so the copy stays
/// with the first copies.
#[test]
fn a_dropped_edge_gives_no_copy_pred() {
    let patches = vec![
        entry(pat("y", "p1", &["x:c"], &[]), 50),
        entry(copy("x", "c", &[]), 10),
    ];
    let r = order_patches(&patches, &[]);
    assert_eq!(preds_of(&patches, &r, 1), [("p1".to_string(), Strength::Author)]);
    // A profile rule the other way beats the author rule.
    let r = order_patches(&patches, &[rule("x:c", &["y:p1"], &[])]);
    assert_eq!(r.overridden_rules, ["y:p1 before x:c"]);
    assert!(preds_of(&patches, &r, 1).is_empty());
    // A cycle of equal rules drops one edge; if it was the one into the copy, no pred.
    let patches = vec![
        entry(pat("y", "p1", &["x:c"], &[]), 50),
        entry(copy("x", "c", &[]), 10),
        entry(pat("y", "p2", &["y:p1"], &[]), 60),
    ];
    let cyc = [rule("x:c", &["y:p2"], &[])];
    let r = order_patches_with(&patches, &[], &[(cyc[0].clone(), "fixpack".into())], &[]);
    assert_eq!(r.cycles.len(), 1);
    let kept: Vec<(usize, usize)> = r.edges.iter().map(|e| (e.from, e.to)).collect();
    assert!(!kept.contains(&(0, 1)), "the weakest edge, p1 before c, was dropped: {kept:?}");
    assert!(preds_of(&patches, &r, 1).is_empty());
}

/// The pattern pass: without ordered copies it is the order's patterns and regexes; an ordered
/// copy goes right after what an edge puts before it, and the rest follows it.
#[test]
fn pattern_pass_runs_an_ordered_copy_as_early_as_its_edges_allow() {
    let patches = vec![
        entry(pat("x", "third", &[], &[]), 10),
        entry(copy("x", "c", &[]), 10),
        entry(pat("y", "p1", &[], &[]), 50),
        entry(pat("z", "p0", &[], &[]), 40),
        entry(pat("w", "late", &[], &[]), 90),
    ];
    let ids = |pass: &[usize]| -> Vec<&str> {
        pass.iter().map(|&i| patches[i].0.id().unwrap_or_default()).collect()
    };
    let r = order_patches(&patches, &[rule("y:p1", &["x:c"], &[])]);
    assert_eq!(names(&patches, &r), ["third", "p0", "p1", "c", "late"]);
    assert_eq!(ids(&pattern_pass(&patches, &r, &[])), ["third", "p0", "p1", "late"]);
    // p1 runs first; third and p0 have no edge to the copy, so they see its text.
    assert_eq!(ids(&pattern_pass(&patches, &r, &[1])), ["p1", "c", "third", "p0", "late"]);

    // Whatever must run before p1 runs before the copy too, in order.
    let r = order_patches(
        &patches,
        &[rule("y:p1", &["x:c"], &[]), rule("z:p0", &["y:p1"], &[])],
    );
    assert_eq!(ids(&pattern_pass(&patches, &r, &[1])), ["p0", "p1", "c", "third", "late"]);

    // A first copy between two patterns takes no turn but passes the edge on: p2 still
    // waits for p1.
    let patches = vec![
        entry(pat("y", "p1", &[], &[]), 50),
        entry(copy("x", "first", &[]), 10),
        entry(pat("y", "p2", &[], &[]), 5),
        entry(copy("x", "c", &[]), 10),
        entry(pat("y", "p3", &[], &[]), 70),
    ];
    let r = order_patches(
        &patches,
        &[
            rule("y:p1", &["x:first", "x:c"], &[]),
            rule("x:first", &["y:p2"], &[]),
        ],
    );
    assert_eq!(names(&patches, &r), ["p1", "first", "p2", "c", "p3"]);
    let ids = |pass: &[usize]| -> Vec<&str> {
        pass.iter().map(|&i| patches[i].0.id().unwrap_or_default()).collect()
    };
    assert_eq!(ids(&pattern_pass(&patches, &r, &[3])), ["p1", "c", "p2", "p3"]);
}
