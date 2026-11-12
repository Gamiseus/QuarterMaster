//! `same_line_edit` (D49, §6.5).
use std::collections::HashMap;
use std::path::PathBuf;

use qm_core::patch::edits::ApplyAs;
use qm_core::patch::plan::HunkSkip;
use qm_core::patch::{InsertPosition, OnFailurePolicy, Patch, PatternPatch, Target};

use crate::conflict::{
    simulate_and_analyze_patches, ConflictResolutionSuggestion, DetectedConflict,
    SimulationReport,
};
use qm_core::patch::table::PatchTable;

const FILE: &str = "includes/stats.lua";
const SRC: &str = "function unit(u)\n  u.speed = 10\n  u.armor = 2\n  u.range = 5\nend\n";

fn at(mod_id: &str, id: &str, pattern: &str, payload: &str) -> PatternPatch {
    PatternPatch {
        pattern: pattern.to_string(),
        position: InsertPosition::At,
        target: Target::Single(FILE.to_string()),
        payload: payload.to_string(),
        match_indent: true,
        times: None,
        overwrite: false,
        id: Some(id.to_string()),
        name: Some(id.to_string()),
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
    }
}

fn a_patch() -> PatternPatch {
    at(
        "amod",
        "amod:1",
        "u.speed = 10\nu.armor = 2\nu.range = 5",
        "u.speed = 10\nu.armor = 4\nu.range = 5\nu.morale = 1",
    )
}

fn b_patch() -> PatternPatch {
    at(
        "bmod",
        "bmod:1",
        "u.armor = 2\nu.range = 5",
        "u.armor = 3\nu.range = 5\nu.stamina = 7",
    )
}

fn simulate(patches: Vec<PatternPatch>) -> SimulationReport {
    simulate_src(patches, SRC)
}

fn simulate_src(patches: Vec<PatternPatch>, src: &str) -> SimulationReport {
    let mut table = PatchTable::default();
    table.targets.insert(FILE.to_string());
    for (i, p) in patches.into_iter().enumerate() {
        let prio = 10 * (i32::try_from(i).unwrap_or(0) + 1);
        table
            .patches
            .push((Patch::Pattern(p), prio, PathBuf::from(format!("M{i}/qm.toml"))));
    }
    let vanilla = HashMap::from([(FILE.to_string(), src.to_string())]);
    simulate_and_analyze_patches(&table, &vanilla)
}

fn same_line(r: &SimulationReport) -> Vec<&DetectedConflict> {
    r.conflicts
        .iter()
        .filter(|c| c.kind == "same_line_edit")
        .collect()
}

/// Two patches change `u.armor`: one warning, which offers to keep either change.
#[test]
fn two_changes_to_one_line_offer_both_choices() {
    let r = simulate(vec![a_patch(), b_patch()]);
    let found = same_line(&r);
    assert_eq!(found.len(), 1, "{:?}", r.conflicts);
    let c = found[0];
    assert_eq!(c.id, "same_line:includes/stats.lua:amod:1:bmod:1:3");
    assert_eq!((c.patch_a_id.as_str(), c.patch_b_id.as_deref()), ("amod:1", Some("bmod:1")));
    assert!(c.description.contains("both change line 3"), "{}", c.description);
    // The side-by-side view: the game's line and each patch's version of it.
    let d = c.same_line.as_ref().expect("the line detail");
    assert_eq!((d.line, d.vanilla.as_str()), (3, "u.armor = 2"));
    assert_eq!((d.a_text.as_deref(), d.b_text.as_deref()), (Some("u.armor = 4"), Some("u.armor = 3")));
    let picks: Vec<(String, u32, String)> = c
        .suggestions
        .iter()
        .filter_map(|s| match s {
            ConflictResolutionSuggestion::SkipHunk {
                patch_id,
                ordinal,
                keep_patch_id,
                ..
            } => Some((patch_id.clone(), *ordinal, keep_patch_id.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        picks,
        [
            ("bmod:1".to_string(), 0, "amod:1".to_string()),
            ("amod:1".to_string(), 0, "bmod:1".to_string()),
        ]
    );
}

/// Once one change is kept the warning is gone, both patches apply, and a skipped line never
/// counts as a gained place (`sites_gained` stays silent).
#[test]
fn a_choice_clears_the_warning() {
    let mut b = b_patch();
    b.qm.skip_hunks.push(HunkSkip {
        ordinal: 0,
        vanilla: "u.armor = 2".into(),
        keep: Some("amod:1".into()),
    });
    b.qm.apply_as = Some(ApplyAs::Edits);
    let r = simulate(vec![a_patch(), b]);
    assert!(same_line(&r).is_empty(), "{:?}", r.conflicts);
    assert!(
        !r.conflicts
            .iter()
            .any(|c| c.kind == "sites_gained" || c.kind == "broken_anchor_order"),
        "{:?}",
        r.conflicts
    );
    assert_eq!((r.applied_patches, r.total_patches), (2, 2));
    assert!(r.traces[FILE].text.contains("u.armor = 4\n  u.range = 5\n  u.stamina = 7"));
}

/// The 3-line Halt-like case: the skipped middle line is relaxed only at the patch's own place,
/// so g(), whose middle line another patch owns, is no gained place.
#[test]
fn a_relaxed_middle_line_gains_no_place() {
    let src = "function f()\n  x = 1\n  y = 2\n  z = 3\nend\nfunction g()\n  x = 1\n  w = 9\n  z = 3\nend\n";
    let a = at("amod", "amod:1", "y = 2", "y = 8");
    let c = at("cmod", "cmod:1", "w = 9", "w = 10");
    let mut halt = at("bmod", "bmod:1", "x = 1\ny = 2\nz = 3", "x = 1\ny = 5\nz = 3\nq = 0");
    halt.qm.skip_hunks.push(HunkSkip {
        ordinal: 0,
        vanilla: "y = 2".into(),
        keep: Some("amod:1".into()),
    });
    halt.qm.apply_as = Some(ApplyAs::Edits);
    let r = simulate_src(vec![a, c, halt], src);
    assert!(
        !r.conflicts.iter().any(|c| c.kind == "sites_gained"),
        "{:?}",
        r.conflicts
    );
    assert_eq!((r.applied_patches, r.total_patches), (3, 3));
    let text = &r.traces[FILE].text;
    assert_eq!(text.matches("q = 0").count(), 1, "{text}");
    assert!(text.contains("  w = 10\n  z = 3\nend\n"), "{text}");
}

/// Patches that only add lines, or patches of one mod, are not a `same_line_edit`.
#[test]
fn inserts_and_one_mod_are_not_reported() {
    let insert = at(
        "bmod",
        "bmod:1",
        "u.armor = 2",
        "u.armor = 2\nu.extra = 1",
    );
    assert!(same_line(&simulate(vec![a_patch(), insert])).is_empty());
    let mut b = b_patch();
    b.mod_id = Some("amod".into());
    assert!(same_line(&simulate(vec![a_patch(), b])).is_empty());
}
