#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::version::{ParsedVersion, VersionReq};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

fn default_load_priority() -> i32 {
    50
}

fn default_entry_patch() -> String {
    "qm.toml".to_string()
}

/// Deserializes either `"authors": ["A", "B"]` or legacy `"author": "A"`.
#[derive(Deserialize)]
#[serde(untagged)]
enum AuthorsCompat {
    List(Vec<String>),
    Single(String),
}

fn deserialize_authors<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<AuthorsCompat>::deserialize(deserializer)?;
    Ok(match opt {
        Some(AuthorsCompat::List(v)) => v,
        Some(AuthorsCompat::Single(s)) => {
            if s.trim().is_empty() {
                Vec::new()
            } else {
                vec![s]
            }
        }
        None => Vec::new(),
    })
}

/// Deserializes legacy `"dependencies"` which could be `["gmenu", "gcore"]` or `{"gcore": ">=1.0.0"}`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum LegacyDependencies {
    List(Vec<String>),
    Map(HashMap<String, String>),
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// Rich dependency and ordering relationships (Fabric / Factorio style).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModRelationships {
    /// Hard dependencies (`mod_id` -> version range like `">=1.0.0"` or `"*"`).
    #[serde(default)]
    pub requires: HashMap<String, String>,

    /// Soft dependencies recommended for best experience.
    #[serde(default)]
    pub recommends: HashMap<String, String>,

    /// Optional companion mods.
    #[serde(default)]
    pub suggests: HashMap<String, String>,

    /// Incompatible mods (`conflicts` or `breaks`).
    #[serde(default, alias = "breaks")]
    pub conflicts: HashMap<String, String>,

    /// Ordering constraint only (load after these mod IDs if they are enabled, without requiring them).
    #[serde(default, alias = "loadAfter")]
    pub load_after: Vec<String>,

    /// Ordering constraint only (load before these mod IDs if they are enabled).
    #[serde(default, alias = "loadBefore")]
    pub load_before: Vec<String>,

    /// Virtual capability IDs satisfied by this mod.
    #[serde(default)]
    pub provides: Vec<String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// Parsed SMAPI-style update key (`GitHub:owner/repo#Asset.zip`, `Nexus:1234`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum ParsedUpdateKey {
    GitHub {
        owner: String,
        repo: String,
        asset: Option<String>,
    },
    Nexus {
        game_domain: String,
        mod_id: String,
    },
    Custom {
        raw: String,
    },
}

impl ParsedUpdateKey {
    pub fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed
            .strip_prefix("GitHub:")
            .or_else(|| trimmed.strip_prefix("github:"))
        {
            let (repo_part, asset) = match rest.split_once('#') {
                Some((r, a)) => (
                    r.trim(),
                    Some(a.trim().to_string()).filter(|s| !s.is_empty()),
                ),
                None => (rest.trim(), None),
            };
            if let Some((owner, repo)) = repo_part.split_once('/') {
                return ParsedUpdateKey::GitHub {
                    owner: owner.trim().to_string(),
                    repo: repo.trim().to_string(),
                    asset,
                };
            }
        } else if let Some(rest) = trimmed
            .strip_prefix("Nexus:")
            .or_else(|| trimmed.strip_prefix("nexus:"))
        {
            let parts: Vec<&str> = rest.split(':').collect();
            match parts.as_slice() {
                [domain, id] => {
                    return ParsedUpdateKey::Nexus {
                        game_domain: domain.trim().to_string(),
                        mod_id: id.trim().to_string(),
                    };
                }
                [id] if !id.trim().is_empty() => {
                    return ParsedUpdateKey::Nexus {
                        game_domain: "mudandblood".to_string(),
                        mod_id: id.trim().to_string(),
                    };
                }
                _ => {}
            }
        }
        ParsedUpdateKey::Custom {
            raw: trimmed.to_string(),
        }
    }
}

