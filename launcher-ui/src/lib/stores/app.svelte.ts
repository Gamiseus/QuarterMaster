import { call } from "../api";
import {
  commands,
  type AppInfo,
  type DetectedPaths,
  type QmEvent,
  type RevealKind,
  type SettingsPatch,
  type SettingsView,
} from "../bindings";
import { applyAppearance } from "../settings/appearance";
import { jobs } from "./jobs.svelte";
import { toasts } from "./toasts.svelte";

export type Page = "mods" | "saves" | "diagnostics" | "settings";
export type Mode = "launcher" | "engine";

const STARTUP_POLL_MS = 250;

export function emptyPatch(): SettingsPatch {
  return {
    game_exe_path: null,
    save_dir_path: null,
    workspace_mods_dir: null,
    theme: null,
    default_launch_method: null,
    opt_in_prereleases: null,
    auto_backup_saves: null,
    max_save_backups: null,
    disable_console_on_launch: null,
    debug_mode: null,
    dev_commands: null,
    auto_check_updates: null,
    auto_install_updates: null,
    clear_workspace_mods_dir: null,
    color_vision: null,
    high_contrast: null,
    text_scale: null,
  };
}


class AppStore {
  info = $state<AppInfo | null>(null);
  settings = $state<SettingsView | null>(null);
  page = $state<Page>("mods");
  startupDone = $state(false);
  mode = $state<Mode>("launcher");
  #shownRecoveries = new Set<string>();
  #seq = 0;

  async load(): Promise<void> {
    const seq = ++this.#seq;
    const [info, settings] = await Promise.all([call(commands.appInfo()), call(commands.settingsGet())]);
    if (seq !== this.#seq) return;
    this.info = info;
    this.setSettings(settings);
  }

  setSettings(settings: SettingsView): void {
    this.settings = settings;
    if (settings) applyAppearance(settings);
  }

  async updateSettings(patch: Partial<SettingsPatch>): Promise<void> {
    const settings = await call(commands.settingsUpdate({ ...emptyPatch(), ...patch }));
    this.#seq++; // newer than any in-flight load
    this.setSettings(settings);
  }

  /** Copies the workspace mods folder into the library now (a tracked job). */
  async syncWorkspace(): Promise<void> {
    const id = await call(commands.workspaceSync());
    await jobs.track(id, {
      label: "Syncing workspace mods",
      success: (out) => {
        const n = out ? out.split(", ").length : 0;
        return n === 0 ? "The workspace has no mods to sync." : `Synced ${n} workspace ${n === 1 ? "mod" : "mods"}.`;
      },
    });
  }

  /** Looks for the game and save folders; changes nothing. */
  autoDetect(): Promise<DetectedPaths> {
    return call(commands.pathsAutoDetect());
  }

  /** Saves the onboarding answers (possibly none) and marks the first run done. */
  async completeOnboarding(patch: Partial<SettingsPatch>): Promise<void> {
    const settings = await call(commands.onboardingComplete({ ...emptyPatch(), ...patch }));
    this.#seq++;
    this.setSettings(settings);
  }

  /** Opens one of the app's fixed folders in Explorer; never an arbitrary path. */
  async reveal(kind: RevealKind): Promise<void> {
    await call(commands.reveal(kind));
  }

  /** Polls startup until done and shows a recovery result once. */
  async awaitStartup(): Promise<void> {
    for (;;) {
      const status = await call(commands.startupStatus());
      if (status.done) {
        if (status.recovered) this.showRecovered(status.recovered);
        this.startupDone = true;
        return;
      }
      await new Promise((r) => setTimeout(r, STARTUP_POLL_MS));
    }
  }

  showRecovered(summary: string): void {
    if (this.#shownRecoveries.has(summary)) return;
    this.#shownRecoveries.add(summary);
    toasts.info(summary);
  }

  handle(e: QmEvent): void {
    if (e.type === "invalidated" && e.scope === "settings") this.load().catch(() => {});
    else if (e.type === "recovered") this.showRecovered(e.summary);
  }
}

export const app = new AppStore();
