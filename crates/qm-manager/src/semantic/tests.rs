use std::collections::HashMap;
use std::path::PathBuf;

use qm_core::patch::module::ModulePatch;
use qm_core::patch::plan::ModMeta;
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, OnFailurePolicy, Patch, PatternPatch, Target};

use super::RunAt;
use crate::conflict::{simulate_and_analyze_patches, DetectedConflict, SimulationReport};

fn pattern(mod_id: &str, id: &str, target: &str, anchor: &str, position: InsertPosition, payload: &str) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: anchor.to_string(),
        position,
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

fn after(mod_id: &str, id: &str, anchor: &str, payload: &str) -> Patch {
    pattern(mod_id, id, "main.lua", anchor, InsertPosition::After, payload)
}

fn module(mod_id: &str, name: &str, content: &str) -> Patch {
    Patch::Module(ModulePatch {
        source: PathBuf::from(format!("{name}.lua")),
        before: None,
        name: name.to_string(),
        load_now: false,
        id: Some(format!("{mod_id}:module")),
        priority: None,
        on_failure: OnFailurePolicy::Warn,
        mod_id: Some(mod_id.to_string()),
        disabled: false,
        qm: Default::default(),
        display_source: String::new(),
        content: content.to_string(),
    })
}

fn table(patches: Vec<Patch>, files: &[(&str, &str)]) -> (PatchTable, HashMap<String, String>) {
    let mut t = PatchTable::default();
    for (i, p) in patches.into_iter().enumerate() {
        if let Patch::Pattern(pp) = &p {
            if let Target::Single(f) = &pp.target {
                t.targets.insert(f.clone());
            }
        }
        t.patches.push((p, 10 + i as i32, PathBuf::from(format!("M{i}/qm.toml"))));
    }
    let vanilla = files.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    (t, vanilla)
}

fn run(patches: Vec<Patch>, files: &[(&str, &str)]) -> SimulationReport {
    let (t, vanilla) = table(patches, files);
    simulate_and_analyze_patches(&t, &vanilla)
}

fn of_kind<'a>(r: &'a SimulationReport, kind: &str) -> Vec<&'a DetectedConflict> {
    r.conflicts.iter().filter(|c| c.kind == kind).collect()
}

const SETTINGS_SRC: &str = "\
-- top
-- bottom
function love.load()
  -- load
end
";

const WRAP: &str = "local _orig = loadModSettings
function loadModSettings()
  if _orig then _orig() end
end";
const PLAIN: &str = "function loadModSettings()
  read()
end";

/// A mod that only calls its copy keeps the value from before the other mod's definition.
#[test]
fn a_copy_that_runs_before_the_definition_is_flagged() {
    let r = run(
        vec![
            after("a", "a:copy", "-- top", "local _orig = loadModSettings"),
            after("gcore", "gcore:settings", "-- bottom", PLAIN),
        ],
        &[("main.lua", SETTINGS_SRC)],
    );
    let c = of_kind(&r, "capture_before_define");
    assert_eq!(c.len(), 1, "{:#?}", r.conflicts);
    assert_eq!(c[0].severity, "warning");
    assert_eq!((c[0].mod_a_id.as_str(), c[0].mod_b_id.as_deref()), ("a", Some("gcore")));
    assert_eq!(c[0].patch_a_id, "a:copy");
    assert_eq!(c[0].target_file, "main.lua");
    assert_eq!(c[0].id, "semantic:capture_before_define:main.lua:a:copy:gcore:settings@loadModSettings");
    assert!(c[0].description.contains("copies `loadModSettings` into `_orig` at main.lua line 2"), "{}", c[0].description);
}

