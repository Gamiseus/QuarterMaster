//! Startup: attach to a running game or recover the journal, warm caches, refresh stale
//! network caches, start watching. Only `recover` may change the game folder here.
use crate::events::QmEvent;
use crate::service::Service;
use crate::watch::WatchHandle;
use std::path::Path;
use std::time::Duration;

const UPDATE_CACHE_FILE: &str = "update-etags.json";
const CATALOG_CACHE_FILE: &str = "community-catalog-cache.json";
const UPDATE_CACHE_MAX_AGE: Duration = Duration::from_secs(15 * 60);
const CATALOG_CACHE_MAX_AGE: Duration =
    Duration::from_secs(qm_manager::community::CACHE_TTL_SECONDS);

pub struct StartupReport {
    pub recovered: Option<String>,
    pub attached_pid: Option<u32>,
    pub watch: Option<WatchHandle>,
}

impl Service {
    pub fn start(&self) -> StartupReport {
        let mut recovered = None;
        let mut attached_pid = None;
        match self.inner.probe.find_game() {
            // Recovery is deferred: the exit watcher runs it before cleanup.
            Some(pid) => {
                self.attach_to_running(pid);
                attached_pid = Some(pid);
            }
            None => match self.recover() {
                Ok(summary) => recovered = summary,
                Err(e) => {
                    tracing::error!("startup recovery failed: {e}");
                    self.inner.sink.emit(QmEvent::Recovered {
                        summary: format!("Recovery failed: {}", e.message),
                    });
                }
            },
        }

        self.warm_mods();
        self.warm_vanilla();

        let cache = &self.paths().cache_dir;
        if is_stale(&cache.join(UPDATE_CACHE_FILE), UPDATE_CACHE_MAX_AGE) {
            self.updates_refresh();
        }
        if is_stale(&cache.join(CATALOG_CACHE_FILE), CATALOG_CACHE_MAX_AGE) {
            self.catalog_refresh();
        }

        let watch = match self.start_watching() {
            Ok(h) => Some(h),
            Err(e) => {
                tracing::error!("file watching could not start: {e}");
                None
            }
        };

        StartupReport {
            recovered,
            attached_pid,
            watch,
        }
    }
}

/// Missing, unreadable, future-dated or at least `max_age` old.
fn is_stale(path: &Path, max_age: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_none_or(|age| age >= max_age)
}
