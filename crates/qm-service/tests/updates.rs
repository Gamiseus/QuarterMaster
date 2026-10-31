#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use qm_service::net::{check_download_url, MAX_DOWNLOAD_BYTES};
use qm_service::testing::{seed_catalog, FakeProbe, RecordingSink};
use qm_service::{CatalogEntry, CatalogQuery, CatalogResult, ErrorKind, Loadable, Service};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn svc() -> (support::Fixture, Service) {
    let fx = support::Fixture::new();
    let s = Service::new(
        fx.paths.clone(),
        Arc::new(RecordingSink::new()),
        Arc::new(FakeProbe::new()),
    );
    (fx, s)
}

#[test]
fn check_download_url_allows_github_release_assets() {
    for url in [
        "https://github.com/owner/repo/releases/download/v1.0.0/mod.zip",
        "https://objects.githubusercontent.com/github-production-release-asset/1/2",
        "https://release-assets.githubusercontent.com/github-production-release-asset/1/2",
        "https://codeload.github.com/owner/repo/zip/refs/heads/main",
        "https://GitHub.com/owner/repo/archive/refs/heads/main.zip",
    ] {
        check_download_url(url).unwrap_or_else(|e| panic!("{url}: {e}"));
    }
    assert_eq!(MAX_DOWNLOAD_BYTES, 512 * 1024 * 1024);
}

#[test]
fn check_download_url_rejects_http_and_other_hosts() {
    for url in [
        "http://github.com/x",
        "https://evil.example/x",
        "https://github.com.evil.example/x",
        "https://user@evil.example/github.com/x",
        "https://github.com:8443/x",
        "ftp://github.com/x",
        "github.com/x",
        "",
    ] {
        let err = check_download_url(url).expect_err(url);
        assert_eq!(err.kind, ErrorKind::InvalidInput, "{url}");
    }
}

#[test]
fn catalog_install_rejects_before_spawning_job() {
    let (_fx, s) = svc();
    let err = s
        .catalog_install("https://evil.example/mod.zip", None)
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    let err = s
        .launcher_update_download("http://github.com/x.exe", "00".repeat(32))
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    assert!(s.jobs().list().is_empty());
}

fn eventually<T>(f: impl Fn() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for Ready");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn updates_get_miss_is_loading_then_fills_offline() {
    let (fx, s) = svc();
    // A fresh ETag cache: startup would skip the refresh, so the miss must schedule it.
    std::fs::create_dir_all(&fx.paths.cache_dir).unwrap();
    std::fs::write(fx.paths.cache_dir.join("update-etags.json"), "{}").unwrap();
    let start = Instant::now();
    let got = s.updates_get();
    assert!(start.elapsed() < Duration::from_millis(50));
    assert!(matches!(got, Loadable::Loading));
    let report = eventually(|| match s.updates_get() {
        Loadable::Ready(r) => Some(r),
        _ => None,
    });
    assert!(report.mods.is_empty());
}

#[test]
fn catalog_get_miss_fills_from_fresh_disk_cache() {
    let (fx, s) = svc();
    std::fs::create_dir_all(&fx.paths.cache_dir).unwrap();
    let disk = serde_json::json!({
        "catalog": [{ "id": "disk", "name": "From disk", "source_type": "GitHub" }],
        "total_count": 1,
        "is_cached": true
    });
    std::fs::write(
        fx.paths.cache_dir.join("community-catalog-cache.json"),
        disk.to_string(),
    )
    .unwrap();
    assert!(matches!(
        s.catalog_get(q(None, None, None)),
        Loadable::Loading
    ));
    // Repeated misses while loading join the running pass.
    assert!(matches!(
        s.catalog_get(q(None, None, None)),
        Loadable::Loading | Loadable::Ready(_)
    ));
    let got = eventually(|| match s.catalog_get(q(None, None, None)) {
        Loadable::Ready(r) => Some(r),
        _ => None,
    });
    assert_eq!(
        got.catalog.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        vec!["disk"]
    );
    assert!(s.jobs().wait_idle(Duration::from_secs(10)));
}

