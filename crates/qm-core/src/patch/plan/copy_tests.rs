//! Adjusted copies that run as their base mod (D58), exact copies and copies of copies (D59).

use super::*;
use crate::handshake::{PatchOutcomeStatus, RunAs};
use crate::patch::table::PatchTable;
use std::fs;
use tempfile::TempDir;

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap_or(dir)).unwrap();
    fs::write(p, text).unwrap();
}

/// Mod folder `id` with a mod.json that requires `requires` and the qm.toml `toml`.
fn add_mod(dir: &Path, id: &str, requires: &[&str], toml: &str) {
    let req: serde_json::Map<String, serde_json::Value> = requires
        .iter()
        .map(|r| (r.to_string(), serde_json::json!(">=1.0.0")))
        .collect();
    let manifest = serde_json::json!({
        "schema_version": 1, "id": id, "name": id, "version": "1.0.0",
        "entry_patch": "qm.toml",
        "relationships": {"requires": req, "load_after": requires},
    });
    write(dir, &format!("{id}/mod.json"), &manifest.to_string());
    write(dir, &format!("{id}/qm.toml"), toml);
}

/// One pattern patch of main.lua, as qm.toml text.
fn pattern(id: &str, anchor: &str, position: &str, payload: &str, extra: &str) -> String {
    format!(
        "[[patches]]\n[patches.pattern]\nid = \"{id}\"\ntarget = \"main.lua\"\npattern = \"{anchor}\"\nposition = \"{position}\"\npayload = \"{payload}\"\n{extra}"
    )
}

fn toml(patches: &[String]) -> String {
    format!(
        "[manifest]\nversion = \"1.0.0\"\npriority = 50\n\n{}",
        patches.join("\n")
    )
}

/// An adjusted copy of `base` (core's), anchored after `x`.
fn copy(id: &str, base: &str, run_as: Option<RunAs>, exact: bool) -> GluePatchEntry {
    GluePatchEntry {
        id: id.to_string(),
        name: format!("Adjusted copy of {base}"),
        replaces_patch_id: Some(base.to_string()),
        priority: 50,
        patch: serde_json::from_value(serde_json::json!({"pattern": {"target": "main.lua",
            "pattern": "x", "position": "after", "payload": "-- copy"}}))
        .unwrap(),
        base_mod_id: Some("core".to_string()),
        run_as,
        exact,
    }
}

fn plan_with(enabled: &[&str], glues: Vec<GluePatchEntry>, extra: serde_json::Value) -> PlanLoad {
    let mut v = serde_json::json!({
        "profile_id": "p", "generated_at": "now", "enabled_mods": enabled,
        "glue_patches": glues,
        "fix_of": {"glue:glue:a": "fx_a", "glue:glue:b": "fx_b"},
    });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    let plan = LoadOrderPlan::parse(&v.to_string());
    assert!(plan.error.is_none() && plan.warnings.is_empty(), "{plan:?}");
    plan
}

fn table(dir: &Path, plan: &PlanLoad) -> PatchTable {
    let raw = load_raw(dir, plan.plan.as_ref()).unwrap();
    PatchTable::from_applied(apply_plan(&raw, plan), dir.to_path_buf())
}

fn ids(t: &PatchTable) -> Vec<(String, String)> {
    t.patches
        .iter()
        .map(|(p, _, _)| {
            let s = |x: Option<&str>| x.unwrap_or_default().to_string();
            (s(p.id()), s(p.mod_id()))
        })
        .collect()
}

/// Core: a patch that fails with `skip_mod` and the base `core:a`; other: a patch after `x`.
fn world(core_extra: &[String]) -> TempDir {
    let temp = TempDir::new().unwrap();
    let mut core = core_extra.to_vec();
    core.push(pattern("core:a", "gone", "after", "-- a", ""));
    add_mod(temp.path(), "core", &[], &toml(&core));
    let other = pattern("other:b", "x", "after", "-- other", "");
    add_mod(temp.path(), "other", &["core"], &toml(&[other]));
    temp
}

#[test]
fn a_base_copy_takes_its_base_s_place_as_its_mod() {
    let temp = world(&[]);
    let plan = plan_with(
        &["core", "other"],
        vec![copy("glue:a", "core:a", Some(RunAs::Base), false)],
        serde_json::json!({}),
    );
    let t = table(temp.path(), &plan);
    assert_eq!(
        ids(&t),
        [
            ("glue:a".into(), "core".into()),
            ("other:b".into(), "other".into())
        ]
    );
    let (p, _, path) = &t.patches[0];
    assert!(p.extra().adjusted_copy);
    assert_eq!(p.extra().replaces.as_deref(), Some("core:a"));
    assert_eq!(p.extra().fix_ids, ["fx_a"]);
    assert!(p.extra().exact_lines.is_none());
    assert!(
        path.starts_with(temp.path().join("core")) || path.starts_with("core"),
        "{path:?}"
    );

    // The record keeps the copy's id and fix next to the base mod (D58).
    let (text, _, recs) = t.apply_patches_offline("main.lua", "x\n").unwrap();
    assert_eq!(recs[0].patch_id, "glue:a");
    assert_eq!(recs[0].mod_id, "core");
    assert_eq!(recs[0].fix_ids, ["fx_a"]);
    // Legacy: the later patch goes on top, right below the line.
    assert_eq!(text, "x\n-- other\n-- copy\n");
    assert!(t.is_adjusted_copy("core", "glue:a"));
    assert!(!t.is_adjusted_copy("other", "other:b"));
}

