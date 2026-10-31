//! Mod queries and actions, plus the `WarmMods` job.
use crate::cache::Loadable;
use crate::dto::{ConfigResetMode, ImportRequest, ModDetail, ModSummary};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::jobs::{JobCtx, JobId, JobKind};
use crate::service::Service;
use qm_manager::{ArchiveModCandidate, InstalledMod};
use qm_schema::{ConfigValue, ModSettingsStore, SecurityScanReport};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SETTINGS_FILE: &str = "saveModSettings.lua";

impl Service {
    pub fn mods_list(&self) -> Loadable<Vec<ModSummary>> {
        let Some(mods) = self.inner.caches.mods.get() else {
            return Loadable::Loading;
        };
        let enabled = self
            .active_profile()
            .map(|p| p.enabled_mods)
            .unwrap_or_default();
        Loadable::Ready(
            mods.iter()
                .map(|m| ModSummary::new(m, enabled.contains(&m.manifest.id)))
                .collect(),
        )
    }

    /// Readme, config, security scan and integrity for one mod; computed once per mod index.
    pub fn mod_detail(&self, id: &str) -> QmResult<ModDetail> {
        let cached = self
            .inner
            .caches
            .mod_detail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned();
        let mut detail = match cached {
            Some(d) => d,
            None => {
                let gen = self.generations().mods;
                let m = self.installed_mod(id)?;
                let detail = ModDetail {
                    summary: ModSummary::new(&m, false),
                    security_report: SecurityScanReport::scan_directory(&m.install_path),
                    integrity: m.integrity.verify_directory(&m.install_path),
                    manifest: m.manifest.clone(),
                    readme_md: m.readme_markdown.clone(),
                    changelog_md: m.changelog_markdown.clone(),
                    config_schema: m.config_schema.clone(),
                    saved_config: HashMap::new(),
                };
                // Skip caching if the mod index was invalidated while this was computed.
                self.with_gens(|g| {
                    if g.mods == gen {
                        self.inner
                            .caches
                            .mod_detail
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(id.to_string(), detail.clone());
                    }
                });
                detail
            }
        };
        // Per-profile state is small and read fresh so profile switches never serve stale values.
        detail.summary.enabled = self
            .active_profile()
            .is_ok_and(|p| p.enabled_mods.iter().any(|m| m == id));
        detail.saved_config = self
            .load_mod_settings()
            .unwrap_or_default()
            .mods
            .remove(id)
            .unwrap_or_default();
        Ok(detail)
    }

    pub fn mod_set_enabled(&self, id: &str, enabled: bool) -> QmResult<ModSummary> {
        self.ensure_not_running()?;
        let m = self.installed_mod(id)?;
        let mut profile = self.active_profile()?;
        if enabled {
            if !profile.enabled_mods.iter().any(|x| x == id) {
                profile.enabled_mods.push(id.to_string());
            }
        } else {
            profile.enabled_mods.retain(|x| x != id);
        }
        self.commit_active_profile(profile)?;
        Ok(ModSummary::new(&m, enabled))
    }

    /// Merge `values` into the active profile's mod settings (mirrored to the save folder).
    pub fn mod_config_set(
        &self,
        id: &str,
        values: HashMap<String, ConfigValue>,
    ) -> QmResult<HashMap<String, ConfigValue>> {
        self.ensure_not_running()?;
        self.installed_mod(id)?;
        let mut store = self.load_mod_settings()?;
        let entry = store.mods.entry(id.to_string()).or_default();
        entry.extend(values);
        let out = entry.clone();
        self.save_mod_settings(&store)?;
        Ok(out)
    }

    pub fn mod_config_reset(
        &self,
        id: &str,
        mode: ConfigResetMode,
    ) -> QmResult<HashMap<String, ConfigValue>> {
        self.ensure_not_running()?;
        let schema = self.installed_mod(id)?.config_schema;
        let mut store = self.load_mod_settings()?;
        match (mode, schema) {
            (ConfigResetMode::Defaults, Some(s)) => store.reset_mod_to_defaults(id, &s),
            (ConfigResetMode::Defaults, None) => {
                store.mods.remove(id);
            }
            (ConfigResetMode::Safe, Some(s)) => store.safe_reset_mod(id, &s),
            (ConfigResetMode::Safe, None) => {}
        }
        self.save_mod_settings(&store)?;
        Ok(store.mods.get(id).cloned().unwrap_or_default())
    }

    pub fn mod_import_inspect(&self, path: &Path) -> QmResult<Vec<ArchiveModCandidate>> {
        if !path.exists() {
            return Err(not_found_path(path));
        }
        self.paths()
            .inspect_archive_candidates(path, false)
            .or_kind(ErrorKind::InvalidInput)
    }

    /// Import in a cancellable `ModImport` job; the job output is the imported mod ids.
    pub fn mod_import(&self, req: ImportRequest) -> QmResult<JobId> {
        if !req.path.exists() {
            return Err(not_found_path(&req.path));
        }
        let svc = self.clone();
        Ok(self.jobs().spawn(JobKind::ModImport, move |ctx| {
            if ctx.is_cancelled() {
                return Err(QmError::new(ErrorKind::Cancelled, "Job cancelled"));
            }
            ctx.progress(0.1, "Importing");
            let summary = svc
                .paths()
                .import_mod_from_path_with_options(
                    &req.path,
                    None,
                    req.allow_unmanaged,
                    req.selected_mod_ids,
                    req.conflict_policy.map(|p| p.as_manager_str().to_string()),
                )
                .or_kind(ErrorKind::InvalidInput)?;
            let ids: Vec<String> = summary
                .imported_mods
                .iter()
                .map(|m| m.manifest.id.clone())
                .collect();
            ctx.progress(0.9, "Updating mod index");
            svc.after_mod_import(&ids)?;
            Ok(Some(ids.join(", ")))
        }))
    }

