#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const assert = require('assert');
const vm = require('vm');

console.log('[Test] Running Bidirectional Cascading Dependency Safety Test Suite...');

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
assert(jsCode.includes('function getDependentPatches'), 'getDependentPatches must be defined');
assert(jsCode.includes('function checkPatchPrerequisites'), 'checkPatchPrerequisites must be defined');

vm.runInContext(jsCode, sandbox);

// Sample Patch Network:
// PatchA <-- PatchB <-- PatchC
// PatchX (standalone)
const allPatches = [
  { id: "PatchA", name: "Core Hook A", requires: [] },
  { id: "PatchB", name: "Feature B", requires: ["PatchA"] },
  { id: "PatchC", name: "Extension C", requires: ["PatchB"] },
  { id: "PatchX", name: "Standalone X", requires: [] }
];

// 2. Test getDependentPatches (Direct and Recursive)
const depsOfA = sandbox.getDependentPatches("PatchA", allPatches, []);
assert.strictEqual(depsOfA.length, 2, 'PatchA should have 2 dependents (PatchB and PatchC)');
const depIdsA = depsOfA.map(p => p.id);
assert(depIdsA.includes("PatchB"));
assert(depIdsA.includes("PatchC"));

const depsOfB = sandbox.getDependentPatches("PatchB", allPatches, []);
assert.strictEqual(depsOfB.length, 1, 'PatchB should have 1 dependent (PatchC)');
assert.strictEqual(depsOfB[0].id, "PatchC");

const depsOfX = sandbox.getDependentPatches("PatchX", allPatches, []);
assert.strictEqual(depsOfX.length, 0, 'PatchX should have 0 dependents');

// 3. Test checkPatchPrerequisites when DISABLING
// Disabling PatchA when all are currently enabled -> must confirm disabling PatchB and PatchC
const disableCheckA = sandbox.checkPatchPrerequisites("PatchA", false, allPatches, []);
assert.strictEqual(disableCheckA.action, "confirm_disable_dependents");
assert.strictEqual(disableCheckA.affectedPatches.length, 2);

// Disabling PatchA when PatchB and PatchC are ALREADY disabled -> can proceed directly
const disableCheckAWithDepsDisabled = sandbox.checkPatchPrerequisites("PatchA", false, allPatches, ["PatchB", "PatchC"]);
assert.strictEqual(disableCheckAWithDepsDisabled.action, "proceed", 'Should proceed when dependents are already disabled');

// Disabling PatchX -> proceeds directly
const disableCheckX = sandbox.checkPatchPrerequisites("PatchX", false, allPatches, []);
assert.strictEqual(disableCheckX.action, "proceed");

// 4. Test checkPatchPrerequisites when ENABLING
// Enabling PatchC when PatchA and PatchB are currently disabled -> must confirm enabling PatchA and PatchB
const enableCheckC = sandbox.checkPatchPrerequisites("PatchC", true, allPatches, ["PatchA", "PatchB", "PatchC"]);
assert.strictEqual(enableCheckC.action, "confirm_enable_prerequisites");
assert.strictEqual(enableCheckC.affectedPatches.length, 2);
const enableAffectedIds = enableCheckC.affectedPatches.map(p => p.id);
assert(enableAffectedIds.includes("PatchA"));
assert(enableAffectedIds.includes("PatchB"));

// Enabling PatchB when PatchA is ALREADY enabled -> proceeds directly
const enableCheckB = sandbox.checkPatchPrerequisites("PatchB", true, allPatches, ["PatchB"]);
assert.strictEqual(enableCheckB.action, "proceed", 'Should proceed when prerequisite PatchA is already active');

console.log('[Test] ALL Bidirectional Cascading Dependency tests PASSED!');
