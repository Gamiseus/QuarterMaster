//! Cache primitives and the cached service state.
use qm_manager::{InstalledMod, SimulationReport, VanillaGameInfo};
use qm_schema::SaveSummary;
use crate::dto::ModDetail;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
}

// Placeholders until Task 9 defines the real types.
#[derive(Debug, Clone, Default)]
pub struct HealthReport;
#[derive(Debug, Clone, Default)]
pub struct UpdatesReport;
#[derive(Debug, Clone, Default)]
pub struct CatalogResult;

// Fields are crate-private so only `Service::invalidate` mutates them; see `testing` helpers.
#[derive(Default)]
pub struct Caches {
    pub(crate) mods: Cached<Arc<Vec<InstalledMod>>>,
    pub(crate) vanilla: Cached<Arc<VanillaFiles>>,
    pub(crate) simulation: Cached<Arc<SimulationReport>>,
    pub(crate) mod_detail: Mutex<HashMap<String, ModDetail>>,
    pub(crate) save_detail: Mutex<HashMap<String, SaveSummary>>,
    pub(crate) health: Cached<HealthReport>,
    pub(crate) updates: Cached<UpdatesReport>,
    pub(crate) catalog: Cached<CatalogResult>,
}
