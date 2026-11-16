import type { FixDraft, FixView, PatchLabel, PatchSnapshot, RunAs } from "../bindings";

export type Tone = "ok" | "warn" | "danger" | "info" | "neutral";

/** Fixes that warn before a launch (D23). */
export type AttentionCounts = { needsReview: number; suspended: number; fellBack: number; total: number };

export function attentionCounts(list: FixView[]): AttentionCounts {
  const n = (state: string) => list.filter((f) => f.status.state === state).length;
  const needsReview = n("needs_review");
  const suspended = n("suspended");
  const fellBack = n("fell_back");
  return { needsReview, suspended, fellBack, total: needsReview + suspended + fellBack };
}

/** "1 fix needs review", "2 fixes are suspended", ... for the non-zero counts. */
export function attentionLines(c: AttentionCounts): string[] {
  const line = (count: number, one: string, many: string) => (count === 0 ? [] : [`${count} ${count === 1 ? one : many}`]);
  return [
    ...line(c.needsReview, "fix needs review", "fixes need review"),
    ...line(c.suspended, "fix is suspended", "fixes are suspended"),
    ...line(c.fellBack, "fix uses its own line", "fixes use their own line"),
  ];
}

/** Where the Fixes view lists a fix (§11.4). */
export function sectionOf(f: FixView): "attention" | "working" {
  return ["needs_review", "suspended", "fell_back", "waiting"].includes(f.status.state) ? "attention" : "working";
}

/** Fixes that run a patch as separate edits or skip one of its edits (D46, D49). */
const EDITS_KINDS = new Set(["apply_as", "skip_hunk"]);

/** The status badge of a row (§11.4). */
export function badgeOf(f: FixView): { tone: Tone; text: string } {
  const s = f.status;
  switch (s.state) {
    case "active":
      return { tone: "ok", text: "Working" };
    case "rebound":
      return { tone: "info", text: "Re-bound" };
    case "followed": {
      const mod = f.related[0]?.mod_name ?? f.related[0]?.mod_id;
      return { tone: "info", text: mod ? `Updated to follow ${mod}` : "Updated" };
    }
    case "waiting":
      return { tone: "neutral", text: "Waiting" };
    case "paused":
      return { tone: "neutral", text: "Paused" };
    case "needs_review":
      return { tone: "warn", text: "Needs review" };
    case "fell_back":
      // Separate edits fall back to one replacement, not to a line (D47).
      return { tone: "warn", text: EDITS_KINDS.has(f.kind) ? "Runs as one replacement" : "Uses its own line" };
    case "suspended":
      return { tone: "danger", text: "Suspended" };
  }
}

export function patchText(l: PatchLabel): string {
  const name = l.name ?? l.patch_id;
  const mod = l.mod_name ?? l.mod_id;
  return mod ? `${name} (${mod})` : name;
}

/** "<file> · <patch name> (<mod>)". */
export function whereText(f: FixView): string {
  return [f.target_file, f.patch ? patchText(f.patch) : null].filter((x) => x).join(" · ");
}

/** "7 Nov", in UTC so it reads the same everywhere. */
export function dateText(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString("en-GB", { day: "numeric", month: "short", timeZone: "UTC" });
}

/** Where a fix came from (§11.4). */
export function provenanceText(f: FixView): string {
  const p = f.provenance;
  if (p.source === "migrated") return "Carried over from an earlier version";
  const date = dateText(p.created_at);
  const when = date ? `, ${date}` : "";
  if (p.conflict_id) return `From the conflict in ${f.target_file ?? "a game file"}${when}`;
  const where: Record<string, string> = {
    resolver: "Made in the conflict resolver",
    order_view: "Made in patch order",
    patch_toggle: "Made when you turned a patch off",
    suggestion: "Made from a suggestion",
    auto_follow: "Made by QuarterMaster",
    compat_export: "Made by you",
  };
  return `${where[p.source] ?? "Made by you"}${when}`;
}

/** The status sentence, when it says more than the badge. */
export function statusNote(f: FixView): string | null {
  const t = f.status_text.trim();
  if (!t || t === "Working" || t === "Paused" || t === badgeOf(f).text) return null;
  return t;
}

/** A failed fix preview, said inline next to the button that runs it again. */
export function couldNotCheck(e: unknown): string {
  const why = (e instanceof Error ? e.message : String(e)).trim().replace(/[.\s]+$/, "");
  return why ? `Could not check this fix: ${why}. Try again.` : "Could not check this fix. Try again.";
}

