//! Typed request and response structs for every service operation.
use crate::cache::Loadable;
use qm_core::handshake::{FixBinding, LoadError, PatchExecutionRecord, PatchOutcomeStatus};
use qm_core::patch::fingerprint::PatchSnapshot;
use qm_manager::fixes::{ApprovedImpact, FixSource, Provenance};
use qm_manager::{
    DetectedConflict, InstalledMod, LaunchMethod, LauncherSettings, ModKind, ModProfile,
    ProfileDiff, ProfileSeparator, SaveCompatibilityReport, SaveProfile, SimulatedFileResult,
    StackBlock,
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
    pub debug_mode: bool,
    pub dev_commands: bool,
    pub first_run_completed: bool,
    pub active_profile_id: String,
    pub active_save_profile_id: String,
    pub portable_mode: bool,
    pub auto_update_mods: HashMap<String, bool>,
    pub last_known_exe_sha256: Option<String>,
    pub auto_check_updates: bool,
    pub auto_install_updates: bool,
    pub color_vision: String,
    pub high_contrast: bool,
    pub text_scale: f32,
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
            debug_mode: s.debug_mode,
            dev_commands: s.dev_commands,
            first_run_completed: s.first_run_completed,
            active_profile_id: s.active_profile_id.clone(),
            active_save_profile_id: s.active_save_profile_id.clone(),
            portable_mode: s.portable_mode,
            auto_update_mods: s.auto_update_mods.clone(),
            last_known_exe_sha256: s.last_known_exe_sha256.clone(),
            auto_check_updates: s.auto_check_updates,
            auto_install_updates: s.auto_install_updates,
            color_vision: s.color_vision.clone(),
            high_contrast: s.high_contrast,
            text_scale: s.text_scale,
        }
    }
}

/// Fields left `None` are unchanged.
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
    pub debug_mode: Option<bool>,
    pub dev_commands: Option<bool>,
    pub auto_check_updates: Option<bool>,
    pub auto_install_updates: Option<bool>,
    /// `Some(true)` unsets the workspace mods folder (`workspace_mods_dir` is ignored then).
    pub clear_workspace_mods_dir: Option<bool>,
    pub color_vision: Option<String>,
    pub high_contrast: Option<bool>,
    pub text_scale: Option<f32>,
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
    /// `<data>/cache/diagnostics`, where bug report bundles are written.
    Diagnostics,
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
    /// The built-in Vanilla profile: no mods, no edits.
    pub is_builtin: bool,
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
            is_builtin: p.id == qm_manager::VANILLA_ID,
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
    pub kind: ModKind,
    /// Set for a faction mod: who it is and which side it replaces.
    pub faction: Option<qm_schema::faction::FactionHeader>,
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
            kind: m.kind,
            faction: qm_schema::faction::read_faction_header(&m.install_path),
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
    /// A module replaces the patch's file, so the game never runs it (G1).
    NotRun,
}

/// One patch of a mod as seen by the cached simulation.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModPatchStatus {
    pub patch_id: String,
    pub name: Option<String>,
    pub target_file: String,
    pub state: PatchState,
    pub conflicts: Vec<DetectedConflict>,
    /// The compatibility mods that changed this patch, as the engine bound them.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[specta(optional)]
    pub changed_by: Vec<String>,
}

/// What a mod's compatibility overrides did, and its rungs that are never used (§11.6).
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct CompatChangesView {
    pub applied: Vec<qm_core::handshake::OverrideReport>,
    /// Every override that does not apply, with why: not required, waiting, missing, changed
    /// since, replaced by a later mod or by the profile, or unreadable.
    pub rejected: Vec<qm_core::handshake::OverrideReport>,
    /// Rungs of the mod's patches, or from its overrides, that are never used (D33).
    pub rejected_rungs: Vec<qm_core::handshake::InvalidRung>,
}

/// Which fixes to save as a compatibility patch, and its name (§8.2).
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct CompatExportRequest {
    pub fix_ids: Vec<String>,
    /// The mod's name; empty: one is made from the mods it changes.
    #[serde(default)]
    pub name: String,
}

