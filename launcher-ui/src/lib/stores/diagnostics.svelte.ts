import { call, QmCommandError } from "../api";
import {
  commands,
  type BisectPreview,
  type BisectStatus,
  type BisectView,
  type CrashReport,
  type HealthReport,
  type Loadable,
  type PreviewResult,
  type QmEvent,
  type RunEntry,
  type TrialChanges,
  type TrialResult,
  type TrialSuspect,
} from "../bindings";
import { app } from "./app.svelte";
import { jobs } from "./jobs.svelte";
import { loadOrder } from "./loadorder.svelte";
import { toasts } from "./toasts.svelte";

/** Health report and crash list for the Diagnostics screen, plus its repair actions. */
class DiagnosticsStore {
  health = $state<Loadable<HealthReport>>({ status: "loading" });
  crashes = $state<CrashReport[] | null>(null);
  #healthSeq = 0;
  #crashSeq = 0;

  async loadHealth(): Promise<void> {
    const seq = ++this.#healthSeq;
    const health = await call(commands.healthGet());
    if (seq === this.#healthSeq) this.health = health;
  }

  async loadCrashes(): Promise<void> {
    const seq = ++this.#crashSeq;
    const crashes = await call(commands.crashesList());
    if (seq === this.#crashSeq) this.crashes = crashes;
  }

  load(): void {
    this.loadHealth().catch(() => {});
    this.loadCrashes().catch(() => {});
  }

  async repairDll(): Promise<void> {
    await call(commands.dllRepair());
    toasts.info("The hook DLL was repaired.");
  }

  /**
   * Restores the clean game folder. An unknown winmm.dll is left in place and reported as an
   * `integrity` error: that is a warning (shown as an info toast), not a failure.
   */
  async restoreClean(): Promise<"ok" | "warning"> {
    try {
      await call(commands.gameRestoreClean(), { expect: ["integrity"] });
    } catch (e) {
      if (e instanceof QmCommandError && e.kind === "integrity") {
        toasts.info(e.message);
        return "warning";
      }
      throw e;
    }
    toasts.info("The game folder is clean.");
    return "ok";
  }

  /** Empties the active profile's overwrite folder (files the game wrote into its Mods folder). */
  async clearOverwrite(): Promise<void> {
    const n = await call(commands.overwriteClear());
    toasts.info(n === 1 ? "Removed 1 overwrite file." : `Removed ${n} overwrite files.`);
  }

  /** Records the current game build, so the game update warning stops. */
  async acknowledgeUpdate(): Promise<void> {
    await call(commands.gameUpdateAcknowledge());
    toasts.info("The game update is marked as checked.");
  }

  async disableSuspect(crashId: string): Promise<void> {
    await loadOrder.whileChanging(() => call(commands.crashDisableSuspect(crashId)));
  }

  /** What crash `crashId` blames, as a test run can leave it out (D68); null when it names nothing. */
  suspect(crashId: string): Promise<TrialSuspect | null> {
    return call(commands.crashSuspect(crashId), { expect: ["not_found"] });
  }

  /** What a test run with `changes` would run, and what keeping them would do; nothing is saved. */
  trialPreview(changes: TrialChanges): Promise<PreviewResult> {
    return call(commands.trialPreview(changes), { expect: ["conflict", "invalid_input", "cancelled"] });
  }

  /** The search for a crash's cause (D68 step 2), or a note when a restart ended the last one. */
  bisectGet(): Promise<BisectStatus> {
    return call(commands.bisectGet());
  }

  /** Starts a search for the cause of crashed run `runId` of the active profile. */
  bisectStart(runId: string): Promise<BisectView> {
    return call(commands.bisectStart(runId), { expect: ["not_found", "invalid_input", "conflict", "game_running"] });
  }

  /** Picks the next test and previews it; tests that would stop a file from compiling are skipped. */
  bisectPreview(): Promise<BisectPreview> {
    return call(commands.bisectPreview(), { expect: ["not_found", "conflict", "invalid_input"] });
  }

  /** The answer to "Did the problem happen?" for a test the launcher could not read. */
  bisectAnswer(result: TrialResult): Promise<BisectView> {
    return call(commands.bisectAnswer(result));
  }

  bisectStop(): Promise<BisectView> {
    return call(commands.bisectStop());
  }

  /** A profile's run log, newest first. */
  runs(profileId: string): Promise<RunEntry[]> {
    return call(commands.runsList(profileId));
  }

  /** Builds the redacted bug report, then opens its fixed folder (never the path the job printed). */
  async buildBundle(): Promise<void> {
    const id = await call(commands.diagnosticBundleBuild());
    await jobs.track(id, { label: "Building the bug report", success: () => "Bug report saved. Its folder is open." });
    await app.reveal("diagnostics");
  }

  handle(e: QmEvent): void {
    if (e.type !== "invalidated") return;
    if (e.scope === "health") this.loadHealth().catch(() => {});
    else if (e.scope === "crashes") this.loadCrashes().catch(() => {});
  }

  /** Test-only. */
  reset(): void {
    this.#healthSeq++;
    this.#crashSeq++;
    this.health = { status: "loading" };
    this.crashes = null;
  }
}

export const diagnostics = new DiagnosticsStore();
