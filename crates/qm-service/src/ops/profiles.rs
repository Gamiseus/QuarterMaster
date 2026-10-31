//! Profile queries and actions.
use crate::dto::{NewProfile, ProfileSummary, SwitchPreview};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::service::Service;
use qm_manager::{decode_profile_code, ModProfile};

const MSG_DELETE_ACTIVE: &str = "Switch to another profile before deleting this one.";

impl Service {
    pub fn profiles_list(&self) -> Vec<ProfileSummary> {
        let active = self.paths().load_settings().active_profile_id;
        self.paths()
            .list_profiles()
            .iter()
            .map(|p| ProfileSummary::new(p, &active))
            .collect()
    }

    pub fn profile_get(&self, id: &str) -> QmResult<ModProfile> {
        self.paths().get_profile(id).or_kind(ErrorKind::NotFound)
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
            self.paths().activate_profile(&p.id).or_kind(ErrorKind::Io)?;
            self.after_active_profile_change();
        } else {
            self.notify(Scope::Profiles);
        }
        Ok(p)
    }

    pub fn profile_delete(&self, id: &str) -> QmResult<()> {
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

    /// Stage the profile, swap saves to its linked save profile and make it active.
    pub fn profile_activate(&self, id: &str) -> QmResult<ModProfile> {
        self.ensure_not_running()?;
        let p = self.profile_get(id)?;
        self.paths().activate_profile(&p.id).or_kind(ErrorKind::Io)?;
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

    pub(crate) fn is_active_profile(&self, id: &str) -> bool {
        self.paths().load_settings().active_profile_id == id
    }

    pub(crate) fn active_profile(&self) -> QmResult<ModProfile> {
        let id = self.paths().load_settings().active_profile_id;
        self.profile_get(&id)
    }

    /// Persist an edited active profile: refused while running; save, restage, invalidate.
    pub(crate) fn commit_active_profile(&self, mut profile: ModProfile) -> QmResult<ModProfile> {
        self.ensure_not_running()?;
        profile.updated_at = jiff::Zoned::now().to_string();
        self.paths().save_profile(&profile).or_kind(ErrorKind::Io)?;
        self.paths()
            .activate_profile(&profile.id)
            .or_kind(ErrorKind::Io)?;
        self.after_active_profile_change();
        Ok(profile)
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
