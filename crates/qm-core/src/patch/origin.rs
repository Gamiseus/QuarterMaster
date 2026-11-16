//! Where every line of a patched file came from.
//!
//! `PatchTable::apply_patches_traced` records each committed edit in application order, in the
//! coordinates of the moment it was made. The trace it returns is built from the apply's
//! `LineMap` (`FileTrace::from_linemap`); `trace` replays the same journal over the game's file
//! and stays as the test oracle for it. Both give every final line an origin and tell, for each
//! edit, which lines it removed. The simulation uses this for exact patch output, overlap
//! detection and error attribution; the game's output is unchanged.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::collections::HashMap;

use serde::Serialize;

use crate::patch::linemap::{end_lines, kept_pairs, LineKind, LineMap, LineTag, Removed, NO_LINE};

/// One committed edit: at byte `start`, `removed` bytes were replaced by `inserted`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JournalEdit {
    pub patch_id: String,
    pub patch_name: String,
    pub mod_id: String,
    /// Index of the patch's record in the `PatchExecutionRecord` list of the same run.
    pub record: usize,
    pub start: usize,
    pub removed: usize,
    pub inserted: String,
    /// Which of the record's edits this is.
    #[serde(default)]
    pub site: u32,
    /// The `LineMap` block the edit created (`linemap::NO_BLOCK` for none).
    #[serde(default)]
    pub block: u32,
}

/// Where a line came from. `edit` indexes the journal; `vanilla` is a 1-based line of the
/// game's file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LineOrigin {
    /// The game's own line, untouched.
    Vanilla(usize),
    /// Added by an edit.
    Inserted { edit: usize },
    /// Re-written unchanged by an edit that replaced it (an `at` patch whose payload repeats a
    /// line of its window).
    Kept { edit: usize, vanilla: Option<usize> },
    /// Changed inside the line by an edit that does not work on whole lines (regex patches).
    Modified { edit: usize, vanilla: Option<usize> },
}

impl LineOrigin {
    pub fn edit(&self) -> Option<usize> {
        match *self {
            LineOrigin::Vanilla(_) => None,
            LineOrigin::Inserted { edit } | LineOrigin::Kept { edit, .. } | LineOrigin::Modified { edit, .. } => Some(edit),
        }
    }

    /// The game's line this one stands for, when there is one.
    pub fn vanilla(&self) -> Option<usize> {
        match *self {
            LineOrigin::Vanilla(j) => Some(j),
            LineOrigin::Inserted { .. } => None,
            LineOrigin::Kept { vanilla, .. } | LineOrigin::Modified { vanilla, .. } => vanilla,
        }
    }

    /// Whether this line is new or changed code from some edit (not the game's line or a
    /// verbatim copy of one).
    pub fn is_patch_output(&self) -> bool {
        matches!(self, LineOrigin::Inserted { .. } | LineOrigin::Modified { .. })
    }
}

/// A line an edit removed, and whether the same edit wrote it back unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RemovedLine {
    pub origin: LineOrigin,
    pub kept: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FileTrace {
    /// The final patched text.
    pub text: String,
    /// One origin per line of `text` (lines as `str::lines` counts them).
    pub lines: Vec<LineOrigin>,
    pub edits: Vec<JournalEdit>,
    /// Per edit, the lines it removed.
    pub removed: Vec<Vec<RemovedLine>>,
    /// The patched text before variable interpolation (what patterns match), when it differs
    /// from `text`. Same lines as `text`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_text: Option<String>,
}

impl FileTrace {
    /// The patch that wrote line `n` (1-based), if any.
    pub fn edit_at(&self, n: usize) -> Option<&JournalEdit> {
        let i = self.lines.get(n.checked_sub(1)?)?.edit()?;
        self.edits.get(i)
    }

    /// 1-based line runs whose origin is one of `patch_id`'s edits.
    pub fn runs_of(&self, patch_id: &str) -> Vec<(usize, usize)> {
        self.runs_where(|e| e.patch_id == patch_id)
    }

