//! N10 semantic checks (D69, plan 2e Phase D §6.9): facts about the patched game files and
//! the mods' modules, in the order the game runs them, and checks on them. Findings are
//! warnings or notes with a "why" text; none blocks a launch. A mod silences a finding it
//! checked with mod.json `overrides = ["<check>:<name>"]`.
//!
//! Run order: `conf.lua`, then `main.lua`'s main chunk in file order, a required file's main
//! chunk where its top-level `require` is (modules before game files of the same name), then
//! `love.load`, then everything else at call time. Two places that both run at call time have
//! no known order. It runs in the launcher's simulation only, never in the DLL.

mod checks;
pub mod index;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use qm_core::patch::order::OrderResult;
use qm_core::patch::origin::FileTrace;
use qm_core::patch::table::PatchTable;
use qm_core::prepare::canonical_path;

use crate::conflict::{DetectedConflict, SimulatedFileResult};
use index::SourceIndex;

pub use checks::{Ctx, Finding};

/// The id prefix of every semantic finding.
pub const ID_PREFIX: &str = "semantic:";

/// The checks of Tasks 37a to 37c. `moved_scope` and `free_name_changed` run on previews of
/// anchor fixes (`placement::moved_code`), the rest in every simulation.
pub const CHECKS: [&str; 14] = [
    "capture_before_define",
    "redefined_global",
    "dead_code",
    "early_return_shadow",
    "table_literal_reset",
    "shared_file_writer",
    "key_collision",
    "double_hook",
    "bypassed_hook",
    "shared_field",
    "name_collision",
    "block_balance",
    "moved_scope",
    "free_name_changed",
];

/// A finding of these checks (never a launch blocker; not a reason to refuse a fix).
pub fn is_semantic(c: &DetectedConflict) -> bool {
    c.id.starts_with(ID_PREFIX)
}

/// The patch that wrote a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub mod_id: String,
    pub patch_id: String,
    pub patch_name: String,
}

/// One Lua source: a game file as patched, or a mod's module.
pub struct Source {
    /// The game file key, or the module name.
    pub key: String,
    pub module: bool,
    pub text: String,
    pub index: SourceIndex,
    /// Per line, the patch that wrote it (index into `Program::origins`), or none for the
    /// game's own line.
    origins: Vec<Option<usize>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Main,
    Load,
    Call,
}

/// When a statement runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAt {
    /// In a main chunk at startup, in this order.
    Main(u32),
    /// In `love.load`, in this order.
    Load(u32),
    /// When a function is called.
    Call,
    /// In a file the game never loads at startup.
    Unknown,
}

impl RunAt {
    /// Whether `self` runs before `other`; None when the order is not known.
    pub fn before(self, other: RunAt) -> Option<bool> {
        let key = |r: RunAt| match r {
            RunAt::Main(s) | RunAt::Load(s) => Some((0u8, s)),
            RunAt::Call => Some((1, 0)),
            RunAt::Unknown => None,
        };
        let (a, b) = (key(self)?, key(other)?);
        if a.0 == 1 && b.0 == 1 {
            return None;
        }
        Some(a < b)
    }
}

/// Every source with its index, and when each line runs.
pub struct Program {
    pub sources: Vec<Source>,
    pub origins: Vec<Origin>,
    by_key: HashMap<String, usize>,
    /// Lowercase vanilla key to the key.
    vanilla_keys: HashMap<String, String>,
    modules: HashMap<String, usize>,
    /// Modules evaluated right before a game file loads: (file, module source).
    load_now: Vec<(String, usize)>,
    phase: Vec<Option<Phase>>,
    seq: Vec<Vec<u32>>,
    load_seq: Vec<HashMap<usize, u32>>,
    next: u32,
}

