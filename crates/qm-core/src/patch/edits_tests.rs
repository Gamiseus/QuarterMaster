//! Edits mode (D46, D47): the normalization table and the engine runs.
use super::*;
use crate::handshake::{LaunchConfig, PatchExecutionRecord, PatchOutcomeStatus, StackMode};
use crate::lenient::Lenient;
use crate::patch::linemap::{LineKind, NO_PATCH};
use crate::patch::parity::{check_dll_parity, check_linemap, check_parity};
use crate::patch::plan::ModMeta;
use crate::patch::table::PatchTable;
use crate::patch::{InsertPosition, OnFailurePolicy, Patch, PatternPatch, Target};
use crate::prepare::simulate_buffer;
use std::path::PathBuf;

const X: &str = "        player.deployedTroops[i].targetx = player.deployedTroops[i].x";
const Y: &str = "        player.deployedTroops[i].targety = player.deployedTroops[i].y";
const IF: &str = "        if(spamTimer <= 0) then ";

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

// --- normalize ----------------------------------------------------------------------------

/// GCore #16: three lines re-emitted with three new ones between `targety` and `if`.
#[test]
fn gcore_halt_runs_as_three_insertions() {
    let p = [X, Y, IF];
    let q = [X, Y, "a = nil", "b = nil", "c = nil", IF];
    let h = normalize(&p, &q).unwrap();
    assert_eq!(
        h,
        [
            Hunk::Keep { p: 0, q: 0 },
            Hunk::Keep { p: 1, q: 1 },
            Hunk::Insert {
                after_p: Some(1),
                q: 2..5
            },
            Hunk::Keep { p: 2, q: 5 },
        ]
    );
    assert_eq!(counts(&h), (3, 3, 0, 0));
}

/// Indent and trailing spaces do not matter; the payload's own lines are compared as written.
#[test]
fn lines_compare_trimmed() {
    let h = normalize_patch("  a()\n  b()  \n", "a()\nx()\nb()").unwrap();
    assert_eq!(
        h,
        [
            Hunk::Keep { p: 0, q: 0 },
            Hunk::Insert {
                after_p: Some(0),
                q: 1..2
            },
            Hunk::Keep { p: 1, q: 2 },
        ]
    );
}

#[test]
fn repeated_end_is_ambiguous() {
    assert_eq!(
        normalize(&["end"], &["end", "x = 1", "end"]),
        Err(EditsRefusal::Ambiguous)
    );
    assert_eq!(
        normalize(&["f()", "end"], &["f()", "end", "end"]),
        Err(EditsRefusal::Ambiguous)
    );
    // One pairing only: not ambiguous.
    assert!(normalize(&["f()", "end"], &["f()", "g()", "end"]).is_ok());
}

/// 60 `end` lines against 120: the number of pairings is far beyond any integer, but the count
/// saturates at 2.
#[test]
fn sixty_end_lines_refuse_ambiguous_without_overflow() {
    let p = vec!["end"; 60];
    let q = vec!["end"; 120];
    assert_eq!(normalize(&p, &q), Err(EditsRefusal::Ambiguous));
    // The same 60 lines on both sides pair one way.
    let same = normalize(&p, &p).unwrap();
    assert_eq!(same.len(), 60);
    assert!(same.iter().all(|h| matches!(h, Hunk::Keep { .. })));
}

#[test]
fn a_wildcard_line_is_changed_never_kept() {
    assert_eq!(
        normalize(&["local x = *", "y()"], &["local x = 5", "y()"]).unwrap(),
        [Hunk::Change { p: 0..1, q: 0..1 }, Hunk::Keep { p: 1, q: 1 }]
    );
    // Even when the payload repeats it word for word.
    assert_eq!(
        normalize(&["a = b * c"], &["a = b * c"]).unwrap(),
        [Hunk::Change { p: 0..1, q: 0..1 }]
    );
    assert_eq!(
        normalize(&["what?"], &["what?"]).unwrap(),
        [Hunk::Change { p: 0..1, q: 0..1 }]
    );
}

