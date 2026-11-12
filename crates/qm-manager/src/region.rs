//! Region options (Phase C §9.2): where a patch's code can go when another patch took the line
//! it hangs on, and the other patch's places for the anchor picker.
//!
//! Everything here reads the cached simulation (its losses and traces); the service previews
//! each option before the UI offers it. Nothing here writes or changes game output.

use std::collections::HashMap;

use wildmatch::WildMatch;

use qm_core::handshake::PatchExecutionRecord;
use qm_core::lua_scan::scan;
use qm_core::patch::anchor::{Edge, Rung, SiteKey};
use qm_core::patch::edits::{self, Hunk};
use qm_core::patch::fingerprint::{short_hash, site_context};
use qm_core::patch::linemap::match_filter;
use qm_core::patch::origin::{FileTrace, LineOrigin};
use qm_core::patch::pattern::{find_windows, Window};
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, Patch, PatternPatch};
use qm_core::prepare::{prepare_buffer, PreparedBuffer};

use crate::conflict::{launch_config_of, SimulationReport};
use crate::consumer::{ConsumerInfo, ConsumerKind, SiteLoss};
use crate::placement::{edge_checks, EdgeCheck};

/// Which patch wrote a line, and how: "inserted", "kept" (written back unchanged) or
/// "modified" (changed in place).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineSource {
    pub kind: &'static str,
    pub patch_id: String,
    pub mod_id: String,
}

/// One line of another patch's place, as the picker lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionLine {
    /// 1-based, as the file shows it.
    pub line: u32,
    pub text: String,
    /// The line before `[vars]` values were filled in (what patterns match).
    pub raw_text: String,
    pub source: Option<LineSource>,
    /// The line is the other patch's own output; other lines cannot be picked.
    pub in_b: bool,
    /// How many of the other patch's lines at this place read the same (trimmed).
    pub matches_in_region: Option<u32>,
}

/// One place of the other patch's change in the patched file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BSite {
    /// The other patch's site index (0-based), as a rung's `b_site` names it.
    pub site: u32,
    pub first_line: u32,
    pub last_line: u32,
    /// The function the place starts in ("" at the top level).
    pub function: String,
    /// Every line from the first to the last of the place, other patches' lines included.
    pub lines: Vec<RegionLine>,
    pub edge_start: EdgeCheck,
    pub edge_end: EdgeCheck,
}

/// One option: a rung for one patch, or the picker (`pick`).
#[derive(Debug, Clone, PartialEq)]
pub struct RegionOffer {
    pub key: String,
    pub label: String,
    /// The class of loss it answers.
    pub kind: ConsumerKind,
    /// The patch the fix places (the asked patch, or the splitter of a Split).
    pub patch_id: String,
    pub anchors: Vec<Rung>,
    pub sites: Vec<SiteKey>,
    /// The option to list first.
    pub suggested: bool,
    /// Shown with it (the guess warning).
    pub note: Option<String>,
    /// The line as the game has it and as the other patch wrote it (Rewritten1to1).
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    /// For an edge option, whether the edge can be used.
    pub edge: Option<EdgeCheck>,
    /// Opens the picker on the other patch's lines; it has no rung.
    pub pick: bool,
    /// Runs the patch as separate edits (an ApplyAs fix, D46) instead of a rung: its lines
    /// are all there, split by the other patch's code (§4.8).
    pub apply_as: bool,
}

/// What `plan` found for one patch.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionPlan {
    pub consumer: Option<ConsumerInfo>,
    /// The patch replaces its lines (`at`): it gets no rungs of its own (D6).
    pub a_is_at: bool,
    /// "Move to another line" makes an adjusted copy (`at` and regex patches, D25). A regex
    /// still gets the region rungs (D52).
    pub move_by_copy: bool,
    /// The other patch (patch id, mod id) whose places `sites` lists.
    pub b: Option<(String, String)>,
    pub offers: Vec<RegionOffer>,
    pub sites: Vec<BSite>,
}

