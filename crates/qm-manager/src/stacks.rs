//! Shared stacks (plan 2e §5.5): lines where code from two or more patches sits on one side,
//! read from the final line map of a simulated file. Stacks are not conflicts (D28).

use std::collections::{BTreeSet, HashMap};

use qm_core::handshake::{PatchExecutionRecord, StackMode, StackReason};
use qm_core::lua_scan;
use qm_core::patch::anchor::SiteKey;
use qm_core::patch::linemap::{LineMap, Side, NO_LINE};
use qm_core::patch::stack::{legacy_slot, line_key, LegacySlot};
use qm_core::prepare::{FileView, PreparedBuffer};
use serde::{Deserialize, Serialize};

/// One line with code from two or more patches on one side, top to bottom.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedStack {
    pub target_file: String,
    /// 1-based line of the shared line, as the file shows it.
    pub file_line: u32,
    pub anchor_text: String,
    pub side: Side,
    /// Where the line is, for example "in love.load".
    pub site_label: String,
    pub mode: StackMode,
    /// In the order the game runs them.
    pub blocks: Vec<StackBlock>,
    /// Patch ids in the older order (newest first after a line, last before it).
    pub legacy: Vec<String>,
    /// `blocks` are not in the older order.
    pub moved: bool,
    /// The game's line, as a stack rule names it (D55). Its context is empty when the line is
    /// another patch's code: no rule can name it.
    pub site: SiteKey,
    /// Unique per file, line and side, and the same across runs while the line is the same.
    pub stack_key: String,
}

/// One patch's code in a shared stack, with every line nested inside it.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackBlock {
    pub patch_id: String,
    pub patch_name: String,
    pub mod_id: String,
    /// 1-based, as the file shows it.
    pub first_line: u32,
    pub last_line: u32,
    /// Why stacking put it here, when not in the older slot.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub reason: Option<StackReason>,
    /// Why the block cannot be moved here, when it cannot.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub locked_reason: Option<String>,
    /// The user's stack rule that placed or holds the block here (D55).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub rule_fix_id: Option<String>,
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Before sorts first at one line.
fn side_rank(side: Side) -> u8 {
    match side {
        Side::Before => 0,
        Side::After => 1,
    }
}

fn site_label(scan: Option<&lua_scan::Scan>, line: usize) -> String {
    match scan.map(|s| s.function_name(line)) {
        None | Some("<main>") => "outside any function".to_string(),
        Some("<anonymous>") => "in an unnamed function".to_string(),
        Some(name) => format!("in {name}"),
    }
}

