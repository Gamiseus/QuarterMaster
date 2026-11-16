#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
//! Rungs: where a patch anchors on another patch's output (Phase C §3.6, §6.1).
//!
//! A pattern or regex patch A can carry a ladder of rungs. Each rung names a place on another
//! patch B's code (text inside B, or B's first or last line) or a text in the whole file. The
//! ladder is built once per table (`PatchTable::prepare`) and only read while patching, so
//! the DLL can share it. Rungs resolve per site of A: a site that no rung can place keeps
//! A's own match, and the record says why.

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wildmatch::WildMatch;

use crate::handshake::{AnchorReport, InvalidRung, SiteResolution};
use crate::lenient::Lenient;
use crate::lua_scan::block_balanced;
use crate::patch::fingerprint::{short_hash, site_context, PatchFingerprint};
use crate::patch::linemap::{LineMap, SiteRun};
use crate::patch::order::{matching_patches, patch_name};
use crate::patch::pattern::{find_split_windows, find_windows, skippable, PatternPatch, Window};
use crate::patch::plan::{ModGraph, ModMeta};
use crate::patch::regex::{uses_capture, RegexPatch};
use crate::patch::{ApplyCtx, InsertPosition, Patch, Priority, Target};

/// Which end of B's lines an edge rung anchors on.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Start,
    End,
}

/// One rung of a patch's anchor ladder.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Rung {
    /// B: "mod_id:patch_id" (no mod:*).
    #[serde(default)]
    pub within: Option<String>,
    /// Needs `within`, excludes `pattern`.
    #[serde(default)]
    pub edge: Option<Edge>,
    /// Inside B if `within`, else the whole current file (a Text rung).
    #[serde(default)]
    pub pattern: Option<String>,
    /// Trimmed equality, no wildcards.
    #[serde(default)]
    pub exact: bool,
    /// 1-based among the matches inside one B site; None means it must be unique.
    #[serde(default)]
    pub nth: Option<u32>,
    /// Before or after only; default A's position.
    #[serde(default)]
    pub position: Option<InsertPosition>,
    /// Which of B's places to use (B's site index, 0-based), for a move to another place
    /// and for an A with no place of its own (D9).
    #[serde(default)]
    pub b_site: Option<u32>,
    /// Edge rungs: short hash of B's boundary line.
    #[serde(default)]
    pub check: Option<String>,
    #[serde(default)]
    pub b_fp: Option<PatchFingerprint>,
}

/// Picks one of A's vanilla sites by its context hash. `ordinal` (1-based, among A's
/// matches in the unpatched file) picks among sites with the same context.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SiteKey {
    pub context: String,
    #[serde(default)]
    pub ordinal: Option<u32>,
}

/// Where a rung came from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnchorSource {
    Author,
    Profile { fix_id: String },
    Compat { mod_id: String },
}

impl AnchorSource {
    /// "author", "profile:<fix id>" or "compat:<mod id>".
    pub fn label(&self) -> String {
        match self {
            AnchorSource::Author => "author".to_string(),
            AnchorSource::Profile { fix_id } => format!("profile:{fix_id}"),
            AnchorSource::Compat { mod_id } => format!("compat:{mod_id}"),
        }
    }

    fn rank(&self) -> u8 {
        match self {
            AnchorSource::Profile { .. } => 0,
            AnchorSource::Compat { .. } => 1,
            AnchorSource::Author => 2,
        }
    }
}

/// The rungs of one patch, profile first, then compat, then author.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LadderSpec {
    pub rungs: Vec<(Rung, AnchorSource)>,
    pub sites: Vec<SiteKey>,
    /// The first profile fix among the rungs.
    pub fix_id: Option<String>,
}

/// A patch's ladder, built in `PatchTable::prepare` and never changed after.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ladder {
    pub spec: LadderSpec,
    /// Per rung: the indexes of B's patches that can run (an adjusted copy of B counts as
    /// B); `None` for a text rung.
    pub b_ix: Vec<Option<Vec<u32>>>,
    /// Per rung: its order edge was dropped to break a cycle (set in `prepare`).
    pub suspended: Vec<bool>,
    /// Per rung: B's content differs from the rung's `b_fp`.
    pub b_drifted: Vec<bool>,
    /// Per rung: why it is never used (an `InvalidRung` reason).
    pub invalid: Vec<Option<String>>,
    /// Per rung: B as the user reads it ("<mod> › <patch name>").
    pub b_label: Vec<String>,
    /// Per rung: why B is not there in any file (turned off, not loaded).
    pub b_absent: Vec<Option<String>>,
    /// Per rung: the reason when it is suspended.
    pub suspended_reason: Vec<Option<String>>,
    /// Per rung: the files B's patches change.
    pub b_targets: Vec<Vec<Target>>,
}

impl Ladder {
    pub fn len(&self) -> usize {
        self.spec.rungs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spec.rungs.is_empty()
    }

    fn rung(&self, r: usize) -> Option<&Rung> {
        self.spec.rungs.get(r).map(|(rung, _)| rung)
    }

    pub(crate) fn source(&self, r: usize) -> String {
        self.spec
            .rungs
            .get(r)
            .map(|(_, s)| s.label())
            .unwrap_or_default()
    }

    /// The reason a rung is suspended, if it is.
    pub fn suspension(&self) -> Option<&str> {
        self.suspended
            .iter()
            .zip(&self.suspended_reason)
            .find_map(|(&s, why)| s.then_some(why.as_deref()).flatten())
    }
}

/// What kind of patch carries a ladder (D52).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostKind {
    /// `inserts_only`: an `at` that runs as edits which only keep lines and add new ones
    /// (§4.7); such an `at` takes rungs.
    Pattern { inserts_only: bool },
    /// `needs_capture`: the payload or `line_prepend` reads a capture group.
    Regex { needs_capture: bool },
}