/** Whether Review... has something to show: changed patch text, or a status to settle. */
export function canReview(f: FixView): boolean {
  if (f.legacy) return false;
  return (f.drift?.length ?? 0) > 0 || f.status.state === "needs_review" || f.status.state === "fell_back";
}

/**
 * Review dialog buttons by kind (§11.4). `then` is what the main button does: keep the fix as it
 * is now, pick which patch it is for (several could be, §9.3), or pick a new line for an Anchor
 * fix that uses its patch's own line.
 */
export function reviewActions(f: FixView): { keep: string; undo: string; then: "accept" | "rebind" | "repick" } {
  const undo = f.kind === "disable" ? "Turn it back on" : "Remove fix";
  if ((f.candidates?.length ?? 0) > 0) return { keep: "Use this patch", undo, then: "rebind" };
  if (f.kind === "anchor" && f.status.state === "fell_back" && f.target_file && f.patch) return { keep: "Pick a new line...", undo, then: "repick" };
  if (f.kind === "disable") return { keep: "Keep it off", undo, then: "accept" };
  return { keep: "Keep this fix", undo, then: "accept" };
}

/**
 * "Run it like <mod>" on an older adjusted copy that runs after every mod's patches (D58): the button text, or null
 * when the backend does not offer it.
 */
export function runLikeText(f: FixView, modNames: Record<string, string>): string | null {
  const mod = f.run_like_mod;
  if (f.legacy || f.paused || f.kind !== "adjusted_copy" || !f.glue_id || !mod) return null;
  return `Run it like ${modNames[mod] ?? mod}`;
}

/**
 * "Run it last again" on a copy that "Run it like <mod>" switched to its base mod (D58): the button text, or null
 * when the backend does not offer it.
 */
export function runLastText(f: FixView): string | null {
  if (f.legacy || f.paused || f.kind !== "adjusted_copy" || !f.glue_id || !f.can_run_last) return null;
  return "Run it last again";
}

/**
 * The fix change that runs the copy `f` as its base mod, at its base's place (`base`), or after every mod's patches
 * again (`legacy`), D58.
 */
export function runLikeDraft(f: FixView, runAs: RunAs = "base"): FixDraft | null {
  if (!f.glue_id) return null;
  return {
    change: { kind: "adjusted_copy_run_as", glue_id: f.glue_id, run_as: runAs },
    label: "", source: f.source, conflict_id: null, option_key: null, approved: null,
  };
}

export type LineDiffRow = { kind: "same" | "removed" | "added"; text: string };

/** A line diff of two texts (longest common subsequence; texts are short). */
export function lineDiff(before: string, after: string): LineDiffRow[] {
  const a = before === "" ? [] : before.split("\n");
  const b = after === "" ? [] : after.split("\n");
  const n = a.length;
  const m = b.length;
  // Too long for a table: show both sides whole.
  if (n * m > 250_000) return [...a.map((text) => ({ kind: "removed" as const, text })), ...b.map((text) => ({ kind: "added" as const, text }))];
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: LineDiffRow[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "same", text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      out.push({ kind: "removed", text: a[i++] });
    } else {
      out.push({ kind: "added", text: b[j++] });
    }
  }
  while (i < n) out.push({ kind: "removed", text: a[i++] });
  while (j < m) out.push({ kind: "added", text: b[j++] });
  return out;
}

/** The parts of a snapshot pair that differ, for the drift diff. */
export function snapshotParts(before: PatchSnapshot, after: PatchSnapshot): { label: string; rows: LineDiffRow[] }[] {
  const parts: { label: string; rows: LineDiffRow[] }[] = [];
  if (before.pattern !== after.pattern) parts.push({ label: "The line it looks for", rows: lineDiff(before.pattern, after.pattern) });
  if (before.payload !== after.payload) parts.push({ label: "The code it adds", rows: lineDiff(before.payload, after.payload) });
  if (before.position !== after.position) {
    parts.push({ label: "Where it goes", rows: lineDiff(before.position ?? "", after.position ?? "") });
  }
  if (before.times !== after.times) {
    const t = (n: number | null) => (n == null ? "every match" : String(n));
    parts.push({ label: "How many times", rows: lineDiff(t(before.times), t(after.times)) });
  }
  return parts;
}
