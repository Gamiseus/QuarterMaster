#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::let_underscore_must_use
)]
//! Tauri-free application layer for QuarterMaster.

mod cache;
pub mod error;
pub mod events;
pub mod game;
mod game_folder;
pub mod jobs;
pub mod ops;
pub mod process;
pub mod service;
pub mod testing;

pub use cache::{Cached, Loadable, VanillaFiles};
pub use error::{ErrorKind, OrKind, QmError, QmResult};
pub use events::{EventSink, LogSource, QmEvent, Scope};
pub use game::{GameStage, GameStatus};
pub use jobs::{JobCtx, JobId, JobInfo, JobKind, JobRunner};
pub use ops::launch::LaunchRequest;
pub use process::{ProcessProbe, WatchTimings, Win32Probe};
pub use service::Service;
