import type { ModFile } from "../bindings";

export type FileKind = "code" | "image" | "sound" | "text" | "other";

/** One visible line of a mod's file tree. */
export type FileRow =
  | { type: "folder"; path: string; name: string; depth: number; files: number; size: number; open: boolean }
  | { type: "file"; path: string; name: string; depth: number; size: number; kind: FileKind };

type Folder = { path: string; name: string; folders: Map<string, Folder>; files: ModFile[]; count: number; size: number };

const KINDS: Record<string, FileKind> = {
  lua: "code", glsl: "code", frag: "code", vert: "code",
  png: "image", jpg: "image", jpeg: "image", gif: "image", bmp: "image", webp: "image",
  ogg: "sound", wav: "sound", mp3: "sound", flac: "sound",
  toml: "text", json: "text", md: "text", txt: "text", csv: "text", xml: "text", ini: "text", cfg: "text", fnt: "text",
};

export function fileKind(path: string): FileKind {
  const dot = path.lastIndexOf(".");
  return (dot === -1 ? undefined : KINDS[path.slice(dot + 1).toLowerCase()]) ?? "other";
}

const byName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });

function tree(files: ModFile[]): Folder {
  const root: Folder = { path: "", name: "", folders: new Map(), files: [], count: 0, size: 0 };
  for (const f of files) {
    const parts = f.path.split("/");
    let at = root;
    at.count += 1;
    at.size += f.size_bytes;
    for (let i = 0; i < parts.length - 1; i++) {
      const name = parts[i];
      let next = at.folders.get(name);
      if (!next) {
        next = { path: at.path ? `${at.path}/${name}` : name, name, folders: new Map(), files: [], count: 0, size: 0 };
        at.folders.set(name, next);
      }
      next.count += 1;
      next.size += f.size_bytes;
      at = next;
    }
    at.files.push(f);
  }
  return root;
}

/** Every folder path in `files` (`assets`, `assets/sprites`, ...). */
export function folderPaths(files: ModFile[]): string[] {
  const out = new Set<string>();
  for (const f of files) {
    let cut = f.path.lastIndexOf("/");
    while (cut !== -1) {
      const dir = f.path.slice(0, cut);
      if (out.has(dir)) break;
      out.add(dir);
      cut = dir.lastIndexOf("/");
    }
  }
  return [...out];
}

/**
 * The lines to show: folders first, then files, each in name order; a closed folder hides
 * what is in it. A folder that only holds one folder is shown joined with it (`assets/sprites`).
 */
export function fileRows(files: ModFile[], open: ReadonlySet<string>): FileRow[] {
  const rows: FileRow[] = [];
  const walk = (dir: Folder, depth: number) => {
    for (const first of [...dir.folders.values()].sort((a, b) => byName(a.name, b.name))) {
      let f = first;
      let name = f.name;
      while (f.files.length === 0 && f.folders.size === 1) {
        f = [...f.folders.values()][0];
        name = `${name}/${f.name}`;
      }
      const isOpen = open.has(f.path);
      rows.push({ type: "folder", path: f.path, name, depth, files: f.count, size: f.size, open: isOpen });
      if (isOpen) walk(f, depth + 1);
    }
    for (const f of [...dir.files].sort((a, b) => byName(a.path, b.path))) {
      rows.push({ type: "file", path: f.path, name: f.path.slice(f.path.lastIndexOf("/") + 1), depth, size: f.size_bytes, kind: fileKind(f.path) });
    }
  };
  walk(tree(files), 0);
  return rows;
}

/** The files whose path has `query` in it, as a flat list with full paths for names. */
export function filterFiles(files: ModFile[], query: string): FileRow[] {
  const q = query.trim().toLowerCase();
  return files
    .filter((f) => f.path.toLowerCase().includes(q))
    .map((f) => ({ type: "file" as const, path: f.path, name: f.path, depth: 0, size: f.size_bytes, kind: fileKind(f.path) }));
}

/** "42 files, 1.3 MB" parts: how many of each kind, most first (`12 code`, `30 images`). */
export function kindCounts(files: ModFile[]): { kind: FileKind; count: number }[] {
  const counts = new Map<FileKind, number>();
  for (const f of files) {
    const k = fileKind(f.path);
    counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  return [...counts.entries()].map(([kind, count]) => ({ kind, count })).sort((a, b) => b.count - a.count);
}