/// A wrapper whose copy runs before another mod's definition, installed after it, drops
/// that definition.
#[test]
fn a_wrapper_installed_after_the_definition_it_missed_is_flagged() {
    let src = "-- top
-- middle
-- bottom
";
    let r = run(
        vec![
            after("to", "to:copy", "-- top", "local _orig = loadModSettings"),
            after("gcore", "gcore:settings", "-- middle", PLAIN),
            after("to", "to:def", "-- bottom", "function loadModSettings()
  if _orig then _orig() end
end"),
        ],
        &[("main.lua", src)],
    );
    let c = of_kind(&r, "capture_before_define");
    assert_eq!(c.len(), 1, "{:#?}", r.conflicts);
    assert_eq!((c[0].mod_a_id.as_str(), c[0].mod_b_id.as_deref()), ("to", Some("gcore")));
}

/// Tactical Overhaul's settings block wraps `loadModSettings` and runs first; a plain GCore
/// definition later drops the wrapper. The copy itself is not the problem.
#[test]
fn a_plain_definition_after_a_wrapper_is_flagged() {
    let r = run(
        vec![after("to", "to:settings", "-- top", WRAP), after("gcore", "gcore:settings", "-- bottom", PLAIN)],
        &[("main.lua", SETTINGS_SRC)],
    );
    assert!(of_kind(&r, "capture_before_define").is_empty(), "{:#?}", r.conflicts);
    let d = of_kind(&r, "redefined_global");
    assert_eq!(d.len(), 1, "{:#?}", r.conflicts);
    assert_eq!((d[0].mod_a_id.as_str(), d[0].mod_b_id.as_deref()), ("to", Some("gcore")));
    assert!(d[0].description.contains("'gcore''s definition runs later"), "{}", d[0].description);
}

/// Wrappers that chain, in either order (the user's GCore and Tactical Overhaul today): nothing
/// to report.
#[test]
fn chained_wrappers_are_fine() {
    let gcore = WRAP.replace("_orig", "_prev");
    for (first, second) in [("to", "gcore"), ("gcore", "to")] {
        let body = |m: &str| if m == "to" { WRAP.to_string() } else { gcore.clone() };
        let r = run(
            vec![
                after(first, &format!("{first}:settings"), "-- top", &body(first)),
                after(second, &format!("{second}:settings"), "-- bottom", &body(second)),
            ],
            &[("main.lua", SETTINGS_SRC)],
        );
        assert!(of_kind(&r, "capture_before_define").is_empty(), "{:#?}", r.conflicts);
        assert!(of_kind(&r, "redefined_global").is_empty(), "{:#?}", r.conflicts);
    }
    // A wrapper after a plain definition keeps the chain too.
    let r = run(
        vec![after("gcore", "gcore:settings", "-- top", PLAIN), after("to", "to:settings", "-- bottom", WRAP)],
        &[("main.lua", SETTINGS_SRC)],
    );
    assert!(of_kind(&r, "capture_before_define").is_empty(), "{:#?}", r.conflicts);
    assert!(of_kind(&r, "redefined_global").is_empty(), "{:#?}", r.conflicts);
}

/// A definition inside `love.load` runs after every main chunk.
#[test]
fn love_load_runs_after_the_main_chunks() {
    let r = run(
        vec![
            after("a", "a:copy", "-- bottom", "local _h = helper"),
            after("b", "b:def", "  -- load", "  function helper() end"),
        ],
        &[("main.lua", SETTINGS_SRC)],
    );
    let c = of_kind(&r, "capture_before_define");
    assert_eq!(c.len(), 1, "{:#?}", r.conflicts);
    assert_eq!(c[0].mod_b_id.as_deref(), Some("b"));
}

/// A required file's main chunk runs where its `require` is; a module wins its name.
#[test]
fn requires_follow_require_order() {
    let main = "-- early\nrequire \"includes/defs\"\n-- late\n";
    let defs = "-- defs\n";
    let files = [("main.lua", main), ("includes/defs.lua", defs)];
    let def = pattern("b", "b:def", "includes/defs.lua", "-- defs", InsertPosition::After, "function helper() end");
    // After the require: the copy sees the definition.
    let r = run(vec![after("a", "a:copy", "-- late", "local _h = helper"), def.clone()], &files);
    assert!(of_kind(&r, "capture_before_define").is_empty(), "{:#?}", r.conflicts);
    // Before it: flagged.
    let r = run(vec![after("a", "a:copy", "-- early", "local _h = helper"), def], &files);
    assert_eq!(of_kind(&r, "capture_before_define").len(), 1, "{:#?}", r.conflicts);

    // The same with a module named like the required file.
    let m = module("b", "includes/defs", "function helper() end\n");
    let r = run(vec![after("a", "a:copy", "-- late", "local _h = helper"), m.clone()], &files);
    assert!(of_kind(&r, "capture_before_define").is_empty(), "{:#?}", r.conflicts);
    let r = run(vec![after("a", "a:copy", "-- early", "local _h = helper"), m], &files);
    let c = of_kind(&r, "capture_before_define");
    assert_eq!(c.len(), 1, "{:#?}", r.conflicts);
    assert!(c[0].description.contains("at includes/defs line 1"), "{}", c[0].description);
}

#[test]
fn run_order_compares_only_known_places() {
    assert_eq!(RunAt::Main(1).before(RunAt::Main(2)), Some(true));
    assert_eq!(RunAt::Load(9).before(RunAt::Main(2)), Some(false));
    assert_eq!(RunAt::Main(9).before(RunAt::Call), Some(true));
    assert_eq!(RunAt::Call.before(RunAt::Call), None);
    assert_eq!(RunAt::Unknown.before(RunAt::Main(0)), None);
}

/// One mod defining a function twice is its own business; two mods inside functions have no
/// known order.
#[test]
fn redefinitions_by_one_mod_or_at_call_time() {
    let r = run(
        vec![
            after("gse", "gse:1", "-- top", "function isHostileTo() end"),
            after("gse", "gse:2", "-- bottom", "function isHostileTo() end"),
        ],
        &[("main.lua", SETTINGS_SRC)],
    );
    assert!(of_kind(&r, "redefined_global").is_empty(), "{:#?}", r.conflicts);

    let src = "function initA()\n  -- a\nend\nfunction initB()\n  -- b\nend\n";
    let r = run(
        vec![
            after("a", "a:1", "  -- a", "  function shared() end"),
            after("b", "b:1", "  -- b", "  function shared() end"),
        ],
        &[("main.lua", src)],
    );
    let d = of_kind(&r, "redefined_global");
    assert_eq!(d.len(), 1, "{:#?}", r.conflicts);
    assert!(d[0].description.contains("Which one runs last depends on when the game calls"), "{}", d[0].description);
}

const KEYS_SRC: &str = "\
function love.keypressed(key)
  if(key == \"f5\") then
    oldStuff()
  end
end
";

/// GConsole's F5 rewrite ends with `if(1 == 0) then`, which turns the game's F5 code off.
fn gconsole() -> Patch {
    pattern(
        "gconsole",
        "gconsole:f5",
        "main.lua",
        "  if(key == \"f5\") then",
        InsertPosition::At,
        "  if(key == \"f5\") then\n    newStuff()\n  end\n  if(1 == 0) then",
    )
}

#[test]
fn code_inside_a_constant_false_guard_never_runs() {
    let r = run(
        vec![gconsole(), after("gcore", "gcore:f5", "    oldStuff()", "    coreStuff()")],
        &[("main.lua", KEYS_SRC)],
    );
    let d = of_kind(&r, "dead_code");
    let warn: Vec<_> = d.iter().filter(|c| c.severity == "warning").collect();
    assert_eq!(warn.len(), 1, "{:#?}", r.conflicts);
    assert_eq!((warn[0].mod_a_id.as_str(), warn[0].mod_b_id.as_deref()), ("gcore", Some("gconsole")));
    assert!(warn[0].description.contains("`if (1 == 0) then` at line 5"), "{}", warn[0].description);
    assert!(warn[0].id.ends_with("@love.keypressed"), "{}", warn[0].id);
    // The guard over the game's own code is a note.
    let info: Vec<_> = d.iter().filter(|c| c.severity == "info").collect();
    assert_eq!(info.len(), 1, "{:#?}", r.conflicts);
    assert_eq!(info[0].mod_a_id, "gconsole");
    assert!(info[0].description.contains("puts 1 line of the game's code (lines 6 to 6)"), "{}", info[0].description);
}

#[test]
fn live_code_and_a_mods_own_guard_are_not_flagged() {
    let r = run(
        vec![
            gconsole(),
            after("gcore", "gcore:top", "function love.keypressed(key)", "  coreStuff()"),
            after("gconsole", "gconsole:own", "    oldStuff()", "    mine()"),
        ],
        &[("main.lua", KEYS_SRC)],
    );
    let warn: Vec<_> = of_kind(&r, "dead_code").into_iter().filter(|c| c.severity == "warning").collect();
    assert!(warn.is_empty(), "{:#?}", r.conflicts);
}

const RETURN_SRC: &str = "\
function love.keypressed(key)
  handle(key)
end
";

/// A `return` at function level in one patch hides another mod's code after it.
#[test]
fn code_after_another_patchs_return_never_runs() {
    let r = run(
        vec![
            after("gdumper", "gdumper:keys", "function love.keypressed(key)", "  do return end"),
            after("gcore", "gcore:keys", "  handle(key)", "  extra(key)"),
        ],
        &[("main.lua", RETURN_SRC)],
    );
    let e = of_kind(&r, "early_return_shadow");
    assert_eq!(e.len(), 1, "{:#?}", r.conflicts);
    assert_eq!((e[0].mod_a_id.as_str(), e[0].mod_b_id.as_deref()), ("gcore", Some("gdumper")));
    assert!(e[0].description.contains("returns from `love.keypressed` at main.lua line 2"), "{}", e[0].description);
}

/// GDumper's real hook returns only when it handled the key: the rest still runs.
#[test]
fn a_conditional_return_hides_nothing() {
    let r = run(
        vec![
            after("gdumper", "gdumper:keys", "function love.keypressed(key)", "  if gdumper and gdumper.onKeyPressed(key) then return end"),
            after("gcore", "gcore:keys", "  handle(key)", "  extra(key)"),
        ],
        &[("main.lua", RETURN_SRC)],
    );
    assert!(of_kind(&r, "early_return_shadow").is_empty(), "{:#?}", r.conflicts);
}

/// mod.json `overrides` silences a finding by check and name, from either mod.
#[test]
fn silences_by_check_and_name() {
    let patches = || vec![gconsole(), after("gcore", "gcore:f5", "    oldStuff()", "    coreStuff()")];
    let warnings = |silence: &[(&str, &str)]| {
        let (mut t, vanilla) = table(patches(), &[("main.lua", KEYS_SRC)]);
        t.mods = ["gconsole", "gcore"]
            .iter()
            .enumerate()
            .map(|(i, id)| ModMeta {
                id: id.to_string(),
                loader_index: i as u32,
                overrides: silence
                    .iter()
                    .filter(|(m, _)| m == id)
                    .map(|(_, s)| s.to_string())
                    .collect(),
                ..ModMeta::default()
            })
            .collect();
        let r = simulate_and_analyze_patches(&t, &vanilla);
        of_kind(&r, "dead_code").iter().filter(|c| c.severity == "warning").count()
    };
    assert_eq!(warnings(&[]), 1);
    assert_eq!(warnings(&[("gcore", "dead_code:love.keypressed")]), 0);
    assert_eq!(warnings(&[("gconsole", " dead_code:* ")]), 0);
    assert_eq!(warnings(&[("gcore", "dead_code:love.update")]), 1);
    assert_eq!(warnings(&[("gcore", "early_return_shadow:love.keypressed")]), 1);
}

/// The silence list comes from mod.json through the loader.
#[test]
fn mod_json_overrides_reach_the_checks() {
    let dir = tempfile::tempdir().unwrap();
    let mods = dir.path();
    std::fs::create_dir_all(mods.join("qm")).unwrap();
    let toml = |payload: &str, anchor: &str| {
        format!(
            "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"p\"\ntarget = \"main.lua\"\n\
             pattern = \"{anchor}\"\nposition = \"after\"\npayload = '''\n{payload}\n'''\n"
        )
    };
    for (id, anchor, payload, overrides) in [
        ("alpha", "-- top", "local _orig = loadModSettings", "[\"capture_before_define:loadModSettings\"]"),
        ("beta", "-- bottom", "function loadModSettings() end", "[]"),
    ] {
        let m = mods.join(id);
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("qm.toml"), toml(payload, anchor)).unwrap();
        std::fs::write(
            m.join("mod.json"),
            format!("{{\"id\": \"{id}\", \"name\": \"{id}\", \"version\": \"1.0.0\", \"overrides\": {overrides}}}"),
        )
        .unwrap();
    }
    let t = PatchTable::load(mods).unwrap();
    assert_eq!(
        t.mods.iter().find(|m| m.id == "alpha").map(|m| m.overrides.clone()),
        Some(vec!["capture_before_define:loadModSettings".to_string()])
    );
    let vanilla = HashMap::from([("main.lua".to_string(), SETTINGS_SRC.to_string())]);
    let r = simulate_and_analyze_patches(&t, &vanilla);
    assert!(of_kind(&r, "capture_before_define").is_empty(), "{:#?}", r.conflicts);
}

/// A semantic finding never refuses a fix (it is not a new conflict for the preview check).
#[test]
fn findings_do_not_count_as_new_conflicts() {
    let clean = run(vec![], &[("main.lua", KEYS_SRC)]);
    let flagged = run(
        vec![gconsole(), after("gcore", "gcore:f5", "    oldStuff()", "    coreStuff()")],
        &[("main.lua", KEYS_SRC)],
    );
    assert!(flagged.conflicts.iter().any(super::is_semantic));
    let check = crate::conflict::compare_resolution(&clean, &flagged);
    assert!(check.new_conflicts.is_empty(), "{:?}", check.new_conflicts);
    assert!(check.refusal().is_none());
}

// Task 37b checks.

/// Another mod's fields on a global table, then a literal that makes a new table: the fields
/// are gone.
#[test]
fn a_table_literal_after_another_mods_fields_is_flagged() {
    let r = run(
        vec![
            after("to", "to:fields", "-- top", "modSettings.squadLimit = 6\nfunction modSettings.load() end"),
            after("gcore", "gcore:literal", "-- bottom", "modSettings = { pathfinding = 1 }"),
        ],
        &[("main.lua", SETTINGS_SRC)],
    );
    let t = of_kind(&r, "table_literal_reset");
    assert_eq!(t.len(), 1, "{:#?}", r.conflicts);
    assert_eq!((t[0].mod_a_id.as_str(), t[0].mod_b_id.as_deref()), ("to", Some("gcore")));
    assert_eq!(t[0].id, "semantic:table_literal_reset:main.lua:to:fields:gcore:literal@modSettings");
    assert!(t[0].description.contains("sets `modSettings.squadLimit` at main.lua line 2"), "{}", t[0].description);

    // Another mod's filled literal before it counts too.
    let r = run(
        vec![
            after("to", "to:literal", "-- top", "modSettings = { squadLimit = 6 }"),
            after("gcore", "gcore:literal", "-- bottom", "modSettings = { pathfinding = 1 }"),
        ],
        &[("main.lua", SETTINGS_SRC)],
    );
    let t = of_kind(&r, "table_literal_reset");
    assert_eq!(t.len(), 1, "{:#?}", r.conflicts);
    assert!(t[0].description.contains("makes `modSettings` with fields"), "{}", t[0].description);
}

/// `X = X or {}` keeps the table; a literal before the fields, fields set in love.load, an
/// empty literal before and one mod's own fields are fine.
#[test]
fn a_table_kept_or_filled_later_is_fine() {
    let cases: [(&str, &str, &str, &str); 4] = [
        ("-- top", "modSettings.squadLimit = 6", "-- bottom", "modSettings = modSettings or {}"),
        ("-- bottom", "modSettings.squadLimit = 6", "-- top", "modSettings = { pathfinding = 1 }"),
        ("  -- load", "  modSettings.squadLimit = 6", "-- bottom", "modSettings = {}"),
        ("-- top", "modSettings = {}", "-- bottom", "modSettings = { pathfinding = 1 }"),
    ];
    for (a_anchor, a_payload, b_anchor, b_payload) in cases {
        let r = run(
            vec![after("to", "to:a", a_anchor, a_payload), after("gcore", "gcore:b", b_anchor, b_payload)],
            &[("main.lua", SETTINGS_SRC)],
        );
        assert!(of_kind(&r, "table_literal_reset").is_empty(), "{a_payload} / {b_payload}: {:#?}", r.conflicts);
    }
    let r = run(
        vec![
            after("gcore", "gcore:a", "-- top", "modSettings.squadLimit = 6"),
            after("gcore", "gcore:b", "-- bottom", "modSettings = { pathfinding = 1 }"),
        ],
        &[("main.lua", SETTINGS_SRC)],
    );
    assert!(of_kind(&r, "table_literal_reset").is_empty(), "{:#?}", r.conflicts);
}

const SAVE_MERGE: &str = "function saveA()
  local ok, loaded = pcall(love.filesystem.load, \"mod_settings.lua\")
  love.filesystem.write(\"mod_settings.lua\", ser(merged))
end";
const SAVE_PLAIN: &str = "function saveB()
  love.filesystem.write(\"mod_settings.lua\", ser(modSettings))
end";

/// Two mods write one file; the one that does not read it first drops the other's data.
#[test]
fn two_mods_writing_one_file_are_flagged() {
    let r = run(
        vec![after("to", "to:save", "-- top", SAVE_MERGE), after("gcore", "gcore:save", "-- bottom", SAVE_PLAIN)],
        &[("main.lua", SETTINGS_SRC)],
    );
    let w = of_kind(&r, "shared_file_writer");
    assert_eq!(w.len(), 1, "{:#?}", r.conflicts);
    assert_eq!(w[0].severity, "warning");
    assert_eq!((w[0].mod_a_id.as_str(), w[0].mod_b_id.as_deref()), ("to", Some("gcore")));
    assert!(w[0].id.ends_with("@mod_settings.lua"), "{}", w[0].id);
    assert!(
        w[0].description.contains("'gcore' writes the whole file without reading it first, so it drops what 'to' saved there."),
        "{}",
        w[0].description
    );

    // Both read first: a note.
    let r = run(
        vec![
            after("to", "to:save", "-- top", SAVE_MERGE),
            after("gcore", "gcore:save", "-- bottom", &SAVE_MERGE.replace("saveA", "saveB")),
        ],
        &[("main.lua", SETTINGS_SRC)],
    );
    let w = of_kind(&r, "shared_file_writer");
    assert_eq!(w.len(), 1, "{:#?}", r.conflicts);
    assert_eq!(w[0].severity, "info");
    assert!(w[0].description.contains("Both read the file first"), "{}", w[0].description);

    // Neither: a warning; a module counts as a writer, and the name's case does not matter.
    let m = module("gmenu", "includes/gmenu_core", "function save() love.filesystem.write('Mod_Settings.lua', s) end\n");
    let r = run(vec![after("gcore", "gcore:save", "-- bottom", SAVE_PLAIN), m], &[("main.lua", SETTINGS_SRC)]);
    let w = of_kind(&r, "shared_file_writer");
    assert_eq!(w.len(), 1, "{:#?}", r.conflicts);
    assert!(w[0].description.contains("Neither reads the file first"), "{}", w[0].description);
}

#[test]
fn one_writer_per_file_is_fine() {
    let r = run(
        vec![
            after("to", "to:save", "-- top", SAVE_PLAIN),
            after("to", "to:save2", "-- bottom", &SAVE_PLAIN.replace("saveB", "saveC")),
            after("gmenu", "gmenu:save", "  -- load", "  love.filesystem.write(\"saveModSettings.lua\", s)"),
        ],
        &[("main.lua", SETTINGS_SRC)],
    );
    assert!(of_kind(&r, "shared_file_writer").is_empty(), "{:#?}", r.conflicts);
}

const KEY_SRC: &str = "\
function love.keypressed(key)
  -- keys
end
function love.keyreleased(key)
  -- released
end
";

/// Two mods act on Ctrl+F9; the modifier may come from a local.
#[test]
fn two_mods_on_one_key_and_modifiers_are_flagged() {
    let r = run(
        vec![
            after(
                "gcore",
                "gcore:bench",
                "  -- keys",
                "  if key == \"f9\" and (love.keyboard.isDown(\"lctrl\") or love.keyboard.isDown(\"rctrl\")) then\n    bench()\n  end",
            ),
            after(
                "gconsole",
                "gconsole:f9",
                "function love.keypressed(key)",
                "  local ctrl = love.keyboard.isDown(\"lctrl\") or love.keyboard.isDown(\"rctrl\")\n  if(key == \"f9\" and ctrl) then\n    console()\n  end",
            ),
        ],
        &[("main.lua", KEY_SRC)],
    );
    let k = of_kind(&r, "key_collision");
    assert_eq!(k.len(), 1, "{:#?}", r.conflicts);
    assert!(k[0].id.ends_with("@ctrl+f9"), "{}", k[0].id);
    assert_eq!(k[0].severity, "warning");
    assert!(k[0].description.contains("acts on Ctrl+F9 at main.lua line 3"), "{}", k[0].description);
    assert!(k[0].description.contains("Both run when the player presses Ctrl+F9"), "{}", k[0].description);
}

/// Other modifiers, another key, another event, one mod twice, or the game's own check: no
/// finding.
#[test]
fn different_keys_modifiers_or_events_are_fine() {
    let ctrl = "  if key == \"f9\" and love.keyboard.isDown(\"lctrl\") then bench() end";
    let cases = [
        ("  if key == \"f9\" and love.keyboard.isDown(\"lshift\") then x() end", "  -- keys"),
        ("  if key == \"f9\" then x() end", "  -- keys"),
        ("  if key == \"f8\" and love.keyboard.isDown(\"rctrl\") then x() end", "  -- keys"),
        ("  if key == \"f9\" and love.keyboard.isDown(\"lctrl\") then x() end", "  -- released"),
    ];
    for (payload, anchor) in cases {
        let r = run(
            vec![after("gcore", "gcore:k", "  -- keys", ctrl), after("gconsole", "gconsole:k", anchor, payload)],
            &[("main.lua", KEY_SRC)],
        );
        assert!(of_kind(&r, "key_collision").is_empty(), "{payload}: {:#?}", r.conflicts);
    }
    let r = run(
        vec![after("gcore", "gcore:a", "  -- keys", ctrl), after("gcore", "gcore:b", "  -- keys", ctrl)],
        &[("main.lua", KEY_SRC)],
    );
    assert!(of_kind(&r, "key_collision").is_empty(), "{:#?}", r.conflicts);
    // A mod and the game's own F9: not this check.
    let src = KEY_SRC.replace("  -- keys", "  -- keys\n  if key == \"f9\" then vanilla() end");
    let r = run(vec![after("gcore", "gcore:k", "  -- keys", "  if key == \"f9\" then x() end")], &[("main.lua", &src)]);
    assert!(of_kind(&r, "key_collision").is_empty(), "{:#?}", r.conflicts);
}

const MIDDLE_SRC: &str = "-- top
-- middle
-- bottom
";

/// A wrapper that copied the function before another mod's wrapper was installed, and is
/// installed after it, skips that wrapper.
#[test]
fn a_wrapper_that_skips_another_wrapper_is_flagged() {
    let r = run(
        vec![
            after("to", "to:copy", "-- top", "local _orig = loadModSettings"),
            after("gcore", "gcore:settings", "-- middle", &WRAP.replace("_orig", "_prev")),
            after("to", "to:def", "-- bottom", "function loadModSettings()\n  if _orig then _orig() end\nend"),
        ],
        &[("main.lua", MIDDLE_SRC)],
    );
    let h = of_kind(&r, "double_hook");
    assert_eq!(h.len(), 1, "{:#?}", r.conflicts);
    assert_eq!((h[0].mod_a_id.as_str(), h[0].mod_b_id.as_deref()), ("gcore", Some("to")));
    assert_eq!(h[0].patch_b_id.as_deref(), Some("to:def"));
    assert!(
        h[0].description.contains("copies `loadModSettings` at main.lua line 2, before 'gcore''s wrapper is installed"),
        "{}",
        h[0].description
    );
    // The same pair is not also a capture_before_define or redefined_global finding.
    assert!(of_kind(&r, "capture_before_define").is_empty(), "{:#?}", r.conflicts);
    assert!(of_kind(&r, "redefined_global").is_empty(), "{:#?}", r.conflicts);
}

/// A later wrapper that never calls its copy drops the earlier wrapper; a plain definition it
/// replaces is a redefined_global.
#[test]
fn a_wrapper_that_never_calls_its_copy_is_flagged() {
    let no_call = "local _p = loadModSettings\nfunction loadModSettings()\n  read()\nend";
    let r = run(
        vec![after("to", "to:settings", "-- top", WRAP), after("gcore", "gcore:settings", "-- bottom", no_call)],
        &[("main.lua", SETTINGS_SRC)],
    );
    let h = of_kind(&r, "double_hook");
    assert_eq!(h.len(), 1, "{:#?}", r.conflicts);
    assert_eq!((h[0].mod_a_id.as_str(), h[0].mod_b_id.as_deref()), ("to", Some("gcore")));
    assert!(h[0].description.contains("never calls the copy it took"), "{}", h[0].description);
    assert!(of_kind(&r, "redefined_global").is_empty(), "{:#?}", r.conflicts);

    let r = run(
        vec![after("to", "to:settings", "-- top", PLAIN), after("gcore", "gcore:settings", "-- bottom", no_call)],
        &[("main.lua", SETTINGS_SRC)],
    );
    assert!(of_kind(&r, "double_hook").is_empty(), "{:#?}", r.conflicts);
    let d = of_kind(&r, "redefined_global");
    assert_eq!(d.len(), 1, "{:#?}", r.conflicts);
    assert_eq!((d[0].mod_a_id.as_str(), d[0].mod_b_id.as_deref()), ("to", Some("gcore")));
}

/// Wrappers that chain, in either order, and wrappers installed when a function is called (no
/// known order): no double_hook.
#[test]
fn chained_or_call_time_wrappers_are_fine() {
    let gcore = WRAP.replace("_orig", "_prev");
    for (first, second) in [("to", "gcore"), ("gcore", "to")] {
        let body = |m: &str| if m == "to" { WRAP.to_string() } else { gcore.clone() };
        let r = run(
            vec![
                after(first, &format!("{first}:settings"), "-- top", &body(first)),
                after(second, &format!("{second}:settings"), "-- bottom", &body(second)),
            ],
            &[("main.lua", SETTINGS_SRC)],
        );
        assert!(of_kind(&r, "double_hook").is_empty(), "{:#?}", r.conflicts);
    }
    let src = "function initA()\n  -- a\nend\nfunction initB()\n  -- b\nend\n";
    let wrap = |l: &str| format!("  local {l} = Button.click\n  Button.click = function() end");
    let r = run(
        vec![after("a", "a:1", "  -- a", &wrap("_a")), after("b", "b:1", "  -- b", &wrap("_b"))],
        &[("main.lua", src)],
    );
    assert!(of_kind(&r, "double_hook").is_empty(), "{:#?}", r.conflicts);
}

/// Silences name the file for shared_file_writer.
#[test]
fn silences_name_the_file() {
    let (mut t, vanilla) = table(
        vec![after("to", "to:save", "-- top", SAVE_MERGE), after("gcore", "gcore:save", "-- bottom", SAVE_PLAIN)],
        &[("main.lua", SETTINGS_SRC)],
    );
    t.mods = vec![ModMeta {
        id: "gcore".to_string(),
        overrides: vec!["shared_file_writer:mod_settings.lua".to_string()],
        ..ModMeta::default()
    }];
    let r = simulate_and_analyze_patches(&t, &vanilla);
    assert!(of_kind(&r, "shared_file_writer").is_empty(), "{:#?}", r.conflicts);
}

// Task 37c checks.

const HOOK_SRC: &str = "\
function calcDamage(a, b)
  return a + b
end
function attack(u, t)
  local d = calcDamage(u, t)
  if(u.faction ~= t.faction) then
    hit(t, d)
  end
end
-- extra
";

/// GSpecialEvents-like rewrite: the damage call and the faction test go through its own code.
fn reroute() -> Patch {
    pattern(
        "gse",
        "gse:hostility",
        "main.lua",
        "  local d = calcDamage(u, t)\n  if(u.faction ~= t.faction) then",
        InsertPosition::At,
        "  local d = gseDamage(u, t)\n  if isHostileTo(u, t) then",
    )
}

const THREAT: &str = "function threat(u, t)\n  if u.faction ~= t.faction then\n    return calcDamage(u, t)\n  end\nend";

/// A payload that calls the game function, or tests the condition, another mod's `at` patch
/// took out of the lines it rewrote.
#[test]
fn code_that_goes_around_another_mods_rewrite_is_flagged() {
    let r = run(vec![reroute(), after("to", "to:threat", "-- extra", THREAT)], &[("main.lua", HOOK_SRC)]);
    let b = of_kind(&r, "bypassed_hook");
    let ids: Vec<&str> = b.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "semantic:bypassed_hook:main.lua:to:threat:gse:hostility@calcDamage",
            "semantic:bypassed_hook:main.lua:to:threat:gse:hostility@u.faction ~= t.faction",
        ],
        "{:#?}",
        r.conflicts
    );
    assert!(b.iter().all(|c| c.severity == "warning" && c.mod_a_id == "to" && c.mod_b_id.as_deref() == Some("gse")));
    assert!(
        b[0].description.contains("rewrites lines of main.lua that call `calcDamage` (now main.lua line 5)"),
        "{}",
        b[0].description
    );
    assert!(b[0].description.contains("calls `calcDamage` itself at main.lua line 13"), "{}", b[0].description);
    assert!(b[1].description.contains("tests `u.faction ~= t.faction` itself"), "{}", b[1].description);
}

/// A rewrite that keeps the call, the same mod's code, and functions the game does not define:
/// no finding.
#[test]
fn code_that_follows_the_rewrite_is_fine() {
    let keeps = pattern(
        "gse",
        "gse:hostility",
        "main.lua",
        "  local d = calcDamage(u, t)\n  if(u.faction ~= t.faction) then",
        InsertPosition::At,
        "  local d = calcDamage(u, t) * 2\n  if(u.faction ~= t.faction) and alive(t) then",
    );
    let r = run(vec![keeps, after("to", "to:threat", "-- extra", THREAT)], &[("main.lua", HOOK_SRC)]);
    // The test changed (`and alive(t)`), so it is reported; the call is kept.
    let ids: Vec<&str> = of_kind(&r, "bypassed_hook").iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["semantic:bypassed_hook:main.lua:to:threat:gse:hostility@u.faction ~= t.faction"]);
    let r = run(vec![reroute(), after("gse", "gse:threat", "-- extra", THREAT)], &[("main.lua", HOOK_SRC)]);
    assert!(of_kind(&r, "bypassed_hook").is_empty(), "{:#?}", r.conflicts);
    // The game still calls `calcDamage` elsewhere (a utility, not rerouted): only the test.
    let src = HOOK_SRC.replace("-- extra", "function heal(u)\n  return calcDamage(u, u)\nend\n-- extra");
    let r = run(vec![reroute(), after("to", "to:threat", "-- extra", THREAT)], &[("main.lua", &src)]);
    let ids: Vec<&str> = of_kind(&r, "bypassed_hook").iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["semantic:bypassed_hook:main.lua:to:threat:gse:hostility@u.faction ~= t.faction"]);
    let builtin = pattern("gse", "gse:hit", "main.lua", "    hit(t, d)", InsertPosition::At, "    hitHard(t, d)");
    let r = run(vec![builtin, after("to", "to:hit", "-- extra", "hit(x, 1)")], &[("main.lua", HOOK_SRC)]);
    assert!(of_kind(&r, "bypassed_hook").is_empty(), "`hit` is not a game function here: {:#?}", r.conflicts);
}

