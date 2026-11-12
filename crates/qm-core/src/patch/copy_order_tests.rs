//! Copy-vs-pattern order (D54): copies run first unless an explicit order rule puts them after
//! a pattern or regex of the same file.
use super::*;
use crate::handshake::LaunchConfig;
use crate::patch::copy::CopyPosition;
use crate::patch::parity::{check_dll_parity, check_linemap, check_parity};
use crate::patch::{InsertPosition, PatternPatch, Target};

const MAIN: &str = "local a = 1\nlocal b = 2\n";
const AI: &str = "local ai = 1\n";

fn pattern(id: &str, target: &str, pattern: &str, payload: &str) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: pattern.to_string(),
        position: InsertPosition::After,
        target: Target::Single(target.to_string()),
        payload: payload.to_string(),
        match_indent: false,
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
    })
}

/// A main.lua prepend of `copied = 1`.
fn copy(id: &str, after: &[&str]) -> Patch {
    Patch::Copy(CopyPatch {
        position: CopyPosition::Prepend,
        target: Target::Single("main.lua".to_string()),
        sources: None,
        payload: Some("copied = 1".to_string()),
        id: Some(id.to_string()),
        name: None,
        priority: None,
        before: Vec::new(),
        after: after.iter().map(|s| s.to_string()).collect(),
        on_failure: OnFailurePolicy::Warn,
        mod_id: id.split(':').next().map(str::to_string),
        disabled: false,
        qm: Default::default(),
        contents: Vec::new(),
    })
}

/// A pattern that patches the copied line.
fn sees_copy(id: &str) -> Patch {
    pattern(
        id,
        "main.lua",
        "copied = 1",
        &format!("-- {id} saw the copy"),
    )
}

fn table(patches: Vec<(Patch, Priority)>) -> PatchTable {
    let mut t = PatchTable::default();
    t.targets.insert("main.lua".to_string());
    t.targets.insert("includes/ai.lua".to_string());
    for (p, prio) in patches {
        let m = p.mod_id().unwrap_or("x").to_string();
        t.patches
            .push((p, prio, PathBuf::from(format!("{m}/qm.toml"))));
    }
    t
}

fn rule(patch: &str, before: &[&str]) -> OrderRule {
    OrderRule {
        patch: patch.to_string(),
        before: before.iter().map(|s| s.to_string()).collect(),
        after: Vec::new(),
    }
}

/// Text and records of main.lua, after checking that every entry point, the DLL and the line
/// map agree.
fn run(t: &PatchTable, file: &str, text: &str) -> (String, Vec<PatchExecutionRecord>) {
    check_parity(t, file, text).unwrap_or_else(|e| panic!("parity: {e}"));
    check_dll_parity(t, file, text, &LaunchConfig::default())
        .unwrap_or_else(|e| panic!("DLL parity: {e}"));
    check_linemap(t, &LaunchConfig::default(), file, text)
        .unwrap_or_else(|e| panic!("line map: {e}"));
    let (text, _, records) = t.apply_patches_offline(file, text).expect("no abort");
    (text, records)
}

fn ids(records: &[PatchExecutionRecord]) -> Vec<&str> {
    records.iter().map(|r| r.patch_id.as_str()).collect()
}

fn rec<'a>(records: &'a [PatchExecutionRecord], id: &str) -> &'a PatchExecutionRecord {
    records
        .iter()
        .find(|r| r.patch_id == id)
        .unwrap_or_else(|| panic!("no record for {id}"))
}

fn ordered_warnings(r: &PatchExecutionRecord) -> Vec<&str> {
    r.warnings
        .iter()
        .map(String::as_str)
        .filter(|w| w.starts_with(COPY_PASS_ORDERED))
        .collect()
}

