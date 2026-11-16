import { call, QmCommandError } from "../api";
import { commands, type ApprovedImpact, type FixChangeDraft, type FixDraft, type PreviewResult } from "../bindings";
import type { FixAction } from "../patches/fixes";
import { confirmReason, fixes } from "./fixes.svelte";
import { toasts } from "./toasts.svelte";

/** The start of the service's refusal while a profile's earlier fixes cannot be imported (D38). */
export const NOT_IMPORTED = "Your existing fixes could not be imported yet";

/**
 * Makes the fixes in one write; while the profile's earlier fixes cannot be imported, runs the legacy
 * command instead, so the resolver keeps working as before (D38). A fix with no legacy command
 * (`legacy` null) reports the refusal. Other errors are reported as `call` reports them.
 */
async function createOr(drafts: FixDraft[], legacy: (() => Promise<unknown>) | null): Promise<void> {
  try {
    if (drafts.length === 1) await call(commands.fixCreate(drafts[0]), { expect: ["invalid_input"] });
    else await call(commands.fixCreateMany(drafts), { expect: ["invalid_input"] });
  } catch (e) {
    if (legacy && e instanceof QmCommandError && e.kind === "invalid_input" && e.message.startsWith(NOT_IMPORTED)) {
      await legacy();
      return;
    }
    if (e instanceof QmCommandError && e.kind === "invalid_input") toasts.error(e.message, e.details);
    throw e;
  }
}

/** A conflict's resolution, from the profile's fixes (§11.1). */
export type Resolved = { conflictId: string; label: string; fixIds: string[] };

/** What undoing a resolution takes along or leaves alone; empty: nothing to confirm. */
export type UndoCheck = { also: string[]; asks: string[] };

/** What the preview showed when the user applied a fix. */
function approvedOf(p: PreviewResult | null | undefined): ApprovedImpact | null {
  if (!p) return null;
  return {
    files_changed: p.impact.files_changed,
    patch_changes: p.impact.patch_changes.length,
    stack_changes: p.impact.stack_changes?.length ?? 0,
    resolved: p.resolved,
  };
}

/**
 * The conflict resolver dialog. Fixes applied from it are ledger fixes that carry the conflict's id,
 * so "Resolved by" and Undo survive a restart (§11.1).
 */
class ResolverStore {
  open = $state(false);
  conflictId = $state<string | null>(null);

  show(conflictId: string): void {
    this.conflictId = conflictId;
    this.open = true;
  }

  /** The fixes that resolved `conflictId` in the active profile. An unmigrated profile has none (D38). */
  resolvedFor(conflictId: string): Resolved | undefined {
    const list = fixes.forConflict(conflictId);
    if (list.length === 0) return undefined;
    return { conflictId, label: list[0].label, fixIds: list.map((f) => f.id) };
  }

  /** Writes `action` as fixes of `conflictId`; `label` names the first one. */
  async apply(
    conflictId: string,
    action: FixAction,
    label: string,
    opts: { optionKey?: string; preview?: PreviewResult | null } = {},
  ): Promise<void> {
    const draft = (change: FixChangeDraft, name: string): FixDraft => ({
      change,
      label: name,
      source: "resolver",
      conflict_id: conflictId,
      option_key: opts.optionKey ?? null,
      approved: approvedOf(opts.preview),
    });
    switch (action.type) {
      case "order": {
        const rule = action.rule;
        await createOr([draft({ kind: "order", rule }, label)], () => call(commands.patchOrderSet(rule)));
        break;
      }
      case "glue":
        await call(commands.gluePatchCreate(action.base, action.pattern, action.position, conflictId));
        break;
      case "disable": {
        // One write for the patch and its dependents: all of them go off, or none.
        const ids = action.ids;
        const drafts = ids.map((id, i) => draft({ kind: "disable", patch_id: id }, i === 0 ? label : ""));
        await createOr(drafts, () => call(commands.patchSetEnabledMany(ids, false)));
        break;
      }
      case "winner": {
        const s = action.suggestion;
        const lose = action.index === 0 ? s.patch_b_id : s.patch_a_id;
        await createOr([draft({ kind: "disable", patch_id: lose }, label)], () => call(commands.conflictResolve(s, action.index)));
        break;
      }
      case "anchor":
      case "fix":
        await createOr([draft(action.change, label)], null);
        break;
    }
    await fixes.load().catch(() => {});
  }

  /**
   * What undoing the resolution of `conflictId` takes along or leaves alone. Nothing is saved. Null
   * when it cannot be checked now (the simulation is still running): `undo` then says what needs a
   * confirm.
   */
  async undoCheck(conflictId: string): Promise<UndoCheck | null> {
    const r = this.resolvedFor(conflictId);
    if (!r) return { also: [], asks: [] };
    const also: string[] = [];
    const asks: string[] = [];
    const label = (id: string) => fixes.list.find((f) => f.id === id)?.label ?? id;
    // One preview at a time (the backend runs one per window).
    for (const id of r.fixIds) {
      let p;
      try {
        p = await fixes.undoPreview(id);
      } catch (e) {
        if (e instanceof QmCommandError && (e.kind === "conflict" || e.kind === "cancelled")) return null;
        throw e;
      }
      also.push(...p.also_undoes.filter((x) => !r.fixIds.includes(x)).map(label));
      asks.push(...p.asks);
    }
    return { also: [...new Set(also)], asks: [...new Set(asks)] };
  }

  /**
   * Undoes the fixes that resolved `conflictId`, newest first. Returns what to confirm when the
   * service refuses an unconfirmed undo (nothing more is undone then), else null.
   */
  async undo(conflictId: string, confirmed = false): Promise<UndoCheck | null> {
    const r = this.resolvedFor(conflictId);
    if (!r) return null;
    try {
      for (const id of [...r.fixIds].reverse()) {
        try {
          await call(commands.fixUndo(id, confirmed), { expect: ["not_found", "conflict"] });
        } catch (e) {
          // A fix undone along with an earlier one is gone already.
          if (e instanceof QmCommandError && e.kind === "not_found") continue;
          const reason = confirmed ? null : confirmReason(e);
          if (reason !== null) return { also: [], asks: [reason] };
          if (e instanceof QmCommandError && e.kind === "conflict") toasts.error(e.message, e.details);
          throw e;
        }
      }
      return null;
    } finally {
      await fixes.load().catch(() => {});
    }
  }

  /** Test-only. */
  reset(): void {
    this.open = false;
    this.conflictId = null;
  }
}

export const resolver = new ResolverStore();
