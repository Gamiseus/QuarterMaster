import type { EngineEntry } from "../bindings";

export interface TreeRow {
  entry: EngineEntry;
  name: string;
  depth: number;
}

/** The folder a path is in ("" for the mod's root). */
export function parentOf(path: string): string {
  const i = path.lastIndexOf("/");
  return i < 0 ? "" : path.slice(0, i);
}

/**
 * The rows of a file tree in display order: folders before files at each level, by name,
 * without the contents of a collapsed folder.
 */
export function treeRows(entries: EngineEntry[], collapsed: Set<string>): TreeRow[] {
  const children = new Map<string, EngineEntry[]>();
  for (const e of entries) {
    const p = parentOf(e.path);
    const list = children.get(p) ?? [];
    list.push(e);
    children.set(p, list);
  }
  const rows: TreeRow[] = [];
  const visit = (folder: string, depth: number) => {
    const list = [...(children.get(folder) ?? [])].sort((a, b) => {
      if (a.folder !== b.folder) return a.folder ? -1 : 1;
      return a.path.toLowerCase().localeCompare(b.path.toLowerCase());
    });
    for (const e of list) {
      rows.push({ entry: e, name: e.path.slice(folder ? folder.length + 1 : 0), depth });
      if (e.folder && !collapsed.has(e.path)) visit(e.path, depth + 1);
    }
  };
  visit("", 0);
  return rows;
}

/** A name for a new file or folder: no slashes at the ends, no empty or dot-only parts. */
export function cleanName(name: string): string | null {
  const parts = name
    .replace(/\\/g, "/")
    .split("/")
    .map((p) => p.trim())
    .filter((p) => p.length > 0);
  if (parts.length === 0 || parts.some((p) => p === "." || p === "..")) return null;
  return parts.join("/");
}
