//! Tests for the suggestions: each kind on small mods staged in a temp folder, previewed the
//! way the service previews them (`table_for`, then the simulation).
use super::*;
use crate::conflict::{simulate_and_analyze_patches, DetectedConflict};
use crate::fixes::{op_disable, FIXES_VERSION};
use crate::profiles::builtin_vanilla_profile;
use qm_core::handshake::FixBinding;
use std::collections::HashSet;
use std::path::Path;
use tempfile::TempDir;

const HALT: &str = "      if key == \"h\" then
        player.deployedTroops[i].targetx = player.deployedTroops[i].x
        player.deployedTroops[i].targety = player.deployedTroops[i].y
        if(spamTimer <= 0) then
          newTextCue(\"Holding!\", player.deployedTroops[i].x, player.deployedTroops[i].y)
        end
      end
";

/// GCore's patch 16 (it rewrites the Halt lines and writes them back).
const GCORE: &str = "[manifest]
version = \"1.0.0\"
priority = 100

[[patches]]
[patches.pattern]
id = \"gcore:main_lua:16\"
name = \"Clear pathfinding on Halt\"
priority = 100
target = \"main.lua\"
pattern = '''
player.deployedTroops[i].targetx = player.deployedTroops[i].x
player.deployedTroops[i].targety = player.deployedTroops[i].y
if(spamTimer <= 0) then'''
position = \"at\"
match_indent = true
times = 1
payload = '''
player.deployedTroops[i].targetx = player.deployedTroops[i].x
player.deployedTroops[i].targety = player.deployedTroops[i].y
pathfinding.clearUnitWaypoints(player.deployedTroops[i])
if(spamTimer <= 0) then'''
";

/// Tactical Overhaul's patch 6 (after the targety line; it runs first by priority).
const TO: &str = "[manifest]
version = \"1.0.0\"
priority = 20

[[patches]]
[patches.pattern]
id = \"tactical_overhaul:main_lua:6\"
name = \"Halt reset\"
priority = 20
target = \"main.lua\"
pattern = \"player.deployedTroops[i].targety = player.deployedTroops[i].y\"
position = \"after\"
times = 1
payload = \"        player.deployedTroops[i].manualOrderTimer = 180\"
";

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

struct World {
    _t: TempDir,
    raw: RawLoad,
    vanilla: HashMap<String, String>,
    enabled: Vec<String>,
}

/// `mods`: (id, name, requires, qm.toml).
fn world(mods: &[(&str, &str, &[&str], &str)], main: &str) -> World {
    let t = TempDir::new().unwrap();
    for (id, name, requires, toml) in mods {
        let req: Vec<String> = requires.iter().map(|r| format!("\"{r}\": \"*\"")).collect();
        write(
            t.path(),
            &format!("{id}/mod.json"),
            &format!(
                r#"{{"id": "{id}", "name": "{name}", "version": "1.0.0", "relationships": {{"requires": {{{}}}}}}}"#,
                req.join(", ")
            ),
        );
        write(t.path(), &format!("{id}/qm.toml"), toml);
    }
    let raw = plan::load_raw(t.path(), None).unwrap();
    World {
        _t: t,
        raw,
        vanilla: HashMap::from([("main.lua".to_string(), main.to_string())]),
        enabled: mods.iter().map(|m| m.0.to_string()).collect(),
    }
}

fn profile(w: &World) -> ModProfile {
    let mut p = builtin_vanilla_profile(None);
    p.id = "p".into();
    p.enabled_mods = w.enabled.clone();
    p.fixes_version = FIXES_VERSION;
    p
}

fn sim(w: &World, p: &ModProfile) -> (PatchTable, SimulationReport) {
    let t = table_for(p, &w.raw, w.enabled.clone(), HashMap::new());
    let r = simulate_and_analyze_patches(&t, &w.vanilla);
    (t, r)
}

fn suggestions(
    w: &World,
    p: &ModProfile,
    bindings: &[FixBinding],
    budget: usize,
) -> Vec<Suggestion> {
    let (table, report) = sim(w, p);
    let ctx = SuggestCtx {
        profile: p,
        raw: Some(&w.raw),
        report: &report,
        table: &table,
        bindings,
        budget,
    };
    suggest(&ctx, &mut |q, files| {
        assert_eq!(files, ["main.lua"]);
        Some(sim(w, q).1)
    })
}

fn disable(p: &mut ModProfile, w: &World, id: &str, source: FixSource) -> String {
    let aliases = HashSet::from([id.to_string()]);
    op_disable(p, Some(&w.raw), id, &aliases, Provenance::new(source)).unwrap()
}

fn halt_world() -> World {
    world(
        &[
            ("gcore", "GCore", &[], GCORE),
            ("tactical_overhaul", "Tactical Overhaul", &[], TO),
        ],
        HALT,
    )
}

/// D26: the user's profile turns GCore's patch 16 off. Turning it back on alone breaks it
/// again (TO's line splits it), so the suggestion turns it on and keeps TO's line next to
/// GCore's copy of it. Applied, every patch applies.
#[test]
fn combined_d26_for_the_users_profile() {
    let w = halt_world();
    let mut p = profile(&w);
    let fix = disable(&mut p, &w, "gcore:main_lua:16", FixSource::Migrated);
    assert!(sim(&w, &p).1.conflicts.is_empty());

    let got = suggestions(&w, &p, &[], 8);
    assert_eq!(got.len(), 1, "{got:?}");
    let s = &got[0];
    assert_eq!(s.kind, SuggestionKind::Combined);
    assert_eq!(
        s.label,
        "Turn GCore › Clear pathfinding on Halt back on, and keep Tactical Overhaul's Halt reset next to GCore's copy of its line"
    );
    assert_eq!(s.removes, std::slice::from_ref(&fix));
    assert!(s.safe);
    let a = &s.adds[0];
    assert_eq!(a.patch_id, "tactical_overhaul:main_lua:6");
    assert_eq!(a.anchors[0].within.as_deref(), Some("gcore:main_lua:16"));
    assert_eq!(
        a.anchors[0].pattern.as_deref(),
        Some("player.deployedTroops[i].targety = player.deployedTroops[i].y")
    );
    assert!(a.anchors[0].exact);
    assert_eq!(a.sites.len(), 1);

    // Applied: both patches apply, nothing conflicts, GCore's lines stay whole.
    let q = apply(&p, Some(&w.raw), s);
    assert!(crate::fixes::find(&q, &fix).is_none());
    let anchor = crate::fixes::fixes(&q)
        .find(|f| matches!(f.change, FixChange::Anchor { .. }))
        .expect("an anchor fix");
    let FixChange::Anchor { anchors, .. } = &anchor.change else {
        unreachable!()
    };
    assert!(anchors[0].b_fp.is_some(), "B's fingerprint is pinned");
    assert!(anchor.snapshots.contains_key("gcore:main_lua:16"));
    let (_, r) = sim(&w, &q);
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_eq!(r.applied_patches, 2);
    assert!(r.traces["main.lua"].text.contains(
        "player.deployedTroops[i].targety = player.deployedTroops[i].y\n        player.deployedTroops[i].manualOrderTimer = 180\n        pathfinding.clearUnitWaypoints"
    ));
}

/// D26 in the user's real order (GCore first): turning patch 16 back on alone would work,
/// since GCore writes the targety line back and TO's code hangs on that copy. The suggestion
/// still ties TO to GCore's copy, as one change, instead of a bare "turn it back on".
#[test]
fn combined_d26_in_todays_order() {
    let gcore = GCORE.replace("priority = 100", "priority = 10");
    let to = TO.replace("priority = 20", "priority = 40");
    let w = world(
        &[
            ("gcore", "GCore", &[], &gcore),
            ("tactical_overhaul", "Tactical Overhaul", &[], &to),
        ],
        HALT,
    );
    let mut p = profile(&w);
    let fix = disable(&mut p, &w, "gcore:main_lua:16", FixSource::Migrated);
    let got = suggestions(&w, &p, &[], 8);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].kind, SuggestionKind::Combined);
    assert_eq!(got[0].removes, [fix]);
    assert!(got[0].safe);
    assert_eq!(got[0].adds[0].patch_id, "tactical_overhaul:main_lua:6");
    let (_, plain) = sim(
        &w,
        &crate::fixes::fixes(&p).fold(p.clone(), |mut q, f| {
            crate::fixes::undo(&mut q, Some(&w.raw), &f.id);
            q
        }),
    );
    let (_, r) = sim(&w, &apply(&p, Some(&w.raw), &got[0]));
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    // Same game code as turning it back on alone.
    assert_eq!(r.traces["main.lua"].text, plain.traces["main.lua"].text);
    let to_rec = r.files_touched[0]
        .records
        .iter()
        .find(|x| x.patch_id == "tactical_overhaul:main_lua:6")
        .unwrap();
    let site = &to_rec.anchoring.as_ref().unwrap().sites[0];
    assert_eq!(site.anchored_on.as_deref(), Some("gcore:main_lua:16"));
}