/// A mod the compatibility patch requires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CompatRequirement {
    pub mod_id: String,
    pub name: String,
    pub version: Option<String>,
    /// The version range written to mod.json: ">=<version>", or "*" with no version.
    pub range: String,
    /// Why, e.g. "it places code next to GCore's change".
    pub reasons: Vec<String>,
}

/// One file of the compatibility patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CompatFile {
    pub name: String,
    pub text: String,
}

/// Whether the game code with the compatibility patch in place of the fixes is the same.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompatEquivalence {
    Equivalent,
    Differs {
        files: Vec<String>,
    },
    /// Nothing to check: no fix can be saved.
    NotChecked {
        reason: String,
    },
}

/// A fix the compatibility patch cannot include, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CompatRefusal {
    pub fix_id: String,
    pub reason: String,
}

/// What "Save as compatibility patch" would make (§8.2). Nothing is written.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CompatExportPreview {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub requires: Vec<CompatRequirement>,
    pub load_after: Vec<String>,
    /// mod.json, qm.toml and README.txt.
    pub files: Vec<CompatFile>,
    pub equivalence: CompatEquivalence,
    /// The fixes it includes, in ledger order.
    pub included: Vec<String>,
    pub refused: Vec<CompatRefusal>,
}

/// What `compat_export` did.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CompatExportResult {
    pub mod_id: String,
    pub version: String,
    /// Where the mod is in the library, for display only.
    pub shown_path: String,
    pub removed_fix_ids: Vec<String>,
}

/// `Default` clears the profile's launch method override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMethodChoice {
    Default,
    SteamUrl,
    DirectExe,
}

/// Fields left `None` are unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct ProfileSettingsPatch {
    pub launch_method: Option<LaunchMethodChoice>,
    pub custom_launch_args: Option<Vec<String>>,
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
    /// The files the autoloader picks up from `assets/`, in path order (empty for most mods).
    pub asset_files: Vec<AssetFile>,
}

/// One file under a mod's `assets/` folder.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AssetFile {
    /// `assets/...`, with forward slashes.
    pub path: String,
    /// Whether it replaces something the game has; `None` until the game files are read.
    pub overrides_vanilla: Option<bool>,
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
    /// One summary per touched file, in `files` order (the Patches tab's file list).
    pub file_stats: Vec<FileStat>,
    /// Whether patched files were compiled with the game's Lua.
    pub compile_check: qm_manager::CompileCheck,
    /// load-order.json could not be used, so the game would load no mods (D16). Blocks launch.
    pub plan_error: Option<String>,
    /// Plan entries that were skipped (D17).
    pub plan_warnings: Vec<String>,
    /// Mods that fail to load, each skipped with the mods that require it (D15). Blocks launch.
    pub load_errors: Vec<LoadError>,
    /// Plan fixes that do not apply as written, such as an adjusted copy waiting for its mod
    /// (D19). Does not block launch.
    pub fix_bindings: Vec<FixBinding>,
    /// Order rules a stronger rule replaced or ignored (§3.9).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[specta(optional)]
    pub overridden_rules: Vec<String>,
    /// Names in order rules that match no loaded patch.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[specta(optional)]
    pub unknown_ids: Vec<String>,
    /// Rungs suspended to break an order cycle; their patches use their own lines.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[specta(optional)]
    pub suspended_rungs: Vec<qm_core::handshake::SuspendedRung>,
    /// Rungs that are never used, with why.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[specta(optional)]
    pub invalid_rungs: Vec<qm_core::handshake::InvalidRung>,
    /// What became of each compatibility mod's overrides (§8.1).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[specta(optional)]
    pub overrides: Vec<qm_core::handshake::OverrideReport>,
    /// What the newer line rules would change in the active profile, when it has not chosen
    /// yet and they change something (the `stacking_v2` notice, D51).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[specta(optional)]
    pub line_rules_change: Option<qm_manager::line_rules::LineRulesDiff>,
    /// Patches that fail because code they anchor on did not apply, by root (G10).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[specta(optional)]
    pub failure_groups: Vec<qm_manager::failure_groups::FailureGroup>,
    /// Patches that anchor on another patch's code (the Order view's dashed edges, G10).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[specta(optional)]
    pub inferred_edges: Vec<qm_manager::failure_groups::InferredEdge>,
}

