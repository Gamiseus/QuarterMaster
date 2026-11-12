import type {
  DetectedConflict, FileStat, InsertPosition, PatchDetail, PatchExecutionRecord, PatchRegion, SharedStack, SimulationOverview,
} from "../bindings";

export type ExplainContext = {
  a?: PatchDetail;
  b?: PatchDetail;
  /** The conflict file's records (for line numbers and which patch runs later). */
  records: PatchExecutionRecord[];
  modName: (id: string) => string;
};

/** Pattern and regex patches have an anchor and a position; copy and module patches do not. */
export function anchorOf(d?: PatchDetail): { pattern: string; position: InsertPosition } | null {
  if (!d) return null;
  const def = d.definition;
  if ("pattern" in def) return { pattern: def.pattern.pattern, position: def.pattern.position };
  if ("regex" in def) return { pattern: def.regex.pattern, position: def.regex.position };
  return null;
}

/** What a patch does, in a few words: "replaces 1 line", "adds 2 lines after". */
export function actionOf(d: PatchDetail | undefined, rec?: PatchExecutionRecord): string {
  const a = anchorOf(d);
  const def = d?.definition;
  const payload = def && "pattern" in def ? def.pattern.payload : def && "regex" in def ? def.regex.payload : "";
  const n = payload ? payload.split("\n").length : (rec?.regions[0] ? rec.regions[0].end_line - rec.regions[0].start_line + 1 : 0);
  const lines = `${n} ${n === 1 ? "line" : "lines"}`;
  if (!a) return def && "module" in def ? "adds a module" : "copies code into the file";
  if (a.position === "at") return `replaces the matched line with ${lines}`;
  return `adds ${lines} ${a.position} the matched line`;
}

/**
 * The inspector's "Runs as" value (§6.6): "3 insertions", or "one replacement (the lines are ambiguous)". Null when it
 * says nothing: a patch that keeps none of its lines runs as one replacement either way.
 */
export function runsAsValue(d: PatchDetail | undefined): string | null {
  const v = d?.runs_as;
  if (!v) return null;
  if (v.asks !== "edits" && (v.edits?.kept ?? 0) === 0) return null;
  return v.text.replace(/^Runs as /, "");
}

/** One mod's version of a line two patches change (D49): its new text, or what it does instead. */
export function versionText(text: string | null | undefined): { text: string; code: boolean } {
  if (text == null) return { text: "Replaces its lines as one block", code: false };
  if (text === "") return { text: "Removes the line", code: false };
  return { text, code: true };
}

/** An info note (`multi_match`): shown with its patch, never counted as a conflict. */
export const isNote = (c: DetectedConflict): boolean => c.severity === "info";

/** Kinds with no patch record in a file: a module, or a `[vars]` name. */
const RECORDLESS = new Set(["duplicate_module", "shadowed_file", "var_collision", "replaced_module_compile_error"]);

/** A mod module that does not compile (plan 2e Phase D 6.2), not a patched file. */
export const isModuleCompileError = (c: DetectedConflict): boolean => c.kind === "compile_error" && c.id.startsWith("compile-module:");

/** A conflict no file view lists: no patch record ties it to a line. */
export const isRecordless = (c: DetectedConflict): boolean => RECORDLESS.has(c.kind) || isModuleCompileError(c);

/** The conflicts (not notes) that no file in `stats` lists, so the Patches tab lists them apart. */
export function unlistedConflicts(conflicts: DetectedConflict[], stats: FileStat[]): DetectedConflict[] {
  const files = new Set(stats.map((s) => s.target_file));
  return conflicts.filter((c) => !isNote(c) && (isRecordless(c) || !files.has(c.target_file)));
}

/** `s` with its first letter in upper case. */
const upperFirst = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** Record warning codes with a plain text after `<code>:<key>: `. */
const NOTE_CODES = ["needs_patch", "edits_refused", "hunk_skipped"];

/**
 * A record's warnings that say why a patch ran as it did, in plain words: a patch it needs (D64), one that ran as one
 * replacement (D47), or an edit it skipped (D49). Other warnings are left out.
 */
