import type { CatalogEntry } from "../bindings";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** `Live` (the Nexus row) and unparseable values pass through; dates read like `12 Oct 2026` (UTC). */
export function formatUpdated(iso: string): string {
  if (iso === "Live") return iso;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()}`;
}

export function formatLicense(spdx: string | null): string {
  return !spdx || spdx === "NOASSERTION" ? "No license" : spdx;
}

/** Every topic of `entries`, unique and sorted. */
export function topicOptions(entries: Pick<CatalogEntry, "topics">[]): string[] {
  return [...new Set(entries.flatMap((e) => e.topics))].sort();
}