/// With no rule the copy runs first, so a pattern can patch the copied code (legacy).
#[test]
fn with_no_rule_copies_run_first() {
    let t = table(vec![(sees_copy("y:p1"), 50), (copy("x:c", &[]), 10)]);
    let (text, records) = run(&t, "main.lua", MAIN);
    assert_eq!(text, format!("copied = 1\n-- y:p1 saw the copy\n{MAIN}"));
    assert_eq!(ids(&records), ["x:c", "y:p1"]);
    assert_eq!(rec(&records, "y:p1").status, PatchOutcomeStatus::Applied);
    assert!(ordered_warnings(rec(&records, "x:c")).is_empty());
}

/// A rule "pattern before copy" by id runs the copy right after that pattern, which no longer
/// sees the copied text. The record says why.
#[test]
fn a_rule_by_id_runs_the_copy_right_after_its_pattern() {
    let mut t = table(vec![(sees_copy("y:p1"), 50), (copy("x:c", &[]), 10)]);
    t.order_rules.push(rule("y:p1", &["x:c"]));
    let (text, records) = run(&t, "main.lua", MAIN);
    assert_eq!(text, format!("copied = 1\n{MAIN}"));
    assert_eq!(ids(&records), ["y:p1", "x:c"]);
    assert_eq!(
        rec(&records, "y:p1").status,
        PatchOutcomeStatus::ZeroMatches
    );
    let c = rec(&records, "x:c");
    assert_eq!(c.status, PatchOutcomeStatus::Applied);
    assert_eq!(
        ordered_warnings(c),
        ["copy_pass_ordered:profile: runs after 'y:p1' because of a profile order rule"]
    );

    // The same from the copy's own `after` list and from a compat rule.
    let t = table(vec![(sees_copy("y:p1"), 50), (copy("x:c", &["y:p1"]), 10)]);
    let (_, records) = run(&t, "main.lua", MAIN);
    assert_eq!(ids(&records), ["y:p1", "x:c"]);
    assert!(ordered_warnings(rec(&records, "x:c"))[0].starts_with("copy_pass_ordered:author:"));
    let mut t = table(vec![(sees_copy("y:p1"), 50), (copy("x:c", &[]), 10)]);
    t.compat_rules
        .push((rule("y:p1", &["x:c"]), "fixpack".to_string()));
    let (_, records) = run(&t, "main.lua", MAIN);
    assert_eq!(ids(&records), ["y:p1", "x:c"]);
    assert!(ordered_warnings(rec(&records, "x:c"))[0].starts_with("copy_pass_ordered:compat:"));
}

/// A pattern of the copy's mod that patches the copied text, and that no rule orders before the
/// copy, still runs after it, even when it would run before the rule's pattern by priority.
#[test]
fn a_pattern_not_ordered_before_the_copy_still_sees_it() {
    let mut t = table(vec![
        (sees_copy("x:third"), 10),
        (copy("x:c", &[]), 10),
        (sees_copy("y:p1"), 50),
        (sees_copy("z:late"), 90),
    ]);
    t.order_rules.push(rule("y:p1", &["x:c"]));
    let order: Vec<&str> = t
        .ordered()
        .order
        .iter()
        .map(|&i| t.patches[i].0.id().unwrap_or_default())
        .collect();
    assert_eq!(order, ["x:third", "y:p1", "x:c", "z:late"]);
    let (text, records) = run(&t, "main.lua", MAIN);
    assert_eq!(ids(&records), ["y:p1", "x:c", "x:third", "z:late"]);
    assert_eq!(
        rec(&records, "y:p1").status,
        PatchOutcomeStatus::ZeroMatches
    );
    for id in ["x:third", "z:late"] {
        assert_eq!(
            rec(&records, id).status,
            PatchOutcomeStatus::Applied,
            "{id}"
        );
    }
    // Legacy slots: the newest insert after a line goes on top.
    assert_eq!(
        text,
        format!("copied = 1\n-- z:late saw the copy\n-- x:third saw the copy\n{MAIN}")
    );
}

