#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::let_underscore_must_use
)]
//! Tauri-free application layer for QuarterMaster.

mod bisect;
mod cache;
pub mod dto;
pub mod error;
pub mod events;
pub mod game;
mod game_folder;
mod github;
mod index;
pub mod jobs;
mod migrate;
pub mod net;
pub mod ops;
pub mod process;
mod refresh;
pub mod service;
pub mod startup;
pub mod testing;
mod watch;

pub use bisect::{
    BisectCandidate, BisectEnd, BisectPreview, BisectStatus, BisectTrial, BisectView,
    CandidateSource, NextTrial, TrialResult,
};
pub use cache::{Cached, Loadable, VanillaFiles};
pub use dto::{
    ASite, DriftDiff, FactionChange, FactionPick, FallbackNotice, FixChangeDraft, FixDraft,
    FixStatus, FixStatusChange, FixSuggestion, FixView, FixesView, JournalClash, JournalUndoChoice,
    JournalUndoPreview, JournalView, LineSource, PatchLabel, RegionOption, RegionOptions,
    RegionRef, RegionSite, RelationWarning, UndoPreview,
};
pub use dto::{
    AnchorCandidate, AppInfo, AssetFile, CatalogEntry, CatalogQuery, CatalogResult,
    CompatChangesView, CompatEquivalence, CompatExportPreview, CompatExportRequest,
    CompatExportResult, CompatFile, CompatRefusal, CompatRequirement, ConfigResetMode,
    ConflictPolicy, FileDiff, FileStat, GameInfo, GameUpdateDetected, HealthLogLine,
    HealthPatchReport, HealthReport, ImportRequest, LaunchMethodChoice, LauncherUpdateInfo,
    LoadOrderUpdate, ModDetail, ModPatchStatus, ModSummary, ModUpdateInfo, NewProfile, PatchDetail,
    PatchFilePosition, PatchState, PreviewChanges, PreviewResult, ProfileSettingsPatch,
    ProfileSummary, RevealKind, SaveSlotSummary, SettingsPatch, SettingsView, SimulationOverview,
    StackChange, SwitchPreview, UpdatesReport,
};
pub use error::{ErrorKind, OrKind, QmError, QmResult};
pub use events::{EventSink, LogSource, QmEvent, Scope};
pub use game::{GameStage, GameStatus};
pub use jobs::{JobCtx, JobId, JobInfo, JobKind, JobRunner};
pub use ops::launch::{
    DiagnosticRunPreview, DiagnosticRunRequest, DiagnosticRunResult, LaunchRequest, SuspectKind,
    TrialRequest, TrialSuspect, WatchSuggestion,
};
pub use process::{ProcessProbe, WatchTimings, Win32Probe};
pub use service::Service;
pub use startup::StartupReport;
pub use watch::{Classified, WatchHandle};