#[test]
fn a_legacy_copy_still_runs_last_as_qm_glue() {
    for run_as in [None, Some(RunAs::Legacy)] {
        let temp = world(&[]);
        let plan = plan_with(
            &["core", "other"],
            vec![copy("glue:a", "core:a", run_as, false)],
            serde_json::json!({}),
        );
        let t = table(temp.path(), &plan);
        assert_eq!(
            ids(&t),
            [
                ("other:b".into(), "other".into()),
                ("glue:a".into(), "qm_glue".into())
            ]
        );
        assert!(t.patches[1].0.extra().adjusted_copy);
        let (text, _, _) = t.apply_patches_offline("main.lua", "x\n").unwrap();
        assert_eq!(text, "x\n-- copy\n-- other\n", "{run_as:?}");
    }
}

#[test]
fn a_base_copy_with_its_base_gone_runs_last_as_qm_glue() {
    let temp = world(&[]);
    let plan = plan_with(
        &["core", "other"],
        vec![copy("glue:a", "core:gone", Some(RunAs::Base), false)],
        serde_json::json!({}),
    );
    let t = table(temp.path(), &plan);
    assert_eq!(
        t.patches.last().map(|(p, _, _)| p.mod_id()),
        Some(Some("qm_glue"))
    );
}

#[test]
fn a_base_copy_is_skipped_with_its_base_mod() {
    let fails = pattern(
        "core:first",
        "nowhere",
        "after",
        "-- f",
        "on_failure = \"skip_mod\"\n",
    );
    let temp = world(&[fails]);
    for (run_as, skipped) in [(Some(RunAs::Base), true), (None, false)] {
        let plan = plan_with(
            &["core", "other"],
            vec![copy("glue:a", "core:a", run_as, false)],
            serde_json::json!({}),
        );
        let t = table(temp.path(), &plan);
        let (_, _, recs) = t.apply_patches_offline("main.lua", "x\n").unwrap();
        let rec = recs.iter().find(|r| r.patch_id == "glue:a").unwrap();
        let want = if skipped {
            PatchOutcomeStatus::SkippedByPolicy
        } else {
            PatchOutcomeStatus::Applied
        };
        assert_eq!(rec.status, want, "{run_as:?}");
    }
}

#[test]
fn a_base_copy_stacks_as_its_base_mod() {
    // Other needs core: in Dependencies its code goes below core's at the shared line. A
    // `Legacy` copy is `qm_glue`, which other does not need, so other's code stays on top.
    let temp = world(&[]);
    let deps = serde_json::json!({"stacking": "dependencies"});
    let base = plan_with(
        &["core", "other"],
        vec![copy("glue:a", "core:a", Some(RunAs::Base), false)],
        deps.clone(),
    );
    let t = table(temp.path(), &base);
    let (text, _, _) = t.apply_patches_offline("main.lua", "x\n").unwrap();
    assert_eq!(text, "x\n-- copy\n-- other\n");
}

#[test]
fn an_exact_copy_matches_only_its_own_text() {
    let temp = world(&[]);
    let mut c = copy("glue:a", "core:a", Some(RunAs::Base), true);
    if let Patch::Pattern(p) = &mut c.patch {
        p.pattern = "local a = x * y".to_string();
    }
    let mut wild = c.clone();
    wild.exact = false;
    let text = "local a = x * y\nlocal a = x + y\n";
    let run = |g: GluePatchEntry| {
        let plan = plan_with(&["core", "other"], vec![g], serde_json::json!({}));
        let t = table(temp.path(), &plan);
        let (_, _, recs) = t.apply_patches_offline("main.lua", text).unwrap();
        recs.iter()
            .find(|r| r.patch_id == "glue:a")
            .map(|r| r.matched_count)
            .unwrap()
    };
    assert_eq!(run(c.clone()), 1);
    assert_eq!(run(wild), 2);

    let plan = plan_with(&["core", "other"], vec![c], serde_json::json!({}));
    let t = table(temp.path(), &plan);
    let lines = t.patches[0].0.extra().exact_lines.clone();
    assert_eq!(lines, Some(vec!["local a = x * y".to_string()]));
}

