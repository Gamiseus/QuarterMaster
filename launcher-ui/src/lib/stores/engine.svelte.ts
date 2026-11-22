import { call, QmCommandError } from "../api";
import {
  commands,
  type EngineDiagnostic,
  type EngineEntry,
  type EngineLibraryMod,
  type EngineLint,
  type EngineProjects,
  type EngineSymbol,
  type EngineTestPlan,
  type QmEvent,
  type TemplateInfo,
} from "../bindings";
import { firstFile, languageOf } from "../engine/text";
import { app } from "./app.svelte";
import { game } from "./game.svelte";
import { sprites } from "./sprites.svelte";
import { studio } from "./studio.svelte";
import { toasts } from "./toasts.svelte";

/** How long typing pauses before the fast checks run. */
export const CHECK_DELAY_MS = 300;

/** What the editor shows: a workspace mod (editable) or a library mod (read-only). */
export type EngineTarget = { kind: "workspace"; folder: string } | { kind: "library"; id: string; version: string; name: string };

export interface OpenFile {
  /** The file's path inside its mod, with `/`. */
  path: string;
  text: string;
  /** What is on disk as far as the editor knows. */
  savedText: string;
  hash: string;
  crlf: boolean;
  bom: boolean;
  readOnly: boolean;
  /** Fast-check problems for the current text. */
  problems: EngineDiagnostic[];
  /** False for Lua without the game's compiler. */
  luaChecked: boolean;
  /** Bumped to make the editor take `text` as a new document (open, reload). */
  docVersion: number;
  /** The file changed on disk while it had unsaved edits. */
  changedOnDisk: boolean;
}

export function isDirty(f: OpenFile): boolean {
  return !f.readOnly && f.text !== f.savedText;
}

class EngineStore {
  projects = $state<EngineProjects | null>(null);
  library = $state<EngineLibraryMod[]>([]);
  target = $state<EngineTarget | null>(null);
  tree = $state<EngineEntry[]>([]);
  treeTruncated = $state(false);
  files = $state<OpenFile[]>([]);
  active = $state<string | null>(null);
  /** The last full check of the target, when it is a workspace mod. */
  lint = $state<EngineLint | null>(null);
  linting = $state(false);
  symbols = $state<EngineSymbol[]>([]);
  /** Files, the Patch Studio, the sprite line-up or the Release helper (workspace mods only). */
  view = $state<"files" | "studio" | "sprites" | "release">("files");
  /** What a test of the open workspace mod would run. */
  testPlan = $state<EngineTestPlan | null>(null);
  /** The test run's log panel is shown. */
  showTest = $state(false);
  /** A request to show a line of a file; the editor pane carries it out. */
  jumpRequest = $state<{ path: string; line: number; seq: number } | null>(null);
  #jumpSeq = 0;
  /** The mod template catalog, read when first needed. */
  templates = $state<TemplateInfo[]>([]);
  /** The template dialog: a new mod from a template, or an example added to the open mod. */
  templateDialog = $state<"new" | "add" | null>(null);
  /** Set while the unsaved-changes dialog waits for an answer. */
  pendingLeave = $state<{ resolve: (ok: boolean) => void } | null>(null);
  #checkTimers = new Map<string, ReturnType<typeof setTimeout>>();
  #checkSeq = new Map<string, number>();
  #loadSeq = 0;

  get activeFile(): OpenFile | null {
    return this.files.find((f) => f.path === this.active) ?? null;
  }

  /** Unsaved file edits, a patch in the studio or a sprite's place not saved yet. */
  get hasUnsaved(): boolean {
    return this.files.some(isDirty) || studio.dirty || sprites.dirty;
  }

  get folder(): string | null {
    return this.target?.kind === "workspace" ? this.target.folder : null;
  }

