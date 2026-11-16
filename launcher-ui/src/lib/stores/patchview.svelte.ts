import { call } from "../api";
import { commands, type FileDiff, type PatchDetail, type QmEvent } from "../bindings";

export type Entry<T> = { status: "loading" } | { status: "ready"; data: T } | { status: "error"; message: string };

const LOADING = { status: "loading" } as const;
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * The Patches tab's selection and data: the selected file's diff and the selected patch's detail.
 * Diffs are cached per file until the simulation changes; responses that arrive after a newer
 * request or a newer simulation are dropped.
 */
class PatchViewStore {
  file = $state<string | null>(null);
  patch = $state<string | null>(null);
  diff = $state<Entry<FileDiff>>(LOADING);
  detail = $state<Entry<PatchDetail> | null>(null);
  /** The center column: the file's changes, its patch order, or the profile's fixes. */
  view = $state<"changes" | "order" | "fixes" | "assets">("changes");
  /** In the order view: this file only, every file (read-only), or the lines several patches share (read-only). */
  orderScope = $state<"file" | "all" | "stacks">("file");
  /** Simulation generation: bumped when the load order or mod set changes. */
  #gen = 0;
  #diffs = new Map<string, FileDiff>();
  #diffSeq = 0;
  #detailSeq = 0;

  selectFile(file: string): void {
    if (file === this.file) return;
    this.file = file;
    void this.#loadDiff();
  }

  selectPatch(id: string | null): void {
    this.patch = id;
    void this.#loadDetail();
  }

  /** Selects `id` and the first file it touches (deep links from other screens). */
  async openPatch(id: string): Promise<void> {
    this.selectPatch(id);
    const seq = this.#detailSeq;
    const d = await this.#fetchDetail(id).catch(() => null);
    if (d && seq === this.#detailSeq && d.positions[0]) this.selectFile(d.positions[0].target_file);
  }

  async #loadDiff(): Promise<void> {
    const file = this.file;
    if (!file) return;
    const seq = ++this.#diffSeq;
    const gen = this.#gen;
    const cached = this.#diffs.get(file);
    if (cached) {
      this.diff = { status: "ready", data: cached };
      return;
    }
    // A new file shows loading; a refetch of the same file keeps the old diff until the new one lands.
    if (this.diff.status !== "ready" || this.diff.data.result.target_file !== file) this.diff = LOADING;
    try {
      const r = await call(commands.fileDiffGet(file), { expect: ["not_found"] });
      if (seq !== this.#diffSeq || gen !== this.#gen) return;
      if (r.status === "ready") {
        this.#diffs.set(file, r.data);
        this.diff = { status: "ready", data: r.data };
      } else {
        this.diff = LOADING;
      }
    } catch (e) {
      if (seq === this.#diffSeq && gen === this.#gen) this.diff = { status: "error", message: message(e) };
    }
  }

  #fetchDetail(id: string): Promise<PatchDetail> {
    return call(commands.patchDetail(id), { expect: ["not_found", "conflict"] });
  }

  async #loadDetail(): Promise<void> {
    const id = this.patch;
    const seq = ++this.#detailSeq;
    if (!id) {
      this.detail = null;
      return;
    }
    if (this.detail?.status !== "ready" || this.detail.data.patch_id !== id) this.detail = LOADING;
    try {
      const d = await this.#fetchDetail(id);
      if (seq === this.#detailSeq) this.detail = { status: "ready", data: d };
    } catch (e) {
      if (seq === this.#detailSeq) this.detail = { status: "error", message: message(e) };
    }
  }

  /** Reloads what is showing (after a profile change, or for Retry). */
  refresh(): void {
    this.#gen++;
    this.#diffs.clear();
    void this.#loadDiff();
    void this.#loadDetail();
  }

  handle(e: QmEvent): void {
    if (e.type === "invalidated" && (e.scope === "load_order" || e.scope === "mods")) this.refresh();
  }

  /** Test-only. */
  reset(): void {
    this.#gen++;
    this.#diffs.clear();
    this.file = null;
    this.patch = null;
    this.diff = LOADING;
    this.detail = null;
    this.view = "changes";
    this.orderScope = "file";
  }
}

export const patchView = new PatchViewStore();
