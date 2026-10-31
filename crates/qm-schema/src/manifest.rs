use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
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
                Some((r, a)) => (r.trim(), Some(a.trim().to_string()).filter(|s| !s.is_empty())),
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
            if parts.len() == 2 {
                return ParsedUpdateKey::Nexus {
                    game_domain: parts[0].trim().to_string(),
                    mod_id: parts[1].trim().to_string(),
                };
            } else if parts.len() == 1 && !parts[0].trim().is_empty() {
                return ParsedUpdateKey::Nexus {
                    game_domain: "mudandblood".to_string(),
                    mod_id: parts[0].trim().to_string(),
                };
            }
        }
        ParsedUpdateKey::Custom {
            raw: trimmed.to_string(),
        }
    }
}

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

    #[serde(
        default,
        alias = "author",
        deserialize_with = "deserialize_authors"
    )]
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
        "required": ["manifest", "patches"],
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
            "vars": {
                "type": "object",
                "additionalProperties": { "type": "string" }
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
                                "on_failure": { "type": "string", "enum": ["warn", "skip_patch", "skip_mod", "abort"] }
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
                                "on_failure": { "type": "string", "enum": ["warn", "skip_patch", "skip_mod", "abort"] }
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
