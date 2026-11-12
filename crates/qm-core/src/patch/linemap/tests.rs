use super::*;
use crate::patch::origin::{trace, trace_makers, FileTrace, JournalEdit, LineOrigin};

/// A small deterministic generator (xorshift64*), so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// A rope, its map and the journal, edited the way the apply loop does.
struct Sim {
    original: String,
    rope: Rope,
    map: LineMap,
    journal: Vec<JournalEdit>,
    removed: Vec<Vec<Removed>>,
    records: u32,
    /// Commits as a pattern patch (implicit anchors while the line rules are on, D51).
    implicit: bool,
}

/// One byte edit of a commit, in the text after the commit's earlier edits.
struct ByteChange {
    start: usize,
    end: usize,
    inserted: String,
    attach: Option<Attach>,
}

impl Sim {
    fn new(text: &str, builtin: &[Range<usize>], edited: &[usize]) -> Sim {
        let rope = Rope::from(text);
        let map = LineMap::new(rope.line_len(), builtin, edited);
        Sim {
            original: text.to_string(),
            rope,
            map,
            journal: Vec::new(),
            removed: Vec::new(),
            records: 0,
            implicit: false,
        }
    }

    /// With the line rules on (D50, D51), committing as pattern patches; `chains` turns
    /// implicit chains on (a mode other than Legacy).
    fn with_line_rules(mut self, chains: bool) -> Sim {
        self.map.set_line_rules(true);
        self.map.set_implicit_chains(chains);
        self.implicit = true;
        self
    }

    /// Commits `changes` as one record, as `PatternPatch::apply_in` and the apply loop do.
    fn commit(&mut self, changes: Vec<ByteChange>, mod_ix: u32) -> CommitInfo {
        let record = self.records;
        self.records += 1;
        let mut edits = Vec::new();
        let mut spans = Vec::new();
        for (site, c) in changes.into_iter().enumerate() {
            let edit = byte_edit(
                &self.rope,
                c.start,
                c.end,
                &c.inserted,
                site as u32,
                c.attach,
            );
            let len = self.rope.byte_len();
            let start = c.start.min(len);
            let end = c.end.clamp(start, len);
            self.rope.delete(start..end);
            self.rope.insert(start, &c.inserted);
            spans.push((start, end - start, c.inserted));
            edits.push(edit);
        }
        let info = self.map.commit_with(
            record,
            record,
            mod_ix,
            &edits,
            self.rope.line_len(),
            self.implicit,
        );
        for (site, (start, removed, inserted)) in spans.into_iter().enumerate() {
            self.journal.push(JournalEdit {
                patch_id: format!("p{record}"),
                patch_name: format!("p{record}"),
                mod_id: format!("m{mod_ix}"),
                record: record as usize,
                start,
                removed,
                inserted,
                site: site as u32,
                block: info.blocks.get(site).copied().unwrap_or(NO_BLOCK),
            });
        }
        self.removed.extend(info.removed.iter().cloned());
        info
    }

    fn line_start(&self, line: usize) -> usize {
        self.rope.byte_of_line(line.min(self.rope.line_len()))
    }

    fn attach(&self, line: usize, side: Side) -> Option<Attach> {
        self.map.get(line).map(|t| Attach {
            uid: t.uid,
            side,
            block: t.block,
        })
    }

    /// Inserts `payload` after line `line`, as a pattern `after` does.
    fn insert_after(&mut self, line: usize, payload: &str, mod_ix: u32) -> CommitInfo {
        let at = self.line_start(line + 1);
        let attach = self.attach(line, Side::After);
        self.commit(
            vec![ByteChange {
                start: at,
                end: at,
                inserted: payload.to_string(),
                attach,
            }],
            mod_ix,
        )
    }

    fn insert_before(&mut self, line: usize, payload: &str, mod_ix: u32) -> CommitInfo {
        let at = self.line_start(line);
        let attach = self.attach(line, Side::Before);
        self.commit(
            vec![ByteChange {
                start: at,
                end: at,
                inserted: payload.to_string(),
                attach,
            }],
            mod_ix,
        )
    }

    fn replace(&mut self, line: usize, count: usize, payload: &str, mod_ix: u32) -> CommitInfo {
        let start = self.line_start(line);
        let end = self.line_start(line + count);
        self.commit(
            vec![ByteChange {
                start,
                end,
                inserted: payload.to_string(),
                attach: None,
            }],
            mod_ix,
        )
    }

    fn text(&self) -> String {
        self.rope.to_string()
    }