/// `save_content` in mod.json: what the mod adds that a save can hold (Phase E D87).
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveContentDecl {
    /// The mod's weapon tag -> the game weapon a saved one becomes while the mod is off.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub weapons: BTreeMap<String, String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// Standardized `mod.json` v1 manifest with full backwards-compatibility for existing mods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModManifest {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema_url: Option<String>,

    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    pub id: String,
    pub name: String,
    pub version: String,

    #[serde(default, alias = "author", deserialize_with = "deserialize_authors")]
    pub authors: Vec<String>,

    #[serde(default)]
    pub description: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,

    #[serde(default)]
    pub tags: Vec<String>,

    /// Ascending load priority: lower numbers load FIRST (e.g., GCore=5, GMenu=10, Gameplay=50).
    #[serde(default = "default_load_priority", alias = "priority")]
    pub load_priority: i32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,

    #[serde(default = "default_entry_patch")]
    pub entry_patch: String,

    /// SMAPI-style update keys, e.g. `["GitHub:Gamiseus/MnB-Mods#GCore.zip", "Nexus:1234"]`.
    #[serde(default, alias = "updateKeys")]
    pub update_keys: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus: Option<String>,

    #[serde(default)]
    pub relationships: ModRelationships,

    /// The oldest QuarterMaster this mod runs on: a version (that version or newer) or a range
    /// such as `">=1.2 <2"`. A QuarterMaster that does not meet it skips the mod (D61).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_qm: Option<String>,

    /// QuarterMaster features this mod needs (`qm_core::QM_FEATURES`). A QuarterMaster without
    /// one of them skips the mod (D61).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub qm_features: Vec<String>,

    /// Launcher check findings this mod's author checked and silences, as
    /// `"<check>:<name>"` or `"<check>:*"`, e.g. `"redefined_global:loadModSettings"` (D69).
    /// Not the qm.toml `[[overrides]]` entries.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub overrides: Vec<String>,

    /// Save content this mod adds to the game, so a save still loads when the mod is off
    /// (Phase E D87).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub save_content: Option<SaveContentDecl>,

    /// Legacy top-level `dependencies` field; merged into `relationships.requires` upon normalization.
    #[serde(default, skip_serializing)]
    dependencies: Option<LegacyDependencies>,
}

impl ModManifest {
    pub fn new(id: String, name: String, version: String) -> Self {
        Self {
            schema_url: Some("../../Quartermaster/schemas/mod.schema.json".to_string()),
            schema_version: CURRENT_SCHEMA_VERSION,
            id,
            name,
            version,
            authors: Vec::new(),
            description: String::new(),
            category: Some("Gameplay".to_string()),
            tags: Vec::new(),
            load_priority: default_load_priority(),
            game_version: Some(">=0.5.3".to_string()),
            entry_patch: default_entry_patch(),
            update_keys: Vec::new(),
            homepage: None,
            github: None,
            nexus: None,
            relationships: ModRelationships::default(),
            min_qm: None,
            qm_features: Vec::new(),
            overrides: Vec::new(),
            save_content: None,
            dependencies: None,
        }
    }

    /// Parse a `mod.json` string and normalize legacy fields into the v1 structure.
    pub fn from_json_str(raw: &str) -> Result<Self, String> {
        let mut manifest: ModManifest =
            serde_json::from_str(raw).map_err(|e| format!("Invalid mod.json: {e}"))?;
        manifest.normalize();
        Ok(manifest)
    }

