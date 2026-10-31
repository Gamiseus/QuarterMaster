//! Update checks, the community catalog, catalog installs and launcher self-update.
use crate::cache::Loadable;
use crate::dto::{CatalogEntry, CatalogQuery, CatalogResult, UpdatesReport};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::jobs::{JobCtx, JobId, JobKind};
use crate::net::{check_download_url, MAX_DOWNLOAD_BYTES};
use crate::service::Service;

impl Service {
    /// Cached update report; a miss starts a refresh (unless one is running) and returns
    /// `Loading`. The pass builds the report from the manager's local ETag cache.
    pub fn updates_get(&self) -> Loadable<UpdatesReport> {
        match self.inner.caches.updates.get() {
            Some(r) => Loadable::Ready(r),
            None => {
                self.refresh_if_idle(JobKind::UpdatesRefresh);
                Loadable::Loading
            }
        }
    }

    pub fn updates_refresh(&self) -> JobId {
        self.refresh(JobKind::UpdatesRefresh)
    }

    /// The cached default catalog filtered locally. A miss starts a refresh (unless one is
    /// running), which reads the manager's disk cache while it is fresh, and returns `Loading`.
    pub fn catalog_get(&self, query: CatalogQuery) -> Loadable<CatalogResult> {
        let Some(mut result) = self.inner.caches.catalog.get() else {
            self.refresh_if_idle(JobKind::CatalogRefresh);
            return Loadable::Loading;
        };
        let text = filter_value(query.text.as_deref()).map(str::to_lowercase);
        let topic = filter_value(query.topic.as_deref());
        let source = filter_value(query.source.as_deref());
        result.catalog.retain(|e| {
            text.as_deref().is_none_or(|t| matches_text(e, t))
                && topic.is_none_or(|t| e.topics.iter().any(|x| x.eq_ignore_ascii_case(t)))
                && source.is_none_or(|s| e.source_type.eq_ignore_ascii_case(s))
        });
        self.annotate_installed(&mut result.catalog);
        result.total_count = u32::try_from(result.catalog.len()).unwrap_or(u32::MAX);
        result.query = text.unwrap_or_default();
        Loadable::Ready(result)
    }

    pub fn catalog_refresh(&self) -> JobId {
        self.refresh(JobKind::CatalogRefresh)
    }

    /// Download and import a catalog mod in a cancellable job; the output is the imported ids.
    pub fn catalog_install(&self, download_url: &str, mod_id: Option<String>) -> QmResult<JobId> {
        check_download_url(download_url)?;
        let url = download_url.trim().to_string();
        let svc = self.clone();
        Ok(self.jobs().spawn(JobKind::CatalogInstall, move |ctx| {
            check_cancel(ctx)?;
            ctx.progress(0.1, "Downloading");
            let v = qm_manager::community::download_and_install_community_mod(
                svc.paths(),
                &url,
                mod_id.as_deref(),
                MAX_DOWNLOAD_BYTES,
                &allow_hop,
            )
            .map_err(download_error)?;
            let ids = imported_ids(&v);
            ctx.progress(0.9, "Updating mod index");
            svc.after_mod_import(&ids)?;
            svc.notify(Scope::Catalog);
            if svc.inner.caches.updates.get().is_some() {
                svc.updates_refresh();
            }
            Ok(Some(ids.join(", ")))
        }))
    }

