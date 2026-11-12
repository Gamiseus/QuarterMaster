//! Parity between the engine's entry points: for the same table, file and text, every entry
//! point must give the same patched text and the same records.
//!
//! Used by the golden corpus (`qm-core/tests/golden.rs`) and the real-mods run
//! (`qm-manager/tests/real_mods.rs`). `run_entry_points` patches the file as it is.
//! `check_dll_parity` compares the DLL's `dll_transform` (plan 2e §4.1), once per name from
//! `dll_chunk_names`, with the simulation: both patch the prepared text (D1).
//! `check_linemap` compares the apply's `LineMap` with the journal replay (`origin::trace`).
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use crate::handshake::{LaunchConfig, PatchExecutionRecord, PatchOutcomeStatus};
use crate::patch::linemap::{
    ChainItem, LineKind, LineMap, Side, NO_BLOCK, NO_INDEX, NO_PATCH, NO_SITE,
};
use crate::patch::origin::{self, FileTrace, LineOrigin};
use crate::patch::table::{PatchTable, Prepared};
use crate::prepare::{dll_transform, simulate_buffer, DllOutcome};

/// What a golden pins for one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordKey {
    pub patch_id: String,
    pub status: PatchOutcomeStatus,
    pub matched_count: usize,
}

impl RecordKey {
    pub fn of(rec: &PatchExecutionRecord) -> Self {
        Self {
            patch_id: rec.patch_id.clone(),
            status: rec.status,
            matched_count: rec.matched_count,
        }
    }
}

/// The result of one entry point.
#[derive(Debug, Clone)]
pub enum Outcome {
    Patched {
        text: String,
        records: Vec<PatchExecutionRecord>,
    },
    /// A patch with `on_failure = "abort"` failed.
    Aborted(String),
}

impl Outcome {
    fn from_result<D>(r: Result<(String, D, Vec<PatchExecutionRecord>), String>) -> Self {
        match r {
            Ok((text, _, records)) => Outcome::Patched { text, records },
            Err(e) => Outcome::Aborted(e),
        }
    }

    /// Records as JSON, so two runs compare field by field.
    fn records_json(records: &[PatchExecutionRecord]) -> String {
        serde_json::to_string(records).unwrap_or_else(|e| format!("<unserializable: {e}>"))
    }
}

/// One entry point's run, labelled for error messages.
#[derive(Debug, Clone)]
pub struct EntryRun {
    pub entry: String,
    pub outcome: Outcome,
}

/// The chunk names the DLL may see for `file` (`@main.lua`, `main.lua`, `@./main.lua`,
/// `.\main.lua`). All name the same game file (D44).
pub fn dll_chunk_names(file: &str) -> [String; 4] {
    [
        format!("@{file}"),
        file.to_string(),
        format!("@./{file}"),
        format!(".\\{}", file.replace('/', "\\")),
    ]
}

/// Runs every entry point on `buffer` as file `file`.
pub fn run_entry_points(table: &PatchTable, file: &str, buffer: &str) -> Vec<EntryRun> {
    let mut runs = vec![
        EntryRun {
            entry: format!("apply_patches_offline({file})"),
            outcome: Outcome::from_result(table.apply_patches_offline(file, buffer)),
        },
        EntryRun {
            entry: format!("apply_patches_offline(@{file})"),
            outcome: Outcome::from_result(table.apply_patches_offline(&format!("@{file}"), buffer)),
        },
    ];

    let traced = match table.apply_patches_traced(file, buffer) {
        Ok((text, _, records, trace)) => {
            if trace.text != text {
                Outcome::Aborted(format!("trace text differs from patched text of {file}"))
            } else {
                Outcome::Patched { text, records }
            }
        }
        Err(e) => Outcome::Aborted(e),
    };
    runs.push(EntryRun {
        entry: format!("apply_patches_traced({file})"),
        outcome: traced,
    });

    // One Prepared for both, as the DLL and the simulation use it.
    let prep = table.prepare();
    runs.push(EntryRun {
        entry: format!("apply_prepared(@{file})"),
        outcome: Outcome::from_result(table.apply_prepared(&prep, &format!("@{file}"), buffer)),
    });
    let traced = match table.apply_prepared_traced(&prep, file, buffer) {
        Ok(t) if t.trace.text != t.text => {
            Outcome::Aborted(format!("trace text differs from patched text of {file}"))
        }
        Ok(t) => Outcome::Patched {
            text: t.text,
            records: t.records,
        },
        Err(e) => Outcome::Aborted(e),
    };
    runs.push(EntryRun {
        entry: format!("apply_prepared_traced({file})"),
        outcome: traced,
    });

    runs
}