/// The shared stacks of one patched file. `lines` and `records` are the apply's final line
/// map and records; `text` is the patched text in the same numbering; `view` maps it to the
/// file's own numbering; `original` is the buffer the apply started from (for site keys).
pub fn shared_stacks(
    target: &str,
    mode: StackMode,
    lines: &LineMap,
    records: &[PatchExecutionRecord],
    text: &str,
    view: &FileView,
    original: &PreparedBuffer,
) -> Vec<SharedStack> {
    // Live uid -> line, so each block's anchor is found once.
    let live: HashMap<u32, usize> = lines
        .tags()
        .iter()
        .enumerate()
        .map(|(i, t)| (t.uid, i))
        .collect();
    let mut keys: BTreeSet<(usize, u8)> = BTreeSet::new();
    // Implicit anchors (D51) count only while implicit chains are on.
    for block in lines.blocks() {
        let Some((uid, side)) = lines.chain_anchor(block) else {
            continue;
        };
        if let Some(&line) = live.get(&uid) {
            keys.insert((line, side_rank(side)));
        }
    }

    let mut out = Vec::new();
    let mut text_lines: Option<Vec<&str>> = None;
    let mut original_lines: Option<Vec<&str>> = None;
    let mut scan: Option<Option<lua_scan::Scan>> = None;
    for (line, rank) in keys {
        let side = if rank == 0 { Side::Before } else { Side::After };
        let items = lines.chain(line, side);
        if items.len() < 2 {
            continue;
        }
        let text_lines = text_lines.get_or_insert_with(|| text.lines().collect());
        let scan =
            scan.get_or_insert_with(|| target.ends_with(".lua").then(|| lua_scan::scan(text)));

        // (application order, patch id, is a copy) per item; records are in application order.
        let mut blocks = Vec::with_capacity(items.len());
        let mut applied: Vec<(u32, String, bool)> = Vec::with_capacity(items.len());
        for item in &items {
            let Some(info) = lines.block(item.block) else {
                continue;
            };
            let rec = records.get(info.record as usize);
            let patch_id = rec.map(|r| r.patch_id.clone()).unwrap_or_default();
            let copy = rec.is_some_and(|r| r.patch_type == "copy");
            applied.push((info.record, patch_id.clone(), copy));
            let first_line = view.line_at_or_after(item.first + 1);
            let moves = || {
                rec.into_iter()
                    .flat_map(|r| &r.stack_moves)
                    .filter(|m| m.site == info.site)
            };
            let rule_fix_id = moves().find_map(|m| match &m.reason {
                StackReason::UserRule { fix_id } => Some(fix_id.clone()),
                _ => None,
            });
            let locked_reason = moves()
                .any(|m| m.reason == StackReason::GuardClamped)
                .then(|| "It stays here so its code keeps its order in this function".to_string());
            blocks.push(StackBlock {
                patch_id,
                patch_name: rec.map(|r| r.patch_name.clone()).unwrap_or_default(),
                mod_id: rec.map(|r| r.mod_id.clone()).unwrap_or_default(),
                first_line: to_u32(first_line),
                last_line: to_u32(view.line_at_or_before(item.last + 1).max(first_line)),
                reason: rec.and_then(|r| {
                    r.stack_moves
                        .iter()
                        .find(|m| m.site == info.site)
                        .map(|m| m.reason.clone())
                }),
                locked_reason,
                rule_fix_id,
            });
        }
        // The older rule: after a line the newest goes on top, before a line right above it;
        // a copy goes to the far end instead (D53).
        applied.sort_by_key(|(record, ..)| *record);
        let mut legacy: Vec<String> = Vec::with_capacity(applied.len());
        for (_, id, copy) in applied {
            let kind = if copy {
                LegacySlot::Far
            } else {
                LegacySlot::Near
            };
            legacy.insert(legacy_slot(side, legacy.len(), kind), id);
        }
        let site_line = match side {
            Side::After => items.first().map_or(line, |c| c.first),
            Side::Before => items.last().map_or(line, |c| c.last),
        } + 1;
        let moved = blocks.iter().map(|b| &b.patch_id).ne(legacy.iter());
        // The game's line the stack hangs on; a line of patch code has no key (D55).
        let vanilla = lines
            .get(line)
            .map(|t| t.vanilla)
            .filter(|&v| v != NO_LINE)
            .map(|v| v as usize);
        let site = match vanilla {
            Some(u) => {
                let orig = original_lines.get_or_insert_with(|| original.text.lines().collect());
                let builtin = |i: usize| {
                    original.builtin.iter().any(|r| r.contains(&i))
                        || original.builtin_edited.contains(&i)
                };
                line_key(orig, builtin, u)
            }
            None => SiteKey {
                context: String::new(),
                ordinal: None,
            },
        };
        let side_name = match side {
            Side::Before => "before",
            Side::After => "after",
        };
        let stack_key = if site.context.is_empty() {
            format!("{target}#line{}#{side_name}", line + 1)
        } else {
            format!(
                "{target}#{}#{}#{side_name}",
                site.context,
                site.ordinal.unwrap_or(0)
            )
        };
        if site.context.is_empty() {
            for b in &mut blocks {
                b.locked_reason.get_or_insert_with(|| {
                    "It hangs on another mod's code, so it cannot be moved here".to_string()
                });
            }
        }
        out.push(SharedStack {
            target_file: target.to_string(),
            file_line: to_u32(
                view.line(line + 1)
                    .unwrap_or_else(|| view.line_at_or_after(line + 1)),
            ),
            anchor_text: text_lines
                .get(line)
                .map(|l| l.trim().to_string())
                .unwrap_or_default(),
            side,
            // Where the stacked code runs: after a function header, that is inside it.
            site_label: site_label(scan.as_ref(), site_line),
            mode,
            blocks,
            legacy,
            moved,
            site,
            stack_key,
        });
    }
    out
}

#[cfg(test)]
mod tests;
