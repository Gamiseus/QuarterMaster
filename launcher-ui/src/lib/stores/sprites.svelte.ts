import { call } from "../api";
import {
  commands,
  type QmEvent,
  type SpriteCatalog,
  type SpriteItem,
  type SpritePlace,
  type SpriteRef,
  type SpriteSaved,
  type SpriteShelf,
  type SpriteSource,
  type SpriteUses,
} from "../bindings";
import { comparisonRect, samePlace, setOf, wholeBox, type Size } from "../engine/sprites";

export type Ground = "checker" | "dark" | "light";

/** One of the two images on the canvas. */
export interface Layer {
  item: SpriteItem;
  /** `data:` URL, null while it loads or when it could not be read. */
  url: string | null;
  /** The size the image loaded at (the catalog's when it had none). */
  size: Size | null;
  /** 0 to 100. */
  opacity: number;
  visible: boolean;
  /** Why the image could not be shown. */
  error: string | null;
}

export function refKey(r: SpriteRef): string {
  return `${r.source}:${r.owner}:${r.path}`;
}

export function shelfKey(source: SpriteSource, owner: string): string {
  return `${source}:${owner}`;
}

/** How many undo steps are kept. */
const UNDO = 100;

/** The Mod Engine's sprite line-up: your image over another, and its place in the game's box. */
class SpriteStore {
  folder = $state<string | null>(null);
  shelves = $state<SpriteShelf[]>([]);
  /** Catalogs by shelf key, read when first shown. */
  catalogs = $state<Record<string, SpriteCatalog>>({});
  /** The image being lined up: a file of the mod. */
  mine = $state<Layer | null>(null);
  /** Where the game shows it, read for `mine`. */
  uses = $state<SpriteUses | null>(null);
  useIndex = $state(0);
  /** The image it is compared with. */
  other = $state<Layer | null>(null);
  otherOnTop = $state(true);
  /** Your image's place now, and as saved. */
  place = $state<SpritePlace | null>(null);
  saved = $state<SpritePlace | null>(null);
  /** Corners keep the shape. */
  keepShape = $state(true);
  ground = $state<Ground>("checker");
  /** Shows one layer then the other, to spot what differs. */
  blinking = $state(false);
  saving = $state(false);
  /** What the last save wrote, until the place changes again. */
  lastSaved = $state<SpriteSaved | null>(null);
  /** An image of the mod to show next (from the Files view); the view picks it up. */
  request = $state<string | null>(null);
  /** Undo and redo steps kept, for the buttons. */
  undoSize = $state(0);
  redoSize = $state(0);
  #undo: SpritePlace[] = [];
  #redo: SpritePlace[] = [];
  #images = new Map<string, string>();
  #seq = 0;

  get use() {
    return this.uses?.uses[this.useIndex] ?? null;
  }

  /** The box the game's code sees; your image's own size when the game does not load it. */
  get box(): Size | null {
    const u = this.use;
    if (u) return { w: u.box_width, h: u.box_height };
    return this.mine?.size ?? null;
  }

  get dirty(): boolean {
    return this.use !== null && this.place !== null && !samePlace(this.place, this.saved);
  }

