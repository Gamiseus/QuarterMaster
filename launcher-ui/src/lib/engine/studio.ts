/** Pure helpers for the Patch Studio. */

/** Removes the indentation the lines share; blank lines do not count. */
export function dedent(text: string): string {
  const lines = text.split("\n");
  const indents = lines.filter((l) => l.trim() !== "").map((l) => /^[ \t]*/.exec(l)?.[0].length ?? 0);
  const common = indents.length ? Math.min(...indents) : 0;
  return lines.map((l) => (l.trim() === "" ? "" : l.slice(common))).join("\n");
}

/** `<mod_id>:<name as an id>`: lower case, words joined with `_`. */
export function slugId(modId: string, name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return modId ? `${modId}:${slug}` : slug;
}

/** How a match count reads, and its tone: one place is what a pattern wants. */
export function matchText(count: number): { text: string; tone: "ok" | "warn" | "danger" } {
  if (count === 0) return { text: "Found nowhere in the file", tone: "danger" };
  if (count === 1) return { text: "Found in 1 place", tone: "ok" };
  return { text: `Found in ${count} places`, tone: "warn" };
}

/**
 * The game's lines around where a payload goes, shown dimmed above and below the payload
 * editor: for `after`, the anchor and what precedes it above, what follows below; for
 * `before`, what precedes above and the anchor below; for `at`, the anchor is replaced, so
 * only its neighbours show.
 */
export function payloadContext(
  text: string,
  sel: { from: number; to: number } | null,
  position: string,
  around = 3,
): { above: string[]; below: string[] } {
  if (!sel) return { above: [], below: [] };
  const lines = text.split("\n");
  const before = (end: number) => lines.slice(Math.max(0, end - around), end);
  const after = (start: number) => lines.slice(start, start + around);
  const anchor = lines.slice(sel.from - 1, sel.to);
  if (position === "before") return { above: before(sel.from - 1), below: [...anchor, ...after(sel.to)].slice(0, Math.max(around, anchor.length)) };
  if (position === "at") return { above: before(sel.from - 1), below: after(sel.to) };
  return { above: [...before(sel.from - 1), ...anchor].slice(-Math.max(around, anchor.length)), below: after(sel.to) };
}
