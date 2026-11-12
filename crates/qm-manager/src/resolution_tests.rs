//! `check_resolution` and the legacy app's resolve (D63), on a staged profile in a temp folder.
use super::*;
use crate::conflict::simulate_and_analyze_patches;
use qm_core::patch::edits::ApplyAs;
use qm_core::patch::{InsertPosition, Patch, PatternPatch, Target};
use std::io::Write;
use std::path::PathBuf;

/// `t.console = false` -> `t.console = true`.
const CONSOLE: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
id = "base"
target = "conf.lua"
pattern = "t.console = false"
position = "at"
payload = "t.console = true"
"#;

/// Anchored on the line CONSOLE rewrites, so it breaks when it runs after it.
fn addon(payload: &str) -> String {
    format!(
        "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"late\"\ntarget = \"conf.lua\"\npattern = \"t.console = false\"\nposition = \"after\"\npayload = \"{payload}\"\n"
    )
}

struct World {
    _tmp: tempfile::TempDir,
    paths: QuarterMasterPaths,
    game_dir: PathBuf,
}

impl World {
    fn add_mod(&self, id: &str, prio: i32, qm_toml: &str) {
        let dir = self._tmp.path().join("src_mods").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let mod_json = serde_json::json!({ "id": id, "name": id, "version": "1.0.0", "load_priority": prio });
        std::fs::write(dir.join("mod.json"), mod_json.to_string()).unwrap();
        std::fs::write(dir.join("qm.toml"), qm_toml).unwrap();
        self.paths.import_mod_from_path(&dir, None, false).unwrap();
    }

    fn staged(&self) -> PathBuf {
        self.paths.profiles_dir.join("default").join("Mods")
    }

    /// profile.json and the staged load-order.json, as bytes.
    fn files(&self) -> (Vec<u8>, Vec<u8>) {
        let read = |p: PathBuf| std::fs::read(p).unwrap_or_default();
        (
            read(self.paths.profiles_dir.join("default").join("profile.json")),
            read(self.staged().join("qm").join("load-order.json")),
        )
    }

    /// The Tier 2 suggestion the staged profile's simulation offers.
    fn glue(&self) -> ConflictResolutionSuggestion {
        self.suggestion(|s| matches!(s, ConflictResolutionSuggestion::GenerateGluePatch { .. }))
    }

    /// The first suggestion the staged profile's simulation offers that `want` takes, with
    /// the simulation.
    fn suggestion(&self, want: impl Fn(&ConflictResolutionSuggestion) -> bool) -> ConflictResolutionSuggestion {
        let r = self.simulation();
        r.conflicts
            .iter()
            .flat_map(|c| &c.suggestions)
            .find(|s| want(s))
            .cloned()
            .unwrap_or_else(|| panic!("no such suggestion: {:?}", r.conflicts))
    }

    /// The staged profile's simulation.
    fn simulation(&self) -> SimulationReport {
        let exe = self.game_dir.join("mnb.exe");
        let (_, vanilla) = self.paths.load_vanilla_lua_files(Some(&exe)).unwrap();
        let table = PatchTable::load(&self.staged()).unwrap();
        simulate_and_analyze_patches(&table, &vanilla)
    }
}

/// A game with `conf.lua`, the mods `core` (priority 10) and `addon` (20, its patch inserts
/// `addon_payload`), both enabled in the staged default profile.
fn world(addon_payload: &str) -> World {
    world_of("t.console = false\n", &[("core", 10, CONSOLE), ("addon", 20, &addon(addon_payload))])
}

/// A game whose `conf.lua` is `conf`, with `mods` (id, priority, qm.toml) enabled in the staged
/// default profile.
fn world_of(conf: &str, mods: &[(&str, i32, &str)]) -> World {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let game_dir = root.join("game");
    let save_dir = root.join("save");
    let workspace = root.join("workspace");
    for d in [&game_dir, &save_dir, &workspace] {
        std::fs::create_dir_all(d).unwrap();
    }
    let exe = game_dir.join("mnb.exe");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&exe).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("main.lua", opts).unwrap();
    zip.write_all(b"gameVersion = \"0.5.3b\"\nversion = \"0.5.3b\"\n").unwrap();
    zip.start_file("conf.lua", opts).unwrap();
    zip.write_all(conf.as_bytes()).unwrap();
    zip.finish().unwrap();

    let paths = QuarterMasterPaths::resolve(Some(&root.join("data")));
    // Settings default to the real game install; overwrite them before anything reads them.
    let mut settings = paths.load_settings();
    settings.game_exe_path = Some(exe);
    settings.save_dir_path = Some(save_dir);
    settings.workspace_mods_dir = Some(workspace);
    settings.first_run_completed = true;
    paths.save_settings(&settings).unwrap();

    let w = World { _tmp: tmp, paths, game_dir };
    for (id, prio, toml) in mods {
        w.add_mod(id, *prio, toml);
    }
    let mut def = w.paths.get_profile("default").unwrap();
    def.enabled_mods = mods.iter().map(|(id, _, _)| id.to_string()).collect();
    w.paths.save_profile(&def).unwrap();
    w.paths.activate_profile("default").unwrap();
    w
}