    /// The map's trace and the journal replay.
    fn traces(&self) -> (FileTrace, FileTrace) {
        let mine = FileTrace::from_linemap(
            self.text(),
            &self.map,
            self.journal.clone(),
            self.removed.clone(),
        );
        let oracle = trace(&self.original, self.journal.clone());
        (mine, oracle)
    }

    /// The map's owners against the replay's (records are patch indexes here).
    fn check_owners(&self) {
        let (_, owners, makers) = trace_makers(&self.original, self.journal.clone());
        let want: Vec<(u32, u32)> = owners
            .iter()
            .map(|o| o.map_or((NO_PATCH, NO_SITE), |(r, s)| (r as u32, s)))
            .collect();
        let got: Vec<(u32, u32)> = self
            .map
            .tags()
            .iter()
            .map(|t| (t.owner, t.owner_site))
            .collect();
        assert_eq!(got, want, "owners");
        let want: Vec<u32> = makers
            .iter()
            .map(|o| o.map_or(NO_PATCH, |(r, _)| r as u32))
            .collect();
        let got: Vec<u32> = self.map.tags().iter().map(|t| t.maker).collect();
        assert_eq!(got, want, "makers");
    }

    fn check_invariants(&self) {
        assert_eq!(self.map.len(), self.rope.line_len(), "one tag per line");
        assert_eq!(self.map.len(), self.text().lines().count());
        for (i, b) in self.map.blocks().iter().enumerate() {
            if b.parent != NO_BLOCK {
                assert!(
                    (b.parent as usize) < i,
                    "block {i}: parent {} is not older",
                    b.parent
                );
                let p = self.map.block(b.parent).unwrap();
                assert_eq!(b.depth, p.depth + 1, "block {i}: depth");
            } else {
                assert_eq!(b.depth, 0, "block {i}: top level has depth 0");
            }
            assert!(
                b.legacy_parent == NO_BLOCK || (b.legacy_parent as usize) < i,
                "block {i}: legacy parent {} is not older",
                b.legacy_parent
            );
            let walked = self.map.anchors_of(i as u32).count();
            assert!(
                walked <= b.depth as usize + 1,
                "block {i}: ancestor walk bounded"
            );
        }
        for t in self.map.tags() {
            let r = self.map.resolve(t.uid);
            assert_eq!(r, t.uid, "a live uid resolves to itself");
            if t.block != NO_BLOCK {
                assert!((t.block as usize) < self.map.blocks().len());
            }
        }
        for &k in self.map.alias.keys() {
            // Terminates, and never at a key that still has an alias.
            let r = self.map.resolve(k);
            assert!(!self.map.alias.contains_key(&r) || r == k);
        }
    }
}

fn kind_of(o: &LineOrigin) -> &'static str {
    match o {
        LineOrigin::Vanilla(_) => "vanilla",
        LineOrigin::Inserted { .. } => "inserted",
        LineOrigin::Kept { .. } => "kept",
        LineOrigin::Modified { .. } => "modified",
    }
}

fn map_kind(k: LineKind) -> &'static str {
    match k {
        LineKind::Vanilla | LineKind::Builtin | LineKind::BuiltinEdited => "vanilla",
        LineKind::Inserted => "inserted",
        LineKind::Kept => "kept",
        LineKind::Modified => "modified",
    }
}

fn random_text(rng: &mut Rng, lines: usize, vocab: &[&str]) -> String {
    let mut s = String::new();
    for _ in 0..lines {
        s.push_str(vocab[rng.below(vocab.len())]);
        s.push('\n');
    }
    if rng.below(3) == 0 && s.ends_with('\n') {
        s.pop();
    }
    s
}

