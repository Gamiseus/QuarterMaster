//! Test in game (Mod Engine, Stage C): a trial run of the active profile with only the mod
//! being made and the mods it requires. A trial changes the profile in memory only, backs the
//! save slot up first and stages the profile's own plan again afterwards (D68), so a test
//! leaves nothing behind.
use crate::error::{ErrorKind, QmError, QmResult};
use crate::game::GameStatus;
use crate::ops::launch::{LaunchRequest, TrialRequest};
use crate::service::Service;
use qm_manager::runs::TrialChanges;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A mod a test needs, with whether it can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineTestMod {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub enabled: bool,
}

/// Why a test cannot run yet, and what fixes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum EngineTestBlock {
    /// The mod folder has no id (no `mod.json` and no `[manifest] id`).
    NoId,
    /// The launcher's mod set has no mod with this id (the Library side without a copy).
    NotInSet,
    /// The active profile has the mod turned off.
    Off,
    /// A mod it requires is not installed.
    RequiredMissing,
    /// A mod it requires is turned off in the active profile.
    RequiredOff,
    /// The Vanilla profile runs no mods.
    Vanilla,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineTestPlan {
    pub mod_id: String,
    pub mod_name: String,
    pub in_set: bool,
    pub enabled: bool,
    /// The mods it requires, followed through their own requirements.
    pub requires: Vec<EngineTestMod>,
    /// Mods on in the active profile that the test turns off.
    pub turned_off: Vec<EngineTestMod>,
    pub blocked: Option<EngineTestBlock>,
    /// The reason in plain words, when blocked.
    pub message: Option<String>,
    /// The launcher is on the Workspace side.
    pub workspace_mode: bool,
}

impl Service {
    /// What a test of the workspace mod in `folder` would run, or why it cannot.
    pub fn engine_test_plan(&self, folder: &str) -> QmResult<EngineTestPlan> {
        let dir = self.engine_project_dir(folder)?;
        let workspace_mode = self.paths().workspace_mode;
        let own = qm_manager::library::inspect_mod_directory(&dir, false).ok();
        let (mod_id, mod_name) = match &own {
            Some(m) if !m.manifest.id.trim().is_empty() => {
                (m.manifest.id.clone(), m.manifest.name.clone())
            }
            _ => {
                return Ok(blocked_plan(
                    String::new(),
                    folder.to_string(),
                    workspace_mode,
                    EngineTestBlock::NoId,
                    "This mod has no id yet: give it a mod.json with an id first.".to_string(),
                ))
            }
        };
        let profile = self.active_profile()?;
        if profile.id == qm_manager::profiles::VANILLA_ID {
            return Ok(blocked_plan(
                mod_id,
                mod_name,
                workspace_mode,
                EngineTestBlock::Vanilla,
                "The Vanilla profile runs no mods. Pick another profile to test in.".to_string(),
            ));
        }
        let installed = self.installed_mods();
        let find = |id: &str| {
            installed
                .iter()
                .find(|m| m.manifest.id.eq_ignore_ascii_case(id))
        };
        let is_on = |id: &str| {
            profile
                .enabled_mods
                .iter()
                .any(|e| e.eq_ignore_ascii_case(id))
        };
        // The mods it requires, through their own requirements; the workspace copy's list
        // first (it may be newer than an installed copy).
        let mut seen: BTreeSet<String> = BTreeSet::new();
        seen.insert(mod_id.to_ascii_lowercase());
        let mut queue: Vec<String> = own
            .as_ref()
            .map(|m| m.manifest.relationships.requires.keys().cloned().collect())
            .unwrap_or_default();
        queue.sort();
        let mut requires = Vec::new();
        while let Some(id) = queue.pop() {
            if !seen.insert(id.to_ascii_lowercase()) {
                continue;
            }
            let m = find(&id);
            if let Some(m) = m {
                let mut next: Vec<String> =
                    m.manifest.relationships.requires.keys().cloned().collect();
                next.sort();
                queue.extend(next);
            }
            requires.push(EngineTestMod {
                name: m.map_or_else(|| id.clone(), |m| m.manifest.name.clone()),
                installed: m.is_some(),
                enabled: is_on(&id),
                id,
            });
        }
        requires.sort_by_key(|a| a.name.to_lowercase());
        let turned_off: Vec<EngineTestMod> = profile
            .enabled_mods
            .iter()
            .filter(|e| !seen.contains(&e.to_ascii_lowercase()))
            .map(|e| EngineTestMod {
                id: e.clone(),
                name: find(e).map_or_else(|| e.clone(), |m| m.manifest.name.clone()),
                installed: find(e).is_some(),
                enabled: true,
            })
            .collect();
        let in_set = find(&mod_id).is_some();
        let enabled = is_on(&mod_id);
        let (blocked, message) = if !in_set {
            (
                Some(EngineTestBlock::NotInSet),
                Some(if workspace_mode {
                    format!("The launcher has not read {mod_name} yet. Sync now reads the workspace folder again.")
                } else {
                    format!("The library has no copy of {mod_name}. Sync now copies the workspace into the library, or switch to the Workspace side to run the folder itself.")
                }),
            )
        } else if !enabled {
            (
                Some(EngineTestBlock::Off),
                Some(format!("{mod_name} is off in the active profile.")),
            )
        } else if let Some(m) = requires.iter().find(|r| !r.installed) {
            (
                Some(EngineTestBlock::RequiredMissing),
                Some(format!(
                    "{mod_name} requires {}, which is not installed.",
                    m.name
                )),
            )
        } else if let Some(m) = requires.iter().find(|r| !r.enabled) {
            (
                Some(EngineTestBlock::RequiredOff),
                Some(format!(
                    "{mod_name} requires {}, which is off in the active profile.",
                    m.name
                )),
            )
        } else {
            (None, None)
        };
        Ok(EngineTestPlan {
            mod_id,
            mod_name,
            in_set,
            enabled,
            requires,
            turned_off,
            blocked,
            message,
            workspace_mode,
        })
    }

    /// Runs the test: a trial of the active profile with every other mod turned off.
    pub fn engine_test_launch(&self, folder: &str) -> QmResult<GameStatus> {
        let plan = self.engine_test_plan(folder)?;
        if let Some(message) = plan.message.filter(|_| plan.blocked.is_some()) {
            return Err(QmError::new(ErrorKind::InvalidInput, message));
        }
        if plan.turned_off.is_empty() {
            // Nothing else is on: the profile's own launch is the test.
            let profile_id = self.paths().load_settings().active_profile_id;
            return self.game_launch(LaunchRequest {
                profile_id,
                method: None,
                without_mods: false,
            });
        }
        self.game_launch_trial(TrialRequest {
            changes: TrialChanges {
                disable_mods: plan.turned_off.iter().map(|m| m.id.clone()).collect(),
                ..TrialChanges::default()
            },
            label: format!("Test of {}", plan.mod_name),
            method: None,
            bisect_id: None,
            step: None,
        })
    }
}

fn blocked_plan(
    mod_id: String,
    mod_name: String,
    workspace_mode: bool,
    why: EngineTestBlock,
    message: String,
) -> EngineTestPlan {
    EngineTestPlan {
        mod_id,
        mod_name,
        in_set: false,
        enabled: false,
        requires: Vec::new(),
        turned_off: Vec::new(),
        blocked: Some(why),
        message: Some(message),
        workspace_mode,
    }
}
