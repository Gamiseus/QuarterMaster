#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const assert = require('assert');
const vm = require('vm');

console.log('[Test] Running Patch Wizard & Two-Tier Dependency Test Suite...');

const jsPath = path.join(__dirname, '../ui/app.js');
const jsCode = fs.readFileSync(jsPath, 'utf8');

const sandbox = {
  console,
  document: {
    getElementById: () => ({ value: '', innerHTML: '', textContent: '', classList: { add: () => {}, remove: () => {} }, appendChild: () => {}, addEventListener: () => {} }),
    querySelector: () => null,
    querySelectorAll: () => [],
    addEventListener: () => {}
  },
  showToast: () => {},
  escapeHtml: (s) => String(s)
};
vm.createContext(sandbox);

// 1. Verify function declarations exist in app.js
assert(jsCode.includes('function filterSchemaCompliantPatches'), 'filterSchemaCompliantPatches must be defined');
assert(jsCode.includes('function addWizardDependency'), 'addWizardDependency must be defined');
assert(jsCode.includes('function removeWizardDependency'), 'removeWizardDependency must be defined');
assert(jsCode.includes('function confirmWizardAndOpenEditor'), 'confirmWizardAndOpenEditor must be defined');

vm.runInContext(jsCode, sandbox);

// 2. Test filterSchemaCompliantPatches
const rawPatches = [
  { id: "GCore-InitGrid", target: "includes/mapBuild.lua", pattern: "initTerrainGrid" },
  { target: "conf.lua", pattern: "window.title" }, // Invalid: missing id
  { id: "NoTarget", pattern: "foo" }, // Invalid: missing target
  { id: "NoPattern", target: "main.lua" }, // Invalid: missing pattern
  { id: "Warfare-Recoil", target: "includes/weapons.lua", pattern: "applyRecoil" }
];

const compliant = sandbox.filterSchemaCompliantPatches(rawPatches);
assert.strictEqual(compliant.length, 2, 'Should only keep 2 schema-compliant patches with explicit IDs');
assert.strictEqual(compliant[0].id, "GCore-InitGrid");
assert.strictEqual(compliant[1].id, "Warfare-Recoil");

// 3. Test Dependency Adding and Removing
sandbox.state.patchWizardData.patchId = "MyNewTerrainPatch";
sandbox.state.patchWizardData.requires = [];

// Add first dependency
sandbox.addWizardDependency("GCore-InitGrid");
assert.strictEqual(sandbox.state.patchWizardData.requires.length, 1);
assert.strictEqual(sandbox.state.patchWizardData.requires[0], "GCore-InitGrid");

// Prevent duplicate
sandbox.addWizardDependency("GCore-InitGrid");
assert.strictEqual(sandbox.state.patchWizardData.requires.length, 1, 'Duplicate must not be added');

// Prevent self-dependency
sandbox.addWizardDependency("MyNewTerrainPatch");
assert.strictEqual(sandbox.state.patchWizardData.requires.length, 1, 'Self-dependency must not be added');

// Add second dependency
sandbox.addWizardDependency("Warfare-Recoil");
assert.strictEqual(sandbox.state.patchWizardData.requires.length, 2);

// Remove dependency
sandbox.removeWizardDependency("GCore-InitGrid");
assert.strictEqual(sandbox.state.patchWizardData.requires.length, 1);
assert.strictEqual(sandbox.state.patchWizardData.requires[0], "Warfare-Recoil");

// 4. Test confirmWizardAndOpenEditor
sandbox.state.currentSourceLines = ["line 1", "line 2", "line 3", "target line 4", "target line 5", "line 6"];
sandbox.state.patchTargetRange = [4, 5];
sandbox.state.patchWizardStep = 1;

sandbox.confirmWizardAndOpenEditor();
assert.strictEqual(sandbox.state.patchWizardStep, 2, 'Should transition to step 2 (Editor)');
assert.strictEqual(sandbox.state.patchWizardData.pattern, "target line 4\ntarget line 5", 'Pattern should capture selected target lines');

console.log('[Test] ALL Patch Wizard & Two-Tier Dependency tests PASSED!');