/// What a ladder reads of the patch that carries it: its author rungs and sites, its
/// position and its kind (D52).
pub(crate) trait RungHost {
    fn author_anchors(&self) -> &[Lenient<Rung>];
    fn author_sites(&self) -> &[Lenient<SiteKey>];
    fn position(&self) -> InsertPosition;
    fn kind(&self) -> HostKind;
}

impl RungHost for PatternPatch {
    fn author_anchors(&self) -> &[Lenient<Rung>] {
        &self.anchors
    }

    fn author_sites(&self) -> &[Lenient<SiteKey>] {
        &self.sites
    }

    fn position(&self) -> InsertPosition {
        self.position
    }

    fn kind(&self) -> HostKind {
        HostKind::Pattern {
            inserts_only: self.position == InsertPosition::At && self.inserts_only(),
        }
    }
}

impl RungHost for RegexPatch {
    fn author_anchors(&self) -> &[Lenient<Rung>] {
        &self.anchors
    }

    fn author_sites(&self) -> &[Lenient<SiteKey>] {
        &self.sites
    }

    fn position(&self) -> InsertPosition {
        self.position
    }

    fn kind(&self) -> HostKind {
        HostKind::Regex {
            needs_capture: uses_capture(&self.payload) || uses_capture(&self.line_prepend),
        }
    }
}

/// A mod as the user reads it: its mod.json name, else its folder, else its id.
pub(crate) fn mod_label(mods: &[ModMeta], id: &str) -> String {
    mods.iter()
        .find(|m| m.id.eq_ignore_ascii_case(id))
        .map(|m| {
            m.name
                .clone()
                .unwrap_or_else(|| if m.folder.is_empty() { m.id.clone() } else { m.folder.clone() })
        })
        .unwrap_or_else(|| id.to_string())
}

/// A patch as the user reads it: "<mod> › <patch name>".
fn patch_label(mods: &[ModMeta], patch: &Patch, ix: usize) -> String {
    let m = patch.mod_id().unwrap_or_default();
    let name = patch
        .name()
        .map(str::to_string)
        .unwrap_or_else(|| patch_name(patch, ix));
    // An adjusted copy belongs to the profile, whichever mod it runs as (D58).
    if m.is_empty() || patch.extra().adjusted_copy {
        return name;
    }
    format!("{} › {name}", mod_label(mods, m))
}

/// The mod a `within` names: the first hit's mod (an adjusted copy's does not count, D58),
/// else the part before the first ':'.
fn within_mod(within: &str, hits: &[usize], patches: &[(Patch, Priority, PathBuf)]) -> String {
    hits.iter()
        .filter_map(|&i| patches.get(i))
        .filter(|(p, _, _)| !p.extra().adjusted_copy)
        .find_map(|(p, _, _)| p.mod_id())
        .map(str::to_string)
        .unwrap_or_else(|| within.split(':').next().unwrap_or_default().to_string())
        .to_lowercase()
}

/// Builds the ladder of every pattern or regex patch that has rungs, and lists the rungs that are
/// never used. Fingerprints only the B of a rung that carries `b_fp` (D32).
pub(crate) fn build_ladders(
    patches: &[(Patch, Priority, PathBuf)],
    mods: &[ModMeta],
    graph: &ModGraph,
) -> (Vec<Option<Ladder>>, Vec<InvalidRung>) {
    let mut invalid_out = Vec::new();
    let ladders = patches
        .iter()
        .enumerate()
        .map(|(a, (patch, _, _))| build_one(a, patch, patches, mods, graph, &mut invalid_out))
        .collect();
    (ladders, invalid_out)
}

