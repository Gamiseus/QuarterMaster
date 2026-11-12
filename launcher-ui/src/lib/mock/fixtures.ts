import type { AppInfo, AssetFile, DetectedConflict, LoadOrderResolution, Loadable, ModPatchStatus, ModProfile, ModSummary, ProfileSummary, SettingsView, SimulationOverview, UpdatesReport } from "../bindings";
import { chainsFor, compatLayer, disabledIn, engineConflicts, fileStatsFor, patchState } from "./patchworld";

export const ready = <T>(data: T): Loadable<T> => ({ status: "ready", data });

export const settings: SettingsView = {
  game_exe_path: "C:\\Games\\Mud and Blood\\mnb.exe",
  save_dir_path: null,
  workspace_mods_dir: "C:\\Dev\\MnB-Mods",
  theme: "system",
  default_launch_method: "steam_url",
  opt_in_prereleases: false,
  auto_backup_saves: true,
  max_save_backups: 10,
  disable_console_on_launch: false,
  debug_mode: false,
  dev_commands: false,
  first_run_completed: true,
  active_profile_id: "default",
  active_save_profile_id: "default",
  portable_mode: false,
  auto_update_mods: {},
  last_known_exe_sha256: null,
  auto_check_updates: true,
  auto_install_updates: false,
  color_vision: "standard",
  high_contrast: false,
  text_scale: 1,
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
    kind: "managed",
    faction: null,
  };
}

/** An asset-only mod (no qm.toml), listed when `__qmMockInit.assetMod` is set. */
export const soundSwap: ModSummary = {
  ...mod("soundswap", "SoundSwap", "1.0.0", true, 70, 1, "Other"),
  authors: ["Unknown Author"],
  kind: "assets",
};

export const soundSwapFiles: AssetFile[] = [
  { path: "assets/music/MudAndBloodFull.mp3", overrides_vanilla: true },
  { path: "assets/sounds/garand1.wav", overrides_vanilla: true },
  { path: "assets/sounds/garand2.wav", overrides_vanilla: true },
  { path: "assets/sounds/new_shot.wav", overrides_vanilla: false },
  { path: "assets/sprites/weapons/top_garand.png", overrides_vanilla: true },
];

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
  vanilla: {
    id: "vanilla",
    name: "Vanilla",
    description: "The game as released, with no mods. Built in.",
    created_at: "2026-10-01T12:00:00Z",
    updated_at: "2026-10-01T12:00:00Z",
    enabled_mods: [],
    separators: [],
    mod_priorities: {},
    linked_save_profile_id: "vanilla",
  },
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
    // Made after stacking shipped, so it starts on dependency order (D40).
    stacking: "dependencies",
    notices_seen: ["stacking_v1"],
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
  is_builtin: p.id === "vanilla",
}));

export { saves } from "./save-data";

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

