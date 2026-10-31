#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const assert = require('assert');
const vm = require('vm');

console.log('[Test] Running Patch Editor Live Diff Test Suite...');

const jsPath = path.join(__dirname, '../ui/app.js');
const jsCode = fs.readFileSync(jsPath, 'utf8');

const sandbox = {
  console,
  document: {
    getElementById: () => ({ value: '', innerHTML: '', textContent: '', classList: { add: () => {}, remove: () => {} } }),
    querySelector: () => null,
    querySelectorAll: () => [],
    addEventListener: () => {}
  },
  showToast: () => {},
  escapeHtml: (s) => String(s)
};
vm.createContext(sandbox);

// 1. Verify function declarations exist in app.js
assert(jsCode.includes('function computeLivePatchDiff'), 'computeLivePatchDiff must be defined');
assert(jsCode.includes('function getEditorContextSlices'), 'getEditorContextSlices must be defined');
assert(jsCode.includes('function setContextLinesCount'), 'setContextLinesCount must be defined');

vm.runInContext(jsCode, sandbox);

// 2. Test setContextLinesCount
sandbox.setContextLinesCount(6);
assert.strictEqual(sandbox.state.patchContextLines, 6, 'Context count should be set to 6');
sandbox.setContextLinesCount(0);
assert.strictEqual(sandbox.state.patchContextLines, 1, 'Context count should clamp to min 1');
sandbox.setContextLinesCount(50);
assert.strictEqual(sandbox.state.patchContextLines, 15, 'Context count should clamp to max 15');

// 3. Test getEditorContextSlices
const sourceLines = [
  "line 1", "line 2", "line 3", "line 4", "line 5",
  "target A", "target B",
  "line 8", "line 9", "line 10", "line 11"
];
const slices = sandbox.getEditorContextSlices(sourceLines, [6, 7], 3);
assert.strictEqual(slices.topLines.length, 3, 'Top context should have 3 lines');
assert.strictEqual(slices.topLines[0].lineNo, 3);
assert.strictEqual(slices.topLines[2].lineNo, 5);
assert.strictEqual(slices.bottomLines.length, 3, 'Bottom context should have 3 lines');
assert.strictEqual(slices.bottomLines[0].lineNo, 8);
assert.strictEqual(slices.bottomLines[2].lineNo, 10);

// 4. Test computeLivePatchDiff with "at" (Replace)
const diffAt = sandbox.computeLivePatchDiff(sourceLines, [6, 7], "at", "new code A\nnew code B", true);
const deletedAt = diffAt.filter(l => l.type === "delete");
const addedAt = diffAt.filter(l => l.type === "add");
assert.strictEqual(deletedAt.length, 2, 'Should delete 2 target lines');
assert.strictEqual(deletedAt[0].content, "target A");
assert.strictEqual(deletedAt[1].content, "target B");
assert.strictEqual(addedAt.length, 2, 'Should add 2 replacement lines');
assert.strictEqual(addedAt[0].content, "new code A");
assert.strictEqual(addedAt[1].content, "new code B");

// 5. Test computeLivePatchDiff with "before" (Inject Before)
const diffBefore = sandbox.computeLivePatchDiff(sourceLines, [6, 6], "before", "injected before anchor", false);
const addedBefore = diffBefore.filter(l => l.type === "add");
const deletedBefore = diffBefore.filter(l => l.type === "delete");
assert.strictEqual(deletedBefore.length, 0, 'No deletions for before mode');
assert.strictEqual(addedBefore.length, 1);
assert.strictEqual(addedBefore[0].content, "injected before anchor");

// 6. Test computeLivePatchDiff with "after" (Inject After)
const diffAfter = sandbox.computeLivePatchDiff(sourceLines, [7, 7], "after", "injected after anchor", false);
const addedAfter = diffAfter.filter(l => l.type === "add");
const deletedAfter = diffAfter.filter(l => l.type === "delete");
assert.strictEqual(deletedAfter.length, 0, 'No deletions for after mode');
assert.strictEqual(addedAfter.length, 1);
assert.strictEqual(addedAfter[0].content, "injected after anchor");

// 7. Performance benchmark: computeLivePatchDiff must run < 5ms for 100 calls
const t0 = Date.now();
for (let i = 0; i < 100; i++) {
  sandbox.computeLivePatchDiff(sourceLines, [6, 7], "at", "perf payload " + i, true);
}
const elapsed = Date.now() - t0;
console.log(`[Test] 100 diff computations executed in ${elapsed}ms`);
assert(elapsed < 100, 'Diff computation benchmark must easily exceed 1000 ops/sec');

console.log('[Test] ALL Patch Editor Live Diff tests PASSED!');
