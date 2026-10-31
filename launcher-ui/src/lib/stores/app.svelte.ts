import { call } from "../api";
import { commands, type AppInfo, type QmEvent, type SettingsPatch, type SettingsView } from "../bindings";
import { toasts } from "./toasts.svelte";

export type Page = "play" | "mods" | "saves" | "diagnostics" | "settings";
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
    github_token: null,
  };
}

function applyTheme(theme: string | undefined): void {
  const root = document.documentElement;
  if (theme === "light" || theme === "dark") root.dataset.theme = theme;
  else delete root.dataset.theme;
}

class AppStore {
  info = $state<AppInfo | null>(null);
  settings = $state<SettingsView | null>(null);
  page = $state<Page>("play");
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
    applyTheme(settings?.theme);
  }

  async updateSettings(patch: Partial<SettingsPatch>): Promise<void> {
    const settings = await call(commands.settingsUpdate({ ...emptyPatch(), ...patch }));
    this.#seq++; // newer than any in-flight load
    this.setSettings(settings);
  }

  /** Polls startup until done and shows a recovery result once. */
  async awaitStartup(): Promise<void> {
    for (;;) {
      const status = await call(commands.startupStatus());
      if (status.done) {
        if (status.recovered) this.showRecovered(status.recovered);
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