  async load(): Promise<void> {
    const seq = ++this.#loadSeq;
    const [projects, library] = await Promise.all([call(commands.engineProjects()), call(commands.engineLibraryMods())]);
    if (seq !== this.#loadSeq) return;
    this.projects = projects;
    this.library = library;
    // The target may have been deleted or renamed outside the launcher.
    if (this.target?.kind === "workspace" && !projects.projects.some((p) => p.folder === this.folder)) {
      if (!this.hasUnsaved) this.clear();
    }
    if (this.symbols.length === 0) {
      call(commands.engineSymbols())
        .then((s) => (this.symbols = s))
        .catch(() => {});
    }
  }

  /**
   * Settles unsaved edits before they would be lost (another mod, another screen, closing the
   * window): true at once when there are none, else the answer of the unsaved-changes dialog.
   */
  confirmLeave(): Promise<boolean> {
    if (!this.hasUnsaved) return Promise.resolve(true);
    this.pendingLeave?.resolve(false);
    return new Promise((resolve) => {
      this.pendingLeave = {
        resolve: (ok) => {
          this.pendingLeave = null;
          resolve(ok);
        },
      };
    });
  }

  /** Drops every unsaved edit; the files on disk stay as they are. */
  discardAll(): void {
    studio.closeDraft();
    sprites.discard();
    for (const f of this.files) {
      if (!isDirty(f)) continue;
      f.text = f.savedText;
      f.changedOnDisk = false;
      f.docVersion++;
    }
  }

  clear(): void {
    studio.reset();
    sprites.reset();
    this.view = "files";
    this.testPlan = null;
    this.target = null;
    this.tree = [];
    this.files = [];
    this.active = null;
    this.lint = null;
  }

  /**
   * Opens a mod with one file open: `path` when the mod has it, else its first file. Unsaved
   * edits are the caller's to settle first.
   */
  async select(target: EngineTarget, path?: string): Promise<void> {
    this.clear();
    this.target = target;
    await this.loadTree();
    const files = this.tree.filter((e) => !e.folder).map((e) => e.path);
    const first = path && files.includes(path) ? path : firstFile(files);
    if (first) await this.open(first);
  }

  async loadTree(): Promise<void> {
    const t = this.target;
    if (!t) return;
    if (t.kind === "workspace") {
      const tree = await call(commands.engineTree(t.folder));
      if (this.target !== t) return;
      this.tree = tree.entries;
      this.treeTruncated = tree.truncated;
    } else {
      const list = await call(commands.modFiles(t.id));
      if (this.target !== t) return;
      const folders = new Set<string>();
      for (const f of list.files) {
        const parts = f.path.split("/");
        for (let i = 1; i < parts.length; i++) folders.add(parts.slice(0, i).join("/"));
      }
      this.tree = [
        ...[...folders].map((p) => ({ path: p, folder: true, size_bytes: 0, kind: "other" as const })),
        ...list.files.map((f) => ({ path: f.path, folder: false, size_bytes: f.size_bytes, kind: kindOf(f.path) })),
      ].sort((a, b) => a.path.toLowerCase().localeCompare(b.path.toLowerCase()));
      this.treeTruncated = list.total_files > list.files.length;
    }
  }

  /** Opens a file in a tab, or shows its tab when it is open. */
  async open(path: string): Promise<void> {
    if (this.files.some((f) => f.path === path)) {
      this.active = path;
      return;
    }
    const t = this.target;
    if (!t) return;
    let file: OpenFile;
    if (t.kind === "workspace") {
      const f = await call(commands.engineFileRead(t.folder, path));
      file = { ...f, savedText: f.text, readOnly: false, problems: [], luaChecked: true, docVersion: 0, changedOnDisk: false };
    } else {
      const v = await call(commands.fileView(t.id, path));
      if (v.kind !== "text" || v.text === null) {
        toasts.info(`${path} is not a text file.`);
        return;
      }
      file = { path, text: v.text, savedText: v.text, hash: "", crlf: false, bom: false, readOnly: true, problems: [], luaChecked: true, docVersion: 0, changedOnDisk: false };
    }
    if (this.target !== t) return;
    this.files = [...this.files, file];
    this.active = path;
    if (!file.readOnly) this.scheduleCheck(path, 0);
  }

  /** The editor's text changed. */
  edit(path: string, text: string): void {
    const f = this.files.find((x) => x.path === path);
    if (!f || f.readOnly || f.text === text) return;
    f.text = text;
    this.scheduleCheck(path, CHECK_DELAY_MS);
  }

  scheduleCheck(path: string, delay: number): void {
    const folder = this.folder;
    if (!folder) return;
    clearTimeout(this.#checkTimers.get(path));
    this.#checkTimers.set(
      path,
      setTimeout(() => void this.#check(folder, path), delay),
    );
  }

  async #check(folder: string, path: string): Promise<void> {
    const f = this.files.find((x) => x.path === path);
    if (!f || this.folder !== folder) return;
    const seq = (this.#checkSeq.get(path) ?? 0) + 1;
    this.#checkSeq.set(path, seq);
    try {
      const result = await call(commands.engineCheck(folder, path, f.text), { expect: ["invalid_input", "not_found"] });
      // An answer for older text is dropped; a newer check is on its way.
      if (this.#checkSeq.get(path) !== seq || this.folder !== folder) return;
      const now = this.files.find((x) => x.path === path);
      if (!now) return;
      now.problems = result.diagnostics;
      now.luaChecked = result.lua_checked;
    } catch {
      // A file that cannot be checked keeps its last problems.
    }
  }

  /** Saves one file. Returns false when it was not saved (changed on disk, or an error). */
  async save(path: string, overwrite = false): Promise<boolean> {
    const folder = this.folder;
    const f = this.files.find((x) => x.path === path);
    if (!folder || !f || f.readOnly) return false;
    try {
      const saved = await call(
        commands.engineFileWrite(folder, {
          path,
          text: f.text,
          expected_hash: overwrite ? null : f.hash,
          crlf: f.crlf,
          bom: f.bom,
        }),
        { expect: ["conflict"] },
      );
      const now = this.files.find((x) => x.path === path);
      if (now) {
        now.savedText = saved.text === now.text ? now.text : saved.text;
        now.hash = saved.hash;
        now.changedOnDisk = false;
      }
      return true;
    } catch (e) {
      if (e instanceof QmCommandError && e.kind === "conflict") {
        const now = this.files.find((x) => x.path === path);
        if (now) now.changedOnDisk = true;
        toasts.error(`${path} changed on disk since you opened it. Reload it, or keep yours and save over it.`);
      }
      return false;
    }
  }

  /** Saves every file with unsaved edits, then runs the mod's full check once. */
  async saveAll(): Promise<void> {
    const dirty = this.files.filter(isDirty).map((f) => f.path);
    let any = false;
    for (const p of dirty) any = (await this.save(p)) || any;
    if (studio.dirty) any = (await studio.save()) || any;
    if (sprites.dirty) any = (await sprites.save()) || any;
    if (any) await this.runLint();
  }

  /** Saves the file and runs the full check. */
  async saveAndCheck(path: string): Promise<void> {
    // The fast checks catch up at once instead of after the typing pause.
    this.scheduleCheck(path, 0);
    if (await this.save(path)) await this.runLint();
  }

  /** Reads the file from disk again, dropping unsaved edits. */
  async reload(path: string): Promise<void> {
    const folder = this.folder;
    if (!folder) return;
    try {
      const f = await call(commands.engineFileRead(folder, path), { expect: ["not_found"] });
      const now = this.files.find((x) => x.path === path);
      if (!now) return;
      Object.assign(now, { text: f.text, savedText: f.text, hash: f.hash, crlf: f.crlf, bom: f.bom, changedOnDisk: false });
      now.docVersion++;
      this.scheduleCheck(path, 0);
    } catch {
      // Deleted on disk: the tab stays with its text, marked unsaved.
      const now = this.files.find((x) => x.path === path);
      if (now) {
        now.savedText = "";
        now.hash = "";
      }
    }
  }

  close(path: string): void {
    clearTimeout(this.#checkTimers.get(path));
    const i = this.files.findIndex((f) => f.path === path);
    if (i < 0) return;
    this.files = this.files.filter((f) => f.path !== path);
    if (this.active === path) this.active = this.files[Math.max(0, i - 1)]?.path ?? null;
  }

  async runLint(): Promise<void> {
    const folder = this.folder;
    if (!folder) return;
    this.linting = true;
    try {
      const result = await call(commands.engineLint(folder));
      if (this.folder === folder) this.lint = result;
      call(commands.engineProjects())
        .then((p) => (this.projects = p))
        .catch(() => {});
    } catch {
      // call() reported it.
    } finally {
      this.linting = false;
    }
  }

  async create(path: string, folder: boolean): Promise<void> {
    const mod = this.folder;
    if (!mod) return;
    await call(commands.engineFileCreate(mod, path, folder));
    await this.loadTree();
    if (!folder) await this.open(path);
  }

  async rename(from: string, to: string): Promise<void> {
    const mod = this.folder;
    if (!mod) return;
    await call(commands.engineFileRename(mod, from, to));
    // Open tabs under the old name follow it.
    for (const f of this.files) {
      if (f.path === from || f.path.startsWith(`${from}/`)) {
        const next = to + f.path.slice(from.length);
        if (this.active === f.path) this.active = next;
        f.path = next;
      }
    }
    await this.loadTree();
  }

  async remove(path: string): Promise<void> {
    const mod = this.folder;
    if (!mod) return;
    await call(commands.engineFileDelete(mod, path));
    for (const f of [...this.files]) if (f.path === path || f.path.startsWith(`${path}/`)) this.close(f.path);
    await this.loadTree();
  }

  /** Copies a library mod into the workspace and opens the copy. */
  async copyToWorkspace(mod: EngineLibraryMod): Promise<void> {
    const folder = await call(commands.engineCopyToWorkspace(mod.id, mod.version));
    toasts.info(`Copied ${mod.name} into the workspace as ${folder}. Edits go to the copy.`);
    await this.load();
    await this.select({ kind: "workspace", folder });
  }

  /**
   * Opens the template dialog once unsaved edits are settled: a new mod would close the open
   * one, and an example may add to its open files.
   */
  async openTemplates(mode: "new" | "add"): Promise<void> {
    if (!(await this.confirmLeave())) return;
    this.templateDialog = mode;
  }

  async loadTemplates(): Promise<void> {
    if (this.templates.length === 0) this.templates = await call(commands.templatesList());
  }

  /**
   * Makes a workspace mod from a template and opens it at the template's first file. Unsaved
   * edits are settled first; false when the user kept them. Name and id problems are thrown
   * for the dialog to show.
   */
  async createFromTemplate(t: TemplateInfo, name: string, modId: string): Promise<boolean> {
    if (!(await this.confirmLeave())) return false;
    const folder = await call(commands.templateCreate(t.id, name, modId), { expect: ["conflict", "invalid_input"] });
    await this.load();
    this.view = "files";
    await this.select({ kind: "workspace", folder }, t.open);
    toasts.info(`Made ${name} from the template "${t.name}", in the workspace folder ${folder}.`);
    return true;
  }

  /**
   * Adds a template's example to the open workspace mod, then shows its first file and checks
   * the mod. Unsaved edits are settled first; false when the user kept them.
   */
  async addExample(t: TemplateInfo): Promise<boolean> {
    const folder = this.folder;
    if (!folder) return false;
    if (!(await this.confirmLeave())) return false;
    const changed = await call(commands.templateAdd(folder, t.id), { expect: ["conflict"] });
    if (this.folder !== folder) return true;
    await this.loadTree();
    // Tabs of the changed files show what is on disk now.
    for (const p of changed) if (this.files.some((f) => f.path === p)) await this.reload(p);
    const first = changed.includes(t.open) ? t.open : changed.find((p) => p !== "mod.json");
    if (first && this.tree.some((e) => e.path === first)) await this.open(first);
    this.view = "files";
    toasts.info(`Added the example "${t.name}": ${changed.length === 1 ? "1 file" : `${changed.length} files`} changed.`);
    void this.runLint();
    return true;
  }

  /**
   * Shows a mod in the Mod Engine: a workspace mod's folder to edit, else the library copy
   * read-only. Unsaved edits are settled first.
   */
  async openMod(mod: { id: string; name: string; version: string }, workspace: boolean): Promise<void> {
    if (!(await this.confirmLeave())) return;
    if (workspace) {
      if (!this.projects) await this.load();
      const p = this.projects?.projects.find((x) => x.id === mod.id);
      if (p) {
        await this.select({ kind: "workspace", folder: p.folder });
        await app.goTo("engine");
        return;
      }
    }
    await this.select({ kind: "library", id: mod.id, version: mod.version, name: mod.name });
    await app.goTo("engine");
  }

  /** Opens a workspace mod's patch in the Patch Studio (from the mod's Patches tab). */
  async openPatch(mod: { id: string; name: string; version: string }, patchId: string): Promise<void> {
    await this.openMod(mod, true);
    const folder = this.folder;
    if (!folder) return;
    this.view = "studio";
    await studio.load(folder);
    const bare = (id: string) => (id.startsWith(`${mod.id}:`) ? id.slice(mod.id.length + 1) : id);
    const p = studio.patches?.patches.find((x) => x.draft.id === patchId || bare(x.draft.id) === bare(patchId));
    if (p?.editable) await studio.edit(p);
    else if (p) toasts.info(`${p.draft.name || p.draft.id} is in ${p.at.file}; the Patch Studio writes qm.toml, so edit that file in the Files view.`);
  }

  /** Shows an image of the open workspace mod in the sprite line-up. */
  openSprite(path: string): void {
    if (!this.folder) return;
    this.view = "sprites";
    sprites.request = path;
  }

  /** Shows `line` of `path` in the editor (the file opens when it is not). */
  jumpTo(path: string, line: number): void {
    this.jumpRequest = { path, line, seq: ++this.#jumpSeq };
  }

  async loadTestPlan(): Promise<void> {
    const folder = this.folder;
    if (!folder) return;
    const plan = await call(commands.engineTestPlan(folder));
    if (this.folder === folder) this.testPlan = plan;
  }

  /** Runs the test once unsaved edits are settled, so the game runs what the editor shows. */
  async startTest(): Promise<void> {
    const folder = this.folder;
    if (!folder) return;
    if (!(await this.confirmLeave())) return;
    const status = await call(commands.engineTestLaunch(folder), { expect: ["invalid_input", "conflict"] });
    game.started(status);
    this.showTest = true;
  }

  async reveal(): Promise<void> {
    const mod = this.folder;
    if (mod) await call(commands.engineReveal(mod));
  }

  handle(e: QmEvent): void {
    // Nothing to keep current until the Mod Engine has been opened.
    if (this.projects === null) return;
    if (e.type === "invalidated" && (e.scope === "settings" || e.scope === "mods")) this.load().catch(() => {});
    if (e.type !== "workspace_changed") return;
    sprites.handle(e);
    if (this.folder !== e.folder) {
      // Another mod changed: only the list may differ (a new mod.json, a new folder).
      this.load().catch(() => {});
      return;
    }
    this.loadTree().catch(() => {});
    const f = this.files.find((x) => x.path === e.path);
    if (!f) return;
    void this.#onDiskChange(f.path);
  }

  /** A file open in a tab changed on disk: a clean tab reloads, an edited one is marked. */
  async #onDiskChange(path: string): Promise<void> {
    const folder = this.folder;
    if (!folder) return;
    let disk;
    try {
      disk = await call(commands.engineFileRead(folder, path), { expect: ["not_found", "invalid_input"] });
    } catch {
      return;
    }
    const f = this.files.find((x) => x.path === path);
    // Our own save, or nothing new.
    if (!f || disk.hash === f.hash) return;
    if (isDirty(f)) {
      f.changedOnDisk = true;
    } else {
      Object.assign(f, { text: disk.text, savedText: disk.text, hash: disk.hash, crlf: disk.crlf, bom: disk.bom });
      f.docVersion++;
      this.scheduleCheck(path, 0);
    }
  }
}

function kindOf(path: string): EngineEntry["kind"] {
  const l = languageOf(path);
  if (l !== "text") return l;
  const ext = path.toLowerCase().split(".").pop() ?? "";
  if (["png", "jpg", "jpeg", "bmp", "gif", "webp", "tga"].includes(ext)) return "image";
  if (["ogg", "wav", "mp3", "flac"].includes(ext)) return "audio";
  if (["txt", "cfg", "ini", "csv", "glsl"].includes(ext)) return "text";
  return "other";
}

export const engine = new EngineStore();