/// What the patches do to one game file.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FileStat {
    pub target_file: String,
    pub original_lines: u32,
    pub patched_lines: u32,
    /// Distinct patches with a record on this file (disabled ones included).
    pub patches: u32,
    pub mods: Vec<String>,
    pub broken: u32,
    pub conflicts: u32,
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
    /// Set when the sidecar and the save file disagree about mods; the stricter answer wins.
    pub tamper_note: Option<String>,
    /// Values in the save from mods no profile on this slot loads (Phase E D86), from the
    /// save's stamp. None when there are none or the save has no stamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leftover: Option<qm_manager::save_fields::LeftoverFields>,
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
    /// The game folder `winmm.dll` hash is a known QuarterMaster build (`false` when absent).
    pub dll_is_known: bool,
    pub game_running_pid: Option<u32>,
    pub game_update_detected: Option<GameUpdateDetected>,
    pub overwrite_files: Vec<String>,
    pub steam_cloud_warning: Option<String>,
    /// `None` when `patch-report.json` is missing or unreadable.
    #[serde(deserialize_with = "lenient")]
    pub patch_report: Option<HealthPatchReport>,
    /// Why `patch-report.json` could not be read or parsed; None when it is fine or missing.
    #[serde(deserialize_with = "lenient")]
    pub patch_report_error: Option<String>,
    pub recent_logs: Vec<HealthLogLine>,
    /// The active profile's newest run log entry (D67, D72); it outlives the game folder's
    /// report. None when the profile has no run yet.
    #[serde(deserialize_with = "lenient")]
    pub last_run: Option<qm_manager::runs::RunEntry>,
    /// A test run's plan (a trial or a run without mods) is still staged for the active
    /// profile (D68). Its own plan is staged again once the game is not running.
    #[serde(deserialize_with = "lenient")]
    pub test_plan: Option<qm_manager::runs::StagedTestPlan>,
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
    /// Records whose match touched QuarterMaster's own lines, refused ones included (D42).
    pub builtin_matches: u32,
    pub records: Vec<PatchExecutionRecord>,
    /// The game could not use load-order.json and loaded no mods (D16).
    #[serde(deserialize_with = "lenient")]
    pub plan_error: Option<String>,
    /// Plan entries the game skipped (D17).
    #[serde(deserialize_with = "lenient_vec")]
    pub plan_warnings: Vec<String>,
    /// Mods the game skipped because they failed to load, with their requirers (D15).
    #[serde(deserialize_with = "lenient_vec")]
    pub load_errors: Vec<LoadError>,
    /// Plan fixes that did not apply as written, such as a waiting adjusted copy (D19).
    #[serde(deserialize_with = "lenient_vec")]
    pub fix_bindings: Vec<FixBinding>,
    /// The game could not use launch-config.json, loaded no mods and used its default save
    /// identity (D65).
    #[serde(deserialize_with = "lenient")]
    pub launch_config_error: Option<String>,
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

/// The entries of a list that deserialize as `T`; anything that is not a list is empty, so a
/// newer DLL's entry cannot hide the rest of the report.
fn lenient_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|x| serde_json::from_value(x).ok())
            .collect(),
        _ => Vec::new(),
    })
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
    /// `normal`, or `busy` when github.com answered 429 and some mods were not checked.
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
    /// Release asset to install, from the last real GitHub check (`None` when unknown).
    pub download_url: Option<String>,
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
    /// The mod index site could not be reached; the catalog is the last good copy.
    pub offline: bool,
    /// When the copy in use was last confirmed current.
    pub copy_from: Option<String>,
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
    /// `registered` (submitted to the site) or `discovered` (found by topic); empty for Nexus.
    pub listing: String,
}