export function recordNotes(r: PatchExecutionRecord | null | undefined): string[] {
  return (r?.warnings ?? []).flatMap((w) => {
    const code = NOTE_CODES.find((c) => w.startsWith(`${c}:`));
    if (!code) return [];
    // "<code>:<key>: <text>"
    const rest = w.slice(code.length + 1);
    const at = rest.indexOf(": ");
    const text = at === -1 ? rest : rest.slice(at + 2);
    return text.trim() ? [`${upperFirst(text.trim().replace(/\.$/, ""))}.`] : [];
  });
}

/**
 * The simulation's order reports as notes (G15): order rules a stronger one replaced, names no patch matches, and
 * places a patch does not use, each with the patch and why.
 */
export function orderNotes(o: SimulationOverview, patchName: (id: string) => string = (id) => id): string[] {
  return [
    ...(o.overridden_rules ?? []).map((r) => `Order rule not used: ${r.replace(/\.$/, "")}.`),
    ...(o.unknown_ids ?? []).map((id) => `An order rule names "${id}", which matches no loaded patch.`),
    // The reason names the patch already.
    ...(o.suspended_rungs ?? []).map((r) => `${upperFirst(r.reason).replace(/\.$/, "")}.`),
    ...(o.invalid_rungs ?? []).map((r) => `${patchName(r.patch_id)}: place ${r.rung + 1} is never used: ${r.detail.replace(/\.$/, "")}.`),
  ];
}

/** A module replaces the record's file, so the game never runs the patch (G1). */
export const isShadowed = (r: PatchExecutionRecord): boolean => r.warnings.some((w) => w.startsWith("shadowed_by_module:"));

const nameOf = (d: PatchDetail | undefined, fallback: string) => d?.name ?? d?.patch_id ?? fallback;

/** A finding of the semantic checks (D69): a warning or a note, never a launch blocker. */
export const isSemantic = (c: DetectedConflict): boolean => c.id.startsWith("semantic:");

/** What a semantic finding is about: the global or function after the id's last "@" (what mod.json `overrides` names). */
export function semanticName(c: DetectedConflict): string {
  const at = c.id.lastIndexOf("@");
  return isSemantic(c) && at !== -1 ? c.id.slice(at + 1) : "";
}

/** A key_collision name ("ctrl+shift+f9") as the player sees it ("Ctrl+Shift+F9"). */
export function keyLabel(name: string): string {
  const m = /^((?:(?:alt|ctrl|gui|shift)\+)*)(.+)$/.exec(name);
  if (!m) return name;
  const mods = m[1].split("+").filter((x) => x !== "").map((x) => x[0].toUpperCase() + x.slice(1));
  const key = m[2].length === 1 || /^f\d+$/.test(m[2]) ? m[2].toUpperCase() : m[2];
  return [...mods, key].join("+");
}

function regionOf(records: PatchExecutionRecord[], id: string | null): PatchRegion | undefined {
  return records.find((r) => r.patch_id === id)?.regions[0];
}

