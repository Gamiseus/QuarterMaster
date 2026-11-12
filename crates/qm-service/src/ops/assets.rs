//! Asset overlaps and faction picks (Phase E Tasks 8 and 9): the asset ledger for the active
//! profile, what a choice would change, and the faction picked for a side.

use std::collections::BTreeMap;

use qm_core::handshake::{LoadOrderPlan, PatchReport};
use qm_core::lenient::Lenient;
use qm_core::patch::faction::{self, LoadedFaction};
use qm_core::patch::plan::RawLoad;
use qm_manager::assets_ledger::{self, AssetLedger, LedgerInput, VANILLA};
use qm_manager::{fixes, ModProfile};
use serde::{Deserialize, Serialize};

use crate::cache::Loadable;
use crate::error::{ErrorKind, QmError, QmResult};
use crate::service::Service;

/// One slot whose winner a choice changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AssetSlotChange {
    pub slot: String,
    pub name: String,
    /// A source, or `vanilla`.
    pub from: String,
    pub to: String,
}

/// What giving `key` to `winner` would do. Nothing is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AssetChoicePreview {
    pub key: String,
    pub winner: String,
    /// The slots that get another winner, in ledger order.
    pub changes: Vec<AssetSlotChange>,
    /// Slots of the group the winner has no file for; they keep their current winner.
    pub unfilled: Vec<String>,
    /// Source -> the files (or, for a faction, the game names) the choice leaves out.
    pub leaves_out: BTreeMap<String, Vec<String>>,
}

/// One enabled faction that replaces a side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FactionCandidate {
    pub faction_id: String,
    pub name: String,
    pub mod_id: String,
    pub priority: i32,
}

/// A side two or more enabled factions replace, and which one runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FactionSideChoice {
    /// `allied` or `axis`.
    pub side: String,
    /// "Allies" or "Germans".
    pub label: String,
    /// The faction the profile picked, when it picked one.
    pub picked: Option<String>,
    /// The faction that runs now: the pick when it is loaded, else the highest priority.
    pub active: Option<String>,
    /// Highest priority first.
    pub candidates: Vec<FactionCandidate>,
}

/// A plan that holds only a profile's faction picks, for `pick_active_for`.
pub(crate) fn pick_plan(picks: &BTreeMap<String, String>) -> LoadOrderPlan {
    LoadOrderPlan {
        factions: picks
            .iter()
            .map(|(side, id)| (side.clone(), Lenient::Ok(id.clone())))
            .collect(),
        ..LoadOrderPlan::default()
    }
}

/// The factions that run for `picks`, one per side.
pub(crate) fn active_factions(
    candidates: Vec<LoadedFaction>,
    picks: &BTreeMap<String, String>,
) -> Vec<LoadedFaction> {
    faction::pick_active_for(candidates, Some(&pick_plan(picks))).active
}

fn not_ready() -> QmError {
    QmError::new(
        ErrorKind::Conflict,
        "The patch simulation is still running. Try again in a moment.",
    )
}

impl Service {
    /// The ledger for profile `p` over the cached mods, with `choices` as its asset choices.
    /// None while the mods or the simulation are not loaded.
    fn ledger_with(
        &self,
        p: &ModProfile,
        raw: &RawLoad,
        choices: &BTreeMap<String, String>,
    ) -> Option<AssetLedger> {
        let report = self.inner.caches.simulation.get()?;
        let listing = self
            .inner
            .caches
            .vanilla
            .get()
            .and_then(|v| v.listing.clone());
        let main = report.traces.get("main.lua").map(|t| t.text.as_str());
        let manifests =
            assets_ledger::all_manifests_in_run_order(raw.patches.iter().map(|rp| &rp.patch), main);
        let active = active_factions(raw.factions.clone(), &p.faction_picks);
        let last_run = self.last_run_asset_winners(&p.id);
        Some(assets_ledger::build(&LedgerInput {
            manifests: &manifests,
            factions: &active,
            listing: listing.as_deref(),
            choices,
            last_run: &last_run,
        }))
    }

