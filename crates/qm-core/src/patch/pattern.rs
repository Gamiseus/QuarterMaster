#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use std::borrow::Cow;
use std::cell::RefCell;
use std::ops::Range;
use std::path::Path;

use crop::Rope;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use wildmatch::WildMatch;

use crate::dump::{ByteDebugEntry, ByteRegion, DebugPatchType, PatchSource};
use crate::handshake::StackMove;
use crate::lenient::Lenient;
use crate::patch::anchor::{Rung, SiteKey};

use super::anchor::{Place, SiteSource};
use super::edits::{self, ApplyAs, EditsRefusal, Hunk, Keeper, Normalized, SkipAt};
use super::linemap::{byte_edit, Attach, Edit, LineKind, LineMap, LineTag, Side, NO_LINE, NO_PATCH};
use super::stack::{place_in_stack, stacked_text, LegacySlot, StackHint};
use super::{ApplyCtx, ApplyOutcome, InsertPosition, OnFailurePolicy, Priority, Target};

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PatternPatch {
    // The pattern that the line will be matched against. Very simple,
    // supports only `?` (one occurrence of any character) and `*` (any number of any character).
    // Patterns are matched against a left-trimmed version of the line, so whitespace does not
    // need to be considered.
    pub pattern: String,

    // The position to insert the target at. `PatternAt::At` replaces the matched line entirely.
    pub position: InsertPosition,
    pub target: Target,
    pub payload: String,
    #[serde(default)]
    pub match_indent: bool,
    // Apply patch at most `times` (or `expect`) times, warn if the number of matches differs.
    #[serde(default, alias = "expect")]
    pub times: Option<usize>,

    /// We keep this field around for legacy compat. It doesn't do anything (and never has).
    #[serde(default)]
    pub overwrite: bool,

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
    /// Rungs: places on another patch's code this patch goes to first (Phase C §3.6). A bad
    /// entry is skipped and reported, and so is a value that is not a list; the patch still
    /// loads and falls back to `pattern`.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "crate::lenient::lenient_list",
        default
    )]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub anchors: Vec<Lenient<Rung>>,
    /// Which of the patch's own places the rungs are for (none: all of them). A bad entry, or
    /// a value that is not a list, is skipped.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "crate::lenient::lenient_list",
        default
    )]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub sites: Vec<Lenient<SiteKey>>,
    /// `"first"` or `"last"`: where this patch's block goes among the code other patches add
    /// at its line (D57). A bad value is ignored with a warning.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub stack: Option<Lenient<StackHint>>,
    /// `"edits"` runs an `at` patch as separate edits next to the lines it keeps (D46). Absent
    /// or `"literal"`: one replacement. A profile fix or a compat mod can change it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub apply_as: Option<Lenient<ApplyAs>>,

    #[serde(skip)]
    pub mod_id: Option<String>,

    #[serde(skip)]
    pub disabled: bool,

    /// Profile and compat data (Phase C §3.3); never read from a patch file.
    #[serde(skip)]
    pub qm: crate::patch::plan::PatchExtra,
}

impl PatternPatch {
    pub fn debug_from_warning_string(&self, path: &Path, warning: String) -> ByteDebugEntry {
        log::warn!("{}", warning);
        ByteDebugEntry {
            patch_source: PatchSource {
                file: path.display().to_string(),
                mod_id: self.mod_id.clone(),
                patch_id: self.id.clone(),
                patch_name: self.name.clone(),
                pattern: Some(self.pattern.clone()),
                patch_type: DebugPatchType::Pattern,
            },
            regions: Vec::new(),
            matched_count: 0,
            expected_count: self.times,
            warnings: Some(vec![warning]),
        }
    }

    /// The mode this patch asks for: a profile fix or compat mod's, else the author's (D13).
    pub fn asked_apply_as(&self) -> ApplyAs {
        self.qm
            .apply_as
            .or_else(|| self.apply_as.as_ref().and_then(Lenient::ok).copied())
            .unwrap_or_default()
    }

    /// Whether this patch runs in edits mode: an `at` patch that asks for it (D46).
    pub fn edits_mode(&self) -> bool {
        self.position == InsertPosition::At && self.asked_apply_as() == ApplyAs::Edits
    }