    /// 1-based line runs written by the patch with record index `record`.
    pub fn runs_of_record(&self, record: usize) -> Vec<(usize, usize)> {
        self.runs_where(|e| e.record == record)
    }

    /// 1-based runs of lines the patch with record index `record` added (not lines it kept or
    /// changed in place).
    pub fn inserted_runs_of_record(&self, record: usize) -> Vec<(usize, usize)> {
        self.runs_matching(|o| matches!(o, LineOrigin::Inserted { edit } if self.edits.get(*edit).is_some_and(|e| e.record == record)))
    }

    fn runs_where(&self, f: impl Fn(&JournalEdit) -> bool) -> Vec<(usize, usize)> {
        self.runs_matching(|o| o.edit().and_then(|e| self.edits.get(e)).is_some_and(&f))
    }

    fn runs_matching(&self, f: impl Fn(&LineOrigin) -> bool) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = Vec::new();
        for (idx, o) in self.lines.iter().enumerate() {
            let mine = f(o);
            if !mine {
                continue;
            }
            let n = idx + 1;
            match out.last_mut() {
                Some(last) if last.1 + 1 == n => last.1 = n,
                _ => out.push((n, n)),
            }
        }
        out
    }
}

impl FileTrace {
    /// The trace of an apply, from its final `LineMap`: `text` is the patched text before
    /// variable interpolation, `edits` the apply's journal and `removed` what `LineMap::commit`
    /// removed for each of those edits, in the same order. Gives what `trace` gives when
    /// replaying the same journal.
    pub fn from_linemap(text: String, map: &LineMap, edits: Vec<JournalEdit>, removed: Vec<Vec<Removed>>) -> FileTrace {
        let index: HashMap<(usize, u32), usize> = edits.iter().enumerate().map(|(i, e)| ((e.record, e.site), i)).collect();
        let origin = |t: &LineTag| -> LineOrigin {
            let vanilla = (t.vanilla != NO_LINE).then(|| t.vanilla as usize + 1);
            let edit = index.get(&(t.record as usize, t.site)).copied();
            match (t.kind, edit) {
                (LineKind::Inserted, Some(edit)) => LineOrigin::Inserted { edit },
                (LineKind::Kept, Some(edit)) => LineOrigin::Kept { edit, vanilla },
                (LineKind::Modified, Some(edit)) => LineOrigin::Modified { edit, vanilla },
                _ => LineOrigin::Vanilla(vanilla.unwrap_or(0)),
            }
        };
        let lines: Vec<LineOrigin> = map.tags().iter().map(origin).collect();
        let mut removed: Vec<Vec<RemovedLine>> = removed
            .iter()
            .map(|gone| gone.iter().map(|r| RemovedLine { origin: origin(&r.tag), kept: r.kept }).collect())
            .collect();
        if removed.len() != edits.len() {
            log::warn!("removed lines out of step with the journal ({} for {} edits)", removed.len(), edits.len());
            removed.resize(edits.len(), Vec::new());
        }
        let mut lines = lines;
        let total = line_count(&text);
        if lines.len() != total {
            log::warn!("line origins out of step ({} for {} lines); padding", lines.len(), total);
            lines.resize(total, LineOrigin::Vanilla(0));
        }
        FileTrace { text, lines, edits, removed, raw_text: None }
    }
}

impl FileTrace {
    /// The lines patterns match: `raw_text` when it has the same lines as `text`, else `text`.
    pub fn raw_lines(&self) -> Vec<&str> {
        let lines: Vec<&str> = self.text.lines().collect();
        match self.raw_text.as_deref().map(|r| r.lines().collect::<Vec<&str>>()) {
            Some(raw) if raw.len() == lines.len() => raw,
            _ => lines,
        }
    }
}

fn line_count(text: &str) -> usize {
    text.lines().count()
}

