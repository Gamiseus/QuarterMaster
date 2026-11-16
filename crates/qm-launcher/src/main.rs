#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::let_underscore_must_use
)]

// `cargo tauri build` enables `tauri/custom-protocol`, which makes tauri-build drop the `dev`
// cfg; a plain release `cargo build` keeps it and would load the dev server URL.
#[cfg(all(not(debug_assertions), dev))]
compile_error!("Build release with `cargo tauri build` (custom-protocol), not plain cargo build.");

use qm_launcher_lib::commands::{run_startup, Startup};
use qm_launcher_lib::{bindings, logging, navigation, sink::TauriSink};
use qm_manager::QuarterMasterPaths;
use qm_service::{EventSink, ProcessProbe, QmEvent, Service, Win32Probe};
use std::sync::Arc;
use tauri::{Manager, RunEvent};

/// `--restore-game`, run by the uninstaller: takes QuarterMaster's `winmm.dll` and `Mods` link
/// out of the game folder without opening a window. Mods, profiles and saves are not touched.
/// Returns the exit code: 0 done, 1 failed, 2 the game is running.
fn restore_game(paths: QuarterMasterPaths) -> i32 {
    struct Quiet;
    impl EventSink for Quiet {
        fn emit(&self, _event: QmEvent) {}
    }
    if Win32Probe.find_game().is_some() {
        tracing::warn!("--restore-game: the game is running; its folder was left as it is");
        return 2;
    }
    let svc = Service::new(paths, Arc::new(Quiet), Arc::new(Win32Probe));
    match svc.tx_restore_clean() {
        Ok(()) => {
            tracing::info!("--restore-game: the game folder is clean");
            0
        }
        Err(e) => {
            tracing::warn!("--restore-game: {}", e.message);
            1
        }
    }
}

/// `--remove-data`, run by the uninstaller when the user asked for it: cleans the game folder,
/// then deletes `qm-data` beside the program. A workspace mods folder is never deleted, even one
/// inside `qm-data`, and links are removed without touching what they point to. Nothing is
/// logged, since the log is in the folder being deleted.
fn remove_data(paths: QuarterMasterPaths) -> i32 {
    let root = paths.root_dir.clone();
    let beside_program = paths.is_portable && root.file_name().is_some_and(|n| n == "qm-data");
    let workspace = paths.load_settings().workspace_mods_dir;
    let cleaned = restore_game(paths);
    if cleaned == 2 || !beside_program {
        return cleaned.max(1);
    }
    let keep = workspace.filter(|w| w.starts_with(&root));
    match qm_manager::relocate::remove_tree_keeping(&root, keep.as_deref()) {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

/// `--path-add` and `--path-remove`, run by the installer and the uninstaller (as administrator):
/// puts the program's folder on the machine `PATH`, so `qm` works in any terminal, or takes it off.
fn machine_path(add: bool) -> i32 {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    match dir.map(|d| qm_manager::syspath::set_machine_path(&d, add)) {
        Some(Ok(_)) => 0,
        _ => 1,
    }
}

fn main() {
    // Switches for the installer and the uninstaller; none of them opens a window.
    let has = |flag: &str| std::env::args().skip(1).any(|a| a == flag);
    if has("--path-add") || has("--path-remove") {
        std::process::exit(machine_path(has("--path-add")));
    }
    let paths = QuarterMasterPaths::resolve(None);
    if has("--remove-data") {
        std::process::exit(remove_data(paths));
    }
    let log_guard = logging::init(&paths.logs_dir);
    logging::install_panic_hook(paths.logs_dir.clone());
    tracing::info!("QuarterMaster {} starting", env!("CARGO_PKG_VERSION"));
    if has("--restore-game") {
        let code = restore_game(paths);
        drop(log_guard);
        std::process::exit(code);
    }

    let builder = bindings::specta_builder();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // The UI opens links itself (https only); no injected click handler.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(navigation::plugin())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let sink = Arc::new(TauriSink(app.handle().clone()));
            let svc = Service::new(paths, sink, Arc::new(Win32Probe));
            app.manage(svc.clone());
            app.manage(Startup::default());
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("qm-startup".into())
                .spawn(move || {
                    let report = run_startup(|| svc.start());
                    if let Some(summary) = &report.recovered {
                        tracing::info!("startup recovery: {summary}");
                    }
                    let state = handle.state::<Startup>();
                    *state.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(report);
                })?;
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            tracing::error!("tauri failed to start: {e}");
            drop(log_guard);
            return;
        }
    };
    // `App::run` ends with process::exit, so flush the log writer on Exit.
    let mut log_guard = Some(log_guard);
    app.run(move |_handle, event| {
        if let RunEvent::Exit = event {
            tracing::info!("QuarterMaster exiting");
            drop(log_guard.take());
        }
    });
}
