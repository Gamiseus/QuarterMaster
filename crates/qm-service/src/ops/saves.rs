//! Save slot queries and actions. Every action ends with `invalidate(Saves)`.
use crate::dto::SaveSlotSummary;
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::service::Service;
use qm_manager::save_cleanup::{leftovers_of, SaveCleanup};
use qm_manager::save_fields::LeftoverFields;
use qm_manager::{SaveBackupEntry, SaveProfile};
use qm_schema::{compare_saves, SaveComparison, SaveSummary};

const RESTORE_BACKUP_LABEL: &str = "Auto-backup before restore";
const MSG_LOST_STAMP: &str = "The save file lost its modded stamp; it stays marked as modded.";
const MSG_CLEANUP: &str = "The save was not cleaned up. It is unchanged.";
const MSG_DELETE_ACTIVE: &str = "Switch to another slot before deleting this one.";
const MSG_SWITCH_LOCKED: &str =
    "This save slot is locked as vanilla, and the active profile loads mods. Pick another slot, or unlock this one.";
const MSG_VANILLA_DELETE: &str = "The Vanilla save slot cannot be deleted. Reset it instead.";

impl Service {
    /// Slot metadata; save files are not parsed, only searched for the `_qm_meta` stamp. A stamp
    /// marks the slot as modded for good; a modded slot whose save lost its stamp gets a note.
    pub fn saves_list(&self) -> Vec<SaveSlotSummary> {
        let paths = self.paths();
        let active = paths.load_settings().active_save_profile_id;
        paths
            .list_save_profiles()
            .into_iter()
            .map(|mut p| {
                let stamped = !p.builtin && paths.slot_has_qm_stamp(&p.id);
                let mut tamper_note = None;
                if stamped && !p.tags.modded {
                    if paths.mark_slot_modded(&p.id).is_ok() {
                        p.tags.modded = true;
                    }
                } else if p.tags.modded && !stamped && paths.slot_save_file(&p.id).is_some() {
                    tamper_note = Some(MSG_LOST_STAMP.to_string());
                }
                // Only a stamped save is parsed, once per change of the saves.
                let leftover = stamped.then(|| self.save_leftover(&p.id)).flatten();
                SaveSlotSummary {
                    is_active: p.id == active,
                    profile: p,
                    tamper_note,
                    leftover,
                }
            })
            .collect()
    }

    /// The values slot `id`'s save holds from mods no profile on the slot loads (D86). The
    /// stamp is read once per change of the saves; a save that cannot be read has none.
    fn save_leftover(&self, id: &str) -> Option<LeftoverFields> {
        let cached = self
            .inner
            .caches
            .save_stamped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned();
        let stamped = match cached {
            Some(s) => s,
            None => {
                let gen = self.generations().saves;
                let s = self.paths().slot_stamped_fields(id).unwrap_or_default();
                self.with_gens(|g| {
                    if g.saves == gen {
                        self.inner
                            .caches
                            .save_stamped
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(id.to_string(), s.clone());
                    }
                });
                s
            }
        };
        if stamped.is_empty() {
            return None;
        }
        let left = leftovers_of(&stamped, &self.paths().slot_enabled_mods(id));
        (!left.is_empty()).then_some(left)
    }

    /// Takes `fields` out of slot `id`'s save (D86): values one removed mod alone left
    /// there. A backup is made first, and the save is written only when the result parses
    /// and the game's Lua loads it. Refused while the game uses the slot.
    pub fn save_cleanup(&self, id: &str, fields: &[String]) -> QmResult<SaveCleanup> {
        self.save_slot(id)?;
        self.ensure_not_running_for(id)?;
        let lua = self.lua_compiler().ok();
        let result = self
            .paths()
            .clean_save(id, fields, lua.as_deref())
            .map_err(|m| QmError::new(ErrorKind::InvalidInput, MSG_CLEANUP).with_details(m));
        self.invalidate(Scope::Saves);
        result
    }

    /// Rename a slot; names are unique (ignoring case). The Vanilla slot keeps its name.
    pub fn save_rename(&self, id: &str, name: &str) -> QmResult<SaveProfile> {
        self.save_slot(id)?;
        let result = self
            .paths()
            .rename_save_profile(id, name)
            .or_kind(ErrorKind::InvalidInput);
        self.invalidate(Scope::Saves);
        self.notify(Scope::Profiles);
        result
    }

