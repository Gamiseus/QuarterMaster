//! Split windows (D48), `at` stacking and `at` rungs in edits mode (Task 20).
use super::*;
use crate::handshake::{LaunchConfig, PatchExecutionRecord, PatchOutcomeStatus, StackMode};
use crate::lenient::Lenient;
use crate::patch::anchor::{AnchorSource, Edge, Rung};
use crate::patch::parity::{check_dll_parity, check_linemap, check_parity};
use crate::patch::plan::ModMeta;
use crate::patch::regex::RegexPatch;
use crate::patch::table::PatchTable;
use crate::patch::{OnFailurePolicy, Patch};
use std::path::PathBuf;

// --- find_split_windows ---------------------------------------------------------------------

fn wm(pattern: &[&str]) -> Vec<WildMatch> {
    pattern.iter().map(|l| WildMatch::new(l)).collect()
}

fn split(lines: &[&str], pattern: &[&str], skip: &[usize]) -> Vec<Vec<usize>> {
    find_split_windows(lines, &wm(pattern), None, |l| skip.contains(&l), |_| true)
        .into_iter()
        .map(|w| w.lines)
        .collect()
}

#[test]
fn with_nothing_to_skip_windows_are_today_s() {
    let lines = ["a", "b", "a", "b", "x", "a"];
    let today = find_windows(&lines, &wm(&["a", "b"]), None, |_, _| true);
    let split = find_split_windows(&lines, &wm(&["a", "b"]), None, |_| false, |_| true);
    assert_eq!(today, split);
    assert_eq!(
        today,
        [
            Window::contiguous(0, 2, String::new()),
            Window::contiguous(2, 2, String::new())
        ]
    );
}

#[test]
fn a_window_skips_lines_it_may_skip_and_ends_on_a_matched_line() {
    let lines = ["a", "f1", "f2", "b", "c"];
    assert_eq!(split(&lines, &["a", "b"], &[1, 2]), [vec![0, 3]]);
    let w = find_split_windows(
        &lines,
        &wm(&["a", "b"]),
        None,
        |l| l == 1 || l == 2,
        |_| true,
    );
    assert_eq!((w[0].line, w[0].len), (0, 4));
    // A line that may not be skipped ends the window.
    assert!(split(&lines, &["a", "b"], &[1]).is_empty());
}

/// A skippable line that reads like a window line is matched first; when the rest then fails
/// the matcher backs up and skips it.
#[test]
fn a_foreign_line_equal_to_a_window_line_backtracks() {
    let lines = ["a", "b", "z", "b", "c"];
    assert_eq!(split(&lines, &["a", "b", "c"], &[1, 2]), [vec![0, 3, 4]]);
    // Of two windows from one start, the shorter wins.
    let lines = ["a", "b", "b"];
    assert_eq!(split(&lines, &["a", "b"], &[1]), [vec![0, 1]]);
}

/// 64 skippable `end` lines between the window's lines: the backtracking is bounded and
/// nothing panics.
#[test]
fn a_long_foreign_run_is_bounded() {
    let mut lines = vec!["a"];
    lines.extend(std::iter::repeat_n("end", 64));
    lines.push("c");
    let skip: Vec<usize> = (1..=64).collect();
    let started = std::time::Instant::now();
    assert!(split(&lines, &["a", "end", "end", "b"], &skip).is_empty());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    // A plain run of skipped lines is no backtracking: the window still matches.
    assert_eq!(
        split(&lines, &["a", "c"], &skip),
        [vec![0, lines.len() - 1]]
    );
    assert_eq!(split(&lines, &["a", "end", "c"], &skip), [vec![0, 1, 65]]);
}

#[test]
fn split_ok_needs_changes_on_lines_with_nothing_between() {
    let change = [Hunk::Keep { p: 0, q: 0 }, Hunk::Change { p: 1..3, q: 1..2 }];
    assert!(edits::split_ok(&change, &[4, 5, 6]));
    assert!(edits::split_ok(&change, &[4, 8, 9]));
    assert!(!edits::split_ok(&change, &[4, 5, 7]));
    let inserts = [
        Hunk::Keep { p: 0, q: 0 },
        Hunk::Insert {
            after_p: Some(0),
            q: 1..2,
        },
        Hunk::Keep { p: 1, q: 2 },
    ];
    assert!(edits::split_ok(&inserts, &[0, 9]));
    assert!(edits::inserts_only(&inserts));
    assert!(!edits::inserts_only(&change));
}

