//! Placement check: does a patch's code still land where its author put it?
//!
//! The author tested the patch with the rest of its mod on the game's own file. For each patch
//! whose lines share a file with other mods, its mod is applied alone to the game's file and the
//! block depth and enclosing function of its added code there are compared with the same code in
//! the full result. A different function or depth means another mod moved or wrapped the code (for
//! example it now runs only inside another mod's `if`). Added code that now starts inside a
//! comment or long string, and an anchor line that is now only a comment, are reported too.
//!
//! Both runs patch the text the game loads (QuarterMaster's edits included, D1); the line
//! numbers in the messages are the file's own.

use std::collections::{BTreeSet, HashMap, HashSet};

use qm_core::handshake::{PatchExecutionRecord, PatchOutcomeStatus};
use qm_core::lua_scan::{scan, Scan};
use qm_core::patch::origin::{FileTrace, LineOrigin};
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, Patch};

use crate::conflict::{DetectedConflict, SimulationReport};
use crate::semantic::index::{free_names, index_watching, Binding, NameUse, SourceIndex};
use crate::semantic::{Finding, Origin};

/// Whether code can go at one edge of another patch's change, and why not (§9.2). An edge
/// inside a block the change leaves open is not offered: the code would run only when that
/// block runs.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EdgeCheck {
    pub allowed: bool,
    pub reason: Option<String>,
}

impl EdgeCheck {
    fn ok() -> Self {
        EdgeCheck {
            allowed: true,
            reason: None,
        }
    }

    fn no(reason: String) -> Self {
        EdgeCheck {
            allowed: false,
            reason: Some(reason),
        }
    }
}

/// The condition of an `if`/`elseif`/`while` line, without its keywords and parentheses.
fn condition_of(line: &str) -> Option<String> {
    let t = line.trim();
    let rest = ["elseif", "if", "while"]
        .iter()
        .find_map(|k| t.strip_prefix(k).filter(|r| r.starts_with([' ', '\t', '('])))?;
    let rest = rest
        .trim()
        .strip_suffix("then")
        .or_else(|| rest.trim().strip_suffix("do"))?
        .trim();
    let rest = match rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        Some(inner) if !inner.contains(['(', ')']) => inner.trim(),
        _ => rest,
    };
    (!rest.is_empty()).then(|| rest.to_string())
}

/// The start and end edges of a change whose lines are `lines` (one site of another patch,
/// top to bottom). Code at an edge is placed only when the change closes every block it opens
/// (the engine's edge gate, D10); the end edge of a change that leaves an `if` open says when
/// the code would run.
pub fn edge_checks(lines: &[&str], b_name: &str) -> (EdgeCheck, EdgeCheck) {
    let text = lines.join("\n");
    let s = scan(&text);
    if s.balanced {
        return (EdgeCheck::ok(), EdgeCheck::ok());
    }
    // The state where a line after the change would start.
    let probe = scan(&format!("{text}\nx"));
    let after = probe.lines.last().copied().unwrap_or_default();
    let end = if after.depth > 0 {
        let opener = lines.get(after.opener.wrapping_sub(1)).copied().unwrap_or_default();
        match condition_of(opener) {
            Some(cond) => EdgeCheck::no(format!("would run only when {cond}")),
            None => EdgeCheck::no(format!("would land inside a block {b_name} leaves open")),
        }
    } else {
        EdgeCheck::no(format!(
            "would land outside the block around {b_name}'s change"
        ))
    };
    let start = EdgeCheck::no(format!(
        "{b_name} does not close every block it opens, so its edges cannot be used"
    ));
    (start, end)
}

/// `mod_id`'s patches alone on `target`, with the table's mods and vars. Clones keep their
/// `PatchExtra`.
pub(crate) fn mod_alone(table: &PatchTable, mod_id: &str, target: &str) -> PatchTable {
    PatchTable {
        targets: HashSet::from([target.to_string()]),
        patches: table
            .patches
            .iter()
            .filter(|(p, _, _)| p.mod_id() == Some(mod_id))
            .cloned()
            .collect(),
        vars: table.vars.clone(),
        mods: table.mods.clone(),
        ..PatchTable::default()
    }
}

