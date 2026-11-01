import type { ConfigValue, ModConfigSchema, ModDetail } from "../bindings";
import { mods } from "./fixtures";

const gmenuSchema: ModConfigSchema = {
  mod_id: "gmenu",
  name: "GMenu",
  options: [
    {
      id: "show_hints",
      label: "Show hints",
      option_type: "checkbox",
      default: true,
      description: "Shows key hints at the bottom of menus.",
      options: [],
      labels: [],
      min: null,
      max: null,
      step: null,
      show_if: null,
    },
    {
      id: "menu_scale",
      label: "Menu scale",
      option_type: "slider",
      default: 1,
      description: null,
      options: [],
      labels: [],
      min: 0.5,
      max: 2,
      step: 0.1,
      show_if: null,
    },
    {
      id: "theme",
      label: "Theme",
      option_type: "cycle",
      default: "olive",
      description: null,
      options: ["olive", "sand"],
      labels: ["Olive", "Sand"],
      min: null,
      max: null,
      step: null,
      show_if: null,
    },
  ],
};

/** Saved option values per mod, like the profile's mod settings file. */
export const savedConfig: Record<string, Record<string, ConfigValue>> = {};

export function resetConfig(id: string): Record<string, ConfigValue> {
  delete savedConfig[id];
  return {};
}

export function detailFor(id: string, enabled: boolean): ModDetail | null {
  const summary = mods.find((m) => m.id === id);
  if (!summary) return null;
  return {
    summary: { ...summary, enabled },
    manifest: {
      id,
      name: summary.name,
      version: summary.version,
      authors: summary.authors,
      description: `${summary.name} for Mud and Blood.`,
      github: "Gamiseus/MnB-Mods",
      relationships: id === "gcore" ? {} : { requires: { gcore: ">=1.0.0" }, load_after: ["gcore"] },
    },
    readme_md: `# ${summary.name}\n\nInstall with QuarterMaster. See the [project page](https://github.com/Gamiseus/MnB-Mods).\n\n![Screenshot](https://example.com/shot.png)`,
    changelog_md: `## ${summary.version}\n\n- Fixes and polish.`,
    config_schema: id === "gmenu" ? gmenuSchema : null,
    saved_config: { ...(savedConfig[id] ?? {}) },
    security_report:
      id === "ai-tweaks"
        ? {
            capabilities: ["dynamic_code_execution"],
            findings: [
              {
                capability: "dynamic_code_execution",
                severity: "low",
                file: "ai.lua",
                line: 12,
                pattern: "loadstring",
                snippet: "local f = loadstring(src)",
              },
            ],
            has_high_risk: false,
          }
        : { capabilities: [], findings: [], has_high_risk: false },
    integrity: { is_pristine: true, modified: [], missing: [], untracked: [] },
  };
}
