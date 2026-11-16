import type { WindowSettings } from "../bindings";

/** Every value left to the game. */
export const EMPTY_WINDOW: WindowSettings = {
  vsync: null,
  resizable: null,
  width: null,
  height: null,
  borderless: null,
  display: null,
  fullscreen: null,
  fullscreen_type: null,
};

export function windowIsEmpty(w: WindowSettings): boolean {
  return Object.values(w).every((v) => v === null || v === undefined);
}

/** A whole positive number from a text field; null for an empty or unusable one. */
export function parseWindowNumber(text: string): number | null {
  const n = Number(text.trim());
  return text.trim() !== "" && Number.isInteger(n) && n > 0 ? n : null;
}

/** Whether two sets of window values are the same (a missing value is the game's, like null). */
export function sameWindow(a: WindowSettings, b: WindowSettings): boolean {
  return (Object.keys(EMPTY_WINDOW) as (keyof WindowSettings)[]).every((k) => (a[k] ?? null) === (b[k] ?? null));
}
