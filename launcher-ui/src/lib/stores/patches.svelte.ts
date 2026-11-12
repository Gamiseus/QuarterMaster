import { call } from "../api";
import { commands, type Loadable, type ModPatchStatus, type QmEvent } from "../bindings";

/** A failed load is kept so the UI can offer Retry instead of loading forever. */
export type PatchesEntry = Loadable<ModPatchStatus[]> | { status: "error"; message: string };

const LOADING: PatchesEntry = { status: "loading" };

/** Per-mod patch status, refetched when the load order or the mod set changes. */
class PatchesStore {
  #cache = $state<Record<string, PatchesEntry>>({});
  /** Bumped on every invalidation so responses started before it are dropped. */
  #generation = 0;
  #seqs = new Map<string, number>();
  /** Shared across mods so a sequence is never reused after `retain` forgets a mod. */
  #lastSeq = 0;
  /** Every mod loaded so far; all of them are refetched on invalidation. */
  #loaded = new Set<string>();
  /** Mods with a current request in flight. */
  #pending = new Set<string>();

  get(modId: string): PatchesEntry {
    return this.#cache[modId] ?? LOADING;
  }

  async load(modId: string): Promise<void> {
    this.#loaded.add(modId);
    this.#pending.add(modId);
    const generation = this.#generation;
    const seq = ++this.#lastSeq;
    this.#seqs.set(modId, seq);
    const current = () => generation === this.#generation && seq === this.#seqs.get(modId);
    if (this.#cache[modId]?.status === "error") this.#cache[modId] = LOADING;
    let status: Loadable<ModPatchStatus[]>;
    try {
      // A mod uninstalled while its refetch was queued answers not_found: keep it inline, not as a toast.
      status = await call(commands.modPatchesGet(modId), { expect: ["not_found"] });
    } catch (e) {
      if (current()) {
        this.#pending.delete(modId);
        this.#cache[modId] = { status: "error", message: e instanceof Error ? e.message : String(e) };
      }
      throw e;
    }
    if (current()) {
      this.#pending.delete(modId);
      this.#cache[modId] = status;
    }
  }

  /** Loads every mod that has no entry and no request in flight (for the list's row status). */
  loadAll(ids: string[]): void {
    for (const id of ids) {
      if (!(id in this.#cache) && !this.#pending.has(id)) this.load(id).catch(() => {});
    }
  }

  /**
   * Patch status depends on the order and mod set: refetch every loaded mod. Old entries stay
   * visible until their replacement lands, so row badges do not flicker.
   */
  #invalidate(): void {
    this.#generation++;
    this.#pending.clear();
    for (const id of this.#loaded) this.load(id).catch(() => {});
  }

  /** Forgets every mod not in `ids` (the installed mods): its entry, request sequence and refetch. */
  retain(ids: string[]): void {
    const keep = new Set(ids);
    for (const id of Object.keys(this.#cache)) if (!keep.has(id)) delete this.#cache[id];
    for (const id of [...this.#seqs.keys()]) if (!keep.has(id)) this.#seqs.delete(id);
    for (const id of [...this.#loaded]) if (!keep.has(id)) this.#loaded.delete(id);
    for (const id of [...this.#pending]) if (!keep.has(id)) this.#pending.delete(id);
  }

  /** Test-only: forget cache, in-flight tracking and loaded mods. */
  reset(): void {
    this.#generation++;
    this.#cache = {};
    this.#seqs.clear();
    this.#loaded.clear();
    this.#pending.clear();
  }

  handle(e: QmEvent): void {
    if (e.type === "invalidated" && (e.scope === "load_order" || e.scope === "mods")) this.#invalidate();
  }
}

export const patches = new PatchesStore();