fn build_one(
    a: usize,
    patch: &Patch,
    patches: &[(Patch, Priority, PathBuf)],
    mods: &[ModMeta],
    graph: &ModGraph,
    invalid_out: &mut Vec<InvalidRung>,
) -> Option<Ladder> {
    let extra = patch.extra();
    let a_name = patch_name(patch, a);
    let a_mod = patch.mod_id().unwrap_or_default().to_lowercase();
    let host: &dyn RungHost = match patch {
        Patch::Pattern(p) => p,
        Patch::Regex(r) => r,
        _ => {
            // Rungs only on pattern and regex patches (D6, D52).
            for (r, (_, source)) in extra.rungs.iter().enumerate() {
                invalid_out.push(InvalidRung {
                    patch_id: a_name.clone(),
                    rung: to_u32(r),
                    reason: "not_pattern".to_string(),
                    detail: format!(
                        "{a_name} is not a pattern or regex patch, so it cannot anchor on other code"
                    ),
                    source: source.label(),
                });
            }
            return None;
        }
    };
    let kind = host.kind();
    let a_pos = host.position();
    let pattern_at = matches!(kind, HostKind::Pattern { .. }) && a_pos == InsertPosition::At;

    let mut rungs: Vec<(Rung, AnchorSource)> = extra.rungs.clone();
    rungs.sort_by_key(|(_, s)| s.rank());
    // A malformed author entry keeps its slot (an empty rung), so rung indexes match the
    // author's list.
    let mut malformed: Vec<Option<String>> = vec![None; rungs.len()];
    if !extra.replace_author_anchors {
        for entry in host.author_anchors() {
            match entry {
                Lenient::Ok(r) => {
                    rungs.push((r.clone(), AnchorSource::Author));
                    malformed.push(None);
                }
                Lenient::Invalid { error, .. } => {
                    rungs.push((Rung::default(), AnchorSource::Author));
                    malformed.push(Some(error.clone()));
                }
            }
        }
    }
    if rungs.is_empty() {
        return None;
    }
    let sites = if extra.sites.is_empty() {
        host.author_sites().iter().filter_map(Lenient::ok).cloned().collect()
    } else {
        extra.sites.clone()
    };
    let fix_id = rungs.iter().find_map(|(_, s)| match s {
        AnchorSource::Profile { fix_id } => Some(fix_id.clone()),
        _ => None,
    });

    let n = rungs.len();
    let mut ladder = Ladder {
        b_ix: Vec::with_capacity(n),
        suspended: vec![false; n],
        b_drifted: vec![false; n],
        invalid: Vec::with_capacity(n),
        b_label: Vec::with_capacity(n),
        b_absent: Vec::with_capacity(n),
        suspended_reason: vec![None; n],
        b_targets: Vec::with_capacity(n),
        spec: LadderSpec {
            rungs: Vec::new(),
            sites,
            fix_id,
        },
    };
    let requires = mods
        .iter()
        .find(|m| m.id.eq_ignore_ascii_case(&a_mod))
        .map(|m| m.requires.clone())
        .unwrap_or_default();
    let required = graph.requires_resolved(&a_mod);

    for (r, (rung, source)) in rungs.iter().enumerate() {
        let mut why: Option<(&str, String)> = None;
        let mut b_ix = None;
        let mut label = rung.within.clone().unwrap_or_else(|| "the file".to_string());
        let mut absent = None;
        let mut targets = Vec::new();
        if let Some(error) = malformed.get(r).cloned().flatten() {
            why = Some((
                "malformed",
                format!("A rung of {a_name} could not be read and is skipped: {error}"),
            ));
        } else if kind == (HostKind::Pattern { inserts_only: false }) && a_pos == InsertPosition::At {
            why = Some(("at_patch", format!("{a_name} replaces lines, so it cannot anchor on other code")));
        } else if pattern_at && rung.edge.is_some() {
            // An `at` in edits mode is placed by its own lines; a rung only says where to
            // find them (§4.7).
            why = Some(("at_patch", format!("{a_name} keeps lines of its own, so it cannot go at the edge of other code")));
        } else if pattern_at && rung.position.is_some_and(|p| p != InsertPosition::At) {
            why = Some(("at_patch", format!("{a_name} places its code by its own lines; a rung cannot change where")));
        } else if rung.edge.is_some() && rung.pattern.is_some() {
            why = Some(("edge_with_pattern", "A rung cannot set both edge and pattern".to_string()));
        } else if rung.edge.is_some() && rung.within.is_none() {
            why = Some(("edge_without_within", "An edge rung needs within".to_string()));
        } else if rung.within.is_none() && rung.pattern.is_none() {
            why = Some(("no_target", "A rung needs within or pattern".to_string()));
        } else if rung.position == Some(InsertPosition::At) {
            why = Some(("position_at", "A rung can only place code before or after".to_string()));
        } else if let HostKind::Regex { needs_capture } = kind {
            // An edge gives a place with no match to read or replace (D52).
            if rung.edge.is_some() && a_pos == InsertPosition::At {
                why = Some((
                    "needs_capture",
                    format!("{a_name} replaces the text it matches, so it cannot go at the edge of other code"),
                ));
            } else if rung.edge.is_some() && needs_capture {
                why = Some((
                    "needs_capture",
                    format!("{a_name} uses the text it matches, so it cannot go at the edge of other code"),
                ));
            } else if rung.edge.is_none() && rung.position.is_some_and(|p| p != a_pos) {
                why = Some((
                    "regex_position",
                    format!("{a_name} places its code by its own position; a rung cannot change it"),
                ));
            }
        }
        if let (None, Some(within)) = (&why, rung.within.as_deref()) {
            let within = within.trim();
            if within.ends_with(":*") {
                why = Some(("mod_wildcard", format!("within must name one patch, not {within}")));
            } else {
                let mut hits: Vec<usize> = matching_patches(within, patches)
                    .into_iter()
                    .filter(|&i| i != a)
                    .collect();
                let aliases = patches.iter().enumerate().filter(|(i, (p, _, _))| {
                    *i != a && p.extra().replaces.as_deref() == Some(within)
                });
                for (i, _) in aliases {
                    if !hits.contains(&i) {
                        hits.push(i);
                    }
                }
                // An author rung honours only the patches of its own mod and of mods it
                // requires (D33). Patch ids are not namespaced, so one `within` can match
                // patches of several mods: the hits of other mods are dropped, and the rung
                // is skipped only when no allowed hit is left.
                let allowed = |m: &str| {
                    *source != AnchorSource::Author
                        || m.is_empty()
                        || m == a_mod
                        || required.contains(m)
                        || requires.iter().any(|r| r == m)
                };
                let hit_mod = |i: usize| {
                    patches
                        .get(i)
                        .and_then(|(p, _, _)| p.mod_id())
                        .unwrap_or_default()
                        .to_lowercase()
                };
                // An adjusted copy is the profile's, whichever mod it runs as (D58).
                let is_copy = |i: usize| {
                    patches
                        .get(i)
                        .is_some_and(|(p, _, _)| p.extra().adjusted_copy)
                };
                let mod_hits: Vec<String> = hits
                    .iter()
                    .filter(|&&i| !is_copy(i))
                    .map(|&i| hit_mod(i))
                    .collect();
                if mod_hits.iter().any(|m| allowed(m)) && mod_hits.iter().any(|m| !allowed(m)) {
                    hits.retain(|&i| is_copy(i) || allowed(&hit_mod(i)));
                }
                if let Some(&first) = hits.first() {
                    if let Some((p, _, _)) = patches.get(first) {
                        label = patch_label(mods, p, first);
                    }
                }
                let b_mod = within_mod(within, &hits, patches);
                let modules = hits
                    .iter()
                    .any(|&i| matches!(patches.get(i), Some((Patch::Module(_), _, _))));
                let gated = !allowed(&b_mod);
                if modules {
                    why = Some(("module_patch", format!("{label} is a module patch")));
                } else if gated {
                    let (am, bm) = (mod_label(mods, &a_mod), mod_label(mods, &b_mod));
                    why = Some((
                        "not_required",
                        format!("{am} places code inside {bm}'s change but does not require {bm}; QuarterMaster uses its own line"),
                    ));
                } else {
                    let on: Vec<u32> = hits
                        .iter()
                        .copied()
                        .filter(|&i| patches.get(i).is_some_and(|(p, _, _)| !p.is_disabled()))
                        .map(to_u32)
                        .collect();
                    if hits.is_empty() {
                        absent = Some(format!("{within} is not loaded"));
                    } else if on.is_empty() {
                        absent = Some(format!("{label} is turned off"));
                    }
                    targets = on
                        .iter()
                        .filter_map(|&b| match patches.get(b as usize) {
                            Some((Patch::Pattern(x), _, _)) => Some(x.target.clone()),
                            Some((Patch::Regex(x), _, _)) => Some(x.target.clone()),
                            Some((Patch::Copy(x), _, _)) => Some(x.target.clone()),
                            _ => None,
                        })
                        .collect();
                    if let (Some(fp), Some(&b)) = (rung.b_fp.as_ref(), on.first()) {
                        if let Some(d) = ladder.b_drifted.get_mut(r) {
                            *d = drifted(patches, mods, b as usize, fp);
                        }
                    }
                    b_ix = Some(on);
                }
            }
        }
        if let Some((code, detail)) = &why {
            invalid_out.push(InvalidRung {
                patch_id: a_name.clone(),
                rung: to_u32(r),
                reason: code.to_string(),
                detail: detail.clone(),
                source: source.label(),
            });
        }
        ladder.invalid.push(why.map(|(c, _)| c.to_string()));
        ladder.b_ix.push(b_ix);
        ladder.b_label.push(label);
        ladder.b_absent.push(absent);
        ladder.b_targets.push(targets);
    }
    if malformed.iter().all(Option::is_some) {
        return None;
    }
    ladder.spec.rungs = rungs;
    Some(ladder)
}

