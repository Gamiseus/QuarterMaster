use super::*;
use crate::conflict::{simulate_and_analyze_patches, SimulationReport};
use qm_core::compile::LuaCompiler;
use qm_core::patch::module::ModulePatch;
use qm_core::patch::{InsertPosition, PatternPatch, Priority};
use std::path::PathBuf;

fn pattern(mod_id: &str, id: &str, target: &str, anchor: &str, payload: &str) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: anchor.to_string(),
        position: InsertPosition::After,
        target: Target::Single(target.to_string()),
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

fn no_times(mut p: Patch) -> Patch {
    if let Patch::Pattern(x) = &mut p {
        x.times = None;
    }
    p
}

fn module(mod_id: &str, id: &str, name: &str, content: &str) -> Patch {
    let mut m: ModulePatch = serde_json::from_value(serde_json::json!({
        "source": "m.lua", "name": name, "before": "main.lua", "id": id
    }))
    .unwrap();
    m.mod_id = Some(mod_id.to_string());
    m.content = content.to_string();
    m.display_source = "m.lua".to_string();
    Patch::Module(m)
}

fn table(patches: Vec<(Patch, Priority)>) -> PatchTable {
    let mut t = PatchTable::default();
    for (i, (p, prio)) in patches.into_iter().enumerate() {
        for n in target_names(&p) {
            t.targets.insert(n.to_string());
        }
        t.patches
            .push((p, prio, PathBuf::from(format!("M{i}/qm.toml"))));
    }
    t
}

fn files(list: &[(&str, &str)]) -> HashMap<String, String> {
    list.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn of_kind<'a>(r: &'a SimulationReport, kind: &str) -> Vec<&'a DetectedConflict> {
    r.conflicts.iter().filter(|c| c.kind == kind).collect()
}

#[test]
fn a_module_named_like_a_game_file_shadows_it() {
    let t = table(vec![
        (
            module("gcore", "gcore:main_lua:1", "includes/ser", "return {}"),
            5,
        ),
        (
            pattern(
                "other",
                "other:ser",
                "includes/ser.lua",
                "local ser = {}",
                "ser.x = 1",
            ),
            10,
        ),
    ]);
    let vanilla = files(&[
        ("main.lua", "local x = 1\n"),
        ("includes/ser.lua", "local ser = {}\nreturn ser\n"),
        ("includes/pathfinding.lua", "return {}\n"),
    ]);
    let r = simulate_and_analyze_patches(&t, &vanilla);
    let s = of_kind(&r, "shadowed_file");
    assert_eq!(s.len(), 1, "{:?}", r.conflicts);
    assert_eq!(s[0].severity, "warning");
    assert_eq!(s[0].target_file, "includes/ser.lua");
    assert_eq!(s[0].mod_a_id, "gcore");
    assert_eq!(s[0].patch_a_id, "gcore:main_lua:1");
    assert!(
        s[0].description
            .starts_with("'includes/ser.lua' won't run: 'gcore' replaces this file"),
        "{}",
        s[0].description
    );
    // The patch on it is flagged as not running.
    let f = r
        .files_touched
        .iter()
        .find(|f| f.target_file == "includes/ser.lua")
        .unwrap();
    let rec = f
        .records
        .iter()
        .find(|x| x.patch_id == "other:ser")
        .unwrap();
    assert!(is_shadowed(rec), "{:?}", rec.warnings);
    assert!(
        rec.warnings[0].starts_with("shadowed_by_module:gcore:"),
        "{:?}",
        rec.warnings
    );
}

