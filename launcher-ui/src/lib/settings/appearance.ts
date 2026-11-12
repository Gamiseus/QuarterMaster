import type { SettingsView } from "../bindings";
import { COLOUR_VISIONS, type ColourVision } from "./cvd";
import { normalizeTheme } from "./form";
import { applyTextScale, clampScale, textScaleVars } from "./typescale";

/**
 * The last applied appearance, for `public/boot.js` to set before the app mounts (no flash of the
 * wrong theme or size). A cache only: settings.json stays the source of truth.
 */
export const APPEARANCE_KEY = "qm.appearance";

export interface BootAppearance {
  theme: "dark" | "light" | null;
  cvd: Exclude<ColourVision, "standard"> | null;
  contrast: "high" | null;
  vars: Record<string, string>;
}

type AppearanceSettings = Pick<SettingsView, "theme" | "color_vision" | "high_contrast" | "text_scale">;

export function bootAppearance(s: AppearanceSettings): BootAppearance {
  const theme = normalizeTheme(s.theme ?? "system");
  const cvd = COLOUR_VISIONS.includes(s.color_vision as ColourVision) ? (s.color_vision as ColourVision) : "standard";
  return {
    theme: theme === "system" ? null : theme,
    cvd: cvd === "standard" ? null : cvd,
    contrast: s.high_contrast ? "high" : null,
    vars: textScaleVars(clampScale(s.text_scale)),
  };
}

/** Sets theme, colour vision, contrast and text size on the document and caches them for the next start. */
export function applyAppearance(s: AppearanceSettings, root: HTMLElement = document.documentElement): void {
  const a = bootAppearance(s);
  const set = (key: "theme" | "cvd" | "contrast", v: string | null) => {
    if (v === null) delete root.dataset[key];
    else root.dataset[key] = v;
  };
  set("theme", a.theme);
  set("cvd", a.cvd);
  set("contrast", a.contrast);
  applyTextScale(root, clampScale(s.text_scale));
  try {
    localStorage.setItem(APPEARANCE_KEY, JSON.stringify(a));
  } catch {
    // Storage can be unavailable; the next start just applies the settings a moment later.
  }
}
