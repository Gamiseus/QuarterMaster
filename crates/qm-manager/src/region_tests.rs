use super::*;
use crate::conflict::simulate_and_analyze_patches;
use qm_core::patch::anchor::AnchorSource;
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
        times: Some(1),
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

fn table(patches: Vec<(Patch, Priority)>) -> PatchTable {
    let mut t = PatchTable::default();
    t.targets.insert("main.lua".to_string());
    for (i, (p, prio)) in patches.into_iter().enumerate() {
        t.patches
            .push((p, prio, PathBuf::from(format!("M{i}/qm.toml"))));
    }
    t
}

fn run(t: &PatchTable, src: &str) -> SimulationReport {
    let vanilla = HashMap::from([("main.lua".to_string(), src.to_string())]);
    simulate_and_analyze_patches(t, &vanilla)
}

/// `t` with the offer's rungs on its patch, as a profile fix compiles them.
fn with_offer(t: &PatchTable, o: &RegionOffer) -> PatchTable {
    let mut t = PatchTable {
        targets: t.targets.clone(),
        patches: t.patches.clone(),
        ..PatchTable::default()
    };
    for (p, _, _) in &mut t.patches {
        if p.id() != Some(o.patch_id.as_str()) {
            continue;
        }
        if matches!(p, Patch::Pattern(_) | Patch::Regex(_)) {
            let x = p.extra_mut();
            x.rungs = o
                .anchors
                .iter()
                .map(|r| {
                    (
                        r.clone(),
                        AnchorSource::Profile {
                            fix_id: "fx".to_string(),
                        },
                    )
                })
                .collect();
            x.sites = o.sites.clone();
            x.fix_ids = vec!["fx".to_string()];
        }
    }
    t
}

const SRC: &str = "function f()\n  x()\n  y()\n  z()\nend\n";

fn plan_of(t: &PatchTable, r: &SimulationReport, id: &str) -> RegionPlan {
    plan(r, t, "main.lua", id, None).expect("the patch is loaded")
}

/// Reemitted: one suggested option, within the other patch's copy of the line. Applied, the
/// code goes back next to that copy.
#[test]
fn reemitted_offers_the_copy_of_the_line() {
    let t = table(vec![
        (
            pattern(
                "b",
                "b:1",
                "  x()\n  y()",
                InsertPosition::At,
                "  x2()\n  y()",
            ),
            10,
        ),
        (
            pattern("a", "a:1", "  x()\n  y()", InsertPosition::After, "  a()"),
            20,
        ),
    ]);
    let r = run(&t, SRC);
    let p = plan_of(&t, &r, "a:1");
    assert_eq!(p.consumer.as_ref().unwrap().kind, ConsumerKind::Reemitted);
    assert_eq!(p.b, Some(("b:1".to_string(), "b".to_string())));
    assert!(!p.a_is_at && !p.move_by_copy);
    assert_eq!(p.offers.len(), 1, "{:?}", p.offers);
    let o = &p.offers[0];
    assert!(o.suggested && !o.pick);
    assert_eq!(o.label, "Keep it next to b's copy of that line");
    assert_eq!(o.patch_id, "a:1");
    assert_eq!(o.anchors[0].within.as_deref(), Some("b:1"));
    assert_eq!(o.anchors[0].pattern.as_deref(), Some("y()"));
    assert!(o.anchors[0].exact);
    assert_eq!(o.anchors[0].b_site, Some(0));
    assert_eq!(o.sites.len(), 1);
    assert_eq!(o.sites[0].ordinal, Some(1));

    let fixed = run(&with_offer(&t, o), SRC);
    assert_eq!(
        fixed.traces["main.lua"].text,
        "function f()\n  x2()\n  y()\n  a()\n  z()\nend\n"
    );
    let rec = fixed.files_touched[0]
        .records
        .iter()
        .find(|x| x.patch_id == "a:1")
        .unwrap();
    let site = &rec.anchoring.as_ref().unwrap().sites[0];
    assert_eq!(site.anchored_on.as_deref(), Some("b:1"));
    assert_eq!(site.vanilla_line, Some(2), "the file's own numbering");

    // The other patch's place, as the picker lists it.
    assert_eq!(p.sites.len(), 1);
    let s = &p.sites[0];
    assert_eq!((s.site, s.first_line, s.last_line), (0, 2, 3));
    assert_eq!(s.function, "f");
    assert!(s.edge_start.allowed && s.edge_end.allowed);
    let kinds: Vec<&str> = s
        .lines
        .iter()
        .map(|l| l.source.as_ref().unwrap().kind)
        .collect();
    assert_eq!(kinds, ["inserted", "kept"]);
    assert!(s
        .lines
        .iter()
        .all(|l| l.in_b && l.matches_in_region == Some(1)));
}