// --- the engine -----------------------------------------------------------------------------

const FILE: &str = "includes/split.lua";
const X: &str = "        player.deployedTroops[i].targetx = player.deployedTroops[i].x";
const Y: &str = "        player.deployedTroops[i].targety = player.deployedTroops[i].y";
const IF: &str = "        if(spamTimer <= 0) then";

fn halt_src() -> String {
    format!("function halt()\n  for i = 1, n do\n{X}\n{Y}\n{IF}\n          spam()\n        end\n  end\nend\n")
}

fn pattern_patch(
    id: &str,
    position: InsertPosition,
    pattern: &str,
    payload: &str,
    apply_as: Option<ApplyAs>,
) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: pattern.to_string(),
        position,
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
        apply_as: apply_as.map(Lenient::Ok),
        mod_id: id.split(':').next().map(str::to_string),
        disabled: false,
        qm: Default::default(),
    })
}

fn regex_patch(id: &str, pattern: &str, payload: &str) -> Patch {
    Patch::Regex(RegexPatch {
        target: Target::Single(FILE.to_string()),
        pattern: pattern.to_string(),
        position: InsertPosition::At,
        root_capture: None,
        payload: payload.to_string(),
        line_prepend: String::new(),
        times: None,
        verbose: false,
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
        mod_id: id.split(':').next().map(str::to_string),
        disabled: false,
        qm: Default::default(),
    })
}

/// GCore #16 (D47: Keep, Keep, Insert 3, Keep).
fn gcore_at(apply_as: Option<ApplyAs>) -> Patch {
    let new = "        player.deployedTroops[i]._pfGoalX = nil\n        player.deployedTroops[i]._savedOrderX = nil\n        player.deployedTroops[i]._savedOrderY = nil\n";
    pattern_patch(
        "gcore:halt:16",
        InsertPosition::At,
        &format!("{X}\n{Y}\n{IF}\n"),
        &format!("{X}\n{Y}\n{new}{IF}\n"),
        apply_as,
    )
}

/// TO #6: two lines after `targety`.
fn to_after() -> Patch {
    pattern_patch(
        "tactical_overhaul:halt:6",
        InsertPosition::After,
        Y.trim(),
        "        player.deployedTroops[i]._inCombatReaction = nil\n        player.deployedTroops[i].combatHaltTimer = 0\n",
        None,
    )
}

/// A table of `patches` (with priorities: lower runs first); `tactical_overhaul` requires
/// `gcore`.
fn table(patches: Vec<(Patch, i32)>, mode: StackMode) -> PatchTable {
    let mut t = PatchTable::default();
    t.targets.insert(FILE.to_string());
    for (p, prio) in patches {
        let m = p.mod_id().unwrap_or("x").to_string();
        t.patches
            .push((p, prio, PathBuf::from(format!("{m}/qm.toml"))));
    }
    let meta = |id: &str, ix: u32, requires: &[&str]| ModMeta {
        id: id.to_string(),
        folder: id.to_string(),
        manifest_ok: true,
        loader_index: ix,
        requires: requires.iter().map(|s| s.to_string()).collect(),
        load_after: requires.iter().map(|s| s.to_string()).collect(),
        ..ModMeta::default()
    };
    t.mods = vec![
        meta("gcore", 0, &[]),
        meta("tactical_overhaul", 1, &["gcore"]),
        meta("bmod", 2, &[]),
        meta("cmod", 3, &[]),
        meta("amod", 4, &["bmod"]),
    ];
    t.stack_mode = mode;
    t
}

/// The text and records of `src`, after checking that every entry point, the DLL and the
/// line map agree.
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

fn pos(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("{needle} not in {text}"))
}