/// The 0-based line holding byte `b` (a byte at the end of the text belongs to the last line,
/// or to a new line when the text ends with a newline).
fn line_of(text: &str, b: usize) -> usize {
    let head = text.as_bytes().get(..b.min(text.len())).unwrap_or_default();
    head.iter().filter(|&&c| c == b'\n').count()
}

fn line_start(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut seen = 0;
    for (i, c) in text.bytes().enumerate() {
        if c == b'\n' {
            seen += 1;
            if seen == line {
                return i + 1;
            }
        }
    }
    text.len()
}

/// A line's owner in the replay (D50): the record and site of the edit that wrote it whole
/// (inserted or kept it), or that first changed a buffer line inside. `LineTag.owner` is the
/// patch of that record.
pub type Owner = Option<(usize, u32)>;

/// Replays `edits` over `original` and returns every final line's origin.
pub fn trace(original: &str, edits: Vec<JournalEdit>) -> FileTrace {
    replay(original, edits).0
}

/// `trace`, plus every final line's owner: the lines an edit writes whole (new or kept) are its
/// own; a line it changes inside keeps the owner it had, and a buffer line gets the edit.
pub fn trace_owners(original: &str, edits: Vec<JournalEdit>) -> (FileTrace, Vec<Owner>) {
    let (trace, owners, _) = replay(original, edits);
    (trace, owners)
}

/// `trace_owners`, plus every final line's maker (`LineTag.maker`): the edit that first made it
/// non-vanilla, which a later edit that keeps the line does not change.
pub fn trace_makers(original: &str, edits: Vec<JournalEdit>) -> (FileTrace, Vec<Owner>, Vec<Owner>) {
    replay(original, edits)
}