#[test]
fn short_targets_resolve_like_the_game() {
    let t = table(vec![
        (
            pattern("a", "a:urb", "urb_engine.lua", "local u = 1", "local v = 2"),
            10,
        ),
        (
            no_times(pattern(
                "b",
                "b:ser",
                "ser.lua",
                "local s = 1",
                "local t = 2",
            )),
            10,
        ),
        (
            pattern(
                "c",
                "c:ser",
                "includes/ser.lua",
                "local s = 1",
                "local w = 3",
            ),
            20,
        ),
    ]);
    let vanilla = files(&[
        ("main.lua", "local x = 1\n"),
        ("includes/urb_engine.lua", "local u = 1\n"),
        ("includes/ser.lua", "local s = 1\n"),
        ("includes/binser.lua", "local s = 1\n"),
    ]);
    let r = simulate_and_analyze_patches(&t, &vanilla);
    let names: Vec<&str> = r
        .files_touched
        .iter()
        .map(|f| f.target_file.as_str())
        .collect();
    // Each file once, in alphabetical order.
    assert_eq!(
        names,
        vec![
            "includes/binser.lua",
            "includes/ser.lua",
            "includes/urb_engine.lua"
        ]
    );
    let urb = &r.files_touched[2];
    assert_eq!(urb.records.len(), 1);
    assert_eq!(urb.records[0].status, PatchOutcomeStatus::Applied);
    assert!(r.traces["includes/urb_engine.lua"]
        .text
        .contains("local v = 2"));
    // ser.lua and includes/ser.lua both name includes/ser.lua: one file, both patches.
    let ser = &r.files_touched[1];
    let ids: Vec<&str> = ser.records.iter().map(|x| x.patch_id.as_str()).collect();
    assert_eq!(ids, vec!["b:ser", "c:ser"]);

    let several = of_kind(&r, "target_matches_several");
    assert_eq!(several.len(), 1, "{:?}", r.conflicts);
    assert_eq!(several[0].id, "target-several:ser.lua");
    assert_eq!(several[0].patch_a_id, "b:ser");
    assert!(
        several[0]
            .description
            .contains("includes/binser.lua, includes/ser.lua"),
        "{}",
        several[0].description
    );
    let boundary = of_kind(&r, "target_without_boundary");
    assert_eq!(boundary.len(), 1, "{:?}", r.conflicts);
    assert_eq!(boundary[0].target_file, "includes/binser.lua");
}

#[test]
fn an_aborting_patch_blocks_and_keeps_its_file() {
    let mut abort = pattern(
        "a",
        "a:gone",
        "main.lua",
        "local missing = 0",
        "local y = 2",
    );
    if let Patch::Pattern(x) = &mut abort {
        x.on_failure = OnFailurePolicy::Abort;
    }
    let t = table(vec![
        (
            pattern("b", "b:ok", "main.lua", "local x = 1", "local z = 3"),
            10,
        ),
        (abort, 20),
    ]);
    let vanilla = files(&[("main.lua", "local x = 1\n")]);
    let r = simulate_and_analyze_patches(&t, &vanilla);
    let a = of_kind(&r, "patch_aborts_file");
    assert_eq!(a.len(), 1, "{:?}", r.conflicts);
    assert_eq!(a[0].severity, "error");
    assert_eq!(a[0].target_file, "main.lua");
    assert_eq!(a[0].mod_a_id, "a");
    assert_eq!(a[0].patch_a_id, "a:gone");
    assert!(
        a[0].description.contains("fails to load 'main.lua'"),
        "{}",
        a[0].description
    );
    // The file stays in the report, with the record that aborts.
    let f = r
        .files_touched
        .iter()
        .find(|f| f.target_file == "main.lua")
        .unwrap();
    let rec = f.records.iter().find(|x| x.patch_id == "a:gone").unwrap();
    assert_eq!(rec.status, PatchOutcomeStatus::Aborted);
    assert_eq!(rec.on_failure, OnFailurePolicy::Abort);
    assert!(f
        .records
        .iter()
        .any(|x| x.patch_id == "b:ok" && x.status == PatchOutcomeStatus::Applied));
}

