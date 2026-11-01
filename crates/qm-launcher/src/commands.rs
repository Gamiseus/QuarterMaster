//! Thin Tauri commands: each runs one `Service` call on the blocking pool.
use qm_core::handshake::CrashReport;
use qm_manager::storage::DetectedPaths;
use qm_manager::{
    ArchiveModCandidate, ConflictResolutionSuggestion, LoadOrderResolution, ModProfile,
    SaveBackupEntry, SaveProfile,
};
use qm_schema::{ConfigValue, SaveComparison, SaveSummary};
use qm_service::{
    AppInfo, CatalogQuery, CatalogResult, ConfigResetMode, ErrorKind, FileDiff, GameStatus,
    HealthReport, ImportRequest, JobId, JobInfo, LaunchRequest, LoadOrderUpdate, Loadable,
    LogSource, ModDetail, ModPatchStatus, ModSummary, NewProfile, ProfileSettingsPatch,
    ProfileSummary, QmError, QmResult, RevealKind, SaveSlotSummary, Service, SettingsPatch,
    SettingsView, SimulationOverview, StartupReport, SwitchPreview, UpdatesReport,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

const MSG_INTERNAL: &str = "Unexpected error. See the launcher log for details.";

/// Run `f` on the blocking pool; a panic becomes an `Internal` error.
async fn run<T: Send + 'static>(
    svc: &Service,
    f: impl FnOnce(&Service) -> QmResult<T> + Send + 'static,
) -> QmResult<T> {
    let svc = svc.clone();
    match tauri::async_runtime::spawn_blocking(move || f(&svc)).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("command task failed: {e}");
            Err(QmError::new(ErrorKind::Internal, MSG_INTERNAL))
        }
    }
}

type Svc<'a> = State<'a, Service>;

// App

