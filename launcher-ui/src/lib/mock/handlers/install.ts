import type { ArchiveModCandidate, ImportRequest, ModProfile } from "../../bindings";
import { emitMock } from "../../events";
import { mods, profileDocs } from "../fixtures";
import { runMockJob } from "../jobs";
import { activeProfile, ensureNotRunning, fail, jobs, log, profileChanged, type Handlers } from "../state";

function candidate(id: string, name: string, version: string): ArchiveModCandidate {
  const installed = mods.find((m) => m.id === id);
  return {
    manifest: { id, name, version },
    relative_folder: id,
    already_installed_version: installed?.version ?? null,
    all_installed_versions: installed ? [...installed.installed_versions] : [],
  };
}

function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** What an archive at `path` contains; the fixed names steer e2e specs. */
function inspect(path: string, allowUnmanaged: boolean): ArchiveModCandidate[] {
  const name = fileName(path);
  const lower = name.toLowerCase();
  if (lower.endsWith("pack.zip")) return [candidate("field-radio", "Field Radio", "1.0.0"), candidate("gmenu", "GMenu", "1.3.0")];
  if (lower.endsWith("badid.zip")) fail("invalid_input", "Mod id '..' is not a valid folder name.");
  if (lower.endsWith("notmod.zip")) {
    if (!allowUnmanaged) fail("invalid_input", `No \`qm.toml\`, \`faction.toml\` or supported \`assets/\` directory found in '${name}'.`);
    return [candidate("notmod", "notmod", "0.0.0")];
  }
  const base = name.replace(/\.zip$/i, "");
  return [candidate(base.toLowerCase().replace(/[^a-z0-9_-]+/g, "-"), base, "1.0.0")];
}

function installOne(c: ArchiveModCandidate, policy: ImportRequest["conflict_policy"]): void {
  const { id, name, version } = c.manifest;
  const existing = mods.find((m) => m.id === id);
  if (!existing) {
    const priority = Math.max(0, ...mods.map((m) => m.load_priority)) + 10;
    mods.push({
      id,
      name,
      version,
      authors: [],
      category: null,
      enabled: false,
      patch_count: 0,
      installed_versions: [version],
      is_workspace_linked: false,
      has_config: false,
      load_priority: priority,
      kind: "managed",
      faction: null,
    });
    return;
  }
  const others = policy === "replace" ? [] : existing.installed_versions.filter((v) => v !== version);
  existing.installed_versions = [...others, version];
  if (policy !== "install_alongside") existing.version = version;
}

function importMods(req: ImportRequest) {
  ensureNotRunning();
  const selected = req.selected_mod_ids && req.selected_mod_ids.length > 0 ? new Set(req.selected_mod_ids) : null;
  const picked = inspect(req.path, req.allow_unmanaged).filter((c) => !selected || selected.has(c.manifest.id));
  log.imports.push(structuredClone(req));
  return runMockJob("mod_import", "Importing", () => {
    ensureNotRunning();
    for (const c of picked) installOne(c, req.conflict_policy);
    emitMock({ type: "invalidated", scope: "mods" });
    return { output: picked.map((c) => c.manifest.id).join(", ") };
  });
}

function strip(p: ModProfile, id: string): void {
  p.enabled_mods = p.enabled_mods.filter((m) => m !== id);
  if (p.mod_priorities) delete p.mod_priorities[id];
  if (p.user_load_after) {
    delete p.user_load_after[id];
    for (const [k, v] of Object.entries(p.user_load_after)) p.user_load_after[k] = v?.filter((m) => m !== id);
  }
  p.locked_mods = (p.locked_mods ?? []).filter((m) => m !== id);
  for (const sep of p.separators ?? []) if (sep.insert_before_mod_id === id) sep.insert_before_mod_id = null;
}

function uninstall(id: string): null {
  ensureNotRunning();
  if (jobs.some((j) => j.kind === "mod_import")) fail("conflict", "Wait for the install to finish.");
  const i = mods.findIndex((m) => m.id === id);
  if (i < 0) fail("not_found", `Mod '${id}' is not installed.`);
  mods.splice(i, 1);
  for (const p of Object.values(profileDocs)) strip(p, id);
  log.uninstalls.push(id);
  profileChanged(activeProfile());
  return null;
}

export const handlers: Handlers = {
  mod_import_inspect: (args) => inspect(String(args.path), Boolean(args.allowUnmanaged)),
  mod_import: (args) => importMods(args.req as ImportRequest),
  mod_uninstall: (args) => uninstall(String(args.id)),
};