/// A Disable that is no longer needed: turning the patch back on adds nothing bad.
#[test]
fn stale_disable_is_suggested() {
    let w = halt_world();
    let mut p = profile(&w);
    let fix = disable(
        &mut p,
        &w,
        "tactical_overhaul:main_lua:6",
        FixSource::Resolver,
    );
    let got = suggestions(&w, &p, &[], 8);
    // TO on again splits GCore's lines, so it is not stale. With GCore off it is.
    assert!(
        got.iter().all(|s| s.kind != SuggestionKind::Stale),
        "{got:?}"
    );

    let w = world(&[("tactical_overhaul", "Tactical Overhaul", &[], TO)], HALT);
    let mut p = profile(&w);
    let fix2 = disable(
        &mut p,
        &w,
        "tactical_overhaul:main_lua:6",
        FixSource::Resolver,
    );
    let got = suggestions(&w, &p, &[], 8);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].kind, SuggestionKind::Stale);
    assert_eq!(got[0].label, "Turn Tactical Overhaul › Halt reset back on");
    assert_eq!((got[0].removes.clone(), got[0].safe), (vec![fix2], true));
    assert_ne!(fix, got[0].removes[0]);
}

/// A mod whose one patch adds `payload` after `anchor` in main.lua.
fn after_toml(id: &str, anchor: &str, payload: &str) -> String {
    format!(
        "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"{id}\"\nname = \"{id}\"\ntarget = \"main.lua\"\npattern = \"{anchor}\"\nposition = \"after\"\ntimes = 1\npayload = \"{payload}\"\n"
    )
}