    /// Load and normalize `mod.json` from disk.
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        Self::from_json_str(&raw)
    }

    /// Consolidates legacy `dependencies`, `github`, and `nexus` fields into `relationships` and `update_keys`.
    pub fn normalize(&mut self) {
        if let Some(legacy_deps) = self.dependencies.take() {
            match legacy_deps {
                LegacyDependencies::List(list) => {
                    for dep_id in list {
                        self.relationships
                            .requires
                            .entry(dep_id.clone())
                            .or_insert_with(|| "*".to_string());
                        if !self.relationships.load_after.contains(&dep_id) {
                            self.relationships.load_after.push(dep_id);
                        }
                    }
                }
                LegacyDependencies::Map(map) => {
                    for (dep_id, req) in map {
                        self.relationships
                            .requires
                            .entry(dep_id.clone())
                            .or_insert(req);
                        if !self.relationships.load_after.contains(&dep_id) {
                            self.relationships.load_after.push(dep_id);
                        }
                    }
                }
            }
        }

        // Ensure all required dependencies are also in load_after by default
        let req_keys: Vec<String> = self.relationships.requires.keys().cloned().collect();
        for dep_id in req_keys {
            if !self.relationships.load_after.contains(&dep_id) {
                self.relationships.load_after.push(dep_id);
            }
        }

        // Convert legacy github URL/repo into update_keys if update_keys is empty
        if let Some(ref gh) = self.github {
            let key = if let Some(rest) = gh.strip_prefix("https://github.com/") {
                let repo_clean = rest.trim_end_matches('/');
                format!("GitHub:{repo_clean}")
            } else if gh.contains('/') && !gh.starts_with("http") {
                format!("GitHub:{gh}")
            } else {
                String::new()
            };
            if !key.is_empty() && !self.update_keys.contains(&key) {
                self.update_keys.push(key);
            }
        }
    }

    pub fn parsed_update_keys(&self) -> Vec<ParsedUpdateKey> {
        self.update_keys
            .iter()
            .map(|k| ParsedUpdateKey::parse(k))
            .collect()
    }

    /// Validate manifest fields and return a list of lint warnings/errors.
    pub fn validate(&self) -> Vec<ManifestDiagnostic> {
        let mut diags = Vec::new();

        if self.id.trim().is_empty() {
            diags.push(ManifestDiagnostic::error("Mod `id` cannot be empty."));
        } else if !self
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            diags.push(ManifestDiagnostic::warning(format!(
                "Mod `id` '{}' should only contain lowercase ASCII letters, digits, '_', or '-'.",
                self.id
            )));
        }

        if self.name.trim().is_empty() {
            diags.push(ManifestDiagnostic::error("Mod `name` cannot be empty."));
        }

        if let Err(e) = ParsedVersion::parse(&self.version) {
            diags.push(ManifestDiagnostic::warning(format!(
                "Mod `version` '{}' is malformed: {e}",
                self.version
            )));
        }

        if let Some(ref gv) = self.game_version {
            if let Err(e) = VersionReq::parse(gv) {
                diags.push(ManifestDiagnostic::error(format!(
                    "Invalid `game_version` requirement '{}': {e}",
                    gv
                )));
            }
        }

        for (dep_id, range) in &self.relationships.requires {
            if let Err(e) = VersionReq::parse(range) {
                diags.push(ManifestDiagnostic::error(format!(
                    "Invalid version range '{}' for required dependency '{}': {e}",
                    range, dep_id
                )));
            }
        }

        if let Some(ref min_qm) = self.min_qm {
            let t = min_qm.trim();
            if t.is_empty() || (ParsedVersion::parse(t).is_err() && VersionReq::parse(t).is_err()) {
                diags.push(ManifestDiagnostic::error(format!(
                    "Invalid `min_qm` '{}': use a version such as '1.2.0' or a range such as '>=1.2 <2'.",
                    min_qm
                )));
            }
        }

        for (dep_id, range) in &self.relationships.conflicts {
            if let Err(e) = VersionReq::parse(range) {
                diags.push(ManifestDiagnostic::error(format!(
                    "Invalid version range '{}' for conflicting mod '{}': {e}",
                    range, dep_id
                )));
            }
        }

        diags
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
}

impl ManifestDiagnostic {
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            message: msg.into(),
        }
    }

    pub fn warning(msg: impl Into<String>) -> Self {
        Self {
            severity: DiagnosticSeverity::Warning,
            message: msg.into(),
        }
    }

    pub fn info(msg: impl Into<String>) -> Self {
        Self {
            severity: DiagnosticSeverity::Info,
            message: msg.into(),
        }
    }
}

