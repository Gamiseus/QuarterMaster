#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use std::ops::Range;
use std::path::Path;

use regex_cursor::engines::meta::Regex;
use regex_cursor::regex_automata::util::captures::Captures;
use regex_cursor::regex_automata::util::interpolate;
use regex_cursor::regex_automata::util::syntax;
use regex_cursor::Input;

use crop::Rope;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::chunk_vec_cursor::IntoCursor;
use crate::dump::{ByteDebugEntry, ByteRegion, DebugPatchType, PatchSource};
use crate::lenient::Lenient;

use super::anchor::{Found, OriginalLines, Place, Rung, SiteKey, SiteSource};
use super::linemap::{byte_edit, Attach, Edit, LineMap, Side};
use super::pattern::Window;
use super::stack::{place_in_stack, stacked_text, LegacySlot, StackHint};
use super::{ApplyCtx, ApplyOutcome, InsertPosition, OnFailurePolicy, Priority, Target};

/// Whether `s` reads a capture group when interpolated: `$1`, `$name` or `${name}` (`$$` is
/// a literal `$`).
pub fn uses_capture(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'$' {
            match b.get(i + 1) {
                Some(b'$') => {
                    i += 2;
                    continue;
                }
                Some(&n) if n.is_ascii_alphanumeric() || n == b'_' || n == b'{' => return true,
                _ => {}
            }
        }
        i += 1;
    }
    false
}

/// The lines bytes `start..end` of `rope` cover (an empty span covers its line).
fn span_lines(rope: &Rope, start: usize, end: usize) -> Range<usize> {
    let len = rope.byte_len();
    let start = start.min(len);
    let end = end.min(len).max(start);
    let first = rope.line_of_byte(start);
    let last = rope.line_of_byte(end.saturating_sub(1).max(start));
    first..last + 1
}

/// The line a whole-line insert at byte `at` of `pre` (the text before the patch) hangs on
/// (D53): before the line at the insert for Before and At, after the line above for After,
/// and the other side at either end of the text. `None` when `at` is inside a line that is
/// not the unterminated last one.
fn insert_anchor(pre: &Rope, at: usize, position: InsertPosition) -> Option<(usize, Side)> {
    let len = pre.byte_len();
    let at = at.min(len);
    let n = pre.line_len();
    let below = if at == 0 || pre.byte(at - 1) == b'\n' {
        pre.line_of_byte(at)
    } else if at == len {
        // A newline-led insert at the end ends the last line first.
        n
    } else {
        return None;
    };
    let before = (below < n).then_some((below, Side::Before));
    let after = below.checked_sub(1).map(|l| (l, Side::After));
    match position {
        InsertPosition::After => after.or(before),
        _ => before.or(after),
    }
}

/// One thing a regex patch applies: a match, or a literal insert at a byte where an edge
/// rung put it (D52), with the rung's place (first line, line count) and side.
enum Item {
    Capture(Captures),
    Edge {
        at: usize,
        place: (usize, usize),
        pos: InsertPosition,
    },
}

/// A regex A's matches for its rungs (D52): its own matches first, then the ones the rungs
/// found inside B, all on the text before the patch.
struct RegexSites<'a> {
    patch: &'a RegexPatch,
    re: &'a Regex,
    rope: &'a Rope,
    lines: &'a LineMap,
    found: std::cell::RefCell<Vec<Captures>>,
}

impl RegexSites<'_> {
    fn place(&self, ix: usize, groups: &Captures) -> Option<Place> {
        let m = groups.get_group(0)?;
        let lines = span_lines(self.rope, m.start, m.end);
        Some(Place {
            line: lines.start,
            len: lines.len(),
            indent: String::new(),
            lines: lines.clone().collect(),
            pos: self.patch.position,
            found: Some(Found {
                ix,
                start: m.start,
                end: m.end,
            }),
        })
    }

    fn own(&self) -> Vec<Place> {
        let found = self.found.borrow();
        found
            .iter()
            .enumerate()
            .filter_map(|(i, g)| self.place(i, g))
            .collect()
    }
}

