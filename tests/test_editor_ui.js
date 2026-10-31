const assert = require('assert');
const path = require('path');
const fs = require('fs');
const vm = require('vm');

console.log("=== RUNNING EDITOR UI & PROBLEMS PANEL INTERACTION TESTS ===");

const APP_JS_PATH = path.resolve(__dirname, '..', 'ui', 'app.js');
const INDEX_HTML_PATH = path.resolve(__dirname, '..', 'ui', 'index.html');
const VANILLA_REF_PATH = path.resolve(__dirname, '..', 'ui', 'vanilla_engine_reference.json');

const appJsSource = fs.readFileSync(APP_JS_PATH, 'utf8');
const indexHtmlSource = fs.readFileSync(INDEX_HTML_PATH, 'utf8');
const vanillaRefData = JSON.parse(fs.readFileSync(VANILLA_REF_PATH, 'utf8'));

// Verify Zero Emojis
assert(!/\p{Extended_Pictographic}/u.test(appJsSource), "app.js must contain 0 emojis");
assert(!/\p{Extended_Pictographic}/u.test(indexHtmlSource), "index.html must contain 0 emojis");

// 1. DOM Mocking
const domRegistry = {};
class MockElement {
  constructor(tag, id = '') {
    this.tagName = tag.toUpperCase();
    this.id = id;
    this.children = [];
    this.attributes = {};
    this.dataset = {};
    this.style = {};
    this.classList = {
      _classes: new Set(),
      add: (c) => this.classList._classes.add(c),
      remove: (c) => this.classList._classes.delete(c),
      contains: (c) => this.classList._classes.has(c),
      toggle: (c, force) => {
        if (force !== undefined) {
          force ? this.classList._classes.add(c) : this.classList._classes.delete(c);
        } else {
          this.classList._classes.has(c) ? this.classList._classes.delete(c) : this.classList._classes.add(c);
        }
      }
    };
    this._innerHTML = '';
    this.textContent = '';
    this.value = '';
    this.selectionStart = 0;
    this.selectionEnd = 0;
    this.scrollTop = 0;
    this.clientHeight = 500;
    this.clientWidth = 800;
    this.disabled = false;
    this.listeners = {};
  }
  get className() { return Array.from(this.classList._classes).join(' '); }
  set className(val) {
    this.classList._classes.clear();
    (val || '').split(/\s+/).filter(Boolean).forEach(c => this.classList._classes.add(c));
  }
  get innerHTML() { return this._innerHTML; }
  set innerHTML(val) {
    this._innerHTML = val;
    if (val.includes('class="problem-row"')) {
      const matchCount = (val.match(/class="problem-row"/g) || []).length;
      this.children = Array.from({ length: matchCount }, (_, idx) => {
        const el = new MockElement('div');
        el.classList.add('problem-row');
        // Extract line from html snippet
        const lineMatch = val.match(/data-line="(\d+)"/);
        if (lineMatch) el.dataset.line = lineMatch[1];
        const colMatch = val.match(/data-col="(\d+)"/);
        if (colMatch) el.dataset.col = colMatch[1];
        return el;
      });
    } else if (val.includes('class="autocomplete-item')) {
      const matchCount = (val.match(/class="autocomplete-item/g) || []).length;
      this.children = Array.from({ length: matchCount }, (_, idx) => {
        const el = new MockElement('div');
        el.classList.add('autocomplete-item');
        el.dataset.idx = String(idx);
        return el;
      });
    } else {
      this.children = [];
    }
  }
  querySelectorAll(sel) {
    if (sel === '.problem-row') return this.children.filter(c => c.classList.contains('problem-row'));
    if (sel === '.autocomplete-item') return this.children.filter(c => c.classList.contains('autocomplete-item'));
    return [];
  }
  closest(sel) {
    if (sel === '#editor-autocomplete-box' && this.id === 'editor-autocomplete-box') return this;
    return null;
  }
  addEventListener(evt, fn) {
    this.listeners[evt] = this.listeners[evt] || [];
    this.listeners[evt].push(fn);
  }
  dispatchEvent(evt) {
    const list = this.listeners[evt.type] || [];
    list.forEach(fn => fn(evt));
  }
  focus() {}
  scrollIntoView() {}
}

[
  'editor-mod-select', 'mod-files-tree', 'mod-file-editor-textarea', 'editor-file-badge',
  'editor-dirty-indicator', 'editor-line-numbers', 'editor-validation-status',
  'editor-problems-counter', 'editor-problems-list', 'editor-problems-file',
  'editor-problems-panel', 'btn-toggle-problems', 'editor-problems-header',
  'editor-autocomplete-box', 'toast-banner'
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
  Event: class { constructor(type) { this.type = type; } },
  fetch: async () => ({ ok: true, json: async () => vanillaRefData, text: async () => JSON.stringify(vanillaRefData) }),
  showToast: () => {}
};

const vmContext = vm.createContext(sandbox);
vm.runInContext(appJsSource, vmContext);

const state = vm.runInContext('state', vmContext);
state.vanillaApi = vm.runInContext('normalizeVanillaReferenceCatalog', vmContext)(vanillaRefData);

// Test 1: Live syntax check on Lua syntax error
state.editorActiveFilePath = 'test.lua';
const textarea = domRegistry['mod-file-editor-textarea'];
const statusEl = domRegistry['editor-validation-status'];
const counterEl = domRegistry['editor-problems-counter'];
const listEl = domRegistry['editor-problems-list'];

// Write invalid Lua code
textarea.value = `function createSquad(size)
  local soldiers = {}
  -- missing end
`;
vm.runInContext('updateLiveEditorValidation()', vmContext);

assert(statusEl.textContent.includes('1 Error'), `Status badge should report error, got: ${statusEl.textContent}`);
assert(statusEl.classList.contains('status-error'), 'Status badge must have status-error class');
assert(counterEl.textContent.includes('1 Error'), `Problems counter should report 1 Error, got: ${counterEl.textContent}`);
assert(listEl.children.length === 1, `Problems list should render 1 problem card, found: ${listEl.children.length}`);
assert(listEl.innerHTML.includes('Unclosed block'), 'Problem card should explain unclosed block');
console.log("  [PASS] Live Lua syntax error detection and Problems panel rendering");

// Test 2: Click problem card to jump to line
const jumpToEditorLine = vm.runInContext('jumpToEditorLine', vmContext);
jumpToEditorLine(1, 1);
assert.strictEqual(textarea.selectionStart, 0, "Line 1 should position selection at offset 0");
console.log("  [PASS] Problem click line-jump navigation");

// Test 3: Fix syntax error and revalidate
textarea.value += "end\n";
vm.runInContext('updateLiveEditorValidation()', vmContext);

assert(statusEl.textContent.includes('OK'), `Status badge should be OK, got: ${statusEl.textContent}`);
assert(statusEl.classList.contains('status-ok'), 'Status badge must have status-ok class');
assert(counterEl.textContent.includes('0 Errors'), `Problems counter should report 0 Errors, got: ${counterEl.textContent}`);
assert(listEl.innerHTML.includes('No problems have been detected'), "Problems list should show clean empty state");
console.log("  [PASS] Live error resolution and clean status badge update");

// Test 4: Autocomplete UI trigger and item insertion
textarea.value = "local grunt = createGru";
textarea.selectionStart = textarea.selectionEnd = textarea.value.length;

vm.runInContext('updateAutocompleteUI()', vmContext);
const autocompleteBox = domRegistry['editor-autocomplete-box'];
assert(autocompleteBox.style.display === 'block', "Autocomplete dropdown should be visible");
assert(autocompleteBox.children.length > 0, "Autocomplete dropdown should have suggestion children");

// Test 5: Insert suggestion
const insertAutocompleteItem = vm.runInContext('insertAutocompleteItem', vmContext);
insertAutocompleteItem(0);
assert(textarea.value.includes('createGrunt'), `Textarea should contain inserted createGrunt function, got: ${textarea.value}`);
assert(autocompleteBox.style.display === 'none', "Autocomplete dropdown should be hidden after insertion");
console.log("  [PASS] Autocomplete popup rendering and symbol insertion");

console.log("=== ALL EDITOR UI TESTS PASSED ===");
