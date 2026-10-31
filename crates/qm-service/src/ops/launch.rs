//! Game launch state machine and the exit watcher that cleans the game folder.
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::QmEvent;
use crate::game::{GameStage, GameStatus};
use crate::service::Service;
use qm_manager::LaunchMethod;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::MutexGuard;
use std::time::Instant;

const MSG_STEAM_TIMEOUT: &str = "Mud and Blood did not start within 60 seconds.";
const BACKUP_LABEL: &str = "Auto-backup before game launch";
const MSG_LOST_TRACK: &str =
    "Lost track of the running game. Restart QuarterMaster after closing it.";

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LaunchRequest {
    pub profile_id: String,
    pub method: Option<LaunchMethod>,
}

impl Service {
    fn lock_status(&self) -> MutexGuard<'_, GameStatus> {
        self.inner.game.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Current status; a game found running while nothing tracks it is attached first.
    pub fn game_status(&self) -> GameStatus {
        let st = self.lock_status().clone();
        if st.is_busy() {
            return st;
        }
        match self.inner.probe.find_game() {
            Some(pid) => {
                self.attach_to_running(pid);
                self.lock_status().clone()
            }
            None => st,
        }
    }

    /// Deploy (or restore for vanilla), spawn the game and start the exit watcher.
    pub fn game_launch(&self, req: LaunchRequest) -> QmResult<GameStatus> {
        self.ensure_not_running()?;
        let paths = self.paths();
        let profile = paths
            .get_profile(&req.profile_id)
            .or_kind(ErrorKind::NotFound)?;
        let settings = paths.load_settings();
        let method = req
            .method
            .or(profile.launch_method)
            .unwrap_or(settings.default_launch_method);
        let modded = method != LaunchMethod::Vanilla && !profile.enabled_mods.is_empty();

        self.claim(GameStatus {
            stage: GameStage::Deploying,
            profile_id: Some(profile.id.clone()),
            is_modded: modded,
        })?;

        let spawned: QmResult<u32> = (|| {
            paths.activate_profile(&profile.id).or_kind(ErrorKind::Io)?;
            if modded {
                self.tx_deploy(&profile.id)?;
            } else {
                self.tx_restore_clean()?;
            }
            if settings.auto_backup_saves {
                paths
                    .create_save_backup(&profile.linked_save_profile_id, BACKUP_LABEL)
                    .or_kind(ErrorKind::Io)?;
            }
            self.inner.probe.spawn(&|| {
                paths
                    .spawn_game_process(&profile, method)
                    .or_kind(ErrorKind::Io)
            })
        })();
        let spawned = spawned.inspect_err(|e| self.fail_with_restore(e.message.clone()))?;

        let (st, started) = if method == LaunchMethod::SteamUrl {
            let st = self.set_stage(GameStage::WaitingForSteam);
            let svc = self.clone();
            (
                st,
                spawn_thread("qm-steam-wait", move || svc.wait_for_steam()),
            )
        } else {
            let st = self.set_stage(GameStage::Running { pid: spawned });
            let svc = self.clone();
            (
                st,
                spawn_thread("qm-game-watch", move || svc.watch(spawned)),
            )
        };
        match started {
            Ok(()) => Ok(st),
            Err(e) => {
                // The game may be running; leave the folder alone.
                tracing::warn!("{e}");
                self.set_stage(GameStage::Failed {
                    reason: MSG_LOST_TRACK.to_string(),
                });
                Err(QmError::new(ErrorKind::Internal, MSG_LOST_TRACK).with_details(e.message))
            }
        }
    }

    /// Restore the game folder to vanilla; refused while the game runs.
    pub fn game_restore_clean(&self) -> QmResult<()> {
        self.ensure_not_running()?;
        self.claim(GameStatus {
            stage: GameStage::CleaningUp,
            profile_id: None,
            is_modded: false,
        })?;
        let result = self.tx_restore_clean();
        self.set_status(GameStatus::idle());
        result
    }

    /// `GameRunning` when a launch is in progress or an untracked game is found (which is attached).
    pub(crate) fn ensure_not_running(&self) -> QmResult<()> {
        if self.game_status().is_busy() {
            Err(QmError::game_running())
        } else {
            Ok(())
        }
    }

