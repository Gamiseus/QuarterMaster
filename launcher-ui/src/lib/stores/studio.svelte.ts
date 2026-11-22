import { call, QmCommandError } from "../api";
import {
  commands,
  type PatchDraft,
  type StudioFileText,
  type StudioGameFiles,
  type StudioMatches,
  type StudioPatch,
  type StudioPatchId,
  type StudioPatchKind,
  type StudioPatchRef,
  type StudioPatches,
  type StudioPreview,
} from "../bindings";
import { dedent, slugId } from "../engine/studio";
import { toasts } from "./toasts.svelte";

/** How long the form rests before the engine works out the result again. */
export const PREVIEW_DELAY_MS = 300;

/** Chosen lines of the game file, 1-based and inclusive. */
export interface LineRange {
  from: number;
  to: number;
}

export function emptyDraft(kind: StudioPatchKind): PatchDraft {
  return {
    kind,
    id: "",
    name: "",
    group: "",
    target: [],
    pattern: "",
    position: kind === "copy" ? "append" : "after",
    payload: "",
    match_indent: kind === "pattern" ? true : null,
    times: null,
    requires: [],
    before: [],
    after: [],
    on_failure: "",
    source: [],
    load_before: "",
  };
}

class StudioStore {
  folder = $state<string | null>(null);
  files = $state<StudioGameFiles | null>(null);
  patches = $state<StudioPatches | null>(null);
  ids = $state<StudioPatchId[]>([]);
  file = $state<StudioFileText | null>(null);
  withMods = $state(false);
  selection = $state<LineRange | null>(null);
  matches = $state<StudioMatches | null>(null);
  draft = $state<PatchDraft | null>(null);
  /** The patch being edited; null for a new one. */
  editing = $state<StudioPatchRef | null>(null);
  /** The hash of the edited patch's file as read. */
  #editingHash = "";
  preview = $state<StudioPreview | null>(null);
  previewing = $state(false);
  saving = $state(false);
  /** The id follows the name until it is typed in. */
  idTouched = $state(false);
  /** Counts drafts opened, so the payload editor knows a new one from an edit. */
  draftSeq = $state(0);
  #saved = "";
  #previewTimer: ReturnType<typeof setTimeout> | undefined;
  #previewSeq = 0;
  #matchSeq = 0;
  #fileSeq = 0;

  get dirty(): boolean {
    return this.draft !== null && JSON.stringify(this.draft) !== this.#saved;
  }

  /** The chosen lines' text, as the file has them. */
  get selectedText(): string {
    const f = this.file;
    const s = this.selection;
    if (!f || !s) return "";
    return f.text.split("\n").slice(s.from - 1, s.to).join("\n");
  }

