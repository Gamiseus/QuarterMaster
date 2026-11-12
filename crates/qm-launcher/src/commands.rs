//! Thin Tauri commands: each runs one `Service` call on the blocking pool.
use qm_core::handshake::{CrashReport, GluePatchEntry, StackMode};
use qm_core::patch::order::OrderRule;
use qm_core::patch::InsertPosition;
use qm_manager::storage::DetectedPaths;
use qm_manager::{
    ArchiveModCandidate, ConflictResolutionSuggestion, LoadOrderResolution, ModProfile,
    ProfileDiff, SaveBackupEntry, SaveProfile, SharedStack,
};
use qm_schema::{ConfigValue, SaveComparison, SaveSummary};
use qm_service::ops::modpacks::ModpackPreview;
use qm_service::ops::sprites::{RibbonInfo, SpriteKind};
use qm_service::{
    AnchorCandidate, AppInfo, BisectPreview, BisectStatus, BisectView, CatalogQuery, CatalogResult,
    CompatChangesView, CompatExportPreview, CompatExportRequest, CompatExportResult,
    ConfigResetMode, ErrorKind, FileDiff, FixDraft, FixView, FixesView, GameStatus, HealthReport,
    ImportRequest, JobId, JobInfo, JournalUndoChoice, JournalUndoPreview, LaunchRequest,
    LoadOrderUpdate, Loadable, LogSource, ModDetail, ModPatchStatus, ModSummary, NewProfile,
    PatchDetail, PreviewChanges, PreviewResult, ProfileSettingsPatch, ProfileSummary, QmError,
    QmResult, RegionOptions, RegionRef, RevealKind, SaveSlotSummary, Service, SettingsPatch,
    SettingsView, SimulationOverview, StartupReport, SwitchPreview, TrialRequest, TrialResult,
    TrialSuspect, UndoPreview, UpdatesReport,
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
pub async fn profile_diff(svc: Svc<'_>, a: String, b: String) -> Result<ProfileDiff, QmError> {
    run(&svc, move |s| s.profile_diff(&a, &b)).await
}

/// Asks where to save the modpack with the native save dialog (the path never comes from the
/// UI), then writes it in a job. `None` when the dialog was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn modpack_export(
    app: tauri::AppHandle,
    svc: Svc<'_>,
    profile_id: String,
) -> Result<Option<JobId>, QmError> {
    use tauri_plugin_dialog::DialogExt;
    let name = svc.profile_get(&profile_id)?.name;
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let file_name = format!("{}.qmpack", safe.trim());
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("QuarterMaster modpack", &["qmpack"])
            .set_file_name(&file_name)
            .blocking_save_file()
    })
    .await
    .map_err(|_| QmError::new(ErrorKind::Internal, MSG_INTERNAL))?;
    let Some(dest) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    run(&svc, move |s| s.modpack_export(&profile_id, dest).map(Some)).await
}