/// The DLL's outcome for a chunk. A chunk it loads unpatched compares as that text with no
/// records.
pub fn dll_outcome(
    table: &PatchTable,
    prep: &Prepared,
    launch: &LaunchConfig,
    chunk_name: &str,
    buffer: &str,
) -> Outcome {
    match dll_transform(table, prep, launch, chunk_name, buffer, false) {
        DllOutcome::Passthrough => Outcome::Patched {
            text: buffer.to_string(),
            records: Vec::new(),
        },
        DllOutcome::Prepared(text) => Outcome::Patched {
            text,
            records: Vec::new(),
        },
        DllOutcome::Patched { text, records, .. } => Outcome::Patched { text, records },
        DllOutcome::SyntaxError(e) => Outcome::Aborted(e),
    }
}

/// The simulation's outcome for `file`: the prepared text, patched with a trace.
pub fn simulation_outcome(
    table: &PatchTable,
    prep: &Prepared,
    launch: &LaunchConfig,
    file: &str,
    buffer: &str,
) -> Outcome {
    match simulate_buffer(table, prep, launch, file, buffer) {
        Ok((_, t)) if t.trace.text != t.text => {
            Outcome::Aborted(format!("trace text differs from patched text of {file}"))
        }
        Ok((_, t)) => Outcome::Patched {
            text: t.text,
            records: t.records,
        },
        Err(e) => Outcome::Aborted(e),
    }
}

/// Runs `dll_transform` for every name of `dll_chunk_names(file)` and the simulation, with
/// one shared `Prepared`, and checks that they agree. Returns the simulation's outcome.
pub fn check_dll_parity(
    table: &PatchTable,
    file: &str,
    buffer: &str,
    launch: &LaunchConfig,
) -> Result<Outcome, String> {
    let prep = table.prepare();
    let sim = EntryRun {
        entry: format!("simulation({file})"),
        outcome: simulation_outcome(table, &prep, launch, file, buffer),
    };
    for name in dll_chunk_names(file) {
        let dll = EntryRun {
            entry: format!("dll_transform({name})"),
            outcome: dll_outcome(table, &prep, launch, &name, buffer),
        };
        compare(&sim, &dll)?;
    }
    Ok(sim.outcome)
}

/// Runs every entry point and checks that they agree. Returns the common outcome, or a
/// description of the first difference.
pub fn check_parity(table: &PatchTable, file: &str, buffer: &str) -> Result<Outcome, String> {
    let mut runs = run_entry_points(table, file, buffer).into_iter();
    let Some(first) = runs.next() else {
        return Err("no entry points ran".to_string());
    };
    for other in runs {
        compare(&first, &other)?;
    }
    Ok(first.outcome)
}

fn compare(a: &EntryRun, b: &EntryRun) -> Result<(), String> {
    match (&a.outcome, &b.outcome) {
        (
            Outcome::Patched {
                text: ta,
                records: ra,
            },
            Outcome::Patched {
                text: tb,
                records: rb,
            },
        ) => {
            if ta != tb {
                return Err(format!(
                    "{} and {} give different text ({} vs {} bytes)",
                    a.entry,
                    b.entry,
                    ta.len(),
                    tb.len()
                ));
            }
            let (ja, jb) = (Outcome::records_json(ra), Outcome::records_json(rb));
            if ja != jb {
                return Err(format!(
                    "{} and {} give different records:\n{ja}\n{jb}",
                    a.entry, b.entry
                ));
            }
            Ok(())
        }
        (Outcome::Aborted(ea), Outcome::Aborted(eb)) if ea == eb => Ok(()),
        _ => Err(format!(
            "{} and {} disagree: {:?} vs {:?}",
            a.entry,
            b.entry,
            summary(&a.outcome),
            summary(&b.outcome)
        )),
    }
}

fn summary(o: &Outcome) -> String {
    match o {
        Outcome::Patched { text, records } => {
            format!("patched ({} bytes, {} records)", text.len(), records.len())
        }
        Outcome::Aborted(e) => format!("aborted: {e}"),
    }
}

/// What `check_linemap` looked at.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LineMapCheck {
    pub lines: usize,
    /// Non-empty chains of buffer lines, and their items.
    pub chains: usize,
    pub items: usize,
}

fn kind_name(k: LineKind) -> &'static str {
    match k {
        LineKind::Vanilla | LineKind::Builtin | LineKind::BuiltinEdited => "vanilla",
        LineKind::Inserted => "inserted",
        LineKind::Kept => "kept",
        LineKind::Modified => "modified",
    }
}

fn origin_name(o: &LineOrigin) -> &'static str {
    match o {
        LineOrigin::Vanilla(_) => "vanilla",
        LineOrigin::Inserted { .. } => "inserted",
        LineOrigin::Kept { .. } => "kept",
        LineOrigin::Modified { .. } => "modified",
    }
}

