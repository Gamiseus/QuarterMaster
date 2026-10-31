const fs = require('fs');
const path = require('path');
const vm = require('vm');
const assert = require('assert');

console.log("=== RUNNING EDITOR LIFECYCLE EMPIRICAL TEST ===");

const UI_DIR = path.resolve(__dirname, '..', 'ui');
const APP_JS_PATH = path.join(UI_DIR, 'app.js');
const INDEX_HTML_PATH = path.join(UI_DIR, 'index.html');

const appJsSource = fs.readFileSync(APP_JS_PATH, 'utf8');
const indexHtmlSource = fs.readFileSync(INDEX_HTML_PATH, 'utf8');

// 1. Emoji Check
const emojiPattern = /\p{Extended_Pictographic}/u;
assert(!emojiPattern.test(appJsSource), "app.js must contain 0 emojis");
assert(!emojiPattern.test(indexHtmlSource), "index.html must contain 0 emojis");

// 2. DOM Mocking
const domRegistry = {};
class MockElement {
  constructor(tag, id = '') {
    this.tagName = tag.toUpperCase();
    this.id = id;
    this.children = [];
    this.attributes = {};
    this.style = {};
    this.classList = {
      _classes: new Set(),
      add(c) { this._classes.add(c); },
      remove(c) { this._classes.delete(c); },
      contains(c) { return this._classes.has(c); },
      toggle(c, force) { if (force !== undefined) { force ? this._classes.add(c) : this._classes.delete(c); } else { this._classes.has(c) ? this._classes.delete(c) : this._classes.add(c); } }
    };
    this._innerHTML = '';
    this.textContent = '';
    this.value = '';
    this.disabled = false;
    this.listeners = {};
  }
  get innerHTML() { return this._innerHTML; }
  set innerHTML(val) {
    this._innerHTML = val;
    const treeMatches = val.match(/class="[^"]*tree-file-item/g);
    if (treeMatches && treeMatches.length > 0) {
      this.children = Array.from({ length: treeMatches.length }, () => new MockElement('div'));
      this.children.forEach(c => c.classList.add('tree-file-item'));
    } else if (val.includes('<option')) {
      const matchCount = (val.match(/<option/g) || []).length;
      this.children = Array.from({ length: matchCount }, () => new MockElement('option'));
    } else {
      this.children = [];
    }
  }
  querySelectorAll(sel) {
    if (sel === '.tree-file-item') return this.children.filter(c => c.classList.contains('tree-file-item'));
    if (sel === 'option') return this.children.filter(c => c.tagName === 'OPTION');
    return [];
  }
  addEventListener(evt, fn) {
    this.listeners[evt] = this.listeners[evt] || [];
    this.listeners[evt].push(fn);
  }
}

[
  'editor-mod-select', 'mod-files-tree', 'mod-file-editor-textarea', 'editor-file-badge',
  'editor-dirty-indicator', 'editor-line-numbers', 'editor-validation-status',
  'toast-banner'
].forEach(id => {
  domRegistry[id] = new MockElement('div', id);
});

const mockDocument = {
  getElementById: (id) => domRegistry[id] || null,
  querySelectorAll: () => [],
  querySelector: () => null,
  addEventListener: () => {}
};
const mockWindow = { addEventListener: () => {}, localStorage: { getItem: () => null, setItem: () => {} } };

const sandbox = {
  console,
  document: mockDocument,
  window: mockWindow,
  fetch: async (url) => {
    let payload = {};
    if (url.includes('/api/devtools/files')) {
      payload = [
        { relative_path: 'qm.toml', file_type: 'toml', size_bytes: 420 },
        { relative_path: 'mod.json', file_type: 'json', size_bytes: 256 },
        { relative_path: 'main.lua', file_type: 'lua', size_bytes: 1024 }
      ];
    } else if (url.includes('/api/devtools/file/read') || url.includes('/api/devtools/read-file')) {
      payload = { content: '[[patches]]\ntarget = "@game/main.lua"\npattern = "foo"\nreplacement = "bar"\n' };
    }
    return {
      ok: true,
      status: 200,
      json: async () => payload,
      text: async () => JSON.stringify(payload)
    };
  },
  showToast: (msg) => {}
};

const vmContext = vm.createContext(sandbox);
vm.runInContext(appJsSource, vmContext);

// Setup sample installed mods in state
const state = vm.runInContext('state', vmContext);
state.data = {
  installed_mods: [
    {
      manifest: { id: 'sample_mod', name: 'Sample Mod', version: '1.0.0' },
      install_path: 'sample_mod'
    }
  ]
};

// 1. Initial Call to initModEditorTab: should populate select and load file tree
vm.runInContext('initModEditorTab()', vmContext);

// Check that editor-mod-select has option
const sel = domRegistry['editor-mod-select'];
assert(sel.children.length > 0, "editor-mod-select should have populated options");
sel.value = 'sample_mod';

// Simulate async loadModFilesTree resolution
(async () => {
  await vm.runInContext('loadModFilesTree("sample_mod")', vmContext);
  const tree = domRegistry['mod-files-tree'];
  console.log("TREE INNERHTML:", tree.innerHTML);
  assert(tree.children.length === 3, `Expected 3 tree items, found ${tree.children.length}`);

  // Confirm auto-selected file is qm.toml
  assert.strictEqual(state.editorActiveFilePath, 'qm.toml', `Expected active file to be 'qm.toml', got '${state.editorActiveFilePath}'`);
  const textarea = domRegistry['mod-file-editor-textarea'];
  assert(textarea.value.includes('[[patches]]'), "Textarea should contain qm.toml content");

  console.log("  [PASS] Auto-load mod files tree and prefer qm.toml / mod.json");
  console.log("=== ALL EDITOR LIFECYCLE TESTS PASSED ===");
})().catch(err => {
  console.error("Test failed:", err);
  process.exit(1);
});