    /// Mods `ids` were (re)imported: invalidate the index and restage the active profile when it
    /// uses one of them (a replaced version dir is gone).
    pub(crate) fn after_mod_import(&self, ids: &[String]) -> QmResult<()> {
        self.invalidate(Scope::Mods);
        let uses_imported = self
            .active_profile()
            .is_ok_and(|p| p.enabled_mods.iter().any(|m| ids.contains(m)));
        if uses_imported && self.ensure_not_running().is_ok() {
            let active = self.paths().load_settings().active_profile_id;
            self.paths()
                .activate_profile(&active)
                .or_kind(ErrorKind::Io)?;
            self.after_active_profile_change();
        }
        Ok(())
    }

    /// Remove every installed version and all profile references to the mod.
    pub fn mod_uninstall(&self, id: &str) -> QmResult<()> {
        self.ensure_not_running()?;
        if !is_plain_id(id) || !self.paths().library_dir.join(id).is_dir() {
            return Err(QmError::new(
                ErrorKind::NotFound,
                format!("Mod '{id}' is not installed."),
            ));
        }
        let was_enabled = self
            .active_profile()
            .is_ok_and(|p| p.enabled_mods.iter().any(|m| m == id));
        self.paths().uninstall_mod(id).or_kind(ErrorKind::Io)?;
        self.invalidate(Scope::Mods);
        let restaged = if was_enabled {
            let active = self.paths().load_settings().active_profile_id;
            self.paths()
                .activate_profile(&active)
                .or_kind(ErrorKind::Io)
                .map(|_| ())
        } else {
            Ok(())
        };
        // Profiles were rewritten either way.
        self.after_active_profile_change();
        restaged
    }

    /// Request the `WarmMods` job that fills the mod index.
    pub fn warm_mods(&self) -> JobId {
        self.refresh(JobKind::WarmMods)
    }

    /// One `WarmMods` pass: sync the workspace into an empty library, then index the library.
    pub(crate) fn warm_mods_pass(&self, ctx: &JobCtx) -> QmResult<Option<String>> {
        let gen = self.generations().mods;
        let settings = self.paths().load_settings();
        if let Some(ws) = settings.workspace_mods_dir.as_deref() {
            if library_is_empty(&self.paths().library_dir) {
                ctx.progress(0.2, "Syncing workspace mods");
                // A failed sync must not leave the index empty: log it and index what is there.
                if let Err(e) = self.paths().sync_workspace_mods(ws) {
                    tracing::warn!("workspace mod sync from {} failed: {e}", ws.display());
                }
            }
        }
        ctx.progress(0.5, "Scanning mod library");
        let mods = self.paths().list_installed_mods();
        let stored = self.with_gens(|g| {
            if g.mods != gen {
                return false;
            }
            self.inner.caches.mods.set(Arc::new(mods));
            true
        });
        if stored {
            self.notify(Scope::Mods);
            self.notify(Scope::LoadOrder);
        }
        // Not stored: the library changed mid-pass and a newer pass is already requested.
        Ok(None)
    }

    /// Latest installed version of `id`, from the index when cached.
    pub(crate) fn installed_mod(&self, id: &str) -> QmResult<InstalledMod> {
        let found = match self.inner.caches.mods.get() {
            Some(mods) => mods.iter().find(|m| m.manifest.id == id).cloned(),
            None => self.paths().get_mod_version(id, None),
        };
        found.ok_or_else(|| {
            QmError::new(ErrorKind::NotFound, format!("Mod '{id}' is not installed."))
        })
    }

    pub(crate) fn installed_mods(&self) -> Arc<Vec<InstalledMod>> {
        self.inner
            .caches
            .mods
            .get()
            .unwrap_or_else(|| Arc::new(self.paths().list_installed_mods()))
    }

    fn profile_settings_path(&self) -> PathBuf {
        let active = self.paths().load_settings().active_profile_id;
        self.paths().profiles_dir.join(active).join(SETTINGS_FILE)
    }

    /// Active profile settings, falling back to the live save folder copy. A file that exists
    /// but cannot be parsed is an error, so writers never overwrite other mods' values.
    fn load_mod_settings(&self) -> QmResult<ModSettingsStore> {
        let path = self.profile_settings_path();
        let source = if path.is_file() {
            Some(path)
        } else {
            self.paths()
                .load_settings()
                .save_dir_path
                .map(|d| d.join(SETTINGS_FILE))
                .filter(|p| p.is_file())
        };
        match source {
            None => Ok(ModSettingsStore::default()),
            Some(p) => ModSettingsStore::from_path(&p).map_err(|e| {
                QmError::new(
                    ErrorKind::Io,
                    "Mod settings file could not be read; it was not changed.",
                )
                .with_details(format!("{}: {e}", p.display()))
            }),
        }
    }

    fn save_mod_settings(&self, store: &ModSettingsStore) -> QmResult<()> {
        store
            .save_to_path(&self.profile_settings_path())
            .or_kind(ErrorKind::Io)?;
        if let Some(dir) = self.paths().load_settings().save_dir_path {
            if let Err(e) = store.save_to_path(&dir.join(SETTINGS_FILE)) {
                tracing::warn!("mirroring mod settings to the save folder failed: {e}");
            }
        }
        Ok(())
    }
}

fn library_is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).map_or(true, |rd| {
        !rd.filter_map(Result::ok)
            .any(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
    })
}

fn is_plain_id(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\', ':'])
}

fn not_found_path(path: &Path) -> QmError {
    QmError::new(
        ErrorKind::NotFound,
        format!("Path does not exist: {}", path.display()),
    )
}