/// The chain of line `a` as the replay shows it: the run of patch lines next to `a`, each
/// line given the block of the edit that wrote it, grouped by the block in its ancestry that
/// hangs on `a`. Lines kept or changed in place keep the map's block (the replay has none).
fn replayed_chain(map: &LineMap, oracle: &FileTrace, a: usize, side: Side) -> Vec<ChainItem> {
    let Some(tag) = map.get(a) else {
        return Vec::new();
    };
    let key = (map.resolve(tag.uid), side);
    let block_of = |l: usize| -> Option<u32> {
        match oracle.lines.get(l)? {
            LineOrigin::Vanilla(_) => None,
            LineOrigin::Inserted { edit } => oracle.edits.get(*edit).map(|e| e.block),
            LineOrigin::Kept { .. } | LineOrigin::Modified { .. } => map.get(l).map(|t| t.block),
        }
    };
    let item_of = |block: u32| -> Option<u32> {
        let mut b = block;
        for _ in 0..=map.blocks().len() {
            let info = map.block(b)?;
            if map.chain_anchor(info) == Some(key) {
                return Some(b);
            }
            b = info.parent;
        }
        None
    };
    let mut items: Vec<ChainItem> = Vec::new();
    let mut add = |l: usize, item: u32| match items.last_mut() {
        Some(last) if last.block == item => {
            last.first = last.first.min(l);
            last.last = last.last.max(l);
        }
        _ => items.push(ChainItem {
            block: item,
            first: l,
            last: l,
            mod_ix: map.block(item).map_or(NO_INDEX, |b| b.mod_ix),
            hint: map.block(item).and_then(|b| b.hint),
        }),
    };
    let next = |l: usize| -> Option<u32> {
        let b = block_of(l)?;
        if b == NO_BLOCK {
            return None;
        }
        item_of(b)
    };
    match side {
        Side::After => {
            let mut l = a + 1;
            while let Some(item) = next(l) {
                add(l, item);
                l += 1;
            }
        }
        Side::Before => {
            let mut l = a;
            while l > 0 {
                l -= 1;
                let Some(item) = next(l) else {
                    break;
                };
                add(l, item);
            }
            items.reverse();
        }
    }
    items
}

