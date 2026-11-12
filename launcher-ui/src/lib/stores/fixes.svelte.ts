import { untrack } from "svelte";
import { call, QmCommandError } from "../api";
import {
  commands, type FixDraft, type FixesView, type FixView, type JournalUndoChoice, type JournalUndoPreview, type JournalView, type QmEvent, type UndoPreview,
} from "../bindings";
import { attentionCounts, type AttentionCounts } from "../patches/fixlist";
import type { Checked } from "./loadorder.svelte";
import { profiles } from "./profiles.svelte";
import { toasts } from "./toasts.svelte";

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The one-time import notice's key in `ModProfile.notices_seen`. */
export const NOTICE_IMPORT = "fixes_import_v1";

/** The start of the service's refusal of an undo that needs a confirm (`fix_undo`). */
export const CONFIRM_FIRST = "Confirm first: ";

/** Whether `e` is the service asking to confirm an undo; the reason, else null. */
export function confirmReason(e: unknown): string | null {
  if (!(e instanceof QmCommandError) || e.kind !== "conflict" || !e.message.startsWith(CONFIRM_FIRST)) return null;
  return e.message.slice(CONFIRM_FIRST.length);
}

/**
 * How a legacy row (D38) is taken out with the command that made it, as the old launcher did: an
 * adjusted copy is removed, an order rule reset, a turned-off patch turned back on. Null for rows
 * with no such command (priorities, load order changes): they change where they were made.
 */
export function legacyRemoval(f: FixView): { text: string; run: () => Promise<unknown> } | null {
  if (!f.legacy) return null;
  const glue = f.glue_id;
  const patch = f.patch?.patch_id;
  if (f.kind === "adjusted_copy" && glue) return { text: "Remove", run: () => call(commands.gluePatchRemove(glue)) };
  if (f.kind === "order" && patch) return { text: "Reset", run: () => call(commands.patchOrderReset(patch)) };
  if (f.kind === "disable" && patch) return { text: "Turn back on", run: () => call(commands.patchSetEnabled(patch, true)) };
  return null;
}

/** The active profile's id, or null while the profiles are not loaded. */
const activeId = () => profiles.activeSummary?.id ?? null;

/**
 * The active profile's fixes (§11.1). Reloads when profiles or the load order change, and when
 * the suggestions the backend makes after a simulation are ready (`fixes`); an answer that
 * arrives after a newer request is dropped. An answer for another profile (the one active
 * before a switch) reads as loading, so nothing shows the old profile's fixes.
 */
class FixesStore {
  #view = $state<Checked<FixesView>>({ status: "loading" });
  #seq = 0;
  #queued = false;
  /** Requests in flight; the last answer may be out of date while one runs. */
  #inFlight = $state(0);
  /** Suggestions put off with "Not now" in this session, for the profile they were put off in. */
  #later = $state<{ profile: string | null; keys: string[] }>({ profile: null, keys: [] });
  /** The batch step the undo dialog asks about (D71); the dialog is mounted once in App. */
  journalStep = $state<JournalView | null>(null);
  journalOpen = $state(false);

  /** The fixes as last loaded; loading while none, or while the answer is for another profile. */
  get view(): Checked<FixesView> {
    const v = this.#view;
    const id = activeId();
    if (v.status === "ready" && id !== null && v.data.profile_id !== id) return { status: "loading" };
    return v;
  }

  set view(v: Checked<FixesView>) {
    this.#view = v;
  }

  /** A reload is running: the shown answer may be about to change. */
  get refreshing(): boolean {
    return this.#inFlight > 0;
  }

  /** Keys of the suggestions put off with "Not now" in the active profile. */
  get later(): ReadonlySet<string> {
    const l = this.#later;
    return new Set(l.profile === activeId() ? l.keys : []);
  }

  /** Hides a suggestion until the app restarts or another profile is opened. */
  putOff(key: string): void {
    const id = activeId();
    const keys = this.#later.profile === id ? this.#later.keys : [];
    this.#later = { profile: id, keys: [...new Set([...keys, key])] };
  }

  /** The fixes, or none while loading or after an error. */
  get list(): FixView[] {
    return this.view.status === "ready" ? this.view.data.fixes : [];
  }

  /** Fixes that warn before a launch (D23): Needs review, Suspended, Uses its own line. */
  get attention(): AttentionCounts {
    return attentionCounts(this.list);
  }

  /** The fixes a conflict's resolution made, oldest first (§11.1). */
  forConflict(conflictId: string): FixView[] {
    return this.list.filter((f) => f.provenance.conflict_id === conflictId);
  }

  /** The fixes that change or name `patchId` (a full id, or `mod:id`). */
  forPatch(patchId: string, modId?: string): FixView[] {
    const names = new Set([patchId, ...(modId ? [`${modId}:${patchId}`] : [])]);
    return this.list.filter((f) => [f.patch, ...f.related].some((l) => l !== null && names.has(l.patch_id)));
  }

