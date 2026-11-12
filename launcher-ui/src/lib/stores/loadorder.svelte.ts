import { untrack } from "svelte";
import { call } from "../api";
import {
  commands,
  type Loadable,
  type LoadOrderResolution,
  type QmEvent,
  type SimulationOverview,
  type SolverDiagnostic,
} from "../bindings";
import { profiles } from "./profiles.svelte";

/** A backend answer, or the error that stopped it (the call already showed a toast). */
export type Checked<T> = Loadable<T> | { status: "error"; message: string };

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

class LoadOrderStore {
  resolution = $state<Checked<LoadOrderResolution>>({ status: "loading" });
  #simulation = $state<Checked<SimulationOverview>>({ status: "loading" });
  #resolutionSeq = 0;
  #simulationSeq = 0;
  /** A resolution request is in flight. */
  #resolving = $state(false);
  /** Writes in flight that change the resolution (e.g. mod_set_enabled). */
  #changing = $state(0);
  /** The last ready simulation and the profile that was active when it landed. */
  #kept = $state<{ profile: string | null; data: SimulationOverview } | null>(null);

  /**
   * The simulation to show. A profile write clears the service's simulation, so while the new
   * one runs this is the last ready one of the same profile: views stay on screen (and keep
   * their state) instead of dropping to a skeleton on every write. Gates use `simulation`.
   */
  get shown(): Checked<SimulationOverview> {
    const sim = this.simulation;
    const kept = this.#kept;
    if (sim.status !== "loading" || !kept || kept.profile !== (profiles.activeSummary?.id ?? null)) return sim;
    return { status: "ready", data: kept.data };
  }

  /** The patch simulation as the service has it now; loading while it runs. */
  get simulation(): Checked<SimulationOverview> {
    return this.#simulation;
  }

  set simulation(v: Checked<SimulationOverview>) {
    this.#simulation = v;
    if (v.status === "ready") this.#kept = { profile: untrack(() => profiles.activeSummary?.id ?? null), data: v.data };
  }

  /** `shown` is the last simulation while a new one runs. */
  get refreshing(): boolean {
    return this.simulation.status === "loading" && this.shown.status === "ready";
  }

  /** Whether `resolution` is loaded and current: false while it loads, refreshes, failed, or a write may change it. */
  get resolutionReady(): boolean {
    return this.resolution.status === "ready" && !this.#resolving && this.#changing === 0;
  }

  /** Error-severity diagnostics of the resolved order; they gate launching. Empty while loading. */
  get errors(): SolverDiagnostic[] {
    if (this.resolution.status !== "ready") return [];
    return this.resolution.data.diagnostics.filter((d) => d.severity === "error");
  }

  /** Loads the resolved order and the patch simulation; each applies independently. */
  async load(): Promise<void> {
    await Promise.all([this.#loadResolution(), this.#loadSimulation()]);
  }

  async #loadResolution(): Promise<void> {
    const seq = ++this.#resolutionSeq;
    this.#resolving = true;
    try {
      const resolution = await call(commands.loadOrderGet());
      if (seq === this.#resolutionSeq) this.resolution = resolution;
    } catch (e) {
      if (seq === this.#resolutionSeq) this.resolution = { status: "error", message: message(e) };
      throw e;
    } finally {
      if (seq === this.#resolutionSeq) this.#resolving = false;
    }
  }

  /** Runs a write that changes the resolution, keeping it not ready until a reload after the write lands. */
  async whileChanging<T>(write: () => Promise<T>): Promise<T> {
    this.#changing++;
    try {
      return await write();
    } finally {
      // Start the reload before dropping the count so the gate never reopens on stale data.
      const reload = this.#loadResolution();
      this.#changing--;
      reload.catch(() => {});
    }
  }

  async #loadSimulation(): Promise<void> {
    const seq = ++this.#simulationSeq;
    try {
      const simulation = await call(commands.simulationGet());
      if (seq === this.#simulationSeq) this.simulation = simulation;
    } catch (e) {
      if (seq === this.#simulationSeq) this.simulation = { status: "error", message: message(e) };
      throw e;
    }
  }

  /** Test-only: forgets the last ready simulation. */
  forget(): void {
    this.#kept = null;
  }

  handle(e: QmEvent): void {
    if (e.type === "invalidated" && e.scope === "load_order") this.load().catch(() => {});
  }
}

export const loadOrder = new LoadOrderStore();
