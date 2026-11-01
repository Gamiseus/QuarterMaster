import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type {
  ConfigValue,
  ConflictResolutionSuggestion,
  GameStatus,
  HealthReport,
  JobInfo,
  LaunchRequest,
  LoadOrderUpdate,
  ModProfile,
  NewProfile,
  ProfileSettingsPatch,
  ProfileSummary,
  QmError,
  SettingsPatch,
  SwitchPreview,
  StartupStatus,
} from "../bindings";
import { emitMock } from "../events";
import { detailFor, resetConfig, savedConfig } from "./detail";
import {
  appInfo,
  loadOrderFor,
  mods,
  patchesFor,
  profileDocs,
  ready,
  saves,
  settings,
  simulationFor,
  updatesReport,
} from "./fixtures";

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

/** What the mock backend was asked to do; e2e specs read it through `window.__qmMockLog`. */
export interface MockLog {
  lastLoadOrderSet: LoadOrderUpdate | null;
  launches: LaunchRequest[];
}

declare global {
  interface Window {
    __qmMockLog?: MockLog;
  }
}

const log: MockLog = { lastLoadOrderSet: null, launches: [] };

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

function activeProfile(): ModProfile {
  return profileDocs[settings.active_profile_id] ?? fail("not_found", "No active profile.");
}

function ensureNotRunning(): void {
  if (game.stage.stage !== "idle" && game.stage.stage !== "failed") {
    fail("game_running", "The game is running. Close it to change mods.");
  }
}

/** Mirrors the service: a profile write invalidates profiles, load order and mod rows. */
function profileChanged(profile: ModProfile): void {
  profile.updated_at = new Date().toISOString();
  setTimeout(() => {
    emitMock({ type: "invalidated", scope: "profiles" });
    emitMock({ type: "invalidated", scope: "load_order" });
    emitMock({ type: "invalidated", scope: "mods" });
  }, 0);
}

function summaries(): ProfileSummary[] {
  return Object.values(profileDocs).map((p) => ({
    id: p.id,
    name: p.name,
    description: p.description ?? "",
    enabled_count: p.enabled_mods.length,
    is_active: p.id === settings.active_profile_id,
    linked_save_profile_id: p.linked_save_profile_id ?? "default",
    updated_at: p.updated_at,
  }));
}

function profileDoc(id: unknown): ModProfile {
  return profileDocs[String(id)] ?? fail("not_found", `No profile ${String(id)}.`);
}

/** Lowercase id from a name, made unique among the existing profiles. */
function freshId(name: string): string {
  const base = name.trim().toLowerCase().replace(/[^a-z0-9_-]/g, "-") || "profile";
  let id = base;
  for (let n = 2; profileDocs[id]; n++) id = `${base}-${n}`;
  return id;
}

function emitProfiles(): void {
  setTimeout(() => {
    emitMock({ type: "invalidated", scope: "profiles" });
    emitMock({ type: "invalidated", scope: "load_order" });
    emitMock({ type: "invalidated", scope: "mods" });
    emitMock({ type: "invalidated", scope: "saves" });
  }, 0);
}

function addProfile(doc: ModProfile): ModProfile {
  profileDocs[doc.id] = doc;
  emitProfiles();
  return doc;
}

function newDoc(id: string, name: string, base?: Partial<ModProfile>): ModProfile {
  const stamp = new Date().toISOString();
  return {
    enabled_mods: [],
    separators: [],
    mod_priorities: {},
    linked_save_profile_id: "default",
    ...base,
    id,
    name,
    created_at: stamp,
    updated_at: stamp,
  };
}

function previewSwitch(id: string): SwitchPreview {
  const from = activeProfile();
  const to = profileDoc(id);
  const added = to.enabled_mods.filter((m) => !from.enabled_mods.includes(m));
  const removed = from.enabled_mods.filter((m) => !to.enabled_mods.includes(m));
  const sharedSave = (to.linked_save_profile_id ?? "default") === (from.linked_save_profile_id ?? "default");
  const warning =
    sharedSave && removed.length > 0
      ? "This save slot was last used with mods that the new profile does not enable. Load it carefully."
      : null;
  return {
    added_mods: added,
    removed_mods: removed,
    diff: { profile_a_id: from.id, profile_b_id: to.id, only_in_a: removed, only_in_b: added, priority_diffs: [], setting_diffs: [] },
    save_compatibility: { is_safe: warning === null, missing_mods_from_save: [], is_tainting_vanilla_save: false, warning_message: warning },
  };
}