/// §10.1 in both orders and both modes: in edits mode GCore's Halt patch always applies,
/// its kept lines stay as the game wrote them, and Dependencies puts GCore's block above TO's.
#[test]
fn the_halt_pair_applies_in_either_order() {
    let src = halt_src();
    for (gcore_prio, to_prio) in [(5, 50), (50, 5)] {
        for mode in [StackMode::Legacy, StackMode::Dependencies] {
            let t = table(
                vec![
                    (gcore_at(Some(ApplyAs::Edits)), gcore_prio),
                    (to_after(), to_prio),
                ],
                mode,
            );
            let (text, records) = run(&t, &src);
            let what = format!("gcore {gcore_prio}, {mode:?}");
            for r in &records {
                assert_eq!(
                    (r.status, r.matched_count),
                    (PatchOutcomeStatus::Applied, 1),
                    "{what}: {}",
                    r.patch_id
                );
                assert!(r.warnings.is_empty(), "{what}: {:?}", r.warnings);
            }
            assert!(text.contains(&format!("{X}\n{Y}\n")), "{what}: {text}");
            assert_eq!(text.matches("targetx").count(), 1, "{what}: {text}");
            let (gcore, to) = (pos(&text, "_savedOrderY"), pos(&text, "_inCombatReaction"));
            assert!(pos(&text, Y) < gcore && pos(&text, Y) < to, "{what}");
            assert!(gcore < pos(&text, IF) && to < pos(&text, IF), "{what}");
            // Legacy: the newer block next to the line. Dependencies: GCore above TO.
            let gcore_above = match mode {
                StackMode::Legacy => gcore_prio > to_prio,
                _ => true,
            };
            assert_eq!(gcore < to, gcore_above, "{what}: {text}");
        }
    }
    // TO first: GCore's three lines go right after `targety`, above TO's lines, and the
    // `if` stays below both.
    let t = table(
        vec![(gcore_at(Some(ApplyAs::Edits)), 50), (to_after(), 5)],
        StackMode::Dependencies,
    );
    let (text, records) = run(&t, &src);
    let want = format!(
        "{Y}\n                player.deployedTroops[i]._pfGoalX = nil\n                player.deployedTroops[i]._savedOrderX = nil\n                player.deployedTroops[i]._savedOrderY = nil\n                player.deployedTroops[i]._inCombatReaction = nil\n                player.deployedTroops[i].combatHaltTimer = 0\n{IF}\n"
    );
    assert!(text.contains(&want), "{text}");
    assert!(record(&records, "gcore:halt:16").stack_moves.is_empty());
}

/// A literal `at` keeps strict windows (D8): with TO's lines inside its window it finds no
/// match, and nothing of TO's is removed.
#[test]
fn a_literal_at_does_not_match_a_split_window() {
    let src = halt_src();
    let t = table(
        vec![(gcore_at(None), 50), (to_after(), 5)],
        StackMode::Dependencies,
    );
    let (text, records) = run(&t, &src);
    let r = record(&records, "gcore:halt:16");
    assert_eq!(r.status, PatchOutcomeStatus::ZeroMatches);
    assert!(text.contains("_inCombatReaction") && !text.contains("_pfGoalX"));
}

/// The window skips TO's new line `b`, which reads like a window line: the matcher matches it,
/// fails, backs up and skips it.
#[test]
fn a_foreign_line_equal_to_a_window_line_is_skipped_on_backtrack() {
    let src = "a()\nb()\nc()\n";
    let to = pattern_patch(
        "tactical_overhaul:t:1",
        InsertPosition::After,
        "a()",
        "b()\nz()\n",
        None,
    );
    let a = pattern_patch(
        "gcore:t:1",
        InsertPosition::At,
        "a()\nb()\nc()",
        "a()\nb()\nnew()\nc()",
        Some(ApplyAs::Edits),
    );
    let (text, records) = run(&table(vec![(a, 50), (to, 5)], StackMode::Legacy), src);
    assert_eq!(text, "a()\nb()\nz()\nb()\nnew()\nc()\n");
    assert_eq!(
        record(&records, "gcore:t:1").status,
        PatchOutcomeStatus::Applied
    );
}

/// Only another patch's new lines are skipped. A line another patch changed inside (Modified)
/// is never skipped, so the window does not match.
#[test]
fn a_modified_line_is_never_skipped() {
    let src = "a()\nmid()\nb()\n";
    let a = || {
        pattern_patch(
            "gcore:m:1",
            InsertPosition::At,
            "a()\nb()",
            "a()\nb()\nnew()",
            Some(ApplyAs::Edits),
        )
    };
    let touch = regex_patch("cmod:m:1", r"mid\(\)", "mid2()");
    let (text, records) = run(&table(vec![(a(), 50), (touch, 5)], StackMode::Legacy), src);
    assert_eq!(text, "a()\nmid2()\nb()\n");
    assert_eq!(
        record(&records, "gcore:m:1").status,
        PatchOutcomeStatus::ZeroMatches
    );
    // The same place with a new line of another patch in the middle matches.
    let src = "a()\nb()\n";
    let insert = pattern_patch("cmod:m:2", InsertPosition::After, "a()", "mid()\n", None);
    let (text, _) = run(&table(vec![(a(), 50), (insert, 5)], StackMode::Legacy), src);
    assert_eq!(text, "a()\nmid()\nb()\nnew()\n");
}

