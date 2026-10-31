import { call } from "../api";
import { commands, type Loadable, type ModSummary, type QmEvent } from "../bindings";

class ModsStore {
  list = $state<Loadable<ModSummary[]>>({ status: "loading" });
  #seq = 0;

  async load(): Promise<void> {
    const seq = ++this.#seq;
    const list = await call(commands.modsList());
    if (seq === this.#seq) this.list = list;
  }

  handle(e: QmEvent): void {
    if (e.type === "invalidated" && e.scope === "mods") this.load().catch(() => {});
  }
}

export const mods = new ModsStore();
