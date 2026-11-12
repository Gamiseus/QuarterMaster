import type {
  AssetChoicePreview,
  AssetGroup,
  AssetLedger,
  AssetSlot,
  FactionSideChoice,
  LedgerNote,
  ModProfile,
} from "../bindings";
import { fail, state, type MockInit } from "./state";

const mockInit = (): MockInit => state.init ?? {};

/** The winner name that means the game's own file. */
export const VANILLA = "vanilla";

interface MockClaim {
  source: string;
  file: string;
  /** For a faction: the game global it sets. */
  target?: string;
  size?: [number, number];
}

interface MockSlot {
  key: string;
  name: string;
  kind: AssetSlot["kind"];
  number?: number;
  game?: { path: string; width?: number; height?: number };
  /** In load order: the last one wins without a choice. */
  claims: MockClaim[];
}

interface MockGroup {
  key: string;
  kind: AssetGroup["kind"];
  label: string;
  game_count: number;
  slots: MockSlot[];
}

/**
 * The overlapping assets of `__qmMockInit.assetOverlaps`: two sound mods that share the game's
 * Garand shots, and a sprite both a mod and a faction replace. Mirrors what
 * `assets_ledger::build` gives the real launcher.
 */
const WORLD: MockGroup[] = [
  {
    key: "sounds/garand#",
    kind: "set",
    label: "sounds/garand#",
    game_count: 4,
    slots: [
      {
        key: "sounds/garand1", name: "garand1", kind: "sound", number: 1, game: { path: "sounds/garand1.wav" },
        claims: [
          { source: "soundswap", file: "assets/sounds/garand1.wav" },
          { source: "voicepack", file: "assets/sounds/garand1.wav" },
        ],
      },
      {
        key: "sounds/garand2", name: "garand2", kind: "sound", number: 2, game: { path: "sounds/garand2.wav" },
        claims: [{ source: "soundswap", file: "assets/sounds/garand2.wav" }],
      },
      {
        key: "sounds/garand5", name: "garand5", kind: "sound", number: 5,
        claims: [{ source: "voicepack", file: "assets/sounds/garand5.wav" }],
      },
    ],
  },
  {
    key: "sprites",
    kind: "folder",
    label: "sprites",
    game_count: 1,
    slots: [
      {
        key: "sprites/allied_idle", name: "allied_idle", kind: "image", game: { path: "sprites/allied_idle.png", width: 32, height: 48 },
        claims: [
          { source: "soundswap", file: "assets/sprites/allied_idle.png", size: [64, 48] },
          { source: "faction:example_army", file: "units/base/idle.png", target: "allied_idle", size: [32, 48] },
        ],
      },
    ],
  },
];

const ORDER = ["soundswap", "voicepack", "faction:example_army"];

export function assetsOn(): boolean {
  return Boolean(mockInit().assetOverlaps);
}

/** The profile's running asset choices: key -> winner. */
export function choicesOf(p: ModProfile): Record<string, string> {
  const out: Record<string, string> = {};
  for (const f of p.fixes ?? []) {
    if (!f.paused && f.change.kind === "asset_choice" && !(f.change.key in out)) out[f.change.key] = f.change.winner;
  }
  return out;
}

const show = (source: string) => (source === VANILLA ? "the game's own file" : source);

/** The ledger with `choices`, as `assets_ledger` returns it. */
export function ledgerWith(choices: Record<string, string>): AssetLedger {
  if (!assetsOn()) return { has_game_files: true, groups: [] };
  const groups: AssetGroup[] = WORLD.map((g) => {
    const groupChoice = choices[g.key] ?? null;
    const notes: LedgerNote[] = [];
    const files = new Map<string, number>();
    for (const s of g.slots) for (const c of s.claims) files.set(c.source, (files.get(c.source) ?? 0) + 1);
    if (g.kind === "set") {
      for (const source of ORDER.filter((s) => files.has(s))) {
        const inGame = g.slots.filter((s) => s.game && s.claims.some((c) => c.source === source)).length;
        if (inGame > 0 && inGame < g.game_count) {
          notes.push({
            code: "only_some_of_set",
            text: `${source} has ${inGame} of the game's ${g.game_count}; it has no file for the other ${g.game_count - inGame}.`,
          });
        }
      }
    }
    const slots: AssetSlot[] = g.slots.map((s) => {
      const slotNotes: LedgerNote[] = [];
      if (g.kind === "set" && !s.game && s.number !== undefined) {
        slotNotes.push({
          code: "added_number",
          text: `Number ${s.number} is not one of the game's ${g.game_count}. It is an addition; nothing is renumbered.`,
        });
      }
      for (const c of s.claims) {
        if (s.game?.width && c.size && (c.size[0] !== s.game.width || c.size[1] !== s.game.height)) {
          slotNotes.push({
            code: "asset_size_differs",
            text: `${c.source}'s image is ${c.size[0]}x${c.size[1]}; the game's is ${s.game.width}x${s.game.height}. Code that draws it at fixed offsets may be off.`,
          });
        }
      }
      const fallback = s.claims[s.claims.length - 1]?.source ?? VANILLA;
      const choice = choices[s.key] ?? groupChoice;
      let winner = fallback;
      if (choice === VANILLA) winner = VANILLA;
      else if (choice && s.claims.some((c) => c.source === choice)) winner = choice;
      else if (choice) slotNotes.push({ code: "filled_from", text: `${choice} has no file here, so ${show(fallback)} is used.` });
      return {
        key: s.key,
        name: s.name,
        kind: s.kind,
        ...(s.number !== undefined ? { number: s.number } : {}),
        ...(s.game ? { game: s.game } : {}),
        claims: s.claims.map((c) => ({
          source: c.source,
          file: c.file,
          ...(c.target ? { target: c.target } : {}),
          ...(c.size ? { width: c.size[0], height: c.size[1] } : {}),
        })),
        ...(choices[s.key] ? { choice: choices[s.key] } : {}),
        winner,
        notes: slotNotes,
      };
    });
    return {
      key: g.key,
      kind: g.kind,
      label: g.label,
      game_count: g.game_count,
      claimants: ORDER.filter((s) => files.has(s)).map((source) => ({ source, files: files.get(source) ?? 0 })),
      ...(groupChoice ? { choice: groupChoice } : {}),
      slots,
      notes,
    };
  });
  return { has_game_files: true, groups };
}

