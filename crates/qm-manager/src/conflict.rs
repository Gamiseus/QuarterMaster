use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use qm_core::handshake::{GluePatchEntry, PatchExecutionRecord, PatchOutcomeStatus};
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, OnFailurePolicy, Patch, PatternPatch, Priority, Target};

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationReport {
    pub files_touched: Vec<SimulatedFileResult>,
    pub conflicts: Vec<DetectedConflict>,
    pub module_registrations: BTreeMap<String, Vec<String>>,
    pub total_patches: usize,
    pub applied_patches: usize,
    pub broken_anchors: usize,
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
    /// Tier 2: Earlier mod replaced the vanilla anchor; offers a 1-click profile-local glue patch
    /// that anchors onto the earlier mod's replacement lines.
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
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedConflict {
    pub id: String,
    pub severity: String, // "error" | "warning"
    pub kind: String,     // "broken_anchor_order" | "broken_anchor_replaced" | "duplicate_module" | "overlapping_region"
    pub target_file: String,
    pub mod_a_id: String,
    pub mod_b_id: Option<String>,
    pub patch_a_id: String,
    pub patch_b_id: Option<String>,
    pub description: String,
    pub suggestions: Vec<ConflictResolutionSuggestion>,
}

/// Runs the Offline Patch Engine across `patch_table` and `vanilla_files` (`rel_path -> content`),
/// builds attributed file diffs, and runs the 3-Tier Conflict Resolver.
pub fn simulate_and_analyze_patches(
    patch_table: &PatchTable,
    vanilla_files: &HashMap<String, String>,
) -> SimulationReport {
    let mut files_touched = Vec::new();
    let mut conflicts = Vec::new();
    let mut module_registrations: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut total_patches = 0usize;
    let mut applied_patches = 0usize;
    let mut broken_anchors = 0usize;

    // 1. Check module registrations for duplicate `[[patches.module]]` collisions
    let sorted_modules: Vec<_> = patch_table
        .patches
        .iter()
        .filter_map(|(p, prio, _)| match p {
            Patch::Module(m) if !m.disabled => Some((m, *prio)),
            _ => None,
        })
        .collect();

    for (m, _prio) in sorted_modules {
        let mod_id = m.mod_id.clone().unwrap_or_else(|| "unknown".to_string());
        module_registrations
            .entry(m.name.clone())
            .or_default()
            .push(mod_id);
    }

    for (module_name, owners) in &module_registrations {
        if owners.len() > 1 {
            let first = owners[0].clone();
            let last = owners.last().unwrap().clone();
            conflicts.push(DetectedConflict {
                id: format!("module-collision:{module_name}"),
                severity: "warning".to_string(),
                kind: "duplicate_module".to_string(),
                target_file: module_name.clone(),
                mod_a_id: first.clone(),
                mod_b_id: Some(last.clone()),
                patch_a_id: format!("{first}:module:{module_name}"),
                patch_b_id: Some(format!("{last}:module:{module_name}")),
                description: format!(
                    "Module '{}' is registered by multiple mods ({:?}); '{}' overwrites earlier definitions.",
                    module_name, owners, last
                ),
                suggestions: vec![ConflictResolutionSuggestion::SelectWinner {
                    patch_a_id: format!("{first}:module:{module_name}"),
                    mod_a_id: first,
                    patch_b_id: format!("{last}:module:{module_name}"),
                    mod_b_id: last,
                    explanation: "Choose which mod provides this Lua module.".to_string(),
                }],
            });
        }
    }

    // 2. Simulate each target file in deterministic alphabetical order
    let mut sorted_targets: Vec<String> = patch_table
        .targets
        .iter()
        .filter(|t| !t.is_empty())
        .map(|t| t.trim_start_matches('@').to_string())
        .collect();
    sorted_targets.sort();
    sorted_targets.dedup();

    for target in sorted_targets {
        let Some(vanilla_src) = vanilla_files.get(&target) else {
            continue;
        };

        let Ok((patched_src, debug, records)) =
            patch_table.apply_patches_offline(&target, vanilla_src)
        else {
            continue;
        };

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

        // Build attributed code lines (keep modified lines + 4 vanilla context lines around each modified block
        // if the file is > 1200 lines, or all lines if <= 1200 lines, so UI rendering stays instant)
        let lines: Vec<&str> = patched_src.lines().collect();
        let mut include_line = vec![lines.len() <= 1200; lines.len()];
        if lines.len() > 1200 {
            for idx in 0..lines.len() {
                if debug.find_entry_at_line(idx + 1).is_some() {
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
            let entry = debug.find_entry_at_line(line_no);
            attributed_lines.push(AttributedCodeLine {
                line_number: line_no,
                text: (*text).to_string(),
                mod_id: entry.and_then(|e| e.patch_source.mod_id.clone()),
                patch_id: entry.and_then(|e| e.patch_source.patch_id.clone()),
                patch_name: entry.and_then(|e| e.patch_source.patch_name.clone()),
            });
        }

        // 3. Diagnose any broken anchors (`ZeroMatches`) on this file using Tier 1 & Tier 2
        for rec in &records {
            if rec.status == PatchOutcomeStatus::ZeroMatches {
                let diag = diagnose_broken_anchor(
                    &target,
                    vanilla_src,
                    rec,
                    &records,
                    patch_table,
                );
                conflicts.push(diag);
            }
        }

        // 4. Detect overlapping regions between different mods on the final buffer
        for i in 0..records.len() {
            for j in (i + 1)..records.len() {
                let ra = &records[i];
                let rb = &records[j];
                if ra.mod_id == rb.mod_id {
                    continue;
                }
                let overlaps = ra.regions.iter().any(|reg_a| {
                    rb.regions.iter().any(|reg_b| {
                        reg_a.start_line <= reg_b.end_line && reg_b.start_line <= reg_a.end_line
                    })
                });
                if overlaps {
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
                            "Patch '{}' (from '{}') and patch '{}' (from '{}') both modify overlapping lines in '{}'.",
                            ra.patch_name, ra.mod_id, rb.patch_name, rb.mod_id, target
                        ),
                        suggestions: vec![ConflictResolutionSuggestion::SelectWinner {
                            patch_a_id: ra.patch_id.clone(),
                            mod_a_id: ra.mod_id.clone(),
                            patch_b_id: rb.patch_id.clone(),
                            mod_b_id: rb.mod_id.clone(),
                            explanation: "Select which patch should stay enabled on this region."
                                .to_string(),
                        }],
                    });
                }
            }
        }

        files_touched.push(SimulatedFileResult {
            target_file: target,
            mods_touching,
            records,
            attributed_lines,
            original_line_count: vanilla_src.lines().count(),
            patched_line_count: lines.len(),
        });
    }

    SimulationReport {
        files_touched,
        conflicts,
        module_registrations,
        total_patches,
        applied_patches,
        broken_anchors,
    }
}

