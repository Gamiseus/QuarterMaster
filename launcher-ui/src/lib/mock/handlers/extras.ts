import type { FileView, LeftoverBackup, ModsSource, WindowSettings } from "../../bindings";
import { emitMock } from "../../events";
import { mods, settings } from "../fixtures";
import { runMockJob } from "../jobs";
import { activeProfile, ensureNotRunning, fail, profileChanged, state, type Handlers } from "../state";

/** Copies of unlinked `Mods` folders; empty unless `__qmMockInit.leftovers` asks for them. */
let leftovers: LeftoverBackup[] | null = null;
function leftoverList(): LeftoverBackup[] {
  leftovers ??= state.init.leftovers
    ? [
        { name: "unlinked-mods-20261102-181455", size_bytes: 1_610_612_736, file_count: 4821, contents: ["GCore", "OldTest", "qm"], modified: 1793643295 },
        { name: "unlinked-mods-20261028-090210", size_bytes: 734_003_200, file_count: 1904, contents: ["OldTest"], modified: 1793178130 },
      ]
    : [];
  return leftovers;
}

/** A 2x2 PNG, so image previews have something to show. */
const PNG = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFElEQVR4nGP8z8Dwn4GBgYGJAQoAHxcCAk+Uzr4AAAAASUVORK5CYII=";

function fileView(source: string, path: string): FileView {
  if (path.includes("..") || path.includes(":")) fail("invalid_input", `'${path}' is not a file inside the folder.`);
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  if (path.includes("missing")) fail("not_found", source === "vanilla" ? `The game has no file ${path}.` : `${source} has no file ${path}.`);
  if (["png", "jpg", "jpeg", "gif", "bmp", "webp"].includes(ext)) {
    return { source, path, kind: "image", size_bytes: 91, text: null, data_url: PNG };
  }
  if (["ogg", "wav", "mp3", "flac"].includes(ext)) {
    return { source, path, kind: "audio", size_bytes: 20480, text: null, data_url: "data:audio/ogg;base64,T2dnUw==" };
  }
  if (["lua", "txt", "toml", "json", "md"].includes(ext)) {
    const who = source === "vanilla" ? "the game" : source;
    const text = `-- ${path} (${who})\nlocal M = {}\n\nfunction M.update(dt)\n    return dt\nend\n\nreturn M\n`;
    return { source, path, kind: "text", size_bytes: text.length, text, data_url: null };
  }
  return { source, path, kind: "other", size_bytes: 4096, text: null, data_url: null };
}

const settingsChanged = () => setTimeout(() => emitMock({ type: "invalidated", scope: "settings" }), 0);

export const handlers: Handlers = {
  mod_color_set: (args) => {
    const id = String(args.modId);
    const map = { ...settings.mod_colors };
    if (args.color === null || args.color === undefined) delete map[id];
    else {
      const c = Number(args.color);
      if (!Number.isInteger(c) || c < 1 || c > 8) fail("invalid_input", "A mod colour is a number from 1 to 8.");
      map[id] = c;
    }
    settings.mod_colors = map;
    settingsChanged();
    return { ...settings };
  },
  window_settings_set: (args) => {
    ensureNotRunning();
    const w = args.window as WindowSettings;
    for (const [label, v] of [["width", w.width], ["height", w.height]] as const) {
      if (v != null && (v < 320 || v > 16384)) fail("invalid_input", `The window ${label} must be between 320 and 16384.`);
    }
    settings.window = { ...w };
    settingsChanged();
    return { ...settings };
  },
  mods_source_set: (args) => {
    ensureNotRunning();
    const source = args.source as ModsSource;
    if (source === "workspace" && !settings.workspace_mods_dir) fail("not_found", "Set a workspace mods folder in Settings first.");
    // The real launcher starts again here; the mock flips in place.
    settings.mods_source = source;
    settings.workspace_mode = source === "workspace";
    settingsChanged();
    return null;
  },
  leftover_backups_list: () => leftoverList().map((b) => ({ ...b })),
  leftover_backup_delete: (args) => {
    const name = String(args.name);
    const list = leftoverList();
    const at = list.findIndex((b) => b.name === name);
    if (at < 0) fail("not_found", `There is no copy called '${name}'.`);
    list.splice(at, 1);
    return null;
  },
  game_dump_full: () => {
    if (!settings.workspace_mods_dir) fail("not_found", "The workspace mods folder is not set.");
    return runMockJob("game_dump", "Writing the game's files", () => ({ output: "1284" }));
  },
  file_view: (args) => fileView(String(args.source), String(args.path)),
  conflict_dismiss: (args) => {
    ensureNotRunning();
    const id = String(args.conflictId);
    const p = activeProfile();
    const had = (p.dismissed_conflicts ?? []).includes(id);
    if (had === Boolean(args.dismissed)) return null;
    p.dismissed_conflicts = args.dismissed ? [...(p.dismissed_conflicts ?? []), id] : (p.dismissed_conflicts ?? []).filter((c) => c !== id);
    profileChanged(p);
    return null;
  },
  mod_version_set: (args) => {
    ensureNotRunning();
    const id = String(args.id);
    const m = mods.find((x) => x.id === id) ?? fail("not_found", `Mod '${id}' is not installed.`);
    const version = (args.version ?? null) as string | null;
    if (version !== null && !m.installed_versions.includes(version)) fail("not_found", `Version ${version} of ${m.name} is not installed.`);
    const newest = m.installed_versions[0] ?? m.version;
    const p = activeProfile();
    const pins = { ...(p.pinned_versions ?? {}) };
    if (version === null || version === newest) delete pins[id];
    else pins[id] = version;
    p.pinned_versions = pins;
    m.version = version ?? newest;
    profileChanged(p);
    setTimeout(() => emitMock({ type: "invalidated", scope: "mods" }), 0);
    return { ...m, enabled: p.enabled_mods.includes(id) };
  },
};
