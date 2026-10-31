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
use qm_launcher_lib::{bindings, logging, sink::TauriSink};
use qm_manager::QuarterMasterPaths;
use qm_service::{Service, Win32Probe};
use std::sync::Arc;
use tauri::{Manager, RunEvent};

fn main() {
    let paths = QuarterMasterPaths::resolve(None);
    let log_guard = logging::init(&paths.logs_dir);
    logging::install_panic_hook(paths.logs_dir.clone());
    tracing::info!("QuarterMaster {} starting", env!("CARGO_PKG_VERSION"));

    let builder = bindings::specta_builder();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
