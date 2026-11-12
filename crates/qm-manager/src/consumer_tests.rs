use super::*;
use crate::conflict::{simulate_and_analyze_patches, DetectedConflict, SimulationReport};
use qm_core::patch::regex::RegexPatch;
use qm_core::patch::{OnFailurePolicy, Priority, Target};
use std::collections::HashMap;
use std::path::PathBuf;

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
    })
}

fn once(mut p: Patch) -> Patch {
    if let Patch::Pattern(x) = &mut p {
        x.times = Some(1);
    }
    p
}

fn regex(mod_id: &str, id: &str, re: &str, payload: &str) -> Patch {
    Patch::Regex(RegexPatch {
        target: Target::Single("main.lua".to_string()),
        pattern: re.to_string(),
        position: InsertPosition::At,
        root_capture: None,
        payload: payload.to_string(),
        line_prepend: String::new(),
        times: Some(1),
        verbose: false,
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
        mod_id: Some(mod_id.to_string()),
        disabled: false,
        qm: Default::default(),
    })
}

fn simulate(patches: Vec<(Patch, Priority)>, src: &str) -> SimulationReport {
    let mut table = PatchTable::default();
    table.targets.insert("main.lua".to_string());
    for (i, (p, prio)) in patches.into_iter().enumerate() {
        table
            .patches
            .push((p, prio, PathBuf::from(format!("M{i}/qm.toml"))));
    }
    let vanilla = HashMap::from([("main.lua".to_string(), src.to_string())]);
    simulate_and_analyze_patches(&table, &vanilla)
}

fn conflict_of<'r>(r: &'r SimulationReport, patch: &str) -> &'r DetectedConflict {
    r.conflicts
        .iter()
        .find(|c| c.patch_a_id == patch && c.consumer.is_some())
        .unwrap_or_else(|| panic!("no conflict with a consumer for {patch}: {:?}", r.conflicts))
}

const SRC: &str = "function f()\n  x()\n  y()\n  z()\nend\n";

