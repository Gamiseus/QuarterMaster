import type { LogSource, QmEvent } from "../bindings";

export const LOG_LIMIT = 200;

export interface LogEntry {
  seq: number;
  source: LogSource;
  line: string;
}

class LogsStore {
  lines = $state<LogEntry[]>([]);
  #seq = 0;

  handle(e: QmEvent): void {
    if (e.type !== "log_line") return;
    this.lines.push({ seq: this.#seq++, source: e.source, line: e.line });
    if (this.lines.length > LOG_LIMIT) this.lines.splice(0, this.lines.length - LOG_LIMIT);
  }
}

export const logs = new LogsStore();