/** One plain sentence or two saying what went wrong (spec 10.1, "What happened"). */
export function explainConflict(c: DetectedConflict, ctx: ExplainContext): string {
  const modA = ctx.modName(c.mod_a_id);
  const modB = c.mod_b_id ? ctx.modName(c.mod_b_id) : "";
  const patchA = nameOf(ctx.a, c.patch_a_id);
  const file = c.target_file;

  switch (c.kind) {
    case "broken_anchor_replaced":
    case "broken_anchor_order": {
      const anchorA = anchorOf(ctx.a);
      const took = c.consumer ? consumerSentence(c, ctx, anchorA?.position) : null;
      if (took) return took;
      const regionB = regionOf(ctx.records, c.patch_b_id);
      const posB = anchorOf(ctx.b)?.position;
      if (c.mod_b_id && regionB && posB) {
        const verbB = posB === "at" ? "replaces" : posB === "after" ? "adds code after" : "adds code before";
        const lineB = posB === "at" ? regionB.start_line : posB === "after" ? regionB.start_line - 1 : regionB.end_line + 1;
        const verbA =
          anchorA?.position === "after" ? "adds code after that line" :
          anchorA?.position === "before" ? "adds code before that line" : "replaces that line too";
        return `${modB} ${verbB} line ${lineB} of ${file}. ${modA} ${verbA}, so "${patchA}" can no longer find it.`;
      }
      if (c.mod_b_id) {
        return `${modA}'s "${patchA}" looks for a line in ${file} that ${modB} changes first, so it can no longer find it.`;
      }
      return anchorA
        ? `${modA}'s "${patchA}" cannot find the line it looks for in ${file}: ${anchorA.pattern.trim()}`
        : `${modA}'s "${patchA}" cannot find the line it looks for in ${file}.`;
    }
    case "overlapping_region": {
      const ra = regionOf(ctx.records, c.patch_a_id);
      const rb = regionOf(ctx.records, c.patch_b_id);
      const start = ra && rb ? Math.max(ra.start_line, rb.start_line) : 0;
      const end = ra && rb ? Math.min(ra.end_line, rb.end_line) : -1;
      if (ra && rb && start <= end) {
        const lines = start === end ? `line ${start}` : `lines ${start} to ${end}`;
        const ia = ctx.records.findIndex((r) => r.patch_id === c.patch_a_id);
        const ib = ctx.records.findIndex((r) => r.patch_id === c.patch_b_id);
        const later = ib > ia ? modB : modA;
        return `${modA} and ${modB} both change ${lines} of ${file}. ${later} runs later, so its change is what the game gets there.`;
      }
      if (rb) {
        // The later patch replaced what the earlier one wrote, so only its lines remain.
        const patchB = nameOf(ctx.b, c.patch_b_id ?? "");
        const at = rb.start_line === rb.end_line ? `line ${rb.start_line}` : `lines ${rb.start_line} to ${rb.end_line}`;
        return `${modB}'s "${patchB}" replaces code that ${modA}'s "${patchA}" added to ${file} (now ${at}). ${modB} runs later, so its version is what the game gets.`;
      }
      return `${modA} and ${modB} both change the same lines of ${file}.`;
    }
    case "sites_lost": {
      const [lost, total] = c.sites_lost ?? [0, 0];
      const took = c.consumer ? consumerSentence(c, ctx, anchorOf(ctx.a)?.position) : null;
      const count = total > 0 ? `${lost} of its ${total} places` : "some of its places";
      return `${took ? took + " " : ""}${modA}'s "${patchA}" lost ${count} in ${file}${modB ? ` to ${modB}` : ""}. It still adds its code at the others.`;
    }
    case "duplicate_module":
      return `${modA} and ${modB} both add the module "${file}". The one loaded last, ${modB}, replaces the other.`;
    case "shadowed_file":
      return `${modA} replaces ${file} with a module of its own, so the game never runs ${file} or the patches on it.`;
    case "target_matches_several":
      return `${modA}'s "${patchA}" names its file "${file}", which matches more than one game file. The game patches each of them.`;
    case "target_without_boundary":
      return `${modA}'s "${patchA}" also patches ${file}, because the game matches the end of the file path.`;
    case "patch_aborts_file":
      return `${modA}'s "${patchA}" cannot find its line in ${file} and is set to stop when that happens, so the game fails to load ${file} and stops with a Lua error.`;
    case "sites_gained":
      return `${modB}'s code has a copy of the line ${modA}'s "${patchA}" looks for in ${file}, so "${patchA}" also adds its code there.`;
    case "multi_match":
      return `${modA}'s "${patchA}" has no match count and matches more than one place in ${file}, so it runs at each of them.`;
    case "target_is_module":
      return `${modA}'s "${patchA}" targets ${file}, which is ${modB}'s module, not a game file. Modules are not patch targets, so the patch never runs.`;
    case "var_collision":
      return `${modA} and ${modB} set the same variable to different values. The game uses ${modB}'s value in every patch.`;
    case "capture_before_define": {
      const name = semanticName(c) || "a function";
      return `${modA}'s "${patchA}" keeps a copy of ${name} from before ${modB} defines it, so it calls the old ${name} (or nothing), not ${modB}'s.`;
    }
    case "redefined_global": {
      const name = semanticName(c) || "the same function";
      if (c.description.includes("depends on when")) {
        return `${modA} and ${modB} both define ${name}. The one that runs last replaces the other, and which one that is depends on when the game calls their code.`;
      }
      return `${modA} and ${modB} both define ${name}. ${modB}'s runs later and replaces ${modA}'s, so ${modA}'s ${name} never runs.`;
    }
    case "dead_code":
      if (isNote(c)) return `${modA}'s "${patchA}" turns off some of the game's code in ${file} with an "if" that is never true.`;
      return `${modA}'s "${patchA}" adds code to ${file} inside an "if" that is never true${modB ? ` (from ${modB})` : ""}, so that code never runs.`;
    case "early_return_shadow": {
      const patchB = nameOf(ctx.b, c.patch_b_id ?? "");
      const where = semanticName(c) || file;
      return `${modB}'s "${patchB}" returns from ${where} before ${modA}'s "${patchA}" code in ${file}, so that code never runs.`;
    }
    case "table_literal_reset": {
      const name = semanticName(c) || "a table";
      return `${modB} sets ${name} to a new table after ${modA} adds to it, so what ${modA} added is gone.`;
    }
    case "shared_file_writer": {
      const name = semanticName(c) || "the same file";
      if (isNote(c)) return `${modA} and ${modB} both save ${name}. Both read it first and keep what is in it.`;
      if (c.description.includes("Neither reads")) {
        return `${modA} and ${modB} both save ${name} without reading it first, so the one that saves last drops what the other saved.`;
      }
      return `${modA} and ${modB} both save ${name}. ${modB} writes the whole file without reading it first, so it can drop what ${modA} saved.`;
    }
    case "key_collision": {
      const key = keyLabel(semanticName(c)) || "the same key";
      const when = c.description.includes("player releases") ? "released" : "pressed";
      return `${modA} and ${modB} both use ${key}, so both act when it is ${when}.`;
    }
    case "double_hook": {
      const name = semanticName(c) || "the same function";
      return `${modA} and ${modB} both wrap ${name}, but ${modB}'s wrapper does not call ${modA}'s, so ${modA}'s wrapper never runs.`;
    }
    case "bypassed_hook": {
      const name = semanticName(c) || "it";
      if (c.description.includes(" tests `")) {
        return `${modB} replaces every "${name}" test in the game's code, but ${modA}'s "${patchA}" still tests it, so ${modB}'s change does not reach that code.`;
      }
      return `${modB} replaces every call of ${name} in the game's code, but ${modA}'s "${patchA}" still calls it, so ${modB}'s change does not reach that code.`;
    }
    case "shared_field": {
      const name = semanticName(c) || "one";
      return `${modA} and ${modB} both keep their own data in the ${name} field of game objects. The game never uses that field, so check that both mean the same by it.`;
    }
    case "name_collision": {
      const name = semanticName(c);
      if (name.startsWith("menuPage=")) {
        return `${modA} and ${modB} both use menu page ${name.slice("menuPage=".length)}, which the game does not use, so their screens share one page number.`;
      }
      return `${modA} and ${modB} both add a button whose click calls ${name || "the same function"}, so both buttons do the same thing.`;
    }
    case "block_balance": {
      if (c.patch_b_id) {
        const patchB = nameOf(ctx.b, c.patch_b_id);
        // The service says why the partner falls short: turned off, fewer places, or not applied.
        const why = c.description.includes("applied at fewer places")
          ? "applies at fewer places"
          : c.description.includes("is turned off")
            ? "is turned off"
            : "did not apply";
        return `${modA}'s "${patchA}" and "${patchB}" open and close one block in ${file}, but "${patchB}" ${why}, so the blocks no longer match and ${file} does not compile.`;
      }
      return `${modA}'s "${patchA}" opens and closes a different number of blocks in ${file}, so ${file} may not compile.`;
    }
    case "moved_scope": {
      const name = semanticName(c) || "another function";
      return `With this fix, ${modA}'s "${patchA}" runs in ${name} instead of where its author put it, so it runs at another time and sees other variables.`;
    }
    case "free_name_changed": {
      const name = semanticName(c) || "a name";
      return `With this fix, ${name} in ${modA}'s "${patchA}" means another variable than the one its author wrote it for.`;
    }
    case "same_line_edit": {
      const at = c.same_line ? `line ${c.same_line.line}` : "the same line";
      return `${modA} and ${modB} both change ${at} of ${file}. Only one change can stay; the other patch can skip its change there and keep the rest.`;
    }
    default:
      return c.description;
  }
}

