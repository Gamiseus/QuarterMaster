//! Typed request and response structs for the app, profile, mod and load-order operations.
use crate::cache::Loadable;
use qm_manager::{
    DetectedConflict, InstalledMod, LaunchMethod, LauncherSettings, ModProfile, ProfileDiff,
    ProfileSeparator, SaveCompatibilityReport, SimulatedFileResult,
};
use qm_schema::{
    ConfigValue, IntegrityVerifyResult, ModConfigSchema, ModManifest, SecurityScanReport,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AppInfo {
    pub version: String,
    pub data_dir: PathBuf,
    pub is_portable: bool,
    pub game: Loadable<Option<GameInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct GameInfo {
    pub version: String,
    pub exe_sha256: String,
    pub lua_file_count: u32,
}

/// `LauncherSettings` without the token itself.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SettingsView {
    pub game_exe_path: Option<PathBuf>,
    pub save_dir_path: Option<PathBuf>,
    pub workspace_mods_dir: Option<PathBuf>,
    pub theme: String,
    pub default_launch_method: LaunchMethod,
    pub opt_in_prereleases: bool,
    pub auto_backup_saves: bool,
    pub max_save_backups: u32,
    pub disable_console_on_launch: bool,
    pub first_run_completed: bool,
    pub active_profile_id: String,
    pub active_save_profile_id: String,
    pub portable_mode: bool,
    pub github_token_set: bool,
    pub auto_update_mods: HashMap<String, bool>,
    pub last_known_exe_sha256: Option<String>,
}

impl From<&LauncherSettings> for SettingsView {
    fn from(s: &LauncherSettings) -> Self {
        Self {
            game_exe_path: s.game_exe_path.clone(),
            save_dir_path: s.save_dir_path.clone(),
            workspace_mods_dir: s.workspace_mods_dir.clone(),
            theme: s.theme.clone(),
            default_launch_method: s.default_launch_method,
            opt_in_prereleases: s.opt_in_prereleases,
            auto_backup_saves: s.auto_backup_saves,
            max_save_backups: u32::try_from(s.max_save_backups).unwrap_or(u32::MAX),
            disable_console_on_launch: s.disable_console_on_launch,
            first_run_completed: s.first_run_completed,
            active_profile_id: s.active_profile_id.clone(),
            active_save_profile_id: s.active_save_profile_id.clone(),
            portable_mode: s.portable_mode,
            github_token_set: s.github_token.as_deref().is_some_and(|t| !t.is_empty()),
            auto_update_mods: s.auto_update_mods.clone(),
            last_known_exe_sha256: s.last_known_exe_sha256.clone(),
        }
    }
}

/// Fields left `None` are unchanged. `github_token: Some("")` clears the token.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SettingsPatch {
    pub game_exe_path: Option<PathBuf>,
    pub save_dir_path: Option<PathBuf>,
    pub workspace_mods_dir: Option<PathBuf>,
    pub theme: Option<String>,
    pub default_launch_method: Option<LaunchMethod>,
    pub opt_in_prereleases: Option<bool>,
    pub auto_backup_saves: Option<bool>,
    pub max_save_backups: Option<u32>,
    pub disable_console_on_launch: Option<bool>,
    pub github_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RevealKind {
    Logs,
    Library,
    GameDir,
    ProfileDir,
    SaveDir,
    DataDir,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled_count: u32,
    pub is_active: bool,
    pub linked_save_profile_id: String,
    pub updated_at: String,
}

impl ProfileSummary {
    pub(crate) fn new(p: &ModProfile, active_id: &str) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
            enabled_count: u32::try_from(p.enabled_mods.len()).unwrap_or(u32::MAX),
            is_active: p.id == active_id,
            linked_save_profile_id: p.linked_save_profile_id.clone(),
            updated_at: p.updated_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct NewProfile {
    pub name: String,
    pub id: Option<String>,
    pub clone_from: Option<String>,
    pub linked_save_profile_id: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SwitchPreview {
    pub added_mods: Vec<String>,
    pub removed_mods: Vec<String>,
    pub diff: ProfileDiff,
    pub save_compatibility: SaveCompatibilityReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub category: Option<String>,
    pub enabled: bool,
    pub patch_count: u32,
    pub installed_versions: Vec<String>,
    pub is_workspace_linked: bool,
    pub has_config: bool,
}

impl ModSummary {
    pub(crate) fn new(m: &InstalledMod, enabled: bool) -> Self {
        Self {
            id: m.manifest.id.clone(),
            name: m.manifest.name.clone(),
            version: m.manifest.version.clone(),
            authors: m.manifest.authors.clone(),
            category: m.manifest.category.clone(),
            enabled,
            patch_count: u32::try_from(m.patch_count).unwrap_or(u32::MAX),
            installed_versions: m.installed_versions.clone(),
            is_workspace_linked: m.is_workspace_linked,
            has_config: m.config_schema.is_some(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModDetail {
    pub summary: ModSummary,
    pub manifest: ModManifest,
    pub readme_md: Option<String>,
    pub changelog_md: Option<String>,
    pub config_schema: Option<ModConfigSchema>,
    pub saved_config: HashMap<String, ConfigValue>,
    pub security_report: SecurityScanReport,
    pub integrity: IntegrityVerifyResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ConfigResetMode {
    Defaults,
    Safe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    Upgrade,
    Replace,
    InstallAlongside,
}

impl ConflictPolicy {
    /// The policy string `import_mod_from_path_with_options` understands.
    pub(crate) fn as_manager_str(self) -> &'static str {
        match self {
            ConflictPolicy::Upgrade => "upgrade",
            ConflictPolicy::Replace => "replace",
            ConflictPolicy::InstallAlongside => "alongside",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ImportRequest {
    pub path: PathBuf,
    pub allow_unmanaged: bool,
    pub selected_mod_ids: Option<Vec<String>>,
    pub conflict_policy: Option<ConflictPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LoadOrderUpdate {
    pub mod_priorities: HashMap<String, i32>,
    pub user_load_after: HashMap<String, Vec<String>>,
    pub locked_mods: Vec<String>,
    pub separators: Vec<ProfileSeparator>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SimulationOverview {
    pub total_patches: u32,
    pub applied_patches: u32,
    pub broken_anchors: u32,
    pub conflicts: Vec<DetectedConflict>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FileDiff {
    pub result: SimulatedFileResult,
    pub original_lines: Vec<String>,
}
