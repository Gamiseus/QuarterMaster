import type {
  AnchorCandidate, ConflictResolutionSuggestion, DetectedConflict, Edge, FixChangeDraft, FixDraft, GluePatchEntry, InsertPosition,
  OrderRule, PatchDetail, PatchExecutionRecord, PatternPatch, PreviewChanges, RegexPatch, RegionOption, RegionOptions, Rung, SiteKey,
} from "../bindings";
import { anchorOf } from "./explain";

/** A draft that puts a patch's code next to other code (a rung each in `anchors`). */
export type AnchorChange = Extract<FixChangeDraft, { kind: "anchor" }>;

/** What applying a fix writes. */
export type FixAction =
  | { type: "order"; rule: OrderRule }
  | { type: "glue"; base: string; pattern: string; position: InsertPosition }
  | { type: "disable"; ids: string[] }
  | { type: "winner"; suggestion: Extract<ConflictResolutionSuggestion, { tier: "select_winner" }>; index: 0 | 1 }
  | { type: "anchor"; change: AnchorChange }
  /** A fix with no legacy form: run a patch as separate edits, or keep one patch's change to a line (D46, D49). */
  | { type: "fix"; change: EditsChange };

/** A draft that runs a patch as separate edits, or skips one of its edits. */
export type EditsChange = Extract<FixChangeDraft, { kind: "apply_as" } | { kind: "skip_hunk" }>;

export type FixOption = {
  key: string;
  label: string;
  /** The legacy what-if change set, for a profile whose earlier fixes are not imported (D38); null when there is none. */
  changes: PreviewChanges | null;
  /** Patch ids the fix turns off, for "what it disables". */
  disables: string[];
  action: FixAction;
  /** Set when the option cannot be used here: why. It gets no preview and no Apply. */
  unavailable?: string | null;
  /** A plain note under the label. */
  note?: string | null;
  /** The line as the game has it and as the other mod wrote it. */
  oldText?: string | null;
  newText?: string | null;
  /** The mod whose code an anchor option places the patch next to. */
  nextTo?: string | null;
};

export type FixContext = {
  a?: PatchDetail;
  b?: PatchDetail;
  /** Enabled patches that need A (or B), from the cascade walk. */
  aDependents: string[];
  bDependents: string[];
  /** Automatic retarget anchors (see `retargetAnchors`). */
  retargets: Retarget[];
  name: (patchId: string) => string;
  /** "Move to another line" makes an adjusted copy (`at` and regex patches, D25); else a fix on the live patch. */
  moveByCopy?: boolean;
  /** Which of A's places a move takes, when A has several (D25). */
  moveSites?: SiteKey[];
};

const BROKEN = new Set(["broken_anchor_replaced", "broken_anchor_order"]);
const OVERLAP = new Set(["overlapping_region", "duplicate_module", "same_anchor"]);

