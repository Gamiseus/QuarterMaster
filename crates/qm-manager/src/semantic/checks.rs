//! The N10 checks (§6.9). Task 37a: `capture_before_define`, `redefined_global` (C4),
//! `dead_code` and `early_return_shadow` (C5). Task 37b: `table_literal_reset` (C4),
//! `shared_file_writer` (C21), `key_collision` (C22) and `double_hook` (C20). Task 37c:
//! `bypassed_hook` (C19), `shared_field` and `name_collision` (C22), `block_balance` (C11);
//! `moved_scope` and `free_name_changed` are in `placement::moved_code`. Each finding names
//! the patch whose code is affected (A) and the patch that causes it (B), with a "why" text.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use qm_core::handshake::PatchOutcomeStatus;
use qm_core::patch::origin::FileTrace;
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, Patch};

use super::index::{self, Capture, Def, DefKind, SourceIndex};
use super::{Origin, Program, RunAt, ID_PREFIX};
use crate::conflict::{DetectedConflict, SimulatedFileResult};

/// What the checks read besides the index: the table, the per-file records, the game's files
/// and the patched texts.
pub struct Ctx<'a> {
    pub table: &'a PatchTable,
    pub files: &'a [SimulatedFileResult],
    pub vanilla: &'a HashMap<String, String>,
    pub traces: &'a std::collections::BTreeMap<String, Arc<FileTrace>>,
}

/// One finding before it becomes a conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub check: &'static str,
    /// What `overrides = ["<check>:<name>"]` names: the global, or the function (the file
    /// for a main chunk).
    pub name: String,
    pub severity: &'static str,
    /// The file (or module) of A's code.
    pub file: String,
    /// The patch whose code is affected.
    pub a: Origin,
    /// The patch that causes it; none for the game's own code.
    pub b: Option<Origin>,
    pub description: String,
}

impl Finding {
    /// `semantic:<check>:<file>:<patch A>:<patch B or ->@<name>`: no line numbers, so the id
    /// stays the same when a fix moves code.
    pub fn into_conflict(self) -> DetectedConflict {
        let b_id = self.b.as_ref().map(|b| b.patch_id.as_str()).unwrap_or("-");
        DetectedConflict {
            id: format!(
                "{ID_PREFIX}{}:{}:{}:{b_id}@{}",
                self.check, self.file, self.a.patch_id, self.name
            ),
            severity: self.severity.to_string(),
            kind: self.check.to_string(),
            target_file: self.file,
            mod_a_id: self.a.mod_id,
            mod_b_id: self.b.as_ref().map(|b| b.mod_id.clone()),
            patch_a_id: self.a.patch_id,
            patch_b_id: self.b.map(|b| b.patch_id),
            description: self.description,
            suggestions: Vec::new(),
            consumer: None,
            sites_lost: None,
            same_line: None,
        }
    }
}

/// Every check, in a fixed order.
pub fn run(p: &Program, cx: &Ctx<'_>) -> Vec<Finding> {
    let mut out = capture_before_define(p);
    out.extend(redefined_global(p));
    out.extend(dead_code(p));
    out.extend(early_return_shadow(p));
    out.extend(table_literal_reset(p));
    out.extend(shared_file_writer(p));
    out.extend(key_collision(p));
    out.extend(double_hook(p));
    out.extend(bypassed_hook(p, cx));
    out.extend(shared_field(p, cx));
    out.extend(name_collision(p, cx));
    out.extend(block_balance(cx));
    out
}

fn same_mod(a: &Origin, b: &Origin) -> bool {
    a.mod_id.eq_ignore_ascii_case(&b.mod_id)
}

fn is_dead(p: &Program, src: usize, line: usize) -> bool {
    p.sources
        .get(src)
        .and_then(|s| s.index.line(line))
        .is_some_and(|l| l.dead.is_some())
}

/// A global definition a patch wrote, with when it runs.
struct PatchDef<'a> {
    src: usize,
    def: &'a Def,
    origin: &'a Origin,
    run: RunAt,
}

impl PatchDef<'_> {
    fn place(&self, p: &Program) -> String {
        place(p, self.src, self.def.line)
    }
}

fn place(p: &Program, src: usize, line: usize) -> String {
    let key = p.sources.get(src).map(|s| s.key.as_str()).unwrap_or("");
    format!("{key} line {line}")
}

fn file_of(p: &Program, src: usize) -> String {
    p.sources.get(src).map(|s| s.key.clone()).unwrap_or_default()
}

/// Definitions patches wrote, outside never-taken branches.
fn patch_defs(p: &Program) -> Vec<PatchDef<'_>> {
    let mut out = Vec::new();
    for (src, s) in p.sources.iter().enumerate() {
        for def in &s.index.defs {
            let Some(origin) = p.origin(src, def.line) else {
                continue;
            };
            if is_dead(p, src, def.line) {
                continue;
            }
            out.push(PatchDef { src, def, origin, run: p.run_at(src, def.line, def.function) });
        }
    }
    out
}

/// A wrapper a patch installed: its copy of the global and its new definition.
struct PatchWrapper<'a> {
    src: usize,
    def: &'a Def,
    capture: &'a Capture,
    origin: &'a Origin,
    def_run: RunAt,
    cap_run: RunAt,
    calls: bool,
}

/// Wrappers patches wrote (copy and definition from one mod), one per definition: the copy it
/// calls, else the nearest one.
fn patch_wrappers(p: &Program) -> Vec<PatchWrapper<'_>> {
    let mut out = Vec::new();
    for (src, s) in p.sources.iter().enumerate() {
        let mut per_def: BTreeMap<usize, PatchWrapper<'_>> = BTreeMap::new();
        for w in &s.index.wrappers {
            let (Some(def), Some(capture)) = (s.index.defs.get(w.def), s.index.captures.get(w.capture)) else {
                continue;
            };
            let (Some(origin), Some(cap_origin)) = (p.origin(src, def.line), p.origin(src, capture.line)) else {
                continue;
            };
            if !same_mod(origin, cap_origin) || is_dead(p, src, def.line) || is_dead(p, src, capture.line) {
                continue;
            }
            let pw = PatchWrapper {
                src,
                def,
                capture,
                origin,
                def_run: p.run_at(src, def.line, def.function),
                cap_run: p.run_at(src, capture.line, capture.function),
                calls: w.calls,
            };
            let better = |old: &PatchWrapper<'_>| {
                (pw.calls && !old.calls) || (pw.calls == old.calls && capture.line > old.capture.line)
            };
            match per_def.get(&w.def) {
                Some(old) if !better(old) => {}
                _ => {
                    per_def.insert(w.def, pw);
                }
            }
        }
        out.extend(per_def.into_values());
    }
    out
}

