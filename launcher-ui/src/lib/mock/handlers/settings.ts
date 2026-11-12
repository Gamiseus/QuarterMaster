import type { DetectedPaths, SettingsPatch, SettingsView } from "../../bindings";
import { emitMock } from "../../events";
import { settings } from "../fixtures";
import { runMockJob } from "../jobs";
import { ensureNotRunning, fail, log, type Handlers } from "../state";

const THEMES = ["system", "dark", "light"];

const detected: DetectedPaths = {
  game_exe_path: "C:\\Games\\Mud and Blood\\mnb.exe",
  save_dir_path: "C:\\Users\\Player\\AppData\\Roaming\\mnb",
  workspace_mods_dir: null,
  library_dir_path: "C:\\QuarterMaster\\library",
  is_steam: true,
  game_version: "3.0.0",
};

/** Same rules as the service's `validate`. */
function validate(p: SettingsPatch): void {
  if (p.game_exe_path !== null) {
    const name = p.game_exe_path.split(/[\\/]/).pop() ?? "";
    if (name.toLowerCase() !== "mnb.exe") {
      fail("invalid_input", `Select mnb.exe from the Mud and Blood folder (${p.game_exe_path} is not it).`);
    }
  }
  if (p.theme !== null && !THEMES.includes(p.theme)) {
    fail("invalid_input", `Theme must be system, dark or light (got "${p.theme}").`);
  }
  if (p.max_save_backups !== null && (p.max_save_backups < 1 || p.max_save_backups > 100)) {
    fail("invalid_input", "Keep between 1 and 100 save backups.");
  }
}

/** What e2e specs read back: the fields the patch set. */
function logged(p: SettingsPatch, onboarding: boolean) {
  const entry: Record<string, unknown> = { onboarding };
  for (const [k, v] of Object.entries(p)) if (v !== null) entry[k] = v;
  return entry;
}

function apply(p: SettingsPatch, onboarding: boolean): SettingsView {
  validate(p);
  if (p.game_exe_path !== null && p.game_exe_path !== settings.game_exe_path) ensureNotRunning();
  log.settingsPatches.push(logged(p, onboarding));
  for (const [k, v] of Object.entries(p)) {
    if (v !== null && k !== "clear_workspace_mods_dir") (settings as Record<string, unknown>)[k] = v;
  }
  if (p.clear_workspace_mods_dir) settings.workspace_mods_dir = null;
  if (onboarding) settings.first_run_completed = true;
  setTimeout(() => emitMock({ type: "invalidated", scope: "settings" }), 0);
  return { ...settings };
}

export const handlers: Handlers = {
  settings_get: () => ({ ...settings }),
  settings_update: (args) => apply(args.patch as SettingsPatch, false),
  paths_auto_detect: () => ({ ...detected }),
  onboarding_complete: (args) => apply(args.patch as SettingsPatch, true),
  workspace_sync: () => {
    if (!settings.workspace_mods_dir) fail("not_found", "The workspace mods folder is not set.");
    ensureNotRunning();
    return runMockJob("workspace_sync", "Syncing workspace mods", () => {
      emitMock({ type: "invalidated", scope: "mods" });
      return { output: "gcore, gmenu" };
    });
  },
  mod_auto_update_set: (args) => {
    const id = String(args.modId);
    const map = { ...settings.auto_update_mods };
    if (args.enabled === null || args.enabled === undefined) delete map[id];
    else map[id] = Boolean(args.enabled);
    settings.auto_update_mods = map;
    setTimeout(() => emitMock({ type: "invalidated", scope: "settings" }), 0);
    return null;
  },
};
