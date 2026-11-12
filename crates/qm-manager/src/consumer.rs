//! Exact consumer detection (Phase C §9.1): for a pattern patch that lost some of the places
//! it matches in the game's file, which patch took each place, and how.
//!
//! The places are the patch's matches in the prepared file before any patch ran. The text just
//! before the patch ran is replayed from the run's journal (`origin::trace` over the edits of
//! the earlier records); a place the patch did not match there is lost, and the lines of the
//! place tell what happened to it. Only the simulation reads this; the game's output is
//! unchanged.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use wildmatch::WildMatch;

use qm_core::handshake::{PatchExecutionRecord, PatchOutcomeStatus};
use qm_core::patch::fingerprint::site_context;
use qm_core::patch::linemap::{match_filter, match_filter_lines};
use qm_core::patch::origin::{self, FileTrace, JournalEdit, LineOrigin};
use qm_core::patch::edits;
use qm_core::patch::pattern::{find_split_windows, Window};
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, Patch, PatternPatch, RegexPatch};
use qm_core::prepare::PreparedBuffer;

/// How another patch took one of a patch's places.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerKind {
    /// The lines are all there, but code from the consumer now sits between them.
    Split,
    /// The consumer wrote the line back unchanged, but not the rest of the place.
    Reemitted,
    /// The consumer replaced the line with one new line.
    #[serde(rename = "rewritten_1to1")]
    Rewritten1to1,
    /// The consumer replaced the line with a block of new lines.
    ReplacedByBlock,
    /// The consumer removed the line and wrote nothing in its place.
    Deleted,
}

/// The patch that took a place, and what the line was and became.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerInfo {
    pub patch_id: String,
    pub mod_id: String,
    pub kind: ConsumerKind,
    /// 1-based line of the place's anchor line in the game's file, as the file shows it.
    pub site_line: u32,
    /// The anchor line as the game has it.
    pub old_text: String,
    /// What is there now: the new line, the new block, the code in between (Split), or the
    /// line written back (Reemitted). None when the line was removed.
    pub new_text: Option<String>,
}

/// A rung that would put a patch's code back next to another patch's copy of its line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RungHint {
    /// The patch the fix would anchor, and its mod.
    pub patch_id: String,
    pub mod_id: String,
    /// The patch whose code it goes next to.
    pub within: String,
    pub within_mod: String,
    /// The line to find inside `within`'s code (trimmed, matched exactly).
    pub pattern: String,
    /// The site key of the anchored patch's place.
    pub context: String,
    /// The anchored patch has one place in the game's file.
    pub single_site: bool,
    /// The anchored patch's position.
    pub position: InsertPosition,
}

/// One lost place of a pattern patch (not sent to the UI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteLoss {
    pub target_file: String,
    /// Index of the patch's record in the file's records.
    pub record: usize,
    pub patch_id: String,
    pub mod_id: String,
    /// Index of the place among the patch's matches in the game's file.
    pub site: u32,
    /// The patch's places in the game's file (at most `times`), and how many it lost.
    pub sites_total: u32,
    pub sites_lost: u32,
    pub position: InsertPosition,
    /// None: the place is gone but no other patch took it (an overlap with another match).
    pub consumer: Option<ConsumerInfo>,
    pub consumer_record: Option<usize>,
    /// Which of the consumer's places (its edit's site) took this one.
    pub consumer_site: Option<u32>,
    /// The site key of this place (§3.7 context), for a fix on it.
    pub context: String,
    pub rung: Option<RungHint>,
    /// The patch could run as separate edits and find this place again (§4.8): its lines are
    /// all there, split by another patch's code.
    pub apply_as: bool,
}

/// The prepared file's lines with an index by trimmed text, so a pattern's places are found
/// without a scan when its first line has no wildcard.
struct Original<'a> {
    buf: &'a PreparedBuffer,
    lines: Vec<&'a str>,
    by_text: HashMap<&'a str, Vec<usize>>,
}

