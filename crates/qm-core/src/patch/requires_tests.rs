//! Patch-level `requires` (plan 2e Phase D, D64, Task 27): a patch whose required patch of
//! the same file did not apply there is skipped, and the required patch runs first. A
//! required patch of another file is judged by the table only (loaded and on), with a
//! warning and never a skip, so the DLL, which patches each file as it loads, and the
//! simulation agree.
use super::*;
use crate::handshake::LaunchConfig;
use crate::patch::order::OrderRule;
use crate::patch::parity::{
    check_dll_parity, check_linemap, check_parity, dll_outcome, simulation_outcome, Outcome,
};
use crate::patch::{InsertPosition, PatternPatch};
use crate::prepare::simulate_buffer;

const A: &str = "includes/a.lua";
const B: &str = "includes/b.lua";
const A_SRC: &str = "local cfg = {}\ncfg.speed = 1\ncfg.armor = 2\nreturn cfg\n";
const B_SRC: &str = "local ui = {}\nui.scale = 1\nreturn ui\n";

/// A pattern patch `mod_id:id` on `file`.
fn pat(
    mod_id: &str,
    id: &str,
    file: &str,
    pattern: &str,
    position: InsertPosition,
    payload: &str,
) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: pattern.to_string(),
        position,
        target: Target::Single(file.to_string()),
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

fn requiring(mut p: Patch, names: &[&str]) -> Patch {
    if let Patch::Pattern(x) = &mut p {
        x.requires = names.iter().map(|n| n.to_string()).collect();
    }
    p
}

/// core:base turns `cfg.speed = 1` into `cfg.speed = 5`.
fn base() -> Patch {
    pat(
        "core",
        "base",
        A,
        "cfg.speed = 1",
        InsertPosition::At,
        "cfg.speed = 5",
    )
}

/// A base whose line is not in the file.
fn broken_base() -> Patch {
    pat(
        "core",
        "base",
        A,
        "cfg.nothing = 0",
        InsertPosition::At,
        "cfg.speed = 5",
    )
}

/// addon:follow adds a line after `cfg.armor = 2`; it matches without core:base.
fn follow(requires: &[&str]) -> Patch {
    let p = pat(
        "addon",
        "follow",
        A,
        "cfg.armor = 2",
        InsertPosition::After,
        "cfg.extra = true",
    );
    requiring(p, requires)
}

/// addon:ui patches the other file and requires core:base.
fn ui(requires: &[&str]) -> Patch {
    let p = pat(
        "addon",
        "ui",
        B,
        "ui.scale = 1",
        InsertPosition::After,
        "ui.dark = true",
    );
    requiring(p, requires)
}

/// Patches with priorities in the order given, so the first one sorts first.
fn table(patches: Vec<Patch>) -> PatchTable {
    let mut t = PatchTable::default();
    t.targets.insert(A.to_string());
    t.targets.insert(B.to_string());
    for (i, p) in patches.into_iter().enumerate() {
        let folder = p.mod_id().unwrap_or("m").to_string();
        t.patches.push((
            p,
            10 * (i as i32 + 1),
            PathBuf::from(folder).join("qm.toml"),
        ));
    }
    t
}

fn disabled(mut p: Patch) -> Patch {
    p.set_disabled(true);
    p
}

/// The simulation of `file`, after checking every entry point, the DLL and the line map.
fn run(t: &PatchTable, file: &str, src: &str) -> TracedApply {
    check_parity(t, file, src).unwrap_or_else(|e| panic!("parity: {e}"));
    check_dll_parity(t, file, src, &LaunchConfig::default())
        .unwrap_or_else(|e| panic!("DLL parity: {e}"));
    check_linemap(t, &LaunchConfig::default(), file, src)
        .unwrap_or_else(|e| panic!("line map: {e}"));
    let prep = t.prepare();
    simulate_buffer(t, &prep, &LaunchConfig::default(), file, src)
        .unwrap()
        .1
}

fn record<'a>(t: &'a TracedApply, id: &str) -> &'a PatchExecutionRecord {
    t.records
        .iter()
        .find(|r| r.patch_id == id)
        .unwrap_or_else(|| panic!("no record {id}: {:#?}", t.records))
}

fn needs(rec: &PatchExecutionRecord) -> Vec<&str> {
    let code = format!("{NEEDS_PATCH}:");
    rec.warnings
        .iter()
        .map(String::as_str)
        .filter(|w| w.starts_with(&code))
        .collect()
}

fn ids(t: &TracedApply) -> Vec<&str> {
    t.records.iter().map(|r| r.patch_id.as_str()).collect()
}