/** Whether `s` reads a capture group: `$1`, `$name` or `${name}` (`$$` is a plain `$`). */
export function usesCapture(s: string): boolean {
  for (let i = 0; i < s.length; i++) {
    if (s[i] !== "$") continue;
    const n = s[i + 1];
    if (n === "$") {
      i++;
      continue;
    }
    if (n !== undefined && /[A-Za-z0-9_{]/.test(n)) return true;
  }
  return false;
}

/** Whether a regex patch reads its match, so a copy on another line would not work (D59). */
export function readsCapture(r: RegexPatch): boolean {
  return r.root_capture !== null || usesCapture(r.payload) || usesCapture(r.line_prepend ?? "");
}

/** What a regex that reads no capture inserts: `line_prepend` on each line, `$$` as `$`. */
function plainPayload(r: RegexPatch): string {
  const prepend = r.line_prepend ?? "";
  const joined = (r.payload.match(/[^\n]*\n|[^\n]+$/g) ?? []).map((l) => prepend + l).join("");
  return joined.replaceAll("$$", "$");
}

/**
 * A local copy of `a` anchored on `pattern`, for previews (the real one comes from `glue_patch_create`). Like the real
 * one it runs as its base mod and matches its line exactly; it keeps the base's `times` (the service fills in the
 * base's place count when there is none, D59). A copy of a copy keeps the older copy's name and base mod, and takes
 * the first copy's base's place count, as the real one does. Null for a regex that reads its match.
 */
export function previewGlue(a: PatchDetail, pattern: string, position: InsertPosition): GluePatchEntry | null {
  const def = a.definition;
  // `a` is an adjusted copy itself (D59).
  const older = a.replaces_patch_id != null;
  let copy: PatternPatch;
  if ("pattern" in def) {
    // A pattern patch is copied whole, as the service copies it.
    copy = { ...def.pattern, pattern, position, times: older ? null : (def.pattern.times ?? null), id: null, name: null };
  } else if ("regex" in def) {
    const r = def.regex;
    if (readsCapture(r)) return null;
    copy = {
      // As the service's copy: the new lines take the anchor line's indent.
      pattern, position, target: r.target, payload: plainPayload(r), match_indent: true, times: r.times ?? null, id: null, name: null,
      requires: r.requires ?? [], priority: null, before: [], after: [], on_failure: r.on_failure ?? "warn",
    };
  } else {
    return null;
  }
  const baseMod = older ? (a.base_mod_id ?? null) : a.mod_id || null;
  return {
    id: `preview:${a.patch_id}`,
    name: older ? (a.name ?? a.patch_id) : `Adjusted copy of ${a.name ?? a.patch_id}`,
    replaces_patch_id: a.patch_id,
    priority: a.priority,
    patch: { pattern: copy },
    ...(baseMod ? { base_mod_id: baseMod } : {}),
    run_as: "base",
    exact: true,
  };
}

/**
 * Whether moving `a` makes an adjusted copy: it replaces its line or is a regex patch (D25). A regex that reads its
 * match moves by a rung instead (D59).
 */
export function movesByCopy(a?: PatchDetail): boolean {
  const def = a?.definition;
  if (!def) return true;
  if ("regex" in def) return !readsCapture(def.regex);
  return "pattern" in def ? def.pattern.position === "at" : true;
}

/** A regex patch places its code by its own position, so a rung on it takes that one (D52). */
export function rungPosition(a: PatchDetail | undefined, picked: InsertPosition): InsertPosition {
  const def = a?.definition;
  return def && "regex" in def ? def.regex.position : picked;
}

/** A Text rung that moves `patchId`'s code to the line that reads `text` (D25). */
export function moveChange(patchId: string, text: string, position: InsertPosition, sites: SiteKey[] = []): AnchorChange {
  return { kind: "anchor", patch_id: patchId, anchors: [{ pattern: text.trim(), exact: true, position }], sites };
}

/** A rung on a line inside another patch's place; `nth` picks among lines that read the same. */
export function withinChange(
  patchId: string, bPatch: string, site: number, text: string, nth: number | null, position: InsertPosition, sites: SiteKey[] = [],
): AnchorChange {
  return { kind: "anchor", patch_id: patchId, anchors: [{ within: bPatch, pattern: text.trim(), exact: true, nth, position, b_site: site }], sites };
}

/** A rung on one edge of another patch's place; `check` is the boundary line's short hash. */
export function edgeChange(patchId: string, bPatch: string, site: number, edge: Edge, check: string | null, sites: SiteKey[] = []): AnchorChange {
  const rung: Rung = { within: bPatch, edge, b_site: site };
  if (check) rung.check = check;
  return { kind: "anchor", patch_id: patchId, anchors: [rung], sites };
}

/** `nearest` marks the nearest-unchanged-line anchor (not offered to a patch that replaces its line). */
export type Retarget = { pattern: string; label: string; nearest: boolean };

/**
 * The anchor a retarget can use automatically (spec 10.2): the nearest line no patch touched that
 * matches exactly once in the patched file. Other mods' new lines are offered by the region
 * options instead (D27).
 */
export function retargetAnchors(c: DetectedConflict, records: PatchExecutionRecord[], candidates: AnchorCandidate[]): Retarget[] {
  const region = records.find((r) => r.patch_id === c.patch_b_id)?.regions[0];
  if (!region) return [];
  const touched = (line: number) =>
    records.some((r) => r.status === "applied" && r.regions.some((g) => line >= g.start_line && line <= g.end_line));
  const near = candidates
    .filter((x) => x.matches_in_file === 1 && !touched(x.line))
    .sort((x, y) => Math.abs(x.line - region.start_line) - Math.abs(y.line - region.start_line) || x.line - y.line)[0];
  return near ? [{ pattern: (near.raw_text ?? near.text).trim(), label: "Anchor on the nearest unchanged line", nearest: true }] : [];
}

/** The fix drafts an action writes; null for an adjusted copy, which has its own command. */
export function changesOf(action: FixAction): FixChangeDraft[] | null {
  switch (action.type) {
    case "order":
      return [{ kind: "order", rule: action.rule }];
    case "disable":
      return action.ids.map((patch_id) => ({ kind: "disable", patch_id }));
    case "winner": {
      const s = action.suggestion;
      return [{ kind: "disable", patch_id: action.index === 0 ? s.patch_b_id : s.patch_a_id }];
    }
    case "anchor":
    case "fix":
      return [action.change];
    case "glue":
      return null;
  }
}

/** The preview of `o` as the fixes it would add (§11.2); `null` when it has none (an adjusted copy). */
export function fixPreview(o: { action: FixAction }, conflictId: string, focus: string): PreviewChanges | null {
  const changes = changesOf(o.action);
  if (!changes) return null;
  const add_fixes: FixDraft[] = changes.map((change) => ({ change, label: "", source: "resolver", conflict_id: conflictId }));
  return { add_fixes, focus_file: focus };
}

/**
 * The region options (§9.2) as resolver options: rungs, and "Run <patch> as separate edits" (D46). "Pick a line
 * inside ..." has no draft and is left out.
 */
export function regionFixOptions(r: RegionOptions, modName: (id: string) => string): FixOption[] {
  const nextTo = r.b_mod_id ? modName(r.b_mod_id) : null;
  return r.options.flatMap((o: RegionOption): FixOption[] => {
    const change = o.draft?.change;
    if (!change || (change.kind !== "anchor" && change.kind !== "apply_as")) return [];
    const edits = change.kind === "apply_as";
    return [{
      key: o.key,
      label: o.label,
      changes: null,
      disables: [],
      action: change.kind === "anchor" ? { type: "anchor", change } : { type: "fix", change },
      unavailable: o.placement_ok ? null : (o.placement_reason ?? "cannot be used here"),
      note: o.note,
      oldText: o.old_text,
      newText: o.new_text,
      // Separate edits keep the patch's own lines: nothing to fall back to.
      nextTo: edits ? null : nextTo,
    }];
  });
}

/** Options whose drafts an earlier option already makes are left out (a region option and the conflict's own). */
export function distinctOptions(options: FixOption[]): FixOption[] {
  const seen = new Set<string>();
  return options.filter((o) => {
    const changes = changesOf(o.action);
    if (!changes) return true;
    const key = JSON.stringify(changes);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/** Every fix the resolver can offer for `c`, before previews say which resolve it. */
export function fixOptions(c: DetectedConflict, ctx: FixContext): FixOption[] {
  const focus = c.target_file;
  const out: FixOption[] = [];
  const a = c.patch_a_id;
  const b = c.patch_b_id;
  // A compile error the check could not pin on a patch has nothing to reorder or disable.
  if (!a) return out;
  const nameA = ctx.name(a);

  if (b) {
    const nameB = ctx.name(b);
    for (const [first, then, label] of [[b, a, `Run ${nameB} before ${nameA}`], [a, b, `Run ${nameA} before ${nameB}`]] as const) {
      const rule: OrderRule = { patch: first, before: [then], after: [] };
      out.push({ key: `order:${first}`, label, changes: { disable: [], enable: [], order_rules: [rule], glue_patches: [], focus_file: focus }, disables: [], action: { type: "order", rule } });
    }
  }

  const anchorA = anchorOf(ctx.a);
  if (BROKEN.has(c.kind) && ctx.a && anchorA) {
    const byCopy = ctx.moveByCopy ?? movesByCopy(ctx.a);
    // A patch that replaces its line would replace an unrelated line if moved anywhere else.
    for (const r of ctx.retargets.filter((x) => !(x.nearest && anchorA.position === "at"))) {
      const label = `${r.label}: ${r.pattern}`;
      if (!byCopy && anchorA.position !== "at") {
        out.push({
          key: `retarget:${r.pattern}`, label, changes: null, disables: [],
          action: { type: "anchor", change: moveChange(a, r.pattern, anchorA.position, ctx.moveSites ?? []) },
        });
        continue;
      }
      const glue = previewGlue(ctx.a, r.pattern, anchorA.position);
      if (!glue) continue;
      out.push({
        key: `retarget:${r.pattern}`,
        label,
        changes: { disable: [a], enable: [], order_rules: [], glue_patches: [glue], focus_file: focus },
        disables: [],
        action: { type: "glue", base: a, pattern: r.pattern, position: anchorA.position },
      });
    }
  }

  // The conflict's own fixes that have no legacy form: separate edits (D46), and whose change to a line stays (D49).
  for (const s of c.suggestions) {
    if (s.tier === "apply_as") {
      const name = ctx.name(s.patch_id);
      out.push({
        key: `edits:${s.patch_id}`,
        label: `Run ${name} as separate edits`,
        changes: null,
        disables: [],
        action: { type: "fix", change: { kind: "apply_as", patch_id: s.patch_id, mode: "edits" } },
        note: `${name}'s lines the game already has stay as they are; the other mod's lines between them stay`,
      });
    } else if (s.tier === "skip_hunk") {
      out.push({
        key: `keep:${s.keep_patch_id}:${s.patch_id}:${s.ordinal}`,
        label: `Keep ${ctx.name(s.keep_patch_id)}'s change to this line`,
        changes: null,
        disables: [],
        action: { type: "fix", change: { kind: "skip_hunk", patch_id: s.patch_id, ordinal: s.ordinal, keep_patch_id: s.keep_patch_id } },
        note: `${ctx.name(s.patch_id)} skips its own change there and still runs its other changes`,
      });
    }
  }

  const disable = (id: string, deps: string[]) => {
    const ids = [id, ...deps];
    out.push({
      key: `disable:${id}`,
      label: `Disable ${ctx.name(id)}`,
      changes: { disable: ids, enable: [], order_rules: [], glue_patches: [], focus_file: focus },
      disables: ids,
      action: { type: "disable", ids },
    });
  };
  disable(a, ctx.aDependents);
  if (b) disable(b, ctx.bDependents);

  if (OVERLAP.has(c.kind)) {
    const winner = c.suggestions.find((s): s is Extract<ConflictResolutionSuggestion, { tier: "select_winner" }> => s.tier === "select_winner");
    if (winner) {
      for (const index of [0, 1] as const) {
        const keep = index === 0 ? winner.patch_a_id : winner.patch_b_id;
        const lose = index === 0 ? winner.patch_b_id : winner.patch_a_id;
        out.push({
          key: `keep:${keep}`,
          label: `Keep ${ctx.name(keep)}`,
          changes: { disable: [lose], enable: [], order_rules: [], glue_patches: [], focus_file: focus },
          disables: [lose],
          action: { type: "winner", suggestion: winner, index },
        });
      }
    }
  }
  return out;
}

/** Whether the resolver offers "Move to another line..." (a pattern or regex patch that lost its line). */
export function canPickTarget(c: DetectedConflict, a?: PatchDetail): boolean {
  return (BROKEN.has(c.kind) || Boolean(c.consumer)) && anchorOf(a) !== null;
}

/** The first 16 hex digits of the SHA-256 of the trimmed line, as an edge rung's `check`; null when it cannot be worked out. */
export async function lineCheck(line: string): Promise<string | null> {
  try {
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(line.trim()));
    return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("").slice(0, 16);
  } catch {
    return null;
  }
}

/** What the anchor picker picked: a line of the file, a line inside another patch's place, or one of its edges. */
export type AnchorPick =
  | { kind: "line"; text: string; position: InsertPosition; aSite: SiteKey | null }
  | { kind: "region"; site: number; text: string; nth: number | null; position: InsertPosition; aSite: SiteKey | null }
  | { kind: "edge"; site: number; edge: Edge; boundary: string; aSite: SiteKey | null };