impl<'a> Original<'a> {
    fn new(buf: &'a PreparedBuffer) -> Self {
        let lines: Vec<&str> = buf.text.lines().collect();
        let mut by_text: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, l) in lines.iter().enumerate() {
            by_text.entry(l.trim()).or_default().push(i);
        }
        Original {
            buf,
            lines,
            by_text,
        }
    }

    /// The windows `pattern` matches, as `find_windows` finds them. Windows over
    /// QuarterMaster's lines are refused as the engine refuses them (`match_filter`, D42);
    /// `removes` is true for an `at` patch. With `exact` (an exact copy, D59) a line matches
    /// only its own text.
    fn windows(&self, pattern: &str, removes: bool, exact: bool) -> Vec<Window> {
        let pat: Vec<&str> = pattern.lines().map(str::trim).collect();
        let Some(first) = pat.first() else {
            return Vec::new();
        };
        let wm: Vec<WildMatch> = pat.iter().map(|l| WildMatch::new(l)).collect();
        let literal = exact || !first.contains(['*', '?']);
        let hit = |l: &str, k: usize| {
            if exact {
                pat.get(k).is_some_and(|p| l.trim() == *p)
            } else {
                wm.get(k).is_some_and(|m| m.matches(l.trim()))
            }
        };
        let all: Vec<usize>;
        let cands: &[usize] = if literal {
            self.by_text.get(first).map_or(&[], Vec::as_slice)
        } else {
            all = (0..self.lines.len()).collect();
            &all
        };
        let len = pat.len();
        let mut out = Vec::new();
        let mut free = 0usize;
        for &i in cands {
            if i < free {
                continue;
            }
            let Some(window) = self.lines.get(i..i + len) else {
                continue;
            };
            if window.iter().enumerate().all(|(k, l)| hit(l, k))
                && match_filter(self.buf, i, len, removes)
            {
                let indent = window
                    .first()
                    .map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect())
                    .unwrap_or_default();
                out.push(Window::contiguous(i, len, indent));
                free = i + len;
            }
        }
        out
    }
}

/// The text just before one record ran, with where each line came from.
struct Before {
    trace: FileTrace,
    /// Lines of `trace.text`.
    lines: Vec<String>,
    /// 1-based prepared line -> index in `lines`, for lines that still stand for it.
    at: HashMap<usize, usize>,
}

impl Before {
    fn new(original: &str, edits: Vec<JournalEdit>) -> Self {
        let trace = origin::trace(original, edits);
        let lines: Vec<String> = trace.text.lines().map(str::to_string).collect();
        let mut at = HashMap::new();
        for (i, o) in trace.lines.iter().enumerate() {
            if matches!(o, LineOrigin::Inserted { .. }) {
                continue;
            }
            if let Some(v) = o.vanilla() {
                at.entry(v).or_insert(i);
            }
        }
        Before { trace, lines, at }
    }

    fn record_of(&self, edit: usize) -> Option<usize> {
        self.trace.edits.get(edit).map(|e| e.record)
    }

    fn site_of(&self, edit: usize) -> Option<u32> {
        self.trace.edits.get(edit).map(|e| e.site)
    }

    /// The lines edit `edit` added that are still there.
    fn inserted_by(&self, edit: usize) -> Vec<String> {
        self.trace
            .lines
            .iter()
            .zip(&self.lines)
            .filter(|(o, _)| matches!(o, LineOrigin::Inserted { edit: e } if *e == edit))
            .map(|(_, l)| l.clone())
            .collect()
    }
}

/// What happened to one lost place: the kind, the consumer's record and the new text.
struct Classified {
    kind: ConsumerKind,
    record: usize,
    /// The consumer's site (the edit's `site`).
    site: Option<u32>,
    new_text: Option<String>,
}

/// The line a place's code hangs on: the last line for `after`, else the first.
fn anchor_line(w: &Window, pos: InsertPosition) -> usize {
    match pos {
        InsertPosition::After => w.line + w.len.saturating_sub(1),
        _ => w.line,
    }
}