  reset(): void {
    clearTimeout(this.#previewTimer);
    this.folder = null;
    this.files = null;
    this.patches = null;
    this.ids = [];
    this.file = null;
    this.selection = null;
    this.matches = null;
    this.closeDraft();
  }

  /** Reads the game's files and the mod's patches. */
  async load(folder: string): Promise<void> {
    if (this.folder !== folder) {
      this.reset();
      this.folder = folder;
    }
    const [files, patches, ids] = await Promise.all([
      call(commands.studioGameFiles()),
      call(commands.studioPatches(folder)),
      call(commands.studioPatchIds(folder)).catch(() => [] as StudioPatchId[]),
    ]);
    if (this.folder !== folder) return;
    this.files = files;
    this.patches = patches;
    this.ids = ids;
  }

  async reloadPatches(): Promise<void> {
    const folder = this.folder;
    if (!folder) return;
    const patches = await call(commands.studioPatches(folder));
    if (this.folder === folder) this.patches = patches;
  }

  async openFile(path: string): Promise<void> {
    const seq = ++this.#fileSeq;
    const file = await call(commands.studioGameFile(path, this.withMods));
    if (seq !== this.#fileSeq) return;
    this.file = file;
    this.selection = null;
    this.matches = null;
  }

  async setWithMods(on: boolean): Promise<void> {
    this.withMods = on;
    const path = this.file?.path;
    if (path) {
      const keep = this.selection;
      await this.openFile(path);
      this.selection = keep;
      this.#countMatches();
    }
    this.schedulePreview();
  }

  /** Chooses lines of the game file; a pattern draft takes them as its pattern. */
  select(range: LineRange | null): void {
    const f = this.file;
    if (!f || !range) {
      this.selection = null;
      this.matches = null;
      return;
    }
    const total = f.text.split("\n").length;
    const from = Math.max(1, Math.min(range.from, range.to, total));
    const to = Math.min(total, Math.max(range.from, range.to));
    this.selection = { from, to };
    const d = this.draft;
    if (d && (d.kind === "pattern" || d.kind === "regex")) {
      if (d.kind === "pattern") d.pattern = dedent(this.selectedText);
      if (d.target.length === 0 || d.target[0] !== f.path) d.target = [f.path];
      this.schedulePreview();
    }
    this.#countMatches();
  }

  /** One more line below the chosen ones (or above, with `up`). */
  extend(up = false): void {
    const s = this.selection;
    if (!s) return;
    this.select(up ? { from: s.from - 1, to: s.to } : { from: s.from, to: s.to + 1 });
  }

  /** One line fewer, from the bottom (or the top, with `up`). */
  shrink(up = false): void {
    const s = this.selection;
    if (!s || s.from === s.to) return;
    this.select(up ? { from: s.from + 1, to: s.to } : { from: s.from, to: s.to - 1 });
  }

  async #countMatches(): Promise<void> {
    const f = this.file;
    const text = this.selectedText;
    if (!f || !text.trim()) {
      this.matches = null;
      return;
    }
    const seq = ++this.#matchSeq;
    try {
      const m = await call(commands.studioMatches(f.path, text, this.withMods), { expect: ["not_found", "conflict"] });
      if (seq === this.#matchSeq) this.matches = m;
    } catch {
      if (seq === this.#matchSeq) this.matches = null;
    }
  }

  /** Starts a new patch of `kind`, on the open file and the chosen lines when there are some. */
  newPatch(kind: StudioPatchKind): void {
    const d = emptyDraft(kind);
    const modId = this.patches?.mod_id ?? "";
    d.id = modId ? `${modId}:` : "";
    if (this.file && kind !== "module") d.target = [this.file.path];
    if (kind === "pattern" && this.selection) d.pattern = dedent(this.selectedText);
    this.draft = d;
    this.draftSeq++;
    this.editing = null;
    this.idTouched = false;
    this.preview = null;
    this.#saved = "";
    this.schedulePreview();
  }

  /** Opens an existing patch in the form, and its game file. */
  async edit(p: StudioPatch): Promise<void> {
    this.draft = structuredClone($state.snapshot(p.draft) as PatchDraft);
    this.draftSeq++;
    this.editing = p.at;
    this.#editingHash = p.hash;
    this.idTouched = true;
    this.preview = null;
    this.#saved = JSON.stringify(this.draft);
    const target = p.draft.target[0];
    if (target && this.files?.files.some((f) => f.path === target)) {
      await this.openFile(target).catch(() => {});
      this.#selectPattern();
    }
    this.schedulePreview();
  }

  /** Selects where an opened pattern patch's lines are, when they are found in the file. */
  #selectPattern(): void {
    const d = this.draft;
    const f = this.file;
    if (!d || !f || d.kind !== "pattern" || !d.pattern.trim()) return;
    const want = d.pattern.split("\n").map((l) => l.trim());
    const lines = f.text.split("\n").map((l) => l.trim());
    for (let i = 0; i + want.length <= lines.length; i++) {
      if (want.every((w, j) => lines[i + j] === w)) {
        this.selection = { from: i + 1, to: i + want.length };
        this.#countMatches();
        return;
      }
    }
  }

  /** The form changed. */
  update(fields: Partial<PatchDraft>): void {
    const d = this.draft;
    if (!d) return;
    Object.assign(d, fields);
    if (fields.name !== undefined && !this.idTouched) {
      d.id = slugId(this.patches?.mod_id ?? "", fields.name);
    }
    if (fields.id !== undefined) this.idTouched = true;
    this.schedulePreview();
  }

  closeDraft(): void {
    clearTimeout(this.#previewTimer);
    this.#previewSeq++;
    this.draft = null;
    this.editing = null;
    this.preview = null;
    this.previewing = false;
    this.#saved = "";
  }

  schedulePreview(delay = PREVIEW_DELAY_MS): void {
    clearTimeout(this.#previewTimer);
    if (!this.draft || !this.folder) return;
    this.#previewTimer = setTimeout(() => void this.#runPreview(), delay);
  }

  async #runPreview(): Promise<void> {
    const folder = this.folder;
    const d = this.draft;
    if (!folder || !d || d.kind === "module" || d.target.length === 0) {
      this.preview = null;
      return;
    }
    const seq = ++this.#previewSeq;
    this.previewing = true;
    try {
      const p = await call(commands.studioPreview(folder, $state.snapshot(d) as PatchDraft, this.editing, this.withMods), {
        expect: ["invalid_input", "not_found", "conflict"],
      });
      if (seq === this.#previewSeq) this.preview = p;
    } catch {
      if (seq === this.#previewSeq) this.preview = null;
    } finally {
      if (seq === this.#previewSeq) this.previewing = false;
    }
  }

  /** Writes the draft into qm.toml. Returns false when it was not saved. */
  async save(): Promise<boolean> {
    const folder = this.folder;
    const d = this.draft;
    if (!folder || !d) return false;
    this.saving = true;
    try {
      const saved = await call(
        commands.studioSave(folder, $state.snapshot(d) as PatchDraft, this.editing, this.editing ? this.#editingHash : (this.patches?.hash ?? null)),
        { expect: ["conflict"] },
      );
      this.editing = saved.at;
      this.#editingHash = saved.hash;
      this.#saved = JSON.stringify(d);
      if (saved.feature_added) toasts.info("Added short_patches to the qm_features in mod.json, since the patch uses the short form.");
      await this.reloadPatches();
      return true;
    } catch (e) {
      if (e instanceof QmCommandError && e.kind === "conflict") {
        toasts.error(`${e.message} The list was read again; check the patch and save again.`);
        await this.reloadPatches().catch(() => {});
      }
      return false;
    } finally {
      this.saving = false;
    }
  }

  async remove(at: StudioPatchRef): Promise<void> {
    const folder = this.folder;
    if (!folder) return;
    const p = this.patches?.patches.find((x) => sameRef(x.at, at));
    await call(commands.studioRemove(folder, at, p?.hash ?? this.patches?.hash ?? null));
    if (this.editing && sameRef(this.editing, at)) this.closeDraft();
    await this.reloadPatches();
  }

  /** Settles an unsaved draft: true when there is none or it may be dropped. */
  confirmDrop(): boolean {
    return !this.dirty;
  }
}

export function sameRef(a: StudioPatchRef, b: StudioPatchRef): boolean {
  return a.file === b.file && a.form === b.form && a.kind === b.kind && a.index === b.index;
}

export const studio = new StudioStore();