/// A semantic finding (D69) is an approximate warning: it does not make turning a patch back
/// on unsafe, as it does not refuse a fix.
#[test]
fn a_semantic_warning_does_not_stop_a_stale_suggestion() {
    let a = after_toml("a:def", "-- top", "function helper() return 1 end");
    let b = after_toml("b:def", "-- bottom", "function helper() return 2 end");
    let w = world(
        &[("a", "Alpha", &[], &a), ("b", "Beta", &[], &b)],
        "-- top\n-- bottom\n",
    );
    let mut p = profile(&w);
    // Both on: B's definition replaces A's.
    let (_, both) = sim(&w, &p);
    assert!(
        both.conflicts
            .iter()
            .any(|c| crate::semantic::is_semantic(c) && c.kind == "redefined_global"),
        "{:?}",
        both.conflicts
    );
    let fix = disable(&mut p, &w, "b:def", FixSource::Resolver);
    let got = suggestions(&w, &p, &[], 8);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].kind, SuggestionKind::Stale);
    assert_eq!((got[0].removes.clone(), got[0].safe), (vec![fix], true));
}

/// A patch the user turned off by hand is a choice: never suggested back on.
#[test]
fn a_patch_turned_off_by_hand_is_left_alone() {
    let w = world(&[("tactical_overhaul", "Tactical Overhaul", &[], TO)], HALT);
    let mut p = profile(&w);
    disable(
        &mut p,
        &w,
        "tactical_overhaul:main_lua:6",
        FixSource::PatchToggle,
    );
    assert!(suggestions(&w, &p, &[], 8).is_empty());
}

const SRC: &str = "function f()\n  x()\n  y()\n  z()\nend\n";
const B_KEEPS_Y: &str = "[manifest]\nversion = \"1.0.0\"\npriority = 10\n\n[[patches]]\n[patches.pattern]\nid = \"b:1\"\nname = \"New x\"\npriority = 10\ntarget = \"main.lua\"\npattern = \"x()\\ny()\"\nposition = \"at\"\nmatch_indent = true\ntimes = 1\npayload = \"x2()\\ny()\"\n";
const A_AFTER_XY: &str = "[manifest]\nversion = \"1.0.0\"\npriority = 20\n\n[[patches]]\n[patches.pattern]\nid = \"a:1\"\nname = \"Log y\"\npriority = 20\ntarget = \"main.lua\"\npattern = \"x()\\ny()\"\nposition = \"after\"\ntimes = 1\npayload = \"  log_y()\"\n";

/// D11: B wrote A's anchor line back, A's mod requires B's mod: follow B's copy.
#[test]
fn follow_when_a_requires_b() {
    let w = world(
        &[
            ("b", "Base", &[], B_KEEPS_Y),
            ("a", "Addon", &["b"], A_AFTER_XY),
        ],
        SRC,
    );
    let p = profile(&w);
    let (_, r) = sim(&w, &p);
    assert!(r
        .conflicts
        .iter()
        .any(|c| c.id == "broken-anchor:main.lua:a:1"));
    let got = suggestions(&w, &p, &[], 8);
    assert_eq!(got.len(), 1, "{got:?}");
    let s = &got[0];
    assert_eq!(s.kind, SuggestionKind::Follow);
    assert_eq!(
        s.label,
        "Keep Addon's Log y next to Base's copy of its line"
    );
    assert_eq!(s.conflict_id.as_deref(), Some("broken-anchor:main.lua:a:1"));
    assert!(s.safe && s.removes.is_empty());
    let (_, after) = sim(&w, &apply(&p, Some(&w.raw), s));
    assert!(after.conflicts.is_empty(), "{:?}", after.conflicts);
    assert_eq!(
        after.traces["main.lua"].text,
        "function f()\n  x2()\n  y()\n  log_y()\n  z()\nend\n"
    );
}