fn classify(before: &Before, w: &Window, pos: InsertPosition) -> Option<Classified> {
    let vlines: Vec<usize> = (w.line..w.line + w.len).collect();
    let anchor = anchor_line(w, pos);
    let found: Vec<Option<usize>> = vlines
        .iter()
        .map(|v| before.at.get(&(v + 1)).copied())
        .collect();
    let origin_at = |i: usize| before.trace.lines.get(i).copied();

    if found.iter().all(Option::is_some) {
        let pos_of: Vec<usize> = found.iter().flatten().copied().collect();
        // Code between the lines of the place: the first such line names the splitter.
        for pair in pos_of.windows(2) {
            let &[a, b] = pair else { continue };
            if b == a + 1 {
                continue;
            }
            if b < a {
                return None;
            }
            let gap = (a + 1)..b;
            let split_edit = gap
                .clone()
                .find_map(|i| origin_at(i).and_then(|o| o.edit()))?;
            let splitter = before.record_of(split_edit)?;
            let text: Vec<&str> = gap
                .filter_map(|i| before.lines.get(i).map(String::as_str))
                .collect();
            return Some(Classified {
                kind: ConsumerKind::Split,
                record: splitter,
                site: before.site_of(split_edit),
                new_text: Some(text.join("\n")),
            });
        }
        // Contiguous: a line changed in place no longer matches.
        for &i in &pos_of {
            if let Some(LineOrigin::Modified { edit, .. }) = origin_at(i) {
                return Some(Classified {
                    kind: ConsumerKind::Rewritten1to1,
                    record: before.record_of(edit)?,
                    site: before.site_of(edit),
                    new_text: before.lines.get(i).cloned(),
                });
            }
        }
        return None;
    }

    // Some line is gone: the first edit that removed one (without writing it back).
    let missing: Vec<usize> = vlines
        .iter()
        .zip(&found)
        .filter(|(_, f)| f.is_none())
        .map(|(v, _)| *v)
        .collect();
    let want: Vec<usize> = if missing.contains(&anchor) {
        vec![anchor]
    } else {
        missing.clone()
    };
    let remover = before
        .trace
        .removed
        .iter()
        .enumerate()
        .find_map(|(e, gone)| {
            gone.iter()
                .position(|r| {
                    !r.kept
                        && r.origin
                            .vanilla()
                            .and_then(|v| v.checked_sub(1))
                            .is_some_and(|v| want.contains(&v))
                })
                .map(|k| (e, k))
        });

    // The anchor line was written back by the consumer: it is still there to anchor on.
    if let Some(&Some(i)) = vlines
        .iter()
        .position(|&v| v == anchor)
        .and_then(|k| found.get(k))
    {
        if let Some(LineOrigin::Kept { edit, .. }) = origin_at(i) {
            let by = remover.map_or(edit, |(e, _)| e);
            let record = before.record_of(by).or_else(|| before.record_of(edit))?;
            return Some(Classified {
                kind: ConsumerKind::Reemitted,
                record,
                site: before.site_of(by),
                new_text: before.lines.get(i).cloned(),
            });
        }
    }

    let (e, k) = remover?;
    let gone = before.trace.removed.get(e)?;
    let edit = before.trace.edits.get(e)?;
    let removed_new = gone.iter().filter(|r| !r.kept).count();
    let kept = gone.len() - removed_new;
    let inserted_new = edit.inserted.lines().count().saturating_sub(kept);
    let added = before.inserted_by(e);
    let (kind, new_text) = if inserted_new == 0 {
        (ConsumerKind::Deleted, None)
    } else if inserted_new == removed_new {
        // The same slot among the lines it did not write back.
        let slot = gone.iter().take(k).filter(|r| !r.kept).count();
        (ConsumerKind::Rewritten1to1, added.get(slot).cloned())
    } else {
        let text = (!added.is_empty()).then(|| added.join("\n"));
        (ConsumerKind::ReplacedByBlock, text)
    };
    Some(Classified {
        kind,
        record: edit.record,
        site: Some(edit.site),
        new_text,
    })
}

/// A patch whose lost places are found: a pattern, or a regex with its matches as windows
/// (D52).
#[derive(Clone, Copy)]
enum Host<'t> {
    Pattern(&'t PatternPatch),
    Regex(&'t RegexPatch),
}

impl Host<'_> {
    fn position(&self) -> InsertPosition {
        match self {
            Host::Pattern(a) => a.position,
            Host::Regex(a) => a.position,
        }
    }

    fn times(&self) -> Option<usize> {
        match self {
            Host::Pattern(a) => a.times,
            Host::Regex(a) => a.times,
        }
    }

    /// The places in the prepared file, numbered as the engine numbers them (D42).
    fn vanilla(&self, original: &Original) -> Vec<Window> {
        match self {
            Host::Pattern(a) => original.windows(&a.pattern, is_at(a), is_exact(a)),
            Host::Regex(a) => a.site_windows(&original.buf.text, |l, removes| {
                match_filter(original.buf, l.start, l.len(), removes)
            }),
        }
    }

    /// The engine's own matches on the text just before the patch ran; `record` is the
    /// patch's record.
    fn replay(&self, before: &Before, buf: &PreparedBuffer, record: usize) -> Vec<Window> {
        match self {
            Host::Pattern(a) => {
                let now: Vec<&str> = before.lines.iter().map(String::as_str).collect();
                let wm: Vec<WildMatch> = a
                    .pattern
                    .lines()
                    .map(|l| WildMatch::new(l.trim()))
                    .collect();
                replay_windows(before, buf, &now, &wm, a, record)
            }
            Host::Regex(a) => a.site_windows(&before.trace.text, |l, removes| {
                match_filter_lines(buf, l.filter_map(|i| prepared_line(before, i)), removes)
            }),
        }
    }
}

