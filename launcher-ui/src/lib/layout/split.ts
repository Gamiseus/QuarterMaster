/** Window widths below this get the list alone, with details in an overlay. */
export const NARROW_MAX = 1100;
/** The list column never grows past this (spec 3.3); the default never starts below 480. */
export const LIST_MAX = 760;
export const LIST_DEFAULT_MIN = 480;
/** The smallest width a user can drag the list to. */
export const LIST_MIN = 360;
/** The detail column keeps at least this much. */
export const DETAIL_MIN = 420;
/** The splitter's own width. */
export const SPLITTER_W = 8;

export type WidthClass = "narrow" | "medium" | "wide";

export function widthClass(windowWidth: number): WidthClass {
  if (windowWidth < NARROW_MAX) return "narrow";
  return windowWidth < 1600 ? "medium" : "wide";
}

/** The largest list width that still leaves the detail column its minimum. */
function maxList(areaWidth: number): number {
  return Math.max(LIST_MIN, Math.min(LIST_MAX, areaWidth - DETAIL_MIN - SPLITTER_W));
}

export function clampListWidth(width: number, areaWidth: number): number {
  return Math.round(Math.min(maxList(areaWidth), Math.max(LIST_MIN, width)));
}

/** Half the area, between 480 and 760, and never so wide the detail column drops below its minimum. */
export function defaultListWidth(areaWidth: number): number {
  const preferred = Math.min(LIST_MAX, Math.max(LIST_DEFAULT_MIN, areaWidth * 0.5));
  return clampListWidth(preferred, areaWidth);
}

const key = (cls: WidthClass) => `qm.mods.split.${cls}`;

/** The width the user chose for this window class; a per-viewer convenience, so storage failures are ignored. */
export function loadSplit(cls: WidthClass): number | null {
  try {
    const raw = localStorage.getItem(key(cls));
    const n = raw === null ? Number.NaN : Number(raw);
    return Number.isFinite(n) && n > 0 ? n : null;
  } catch {
    return null;
  }
}

export function saveSplit(cls: WidthClass, width: number | null): void {
  try {
    if (width === null) localStorage.removeItem(key(cls));
    else localStorage.setItem(key(cls), String(Math.round(width)));
  } catch {
    // Private mode or blocked storage: the default is used next time.
  }
}
