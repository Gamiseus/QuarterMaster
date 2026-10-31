//! Game launch status types.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum GameStage {
    Idle,
    Deploying,
    WaitingForSteam,
    Running { pid: u32 },
    CleaningUp,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct GameStatus {
    pub stage: GameStage,
    pub profile_id: Option<String>,
    pub is_modded: bool,
}

impl GameStatus {
    pub fn idle() -> Self {
        Self {
            stage: GameStage::Idle,
            profile_id: None,
            is_modded: false,
        }
    }

    /// True while a launch or cleanup is in progress.
    pub fn is_busy(&self) -> bool {
        !matches!(self.stage, GameStage::Idle | GameStage::Failed { .. })
    }
}

impl Default for GameStatus {
    fn default() -> Self {
        Self::idle()
    }
}
