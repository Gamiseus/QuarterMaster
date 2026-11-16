import type { LeftoverFields, SaveBackupEntry, SaveCleanup, SaveComparison, SaveSlotSummary, SaveSummary } from "../../bindings";
import { emitMock } from "../../events";
import { settings } from "../fixtures";
import { saveSummaries, saves, saveSummary } from "../save-data";
import { activeProfile, ensureNotRunning, fail, isGameBusy, log, state, type Handlers } from "../state";

const EMPTY = saveSummary({ file_name: "saveMNB3.lua (New / Empty Slot)", area_of_operations: "No Campaign Started" });

function slot(id: string): SaveSlotSummary {
  return saves.find((s) => s.profile.id === id) ?? fail("not_found", `Save slot '${id}' does not exist.`);
}

function changed(...scopes: ("saves" | "profiles")[]): void {
  setTimeout(() => scopes.forEach((scope) => emitMock({ type: "invalidated", scope })), 0);
}

/** Like the service: only the active slot is off limits while the game runs. */
function ensureNotRunningFor(id: string): void {
  if (id === settings.active_save_profile_id && isGameBusy()) {
    fail("game_running", "The game is running. Close it to change the active save slot.");
  }
}

/** Scenario `saveLeftover`: what the Ironman save's stamp lists and the save still has. */
const LEFTOVER_SLOT = "ironman";
const stamped: Record<string, string[]> = { perkPoints: ["old_perks"], trenchKit: ["trench_kit"], sharedStash: ["old_perks", "trench_kit"] };
const cleaned = new Set<string>();

/** Like the service: the leftovers of a slot, null when there are none. */
export function leftoverOf(id: string): LeftoverFields | null {
  if (!state.init.saveLeftover || id !== LEFTOVER_SLOT) return null;
  const fields = Object.keys(stamped).filter((f) => !cleaned.has(f)).sort();
  if (fields.length === 0) return null;
  return {
    mods: [...new Set(fields.flatMap((f) => stamped[f]))].sort(),
    fields,
    cleanable: fields.filter((f) => stamped[f].length === 1),
    owners: Object.fromEntries(fields.map((f) => [f, stamped[f]])),
  };
}

function cleanup(id: string, fields: string[]): SaveCleanup {
  slot(id);
  ensureNotRunningFor(id);
  const allowed = leftoverOf(id)?.cleanable ?? [];
  const bad = fields.find((f) => !allowed.includes(f));
  if (fields.length === 0 || bad !== undefined) {
    fail("invalid_input", "The save was not cleaned up. It is unchanged.");
  }
  const made = backup(id, "Before clean up");
  fields.forEach((f) => cleaned.add(f));
  changed("saves");
  return { removed: fields, backup_id: made.id };
}

/** Test-only: forget what was cleaned. */
export function resetLeftovers(): void {
  cleaned.clear();
}

function list(): SaveSlotSummary[] {
  return saves.map((s) => ({ ...s, is_active: s.profile.id === settings.active_save_profile_id, leftover: leftoverOf(s.profile.id) }));
}

const VANILLA = "vanilla";

function nameTaken(name: string, except?: string): boolean {
  return saves.some((s) => s.profile.id !== except && s.profile.name.toLowerCase() === name.trim().toLowerCase());
}

function rename(id: string, name: string) {
  const s = slot(id);
  if (id === VANILLA) fail("invalid_input", "The Vanilla save slot cannot be renamed.");
  if (!name.trim()) fail("invalid_input", "A save slot needs a name.");
  if (nameTaken(name, id)) fail("invalid_input", `A save slot named '${name.trim()}' already exists.`);
  s.profile.name = name.trim();
  changed("saves", "profiles");
  return s.profile;
}

