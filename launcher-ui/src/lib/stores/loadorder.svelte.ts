import { call } from "../api";
import {
  commands,
  type Loadable,
  type LoadOrderResolution,
  type QmEvent,
  type SimulationOverview,
  type SolverDiagnostic,
} from "../bindings";

class LoadOrderStore {
  resolution = $state<Loadable<LoadOrderResolution>>({ status: "loading" });
  simulation = $state<Loadable<SimulationOverview>>({ status: "loading" });
  #resolutionSeq = 0;
  #simulationSeq = 0;
  /** A resolution request is in flight. */
  #resolving = $state(false);
  /** Writes in flight that change the resolution (e.g. mod_set_enabled). */
  #changing = $state(0);

  /** Whether `resolution` is loaded and current: false while it loads, refreshes or a write may change it. */
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
    const simulation = await call(commands.simulationGet());
    if (seq === this.#simulationSeq) this.simulation = simulation;
  }

  handle(e: QmEvent): void {
    if (e.type === "invalidated" && e.scope === "load_order") this.load().catch(() => {});
  }
}

export const loadOrder = new LoadOrderStore();
