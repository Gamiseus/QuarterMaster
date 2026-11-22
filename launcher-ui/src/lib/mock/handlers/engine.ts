import type {
  EngineCheck,
  EngineDiagnostic,
  EngineEntry,
  EngineFile,
  EngineFileKind,
  EngineLint,
  EngineModKind,
  EngineProject,
  EngineSymbol,
  EngineWriteRequest,
} from "../../bindings";
import qmSchema from "../../../../../schemas/qm.schema.json";
import { emitMock } from "../../events";
import { mods, settings } from "../fixtures";
import { fail, log, state, type Handlers } from "../state";

/** A workspace mod's files: text by path, `null` for an empty folder. */
export type Folder = Map<string, string | null>;

const README = "# Field Radio\n\nCalls in artillery from the field radio.\n";
/** TOML's multi-line string quotes. */
const TRIPLE = "'''";

function seed(): Map<string, Folder> {
  return new Map<string, Folder>([
    [
      "FieldRadio",
      new Map<string, string | null>([
        [
          "mod.json",
          '{\n  "id": "field_radio",\n  "name": "Field Radio",\n  "version": "0.2.0",\n  "authors": ["You"],\n  "description": "Calls in artillery from the field radio."\n}\n',
        ],
        [
          "qm.toml",
          [
            "[manifest]",
            'version = "0.2.0"',
            "",
            "[[patches]]",
            "[patches.pattern]",
            'id = "field_radio:call"',
            'name = "Log radio calls"',
            'target = "main.lua"',
            'pattern = "function callArtillery(x, y)"',
            'position = "after"',
            `payload = ${TRIPLE}`,
            '    QM.log("radio call")',
            TRIPLE,
            "",
            "[[patches]]",
            "[patches.pattern]",
            'id = "field_radio:range"',
            'name = "Longer radio range"',
            'target = "includes/radio.lua"',
            'pattern = "local RANGE = 400"',
            'position = "at"',
            'payload = "local RANGE = 650"',
            "",
          ].join("\n"),
        ],
        ["includes/radio.lua", "local Radio = {}\nlocal RANGE = 400\n\nfunction Radio.call(target)\n    return target\nend\n\nreturn Radio\n"],
        ["README.md", README],
        ["assets", null],
      ]),
    ],
    [
      "SquadSettings",
      new Map<string, string | null>([
        ["mod.json", '{\n  "id": "squad_settings",\n  "name": "Squad Settings",\n  "version": "1.0.0",\n  "authors": ["You"]\n}\n'],
        [
          "config.toml",
          'id = "squad_settings"\nname = "Squad Settings"\n\n[[options]]\nid = "squad_size"\nlabel = "Squad size"\ntype = "slider"\nmin = 4\nmax = 12\ndefault = 8\n',
        ],
        ["qm.toml", '[manifest]\nversion = "1.0.0"\n\n[[patches]]\n[patches.module]\nid = "squad_settings:main"\nsource = "main.lua"\nname = "includes/squad_settings"\n'],
        ["main.lua", "local size = modSettings.squad_settings.squad_size\nreturn size\n"],
      ]),
    ],
    [
      "EasternFront",
      new Map<string, string | null>([
        ["mod.json", '{\n  "id": "eastern_front",\n  "name": "Eastern Front",\n  "version": "0.1.0",\n  "authors": ["You"]\n}\n'],
        ["faction.toml", 'schema = 1\nid = "eastern_front"\nname = "Eastern Front"\nreplaces = "allies"\n'],
      ]),
    ],
    ["Scratch", new Map<string, string | null>()],
  ]);
}

let workspace: Map<string, Folder> | null = null;
const lastChecks = new Map<string, { errors: number; warnings: number }>();

function ws(): Map<string, Folder> {
  if (!settings.workspace_mods_dir) fail("not_found", "The workspace mods folder is not set.");
  if (!workspace) {
    workspace = seed();
    if (state.init.spriteMods) for (const [folder, files] of spriteMods()) workspace.set(folder, files);
  }
  return workspace;
}

/** The sprite line-up's mods (`__qmMockInit.spriteMods`); an image's text stands for its bytes. */
function spriteMods(): [string, Folder][] {
  return [
    [
      "NewUniforms",
      new Map<string, string | null>([
        ["mod.json", '{\n  "id": "new_uniforms",\n  "name": "New Uniforms",\n  "version": "1.0.0",\n  "authors": ["You"]\n}\n'],
        ["assets/sprites/allied_fire.png", "PNG 64x40"],
        ["assets/sprites/allied_walk1.png", "PNG 64x40"],
        ["assets/sprites/allied_walk2.png", "PNG 64x40"],
        ["sketches/idea.png", "PNG 20x20"],
      ]),
    ],
    [
      "RedArmy",
      new Map<string, string | null>([
        ["mod.json", '{\n  "id": "red_army",\n  "name": "Red Army",\n  "version": "0.1.0"\n}\n'],
        ["faction.toml", 'schema = 1\nid = "red_army"\nname = "Red Army"\nreplaces = "allied"\n\n[units.base]\nframes = { fire = "art/fire.png" }\n'],
        ["art/fire.png", "PNG 70x35"],
      ]),
    ],
  ];
}

