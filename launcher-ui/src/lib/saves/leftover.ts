import type { LeftoverFields } from "../bindings";

/** The line under a slot's name when its save holds values of mods no profile on it loads. */
export function leftoverNote(l: LeftoverFields): string {
  const n = l.fields.length;
  return `This save has ${n === 1 ? "1 value" : `${n} values`} from ${l.mods.length === 1 ? "a mod" : "mods"} no profile on this slot loads: ${l.mods.join(", ")}.`;
}

export interface LeftoverRow {
  field: string;
  /** The mods the save's stamp lists for the field. */
  mods: string[];
  /** False when more than one mod wrote it: Clean up never removes those. */
  cleanable: boolean;
}

/** One row per leftover value, the removable ones first. */
export function leftoverRows(l: LeftoverFields): LeftoverRow[] {
  const rows = l.fields.map((field) => ({
    field,
    mods: l.owners?.[field] ?? [],
    cleanable: l.cleanable.includes(field),
  }));
  return [...rows.filter((r) => r.cleanable), ...rows.filter((r) => !r.cleanable)];
}

/** The toast after a clean up. */
export function cleanedText(removed: number, slotName: string): string {
  return `Removed ${removed === 1 ? "1 value" : `${removed} values`} from ${slotName}. A backup was made first.`;
}