/// The loaded pattern or regex patch of a record.
fn host_of<'t>(table: &'t PatchTable, rec: &PatchExecutionRecord) -> Option<Host<'t>> {
    table.patches.iter().find_map(|(p, _, _)| {
        if p.id() != Some(rec.patch_id.as_str()) || p.mod_id() != Some(rec.mod_id.as_str()) {
            return None;
        }
        match p {
            Patch::Pattern(x) => Some(Host::Pattern(x)),
            Patch::Regex(x) => Some(Host::Regex(x)),
            _ => None,
        }
    })
}

fn pattern_of<'t>(table: &'t PatchTable, rec: &PatchExecutionRecord) -> Option<&'t PatternPatch> {
    table.patches.iter().find_map(|(p, _, _)| match p {
        Patch::Pattern(x)
            if p.id() == Some(rec.patch_id.as_str()) && p.mod_id() == Some(rec.mod_id.as_str()) =>
        {
            Some(x)
        }
        _ => None,
    })
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn is_at(a: &PatternPatch) -> bool {
    a.position == InsertPosition::At
}

/// Whether `a` is an exact copy, whose lines match only their own text (D59).
fn is_exact(a: &PatternPatch) -> bool {
    a.exact_lines().is_some()
}

/// The engine's own match of pattern patch `a` (record `record`) on the text just before it
/// ran (`now`, the lines of `before`): windows over QuarterMaster's lines are refused as the
/// engine refuses them (D42). In edits mode a window skips other patches' new lines, and a
/// split site the engine refuses is refused here too (D48).
fn replay_windows(
    before: &Before,
    buf: &PreparedBuffer,
    now: &[&str],
    wm: &[WildMatch],
    a: &PatternPatch,
    record: usize,
) -> Vec<Window> {
    let removes = is_at(a);
    let normalized = a.edits_mode().then(|| a.normalized());
    let hunks = normalized.as_ref().and_then(|n| n.as_ref().ok());
    let split = a.split_windows() && (!removes || hunks.is_some());
    let long = hunks
        .filter(|_| split)
        .map(|_| qm_core::lua_scan::scan(&before.trace.text));
    let skip = |l: usize| {
        split
            && matches!(
                before.trace.lines.get(l),
                Some(LineOrigin::Inserted { edit }) if before.record_of(*edit) != Some(record)
            )
    };
    find_split_windows(now, wm, a.exact_lines(), skip, |w| {
        let ok = match_filter_lines(
            buf,
            (w.line..w.line + w.len).filter_map(|l| prepared_line(before, l)),
            removes,
        );
        let split_refused = w.lines.len() != w.len
            && hunks.is_some_and(|h| {
                !edits::split_ok(h, &w.lines)
                    || long
                        .as_ref()
                        .is_some_and(|s| edits::site_in_long(s, w.line..w.line + w.len))
            });
        ok && !split_refused
    })
}

/// The prepared line (0-based) line `l` of the text before a record stands for; new code
/// stands for none.
fn prepared_line(before: &Before, l: usize) -> Option<usize> {
    match before.trace.lines.get(l)? {
        LineOrigin::Inserted { .. } => None,
        o => o.vanilla()?.checked_sub(1),
    }
}