#[test]
fn a_copy_of_a_copy_replaces_the_older_copy() {
    // An old copy of a copy names the older copy and `qm_glue`: it no longer waits for
    // `qm_glue`, the older copy stops, and the base stays replaced.
    let temp = world(&[]);
    let older = copy("glue:a", "core:a", None, false);
    let mut newer = copy("glue:b", "glue:a", None, false);
    newer.base_mod_id = Some("qm_glue".to_string());
    if let Patch::Pattern(p) = &mut newer.patch {
        p.payload = "-- newer".to_string();
    }
    let plan = plan_with(
        &["core", "other"],
        vec![older, newer],
        serde_json::json!({}),
    );
    let raw = load_raw(temp.path(), plan.plan.as_ref()).unwrap();
    let applied = apply_plan(&raw, &plan);
    assert!(
        applied.report.bindings.is_empty(),
        "{:?}",
        applied.report.bindings
    );
    let t = PatchTable::from_applied(applied, temp.path().to_path_buf());
    let got: Vec<String> = ids(&t).into_iter().map(|(id, _)| id).collect();
    assert_eq!(got, ["other:b", "glue:b"]);
    let (text, _, _) = t.apply_patches_offline("main.lua", "x\n").unwrap();
    assert_eq!(text, "x\n-- newer\n-- other\n");

    // While the first copy's base mod is off, the newer copy waits for it too.
    let older = copy("glue:a", "core:a", None, false);
    let mut newer = copy("glue:b", "glue:a", None, false);
    newer.base_mod_id = Some("qm_glue".to_string());
    let plan = plan_with(&["other"], vec![older, newer], serde_json::json!({}));
    let raw = load_raw(temp.path(), plan.plan.as_ref()).unwrap();
    let applied = apply_plan(&raw, &plan);
    let got: Vec<String> = applied
        .patches
        .iter()
        .map(|(p, _, _)| p.id().unwrap_or_default().to_string())
        .collect();
    assert_eq!(got, ["other:b"]);
    assert_eq!(applied.report.bindings.len(), 1);
    assert_eq!(applied.report.bindings[0].key, "glue:glue:b");
    assert!(matches!(
        applied.report.bindings[0].status,
        BindingStatus::Waiting { .. }
    ));
}

#[test]
fn a_compat_rule_never_names_an_adjusted_copy() {
    // A compat mod that requires core runs `core:*` after other, and names the copy by id.
    let temp = world(&[]);
    let compat = "[manifest]\nversion = \"1.0.0\"\n\n[[overrides]]\npatch = \"core:a\"\nafter = [\"other:*\"]\n\n[[overrides]]\npatch = \"core:glue:a\"\npriority = 1\n\n[[overrides]]\npatch = \"other:b\"\nbefore = [\"qm_glue:*\"]\n";
    add_mod(temp.path(), "fixpack", &["core", "other"], compat);
    let plan = plan_with(
        &["core", "other", "fixpack"],
        vec![copy("glue:a", "core:a", Some(RunAs::Base), false)],
        serde_json::json!({}),
    );
    let raw = load_raw(temp.path(), plan.plan.as_ref()).unwrap();
    let applied = apply_plan(&raw, &plan);
    let named = applied
        .report
        .overrides
        .iter()
        .find(|o| o.target == "core:glue:a")
        .expect("a report for the copy");
    assert_eq!(named.status, "invalid");
    assert!(named.detail.contains("adjusted copy"), "{}", named.detail);
    // A Legacy copy's mod, `qm_glue`, is still refused by the D12 gate.
    let glue_rule = applied
        .report
        .overrides
        .iter()
        .find(|o| o.target == "other:b")
        .expect("a report for the qm_glue rule");
    assert_eq!(
        glue_rule.status, "rejected_not_required",
        "{}",
        glue_rule.detail
    );
    // `core:a` is replaced by the profile's copy, so its compat rule is the profile's to
    // decide; a `core:*` rule does not reach the copy either.
    let t = PatchTable::from_applied(applied, temp.path().to_path_buf());
    let order = crate::patch::order::order_patches_with(
        &t.patches,
        &[],
        &[(
            OrderRule {
                patch: "core:*".into(),
                before: vec![],
                after: vec!["other:*".into()],
            },
            "fixpack".into(),
        )],
        &[],
    );
    let pos = |id: &str| {
        order
            .order
            .iter()
            .position(|&i| t.patches[i].0.id() == Some(id))
            .unwrap()
    };
    assert!(pos("glue:a") < pos("other:b"), "{:?}", order.order);
    // The same rule from the profile moves it.
    let order = crate::patch::order::order_patches(
        &t.patches,
        &[OrderRule {
            patch: "core:*".into(),
            before: vec![],
            after: vec!["other:*".into()],
        }],
    );
    let pos = |id: &str| {
        order
            .order
            .iter()
            .position(|&i| t.patches[i].0.id() == Some(id))
            .unwrap()
    };
    assert!(pos("glue:a") > pos("other:b"), "{:?}", order.order);
}