/// A change hunk over lines another patch's code sits between is refused for that site with
/// `edits_refused:split_change`; nothing is written there and the other patch's lines stay.
#[test]
fn a_split_change_is_refused_and_the_foreign_lines_stay() {
    let src = "a()\nb()\nc()\nd()\n";
    let insert = pattern_patch(
        "cmod:s:1",
        InsertPosition::After,
        "b()",
        "foreign()\n",
        None,
    );
    let a = pattern_patch(
        "gcore:s:1",
        InsertPosition::At,
        "a()\nb()\nc()",
        "a()\nB()\nC()",
        Some(ApplyAs::Edits),
    );
    let (text, records) = run(
        &table(
            vec![(a.clone(), 50), (insert.clone(), 5)],
            StackMode::Legacy,
        ),
        src,
    );
    assert_eq!(text, "a()\nb()\nforeign()\nc()\nd()\n");
    let r = record(&records, "gcore:s:1");
    assert_eq!(r.status, PatchOutcomeStatus::ZeroMatches);
    assert_eq!(r.warnings.len(), 2, "{:?}", r.warnings);
    assert!(
        r.warnings.iter().any(|w| w.starts_with(&format!(
            "edits_refused:split_change: the match at line 1 of '{FILE}' is skipped: "
        ))),
        "{:?}",
        r.warnings
    );
    // A second, unsplit place still runs: the status follows from what is left.
    let src = "a()\nb()\nc()\nx()\na()\nb()\nc()\n";
    let Patch::Pattern(mut once) = insert else {
        unreachable!()
    };
    once.times = Some(1);
    let (text, records) = run(
        &table(vec![(a, 50), (Patch::Pattern(once), 5)], StackMode::Legacy),
        src,
    );
    assert_eq!(text, "a()\nb()\nforeign()\nc()\nx()\na()\nB()\nC()\n");
    let r = record(&records, "gcore:s:1");
    assert_eq!(
        (r.status, r.matched_count),
        (PatchOutcomeStatus::Applied, 1)
    );
}

/// Insert hunks may span a split: inserts after several kept lines each go after their own
/// kept line, and never past the next matched line.
#[test]
fn inserts_between_split_lines_stay_between_them() {
    let src = "a()\nb()\nc()\n";
    let insert = pattern_patch(
        "cmod:i:1",
        InsertPosition::After,
        "a()",
        "f1()\nf2()\n",
        None,
    );
    let a = pattern_patch(
        "gcore:i:1",
        InsertPosition::At,
        "a()\nb()\nc()",
        "a()\nn1()\nb()\nn2()\nc()",
        Some(ApplyAs::Edits),
    );
    for mode in [
        StackMode::Legacy,
        StackMode::Dependencies,
        StackMode::LoadOrder,
    ] {
        let (text, records) = run(
            &table(vec![(a.clone(), 50), (insert.clone(), 5)], mode),
            src,
        );
        let r = record(&records, "gcore:i:1");
        assert_eq!(r.status, PatchOutcomeStatus::Applied, "{mode:?}");
        let want = match mode {
            // Legacy and Dependencies (unrelated mods): next to the line.
            StackMode::LoadOrder => "a()\nf1()\nf2()\nn1()\nb()\nn2()\nc()\n",
            _ => "a()\nn1()\nf1()\nf2()\nb()\nn2()\nc()\n",
        };
        assert_eq!(text, want, "{mode:?}");
    }
}