/// Rewritten1to1: follow the new line, with the old and new text, not suggested.
#[test]
fn rewritten_offers_to_follow_the_new_line() {
    let t = table(vec![
        (
            pattern("b", "b:1", "  y()", InsertPosition::At, "  y2()"),
            10,
        ),
        (
            pattern("a", "a:1", "  y()", InsertPosition::Before, "  a()"),
            20,
        ),
    ]);
    let r = run(&t, SRC);
    let p = plan_of(&t, &r, "a:1");
    assert_eq!(p.offers.len(), 1);
    let o = &p.offers[0];
    assert!(!o.suggested);
    assert_eq!(o.label, "Follow b's new version of that line");
    assert_eq!(o.old_text.as_deref(), Some("y()"));
    assert_eq!(o.new_text.as_deref(), Some("y2()"));
    assert_eq!(o.anchors[0].pattern.as_deref(), Some("y2()"));
    let fixed = run(&with_offer(&t, o), SRC);
    assert_eq!(
        fixed.traces["main.lua"].text,
        "function f()\n  x()\n  a()\n  y2()\n  z()\nend\n"
    );
}

/// ReplacedByBlock: after and before the change, and the picker. A balanced change allows
/// both edges; placed after it, the code goes below the whole block.
#[test]
fn replaced_by_block_offers_edges_and_the_picker() {
    let t = table(vec![
        (
            pattern(
                "b",
                "b:1",
                "  y()",
                InsertPosition::At,
                "  if ok then\n    y2()\n  end",
            ),
            10,
        ),
        (
            pattern("a", "a:1", "  y()", InsertPosition::After, "  a()"),
            20,
        ),
    ]);
    let r = run(&t, SRC);
    let p = plan_of(&t, &r, "a:1");
    let labels: Vec<&str> = p.offers.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Put it after b's change",
            "Put it before b's change",
            "Pick a line inside b's change..."
        ]
    );
    let after = &p.offers[0];
    assert_eq!(after.anchors[0].edge, Some(Edge::End));
    assert!(after.anchors[0].check.is_some());
    assert!(after.edge.as_ref().unwrap().allowed);
    assert!(p.offers[2].pick && p.offers[2].anchors.is_empty());
    assert!(after.note.is_none());
    let fixed = run(&with_offer(&t, after), SRC);
    assert_eq!(
        fixed.traces["main.lua"].text,
        "function f()\n  x()\n  if ok then\n    y2()\n  end\n  a()\n  z()\nend\n"
    );
    let before = run(&with_offer(&t, &p.offers[1]), SRC);
    assert_eq!(
        before.traces["main.lua"].text,
        "function f()\n  x()\n  a()\n  if ok then\n    y2()\n  end\n  z()\nend\n"
    );
}