/// How a later wrapper drops an earlier one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dropped {
    /// The later wrapper copied the global before the earlier one was installed.
    CopiedFirst,
    /// The later wrapper never calls its copy.
    NoCall,
}

/// Whether wrapper `late` (another mod, installed after `early`) drops `early`.
fn dropped(early: &PatchWrapper<'_>, late: &PatchWrapper<'_>) -> Option<Dropped> {
    if same_mod(early.origin, late.origin)
        || early.def.name != late.def.name
        || early.def_run.before(late.def_run) != Some(true)
    {
        return None;
    }
    if late.cap_run.before(early.def_run) == Some(true) {
        Some(Dropped::CopiedFirst)
    } else if !late.calls {
        Some(Dropped::NoCall)
    } else {
        None
    }
}

/// The wrapper `d` installs, if it is one.
fn wrapper_of<'w, 'a>(ws: &'w [PatchWrapper<'a>], d: &PatchDef<'_>) -> Option<&'w PatchWrapper<'a>> {
    ws.iter().find(|w| w.src == d.src && std::ptr::eq(w.def, d.def))
}

/// `local _orig = f` in A runs before B's patch defines `f`: A keeps the old value (or nil).
/// It matters when A only calls its copy, or when A's own `f` replaces B's later. When B's
/// `f` replaces A's wrapper, the chain is B's business (`redefined_global` when B drops it).
/// When both are wrappers, `double_hook` reports it.
fn capture_before_define(p: &Program) -> Vec<Finding> {
    let defs = patch_defs(p);
    let ws = patch_wrappers(p);
    let mut out = Vec::new();
    for (src, s) in p.sources.iter().enumerate() {
        for c in &s.index.captures {
            let Some(a) = p.origin(src, c.line) else {
                continue;
            };
            if is_dead(p, src, c.line) {
                continue;
            }
            let run = p.run_at(src, c.line, c.function);
            for d in defs.iter().filter(|d| d.def.name == c.name && !same_mod(d.origin, a)) {
                if run.before(d.run) != Some(true) {
                    continue;
                }
                let own: Vec<&PatchDef<'_>> =
                    defs.iter().filter(|o| o.def.name == c.name && same_mod(o.origin, a)).collect();
                if !own.is_empty() && !own.iter().any(|o| d.run.before(o.run) == Some(true)) {
                    continue;
                }
                let hook = wrapper_of(&ws, d).is_some_and(|wd| {
                    ws.iter().any(|wa| same_mod(wa.origin, a) && dropped(wd, wa).is_some())
                });
                if hook {
                    continue;
                }
                out.push(Finding {
                    check: "capture_before_define",
                    name: c.name.clone(),
                    severity: "warning",
                    file: s.key.clone(),
                    a: a.clone(),
                    b: Some(d.origin.clone()),
                    description: format!(
                        "Patch '{}' (from '{}') copies `{}` into `{}` at {}, which runs before patch '{}' (from '{}') defines `{}` at {}. The copy keeps the old value (or nil), so code that calls `{}` does not run '{}''s version.",
                        a.patch_name, a.mod_id, c.name, c.local, place(p, src, c.line),
                        d.origin.patch_name, d.origin.mod_id, c.name, d.place(p),
                        c.local, d.origin.mod_id
                    ),
                });
            }
        }
    }
    out
}

/// Whether `d`'s mod copies the name before `d` defines it: a wrapper that keeps the chain.
fn wraps(p: &Program, d: &PatchDef<'_>) -> bool {
    p.sources.iter().enumerate().any(|(src, s)| {
        s.index.captures.iter().any(|c| {
            c.name == d.def.name
                && p.origin(src, c.line).is_some_and(|o| same_mod(o, d.origin))
                && (p.run_at(src, c.line, c.function).before(d.run) == Some(true)
                    || (src == d.src && c.line < d.def.line))
        })
    })
}

/// Two mods define one global function and the later one does not wrap the other (or wraps
/// it and never calls its copy): the earlier definition is lost. Two wrappers are left to
/// `double_hook`.
fn redefined_global(p: &Program) -> Vec<Finding> {
    let defs = patch_defs(p);
    let ws = patch_wrappers(p);
    let mut by_name: BTreeMap<&str, Vec<&PatchDef<'_>>> = BTreeMap::new();
    for d in defs.iter().filter(|d| d.def.kind == DefKind::Function) {
        by_name.entry(d.def.name.as_str()).or_default().push(d);
    }
    let mut out = Vec::new();
    for (name, list) in by_name {
        let mods: BTreeSet<String> = list.iter().map(|d| d.origin.mod_id.to_lowercase()).collect();
        if mods.len() < 2 {
            continue;
        }
        // The definition every other one runs before, when there is one.
        let winner = list.iter().find(|w| {
            list.iter().all(|d| std::ptr::eq(*d, **w) || d.run.before(w.run) == Some(true))
        });
        match winner {
            Some(w) => {
                let w_hook = wrapper_of(&ws, w);
                if wraps(p, w) && !w_hook.is_some_and(|h| !h.calls) {
                    continue;
                }
                // Each other mod's last definition is lost.
                let mut lost: BTreeMap<String, &PatchDef<'_>> = BTreeMap::new();
                for d in list.iter().filter(|d| !same_mod(d.origin, w.origin)) {
                    lost.insert(d.origin.mod_id.to_lowercase(), *d);
                }
                for d in lost.values() {
                    if w_hook.is_some() && wrapper_of(&ws, d).is_some() {
                        continue;
                    }
                    out.push(Finding {
                        check: "redefined_global",
                        name: name.to_string(),
                        severity: "warning",
                        file: file_of(p, d.src),
                        a: d.origin.clone(),
                        b: Some(w.origin.clone()),
                        description: format!(
                            "Patch '{}' (from '{}') defines `{name}` at {}, and patch '{}' (from '{}') defines it again at {}. '{}''s definition runs later and replaces the other, so '{}''s `{name}` never runs.",
                            d.origin.patch_name, d.origin.mod_id, d.place(p),
                            w.origin.patch_name, w.origin.mod_id, w.place(p),
                            w.origin.mod_id, d.origin.mod_id
                        ),
                    });
                }
            }
            None => {
                if list.iter().any(|d| wraps(p, d)) {
                    continue;
                }
                let Some(first) = list.first() else {
                    continue;
                };
                let Some(other) = list.iter().find(|d| !same_mod(d.origin, first.origin)) else {
                    continue;
                };
                out.push(Finding {
                    check: "redefined_global",
                    name: name.to_string(),
                    severity: "warning",
                    file: file_of(p, first.src),
                    a: first.origin.clone(),
                    b: Some(other.origin.clone()),
                    description: format!(
                        "Patch '{}' (from '{}') defines `{name}` at {}, and patch '{}' (from '{}') defines it too at {}. Which one runs last depends on when the game calls the code around them; the last one replaces the other.",
                        first.origin.patch_name, first.origin.mod_id, first.place(p),
                        other.origin.patch_name, other.origin.mod_id, other.place(p)
                    ),
                });
            }
        }
    }
    out
}

