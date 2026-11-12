//! Per-line tags of one buffer while it is patched (plan 2e §3.5).
//!
//! Every line knows where it came from (the buffer, QuarterMaster's own lines, or a patch),
//! which record wrote it and which block it belongs to. Inserts form a block tree: an insert
//! hangs on its anchor line, and when that line belongs to a block, that block is its parent.
//! The apply loop commits each patch's edits here, so the regions, the crash attribution and
//! the simulation's line origins come from one place. Built per buffer and then dropped; the
//! DLL keeps only the record of each line.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::collections::{BTreeMap, VecDeque};
use std::ops::Range;

use crop::Rope;

use crate::patch::stack::StackHint;

/// `LineTag.record` of a line no patch wrote.
pub const NO_RECORD: u32 = u32::MAX;
/// `LineTag.block` of a line outside any block, and `BlockInfo.parent` at top level.
pub const NO_BLOCK: u32 = u32::MAX;
/// `LineTag.vanilla` of a line that stands for no line of the buffer.
pub const NO_LINE: u32 = u32::MAX;
/// A patch or mod index that is not known.
pub const NO_INDEX: u32 = u32::MAX;
/// `LineTag.owner` of a line no patch owns (buffer lines and QuarterMaster's own).
pub const NO_PATCH: u32 = u32::MAX;
/// `LineTag.owner_site` of a line no patch owns.
pub const NO_SITE: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    /// A line of the buffer, untouched.
    Vanilla,
    /// A line QuarterMaster added before any patch ran.
    Builtin,
    /// A line of the buffer QuarterMaster rewrote in place.
    BuiltinEdited,
    /// New code from a patch.
    Inserted,
    /// A line an `at` patch removed and wrote back unchanged.
    Kept,
    /// A line an edit changed without working on whole lines.
    Modified,
}

/// `StackSide` in the bindings (the faction `Side` has the plain name).
#[cfg_attr(feature = "specta", derive(specta::Type), specta(rename = "StackSide"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Before,
    After,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineTag {
    pub uid: u32,
    /// Index of the record that last wrote the line, or `NO_RECORD`.
    pub record: u32,
    /// Which of that record's edits wrote it.
    pub site: u32,
    pub kind: LineKind,
    /// `NO_BLOCK` for buffer lines and lines kept from them.
    pub block: u32,
    /// The 0-based buffer line this one stands for, counted as `origin::trace` counts it, or
    /// `NO_LINE` for new code.
    pub vanilla: u32,
    /// The patch that wrote the line whole (inserted it, or kept it with an `at`), or the first
    /// to change a buffer line inside it (D50); `NO_PATCH` for none. A later edit inside the
    /// line changes `record`, never this.
    pub owner: u32,
    /// That patch's site, or `NO_SITE`.
    pub owner_site: u32,
    /// The patch that first made the line non-vanilla (inserted, kept or changed it), or
    /// `NO_PATCH`. Unlike `owner`, an `at` that later keeps the line leaves it: a skipped edit
    /// (D49) finds the kept patch's change by it.
    pub maker: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BlockInfo {
    pub record: u32,
    pub patch_ix: u32,
    pub mod_ix: u32,
    pub site: u32,
    /// The resolved uid of the line the block hangs on, and on which side. `None` for
    /// Replace blocks and inserts with no anchor (copy patches, regex).
    pub anchor: Option<(u32, Side)>,
    /// Always an older block, or `NO_BLOCK` at top level.
    pub parent: u32,
    pub depth: u32,
    /// `anchor` was set by the line rules (D51), not by the patch: chains use it only while
    /// implicit chains are on, and it never counts for coverage.
    pub implicit: bool,
    /// The patch's `stack` key (D57). It stays with the block, so later inserts at its line
    /// never go above a `first` block or below a `last` one.
    pub hint: Option<StackHint>,
    /// A copy or regex block whose anchor D53 added for stacking: in Legacy it does not count
    /// for coverage, so Legacy rung resolution stays as before D53.
    pub stack_only: bool,
    /// The parent the block had before D53: the block of the line above it (or of the first
    /// line it replaced), as for an insert with no anchor. The Legacy coverage walk steps
    /// from a `stack_only` block to this one, as it stepped past an anchorless block then.
    pub legacy_parent: u32,
}

/// The line an insert hangs on, read from the map before the patch: its uid, the side, and
/// its block (the new block's parent).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Attach {
    pub uid: u32,
    pub side: Side,
    pub block: u32,
}

/// One edit of a patch. Line numbers are 0-based, in the text after the patch's earlier
/// edits (edits are applied in order, as the patches make them).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    /// Whole lines: `removed` lines at `at` replaced by `inserted` lines. `kept` pairs an
    /// inserted line with the removed line it writes back unchanged (offsets in each).
    Lines {
        at: usize,
        removed: usize,
        inserted: usize,
        site: u32,
        attach: Option<Attach>,
        kept: Vec<(u32, u32)>,
    },
    /// Not line-aligned (copy append, after or at on an unterminated last line, a regex
    /// inside a line): every line it touches becomes Modified.
    Touched {
        first_line: usize,
        removed_lines: usize,
        inserted_lines: usize,
        site: u32,
    },
}

impl Edit {
    pub fn site(&self) -> u32 {
        match self {
            Edit::Lines { site, .. } | Edit::Touched { site, .. } => *site,
        }
    }
}

