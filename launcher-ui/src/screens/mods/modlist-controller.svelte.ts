import { call, QmCommandError } from "../../lib/api";
import {
  commands,
  type LoadOrderResolution,
  type LoadOrderUpdate,
  type ModProfile,
  type ModSummary,
  type ProfileSeparator,
} from "../../lib/bindings";
import { buildOrder, moveRows, rowKey, toLoadOrderUpdate, type OrderRow } from "../../lib/modlist/order";
import { UndoStack } from "../../lib/modlist/undo";
import { app } from "../../lib/stores/app.svelte";
import { loadOrder } from "../../lib/stores/loadorder.svelte";
import { mods as modsStore } from "../../lib/stores/mods.svelte";
import { profiles } from "../../lib/stores/profiles.svelte";
import { toasts } from "../../lib/stores/toasts.svelte";

/** Everything undo/redo covers: order (with separators), locks, enabled flags. */
export interface ListState {
  rows: OrderRow[];
  locked: string[];
  enabled: Record<string, boolean>;
}

export type SeparatorPatch = Partial<Pick<ProfileSeparator, "title" | "color" | "collapsed">>;

function sameRows(a: OrderRow[], b: OrderRow[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((r, i) => {
    const o = b[i];
    if (rowKey(r) !== rowKey(o)) return false;
    if (r.kind === "mod" || o.kind === "mod") return true;
    return r.sep.title === o.sep.title && r.sep.color === o.sep.color && !!r.sep.collapsed === !!o.sep.collapsed;
  });
}

function sameOrder(a: ListState, b: ListState): boolean {
  return sameRows(a.rows, b.rows) && a.locked.join("\n") === b.locked.join("\n");
}

function sameState(a: ListState, b: ListState): boolean {
  const ids = new Set([...Object.keys(a.enabled), ...Object.keys(b.enabled)]);
  return sameOrder(a, b) && [...ids].every((id) => a.enabled[id] === b.enabled[id]);
}

/** `target` with each separator's collapsed flag taken from `current` (collapse is not history). */
function keepCollapse(target: ListState, current: ListState): ListState {
  const flags = new Map(current.rows.flatMap((r) => (r.kind === "separator" ? [[r.sep.id, r.sep.collapsed]] : [])));
  const rows = target.rows.map((r): OrderRow =>
    r.kind === "separator" && flags.has(r.sep.id)
      ? { kind: "separator", sep: { ...r.sep, collapsed: flags.get(r.sep.id) } }
      : r,
  );
  return { ...target, rows };
}

function newSeparatorId(): string {
  const uuid = globalThis.crypto?.randomUUID?.();
  return `sep-${uuid ?? `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`}`;
}

/**
 * Mod list state with optimistic writes. Each change applies at once, then persists through
 * `load_order_set` / `mod_set_enabled` (queued, one at a time). Only a successful write records
 * an undo step (collapse/expand persists but is not an undo step); a failure rolls back to the last
 * saved state and drops the queued changes made on top of it. Undo/redo pressed during a write waits
 * for the queue to drain. Mutations are ignored until startup is done.
 */
export class ModListController {
  rows = $state<OrderRow[]>([]);
  locked = $state<Set<string>>(new Set());
  enabled = $state<Record<string, boolean>>({});
  #pending = $state(0);
  #historyVersion = $state(0);
  #history = new UndoStack<ListState>(50);
  #profile: ModProfile | null = null;
  #saved: ListState = { rows: [], locked: [], enabled: {} };
  #tail: Promise<void> = Promise.resolve();
  /** Bumped on every rollback; queued writes from an older generation are dropped. */
  #generation = 0;

  get busy(): boolean {
    return this.#pending > 0;
  }

  get canUndo(): boolean {
    void this.#historyVersion;
    return this.#history.canUndo;
  }

  get canRedo(): boolean {
    void this.#historyVersion;
    return this.#history.canRedo;
  }

  /** Replace the state from store data; a different profile starts a fresh history. */
  load(mods: ModSummary[], profile: ModProfile): void {
    if (this.#profile?.id !== profile.id) {
      this.#history.clear();
      this.#historyVersion++;
    }
    this.#profile = profile;
    const next: ListState = {
      rows: buildOrder(mods, profile),
      locked: [...(profile.locked_mods ?? [])],
      enabled: Object.fromEntries(mods.map((m) => [m.id, m.enabled])),
    };
    this.#saved = next;
    const current = this.#snapshot();
    if (!sameOrder(current, next) || JSON.stringify(current.enabled) !== JSON.stringify(next.enabled)) {
      this.#apply(next);
    }
  }

  /** Like load, but skipped while writes are in flight (the stores lag the optimistic state). */
  sync(mods: ModSummary[], profile: ModProfile): void {
    if (this.busy) return;
    this.load(mods, profile);
  }

  /** Test helper: forget everything. */
  reset(): void {
    this.#history.clear();
    this.#historyVersion++;
    this.#profile = null;
    this.#generation++;
    this.#saved = { rows: [], locked: [], enabled: {} };
    this.#apply(this.#saved);
  }

  move(selectedIds: string[], toIndex: number): Promise<void> {
    const cur = this.#snapshot();
    const rows = moveRows(cur.rows, selectedIds, toIndex, this.locked);
    return this.#change({ ...cur, rows });
  }

  setEnabled(ids: string[], enabled: boolean): Promise<void> {
    const cur = this.#snapshot();
    const next = { ...cur.enabled };
    for (const id of ids) if (id in next) next[id] = enabled;
    return this.#change({ ...cur, enabled: next });
  }

  /** `beforeId` is a row key (mod id or `sep:<id>`); null appends at the end. */
  addSeparator(title: string, color: string, beforeId: string | null): Promise<void> {
    const cur = this.#snapshot();
    const row: OrderRow = {
      kind: "separator",
      sep: { id: newSeparatorId(), title, color, insert_before_mod_id: null, collapsed: false },
    };
    const at = beforeId === null ? -1 : cur.rows.findIndex((r) => rowKey(r) === beforeId);
    const rows = [...cur.rows];
    rows.splice(at < 0 ? rows.length : at, 0, row);
    return this.#change({ ...cur, rows });
  }

  updateSeparator(id: string, patch: SeparatorPatch): Promise<void> {
    const cur = this.#snapshot();
    const rows = cur.rows.map((r): OrderRow =>
      r.kind === "separator" && r.sep.id === id ? { kind: "separator", sep: { ...r.sep, ...patch } } : r,
    );
    const collapseOnly = Object.keys(patch).every((k) => k === "collapsed");
    return this.#change({ ...cur, rows }, !collapseOnly);
  }

  removeSeparator(id: string): Promise<void> {
    const cur = this.#snapshot();
    return this.#change({ ...cur, rows: cur.rows.filter((r) => !(r.kind === "separator" && r.sep.id === id)) });
  }

  toggleLock(id: string): Promise<void> {
    const cur = this.#snapshot();
    const locked = cur.locked.includes(id) ? cur.locked.filter((x) => x !== id) : [...cur.locked, id];
    return this.#change({ ...cur, locked });
  }

  undo(): Promise<void> {
    return this.#whenIdle(() => this.#travel("undo"));
  }

  redo(): Promise<void> {
    return this.#whenIdle(() => this.#travel("redo"));
  }

  /** Runs `fn` now, or once the write queue drains (so history matches the saved state). */
  #whenIdle(fn: () => Promise<void>): Promise<void> {
    if (!this.busy) return fn();
    return this.#tail.catch(() => {}).then(() => this.#whenIdle(fn));
  }

  #travel(dir: "undo" | "redo"): Promise<void> {
    if (!this.#canMutate()) return Promise.resolve();
    const cur = this.#snapshot();
    const target = dir === "undo" ? this.#history.undo(cur) : this.#history.redo(cur);
    if (!target) return Promise.resolve();
    this.#historyVersion++;
    const restore = () => (dir === "undo" ? this.#history.redo(target) : this.#history.undo(target));
    return this.#commit(cur, keepCollapse(target, cur), false, restore);
  }

  #canMutate(): boolean {
    return app.startupDone && this.#profile !== null;
  }

  #snapshot(): ListState {
    return { rows: [...this.rows], locked: [...this.locked], enabled: { ...this.enabled } };
  }

  #apply(state: ListState): void {
    this.rows = state.rows;
    this.locked = new Set(state.locked);
    this.enabled = state.enabled;
  }

  #change(next: ListState, record = true): Promise<void> {
    if (!this.#canMutate()) return Promise.resolve();
    const cur = this.#snapshot();
    if (sameState(cur, next)) return Promise.resolve();
    return this.#commit(cur, next, record);
  }

  /** Apply `after` now; persist it after earlier writes; on failure roll back and run `onFail`. */
  #commit(before: ListState, after: ListState, record: boolean, onFail?: () => void): Promise<void> {
    this.#apply(after);
    const generation = this.#generation;
    this.#pending++;
    const run = async () => {
      if (generation !== this.#generation) {
        onFail?.();
        return;
      }
      try {
        await this.#persist(before, after);
        this.#saved = after;
        if (record) this.#history.push(before);
      } catch (e) {
        // call() already showed command errors (toast or game-running banner).
        if (!(e instanceof QmCommandError)) {
          console.error(e);
          toasts.error("Couldn't save the mod list. Your change was undone.", String(e));
        }
        // Part of a multi-step write may have been saved; keep that part undoable.
        if (record && !sameState(this.#saved, before)) this.#history.push(before);
        this.#generation++;
        onFail?.();
        this.#apply(this.#saved);
      } finally {
        this.#historyVersion++;
      }
    };
    const task = this.#tail.then(run).finally(() => this.#pending--);
    this.#tail = task;
    return task;
  }

  async #persist(before: ListState, after: ListState): Promise<void> {
    const profile = this.#profile as ModProfile;
    if (!sameOrder(before, after)) {
      const update = toLoadOrderUpdate(after.rows, { ...profile, locked_mods: after.locked });
      const resolution = await call(commands.loadOrderSet(update));
      this.#saved = { ...this.#saved, rows: after.rows, locked: after.locked };
      this.#orderSaved(update, resolution);
    }
    for (const id of Object.keys(after.enabled)) {
      const value = after.enabled[id];
      if (value === before.enabled[id]) continue;
      const summary = await loadOrder.whileChanging(() => call(commands.modSetEnabled(id, value)));
      this.#saved = { ...this.#saved, enabled: { ...this.#saved.enabled, [id]: value } };
      this.#enabledSaved(id, value, summary);
    }
  }

  /** Mirror a saved order into the stores so a later sync agrees with it. */
  #orderSaved(update: LoadOrderUpdate, resolution: LoadOrderResolution): void {
    const patch = { mod_priorities: update.mod_priorities, separators: update.separators, locked_mods: update.locked_mods };
    this.#profile = { ...(this.#profile as ModProfile), ...patch };
    if (profiles.active?.id === this.#profile.id) profiles.active = { ...profiles.active, ...patch };
    loadOrder.resolution = { status: "ready", data: resolution };
  }

  #enabledSaved(id: string, enabled: boolean, summary: ModSummary): void {
    const toggle = (list: string[]) => (enabled ? [...new Set([...list, id])] : list.filter((x) => x !== id));
    this.#profile = { ...(this.#profile as ModProfile), enabled_mods: toggle(this.#profile?.enabled_mods ?? []) };
    if (profiles.active?.id === this.#profile.id) {
      profiles.active = { ...profiles.active, enabled_mods: toggle(profiles.active.enabled_mods) };
    }
    const list = modsStore.list;
    if (list.status === "ready") {
      modsStore.list = { status: "ready", data: list.data.map((m) => (m.id === id ? { ...m, ...summary, enabled } : m)) };
    }
  }
}

/** The app's mod list; a singleton so undo history survives leaving the Mods screen. */
export const modList = new ModListController();
