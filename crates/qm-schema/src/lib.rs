#![recursion_limit = "256"]

pub mod config_schema;
pub mod manifest;
pub mod save_parser;
pub mod security;
pub mod version;

pub use config_schema::{
    ConfigOptionDef, ConfigOptionType, ConfigValue, ModConfigSchema, ModSettingsStore, ShowIfRule,
};
pub use manifest::{
    generate_mod_json_schema, generate_qm_toml_schema, DiagnosticSeverity, ManifestDiagnostic,
    ModManifest, ModRelationships, ParsedUpdateKey, ReleaseIndex, ReleaseModEntry,
    CURRENT_SCHEMA_VERSION,
};
pub use save_parser::{
    campaign_rank_title, classic_rank_title, compare_saves, soldier_rank_title, trench_rank_title,
    ReinforcementCardSummary, SaveComparison, SaveQmMeta, SaveSummary, SerDocument, SerTable,
    SerValue, SoldierSummary,
};
pub use security::{
    sha256_hex, IntegrityManifest, IntegrityVerifyResult, SecurityCapability, SecurityFinding,
    SecurityScanReport,
};
pub use version::{
    evaluate_update, CompOp, Comparator, ParsedVersion, PreReleaseIdent, UpdateEvaluation,
    VersionReq,
};