function updateProfile(id: string, patch: ProfileSettingsPatch): ModProfile {
  const p = profileDoc(id);
  if (id === settings.active_profile_id) ensureNotRunning();
  if (patch.linked_save_profile_id !== null && !saves.some((s) => s.profile.id === patch.linked_save_profile_id)) {
    fail("not_found", `Save slot '${patch.linked_save_profile_id}' does not exist.`);
  }
  if (patch.launch_method !== null) p.launch_method = patch.launch_method === "default" ? null : patch.launch_method;
  if (patch.custom_launch_args !== null) p.custom_launch_args = patch.custom_launch_args;
  if (patch.disable_console !== null) p.disable_console = patch.disable_console;
  if (patch.linked_save_profile_id !== null) p.linked_save_profile_id = patch.linked_save_profile_id;
  p.updated_at = new Date().toISOString();
  emitProfiles();
  return p;
}

function importCode(code: string, newId: string): ModProfile {
  if (!code.startsWith("QM1-")) fail("invalid_input", "That is not a QuarterMaster share code.");
  if (!/^[A-Za-z0-9_-]+$/.test(newId)) fail("invalid_input", "Profile ID may only contain letters, digits, '_' and '-'.");
  if (profileDocs[newId]) fail("conflict", `A profile with ID '${newId}' already exists.`);
  let pack: { name: string; enabled_mods: string[] };
  try {
    pack = JSON.parse(atob(code.slice(4)));
  } catch {
    fail("invalid_input", "That share code is damaged.");
  }
  return addProfile(newDoc(newId, pack.name, { enabled_mods: pack.enabled_mods }));
}

function setLoadOrder(update: LoadOrderUpdate) {
  ensureNotRunning();
  log.lastLoadOrderSet = structuredClone(update);
  const p = activeProfile();
  p.mod_priorities = { ...update.mod_priorities };
  p.user_load_after = { ...update.user_load_after };
  p.locked_mods = [...update.locked_mods];
  p.separators = update.separators.map((sep) => ({ ...sep }));
  profileChanged(p);
  return loadOrderFor(p);
}

function setEnabled(id: string, enabled: boolean) {
  ensureNotRunning();
  const m = mods.find((x) => x.id === id) ?? fail("not_found", `No mod ${id}.`);
  const p = activeProfile();
  p.enabled_mods = enabled ? [...new Set([...p.enabled_mods, id])] : p.enabled_mods.filter((x) => x !== id);
  profileChanged(p);
  return { ...m, enabled };
}

function setPatchEnabled(patchId: string, enabled: boolean): null {
  ensureNotRunning();
  const p = activeProfile();
  const off = new Set(p.disabled_patches ?? []);
  if (enabled) off.delete(patchId);
  else off.add(patchId);
  p.disabled_patches = [...off];
  profileChanged(p);
  return null;
}

function resolveConflict(s: ConflictResolutionSuggestion, winner: number | null): null {
  ensureNotRunning();
  if (s.tier === "select_winner") {
    if (winner !== 0 && winner !== 1) fail("invalid_input", "Choose which patch wins.");
    return setPatchEnabled(winner === 0 ? s.patch_b_id : s.patch_a_id, false);
  }
  profileChanged(activeProfile());
  return null;
}

