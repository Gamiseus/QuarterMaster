#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use super::linemap::{byte_edit, Attach, Edit, LineMap, Side};
use super::stack::{place_in_stack, stacked_text, LegacySlot};
use super::{ApplyCtx, ApplyOutcome, OnFailurePolicy, Priority, Target};
use crate::dump::{ByteDebugEntry, ByteRegion, DebugPatchType, PatchSource};
use crate::prepare::COPY_AFTER_FOOTER_RETURN;
use crop::Rope;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CopyPosition {
    Prepend,
    Append,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CopyPatch {
    pub position: CopyPosition,
    pub target: Target,
    pub sources: Option<Vec<PathBuf>>,

    pub payload: Option<String>,

    pub id: Option<String>,
    pub name: Option<String>,
    pub priority: Option<Priority>,
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub on_failure: OnFailurePolicy,

    #[serde(skip)]
    pub mod_id: Option<String>,

    #[serde(skip)]
    pub disabled: bool,

    /// Profile and compat data (Phase C §3.3); never read from a patch file.
    #[serde(skip)]
    pub qm: crate::patch::plan::PatchExtra,

    // Buffer contents read at load time. We do this to support zip-based mods that we can't arbitrarily read from.
    #[serde(skip)]
    pub contents: Vec<String>,
}

impl CopyPatch {
    /// Apply a copy patch onto the provided buffer and name.
    pub fn apply(&self, target: &str, rope: &mut Rope, path: &Path) -> Option<ByteDebugEntry> {
        let lines = LineMap::new(rope.line_len(), &[], &[]);
        self.apply_in(target, rope, path, &ApplyCtx::legacy(&lines))
            .map(|o| o.entry)
    }

    /// `apply`, plus the edit in lines (§4.3): whole lines (an append's leading newline ends
    /// the last line when it has none). On a file QuarterMaster frames (main.lua), copies stay
    /// inside the frame (D43). A prepend hangs before the file's first own line and an append
    /// after its last, and both stack there; today's place is the far end of the stack (D53).
    pub(crate) fn apply_in(
        &self,
        target: &str,
        rope: &mut Rope,
        path: &Path,
        ctx: &ApplyCtx,
    ) -> Option<ApplyOutcome> {
        if !self.target.can_apply(target) {
            return None;
        }

        // Contents, then payload (if defined). Each prepend goes on top of the ones before it,
        // each append below them.
        let payloads: Vec<&str> = self
            .contents
            .iter()
            .map(|s| s.as_str())
            .chain(self.payload.as_deref())
            .collect();
        let frame = ctx.lines.frame_start().zip(ctx.lines.frame_end());
        let mut warnings = Vec::new();
        // An append ending in a top-level `return` stays after the footer (D43).
        let framed = frame.filter(|_| {
            if self.position != CopyPosition::Append {
                return true;
            }
            let ends_in_return = crate::lua_scan::scan(&payloads.join("\n")).top_level_return;
            if ends_in_return {
                let w = format!(
                    "{COPY_AFTER_FOOTER_RETURN}: the code ends in a top-level return, so it stays after QuarterMaster's footer in '{target}'"
                );
                log::warn!("{w}");
                warnings.push(w);
            }
            !ends_in_return
        });
        // The line the copy hangs on (D53). An append kept after the footer has none.
        let after_footer = frame.is_some() && framed.is_none();
        let anchor = match self.position {
            CopyPosition::Prepend => ctx.lines.first_own_line().map(|l| (l, Side::Before)),
            CopyPosition::Append if after_footer => None,
            CopyPosition::Append => ctx.lines.last_own_line().map(|l| (l, Side::After)),
        };
        let attach = anchor.and_then(|(line, side)| {
            ctx.lines.get(line).map(|t| Attach {
                uid: t.uid,
                side,
                block: t.block,
            })
        });
        let mut stack_moves = Vec::new();
        let moved = anchor
            .and_then(|(line, side)| place_in_stack(ctx, side, line, &[], 0, 0, LegacySlot::Far))
            .and_then(|p| {
                stack_moves.push(p.report);
                p.line
            });

        let mut text = String::new();
        for content in &payloads {
            match self.position {
                CopyPosition::Prepend => {
                    let mut piece = String::with_capacity(content.len() + 1 + text.len());
                    piece.push_str(content);
                    piece.push('\n');
                    piece.push_str(&text);
                    text = piece;
                }
                // Inside the frame (whole lines just above the footer), or moved in a stack.
                CopyPosition::Append if framed.is_some() || moved.is_some() => {
                    text.push_str(content);
                    text.push('\n');
                }
                CopyPosition::Append => {
                    text.push('\n');
                    text.push_str(content);
                }
            }
        }

        let mut byte_regions = Vec::new();
        let mut edits = Vec::new();
        let mut touches_builtin = false;
        if !text.is_empty() {
            let line_len = rope.line_len();
            let at = match (self.position, framed, moved) {
                (_, _, Some(line)) => rope.byte_of_line(line.min(line_len)),
                (CopyPosition::Prepend, Some((top, _)), None) => {
                    rope.byte_of_line(top.min(line_len))
                }
                (CopyPosition::Append, Some((_, footer)), None) => {
                    rope.byte_of_line(footer.min(line_len))
                }
                (CopyPosition::Prepend, None, None) => 0,
                (CopyPosition::Append, None, None) => rope.byte_len(),
            };
            if moved.is_some() {
                text = stacked_text(rope, at, &text).into_owned();
            }
            let edit = byte_edit(rope, at, at, &text, 0, attach);
            if let Edit::Touched {
                first_line,
                removed_lines,
                ..
            } = &edit
            {
                touches_builtin = (*first_line..first_line + removed_lines)
                    .any(|l| ctx.lines.is_builtin_line(l));
            }
            edits.push(edit);
            rope.insert(at, &text);
            byte_regions.push(ByteRegion {
                start: at,
                end: at + text.len(),
                delta: text.len() as isize,
            });
        }
        let entry = ByteDebugEntry {
            patch_source: PatchSource {
                file: path.display().to_string(),
                mod_id: self.mod_id.clone(),
                patch_id: self.id.clone(),
                patch_name: self.name.clone(),
                pattern: None,
                patch_type: DebugPatchType::Copy,
            },
            regions: byte_regions,
            matched_count: 1,
            expected_count: Some(1),
            warnings: (!warnings.is_empty()).then_some(warnings),
        };
        Some(ApplyOutcome {
            entry,
            edits,
            touches_builtin,
            stack_moves,
            anchoring: None,
        })
    }
}
