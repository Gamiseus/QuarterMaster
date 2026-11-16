import type {
  LoadOrderResolution,
  LoadOrderUpdate,
  ModProfile,
  ModSummary,
  ProfileSeparator,
} from "../bindings";

export type OrderRow =
  | { kind: "mod"; id: string }
  | { kind: "separator"; sep: ProfileSeparator };

/** Selection key of a row: the mod id, or `sep:<id>` for separators. */
export function rowKey(row: OrderRow): string {
  return row.kind === "mod" ? row.id : `sep:${row.sep.id}`;
}

/**
 * Mods ascending by profile priority (falling back to the manifest priority), ties by id.
 * Separators sit before the row they point at; unresolved ones go last in array order.
 * Enabled state is deliberately not an input, so toggling a mod never moves it.
 */
export function buildOrder(mods: ModSummary[], profile: ModProfile): OrderRow[] {
  const priorities = profile.mod_priorities ?? {};
  const sorted = [...mods].sort((a, b) => {
    const pa = priorities[a.id] ?? a.load_priority;
    const pb = priorities[b.id] ?? b.load_priority;
    if (pa !== pb) return pa - pb;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });
  const known = new Set(sorted.map((m) => m.id));
  const before = new Map<string, ProfileSeparator[]>();
  const orphans: ProfileSeparator[] = [];
  for (const sep of profile.separators ?? []) {
    const target = sep.insert_before_mod_id;
    if (target !== null && known.has(target)) {
      const list = before.get(target) ?? [];
      list.push(sep);
      before.set(target, list);
    } else {
      orphans.push(sep);
    }
  }
  const rows: OrderRow[] = [];
  for (const mod of sorted) {
    for (const sep of before.get(mod.id) ?? []) rows.push({ kind: "separator", sep });
    rows.push({ kind: "mod", id: mod.id });
  }
  for (const sep of orphans) rows.push({ kind: "separator", sep });
  return rows;
}

/**
 * Move the selected rows (relative order kept) so the block starts at `toIndex` of the list
 * without them. Locked mod rows then snap back to their original absolute index and every
 * other row flows around them. Locked ids in the selection are ignored. Pure.
 */
export function moveRows(
  rows: OrderRow[],
  selectedIds: string[],
  toIndex: number,
  locked: Set<string>,
): OrderRow[] {
  const isLocked = (r: OrderRow) => r.kind === "mod" && locked.has(r.id);
  const selected = new Set(selectedIds);
  const moving = rows.filter((r) => selected.has(rowKey(r)) && !isLocked(r));
  if (moving.length === 0) return [...rows];
  const movingSet = new Set(moving);

  // 1. Remove the block and re-insert it at toIndex (clamped) in the remainder.
  const rest = rows.filter((r) => !movingSet.has(r));
  const at = Math.max(0, Math.min(toIndex, rest.length));
  const moved = [...rest.slice(0, at), ...moving, ...rest.slice(at)];

  // 2. Pull locked rows out and put each back at its original absolute index, lowest first.
  const lockedAt = rows.map((r, i) => ({ r, i })).filter(({ r }) => isLocked(r));
  if (lockedAt.length === 0) return moved;
  const lockedSet = new Set(lockedAt.map(({ r }) => r));
  const result = moved.filter((r) => !lockedSet.has(r));
  for (const { r, i } of lockedAt) {
    result.splice(Math.min(i, result.length), 0, r);
  }
  return result;
}

/** Persistable form of the current row order: priority = (mod position + 1) * 10. */
export function toLoadOrderUpdate(rows: OrderRow[], profile: ModProfile): LoadOrderUpdate {
  const mod_priorities: Partial<Record<string, number>> = {};
  let position = 0;
  for (const row of rows) {
    if (row.kind === "mod") {
      position += 1;
      mod_priorities[row.id] = position * 10;
    }
  }
  const separators: ProfileSeparator[] = [];
  rows.forEach((row, i) => {
    if (row.kind !== "separator") return;
    let next: string | null = null;
    for (let j = i + 1; j < rows.length; j++) {
      const candidate = rows[j];
      if (candidate.kind === "mod") {
        next = candidate.id;
        break;
      }
    }
    separators.push({ ...row.sep, insert_before_mod_id: next });
  });
  const loadAfter: Partial<Record<string, string[]>> = {};
  for (const [k, v] of Object.entries(profile.user_load_after ?? {})) {
    if (v) loadAfter[k] = [...v];
  }
  return {
    mod_priorities,
    user_load_after: loadAfter,
    locked_mods: [...(profile.locked_mods ?? [])],
    separators,
  };
}

export function resolvedPosition(
  resolution: LoadOrderResolution,
): Map<string, { index: number; reasons: string[] }> {
  const map = new Map<string, { index: number; reasons: string[] }>();
  for (const entry of resolution.ordered_mods) {
    map.set(entry.mod_id, { index: entry.order_index, reasons: entry.reasons });
  }
  return map;
}

/**
 * Enabled mods (those present in the resolution) the solver displaced. Taking solver rank in
 * the user's order, the longest increasing subsequence is the set of mods that kept their
 * relative order; everything outside it is reported. One displaced mod therefore flags only
 * itself. When several subsequences tie, the one found by patience sorting with leftmost
 * replacement is used, which is deterministic for a given input.
 */
export function movedBySolver(rows: OrderRow[], resolution: LoadOrderResolution): Set<string> {
  const solverRank = new Map<string, number>();
  [...resolution.ordered_mods]
    .sort((a, b) => a.order_index - b.order_index)
    .forEach((e, i) => solverRank.set(e.mod_id, i));
  const userOrder = rows.flatMap((r) => (r.kind === "mod" && solverRank.has(r.id) ? [r.id] : []));
  const seq = userOrder.map((id) => solverRank.get(id) as number);

  // Patience sorting: tails[k] = index in seq of the smallest tail of an increasing run of length k+1.
  const tails: number[] = [];
  const prev: number[] = new Array(seq.length).fill(-1);
  seq.forEach((value, i) => {
    let lo = 0;
    let hi = tails.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (seq[tails[mid]] < value) lo = mid + 1;
      else hi = mid;
    }
    if (lo > 0) prev[i] = tails[lo - 1];
    tails[lo] = i;
  });
  const keep = new Set<number>();
  for (let i = tails.length > 0 ? tails[tails.length - 1] : -1; i >= 0; i = prev[i]) keep.add(i);

  const moved = new Set<string>();
  userOrder.forEach((id, i) => {
    if (!keep.has(i)) moved.add(id);
  });
  return moved;
}