/** Simulated launch: deploy, wait for Steam, run. */
function launch(req: LaunchRequest): GameStatus {
  if (game.stage.stage !== "idle" && game.stage.stage !== "failed") fail("game_running", "The game is already running.");
  log.launches.push({ ...req });
  const status: GameStatus = { stage: { stage: "deploying" }, profile_id: req.profile_id, is_modded: req.method !== "vanilla" };
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
      return summaries();
    case "profile_preview_switch":
      return previewSwitch(String(args?.id));
    case "profile_activate": {
      ensureNotRunning();
      const p = profileDoc(args?.id);
      settings.active_profile_id = p.id;
      emitProfiles();
      return p;
    }
    case "profile_create": {
      const req = args?.req as NewProfile;
      if (!req.name.trim()) fail("invalid_input", "Profile name cannot be empty.");
      const id = req.id ?? freshId(req.name);
      if (profileDocs[id]) fail("conflict", `A profile with ID '${id}' already exists.`);
      const src = req.clone_from ? profileDoc(req.clone_from) : undefined;
      return addProfile(
        newDoc(id, req.name.trim(), {
          ...(src ? structuredClone(src) : {}),
          description: req.description ?? src?.description ?? "",
          linked_save_profile_id: req.linked_save_profile_id ?? src?.linked_save_profile_id ?? "default",
        }),
      );
    }
    case "profile_rename": {
      const p = profileDoc(args?.id);
      if (p.id === settings.active_profile_id) ensureNotRunning();
      p.name = String(args?.name).trim();
      p.updated_at = new Date().toISOString();
      emitProfiles();
      return p;
    }
    case "profile_clone": {
      const src = profileDoc(args?.srcId);
      const name = String(args?.newName).trim();
      if (!name) fail("invalid_input", "Profile name cannot be empty.");
      return addProfile(newDoc(freshId(name), name, structuredClone(src)));
    }
    case "profile_update":
      return updateProfile(String(args?.id), args?.patch as ProfileSettingsPatch);
    case "profile_delete": {
      const p = profileDoc(args?.id);
      if (p.id === settings.active_profile_id) fail("conflict", "Switch to another profile before deleting this one.");
      delete profileDocs[p.id];
      emitProfiles();
      return null;
    }
    case "profile_export_code": {
      const p = profileDoc(args?.id);
      return `QM1-${btoa(JSON.stringify({ name: p.name, enabled_mods: p.enabled_mods }))}`;
    }
    case "profile_import_code":
      return importCode(String(args?.code), String(args?.newId));
    case "mods_list": {
      const enabled = new Set(activeProfile().enabled_mods);
      return ready(mods.map((m) => ({ ...m, enabled: enabled.has(m.id) })));
    }
    case "mod_detail": {
      const id = String(args?.id);
      return detailFor(id, activeProfile().enabled_mods.includes(id)) ?? fail("not_found", `No mod ${id}.`);
    }
    case "mod_config_set": {
      ensureNotRunning();
      const id = String(args?.id);
      savedConfig[id] = { ...(savedConfig[id] ?? {}), ...(args?.values as Record<string, ConfigValue>) };
      return savedConfig[id];
    }
    case "mod_config_reset":
      ensureNotRunning();
      return resetConfig(String(args?.id));
    case "patch_set_enabled":
      return setPatchEnabled(String(args?.patchId), Boolean(args?.enabled));
    case "conflict_resolve":
      return resolveConflict(
        args?.suggestion as ConflictResolutionSuggestion,
        (args?.winnerIndex as number | null | undefined) ?? null,
      );
    case "plugin:opener|open_url":
      return null;
    case "mod_set_enabled":
      return setEnabled(String(args?.id), Boolean(args?.enabled));
    case "load_order_set":
      return setLoadOrder(args?.update as LoadOrderUpdate);
    case "profile_get": {
      const id = String(args?.id);
      return profileDocs[id] ?? fail("not_found", `No profile ${id}.`);
    }
    case "load_order_get":
      return ready(loadOrderFor(activeProfile()));
    case "simulation_get":
      return ready(simulationFor(activeProfile()));
    case "updates_get":
      return ready(updatesReport);
    case "mod_patches_get":
      return ready(patchesFor(String(args?.modId), activeProfile()));
    case "saves_list":
      return saves;
    case "game_status":
      return game;
    case "game_launch":
      return launch(args?.req as LaunchRequest);
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
  // Round-trip through JSON like real IPC, so callers never share fixture objects.
  mockIPC((cmd, args) => {
    const result = handle(cmd, args as Record<string, unknown> | undefined);
    return result === undefined ? null : JSON.parse(JSON.stringify(result));
  });
  window.__qmMockEmit = emitMock;
  window.__qmMockLog = log;
}
