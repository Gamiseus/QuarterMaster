import type {
  FixSuggestion,
  GameStatus,
  ImportRequest,
  JobInfo,
  LaunchRequest,
  LoadError,
  LoadOrderUpdate,
  ModProfile,
  OrderRule,
  QmError,
  RevealKind,
  RunOutcome,
  StackMode,
  TrialRequest,
} from "../bindings";
import { emitMock } from "../events";
import { profileDocs, settings } from "./fixtures";

/** What the mock backend was asked to do; e2e specs read it through `window.__qmMockLog`. */
export interface MockLog {
  lastLoadOrderSet: LoadOrderUpdate | null;
  launches: LaunchRequest[];
  imports: ImportRequest[];
  uninstalls: string[];
  updateInstalls: string[];
  catalogInstalls: string[];
  reveals: RevealKind[];
  /** Dossier exports: the slot name and the text, as the save dialog would have written it. */
  dossierExports: { slotName: string; text: string }[];
  /** How many explicit update checks ran (Check now, the 6-hour timer). */
  updateChecks: number;
  /** Settings and onboarding patches. */
  settingsPatches: Record<string, unknown>[];
  /** Glue patches created and order rules set (patch tools). */
  glueCreates: string[];
  orderRules: OrderRule[];
  /** Stacking modes set and one-time notices dismissed. */
  stackingSets: StackMode[];
  noticeDismissals: string[];
  /** Fix ledger writes, as "create:<kind>:<patch>", "undo:<id>", "pause:<id>", ... */
  fixes: string[];
  /** Compatibility patch exports (Task 19). */
  compatExports: string[];
  /** Test runs started from "Find the cause" (D68). */
  trials: TrialRequest[];
}

/** Set by e2e specs (via an init script) before the app loads to steer the mock backend. */
export interface MockInit {
  firstRun?: boolean;
  /** Answers for `plugin:dialog|open`, used in order; `null` is a cancelled dialog. */
  dialog?: (string | null)[];
  restore?: "ok" | "integrity";
  /** github.com answers 429: the update report says `busy`. */
  githubBusy?: boolean;
  /** The mod index site is unreachable: Browse shows the offline copy. */
  indexOffline?: boolean;
  /** Adds SoundSwap, an asset-only mod, to the library and the active profile. */
  assetMod?: boolean;
  /** The asset ledger lists overlaps: two sound mods and a faction on the same files. */
  assetOverlaps?: boolean;
  /** Two enabled factions replace the Allies, so the faction picker shows. */
  twoFactions?: boolean;
  /** The Ironman slot's save holds values from two mods no profile loads any more. */
  saveLeftover?: boolean;
  /** The code checks flagged names, so a diagnostic test run has names to suggest. */
  traceSuggestions?: boolean;
  /** Starting health report: left modded with a foreign DLL, or a mismatched QuarterMaster DLL. */
  health?: "healthy" | "modded" | "mismatch" | "attention";
  /** The active profile's run log holds one finished run with this outcome (D67). None by default. */
  lastRun?: RunOutcome;
  /** How a test run from "Find the cause" ends (D68); clean by default. */
  trialOutcome?: RunOutcome;
  /** A fix, patch or mod id: a test run crashes unless it leaves this out (the search's seeded cause, D68). */
  bisectCulprit?: string;
  /** How long a test run plays before the game closes, in ms (default 600). */
  trialMs?: number;
  /** The save backup before a test run fails, so the test is refused. */
  trialBackupFails?: boolean;
  /** Starting appearance settings (colour vision, contrast, text size). */
  appearance?: { color_vision?: string; high_contrast?: boolean; text_scale?: number };
  /** The simulation reports that load-order.json cannot be used (the game would load no mods). */
  planError?: string;
  /** The simulation reports mods that fail to load (each skipped with its requirers). */
  loadErrors?: LoadError[];
  /** The active profile is from before stacking: no mode set and the stacking notice unseen (D3). */
  stackingUnseen?: boolean;
  /** The active profile has a Disable fix whose patch its mod changed since (Needs review), and a working order fix. */
  driftedFix?: boolean;
  /** The active profile has a Disable fix that a GMenu update made Ambiguous: Review asks which patch it is for (§9.3). */
  ambiguousFix?: boolean;
  /** The active profile has an adjusted copy of Vehicle init from before copies ran as their base mod (D58). */
  legacyCopy?: boolean;
  /** The working order fix was saved as a compatibility patch and a newer rule for its patch made since (D71). */
  exportClash?: boolean;
  /** Importing the earlier fixes into the ledger fails with this reason (D38). */
  migrationError?: string;
  /** Adds GCore's Clear pathfinding on Halt, which takes the line AI Tweaks' Halt reset needs (region options, §9.2). */
  regionConflict?: boolean;
  /** Adds the Halt split of §10.1: GCore's line splits AI Tweaks' Clear path on Halt, which then runs as separate edits. */
  haltSplit?: boolean;
  /** The simulation reports order rules and places it does not use (G15). */
  orderReports?: boolean;
  /** Minimap is turned off, so Compass, which anchors on Minimap's code, fails under it (G10). */
  failureChain?: boolean;
  /** Adds AI Compat, a compatibility mod with one applied and one ignored override (§11.7). */
  compatMod?: boolean;
  /** "Save as compatibility patch" finds that the game code would change (the export is refused). */
  exportDiffers?: boolean;
  /** A preview of an Anchor fix with a rung on the line that reads this stops its file from compiling (§11.7). */
  breaksOn?: string;
  /** Suggested fixes the Fixes view lists (§9.1); none by default. */
  suggestions?: FixSuggestion[];
  /** How long the suggestions stay pending after the first read, like the backend's job. */
  suggestionsDelayMs?: number;
  /** The suggestions stay pending until `window.__qmMockReleaseSuggestions()` (tests that must not race a timer). */
  suggestionsHeld?: boolean;
  /** The active profile has a Disable fix made before a game update changed its file, so it needs review (D74). */
  gameUpdated?: boolean;
  /**
   * Vehicle Pack (Red Army) and AI Tweaks (Blue Army) each replace the allied side, and a load order
   * fix gave Vehicle Pack the higher priority, so Red Army runs. Previews list faction switches (D73).
   */
  factionOrder?: boolean;
}