const UNIT_SRC: &str = "\
function update(unit)
  unit.x = unit.x + 1
end
-- top
-- bottom
";

/// Two mods keep data in one new field of a unit: a note.
#[test]
fn two_mods_setting_one_new_unit_field_is_a_note() {
    let r = run(
        vec![
            after("gcore", "gcore:path", "-- top", "function plan(unit, wps)\n  unit.pathQueue = wps\nend"),
            module("to", "smart_move", "local function halt(dude)\n  dude.pathQueue = nil\nend\n"),
        ],
        &[("main.lua", UNIT_SRC)],
    );
    let f = of_kind(&r, "shared_field");
    assert_eq!(f.len(), 1, "{:#?}", r.conflicts);
    assert_eq!(f[0].severity, "info");
    assert_eq!(f[0].id, "semantic:shared_field:main.lua:gcore:path:to:module@pathQueue");
    assert!(f[0].description.contains("sets `unit.pathQueue` at main.lua line 6"), "{}", f[0].description);
    assert!(f[0].description.contains("sets `dude.pathQueue` at smart_move line 2"), "{}", f[0].description);
}

/// A field the game sets, a mod's own table, `self`, or one mod: no note.
#[test]
fn game_fields_and_own_tables_are_fine() {
    let cases = [
        ("function a(dude) dude.x = 1 end", "function b(u) u.x = 2 end"),
        ("local M = {}\nM.cache = 1", "local M = {}\nM.cache = 2"),
        ("function A:new() self.cache = 1 end", "function B:new() self.cache = 2 end"),
    ];
    for (a, b) in cases {
        let r = run(
            vec![after("gcore", "gcore:a", "-- top", a), after("to", "to:b", "-- bottom", b)],
            &[("main.lua", UNIT_SRC)],
        );
        assert!(of_kind(&r, "shared_field").is_empty(), "{a} / {b}: {:#?}", r.conflicts);
    }
    let r = run(
        vec![
            after("gcore", "gcore:a", "-- top", "function a(d) d.pathQueue = 1 end"),
            after("gcore", "gcore:b", "-- bottom", "function b(d) d.pathQueue = 2 end"),
        ],
        &[("main.lua", UNIT_SRC)],
    );
    assert!(of_kind(&r, "shared_field").is_empty(), "{:#?}", r.conflicts);
}

