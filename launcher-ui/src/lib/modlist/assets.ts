import type { AssetFile } from "../bindings";

export type AssetGroup = {
  /** Folder under `assets/` (`sounds/vo`), or "" for files directly in it. */
  folder: string;
  files: { name: string; path: string; overrides: boolean | null }[];
};

/** Asset files grouped by folder, in path order (the backend already sorts by path). */
export function groupAssetFiles(files: AssetFile[]): AssetGroup[] {
  const groups = new Map<string, AssetGroup>();
  for (const f of files) {
    const rel = f.path.replace(/^assets\//, "");
    const cut = rel.lastIndexOf("/");
    const folder = cut === -1 ? "" : rel.slice(0, cut);
    const name = cut === -1 ? rel : rel.slice(cut + 1);
    let g = groups.get(folder);
    if (!g) {
      g = { folder, files: [] };
      groups.set(folder, g);
    }
    g.files.push({ name, path: f.path, overrides: f.overrides_vanilla });
  }
  return [...groups.values()];
}

/** "12 files: 8 override the game, 4 are new." (or just the count while overrides are unknown). */
export function assetSummary(files: AssetFile[]): string {
  const n = files.length;
  const count = `${n} ${n === 1 ? "file" : "files"}`;
  if (n === 0 || files.some((f) => f.overrides_vanilla === null)) return `${count}.`;
  const over = files.filter((f) => f.overrides_vanilla).length;
  const added = n - over;
  const parts = [];
  if (over > 0) parts.push(`${over} ${over === 1 ? "overrides" : "override"} the game`);
  if (added > 0) parts.push(`${added} ${added === 1 ? "is" : "are"} new`);
  return `${count}: ${parts.join(", ")}.`;
}