declare global {
  interface Window {
    __qmMockLog?: MockLog;
    __qmMockInit?: MockInit;
    __qmMockReleaseSuggestions?: () => void;
  }
}

export const log: MockLog = { lastLoadOrderSet: null, launches: [], imports: [], uninstalls: [], updateInstalls: [], catalogInstalls: [], reveals: [], dossierExports: [], updateChecks: 0, settingsPatches: [], glueCreates: [], orderRules: [], stackingSets: [], noticeDismissals: [], fixes: [], compatExports: [], trials: [] };

/** Running mock jobs, as `jobs_list` reports them. */
export const jobs: JobInfo[] = [];

/** Mutable mock backend state shared by every handler module. */
export const state: { game: GameStatus; init: MockInit; simulating: boolean } = {
  game: { stage: { stage: "idle" }, profile_id: null, is_modded: false },
  init: {},
  /** A profile write cleared the simulation and the new one has not landed (`simulation_get` is loading). */
  simulating: false,
};

/** How long the mock's simulation runs after a profile write. */
const SIMULATION_MS = 40;
let simulationTimer: ReturnType<typeof setTimeout> | null = null;

export function fail(kind: QmError["kind"], message: string): never {
  const error: QmError = { kind, message, details: null };
  throw error;
}

export function setGame(status: GameStatus): void {
  state.game = status;
  emitMock({ type: "game_status_changed", status });
}

export function isGameBusy(): boolean {
  return state.game.stage.stage !== "idle" && state.game.stage.stage !== "failed";
}

export function ensureNotRunning(): void {
  if (isGameBusy()) fail("game_running", "The game is running. Close it to change mods.");
}

export function activeProfile(): ModProfile {
  return profileDocs[settings.active_profile_id] ?? fail("not_found", "No active profile.");
}

const PERSIST_KEY = "qm-mock-profiles";

/** With `?mockPersist=1` the profiles survive a reload (e2e of undo after a restart). */
export function persisting(): boolean {
  try {
    return new URLSearchParams(window.location.search).get("mockPersist") === "1";
  } catch {
    return false;
  }
}

function persist(): void {
  if (!persisting()) return;
  try {
    sessionStorage.setItem(PERSIST_KEY, JSON.stringify(profileDocs));
  } catch {
    // Storage blocked or full: the mock just forgets on reload.
  }
}

/** Loads the saved profiles; true when there were some. */
export function restoreProfiles(): boolean {
  if (!persisting()) return false;
  try {
    const raw = sessionStorage.getItem(PERSIST_KEY);
    if (!raw) return false;
    const saved = JSON.parse(raw) as Record<string, ModProfile>;
    for (const k of Object.keys(profileDocs)) delete profileDocs[k];
    Object.assign(profileDocs, saved);
    return true;
  } catch {
    return false;
  }
}

/**
 * Mirrors the service: a profile write invalidates profiles, load order and mod rows, and clears
 * the simulation; `simulation_get` is loading until the new one lands with another load order
 * event, so the UI's refresh path runs as it does with the real backend.
 */
export function profileChanged(profile: ModProfile): void {
  profile.updated_at = new Date().toISOString();
  persist();
  state.simulating = true;
  setTimeout(() => {
    emitMock({ type: "invalidated", scope: "profiles" });
    emitMock({ type: "invalidated", scope: "load_order" });
    emitMock({ type: "invalidated", scope: "mods" });
  }, 0);
  if (simulationTimer !== null) clearTimeout(simulationTimer);
  simulationTimer = setTimeout(() => {
    simulationTimer = null;
    state.simulating = false;
    emitMock({ type: "invalidated", scope: "load_order" });
  }, SIMULATION_MS);
}

/** Handler maps: command name to handler (args are the raw IPC payload); each `handlers/*` module exports one. */
export type Handlers = Record<string, (args: any) => unknown>;
