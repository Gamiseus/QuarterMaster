//! App info, settings, onboarding and reveal.
use crate::cache::Loadable;
use crate::dto::{AppInfo, GameInfo, RevealKind, SettingsPatch, SettingsView};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::service::Service;
use qm_manager::storage::DetectedPaths;
use qm_manager::LauncherSettings;
use std::path::{Path, PathBuf};

const MAX_BACKUPS: std::ops::RangeInclusive<u32> = 1..=100;

impl Service {
    pub fn app_info(&self) -> AppInfo {
        let game = match self.inner.caches.vanilla.get() {
            None => Loadable::Loading,
            Some(v) => Loadable::Ready(v.info.as_ref().map(|i| GameInfo {
                version: i.game_version.clone(),
                exe_sha256: i.exe_sha256.clone(),
                lua_file_count: u32::try_from(i.lua_file_count).unwrap_or(u32::MAX),
            })),
        };
        AppInfo {
            version: qm_core::QM_VERSION.to_string(),
            data_dir: self.paths().root_dir.clone(),
            is_portable: self.paths().is_portable,
            game,
        }
    }

    pub fn settings_get(&self) -> SettingsView {
        SettingsView::from(&self.paths().load_settings())
    }

    pub fn settings_update(&self, patch: SettingsPatch) -> QmResult<SettingsView> {
        self.apply_settings(patch, false)
    }

    pub fn paths_auto_detect(&self) -> DetectedPaths {
        self.paths().auto_detect_all_paths()
    }

    /// Apply the onboarding answers, mark onboarding done and warm the caches.
    pub fn onboarding_complete(&self, patch: SettingsPatch) -> QmResult<SettingsView> {
        let view = self.apply_settings(patch, true)?;
        self.warm_mods();
        Ok(view)
    }

    /// One of a fixed set of folders, for the shell to open.
    pub fn reveal_path(&self, kind: RevealKind) -> QmResult<PathBuf> {
        let paths = self.paths();
        let settings = paths.load_settings();
        let (path, owned) = match kind {
            RevealKind::Logs => (paths.logs_dir.clone(), true),
            RevealKind::Library => (paths.library_dir.clone(), true),
            RevealKind::DataDir => (paths.root_dir.clone(), true),
            RevealKind::ProfileDir => (paths.profiles_dir.join(&settings.active_profile_id), true),
            RevealKind::GameDir => (
                settings
                    .game_exe_path
                    .as_deref()
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
                    .ok_or_else(|| not_set("The game folder"))?,
                false,
            ),
            RevealKind::SaveDir => (
                settings
                    .save_dir_path
                    .ok_or_else(|| not_set("The save folder"))?,
                false,
            ),
        };
        if owned {
            std::fs::create_dir_all(&path).or_kind(ErrorKind::Io)?;
        }
        if !path.is_dir() {
            return Err(QmError::new(
                ErrorKind::NotFound,
                format!("Folder not found: {}", path.display()),
            ));
        }
        Ok(path)
    }

    fn apply_settings(
        &self,
        patch: SettingsPatch,
        complete_onboarding: bool,
    ) -> QmResult<SettingsView> {
        let mut s = self.paths().load_settings();
        validate(&patch)?;
        if let Some(exe) = &patch.game_exe_path {
            if s.game_exe_path.as_ref() != Some(exe) {
                self.ensure_not_running()?;
            }
        }
        apply(&mut s, patch);
        if complete_onboarding {
            s.first_run_completed = true;
        }
        self.paths().save_settings(&s).or_kind(ErrorKind::Io)?;
        // Also requests WarmVanilla, which rereads the game path.
        self.invalidate(Scope::Settings);
        Ok(SettingsView::from(&s))
    }
}

fn validate(p: &SettingsPatch) -> QmResult<()> {
    if let Some(exe) = &p.game_exe_path {
        let named_mnb = exe
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("mnb.exe"));
        if !named_mnb || !exe.is_file() {
            return Err(invalid(format!(
                "Select mnb.exe from the Mud and Blood folder ({} is not it).",
                exe.display()
            )));
        }
    }
    for (label, dir) in [
        ("Save folder", &p.save_dir_path),
        ("Workspace mods folder", &p.workspace_mods_dir),
    ] {
        if let Some(d) = dir {
            if !d.is_dir() {
                return Err(invalid(format!("{label} not found: {}", d.display())));
            }
        }
    }
    if let Some(n) = p.max_save_backups {
        if !MAX_BACKUPS.contains(&n) {
            return Err(invalid("Keep between 1 and 100 save backups.".to_string()));
        }
    }
    Ok(())
}

fn apply(s: &mut LauncherSettings, p: SettingsPatch) {
    if let Some(v) = p.game_exe_path {
        s.game_exe_path = Some(v);
    }
    if let Some(v) = p.save_dir_path {
        s.save_dir_path = Some(v);
    }
    if let Some(v) = p.workspace_mods_dir {
        s.workspace_mods_dir = Some(v);
    }
    if let Some(v) = p.theme {
        s.theme = v;
    }
    if let Some(v) = p.default_launch_method {
        s.default_launch_method = v;
    }
    if let Some(v) = p.opt_in_prereleases {
        s.opt_in_prereleases = v;
    }
    if let Some(v) = p.auto_backup_saves {
        s.auto_backup_saves = v;
    }
    if let Some(v) = p.max_save_backups {
        s.max_save_backups = v as usize;
    }
    if let Some(v) = p.disable_console_on_launch {
        s.disable_console_on_launch = v;
    }
    if let Some(v) = p.github_token {
        s.github_token = if v.is_empty() { None } else { Some(v) };
    }
}

fn invalid(msg: String) -> QmError {
    QmError::new(ErrorKind::InvalidInput, msg)
}

fn not_set(what: &str) -> QmError {
    QmError::new(ErrorKind::NotFound, format!("{what} is not set."))
}
