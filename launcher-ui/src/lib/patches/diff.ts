import type { AttributedCodeLine, PatchExecutionRecord } from "../bindings";

export type DiffSide = { n: number; text: string };

/** One aligned line pair. `left` is the game's file, `right` the patched file. */
export type DiffRow = {
  kind: "same" | "added" | "removed" | "replaced";
  left?: DiffSide;
  right?: DiffSide;
  mod?: string;
  patch?: string;
};

/** Unchanged rows `start` to `end` (exclusive) folded into one row. */
export type FoldRow = { kind: "fold"; start: number; end: number };

export type DisplayRow = DiffRow | FoldRow;

/** Folds shorter than this stay open: a fold row saves nothing over three lines. */
const MIN_FOLD = 4;

type Added = { side: DiffSide; mod?: string; patch?: string };

/**
 * Aligns the game's file with the patched file. Patched lines with no patch attribution are the
 * game's own lines, in order, so they anchor the alignment; between two of them, the game's lines
 * that are gone were removed and the attributed lines were added. A removed and an added line at
 * the same place pair up as `replaced`.
 *
 * Over 1,200 lines the simulation keeps only patched lines and a few around them; the lines it
 * left out are unchanged, so they are rebuilt from the game's file. `patchedLineCount` (from the
 * simulation) says how long the patched file is, so its unchanged tail can be rebuilt too.
 */
export function alignRows(
  original: string[],
  attributed: AttributedCodeLine[],
  records: PatchExecutionRecord[],
  patchedLineCount?: number,
): DiffRow[] {
  const byLine = new Map(attributed.map((a) => [a.line_number, a]));
  const last = patchedLineCount ?? attributed.reduce((m, a) => Math.max(m, a.line_number), 0);
  const rows: DiffRow[] = [];
  let removed: DiffSide[] = [];
  let added: Added[] = [];
  let oi = 0;

  const flush = (nextRight: number) => {
    const pairs = Math.min(removed.length, added.length);
    for (let k = 0; k < pairs; k++) {
      rows.push({ kind: "replaced", left: removed[k], right: added[k].side, mod: added[k].mod, patch: added[k].patch });
    }
    for (const a of added.slice(pairs)) rows.push({ kind: "added", right: a.side, mod: a.mod, patch: a.patch });
    const rest = removed.slice(pairs);
    if (rest.length > 0) {
      // A removal with nothing added in its place: credit the patch whose region sits there.
      const by = added[0] ?? recordAt(records, nextRight);
      for (const side of rest) rows.push({ kind: "removed", left: side, mod: by?.mod, patch: by?.patch });
    }
    removed = [];
    added = [];
  };

  for (let n = 1; n <= last; n++) {
    const a = byLine.get(n);
    if (a?.patch_id) {
      added.push({ side: { n, text: a.text }, mod: a.mod_id ?? undefined, patch: a.patch_id });
      continue;
    }
    if (a) {
      const j = original.indexOf(a.text, oi);
      if (j === -1) {
        // Not the game's text after all: show it as added without a patch.
        added.push({ side: { n, text: a.text } });
        continue;
      }
      for (; oi < j; oi++) removed.push({ n: oi + 1, text: original[oi] });
    }
    flush(n);
    const text = a ? a.text : (original[oi] ?? "");
    rows.push({ kind: "same", left: { n: oi + 1, text: original[oi] ?? text }, right: { n, text } });
    oi++;
  }
  for (; oi < original.length; oi++) removed.push({ n: oi + 1, text: original[oi] });
  flush(last + 1);
  return rows;
}

function recordAt(records: PatchExecutionRecord[], line: number): { mod?: string; patch?: string } | undefined {
  const r = records.find((rec) => rec.regions.some((g) => g.start_line === line));
  return r ? { mod: r.mod_id, patch: r.patch_id } : undefined;
}

/**
 * Keeps `context` unchanged rows around every change and folds longer unchanged runs.
 * `expanded` holds the `start` of folds the user opened.
 */
export function collapse(rows: DiffRow[], context = 3, expanded: ReadonlySet<number> = new Set()): DisplayRow[] {
  const keep = rows.map((r) => r.kind !== "same");
  rows.forEach((r, i) => {
    if (r.kind === "same") return;
    for (let k = Math.max(0, i - context); k <= Math.min(rows.length - 1, i + context); k++) keep[k] = true;
  });
  const out: DisplayRow[] = [];
  let i = 0;
  while (i < rows.length) {
    if (keep[i]) {
      out.push(rows[i++]);
      continue;
    }
    const start = i;
    while (i < rows.length && !keep[i]) i++;
    if (i - start < MIN_FOLD || expanded.has(start)) out.push(...rows.slice(start, i));
    else out.push({ kind: "fold", start, end: i });
  }
  return out;
}

const isChange = (r: { kind: string }) => r.kind !== "same" && r.kind !== "fold";

/** The first row of each change (a run of changed rows counts once), for Next and Previous. */
export function changeIndices(rows: { kind: string }[]): number[] {
  return rows.flatMap((r, i) => (isChange(r) && (i === 0 || !isChange(rows[i - 1])) ? [i] : []));
}

/** One rendered line. Split mode shows a row per line; unified mode splits `replaced` in two. */
export type ViewLine = {
  kind: DiffRow["kind"] | "fold";
  left?: DiffSide;
  right?: DiffSide;
  mod?: string;
  patch?: string;
  /** For a fold: the rows it hides. */
  fold?: FoldRow;
  key: string;
};

export function viewLines(rows: DisplayRow[], mode: "split" | "unified"): ViewLine[] {
  const out: ViewLine[] = [];
  rows.forEach((r, i) => {
    if (r.kind === "fold") {
      out.push({ kind: "fold", fold: r, key: `f${r.start}` });
    } else if (mode === "unified" && r.kind === "replaced") {
      out.push({ kind: "removed", left: r.left, mod: r.mod, patch: r.patch, key: `${i}-` });
      out.push({ kind: "added", right: r.right, mod: r.mod, patch: r.patch, key: `${i}+` });
    } else {
      out.push({ ...r, key: `${i}` });
    }
  });
  return out;
}