#[test]
fn invariants_hold_after_random_edit_sequences() {
    let vocab = [
        "local a = 1",
        "end",
        "",
        "  if x then",
        "return y",
        "-- note",
        "end",
    ];
    let mut seen: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut nested = 0usize;
    let mut kept_removed = 0usize;
    let mut implicit = 0usize;
    for (seed, rules) in (1..=60u64).flat_map(|s| [(s, None), (s, Some(false)), (s, Some(true))]) {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let n = 1 + rng.below(12);
        let text = random_text(&mut rng, n, &vocab);
        let lines = text.lines().count();
        let builtin = if lines > 2 {
            vec![0..1, lines - 1..lines]
        } else {
            vec![]
        };
        let edited = if lines > 3 { vec![2] } else { vec![] };
        let mut sim = Sim::new(&text, &builtin, &edited);
        if let Some(chains) = rules {
            sim = sim.with_line_rules(chains);
        }
        for step in 0..25 {
            let len = sim.rope.line_len();
            let mod_ix = rng.below(3) as u32;
            let count = 1 + rng.below(3);
            let payload = random_text(&mut rng, count, &vocab) + "\n";
            let payload = payload.replace("\n\n", "\n");
            match rng.below(6) {
                0 => {
                    sim.insert_after(rng.below(len.max(1)), &payload, mod_ix);
                }
                1 => {
                    sim.insert_before(rng.below(len.max(1)), &payload, mod_ix);
                }
                2 => {
                    // An `at` that writes back some of the lines it removes.
                    let line = rng.below(len.max(1));
                    let count = 1 + rng.below(3);
                    let start = sim.line_start(line);
                    let end = sim.line_start(line + count);
                    let old = sim.rope.byte_slice(start..end).to_string();
                    let mut new = String::new();
                    for (i, l) in old.lines().enumerate() {
                        if i % 2 == 0 {
                            new.push_str(&format!("  {l}\n"));
                        }
                        new.push_str(&format!("x{step}\n"));
                    }
                    sim.replace(line, count, &new, mod_ix);
                }
                3 => {
                    // Inside a line (a regex, or an append at the end).
                    let blen = sim.rope.byte_len();
                    let a = rng.below(blen + 1);
                    let b = (a + rng.below(6)).min(blen);
                    let ins = ["X", "y\nz", "\n", "", "q\n"][rng.below(5)];
                    sim.commit(
                        vec![ByteChange {
                            start: a,
                            end: b,
                            inserted: ins.to_string(),
                            attach: None,
                        }],
                        mod_ix,
                    );
                }
                4 => {
                    // Two sites in one record, the second in the text after the first.
                    let l1 = rng.below(len.max(1));
                    let p1 = sim.line_start(l1 + 1);
                    let attach = sim.attach(l1, Side::After);
                    let first = ByteChange {
                        start: p1,
                        end: p1,
                        inserted: payload.clone(),
                        attach,
                    };
                    let shift = payload.len();
                    let l2 = l1 + 1 + rng.below(3);
                    let p2 = sim.line_start(l2 + 1).max(p1) + shift;
                    let second = ByteChange {
                        start: p2,
                        end: p2,
                        inserted: format!("s{step}\n"),
                        attach: None,
                    };
                    sim.commit(vec![first, second], mod_ix);
                }
                _ => {
                    // A pure delete.
                    let line = rng.below(len.max(1));
                    sim.replace(line, 1, "", mod_ix);
                }
            }
            sim.check_invariants();
        }
        let (mine, oracle) = sim.traces();
        let mk: Vec<_> = sim.map.tags().iter().map(|t| map_kind(t.kind)).collect();
        let ok: Vec<_> = oracle.lines.iter().map(kind_of).collect();
        assert_eq!(mk, ok, "seed {seed}: kinds");
        assert_eq!(mine, oracle, "seed {seed}: trace");
        sim.check_owners();
        implicit += sim.map.blocks().iter().filter(|b| b.implicit).count();
        if rules.is_none() {
            assert!(sim.map.blocks().iter().all(|b| !b.implicit), "seed {seed}");
        }
        for k in mk {
            *seen.entry(k).or_insert(0usize) += 1;
        }
        nested += sim
            .map
            .blocks()
            .iter()
            .filter(|b| b.parent != NO_BLOCK)
            .count();
        kept_removed += sim.removed.iter().flatten().filter(|r| r.kept).count();
    }
    // The sequences cover every kind, nested blocks and kept lines.
    for k in ["vanilla", "inserted", "kept", "modified"] {
        assert!(
            seen.get(k).copied().unwrap_or(0) > 0,
            "no {k} lines: {seen:?}"
        );
    }
    assert!(
        nested > 0 && kept_removed > 0 && implicit > 0,
        "nested {nested}, kept {kept_removed}, implicit {implicit}"
    );
}

