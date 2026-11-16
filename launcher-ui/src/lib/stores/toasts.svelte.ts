import type { JobId } from "../bindings";

export interface ToastAction {
  label: string;
  run: () => void;
}

interface ToastBase {
  message: string;
  details: string | null;
  action?: ToastAction;
  /** More buttons after `action`. An info toast with buttons stays until dismissed. */
  actions?: ToastAction[];
  /** Closes the toast after this long, also when it has buttons. */
  closeAfterMs?: number;
}

/** A toast before it has an id; `progress` toasts follow a running job. */
export type ToastInit = ToastBase & ({ tone: "info" | "error" } | { tone: "progress"; jobId: JobId });

export type Toast = ToastInit & { id: number };

const INFO_TIMEOUT_MS = 5000;

class ToastStore {
  items = $state<Toast[]>([]);
  #next = 1;
  /** One auto-dismiss timer per toast id; replaced or cleared so a stale one never closes a newer toast. */
  #timers = new Map<number, ReturnType<typeof setTimeout>>();

  #push(toast: ToastInit): number {
    const id = this.#next++;
    this.items.push({ ...toast, id });
    this.#expire(id, toast);
    return id;
  }

  #expire(id: number, toast: ToastInit): void {
    this.#clearTimer(id);
    const sticky = toast.action !== undefined || (toast.actions?.length ?? 0) > 0;
    const after = toast.closeAfterMs ?? (toast.tone === "info" && !sticky ? INFO_TIMEOUT_MS : null);
    if (after !== null) this.#timers.set(id, setTimeout(() => this.dismiss(id), after));
  }

  #clearTimer(id: number): void {
    const t = this.#timers.get(id);
    if (t !== undefined) clearTimeout(t);
    this.#timers.delete(id);
  }

  info(message: string): number {
    return this.#push({ tone: "info", message, details: null });
  }

  /**
   * An info toast with buttons. It stays until a button or Dismiss closes it, unless
   * `closeAfterMs` is given: then it also closes by itself after that long.
   */
  notice(message: string, actions: ToastAction[], closeAfterMs?: number): number {
    return this.#push({ tone: "info", message, details: null, actions, ...(closeAfterMs ? { closeAfterMs } : {}) });
  }

  error(message: string, details: string | null = null, action?: ToastAction): number {
    return this.#push({ tone: "error", message, details, action });
  }

  /** A toast whose bar follows the job's live progress; it stays until replaced or dismissed. */
  progress(jobId: JobId, label: string): number {
    return this.#push({ tone: "progress", jobId, message: label, details: null });
  }

  /** Swaps toast `id` in place, keeping its id; pushes a new one if it was dismissed. */
  replace(id: number, toast: ToastInit): number {
    const i = this.items.findIndex((t) => t.id === id);
    if (i < 0) return this.#push(toast);
    this.items[i] = { ...toast, id };
    this.#expire(id, toast);
    return id;
  }

  dismiss(id: number): void {
    this.#clearTimer(id);
    this.items = this.items.filter((t) => t.id !== id);
  }
}

export const toasts = new ToastStore();