function create(name: string) {
  if (!name.trim()) fail("invalid_input", "A save slot needs a name.");
  if (nameTaken(name)) fail("invalid_input", `A save slot named '${name.trim()}' already exists.`);
  const base = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "slot";
  let id = base === VANILLA ? "slot" : base;
  for (let n = 2; saves.some((s) => s.profile.id === id); n++) id = `${base}-${n}`;
  const profile = {
    id,
    name: name.trim(),
    created_at: new Date().toISOString(),
    last_played_at: null,
    active_mods_fingerprint: {},
    backups: [],
    is_vanilla_clean: false,
    tags: { modded: false, edited: false },
    locked_vanilla: false,
    builtin: false,
  };
  saves.push({ profile, is_active: false, tamper_note: null, leftover: null });
  changed("saves");
  return profile;
}

function remove(id: string): null {
  slot(id);
  if (id === VANILLA) fail("invalid_input", "The Vanilla save slot cannot be deleted. Reset it instead.");
  if (id === settings.active_save_profile_id) fail("conflict", "Switch to another slot before deleting this one.");
  saves.splice(saves.findIndex((s) => s.profile.id === id), 1);
  delete saveSummaries[id];
  changed("saves", "profiles");
  return null;
}

function reset(id: string): null {
  ensureNotRunning();
  slot(id);
  backup(id, "Before reset");
  delete saveSummaries[id];
  changed("saves");
  return null;
}

function lock(id: string, locked: boolean) {
  const s = slot(id);
  if (id === VANILLA && !locked) fail("invalid_input", "The Vanilla save slot is always locked.");
  s.profile.locked_vanilla = locked;
  s.profile.is_vanilla_clean = locked;
  changed("saves");
  return s.profile;
}

function detail(id: string): SaveSummary {
  slot(id);
  return saveSummaries[id] ?? EMPTY;
}

function compare(a: string, b: string): SaveComparison {
  const x = detail(a);
  const y = detail(b);
  const names = (s: SaveSummary) => new Set(s.deployed_squad.map((m) => m.name));
  const [inX, inY] = [names(x), names(y)];
  const onlyY = [...inY].filter((n) => !inX.has(n)).sort();
  const onlyX = [...inX].filter((n) => !inY.has(n)).sort();
  const promoted = y.deployed_squad
    .filter((m) => {
      const old = x.deployed_squad.find((o) => o.name === m.name);
      return old !== undefined && (m.rank > old.rank || m.level > old.level);
    })
    .map((m) => {
      const old = x.deployed_squad.find((o) => o.name === m.name)!;
      return `${m.name} (Rank ${old.rank} -> ${m.rank}, Kills ${old.kills} -> ${m.kills})`;
    });
  return {
    ao_a: x.area_of_operations, ao_b: y.area_of_operations,
    day_a: x.day, day_b: y.day, day_delta: y.day - x.day,
    tp_a: x.tactical_points, tp_b: y.tactical_points, tp_delta: y.tactical_points - x.tactical_points,
    rank_a: x.campaign_rank, rank_b: y.campaign_rank, rank_delta: y.campaign_rank - x.campaign_rank,
    experience_a: x.campaign_experience, experience_b: y.campaign_experience,
    experience_delta: y.campaign_experience - x.campaign_experience,
    campaign_kills_a: x.campaign_kills, campaign_kills_b: y.campaign_kills,
    campaign_kills_delta: y.campaign_kills - x.campaign_kills,
    campaign_casualties_a: x.campaign_casualties, campaign_casualties_b: y.campaign_casualties,
    campaign_casualties_delta: y.campaign_casualties - x.campaign_casualties,
    classic_max_wave_a: x.classic_max_wave, classic_max_wave_b: y.classic_max_wave,
    classic_max_wave_delta: y.classic_max_wave - x.classic_max_wave,
    classic_kills_delta: y.classic_total_kills - x.classic_total_kills,
    soldiers_only_in_a: onlyX, soldiers_only_in_b: onlyY, added_soldiers: onlyY, lost_soldiers: onlyX,
    promoted_soldiers: promoted, added_mods: [], removed_mods: [],
  };
}

const MSG_SWITCH_LOCKED = "This save slot is locked as vanilla, and the active profile loads mods. Pick another slot, or unlock this one.";