/// Local filter over the cached default catalog. `None`, empty, or `"all"` means no filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct CatalogQuery {
    pub text: Option<String>,
    pub topic: Option<String>,
    pub source: Option<String>,
}

/// Where a patch sits among the patches that touch one file, in application order.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PatchFilePosition {
    pub target_file: String,
    /// 0-based position in that file's order.
    pub index: u32,
    /// How many patches touch that file.
    pub count: u32,
}

/// Everything the patch inspector shows about one patch (plan 2d Task 1).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PatchDetail {
    pub patch_id: String,
    pub name: Option<String>,
    pub mod_id: String,
    /// The patch file it came from, relative to the profile's `Mods` folder.
    pub source_file: String,
    /// Effective priority after mod and profile overrides.
    pub priority: i32,
    /// The full definition: type, target, position, pattern or regex, payload, times, on_failure.
    pub definition: qm_core::patch::Patch,
    pub positions: Vec<PatchFilePosition>,
    /// The author's `before` / `after` lists.
    pub declared_before: Vec<String>,
    pub declared_after: Vec<String>,
    /// The active profile's order rules that name this patch.
    pub profile_rules: Vec<qm_core::patch::order::OrderRule>,
    pub requires: Vec<String>,
    /// Ids of the patches whose `requires` name this one.
    pub required_by: Vec<String>,
    pub disabled: bool,
    /// For a glue patch, the patch it replaces.
    pub replaces_patch_id: Option<String>,
    /// For a glue patch, its `base_mod_id`: a copy of it records the same mod (D59).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[specta(optional)]
    pub base_mod_id: Option<String>,
    /// For an `at` pattern patch, how it runs: as one replacement or as separate edits (§6.6).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[specta(optional)]
    pub runs_as: Option<qm_manager::RunsAsVerdict>,
}

/// A what-if change set for `simulation_preview`; nothing in it is saved.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct PreviewChanges {
    #[serde(default)]
    pub disable: Vec<String>,
    #[serde(default)]
    pub enable: Vec<String>,
    /// Each replaces the profile's rule for the same patch, as `patch_order_set` would.
    #[serde(default)]
    pub order_rules: Vec<qm_core::patch::order::OrderRule>,
    #[serde(default)]
    pub glue_patches: Vec<qm_core::handshake::GluePatchEntry>,
    /// The stacking mode to preview, as `stacking_set` would set it.
    #[serde(default)]
    pub stacking: Option<qm_core::handshake::StackMode>,
    /// Fixes to add, as `fix_create` would add them. An unmigrated profile is migrated in
    /// memory first (D38).
    #[serde(default)]
    pub add_fixes: Vec<FixDraft>,
    /// Ids of fixes to undo, each alone as `fix_suggestion_apply` undoes them (later fixes on
    /// the same patch stay). Done before `add_fixes`.
    #[serde(default)]
    pub remove_fixes: Vec<String>,
    /// The file whose patched result to return.
    #[serde(default)]
    pub focus_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PreviewResult {
    pub file: Option<SimulatedFileResult>,
    pub conflicts: Vec<DetectedConflict>,
    /// Ids of current conflicts the changes would remove. Empty when the changes would stop a
    /// file from compiling: such a change resolves nothing.
    pub resolved: Vec<String>,
    /// Ids of conflicts the changes would add.
    pub new_conflicts: Vec<String>,
    /// Everything else the changes would do.
    pub impact: PreviewImpact,
}