    /// Track a game that was already running (e.g. found at startup) and clean up when it exits.
    pub(crate) fn attach_to_running(&self, pid: u32) {
        let settings = self.paths().load_settings();
        let is_modded = settings
            .game_exe_path
            .as_deref()
            .and_then(Path::parent)
            .is_some_and(|d| std::fs::symlink_metadata(d.join("Mods")).is_ok());
        let status = GameStatus {
            stage: GameStage::Running { pid },
            profile_id: Some(settings.active_profile_id),
            is_modded,
        };
        {
            let mut g = self.lock_status();
            if g.is_busy() {
                return;
            }
            *g = status.clone();
        }
        self.emit(status);
        let svc = self.clone();
        if let Err(e) = spawn_thread("qm-game-watch", move || svc.watch(pid)) {
            self.set_stage(GameStage::Failed { reason: e.message });
        }
    }

    /// Atomically move from a non-busy stage to `status`.
    fn claim(&self, status: GameStatus) -> QmResult<()> {
        {
            let mut g = self.lock_status();
            if g.is_busy() {
                return Err(QmError::game_running());
            }
            *g = status.clone();
        }
        self.emit(status);
        Ok(())
    }

    /// Poll for the game process Steam starts.
    fn wait_for_steam(&self) {
        let deadline = Instant::now() + self.timings.steam_timeout;
        loop {
            if let Some(pid) = self.inner.probe.find_game() {
                self.set_stage(GameStage::Running { pid });
                self.watch(pid);
                return;
            }
            if Instant::now() >= deadline {
                self.fail_with_restore(MSG_STEAM_TIMEOUT.to_string());
                return;
            }
            std::thread::sleep(self.timings.steam_poll);
        }
    }

    /// Block until `pid` exits, then clean the game folder. Holds no lock while waiting.
    fn watch(&self, pid: u32) {
        if let Err(e) = self.inner.probe.wait_for_exit(pid) {
            tracing::warn!("waiting for game process {pid} failed: {e}");
            // Only touch the folder when the game is really gone.
            if self.inner.probe.find_game().is_none() {
                self.fail_with_restore(e.message);
            } else {
                self.set_stage(GameStage::Failed { reason: e.message });
            }
            return;
        }
        self.set_stage(GameStage::CleaningUp);
        let result = self
            .recover()
            .map(|_| ())
            .and_then(|()| self.tx_restore_clean());
        match result {
            Ok(()) => {
                self.set_status(GameStatus::idle());
            }
            Err(e) => {
                tracing::warn!("cleanup after game exit failed: {e}");
                self.set_stage(GameStage::Failed { reason: e.message });
            }
        }
    }

    /// Attempt a restore, then enter `Failed` (restore first so the folder is clean when observed).
    /// An unknown winmm.dll left in place (Integrity) does not count as a failed restore.
    fn fail_with_restore(&self, reason: String) {
        let reason = match self.tx_restore_clean() {
            Ok(()) => reason,
            Err(e) if e.kind == ErrorKind::Integrity => {
                tracing::warn!("{e}");
                reason
            }
            Err(e) => {
                tracing::warn!("restore after failed launch did not complete: {e}");
                format!(
                    "{}. Restoring the game folder also failed: {}",
                    reason.trim_end_matches('.'),
                    e.message
                )
            }
        };
        self.set_stage(GameStage::Failed { reason });
    }

    fn set_stage(&self, stage: GameStage) -> GameStatus {
        let status = {
            let mut g = self.lock_status();
            g.stage = stage;
            g.clone()
        };
        self.emit(status.clone());
        status
    }

    fn set_status(&self, status: GameStatus) -> GameStatus {
        *self.lock_status() = status.clone();
        self.emit(status.clone());
        status
    }

    fn emit(&self, status: GameStatus) {
        self.inner.sink.emit(QmEvent::GameStatusChanged { status });
    }
}

fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) -> QmResult<()> {
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(f)
        .map(|_| ())
        .map_err(|e| QmError::new(ErrorKind::Internal, format!("Failed to start {name}: {e}")))
}