impl SiteSource for RegexSites<'_> {
    fn label(&self) -> String {
        let p = self.patch;
        p.name.clone().or_else(|| p.id.clone()).unwrap_or_default()
    }

    fn position(&self) -> InsertPosition {
        self.patch.position
    }

    fn vanilla(&self, original: &OriginalLines, lines: &LineMap) -> Vec<Window> {
        self.patch.site_windows(original.text(), |l, removes| {
            !lines.original_blocks_match(l, removes)
        })
    }

    fn search(&self, ranges: &[Range<usize>]) -> Option<Vec<Place>> {
        let rope = self.rope;
        let line_len = rope.line_len();
        let mut out = Vec::new();
        for r in ranges {
            let start = rope.byte_of_line(r.start.min(line_len));
            let end = rope.byte_of_line(r.end.min(line_len)).max(start);
            if start == end {
                continue;
            }
            // The whole text stays visible to look-around; only the span is searched.
            let mut input = Input::new(rope.into_cursor());
            input.set_span(start..end);
            for groups in self.re.captures_iter(input) {
                let Some(m) = groups.get_group(0) else { continue };
                if m.start >= end {
                    continue;
                }
                // QuarterMaster's lines are never matched (D42).
                let blocked = self
                    .patch
                    .match_lines(rope, &groups)
                    .is_some_and(|(l, removes)| self.lines.blocks_match(l, removes));
                if blocked {
                    continue;
                }
                let ix = self.found.borrow().len();
                if let Some(p) = self.place(ix, &groups) {
                    self.found.borrow_mut().push(groups);
                    out.push(p);
                }
            }
        }
        Some(out)
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RegexPatch {
    pub target: Target,

    // The Regex pattern that will be used to both match and create capture groups.
    pub pattern: String,

    // The position to insert the payload relative to the match/capture group.
    pub position: InsertPosition,

    // The target root capture group. The insert position is relative to this group.
    // Defaults to $0 if not set (the entire match).
    pub root_capture: Option<String>,

    // The payload that will be inserted. Regex capture groups can be interpolated
    // by $index.
    pub payload: String,

    // A string or Regex capture to prepend onto the start of each LINE of the payload.
    // This value defaults to an empty string.
    #[serde(default)]
    pub line_prepend: String,

    // Apply patch at most `times` (or `expect`) times, warn if the number of matches differs.
    #[serde(default, alias = "expect")]
    pub times: Option<usize>,

    // If enabled, whitespace is ignored unless escaped
    #[serde(default)]
    pub verbose: bool,

    pub id: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    pub priority: Option<Priority>,
    /// Patches this one must run before. Each entry is a patch id (`hud_overlay`, or a full id
    /// like `gcore:main_lua:1`), `mod_id:patch_id`, or `mod_id:*` for every patch of that mod.
    /// Honoured by `patch::order`; a profile rule in `load-order.json` can override it.
    #[serde(default)]
    pub before: Vec<String>,
    /// Patches this one must run after, in the same forms as `before`.
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub on_failure: OnFailurePolicy,
    /// Rungs: places on another patch's code this patch searches first (D52). A bad entry is
    /// skipped and reported, and so is a value that is not a list; the patch still loads and
    /// falls back to its own matches.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "crate::lenient::lenient_list",
        default
    )]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub anchors: Vec<Lenient<Rung>>,
    /// Which of the patch's own matches the rungs are for (none: all of them). A bad entry,
    /// or a value that is not a list, is skipped.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "crate::lenient::lenient_list",
        default
    )]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub sites: Vec<Lenient<SiteKey>>,
    /// `"first"` or `"last"`, as on a pattern patch (D57).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub stack: Option<Lenient<StackHint>>,

    #[serde(skip)]
    pub mod_id: Option<String>,

    #[serde(skip)]
    pub disabled: bool,

    /// Profile and compat data (Phase C §3.3); never read from a patch file.
    #[serde(skip)]
    pub qm: crate::patch::plan::PatchExtra,
}

impl RegexPatch {
    pub fn debug_from_warning_string(&self, path: &Path, warning: String) -> ByteDebugEntry {
        log::warn!("{}", warning);
        ByteDebugEntry {
            patch_source: PatchSource {
                file: path.display().to_string(),
                mod_id: self.mod_id.clone(),
                patch_id: self.id.clone(),
                patch_name: self.name.clone(),
                pattern: Some(self.pattern.clone()),
                patch_type: DebugPatchType::Regex,
            },
            regions: Vec::new(),
            matched_count: 0,
            expected_count: self.times,
            warnings: Some(vec![warning]),
        }
    }

