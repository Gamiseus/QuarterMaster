import { call } from "../api";
import { commands, type ImportRequest, type JobKind } from "../bindings";
import { app } from "./app.svelte";
import { game } from "./game.svelte";
import { jobs } from "./jobs.svelte";
import { updates } from "./updates.svelte";

/** Jobs that hold the backend's install lock (it refuses uninstall and launch meanwhile). */
const INSTALL_KINDS: ReadonlySet<JobKind> = new Set<JobKind>(["mod_import", "catalog_install", "mod_update_install", "workspace_sync", "modpack_import"]);

/** Runs mod installs one request after another; only one install runs at a time. */
class InstallsStore {
  #active = $state(false);

  /** An install started here is still running, or any install job is (also one started elsewhere). */
  get running(): boolean {
    return this.#active || updates.installing || jobs.list.some((j) => INSTALL_KINDS.has(j.kind));
  }

  /** Why installing (or uninstalling) is not possible right now, or null when it is. */
  blockReason(action: "install" | "uninstall" = "install"): string | null {
    if (!app.startupDone) return "Available once startup has finished.";
    if (game.isBusy) return action === "install" ? "Close the game to install mods." : "Close the game to uninstall mods.";
    if (this.running) return "Wait for the install to finish.";
    return null;
  }

  /**
   * Starts each request as a tracked job once the previous one finished. `onstarted` runs when the first
   * job starts; a refusal before that rejects (call() already reported it), later failures stop the rest.
   */
  async run(requests: ImportRequest[], names: (ids: string[]) => string, onstarted: () => void): Promise<void> {
    if (this.#active || requests.length === 0) return;
    this.#active = true;
    let started = false;
    try {
      for (const req of requests) {
        const id = await call(commands.modImport(req));
        if (!started) {
          started = true;
          onstarted();
        }
        const label = names(req.selected_mod_ids ?? []);
        await jobs.track(id, { label: `Installing ${label}`, success: () => `Installed ${label}.` });
      }
    } catch (e) {
      // track() already showed the failure; only a refusal of the first request reaches the caller.
      if (!started) throw e;
    } finally {
      this.#active = false;
    }
  }
}

export const installs = new InstallsStore();
