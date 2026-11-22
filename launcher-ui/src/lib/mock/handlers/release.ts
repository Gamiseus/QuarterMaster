import type { EngineTestPlan, GameStatus, ReleaseInfo, ReleaseZip } from "../../bindings";
import { emitMock } from "../../events";
import { fail, state, type Handlers } from "../state";
import { launchTrial } from "./diagnostics";
import { mockFileText, mockHash, mockWriteFile } from "./engine";

/** Test plans for the mock workspace: Field Radio runs; Squad Settings is off. */
function plan(folder: string): EngineTestPlan {
  const others = [
    { id: "gcore", name: "GCore", installed: true, enabled: true },
    { id: "tactical-hud", name: "Tactical HUD", installed: true, enabled: true },
    { id: "ai-tweaks", name: "AI Tweaks", installed: true, enabled: true },
  ];
  if (folder === "FieldRadio") {
    return {
      mod_id: "field_radio",
      mod_name: "Field Radio",
      in_set: true,
      enabled: !state.init.testOff,
      requires: [{ id: "gmenu", name: "GMenu", installed: true, enabled: true }],
      turned_off: others,
      blocked: state.init.testOff ? "off" : null,
      message: state.init.testOff ? "Field Radio is off in the active profile." : null,
      workspace_mode: false,
    };
  }
  return {
    mod_id: folder.toLowerCase(),
    mod_name: folder,
    in_set: false,
    enabled: false,
    requires: [],
    turned_off: [],
    blocked: "not_in_set",
    message: `The library has no copy of ${folder}. Sync now copies the workspace into the library, or switch to the Workspace side to run the folder itself.`,
    workspace_mode: false,
  };
}

const zips = new Map<string, ReleaseZip>();

function jsonOf(folder: string): Record<string, unknown> {
  try {
    return JSON.parse(mockFileText(folder, "mod.json") ?? "{}") as Record<string, unknown>;
  } catch {
    return {};
  }
}

function next(v: string): string[] {
  const [a, b, c] = v.split(".").map((n) => Number(n) || 0);
  return [`${a}.${b}.${c + 1}`, `${a}.${b + 1}.0`, `${a + 1}.0.0`];
}

function info(folder: string): ReleaseInfo {
  const json = jsonOf(folder);
  const version = typeof json.version === "string" ? json.version : null;
  const toml = mockFileText(folder, "qm.toml") ?? "";
  const tomlVersion = /^version = "([^"]+)"/m.exec(toml)?.[1] ?? null;
  const log = mockFileText(folder, "CHANGELOG.md") ?? "";
  const top = /^## (.+)$/m.exec(log);
  const after = top ? log.slice((top.index ?? 0) + top[0].length) : "";
  const section = after.split(/^## /m)[0];
  return {
    mod_id: typeof json.id === "string" ? json.id : folder,
    name: typeof json.name === "string" ? json.name : folder,
    json_version: version,
    toml_version: tomlVersion,
    next: next(version ?? tomlVersion ?? "0.0.0"),
    repository: folder === "FieldRadio" ? "you/field-radio" : null,
    has_publish: toml.includes("[publish]"),
    has_readme: mockFileText(folder, "README.md") !== undefined,
    changelog_version: top ? top[1].trim() : null,
    changelog_lines: section
      .split("\n")
      .filter((l) => l.startsWith("- "))
      .map((l) => l.slice(2).trim()),
    last_zip: zips.get(folder) ?? null,
    asset_name: `${folder}.zip`,
  };
}

export const handlers: Handlers = {
  engine_test_plan: (args) => plan(String(args.folder)),
  engine_test_launch: (args): GameStatus => {
    const p = plan(String(args.folder));
    if (p.blocked) fail("invalid_input", p.message ?? "This test cannot run yet.");
    const status = launchTrial({ changes: { pause_fixes: [], disable_patches: [], disable_mods: p.turned_off.map((m) => m.id) }, label: `Test of ${p.mod_name}` });
    // The game writes an error from the mod's code while it runs.
    setTimeout(() => {
      emitMock({ type: "log_line", source: "runtime", line: "[QM] Loading Field Radio" });
      emitMock({ type: "log_line", source: "runtime_errors", line: "Error: Mods/field_radio/includes/radio.lua:4: attempt to call a nil value (global 'callArtillery')" });
    }, 50);
    return status;
  },
  release_info: (args) => info(String(args.folder)),
  release_prepare: (args) => {
    const folder = String(args.folder);
    const version = String(args.version).trim();
    if (!/^\d+\.\d+\.\d+$/.test(version)) fail("invalid_input", `'${version}' is not a version: write Major.Minor.Patch, such as 1.2.0.`);
    const current = info(folder);
    const now = current.json_version ?? current.toml_version ?? "0.0.0";
    const num = (v: string) => v.split(".").map(Number).reduce((a, n) => a * 1000 + n, 0);
    if (num(version) <= num(now)) fail("invalid_input", `${version} is not newer than the mod's version now.`);
    const json = mockFileText(folder, "mod.json");
    if (json) mockWriteFile(folder, "mod.json", json.replace(/"version": "[^"]*"/, `"version": "${version}"`));
    const toml = mockFileText(folder, "qm.toml");
    if (toml) mockWriteFile(folder, "qm.toml", toml.replace(/^version = "[^"]*"/m, `version = "${version}"`));
    const lines = (args.lines as string[]).filter((l) => l.trim());
    if (lines.length) {
      const old = mockFileText(folder, "CHANGELOG.md") ?? "# Changelog\n\n";
      const at = old.indexOf("\n## ");
      const section = `## ${version}\n\n${lines.map((l) => `- ${l}`).join("\n")}\n\n`;
      mockWriteFile(folder, "CHANGELOG.md", at < 0 ? `${old.trimEnd()}\n\n${section}` : `${old.slice(0, at + 1)}${section}${old.slice(at + 1)}`);
    }
    return null;
  },
  release_pack: (args) => {
    const folder = String(args.folder);
    if (state.init.packFails) fail("invalid_input", "The check found problems, so nothing was packed: Patch 'Longer radio range' (field_radio:range) had 0 matches on 'includes/radio.lua'.");
    const i = info(folder);
    const zip: ReleaseZip = {
      name: `${folder}.zip`,
      size_bytes: 18_432,
      sha256: mockHash(`${folder}${i.json_version}`).repeat(8),
      version: i.json_version ?? "",
    };
    zips.set(folder, zip);
    return zip;
  },
  release_reveal: () => null,
};