#[test]
fn an_insert_after_an_inserted_line_hangs_under_its_block() {
    let mut sim = Sim::new("a\nb\nc\n", &[], &[]);
    let outer = sim.insert_after(0, "o1\no2\n", 0).blocks[0];
    // "o1" is line 1; insert after it.
    let inner = sim.insert_after(1, "i1\n", 1).blocks[0];
    assert_eq!(sim.text(), "a\no1\ni1\no2\nb\nc\n");
    let ob = sim.map.block(outer).unwrap();
    let ib = sim.map.block(inner).unwrap();
    assert_eq!(ob.parent, NO_BLOCK);
    assert_eq!(ob.depth, 0);
    assert_eq!(ob.anchor, Some((0, Side::After)));
    assert_eq!(ib.parent, outer);
    assert_eq!(ib.depth, 1);
    let o1_uid = sim.map.get(1).unwrap().uid;
    assert_eq!(ib.anchor, Some((o1_uid, Side::After)));
    // The nested insert travels with the outer item in the chain of "a".
    let chain = sim.map.chain(0, Side::After);
    assert_eq!(
        chain,
        vec![ChainItem {
            block: outer,
            first: 1,
            last: 3,
            mod_ix: 0,
            hint: None
        }]
    );
    // In the chain of "o1", it is the item.
    let chain = sim.map.chain(1, Side::After);
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].block, inner);
    assert_eq!((chain[0].first, chain[0].last), (2, 2));
    // Its anchors: its own, then the outer block's.
    let anchors: Vec<_> = sim.map.anchors_of(inner).collect();
    assert_eq!(anchors, vec![(o1_uid, Side::After), (0, Side::After)]);
    // The site run of the outer patch is its own lines only.
    let runs = sim.map.site_runs(0, false);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].lines, vec![1, 3]);
    assert_eq!(runs[0].blocks, vec![outer]);
}

#[test]
fn kept_lines_keep_their_uid_and_no_block() {
    let mut sim = Sim::new("x\ntarget\ny\n", &[], &[]);
    let info = sim.replace(1, 1, "pre\n  target\npost\n", 0);
    let rho = info.blocks[0];
    assert_eq!(sim.text(), "x\npre\n  target\npost\ny\n");
    let kept = *sim.map.get(2).unwrap();
    assert_eq!(kept.kind, LineKind::Kept);
    assert_eq!(kept.uid, 1, "the removed line's uid");
    assert_eq!(
        kept.block, NO_BLOCK,
        "a re-emitted vanilla line stays top level"
    );
    assert_eq!(kept.vanilla, 1);
    for l in [1, 3] {
        let t = sim.map.get(l).unwrap();
        assert_eq!((t.kind, t.block), (LineKind::Inserted, rho));
    }
    let b = sim.map.block(rho).unwrap();
    assert_eq!((b.anchor, b.parent), (None, NO_BLOCK));
    assert_eq!(
        info.removed[0],
        vec![Removed {
            tag: LineTag {
                record: NO_RECORD,
                kind: LineKind::Vanilla,
                owner: NO_PATCH,
                owner_site: NO_SITE,
                maker: NO_PATCH,
                ..kept
            },
            kept: true
        }]
    );
    assert_eq!((kept.owner, kept.owner_site), (0, 0), "the `at` owns the line it kept");
    // The Replace block has no anchor, so the kept line's after-chain is empty until an
    // insert hangs on it (today's rules; with the line rules on it is a chain item, D51:
    // `with_the_line_rules_a_kept_line_chains_its_replace_block`).
    assert!(sim.map.chain(2, Side::After).is_empty());
    let after = sim.insert_after(2, "follow\n", 1).blocks[0];
    let chain = sim.map.chain(2, Side::After);
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].block, after);
    assert_eq!(sim.map.block(after).unwrap().parent, NO_BLOCK);
}

#[test]
fn a_replaced_line_aliases_to_its_replacement() {
    let mut sim = Sim::new("a\nb\nc\n", &[], &[]);
    let b_uid = sim.map.get(1).unwrap().uid;
    sim.replace(1, 1, "B\n", 0);
    let new_uid = sim.map.get(1).unwrap().uid;
    assert_ne!(new_uid, b_uid);
    assert_eq!(sim.map.resolve(b_uid), new_uid);
    // A pure delete adds no alias.
    let c_uid = sim.map.get(2).unwrap().uid;
    sim.replace(2, 1, "", 0);
    assert_eq!(sim.map.resolve(c_uid), c_uid);
    assert_eq!(sim.text(), "a\nB\n");
}

#[test]
fn before_chains_mirror_after_chains() {
    let mut sim = Sim::new("a\nb\n", &[], &[]);
    let first = sim.insert_before(1, "p1\n", 0).blocks[0];
    // A later `before` on the same line goes right above it (today's order).
    let second = sim.insert_before(2, "p2\n", 1).blocks[0];
    assert_eq!(sim.text(), "a\np1\np2\nb\n");
    let chain = sim.map.chain(3, Side::Before);
    assert_eq!(
        chain.iter().map(|c| c.block).collect::<Vec<_>>(),
        vec![first, second]
    );
    assert_eq!(
        chain.iter().map(|c| c.mod_ix).collect::<Vec<_>>(),
        vec![0, 1]
    );
    // The before-chain of "b" is not part of the after-chain of "a".
    assert!(sim.map.chain(0, Side::After).is_empty());
}