const quote = (t: string) => `"${t.trim()}"`;

/** What the patch that took A's line did to it (§9.1), as one or two sentences; null when unknown. */
function consumerSentence(c: DetectedConflict, ctx: ExplainContext, posA: InsertPosition | undefined): string | null {
  const k = c.consumer;
  if (!k) return null;
  const modA = ctx.modName(c.mod_a_id);
  const modB = ctx.modName(k.mod_id);
  const patchA = nameOf(ctx.a, c.patch_a_id);
  const file = c.target_file;
  const line = k.site_line > 0 ? `line ${k.site_line}` : "a line";
  const verbA =
    posA === "after" ? "adds code after that line" : posA === "before" ? "adds code before that line" : posA === "at" ? "replaces that line too" : "looks for that line";
  const lost = `${modA} ${verbA}, so "${patchA}" can no longer find it.`;
  const added = (k.new_text ?? "").split("\n").filter((l) => l.trim() !== "").length;
  switch (k.kind) {
    case "rewritten_1to1":
      return `${modB} replaces ${line} of ${file}. ${lost}`;
    case "replaced_by_block":
      return `${modB} replaces ${line} of ${file} with ${added} ${added === 1 ? "line" : "lines"} of its own. ${lost}`;
    case "deleted":
      return `${modB} removes ${line} of ${file}. ${lost}`;
    case "reemitted":
      return `${modB} rewrites the code at ${line} of ${file} and writes ${quote(k.old_text)} back, but not the lines around it. So "${patchA}" can no longer find its place.`;
    case "split":
      return `${modB} adds code between the lines "${patchA}" looks for at ${line} of ${file}, so ${modA} can no longer find them.`;
  }
}

