use super::*;
use crate::conflict::{simulate_and_analyze_patches, SimulationReport};
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, OnFailurePolicy, Patch, PatternPatch, Priority, Target};
use std::collections::HashMap;
use std::path::PathBuf;

const SRC: &str = "function f()\n  x()\n  y()\n  z()\nend\n";

fn pattern(mod_id: &str, id: &str, anchor: &str, position: InsertPosition, payload: &str) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: anchor.to_string(),
        position,
        target: Target::Single("main.lua".to_string()),
        payload: payload.to_string(),
        match_indent: false,
        times: None,
        overwrite: false,
        id: Some(id.to_string()),
        name: Some(format!("{id} name")),
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
        mod_id: Some(mod_id.to_string()),
        disabled: false,
        qm: Default::default(),
    })
}

fn off(mut p: Patch) -> Patch {
    if let Patch::Pattern(x) = &mut p {
        x.disabled = true;
    }
    p
}

fn simulate(patches: Vec<(Patch, Priority)>) -> SimulationReport {
    let mut table = PatchTable::default();
    table.targets.insert("main.lua".to_string());
    for (i, (p, prio)) in patches.into_iter().enumerate() {
        table
            .patches
            .push((p, prio, PathBuf::from(format!("M{i}/qm.toml"))));
    }
    let vanilla = HashMap::from([("main.lua".to_string(), SRC.to_string())]);
    simulate_and_analyze_patches(&table, &vanilla)
}

fn members(g: &FailureGroup) -> Vec<&str> {
    g.members.iter().map(|m| m.patch_id.as_str()).collect()
}

fn edges(r: &SimulationReport) -> Vec<(&str, &str)> {
    r.inferred_edges
        .iter()
        .map(|e| (e.patch_id.as_str(), e.uses_patch_id.as_str()))
        .collect()
}

/// C replaces `y()`, so R loses its line; X hangs on R's code and Z on X's.
fn chain(r: Patch) -> SimulationReport {
    simulate(vec![
        (pattern("c", "c:take", "  y()", InsertPosition::At, "  c()"), 10),
        (r, 20),
        (pattern("x", "x:next", "  r1()", InsertPosition::After, "  x1()"), 30),
        (pattern("z", "z:last", "  x1()", InsertPosition::After, "  z1()"), 40),
    ])
}

#[test]
fn a_three_patch_chain_groups_under_its_root() {
    let r = chain(pattern("r", "r:root", "  y()", InsertPosition::After, "  r1()"));
    assert_eq!(r.failure_groups.len(), 1, "{:?}", r.failure_groups);
    let g = &r.failure_groups[0];
    assert_eq!(g.root, "r:root");
    assert_eq!(g.root_mod, "r");
    assert_eq!(g.root_name, "r:root name");
    assert_eq!(g.reason, FailureReason::NoMatch);
    assert_eq!(g.target_file, "main.lua");
    assert_eq!(members(g), ["x:next", "z:last"]);
    assert_eq!(g.members[1].mod_id, "z");
    assert_eq!(g.members[1].name, "z:last name");
    // Each failed member still has its own broken-anchor conflict to fold.
    for id in ["r:root", "x:next", "z:last"] {
        assert!(
            r.conflicts.iter().any(|c| c.patch_a_id == id && c.kind.starts_with("broken_anchor")),
            "{id}: {:?}",
            r.conflicts
        );
    }
    assert_eq!(edges(&r), [("x:next", "r:root"), ("z:last", "x:next")]);
}

#[test]
fn a_patch_failing_for_its_own_reason_is_not_grouped() {
    let r = simulate(vec![
        (pattern("c", "c:take", "  y()", InsertPosition::At, "  c()"), 10),
        // Its line is in the game's file: C took it.
        (pattern("r", "r:root", "  y()", InsertPosition::After, "  r1()"), 20),
        // Its line is nowhere.
        (pattern("t", "t:typo", "  q()", InsertPosition::After, "  t1()"), 30),
    ]);
    assert!(r.failure_groups.is_empty(), "{:?}", r.failure_groups);
    assert!(r.inferred_edges.is_empty(), "{:?}", r.inferred_edges);
}

#[test]
fn a_turned_off_root_names_why() {
    let r = chain(off(pattern("r", "r:root", "  x()", InsertPosition::After, "  r1()")));
    assert_eq!(r.failure_groups.len(), 1, "{:?}", r.failure_groups);
    let g = &r.failure_groups[0];
    assert_eq!(g.root, "r:root");
    assert_eq!(g.reason, FailureReason::TurnedOff);
    assert_eq!(members(g), ["x:next", "z:last"]);
}