/// A line an edit removed, and whether the same edit wrote it back unchanged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Removed {
    pub tag: LineTag,
    pub kept: bool,
}

/// What `LineMap::commit` did, per edit: the block it created (`NO_BLOCK` for none) and the
/// lines it removed.
#[derive(Clone, Debug, Default)]
pub struct CommitInfo {
    pub blocks: Vec<u32>,
    pub removed: Vec<Vec<Removed>>,
}

/// One site of a patch in the map: the lines its record wrote there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteRun {
    pub site: u32,
    pub first: usize,
    pub last: usize,
    pub lines: Vec<usize>,
    pub blocks: Vec<u32>,
}

/// One top-level item of a chain (§5.2): a block anchored on the chain's line, with every
/// line nested inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainItem {
    pub block: u32,
    pub first: usize,
    pub last: usize,
    pub mod_ix: u32,
    /// The block's author hint (D57).
    pub hint: Option<StackHint>,
}

#[derive(Clone, Debug, Default)]
pub struct LineMap {
    tags: Vec<LineTag>,
    blocks: Vec<BlockInfo>,
    next_uid: u32,
    alias: BTreeMap<u32, u32>,
    /// Lines of the buffer the map started from; their uids are `0..base_len`.
    base_len: usize,
    builtin: Vec<Range<usize>>,
    builtin_edited: Vec<usize>,
    /// record -> patch index, for every committed record.
    record_patch: BTreeMap<u32, u32>,
    /// The plan's `line_rules_v2`: anchorless pattern and `at` inserts get an implicit anchor.
    line_rules: bool,
    /// Chains follow implicit anchors (line rules on, and a stacking mode other than Legacy).
    implicit_chains: bool,
    /// A commit did not match the rope's line count and the map was resized.
    resized: bool,
    /// Patch index -> its author hint (D57), sorted by index; new blocks take it.
    hints: Vec<(u32, StackHint)>,
    /// Some block carries a hint.
    hinted_blocks: bool,
}

/// D42: whether a match over `lines` touches QuarterMaster's code in a way it may not. Any
/// match over a line QuarterMaster added (`added`) is refused; one that removes or rewrites
/// (`removes`) a line QuarterMaster rewrote in place (`edited`) is refused too.
pub fn builtin_blocks(
    lines: impl IntoIterator<Item = usize>,
    removes: bool,
    added: impl Fn(usize) -> bool,
    edited: impl Fn(usize) -> bool,
) -> bool {
    lines
        .into_iter()
        .any(|l| added(l) || (removes && edited(l)))
}

/// Whether a match of `len` lines at 0-based `line` of a prepared text may stand (D42). The
/// manager's mirrors use it, so they number and replay matches as the engine does.
pub fn match_filter(
    prepared: &crate::prepare::PreparedBuffer,
    line: usize,
    len: usize,
    removes: bool,
) -> bool {
    match_filter_lines(prepared, line..line + len, removes)
}

/// `match_filter` over any set of 0-based prepared lines.
pub fn match_filter_lines(
    prepared: &crate::prepare::PreparedBuffer,
    lines: impl IntoIterator<Item = usize>,
    removes: bool,
) -> bool {
    !builtin_blocks(
        lines,
        removes,
        |l| prepared.is_builtin(l),
        |l| prepared.builtin_edited.contains(&l),
    )
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(NO_INDEX)
}

/// The remaining lines of a commit: lines taken back from the output first, then the old
/// tags in order.
struct Cursor<'a> {
    pending: VecDeque<LineTag>,
    old: &'a [LineTag],
    pos: usize,
}

impl Cursor<'_> {
    fn next(&mut self) -> Option<LineTag> {
        if let Some(t) = self.pending.pop_front() {
            return Some(t);
        }
        let t = self.old.get(self.pos).copied();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// The line `next` would return.
    fn peek(&self) -> Option<LineTag> {
        self.pending
            .front()
            .copied()
            .or_else(|| self.old.get(self.pos).copied())
    }
}

/// The owner of a line an edit changes inside: the line's own owner when it has one, else the
/// edit's. A line an `at` keeps is the keeper's (§4.3 item 1), so this is for Touched edits.
fn owned(old: &LineTag, patch_ix: u32, site: u32) -> (u32, u32) {
    if old.owner == NO_PATCH {
        (patch_ix, site)
    } else {
        (old.owner, old.owner_site)
    }
}

/// The maker of a line an edit keeps or changes inside: the line's own, else the edit's patch.
fn made(old: &LineTag, patch_ix: u32) -> u32 {
    if old.maker == NO_PATCH {
        patch_ix
    } else {
        old.maker
    }
}