    /// The lines a match covers before any change, and whether its edit rewrites them (D42).
    /// Every regex edit rewrites the lines it spans, except a whole-line insert before or after
    /// them; an insert inside a line also rewrites the line it goes into.
    fn match_lines(&self, rope: &Rope, groups: &Captures) -> Option<(Range<usize>, bool)> {
        let m = groups.get_group(0)?;
        let len = rope.byte_len();
        let start = m.start.min(len);
        let end = m.end.min(len).max(start);
        let first = rope.line_of_byte(start);
        let last = rope.line_of_byte(end.saturating_sub(1).max(start));
        let lines = first..last + 1;
        if self.position == InsertPosition::At {
            return Some((lines, true));
        }
        let name = self.root_capture.as_deref().unwrap_or("0").replace('$', "");
        let root = match name.parse::<usize>() {
            Ok(i) => groups.get_group(i),
            Err(_) => groups.get_group_by_name(&name),
        }
        .unwrap_or(m);
        let at = match self.position {
            InsertPosition::After => root.end,
            _ => root.start,
        }
        .min(len);
        let line_start = at == 0 || rope.byte(at - 1) == b'\n';
        if line_start && self.payload.ends_with('\n') {
            return Some((lines, false));
        }
        let k = rope.line_of_byte(at);
        Some((lines.start.min(k)..lines.end.max(k + 1), true))
    }

    pub fn apply(&self, target: &str, rope: &mut Rope, path: &Path) -> Option<ByteDebugEntry> {
        let lines = LineMap::new(rope.line_len(), &[], &[]);
        self.apply_in(target, rope, path, &ApplyCtx::legacy(&lines))
            .map(|o| o.entry)
    }

    /// The compiled pattern, or the build error as `{:?}` prints it.
    fn build_regex(&self) -> Result<Regex, String> {
        Regex::builder()
            .syntax(
                syntax::Config::new()
                    .multi_line(true)
                    .crlf(true)
                    .ignore_whitespace(self.verbose),
            )
            .build(&self.pattern)
            .map_err(|e| format!("{e:?}"))
    }

    /// The patch's matches in `text` as line windows (group 0), in text order. `allow(lines,
    /// removes)` can refuse a match, as the engine refuses matches on QuarterMaster's lines
    /// (D42). Empty when the pattern does not compile. The vanilla sites of a regex A (D52).
    pub fn site_windows(
        &self,
        text: &str,
        allow: impl Fn(Range<usize>, bool) -> bool,
    ) -> Vec<Window> {
        let Ok(re) = self.build_regex() else {
            return Vec::new();
        };
        let rope = Rope::from(text);
        re.captures_iter(Input::new((&rope).into_cursor()))
            .filter(|g| {
                self.match_lines(&rope, g)
                    .is_none_or(|(l, removes)| allow(l, removes))
            })
            .filter_map(|g| {
                let m = g.get_group(0)?;
                let lines = span_lines(&rope, m.start, m.end);
                Some(Window::contiguous(lines.start, lines.len(), String::new()))
            })
            .collect()
    }

    /// Whether an edge rung can place this patch (D52): it inserts (no `at`) and its payload
    /// and `line_prepend` read no capture group.
    pub fn takes_edge_rungs(&self) -> bool {
        self.position != InsertPosition::At
            && !uses_capture(&self.payload)
            && !uses_capture(&self.line_prepend)
    }

    /// Whether this patch reads its match: its payload or `line_prepend` reads a capture
    /// group, or it places code by `root_capture`. Such a patch cannot be copied to another
    /// line as a pattern patch (D59).
    pub fn reads_capture(&self) -> bool {
        self.root_capture.is_some()
            || uses_capture(&self.payload)
            || uses_capture(&self.line_prepend)
    }

    /// The text this patch inserts when it reads no capture: `line_prepend` on each line and
    /// `$$` as `$`, as the patch writes it. None when it reads a capture.
    pub fn plain_payload(&self) -> Option<String> {
        if self.reads_capture() {
            return None;
        }
        let mut payload = self.edge_payload();
        if !self.payload.ends_with('\n') {
            payload.pop();
        }
        Some(payload)
    }

