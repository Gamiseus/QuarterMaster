//! Profile queries and actions.
use crate::dto::{
    LaunchMethodChoice, NewProfile, ProfileSettingsPatch, ProfileSummary, SwitchPreview,
};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::service::Service;
use qm_core::patch::plan::RawLoad;
use qm_manager::{
    decode_profile_code, ActivateError, ActivateExtras, Activation, LaunchMethod, ModProfile,
    KNOWN_NOTICES, VANILLA_ID,
};
use std::sync::Arc;

const MSG_DELETE_ACTIVE: &str = "Switch to another profile before deleting this one.";
pub(crate) const MSG_VANILLA_PROFILE: &str =
    "The Vanilla profile is built in and never loads mods.";
pub(crate) const MSG_VANILLA_SLOT: &str = "The Vanilla save slot belongs to the Vanilla profile.";
const MSG_VANILLA_CLONE: &str =
    "The Vanilla profile cannot be cloned. Create a new profile instead.";

pub(crate) fn vanilla_profile_error() -> QmError {
    QmError::new(ErrorKind::InvalidInput, MSG_VANILLA_PROFILE)
}

fn vanilla_slot_error() -> QmError {
    QmError::new(ErrorKind::InvalidInput, MSG_VANILLA_SLOT)
}

impl Service {
    pub fn profiles_list(&self) -> Vec<ProfileSummary> {
        let active = self.paths().load_settings().active_profile_id;
        let (profiles, notices) = self.paths().list_profiles_reporting();
        self.report_profile_notices(notices);
        profiles
            .iter()
            .map(|p| ProfileSummary::new(p, &active))
            .collect()
    }

    /// Unreadable profile files that were set aside, shown to the user.
    pub(crate) fn report_profile_notices(&self, notices: Vec<String>) {
        for summary in notices {
            tracing::warn!("{summary}");
            self.inner
                .sink
                .emit(crate::events::QmEvent::Recovered { summary });
        }
    }

    /// Stages `id` and makes it active, passing activation notices to the user. A staged
    /// plan that does not read back cleanly is `invalid_input` (§7.3).
    pub(crate) fn stage_profile(&self, id: &str) -> QmResult<()> {
        // The cached vanilla files let a first migration compare the patched text (§7.4).
        let vanilla = self.vanilla_files();
        let extras = ActivateExtras {
            vanilla: vanilla.as_ref().map(|v| &v.files),
            trial: None,
        };
        self.report_activation(self.paths().activate_profile_with(id, extras))
    }

    /// `stage_profile` with a trial's changes on a copy of the profile (D68). The profile file
    /// is not written.
    pub(crate) fn stage_profile_trial(
        &self,
        id: &str,
        changes: &qm_manager::runs::TrialChanges,
    ) -> QmResult<()> {
        let extras = ActivateExtras {
            vanilla: None,
            trial: Some(changes),
        };
        self.report_activation(self.paths().activate_profile_with(id, extras))
    }

    /// `stage_profile` with no mods and a no-mods plan, for "Launch without mods" (D16).
    pub(crate) fn stage_profile_without_mods(&self, id: &str) -> QmResult<()> {
        self.report_activation(self.paths().activate_profile_without_mods(id))
    }

    /// Shows the notices that reads without a reporter (such as `get_profile`) left pending.
    pub(crate) fn drain_profile_notices(&self) {
        self.report_profile_notices(self.paths().take_profile_notices());
    }

    fn report_activation(&self, activation: Result<Activation, ActivateError>) -> QmResult<()> {
        self.drain_profile_notices();
        match activation {
            Ok(a) => {
                for summary in a.warnings {
                    tracing::warn!("{summary}");
                    self.inner
                        .sink
                        .emit(crate::events::QmEvent::Recovered { summary });
                }
                Ok(())
            }
            Err(ActivateError::InvalidPlan(m)) => Err(QmError::new(ErrorKind::InvalidInput, m)),
            Err(ActivateError::Failed(m)) => Err(QmError::new(ErrorKind::Io, m)),
        }
    }

    pub fn profile_get(&self, id: &str) -> QmResult<ModProfile> {
        let p = self.paths().get_profile(id).or_kind(ErrorKind::NotFound);
        self.drain_profile_notices();
        p
    }

    /// Mods only in A or B, priority and setting differences between two profiles.
    pub fn profile_diff(&self, a: &str, b: &str) -> QmResult<qm_manager::ProfileDiff> {
        self.profile_get(a)?;
        self.profile_get(b)?;
        self.paths()
            .diff_profiles(a, b)
            .or_kind(ErrorKind::NotFound)
    }

    pub fn profile_preview_switch(&self, id: &str) -> QmResult<SwitchPreview> {
        let target = self.profile_get(id)?;
        let active = self.paths().load_settings().active_profile_id;
        let diff = self
            .paths()
            .diff_profiles(&active, &target.id)
            .or_kind(ErrorKind::NotFound)?;
        Ok(SwitchPreview {
            added_mods: diff.only_in_b.clone(),
            removed_mods: diff.only_in_a.clone(),
            save_compatibility: self.paths().check_save_compatibility(&target),
            diff,
        })
    }

