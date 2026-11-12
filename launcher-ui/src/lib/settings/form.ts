import type { DetectedPaths, SettingsPatch, SettingsView } from "../bindings";

export type Theme = "system" | "dark" | "light";

/** The backend accepts only these three; anything else (a legacy or hand-edited value) shows as System. */
export function normalizeTheme(t: string): Theme {
  return t === "dark" || t === "light" ? t : "system";
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

export type OnboardingStep = "detect" | "confirm" | "launch";
const STEPS: OnboardingStep[] = ["detect", "confirm", "launch"];

export function nextStep(s: OnboardingStep): OnboardingStep {
  return STEPS[Math.min(STEPS.length - 1, STEPS.indexOf(s) + 1)];
}

export function prevStep(s: OnboardingStep): OnboardingStep {
  return STEPS[Math.max(0, STEPS.indexOf(s) - 1)];
}