/// A change that leaves an `if` open: both edges are greyed, the end with when the code would
/// run (§9.2, GCore #16-like).
#[test]
fn edges_of_an_open_block_are_greyed() {
    let t = table(vec![
        (
            pattern(
                "b",
                "b:1",
                "  y()",
                InsertPosition::At,
                "  y2()\n  if ok then",
            ),
            10,
        ),
        (
            pattern("a", "a:1", "  y()", InsertPosition::After, "  a()"),
            20,
        ),
    ]);
    let r = run(&t, "function f()\n  x()\n  y()\n  z()\n  end\nend\n");
    let p = plan_of(&t, &r, "a:1");
    let end = p
        .offers
        .iter()
        .find(|o| o.key.contains(":edge_end:"))
        .unwrap();
    let gate = end.edge.as_ref().unwrap();
    assert!(!gate.allowed);
    assert_eq!(gate.reason.as_deref(), Some("would run only when ok"));
    let start = p
        .offers
        .iter()
        .find(|o| o.key.contains(":edge_start:"))
        .unwrap();
    assert!(!start.edge.as_ref().unwrap().allowed);
    assert!(!p.sites[0].edge_end.allowed);
}

/// Deleted: edges only, flagged as a guess.
#[test]
fn deleted_offers_edges_flagged_as_a_guess() {
    let t = table(vec![
        (regex("b", "b:1", r"(?m)^  y\(\)\n  z\(\)\n", "  z()\n"), 10),
        (
            pattern("a", "a:1", "  y()", InsertPosition::After, "  a()"),
            20,
        ),
    ]);
    let r = run(&t, SRC);
    let p = plan_of(&t, &r, "a:1");
    assert_eq!(p.consumer.as_ref().unwrap().kind, ConsumerKind::Deleted);
    assert_eq!(p.offers.len(), 2, "{:?}", p.offers);
    for o in &p.offers {
        assert!(o.anchors[0].edge.is_some());
        assert_eq!(
            o.note.as_deref(),
            Some("b removed this line; placing next to its change is a guess")
        );
    }
}

/// A Split of an `at` patch (R1 in reversed order): the options are running the victim as
/// separate edits (§4.8), and a rung for the splitter on the victim's copy of its line. The
/// victim itself gets no rung (D6).
#[test]
fn split_of_an_at_patch_moves_the_splitter() {
    let halt = "      if key == \"h\" then\n        p.tx = p.x\n        p.ty = p.y\n        if(spamTimer <= 0) then\n          cue()\n        end\n      end\n";
    let mut gcore = pattern(
        "gcore",
        "gcore:16",
        "p.tx = p.x\np.ty = p.y\nif(spamTimer <= 0) then",
        InsertPosition::At,
        "p.tx = p.x\np.ty = p.y\nclear(p)\nif(spamTimer <= 0) then",
    );
    if let Patch::Pattern(x) = &mut gcore {
        x.match_indent = true;
    }
    let to = pattern(
        "to",
        "to:6",
        "p.ty = p.y",
        InsertPosition::After,
        "        p.timer = 180",
    );
    let t = table(vec![(to, 20), (gcore, 100)]);
    let r = run(&t, halt);
    let p = plan_of(&t, &r, "gcore:16");
    assert!(p.a_is_at && p.move_by_copy);
    assert_eq!(p.offers.len(), 2, "{:?}", p.offers);
    let e = &p.offers[0];
    assert!(e.apply_as && e.suggested && e.anchors.is_empty(), "{e:?}");
    assert_eq!((e.patch_id.as_str(), e.label.as_str()), ("gcore:16", "Run gcore:16 as separate edits"));
    // Its card says where GCore's line goes and that TO's line stays (§9.7).
    assert_eq!(
        e.note.as_deref(),
        Some("gcore's 1 new line goes in after \"p.ty = p.y\"; to's lines next to it stay")
    );
    // As edits GCore skips TO's line: both apply, GCore's line right after `p.ty`.
    let mut edits = t.clone();
    for (x, _, _) in edits.patches.iter_mut() {
        if let Patch::Pattern(x) = x {
            if x.id.as_deref() == Some("gcore:16") {
                x.qm.apply_as = Some(qm_core::patch::edits::ApplyAs::Edits);
            }
        }
    }
    let fixed = run(&edits, halt);
    let text = &fixed.traces["main.lua"].text;
    assert!(
        text.contains("        p.ty = p.y\n        clear(p)\n        p.timer = 180\n        if(spamTimer"),
        "{text}"
    );
    assert!(fixed.losses.is_empty(), "{:?}", fixed.losses);
    let o = &p.offers[1];
    assert!(!o.apply_as);
    assert_eq!(o.kind, ConsumerKind::Split);
    assert_eq!(o.patch_id, "to:6");
    assert_eq!(o.anchors[0].within.as_deref(), Some("gcore:16"));
    assert_eq!(o.anchors[0].pattern.as_deref(), Some("p.ty = p.y"));
    assert_eq!(o.label, "Keep to's to:6 next to gcore's copy of its line");
    // The rung's order edge runs GCore first: both apply.
    let fixed = run(&with_offer(&t, o), halt);
    let text = &fixed.traces["main.lua"].text;
    assert!(
        text.contains("p.ty = p.y\n        p.timer = 180\n        clear(p)\n"),
        "{text}"
    );
    assert!(fixed.files_touched[0]
        .records
        .iter()
        .all(|x| x.status == qm_core::handshake::PatchOutcomeStatus::Applied));
}

