//! Cache primitives and the cached service state.
use crate::dto::{CatalogResult, HealthReport, ModDetail, UpdatesReport};
use qm_manager::{InstalledMod, SimulationReport, VanillaGameInfo};
use qm_schema::SaveSummary;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum Loadable<T> {
    Loading,
    Ready(T),
}

/// A single clearable value behind a mutex.
pub struct Cached<T: Clone> {
    slot: Mutex<Option<T>>,
}

impl<T: Clone> Default for Cached<T> {
    fn default() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }
}

impl<T: Clone> Cached<T> {
    pub fn get(&self) -> Option<T> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set(&self, value: T) {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
    }

    pub fn clear(&self) {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[derive(Debug, Clone)]
pub struct VanillaFiles {
    pub game_version: String,
    pub info: Option<VanillaGameInfo>,
    pub files: HashMap<String, String>,
    /// What the game loads, for marking a mod's assets as overrides; `None` without game files.
    pub assets: Option<Arc<qm_core::patch::loader::VanillaAssetIndex>>,
    /// Every asset file in the game archive (D79); `None` without one, and then `assets`
    /// knows only what the Lua sources name.
    pub listing: Option<Arc<qm_manager::vanilla_assets::VanillaAssetListing>>,
}

/// The SHA-256 of the cached vanilla files, made on first use for the files they came from
/// (D74).
#[derive(Default)]
pub(crate) struct VanillaHashes {
    #[allow(clippy::type_complexity)]
    slot: Mutex<Option<(Arc<VanillaFiles>, HashMap<String, String>)>>,
}

impl VanillaHashes {
    /// The hash of game file `file` in `vanilla`; None when it has no such file.
    pub(crate) fn get(&self, vanilla: &Arc<VanillaFiles>, file: &str) -> Option<String> {
        let text = vanilla.files.get(file)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(&*slot, Some((v, _)) if Arc::ptr_eq(v, vanilla)) {
            *slot = Some((vanilla.clone(), HashMap::new()));
        }
        let (_, map) = slot.as_mut()?;
        Some(
            map.entry(file.to_string())
                .or_insert_with(|| qm_manager::fixes::game_stamp::sha256_hex(text))
                .clone(),
        )
    }
}

/// What the `FixSuggestions` job made for one simulation and profile (D70).
#[derive(Debug, Clone)]
pub(crate) enum SuggestionsState {
    Ready(Vec<crate::dto::FixSuggestion>),
    /// No suggestions could be made; the view shows why.
    Unavailable(String),
}

/// The bindings the game would report and the order results, with the table they come from.
pub(crate) type PlanView = (
    qm_core::patch::plan::PlanReport,
    qm_core::patch::table::PatchTable,
);

// Fields are crate-private so only `Service::invalidate` mutates them; see `testing` helpers.
#[derive(Default)]
pub struct Caches {
    pub(crate) mods: Cached<Arc<Vec<InstalledMod>>>,
    pub(crate) vanilla: Cached<Arc<VanillaFiles>>,
    /// The game files' hashes, for the game-update check of fixes (D74).
    pub(crate) vanilla_hashes: VanillaHashes,
    pub(crate) simulation: Cached<Arc<SimulationReport>>,
    /// The patch table the cached simulation ran; set and cleared with it.
    pub(crate) patch_table: Cached<Arc<qm_core::patch::table::PatchTable>>,
    /// What that simulation read from the staged Mods folder, for previews that lay another
    /// plan over the same mods; set and cleared with it.
    pub(crate) raw_load: Cached<Arc<qm_core::patch::plan::RawLoad>>,
    /// The Fixes view's suggestions: the simulation generation and profile they were made
    /// for, and the result; cleared with the simulation.
    pub(crate) suggestions: Cached<(u32, u64, Arc<SuggestionsState>)>,
    /// The Fixes view's plan view (bindings and order results, and the table) with the
    /// staged plan's note it was built with, keyed like the suggestions; cleared with them.
    #[allow(clippy::type_complexity)]
    pub(crate) plan_view: Cached<(u32, u64, Option<Arc<PlanView>>, Option<String>)>,
    /// How many plan views were built (tests check the cache with it).
    pub(crate) plan_views_built: AtomicU32,
    /// The line rules comparison (D51) of one cached simulation: the report it was made with,
    /// and the result (`None`: not run for this profile). Read only while that report is
    /// still the cached one.
    #[allow(clippy::type_complexity)]
    pub(crate) line_rules: Cached<(
        Arc<SimulationReport>,
        Option<Arc<qm_manager::line_rules::LineRulesDiff>>,
    )>,
    /// How many line rules comparisons ran (tests check the gate with it).
    pub(crate) line_rules_passes: AtomicU32,
    pub(crate) mod_detail: Mutex<HashMap<String, ModDetail>>,
    pub(crate) save_detail: Mutex<HashMap<String, SaveSummary>>,
    /// Per slot, the stamp's mod -> fields still in the save (Phase E D86); cleared with
    /// `save_detail`.
    pub(crate) save_stamped:
        Mutex<HashMap<String, std::collections::BTreeMap<String, Vec<String>>>>,
    pub(crate) health: Cached<HealthReport>,
    pub(crate) updates: Cached<UpdatesReport>,
    pub(crate) catalog: Cached<CatalogResult>,
    /// The game's Lua for the simulation's compile check, keyed by the DLL path it came from
    /// (or why it did not load).
    #[allow(clippy::type_complexity)]
    pub(crate) lua: Mutex<
        Option<(
            std::path::PathBuf,
            Result<Arc<qm_core::compile::LuaCompiler>, String>,
        )>,
    >,
}
