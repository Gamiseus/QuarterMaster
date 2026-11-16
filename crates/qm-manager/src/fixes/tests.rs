//! Tests for the fix ledger: migration, the compiled plan and the ledger operations.
use super::*;
use crate::profiles::builtin_vanilla_profile;
use qm_core::handshake::StackMode;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// GCore's patch 16, as the user's profile turns it off.
const GCORE: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"gcore:main_lua:16\"\ntarget = \"main.lua\"\npattern = \"love.audio.play(snd_halt)\"\nposition = \"after\"\npayload = \"-- gcore halt\"\n";
const OTHER: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"other:a\"\ntarget = \"main.lua\"\npattern = \"love.audio.play(snd_halt)\"\nposition = \"before\"\npayload = \"-- other\"\n";

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
    let raw = plan::load_raw(t.path(), None).unwrap();
    (t, raw)
}

/// An unmigrated profile with the legacy fields `f` sets.
fn old_profile(f: impl FnOnce(&mut ModProfile)) -> ModProfile {
    let mut p = builtin_vanilla_profile(None);
    p.id = "p".into();
    p.enabled_mods = vec!["gcore".into(), "other".into()];
    p.fixes_version = 0;
    f(&mut p);
    p
}

fn the_users_profile() -> ModProfile {
    old_profile(|p| p.disabled_patches = vec!["gcore:main_lua:16".into()])
}

fn vanilla() -> HashMap<String, String> {
    HashMap::from([(
        "main.lua".to_string(),
        "gameVersion = \"0.5.3b\"\nlove.audio.play(snd_halt)\n".to_string(),
    )])
}

#[test]
fn the_users_profile_migrates_to_one_pinned_disable() {
    let (_t, raw) = mods();
    let old = the_users_profile();
    let m = migrate_checked(&old, &raw, None).unwrap();
    assert_eq!(m.imported, 1);
    let p = m.profile;
    assert_eq!(p.fixes_version, FIXES_VERSION);
    let all: Vec<&ProfileFix> = fixes(&p).collect();
    assert_eq!(all.len(), 1);
    let f = all[0];
    assert!(f.id.starts_with("fx_") && f.id.len() == 15, "{}", f.id);
    assert_eq!(f.label, "Turned off before this version");
    assert_eq!(f.provenance.source, FixSource::Migrated);
    let FixChange::Disable { patch } = &f.change else {
        panic!("{:?}", f.change)
    };
    assert_eq!(patch.id, "gcore:main_lua:16");
    assert_eq!(patch.mod_id, "gcore");
    let fp = patch.fp.as_ref().expect("fingerprint pinned");
    assert_eq!((fp.source.as_str(), fp.ordinal), ("qm.toml", 1));
    assert_eq!(fp.mod_version.as_deref(), Some("1.0.0"));
    let snap = f
        .snapshots
        .get("gcore:main_lua:16")
        .expect("snapshot stored");
    assert_eq!(snap.payload, "-- gcore halt");
    assert_eq!(
        f.provenance.mods,
        [("gcore".to_string(), Some("1.0.0".to_string()))]
    );
    // The legacy field is unchanged, the journal says what happened, the notice is pending.
    assert_eq!(p.disabled_patches, ["gcore:main_lua:16"]);
    assert_eq!(p.fix_journal.len(), 1);
    let entry = p.fix_journal[0].ok().unwrap();
    assert_eq!(entry.op, "migrated");
    assert_eq!(entry.added, std::slice::from_ref(&f.id));
    assert!(!p.notice_seen(NOTICE_FIXES_IMPORT));

    // The plan binds the disable to that one patch.
    let plan = compile_plan(&p, vec!["gcore".into()], HashMap::new());
    assert!(plan.refs.contains_key("gcore:main_lua:16"));
    assert_eq!(
        plan.fix_of
            .get("disable:gcore:main_lua:16")
            .and_then(Lenient::ok),
        Some(&f.id)
    );

    // Idempotent: a migrated profile is left as it is.
    let again = migrate(&p, &raw, None).unwrap();
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        serde_json::to_value(&p).unwrap()
    );
}

#[test]
fn migration_without_game_files() {
    let (_t, raw) = mods();
    let m = migrate_checked(&the_users_profile(), &raw, None).unwrap();
    assert_eq!(
        m.text_files, 0,
        "no vanilla files: the structural guard only"
    );
    assert_eq!(m.profile.fixes_version, 1);
}

#[test]
fn migration_text_guard_with_vanilla() {
    let (_t, raw) = mods();
    let files = vanilla();
    let m = migrate_checked(&the_users_profile(), &raw, Some(&files)).unwrap();
    assert_eq!(m.text_files, 1, "main.lua compared");
    assert_eq!(m.profile.fixes_version, 1);
}

#[test]
fn every_legacy_field_migrates_with_the_same_output() {
    let (_t, raw) = mods();
    let glue = GluePatchEntry {
        id: "glue:other:a".into(),
        name: "Adjusted copy of other:a".into(),
        replaces_patch_id: Some("other:a".into()),
        priority: 5,
        patch: raw.patches[1].patch.clone(),
        base_mod_id: Some("other".into()),
        run_as: None,
        exact: false,
    };
    let old = old_profile(|p| {
        p.disabled_patches = vec!["other:a".into(), "gone:x".into()];
        p.glue_patches = vec![glue];
        p.patch_priorities = HashMap::from([("gcore:main_lua:16".into(), 3)]);
        p.patch_order = vec![OrderRule {
            patch: "gcore:main_lua:16".into(),
            before: vec!["other:*".into()],
            after: Vec::new(),
        }];
    });
    let files = vanilla();
    let m = migrate_checked(&old, &raw, Some(&files)).unwrap();
    let kinds: Vec<&str> = fixes(&m.profile).map(ProfileFix::kind).collect();
    // The copy owns the disable of its base.
    assert_eq!(kinds, ["disable", "adjusted_copy", "order", "priority"]);
    let gone = fixes(&m.profile).next().unwrap();
    let FixChange::Disable { patch } = &gone.change else {
        panic!()
    };
    assert!(
        patch.fp.is_none(),
        "a patch that is not loaded keeps matching by name"
    );
    let copy = fixes(&m.profile).nth(1).unwrap();
    assert!(matches!(
        &copy.change,
        FixChange::AdjustedCopy { disables_base: true, base: Some(b), .. } if b.fp.is_some()
    ));
    let mut a = m.profile.disabled_patches.clone();
    a.sort();
    assert_eq!(a, ["gone:x", "other:a"]);
}

#[test]
fn user_load_after_migrates_to_mod_order() {
    let (_t, raw) = mods();
    let old = old_profile(|p| {
        p.user_load_after = HashMap::from([("gcore".into(), vec!["other".into()])]);
        p.mod_priorities = HashMap::from([("other".into(), 40)]);
    });
    let p = migrate(&old, &raw, None).unwrap();
    let f = fixes(&p).next().unwrap();
    assert!(matches!(
        &f.change,
        FixChange::ModOrder { move_mod_id, before_mod_id, priority: None }
            if move_mod_id == "other" && before_mod_id == "gcore"
    ));
    assert_eq!(
        f.undo,
        UndoRecord::RestoreModOrder {
            prev_priority: None,
            added_load_after: true
        }
    );
    // The load order itself is not a fix and stays a profile field.
    assert_eq!(p.mod_priorities, old.mod_priorities);
    assert_eq!(p.user_load_after, old.user_load_after);
    assert!(mod_order_intact(&p, f));
}

#[test]
fn a_profile_with_nothing_to_import_sees_no_notice() {
    let p = migrate(&old_profile(|_| {}), &RawLoad::default(), None).unwrap();
    assert_eq!(p.fixes_version, 1);
    assert!(p.fixes.is_empty() && p.fix_journal.is_empty());
    assert!(p.notice_seen(NOTICE_FIXES_IMPORT));
}

#[test]
fn a_forced_failure_keeps_the_profile() {
    let (_t, raw) = mods();
    set_fail_migration(true);
    let e = migrate(&the_users_profile(), &raw, None).unwrap_err();
    set_fail_migration(false);
    assert!(e.contains("stopped"), "{e}");
}

#[test]
fn load_order_json_has_no_snapshots() {
    let (_t, raw) = mods();
    let p = migrate(&the_users_profile(), &raw, None).unwrap();
    assert!(serde_json::to_string(&p).unwrap().contains("\"snapshots\""));
    let plan = compile_plan(&p, vec!["gcore".into()], HashMap::new());
    let json = serde_json::to_string(&plan).unwrap();
    assert!(
        !json.contains("snapshots") && !json.contains("taken_at"),
        "{json}"
    );
}

#[test]
fn paused_fixes_are_not_compiled() {
    let (_t, raw) = mods();
    let mut p = migrate(&the_users_profile(), &raw, None).unwrap();
    let id = fixes(&p).next().unwrap().id.clone();
    find_mut(&mut p, &id).unwrap().paused = true;
    sync_legacy(&mut p);
    assert!(p.disabled_patches.is_empty());
    let plan = compile_plan(&p, Vec::new(), HashMap::new());
    assert!(plan.disabled_patches.is_empty() && plan.refs.is_empty() && plan.fix_of.is_empty());
}

#[test]
fn legacy_ops_on_an_unmigrated_profile_match_the_old_commands() {
    let mut p = old_profile(|_| {});
    let prov = || Provenance::new(FixSource::PatchToggle);
    let a = HashSet::from(["x".to_string(), "m:x".to_string()]);
    assert_eq!(op_disable(&mut p, None, "x", &a, prov()), None);
    op_disable(&mut p, None, "m:x", &a, prov());
    assert_eq!(p.disabled_patches, ["x"]);
    op_enable(&mut p, &a);
    assert!(p.disabled_patches.is_empty());
    let rule = OrderRule {
        patch: "x".into(),
        before: vec!["y".into()],
        after: Vec::new(),
    };
    op_order_set(&mut p, None, rule.clone(), prov());
    op_order_set(&mut p, None, rule.clone(), prov());
    assert_eq!(p.patch_order, [rule]);
    op_order_reset(&mut p, "x");
    assert!(p.patch_order.is_empty());
    assert!(p.fixes.is_empty(), "no ledger before migration");
}

fn glue_for(base: &str, raw: &RawLoad) -> GluePatchEntry {
    GluePatchEntry {
        id: format!("glue:{base}"),
        name: format!("Adjusted copy of {base}"),
        replaces_patch_id: Some(base.to_string()),
        priority: 1,
        patch: raw.patches[0].patch.clone(),
        base_mod_id: None,
        run_as: None,
        exact: false,
    }
}