/// Without the dependency, nothing is proposed: B's line is not A's to follow.
#[test]
fn no_follow_without_a_dependency() {
    let w = world(
        &[
            ("b", "Base", &[], B_KEEPS_Y),
            ("a", "Addon", &[], A_AFTER_XY),
        ],
        SRC,
    );
    let got = suggestions(&w, &profile(&w), &[], 8);
    assert!(got.is_empty(), "{got:?}");
}

/// An Anchor fix whose rungs all use the patch's own line, while the patch applies as
/// written, is no longer needed.
#[test]
fn retire_an_anchor_fix_that_falls_back() {
    let w = world(
        &[
            ("b", "Base", &[], B_KEEPS_Y),
            ("a", "Addon", &["b"], A_AFTER_XY),
        ],
        SRC,
    );
    let mut p = profile(&w);
    disable(&mut p, &w, "b:1", FixSource::PatchToggle);
    let fid = crate::fixes::op_anchor_set(
        &mut p,
        Some(&w.raw),
        "a:1",
        vec![Rung {
            within: Some("b:1".to_string()),
            pattern: Some("y()".to_string()),
            exact: true,
            ..Rung::default()
        }],
        Vec::new(),
        None,
        Provenance::new(FixSource::Resolver),
    )
    .unwrap();
    let got = suggestions(&w, &p, &[], 8);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].kind, SuggestionKind::Retire);
    assert_eq!(
        (got[0].removes.clone(), got[0].safe),
        (vec![fid.clone()], true)
    );
    assert_eq!(got[0].fix_id.as_deref(), Some(fid.as_str()));
}

/// A fix whose patch moved asks the user to check it; no preview runs.
#[test]
fn rebind_asks_without_a_preview() {
    let w = world(&[("tactical_overhaul", "Tactical Overhaul", &[], TO)], HALT);
    let mut p = profile(&w);
    let fid = disable(
        &mut p,
        &w,
        "tactical_overhaul:main_lua:6",
        FixSource::PatchToggle,
    );
    let b = FixBinding {
        fix_id: fid.clone(),
        key: "disable:tactical_overhaul:old".to_string(),
        status: BindingStatus::Rebound {
            from: "tactical_overhaul:old".to_string(),
            to: "tactical_overhaul:main_lua:6".to_string(),
        },
        detail: String::new(),
        new_id: Some("tactical_overhaul:main_lua:6".to_string()),
    };
    let got = suggestions(&w, &p, &[b], 0);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].kind, SuggestionKind::Rebind);
    assert_eq!(got[0].fix_id.as_deref(), Some(fid.as_str()));
    assert!(got[0].adds.is_empty() && got[0].removes.is_empty() && !got[0].safe);
}

/// With no previews left, nothing that needs one is listed.
#[test]
fn the_budget_bounds_the_previews() {
    let w = halt_world();
    let mut p = profile(&w);
    disable(&mut p, &w, "gcore:main_lua:16", FixSource::Migrated);
    assert!(suggestions(&w, &p, &[], 1).is_empty());
    assert!(suggestions(&w, &p, &[], 0).is_empty());
}

/// An info note (`multi_match`) is not a conflict: a change that adds or removes one neither
/// adds nor resolves a conflict.
#[test]
fn info_notes_are_not_counted_in_the_delta() {
    let w = halt_world();
    let (_, cur) = sim(&w, &profile(&w));
    let note = |id: &str, severity: &str| DetectedConflict {
        id: id.into(),
        severity: severity.into(),
        kind: "multi_match".into(),
        target_file: "main.lua".into(),
        mod_a_id: "gcore".into(),
        mod_b_id: None,
        patch_a_id: "gcore:main_lua:16".into(),
        patch_b_id: None,
        description: String::new(),
        suggestions: Vec::new(),
        consumer: None,
        sites_lost: None,
        same_line: None,
    };
    let files = ["main.lua".to_string()];
    let mut next = cur.clone();
    next.conflicts.push(note("multi:a", "info"));
    let d = delta(&cur, &next, &files);
    assert!(d.new_conflicts.is_empty() && d.resolved.is_empty());
    let d = delta(&next, &cur, &files);
    assert!(d.new_conflicts.is_empty() && d.resolved.is_empty());
    // A warning still counts.
    next.conflicts.push(note("multi:b", "warning"));
    let d = delta(&cur, &next, &files);
    assert_eq!(d.new_conflicts, BTreeSet::from(["multi:b".to_string()]));
}