#[test]
fn builtin_lines_are_tagged_and_followed_into_in_line_edits() {
    let mut sim = Sim::new("qm\nconf\nvanilla\nqm2\n", &[0..1, 3..4], &[1]);
    let kinds: Vec<_> = sim.map.tags().iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            LineKind::Builtin,
            LineKind::BuiltinEdited,
            LineKind::Vanilla,
            LineKind::Builtin
        ]
    );
    assert!(sim.map.is_builtin_line(0) && sim.map.is_builtin_line(1));
    assert!(!sim.map.is_builtin_line(2));
    // A change inside QuarterMaster's line keeps it QuarterMaster's; a vanilla one does not.
    sim.commit(
        vec![ByteChange {
            start: 4,
            end: 4,
            inserted: "X".to_string(),
            attach: None,
        }],
        0,
    );
    assert_eq!(sim.text(), "qm\ncXonf\nvanilla\nqm2\n");
    assert_eq!(sim.map.get(1).unwrap().kind, LineKind::Modified);
    assert!(sim.map.is_builtin_line(1));
    sim.commit(
        vec![ByteChange {
            start: 9,
            end: 9,
            inserted: "Y".to_string(),
            attach: None,
        }],
        0,
    );
    assert!(!sim.map.is_builtin_line(2));
}

#[test]
fn records_and_runs_follow_the_latest_writer() {
    let mut sim = Sim::new("a\nb\n", &[], &[]);
    sim.insert_after(0, "x\ny\n", 0);
    // Record 1 replaces "y" (record 0's line).
    sim.replace(2, 1, "Y\n", 1);
    assert_eq!(sim.text(), "a\nx\nY\nb\n");
    assert_eq!(sim.map.record_of_line(0), None);
    assert_eq!(sim.map.record_of_line(1), Some(0));
    assert_eq!(sim.map.record_of_line(2), Some(1));
    assert_eq!(sim.map.record_of_line(99), None);
    let runs = sim.map.record_runs();
    assert_eq!(runs.get(&0), Some(&vec![(2, 2)]));
    assert_eq!(runs.get(&1), Some(&vec![(3, 3)]));
    assert_eq!(sim.map.line_records(), vec![NO_RECORD, 0, 1, NO_RECORD]);
}

#[test]
fn out_of_range_edits_and_lookups_never_panic() {
    // A range past the end, and a reversed one.
    let reversed = Range { start: 7, end: 2 };
    let mut map = LineMap::new(3, &[0..99, reversed], &[50]);
    assert_eq!(map.len(), 3);
    let edits = vec![
        Edit::Lines {
            at: 100,
            removed: 5,
            inserted: 2,
            site: 0,
            attach: Some(Attach {
                uid: 9999,
                side: Side::After,
                block: 12345,
            }),
            kept: vec![(7, 7), (0, 99)],
        },
        Edit::Touched {
            first_line: 1000,
            removed_lines: 1000,
            inserted_lines: 3,
            site: 1,
        },
        Edit::Lines {
            at: 0,
            removed: 1000,
            inserted: 0,
            site: 2,
            attach: None,
            kept: vec![],
        },
    ];
    // The rope says 4 lines; the map is resized to it.
    let info = map.commit(0, 0, 0, &edits, 4);
    assert_eq!(map.len(), 4);
    assert_eq!(info.blocks.len(), 3);
    for b in map.blocks() {
        assert!(b.parent == NO_BLOCK || (b.parent as usize) < map.blocks().len());
    }
    assert!(map.get(99).is_none());
    assert!(map.chain(99, Side::After).is_empty());
    assert!(map.chain(0, Side::Before).is_empty());
    assert_eq!(map.anchors_of(NO_BLOCK).count(), 0);
    assert!(!map.is_builtin_line(1234));
    assert_eq!(map.resolve(424242), 424242);

    // Byte offsets past the end are clamped.
    let rope = Rope::from("ab\ncd");
    let e = byte_edit(&rope, 99, 7, "x\n", 0, None);
    assert!(matches!(e, Edit::Touched { first_line: 1, .. }), "{e:?}");
    let e = byte_edit(&Rope::from(""), 5, 9, "", 0, None);
    assert_eq!(
        e,
        Edit::Lines {
            at: 0,
            removed: 0,
            inserted: 0,
            site: 0,
            attach: None,
            kept: vec![]
        }
    );
}

