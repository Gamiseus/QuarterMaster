//! Per-edit choices (D49, Task 21): a skipped edit of an `at` patch does not run, and its lines
//! match the game's text or the kept patch's lines at the patch's own places.
use super::*;
use crate::handshake::{LaunchConfig, PatchExecutionRecord, PatchOutcomeStatus, StackMode};
use crate::patch::parity::{check_dll_parity, check_linemap, check_parity};
use crate::patch::plan::{HunkSkip, ModMeta};
use crate::patch::table::PatchTable;
use crate::patch::{OnFailurePolicy, Patch};
use std::path::PathBuf;

const FILE: &str = "includes/skip.lua";

fn src() -> String {
    "function f()\n  local a = 1\n  local b = 2\n  local c = 3\nend\n".to_string()
}

fn at_patch(id: &str, pattern: &str, payload: &str) -> PatternPatch {
    PatternPatch {
        pattern: pattern.to_string(),
        position: InsertPosition::At,
        target: Target::Single(FILE.to_string()),
        payload: payload.to_string(),
        match_indent: true,
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
    }
}

/// A's change: `b` becomes 20, and `d` goes below `c` (Keep, Change, Keep, Insert).
fn a_patch() -> PatternPatch {
    at_patch(
        "amod:skip:1",
        "local a = 1\nlocal b = 2\nlocal c = 3",
        "local a = 1\nlocal b = 20\nlocal c = 3\nlocal d = 4",
    )
}

/// B's change: `b` becomes 200, and `e` goes below `c` (Change, Keep, Insert).
fn b_patch() -> PatternPatch {
    at_patch(
        "bmod:skip:1",
        "local b = 2\nlocal c = 3",
        "local b = 200\nlocal c = 3\nlocal e = 5",
    )
}

/// `p` with edit `ordinal` skipped and edits mode on, as the plan sets them.
fn skipping(mut p: PatternPatch, ordinal: u32, keep: Option<&str>) -> PatternPatch {
    let h = p.normalized().expect("normalizes");
    let (_, hunk) = edits::edit_hunks(&h)
        .find(|(o, _)| *o == ordinal)
        .expect("the edit");
    p.qm.skip_hunks.push(HunkSkip {
        ordinal,
        vanilla: edits::hunk_vanilla(&p.pattern, hunk),
        keep: keep.map(str::to_string),
    });
    p.qm.apply_as = Some(ApplyAs::Edits);
    p
}

/// A table of `patches` in order (each one's priority is its place).
fn table(patches: Vec<PatternPatch>, mode: StackMode) -> PatchTable {
    let mut t = PatchTable::default();
    t.targets.insert(FILE.to_string());
    for (i, p) in patches.into_iter().enumerate() {
        let m = p.mod_id.clone().unwrap_or_default();
        let prio = i32::try_from(i).expect("small") * 10;
        t.patches
            .push((Patch::Pattern(p), prio, PathBuf::from(format!("{m}/qm.toml"))));
    }
    let meta = |id: &str, ix: u32| ModMeta {
        id: id.to_string(),
        folder: id.to_string(),
        manifest_ok: true,
        loader_index: ix,
        ..ModMeta::default()
    };
    t.mods = vec![meta("amod", 0), meta("bmod", 1), meta("cmod", 2)];
    t.stack_mode = mode;
    t
}

fn run(t: &PatchTable, src: &str) -> (String, Vec<PatchExecutionRecord>) {
    check_parity(t, FILE, src).unwrap_or_else(|e| panic!("parity: {e}"));
    check_dll_parity(t, FILE, src, &LaunchConfig::default())
        .unwrap_or_else(|e| panic!("DLL parity: {e}"));
    check_linemap(t, &LaunchConfig::default(), FILE, src)
        .unwrap_or_else(|e| panic!("line map: {e}"));
    let (text, _, records) = t.apply_patches_offline(FILE, src).expect("no abort");
    (text, records)
}

fn record<'a>(records: &'a [PatchExecutionRecord], id: &str) -> &'a PatchExecutionRecord {
    records
        .iter()
        .find(|r| r.patch_id == id)
        .unwrap_or_else(|| panic!("no record for {id}"))
}

fn applied(records: &[PatchExecutionRecord], id: &str) -> bool {
    let r = record(records, id);
    r.status == PatchOutcomeStatus::Applied && r.matched_count == 1
}

