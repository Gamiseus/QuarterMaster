import type { TemplateAddPreview, TemplateChange, TemplateFileView, TemplateInfo } from "../../bindings";
import { emitMock } from "../../events";
import { fail, log, type Handlers } from "../state";
import { mockWorkspace, type Folder } from "./engine";

// The real templates, so the mock's catalog is the one the launcher ships.
const META = import.meta.glob("../../../../../templates/*/template.toml", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const TEXT = import.meta.glob("../../../../../templates/*/**/*.{lua,toml,json,md,txt}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;
const ALL = Object.keys(import.meta.glob("../../../../../templates/*/**/*", { query: "?url", import: "default" }));
const PREFIX = "../../../../../templates/";

/** The site's page for each wiki part (as `WIKI_PAGES` in qm-manager). */
const WIKI_PAGES = [
  "what-a-mod-is",
  "replacing-art-and-sound",
  "faction-mods",
  "changing-game-code",
  "adding-your-own-lua-files",
  "settings-keybinds-and-menus",
  "working-with-other-mods",
  "game-reference",
  "tools",
  "testing-and-debugging",
  "packaging-and-publishing",
  "the-mod-engine",
];

const PATCH_LISTS = ["patches", "pattern", "regex", "copy", "module", "overrides", "asset_choices"];
const CONFIG_LISTS = ["options", "keybinds", "modes", "pages"];

/** template.toml's keys: strings, numbers and string lists, plus `[[examples]]` tables. */
function parseMeta(text: string): { top: Record<string, unknown>; examples: Record<string, unknown>[] } {
  const top: Record<string, unknown> = {};
  const examples: Record<string, unknown>[] = [];
  let cur = top;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    if (line === "[[examples]]") {
      cur = {};
      examples.push(cur);
      continue;
    }
    const m = /^(\w+)\s*=\s*(.+)$/.exec(line);
    // TOML's basic strings, integers and string lists read as JSON.
    if (m) cur[m[1]] = JSON.parse(m[2]);
  }
  return { top, examples };
}

function anchorOf(heading: string): string {
  return [...heading.toLowerCase()]
    .map((c) => (c === " " ? "-" : /[\p{L}\p{N}_-]/u.test(c) ? c : ""))
    .join("");
}

function filesOf(id: string): string[] {
  return ALL.filter((k) => k.startsWith(`${PREFIX}${id}/`))
    .map((k) => k.slice(PREFIX.length + id.length + 1))
    .filter((p) => p !== "template.toml")
    .sort();
}

const CATALOG: TemplateInfo[] = Object.entries(META)
  .map(([key, text]) => {
    const id = key.slice(PREFIX.length).split("/")[0];
    const { top, examples } = parseMeta(text);
    const section = String(top.section);
    const anchor = typeof top.anchor === "string" && top.anchor ? top.anchor : anchorOf(section);
    const part = Number(top.part);
    return {
      id,
      name: String(top.name),
      part,
      order: Number(top.order ?? 0),
      section,
      anchor,
      wiki_url: `https://gamiseus.github.io/wiki/${WIKI_PAGES[part - 1] ?? "all"}/#${anchor}`,
      summary: String(top.summary),
      open: String(top.open),
      try_next: (top.try_next as string[] | undefined) ?? [],
      aliases: (top.aliases as string[] | undefined) ?? [],
      examples: examples.map((e) => ({ section: String(e.section), block: Number(e.block), file: String(e.file) })),
      files: filesOf(id),
    };
  })
  .sort((a, b) => a.part - b.part || a.order - b.order || a.id.localeCompare(b.id));

function find(id: string): TemplateInfo {
  return CATALOG.find((t) => t.id === id || t.aliases.includes(id)) ?? fail("not_found", `There is no template '${id}'.`);
}

interface Names {
  id: string;
  name: string;
  folder: string;
  global: string;
}

const EXAMPLE: Names = { id: "example_mod", name: "Example Mod", folder: "ExampleMod", global: "ExampleMod" };

/** As `TemplateNames::new`: a message for names that do not make a mod. */
function namesFor(rawId: string, rawName: string): Names | string {
  const id = [...rawId.trim()].map((c) => (/[A-Za-z0-9_-]/.test(c) ? c.toLowerCase() : "_")).join("");
  if (!/^[a-z]/.test(id)) return "A mod id starts with a letter and has only letters, digits, _ and -.";
  const name = rawName.trim();
  const folder = [...name].filter((c) => /[A-Za-z0-9_-]/.test(c)).join("");
  if (!folder) return "The mod's name needs at least one letter or digit.";
  let global = "";
  let upper = true;
  for (const c of name) {
    if (/[A-Za-z0-9]/.test(c)) {
      global += upper ? c.toUpperCase() : c;
      upper = false;
    } else upper = true;
  }
  if (!/^[A-Za-z]/.test(global)) global = `Mod${global}`;
  return { id, name, folder, global };
}

function apply(text: string, n: Names): string {
  if (n.id === EXAMPLE.id && n.name === EXAMPLE.name) return text;
  const camel = n.global.charAt(0).toLowerCase() + n.global.slice(1);
  return text
    .replaceAll("example_mod", n.id)
    .replaceAll("example_army", n.id.replaceAll("-", "_"))
    .replaceAll("Example Mod", n.name)
    .replaceAll("Example Army", n.name)
    .replaceAll("ExampleMod", n.global)
    .replace(/(?<![A-Za-z0-9_])example(?=[A-Z])/g, camel);
}

/** A template's files with these names: text, or "" for a binary file. */
function render(id: string, n: Names): Map<string, string> {
  const t = find(id);
  return new Map(t.files.map((p) => [apply(p, n), apply(TEXT[`${PREFIX}${t.id}/${p}`] ?? "", n)]));
}

/** The ids and module names an example's TOML lists, by entry. */
function entriesOf(text: string, lists: string[]): { label: string; id: string | null }[] {
  const out: { label: string; id: string | null }[] = [];
  let inList = false;
  for (const line of text.split("\n")) {
    const head = /^\s*\[\[?([\w.]+)\]\]?/.exec(line);
    if (head) {
      inList = lists.includes(head[1].split(".").pop() ?? "") || lists.includes(head[1].split(".")[0]);
      if (/^\s*\[\[/.test(line) && inList) out.push({ label: `a [[${head[1]}]] entry`, id: null });
      continue;
    }
    const id = /^\s*id\s*=\s*"([^"]*)"/.exec(line)?.[1];
    if (id && inList && out.length && out[out.length - 1].id === null) {
      out[out.length - 1] = { label: id, id };
    }
  }
  return out;
}

function plan(folder: string, id: string): { preview: TemplateAddPreview; writes: Map<string, string> } {
  const t = find(id);
  const mine: Folder = mockWorkspace().get(folder) ?? fail("not_found", `No mod folder '${folder}' in the workspace.`);
  const blocked: string[] = [];
  if (t.id === "empty") blocked.push("The empty template has nothing to add.");
  if (t.id === "compat-patch") blocked.push("A compatibility patch is a mod of its own: make it with New mod.");
  let manifest: { id?: string; name?: string } = {};
  try {
    manifest = JSON.parse(mine.get("mod.json") ?? "{}") as typeof manifest;
  } catch {
    // A mod.json that does not read keeps the folder's names.
  }
  const base = namesFor(manifest.id ?? "example_mod", manifest.name ?? folder);
  const names = typeof base === "string" ? EXAMPLE : base;
  const changes: TemplateChange[] = [];
  const writes = new Map<string, string>();
  for (const [rel, text] of render(t.id, names)) {
    const have = mine.get(rel);
    const exists = have !== undefined;
    if (rel === "mod.json") {
      const wanted = ((JSON.parse(text) as { qm_features?: string[] }).qm_features ?? []).filter(Boolean);
      if (!have) continue;
      const doc = JSON.parse(have) as { qm_features?: string[] };
      const added = wanted.filter((f) => !(doc.qm_features ?? []).includes(f));
      if (added.length) {
        doc.qm_features = [...(doc.qm_features ?? []), ...added];
        changes.push({ path: rel, kind: "merge", detail: `lists ${added.join(", ")} in qm_features` });
        writes.set(rel, `${JSON.stringify(doc, null, 2)}\n`);
      }
    } else if ((rel === "README.md" || rel === "CHANGELOG.md") && exists) {
      changes.push({ path: rel, kind: "skip", detail: "the mod has its own" });
    } else if ((rel === "qm.toml" || rel === "config.toml") && exists && have !== null) {
      const entries = entriesOf(text, rel === "qm.toml" ? PATCH_LISTS : CONFIG_LISTS);
      const clashes = entries.filter((e) => e.id && have.includes(`id = "${e.id}"`));
      for (const c of clashes) blocked.push(`${rel} already has ${c.id}.`);
      if (entries.length && !clashes.length) {
        const first = text.search(/^\s*\[\[/m);
        changes.push({ path: rel, kind: "merge", detail: `adds ${entries.map((e) => e.label).join(", ")}` });
        writes.set(rel, `${have.trimEnd()}\n\n# Added from the template "${t.name}".\n${text.slice(Math.max(0, first)).trimStart()}`);
      }
    } else if (rel === "faction.toml" && exists && have !== null) {
      // The faction's sections the mod lacks join it; [[weapons]] and [[vehicles]] entries always.
      const heads = [...new Set([...text.matchAll(/^\[\[?[\w.]+\]\]?/gm)].map((m) => m[0]))];
      const fresh = heads.filter((h) => h.startsWith("[[") || !have.includes(h));
      if (fresh.length) {
        changes.push({ path: rel, kind: "merge", detail: `adds ${fresh.join(", ")}` });
        writes.set(rel, `${have.trimEnd()}\n\n# Added from the template "${t.name}".\n${text.slice(Math.max(0, text.search(/^\[/m)))}`);
      }
    } else if (exists && have === text) {
      changes.push({ path: rel, kind: "skip", detail: "the mod has the same file" });
    } else if (exists) {
      blocked.push(`The mod already has ${rel}.`);
    } else {
      changes.push({ path: rel, kind: "add", detail: "a new file" });
      writes.set(rel, text);
    }
  }
  if (!blocked.length && writes.size === 0) blocked.push("The mod already has everything this example adds.");
  return { preview: { changes, blocked }, writes };
}

export const handlers: Handlers = {
  templates_list: () => CATALOG,
  template_preview: (args) => {
    const t = find(String(args.id));
    return [...render(t.id, EXAMPLE)].map(([path, text]): TemplateFileView => {
      const binary = TEXT[`${PREFIX}${t.id}/${path}`] === undefined;
      return { path, text: binary ? null : text, size_bytes: binary ? 2048 : text.length };
    });
  },
  template_create: (args) => {
    const t = find(String(args.id));
    const names = namesFor(String(args.modId), String(args.name));
    if (typeof names === "string") fail("invalid_input", names);
    const ws = mockWorkspace();
    if (ws.has(names.folder)) fail("conflict", `The workspace already has a folder named ${names.folder}.`);
    const files: Folder = new Map(render(t.id, names));
    if (!files.has("README.md")) files.set("README.md", `# ${names.name}\n\nStarted from the template "${t.name}": ${apply(t.summary, names)}\n`);
    ws.set(names.folder, files);
    log.templateCreates.push(`${t.id}:${names.folder}`);
    return names.folder;
  },
  template_add_preview: (args) => plan(String(args.folder), String(args.id)).preview,
  template_add: (args) => {
    const folder = String(args.folder);
    const { preview, writes } = plan(folder, String(args.id));
    if (preview.blocked.length) fail("conflict", preview.blocked[0]);
    const mine = mockWorkspace().get(folder);
    for (const [rel, text] of writes) {
      mine?.set(rel, text);
      emitMock({ type: "workspace_changed", folder, path: rel });
    }
    log.templateAdds.push(`${String(args.id)}:${folder}`);
    return [...writes.keys()];
  },
};