/// A patch's code inside a branch that never runs (`if(1 == 0) then`). A guard a patch adds
/// around the game's own code is a note.
fn dead_code(p: &Program) -> Vec<Finding> {
    let mut out = Vec::new();
    for (src, s) in p.sources.iter().enumerate() {
        // Per guard: the first line of each patch's code in it, and the game's lines.
        let mut code: BTreeMap<(usize, usize), (usize, &Origin)> = BTreeMap::new();
        let mut game: BTreeMap<usize, (usize, usize, usize)> = BTreeMap::new();
        for (i, facts) in s.index.lines.iter().enumerate() {
            let (Some(g), true) = (facts.dead, facts.code) else {
                continue;
            };
            let line = i + 1;
            let ix = s.origins.get(i).copied().flatten();
            match (ix, p.origin(src, line)) {
                (Some(ix), Some(o)) => {
                    code.entry((g, ix)).or_insert((line, o));
                }
                _ => {
                    let e = game.entry(g).or_insert((line, line, 0));
                    e.1 = line;
                    e.2 += 1;
                }
            }
        }
        let guard_text = |g: usize| {
            s.index
                .guards
                .get(g)
                .map(|g| (g.line, g.condition.as_str(), g.function))
                .unwrap_or((0, "", 0))
        };
        for ((g, _), (line, a)) in code {
            let (gline, cond, function) = guard_text(g);
            let b = p.origin(src, gline);
            if b.is_some_and(|b| same_mod(a, b)) {
                continue;
            }
            let by = match b {
                Some(b) => format!("from patch '{}' of '{}'", b.patch_name, b.mod_id),
                None => "in the game's code".to_string(),
            };
            out.push(Finding {
                check: "dead_code",
                name: p.scope_name(src, function),
                severity: "warning",
                file: s.key.clone(),
                a: a.clone(),
                b: b.cloned(),
                description: format!(
                    "Patch '{}' (from '{}') adds code at {}, inside a branch of `if {cond} then` at line {gline} ({by}) that never runs. That code never runs.",
                    a.patch_name, a.mod_id, place(p, src, line)
                ),
            });
        }
        for (g, (first, last, n)) in game {
            let (gline, cond, function) = guard_text(g);
            let Some(b) = p.origin(src, gline) else {
                continue;
            };
            let lines = if n == 1 { "1 line".to_string() } else { format!("{n} lines") };
            out.push(Finding {
                check: "dead_code",
                name: p.scope_name(src, function),
                severity: "info",
                file: s.key.clone(),
                a: b.clone(),
                b: None,
                description: format!(
                    "Patch '{}' (from '{}') adds `if {cond} then` at {}, which puts {lines} of the game's code (lines {first} to {last}) in a branch that never runs.",
                    b.patch_name, b.mod_id, place(p, src, gline)
                ),
            });
        }
    }
    out
}

/// A patch's code after another patch's `return` in the same function: it never runs (and a
/// plain `return` there does not compile).
fn early_return_shadow(p: &Program) -> Vec<Finding> {
    let mut out = Vec::new();
    for (src, s) in p.sources.iter().enumerate() {
        for r in s.index.returns.iter().filter(|r| r.function_level) {
            let Some(b) = p.origin(src, r.line) else {
                continue;
            };
            if is_dead(p, src, r.line) {
                continue;
            }
            let end = s.index.end_line(r.function);
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            for line in r.line + 1..=end {
                if !s.index.line(line).is_some_and(|l| l.code) {
                    continue;
                }
                let Some(a) = p.origin(src, line) else {
                    continue;
                };
                if same_mod(a, b) || !seen.insert(a.patch_id.as_str()) {
                    continue;
                }
                let scope = s.index.function_name(r.function);
                let what = if r.function == 0 { "the file's main chunk".to_string() } else { format!("`{scope}`") };
                out.push(Finding {
                    check: "early_return_shadow",
                    name: p.scope_name(src, r.function),
                    severity: "warning",
                    file: s.key.clone(),
                    a: a.clone(),
                    b: Some(b.clone()),
                    description: format!(
                        "Patch '{}' (from '{}') returns from {what} at {}. Patch '{}' (from '{}') adds code after it at line {line}, so that code never runs.",
                        b.patch_name, b.mod_id, place(p, src, r.line), a.patch_name, a.mod_id
                    ),
                });
            }
        }
    }
    out
}

/// Another mod's patch adds fields to global table `X` (or fills its own `X = {...}`), and
/// then a patch runs `X = {...}`: a new table, so those fields are gone. `X = X or {}` keeps
/// the table and is not a literal here.
fn table_literal_reset(p: &Program) -> Vec<Finding> {
    let defs = patch_defs(p);
    let mut out = Vec::new();
    for lit in defs.iter().filter(|d| d.def.kind == DefKind::Table) {
        let prefix = format!("{}.", lit.def.name);
        // Per other mod, its first field (or filled literal) that runs before this literal.
        let mut hit: BTreeMap<String, (usize, usize, &Origin, String, bool)> = BTreeMap::new();
        for (src, s) in p.sources.iter().enumerate() {
            for f in s.index.field_writes.iter().filter(|f| !f.local && f.name.starts_with(&prefix)) {
                let Some(a) = p.origin(src, f.line) else {
                    continue;
                };
                if same_mod(a, lit.origin) || is_dead(p, src, f.line) {
                    continue;
                }
                if p.run_at(src, f.line, f.function).before(lit.run) != Some(true) {
                    continue;
                }
                hit.entry(a.mod_id.to_lowercase()).or_insert((src, f.line, a, f.name.clone(), false));
            }
        }
        for d in defs.iter().filter(|d| {
            d.def.kind == DefKind::Table
                && d.def.name == lit.def.name
                && !d.def.empty
                && !same_mod(d.origin, lit.origin)
                && d.run.before(lit.run) == Some(true)
        }) {
            hit.entry(d.origin.mod_id.to_lowercase())
                .or_insert((d.src, d.def.line, d.origin, d.def.name.clone(), true));
        }
        let name = &lit.def.name;
        for (src, line, a, field, filled) in hit.into_values() {
            let what = if filled {
                format!("makes `{name}` with fields at {}", place(p, src, line))
            } else {
                format!("sets `{field}` at {}", place(p, src, line))
            };
            out.push(Finding {
                check: "table_literal_reset",
                name: name.clone(),
                severity: "warning",
                file: file_of(p, src),
                a: a.clone(),
                b: Some(lit.origin.clone()),
                description: format!(
                    "Patch '{}' (from '{}') {what}. Patch '{}' (from '{}') sets `{name} = {{...}}` at {}, which runs later and makes a new table, so what '{}' put in `{name}` is gone.",
                    a.patch_name, a.mod_id, lit.origin.patch_name, lit.origin.mod_id,
                    lit.place(p), a.mod_id
                ),
            });
        }
    }
    out
}

