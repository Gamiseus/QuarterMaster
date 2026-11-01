import { call } from "../api";
import { commands, type ModProfile, type ProfileSummary, type QmEvent } from "../bindings";

class ProfilesStore {
  list = $state<ProfileSummary[]>([]);
  loaded = $state(false);
  /** Full active profile (separators, priorities, locks); null until loaded. */
  active = $state<ModProfile | null>(null);
  #seq = 0;
  #activeSeq = 0;

  get activeSummary(): ProfileSummary | null {
    return this.list.find((p) => p.is_active) ?? null;
  }

  async load(): Promise<void> {
    const seq = ++this.#seq;
    const list = await call(commands.profilesList());
    if (seq !== this.#seq) return;
    this.list = list;
    this.loaded = true;
    await this.loadActive();
  }

  /** Reloads only the active profile document. */
  async loadActive(): Promise<void> {
    const seq = ++this.#activeSeq;
    const id = this.activeSummary?.id;
    if (!id) {
      this.active = null;
      return;
    }
    if (this.active?.id !== id) this.active = null; // never show a stale profile for the new id
    const profile = await call(commands.profileGet(id));
    if (seq === this.#activeSeq) this.active = profile;
  }

  handle(e: QmEvent): void {
    if (e.type !== "invalidated") return;
    if (e.scope === "profiles") this.load().catch(() => {});
    else if (e.scope === "load_order") this.loadActive().catch(() => {});
  }
}

export const profiles = new ProfilesStore();
