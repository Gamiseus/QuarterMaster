#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use qm_service::testing::{FakeProbe, RecordingSink};
use qm_service::{ErrorKind, Loadable, ModPatchStatus, PatchState, Service};
use std::sync::Arc;
use std::time::Duration;

const CONSOLE_PATCH: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
target = "conf.lua"
pattern = "t.console = false"
position = "at"
payload = "t.console = true"
"#;

/// Rewrites the console line that `CONSOLE_PATCH` already rewrote, so the two overlap.
const FOLLOW_UP_PATCH: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
target = "conf.lua"
pattern = "t.console = true"
position = "at"
payload = "t.console = 1"
"#;

const MISSING_ANCHOR_PATCH: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
target = "conf.lua"
pattern = "no such line"
position = "at"
payload = "x"
"#;

const SKIP_PATCH_MISS: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
target = "conf.lua"
pattern = "no such line"
position = "at"
payload = "x"
on_failure = "skip_patch"
"#;

/// First patch misses under `skip_mod`; the second would apply but its mod is skipped.
const SKIP_MOD_MISS: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
name = "a-miss"
target = "conf.lua"
pattern = "no such line"
position = "at"
payload = "x"
on_failure = "skip_mod"

[[patches]]
[patches.pattern]
name = "b-follower"
target = "conf.lua"
pattern = "t.console = false"
position = "at"
payload = "t.console = true"
"#;

struct Ctx {
    fx: support::Fixture,
    svc: Service,
}

fn ctx() -> Ctx {
    let fx = support::Fixture::new();
    let svc = Service::new(
        fx.paths.clone(),
        Arc::new(RecordingSink::new()),
        Arc::new(FakeProbe::new()),
    );
    Ctx { fx, svc }
}

fn idle(svc: &Service) {
    assert!(svc.jobs().wait_idle(Duration::from_secs(30)));
}

fn warmed() -> Ctx {
    warmed_with(&[("probe", 50, CONSOLE_PATCH)], &[])
}

/// Install `(id, load_priority, qm.toml)` mods, enable them all, seed `disabled_patches`
/// and run every warm-up job.
fn warmed_with(mods: &[(&str, i32, &str)], disabled: &[&str]) -> Ctx {
    let c = ctx();
    for (id, prio, toml) in mods {
        c.fx.add_mod_with_priority(id, "1.0.0", toml, Some(*prio));
    }
    let mut def = c.fx.paths.get_profile("default").unwrap();
    def.enabled_mods = mods.iter().map(|(id, _, _)| id.to_string()).collect();
    def.disabled_patches = disabled.iter().map(|d| d.to_string()).collect();
    c.fx.paths.save_profile(&def).unwrap();
    c.svc.profile_activate("default").unwrap();
    c.svc.warm_mods();
    c.svc.warm_vanilla();
    idle(&c.svc);
    c
}

fn patches(svc: &Service, id: &str) -> Vec<ModPatchStatus> {
    match svc.mod_patches_get(id).unwrap() {
        Loadable::Ready(v) => v,
        Loadable::Loading => panic!("patches still loading"),
    }
}

#[test]
fn mod_patches_loading_before_simulation() {
    let c = ctx();
    c.fx.add_mod("probe", "1.0.0", CONSOLE_PATCH);
    assert!(matches!(
        c.svc.mod_patches_get("probe").unwrap(),
        Loadable::Loading
    ));
}

#[test]
fn mod_patches_reports_applied_patch() {
    let c = warmed();
    let list = patches(&c.svc, "probe");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].target_file, "conf.lua");
    assert_eq!(list[0].state, PatchState::Applied);
    assert!(list[0].conflicts.is_empty());
}

#[test]
fn disabled_patch_reported_disabled() {
    let c = warmed();
    let id = patches(&c.svc, "probe")[0].patch_id.clone();
    c.svc.patch_set_enabled(&id, false).unwrap();
    idle(&c.svc);
    let list = patches(&c.svc, "probe");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].patch_id, id);
    assert_eq!(list[0].state, PatchState::Disabled);
}

