import { call } from "../api";
import { commands, type QmEvent, type SaveSlotSummary } from "../bindings";

/** Save slot list; the shell uses it to name the active profile's linked slot. */
class SavesStore {
  list = $state<SaveSlotSummary[] | null>(null);
  #seq = 0;

  async load(): Promise<void> {
    const seq = ++this.#seq;
    const list = await call(commands.savesList());
    if (seq === this.#seq) this.list = list;
  }

  /** Display name for a slot id; falls back to the id until the list is loaded. */
  nameFor(id: string): string {
    return this.list?.find((s) => s.profile.id === id)?.profile.name ?? id;
  }

  handle(e: QmEvent): void {
    if (e.type === "invalidated" && e.scope === "saves") this.load().catch(() => {});
  }
}

export const saves = new SavesStore();
