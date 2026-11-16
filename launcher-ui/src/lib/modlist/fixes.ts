import type { ModSummary, SolverDiagnostic, UpdatesReport } from "../bindings";

export type FixAction =
  | { kind: "enable" | "disable"; ids: string[] }
  | { kind: "update"; modId: string }
  | { kind: "browse"; text: string }
  | { kind: "open"; modId: string; tab: "load-order" };

export interface Fix {
  label: string;
  action: FixAction;
}

/** The one-click fix offered for a solver diagnostic, or null when there is none. */
export function fixFor(d: SolverDiagnostic, mods: ModSummary[], updates: UpdatesReport | null): Fix | null {
  const byId = new Map(mods.map((m) => [m.id, m]));
  const name = (id: string) => byId.get(id)?.name ?? id;
  const related = d.related_mod_id;
  switch (d.kind) {
    case "missing_dependency":
      if (!related) return null;
      // `related` is the `requires` key: an installed mod id, or a name no installed mod provides.
      if (byId.has(related)) return { label: `Enable ${name(related)}`, action: { kind: "enable", ids: [related] } };
      return { label: `Find ${related} in catalog`, action: { kind: "browse", text: related } };
    case "version_mismatch": {
      const update = related ? updates?.mods.find((u) => u.mod_id === related && u.update_available && u.download_url) : undefined;
      if (related && update) return { label: `Update ${name(related)}`, action: { kind: "update", modId: related } };
      return { label: `Disable ${name(d.mod_id)}`, action: { kind: "disable", ids: [d.mod_id] } };
    }
    case "conflict":
      if (!related) return null;
      return { label: `Disable ${name(related)}`, action: { kind: "disable", ids: [related] } };
    case "cycle":
      return { label: "Open load order", action: { kind: "open", modId: d.mod_id, tab: "load-order" } };
    default:
      return null;
  }
}

/** Stable key for a diagnostic row. */
export function diagnosticKey(d: SolverDiagnostic): string {
  return `${d.kind}:${d.mod_id}:${d.related_mod_id ?? ""}`;
}
