/** CodeMirror completion sources for the Mod Engine's editor, built on the lists in complete.ts. */
import type { Completion, CompletionContext, CompletionResult, CompletionSource } from "@codemirror/autocomplete";
import type { EngineSymbol } from "../bindings";
import { luaNames, manifestKeys, tomlKeysFor, type Suggestion } from "./complete";
import { languageOf } from "./text";

function toCompletion(s: Suggestion, apply?: string): Completion {
  return { label: s.label, detail: s.detail || undefined, info: s.info || undefined, type: s.kind, apply };
}

/** Lua: names as they are typed, dots and colons included (`QM.Menu`, `obj:method`). */
function luaSource(symbols: () => EngineSymbol[]): CompletionSource {
  return (ctx: CompletionContext): CompletionResult | null => {
    const word = ctx.matchBefore(/[A-Za-z_][\w.:]*/);
    if (!word || (word.from === word.to && !ctx.explicit)) return null;
    // Not inside a comment or a string on this line.
    const line = ctx.state.doc.lineAt(ctx.pos);
    const before = line.text.slice(0, word.from - line.from);
    if (before.includes("--") || (before.split('"').length - 1) % 2 === 1 || (before.split("'").length - 1) % 2 === 1) return null;
    const options = luaNames(symbols(), word.text).map((s) => toCompletion(s));
    return options.length ? { from: word.from, options, validFor: /^[\w.:]*$/ } : null;
  };
}

/** TOML: the keys of the table the cursor is in, at the start of a line. */
function tomlSource(path: string): CompletionSource {
  return (ctx: CompletionContext): CompletionResult | null => {
    const line = ctx.state.doc.lineAt(ctx.pos);
    const before = line.text.slice(0, ctx.pos - line.from);
    if (!/^\s*[A-Za-z0-9_-]*$/.test(before)) return null;
    const word = ctx.matchBefore(/[A-Za-z0-9_-]*/);
    if (!word || (word.from === word.to && !ctx.explicit)) return null;
    const keys = tomlKeysFor(path, ctx.state.doc.toString(), ctx.pos);
    if (keys.length === 0) return null;
    return { from: word.from, options: keys.map((k) => toCompletion(k, `${k.label} = `)), validFor: /^[A-Za-z0-9_-]*$/ };
  };
}

/** mod.json: the manifest's keys inside a quoted key at the top level. */
function manifestSource(): CompletionSource {
  return (ctx: CompletionContext): CompletionResult | null => {
    const word = ctx.matchBefore(/"[A-Za-z_]*/);
    if (!word) return null;
    const line = ctx.state.doc.lineAt(ctx.pos);
    const before = line.text.slice(0, word.from - line.from);
    // A key starts a line, or follows `{` or `,`; a value follows `:`.
    if (!/^\s*$|[{,]\s*$/.test(before)) return null;
    return {
      from: word.from + 1,
      options: manifestKeys().map((k) => toCompletion(k)),
      validFor: /^[A-Za-z_]*$/,
    };
  };
}

/** The completion source for a file, or undefined when nothing is offered for its kind. */
export function completionFor(path: string, symbols: () => EngineSymbol[]): CompletionSource | undefined {
  const lang = languageOf(path);
  if (lang === "lua") return luaSource(symbols);
  if (lang === "toml") {
    const lower = path.toLowerCase();
    const known = lower === "qm.toml" || lower === "quartermaster.toml" || lower === "faction.toml" || (lower.startsWith("qm/") && lower.endsWith(".toml"));
    return known ? tomlSource(path) : undefined;
  }
  if (path.toLowerCase() === "mod.json") return manifestSource();
  return undefined;
}
