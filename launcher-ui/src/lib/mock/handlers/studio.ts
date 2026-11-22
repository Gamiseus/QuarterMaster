import type {
  PatchDraft,
  StudioChangedRun,
  StudioFileText,
  StudioGameFiles,
  StudioMatches,
  StudioPatch,
  StudioPatchId,
  StudioPatchKind,
  StudioPatchRef,
  StudioPatches,
  StudioPreview,
  StudioSaved,
} from "../../bindings";
import { fail, state, type Handlers } from "../state";
import { mockFileText, mockHash, mockWriteFile } from "./engine";

/** A small game: three Lua files. */
const GAME: Record<string, string> = {
  "main.lua": [
    "function love.load()",
    "    setup()",
    "    loadSounds()",
    "end",
    "",
    "function love.update(dt)",
    "    setup()",
    "    tick(dt)",
    "    updateUnits(dt)",
    "end",
    "",
    "function callArtillery(x, y)",
    "    local shells = 3",
    "    for i = 1, shells do",
    "        spawnShell(x, y)",
    "    end",
    "end",
    "",
    "function love.draw()",
    "    drawMap()",
    "    drawUnits()",
    "    drawHud()",
    "end",
    "",
  ].join("\n"),
  "includes/radio.lua": ["local RANGE = 400", "", "function radioRange()", "    return RANGE", "end", ""].join("\n"),
  "includes/hud.lua": ["function drawHud()", "    drawCompass()", "    drawAmmo()", "end", ""].join("\n"),
};

/** Tactical HUD's change to hud.lua in the active profile. */
const HUD_WITH_MODS = ["function drawHud()", "    drawCompass()", "    drawMinimap()", "    drawAmmo()", "end", ""].join("\n");

const PROFILE_IDS: StudioPatchId[] = [
  { id: "tactical-hud:minimap", name: "Minimap", mod_id: "tactical-hud" },
  { id: "gcore:init", name: "Init", mod_id: "gcore" },
  { id: "ai-tweaks:overlay", name: "AI overlay", mod_id: "ai-tweaks" },
];

function draft(kind: StudioPatchKind, fields: Partial<PatchDraft>): PatchDraft {
  return {
    kind,
    id: "",
    name: "",
    group: "",
    target: [],
    pattern: "",
    position: "",
    payload: "",
    match_indent: null,
    times: null,
    requires: [],
    before: [],
    after: [],
    on_failure: "",
    source: [],
    load_before: "",
    ...fields,
  };
}

const lists = new Map<string, StudioPatch[]>();

function seed(folder: string): StudioPatch[] {
  if (folder === "FieldRadio") {
    return [
      draft("pattern", {
        id: "field_radio:call",
        name: "Log radio calls",
        target: ["main.lua"],
        pattern: "function callArtillery(x, y)",
        position: "after",
        payload: '    QM.log("radio call")\n',
      }),
      draft("pattern", {
        id: "field_radio:range",
        name: "Longer radio range",
        target: ["includes/radio.lua"],
        pattern: "local RANGE = 400",
        position: "at",
        payload: "local RANGE = 650",
      }),
    ].map((d, i) => ({ at: { file: "qm.toml", form: "long", kind: "pattern", index: i }, draft: d, editable: true, hash: "" }));
  }
  if (folder === "SquadSettings") {
    return [
      {
        at: { file: "qm.toml", form: "long", kind: "module", index: 0 },
        draft: draft("module", { id: "squad_settings:main", name: "includes/squad_settings", source: ["main.lua"] }),
        editable: true,
        hash: "",
      },
    ];
  }
  return [];
}

function list(folder: string): StudioPatch[] {
  if (!lists.has(folder)) lists.set(folder, seed(folder));
  return lists.get(folder) as StudioPatch[];
}

function modId(folder: string): string {
  const json = mockFileText(folder, "mod.json");
  try {
    return (json && (JSON.parse(json) as { id?: string }).id) || folder;
  } catch {
    return folder;
  }
}

function version(folder: string): string {
  const json = mockFileText(folder, "mod.json");
  try {
    return (json && (JSON.parse(json) as { version?: string }).version) || "1.0.0";
  } catch {
    return "1.0.0";
  }
}

