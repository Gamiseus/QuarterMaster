import { call } from "../api";
import { commands, type CatalogEntry, type CatalogQuery, type CatalogResult, type Loadable, type QmEvent } from "../bindings";
import { topicOptions } from "../catalog/format";
import { jobs } from "./jobs.svelte";

const DEBOUNCE_MS = 250;
const EMPTY: CatalogQuery = { text: null, topic: null, source: null };

/** The community catalog for the Browse tab; loads the first time Browse opens. */
class CatalogStore {
  query = $state<CatalogQuery>({ ...EMPTY });
  /** The search box as typed; `query.text` follows it after a short pause. */
  text = $state("");
  result = $state<Loadable<CatalogResult>>({ status: "loading" });
  /** Every topic seen so far, so filtering by one does not hide the others. */
  topics = $state<string[]>([]);
  #seq = 0;
  #started = false;
  #timer: ReturnType<typeof setTimeout> | null = null;
  /** A refresh this store started because the backend had no catalog yet. */
  #filling = false;

  async load(): Promise<void> {
    this.#started = true;
    const seq = ++this.#seq;
    const result = await call(commands.catalogGet({ ...this.query }), { retry: () => this.load() });
    if (seq !== this.#seq) return;
    this.result = result;
    if (result.status === "ready") {
      this.topics = topicOptions([{ topics: this.topics }, ...result.data.catalog]);
    } else if (!this.#filling) {
      // The backend starts a fetch on a miss but nobody reports its failure: track one here.
      this.#filling = true;
      this.refresh()
        .catch(() => {})
        .finally(() => (this.#filling = false));
    }
  }

  ensureLoaded(): void {
    if (!this.#started) this.load().catch(() => {});
  }

  setText(text: string, immediate = false): void {
    this.text = text;
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
    const apply = () => {
      this.#timer = null;
      this.#setQuery({ text: text.trim() || null });
    };
    if (immediate) apply();
    else this.#timer = setTimeout(apply, DEBOUNCE_MS);
  }

  setTopic(topic: string | null): void {
    this.#setQuery({ topic });
  }

  setSource(source: string | null): void {
    this.#setQuery({ source });
  }

  clearFilters(): void {
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
    this.text = "";
    this.#setQuery({ ...EMPTY });
  }

  get filtering(): boolean {
    return this.text.trim() !== "" || this.query.topic !== null || this.query.source !== null;
  }

  async refresh(): Promise<void> {
    const retry = () => this.refresh().catch(() => {});
    const id = await call(commands.catalogRefresh(), { retry });
    await jobs.track(id, { label: "Refreshing the catalog", success: () => "Catalog refreshed.", retry });
  }

  /** Catalog ids are `owner_repo`, not manifest ids, so the backend gets no mod id. */
  async install(entry: CatalogEntry): Promise<void> {
    const url = entry.download_url;
    if (!url) return;
    const retry = () => this.install(entry).catch(() => {});
    const id = await call(commands.catalogInstall(url, null), { retry });
    await jobs.track(id, { label: `Installing ${entry.name}`, success: () => `Installed ${entry.name}.`, retry });
  }

  handle(e: QmEvent): void {
    if (!this.#started || e.type !== "invalidated") return;
    if (e.scope === "catalog" || e.scope === "mods") this.load().catch(() => {});
  }

  #setQuery(patch: Partial<CatalogQuery>): void {
    this.query = { ...this.query, ...patch };
    if (this.#started) this.load().catch(() => {});
  }

  /** Test-only: back to the state before Browse first opened. */
  reset(): void {
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
    this.#seq++;
    this.#started = false;
    this.#filling = false;
    this.query = { ...EMPTY };
    this.text = "";
    this.result = { status: "loading" };
    this.topics = [];
  }
}

export const catalog = new CatalogStore();