    /// A new, empty slot: a fresh game start.
    pub fn save_create(&self, name: &str) -> QmResult<SaveProfile> {
        let result = self
            .paths()
            .create_save_profile(name)
            .or_kind(ErrorKind::InvalidInput);
        self.invalidate(Scope::Saves);
        result
    }

    /// Move a slot to `saves/.deleted/` (kept 30 days). Refused for the active and the Vanilla
    /// slot. Profiles that used it get "New slot on first launch".
    pub fn save_delete(&self, id: &str) -> QmResult<()> {
        let slot = self.save_slot(id)?;
        if slot.builtin {
            return Err(QmError::new(ErrorKind::InvalidInput, MSG_VANILLA_DELETE));
        }
        if self.paths().load_settings().active_save_profile_id == id {
            return Err(QmError::new(ErrorKind::Conflict, MSG_DELETE_ACTIVE));
        }
        let result = self
            .paths()
            .delete_save_profile(id)
            .map(|_| ())
            .or_kind(ErrorKind::Io);
        if result.is_ok() {
            for mut p in self.paths().list_profiles() {
                if p.linked_save_profile_id == id {
                    p.linked_save_profile_id = String::new();
                    if let Err(e) = self.save_profile(&p) {
                        tracing::warn!(
                            "could not unlink profile {} from deleted slot {id}: {e}",
                            p.id
                        );
                    }
                }
            }
        }
        self.invalidate(Scope::Saves);
        self.invalidate(Scope::Profiles);
        result
    }

    /// Back up, then clear the slot to a fresh new game. The one destructive action the
    /// Vanilla slot allows; refused while the game runs.
    pub fn save_reset(&self, id: &str) -> QmResult<()> {
        self.ensure_not_running()?;
        self.save_slot(id)?;
        let result = self
            .paths()
            .reset_save_profile(id)
            .map(|_| ())
            .or_kind(ErrorKind::Io);
        self.invalidate(Scope::Saves);
        result
    }

    /// Lock or unlock a slot as vanilla. The Vanilla slot is always locked.
    pub fn save_lock_set(&self, id: &str, locked: bool) -> QmResult<SaveProfile> {
        self.save_slot(id)?;
        let result = self
            .paths()
            .set_save_lock(id, locked)
            .or_kind(ErrorKind::InvalidInput);
        self.invalidate(Scope::Saves);
        result
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
        let active = self.paths().load_settings().active_profile_id;
        if (active == qm_manager::VANILLA_ID) != (id == qm_manager::VANILLA_ID) {
            return Err(crate::error::QmError::new(
                crate::error::ErrorKind::InvalidInput,
                if id == qm_manager::VANILLA_ID {
                    crate::ops::profiles::MSG_VANILLA_SLOT
                } else {
                    "The Vanilla profile always uses the Vanilla save slot."
                },
            ));
        }
        self.ensure_not_running()?;
        let target = self.save_slot(id)?;
        // A profile with mods cannot move onto a slot locked as vanilla (the launch would refuse it).
        if target.locked_vanilla
            && active != qm_manager::VANILLA_ID
            && self
                .active_profile()
                .is_ok_and(|p| !p.enabled_mods.is_empty())
        {
            return Err(QmError::new(ErrorKind::Conflict, MSG_SWITCH_LOCKED));
        }
        let result = self.tx_switch_save(id);
        self.invalidate(Scope::Saves);
        // The switch also relinks the active profile to the new slot.
        self.invalidate(Scope::Profiles);
        result
    }

    pub fn save_clone(&self, src_id: &str, new_id: &str, new_name: &str) -> QmResult<SaveProfile> {
        self.save_slot(src_id)?;
        self.ensure_not_running_for(src_id)?;
        let result = self
            .paths()
            .clone_save_profile(src_id, new_id, new_name)
            .map_err(clone_error);
        self.invalidate(Scope::Saves);
        result
    }

    pub fn save_backup(&self, id: &str, label: &str) -> QmResult<SaveBackupEntry> {
        self.save_slot(id)?;
        self.ensure_not_running_for(id)?;
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