#[test]
fn applied_anchors_on_other_code_are_edges_and_no_group() {
    let r = simulate(vec![
        (pattern("r", "r:root", "  y()", InsertPosition::After, "  r1()"), 20),
        (pattern("x", "x:next", "  r1()", InsertPosition::After, "  x1()"), 30),
        // An anchor on the game's line is no edge.
        (pattern("v", "v:plain", "  z()", InsertPosition::After, "  v1()"), 40),
    ]);
    assert!(r.failure_groups.is_empty(), "{:?}", r.failure_groups);
    assert_eq!(edges(&r), [("x:next", "r:root")]);
}

/// X needs R's code but runs first: it fails for its order, not because R did not apply.
#[test]
fn a_patch_that_runs_before_the_code_it_needs_is_not_grouped() {
    let r = simulate(vec![
        (pattern("x", "x:next", "  r1()", InsertPosition::After, "  x1()"), 10),
        (pattern("r", "r:root", "  y()", InsertPosition::After, "  r1()"), 20),
    ]);
    assert!(r.failure_groups.is_empty(), "{:?}", r.failure_groups);
    assert_eq!(edges(&r), [("x:next", "r:root")]);
}

/// R runs before X and fails: X anchors on R's line, so it fails because R does not apply.
/// Grouping X under R needs R to run first.
#[test]
fn a_failed_patch_that_runs_later_is_not_a_cause() {
    let r = simulate(vec![
        (pattern("c", "c:take", "  y()", InsertPosition::At, "  c()"), 5),
        (pattern("x", "x:next", "  r1()", InsertPosition::After, "  x1()"), 10),
        (pattern("r", "r:root", "  y()", InsertPosition::After, "  r1()"), 20),
    ]);
    assert!(r.failure_groups.is_empty(), "{:?}", r.failure_groups);
    // Still an inferred edge: X needs R's line.
    assert_eq!(edges(&r), [("x:next", "r:root")]);
}

/// A member whose line is in code that applied before it and in code that did not is not
/// grouped. S's line was there for X, but T took it.
#[test]
fn a_line_that_applied_code_also_has_is_not_a_cause() {
    let r = simulate(vec![
        (pattern("c", "c:take", "  y()", InsertPosition::At, "  c()"), 10),
        (pattern("r", "r:root", "  y()", InsertPosition::After, "  r1()"), 20),
        (pattern("s", "s:early", "  z()", InsertPosition::After, "  r1()"), 24),
        (pattern("t", "t:take", "  r1()", InsertPosition::At, "  t1()"), 26),
        (pattern("x", "x:next", "  r1()", InsertPosition::After, "  x1()"), 30),
    ]);
    let status = |id: &str| {
        r.files_touched[0].records.iter().find(|rec| rec.patch_id == id).map(|rec| rec.status)
    };
    assert_eq!(status("t:take"), Some(PatchOutcomeStatus::Applied));
    assert_eq!(status("x:next"), Some(PatchOutcomeStatus::ZeroMatches));
    assert!(r.failure_groups.is_empty(), "{:?}", r.failure_groups);
}

/// Code that applies after X does not keep X out of R's group: it came too late for X.
#[test]
fn a_line_that_later_code_has_does_not_keep_a_member_out() {
    let r = simulate(vec![
        (pattern("c", "c:take", "  y()", InsertPosition::At, "  c()"), 10),
        (pattern("r", "r:root", "  y()", InsertPosition::After, "  r1()"), 20),
        (pattern("s", "s:late", "  z()", InsertPosition::After, "  r1()"), 40),
        (pattern("x", "x:next", "  r1()", InsertPosition::After, "  x1()"), 30),
    ]);
    assert_eq!(r.failure_groups.len(), 1, "{:?}", r.failure_groups);
    assert_eq!(r.failure_groups[0].root, "r:root");
    assert_eq!(members(&r.failure_groups[0]), ["x:next"]);
}

/// A patch skipped for a requirement did not try to match, so it uses no other patch's code.
#[test]
fn a_skipped_patch_has_no_inferred_edge() {
    let mut x = pattern("x", "x:next", "  r1()", InsertPosition::After, "  x1()");
    if let Patch::Pattern(p) = &mut x {
        p.requires = vec!["q:miss".to_string()];
    }
    let r = simulate(vec![
        (pattern("q", "q:miss", "  nothere()", InsertPosition::After, "  q1()"), 10),
        (pattern("r", "r:root", "  y()", InsertPosition::After, "  r1()"), 20),
        (x, 30),
    ]);
    let rec = r.files_touched[0]
        .records
        .iter()
        .find(|rec| rec.patch_id == "x:next")
        .unwrap();
    assert!(skipped(rec), "{rec:?}");
    assert!(r.inferred_edges.is_empty(), "{:?}", r.inferred_edges);
    assert!(r.failure_groups.is_empty(), "{:?}", r.failure_groups);
}