/// Multi-mod release index (`qm-release-index.json`) attached to GitHub Releases.
/// Allows a single GitHub API call to check accurate per-mod versions and SHA-256 digests
/// even in monorepos like `MnB-Mods`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseIndex {
    pub schema_version: u32,
    pub generated_at: String,
    pub repository: String,
    pub release_tag: String,
    pub mods: HashMap<String, ReleaseModEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseModEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub asset_name: String,
    pub sha256: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
}

/// Generates the official JSON Schema for `mod.json` v1.
pub fn generate_mod_json_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "QuarterMaster Mod Manifest (mod.json v1)",
        "description": "Metadata, dependency relationships, load priority, and update keys for a QuarterMaster mod.",
        "type": "object",
        "required": ["id", "name", "version"],
        "properties": {
            "$schema": { "type": "string" },
            "schema_version": {
                "type": "integer",
                "default": 1,
                "description": "Schema version (currently 1)."
            },
            "id": {
                "type": "string",
                "pattern": "^[a-z0-9_-]+$",
                "description": "Unique lowercase identifier for the mod (e.g. 'gcore', 'tactical_overhaul')."
            },
            "name": {
                "type": "string",
                "description": "Human-readable display name of the mod."
            },
            "version": {
                "type": "string",
                "description": "Semantic version of the mod (e.g. '1.0.0')."
            },
            "authors": {
                "type": "array",
                "items": { "type": "string" },
                "description": "List of mod authors."
            },
            "description": {
                "type": "string",
                "description": "Summary description of the mod's features."
            },
            "category": {
                "type": "string",
                "enum": ["Core", "UI", "Gameplay", "Content", "Audio", "Visuals", "Tools", "Dev", "Modpack", "Other"],
                "description": "Primary organizational category."
            },
            "tags": {
                "type": "array",
                "items": { "type": "string" }
            },
            "load_priority": {
                "type": "integer",
                "default": 50,
                "description": "Ascending load priority: lower numbers execute FIRST (e.g. 5 = Core Framework, 50 = Gameplay, 90 = Dev/Tools)."
            },
            "game_version": {
                "type": "string",
                "description": "Supported Mud and Blood version range (e.g. '>=0.5.3')."
            },
            "entry_patch": {
                "type": "string",
                "default": "qm.toml",
                "description": "Root patch manifest file (defaults to 'qm.toml')."
            },
            "update_keys": {
                "type": "array",
                "items": { "type": "string" },
                "description": "SMAPI-style update keys, e.g. 'GitHub:owner/repo#Asset.zip' or 'Nexus:1234'."
            },
            "homepage": { "type": "string" },
            "min_qm": {
                "type": "string",
                "description": "Oldest QuarterMaster this mod runs on: a version ('1.2.0' means that version or newer) or a range ('>=1.2 <2'). An older QuarterMaster skips the mod."
            },
            "qm_features": {
                "type": "array",
                "items": { "type": "string" },
                "description": "QuarterMaster features this mod needs, e.g. 'overrides' or 'anchors'. A QuarterMaster without one of them skips the mod."
            },
            "save_content": {
                "type": "object",
                "description": "What this mod adds that a save can hold, so the save still loads when the mod is off. See the modding wiki, Part 1, Saves and your mod: https://gamiseus.github.io/wiki/",
                "properties": {
                    "weapons": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "The mod's weapon tag -> the game weapon (its tag, e.g. 'k98') a saved one becomes while the mod is off."
                    }
                }
            },
            "overrides": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Launcher check findings the author checked and silences, as '<check>:<name>' or '<check>:*', e.g. 'redefined_global:loadModSettings'. Not the qm.toml overrides. See the modding wiki, Part 7: https://gamiseus.github.io/wiki/"
            },
            "relationships": {
                "type": "object",
                "properties": {
                    "requires": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Hard dependencies mapped to version ranges (e.g. {'gcore': '>=1.0.0'})."
                    },
                    "recommends": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    },
                    "suggests": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    },
                    "conflicts": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    },
                    "load_after": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "load_before": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "provides": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                }
            }
        }
    })
}

