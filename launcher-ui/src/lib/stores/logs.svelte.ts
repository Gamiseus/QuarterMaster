import { call } from "../api";
import { commands, type LogSource, type QmEvent } from "../bindings";

/** Lines kept per source. */
export const LOG_CAP = 1000;
/** Lines read from the log file when a source is first shown. */
const SEED_LINES = 500;
const SOURCES: LogSource[] = ["runtime", "runtime_errors", "launcher"];
/** The game logs restart with the profile that launches. */
const RUNTIME: LogSource[] = ["runtime", "runtime_errors"];

export interface LogEntry {
  seq: number;
  line: string;
}

const empty = (): Record<LogSource, LogEntry[]> => ({ runtime: [], runtime_errors: [], launcher: [] });

/** One ring of recent lines per log source: seeded from the file, then fed by `log_line` events. */
class LogsStore {
  lines = $state<Record<LogSource, LogEntry[]>>(empty());
  #seq = 0;
  /** Live lines that arrived while a seed was being read; appended after it. */
  #buffers = new Map<LogSource, LogEntry[]>();
  #loadSeq = new Map<LogSource, number>();
  #loaded = new Set<LogSource>();

  #entry(line: string): LogEntry {
    return { seq: this.#seq++, line };
  }

  #set(source: LogSource, entries: LogEntry[]): void {
    this.lines[source] = entries.length > LOG_CAP ? entries.slice(entries.length - LOG_CAP) : entries;
  }

  /** Seeds `source` from its file; events that arrive meanwhile are kept and appended. */
  async load(source: LogSource): Promise<void> {
    const seq = (this.#loadSeq.get(source) ?? 0) + 1;
    this.#loadSeq.set(source, seq);
    this.#buffers.set(source, []);
    this.#loaded.add(source);
    let tail: string[] | null = null;
    try {
      tail = await call(commands.logsTail(source, SEED_LINES));
    } finally {
      if (this.#loadSeq.get(source) === seq) {
        const buffered = this.#buffers.get(source) ?? [];
        this.#buffers.delete(source);
        // A failed seed keeps the current lines and the live ones that arrived meanwhile.
        const base = tail === null ? this.lines[source] : tail.map((l) => this.#entry(l));
        this.#set(source, [...base, ...buffered]);
      }
    }
  }

  /** Loads `source` the first time it is shown. */
  ensureLoaded(source: LogSource): void {
    if (!this.#loaded.has(source)) this.load(source).catch(() => {});
  }

  clear(source: LogSource): void {
    this.lines[source] = [];
  }

  handle(e: QmEvent): void {
    if (e.type === "log_line") {
      const entry = this.#entry(e.line);
      const buffer = this.#buffers.get(e.source);
      if (buffer) buffer.push(entry);
      else this.#set(e.source, [...this.lines[e.source], entry]);
    } else if (e.type === "invalidated" && e.scope === "profiles") {
      for (const s of RUNTIME) if (this.#loaded.has(s)) this.load(s).catch(() => {});
    }
  }

  /** Test-only: empty every source. */
  reset(): void {
    this.lines = empty();
    this.#buffers.clear();
    this.#loaded.clear();
    for (const s of SOURCES) this.#loadSeq.set(s, (this.#loadSeq.get(s) ?? 0) + 1);
  }
}

export const logs = new LogsStore();