#[test]
fn removed_and_leading_lines() {
    assert_eq!(
        normalize(&["a", "b", "c"], &["a", "c"]).unwrap(),
        [
            Hunk::Keep { p: 0, q: 0 },
            Hunk::Delete { p: 1..2 },
            Hunk::Keep { p: 2, q: 1 }
        ]
    );
    assert_eq!(
        normalize(&["a"], &["x", "a"]).unwrap(),
        [
            Hunk::Insert {
                after_p: None,
                q: 0..1
            },
            Hunk::Keep { p: 0, q: 1 }
        ]
    );
    // An empty payload is one empty line, as a literal run writes it.
    assert_eq!(
        normalize_patch("a", "").unwrap(),
        [Hunk::Change { p: 0..1, q: 0..1 }]
    );
}

/// GCore's `los` wrapper: the kept `function los(...)` header is followed by a line that opens
/// a block, so the old body would end up in the new function.
#[test]
fn the_los_wrapper_refuses_scope() {
    let payload = "function los(x1, x2, y1, y2)\n  if los_physics then\n    return los_physics.checkLOS(x1, x2, y1, y2)\n  end\n  return vanilla_los(x1, x2, y1, y2)\nend\n\nfunction vanilla_los(x1, x2, y1, y2)\n";
    assert_eq!(
        normalize_patch("function los(x1, x2, y1, y2)", payload),
        Err(EditsRefusal::Scope)
    );
    // A block that opens next to a kept line, or one that closes there.
    assert_eq!(
        normalize(&["a()", "b()"], &["a()", "if x then", "b()", "end"]),
        Err(EditsRefusal::Scope)
    );
    // New lines that only call or assign keep the block structure.
    assert!(normalize(&["a()", "b()"], &["a()", "if x then y() end", "b()"]).is_ok());
}

#[test]
fn a_kept_line_in_a_long_string_of_the_payload_refuses_long_string() {
    assert_eq!(
        normalize(&["x = 1"], &["s = [[", "x = 1", "]]"]),
        Err(EditsRefusal::LongString)
    );
    assert_eq!(
        normalize(&["x = 1"], &["x = 1", "--[[ note", "]]"]),
        Ok(vec![
            Hunk::Keep { p: 0, q: 0 },
            Hunk::Insert {
                after_p: Some(0),
                q: 1..3
            }
        ])
    );
}

#[test]
fn a_huge_comparison_refuses_too_large() {
    let p = vec!["x"; 2000];
    let q = vec!["y"; 2000];
    assert_eq!(normalize(&p, &q), Err(EditsRefusal::TooLarge));
}

#[test]
fn refused_warnings_name_the_reason() {
    let w = refused_warning(EditsRefusal::Ambiguous, None);
    assert!(w.starts_with("edits_refused:ambiguous: "), "{w}");
    assert!(w.contains("one replacement"), "{w}");
    let w = refused_warning(EditsRefusal::LongString, Some("the match at line 4".into()));
    assert!(
        w.starts_with("edits_refused:long_string: the match at line 4"),
        "{w}"
    );
}

// --- the engine ---------------------------------------------------------------------------

const FILE: &str = "includes/halt.lua";

fn halt_src() -> String {
    format!("function halt()\n  for i = 1, n do\n{X}\n{Y}\n{IF}\n          spam()\n        end\n  end\nend\n")
}

fn gcore_at(apply_as: Option<ApplyAs>) -> Patch {
    let pattern = format!("{X}\n{Y}\n{IF}\n");
    let payload = format!(
        "{X}\n{Y}\n        player.deployedTroops[i]._pfGoalX = nil\n        player.deployedTroops[i]._savedOrderX = nil\n{IF}\n"
    );
    pattern_patch(
        "gcore:halt:16",
        InsertPosition::At,
        &pattern,
        &payload,
        apply_as,
    )
}

