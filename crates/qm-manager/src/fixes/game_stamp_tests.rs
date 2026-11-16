//! Tests for the game-update stamps (D74).
use super::super::*;
use super::*;
use crate::profiles::builtin_vanilla_profile;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const GCORE: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"gcore:main_lua:16\"\ntarget = \"main.lua\"\npattern = \"love.audio.play(snd_halt)\"\nposition = \"after\"\npayload = \"-- gcore halt\"\n";
const OTHER: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"other:a\"\ntarget = \"conf.lua\"\npattern = \"t.console = false\"\nposition = \"after\"\npayload = \"-- other\"\n";

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn mods() -> (TempDir, RawLoad) {
    let t = TempDir::new().unwrap();
    write(
        t.path(),
        "gcore/mod.json",
        r#"{"id": "gcore", "name": "GCore", "version": "1.0.0"}"#,
    );
    write(t.path(), "gcore/qm.toml", GCORE);
    write(
        t.path(),
        "other/mod.json",
        r#"{"id": "other", "name": "Other", "version": "2.0.0"}"#,
    );
    write(t.path(), "other/qm.toml", OTHER);
    let raw = qm_core::patch::plan::load_raw(t.path(), None).unwrap();
    (t, raw)
}

/// A migrated profile with one fix from before (the migrated disable of GCore's patch).
fn migrated(raw: &RawLoad) -> ModProfile {
    let mut p = builtin_vanilla_profile(None);
    p.id = "p".into();
    p.enabled_mods = vec!["gcore".into(), "other".into()];
    p.fixes_version = 0;
    p.disabled_patches = vec!["gcore:main_lua:16".into()];
    migrate(&p, raw, None).unwrap()
}

fn game() -> Vec<String> {
    vec![
        "main.lua".to_string(),
        "conf.lua".to_string(),
        "lib/main.lua".to_string(),
    ]
}

fn hashes(v: &[(&str, &str)]) -> HashMap<String, String> {
    v.iter()
        .map(|(k, h)| (k.to_string(), h.to_string()))
        .collect()
}

fn by_id<'a>(p: &'a ModProfile, id: &str) -> &'a ProfileFix {
    fixes(p).find(|f| f.id == id).unwrap()
}

