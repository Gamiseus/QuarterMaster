// Texts of the batch-step undo dialog (D71, §9.5).
import type { JournalUndoChoice, JournalUndoPreview, PreviewImpact } from "../bindings";
import { factionChanges, outcomeChanges } from "./impact";

/** `Undo "<label>"?`, with the label's own quotes dropped. */
export function journalTitle(label: string): string {
  return `Undo "${label.replaceAll('"', "")}"?`;
}

/** What the undo does, in one sentence. */
export function journalBody(op: string): string {
  if (op === "compat_export") return "Your fixes come back and the compatibility patch is turned off. It stays in your mod library.";
  if (op === "migrated") return "Your fixes go back to how they were before the import. The import can be tried again later.";
  return "The step is undone.";
}

/** The toast once the undo is done. */
export function journalDone(op: string): string {
  if (op === "compat_export") return "Your fixes are back. The compatibility patch is turned off.";
  return "The step is undone.";
}

/** The two ways on when newer fixes clash, in the order the dialog lists them. */
export const JOURNAL_CHOICES: { value: JournalUndoChoice; label: string }[] = [
  { value: "pause_newer", label: "Pause the newer fixes and bring yours back" },
  { value: "restore_paused", label: "Bring yours back paused" },
];

/** The preview of `choice`, or of the plain undo when nothing clashes. */
export function choiceImpact(p: JournalUndoPreview, choice: JournalUndoChoice | null): PreviewImpact | null {
  if (p.clashes.length === 0) return p.impact_keep;
  if (choice === "pause_newer") return p.impact_pause;
  if (choice === "restore_paused") return p.impact_keep;
  return null;
}

/**
 * A summary of an impact, for the option's accessible description: everything the notes under
 * the option show (ImpactNotes), the faction warning (D73) included, since the notes are hidden
 * from screen readers.
 */
export function impactSummary(
  impact: PreviewImpact | null,
  modName: (modId: string) => string = (id) => id,
  name: (patchId: string) => string = (id) => id,
): string {
  if (!impact) return "The game code check is not ready yet.";
  const parts: string[] = [];
  if (impact.breaks_file) parts.push(`Would stop ${impact.compile_errors_new.join(", ")} from compiling.`);
  const factions = factionChanges(impact, modName);
  if (factions.length > 0) parts.push(`${factions.join("; ")}. Switching factions resets that side's unlocks in your save.`);
  const outcomes = outcomeChanges(impact, name);
  const stacks = (impact.stack_changes ?? []).length;
  const fixed = impact.compile_errors_fixed;
  const n = impact.files_changed.length;
  if (!impact.breaks_file && factions.length === 0 && outcomes.length === 0 && stacks === 0 && fixed.length === 0 && n === 0) {
    return "The game code does not change.";
  }
  if (outcomes.length > 0) parts.push(`Also: ${outcomes.join("; ")}.`);
  if (fixed.length > 0) parts.push(`Makes ${fixed.join(", ")} compile again.`);
  if (n > 0) parts.push(`Changes ${n === 1 ? "1 file" : `${n} files`}: ${impact.files_changed.join(", ")}.`);
  if (stacks > 0) parts.push(`Changes the order at ${stacks === 1 ? "1 shared line" : `${stacks} shared lines`}.`);
  return parts.join(" ");
}

/** Undo can run: the preview is back, and a choice is made when newer fixes clash. */
export function canUndo(p: JournalUndoPreview | null, failed: boolean, choice: JournalUndoChoice | null): boolean {
  if (failed) return true;
  if (!p) return false;
  return p.clashes.length === 0 || choice !== null;
}
