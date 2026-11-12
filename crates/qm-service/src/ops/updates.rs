//! Update checks, the community catalog, catalog installs and launcher self-update.
use crate::cache::Loadable;
use crate::dto::{CatalogEntry, CatalogQuery, CatalogResult, ModUpdateInfo, UpdatesReport};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::github::{self, CheckError};
use crate::jobs::{JobCtx, JobId, JobKind};
use crate::net::{MAX_DOWNLOAD_BYTES, MSG_NOT_ALLOWED};
use crate::ops::mods::is_plain_id;
use crate::service::Service;
use qm_manager::community;
use qm_schema::IndexEntry;
use qm_schema::{ParsedUpdateKey, ParsedVersion};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;

/// Real GitHub check results per raw update key; `updates_pass` is its only writer.
const UPDATE_CACHE_FILE: &str = "update-etags.json";
const MSG_HASH_MISMATCH: &str =
    "The download does not match the published mod. It was not installed.";

impl Service {
    /// Cached update report; a miss starts a refresh (unless one is running) and returns
    /// `Loading`. The pass asks GitHub for each mod's latest release (conditionally).
    pub fn updates_get(&self) -> Loadable<UpdatesReport> {
        match self.inner.caches.updates.get() {
            Some(r) => Loadable::Ready(r),
            None => {
                self.refresh_if_idle(JobKind::UpdatesRefresh);
                Loadable::Loading
            }
        }
    }

    /// An update check that asks the network. A cache miss (`updates_get`) only reads the
    /// stored results, so turning off automatic checks keeps the launcher offline.
    pub fn updates_refresh(&self) -> JobId {
        self.inner.updates_network.store(true, Ordering::SeqCst);
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

    /// Download and import a catalog mod in a `CatalogInstall` job (not cancellable); the
    /// output is the imported ids. The download finishes before the install lock is taken.
    pub fn catalog_install(&self, download_url: &str, mod_id: Option<String>) -> QmResult<JobId> {
        self.check_download(download_url)?;
        self.ensure_not_running()?;
        let url = download_url.trim().to_string();
        let svc = self.clone();
        Ok(self.jobs().spawn(JobKind::CatalogInstall, move |ctx| {
            ctx.progress(0.1, "Downloading");
            let allow = svc.net().allow_download.as_ref();
            let zip = community::download_to_temp(svc.paths(), &url, MAX_DOWNLOAD_BYTES, allow)
                .map_err(download_error)?;
            svc.verify_against_index(&url, &zip)?;
            let _install = svc.lock_install();
            if let Err(e) = svc.ensure_not_running() {
                if let Err(rm) = std::fs::remove_file(&zip) {
                    tracing::warn!("could not remove {}: {rm}", zip.display());
                }
                return Err(e);
            }
            ctx.progress(0.6, "Installing");
            let v = community::install_downloaded(
                svc.paths(),
                &zip,
                &url,
                mod_id.as_deref(),
                "upgrade",
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

    /// Install the update the cached report offers for `mod_id` in a `ModUpdateInstall` job (not
    /// cancellable). The new version goes in alongside the old one: the active profile moves to
    /// it, other profiles stay pinned to the version they used. The output is the new version.
    pub fn mod_update_install(&self, mod_id: &str) -> QmResult<JobId> {
        if !is_plain_id(mod_id) {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                format!("'{mod_id}' is not a valid mod id."),
            ));
        }
        self.ensure_not_running()?;
        let installed = self.installed_mod(mod_id)?;
        if installed.is_workspace_linked {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "This mod is linked to your workspace and updates from there.",
            ));
        }
        // The report's URL is untrusted: the job checks it (and every redirect) with the policy.
        let url = self
            .inner
            .caches
            .updates
            .get()
            .and_then(|r| {
                r.mods
                    .into_iter()
                    .find(|m| m.mod_id == mod_id && m.update_available)
            })
            .and_then(|m| m.download_url)
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                QmError::new(
                    ErrorKind::InvalidInput,
                    "No update is available for this mod.",
                )
            })?;
        let mod_id = mod_id.to_string();
        let svc = self.clone();
        Ok(self.jobs().spawn(JobKind::ModUpdateInstall, move |ctx| {
            ctx.progress(0.1, "Downloading");
            let allow = svc.net().allow_download.as_ref();
            let zip = community::download_to_temp(svc.paths(), &url, MAX_DOWNLOAD_BYTES, allow)
                .map_err(download_error)?;
            svc.verify_against_index(&url, &zip)?;
            let _install = svc.lock_install();
            if let Err(e) = svc.ensure_not_running() {
                if let Err(rm) = std::fs::remove_file(&zip) {
                    tracing::warn!("could not remove {}: {rm}", zip.display());
                }
                return Err(e);
            }
            ctx.progress(0.6, "Installing");
            let v =
                community::install_downloaded(svc.paths(), &zip, &url, Some(&mod_id), "alongside")
                    .map_err(download_error)?;
            let Some(version) = imported_version(&v, &mod_id) else {
                return Err(QmError::new(
                    ErrorKind::InvalidInput,
                    format!("The download does not contain {mod_id}."),
                ));
            };
            ctx.progress(0.9, "Updating profile");
            let mut active = svc.active_profile()?;
            let had_pin = active.pinned_versions.remove(&mod_id).is_some();
            if had_pin || active.enabled_mods.contains(&mod_id) {
                svc.commit_active_profile(active)?;
            }
            svc.invalidate(Scope::Mods);
            svc.mark_updated(&mod_id, &version);
            Ok(Some(version))
        }))
    }

