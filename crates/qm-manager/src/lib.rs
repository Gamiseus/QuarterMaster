pub mod community;
pub mod conflict;
pub mod devtools;
pub mod library;
pub mod profiles;
pub mod solver;
pub mod storage;

pub use conflict::{
    simulate_and_analyze_patches, AttributedCodeLine, ConflictResolutionSuggestion,
    DetectedConflict, SimulatedFileResult, SimulationReport,
};
pub use devtools::{
    decode_profile_code, encode_profile_code, lint_mod_directory, package_mod_release,
    register_windows_protocols, scaffold_new_mod, ModLintReport, ModpackImportPreview,
    ModpackManifest, ModpackModEntry,
};
pub use library::{
    copy_directory_clean, extract_zip_safely, inspect_mod_directory, validate_safe_relative_path,
    ArchiveModCandidate, ImportSummary, InstalledMod,
};
pub use profiles::{
    create_dir_junction_or_copy, is_game_running, is_reparse_point, remove_junction_or_dir,
    ModProfile, ProfileDiff, ProfileSeparator, SaveBackupEntry, SaveCompatibilityReport,
    SaveProfile,
};
pub use solver::{
    default_community_masterlist, resolve_load_order, CommunityRule, LoadOrderResolution,
    ResolvedModOrderEntry, SolverDiagnostic,
};
pub use storage::{
    detect_game_installation, detect_save_directory, detect_workspace_mods_dir,
    extract_game_version_from_main_lua, LaunchMethod, LauncherSettings, PendingTransaction,
    QuarterMasterPaths, StateJournal, VanillaGameInfo,
};