#[test]
fn glue_remove_keeps_a_pre_existing_disable() {
    let (_t, raw) = mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let base = "gcore:main_lua:16";
    let aliases = HashSet::from([base.to_string()]);
    let prov = || Provenance::new(FixSource::Resolver);
    op_disable(&mut p, Some(&raw), base, &aliases, prov());
    op_glue_add(&mut p, Some(&raw), glue_for(base, &raw), true, prov());
    let copy = fixes(&p).find(|f| f.kind() == "adjusted_copy").unwrap();
    assert_eq!(copy.undo, UndoRecord::KeepDisable);
    assert!(op_glue_remove(&mut p, &format!("glue:{base}")));
    assert_eq!(p.disabled_patches, [base], "the user's own disable stays");
    assert!(p.glue_patches.is_empty());

    // A copy made on a patch that was on turns it back on when it goes (legacy).
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    op_glue_add(&mut p, Some(&raw), glue_for(base, &raw), true, prov());
    assert_eq!(p.disabled_patches, [base]);
    assert!(op_glue_remove(&mut p, &format!("glue:{base}")));
    assert!(p.disabled_patches.is_empty());
    assert!(!op_glue_remove(&mut p, "glue:none"));
}

/// D58: "Run it like <mod>" switches a running legacy copy in place: it keeps its fix id and
/// undo, gets the mod it runs as, and can switch back.
#[test]
fn a_legacy_copy_runs_like_its_base_mod() {
    let (_t, raw) = mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let base = "gcore:main_lua:16";
    let glue_id = format!("glue:{base}");
    let prov = || Provenance::new(FixSource::Resolver);
    let fix = op_glue_add(&mut p, Some(&raw), glue_for(base, &raw), true, prov()).unwrap();
    assert!(op_glue_run_as(&mut p, Some(&raw), "glue:none", RunAs::Base).is_none());
    let id = op_glue_run_as(&mut p, Some(&raw), &glue_id, RunAs::Base);
    assert_eq!(id.as_deref(), Some(fix.as_str()));
    assert_eq!(p.glue_patches[0].run_as, Some(RunAs::Base));
    assert_eq!(p.glue_patches[0].base_mod_id.as_deref(), Some("gcore"));
    assert_eq!(find(&p, &fix).unwrap().undo, UndoRecord::Remove);
    assert_eq!(p.disabled_patches, [base], "the base stays off");
    // The switch is in the history, under a kind older versions read, and can go back.
    let last = find(&p, &fix).unwrap().provenance.history.last().cloned().unwrap();
    assert_eq!((last.what, last.detail.as_str()), (FixEventKind::Rebound, RUN_LIKE_EVENT));
    assert!(copy_switched_to_base(&find(&p, &fix).unwrap()));
    // Switching to the mode it has already logs nothing.
    let events = find(&p, &fix).unwrap().provenance.history.len();
    op_glue_run_as(&mut p, Some(&raw), &glue_id, RunAs::Base).unwrap();
    assert_eq!(find(&p, &fix).unwrap().provenance.history.len(), events);

    // Back to the older way: absent, as old copies have it; the mod stays recorded.
    op_glue_run_as(&mut p, Some(&raw), &glue_id, RunAs::Legacy).unwrap();
    assert_eq!(p.glue_patches[0].run_as, None);
    assert_eq!(p.glue_patches[0].base_mod_id.as_deref(), Some("gcore"));
    let last = find(&p, &fix).unwrap().provenance.history.last().cloned().unwrap();
    assert_eq!(last.detail, RUN_LAST_EVENT);
    assert!(!copy_switched_to_base(&find(&p, &fix).unwrap()));

    // A paused copy is not changed, nor is a profile that is not migrated.
    find_mut(&mut p, &fix).unwrap().paused = true;
    assert!(op_glue_run_as(&mut p, Some(&raw), &glue_id, RunAs::Base).is_none());
    let mut old = old_profile(|q| q.glue_patches = vec![glue_for(base, &raw)]);
    assert!(op_glue_run_as(&mut old, Some(&raw), &glue_id, RunAs::Base).is_none());
    assert_eq!(old.glue_patches[0].run_as, None);
}

/// D59: a copy of a copy runs like the first copy's base mod; the older copy no longer runs.
#[test]
fn a_copy_of_a_copy_runs_like_the_first_bases_mod() {
    let (_t, raw) = mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = || Provenance::new(FixSource::Resolver);
    let first = glue_for("other:a", &raw);
    let second = GluePatchEntry {
        id: "glue:other:a:2".into(),
        replaces_patch_id: Some(first.id.clone()),
        ..glue_for("other:a", &raw)
    };
    op_glue_add(&mut p, Some(&raw), first.clone(), true, prov()).unwrap();
    op_glue_add(&mut p, Some(&raw), second.clone(), false, prov()).unwrap();
    assert_eq!(copy_base_mod(&p, &raw, &second).as_deref(), Some("other"));
    assert_eq!(copy_base_mod(&p, &raw, &first).as_deref(), Some("other"));
    assert!(copy_superseded(&p, &first.id));
    assert!(!copy_superseded(&p, &second.id));
    assert_eq!(copy_fix(&p, &second.id).map(ProfileFix::kind), Some("adjusted_copy"));

    // A copy of a patch that is not loaded runs last either way.
    let gone = GluePatchEntry {
        replaces_patch_id: Some("gone:x".into()),
        ..glue_for("other:a", &raw)
    };
    assert_eq!(copy_base_mod(&p, &raw, &gone), None);
}

#[test]
fn undo_with_a_later_fix_on_the_same_patch() {
    let (_t, raw) = mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let id = "gcore:main_lua:16";
    let prov = || Provenance::new(FixSource::Resolver);
    let first = op_priority_set(&mut p, Some(&raw), id, 10, prov()).unwrap();
    let rule = OrderRule {
        patch: id.into(),
        before: vec!["other:a".into()],
        after: Vec::new(),
    };
    let order = op_order_set(&mut p, Some(&raw), rule, prov()).unwrap();
    // Replacing the priority keeps the old one for undo.
    let second = op_priority_set(&mut p, Some(&raw), id, 20, prov()).unwrap();
    assert!(find(&p, &first).is_none());
    assert_eq!(
        find(&p, &second).unwrap().undo,
        UndoRecord::RestorePriority(Some(10))
    );
    assert_eq!(later_on_same_patch(&p, &order), std::slice::from_ref(&second));
    assert!(later_on_same_patch(&p, &second).is_empty());

    // Undoing the newest puts the earlier priority back.
    undo(&mut p, Some(&raw), &second).unwrap();
    assert_eq!(p.patch_priorities.get(id), Some(&10));
    assert_eq!(p.patch_order.len(), 1);
    undo(&mut p, Some(&raw), &order).unwrap();
    assert!(p.patch_order.is_empty());
}

#[test]
fn a_replaced_anchor_fix_comes_back_on_undo() {
    let (_t, raw) = mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let id = "other:a";
    let prov = || Provenance::new(FixSource::Resolver);
    let rung = |b_site| Rung {
        within: Some("gcore:main_lua:16".into()),
        pattern: Some("-- gcore halt".into()),
        exact: true,
        b_site: Some(b_site),
        ..Rung::default()
    };
    let site = |n| SiteKey {
        context: "ctx".into(),
        ordinal: Some(n),
    };
    let label = Some("Place 1".to_string());
    let first =
        op_anchor_set(&mut p, Some(&raw), id, vec![rung(0)], vec![site(1)], label, prov()).unwrap();
    assert_eq!(find(&p, &first).unwrap().undo, UndoRecord::Remove);
    // Moving another place replaces the fix; undo puts the first one back.
    let second =
        op_anchor_set(&mut p, Some(&raw), id, vec![rung(1)], vec![site(2)], None, prov()).unwrap();
    assert!(find(&p, &first).is_none());
    assert!(matches!(
        &find(&p, &second).unwrap().undo,
        UndoRecord::RestoreAnchor { label, sites, .. } if label == "Place 1" && sites == &[site(1)]
    ));
    undo(&mut p, Some(&raw), &second).unwrap();
    let back: Vec<&ProfileFix> = fixes(&p).collect();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].label, "Place 1");
    let FixChange::Anchor { patch, anchors, sites, .. } = &back[0].change else {
        panic!("an anchor fix");
    };
    assert_eq!(patch.id, id);
    assert_eq!(anchors[0].b_site, Some(0));
    assert!(anchors[0].b_fp.is_some(), "B's fingerprint is kept");
    assert_eq!(sites, &[site(1)]);
    assert_eq!(back[0].undo, UndoRecord::Remove);
}

#[test]
fn mod_order_undo_reverses_only_what_is_still_there() {
    let mut p = old_profile(|p| p.mod_priorities = HashMap::from([("other".into(), 50)]));
    p.fixes_version = 1;
    let prov = || Provenance::new(FixSource::Resolver);
    let id = op_mod_order(&mut p, None, "other", "gcore", 5, prov()).unwrap();
    assert_eq!(
        p.user_load_after.get("gcore"),
        Some(&vec!["other".to_string()])
    );
    let f = find(&p, &id).unwrap();
    assert!(undo_asks(&p, &f).is_empty());
    let mut q = p.clone();
    undo(&mut q, None, &id).unwrap();
    assert_eq!(q.mod_priorities.get("other"), Some(&50));
    assert!(q.user_load_after.is_empty());

    // The load order was edited since: undo asks, and keeps the user's priority.
    p.mod_priorities.insert("other".into(), 70);
    assert!(!mod_order_intact(&p, &f));
    assert_eq!(undo_asks(&p, &f).len(), 1);
    undo(&mut p, None, &id).unwrap();
    assert_eq!(p.mod_priorities.get("other"), Some(&70));
    assert!(p.user_load_after.is_empty());
}

#[test]
fn accept_pins_the_patch_as_it_is_now() {
    let (t, raw) = mods();
    let mut p = migrate(&the_users_profile(), &raw, None).unwrap();
    let id = fixes(&p).next().unwrap().id.clone();
    write(
        t.path(),
        "gcore/qm.toml",
        &GCORE.replace("-- gcore halt", "-- gcore halt v2"),
    );
    let raw2 = plan::load_raw(t.path(), None).unwrap();
    let old_fp = fixes(&p).next().unwrap().refs()[0].fp.clone();
    assert!(accept(&mut p, &raw2, &id));
    let f = find(&p, &id).unwrap();
    assert_ne!(f.refs()[0].fp, old_fp);
    assert_eq!(f.snapshots["gcore:main_lua:16"].payload, "-- gcore halt v2");
    assert_eq!(
        f.provenance.history.last().unwrap().what,
        FixEventKind::Accepted
    );

    // Re-binding points the disable at another patch.
    rebind(&mut p, &raw2, &id, "other:a").unwrap();
    assert_eq!(p.disabled_patches, ["other:a"]);
    assert!(rebind(&mut p, &raw2, &id, "nope").is_err());
}

#[test]
fn reconcile_adopts_hand_edits() {
    let (_t, raw) = mods();
    let mut p = migrate(&the_users_profile(), &raw, None).unwrap();
    p.disabled_patches.push("other:a".into());
    assert_eq!(reconcile(&mut p, Some(&raw)), 1);
    sync_legacy(&mut p);
    assert_eq!(p.disabled_patches, ["gcore:main_lua:16", "other:a"]);
    assert_eq!(reconcile(&mut p, Some(&raw)), 0);
}