fn entry(id: &str, name: &str, desc: &str, topics: &[&str], source: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.into(),
        name: name.into(),
        description: desc.into(),
        topics: topics.iter().map(|t| t.to_string()).collect(),
        source_type: source.into(),
        ..Default::default()
    }
}

fn ids(r: Loadable<CatalogResult>) -> Vec<String> {
    match r {
        Loadable::Ready(c) => {
            assert_eq!(c.total_count as usize, c.catalog.len());
            c.catalog.into_iter().map(|e| e.id).collect()
        }
        Loadable::Loading => panic!("catalog loading"),
    }
}

fn q(text: Option<&str>, topic: Option<&str>, source: Option<&str>) -> CatalogQuery {
    CatalogQuery {
        text: text.map(String::from),
        topic: topic.map(String::from),
        source: source.map(String::from),
    }
}

#[test]
fn catalog_get_filters_locally() {
    let (_fx, s) = svc();
    seed_catalog(
        &s,
        CatalogResult {
            catalog: vec![
                entry(
                    "a",
                    "Better Medics",
                    "Heals faster",
                    &["medic", "quartermaster-mod"],
                    "GitHub",
                ),
                entry(
                    "b",
                    "Nexus hub",
                    "Community mods",
                    &["nexusmods"],
                    "NexusMods",
                ),
            ],
            total_count: 2,
            ..Default::default()
        },
    );
    assert_eq!(ids(s.catalog_get(q(None, None, None))), vec!["a", "b"]);
    assert_eq!(ids(s.catalog_get(q(Some("MEDICS"), None, None))), vec!["a"]);
    assert_eq!(
        ids(s.catalog_get(q(Some("community"), None, None))),
        vec!["b"]
    );
    assert_eq!(
        ids(s.catalog_get(q(Some("nexusmods"), None, None))),
        vec!["b"]
    );
    assert_eq!(ids(s.catalog_get(q(None, Some("medic"), None))), vec!["a"]);
    assert_eq!(
        ids(s.catalog_get(q(None, Some("med"), None))),
        Vec::<String>::new()
    );
    assert_eq!(
        ids(s.catalog_get(q(None, None, Some("NexusMods")))),
        vec!["b"]
    );
    assert_eq!(
        ids(s.catalog_get(q(Some("heals"), None, Some("NexusMods")))),
        Vec::<String>::new()
    );
    assert!(s.jobs().list().is_empty());
}

#[test]
fn catalog_entry_counts_saturate_instead_of_failing() {
    let raw = serde_json::json!({
        "catalog": [{ "id": "a", "stargazers_count": 5_000_000_000u64, "forks_count": 7 }],
        "total_count": 1
    });
    let r: CatalogResult = serde_json::from_value(raw).unwrap();
    assert_eq!(r.catalog[0].stargazers_count, u32::MAX);
    assert_eq!(r.catalog[0].forks_count, 7);
}

#[test]
fn catalog_installed_flag_ignores_empty_update_keys() {
    let (fx, s) = svc();
    fx.add_mod("probe", "1.0.0", "[manifest]\nversion = \"1.0.0\"\n");
    let mut mods = fx.paths.list_installed_mods();
    mods[0].manifest.update_keys = vec![String::new()];
    qm_service::testing::seed_mods(&s, mods);
    seed_catalog(
        &s,
        CatalogResult {
            catalog: vec![
                CatalogEntry {
                    id: "x".into(),
                    name: "Other".into(),
                    update_key: Some(String::new()),
                    ..Default::default()
                },
                CatalogEntry {
                    id: "p".into(),
                    name: "PROBE".into(),
                    ..Default::default()
                },
            ],
            total_count: 2,
            ..Default::default()
        },
    );
    let Loadable::Ready(r) = s.catalog_get(q(None, None, None)) else {
        panic!("catalog loading")
    };
    assert!(!r.catalog[0].installed);
    assert!(r.catalog[1].installed);
    assert_eq!(r.catalog[1].installed_version.as_deref(), Some("1.0.0"));
}