    /// The payload for an edge rung's place (D52): no capture is read, so only `$$` changes.
    /// Always whole lines.
    fn edge_payload(&self) -> String {
        let mut prepend = String::new();
        interpolate::string(&self.line_prepend, |_, _| {}, |_| None, &mut prepend);
        let joined = std::format!(
            "{}",
            self.payload
                .split_inclusive('\n')
                .format_with("", |x, f| f(&format_args!("{}{}", prepend, x)))
        );
        let mut payload = String::new();
        interpolate::string(&joined, |_, _| {}, |_| None, &mut payload);
        if !payload.ends_with('\n') {
            payload.push('\n');
        }
        payload
    }

    /// `apply`, plus the edits in lines (§4.3). A whole-line insert becomes `Edit::Lines`
    /// hung on the line it goes next to, and stacks there like a pattern insert (D53); a
    /// whole-line replacement has no anchor; changes inside lines are `Edit::Touched` and keep
    /// their place. With rungs, the ladder decides each site (D52).
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

        let input = Input::new(rope.into_cursor());
        let re = match self.build_regex() {
            Ok(r) => r,
            Err(e) => {
                return Some(ApplyOutcome::unchanged(self.debug_from_warning_string(
                    path,
                    format!(
                        "Failed to compile Regex '{}' for regex patch from {}: {e}",
                        self.pattern,
                        path.display()
                    ),
                )));
            }
        };

        let mut captures = re.captures_iter(input).collect_vec();
        // D42: a match over a line QuarterMaster added is dropped, and so is one that rewrites
        // a line it changed in place. Only a whole-line insert next to such a line stays.
        let mut refused: Vec<(usize, usize)> = Vec::new();
        captures.retain(|groups| {
            let Some((lines, removes)) = self.match_lines(rope, groups) else {
                return true;
            };
            let blocked = ctx.lines.blocks_match(lines.clone(), removes);
            if blocked {
                refused.push((lines.start, lines.len()));
            }
            !blocked
        });
        // Named by the first refused line, as a rewrite would see it.
        let skip_warnings = ctx.lines.skip_warnings(&refused, true, target);
        for w in &skip_warnings {
            log::warn!("{w}");
        }

        // With rungs, the ladder decides each site; the matches it finds inside B join the
        // own matches (D52).
        let (mut captures, anchoring, ladder_warnings) = match ctx.ladder {
            None => (
                captures.into_iter().map(Item::Capture).collect_vec(),
                None,
                Vec::new(),
            ),
            Some(ladder) => {
                let rope_lines = rope.raw_lines().map(|x| x.to_string()).collect_vec();
                let line_refs = rope_lines.iter().map(String::as_str).collect_vec();
                let sites = RegexSites {
                    patch: self,
                    re: &re,
                    rope,
                    lines: ctx.lines,
                    found: std::cell::RefCell::new(captures),
                };
                let own = sites.own();
                let r = super::anchor::resolve(ladder, &sites, own, &line_refs, ctx, target);
                let found = sites.found.into_inner();
                let line_len = rope.line_len();
                let items = r
                    .places
                    .into_iter()
                    .filter_map(|p| match p.found {
                        Some(f) => found.get(f.ix).cloned().map(Item::Capture),
                        None => {
                            let line = match p.pos {
                                InsertPosition::After => p.line + p.len,
                                _ => p.line,
                            };
                            Some(Item::Edge {
                                at: rope.byte_of_line(line.min(line_len)),
                                place: (p.line, p.len),
                                pos: p.pos,
                            })
                        }
                    })
                    .collect_vec();
                for w in &r.warnings {
                    log::warn!("{w}");
                }
                (items, Some(r.report), r.warnings)
            }
        };