/// The lost places of every pattern and regex record of one file. `buf` is the prepared file
/// the run started from, `trace` the run's trace in the same numbering, and `file_line` maps a
/// 1-based prepared line to the line the file shows. A regex's places are its matches (D52).
pub(crate) fn find_losses(
    table: &PatchTable,
    target: &str,
    buf: &PreparedBuffer,
    trace: &FileTrace,
    records: &[PatchExecutionRecord],
    file_line: &dyn Fn(usize) -> usize,
) -> Vec<SiteLoss> {
    let original = Original::new(buf);
    let is_builtin = |i: usize| buf.is_builtin(i) || buf.builtin_edited.contains(&i);
    let mut out = Vec::new();
    for (f, rec) in records.iter().enumerate() {
        if !matches!(rec.patch_type.as_str(), "pattern" | "regex")
            || !matches!(
                rec.status,
                PatchOutcomeStatus::Applied
                    | PatchOutcomeStatus::ZeroMatches
                    | PatchOutcomeStatus::CountMismatch
            )
        {
            continue;
        }
        let Some(a) = host_of(table, rec) else {
            continue;
        };
        let vanilla = a.vanilla(&original);
        if vanilla.is_empty() {
            continue;
        }
        // Places a rung put somewhere else count as placed.
        let by_rung: Vec<u32> = rec
            .anchoring
            .iter()
            .flat_map(|r| &r.sites)
            .filter(|s| s.rung.is_some())
            .filter_map(|s| s.vanilla_line)
            .collect();
        if rec.matched_count >= vanilla.len() && rec.anchoring.is_none() {
            continue;
        }
        let edits: Vec<JournalEdit> = trace
            .edits
            .iter()
            .filter(|e| e.record < f)
            .cloned()
            .collect();
        let before = Before::new(&buf.text, edits);
        let matched = a.replay(&before, buf, f);
        let first_lines: Vec<usize> = matched
            .iter()
            .filter_map(|w| before.trace.lines.get(w.line))
            .filter(|o| !matches!(o, LineOrigin::Inserted { .. }))
            .filter_map(LineOrigin::vanilla)
            .collect();
        // With `times`, only the places the patch would use in the game's file count.
        let used = a.times().map_or(vanilla.len(), |t| t.min(vanilla.len()));
        let lost: Vec<(usize, &Window)> = vanilla
            .iter()
            .take(used)
            .enumerate()
            .filter(|(_, w)| {
                // `vanilla_line` is in the file's own numbering.
                !first_lines.contains(&(w.line + 1))
                    && !buf
                        .file_line(w.line)
                        .is_some_and(|l| by_rung.contains(&to_u32(l)))
            })
            .collect();
        if lost.is_empty() {
            continue;
        }
        for &(si, w) in &lost {
            let c = classify(&before, w, a.position());
            let anchor = anchor_line(w, a.position());
            let consumer_rec = c.as_ref().and_then(|c| records.get(c.record));
            let consumer = c.as_ref().zip(consumer_rec).map(|(c, b)| ConsumerInfo {
                patch_id: b.patch_id.clone(),
                mod_id: b.mod_id.clone(),
                kind: c.kind,
                site_line: to_u32(file_line(anchor + 1)),
                old_text: original
                    .lines
                    .get(anchor)
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default(),
                new_text: c.new_text.clone(),
            });
            let rung = c.as_ref().zip(consumer_rec).and_then(|(c, b)| {
                hint(
                    table,
                    &original,
                    &is_builtin,
                    rec,
                    a,
                    w,
                    vanilla.len(),
                    c,
                    b,
                )
            });
            let apply_as = c.as_ref().is_some_and(|c| offers_apply_as(a, c));
            out.push(SiteLoss {
                target_file: target.to_string(),
                record: f,
                patch_id: rec.patch_id.clone(),
                mod_id: rec.mod_id.clone(),
                site: to_u32(si),
                sites_total: to_u32(used),
                sites_lost: to_u32(lost.len()),
                position: a.position(),
                consumer,
                consumer_record: c.as_ref().map(|c| c.record),
                consumer_site: c.as_ref().and_then(|c| c.site),
                context: site_context(&original.lines, is_builtin, w.line, w.len),
                rung,
                apply_as,
            });
        }
    }
    out
}

/// A pattern patch that matched more places just before it ran than in the game's file: code
/// from another mod's patch has a copy of its lines (G9). Not sent to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SiteGain {
    /// Index of the patch's record in the file's records.
    pub record: usize,
    /// The patch's places in the game's file, and just before it ran.
    pub vanilla: u32,
    pub now: u32,
    /// Records of the other mods' patches whose code holds the new places, first one first.
    pub sources: Vec<usize>,
}