/// One mod's writes of one file name.
struct FileWriter<'a> {
    src: usize,
    line: usize,
    origin: &'a Origin,
    path: &'a str,
    /// Every write is in a function that reads the file before it.
    merges: bool,
}

/// Two mods write the same file name. A writer merges when its function reads the file
/// before writing it; one that does not drops what the other saved.
fn shared_file_writer(p: &Program) -> Vec<Finding> {
    let mut by_path: BTreeMap<String, BTreeMap<String, FileWriter<'_>>> = BTreeMap::new();
    for (src, s) in p.sources.iter().enumerate() {
        for w in s.index.files.iter().filter(|f| f.write) {
            let Some(origin) = p.origin(src, w.line) else {
                continue;
            };
            if is_dead(p, src, w.line) {
                continue;
            }
            let merges = s.index.files.iter().any(|r| {
                !r.write && r.function == w.function && r.line <= w.line && r.path.eq_ignore_ascii_case(&w.path)
            });
            by_path
                .entry(w.path.to_lowercase())
                .or_default()
                .entry(origin.mod_id.to_lowercase())
                .and_modify(|e| e.merges &= merges)
                .or_insert(FileWriter { src, line: w.line, origin, path: &w.path, merges });
        }
    }
    let mut out = Vec::new();
    for writers in by_path.values() {
        let list: Vec<&FileWriter<'_>> = writers.values().collect();
        for (i, x) in list.iter().enumerate() {
            for y in list.iter().skip(i + 1) {
                // A is the writer whose data can be lost.
                let (a, b, severity) = match (x.merges, y.merges) {
                    (true, true) => (x, y, "info"),
                    (false, true) => (y, x, "warning"),
                    _ => (x, y, "warning"),
                };
                let why = match (a.merges, b.merges) {
                    (true, true) => "Both read the file first and keep what is in it.".to_string(),
                    (true, false) => format!(
                        "'{}' writes the whole file without reading it first, so it drops what '{}' saved there.",
                        b.origin.mod_id, a.origin.mod_id
                    ),
                    _ => "Neither reads the file first, so the one that writes last drops what the other saved.".to_string(),
                };
                out.push(Finding {
                    check: "shared_file_writer",
                    name: a.path.to_string(),
                    severity,
                    file: file_of(p, a.src),
                    a: a.origin.clone(),
                    b: Some(b.origin.clone()),
                    description: format!(
                        "Patch '{}' (from '{}') writes `{}` at {}, and patch '{}' (from '{}') writes it too at {}. {why}",
                        a.origin.patch_name, a.origin.mod_id, a.path, place(p, a.src, a.line),
                        b.origin.patch_name, b.origin.mod_id, place(p, b.src, b.line)
                    ),
                });
            }
        }
    }
    out
}

/// Whether function `f` (or one around it) handles key releases.
fn key_event(ix: &SourceIndex, mut f: usize) -> &'static str {
    for _ in 0..=ix.functions.len() {
        let Some(func) = ix.functions.get(f) else {
            break;
        };
        if func.name.to_ascii_lowercase().contains("release") {
            return "release";
        }
        if f == 0 {
            break;
        }
        f = func.parent;
    }
    "press"
}

