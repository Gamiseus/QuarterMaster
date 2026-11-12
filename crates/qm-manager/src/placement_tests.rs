use super::*;
use crate::conflict::{simulate_and_analyze_patches, SimulationReport};
use qm_core::patch::anchor::{AnchorSource, Rung};
use qm_core::patch::{OnFailurePolicy, PatternPatch, Priority, Target};
use std::collections::HashMap;
use std::path::PathBuf;

fn pattern(mod_id: &str, id: &str, anchor: &str, position: InsertPosition, payload: &str) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: anchor.to_string(),
        position,
        target: Target::Single("main.lua".to_string()),
        payload: payload.to_string(),
        match_indent: true,
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

const HALT: &str = "function keys(key)
  if key == \"h\" then
    targetx = x
    targety = y
    if(spamTimer <= 0) then
      newTextCue(\"Holding!\")
    end
  end
end
";

/// The lines patch `id` wrote in the patched file.
fn site_lines<'r>(r: &'r SimulationReport, id: &str) -> Vec<&'r str> {
    let rec = r.files_touched[0]
        .records
        .iter()
        .find(|x| x.patch_id == id)
        .unwrap();
    let region = &rec.regions[0];
    r.traces["main.lua"]
        .text
        .lines()
        .skip(region.start_line - 1)
        .take(region.end_line - region.start_line + 1)
        .collect()
}

/// A GCore #16-like change ends with an open `if`: its end edge is greyed with when the code
/// would run, and its start edge too (the engine never places code at its edges, D10).
#[test]
fn placement_greys_the_edge_after_an_open_if() {
    let b = pattern(
        "gcore",
        "gcore:main_lua:16",
        "targetx = x\ntargety = y\nif(spamTimer <= 0) then",
        InsertPosition::At,
        "targetx = x\ntargety = y\nclearUnitWaypoints()\nif(spamTimer <= 0) then",
    );
    let r = simulate(vec![(b, 10)], HALT);
    let lines = site_lines(&r, "gcore:main_lua:16");
    assert_eq!(lines.len(), 4, "{lines:?}");
    let (start, end) = edge_checks(&lines, "GCore › Clear pathfinding on Halt");
    assert!(!end.allowed);
    assert_eq!(
        end.reason.as_deref(),
        Some("would run only when spamTimer <= 0")
    );
    assert!(!start.allowed);
    assert!(start.reason.unwrap().contains("does not close every block"));
}

/// A change that closes what it opens has both edges.
#[test]
fn a_balanced_change_has_both_edges() {
    let b = pattern(
        "gcore",
        "gcore:wrap",
        "targetx = x",
        InsertPosition::At,
        "if ready then\n  targetx = x\nend",
    );
    let r = simulate(vec![(b, 10)], HALT);
    let lines = site_lines(&r, "gcore:wrap");
    let (start, end) = edge_checks(&lines, "GCore");
    assert!(start.allowed && end.allowed);
    assert_eq!(start.reason, None);
}

/// A change that closes a block it did not open: code after it would leave the block.
#[test]
fn an_edge_past_a_closed_block_is_greyed() {
    let (_, end) = edge_checks(&["  x()", "end"], "B");
    assert!(!end.allowed);
    assert_eq!(
        end.reason.as_deref(),
        Some("would land outside the block around B's change")
    );
}

#[test]
fn conditions_read_from_if_and_while_lines() {
    assert_eq!(
        condition_of("if(spamTimer <= 0) then").as_deref(),
        Some("spamTimer <= 0")
    );
    assert_eq!(
        condition_of("  elseif a and b then").as_deref(),
        Some("a and b")
    );
    assert_eq!(condition_of("while running do").as_deref(), Some("running"));
    assert_eq!(condition_of("if f(x) then").as_deref(), Some("f(x)"));
    assert_eq!(condition_of("for i = 1, 3 do"), None);
    assert_eq!(condition_of("iffy = 1"), None);
}

/// A patch placed by a rung is compared with where its own line puts it alone: the solo run
/// has no B, so the rung falls back (§9.2). Here B wraps the line in an `if`, so the code now
/// runs only inside it, and that is reported.
#[test]
fn anchored_patches_are_checked_against_their_own_line() {
    let src = "function update()\n  move()\nend\n";
    let b = pattern(
        "b",
        "b:wrap",
        "move()",
        InsertPosition::At,
        "if ready then\n  move(1)\nend",
    );
    let mut a = pattern("a", "a:log", "move()", InsertPosition::After, "log_move()");
    a.extra_mut().rungs = vec![(
        Rung {
            within: Some("b:wrap".to_string()),
            pattern: Some("move(1)".to_string()),
            exact: true,
            ..Rung::default()
        },
        AnchorSource::Profile {
            fix_id: "fx_1".to_string(),
        },
    )];
    let r = simulate(vec![(b, 10), (a, 20)], src);
    assert_eq!(
        r.traces["main.lua"].text,
        "function update()\n  if ready then\n    move(1)\n    log_move()\n  end\nend\n"
    );
    let p: Vec<_> = r
        .conflicts
        .iter()
        .filter(|c| c.kind == "placement_changed")
        .collect();
    assert_eq!(p.len(), 1, "{:?}", r.conflicts);
    assert_eq!(p[0].patch_a_id, "a:log");
    assert_eq!(p[0].mod_b_id.as_deref(), Some("b"));
}

