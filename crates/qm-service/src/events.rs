//! Typed events pushed from the service layer to the UI.
use crate::error::QmError;
use crate::game::GameStatus;
use crate::jobs::{JobId, JobKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Mods,
    Profiles,
    LoadOrder,
    Saves,
    Crashes,
    Health,
    Settings,
    Updates,
    Catalog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LogSource {
    Runtime,
    RuntimeErrors,
    Launcher,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QmEvent {
    GameStatusChanged {
        status: GameStatus,
    },
    JobProgress {
        id: JobId,
        kind: JobKind,
        progress: f32,
        label: String,
    },
    JobFinished {
        id: JobId,
        kind: JobKind,
        error: Option<QmError>,
        output: Option<String>,
    },
    Invalidated {
        scope: Scope,
    },
    LogLine {
        source: LogSource,
        line: String,
    },
    Recovered {
        summary: String,
    },
}

pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: QmEvent);
}