/** The simulation checks of plan 2e Phase D §6.1, one of each kind, for tests (not reported by default). */
export const diagnosticConflicts: DetectedConflict[] = [
  {
    id: "shadowed:includes/ser.lua", severity: "warning", kind: "shadowed_file", target_file: "includes/ser.lua",
    mod_a_id: "gcore", mod_b_id: null, patch_a_id: "gcore:main_lua:1", patch_b_id: null,
    description: "'includes/ser.lua' won't run: 'gcore' replaces this file with its module 'includes/ser'. Patches on this file do not run in the game.",
    suggestions: [],
  },
  {
    id: "target-several:ser.lua", severity: "warning", kind: "target_matches_several", target_file: "ser.lua",
    mod_a_id: "ai-tweaks", mod_b_id: null, patch_a_id: "ai-ser", patch_b_id: null,
    description: "The target 'ser.lua' of 'ai-ser' (from 'ai-tweaks') names 2 game files: includes/binser.lua, includes/ser.lua. The game patches each of them.",
    suggestions: [],
  },
  {
    id: "target-boundary:ser.lua:includes/binser.lua", severity: "warning", kind: "target_without_boundary", target_file: "includes/binser.lua",
    mod_a_id: "ai-tweaks", mod_b_id: null, patch_a_id: "ai-ser", patch_b_id: null,
    description: "The target 'ser.lua' of 'ai-ser' (from 'ai-tweaks') also names 'includes/binser.lua', because the game matches the end of the file path. Write the full path to patch only the file you mean.",
    suggestions: [],
  },
  {
    id: "abort:main.lua:menu-entry", severity: "error", kind: "patch_aborts_file", target_file: "main.lua",
    mod_a_id: "gmenu", mod_b_id: null, patch_a_id: "menu-entry", patch_b_id: null,
    description: "Patch 'Menu entry' (from 'gmenu') does not match in 'main.lua' and has on_failure = \"abort\", so the game fails to load 'main.lua' and stops with a Lua error.",
    suggestions: [],
  },
  {
    id: "sites-gained:scripts/ai.lua:ai-overlay", severity: "warning", kind: "sites_gained", target_file: "scripts/ai.lua",
    mod_a_id: "ai-tweaks", mod_b_id: "gcore", patch_a_id: "ai-overlay", patch_b_id: "core-targets",
    description: "Patch 'ai-overlay' (from 'ai-tweaks') matches 2 places in 'scripts/ai.lua', but the game's file has 1: code from 'core-targets' (from 'gcore') has a copy of its lines, so it also runs there.",
    suggestions: [],
  },
  {
    id: "multi-match:scripts/ai.lua:core-targets", severity: "info", kind: "multi_match", target_file: "scripts/ai.lua",
    mod_a_id: "gcore", mod_b_id: null, patch_a_id: "core-targets", patch_b_id: null,
    description: "Patch 'Target picker' (from 'gcore') has no `times` and matches 3 places in 2 functions of 'scripts/ai.lua'. It runs at each of them.",
    suggestions: [],
  },
  {
    id: "var-collision:SPEED", severity: "warning", kind: "var_collision", target_file: "",
    mod_a_id: "ai-tweaks", mod_b_id: "gcore", patch_a_id: "vars:SPEED", patch_b_id: null,
    description: "Mods set the variable 'SPEED' to different values: '1' (ai-tweaks), '2' (gcore). The game uses '2' from 'gcore' in every patch.",
    suggestions: [],
  },
];

