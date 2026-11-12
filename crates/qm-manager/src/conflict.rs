use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use qm_core::dump::PatchRegion;
use qm_core::handshake::{
    FixBinding, GluePatchEntry, InvalidRung, LaunchConfig, LoadError, OverrideReport,
    PatchExecutionRecord, PatchOutcomeStatus, RunAs, SuspendedRung,
};
use qm_core::compile::LuaCompiler;
use qm_core::patch::edits::{self, ApplyAs, EditsRefusal};
use qm_core::patch::origin::FileTrace;
use qm_core::patch::table::{PatchTable, TracedApply};
use qm_core::prepare::{simulate_buffer, FileView};
use qm_core::patch::{InsertPosition, OnFailurePolicy, Patch, PatternPatch, Priority, Target};

pub use crate::consumer::{ConsumerInfo, ConsumerKind, RungHint, SiteLoss};
pub use crate::failure_groups::{FailureGroup, FailureMember, FailureReason, InferredEdge};
pub use crate::stacks::{SharedStack, StackBlock};
use crate::stacks::shared_stacks;
pub use crate::resolution::{
    apply_suggestion, check_resolution, compare_resolution, resolve_conflict, ResolutionCheck,
    ResolutionImpact, NOT_IMPORTED,
};

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationReport {
    pub files_touched: Vec<SimulatedFileResult>,
    pub conflicts: Vec<DetectedConflict>,
    pub module_registrations: BTreeMap<String, Vec<String>>,
    pub total_patches: usize,
    pub applied_patches: usize,
    pub broken_anchors: usize,
    /// Patch order cycles the engine had to break (`kind: "patch_cycle"`).
    #[serde(default)]
    pub order_diagnostics: Vec<crate::solver::SolverDiagnostic>,
    /// Per touched file, the full patched text and where each line came from, as the file shows
    /// it: QuarterMaster's own lines hidden, the file's own numbering (D1). Kept in memory for
    /// the service (anchor search, previews); never sent to the UI.
    #[serde(skip, default)]
    #[cfg_attr(feature = "specta", specta(skip))]
    pub traces: BTreeMap<String, Arc<FileTrace>>,
    /// Whether patched files were compiled with the game's Lua (`compile_error` conflicts).
    #[serde(default)]
    pub compile_check: CompileCheck,
    /// load-order.json could not be used, so the game loads no mods (D16).
    #[serde(default)]
    pub plan_error: Option<String>,
    /// Plan entries that were skipped (D17).
    #[serde(default)]
    pub plan_warnings: Vec<String>,
    /// Mods that fail to load, each skipped with its requirers, as in the game (D15).
    #[serde(default)]
    pub load_errors: Vec<LoadError>,
    /// Plan fixes that do not apply as written, as in the game (an adjusted copy waiting for
    /// its mod, D19).
    #[serde(default)]
    pub fix_bindings: Vec<FixBinding>,
    /// Order rules a stronger rule replaced or ignored, as in the game (§3.9).
    #[serde(default)]
    pub overridden_rules: Vec<String>,
    /// Names in order rules that match no loaded patch.
    #[serde(default)]
    pub unknown_ids: Vec<String>,
    /// Rungs suspended to break an order cycle; their patches use their own lines.
    #[serde(default)]
    pub suspended_rungs: Vec<SuspendedRung>,
    /// Rungs that are never used, with why.
    #[serde(default)]
    pub invalid_rungs: Vec<InvalidRung>,
    /// What became of each compatibility mod's overrides, as in the game (§8.1).
    #[serde(default)]
    pub overrides: Vec<OverrideReport>,
    /// Places pattern patches lost to other patches (§9.1), for the suggestions. Kept in
    /// memory for the service; never sent to the UI.
    #[serde(skip, default)]
    #[cfg_attr(feature = "specta", specta(skip))]
    pub losses: Vec<SiteLoss>,
    /// Per file, patches whose code hangs on a line another mod's patch wrote back unchanged,
    /// with the rung that ties them to it (D26). In memory only.
    #[serde(skip, default)]
    #[cfg_attr(feature = "specta", specta(skip))]
    pub kept_anchors: Vec<(String, RungHint)>,
    /// How each `at` pattern patch runs: literal or as separate edits (D46, §6.6).
    #[serde(default)]
    pub runs_as: Vec<RunsAsVerdict>,
    /// Patches that fail because code they anchor on did not apply, by root (G10).
    #[serde(default)]
    pub failure_groups: Vec<FailureGroup>,
    /// Patches that anchor on another patch's code, for the Order view (G10).
    #[serde(default)]
    pub inferred_edges: Vec<InferredEdge>,
    /// The save fields the mods write, with their owners (Phase E D84).
    #[serde(default)]
    pub save_fields: crate::save_fields::SaveFieldRegistry,
}

/// How one `at` pattern patch runs (D46, §6.6). The separate edits are worked out for every
/// `at` patch, whatever it asks for, so the launcher can offer them.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunsAsVerdict {
    pub patch_id: String,
    pub mod_id: String,
    /// What it asks for: a profile fix, a compat mod or its author.
    pub asks: ApplyAs,
    /// What it runs as: edits only when it asks for them and its lines pair one way.
    pub runs_as: ApplyAs,
    /// Its separate edits, counted in lines, when its lines pair one way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<EditCounts>,
    /// Why it cannot run as separate edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<EditsRefusal>,
    /// "Runs as 3 insertions" or "Runs as one replacement (the lines are ambiguous)".
    pub text: String,
}

/// The separate edits of an `at` patch, in lines.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditCounts {
    pub kept: u32,
    pub inserted: u32,
    pub changed: u32,
    pub removed: u32,
}

/// The verdict of every `at` pattern patch in the table, from the prepared hunks.
pub fn runs_as_verdicts(
    table: &PatchTable,
    prepared: &qm_core::patch::table::Prepared,
) -> Vec<RunsAsVerdict> {
    (0..table.patches.len())
        .filter_map(|ix| runs_as_verdict(table, prepared, ix))
        .collect()
}

/// The verdict of patch `ix` of the table; None when it is not an `at` pattern patch.
pub fn runs_as_verdict(
    table: &PatchTable,
    prepared: &qm_core::patch::table::Prepared,
    ix: usize,
) -> Option<RunsAsVerdict> {
    let n = |x: usize| u32::try_from(x).unwrap_or(u32::MAX);
    let Patch::Pattern(p) = &table.patches.get(ix)?.0 else {
        return None;
    };
    let normalized = prepared.hunks(ix)?;
    let asks = p.asked_apply_as();
    let edits = normalized.as_ref().ok().map(|h| {
        let (kept, inserted, changed, removed) = edits::counts(h);
        EditCounts {
            kept: n(kept),
            inserted: n(inserted),
            changed: n(changed),
            removed: n(removed),
        }
    });
    let refused = normalized.as_ref().err().copied();
    let runs_as = match (asks, edits) {
        (ApplyAs::Edits, Some(_)) => ApplyAs::Edits,
        _ => ApplyAs::Literal,
    };
    let text = match (runs_as, edits, refused) {
        (ApplyAs::Edits, Some(c), _) => runs_as_text(&c),
        (_, _, Some(r)) if asks == ApplyAs::Edits => {
            format!("Runs as one replacement ({})", r.why())
        }
        _ => "Runs as one replacement".to_string(),
    };
    Some(RunsAsVerdict {
        patch_id: p.id.clone().unwrap_or_default(),
        mod_id: p.mod_id.clone().unwrap_or_default(),
        asks,
        runs_as,
        edits,
        refused,
        text,
    })
}

