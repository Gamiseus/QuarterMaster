#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
//! Edits mode for `at` patches (D46, D47). An `at` patch's pattern lines are compared with its
//! payload lines (trimmed, by LCS). Lines both have are kept: they stay in the file untouched.
//! The other payload lines become inserts next to a kept line, changes or removals. The
//! trimmed lines of the result are the same as a literal run's.
//!
//! The DLL and the simulation compute this once per patch in `PatchTable::prepare`. A patch
//! runs in edits mode only when its author, a compat mod or a profile fix asks for it.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::lua_scan;
use crate::patch::fingerprint::PatchFingerprintLite;

/// How an `at` patch runs. Absent means literal.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ApplyAs {
    /// The whole match is removed and the payload written in its place (today's rule).
    #[default]
    Literal,
    /// Separate edits: inserts next to the kept lines, changes and removals.
    Edits,
}

/// One edit of the normalized form. `p` indexes pattern lines, `q` payload lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hunk {
    /// Pattern line `p` is payload line `q`: the line stays as it is.
    Keep { p: usize, q: usize },
    /// Payload lines `q` go after kept pattern line `after_p`, or before pattern line 0 (a
    /// kept line) when `None`.
    Insert {
        after_p: Option<usize>,
        q: Range<usize>,
    },
    /// Pattern lines `p` are replaced by payload lines `q`.
    Change { p: Range<usize>, q: Range<usize> },
    /// Pattern lines `p` are removed.
    Delete { p: Range<usize> },
}

/// Why a patch that asks for edits mode runs literal.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditsRefusal {
    /// More than one way to pair the lines (repeated lines such as `end`).
    Ambiguous,
    /// A new line next to a kept line opens or closes a block.
    Scope,
    /// A kept or new line is inside a long string or long comment.
    LongString,
    /// The pattern and payload are too long to compare.
    TooLarge,
    /// Another mod's new lines sit between lines one change or removal works on (D48). Only
    /// that site is refused, and nothing is written there.
    SplitChange,
}

impl EditsRefusal {
    /// The code in `edits_refused:<code>`.
    pub fn code(self) -> &'static str {
        match self {
            EditsRefusal::Ambiguous => "ambiguous",
            EditsRefusal::Scope => "scope",
            EditsRefusal::LongString => "long_string",
            EditsRefusal::TooLarge => "too_large",
            EditsRefusal::SplitChange => "split_change",
        }
    }

    /// Why, in plain words.
    pub fn why(self) -> &'static str {
        match self {
            EditsRefusal::Ambiguous => "the lines are ambiguous",
            EditsRefusal::Scope => "a new line next to a kept line opens or closes a block",
            EditsRefusal::LongString => "a line is inside a long string or comment",
            EditsRefusal::TooLarge => "the pattern and payload are too long to compare",
            EditsRefusal::SplitChange => "another mod's code sits between lines it changes",
        }
    }
}

/// The normalized form of one `at` patch, or why it has none.
pub type Normalized = Result<Vec<Hunk>, EditsRefusal>;

/// Warning code on a record whose edits mode was refused (the patch, or one site, ran literal).
pub const EDITS_REFUSED: &str = "edits_refused";

/// The most pattern-by-payload cells the comparison builds (about 6 MB).
pub const MAX_CELLS: usize = 1 << 20;

/// The `edits_refused:<reason>` warning; `site` names the match when only it ran literal.
pub fn refused_warning(reason: EditsRefusal, site: Option<String>) -> String {
    match site {
        Some(site) => format!(
            "{EDITS_REFUSED}:{}: {site} runs as one replacement: {}",
            reason.code(),
            reason.why()
        ),
        None => format!(
            "{EDITS_REFUSED}:{}: runs as one replacement: {}",
            reason.code(),
            reason.why()
        ),
    }
}

/// The `edits_refused:<reason>` warning for a match that is skipped, not run literal: its
/// lines are split by another mod's code (D48).
pub fn skipped_warning(reason: EditsRefusal, site: &str) -> String {
    format!(
        "{EDITS_REFUSED}:{}: {site} is skipped: {}",
        reason.code(),
        reason.why()
    )
}

/// One edit of an `at` patch, named so a profile fix or a compat mod can skip it (D49).
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct HunkKey {
    /// The patch's content when the key was made; with other content nothing is skipped.
    pub fp: PatchFingerprintLite,
    /// The edit's pattern lines, trimmed and joined by `\n` (empty for an insert).
    pub vanilla: String,
    /// The edit's place among the patch's edits, from 0 (kept lines do not count).
    pub ordinal: u32,
    /// The patch whose change to these lines stays ("mod:patch" or a patch id). Its new lines
    /// stand in for the skipped lines when the patch looks for its place. Absent: any other
    /// patch's lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub keep: Option<String>,
}

/// Whose lines stand in for a skipped edit's pattern lines when the patch looks for its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keeper {
    /// Any other patch's (the skip names none).
    Any,
    /// This table index's.
    Patch(u32),
    /// No patch's: the one it names is not loaded.
    Gone,
}