    /// Patch the cached report after an update install (no new network check).
    fn mark_updated(&self, mod_id: &str, version: &str) {
        let patched = self.with_gens(|_| {
            let Some(mut report) = self.inner.caches.updates.get() else {
                return false;
            };
            for m in report.mods.iter_mut().filter(|m| m.mod_id == mod_id) {
                m.installed_version = version.to_string();
                m.update_available = false;
            }
            self.inner.caches.updates.set(report);
            true
        });
        if patched {
            self.notify(Scope::Updates);
        }
    }

    /// Download and stage a launcher build; `sha256` must match. The output is the staged path.
    pub fn launcher_update_download(&self, download_url: &str, sha256: String) -> QmResult<JobId> {
        self.check_download(download_url)?;
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
                        svc.net().allow_download.as_ref(),
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

    /// One `UpdatesRefresh` pass: the manager's offline report, then the mod index (one
    /// request for every indexed mod) and github.com's latest-release redirect for GitHub mods
    /// that are not indexed. A 429 from github.com stops those checks with mode `busy`. Without
    /// `network` (a cache miss) only stored results are used. The REST API is never called.
    pub(crate) fn updates_pass(&self, ctx: &JobCtx) -> QmResult<Option<String>> {
        let gen = self.generations().updates;
        let network = self.inner.updates_network.swap(false, Ordering::SeqCst);
        let cache_file = self.paths().cache_dir.join(UPDATE_CACHE_FILE);
        let mut entries = read_entries(&cache_file);
        ctx.progress(0.1, "Checking for updates");
        let raw = self
            .paths()
            .check_for_updates_with(false)
            .or_kind(ErrorKind::Network)?;
        let mut report: UpdatesReport = parse(raw, "update report")?;
        let index = self.index_load(network);

        let mut checked: HashMap<String, Outcome> = HashMap::new();
        let mut busy = false;
        let total = report.mods.len().max(1);
        for (i, m) in report.mods.iter_mut().enumerate() {
            // The index is matched by mod id, so an entry is only this mod's update when it comes
            // from the repository the installed mod names. Otherwise the mod's own key is asked.
            let listed = index
                .file
                .find(&m.mod_id)
                .filter(|e| same_source(&m.update_key, &e.repository));
            if let Some(entry) = listed {
                m.latest_version = entry.latest_version.clone();
                m.download_url = Some(entry.download_url.clone());
                m.update_available = is_newer(&m.latest_version, &m.installed_version);
                continue;
            }
            let ParsedUpdateKey::GitHub { owner, repo, asset } =
                ParsedUpdateKey::parse(&m.update_key)
            else {
                continue;
            };
            let key = m.update_key.clone();
            let outcome = match checked.get(&key) {
                Some(o) => *o,
                None if busy || !network => Outcome::Stored,
                None => {
                    ctx.progress(0.2 + 0.7 * i as f32 / total as f32, "Checking for updates");
                    let o =
                        match github::latest_release(self.net(), &owner, &repo, asset.as_deref()) {
                            Ok(rel) => {
                                merge_entry(&mut entries, &key, |e| {
                                    e.remove("etag");
                                    e.insert("latest_version".into(), rel.tag.into());
                                    e.insert("download_url".into(), rel.download_url.into());
                                    e.insert("checked_at".into(), now().into());
                                });
                                Outcome::Stored
                            }
                            Err(CheckError::NotFound) => Outcome::NotFound,
                            Err(CheckError::Busy) => {
                                busy = true;
                                Outcome::Stored
                            }
                            Err(CheckError::Network(e)) => {
                                tracing::warn!("update check for {key} failed: {e}");
                                Outcome::Stored
                            }
                        };
                    checked.insert(key.clone(), o);
                    o
                }
            };
            match outcome {
                Outcome::Stored => apply_entry(m, entries.get(&key)),
                Outcome::NotFound => {
                    m.update_available = false;
                    m.download_url = None;
                }
            }
        }
        report.rate_limit_mode = if busy { "busy" } else { "normal" }.to_string();
        if network {
            if let Err(e) = write_entries(&cache_file, &entries) {
                tracing::warn!("could not write {}: {e}", cache_file.display());
            }
        }
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

    /// `InvalidInput` (before any job starts) unless the policy allows downloading `url`.
    fn check_download(&self, url: &str) -> QmResult<()> {
        (self.net().allow_download)(url.trim())
            .map_err(|e| QmError::new(ErrorKind::InvalidInput, MSG_NOT_ALLOWED).with_details(e))
    }

    /// A download the index lists must match its SHA-256; a mismatch deletes the file and is
    /// refused. Downloads the index does not list (github.com fallback) are not checked here.
    fn verify_against_index(&self, url: &str, file: &Path) -> QmResult<()> {
        let index = self.index_load(false);
        let Some(entry) = index
            .file
            .mods
            .iter()
            .find(|m| m.download_url.trim() == url.trim())
        else {
            return Ok(());
        };
        let matches = std::fs::read(file)
            .map(|b| qm_schema::sha256_hex(&b).eq_ignore_ascii_case(&entry.sha256))
            .unwrap_or(false);
        if matches {
            return Ok(());
        }
        if let Err(e) = std::fs::remove_file(file) {
            tracing::warn!("could not remove {}: {e}", file.display());
        }
        Err(QmError::new(ErrorKind::Integrity, MSG_HASH_MISMATCH))
    }

    /// One `CatalogRefresh` pass: the catalog is the mod index (one conditional request) plus
    /// the Nexus Mods row.
    pub(crate) fn catalog_pass(&self, _ctx: &JobCtx) -> QmResult<Option<String>> {
        let gen = self.generations().catalog;
        let index = self.index_load(true);
        let mut catalog: Vec<CatalogEntry> = index.file.mods.iter().map(catalog_entry).collect();
        catalog.push(nexus_entry());
        let result = CatalogResult {
            total_count: u32::try_from(catalog.len()).unwrap_or(u32::MAX),
            catalog,
            is_cached: index.offline,
            query: String::new(),
            rate_limited: false,
            error_message: (index.offline && index.copy_from.is_none()).then(|| {
                "The mod list could not be loaded. Check your connection and try again.".to_string()
            }),
            offline: index.offline,
            copy_from: index.copy_from,
        };
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

/// The index entry comes from the repository the installed mod's GitHub update key names. A mod
/// without a GitHub key names no source to compare, so the index is taken as it is.
fn same_source(update_key: &str, repository: &str) -> bool {
    match ParsedUpdateKey::parse(update_key) {
        ParsedUpdateKey::GitHub { owner, repo, .. } => {
            format!("{owner}/{repo}").eq_ignore_ascii_case(repository.trim())
        }
        _ => true,
    }
}

fn catalog_entry(e: &IndexEntry) -> CatalogEntry {
    let owner = e
        .repository
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string();
    CatalogEntry {
        id: e.id.clone(),
        name: e.name.clone(),
        repo_full_name: e.repository.clone(),
        owner,
        description: e.summary.clone(),
        stargazers_count: 0,
        forks_count: 0,
        updated_at: e.released_at.clone(),
        html_url: if e.page_url.is_empty() {
            format!("https://github.com/{}", e.repository)
        } else {
            e.page_url.clone()
        },
        topics: e.topics.clone(),
        license: e.license.clone(),
        source_type: "GitHub".to_string(),
        latest_release_tag: Some(e.release_tag.clone()).filter(|t| !t.is_empty()),
        download_url: Some(e.download_url.clone()),
        download_asset_name: Some(e.asset_name.clone()).filter(|a| !a.is_empty()),
        installed: false,
        installed_version: None,
        update_key: Some(if e.asset_name.is_empty() {
            format!("GitHub:{}", e.repository)
        } else {
            format!("GitHub:{}#{}", e.repository, e.asset_name)
        }),
        listing: match e.listing {
            qm_schema::Listing::Registered => "registered",
            qm_schema::Listing::Discovered => "discovered",
        }
        .to_string(),
    }
}

/// The Nexus Mods hub: a link, not an installable mod.
fn nexus_entry() -> CatalogEntry {
    CatalogEntry {
        id: "nexus_mudandblood".to_string(),
        name: "Mud and Blood on Nexus Mods".to_string(),
        repo_full_name: "NexusMods:mudandblood".to_string(),
        owner: "NexusMods".to_string(),
        description: "Official Mud and Blood modding community hub on Nexus Mods.".to_string(),
        updated_at: "Live".to_string(),
        html_url: "https://www.nexusmods.com/mudandblood/mods".to_string(),
        topics: vec!["nexusmods".to_string(), "mud-and-blood".to_string()],
        source_type: "NexusMods".to_string(),
        update_key: Some("Nexus:mudandblood".to_string()),
        ..Default::default()
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

/// What one GitHub answer means for every mod sharing that update key.
#[derive(Debug, Clone, Copy)]
enum Outcome {
    /// Use the (possibly just updated) stored entry.
    Stored,
    /// The repository or its latest release is gone: no update.
    NotFound,
}

fn now() -> String {
    jiff::Zoned::now().to_string()
}

/// The stored entries; a missing or unreadable file is an empty cache.
fn read_entries(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write via a temp file and rename, so a crash never leaves half a file.
fn write_entries(path: &Path, entries: &Map<String, Value>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(entries).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// Update the fields of `key`'s entry, keeping the others (old entries may hold any shape).
fn merge_entry(
    entries: &mut Map<String, Value>,
    key: &str,
    f: impl FnOnce(&mut Map<String, Value>),
) {
    let entry = entries
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    if let Value::Object(map) = entry {
        f(map);
    }
}

/// Take `latest_version` and `download_url` from a stored entry; without one the offline
/// values stand.
fn apply_entry(m: &mut ModUpdateInfo, entry: Option<&Value>) {
    let Some(entry) = entry else {
        return;
    };
    if let Some(latest) = entry.get("latest_version").and_then(Value::as_str) {
        m.latest_version = latest.to_string();
    }
    m.download_url = entry
        .get("download_url")
        .and_then(Value::as_str)
        .map(str::to_string);
    m.update_available = is_newer(&m.latest_version, &m.installed_version);
}

fn is_newer(latest: &str, installed: &str) -> bool {
    match (
        ParsedVersion::parse(latest),
        ParsedVersion::parse(installed),
    ) {
        (Ok(l), Ok(i)) => l > i,
        _ => false,
    }
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

/// The version of `mod_id` in the manager's install result, if it was imported.
fn imported_version(v: &serde_json::Value, mod_id: &str) -> Option<String> {
    v.pointer("/import_summary/imported_mods")?
        .as_array()?
        .iter()
        .find(|m| m.pointer("/manifest/id").and_then(Value::as_str) == Some(mod_id))?
        .pointer("/manifest/version")?
        .as_str()
        .map(str::to_string)
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
