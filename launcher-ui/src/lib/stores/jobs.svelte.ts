import { call } from "../api";
import { commands, type JobId, type JobInfo, type QmEvent } from "../bindings";
import { toasts } from "./toasts.svelte";

class JobsStore {
  list = $state<JobInfo[]>([]);
  #seq = 0;
  /** Job events seen while a load is in flight; `null` means finished. */
  #since: Map<JobId, JobInfo | null> | null = null;

  async load(): Promise<void> {
    const seq = ++this.#seq;
    const since = new Map<JobId, JobInfo | null>();
    this.#since = since;
    let list: JobInfo[];
    try {
      list = await call(commands.jobsList());
    } finally {
      if (this.#since === since) this.#since = null;
    }
    if (seq !== this.#seq) return;
    const merged = list.filter((j) => !since.has(j.id));
    for (const job of since.values()) if (job) merged.push(job);
    this.list = merged;
  }

  handle(e: QmEvent): void {
    if (e.type === "job_progress") {
      let job = this.list.find((j) => j.id === e.id);
      if (job) {
        job.progress = e.progress;
        job.label = e.label;
      } else {
        job = { id: e.id, kind: e.kind, progress: e.progress, label: e.label, cancelling: false };
        this.list.push(job);
      }
      this.#since?.set(e.id, { ...job });
    } else if (e.type === "job_finished") {
      this.list = this.list.filter((j) => j.id !== e.id);
      this.#since?.set(e.id, null);
      if (e.error && e.error.kind !== "cancelled") toasts.error(e.error.message, e.error.details);
    }
  }
}

export const jobs = new JobsStore();
