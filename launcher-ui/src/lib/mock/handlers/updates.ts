import type { CatalogQuery, CatalogResult, UpdatesReport } from "../../bindings";
import { emitMock } from "../../events";
import { catalogFixture } from "../catalog";
import { mods, ready, updatesReport } from "../fixtures";
import { runMockJob } from "../jobs";
import { ensureNotRunning, fail, log, state, type Handlers } from "../state";

const invalidate = (scope: "updates" | "mods" | "catalog") => emitMock({ type: "invalidated", scope });

function report(): UpdatesReport {
  return { ...updatesReport, rate_limit_mode: state.init.githubBusy ? "busy" : updatesReport.rate_limit_mode };
}

function installUpdate(modId: string) {
  ensureNotRunning();
  const info = updatesReport.mods.find((m) => m.mod_id === modId && m.update_available);
  if (!info) fail("not_found", `No update for '${modId}'.`);
  log.updateInstalls.push(modId);
  return runMockJob("mod_update_install", `Updating ${info.name}`, () => {
    ensureNotRunning();
    const m = mods.find((x) => x.id === modId);
    if (m) {
      m.version = info.latest_version;
      m.installed_versions = [...new Set([...m.installed_versions, info.latest_version])];
    }
    info.installed_version = info.latest_version;
    info.update_available = false;
    invalidate("mods");
    invalidate("updates");
    return { output: modId };
  });
}

const filter = (v: string | null | undefined) => (v && v.trim() && v.trim().toLowerCase() !== "all" ? v.trim().toLowerCase() : null);

function catalogGet(query: CatalogQuery) {
  const text = filter(query.text);
  const topic = filter(query.topic);
  const source = filter(query.source);
  const entries = catalogFixture
    .map(({ entry }) => {
      const installed = mods.find((m) => m.name.toLowerCase() === entry.name.toLowerCase());
      return { ...entry, installed: installed !== undefined, installed_version: installed?.version ?? null };
    })
    .filter(
      (e) =>
        (!text || [e.name, e.description, e.owner, e.repo_full_name].some((s) => s.toLowerCase().includes(text))) &&
        (!topic || e.topics.includes(topic)) &&
        (!source || e.source_type.toLowerCase() === source),
    );
  const result: CatalogResult = {
    catalog: entries,
    total_count: entries.length,
    is_cached: true,
    query: text ?? "",
    rate_limited: false,
    error_message: null,
    offline: state.init.indexOffline ?? false,
    copy_from: "2026-11-05T18:30:00Z",
  };
  return ready(result);
}

function catalogInstall(downloadUrl: string) {
  ensureNotRunning();
  const item = catalogFixture.find((c) => c.entry.download_url === downloadUrl);
  if (!item) fail("invalid_input", "This download is not in the catalog.");
  log.catalogInstalls.push(downloadUrl);
  return runMockJob("catalog_install", `Installing ${item.entry.name}`, () => {
    ensureNotRunning();
    const { id, version } = item.manifest;
    const existing = mods.find((m) => m.id === id);
    if (existing) {
      existing.version = version;
      existing.installed_versions = [...new Set([...existing.installed_versions, version])];
    } else {
      mods.push({
        id,
        name: item.entry.name,
        version,
        authors: [item.entry.owner],
        category: null,
        enabled: false,
        patch_count: 0,
        installed_versions: [version],
        is_workspace_linked: false,
        has_config: false,
        load_priority: Math.max(0, ...mods.map((m) => m.load_priority)) + 10,
        kind: "managed",
        faction: null,
      });
    }
    invalidate("mods");
    invalidate("catalog");
    return { output: id };
  });
}

export const handlers: Handlers = {
  updates_get: () => ready(report()),
  updates_refresh: () => {
    log.updateChecks++;
    return runMockJob("updates_refresh", "Checking for updates", () => {
      invalidate("updates");
      return {};
    });
  },
  mod_update_install: (args) => installUpdate(String(args.modId)),
  catalog_get: (args) => catalogGet(args.query as CatalogQuery),
  catalog_refresh: () =>
    runMockJob("catalog_refresh", "Loading community catalog", () => {
      invalidate("catalog");
      return {};
    }),
  catalog_install: (args) => catalogInstall(String(args.downloadUrl)),
};
