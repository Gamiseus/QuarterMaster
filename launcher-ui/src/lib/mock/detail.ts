import type { ConfigValue, ModConfigSchema, ModDetail, ModRelationships } from "../bindings";
import { mods, soundSwapFiles } from "./fixtures";
import { MOD_RELATIONS } from "./patchworld";

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
      button_text: null,
      aliases: [],
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
      button_text: null,
      aliases: [],
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
      button_text: null,
      aliases: [],
    },
  ],
  modes: [],
  pages: [],
  keybinds: [],
};

/** Saved option values per mod, like the profile's mod settings file. */
export const savedConfig: Record<string, Record<string, ConfigValue>> = {};

export function resetConfig(id: string): Record<string, ConfigValue> {
  delete savedConfig[id];
  return {};
}

/** A readme long enough to need scrolling, so layout tests see a real long detail panel (Vehicle Pack). */
const LONG_README = [
  "# Vehicle Pack",
  "",
  "Adds a motor pool to the campaign: jeeps, half-tracks and a light tank, each with its own upkeep and crew rules.",
  "",
  ...Array.from({ length: 12 }, (_, i) =>
    [
      `## Vehicle ${i + 1}`,
      "",
      "Crews need a driver and a gunner. Vehicles take fuel from the supply line each day and stall when it runs out. Mines and anti-tank guns damage them; mechanics repair them between missions.",
      "",
      "- Speed: medium",
      "- Armour: light",
      "- Crew: 2",
      "",
    ].join("\n"),
  ),
].join("\n");

/** The manifest's relationships, from the relations the mock's stacking reads. */
function relationsOf(id: string): ModRelationships {
  const r = MOD_RELATIONS[id];
  if (!r || (r.requires.length === 0 && r.load_after.length === 0 && r.load_before.length === 0)) return {};
  return {
    requires: Object.fromEntries(r.requires.map((m) => [m, ">=1.0.0"])),
    ...(r.load_after.length > 0 ? { load_after: r.load_after } : {}),
    ...(r.load_before.length > 0 ? { load_before: r.load_before } : {}),
  };
}

export function detailFor(id: string, enabled: boolean): ModDetail | null {
  const summary = mods.find((m) => m.id === id);
  if (!summary) return null;
  if (summary.kind === "assets") {
    return {
      summary: { ...summary, enabled },
      manifest: { id, name: summary.name, version: summary.version, authors: summary.authors, category: summary.category },
      readme_md: null,
      changelog_md: null,
      config_schema: null,
      saved_config: {},
      security_report: { capabilities: [], findings: [], has_high_risk: false },
      integrity: { is_pristine: true, modified: [], missing: [], untracked: [] },
      asset_files: soundSwapFiles,
    };
  }
  return {
    summary: { ...summary, enabled },
    manifest: {
      id,
      name: summary.name,
      version: summary.version,
      authors: summary.authors,
      description: `${summary.name} for Mud and Blood.`,
      github: "Gamiseus/MnB-Mods",
      relationships: relationsOf(id),
    },
    readme_md: id === "vehicle-pack" ? LONG_README : `# ${summary.name}\n\nInstall with QuarterMaster. See the [project page](https://github.com/Gamiseus/MnB-Mods).\n\n![Screenshot](https://example.com/shot.png)`,
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
    asset_files: [],
  };
}