/// `Ctrl+Shift+F9`: modifiers first, a short key in capitals.
fn key_label(modifiers: &[&str], key: &str) -> String {
    let mut parts: Vec<String> = modifiers
        .iter()
        .map(|m| {
            let mut c = m.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect();
    let fkey = key.len() > 1
        && key.starts_with('f')
        && key.get(1..).is_some_and(|r| r.bytes().all(|b| b.is_ascii_digit()));
    parts.push(if key.chars().count() == 1 || fkey { key.to_uppercase() } else { key.to_string() });
    parts.join("+")
}

/// One mod's first check of a key: source, line, patch, label.
type KeyUse<'a> = (usize, usize, &'a Origin, String);

/// Two mods act on the same key with the same modifiers (`key == "f9"` plus the same
/// `isDown` modifier checks in the condition), in the same event (press or release).
fn key_collision(p: &Program) -> Vec<Finding> {
    // (event, name) -> per mod: its first check.
    let mut by_key: BTreeMap<(&str, String), BTreeMap<String, KeyUse<'_>>> = BTreeMap::new();
    for (src, s) in p.sources.iter().enumerate() {
        for k in &s.index.keys {
            let Some(origin) = p.origin(src, k.line) else {
                continue;
            };
            if is_dead(p, src, k.line) {
                continue;
            }
            let mut name: String = k.modifiers.iter().map(|m| format!("{m}+")).collect();
            name.push_str(&k.key);
            by_key
                .entry((key_event(&s.index, k.function), name))
                .or_default()
                .entry(origin.mod_id.to_lowercase())
                .or_insert((src, k.line, origin, key_label(&k.modifiers, &k.key)));
        }
    }
    let mut out = Vec::new();
    for ((event, name), checks) in &by_key {
        let list: Vec<_> = checks.values().collect();
        for (i, (a_src, a_line, a, label)) in list.iter().copied().enumerate() {
            for (b_src, b_line, b, _) in list.iter().copied().skip(i + 1) {
                let verb = if *event == "release" { "releases" } else { "presses" };
                out.push(Finding {
                    check: "key_collision",
                    name: name.clone(),
                    severity: "warning",
                    file: file_of(p, *a_src),
                    a: (*a).clone(),
                    b: Some((*b).clone()),
                    description: format!(
                        "Patch '{}' (from '{}') acts on {label} at {}, and patch '{}' (from '{}') acts on {label} too at {}. Both run when the player {verb} {label}, unless one of them stops the other first.",
                        a.patch_name, a.mod_id, place(p, *a_src, *a_line),
                        b.patch_name, b.mod_id, place(p, *b_src, *b_line)
                    ),
                });
            }
        }
    }
    out
}

/// Two mods wrap the same global function (`local _orig = f` then a new `f`), and the one
/// installed later drops the other: it copied `f` before the other wrapper was installed,
/// or it never calls its copy.
fn double_hook(p: &Program) -> Vec<Finding> {
    let ws = patch_wrappers(p);
    let mut out = Vec::new();
    for early in &ws {
        for late in &ws {
            let Some(how) = dropped(early, late) else {
                continue;
            };
            let name = &early.def.name;
            let (a, b) = (early.origin, late.origin);
            let how = match how {
                Dropped::CopiedFirst => format!(
                    "Patch '{}' (from '{}') wraps it too, but it copies `{name}` at {}, before '{}''s wrapper is installed, and installs its own at {}. Its wrapper calls the copy it took, so it skips '{}''s.",
                    b.patch_name, b.mod_id, place(p, late.src, late.capture.line), a.mod_id,
                    place(p, late.src, late.def.line), a.mod_id
                ),
                Dropped::NoCall => format!(
                    "Patch '{}' (from '{}') wraps it again later at {}, but its new `{name}` never calls the copy it took, so it does not call '{}''s.",
                    b.patch_name, b.mod_id, place(p, late.src, late.def.line), a.mod_id
                ),
            };
            out.push(Finding {
                check: "double_hook",
                name: name.clone(),
                severity: "warning",
                file: file_of(p, early.src),
                a: a.clone(),
                b: Some(b.clone()),
                description: format!(
                    "Patch '{}' (from '{}') wraps `{name}` at {}. {how} '{}''s wrapper never runs.",
                    a.patch_name, a.mod_id, place(p, early.src, early.def.line), a.mod_id
                ),
            });
        }
    }
    out
}

// Task 37c checks.

fn find_patch<'t>(table: &'t PatchTable, id: &str, mod_id: &str) -> Option<(usize, &'t Patch)> {
    table
        .patches
        .iter()
        .enumerate()
        .find(|(_, (p, _, _))| p.id() == Some(id) && p.mod_id().is_some_and(|m| m.eq_ignore_ascii_case(mod_id)))
        .map(|(i, (p, _, _))| (i, p))
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `word` stands alone at byte `i` of `text` (no name character on either side).
fn whole_word(text: &str, i: usize, word: &str) -> bool {
    let bytes = text.as_bytes();
    let before = i.checked_sub(1).and_then(|j| bytes.get(j)).is_some_and(|&b| is_ident(b));
    let after = bytes.get(i + word.len()).is_some_and(|&b| is_ident(b));
    !before && !after
}

fn is_chain(b: u8) -> bool {
    is_ident(b) || b == b'.' || b == b':'
}

/// The functions the game defines (`function a.b(`, `a.b = function`), in one pass.
fn vanilla_functions(vanilla: &HashMap<String, String>) -> HashSet<String> {
    let mut out = HashSet::new();
    for text in vanilla.values() {
        for (i, _) in text.match_indices("function") {
            if !whole_word(text, i, "function") {
                continue;
            }
            // `function a.b(`.
            let rest = text.get(i + "function".len()..).unwrap_or("");
            let name: String = rest
                .trim_start_matches([' ', '\t'])
                .bytes()
                .take_while(|&b| is_chain(b))
                .map(char::from)
                .collect();
            if !name.is_empty() {
                out.insert(name);
            }
            // `a.b = function`.
            let head = text.get(..i).unwrap_or("").trim_end_matches([' ', '\t']);
            if let Some(h) = head.strip_suffix('=').filter(|h| !h.ends_with(['=', '~', '<', '>'])) {
                let h = h.trim_end_matches([' ', '\t']);
                let start = h.bytes().rev().take_while(|&b| is_chain(b)).count();
                if let Some(n) = h.get(h.len() - start..).filter(|n| !n.is_empty()) {
                    out.insert(n.to_string());
                }
            }
        }
    }
    out
}

/// Whether the game's own code still calls `name` after the patches: a game line (not a
/// patch's) of a patched file, or any call in a file no patch touched.
fn game_still_calls(p: &Program, vanilla: &HashMap<String, String>, name: &str) -> bool {
    vanilla.iter().filter(|(_, t)| t.contains(name)).any(|(key, text)| {
        match p.sources.iter().enumerate().find(|(_, s)| !s.module && s.key == *key) {
            Some((src, s)) => s.index.calls.iter().any(|c| c.name == name && p.origin(src, c.line).is_none()),
            None => text.match_indices(name).any(|(i, _)| {
                let before = text.get(..i).unwrap_or("");
                let rest = text.get(i + name.len()..).unwrap_or("").trim_start_matches([' ', '\t']);
                !before.as_bytes().last().is_some_and(|&b| is_chain(b))
                    && !rest.as_bytes().first().is_some_and(|&b| is_ident(b))
                    && rest.starts_with(['(', '"', '\'', '{'])
                    && !before.trim_end_matches([' ', '\t']).ends_with("function")
            }),
        }
    })
}

/// Whether a line of the game's own code still has the token run `cond`.
fn game_still_tests(p: &Program, vanilla: &HashMap<String, String>, cond: &[String]) -> bool {
    let has = |line: &str| {
        let toks = index::token_texts(line);
        toks.windows(cond.len()).any(|w| w.iter().zip(cond).all(|(a, b)| a == b))
    };
    let Some(word) = cond.iter().max_by_key(|t| t.len()) else {
        return false;
    };
    vanilla.iter().filter(|(_, t)| t.contains(word.as_str())).any(|(key, text)| {
        match p.sources.iter().enumerate().find(|(_, s)| !s.module && s.key == *key) {
            Some((src, s)) => s
                .text
                .lines()
                .enumerate()
                .any(|(i, l)| l.contains(word.as_str()) && p.origin(src, i + 1).is_none() && has(l)),
            None => text.lines().any(|l| l.contains(word.as_str()) && has(l)),
        }
    })
}

/// The game sets a field or name `field` somewhere (`.field =`, `{ field = `): not a new field.
fn vanilla_sets(vanilla: &HashMap<String, String>, field: &str) -> bool {
    vanilla.values().any(|text| {
        text.match_indices(field).any(|(i, _)| {
            if !whole_word(text, i, field) {
                return false;
            }
            let rest = text.get(i + field.len()..).unwrap_or("").trim_start_matches([' ', '\t']);
            rest.starts_with('=') && !rest.starts_with("==")
        })
    })
}

/// Save field -> the mods that write it as `player.<field> = ...` (Phase E D84): a plain
/// value set on the global `player` table that the game's own code never sets. Writes in
/// dead code and QuarterMaster's own patches are left out.
pub(super) fn save_field_owners(
    p: &Program,
    vanilla: &HashMap<String, String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (src, s) in p.sources.iter().enumerate() {
        for w in &s.index.field_writes {
            if w.method || w.local || w.own_table {
                continue;
            }
            // Only `player.<field>`: a deeper write sets a part of a table someone else made.
            let mut parts = w.name.split('.');
            let (Some("player"), Some(field), None) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if field.is_empty() || is_dead(p, src, w.line) {
                continue;
            }
            let Some(origin) = p.origin(src, w.line) else {
                continue;
            };
            if origin.mod_id.eq_ignore_ascii_case("quartermaster") {
                continue;
            }
            out.entry(field.to_string())
                .or_default()
                .insert(origin.mod_id.to_lowercase());
        }
    }
    out.retain(|field, _| !vanilla_sets(vanilla, field));
    out
}

/// The game has the string literal `s`.
fn vanilla_literal(vanilla: &HashMap<String, String>, s: &str) -> bool {
    let (a, b) = (format!("\"{s}\""), format!("'{s}'"));
    vanilla.values().any(|t| t.contains(a.as_str()) || t.contains(b.as_str()))
}

/// The menu pages the game uses (`menuPage == 4`, `menuPage = 11`, `4 == menuPage`).
fn vanilla_pages(vanilla: &HashMap<String, String>) -> HashSet<String> {
    let digits = |s: &str| -> String { s.chars().take_while(|c| c.is_ascii_digit()).collect() };
    let mut out = HashSet::new();
    for text in vanilla.values().filter(|t| t.contains("menuPage")) {
        for (i, _) in text.match_indices("menuPage") {
            if !whole_word(text, i, "menuPage") {
                continue;
            }
            let rest = text.get(i + "menuPage".len()..).unwrap_or("").trim_start_matches([' ', '\t']);
            let rest = rest.strip_prefix("==").or_else(|| rest.strip_prefix('='));
            if let Some(r) = rest {
                let n = digits(r.trim_start_matches([' ', '\t']));
                if !n.is_empty() {
                    out.insert(n);
                }
            }
            let head = text.get(..i).unwrap_or("").trim_end_matches([' ', '\t', '(']);
            if let Some(h) = head.strip_suffix("==") {
                let h = h.trim_end_matches([' ', '\t']);
                let n: String = h.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
                if !n.is_empty() {
                    out.insert(n);
                }
            }
        }
    }
    out
}

/// Where a patch's code starts in `file` (`main.lua line 9`), or the file when it adds none.
fn patch_place(p: &Program, file: &str, patch_id: &str, mod_id: &str) -> String {
    for (src, s) in p.sources.iter().enumerate().filter(|(_, s)| s.key == file) {
        for line in 1..=s.origins.len() {
            if p.origin(src, line).is_some_and(|o| o.patch_id == patch_id && o.mod_id.eq_ignore_ascii_case(mod_id)) {
                return place(p, src, line);
            }
        }
    }
    file.to_string()
}

/// Tokens as Lua text: `unit.faction ~= target.faction`.
fn show(tokens: &[&str]) -> String {
    let mut s = String::new();
    let mut prev = "";
    for t in tokens {
        let tight = matches!(*t, "." | ":" | ")" | "]" | ",") || matches!(prev, "." | ":" | "(" | "[" | "#" | "")
            || (*t == "(" && prev.bytes().last().is_some_and(is_ident));
        if !tight {
            s.push(' ');
        }
        s.push_str(t);
        prev = t;
    }
    s
}

/// What an `at` patch rewrote: a call of a game function it no longer makes in its lines, or
/// a condition it no longer tests.
struct Rewrite {
    b: Origin,
    file: String,
    call: Option<String>,
    condition: Vec<String>,
}

/// B's `at` patches take a call of a game function, or a condition, out of every line of the
/// game's code that had it (B reroutes it), and a payload of A still calls or tests it: A's
/// code goes around B's change. Literal only: a call is a plain name the game defines, a
/// condition is compared token by token, so A's variable names must match.
fn bypassed_hook(p: &Program, cx: &Ctx<'_>) -> Vec<Finding> {
    let mut game_functions: Option<HashSet<String>> = None;
    let mut rewrites: Vec<Rewrite> = Vec::new();
    for file in cx.files {
        for rec in file.records.iter().filter(|r| r.status == PatchOutcomeStatus::Applied && r.patch_type == "pattern") {
            let Some((_, Patch::Pattern(b))) = find_patch(cx.table, &rec.patch_id, &rec.mod_id) else {
                continue;
            };
            if b.position != InsertPosition::At {
                continue;
            }
            let b_origin = Origin {
                mod_id: rec.mod_id.clone(),
                patch_id: rec.patch_id.clone(),
                patch_name: rec.patch_name.clone(),
            };
            let kept_calls: HashSet<String> = index::index(&b.payload).calls.into_iter().map(|c| c.name).collect();
            let mut calls: Vec<String> = index::index(&b.pattern)
                .calls
                .into_iter()
                .map(|c| c.name)
                .filter(|n| !kept_calls.contains(n))
                .collect();
            calls.sort();
            calls.dedup();
            for name in calls {
                let game = game_functions.get_or_insert_with(|| vanilla_functions(cx.vanilla)).contains(&name);
                if game {
                    rewrites.push(Rewrite { b: b_origin.clone(), file: rec.target_file.clone(), call: Some(name), condition: Vec::new() });
                }
            }
            let kept = index::conditions(&b.payload);
            for c in index::conditions(&b.pattern) {
                let named = c.iter().any(|t| t.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_'));
                if c.len() < 3 || !named || c.iter().any(|t| matches!(*t, "*" | "?")) || kept.contains(&c) {
                    continue;
                }
                let condition = c.iter().map(|t| t.to_string()).collect();
                rewrites.push(Rewrite { b: b_origin.clone(), file: rec.target_file.clone(), call: None, condition });
            }
        }
    }
    // Only what no line of the game's own code does any more is rerouted.
    let mut gone: HashMap<String, bool> = HashMap::new();
    rewrites.retain(|r| {
        let key = r.call.clone().unwrap_or_else(|| r.condition.join(" "));
        *gone.entry(key).or_insert_with(|| match &r.call {
            Some(name) => !game_still_calls(p, cx.vanilla, name),
            None => !game_still_tests(p, cx.vanilla, &r.condition),
        })
    });
    if rewrites.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
    let any_condition = rewrites.iter().any(|r| r.call.is_none());
    for (src, s) in p.sources.iter().enumerate() {
        let lines: Vec<&str> = if any_condition { s.text.lines().collect() } else { Vec::new() };
        let mut hit = |line: usize, r: &Rewrite, what: String, out: &mut Vec<Finding>| {
            let Some(a) = p.origin(src, line) else {
                return;
            };
            if same_mod(a, &r.b) || is_dead(p, src, line) {
                return;
            }
            if !seen.insert((a.patch_id.clone(), r.b.patch_id.clone(), what.clone())) {
                return;
            }
            let b_at = patch_place(p, &r.file, &r.b.patch_id, &r.b.mod_id);
            let (did, does) = match &r.call {
                Some(name) => (
                    format!(
                        "rewrites lines of {} that call `{name}` (now {b_at}), and no line of the game's own code calls it any more",
                        r.file
                    ),
                    format!("calls `{name}` itself"),
                ),
                None => (
                    format!(
                        "rewrites lines of {} that test `{what}` (now {b_at}), and no line of the game's own code tests it any more",
                        r.file
                    ),
                    format!("tests `{what}` itself"),
                ),
            };
            out.push(Finding {
                check: "bypassed_hook",
                name: what,
                severity: "warning",
                file: s.key.clone(),
                a: a.clone(),
                b: Some(r.b.clone()),
                description: format!(
                    "Patch '{}' (from '{}') {did}. Patch '{}' (from '{}') {does} at {}, so '{}''s change does not reach that code. This compares the text only.",
                    r.b.patch_name, r.b.mod_id, a.patch_name, a.mod_id, place(p, src, line), r.b.mod_id
                ),
            });
        };
        for c in &s.index.calls {
            for r in rewrites.iter().filter(|r| r.call.as_deref() == Some(c.name.as_str())) {
                hit(c.line, r, c.name.clone(), &mut out);
            }
        }
        if !any_condition {
            continue;
        }
        for (i, text) in lines.iter().enumerate() {
            let line = i + 1;
            if p.origin(src, line).is_none() {
                continue;
            }
            let toks = index::token_texts(text);
            for r in rewrites.iter().filter(|r| r.call.is_none()) {
                let n = r.condition.len();
                if toks.windows(n).any(|w| w.iter().zip(&r.condition).all(|(a, b)| a == b)) {
                    let cond: Vec<&str> = r.condition.iter().map(String::as_str).collect();
                    hit(line, r, show(&cond), &mut out);
                }
            }
        }
    }
    out
}

type FieldSite<'a> = (usize, usize, &'a Origin, &'a str);
type NameSite<'a> = (usize, usize, &'a Origin, String);

/// Two mods set the same field on a unit, the player or another game table (`dude.pathQueue =`,
/// `player.x =`), and the game's own code never sets it: both use one new field. A note. The
/// table is a parameter or a local that is not the mod's own table literal (`self` is left
/// out), or the global `player`.
fn shared_field(p: &Program, cx: &Ctx<'_>) -> Vec<Finding> {
    // Field -> per mod: its first write (source, line, patch, full name).
    let mut by_field: BTreeMap<&str, BTreeMap<String, FieldSite<'_>>> = BTreeMap::new();
    for (src, s) in p.sources.iter().enumerate() {
        for w in &s.index.field_writes {
            if w.method || w.own_table {
                continue;
            }
            let base = w.name.split('.').next().unwrap_or("");
            if !((w.local && base != "self") || (!w.local && base == "player")) {
                continue;
            }
            let Some(origin) = p.origin(src, w.line) else {
                continue;
            };
            if is_dead(p, src, w.line) {
                continue;
            }
            let field = w.name.rsplit('.').next().unwrap_or("");
            if field.is_empty() {
                continue;
            }
            by_field
                .entry(field)
                .or_default()
                .entry(origin.mod_id.to_lowercase())
                .or_insert((src, w.line, origin, w.name.as_str()));
        }
    }
    let mut out = Vec::new();
    for (field, writers) in by_field.iter().filter(|(_, w)| w.len() > 1) {
        if vanilla_sets(cx.vanilla, field) {
            continue;
        }
        let list: Vec<_> = writers.values().collect();
        for (i, (a_src, a_line, a, a_name)) in list.iter().copied().enumerate() {
            for (b_src, b_line, b, b_name) in list.iter().copied().skip(i + 1) {
                out.push(Finding {
                    check: "shared_field",
                    name: field.to_string(),
                    severity: "info",
                    file: file_of(p, *a_src),
                    a: (*a).clone(),
                    b: Some((*b).clone()),
                    description: format!(
                        "Patch '{}' (from '{}') sets `{a_name}` at {}, and patch '{}' (from '{}') sets `{b_name}` at {}. The game's own code never sets `{field}`, so both mods keep their own data in one field; check that they mean the same by it.",
                        a.patch_name, a.mod_id, place(p, *a_src, *a_line),
                        b.patch_name, b.mod_id, place(p, *b_src, *b_line)
                    ),
                });
            }
        }
    }
    out
}

/// Two mods add a button with one label (a click calls the one global `<label>Button`), or use
/// one menu page number the game does not use. Literal labels and page numbers only.
fn name_collision(p: &Program, cx: &Ctx<'_>) -> Vec<Finding> {
    // Name -> per mod: its first use (source, line, patch, what it is).
    let mut by_name: BTreeMap<String, BTreeMap<String, NameSite<'_>>> = BTreeMap::new();
    let mut pages: Option<HashSet<String>> = None;
    for (src, s) in p.sources.iter().enumerate() {
        let uses = s
            .index
            .buttons
            .iter()
            .map(|b| (format!("{}Button", b.label), b.line, format!("a button labelled \"{}\"", b.label), Some(&b.label)))
            .chain(s.index.pages.iter().map(|g| (format!("menuPage={}", g.page), g.line, format!("menu page {}", g.page), None)));
        for (name, line, what, label) in uses {
            let Some(origin) = p.origin(src, line) else {
                continue;
            };
            if is_dead(p, src, line) {
                continue;
            }
            let game = match label {
                Some(l) => vanilla_literal(cx.vanilla, l),
                None => pages
                    .get_or_insert_with(|| vanilla_pages(cx.vanilla))
                    .contains(name.trim_start_matches("menuPage=")),
            };
            if game {
                continue;
            }
            by_name.entry(name).or_default().entry(origin.mod_id.to_lowercase()).or_insert((src, line, origin, what));
        }
    }
    let mut out = Vec::new();
    for (name, users) in by_name.iter().filter(|(_, u)| u.len() > 1) {
        let list: Vec<_> = users.values().collect();
        for (i, (a_src, a_line, a, what)) in list.iter().enumerate() {
            for (b_src, b_line, b, _) in list.iter().skip(i + 1) {
                let why = if name.starts_with("menuPage=") {
                    "The game has no such page, so both mods draw and handle their own screen on one page number.".to_string()
                } else {
                    format!("A click on either calls the one global `{name}`, so both buttons do the same thing.")
                };
                out.push(Finding {
                    check: "name_collision",
                    name: name.clone(),
                    severity: "warning",
                    file: file_of(p, *a_src),
                    a: (*a).clone(),
                    b: Some((*b).clone()),
                    description: format!(
                        "Patch '{}' (from '{}') uses {what} at {}, and patch '{}' (from '{}') uses it too at {}. {why}",
                        a.patch_name, a.mod_id, place(p, *a_src, *a_line),
                        b.patch_name, b.mod_id, place(p, *b_src, *b_line)
                    ),
                });
            }
        }
    }
    out
}

/// One pattern patch of a mod on a file: its place in the table, its block count and how often
/// it applied.
struct Part<'a> {
    at: usize,
    rec: &'a qm_core::handshake::PatchExecutionRecord,
    net: i64,
    sites: i64,
}