        if captures.is_empty() {
            let warning = format!(
                "Regex '{}' on target '{target}' for regex patch from {} resulted in no matches",
                self.pattern.escape_debug(),
                path.display()
            );
            let mut out = ApplyOutcome::unchanged(self.debug_from_warning_string(path, warning));
            if !skip_warnings.is_empty() || !ladder_warnings.is_empty() {
                let w = out.entry.warnings.get_or_insert_with(Vec::new);
                w.splice(0..0, ladder_warnings);
                w.extend(skip_warnings);
            }
            out.anchoring = anchoring;
            return Some(out);
        }
        let raw_match_count = captures.len();
        let mut warnings = ladder_warnings;
        warnings.extend(skip_warnings);
        if let Some(times) = self.times {
            fn warn_regex_mismatch(
                pattern: &str,
                target: &str,
                found_matches: usize,
                wanted_matches: usize,
                path: &Path,
            ) -> String {
                let warn_msg: String = if pattern.lines().count() > 1 {
                    format!("Regex '''\n{pattern}''' on target '{target}' for regex patch from {} resulted in {found_matches} matches, wanted {wanted_matches}", path.display())
                } else {
                    format!("Regex '{pattern}' on target '{target}' for regex patch from {} resulted in {found_matches} matches, wanted {wanted_matches}", path.display())
                };
                for line in warn_msg.lines() {
                    log::warn!("{}", line)
                }
                warn_msg
            }
            if captures.len() < times {
                warnings.push(warn_regex_mismatch(
                    &self.pattern,
                    target,
                    captures.len(),
                    times,
                    path,
                ));
            }
            if captures.len() > times {
                warnings.push(warn_regex_mismatch(
                    &self.pattern,
                    target,
                    captures.len(),
                    times,
                    path,
                ));
                log::warn!("Ignoring excess matches");
                warnings.push("Ignoring excess matches".to_string());
                captures.truncate(times);
            }
        }

        // The lines each placed match covers, before any change.
        let touches_builtin = captures.iter().any(|item| {
            let Item::Capture(groups) = item else {
                return false;
            };
            groups.get_group(0).is_some_and(|m| {
                let len = rope.byte_len();
                let first = rope.line_of_byte(m.start.min(len));
                let last = rope.line_of_byte(m.end.saturating_sub(1).max(m.start).min(len));
                (first..=last).any(|l| ctx.lines.is_builtin_line(l))
            })
        });

        // Running byte offset to keep byte references valid after rope mutations.
        let mut delta = 0_isize;
        // The text before the patch: anchors and stack slots are read in its lines.
        let pre = rope.clone();
        // Each item's lines before the patch, for the stacking guard.
        let windows: Vec<(usize, usize)> = captures
            .iter()
            .map(|item| match item {
                Item::Capture(groups) => groups.get_group(0).map_or((0, 0), |m| {
                    let l = span_lines(&pre, m.start, m.end);
                    (l.start, l.len())
                }),
                Item::Edge { place, .. } => *place,
            })
            .collect();
        let mut stack_moves = Vec::new();
        let attach_at = |line: usize, side: Side| {
            ctx.lines.get(line).map(|t| Attach {
                uid: t.uid,
                side,
                block: t.block,
            })
        };
        // A whole-line insert's place after stacking (D53): the byte to insert at, when the
        // slot is not today's.
        let mut stacked = |rope: &Rope, j: usize, site: u32, anchor: (usize, Side)| {
            let p = place_in_stack(ctx, anchor.1, anchor.0, &windows, j, site, LegacySlot::Near)?;
            stack_moves.push(p.report);
            let line_delta = rope.line_len() as isize - pre.line_len() as isize;
            let line = p.line?.saturating_add_signed(line_delta);
            Some(rope.byte_of_line(line.min(rope.line_len())))
        };

        // Collect byte regions during patching.
        let mut byte_regions: Vec<ByteRegion> = Vec::new();
        let mut edits: Vec<Edit> = Vec::new();
        // A match position after the changes so far, clamped to the rope.
        let shift = |pos: usize, delta: isize, rope: &Rope| -> usize {
            pos.saturating_add_signed(delta).min(rope.byte_len())
        };

