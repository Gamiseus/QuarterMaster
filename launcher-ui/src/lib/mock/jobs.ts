import type { JobId, JobKind, QmError } from "../bindings";
import { emitMock } from "../events";
import { jobs } from "./state";

let nextId = 1;

/**
 * Simulates a backend job: `job_progress` at 0.3 and 0.7, then `job_finished`, over about 300 ms.
 * `finish` runs just before `job_finished` and returns the job's output or error.
 */
export function runMockJob(kind: JobKind, label: string, finish: () => { output?: string; error?: QmError }): JobId {
  const id = nextId++;
  jobs.push({ id, kind, progress: 0, label, cancelling: false });
  const step = (progress: number) => {
    const job = jobs.find((j) => j.id === id);
    if (job) job.progress = progress;
    emitMock({ type: "job_progress", id, kind, progress, label });
  };
  setTimeout(() => step(0.3), 100);
  setTimeout(() => step(0.7), 200);
  setTimeout(() => {
    let result: { output?: string; error?: QmError };
    try {
      result = finish();
    } catch (e) {
      // A handler bug (a plain Error) must still look like a backend error, not a kindless object.
      const isQmError = typeof e === "object" && e !== null && "kind" in e && "message" in e;
      result = { error: isQmError ? (e as QmError) : { kind: "internal", message: String(e), details: null } };
    }
    const i = jobs.findIndex((j) => j.id === id);
    if (i >= 0) jobs.splice(i, 1);
    emitMock({ type: "job_finished", id, kind, output: result.output ?? null, error: result.error ?? null });
  }, 300);
  return id;
}