/// One place of a patch in the game's file, for a fix that moves only that one (D25).
#[derive(Debug, Clone, PartialEq)]
pub struct ASite {
    pub key: SiteKey,
    /// 1-based line of the place's first line, as the file shows it.
    pub line: u32,
    /// 1-based line the place's code hangs on (the last line for `after`, else the first),
    /// as a conflict's `consumer.site_line` names it.
    pub anchor_line: u32,
    pub text: String,
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The serde name of a consumer kind.
pub fn kind_name(k: ConsumerKind) -> &'static str {
    match k {
        ConsumerKind::Split => "split",
        ConsumerKind::Reemitted => "reemitted",
        ConsumerKind::Rewritten1to1 => "rewritten_1to1",
        ConsumerKind::ReplacedByBlock => "replaced_by_block",
        ConsumerKind::Deleted => "deleted",
    }
}

/// Who wrote line `n` (1-based) of `trace`.
pub fn line_source(trace: &FileTrace, n: usize) -> Option<LineSource> {
    let origin = trace.lines.get(n.checked_sub(1)?)?;
    let kind = match origin {
        LineOrigin::Vanilla(_) => return None,
        LineOrigin::Inserted { .. } => "inserted",
        LineOrigin::Kept { .. } => "kept",
        LineOrigin::Modified { .. } => "modified",
    };
    let edit = trace.edits.get(origin.edit()?)?;
    Some(LineSource {
        kind,
        patch_id: edit.patch_id.clone(),
        mod_id: edit.mod_id.clone(),
    })
}

/// The record of `patch_id` (and `mod_id`, when given) in `target`, with its index.
pub fn find_record<'r>(
    report: &'r SimulationReport,
    target: &str,
    patch_id: &str,
    mod_id: Option<&str>,
) -> Option<(usize, &'r PatchExecutionRecord)> {
    let file = report
        .files_touched
        .iter()
        .find(|f| f.target_file == target)?;
    file.records
        .iter()
        .enumerate()
        .find(|(_, r)| r.patch_id == patch_id && mod_id.is_none_or(|m| r.mod_id == m))
}

/// The places of the patch with record `b_record` in `trace`, top to bottom. `b_name` names
/// it in the edge reasons.
pub fn b_sites(trace: &FileTrace, b_record: usize, b_name: &str) -> Vec<BSite> {
    let text: Vec<&str> = trace.text.lines().collect();
    let raw = trace.raw_lines();
    // Site -> its lines (0-based), in file order.
    let mut by_site: Vec<(u32, Vec<usize>)> = Vec::new();
    for (i, o) in trace.lines.iter().enumerate() {
        let Some(e) = o.edit().and_then(|e| trace.edits.get(e)) else {
            continue;
        };
        if e.record != b_record {
            continue;
        }
        match by_site.iter_mut().find(|(s, _)| *s == e.site) {
            Some((_, v)) => v.push(i),
            None => by_site.push((e.site, vec![i])),
        }
    }
    by_site.sort_by_key(|(s, _)| *s);
    if by_site.is_empty() {
        return Vec::new();
    }
    let funcs = scan(&trace.text);
    by_site
        .into_iter()
        .filter_map(|(site, mine)| {
            let (&first, &last) = (mine.first()?, mine.last()?);
            let raw_of = |i: usize| {
                raw.get(i)
                    .or_else(|| text.get(i))
                    .copied()
                    .unwrap_or_default()
            };
            let own: Vec<&str> = mine.iter().map(|&i| raw_of(i)).collect();
            let mut counts: HashMap<&str, u32> = HashMap::new();
            for l in &own {
                *counts.entry(l.trim()).or_default() += 1;
            }
            let lines = (first..=last)
                .map(|i| {
                    let in_b = mine.contains(&i);
                    RegionLine {
                        line: to_u32(i + 1),
                        text: text.get(i).copied().unwrap_or_default().to_string(),
                        raw_text: raw_of(i).to_string(),
                        source: line_source(trace, i + 1),
                        in_b,
                        matches_in_region: in_b
                            .then(|| counts.get(raw_of(i).trim()).copied().unwrap_or(0)),
                    }
                })
                .collect();
            let (edge_start, edge_end) = edge_checks(&own, b_name);
            Some(BSite {
                site,
                first_line: to_u32(first + 1),
                last_line: to_u32(last + 1),
                function: funcs.function_name(first + 1).to_string(),
                lines,
                edge_start,
                edge_end,
            })
        })
        .collect()
}