const MENU_SRC: &str = "\
function menu()
  if(menuPage == 1) then
    buttons[#buttons+1] = Button.new(725, 185, 200, 465, \"menu_options\", \"active\", true, over)
  end
  -- menu
end
";

/// Two mods add a button with one label, or use one page number the game does not use.
#[test]
fn two_mods_with_one_button_label_or_page_are_flagged() {
    let gmenu = "  if(menuPage == 85) then\n    buttons[#buttons+1] = Button.new(1, 2, 3, 4, \"menu_mods\", \"active\", true)\n  end";
    let other = "  if menuPage == 85 then\n    Button.new(5, 6, 7, 8, 'menu_mods', \"active\")\n  end";
    let r = run(
        vec![after("gmenu", "gmenu:page", "  -- menu", gmenu), after("other", "other:page", "  -- menu", other)],
        &[("main.lua", MENU_SRC)],
    );
    let n = of_kind(&r, "name_collision");
    let ids: Vec<&str> = n.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "semantic:name_collision:main.lua:gmenu:page:other:page@menuPage=85",
            "semantic:name_collision:main.lua:gmenu:page:other:page@menu_modsButton",
        ],
        "{:#?}",
        r.conflicts
    );
    assert!(n[0].description.contains("uses menu page 85 at main.lua line"), "{}", n[0].description);
    assert!(n[1].description.contains("calls the one global `menu_modsButton`"), "{}", n[1].description);
}