/// The gained places of every pattern record of one file. Only patches without rungs that
/// have a place in the game's file and matched more places than it has are checked.
pub(crate) fn find_gains(
    table: &PatchTable,
    buf: &PreparedBuffer,
    trace: &FileTrace,
    records: &[PatchExecutionRecord],
) -> Vec<SiteGain> {
    let original = Original::new(buf);
    let mut out = Vec::new();
    for (f, rec) in records.iter().enumerate() {
        if rec.patch_type != "pattern"
            || rec.anchoring.is_some()
            || !matches!(
                rec.status,
                PatchOutcomeStatus::Applied | PatchOutcomeStatus::CountMismatch
            )
        {
            continue;
        }
        let Some(a) = pattern_of(table, rec) else {
            continue;
        };
        // A patch with no place in the game's file is meant for another mod's code.
        let vanilla = original.windows(&a.pattern, is_at(a), is_exact(a));
        if vanilla.is_empty() || rec.matched_count <= vanilla.len() {
            continue;
        }
        // 1-based first lines of the game's places.
        let starts: Vec<usize> = vanilla.iter().map(|w| w.line + 1).collect();
        let edits: Vec<JournalEdit> = trace
            .edits
            .iter()
            .filter(|e| e.record < f)
            .cloned()
            .collect();
        let before = Before::new(&buf.text, edits);
        let now: Vec<&str> = before.lines.iter().map(String::as_str).collect();
        let wm: Vec<WildMatch> = a
            .pattern
            .lines()
            .map(|l| WildMatch::new(l.trim()))
            .collect();
        let matched = replay_windows(&before, buf, &now, &wm, a, f);
        let mut sources: Vec<usize> = Vec::new();
        for w in &matched {
            let first = before.trace.lines.get(w.line);
            let is_vanilla = first.is_some_and(|o| {
                !matches!(o, LineOrigin::Inserted { .. })
                    && o.vanilla().is_some_and(|v| starts.contains(&v))
            });
            if is_vanilla {
                continue;
            }
            // The first line of the place some patch wrote names where the copy came from.
            let source = (w.line..w.line + w.len)
                .filter_map(|i| before.trace.lines.get(i).and_then(LineOrigin::edit))
                .find_map(|e| before.record_of(e));
            if let Some(s) = source {
                let other = records.get(s).is_some_and(|b| b.mod_id != rec.mod_id);
                if other && !sources.contains(&s) {
                    sources.push(s);
                }
            }
        }
        if sources.is_empty() {
            continue;
        }
        out.push(SiteGain {
            record: f,
            vanilla: to_u32(vanilla.len()),
            now: to_u32(matched.len()),
            sources,
        });
    }
    out
}

/// A pattern patch without `times` that matches more than one place in the game's file (G9,
/// an info note). Not sent to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MultiMatch {
    pub record: usize,
    pub sites: u32,
    /// Distinct functions (the main chunk counts as one) holding the places.
    pub functions: u32,
}

pub(crate) fn find_multi_matches(
    table: &PatchTable,
    target: &str,
    buf: &PreparedBuffer,
    records: &[PatchExecutionRecord],
) -> Vec<MultiMatch> {
    let original = Original::new(buf);
    let mut scan: Option<Option<qm_core::lua_scan::Scan>> = None;
    let mut out = Vec::new();
    for (f, rec) in records.iter().enumerate() {
        if rec.patch_type != "pattern" || rec.status == PatchOutcomeStatus::SkippedDisabled {
            continue;
        }
        let Some(a) = pattern_of(table, rec).filter(|a| a.times.is_none()) else {
            continue;
        };
        let vanilla = original.windows(&a.pattern, is_at(a), is_exact(a));
        if vanilla.len() < 2 {
            continue;
        }
        let scan = scan.get_or_insert_with(|| {
            target
                .ends_with(".lua")
                .then(|| qm_core::lua_scan::scan(&buf.text))
        });
        let functions: std::collections::BTreeSet<usize> = vanilla
            .iter()
            .map(|w| {
                scan.as_ref()
                    .and_then(|s| s.lines.get(w.line))
                    .map_or(0, |l| l.function)
            })
            .collect();
        out.push(MultiMatch {
            record: f,
            sites: to_u32(vanilla.len()),
            functions: to_u32(functions.len()),
        });
    }
    out
}

