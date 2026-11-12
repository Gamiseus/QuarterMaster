import { untrack } from "svelte";
import { call } from "../api";
import { commands, type StackChange, type StackMode } from "../bindings";
import { loadOrder } from "./loadorder.svelte";
import { profiles } from "./profiles.svelte";

/** The one-time stacking notice's key in `ModProfile.notices_seen` (D3). */
export const NOTICE_STACKING = "stacking_v1";

export type StackChanges = { status: "loading" } | { status: "ready"; data: StackChange[] } | { status: "error" };

/**
 * The active profile's stacking mode and the one-time notice (D3): a profile from before stacking
 * keeps the older order until the user picks a mode. While the notice is unseen, the store
 * previews dependency order once per simulation, so the notice can say what it changes.
 */
class StackingStore {
  #changes = $state<StackChanges | null>(null);
  #key: unknown = null;
  /** The profile `#changes` is about. */
  #profile = $state<string | null>(null);
  #pending: Promise<void> = Promise.resolve();
  #seq = 0;

  /** The mode the game uses: a profile with no mode keeps the older order. */
  get mode(): StackMode {
    return profiles.active?.stacking ?? "legacy";
  }

  /** The active profile has not seen the stacking notice. The Vanilla profile has nothing to tell. */
  get unseen(): boolean {
    const p = profiles.active;
    const summary = profiles.activeSummary;
    if (!p || !summary || summary.is_builtin || p.id !== summary.id) return false;
    return !(p.notices_seen ?? []).includes(NOTICE_STACKING);
  }

  /** What dependency order would change in the active profile; null until checked. */
  get changes(): StackChanges | null {
    return this.#profile === (profiles.active?.id ?? null) ? this.#changes : null;
  }

  /** How many shared lines dependency order changes; null while unknown. */
  get count(): number | null {
    return this.changes?.status === "ready" ? this.changes.data.length : null;
  }

  /** The Patches tab shows the notice: unseen, and dependency order changes something (or the check failed). */
  get showNotice(): boolean {
    const c = this.changes;
    return this.unseen && c !== null && (c.status === "error" || (c.status === "ready" && c.data.length > 0));
  }

  /** The pre-launch dialog shows the notice: unseen, unless dependency order is known to change nothing. */
  get noticeBeforeLaunch(): boolean {
    return this.unseen && !(this.changes?.status === "ready" && this.changes.data.length === 0);
  }

  /** Previews dependency order for the notice, once per simulation, while the notice is unseen. */
  ensure(): Promise<void> {
    const sim = loadOrder.simulation;
    const id = profiles.active?.id ?? null;
    // Another profile's answer never stands in for this one's, even while its simulation runs.
    if (id !== untrack(() => this.#profile)) {
      this.#seq++;
      this.#key = null;
      this.#pending = Promise.resolve();
      this.#changes = null;
      this.#profile = id;
    }
    if (!this.unseen || sim.status !== "ready") return Promise.resolve();
    if (sim.data === this.#key) return this.#pending;
    this.#key = sim.data;
    const seq = ++this.#seq;
    // A refresh of the same profile keeps the last answer until the new one lands.
    if (untrack(() => this.#changes?.status) !== "ready") this.#changes = { status: "loading" };
    this.#pending = call(commands.simulationPreview({ stacking: "dependencies" }), { expect: ["cancelled", "conflict"] })
      .then((r) => {
        if (seq === this.#seq) this.#changes = { status: "ready", data: r.impact.stack_changes ?? [] };
      })
      .catch(() => {
        if (seq === this.#seq) {
          this.#changes = { status: "error" };
          this.#key = null;
        }
      });
    return this.#pending;
  }

  /** Sets the profile's mode; the notice counts as seen. */
  async set(mode: StackMode): Promise<void> {
    await call(commands.stackingSet(mode));
    this.#seen(mode);
  }

  /** Marks the notice seen and keeps the older order. */
  async dismiss(): Promise<void> {
    await call(commands.noticeDismiss(NOTICE_STACKING));
    this.#seen(null);
  }

  /** Shows the write at once; the profile reload that follows confirms it. */
  #seen(mode: StackMode | null): void {
    const p = profiles.active;
    if (!p) return;
    const seen = [...new Set([...(p.notices_seen ?? []), NOTICE_STACKING])];
    profiles.active = { ...p, stacking: mode ?? p.stacking ?? null, notices_seen: seen };
  }

  /** Test-only. */
  reset(): void {
    this.#changes = null;
    this.#key = null;
    this.#profile = null;
    this.#pending = Promise.resolve();
    this.#seq++;
  }
}

export const stacking = new StackingStore();