function project(folder: string): Folder {
  return ws().get(folder) ?? fail("not_found", `No mod folder '${folder}' in the workspace.`);
}

function safePath(path: string): string {
  const p = path.replace(/\\/g, "/");
  if (!p || p.startsWith("/") || p.includes(":") || p.split("/").some((x) => x === ".." || x === "." || x === "")) {
    fail("invalid_input", `'${path}' is not a path inside the mod.`);
  }
  return p;
}

/** FNV-1a: a stand-in for the backend's SHA-256 of the bytes on disk. */
function hash(text: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
}

function kindOf(path: string): EngineFileKind {
  const ext = path.toLowerCase().split(".").pop() ?? "";
  if (ext === "lua") return "lua";
  if (ext === "toml") return "toml";
  if (ext === "json") return "json";
  if (ext === "md") return "markdown";
  if (["txt", "cfg", "ini", "csv"].includes(ext)) return "text";
  if (["png", "jpg", "jpeg", "bmp", "gif", "webp", "tga"].includes(ext)) return "image";
  if (["ogg", "wav", "mp3", "flac"].includes(ext)) return "audio";
  return "other";
}

/** Every folder a mod has: the empty ones and those its files are in. */
function folders(f: Folder): Set<string> {
  const out = new Set<string>();
  for (const [p, text] of f) {
    const parts = p.split("/");
    for (let i = 1; i < parts.length; i++) out.add(parts.slice(0, i).join("/"));
    if (text === null) out.add(p);
  }
  return out;
}

function manifestOf(f: Folder): { id?: string; name?: string; version?: string } | null {
  const text = f.get("mod.json");
  if (!text) return null;
  try {
    return JSON.parse(text) as { id?: string; name?: string; version?: string };
  } catch {
    return null;
  }
}

function modKind(f: Folder): EngineModKind {
  if (f.has("faction.toml")) return "faction";
  // Like the backend: patch files, or only a mod.json.
  if (f.has("qm.toml") || f.has("mod.json") || [...f.keys()].some((p) => p.startsWith("qm/") && p.endsWith(".toml"))) return "patch";
  if ([...f.keys()].some((p) => p.startsWith("assets/") && f.get(p) !== null)) return "assets";
  return "unmanaged";
}

function diag(path: string, line: number | null, severity: EngineDiagnostic["severity"], message: string, source: string): EngineDiagnostic {
  return { path, line, column: line === null ? null : 1, end_line: null, end_column: null, severity, message, source };
}

type SchemaNode = { properties?: Record<string, SchemaNode>; items?: SchemaNode };

/** Every key a patch file's tables take, from the published schema. */
const PATCH_KEYS = new Set<string>();
(function collect(node: SchemaNode | undefined) {
  for (const [k, v] of Object.entries(node?.properties ?? {})) {
    PATCH_KEYS.add(k);
    collect(v.items ?? v);
  }
})(qmSchema as unknown as SchemaNode);

/** A small stand-in for the backend's checks: TOML lines, JSON syntax and Lua brackets. */
function check(path: string, text: string): EngineCheck {
  const out: EngineDiagnostic[] = [];
  const kind = kindOf(path);
  if (kind === "toml") {
    const patchFile = path === "qm.toml" || path.startsWith("qm/");
    let inString = false;
    text.split("\n").forEach((raw, i) => {
      const line = raw.trim();
      // Inside a multi-line string every line is text.
      const quotes = line.split(TRIPLE).length - 1;
      if (inString) {
        if (quotes % 2 === 1) inString = false;
        return;
      }
      if (quotes % 2 === 1) inString = true;
      if (!line || line.startsWith("#") || /^\[\[?[^\]]+\]\]?$/.test(line)) return;
      if (!/^[A-Za-z0-9_."-]+\s*=\s*\S/.test(line)) {
        out.push(diag(path, i + 1, "error", line.includes("=") ? "A value is missing after `=`." : "Expected `key = value` on this line.", "toml"));
        return;
      }
      const key = /^([A-Za-z_]+)\s*=/.exec(line)?.[1];
      if (patchFile && key && !PATCH_KEYS.has(key)) {
        out.push(diag(path, i + 1, "warning", `Unknown key \`${key}\`: the loader ignores it.`, "patches"));
      }
    });
  } else if (kind === "json") {
    try {
      JSON.parse(text);
    } catch (e) {
      out.push(diag(path, 1, "error", `Not valid JSON: ${e instanceof Error ? e.message : String(e)}`, "json"));
    }
  } else if (kind === "lua" && !state.init.engineNoLua) {
    const opens = (text.match(/\(/g) ?? []).length;
    const closes = (text.match(/\)/g) ?? []).length;
    if (opens !== closes) {
      const lines = text.split("\n");
      out.push(diag(path, Math.max(1, lines.length - (lines.at(-1) === "" ? 1 : 0)), "error", "')' expected near the end of the file.", "lua"));
    }
  }
  return { diagnostics: out, lua_checked: kind !== "lua" || !state.init.engineNoLua };
}