/// Patches whose code hangs on a line another mod's `at` patch wrote back unchanged (R1 in
/// today's order): the code is there only because that patch keeps the line. Each gets the
/// rung that ties it to that patch's copy, for the combined suggestion (D26).
pub(crate) fn find_kept_anchors(
    table: &PatchTable,
    target: &str,
    buf: &PreparedBuffer,
    trace: &FileTrace,
    records: &[PatchExecutionRecord],
) -> Vec<(String, RungHint)> {
    // Prepared line (1-based) -> the record that wrote it back.
    let mut kept: HashMap<usize, usize> = HashMap::new();
    for o in &trace.lines {
        if let LineOrigin::Kept {
            edit,
            vanilla: Some(v),
        } = *o
        {
            if let Some(e) = trace.edits.get(edit) {
                kept.insert(v, e.record);
            }
        }
    }
    if kept.is_empty() {
        return Vec::new();
    }
    let original = Original::new(buf);
    let is_builtin = |i: usize| buf.is_builtin(i) || buf.builtin_edited.contains(&i);
    let lines = original.lines.as_slice();
    let mut out = Vec::new();
    for rec in records {
        if rec.patch_type != "pattern" || rec.status != PatchOutcomeStatus::Applied {
            continue;
        }
        let Some(a) = pattern_of(table, rec) else {
            continue;
        };
        if a.position == InsertPosition::At {
            continue;
        }
        let vanilla = original.windows(&a.pattern, is_at(a), is_exact(a));
        for w in &vanilla {
            let anchor = anchor_line(w, a.position);
            let Some(b) = kept.get(&(anchor + 1)).and_then(|&r| records.get(r)) else {
                continue;
            };
            if b.mod_id == rec.mod_id {
                continue;
            }
            let Some(text) = lines.get(anchor).map(|l| l.trim().to_string()) else {
                continue;
            };
            out.push((
                target.to_string(),
                RungHint {
                    patch_id: rec.patch_id.clone(),
                    mod_id: rec.mod_id.clone(),
                    within: b.patch_id.clone(),
                    within_mod: b.mod_id.clone(),
                    pattern: text,
                    context: site_context(lines, is_builtin, w.line, w.len),
                    single_site: vanilla.len() == 1,
                    position: a.position,
                },
            ));
        }
    }
    out
}

/// A pattern patch that anchors on another patch's output (G10): `record` matched, or would
/// match, a line that `uses` wrote. Indexes into the file's records. Not sent to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Use {
    pub record: usize,
    pub uses: usize,
}

/// Every anchor on another patch's output in one file. Only pattern patches with no place in
/// the game's file count: their lines are another patch's code.
/// - Applied: the patches whose code holds the lines of its places, just before it ran.
/// - Failed: the patches whose code has every line of its pattern the game's file lacks,
///   before or after it in the order (failure groups count only the ones before it).
pub(crate) fn find_uses(
    table: &PatchTable,
    buf: &PreparedBuffer,
    trace: &FileTrace,
    records: &[PatchExecutionRecord],
) -> Vec<Use> {
    let original = Original::new(buf);
    let mut out = Vec::new();
    for (f, rec) in records.iter().enumerate() {
        // A patch that did not try to match (off, or skipped with its mod or for a
        // requirement) uses nothing.
        if rec.patch_type != "pattern"
            || rec.status == PatchOutcomeStatus::SkippedDisabled
            || crate::failure_groups::skipped(rec)
        {
            continue;
        }
        let Some(a) = pattern_of(table, rec) else {
            continue;
        };
        if !original.windows(&a.pattern, is_at(a), is_exact(a)).is_empty() {
            continue;
        }
        let mut found: Vec<usize> = Vec::new();
        if rec.matched_count > 0 {
            let edits: Vec<JournalEdit> = trace
                .edits
                .iter()
                .filter(|e| e.record < f)
                .cloned()
                .collect();
            let before = Before::new(&buf.text, edits);
            let now: Vec<&str> = before.lines.iter().map(String::as_str).collect();
            let wm: Vec<WildMatch> = a
                .pattern
                .lines()
                .map(|l| WildMatch::new(l.trim()))
                .collect();
            for w in replay_windows(&before, buf, &now, &wm, a, f) {
                for i in w.line..w.line + w.len {
                    let by = before
                        .trace
                        .lines
                        .get(i)
                        .filter(|o| o.is_patch_output())
                        .and_then(LineOrigin::edit)
                        .and_then(|e| before.record_of(e));
                    if let Some(g) = by.filter(|&g| g != f && !found.contains(&g)) {
                        found.push(g);
                    }
                }
            }
        } else {
            let line_hit = line_matcher(a);
            // The pattern's lines the game's file does not have.
            let missing: Vec<usize> = (0..a.pattern.lines().count())
                .filter(|&k| !original.lines.iter().any(|l| line_hit(l, k)))
                .collect();
            if missing.is_empty() {
                continue;
            }
            for (g, other) in records.iter().enumerate() {
                if g == f || other.patch_id == rec.patch_id {
                    continue;
                }
                let code = output_lines(table, other);
                if missing
                    .iter()
                    .all(|&k| code.iter().any(|l| line_hit(l, k)))
                {
                    found.push(g);
                }
            }
        }
        out.extend(found.into_iter().map(|uses| Use { record: f, uses }));
    }
    out
}