  async load(): Promise<void> {
    const seq = ++this.#seq;
    // A refresh keeps the last answer until the new one lands (`view` hides one for another
    // profile). Untracked: callers may run in an effect.
    untrack(() => {
      if (this.#view.status === "error") this.#view = { status: "loading" };
      this.#inFlight++;
    });
    try {
      const data = await call(commands.fixesList(), { expect: ["not_found"] });
      if (seq === this.#seq) this.#view = { status: "ready", data };
    } catch (e) {
      if (seq === this.#seq) this.#view = { status: "error", message: message(e) };
      throw e;
    } finally {
      untrack(() => this.#inFlight--);
    }
  }

  /** What undoing `id` does; nothing is saved. */
  undoPreview(id: string): Promise<UndoPreview> {
    return call(commands.fixUndoPreview(id), { expect: ["conflict"] });
  }

  /**
   * Undoes `id`; `confirmed` once the user saw what else it takes along. A refusal that asks for a
   * confirm is rethrown without a toast (see `confirmReason`), for callers that ask inline.
   */
  async undo(id: string, confirmed: boolean): Promise<void> {
    try {
      await call(commands.fixUndo(id, confirmed), { expect: ["conflict"] });
    } catch (e) {
      if (e instanceof QmCommandError && e.kind === "conflict" && confirmReason(e) === null) toasts.error(e.message, e.details);
      throw e;
    }
    await this.load().catch(() => {});
  }

  /** The batch steps that can be undone, as last loaded. */
  get journal(): JournalView[] {
    return this.view.status === "ready" ? this.view.data.journal : [];
  }

  /**
   * Opens the undo dialog for the step made at `at`, from any screen. Positions move as steps are
   * undone, so the step is looked up after a reload.
   */
  async askJournalUndo(at: string): Promise<void> {
    await this.load();
    const step = this.journal.find((j) => j.at === at);
    if (!step) {
      toasts.info("That step is already undone.");
      return;
    }
    this.journalStep = step;
    this.journalOpen = true;
  }

  /** What undoing the step at `index` does, with each choice when newer fixes clash; nothing is saved. */
  journalUndoPreview(index: number): Promise<JournalUndoPreview> {
    return call(commands.fixJournalUndoPreview(index), { expect: ["conflict", "not_found", "invalid_input"] });
  }

  /**
   * Undoes the step made at `at`, with `choice` when newer fixes clash. A refusal is rethrown
   * without a toast, for the dialog to show. Resolves false when the step is already undone.
   */
  async journalUndo(at: string, choice: JournalUndoChoice | null): Promise<boolean> {
    const step = this.journal.find((j) => j.at === at);
    if (!step) {
      toasts.info("That step is already undone.");
      return false;
    }
    await call(commands.fixJournalUndo(step.index, choice), { expect: ["conflict"] });
    await this.load().catch(() => {});
    return true;
  }

  async setPaused(id: string, paused: boolean): Promise<void> {
    await call(commands.fixSetPaused(id, paused));
    await this.load().catch(() => {});
  }

  /** Keeps the fix as it is and pins it to the patch as it is now. */
  async accept(id: string): Promise<void> {
    await call(commands.fixAccept(id));
    await this.load().catch(() => {});
  }

  /** Points the fix at `patchId`, one of the patches it could mean (§9.3). */
  async rebind(id: string, patchId: string): Promise<void> {
    await call(commands.fixRebind(id, patchId));
    await this.load().catch(() => {});
  }

  /**
   * Replaces the fix with `draft` in one write (the Anchor fix's new line, §11.4); the
   * preview of the same change was shown first.
   */
  async replace(id: string, draft: FixDraft): Promise<void> {
    await call(commands.fixSuggestionApply([draft], [id]));
    await this.load().catch(() => {});
  }

  /** Makes one fix from `draft`; the preview of the same change was shown first. */
  async create(draft: FixDraft): Promise<FixView> {
    const made = await call(commands.fixCreate(draft));
    await this.load().catch(() => {});
    return made;
  }

  /** Takes out a legacy row with its own command (`legacyRemoval`). */
  async removeLegacy(f: FixView): Promise<void> {
    const removal = legacyRemoval(f);
    if (!removal) return;
    await removal.run();
    await this.load().catch(() => {});
  }

  /** Imports the earlier fixes again after the import stopped (D38). */
  async retryImport(): Promise<void> {
    const seq = ++this.#seq;
    const data = await call(commands.fixesImport());
    if (seq === this.#seq) this.#view = { status: "ready", data };
  }

  async dismissImportNotice(): Promise<void> {
    await call(commands.noticeDismiss(NOTICE_IMPORT));
  }

  handle(e: QmEvent): void {
    if (e.type !== "invalidated" || (e.scope !== "profiles" && e.scope !== "load_order" && e.scope !== "fixes")) return;
    // One profile write sends both scopes: reload once.
    if (this.#queued) return;
    this.#queued = true;
    queueMicrotask(() => {
      this.#queued = false;
      this.load().catch(() => {});
    });
  }

  /** Test-only. */
  reset(): void {
    this.#seq++;
    this.#queued = false;
    this.#view = { status: "loading" };
    this.#later = { profile: null, keys: [] };
    this.journalStep = null;
    this.journalOpen = false;
  }
}

export const fixes = new FixesStore();