/// One skipped edit of a patch, ready for its apply (D49).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkipAt {
    pub ordinal: u32,
    pub vanilla: String,
    pub keep: Keeper,
    /// The kept patch's name, for the warning.
    pub keep_name: Option<String>,
}

/// Warning code on a record that skipped one of its edits (D49).
pub const HUNK_SKIPPED: &str = "hunk_skipped";

/// The edits of the hunks (kept lines left out), each with its ordinal.
pub fn edit_hunks(hunks: &[Hunk]) -> impl Iterator<Item = (u32, &Hunk)> {
    hunks
        .iter()
        .filter(|h| !matches!(h, Hunk::Keep { .. }))
        .enumerate()
        .map(|(i, h)| (u32::try_from(i).unwrap_or(u32::MAX), h))
}

/// The pattern lines a change or removal works on, trimmed and joined by `\n`; empty for an
/// insert.
pub fn hunk_vanilla(pattern: &str, hunk: &Hunk) -> String {
    let p = match hunk {
        Hunk::Change { p, .. } | Hunk::Delete { p } => p.clone(),
        _ => return String::new(),
    };
    pattern
        .lines()
        .skip(p.start)
        .take(p.len())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether edit `ordinal` of an `at` patch with this pattern and payload still works on
/// `vanilla`; else why not, in plain words.
pub fn find_hunk(pattern: &str, payload: &str, ordinal: u32, vanilla: &str) -> Result<(), String> {
    let hunks = normalize_patch(pattern, payload)
        .map_err(|r| format!("it no longer runs as separate edits ({})", r.why()))?;
    let same = edit_hunks(&hunks)
        .find(|(o, _)| *o == ordinal)
        .is_some_and(|(_, h)| hunk_vanilla(pattern, h) == vanilla);
    if same {
        Ok(())
    } else {
        Err("the change it skips is no longer there".to_string())
    }
}

/// The `hunk_skipped:<ordinal>` warning of a skipped edit; `keep` names the patch whose
/// change stays.
pub fn skipped_hunk_warning(ordinal: u32, vanilla: &str, keep: Option<&str>) -> String {
    let what = match vanilla.lines().next() {
        Some(line) => format!("the change to '{line}'"),
        None => "a new line".to_string(),
    };
    match keep {
        Some(k) => format!("{HUNK_SKIPPED}:{ordinal}: {what} is skipped; {k}'s change stays"),
        None => format!("{HUNK_SKIPPED}:{ordinal}: {what} is skipped"),
    }
}

/// Whether the hunks only keep lines and add new ones: such an `at` patch can take rungs
/// (§4.7).
pub fn inserts_only(hunks: &[Hunk]) -> bool {
    hunks
        .iter()
        .all(|h| matches!(h, Hunk::Keep { .. } | Hunk::Insert { .. }))
}

/// Whether the hunks can run on a window whose matched lines are `lines` (pattern line `p` is
/// line `lines[p]`): each change and removal works on lines with nothing between them (D48).
pub fn split_ok(hunks: &[Hunk], lines: &[usize]) -> bool {
    hunks.iter().all(|h| {
        let p = match h {
            Hunk::Change { p, .. } | Hunk::Delete { p } => p,
            _ => return true,
        };
        match (lines.get(p.start), p.end.checked_sub(1).and_then(|e| lines.get(e))) {
            (Some(&a), Some(&b)) => b.checked_sub(a) == Some(p.len().saturating_sub(1)),
            _ => false,
        }
    })
}

/// The text a literal `at` writes: each payload line with `indent` in front, ending in a
/// newline (an empty payload is one empty line).
pub fn literal_payload(payload: &str, indent: &str) -> String {
    let mut out = String::with_capacity(payload.len() + 1);
    for line in payload.split_inclusive('\n') {
        out.push_str(indent);
        out.push_str(line);
    }
    if !payload.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// The normalized form of an `at` patch with this pattern and payload.
pub fn normalize_patch(pattern: &str, payload: &str) -> Normalized {
    let lit = literal_payload(payload, "");
    let q: Vec<&str> = lit.split_inclusive('\n').collect();
    let p: Vec<&str> = pattern.lines().collect();
    normalize(&p, &q)
}

/// Whether a pattern line has a wildcard (never kept, D47).
fn wild(line: &str) -> bool {
    line.contains(['*', '?'])
}

fn add(a: u8, b: u8) -> u8 {
    a.saturating_add(b).min(2)
}

/// Compares the pattern lines with the payload lines (D47). Never panics.
pub fn normalize(pattern: &[&str], payload: &[&str]) -> Normalized {
    let p: Vec<&str> = pattern.iter().map(|l| l.trim()).collect();
    let q: Vec<&str> = payload.iter().map(|l| l.trim()).collect();
    let (n, m) = (p.len(), q.len());
    let w = m + 1;
    let cells = (n + 1)
        .checked_mul(w)
        .filter(|&c| c <= MAX_CELLS)
        .ok_or(EditsRefusal::TooLarge)?;
    let keep = |i: usize, j: usize| match (p.get(i), q.get(j)) {
        (Some(a), Some(b)) => !wild(a) && a == b,
        _ => false,
    };
    // Suffix tables: the LCS length of p[i..], q[j..]; how many optimal pairings it has
    // (capped at 2: only "one or more" matters, so repeated lines never overflow); and how
    // many of them pair p[i].
    let mut len = vec![0u32; cells];
    let mut all = vec![1u8; cells];
    let mut first = vec![0u8; cells];
    let at = |v: &[u32], i: usize, j: usize| v.get(i * w + j).copied().unwrap_or(0);
    let at8 = |v: &[u8], i: usize, j: usize| v.get(i * w + j).copied().unwrap_or(0);
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let diag = at(&len, i + 1, j + 1);
            let down = at(&len, i + 1, j);
            let right = at(&len, i, j + 1);
            let pair = keep(i, j);
            let best = down.max(right).max(if pair { diag + 1 } else { 0 });
            let mut f = 0u8;
            if pair && diag + 1 == best {
                f = add(f, at8(&all, i + 1, j + 1));
            }
            if right == best {
                f = add(f, at8(&first, i, j + 1));
            }
            let mut a = f;
            if down == best {
                a = add(a, at8(&all, i + 1, j));
            }
            let k = i * w + j;
            if let Some(x) = len.get_mut(k) {
                *x = best;
            }
            if let Some(x) = all.get_mut(k) {
                *x = a;
            }
            if let Some(x) = first.get_mut(k) {
                *x = f;
            }
        }
    }
    if at8(&all, 0, 0) > 1 {
        return Err(EditsRefusal::Ambiguous);
    }

    // The one pairing, top to bottom.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        let here = at(&len, i, j);
        if keep(i, j) && at(&len, i + 1, j + 1) + 1 == here {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if at(&len, i + 1, j) == here {
            i += 1;
        } else {
            j += 1;
        }
    }

    let mut hunks = Vec::new();
    let (mut pi, mut qj) = (0usize, 0usize);
    let mut above: Option<usize> = None;
    for &(i, j) in pairs.iter().chain(std::iter::once(&(n, m))) {
        let (gp, gq) = (pi..i, qj..j);
        match (gp.is_empty(), gq.is_empty()) {
            (false, false) => hunks.push(Hunk::Change { p: gp, q: gq }),
            (true, false) => hunks.push(Hunk::Insert {
                after_p: above,
                q: gq,
            }),
            (false, true) => hunks.push(Hunk::Delete { p: gp }),
            (true, true) => {}
        }
        if i < n {
            hunks.push(Hunk::Keep { p: i, q: j });
            above = Some(i);
        }
        pi = i + 1;
        qj = j + 1;
    }

    // A kept line, or a line next to one, inside a long string or comment of the payload
    // would change bytes Lua reads.
    let joined: String = payload
        .iter()
        .map(|l| {
            if l.ends_with('\n') {
                (*l).to_string()
            } else {
                format!("{l}\n")
            }
        })
        .collect();
    let scan = lua_scan::scan(&joined);
    let starts_long = |k: usize| scan.lines.get(k).is_some_and(|l| l.starts_in_long);
    for h in &hunks {
        if let Hunk::Keep { q, .. } = h {
            if starts_long(*q) || starts_long(q + 1) {
                return Err(EditsRefusal::LongString);
            }
        }
    }

    // New lines next to a kept line must not open or close a block (the los wrapper).
    for (k, h) in hunks.iter().enumerate() {
        let Hunk::Insert { after_p, q } = h else {
            continue;
        };
        let lines = payload.get(q.clone()).unwrap_or_default();
        let text: String = lines.concat();
        let kept_below = matches!(hunks.get(k + 1), Some(Hunk::Keep { .. }));
        let first_moves =
            after_p.is_some() && lines.first().is_some_and(|l| !lua_scan::block_balanced(l));
        let last_moves = kept_below && lines.last().is_some_and(|l| !lua_scan::block_balanced(l));
        if !lua_scan::block_balanced(&text) || first_moves || last_moves {
            return Err(EditsRefusal::Scope);
        }
    }
    Ok(hunks)
}

/// Whether a site over lines `lines` of a scanned text may run in edits mode: no line of it,
/// and not the line right after it, starts inside a long string or comment (D47).
pub fn site_in_long(scan: &lua_scan::Scan, lines: Range<usize>) -> bool {
    (lines.start..=lines.end).any(|l| scan.lines.get(l).is_some_and(|i| i.starts_in_long))
}

/// What the hunks do, counted in lines: (kept, inserted, changed, removed).
pub fn counts(hunks: &[Hunk]) -> (usize, usize, usize, usize) {
    let mut c = (0, 0, 0, 0);
    for h in hunks {
        match h {
            Hunk::Keep { .. } => c.0 += 1,
            Hunk::Insert { q, .. } => c.1 += q.len(),
            Hunk::Change { p, .. } => c.2 += p.len(),
            Hunk::Delete { p } => c.3 += p.len(),
        }
    }
    c
}

#[cfg(test)]
#[path = "edits_tests.rs"]
mod tests;
