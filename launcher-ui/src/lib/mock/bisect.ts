import type { BisectCandidate, BisectEnd, BisectTrial, BisectView, NextTrial, ProfileFix, RunEntry, RunOutcome, TrialChanges, TrialResult, TrialSuspect } from "../bindings";
import { readTest } from "../diagnostics/safelaunch";

/** Mirrors `qm-service/src/bisect.rs` (D68 step 2); the messages are the service's. */
export const MSG_TWO_UNKNOWN = "Two tests could not tell whether the problem happened.";
export const MSG_NONE_LEFT = "The problem came back without each of them, so none of them is the cause.";
export const MSG_ALL_BREAK = "Every test that is left would stop a file from compiling.";
export const NOTE_RESTART = "The search for the cause ended when QuarterMaster closed. Start it again if the problem is still there.";
export const MSG_NOTHING =
  "There is nothing to test: the crash names no fix, patch or mod, and no fix changed since the last run without problems.";

interface Pending {
  step: number;
  leftOut: number[];
  runId: string | null;
}

/** Trials a halving search of `n` items takes. */
export function halvings(n: number): number {
  let steps = 0;
  for (let left = n; left > 1; left = Math.ceil(left / 2)) steps++;
  return steps;
}

const same = (a: number[], b: number[]) => a.length === b.length && a.every((x, i) => x === b[i]);

function pushNew(to: string[], from: string[]): void {
  for (const x of from) if (!to.includes(x)) to.push(x);
}

export class MockBisect {
  remaining: number[];
  confirmed = false;
  trials: BisectTrial[] = [];
  skipped: number[][] = [];
  unknowns = 0;
  running: Pending | null = null;
  ask: Pending | null = null;
  end: BisectEnd | null = null;
  message: string | null = null;
  backup: [string, string] | null = null;

  constructor(
    readonly id: string,
    readonly profileId: string,
    readonly runId: string,
    readonly candidates: BisectCandidate[],
    /** How the crashed run failed: a test is read against it. */
    readonly origin: RunOutcome = "lua_error",
  ) {
    this.remaining = candidates.map((_, i) => i);
    this.settle();
  }

  options(): number[][] {
    const r = this.remaining;
    if (r.length === 0) return [];
    if (r.length === 1) return this.confirmed ? [] : [[...r]];
    const half = Math.ceil(r.length / 2);
    const out = [r.slice(0, half), r.slice(half)];
    for (const i of r) if (!out.some((o) => same(o, [i]))) out.push([i]);
    return out;
  }

  nextSplit(): number[] | null {
    return this.options().find((o) => !this.skipped.some((s) => same(s, o))) ?? null;
  }

  step(): number {
    return this.trials.length + 1;
  }

  changesFor(leftOut: number[]): TrialChanges {
    const c: TrialChanges = { pause_fixes: [], disable_patches: [], disable_mods: [] };
    for (const i of leftOut) {
      const s = this.candidates[i]?.suspect.changes;
      if (!s) continue;
      pushNew(c.pause_fixes, s.pause_fixes);
      pushNew(c.disable_patches, s.disable_patches);
      pushNew(c.disable_mods, s.disable_mods);
    }
    return c;
  }

  labelFor(leftOut: number[]): string {
    const names = leftOut.flatMap((i) => (this.candidates[i] ? [this.candidates[i].suspect.label] : []));
    if (names.length <= 1) return names[0] ?? "";
    if (names.length === 2) return `${names[0]} and ${names[1]}`;
    if (names.length === 3) return `${names[0]}, ${names[1]} and ${names[2]}`;
    return `${names[0]}, ${names[1]} and ${names.length - 2} more`;
  }

  get open(): boolean {
    return this.end === null;
  }

  get idle(): boolean {
    return this.running === null && this.ask === null;
  }

  record(step: number, leftOut: number[], runId: string | null, result: TrialResult): void {
    if (result === "fine") {
      this.remaining = [...leftOut];
      this.confirmed = true;
    } else if (result === "crashed") {
      this.remaining = this.remaining.filter((i) => !leftOut.includes(i));
    } else {
      this.unknowns++;
    }
    this.trials.push({ step, left_out: [...leftOut], run_id: runId, result });
    this.settle();
  }

  skip(leftOut: number[]): void {
    if (!this.skipped.some((s) => same(s, leftOut))) this.skipped.push(leftOut);
    this.settle();
  }

  finish(end: BisectEnd, message: string | null): void {
    this.end = end;
    this.message = message;
    this.running = null;
    this.ask = null;
  }