#[test]
fn clones_get_new_fix_ids_and_keep_stacking() {
    let temp = TempDir::new().unwrap();
    let paths = crate::storage::QuarterMasterPaths::resolve(Some(temp.path()));
    let (_m, raw) = mods();
    let mut src = migrate(&the_users_profile(), &raw, None).unwrap();
    src.id = "src".into();
    src.stacking = Some(StackMode::LoadOrder);
    paths.save_profile(&src).unwrap();
    let old_id = fixes(&src).next().unwrap().id.clone();
    for clone in [
        paths.clone_profile("src", "c1", "C1").unwrap(),
        paths
            .create_profile_with_id(Some("c2"), "C2", Some("src"), None, None)
            .unwrap(),
    ] {
        assert_eq!(clone.stacking, Some(StackMode::LoadOrder));
        assert_eq!(clone.notices_seen, src.notices_seen);
        let ids: Vec<&String> = fixes(&clone).map(|f| &f.id).collect();
        assert_eq!(ids.len(), 1);
        assert_ne!(ids[0], &old_id);
        assert!(clone.fix_journal.is_empty());
        assert_eq!(clone.disabled_patches, src.disabled_patches);
    }
    // A new profile has nothing to migrate.
    let fresh = paths.create_profile("Fresh", None, None, None).unwrap();
    assert_eq!(fresh.fixes_version, FIXES_VERSION);
}

#[test]
fn unknown_fix_kinds_round_trip() {
    let mut p = builtin_vanilla_profile(None);
    p.fixes = serde_json::from_str(r#"[{"id": "fx_new", "label": "x", "change": {"kind": "stack_rule"}, "provenance": {"created_at": "t", "source": "resolver"}}]"#).unwrap();
    assert!(p.fixes[0].error().is_some());
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["fixes"][0]["change"]["kind"], "stack_rule");
    assert_eq!(fixes(&p).count(), 0);
    let plan = compile_plan(&p, Vec::new(), HashMap::new());
    assert!(plan.fix_of.is_empty());
}

/// A data dir with GCore in the library and the default profile turning off its patch 16, as
/// a profile from before the ledger.
fn library_with_users_profile() -> (TempDir, crate::storage::QuarterMasterPaths) {
    let temp = TempDir::new().unwrap();
    let paths = crate::storage::QuarterMasterPaths::resolve(Some(&temp.path().join("data")));
    let src = temp.path().join("src").join("gcore");
    write(
        &src,
        "mod.json",
        r#"{"id": "gcore", "name": "GCore", "version": "1.0.0"}"#,
    );
    write(&src, "qm.toml", GCORE);
    paths.import_mod_from_path(&src, None, false).unwrap();
    let mut p = paths.get_profile("default").unwrap();
    p.enabled_mods = vec!["gcore".into()];
    p.disabled_patches = vec!["gcore:main_lua:16".into()];
    p.fixes_version = 0;
    paths.save_profile(&p).unwrap();
    (temp, paths)
}

fn staged(paths: &crate::storage::QuarterMasterPaths) -> LoadOrderPlan {
    LoadOrderPlan::read(&paths.profiles_dir.join("default").join("Mods"))
        .plan
        .unwrap()
}

#[test]
fn activation_migrates_once_and_keeps_a_backup() {
    let (_t, paths) = library_with_users_profile();
    paths.activate_profile("default").unwrap();
    let p = paths.get_profile("default").unwrap();
    assert_eq!(p.fixes_version, 1);
    let ids: Vec<String> = fixes(&p).map(|f| f.id.clone()).collect();
    assert_eq!(ids.len(), 1);
    let bak = fs::read_to_string(paths.fixes_backup_file("default")).unwrap();
    let bak: ModProfile = serde_json::from_str(&bak).unwrap();
    assert_eq!(bak.fixes_version, 0);
    assert_eq!(bak.disabled_patches, ["gcore:main_lua:16"]);
    let plan = staged(&paths);
    assert!(plan.refs.contains_key("gcore:main_lua:16"));
    assert!(plan.is_disabled(&["gcore:main_lua:16"]));

    // Once only.
    paths.activate_profile("default").unwrap();
    let again = paths.get_profile("default").unwrap();
    let ids2: Vec<String> = fixes(&again).map(|f| f.id.clone()).collect();
    assert_eq!(ids, ids2);
    assert_eq!(again.fix_journal.len(), 1);
}

#[test]
fn migration_abort_keeps_schema_0_and_the_legacy_plan() {
    let (_t, paths) = library_with_users_profile();
    set_fail_migration(true);
    let a = paths.activate_profile_reporting("default");
    set_fail_migration(false);
    let a = a.unwrap();
    assert!(
        a.warnings
            .iter()
            .any(|w| w.contains("could not be imported")),
        "{:?}",
        a.warnings
    );
    let p = paths.get_profile("default").unwrap();
    assert_eq!(p.fixes_version, 0);
    assert!(p.fixes.is_empty());
    assert!(p
        .migration_error
        .as_deref()
        .unwrap_or("")
        .contains("stopped"));
    let plan = staged(&paths);
    assert!(plan.is_disabled(&["gcore:main_lua:16"]));
    assert!(plan.refs.is_empty() && plan.fix_of.is_empty());
    assert!(!paths.fixes_backup_file("default").exists());

    // The next activation imports them.
    paths.activate_profile("default").unwrap();
    let p = paths.get_profile("default").unwrap();
    assert_eq!(p.fixes_version, 1);
    assert!(p.migration_error.is_none());
}

/// Points the migrated disable at a mod that is not loaded: the plan fields stay the same,
/// the game's patches do not.
fn unpin_the_disable(p: &mut ModProfile) {
    for f in fixes_mut(p) {
        if let FixChange::Disable { patch } = &mut f.change {
            patch.mod_id = "elsewhere".into();
        }
    }
}

#[test]
fn the_byte_guard_rejects_a_real_difference() {
    let (_t, raw) = mods();
    let old = the_users_profile();
    set_tamper_migration(Some(unpin_the_disable));
    let e = migrate_checked(&old, &raw, None).unwrap_err();
    set_tamper_migration(None);
    assert!(e.contains("would change the game's patches"), "{e}");

    // The text guard on its own sees the same difference in main.lua.
    let mut new = migrate(&old, &raw, None).unwrap();
    unpin_the_disable(&mut new);
    same_fields(&old, &new).unwrap();
    let e = same_text(&old, &new, &raw, &vanilla()).unwrap_err();
    assert!(e.contains("would change main.lua"), "{e}");
    assert!(same_applied(&old, &new, &raw).is_err());
}

#[test]
fn an_activation_whose_guard_fails_keeps_schema_0() {
    let (_t, paths) = library_with_users_profile();
    set_tamper_migration(Some(unpin_the_disable));
    let a = paths.activate_profile_reporting("default");
    set_tamper_migration(None);
    a.unwrap();
    let p = paths.get_profile("default").unwrap();
    assert_eq!(p.fixes_version, 0);
    assert!(p.fixes.is_empty());
    let why = p.migration_error.unwrap_or_default();
    assert!(why.contains("would change"), "{why}");
    assert!(staged(&paths).refs.is_empty());
}

#[test]
fn an_undone_import_is_not_run_again() {
    let (_t, raw) = mods();
    let mut old = the_users_profile();
    old.migration_error = Some(IMPORT_UNDONE.into());
    assert_eq!(migrate(&old, &raw, None).unwrap_err(), IMPORT_UNDONE);
    old.migration_error = None;
    assert_eq!(migrate(&old, &raw, None).unwrap().fixes_version, 1);
}

/// One mod `a` with patch `z_patch` named `n`: two names for one patch.
fn named_mod() -> (TempDir, RawLoad) {
    let t = TempDir::new().unwrap();
    write(
        t.path(),
        "a/mod.json",
        r#"{"id": "a", "name": "A", "version": "1.0.0"}"#,
    );
    write(
        t.path(),
        "a/qm.toml",
        "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"z_patch\"\nname = \"n\"\ntarget = \"main.lua\"\npattern = \"love.audio.play(snd_halt)\"\nposition = \"after\"\npayload = \"-- a\"\n",
    );
    let raw = plan::load_raw(t.path(), None).unwrap();
    (t, raw)
}

fn priority_of(raw: &RawLoad, p: &ModProfile) -> Priority {
    let enabled = raw.mods.iter().map(|m| m.id.clone()).collect();
    let a = applied_for(raw, &compile_plan(p, enabled, HashMap::new())).unwrap();
    a.patches
        .iter()
        .find(|(x, _, _)| x.id() == Some("z_patch"))
        .map(|(_, prio, _)| *prio)
        .unwrap()
}

#[test]
fn priorities_on_two_names_of_one_patch_keep_the_legacy_order() {
    let (_t, raw) = named_mod();
    let old = old_profile(|p| {
        p.enabled_mods = vec!["a".into()];
        p.patch_priorities = HashMap::from([("z_patch".into(), 10), ("a:n".into(), 20)]);
    });
    assert_eq!(priority_of(&raw, &old), 10, "the id comes first");
    let mut p = migrate(&old, &raw, None).unwrap();
    assert_eq!(fixes(&p).filter(|f| f.kind() == "priority").count(), 2);
    assert_eq!(priority_of(&raw, &p), 10, "pinned: still the id first");

    // A new priority on one name replaces the fixes on both names.
    let prov = Provenance::new(FixSource::Resolver);
    let id = op_priority_set(&mut p, Some(&raw), "a:n", 30, prov).unwrap();
    assert_eq!(fixes(&p).filter(|f| f.kind() == "priority").count(), 1);
    assert_eq!(p.patch_priorities, HashMap::from([("a:n".into(), 30)]));
    assert_eq!(priority_of(&raw, &p), 30);
    // Undo puts back the one that took effect.
    assert_eq!(
        find(&p, &id).unwrap().undo,
        UndoRecord::RestorePriority(Some(10))
    );
    undo(&mut p, Some(&raw), &id).unwrap();
    assert_eq!(priority_of(&raw, &p), 10);
}

#[test]
fn a_disable_taken_out_by_hand_stays_out() {
    let (_t, raw) = mods();
    let mut p = migrate(&the_users_profile(), &raw, None).unwrap();
    let first = fixes(&p).next().unwrap().id.clone();
    // The old launcher turns the patch back on in profile.json.
    p.disabled_patches.clear();
    assert_eq!(reconcile(&mut p, Some(&raw)), 0);
    assert!(find(&p, &first).unwrap().paused);
    let aliases = HashSet::from(["other:a".to_string()]);
    let prov = Provenance::new(FixSource::PatchToggle);
    op_disable(&mut p, Some(&raw), "other:a", &aliases, prov);
    assert_eq!(
        p.disabled_patches,
        ["other:a"],
        "gcore:main_lua:16 stays on"
    );

    // A priority changed by hand: the old fix is paused, the new value adopted.
    let prov = Provenance::new(FixSource::Resolver);
    let prio = op_priority_set(&mut p, Some(&raw), "other:a", 5, prov).unwrap();
    p.patch_priorities.insert("other:a".into(), 9);
    assert_eq!(reconcile(&mut p, Some(&raw)), 1);
    sync_legacy(&mut p);
    assert!(find(&p, &prio).unwrap().paused);
    assert_eq!(p.patch_priorities.get("other:a"), Some(&9));
}

