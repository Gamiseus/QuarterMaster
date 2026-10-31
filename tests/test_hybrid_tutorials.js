#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const assert = require('assert');
const vm = require('vm');

console.log('[Test] Running Hybrid Templates & Interactive Tutorial Hub Test Suite...');

const jsPath = path.join(__dirname, '../ui/app.js');
const jsCode = fs.readFileSync(jsPath, 'utf8');

const sandbox = {
  console,
  setTimeout,
  clearTimeout,
  document: {
    getElementById: () => ({ value: '', innerHTML: '', textContent: '', style: {}, classList: { add: () => {}, remove: () => {} }, click: () => {} }),
    querySelector: () => null,
    querySelectorAll: () => [],
    addEventListener: () => {}
  },
  showToast: () => {},
  escapeHtml: (s) => String(s),
  prompt: (msg, def) => def || "my_new_mod",
  apiPost: async () => ({ ok: true }),
  refreshState: async () => {}
};
vm.createContext(sandbox);

// 1. Verify declarations
assert(jsCode.includes('const TUTORIAL_TEMPLATES ='), 'TUTORIAL_TEMPLATES must be defined');
assert(jsCode.includes('function selectTutorialTemplate'), 'selectTutorialTemplate must be defined');
assert(jsCode.includes('function renderTutorialsHubHtml'), 'renderTutorialsHubHtml must be defined');
assert(jsCode.includes('function scaffoldAndOpenInStudio'), 'scaffoldAndOpenInStudio must be defined');

vm.runInContext(jsCode, sandbox);

// 2. Test TUTORIAL_TEMPLATES
const templates = sandbox.TUTORIAL_TEMPLATES;
assert(Array.isArray(templates), 'TUTORIAL_TEMPLATES must be an array');
assert.strictEqual(templates.length, 5, 'Must have 5 tutorial templates');

const expectedIds = ['minimal-patch', 'weapon-ballistics', 'map-terrain', 'lua-module', 'asset-override'];
for (const id of expectedIds) {
  assert(templates.some(t => t.id === id), `Must include template ${id}`);
}

// 3. Test selectTutorialTemplate
const tpl = sandbox.selectTutorialTemplate('weapon-ballistics');
assert.strictEqual(tpl.id, 'weapon-ballistics');
assert.strictEqual(sandbox.state.activeTutorialTemplateId, 'weapon-ballistics');
assert(tpl.steps.length >= 3, 'Template must define at least 3 tutorial steps');
assert(tpl.fileTree, 'Template must define file tree structure');

// 4. Test renderTutorialsHubHtml
const html = sandbox.renderTutorialsHubHtml(tpl);
assert(html.includes('Weapon &amp; Ballistics') || html.includes('Weapon & Ballistics'), 'HTML must render template title');
assert(html.includes('Manifest &amp; Metadata') || html.includes('Manifest & Metadata'), 'HTML must render Step 1');
assert(html.includes('In-Memory Patching'), 'HTML must render Step 2');
assert(html.includes('Scaffold Mod from Template'), 'HTML must render Scaffold button');

// 5. Test scaffoldAndOpenInStudio
sandbox.scaffoldAndOpenInStudio('minimal-patch', 'my_custom_patch').then(() => {
  assert.strictEqual(sandbox.state.activeModTab, 'studio', 'Must transition to Patch Studio upon scaffolding');
  console.log('[Test] ALL Hybrid Templates & Interactive Tutorial Hub tests PASSED!');
}).catch(err => {
  console.error(err);
  process.exit(1);
});
