import type { IssueReportRequest, IssueReportResult, LegacyData, ManualImportResult, ManualInstall } from "../../bindings";
import { emitMock } from "../../events";
import { settings } from "../fixtures";
import { ensureNotRunning, fail, log, state, type Handlers } from "../state";

/** A manual install in the game folder; there until it is imported. Asked for by `__qmMockInit.manualInstall`. */
let manualDone = false;
function manualInstall(): ManualInstall | null {
  if (!state.init.manualInstall || manualDone) return null;
  return {
    game_dir: "C:\\Games\\Mud and Blood",
    has_loader: true,
    has_mods_folder: true,
    mods: [
      { id: "night-ops", name: "Night Ops", version: "1.0.0", version_missing: false, installed_version: null, folder: "NightOps" },
      { id: "old_sounds", name: "Old Sounds", version: "0.0.0", version_missing: true, installed_version: null, folder: "Old Sounds" },
      { id: "gcore", name: "GCore", version: "1.1.0", version_missing: false, installed_version: "1.2.0", folder: "GCore" },
    ],
    unreadable: [{ folder: "Broken", reason: "Failed to parse mod.json: missing field `id`" }],
  };
}

let legacyDone = false;
function legacyData(): LegacyData | null {
  if (!state.init.legacyData || legacyDone) return null;
  return { path: "C:\\Users\\Player\\AppData\\Roaming\\QuarterMaster", mods: 6, profiles: 2 };
}

export const handlers: Handlers = {
  manual_install_check: () => manualInstall(),
  manual_install_import: () => {
    ensureNotRunning();
    const found = manualInstall() ?? fail("not_found", "No manual install was found in the game folder.");
    manualDone = true;
    log.manualImports += 1;
    const result: ManualImportResult = {
      imported: found.mods.filter((m) => m.installed_version === null).map((m) => m.name),
      unversioned: found.mods.filter((m) => m.installed_version === null && m.version_missing).map((m) => m.name),
      skipped: [
        ...found.unreadable,
        ...found.mods
          .filter((m) => m.installed_version !== null)
          .map((m) => ({ folder: m.folder, reason: `${m.name} ${m.installed_version} is already in the library.` })),
      ],
      enabled: true,
      backup_dir: "C:\\QuarterMaster\\backups\\manual-install-20261115-101500",
      loader_removed: found.has_loader,
    };
    setTimeout(() => emitMock({ type: "invalidated", scope: "mods" }), 0);
    return result;
  },
  legacy_data_check: () => legacyData(),
  legacy_data_move: () => {
    ensureNotRunning();
    if (!legacyData()) fail("not_found", "There is no earlier data to move.");
    legacyDone = true;
    log.legacyMoves += 1;
    settings.first_run_completed = true;
    return { files: 412, old_removed: true };
  },
  app_restart: () => {
    log.restarts += 1;
    return null;
  },
  issue_report_build: (args) => {
    const req = args.req as IssueReportRequest;
    if (!req.title.trim() || !req.description.trim()) fail("invalid_input", "Give the report a title and say what happened.");
    log.issueReports.push(req);
    const result: IssueReportResult = {
      url: `https://github.com/Gamiseus/QuarterMaster/issues/new?template=bug_report.yml&title=${encodeURIComponent(req.title.trim())}`,
      file: req.attach_files ? "C:\\QuarterMaster\\cache\\diagnostics\\QuarterMaster-report-20261115-101500.zip" : null,
    };
    return result;
  },
};