/// The H-key Halt lines (R1): TO adds after the targety line, GCore rewrites the three lines.
const HALT: &str = "      if key == \"h\" then
        player.deployedTroops[i].targetx = player.deployedTroops[i].x
        player.deployedTroops[i].targety = player.deployedTroops[i].y
        if(spamTimer <= 0) then
          newTextCue(\"Holding!\", player.deployedTroops[i].x, player.deployedTroops[i].y)
        end
      end
";

fn gcore_16() -> Patch {
    let mut p = once(pattern(
        "gcore",
        "gcore:main_lua:16",
        "player.deployedTroops[i].targetx = player.deployedTroops[i].x\nplayer.deployedTroops[i].targety = player.deployedTroops[i].y\nif(spamTimer <= 0) then",
        InsertPosition::At,
        "player.deployedTroops[i].targetx = player.deployedTroops[i].x\nplayer.deployedTroops[i].targety = player.deployedTroops[i].y\npathfinding.clearUnitWaypoints(player.deployedTroops[i])\nif(spamTimer <= 0) then",
    ));
    if let Patch::Pattern(x) = &mut p {
        x.match_indent = true;
    }
    p
}

fn to_6() -> Patch {
    once(pattern(
        "tactical_overhaul",
        "tactical_overhaul:main_lua:6",
        "player.deployedTroops[i].targety = player.deployedTroops[i].y",
        InsertPosition::After,
        "        player.deployedTroops[i].manualOrderTimer = 180",
    ))
}

/// R1 in reversed order: TO's line goes in first and splits GCore's window, so GCore loses
/// its only place. The fix is a rung for TO on GCore's copy of the targety line.
#[test]
fn r1_reversed_is_split_by_to() {
    let r = simulate(vec![(to_6(), 20), (gcore_16(), 100)], HALT);
    let c = conflict_of(&r, "gcore:main_lua:16");
    assert_eq!(c.kind, "broken_anchor_order");
    assert_eq!(c.mod_b_id.as_deref(), Some("tactical_overhaul"));
    assert_eq!(c.sites_lost, Some((1, 1)));
    let info = c.consumer.as_ref().unwrap();
    assert_eq!(info.kind, ConsumerKind::Split);
    assert_eq!(info.patch_id, "tactical_overhaul:main_lua:6");
    assert_eq!(info.mod_id, "tactical_overhaul");
    assert_eq!(info.site_line, 2);
    assert_eq!(
        info.new_text.as_deref(),
        Some("        player.deployedTroops[i].manualOrderTimer = 180")
    );
    let loss = r
        .losses
        .iter()
        .find(|l| l.patch_id == "gcore:main_lua:16")
        .unwrap();
    let h = loss.rung.as_ref().expect("a rung for the splitter");
    assert_eq!(h.patch_id, "tactical_overhaul:main_lua:6");
    assert_eq!(h.within, "gcore:main_lua:16");
    assert_eq!(
        h.pattern,
        "player.deployedTroops[i].targety = player.deployedTroops[i].y"
    );
    assert!(h.single_site);
    assert_eq!(h.position, InsertPosition::After);
    assert_eq!(h.context.len(), 32);
}

/// In today's order GCore runs first and writes the targety line back; TO still finds it, so
/// nothing is lost.
#[test]
fn r1_in_order_loses_nothing() {
    let r = simulate(vec![(gcore_16(), 10), (to_6(), 20)], HALT);
    assert!(r.losses.is_empty(), "{:?}", r.losses);
    assert!(
        r.conflicts.iter().all(|c| c.consumer.is_none()),
        "{:?}",
        r.conflicts
    );
}

/// B rewrites both lines of A's two-line pattern but writes A's anchor line back.
#[test]
fn reemitted_anchor_line() {
    let r = simulate(
        vec![
            (
                once(pattern(
                    "b",
                    "b:1",
                    "  x()\n  y()",
                    InsertPosition::At,
                    "  x2()\n  y()",
                )),
                10,
            ),
            (
                once(pattern(
                    "a",
                    "a:1",
                    "  x()\n  y()",
                    InsertPosition::After,
                    "  a()",
                )),
                20,
            ),
        ],
        SRC,
    );
    let c = conflict_of(&r, "a:1");
    let info = c.consumer.as_ref().unwrap();
    assert_eq!(info.kind, ConsumerKind::Reemitted);
    assert_eq!(info.patch_id, "b:1");
    assert_eq!(info.site_line, 3);
    assert_eq!(info.old_text, "y()");
    assert_eq!(info.new_text.as_deref(), Some("  y()"));
    let h = r.losses[0].rung.as_ref().unwrap();
    assert_eq!(
        (h.patch_id.as_str(), h.within.as_str(), h.pattern.as_str()),
        ("a:1", "b:1", "y()")
    );
}

#[test]
fn rewritten_one_to_one() {
    let r = simulate(
        vec![
            (
                once(pattern("b", "b:1", "  y()", InsertPosition::At, "  y2()")),
                10,
            ),
            (
                once(pattern(
                    "a",
                    "a:1",
                    "  y()",
                    InsertPosition::Before,
                    "  a()",
                )),
                20,
            ),
        ],
        SRC,
    );
    let info = conflict_of(&r, "a:1").consumer.clone().unwrap();
    assert_eq!(info.kind, ConsumerKind::Rewritten1to1);
    assert_eq!(
        (info.old_text.as_str(), info.new_text.as_deref()),
        ("y()", Some("  y2()"))
    );
    assert!(r.losses[0].rung.is_none());
}

#[test]
fn replaced_by_block() {
    let r = simulate(
        vec![
            (
                once(pattern(
                    "b",
                    "b:1",
                    "  y()",
                    InsertPosition::At,
                    "  if ok then\n    y2()\n  end",
                )),
                10,
            ),
            (
                once(pattern("a", "a:1", "  y()", InsertPosition::After, "  a()")),
                20,
            ),
        ],
        SRC,
    );
    let info = conflict_of(&r, "a:1").consumer.clone().unwrap();
    assert_eq!(info.kind, ConsumerKind::ReplacedByBlock);
    assert_eq!(
        info.new_text.as_deref(),
        Some("  if ok then\n    y2()\n  end")
    );
}

#[test]
fn deleted_line() {
    let r = simulate(
        vec![
            (regex("b", "b:1", r"(?m)^  y\(\)\n", ""), 10),
            (
                once(pattern("a", "a:1", "  y()", InsertPosition::After, "  a()")),
                20,
            ),
        ],
        SRC,
    );
    assert_eq!(
        r.traces["main.lua"].text,
        "function f()\n  x()\n  z()\nend\n"
    );
    let info = conflict_of(&r, "a:1").consumer.clone().unwrap();
    assert_eq!(info.kind, ConsumerKind::Deleted);
    assert_eq!(info.patch_id, "b:1");
    assert_eq!(info.new_text, None);
}

/// A line changed inside the line (a regex) keeps its place but no longer matches.
#[test]
fn changed_in_place_counts_as_rewritten() {
    let r = simulate(
        vec![
            (regex("b", "b:1", r"y\(\)", "y(1)"), 10),
            (
                once(pattern("a", "a:1", "  y()", InsertPosition::After, "  a()")),
                20,
            ),
        ],
        SRC,
    );
    let info = conflict_of(&r, "a:1").consumer.clone().unwrap();
    assert_eq!(info.kind, ConsumerKind::Rewritten1to1);
    assert_eq!(info.new_text.as_deref(), Some("  y(1)"));
}

/// Split of a patch that is not an `at`: no rung is offered (only the victim-is-B case).
#[test]
fn split_window_of_an_insert() {
    let r = simulate(
        vec![
            (
                once(pattern("b", "b:1", "  x()", InsertPosition::After, "  b()")),
                10,
            ),
            (
                once(pattern(
                    "a",
                    "a:1",
                    "  x()\n  y()",
                    InsertPosition::After,
                    "  a()",
                )),
                20,
            ),
        ],
        SRC,
    );
    let info = conflict_of(&r, "a:1").consumer.clone().unwrap();
    assert_eq!(info.kind, ConsumerKind::Split);
    assert_eq!(info.patch_id, "b:1");
    assert_eq!(info.new_text.as_deref(), Some("  b()"));
    assert!(r.losses[0].rung.is_none());
}

/// Partial consumption: A has three places, B rewrites one, A still adds its code at the other
/// two. A `sites_lost` warning names B.
#[test]
fn partial_consumption_three_to_two() {
    let src = "a = 1\nmark()\nb = 2\nmark()\nc = 3\nmark()\n";
    let r = simulate(
        vec![
            (
                once(pattern("b", "b:1", "mark()", InsertPosition::At, "mark2()")),
                10,
            ),
            (
                pattern("a", "a:1", "mark()", InsertPosition::After, "added()"),
                20,
            ),
        ],
        src,
    );
    let rec = r.files_touched[0]
        .records
        .iter()
        .find(|x| x.patch_id == "a:1")
        .unwrap();
    assert_eq!(
        (rec.status, rec.matched_count),
        (PatchOutcomeStatus::Applied, 2)
    );
    let c = conflict_of(&r, "a:1");
    assert_eq!(c.kind, "sites_lost");
    assert_eq!(c.severity, "warning");
    assert_eq!(c.sites_lost, Some((1, 3)));
    assert_eq!(c.patch_b_id.as_deref(), Some("b:1"));
    let info = c.consumer.as_ref().unwrap();
    assert_eq!(
        (info.kind, info.site_line),
        (ConsumerKind::Rewritten1to1, 2)
    );
    assert_eq!(r.losses.len(), 1);
    assert_eq!(r.losses[0].site, 0);
}

/// The same with both patches in one mod: the mod's own business, no `sites_lost` warning.
#[test]
fn partial_consumption_within_one_mod_is_not_a_conflict() {
    let src = "a = 1
mark()
b = 2
mark()
c = 3
mark()
";
    let r = simulate(
        vec![
            (
                once(pattern("a", "a:b", "mark()", InsertPosition::At, "mark2()")),
                10,
            ),
            (
                pattern("a", "a:1", "mark()", InsertPosition::After, "added()"),
                20,
            ),
        ],
        src,
    );
    assert!(
        !r.conflicts.iter().any(|c| c.kind == "sites_lost"),
        "{:?}",
        r.conflicts
    );
    assert_eq!(r.losses.len(), 1, "the loss is still recorded");
}

/// With `times`, a place the patch would not use anyway is not lost.
#[test]
fn places_past_times_do_not_count() {
    let src = "mark()\nmark()\nend_here\n";
    let r = simulate(
        vec![
            (
                once(pattern(
                    "b",
                    "b:1",
                    "mark()\nend_here",
                    InsertPosition::At,
                    "other()\nend_here",
                )),
                10,
            ),
            (
                once(pattern(
                    "a",
                    "a:1",
                    "mark()",
                    InsertPosition::After,
                    "added()",
                )),
                20,
            ),
        ],
        src,
    );
    assert_eq!(
        r.traces["main.lua"].text,
        "mark()\nadded()\nother()\nend_here\n"
    );
    assert!(r.losses.is_empty(), "{:?}", r.losses);
}

/// The prefix QuarterMaster adds to main.lua does not shift the line numbers it reports.
#[test]
fn site_lines_are_file_lines() {
    let r = simulate(
        vec![
            (
                once(pattern("b", "b:1", "  z()", InsertPosition::At, "  z2()")),
                10,
            ),
            (
                once(pattern(
                    "a",
                    "a:1",
                    "  z()",
                    InsertPosition::Before,
                    "  a()",
                )),
                20,
            ),
        ],
        SRC,
    );
    assert_eq!(
        conflict_of(&r, "a:1").consumer.as_ref().unwrap().site_line,
        4
    );
}

/// A pattern whose only place is in QuarterMaster's own code has no place in the game's file
/// (D42): no loss, no consumer and no multi-match note, as the engine skips it too.
#[test]
fn a_match_on_quartermaster_s_code_is_no_place() {
    let steam = "print(\"[QM] *** STEAM SAFETY HOOK INITIALIZED ***\")";
    let r = simulate(
        vec![
            (pattern("a", "a:1", steam, InsertPosition::After, "a()"), 10),
            // `end` matches QuarterMaster's code many times, the file's once.
            (pattern("c", "c:1", "end", InsertPosition::After, "c()"), 20),
            (
                once(pattern("b", "b:1", "  z()", InsertPosition::At, "  z()\n  b()")),
                30,
            ),
        ],
        SRC,
    );
    let recs = &r.files_touched[0].records;
    let rec = |id: &str| recs.iter().find(|x| x.patch_id == id).unwrap();
    assert_eq!(rec("a:1").status, PatchOutcomeStatus::ZeroMatches);
    assert_eq!((rec("c:1").status, rec("c:1").matched_count), (PatchOutcomeStatus::Applied, 1));
    assert!(r.losses.is_empty(), "{:?}", r.losses);
    assert!(r.kept_anchors.is_empty(), "{:?}", r.kept_anchors);
    // a:1 is reported as a patch with no match, with no other patch as its cause.
    let of = |id: &'static str| r.conflicts.iter().filter(move |c| c.patch_a_id == id);
    assert!(of("a:1").all(|c| c.consumer.is_none() && c.mod_b_id.is_none()), "{:?}", r.conflicts);
    assert_eq!(of("c:1").count(), 0, "{:?}", r.conflicts);
}

/// A regex loses its match the way a pattern loses its window (D52): the place, the patch that
/// took it and how are found from the regex's matches.
#[test]
fn a_regex_that_lost_its_match_names_the_consumer() {
    let r = simulate(
        vec![
            (
                pattern("b", "b:1", "  y()", InsertPosition::At, "  y2()"),
                10,
            ),
            (regex("a", "a:1", r"(?m)^  y\(\)$", "  y() -- a"), 20),
        ],
        SRC,
    );
    let info = conflict_of(&r, "a:1").consumer.clone().unwrap();
    assert_eq!(info.kind, ConsumerKind::Rewritten1to1);
    assert_eq!(info.patch_id, "b:1");
    assert_eq!(info.site_line, 3);
    assert_eq!(info.new_text.as_deref(), Some("  y2()"));
    let loss = r.losses.iter().find(|l| l.patch_id == "a:1").unwrap();
    assert_eq!((loss.site, loss.sites_total, loss.sites_lost), (0, 1, 1));
    assert_eq!(loss.position, InsertPosition::At);
}

fn edits(mut p: Patch) -> Patch {
    if let Patch::Pattern(x) = &mut p {
        x.qm.apply_as = Some(qm_core::patch::edits::ApplyAs::Edits);
    }
    p
}

/// A failing `at` is offered "Run it as separate edits" only when that can change something:
/// it does not run as edits yet.
#[test]
fn an_at_already_running_as_edits_is_not_offered_edits_again() {
    let is_apply_as = |r: &SimulationReport| {
        r.conflicts
            .iter()
            .filter(|c| c.patch_a_id == "a:1")
            .flat_map(|c| c.suggestions.iter())
            .any(|s| matches!(s, crate::conflict::ConflictResolutionSuggestion::ApplyAs { .. }))
    };
    let rewrite = || once(pattern("b", "b:1", "  y()", InsertPosition::At, "  y2()"));
    let at = || once(pattern("a", "a:1", "  y()", InsertPosition::At, "  y() -- a"));
    let r = simulate(vec![(rewrite(), 10), (at(), 20)], SRC);
    assert!(is_apply_as(&r), "{:?}", r.conflicts);
    let r = simulate(vec![(rewrite(), 10), (edits(at()), 20)], SRC);
    assert!(!is_apply_as(&r), "{:?}", r.conflicts);
}

/// Every Split offers "Run it as separate edits" (§4.8), for a multi-line before/after victim
/// as for an `at` one, and the conflict carries the ApplyAs suggestion.
#[test]
fn a_split_offers_separate_edits() {
    let after = || {
        once(pattern(
            "a",
            "a:1",
            "  x()\n  y()",
            InsertPosition::After,
            "  a()",
        ))
    };
    let splitter = || once(pattern("b", "b:1", "  x()", InsertPosition::After, "  b()"));
    let r = simulate(vec![(splitter(), 10), (after(), 20)], SRC);
    let loss = r.losses.iter().find(|l| l.patch_id == "a:1").unwrap();
    assert!(loss.apply_as, "{loss:?}");
    let c = conflict_of(&r, "a:1");
    assert!(
        c.suggestions.iter().any(|s| matches!(
            s,
            crate::conflict::ConflictResolutionSuggestion::ApplyAs { patch_id, .. } if patch_id == "a:1"
        )),
        "{:?}",
        c.suggestions
    );
    // GCore's `at` split by TO.
    let r = simulate(vec![(to_6(), 20), (gcore_16(), 100)], HALT);
    let loss = r
        .losses
        .iter()
        .find(|l| l.patch_id == "gcore:main_lua:16")
        .unwrap();
    assert!(loss.apply_as, "{loss:?}");
    assert!(loss.rung.is_some(), "the splitter rung is still offered");

    // In edits mode the victim finds its split place again: nothing is lost.
    let r = simulate(vec![(splitter(), 10), (edits(after()), 20)], SRC);
    assert!(r.losses.is_empty(), "{:?}", r.losses);
    let r = simulate(vec![(to_6(), 20), (edits(gcore_16()), 100)], HALT);
    assert!(r.losses.is_empty(), "{:?}", r.losses);

    // Other kinds of loss do not offer it.
    let r = simulate(
        vec![
            (
                once(pattern("b", "b:1", "  x()", InsertPosition::At, "  X()")),
                10,
            ),
            (after(), 20),
        ],
        SRC,
    );
    let loss = r.losses.iter().find(|l| l.patch_id == "a:1").unwrap();
    assert!(!loss.apply_as, "{loss:?}");
}

/// The simulation mirrors a split site the engine refuses (D48): a change across another
/// patch's line is still a lost place, named as a Split.
#[test]
fn a_refused_split_change_is_still_a_loss() {
    let at = edits(once(pattern(
        "a",
        "a:1",
        "  x()\n  y()",
        InsertPosition::At,
        "  X()\n  Y()",
    )));
    let splitter = once(pattern("b", "b:1", "  x()", InsertPosition::After, "  b()"));
    let r = simulate(vec![(splitter, 10), (at, 20)], SRC);
    let loss = r.losses.iter().find(|l| l.patch_id == "a:1").unwrap();
    assert_eq!(
        loss.consumer.as_ref().map(|c| c.kind),
        Some(ConsumerKind::Split)
    );
    assert!(!loss.apply_as, "it already runs as edits");
}

/// An exact copy's line matches only its own text in every mirror (D59): its places in the
/// game's file (`a_sites`, the consumer's), and its loss when another patch rewrites its line,
/// agree with the engine. With wildcards `x * y` also matches `x + y`.
#[test]
fn an_exact_copy_has_one_place_in_every_mirror() {
    let src = "function f()\n  local a = x * y\n  local a = x + y\nend\n";
    let mut a = pattern(
        "a",
        "a:copy",
        "  local a = x * y",
        InsertPosition::After,
        "  a()",
    );
    a.extra_mut().exact_lines = Some(vec!["local a = x * y".to_string()]);
    a.extra_mut().adjusted_copy = true;
    let Patch::Pattern(ap) = &a else {
        unreachable!()
    };
    let buf = crate::region::prepared_original(&PatchTable::default(), "main.lua", src);
    let original = Original::new(&buf);
    assert_eq!(original.windows(&ap.pattern, false, true).len(), 1);
    assert_eq!(original.windows(&ap.pattern, false, false).len(), 2);
    assert_eq!(crate::region::a_sites(ap, &buf).len(), 1);

    // Alone, the engine finds the one place too.
    let alone = simulate(vec![(a.clone(), 20)], src);
    let rec = &alone.files_touched[0].records[0];
    assert_eq!(rec.matched_count, 1);
    assert!(alone.losses.is_empty(), "{:?}", alone.losses);

    // B rewrites the line: the copy loses its one place, and the loss names B.
    let b = once(pattern(
        "b",
        "b:1",
        "  local a = x * y",
        InsertPosition::At,
        "  local a = 2",
    ));
    let r = simulate(vec![(b, 10), (a, 20)], src);
    let losses: Vec<&SiteLoss> = r.losses.iter().filter(|l| l.patch_id == "a:copy").collect();
    assert_eq!(losses.len(), 1, "{:?}", r.losses);
    assert_eq!(losses[0].sites_total, 1);
    assert_eq!(
        losses[0].consumer.as_ref().map(|c| c.patch_id.as_str()),
        Some("b:1")
    );
}