#[test]
fn a_required_patch_of_the_same_file_runs_first() {
    // By priority follow sorts first; its requirement puts core:base before it.
    let t = table(vec![follow(&["core:base"]), base()]);
    let prep = t.prepare();
    assert_eq!(prep.order().requires_edges, [(1, 0)]);
    assert_eq!(prep.order().order, [1, 0]);
    assert!(prep.order().implicit_edges.is_empty(), "no rung");
    let r = run(&t, A, A_SRC);
    assert_eq!(ids(&r), ["base", "follow"]);
    assert!(r
        .records
        .iter()
        .all(|x| x.status == PatchOutcomeStatus::Applied));
    assert!(needs(record(&r, "follow")).is_empty());
    assert_eq!(
        r.text,
        "local cfg = {}\ncfg.speed = 5\ncfg.armor = 2\ncfg.extra = true\nreturn cfg\n"
    );
}

#[test]
fn a_failed_required_patch_skips_the_patch_and_what_needs_it() {
    let third = requiring(
        pat(
            "addon",
            "third",
            A,
            "return cfg",
            InsertPosition::Before,
            "cfg.third = 3",
        ),
        &["addon:follow"],
    );
    let t = table(vec![broken_base(), follow(&["core:base"]), third]);
    let r = run(&t, A, A_SRC);
    assert_eq!(record(&r, "base").status, PatchOutcomeStatus::ZeroMatches);
    let f = record(&r, "follow");
    assert_eq!(
        (f.status, f.matched_count),
        (PatchOutcomeStatus::SkippedByPolicy, 0)
    );
    assert_eq!(
        f.warnings,
        ["needs_patch:core:base: needs 'core:base', which did not apply"]
    );
    assert!(skipped_for_requirement(f));
    let third = record(&r, "third");
    assert_eq!(third.status, PatchOutcomeStatus::SkippedByPolicy);
    assert_eq!(
        third.warnings,
        ["needs_patch:addon:follow: needs 'addon:follow', which did not apply"]
    );
    assert_eq!(r.text, A_SRC, "nothing applied");
    assert!(!skipped_for_requirement(record(&r, "base")));
}

#[test]
fn a_disabled_required_patch_skips_the_patch() {
    let t = table(vec![disabled(base()), follow(&["core:base"])]);
    let r = run(&t, A, A_SRC);
    assert_eq!(
        record(&r, "base").status,
        PatchOutcomeStatus::SkippedDisabled
    );
    let f = record(&r, "follow");
    assert_eq!(f.status, PatchOutcomeStatus::SkippedByPolicy);
    assert_eq!(
        needs(f),
        ["needs_patch:core:base: needs 'core:base', which did not apply"]
    );
    assert_eq!(r.text, A_SRC);
}

#[test]
fn a_required_patch_in_another_mod_resolves_by_mod_and_id() {
    // `base` alone names core's patch too; `other:base` names nothing.
    let t = table(vec![base(), follow(&["base"])]);
    assert_eq!(t.prepare().requires(1)[0].hits, [0]);
    let t = table(vec![base(), follow(&["other:base"])]);
    let r = run(&t, A, A_SRC);
    let f = record(&r, "follow");
    assert_eq!(
        f.status,
        PatchOutcomeStatus::Applied,
        "an unknown name only warns"
    );
    assert_eq!(
        needs(f),
        ["needs_patch:other:base: needs 'other:base', which matches no loaded patch"]
    );
    // `core:*` names every core patch.
    let t = table(vec![broken_base(), follow(&["core:*"])]);
    let r = run(&t, A, A_SRC);
    assert_eq!(
        record(&r, "follow").status,
        PatchOutcomeStatus::SkippedByPolicy
    );
}

#[test]
fn a_required_patch_of_another_file_never_skips() {
    // On and loaded: no warning, on either file.
    let t = table(vec![base(), ui(&["core:base"])]);
    assert!(
        t.prepare().order().requires_edges.is_empty(),
        "no edge across files"
    );
    let r = run(&t, B, B_SRC);
    let u = record(&r, "ui");
    assert_eq!(u.status, PatchOutcomeStatus::Applied);
    assert!(u.warnings.is_empty(), "{:?}", u.warnings);

    // It fails in its own file: still no warning and no skip here.
    let t = table(vec![broken_base(), ui(&["core:base"])]);
    let r = run(&t, B, B_SRC);
    assert_eq!(record(&r, "ui").status, PatchOutcomeStatus::Applied);
    assert!(record(&r, "ui").warnings.is_empty());

    // Turned off: a warning only.
    let t = table(vec![disabled(base()), ui(&["core:base"])]);
    let r = run(&t, B, B_SRC);
    let u = record(&r, "ui");
    assert_eq!(u.status, PatchOutcomeStatus::Applied);
    assert_eq!(
        needs(u),
        ["needs_patch:core:base: needs 'core:base', which is turned off; it patches another file, so this patch still runs"]
    );
    assert!(!skipped_for_requirement(u));
    assert_eq!(
        r.text,
        "local ui = {}\nui.scale = 1\nui.dark = true\nreturn ui\n"
    );
}