impl LineMap {
    /// A map of a buffer of `line_count` lines. `builtin` lines were added by QuarterMaster,
    /// `builtin_edited` lines rewritten by it (0-based; out-of-range entries are ignored).
    pub fn new(line_count: usize, builtin: &[Range<usize>], builtin_edited: &[usize]) -> Self {
        let mut tags: Vec<LineTag> = (0..line_count)
            .map(|i| LineTag {
                uid: to_u32(i),
                record: NO_RECORD,
                site: 0,
                kind: LineKind::Vanilla,
                block: NO_BLOCK,
                vanilla: to_u32(i),
                owner: NO_PATCH,
                owner_site: NO_SITE,
                maker: NO_PATCH,
            })
            .collect();
        let builtin: Vec<Range<usize>> = builtin
            .iter()
            .map(|r| r.start.min(line_count)..r.end.min(line_count))
            .filter(|r| !r.is_empty())
            .collect();
        for r in &builtin {
            for t in tags.get_mut(r.clone()).into_iter().flatten() {
                t.kind = LineKind::Builtin;
            }
        }
        let mut edited: Vec<usize> = builtin_edited
            .iter()
            .copied()
            .filter(|&i| i < line_count)
            .collect();
        edited.sort_unstable();
        edited.dedup();
        for &i in &edited {
            if let Some(t) = tags.get_mut(i) {
                t.kind = LineKind::BuiltinEdited;
            }
        }
        LineMap {
            tags,
            blocks: Vec::new(),
            next_uid: to_u32(line_count),
            alias: BTreeMap::new(),
            base_len: line_count,
            builtin,
            builtin_edited: edited,
            record_patch: BTreeMap::new(),
            line_rules: false,
            implicit_chains: false,
            resized: false,
            hints: Vec::new(),
            hinted_blocks: false,
        }
    }

    /// Whether a commit ever resized the map (its edits did not match the rope).
    pub fn resized(&self) -> bool {
        self.resized
    }

    /// Whether `uid`, a line of the buffer the map started from, was added by QuarterMaster.
    fn uid_is_added(&self, uid: u32) -> bool {
        let u = uid as usize;
        u < self.base_len && self.builtin.iter().any(|r| r.contains(&u))
    }

    /// Whether `uid`, a line of the buffer the map started from, was rewritten in place.
    fn uid_is_edited(&self, uid: u32) -> bool {
        let u = uid as usize;
        u < self.base_len && self.builtin_edited.binary_search(&u).is_ok()
    }

    /// D42 for a match over `lines` of the current text: true when it covers a line
    /// QuarterMaster added, or `removes` and it covers a line QuarterMaster rewrote in place
    /// (a line a patch changed inside keeps what it was).
    pub fn blocks_match(&self, lines: Range<usize>, removes: bool) -> bool {
        let kind = |l: usize| self.tags.get(l).map(|t| (t.kind, t.uid));
        builtin_blocks(
            lines,
            removes,
            |l| match kind(l) {
                Some((LineKind::Builtin, _)) => true,
                Some((LineKind::Modified, uid)) => self.uid_is_added(uid),
                _ => false,
            },
            |l| match kind(l) {
                Some((LineKind::BuiltinEdited, _)) => true,
                Some((LineKind::Modified, uid)) => self.uid_is_edited(uid),
                _ => false,
            },
        )
    }

    /// `blocks_match` over the buffer the map started from (its line `i` is uid `i`).
    pub fn original_blocks_match(&self, lines: Range<usize>, removes: bool) -> bool {
        builtin_blocks(
            lines,
            removes,
            |l| self.uid_is_added(to_u32(l)),
            |l| self.uid_is_edited(to_u32(l)),
        )
    }

    /// The `matched_qm_startup_code: skipped` warning for a refused match over `lines` of the
    /// current text, naming its first refused line.
    pub fn skip_warning(&self, lines: Range<usize>, removes: bool, target: &str) -> String {
        let first = lines
            .clone()
            .find(|&l| self.blocks_match(l..l + 1, removes))
            .unwrap_or(lines.start);
        let file_line = self.tags.get(first).and_then(|t| self.file_line(t.uid));
        crate::prepare::builtin_skip_warning(target, first, file_line)
    }

    /// `skip_warning` for each refused window (line, len), at most five, then a count of the
    /// rest.
    pub fn skip_warnings(
        &self,
        windows: &[(usize, usize)],
        removes: bool,
        target: &str,
    ) -> Vec<String> {
        const SHOWN: usize = 5;
        let mut out: Vec<String> = Vec::new();
        let mut more = 0usize;
        for &(l, n) in windows {
            let w = self.skip_warning(l..l + n.max(1), removes, target);
            if out.contains(&w) {
                continue;
            }
            if out.len() < SHOWN {
                out.push(w);
            } else {
                more += 1;
            }
        }
        if more > 0 {
            out.push(format!(
                "{}: {more} more matches in '{target}' cover QuarterMaster's code",
                crate::prepare::BUILTIN_MATCH_SKIPPED
            ));
        }
        out
    }

    /// The current line of `uid`.
    fn line_of_uid(&self, uid: usize) -> Option<usize> {
        let uid = to_u32(uid);
        self.tags.iter().position(|t| t.uid == uid)
    }

    /// The top of QuarterMaster's frame (D43): the line right after the lines it added at the
    /// top of the file (the Steam safety block), when it added any there.
    pub fn frame_start(&self) -> Option<usize> {
        let top = self.builtin.first().filter(|r| r.start == 0)?;
        self.line_of_uid(top.end.checked_sub(1)?).map(|l| l + 1)
    }

    /// The first line QuarterMaster added below the file's own lines (the footer), when it
    /// added lines at the top too (D43).
    pub fn frame_end(&self) -> Option<usize> {
        self.frame_start()?;
        let footer = self.builtin.get(1)?;
        self.line_of_uid(footer.start)
    }