function scopeOf(ledger: AssetLedger, key: string): { sources: string[]; slots: AssetSlot[] } | null {
  const g = ledger.groups.find((x) => x.key === key);
  if (g) return { sources: g.claimants.map((c) => c.source), slots: g.slots.filter((s) => !s.choice) };
  const s = ledger.groups.flatMap((x) => x.slots).find((x) => x.key === key);
  return s ? { sources: s.claims.map((c) => c.source), slots: [s] } : null;
}

/** What giving `key` to `winner` leaves out, per source (`assets_ledger::exclusions_of`). */
export function excludesOf(p: ModProfile, key: string, winner: string): Record<string, string[]> {
  const after = ledgerWith({ ...choicesOf(p), [key]: winner });
  const scope = scopeOf(after, key);
  const out: Record<string, string[]> = {};
  for (const s of scope?.slots ?? []) {
    const decided = winner === VANILLA || s.claims.some((c) => c.source === winner);
    if (!decided) continue;
    for (const c of s.claims.filter((x) => x.source !== winner)) {
      (out[c.source] ??= []).push(c.target ?? c.file);
    }
  }
  for (const k of Object.keys(out)) out[k] = [...new Set(out[k])].sort();
  return out;
}

/** Like the service: the key must be in the ledger and the winner one of its sources. */
export function checkChoice(p: ModProfile, key: string, winner: string): void {
  const scope = scopeOf(ledgerWith(choicesOf(p)), key);
  if (!scope) fail("not_found", "No enabled mod or faction claims that file or group any more.");
  if (winner !== VANILLA && !scope.sources.includes(winner)) fail("invalid_input", `${winner} has no file there.`);
}

export function choicePreview(p: ModProfile, key: string, winner: string): AssetChoicePreview {
  checkChoice(p, key, winner);
  const before = ledgerWith(choicesOf(p));
  const after = ledgerWith({ ...choicesOf(p), [key]: winner });
  const was = new Map(before.groups.flatMap((g) => g.slots).map((s) => [s.key, s.winner]));
  const scope = scopeOf(after, key);
  const changes = after.groups
    .flatMap((g) => g.slots)
    .filter((s) => was.get(s.key) !== s.winner)
    .map((s) => ({ slot: s.key, name: s.name, from: was.get(s.key) ?? VANILLA, to: s.winner }));
  const unfilled = winner === VANILLA ? [] : (scope?.slots ?? []).filter((s) => s.winner !== winner).map((s) => s.name);
  return { key, winner, changes, unfilled, leaves_out: excludesOf(p, key, winner) };
}

/** "Use <winner> for <key>", as the service labels the fix. */
export function choiceLabel(key: string, winner: string): string {
  const who = winner === VANILLA ? "the game's own files" : winner.startsWith("faction:") ? `the faction ${winner.slice(8)}` : winner;
  return `Use ${who} for ${key}`;
}

const FACTIONS = [
  { faction_id: "example_army", name: "Example Army", mod_id: "example_mod", priority: 60 },
  { faction_id: "other_army", name: "Other Army", mod_id: "other_mod", priority: 40 },
];

/** The sides two enabled factions replace (`__qmMockInit.twoFactions`). */
export function factionChoices(p: ModProfile): FactionSideChoice[] {
  if (!mockInit().twoFactions) return [];
  const picked = p.faction_picks?.allied ?? null;
  const active = FACTIONS.find((f) => f.faction_id === picked)?.faction_id ?? FACTIONS[0].faction_id;
  return [{ side: "allied", label: "Allies", picked, active, candidates: FACTIONS }];
}

export function pickFaction(p: ModProfile, side: string, id: string | null): void {
  if (side !== "allied" && side !== "axis") fail("invalid_input", "Pick the Allies or the Germans.");
  const picks = { ...(p.faction_picks ?? {}) };
  if (id === null || id.trim() === "") {
    delete picks[side];
  } else {
    const known = side === "allied" && Boolean(mockInit().twoFactions) && FACTIONS.some((f) => f.faction_id === id);
    if (!known) fail("not_found", `No enabled mod has the faction '${id}' for the ${side === "allied" ? "Allies" : "Germans"}.`);
    picks[side] = id;
  }
  p.faction_picks = picks;
}