/// The simulation's `LineMap` for `file` against the journal replay (`origin::trace`, the
/// oracle): the same kind, owner and maker for every line, the same trace, and for every
/// buffer line the same chains on both sides as the replay's patch lines show (implicit
/// anchors included, as the map's chain rule uses them). Every implicit anchor hangs on a line
/// older than its block.
pub fn check_linemap(
    table: &PatchTable,
    launch: &LaunchConfig,
    file: &str,
    buffer: &str,
) -> Result<LineMapCheck, String> {
    let prep = table.prepare();
    let (buf, t) = simulate_buffer(table, &prep, launch, file, buffer)?;
    let (oracle, owners, makers) = origin::trace_makers(&buf.text, t.trace.edits.clone());
    let tags = t.lines.tags();
    // Every commit's edits matched the text (D43 appends included): the map never resized.
    if t.lines.resized() {
        return Err(format!("{file}: the line map was resized during the apply"));
    }
    if tags.len() != oracle.lines.len() {
        return Err(format!(
            "{file}: the map has {} lines, the replay {}",
            tags.len(),
            oracle.lines.len()
        ));
    }
    for (i, (tag, o)) in tags.iter().zip(&oracle.lines).enumerate() {
        if kind_name(tag.kind) != origin_name(o) {
            return Err(format!(
                "{file}: line {}: the map says {:?}, the replay {o:?}",
                i + 1,
                tag.kind
            ));
        }
    }
    if t.trace.lines != oracle.lines {
        return Err(format!("{file}: the map's line origins differ from the replay"));
    }
    if t.trace.removed != oracle.removed {
        return Err(format!("{file}: the map's removed lines differ from the replay"));
    }
    for (i, (tag, owner)) in tags.iter().zip(&owners).enumerate() {
        let want = match owner {
            Some((record, site)) => {
                let record = u32::try_from(*record).unwrap_or(u32::MAX);
                (t.lines.patch_of_record(record).unwrap_or(NO_INDEX), *site)
            }
            None => (NO_PATCH, NO_SITE),
        };
        if (tag.owner, tag.owner_site) != want {
            return Err(format!(
                "{file}: line {}: the map's owner is {:?}, the replay's {want:?}",
                i + 1,
                (tag.owner, tag.owner_site)
            ));
        }
    }
    for (i, (tag, maker)) in tags.iter().zip(&makers).enumerate() {
        let want = match maker {
            Some((record, _)) => {
                let record = u32::try_from(*record).unwrap_or(u32::MAX);
                t.lines.patch_of_record(record).unwrap_or(NO_INDEX)
            }
            None => NO_PATCH,
        };
        if tag.maker != want {
            return Err(format!(
                "{file}: line {}: the map's maker is {}, the replay's {want}",
                i + 1,
                tag.maker
            ));
        }
    }
    let first_uid = |b: u32| -> Option<u32> {
        tags.iter().filter(|t| t.block == b).map(|t| t.uid).min()
    };
    for (b, info) in t.lines.blocks().iter().enumerate() {
        if !info.implicit {
            continue;
        }
        let b = u32::try_from(b).unwrap_or(u32::MAX);
        let ok = match (info.anchor, first_uid(b)) {
            (None, _) => false,
            (Some((uid, _)), Some(first)) => uid < first,
            (Some(_), None) => true,
        };
        if !ok {
            return Err(format!(
                "{file}: block {b} has an implicit anchor {:?} that is not an older line",
                info.anchor
            ));
        }
    }
    let mut check = LineMapCheck {
        lines: tags.len(),
        ..LineMapCheck::default()
    };
    for (a, tag) in tags.iter().enumerate() {
        if !matches!(
            tag.kind,
            LineKind::Vanilla | LineKind::Builtin | LineKind::BuiltinEdited
        ) {
            continue;
        }
        for side in [Side::After, Side::Before] {
            let got = t.lines.chain(a, side);
            let want = replayed_chain(&t.lines, &oracle, a, side);
            if got != want {
                return Err(format!(
                    "{file}: chain of line {} ({side:?}): the map gives {got:?}, the replay {want:?}",
                    a + 1
                ));
            }
            if !got.is_empty() {
                check.chains += 1;
                check.items += got.len();
            }
        }
    }
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::{InsertPosition, OnFailurePolicy, Patch, PatternPatch, Target};
    use std::path::PathBuf;

    fn pattern(id: &str, pattern: &str, on_failure: OnFailurePolicy) -> Patch {
        Patch::Pattern(PatternPatch {
            pattern: pattern.to_string(),
            position: InsertPosition::After,
            target: Target::Single("main.lua".to_string()),
            payload: "local y = 2".to_string(),
            match_indent: true,
            times: None,
            overwrite: false,
            id: Some(id.to_string()),
            name: None,
            description: None,
            requires: Vec::new(),
            priority: None,
            before: Vec::new(),
            after: Vec::new(),
            on_failure,
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            apply_as: None,
            mod_id: Some("mod_a".to_string()),
            disabled: false,
            qm: Default::default(),
        })
    }

    #[test]
    fn entry_points_agree_on_a_patch() {
        let mut table = PatchTable::default();
        table.targets.insert("main.lua".to_string());
        let p = pattern("mod_a:1", "local x = 1", OnFailurePolicy::Warn);
        table.patches.push((p, 10, PathBuf::from("ModA/qm.toml")));
        let out = check_parity(&table, "main.lua", "local x = 1\nreturn x\n").unwrap();
        let Outcome::Patched { text, records } = out else {
            panic!("expected a patched outcome");
        };
        assert_eq!(text, "local x = 1\nlocal y = 2\nreturn x\n");
        assert_eq!(
            records.iter().map(RecordKey::of).collect::<Vec<_>>(),
            vec![RecordKey {
                patch_id: "mod_a:1".to_string(),
                status: PatchOutcomeStatus::Applied,
                matched_count: 1
            }]
        );
    }

    #[test]
    fn entry_points_agree_on_an_abort() {
        let mut table = PatchTable::default();
        table.targets.insert("main.lua".to_string());
        let p = pattern("mod_a:1", "no such line", OnFailurePolicy::Abort);
        table.patches.push((p, 10, PathBuf::from("ModA/qm.toml")));
        let out = check_parity(&table, "main.lua", "local x = 1\n").unwrap();
        assert!(matches!(out, Outcome::Aborted(_)));
    }

    #[test]
    fn a_difference_is_reported() {
        let a = EntryRun {
            entry: "a".to_string(),
            outcome: Outcome::Patched {
                text: "x".to_string(),
                records: Vec::new(),
            },
        };
        let b = EntryRun {
            entry: "b".to_string(),
            outcome: Outcome::Patched {
                text: "y".to_string(),
                records: Vec::new(),
            },
        };
        assert!(compare(&a, &b).unwrap_err().contains("different text"));
        let c = EntryRun {
            entry: "c".to_string(),
            outcome: Outcome::Aborted("boom".to_string()),
        };
        assert!(compare(&a, &c).unwrap_err().contains("disagree"));
    }
}
