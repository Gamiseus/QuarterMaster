#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const assert = require('assert');
const vm = require('vm');

console.log('[Test] Running Studio UX Refinements Test Suite...');

const jsPath = path.join(__dirname, '../ui/app.js');
const jsCode = fs.readFileSync(jsPath, 'utf8');

const calls = [];
const sandbox = {
  console,
  setTimeout,
  clearTimeout,
  fetch: async (url, opts) => {
    calls.push({ type: 'fetch', url, opts });
    return {
      ok: true,
      json: async () => ({ ok: true, saved: true, content: '# === GENERAL ===\n' })
    };
  },
  document: {
    addEventListener: () => {},
    getElementById: (id) => ({
      id,
      value: id === 'wizard-patch-id' ? 'TestRefinedPatch' : (id === 'wizard-mod-select' ? 'GCore' : ''),
      innerHTML: '',
      textContent: '',
      style: {},
      scrollHeight: 160,
      classList: {
        add: () => {},
        remove: () => {},
        contains: () => false,
        toggle: () => {}
      },
      addEventListener: () => {},
      querySelectorAll: () => []
    }),
    querySelector: () => null,
    querySelectorAll: () => []
  },
  showToast: (msg, isErr) => calls.push({ type: 'toast', msg, isErr }),
  apiGet: async (url) => {
    calls.push({ type: 'apiGet', url });
    return { content: '# === GENERAL ===\n' };
  },
  apiPost: async (url, body) => {
    calls.push({ type: 'apiPost', url, body });
    return { ok: true, saved: true };
  },
  refreshState: async () => calls.push({ type: 'refreshState' }),
  escapeHtml: (s) => String(s)
};
vm.createContext(sandbox);
vm.runInContext(jsCode, sandbox);

// 1. Verify clearPatchSelection
sandbox.state.patchTargetRange = [10, 20];
sandbox.clearPatchSelection();
assert.strictEqual(sandbox.state.patchTargetRange, null, 'clearPatchSelection must reset patchTargetRange to null');

// 2. Verify autoResizeTextarea
const mockTextarea = { style: {}, scrollHeight: 180 };
sandbox.autoResizeTextarea(mockTextarea);
assert(mockTextarea.style.height.includes('px'), 'Textarea height should be set in px');
assert.strictEqual(mockTextarea.style.height, '184px', 'Textarea height should be scrollHeight + 4');

// 3. Verify inline unified diff in left pane when patchWizardStep === 2
const sampleVanilla = [
  "local a = 1",
  "local targetLine1 = 2",
  "local targetLine2 = 3",
  "local c = 4"
];
sandbox.state.currentSourceLines = sampleVanilla;
sandbox.state.patchTargetRange = [2, 3];
sandbox.state.patchWizardStep = 2; // Editing mode
sandbox.state.patchWizardData = {
  patchId: "InjectDiffTest",
  position: "at",
  pattern: "local targetLine1 = 2\nlocal targetLine2 = 3",
  payload: "local newPayloadLine = 99",
  matchIndent: true
};

const diffHtml = sandbox.renderPatchStudioSourceLines(sampleVanilla, [2, 3], [], false);
assert(diffHtml.includes('diff-delete'), 'Must contain diff-delete rows in left pane during editing');
assert(diffHtml.includes('diff-add'), 'Must contain diff-add rows in left pane during editing');
assert(diffHtml.includes('REMOVED'), 'Must display REMOVED badge');
assert(diffHtml.includes('ADDED'), 'Must display ADDED badge');
assert(diffHtml.includes('local newPayloadLine = 99'), 'Must render payload line inline in the source view');
assert(diffHtml.includes('local a = 1'), 'Must preserve vanilla lines before target');
assert(diffHtml.includes('local c = 4'), 'Must preserve vanilla lines after target');

// 4. Verify returning to Wizard retains in-progress patch
sandbox.state.patchWizardStep = 2;
sandbox.renderPatchStudioRightPane(); // Renders step 2 editor
assert.strictEqual(sandbox.state.patchWizardData.patchId, "InjectDiffTest", 'In-progress patchId must be preserved');
assert.strictEqual(sandbox.state.patchWizardData.payload, "local newPayloadLine = 99", 'In-progress payload must be preserved');

// Transition back to step 1
sandbox.state.patchWizardStep = 1;
sandbox.renderPatchStudioRightPane(); // Renders step 1 wizard
assert.strictEqual(sandbox.state.patchWizardData.patchId, "InjectDiffTest", 'Patch ID must persist in wizard step 1');
assert.strictEqual(sandbox.state.patchWizardData.payload, "local newPayloadLine = 99", 'Payload must persist in wizard step 1');

// 5. Verify savePatchToModToml posts to /api/devtools/file/save with mod_dir
(async () => {
  sandbox.state.patchStudioActiveFile = "includes/mapBuild.lua";
  sandbox.state.patchWizardData = {
    patchId: "SaveVerifyPatch",
    targetMod: "GCore",
    position: "after",
    pattern: "local targetLine1 = 2",
    payload: "local injected = 100",
    category: "# BATTLEFIELD & MAP BUILD"
  };

  await sandbox.savePatchToModToml();

  const saveCall = calls.find(c => c.type === 'fetch' && c.opts?.method === 'POST');
  assert(saveCall, 'fetch POST must have been called to save patch');
  assert.strictEqual(saveCall.url, '/api/devtools/file/save', 'Must POST to /api/devtools/file/save');
  const body = JSON.parse(saveCall.opts.body);
  assert.strictEqual(body.mod_dir, 'GCore', 'Must include mod_dir in request body');
  assert.strictEqual(body.file_path, 'qm.toml', 'Must target qm.toml');
  assert(body.content.includes('[[patches]]'), 'Serialized content must include [[patches]]');
  assert(body.content.includes('SaveVerifyPatch'), 'Serialized content must include patch ID');

  console.log('[Test] ALL Studio UX Refinements tests PASSED!');
})();
