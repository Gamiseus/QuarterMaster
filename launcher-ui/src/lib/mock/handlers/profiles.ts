import type { ModProfile, NewProfile, ProfileSettingsPatch, ProfileSummary, SwitchPreview } from "../../bindings";
import { emitMock } from "../../events";
import { profileDocs, saves, settings } from "../fixtures";
import { runMockJob } from "../jobs";
import { activeProfile, ensureNotRunning, fail, type Handlers } from "../state";

function summaries(): ProfileSummary[] {
  return Object.values(profileDocs).map((p) => ({
    id: p.id,
    name: p.name,
    description: p.description ?? "",
    enabled_count: p.enabled_mods.length,
    is_active: p.id === settings.active_profile_id,
    linked_save_profile_id: p.linked_save_profile_id ?? "default",
    updated_at: p.updated_at,
    is_builtin: p.id === "vanilla",
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
    // New and imported profiles start on dependency order; clones copy the source's (D40).
    stacking: "dependencies",
    notices_seen: ["stacking_v1"],
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
  if (patch.linked_save_profile_id === "vanilla") fail("invalid_input", "The Vanilla save slot belongs to the Vanilla profile.");
  // An empty id means "New slot on first launch".
  if (patch.linked_save_profile_id && !saves.some((s) => s.profile.id === patch.linked_save_profile_id)) {
    fail("not_found", `Save slot '${patch.linked_save_profile_id}' does not exist.`);
  }
  if (patch.launch_method !== null) p.launch_method = patch.launch_method === "default" ? null : patch.launch_method;
  if (patch.custom_launch_args !== null) p.custom_launch_args = patch.custom_launch_args;
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

function createProfile(req: NewProfile): ModProfile {
  if (!req.name.trim()) fail("invalid_input", "Profile name cannot be empty.");
  const id = req.id ?? freshId(req.name);
  if (profileDocs[id]) fail("conflict", `A profile with ID '${id}' already exists.`);
  const src = req.clone_from ? profileDoc(req.clone_from) : undefined;
  return addProfile(
    newDoc(id, req.name.trim(), {
      ...(src ? { ...structuredClone(src), stacking: src.stacking ?? null, notices_seen: [...(src.notices_seen ?? [])] } : {}),
      description: req.description ?? src?.description ?? "",
      linked_save_profile_id: req.linked_save_profile_id ?? src?.linked_save_profile_id ?? "default",
    }),
  );
}

export const handlers: Handlers = {
  profiles_list: () => summaries(),
  profile_preview_switch: (args) => previewSwitch(String(args.id)),
  modpack_export: (args) => {
    const p = profileDoc(args.profileId);
    return runMockJob("modpack_export", `Exporting ${p.name}`, () => ({ output: `${p.name}.qmpack` }));
  },
  modpack_inspect: (args) => {
    if (!String(args.path).toLowerCase().endsWith(".qmpack")) fail("invalid_input", "This is not a QuarterMaster modpack.");
    return {
      name: "Night Ops",
      description: "Night missions with a lighter HUD.",
      mods: [
        { id: "gcore", name: "GCore", version: "1.4.0", installed: true, bundled: true },
        { id: "night-ops", name: "Night Ops", version: "0.4.0", installed: false, bundled: true },
        { id: "fog-of-war", name: "Fog of War", version: "1.0.0", installed: false, bundled: false },
      ],
    };
  },
  modpack_import: (args) => {
    ensureNotRunning();
    const name = String(args.name).trim();
    if (!name) fail("invalid_input", "The profile needs a name.");
    return runMockJob("modpack_import", `Importing ${name}`, () => {
      const p = createProfile({ name, id: null, clone_from: null, linked_save_profile_id: null, description: null });
      p.enabled_mods = ["gcore", "night-ops", "fog-of-war"];
      p.linked_save_profile_id = args.slot === null || args.slot === undefined ? "" : String(args.slot);
      emitProfiles();
      return { output: p.id };
    });
  },
  profile_diff: (args) => {
    const a = profileDoc(args.a);
    const b = profileDoc(args.b);
    const prio = (p: ModProfile, id: string) => p.mod_priorities?.[id] ?? 50;
    return {
      profile_a_id: a.id,
      profile_b_id: b.id,
      only_in_a: a.enabled_mods.filter((m) => !b.enabled_mods.includes(m)),
      only_in_b: b.enabled_mods.filter((m) => !a.enabled_mods.includes(m)),
      priority_diffs: a.enabled_mods
        .filter((m) => b.enabled_mods.includes(m) && prio(a, m) !== prio(b, m))
        .map((m) => [m, prio(a, m), prio(b, m)]),
      setting_diffs: [],
    };
  },
  profile_activate: (args) => {
    ensureNotRunning();
    const p = profileDoc(args.id);
    settings.active_profile_id = p.id;
    emitProfiles();
    return p;
  },
  profile_create: (args) => createProfile(args.req as NewProfile),
  profile_rename: (args) => {
    const p = profileDoc(args.id);
    if (p.id === settings.active_profile_id) ensureNotRunning();
    p.name = String(args.name).trim();
    p.updated_at = new Date().toISOString();
    emitProfiles();
    return p;
  },
  profile_clone: (args) => {
    const src = profileDoc(args.srcId);
    const name = String(args.newName).trim();
    if (!name) fail("invalid_input", "Profile name cannot be empty.");
    return addProfile(newDoc(freshId(name), name, { ...structuredClone(src), stacking: src.stacking ?? null, notices_seen: [...(src.notices_seen ?? [])] }));
  },
  profile_update: (args) => updateProfile(String(args.id), args.patch as ProfileSettingsPatch),
  profile_delete: (args) => {
    const p = profileDoc(args.id);
    if (p.id === settings.active_profile_id) fail("conflict", "Switch to another profile before deleting this one.");
    delete profileDocs[p.id];
    emitProfiles();
    return null;
  },
  profile_export_code: (args) => {
    const p = profileDoc(args.id);
    return `QM1-${btoa(JSON.stringify({ name: p.name, enabled_mods: p.enabled_mods }))}`;
  },
  profile_import_code: (args) => importCode(String(args.code), String(args.newId)),
  profile_get: (args) => {
    const id = String(args.id);
    return profileDocs[id] ?? fail("not_found", `No profile ${id}.`);
  },
};