/// What a previewed change does beyond the conflict it targets.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct PreviewImpact {
    /// Game files whose patched text would change.
    pub files_changed: Vec<String>,
    /// Patches whose outcome or match count would change.
    pub patch_changes: Vec<PatchChange>,
    /// Files that compile now and would not after the change.
    pub compile_errors_new: Vec<String>,
    /// Files that do not compile now and still would not.
    pub compile_errors_existing: Vec<String>,
    /// Files that do not compile now and would after the change.
    pub compile_errors_fixed: Vec<String>,
    /// Ids of new placement warnings (code moved, wrapped or commented out by another mod).
    pub placement_new: Vec<String>,
    /// The change would stop a file from compiling; the UI does not offer Apply.
    pub breaks_file: bool,
    /// Shared lines whose code would run in a different order (§5.5).
    #[serde(default)]
    pub stack_changes: Vec<StackChange>,
    /// Order rules that run a mod's patch before a mod it depends on (D39).
    #[serde(default)]
    pub relation_warnings: Vec<RelationWarning>,
    /// Places where a patch would use its own line instead of the one a fix puts it on.
    #[serde(default)]
    pub fallbacks_new: Vec<FallbackNotice>,
    /// Places that use their own line now and would not after the change.
    #[serde(default)]
    pub fallbacks_fixed: Vec<FallbackNotice>,
    /// Fixes whose status would change (a fix the change adds or removes has no status on
    /// that side).
    #[serde(default)]
    pub fix_status_changes: Vec<FixStatusChange>,
    /// Sides whose active faction would change (D73): a new faction resets that side's
    /// unlocks in the save.
    #[serde(default)]
    pub faction_changes: Vec<FactionChange>,
}

/// A side whose active faction a change would switch (D73).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FactionChange {
    /// "allied" | "axis".
    pub side: String,
    /// The faction now; None: the game's own.
    pub before: Option<FactionPick>,
    /// The faction after the change; None: the game's own.
    pub after: Option<FactionPick>,
}

/// The faction a mod gives a side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FactionPick {
    pub mod_id: String,
    pub faction_id: String,
    pub name: String,
}

/// A place where an anchored patch uses its own line, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FallbackNotice {
    pub file: String,
    pub patch_id: String,
    /// 1-based line of the place in the game's file, as the file shows it (0 when unknown).
    pub site_line: u32,
    pub reason: String,
}

/// A fix whose status a change would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FixStatusChange {
    pub fix_id: String,
    pub before: Option<FixStatus>,
    pub after: Option<FixStatus>,
}

/// An order rule that runs `mod_id`'s patch before a patch of `depends_on`, which `mod_id`
/// requires or loads after (D39). It warns; it does not block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RelationWarning {
    /// The rule's patch.
    pub rule: String,
    pub mod_id: String,
    pub depends_on: String,
    pub text: String,
}

/// A shared line whose code would run in a different order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StackChange {
    pub file: String,
    /// 1-based, as the file shows it after the change.
    pub line: u32,
    pub side: qm_core::patch::linemap::Side,
    pub anchor_text: String,
    /// Top to bottom, now and after the change.
    pub before: Vec<StackBlock>,
    pub after: Vec<StackBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PatchChange {
    pub patch_id: String,
    pub mod_id: String,
    pub target_file: String,
    /// `None`: the patch has no record on this file (before or after).
    pub before: Option<PatchOutcomeStatus>,
    pub after: Option<PatchOutcomeStatus>,
    pub before_matches: u32,
    pub after_matches: u32,
}

/// A line that could anchor a patch: `matches_in_file` above 1 means the anchor is ambiguous.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AnchorCandidate {
    /// 1-based line in the patched file.
    pub line: u32,
    pub text: String,
    /// Lines of the patched file this one matches as a pattern anchor (before `[vars]`
    /// values are filled in, as patterns match).
    pub matches_in_file: u32,
    /// The patch that wrote the line; none for the game's own line.
    #[serde(default)]
    pub origin: Option<LineSource>,
    /// In region mode, how many of the other patch's lines at this place read the same; none
    /// for a line the other patch did not write (it cannot be picked).
    #[serde(default)]
    pub matches_in_region: Option<u32>,
    /// The line before `[vars]` values are filled in: what a pattern on it must say.
    #[serde(default)]
    pub raw_text: String,
}