/// A before/after patch in edits mode finds its place again when another patch split its
/// lines; without edits mode it does not match (D48).
#[test]
fn a_multi_line_after_patch_in_edits_mode_finds_a_split_site() {
    let src = "a()\nb()\nc()\n";
    let insert = pattern_patch("cmod:b:1", InsertPosition::After, "a()", "f()\n", None);
    let after = |apply_as| {
        pattern_patch(
            "gcore:b:1",
            InsertPosition::After,
            "a()\nb()",
            "new()\n",
            apply_as,
        )
    };
    let (text, records) = run(
        &table(
            vec![(after(None), 50), (insert.clone(), 5)],
            StackMode::Legacy,
        ),
        src,
    );
    assert_eq!(text, "a()\nf()\nb()\nc()\n");
    assert_eq!(
        record(&records, "gcore:b:1").status,
        PatchOutcomeStatus::ZeroMatches
    );
    let (text, records) = run(
        &table(
            vec![(after(Some(ApplyAs::Edits)), 50), (insert.clone(), 5)],
            StackMode::Legacy,
        ),
        src,
    );
    assert_eq!(text, "a()\nf()\nb()\nnew()\nc()\n");
    assert_eq!(
        record(&records, "gcore:b:1").status,
        PatchOutcomeStatus::Applied
    );
    // Before uses the first matched line.
    let before = pattern_patch(
        "gcore:b:2",
        InsertPosition::Before,
        "a()\nb()",
        "new()\n",
        Some(ApplyAs::Edits),
    );
    let (text, _) = run(
        &table(vec![(before, 50), (insert, 5)], StackMode::Legacy),
        src,
    );
    assert_eq!(text, "new()\na()\nf()\nb()\nc()\n");
}

/// A site in a long string with a split window is skipped, never run literal (it would remove
/// the other patch's lines).
#[test]
fn a_split_site_in_a_long_string_is_skipped() {
    let src = "s = [[\na()\nb()\n]]\n";
    let insert = pattern_patch("cmod:l:1", InsertPosition::After, "a()", "f()\n", None);
    let a = pattern_patch(
        "gcore:l:1",
        InsertPosition::At,
        "a()\nb()",
        "a()\nx()\nb()",
        Some(ApplyAs::Edits),
    );
    let (text, records) = run(&table(vec![(a, 50), (insert, 5)], StackMode::Legacy), src);
    assert_eq!(text, "s = [[\na()\nf()\nb()\n]]\n");
    let r = record(&records, "gcore:l:1");
    assert_eq!(r.status, PatchOutcomeStatus::ZeroMatches);
    assert!(
        r.warnings
            .iter()
            .any(|w| w.starts_with("edits_refused:long_string: ") && w.contains("is skipped")),
        "{:?}",
        r.warnings
    );
}

// --- rungs on an edits-mode `at` (§4.7) -----------------------------------------------------

fn with_rung(p: Patch, rung: Rung) -> Patch {
    let Patch::Pattern(mut x) = p else {
        unreachable!()
    };
    x.qm.rungs = vec![(
        rung,
        AnchorSource::Profile {
            fix_id: "fx_1".to_string(),
        },
    )];
    Patch::Pattern(x)
}

fn invalid_reasons(t: &PatchTable) -> Vec<(String, String)> {
    t.prepare()
        .report()
        .invalid_rungs
        .iter()
        .map(|r| (r.reason.clone(), r.detail.clone()))
        .collect()
}

/// A rung on an `at` patch is valid only in edits mode with only kept and new lines, and
/// never at an edge or with its own position.
#[test]
fn an_at_takes_rungs_only_as_inserts_in_edits_mode() {
    let within = Rung {
        within: Some("bmod:copy:1".to_string()),
        b_site: Some(0),
        ..Rung::default()
    };
    let b = pattern_patch(
        "bmod:copy:1",
        InsertPosition::After,
        "z()",
        "a()\nb()\n",
        None,
    );
    let at = |payload: &str, apply_as| {
        pattern_patch(
            "amod:at:1",
            InsertPosition::At,
            "a()\nb()",
            payload,
            apply_as,
        )
    };
    let check = |a: Patch, want: Option<&str>| {
        let t = table(vec![(b.clone(), 5), (a, 50)], StackMode::Legacy);
        let got = invalid_reasons(&t);
        match want {
            None => assert!(got.is_empty(), "{got:?}"),
            Some(w) => {
                assert_eq!(got.len(), 1, "{got:?}");
                assert_eq!(got[0].0, "at_patch");
                assert!(got[0].1.contains(w), "{got:?}");
            }
        }
    };
    check(
        with_rung(at("a()\nnew()\nb()", None), within.clone()),
        Some("replaces lines"),
    );
    check(
        with_rung(at("a()\nnew()\nb()", Some(ApplyAs::Edits)), within.clone()),
        None,
    );
    check(
        with_rung(at("a()\nB()", Some(ApplyAs::Edits)), within.clone()),
        Some("replaces lines"),
    );
    let edge = Rung {
        edge: Some(Edge::End),
        ..within.clone()
    };
    check(
        with_rung(at("a()\nnew()\nb()", Some(ApplyAs::Edits)), edge),
        Some("edge"),
    );
    let moved = Rung {
        position: Some(InsertPosition::After),
        ..within
    };
    check(
        with_rung(at("a()\nnew()\nb()", Some(ApplyAs::Edits)), moved),
        Some("its own lines"),
    );
}