/// Without a choice the second patch loses its place: the line it changes is gone.
#[test]
fn without_a_choice_the_second_patch_misses() {
    for (first, second) in [(a_patch(), b_patch()), (b_patch(), a_patch())] {
        let second_id = second.id.clone().unwrap_or_default();
        let t = table(vec![first, second], StackMode::Legacy);
        let (_, records) = run(&t, &src());
        assert_eq!(
            record(&records, &second_id).status,
            PatchOutcomeStatus::ZeroMatches
        );
    }
}

/// Keep A and Keep B, in both orders and in Legacy and Dependencies: the kept change is the
/// only one on `b`, and both patches' other edits apply.
#[test]
fn keep_a_and_keep_b_in_both_orders() {
    for mode in [StackMode::Legacy, StackMode::Dependencies] {
        for a_first in [true, false] {
            for keep_a in [true, false] {
                let (a, b) = if keep_a {
                    (a_patch(), skipping(b_patch(), 0, Some("amod:skip:1")))
                } else {
                    (skipping(a_patch(), 0, Some("bmod:skip:1")), b_patch())
                };
                let patches = if a_first { vec![a, b] } else { vec![b, a] };
                let t = table(patches, mode);
                let (text, records) = run(&t, &src());
                let what = format!("{mode:?}, a first {a_first}, keep a {keep_a}: {text}");
                assert!(applied(&records, "amod:skip:1"), "{what}");
                assert!(applied(&records, "bmod:skip:1"), "{what}");
                let (kept, gone) = if keep_a {
                    ("local b = 20\n", "local b = 200")
                } else {
                    ("local b = 200\n", "local b = 20\n")
                };
                assert_eq!(text.matches(kept).count(), 1, "{what}");
                assert!(!text.contains(gone), "{what}");
                assert!(!text.contains("local b = 2\n"), "{what}");
                assert!(text.contains("local d = 4") && text.contains("local e = 5"), "{what}");
                assert_eq!(text.matches("local a = 1").count(), 1, "{what}");
                assert_eq!(text.matches("local c = 3").count(), 1, "{what}");
                // The skipped patch says so.
                let skipper = if keep_a { "bmod:skip:1" } else { "amod:skip:1" };
                let w = &record(&records, skipper).warnings;
                assert!(
                    w.iter().any(|w| w.starts_with("hunk_skipped:0: the change to 'local b = 2' is skipped")),
                    "{what}: {w:?}"
                );
            }
        }
    }
}

/// A skipped change lets the patch's other edits apply: its insert lands below `c`, and the
/// kept lines stay as the game wrote them.
#[test]
fn a_skipped_change_lets_the_other_edits_apply() {
    let t = table(
        vec![a_patch(), skipping(b_patch(), 0, Some("amod:skip:1"))],
        StackMode::Legacy,
    );
    let (text, _) = run(&t, &src());
    assert!(text.contains("  local c = 3\n"), "{text}");
    let c = text.find("local c = 3").expect("c");
    assert!(text.find("local e = 5").expect("e") > c, "{text}");
}

/// The skipped line matches only the game's text or the kept patch's line: another patch's
/// change there is not the kept one, so the place is lost.
#[test]
fn a_skipped_line_matches_only_the_game_s_text_or_the_kept_patch_s() {
    let c_patch = at_patch("cmod:skip:1", "local b = 2", "local b = 7");
    let t = table(
        vec![
            c_patch,
            a_patch(),
            skipping(b_patch(), 0, Some("amod:skip:1")),
        ],
        StackMode::Legacy,
    );
    // C changed `b` first, so A misses too; B's skipped line is C's, not A's.
    let (text, records) = run(&t, &src());
    assert_eq!(
        record(&records, "bmod:skip:1").status,
        PatchOutcomeStatus::ZeroMatches,
        "{text}"
    );
    // With no kept patch named, any other patch's line stands in.
    let t = table(
        vec![
            at_patch("cmod:skip:1", "local b = 2", "local b = 7"),
            skipping(b_patch(), 0, None),
        ],
        StackMode::Legacy,
    );
    let (text, records) = run(&t, &src());
    assert!(applied(&records, "bmod:skip:1"), "{text}");
    assert!(text.contains("local b = 7") && text.contains("local e = 5"), "{text}");
    // A kept patch that is not loaded: only the game's text matches.
    let t = table(
        vec![
            at_patch("cmod:skip:1", "local b = 2", "local b = 7"),
            skipping(b_patch(), 0, Some("zmod:gone")),
        ],
        StackMode::Legacy,
    );
    let (_, records) = run(&t, &src());
    assert_eq!(
        record(&records, "bmod:skip:1").status,
        PatchOutcomeStatus::ZeroMatches
    );
}

