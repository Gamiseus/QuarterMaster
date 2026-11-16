import { SvelteMap } from "svelte/reactivity";
import { call } from "../api";
import {
  commands,
  type QmEvent,
  type RibbonInfo,
  type SaveBackupEntry,
  type SaveCleanup,
  type SaveComparison,
  type SaveSlotSummary,
  type SaveSummary,
  type SpriteKind,
} from "../bindings";
import { slug } from "../slug";

export type SaveDetail = { status: "loading" } | { status: "ready"; data: SaveSummary } | { status: "error"; message: string };

/** Save slots: the list (also used by the shell's slot chip), the selected slot and its dossier. */
class SavesStore {
  list = $state<SaveSlotSummary[] | null>(null);
  /** The slot open on the Saves screen; null means the active slot. */
  selected = $state<string | null>(null);
  detail = new SvelteMap<string, SaveDetail>();
  #seq = 0;
  #detailSeq = new Map<string, number>();
  #lastDetailSeq = 0;

  get activeId(): string | null {
    return this.list?.find((s) => s.is_active)?.profile.id ?? null;
  }

  /** The selected slot when it still exists, else the active one. */
  get current(): string | null {
    if (this.selected && this.list?.some((s) => s.profile.id === this.selected)) return this.selected;
    return this.activeId;
  }

  async load(): Promise<void> {
    const seq = ++this.#seq;
    const list = await call(commands.savesList());
    if (seq === this.#seq) this.list = list;
  }

  /** Display name for a slot id; falls back to the id until the list is loaded. */
  nameFor(id: string): string {
    return this.list?.find((s) => s.profile.id === id)?.profile.name ?? id;
  }

  async loadDetail(id: string): Promise<void> {
    const seq = ++this.#lastDetailSeq;
    this.#detailSeq.set(id, seq);
    if (this.detail.get(id)?.status !== "ready") this.detail.set(id, { status: "loading" });
    try {
      const data = await call(commands.saveDetail(id));
      if (this.#detailSeq.get(id) === seq) this.detail.set(id, { status: "ready", data });
    } catch (e) {
      if (this.#detailSeq.get(id) === seq) this.detail.set(id, { status: "error", message: e instanceof Error ? e.message : String(e) });
    }
  }

  compare(a: string, b: string): Promise<SaveComparison> {
    return call(commands.savesCompare(a, b));
  }

  async activate(id: string): Promise<void> {
    await call(commands.saveActivate(id));
  }

  /** Clones `src` under `name`; the new slot gets a free id made from the name and is selected. */
  async clone(src: string, name: string): Promise<void> {
    const taken = new Set((this.list ?? []).map((s) => s.profile.id));
    const base = slug(name) || "slot";
    let id = base;
    for (let n = 2; taken.has(id); n++) id = `${base}-${n}`;
    const profile = await call(commands.saveClone(src, id, name));
    this.selected = profile.id;
  }

  backup(id: string, label: string): Promise<SaveBackupEntry> {
    return call(commands.saveBackup(id, label));
  }

  async restore(id: string, backupId: string): Promise<void> {
    await call(commands.saveRestore(id, backupId));
  }

  async deleteBackup(id: string, backupId: string): Promise<void> {
    await call(commands.saveDeleteBackup(id, backupId));
  }

  async rename(id: string, name: string): Promise<void> {
    await call(commands.saveRename(id, name));
  }

  /** A new, empty slot; it is selected. */
  async create(name: string): Promise<void> {
    const p = await call(commands.saveCreate(name));
    this.selected = p.id;
  }

  async remove(id: string): Promise<void> {
    await call(commands.saveDelete(id));
    if (this.selected === id) this.selected = null;
  }

  async resetSlot(id: string): Promise<void> {
    await call(commands.saveReset(id));
  }

  /** Takes values a removed mod left out of the slot's save; a backup is made first. */
  cleanup(id: string, fields: string[]): Promise<SaveCleanup> {
    return call(commands.saveCleanup(id, fields));
  }

  async setLock(id: string, locked: boolean): Promise<void> {
    await call(commands.saveLockSet(id, locked));
  }

  /** Writes the dossier where the user picks; resolves to the file name, or null when cancelled. */
  exportDossier(slotName: string, text: string): Promise<string | null> {
    return call(commands.saveDossierExport(slotName, text));
  }

  /** Ribbon names from the game, loaded once. */
  ribbons = $state<Map<number, RibbonInfo> | null>(null);
  #ribbonsLoading = false;

  loadRibbons(): void {
    if (this.ribbons || this.#ribbonsLoading) return;
    this.#ribbonsLoading = true;
    call(commands.gameRibbonsGet())
      .then((list) => (this.ribbons = new Map(list.map((r) => [r.id, r]))))
      .catch(() => (this.ribbons = new Map()))
      .finally(() => (this.#ribbonsLoading = false));
  }

  /** Sprite data URLs by `kind:id`; `null` once the game has no such image. */
  sprites = new SvelteMap<string, string | null>();
  #spriteLoads = new Set<string>();

  loadSprite(kind: SpriteKind, id: number): void {
    const key = `${kind}:${id}`;
    if (this.sprites.has(key) || this.#spriteLoads.has(key)) return;
    this.#spriteLoads.add(key);
    // Quiet: a missing image just shows the placeholder.
    commands
      .gameSpriteGet(kind, id)
      .then((r) => this.sprites.set(key, r.status === "ok" ? r.data : null))
      .catch(() => this.sprites.set(key, null))
      .finally(() => this.#spriteLoads.delete(key));
  }

  handle(e: QmEvent): void {
    if (e.type !== "invalidated" || e.scope !== "saves") return;
    // Any save write can change any dossier; the screen reloads the one it shows.
    this.#detailSeq.clear();
    this.detail.clear();
    this.load().catch(() => {});
  }

  /** Test-only: forget everything. */
  reset(): void {
    this.#seq++;
    this.list = null;
    this.selected = null;
    this.#detailSeq.clear();
    this.detail.clear();
    this.ribbons = null;
    this.sprites.clear();
  }
}

export const saves = new SavesStore();