    /// Whether this patch's windows may skip other patches' new lines (D48): it asks for edits
    /// mode. A before or after patch then finds a split site again; an `at` patch only when its
    /// lines pair, since a literal run would remove the skipped lines.
    pub fn split_windows(&self) -> bool {
        self.asked_apply_as() == ApplyAs::Edits
    }

    /// The literal pattern lines of an exact adjusted copy (D59): each matches only a line
    /// with that text, trimmed. None for every other patch, whose lines are wildcards.
    pub fn exact_lines(&self) -> Option<&[String]> {
        self.qm.exact_lines.as_deref()
    }

    /// The normalized form of this patch's pattern and payload (D47).
    pub fn normalized(&self) -> Normalized {
        edits::normalize_patch(&self.pattern, &self.payload)
    }

    /// Whether this `at` patch can take rungs (§4.7): it runs as edits that only keep lines
    /// and add new ones.
    pub fn inserts_only(&self) -> bool {
        self.edits_mode() && self.normalized().is_ok_and(|h| edits::inserts_only(&h))
    }

    /// This patch's matches in `lines` (the text before it runs), split as D48 allows: `hunks`
    /// are its edits-mode hunks, when it runs as edits.
    pub(crate) fn own_windows(
        &self,
        lines: &[&str],
        wm: &[WildMatch],
        ctx: &ApplyCtx,
        hunks: Option<&[Hunk]>,
        allow: impl Fn(&Window) -> bool,
    ) -> Vec<Window> {
        let split =
            self.split_windows() && (self.position != InsertPosition::At || hunks.is_some());
        find_split_windows(
            lines,
            wm,
            self.exact_lines(),
            |l| split && skippable(ctx.lines, ctx.me, l),
            allow,
        )
    }