/** The semantic checks of plan 2e Phase D 6.9 (Tasks 37a to 37c), one of each, for tests (not reported by default). */
export const semanticConflicts: DetectedConflict[] = [
  {
    id: "semantic:capture_before_define:main.lua:ai-tweaks:copy:gcore:settings@loadModSettings", severity: "warning", kind: "capture_before_define",
    target_file: "main.lua", mod_a_id: "ai-tweaks", mod_b_id: "gcore", patch_a_id: "ai-tweaks:copy", patch_b_id: "gcore:settings",
    description: "Patch 'copy' (from 'ai-tweaks') copies `loadModSettings` into `_orig` at main.lua line 12, which runs before patch 'settings' (from 'gcore') defines `loadModSettings` at main.lua line 40. The copy keeps the old value (or nil), so code that calls `_orig` does not run 'gcore''s version.",
    suggestions: [],
  },
  {
    id: "semantic:redefined_global:main.lua:ai-tweaks:settings:gcore:settings@saveModSettings", severity: "warning", kind: "redefined_global",
    target_file: "main.lua", mod_a_id: "ai-tweaks", mod_b_id: "gcore", patch_a_id: "ai-tweaks:settings", patch_b_id: "gcore:settings",
    description: "Patch 'settings' (from 'ai-tweaks') defines `saveModSettings` at main.lua line 20, and patch 'settings' (from 'gcore') defines it again at main.lua line 52. 'gcore''s definition runs later and replaces the other, so 'ai-tweaks''s `saveModSettings` never runs.",
    suggestions: [],
  },
  {
    id: "semantic:dead_code:main.lua:gcore:f5:gconsole:f5@love.keypressed", severity: "warning", kind: "dead_code",
    target_file: "main.lua", mod_a_id: "gcore", mod_b_id: "gconsole", patch_a_id: "gcore:f5", patch_b_id: "gconsole:f5",
    description: "Patch 'f5' (from 'gcore') adds code at main.lua line 9400, inside a branch of `if (1 == 0) then` at line 9357 (from patch 'F5-F8 dev controls' of 'gconsole') that never runs. That code never runs.",
    suggestions: [],
  },
  {
    id: "semantic:dead_code:main.lua:gconsole:f5:-@love.keypressed", severity: "info", kind: "dead_code",
    target_file: "main.lua", mod_a_id: "gconsole", mod_b_id: null, patch_a_id: "gconsole:f5", patch_b_id: null,
    description: "Patch 'F5-F8 dev controls' (from 'gconsole') adds `if (1 == 0) then` at main.lua line 9357, which puts 3 lines of the game's code (lines 9570 to 9574) in a branch that never runs.",
    suggestions: [],
  },
  {
    id: "semantic:early_return_shadow:main.lua:gcore:keys:gdumper:keys@love.keypressed", severity: "warning", kind: "early_return_shadow",
    target_file: "main.lua", mod_a_id: "gcore", mod_b_id: "gdumper", patch_a_id: "gcore:keys", patch_b_id: "gdumper:keys",
    description: "Patch 'keys' (from 'gdumper') returns from `love.keypressed` at main.lua line 2. Patch 'keys' (from 'gcore') adds code after it at line 4, so that code never runs.",
    suggestions: [],
  },
  {
    id: "semantic:table_literal_reset:main.lua:ai-tweaks:fields:gcore:settings@modSettings", severity: "warning", kind: "table_literal_reset",
    target_file: "main.lua", mod_a_id: "ai-tweaks", mod_b_id: "gcore", patch_a_id: "ai-tweaks:fields", patch_b_id: "gcore:settings",
    description: "Patch 'fields' (from 'ai-tweaks') sets `modSettings.squadLimit` at main.lua line 30. Patch 'settings' (from 'gcore') sets `modSettings = {...}` at main.lua line 52, which runs later and makes a new table, so what 'ai-tweaks' put in `modSettings` is gone.",
    suggestions: [],
  },
  {
    id: "semantic:shared_file_writer:main.lua:ai-tweaks:save:gcore:settings@mod_settings.lua", severity: "warning", kind: "shared_file_writer",
    target_file: "main.lua", mod_a_id: "ai-tweaks", mod_b_id: "gcore", patch_a_id: "ai-tweaks:save", patch_b_id: "gcore:settings",
    description: "Patch 'save' (from 'ai-tweaks') writes `mod_settings.lua` at main.lua line 87, and patch 'settings' (from 'gcore') writes it too at main.lua line 147. 'gcore' writes the whole file without reading it first, so it drops what 'ai-tweaks' saved there.",
    suggestions: [],
  },
  {
    id: "semantic:shared_file_writer:main.lua:gcore:settings:ai-tweaks:save@mod_settings.lua", severity: "info", kind: "shared_file_writer",
    target_file: "main.lua", mod_a_id: "gcore", mod_b_id: "ai-tweaks", patch_a_id: "gcore:settings", patch_b_id: "ai-tweaks:save",
    description: "Patch 'settings' (from 'gcore') writes `mod_settings.lua` at main.lua line 147, and patch 'save' (from 'ai-tweaks') writes it too at main.lua line 87. Both read the file first and keep what is in it.",
    suggestions: [],
  },
  {
    id: "semantic:key_collision:main.lua:gcore:bench:gconsole:f9@ctrl+f9", severity: "warning", kind: "key_collision",
    target_file: "main.lua", mod_a_id: "gcore", mod_b_id: "gconsole", patch_a_id: "gcore:bench", patch_b_id: "gconsole:f9",
    description: "Patch 'bench' (from 'gcore') acts on Ctrl+F9 at main.lua line 9400, and patch 'f9' (from 'gconsole') acts on Ctrl+F9 too at main.lua line 9350. Both run when the player presses Ctrl+F9, unless one of them stops the other first.",
    suggestions: [],
  },
  {
    id: "semantic:double_hook:main.lua:ai-tweaks:settings:gcore:settings@loadModSettings", severity: "warning", kind: "double_hook",
    target_file: "main.lua", mod_a_id: "ai-tweaks", mod_b_id: "gcore", patch_a_id: "ai-tweaks:settings", patch_b_id: "gcore:settings",
    description: "Patch 'settings' (from 'ai-tweaks') wraps `loadModSettings` at main.lua line 20. Patch 'settings' (from 'gcore') wraps it again later at main.lua line 52, but its new `loadModSettings` never calls the copy it took, so it does not call 'ai-tweaks''s. 'ai-tweaks''s wrapper never runs.",
    suggestions: [],
  },
  {
    id: "semantic:bypassed_hook:main.lua:ai-tweaks:threat:gcore:hostility@calcDamage", severity: "warning", kind: "bypassed_hook",
    target_file: "main.lua", mod_a_id: "ai-tweaks", mod_b_id: "gcore", patch_a_id: "ai-tweaks:threat", patch_b_id: "gcore:hostility",
    description: "Patch 'hostility' (from 'gcore') rewrites lines of main.lua that call `calcDamage` (now main.lua line 5), and no line of the game's own code calls it any more. Patch 'threat' (from 'ai-tweaks') calls `calcDamage` itself at main.lua line 13, so 'gcore''s change does not reach that code. This compares the text only.",
    suggestions: [],
  },
  {
    id: "semantic:shared_field:includes/pathfinding:gcore:pathfinding:ai-tweaks:halt@pathQueue", severity: "info", kind: "shared_field",
    target_file: "includes/pathfinding", mod_a_id: "gcore", mod_b_id: "ai-tweaks", patch_a_id: "gcore:pathfinding", patch_b_id: "ai-tweaks:halt",
    description: "Patch 'includes/pathfinding' (from 'gcore') sets `u.pathQueue` at includes/pathfinding line 576, and patch 'halt' (from 'ai-tweaks') sets `dude.pathQueue` at includes/ai.lua line 337. The game's own code never sets `pathQueue`, so both mods keep their own data in one field; check that they mean the same by it.",
    suggestions: [],
  },
  {
    id: "semantic:name_collision:main.lua:gmenu:page:ai-tweaks:page@menuPage=85", severity: "warning", kind: "name_collision",
    target_file: "main.lua", mod_a_id: "gmenu", mod_b_id: "ai-tweaks", patch_a_id: "gmenu:page", patch_b_id: "ai-tweaks:page",
    description: "Patch 'page' (from 'gmenu') uses menu page 85 at main.lua line 67, and patch 'page' (from 'ai-tweaks') uses it too at main.lua line 90. The game has no such page, so both mods draw and handle their own screen on one page number.",
    suggestions: [],
  },
  {
    id: "semantic:block_balance:main.lua:ai-tweaks:open:ai-tweaks:close@main.lua", severity: "warning", kind: "block_balance",
    target_file: "main.lua", mod_a_id: "ai-tweaks", mod_b_id: "ai-tweaks", patch_a_id: "ai-tweaks:open", patch_b_id: "ai-tweaks:close",
    description: "Patch 'open' (from 'ai-tweaks') opens a block in main.lua that patch 'close' of the same mod closes, but 'close' is turned off. The file is left with 1 block never closed, so it does not compile.",
    suggestions: [],
  },
  {
    id: "semantic:moved_scope:main.lua:ai-tweaks:move:-@love.draw", severity: "warning", kind: "moved_scope",
    target_file: "main.lua", mod_a_id: "ai-tweaks", mod_b_id: null, patch_a_id: "ai-tweaks:move", patch_b_id: null,
    description: "With this change, patch 'move' (from 'ai-tweaks') adds its code in `love.draw` at main.lua line 9; before, it was in `love.update`. Code in another function runs at another time and sees other locals, so check that it still does what it should.",
    suggestions: [],
  },
  {
    id: "semantic:free_name_changed:main.lua:ai-tweaks:move:-@dt", severity: "warning", kind: "free_name_changed",
    target_file: "main.lua", mod_a_id: "ai-tweaks", mod_b_id: null, patch_a_id: "ai-tweaks:move", patch_b_id: null,
    description: "With this change, `dt` in patch 'move' (from 'ai-tweaks') at main.lua line 9 means the global `dt`; before, it meant the local `dt` of `love.update` (`function love.update(dt)`). The code reads or sets another variable than its author wrote it for. Scope is read from the text, so this can be wrong for unusual code.",
    suggestions: [],
  },
];