/// Whether patch `b`'s content differs from `fp` (anchor or payload).
fn drifted(
    patches: &[(Patch, Priority, PathBuf)],
    mods: &[ModMeta],
    b: usize,
    fp: &PatchFingerprint,
) -> bool {
    let Some((patch, _, _)) = patches.get(b) else {
        return false;
    };
    let base = patch
        .mod_id()
        .and_then(|m| mods.iter().find(|x| x.id.eq_ignore_ascii_case(m)))
        .map(|m| m.base.as_path())
        .unwrap_or(Path::new(""));
    !patch.fingerprint(base, &fp.source, fp.ordinal).same_content(fp)
}

/// The required order edges of the ladders, as (B, A, rung): B runs before A. A rung adds
/// none when it is invalid, when its B is off, or when B drifted from the rung's `b_fp`
/// (§4.5).
pub(crate) fn required_edges(ladders: &[Option<Ladder>]) -> Vec<(usize, usize, u32)> {
    let mut out = Vec::new();
    for (a, ladder) in ladders.iter().enumerate() {
        let Some(l) = ladder else { continue };
        for r in 0..l.len() {
            let ok = l.invalid.get(r).is_some_and(Option::is_none)
                && !l.b_drifted.get(r).copied().unwrap_or(true);
            if !ok {
                continue;
            }
            for &b in l.b_ix.get(r).and_then(Option::as_ref).into_iter().flatten() {
                out.push((b as usize, a, to_u32(r)));
            }
        }
    }
    out
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The buffer before any patch; its lines are found on first use.
pub(crate) struct OriginalLines<'a> {
    text: &'a str,
    /// Byte range of each line, as `str::lines` splits them.
    ranges: std::cell::OnceCell<Vec<(usize, usize)>>,
}

impl<'a> OriginalLines<'a> {
    pub fn new(text: &'a str) -> Self {
        OriginalLines {
            text,
            ranges: std::cell::OnceCell::new(),
        }
    }

    /// The whole buffer.
    pub fn text(&self) -> &'a str {
        self.text
    }

    pub fn get(&self) -> Vec<&'a str> {
        let text = self.text;
        let ranges = self.ranges.get_or_init(|| {
            let base = text.as_ptr() as usize;
            text.lines()
                .map(|l| {
                    let start = (l.as_ptr() as usize).saturating_sub(base);
                    (start, start + l.len())
                })
                .collect()
        });
        ranges
            .iter()
            .map(|&(a, b)| text.get(a..b).unwrap_or_default())
            .collect()
    }
}

/// One place a patch puts its payload: a window of the text before the patch and where to
/// put the code relative to it. A regex match carries its capture in `found`; a regex place
/// without one is a literal insert at an edge (D52).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Place {
    pub line: usize,
    pub len: usize,
    pub indent: String,
    pub pos: InsertPosition,
    pub found: Option<Found>,
    /// The matched lines (`line..line + len` unless a split window skipped some, D48).
    pub lines: Vec<usize>,
}

/// A regex match: its index in the patch's match list and its bytes (group 0) in the text
/// before the patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub ix: usize,
    pub start: usize,
    pub end: usize,
}

impl Place {
    pub(crate) fn from_window(w: Window, pos: InsertPosition) -> Self {
        Place {
            line: w.line,
            len: w.len,
            indent: w.indent,
            pos,
            found: None,
            lines: w.lines,
        }
    }

    /// The line the code goes in front of.
    fn insert_line(&self) -> usize {
        match self.pos {
            InsertPosition::After => self.line + self.len,
            _ => self.line,
        }
    }

    /// Two places put code at the same spot: the same match, or the same line and side.
    fn same_spot(&self, other: &Place) -> bool {
        match (self.found, other.found) {
            (Some(a), Some(b)) => (a.start, a.end) == (b.start, b.end),
            _ => self.pos == other.pos && self.insert_line() == other.insert_line(),
        }
    }

    /// `other`, which sorts after this place, overlaps it: two matches share bytes, or the
    /// lines overlap.
    fn overlaps(&self, other: &Place) -> bool {
        match (self.found, other.found) {
            (Some(a), Some(b)) => b.start < a.end,
            _ => other.line < self.line + self.len.max(1),
        }
    }
}

