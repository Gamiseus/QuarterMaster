//! Renaming a profile's zip stem names to the zip's mod.json id (D62, Phase D Task 9b).
use super::*;
use crate::fixes::{
    compile_plan, fixes, ledger_fields, migrate_checked, FixChange, FixEventKind, FIXES_VERSION,
};
use crate::profiles::builtin_vanilla_profile;
use qm_core::handshake::{LaunchConfig, LoadOrderPlan};
use qm_core::patch::order::OrderRule;
use qm_core::patch::plan::load_raw;
use qm_core::patch::table::PatchTable;
use qm_core::patch::Priority;
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// Two pattern patches with no id: the loader's fallback ids `<mod>:main_lua:1` and `:2`.
const ZIP_TOML: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
target = "main.lua"
pattern = "local x = 1"
position = "after"
payload = "-- zip 1"

[[patches]]
[patches.pattern]
target = "main.lua"
pattern = "local x = 1"
position = "after"
payload = "-- zip 2"
"#;

const OTHER_TOML: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
id = "other:a"
target = "main.lua"
pattern = "local x = 1"
position = "after"
payload = "-- other"
"#;

const MOD_A: &str = r#"{"id": "mod_a", "name": "Mod A", "version": "1.2.0"}"#;

fn make_zip(mods: &Path, name: &str, files: &[(&str, &str)]) {
    let mut zip = ZipWriter::new(fs::File::create(mods.join(name)).unwrap());
    for (name, content) in files {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

/// `PackA.zip` (mod.json id `mod_a`, so its stem names are `packa:...`) and the folder mod
/// `other`.
fn mods() -> TempDir {
    let t = TempDir::new().unwrap();
    fs::create_dir_all(t.path().join("qm")).unwrap();
    make_zip(
        t.path(),
        "PackA.zip",
        &[("ModA/mod.json", MOD_A), ("ModA/qm.toml", ZIP_TOML)],
    );
    let other = t.path().join("other");
    fs::create_dir_all(&other).unwrap();
    fs::write(
        other.join("mod.json"),
        r#"{"id": "other", "name": "Other", "version": "2.0.0"}"#,
    )
    .unwrap();
    fs::write(other.join("qm.toml"), OTHER_TOML).unwrap();
    t
}

/// An unmigrated profile with `f`'s fields.
fn old_profile(f: impl FnOnce(&mut ModProfile)) -> ModProfile {
    let mut p = builtin_vanilla_profile(None);
    p.id = "p".into();
    p.fixes_version = 0;
    f(&mut p);
    p
}

/// A profile written while the zip's id was its stem.
fn stem_profile() -> ModProfile {
    old_profile(|p| {
        p.enabled_mods = vec!["packa".into(), "other".into()];
        p.mod_priorities = HashMap::from([("packa".into(), 7)]);
        p.user_load_after = HashMap::from([("other".into(), vec!["packa".into()])]);
        p.disabled_patches = vec!["packa:main_lua:1".into()];
        p.patch_priorities = HashMap::from([("packa:packa:main_lua:2".into(), 77)]);
        p.patch_order = vec![OrderRule {
            patch: "packa:*".into(),
            before: vec!["other:a".into()],
            after: Vec::new(),
        }];
    })
}

type Outcome = (Vec<(Option<String>, bool, Priority)>, Option<String>);

/// What the game gets from `p`: each patch (id, disabled, priority) and main.lua's text.
fn outcome(dir: &Path, p: &ModProfile) -> Outcome {
    let prios = p.mod_priorities.clone();
    let plan = compile_plan(p, p.enabled_mods.clone(), prios);
    let read = LoadOrderPlan::parse(&serde_json::to_string(&plan).unwrap());
    assert!(read.error.is_none(), "{:?}", read.error);
    let raw = load_raw(dir, read.plan.as_ref()).unwrap();
    let applied = plan::apply_plan(&raw, &read);
    let patches = applied
        .patches
        .iter()
        .map(|(x, prio, _)| (x.id().map(str::to_string), x.is_disabled(), *prio))
        .collect();
    let table = PatchTable::from_applied(applied, raw.mod_dir.clone());
    let prepared = table.prepare();
    let text = qm_core::prepare::simulate_buffer(
        &table,
        &prepared,
        &LaunchConfig::default(),
        "main.lua",
        "local x = 1\n",
    )
    .ok()
    .map(|(_, a)| a.text);
    (patches, text)
}

fn packa() -> Vec<ZipRename> {
    vec![ZipRename {
        stem: "packa".into(),
        id: "mod_a".into(),
    }]
}

#[test]
fn stem_references_move_to_the_mod_json_id_with_the_same_output() {
    let t = mods();
    let raw = load_raw(t.path(), None).unwrap();
    assert_eq!(zip_stems(&raw), packa());
    let old = stem_profile();
    let before = outcome(t.path(), &old);
    // The disabled patch is off and the stem priorities apply.
    assert!(before
        .0
        .contains(&(Some("mod_a:main_lua:1".into()), true, 7)));
    assert!(before
        .0
        .contains(&(Some("mod_a:main_lua:2".into()), false, 77)));

    let mut p = old.clone();
    assert_eq!(rename_zip_stems(&mut p, &raw), packa());
    assert_eq!(p.enabled_mods, ["mod_a", "other"]);
    assert_eq!(p.mod_priorities, HashMap::from([("mod_a".into(), 7)]));
    assert_eq!(
        p.user_load_after,
        HashMap::from([("other".into(), vec!["mod_a".to_string()])])
    );
    assert_eq!(p.disabled_patches, ["mod_a:main_lua:1"]);
    assert_eq!(
        p.patch_priorities,
        HashMap::from([("mod_a:mod_a:main_lua:2".into(), 77)])
    );
    assert_eq!(p.patch_order[0].patch, "mod_a:*");
    assert_eq!(p.patch_order[0].before, ["other:a"]);

    // A disabled patch stays disabled across the rename, and the game's text is the same.
    assert_eq!(outcome(t.path(), &p), before);

    // Nothing is left to rename.
    let again = p.clone();
    assert!(rename_zip_stems(&mut p, &raw).is_empty());
    assert_eq!(
        serde_json::to_value(&p).unwrap(),
        serde_json::to_value(&again).unwrap()
    );
}

#[test]
fn a_migrated_profile_renames_its_fixes_and_their_history_lists_it() {
    let t = mods();
    let raw = load_raw(t.path(), None).unwrap();
    let mut p = migrate_checked(&stem_profile(), &raw, None)
        .unwrap()
        .profile;
    assert_eq!(p.fixes_version, FIXES_VERSION);
    let before = outcome(t.path(), &p);

    assert_eq!(rename_zip_stems(&mut p, &raw), packa());
    let disable = fixes(&p)
        .find(|f| matches!(f.change, FixChange::Disable { .. }))
        .unwrap();
    let FixChange::Disable { patch } = &disable.change else {
        unreachable!()
    };
    assert_eq!(patch.id, "mod_a:main_lua:1");
    assert_eq!(patch.mod_id, "mod_a");
    assert!(disable.snapshots.contains_key("mod_a:main_lua:1"));
    assert!(!disable.snapshots.contains_key("packa:main_lua:1"));
    let last = disable.provenance.history.last().unwrap();
    assert_eq!(last.what, FixEventKind::Rebound);
    assert_eq!(last.detail, "packa is now mod_a");
    // One event for the one renamed mod.
    let rebound = disable
        .provenance
        .history
        .iter()
        .filter(|e| e.what == FixEventKind::Rebound)
        .count();
    assert_eq!(rebound, 1);

    let order = fixes(&p)
        .find(|f| matches!(f.change, FixChange::ModOrder { .. }))
        .unwrap();
    assert!(matches!(
        &order.change,
        FixChange::ModOrder { move_mod_id, before_mod_id, .. }
            if move_mod_id == "mod_a" && before_mod_id == "other"
    ));
    assert_eq!(
        order.provenance.history.last().map(|e| e.what),
        Some(FixEventKind::Rebound)
    );

    // The legacy fields are the ledger's compiled view, as before.
    let compiled = ledger_fields(&p);
    assert_eq!(p.disabled_patches, compiled.disabled_patches);
    assert_eq!(p.patch_priorities, compiled.patch_priorities);
    assert_eq!(p.patch_order, compiled.patch_order);
    assert_eq!(p.disabled_patches, ["mod_a:main_lua:1"]);
    let plan = compile_plan(&p, p.enabled_mods.clone(), HashMap::new());
    assert!(plan.refs.contains_key("mod_a:main_lua:1"));
    assert_eq!(outcome(t.path(), &p), before);
}

#[test]
fn a_profile_without_stem_names_is_unchanged() {
    let t = mods();
    let raw = load_raw(t.path(), None).unwrap();
    let old = old_profile(|p| {
        p.enabled_mods = vec!["mod_a".into(), "other".into()];
        p.disabled_patches = vec!["mod_a:main_lua:1".into(), "other:a".into()];
        p.patch_priorities = HashMap::from([("mod_a:main_lua:2".into(), 3)]);
    });
    for mut p in [
        old.clone(),
        migrate_checked(&old, &raw, None).unwrap().profile,
    ] {
        let was = serde_json::to_value(&p).unwrap();
        assert!(rename_zip_stems(&mut p, &raw).is_empty());
        assert_eq!(serde_json::to_value(&p).unwrap(), was);
    }
}

#[test]
fn a_stem_that_names_a_folder_mod_is_not_renamed() {
    let t = mods();
    let folder = t.path().join("PackA");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("qm.toml"),
        OTHER_TOML.replace("other:a", "dir:a"),
    )
    .unwrap();
    let raw = load_raw(t.path(), None).unwrap();
    assert!(zip_stems(&raw).is_empty());
    let mut p = stem_profile();
    let was = serde_json::to_value(&p).unwrap();
    assert!(rename_zip_stems(&mut p, &raw).is_empty());
    assert_eq!(serde_json::to_value(&p).unwrap(), was);
}

#[test]
fn a_patch_priority_another_entry_beats_keeps_its_stem_name() {
    let t = mods();
    let raw = load_raw(t.path(), None).unwrap();
    let old = old_profile(|p| {
        p.enabled_mods = vec!["packa".into()];
        p.patch_priorities = HashMap::from([
            ("packa:main_lua:2".into(), 5),
            ("mod_a:mod_a:main_lua:2".into(), 9),
        ]);
    });
    let before = outcome(t.path(), &old);
    let mut p = old.clone();
    assert_eq!(rename_zip_stems(&mut p, &raw), packa());
    assert_eq!(p.enabled_mods, ["mod_a"]);
    assert_eq!(p.patch_priorities, old.patch_priorities);
    assert_eq!(outcome(t.path(), &p), before);
}

#[test]
fn an_asset_zip_patch_name_moves_to_its_new_id() {
    let t = TempDir::new().unwrap();
    fs::create_dir_all(t.path().join("qm")).unwrap();
    make_zip(
        t.path(),
        "Sounds.zip",
        &[
            (
                "SoundMod/mod.json",
                r#"{"id": "sound_mod", "name": "S", "version": "1.0.0"}"#,
            ),
            ("SoundMod/assets/sounds/m60_1.wav", "RIFF"),
        ],
    );
    let raw = load_raw(t.path(), None).unwrap();
    let mut p = old_profile(|p| {
        p.enabled_mods = vec!["sounds".into()];
        p.disabled_patches = vec!["sounds_auto_assets".into()];
    });
    let renamed = rename_zip_stems(&mut p, &raw);
    assert_eq!(renamed.len(), 1);
    assert_eq!(p.enabled_mods, ["sound_mod"]);
    assert_eq!(p.disabled_patches, ["sound_mod_auto_assets"]);
}