/** Every conflict the mock can report; one is live only while both of its mods are enabled. */
const candidateConflicts: DetectedConflict[] = [conflict, conflictMedics];

/** Base per-mod patches (before enabled state and conflicts apply); GMenu and AI Tweaks each have one broken patch (GMenu without a conflict). */
export const patchesByMod: Record<string, ModPatchStatus[]> = {
  gcore: [
    { patch_id: "core-init", name: "Core init", target_file: "main.lua", state: "applied", conflicts: [] },
    { patch_id: "core-targets", name: "Target picker", target_file: "scripts/ai.lua", state: "applied", conflicts: [] },
  ],
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
    { patch_id: "hud-compass", name: "Compass", target_file: "scripts/ui/hud.lua", state: "applied", conflicts: [] },
  ],
  "vehicle-pack": [
    {
      patch_id: "vehicle-spawn",
      name: "Vehicle spawn",
      target_file: "scripts/vehicles.lua",
      state: "applied",
      conflicts: [],
    },
    { patch_id: "vehicle-init", name: "Vehicle init", target_file: "main.lua", state: "applied", conflicts: [] },
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
    { patch_id: "ai-init", name: "AI init", target_file: "main.lua", state: "applied", conflicts: [] },
  ],
};

/** Patch rows for one mod as the profile sees them: state follows enabled mods and disabled patches. */
export function patchesFor(modId: string, profile: ModProfile): ModPatchStatus[] {
  const live = liveConflicts(profile);
  const modOn = profile.enabled_mods.includes(modId);
  const off = disabledIn(profile);
  const changed = compatLayer(profile).changedBy;
  // The mock patch engine decides applied, broken or disabled; adjusted copies stand in for their base.
  return (patchesByMod[modId] ?? []).map((p) => ({
    ...p,
    state: !modOn || off.has(p.patch_id) ? "disabled" : (patchState(p.patch_id, profile) ?? p.state),
    conflicts: live.filter((c) => c.patch_a_id === p.patch_id || c.patch_b_id === p.patch_id),
    ...(changed.has(p.patch_id) ? { changed_by: changed.get(p.patch_id) } : {}),
  }));
}