/// The game's own labels and pages, other labels and one mod: no finding.
#[test]
fn game_labels_and_pages_are_fine() {
    let a = "  if(menuPage == 1) then\n    Button.new(1, 2, 3, 4, \"menu_options\", \"active\")\n  end";
    let b = "  if(menuPage == 1) then\n    Button.new(1, 2, 3, 4, \"menu_options\", \"active\")\n    Button.new(1, 2, 3, 4, \"other_mods\", \"active\")\n  end";
    let r = run(
        vec![after("gmenu", "gmenu:page", "  -- menu", a), after("other", "other:page", "  -- menu", b)],
        &[("main.lua", MENU_SRC)],
    );
    assert!(of_kind(&r, "name_collision").is_empty(), "{:#?}", r.conflicts);
}

const BLOCK_SRC: &str = "\
function update()
  -- a
  work()
  -- b
end
";

/// Tactical Overhaul's #4 opens `if ... else` and #5 adds the `end`: when #5 does not apply,
/// the file is unbalanced.
#[test]
fn a_block_opened_by_one_patch_and_never_closed_is_flagged() {
    let open = || after("to", "to:4", "  -- a", "  if(#smartBatch > 1) then\n    batch()\n  else");
    let close = |anchor: &str| after("to", "to:5", anchor, "  end");
    let r = run(vec![open(), close("  -- missing")], &[("main.lua", BLOCK_SRC)]);
    let b = of_kind(&r, "block_balance");
    assert_eq!(b.len(), 1, "{:#?}", r.conflicts);
    assert_eq!(b[0].id, "semantic:block_balance:main.lua:to:4:to:5@main.lua");
    assert!(
        b[0].description.contains("opens a block in main.lua that patch 'to:5' of the same mod closes, but 'to:5' did not apply. The file is left with 1 block never closed"),
        "{}",
        b[0].description
    );
    // Turned off.
    let mut off = close("  -- b");
    if let Patch::Pattern(p) = &mut off {
        p.disabled = true;
    }
    let r = run(vec![open(), off], &[("main.lua", BLOCK_SRC)]);
    let b = of_kind(&r, "block_balance");
    assert_eq!(b.len(), 1, "{:#?}", r.conflicts);
    assert!(b[0].description.contains("but 'to:5' is turned off"), "{}", b[0].description);
    // A closer alone.
    let r = run(vec![close("  -- b")], &[("main.lua", BLOCK_SRC)]);
    let b = of_kind(&r, "block_balance");
    assert_eq!(b.len(), 1, "{:#?}", r.conflicts);
    assert!(b[0].description.contains("closes 1 block it does not open"), "{}", b[0].description);
}

