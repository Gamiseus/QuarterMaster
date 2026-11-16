// Root-cause groups and inferred edges (G10): patches that fail because code they anchor on did
// not apply fold under that code's patch, and the Order view draws every anchor on another
// patch's code as a dashed edge.
import type { DetectedConflict, FailureGroup, FailureMember, InferredEdge } from "../bindings";

/** Conflicts that say a patch did not apply: these fold into a group. */
const FAILURE_KINDS = new Set(["broken_anchor_order", "broken_anchor_replaced", "patch_aborts_file"]);

/** "2 patches fail because Tactical HUD's Minimap is turned off." */
export function groupText(g: FailureGroup, modName: (id: string) => string): string {
  const n = g.members.length;
  const why = g.reason === "turned_off" ? "is turned off" : g.reason === "skipped" ? "is skipped" : "no longer applies";
  return `${n === 1 ? "1 patch fails" : `${n} patches fail`} because ${modName(g.root_mod)}'s ${g.root_name} ${why}.`;
}

export type GroupMemberRow = { member: FailureMember; conflicts: DetectedConflict[] };

/** A conflicts list row: one conflict, or a group with its root's and members' conflicts folded in. */
export type ConflictRow =
  | { kind: "one"; conflict: DetectedConflict }
  | { kind: "group"; group: FailureGroup; root: DetectedConflict[]; members: GroupMemberRow[] };

/**
 * The conflicts list with each group's failures folded into one row, placed where its first
 * conflict was. A group with no conflict in the list adds no row.
 */
export function foldConflicts(conflicts: DetectedConflict[], groups: FailureGroup[]): ConflictRow[] {
  const key = (file: string, patch: string) => `${file}\n${patch}`;
  const owner = new Map<string, FailureGroup>();
  for (const g of groups) {
    owner.set(key(g.target_file, g.root), g);
    for (const m of g.members) owner.set(key(g.target_file, m.patch_id), g);
  }
  const rows: ConflictRow[] = [];
  const placed = new Map<FailureGroup, Extract<ConflictRow, { kind: "group" }>>();
  for (const c of conflicts) {
    const g = FAILURE_KINDS.has(c.kind) ? owner.get(key(c.target_file, c.patch_a_id)) : undefined;
    if (!g) {
      rows.push({ kind: "one", conflict: c });
      continue;
    }
    let row = placed.get(g);
    if (!row) {
      row = { kind: "group", group: g, root: [], members: g.members.map((member) => ({ member, conflicts: [] })) };
      placed.set(g, row);
      rows.push(row);
    }
    if (c.patch_a_id === g.root) row.root.push(c);
    else row.members.find((m) => m.member.patch_id === c.patch_a_id)?.conflicts.push(c);
  }
  return rows;
}

/** "Inferred: Compass uses Minimap's line" */
export function edgeText(user: string, source: string): string {
  return `Inferred: ${user} uses ${source}'s line`;
}

/** An inferred edge between two rows of the Order view: `user` anchors on `source`'s code. */
export type EdgeLane = { user: number; source: number; lane: number; text: string };

/** What one lane draws in one row. */
export type LaneSegment = "none" | "top" | "bottom" | "through";

/**
 * The file's inferred edges between the rows `ids` (top to bottom), each in a lane no other
 * edge it overlaps uses. `name` gives a row's patch name for the tooltip.
 */
export function edgeLanes(ids: string[], edges: InferredEdge[], name: (id: string) => string): EdgeLane[] {
  const out: EdgeLane[] = [];
  const spans = edges
    .map((e) => ({ e, user: ids.indexOf(e.patch_id), source: ids.indexOf(e.uses_patch_id) }))
    .filter((s) => s.user !== -1 && s.source !== -1 && s.user !== s.source)
    .sort((a, b) => Math.min(a.user, a.source) - Math.min(b.user, b.source) || Math.max(a.user, a.source) - Math.max(b.user, b.source));
  for (const s of spans) {
    if (out.some((o) => o.user === s.user && o.source === s.source)) continue;
    const lo = Math.min(s.user, s.source);
    const hi = Math.max(s.user, s.source);
    let lane = 0;
    while (out.some((o) => o.lane === lane && Math.min(o.user, o.source) <= hi && lo <= Math.max(o.user, o.source))) lane++;
    out.push({ user: s.user, source: s.source, lane, text: edgeText(name(s.e.patch_id), name(s.e.uses_patch_id)) });
  }
  return out;
}

/** Per lane, what row `row` draws: an edge starts below its middle (top), ends above it (bottom) or passes. */
export function laneSegments(lanes: EdgeLane[], row: number): LaneSegment[] {
  const count = lanes.reduce((n, l) => Math.max(n, l.lane + 1), 0);
  const out: LaneSegment[] = Array.from({ length: count }, () => "none");
  for (const l of lanes) {
    const lo = Math.min(l.user, l.source);
    const hi = Math.max(l.user, l.source);
    if (row === lo) out[l.lane] = "top";
    else if (row === hi) out[l.lane] = "bottom";
    else if (row > lo && row < hi && out[l.lane] === "none") out[l.lane] = "through";
  }
  return out;
}
