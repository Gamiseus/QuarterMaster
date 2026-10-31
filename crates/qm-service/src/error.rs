use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    NotFound,
    InvalidInput,
    Conflict,
    GameRunning,
    Io,
    Network,
    Integrity,
    Cancelled,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, thiserror::Error)]
#[error("{message}")]
pub struct QmError {
    pub kind: ErrorKind,
    pub message: String,
    pub details: Option<String>,
}

impl QmError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    pub fn game_running() -> Self {
        Self::new(
            ErrorKind::GameRunning,
            "Mud and Blood is running. Close the game first.",
        )
    }
}

pub type QmResult<T> = Result<T, QmError>;

pub trait OrKind<T> {
    fn or_kind(self, kind: ErrorKind) -> QmResult<T>;
}

impl<T> OrKind<T> for Result<T, String> {
    fn or_kind(self, kind: ErrorKind) -> QmResult<T> {
        self.map_err(|m| QmError::new(kind, m))
    }
}

impl<T> OrKind<T> for Result<T, std::io::Error> {
    fn or_kind(self, kind: ErrorKind) -> QmResult<T> {
        self.map_err(|e| QmError::new(kind, e.to_string()))
    }
}