/// An `at` patch whose line another patch rewrote gets no options of its own (D6).
#[test]
fn an_at_patch_gets_no_rungs() {
    let t = table(vec![
        (
            pattern("b", "b:1", "  y()", InsertPosition::At, "  y2()"),
            10,
        ),
        (
            pattern("a", "a:1", "  y()", InsertPosition::At, "  y3()"),
            20,
        ),
    ]);
    let r = run(&t, SRC);
    let p = plan_of(&t, &r, "a:1");
    assert!(p.a_is_at && p.move_by_copy);
    assert!(p.offers.is_empty());
    assert!(p.consumer.is_some());
    assert_eq!(p.sites.len(), 1, "the other patch's place is still listed");
}

/// The picker reads the text patterns match, before `[vars]` values are filled in.
#[test]
fn region_lines_keep_the_pre_vars_text() {
    let mut t = table(vec![(
        pattern(
            "b",
            "b:1",
            "  y()",
            InsertPosition::After,
            "  speed = {{qm:speed}}\n  speed = {{qm:speed}}",
        ),
        10,
    )]);
    t.vars.insert("speed".to_string(), "5".to_string());
    let r = run(&t, SRC);
    let trace = &r.traces["main.lua"];
    assert!(trace.text.contains("  speed = 5\n"));
    let sites = b_sites(trace, 0, "b");
    let lines = &sites[0].lines;
    assert_eq!(lines[0].text, "  speed = 5");
    assert_eq!(lines[0].raw_text, "  speed = {{qm:speed}}");
    assert_eq!(lines[0].matches_in_region, Some(2));
}

/// A patch's places in the game file, with keys that pick one of them.
#[test]
fn a_sites_list_every_place() {
    let src = "a = 1\nmark()\nb = 2\nmark()\n";
    let t = table(vec![]);
    let buf = prepared_original(&t, "main.lua", src);
    let Patch::Pattern(a) = pattern("a", "a:1", "mark()", InsertPosition::After, "x()") else {
        unreachable!()
    };
    let sites = a_sites(&a, &buf);
    assert_eq!(sites.iter().map(|s| s.line).collect::<Vec<_>>(), [2, 4]);
    assert_eq!(sites[1].key.ordinal, Some(2));
    assert_ne!(sites[0].key.context, sites[1].key.context);
    assert_eq!(sites[0].text, "mark()");
    assert_eq!(sites.iter().map(|s| s.anchor_line).collect::<Vec<_>>(), [2, 4]);
}

/// A multi-line `after` place hangs on its last line, as `consumer.site_line` does.
#[test]
fn a_sites_anchor_line_is_the_last_line_for_after() {
    let src = "a = 1
mark()
b = 2
c = 3
";
    let t = table(vec![]);
    let buf = prepared_original(&t, "main.lua", src);
    let go = |pos| {
        let Patch::Pattern(a) = pattern("a", "a:1", "mark()
b = 2", pos, "x()") else {
            unreachable!()
        };
        a_sites(&a, &buf).iter().map(|s| (s.line, s.anchor_line)).collect::<Vec<_>>()
    };
    assert_eq!(go(InsertPosition::After), [(2, 3)]);
    assert_eq!(go(InsertPosition::Before), [(2, 2)]);
}