fn replay(original: &str, edits: Vec<JournalEdit>) -> (FileTrace, Vec<Owner>, Vec<Owner>) {
    let mut text = original.to_string();
    let mut lines: Vec<LineOrigin> = (1..=line_count(original)).map(LineOrigin::Vanilla).collect();
    let mut owners: Vec<Owner> = vec![None; lines.len()];
    let mut makers: Vec<Owner> = vec![None; lines.len()];
    let mut removed_all = Vec::with_capacity(edits.len());

    for (ei, e) in edits.iter().enumerate() {
        let start = e.start.min(text.len());
        let end = start.saturating_add(e.removed).min(text.len());
        let k = line_of(&text, start);
        let Some(removed_text) = text.get(start..end) else {
            log::warn!("journal edit {ei} is not on character boundaries; skipped");
            removed_all.push(Vec::new());
            continue;
        };
        let aligned = start == line_start(&text, k)
            && (removed_text.is_empty() || removed_text.ends_with('\n'))
            && (e.inserted.is_empty() || e.inserted.ends_with('\n'));

        // A newline-led insert at the end of the text is whole lines too (D53).
        let terminated = text.is_empty() || text.ends_with('\n');
        let end_new = if aligned { None } else { end_lines(start == text.len() && end == start, terminated, &e.inserted) };

        let removed: Vec<RemovedLine>;
        let new_origins: Vec<LineOrigin>;
        let new_owners: Vec<Owner>;
        let new_makers: Vec<Owner>;
        let mine = Some((e.record, e.site));
        let (from, to);
        if let Some(n) = end_new {
            new_origins = vec![LineOrigin::Inserted { edit: ei }; n];
            new_owners = vec![mine; n];
            new_makers = vec![mine; n];
            removed = Vec::new();
            from = lines.len();
            to = lines.len();
        } else if aligned {
            let n_removed = removed_text.matches('\n').count();
            let old_lines: Vec<&str> = removed_text.lines().collect();
            let old_origins: Vec<LineOrigin> =
                lines.get(k.min(lines.len())..(k + n_removed).min(lines.len())).map(<[LineOrigin]>::to_vec).unwrap_or_default();
            let mut kept = vec![false; old_origins.len()];
            let new_lines: Vec<&str> = e.inserted.lines().collect();
            let mut kept_of: Vec<Option<usize>> = vec![None; new_lines.len()];
            for (j, i) in kept_pairs(&old_lines, &new_lines) {
                if let (Some(slot), Some(flag)) = (kept_of.get_mut(j as usize), kept.get_mut(i as usize)) {
                    *slot = Some(i as usize);
                    *flag = true;
                }
            }
            new_origins = kept_of
                .iter()
                .map(|slot| match slot.and_then(|i| old_origins.get(i)) {
                    Some(o) => LineOrigin::Kept { edit: ei, vanilla: o.vanilla() },
                    None => LineOrigin::Inserted { edit: ei },
                })
                .collect();
            // Whole lines, kept ones included, are the edit's (§4.3 item 1).
            new_owners = vec![mine; kept_of.len()];
            let old_makers: Vec<Owner> =
                makers.get(k.min(makers.len())..(k + n_removed).min(makers.len())).map(<[Owner]>::to_vec).unwrap_or_default();
            new_makers = kept_of
                .iter()
                .map(|slot| match slot.and_then(|i| old_makers.get(i)) {
                    Some(&o) => o.or(mine),
                    None => mine,
                })
                .collect();
            removed = old_origins.iter().zip(&kept).map(|(&origin, &kept)| RemovedLine { origin, kept }).collect();
            from = k.min(lines.len());
            to = (k + n_removed).min(lines.len());
        } else {
            // Inside a line: every line the edit touches becomes Modified.
            let old_total = lines.len();
            let m = if e.removed > 0 { line_of(&text, end.saturating_sub(1)) } else { k };
            let first = k.min(old_total);
            let last_excl = (m + 1).min(old_total).max(first);
            let suffix = old_total - last_excl;
            let mut next = String::with_capacity(text.len() + e.inserted.len());
            next.push_str(text.get(..start).unwrap_or_default());
            next.push_str(&e.inserted);
            next.push_str(text.get(end..).unwrap_or_default());
            let middle = line_count(&next).saturating_sub(first + suffix);
            let vanilla = lines.get(first).and_then(|o| o.vanilla());
            removed = lines
                .get(first..last_excl)
                .unwrap_or_default()
                .iter()
                .map(|&origin| RemovedLine { origin, kept: false })
                .collect();
            new_origins = (0..middle).map(|_| LineOrigin::Modified { edit: ei, vanilla }).collect();
            let keep = |of: &[Owner]| -> Vec<Owner> {
                (0..middle)
                    .map(|i| match of.get(first + i).filter(|_| first + i < last_excl) {
                        Some(&o) => o.or(mine),
                        None => mine,
                    })
                    .collect()
            };
            new_owners = keep(&owners);
            new_makers = keep(&makers);
            from = first;
            to = last_excl;
        }
        lines.splice(from..to.max(from), new_origins);
        let (of, ot) = (from.min(owners.len()), to.max(from).min(owners.len()));
        owners.splice(of..ot, new_owners);
        let (mf, mt) = (from.min(makers.len()), to.max(from).min(makers.len()));
        makers.splice(mf..mt, new_makers);
        // `start..end` was checked by `text.get` above.
        text.replace_range(start..end, &e.inserted);
        removed_all.push(removed);
    }

    // Belt and braces: the origin list always matches the text it describes.
    let total = line_count(&text);
    if lines.len() != total {
        log::warn!("line origins out of step ({} for {} lines); padding", lines.len(), total);
        lines.resize(total, LineOrigin::Vanilla(0));
    }
    owners.resize(total, None);
    makers.resize(total, None);
    (FileTrace { text, lines, edits, removed: removed_all, raw_text: None }, owners, makers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(id: &str, start: usize, removed: usize, inserted: &str) -> JournalEdit {
        JournalEdit {
            patch_id: id.into(),
            patch_name: id.into(),
            mod_id: format!("m-{id}"),
            record: 0,
            start,
            removed,
            inserted: inserted.into(),
            site: 0,
            block: crate::patch::linemap::NO_BLOCK,
        }
    }

    #[test]
    fn insertions_replacements_and_kept_lines() {
        let src = "a\nb\nc\n";
        // p1 inserts after a; p2 replaces b with "B2\nb" (keeps b, adds B2).
        let t = trace(src, vec![edit("p1", 2, 0, "x\n"), edit("p2", 4, 2, "B2\nb\n")]);
        assert_eq!(t.text, "a\nx\nB2\nb\nc\n");
        assert_eq!(
            t.lines,
            vec![
                LineOrigin::Vanilla(1),
                LineOrigin::Inserted { edit: 0 },
                LineOrigin::Inserted { edit: 1 },
                LineOrigin::Kept { edit: 1, vanilla: Some(2) },
                LineOrigin::Vanilla(3),
            ]
        );
        assert_eq!(t.removed[1], vec![RemovedLine { origin: LineOrigin::Vanilla(2), kept: true }]);
        assert_eq!(t.runs_of("p2"), vec![(3, 4)]);
        assert_eq!(t.edit_at(2).unwrap().patch_id, "p1");
    }

    #[test]
    fn a_later_patch_replacing_another_patchs_line_removes_its_output() {
        let src = "t.console = false\n";
        let t = trace(src, vec![edit("alpha", 0, 18, "t.console = true\n"), edit("beta", 0, 17, "t.console = 1\n")]);
        assert_eq!(t.lines, vec![LineOrigin::Inserted { edit: 1 }]);
        assert_eq!(t.removed[1], vec![RemovedLine { origin: LineOrigin::Inserted { edit: 0 }, kept: false }]);
    }

    #[test]
    fn edits_inside_a_line_mark_it_modified() {
        let src = "local x = 1\nlocal y = 2\n";
        // A regex replaces "1" with "10" on line 1.
        let t = trace(src, vec![edit("re", 10, 1, "10")]);
        assert_eq!(t.text, "local x = 10\nlocal y = 2\n");
        assert_eq!(t.lines, vec![LineOrigin::Modified { edit: 0, vanilla: Some(1) }, LineOrigin::Vanilla(2)]);
    }

    #[test]
    fn a_newline_led_append_at_the_end_is_whole_lines() {
        // D53: a copy append's bytes on a file that ends with a newline, and on one that does
        // not: the new lines are Inserted and the last line stays as it was.
        let t = trace("a\n", vec![edit("tail", 2, 0, "\nc")]);
        assert_eq!(t.text, "a\n\nc");
        assert_eq!(t.lines, vec![LineOrigin::Vanilla(1), LineOrigin::Inserted { edit: 0 }, LineOrigin::Inserted { edit: 0 }]);
        let t = trace("a\nb", vec![edit("tail", 3, 0, "\nc\nd")]);
        assert_eq!(t.text, "a\nb\nc\nd");
        assert_eq!(
            t.lines,
            vec![LineOrigin::Vanilla(1), LineOrigin::Vanilla(2), LineOrigin::Inserted { edit: 0 }, LineOrigin::Inserted { edit: 0 }]
        );
        assert!(t.removed[0].is_empty());
        // Not at the end: inside the line, as before.
        let t = trace("a\nb", vec![edit("mid", 1, 0, "\nc")]);
        assert_eq!(t.lines[0], LineOrigin::Modified { edit: 0, vanilla: Some(1) });
    }

    #[test]
    fn origins_always_match_the_line_count() {
        let src = "a\nb";
        // Append without a trailing newline, then a prepend.
        let t = trace(src, vec![edit("tail", 3, 0, "\nc"), edit("head", 0, 0, "z\n")]);
        assert_eq!(t.text, "z\na\nb\nc");
        assert_eq!(t.lines.len(), 4);
        assert_eq!(t.lines[0], LineOrigin::Inserted { edit: 1 });
    }
}
