//! Startup: attach to a running game or recover the journal, warm caches, refresh stale
//! network caches, start watching. Only `recover` may change the game folder here.
use crate::events::QmEvent;
use crate::service::Service;
use crate::watch::WatchHandle;
use std::path::Path;
use std::time::Duration;

const UPDATE_CACHE_FILE: &str = "update-etags.json";
const CATALOG_CACHE_FILE: &str = crate::index::INDEX_CACHE_FILE;
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
        // First, so an unreadable profile.json is reported before anything lists profiles.
        let (_, notices) = self.paths().list_profiles_reporting();
        self.report_profile_notices(notices);
        let mut recovered = None;
        let mut attached_pid = None;
        match self.inner.probe.find_game() {
            // Recovery is deferred: the exit watcher runs it before cleanup.
            Some(pid) => {
                self.attach_to_running(pid);
                attached_pid = Some(pid);
            }
            None => {
                // Before recovery, while what the runs left is still in the folders.
                self.finish_runs_left_open();
                match self.recover() {
                    Ok(summary) => recovered = summary,
                    Err(e) => {
                        tracing::error!("startup recovery failed: {e}");
                        self.inner.sink.emit(QmEvent::Recovered {
                            summary: format!("Recovery failed: {}", e.message),
                        });
                    }
                }
                // Only while the game is closed: it may switch the active profile. A running game
                // defers it to the next start.
                match self.migrate_launch_model() {
                    Ok(Some(notice)) => {
                        recovered = Some(match recovered {
                            Some(r) => format!("{r} {notice}"),
                            None => notice,
                        });
                    }
                    Ok(None) => {}
                    Err(e) => tracing::error!("launch settings migration failed: {e}"),
                }
                // A test run's plan must not stay staged after the launcher closed (D68).
                self.restage_left_trial();
            }
        }

        self.warm_mods();
        self.warm_vanilla();

        let cache = &self.paths().cache_dir;
        let auto_check = self.paths().load_settings().auto_check_updates;
        if auto_check && is_stale(&cache.join(UPDATE_CACHE_FILE), UPDATE_CACHE_MAX_AGE) {
            self.updates_refresh();
        }
        if auto_check && is_stale(&cache.join(CATALOG_CACHE_FILE), CATALOG_CACHE_MAX_AGE) {
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