    /// This patch's places in the game's file that no strict window holds, matched with the
    /// lines of its skipped edits relaxed (D49): such a line is the game's line with its text,
    /// or a line the kept patch made where the game's line was. At most one window per place,
    /// so a skipped edit never adds a match. Other lines must be the game's, still matching,
    /// and only other patches' new lines may sit between them (D48).
    fn relaxed_windows(
        &self,
        lines: &[&str],
        wm: &[WildMatch],
        ctx: &ApplyCtx,
        relaxed: &[Option<Keeper>],
        strict: &[Window],
    ) -> Vec<Window> {
        let Some(original) = ctx.original else {
            return Vec::new();
        };
        let tags = ctx.lines.tags();
        // The game's line -> where it is now (the first line that stands for it).
        let mut at_of: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for (i, t) in tags.iter().enumerate() {
            if t.vanilla != NO_LINE && t.kind != LineKind::Inserted {
                at_of.entry(t.vanilla).or_insert(i);
            }
        }
        let at = |v: usize| u32::try_from(v).ok().and_then(|v| at_of.get(&v).copied());
        // By maker, not owner: a line another `at` kept is still the kept patch's change.
        let owns = |k: Keeper, t: &LineTag| {
            t.maker != NO_PATCH
                && t.maker != ctx.me
                && match k {
                    Keeper::Any => true,
                    Keeper::Patch(p) => t.maker == p,
                    Keeper::Gone => false,
                }
        };
        let source = original.get();
        let taken: Vec<u32> = strict
            .iter()
            .flat_map(|w| w.lines.iter())
            .filter_map(|&l| tags.get(l).map(|t| t.vanilla))
            .filter(|&v| v != NO_LINE)
            .collect();
        let mut out: Vec<Window> = Vec::new();
        for site in SiteSource::vanilla(self, original, ctx.lines) {
            let held = (site.line..site.line + site.len)
                .any(|v| u32::try_from(v).is_ok_and(|v| taken.contains(&v)));
            if held {
                continue;
            }
            let mut found: Vec<usize> = Vec::with_capacity(wm.len());
            let mut ok = true;
            for (p, m) in wm.iter().enumerate() {
                let v = site.line + p;
                let keeper = relaxed.get(p).copied().flatten();
                let here = at(v).filter(|&l| match keeper {
                    None => lines.get(l).is_some_and(|x| m.matches(x.trim())),
                    Some(k) => {
                        let same = lines.get(l).map(|x| x.trim()) == source.get(v).map(|x| x.trim());
                        same || tags.get(l).is_some_and(|t| owns(k, t))
                    }
                });
                let line = match (here, keeper) {
                    (Some(l), _) => Some(l),
                    (None, None) => None,
                    // The game's line is gone: the kept patch's first line in its gap.
                    (None, Some(k)) if at(v).is_none() => {
                        let lo = match found.last() {
                            Some(&l) => l + 1,
                            None => (0..v).rev().find_map(&at).map_or(0, |l| l + 1),
                        };
                        let hi = (v + 1..source.len())
                            .find_map(&at)
                            .unwrap_or(lines.len());
                        (lo..hi).find(|&l| tags.get(l).is_some_and(|t| owns(k, t)))
                    }
                    (None, Some(_)) => None,
                };
                match line {
                    Some(l) if found.last().is_none_or(|&prev| l > prev) => found.push(l),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            let (Some(&first), Some(&last)) = (found.first(), found.last()) else {
                continue;
            };
            let gaps_ok = (first..=last)
                .all(|l| found.contains(&l) || skippable(ctx.lines, ctx.me, l));
            let overlaps = strict
                .iter()
                .chain(&out)
                .any(|w| first < w.line + w.len && w.line <= last);
            if !ok || !gaps_ok || overlaps {
                continue;
            }
            out.push(Window {
                line: first,
                len: last + 1 - first,
                indent: lines
                    .get(first)
                    .map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect())
                    .unwrap_or_default(),
                lines: found,
            });
        }
        out
    }

    /// Apply the pattern patch onto the rope.
    /// Returns `Some(ByteDebugEntry)` if the target matched this patch's target filter.
    pub fn apply(&self, target: &str, rope: &mut Rope, path: &Path) -> Option<ByteDebugEntry> {
        let lines = LineMap::new(rope.line_len(), &[], &[]);
        self.apply_in(target, rope, path, &ApplyCtx::legacy(&lines))
            .map(|o| o.entry)
    }

    /// `apply`, plus the edits in lines (§4.3). `ctx.lines` is the map before this patch.
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

        let wm_lines = self
            .pattern
            .lines()
            .map(|x| x.trim())
            .map(WildMatch::new)
            .collect_vec();
        if wm_lines.is_empty() {
            return Some(ApplyOutcome::unchanged(self.debug_from_warning_string(
                path,
                format!(
                    "Pattern on target '{target}' for pattern patch from {} has no lines",
                    path.display()
                ),
            )));
        }

        let rope_lines = rope.raw_lines().map(|x| x.to_string()).collect_vec();
        let line_refs = rope_lines.iter().map(String::as_str).collect_vec();
        // Edits mode (D46): the hunks `prepare` made, or made here without a table. A refused
        // normalization runs literal and says why.
        let normalized: Option<Cow<Normalized>> = self.edits_mode().then(|| match ctx.hunks {
            Some(h) => Cow::Borrowed(h),
            None => Cow::Owned(self.normalized()),
        });
        let all_hunks: Option<&[Hunk]> = match normalized.as_deref() {
            Some(Ok(h)) => Some(h.as_slice()),
            _ => None,
        };
        // Skipped edits (D49) do not run. Their pattern lines match the game's text, or the
        // kept patch's lines, at this patch's own places in the game's file.
        let skipped: Vec<(usize, &SkipAt)> = all_hunks.map_or_else(Vec::new, |h| {
            ctx.skips
                .iter()
                .filter_map(|s| hunk_index(h, s.ordinal).map(|i| (i, s)))
                .collect()
        });
        let active: Option<Vec<Hunk>> = all_hunks.map(|h| {
            h.iter()
                .enumerate()
                .filter(|(i, _)| !skipped.iter().any(|(k, _)| k == i))
                .map(|(_, x)| x.clone())
                .collect()
        });
        let hunks: Option<&[Hunk]> = active.as_deref();
        let mut relaxed: Vec<Option<Keeper>> = vec![None; wm_lines.len()];
        for (i, s) in &skipped {
            if let Some(Hunk::Change { p, .. } | Hunk::Delete { p }) =
                all_hunks.and_then(|h| h.get(*i))
            {
                for slot in relaxed.iter_mut().skip(p.start).take(p.len()) {
                    *slot = Some(s.keep);
                }
            }
        }
        // Long strings and comments of the text, read once for every site.
        let long = hunks.map(|_| crate::lua_scan::scan(&rope.to_string()));
        // QuarterMaster's added lines never match; an `at` never matches a line it rewrote in
        // place (D42). Refused windows are reported.
        let removes = self.position == InsertPosition::At;
        let refused: RefCell<Vec<(usize, usize)>> = RefCell::new(Vec::new());
        let split_refused: RefCell<Vec<(usize, EditsRefusal)>> = RefCell::new(Vec::new());
        let allow = |w: &Window| {
            if ctx.lines.blocks_match(w.line..w.line + w.len, removes) {
                refused.borrow_mut().push((w.line, w.len));
                return false;
            }
            // A split site never runs literal: that would remove the other mod's lines.
            let why = match hunks {
                _ if w.lines.len() == w.len => None,
                Some(h) if !edits::split_ok(h, &w.lines) => Some(EditsRefusal::SplitChange),
                Some(_)
                    if long.as_ref().is_some_and(|s| {
                        edits::site_in_long(s, w.line..w.line + w.len)
                    }) =>
                {
                    Some(EditsRefusal::LongString)
                }
                _ => None,
            };
            if let Some(why) = why {
                split_refused.borrow_mut().push((w.line, why));
            }
            why.is_none()
        };
        let mut own = self.own_windows(&line_refs, &wm_lines, ctx, hunks, allow);
        if relaxed.iter().any(Option::is_some) {
            let more = self.relaxed_windows(&line_refs, &wm_lines, ctx, &relaxed, &own);
            own.extend(more.into_iter().filter(|w| allow(w)));
            own.sort_by_key(|w| w.line);
        }
        let mut skip_warnings = ctx
            .lines
            .skip_warnings(&refused.into_inner(), removes, target);
        for (line, why) in split_refused.into_inner() {
            let at = ctx
                .lines
                .get(line)
                .and_then(|t| ctx.lines.file_line(t.uid))
                .unwrap_or(line + 1);
            skip_warnings.push(edits::skipped_warning(
                why,
                &format!("the match at line {at} of '{target}'"),
            ));
        }
        for w in &skip_warnings {
            log::warn!("{w}");
        }
        // With rungs, the ladder decides each site (§6.1). An `at` patch has one only in edits
        // mode, when it only keeps lines and adds new ones (§4.7).
        let ladder = ctx
            .ladder
            .filter(|_| self.position != InsertPosition::At || hunks.is_some_and(edits::inserts_only));
        let own: Vec<Place> = own
            .into_iter()
            .map(|w| Place::from_window(w, self.position))
            .collect();
        let (mut matches, anchoring, ladder_warnings) = match ladder {
            Some(ladder) => {
                let r = super::anchor::resolve(ladder, self, own, &line_refs, ctx, target);
                (r.places, Some(r.report), r.warnings)
            }
            None => (own, None, Vec::new()),
        };
        for w in &ladder_warnings {
            log::warn!("{w}");
        }

        if matches.is_empty() {
            let mut out = ApplyOutcome::unchanged(self.debug_from_warning_string(
                path,
                format!(
                    "Pattern '{}' on target '{target}' for pattern patch from {} resulted in no matches",
                    self.pattern.escape_debug(),
                    path.display(),
                ),
            ));
            // Why it ran literal, as on the matched path (D47).
            let refused = normalized
                .as_deref()
                .and_then(|n| n.as_ref().err())
                .map(|reason| edits::refused_warning(*reason, None));
            if let Some(w) = &refused {
                log::warn!("{w}");
            }
            if !ladder_warnings.is_empty() || !skip_warnings.is_empty() || refused.is_some() {
                let w = out.entry.warnings.get_or_insert_with(Vec::new);
                w.splice(0..0, ladder_warnings);
                w.extend(skip_warnings);
                w.extend(refused);
            }
            out.anchoring = anchoring;
            return Some(out);
        }
        let raw_match_count = matches.len();
        let mut warnings = ladder_warnings;
        warnings.extend(skip_warnings);
        if let Some(Err(reason)) = normalized.as_deref() {
            let w = edits::refused_warning(*reason, None);
            log::warn!("{w}");
            warnings.push(w);
        }
        for (_, s) in &skipped {
            let w = edits::skipped_hunk_warning(s.ordinal, &s.vanilla, s.keep_name.as_deref());
            log::info!("{w}");
            warnings.push(w);
        }
        if let Some(times) = self.times {
            fn warn_pattern_mismatch(
                pattern: &str,
                target: &str,
                found_matches: usize,
                wanted_matches: usize,
                path: &Path,
            ) -> String {
                let warn_msg: String = if pattern.lines().count() > 1 {
                    format!("Pattern '''\n{pattern}''' on target '{target}' for pattern patch from {} resulted in {found_matches} matches, wanted {wanted_matches}", path.display())
                } else {
                    format!("Pattern '{pattern}' on target '{target}' for pattern patch from {} resulted in {found_matches} matches, wanted {wanted_matches}", path.display())
                };
                for line in warn_msg.lines() {
                    log::warn!("{}", line)
                }
                warn_msg
            }
            if matches.len() < times {
                warnings.push(warn_pattern_mismatch(
                    &self.pattern,
                    target,
                    matches.len(),
                    times,
                    path,
                ));
            }
            if matches.len() > times {
                warnings.push(warn_pattern_mismatch(
                    &self.pattern,
                    target,
                    matches.len(),
                    times,
                    path,
                ));
                log::warn!("Ignoring excess matches");
                warnings.push("Ignoring excess matches".to_string());
                matches.truncate(times);
            }
        }

        let touches_builtin = matches
            .iter()
            .any(|w| (w.line..w.line + w.len).any(|l| ctx.lines.is_builtin_line(l)));

        // Track the +/- index offset caused by previous line injections.
        let mut line_delta: isize = 0;

        // Collect byte regions during patching.
        let mut byte_regions: Vec<ByteRegion> = Vec::new();
        let mut edits: Vec<Edit> = Vec::new();
        let windows: Vec<(usize, usize)> = matches.iter().map(|w| (w.line, w.len)).collect();
        let mut stack_moves: Vec<StackMove> = Vec::new();

        for (j, window) in matches.into_iter().enumerate() {
            let site = u32::try_from(edits.len()).unwrap_or(u32::MAX);
            let indent = if self.match_indent {
                window.indent.as_str()
            } else {
                ""
            };
            let adjusted_line_idx = window.line.saturating_add_signed(line_delta);
            let line_count = rope.line_len();
            if adjusted_line_idx + window.len > line_count {
                warnings.push(format!(
                    "Pattern '{}' on target '{target}' for pattern patch from {}: match at line {} is past the end of the text; clamped",
                    self.pattern.escape_debug(),
                    path.display(),
                    adjusted_line_idx + 1
                ));
            }
            let start = rope.byte_of_line(adjusted_line_idx.min(line_count));
            let end = rope.byte_of_line((adjusted_line_idx + window.len).min(line_count));

            let mut payload = self
                .payload
                .split_inclusive('\n')
                .format_with("", |x, f| f(&format_args!("{}{}", indent, x)))
                .to_string();
            if !self.payload.ends_with('\n') {
                payload.push('\n');
            }
            let payload_lines = payload.lines().count() as isize;
            let payload_bytes = payload.len();
            // The anchor line, read from the map before this patch. When the lookup fails, the
            // commit hangs the insert on a neighbour line under the line rules (D51).
            let attach = |line: usize, side: Side| {
                ctx.lines.get(line).map(|t| Attach {
                    uid: t.uid,
                    side,
                    block: t.block,
                })
            };

            // A stacked insert's byte offset, when it moved off the legacy slot.
            let mut moved_to = |side: Side| -> Option<usize> {
                let anchor = match side {
                    Side::Before => window.line,
                    Side::After => window.line + window.len.saturating_sub(1),
                };
                let p = place_in_stack(ctx, side, anchor, &windows, j, site, LegacySlot::Near)?;
                stack_moves.push(p.report);
                let line = p.line?.saturating_add_signed(line_delta);
                Some(rope.byte_of_line(line.min(rope.line_len())))
            };

            match window.pos {
                InsertPosition::Before => {
                    let moved = moved_to(Side::Before);
                    let start = moved.unwrap_or(start);
                    let text = match moved {
                        Some(at) => stacked_text(rope, at, &payload),
                        None => Cow::Borrowed(payload.as_str()),
                    };
                    edits.push(byte_edit(
                        rope,
                        start,
                        start,
                        &text,
                        site,
                        attach(window.line, Side::Before),
                    ));
                    rope.insert(start, &text);
                    byte_regions.push(ByteRegion {
                        start,
                        end: start + text.len(),
                        delta: text.len() as isize,
                    });
                    line_delta += payload_lines;
                }
                InsertPosition::After => {
                    let last = window.line + window.len.saturating_sub(1);
                    let moved = moved_to(Side::After);
                    let end = moved.unwrap_or(end);
                    let text = match moved {
                        Some(at) => stacked_text(rope, at, &payload),
                        None => Cow::Borrowed(payload.as_str()),
                    };
                    edits.push(byte_edit(
                        rope,
                        end,
                        end,
                        &text,
                        site,
                        attach(last, Side::After),
                    ));
                    rope.insert(end, &text);
                    byte_regions.push(ByteRegion {
                        start: end,
                        end: end + text.len(),
                        delta: text.len() as isize,
                    });
                    line_delta += payload_lines;
                }
                InsertPosition::At => {
                    let in_long = long.as_ref().is_some_and(|s| {
                        edits::site_in_long(s, window.line..window.line + window.len)
                    });
                    if in_long {
                        let line = ctx
                            .lines
                            .get(window.line)
                            .and_then(|t| ctx.lines.file_line(t.uid))
                            .unwrap_or(window.line + 1);
                        let site = format!("the match at line {line} of '{target}'");
                        // A split site never runs literal (D48).
                        let split = window.lines.len() != window.len;
                        let w = if split {
                            edits::skipped_warning(EditsRefusal::LongString, &site)
                        } else {
                            edits::refused_warning(EditsRefusal::LongString, Some(site))
                        };
                        log::warn!("{w}");
                        warnings.push(w);
                        if split {
                            continue;
                        }
                    }
                    if let Some(hunks) = hunks.filter(|_| !in_long) {
                        let lit = edits::literal_payload(&self.payload, indent);
                        line_delta += apply_hunks(
                            ctx,
                            rope,
                            &mut edits,
                            &mut byte_regions,
                            &mut stack_moves,
                            Site {
                                hunks,
                                lit: &lit,
                                lines: &window.lines,
                                windows: &windows,
                                j,
                                line_delta,
                            },
                        );
                        continue;
                    }
                    edits.push(byte_edit(rope, start, end, &payload, site, None));
                    let removed_bytes = end - start;
                    rope.delete(start..end);
                    rope.insert(start, &payload);
                    byte_regions.push(ByteRegion {
                        start,
                        end: start + payload_bytes,
                        delta: payload_bytes as isize - removed_bytes as isize,
                    });
                    line_delta += payload_lines - window.len as isize;
                }
            };
        }

        let entry = ByteDebugEntry {
            patch_source: PatchSource {
                file: path.display().to_string(),
                mod_id: self.mod_id.clone(),
                patch_id: self.id.clone(),
                patch_name: self.name.clone(),
                pattern: Some(self.pattern.clone()),
                patch_type: DebugPatchType::Pattern,
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

/// The index in `hunks` of edit `ordinal` (`edits::edit_hunks`).
fn hunk_index(hunks: &[Hunk], ordinal: u32) -> Option<usize> {
    hunks
        .iter()
        .enumerate()
        .filter(|(_, h)| !matches!(h, Hunk::Keep { .. }))
        .nth(ordinal as usize)
        .map(|(i, _)| i)
}

/// One edits-mode site (D47): the hunks, the literal payload with the site's indent, the
/// window's matched lines (pre-patch; pattern line `p` is `lines[p]`, D48), every window of
/// the patch, this one's index, and the line shift of the earlier sites.
struct Site<'a> {
    hunks: &'a [Hunk],
    lit: &'a str,
    lines: &'a [usize],
    windows: &'a [(usize, usize)],
    j: usize,
    line_delta: isize,
}

/// Emits one edit per hunk of an edits-mode site, top to bottom: kept lines stay untouched,
/// inserts hang on the kept line next to them and stack there, changes and removals replace
/// their own lines. Inserted and changed lines get the literal bytes. Lines a split window
/// skipped stay where they are; an insert never moves past the next matched line. Returns the
/// lines added.
fn apply_hunks(
    ctx: &ApplyCtx,
    rope: &mut Rope,
    edits: &mut Vec<Edit>,
    regions: &mut Vec<ByteRegion>,
    stack_moves: &mut Vec<StackMove>,
    s: Site,
) -> isize {
    let qlines: Vec<&str> = s.lit.split_inclusive('\n').collect();
    let text_of = |q: &Range<usize>| -> String {
        qlines
            .get(q.clone())
            .map(|v| v.concat())
            .unwrap_or_default()
    };
    let mut local: isize = 0;
    for h in s.hunks {
        let site = u32::try_from(edits.len()).unwrap_or(u32::MAX);
        let delta = s.line_delta + local;
        let (from, removed, content, added) = match h {
            Hunk::Keep { .. } => continue,
            Hunk::Insert { after_p, q } => {
                let ((anchor, side), next) = match after_p {
                    Some(p) => (
                        (s.lines.get(*p).copied(), Side::After),
                        s.lines.get(p + 1).copied(),
                    ),
                    None => ((s.lines.first().copied(), Side::Before), None),
                };
                let Some(anchor) = anchor else { continue };
                let mut line = match side {
                    Side::After => anchor + 1,
                    Side::Before => anchor,
                };
                // Between two matched lines the next one bounds the slot, as the next site
                // does for the last one.
                let inside;
                let (guard, j) = match next {
                    Some(n) => {
                        inside = [(anchor, 1), (n, 1)];
                        (&inside[..], 0)
                    }
                    None => (s.windows, s.j),
                };
                if let Some(p) = place_in_stack(ctx, side, anchor, guard, j, site, LegacySlot::Near)
                {
                    stack_moves.push(p.report);
                    if let Some(l) = p.line {
                        line = l;
                    }
                }
                let at = rope.byte_of_line(line.saturating_add_signed(delta).min(rope.line_len()));
                let mut content = text_of(q);
                let len = rope.byte_len();
                // After an unterminated last line the new lines start with its newline.
                if at == len && len > 0 && rope.byte(len - 1) != b'\n' {
                    content = format!("\n{}", content.strip_suffix('\n').unwrap_or(&content));
                }
                let attach = ctx.lines.get(anchor).map(|t| Attach {
                    uid: t.uid,
                    side,
                    block: t.block,
                });
                edits.push(byte_edit(rope, at, at, &content, site, attach));
                rope.insert(at, &content);
                regions.push(ByteRegion {
                    start: at,
                    end: at + content.len(),
                    delta: content.len() as isize,
                });
                local += q.len() as isize;
                continue;
            }
            // `split_ok` made sure these lines have nothing between them.
            Hunk::Change { p, q } => match s.lines.get(p.start) {
                Some(&l) => (l, p.len(), text_of(q), q.len()),
                None => continue,
            },
            Hunk::Delete { p } => match s.lines.get(p.start) {
                Some(&l) => (l, p.len(), String::new(), 0),
                None => continue,
            },
        };
        let count = rope.line_len();
        let from = from.saturating_add_signed(delta);
        let start = rope.byte_of_line(from.min(count));
        let end = rope.byte_of_line((from + removed).min(count));
        edits.push(byte_edit(rope, start, end, &content, site, None));
        rope.delete(start..end);
        rope.insert(start, &content);
        regions.push(ByteRegion {
            start,
            end: start + content.len(),
            delta: content.len() as isize - (end - start) as isize,
        });
        local += added as isize - removed as isize;
    }
    local
}

/// One match of a pattern: its first line (0-based), its line count from the first to the
/// last matched line, the first line's leading spaces and tabs, and the matched lines. A
/// window in edits mode may skip other patches' new lines (D48); then `lines` has fewer
/// entries than `len`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub line: usize,
    pub len: usize,
    pub indent: String,
    pub lines: Vec<usize>,
}

impl Window {
    /// A window of `len` lines with nothing skipped.
    pub fn contiguous(line: usize, len: usize, indent: String) -> Self {
        Window {
            line,
            len,
            indent,
            lines: (line..line + len).collect(),
        }
    }
}

/// How many times a split window search backs up from one start line (D48).
pub const SPLIT_STEPS: usize = 64;

/// The non-overlapping windows of `lines` that match, top to bottom: each line, trimmed,
/// against `wm`, or, with `exact`, equal to the trimmed `exact` line. `allow(line, len)` can
/// reject a window; scanning then goes on from the next line.
pub fn find_windows(
    lines: &[&str],
    wm: &[WildMatch],
    exact: Option<&[String]>,
    allow: impl Fn(usize, usize) -> bool,
) -> Vec<Window> {
    find_split_windows(lines, wm, exact, |_| false, |w| allow(w.line, w.len))
}

/// `find_windows`, where a window may skip the lines `skip` names (D48). It starts and ends
/// on matched lines. A line is matched when it can be and skipped only when the rest of the
/// window then cannot match, at most `SPLIT_STEPS` times per start line, so of two windows
/// from one start the shorter wins.
pub fn find_split_windows(
    lines: &[&str],
    wm: &[WildMatch],
    exact: Option<&[String]>,
    skip: impl Fn(usize) -> bool,
    allow: impl Fn(&Window) -> bool,
) -> Vec<Window> {
    let n = exact.map_or(wm.len(), |e| e.len());
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    let hit = |k: usize, l: usize| -> bool {
        let Some(source) = lines.get(l) else {
            return false;
        };
        match exact {
            Some(e) => e.get(k).is_some_and(|want| source.trim() == want.trim()),
            None => wm.get(k).is_some_and(|m| m.matches(source.trim())),
        }
    };
    let skip = |l: usize| l < lines.len() && skip(l);
    let mut matched: Vec<usize> = Vec::with_capacity(n);
    let mut i = 0usize;
    while i + n <= lines.len() {
        if hit(0, i) && match_from(i, n, &hit, &skip, &mut matched) {
            let last = matched.last().copied().unwrap_or(i);
            let window = Window {
                line: i,
                len: last + 1 - i,
                indent: lines
                    .get(i)
                    .map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect())
                    .unwrap_or_default(),
                lines: matched.clone(),
            };
            if allow(&window) {
                out.push(window);
                i = last + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Matches the `n` pattern lines from line `start` (pattern line 0 matched there) into
/// `matched`: the next pattern line goes on the next line that matches, past lines `skip`
/// allows. When the rest cannot match, the last matched line that `skip` allows is skipped
/// instead, at most `SPLIT_STEPS` times.
fn match_from(
    start: usize,
    n: usize,
    hit: &impl Fn(usize, usize) -> bool,
    skip: &impl Fn(usize) -> bool,
    matched: &mut Vec<usize>,
) -> bool {
    matched.clear();
    matched.push(start);
    let mut l = start + 1;
    let mut steps = 0usize;
    loop {
        let k = matched.len();
        if k >= n {
            return true;
        }
        if hit(k, l) {
            matched.push(l);
            l += 1;
            continue;
        }
        if skip(l) {
            l += 1;
            continue;
        }
        // Back up: skip the last matched line that may be skipped, and go on below it.
        loop {
            if matched.len() <= 1 || steps >= SPLIT_STEPS {
                return false;
            }
            steps += 1;
            match matched.pop() {
                Some(p) if skip(p) => {
                    l = p + 1;
                    break;
                }
                Some(_) => {}
                None => return false,
            }
        }
    }
}

/// Whether a split window of patch `me` may skip line `l` (D48): new code another patch
/// inserted. Kept and changed lines, the game's and QuarterMaster's never are.
pub(crate) fn skippable(lines: &LineMap, me: u32, l: usize) -> bool {
    lines
        .get(l)
        .is_some_and(|t| t.kind == LineKind::Inserted && t.owner != me)
}

#[cfg(test)]
#[path = "split_tests.rs"]
mod split_tests;

#[cfg(test)]
#[path = "skip_tests.rs"]
mod skip_tests;