/** The line a card shows for a fix that puts code next to another mod's code. */
export function fallbackLine(modB: string): string {
  return `Uses its own line if ${modB} is off.`;
}

/**
 * Why a patch's code sits where it does at a shared line, for each line where stacking moved it:
 * "Tactical Overhaul loads after GCore, so its code goes below GCore's at line 16."
 */
export function stackSentences(patchId: string, stacks: SharedStack[], modName: (id: string) => string): string[] {
  return stacks.flatMap((s) => {
    const b = s.blocks.find((x) => x.patch_id === patchId);
    const r = b?.reason;
    if (!b || !r) return [];
    const me = modName(b.mod_id);
    const line = s.file_line;
    switch (r.kind) {
      case "below_dependency":
        return [`${me} loads after ${modName(r.of)}, so its code goes below ${modName(r.of)}'s at line ${line}.`];
      case "above_dependent":
        return [`${modName(r.of)} loads after ${me}, so ${me}'s code goes above ${modName(r.of)}'s at line ${line}.`];
      case "load_order":
        return [`Code at line ${line} runs in load order, so ${me}'s code comes after the mods loaded before it.`];
      case "unsatisfiable":
        return [`At line ${line}, ${me} should go below ${modName(r.below)} and above ${modName(r.above)}. No place fits both, so it goes below ${modName(r.below)}.`];
      case "guard_clamped":
        return [`${me}'s code stays in the older place at line ${line}, so it does not cross its next match.`];
      case "user_rule":
        return [`You moved ${me}'s code at line ${line}.`];
      case "compat_rule":
        return [`${modName(r.mod_id)} moves ${me}'s code at line ${line}.`];
      case "author_hint": {
        if (r.of === b.mod_id) {
          const where = r.hint === "first" ? "on top" : "at the bottom";
          return [`${me} asks to run ${r.hint} at line ${line}, so its code goes ${where}.`];
        }
        const where = r.hint === "first" ? "below" : "above";
        return [`${modName(r.of)} asks to run ${r.hint} at line ${line}, so ${me}'s code goes ${where} it.`];
      }
    }
  });
}