function lint(folder: string): EngineLint {
  const f = project(folder);
  const diagnostics: EngineDiagnostic[] = [];
  for (const [p, text] of f) if (text !== null) diagnostics.push(...check(p, text).diagnostics);
  let patchCount = 0;
  let zero = 0;
  for (const [p, text] of f) {
    if (text === null || !(p === "qm.toml" || (p.startsWith("qm/") && p.endsWith(".toml")))) continue;
    const lines = text.split("\n");
    let current: { id: string; name: string; line: number } | null = null;
    lines.forEach((l, i) => {
      if (/^\s*\[\[patches\]\]/.test(l)) {
        patchCount++;
        current = { id: "", name: "", line: i + 1 };
      }
      const id = /^\s*id\s*=\s*"([^"]*)"/.exec(l)?.[1];
      const name = /^\s*name\s*=\s*"([^"]*)"/.exec(l)?.[1];
      if (current && id) current.id = id;
      if (current && name) current.name = name;
      if (current && /^\s*pattern\s*=.*NOT_IN_GAME/.test(l)) {
        zero++;
        diagnostics.push(diag(p, current.line, "warning", `Patch '${current.name}' (${current.id}) had 0 matches: its pattern is not in the game's code.`, "lint"));
      }
    });
  }
  const errors = diagnostics.filter((d) => d.severity === "error").length;
  const warnings = diagnostics.filter((d) => d.severity === "warning").length;
  lastChecks.set(folder, { errors, warnings });
  return {
    passed: errors === 0,
    diagnostics,
    patch_count: patchCount,
    matched_patches: patchCount - zero,
    zero_match_patches: zero,
    capabilities: [],
    game_checked: true,
  };
}

function file(folder: string, path: string): EngineFile {
  const p = safePath(path);
  const text = project(folder).get(p);
  if (text === undefined) fail("not_found", `No file '${p}' in ${folder}.`);
  if (text === null) fail("invalid_input", `'${p}' is a folder.`);
  const k = kindOf(p);
  if (k === "image" || k === "audio") fail("invalid_input", `'${p}' is not a text file.`);
  return { path: p, text, hash: hash(text), crlf: false, bom: false };
}

/** Changes a file as another program would, and sends the event the folder watcher would. */
export function writeFromOutside(folder: string, path: string, text: string): void {
  project(folder).set(path, text);
  emitMock({ type: "workspace_changed", folder, path });
}

declare global {
  interface Window {
    __qmMockEngineWrite?: typeof writeFromOutside;
  }
}