        for (j, item) in captures.into_iter().enumerate() {
            let groups = match item {
                Item::Capture(groups) => groups,
                Item::Edge { at, place, pos } => {
                    // A literal insert of whole lines at an edge rung's place (D52). It hangs
                    // on the place's edge line and stacks there (D53).
                    let mut at = shift(at, delta, rope);
                    let mut payload = self.edge_payload();
                    let site = u32::try_from(edits.len()).unwrap_or(u32::MAX);
                    let anchor = match pos {
                        InsertPosition::After => {
                            (place.0 + place.1.saturating_sub(1), Side::After)
                        }
                        _ => (place.0, Side::Before),
                    };
                    if at > 0 && rope.byte(at - 1) != b'\n' {
                        payload.insert(0, '\n');
                    } else if let Some(moved) = stacked(rope, j, site, anchor) {
                        at = moved;
                        payload = stacked_text(rope, at, &payload).into_owned();
                    }
                    let payload_bytes = payload.len();
                    let attach = attach_at(anchor.0, anchor.1);
                    edits.push(byte_edit(rope, at, at, &payload, site, attach));
                    rope.insert(at, &payload);
                    byte_regions.push(ByteRegion {
                        start: at,
                        end: at + payload_bytes,
                        delta: payload_bytes as isize,
                    });
                    delta += payload_bytes as isize;
                    continue;
                }
            };
            // Get the entire captured span (index 0); every match has it.
            let Some(base) = groups.get_group(0) else {
                continue;
            };
            let base_start = shift(base.start, delta, rope);
            let base_end = shift(base.end, delta, rope).max(base_start);

            let base_str = rope.byte_slice(base_start..base_end).to_string();

            // A `$n` with no such group in this match interpolates as empty, with a warning.
            let mut missing_groups: Vec<String> = Vec::new();

            // Interpolate capture groups into self.line_prepend, if any capture groups exist within.
            let mut line_prepend = String::new();
            interpolate::string(
                &self.line_prepend,
                |index, dest| {
                    let Some(span) = groups.get_group(index) else {
                        missing_groups.push(format!("${index} in line_prepend"));
                        return;
                    };
                    let start = shift(span.start, delta, rope);
                    let end = shift(span.end, delta, rope).max(start);

                    let rope_slice = rope.byte_slice(start..end);

                    dest.push_str(&rope_slice.to_string());
                },
                |name| {
                    let pid = groups.pattern()?;
                    groups.group_info().to_index(pid, name)
                },
                &mut line_prepend,
            );

            // Cleanup and convert the specified root capture to a span. A missing one skips
            // the patch: nothing it changed so far is kept.
            let target_group = {
                let group_name = self.root_capture.as_deref().unwrap_or("0").replace('$', "");

                let found = if let Ok(idx) = group_name.parse::<usize>() {
                    groups.get_group(idx).ok_or_else(|| {
                        format!("The capture group at index {idx} could not be found in '{base_str}' with the Regex '{}' for regex patch from {}", self.pattern, path.display())
                    })
                } else {
                    groups.get_group_by_name(&group_name).ok_or_else(|| {
                        format!("The capture group with name '{group_name}' could not be found in '{base_str}' with the Regex '{}' for regex patch from {}", self.pattern, path.display())
                    })
                };
                match found {
                    Ok(span) => span,
                    Err(warning) => {
                        return Some(ApplyOutcome::unchanged(
                            self.debug_from_warning_string(path, warning),
                        ))
                    }
                }
            };

            let target_start = shift(target_group.start, delta, rope);
            let target_end = shift(target_group.end, delta, rope).max(target_start);

            let new_payload = std::format!(
                "{}",
                self.payload
                    .split_inclusive('\n')
                    .format_with("", |x, f| f(&format_args!("{}{}", line_prepend, x)))
            );

            let mut payload = String::new();

            interpolate::string(
                &new_payload,
                |index, dest| {
                    let Some(span) = groups.get_group(index) else {
                        missing_groups.push(format!("${index} in payload"));
                        return;
                    };
                    let start = shift(span.start, delta, rope);
                    let end = shift(span.end, delta, rope).max(start);

                    let rope_slice = rope.byte_slice(start..end);

                    dest.push_str(&rope_slice.to_string());
                },
                |name| {
                    let pid = groups.pattern()?;
                    groups.group_info().to_index(pid, name)
                },
                &mut payload,
            );

            for missing in missing_groups {
                let warning = format!(
                    "Regex '{}' for regex patch from {} has no capture group for {missing} in '{}'; it was left empty",
                    self.pattern.escape_debug(),
                    path.display(),
                    base_str.escape_debug()
                );
                log::warn!("{}", warning);
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }

            if payload.starts_with(|x: char| x.is_ascii_alphanumeric() || x == '_') {
                let pre_pt = if let InsertPosition::After = self.position {
                    target_end
                } else {
                    target_start
                };
                if pre_pt > 0 {
                    let byte_on_left = rope.byte(pre_pt - 1);
                    if byte_on_left.is_ascii_alphanumeric() || byte_on_left == b'_' {
                        payload.insert(0, ' ');
                    }
                }
            }

            if payload.ends_with(|x: char| x.is_ascii_alphanumeric() || x == '_') {
                let post_pt = if let InsertPosition::Before = self.position {
                    target_start
                } else {
                    target_end
                };
                if post_pt < rope.byte_len() {
                    let byte_on_right = rope.byte(post_pt);
                    if byte_on_right.is_ascii_alphanumeric() || byte_on_right == b'_' {
                        payload.push(' ');
                    }
                }
            }

            let site = u32::try_from(edits.len()).unwrap_or(u32::MAX);

            // Where the payload goes now, and the same place before the patch.
            let (start, end, pre_at) = match self.position {
                InsertPosition::Before => (target_start, target_start, target_group.start),
                InsertPosition::After => (target_end, target_end, target_group.end),
                InsertPosition::At => (target_start, target_end, target_group.start),
            };
            // Only a pure insert of whole lines hangs on a line and stacks (D53); a
            // replacement and an insert inside a line keep their place.
            let pure = start == end
                && matches!(
                    byte_edit(rope, start, start, &payload, site, None),
                    Edit::Lines { .. }
                );
            let anchor = if pure {
                insert_anchor(&pre, pre_at, self.position)
            } else {
                None
            };
            let whole = payload.ends_with('\n') && (start == 0 || rope.byte(start - 1) == b'\n');
            let moved = anchor
                .filter(|_| whole)
                .and_then(|a| stacked(rope, j, site, a));
            let (start, end) = moved.map_or((start, end), |m| (m, m));
            if let Some(m) = moved {
                payload = stacked_text(rope, m, &payload).into_owned();
            }
            let payload_bytes = payload.len();
            let attach = anchor.and_then(|(line, side)| attach_at(line, side));
            edits.push(byte_edit(rope, start, end, &payload, site, attach));
            let old_len = end - start;
            if old_len > 0 {
                rope.delete(start..end);
            }
            rope.insert(start, &payload);
            byte_regions.push(ByteRegion {
                start,
                end: start + payload_bytes,
                delta: payload_bytes as isize - old_len as isize,
            });
            delta -= old_len as isize;
            delta += payload_bytes as isize;
        }