#[test]
fn a_copy_whose_base_was_turned_back_on_does_not_turn_it_off_again() {
    let (_t, raw) = mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let base = "gcore:main_lua:16";
    let aliases = HashSet::from([base.to_string()]);
    let prov = || Provenance::new(FixSource::Resolver);
    op_disable(&mut p, Some(&raw), base, &aliases, prov());
    let copy = op_glue_add(&mut p, Some(&raw), glue_for(base, &raw), true, prov()).unwrap();
    assert_eq!(find(&p, &copy).unwrap().undo, UndoRecord::KeepDisable);
    op_enable(&mut p, &aliases);
    assert!(p.disabled_patches.is_empty());
    undo(&mut p, Some(&raw), &copy).unwrap();
    assert!(p.disabled_patches.is_empty(), "the base stays on");
    assert!(fixes(&p).next().is_none());
}

#[test]
fn accept_keeps_the_snapshot_of_a_patch_that_is_not_loaded() {
    let (_t, raw) = mods();
    let mut p = migrate(&the_users_profile(), &raw, None).unwrap();
    let id = fixes(&p).next().unwrap().id.clone();
    let t2 = TempDir::new().unwrap();
    write(
        t2.path(),
        "other/mod.json",
        r#"{"id": "other", "name": "Other", "version": "2.0.0"}"#,
    );
    write(t2.path(), "other/qm.toml", OTHER);
    let without_gcore = plan::load_raw(t2.path(), None).unwrap();
    assert!(accept(&mut p, &without_gcore, &id));
    let f = find(&p, &id).unwrap();
    assert_eq!(f.snapshots["gcore:main_lua:16"].payload, "-- gcore halt");
}

/// A fix the user paused stays paused when a new one is set on the same patch: undoing the
/// new one does not turn the paused one on (it was not running before).
#[test]
fn setting_a_fix_keeps_a_paused_one_paused() {
    let (_t, raw) = mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = || Provenance::new(FixSource::Resolver);
    let pause = |p: &mut ModProfile, id: &str| {
        find_mut(p, id).unwrap().paused = true;
        sync_legacy(p);
    };

    // Priority.
    let five = op_priority_set(&mut p, Some(&raw), "other:a", 5, prov()).unwrap();
    pause(&mut p, &five);
    assert!(p.patch_priorities.is_empty());
    let nine = op_priority_set(&mut p, Some(&raw), "other:a", 9, prov()).unwrap();
    assert!(find(&p, &five).unwrap().paused, "kept, paused");
    assert_eq!(find(&p, &nine).unwrap().undo, UndoRecord::Remove);
    assert_eq!(p.patch_priorities.get("other:a"), Some(&9));
    assert_eq!(
        running_clashes(&p, &find(&p, &five).unwrap()),
        std::slice::from_ref(&nine)
    );
    undo(&mut p, Some(&raw), &nine).unwrap();
    assert!(
        p.patch_priorities.is_empty(),
        "no priority, as before the set"
    );
    assert!(find(&p, &five).unwrap().paused);
    assert!(running_clashes(&p, &find(&p, &five).unwrap()).is_empty());

    // Order rule.
    let rule = |b: &str| OrderRule {
        patch: "other:a".into(),
        before: vec![b.into()],
        after: Vec::new(),
    };
    let first = op_order_set(&mut p, Some(&raw), rule("gcore:main_lua:16"), prov()).unwrap();
    pause(&mut p, &first);
    let second = op_order_set(&mut p, Some(&raw), rule("x"), prov()).unwrap();
    assert!(find(&p, &first).unwrap().paused);
    assert_eq!(find(&p, &second).unwrap().undo, UndoRecord::Remove);
    assert_eq!(p.patch_order, [rule("x")], "one running rule");
    undo(&mut p, Some(&raw), &second).unwrap();
    assert!(p.patch_order.is_empty());

    // Anchor.
    let rung = Rung {
        within: Some("gcore:main_lua:16".into()),
        pattern: Some("-- gcore halt".into()),
        exact: true,
        ..Rung::default()
    };
    let a1 = op_anchor_set(
        &mut p,
        Some(&raw),
        "other:a",
        vec![rung.clone()],
        Vec::new(),
        None,
        prov(),
    )
    .unwrap();
    pause(&mut p, &a1);
    let a2 = op_anchor_set(
        &mut p,
        Some(&raw),
        "other:a",
        vec![rung],
        Vec::new(),
        None,
        prov(),
    )
    .unwrap();
    assert_eq!(find(&p, &a2).unwrap().undo, UndoRecord::Remove);
    undo(&mut p, Some(&raw), &a2).unwrap();
    assert!(find(&p, &a1).unwrap().paused);
    assert_eq!(fixes(&p).filter(|f| !f.paused).count(), 0);
}

/// A new copy never takes a paused copy's place: the paused one stays, the new one gets an
/// id of its own (and so does its patch, when it carried the copy's id).
#[test]
fn a_new_copy_keeps_a_paused_copy() {
    let (_t, raw) = mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let base = "gcore:main_lua:16";
    let prov = || Provenance::new(FixSource::Resolver);
    let old = op_glue_add(&mut p, Some(&raw), glue_for(base, &raw), true, prov()).unwrap();
    find_mut(&mut p, &old).unwrap().paused = true;
    sync_legacy(&mut p);
    assert!(p.glue_patches.is_empty());
    let id = format!("glue:{base}");
    assert!(glue_id_taken(&p, &id), "a paused copy's id is taken");

    let mut again = glue_for(base, &raw);
    again.patch.set_id(id.clone());
    let new = op_glue_add(&mut p, Some(&raw), again, true, prov()).unwrap();
    assert!(find(&p, &old).unwrap().paused, "the paused copy is kept");
    let FixChange::AdjustedCopy { glue, .. } = &find(&p, &new).unwrap().change else {
        panic!("a copy");
    };
    assert_eq!(glue.id, format!("{id}:2"));
    assert_eq!(glue.patch.id(), Some(glue.id.as_str()));
    assert_eq!(p.glue_patches.len(), 1);
    undo(&mut p, Some(&raw), &new).unwrap();
    assert!(find(&p, &old).unwrap().paused);
    assert!(p.glue_patches.is_empty());

    // A running copy with the same id is still replaced.
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let a = op_glue_add(&mut p, Some(&raw), glue_for(base, &raw), true, prov()).unwrap();
    let b = op_glue_add(&mut p, Some(&raw), glue_for(base, &raw), true, prov()).unwrap();
    assert!(find(&p, &a).is_none());
    let FixChange::AdjustedCopy { glue, .. } = &find(&p, &b).unwrap().change else {
        panic!("a copy");
    };
    assert_eq!(glue.id, id);
}

/// A copy edited by hand under its id (or by the old launcher's configure) is a hand edit:
/// the fix is paused and the edited copy adopted, so the next write keeps the edit.
#[test]
fn a_copy_edited_under_its_id_is_adopted() {
    let (_t, raw) = mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let base = "gcore:main_lua:16";
    let prov = || Provenance::new(FixSource::Resolver);
    let copy = op_glue_add(&mut p, Some(&raw), glue_for(base, &raw), true, prov()).unwrap();
    let before = p.clone();
    assert_eq!(reconcile(&mut p, Some(&raw)), 0);
    assert!(same_plan_fields(&before, &p).is_ok());

    p.glue_patches[0].priority = 42;
    assert!(
        same_plan_fields(&before, &p).is_err(),
        "content counts, not only the id"
    );
    assert_eq!(reconcile(&mut p, Some(&raw)), 1);
    assert!(find(&p, &copy).unwrap().paused);
    sync_legacy(&mut p);
    assert_eq!(p.glue_patches.len(), 1);
    assert_eq!(p.glue_patches[0].priority, 42, "the edit stays");
    assert_eq!(reconcile(&mut p, Some(&raw)), 0);
}

/// A journal entry (or a fix in it) of a kind this version does not know is kept as it is;
/// the profile still reads.
#[test]
fn unknown_journal_entries_round_trip() {
    let raw = r#"{"id": "p", "name": "P", "fix_journal": [
        {"at": "t", "op": "compat_export", "label": "x", "removed": [
            {"id": "fx_new", "label": "x", "change": {"kind": "stack_rule"}, "provenance": {"created_at": "t", "source": "resolver"}}
        ]},
        {"at": "t", "op": "newer_op", "label": "y", "removed": "not a list"}
    ]}"#;
    let mut v: serde_json::Value = serde_json::to_value(builtin_vanilla_profile(None)).unwrap();
    let extra: serde_json::Value = serde_json::from_str(raw).unwrap();
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    let p: ModProfile = serde_json::from_value(v).unwrap();
    let first = p.fix_journal[0].ok().unwrap();
    assert!(first.removed[0].error().is_some());
    assert!(p.fix_journal[1].error().is_some());
    let back = serde_json::to_value(&p).unwrap();
    assert_eq!(
        back["fix_journal"][0]["removed"][0]["change"]["kind"],
        "stack_rule"
    );
    assert_eq!(back["fix_journal"][1]["removed"], "not a list");
}

/// Fix labels name mods and patches as the user reads them (§11), not by their ids; a name
/// that is not one loaded patch stays as written.
#[test]
fn fix_labels_use_mod_and_patch_names() {
    let t = TempDir::new().unwrap();
    let toml = |id: &str, name: &str, position: &str| {
        format!("[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"{id}\"\nname = \"{name}\"\ntarget = \"main.lua\"\npattern = \"love.audio.play(snd_halt)\"\nposition = \"{position}\"\npayload = \"-- {id}\"\n")
    };
    write(
        t.path(),
        "gcore/mod.json",
        r#"{"id": "gcore", "name": "GCore", "version": "1.0.0"}"#,
    );
    write(t.path(), "gcore/qm.toml", &toml("gcore:halt", "Halt sound", "after"));
    write(
        t.path(),
        "other/mod.json",
        r#"{"id": "other", "name": "Tactical Overhaul", "version": "1.0.0"}"#,
    );
    write(t.path(), "other/qm.toml", &toml("other:a", "Halt order", "before"));
    let raw = plan::load_raw(t.path(), None).unwrap();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = || Provenance::new(FixSource::Resolver);
    let label = |p: &ModProfile, id: &str| find(p, id).unwrap().label;

    let aliases = HashSet::from(["gcore:halt".to_string()]);
    let off = op_disable(&mut p, Some(&raw), "gcore:halt", &aliases, prov()).unwrap();
    assert_eq!(label(&p, &off), "Turn off Halt sound");
    let rule = OrderRule {
        patch: "other:a".into(),
        before: vec!["gcore:halt".into()],
        after: Vec::new(),
    };
    let order = op_order_set(&mut p, Some(&raw), rule, prov()).unwrap();
    assert_eq!(label(&p, &order), "Run Halt order before Halt sound");
    let prio = op_priority_set(&mut p, Some(&raw), "other:a", 45, prov()).unwrap();
    assert_eq!(label(&p, &prio), "Priority 45 for Halt order");
    let swap = op_mod_order(&mut p, Some(&raw), "other", "gcore", 5, prov()).unwrap();
    assert_eq!(label(&p, &swap), "Load Tactical Overhaul before GCore");

    let gone = HashSet::from(["gone:x".to_string()]);
    let off = op_disable(&mut p, Some(&raw), "gone:x", &gone, prov()).unwrap();
    assert_eq!(label(&p, &off), "Turn off gone:x");
    let swap = op_mod_order(&mut p, None, "other", "gcore", 5, prov()).unwrap();
    assert_eq!(label(&p, &swap), "Load other before gcore");
}

