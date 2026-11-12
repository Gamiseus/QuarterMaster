pub mod assets_ledger;
pub mod community;
pub mod conflict;
mod consumer;
pub mod diagnostics_extra;
pub mod failure_groups;
pub mod placement;
pub mod devtools;
pub mod fixes;
pub mod library;
pub mod line_rules;
pub mod profiles;
pub mod region;
mod same_line;
pub mod semantic;
mod resolution;
pub mod runs;
pub mod save_cleanup;
pub mod save_fields;
pub mod solver;
pub mod stacks;
pub mod storage;
pub mod suggestions;
pub mod vanilla_assets;
pub mod winproc;

pub use conflict::{
    simulate_and_analyze_patches, simulate_and_analyze_patches_with, AttributedCodeLine,
    CompileCheck, ConflictResolutionSuggestion, DetectedConflict, EditCounts, RunsAsVerdict,
    SameLineDetail, SharedStack, SimulatedFileResult, SimulationOptions, SimulationReport,
    StackBlock, check_resolution, ResolutionCheck, ResolutionImpact,
};
pub use devtools::{
    decode_profile_code, encode_profile_code, lint_mod_directory, package_mod_release,
    register_windows_protocols, scaffold_new_mod, ModLintReport, ModpackImportPreview,
    ModpackManifest, ModpackModEntry,
};
pub use library::{
    copy_directory_clean, extract_zip_safely, inspect_mod_directory, validate_safe_relative_path,
    ArchiveModCandidate, ImportSummary, InstalledMod, ModKind,
};
pub use profiles::{
    build_plan, build_plan_for, resolve_profile_order, ActivateError, ActivateExtras, Activation, create_dir_junction_or_copy, is_game_running, is_reparse_point, remove_junction_or_dir,
    builtin_vanilla_profile, builtin_vanilla_slot, ModProfile, ProfileDiff, ProfileSeparator,
    SaveBackupEntry, SaveCompatibilityReport, SaveProfile, SaveTags, KNOWN_NOTICES, NOTICE_STACKING,
    NOTICE_LINE_RULES, UNKNOWN_DLL_ERROR_PREFIX, VANILLA_ID,
};
#[cfg(any(test, feature = "test-seams"))]
pub use profiles::{corrupt_next_staged_plan, set_corrupt_plan_after_overlay};
pub use solver::{
    default_community_masterlist, resolve_load_order, CommunityRule, LoadOrderResolution,
    ResolvedModOrderEntry, SolverDiagnostic,
};
pub use storage::{
    detect_game_installation, detect_save_directory, detect_workspace_mods_dir,
    extract_game_version_from_main_lua, LaunchMethod, LauncherSettings, PendingTransaction,
    QuarterMasterPaths, StateJournal, VanillaGameInfo,
};
