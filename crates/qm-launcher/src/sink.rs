//! Forwards service events to the webview.
use qm_service::{EventSink, QmEvent};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Tauri event name for every service event.
pub const QM_EVENT: &str = "qm://event";

/// Bindings-only wrapper so `QmEvent` can be registered as a typed tauri-specta event without
/// adding a Tauri dependency to `qm-service`. Serializes exactly like `QmEvent`.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct QmEventPayload(pub QmEvent);

impl tauri_specta::Event for QmEventPayload {
    const NAME: &'static str = QM_EVENT;
}

pub struct TauriSink(pub AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: QmEvent) {
        if let Err(e) = self.0.emit(QM_EVENT, &event) {
            tracing::warn!("failed to emit {QM_EVENT}: {e}");
        }
    }
}
