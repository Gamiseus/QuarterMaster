export type SelectMode = "replace" | "toggle" | "range";

class SelectionStore {
  selected = $state<Set<string>>(new Set());
  /** Last clicked row; the anchor for range selection. */
  focused = $state<string | null>(null);

  select(id: string, mode: SelectMode, order: string[]): void {
    if (mode === "toggle") {
      const next = new Set(this.selected);
      if (!next.delete(id)) next.add(id);
      this.selected = next;
      this.focused = id;
    } else {
      const from = mode === "range" && this.focused !== null ? order.indexOf(this.focused) : -1;
      const to = order.indexOf(id);
      if (from >= 0 && to >= 0) {
        const [lo, hi] = from <= to ? [from, to] : [to, from];
        this.selected = new Set(order.slice(lo, hi + 1));
      } else {
        this.selected = new Set([id]);
        this.focused = id;
      }
    }
  }

  clear(): void {
    this.selected = new Set();
    this.focused = null;
  }
}

export const selection = new SelectionStore();