/// Both halves applied, an `at` that swaps one `if` for another, balanced payloads: fine.
#[test]
fn balanced_patches_are_fine() {
    let r = run(
        vec![
            after("to", "to:4", "  -- a", "  if(#smartBatch > 1) then\n    batch()\n  else"),
            after("to", "to:5", "  -- b", "  end"),
            pattern("gcore", "gcore:w", "main.lua", "  work()", InsertPosition::At, "  if ready then work() end"),
            after("gconsole", "gconsole:f", "function update()", "  for i = 1, 2 do\n    tick(i)\n  end"),
        ],
        &[("main.lua", BLOCK_SRC)],
    );
    assert!(of_kind(&r, "block_balance").is_empty(), "{:#?}", r.conflicts);
    let swap = pattern("gconsole", "gconsole:if", "main.lua", "  if(key == \"f5\") then", InsertPosition::At, "  if(1 == 0) then");
    let r = run(vec![swap], &[("main.lua", KEYS_SRC)]);
    assert!(of_kind(&r, "block_balance").is_empty(), "{:#?}", r.conflicts);
}

/// Phase E D84: the save fields the mods write as `player.<field> = ...`.
#[test]
fn the_save_field_registry_lists_what_mods_write_on_player() {
    let main = "player = {}\nplayer.rank = 1\nfunction love.load()\n  setup()\nend\nfunction tick(dude)\n  step()\nend\n";
    let r = run(
        vec![
            after("gcore", "g1", "  setup()", "  player.option_a = 1\n  player.option_shared = 2\n  player.rank = 3"),
            after("to", "t1", "  step()", "  player.option_shared = player.option_shared or 5\n  player.option_t = {}\n  player.option_t.deep = 1\n  dude.option_local = 1\n  local p = player\n  p.viaLocal = 1"),
            after("quartermaster", "q1", "player = {}", "player.qmFaction = {}\nplayer.byQm = 1"),
            module("gcore", "includes/gmod", "local M = {}\nfunction M.init()\n  player.option_mod = true\n  player.option_fn = function() end\nend\nif false then\n  player.option_dead = 1\nend\nreturn M\n"),
        ],
        &[("main.lua", main)],
    );
    let reg = &r.save_fields;
    let fields: Vec<_> = reg.fields.keys().map(String::as_str).collect();
    assert_eq!(
        fields,
        ["option_a", "option_mod", "option_shared", "option_t"],
        "{reg:?}"
    );
    assert_eq!(reg.single_owner("option_a"), Some("gcore"));
    assert_eq!(reg.single_owner("option_mod"), Some("gcore"), "a module's write counts");
    assert_eq!(reg.owners("option_shared"), ["gcore", "to"]);
    assert_eq!(reg.single_owner("option_t"), Some("to"));
    // The game sets `rank`; a function value, a deeper write, a write through a local or a
    // parameter, dead code and QuarterMaster's own code are not listed.
    for absent in ["rank", "option_fn", "deep", "option_local", "viaLocal", "option_dead", "byQm", "qmFaction"] {
        assert!(reg.owners(absent).is_empty(), "{absent}");
    }
    assert_eq!(reg.by_mod()["to"], ["option_shared", "option_t"]);
}

#[test]
fn no_mods_give_an_empty_save_field_registry() {
    let r = run(Vec::new(), &[("main.lua", "player = {}\nplayer.rank = 1\n")]);
    assert!(r.save_fields.is_empty());
}
