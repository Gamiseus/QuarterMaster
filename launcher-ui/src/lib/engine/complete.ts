/**
 * What the Mod Engine's editor offers while typing: the keys a TOML table or `mod.json` takes
 * (from the editor schemas QuarterMaster publishes), and Lua names (the game's own globals and
 * QuarterMaster's Lua API). Pure functions; the CodeMirror glue is in CodeEditor.svelte.
 */
import factionSchema from "../../../../schemas/faction.schema.json";
import modSchema from "../../../../schemas/mod.schema.json";
import qmSchema from "../../../../schemas/qm.schema.json";
import type { EngineSymbol } from "../bindings";

export interface Suggestion {
  label: string;
  /** Shown beside the label: the value's type, or a function's arguments. */
  detail: string;
  /** The longer description, when the schema has one. */
  info: string;
  kind: "property" | "function" | "variable" | "keyword" | "namespace";
}

type Schema = {
  properties?: Record<string, Schema>;
  items?: Schema;
  description?: string;
  type?: string | string[];
  oneOf?: Schema[];
  enum?: unknown[];
};

/** The table a TOML cursor is in, as a dotted path: `manifest`, `pattern`, `patches.pattern`, or "" at the top. */
export function tomlTableAt(text: string, offset: number): string {
  const before = text.slice(0, offset);
  const lines = before.split("\n");
  // The current line is still being typed; a header is only a whole line above it.
  lines.pop();
  for (let i = lines.length - 1; i >= 0; i--) {
    const m = /^\s*\[\[?\s*([A-Za-z0-9_.\-"]+?)\s*\]\]?\s*(#.*)?$/.exec(lines[i]);
    if (m) return m[1].replace(/"/g, "");
  }
  return "";
}

function typeText(s: Schema | undefined): string {
  if (!s) return "";
  if (s.enum) return s.enum.map((v) => JSON.stringify(v)).join(" | ");
  if (Array.isArray(s.type)) return s.type.join(" | ");
  if (s.type) return s.type;
  if (s.oneOf) return s.oneOf.map((o) => typeText(o)).filter(Boolean).join(" | ");
  return "";
}

function propsOf(schema: Schema | undefined): Suggestion[] {
  return Object.entries(schema?.properties ?? {}).map(([label, s]) => ({
    label,
    detail: typeText(s),
    info: s.description ?? "",
    kind: "property" as const,
  }));
}

/** The schema of a table path in `root`: `a.b` walks into properties, and an array's items. */
function walk(root: Schema, path: string[]): Schema | undefined {
  let at: Schema | undefined = root;
  for (const part of path) {
    const next: Schema | undefined = at?.properties?.[part];
    at = next?.items ?? next;
    if (!at) return undefined;
  }
  return at;
}

/** The keys of the TOML table at `table` in a patch file (`qm.toml`, `qm/*.toml`). */
export function patchFileKeys(table: string): Suggestion[] {
  const root = qmSchema as unknown as Schema;
  if (table === "") return propsOf(root);
  // `[[patches]]` alone takes the type tables; `[patches.pattern]` takes that type's keys.
  const parts = table.split(".");
  return propsOf(walk(root, parts));
}

/** The keys of the TOML table at `table` in `faction.toml`. */
export function factionKeys(table: string): Suggestion[] {
  const root = factionSchema as unknown as Schema;
  if (table === "") return propsOf(root);
  return propsOf(walk(root, table.split(".")));
}

/** The keys a top-level `mod.json` object takes. */
export function manifestKeys(): Suggestion[] {
  return propsOf(modSchema as unknown as Schema);
}

const LUA_KEYWORDS = [
  "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
  "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/** QuarterMaster's Lua API, as the modding wiki documents it. */
const QM_API: { label: string; info: string }[] = [
  { label: "QM.settings", info: "Each mod's options by mod id: QM.settings.<mod_id>.<option_id>." },
  { label: "QM.Menu", info: "Mod Operations cards and pages, and options built at run time." },
  { label: "QM.Input", info: "Keybind actions: register, read and rebind." },
  { label: "QM.Sprites", info: "The size an image is shown at." },
  { label: "QM.Ordnance", info: "Ordnance (grenades, rifle grenades, mortars, artillery, air strikes, drops): QM.Ordnance.register{ id, kind (\"thrown\", \"rifle\", \"mortar\", \"artillery\", \"airstrike\", \"drop\"), sprite, explosion, range, cooldown, shots, spread, delay, interval, plane, contents, on_open, skill, replaces_frag, replaces_rifle_grenade, replaces_mortar, replaces_artillery, replaces_bombing, replaces_strafing, replaces_drop }; callIn, giveSkill, swapFrag, swapRifle, swapMortar, swapArtillery, swapBombing, swapStrafing, swapDrop." },
  { label: "QM.Skills", info: "Class skills: QM.Skills.set{ class, slot, name, description, icon, cooldown, unlock, use = function(soldier) | ordnance | game = \"light:3\" | remove = true }; add{...} (the next new slot), remove(class, slot), get(class, slot)." },
  { label: "QM.Explosions", info: "Explosions: QM.Explosions.register{ id, based_on, ... }; presets frag, contact, at_grenade, rifle_grenade, mortar, faust, artillery, heavy_mortar, bomb, strafe." },
  { label: "QM.Events", info: "Game events: QM.Events.on(name, fn, owner), off(id), emit(name, ...). battle_start, battle_end, wave_start, game_saved, game_loaded." },
  { label: "modSettings", info: "Every mod's options in one table; a checkbox is 1 or 0." },
];

/** Lua names starting with `prefix`: QuarterMaster's API, the game's globals and keywords. */
export function luaNames(symbols: EngineSymbol[], prefix: string): Suggestion[] {
  const p = prefix.toLowerCase();
  const out: Suggestion[] = [];
  for (const a of QM_API) {
    if (a.label.toLowerCase().startsWith(p)) out.push({ label: a.label, detail: "QuarterMaster", info: a.info, kind: "namespace" });
  }
  for (const s of symbols) {
    if (!s.name.toLowerCase().startsWith(p)) continue;
    out.push({
      label: s.name,
      detail: s.kind === "function" ? s.signature.replace(/^function/, "") : "table",
      info: `The game's own, in ${s.file} line ${s.line}.`,
      kind: s.kind === "function" ? "function" : "variable",
    });
    if (out.length > 200) break;
  }
  for (const k of LUA_KEYWORDS) if (k.startsWith(p)) out.push({ label: k, detail: "", info: "", kind: "keyword" });
  return out;
}

/** Which key list a TOML file uses. */
export function tomlKeysFor(path: string, text: string, offset: number): Suggestion[] {
  const lower = path.toLowerCase();
  const table = tomlTableAt(text, offset);
  if (lower === "qm.toml" || lower === "quartermaster.toml" || (lower.startsWith("qm/") && lower.endsWith(".toml"))) {
    return patchFileKeys(table);
  }
  if (lower === "faction.toml") return factionKeys(table);
  return [];
}
