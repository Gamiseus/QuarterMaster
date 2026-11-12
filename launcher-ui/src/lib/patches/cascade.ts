import type { PatchDetail } from "../bindings";

export type FetchDetail = (id: string) => Promise<PatchDetail | undefined>;

/** Breadth-first from `rootId` along `next`, keeping the patches `wanted` accepts. */
async function walk(
  rootId: string,
  fetch: FetchDetail,
  next: (d: PatchDetail) => string[],
  wanted: (d: PatchDetail) => boolean,
): Promise<PatchDetail[]> {
  const root = await fetch(rootId).catch(() => undefined);
  if (!root) return [];
  const seen = new Set([rootId, root.patch_id]);
  const out: PatchDetail[] = [];
  const queue = [root];
  for (let d = queue.shift(); d; d = queue.shift()) {
    for (const id of next(d)) {
      if (seen.has(id)) continue;
      seen.add(id);
      const n = await fetch(id).catch(() => undefined);
      if (!n || (n.patch_id !== id && seen.has(n.patch_id))) continue;
      seen.add(n.patch_id);
      if (wanted(n)) out.push(n);
      queue.push(n);
    }
  }
  return out;
}

/** Every enabled patch that needs `rootId`, directly or through another patch, nearest first. */
export function dependentsToDisable(rootId: string, fetch: FetchDetail): Promise<PatchDetail[]> {
  return walk(rootId, fetch, (d) => d.required_by, (d) => !d.disabled);
}

/** Every disabled patch `rootId` needs, directly or through another patch, nearest first. */
export function requirementsToEnable(rootId: string, fetch: FetchDetail): Promise<PatchDetail[]> {
  return walk(rootId, fetch, (d) => d.requires, (d) => d.disabled);
}
