import { call } from "../api";
import { commands, type Loadable, type QmEvent, type UpdatesReport } from "../bindings";

class UpdatesStore {
  report = $state<Loadable<UpdatesReport>>({ status: "loading" });
  #seq = 0;

  async load(): Promise<void> {
    const seq = ++this.#seq;
    const report = await call(commands.updatesGet());
    if (seq === this.#seq) this.report = report;
  }

  handle(e: QmEvent): void {
    if (e.type === "invalidated" && e.scope === "updates") this.load().catch(() => {});
  }
}

export const updates = new UpdatesStore();