#[test]
fn sha256_hex_is_the_full_lowercase_hash() {
    assert_eq!(
        sha256_hex(""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn a_new_fix_is_stamped_with_the_files_its_patch_targets() {
    let (_t, raw) = mods();
    let mut p = migrated(&raw);
    let old = fixes(&p).next().unwrap().id.clone();
    assert!(!by_id(&p, &old).fresh, "a migrated fix is not stamped");
    let aliases = HashSet::from(["other:a".to_string()]);
    let prov = Provenance::new(FixSource::PatchToggle);
    op_disable(&mut p, Some(&raw), "other:a", &aliases, prov);
    let new = fixes(&p).find(|f| f.id != old).unwrap().id.clone();
    assert!(by_id(&p, &new).fresh);

    let now = hashes(&[
        ("main.lua", "m1"),
        ("conf.lua", "c1"),
        ("lib/main.lua", "l1"),
    ]);
    let hash = |f: &str| now.get(f).cloned();
    let files = game();
    assert_eq!(stamp_fresh(&mut p, Some(&raw), Some((&files, &hash))), 1);
    let f = by_id(&p, &new);
    assert!(!f.fresh);
    assert_eq!(
        f.vanilla_hashes,
        BTreeMap::from([("conf.lua".to_string(), "c1".to_string())])
    );
    // The fix from before stamps nothing until it is accepted.
    assert!(by_id(&p, &old).vanilla_hashes.is_empty());
    assert!(game_drift(by_id(&p, &old), &|_| None).is_empty());
}

#[test]
fn a_target_names_every_game_file_whose_path_ends_with_it() {
    let (_t, raw) = mods();
    let p = migrated(&raw);
    let f = fixes(&p).next().unwrap();
    let files: Vec<String> = game_files(f, Some(&raw), &game()).into_iter().collect();
    assert_eq!(files, ["lib/main.lua", "main.lua"]);
    // A load order fix names mods, not patches.
    let mut order = f.clone();
    order.change = FixChange::ModOrder {
        move_mod_id: "gcore".into(),
        before_mod_id: "other".into(),
        priority: Some(10),
    };
    assert!(game_files(&order, Some(&raw), &game()).is_empty());
}

#[test]
fn a_changed_or_missing_file_is_drift_and_accept_stamps_again() {
    let (_t, raw) = mods();
    let mut p = migrated(&raw);
    let id = fixes(&p).next().unwrap().id.clone();
    assert!(accept(&mut p, &raw, &id));
    assert!(by_id(&p, &id).fresh);
    let files = game();
    let then = hashes(&[("main.lua", "m1"), ("lib/main.lua", "l1")]);
    stamp_fresh(
        &mut p,
        Some(&raw),
        Some((&files, &|f: &str| then.get(f).cloned())),
    );
    assert_eq!(by_id(&p, &id).vanilla_hashes.len(), 2);
    assert!(game_drift(by_id(&p, &id), &|f| then.get(f).cloned()).is_empty());

    // The game changed main.lua and dropped lib/main.lua.
    let now = hashes(&[("main.lua", "m2")]);
    let drift = game_drift(by_id(&p, &id), &|f| now.get(f).cloned());
    assert_eq!(
        drift,
        [
            GameFileDrift {
                file: "lib/main.lua".into(),
                before: "l1".into(),
                after: None,
            },
            GameFileDrift {
                file: "main.lua".into(),
                before: "m1".into(),
                after: Some("m2".into()),
            },
        ]
    );

    // Accepting it stamps it against the game as it is now.
    assert!(accept(&mut p, &raw, &id));
    let files = vec!["main.lua".to_string()];
    stamp_fresh(
        &mut p,
        Some(&raw),
        Some((&files, &|f: &str| now.get(f).cloned())),
    );
    assert!(game_drift(by_id(&p, &id), &|f| now.get(f).cloned()).is_empty());
}

#[test]
fn without_game_files_the_fix_waits_for_its_stamp() {
    let (_t, raw) = mods();
    let mut p = migrated(&raw);
    let id = fixes(&p).next().unwrap().id.clone();
    find_mut(&mut p, &id).unwrap().vanilla_hashes =
        BTreeMap::from([("main.lua".to_string(), "m1".to_string())]);
    assert!(accept(&mut p, &raw, &id));
    assert_eq!(stamp_fresh(&mut p, Some(&raw), None), 0);
    let f = by_id(&p, &id);
    assert!(!f.fresh);
    assert!(f.stamp_pending && has_pending(&p));
    // The old stamp is kept but not compared: the accept said the game is fine as it is.
    assert_eq!(
        f.vanilla_hashes.get("main.lua").map(String::as_str),
        Some("m1")
    );
    assert!(game_drift(f, &|_| Some("m2".to_string())).is_empty());
    // The mark is stored, so a later save stamps it.
    let json = serde_json::to_value(f).unwrap();
    assert_eq!(json["stamp_pending"], true);
    let back: ProfileFix = serde_json::from_value(json).unwrap();
    assert!(back.stamp_pending);

    // Without the mod load the fix's patches are not known either.
    let files = game();
    let now = hashes(&[("main.lua", "m2"), ("lib/main.lua", "l2")]);
    let hash = |f: &str| now.get(f).cloned();
    assert_eq!(stamp_fresh(&mut p, None, Some((&files, &hash))), 0);
    assert!(by_id(&p, &id).stamp_pending);

    // Once both can be read, the next save stamps it against the game as it is then.
    assert_eq!(stamp_fresh(&mut p, Some(&raw), Some((&files, &hash))), 1);
    let f = by_id(&p, &id);
    assert!(!f.stamp_pending && !has_pending(&p));
    assert_eq!(
        f.vanilla_hashes.get("main.lua").map(String::as_str),
        Some("m2")
    );
    assert!(game_drift(f, &hash).is_empty());
    let json = serde_json::to_value(f).unwrap();
    assert!(json.get("stamp_pending").is_none());
}

#[test]
fn the_stamp_is_stored_only_when_set_and_the_mark_never() {
    let (_t, raw) = mods();
    let mut p = migrated(&raw);
    let id = fixes(&p).next().unwrap().id.clone();
    let json = serde_json::to_value(by_id(&p, &id)).unwrap();
    assert!(json.get("vanilla_hashes").is_none());
    assert!(json.get("fresh").is_none());
    {
        let f = find_mut(&mut p, &id).unwrap();
        f.vanilla_hashes = BTreeMap::from([("main.lua".to_string(), "m1".to_string())]);
        f.fresh = true;
    }
    let json = serde_json::to_value(by_id(&p, &id)).unwrap();
    assert_eq!(json["vanilla_hashes"]["main.lua"], "m1");
    assert!(json.get("fresh").is_none());
    let back: ProfileFix = serde_json::from_value(json).unwrap();
    assert_eq!(back.vanilla_hashes.len(), 1);
    assert!(!back.fresh);
}
