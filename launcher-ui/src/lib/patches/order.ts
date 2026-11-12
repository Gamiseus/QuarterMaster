import type { OrderRule } from "../bindings";

/** One patch in a file's order view. */
export type OrderItem = {
  id: string;
  mod: string;
  name: string;
  priority: number;
  /** The author's `before` / `after` names (patch ids, `mod:id` or `mod:*`). */
  before: string[];
  after: string[];
};

/** An author rule between two patches in the list: `first` must run before `then`. */
export type AuthorEdge = { first: string; then: string; setBy: string };

/** Whether rule name `name` refers to `it` (the engine's forms: id, `mod:id`, `mod:*`). */
export function refersTo(name: string, it: OrderItem): boolean {
  return name === it.id || name === `${it.mod}:${it.id}` || name === `${it.mod}:*`;
}

export function authorEdges(list: OrderItem[]): AuthorEdge[] {
  const out: AuthorEdge[] = [];
  for (const p of list) {
    for (const q of list) {
      if (p === q) continue;
      if (p.before.some((n) => refersTo(n, q))) out.push({ first: p.id, then: q.id, setBy: p.mod });
      if (p.after.some((n) => refersTo(n, q))) out.push({ first: q.id, then: p.id, setBy: p.mod });
    }
  }
  return out;
}

function moved<T>(list: T[], from: number, to: number): T[] {
  const out = [...list];
  const [x] = out.splice(from, 1);
  out.splice(to, 0, x);
  return out;
}

/**
 * The one profile rule that puts the patch at `from` where `to` is: before the patch now at `to`
 * when moving up, after it when moving down. Null when nothing moves.
 */
export function moveToRule(list: OrderItem[], from: number, to: number): OrderRule | null {
  if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length) return null;
  const x = list[from];
  const target = list[to];
  return to < from ? { patch: x.id, before: [target.id], after: [] } : { patch: x.id, before: [], after: [target.id] };
}

/** Author rules the move would break (they hold now and would not after it). */
export function crossedEdges(list: OrderItem[], from: number, to: number): AuthorEdge[] {
  const next = moved(list, from, to).map((x) => x.id);
  const idx = (ids: string[], id: string) => ids.indexOf(id);
  const now = list.map((x) => x.id);
  return authorEdges(list).filter(
    (e) => idx(now, e.first) < idx(now, e.then) && idx(next, e.first) > idx(next, e.then),
  );
}
