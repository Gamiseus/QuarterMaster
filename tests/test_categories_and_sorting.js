const fs = require('fs');
const path = require('path');
const vm = require('vm');
const assert = require('assert');

const UI_DIR = path.resolve(__dirname, '..', 'ui');
const APP_JS_PATH = path.join(UI_DIR, 'app.js');
const REF_JSON_PATH = path.join(UI_DIR, 'vanilla_engine_reference.json');

const appJsSource = fs.readFileSync(APP_JS_PATH, 'utf8');
const catalogRaw = fs.readFileSync(REF_JSON_PATH, 'utf8');

// Lightweight Mock DOM
const domRegistry = {};
class MockElement {
  constructor(tag, id = '', initialClasses = []) {
    this.tagName = tag.toUpperCase();
    this.id = id;
    this._classes = new Set(initialClasses);
    this.dataset = {};
    this.hidden = false;
    this._innerHTML = '';
    this._textContent = '';
    this.value = '';
    this.checked = false;
    this.listeners = {};
    this.onclick = null;
    this.style = {};

    const self = this;
    this.classList = {
      add: (c) => self._classes.add(c),
      remove: (c) => self._classes.delete(c),
      contains: (c) => self._classes.has(c),
      toggle: (c, force) => {
        if (force === undefined) {
          if (self._classes.has(c)) { self._classes.delete(c); return false; }
          else { self._classes.add(c); return true; }
        } else if (force) {
          self._classes.add(c); return true;
        } else {
          self._classes.delete(c); return false;
        }
      }
    };
  }

  get innerHTML() { return this._innerHTML; }
  set innerHTML(val) { this._innerHTML = val; }
  get textContent() { return this._textContent; }
  set textContent(val) { this._textContent = val; }

  addEventListener(event, fn) {
    if (!this.listeners[event]) this.listeners[event] = [];
    this.listeners[event].push(fn);
  }
}

// Register necessary DOM elements
const ids = [
  'toast-banner', 'modtools-tab-scaffold', 'modtools-tab-api-docs', 'modtools-tab-editor',
  'api-search-input', 'api-module-filter', 'api-kind-filter', 'api-symbol-count',
  'api-symbols-list', 'api-symbol-detail', 'vanilla-api-bundled', 'api-category-pills',
  'api-action-filter', 'api-sort-by', 'btn-api-sort-dir', 'api-toggle-grouping',
  'api-file-filter-input', 'api-files-list', 'api-file-active-title', 'api-list-subtitle'
];

for (const id of ids) {
  domRegistry[id] = new MockElement('div', id);
}

domRegistry['api-kind-filter'].value = 'all';
domRegistry['api-sort-by'].value = 'default';
domRegistry['api-action-filter'].value = 'all';
domRegistry['api-toggle-grouping'].checked = true;
domRegistry['vanilla-api-bundled'].textContent = catalogRaw;

const mockDocument = {
  getElementById: (id) => domRegistry[id] || null,
  querySelectorAll: (selector) => {
    if (selector === '.category-pill') return [];
    if (selector === '.api-file-item') return [];
    if (selector === '.api-symbol-item') return [];
    return [];
  },
  createElement: (tag) => new MockElement(tag),
  body: { appendChild: () => {}, removeChild: () => {} },
  addEventListener: () => {}
};

const mockWindow = { addEventListener: () => {}, localStorage: { getItem: () => null, setItem: () => {} } };
const sandbox = { console, document: mockDocument, window: mockWindow, navigator: { clipboard: { writeText: async () => {} } }, HTMLDialogElement: { prototype: {} }, setTimeout, clearTimeout };
const vmContext = vm.createContext(sandbox);

vm.runInContext(appJsSource, vmContext);

console.log("=== RUNNING CATEGORIES & SORTING EMPIRICAL TEST ===");

(async () => {
  // Initialize
  await vm.runInContext('initApiDocsTab()', vmContext);
  const state = vm.runInContext('state', vmContext);

  assert(state.vanillaApi !== null, "Catalog must be loaded");
  assert(state.vanillaApi.symbols.length >= 2155, "Must have all vanilla symbols");

  // 1. Test Category Filtering
  console.log("Testing Category Filtering...");
  state.apiActiveCategory = 'create';
  vm.runInContext('renderApiSymbolsList()', vmContext);
  const countEl = domRegistry['api-symbol-count'];
  console.log("  Filtered 'create' count:", countEl.textContent);
  assert(countEl.textContent.includes('629'), "Expected 629 Creation symbols");

  // Test Rendering Category
  state.apiActiveCategory = 'draw';
  vm.runInContext('renderApiSymbolsList()', vmContext);
  console.log("  Filtered 'draw' count:", countEl.textContent);
  assert(countEl.textContent.includes('53'), "Expected 53 Rendering symbols");

  // Test Conditions Category
  state.apiActiveCategory = 'check';
  vm.runInContext('renderApiSymbolsList()', vmContext);
  console.log("  Filtered 'check' count:", countEl.textContent);
  assert(parseInt(countEl.textContent) >= 50, "Expected at least 50 Check symbols");

  // 2. Test File Scoping with Categories
  console.log("Testing File Scoping + Category...");
  state.apiActiveFile = 'includes/mapBuild.lua';
  state.apiActiveCategory = 'create';
  vm.runInContext('renderApiSymbolsList()', vmContext);
  console.log("  mapBuild.lua + create count:", countEl.textContent);
  assert(countEl.textContent.includes('163'), "mapBuild.lua should have 163 Creation symbols");

  // 3. Test Action Sub-Derivation Filtering
  console.log("Testing Action Sub-Filter...");
  state.apiActiveAction = 'place';
  vm.runInContext('renderApiSymbolsList()', vmContext);
  console.log("  mapBuild.lua + create + action:place count:", countEl.textContent);
  assert(parseInt(countEl.textContent) > 0, "Should have place* functions in mapBuild.lua");

  // 4. Test Sorting by Name
  console.log("Testing Sorting...");
  state.apiActiveAction = 'all';
  state.apiSortBy = 'name';
  state.apiSortDir = 'asc';
  vm.runInContext('renderApiSymbolsList()', vmContext);
  const listHtml = domRegistry['api-symbols-list'].innerHTML;
  assert(listHtml.includes('api-symbol-item'), "List should contain rendered items");

  // 5. Test Sorting by Category with Group Headers
  state.apiSortBy = 'category';
  state.apiGroupByCat = true;
  state.apiActiveCategory = 'all';
  vm.runInContext('renderApiSymbolsList()', vmContext);
  const groupedHtml = domRegistry['api-symbols-list'].innerHTML;
  assert(groupedHtml.includes('api-symbol-group-header'), "Should render category group divider headers");

  // 6. Test Zero Emojis in Output
  const emojiPattern = /\p{Extended_Pictographic}/u;
  const match = groupedHtml.match(emojiPattern);
  if (match) {
    console.error("EMOJI FOUND IN OUTPUT:", match[0], "codePoint:", match[0].codePointAt(0).toString(16));
  }
  assert(!emojiPattern.test(groupedHtml), "Rendered symbols list must contain 0 emojis");

  console.log("\nALL CATEGORY, SORTING & EMOJI TESTS PASSED CLEANLY!");
})();