        let entry = ByteDebugEntry {
            patch_source: PatchSource {
                file: path.display().to_string(),
                mod_id: self.mod_id.clone(),
                patch_id: self.id.clone(),
                patch_name: self.name.clone(),
                pattern: Some(self.pattern.clone()),
                patch_type: DebugPatchType::Regex,
            },
            regions: byte_regions,
            matched_count: raw_match_count,
            expected_count: self.times,
            warnings: if warnings.is_empty() {
                None
            } else {
                Some(warnings)
            },
        };
        Some(ApplyOutcome {
            entry,
            edits,
            touches_builtin,
            stack_moves,
            anchoring,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handshake::PatchOutcomeStatus;
    use crate::patch::table::PatchTable;
    use crate::patch::Patch;
    use std::path::PathBuf;

    fn regex_patch(pattern: &str, root_capture: Option<&str>, payload: &str) -> RegexPatch {
        RegexPatch {
            target: Target::Single("main.lua".to_string()),
            pattern: pattern.to_string(),
            position: InsertPosition::After,
            root_capture: root_capture.map(str::to_string),
            payload: payload.to_string(),
            line_prepend: String::new(),
            times: None,
            verbose: false,
            id: Some("m:main_lua:1".to_string()),
            name: Some("regex".to_string()),
            description: None,
            requires: Vec::new(),
            priority: None,
            before: Vec::new(),
            after: Vec::new(),
            on_failure: OnFailurePolicy::Warn,
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            mod_id: Some("m".to_string()),
            disabled: false,
            qm: Default::default(),
        }
    }

    fn run(p: RegexPatch, src: &str) -> (String, crate::handshake::PatchExecutionRecord) {
        let mut table = PatchTable::default();
        table.targets.insert("main.lua".to_string());
        table
            .patches
            .push((Patch::Regex(p), 50, PathBuf::from("m/qm.toml")));
        let (text, _, mut records) = table.apply_patches_offline("main.lua", src).unwrap();
        assert_eq!(records.len(), 1);
        (text, records.remove(0))
    }

    #[test]
    fn missing_group_interpolates_empty_with_warning() {
        // $3 does not exist and the optional group 2 does not take part in the match.
        let p = regex_patch(r"(local x)( = 9)?", None, " -- [$2|$3]");
        let (text, rec) = run(p, "local x = 1\n");
        assert_eq!(text, "local x -- [|] = 1\n");
        assert_eq!(rec.status, PatchOutcomeStatus::Applied);
        assert_eq!(rec.warnings.len(), 2, "{:?}", rec.warnings);
        assert!(rec.warnings.iter().any(|w| w.contains("$2 in payload")));
        assert!(rec.warnings.iter().any(|w| w.contains("$3 in payload")));
    }

    #[test]
    fn missing_group_in_line_prepend_warns() {
        let mut p = regex_patch(r"local x", None, "\n-- added\n");
        p.line_prepend = "$1".to_string();
        let (text, rec) = run(p, "local x = 1\n");
        assert_eq!(text, "local x\n-- added\n = 1\n");
        assert!(rec.warnings.iter().any(|w| w.contains("$1 in line_prepend")));
    }

    #[test]
    fn missing_root_capture_skips_with_warning() {
        let src = "local x = 1\nlocal x = 2\n";
        for root in ["$5", "nope"] {
            let p = regex_patch(r"local (?P<name>x)", Some(root), " -- y");
            let (text, rec) = run(p, src);
            assert_eq!(text, src, "root {root}: nothing is changed");
            assert_eq!(rec.status, PatchOutcomeStatus::ZeroMatches);
            assert_eq!(rec.warnings.len(), 1);
            assert!(rec.warnings[0].contains("could not be found"), "{:?}", rec.warnings);
        }
    }

    #[test]
    fn missing_root_capture_keeps_no_partial_edit() {
        // The first match has group 1, the second does not: the patch keeps neither.
        let src = "a1\nb\n";
        let mut p = regex_patch(r"(?m)^(?:a(1)|b)$", Some("1"), "!");
        p.on_failure = OnFailurePolicy::SkipPatch;
        let (text, rec) = run(p, src);
        assert_eq!(text, src);
        assert_eq!(rec.status, PatchOutcomeStatus::SkippedByPolicy);
    }

    /// Runs `pattern` on an empty target in the DLL and in the simulation.
    fn run_on_empty(pattern: &str) -> (String, PatchOutcomeStatus) {
        use crate::handshake::LaunchConfig;
        use crate::prepare::{dll_transform, simulate_buffer, DllOutcome};
        let target = "includes/empty.lua";
        let mut p = regex_patch(pattern, None, " -- y");
        p.target = Target::Single(target.to_string());
        let mut table = PatchTable::default();
        table.targets.insert(target.to_string());
        table
            .patches
            .push((Patch::Regex(p), 50, PathBuf::from("m/qm.toml")));
        let prep = table.prepare();
        let launch = LaunchConfig::default();

        let (dll_text, dll_status) =
            match dll_transform(&table, &prep, &launch, "@includes/empty.lua", "", false) {
                DllOutcome::Patched { text, records, .. } => {
                    assert_eq!(records.len(), 1);
                    (text, records[0].status)
                }
                other => panic!("expected Patched, got {other:?}"),
            };
        let (_, traced) = simulate_buffer(&table, &prep, &launch, target, "").unwrap();
        assert_eq!(traced.records.len(), 1);
        assert_eq!(traced.text, dll_text, "{pattern}: the DLL and the simulation agree");
        assert_eq!(traced.records[0].status, dll_status);
        (dll_text, dll_status)
    }

    #[test]
    fn empty_target_does_not_panic() {
        // An empty rope has no chunk; the regex cursor used to index chunk 0.
        let (text, status) = run_on_empty(r"local");
        assert_eq!(text, "");
        assert_eq!(status, PatchOutcomeStatus::ZeroMatches);
        // A pattern that matches the empty string reads the cursor's chunk (it panicked).
        let (text, status) = run_on_empty(r"x*");
        assert_eq!(text, " -- y");
        assert_eq!(status, PatchOutcomeStatus::Applied);
    }
}