/// Generates the official JSON Schema for `qm.toml` patch files.
pub fn generate_qm_toml_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "QuarterMaster Patch Manifest (qm.toml)",
        "description": "In-memory Lua patch definitions for QuarterMaster.",
        "type": "object",
        "required": ["manifest"],
        "definitions": {
            "rung": {
                "type": "object",
                "description": "A place on another patch's code (within) or in the whole file. See the modding wiki, Part 7: https://gamiseus.github.io/wiki/",
                "properties": {
                    "within": { "type": "string", "description": "The other patch, as \"mod_id:patch_id\"." },
                    "edge": { "type": "string", "enum": ["start", "end"] },
                    "pattern": { "type": "string" },
                    "exact": { "type": "boolean", "default": false },
                    "nth": { "type": "integer", "minimum": 1 },
                    "position": { "type": "string", "enum": ["before", "after"] },
                    "b_site": { "type": "integer", "minimum": 0 },
                    "check": { "type": "string" },
                    "b_fp": {
                        "type": "object",
                        "description": "The other patch's content when the rung was made.",
                        "required": ["anchor", "payload", "source", "ordinal"],
                        "properties": {
                            "v": { "type": "integer" },
                            "anchor": { "type": "string" },
                            "payload": { "type": "string" },
                            "source": { "type": "string" },
                            "ordinal": { "type": "integer" },
                            "mod_version": { "type": "string" }
                        }
                    }
                }
            },
            "site": {
                "type": "object",
                "required": ["context"],
                "properties": {
                    "context": { "type": "string" },
                    "ordinal": { "type": "integer", "minimum": 1 }
                }
            },
            "override": {
                "type": "object",
                "required": ["patch"],
                "additionalProperties": false,
                "properties": {
                    "patch": { "type": "string", "description": "The patch to change, as \"mod_id:patch_id\"." },
                    "fingerprint": {
                        "type": "object",
                        "required": ["anchor", "payload"],
                        "properties": {
                            "anchor": { "type": "string" },
                            "payload": { "type": "string" },
                            "source": { "type": "string" },
                            "ordinal": { "type": "integer" }
                        }
                    },
                    "disable": { "type": "boolean", "default": false },
                    "priority": { "type": "integer" },
                    "before": { "type": "array", "items": { "type": "string" } },
                    "after": { "type": "array", "items": { "type": "string" } },
                    "anchors": { "type": "array", "items": { "$ref": "#/definitions/rung" } },
                    "sites": { "type": "array", "items": { "$ref": "#/definitions/site" } },
                    "replace_anchors": { "type": "boolean", "default": false },
                    "apply_as": { "type": "string", "enum": ["literal", "edits"] },
                    "skip_hunks": {
                        "type": "array",
                        "description": "Edits of the \"at\" patch that do not run, so another patch's change to the same lines stays. The patch then runs as separate edits.",
                        "items": { "$ref": "#/definitions/hunk" }
                    },
                    "stack_below": { "type": "string", "description": "At a shared line, put this patch's block below this patch's block, as \"mod_id:patch_id\"." },
                    "stack_above": { "type": "string", "description": "At a shared line, put this patch's block above this patch's block, as \"mod_id:patch_id\"." },
                    "stack_site": { "$ref": "#/definitions/site", "description": "The game's line the stack rule is for. Every line where both blocks meet when absent." },
                    "stack_side": { "type": "string", "enum": ["before", "after"], "description": "The side of that line. Both sides when absent." }
                }
            },
            "hunk": {
                "type": "object",
                "description": "One edit of an \"at\" patch.",
                "required": ["fp", "vanilla", "ordinal"],
                "properties": {
                    "fp": {
                        "type": "object",
                        "description": "The patch's content when the choice was made.",
                        "required": ["anchor", "payload"],
                        "properties": {
                            "anchor": { "type": "string" },
                            "payload": { "type": "string" },
                            "source": { "type": "string" },
                            "ordinal": { "type": "integer" }
                        }
                    },
                    "vanilla": { "type": "string", "description": "The edit's pattern lines, trimmed, one per line." },
                    "ordinal": { "type": "integer", "minimum": 0, "description": "The edit's place among the patch's edits, from 0." },
                    "keep": { "type": "string", "description": "The patch whose change to these lines stays, as \"mod_id:patch_id\"." }
                }
            }
        },
        "properties": {
            "manifest": {
                "type": "object",
                "required": ["version"],
                "properties": {
                    "version": { "type": "string" },
                    "priority": {
                        "type": "integer",
                        "default": 50,
                        "description": "Ascending priority (lower numbers run first)."
                    },
                    "dump_lua": { "type": "boolean", "default": false }
                }
            },
            "publish": {
                "type": "object",
                "description": "Lists the mod on the QuarterMaster site and in the launcher's Browse tab. The game ignores this table. See the modding wiki, Part 11: https://gamiseus.github.io/wiki/",
                "required": ["repository"],
                "properties": {
                    "repository": { "type": "string", "description": "The GitHub repository that publishes the mod, as \"owner/name\"." },
                    "asset": { "type": "string", "description": "The release file to install, a .zip or .qmpack. Without it, the first one in the release." },
                    "summary": { "type": "string", "maxLength": 140, "description": "One line for the mod list. Without it, the first line of the mod's description." },
                    "image": { "type": "string", "description": "A picture for the mod's card: a .png, .jpg, .webp or .gif in the mod's folder, at most 1 MB." },
                    "channel": { "type": "string", "enum": ["stable"] }
                }
            },
            "vars": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            },
            "sprites": {
                "type": "object",
                "description": "The size an image is shown at, by the name of its global (e.g. top_m1), whatever the file's resolution. A replaced image takes the original's size without a rule. See the modding wiki, Part 2: https://gamiseus.github.io/wiki/",
                "additionalProperties": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "size": {
                            "type": "array",
                            "items": { "type": "number", "exclusiveMinimum": 0 },
                            "minItems": 2,
                            "maxItems": 2,
                            "description": "Width and height to show the image at, in the game's pixels."
                        },
                        "scale": {
                            "type": "number",
                            "exclusiveMinimum": 0,
                            "description": "A fraction of the image's own pixel size: 0.25 shows a 400x400 file at 100x100."
                        },
                        "like": {
                            "type": "string",
                            "description": "Show the image at the size another image global is shown at."
                        }
                    }
                }
            },
            "overrides": {
                "type": "array",
                "description": "Changes to other mods' patches. Every mod an entry names must be in this mod's requires.",
                "items": { "$ref": "#/definitions/override" }
            },
            "patches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "priority": { "type": "integer" },
                        "before": { "type": "array", "items": { "type": "string" } },
                        "after": { "type": "array", "items": { "type": "string" } },
                        "on_failure": {
                            "type": "string",
                            "enum": ["warn", "skip_patch", "skip_mod", "abort"],
                            "default": "warn"
                        },
                        "pattern": {
                            "type": "object",
                            "required": ["target", "pattern", "position", "payload"],
                            "properties": {
                                "id": { "type": "string" },
                                "name": { "type": "string" },
                                "target": {
                                    "oneOf": [
                                        { "type": "string" },
                                        { "type": "array", "items": { "type": "string" } }
                                    ]
                                },
                                "pattern": { "type": "string" },
                                "position": { "type": "string", "enum": ["at", "before", "after"] },
                                "payload": { "type": "string" },
                                "match_indent": { "type": "boolean", "default": false },
                                "times": { "type": "integer", "minimum": 1 },
                                "expect": { "type": "integer", "minimum": 1 },
                                "on_failure": { "type": "string", "enum": ["warn", "skip_patch", "skip_mod", "abort"] },
                                "anchors": { "type": "array", "items": { "$ref": "#/definitions/rung" } },
                                "sites": { "type": "array", "items": { "$ref": "#/definitions/site" } },
                                "apply_as": {
                                    "type": "string",
                                    "enum": ["literal", "edits"],
                                    "default": "literal",
                                    "description": "\"edits\" runs an \"at\" patch as separate edits next to the lines it keeps."
                                },
                                "stack": {
                                    "type": "string",
                                    "enum": ["first", "last"],
                                    "description": "Where this patch's block goes among the code other patches add at its line: \"first\" on top (it runs first), \"last\" at the bottom. A mod it needs stays above it."
                                },
                                "requires": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "Patches this patch needs, by id, \"mod_id:patch_id\" or \"mod_id:*\". One in the same file runs first; when it did not apply, this patch is skipped."
                                }
                            }
                        },
                        "regex": {
                            "type": "object",
                            "required": ["target", "pattern", "position", "payload"],
                            "properties": {
                                "id": { "type": "string" },
                                "name": { "type": "string" },
                                "target": {
                                    "oneOf": [
                                        { "type": "string" },
                                        { "type": "array", "items": { "type": "string" } }
                                    ]
                                },
                                "pattern": { "type": "string" },
                                "position": { "type": "string", "enum": ["at", "before", "after"] },
                                "root_capture": { "type": "string" },
                                "payload": { "type": "string" },
                                "line_prepend": { "type": "string" },
                                "verbose": { "type": "boolean", "default": false },
                                "times": { "type": "integer", "minimum": 1 },
                                "expect": { "type": "integer", "minimum": 1 },
                                "on_failure": { "type": "string", "enum": ["warn", "skip_patch", "skip_mod", "abort"] },
                                "anchors": { "type": "array", "items": { "$ref": "#/definitions/rung" } },
                                "sites": { "type": "array", "items": { "$ref": "#/definitions/site" } },
                                "stack": {
                                    "type": "string",
                                    "enum": ["first", "last"],
                                    "description": "Where this patch's block goes among the code other patches add at its line: \"first\" on top (it runs first), \"last\" at the bottom. A mod it needs stays above it."
                                },
                                "requires": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "Patches this patch needs, by id, \"mod_id:patch_id\" or \"mod_id:*\". One in the same file runs first; when it did not apply, this patch is skipped."
                                }
                            }
                        },
                        "copy": {
                            "type": "object",
                            "required": ["target", "position"],
                            "properties": {
                                "id": { "type": "string" },
                                "name": { "type": "string" },
                                "target": {
                                    "oneOf": [
                                        { "type": "string" },
                                        { "type": "array", "items": { "type": "string" } }
                                    ]
                                },
                                "position": { "type": "string", "enum": ["prepend", "append"] },
                                "sources": { "type": "array", "items": { "type": "string" } },
                                "payload": { "type": "string" }
                            }
                        },
                        "module": {
                            "type": "object",
                            "required": ["source", "name"],
                            "properties": {
                                "id": { "type": "string" },
                                "source": { "type": "string" },
                                "name": { "type": "string" },
                                "before": { "type": "string" },
                                "load_now": { "type": "boolean", "default": false }
                            }
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_legacy_gspecialevents_and_tactical_overhaul() {
        let legacy_gse = r#"{
            "id": "g_special_events",
            "name": "GSpecialEvents",
            "version": "1.0.0",
            "description": "Special Events",
            "author": "Mud and Blood Modding Team",
            "priority": 80,
            "dependencies": ["gmenu", "gcore"]
        }"#;
        let m = ModManifest::from_json_str(legacy_gse).unwrap();
        assert_eq!(m.authors, vec!["Mud and Blood Modding Team"]);
        assert_eq!(m.load_priority, 80);
        assert_eq!(m.relationships.requires.get("gmenu").unwrap(), "*");
        assert_eq!(m.relationships.requires.get("gcore").unwrap(), "*");
        assert!(m.relationships.load_after.contains(&"gcore".to_string()));

        let legacy_to = r#"{
            "id": "tactical_overhaul",
            "name": "Tactical Overhaul",
            "version": "1.0.0",
            "authors": ["Community Mod Team"],
            "load_priority": 20,
            "dependencies": {
                "gcore": ">=1.0.0"
            },
            "update_keys": ["GitHub:Gamiseus/MnB-Mods#TacticalOverhaul.zip"]
        }"#;
        let m2 = ModManifest::from_json_str(legacy_to).unwrap();
        assert_eq!(m2.relationships.requires.get("gcore").unwrap(), ">=1.0.0");
        assert_eq!(
            m2.parsed_update_keys()[0],
            ParsedUpdateKey::GitHub {
                owner: "Gamiseus".to_string(),
                repo: "MnB-Mods".to_string(),
                asset: Some("TacticalOverhaul.zip".to_string()),
            }
        );
    }

    #[test]
    fn reads_feature_gates_and_leaves_them_out_when_unset() {
        let m = ModManifest::from_json_str(
            r#"{"id": "c", "name": "C", "version": "1.0.0", "min_qm": "1.1.0", "qm_features": ["overrides", "anchors"]}"#,
        )
        .unwrap();
        assert_eq!(m.min_qm.as_deref(), Some("1.1.0"));
        assert_eq!(m.qm_features, ["overrides", "anchors"]);
        assert!(m
            .validate()
            .iter()
            .all(|d| d.severity != DiagnosticSeverity::Error));
        let back = serde_json::to_value(&m).unwrap();
        assert_eq!(back["min_qm"], "1.1.0");
        assert_eq!(
            back["qm_features"],
            serde_json::json!(["overrides", "anchors"])
        );

        let plain =
            ModManifest::from_json_str(r#"{"id": "p", "name": "P", "version": "1.0.0"}"#).unwrap();
        assert_eq!(plain.min_qm, None);
        assert!(plain.qm_features.is_empty());
        let v = serde_json::to_value(&plain).unwrap();
        assert!(
            v.get("min_qm").is_none() && v.get("qm_features").is_none(),
            "{v}"
        );

        let silenced = ModManifest::from_json_str(
            r#"{"id": "s", "name": "S", "version": "1.0.0", "overrides": ["dead_code:love.keypressed"]}"#,
        )
        .unwrap();
        assert_eq!(silenced.overrides, ["dead_code:love.keypressed"]);
        assert_eq!(
            serde_json::to_value(&silenced).unwrap()["overrides"],
            serde_json::json!(["dead_code:love.keypressed"])
        );
        assert!(v.get("overrides").is_none(), "{v}");
        assert!(v.get("save_content").is_none(), "{v}");

        let weapons = ModManifest::from_json_str(
            r#"{"id": "w", "name": "W", "version": "1.0.0", "save_content": {"weapons": {"example_rifle": "k98"}}}"#,
        )
        .unwrap();
        assert_eq!(
            weapons.save_content.as_ref().unwrap().weapons["example_rifle"],
            "k98"
        );
        assert_eq!(
            serde_json::to_value(&weapons).unwrap()["save_content"]["weapons"]["example_rifle"],
            "k98"
        );

        let bad = ModManifest::from_json_str(
            r#"{"id": "b", "name": "B", "version": "1.0.0", "min_qm": "soon"}"#,
        )
        .unwrap();
        assert!(bad
            .validate()
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("min_qm")));
    }

    #[test]
    fn generates_and_writes_json_schemas() {
        let schemas_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("schemas");
        std::fs::create_dir_all(&schemas_dir).unwrap();

        let mod_schema = serde_json::to_string_pretty(&generate_mod_json_schema()).unwrap();
        std::fs::write(schemas_dir.join("mod.schema.json"), &mod_schema).unwrap();

        let qm_schema = serde_json::to_string_pretty(&generate_qm_toml_schema()).unwrap();
        std::fs::write(schemas_dir.join("qm.schema.json"), &qm_schema).unwrap();

        assert!(schemas_dir.join("mod.schema.json").exists());
        assert!(schemas_dir.join("qm.schema.json").exists());
    }
}