/// Runs Tier 1 (Automatic Order-Swap Test) and Tier 2 (1-Click Re-Anchoring Glue Patch Generator)
/// when `failed_rec` had 0 matches on `target`.
fn diagnose_broken_anchor(
    target: &str,
    vanilla_src: &str,
    failed_rec: &PatchExecutionRecord,
    file_records: &[PatchExecutionRecord],
    full_table: &PatchTable,
) -> DetectedConflict {
    let mut suggestions = Vec::new();
    let mut culprit_mod_id: Option<String> = None;
    let mut culprit_patch_id: Option<String> = None;

    // Find the failing Patch object
    let failing_tuple = full_table
        .patches
        .iter()
        .find(|(p, _, _)| p.id() == Some(failed_rec.patch_id.as_str()));

    if let Some((failing_patch, _failing_prio, failing_path)) = failing_tuple {
        // First test: Does `failing_patch` match against unmodified `vanilla_src` on its own?
        let mut solo_table = PatchTable::default();
        solo_table.targets.insert(target.to_string());
        solo_table
            .patches
            .push((failing_patch.clone(), 0, failing_path.clone()));

        let matches_vanilla_alone = solo_table
            .apply_patches_offline(target, vanilla_src)
            .ok()
            .and_then(|(_, _, recs)| recs.first().map(|r| r.matched_count > 0))
            .unwrap_or(false);

        if matches_vanilla_alone {
            // Another mod's earlier patch on `target` clobbered the vanilla anchor!
            // Find which earlier mod/patch broke it and test Tier 1 (Order Swap):
            let earlier_mods: Vec<String> = file_records
                .iter()
                .filter(|r| r.priority <= failed_rec.priority && r.mod_id != failed_rec.mod_id)
                .map(|r| r.mod_id.clone())
                .collect();

            for cand_mod in earlier_mods {
                let cand_min_prio = file_records
                    .iter()
                    .filter(|r| r.mod_id == cand_mod)
                    .map(|r| r.priority)
                    .min()
                    .unwrap_or(10);
                let test_prio = cand_min_prio - 5;

                // Build a test table where `failed_rec.mod_id` runs at `test_prio` (before `cand_mod`)
                let swapped_table = PatchTable {
                    mod_dir: full_table.mod_dir.clone(),
                    targets: full_table.targets.clone(),
                    vars: full_table.vars.clone(),
                    patches: full_table
                        .patches
                        .iter()
                        .map(|(p, prio, path)| {
                            if p.mod_id() == Some(failed_rec.mod_id.as_str()) {
                                (p.clone(), test_prio, path.clone())
                            } else {
                                (p.clone(), *prio, path.clone())
                            }
                        })
                        .collect(),
                };

                if let Ok((_, _, swapped_recs)) =
                    swapped_table.apply_patches_offline(target, vanilla_src)
                {
                    let all_succeeded = swapped_recs
                        .iter()
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

            // Tier 2: Also offer a 1-Click Re-Anchoring Glue Patch if `failing_patch` is a PatternPatch
            if let Patch::Pattern(pat_patch) = failing_patch {
                if let Some(earlier_rec) = file_records
                    .iter()
                    .find(|r| r.priority <= failed_rec.priority && r.mod_id != failed_rec.mod_id)
                {
                    if culprit_mod_id.is_none() {
                        culprit_mod_id = Some(earlier_rec.mod_id.clone());
                        culprit_patch_id = Some(earlier_rec.patch_id.clone());
                    }

                    // Find the earlier patch's payload last line to use as a new anchor
                    if let Some((Patch::Pattern(earlier_pat), _, _)) = full_table
                        .patches
                        .iter()
                        .find(|(p, _, _)| p.id() == Some(earlier_rec.patch_id.as_str()))
                    {
                        if let Some(new_anchor_line) = earlier_pat
                            .payload
                            .lines()
                            .rev()
                            .map(|l| l.trim())
                            .find(|l| !l.is_empty() && !l.starts_with("--"))
                        {
                            let glue = GluePatchEntry {
                                id: format!("glue:{}", failed_rec.patch_id),
                                name: format!(
                                    "Re-anchored '{}' after '{}'",
                                    failed_rec.patch_name, earlier_rec.mod_id
                                ),
                                replaces_patch_id: Some(failed_rec.patch_id.clone()),
                                priority: failed_rec.priority,
                                patch: Patch::Pattern(PatternPatch {
                                    pattern: new_anchor_line.to_string(),
                                    position: InsertPosition::After,
                                    target: Target::Single(target.to_string()),
                                    payload: pat_patch.payload.clone(),
                                    match_indent: pat_patch.match_indent,
                                    times: pat_patch.times,
                                    overwrite: false,
                                    id: Some(format!("glue:{}", failed_rec.patch_id)),
                                    name: Some(format!(
                                        "Glue: {} + {}",
                                        earlier_rec.mod_id, failed_rec.mod_id
                                    )),
                                    priority: Some(failed_rec.priority),
                                    description: None,
                                    requires: Vec::new(),
                                    before: Vec::new(),
                                    after: Vec::new(),
                                    on_failure: OnFailurePolicy::Warn,
                                    mod_id: Some("qm_glue".to_string()),
                                    disabled: false,
                                }),
                            };
                            suggestions.push(ConflictResolutionSuggestion::GenerateGluePatch {
                                glue_patch: glue,
                                explanation: format!(
                                    "Generate a profile-local compatibility patch that re-anchors '{}' onto '{}'\'s output (`{}`).",
                                    failed_rec.patch_name, earlier_rec.mod_id, new_anchor_line
                                ),
                            });
                        }
                    }
                }
            }
        }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
            mod_id: Some("tactical_overhaul".to_string()),
            disabled: false,
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
            mod_id: Some("gcore".to_string()),
            disabled: false,
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

        // Verify Tier 1 (SwapModOrder) AND Tier 2 (GenerateGluePatch) suggestions were both generated!
        assert!(conflict.suggestions.iter().any(|s| matches!(
            s,
            ConflictResolutionSuggestion::SwapModOrder {
                move_mod_id,
                before_mod_id,
                ..
            } if move_mod_id == "gcore" && before_mod_id == "tactical_overhaul"
        )));
        assert!(conflict.suggestions.iter().any(|s| matches!(
            s,
            ConflictResolutionSuggestion::GenerateGluePatch { .. }
        )));
    }
}
