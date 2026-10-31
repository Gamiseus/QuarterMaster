//! Journaled, recoverable game-folder transactions and winmm.dll ownership checks.
//!
//! Every transaction: write intent to the state journal, perform the change, verify, commit.
//! All of them, and `recover`, hold the single game-folder mutex.
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::QmEvent;
use crate::service::Service;
use qm_manager::{PendingTransaction, StateJournal, UNKNOWN_DLL_ERROR_PREFIX};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

const KIND_DEPLOY: &str = "deploy";
const KIND_RESTORE: &str = "restore_clean";
const KIND_DLL_REPAIR: &str = "dll_repair";
const KIND_SAVE_SWAP: &str = "save_swap";
const KIND_PROFILE_SWAP: &str = "profile_swap";

/// `to_id` for transactions that have no profile or save target.
const TARGET_VANILLA: &str = "vanilla";
const TARGET_DLL: &str = "winmm.dll";

const HASHES_FILE: &str = "qm-dll-hashes.json";
const MSG_UNKNOWN_DLL: &str =
    "The winmm.dll in the game folder is not a QuarterMaster build. It was left in place.";
const MSG_DLL_MISSING: &str = "winmm.dll was not found next to qm-launcher.exe.";
const MSG_DLL_MISMATCH: &str =
    "The winmm.dll in the game folder does not match the QuarterMaster build that was copied.";
const SUMMARY_UNKNOWN_DLL: &str = " An unknown winmm.dll was left in place.";

fn unknown_dll() -> QmError {
    QmError::new(ErrorKind::Integrity, MSG_UNKNOWN_DLL)
}

fn is_unknown_dll(msg: &str) -> bool {
    msg.starts_with(UNKNOWN_DLL_ERROR_PREFIX)
}

/// `Integrity` unless the installed DLL hash equals the source hash.
fn verify_installed(installed: &str, source: &str) -> QmResult<()> {
    if installed.eq_ignore_ascii_case(source) {
        Ok(())
    } else {
        Err(QmError::new(ErrorKind::Integrity, MSG_DLL_MISMATCH)
            .with_details(format!("expected sha256 {source}, found {installed}")))
    }
}

fn hash_file(path: &Path) -> QmResult<String> {
    let bytes = fs::read(path).or_kind(ErrorKind::Io)?;
    Ok(qm_schema::sha256_hex(&bytes))
}

impl Service {
    fn lock_game_folder(&self) -> MutexGuard<'_, ()> {
        self.inner
            .game_folder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// The QuarterMaster `winmm.dll` to deploy: `QM_WINMM_DLL` if set, else next to the exe.
    pub fn source_dll(&self) -> QmResult<PathBuf> {
        let _g = self.lock_game_folder();
        source_dll_path()
    }

    /// Hashes of every known QuarterMaster `winmm.dll` build.
    pub fn known_dll_hashes(&self) -> HashSet<String> {
        let _g = self.lock_game_folder();
        self.known_hashes()
    }

    /// Deploy `profile_id` into the game folder (Mods junction plus winmm.dll).
    pub fn tx_deploy(&self, profile_id: &str) -> QmResult<()> {
        let _g = self.lock_game_folder();
        let game_dir = self.game_dir()?;
        self.check_game_dll(&game_dir)?;
        let source = source_dll_path()?;
        let source_hash = hash_file(&source)?;
        // Record before copying so a crash after the copy still leaves a recognised DLL.
        self.record_hash(&source_hash)?;

        // Stage the profile first: activate_profile journals its own profile_swap.
        let paths = self.paths();
        if !paths.profiles_dir.join(profile_id).join("Mods").is_dir() {
            paths.activate_profile(profile_id).or_kind(ErrorKind::Io)?;
        }

        self.begin(KIND_DEPLOY, None, profile_id)?;
        let result = (|| {
            paths
                .deploy_profile_to_game(profile_id, &source)
                .or_kind(ErrorKind::Io)?;
            let dll = game_dir.join("winmm.dll");
            if !dll.is_file() {
                return Err(QmError::new(
                    ErrorKind::Integrity,
                    "Deploy finished but winmm.dll is missing from the game folder.",
                ));
            }
            verify_installed(&hash_file(&dll)?, &source_hash)?;
            if fs::symlink_metadata(game_dir.join("Mods")).is_err() {
                return Err(QmError::new(
                    ErrorKind::Integrity,
                    "Deploy finished but the Mods link is missing from the game folder.",
                ));
            }
            Ok(())
        })();

        match result {
            Ok(()) => self.commit(),
            Err(e) => {
                // Roll back to vanilla; leave the entry for startup recovery if that fails too.
                match self.restore_tolerating_unknown() {
                    Ok(_) => {
                        if let Err(ce) = self.clear_pending() {
                            tracing::warn!("failed to clear journal after deploy rollback: {ce}");
                        }
                        Err(e)
                    }
                    Err(re) => {
                        tracing::warn!("deploy rollback failed: {re}");
                        let prior = e
                            .details
                            .clone()
                            .map(|d| format!("{d}; "))
                            .unwrap_or_default();
                        Err(e.with_details(format!(
                            "{prior}Rollback failed, the game folder may be partly modded: {}",
                            re.message
                        )))
                    }
                }
            }
        }
    }

