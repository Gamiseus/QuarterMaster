import type { SaveComparison } from "../bindings";

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Backend timestamps are zoned (`2026-11-06T10:15:30-05:00[America/New_York]`); shown in local time.
 * Anything unparseable passes through.
 */
export function formatTimestamp(ts: string): string {
  const d = new Date(ts.replace(/\[[^\]]*\]$/, ""));
  if (Number.isNaN(d.getTime())) return ts;
  return d.toLocaleString(undefined, { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
}

export function signed(n: number): string {
  return n > 0 ? `+${n}` : String(n);
}

export interface MetricRow {
  label: string;
  a: number;
  b: number;
  delta: number;
}

/** The compare dialog's metric table, `b` relative to `a`. */
export function metricRows(c: SaveComparison): MetricRow[] {
  return [
    { label: "Campaign day", a: c.day_a, b: c.day_b, delta: c.day_delta },
    { label: "Rank", a: c.rank_a, b: c.rank_b, delta: c.rank_delta },
    { label: "Experience", a: c.experience_a, b: c.experience_b, delta: c.experience_delta },
    { label: "Kills", a: c.campaign_kills_a, b: c.campaign_kills_b, delta: c.campaign_kills_delta },
    { label: "Casualties", a: c.campaign_casualties_a, b: c.campaign_casualties_b, delta: c.campaign_casualties_delta },
    { label: "Tactical points", a: c.tp_a, b: c.tp_b, delta: c.tp_delta },
    { label: "Classic best wave", a: c.classic_max_wave_a, b: c.classic_max_wave_b, delta: c.classic_max_wave_delta },
  ];
}