/// `vanilla` prepared as the game prepares `target` for `table`'s profile (D1).
pub fn prepared_original(table: &PatchTable, target: &str, vanilla: &str) -> PreparedBuffer {
    prepare_buffer(target, vanilla, &launch_config_of(&table.mod_dir))
}

/// A pattern patch's places in the prepared game file, with their site keys (§3.7).
pub fn a_sites(a: &PatternPatch, buf: &PreparedBuffer) -> Vec<ASite> {
    let lines: Vec<&str> = buf.text.lines().collect();
    let wm: Vec<WildMatch> = a
        .pattern
        .lines()
        .map(|l| WildMatch::new(l.trim()))
        .collect();
    // Numbered as the engine numbers them: QuarterMaster's lines are never sites (D42).
    let removes = a.position == InsertPosition::At;
    // An exact copy's lines match only their own text (D59), as in the engine.
    let windows = find_windows(&lines, &wm, a.exact_lines(), |i, len| {
        match_filter(buf, i, len, removes)
    });
    sites_of(&windows, a.position, buf)
}

/// A pattern or regex patch's places in the prepared game file (a regex's are its matches,
/// D52); empty for other kinds.
pub fn patch_sites(a: &Patch, buf: &PreparedBuffer) -> Vec<ASite> {
    match a {
        Patch::Pattern(p) => a_sites(p, buf),
        Patch::Regex(r) => {
            let windows = r.site_windows(&buf.text, |l, removes| {
                match_filter(buf, l.start, l.len(), removes)
            });
            sites_of(&windows, r.position, buf)
        }
        _ => Vec::new(),
    }
}

fn sites_of(windows: &[Window], position: InsertPosition, buf: &PreparedBuffer) -> Vec<ASite> {
    let lines: Vec<&str> = buf.text.lines().collect();
    let is_builtin = |i: usize| buf.is_builtin(i) || buf.builtin_edited.contains(&i);
    let shown = |i: usize| (0..i).filter(|&k| !buf.is_builtin(k)).count() + 1;
    windows
        .iter()
        .enumerate()
        .map(|(k, w)| ASite {
            key: SiteKey {
                context: site_context(&lines, is_builtin, w.line, w.len),
                ordinal: Some(to_u32(k + 1)),
            },
            line: to_u32(shown(w.line)),
            anchor_line: to_u32(shown(match position {
                InsertPosition::After => w.line + w.len.saturating_sub(1),
                _ => w.line,
            })),
            text: lines
                .get(w.line)
                .map(|l| l.trim().to_string())
                .unwrap_or_default(),
        })
        .collect()
}

/// Display names: mod names from the table, patch names from the records.
struct Names<'a> {
    report: &'a SimulationReport,
    table: &'a PatchTable,
}

impl Names<'_> {
    fn mod_name(&self, id: &str) -> String {
        self.table
            .mods
            .iter()
            .find(|m| m.id.eq_ignore_ascii_case(id))
            .and_then(|m| m.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| id.to_string())
    }

    fn patch_name(&self, id: &str) -> String {
        self.report
            .files_touched
            .iter()
            .flat_map(|f| &f.records)
            .find(|r| r.patch_id == id)
            .map(|r| r.patch_name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.to_string())
    }
}