    /// Restore the game folder to vanilla, leaving any unknown winmm.dll in place.
    pub fn tx_restore_clean(&self) -> QmResult<()> {
        let _g = self.lock_game_folder();
        self.begin(KIND_RESTORE, None, TARGET_VANILLA)?;
        let known = self.known_hashes();
        match self.paths().restore_clean_game_dir(&known) {
            Ok(()) => self.commit(),
            Err(m) if is_unknown_dll(&m) => {
                // The junction step ran; the folder is as clean as we may make it.
                tracing::warn!("{m}");
                self.commit()?;
                Err(unknown_dll().with_details(m))
            }
            // Pending entry stays so startup recovery completes the restore.
            Err(m) => Err(QmError::new(ErrorKind::Io, m)),
        }
    }

    /// Reinstall the QuarterMaster winmm.dll; returns the installed hash.
    pub fn tx_repair_dll(&self) -> QmResult<String> {
        let _g = self.lock_game_folder();
        let game_dir = self.game_dir()?;
        self.check_game_dll(&game_dir)?;
        let source = source_dll_path()?;
        let source_hash = hash_file(&source)?;
        self.record_hash(&source_hash)?;

        self.begin(KIND_DLL_REPAIR, None, TARGET_DLL)?;
        let result = self
            .paths()
            .repair_winmm_dll(&source)
            .or_kind(ErrorKind::Io)
            .and_then(|hash| verify_installed(&hash, &source_hash).map(|()| hash));
        match result {
            Ok(hash) => {
                self.commit()?;
                Ok(hash)
            }
            Err(e) => {
                // The copy is atomic, so the folder holds the old or the new DLL either way.
                if let Err(ce) = self.clear_pending() {
                    tracing::warn!("failed to clear journal after dll repair error: {ce}");
                }
                Err(e)
            }
        }
    }

    /// Switch the active save slot, with an optional auto-backup first.
    pub fn tx_switch_save(&self, save_id: &str) -> QmResult<()> {
        let _g = self.lock_game_folder();
        let paths = self.paths();
        let settings = paths.load_settings();
        let current = settings.active_save_profile_id.clone();
        if settings.auto_backup_saves {
            paths
                .create_save_backup(&current, "Auto-backup before save switch")
                .or_kind(ErrorKind::Io)?;
        }
        self.begin(KIND_SAVE_SWAP, Some(current.clone()), save_id)?;
        match paths.activate_save_profile(save_id) {
            Ok(()) => self.commit(),
            Err(m) => {
                match paths.activate_save_profile(&current) {
                    Ok(()) => {
                        if let Err(ce) = self.clear_pending() {
                            tracing::warn!("failed to clear journal after save rollback: {ce}");
                        }
                    }
                    Err(re) => tracing::warn!("save switch rollback failed: {re}"),
                }
                Err(QmError::new(ErrorKind::Io, m))
            }
        }
    }

    /// Complete or roll back an interrupted transaction left in the journal.
    pub fn recover(&self) -> QmResult<Option<String>> {
        let _g = self.lock_game_folder();
        let paths = self.paths();
        let Some(tx) = StateJournal::load(&paths.journal_file).pending_transaction else {
            return Ok(None);
        };
        let left_unknown = self.apply_recovery(&tx)?;
        self.clear_pending()?;
        let mut summary = format!(
            "Recovered from an interrupted {} (target '{}').",
            tx.kind, tx.to_id
        );
        if left_unknown {
            summary.push_str(SUMMARY_UNKNOWN_DLL);
        }
        self.inner.sink.emit(QmEvent::Recovered {
            summary: summary.clone(),
        });
        Ok(Some(summary))
    }

