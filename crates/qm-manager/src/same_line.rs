//! Two patches that change one line of the game's file (D49, §6.5): a `same_line_edit`
//! warning with a choice of whose change stays. The other patch then skips its own change to
//! that line (a SkipHunk fix) and still runs its other edits.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use qm_core::handshake::{PatchExecutionRecord, PatchOutcomeStatus};
use qm_core::patch::edits::{self, Hunk};
use qm_core::patch::linemap::match_filter;
use qm_core::patch::origin::{FileTrace, LineOrigin};
use qm_core::patch::pattern::find_windows;
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, Patch, PatternPatch};
use qm_core::prepare::PreparedBuffer;
use wildmatch::WildMatch;

use crate::conflict::{ConflictResolutionSuggestion, DetectedConflict, SameLineDetail};

/// The `at` pattern patch of record `rec`.
fn at_patch<'t>(table: &'t PatchTable, rec: &PatchExecutionRecord) -> Option<&'t PatternPatch> {
    table.patches.iter().find_map(|(p, _, _)| match p {
        Patch::Pattern(x)
            if x.position == InsertPosition::At
                && p.id() == Some(rec.patch_id.as_str())
                && p.mod_id() == Some(rec.mod_id.as_str()) =>
        {
            Some(x)
        }
        _ => None,
    })
}

/// What one record changes: per 0-based prepared line, the edit that changes or removes it
/// (`None` when the patch cannot run as separate edits, so it has no edit to skip).
fn changed_lines(p: &PatternPatch, lines: &[&str], buf: &PreparedBuffer) -> Vec<(usize, Option<u32>)> {
    let wm: Vec<WildMatch> = p.pattern.lines().map(|l| WildMatch::new(l.trim())).collect();
    let windows = find_windows(lines, &wm, None, |i, len| match_filter(buf, i, len, true));
    let hunks = p.normalized().ok();
    // The engine skips these edits only in edits mode.
    let skipped: Vec<u32> = if p.edits_mode() {
        p.qm.skip_hunks.iter().map(|s| s.ordinal).collect()
    } else {
        Vec::new()
    };
    let mut out = Vec::new();
    for w in windows {
        match &hunks {
            Some(h) => {
                for (o, hunk) in edits::edit_hunks(h) {
                    let (Hunk::Change { p: range, .. } | Hunk::Delete { p: range }) = hunk else {
                        continue;
                    };
                    if skipped.contains(&o) {
                        continue;
                    }
                    out.extend(range.clone().map(|k| (w.line + k, Some(o))));
                }
            }
            None => out.extend((w.line..w.line + w.len).map(|l| (l, None))),
        }
    }
    out
}

/// One `same_line_edit` warning per pair of patches of different mods that both change or
/// remove a line of the game's file, where one of them did remove it (from the line origins).
/// Each offers "Keep <A>'s change" and "Keep <B>'s change" when that patch has edits to skip.
pub(crate) fn same_line_edits(
    table: &PatchTable,
    target: &str,
    buf: &PreparedBuffer,
    trace: &FileTrace,
    records: &[PatchExecutionRecord],
) -> Vec<DetectedConflict> {
    let lines: Vec<&str> = buf.text.lines().collect();
    let mut by_line: BTreeMap<usize, Vec<(usize, Option<u32>)>> = BTreeMap::new();
    for (ri, rec) in records.iter().enumerate() {
        if rec.patch_type != "pattern" || rec.status == PatchOutcomeStatus::SkippedDisabled {
            continue;
        }
        let Some(p) = at_patch(table, rec) else {
            continue;
        };
        for (line, o) in changed_lines(p, &lines, buf) {
            by_line.entry(line).or_default().push((ri, o));
        }
    }
    if by_line.values().all(|v| v.len() < 2) {
        return Vec::new();
    }
    // The game's lines (0-based) each record removed without writing them back.
    let mut removed: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (e, gone) in trace.removed.iter().enumerate() {
        let Some(rec) = trace.edits.get(e).map(|x| x.record) else {
            continue;
        };
        for r in gone.iter().filter(|r| !r.kept) {
            if let LineOrigin::Vanilla(v) = r.origin {
                removed.insert((rec, v.saturating_sub(1)));
            }
        }
    }
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut out = Vec::new();
    for (&line, who) in &by_line {
        for (i, &(a, oa)) in who.iter().enumerate() {
            for &(b, ob) in who.iter().skip(i + 1) {
                let (Some(ra), Some(rb)) = (records.get(a), records.get(b)) else {
                    continue;
                };
                let pair = (a.min(b), a.max(b));
                let taken = removed.contains(&(a, line)) || removed.contains(&(b, line));
                if a == b || ra.mod_id == rb.mod_id || !taken || !seen.insert(pair) {
                    continue;
                }
                out.push(conflict(table, target, buf, &lines, line, (ra, oa), (rb, ob)));
            }
        }
    }
    out
}