    /// The current line of the first line of the file's own code (D53): the line right after
    /// the Steam safety block on a file QuarterMaster frames, else line 0 of the buffer. `None`
    /// when the file has no line of its own or the line is gone.
    pub fn first_own_line(&self) -> Option<usize> {
        let uid = match self.builtin.first() {
            Some(top) if top.start == 0 => top.end,
            _ => 0,
        };
        self.own_line(uid)
    }

    /// The current line of the last line of the file's own code (D53): the line above the
    /// footer on a file QuarterMaster frames, else the buffer's last line.
    pub fn last_own_line(&self) -> Option<usize> {
        let end = match (self.builtin.first(), self.builtin.get(1)) {
            (Some(top), Some(footer)) if top.start == 0 => footer.start,
            (Some(top), None) if top.start > 0 => top.start,
            _ => self.base_len,
        };
        self.own_line(end.checked_sub(1)?)
    }

    /// The current line of buffer line `uid` (followed through `at` rewrites), when it is a
    /// line of the file's own code.
    fn own_line(&self, uid: usize) -> Option<usize> {
        if uid >= self.base_len || self.uid_is_added(to_u32(uid)) {
            return None;
        }
        self.line_of_uid(self.resolve(to_u32(uid)) as usize)
    }

    /// The plan's `line_rules_v2` for this apply (D50, D51): later pattern and `at` commits
    /// record implicit anchors. Off by default (today's rules).
    pub fn set_line_rules(&mut self, on: bool) {
        self.line_rules = on;
    }

    /// Whether chains follow implicit anchors: only with the line rules on in Dependencies or
    /// LoadOrder (D51). Off, a chain sees an implicit block as having no anchor (today's rule).
    pub fn set_implicit_chains(&mut self, on: bool) {
        self.implicit_chains = on;
    }

    pub fn implicit_chains(&self) -> bool {
        self.implicit_chains
    }

    /// The author hints of the table's patches for this apply (D57), as (patch index, hint).
    /// Blocks made after this carry their patch's hint.
    pub fn set_hints(&mut self, hints: &[(u32, StackHint)]) {
        self.hints = hints.to_vec();
        self.hints.sort_by_key(|&(ix, _)| ix);
        self.hints.dedup_by_key(|&mut (ix, _)| ix);
    }

    /// Whether a block of this map carries an author hint (D57).
    pub fn has_hinted_blocks(&self) -> bool {
        self.hinted_blocks
    }

    fn hint_of(&self, patch_ix: u32) -> Option<StackHint> {
        self.hints
            .binary_search_by_key(&patch_ix, |&(ix, _)| ix)
            .ok()
            .and_then(|i| self.hints.get(i))
            .map(|&(_, h)| h)
    }