/** The TOML string for a value: a `'''` block for several lines. */
function tomlString(s: string): string {
  if (!s.includes("\n")) return JSON.stringify(s);
  return `'''\n${s.endsWith("\n") ? s : `${s}\n`}'''`;
}

/** The mock writes every patch in the long form, as Field Radio's file is. */
function render(folder: string): string {
  const out = ["[manifest]", `version = ${JSON.stringify(version(folder))}`];
  for (const p of list(folder)) {
    const d = p.draft;
    out.push("", "[[patches]]");
    if (d.before.length) out.push(`before = ${JSON.stringify(d.before)}`);
    if (d.after.length) out.push(`after = ${JSON.stringify(d.after)}`);
    out.push(`[patches.${d.kind}]`);
    const put = (k: string, v: string) => v.trim() && out.push(`${k} = ${tomlString(v)}`);
    put("id", d.id);
    put("name", d.name);
    put("group", d.group);
    if (d.kind === "module") {
      put("source", d.source[0] ?? "");
      put("before", d.load_before);
      continue;
    }
    if (d.target.length) out.push(`target = ${JSON.stringify(d.target[0])}`);
    put("pattern", d.pattern);
    put("position", d.position);
    if (d.times !== null) out.push(`times = ${d.times}`);
    if (d.match_indent !== null && d.kind === "pattern") out.push(`match_indent = ${d.match_indent}`);
    if (d.requires.length) out.push(`requires = ${JSON.stringify(d.requires)}`);
    put("on_failure", d.on_failure);
    out.push(`payload = ${tomlString(d.payload)}`);
  }
  return `${out.join("\n")}\n`;
}

function gameText(path: string, withMods: boolean): string {
  const text = GAME[path] ?? fail("not_found", `The game has no file ${path}.`);
  return withMods && path === "includes/hud.lua" ? HUD_WITH_MODS : text;
}

/** First lines (0-based) of each place the trimmed `pattern` lines are found. */
function places(text: string, pattern: string, regex: boolean): number[] {
  const lines = text.split("\n");
  if (regex) {
    let re: RegExp;
    try {
      re = new RegExp(pattern);
    } catch {
      return [];
    }
    return lines.flatMap((l, i) => (re.test(l) ? [i] : []));
  }
  const want = pattern
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l !== "");
  if (want.length === 0) return [];
  const out: number[] = [];
  for (let i = 0; i + want.length <= lines.length; i++) {
    if (want.every((w, j) => lines[i + j].trim() === w)) out.push(i);
  }
  return out;
}

function preview(folder: string, d: PatchDraft, withMods: boolean): StudioPreview {
  const target = d.target[0] ?? fail("invalid_input", "Choose the game file the patch changes.");
  const base = gameText(target, withMods);
  const empty = (kind: StudioPreview["outcome"]["kind"], messages: string[]): StudioPreview => ({
    target,
    text: base,
    changed: [],
    outcome: { kind, matched: 0, expected: null, messages },
    others: [],
    compile_error: null,
    compile_checked: true,
    mod_in_base: false,
  });
  if (!d.id.trim()) return empty("invalid", ["Give the patch an id."]);
  const regex = d.kind === "regex";
  const found = places(base, d.pattern, regex);
  if (found.length === 0) return empty("no_match", []);
  if (d.times !== null && d.times !== found.length) {
    return { ...empty("wrong_count", [`Expected ${d.times} matches, found ${found.length}.`]), outcome: { kind: "wrong_count", matched: found.length, expected: d.times, messages: [] } };
  }
  const span = regex ? 1 : d.pattern.split("\n").filter((l) => l.trim() !== "").length;
  const lines = base.split("\n");
  const payload = d.payload.replace(/\n$/, "").split("\n");
  const changed: StudioChangedRun[] = [];
  // From the last place up, so earlier line numbers stay put; runs are fixed after.
  const inserts: { at: number; remove: number }[] = [];
  for (const start of [...found].reverse()) {
    const at = d.position === "before" ? start : d.position === "after" ? start + span : start;
    const remove = d.position === "at" ? span : 0;
    lines.splice(at, remove, ...payload);
    inserts.unshift({ at, remove });
  }
  let shift = 0;
  for (const ins of inserts) {
    changed.push({ start: ins.at + shift + 1, end: ins.at + shift + payload.length, mine: true });
    shift += payload.length - ins.remove;
  }
  const text = lines.join("\n");
  const opens = (text.match(/\(/g) ?? []).length;
  const closes = (text.match(/\)/g) ?? []).length;
  const others = list(folder)
    .filter((p) => p.draft.id !== d.id && p.draft.target[0] === target)
    .map((p) => ({ patch_id: p.draft.id, name: p.draft.name, applied: places(base, p.draft.pattern, false).length > 0 }));
  return {
    target,
    text,
    changed,
    outcome: { kind: "applied", matched: found.length, expected: d.times, messages: [] },
    others,
    compile_error: state.init.engineNoLua || opens === closes ? null : `line ${lines.length}: ')' expected near the end of the file`,
    compile_checked: !state.init.engineNoLua,
    mod_in_base: false,
  };
}