/// Copies the game's `lua51.dll` into the world's game folder; false when it is not on this
/// machine (the compile check cannot run then).
fn with_game_lua(w: &World) -> bool {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let found = [root.join("../../../game/lua51.dll"), root.join("../../game/lua51.dll")]
        .into_iter()
        .find(|p| p.is_file());
    match found {
        Some(p) => std::fs::copy(p, w.game_dir.join("lua51.dll")).is_ok(),
        None => false,
    }
}

/// D63: the legacy app's path refuses a copy that stops a file from compiling, and writes
/// nothing.
#[test]
fn a_glue_that_breaks_a_file_is_refused_with_nothing_written() {
    let w = world("if t.console then");
    if !with_game_lua(&w) {
        eprintln!("lua51.dll not found; skipped");
        return;
    }
    let sug = w.glue();
    let p = w.paths.get_profile("default").unwrap();
    let check = check_resolution(&w.paths, &p, &sug, 0).unwrap();
    assert!(check.breaks_file, "{check:?}");
    assert_eq!(check.impact.compile_errors_new, ["conf.lua"]);
    let why = check.refusal().expect("a refusal");
    assert_eq!(why, "The fix was not applied: it would stop conf.lua from compiling.");

    let before = w.files();
    assert_eq!(resolve_conflict(&w.paths, &sug, 0).unwrap_err(), why);
    assert!(before == w.files(), "profile.json or the staged plan changed");
    assert!(w.paths.get_profile("default").unwrap().glue_patches.is_empty());
}

/// A copy that resolves the conflict and breaks nothing is applied and staged.
#[test]
fn a_clean_glue_is_checked_then_applied() {
    let w = world("-- addon");
    let sug = w.glue();
    let ConflictResolutionSuggestion::GenerateGluePatch { glue_patch, .. } = &sug else {
        unreachable!()
    };
    let p = w.paths.get_profile("default").unwrap();
    let check = check_resolution(&w.paths, &p, &sug, 0).unwrap();
    assert!(!check.breaks_file && check.new_conflicts.is_empty(), "{check:?}");
    assert_eq!(check.refusal(), None);
    assert_eq!(check.impact.files_changed, ["conf.lua"]);
    assert_eq!(check.impact.resolved.len(), 1, "{check:?}");
    assert!(check.impact.resolved[0].starts_with("broken-anchor:conf.lua:"), "{check:?}");

    let got = resolve_conflict(&w.paths, &sug, 0).unwrap();
    assert_eq!(got, check);
    let saved = w.paths.get_profile("default").unwrap();
    assert!(saved.glue_patches.iter().any(|g| g.id == glue_patch.id), "{:?}", saved.glue_patches);
    let plan = String::from_utf8(w.files().1).unwrap();
    assert!(plan.contains(&glue_patch.id), "{plan}");
}

/// "Run it as separate edits" on a profile whose fixes are not imported: refused, nothing
/// changes.
#[test]
fn apply_as_needs_imported_fixes() {
    let sug = ConflictResolutionSuggestion::ApplyAs {
        patch_id: "a:p".into(),
        mod_id: "a".into(),
        explanation: String::new(),
    };
    let mut p = crate::profiles::builtin_vanilla_profile(None);
    p.fixes_version = 0;
    let before = serde_json::to_string(&p).unwrap();
    assert_eq!(apply_suggestion(&mut p, &sug, 0, None).unwrap_err(), NOT_IMPORTED);
    assert_eq!(serde_json::to_string(&p).unwrap(), before);
}

/// `core` adds a line inside the two lines `addon`'s `at` rewrites (with one new line
/// between them), so the `at` loses its place: the Halt split of §10.1 in small.
const SPLIT_CONF: &str = "t.a = 1\nt.b = 2\nt.c = 3\n";
const SPLITTER: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
id = "core:split"
target = "conf.lua"
pattern = "t.a = 1"
position = "after"
payload = "t.x = 0"
"#;
const SPLIT_AT: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
id = "addon:at"
name = "Halt lines"
target = "conf.lua"
pattern = '''
t.a = 1
t.b = 2
'''
position = "at"
payload = '''
t.a = 1
t.y = 0
t.b = 2
'''
"#;

