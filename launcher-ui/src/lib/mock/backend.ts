import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type {
  AppInfo,
  GameStatus,
  HealthReport,
  JobInfo,
  Loadable,
  ModSummary,
  ProfileSummary,
  QmError,
  SaveSlotSummary,
  SettingsPatch,
  SettingsView,
  StartupStatus,
} from "../bindings";
import { emitMock } from "../events";

const ready = <T>(data: T): Loadable<T> => ({ status: "ready", data });

const settings: SettingsView = {
  game_exe_path: "C:\\Games\\Mud and Blood\\mnb.exe",
  save_dir_path: null,
  workspace_mods_dir: null,
  theme: "system",
  default_launch_method: "steam_url",
  opt_in_prereleases: false,
  auto_backup_saves: true,
  max_save_backups: 10,
  disable_console_on_launch: false,
  first_run_completed: true,
  active_profile_id: "default",
  active_save_profile_id: "default",
  portable_mode: false,
  github_token_set: false,
  auto_update_mods: {},
  last_known_exe_sha256: null,
};

const appInfo: AppInfo = {
  version: "1.0.0-mock",
  data_dir: "C:\\Users\\Player\\AppData\\Roaming\\QuarterMaster",
  is_portable: false,
  game: ready({ version: "3.0.0", exe_sha256: "0".repeat(64), lua_file_count: 412 }),
};

const profiles: ProfileSummary[] = [
  {
    id: "default",
    name: "Default",
    description: "",
    enabled_count: 2,
    is_active: true,
    linked_save_profile_id: "default",
    updated_at: "2026-10-30T12:00:00Z",
  },
  {
    id: "vanilla-plus",
    name: "Vanilla Plus",
    description: "",
    enabled_count: 1,
    is_active: false,
    linked_save_profile_id: "default",
    updated_at: "2026-10-29T12:00:00Z",
  },
];

function mod(id: string, name: string, version: string, enabled: boolean): ModSummary {
  return {
    id,
    name,
    version,
    authors: ["Gamiseus"],
    category: null,
    enabled,
    patch_count: 3,
    installed_versions: [version],
    is_workspace_linked: false,
    has_config: false,
  };
}

const mods: ModSummary[] = [
  mod("gcore", "GCore", "1.4.0", true),
  mod("gmenu", "GMenu", "1.2.1", true),
  mod("better-medics", "Better Medics", "0.3.0", false),
];

const saves: SaveSlotSummary[] = [
  {
    profile: { id: "default", name: "Main Campaign", created_at: "2026-10-01T12:00:00Z", last_played_at: null },
    is_active: true,
  },
  {
    profile: { id: "ironman", name: "Ironman", created_at: "2026-10-12T12:00:00Z", last_played_at: null },
    is_active: false,
  },
];

const health: HealthReport = {
  dll_present: true,
  is_game_dir_modded: false,
  source_dll_ready: true,
  dll_sha256: null,
  source_dll_sha256: null,
  dll_matches_build: true,
  game_running_pid: null,
  game_update_detected: null,
  overwrite_files: [],
  steam_cloud_warning: null,
  patch_report: null,
  recent_logs: [],
};

const startup: StartupStatus = { done: true, recovered: null, attached_pid: null };
const jobs: JobInfo[] = [];
let game: GameStatus = { stage: { stage: "idle" }, profile_id: null, is_modded: false };

function fail(kind: QmError["kind"], message: string): never {
  const error: QmError = { kind, message, details: null };
  throw error;
}

function setGame(status: GameStatus): void {
  game = status;
  emitMock({ type: "game_status_changed", status });
}

/** Simulated launch: deploy, wait for Steam, run. */
function launch(profileId: string): GameStatus {
  if (game.stage.stage !== "idle" && game.stage.stage !== "failed") fail("game_running", "The game is already running.");
  const status: GameStatus = { stage: { stage: "deploying" }, profile_id: profileId, is_modded: true };
  game = status;
  setTimeout(() => setGame({ ...status, stage: { stage: "waiting_for_steam" } }), 600);
  setTimeout(() => setGame({ ...status, stage: { stage: "running", pid: 4242 } }), 1400);
  return status;
}

function handle(cmd: string, args: Record<string, unknown> | undefined): unknown {
  switch (cmd) {
    case "app_info":
      return appInfo;
    case "settings_get":
      return settings;
    case "settings_update": {
      const patch = args?.patch as SettingsPatch;
      for (const [k, v] of Object.entries(patch)) {
        if (v !== null && k !== "github_token") (settings as Record<string, unknown>)[k] = v;
      }
      return settings;
    }
    case "startup_status":
      return startup;
    case "profiles_list":
      return profiles;
    case "mods_list":
      return ready(mods);
    case "saves_list":
      return saves;
    case "game_status":
      return game;
    case "game_launch":
      return launch(String((args?.req as { profile_id: string }).profile_id));
    case "health_get":
      return ready(health);
    case "jobs_list":
      return jobs;
    case "logs_tail":
      return [];
    case "plugin:dialog|open":
      return "C:\\Games\\Mud and Blood\\mnb.exe";
    default:
      if (cmd.startsWith("plugin:window|")) return null;
      return fail("internal", `Mock backend has no handler for ${cmd}.`);
  }
}

/** Answers commands with fixture data and exposes `window.__qmMockEmit`. */
export function installMockBackend(): void {
  mockWindows("main");
  mockIPC((cmd, args) => handle(cmd, args as Record<string, unknown> | undefined));
  window.__qmMockEmit = emitMock;
}