    /// Slot -> source from the report of the profile's last run, when the game wrote one for
    /// this profile.
    fn last_run_asset_winners(&self, profile_id: &str) -> BTreeMap<String, String> {
        let mods = self.paths().profiles_dir.join(profile_id).join("Mods");
        match PatchReport::load_from_mod_dir(&mods) {
            Ok(Some(r)) if r.profile_id.as_deref().is_none_or(|id| id == profile_id) => r
                .asset_winners
                .into_iter()
                .map(|w| (w.slot, w.source))
                .collect(),
            _ => BTreeMap::new(),
        }
    }

    /// The asset ledger of the active profile: every sound, image or font two sources
    /// claim, grouped, with today's winner and the profile's choices.
    pub fn assets_ledger(&self) -> QmResult<Loadable<AssetLedger>> {
        let p = self.active_profile()?;
        let Some(raw) = self.current_raw() else {
            return Ok(Loadable::Loading);
        };
        Ok(
            match self.ledger_with(&p, &raw, &fixes::asset_choices(&p)) {
                Some(l) => Loadable::Ready(l),
                None => Loadable::Loading,
            },
        )
    }

    /// The ledger with `key` given to `winner`, and what that choice leaves out. Errors when
    /// `key` names no group or slot, or `winner` is not the game or one of its sources.
    pub(crate) fn ledger_for_choice(
        &self,
        p: &ModProfile,
        raw: &RawLoad,
        key: &str,
        winner: &str,
    ) -> QmResult<(AssetLedger, AssetLedger, BTreeMap<String, Vec<String>>)> {
        let current = fixes::asset_choices(p);
        let before = self.ledger_with(p, raw, &current).ok_or_else(not_ready)?;
        let sources: Vec<&str> = match (before.group(key), before.slot(key)) {
            (Some(g), _) => g.claimants.iter().map(|c| c.source.as_str()).collect(),
            (None, Some(s)) => s.claims.iter().map(|c| c.source.as_str()).collect(),
            (None, None) => {
                return Err(QmError::new(
                    ErrorKind::NotFound,
                    "No enabled mod or faction claims that file or group any more.",
                ))
            }
        };
        if winner != VANILLA && !sources.contains(&winner) {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                format!("{winner} has no file there."),
            ));
        }
        let mut next = current;
        next.insert(key.to_string(), winner.to_string());
        let after = self.ledger_with(p, raw, &next).ok_or_else(not_ready)?;
        let leaves_out = assets_ledger::exclusions_of(&after, key);
        Ok((before, after, leaves_out))
    }

    /// What giving the group or slot `key` to `winner` would change. Nothing is stored.
    pub fn asset_choice_preview(&self, key: &str, winner: &str) -> QmResult<AssetChoicePreview> {
        let p = self.active_profile()?;
        let raw = self.current_raw().ok_or_else(not_ready)?;
        let (before, after, leaves_out) = self.ledger_for_choice(&p, &raw, key, winner)?;
        let mut changes = Vec::new();
        let mut unfilled = Vec::new();
        for s in after.groups.iter().flat_map(|g| &g.slots) {
            let was = before
                .slot(&s.key)
                .map(|b| b.winner.as_str())
                .unwrap_or(VANILLA);
            if was != s.winner {
                changes.push(AssetSlotChange {
                    slot: s.key.clone(),
                    name: s.name.clone(),
                    from: was.to_string(),
                    to: s.winner.clone(),
                });
            }
            let in_scope = s.key == key
                || (s.choice.is_none()
                    && after
                        .group(key)
                        .is_some_and(|g| g.slots.iter().any(|x| x.key == s.key)));
            if in_scope && winner != VANILLA && s.winner != winner {
                unfilled.push(s.name.clone());
            }
        }
        Ok(AssetChoicePreview {
            key: key.to_string(),
            winner: winner.to_string(),
            changes,
            unfilled,
            leaves_out,
        })
    }

    /// Takes back the running choice for the group or slot `key`: the source loaded last
    /// wins again (or the choice this one replaced comes back, as with any undo).
    pub fn asset_choice_clear(&self, key: &str) -> QmResult<()> {
        let p = self.active_profile()?;
        let id = fixes::fixes(&p)
            .filter(|f| !f.paused)
            .find(|f| matches!(&f.change, fixes::FixChange::AssetChoice { key: k, .. } if k == key))
            .map(|f| f.id.clone())
            .ok_or_else(|| {
                QmError::new(ErrorKind::NotFound, "There is no choice for that any more.")
            })?;
        self.fix_undo(&id, true)
    }

    /// Brings the stored `excludes` of the active profile's asset choices in line with the
    /// current ledger (a mod added or renamed files), after the simulation of generation
    /// `gen` was stored. It commits only when a list changed, and only while no other profile
    /// write happened since the pass began (`writes`): then the next simulation tries again.
    pub(crate) fn refresh_asset_choices(&self, gen: u32, writes: u64) {
        let Ok(p) = self.active_profile() else {
            return;
        };
        let choices = fixes::asset_choices(&p);
        if choices.is_empty() || self.game_status().is_busy() {
            return;
        }
        let Some(raw) = self.current_raw() else {
            return;
        };
        let Some(ledger) = self.ledger_with(&p, &raw, &choices) else {
            return;
        };
        let by_key = assets_ledger::exclusions_by_key(&ledger);
        let mut fresh = p;
        if fixes::refresh_asset_excludes(&mut fresh, &by_key) == 0 {
            return;
        }
        if self.profile_writes() != writes || self.generations().sim != gen {
            return;
        }
        if let Err(e) = self.commit_active_profile(fresh) {
            tracing::warn!("could not update the asset choices: {}", e.message);
        }
    }

    /// The sides two or more enabled factions replace, with the profile's pick. Empty when no
    /// side needs a choice.
    pub fn faction_choices(&self) -> QmResult<Loadable<Vec<FactionSideChoice>>> {
        let p = self.active_profile()?;
        let Some(raw) = self.current_raw() else {
            return Ok(Loadable::Loading);
        };
        let active = active_factions(raw.factions.clone(), &p.faction_picks);
        let mut out = Vec::new();
        for side in [faction_side("allied"), faction_side("axis")]
            .into_iter()
            .flatten()
        {
            let mut candidates: Vec<FactionCandidate> = raw
                .factions
                .iter()
                .filter(|lf| lf.faction.replaces == side)
                .map(|lf| FactionCandidate {
                    faction_id: lf.faction.id.clone(),
                    name: lf.faction.name.clone(),
                    mod_id: lf.mod_id.clone(),
                    priority: lf.priority,
                })
                .collect();
            if candidates.len() < 2 {
                continue;
            }
            candidates.sort_by(|a, b| (b.priority, &b.mod_id).cmp(&(a.priority, &a.mod_id)));
            out.push(FactionSideChoice {
                side: side.as_str().to_string(),
                label: side.label().to_string(),
                picked: p.faction_picks.get(side.as_str()).cloned(),
                active: active
                    .iter()
                    .find(|lf| lf.faction.replaces == side)
                    .map(|lf| lf.faction.id.clone()),
                candidates,
            });
        }
        Ok(Loadable::Ready(out))
    }

    /// Picks the faction for `side` (`allied` or `axis`), or with `None` goes back to the mod
    /// priority. The faction must be one an enabled mod has for that side.
    pub fn faction_pick_set(&self, side: &str, faction_id: Option<String>) -> QmResult<()> {
        self.ensure_not_running()?;
        let Some(want) = faction_side(side) else {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "Pick the Allies or the Germans.",
            ));
        };
        let mut p = self.active_profile()?;
        match faction_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        {
            None => {
                p.faction_picks.remove(want.as_str());
            }
            Some(id) => {
                let raw = self.current_raw().ok_or_else(not_ready)?;
                let known = raw
                    .factions
                    .iter()
                    .any(|lf| lf.faction.replaces == want && lf.faction.id == id);
                if !known {
                    return Err(QmError::new(
                        ErrorKind::NotFound,
                        format!(
                            "No enabled mod has the faction '{id}' for the {}.",
                            want.label()
                        ),
                    ));
                }
                p.faction_picks.insert(want.as_str().to_string(), id);
            }
        }
        self.commit_active_profile(p).map(|_| ())
    }
}

fn faction_side(side: &str) -> Option<qm_schema::faction::Side> {
    use qm_schema::faction::Side;
    [Side::Allied, Side::Axis]
        .into_iter()
        .find(|s| s.as_str() == side)
}
