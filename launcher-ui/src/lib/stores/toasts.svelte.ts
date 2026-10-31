export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: number;
  tone: "info" | "error";
  message: string;
  details: string | null;
  action?: ToastAction;
}

const INFO_TIMEOUT_MS = 5000;

class ToastStore {
  items = $state<Toast[]>([]);
  #next = 1;

  #push(toast: Omit<Toast, "id">): number {
    const id = this.#next++;
    this.items.push({ id, ...toast });
    return id;
  }

  info(message: string): number {
    const id = this.#push({ tone: "info", message, details: null });
    setTimeout(() => this.dismiss(id), INFO_TIMEOUT_MS);
    return id;
  }

  error(message: string, details: string | null = null, action?: ToastAction): number {
    return this.#push({ tone: "error", message, details, action });
  }

  dismiss(id: number): void {
    this.items = this.items.filter((t) => t.id !== id);
  }
}

export const toasts = new ToastStore();