    pub fn len(&self) -> usize {
        self.tags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    pub fn tags(&self) -> &[LineTag] {
        &self.tags
    }

    pub fn blocks(&self) -> &[BlockInfo] {
        &self.blocks
    }

    pub fn block(&self, ix: u32) -> Option<&BlockInfo> {
        self.blocks.get(ix as usize)
    }

    pub fn get(&self, line: usize) -> Option<&LineTag> {
        self.tags.get(line)
    }

    /// The patch index of a committed record.
    pub fn patch_of_record(&self, record: u32) -> Option<u32> {
        self.record_patch.get(&record).copied()
    }

    fn fresh_uid(&mut self) -> u32 {
        let u = self.next_uid;
        self.next_uid = self.next_uid.saturating_add(1);
        u
    }

    /// The live uid a removed line now stands for (its first replacement line). Follows the
    /// alias at most `alias.len()` steps.
    pub fn resolve(&self, uid: u32) -> u32 {
        let mut u = uid;
        for _ in 0..self.alias.len() {
            match self.alias.get(&u) {
                Some(&next) if next != u => u = next,
                _ => break,
            }
        }
        u
    }

    /// The 1-based line of an original line (by uid) in the file's own numbering, without
    /// QuarterMaster's added lines. None for an added line or a uid past the original buffer.
    pub fn file_line(&self, uid: u32) -> Option<usize> {
        let u = uid as usize;
        if u >= self.base_len {
            return None;
        }
        crate::prepare::file_line(&self.builtin, u)
    }

    /// Whether a line of the buffer the map started from (by uid) is QuarterMaster's.
    pub fn uid_is_builtin(&self, uid: u32) -> bool {
        let u = uid as usize;
        u < self.base_len
            && (self.builtin.iter().any(|r| r.contains(&u))
                || self.builtin_edited.binary_search(&u).is_ok())
    }

    /// Whether `line` is QuarterMaster's: a line it added or rewrote, or one of those changed
    /// inside the line by a patch.
    pub fn is_builtin_line(&self, line: usize) -> bool {
        self.tags.get(line).is_some_and(|t| match t.kind {
            LineKind::Builtin | LineKind::BuiltinEdited => true,
            LineKind::Modified => self.uid_is_builtin(t.uid),
            _ => false,
        })
    }

    fn new_block(&mut self, mut b: BlockInfo) -> u32 {
        b.hint = self.hint_of(b.patch_ix);
        self.hinted_blocks |= b.hint.is_some();
        let ix = to_u32(self.blocks.len());
        self.blocks.push(b);
        ix
    }

    /// Applies one patch's `edits`, in order, as record `record`. `rope_lines` is the line
    /// count of the patched text: when the map does not match it, it is resized with a
    /// warning. Never panics; out-of-range edits are clamped.
    pub fn commit(
        &mut self,
        record: u32,
        patch_ix: u32,
        mod_ix: u32,
        edits: &[Edit],
        rope_lines: usize,
    ) -> CommitInfo {
        self.commit_with(record, patch_ix, mod_ix, edits, rope_lines, false)
    }

    /// `commit`; `implicit` says the edits are a pattern or `at` patch's, whose anchorless
    /// inserts get an implicit anchor while the line rules are on (D51).
    pub fn commit_with(
        &mut self,
        record: u32,
        patch_ix: u32,
        mod_ix: u32,
        edits: &[Edit],
        rope_lines: usize,
        implicit: bool,
    ) -> CommitInfo {
        let implicit = implicit && self.line_rules;
        if record != NO_RECORD {
            self.record_patch.insert(record, patch_ix);
        }
        let old = std::mem::take(&mut self.tags);
        let mut out: Vec<LineTag> = Vec::with_capacity(old.len() + 8);
        let mut cur = Cursor {
            pending: VecDeque::new(),
            old: &old,
            pos: 0,
        };
        let mut info = CommitInfo::default();

        for edit in edits {
            let (at, removed) = match edit {
                Edit::Lines { at, removed, .. } => (*at, *removed),
                Edit::Touched {
                    first_line,
                    removed_lines,
                    ..
                } => (*first_line, *removed_lines),
            };
            // Move to `at`: back into the output already written, or forward over old lines.
            if at < out.len() {
                let tail = out.split_off(at);
                for t in tail.into_iter().rev() {
                    cur.pending.push_front(t);
                }
            }
            while out.len() < at {
                match cur.next() {
                    Some(t) => out.push(t),
                    None => break,
                }
            }
            let gone: Vec<LineTag> = (0..removed).map_while(|_| cur.next()).collect();
            let site = edit.site();

            match edit {
                Edit::Lines {
                    inserted,
                    attach,
                    kept,
                    ..
                } => {
                    let mut kept_of: Vec<Option<usize>> = vec![None; *inserted];
                    let mut is_kept = vec![false; gone.len()];
                    for &(ins, rem) in kept {
                        let (ins, rem) = (ins as usize, rem as usize);
                        if rem < gone.len() && !is_kept.get(rem).copied().unwrap_or(true) {
                            if let Some(slot) = kept_of.get_mut(ins) {
                                if slot.is_none() {
                                    *slot = Some(rem);
                                    if let Some(k) = is_kept.get_mut(rem) {
                                        *k = true;
                                    }
                                }
                            }
                        }
                    }
                    let block = if *inserted == 0 && gone.is_empty() {
                        NO_BLOCK
                    } else {
                        // An insert with no anchor nests in the block of the line above it.
                        let first_block = match gone.first() {
                            Some(t) => t.block,
                            None => out.last().map_or(NO_BLOCK, |t| t.block),
                        };
                        let explicit = match attach {
                            Some(a) if gone.is_empty() => Some(a),
                            _ => None,
                        };
                        let (anchor, parent, is_implicit) = match explicit {
                            Some(a) => (Some((self.resolve(a.uid), a.side)), a.block, false),
                            None => {
                                // D51: the parent stays the block above, as today.
                                let a = if implicit {
                                    self.implicit_anchor(&gone, &kept_of, out.last(), cur.peek())
                                } else {
                                    None
                                };
                                (a, first_block, a.is_some())
                            }
                        };
                        let (parent, depth) = match self.blocks.get(parent as usize) {
                            Some(p) => (parent, p.depth.saturating_add(1)),
                            None => (NO_BLOCK, 0),
                        };
                        let legacy_parent = if self.blocks.get(first_block as usize).is_some() {
                            first_block
                        } else {
                            NO_BLOCK
                        };
                        self.new_block(BlockInfo {
                            record,
                            patch_ix,
                            mod_ix,
                            site,
                            anchor,
                            parent,
                            depth,
                            implicit: is_implicit,
                            hint: None,
                            stack_only: false,
                            legacy_parent,
                        })
                    };
                    let first_new = out.len();
                    for slot in &kept_of {
                        let tag = match slot.and_then(|r| gone.get(r)) {
                            Some(old) => LineTag {
                                record,
                                site,
                                kind: LineKind::Kept,
                                owner: patch_ix,
                                owner_site: site,
                                maker: made(old, patch_ix),
                                ..*old
                            },
                            None => LineTag {
                                uid: self.fresh_uid(),
                                record,
                                site,
                                kind: LineKind::Inserted,
                                block,
                                vanilla: NO_LINE,
                                owner: patch_ix,
                                owner_site: site,
                                maker: patch_ix,
                            },
                        };
                        out.push(tag);
                    }
                    if let Some(first) = out.get(first_new).map(|t| t.uid) {
                        for (t, &k) in gone.iter().zip(&is_kept) {
                            if !k && t.uid != first {
                                self.alias.insert(t.uid, first);
                            }
                        }
                    }
                    info.blocks.push(block);
                    info.removed.push(
                        gone.iter()
                            .zip(&is_kept)
                            .map(|(&tag, &kept)| Removed { tag, kept })
                            .collect(),
                    );
                }
                Edit::Touched { inserted_lines, .. } => {
                    let vanilla = gone.first().map_or(NO_LINE, |t| t.vanilla);
                    let first_block = gone.first().map_or(NO_BLOCK, |t| t.block);
                    let first_new = out.len();
                    // With the line rules, whole new lines below a touched line get their own
                    // block with an implicit anchor on the line above (D51).
                    let mut new_lines_block: Option<u32> = None;
                    for i in 0..*inserted_lines {
                        let tag = match gone.get(i) {
                            Some(old) => {
                                let (owner, owner_site) = owned(old, patch_ix, site);
                                LineTag {
                                    record,
                                    site,
                                    kind: LineKind::Modified,
                                    vanilla,
                                    owner,
                                    owner_site,
                                    maker: made(old, patch_ix),
                                    ..*old
                                }
                            }
                            None => {
                                if implicit && new_lines_block.is_none() {
                                    new_lines_block = out.last().map(|above| {
                                        self.implicit_block(
                                            record,
                                            patch_ix,
                                            mod_ix,
                                            site,
                                            (above.uid, Side::After),
                                            first_block,
                                        )
                                    });
                                }
                                let block = new_lines_block.unwrap_or(first_block);
                                LineTag {
                                    uid: self.fresh_uid(),
                                    record,
                                    site,
                                    kind: LineKind::Modified,
                                    block,
                                    vanilla,
                                    owner: patch_ix,
                                    owner_site: site,
                                    maker: patch_ix,
                                }
                            }
                        };
                        out.push(tag);
                    }
                    if let Some(first) = out.get(first_new).map(|t| t.uid) {
                        for t in gone.iter().skip(*inserted_lines) {
                            self.alias.insert(t.uid, first);
                        }
                    }
                    info.blocks.push(new_lines_block.unwrap_or(NO_BLOCK));
                    info.removed.push(
                        gone.iter()
                            .map(|&tag| Removed { tag, kept: false })
                            .collect(),
                    );
                }
            }
        }
        while let Some(t) = cur.next() {
            out.push(t);
        }
        drop(cur);
        self.tags = out;

        if self.tags.len() != rope_lines {
            log::warn!(
                "line map out of step ({} for {} lines); resized",
                self.tags.len(),
                rope_lines
            );
            self.resized = true;
            self.resize(rope_lines);
        }
        info
    }

    fn resize(&mut self, n: usize) {
        if self.tags.len() > n {
            self.tags.truncate(n);
            return;
        }
        while self.tags.len() < n {
            let uid = self.fresh_uid();
            self.tags.push(LineTag {
                uid,
                record: NO_RECORD,
                site: 0,
                kind: LineKind::Vanilla,
                block: NO_BLOCK,
                vanilla: NO_LINE,
                owner: NO_PATCH,
                owner_site: NO_SITE,
                maker: NO_PATCH,
            });
        }
    }

    /// The implicit anchor of an anchorless `Lines` edit (D51), or `None`:
    /// - a pure insert hangs after the line above it, or before the line below at the top of
    ///   the file;
    /// - a Replace hangs after the first line it kept that has a new line right below it (the
    ///   insert-only `at`, "L -> L\nr");
    /// - a Replace with no such kept line has none: replaced code is not an insertion next to
    ///   a line.
    fn implicit_anchor(
        &self,
        gone: &[LineTag],
        kept_of: &[Option<usize>],
        above: Option<&LineTag>,
        below: Option<LineTag>,
    ) -> Option<(u32, Side)> {
        if gone.is_empty() {
            return match (above, below) {
                (Some(a), _) => Some((a.uid, Side::After)),
                (None, Some(b)) => Some((b.uid, Side::Before)),
                (None, None) => None,
            };
        }
        let kept = kept_of
            .iter()
            .zip(kept_of.iter().skip(1))
            .find_map(|(above, here)| match (above, here) {
                (Some(r), None) => gone.get(*r),
                _ => None,
            })?;
        Some((kept.uid, Side::After))
    }

    /// A new block with an implicit anchor (D51), nested like an anchorless insert.
    fn implicit_block(
        &mut self,
        record: u32,
        patch_ix: u32,
        mod_ix: u32,
        site: u32,
        anchor: (u32, Side),
        parent: u32,
    ) -> u32 {
        let (parent, depth) = match self.blocks.get(parent as usize) {
            Some(p) => (parent, p.depth.saturating_add(1)),
            None => (NO_BLOCK, 0),
        };
        self.new_block(BlockInfo {
            record,
            patch_ix,
            mod_ix,
            site,
            anchor: Some(anchor),
            parent,
            depth,
            implicit: true,
            hint: None,
            stack_only: false,
            legacy_parent: parent,
        })
    }

    /// The record that wrote `line`, for crash attribution.
    pub fn record_of_line(&self, line: usize) -> Option<u32> {
        self.tags
            .get(line)
            .map(|t| t.record)
            .filter(|&r| r != NO_RECORD)
    }

    /// The record of every line (`NO_RECORD` for lines no patch wrote).
    pub fn line_records(&self) -> Vec<u32> {
        self.tags.iter().map(|t| t.record).collect()
    }

    /// Per record, the 1-based line runs it wrote that are still in the text.
    pub fn record_runs(&self) -> BTreeMap<u32, Vec<(usize, usize)>> {
        let mut out: BTreeMap<u32, Vec<(usize, usize)>> = BTreeMap::new();
        for (i, t) in self.tags.iter().enumerate() {
            if t.record == NO_RECORD {
                continue;
            }
            let n = i + 1;
            let runs = out.entry(t.record).or_default();
            match runs.last_mut() {
                Some(last) if last.1 + 1 == n => last.1 = n,
                _ => runs.push((n, n)),
            }
        }
        out
    }

    /// The lines patch `patch_ix` wrote, per site (its own lines only: code another patch
    /// nested inside them is not part of the run). `by_owner` (the line rules, D50) takes the
    /// lines the patch owns, grouped by their owner site, so a line another patch later
    /// changed inside stays in the run; without it, the lines whose last writer is the patch,
    /// grouped by that writer's site (today's rule).
    pub fn site_runs(&self, patch_ix: u32, by_owner: bool) -> Vec<SiteRun> {
        let mut runs: BTreeMap<u32, SiteRun> = BTreeMap::new();
        for (i, t) in self.tags.iter().enumerate() {
            let site = if by_owner {
                if t.owner == NO_PATCH || t.owner != patch_ix {
                    continue;
                }
                t.owner_site
            } else {
                if t.record == NO_RECORD || self.patch_of_record(t.record) != Some(patch_ix) {
                    continue;
                }
                t.site
            };
            let run = runs.entry(site).or_insert_with(|| SiteRun {
                site,
                first: i,
                last: i,
                lines: Vec::new(),
                blocks: Vec::new(),
            });
            run.last = i;
            run.lines.push(i);
            let own = self
                .blocks
                .get(t.block as usize)
                .is_some_and(|b| b.patch_ix == patch_ix);
            if own && !run.blocks.contains(&t.block) {
                run.blocks.push(t.block);
            }
        }
        runs.into_values().collect()
    }

    /// A block's anchor, resolved to the line it now stands for (an `at` may have rewritten
    /// the line since the block was made).
    pub fn resolved_anchor(&self, info: &BlockInfo) -> Option<(u32, Side)> {
        info.anchor.map(|(u, s)| (self.resolve(u), s))
    }

    /// A block's anchor as chains see it: resolved, and `None` for an implicit anchor while
    /// implicit chains are off (D51).
    pub fn chain_anchor(&self, info: &BlockInfo) -> Option<(u32, Side)> {
        if info.implicit && !self.implicit_chains {
            return None;
        }
        self.resolved_anchor(info)
    }

    /// The block in `block`'s ancestry (itself first) anchored on `anchor` (resolved).
    fn item_in_ancestry(&self, block: u32, anchor: (u32, Side)) -> Option<u32> {
        let mut b = block;
        for _ in 0..=self.blocks.len() {
            let info = self.blocks.get(b as usize)?;
            if self.chain_anchor(info) == Some(anchor) {
                return Some(b);
            }
            b = info.parent;
        }
        None
    }

    /// The items stacked on `anchor_line`'s `side` (§5.2), top to bottom. An item's lines
    /// include every block nested inside it.
    pub fn chain(&self, anchor_line: usize, side: Side) -> Vec<ChainItem> {
        let Some(tag) = self.tags.get(anchor_line) else {
            return Vec::new();
        };
        let key = (self.resolve(tag.uid), side);
        let item_at = |l: usize| -> Option<u32> {
            let t = self.tags.get(l)?;
            self.item_in_ancestry(t.block, key)
        };
        let mut items: Vec<ChainItem> = Vec::new();
        let push = |items: &mut Vec<ChainItem>, block: u32, l: usize| match items.last_mut() {
            Some(last) if last.block == block => {
                last.first = last.first.min(l);
                last.last = last.last.max(l);
            }
            _ => items.push(ChainItem {
                block,
                first: l,
                last: l,
                mod_ix: self
                    .blocks
                    .get(block as usize)
                    .map_or(NO_INDEX, |b| b.mod_ix),
                hint: self.blocks.get(block as usize).and_then(|b| b.hint),
            }),
        };
        match side {
            Side::After => {
                let mut l = anchor_line + 1;
                while let Some(block) = item_at(l) {
                    push(&mut items, block, l);
                    l += 1;
                }
            }
            Side::Before => {
                let mut l = anchor_line;
                while l > 0 {
                    l -= 1;
                    let Some(block) = item_at(l) else {
                        break;
                    };
                    push(&mut items, block, l);
                }
                items.reverse();
            }
        }
        items
    }

    /// The anchors of `block` and of each of its ancestors, nearest first, resolved.
    pub fn anchors_of(&self, block: u32) -> impl Iterator<Item = (u32, Side)> + '_ {
        self.anchors_of_where(block, true)
    }