#[test]
fn byte_edits_classify_like_the_trace() {
    // Inserting whole lines at a line start.
    let rope = Rope::from("a\nb\n");
    assert!(matches!(
        byte_edit(&rope, 2, 2, "x\ny\n", 0, None),
        Edit::Lines {
            at: 1,
            removed: 0,
            inserted: 2,
            ..
        }
    ));
    // After an unterminated last line: the line is touched.
    let rope = Rope::from("a\nend");
    assert_eq!(
        byte_edit(&rope, 5, 5, "print()\n", 0, None),
        Edit::Touched {
            first_line: 1,
            removed_lines: 1,
            inserted_lines: 1,
            site: 0
        }
    );
    // Copy append on a text ending with a newline: two whole new lines (D53).
    let rope = Rope::from("a\n");
    assert_eq!(
        byte_edit(&rope, 2, 2, "\nc", 0, None),
        Edit::Lines {
            at: 1,
            removed: 0,
            inserted: 2,
            site: 0,
            attach: None,
            kept: vec![]
        }
    );
    // On an unterminated last line the leading newline ends that line: one new line.
    let rope = Rope::from("a\nb");
    assert_eq!(
        byte_edit(&rope, 3, 3, "\nc", 0, None),
        Edit::Lines {
            at: 2,
            removed: 0,
            inserted: 1,
            site: 0,
            attach: None,
            kept: vec![]
        }
    );
    // Not at the end of the text: inside a line, as before.
    assert!(matches!(
        byte_edit(&rope, 1, 1, "\nc", 0, None),
        Edit::Touched { first_line: 0, .. }
    ));
    // The rope's line numbers at its end, as the commit reads them.
    assert_eq!(Rope::from("a\nb\n").line_of_byte(4), 2);
    assert_eq!(Rope::from("a\nb").line_len(), 2);
    // `at` re-emitting its line.
    let rope = Rope::from("x\nt.a = 1\ny\n");
    assert_eq!(
        byte_edit(&rope, 2, 10, "-- new\n  t.a = 1\n", 0, None),
        Edit::Lines {
            at: 1,
            removed: 1,
            inserted: 2,
            site: 0,
            attach: None,
            kept: vec![(1, 0)]
        }
    );
}

#[test]
fn kept_pairs_follow_the_trace_rule() {
    assert_eq!(
        kept_pairs(&["a", "b", "c"], &["b", "x", "c", "a"]),
        vec![(0, 1), (2, 2)]
    );
    assert_eq!(
        kept_pairs(&["", "a"], &["", "a"]),
        vec![(1, 1)],
        "blank lines are never kept"
    );
    assert!(kept_pairs(&[], &["a"]).is_empty());
}

#[test]
fn a_rewritten_anchor_keeps_its_chain() {
    // After "a": block g. Then an `at` rewrites "a"; g must be in the chain of "A".
    let mut sim = Sim::new("a\nb\n", &[], &[]);
    let g = sim.insert_after(0, "g\n", 0).blocks[0];
    sim.replace(0, 1, "A\n", 1);
    assert_eq!(sim.text(), "A\ng\nb\n");
    let chain = sim.map.chain(0, Side::After);
    assert_eq!(
        chain,
        vec![ChainItem {
            block: g,
            first: 1,
            last: 1,
            mod_ix: 0,
            hint: None
        }]
    );
    let new_uid = sim.map.get(0).unwrap().uid;
    assert_eq!(sim.map.anchors_of(g).collect::<Vec<_>>(), vec![(new_uid, Side::After)]);
    // The before mirror.
    let mut sim = Sim::new("h\nm\n", &[], &[]);
    let t = sim.insert_before(1, "t\n", 0).blocks[0];
    sim.replace(2, 1, "M\n", 1);
    assert_eq!(sim.text(), "h\nt\nM\n");
    let chain = sim.map.chain(2, Side::Before);
    assert_eq!(chain.iter().map(|c| c.block).collect::<Vec<_>>(), vec![t]);
}

#[test]
fn an_anchorless_insert_nests_in_the_block_above_it() {
    let mut sim = Sim::new("a\nb\n", &[], &[]);
    let g = sim.insert_after(0, "g1\ng2\n", 0).blocks[0];
    // A whole-line insert with no anchor between g1 and g2 (a regex does this).
    let r = sim.replace(2, 0, "r\n", 1).blocks[0];
    assert_eq!(sim.text(), "a\ng1\nr\ng2\nb\n");
    let rb = sim.map.block(r).unwrap();
    assert_eq!((rb.anchor, rb.parent, rb.depth), (None, g, 1));
    assert_eq!(
        sim.map.chain(0, Side::After),
        vec![ChainItem {
            block: g,
            first: 1,
            last: 3,
            mod_ix: 0,
            hint: None
        }]
    );
    // Below a buffer line it stays top level (today's rules; with the line rules it also
    // hangs on that line, D51: `with_the_line_rules_an_anchorless_insert_hangs_on_the_line_above`).
    let top = sim.replace(5, 0, "z\n", 1).blocks[0];
    assert_eq!(sim.map.block(top).unwrap().parent, NO_BLOCK);
    sim.check_invariants();
}

