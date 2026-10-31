const assert = require('assert');
const path = require('path');
const fs = require('fs');
const vm = require('vm');

console.log("=== RUNNING EDITOR AUTOCOMPLETE UNIT TESTS ===");

const APP_JS_PATH = path.resolve(__dirname, '..', 'ui', 'app.js');
const VANILLA_REF_PATH = path.resolve(__dirname, '..', 'ui', 'vanilla_engine_reference.json');

const appJsSource = fs.readFileSync(APP_JS_PATH, 'utf8');
const vanillaRefData = JSON.parse(fs.readFileSync(VANILLA_REF_PATH, 'utf8'));

// Ensure Zero Emojis
assert(!/\p{Extended_Pictographic}/u.test(appJsSource), "app.js must contain 0 emojis");

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
    this.selectionStart = 0;
    this.selectionEnd = 0;
    this.scrollTop = 0;
    this.clientHeight = 500;
    this.clientWidth = 800;
    this.disabled = false;
    this.listeners = {};
  }
  get innerHTML() { return this._innerHTML; }
  set innerHTML(val) { this._innerHTML = val; }
  querySelectorAll() { return []; }
  addEventListener(evt, fn) {
    this.listeners[evt] = this.listeners[evt] || [];
    this.listeners[evt].push(fn);
  }
  dispatchEvent(evt) {
    const list = this.listeners[evt.type] || [];
    list.forEach(fn => fn(evt));
  }
  focus() {}
}

['editor-mod-select', 'mod-files-tree', 'mod-file-editor-textarea', 'editor-file-badge',
 'editor-dirty-indicator', 'editor-line-numbers', 'editor-validation-status',
 'editor-problems-counter', 'editor-problems-list', 'editor-problems-file',
 'editor-problems-panel', 'btn-toggle-problems', 'editor-autocomplete-box'].forEach(id => {
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
    return {
      ok: true,
      status: 200,
      json: async () => vanillaRefData,
      text: async () => JSON.stringify(vanillaRefData)
    };
  },
  showToast: () => {}
};

const vmContext = vm.createContext(sandbox);
vm.runInContext(appJsSource, vmContext);

// Pre-load vanilla API into state
const state = vm.runInContext('state', vmContext);
state.vanillaApi = vm.runInContext('normalizeVanillaReferenceCatalog', vmContext)(vanillaRefData);
assert(state.vanillaApi.symbols.length > 2000, "Vanilla symbols should be > 2000");

const getAutocompleteSuggestions = sandbox.getAutocompleteSuggestions;
assert(typeof getAutocompleteSuggestions === 'function', "getAutocompleteSuggestions function must be defined");

// ==========================================
// 1. LUA AUTOCOMPLETE TESTS
// ==========================================
console.log("Checking Lua Autocomplete...");

// 1.1 Match vanilla API function: 'createGrunt'
const luaSuggestions1 = getAutocompleteSuggestions("script.lua", "local d = createG", 17);
assert(luaSuggestions1.length > 0, "Should suggest symbols matching 'createG'");
assert(luaSuggestions1.some(s => s.name === "createGrunt" && s.type === "func"), "Should suggest 'createGrunt' function");
console.log("  [PASS] Lua vanilla API function suggestion ('createGrunt')");

// 1.2 Match vanilla class: 'player.'
const luaSuggestions2 = getAutocompleteSuggestions("script.lua", "player.", 7);
assert(luaSuggestions2.length > 0, "Should suggest player class fields for 'player.'");
assert(luaSuggestions2.some(s => s.name.startsWith("player.")), "Should suggest 'player.deployedTroops' etc.");
console.log("  [PASS] Lua class member suggestion ('player.*')");

// 1.3 Match Lua keyword / snippet: 'func'
const luaSuggestions3 = getAutocompleteSuggestions("script.lua", "func", 4);
assert(luaSuggestions3.some(s => s.name === "function" || s.type === "snippet"), "Should suggest 'function' keyword/snippet");
console.log("  [PASS] Lua keyword / snippet suggestion ('function')");

// ==========================================
// 2. TOML AUTOCOMPLETE TESTS
// ==========================================
console.log("Checking TOML Autocomplete...");

// 2.1 Match patch target file: '@game/map'
const tomlSuggestions1 = getAutocompleteSuggestions("qm.toml", 'target = "@game/map', 19);
assert(tomlSuggestions1.length > 0, "Should suggest game files matching '@game/map'");
assert(tomlSuggestions1.some(s => s.insertText.includes("mapBuild.lua")), "Should suggest '@game/mapBuild.lua'");
console.log("  [PASS] TOML @game/ target file suggestion ('mapBuild.lua')");

// 2.2 Match TOML patch keys: 'patt'
const tomlSuggestions2 = getAutocompleteSuggestions("qm.toml", 'patt', 4);
assert(tomlSuggestions2.some(s => s.name.includes("pattern")), "Should suggest 'pattern' key");
console.log("  [PASS] TOML key suggestion ('pattern')");

// ==========================================
// 3. JSON AUTOCOMPLETE TESTS
// ==========================================
console.log("Checking JSON Autocomplete...");

// 3.1 Match mod.json manifest fields: '"lo'
const jsonSuggestions1 = getAutocompleteSuggestions("mod.json", '  "lo', 5);
assert(jsonSuggestions1.some(s => s.name.includes("load_priority")), "Should suggest 'load_priority' field");
console.log("  [PASS] JSON manifest field suggestion ('load_priority')");

console.log("=== ALL AUTOCOMPLETE UNIT TESTS PASSED ===");