function activate(id: string): null {
  ensureNotRunning();
  const target = slot(id);
  if (target.profile.locked_vanilla && activeProfile().enabled_mods.length > 0) fail("conflict", MSG_SWITCH_LOCKED);
  settings.active_save_profile_id = id;
  activeProfile().linked_save_profile_id = id;
  changed("saves", "profiles");
  return null;
}

function clone(src: string, newId: string, newName: string) {
  const from = slot(src);
  ensureNotRunningFor(src);
  if (saves.some((s) => s.profile.id === newId)) fail("conflict", `A save slot named '${newId}' already exists.`);
  const profile = {
    ...structuredClone(from.profile),
    id: newId,
    name: newName,
    created_at: new Date().toISOString(),
    last_played_at: null,
    backups: [],
    // Like the service: a copy keeps the history (tags) but never the lock or the built-in flag.
    is_vanilla_clean: false,
    locked_vanilla: false,
    builtin: false,
  };
  saves.push({ profile, is_active: false, tamper_note: null, leftover: null });
  if (saveSummaries[src]) saveSummaries[newId] = structuredClone(saveSummaries[src]);
  changed("saves");
  return profile;
}

let backupCount = 0;

export function backup(id: string, label: string): SaveBackupEntry {
  const s = slot(id);
  ensureNotRunningFor(id);
  backupCount++;
  const backupId = `mock-${backupCount}`;
  const entry: SaveBackupEntry = {
    id: backupId,
    timestamp: `${new Date().toISOString().replace("Z", "+00:00")}[UTC]`,
    label: label.trim() || "Manual backup",
    path: `C:\\QuarterMaster\\saves\\${id}\\backups\\${backupId}`,
    size_bytes: 45_056,
  };
  s.profile.backups = [entry, ...(s.profile.backups ?? [])];
  changed("saves");
  return entry;
}

function findBackup(id: string, backupId: string): SaveBackupEntry {
  return slot(id).profile.backups?.find((b) => b.id === backupId) ?? fail("not_found", "That backup no longer exists.");
}

export const handlers: Handlers = {
  saves_list: () => list(),
  save_detail: (args) => detail(String(args.id)),
  saves_compare: (args) => compare(String(args.a), String(args.b)),
  save_activate: (args) => activate(String(args.id)),
  save_clone: (args) => clone(String(args.srcId), String(args.newId), String(args.newName)),
  save_backup: (args) => backup(String(args.id), String(args.label)),
  save_cleanup: (args) => cleanup(String(args.id), (args.fields as string[]) ?? []),
  save_restore: (args) => {
    ensureNotRunning();
    findBackup(String(args.id), String(args.backupId));
    changed("saves");
    return null;
  },
  // A striped bar stands in for the game's ribbon and award images.
  game_sprite_get: (args) => {
    const hue = (Number(args.id) * 67) % 360;
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="96" height="32"><rect width="96" height="32" fill="hsl(${hue} 45% 40%)"/><rect x="40" width="16" height="32" fill="hsl(${(hue + 180) % 360} 45% 55%)"/></svg>`;
    return `data:image/svg+xml;base64,${btoa(svg)}`;
  },
  game_ribbons_get: () => [
    { id: 1, name: "Combat", description: "Kill 100 enemies, +1 damage to all attacks" },
    { id: 2, name: "Veteran", description: "Kill 1000 enemies, up to +10 dmg to all attacks" },
  ],
  save_dossier_export: (args) => {
    log.dossierExports.push({ slotName: String(args.slotName), text: String(args.text) });
    return `${String(args.slotName)} dossier.txt`;
  },
  save_rename: (args) => rename(String(args.id), String(args.name)),
  save_create: (args) => create(String(args.name)),
  save_delete: (args) => remove(String(args.id)),
  save_reset: (args) => reset(String(args.id)),
  save_lock_set: (args) => lock(String(args.id), Boolean(args.locked)),
  save_delete_backup: (args) => {
    const s = slot(String(args.id));
    findBackup(String(args.id), String(args.backupId));
    s.profile.backups = (s.profile.backups ?? []).filter((b) => b.id !== args.backupId);
    changed("saves");
    return null;
  },
};