/// What `resolve` reads of A (D52): a pattern's windows or a regex's matches.
pub(crate) trait SiteSource {
    /// A's name in messages.
    fn label(&self) -> String;
    fn position(&self) -> InsertPosition;
    /// A's places in the buffer before any patch, numbered as its own matches are: never on
    /// QuarterMaster's lines (D42).
    fn vanilla(&self, original: &OriginalLines, lines: &LineMap) -> Vec<Window>;
    /// A regex A's matches in the current text inside the line ranges, in text order. None
    /// for a pattern A.
    fn search(&self, _ranges: &[Range<usize>]) -> Option<Vec<Place>> {
        None
    }
    /// A is placed by its own lines: an `at` in edits mode (§4.7). A rung then picks one of
    /// A's own matches instead of a line.
    fn by_own_lines(&self) -> bool {
        false
    }
    /// A's scoped windows may skip other patches' new lines (D48).
    fn split(&self) -> bool {
        false
    }
}

impl SiteSource for PatternPatch {
    fn label(&self) -> String {
        self.name.clone().or_else(|| self.id.clone()).unwrap_or_default()
    }

    fn position(&self) -> InsertPosition {
        self.position
    }

    fn vanilla(&self, original: &OriginalLines, lines: &LineMap) -> Vec<Window> {
        let wm: Vec<WildMatch> = self.pattern.lines().map(|x| WildMatch::new(x.trim())).collect();
        let removes = self.position == InsertPosition::At;
        find_windows(&original.get(), &wm, self.exact_lines(), |i, len| {
            !lines.original_blocks_match(i..i + len, removes)
        })
    }

    fn by_own_lines(&self) -> bool {
        self.position == InsertPosition::At
    }

    fn split(&self) -> bool {
        self.split_windows()
    }
}

/// The contiguous ranges of the sorted lines `lines`.
fn ranges_of(lines: &[usize]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for &l in lines {
        match out.last_mut() {
            Some(r) if r.end == l => r.end = l + 1,
            _ => out.push(l..l + 1),
        }
    }
    out
}

/// `resolve`'s result: the places to apply, in order, and the report.
pub(crate) struct Resolved {
    pub places: Vec<Place>,
    pub report: AnchorReport,
    pub warnings: Vec<String>,
}

/// A rung that found its place.
struct Hit {
    place: Place,
    followed: bool,
    /// Every line of B in this file (for an A with no place of its own).
    b_lines: Vec<usize>,
    /// The lines of the B run that covers A's vanilla site (D9), or none: A's own matches
    /// there stand for that site.
    covered: Vec<usize>,
}

fn indent_of(line: &str) -> String {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// A rung's matcher: wildcards per trimmed line, or trimmed equality.
fn matcher(pattern: &str) -> (Vec<WildMatch>, Vec<String>) {
    let lines: Vec<String> = pattern.lines().map(|l| l.trim().to_string()).collect();
    let wm = lines.iter().map(|l| WildMatch::new(l)).collect();
    (wm, lines)
}

/// Picks the `nth` (1-based) window or match, or the only one.
fn pick<T>(found: Vec<T>, nth: Option<u32>, what: &str) -> Result<T, String> {
    let k = found.len();
    if k == 0 {
        return Err(format!("the line is no longer in {what}"));
    }
    match nth {
        Some(n) => found
            .into_iter()
            .nth((n as usize).saturating_sub(1))
            .ok_or_else(|| format!("{what} has {k} matching lines, fewer than {n}")),
        None if k == 1 => found
            .into_iter()
            .next()
            .ok_or_else(|| format!("the line is no longer in {what}")),
        None => Err(format!("the line matches {k} lines in {what}; pick one")),
    }
}

/// Whether B's run `run` covers A's vanilla site with resolved uids `uids` (D9): a line of
/// the run stands for one of them (a kept or touched line, or a replaced one through its
/// alias), or a block of the run, or one of its ancestors, hangs on one of them. Implicit
/// anchors never count (D51): they order code, they are not B touching the line. In Legacy
/// (`legacy`) neither do the stacking anchors of copy and regex blocks (D53).
fn covers(lines: &LineMap, run: &SiteRun, uids: &[u32], legacy: bool) -> bool {
    run.lines
        .iter()
        .any(|&l| lines.get(l).is_some_and(|t| uids.contains(&t.uid)))
        || run.blocks.iter().any(|&b| {
            lines
                .coverage_anchors_of(b, legacy)
                .any(|(u, _)| uids.contains(&u))
        })
}

struct Resolver<'a, 'c> {
    ladder: &'a Ladder,
    src: &'a dyn SiteSource,
    ctx: &'a ApplyCtx<'c>,
    current: &'a [&'a str],
    target: &'a str,
    /// A has exactly one selected site, so a rung may move it to another of B's places.
    one_site: bool,
    /// A's own matches, when A is placed by its own lines (§4.7).
    own_at: Option<Vec<Place>>,
}

impl Resolver<'_, '_> {
    /// The place at window `w` that a rung found: the window itself for a pattern A, the one
    /// match of a regex A inside it (D52).
    fn place_at(&self, w: Window, pos: InsertPosition, what: &str) -> Result<Place, String> {
        if let Some(own) = &self.own_at {
            // The one match of A whose lines hold the rung's lines.
            let found: Vec<&Place> = own
                .iter()
                .filter(|p| w.lines.iter().all(|l| p.lines.contains(l)))
                .collect();
            return match found.as_slice() {
                [one] => Ok((*one).clone()),
                [] => Err(format!("its lines are not at that line in {what}")),
                more => Err(format!(
                    "its lines are at that line {} times in {what}; it needs one",
                    more.len()
                )),
            };
        }
        let range = w.line..w.line + w.len;
        match self.src.search(std::slice::from_ref(&range)) {
            None => Ok(Place::from_window(w, pos)),
            Some(found) => match found.len() {
                1 => found
                    .into_iter()
                    .next()
                    .ok_or_else(|| format!("the regex finds nothing on that line in {what}")),
                0 => Err(format!("the regex finds nothing on that line in {what}")),
                k => Err(format!(
                    "the regex matches {k} times on that line in {what}; it needs one"
                )),
            },
        }
    }

