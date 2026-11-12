import { call } from "../api";
import { commands, type JobKind, type Loadable, type ModUpdateInfo, type QmEvent, type UpdatesReport } from "../bindings";
import { app } from "./app.svelte";
import { game } from "./game.svelte";
import { jobs } from "./jobs.svelte";
import { loadOrder } from "./loadorder.svelte";
import { mods } from "./mods.svelte";
import { toasts } from "./toasts.svelte";

/** Automatic checks while the launcher is open (the service checks once at startup). */
export const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
const LIBRARY_JOBS: ReadonlySet<JobKind> = new Set<JobKind>(["mod_import", "catalog_install", "mod_update_install", "workspace_sync", "modpack_import"]);

/** A mod's own automatic update choice: `default` follows the global setting. */
export type AutoUpdateChoice = "default" | "always" | "never";

const key = (u: ModUpdateInfo) => `${u.mod_id}@${u.latest_version}`;

class UpdatesStore {
  report = $state<Loadable<UpdatesReport>>({ status: "loading" });
  /** An update install or Update all started here is still running. */
  installing = $state(false);
  /** Opens the Installed tab filtered to updates; set by the app shell. */
  onReview: (() => void) | null = null;
  #seq = 0;
  /** Updates already installed automatically or offered in a toast; each is handled once. */
  #announced = new Set<string>();
  #readyToast: number | null = null;

  /** Mods with an update that can be installed (a release asset was found). */
  get available(): ModUpdateInfo[] {
    if (this.report.status !== "ready") return [];
    return this.report.data.mods.filter((m) => m.update_available && m.download_url);
  }

  /** github.com answered 429 to a fallback check; the rest kept their cached results. */
  get busy(): boolean {
    return this.report.status === "ready" && this.report.data.rate_limit_mode === "busy";
  }

  async load(): Promise<void> {
    const seq = ++this.#seq;
    const report = await call(commands.updatesGet());
    if (seq !== this.#seq) return;
    this.report = report;
    this.announce();
  }

  /** Starts the periodic check; the returned function stops it. */
  schedule(): () => void {
    const timer = setInterval(() => this.refresh().catch(() => {}), CHECK_INTERVAL_MS);
    return () => clearInterval(timer);
  }

  /** The per-mod choice, from the settings' `auto_update_mods` map. */
  choiceFor(modId: string): AutoUpdateChoice {
    const v = app.settings?.auto_update_mods[modId];
    return v === undefined ? "default" : v ? "always" : "never";
  }

  async setChoice(modId: string, choice: AutoUpdateChoice): Promise<void> {
    await call(commands.modAutoUpdateSet(modId, choice === "default" ? null : choice === "always"));
  }

  /** Updates the user can take: workspace-linked mods are the user's own files and never replaced. */
  get eligible(): ModUpdateInfo[] {
    const list = mods.list.status === "ready" ? mods.list.data : [];
    return this.available.filter((u) => !list.find((m) => m.id === u.mod_id)?.is_workspace_linked);
  }

  /**
   * Handles updates not seen before: installs the ones set to update automatically while nothing
   * blocks it, and offers the rest in one toast (replacing an earlier one).
   */
  announce(): void {
    const s = app.settings;
    // Nothing during the first-run dialog; the next settings load after it runs this again.
    if (!s || !s.first_run_completed || !app.startupDone || this.report.status !== "ready") return;
    const fresh = this.eligible.filter((u) => !this.#announced.has(key(u)));
    if (fresh.length === 0) return;
    for (const u of fresh) this.#announced.add(key(u));
    const auto = fresh.filter((u) => s.auto_update_mods[u.mod_id] ?? s.auto_install_updates);
    const idle = !game.isBusy && !this.installing && !jobs.list.some((j) => LIBRARY_JOBS.has(j.kind));
    const now = idle ? auto : [];
    const ask = fresh.filter((u) => !now.includes(u));
    if (now.length > 0) void this.installMods(now.map((u) => u.mod_id));
    if (ask.length > 0) this.#offer(ask.map((u) => u.mod_id));
  }

  #offer(ids: string[]): void {
    if (this.#readyToast !== null) toasts.dismiss(this.#readyToast);
    const n = ids.length;
    this.#readyToast = toasts.notice(n === 1 ? "1 mod update is ready." : `${n} mod updates are ready.`, [
      { label: "Install", run: () => void this.installMods(ids) },
      { label: "Review", run: () => this.onReview?.() },
    ]);
  }

  /** Checks again (the mod index, then github.com for mods it does not list); failures offer Retry. */
  async refresh(): Promise<void> {
    const retry = () => this.refresh().catch(() => {});
    const id = await call(commands.updatesRefresh(), { retry });
    await jobs.track(id, { label: "Checking for updates", success: () => "Update check finished.", retry });
  }

  /** Installs one mod's update and moves the active profile to it. Rejects on failure (already reported). */
  async install(modId: string): Promise<void> {
    const update = this.available.find((m) => m.mod_id === modId);
    const name = update?.name ?? modId;
    const version = update?.latest_version;
    const retry = () => this.install(modId).catch(() => {});
    const nested = this.installing;
    this.installing = true;
    try {
      await loadOrder.whileChanging(async () => {
        const id = await call(commands.modUpdateInstall(modId), { retry });
        await jobs.track(id, {
          label: `Updating ${name}`,
          success: () => (version ? `Updated ${name} to ${version}.` : `Updated ${name}.`),
          retry,
        });
      });
    } finally {
      if (!nested) this.installing = false;
    }
  }

  /** Installs every available update, one at a time; stops at the first failure. */
  installAll(): Promise<void> {
    return this.installMods(this.available.map((m) => m.mod_id));
  }

  /** Installs these mods' updates one at a time (skipping any no longer available); stops at the first failure. */
  async installMods(modIds: string[]): Promise<void> {
    if (this.installing) return;
    const ids = modIds.filter((id) => this.available.some((m) => m.mod_id === id));
    if (ids.length === 0) return;
    this.installing = true;
    try {
      for (const id of ids) await this.install(id);
    } catch {
      // The failed update already showed its error; the rest are skipped.
    } finally {
      this.installing = false;
    }
  }

  /** Test-only. */
  reset(): void {
    this.#seq++;
    this.report = { status: "loading" };
    this.installing = false;
    this.#announced.clear();
    this.#readyToast = null;
  }

  handle(e: QmEvent): void {
    if (e.type === "invalidated" && e.scope === "updates") this.load().catch(() => {});
  }
}

export const updates = new UpdatesStore();
