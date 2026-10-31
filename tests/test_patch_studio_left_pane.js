#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const assert = require('assert');

console.log('[Test] Running Patch Studio Left Pane Test Suite...');

const jsPath = path.join(__dirname, '../ui/app.js');
const jsCode = fs.readFileSync(jsPath, 'utf8');

// Load sandbox to test functions in isolation
const vm = require('vm');
const sandbox = {
  console,
  document: {
    getElementById: () => null,
    querySelector: () => null,
    querySelectorAll: () => [],
    addEventListener: () => {}
  },
  state: {
    engineShowOtherPatches: true,
    patchTargetRange: [10, 15],
    currentSourceLines: new Array(50).fill("").map((_, i) => `line ${i + 1}`)
  },
  escapeHtml: (s) => String(s)
};
vm.createContext(sandbox);

// 1. Verify function declarations exist in app.js
assert(jsCode.includes('function setPatchTargetRange'), 'setPatchTargetRange must be defined');
assert(jsCode.includes('function adjustPatchBoundary'), 'adjustPatchBoundary must be defined');
assert(jsCode.includes('function togglePatchOverlay'), 'togglePatchOverlay must be defined');
assert(jsCode.includes('function renderPatchStudioSourceLines'), 'renderPatchStudioSourceLines must be defined');

// Execute app.js in sandbox
vm.runInContext(jsCode, sandbox);

// Set source lines for testing
sandbox.state.currentSourceLines = new Array(50).fill("").map((_, i) => `line ${i + 1}`);

// 2. Test setPatchTargetRange
sandbox.setPatchTargetRange(14, 20);
assert.strictEqual(sandbox.state.patchTargetRange[0], 14, 'Start line should be 14');
assert.strictEqual(sandbox.state.patchTargetRange[1], 20, 'End line should be 20');

// Test boundary safety
sandbox.setPatchTargetRange(-5, 1000);
assert(sandbox.state.patchTargetRange[0] >= 1, 'Start line must be >= 1');
assert(sandbox.state.patchTargetRange[1] <= sandbox.state.currentSourceLines.length, 'End line must not exceed line count');

// 3. Test adjustPatchBoundary
// Extend top boundary
sandbox.state.patchTargetRange = [10, 15];
sandbox.adjustPatchBoundary('top', 'extend');
assert.strictEqual(sandbox.state.patchTargetRange[0], 9, 'Top extend should decrement start line to 9');

// Shrink top boundary
sandbox.adjustPatchBoundary('top', 'shrink');
assert.strictEqual(sandbox.state.patchTargetRange[0], 10, 'Top shrink should increment start line to 10');

// Extend bottom boundary
sandbox.adjustPatchBoundary('bottom', 'extend');
assert.strictEqual(sandbox.state.patchTargetRange[1], 16, 'Bottom extend should increment end line to 16');

// Shrink bottom boundary
sandbox.adjustPatchBoundary('bottom', 'shrink');
assert.strictEqual(sandbox.state.patchTargetRange[1], 15, 'Bottom shrink should decrement end line to 15');

// Cannot shrink top past bottom
sandbox.state.patchTargetRange = [15, 15];
sandbox.adjustPatchBoundary('top', 'shrink');
assert.strictEqual(sandbox.state.patchTargetRange[0], 15, 'Top shrink must not cross bottom');

// 4. Test togglePatchOverlay
sandbox.togglePatchOverlay(false);
assert.strictEqual(sandbox.state.engineShowOtherPatches, false, 'Overlay should be disabled');
sandbox.togglePatchOverlay(true);
assert.strictEqual(sandbox.state.engineShowOtherPatches, true, 'Overlay should be enabled');

// 5. Test renderPatchStudioSourceLines
const sampleLines = ["local a = 1", "local b = 2", "local c = 3", "local d = 4", "local e = 5"];
const overlayPatches = [
  { patch_id: "GCore-Init", line_range: [1, 2] }
];
const html = sandbox.renderPatchStudioSourceLines(sampleLines, [3, 4], overlayPatches, true);
assert(html.includes('target-highlight'), 'HTML must include target-highlight class for lines 3 and 4');
assert(html.includes('other-patch-highlight'), 'HTML must include other-patch-highlight for lines 1 and 2');
assert(html.includes('[Patch: GCore-Init]'), 'HTML must include patch badge label');
assert(html.includes('adjustPatchBoundary'), 'HTML must include boundary control buttons');

console.log('[Test] ALL Patch Studio Left Pane tests PASSED!');