    /// Places rung `r` for vanilla site `s` (resolved uids), or says why it cannot.
    fn try_rung(&self, r: usize, s: Option<&[u32]>) -> Result<Hit, String> {
        let rung = self.ladder.rung(r).ok_or_else(|| "no rung".to_string())?;
        let a_pos = self.src.position();
        let Some(within) = rung.within.as_deref() else {
            // A text rung: the whole file.
            let pattern = rung.pattern.as_deref().unwrap_or_default();
            let (wm, exact) = matcher(pattern);
            let pos = rung.position.unwrap_or(a_pos);
            // QuarterMaster's lines are not places (D42).
            let removes = pos == InsertPosition::At;
            let found = find_windows(
                self.current,
                &wm,
                rung.exact.then_some(exact.as_slice()),
                |i, len| !self.ctx.lines.blocks_match(i..i + len, removes),
            );
            let w = pick(found, rung.nth, self.target)?;
            return Ok(Hit {
                place: self.place_at(w, pos, self.target)?,
                followed: false,
                b_lines: Vec::new(),
                covered: Vec::new(),
            });
        };
        let label = self.ladder.b_label.get(r).cloned().unwrap_or_else(|| within.to_string());
        if let Some(why) = self.ladder.b_absent.get(r).and_then(Option::as_ref) {
            return Err(why.clone());
        }
        let bs: Vec<usize> = self
            .ladder
            .b_ix
            .get(r)
            .and_then(Option::as_ref)
            .into_iter()
            .flatten()
            .filter(|&&b| b != self.ctx.me)
            .map(|&b| b as usize)
            .collect();
        let targets = self.ladder.b_targets.get(r).map(Vec::as_slice).unwrap_or(&[]);
        if !targets.is_empty() && !targets.iter().any(|t| t.can_apply(self.target)) {
            return Err(format!(
                "{label} changes another file; this rung never resolves in {}",
                self.target
            ));
        }
        let drift = self.ladder.b_drifted.get(r).copied().unwrap_or(false);
        if drift && rung.edge.is_some() {
            return Err(format!("{label} changed since this fix was made"));
        }
        let runs: Vec<SiteRun> = bs
            .iter()
            .filter(|&&b| self.ctx.committed.get(b).is_some_and(|c| !c.is_empty()))
            .flat_map(|&b| self.ctx.lines.site_runs(to_u32(b), self.ctx.line_rules))
            .collect();
        if runs.is_empty() {
            return Err(format!("{label} made no change in {}", self.target));
        }
        let b_lines: Vec<usize> = runs.iter().flat_map(|r| r.lines.iter().copied()).collect();
        let by_site = |n: u32| {
            runs.iter()
                .find(|run| run.site == n)
                .ok_or_else(|| format!("{label} has no place {}", n + 1))
        };
        let legacy = self
            .ctx
            .stack
            .as_ref()
            .is_none_or(|st| st.mode == crate::handshake::StackMode::Legacy);
        let covering = s.and_then(|uids| {
            runs.iter()
                .find(|run| covers(self.ctx.lines, run, uids, legacy))
        });
        let covered = covering.map(|run| run.lines.clone()).unwrap_or_default();
        let run = match s {
            Some(_) => match covering {
                Some(run) => run,
                None => match rung.b_site {
                    Some(n) if self.one_site => by_site(n)?,
                    _ => {
                        return Err(format!(
                            "{label} does not touch this line; pick which of its places to use"
                        ))
                    }
                },
            },
            None => match (rung.b_site, runs.as_slice()) {
                (Some(n), _) => by_site(n)?,
                (None, [one]) => one,
                (None, _) => return Err(format!("{label} has several places; pick a site")),
            },
        };
        let line_text = |l: usize| self.current.get(l).copied().unwrap_or_default();
        if let Some(edge) = rung.edge {
            let (line, pos) = match edge {
                Edge::Start => (run.first, InsertPosition::Before),
                Edge::End => (run.last, InsertPosition::After),
            };
            if let Some(want) = rung.check.as_deref() {
                if short_hash(line_text(line)) != want {
                    return Err(format!("{label}'s edge line changed"));
                }
            }
            let text: Vec<&str> = run.lines.iter().map(|&l| line_text(l)).collect();
            if !block_balanced(&text.join("\n")) {
                return Err(format!(
                    "{label} leaves a block open, so nothing is placed at its edge"
                ));
            }
            return Ok(Hit {
                place: Place {
                    line,
                    len: 1,
                    indent: indent_of(line_text(line)),
                    pos,
                    found: None,
                    lines: vec![line],
                },
                followed: false,
                b_lines,
                covered,
            });
        }
        if let (None, Some(own)) = (rung.pattern.as_ref(), &self.own_at) {
            // An `at` A with no line to find: its own match whose lines are all B's (D8).
            let inside: Vec<Place> = own
                .iter()
                .filter(|p| p.lines.iter().all(|l| run.lines.contains(l)))
                .cloned()
                .collect();
            let what = format!("{label}'s change");
            return Ok(Hit {
                place: pick(inside, rung.nth, &what)?,
                followed: drift,
                b_lines,
                covered,
            });
        }
        if rung.pattern.is_none() {
            // A regex A with no line to find: its matches inside B's lines at this place,
            // each contiguous range searched on its own (D52).
            if let Some(found) = self.src.search(&ranges_of(&run.lines)) {
                let what = format!("{label}'s change");
                let place = match found.len() {
                    0 => Err(format!("the regex finds nothing in {what}")),
                    _ => pick(found, rung.nth, &what),
                }?;
                return Ok(Hit {
                    place,
                    followed: drift,
                    b_lines,
                    covered,
                });
            }
        }
        let pattern = rung.pattern.as_deref().unwrap_or_default();
        let (wm, exact) = matcher(pattern);
        let allowed: BTreeSet<usize> = run.lines.iter().copied().collect();
        let pos = rung.position.unwrap_or(a_pos);
        let removes = pos == InsertPosition::At;
        // In edits mode the window may skip other patches' new lines (D48); the lines it
        // matches are B's.
        let split = self.src.split();
        let found = find_split_windows(
            self.current,
            &wm,
            rung.exact.then_some(exact.as_slice()),
            |l| split && skippable(self.ctx.lines, self.ctx.me, l),
            |w| {
                w.lines.iter().all(|l| allowed.contains(l))
                    && !self.ctx.lines.blocks_match(w.line..w.line + w.len, removes)
            },
        );
        let w = pick(found, rung.nth, &label)?;
        Ok(Hit {
            place: self.place_at(w, pos, &label)?,
            followed: drift,
            b_lines,
            covered,
        })
    }
}