// ---- ApplyAs fixes (D46, Task 19b) ----

/// GCore's `at` patch on three lines and another mod's `at` patch, both literal by default.
const GCORE_AT: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"gcore:main_lua:16\"\nname = \"Clear pathfinding on Halt\"\ntarget = \"main.lua\"\npattern = \"a = 1\\nb = 2\\nc = 3\"\nposition = \"at\"\npayload = \"a = 1\\nb = 2\\nx = 0\\nc = 3\"\n";
const OTHER_AT: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"other:a\"\ntarget = \"main.lua\"\npattern = \"d = 4\"\nposition = \"at\"\npayload = \"d = 4\\ny = 0\"\n";

const HALT: &str = "gcore:main_lua:16";

fn at_mods() -> (TempDir, RawLoad) {
    let t = TempDir::new().unwrap();
    write(
        t.path(),
        "gcore/mod.json",
        r#"{"id": "gcore", "name": "GCore", "version": "1.0.0"}"#,
    );
    write(t.path(), "gcore/qm.toml", GCORE_AT);
    write(
        t.path(),
        "other/mod.json",
        r#"{"id": "other", "name": "Other", "version": "2.0.0"}"#,
    );
    write(t.path(), "other/qm.toml", OTHER_AT);
    let raw = plan::load_raw(t.path(), None).unwrap();
    (t, raw)
}