fn to_after() -> Patch {
    pattern_patch(
        "tactical_overhaul:halt:6",
        InsertPosition::After,
        Y.trim(),
        "        player.deployedTroops[i]._inCombatReaction = nil\n",
        None,
    )
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

/// A table of `patches` (with priorities); `tactical_overhaul` requires `gcore`.
fn table(patches: Vec<(Patch, i32)>, mode: StackMode) -> PatchTable {
    let mut t = PatchTable::default();
    t.targets.insert(FILE.to_string());
    for (p, prio) in patches {
        let m = p.mod_id().unwrap_or("x").to_string();
        t.patches
            .push((p, prio, PathBuf::from(format!("{m}/qm.toml"))));
    }
    t.mods = vec![
        ModMeta {
            id: "gcore".to_string(),
            folder: "GCore".to_string(),
            manifest_ok: true,
            ..ModMeta::default()
        },
        ModMeta {
            id: "tactical_overhaul".to_string(),
            folder: "TacticalOverhaul".to_string(),
            manifest_ok: true,
            loader_index: 1,
            requires: ["gcore".to_string()].into(),
            load_after: ["gcore".to_string()].into(),
            ..ModMeta::default()
        },
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

fn trimmed(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).collect()
}

/// Kept lines keep the game's bytes; new lines get the literal bytes (the match's indent in
/// front of the payload line). The trimmed lines are the literal run's.
#[test]
fn edits_mode_keeps_vanilla_bytes_and_literal_new_lines() {
    let src = halt_src();
    let literal = table(vec![(gcore_at(None), 5)], StackMode::Legacy);
    let (lit, _) = run(&literal, &src);
    let edits = table(vec![(gcore_at(Some(ApplyAs::Edits)), 5)], StackMode::Legacy);
    let (text, records) = run(&edits, &src);
    assert_eq!(trimmed(&text), trimmed(&lit));
    let want = format!(
        "function halt()\n  for i = 1, n do\n{X}\n{Y}\n                player.deployedTroops[i]._pfGoalX = nil\n                player.deployedTroops[i]._savedOrderX = nil\n{IF}\n          spam()\n        end\n  end\nend\n"
    );
    assert_eq!(text, want);
    // Literal re-emits the kept lines with the indent twice.
    assert!(lit.contains(&format!("        {X}\n")), "{lit}");
    let r = &records[0];
    assert_eq!(
        (r.status, r.matched_count),
        (PatchOutcomeStatus::Applied, 1)
    );
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(r.regions.len(), 1, "{:?}", r.regions);
}

/// The kept lines are never touched: they stay buffer lines with no owner, and only the new
/// lines are the patch's.
#[test]
fn kept_lines_stay_untouched_in_the_line_map() {
    let src = halt_src();
    let t = table(vec![(gcore_at(Some(ApplyAs::Edits)), 5)], StackMode::Legacy);
    run(&t, &src);
    let prep = t.prepare();
    let (_, traced) = simulate_buffer(&t, &prep, &LaunchConfig::default(), FILE, &src).unwrap();
    let tags = traced.lines.tags();
    let text: Vec<&str> = lines(&traced.text);
    for (i, line) in text.iter().enumerate() {
        let tag = tags[i];
        if line.contains("_pfGoalX") || line.contains("_savedOrderX") {
            assert_eq!(tag.kind, LineKind::Inserted, "line {}", i + 1);
            assert_eq!(tag.owner, 0, "line {}", i + 1);
        } else {
            assert_eq!(tag.kind, LineKind::Vanilla, "line {}: {line}", i + 1);
            assert_eq!(tag.owner, NO_PATCH, "line {}", i + 1);
        }
    }
    // Literal mode keeps its Kept tagging.
    let lit = table(vec![(gcore_at(None), 5)], StackMode::Legacy);
    let (_, traced) =
        simulate_buffer(&lit, &lit.prepare(), &LaunchConfig::default(), FILE, &src).unwrap();
    assert!(traced.lines.tags().iter().any(|t| t.kind == LineKind::Kept));
}

/// GCore first, then TO's insert after `targety`: Legacy puts TO's line on top (next to the
/// line), Dependencies below GCore's block, because TO requires GCore.
#[test]
fn the_halt_pair_with_gcore_first() {
    let src = halt_src();
    let pair = || vec![(gcore_at(Some(ApplyAs::Edits)), 5), (to_after(), 50)];
    let (legacy, records) = run(&table(pair(), StackMode::Legacy), &src);
    assert!(records
        .iter()
        .all(|r| r.status == PatchOutcomeStatus::Applied));
    let at = |text: &str, needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle}"));
    assert!(
        at(&legacy, "_inCombatReaction") < at(&legacy, "_pfGoalX"),
        "{legacy}"
    );
    let (deps, records) = run(&table(pair(), StackMode::Dependencies), &src);
    assert!(records
        .iter()
        .all(|r| r.status == PatchOutcomeStatus::Applied));
    assert!(
        at(&deps, "_savedOrderX") < at(&deps, "_inCombatReaction"),
        "{deps}"
    );
    assert!(at(&deps, Y) < at(&deps, "_pfGoalX") && at(&deps, "_inCombatReaction") < at(&deps, IF));
}

/// An edits-mode insert stacks like a pattern insert: in Dependencies TO's new line goes below
/// GCore's code after the same kept line, because TO requires GCore.
#[test]
fn an_edits_insert_stacks_below_its_dependency() {
    let src = halt_src();
    let gcore = pattern_patch(
        "gcore:halt:17",
        InsertPosition::After,
        X.trim(),
        "gx = 1\n",
        None,
    );
    let to = pattern_patch(
        "tactical_overhaul:halt:7",
        InsertPosition::At,
        &format!("{X}\n"),
        &format!("{X}\n        tox = 1\n"),
        Some(ApplyAs::Edits),
    );
    let (text, records) = run(
        &table(
            vec![(gcore.clone(), 5), (to.clone(), 50)],
            StackMode::Dependencies,
        ),
        &src,
    );
    assert!(records
        .iter()
        .all(|r| r.status == PatchOutcomeStatus::Applied));
    assert!(
        text.contains(&format!("{X}\n        gx = 1\n                tox = 1\n")),
        "{text}"
    );
    let r = records
        .iter()
        .find(|r| r.patch_id == "tactical_overhaul:halt:7")
        .unwrap();
    assert_eq!(r.stack_moves.len(), 1, "{:?}", r.stack_moves);
    assert_eq!(
        (r.stack_moves[0].legacy_slot, r.stack_moves[0].slot),
        (0, 1)
    );
    // Legacy keeps today's place: the newest insert next to the line.
    let (text, records) = run(&table(vec![(gcore, 5), (to, 50)], StackMode::Legacy), &src);
    assert!(
        text.contains(&format!("{X}\n                tox = 1\n        gx = 1\n")),
        "{text}"
    );
    assert!(records.iter().all(|r| r.stack_moves.is_empty()));
}

/// A refused normalization runs literal and says why.
#[test]
fn a_refused_normalization_runs_literal_with_the_reason() {
    let src = "f()\nend\nend\n";
    let p = |a| {
        pattern_patch(
            "gcore:halt:1",
            InsertPosition::At,
            "f()\nend",
            "f()\nend\nend",
            a,
        )
    };
    let (lit, _) = run(&table(vec![(p(None), 5)], StackMode::Legacy), src);
    let (text, records) = run(
        &table(vec![(p(Some(ApplyAs::Edits)), 5)], StackMode::Legacy),
        src,
    );
    assert_eq!(text, lit);
    let w = &records[0].warnings;
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].starts_with("edits_refused:ambiguous: "), "{w:?}");
    assert_eq!(records[0].status, PatchOutcomeStatus::Applied);

    // With no match, the record still says why it would run literal.
    let (_, records) = run(
        &table(vec![(p(Some(ApplyAs::Edits)), 5)], StackMode::Legacy),
        "g()\n",
    );
    let w = &records[0].warnings;
    assert!(w.iter().any(|w| w.starts_with("edits_refused:ambiguous: ")), "{w:?}");
    assert_ne!(records[0].status, PatchOutcomeStatus::Applied);
}

