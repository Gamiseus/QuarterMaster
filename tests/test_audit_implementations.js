const fs = require('fs');
const path = require('path');
const vm = require('vm');
const assert = require('assert');

console.log("=== RUNNING AUDIT IMPLEMENTATIONS EMPIRICAL TEST ===");

const UI_DIR = path.resolve(__dirname, '..', 'ui');
const APP_JS_PATH = path.join(UI_DIR, 'app.js');
const INDEX_HTML_PATH = path.join(UI_DIR, 'index.html');

const appJsSource = fs.readFileSync(APP_JS_PATH, 'utf8');
const indexHtmlSource = fs.readFileSync(INDEX_HTML_PATH, 'utf8');

// 1. Emoji Check
const emojiPattern = /\p{Extended_Pictographic}/u;
assert(!emojiPattern.test(appJsSource), "app.js must contain 0 emojis");
assert(!emojiPattern.test(indexHtmlSource), "index.html must contain 0 emojis");
console.log("  [PASS] Zero emojis in app.js and index.html");

// 2. Mock DOM for VM evaluation
const domRegistry = {};
class MockElement {
  constructor(tag, id = '') {
    this.tagName = tag.toUpperCase();
    this.id = id;
    this.children = [];
    this.attributes = {};
    this.style = {};
    this.classList = {
      add: (c) => this._classes.add(c),
      remove: (c) => this._classes.delete(c),
      contains: (c) => this._classes.has(c),
      toggle: (c, force) => force ? this._classes.add(c) : this._classes.delete(c)
    };
    this._classes = new Set();
    this._innerHTML = '';
    this.textContent = '';
    this.value = '';
    this.disabled = false;
    this.listeners = {};
  }
  get innerHTML() { return this._innerHTML; }
  set innerHTML(val) {
    this._innerHTML = val;
    if (val.includes('class="tree-file-item"')) {
      const matchCount = (val.match(/class="tree-file-item"/g) || []).length;
      this.children = Array.from({ length: matchCount }, () => new MockElement('div'));
      this.children.forEach(c => c._classes.add('tree-file-item'));
    }
  }
  querySelectorAll(sel) {
    if (sel === '.tree-file-item') return this.children.filter(c => c._classes.has('tree-file-item'));
    return [];
  }
  closest() { return null; }
  getBoundingClientRect() { return { top: 0, bottom: 500, left: 0, right: 800, width: 800, height: 500 }; }
  addEventListener(evt, fn) {
    this.listeners[evt] = this.listeners[evt] || [];
    this.listeners[evt].push(fn);
  }
}

['editor-mod-select', 'mod-files-tree', 'mod-file-editor-textarea', 'editor-validation-status',
 'editor-dirty-indicator', 'editor-line-numbers', 'launch-primary-label', 'btn-launch-primary',
 'btn-copy-loadout-md', 'btn-undo-order', 'btn-redo-order', 'toast-banner'].forEach(id => {
  domRegistry[id] = new MockElement('div', id);
});

const mockDocument = {
  getElementById: (id) => domRegistry[id] || null,
  querySelectorAll: () => [],
  querySelector: (sel) => null,
  addEventListener: () => {}
};
const mockWindow = { addEventListener: () => {}, localStorage: { getItem: () => null, setItem: () => {} } };
const sandbox = {
  console,
  document: mockDocument,
  window: mockWindow,
  navigator: { clipboard: { writeText: async () => {} } },
  HTMLDialogElement: { prototype: {} },
  setTimeout,
  clearTimeout,
  setInterval: () => 1,
  clearInterval: () => {}
};
const vmContext = vm.createContext(sandbox);
vm.runInContext(appJsSource, vmContext);

// 3. Test Markdown Linter
console.log("Testing Markdown Linter...");
const validMd = "# Header\n\n```lua\nlocal x = 1\n```\n\nText paragraph.";
const invalidMd = "# Header\n\n```lua\nlocal x = 1\n\nMissing closing fence";

const checkMarkdownSyntax = vmContext.checkMarkdownSyntax;
assert(typeof checkMarkdownSyntax === 'function', "checkMarkdownSyntax must be defined in app.js");

const resValid = checkMarkdownSyntax(validMd);
assert(resValid.valid === true, "Valid markdown should pass validation");

const resInvalid = checkMarkdownSyntax(invalidMd);
assert(resInvalid.valid === false, "Unclosed code block must fail validation");
assert(resInvalid.line === 3, "Failure line should pinpoint fence start (line 3)");
console.log("  [PASS] checkMarkdownSyntax correctly detects balanced and unclosed fences");

// 4. Test Mod Editor Auto-Load Fix
console.log("Testing Mod Editor Auto-Load Lifecycle...");
const state = vm.runInContext('state', vmContext);
state.data = {
  installed_mods: [
    { manifest: { id: "EngineInterceptor", name: "Engine Interceptor", version: "1.0.0" }, install_path: "mods/EngineInterceptor" },
    { manifest: { id: "GCore", name: "GCore", version: "1.0.0" }, install_path: "mods/GCore" }
  ],
  active_profile: { id: "default", name: "Default Profile", enabled_mods: ["EngineInterceptor", "GCore"], mod_priorities: {} }
};

// Simulate opening the editor tab when state.editorActiveModDir is already set to the first mod
state.editorActiveModDir = "EngineInterceptor";
let loadTreeCalled = false;
let loadedDir = null;
sandbox.loadModFilesTree = (dir) => {
  loadTreeCalled = true;
  loadedDir = dir;
  domRegistry['mod-files-tree'].innerHTML = '<div class="tree-file-item">qm.toml</div>';
};

// First call: Tree is empty, so it MUST call loadModFilesTree even though activeMod matches editorActiveModDir
vm.runInContext('initModEditorTab()', vmContext);
assert(loadTreeCalled === true, "initModEditorTab must load file tree when tree container is empty");
assert(loadedDir === "EngineInterceptor", "Must load the active mod's directory");
console.log("  [PASS] initModEditorTab auto-loads file tree without requiring dropdown toggle");

console.log("\nALL AUDIT IMPLEMENTATION UNIT TESTS PASSED CLEANLY!");
