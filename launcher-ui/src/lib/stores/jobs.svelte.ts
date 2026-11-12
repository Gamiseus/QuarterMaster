import { call, QmCommandError } from "../api";
import { commands, type JobId, type JobInfo, type JobKind, type QmError, type QmEvent } from "../bindings";
import { game } from "./game.svelte";
import { toasts } from "./toasts.svelte";

/** Kinds the UI always tracks with `track`, so `handle` never toasts their errors. */
const TRACKED_KINDS: ReadonlySet<JobKind> = new Set<JobKind>([
  "mod_import",
  "catalog_install",
  "mod_update_install",
  "diagnostic_bundle",
  "updates_refresh",
  "catalog_refresh",
  "workspace_sync",
  "modpack_export",
  "modpack_import",
]);

/** How many finished jobs are remembered for late `whenFinished` callers. */
const REMEMBERED = 50;

interface Finish {
  output: string | null;
  error: QmError | null;
}

interface Waiter {
  resolve: (output: string | null) => void;
  reject: (err: QmCommandError) => void;
}

export interface TrackOptions {
  label: string;
  /** Message for the info toast shown when the job succeeds. */
  success: (output: string | null) => string;
  /** Offered as Retry when the job fails with a network error. */
  retry?: () => unknown;
}

function settle(w: Waiter, f: Finish): void {
  if (f.error) w.reject(new QmCommandError(f.error));
  else w.resolve(f.output);
}

class JobsStore {
  list = $state<JobInfo[]>([]);
  #seq = 0;
  /** Job events seen while a load is in flight; `null` means finished. */
  #since: Map<JobId, JobInfo | null> | null = null;
  #finished = new Map<JobId, Finish>();
  #waiters = new Map<JobId, Waiter[]>();
  #tracked = new Set<JobId>();

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

  /** Resolves with the job's output, or rejects with its error; remembers recent finishes. */
  whenFinished(id: JobId): Promise<string | null> {
    return new Promise((resolve, reject) => {
      const waiter = { resolve, reject };
      const done = this.#finished.get(id);
      if (done) return settle(waiter, done);
      const list = this.#waiters.get(id) ?? [];
      list.push(waiter);
      this.#waiters.set(id, list);
    });
  }

  /** Shows a progress toast for job `id`, then a success or error toast. Rejects like `whenFinished`. */
  async track(id: JobId, opts: TrackOptions): Promise<string | null> {
    this.#tracked.add(id);
    const toast = toasts.progress(id, opts.label);
    try {
      const output = await this.whenFinished(id);
      toasts.replace(toast, { tone: "info", message: opts.success(output), details: null });
      return output;
    } catch (e) {
      const err = e as QmCommandError;
      if (err.kind === "game_running") {
        toasts.dismiss(toast);
        game.banner = err.message;
      } else if (err.kind === "cancelled") {
        toasts.dismiss(toast);
      } else {
        const retry = err.kind === "network" ? opts.retry : undefined;
        toasts.replace(toast, {
          tone: "error",
          message: err.message,
          details: err.details,
          action: retry ? { label: "Retry", run: () => void retry() } : undefined,
        });
      }
      throw err;
    } finally {
      this.#tracked.delete(id);
    }
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
      const silent = this.#tracked.has(e.id) || TRACKED_KINDS.has(e.kind);
      this.#remember(e.id, { output: e.output, error: e.error });
      if (e.error && e.error.kind !== "cancelled" && !silent) toasts.error(e.error.message, e.error.details);
    }
  }

  #remember(id: JobId, finish: Finish): void {
    this.#finished.set(id, finish);
    if (this.#finished.size > REMEMBERED) {
      const oldest = this.#finished.keys().next().value;
      if (oldest !== undefined) this.#finished.delete(oldest);
    }
    const waiters = this.#waiters.get(id);
    this.#waiters.delete(id);
    for (const w of waiters ?? []) settle(w, finish);
  }
}

export const jobs = new JobsStore();
