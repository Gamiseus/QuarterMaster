import type { AssetChoicePreview, AssetGroup, AssetSlot, FixDraft } from "../bindings";

/** The winner name that means the game's own file. */
export const VANILLA = "vanilla";

/** A source as the user reads it: a mod's name, "the faction X", or the game. */
export function sourceName(source: string, modName: (id: string) => string): string {
  if (source === VANILLA) return "The game";
  if (source.startsWith("faction:")) return `Faction ${source.slice("faction:".length)}`;
  return modName(source);
}

/** A group's heading: a numbered set reads "sounds/garand 1-4", a folder is its path. */
export function groupTitle(g: AssetGroup): string {
  if (g.kind !== "set") return g.label || "(top folder)";
  const base = g.label.replace(/#$/, "");
  return g.game_count > 0 ? `${base} 1-${g.game_count}` : `${base} (numbered)`;
}

/** "SoundSwap 2 of 4, VoicePack 1 of 4": how much of the group each source has. */
export function groupSummary(g: AssetGroup, modName: (id: string) => string): string {
  const of = g.kind === "set" && g.game_count > 0 ? ` of ${g.game_count}` : "";
  return g.claimants
    .map((c) => {
      const inGame = g.kind === "set" && g.game_count > 0 ? g.slots.filter((s) => s.game && s.claims.some((x) => x.source === c.source)).length : c.files;
      const extra = c.files - inGame;
      return `${sourceName(c.source, modName)} ${inGame}${of}${extra > 0 && of ? `, plus ${extra} more` : ""}`;
    })
    .join("; ");
}

/** The picker's items for a group or a slot: "Last loaded wins" (no choice), each source, the game. */
export function winnerItems(
  sources: string[],
  hasGameFile: boolean,
  modName: (id: string) => string,
): { value: string; label: string }[] {
  return [
    { value: "", label: "Last loaded wins" },
    ...sources.map((s) => ({ value: s, label: sourceName(s, modName) })),
    ...(hasGameFile ? [{ value: VANILLA, label: "The game's own files" }] : []),
  ];
}

/** Who wins every slot of the group now, when it is one source ("mixed" otherwise). */
export function groupWinner(g: AssetGroup): string | null {
  const winners = new Set(g.slots.map((s) => s.winner));
  return winners.size === 1 ? [...winners][0] : null;
}

/** "garand1: SoundSwap, then VoicePack" - a slot's sources in load order. */
export function slotSources(s: AssetSlot, modName: (id: string) => string): string {
  return s.claims.map((c) => sourceName(c.source, modName)).join(", then ");
}

/** The fix draft for a choice. */
export function choiceDraft(key: string, winner: string): FixDraft {
  return { change: { kind: "asset_choice", key, winner }, label: "", source: "resolver", conflict_id: null, option_key: null, approved: null };
}

/** The lines of the confirm dialog: what changes hands, what stays, and what is left out. */
export function previewLines(p: AssetChoicePreview, modName: (id: string) => string): string[] {
  const out: string[] = [];
  if (p.changes.length === 0) out.push("No file changes hands: this only records the choice.");
  for (const c of p.changes) {
    out.push(`${c.name}: ${sourceName(c.to, modName)} instead of ${sourceName(c.from, modName).replace(/^The /, "the ")}.`);
  }
  if (p.unfilled.length > 0) {
    const who = sourceName(p.winner, modName);
    out.push(`${who} has no file for ${p.unfilled.join(", ")}; ${p.unfilled.length === 1 ? "it stays" : "they stay"} as now.`);
  }
  for (const [source, files] of Object.entries(p.leaves_out ?? {})) {
    const n = (files ?? []).length;
    out.push(`${sourceName(source, modName)} leaves out ${n === 1 ? "1 file" : `${n} files`}.`);
  }
  return out;
}
