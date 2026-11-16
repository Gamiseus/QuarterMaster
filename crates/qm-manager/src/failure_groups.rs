//! Root-cause grouping (G10, C9): a patch that fails because it anchors on another patch's code,
//! and that patch did not apply, is grouped under the first patch of the chain that failed for
//! a reason of its own. The Order view also draws every anchor on another patch's code as an
//! inferred edge. Only the simulation reads this; the game's output is unchanged.

use serde::{Deserialize, Serialize};

use qm_core::handshake::{PatchExecutionRecord, PatchOutcomeStatus};
use qm_core::patch::table::{skipped_for_requirement, SKIPPED_WITH_MOD_WARNING};

use crate::consumer::Use;

/// Why the root of a failure group does not apply.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// It no longer finds its line.
    NoMatch,
    /// The profile turned it off.
    TurnedOff,
    /// It was skipped with its mod, or because a patch it requires did not apply.
    Skipped,
}

/// A patch that fails because of the group's root.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureMember {
    pub patch_id: String,
    pub mod_id: String,
    pub name: String,
}

/// Patches in one file that fail because `root` does not apply: each anchors on the code of the
/// root or of another member.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureGroup {
    pub target_file: String,
    /// The root's patch id.
    pub root: String,
    pub root_mod: String,
    pub root_name: String,
    /// In the order the patches run.
    pub members: Vec<FailureMember>,
    pub reason: FailureReason,
}

/// A patch that anchors on another patch's code: `patch_id` uses a line `uses_patch_id` wrote.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferredEdge {
    pub target_file: String,
    pub patch_id: String,
    pub mod_id: String,
    pub uses_patch_id: String,
    pub uses_mod_id: String,
}

/// Skipped with its mod or for a requirement: the patch did not try to match.
pub(crate) fn skipped(r: &PatchExecutionRecord) -> bool {
    r.status == PatchOutcomeStatus::SkippedByPolicy
        && (r.warnings.iter().any(|w| w == SKIPPED_WITH_MOD_WARNING) || skipped_for_requirement(r))
}

/// The patch tried to match and found no place.
fn failed(r: &PatchExecutionRecord) -> bool {
    match r.status {
        PatchOutcomeStatus::ZeroMatches | PatchOutcomeStatus::Aborted => true,
        PatchOutcomeStatus::SkippedByPolicy => !skipped(r),
        _ => false,
    }
}

/// The patch's code is in the file.
fn applied(r: &PatchExecutionRecord) -> bool {
    matches!(r.status, PatchOutcomeStatus::Applied | PatchOutcomeStatus::CountMismatch)
}

fn reason_of(r: &PatchExecutionRecord) -> FailureReason {
    if r.status == PatchOutcomeStatus::SkippedDisabled {
        FailureReason::TurnedOff
    } else if skipped(r) {
        FailureReason::Skipped
    } else {
        FailureReason::NoMatch
    }
}

/// The inferred edges of one file, one per pair, in record order.
pub(crate) fn inferred_edges(
    target: &str,
    records: &[PatchExecutionRecord],
    uses: &[Use],
) -> Vec<InferredEdge> {
    let mut out: Vec<InferredEdge> = Vec::new();
    for u in uses {
        let (Some(a), Some(b)) = (records.get(u.record), records.get(u.uses)) else {
            continue;
        };
        if a.patch_id == b.patch_id {
            continue;
        }
        let edge = InferredEdge {
            target_file: target.to_string(),
            patch_id: a.patch_id.clone(),
            mod_id: a.mod_id.clone(),
            uses_patch_id: b.patch_id.clone(),
            uses_mod_id: b.mod_id.clone(),
        };
        if !out.contains(&edge) {
            out.push(edge);
        }
    }
    out
}

/// The failure groups of one file. A failed patch whose line is only in code that did not
/// apply hangs under that code's patch; a failed patch with a line in applied code, or with no
/// such line, fails for its own reason and is the root of its own chain.
pub(crate) fn failure_groups(
    target: &str,
    records: &[PatchExecutionRecord],
    uses: &[Use],
) -> Vec<FailureGroup> {
    let mut parent: Vec<Option<usize>> = vec![None; records.len()];
    for (f, rec) in records.iter().enumerate() {
        if !failed(rec) {
            continue;
        }
        // Only code that ran before it: a later patch's line was not there to find.
        let sources: Vec<usize> = uses
            .iter()
            .filter(|u| u.record == f && u.uses < f)
            .map(|u| u.uses)
            .collect();
        let held = |g: &usize| records.get(*g).is_some_and(|r| !applied(r));
        if !sources.is_empty() && sources.iter().all(held) {
            if let Some(p) = parent.get_mut(f) {
                *p = sources.first().copied();
            }
        }
    }
    let root_of = |f: usize| -> Option<usize> {
        let mut at = f;
        for _ in 0..records.len() {
            match parent.get(at).copied().flatten() {
                Some(p) if p == f => return None,
                Some(p) => at = p,
                None => return Some(at),
            }
        }
        None
    };
    let mut out: Vec<FailureGroup> = Vec::new();
    for (f, rec) in records.iter().enumerate() {
        if parent.get(f).copied().flatten().is_none() {
            continue;
        }
        let Some(r) = root_of(f) else {
            continue;
        };
        let Some(root) = records.get(r) else {
            continue;
        };
        let member = FailureMember {
            patch_id: rec.patch_id.clone(),
            mod_id: rec.mod_id.clone(),
            name: rec.patch_name.clone(),
        };
        match out.iter_mut().find(|g| g.root == root.patch_id) {
            Some(g) => g.members.push(member),
            None => out.push(FailureGroup {
                target_file: target.to_string(),
                root: root.patch_id.clone(),
                root_mod: root.mod_id.clone(),
                root_name: root.patch_name.clone(),
                members: vec![member],
                reason: reason_of(root),
            }),
        }
    }
    out
}

#[cfg(test)]
#[path = "failure_groups_tests.rs"]
mod tests;
