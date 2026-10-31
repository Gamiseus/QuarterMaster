#![allow(clippy::unwrap_used)]
mod support;
use qm_service::testing::{
    cached, empty_simulation_report, seed_all_but_simulation, seed_mods, seed_simulation,
    seed_vanilla, FakeProbe, RecordingSink,
};
use qm_service::{JobKind, Loadable, QmEvent, Scope, Service, VanillaFiles};
use std::sync::Arc;

fn svc() -> (support::Fixture, Arc<RecordingSink>, Service) {
    let fx = support::Fixture::new();
    let sink = Arc::new(RecordingSink::new());
    let s = Service::new(fx.paths.clone(), sink.clone(), Arc::new(FakeProbe::new()));
    (fx, sink, s)
}

/// Occupy the refill singletons (WarmMods, WarmVanilla, Simulation) so refreshes requested by
/// `invalidate` join these jobs and cannot refill caches while a test asserts. Dropping the guard
/// releases them and waits for the retried refills to finish.
struct Hold<'a> {
    svc: &'a Service,
    release: Vec<std::sync::mpsc::Sender<()>>,
}

fn hold(svc: &Service) -> Hold<'_> {
    let mut release = Vec::new();
    for kind in [JobKind::WarmMods, JobKind::WarmVanilla, JobKind::Simulation] {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        svc.jobs().spawn(kind, move |_| {
            let _ = rx.recv();
            Ok(None)
        });
        release.push(tx);
    }
    Hold { svc, release }
}

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        self.release.clear();
        // Retries start right after the held jobs leave the table; let them settle.
        for _ in 0..5 {
            assert!(self
                .svc
                .jobs()
                .wait_idle(std::time::Duration::from_secs(20)));
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }
}

fn scopes(sink: &RecordingSink) -> Vec<Scope> {
    sink.events()
        .into_iter()
        .filter_map(|e| match e {
            QmEvent::Invalidated { scope } => Some(scope),
            _ => None,
        })
        .collect()
}

#[test]
fn loadable_serializes_tagged() {
    let ready = serde_json::to_string(&Loadable::<u32>::Ready(3)).unwrap();
    assert_eq!(ready, r#"{"status":"ready","data":3}"#);
    let loading = serde_json::to_string(&Loadable::<u32>::Loading).unwrap();
    assert_eq!(loading, r#"{"status":"loading"}"#);
}

#[test]
fn invalidate_mods_clears_and_emits_mods_and_load_order() {
    let (_fx, sink, s) = svc();
    let _h = hold(&s);
    seed_mods(&s, vec![]);
    s.invalidate(Scope::Mods);
    assert!(!cached(&s).mods);
    assert_eq!(scopes(&sink), vec![Scope::Mods, Scope::LoadOrder]);
}

#[test]
fn invalidate_settings_clears_vanilla() {
    let (_fx, sink, s) = svc();
    let _h = hold(&s);
    seed_vanilla(
        &s,
        VanillaFiles {
            game_version: "x".into(),
            info: None,
            files: Default::default(),
        },
    );
    s.invalidate(Scope::Settings);
    assert!(!cached(&s).vanilla);
    assert_eq!(
        scopes(&sink),
        vec![Scope::Settings, Scope::LoadOrder, Scope::Health]
    );
}

#[test]
fn invalidation_table_rows() {
    use Scope::*;
    // (scope, caches cleared, events emitted in order)
    let rows: Vec<(Scope, &[&str], Vec<Scope>)> = vec![
        (
            Mods,
            &["mods", "mod_detail", "simulation"],
            vec![Mods, LoadOrder],
        ),
        (Profiles, &["simulation"], vec![Profiles, LoadOrder]),
        (LoadOrder, &["simulation"], vec![LoadOrder]),
        (Saves, &["save_detail"], vec![Saves]),
        (
            Settings,
            &["vanilla", "simulation", "health"],
            vec![Settings, LoadOrder, Health],
        ),
        (Health, &["health"], vec![Health]),
        (Crashes, &[], vec![Crashes]),
        (Updates, &["updates"], vec![Updates]),
        (Catalog, &["catalog"], vec![Catalog]),
    ];
    for (scope, cleared, emitted) in rows {
        let (_fx, sink, s) = svc();
        let h = hold(&s);
        seed_all_but_simulation(&s);
        seed_simulation(&s, empty_simulation_report());
        let before = cached(&s);
        assert!(before.mods && before.vanilla && before.simulation && before.health);
        assert!(!before.mod_detail_empty && !before.save_detail_empty);
        assert!(before.updates && before.catalog);
        s.invalidate(scope);
        let a = cached(&s);
        let has = |n: &str| cleared.contains(&n);
        assert_eq!(a.mods, !has("mods"), "{scope:?} mods");
        assert_eq!(a.vanilla, !has("vanilla"), "{scope:?} vanilla");
        assert_eq!(a.simulation, !has("simulation"), "{scope:?} simulation");
        assert_eq!(
            a.mod_detail_empty,
            has("mod_detail"),
            "{scope:?} mod_detail"
        );
        assert_eq!(
            a.save_detail_empty,
            has("save_detail"),
            "{scope:?} save_detail"
        );
        assert_eq!(a.health, !has("health"), "{scope:?} health");
        assert_eq!(a.updates, !has("updates"), "{scope:?} updates");
        assert_eq!(a.catalog, !has("catalog"), "{scope:?} catalog");
        assert_eq!(scopes(&sink), emitted, "{scope:?} events");
        drop(h);
    }
}