/// `mod_id:*` rules (an existing Order fix, a compat rule) never move a copy: output and records
/// stay as before D54.
#[test]
fn wildcard_rules_leave_copies_in_the_first_pass() {
    let patches = || {
        vec![
            (sees_copy("x:third"), 10),
            (copy("x:c", &[]), 10),
            (sees_copy("y:p1"), 50),
        ]
    };
    let mut profile = table(patches());
    profile.order_rules.push(rule("y:*", &["x:*"]));
    let mut compat = table(patches());
    compat
        .compat_rules
        .push((rule("y:*", &["x:*"]), "fixpack".to_string()));
    for t in [profile, compat] {
        let (text, records) = run(&t, "main.lua", MAIN);
        assert_eq!(ids(&records), ["x:c", "y:p1", "x:third"]);
        assert!(records
            .iter()
            .all(|r| r.status == PatchOutcomeStatus::Applied));
        assert!(ordered_warnings(rec(&records, "x:c")).is_empty());
        assert_eq!(
            text,
            format!("copied = 1\n-- x:third saw the copy\n-- y:p1 saw the copy\n{MAIN}")
        );
    }
}

/// The pass is decided per file: a rule "ai.lua pattern before main.lua copy" leaves the copy
/// with the first copies on main.lua.
#[test]
fn a_rule_from_another_file_leaves_the_copy_first() {
    let mut t = table(vec![
        (
            pattern("y:ai", "includes/ai.lua", "local ai = 1", "-- y:ai"),
            5,
        ),
        (sees_copy("y:p1"), 50),
        (copy("x:c", &[]), 10),
    ]);
    t.order_rules.push(rule("y:ai", &["x:c"]));
    assert_eq!(t.prepare().copy_preds(2).len(), 1, "y:ai is a pred");
    let (text, records) = run(&t, "main.lua", MAIN);
    assert_eq!(ids(&records), ["x:c", "y:p1"]);
    assert_eq!(text, format!("copied = 1\n-- y:p1 saw the copy\n{MAIN}"));
    assert!(ordered_warnings(rec(&records, "x:c")).is_empty());
    let (text, records) = run(&t, "includes/ai.lua", AI);
    assert_eq!(ids(&records), ["y:ai"]);
    assert_eq!(text, "local ai = 1\n-- y:ai\n");
}

/// A `skip_mod` failure skips an ordered copy of the same mod that runs after it; a copy that
/// ran first stays applied.
#[test]
fn skip_mod_skips_a_later_ordered_copy() {
    let mut fails = pattern("x:fails", "main.lua", "not in the file", "-- never");
    if let Patch::Pattern(p) = &mut fails {
        p.on_failure = OnFailurePolicy::SkipMod;
    }
    let mut early = copy("x:early", &[]);
    if let Patch::Copy(c) = &mut early {
        c.payload = Some("early = 1".to_string());
    }
    let t = table(vec![
        (fails, 5),
        (copy("x:c", &["x:fails"]), 10),
        (early, 20),
    ]);
    let (text, records) = run(&t, "main.lua", MAIN);
    assert_eq!(ids(&records), ["x:early", "x:fails", "x:c"]);
    assert_eq!(rec(&records, "x:early").status, PatchOutcomeStatus::Applied);
    assert_eq!(
        rec(&records, "x:fails").status,
        PatchOutcomeStatus::SkippedByPolicy
    );
    let c = rec(&records, "x:c");
    assert_eq!(c.status, PatchOutcomeStatus::SkippedByPolicy);
    assert_eq!(c.warnings, [SKIPPED_WITH_MOD_WARNING]);
    assert_eq!(text, format!("early = 1\n{MAIN}"));
}

/// A disabled ordered copy is reported at its place in the pattern pass.
#[test]
fn a_disabled_ordered_copy_is_reported_in_its_pass() {
    let mut t = table(vec![(sees_copy("y:p1"), 50), (copy("x:c", &[]), 10)]);
    t.order_rules.push(rule("y:p1", &["x:c"]));
    t.patches[1].0.set_disabled(true);
    let (text, records) = run(&t, "main.lua", MAIN);
    assert_eq!(text, MAIN);
    assert_eq!(ids(&records), ["y:p1", "x:c"]);
    assert_eq!(
        rec(&records, "x:c").status,
        PatchOutcomeStatus::SkippedDisabled
    );
}