#[tauri::command]
#[specta::specta]
pub async fn app_info(svc: Svc<'_>) -> Result<AppInfo, QmError> {
    run(&svc, move |s| Ok(s.app_info())).await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_get(svc: Svc<'_>) -> Result<SettingsView, QmError> {
    run(&svc, move |s| Ok(s.settings_get())).await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_update(svc: Svc<'_>, patch: SettingsPatch) -> Result<SettingsView, QmError> {
    run(&svc, move |s| s.settings_update(patch)).await
}

#[tauri::command]
#[specta::specta]
pub async fn paths_auto_detect(svc: Svc<'_>) -> Result<DetectedPaths, QmError> {
    run(&svc, move |s| Ok(s.paths_auto_detect())).await
}

#[tauri::command]
#[specta::specta]
pub async fn onboarding_complete(
    svc: Svc<'_>,
    patch: SettingsPatch,
) -> Result<SettingsView, QmError> {
    run(&svc, move |s| s.onboarding_complete(patch)).await
}

/// Open one of a fixed set of folders in Explorer.
#[tauri::command]
#[specta::specta]
pub async fn reveal(svc: Svc<'_>, kind: RevealKind) -> Result<(), QmError> {
    run(&svc, move |s| {
        let path = s.reveal_path(kind)?;
        std::process::Command::new(explorer_exe())
            .arg(&path)
            .spawn()
            .map(drop)
            .map_err(|e| {
                QmError::new(ErrorKind::Io, "Could not open Explorer.").with_details(e.to_string())
            })
    })
    .await
}

/// `%SystemRoot%\explorer.exe` by absolute path, never a PATH or app-dir search.
fn explorer_exe() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(root).join("explorer.exe")
}

/// Managed startup state; `None` until `Service::start` returns. Holds the file watcher alive.
#[derive(Default)]
pub struct Startup(pub Mutex<Option<StartupReport>>);

/// Shown when startup panicked, so the UI stops polling and points at the log.
pub const STARTUP_FAILED: &str = "Startup failed. See the launcher log.";

/// Runs `start`, turning a panic into a finished report that says startup failed.
pub fn run_startup(start: impl FnOnce() -> StartupReport) -> StartupReport {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(start)).unwrap_or_else(|_| {
        tracing::error!("startup panicked");
        StartupReport {
            recovered: Some(STARTUP_FAILED.to_string()),
            attached_pid: None,
            watch: None,
        }
    })
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct StartupStatus {
    pub done: bool,
    pub recovered: Option<String>,
    pub attached_pid: Option<u32>,
}

/// Startup outcome, so the UI sees a recovery that ran before it listened for events.
#[tauri::command]
#[specta::specta]
pub async fn startup_status(startup: State<'_, Startup>) -> Result<StartupStatus, QmError> {
    let report = startup.0.lock().unwrap_or_else(|e| e.into_inner());
    Ok(match report.as_ref() {
        Some(r) => StartupStatus {
            done: true,
            recovered: r.recovered.clone(),
            attached_pid: r.attached_pid,
        },
        None => StartupStatus {
            done: false,
            recovered: None,
            attached_pid: None,
        },
    })
}

// Profiles

#[tauri::command]
#[specta::specta]
pub async fn profiles_list(svc: Svc<'_>) -> Result<Vec<ProfileSummary>, QmError> {
    run(&svc, move |s| Ok(s.profiles_list())).await
}

#[tauri::command]
#[specta::specta]
pub async fn profile_get(svc: Svc<'_>, id: String) -> Result<ModProfile, QmError> {
    run(&svc, move |s| s.profile_get(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn profile_preview_switch(svc: Svc<'_>, id: String) -> Result<SwitchPreview, QmError> {
    run(&svc, move |s| s.profile_preview_switch(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn profile_create(svc: Svc<'_>, req: NewProfile) -> Result<ModProfile, QmError> {
    run(&svc, move |s| s.profile_create(req)).await
}

#[tauri::command]
#[specta::specta]
pub async fn profile_rename(svc: Svc<'_>, id: String, name: String) -> Result<ModProfile, QmError> {
    run(&svc, move |s| s.profile_rename(&id, &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn profile_update(
    svc: Svc<'_>,
    id: String,
    patch: ProfileSettingsPatch,
) -> Result<ModProfile, QmError> {
    run(&svc, move |s| s.profile_update(&id, patch)).await
}

#[tauri::command]
#[specta::specta]
pub async fn profile_delete(svc: Svc<'_>, id: String) -> Result<(), QmError> {
    run(&svc, move |s| s.profile_delete(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn profile_clone(
    svc: Svc<'_>,
    src_id: String,
    new_name: String,
) -> Result<ModProfile, QmError> {
    run(&svc, move |s| s.profile_clone(&src_id, &new_name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn profile_activate(svc: Svc<'_>, id: String) -> Result<ModProfile, QmError> {
    run(&svc, move |s| s.profile_activate(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn profile_export_code(svc: Svc<'_>, id: String) -> Result<String, QmError> {
    run(&svc, move |s| s.profile_export_code(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn profile_import_code(
    svc: Svc<'_>,
    code: String,
    new_id: String,
) -> Result<ModProfile, QmError> {
    run(&svc, move |s| s.profile_import_code(&code, &new_id)).await
}

// Mods

#[tauri::command]
#[specta::specta]
pub async fn mods_list(svc: Svc<'_>) -> Result<Loadable<Vec<ModSummary>>, QmError> {
    run(&svc, move |s| Ok(s.mods_list())).await
}

#[tauri::command]
#[specta::specta]
pub async fn mod_patches_get(
    svc: Svc<'_>,
    mod_id: String,
) -> Result<Loadable<Vec<ModPatchStatus>>, QmError> {
    run(&svc, move |s| s.mod_patches_get(&mod_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn mod_detail(svc: Svc<'_>, id: String) -> Result<ModDetail, QmError> {
    run(&svc, move |s| s.mod_detail(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn mod_set_enabled(
    svc: Svc<'_>,
    id: String,
    enabled: bool,
) -> Result<ModSummary, QmError> {
    run(&svc, move |s| s.mod_set_enabled(&id, enabled)).await
}

#[tauri::command]
#[specta::specta]
pub async fn mod_config_set(
    svc: Svc<'_>,
    id: String,
    values: HashMap<String, ConfigValue>,
) -> Result<HashMap<String, ConfigValue>, QmError> {
    run(&svc, move |s| s.mod_config_set(&id, values)).await
}

#[tauri::command]
#[specta::specta]
pub async fn mod_config_reset(
    svc: Svc<'_>,
    id: String,
    mode: ConfigResetMode,
) -> Result<HashMap<String, ConfigValue>, QmError> {
    run(&svc, move |s| s.mod_config_reset(&id, mode)).await
}

#[tauri::command]
#[specta::specta]
pub async fn mod_import_inspect(
    svc: Svc<'_>,
    path: PathBuf,
) -> Result<Vec<ArchiveModCandidate>, QmError> {
    run(&svc, move |s| s.mod_import_inspect(&path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn mod_import(svc: Svc<'_>, req: ImportRequest) -> Result<JobId, QmError> {
    run(&svc, move |s| s.mod_import(req)).await
}

#[tauri::command]
#[specta::specta]
pub async fn mod_uninstall(svc: Svc<'_>, id: String) -> Result<(), QmError> {
    run(&svc, move |s| s.mod_uninstall(&id)).await
}

// Load order

#[tauri::command]
#[specta::specta]
pub async fn load_order_get(svc: Svc<'_>) -> Result<Loadable<LoadOrderResolution>, QmError> {
    run(&svc, move |s| Ok(s.load_order_get())).await
}

#[tauri::command]
#[specta::specta]
pub async fn simulation_get(svc: Svc<'_>) -> Result<Loadable<SimulationOverview>, QmError> {
    run(&svc, move |s| Ok(s.simulation_get())).await
}

#[tauri::command]
#[specta::specta]
pub async fn file_diff_get(
    svc: Svc<'_>,
    target_file: String,
) -> Result<Loadable<FileDiff>, QmError> {
    run(&svc, move |s| s.file_diff_get(&target_file)).await
}

#[tauri::command]
#[specta::specta]
pub async fn load_order_set(
    svc: Svc<'_>,
    update: LoadOrderUpdate,
) -> Result<LoadOrderResolution, QmError> {
    run(&svc, move |s| s.load_order_set(update)).await
}

#[tauri::command]
#[specta::specta]
pub async fn patch_set_enabled(
    svc: Svc<'_>,
    patch_id: String,
    enabled: bool,
) -> Result<(), QmError> {
    run(&svc, move |s| s.patch_set_enabled(&patch_id, enabled)).await
}

#[tauri::command]
#[specta::specta]
pub async fn conflict_resolve(
    svc: Svc<'_>,
    suggestion: ConflictResolutionSuggestion,
    winner_index: Option<u32>,
) -> Result<(), QmError> {
    run(&svc, move |s| s.conflict_resolve(suggestion, winner_index)).await
}

// Saves

#[tauri::command]
#[specta::specta]
pub async fn saves_list(svc: Svc<'_>) -> Result<Vec<SaveSlotSummary>, QmError> {
    run(&svc, move |s| Ok(s.saves_list())).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_detail(svc: Svc<'_>, id: String) -> Result<SaveSummary, QmError> {
    run(&svc, move |s| s.save_detail(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn saves_compare(svc: Svc<'_>, a: String, b: String) -> Result<SaveComparison, QmError> {
    run(&svc, move |s| s.saves_compare(&a, &b)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_activate(svc: Svc<'_>, id: String) -> Result<(), QmError> {
    run(&svc, move |s| s.save_activate(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_clone(
    svc: Svc<'_>,
    src_id: String,
    new_id: String,
    new_name: String,
) -> Result<SaveProfile, QmError> {
    run(&svc, move |s| s.save_clone(&src_id, &new_id, &new_name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_backup(
    svc: Svc<'_>,
    id: String,
    label: String,
) -> Result<SaveBackupEntry, QmError> {
    run(&svc, move |s| s.save_backup(&id, &label)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_restore(svc: Svc<'_>, id: String, backup_id: String) -> Result<(), QmError> {
    run(&svc, move |s| s.save_restore(&id, &backup_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_delete_backup(
    svc: Svc<'_>,
    id: String,
    backup_id: String,
) -> Result<(), QmError> {
    run(&svc, move |s| s.save_delete_backup(&id, &backup_id)).await
}

// Launch

#[tauri::command]
#[specta::specta]
pub async fn game_status(svc: Svc<'_>) -> Result<GameStatus, QmError> {
    run(&svc, move |s| Ok(s.game_status())).await
}

#[tauri::command]
#[specta::specta]
pub async fn game_launch(svc: Svc<'_>, req: LaunchRequest) -> Result<GameStatus, QmError> {
    run(&svc, move |s| s.game_launch(req)).await
}

#[tauri::command]
#[specta::specta]
pub async fn game_restore_clean(svc: Svc<'_>) -> Result<(), QmError> {
    run(&svc, move |s| s.game_restore_clean()).await
}

// Diagnostics

#[tauri::command]
#[specta::specta]
pub async fn health_get(svc: Svc<'_>) -> Result<Loadable<HealthReport>, QmError> {
    run(&svc, move |s| Ok(s.health_get())).await
}

#[tauri::command]
#[specta::specta]
pub async fn crashes_list(svc: Svc<'_>) -> Result<Vec<CrashReport>, QmError> {
    run(&svc, move |s| Ok(s.crashes_list())).await
}

#[tauri::command]
#[specta::specta]
pub async fn logs_tail(
    svc: Svc<'_>,
    source: LogSource,
    lines: u32,
) -> Result<Vec<String>, QmError> {
    run(&svc, move |s| s.logs_tail(source, lines)).await
}

#[tauri::command]
#[specta::specta]
pub async fn diagnostic_bundle_build(svc: Svc<'_>) -> Result<JobId, QmError> {
    run(&svc, move |s| s.diagnostic_bundle_build()).await
}

#[tauri::command]
#[specta::specta]
pub async fn crash_disable_suspect(svc: Svc<'_>, crash_id: String) -> Result<(), QmError> {
    run(&svc, move |s| s.crash_disable_suspect(&crash_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn dll_repair(svc: Svc<'_>) -> Result<String, QmError> {
    run(&svc, move |s| s.dll_repair()).await
}

// Updates and catalog

#[tauri::command]
#[specta::specta]
pub async fn updates_get(svc: Svc<'_>) -> Result<Loadable<UpdatesReport>, QmError> {
    run(&svc, move |s| Ok(s.updates_get())).await
}

#[tauri::command]
#[specta::specta]
pub async fn updates_refresh(svc: Svc<'_>) -> Result<JobId, QmError> {
    run(&svc, move |s| Ok(s.updates_refresh())).await
}

#[tauri::command]
#[specta::specta]
pub async fn catalog_get(
    svc: Svc<'_>,
    query: CatalogQuery,
) -> Result<Loadable<CatalogResult>, QmError> {
    run(&svc, move |s| Ok(s.catalog_get(query))).await
}

#[tauri::command]
#[specta::specta]
pub async fn catalog_refresh(svc: Svc<'_>) -> Result<JobId, QmError> {
    run(&svc, move |s| Ok(s.catalog_refresh())).await
}

#[tauri::command]
#[specta::specta]
pub async fn catalog_install(
    svc: Svc<'_>,
    download_url: String,
    mod_id: Option<String>,
) -> Result<JobId, QmError> {
    run(&svc, move |s| s.catalog_install(&download_url, mod_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn launcher_update_download(
    svc: Svc<'_>,
    download_url: String,
    sha256: String,
) -> Result<JobId, QmError> {
    run(&svc, move |s| {
        s.launcher_update_download(&download_url, sha256)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn launcher_update_apply(svc: Svc<'_>) -> Result<(), QmError> {
    run(&svc, move |s| s.launcher_update_apply()).await
}

#[tauri::command]
#[specta::specta]
pub async fn launcher_update_rollback(svc: Svc<'_>) -> Result<(), QmError> {
    run(&svc, move |s| s.launcher_update_rollback()).await
}

// Jobs

#[tauri::command]
#[specta::specta]
pub async fn jobs_list(svc: Svc<'_>) -> Result<Vec<JobInfo>, QmError> {
    run(&svc, move |s| Ok(s.jobs().list())).await
}

#[tauri::command]
#[specta::specta]
pub async fn job_cancel(svc: Svc<'_>, id: JobId) -> Result<(), QmError> {
    run(&svc, move |s| s.jobs().cancel(id)).await
}