/// "Runs as 3 insertions", "Runs as 1 insertion and 1 changed line".
fn runs_as_text(c: &EditCounts) -> String {
    let count = |n: u32, one: &str, many: &str| match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {many}")),
    };
    let parts: Vec<String> = [
        count(c.inserted, "insertion", "insertions"),
        count(c.changed, "changed line", "changed lines"),
        count(c.removed, "removed line", "removed lines"),
    ]
    .into_iter()
    .flatten()
    .collect();
    match parts.as_slice() {
        [] => "Runs as separate edits that change no line".to_string(),
        [one] => format!("Runs as {one}"),
        [rest @ .., last] => format!("Runs as {} and {last}", rest.join(", ")),
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompileCheck {
    pub ran: bool,
    pub files_checked: usize,
    /// Why the check did not run, or files it could not judge.
    pub note: Option<String>,
}

/// Optional extras for a simulation run.
#[derive(Default, Clone, Copy)]
pub struct SimulationOptions<'a> {
    /// The game's Lua, for the compile check. `None` skips it.
    pub compiler: Option<&'a LuaCompiler>,
    /// The launch config the DLL reads, for QuarterMaster's own edits. `None` reads
    /// `qm/launch-config.json` from the table's Mods folder once per run.
    pub launch: Option<&'a LaunchConfig>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedFileResult {
    pub target_file: String,
    pub mods_touching: Vec<String>,
    pub records: Vec<PatchExecutionRecord>,
    pub attributed_lines: Vec<AttributedCodeLine>,
    pub original_line_count: usize,
    pub patched_line_count: usize,
    /// Lines with code from two or more patches on one side (§5.5).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub stacks: Vec<SharedStack>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributedCodeLine {
    pub line_number: usize,
    pub text: String,
    pub mod_id: Option<String>,
    pub patch_id: Option<String>,
    pub patch_name: Option<String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tier", rename_all = "snake_case")]
pub enum ConflictResolutionSuggestion {
    /// Tier 1: Flipping the execution order of these two mods/patches resolves the broken anchor
    /// so both patches apply cleanly without editing any code.
    SwapModOrder {
        move_mod_id: String,
        before_mod_id: String,
        move_patch_id: String,
        suggested_priority: Priority,
        explanation: String,
    },
    /// Tier 2: another mod's patch replaced the anchor line of a before/after patch; a
    /// profile-local copy anchored on that patch's code. It is simulated before it is applied,
    /// and refused when it breaks a file or adds a conflict (D63).
    GenerateGluePatch {
        glue_patch: GluePatchEntry,
        explanation: String,
    },
    /// Tier 3: Mutually exclusive replacement (`position = "at"` or duplicate module);
    /// lets the user pick which mod's patch wins.
    SelectWinner {
        patch_a_id: String,
        mod_a_id: String,
        patch_b_id: String,
        mod_b_id: String,
        explanation: String,
    },
    /// A pattern patch whose lines another mod changed or split: run it as separate edits
    /// (D46). Made as an ApplyAs fix, previewed before it is applied (D63).
    ApplyAs {
        patch_id: String,
        mod_id: String,
        explanation: String,
    },
    /// Two patches change one line: keep `keep_patch_id`'s change, so `patch_id` skips its
    /// own change there (edit `ordinal`) and runs its other edits (D49). Made as a SkipHunk
    /// fix, previewed before it is applied.
    SkipHunk {
        patch_id: String,
        mod_id: String,
        ordinal: u32,
        keep_patch_id: String,
        keep_mod_id: String,
        explanation: String,
    },
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedConflict {
    pub id: String,
    pub severity: String, // "error" | "warning" | "info"
    pub kind: String,     // "broken_anchor_order" | "duplicate_module" | "shadowed_file" | "patch_aborts_file" | "target_is_module" | ...
    pub target_file: String,
    pub mod_a_id: String,
    pub mod_b_id: Option<String>,
    pub patch_a_id: String,
    pub patch_b_id: Option<String>,
    pub description: String,
    pub suggestions: Vec<ConflictResolutionSuggestion>,
    /// The patch that took patch A's place, and how (§9.1). Set on broken anchors and lost
    /// places when another patch took the line.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub consumer: Option<ConsumerInfo>,
    /// `(lost, total)`: how many of patch A's places in the game's file it lost.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub sites_lost: Option<(u32, u32)>,
    /// For a `same_line_edit`, the line and each patch's version of it (D49).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub same_line: Option<SameLineDetail>,
}

/// The line two patches both change (D49, §6.5), for the side-by-side view.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SameLineDetail {
    /// 1-based, as the game's file numbers it.
    pub line: u32,
    /// The line as the game has it, trimmed.
    pub vanilla: String,
    /// What patch A writes in its place, trimmed ("" when it removes the line). None when A
    /// replaces its lines as one block, so no one line of it stands for this one.
    pub a_text: Option<String>,
    pub b_text: Option<String>,
}

/// Runs the Offline Patch Engine across `patch_table` and `vanilla_files` (`rel_path -> content`),
/// builds attributed file diffs, and runs the 3-Tier Conflict Resolver.
pub fn simulate_and_analyze_patches(
    patch_table: &PatchTable,
    vanilla_files: &HashMap<String, String>,
) -> SimulationReport {
    simulate_and_analyze_patches_with(patch_table, vanilla_files, SimulationOptions::default())
}

/// The launch config in `mod_dir`, or the default. An empty `mod_dir` (a table never
/// staged) reads nothing, never the working directory's `qm/launch-config.json`.
pub(crate) fn launch_config_of(mod_dir: &Path) -> LaunchConfig {
    read_launch_config(mod_dir).0
}

/// `launch_config_of` with why the file could not be used. The DLL then uses the defaults and
/// loads no mods (D65).
fn read_launch_config(mod_dir: &Path) -> (LaunchConfig, Option<String>) {
    if mod_dir.as_os_str().is_empty() {
        return (LaunchConfig::default(), None);
    }
    match LaunchConfig::load_from_mod_dir(mod_dir) {
        Ok(cfg) => (cfg.unwrap_or_default(), None),
        Err(e) => (LaunchConfig::default(), Some(e)),
    }
}

/// A launch-config.json the game cannot use, as a load error: it blocks a launch, like a
/// plan error (D65).
fn launch_config_load_error(error: String) -> LoadError {
    LoadError {
        mod_id: String::new(),
        folder: "qm/launch-config.json".to_string(),
        message: format!("{error}. The game would start with no mods"),
        skipped_dependents: Vec::new(),
    }
}

/// `simulate_and_analyze_patches` with options (the compile check).
pub fn simulate_and_analyze_patches_with(
    patch_table: &PatchTable,
    vanilla_files: &HashMap<String, String>,
    options: SimulationOptions<'_>,
) -> SimulationReport {
    // Order and the rest of the derived state are built once for every file of the run.
    let prepared = patch_table.prepare();
    // The DLL prepares each file from this config before it patches it (D1).
    let loaded_launch;
    let mut launch_error = None;
    let launch = match options.launch {
        Some(l) => l,
        None => {
            let (cfg, error) = read_launch_config(&patch_table.mod_dir);
            loaded_launch = cfg;
            launch_error = error;
            &loaded_launch
        }
    };
    if let Some(error) = launch_error {
        // The DLL then loads no mods (qm-core `load_for_init`): simulate what it runs.
        let empty = PatchTable {
            mod_dir: patch_table.mod_dir.clone(),
            ..PatchTable::default()
        };
        let options = SimulationOptions { launch: Some(launch), ..options };
        let mut report = simulate_and_analyze_patches_with(&empty, vanilla_files, options);
        report.load_errors.push(launch_config_load_error(error));
        return report;
    }
    let mut files_touched = Vec::new();
    let mut conflicts = Vec::new();
    let mut total_patches = 0usize;
    let mut applied_patches = 0usize;
    let mut broken_anchors = 0usize;
    let mut traces: BTreeMap<String, Arc<FileTrace>> = BTreeMap::new();
    let mut compile_check = CompileCheck {
        ran: options.compiler.is_some(),
        files_checked: 0,
        note: options
            .compiler
            .is_none()
            .then(|| "The game's lua51.dll was not found, so patched files were not compiled.".to_string()),
    };
    let mut uncheckable: Vec<String> = Vec::new();
    let mut losses: Vec<SiteLoss> = Vec::new();
    let mut kept_anchors: Vec<(String, RungHint)> = Vec::new();
    let mut failure_groups: Vec<FailureGroup> = Vec::new();
    let mut inferred_edges: Vec<InferredEdge> = Vec::new();

    // 1. Modules: the one the DLL preloads last wins (G7); one named like a game file
    // replaces it (G1).
    let modules = crate::diagnostics_extra::module_report(patch_table, prepared.order(), vanilla_files);
    let module_registrations = modules.registrations;
    conflicts.extend(modules.conflicts);
    // Every module compiles as the game loads it (§6.2); a patch on a module never runs.
    if let Some(compiler) = options.compiler {
        conflicts.extend(crate::diagnostics_extra::module_compile_conflicts(
            compiler, patch_table, prepared.order(),
        ));
    }
    conflicts.extend(crate::diagnostics_extra::module_target_warnings(patch_table, vanilla_files));
    conflicts.extend(crate::diagnostics_extra::var_conflicts(patch_table));

    // 2. Simulate each vanilla file the targets name, once, in alphabetical order (G2).
    let (sorted_targets, target_warnings) =
        crate::diagnostics_extra::resolve_targets(patch_table, vanilla_files);
    conflicts.extend(target_warnings);

    for target in sorted_targets {
        let Some(vanilla_src) = vanilla_files.get(&target) else {
            continue;
        };

        // The text the game loads: QuarterMaster's edits first, then the patches. `game_src`
        // and `game_trace` use the game's numbering; `trace` is the file's view of it. A
        // patch that aborts the file is reported, and the file is shown as if it warned (G3).
        let (prepared_buf, traced, aborted) =
            match simulate_buffer(patch_table, &prepared, launch, &target, vanilla_src) {
                Ok((buf, traced)) => (buf, traced, false),
                Err(message) => {
                    let rerun = crate::diagnostics_extra::rerun_without_abort(
                        patch_table, launch, &target, vanilla_src,
                    );
                    let rec = rerun
                        .as_ref()
                        .and_then(|(_, t, i)| i.and_then(|i| t.records.get(i)));
                    conflicts.push(crate::diagnostics_extra::abort_conflict(&target, &message, rec));
                    match rerun {
                        Some((buf, traced, _)) => (buf, traced, true),
                        None => continue,
                    }
                }
            };
        let TracedApply { mut records, trace: game_trace, lines: line_map, .. } = traced;
        for shadow in modules.shadows.iter().filter(|s| s.file == target) {
            crate::diagnostics_extra::mark_shadowed(&mut records, shadow);
        }
        let game_src = prepared_buf.text.as_str();
        let view = FileView::new(&game_trace, &prepared_buf);
        let trace = &view.trace;
        let patched_src = trace.text.as_str();

        // Exact regions: the lines each patch's edits wrote and that survived later patches.
        for (idx, rec) in records.iter_mut().enumerate() {
            if rec.status != PatchOutcomeStatus::SkippedDisabled {
                let runs = trace.runs_of_record(idx);
                if !runs.is_empty() {
                    rec.regions = runs
                        .into_iter()
                        .map(|(start_line, end_line)| PatchRegion { start_line, end_line })
                        .collect();
                } else {
                    rec.regions = view.regions(&rec.regions);
                }
            }
        }

        if records.is_empty() {
            continue;
        }

        let mut mods_touching = Vec::new();
        for rec in &records {
            if !mods_touching.contains(&rec.mod_id) {
                mods_touching.push(rec.mod_id.clone());
            }
            match rec.status {
                PatchOutcomeStatus::Applied => {
                    total_patches += 1;
                    applied_patches += 1;
                }
                PatchOutcomeStatus::ZeroMatches => {
                    total_patches += 1;
                    broken_anchors += 1;
                }
                PatchOutcomeStatus::SkippedDisabled => {}
                _ => {
                    total_patches += 1;
                }
            }
        }

        // Attributed lines: all lines for files up to 1,200 lines, otherwise patch output plus 4
        // lines of context around it, so the UI stays fast. The full text stays in `traces`.
        let lines: Vec<&str> = patched_src.lines().collect();
        let mut include_line = vec![lines.len() <= 1200; lines.len()];
        if lines.len() > 1200 {
            for idx in 0..lines.len() {
                if trace.lines.get(idx).is_some_and(|o| o.edit().is_some()) {
                    let start = idx.saturating_sub(4);
                    let end = (idx + 4).min(lines.len().saturating_sub(1));
                    for k in start..=end {
                        include_line[k] = true;
                    }
                }
            }
        }

        let mut attributed_lines = Vec::new();
        for (idx, text) in lines.iter().enumerate() {
            if !include_line[idx] {
                continue;
            }
            let line_no = idx + 1;
            let edit = trace.edit_at(line_no);
            attributed_lines.push(AttributedCodeLine {
                line_number: line_no,
                text: (*text).to_string(),
                mod_id: edit.map(|e| e.mod_id.clone()),
                patch_id: edit.map(|e| e.patch_id.clone()),
                patch_name: edit.map(|e| e.patch_name.clone()),
            });
        }

        // 3. Places pattern patches lost to other patches, with the patch that took each.
        let file_line = |n: usize| view.line(n).unwrap_or_else(|| view.line_at_or_after(n));
        let file_losses = crate::consumer::find_losses(
            patch_table, &target, &prepared_buf, &game_trace, &records, &file_line,
        );

        // 4. Diagnose any broken anchors (`ZeroMatches`) on this file using Tier 1 & Tier 2,
        // and report places lost by patches that still matched elsewhere.
        for (idx, rec) in records.iter().enumerate() {
            let loss = file_losses
                .iter()
                .find(|l| l.record == idx && l.consumer.is_some())
                .or_else(|| file_losses.iter().find(|l| l.record == idx));
            if rec.status == PatchOutcomeStatus::ZeroMatches {
                let diag = diagnose_broken_anchor(
                    &target,
                    game_src,
                    rec,
                    &records,
                    patch_table,
                    loss,
                );
                conflicts.push(diag);
            } else if let Some((l, c)) = loss.and_then(|l| l.consumer.as_ref().map(|c| (l, c))) {
                // A patch of the same mod taking the place is the mod's own business.
                if c.mod_id != rec.mod_id {
                    conflicts.push(lost_sites_conflict(&target, rec, l, c));
                }
            }
        }
        losses.extend(file_losses);
        // Places a patch gained from another mod's copy of its lines, and patches without
        // `times` that match several places (G9).
        for g in crate::consumer::find_gains(patch_table, &prepared_buf, &game_trace, &records) {
            if let Some(c) = gained_sites_conflict(&target, &records, &g) {
                conflicts.push(c);
            }
        }
        for m in crate::consumer::find_multi_matches(patch_table, &target, &prepared_buf, &records) {
            if let Some(rec) = records.get(m.record) {
                conflicts.push(multi_match_note(&target, rec, &m));
            }
        }
        kept_anchors.extend(crate::consumer::find_kept_anchors(
            patch_table, &target, &prepared_buf, &game_trace, &records,
        ));
        // Patches that anchor on another patch's code, and the failures that follow (G10).
        let uses = crate::consumer::find_uses(patch_table, &prepared_buf, &game_trace, &records);
        inferred_edges.extend(crate::failure_groups::inferred_edges(&target, &records, &uses));
        failure_groups.extend(crate::failure_groups::failure_groups(&target, &records, &uses));
        // Two patches that change one line of the game's file (D49).
        conflicts.extend(crate::same_line::same_line_edits(
            patch_table, &target, &prepared_buf, &game_trace, &records,
        ));

        // 5. Overlaps: a patch removed (without writing back) lines another mod's patch wrote.
        // Patches that only sit next to each other, or inside another patch's span, are fine.
        let mut pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
        for (ei, edit) in trace.edits.iter().enumerate() {
            for gone in &trace.removed[ei] {
                if gone.kept || !gone.origin.is_patch_output() {
                    continue;
                }
                let Some(owner) = gone.origin.edit().map(|o| trace.edits[o].record) else {
                    continue;
                };
                if owner != edit.record && records[owner].mod_id != records[edit.record].mod_id {
                    pairs.insert((owner, edit.record));
                }
            }
        }
        for (i, j) in pairs {
            let ra = &records[i];
            let rb = &records[j];
            conflicts.push(DetectedConflict {
                id: format!("overlap:{}:{}:{}", target, ra.patch_id, rb.patch_id),
                severity: "warning".to_string(),
                kind: "overlapping_region".to_string(),
                target_file: target.clone(),
                mod_a_id: ra.mod_id.clone(),
                mod_b_id: Some(rb.mod_id.clone()),
                patch_a_id: ra.patch_id.clone(),
                patch_b_id: Some(rb.patch_id.clone()),
                description: format!(
                    "Patch '{}' (from '{}') replaces lines that patch '{}' (from '{}') wrote in '{}'.",
                    rb.patch_name, rb.mod_id, ra.patch_name, ra.mod_id, target
                ),
                suggestions: vec![ConflictResolutionSuggestion::SelectWinner {
                    patch_a_id: ra.patch_id.clone(),
                    mod_a_id: ra.mod_id.clone(),
                    patch_b_id: rb.patch_id.clone(),
                    mod_b_id: rb.mod_id.clone(),
                    explanation: "Select which patch should stay enabled on this region."
                        .to_string(),
                }],
                consumer: None,
                sites_lost: None,
                same_line: None,
            });
        }

        // 6. Compile the patched file with the game's Lua.
        // A file the game fails to load is not compiled.
        let compiled = target.ends_with(".lua") && !trace.edits.is_empty() && !aborted;
        if let (Some(compiler), true) = (options.compiler, compiled) {
            compile_check.files_checked += 1;
            let to_file = |n: usize| view.line(n);
            match qm_core::compile::check_file(compiler, &target, game_src, &game_trace, &to_file) {
                Ok(None) => {}
                Ok(Some(err)) => conflicts.push(compile_conflict(&target, &records, &err)),
                Err(_) => uncheckable.push(target.clone()),
            }
        }

        // 7. Placement: code that another mod moved, wrapped or commented out.
        if target.ends_with(".lua") && mods_touching.len() > 1 {
            let to_file = |n: usize| view.line_at_or_after(n);
            conflicts.extend(crate::placement::check_placement(
                patch_table, &target, game_src, &game_trace, &records, &to_file,
            ));
        }

        let patched_line_count = lines.len();
        let stacks = shared_stacks(
            &target, patch_table.stack_mode, &line_map, &records, &game_trace.text, &view,
            &prepared_buf,
        );
        traces.insert(target.clone(), Arc::new(view.trace));
        files_touched.push(SimulatedFileResult {
            target_file: target,
            mods_touching,
            records,
            attributed_lines,
            original_line_count: vanilla_src.lines().count(),
            patched_line_count,
            stacks,
        });
    }

    // N10: what the code means, in the order the game runs it (D69). Warnings and notes only.
    let analysis = crate::semantic::analyze(
        patch_table, prepared.order(), vanilla_files, &traces, &files_touched,
    );
    conflicts.extend(analysis.conflicts);

    SimulationReport {
        files_touched,
        conflicts,
        module_registrations,
        total_patches,
        applied_patches,
        broken_anchors,
        order_diagnostics: crate::solver::patch_cycle_diagnostics(&prepared.order().cycles),
        traces,
        plan_error: patch_table.plan_report.plan_error.clone(),
        plan_warnings: patch_table.plan_report.plan_warnings.clone(),
        load_errors: patch_table.load_errors.clone(),
        fix_bindings: patch_table.plan_report.bindings.clone(),
        overridden_rules: prepared.report().overridden_rules.clone(),
        unknown_ids: prepared.report().unknown_ids.clone(),
        suspended_rungs: prepared.report().suspended_rungs.clone(),
        invalid_rungs: prepared.report().invalid_rungs.clone(),
        overrides: patch_table.plan_report.overrides.clone(),
        losses,
        kept_anchors,
        runs_as: runs_as_verdicts(patch_table, &prepared),
        failure_groups,
        inferred_edges,
        save_fields: analysis.save_fields,
        compile_check: {
            if !uncheckable.is_empty() {
                compile_check.note = Some(format!(
                    "The game's own {} did not compile, so patches there were not checked.",
                    uncheckable.join(", ")
                ));
            }
            compile_check
        },
    }
}

/// A `sites_lost` warning: patch A still matched, but another patch took some of its places
/// in the game's file, so its code is missing there.
fn lost_sites_conflict(
    target: &str,
    rec: &PatchExecutionRecord,
    loss: &SiteLoss,
    consumer: &ConsumerInfo,
) -> DetectedConflict {
    DetectedConflict {
        id: format!("sites-lost:{}:{}", target, rec.patch_id),
        severity: "warning".to_string(),
        kind: "sites_lost".to_string(),
        target_file: target.to_string(),
        mod_a_id: rec.mod_id.clone(),
        mod_b_id: Some(consumer.mod_id.clone()),
        patch_a_id: rec.patch_id.clone(),
        patch_b_id: Some(consumer.patch_id.clone()),
        description: format!(
            "Patch '{}' (from '{}') lost {} of its {} places in '{}': '{}' (from '{}') changed line {} first.",
            rec.patch_name,
            rec.mod_id,
            loss.sites_lost,
            loss.sites_total,
            target,
            consumer.patch_id,
            consumer.mod_id,
            consumer.site_line
        ),
        suggestions: Vec::new(),
        consumer: Some(consumer.clone()),
        sites_lost: Some((loss.sites_lost, loss.sites_total)),
        same_line: None,
    }
}

/// A `sites_gained` warning: patch A matches more places than the game's file has, because
/// another mod's code has a copy of its lines, so it also adds its code there.
fn gained_sites_conflict(
    target: &str,
    records: &[PatchExecutionRecord],
    g: &crate::consumer::SiteGain,
) -> Option<DetectedConflict> {
    let rec = records.get(g.record)?;
    let src = g.sources.first().and_then(|&i| records.get(i))?;
    Some(DetectedConflict {
        id: format!("sites-gained:{}:{}", target, rec.patch_id),
        severity: "warning".to_string(),
        kind: "sites_gained".to_string(),
        target_file: target.to_string(),
        mod_a_id: rec.mod_id.clone(),
        mod_b_id: Some(src.mod_id.clone()),
        patch_a_id: rec.patch_id.clone(),
        patch_b_id: Some(src.patch_id.clone()),
        description: format!(
            "Patch '{}' (from '{}') matches {} places in '{}', but the game's file has {}: code from '{}' (from '{}') has a copy of its lines, so it also runs there.",
            rec.patch_name, rec.mod_id, g.now, target, g.vanilla, src.patch_id, src.mod_id
        ),
        suggestions: Vec::new(),
        consumer: None,
        sites_lost: None,
        same_line: None,
    })
}

/// A `multi_match` note: a patch without `times` matches several places of the game's file.
fn multi_match_note(
    target: &str,
    rec: &PatchExecutionRecord,
    m: &crate::consumer::MultiMatch,
) -> DetectedConflict {
    let functions = if m.functions == 1 {
        "1 function".to_string()
    } else {
        format!("{} functions", m.functions)
    };
    DetectedConflict {
        id: format!("multi-match:{}:{}", target, rec.patch_id),
        severity: "info".to_string(),
        kind: "multi_match".to_string(),
        target_file: target.to_string(),
        mod_a_id: rec.mod_id.clone(),
        mod_b_id: None,
        patch_a_id: rec.patch_id.clone(),
        patch_b_id: None,
        description: format!(
            "Patch '{}' (from '{}') has no `times` and matches {} places in {} of '{}'. It runs at each of them.",
            rec.patch_name, rec.mod_id, m.sites, functions, target
        ),
        suggestions: Vec::new(),
        consumer: None,
        sites_lost: None,
        same_line: None,
    }
}

/// A `compile_error` conflict: patch A is the first patch after which the file stops compiling;
/// patch B, when different, wrote the line the error points at.
fn compile_conflict(
    target: &str,
    records: &[PatchExecutionRecord],
    err: &qm_core::compile::FileCompileError,
) -> DetectedConflict {
    let a = err.breaking_record.or(err.line_record).and_then(|i| records.get(i));
    let b = err
        .line_record
        .filter(|&i| Some(i) != err.breaking_record)
        .and_then(|i| records.get(i))
        .filter(|rb| a.is_none_or(|ra| ra.mod_id != rb.mod_id || ra.patch_id != rb.patch_id));
    let place = match (err.line, err.opened_at) {
        (Some(l), _) => format!("line {l}"),
        (None, Some(o)) => format!("the end of the file (block opened at line {o})"),
        (None, None) => "the end of the file".to_string(),
    };
    let who = match a {
        Some(ra) => format!(" after patch '{}' (from '{}')", ra.patch_name, ra.mod_id),
        None => String::new(),
    };
    DetectedConflict {
        id: format!("compile:{}:{}", target, a.map(|r| r.patch_id.as_str()).unwrap_or("?")),
        severity: "error".to_string(),
        kind: "compile_error".to_string(),
        target_file: target.to_string(),
        mod_a_id: a.map(|r| r.mod_id.clone()).unwrap_or_default(),
        mod_b_id: b.map(|r| r.mod_id.clone()),
        patch_a_id: a.map(|r| r.patch_id.clone()).unwrap_or_default(),
        patch_b_id: b.map(|r| r.patch_id.clone()),
        description: format!(
            "'{target}' does not compile{who}: {place}: {}. The game would stop with a Lua error when it loads this file.",
            err.message
        ),
        suggestions: Vec::new(),
        consumer: None,
        sites_lost: None,
        same_line: None,
    }
}

/// `patch` alone on `target`, with the full table's mods. The clone keeps its `PatchExtra`, so
/// its rungs stay unresolved without their B and it falls back to its own anchor.
fn solo_table(full: &PatchTable, patch: &Patch, path: &Path, target: &str) -> PatchTable {
    let mut t = PatchTable {
        mods: full.mods.clone(),
        ..PatchTable::default()
    };
    t.targets.insert(target.to_string());
    t.patches.push((patch.clone(), 0, path.to_path_buf()));
    t
}

/// The full table with every patch of `mod_id` at `prio` (the Tier 1 swap test).
fn swapped_table(full: &PatchTable, mod_id: &str, prio: Priority) -> PatchTable {
    let mut t = full.clone();
    for (p, p_prio, _) in &mut t.patches {
        if p.mod_id() == Some(mod_id) {
            *p_prio = prio;
        }
    }
    t
}

/// Runs Tier 1 (Automatic Order-Swap Test) and Tier 2 (1-Click Re-Anchoring Glue Patch Generator)
/// when `failed_rec` had 0 matches on `target`.
fn diagnose_broken_anchor(
    target: &str,
    vanilla_src: &str,
    failed_rec: &PatchExecutionRecord,
    file_records: &[PatchExecutionRecord],
    full_table: &PatchTable,
    loss: Option<&SiteLoss>,
) -> DetectedConflict {
    let mut suggestions = Vec::new();
    let mut culprit_mod_id: Option<String> = None;
    let mut culprit_patch_id: Option<String> = None;
    // The patch that took the line, when the simulation saw it happen (§9.1).
    let consumer = loss.and_then(|l| l.consumer.as_ref());
    let consumer_rec = loss
        .and_then(|l| l.consumer_record)
        .and_then(|i| file_records.get(i))
        .filter(|r| r.mod_id != failed_rec.mod_id);

    // Find the failing Patch object
    let failing_tuple = full_table
        .patches
        .iter()
        .find(|(p, _, _)| p.id() == Some(failed_rec.patch_id.as_str()));

    if let Some((failing_patch, _failing_prio, failing_path)) = failing_tuple {
        // First test: Does `failing_patch` match against unmodified `vanilla_src` on its own?
        let solo_table = solo_table(full_table, failing_patch, failing_path, target);

        // Its places in the file when it runs alone.
        let solo_sites = solo_table
            .apply_patches_offline(target, vanilla_src)
            .ok()
            .and_then(|(_, _, recs)| recs.first().map(|r| r.matched_count))
            .unwrap_or(0);

        if solo_sites > 0 {
            // Another mod's earlier patch on `target` clobbered the vanilla anchor!
            // Find which earlier mod/patch broke it and test Tier 1 (Order Swap):
            let mut earlier_mods: Vec<String> = file_records
                .iter()
                .filter(|r| r.priority <= failed_rec.priority && r.mod_id != failed_rec.mod_id)
                .map(|r| r.mod_id.clone())
                .collect();
            // The consumer's mod is tried first.
            if let Some(c) = consumer_rec {
                earlier_mods.retain(|m| *m != c.mod_id);
                earlier_mods.insert(0, c.mod_id.clone());
            }

            for cand_mod in earlier_mods {
                let cand_min_prio = file_records
                    .iter()
                    .filter(|r| r.mod_id == cand_mod)
                    .map(|r| r.priority)
                    .min()
                    .unwrap_or(10);
                let test_prio = cand_min_prio - 5;

                // Build a test table where `failed_rec.mod_id` runs at `test_prio` (before `cand_mod`)
                let swapped_table = swapped_table(full_table, &failed_rec.mod_id, test_prio);

                if let Ok((_, _, swapped_recs)) =
                    swapped_table.apply_patches_offline(target, vanilla_src)
                {
                    // A patch the profile turned off is not a failure of the swap (G5).
                    let all_succeeded = swapped_recs
                        .iter()
                        .filter(|r| r.status != PatchOutcomeStatus::SkippedDisabled)
                        .all(|r| r.status == PatchOutcomeStatus::Applied);
                    if all_succeeded {
                        culprit_mod_id = Some(cand_mod.clone());
                        culprit_patch_id = file_records
                            .iter()
                            .find(|r| r.mod_id == cand_mod)
                            .map(|r| r.patch_id.clone());
                        suggestions.push(ConflictResolutionSuggestion::SwapModOrder {
                            move_mod_id: failed_rec.mod_id.clone(),
                            before_mod_id: cand_mod.clone(),
                            move_patch_id: failed_rec.patch_id.clone(),
                            suggested_priority: test_prio,
                            explanation: format!(
                                "Load '{}' (priority {}) before '{}' (priority {}) so both patches match '{}' cleanly without any code changes.",
                                failed_rec.mod_id, test_prio, cand_mod, cand_min_prio, target
                            ),
                        });
                        break;
                    }
                }
            }

            // Tier 2 (D63). An `at` patch is offered edits mode. A before/after pattern patch
            // gets a copy on the code of the patch that replaced its line, only when the
            // simulation saw that patch take it; regex patches get neither.
            if let Patch::Pattern(pat_patch) = failing_patch {
                if let Some(earlier_rec) = consumer_rec.or_else(|| {
                    file_records
                        .iter()
                        .find(|r| r.priority <= failed_rec.priority && r.mod_id != failed_rec.mod_id)
                }) {
                    if culprit_mod_id.is_none() {
                        culprit_mod_id = Some(earlier_rec.mod_id.clone());
                        culprit_patch_id = Some(earlier_rec.patch_id.clone());
                    }

                    // A before/after patch whose lines another patch split finds its place
                    // again as separate edits (§4.8).
                    let split = loss.is_some_and(|l| l.apply_as);
                    // An `at` already running as edits, or whose lines do not pair, gains
                    // nothing from it (as `offers_apply_as`).
                    let is_at = pat_patch.position == InsertPosition::At;
                    let offer = split
                        || (is_at && !pat_patch.split_windows() && pat_patch.normalized().is_ok());
                    if offer {
                        suggestions.push(ConflictResolutionSuggestion::ApplyAs {
                            patch_id: failed_rec.patch_id.clone(),
                            mod_id: failed_rec.mod_id.clone(),
                            explanation: if is_at {
                                format!(
                                    "Run '{}' as separate edits instead of one replacement.",
                                    failed_rec.patch_name
                                )
                            } else {
                                format!(
                                    "Run '{}' as separate edits, so it finds its lines around the other mod's code.",
                                    failed_rec.patch_name
                                )
                            },
                        });
                    } else if is_at {
                        // No offer: an `at` gets no copy.
                    } else if let Some(s) = consumer
                        .zip(consumer_rec)
                        .and_then(|(c, rec)| reanchored_glue(target, failed_rec, pat_patch, c, rec, full_table, solo_sites))
                    {
                        suggestions.push(s);
                    }
                }
            }
        }
    }

    if let Some(c) = consumer.filter(|c| c.mod_id != failed_rec.mod_id) {
        culprit_mod_id = Some(c.mod_id.clone());
        culprit_patch_id = Some(c.patch_id.clone());
    }

    let kind = if !suggestions.is_empty() {
        "broken_anchor_order".to_string()
    } else {
        "broken_anchor_replaced".to_string()
    };

    let description = if let Some(ref c_mod) = culprit_mod_id {
        format!(
            "Patch '{}' (from '{}') failed to match on '{}' because earlier mod '{}' modified the vanilla anchor.",
            failed_rec.patch_name, failed_rec.mod_id, target, c_mod
        )
    } else {
        format!(
            "Patch '{}' (from '{}') resulted in 0 matches on '{}'.",
            failed_rec.patch_name, failed_rec.mod_id, target
        )
    };

    DetectedConflict {
        id: format!("broken-anchor:{}:{}", target, failed_rec.patch_id),
        severity: "error".to_string(),
        kind,
        target_file: target.to_string(),
        mod_a_id: failed_rec.mod_id.clone(),
        mod_b_id: culprit_mod_id,
        patch_a_id: failed_rec.patch_id.clone(),
        patch_b_id: culprit_patch_id,
        description,
        suggestions,
        consumer: consumer.cloned(),
        sites_lost: loss.map(|l| (l.sites_lost, l.sites_total)),
        same_line: None,
    }
}

/// The Tier 2 copy of the before/after pattern patch `victim` (D63): its payload on the first
/// (Before) or last (After) code line of what `consumer` wrote in its line's place, on its own
/// side, with as many places as it has alone. None when the consumer did not replace the line
/// with code of its own, or is not a pattern patch.
fn reanchored_glue(
    target: &str,
    failed_rec: &PatchExecutionRecord,
    victim: &PatternPatch,
    consumer: &ConsumerInfo,
    consumer_rec: &PatchExecutionRecord,
    full_table: &PatchTable,
    sites: usize,
) -> Option<ConflictResolutionSuggestion> {
    if !matches!(consumer.kind, ConsumerKind::Rewritten1to1 | ConsumerKind::ReplacedByBlock) {
        return None;
    }
    let side = match victim.position {
        InsertPosition::Before => "before",
        InsertPosition::After => "after",
        InsertPosition::At => return None,
    };
    let Some((Patch::Pattern(b), _, _)) = full_table
        .patches
        .iter()
        .find(|(p, _, _)| p.id() == Some(consumer_rec.patch_id.as_str()))
    else {
        return None;
    };
    let mut code = b
        .payload
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("--"));
    let anchor = if victim.position == InsertPosition::Before {
        code.next()
    } else {
        code.next_back()
    }?;
    let id = format!("glue:{}", failed_rec.patch_id);
    let glue = GluePatchEntry {
        id: id.clone(),
        name: format!(
            "Re-anchored '{}' {side} '{}'",
            failed_rec.patch_name, consumer_rec.mod_id
        ),
        replaces_patch_id: Some(failed_rec.patch_id.clone()),
        priority: failed_rec.priority,
        patch: Patch::Pattern(PatternPatch {
            pattern: anchor.to_string(),
            position: victim.position,
            target: Target::Single(target.to_string()),
            payload: victim.payload.clone(),
            match_indent: victim.match_indent,
            times: Some(sites),
            overwrite: false,
            id: Some(id),
            name: Some(format!("Glue: {} + {}", consumer_rec.mod_id, failed_rec.mod_id)),
            priority: Some(failed_rec.priority),
            description: None,
            requires: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            on_failure: OnFailurePolicy::Warn,
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            apply_as: None,
            mod_id: Some("qm_glue".to_string()),
            disabled: false,
            qm: Default::default(),
        }),
        base_mod_id: Some(failed_rec.mod_id.clone()),
        // A new copy runs as its base mod, and its anchor line matches only its own text
        // (D58, D59).
        run_as: Some(RunAs::Base),
        exact: true,
    };
    Some(ConflictResolutionSuggestion::GenerateGluePatch {
        glue_patch: glue,
        explanation: format!(
            "Add a compatibility patch that puts '{}' {side} the code '{}' wrote in its place (`{}`).",
            failed_rec.patch_name, consumer_rec.mod_id, anchor
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_table_never_staged_reads_no_launch_config() {
        assert_eq!(launch_config_of(Path::new("")).save_identity_override, None);
        let dir = tempfile::tempdir().unwrap();
        let cfg = LaunchConfig {
            save_identity_override: Some("mnb_x".to_string()),
            ..LaunchConfig::default()
        };
        cfg.save_to_mod_dir(dir.path()).unwrap();
        assert_eq!(
            launch_config_of(dir.path()).save_identity_override.as_deref(),
            Some("mnb_x")
        );
    }

    #[test]
    fn a_broken_launch_config_is_a_blocking_load_error() {
        let dir = tempfile::tempdir().unwrap();
        let table = PatchTable {
            mod_dir: dir.path().to_path_buf(),
            ..PatchTable::default()
        };
        let vanilla = HashMap::from([("main.lua".to_string(), "local x = 1\n".to_string())]);
        assert!(simulate_and_analyze_patches(&table, &vanilla).load_errors.is_empty());

        std::fs::create_dir_all(dir.path().join("qm")).unwrap();
        std::fs::write(dir.path().join("qm").join("launch-config.json"), "{\"vanilla_mode\":").unwrap();
        let report = simulate_and_analyze_patches(&table, &vanilla);
        assert_eq!(report.load_errors.len(), 1, "{:?}", report.load_errors);
        let e = &report.load_errors[0];
        assert_eq!(e.folder, "qm/launch-config.json");
        assert!(e.mod_id.is_empty());
        assert!(e.message.starts_with("launch-config.json is not valid: "), "{}", e.message);
        assert!(e.message.ends_with("The game would start with no mods"), "{}", e.message);
        // The builtins are still prepared from the defaults.
        assert_eq!(launch_config_of(dir.path()).save_identity_override, None);

        // Like the DLL, the simulation then applies no mod patch.
        let mut table = PatchTable {
            mod_dir: dir.path().to_path_buf(),
            ..PatchTable::default()
        };
        table.targets.insert("main.lua".to_string());
        table.patches.push((
            insert_after_anchor("a", "a:p", &[]),
            50,
            std::path::PathBuf::from("a/qm.toml"),
        ));
        let vanilla = HashMap::from([("main.lua".to_string(), "-- anchor\n".to_string())]);
        let report = simulate_and_analyze_patches(&table, &vanilla);
        assert_eq!(report.load_errors.len(), 1, "{:?}", report.load_errors);
        assert_eq!(report.total_patches, 0);
        assert!(
            report.files_touched.iter().all(|f| f.records.is_empty()),
            "{:?}",
            report.files_touched
        );
    }

    fn insert_after_anchor(mod_id: &str, id: &str, before: &[&str]) -> Patch {
        Patch::Pattern(PatternPatch {
            pattern: "-- anchor".to_string(),
            position: InsertPosition::After,
            target: Target::Single("main.lua".to_string()),
            payload: format!("-- {mod_id}"),
            match_indent: false,
            times: Some(1),
            overwrite: false,
            id: Some(id.to_string()),
            name: Some(id.to_string()),
            description: None,
            requires: Vec::new(),
            priority: None,
            before: before.iter().map(|s| s.to_string()).collect(),
            after: Vec::new(),
            on_failure: OnFailurePolicy::Warn,
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            apply_as: None,
            mod_id: Some(mod_id.to_string()),
            disabled: false,
            qm: Default::default(),
        })
    }

    fn pattern(mod_id: &str, id: &str, anchor: &str, position: InsertPosition, payload: &str) -> Patch {
        Patch::Pattern(PatternPatch {
            pattern: anchor.to_string(),
            position,
            target: Target::Single("main.lua".to_string()),
            payload: payload.to_string(),
            match_indent: false,
            times: Some(1),
            overwrite: false,
            id: Some(id.to_string()),
            name: Some(id.to_string()),
            description: None,
            requires: Vec::new(),
            priority: None,
            before: Vec::new(),
            after: Vec::new(),
            on_failure: OnFailurePolicy::Warn,
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            apply_as: None,
            mod_id: Some(mod_id.to_string()),
            disabled: false,
            qm: Default::default(),
        })
    }

    fn simulate(patches: Vec<(Patch, Priority)>, src: &str) -> SimulationReport {
        let mut table = PatchTable::default();
        table.targets.insert("main.lua".to_string());
        for (i, (p, prio)) in patches.into_iter().enumerate() {
            table.patches.push((p, prio, PathBuf::from(format!("M{i}/qm.toml"))));
        }
        let vanilla = HashMap::from([("main.lua".to_string(), src.to_string())]);
        simulate_and_analyze_patches(&table, &vanilla)
    }

    const HALT_SRC: &str = "function halt()
  stop()
end
";

    /// A patch anchored inside another mod's replacement (TacticalOverhaul #6 inside GCore's
    /// Halt block) only adds lines: not an overlap.
    #[test]
    fn inserting_inside_another_mods_output_is_not_an_overlap() {
        let r = simulate(
            vec![
                (pattern("gcore", "gcore:halt", "  stop()", InsertPosition::At, "  stop()
  clear_path()"), 10),
                (pattern("to", "to:6", "  clear_path()", InsertPosition::After, "  log_halt()"), 20),
            ],
            HALT_SRC,
        );
        let f = &r.files_touched[0];
        assert_eq!(r.traces["main.lua"].text, "function halt()
  stop()
  clear_path()
  log_halt()
end
");
        assert!(r.conflicts.iter().all(|c| c.kind != "overlapping_region"), "{:?}", r.conflicts);
        // Exact regions and attribution: gcore wrote lines 2-3 (2 kept), to wrote line 4.
        let gcore = f.records.iter().find(|x| x.patch_id == "gcore:halt").unwrap();
        assert_eq!(gcore.regions, vec![PatchRegion { start_line: 2, end_line: 3 }]);
        let to = f.records.iter().find(|x| x.patch_id == "to:6").unwrap();
        assert_eq!(to.regions, vec![PatchRegion { start_line: 4, end_line: 4 }]);
        assert_eq!(f.attributed_lines[3].mod_id.as_deref(), Some("to"));
        assert_eq!(f.attributed_lines[0].mod_id, None);
    }

    /// Replacing a line another mod wrote is an overlap, reported once.
    #[test]
    fn replacing_another_mods_line_is_an_overlap() {
        let r = simulate(
            vec![
                (pattern("a", "a:1", "  stop()", InsertPosition::At, "  stop_now()"), 10),
                (pattern("b", "b:1", "  stop_now()", InsertPosition::At, "  stop_later()"), 20),
            ],
            HALT_SRC,
        );
        let overlaps: Vec<_> = r.conflicts.iter().filter(|c| c.kind == "overlapping_region").collect();
        assert_eq!(overlaps.len(), 1, "{:?}", r.conflicts);
        assert_eq!(overlaps[0].patch_a_id, "a:1");
        assert_eq!(overlaps[0].patch_b_id.as_deref(), Some("b:1"));
    }

    /// Another mod wraps the line a patch is anchored on in an `if`: the patch's code now runs
    /// only when that condition holds.
    #[test]
    fn code_wrapped_by_another_mod_is_a_placement_warning() {
        let src = "function update()
  move()
  draw()
end
";
        let r = simulate(
            vec![
                (pattern("wrap", "wrap:1", "  move()", InsertPosition::At, "  if ready then
  move()"), 10),
                (pattern("wrap", "wrap:2", "  draw()", InsertPosition::At, "  draw()
  end"), 11),
                (pattern("log", "log:1", "  move()", InsertPosition::After, "  log_move()"), 20),
            ],
            src,
        );
        let p: Vec<_> = r.conflicts.iter().filter(|c| c.kind == "placement_changed").collect();
        assert_eq!(p.len(), 1, "{:?}", r.conflicts);
        assert_eq!(p[0].patch_a_id, "log:1");
        assert_eq!(p[0].mod_b_id.as_deref(), Some("wrap"));
        assert!(p[0].description.contains("deeper inside a block"), "{}", p[0].description);
    }

    /// Code added next to another mod's code at the same depth is fine.
    #[test]
    fn neighbours_at_the_same_depth_are_not_flagged() {
        let src = "function update()
  move()
end
";
        let r = simulate(
            vec![
                (pattern("a", "a:1", "  move()", InsertPosition::After, "  a()"), 10),
                (pattern("b", "b:1", "  move()", InsertPosition::After, "  b()"), 20),
            ],
            src,
        );
        assert!(r.conflicts.iter().all(|c| !c.kind.starts_with("placement") && c.kind != "anchor_in_comment"), "{:?}", r.conflicts);
    }

    /// Another mod comments out a block; code added inside it never runs.
    #[test]
    fn code_inside_another_mods_comment_is_flagged() {
        let src = "function update()
  move()
  draw()
end
";
        let r = simulate(
            vec![
                (pattern("off", "off:1", "  move()", InsertPosition::At, "  --[[
  move()"), 10),
                (pattern("off", "off:2", "  draw()", InsertPosition::At, "  draw()
  ]]"), 11),
                (pattern("log", "log:1", "  move()", InsertPosition::After, "  log_move()"), 20),
            ],
            src,
        );
        assert!(r.conflicts.iter().any(|c| c.kind == "placement_in_comment" && c.patch_a_id == "log:1"), "{:?}", r.conflicts);
    }

    /// Large files keep their full text in the trace even though attributed lines are trimmed.
    #[test]
    fn large_files_keep_the_full_text() {
        let src: String = (0..2000).map(|i| format!("line{i}
")).collect();
        let r = simulate(vec![(pattern("a", "a:1", "line1000", InsertPosition::After, "added"), 10)], &src);
        assert!(r.files_touched[0].attributed_lines.len() < 20);
        assert_eq!(r.traces["main.lua"].text.lines().count(), 2001);
    }

    #[test]
    fn simulation_uses_engine_order_and_reports_patch_cycles() {
        let mut table = PatchTable::default();
        table.targets.insert("main.lua".to_string());
        // Priorities say a, b, c; the profile rule moves c first. a and b also demand to run
        // before each other, a cycle the engine must break and report.
        table.patches.push((insert_after_anchor("ma", "a", &["mb:b"]), 10, PathBuf::from("A/qm.toml")));
        table.patches.push((insert_after_anchor("mb", "b", &["ma:a"]), 20, PathBuf::from("B/qm.toml")));
        table.patches.push((insert_after_anchor("mc", "c", &[]), 30, PathBuf::from("C/qm.toml")));
        table.order_rules = vec![qm_core::patch::order::OrderRule {
            patch: "mc:c".into(),
            before: vec!["ma:*".into(), "mb:*".into()],
            after: vec![],
        }];
        let vanilla = HashMap::from([("main.lua".to_string(), "-- anchor
".to_string())]);

        let report = simulate_and_analyze_patches(&table, &vanilla);
        let ids: Vec<String> = table.ordered().order.iter().map(|&i| table.patches[i].0.id().unwrap().to_string()).collect();
        let file = &report.files_touched[0];
        let applied: Vec<&str> = file.records.iter().map(|r| r.patch_id.as_str()).collect();
        assert_eq!(applied, ids);
        assert_eq!(ids[0], "c");
        assert_eq!(report.order_diagnostics.len(), 1);
        let d = &report.order_diagnostics[0];
        assert_eq!(d.kind, "patch_cycle");
        assert_eq!(d.severity, qm_schema::DiagnosticSeverity::Warning);
        assert!(d.message.contains("ma:a, then mb:b"), "{}", d.message);
    }

    #[test]
    fn tier1_and_tier2_resolve_h_key_halt_anchor_conflict() {
        // Reproduces the exact H-key halt conflict between TacticalOverhaul (prio 20) and GCore (prio 100)
        let vanilla_main = r#"
      if key == "h" then
        player.deployedTroops[i].targetx = player.deployedTroops[i].x
        player.deployedTroops[i].targety = player.deployedTroops[i].y
        if(spamTimer <= 0) then
          newTextCue("Holding!", player.deployedTroops[i].x, player.deployedTroops[i].y)
        end
      end
"#;

        let to_patch = Patch::Pattern(PatternPatch {
            pattern: "player.deployedTroops[i].targety = player.deployedTroops[i].y".to_string(),
            position: InsertPosition::After,
            target: Target::Single("main.lua".to_string()),
            payload: "        player.deployedTroops[i].manualOrderTimer = 180".to_string(),
            match_indent: false,
            times: Some(1),
            overwrite: false,
            id: Some("tactical_overhaul:main_lua:halt".to_string()),
            name: Some("H-key manualOrderTimer".to_string()),
            description: None,
            requires: Vec::new(),
            priority: Some(20),
            before: Vec::new(),
            after: Vec::new(),
            on_failure: OnFailurePolicy::Warn,
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            apply_as: None,
            mod_id: Some("tactical_overhaul".to_string()),
            disabled: false,
            qm: Default::default(),
        });

        let gcore_patch = Patch::Pattern(PatternPatch {
            pattern: "player.deployedTroops[i].targetx = player.deployedTroops[i].x\nplayer.deployedTroops[i].targety = player.deployedTroops[i].y\nif(spamTimer <= 0) then".to_string(),
            position: InsertPosition::At,
            target: Target::Single("main.lua".to_string()),
            payload: "player.deployedTroops[i].targetx = player.deployedTroops[i].x\nplayer.deployedTroops[i].targety = player.deployedTroops[i].y\npathfinding.clearUnitWaypoints(player.deployedTroops[i])\nif(spamTimer <= 0) then".to_string(),
            match_indent: true,
            times: Some(1),
            overwrite: false,
            id: Some("gcore:main_lua:halt".to_string()),
            name: Some("H-key clearUnitWaypoints".to_string()),
            description: None,
            requires: Vec::new(),
            priority: Some(100),
            before: Vec::new(),
            after: Vec::new(),
            on_failure: OnFailurePolicy::Warn,
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            apply_as: None,
            mod_id: Some("gcore".to_string()),
            disabled: false,
            qm: Default::default(),
        });

        let mut table = PatchTable::default();
        table.targets.insert("main.lua".to_string());
        table.patches.push((to_patch, 20, PathBuf::from("TacticalOverhaul/qm.toml")));
        table.patches.push((gcore_patch, 100, PathBuf::from("GCore/qm.toml")));

        let mut vanilla_files = HashMap::new();
        vanilla_files.insert("main.lua".to_string(), vanilla_main.to_string());

        let report = simulate_and_analyze_patches(&table, &vanilla_files);
        assert_eq!(report.broken_anchors, 1);
        assert_eq!(report.conflicts.len(), 1);

        let conflict = &report.conflicts[0];
        assert_eq!(conflict.mod_a_id, "gcore");
        assert_eq!(conflict.mod_b_id.as_deref(), Some("tactical_overhaul"));

        // Tier 1 (SwapModOrder) is offered. GCore's patch is an `at`, so Tier 2 offers running
        // it as separate edits instead of a copy (D63).
        assert!(conflict.suggestions.iter().any(|s| matches!(
            s,
            ConflictResolutionSuggestion::SwapModOrder {
                move_mod_id,
                before_mod_id,
                ..
            } if move_mod_id == "gcore" && before_mod_id == "tactical_overhaul"
        )));
        assert!(
            !conflict.suggestions.iter().any(|s| matches!(s, ConflictResolutionSuggestion::GenerateGluePatch { .. })),
            "{:?}",
            conflict.suggestions
        );
        let apply_as = conflict.suggestions.iter().find_map(|s| match s {
            ConflictResolutionSuggestion::ApplyAs { patch_id, mod_id, explanation } => Some((patch_id, mod_id, explanation)),
            _ => None,
        });
        let (patch_id, mod_id, explanation) = apply_as.expect("an ApplyAs suggestion");
        assert_eq!((patch_id.as_str(), mod_id.as_str()), ("gcore:main_lua:halt", "gcore"));
        assert_eq!(explanation, "Run 'H-key clearUnitWaypoints' as separate edits instead of one replacement.");
        assert_eq!(conflict.kind, "broken_anchor_order");
    }

    /// `function f()` with two `x()` lines.
    const TWO_X: &str = "function f()\n  x()\n  y()\n  x()\nend\n";

    /// Replaces every `x()` line with a block that starts and ends with a comment.
    fn block_b() -> Patch {
        let mut b = pattern("b", "b:block", "  x()", InsertPosition::At, "  -- b starts\n  b_first()\n  b_last()\n  -- b ends");
        if let Patch::Pattern(x) = &mut b {
            x.times = None;
        }
        b
    }

    /// B replaces every `x()` line with its block; A, after it, is anchored on `x()`.
    fn replaced_x(position: InsertPosition) -> SimulationReport {
        let mut a = pattern("a", "a:add", "  x()", position, "  a()");
        if let Patch::Pattern(x) = &mut a {
            x.times = None;
        }
        simulate(vec![(block_b(), 10), (a, 20)], TWO_X)
    }

    fn glue_of(r: &SimulationReport, patch: &str) -> Option<GluePatchEntry> {
        r.conflicts
            .iter()
            .filter(|c| c.patch_a_id == patch)
            .flat_map(|c| &c.suggestions)
            .find_map(|s| match s {
                ConflictResolutionSuggestion::GenerateGluePatch { glue_patch, .. } => Some(glue_patch.clone()),
                _ => None,
            })
    }

    /// D63: a before/after patch whose line a known patch replaced gets a copy on that patch's
    /// code, on its own side, with its own place count. A Before victim goes on B's first code
    /// line, an After victim on the last, so neither lands inside B's block.
    #[test]
    fn a_glue_follows_its_victims_side_and_place_count() {
        for (position, anchor, want) in [
            (
                InsertPosition::Before,
                "b_first()",
                "function f()\n  -- b starts\n  a()\n  b_first()\n  b_last()\n  -- b ends\n  y()\n  -- b starts\n  a()\n  b_first()\n  b_last()\n  -- b ends\nend\n",
            ),
            (
                InsertPosition::After,
                "b_last()",
                "function f()\n  -- b starts\n  b_first()\n  b_last()\n  a()\n  -- b ends\n  y()\n  -- b starts\n  b_first()\n  b_last()\n  a()\n  -- b ends\nend\n",
            ),
        ] {
            let r = replaced_x(position);
            let c = r.conflicts.iter().find(|c| c.patch_a_id == "a:add").expect("a conflict for a:add");
            assert_eq!(c.consumer.as_ref().map(|x| x.kind), Some(ConsumerKind::ReplacedByBlock));
            let glue = glue_of(&r, "a:add").unwrap_or_else(|| panic!("no glue: {:?}", c.suggestions));
            assert_eq!(glue.replaces_patch_id.as_deref(), Some("a:add"));
            // A new copy: it runs as A's mod and its anchor matches only its text (D58, D59).
            assert!(glue.exact);
            assert_eq!(glue.run_as, Some(RunAs::Base));
            let Patch::Pattern(g) = &glue.patch else { panic!("not a pattern glue") };
            assert_eq!(g.pattern, anchor);
            assert_eq!(g.position, position);
            // Both places, from the run of A alone.
            assert_eq!(g.times, Some(2));
            assert_eq!(g.payload, "  a()");

            // The copy puts A's line next to B's block at both places.
            let with = simulate(vec![(block_b(), 10), (glue.patch.clone(), 20)], TWO_X);
            assert_eq!(with.traces["main.lua"].text, want, "{position:?}");
            assert!(with.files_touched[0].records.iter().all(|x| x.status == PatchOutcomeStatus::Applied));
        }
    }

    /// No copy when no patch of another mod was seen taking the line: the first earlier record
    /// of another mod (here C's unrelated line) is not a culprit to anchor on (D63).
    #[test]
    fn no_glue_on_the_first_earlier_record_alone() {
        let r = simulate(
            vec![
                (pattern("c", "c:other", "  y()", InsertPosition::After, "  c()"), 5),
                (pattern("a", "a:replace", "  x()", InsertPosition::At, "  z()"), 15),
                (pattern("a", "a:add", "  x()", InsertPosition::After, "  a()"), 20),
            ],
            "function f()\n  x()\n  y()\nend\n",
        );
        let c = r.conflicts.iter().find(|c| c.patch_a_id == "a:add").expect("a conflict for a:add");
        assert!(glue_of(&r, "a:add").is_none(), "{:?}", c.suggestions);
    }

    /// A regex patch never gets a copy or the edits-mode offer.
    #[test]
    fn a_regex_victim_gets_no_glue() {
        let regex: qm_core::patch::regex::RegexPatch = toml::from_str(
            "pattern = 'halt_x\\(\\)'\nposition = 'after'\ntarget = 'main.lua'\npayload = '\\n  r()'\nid = 'a:re'\nmod_id = 'a'\n",
        )
        .unwrap();
        let r = simulate(
            vec![
                (pattern("b", "b:block", "  halt_x()", InsertPosition::At, "  b_first()\n  b_last()"), 10),
                (Patch::Regex(regex), 20),
            ],
            "function f()\n  halt_x()\nend\n",
        );
        let c = r.conflicts.iter().find(|c| c.patch_a_id == "a:re").expect("a conflict for a:re");
        assert!(
            c.suggestions.iter().all(|s| !matches!(
                s,
                ConflictResolutionSuggestion::GenerateGluePatch { .. } | ConflictResolutionSuggestion::ApplyAs { .. }
            )),
            "{:?}",
            c.suggestions
        );
    }

    /// G5: a patch the profile turned off on the same file does not hide Tier 1.
    #[test]
    fn tier1_is_offered_with_a_disabled_patch_in_the_file() {
        let mut off = pattern("d", "d:off", "  y()", InsertPosition::After, "  d()");
        if let Patch::Pattern(x) = &mut off {
            x.disabled = true;
        }
        let r = simulate(
            vec![
                (pattern("b", "b:line", "  x()", InsertPosition::At, "  b()"), 10),
                (pattern("a", "a:add", "  x()", InsertPosition::After, "  a()"), 20),
                (off, 30),
            ],
            "function f()\n  x()\n  y()\nend\n",
        );
        let f = &r.files_touched[0];
        assert!(f.records.iter().any(|x| x.patch_id == "d:off" && x.status == PatchOutcomeStatus::SkippedDisabled));
        let c = r.conflicts.iter().find(|c| c.patch_a_id == "a:add").expect("a conflict for a:add");
        assert!(
            c.suggestions.iter().any(|s| matches!(
                s,
                ConflictResolutionSuggestion::SwapModOrder { move_mod_id, before_mod_id, .. }
                    if move_mod_id == "a" && before_mod_id == "b"
            )),
            "{:?}",
            c.suggestions
        );
    }

    #[test]
    fn derived_tables_keep_extras() {
        use qm_core::patch::anchor::{AnchorSource, Rung};
        use qm_core::patch::plan::ModMeta;
        let mut a = insert_after_anchor("a", "a:one", &[]);
        a.extra_mut().rungs.push((
            Rung {
                within: Some("b:two".to_string()),
                ..Rung::default()
            },
            AnchorSource::Profile {
                fix_id: "f1".to_string(),
            },
        ));
        a.extra_mut().fix_ids.push("f1".to_string());
        let extra = a.extra().clone();
        let mut full = PatchTable::default();
        full.targets.insert("main.lua".to_string());
        full.patches.push((a.clone(), 10, PathBuf::from("A/qm.toml")));
        full.patches
            .push((insert_after_anchor("b", "b:two", &[]), 20, PathBuf::from("B/qm.toml")));
        full.mods = vec![
            ModMeta {
                id: "a".to_string(),
                ..ModMeta::default()
            },
            ModMeta {
                id: "b".to_string(),
                loader_index: 1,
                ..ModMeta::default()
            },
        ];
        full.stack_mode = qm_core::handshake::StackMode::Dependencies;

        let solo = solo_table(&full, &a, std::path::Path::new("A/qm.toml"), "main.lua");
        assert_eq!(solo.patches[0].0.extra(), &extra);
        assert_eq!(solo.mods, full.mods);

        let swapped = swapped_table(&full, "a", 5);
        assert_eq!(swapped.patches[0].0.extra(), &extra);
        assert_eq!(swapped.patches[0].1, 5);
        assert_eq!(swapped.patches[1].1, 20);
        assert_eq!(swapped.mods, full.mods);
        assert_eq!(swapped.stack_mode, full.stack_mode);

        let alone = crate::placement::mod_alone(&full, "a", "main.lua");
        assert_eq!(alone.patches.len(), 1);
        assert_eq!(alone.patches[0].0.extra(), &extra);
        assert_eq!(alone.mods, full.mods);
    }

    fn table_of(patches: Vec<(Patch, Priority)>) -> PatchTable {
        let mut table = PatchTable::default();
        table.targets.insert("main.lua".to_string());
        for (i, (p, prio)) in patches.into_iter().enumerate() {
            table.patches.push((p, prio, PathBuf::from(format!("M{i}/qm.toml"))));
        }
        table
    }

    /// The first non-empty line of QuarterMaster's main.lua prefix.
    fn first_prefix_line() -> String {
        qm_core::patch::loader::STEAM_SAFETY_PAYLOAD
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap()
            .trim()
            .to_string()
    }

    fn game_lua() -> Option<LuaCompiler> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        [root.join("../../../game"), root.join("../../game")]
            .iter()
            .find_map(|d| LuaCompiler::find(d))
            .and_then(|p| LuaCompiler::load(&p).ok())
    }

    /// The simulation patches main.lua as the DLL prepares it (D1), but the file keeps its own
    /// numbering: QuarterMaster's lines are hidden from the diff, the regions and the trace.
    #[test]
    fn prepared_main_lua_keeps_file_numbers() {
        let r = simulate(
            vec![(pattern("to", "to:6", "  stop()", InsertPosition::After, "  log_halt()"), 20)],
            HALT_SRC,
        );
        let f = &r.files_touched[0];
        assert_eq!(r.traces["main.lua"].text, "function halt()\n  stop()\n  log_halt()\nend\n");
        assert_eq!((f.original_line_count, f.patched_line_count), (3, 4));
        let numbered: Vec<(usize, &str, Option<&str>)> = f
            .attributed_lines
            .iter()
            .map(|l| (l.line_number, l.text.as_str(), l.mod_id.as_deref()))
            .collect();
        assert_eq!(
            numbered,
            vec![
                (1, "function halt()", None),
                (2, "  stop()", None),
                (3, "  log_halt()", Some("to")),
                (4, "end", None),
            ]
        );
        assert_eq!(f.records[0].regions, vec![PatchRegion { start_line: 3, end_line: 3 }]);
        assert!(f.records[0].warnings.is_empty(), "{:?}", f.records[0].warnings);
    }

    /// A patch anchored on QuarterMaster's own prefix never matches (D42), in the game and in
    /// the simulation alike: no match, the `matched_qm_startup_code: skipped` warning, and the
    /// same records as the DLL.
    #[test]
    fn a_match_on_the_prefix_is_simulated_like_the_dll() {
        let patches = vec![
            (pattern("to", "to:6", "  stop()", InsertPosition::After, "  log_halt()"), 20),
            (pattern("hook", "hook:1", &first_prefix_line(), InsertPosition::After, "-- after qm"), 30),
        ];
        let table = table_of(patches.clone());
        let r = simulate(patches, HALT_SRC);
        let f = &r.files_touched[0];
        assert_eq!(
            r.traces["main.lua"].text,
            "function halt()\n  stop()\n  log_halt()\nend\n"
        );
        let hook = f.records.iter().find(|x| x.patch_id == "hook:1").unwrap();
        assert_eq!(hook.status, PatchOutcomeStatus::ZeroMatches);
        assert!(qm_core::prepare::is_builtin_match(hook), "{:?}", hook.warnings);
        assert!(
            hook.warnings.iter().any(|w| w.starts_with(qm_core::prepare::BUILTIN_MATCH_SKIPPED)),
            "{:?}",
            hook.warnings
        );
        assert!(hook.regions.is_empty());
        let to = f.records.iter().find(|x| x.patch_id == "to:6").unwrap();
        assert!(!qm_core::prepare::is_builtin_match(to));

        let prep = table.prepare();
        let launch = LaunchConfig::default();
        let qm_core::prepare::DllOutcome::Patched { text, records, .. } =
            qm_core::prepare::dll_transform(&table, &prep, &launch, "@main.lua", HALT_SRC, false)
        else {
            panic!("the DLL patches main.lua");
        };
        assert!(!text.contains("-- after qm"));
        let key = |r: &PatchExecutionRecord| {
            (r.patch_id.clone(), r.status, r.matched_count, r.warnings.clone())
        };
        assert_eq!(
            records.iter().map(key).collect::<Vec<_>>(),
            f.records.iter().map(key).collect::<Vec<_>>()
        );
    }

    /// The compile check compiles the text the DLL builds: a block a patch leaves open at the
    /// end of the file runs into QuarterMaster's footer, and the error is reported at the
    /// file's line where the block opened. (A patch can no longer land inside the prefix, D42.)
    #[test]
    fn the_compile_check_compiles_the_dll_text() {
        let Some(lua) = game_lua() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let patches = vec![(
            pattern("hook", "hook:1", "end", InsertPosition::After, "if qm_never then"),
            30,
        )];
        let table = table_of(patches);
        let vanilla = HashMap::from([("main.lua".to_string(), HALT_SRC.to_string())]);
        let r = simulate_and_analyze_patches_with(
            &table,
            &vanilla,
            SimulationOptions { compiler: Some(&lua), launch: None },
        );
        let c = r
            .conflicts
            .iter()
            .find(|c| c.kind == "compile_error")
            .unwrap_or_else(|| panic!("no compile error: {:?}", r.conflicts));
        assert_eq!(c.patch_a_id, "hook:1");
        assert!(c.description.contains("block opened at line 4"), "{}", c.description);
    }

    /// R1 row 3: a profile rule against a rung is ignored, and the simulation reports it as
    /// the game does (§3.9).
    #[test]
    fn the_order_results_reach_the_simulation_report() {
        use qm_core::patch::anchor::{AnchorSource, Rung};
        use qm_core::patch::order::OrderRule;
        let mut a = pattern("to", "to:6", "  stop()", InsertPosition::After, "  log_halt()");
        a.extra_mut().rungs = vec![(
            Rung {
                within: Some("gcore:halt".to_string()),
                pattern: Some("clear_path()".to_string()),
                exact: true,
                ..Rung::default()
            },
            AnchorSource::Profile {
                fix_id: "fx_1".to_string(),
            },
        )];
        a.extra_mut().fix_ids = vec!["fx_1".to_string()];
        let b = pattern("gcore", "gcore:halt", "  stop()", InsertPosition::At, "  stop()\n  clear_path()");
        let mut table = PatchTable::default();
        table.targets.insert("main.lua".to_string());
        table.patches.push((a, 10, PathBuf::from("TO/qm.toml")));
        table.patches.push((b, 20, PathBuf::from("GCore/qm.toml")));
        table.order_rules = vec![OrderRule {
            patch: "to:6".to_string(),
            before: vec!["gcore:halt".to_string()],
            after: Vec::new(),
        }];
        let vanilla = HashMap::from([("main.lua".to_string(), HALT_SRC.to_string())]);
        let r = simulate_and_analyze_patches(&table, &vanilla);
        assert_eq!(
            r.overridden_rules,
            ["profile rule to:6 before gcore:halt ignored: to:6 is anchored on gcore:halt"]
        );
        assert_eq!(
            r.traces["main.lua"].text,
            "function halt()\n  stop()\n  clear_path()\n  log_halt()\nend\n"
        );
        let to = r.files_touched[0].records.iter().find(|x| x.patch_id == "to:6").unwrap();
        assert_eq!(to.fix_ids, ["fx_1"]);
        assert_eq!(to.anchoring.as_ref().unwrap().sites[0].rung, Some(0));
        assert!(r.suspended_rungs.is_empty() && r.invalid_rungs.is_empty());
    }

    /// Every `at` patch gets a verdict (D46, §6.6): what it runs as, and its separate edits even
    /// when it runs literal.
    #[test]
    fn at_patches_say_how_they_run() {
        use qm_core::lenient::Lenient;
        let with = |p: Patch, a: Option<ApplyAs>| match p {
            Patch::Pattern(mut x) => {
                x.apply_as = a.map(Lenient::Ok);
                Patch::Pattern(x)
            }
            other => other,
        };
        let src = "a()\nb()\nend\nend\n";
        let r = simulate(
            vec![
                (with(pattern("g", "g:1", "a()\nb()", InsertPosition::At, "a()\nx()\ny()\nz()\nb()"), Some(ApplyAs::Edits)), 10),
                (pattern("g", "g:2", "a()", InsertPosition::At, "a()\nw()"), 20),
                (with(pattern("g", "g:3", "end", InsertPosition::At, "end\nq()\nend"), Some(ApplyAs::Edits)), 30),
                (pattern("g", "g:4", "b()", InsertPosition::After, "-- after"), 40),
            ],
            src,
        );
        let v = |id: &str| r.runs_as.iter().find(|v| v.patch_id == id).cloned();
        let one = v("g:1").expect("g:1");
        assert_eq!((one.asks, one.runs_as), (ApplyAs::Edits, ApplyAs::Edits));
        assert_eq!(one.edits, Some(EditCounts { kept: 2, inserted: 3, changed: 0, removed: 0 }));
        assert_eq!(one.text, "Runs as 3 insertions");
        let two = v("g:2").expect("g:2");
        assert_eq!((two.asks, two.runs_as), (ApplyAs::Literal, ApplyAs::Literal));
        assert_eq!(two.edits.map(|e| e.inserted), Some(1));
        assert_eq!(two.text, "Runs as one replacement");
        let three = v("g:3").expect("g:3");
        assert_eq!((three.runs_as, three.refused), (ApplyAs::Literal, Some(EditsRefusal::Ambiguous)));
        assert_eq!(three.text, "Runs as one replacement (the lines are ambiguous)");
        assert!(v("g:4").is_none(), "only `at` patches");
        assert_eq!(
            runs_as_text(&EditCounts { kept: 1, inserted: 1, changed: 2, removed: 1 }),
            "Runs as 1 insertion, 2 changed lines and 1 removed line"
        );
    }
}