/// How each pattern patch of `p`'s plan asks to run, with the fix ids it carries.
fn modes(raw: &RawLoad, p: &ModProfile) -> BTreeMap<String, (ApplyAs, Vec<String>)> {
    let plan = compile_plan(p, vec!["gcore".into(), "other".into()], HashMap::new());
    let a = applied_for(raw, &plan).unwrap();
    a.patches
        .iter()
        .filter_map(|(p, _, _)| match p {
            qm_core::patch::Patch::Pattern(x) => Some((
                x.id.clone().unwrap_or_default(),
                (x.asked_apply_as(), x.qm.fix_ids.clone()),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn an_apply_as_fix_is_stored_compiled_and_round_trips() {
    let (_t, raw) = at_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = Provenance::new(FixSource::Resolver);
    let id = op_apply_as_set(&mut p, Some(&raw), HALT, ApplyAs::Edits, None, prov).unwrap();
    let f = find(&p, &id).unwrap();
    assert_eq!(f.kind(), "apply_as");
    assert_eq!(f.label, "Run Clear pathfinding on Halt as separate edits");
    assert_eq!(f.subject(), Some(HALT));
    assert_eq!(f.undo, UndoRecord::Remove);
    let FixChange::ApplyAs { patch, mode } = &f.change else {
        panic!("{:?}", f.change)
    };
    assert_eq!((patch.mod_id.as_str(), *mode), ("gcore", ApplyAs::Edits));
    assert!(patch.fp.is_some(), "pinned");
    assert!(f.snapshots.contains_key(HALT));
    assert_eq!(
        f.provenance.mods,
        [("gcore".to_string(), Some("1.0.0".to_string()))]
    );
    // No legacy field holds it.
    assert!(!has_legacy_entries(&p));

    // profile.json round trip.
    let json = serde_json::to_string(&p).unwrap();
    assert!(
        json.contains(r#""kind":"apply_as""#) && json.contains(r#""mode":"edits""#),
        "{json}"
    );
    let back: ModProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
    assert_eq!(find(&back, &id).unwrap().kind(), "apply_as");

    // The plan carries the mode, the ref and the fix id; the game runs it as edits.
    let plan = compile_plan(&p, vec!["gcore".into()], HashMap::new());
    assert_eq!(plan.apply_as.get(HALT), Some(&Lenient::Ok(ApplyAs::Edits)));
    assert!(plan.refs.contains_key(HALT));
    assert_eq!(
        plan.fix_of
            .get(&format!("apply_as:{HALT}"))
            .and_then(Lenient::ok),
        Some(&id)
    );
    let m = modes(&raw, &p);
    assert_eq!(m[HALT], (ApplyAs::Edits, vec![id.clone()]));
    assert_eq!(m["other:a"], (ApplyAs::Literal, Vec::new()));
    let a = applied_for(&raw, &plan).unwrap();
    let b = a.report.bindings.iter().find(|b| b.fix_id == id).unwrap();
    assert_eq!(b.key, format!("apply_as:{HALT}"));
    assert_eq!(b.status, qm_core::handshake::BindingStatus::Bound);
}

#[test]
fn a_paused_apply_as_fix_leaves_the_patch_literal() {
    let (_t, raw) = at_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = Provenance::new(FixSource::Resolver);
    let id = op_apply_as_set(&mut p, Some(&raw), HALT, ApplyAs::Edits, None, prov).unwrap();
    find_mut(&mut p, &id).unwrap().paused = true;
    sync_legacy(&mut p);
    let plan = compile_plan(&p, vec!["gcore".into()], HashMap::new());
    assert!(plan.apply_as.is_empty() && plan.fix_of.is_empty() && plan.refs.is_empty());
    assert_eq!(modes(&raw, &p)[HALT], (ApplyAs::Literal, Vec::new()));
}

#[test]
fn a_replaced_apply_as_fix_comes_back_on_undo() {
    let (_t, raw) = at_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = || Provenance::new(FixSource::Resolver);
    let label = Some("Run it as separate edits".to_string());
    let first = op_apply_as_set(&mut p, Some(&raw), HALT, ApplyAs::Edits, label, prov()).unwrap();
    assert_eq!(find(&p, &first).unwrap().label, "Run it as separate edits");
    let second =
        op_apply_as_set(&mut p, Some(&raw), HALT, ApplyAs::Literal, None, prov()).unwrap();
    assert!(find(&p, &first).is_none(), "one running ApplyAs per patch");
    let f = find(&p, &second).unwrap();
    assert_eq!(f.label, "Run Clear pathfinding on Halt as one replacement");
    assert_eq!(
        f.undo,
        UndoRecord::RestoreApplyAs {
            label: "Run it as separate edits".into(),
            mode: ApplyAs::Edits
        }
    );
    assert_eq!(modes(&raw, &p)[HALT].0, ApplyAs::Literal);
    assert!(later_on_same_patch(&p, &second).is_empty());
    assert!(undo_asks(&p, &f).is_empty());

    // Undo puts the edits back, under their label; undoing that leaves the author's way.
    undo(&mut p, Some(&raw), &second).unwrap();
    let back: Vec<&ProfileFix> = fixes(&p).collect();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].label, "Run it as separate edits");
    assert!(matches!(
        back[0].change,
        FixChange::ApplyAs {
            mode: ApplyAs::Edits,
            ..
        }
    ));
    assert_eq!(back[0].undo, UndoRecord::Remove);
    let back_id = back[0].id.clone();
    assert_eq!(modes(&raw, &p)[HALT], (ApplyAs::Edits, vec![back_id.clone()]));
    undo(&mut p, Some(&raw), &back_id).unwrap();
    assert_eq!(fixes(&p).count(), 0);
    assert_eq!(modes(&raw, &p)[HALT].0, ApplyAs::Literal);

    // A later fix on the same patch is listed for undo.
    let a = op_apply_as_set(&mut p, Some(&raw), HALT, ApplyAs::Edits, None, prov()).unwrap();
    let aliases = HashSet::from([HALT.to_string()]);
    let off = op_disable(&mut p, Some(&raw), HALT, &aliases, prov()).unwrap();
    assert_eq!(later_on_same_patch(&p, &a), [off]);
}

#[test]
fn setting_apply_as_keeps_a_paused_one_paused() {
    let (_t, raw) = at_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = || Provenance::new(FixSource::Resolver);
    let first = op_apply_as_set(&mut p, Some(&raw), HALT, ApplyAs::Edits, None, prov()).unwrap();
    find_mut(&mut p, &first).unwrap().paused = true;
    let second =
        op_apply_as_set(&mut p, Some(&raw), HALT, ApplyAs::Edits, None, prov()).unwrap();
    assert!(find(&p, &first).unwrap().paused, "kept, paused");
    assert_eq!(find(&p, &second).unwrap().undo, UndoRecord::Remove);
    assert_eq!(
        running_clashes(&p, &find(&p, &first).unwrap()),
        std::slice::from_ref(&second)
    );
    undo(&mut p, Some(&raw), &second).unwrap();
    assert!(find(&p, &first).unwrap().paused);
    assert_eq!(modes(&raw, &p)[HALT].0, ApplyAs::Literal);
    // Another patch is another slot.
    let other =
        op_apply_as_set(&mut p, Some(&raw), "other:a", ApplyAs::Edits, None, prov()).unwrap();
    assert!(running_clashes(&p, &find(&p, &other).unwrap()).is_empty());
    // An unmigrated profile has no ledger: nothing is written.
    let mut old = old_profile(|_| {});
    let before = serde_json::to_string(&old).unwrap();
    let none = op_apply_as_set(&mut old, Some(&raw), HALT, ApplyAs::Edits, None, prov());
    assert!(none.is_none());
    assert_eq!(serde_json::to_string(&old).unwrap(), before);
}

#[test]
fn an_apply_as_fix_follows_a_renamed_patch_and_rebinds() {
    let (t, raw) = at_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = Provenance::new(FixSource::Resolver);
    let id = op_apply_as_set(&mut p, Some(&raw), HALT, ApplyAs::Edits, None, prov).unwrap();

    // The author renames the patch: the plan follows it by its content (Rebound).
    write(t.path(), "gcore/qm.toml", &GCORE_AT.replace(HALT, "gcore:halt"));
    let raw2 = plan::load_raw(t.path(), None).unwrap();
    let plan = compile_plan(&p, vec!["gcore".into()], HashMap::new());
    let a = applied_for(&raw2, &plan).unwrap();
    let b = a.report.bindings.iter().find(|b| b.fix_id == id).unwrap();
    assert!(
        matches!(&b.status, qm_core::handshake::BindingStatus::Rebound { to, .. } if to == "gcore:halt"),
        "{b:?}"
    );
    assert_eq!(
        modes(&raw2, &p)["gcore:halt"],
        (ApplyAs::Edits, vec![id.clone()])
    );

    // Accept renames the fix.
    assert!(accept(&mut p, &raw2, &id));
    let f = find(&p, &id).unwrap();
    assert_eq!(f.subject(), Some("gcore:halt"));
    assert!(f.snapshots.contains_key("gcore:halt"));
    let plan = compile_plan(&p, vec!["gcore".into()], HashMap::new());
    assert!(plan.apply_as.contains_key("gcore:halt") && !plan.apply_as.contains_key(HALT));

    // Rebind points it at another patch.
    rebind(&mut p, &raw2, &id, "other:a").unwrap();
    let f = find(&p, &id).unwrap();
    assert_eq!(f.subject(), Some("other:a"));
    assert_eq!(f.refs()[0].mod_id, "other");
    let m = modes(&raw2, &p);
    assert_eq!(m["other:a"], (ApplyAs::Edits, vec![id.clone()]));
    assert_eq!(m["gcore:halt"].0, ApplyAs::Literal);
}

/// A launcher from before ApplyAs keeps the fix as it is and writes it back unchanged.
#[test]
fn an_old_reader_keeps_an_apply_as_fix() {
    /// The fix kinds an older launcher knows.
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum OldChange {
        Disable { patch: PatchRef },
    }
    #[derive(Debug, Serialize, Deserialize)]
    struct OldFix {
        id: String,
        label: String,
        change: OldChange,
        provenance: Provenance,
    }
    let (_t, raw) = at_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = Provenance::new(FixSource::Resolver);
    op_apply_as_set(&mut p, Some(&raw), HALT, ApplyAs::Edits, None, prov).unwrap();
    let stored = serde_json::to_value(&p.fixes).unwrap();
    let old: Vec<Lenient<OldFix>> = serde_json::from_value(stored.clone()).unwrap();
    assert!(old[0].error().is_some(), "an unknown kind");
    assert_eq!(serde_json::to_value(&old).unwrap(), stored, "written back as read");

    // This launcher keeps a mode it does not know the same way.
    let mut later = stored.clone();
    later[0]["change"]["mode"] = serde_json::json!("hunks");
    let read: Vec<Lenient<ProfileFix>> = serde_json::from_value(later.clone()).unwrap();
    assert!(read[0].error().is_some());
    assert_eq!(serde_json::to_value(&read).unwrap(), later);
    let mut q = p.clone();
    q.fixes = read;
    assert!(compile_plan(&q, Vec::new(), HashMap::new())
        .apply_as
        .is_empty());
}

// ---- SkipHunk fixes (D49, Task 21) ----

/// Two mods whose `at` patches both change `b = 2`.
const AMOD_AT: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"amod:a\"\nname = \"Armor tweak\"\ntarget = \"main.lua\"\npattern = \"a = 1\\nb = 2\"\nposition = \"at\"\npayload = \"a = 1\\nb = 20\"\n";
const BMOD_AT: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"bmod:b\"\nname = \"Range tweak\"\ntarget = \"main.lua\"\npattern = \"b = 2\\nc = 3\"\nposition = \"at\"\npayload = \"b = 30\\nc = 3\\ny = 0\"\n";

fn hunk_mods() -> (TempDir, RawLoad) {
    let t = TempDir::new().unwrap();
    write(
        t.path(),
        "amod/mod.json",
        r#"{"id": "amod", "name": "AMod", "version": "1.0.0"}"#,
    );
    write(t.path(), "amod/qm.toml", AMOD_AT);
    write(
        t.path(),
        "bmod/mod.json",
        r#"{"id": "bmod", "name": "BMod", "version": "1.0.0"}"#,
    );
    write(t.path(), "bmod/qm.toml", BMOD_AT);
    let raw = plan::load_raw(t.path(), None).unwrap();
    (t, raw)
}

/// Per pattern patch of `p`'s plan: (its skipped edits' ordinals, its mode, its fix ids).
fn skips(raw: &RawLoad, p: &ModProfile) -> BTreeMap<String, (Vec<u32>, ApplyAs, Vec<String>)> {
    let plan = compile_plan(p, vec!["amod".into(), "bmod".into()], HashMap::new());
    let a = applied_for(raw, &plan).unwrap();
    a.patches
        .iter()
        .filter_map(|(p, _, _)| match p {
            qm_core::patch::Patch::Pattern(x) => Some((
                x.id.clone().unwrap_or_default(),
                (
                    x.qm.skip_hunks.iter().map(|s| s.ordinal).collect(),
                    x.asked_apply_as(),
                    x.qm.fix_ids.clone(),
                ),
            )),
            _ => None,
        })
        .collect()
}

fn keep_a(p: &mut ModProfile, raw: &RawLoad) -> String {
    let key = hunk_key(raw, "bmod:b", 0, Some("amod:a".into())).unwrap();
    let prov = Provenance::new(FixSource::Resolver);
    op_hunk_skip(p, Some(raw), "bmod:b", key, None, prov).unwrap()
}

#[test]
fn a_skip_hunk_fix_is_stored_compiled_and_round_trips() {
    let (_t, raw) = hunk_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let id = keep_a(&mut p, &raw);
    let f = find(&p, &id).unwrap();
    assert_eq!(f.kind(), "skip_hunk");
    assert_eq!(f.label, "Keep Armor tweak's change to b = 2");
    assert_eq!(f.subject(), Some("bmod:b"));
    let FixChange::SkipHunk { patch, hunk } = &f.change else {
        panic!("{:?}", f.change)
    };
    assert!(patch.fp.is_some(), "pinned");
    assert_eq!(
        (hunk.vanilla.as_str(), hunk.ordinal, hunk.keep.as_deref()),
        ("b = 2", 0, Some("amod:a"))
    );
    assert!(!has_legacy_entries(&p));

    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains(r#""kind":"skip_hunk""#), "{json}");
    let back: ModProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&back).unwrap(), json);

    // The plan skips the edit, runs the patch as edits and names the fix.
    let plan = compile_plan(&p, vec!["amod".into(), "bmod".into()], HashMap::new());
    assert_eq!(plan.skip_hunks.len(), 1);
    assert!(plan.apply_as.is_empty(), "the bound skip turns edits mode on");
    let s = skips(&raw, &p);
    assert_eq!(s["bmod:b"], (vec![0], ApplyAs::Edits, vec![id.clone()]));
    assert_eq!(s["amod:a"], (Vec::new(), ApplyAs::Literal, Vec::new()));
    let a = applied_for(&raw, &plan).unwrap();
    let b = a
        .report
        .bindings
        .iter()
        .find(|b| b.key.starts_with("hunk:"))
        .unwrap();
    assert_eq!((b.key.as_str(), b.fix_id.as_str()), ("hunk:bmod:b:0", id.as_str()));
    assert_eq!(b.status, qm_core::handshake::BindingStatus::Bound);

    // Paused: nothing is skipped and the patch runs as its author wrote it.
    find_mut(&mut p, &id).unwrap().paused = true;
    sync_legacy(&mut p);
    let plan = compile_plan(&p, vec!["amod".into()], HashMap::new());
    assert!(plan.skip_hunks.is_empty() && plan.apply_as.is_empty());
    assert_eq!(
        skips(&raw, &p)["bmod:b"],
        (Vec::new(), ApplyAs::Literal, Vec::new())
    );
}

/// Keep B replaces Keep A (they exclude each other), and undo brings Keep A back.
#[test]
fn keep_b_replaces_keep_a_and_undo_brings_it_back() {
    let (_t, raw) = hunk_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let first = keep_a(&mut p, &raw);
    let key = hunk_key(&raw, "amod:a", 0, Some("bmod:b".into())).unwrap();
    let prov = Provenance::new(FixSource::Resolver);
    let second = op_hunk_skip(&mut p, Some(&raw), "amod:a", key, None, prov).unwrap();
    assert!(find(&p, &first).is_none());
    let f = find(&p, &second).unwrap();
    assert_eq!(f.label, "Keep Range tweak's change to b = 2");
    assert!(matches!(&f.undo, UndoRecord::RestoreHunk { patch, .. } if patch.id == "bmod:b"));
    let s = skips(&raw, &p);
    assert_eq!(
        (s["amod:a"].0.clone(), s["bmod:b"].0.clone()),
        (vec![0], Vec::new())
    );
    assert!(undo_asks(&p, &f).is_empty());

    undo(&mut p, Some(&raw), &second).unwrap();
    let back: Vec<&ProfileFix> = fixes(&p).collect();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].label, "Keep Armor tweak's change to b = 2");
    let back_id = back[0].id.clone();
    assert_eq!(skips(&raw, &p)["bmod:b"].0, vec![0]);
    undo(&mut p, Some(&raw), &back_id).unwrap();
    assert_eq!(fixes(&p).count(), 0);

    // The same edit again replaces the running fix; a paused one stays paused.
    let a = keep_a(&mut p, &raw);
    find_mut(&mut p, &a).unwrap().paused = true;
    let b = keep_a(&mut p, &raw);
    assert!(find(&p, &a).unwrap().paused);
    assert_eq!(find(&p, &b).unwrap().undo, UndoRecord::Remove);
    assert_eq!(running_clashes(&p, &find(&p, &a).unwrap()), [b]);
}

/// A skip of A's edit and Keep A on B both run; Keep B replaces both, and undo puts both back.
#[test]
fn undo_puts_back_every_skipped_edit_a_new_choice_replaced() {
    let (_t, raw) = hunk_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = || Provenance::new(FixSource::Resolver);
    let key = hunk_key(&raw, "amod:a", 0, None).unwrap();
    let skip_a = op_hunk_skip(&mut p, Some(&raw), "amod:a", key, None, prov()).unwrap();
    let keep = keep_a(&mut p, &raw);
    assert!(find(&p, &skip_a).is_some() && find(&p, &keep).is_some(), "both run");

    let key = hunk_key(&raw, "amod:a", 0, Some("bmod:b".into())).unwrap();
    let keep_b = op_hunk_skip(&mut p, Some(&raw), "amod:a", key, None, prov()).unwrap();
    assert!(find(&p, &skip_a).is_none() && find(&p, &keep).is_none());

    undo(&mut p, Some(&raw), &keep_b).unwrap();
    let mut back: Vec<(String, Option<String>)> = fixes(&p)
        .filter_map(|f| match &f.change {
            FixChange::SkipHunk { patch, hunk } => Some((patch.id.clone(), hunk.keep.clone())),
            _ => None,
        })
        .collect();
    back.sort();
    assert_eq!(
        back,
        [
            ("amod:a".to_string(), None),
            ("bmod:b".to_string(), Some("amod:a".to_string()))
        ]
    );
}

/// Both patches change two lines: `a = 1` and `c = 3`.
fn two_line_mods() -> (TempDir, RawLoad) {
    let t = TempDir::new().unwrap();
    let at = |id: &str, name: &str, a: &str, c: &str| {
        format!(
            "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"{id}\"\nname = \"{name}\"\ntarget = \"main.lua\"\npattern = \"a = 1\\nb = 2\\nc = 3\"\nposition = \"at\"\npayload = \"a = {a}\\nb = 2\\nc = {c}\"\n"
        )
    };
    for (m, id, name, a, c) in [
        ("amod", "amod:a", "Armor tweak", "10", "30"),
        ("bmod", "bmod:b", "Range tweak", "11", "31"),
    ] {
        write(
            t.path(),
            &format!("{m}/mod.json"),
            &format!(r#"{{"id": "{m}", "name": "{m}", "version": "1.0.0"}}"#),
        );
        write(t.path(), &format!("{m}/qm.toml"), &at(id, name, a, c));
    }
    let raw = plan::load_raw(t.path(), None).unwrap();
    (t, raw)
}

/// Keep A on one shared line and Keep B on another fit together: the second choice does not
/// replace the first.
#[test]
fn keep_a_on_one_line_and_keep_b_on_another_both_stay() {
    let (_t, raw) = two_line_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = || Provenance::new(FixSource::Resolver);
    // Keep A's `a = 10`: B skips its edit to `a = 1`.
    let key = hunk_key(&raw, "bmod:b", 0, Some("amod:a".into())).unwrap();
    assert_eq!(key.vanilla, "a = 1");
    let first = op_hunk_skip(&mut p, Some(&raw), "bmod:b", key, None, prov()).unwrap();
    // Keep B's `c = 31`: A skips its edit to `c = 3`.
    let key = hunk_key(&raw, "amod:a", 1, Some("bmod:b".into())).unwrap();
    assert_eq!(key.vanilla, "c = 3");
    let second = op_hunk_skip(&mut p, Some(&raw), "amod:a", key, None, prov()).unwrap();
    assert!(find(&p, &first).is_some(), "the line-a choice stays");
    assert_eq!(find(&p, &second).unwrap().undo, UndoRecord::Remove);
    let s = skips(&raw, &p);
    assert_eq!((s["amod:a"].0.clone(), s["bmod:b"].0.clone()), (vec![1], vec![0]));
    assert!(running_clashes(&p, &find(&p, &first).unwrap()).is_empty());

    // Keep B on line a replaces Keep A there, and only there.
    let key = hunk_key(&raw, "amod:a", 0, Some("bmod:b".into())).unwrap();
    let third = op_hunk_skip(&mut p, Some(&raw), "amod:a", key, None, prov()).unwrap();
    assert!(find(&p, &first).is_none());
    assert!(find(&p, &second).is_some());
    assert!(matches!(&find(&p, &third).unwrap().undo, UndoRecord::RestoreHunk { patch, .. } if patch.id == "bmod:b"));
}

/// A paused Keep A does not block Keep B on the same line; resuming Keep A then clashes with
/// it, so both never run.
#[test]
fn resuming_keep_a_clashes_with_a_running_keep_b_on_the_same_line() {
    let (_t, raw) = two_line_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = || Provenance::new(FixSource::Resolver);
    let key = hunk_key(&raw, "bmod:b", 0, Some("amod:a".into())).unwrap();
    let keep_a = op_hunk_skip(&mut p, Some(&raw), "bmod:b", key, None, prov()).unwrap();
    find_mut(&mut p, &keep_a).unwrap().paused = true;
    let key = hunk_key(&raw, "amod:a", 0, Some("bmod:b".into())).unwrap();
    let keep_b = op_hunk_skip(&mut p, Some(&raw), "amod:a", key, None, prov()).unwrap();
    assert!(find(&p, &keep_a).unwrap().paused, "a paused choice stays");
    let a = find(&p, &keep_a).unwrap().clone();
    assert_eq!(running_clashes(&p, &a), std::slice::from_ref(&keep_b));
    // Keep B on the other line does not clash.
    let b = find(&p, &keep_b).unwrap().clone();
    find_mut(&mut p, &keep_b).unwrap().paused = true;
    let key = hunk_key(&raw, "amod:a", 1, Some("bmod:b".into())).unwrap();
    op_hunk_skip(&mut p, Some(&raw), "amod:a", key, None, prov()).unwrap();
    assert!(running_clashes(&p, &a).is_empty());
    assert!(running_clashes(&p, &b).is_empty());
}

/// A skip runs the patch as edits even over an ApplyAs fix that runs it as one replacement.
#[test]
fn a_skip_hunk_fix_adds_edits_mode() {
    let (_t, raw) = hunk_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let prov = Provenance::new(FixSource::Resolver);
    op_apply_as_set(&mut p, Some(&raw), "bmod:b", ApplyAs::Literal, None, prov).unwrap();
    assert_eq!(skips(&raw, &p)["bmod:b"].1, ApplyAs::Literal);
    keep_a(&mut p, &raw);
    assert_eq!(skips(&raw, &p)["bmod:b"].1, ApplyAs::Edits);
}

/// The author changes the patch: the key no longer fits, nothing is skipped and the fix needs
/// review. Accept pins it to the patch as it is now. A rename is followed, and rebind moves it.
#[test]
fn a_skip_hunk_fix_follows_renames_and_drift_needs_review() {
    let (t, raw) = hunk_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let id = keep_a(&mut p, &raw);

    // The payload changed below the skipped line.
    write(t.path(), "bmod/qm.toml", &BMOD_AT.replace("y = 0", "y = 9"));
    let raw2 = plan::load_raw(t.path(), None).unwrap();
    let plan = compile_plan(&p, vec!["amod".into(), "bmod".into()], HashMap::new());
    let a = applied_for(&raw2, &plan).unwrap();
    let b = a.report.bindings.iter().find(|b| b.fix_id == id).unwrap();
    assert!(
        matches!(b.status, qm_core::handshake::BindingStatus::Drifted { .. }),
        "{b:?}"
    );
    assert_eq!(skips(&raw2, &p)["bmod:b"].0, Vec::<u32>::new());
    assert!(accept(&mut p, &raw2, &id));
    assert_eq!(skips(&raw2, &p)["bmod:b"].0, vec![0]);

    // Renamed: the plan follows it, and accept renames the fix.
    write(
        t.path(),
        "bmod/qm.toml",
        &BMOD_AT
            .replace("y = 0", "y = 9")
            .replace("bmod:b", "bmod:renamed"),
    );
    let raw3 = plan::load_raw(t.path(), None).unwrap();
    assert_eq!(skips(&raw3, &p)["bmod:renamed"].0, vec![0]);
    assert!(accept(&mut p, &raw3, &id));
    assert_eq!(find(&p, &id).unwrap().subject(), Some("bmod:renamed"));

    // The kept patch is renamed too: the key follows it.
    let mut q = p.clone();
    for f in fixes_mut(&mut q) {
        rename(&mut f.change, "amod:a", "amod:z");
    }
    let FixChange::SkipHunk { hunk, .. } = &find(&q, &id).unwrap().change else {
        panic!("a skip");
    };
    assert_eq!(hunk.keep.as_deref(), Some("amod:z"));

    // Rebind to the other patch: its own content and edit.
    rebind(&mut p, &raw3, &id, "amod:a").unwrap();
    let FixChange::SkipHunk { patch, hunk } = &find(&p, &id).unwrap().change else {
        panic!("a skip");
    };
    assert_eq!(patch.id, "amod:a");
    let fp = Fingerprints::new(&raw3, FpMode::All).get(0).unwrap().lite();
    assert_eq!((&hunk.fp, hunk.vanilla.as_str()), (&fp, "b = 2"));
}

#[test]
fn a_hunk_key_needs_an_at_patch_and_an_edit() {
    let (_t, raw) = mods();
    assert!(hunk_key(&raw, "gcore:main_lua:16", 0, None)
        .unwrap_err()
        .contains("does not replace lines"));
    let (_t, raw) = hunk_mods();
    assert!(hunk_key(&raw, "bmod:b", 5, None)
        .unwrap_err()
        .contains("no change number 6"));
    assert!(hunk_key(&raw, "zmod:x", 0, None)
        .unwrap_err()
        .contains("No loaded patch"));
}

/// A launcher from before SkipHunk keeps the fix as it is and writes it back unchanged.
#[test]
fn an_old_reader_keeps_a_skip_hunk_fix() {
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum OldChange {
        Disable { patch: PatchRef },
    }
    #[derive(Debug, Serialize, Deserialize)]
    struct OldFix {
        id: String,
        label: String,
        change: OldChange,
        provenance: Provenance,
    }
    let (_t, raw) = hunk_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    keep_a(&mut p, &raw);
    let stored = serde_json::to_value(&p.fixes).unwrap();
    let old: Vec<Lenient<OldFix>> = serde_json::from_value(stored.clone()).unwrap();
    assert!(old[0].error().is_some(), "an unknown kind");
    assert_eq!(serde_json::to_value(&old).unwrap(), stored, "written back as read");
}

// ---- StackRule fixes (D55, Task 22) ----

/// Two mods that both add a line after `x = 1`.
const GCORE_AFTER: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"gcore:main_lua:3\"\nname = \"Core setup\"\ntarget = \"main.lua\"\npattern = \"x = 1\"\nposition = \"after\"\npayload = \"gcore = 1\"\n";
const TO_AFTER: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = \"to:main_lua:2\"\nname = \"Tactics setup\"\ntarget = \"main.lua\"\npattern = \"x = 1\"\nposition = \"after\"\npayload = \"to = 1\"\n";

const CORE: &str = "gcore:main_lua:3";
const TACT: &str = "to:main_lua:2";

fn stack_mods() -> (TempDir, RawLoad) {
    let t = TempDir::new().unwrap();
    write(
        t.path(),
        "gcore/mod.json",
        r#"{"id": "gcore", "name": "GCore", "version": "1.0.0"}"#,
    );
    write(t.path(), "gcore/qm.toml", GCORE_AFTER);
    write(
        t.path(),
        "to/mod.json",
        r#"{"id": "to", "name": "Tactical Overhaul", "version": "2.0.0"}"#,
    );
    write(t.path(), "to/qm.toml", TO_AFTER);
    let raw = plan::load_raw(t.path(), None).unwrap();
    (t, raw)
}

fn line(n: u32) -> SiteKey {
    SiteKey {
        context: format!("ctx{n}"),
        ordinal: Some(1),
    }
}

type Specs = BTreeMap<String, (Vec<qm_core::patch::stack::StackRuleSpec>, Vec<String>)>;

/// The stack rules each patch carries in `p`'s plan, with its fix ids.
fn stack_specs(raw: &RawLoad, p: &ModProfile) -> Specs {
    let plan = compile_plan(p, vec!["gcore".into(), "to".into()], HashMap::new());
    applied_for(raw, &plan)
        .unwrap()
        .patches
        .iter()
        .map(|(p, _, _)| {
            (
                p.id().unwrap_or_default().to_string(),
                (p.extra().stack_rules.clone(), p.extra().fix_ids.clone()),
            )
        })
        .collect()
}

/// A rule putting GCore's block below or above TO's after line `site`.
fn set_rule(
    p: &mut ModProfile,
    raw: &RawLoad,
    below: bool,
    site: SiteKey,
    side: Side,
) -> Option<String> {
    let (b, a) = if below { (Some(TACT), None) } else { (None, Some(TACT)) };
    op_stack_set(
        p,
        Some(raw),
        CORE,
        site,
        side,
        b,
        a,
        None,
        Provenance::new(FixSource::OrderView),
    )
}

#[test]
fn a_stack_rule_fix_is_stored_compiled_and_round_trips() {
    let (_t, raw) = stack_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let id = set_rule(&mut p, &raw, true, line(1), Side::After).unwrap();
    let f = find(&p, &id).unwrap();
    assert_eq!(f.kind(), "stack_rule");
    assert_eq!(f.label, "Put Core setup's block below Tactics setup's");
    assert_eq!(f.subject(), Some(CORE));
    assert_eq!(f.undo, UndoRecord::Remove);
    let refs = f.refs();
    assert_eq!(refs.len(), 2);
    assert!(refs.iter().all(|r| r.fp.is_some()), "both pinned");
    assert!(f.snapshots.contains_key(CORE) && f.snapshots.contains_key(TACT));
    let mods: Vec<&str> = f.provenance.mods.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(mods, ["gcore", "to"]);
    assert!(!has_legacy_entries(&p));
    assert_eq!(
        stack_label(Some(&raw), CORE, None, Some(refs[1]), Some(" x = 1 ")),
        "Put Core setup's block above Tactics setup's at x = 1"
    );

    // profile.json round trip.
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains(r#""kind":"stack_rule""#), "{json}");
    let back: ModProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&back).unwrap(), json);

    // The plan carries the rule with its fix id; the patch gets it with both bindings bound.
    let plan = compile_plan(&p, vec!["gcore".into(), "to".into()], HashMap::new());
    assert_eq!(plan.stack_rules.len(), 1);
    let r = plan.stack_rules[0].ok().unwrap();
    assert_eq!((r.fix_id.as_str(), r.side), (id.as_str(), Side::After));
    assert_eq!(r.below.as_ref().map(|b| b.id.as_str()), Some(TACT));
    let specs = stack_specs(&raw, &p);
    let (rules, fix_ids) = &specs[CORE];
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].site, Some(line(1)));
    assert!(rules[0].below.is_some() && rules[0].above.is_none());
    assert_eq!(fix_ids, &vec![id.clone()]);
    assert!(specs[TACT].0.is_empty(), "the other patch gets it in prepare");
    let a = applied_for(&raw, &plan).unwrap();
    let b: Vec<_> = a.report.bindings.iter().filter(|b| b.fix_id == id).collect();
    assert_eq!(b.len(), 2);
    assert!(b
        .iter()
        .all(|b| b.status == qm_core::handshake::BindingStatus::Bound));
}

