//! Process probing abstraction so game-watch logic is testable without a real exe.
use crate::error::QmResult;
use std::time::Duration;

pub trait ProcessProbe: Send + Sync + 'static {
    /// Pid of a running game process, if any.
    fn find_game(&self) -> Option<u32>;
    /// Block until the given pid exits.
    fn wait_for_exit(&self, pid: u32) -> QmResult<()>;
    /// Spawn the game via `spawn_fn`; fakes may skip the closure and return a fake pid.
    fn spawn(&self, spawn_fn: &dyn Fn() -> QmResult<u32>) -> QmResult<u32>;
}

#[derive(Debug, Clone, Copy)]
pub struct WatchTimings {
    pub steam_poll: Duration,
    pub steam_timeout: Duration,
}

impl Default for WatchTimings {
    fn default() -> Self {
        Self {
            steam_poll: Duration::from_millis(500),
            steam_timeout: Duration::from_secs(60),
        }
    }
}

const GAME_EXE: &str = "mnb.exe";

/// Real probe backed by Win32 process APIs.
#[derive(Debug, Default, Clone, Copy)]
pub struct Win32Probe;

impl ProcessProbe for Win32Probe {
    fn find_game(&self) -> Option<u32> {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };
        // SAFETY: plain Win32 calls; the entry buffer is sized via dwSize and the handle is closed.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut found = None;
            let mut more = Process32FirstW(snap, &mut entry).is_ok();
            while more {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if name.eq_ignore_ascii_case(GAME_EXE) {
                    found = Some(entry.th32ProcessID);
                    break;
                }
                more = Process32NextW(snap, &mut entry).is_ok();
            }
            if let Err(e) = CloseHandle(snap) {
                tracing::warn!("CloseHandle(snapshot) failed: {e}");
            }
            found
        }
    }

    fn wait_for_exit(&self, pid: u32) -> QmResult<()> {
        use crate::error::{ErrorKind, QmError};
        use windows::Win32::Foundation::{CloseHandle, WAIT_FAILED};
        use windows::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
        };
        // SAFETY: the handle comes from OpenProcess and is closed before returning.
        unsafe {
            // A pid that cannot be opened has already exited (or was never ours to watch).
            let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
                return Ok(());
            };
            let res = WaitForSingleObject(handle, INFINITE);
            if let Err(e) = CloseHandle(handle) {
                tracing::warn!("CloseHandle(process) failed: {e}");
            }
            if res == WAIT_FAILED {
                return Err(QmError::new(
                    ErrorKind::Internal,
                    format!("Waiting for process {pid} failed."),
                ));
            }
        }
        Ok(())
    }

    fn spawn(&self, spawn_fn: &dyn Fn() -> QmResult<u32>) -> QmResult<u32> {
        spawn_fn()
    }
}