impl Program {
    fn add(&mut self, key: String, module: bool, text: String, origins: Vec<Option<usize>>) -> usize {
        let index = index::index(&text);
        self.sources.push(Source { key: key.clone(), module, text, index, origins });
        self.phase.push(None);
        self.seq.push(Vec::new());
        self.load_seq.push(HashMap::new());
        let i = self.sources.len() - 1;
        if !module {
            self.by_key.insert(key, i);
        }
        i
    }

    fn origin_id(&mut self, interned: &mut HashMap<(String, String), usize>, o: Origin) -> usize {
        let key = (o.mod_id.clone(), o.patch_id.clone());
        *interned.entry(key).or_insert_with(|| {
            self.origins.push(o);
            self.origins.len() - 1
        })
    }

    /// The game file `key` (any case), indexed on first use.
    fn file(&mut self, key: &str, vanilla: &HashMap<String, String>) -> Option<usize> {
        if let Some(&i) = self.by_key.get(key) {
            return Some(i);
        }
        let real = self.vanilla_keys.get(&key.to_lowercase())?.clone();
        if let Some(&i) = self.by_key.get(&real) {
            return Some(i);
        }
        let text = vanilla.get(&real)?.clone();
        let lines = text.lines().count();
        Some(self.add(real, false, text, vec![None; lines]))
    }

    /// What `require(name)` loads: a module of that name, else the game file.
    fn resolve(&mut self, name: &str, vanilla: &HashMap<String, String>) -> Option<usize> {
        if let Some(&i) = self.modules.get(name) {
            return Some(i);
        }
        let path = if name.contains('/') { name.to_string() } else { name.replace('.', "/") };
        let path = canonical_path(&path);
        self.file(&format!("{path}.lua"), vanilla)
            .or_else(|| self.file(&format!("{path}/init.lua"), vanilla))
    }

    /// Runs `src`'s main chunk now, with the files it requires at the top level.
    fn walk(&mut self, src: usize, phase: Phase, vanilla: &HashMap<String, String>) {
        if self.phase.get(src).copied().flatten().is_some() {
            return;
        }
        if let Some(p) = self.phase.get_mut(src) {
            *p = Some(phase);
        }
        let Some(s) = self.sources.get(src) else {
            return;
        };
        if !s.module {
            let key = s.key.clone();
            let before: Vec<usize> =
                self.load_now.iter().filter(|(f, _)| *f == key).map(|(_, m)| *m).collect();
            for m in before {
                self.walk(m, phase, vanilla);
            }
        }
        let Some(s) = self.sources.get(src) else {
            return;
        };
        let lines = s.index.lines.len();
        let requires: Vec<(usize, String)> = s
            .index
            .requires
            .iter()
            .filter(|r| r.function == 0)
            .map(|r| (r.line, r.name.clone()))
            .collect();
        let mut seqs = Vec::with_capacity(lines);
        let mut req = requires.iter().peekable();
        for line in 1..=lines {
            seqs.push(self.next);
            self.next = self.next.saturating_add(1);
            while let Some((_, name)) = req.next_if(|(l, _)| *l <= line) {
                if let Some(t) = self.resolve(name, vanilla) {
                    self.walk(t, phase, vanilla);
                }
            }
        }
        if let Some(q) = self.seq.get_mut(src) {
            *q = seqs;
        }
    }