export const handlers: Handlers = {
  engine_projects: () => {
    if (!settings.workspace_mods_dir) return { workspace_set: false, projects: [] };
    const projects: EngineProject[] = [...ws()].map(([folder, f]) => {
      const m = manifestOf(f);
      return {
        folder,
        name: m?.name ?? folder,
        id: m?.id ?? null,
        version: m?.version ?? null,
        kind: modKind(f),
        in_library: m?.id ? mods.some((x) => x.id === m.id) : false,
        last_check: lastChecks.get(folder) ?? null,
      };
    });
    projects.sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
    return { workspace_set: true, projects };
  },
  engine_library_mods: () => mods.map((m) => ({ id: m.id, name: m.name, version: m.version })),
  engine_tree: (args) => {
    const f = project(String(args.folder));
    const entries: EngineEntry[] = [...folders(f)].map((p) => ({ path: p, folder: true, size_bytes: 0, kind: "other" }));
    for (const [p, text] of f) if (text !== null) entries.push({ path: p, folder: false, size_bytes: text.length, kind: kindOf(p) });
    entries.sort((a, b) => a.path.toLowerCase().localeCompare(b.path.toLowerCase()));
    return { entries, truncated: false };
  },
  engine_file_read: (args) => file(String(args.folder), String(args.path)),
  engine_file_write: (args) => {
    const folder = String(args.folder);
    const req = args.req as EngineWriteRequest;
    const p = safePath(req.path);
    const f = project(folder);
    const now = f.get(p);
    if (now === null) fail("invalid_input", `'${p}' is a folder.`);
    if (req.expected_hash !== null) {
      if (now === undefined) fail("conflict", `'${p}' was deleted or moved since it was opened.`);
      if (hash(now) !== req.expected_hash) fail("conflict", `'${p}' changed on disk since it was opened.`);
    }
    f.set(p, req.text);
    return { path: p, text: req.text, hash: hash(req.text), crlf: req.crlf, bom: req.bom };
  },
  engine_file_create: (args) => {
    const f = project(String(args.folder));
    const p = safePath(String(args.path));
    if (f.has(p) || folders(f).has(p)) fail("conflict", `'${p}' already exists.`);
    f.set(p, args.isFolder ? null : "");
    return null;
  },
  engine_file_rename: (args) => {
    const folder = String(args.folder);
    const f = project(folder);
    const from = safePath(String(args.from));
    const to = safePath(String(args.to));
    const isFolder = folders(f).has(from);
    if (!f.has(from) && !isFolder) fail("not_found", `No file '${from}' in ${folder}.`);
    if (to.startsWith(`${from}/`)) fail("invalid_input", `'${from}' cannot move into itself.`);
    if ((f.has(to) || folders(f).has(to)) && from.toLowerCase() !== to.toLowerCase()) fail("conflict", `'${to}' already exists.`);
    for (const [p, text] of [...f]) {
      if (p === from || p.startsWith(`${from}/`)) {
        f.delete(p);
        f.set(to + p.slice(from.length), text);
      }
    }
    return null;
  },
  engine_file_delete: (args) => {
    const folder = String(args.folder);
    const f = project(folder);
    const p = safePath(String(args.path));
    if (!f.has(p) && !folders(f).has(p)) fail("not_found", `No file '${p}' in ${folder}.`);
    for (const k of [...f.keys()]) if (k === p || k.startsWith(`${p}/`)) f.delete(k);
    // A folder that held only the removed item stays, as it would on disk.
    const parent = p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : "";
    if (parent && !folders(f).has(parent)) f.set(parent, null);
    return null;
  },
  engine_check: (args) => {
    project(String(args.folder));
    return check(safePath(String(args.path)), String(args.text));
  },
  engine_lint: (args) => lint(String(args.folder)),
  engine_symbols: (): EngineSymbol[] => [
    { name: "Radio.call", kind: "function", signature: "function(target)", file: "includes/radio.lua", line: 3 },
    { name: "Squad", kind: "table", signature: "", file: "includes/squad.lua", line: 1 },
    { name: "Squad.spawn", kind: "function", signature: "function(kind, x, y)", file: "includes/squad.lua", line: 12 },
    { name: "spawnUnit", kind: "function", signature: "function(kind, x, y, side)", file: "includes/units.lua", line: 40 },
  ],
  engine_copy_to_workspace: (args) => {
    const m = mods.find((x) => x.id === args.modId && x.version === args.version) ?? fail("not_found", `Mod '${String(args.modId)}' ${String(args.version)} is not installed.`);
    const base = m.name.replace(/[^A-Za-z0-9 _-]/g, "").replace(/\s+/g, "");
    let folder = base || m.id;
    for (let n = 2; ws().has(folder); n++) folder = `${base}-${n}`;
    ws().set(
      folder,
      new Map<string, string | null>([
        ["mod.json", `{\n  "id": "${m.id}",\n  "name": "${m.name}",\n  "version": "${m.version}"\n}\n`],
        ["qm.toml", `[manifest]\nversion = "${m.version}"\n\n[[patches]]\n[patches.module]\nid = "${m.id}:main"\nsource = "main.lua"\nname = "includes/${m.id}"\n`],
        ["main.lua", "local M = {}\n\nreturn M\n"],
      ]),
    );
    return folder;
  },
  engine_reveal: (args) => {
    project(String(args.folder));
    log.engineReveals.push(String(args.folder));
    return null;
  },
};

/** The mock workspace: folder name to files (throws when no workspace is set). */
export function mockWorkspace(): Map<string, Folder> {
  return ws();
}

/** A workspace file's text, for the Patch Studio's mock (undefined when there is none). */
export function mockFileText(folder: string, path: string): string | undefined {
  const t = project(folder).get(path);
  return t === null ? undefined : t;
}

/** Writes a workspace file as the backend would, and sends the event the watcher would. */
export function mockWriteFile(folder: string, path: string, text: string): string {
  project(folder).set(path, text);
  emitMock({ type: "workspace_changed", folder, path });
  return hash(text);
}

export function mockHash(text: string): string {
  return hash(text);
}
