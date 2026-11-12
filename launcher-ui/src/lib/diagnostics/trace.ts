import type { DiagnosticRunPreview, DiagnosticRunResult, GlobalChange, HookCaller, HookCalls } from "../bindings";

/** Names typed into a field, split at commas and spaces, each once. */
export function parseNames(text: string): string[] {
  const out: string[] = [];
  for (const n of text.split(/[\s,]+/)) {
    if (n && !out.includes(n)) out.push(n);
  }
  return out;
}

/** The fields' starting text: the names the code checks flagged. */
export function suggestedText(p: DiagnosticRunPreview): { globals: string; hooks: string } {
  return { globals: p.globals.map((g) => g.name).join(", "), hooks: p.hooks.map((h) => h.name).join(", ") };
}

/** A chunk name as a file: `@main.lua` is main.lua, a module chunk is its file. */
export function sourceFile(source: string): string {
  const module = source.match(/^=?\[qm (\S+) "([^"]+)"\]$/);
  if (module) return `${module[2]} (module ${module[1]})`;
  return source.replace(/^[@=]/, "");
}

/** Who wrote a line: a mod's patch, or the game. */
function writer(modId: string | null | undefined, patchId: string | null | undefined, modName: (id: string) => string): string {
  if (!modId && !patchId) return "the game's own code";
  if (!modId) return `the patch ${patchId}`;
  return patchId ? `${modName(modId)} (patch ${patchId})` : modName(modId);
}

/** One change of a watched global, as a sentence. */
export function changeLine(c: GlobalChange, modName: (id: string) => string): string {
  const what = c.value_type === "nil" ? "Removed" : `Set to a ${c.value_type}`;
  return `${what} by ${writer(c.mod_id, c.patch_id, modName)}, ${sourceFile(c.source ?? "")} line ${c.line ?? 0}.`;
}

export interface GlobalTrace {
  name: string;
  changes: GlobalChange[];
  /** How many different mods changed it; 2 or more is the usual sign of a clash. */
  mods: number;
}

/** The trace by watched global, in the order the names were given. */
export function globalTraces(r: DiagnosticRunResult): GlobalTrace[] {
  return (r.watched.globals ?? []).map((name) => {
    const changes = r.globals_trace.filter((c) => c.name === name).sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0));
    const mods = new Set(changes.map((c) => c.mod_id).filter((m) => m)).size;
    return { name, changes, mods };
  });
}

/** "Never changed while the game loaded." or "Changed 3 times, by 2 mods." */
export function traceSummary(t: GlobalTrace): string {
  const n = t.changes.length;
  if (n === 0) return "Never set while the game loaded.";
  const times = n === 1 ? "Set once" : `Set ${n} times`;
  if (t.mods === 0) return `${times}, by the game's own code only.`;
  return `${times}, by ${t.mods === 1 ? "1 mod" : `${t.mods} mods`}.`;
}

/** One place that calls a watched function. */
export function callerLine(c: HookCaller, modName: (id: string) => string): string {
  const calls = c.calls === 1 ? "1 call" : `${c.calls ?? 0} calls`;
  return `${sourceFile(c.source)} line ${c.line}: ${calls}, in ${writer(c.mod_id, c.patch_id, modName)}.`;
}

/** "Called 3 times from 2 places." */
export function hookSummary(h: HookCalls): string {
  if (h.missing) return "No function of this name existed after the game loaded.";
  const total = h.total ?? 0;
  if (total === 0) return "Never called during the run.";
  const places = (h.callers ?? []).length;
  return `Called ${total === 1 ? "once" : `${total} times`} from ${places === 1 ? "1 place" : `${places} places`}.`;
}

/** What the result view says before the lists. */
export function resultStatus(r: DiagnosticRunResult): string {
  if (r.running) return "The game is running. Close it to see the full result.";
  if (!r.reported) return "The game wrote no report for this run, so there is nothing to show.";
  return "The game has closed. This is what it reported.";
}