  settle(): void {
    if (!this.open || !this.idle) return;
    if (this.unknowns >= 2) this.finish("could_not_tell", MSG_TWO_UNKNOWN);
    else if (this.remaining.length === 0) this.finish("not_found", MSG_NONE_LEFT);
    else if (this.remaining.length === 1 && this.confirmed) this.finish("found", null);
    else if (this.nextSplit() === null) this.finish("could_not_tell", MSG_ALL_BREAK);
  }

  nextTrial(step: number, leftOut: number[]): NextTrial {
    return { step, left_out: [...leftOut], label: this.labelFor(leftOut), changes: this.changesFor(leftOut) };
  }

  /** The service's hook after a test run finished. */
  afterRun(run: RunEntry): void {
    const t = run.trial;
    if (!t || !this.open || t.bisect_id !== this.id || this.running?.step !== t.step) return;
    const p = { ...this.running, runId: run.run_id };
    this.running = null;
    if (!this.backup && t.save_slot_id && t.save_backup_id) this.backup = [t.save_slot_id, t.save_backup_id];
    const reading = readTest(run, this.origin);
    const result: TrialResult | null =
      reading !== "ask" ? reading : run.outcome === "unknown" && run.message !== null ? "unknown" : null;
    if (result) this.record(p.step, p.leftOut, p.runId, result);
    else this.ask = p;
  }

  view(): BisectView {
    const pending = this.running ?? this.ask;
    let next: NextTrial | null = null;
    if (pending) next = this.nextTrial(pending.step, pending.leftOut);
    else if (this.open) {
      const s = this.nextSplit();
      if (s) next = this.nextTrial(this.step(), s);
    }
    const left = !this.open ? 0 : this.remaining.length === 1 ? 1 : halvings(this.remaining.length);
    return {
      id: this.id,
      profile_id: this.profileId,
      run_id: this.runId,
      candidates: structuredClone(this.candidates),
      remaining: [...this.remaining],
      trials: structuredClone(this.trials),
      next,
      expected_trials: this.trials.length + left,
      running: this.running !== null,
      ask: this.ask !== null,
      skipped: this.skipped.length,
      end: this.end,
      found: this.end === "found" && this.remaining.length === 1 ? this.remaining[0] : null,
      message: this.message,
      save_slot_id: this.backup?.[0] ?? null,
      save_backup_id: this.backup?.[1] ?? null,
    };
  }
}

/** When a fix was last created, changed or resumed (ms; 0 when unknown). */
function changedAt(f: ProfileFix): number {
  const kinds = ["created", "pinned", "rebound", "accepted", "resumed"];
  const times = [f.provenance.created_at, ...(f.provenance.history ?? []).filter((e) => kinds.includes(e.what)).map((e) => e.at)];
  return Math.max(0, ...times.map((t) => Date.parse(t.replace(/\[.*\]$/, ""))).filter((n) => !Number.isNaN(n)));
}

function cleared(s: TrialSuspect, c: TrialChanges): boolean {
  const list = s.kind === "fix" ? c.pause_fixes : s.kind === "patch" ? c.disable_patches : c.disable_mods;
  return list.some((x) => x.toLowerCase() === s.id.toLowerCase());
}

export function fixSuspect(f: ProfileFix): TrialSuspect {
  return { kind: "fix", id: f.id, label: `the fix "${f.label}"`, name: f.label, changes: { pause_fixes: [f.id], disable_patches: [], disable_mods: [] } };
}

/** The candidates in §7.3 order, as `bisect::candidates` builds them. */
export function candidatesFor(input: {
  suspect: TrialSuspect | null;
  fixes: ProfileFix[];
  lastClean: RunEntry | null;
  filePatches: TrialSuspect[];
  cleared: TrialChanges;
}): BisectCandidate[] {
  const out: BisectCandidate[] = [];
  const add = (s: TrialSuspect, source: BisectCandidate["source"]) => {
    const seen = out.some((c) => c.suspect.kind === s.kind && c.suspect.id.toLowerCase() === s.id.toLowerCase());
    if (!seen && !cleared(s, input.cleared)) out.push({ suspect: s, source });
  };
  if (input.suspect) add(input.suspect, "suspect");
  const since = input.lastClean ? Date.parse(input.lastClean.started_at.replace(/\[.*\]$/, "")) : null;
  const recent = input.fixes
    .filter((f) => !f.paused && f.change.kind !== "mod_order")
    .filter((f) => {
      const clean = input.lastClean;
      if (!clean) return true;
      return !clean.active_fixes.includes(f.id) || (since !== null && !Number.isNaN(since) && changedAt(f) > since);
    })
    .map((f) => ({ f, at: changedAt(f) }))
    .sort((a, b) => b.at - a.at);
  for (const { f } of recent) add(fixSuspect(f), "recent_fix");
  for (const s of input.filePatches) add(s, "crash_file");
  return out;
}