/// QuarterMaster's own code has `end` lines too; the manager numbers a patch's places as the
/// engine does, without them (D42), so a site key it writes picks the same place.
#[test]
fn a_sites_number_places_as_the_engine_does() {
    let src = "a()\nb()\nend\nc()\nd()\na()\nb()\nend\nc()\nd()\n";
    let t = table(vec![]);
    let buf = prepared_original(&t, "main.lua", src);
    let ends = buf.text.lines().filter(|l| l.trim() == "end").count();
    assert!(ends > 2, "QuarterMaster's code has `end` lines too");
    let Patch::Pattern(mut a) = pattern("a", "a:1", "end", InsertPosition::After, "x()") else {
        unreachable!()
    };
    a.times = None;
    let sites = a_sites(&a, &buf);
    assert_eq!(sites.iter().map(|s| s.line).collect::<Vec<_>>(), [3, 8]);
    assert_eq!(sites[0].key.context, sites[1].key.context);
    assert_eq!(sites[1].key.ordinal, Some(2));

    // The engine picks the second place by that key and moves it with a text rung.
    a.qm.sites = vec![sites[1].key.clone()];
    a.qm.rungs = vec![(
        Rung {
            pattern: Some("c()".to_string()),
            nth: Some(1),
            ..Rung::default()
        },
        AnchorSource::Author,
    )];
    let t = table(vec![(Patch::Pattern(a), 10)]);
    let r = run(&t, src);
    let rec = &r.files_touched[0].records[0];
    let site = &rec.anchoring.as_ref().expect("a ladder").sites[0];
    assert_eq!((site.vanilla_line, site.rung), (Some(8), Some(0)), "{rec:?}");
    assert_eq!(
        r.traces["main.lua"].text,
        "a()\nb()\nend\nx()\nc()\nx()\nd()\na()\nb()\nend\nc()\nd()\n"
    );
}

fn regex_at(
    mod_id: &str,
    id: &str,
    re: &str,
    position: InsertPosition,
    payload: &str,
) -> Patch {
    let mut p = regex(mod_id, id, re, payload);
    if let Patch::Regex(x) = &mut p {
        x.position = position;
    }
    p
}

/// A regex that lost its line to another patch's block gets region rungs (D52): both
/// edges and the picker; "Move to another line" still makes an adjusted copy (D25). Placed
/// after the change, its lines go below the whole block.
#[test]
fn a_regex_gets_region_rungs() {
    let b = pattern(
        "b",
        "b:1",
        "  y()",
        InsertPosition::At,
        "  if ok then\n    y2()\n  end",
    );
    let a = regex_at("a", "a:1", r"(?m)^  y\(\)\n", InsertPosition::After, "  a()\n");
    let t = table(vec![(b.clone(), 10), (a, 20)]);
    let r = run(&t, SRC);
    let p = plan_of(&t, &r, "a:1");
    assert_eq!(
        p.consumer.as_ref().unwrap().kind,
        ConsumerKind::ReplacedByBlock
    );
    assert!(!p.a_is_at && p.move_by_copy);
    let labels: Vec<&str> = p.offers.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Put it after b's change",
            "Put it before b's change",
            "Pick a line inside b's change..."
        ]
    );
    let fixed = run(&with_offer(&t, &p.offers[0]), SRC);
    assert_eq!(
        fixed.traces["main.lua"].text,
        "function f()\n  x()\n  if ok then\n    y2()\n  end\n  a()\n  z()\nend\n"
    );
    let rec = fixed.files_touched[0]
        .records
        .iter()
        .find(|x| x.patch_id == "a:1")
        .unwrap();
    assert_eq!(rec.status, qm_core::handshake::PatchOutcomeStatus::Applied);
    assert_eq!(rec.anchoring.as_ref().unwrap().sites[0].rung, Some(0));

    // A regex that reads its match has nothing to put at an edge: only the picker. A copy
    // on another line would lose the match, so a move is a rung too (D59).
    let a = regex_at("a", "a:1", r"(?m)^  y\(\)\n", InsertPosition::After, "  a($0)\n");
    let t = table(vec![(b, 10), (a, 20)]);
    let r = run(&t, SRC);
    let p = plan_of(&t, &r, "a:1");
    assert_eq!(p.offers.len(), 1, "{:?}", p.offers);
    assert!(p.offers[0].pick);
    assert!(!p.move_by_copy);
}

