import type { DetectedPaths, SettingsPatch, SettingsView } from "../bindings";

/** Every theme, in the order Settings lists them. `system` follows Windows between Field dark and Field light. */
export const THEMES = [
  { value: "system", label: "System", scheme: null },
  { value: "dark", label: "Field dark", scheme: "dark" },
  { value: "light", label: "Field light", scheme: "light" },
  { value: "graphite", label: "Graphite", scheme: "dark" },
  { value: "black", label: "Black", scheme: "dark" },
  { value: "midnight", label: "Midnight", scheme: "dark" },
  { value: "paper", label: "Paper", scheme: "light" },
  { value: "desert", label: "Desert", scheme: "light" },
] as const;
export type Theme = (typeof THEMES)[number]["value"];

/** The backend accepts only these; anything else (a legacy or hand-edited value) shows as System. */
export function normalizeTheme(t: string): Theme {
  return THEMES.find((x) => x.value === t)?.value ?? "system";
}

/** Auto-backups to keep: a whole number from 1 to 100. */
export function clampBackups(n: number): number {
  if (!Number.isFinite(n)) return 1;
  return Math.min(100, Math.max(1, Math.round(n)));
}

const PATH_KEYS = ["game_exe_path", "save_dir_path", "workspace_mods_dir"] as const;
export type PathKey = (typeof PATH_KEYS)[number];

/** The detected paths worth saving: found (non-null) and different from what is set. */
export function pathsPatch(detected: Pick<DetectedPaths, PathKey>, current: SettingsView): Partial<SettingsPatch> {
  const patch: Partial<SettingsPatch> = {};
  for (const key of PATH_KEYS) {
    const value = detected[key];
    if (value !== null && value !== current[key]) patch[key] = value;
  }
  return patch;
}

export type OnboardingStep = "game" | "workspace" | "launch";
const STEPS: OnboardingStep[] = ["game", "workspace", "launch"];

export function nextStep(s: OnboardingStep): OnboardingStep {
  return STEPS[Math.min(STEPS.length - 1, STEPS.indexOf(s) + 1)];
}

export function prevStep(s: OnboardingStep): OnboardingStep {
  return STEPS[Math.max(0, STEPS.indexOf(s) - 1)];
}
