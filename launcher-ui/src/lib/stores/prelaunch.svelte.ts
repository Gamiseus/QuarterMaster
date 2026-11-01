import { app } from "./app.svelte";
import { game } from "./game.svelte";
import { loadOrder } from "./loadorder.svelte";
import { profiles } from "./profiles.svelte";

/** Shared state for the pre-launch gate: error diagnostics block a plain launch until confirmed. */
class PreLaunchStore {
  open = $state(false);
  launching = $state(false);
  /** UI-only launch mode: start the active profile's game without mods. Any profile choice clears it. */
  vanilla = $state(false);

  /**
   * Whether a launch may start now: startup finished, nothing running, a profile is active, and
   * (unless vanilla) the resolution is current so the error gate cannot be skipped.
   */
  get canLaunch(): boolean {
    const gateReady = this.vanilla || loadOrder.resolutionReady;
    return app.startupDone && !game.isBusy && !this.launching && profiles.activeSummary !== null && gateReady;
  }

  /** Opens the dialog; does nothing when there is no error diagnostic. */
  review(): void {
    if (loadOrder.errors.length > 0) this.open = true;
  }

  /** Launch button entry point: gated by errors (a vanilla launch loads no mods, so it is not). */
  async request(): Promise<void> {
    if (!this.vanilla && loadOrder.errors.length > 0) this.open = true;
    else await this.launch();
  }

  /** Launches the active profile, ignoring diagnostics. */
  async launch(): Promise<void> {
    const id = profiles.activeSummary?.id;
    if (!id || !this.canLaunch) return;
    this.launching = true;
    try {
      if (this.vanilla) await game.launch(id, "vanilla");
      else await game.launch(id);
    } catch {
      // Routed to the UI by call().
    } finally {
      this.launching = false;
    }
  }
}

export const prelaunch = new PreLaunchStore();