/// `vanilla` is the prepared text and `trace` the full run over it. `to_file` maps a 1-based
/// line of the patched text to the file line shown to the user.
pub(crate) fn check_placement(
    table: &PatchTable,
    target: &str,
    vanilla: &str,
    trace: &FileTrace,
    records: &[PatchExecutionRecord],
    to_file: &dyn Fn(usize) -> usize,
) -> Vec<DetectedConflict> {
    let mut out = Vec::new();
    let mut full: Option<Scan> = None;
    for (idx, rec) in records.iter().enumerate() {
        if rec.status != PatchOutcomeStatus::Applied || !matches!(rec.patch_type.as_str(), "pattern" | "regex") {
            continue;
        }
        let runs = trace.inserted_runs_of_record(idx);
        if runs.is_empty() {
            continue;
        }
        // Only lines near other mods' output can have moved.
        let others = trace
            .edits
            .iter()
            .any(|e| e.record != idx && records.get(e.record).is_some_and(|r| r.mod_id != rec.mod_id));
        if !others {
            continue;
        }
        let Some(patch) = table
            .patches
            .iter()
            .map(|(p, _, _)| p)
            .find(|p| p.id() == Some(rec.patch_id.as_str()) && p.mod_id() == Some(rec.mod_id.as_str()))
        else {
            continue;
        };
        // The author tested the patch with the rest of its mod: apply the mod alone.
        let solo = mod_alone(table, &rec.mod_id, target);
        let Ok((_, _, solo_records, solo_trace)) = solo.apply_patches_traced(target, vanilla) else {
            continue;
        };
        let Some(solo_idx) = solo_records.iter().position(|r| r.patch_id == rec.patch_id) else {
            continue;
        };
        if solo_records[solo_idx].status != PatchOutcomeStatus::Applied {
            // Anchored on another mod's code: there is no solo placement to compare with.
            continue;
        }
        let solo_runs = solo_trace.inserted_runs_of_record(solo_idx);
        if solo_runs.len() != runs.len() {
            continue;
        }
        let full = full.get_or_insert_with(|| scan(&trace.text));
        let alone = scan(&solo_trace.text);
        let position = match patch {
            Patch::Pattern(p) => p.position,
            Patch::Regex(r) => r.position,
            _ => continue,
        };
        if let Some(c) = compare(target, rec, trace, full, &alone, &runs, &solo_runs, position, records, to_file) {
            out.push(c);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn compare(
    target: &str,
    rec: &PatchExecutionRecord,
    trace: &FileTrace,
    full: &Scan,
    alone: &Scan,
    runs: &[(usize, usize)],
    solo_runs: &[(usize, usize)],
    position: InsertPosition,
    records: &[PatchExecutionRecord],
    to_file: &dyn Fn(usize) -> usize,
) -> Option<DetectedConflict> {
    for (&(start, end), &(solo_start, solo_end)) in runs.iter().zip(solo_runs) {
        let (f, a) = (full.lines.get(start - 1)?, alone.lines.get(solo_start - 1)?);
        // The mod whose line opened the block the code now sits in, if a mod wrote it.
        let opener_mod = || {
            trace
                .edit_at(f.opener)
                .map(|e| e.record)
                .and_then(|r| records.get(r))
                .filter(|r| r.mod_id != rec.mod_id)
        };
        let (kind, what) = if f.starts_in_long && !a.starts_in_long {
            ("placement_in_comment", format!("now starts inside a comment or long string at line {}, so it never runs", to_file(start)))
        } else if full.function_name(start) != alone.function_name(solo_start) {
            (
                "placement_changed",
                format!(
                    "now lands in function '{}' at line {} instead of '{}'",
                    full.function_name(start),
                    to_file(start),
                    alone.function_name(solo_start)
                ),
            )
        } else if f.depth != a.depth {
            let how = if f.depth > a.depth { "deeper inside a block" } else { "outside a block it was written for" };
            (
                "placement_changed",
                format!("now lands {how} at line {} (block depth {} instead of {})", to_file(start), f.depth, a.depth),
            )
        } else {
            // The line the anchor matched: just before the added code for `after`, just after
            // it for `before`. In the full run, other mods' code stacked at the same line sits
            // in between and is skipped (stacks are not conflicts, D28).
            let other_mods_line = |l: usize| {
                matches!(trace.lines.get(l.wrapping_sub(1)), Some(LineOrigin::Inserted { .. }))
                    && trace
                        .edit_at(l)
                        .and_then(|e| records.get(e.record))
                        .is_some_and(|r| r.mod_id != rec.mod_id)
            };
            let (anchor, solo_anchor) = match position {
                InsertPosition::After => {
                    let mut l = start.checked_sub(1);
                    while let Some(n) = l.filter(|&n| n > 0 && other_mods_line(n)) {
                        l = n.checked_sub(1);
                    }
                    (l, solo_start.checked_sub(1))
                }
                InsertPosition::Before => {
                    let mut l = end + 1;
                    while l <= trace.lines.len() && other_mods_line(l) {
                        l += 1;
                    }
                    (Some(l), Some(solo_end + 1))
                }
                InsertPosition::At => (None, None),
            };
            let is_code = |s: &Scan, l: Option<usize>| l.and_then(|l| s.lines.get(l.wrapping_sub(1))).map(|i| i.code && !i.starts_in_long);
            match (is_code(full, anchor), is_code(alone, solo_anchor)) {
                (Some(false), Some(true)) => (
                    "anchor_in_comment",
                    format!("matched its anchor inside a comment at line {}", anchor.map_or(0, to_file)),
                ),
                _ => continue,
            }
        };
        let other = opener_mod();
        return Some(DetectedConflict {
            id: format!("{kind}:{target}:{}", rec.patch_id),
            severity: "warning".to_string(),
            kind: kind.to_string(),
            target_file: target.to_string(),
            mod_a_id: rec.mod_id.clone(),
            mod_b_id: other.map(|o| o.mod_id.clone()),
            patch_a_id: rec.patch_id.clone(),
            patch_b_id: other.map(|o| o.patch_id.clone()),
            description: format!(
                "Patch '{}' (from '{}') {what}{} in '{target}'. Check that it still does what it should.",
                rec.patch_name,
                rec.mod_id,
                other.map(|o| format!(", inside code from '{}'", o.mod_id)).unwrap_or_default(),
            ),
            suggestions: Vec::new(),
            consumer: None,
            sites_lost: None,
            same_line: None,
        });
    }
    None
}

/// What says where a patch's code goes: its definition (without `disabled`, priority and
/// order lists) and the profile's or compat mods' anchors.
fn anchor_key(p: &Patch) -> Option<String> {
    let mut v = serde_json::to_value(p).ok()?;
    if let Some(o) = v.as_object_mut() {
        for inner in o.values_mut() {
            if let Some(i) = inner.as_object_mut() {
                for k in ["disabled", "priority", "before", "after", "requires", "description", "name"] {
                    i.remove(k);
                }
            }
        }
    }
    let e = p.extra();
    Some(format!("{v}|{:?}|{:?}|{}|{:?}", e.rungs, e.sites, e.replace_author_anchors, e.exact_lines))
}

/// 1-based runs of lines `patch_id` (of `mod_id`) added in `trace`.
fn added_runs(trace: &FileTrace, patch_id: &str, mod_id: &str) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, o) in trace.lines.iter().enumerate() {
        let mine = matches!(o, LineOrigin::Inserted { edit } if trace.edits.get(*edit)
            .is_some_and(|e| e.patch_id == patch_id && e.mod_id.eq_ignore_ascii_case(mod_id)));
        if !mine {
            continue;
        }
        let n = i + 1;
        match out.last_mut() {
            Some(last) if last.1 + 1 == n => last.1 = n,
            _ => out.push((n, n)),
        }
    }
    out
}

/// Where code was or is: a text, its index, the names its lines use and the runs.
struct Placed {
    index: SourceIndex,
    uses: Vec<NameUse>,
    runs: Vec<(usize, usize)>,
}

impl Placed {
    fn new(text: &str, runs: Vec<(usize, usize)>) -> Placed {
        let (index, uses) = index_watching(text, &runs);
        Placed { index, uses, runs }
    }

    fn function(&self, line: usize) -> String {
        let f = self.index.line(line).map(|l| l.function).unwrap_or(0);
        self.index.function_path(f)
    }
}

fn shown(path: &str) -> String {
    if path == "<main>" {
        "the file's main chunk".to_string()
    } else {
        format!("`{path}`")
    }
}

fn binding_text(name: &str, b: &Binding) -> String {
    match b {
        Binding::Global => format!("the global `{name}`"),
        Binding::Local { function, decl, .. } => {
            format!("the local `{name}` of {} (`{decl}`)", shown(function))
        }
    }
}

/// Code an anchor fix moves (§6.9, Task 37c), for its preview: each patch whose anchors the
/// change sets (an Anchor fix, or a new adjusted copy of a patch) is compared with where its
/// code was before. Before is the current simulation when the patch applied there, else its
/// mod alone on the game's file (where its author put it). `moved_scope`: the code now sits
/// in another function. `free_name_changed`: a name the code uses but does not declare now
/// resolves to another local, or to a global instead of a local (or the other way). Scope is
/// approximate (`semantic::index`). Silences from mod.json `overrides` apply.
pub fn moved_code(
    cur_table: &PatchTable,
    cur: &SimulationReport,
    next_table: &PatchTable,
    next: &SimulationReport,
    vanilla: &HashMap<String, String>,
) -> Vec<DetectedConflict> {
    let find = |t: &'_ PatchTable, id: &str, m: Option<&str>| -> Option<Patch> {
        t.patches
            .iter()
            .map(|(p, _, _)| p)
            .find(|p| p.id() == Some(id) && m.is_none_or(|m| p.mod_id().is_some_and(|x| x.eq_ignore_ascii_case(m))))
            .cloned()
    };
    // (patch id now, its mod, the patch it was).
    let mut moved: Vec<(String, String, Patch)> = Vec::new();
    for (p, _, _) in &next_table.patches {
        if p.is_disabled() || !matches!(p, Patch::Pattern(_) | Patch::Regex(_)) {
            continue;
        }
        let (Some(id), Some(m)) = (p.id(), p.mod_id()) else {
            continue;
        };
        let before = match find(cur_table, id, Some(m)) {
            Some(c) if anchor_key(&c) == anchor_key(p) => continue,
            Some(c) => c,
            None => match p.extra().replaces.as_deref().filter(|_| p.extra().adjusted_copy) {
                Some(base) => match find(cur_table, base, None) {
                    Some(b) => b,
                    None => continue,
                },
                None => continue,
            },
        };
        moved.push((id.to_string(), m.to_string(), before));
    }
    let mut out = Vec::new();
    for (id, m, before) in &moved {
        let (Some(b_id), Some(b_mod)) = (before.id(), before.mod_id()) else {
            continue;
        };
        for (target, trace) in next.traces.iter().filter(|(k, _)| k.ends_with(".lua")) {
            let after_runs = added_runs(trace, id, m);
            let Some(&(first, _)) = after_runs.first() else {
                continue;
            };
            let Some(edit) = trace.edit_at(first) else {
                continue;
            };
            let a = Origin { mod_id: edit.mod_id.clone(), patch_id: edit.patch_id.clone(), patch_name: edit.patch_name.clone() };
            // Where the code was: the current run, else the mod alone on the game's file.
            let was = match cur.traces.get(target) {
                Some(t) if !added_runs(t, b_id, b_mod).is_empty() => Some(Placed::new(&t.text, added_runs(t, b_id, b_mod))),
                _ => vanilla.get(target).and_then(|src| {
                    let buf = crate::region::prepared_original(cur_table, target, src);
                    let solo = mod_alone(cur_table, b_mod, target);
                    let (_, _, _, t) = solo.apply_patches_traced(target, &buf.text).ok()?;
                    let runs = added_runs(&t, b_id, b_mod);
                    (!runs.is_empty()).then(|| Placed::new(&t.text, runs))
                }),
            };
            let Some(was) = was else {
                continue;
            };
            let now = Placed::new(&trace.text, after_runs);
            let place = |line: usize| format!("{target} line {line}");
            let mut found: Vec<Finding> = Vec::new();
            let was_in: BTreeSet<String> = was.runs.iter().map(|&(s, _)| was.function(s)).collect();
            if let Some(&(s, _)) = now.runs.iter().find(|&&(s, _)| !was_in.contains(&now.function(s))) {
                let to = now.function(s);
                let from = was_in.iter().next().cloned().unwrap_or_default();
                found.push(Finding {
                    check: "moved_scope",
                    name: to.rsplit(" > ").next().unwrap_or(&to).to_string(),
                    severity: "warning",
                    file: target.clone(),
                    a: a.clone(),
                    b: None,
                    description: format!(
                        "With this change, patch '{}' (from '{}') adds its code in {} at {}; before, it was in {}. Code in another function runs at another time and sees other locals, so check that it still does what it should.",
                        a.patch_name, a.mod_id, shown(&to), place(s), shown(&from)
                    ),
                });
            }
            for (i, &(s, e)) in now.runs.iter().enumerate() {
                let Some(&(ws, we)) = was.runs.get(i).or(was.runs.first()) else {
                    continue;
                };
                let old: HashMap<&str, &NameUse> =
                    free_names(&was.uses, ws, we).into_iter().map(|u| (u.name.as_str(), u)).collect();
                for u in free_names(&now.uses, s, e) {
                    let Some(o) = old.get(u.name.as_str()) else {
                        continue;
                    };
                    if o.binding.same(&u.binding) || found.iter().any(|f| f.name == u.name) {
                        continue;
                    }
                    found.push(Finding {
                        check: "free_name_changed",
                        name: u.name.clone(),
                        severity: "warning",
                        file: target.clone(),
                        a: a.clone(),
                        b: None,
                        description: format!(
                            "With this change, `{}` in patch '{}' (from '{}') at {} means {}; before, it meant {}. The code reads or sets another variable than its author wrote it for. Scope is read from the text, so this can be wrong for unusual code.",
                            u.name, a.patch_name, a.mod_id, place(u.line),
                            binding_text(&u.name, &u.binding), binding_text(&o.name, &o.binding)
                        ),
                    });
                }
            }
            out.extend(found.into_iter().filter(|f| !crate::semantic::silenced(next_table, f)).map(Finding::into_conflict));
        }
    }
    out
}

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;