/// Moving the other block at the same line replaces the rule held on this one, and undo puts
/// it back on its own patch.
#[test]
fn an_opposite_stack_rule_replaces_the_older_one() {
    let (_t, raw) = stack_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    // GCore below TO, held on GCore.
    let first = set_rule(&mut p, &raw, true, line(1), Side::After).unwrap();
    // Then TO below GCore, held on TO.
    let prov = Provenance::new(FixSource::OrderView);
    let second = op_stack_set(
        &mut p,
        Some(&raw),
        TACT,
        line(1),
        Side::After,
        Some(CORE),
        None,
        None,
        prov,
    )
    .unwrap();
    assert!(find(&p, &first).is_none(), "the older choice is replaced");
    let f = find(&p, &second).unwrap();
    assert!(
        matches!(&f.undo, UndoRecord::RestoreStack { patch: Some(o), below: Some(b), .. } if o.id == CORE && b.id == TACT),
        "{:?}",
        f.undo
    );
    assert_eq!(compile_plan(&p, Vec::new(), HashMap::new()).stack_rules.len(), 1);
    undo(&mut p, Some(&raw), &second).unwrap();
    let back: Vec<&ProfileFix> = fixes(&p).collect();
    assert_eq!(back.len(), 1);
    assert!(
        matches!(&back[0].change, FixChange::StackRule { patch, below: Some(b), .. } if patch.id == CORE && b.id == TACT),
        "{:?}",
        back[0].change
    );
    // A paused rule is kept; resuming it would clash with the other block's rule.
    let restored = back[0].id.clone();
    find_mut(&mut p, &restored).unwrap().paused = true;
    let prov = Provenance::new(FixSource::OrderView);
    let newer = op_stack_set(&mut p, Some(&raw), TACT, line(1), Side::After, Some(CORE), None, None, prov)
        .unwrap();
    let paused = find(&p, &restored).unwrap().clone();
    assert_eq!(running_clashes(&p, &paused), std::slice::from_ref(&newer));
    undo(&mut p, Some(&raw), &newer).unwrap();
    find_mut(&mut p, &restored).unwrap().paused = false;

    // A rule of the other block at another line stays.
    let other_line = set_rule(&mut p, &raw, true, line(2), Side::After).unwrap();
    let prov = Provenance::new(FixSource::OrderView);
    op_stack_set(&mut p, Some(&raw), TACT, line(1), Side::After, Some(CORE), None, None, prov)
        .unwrap();
    assert!(find(&p, &other_line).is_some());
}