  #counts(): void {
    this.undoSize = this.#undo.length;
    this.redoSize = this.#redo.length;
  }

  /** The comparison's rectangle in the box, centred on the box's middle. */
  get otherRect() {
    const box = this.box;
    const size = this.other?.size;
    if (!box || !size) return null;
    return comparisonRect(box, size);
  }

  /** Clears everything for another mod. */
  reset(): void {
    this.#seq++;
    this.folder = null;
    this.shelves = [];
    this.catalogs = {};
    this.mine = null;
    this.uses = null;
    this.useIndex = 0;
    this.other = null;
    this.place = null;
    this.saved = null;
    this.lastSaved = null;
    this.blinking = false;
    this.request = null;
    this.#undo = [];
    this.#redo = [];
    this.#counts();
    // Game images stay cached; a mod's are read again.
    for (const k of [...this.#images.keys()]) if (!k.startsWith("game:")) this.#images.delete(k);
  }

  async load(folder: string): Promise<void> {
    if (this.folder !== folder) {
      // An image asked for from the Files view is kept for the view to show.
      const asked = this.request;
      this.reset();
      this.folder = folder;
      this.request = asked;
    }
    const shelves = await call(commands.engineSpriteShelves(folder));
    if (this.folder === folder) this.shelves = shelves;
  }

  async catalog(source: SpriteSource, owner: string, fresh = false): Promise<SpriteCatalog> {
    const key = shelfKey(source, owner);
    const have = this.catalogs[key];
    if (have && !fresh) return have;
    const c = await call(commands.engineSpriteCatalog(source, owner));
    this.catalogs = { ...this.catalogs, [key]: c };
    return c;
  }

  /** The image's `data:` URL; the game's are kept, a mod's read again with `fresh`. */
  async image(r: SpriteRef, fresh = false): Promise<string> {
    const key = refKey(r);
    const have = this.#images.get(key);
    if (have && !fresh) return have;
    const url = await call(commands.engineSpriteImage(r), { expect: ["not_found", "invalid_input"] });
    this.#images.set(key, url);
    return url;
  }

  async #layer(item: SpriteItem, opacity: number): Promise<Layer> {
    const layer: Layer = {
      item,
      url: null,
      size: item.width > 0 && item.height > 0 ? { w: item.width, h: item.height } : null,
      opacity,
      visible: true,
      error: null,
    };
    try {
      layer.url = await this.image(item.sprite);
      const natural = await naturalSize(layer.url);
      if (natural) layer.size = natural;
    } catch (e) {
      layer.error = e instanceof Error ? e.message : String(e);
    }
    return layer;
  }

  /** Picks the mod's image to line up, finds where the game shows it, and puts the game's
   * image there under it. Unsaved changes are the caller's to settle first. */
  async pickMine(path: string): Promise<void> {
    const folder = this.folder;
    if (!folder) return;
    const seq = ++this.#seq;
    const item: SpriteItem = {
      sprite: { source: "workspace", owner: folder, path },
      name: path.split("/").pop()?.replace(/\.[^.]+$/, "") ?? path,
      width: 0,
      height: 0,
    };
    const [layer, uses] = await Promise.all([
      this.#layer(item, this.mine?.opacity ?? 100),
      call(commands.engineSpriteUses(folder, path)),
    ]);
    if (seq !== this.#seq || this.folder !== folder) return;
    if (uses.file.width > 0) layer.item = uses.file;
    this.mine = layer;
    this.uses = uses;
    this.useIndex = 0;
    this.lastSaved = null;
    this.#undo = [];
    this.#redo = [];
    this.#counts();
    await this.#applyUse(seq);
  }

  /** Shows use `i` of your image: its saved place, and the game's image there to compare. */
  async chooseUse(i: number): Promise<void> {
    if (!this.uses || i < 0 || i >= this.uses.uses.length) return;
    this.useIndex = i;
    this.lastSaved = null;
    this.#undo = [];
    this.#redo = [];
    this.#counts();
    await this.#applyUse(++this.#seq);
  }

  async #applyUse(seq: number): Promise<void> {
    const u = this.use;
    const box = this.box;
    this.saved = u ? { ...u.place } : null;
    this.place = u ? { ...u.place } : box ? wholeBox(box) : null;
    if (u?.original) {
      const keep = this.other;
      const layer = await this.#layer(u.original, keep?.opacity ?? 50);
      if (seq !== this.#seq) return;
      this.other = layer;
    }
  }

  async pickOther(item: SpriteItem): Promise<void> {
    const seq = this.#seq;
    const layer = await this.#layer(item, this.other?.opacity ?? 50);
    if (seq !== this.#seq) return;
    layer.visible = true;
    this.other = layer;
  }

  /** The numbered set the layer's image belongs to, from its shelf's catalog. */
  async setFor(layer: Layer | null): Promise<SpriteItem[]> {
    if (!layer) return [];
    const s = layer.item.sprite;
    try {
      const c = await this.catalog(s.source, s.owner);
      return setOf(layer.item, c.items);
    } catch {
      return [layer.item];
    }
  }

  /** Changes the place; `record` keeps the old one for Undo (once per drag, not per move). */
  setPlace(p: SpritePlace, record = true): void {
    if (this.place && record && !samePlace(this.place, p)) {
      this.#undo.push({ ...this.place });
      if (this.#undo.length > UNDO) this.#undo.shift();
      this.#redo = [];
      this.#counts();
    }
    this.place = p;
    this.lastSaved = null;
  }

  /** Keeps the place before a drag, for Undo. */
  beginChange(): void {
    if (!this.place) return;
    this.#undo.push({ ...this.place });
    if (this.#undo.length > UNDO) this.#undo.shift();
    this.#redo = [];
    this.#counts();
    this.lastSaved = null;
  }

  undo(): void {
    const p = this.#undo.pop();
    if (!p || !this.place) return;
    this.#redo.push({ ...this.place });
    this.place = p;
    this.lastSaved = null;
    this.#counts();
  }

  redo(): void {
    const p = this.#redo.pop();
    if (!p || !this.place) return;
    this.#undo.push({ ...this.place });
    this.place = p;
    this.lastSaved = null;
    this.#counts();
  }

  /** Back to the saved place. */
  discard(): void {
    if (this.saved && this.place) this.place = { ...this.saved };
  }

  /** Writes the place into the mod. False when it could not be saved. */
  async save(): Promise<boolean> {
    const folder = this.folder;
    const u = this.use;
    const p = this.place;
    if (!folder || !u || !p) return false;
    this.saving = true;
    try {
      const saved = await call(
        commands.engineSpriteSave(folder, { global: u.global, target: u.target, place: p, box_width: u.box_width, box_height: u.box_height }),
      );
      this.saved = { ...p };
      this.lastSaved = saved;
      // The use now carries the saved place.
      const uses = this.uses;
      if (uses) {
        const now = uses.uses[this.useIndex];
        if (now) {
          now.place = { ...p };
          now.placed = !samePlace(p, wholeBox({ w: now.box_width, h: now.box_height }));
        }
      }
      return true;
    } catch {
      return false;
    } finally {
      this.saving = false;
    }
  }

  /** A file of the open mod changed on disk: your image or the comparison shows it, and a
   * place written by hand is read in when nothing here is unsaved. */
  handle(e: QmEvent): void {
    if (e.type !== "workspace_changed" || e.folder !== this.folder) return;
    const path = e.path;
    for (const which of ["mine", "other"] as const) {
      const layer = this[which];
      const s = layer?.item.sprite;
      if (!layer || !s || s.source !== "workspace" || s.owner !== e.folder || s.path !== path) continue;
      void this.image(s, true)
        .then(async (url) => {
          const natural = await naturalSize(url);
          const now = this[which];
          if (now && refKey(now.item.sprite) === refKey(s)) {
            now.url = url;
            if (natural) now.size = natural;
            now.error = null;
          }
        })
        .catch(() => {});
    }
    // A new image in the mod shows in the picker.
    const key = shelfKey("workspace", e.folder);
    if (this.catalogs[key] && /\.(png|jpe?g|bmp)$/i.test(path)) {
      void this.catalog("workspace", e.folder, true).catch(() => {});
    }
    const lower = path.toLowerCase();
    if ((lower === "qm.toml" || lower === "faction.toml") && this.mine && !this.dirty && !this.saving) {
      void this.#reread();
    }
  }

  async #reread(): Promise<void> {
    const folder = this.folder;
    const mine = this.mine;
    if (!folder || !mine) return;
    try {
      const uses = await call(commands.engineSpriteUses(folder, mine.item.sprite.path), { expect: ["not_found", "invalid_input"] });
      if (this.folder !== folder || this.mine !== mine || this.dirty) return;
      const global = this.use?.global;
      this.uses = uses;
      const i = uses.uses.findIndex((u) => u.global === global);
      this.useIndex = Math.max(0, i);
      const u = this.use;
      if (u) {
        this.saved = { ...u.place };
        this.place = { ...u.place };
      }
    } catch {
      // The file was removed; the view keeps what it shows.
    }
  }
}

/** The pixel size an image loads at. */
export function naturalSize(url: string): Promise<Size | null> {
  if (typeof Image === "undefined") return Promise.resolve(null);
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => resolve(img.naturalWidth > 0 ? { w: img.naturalWidth, h: img.naturalHeight } : null);
    img.onerror = () => resolve(null);
    img.src = url;
  });
}

export const sprites = new SpriteStore();