/// A within rung with no line to find puts an inserts-only `at` on its own lines inside B's
/// copy of them, instead of on the game's lines.
#[test]
fn an_inserts_only_at_follows_b_s_copy_of_its_lines() {
    let src = "a()\nb()\nz()\n";
    let b = pattern_patch(
        "bmod:copy:1",
        InsertPosition::After,
        "z()",
        "a()\nb()\n",
        None,
    );
    let rung = Rung {
        within: Some("bmod:copy:1".to_string()),
        b_site: Some(0),
        ..Rung::default()
    };
    let a = |apply_as| {
        with_rung(
            pattern_patch(
                "amod:at:1",
                InsertPosition::At,
                "a()\nb()",
                "a()\nnew()\nb()",
                apply_as,
            ),
            rung.clone(),
        )
    };
    // B runs first through the rung's Required edge, although A's priority is lower.
    let t = table(
        vec![(a(Some(ApplyAs::Edits)), 1), (b.clone(), 5)],
        StackMode::Legacy,
    );
    let (text, records) = run(&t, src);
    assert_eq!(text, "a()\nb()\nz()\na()\nnew()\nb()\n");
    let r = record(&records, "amod:at:1");
    assert_eq!(r.status, PatchOutcomeStatus::Applied);
    let sites = &r.anchoring.as_ref().expect("anchoring").sites;
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].rung, Some(0));
    // A literal `at` ignores the rung and keeps today's run.
    let t = table(vec![(a(None), 1), (b, 5)], StackMode::Legacy);
    let (text, records) = run(&t, src);
    assert_eq!(text, "a()\nnew()\nb()\nz()\na()\nb()\n");
    assert!(record(&records, "amod:at:1").anchoring.is_none());
}

/// A scoped rung window of an A in edits mode skips other patches' new lines inside B's code
/// (D48); in literal mode the foreign line rejects it (D8) and A uses its own line.
#[test]
fn a_scoped_rung_window_skips_foreign_lines_in_edits_mode() {
    let src = "z()\nq()\n";
    let b = pattern_patch(
        "bmod:p:1",
        InsertPosition::After,
        "z()",
        "p1()\np2()\n",
        None,
    );
    let c = pattern_patch("cmod:p:1", InsertPosition::After, "p1()", "c()\n", None);
    let rung = Rung {
        within: Some("bmod:p:1".to_string()),
        pattern: Some("p1()\np2()".to_string()),
        exact: true,
        b_site: Some(0),
        ..Rung::default()
    };
    let a = |apply_as| {
        with_rung(
            pattern_patch(
                "amod:p:1",
                InsertPosition::After,
                "q()",
                "new()\n",
                apply_as,
            ),
            rung.clone(),
        )
    };
    let patches = |apply_as| vec![(b.clone(), 5), (c.clone(), 6), (a(apply_as), 50)];
    let (text, _) = run(
        &table(patches(Some(ApplyAs::Edits)), StackMode::Legacy),
        src,
    );
    assert_eq!(text, "z()\np1()\nc()\np2()\nnew()\nq()\n");
    let (text, records) = run(&table(patches(None), StackMode::Legacy), src);
    assert_eq!(text, "z()\np1()\nc()\np2()\nq()\nnew()\n");
    let sites = &record(&records, "amod:p:1")
        .anchoring
        .as_ref()
        .expect("anchoring")
        .sites;
    assert!(sites[0].fallback.is_some(), "{sites:?}");
}