/// A site inside a long string runs literal with `edits_refused:long_string`; the other site
/// still runs as edits.
#[test]
fn a_site_in_a_long_string_runs_literal() {
    let src = "a()\nb()\ns = [[\na()\nb()\n]]\n";
    let p = |a| {
        pattern_patch(
            "gcore:halt:2",
            InsertPosition::At,
            "a()\nb()",
            "a()\nx()\nb()",
            a,
        )
    };
    let (text, records) = run(
        &table(vec![(p(Some(ApplyAs::Edits)), 5)], StackMode::Legacy),
        src,
    );
    assert_eq!(text, "a()\nx()\nb()\ns = [[\na()\nx()\nb()\n]]\n");
    let w = &records[0].warnings;
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].starts_with("edits_refused:long_string: the match at line 4 of 'includes/halt.lua'"),
        "{w:?}"
    );
    assert_eq!(records[0].matched_count, 2);
}

/// Changes and removals replace only their own lines; several hunks in one site stay in order.
#[test]
fn changes_and_removals_in_one_site() {
    let src = "  a()\n  b()\n  c()\n  d()\n";
    let p = pattern_patch(
        "gcore:halt:3",
        InsertPosition::At,
        "a()\nb()\nc()\nd()",
        "a()\nB()\nc()\nnew()",
        Some(ApplyAs::Edits),
    );
    let lit = pattern_patch(
        "gcore:halt:3",
        InsertPosition::At,
        "a()\nb()\nc()\nd()",
        "a()\nB()\nc()\nnew()",
        None,
    );
    let (text, records) = run(&table(vec![(p, 5)], StackMode::Legacy), src);
    assert_eq!(text, "  a()\n  B()\n  c()\n  new()\n");
    assert_eq!(records[0].regions.len(), 2, "{:?}", records[0].regions);
    let (lit, _) = run(&table(vec![(lit, 5)], StackMode::Legacy), src);
    assert_eq!(trimmed(&text), trimmed(&lit));
    // A removal between kept lines, at two sites.
    let src = "a()\nb()\nc()\na()\nb()\nc()\n";
    let p = pattern_patch(
        "gcore:halt:4",
        InsertPosition::At,
        "a()\nb()\nc()",
        "a()\nc()",
        Some(ApplyAs::Edits),
    );
    let (text, records) = run(&table(vec![(p, 5)], StackMode::Legacy), src);
    assert_eq!(text, "a()\nc()\na()\nc()\n");
    assert_eq!(records[0].matched_count, 2);
}

