//! Save slot queries and actions. Every action ends with `invalidate(Saves)`.
use crate::dto::SaveSlotSummary;
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::service::Service;
use qm_manager::{SaveBackupEntry, SaveProfile};
use qm_schema::{compare_saves, SaveComparison, SaveSummary};

const RESTORE_BACKUP_LABEL: &str = "Auto-backup before restore";

impl Service {
    /// Slot metadata only; save files are not parsed.
    pub fn saves_list(&self) -> Vec<SaveSlotSummary> {
        let active = self.paths().load_settings().active_save_profile_id;
        self.paths()
            .list_save_profiles()
            .into_iter()
            .map(|p| SaveSlotSummary {
                is_active: p.id == active,
                profile: p,
            })
            .collect()
    }

    /// Parsed summary of the slot's main save, cached per slot until `invalidate(Saves)`.
    pub fn save_detail(&self, id: &str) -> QmResult<SaveSummary> {
        let cached = self
            .inner
            .caches
            .save_detail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned();
        if let Some(s) = cached {
            return Ok(s);
        }
        self.save_slot(id)?;
        let gen = self.generations().saves;
        let summary = self.paths().inspect_save_profile(id).map_err(|m| {
            QmError::new(ErrorKind::Io, "The save file could not be read.").with_details(m)
        })?;
        self.with_gens(|g| {
            if g.saves == gen {
                self.inner
                    .caches
                    .save_detail
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.to_string(), summary.clone());
            }
        });
        Ok(summary)
    }

    /// Compare slot `a` (older) with slot `b` (newer).
    pub fn saves_compare(&self, a: &str, b: &str) -> QmResult<SaveComparison> {
        let older = self.save_detail(a)?;
        let newer = self.save_detail(b)?;
        Ok(compare_saves(&older, &newer))
    }

    pub fn save_activate(&self, id: &str) -> QmResult<()> {
        self.ensure_not_running()?;
        self.save_slot(id)?;
        let result = self.tx_switch_save(id);
        self.invalidate(Scope::Saves);
        result
    }

    pub fn save_clone(&self, src_id: &str, new_id: &str, new_name: &str) -> QmResult<SaveProfile> {
        self.save_slot(src_id)?;
        let result = self
            .paths()
            .clone_save_profile(src_id, new_id, new_name)
            .map_err(clone_error);
        self.invalidate(Scope::Saves);
        result
    }

    pub fn save_backup(&self, id: &str, label: &str) -> QmResult<SaveBackupEntry> {
        self.save_slot(id)?;
        let result = self
            .paths()
            .create_save_backup(id, label)
            .or_kind(ErrorKind::Io);
        self.invalidate(Scope::Saves);
        result
    }

    /// Restore `backup_id` into slot `id`, backing up the current state first when
    /// `auto_backup_saves` is on.
    pub fn save_restore(&self, id: &str, backup_id: &str) -> QmResult<()> {
        self.ensure_not_running()?;
        self.save_slot(id)?;
        let snap = self.backup_dir(id, backup_id)?;
        let result = self.restore_with_backup(id, backup_id, &snap);
        self.invalidate(Scope::Saves);
        result
    }

    pub fn save_delete_backup(&self, id: &str, backup_id: &str) -> QmResult<()> {
        self.save_slot(id)?;
        self.backup_dir(id, backup_id)?;
        let result = self
            .paths()
            .delete_save_backup(id, backup_id)
            .or_kind(ErrorKind::Io);
        self.invalidate(Scope::Saves);
        result
    }

    fn restore_with_backup(
        &self,
        id: &str,
        backup_id: &str,
        snap: &std::path::Path,
    ) -> QmResult<()> {
        if self.paths().load_settings().auto_backup_saves {
            // Creating a backup prunes the oldest ones, which may be the one being restored.
            let keep = read_dir_files(snap)?;
            self.paths()
                .create_save_backup(id, RESTORE_BACKUP_LABEL)
                .or_kind(ErrorKind::Io)?;
            if !snap.is_dir() {
                std::fs::create_dir_all(snap).or_kind(ErrorKind::Io)?;
                for (name, bytes) in &keep {
                    std::fs::write(snap.join(name), bytes).or_kind(ErrorKind::Io)?;
                }
            }
        }
        self.paths()
            .restore_save_backup(id, backup_id)
            .or_kind(ErrorKind::Io)
    }

    /// The slot `id`, or `NotFound`.
    fn save_slot(&self, id: &str) -> QmResult<SaveProfile> {
        self.paths()
            .list_save_profiles()
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| {
                QmError::new(
                    ErrorKind::NotFound,
                    format!("Save slot '{id}' was not found."),
                )
            })
    }

    fn backup_dir(&self, id: &str, backup_id: &str) -> QmResult<std::path::PathBuf> {
        let dir = self
            .paths()
            .saves_dir
            .join(id)
            .join("backups")
            .join(backup_id);
        if is_plain_name(backup_id) && dir.is_dir() {
            Ok(dir)
        } else {
            Err(QmError::new(
                ErrorKind::NotFound,
                format!("Backup '{backup_id}' was not found."),
            ))
        }
    }
}

fn is_plain_name(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', ':'])
}

/// Top-level files of a backup snapshot (save files plus `backup_meta.json`).
fn read_dir_files(dir: &std::path::Path) -> QmResult<Vec<(std::ffi::OsString, Vec<u8>)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).or_kind(ErrorKind::Io)? {
        let entry = entry.or_kind(ErrorKind::Io)?;
        if entry.path().is_file() {
            out.push((
                entry.file_name(),
                std::fs::read(entry.path()).or_kind(ErrorKind::Io)?,
            ));
        }
    }
    Ok(out)
}

/// Map `clone_save_profile` errors; the manager reports them only by message.
fn clone_error(msg: String) -> QmError {
    let kind = if msg.contains("already exists") {
        ErrorKind::Conflict
    } else if msg.contains("not found") {
        ErrorKind::NotFound
    } else {
        ErrorKind::InvalidInput
    };
    QmError::new(kind, msg)
}