    /// `anchors_of` without implicit anchors (D51): what coverage reads, so an implicit anchor
    /// never makes a patch count as touching a line.
    pub fn explicit_anchors_of(&self, block: u32) -> impl Iterator<Item = (u32, Side)> + '_ {
        self.anchors_of_where(block, false)
    }

    /// `explicit_anchors_of` for coverage. In Legacy (`legacy`) a block that D53 hung only
    /// for stacking (a copy or regex block) counts as it did before D53: its anchor is
    /// skipped and the walk goes on to its `legacy_parent`.
    pub fn coverage_anchors_of(
        &self,
        block: u32,
        legacy: bool,
    ) -> impl Iterator<Item = (u32, Side)> + '_ {
        let mut b = block;
        let mut steps = 0usize;
        std::iter::from_fn(move || {
            while steps <= self.blocks.len() {
                steps += 1;
                let info = self.blocks.get(b as usize)?;
                if legacy && info.stack_only {
                    b = info.legacy_parent;
                    continue;
                }
                b = info.parent;
                if info.implicit {
                    continue;
                }
                if let Some(a) = self.resolved_anchor(info) {
                    return Some(a);
                }
            }
            None
        })
    }

    /// Marks the blocks `record` made (a copy or regex record) as hung only for stacking.
    pub fn mark_stack_only(&mut self, record: u32) {
        for b in self.blocks.iter_mut().filter(|b| b.record == record) {
            b.stack_only = true;
        }
    }

    fn anchors_of_where(
        &self,
        block: u32,
        implicit: bool,
    ) -> impl Iterator<Item = (u32, Side)> + '_ {
        let mut b = block;
        let mut steps = 0usize;
        std::iter::from_fn(move || {
            while steps <= self.blocks.len() {
                steps += 1;
                let info = self.blocks.get(b as usize)?;
                b = info.parent;
                if info.implicit && !implicit {
                    continue;
                }
                if let Some(a) = self.resolved_anchor(info) {
                    return Some(a);
                }
            }
            None
        })
    }
}