/// Who wrote a line of the patched file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LineSource {
    /// "inserted" | "kept" (written back unchanged) | "modified" (changed in place).
    pub kind: String,
    pub patch_id: String,
    pub mod_id: String,
}

/// One place of another patch's change, for `anchor_candidates` in region mode.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RegionRef {
    pub patch_id: String,
    /// The patch's site index, as `RegionSite.site` gives it.
    pub site: u32,
}

/// Where a patch's code can go when another patch took its line (§9.2).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RegionOptions {
    pub target_file: String,
    pub patch_id: String,
    /// The patch that took the line, and how.
    pub consumer: Option<qm_manager::conflict::ConsumerInfo>,
    /// The patch replaces its lines (`at`): it gets no options of its own (D6).
    pub a_is_at: bool,
    /// "Move to another line" makes an adjusted copy for this patch (`at` and regex, D25).
    pub move_by_copy: bool,
    /// The patch whose places `sites` lists.
    pub b_patch_id: Option<String>,
    pub b_mod_id: Option<String>,
    /// Suggested first. Each was previewed; one that fails says why in `placement_reason`.
    pub options: Vec<RegionOption>,
    pub sites: Vec<RegionSite>,
    /// The patch's own places in the game's file, for a move of one of them (D25).
    pub a_sites: Vec<ASite>,
}

/// One region option. `draft` is none for "Pick a line inside ...", which opens the picker.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RegionOption {
    pub key: String,
    pub label: String,
    /// The class of the loss it answers: "split" | "reemitted" | "rewritten_1to1" |
    /// "replaced_by_block" | "deleted".
    pub kind: String,
    pub draft: Option<FixDraft>,
    /// False: greyed, with the reason.
    pub placement_ok: bool,
    pub placement_reason: Option<String>,
    pub suggested: bool,
    pub note: Option<String>,
    /// The line as the game has it and as the other patch wrote it, side by side.
    pub old_text: Option<String>,
    pub new_text: Option<String>,
}

/// One place of the other patch's change.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RegionSite {
    pub site: u32,
    /// 1-based, as the patched file shows it.
    pub first_line: u32,
    pub last_line: u32,
    /// The function the place starts in ("" at the top level).
    pub function: String,
    /// Every line from the first to the last; lines the other patch did not write have no
    /// `matches_in_region`.
    pub lines: Vec<AnchorCandidate>,
    pub edge_start: qm_manager::placement::EdgeCheck,
    pub edge_end: qm_manager::placement::EdgeCheck,
}

/// One place of a patch in the game's file.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ASite {
    pub key: qm_core::patch::anchor::SiteKey,
    /// 1-based, as the file shows it.
    pub line: u32,
    /// 1-based line the place's code hangs on (its last line for `after`), as
    /// `consumer.site_line` names it.
    pub anchor_line: u32,
    pub text: String,
}

// ---- Fixes (Phase C §10) ----

/// A fix to make. The backend finds the patches and pins their fingerprints.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FixDraft {
    pub change: FixChangeDraft,
    /// Empty: a label is made from the change.
    #[serde(default)]
    pub label: String,
    pub source: FixSource,
    #[serde(default)]
    pub conflict_id: Option<String>,
    #[serde(default)]
    pub option_key: Option<String>,
    #[serde(default)]
    pub approved: Option<ApprovedImpact>,
}