/// Resolves A's ladder in one buffer (§6.1). `own` are A's matches in the current text
/// (today's places, in text order). Sites a rung places are claimed from `own`; the others
/// keep their own match. Never panics.
pub(crate) fn resolve(
    ladder: &Ladder,
    a: &dyn SiteSource,
    own: Vec<Place>,
    current: &[&str],
    ctx: &ApplyCtx,
    target: &str,
) -> Resolved {
    let lines = ctx.lines;
    let a_name = a.label();
    let original: Vec<&str> = ctx.original.map(|o| o.get()).unwrap_or_default();
    let original = original.as_slice();
    let vanilla = ctx
        .original
        .map(|o| a.vanilla(o, lines))
        .unwrap_or_default();
    let mut sites_out: Vec<SiteResolution> = Vec::new();
    let mut warnings = Vec::new();

    // The sites the rungs are for.
    let selected: Vec<usize> = if ladder.spec.sites.is_empty() {
        (0..vanilla.len()).collect()
    } else {
        let contexts: Vec<String> = vanilla
            .iter()
            .map(|w| site_context(original, |i| lines.uid_is_builtin(to_u32(i)), w.line, w.len))
            .collect();
        let mut out = BTreeSet::new();
        for (k, key) in ladder.spec.sites.iter().enumerate() {
            let cands: Vec<usize> = (0..vanilla.len())
                .filter(|&i| contexts.get(i) == Some(&key.context))
                .collect();
            let picked: Vec<usize> = match (cands.as_slice(), key.ordinal) {
                ([_, _, ..], Some(n)) => cands
                    .iter()
                    .copied()
                    .filter(|&i| i + 1 == n as usize)
                    .collect(),
                _ => cands,
            };
            if picked.is_empty() {
                sites_out.push(SiteResolution {
                    site: to_u32(k),
                    lost: true,
                    fallback: Some(format!("this place is no longer in {target}")),
                    ..SiteResolution::default()
                });
            }
            out.extend(picked);
        }
        out.into_iter().collect()
    };
    // A text rung moves one site; with several, the ladder must select exactly one.
    let single_site = vanilla.len() <= 1 || (!ladder.spec.sites.is_empty() && selected.len() == 1);
    let resolver = Resolver {
        ladder,
        src: a,
        ctx,
        current,
        target,
        one_site: selected.len() == 1,
        own_at: a.by_own_lines().then(|| own.clone()),
    };

    // Rungs usable in this file, in ladder order, and why the others are not.
    let mut usable = Vec::new();
    let mut first_reason: Option<String> = None;
    for r in 0..ladder.len() {
        if ladder.invalid.get(r).is_some_and(Option::is_some) {
            continue;
        }
        if ladder.suspended.get(r).copied().unwrap_or(false) {
            let why = ladder
                .suspended_reason
                .get(r)
                .cloned()
                .flatten()
                .unwrap_or_else(|| "the rung is suspended".to_string());
            first_reason.get_or_insert(why);
            continue;
        }
        let text = ladder.rung(r).is_some_and(|x| x.within.is_none());
        if text && !single_site {
            let why = format!("pick which of {a_name}'s places to move");
            warnings.push(format!("{a_name}: {why}; it uses its own lines in {target}"));
            first_reason.get_or_insert(why);
            continue;
        }
        usable.push(r);
    }

    let mut claimed = vec![false; own.len()];
    let mut placed: Vec<Place> = Vec::new();
    let mut source: Option<String> = None;
    let uids_of = |w: &Window| -> Vec<u32> {
        (w.line..w.line + w.len)
            .map(|u| lines.resolve(to_u32(u)))
            .collect()
    };
    // The own match at each vanilla site: its first line's uid resolves to the site's. Regex
    // matches can share a line; each own match stands for one site, in order.
    let mut own_at_site: Vec<Option<usize>> = Vec::with_capacity(vanilla.len());
    for s in &vanilla {
        let first = lines.resolve(to_u32(s.line));
        let at = own.iter().enumerate().position(|(i, o)| {
            !own_at_site.contains(&Some(i))
                && lines
                    .get(o.line)
                    .is_some_and(|t| lines.resolve(t.uid) == first)
        });
        own_at_site.push(at);
    }
    // The sites placed so far and their places, to keep one place per site (D9).
    let mut placed_by: Vec<usize> = Vec::new();
    for &si in &selected {
        let Some(s) = vanilla.get(si) else { continue };
        let uids = uids_of(s);
        let own_at = own_at_site.get(si).copied().flatten();
        let mut res = SiteResolution {
            site: to_u32(si),
            vanilla_line: lines.file_line(to_u32(s.line)).map(to_u32),
            ..SiteResolution::default()
        };
        let mut reason = first_reason.clone();
        for &r in &usable {
            match resolver.try_rung(r, Some(&uids)) {
                Ok(hit) => {
                    // Another site already goes there: this site keeps its own match, so
                    // it is not lost (D9).
                    let taken = placed.iter().position(|p| p.same_spot(&hit.place));
                    if let Some(k) = taken {
                        let other = placed_by
                            .get(k)
                            .and_then(|&o| vanilla.get(o))
                            .and_then(|w| lines.file_line(to_u32(w.line)));
                        reason.get_or_insert(match other {
                            Some(l) => format!(
                                "the rung puts it where its place at line {l} already goes"
                            ),
                            None => "the rung puts it where another of its places already goes"
                                .to_string(),
                        });
                        continue;
                    }
                    // A's own matches for this site: the one on its line, and any on the
                    // lines of the B run that covers it (B replaced the line), but never
                    // another site's (D9: one place per site).
                    let mine = |i: usize| {
                        own_at == Some(i)
                            || (!own_at_site.contains(&Some(i))
                                && own.get(i).is_some_and(|o| {
                                    !o.lines.is_empty()
                                        && o.lines.iter().all(|l| hit.covered.contains(l))
                                }))
                    };
                    for (i, c) in claimed.iter_mut().enumerate() {
                        if mine(i) {
                            *c = true;
                        }
                    }
                    placed.push(hit.place);
                    placed_by.push(si);
                    res.rung = Some(to_u32(r));
                    res.anchored_on = Some(anchored_on(ladder, r));
                    res.followed = hit.followed;
                    res.b_drift = hit.followed;
                    source.get_or_insert_with(|| ladder.source(r));
                    reason = None;
                    break;
                }
                Err(why) => {
                    reason.get_or_insert(why);
                }
            }
        }
        if res.rung.is_none() {
            res.fallback = match (reason, own_at) {
                (Some(why), _) => Some(why),
                (None, None) => Some(format!("the line is no longer in {target}")),
                (None, Some(_)) => None,
            };
        }
        sites_out.push(res);
    }
    if vanilla.is_empty() {
        // A exists only on B's code (D9): one place, at the B site the rung picks.
        let mut res = SiteResolution::default();
        let mut reason = first_reason.clone();
        for &r in &usable {
            match resolver.try_rung(r, None) {
                Ok(hit) => {
                    // A text rung has no B lines: A has no place of its own to keep, so every
                    // own match goes (no second copy, D9).
                    let text = ladder.rung(r).is_some_and(|x| x.within.is_none());
                    for (i, o) in own.iter().enumerate() {
                        let inside =
                            text || o.lines.iter().all(|l| hit.b_lines.contains(l));
                        if inside {
                            if let Some(c) = claimed.get_mut(i) {
                                *c = true;
                            }
                        }
                    }
                    placed.push(hit.place);
                    res.rung = Some(to_u32(r));
                    res.anchored_on = Some(anchored_on(ladder, r));
                    res.followed = hit.followed;
                    res.b_drift = hit.followed;
                    source.get_or_insert_with(|| ladder.source(r));
                    reason = None;
                    break;
                }
                Err(why) => {
                    reason.get_or_insert(why);
                }
            }
        }
        if res.rung.is_none() {
            res.fallback = reason;
        }
        sites_out.push(res);
    }

    // Placements and the unclaimed own matches, in text order, never overlapping.
    let mut all: Vec<Place> = placed;
    all.extend(
        own.into_iter()
            .zip(&claimed)
            .filter(|(_, &c)| !c)
            .map(|(p, _)| p),
    );
    all.sort_by_key(|p| (p.line, p.found.map_or(0, |f| f.start), p.len));
    let mut places: Vec<Place> = Vec::with_capacity(all.len());
    for p in all {
        if let Some(last) = places.last() {
            if last.same_spot(&p) {
                continue;
            }
            if last.overlaps(&p) {
                warnings.push(format!(
                    "{a_name}: overlapping placement at line {} in {target} skipped",
                    p.line + 1
                ));
                continue;
            }
        }
        places.push(p);
    }
    sites_out.sort_by_key(|s| (s.lost, s.site));
    let source = source
        .or_else(|| usable.first().map(|&r| ladder.source(r)))
        .unwrap_or_else(|| ladder.source(0));
    Resolved {
        places,
        report: AnchorReport {
            sites: sites_out,
            source,
        },
        warnings,
    }
}

