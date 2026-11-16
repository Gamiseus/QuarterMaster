import { rowKey, type OrderRow } from "./order";

/** Kit separator colors; must match --qm-sep-1..6 in tokens.css (checked by modlist-view.test.ts). */
export const SEP_HEX = ["#d4a742", "#6b7a45", "#b5562e", "#5b7a8c", "#c9b98f", "#6e8070"] as const;
export const SEP_NAMES = ["Brass", "Olive", "Rust", "Steel", "Sand", "Slate"] as const;

function parseHex(value: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(value.trim());
  if (!m) return null;
  const h = m[1].length === 3 ? [...m[1]].map((c) => c + c).join("") : m[1];
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)) as [number, number, number];
}

/**
 * Kit color index (1..6) for a stored separator color: "sep-N" maps to N; any other value
 * (the backend seeds hex) maps to the nearest kit color by RGB distance; unparsable -> 1.
 */
export function sepColorIndex(color: string): number {
  const named = /^sep-([1-6])$/.exec(color);
  if (named) return Number(named[1]);
  const rgb = parseHex(color);
  if (!rgb) return 1;
  let best = 1;
  let bestDist = Infinity;
  SEP_HEX.forEach((hex, i) => {
    const [r, g, b] = parseHex(hex) as [number, number, number];
    const d = (r - rgb[0]) ** 2 + (g - rgb[1]) ** 2 + (b - rgb[2]) ** 2;
    if (d < bestDist) {
      bestDist = d;
      best = i + 1;
    }
  });
  return best;
}

/** Rows with the mods of collapsed separator sections removed; ids in `keep` always stay. */
export function collapseRows(rows: OrderRow[], keep: Set<string> = new Set()): OrderRow[] {
  let hidden = false;
  return rows.filter((row) => {
    if (row.kind === "separator") {
      hidden = row.sep.collapsed === true;
      return true;
    }
    return !hidden || keep.has(row.id);
  });
}

/** Mod count per separator id: the mods between it and the next separator. */
export function sectionCounts(rows: OrderRow[]): Map<string, number> {
  const counts = new Map<string, number>();
  let current: string | null = null;
  for (const row of rows) {
    if (row.kind === "separator") {
      current = row.sep.id;
      counts.set(current, 0);
    } else if (current !== null) {
      counts.set(current, (counts.get(current) ?? 0) + 1);
    }
  }
  return counts;
}

/** Keys that actually move: selected rows minus locked mods (as moveRows treats them). */
function movingKeys(rows: OrderRow[], keys: Iterable<string>, locked: Set<string>): Set<string> {
  const wanted = new Set(keys);
  const out = new Set<string>();
  for (const r of rows) {
    const k = rowKey(r);
    if (wanted.has(k) && !(r.kind === "mod" && locked.has(r.id))) out.add(k);
  }
  return out;
}

function restIndex(rows: OrderRow[], moving: Set<string>, key: string): number {
  return rows.filter((r) => !moving.has(rowKey(r))).findIndex((r) => rowKey(r) === key);
}

/**
 * moveRows target for moving `keys` one visible step up (-1) or down (+1): past the nearest
 * visible row that is neither moving nor a locked mod. Null when there is nowhere to go.
 */
export function stepTarget(
  rows: OrderRow[],
  keys: Iterable<string>,
  visibleKeys: string[],
  dir: -1 | 1,
  locked: Set<string>,
): number | null {
  const moving = movingKeys(rows, keys, locked);
  if (moving.size === 0) return null;
  const positions = visibleKeys.flatMap((k, i) => (moving.has(k) ? [i] : []));
  if (positions.length === 0) return null;
  const isLocked = (k: string) => locked.has(k) && rows.some((r) => r.kind === "mod" && r.id === k);
  const start = dir > 0 ? Math.max(...positions) : Math.min(...positions);
  for (let i = start + dir; i >= 0 && i < visibleKeys.length; i += dir) {
    const k = visibleKeys[i];
    if (moving.has(k) || isLocked(k)) continue;
    const at = restIndex(rows, moving, k);
    return dir > 0 ? at + 1 : at;
  }
  return null;
}

/**
 * moveRows target after a drag: `finalVisible` is the visible key order with `dragged` at its
 * drop position. The moving block lands right after the nearest non-moving row above it.
 */
export function dropTarget(
  rows: OrderRow[],
  keys: Iterable<string>,
  finalVisible: string[],
  dragged: string,
  locked: Set<string>,
): number {
  const moving = movingKeys(rows, keys, locked);
  const list = finalVisible.filter((k) => k === dragged || !moving.has(k));
  const at = list.indexOf(dragged);
  for (let i = at - 1; i >= 0; i--) {
    if (!moving.has(list[i])) return restIndex(rows, moving, list[i]) + 1;
  }
  for (let i = at + 1; i < list.length; i++) {
    if (!moving.has(list[i])) return restIndex(rows, moving, list[i]);
  }
  return 0;
}