/// A 3-line Halt-like window with its middle line skipped: a place that only matches with the
/// middle line relaxed is not one of the patch's places in the game's file, so it is never
/// matched, and the match count stays at the game's. In g() another patch owns the middle
/// line, so a relaxed matcher used everywhere would take g() as a second place.
#[test]
fn a_relaxed_line_never_adds_a_place() {
    let src = "function f()\n  x = 1\n  y = 2\n  z = 3\nend\nfunction g()\n  x = 1\n  w = 9\n  z = 3\nend\n";
    let halt = at_patch("bmod:halt:1", "x = 1\ny = 2\nz = 3", "x = 1\ny = 5\nz = 3\nq = 0");
    // Keep A's change to `y` (A: y = 2 -> y = 8).
    let a = at_patch("amod:halt:1", "y = 2", "y = 8");
    // C owns g()'s middle line.
    let c = at_patch("cmod:halt:1", "w = 9", "w = 10");
    for keep in [Some("amod:halt:1"), None] {
        let t = table(
            vec![a.clone(), c.clone(), skipping(halt.clone(), 0, keep)],
            StackMode::Legacy,
        );
        let (text, records) = run(&t, src);
        let r = record(&records, "bmod:halt:1");
        assert_eq!((r.status, r.matched_count), (PatchOutcomeStatus::Applied, 1), "{text}");
        assert_eq!(text.matches("q = 0").count(), 1, "{text}");
        let g = text.find("function g").expect("g");
        assert!(text.find("q = 0").expect("q") < g, "{text}");
        assert!(text.contains("  w = 10\n  z = 3\nend\n"), "{text}");
    }
    // Without A, the skipped line is the game's: one place, as before.
    let t = table(vec![skipping(halt, 0, Some("amod:halt:1"))], StackMode::Legacy);
    let (text, records) = run(&t, src);
    assert_eq!(record(&records, "bmod:halt:1").matched_count, 1, "{text}");
    assert!(text.contains("  y = 2\n") && !text.contains("y = 5"), "{text}");
}

/// The kept patch replaced one line with two: the second is its own new line, which the
/// window steps over.
#[test]
fn a_kept_change_of_one_line_into_two_still_matches() {
    let a = at_patch("amod:skip:1", "local b = 2", "local b = 20\nlocal bb = 21");
    let t = table(
        vec![a, skipping(b_patch(), 0, Some("amod:skip:1"))],
        StackMode::Dependencies,
    );
    let (text, records) = run(&t, &src());
    assert!(applied(&records, "bmod:skip:1"), "{text}");
    assert!(text.contains("local bb = 21\n  local c = 3\n  local e = 5\n"), "{text}");
}

/// Task 13b: a third patch's `at` keeps the kept patch's changed line. The keeper owns the
/// line now (for rungs), but it is still the kept patch's change, so the skipped line finds it
/// under both line rule settings.
#[test]
fn a_kept_change_that_another_at_keeps_still_matches() {
    let q = at_patch("cmod:skip:1", "local b = 20", "local b = 20\nlocal q = 9");
    for mode in [StackMode::Legacy, StackMode::Dependencies] {
        for rules in [false, true] {
            let mut t = table(
                vec![
                    a_patch(),
                    q.clone(),
                    skipping(b_patch(), 0, Some("amod:skip:1")),
                ],
                mode,
            );
            t.line_rules_v2 = rules;
            let (text, records) = run(&t, &src());
            let what = format!("{mode:?}, rules {rules}: {text}");
            assert!(applied(&records, "cmod:skip:1"), "{what}");
            assert!(applied(&records, "bmod:skip:1"), "{what}");
            assert_eq!(text.matches("local b = 20\n").count(), 1, "{what}");
            assert!(!text.contains("local b = 200"), "{what}");
            assert!(text.contains("local q = 9") && text.contains("local e = 5"), "{what}");
        }
    }
}

/// A skip of an edit the patch does not have changes nothing.
#[test]
fn a_skip_of_an_unknown_edit_does_nothing() {
    let mut b = b_patch();
    b.qm.skip_hunks.push(HunkSkip {
        ordinal: 9,
        vanilla: String::new(),
        keep: None,
    });
    b.qm.apply_as = Some(ApplyAs::Edits);
    let t = table(vec![b], StackMode::Legacy);
    let (text, records) = run(&t, &src());
    assert!(applied(&records, "bmod:skip:1"), "{text}");
    assert!(text.contains("local b = 200"), "{text}");
    assert!(record(&records, "bmod:skip:1").warnings.is_empty());
}