// ---- Line rules (D50, D51) ----

#[test]
fn with_the_line_rules_a_kept_line_chains_its_replace_block() {
    // The twin of `kept_lines_keep_their_uid_and_no_block` (D51): the Replace block hangs
    // after the kept line, so the kept line's after-chain holds it before any insert does.
    for chains in [false, true] {
        let mut sim = Sim::new("x\ntarget\ny\n", &[], &[]).with_line_rules(chains);
        let rho = sim.replace(1, 1, "pre\n  target\npost\n", 0).blocks[0];
        let b = *sim.map.block(rho).unwrap();
        assert_eq!((b.anchor, b.parent, b.implicit), (Some((1, Side::After)), NO_BLOCK, true));
        let chain: Vec<u32> = sim.map.chain(2, Side::After).iter().map(|c| c.block).collect();
        if chains {
            assert_eq!(chain, vec![rho]);
        } else {
            assert!(chain.is_empty(), "Legacy keeps today's chain rule");
        }
        // Coverage never reads it.
        assert_eq!(sim.map.anchors_of(rho).count(), 1);
        assert_eq!(sim.map.explicit_anchors_of(rho).count(), 0);
        sim.check_invariants();
        sim.check_owners();
    }
    // A Replace with no kept line above a new line gets none.
    let mut sim = Sim::new("x\ntarget\ny\n", &[], &[]).with_line_rules(true);
    let r = sim.replace(1, 1, "new\n  target\n", 0).blocks[0];
    assert_eq!(sim.map.block(r).unwrap().anchor, None);
    let r = sim.replace(0, 1, "X\n", 0).blocks[0];
    assert_eq!(sim.map.block(r).unwrap().anchor, None);
}

#[test]
fn with_the_line_rules_an_anchorless_insert_hangs_on_the_line_above() {
    // The twin of `an_anchorless_insert_nests_in_the_block_above_it` (D51).
    let mut sim = Sim::new("a\nb\n", &[], &[]).with_line_rules(true);
    let g = sim.insert_after(0, "g1\ng2\n", 0).blocks[0];
    let r = sim.replace(2, 0, "r\n", 1).blocks[0];
    let g1 = sim.map.get(1).unwrap().uid;
    let rb = *sim.map.block(r).unwrap();
    assert_eq!(
        (rb.anchor, rb.parent, rb.depth, rb.implicit),
        (Some((g1, Side::After)), g, 1, true)
    );
    // Below a buffer line it is top level and now an item of that line's chain.
    let top = sim.replace(5, 0, "z\n", 1).blocks[0];
    let tb = *sim.map.block(top).unwrap();
    assert_eq!((tb.parent, tb.implicit), (NO_BLOCK, true));
    let b_line = 4;
    assert_eq!(sim.text().lines().nth(b_line), Some("b"));
    let chain: Vec<u32> = sim.map.chain(b_line, Side::After).iter().map(|c| c.block).collect();
    assert_eq!(chain, vec![top]);
    sim.map.set_implicit_chains(false);
    assert!(sim.map.chain(b_line, Side::After).is_empty());
    // At the top of the file it hangs before the line below.
    let mut sim = Sim::new("a\n", &[], &[]).with_line_rules(true);
    let head = sim.replace(0, 0, "h\n", 0).blocks[0];
    assert_eq!(sim.map.block(head).unwrap().anchor, Some((0, Side::Before)));
    sim.check_invariants();
    sim.check_owners();
}

#[test]
fn an_explicit_anchor_is_never_implicit() {
    let mut sim = Sim::new("a\nb\n", &[], &[]).with_line_rules(true);
    let g = sim.insert_after(0, "g\n", 0).blocks[0];
    let b = sim.map.block(g).unwrap();
    assert_eq!((b.anchor, b.implicit), (Some((0, Side::After)), false));
    // Without the line rules, a pattern commit records no implicit anchor.
    let mut sim = Sim::new("a\nb\n", &[], &[]);
    sim.implicit = true;
    let r = sim.replace(1, 0, "r\n", 0).blocks[0];
    assert_eq!(sim.map.block(r).unwrap().anchor, None);
}