#[test]
fn the_module_preloaded_last_wins_and_identical_modules_are_fine() {
    // Table order a, b; the DLL preloads by priority, so b (10) then a (20): a wins.
    let t = table(vec![
        (module("a", "a:m", "m", "return 1"), 20),
        (module("b", "b:m", "m", "return 2"), 10),
    ]);
    let order = t.prepare().order().order.clone();
    let last = order
        .iter()
        .rev()
        .find_map(|&i| match &t.patches[i].0 {
            Patch::Module(m) => m.mod_id.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(last, "a");
    let vanilla = files(&[("main.lua", "local x = 1\n")]);
    let r = simulate_and_analyze_patches(&t, &vanilla);
    let d = of_kind(&r, "duplicate_module");
    assert_eq!(d.len(), 1, "{:?}", r.conflicts);
    assert_eq!(d[0].mod_b_id.as_deref(), Some("a"));
    assert_eq!(d[0].patch_b_id.as_deref(), Some("a:m"));
    assert_eq!(d[0].mod_a_id, "b");
    assert_eq!(
        r.module_registrations["m"],
        vec!["b".to_string(), "a".to_string()]
    );

    let same = table(vec![
        (module("a", "a:m", "m", "return 1"), 20),
        (module("b", "b:m", "m", "return 1"), 10),
    ]);
    let r = simulate_and_analyze_patches(&same, &vanilla);
    assert!(
        of_kind(&r, "duplicate_module").is_empty(),
        "{:?}",
        r.conflicts
    );
}

#[test]
fn a_load_now_module_wins_whatever_its_place() {
    // b runs first by priority but is load_now: the DLL writes it when its `before` chunk
    // loads, after main.lua preloaded a, so b wins.
    let mut b = module("b", "b:m", "m", "return 2");
    if let Patch::Module(m) = &mut b {
        m.load_now = true;
    }
    let t = table(vec![(module("a", "a:m", "m", "return 1"), 20), (b, 10)]);
    let vanilla = files(&[("main.lua", "local x = 1\n"), ("m.lua", "local y = 1\n")]);
    let r = simulate_and_analyze_patches(&t, &vanilla);
    let d = of_kind(&r, "duplicate_module");
    assert_eq!(d.len(), 1, "{:?}", r.conflicts);
    assert_eq!(d[0].mod_b_id.as_deref(), Some("b"));
    assert_eq!(d[0].mod_a_id, "a");
    let s = of_kind(&r, "shadowed_file");
    assert_eq!(s.len(), 1, "{:?}", r.conflicts);
    assert_eq!(s[0].mod_a_id, "b");
    assert_eq!(
        r.module_registrations["m"],
        vec!["a".to_string(), "b".to_string()]
    );
}

#[test]
fn a_copy_of_an_anchor_is_a_gained_place() {
    let src = "function f()\n  move()\nend\n";
    let t = table(vec![
        (
            pattern(
                "dup",
                "dup:1",
                "main.lua",
                "  move()",
                "  if fast then\n    move()\n  end",
            ),
            10,
        ),
        (
            no_times(pattern("log", "log:1", "main.lua", "move()", "  log()")),
            20,
        ),
    ]);
    let r = simulate_and_analyze_patches(&t, &files(&[("main.lua", src)]));
    let g = of_kind(&r, "sites_gained");
    assert_eq!(g.len(), 1, "{:?}", r.conflicts);
    assert_eq!(g[0].patch_a_id, "log:1");
    assert_eq!(g[0].patch_b_id.as_deref(), Some("dup:1"));
    assert!(
        g[0].description.contains("matches 2 places"),
        "{}",
        g[0].description
    );
    // One place in the game's file: no multi_match note.
    assert!(of_kind(&r, "multi_match").is_empty(), "{:?}", r.conflicts);
}

/// A patch with no place in the game's file is meant for another mod's code: nothing gained.
#[test]
fn a_patch_on_another_mods_code_gains_nothing() {
    let src = "function f()\n  move()\nend\n";
    let t = table(vec![
        (pattern("a", "a:1", "main.lua", "  move()", "  jump()"), 10),
        (
            no_times(pattern("b", "b:1", "main.lua", "jump()", "  land()")),
            20,
        ),
    ]);
    let r = simulate_and_analyze_patches(&t, &files(&[("main.lua", src)]));
    assert!(r.traces["main.lua"].text.contains("land()"));
    assert!(of_kind(&r, "sites_gained").is_empty(), "{:?}", r.conflicts);
}

#[test]
fn a_patch_without_times_on_three_places_gets_a_note() {
    let src = "function a()\n  tick()\n  tick()\nend\nfunction b()\n  tick()\nend\n";
    let t = table(vec![
        (
            no_times(pattern("m", "m:tick", "main.lua", "tick()", "  log()")),
            10,
        ),
        (
            pattern("n", "n:tick", "main.lua", "function b()", "  -- b"),
            10,
        ),
    ]);
    let r = simulate_and_analyze_patches(&t, &files(&[("main.lua", src)]));
    let m = of_kind(&r, "multi_match");
    assert_eq!(m.len(), 1, "{:?}", r.conflicts);
    assert_eq!(m[0].severity, "info");
    assert_eq!(m[0].patch_a_id, "m:tick");
    assert!(
        m[0].description.contains("matches 3 places in 2 functions"),
        "{}",
        m[0].description
    );
}

#[test]
fn two_mods_with_one_var_and_different_values() {
    let dir = tempfile::tempdir().unwrap();
    let mods = dir.path();
    std::fs::create_dir_all(mods.join("qm")).unwrap();
    for (name, speed) in [("Alpha", "1"), ("Beta", "2")] {
        let m = mods.join(name);
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(
            m.join("qm.toml"),
            format!(
                "[manifest]\nversion = \"1.0.0\"\n\n[vars]\nSPEED = \"{speed}\"\n\n[[patches]]\n[patches.pattern]\n\
                 target = \"main.lua\"\npattern = \"local x = 1\"\nposition = \"after\"\npayload = \"local s = {{{{SPEED}}}}\"\n"
            ),
        )
        .unwrap();
    }
    let t = PatchTable::load(mods).unwrap();
    let r = simulate_and_analyze_patches(&t, &files(&[("main.lua", "local x = 1\n")]));
    let v = of_kind(&r, "var_collision");
    assert_eq!(v.len(), 1, "{:?}", r.conflicts);
    let mods_named = [
        v[0].mod_a_id.as_str(),
        v[0].mod_b_id.as_deref().unwrap_or_default(),
    ];
    assert!(
        mods_named.contains(&"alpha") && mods_named.contains(&"beta"),
        "{mods_named:?}"
    );
    assert!(v[0].description.contains("'SPEED'"), "{}", v[0].description);
}

/// The game's Lua next to the project, for the compile check, when it is there.
fn game_lua() -> Option<LuaCompiler> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    [root.join("../../../game"), root.join("../../game")]
        .iter()
        .find_map(|d| LuaCompiler::find(d))
        .and_then(|p| LuaCompiler::load(&p).ok())
}

fn module_at(mod_id: &str, name: &str, source: &str, content: &str, load_now: bool) -> Patch {
    let mut p = module(mod_id, &format!("{mod_id}:m"), name, content);
    if let Patch::Module(m) = &mut p {
        m.source = PathBuf::from(source);
        m.display_source = source.to_string();
        m.load_now = load_now;
    }
    p
}

fn simulate_with(t: &PatchTable, vanilla: &HashMap<String, String>, lua: &LuaCompiler) -> SimulationReport {
    crate::conflict::simulate_and_analyze_patches_with(
        t,
        vanilla,
        crate::conflict::SimulationOptions {
            compiler: Some(lua),
            launch: None,
        },
    )
}

/// §6.2: a module with a syntax error is a blocking `compile_error` on the module, at the
/// module file's own line; a module that compiles gives nothing.
#[test]
fn a_module_that_does_not_compile_blocks_a_launch() {
    let Some(lua) = game_lua() else {
        eprintln!("lua51.dll not found; skipped");
        return;
    };
    let t = table(vec![
        (module_at("good", "lib/good", "good.lua", "local M = {}\nreturn M\n", false), 10),
        (
            module_at("bad", "lib/bad", "src/bad.lua", "local M = {}\nfunction M.f(\nreturn M\n", false),
            20,
        ),
    ]);
    let vanilla = files(&[("main.lua", "local x = 1\n")]);
    let r = simulate_with(&t, &vanilla, &lua);
    let c = of_kind(&r, "compile_error");
    assert_eq!(c.len(), 1, "{:?}", r.conflicts);
    assert_eq!(c[0].severity, "error");
    assert_eq!(c[0].id, "compile-module:lib/bad:bad:m");
    assert_eq!(c[0].target_file, "lib/bad");
    assert_eq!((c[0].mod_a_id.as_str(), c[0].patch_a_id.as_str()), ("bad", "bad:m"));
    assert!(
        c[0].description
            .starts_with("Module 'lib/bad' from 'bad' (src/bad.lua) does not compile: line 3: "),
        "{}",
        c[0].description
    );
    assert!(
        c[0].description.ends_with("The game would stop with a Lua error when a mod requires this module."),
        "{}",
        c[0].description
    );

    // Without the game's Lua nothing is compiled.
    let r = simulate_and_analyze_patches(&t, &vanilla);
    assert!(of_kind(&r, "compile_error").is_empty(), "{:?}", r.conflicts);
}

/// A load_now module runs when its `before` file loads; a broken module another module of the
/// same name replaces is never used, so it only warns.
#[test]
fn load_now_and_replaced_modules_that_do_not_compile() {
    let Some(lua) = game_lua() else {
        eprintln!("lua51.dll not found; skipped");
        return;
    };
    let t = table(vec![(module_at("now", "boot", "boot.lua", "if then end\n", true), 10)]);
    let vanilla = files(&[("main.lua", "local x = 1\n")]);
    let r = simulate_with(&t, &vanilla, &lua);
    let c = of_kind(&r, "compile_error");
    assert_eq!(c.len(), 1, "{:?}", r.conflicts);
    assert!(
        c[0].description.ends_with("The game would stop with a Lua error when it loads 'main.lua'."),
        "{}",
        c[0].description
    );

    // `old` (priority 10) is preloaded first and `new` (20) replaces it.
    let t = table(vec![
        (module_at("old", "util", "util.lua", "return {\n", false), 10),
        (module_at("new", "util", "util.lua", "return {}\n", false), 20),
    ]);
    let r = simulate_with(&t, &vanilla, &lua);
    assert!(of_kind(&r, "compile_error").is_empty(), "{:?}", r.conflicts);
    let w = of_kind(&r, REPLACED_MODULE_COMPILE_ERROR);
    assert_eq!(w.len(), 1, "{:?}", r.conflicts);
    assert_eq!(w[0].severity, "warning");
    assert_eq!(w[0].mod_a_id, "old");
    assert_eq!(w[0].mod_b_id.as_deref(), Some("new"));
    assert!(w[0].description.contains("The game does not use it: 'new' replaces it"), "{}", w[0].description);

    // Swapped: the broken one wins, so it blocks.
    let t = table(vec![
        (module_at("old", "util", "util.lua", "return {\n", false), 20),
        (module_at("new", "util", "util.lua", "return {}\n", false), 10),
    ]);
    let r = simulate_with(&t, &vanilla, &lua);
    assert_eq!(of_kind(&r, "compile_error").len(), 1, "{:?}", r.conflicts);
    assert!(of_kind(&r, REPLACED_MODULE_COMPILE_ERROR).is_empty());
}

/// D44: modules are not patch targets. A patch whose target names a module's source file or
/// `<name>.lua`, and no game file, never runs and warns `target_is_module`.
#[test]
fn a_patch_on_a_module_source_warns() {
    let mut off = pattern("p", "p:off", "lib/util.lua", "local M = {}", "-- off");
    off.set_disabled(true);
    let t = table(vec![
        (module_at("lib", "mylib/core", "lib/util.lua", "local M = {}\nreturn M\n", false), 10),
        (pattern("p", "p:source", "lib/util.lua", "local M = {}", "M.x = 1"), 20),
        (pattern("p", "p:short", "util.lua", "local M = {}", "M.x = 1"), 20),
        (pattern("p", "p:name", "mylib/core.lua", "local M = {}", "M.x = 1"), 20),
        (pattern("p", "p:game", "includes/ser.lua", "local ser = {}", "ser.x = 1"), 20),
        (pattern("p", "p:other", "nothing.lua", "local M = {}", "M.x = 1"), 20),
        (off, 20),
    ]);
    let vanilla = files(&[
        ("main.lua", "local x = 1\n"),
        ("includes/ser.lua", "local ser = {}\nreturn ser\n"),
    ]);
    let r = simulate_and_analyze_patches(&t, &vanilla);
    let w = of_kind(&r, "target_is_module");
    let mut ids: Vec<&str> = w.iter().map(|c| c.patch_a_id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["p:name", "p:short", "p:source"], "{:?}", r.conflicts);
    let c = w.iter().find(|c| c.patch_a_id == "p:source").unwrap();
    assert_eq!(c.severity, "warning");
    assert_eq!(c.target_file, "lib/util.lua");
    assert_eq!(c.mod_b_id.as_deref(), Some("lib"));
    assert_eq!(c.patch_b_id.as_deref(), Some("lib:m"));
    assert_eq!(
        c.description,
        "The target 'lib/util.lua' of 'p:source' (from 'p') is the module 'mylib/core' ('lib/util.lua' in 'lib'), not a game file. Modules are not patch targets, so this patch never runs."
    );

    // A target that also names a game file patches it: no warning.
    let t = table(vec![
        (module_at("lib", "x", "ser.lua", "return {}\n", false), 10),
        (pattern("p", "p:ser", "ser.lua", "local ser = {}", "ser.x = 1"), 20),
    ]);
    let r = simulate_and_analyze_patches(&t, &vanilla);
    assert!(of_kind(&r, "target_is_module").is_empty(), "{:?}", r.conflicts);
}