    /// Applies the recovery action; returns true when an unknown winmm.dll was left in place.
    fn apply_recovery(&self, tx: &PendingTransaction) -> QmResult<bool> {
        let paths = self.paths();
        let fallback = tx.from_id.as_deref().unwrap_or(&tx.to_id);
        match tx.kind.as_str() {
            KIND_DEPLOY | KIND_RESTORE => self.restore_tolerating_unknown(),
            KIND_DLL_REPAIR => {
                let game_dir = self.game_dir()?;
                if let Err(e) = self.check_game_dll(&game_dir) {
                    tracing::warn!("dll_repair recovery skipped: {e}");
                    return Ok(e.kind == ErrorKind::Integrity);
                }
                let source = match source_dll_path() {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("dll_repair recovery skipped: {e}");
                        return Ok(false);
                    }
                };
                let source_hash = hash_file(&source)?;
                self.record_hash(&source_hash)?;
                let hash = paths.repair_winmm_dll(&source).or_kind(ErrorKind::Io)?;
                verify_installed(&hash, &source_hash)?;
                Ok(false)
            }
            KIND_SAVE_SWAP => paths
                .activate_save_profile(fallback)
                .or_kind(ErrorKind::Io)
                .map(|()| false),
            KIND_PROFILE_SWAP => paths
                .activate_profile(fallback)
                .map(|_| false)
                .or_kind(ErrorKind::Io),
            other => {
                tracing::warn!("unknown journal transaction kind '{other}'; clearing it");
                Ok(false)
            }
        }
    }

    /// Restore clean; an unknown DLL is left in place and reported as `Ok(true)`, not a failure.
    fn restore_tolerating_unknown(&self) -> QmResult<bool> {
        match self.paths().restore_clean_game_dir(&self.known_hashes()) {
            Ok(()) => Ok(false),
            Err(m) if is_unknown_dll(&m) => {
                tracing::warn!("{m}");
                Ok(true)
            }
            Err(m) => Err(QmError::new(ErrorKind::Io, m)),
        }
    }

    fn game_dir(&self) -> QmResult<PathBuf> {
        let settings = self.paths().load_settings();
        settings
            .game_exe_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                QmError::new(
                    ErrorKind::NotFound,
                    "Mud and Blood executable (mnb.exe) path is not configured.",
                )
            })
    }

    /// `Integrity` when `<game_dir>/winmm.dll` exists and is not a known build.
    fn check_game_dll(&self, game_dir: &Path) -> QmResult<()> {
        let dll = game_dir.join("winmm.dll");
        if !dll.is_file() {
            return Ok(());
        }
        let hash = hash_file(&dll)?;
        if self.known_hashes().contains(&hash) {
            Ok(())
        } else {
            Err(unknown_dll().with_details(format!("sha256 {hash}")))
        }
    }

    fn hashes_path(&self) -> PathBuf {
        self.paths().cache_dir.join(HASHES_FILE)
    }

    fn recorded_hashes(&self) -> Vec<String> {
        fs::read_to_string(self.hashes_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|h| h.to_ascii_lowercase())
            .collect()
    }

    fn known_hashes(&self) -> HashSet<String> {
        let mut set: HashSet<String> = self.recorded_hashes().into_iter().collect();
        if let Ok(h) = source_dll_path().and_then(|p| hash_file(&p)) {
            set.insert(h);
        }
        set
    }

    fn record_hash(&self, hash: &str) -> QmResult<()> {
        let hash = hash.to_ascii_lowercase();
        let mut list = self.recorded_hashes();
        if list.contains(&hash) {
            return Ok(());
        }
        list.push(hash);
        let path = self.hashes_path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).or_kind(ErrorKind::Io)?;
        }
        let json = serde_json::to_string_pretty(&list)
            .map_err(|e| QmError::new(ErrorKind::Internal, e.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json).or_kind(ErrorKind::Io)?;
        fs::rename(&tmp, &path).or_kind(ErrorKind::Io)
    }

    fn begin(&self, kind: &str, from_id: Option<String>, to_id: &str) -> QmResult<()> {
        let file = &self.paths().journal_file;
        StateJournal::load(file)
            .begin_transaction(file, kind, from_id, to_id, None)
            .or_kind(ErrorKind::Io)
    }

    fn commit(&self) -> QmResult<()> {
        let file = &self.paths().journal_file;
        StateJournal::load(file)
            .commit_transaction(file)
            .or_kind(ErrorKind::Io)
    }

    /// Drop the pending entry without applying its commit side effects.
    fn clear_pending(&self) -> QmResult<()> {
        let file = &self.paths().journal_file;
        let mut journal = StateJournal::load(file);
        journal.pending_transaction = None;
        journal.save(file).or_kind(ErrorKind::Io)
    }
}

fn source_dll_path() -> QmResult<PathBuf> {
    let candidate = match std::env::var_os("QM_WINMM_DLL").filter(|v| !v.is_empty()) {
        Some(v) => Some(PathBuf::from(v)),
        None => std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("winmm.dll"))),
    };
    candidate
        .filter(|p| p.is_file())
        .ok_or_else(|| QmError::new(ErrorKind::NotFound, MSG_DLL_MISSING))
}
