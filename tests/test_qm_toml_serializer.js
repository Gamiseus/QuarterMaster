#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const assert = require('assert');
const vm = require('vm');

console.log('[Test] Running Smart qm.toml Category Block Serializer Test Suite...');

const jsPath = path.join(__dirname, '../ui/app.js');
const jsCode = fs.readFileSync(jsPath, 'utf8');

const sandbox = {
  console,
  document: {
    getElementById: () => null,
    querySelector: () => null,
    querySelectorAll: () => [],
    addEventListener: () => {}
  },
  showToast: () => {},
  escapeHtml: (s) => String(s)
};
vm.createContext(sandbox);

// 1. Verify function declarations exist in app.js
assert(jsCode.includes('function serializePatchToToml'), 'serializePatchToToml must be defined');
assert(jsCode.includes('function insertOrUpdatePatchInQmToml'), 'insertOrUpdatePatchInQmToml must be defined');

vm.runInContext(jsCode, sandbox);

// 2. Test serializePatchToToml
const samplePatch = {
  id: "CustomTerrainGenerator",
  target: "includes/mapBuild.lua",
  pattern: "for x = 1, mapWidth do\n    for y = 1, mapHeight do",
  position: "at",
  matchIndent: true,
  requires: ["GCore-InitGridHook"],
  description: "Custom biome terrain generator",
  payload: "print('[Mod] Building custom biomes')",
  category: "# BATTLEFIELD & MAP BUILD"
};

const tomlString = sandbox.serializePatchToToml(samplePatch);
assert(tomlString.includes('[[patches]]'), 'Must have [[patches]] header');
assert(tomlString.includes('[patches.pattern]'), 'Must have [patches.pattern] header');
assert(tomlString.includes('id = "CustomTerrainGenerator"'), 'Must have id');
assert(tomlString.includes('target = "includes/mapBuild.lua"'), 'Must have target');
assert(tomlString.includes("position = \"at\""), 'Must have position');
assert(tomlString.includes('match_indent = true'), 'Must have match_indent');
assert(tomlString.includes('requires = ["GCore-InitGridHook"]'), 'Must have requires array');
assert(tomlString.includes("'''"), 'Multiline strings must use triple single quotes');

// 3. Test insertOrUpdatePatchInQmToml: inserting into existing category
const existingWithCategory = `# ==============================================================================
# BATTLEFIELD & MAP BUILD
# ==============================================================================

[[patches]]
[patches.pattern]
id = "ExistingMapPatch"
target = "includes/mapBuild.lua"
pattern = 'function oldMap()'
position = "after"
payload = 'print("old")'

# ==============================================================================
# AUDIO & SFX
# ==============================================================================
`;

const updated1 = sandbox.insertOrUpdatePatchInQmToml(existingWithCategory, samplePatch);
assert(updated1.includes('ExistingMapPatch'), 'Existing patch must be preserved');
assert(updated1.includes('CustomTerrainGenerator'), 'New patch must be inserted');
// Must be placed before the AUDIO & SFX category
const customIdx = updated1.indexOf('CustomTerrainGenerator');
const audioIdx = updated1.indexOf('AUDIO & SFX');
assert(customIdx < audioIdx, 'New patch must be placed inside BATTLEFIELD category before next category');

// 4. Test insertOrUpdatePatchInQmToml: creating new category
const existingNoCategory = `# GCore Mod Configuration
[mod]
id = "GCore"
version = "1.0.0"
`;

const updated2 = sandbox.insertOrUpdatePatchInQmToml(existingNoCategory, samplePatch);
assert(updated2.includes('GCore Mod Configuration'), 'Original headers preserved');
assert(updated2.includes('# BATTLEFIELD & MAP BUILD'), 'New category header created');
assert(updated2.includes('CustomTerrainGenerator'), 'Patch serialized in new category');

// 5. Test insertOrUpdatePatchInQmToml: updating existing patch in place
const modifiedPatch = {
  ...samplePatch,
  payload: "print('[Mod] UPDATED custom biomes logic')"
};
const updated3 = sandbox.insertOrUpdatePatchInQmToml(updated1, modifiedPatch);
const matches = updated3.match(/id = "CustomTerrainGenerator"/g);
assert.strictEqual(matches.length, 1, 'Should update existing patch in place, not duplicate');
assert(updated3.includes('UPDATED custom biomes logic'), 'Payload should be updated');

// 6. Validate result with QuarterMaster TOML linter
const lintResult = sandbox.lintTomlSource(updated3);
assert.strictEqual(lintResult.errors.length, 0, 'Serialized TOML must produce 0 lint errors');
assert.strictEqual(lintResult.warnings.length, 0, 'Serialized TOML must produce 0 lint warnings');

console.log('[Test] ALL Smart qm.toml Category Block Serializer tests PASSED!');