// Task 37c: code an anchor fix moves (moved_scope, free_name_changed).

const MOVE_SRC: &str = "local scale = 2
function love.update(dt)
  step()
  rest()
end
function love.draw()
  local scale = 1
  paint()
end
";

fn table_of(patches: Vec<Patch>) -> PatchTable {
    let mut table = PatchTable::default();
    table.targets.insert("main.lua".to_string());
    for (i, p) in patches.into_iter().enumerate() {
        table.patches.push((p, 10 + i as Priority, PathBuf::from(format!("M{i}/qm.toml"))));
    }
    table
}

/// GCore rewrites the line Tactical Overhaul's patch anchors on, so it does not apply.
fn moved_base() -> Vec<Patch> {
    vec![
        pattern("gcore", "gcore:step", "step()", InsertPosition::At, "gcoreStep()"),
        pattern("to", "to:move", "step()", InsertPosition::After, "move(dt * scale)"),
    ]
}

/// The profile's anchor for `to:move`, as an Anchor fix gives it.
fn anchored(line: &str) -> Vec<Patch> {
    let mut v = moved_base();
    v[1].extra_mut().rungs = vec![(
        Rung { pattern: Some(line.to_string()), exact: true, ..Rung::default() },
        AnchorSource::Profile { fix_id: "fx_1".to_string() },
    )];
    v[1].extra_mut().replace_author_anchors = true;
    v
}

fn moved(next: Vec<Patch>) -> Vec<DetectedConflict> {
    let vanilla = HashMap::from([("main.lua".to_string(), MOVE_SRC.to_string())]);
    let (cur_t, next_t) = (table_of(moved_base()), table_of(next));
    let cur = simulate_and_analyze_patches(&cur_t, &vanilla);
    let rec = cur.files_touched[0].records.iter().find(|r| r.patch_id == "to:move").unwrap();
    assert_ne!(rec.status, PatchOutcomeStatus::Applied, "the patch has lost its line");
    let next = simulate_and_analyze_patches(&next_t, &vanilla);
    moved_code(&cur_t, &cur, &next_t, &next, &vanilla)
}

#[test]
fn an_anchor_fix_that_moves_code_to_another_function_is_flagged() {
    let found = moved(anchored("paint()"));
    let kinds: Vec<(&str, &str)> = found.iter().map(|c| (c.kind.as_str(), c.id.as_str())).collect();
    assert_eq!(
        kinds,
        vec![
            ("moved_scope", "semantic:moved_scope:main.lua:to:move:-@love.draw"),
            ("free_name_changed", "semantic:free_name_changed:main.lua:to:move:-@dt"),
            ("free_name_changed", "semantic:free_name_changed:main.lua:to:move:-@scale"),
        ]
    );
    assert!(found.iter().all(|c| c.severity == "warning" && crate::semantic::is_semantic(c)));
    assert!(
        found[0].description.contains("adds its code in `love.draw` at main.lua line 9; before, it was in `love.update`"),
        "{}",
        found[0].description
    );
    assert!(
        found[1].description.contains("means the global `dt`; before, it meant the local `dt` of `love.update` (`function love.update(dt)`)"),
        "{}",
        found[1].description
    );
    assert!(
        found[2].description.contains("the local `scale` of `love.draw` (`local scale = 1`); before, it meant the local `scale` of the file's main chunk"),
        "{}",
        found[2].description
    );
}

#[test]
fn an_anchor_fix_in_the_same_function_with_the_same_names_is_fine() {
    assert!(moved(anchored("rest()")).is_empty());
    // No change: nothing moved.
    assert!(moved(moved_base()).is_empty());
}

/// A new adjusted copy is compared with the patch it replaces.
#[test]
fn an_adjusted_copy_is_compared_with_its_base() {
    let mut next = moved_base();
    if let Patch::Pattern(p) = &mut next[1] {
        p.disabled = true;
    }
    let mut copy = pattern("to", "glue:to:move", "paint()", InsertPosition::After, "move(dt * scale)");
    copy.extra_mut().adjusted_copy = true;
    copy.extra_mut().replaces = Some("to:move".to_string());
    next.push(copy);
    let found = moved(next);
    assert_eq!(found.iter().filter(|c| c.kind == "moved_scope").count(), 1, "{found:?}");
    assert!(found.iter().all(|c| c.patch_a_id == "glue:to:move"), "{found:?}");
}

/// mod.json `overrides` silence these findings too.
#[test]
fn moved_code_findings_can_be_silenced() {
    let vanilla = HashMap::from([("main.lua".to_string(), MOVE_SRC.to_string())]);
    let cur_t = table_of(moved_base());
    let mut next_t = table_of(anchored("paint()"));
    next_t.mods = vec![qm_core::patch::plan::ModMeta {
        id: "to".to_string(),
        overrides: vec!["moved_scope:*".to_string(), "free_name_changed:dt".to_string()],
        ..Default::default()
    }];
    let cur = simulate_and_analyze_patches(&cur_t, &vanilla);
    let next = simulate_and_analyze_patches(&next_t, &vanilla);
    let found = moved_code(&cur_t, &cur, &next_t, &next, &vanilla);
    let names: Vec<&str> = found.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(names, vec!["semantic:free_name_changed:main.lua:to:move:-@scale"]);
}