/// What edit `o` of `rec`'s patch writes in place of the lines it changes, trimmed and
/// joined by `\n` ("" for a removal). None without such an edit: the patch then replaces
/// its lines as one block.
fn new_text(table: &PatchTable, rec: &PatchExecutionRecord, o: Option<u32>) -> Option<String> {
    let p = at_patch(table, rec)?;
    let hunks = p.normalized().ok()?;
    let (_, hunk) = edits::edit_hunks(&hunks).find(|(k, _)| Some(*k) == o)?;
    match hunk {
        Hunk::Change { q, .. } => {
            let lit = edits::literal_payload(&p.payload, "");
            let text: Vec<&str> = lit
                .split_inclusive('\n')
                .skip(q.start)
                .take(q.len())
                .map(str::trim)
                .collect();
            Some(text.join("\n"))
        }
        Hunk::Delete { .. } => Some(String::new()),
        _ => None,
    }
}

fn conflict(
    table: &PatchTable,
    target: &str,
    buf: &PreparedBuffer,
    lines: &[&str],
    line: usize,
    (ra, oa): (&PatchExecutionRecord, Option<u32>),
    (rb, ob): (&PatchExecutionRecord, Option<u32>),
) -> DetectedConflict {
    let n = buf.file_line(line).unwrap_or(line + 1);
    let text = lines.get(line).map_or("", |l| l.trim());
    let keep = |kept: &PatchExecutionRecord, other: &PatchExecutionRecord, o: Option<u32>| {
        o.map(|ordinal| ConflictResolutionSuggestion::SkipHunk {
            patch_id: other.patch_id.clone(),
            mod_id: other.mod_id.clone(),
            ordinal,
            keep_patch_id: kept.patch_id.clone(),
            keep_mod_id: kept.mod_id.clone(),
            explanation: format!(
                "Keep {}'s change to line {n}. {} skips its own change there and still runs its other changes.",
                kept.patch_name, other.patch_name
            ),
        })
    };
    let suggestions = [keep(ra, rb, ob), keep(rb, ra, oa)]
        .into_iter()
        .flatten()
        .collect();
    DetectedConflict {
        id: format!("same_line:{target}:{}:{}:{n}", ra.patch_id, rb.patch_id),
        severity: "warning".to_string(),
        kind: "same_line_edit".to_string(),
        target_file: target.to_string(),
        mod_a_id: ra.mod_id.clone(),
        mod_b_id: Some(rb.mod_id.clone()),
        patch_a_id: ra.patch_id.clone(),
        patch_b_id: Some(rb.patch_id.clone()),
        description: format!(
            "Patch '{}' (from '{}') and patch '{}' (from '{}') both change line {n} of '{target}' ({text}). Only one change can stay.",
            ra.patch_name, ra.mod_id, rb.patch_name, rb.mod_id
        ),
        suggestions,
        consumer: None,
        sites_lost: None,
        same_line: Some(SameLineDetail {
            line: u32::try_from(n).unwrap_or(u32::MAX),
            vanilla: text.to_string(),
            a_text: new_text(table, ra, oa),
            b_text: new_text(table, rb, ob),
        }),
    }
}

#[cfg(test)]
#[path = "same_line_tests.rs"]
mod tests;