    /// Download and stage a launcher build; `sha256` must match. The output is the staged path.
    pub fn launcher_update_download(&self, download_url: &str, sha256: String) -> QmResult<JobId> {
        check_download_url(download_url)?;
        let sha = sha256.trim().to_ascii_lowercase();
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "SHA-256 must be 64 hexadecimal characters.",
            ));
        }
        let url = download_url.trim().to_string();
        let svc = self.clone();
        Ok(self
            .jobs()
            .spawn(JobKind::LauncherUpdateDownload, move |ctx| {
                check_cancel(ctx)?;
                ctx.progress(0.1, "Downloading");
                let staged = svc
                    .paths()
                    .download_and_stage_launcher_update(
                        &url,
                        Some(&sha),
                        MAX_DOWNLOAD_BYTES,
                        &allow_hop,
                    )
                    .map_err(download_error)?;
                svc.notify(Scope::Updates);
                Ok(Some(staged.display().to_string()))
            }))
    }

    /// Swap in the staged launcher build and restart (the process exits shortly after).
    pub fn launcher_update_apply(&self) -> QmResult<()> {
        self.ensure_not_running()?;
        self.paths()
            .apply_launcher_update_and_restart()
            .map_err(missing_or_io)
    }

    /// Restore `qm-launcher.old.exe` and restart (the process exits shortly after).
    pub fn launcher_update_rollback(&self) -> QmResult<()> {
        self.ensure_not_running()?;
        self.paths()
            .rollback_launcher_update_and_restart()
            .map_err(missing_or_io)
    }

    /// One `UpdatesRefresh` pass.
    pub(crate) fn updates_pass(&self, ctx: &JobCtx) -> QmResult<Option<String>> {
        let gen = self.generations().updates;
        ctx.progress(0.2, "Checking for updates");
        let raw = self
            .paths()
            .check_for_updates()
            .or_kind(ErrorKind::Network)?;
        let report: UpdatesReport = parse(raw, "update report")?;
        let stored = self.with_gens(|g| {
            if g.updates != gen {
                return false;
            }
            self.inner.caches.updates.set(report);
            true
        });
        if stored {
            self.notify(Scope::Updates);
        }
        Ok(None)
    }

    /// One `CatalogRefresh` pass. The first fill may use the manager's 15-minute disk cache;
    /// later refreshes force a new lookup.
    pub(crate) fn catalog_pass(&self, ctx: &JobCtx) -> QmResult<Option<String>> {
        let gen = self.generations().catalog;
        let force = self.inner.caches.catalog.get().is_some();
        ctx.progress(0.2, "Loading community catalog");
        let raw =
            qm_manager::community::get_community_catalog(self.paths(), None, None, None, force);
        let result: CatalogResult = parse(raw, "community catalog")?;
        let stored = self.with_gens(|g| {
            if g.catalog != gen {
                return false;
            }
            self.inner.caches.catalog.set(result);
            true
        });
        if stored {
            self.notify(Scope::Catalog);
        }
        Ok(None)
    }

    /// Recompute `installed` flags from the mod index when it is cached (it changes on install).
    fn annotate_installed(&self, entries: &mut [CatalogEntry]) {
        let Some(mods) = self.inner.caches.mods.get() else {
            return;
        };
        for e in entries {
            let repo = e.repo_full_name.to_ascii_lowercase();
            let key = e.update_key.as_deref().unwrap_or_default();
            let found = mods.iter().find(|m| {
                (!e.name.is_empty() && m.manifest.name.eq_ignore_ascii_case(&e.name))
                    || m.manifest.update_keys.iter().any(|k| {
                        (!k.is_empty() && !key.is_empty() && k.eq_ignore_ascii_case(key))
                            || (!repo.is_empty() && k.to_ascii_lowercase().contains(&repo))
                    })
            });
            e.installed = found.is_some();
            e.installed_version = found.map(|m| m.manifest.version.clone());
        }
    }
}

fn filter_value(v: Option<&str>) -> Option<&str> {
    v.map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
}

fn matches_text(e: &CatalogEntry, needle_lower: &str) -> bool {
    e.name.to_lowercase().contains(needle_lower)
        || e.description.to_lowercase().contains(needle_lower)
        || e.topics
            .iter()
            .any(|t| t.to_lowercase().contains(needle_lower))
}

fn parse<T: serde::de::DeserializeOwned>(raw: serde_json::Value, what: &str) -> QmResult<T> {
    serde_json::from_value(raw).map_err(|e| {
        QmError::new(
            ErrorKind::Internal,
            format!("The {what} could not be read."),
        )
        .with_details(e.to_string())
    })
}

/// The allowlist, applied by the manager to the start URL and every redirect hop.
fn allow_hop(url: &str) -> Result<(), String> {
    check_download_url(url).map_err(|e| format!("{} ({url})", e.message))
}

fn check_cancel(ctx: &JobCtx) -> QmResult<()> {
    if ctx.is_cancelled() {
        Err(QmError::new(ErrorKind::Cancelled, "Job cancelled"))
    } else {
        Ok(())
    }
}

/// Transport failures are `Network`; size, checksum and archive checks are `InvalidInput`.
fn download_error(msg: String) -> QmError {
    let kind = if msg.starts_with("Download request failed") || msg.starts_with("Read error") {
        ErrorKind::Network
    } else if msg.starts_with("Failed to") {
        ErrorKind::Io
    } else {
        ErrorKind::InvalidInput
    };
    QmError::new(kind, msg)
}

fn missing_or_io(msg: String) -> QmError {
    let kind = if msg.starts_with("No ") {
        ErrorKind::NotFound
    } else {
        ErrorKind::Io
    };
    QmError::new(kind, msg)
}

/// Mod ids from the manager's install result (`import_summary.imported_mods[].manifest.id`).
fn imported_ids(v: &serde_json::Value) -> Vec<String> {
    v.pointer("/import_summary/imported_mods")
        .and_then(|m| m.as_array())
        .map(|mods| {
            mods.iter()
                .filter_map(|m| m.pointer("/manifest/id").and_then(|id| id.as_str()))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}