/// What a draft changes, by patch id.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FixChangeDraft {
    Disable {
        patch_id: String,
    },
    Order {
        rule: qm_core::patch::order::OrderRule,
    },
    Priority {
        patch_id: String,
        priority: i32,
    },
    /// Put the patch's code next to another patch's code (a rung each in `anchors`).
    Anchor {
        patch_id: String,
        anchors: Vec<qm_core::patch::anchor::Rung>,
        /// Which of the patch's places the rungs are for (none: all of them).
        #[serde(default)]
        sites: Vec<qm_core::patch::anchor::SiteKey>,
    },
    /// Run an `at` patch as separate edits (or as one replacement again), D46.
    ApplyAs {
        patch_id: String,
        mode: qm_core::patch::edits::ApplyAs,
    },
    /// Skip one edit of an `at` patch so another patch's change to the same lines stays
    /// (D49): edit `ordinal` of `patch_id` (from 0), keeping `keep_patch_id`'s change.
    SkipHunk {
        patch_id: String,
        ordinal: u32,
        #[serde(default)]
        keep_patch_id: Option<String>,
    },
    /// At the game's line `site` (a shared stack's `site`), on `side`, put `patch_id`'s block
    /// below `below_patch_id`'s block or above `above_patch_id`'s (D55). One of the two.
    StackRule {
        patch_id: String,
        site: qm_core::patch::anchor::SiteKey,
        side: qm_core::patch::linemap::Side,
        #[serde(default)]
        below_patch_id: Option<String>,
        #[serde(default)]
        above_patch_id: Option<String>,
    },
    /// Run the adjusted copy `glue_id` as its base mod at the base's place (`Base`, "Run it
    /// like <mod>"), or as `qm_glue` after every mod patch (`Legacy`), D58. It changes the
    /// copy's fix in place.
    AdjustedCopyRunAs {
        glue_id: String,
        run_as: qm_core::handshake::RunAs,
    },
    /// Give the asset group or slot `key` (from the asset ledger) to `winner`: a mod id,
    /// `faction:<id>`, or `vanilla` for the game's own files (Phase E D76).
    AssetChoice {
        key: String,
        winner: String,
    },
}

/// How a fix is doing, worked out on every read (§7.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FixStatus {
    Active,
    Rebound { from: String, to: String },
    Followed,
    FellBack { sites: u32, reason: String },
    Waiting { mods: Vec<String> },
    NeedsReview { reasons: Vec<String> },
    Suspended { reason: String },
    Paused,
}

/// A patch as the Fixes view names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PatchLabel {
    pub patch_id: String,
    pub mod_id: String,
    pub name: Option<String>,
    pub mod_name: Option<String>,
}

/// A patch's text when the fix was made against its text now (D41).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct DriftDiff {
    pub patch_id: String,
    /// "fix" (the patch the fix changes) | "anchor_target" (a patch it refers to).
    pub role: String,
    pub before: PatchSnapshot,
    pub after: PatchSnapshot,
}

/// One fix in the Fixes view.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FixView {
    pub id: String,
    pub label: String,
    /// "disable" | "order" | "priority" | "anchor" | "adjusted_copy" | "mod_order" |
    /// "apply_as".
    pub kind: String,
    pub status: FixStatus,
    pub status_text: String,
    pub target_file: Option<String>,
    pub patch: Option<PatchLabel>,
    pub related: Vec<PatchLabel>,
    pub source: FixSource,
    pub provenance: Provenance,
    pub paused: bool,
    pub fallback_text: Option<String>,
    pub drift: Option<Vec<DriftDiff>>,
    pub can_export: bool,
    pub export_blocked_reason: Option<String>,
    /// A read-only row of a profile that is not migrated yet (D38); change it with the
    /// command that made it.
    pub legacy: bool,
    /// The patches the fix could mean now that several match it (Ambiguous, §7.2); the
    /// user picks one with `fix_rebind`.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[specta(optional)]
    pub candidates: Vec<PatchLabel>,
    /// An adjusted copy's glue id, for `glue_patch_remove` on a legacy row (D38).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[specta(optional)]
    pub glue_id: Option<String>,
    /// For a running adjusted copy that runs as `qm_glue` after every mod patch: the mod it
    /// can run as instead, at its base's place ("Run it like <mod>", D58).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[specta(optional)]
    pub run_like_mod: Option<String>,
    /// A running copy that "Run it like <mod>" switched to its base mod: it can go back to
    /// running after every mod patch ("Run it last again", D58).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    #[specta(optional)]
    pub can_run_last: bool,
    /// The game files the fix's patches target that changed since it was made or accepted
    /// (D74); the fix then needs review.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[specta(optional)]
    pub game_drift: Vec<qm_manager::fixes::GameFileDrift>,
}