#[tauri::command]
#[specta::specta]
pub async fn modpack_inspect(svc: Svc<'_>, path: PathBuf) -> Result<ModpackPreview, QmError> {
    run(&svc, move |s| s.modpack_inspect(&path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn modpack_import(
    svc: Svc<'_>,
    path: PathBuf,
    name: String,
    slot: Option<String>,
) -> Result<JobId, QmError> {
    run(&svc, move |s| s.modpack_import(path, name, slot)).await
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
pub async fn compat_changes_get(
    svc: Svc<'_>,
    mod_id: String,
) -> Result<Loadable<CompatChangesView>, QmError> {
    run(&svc, move |s| s.compat_changes_get(&mod_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn compat_export_preview(
    svc: Svc<'_>,
    req: CompatExportRequest,
) -> Result<CompatExportPreview, QmError> {
    run(&svc, move |s| s.compat_export_preview(req)).await
}

#[tauri::command]
#[specta::specta]
pub async fn compat_export(
    svc: Svc<'_>,
    req: CompatExportRequest,
) -> Result<CompatExportResult, QmError> {
    run(&svc, move |s| s.compat_export(req)).await
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
    allow_unmanaged: bool,
) -> Result<Vec<ArchiveModCandidate>, QmError> {
    run(&svc, move |s| s.mod_import_inspect(&path, allow_unmanaged)).await
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

// Patch tools (plan 2d)

#[tauri::command]
#[specta::specta]
pub async fn patch_detail(svc: Svc<'_>, patch_id: String) -> Result<PatchDetail, QmError> {
    run(&svc, move |s| s.patch_detail(&patch_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn simulation_preview(
    svc: Svc<'_>,
    changes: PreviewChanges,
) -> Result<PreviewResult, QmError> {
    run(&svc, move |s| s.simulation_preview(changes)).await
}

#[tauri::command]
#[specta::specta]
pub async fn trial_preview(
    svc: Svc<'_>,
    changes: qm_manager::runs::TrialChanges,
) -> Result<PreviewResult, QmError> {
    run(&svc, move |s| s.trial_preview(changes)).await
}

#[tauri::command]
#[specta::specta]
pub async fn anchor_candidates(
    svc: Svc<'_>,
    target_file: String,
    line: u32,
    radius: u32,
    region: Option<RegionRef>,
) -> Result<Vec<AnchorCandidate>, QmError> {
    run(&svc, move |s| {
        s.anchor_candidates(&target_file, line, radius, region)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn region_options(
    svc: Svc<'_>,
    target_file: String,
    patch_id: String,
    conflict_id: Option<String>,
) -> Result<RegionOptions, QmError> {
    run(&svc, move |s| {
        s.region_options(&target_file, &patch_id, conflict_id.as_deref())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn glue_patch_create(
    svc: Svc<'_>,
    base_patch_id: String,
    pattern: String,
    position: InsertPosition,
    conflict_id: Option<String>,
) -> Result<GluePatchEntry, QmError> {
    run(&svc, move |s| {
        s.glue_patch_create_for(&base_patch_id, &pattern, position, conflict_id.as_deref())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn glue_patch_remove(svc: Svc<'_>, glue_id: String) -> Result<(), QmError> {
    run(&svc, move |s| s.glue_patch_remove(&glue_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn patch_order_set(svc: Svc<'_>, rule: OrderRule) -> Result<(), QmError> {
    run(&svc, move |s| s.patch_order_set(rule)).await
}

#[tauri::command]
#[specta::specta]
pub async fn patch_order_reset(svc: Svc<'_>, patch_id: String) -> Result<(), QmError> {
    run(&svc, move |s| s.patch_order_reset(&patch_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn stacks_get(
    svc: Svc<'_>,
    target_file: Option<String>,
) -> Result<Vec<SharedStack>, QmError> {
    run(&svc, move |s| s.stacks_get(target_file.as_deref())).await
}

#[tauri::command]
#[specta::specta]
pub async fn stacking_set(svc: Svc<'_>, mode: StackMode) -> Result<(), QmError> {
    run(&svc, move |s| s.stacking_set(mode)).await
}

#[tauri::command]
#[specta::specta]
pub async fn line_rules_set(svc: Svc<'_>, on: bool) -> Result<(), QmError> {
    run(&svc, move |s| s.line_rules_set(on)).await
}

#[tauri::command]
#[specta::specta]
pub async fn notice_dismiss(svc: Svc<'_>, key: String) -> Result<(), QmError> {
    run(&svc, move |s| s.notice_dismiss(&key)).await
}

// Fixes (Phase C)

#[tauri::command]
#[specta::specta]
pub async fn fixes_list(svc: Svc<'_>) -> Result<FixesView, QmError> {
    run(&svc, move |s| s.fixes_list()).await
}

#[tauri::command]
#[specta::specta]
pub async fn fixes_import(svc: Svc<'_>) -> Result<FixesView, QmError> {
    run(&svc, move |s| s.fixes_import()).await
}

#[tauri::command]
#[specta::specta]
pub async fn fix_create(svc: Svc<'_>, draft: FixDraft) -> Result<FixView, QmError> {
    run(&svc, move |s| s.fix_create(draft)).await
}

#[tauri::command]
#[specta::specta]
pub async fn fix_create_many(svc: Svc<'_>, drafts: Vec<FixDraft>) -> Result<Vec<FixView>, QmError> {
    run(&svc, move |s| s.fix_create_many(drafts)).await
}

#[tauri::command]
#[specta::specta]
pub async fn fix_suggestion_apply(
    svc: Svc<'_>,
    drafts: Vec<FixDraft>,
    removes: Vec<String>,
) -> Result<Vec<FixView>, QmError> {
    run(&svc, move |s| s.fix_suggestion_apply(drafts, removes)).await
}

#[tauri::command]
#[specta::specta]
pub async fn fix_undo_preview(svc: Svc<'_>, fix_id: String) -> Result<UndoPreview, QmError> {
    run(&svc, move |s| s.fix_undo_preview(&fix_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn fix_undo(svc: Svc<'_>, fix_id: String, confirmed: bool) -> Result<(), QmError> {
    run(&svc, move |s| s.fix_undo(&fix_id, confirmed)).await
}

#[tauri::command]
#[specta::specta]
pub async fn fix_journal_undo_preview(
    svc: Svc<'_>,
    entry_index: u32,
) -> Result<JournalUndoPreview, QmError> {
    run(&svc, move |s| s.fix_journal_undo_preview(entry_index)).await
}

#[tauri::command]
#[specta::specta]
pub async fn fix_journal_undo(
    svc: Svc<'_>,
    entry_index: u32,
    choice: Option<JournalUndoChoice>,
) -> Result<(), QmError> {
    run(&svc, move |s| s.fix_journal_undo(entry_index, choice)).await
}

#[tauri::command]
#[specta::specta]
pub async fn fix_set_paused(
    svc: Svc<'_>,
    fix_id: String,
    paused: bool,
) -> Result<FixView, QmError> {
    run(&svc, move |s| s.fix_set_paused(&fix_id, paused)).await
}

#[tauri::command]
#[specta::specta]
pub async fn fix_accept(svc: Svc<'_>, fix_id: String) -> Result<FixView, QmError> {
    run(&svc, move |s| s.fix_accept(&fix_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn fix_rebind(
    svc: Svc<'_>,
    fix_id: String,
    patch_id: String,
) -> Result<FixView, QmError> {
    run(&svc, move |s| s.fix_rebind(&fix_id, &patch_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn patch_set_enabled_many(
    svc: Svc<'_>,
    patch_ids: Vec<String>,
    enabled: bool,
) -> Result<(), QmError> {
    run(&svc, move |s| s.patch_set_enabled_many(&patch_ids, enabled)).await
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
pub async fn game_launch_trial(svc: Svc<'_>, req: TrialRequest) -> Result<GameStatus, QmError> {
    run(&svc, move |s| s.game_launch_trial(req)).await
}

#[tauri::command]
#[specta::specta]
pub async fn diagnostic_run_preview(
    svc: Svc<'_>,
) -> Result<qm_service::DiagnosticRunPreview, QmError> {
    run(&svc, move |s| Ok(s.diagnostic_run_preview())).await
}

#[tauri::command]
#[specta::specta]
pub async fn game_launch_diagnostic(
    svc: Svc<'_>,
    req: qm_service::DiagnosticRunRequest,
) -> Result<GameStatus, QmError> {
    run(&svc, move |s| s.game_launch_diagnostic(req)).await
}

#[tauri::command]
#[specta::specta]
pub async fn diagnostic_run_result(
    svc: Svc<'_>,
) -> Result<Option<qm_service::DiagnosticRunResult>, QmError> {
    run(&svc, move |s| Ok(s.diagnostic_run_result())).await
}

#[tauri::command]
#[specta::specta]
pub async fn bisect_start(svc: Svc<'_>, run_id: String) -> Result<BisectView, QmError> {
    run(&svc, move |s| s.bisect_start(&run_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn bisect_get(svc: Svc<'_>) -> Result<BisectStatus, QmError> {
    run(&svc, move |s| Ok(s.bisect_get())).await
}

#[tauri::command]
#[specta::specta]
pub async fn bisect_preview(svc: Svc<'_>) -> Result<BisectPreview, QmError> {
    run(&svc, move |s| s.bisect_preview()).await
}

#[tauri::command]
#[specta::specta]
pub async fn bisect_launch(
    svc: Svc<'_>,
    method: Option<qm_manager::LaunchMethod>,
) -> Result<GameStatus, QmError> {
    run(&svc, move |s| s.bisect_launch(method)).await
}

#[tauri::command]
#[specta::specta]
pub async fn bisect_answer(svc: Svc<'_>, result: TrialResult) -> Result<BisectView, QmError> {
    run(&svc, move |s| s.bisect_answer(result)).await
}

#[tauri::command]
#[specta::specta]
pub async fn bisect_stop(svc: Svc<'_>) -> Result<BisectView, QmError> {
    run(&svc, move |s| s.bisect_stop()).await
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
pub async fn runs_list(
    svc: Svc<'_>,
    profile_id: String,
) -> Result<Vec<qm_manager::runs::RunEntry>, QmError> {
    run(&svc, move |s| s.runs_list(&profile_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn assets_ledger(
    svc: Svc<'_>,
) -> Result<Loadable<qm_manager::assets_ledger::AssetLedger>, QmError> {
    run(&svc, |s| s.assets_ledger()).await
}

#[tauri::command]
#[specta::specta]
pub async fn asset_choice_preview(
    svc: Svc<'_>,
    key: String,
    winner: String,
) -> Result<qm_service::ops::assets::AssetChoicePreview, QmError> {
    run(&svc, move |s| s.asset_choice_preview(&key, &winner)).await
}

#[tauri::command]
#[specta::specta]
pub async fn asset_choice_clear(svc: Svc<'_>, key: String) -> Result<(), QmError> {
    run(&svc, move |s| s.asset_choice_clear(&key)).await
}

#[tauri::command]
#[specta::specta]
pub async fn faction_choices(
    svc: Svc<'_>,
) -> Result<Loadable<Vec<qm_service::ops::assets::FactionSideChoice>>, QmError> {
    run(&svc, |s| s.faction_choices()).await
}

#[tauri::command]
#[specta::specta]
pub async fn faction_pick_set(
    svc: Svc<'_>,
    side: String,
    faction_id: Option<String>,
) -> Result<(), QmError> {
    run(&svc, move |s| s.faction_pick_set(&side, faction_id)).await
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
pub async fn crash_suspect(
    svc: Svc<'_>,
    crash_id: String,
) -> Result<Option<TrialSuspect>, QmError> {
    run(&svc, move |s| s.crash_suspect(&crash_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn dll_repair(svc: Svc<'_>) -> Result<String, QmError> {
    run(&svc, move |s| s.dll_repair()).await
}

#[tauri::command]
#[specta::specta]
pub async fn overwrite_clear(svc: Svc<'_>) -> Result<u32, QmError> {
    run(&svc, move |s| s.overwrite_clear()).await
}

#[tauri::command]
#[specta::specta]
pub async fn game_update_acknowledge(svc: Svc<'_>) -> Result<(), QmError> {
    run(&svc, move |s| s.game_update_acknowledge()).await
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_sync(svc: Svc<'_>) -> Result<JobId, QmError> {
    run(&svc, move |s| s.workspace_sync()).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_rename(svc: Svc<'_>, id: String, name: String) -> Result<SaveProfile, QmError> {
    run(&svc, move |s| s.save_rename(&id, &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_create(svc: Svc<'_>, name: String) -> Result<SaveProfile, QmError> {
    run(&svc, move |s| s.save_create(&name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_delete(svc: Svc<'_>, id: String) -> Result<(), QmError> {
    run(&svc, move |s| s.save_delete(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_cleanup(
    svc: Svc<'_>,
    id: String,
    fields: Vec<String>,
) -> Result<qm_manager::save_cleanup::SaveCleanup, QmError> {
    run(&svc, move |s| s.save_cleanup(&id, &fields)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_reset(svc: Svc<'_>, id: String) -> Result<(), QmError> {
    run(&svc, move |s| s.save_reset(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_lock_set(svc: Svc<'_>, id: String, locked: bool) -> Result<SaveProfile, QmError> {
    run(&svc, move |s| s.save_lock_set(&id, locked)).await
}

#[tauri::command]
#[specta::specta]
pub async fn game_sprite_get(svc: Svc<'_>, kind: SpriteKind, id: u32) -> Result<String, QmError> {
    run(&svc, move |s| s.game_sprite_get(kind, id)).await
}

/// Largest dossier text accepted for export.
const MAX_DOSSIER_BYTES: usize = 1024 * 1024;

/// Asks where to save a slot's dossier with the native save dialog, then writes `text` there.
/// The path comes only from that dialog, never from the UI. Returns the file name, or `None`
/// when the dialog was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn save_dossier_export(
    app: tauri::AppHandle,
    slot_name: String,
    text: String,
) -> Result<Option<String>, QmError> {
    use tauri_plugin_dialog::DialogExt;
    if text.len() > MAX_DOSSIER_BYTES {
        return Err(QmError::new(
            ErrorKind::InvalidInput,
            "The dossier is too large to export.",
        ));
    }
    let safe: String = slot_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let file_name = format!("{} dossier.txt", safe.trim());
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Text", &["txt"])
            .set_file_name(&file_name)
            .blocking_save_file()
    })
    .await
    .map_err(|_| QmError::new(ErrorKind::Internal, MSG_INTERNAL))?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    std::fs::write(&path, text.as_bytes()).map_err(|e| {
        QmError::new(ErrorKind::Io, "The dossier could not be saved.").with_details(e.to_string())
    })?;
    Ok(path.file_name().map(|n| n.to_string_lossy().into_owned()))
}

#[tauri::command]
#[specta::specta]
pub async fn game_ribbons_get(svc: Svc<'_>) -> Result<Vec<RibbonInfo>, QmError> {
    run(&svc, move |s| s.game_ribbons_get()).await
}

#[tauri::command]
#[specta::specta]
pub async fn mod_auto_update_set(
    svc: Svc<'_>,
    mod_id: String,
    enabled: Option<bool>,
) -> Result<(), QmError> {
    run(&svc, move |s| s.mod_auto_update_set(&mod_id, enabled)).await
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
pub async fn mod_update_install(svc: Svc<'_>, mod_id: String) -> Result<JobId, QmError> {
    run(&svc, move |s| s.mod_update_install(&mod_id)).await
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
