import type {
  AppInfo,
  DetectedConflict,
  Loadable,
  LoadOrderResolution,
  ModPatchStatus,
  ModProfile,
  ModSummary,
  ProfileSummary,
  SaveSlotSummary,
  SettingsView,
  SimulationOverview,
  UpdatesReport,
} from "../bindings";

export const ready = <T>(data: T): Loadable<T> => ({ status: "ready", data });

export const settings: SettingsView = {
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

export const appInfo: AppInfo = {
  version: "1.0.0-mock",
  data_dir: "C:\\Users\\Player\\AppData\\Roaming\\QuarterMaster",
  is_portable: false,
  game: ready({ version: "3.0.0", exe_sha256: "0".repeat(64), lua_file_count: 412 }),
};

function mod(
  id: string,
  name: string,
  version: string,
  enabled: boolean,
  load_priority: number,
  patch_count: number,
  category: string | null = null,
): ModSummary {
  return {
    id,
    name,
    version,
    authors: ["Gamiseus"],
    category,
    enabled,
    patch_count,
    installed_versions: [version],
    is_workspace_linked: false,
    has_config: false,
    load_priority,
  };
}

export const mods: ModSummary[] = [
  mod("gcore", "GCore", "1.4.0", true, 10, 1, "Framework"),
  { ...mod("gmenu", "GMenu", "1.2.1", true, 20, 2, "Framework"), has_config: true },
  mod("better-medics", "Better Medics", "0.3.0", false, 30, 1, "Gameplay"),
  mod("tactical-hud", "Tactical HUD", "2.0.1", true, 40, 1, "Interface"),
  mod("vehicle-pack", "Vehicle Pack", "1.1.0", true, 50, 1, "Content"),
  mod("ai-tweaks", "AI Tweaks", "0.9.2", true, 60, 2, "Gameplay"),
];

const now = "2026-10-30T12:00:00Z";

/** Full profile documents keyed by id. */
export const profileDocs: Record<string, ModProfile> = {
  default: {
    id: "default",
    name: "Default",
    description: "",
    created_at: "2026-10-01T12:00:00Z",
    updated_at: now,
    enabled_mods: ["gcore", "gmenu", "tactical-hud", "vehicle-pack", "ai-tweaks"],
    separators: [
      { id: "sep-framework", title: "Framework", color: "sep-1", insert_before_mod_id: "gcore" },
      { id: "sep-gameplay", title: "Gameplay", color: "sep-3", insert_before_mod_id: "tactical-hud" },
    ],
    mod_priorities: { gcore: 10, gmenu: 20, "tactical-hud": 40, "vehicle-pack": 50, "ai-tweaks": 60 },
    linked_save_profile_id: "default",
    locked_mods: ["gcore"],
  },
  "vanilla-plus": {
    id: "vanilla-plus",
    name: "Vanilla Plus",
    description: "Light additions only.",
    created_at: "2026-10-05T12:00:00Z",
    updated_at: "2026-10-29T12:00:00Z",
    enabled_mods: ["gcore"],
    separators: [],
    mod_priorities: { gcore: 10 },
    linked_save_profile_id: "default",
  },
};

export const profiles: ProfileSummary[] = Object.values(profileDocs).map((p) => ({
  id: p.id,
  name: p.name,
  description: p.description ?? "",
  enabled_count: p.enabled_mods.length,
  is_active: p.id === settings.active_profile_id,
  linked_save_profile_id: p.linked_save_profile_id ?? "default",
  updated_at: p.updated_at,
}));

export const saves: SaveSlotSummary[] = [
  {
    profile: { id: "default", name: "Main Campaign", created_at: "2026-10-01T12:00:00Z", last_played_at: null },
    is_active: true,
  },
  {
    profile: { id: "ironman", name: "Ironman", created_at: "2026-10-12T12:00:00Z", last_played_at: null },
    is_active: false,
  },
];

export const conflict: DetectedConflict = {
  id: "conflict-hud-ai",
  severity: "warning",
  kind: "same_anchor",
  target_file: "scripts/ui/hud.lua",
  mod_a_id: "tactical-hud",
  mod_b_id: "ai-tweaks",
  patch_a_id: "hud-minimap",
  patch_b_id: "ai-overlay",
  description: "Both patches replace the same line in hud.lua.",
  suggestions: [
    {
      tier: "select_winner",
      patch_a_id: "hud-minimap",
      mod_a_id: "tactical-hud",
      patch_b_id: "ai-overlay",
      mod_b_id: "ai-tweaks",
      explanation: "Pick which patch is kept.",
    },
  ],
};

/** Appears only while both Better Medics and Vehicle Pack are enabled. */
export const conflictMedics: DetectedConflict = {
  id: "conflict-medics-vehicle",
  severity: "warning",
  kind: "same_anchor",
  target_file: "scripts/vehicles.lua",
  mod_a_id: "better-medics",
  mod_b_id: "vehicle-pack",
  patch_a_id: "medic-heal",
  patch_b_id: "vehicle-spawn",
  description: "Both patches change the vehicle crew heal rate.",
  suggestions: [
    {
      tier: "select_winner",
      patch_a_id: "medic-heal",
      mod_a_id: "better-medics",
      patch_b_id: "vehicle-spawn",
      mod_b_id: "vehicle-pack",
      explanation: "Pick which patch is kept.",
    },
  ],
};

/** Every conflict the mock can report; one is live only while both of its mods are enabled. */
const candidateConflicts: DetectedConflict[] = [conflict, conflictMedics];

/** Base per-mod patches (before enabled state and conflicts apply); GMenu and AI Tweaks each have one broken patch (GMenu without a conflict). */
export const patchesByMod: Record<string, ModPatchStatus[]> = {
  gcore: [{ patch_id: "core-init", name: "Core init", target_file: "main.lua", state: "applied", conflicts: [] }],
  gmenu: [
    { patch_id: "menu-entry", name: "Menu entry", target_file: "scripts/ui/menu.lua", state: "applied", conflicts: [] },
    { patch_id: "menu-hotkeys", name: "Hotkeys", target_file: "scripts/ui/keys.lua", state: "broken", conflicts: [] },
  ],
  "better-medics": [
    { patch_id: "medic-heal", name: "Heal rate", target_file: "scripts/unit.lua", state: "applied", conflicts: [] },
  ],
  "tactical-hud": [
    {
      patch_id: "hud-minimap",
      name: "Minimap",
      target_file: "scripts/ui/hud.lua",
      state: "applied",
      conflicts: [conflict],
    },
  ],
  "vehicle-pack": [
    {
      patch_id: "vehicle-spawn",
      name: "Vehicle spawn",
      target_file: "scripts/vehicles.lua",
      state: "applied",
      conflicts: [],
    },
  ],
  "ai-tweaks": [
    {
      patch_id: "ai-overlay",
      name: "AI overlay",
      target_file: "scripts/ui/hud.lua",
      state: "applied",
      conflicts: [conflict],
    },
    { patch_id: "ai-targeting", name: "Targeting", target_file: "scripts/ai.lua", state: "broken", conflicts: [] },
  ],
};

/** Patch rows for one mod as the profile sees them: state follows enabled mods and disabled patches. */
export function patchesFor(modId: string, profile: ModProfile): ModPatchStatus[] {
  const live = liveConflicts(profile);
  const modOn = profile.enabled_mods.includes(modId);
  const off = new Set(profile.disabled_patches ?? []);
  return (patchesByMod[modId] ?? []).map((p) => ({
    ...p,
    state: p.state === "broken" ? "broken" : modOn && !off.has(p.patch_id) ? "applied" : "disabled",
    conflicts: live.filter((c) => c.patch_a_id === p.patch_id || c.patch_b_id === p.patch_id),
  }));
}

/** Conflicts whose two mods are enabled and whose two patches are both active. */
export function liveConflicts(profile: ModProfile): DetectedConflict[] {
  const on = new Set(profile.enabled_mods);
  const off = new Set(profile.disabled_patches ?? []);
  return candidateConflicts.filter(
    (c) =>
      on.has(c.mod_a_id) &&
      c.mod_b_id !== null &&
      on.has(c.mod_b_id) &&
      !off.has(c.patch_a_id) &&
      !off.has(c.patch_b_id ?? ""),
  );
}

export function simulationFor(profile: ModProfile): SimulationOverview {
  const patches = profile.enabled_mods.flatMap((id) => patchesFor(id, profile));
  return {
    total_patches: patches.length,
    applied_patches: patches.filter((p) => p.state === "applied").length,
    broken_anchors: patches.filter((p) => p.state === "broken").length,
    conflicts: liveConflicts(profile),
    files: [...new Set(patches.map((p) => p.target_file))],
  };
}

export function loadOrderFor(profile: ModProfile): LoadOrderResolution {
  const priority = (id: string, fallback: number) => profile.mod_priorities?.[id] ?? fallback;
  const enabled = mods
    .filter((m) => profile.enabled_mods.includes(m.id))
    .map((m) => ({ m, p: priority(m.id, m.load_priority) }))
    .sort((a, b) => a.p - b.p || (a.m.id < b.m.id ? -1 : 1));
  return {
    ordered_mods: enabled.map(({ m, p }, i) => ({
      mod_id: m.id,
      name: m.name,
      version: m.version,
      effective_priority: p,
      order_index: i,
      reasons: [`Priority ${p}`],
    })),
    diagnostics: profile.enabled_mods.includes("gcore")
      ? []
      : enabled
          .filter(({ m }) => m.id !== "gcore")
          .map(({ m }) => ({
            severity: "error" as const,
            mod_id: m.id,
            kind: "missing_dependency",
            message: `${m.name} requires GCore, which is not enabled.`,
            related_mod_id: "gcore",
          })),
    has_cycles: false,
    cycle_paths: [],
    rule_hierarchy: "priority",
  };
}

export const updatesReport: UpdatesReport = {
  checked_at: now,
  github_token_configured: false,
  rate_limit_mode: "unauthenticated",
  launcher: {
    current_version: "1.0.0-mock",
    latest_version: "1.0.0-mock",
    update_available: false,
    opt_in_prereleases: false,
    signature_verification: "verified",
    rollback_available: false,
  },
  mods: [
    {
      mod_id: "gmenu",
      name: "GMenu",
      installed_version: "1.2.1",
      latest_version: "1.3.0",
      update_available: true,
      update_key: "GitHub:Gamiseus/MnB-Mods#GMenu.zip",
      pinned_source_url: null,
      source_changed_warning: false,
      auto_update: false,
      changelog: "Adds keyboard navigation to menus.",
    },
  ],
};