/// D46: the legacy app's path makes the edits-mode offer as an ApplyAs fix after the check,
/// and the staged plan runs the patch as separate edits.
#[test]
fn apply_as_is_checked_then_made_as_a_fix() {
    let w = world_of(SPLIT_CONF, &[("core", 10, SPLITTER), ("addon", 20, SPLIT_AT)]);
    let sug = w.suggestion(|s| matches!(s, ConflictResolutionSuggestion::ApplyAs { .. }));
    let ConflictResolutionSuggestion::ApplyAs { patch_id, .. } = &sug else {
        unreachable!()
    };
    assert_eq!(patch_id, "addon:at");
    let p = w.paths.get_profile("default").unwrap();
    let check = check_resolution(&w.paths, &p, &sug, 0).unwrap();
    assert_eq!(check.refusal(), None, "{check:?}");
    assert_eq!(check.impact.files_changed, ["conf.lua"]);
    assert_eq!(check.impact.resolved.len(), 1, "{check:?}");
    // The check writes nothing.
    assert!(crate::fixes::fixes(&w.paths.get_profile("default").unwrap()).all(|f| !matches!(f.change, crate::fixes::FixChange::ApplyAs { .. })));

    let got = resolve_conflict(&w.paths, &sug, 0).unwrap();
    assert_eq!(got, check);
    let saved = w.paths.get_profile("default").unwrap();
    let fix = crate::fixes::fixes(&saved)
        .find(|f| matches!(&f.change, crate::fixes::FixChange::ApplyAs { patch, mode: ApplyAs::Edits } if patch.id == "addon:at"))
        .unwrap_or_else(|| panic!("no ApplyAs fix: {:?}", saved.fixes));
    assert_eq!(fix.label, "Run Halt lines as separate edits");
    let plan = String::from_utf8(w.files().1).unwrap();
    assert!(plan.contains("\"apply_as\""), "{plan}");
    // The staged profile now applies both patches.
    let r = w.simulation();
    assert!(r.conflicts.iter().all(|c| c.kind != "broken_anchor_order"), "{:?}", r.conflicts);
}

/// D49: keeping one patch's change to a line skips the other patch's change there, as a
/// SkipHunk fix keyed on the profile's mods.
#[test]
fn skip_hunk_is_made_as_a_fix() {
    let a = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"core:b\"\ntarget = \"conf.lua\"\npattern = '''\nt.a = 1\nt.b = 2\n'''\nposition = \"at\"\npayload = '''\nt.a = 1\nt.b = 5\n'''\n";
    let b = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"addon:b\"\ntarget = \"conf.lua\"\npattern = '''\nt.b = 2\nt.c = 3\n'''\nposition = \"at\"\npayload = '''\nt.b = 7\nt.c = 3\n'''\n";
    let w = world_of(SPLIT_CONF, &[("core", 10, a), ("addon", 20, b)]);
    let sug = w.suggestion(|s| {
        matches!(s, ConflictResolutionSuggestion::SkipHunk { patch_id, .. } if patch_id == "addon:b")
    });
    let mut p = w.paths.get_profile("default").unwrap();
    // The edit is keyed on the mods: without them it cannot be made.
    assert!(apply_suggestion(&mut p.clone(), &sug, 0, None).is_err());
    let manifests: Vec<ModManifest> = w.paths.list_installed_mods().into_iter().map(|m| m.manifest).collect();
    let raw = raw_for(&p, &manifests, &w.staged()).unwrap();
    apply_suggestion(&mut p, &sug, 0, Some(&raw)).unwrap();
    let fix = crate::fixes::fixes(&p)
        .find_map(|f| match &f.change {
            crate::fixes::FixChange::SkipHunk { patch, hunk } => Some((patch.id.clone(), hunk.keep.clone(), hunk.vanilla.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no SkipHunk fix: {:?}", p.fixes));
    assert_eq!(fix, ("addon:b".to_string(), Some("core:b".to_string()), "t.b = 2".to_string()));
}

fn at(mod_id: &str, id: &str, anchor: &str, payload: &str) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: anchor.to_string(),
        position: InsertPosition::At,
        target: Target::Single("conf.lua".to_string()),
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
        on_failure: qm_core::patch::OnFailurePolicy::Warn,
        anchors: Vec::new(),
        sites: Vec::new(),
        stack: None,
        apply_as: None,
        mod_id: Some(mod_id.to_string()),
        disabled: false,
        qm: Default::default(),
    })
}

/// A change that adds a conflict is refused, and the refusal names it.
#[test]
fn a_new_conflict_is_a_refusal() {
    let vanilla = HashMap::from([("conf.lua".to_string(), "t.console = false\n".to_string())]);
    let current = simulate_and_analyze_patches(&PatchTable::default(), &vanilla);
    let mut table = PatchTable::default();
    table.targets.insert("conf.lua".to_string());
    table.patches.push((at("a", "a:1", "t.console = false", "t.console = true"), 10, PathBuf::from("A/qm.toml")));
    table.patches.push((at("b", "b:1", "t.console = true", "t.console = 1"), 20, PathBuf::from("B/qm.toml")));
    let next = simulate_and_analyze_patches(&table, &vanilla);
    let check = compare_resolution(&current, &next);
    assert!(!check.breaks_file);
    assert_eq!(check.new_conflicts, ["overlap:conf.lua:a:1:b:1"]);
    assert_eq!(check.impact.files_changed, ["conf.lua"]);
    assert_eq!(
        check.refusal().as_deref(),
        Some("The fix was not applied: it would add a new conflict (overlap:conf.lua:a:1:b:1).")
    );
    // The other way round removes it.
    let back = compare_resolution(&next, &current);
    assert_eq!(back.impact.resolved, ["overlap:conf.lua:a:1:b:1"]);
    assert_eq!(back.refusal(), None);
}