/// The DLL patches each file as the game loads it, in any order; the simulation patches each
/// file alone. Both give the same records, the cross-file warning included.
#[test]
fn a_cross_file_requirement_is_the_same_in_the_dll_and_the_simulation() {
    let t = table(vec![
        disabled(base()),
        ui(&["core:base"]),
        follow(&["core:base"]),
    ]);
    let prep = t.prepare();
    let launch = LaunchConfig::default();
    let records = |o: Outcome| match o {
        Outcome::Patched { records, .. } => records,
        Outcome::Aborted(e) => panic!("{e}"),
    };
    // b.lua first, then a.lua, then b.lua again.
    let b1 = records(dll_outcome(&t, &prep, &launch, &format!("@{B}"), B_SRC));
    let a = records(dll_outcome(&t, &prep, &launch, &format!("@{A}"), A_SRC));
    let b2 = records(dll_outcome(&t, &prep, &launch, &format!("@{B}"), B_SRC));
    let sim_a = records(simulation_outcome(&t, &prep, &launch, A, A_SRC));
    let sim_b = records(simulation_outcome(&t, &prep, &launch, B, B_SRC));
    let json = |r: &[PatchExecutionRecord]| serde_json::to_string(r).unwrap();
    assert_eq!(json(&b1), json(&sim_b));
    assert_eq!(json(&b2), json(&sim_b));
    assert_eq!(json(&a), json(&sim_a));
    let ui_rec = sim_b.iter().find(|r| r.patch_id == "ui").unwrap();
    assert_eq!(ui_rec.status, PatchOutcomeStatus::Applied);
    assert_eq!(needs(ui_rec).len(), 1);
    let f = sim_a.iter().find(|r| r.patch_id == "follow").unwrap();
    assert_eq!(f.status, PatchOutcomeStatus::SkippedByPolicy);
    run(&t, A, A_SRC);
    run(&t, B, B_SRC);
}

#[test]
fn two_patches_that_require_each_other_both_apply() {
    let base = requiring(base(), &["addon:follow"]);
    let t = table(vec![base, follow(&["core:base"])]);
    let prep = t.prepare();
    assert_eq!(prep.order().cycles.len(), 1, "{:?}", prep.order());
    let r = run(&t, A, A_SRC);
    assert!(r
        .records
        .iter()
        .all(|x| x.status == PatchOutcomeStatus::Applied));
    let first = &r.records[0];
    assert_eq!(needs(first).len(), 1, "{:?}", first.warnings);
    assert!(needs(first)[0].contains("runs after it in 'includes/a.lua', so it was not checked"));
    assert!(needs(&r.records[1]).is_empty());
}

#[test]
fn a_profile_rule_against_a_requirement_is_ignored() {
    let mut t = table(vec![base(), follow(&["core:base"])]);
    t.order_rules = vec![OrderRule {
        patch: "addon:follow".to_string(),
        before: vec!["core:base".to_string()],
        after: Vec::new(),
    }];
    let prep = t.prepare();
    assert_eq!(prep.order().order, [0, 1]);
    assert_eq!(
        prep.order().overridden_rules,
        ["profile rule addon:follow before core:base ignored: addon:follow requires core:base"]
    );
}

#[test]
fn the_stamp_changes_when_requires_changes() {
    let plain = table(vec![base(), follow(&[])]).stamp();
    let with = table(vec![base(), follow(&["core:base"])]).stamp();
    let other = table(vec![base(), follow(&["core:*"])]).stamp();
    assert_ne!(plain, with);
    assert_ne!(with, other);
    // A Prepared from before the change is rebuilt, so the skip follows the new list.
    let old = table(vec![broken_base(), follow(&[])]);
    let prep = old.prepare();
    let new = table(vec![broken_base(), follow(&["core:base"])]);
    let (_, t) = simulate_buffer(&new, &prep, &LaunchConfig::default(), A, A_SRC).unwrap();
    assert_eq!(
        record(&t, "follow").status,
        PatchOutcomeStatus::SkippedByPolicy
    );
}
