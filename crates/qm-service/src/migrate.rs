//! One-time launch settings migration (plan 2c Task 6): one launch model with a built-in
//! Vanilla profile, Steam or Direct as the method, and one global console setting.
//!
//! It is idempotent rather than journaled: every step looks at the current files, so a run that
//! stops part way simply finishes on the next start. The version marker is written last.
use crate::error::{ErrorKind, OrKind, QmResult};
use crate::service::Service;
use qm_manager::profiles::VANILLA_ID;
use qm_manager::LaunchMethod;

pub(crate) const LAUNCH_MODEL_VERSION: u32 = 1;

const MSG_CONSOLE: &str =
    "The console is now one setting for all profiles. It is shown; turn it off in Settings.";

impl Service {
    /// Runs the migration when it has not run yet; returns a notice when anything changed.
    /// Only call while the game is not running (it may switch the active profile).
    pub(crate) fn migrate_launch_model(&self) -> QmResult<Option<String>> {
        let paths = self.paths();
        let mut settings = paths.load_settings();
        if settings.launch_model_version >= LAUNCH_MODEL_VERSION {
            return Ok(None);
        }
        let mut updated = 0usize;
        let mut kept_modded: Vec<String> = Vec::new();
        let mut console_note = false;

        for mut p in paths.list_profiles() {
            if p.id == VANILLA_ID {
                continue;
            }
            let vanilla_method = p.launch_method == Some(LaunchMethod::Vanilla);
            if vanilla_method && p.enabled_mods.is_empty() {
                // A profile that only meant "play vanilla": the built-in Vanilla profile does that now.
                let active = paths.load_settings().active_profile_id;
                if active == p.id {
                    paths.activate_profile(VANILLA_ID).or_kind(ErrorKind::Io)?;
                }
                let active = paths.load_settings().active_profile_id;
                paths
                    .delete_profile(&p.id, &active)
                    .or_kind(ErrorKind::Io)?;
                updated += 1;
                continue;
            }
            let mut changed = false;
            if vanilla_method {
                p.launch_method = None;
                kept_modded.push(p.name.clone());
                changed = true;
            }
            if p.disable_console {
                if !settings.disable_console_on_launch {
                    console_note = true;
                }
                p.disable_console = false;
                changed = true;
            }
            if p.linked_save_profile_id == VANILLA_ID {
                // The Vanilla slot belongs to the Vanilla profile; copy its save so no progress is lost.
                let slot_id = unique_slot_id(self, &p.id);
                let mut slot = paths
                    .clone_save_profile(VANILLA_ID, &slot_id, &p.name)
                    .or_kind(ErrorKind::Io)?;
                slot.is_vanilla_clean = false;
                paths.save_save_profile(&slot).or_kind(ErrorKind::Io)?;
                p.linked_save_profile_id = slot.id;
                changed = true;
            }
            if changed {
                p.updated_at = jiff::Zoned::now().to_string();
                paths.save_profile(&p).or_kind(ErrorKind::Io)?;
                if paths.load_settings().active_profile_id == p.id {
                    // Restage so the launch config and linked slot match the edited profile.
                    paths.activate_profile(&p.id).or_kind(ErrorKind::Io)?;
                }
                updated += 1;
            }
        }

        settings = paths.load_settings();
        if settings.default_launch_method == LaunchMethod::Vanilla {
            settings.default_launch_method = LaunchMethod::DirectExe;
        }
        settings.launch_model_version = LAUNCH_MODEL_VERSION;
        paths.save_settings(&settings).or_kind(ErrorKind::Io)?;
        paths.repair_vanilla_builtins().or_kind(ErrorKind::Io)?;

        if updated == 0 {
            return Ok(None);
        }
        let mut notice = format!(
            "Launch settings were simplified: {updated} {} updated.",
            if updated == 1 { "profile" } else { "profiles" }
        );
        if !kept_modded.is_empty() {
            notice.push_str(&format!(
                " {} now {} the default launch method; play vanilla with the Vanilla profile.",
                kept_modded.join(", "),
                if kept_modded.len() == 1 {
                    "uses"
                } else {
                    "use"
                }
            ));
        }
        if console_note {
            notice.push(' ');
            notice.push_str(MSG_CONSOLE);
        }
        Ok(Some(notice))
    }
}

/// `<profile id>` (or `<profile id>-2`, ...) that no save slot uses yet.
fn unique_slot_id(svc: &Service, base: &str) -> String {
    let taken: Vec<String> = svc
        .paths()
        .list_save_profiles()
        .into_iter()
        .map(|s| s.id)
        .collect();
    let base = if base == VANILLA_ID { "profile" } else { base };
    let mut id = base.to_string();
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}
