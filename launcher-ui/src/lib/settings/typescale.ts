/**
 * Text size. Small text grows most and large headings least, so a larger scale stays readable
 * without blowing up the layout: a base size `f` at scale `s` becomes
 * `f * (1 + (s - 1) * (14 / f) ** 0.6)`, rounded to 0.5 px.
 */

export const TEXT_SCALE_MIN = 1;
export const TEXT_SCALE_MAX = 1.75;
export const TEXT_SCALE_STEP = 0.125;

/** The `--qm-text-*` tokens at 100% (tokens.css holds the same values as its defaults). */
export const BASE_TEXT: Record<string, number> = {
  "--qm-text-11": 11,
  "--qm-text-12": 12,
  "--qm-text-13": 13,
  "--qm-text-14": 14,
  "--qm-text-16": 16,
  "--qm-text-20": 20,
  "--qm-text-28": 28,
};

/** Control sizes at 100%; they grow by `controlScale` so controls keep fitting their labels. */
export const BASE_CONTROLS: Record<string, number> = {
  "--qm-row-h": 34,
  "--qm-control-box": 18,
  "--qm-switch-w": 36,
  "--qm-switch-h": 20,
  "--qm-switch-thumb": 14,
  "--qm-kbd-min": 20,
  "--qm-icon-size": 16,
};

const half = (n: number) => Math.round(n * 2) / 2;

export function clampScale(s: number): number {
  if (!Number.isFinite(s)) return 1;
  const stepped = Math.round((s - 1) / TEXT_SCALE_STEP) * TEXT_SCALE_STEP + 1;
  return Math.min(TEXT_SCALE_MAX, Math.max(TEXT_SCALE_MIN, stepped));
}

export function scaledSize(f: number, s: number): number {
  return half(f * (1 + (s - 1) * (14 / f) ** 0.6));
}

export function controlScale(s: number): number {
  return Math.sqrt(1 + (s - 1) * 0.8);
}

/** Every text and control token for scale `s`, as CSS values. At 100% the map is empty (tokens.css applies). */
export function textScaleVars(s: number): Record<string, string> {
  const scale = clampScale(s);
  if (scale === 1) return {};
  const k = controlScale(scale);
  const vars: Record<string, string> = {};
  for (const [name, f] of Object.entries(BASE_TEXT)) vars[name] = `${scaledSize(f, scale)}px`;
  for (const [name, f] of Object.entries(BASE_CONTROLS)) vars[name] = `${half(f * k)}px`;
  return vars;
}

/** Writes the tokens for scale `s` on `root`, removing them at 100%. */
export function applyTextScale(root: HTMLElement, s: number): void {
  const vars = textScaleVars(s);
  for (const name of [...Object.keys(BASE_TEXT), ...Object.keys(BASE_CONTROLS)]) {
    if (vars[name]) root.style.setProperty(name, vars[name]);
    else root.style.removeProperty(name);
  }
}