/// A regex's places are its matches, numbered as the engine numbers them (D42, D52).
#[test]
fn regex_sites_number_places_as_the_engine_does() {
    let src = "a()\nb()\nend\nc()\nd()\na()\nb()\nend\nc()\nd()\n";
    let t = table(vec![]);
    let buf = prepared_original(&t, "main.lua", src);
    let mut a = regex_at("a", "a:1", r"(?m)^end$", InsertPosition::After, "\nx()");
    if let Patch::Regex(x) = &mut a {
        x.times = None;
    }
    let sites = patch_sites(&a, &buf);
    assert_eq!(sites.iter().map(|s| s.line).collect::<Vec<_>>(), [3, 8]);
    assert_eq!(sites[1].key.ordinal, Some(2));

    // The engine picks the second place by that key.
    if let Patch::Regex(x) = &mut a {
        x.qm.sites = vec![sites[1].key.clone()];
        x.qm.rungs = vec![(
            Rung {
                pattern: Some("end".to_string()),
                nth: Some(2),
                ..Rung::default()
            },
            AnchorSource::Author,
        )];
    }
    let t = table(vec![(a, 10)]);
    let r = run(&t, src);
    let rec = &r.files_touched[0].records[0];
    let site = &rec.anchoring.as_ref().expect("a ladder").sites[0];
    assert_eq!((site.vanilla_line, site.rung), (Some(8), Some(0)), "{rec:?}");
}

/// A before/after patch on two lines whose lines another patch split is offered "Run it as
/// separate edits" (§4.8), and gets no rung for the splitter (only an `at` victim does).
#[test]
fn split_of_an_after_patch_offers_separate_edits() {
    let t = table(vec![
        (
            pattern("b", "b:1", "  x()", InsertPosition::After, "  b()"),
            10,
        ),
        (
            pattern("a", "a:1", "  x()\n  y()", InsertPosition::After, "  a()"),
            20,
        ),
    ]);
    let r = run(&t, SRC);
    let p = plan_of(&t, &r, "a:1");
    assert!(!p.a_is_at);
    let edits: Vec<&RegionOffer> = p.offers.iter().filter(|o| o.apply_as).collect();
    assert_eq!(edits.len(), 1, "{:?}", p.offers);
    assert_eq!(edits[0].patch_id, "a:1");
    assert_eq!(edits[0].kind, ConsumerKind::Split);
    assert_eq!(
        edits[0].note.as_deref(),
        Some("a's lines find their place again around b's lines, which stay")
    );
    assert!(p.offers.iter().all(|o| o.apply_as), "{:?}", p.offers);
}

/// Two places split at two sites of the other patch make two groups, but "Run it as separate
/// edits" is one offer for the whole patch.
#[test]
fn two_split_places_get_one_separate_edits_offer() {
    let src = "function f()\n  x()\n  y()\nend\nfunction g()\n  x()\n  y()\nend\n";
    let t = table(vec![
        (
            pattern("b", "b:1", "  x()", InsertPosition::After, "  b()"),
            10,
        ),
        (
            pattern("a", "a:1", "  x()\n  y()", InsertPosition::After, "  a()"),
            20,
        ),
    ]);
    let r = run(&t, src);
    let p = plan_of(&t, &r, "a:1");
    let edits = p.offers.iter().filter(|o| o.apply_as).count();
    assert_eq!(edits, 1, "{:?}", p.offers);
}
