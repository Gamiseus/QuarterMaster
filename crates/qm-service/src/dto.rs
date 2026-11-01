//! Typed request and response structs for every service operation.
use crate::cache::Loadable;
use qm_core::handshake::PatchExecutionRecord;
use qm_manager::{
    DetectedConflict, InstalledMod, LaunchMethod, LauncherSettings, ModProfile, ProfileDiff,
    ProfileSeparator, SaveCompatibilityReport, SaveProfile, SimulatedFileResult,
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
    pub load_priority: i32,
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
            load_priority: m.manifest.load_priority,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PatchState {
    Applied,
    Broken,
    Disabled,
    Skipped,
}

/// One patch of a mod as seen by the cached simulation.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModPatchStatus {
    pub patch_id: String,
    pub name: Option<String>,
    pub target_file: String,
    pub state: PatchState,
    pub conflicts: Vec<DetectedConflict>,
}

/// `Default` clears the profile's launch method override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMethodChoice {
    Default,
    SteamUrl,
    DirectExe,
    Vanilla,
}

/// Fields left `None` are unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct ProfileSettingsPatch {
    pub launch_method: Option<LaunchMethodChoice>,
    pub custom_launch_args: Option<Vec<String>>,
    pub disable_console: Option<bool>,
    pub linked_save_profile_id: Option<String>,
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

// ---- Saves ----

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SaveSlotSummary {
    pub profile: SaveProfile,
    pub is_active: bool,
}

// ---- Health (mirrors `check_system_health`) ----

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct HealthReport {
    pub dll_present: bool,
    pub is_game_dir_modded: bool,
    pub source_dll_ready: bool,
    pub dll_sha256: Option<String>,
    pub source_dll_sha256: Option<String>,
    pub dll_matches_build: bool,
    pub game_running_pid: Option<u32>,
    pub game_update_detected: Option<GameUpdateDetected>,
    pub overwrite_files: Vec<String>,
    pub steam_cloud_warning: Option<String>,
    /// `None` when `patch-report.json` is missing or unreadable.
    #[serde(deserialize_with = "lenient")]
    pub patch_report: Option<HealthPatchReport>,
    pub recent_logs: Vec<HealthLogLine>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct GameUpdateDetected {
    pub previous_sha256: String,
    pub current_sha256: String,
}

/// `Mods/qm/patch-report.json` as written by winmm.dll.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct HealthPatchReport {
    pub qm_version: String,
    pub timestamp: String,
    pub profile_id: Option<String>,
    pub total_patches_applied: u32,
    pub total_warnings: u32,
    pub total_failures: u32,
    pub records: Vec<PatchExecutionRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct HealthLogLine {
    pub level: String,
    pub line: String,
    pub file: String,
}

/// Any value that does not deserialize as `T` (including `null`) becomes `None`.
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

/// Non-negative integers clamp to `u32::MAX`; anything else (negative, fractional, non-number)
/// becomes 0, so one odd count cannot fail the whole catalog.
fn saturating_u32<'de, D>(d: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_u64()
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)))
}

// ---- Updates (mirrors `check_for_updates`) ----

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct UpdatesReport {
    pub checked_at: String,
    pub github_token_configured: bool,
    pub rate_limit_mode: String,
    pub launcher: LauncherUpdateInfo,
    pub mods: Vec<ModUpdateInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct LauncherUpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub opt_in_prereleases: bool,
    pub signature_verification: String,
    pub rollback_available: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct ModUpdateInfo {
    pub mod_id: String,
    pub name: String,
    pub installed_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub update_key: String,
    pub pinned_source_url: Option<String>,
    pub source_changed_warning: bool,
    pub auto_update: bool,
    pub changelog: String,
}

// ---- Catalog (mirrors `get_community_catalog`) ----

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct CatalogResult {
    pub catalog: Vec<CatalogEntry>,
    pub total_count: u32,
    pub is_cached: bool,
    pub query: String,
    pub rate_limited: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub repo_full_name: String,
    pub owner: String,
    pub description: String,
    #[serde(deserialize_with = "saturating_u32")]
    pub stargazers_count: u32,
    #[serde(deserialize_with = "saturating_u32")]
    pub forks_count: u32,
    pub updated_at: String,
    pub html_url: String,
    pub topics: Vec<String>,
    pub license: Option<String>,
    pub source_type: String,
    pub latest_release_tag: Option<String>,
    pub download_url: Option<String>,
    pub download_asset_name: Option<String>,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub update_key: Option<String>,
}

/// Local filter over the cached default catalog. `None`, empty, or `"all"` means no filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct CatalogQuery {
    pub text: Option<String>,
    pub topic: Option<String>,
    pub source: Option<String>,
}