/// Pairs each inserted line with the removed line it writes back unchanged: the first later
/// removed line with the same trimmed, non-empty text, in order (`origin::trace`'s rule).
/// Returns `(inserted offset, removed offset)`.
pub fn kept_pairs(removed: &[&str], inserted: &[&str]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (j, l) in inserted.iter().enumerate() {
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        let hit = removed
            .iter()
            .enumerate()
            .skip(cursor)
            .find(|(_, r)| r.trim() == t)
            .map(|(i, _)| i);
        if let Some(i) = hit {
            out.push((to_u32(j), to_u32(i)));
            cursor = i + 1;
        }
    }
    out
}

/// D53: the lines a pure insert at the very end of the text adds when the insert starts with
/// a newline (the bytes of a copy append on a file QuarterMaster does not frame), or `None`
/// for any other edit. The newline ends the last line when it has none (`terminated` false),
/// else it starts an empty line. Only for an edit that is not already whole lines.
pub fn end_lines(at_end: bool, terminated: bool, inserted: &str) -> Option<usize> {
    if !at_end || !inserted.starts_with('\n') {
        return None;
    }
    let body = if terminated {
        inserted
    } else {
        inserted.get(1..).unwrap_or_default()
    };
    Some(body.lines().count())
}

/// The `Edit` for replacing bytes `start..end` of `rope` (before the change) with `inserted`.
/// Whole-line changes are `Lines`, and so is a newline-led insert at the end of the text
/// (`end_lines`); anything else is `Touched`, with the same line counts `origin::trace`
/// gives. Offsets are clamped to the rope.
pub fn byte_edit(
    rope: &Rope,
    start: usize,
    end: usize,
    inserted: &str,
    site: u32,
    attach: Option<Attach>,
) -> Edit {
    let len = rope.byte_len();
    let start = start.min(len);
    let end = end.clamp(start, len);
    let k = rope.line_of_byte(start);
    let at_line_start = start == 0 || rope.byte(start - 1) == b'\n';
    let removed_text = if end > start {
        rope.byte_slice(start..end).to_string()
    } else {
        String::new()
    };
    let aligned = at_line_start
        && (removed_text.is_empty() || removed_text.ends_with('\n'))
        && (inserted.is_empty() || inserted.ends_with('\n'));
    if aligned {
        let old: Vec<&str> = removed_text.lines().collect();
        let new: Vec<&str> = inserted.lines().collect();
        return Edit::Lines {
            at: k,
            removed: removed_text.matches('\n').count(),
            inserted: new.len(),
            site,
            attach,
            kept: kept_pairs(&old, &new),
        };
    }
    let terminated = len == 0 || rope.byte(len - 1) == b'\n';
    if let Some(n) = end_lines(start == len && end == start, terminated, inserted) {
        return Edit::Lines {
            at: rope.line_len(),
            removed: 0,
            inserted: n,
            site,
            attach,
            kept: Vec::new(),
        };
    }
    let old_total = rope.line_len();
    let m = if end > start {
        rope.line_of_byte(end - 1)
    } else {
        k
    };
    let first = k.min(old_total);
    let last_excl = (m + 1).min(old_total).max(first);
    let suffix = old_total - last_excl;
    // Lines of the text after the change, as `str::lines` counts them.
    let newlines =
        rope.line_of_byte(len) - (rope.line_of_byte(end) - k) + inserted.matches('\n').count();
    let new_len = len - (end - start) + inserted.len();
    let last_byte = if end < len {
        Some(rope.byte(len - 1))
    } else if let Some(b) = inserted.as_bytes().last() {
        Some(*b)
    } else if start > 0 {
        Some(rope.byte(start - 1))
    } else {
        None
    };
    let new_total = newlines + usize::from(new_len > 0 && last_byte != Some(b'\n'));
    Edit::Touched {
        first_line: first,
        removed_lines: last_excl - first,
        inserted_lines: new_total.saturating_sub(first + suffix),
        site,
    }
}

#[cfg(test)]
mod tests;