/** Conflicts whose two mods are enabled and whose two patches are both active. */
export function liveConflicts(profile: ModProfile): DetectedConflict[] {
  const on = new Set(profile.enabled_mods);
  const off = disabledIn(profile);
  return [
    ...candidateConflicts.filter(
      (c) =>
        on.has(c.mod_a_id) &&
        c.mod_b_id !== null &&
        on.has(c.mod_b_id) &&
        !off.has(c.patch_a_id) &&
        !off.has(c.patch_b_id ?? ""),
    ),
    ...engineConflicts(profile),
  ];
}

export function simulationFor(profile: ModProfile): SimulationOverview {
  const overrides = compatLayer(profile).reports;
  const patches = profile.enabled_mods.flatMap((id) => patchesFor(id, profile));
  const { failure_groups, inferred_edges } = chainsFor(profile);
  return {
    total_patches: patches.length,
    applied_patches: patches.filter((p) => p.state === "applied").length,
    broken_anchors: patches.filter((p) => p.state === "broken").length,
    conflicts: liveConflicts(profile),
    files: [...new Set(patches.map((p) => p.target_file))],
    file_stats: fileStatsFor(profile),
    compile_check: { ran: true, files_checked: new Set(patches.map((p) => p.target_file)).size, note: null },
    plan_error: null,
    plan_warnings: [],
    load_errors: [],
    fix_bindings: [],
    ...(overrides.length > 0 ? { overrides } : {}),
    ...(failure_groups.length > 0 ? { failure_groups } : {}),
    ...(inferred_edges.length > 0 ? { inferred_edges } : {}),
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
  rate_limit_mode: "normal",
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
      download_url: "https://github.com/Gamiseus/MnB-Mods/releases/download/v1.3.0/GMenu.zip",
    },
  ],
};