/// Three blocks at one line: a new rule replaces both the same-slot rule and the other
/// block's rule that names this patch, and undo puts both back.
#[test]
fn undo_puts_back_every_stack_rule_a_new_rule_replaced() {
    let (_t, raw) = stack_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    const THIRD: &str = "third:c";
    let rule = |p: &mut ModProfile, id: &str, below: &str| {
        let prov = Provenance::new(FixSource::OrderView);
        op_stack_set(p, Some(&raw), id, line(1), Side::After, Some(below), None, None, prov)
            .unwrap()
    };
    let r1 = rule(&mut p, CORE, TACT);
    let r3 = rule(&mut p, THIRD, CORE);
    assert!(find(&p, &r1).is_some() && find(&p, &r3).is_some(), "both run");

    let new = rule(&mut p, CORE, THIRD);
    assert!(find(&p, &r1).is_none() && find(&p, &r3).is_none());
    // profile.json round trip keeps both records.
    let json = serde_json::to_string(&p).unwrap();
    let back: ModProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
    p = back;

    undo(&mut p, Some(&raw), &new).unwrap();
    let mut rules: Vec<(String, String)> = fixes(&p)
        .filter_map(|f| match &f.change {
            FixChange::StackRule {
                patch,
                below: Some(b),
                ..
            } => Some((patch.id.clone(), b.id.clone())),
            _ => None,
        })
        .collect();
    rules.sort();
    assert_eq!(
        rules,
        [
            (CORE.to_string(), TACT.to_string()),
            (THIRD.to_string(), CORE.to_string())
        ]
    );
}

#[test]
fn a_stack_rule_fix_pauses_replaces_and_undoes() {
    let (_t, raw) = stack_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let first = set_rule(&mut p, &raw, true, line(1), Side::After).unwrap();
    // Paused: not compiled.
    find_mut(&mut p, &first).unwrap().paused = true;
    assert!(compile_plan(&p, Vec::new(), HashMap::new())
        .stack_rules
        .is_empty());
    find_mut(&mut p, &first).unwrap().paused = false;

    // The same patch, line and side: replaced, and undo brings the first back.
    let second = set_rule(&mut p, &raw, false, line(1), Side::After).unwrap();
    assert!(find(&p, &first).is_none());
    let f = find(&p, &second).unwrap();
    assert!(
        matches!(&f.undo, UndoRecord::RestoreStack { below: Some(b), above: None, .. } if b.id == TACT),
        "{:?}",
        f.undo
    );
    undo(&mut p, Some(&raw), &second).unwrap();
    let back: Vec<&ProfileFix> = fixes(&p).collect();
    assert_eq!(back.len(), 1);
    assert!(
        matches!(&back[0].change, FixChange::StackRule { below: Some(b), .. } if b.id == TACT)
    );
    let back_id = back[0].id.clone();

    // Another line or side is another slot.
    let other_line = set_rule(&mut p, &raw, true, line(2), Side::After).unwrap();
    let other_side = set_rule(&mut p, &raw, true, line(1), Side::Before).unwrap();
    for id in [&other_line, &other_side] {
        assert!(running_clashes(&p, &find(&p, id).unwrap()).is_empty());
    }
    assert!(find(&p, &back_id).is_some());
    // A paused one stays paused when a new rule takes its slot.
    find_mut(&mut p, &back_id).unwrap().paused = true;
    let third = set_rule(&mut p, &raw, false, line(1), Side::After).unwrap();
    assert!(find(&p, &back_id).unwrap().paused, "kept, paused");
    assert_eq!(find(&p, &third).unwrap().undo, UndoRecord::Remove);
    assert_eq!(
        running_clashes(&p, &find(&p, &back_id).unwrap()),
        std::slice::from_ref(&third)
    );
    assert_eq!(
        compile_plan(&p, Vec::new(), HashMap::new())
            .stack_rules
            .len(),
        3
    );

    // An unmigrated profile has no ledger: nothing is written.
    let mut old = old_profile(|_| {});
    let before = serde_json::to_string(&old).unwrap();
    assert!(set_rule(&mut old, &raw, true, line(1), Side::After).is_none());
    assert_eq!(serde_json::to_string(&old).unwrap(), before);
}

#[test]
fn a_stack_rule_fix_follows_renames_and_rebinds() {
    let (t, raw) = stack_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    let id = set_rule(&mut p, &raw, true, line(1), Side::After).unwrap();

    // The other mod's author renames its patch: the rule follows it by its content.
    write(t.path(), "to/qm.toml", &TO_AFTER.replace(TACT, "to:setup"));
    let raw2 = plan::load_raw(t.path(), None).unwrap();
    let plan = compile_plan(&p, vec!["gcore".into(), "to".into()], HashMap::new());
    let a = applied_for(&raw2, &plan).unwrap();
    let b = a
        .report
        .bindings
        .iter()
        .find(|b| b.fix_id == id && b.key == format!("stack:{TACT}"))
        .unwrap();
    assert!(
        matches!(&b.status, qm_core::handshake::BindingStatus::Rebound { to, .. } if to == "to:setup"),
        "{b:?}"
    );
    let specs = stack_specs(&raw2, &p);
    assert_eq!(specs[CORE].0[0].below.as_deref(), Some("to:setup"));

    // Accept renames the ref.
    assert!(accept(&mut p, &raw2, &id));
    let f = find(&p, &id).unwrap();
    let FixChange::StackRule { below, .. } = &f.change else {
        panic!("{:?}", f.change)
    };
    assert_eq!(below.as_ref().map(|b| b.id.as_str()), Some("to:setup"));
    assert!(f.snapshots.contains_key("to:setup"));

    // Rebind points the rule's own patch at another one.
    rebind(&mut p, &raw2, &id, "to:setup").unwrap();
    assert_eq!(find(&p, &id).unwrap().subject(), Some("to:setup"));
}

/// A launcher from before StackRule keeps the fix as it is and writes it back unchanged.
#[test]
fn an_old_reader_keeps_a_stack_rule_fix() {
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum OldChange {
        Disable { patch: PatchRef },
    }
    #[derive(Debug, Serialize, Deserialize)]
    struct OldFix {
        id: String,
        label: String,
        change: OldChange,
        provenance: Provenance,
    }
    let (_t, raw) = stack_mods();
    let mut p = migrate(&old_profile(|_| {}), &raw, None).unwrap();
    set_rule(&mut p, &raw, true, line(1), Side::After).unwrap();
    let stored = serde_json::to_value(&p.fixes).unwrap();
    let old: Vec<Lenient<OldFix>> = serde_json::from_value(stored.clone()).unwrap();
    assert!(old[0].error().is_some(), "an unknown kind");
    assert_eq!(serde_json::to_value(&old).unwrap(), stored, "written back as read");
    // A side this launcher does not know is kept the same way.
    let mut later = stored.clone();
    later[0]["change"]["side"] = serde_json::json!("inside");
    let read: Vec<Lenient<ProfileFix>> = serde_json::from_value(later.clone()).unwrap();
    assert!(read[0].error().is_some());
    assert_eq!(serde_json::to_value(&read).unwrap(), later);
    let mut q = p.clone();
    q.fixes = read;
    assert!(compile_plan(&q, Vec::new(), HashMap::new())
        .stack_rules
        .is_empty());
}
