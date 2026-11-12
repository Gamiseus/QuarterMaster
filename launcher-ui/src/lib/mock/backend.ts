import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emitMock } from "../events";
import { mods as fixtureMods, profileDocs, settings, soundSwap } from "./fixtures";
import { handlers as app } from "./handlers/app";
import { handlers as diagnostics } from "./handlers/diagnostics";
import { handlers as install } from "./handlers/install";
import { handlers as mods } from "./handlers/mods";
import { handlers as profiles } from "./handlers/profiles";
import { handlers as savesHandlers } from "./handlers/saves";
import { handlers as settingsHandlers } from "./handlers/settings";
import { handlers as updates } from "./handlers/updates";
import { addCompatScenario, COMPAT_MOD, seedExportClash } from "./compat";
import { releaseSuggestions, seedAmbiguousFix, seedDriftedFixes, seedFactionOrder, seedGameUpdate, seedLegacyCopy } from "./ledger";
import { addHaltScenario, addHaltSplitScenario } from "./patchworld";
import { fail, log, restoreProfiles, state, type Handlers } from "./state";

export type { MockInit, MockLog } from "./state";

const all = new Map<string, Handlers[string]>(
  [app, settingsHandlers, profiles, savesHandlers, mods, install, updates, diagnostics].flatMap((h) => Object.entries(h)),
);

function handle(cmd: string, args: Record<string, unknown> | undefined): unknown {
  const handler = all.get(cmd);
  // Like real IPC, handlers get a copy: never the caller's (possibly reactive) objects.
  if (handler) return handler(args ? (JSON.parse(JSON.stringify(args)) as Record<string, unknown>) : {});
  if (cmd.startsWith("plugin:window|")) return null;
  return fail("internal", `Mock backend has no handler for ${cmd}.`);
}

/** Applies `window.__qmMockInit`, set by e2e specs before the app loads. */
function applyInit(): void {
  const init = window.__qmMockInit ?? {};
  state.init = { ...init, dialog: init.dialog ? [...init.dialog] : undefined };
  if (init.firstRun) settings.first_run_completed = false;
  // Profiles saved by `?mockPersist=1` already carry the starting state.
  const restored = restoreProfiles();
  const active = restored ? undefined : profileDocs[settings.active_profile_id];
  if (active && init.stackingUnseen) {
    active.stacking = null;
    active.notices_seen = [];
  }
  if (active && init.driftedFix) seedDriftedFixes(active);
  if (active && init.ambiguousFix) seedAmbiguousFix(active);
  if (active && init.gameUpdated) seedGameUpdate(active);
  if (active && init.factionOrder) seedFactionOrder(active);
  if (active && init.legacyCopy) seedLegacyCopy(active);
  if (active && init.exportClash) seedExportClash(active);
  if (active && init.failureChain) active.disabled_patches = [...new Set([...(active.disabled_patches ?? []), "hud-minimap"])];
  if (active && init.migrationError) {
    active.disabled_patches = [...new Set([...(active.disabled_patches ?? []), "vehicle-init"])];
    active.migration_error = init.migrationError;
  }
  if (init.regionConflict) addHaltScenario();
  if (init.haltSplit) addHaltSplitScenario();
  if (init.compatMod && addCompatScenario() && !restored) {
    profileDocs.default.enabled_mods.push(COMPAT_MOD);
    profileDocs.default.mod_priorities = { ...profileDocs.default.mod_priorities, [COMPAT_MOD]: 70 };
  }
  if (init.appearance) Object.assign(settings, init.appearance);
  if (init.assetMod && !fixtureMods.some((m) => m.id === soundSwap.id)) {
    fixtureMods.push(soundSwap);
    if (!restored) profileDocs.default.enabled_mods.push(soundSwap.id);
  }
}

/** Answers commands with fixture data and exposes `window.__qmMockEmit`. */
export function installMockBackend(): void {
  applyInit();
  mockWindows("main");
  // Round-trip through JSON like real IPC, so callers never share fixture objects.
  mockIPC((cmd, args) => {
    const result = handle(cmd, args as Record<string, unknown> | undefined);
    return result === undefined ? null : JSON.parse(JSON.stringify(result));
  });
  window.__qmMockEmit = emitMock;
  window.__qmMockLog = log;
  window.__qmMockReleaseSuggestions = releaseSuggestions;
}