/// Whether a line matches line `k` of `a`'s pattern, as the engine matches: trimmed, with
/// wildcards unless `a` is an exact copy (D59).
fn line_matcher(a: &PatternPatch) -> impl Fn(&str, usize) -> bool + '_ {
    let pat: Vec<&str> = a.pattern.lines().map(str::trim).collect();
    let wm: Vec<WildMatch> = pat.iter().map(|l| WildMatch::new(l)).collect();
    let exact = is_exact(a);
    move |l: &str, k: usize| {
        if exact {
            pat.get(k).is_some_and(|p| l.trim() == *p)
        } else {
            wm.get(k).is_some_and(|m| m.matches(l.trim()))
        }
    }
}

/// The lines a record's patch writes: a pattern's payload, or a copy's inline content.
fn output_lines(table: &PatchTable, rec: &PatchExecutionRecord) -> Vec<String> {
    let text = table.patches.iter().find_map(|(p, _, _)| {
        if p.id() != Some(rec.patch_id.as_str()) || p.mod_id() != Some(rec.mod_id.as_str()) {
            return None;
        }
        match p {
            Patch::Pattern(x) => Some(x.payload.as_str()),
            Patch::Copy(x) => x.payload.as_deref(),
            _ => None,
        }
    });
    text.map(|t| t.lines().map(|l| l.trim().to_string()).collect())
        .unwrap_or_default()
}

/// The rung that puts the code back, when there is a safe one:
/// - Reemitted: the patch on the consumer's copy of its anchor line;
/// - Split of an `at` patch: the splitter on the victim's copy of the splitter's line (the
///   victim then runs first, as the rung's order edge says).
#[allow(clippy::too_many_arguments)]
fn hint(
    table: &PatchTable,
    original: &Original,
    is_builtin: &dyn Fn(usize) -> bool,
    rec: &PatchExecutionRecord,
    a: Host,
    w: &Window,
    sites: usize,
    c: &Classified,
    b: &PatchExecutionRecord,
) -> Option<RungHint> {
    let lines = original.lines.as_slice();
    match c.kind {
        // A regex then searches the consumer's copy of the line (D52).
        ConsumerKind::Reemitted if a.position() != InsertPosition::At => {
            let anchor = anchor_line(w, a.position());
            Some(RungHint {
                patch_id: rec.patch_id.clone(),
                mod_id: rec.mod_id.clone(),
                within: b.patch_id.clone(),
                within_mod: b.mod_id.clone(),
                pattern: lines.get(anchor)?.trim().to_string(),
                context: site_context(lines, is_builtin, w.line, w.len),
                single_site: sites == 1,
                position: a.position(),
            })
        }
        ConsumerKind::Split if a.position() == InsertPosition::At => {
            let Host::Pattern(a) = a else {
                return None;
            };
            // The splitter's own place inside the victim's window.
            let s = pattern_of(table, b)?;
            if s.position == InsertPosition::At || s.pattern.lines().count() != 1 {
                return None;
            }
            let theirs = original.windows(&s.pattern, is_at(s), is_exact(s));
            let sw = theirs
                .iter()
                .find(|x| x.line >= w.line && x.line < w.line + w.len)?;
            let text = lines.get(sw.line)?.trim().to_string();
            // The victim must write that line back, or the rung finds nothing.
            let keeps = a.payload.lines().any(|l| l.trim() == text);
            keeps.then(|| RungHint {
                patch_id: b.patch_id.clone(),
                mod_id: b.mod_id.clone(),
                within: rec.patch_id.clone(),
                within_mod: rec.mod_id.clone(),
                pattern: text,
                context: site_context(lines, is_builtin, sw.line, sw.len),
                single_site: theirs.len() == 1,
                position: s.position,
            })
        }
        _ => None,
    }
}

/// Whether "Run <victim> as separate edits" (ApplyAs, D46) finds a split place again (§4.8):
/// a pattern victim that does not run as edits yet, and, for an `at`, whose lines pair one
/// way. Every Split qualifies, whatever the victim's position.
fn offers_apply_as(a: Host, c: &Classified) -> bool {
    let Host::Pattern(a) = a else {
        return false;
    };
    c.kind == ConsumerKind::Split
        && !a.split_windows()
        && (!is_at(a) || a.normalized().is_ok())
}

#[cfg(test)]
#[path = "consumer_tests.rs"]
mod tests;
