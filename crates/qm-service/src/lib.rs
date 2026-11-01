#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::let_underscore_must_use
)]
//! Tauri-free application layer for QuarterMaster.

mod cache;
pub mod dto;
pub mod error;
pub mod events;
pub mod game;
mod game_folder;
pub mod jobs;
pub mod net;
pub mod ops;
pub mod process;
mod refresh;
pub mod service;
pub mod startup;
pub mod testing;
mod watch;

pub use cache::{Cached, Loadable, VanillaFiles};
pub use dto::{
    AppInfo, CatalogEntry, CatalogQuery, CatalogResult, ConfigResetMode, ConflictPolicy, FileDiff,
    GameInfo, GameUpdateDetected, HealthLogLine, HealthPatchReport, HealthReport, ImportRequest,
    LaunchMethodChoice, LauncherUpdateInfo, LoadOrderUpdate, ModDetail, ModPatchStatus, ModSummary,
    ModUpdateInfo, NewProfile, PatchState, ProfileSettingsPatch, ProfileSummary, RevealKind,
    SaveSlotSummary, SettingsPatch, SettingsView, SimulationOverview, SwitchPreview, UpdatesReport,
};
pub use error::{ErrorKind, OrKind, QmError, QmResult};
pub use events::{EventSink, LogSource, QmEvent, Scope};
pub use game::{GameStage, GameStatus};
pub use jobs::{JobCtx, JobId, JobInfo, JobKind, JobRunner};
pub use ops::launch::LaunchRequest;
pub use process::{ProcessProbe, WatchTimings, Win32Probe};
pub use service::Service;
pub use startup::StartupReport;
pub use watch::{Classified, WatchHandle};
