import type { DetectedConflict, LoadError } from "../bindings";
import { waitingNotes } from "../diagnostics/health";
import { attentionLines } from "../patches/fixlist";
import { app } from "./app.svelte";
import { fixes } from "./fixes.svelte";
import { game } from "./game.svelte";
import { lineRules } from "./linerules.svelte";
import { loadOrder } from "./loadorder.svelte";
import { profiles } from "./profiles.svelte";
import { stacking } from "./stacking.svelte";

/** Shared state for the pre-launch gate: error diagnostics block a plain launch until confirmed. */
class PreLaunchStore {
  open = $state(false);
  launching = $state(false);
  /**
   * The dialog opened with the stacking notice (D3) or the line rules notice (D51); it stays open
   * after the choice so the user can launch.
   */
  notice = $state(false);
  /** The Launch button is waiting for the stacking check before it opens the notice. */
  checking = $state(false);

  /** The built-in Vanilla profile is active: it loads no mods, so the load order gate does not apply. */
  get vanillaActive(): boolean {
    return profiles.activeSummary?.is_builtin ?? false;
  }

  /**
   * Whether a launch may start now: startup finished, nothing running, a profile is active, and
   * (unless it is the Vanilla profile) the resolution is current and the simulation is ready, so
   * the error gate cannot be skipped: files that do not compile, mods that fail to load and an
   * unusable load order file are only known from the simulation, which a profile write clears.
   */
  get canLaunch(): boolean {
    const gateReady = this.vanillaActive || (loadOrder.resolutionReady && loadOrder.simulation.status === "ready");
    return app.startupDone && !game.isBusy && !this.launching && profiles.activeSummary !== null && gateReady;
  }

  /** Patched files that would not compile (the game would stop with a Lua error). */
  get compileErrors(): DetectedConflict[] {
    const sim = loadOrder.simulation;
    return sim.status === "ready" ? sim.data.conflicts.filter((c) => c.kind === "compile_error") : [];
  }

  /** Files a patch aborts (`on_failure = "abort"`): the game would fail to load them with a Lua error. */
  get abortedFiles(): DetectedConflict[] {
    const sim = loadOrder.simulation;
    return sim.status === "ready" ? sim.data.conflicts.filter((c) => c.kind === "patch_aborts_file") : [];
  }

  /** The profile's load-order.json cannot be used: the game would start with no mods. */
  get planError(): string | null {
    const sim = loadOrder.simulation;
    return sim.status === "ready" ? sim.data.plan_error : null;
  }

  /** Mods that fail to load. The game skips each one and the mods that require it. */
  get loadErrors(): LoadError[] {
    const sim = loadOrder.simulation;
    return sim.status === "ready" ? sim.data.load_errors : [];
  }

  /** Adjusted copies the game skips while their mod is off (D19). They do not block a launch. */
  get waiting(): string[] {
    const sim = loadOrder.simulation;
    return sim.status === "ready" ? waitingNotes(sim.data.fix_bindings) : [];
  }

  /** Fixes to look at before playing (D23): Needs review, Suspended, Uses its own line. They do not block. */
  get fixWarnings(): string[] {
    return attentionLines(fixes.attention);
  }

  /**
   * Whether anything blocks a plain launch: load order errors, files that do not compile or that
   * a patch aborts, mods that fail to load, or a load order file the game cannot use.
   */
  get blocked(): boolean {
    return (
      loadOrder.errors.length > 0 ||
      this.compileErrors.length > 0 ||
      this.abortedFiles.length > 0 ||
      this.loadErrors.length > 0 ||
      this.planError !== null
    );
  }

  /** Opens the dialog; does nothing when nothing blocks a launch. */
  review(): void {
    if (this.blocked) this.open = true;
  }

  /**
   * Launch button entry point: gated by errors (the Vanilla profile loads no mods, so it is not), and
   * by the stacking notice while this profile has not seen it, so the user never first meets the new
   * order after playing (D3), and by the line rules notice while the newer rules would change this
   * profile and it has no answer (D51; launching without one keeps the older rules). Fixes that
   * need a look open the dialog too, with Launch (D23).
   */
  async request(): Promise<void> {
    if (this.vanillaActive) return this.launch();
    if (this.checking) return;
    if (stacking.unseen) {
      this.checking = true;
      try {
        await stacking.ensure();
      } finally {
        this.checking = false;
      }
    }
    if (stacking.noticeBeforeLaunch || lineRules.showNotice) {
      this.notice = true;
      this.open = true;
    } else if (this.blocked || this.fixWarnings.length > 0) {
      this.open = true;
    } else {
      await this.launch();
    }
  }

  /**
   * Launches the active profile, ignoring diagnostics; `withoutMods` starts the game with no mods.
   * `noticeShown`: the dialog showed the stacking notice; left unanswered, it counts as seen and the
   * older order stays.
   */
  async launch(withoutMods = false, noticeShown = false): Promise<void> {
    const id = profiles.activeSummary?.id;
    if (!id || !this.canLaunch) return;
    this.launching = true;
    try {
      if (noticeShown && !withoutMods && stacking.unseen) await stacking.dismiss().catch(() => {});
      if (withoutMods) await game.launch(id, null, true);
      else await game.launch(id);
    } catch {
      // Routed to the UI by call().
    } finally {
      this.launching = false;
    }
  }
}

export const prelaunch = new PreLaunchStore();
