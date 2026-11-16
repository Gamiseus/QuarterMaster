/** A run of a log line: `match` runs are the filter text, shown highlighted. */
export interface LinePart {
  text: string;
  match: boolean;
}

function pattern(query: string): RegExp | null {
  const q = query.trim();
  return q ? new RegExp(q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi") : null;
}

/** Case-insensitive substring test; an empty filter matches every line. */
export function lineMatches(line: string, query: string): boolean {
  const re = pattern(query);
  return re === null || re.test(line);
}

/**
 * Splits a line into plain and matching runs, so the view can mark matches with elements built
 * from text. Nothing here is ever parsed as HTML.
 */
export function splitMatches(line: string, query: string): LinePart[] {
  const re = pattern(query);
  if (re === null) return [{ text: line, match: false }];
  const parts: LinePart[] = [];
  let at = 0;
  for (const m of line.matchAll(re)) {
    const start = m.index ?? 0;
    if (start > at) parts.push({ text: line.slice(at, start), match: false });
    parts.push({ text: m[0], match: true });
    at = start + m[0].length;
  }
  if (at < line.length) parts.push({ text: line.slice(at), match: false });
  return parts;
}