/// A fix the launcher suggests (§9.3). Apply makes `drafts` and undoes `removes`.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FixSuggestion {
    pub key: String,
    /// "stale" | "follow" | "combined" | "rebind" | "retire".
    pub kind: String,
    pub label: String,
    pub drafts: Vec<FixDraft>,
    /// Ids of fixes the suggestion removes.
    pub removes: Vec<String>,
    /// The preview showed no new conflict, broken file or moved code.
    pub safe: bool,
    /// The fix it is about. A "rebind" suggestion has no drafts: the user reviews this fix.
    #[serde(default)]
    pub fix_id: Option<String>,
}

/// One batch operation that can be undone.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct JournalView {
    pub index: u32,
    pub at: String,
    /// "migrated" | "compat_export".
    pub op: String,
    pub label: String,
    pub removed: u32,
    pub added: u32,
}

/// Everything the Fixes view shows.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FixesView {
    /// The profile these fixes belong to; the UI drops an answer for another profile.
    pub profile_id: String,
    pub fixes: Vec<FixView>,
    pub suggestions: Vec<FixSuggestion>,
    /// The suggestions are being made in the background; an `Invalidated { Fixes }` event
    /// follows once they are ready.
    pub suggestions_pending: bool,
    /// Why there are no suggestions, when they could not be made (D70).
    pub suggestions_note: Option<String>,
    pub stacking: qm_core::handshake::StackMode,
    pub stacking_acknowledged: bool,
    pub plan_error: Option<String>,
    pub load_errors: Vec<LoadError>,
    /// The staged load-order.json could not be read, so this view uses the profile's own mod
    /// order; says why.
    pub plan_note: Option<String>,
    /// Ledger entries this version cannot read; they are kept as they are.
    pub unreadable_fixes: u32,
    pub journal: Vec<JournalView>,
    /// False: the rows are the legacy fields, read-only (D38).
    pub migrated: bool,
    pub migration_error: Option<String>,
}

/// What undoing a fix would do.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct UndoPreview {
    pub impact: PreviewImpact,
    /// Later fixes on the same patch that are undone with it.
    pub also_undoes: Vec<String>,
    /// Things the user changed since, that undo leaves as they are; the user confirms first.
    pub asks: Vec<String>,
}

/// What undoing a batch step would do (D71). Nothing is saved.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct JournalUndoPreview {
    /// "migrated" | "compat_export".
    pub op: String,
    pub label: String,
    /// Labels of the fixes the undo brings back.
    pub restores: Vec<String>,
    /// Names of the mods the undo turns off.
    pub turns_off_mods: Vec<String>,
    /// Fixes that would come back next to a newer running fix holding the same thing. When
    /// there are any, the undo needs a choice.
    pub clashes: Vec<JournalClash>,
    /// The undo that keeps every newer fix running: with clashes, the clashing fixes come back
    /// paused (`RestorePaused`). None while the simulation is not ready.
    pub impact_keep: Option<PreviewImpact>,
    /// With clashes: the newer fixes paused and the old ones back running (`PauseNewer`).
    /// None without clashes, or while the simulation is not ready.
    pub impact_pause: Option<PreviewImpact>,
}

/// A fix an undo brings back and the newer running fix it clashes with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JournalClash {
    pub restored_fix_id: String,
    pub restored_label: String,
    pub newer_fix_id: String,
    pub newer_label: String,
    /// What both hold: the restored fix's kind ("order", "priority", ...).
    pub slot: String,
    /// The patch the restored fix is about; empty when it has none.
    pub patch_id: String,
}

/// How an undo with clashes goes on (D71).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum JournalUndoChoice {
    /// Pause the newer clashing fixes and bring the old ones back running.
    PauseNewer,
    /// Bring the clashing old fixes back paused; the newer ones keep running.
    RestorePaused,
}