    pub fn profile_create(&self, req: NewProfile) -> QmResult<ModProfile> {
        if req.name.trim().is_empty() {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "Profile name cannot be empty.",
            ));
        }
        if req.clone_from.as_deref() == Some(VANILLA_ID) {
            return Err(QmError::new(ErrorKind::InvalidInput, MSG_VANILLA_CLONE));
        }
        if req.linked_save_profile_id.as_deref() == Some(VANILLA_ID) {
            return Err(vanilla_slot_error());
        }
        if let Some(src) = req.clone_from.as_deref() {
            self.profile_get(src)?;
        }
        let p = self
            .paths()
            .create_profile_with_id(
                req.id.as_deref(),
                &req.name,
                req.clone_from.as_deref(),
                req.linked_save_profile_id.as_deref(),
                req.description.as_deref(),
            )
            .map_err(create_error)?;
        self.notify(Scope::Profiles);
        Ok(p)
    }

    pub fn profile_rename(&self, id: &str, name: &str) -> QmResult<ModProfile> {
        if id == VANILLA_ID {
            return Err(vanilla_profile_error());
        }
        self.profile_get(id)?;
        let active = self.is_active_profile(id);
        if active {
            self.ensure_not_running()?;
        }
        let p = self
            .paths()
            .rename_profile(id, name)
            .or_kind(ErrorKind::InvalidInput)?;
        if active {
            // The staged launch config carries the profile name.
            self.stage_profile(&p.id)?;
            self.after_active_profile_change();
        } else {
            self.notify(Scope::Profiles);
        }
        Ok(p)
    }

    /// Edit launch settings and the linked save slot. Active profile edits are refused while the
    /// game runs; inactive ones are always allowed.
    pub fn profile_update(&self, id: &str, patch: ProfileSettingsPatch) -> QmResult<ModProfile> {
        if id == VANILLA_ID {
            return Err(vanilla_profile_error());
        }
        if patch.linked_save_profile_id.as_deref() == Some(VANILLA_ID) {
            return Err(vanilla_slot_error());
        }
        let mut p = self.profile_get(id)?;
        if let Some(save_id) = patch
            .linked_save_profile_id
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            if !self
                .paths()
                .list_save_profiles()
                .iter()
                .any(|s| s.id == save_id)
            {
                return Err(QmError::new(
                    ErrorKind::NotFound,
                    format!("Save slot '{save_id}' does not exist."),
                ));
            }
        }
        let save_changed = patch
            .linked_save_profile_id
            .as_deref()
            .is_some_and(|s| s != p.linked_save_profile_id);
        if let Some(choice) = patch.launch_method {
            p.launch_method = match choice {
                LaunchMethodChoice::Default => None,
                LaunchMethodChoice::SteamUrl => Some(LaunchMethod::SteamUrl),
                LaunchMethodChoice::DirectExe => Some(LaunchMethod::DirectExe),
            };
        }
        if let Some(args) = patch.custom_launch_args {
            p.custom_launch_args = args;
        }
        if let Some(save_id) = patch.linked_save_profile_id {
            p.linked_save_profile_id = save_id;
        }
        if self.is_active_profile(id) {
            let p = self.commit_active_profile(p)?;
            if save_changed {
                self.invalidate(Scope::Saves);
            }
            Ok(p)
        } else {
            p.updated_at = jiff::Zoned::now().to_string();
            self.save_profile(&p).or_kind(ErrorKind::Io)?;
            self.notify(Scope::Profiles);
            Ok(p)
        }
    }

    pub fn profile_delete(&self, id: &str) -> QmResult<()> {
        if id == VANILLA_ID {
            return Err(vanilla_profile_error());
        }
        self.profile_get(id)?;
        if self.is_active_profile(id) {
            return Err(QmError::new(ErrorKind::Conflict, MSG_DELETE_ACTIVE));
        }
        let active = self.paths().load_settings().active_profile_id;
        self.paths()
            .delete_profile(id, &active)
            .or_kind(ErrorKind::InvalidInput)?;
        self.notify(Scope::Profiles);
        Ok(())
    }

    pub fn profile_clone(&self, src_id: &str, new_name: &str) -> QmResult<ModProfile> {
        let src = self.profile_get(src_id)?;
        self.profile_create(NewProfile {
            name: new_name.to_string(),
            id: None,
            clone_from: Some(src.id),
            linked_save_profile_id: Some(src.linked_save_profile_id),
            description: None,
        })
    }

    /// Stage the profile, swap saves to its linked save profile and make it active. A profile
    /// set to "New slot on first launch" gets its slot now, and the notice says so.
    pub fn profile_activate(&self, id: &str) -> QmResult<ModProfile> {
        self.ensure_not_running()?;
        self.profile_get(id)?;
        if let Some(name) = self.paths().ensure_linked_slot(id).or_kind(ErrorKind::Io)? {
            self.inner.sink.emit(crate::events::QmEvent::Recovered {
                summary: format!("Created the save slot \"{name}\" for this profile."),
            });
        }
        let p = self.profile_get(id)?;
        self.stage_profile(&p.id)?;
        self.invalidate(Scope::Saves);
        self.after_active_profile_change();
        Ok(p)
    }

    pub fn profile_export_code(&self, id: &str) -> QmResult<String> {
        let p = self.profile_get(id)?;
        self.paths()
            .export_profile_code(&p.id)
            .or_kind(ErrorKind::Internal)
    }

    /// Create a new, inactive profile from a `QM1-` code.
    pub fn profile_import_code(&self, code: &str, new_id: &str) -> QmResult<ModProfile> {
        let pack = decode_profile_code(code).or_kind(ErrorKind::InvalidInput)?;
        let valid = !new_id.is_empty()
            && new_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "Profile ID may only contain letters, digits, '_' and '-'.",
            ));
        }
        if self.paths().get_profile(new_id).is_ok() {
            return Err(QmError::new(
                ErrorKind::Conflict,
                format!("A profile with ID '{new_id}' already exists."),
            ));
        }
        let p = self
            .paths()
            .apply_modpack_as_profile(&pack, new_id)
            .or_kind(ErrorKind::Io)?;
        self.notify(Scope::Profiles);
        Ok(p)
    }

    /// Marks a one-time notice seen in the active profile (`stacking_v1`, `fixes_import_v1`).
    pub fn notice_dismiss(&self, key: &str) -> QmResult<()> {
        if !KNOWN_NOTICES.contains(&key) {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                format!("There is no notice called '{key}'."),
            ));
        }
        self.ensure_not_running()?;
        let mut p = self.active_profile()?;
        // The Vanilla profile has nothing to tell.
        if p.id == VANILLA_ID || !p.mark_notice_seen(key) {
            return Ok(());
        }
        self.commit_active_profile(p).map(|_| ())
    }

    pub(crate) fn is_active_profile(&self, id: &str) -> bool {
        self.paths().load_settings().active_profile_id == id
    }

    pub(crate) fn active_profile(&self) -> QmResult<ModProfile> {
        let id = self.paths().load_settings().active_profile_id;
        self.profile_get(&id)
    }

    /// Persist an edited active profile: refused while running; save, restage, invalidate.
    pub(crate) fn commit_active_profile(&self, mut profile: ModProfile) -> QmResult<ModProfile> {
        // Every mod, order and patch edit lands here: the Vanilla profile never changes.
        if profile.id == VANILLA_ID {
            return Err(vanilla_profile_error());
        }
        self.ensure_not_running()?;
        profile.updated_at = jiff::Zoned::now().to_string();
        // Fixes made or accepted in this write are stamped with the game files now (D74).
        self.stamp_fresh_fixes(&mut profile);
        // A zip's stem id moves to its mod.json id on the next save (D62).
        let raw = self.zip_raw();
        let renamed = self
            .with_profile_write(|| {
                self.paths()
                    .save_profile_renaming(&mut profile, raw.as_deref())
            })
            .or_kind(ErrorKind::Io)?;
        for z in renamed {
            let summary = format!(
                "Zip mod \"{}\" is now \"{}\", the id in its mod.json. This profile uses the new id.",
                z.stem, z.id
            );
            tracing::info!("{summary}");
            self.inner
                .sink
                .emit(crate::events::QmEvent::Recovered { summary });
        }
        self.stage_profile(&profile.id)?;
        self.after_active_profile_change();
        Ok(profile)
    }

    /// The load to rename zip stem ids with (D62): the cached one, else the staged one when
    /// the staged Mods folder holds a zip.
    fn zip_raw(&self) -> Option<Arc<RawLoad>> {
        if let Some(raw) = self.inner.caches.raw_load.get() {
            return Some(raw);
        }
        let has_zip = std::fs::read_dir(self.staged_mods_dir()).is_ok_and(|rd| {
            rd.filter_map(Result::ok).any(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("zip"))
            })
        });
        if has_zip {
            self.current_raw()
        } else {
            None
        }
    }

    /// The active profile's contents changed: simulation is stale and mod rows' `enabled` flags
    /// may differ (the mods cache itself stays valid).
    pub(crate) fn after_active_profile_change(&self) {
        self.invalidate(Scope::Profiles);
        self.notify(Scope::Mods);
    }
}

/// Map `create_profile_with_id` errors; the manager reports duplicates only by message.
fn create_error(msg: String) -> QmError {
    let kind = if msg.contains("already exists") {
        ErrorKind::Conflict
    } else {
        ErrorKind::InvalidInput
    };
    QmError::new(kind, msg)
}