fn blocks(n: i64) -> String {
    if n == 1 { "1 block".to_string() } else { format!("{n} blocks") }
}

fn origin_of(rec: &qm_core::handshake::PatchExecutionRecord) -> Origin {
    Origin { mod_id: rec.mod_id.clone(), patch_id: rec.patch_id.clone(), patch_name: rec.patch_name.clone() }
}

/// The blocks one mod's patches open and close in a file, counted exactly (`function`, `if`,
/// `do`, `repeat` against `end`, `until`; an `at` patch counts against the lines it replaces)
/// and weighed by how often each applied. Patches that open and close each other are paired
/// in the mod's order; when one of a pair did not apply, the file is left unbalanced.
fn block_balance(cx: &Ctx<'_>) -> Vec<Finding> {
    let mut out = Vec::new();
    for file in cx.files.iter().filter(|f| f.target_file.ends_with(".lua")) {
        let trace = cx.traces.get(&file.target_file);
        let mut per_mod: BTreeMap<String, Vec<Part<'_>>> = BTreeMap::new();
        for (ri, rec) in file.records.iter().enumerate().filter(|(_, r)| r.patch_type == "pattern") {
            let Some((at, Patch::Pattern(pp))) = find_patch(cx.table, &rec.patch_id, &rec.mod_id) else {
                continue;
            };
            let window = if pp.position == InsertPosition::At { index::block_delta(&pp.pattern) } else { 0 };
            let net = index::block_delta(&pp.payload) - window;
            if net == 0 {
                continue;
            }
            let sites = if rec.status == PatchOutcomeStatus::Applied {
                let n = trace
                    .map(|t| t.edits.iter().filter(|e| e.record == ri).map(|e| e.site).collect::<BTreeSet<_>>().len())
                    .unwrap_or(0);
                i64::try_from(n.max(1)).unwrap_or(1)
            } else {
                0
            };
            per_mod.entry(rec.mod_id.to_lowercase()).or_default().push(Part { at, rec, net, sites });
        }
        for parts in per_mod.values_mut() {
            parts.sort_by_key(|x| x.at);
            // Groups: from a patch that opens or closes blocks to the one that brings the
            // count back to zero.
            let mut groups: Vec<Vec<&Part<'_>>> = Vec::new();
            let mut open = 0i64;
            for x in parts.iter() {
                if open == 0 {
                    groups.push(Vec::new());
                }
                if let Some(g) = groups.last_mut() {
                    g.push(x);
                }
                open += x.net;
            }
            for g in groups {
                let applied: i64 = g.iter().map(|x| x.net * x.sites).sum();
                if applied == 0 {
                    continue;
                }
                let closed = g.iter().map(|x| x.net).sum::<i64>() == 0;
                let most = g.iter().map(|x| x.sites).max().unwrap_or(0);
                let n = applied.abs();
                let left = if applied > 0 {
                    format!("{} never closed", blocks(n))
                } else if n == 1 {
                    "1 extra `end`".to_string()
                } else {
                    format!("{n} extra `end`s")
                };
                let present = g.iter().find(|x| x.sites == most && most > 0);
                let missing = g.iter().find(|x| x.sites < most);
                let Some(a) = present.or_else(|| g.iter().find(|x| x.sites > 0)) else {
                    continue;
                };
                let (b, description) = match (closed, missing) {
                    (true, Some(b)) => {
                        let why = match b.rec.status {
                            PatchOutcomeStatus::SkippedDisabled => "is turned off",
                            PatchOutcomeStatus::Applied => "applied at fewer places",
                            _ => "did not apply",
                        };
                        let (verb, partner) = if a.net > 0 { ("opens", "closes") } else { ("closes", "opens") };
                        (
                            Some(origin_of(b.rec)),
                            format!(
                                "Patch '{}' (from '{}') {verb} a block in {} that patch '{}' of the same mod {partner}, but '{}' {why}. The file is left with {left}, so it does not compile.",
                                a.rec.patch_name, a.rec.mod_id, file.target_file, b.rec.patch_name, b.rec.patch_name
                            ),
                        )
                    }
                    _ => {
                        let what = if applied > 0 {
                            format!("leaves {} open", blocks(n))
                        } else {
                            format!("closes {} it does not open", blocks(n))
                        };
                        (
                            None,
                            format!(
                                "Patch '{}' (from '{}') {what} in {}, and no other patch of '{}' there makes up for it. Unless the game's own lines do, the file is left with {left} and does not compile.",
                                a.rec.patch_name, a.rec.mod_id, file.target_file, a.rec.mod_id
                            ),
                        )
                    }
                };
                out.push(Finding {
                    check: "block_balance",
                    name: file.target_file.clone(),
                    severity: "warning",
                    file: file.target_file.clone(),
                    a: origin_of(a.rec),
                    b,
                    description,
                });
            }
        }
    }
    out
}
