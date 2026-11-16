import type { CatalogEntry } from "../bindings";

/** What installing a catalog entry adds (the manifest inside its download). */
export interface CatalogFixture {
  entry: CatalogEntry;
  manifest: { id: string; version: string };
}

function github(
  repo: string,
  name: string,
  description: string,
  stars: number,
  updated: string,
  topics: string[],
  license: string | null,
  manifest: { id: string; version: string },
  asset: string | null,
): CatalogFixture {
  const [owner] = repo.split("/");
  const tag = `v${manifest.version}`;
  return {
    manifest,
    entry: {
      id: repo.replace("/", "_"),
      name,
      repo_full_name: repo,
      owner,
      description,
      stargazers_count: stars,
      forks_count: 0,
      updated_at: updated,
      html_url: `https://github.com/${repo}`,
      topics,
      license,
      source_type: "GitHub",
      latest_release_tag: asset ? tag : null,
      download_url: asset ? `https://github.com/${repo}/releases/download/${tag}/${asset}` : null,
      download_asset_name: asset,
      installed: false,
      installed_version: null,
      listing: "registered",
    update_key: `GitHub:${repo}`,
    },
  };
}

/** Four GitHub entries (Tactical HUD is installed in the mod fixture) and the Nexus row. */
export const catalogFixture: CatalogFixture[] = [
  github(
    "Gamiseus/tactical-hud",
    "Tactical HUD",
    "Minimap, squad status and objective markers.",
    48,
    "2026-10-12T09:00:00Z",
    ["hud", "interface"],
    "MIT",
    { id: "tactical-hud", version: "2.0.1" },
    "TacticalHUD.zip",
  ),
  github(
    "fieldcraft/hud-compass",
    "HUD Compass",
    "A compass strip with bearings to the nearest objective.",
    12,
    "2026-09-28T17:30:00Z",
    ["hud", "interface"],
    "Apache-2.0",
    { id: "hud-compass", version: "1.4.0" },
    "HudCompass.zip",
  ),
  github(
    "quartermaster-mods/field-rations",
    "Field Rations",
    "Supply drops include rations that restore morale.",
    7,
    "2026-08-03T08:15:00Z",
    ["gameplay"],
    "NOASSERTION",
    { id: "field-rations", version: "0.2.0" },
    "FieldRations.zip",
  ),
  github(
    "nightwatch/night-ops",
    "Night Ops",
    "Night missions with limited visibility. No release yet.",
    3,
    "2026-07-19T21:00:00Z",
    ["content", "missions"],
    null,
    { id: "night-ops", version: "0.1.0" },
    null,
  ),
  {
    manifest: { id: "nexus", version: "0" },
    entry: {
      id: "nexus_mudandblood",
      name: "Mud and Blood on Nexus Mods",
      repo_full_name: "NexusMods:mudandblood",
      owner: "NexusMods",
      description: "Official Mud and Blood modding community hub on Nexus Mods.",
      stargazers_count: 0,
      forks_count: 0,
      updated_at: "Live",
      html_url: "https://www.nexusmods.com/mudandblood/mods",
      topics: ["nexusmods", "mud-and-blood"],
      license: null,
      source_type: "NexusMods",
      latest_release_tag: null,
      download_url: null,
      download_asset_name: null,
      installed: false,
      installed_version: null,
      listing: "registered",
    update_key: "Nexus:mudandblood",
    },
  },
];