/// An insert after the file's last line when it has no newline.
#[test]
fn an_insert_after_an_unterminated_last_line() {
    let src = "x()\na()";
    let p = pattern_patch(
        "gcore:halt:5",
        InsertPosition::At,
        "a()",
        "a()\nz()",
        Some(ApplyAs::Edits),
    );
    let (text, _) = run(&table(vec![(p, 5)], StackMode::Legacy), src);
    assert_eq!(text, "x()\na()\nz()");
}

/// The mode a patch runs in: the profile or a compat mod wins over the author (D13), and only
/// an `at` patch has one.
#[test]
fn the_asked_mode_layers_over_the_author() {
    let Patch::Pattern(mut p) = gcore_at(Some(ApplyAs::Edits)) else {
        unreachable!()
    };
    assert!(p.edits_mode());
    p.qm.apply_as = Some(ApplyAs::Literal);
    assert!(!p.edits_mode());
    p.apply_as = None;
    p.qm.apply_as = Some(ApplyAs::Edits);
    assert!(p.edits_mode());
    p.apply_as = Some(Lenient::Invalid {
        raw: serde_json::json!("sometimes"),
        error: "unknown variant".into(),
    });
    p.qm.apply_as = None;
    assert_eq!(p.asked_apply_as(), ApplyAs::Literal);
    let Patch::Pattern(after) = to_after() else {
        unreachable!()
    };
    let mut after = after;
    after.apply_as = Some(Lenient::Ok(ApplyAs::Edits));
    assert!(!after.edits_mode());
}

/// The author key reads from qm.toml.
#[test]
fn apply_as_reads_from_qm_toml() {
    let f: crate::patch::PatchFile = toml::from_str(
        "[manifest]\nversion = \"1.0.0\"\n[[patches]]\n[patches.pattern]\ntarget = \"main.lua\"\npattern = \"a\"\nposition = \"at\"\npayload = \"a\\nb\"\napply_as = \"edits\"\n[[patches]]\n[patches.pattern]\ntarget = \"main.lua\"\npattern = \"a\"\nposition = \"at\"\npayload = \"a\"\napply_as = \"often\"\n",
    )
    .unwrap();
    let Patch::Pattern(p) = &f.patches[0] else {
        unreachable!()
    };
    assert_eq!(p.apply_as, Some(Lenient::Ok(ApplyAs::Edits)));
    let Patch::Pattern(p) = &f.patches[1] else {
        unreachable!()
    };
    assert!(p.apply_as.as_ref().is_some_and(|a| a.error().is_some()));
}