fn anchored_on(ladder: &Ladder, r: usize) -> String {
    ladder
        .rung(r)
        .and_then(|x| x.within.clone())
        .unwrap_or_else(|| "text".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_nth_and_unique() {
        let w = |line| Window::contiguous(line, 1, String::new());
        assert_eq!(pick(vec![w(3)], None, "B").map(|x| x.line), Ok(3));
        assert!(pick(vec![w(3), w(5)], None, "B")
            .unwrap_err()
            .contains("matches 2 lines"));
        assert_eq!(pick(vec![w(3), w(5)], Some(2), "B").map(|x| x.line), Ok(5));
        assert!(pick(vec![w(3)], Some(2), "B").is_err());
        assert!(pick(Vec::<Window>::new(), None, "B").unwrap_err().contains("no longer"));
    }

    #[test]
    fn an_implicit_anchor_never_covers_a_line() {
        use crate::patch::linemap::Edit;
        // Patch 1 adds a line below line 0 with no anchor of its own; with the line rules
        // it hangs on line 0 implicitly (D51), which must not count as touching it.
        for (rules, attach) in [(false, false), (true, false), (true, true)] {
            let mut map = LineMap::new(2, &[], &[]);
            map.set_line_rules(rules);
            map.set_implicit_chains(rules);
            let attach = attach.then_some(crate::patch::linemap::Attach {
                uid: 0,
                side: crate::patch::linemap::Side::After,
                block: crate::patch::linemap::NO_BLOCK,
            });
            let edit = Edit::Lines {
                at: 1,
                removed: 0,
                inserted: 1,
                site: 0,
                attach,
                kept: Vec::new(),
            };
            map.commit_with(0, 1, 0, &[edit], 3, true);
            let runs = map.site_runs(1, rules);
            assert_eq!(runs.len(), 1);
            let implicit = map.blocks().first().is_some_and(|b| b.implicit);
            assert_eq!(implicit, rules && attach.is_none());
            assert_eq!(covers(&map, &runs[0], &[0], false), attach.is_some(), "{rules} {attach:?}");
        }
    }

    #[test]
    fn source_labels_and_rank() {
        let p = AnchorSource::Profile {
            fix_id: "fx_1".to_string(),
        };
        assert_eq!(p.label(), "profile:fx_1");
        assert_eq!(AnchorSource::Author.label(), "author");
        assert!(p.rank() < AnchorSource::Author.rank());
    }
}