function patches(folder: string): StudioPatches {
  const text = mockFileText(folder, "qm.toml");
  return { patches: list(folder), hash: text === undefined ? "" : mockHash(text), mod_id: modId(folder), long_form: true };
}

function checkHash(folder: string, expected: unknown): void {
  const text = mockFileText(folder, "qm.toml");
  const now = text === undefined ? "" : mockHash(text);
  if (typeof expected === "string" && expected !== "" && expected !== now) fail("conflict", "qm.toml changed on disk since it was read.");
}

export const handlers: Handlers = {
  studio_game_files: (): StudioGameFiles => {
    if (state.init.studioNoGame) return { game_ready: false, files: [] };
    return {
      game_ready: true,
      files: Object.entries(GAME).map(([path, text]) => ({
        path,
        lines: text.split("\n").length - 1,
        changed_by: path === "includes/hud.lua" ? 1 : 0,
      })),
    };
  },
  studio_game_file: (args): StudioFileText => {
    const path = String(args.path);
    const withMods = Boolean(args.withMods) && path === "includes/hud.lua";
    return {
      path,
      text: gameText(path, withMods),
      with_mods: withMods,
      runs: withMods ? [{ start: 3, end: 3, mod_id: "tactical-hud", patch_id: "tactical-hud:minimap", patch_name: "Minimap" }] : [],
    };
  },
  studio_matches: (args): StudioMatches => {
    const found = places(gameText(String(args.path), Boolean(args.withMods)), String(args.lines), false);
    return { count: found.length, lines: found.slice(0, 20).map((i) => i + 1) };
  },
  studio_patches: (args) => patches(String(args.folder)),
  studio_patch_ids: (args): StudioPatchId[] => {
    const folder = String(args.folder);
    const own = list(folder).map((p) => ({ id: p.draft.id, name: p.draft.name, mod_id: modId(folder) }));
    return [...own, ...PROFILE_IDS];
  },
  studio_preview: (args) => preview(String(args.folder), args.draft as PatchDraft, Boolean(args.withMods)),
  studio_save: (args): StudioSaved => {
    const folder = String(args.folder);
    const d = args.draft as PatchDraft;
    if (!d.id.trim()) fail("invalid_input", "Give the patch an id.");
    checkHash(folder, args.expectedHash);
    const items = list(folder);
    const editing = args.editing as StudioPatchRef | null;
    let at: StudioPatchRef;
    if (editing) {
      const i = items.findIndex((p) => p.at.index === editing.index && p.at.kind === editing.kind);
      if (i < 0) fail("conflict", "That patch is no longer where it was in qm.toml. Open it again.");
      at = { ...editing, kind: d.kind ?? editing.kind };
      items[i] = { at, draft: d, editable: true, hash: "" };
    } else {
      at = { file: "qm.toml", form: "long", kind: d.kind ?? "pattern", index: items.length };
      items.push({ at, draft: d, editable: true, hash: "" });
    }
    const hash = mockWriteFile(folder, "qm.toml", render(folder));
    return { at, hash, feature_added: false };
  },
  studio_remove: (args): string => {
    const folder = String(args.folder);
    checkHash(folder, args.expectedHash);
    const at = args.at as StudioPatchRef;
    const items = list(folder);
    const i = items.findIndex((p) => p.at.index === at.index && p.at.kind === at.kind);
    if (i < 0) fail("conflict", "That patch is no longer where it was in qm.toml. Open it again.");
    items.splice(i, 1);
    items.forEach((p, n) => (p.at = { ...p.at, index: n }));
    return mockWriteFile(folder, "qm.toml", render(folder));
  },
};