    /// `love.load` bodies of the files loaded at startup, in load order.
    fn walk_load(&mut self, vanilla: &HashMap<String, String>) {
        let started: Vec<usize> = (0..self.sources.len())
            .filter(|&i| self.phase.get(i).copied().flatten() == Some(Phase::Main))
            .collect();
        for src in started {
            let Some(s) = self.sources.get(src) else {
                continue;
            };
            let bodies: Vec<(usize, usize, usize)> = s
                .index
                .functions
                .iter()
                .enumerate()
                .filter(|(i, f)| *i != 0 && f.parent == 0 && f.name == "love.load")
                .map(|(i, f)| (i, f.line, f.end_line))
                .collect();
            for (f, start, end) in bodies {
                let requires: Vec<(usize, String)> = self
                    .sources
                    .get(src)
                    .map(|s| {
                        s.index
                            .requires
                            .iter()
                            .filter(|r| r.function == f)
                            .map(|r| (r.line, r.name.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                for line in start..=end {
                    let n = self.next;
                    self.next = self.next.saturating_add(1);
                    if let Some(m) = self.load_seq.get_mut(src) {
                        m.insert(line, n);
                    }
                    for (_, name) in requires.iter().filter(|(l, _)| *l == line) {
                        if let Some(t) = self.resolve(name, vanilla) {
                            self.walk(t, Phase::Load, vanilla);
                        }
                    }
                }
            }
        }
    }

    /// When the statement at `line` of `src`, in function `function`, runs.
    pub fn run_at(&self, src: usize, line: usize, function: usize) -> RunAt {
        let phase = self.phase.get(src).copied().flatten();
        if function == 0 {
            let seq = self.seq.get(src).and_then(|q| q.get(line.wrapping_sub(1))).copied();
            return match (phase, seq) {
                (Some(Phase::Main), Some(s)) => RunAt::Main(s),
                (Some(Phase::Load), Some(s)) => RunAt::Load(s),
                (Some(Phase::Call), _) => RunAt::Call,
                _ => RunAt::Unknown,
            };
        }
        let in_load = self
            .sources
            .get(src)
            .and_then(|s| s.index.functions.get(function))
            .is_some_and(|f| f.parent == 0 && f.name == "love.load");
        match self.load_seq.get(src).and_then(|m| m.get(&line)) {
            Some(&s) if in_load => RunAt::Load(s),
            _ if phase.is_none() => RunAt::Unknown,
            _ => RunAt::Call,
        }
    }

    /// The patch that wrote 1-based `line` of `src`.
    pub fn origin(&self, src: usize, line: usize) -> Option<&Origin> {
        let o = (*self.sources.get(src)?.origins.get(line.checked_sub(1)?)?)?;
        self.origins.get(o)
    }

    /// The name a finding in function `f` of `src` uses: the function, or the file for the
    /// main chunk and anonymous functions.
    pub fn scope_name(&self, src: usize, f: usize) -> String {
        let Some(s) = self.sources.get(src) else {
            return String::new();
        };
        let name = s.index.function_name(f);
        if f == 0 || name.starts_with('<') {
            s.key.clone()
        } else {
            name.to_string()
        }
    }
}

/// Indexes the patched files (`traces`, as the file shows them), the enabled modules and
/// the game files they require, and works out when each line runs.
pub fn build(
    table: &PatchTable,
    order: &OrderResult,
    vanilla: &HashMap<String, String>,
    traces: &BTreeMap<String, Arc<FileTrace>>,
) -> Program {
    let mut p = Program {
        sources: Vec::new(),
        origins: Vec::new(),
        by_key: HashMap::new(),
        vanilla_keys: vanilla.keys().map(|k| (k.to_lowercase(), k.clone())).collect(),
        modules: HashMap::new(),
        load_now: Vec::new(),
        phase: Vec::new(),
        seq: Vec::new(),
        load_seq: Vec::new(),
        next: 0,
    };
    let mut interned: HashMap<(String, String), usize> = HashMap::new();
    for (key, trace) in traces.iter().filter(|(k, _)| k.ends_with(".lua")) {
        let mut origins = Vec::with_capacity(trace.lines.len());
        for line in &trace.lines {
            let o = line
                .is_patch_output()
                .then(|| line.edit().and_then(|e| trace.edits.get(e)))
                .flatten()
                .map(|e| Origin {
                    mod_id: e.mod_id.clone(),
                    patch_id: e.patch_id.clone(),
                    patch_name: e.patch_name.clone(),
                });
            origins.push(o.map(|o| p.origin_id(&mut interned, o)));
        }
        p.add(key.clone(), false, trace.text.clone(), origins);
    }
    // The module the DLL preloads last wins its name (G7).
    let mut winners: BTreeMap<String, crate::diagnostics_extra::ModuleUse<'_>> = BTreeMap::new();
    for u in crate::diagnostics_extra::module_uses(table, order) {
        winners.insert(u.name.clone(), u);
    }
    for (name, u) in winners {
        let o = p.origin_id(
            &mut interned,
            Origin { mod_id: u.mod_id.clone(), patch_id: u.patch_id.clone(), patch_name: name.clone() },
        );
        let lines = u.module.content.lines().count();
        let i = p.add(name.clone(), true, u.module.content.clone(), vec![Some(o); lines]);
        p.modules.insert(name, i);
        if u.module.load_now {
            if let Some(before) = &u.module.before {
                p.load_now.push((canonical_path(before), i));
            }
        }
    }
    for root in ["conf.lua", "main.lua"] {
        if let Some(i) = p.file(root, vanilla) {
            p.walk(i, Phase::Main, vanilla);
        }
    }
    p.walk_load(vanilla);
    // A file required only from a function runs at call time.
    let n = p.sources.len();
    for src in 0..n {
        let names: Vec<String> = p
            .sources
            .get(src)
            .map(|s| s.index.requires.iter().map(|r| r.name.clone()).collect())
            .unwrap_or_default();
        for name in names {
            if let Some(t) = p.resolve(&name, vanilla) {
                if let Some(ph) = p.phase.get_mut(t) {
                    ph.get_or_insert(Phase::Call);
                }
            }
        }
    }
    p
}

/// Whether a mod named by `f` silences it with mod.json `overrides` (`<check>:<name>` or
/// `<check>:*`).
pub fn silenced(table: &PatchTable, f: &Finding) -> bool {
    let exact = format!("{}:{}", f.check, f.name);
    let any = format!("{}:*", f.check);
    std::iter::once(&f.a)
        .chain(f.b.as_ref())
        .filter_map(|o| table.mods.iter().find(|m| m.id.eq_ignore_ascii_case(&o.mod_id)))
        .any(|m| m.overrides.iter().map(|s| s.trim()).any(|s| s == exact || s == any))
}

/// What one pass over the indexed program gives.
pub struct Analysis {
    /// The semantic findings, as conflicts, minus the ones a mod silences.
    pub conflicts: Vec<DetectedConflict>,
    /// The save fields the mods write (Phase E D84).
    pub save_fields: crate::save_fields::SaveFieldRegistry,
}

/// The semantic findings of one simulation, as conflicts, minus the ones a mod silences.
/// `files` are the simulation's per-file results (records), `traces` their texts.
pub fn findings(
    table: &PatchTable,
    order: &OrderResult,
    vanilla: &HashMap<String, String>,
    traces: &BTreeMap<String, Arc<FileTrace>>,
    files: &[SimulatedFileResult],
) -> Vec<DetectedConflict> {
    analyze(table, order, vanilla, traces, files).conflicts
}

/// The findings and the save field registry, from one index of the program.
pub fn analyze(
    table: &PatchTable,
    order: &OrderResult,
    vanilla: &HashMap<String, String>,
    traces: &BTreeMap<String, Arc<FileTrace>>,
    files: &[SimulatedFileResult],
) -> Analysis {
    let program = build(table, order, vanilla, traces);
    let save_fields = crate::save_fields::SaveFieldRegistry::from_owners(
        checks::save_field_owners(&program, vanilla),
    );
    let cx = Ctx { table, files, vanilla, traces };
    let mut out: BTreeMap<String, DetectedConflict> = BTreeMap::new();
    for f in checks::run(&program, &cx) {
        if silenced(table, &f) {
            continue;
        }
        let c = f.into_conflict();
        out.entry(c.id.clone()).or_insert(c);
    }
    Analysis {
        conflicts: out.into_values().collect(),
        save_fields,
    }
}

#[cfg(test)]
mod tests;