/// The loaded patch a record id names (with its mod, when given).
fn patch_of<'t>(table: &'t PatchTable, id: &str, mod_id: Option<&str>) -> Option<&'t Patch> {
    table
        .patches
        .iter()
        .map(|(p, _, _)| p)
        .find(|p| p.id() == Some(id) && mod_id.is_none_or(|m| p.mod_id() == Some(m)))
}

/// The region options for `patch_id` in `target` (§9.2), from the simulation's losses.
/// `b_hint` names the other patch when no loss names one (the conflict's second patch).
/// None when the patch is not loaded.
pub fn plan(
    report: &SimulationReport,
    table: &PatchTable,
    target: &str,
    patch_id: &str,
    b_hint: Option<&str>,
) -> Option<RegionPlan> {
    let a_mod = find_record(report, target, patch_id, None).map(|(_, r)| r.mod_id.clone());
    let a = patch_of(table, patch_id, a_mod.as_deref())?;
    // A regex takes rungs (D52), and "Move to another line" makes an adjusted copy of it
    // (D25), unless it reads its match: a copy would lose the match, so a rung moves it
    // (D59). An edge places a regex only when it inserts and reads no capture.
    let (a_is_at, move_by_copy, rungs_ok, edges_ok) = match a {
        Patch::Pattern(p) => {
            let at = p.position == InsertPosition::At;
            (at, at, !at, true)
        }
        Patch::Regex(r) => (false, !r.reads_capture(), true, r.takes_edge_rungs()),
        _ => (false, true, false, false),
    };
    let names = Names { report, table };
    let losses: Vec<&SiteLoss> = report
        .losses
        .iter()
        .filter(|l| l.target_file == target && l.patch_id == patch_id)
        .collect();
    let consumer = losses.iter().find_map(|l| l.consumer.clone());
    let b: Option<(String, String)> = match &consumer {
        Some(c) => Some((c.patch_id.clone(), c.mod_id.clone())),
        None => b_hint.and_then(|id| {
            find_record(report, target, id, None)
                .map(|(_, r)| (r.patch_id.clone(), r.mod_id.clone()))
        }),
    };
    let sites = match (&b, report.traces.get(target)) {
        (Some((id, m)), Some(trace)) => find_record(report, target, id, Some(m))
            .map(|(ix, _)| b_sites(trace, ix, &names.mod_name(m)))
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    // Lost places by who took them and how; one option set per group.
    let mut groups: Vec<(&ConsumerInfo, Option<u32>, Vec<&SiteLoss>)> = Vec::new();
    for l in &losses {
        let Some(c) = &l.consumer else { continue };
        match groups
            .iter_mut()
            .find(|(g, s, _)| g.patch_id == c.patch_id && g.kind == c.kind && *s == l.consumer_site)
        {
            Some((_, _, v)) => v.push(l),
            None => groups.push((c, l.consumer_site, vec![l])),
        }
    }

    let mut offers: Vec<RegionOffer> = Vec::new();
    // "Run it as separate edits" covers the whole patch: one offer, whatever the groups.
    let mut edits_offered = false;
    for (c, b_site, group) in groups {
        let b_name = names.mod_name(&c.mod_id);
        let one = group.len() == 1;
        let keys: Vec<SiteKey> = group
            .iter()
            .map(|l| SiteKey {
                context: l.context.clone(),
                ordinal: Some(l.site + 1),
            })
            .collect();
        let b_site_of_group = if one { b_site } else { None };
        let site_view = b_site.and_then(|s| sites.iter().find(|x| x.site == s));
        let key = |variant: &str| {
            format!(
                "region:{}:{}:{variant}:{}",
                kind_name(c.kind),
                c.patch_id,
                b_site.map_or_else(|| "-".to_string(), |s| s.to_string())
            )
        };
        let offer = |key: String, label: String, anchors: Vec<Rung>| RegionOffer {
            key,
            label,
            kind: c.kind,
            patch_id: patch_id.to_string(),
            anchors,
            sites: keys.clone(),
            suggested: false,
            note: None,
            old_text: None,
            new_text: None,
            edge: None,
            pick: false,
            apply_as: false,
        };
        match c.kind {
            ConsumerKind::Reemitted if rungs_ok => {
                let pattern = c.old_text.trim().to_string();
                let rung = Rung {
                    within: Some(c.patch_id.clone()),
                    pattern: Some(pattern.clone()),
                    exact: true,
                    nth: site_view.and_then(|s| nth_kept(s, &pattern)),
                    b_site: b_site_of_group,
                    ..Rung::default()
                };
                let mut o = offer(
                    key("within"),
                    format!("Keep it next to {b_name}'s copy of that line"),
                    vec![rung],
                );
                o.suggested = true;
                offers.push(o);
            }
            ConsumerKind::Rewritten1to1 if rungs_ok => {
                let Some(new) = c
                    .new_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                else {
                    continue;
                };
                let rung = Rung {
                    within: Some(c.patch_id.clone()),
                    pattern: Some(new.to_string()),
                    exact: true,
                    nth: site_view.and_then(|s| nth_of(s, new)),
                    b_site: b_site_of_group,
                    ..Rung::default()
                };
                let mut o = offer(
                    key("follow"),
                    format!("Follow {b_name}'s new version of that line"),
                    vec![rung],
                );
                o.old_text = Some(c.old_text.clone());
                o.new_text = Some(new.to_string());
                offers.push(o);
            }
            ConsumerKind::ReplacedByBlock | ConsumerKind::Deleted if rungs_ok => {
                let note = (c.kind == ConsumerKind::Deleted).then(|| {
                    format!("{b_name} removed this line; placing next to its change is a guess")
                });
                let edges = [
                    (
                        Edge::End,
                        "edge_end",
                        format!("Put it after {b_name}'s change"),
                    ),
                    (
                        Edge::Start,
                        "edge_start",
                        format!("Put it before {b_name}'s change"),
                    ),
                ];
                // A regex that reads its match or replaces it has nothing to put at an edge.
                for (edge, variant, label) in edges.into_iter().filter(|_| edges_ok) {
                    let (check, gate) = match site_view {
                        Some(s) => {
                            let boundary = match edge {
                                Edge::Start => s.lines.iter().find(|l| l.in_b),
                                Edge::End => s.lines.iter().rev().find(|l| l.in_b),
                            };
                            let gate = match edge {
                                Edge::Start => s.edge_start.clone(),
                                Edge::End => s.edge_end.clone(),
                            };
                            (boundary.map(|l| short_hash(&l.raw_text)), gate)
                        }
                        None => (
                            None,
                            EdgeCheck {
                                allowed: false,
                                reason: Some(format!("{b_name}'s change is no longer in {target}")),
                            },
                        ),
                    };
                    let rung = Rung {
                        within: Some(c.patch_id.clone()),
                        edge: Some(edge),
                        b_site: b_site_of_group,
                        check,
                        ..Rung::default()
                    };
                    let mut o = offer(key(variant), label, vec![rung]);
                    o.note = note.clone();
                    o.edge = Some(gate);
                    offers.push(o);
                }
                if c.kind == ConsumerKind::ReplacedByBlock {
                    let mut o = offer(
                        key("pick"),
                        format!("Pick a line inside {b_name}'s change..."),
                        Vec::new(),
                    );
                    o.pick = true;
                    offers.push(o);
                }
            }
            ConsumerKind::Split => {
                // The asked patch is the victim. As separate edits it skips the splitter's
                // lines and finds its place again (§4.8).
                if !edits_offered && group.iter().any(|l| l.apply_as) {
                    edits_offered = true;
                    let mut o = offer(
                        key("edits"),
                        format!("Run {} as separate edits", names.patch_name(patch_id)),
                        Vec::new(),
                    );
                    o.apply_as = true;
                    o.suggested = true;
                    let a_name = a_mod.as_deref().map(|m| names.mod_name(m)).unwrap_or_default();
                    o.note = edits_note(a, &a_name, &b_name);
                    offers.push(o);
                }
                // An `at` victim: the splitter goes on its copy of the line.
                for l in group.iter().filter(|_| a_is_at) {
                    let Some(h) = &l.rung else { continue };
                    let rung = Rung {
                        within: Some(h.within.clone()),
                        pattern: Some(h.pattern.clone()),
                        exact: true,
                        ..Rung::default()
                    };
                    let mut o = offer(
                        format!("{}:{}", key("splitter"), l.site),
                        format!(
                            "Keep {}'s {} next to {}'s copy of its line",
                            names.mod_name(&h.mod_id),
                            names.patch_name(&h.patch_id),
                            names.mod_name(&h.within_mod)
                        ),
                        vec![rung],
                    );
                    o.patch_id = h.patch_id.clone();
                    o.sites = vec![SiteKey {
                        context: h.context.clone(),
                        ordinal: None,
                    }];
                    o.suggested = true;
                    offers.push(o);
                }
            }
            _ => {}
        }
    }
    // Suggested options first, then as found.
    offers.sort_by_key(|o| !o.suggested);
    Some(RegionPlan {
        consumer,
        a_is_at,
        move_by_copy,
        b,
        offers,
        sites,
    })
}

/// The card text of "Run <A> as separate edits" (§9.7): where A's new lines go, and that
/// the other mod's lines between A's lines stay.
fn edits_note(a: &Patch, a_name: &str, b_name: &str) -> Option<String> {
    let Patch::Pattern(p) = a else {
        return None;
    };
    if p.position != InsertPosition::At {
        return Some(format!(
            "{a_name}'s lines find their place again around {b_name}'s lines, which stay"
        ));
    }
    let hunks = p.normalized().ok()?;
    let line = |k: usize| p.pattern.lines().nth(k).map(str::trim).unwrap_or_default().to_string();
    let inserts: Vec<(Option<usize>, usize)> = hunks
        .iter()
        .filter_map(|h| match h {
            Hunk::Insert { after_p, q } => Some((*after_p, q.len())),
            _ => None,
        })
        .collect();
    let only_inserts = edits::inserts_only(&hunks);
    match inserts.as_slice() {
        [(after, n)] if only_inserts => {
            let (lines, them) = match n {
                1 => ("1 new line goes".to_string(), "it"),
                n => (format!("{n} new lines go"), "them"),
            };
            let place = match after {
                Some(k) => format!("after \"{}\"", line(*k)),
                None => format!("before \"{}\"", line(0)),
            };
            Some(format!("{a_name}'s {lines} in {place}; {b_name}'s lines next to {them} stay"))
        }
        _ => Some(format!(
            "{a_name}'s lines the game already has stay as they are; {b_name}'s lines between them stay"
        )),
    }
}

/// `nth` for `pattern` among the other patch's lines at one place: None when it is unique,
/// else the 1-based place of the one line it wrote back unchanged.
fn nth_kept(site: &BSite, pattern: &str) -> Option<u32> {
    let hits: Vec<&RegionLine> = site
        .lines
        .iter()
        .filter(|l| l.in_b && l.raw_text.trim() == pattern)
        .collect();
    if hits.len() < 2 {
        return None;
    }
    let kept: Vec<usize> = hits
        .iter()
        .enumerate()
        .filter(|(_, l)| l.source.as_ref().is_some_and(|s| s.kind == "kept"))
        .map(|(k, _)| k)
        .collect();
    match kept.as_slice() {
        [one] => Some(to_u32(one + 1)),
        _ => None,
    }
}

/// `nth` for `pattern` among the other patch's lines at one place: None when it is unique,
/// else the first of them.
fn nth_of(site: &BSite, pattern: &str) -> Option<u32> {
    let n = site
        .lines
        .iter()
        .filter(|l| l.in_b && l.raw_text.trim() == pattern)
        .count();
    (n > 1).then_some(1)
}

#[cfg(test)]
#[path = "region_tests.rs"]
mod tests;
