import type { ModSummary } from "../bindings";
import type { OrderRow } from "./order";
import type { ModStatus } from "./status";

export type FilterChip = "enabled" | "disabled" | "conflicts" | "updates";

/** Text matches name or id (case-insensitive); chips AND together; separators always stay. */
export function filterRows(
  rows: OrderRow[],
  mods: Map<string, ModSummary>,
  status: Map<string, ModStatus>,
  text: string,
  chips: Set<FilterChip>,
): OrderRow[] {
  const needle = text.trim().toLowerCase();
  return rows.filter((row) => {
    if (row.kind === "separator") return true;
    const mod = mods.get(row.id);
    if (needle) {
      const hay = `${mod?.name ?? ""}\n${row.id}`.toLowerCase();
      if (!hay.includes(needle)) return false;
    }
    if (chips.size === 0) return true;
    const st = status.get(row.id);
    if (chips.has("enabled") && !mod?.enabled) return false;
    if (chips.has("disabled") && (mod === undefined || mod.enabled)) return false;
    if (chips.has("conflicts") && !((st?.conflicts ?? 0) > 0)) return false;
    if (chips.has("updates") && !st?.updateAvailable) return false;
    return true;
  });
}