#[test]
fn owners_survive_a_later_edit_inside_the_line() {
    let mut sim = Sim::new("a\nb\n", &[], &[]);
    sim.insert_after(0, "x\ny\n", 0);
    // Record 1 edits inside "x" (a regex).
    sim.commit(
        vec![ByteChange {
            start: 2,
            end: 3,
            inserted: "X".to_string(),
            attach: None,
        }],
        1,
    );
    assert_eq!(sim.text(), "a\nX\ny\nb\n");
    let x = *sim.map.get(1).unwrap();
    assert_eq!((x.kind, x.record, x.owner, x.owner_site), (LineKind::Modified, 1, 0, 0));
    // A buffer line it changes is its own.
    sim.commit(
        vec![ByteChange {
            start: 0,
            end: 1,
            inserted: "A".to_string(),
            attach: None,
        }],
        2,
    );
    let a = *sim.map.get(0).unwrap();
    assert_eq!((a.owner, a.owner_site), (2, 0));
    // Today's runs follow the last writer; owner runs keep the line.
    assert_eq!(sim.map.site_runs(0, false)[0].lines, vec![2]);
    assert_eq!(sim.map.site_runs(0, true)[0].lines, vec![1, 2]);
    assert!(sim.map.site_runs(1, true).is_empty());
    assert_eq!(sim.map.site_runs(1, false)[0].lines, vec![1]);
    sim.check_owners();
}

#[test]
fn a_kept_line_takes_the_keeper_as_owner() {
    // §4.3 item 1 (Task 13b): record 0 inserts `x`, record 1's `at` keeps it and adds `y`.
    // The kept line is record 1's under both run rules.
    let mut sim = Sim::new("a\nb\n", &[], &[]);
    sim.insert_after(0, "x\n", 0);
    sim.replace(1, 1, "x\ny\n", 1);
    assert_eq!(sim.text(), "a\nx\ny\nb\n");
    let x = *sim.map.get(1).unwrap();
    assert_eq!(
        (x.kind, x.record, x.owner, x.owner_site),
        (LineKind::Kept, 1, 1, 0)
    );
    // The maker stays the patch that inserted it (D49 reads it).
    assert_eq!(x.maker, 0);
    assert_eq!(sim.map.site_runs(1, true)[0].lines, vec![1, 2]);
    assert_eq!(sim.map.site_runs(1, false)[0].lines, vec![1, 2]);
    assert!(sim.map.site_runs(0, true).is_empty());
    sim.check_owners();
    // Record 2 edits inside the kept line: the owner stays the keeper.
    sim.commit(
        vec![ByteChange {
            start: 2,
            end: 3,
            inserted: "X".to_string(),
            attach: None,
        }],
        2,
    );
    assert_eq!(sim.text(), "a\nX\ny\nb\n");
    let x = *sim.map.get(1).unwrap();
    assert_eq!(
        (x.kind, x.record, x.owner, x.owner_site),
        (LineKind::Modified, 2, 1, 0)
    );
    assert_eq!(x.maker, 0);
    assert_eq!(sim.map.site_runs(1, true)[0].lines, vec![1, 2]);
    assert_eq!(sim.map.site_runs(1, false)[0].lines, vec![2]);
    sim.check_owners();
}

#[test]
fn owner_runs_group_by_the_owner_site() {
    // Patch 0 has two sites; patch 1 changes a line of site 1 as its own site 0.
    let mut sim = Sim::new("v\nw\nv\n", &[], &[]);
    let p1 = sim.line_start(1);
    let p2 = sim.line_start(3);
    let (a1, a2) = (sim.attach(0, Side::After), sim.attach(2, Side::After));
    sim.commit(
        vec![
            ByteChange {
                start: p1,
                end: p1,
                inserted: "bx = 1\nby = 2\n".to_string(),
                attach: a1,
            },
            ByteChange {
                start: p2 + 14,
                end: p2 + 14,
                inserted: "bx = 1\nby = 2\n".to_string(),
                attach: a2,
            },
        ],
        0,
    );
    assert_eq!(sim.text(), "v\nbx = 1\nby = 2\nw\nv\nbx = 1\nby = 2\n");
    let at = sim.line_start(5) + 5;
    sim.commit(
        vec![ByteChange {
            start: at,
            end: at + 1,
            inserted: "10".to_string(),
            attach: None,
        }],
        1,
    );
    assert_eq!(sim.map.get(5).unwrap().site, 0, "the toucher's site");
    let by_owner = sim.map.site_runs(0, true);
    assert_eq!(by_owner.len(), 2);
    assert_eq!((by_owner[1].site, by_owner[1].lines.clone()), (1, vec![5, 6]));
    let today = sim.map.site_runs(0, false);
    assert_eq!((today[1].site, today[1].lines.clone()), (1, vec![6]));
    sim.check_owners();
}
