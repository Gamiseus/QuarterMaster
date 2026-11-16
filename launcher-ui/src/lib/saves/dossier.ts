import type { SaveProfile, SaveSummary, SoldierSummary } from "../bindings";

export interface DossierRow {
  label: string;
  value: string;
}

export type GameMode = "campaign" | "classic" | "trench";

export interface ModeSection {
  mode: GameMode;
  title: string;
  rows: DossierRow[];
}

export interface SoldierRow {
  name: string;
  rank: string;
  role: string;
  level: number;
  kills: number;
  /** Award icon ids (`sprites/award<N>.png`). */
  awards: number[];
  group: "Deployed" | "Reserve";
}

/** A slot with no save file yet: the backend sends a blank summary for it. */
export function isEmptySave(s: SaveSummary): boolean {
  return (
    s.day === 0 &&
    s.missions_completed === 0 &&
    s.deployed_squad.length === 0 &&
    s.reserve_soldiers.length === 0 &&
    !hasClassic(s) &&
    !hasTrench(s)
  );
}

const hasCampaign = (s: SaveSummary) =>
  s.day > 0 || s.missions_completed > 0 || s.campaign_experience > 0 || s.campaign_kills > 0;
const hasClassic = (s: SaveSummary) => s.classic_experience > 0 || s.classic_max_wave > 0 || s.classic_total_kills > 0;
const hasTrench = (s: SaveSummary) => s.trench_xp > 0 || s.trench_axis_kills > 0 || s.trench_sectors.length > 0;

const n = (v: number) => v.toLocaleString("en-US");
const row = (label: string, value: string | number): DossierRow => ({ label, value: typeof value === "number" ? n(value) : value });

/**
 * One section per game mode the save has progress in (the save keeps the three apart; the
 * field-to-variable table is in qm-schema's save_parser.rs).
 */
export function modeSections(s: SaveSummary): ModeSection[] {
  const out: ModeSection[] = [];
  if (hasCampaign(s)) {
    const rows = [
      row("Rank", s.campaign_rank_title || String(s.campaign_rank)),
      row("Experience", s.campaign_experience),
      row("Campaign day", s.day),
    ];
    if (s.area_of_operations) rows.push(row("Area of operations", s.area_of_operations));
    if (s.german_commander && s.german_commander !== "Unknown") rows.push(row("German commander", s.german_commander));
    rows.push(
      row("Missions", s.missions_completed),
      row("Win streak", s.win_streak),
      row("Kills", s.campaign_kills),
      row("Casualties", s.campaign_casualties),
      row("Tactical points", s.tactical_points),
      row("Tactical points spent", s.total_tp_spent),
      row("Command points", s.command_points),
      row("Free planes", s.free_planes),
    );
    if (s.active_battle_conditions.length > 0) rows.push(row("Battle conditions", s.active_battle_conditions.join(", ")));
    out.push({ mode: "campaign", title: "Campaign", rows });
  }
  if (hasClassic(s)) {
    out.push({
      mode: "classic",
      title: "Classic",
      rows: [
        row("Rank", s.rank_title || String(s.classic_rank)),
        row("Experience", s.classic_experience),
        row("Best wave", s.classic_max_wave),
        row("Current wave", s.classic_current_wave),
        row("Total kills", s.classic_total_kills),
        row("Total casualties", s.classic_total_casualties),
      ],
    });
  }
  if (hasTrench(s)) {
    const rows = [
      row("Rank", s.trench_rank_title || String(s.trench_rank)),
      row("Experience", s.trench_xp),
      row("Axis kills", s.trench_axis_kills),
      row("Total casualties", s.trench_total_casualties),
      row("Escalation", s.trench_escalation),
    ];
    if (s.trench_sectors.length > 0) rows.push(row("Sectors held", s.trench_sectors.join(", ")));
    if (s.reinforcement_cards.length > 0) rows.push(row("Reinforcement cards", s.reinforcement_cards.length));
    out.push({ mode: "trench", title: "Trench", rows });
  }
  return out;
}

/** Deployed, reserve and fallen counts, and unlocked classes. */
export function squadRows(s: SaveSummary): DossierRow[] {
  const rows = [
    row("Deployed", s.deployed_squad.length),
    row("Reserves", s.reserve_soldiers.length),
    row("Fallen", s.fallen_soldiers_count),
  ];
  if (s.unlocked_classes.length > 0) rows.push(row("Unlocked classes", s.unlocked_classes.join(", ")));
  return rows;
}

const soldier = (x: SoldierSummary, group: SoldierRow["group"]): SoldierRow => ({
  name: x.name,
  rank: x.rank_title || String(x.rank),
  role: x.specialization,
  level: x.level,
  kills: x.kills,
  awards: x.award_icons ?? [],
  group,
});

/** Deployed soldiers, then the reserve, in save order. */
export function soldierRows(s: SaveSummary): SoldierRow[] {
  return [...s.deployed_squad.map((x) => soldier(x, "Deployed")), ...s.reserve_soldiers.map((x) => soldier(x, "Reserve"))];
}

/** A plain-text dossier for export: every mode with progress, the squad, soldiers and mods. */
export function dossierText(s: SaveSummary, profile: Pick<SaveProfile, "name" | "active_mods_fingerprint">): string {
  const lines: string[] = [`${profile.name}: dossier`, ""];
  const block = (title: string, rows: DossierRow[]) => {
    lines.push(title.toUpperCase());
    const w = Math.max(...rows.map((r) => r.label.length));
    for (const r of rows) lines.push(`  ${r.label.padEnd(w)}  ${r.value}`);
    lines.push("");
  };
  const sections = modeSections(s);
  if (sections.length === 0) lines.push("No progress yet.", "");
  for (const sec of sections) block(sec.title, sec.rows);
  block("Squad", squadRows(s));
  const soldiers = soldierRows(s);
  if (soldiers.length > 0) {
    lines.push("SOLDIERS");
    for (const x of soldiers) {
      lines.push(`  ${x.group.padEnd(8)} ${x.name} (${x.rank}${x.role ? `, ${x.role}` : ""}): level ${x.level}, ${x.kills} kills, ${x.awards.length} awards`);
    }
    lines.push("");
  }
  if (s.ribbons.length > 0) lines.push(`RIBBONS`, `  ${s.ribbons.join(", ")}`, "");
  const mods = Object.entries(profile.active_mods_fingerprint ?? {}).filter((e): e is [string, string] => e[1] !== undefined);
  if (mods.length > 0) {
    lines.push("MODS LAST USED");
    for (const [id, v] of mods.sort(([a], [b]) => a.localeCompare(b))) lines.push(`  ${id} ${v}`);
    lines.push("");
  }
  return lines.join("\n").trimEnd() + "\n";
}
