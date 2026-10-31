import { call } from "../api";
import { commands, type ProfileSummary, type QmEvent } from "../bindings";

class ProfilesStore {
  list = $state<ProfileSummary[]>([]);
  loaded = $state(false);
  #seq = 0;

  get active(): ProfileSummary | null {
    return this.list.find((p) => p.is_active) ?? null;
  }

  async load(): Promise<void> {
    const seq = ++this.#seq;
    const list = await call(commands.profilesList());
    if (seq !== this.#seq) return;
    this.list = list;
    this.loaded = true;
  }

  handle(e: QmEvent): void {
    if (e.type === "invalidated" && e.scope === "profiles") this.load().catch(() => {});
  }
}

export const profiles = new ProfilesStore();