#[test]
fn unknown_mod_is_not_found() {
    let c = warmed();
    assert_eq!(
        c.svc.mod_patches_get("nope").unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn overlapping_mods_each_carry_the_conflict() {
    let c = warmed_with(
        &[("alpha", 10, CONSOLE_PATCH), ("beta", 20, FOLLOW_UP_PATCH)],
        &[],
    );
    let a = patches(&c.svc, "alpha");
    let b = patches(&c.svc, "beta");
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_eq!(a[0].conflicts.len(), 1);
    assert_eq!(b[0].conflicts.len(), 1);
    assert_eq!(a[0].conflicts[0].kind, "overlapping_region");
    assert_eq!(a[0].conflicts[0].id, b[0].conflicts[0].id);
}

#[test]
fn unmatched_pattern_is_broken_with_anchor_conflict() {
    let c = warmed_with(&[("probe", 50, MISSING_ANCHOR_PATCH)], &[]);
    let list = patches(&c.svc, "probe");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, PatchState::Broken);
    assert_eq!(list[0].conflicts.len(), 1);
    assert!(list[0].conflicts[0].kind.starts_with("broken_anchor"));
}

#[test]
fn failed_match_under_skip_patch_is_broken() {
    let c = warmed_with(&[("probe", 50, SKIP_PATCH_MISS)], &[]);
    let list = patches(&c.svc, "probe");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, PatchState::Broken);
}

#[test]
fn patch_skipped_with_its_mod_stays_skipped() {
    let c = warmed_with(&[("probe", 50, SKIP_MOD_MISS)], &[]);
    let list = patches(&c.svc, "probe");
    assert_eq!(list.len(), 2);
    let state = |n: &str| {
        list.iter()
            .find(|p| p.name.as_deref() == Some(n))
            .unwrap()
            .state
    };
    assert_eq!(state("a-miss"), PatchState::Broken);
    assert_eq!(state("b-follower"), PatchState::Skipped);
}

#[test]
fn disabled_ghost_patch_listed_once_without_target() {
    let c = warmed_with(&[("probe", 50, CONSOLE_PATCH)], &["probe:ghost"]);
    let list = patches(&c.svc, "probe");
    assert_eq!(list.len(), 2);
    let ghosts: Vec<_> = list
        .iter()
        .filter(|p| p.patch_id == "probe:ghost")
        .collect();
    assert_eq!(ghosts.len(), 1);
    assert_eq!(ghosts[0].state, PatchState::Disabled);
    assert!(ghosts[0].target_file.is_empty());
    assert_eq!(ghosts[0].name.as_deref(), Some("ghost"));
    // The real patch is untouched.
    assert!(list
        .iter()
        .any(|p| p.target_file == "conf.lua" && p.state == PatchState::Applied));
}

#[test]
fn failed_patch_with_skip_mod_in_path_is_broken() {
    // The mod id (hence its folder and source path) and the pattern both contain "skip_mod";
    // the failure warning quotes them but is not the skipped-with-mod marker.
    const TOML: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
target = "conf.lua"
pattern = "skip_mod missing anchor"
position = "at"
payload = "x"
on_failure = "skip_patch"
"#;
    let c = warmed_with(&[("skip_mod_tweaks", 50, TOML)], &[]);
    let list = patches(&c.svc, "skip_mod_tweaks");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, PatchState::Broken);
}

const NAMED_PATCH: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
name = "console"
target = "conf.lua"
pattern = "t.console = false"
position = "at"
payload = "t.console = true"
"#;

#[test]
fn enabling_by_record_id_clears_a_named_disable() {
    let c = warmed_with(&[("probe", 50, NAMED_PATCH)], &["probe:console"]);
    let list = patches(&c.svc, "probe");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, PatchState::Disabled);
    let record_id = list[0].patch_id.clone();
    assert_ne!(record_id, "probe:console");

    c.svc.patch_set_enabled(&record_id, true).unwrap();
    idle(&c.svc);
    let disabled = c.fx.paths.get_profile("default").unwrap().disabled_patches;
    assert!(disabled.is_empty(), "left behind: {disabled:?}");
    assert_eq!(patches(&c.svc, "probe")[0].state, PatchState::Applied);
}

#[test]
fn enabling_by_named_id_clears_a_record_id_disable() {
    let c = warmed_with(&[("probe", 50, NAMED_PATCH)], &[]);
    let record_id = patches(&c.svc, "probe")[0].patch_id.clone();
    c.svc.patch_set_enabled(&record_id, false).unwrap();
    idle(&c.svc);
    assert_eq!(patches(&c.svc, "probe")[0].state, PatchState::Disabled);

    c.svc.patch_set_enabled("probe:console", true).unwrap();
    idle(&c.svc);
    let disabled = c.fx.paths.get_profile("default").unwrap().disabled_patches;
    assert!(disabled.is_empty(), "left behind: {disabled:?}");
    assert_eq!(patches(&c.svc, "probe")[0].state, PatchState::Applied);
}
