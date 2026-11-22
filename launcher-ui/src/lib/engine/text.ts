/** Positions and languages for the Mod Engine's editor. Pure functions, unit-tested. */

export type EditorLanguage = "lua" | "toml" | "json" | "markdown" | "text";

/** The editor language of a file, from its name. */
export function languageOf(path: string): EditorLanguage {
  const ext = path.toLowerCase().split(".").pop() ?? "";
  if (ext === "lua") return "lua";
  if (ext === "toml") return "toml";
  if (ext === "json") return "json";
  if (ext === "md") return "markdown";
  return "text";
}

/**
 * The 0-based character offset of a 1-based line and column in `text`. A missing column is
 * the start of the line; a place past the end is clamped to the text.
 */
export function offsetOf(text: string, line: number, column: number | null): number {
  if (line < 1) return 0;
  let at = 0;
  for (let l = 1; l < line; l++) {
    const next = text.indexOf("\n", at);
    if (next < 0) return text.length;
    at = next + 1;
  }
  const end = text.indexOf("\n", at);
  const lineEnd = end < 0 ? text.length : end;
  return Math.min(at + Math.max(0, (column ?? 1) - 1), lineEnd);
}

/** The range a diagnostic covers: its span, else the whole line it names. */
export function rangeOf(
  text: string,
  d: { line: number | null; column: number | null; end_line: number | null; end_column: number | null },
): { from: number; to: number } | null {
  if (d.line === null) return null;
  const from = offsetOf(text, d.line, d.column);
  if (d.end_line !== null) {
    const to = offsetOf(text, d.end_line, d.end_column);
    return { from, to: Math.max(from, to) };
  }
  // A place without an end: underline to the end of its line, or one character.
  const lineEnd = text.indexOf("\n", from);
  const to = d.column === null ? (lineEnd < 0 ? text.length : lineEnd) : from + 1;
  return { from, to: Math.min(Math.max(from, to), text.length) };
}

/** The file opened first in a mod: `qm.toml`, else `mod.json`, else `faction.toml`, else the first text file. */
export function firstFile(paths: string[]): string | null {
  for (const want of ["qm.toml", "mod.json", "faction.toml", "config.toml"]) {
    const hit = paths.find((p) => p.toLowerCase() === want);
    if (hit) return hit;
  }
  return paths.find((p) => languageOf(p) !== "text" || p.toLowerCase().endsWith(".txt")) ?? null;
}
