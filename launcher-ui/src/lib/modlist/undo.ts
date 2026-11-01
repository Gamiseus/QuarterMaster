/** Bounded undo/redo history of whole-state snapshots. */
export class UndoStack<T> {
  private past: T[] = [];
  private future: T[] = [];
  private readonly limit: number;

  constructor(limit = 50) {
    this.limit = limit;
  }

  /** Record the state as it was before a change; invalidates redo. */
  push(state: T): void {
    this.past.push(state);
    if (this.past.length > this.limit) this.past.shift();
    this.future = [];
  }

  undo(current: T): T | null {
    if (this.past.length === 0) return null;
    this.future.push(current);
    return this.past.pop() as T;
  }

  redo(current: T): T | null {
    if (this.future.length === 0) return null;
    this.past.push(current);
    return this.future.pop() as T;
  }

  get canUndo(): boolean {
    return this.past.length > 0;
  }

  get canRedo(): boolean {
    return this.future.length > 0;
  }

  clear(): void {
    this.past = [];
    this.future = [];
  }
}
