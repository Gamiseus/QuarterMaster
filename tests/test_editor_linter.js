const assert = require('assert');
const path = require('path');
const fs = require('fs');
const vm = require('vm');

console.log("=== RUNNING EDITOR LINTER UNIT TESTS ===");

const APP_JS_PATH = path.resolve(__dirname, '..', 'ui', 'app.js');
const appJsSource = fs.readFileSync(APP_JS_PATH, 'utf8');

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
    this.disabled = false;
    this.listeners = {};
  }
  get innerHTML() { return this._innerHTML; }
  set innerHTML(val) { this._innerHTML = val; }
  querySelectorAll(sel) { return []; }
  addEventListener(evt, fn) {
    this.listeners[evt] = this.listeners[evt] || [];
    this.listeners[evt].push(fn);
  }
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
  fetch: async () => ({ ok: true, json: async () => ({}), text: async () => "{}" }),
  showToast: () => {}
};

const vmContext = vm.createContext(sandbox);
vm.runInContext(appJsSource, vmContext);

const lintSourceDocument = sandbox.lintSourceDocument;
assert(typeof lintSourceDocument === 'function', "lintSourceDocument function must be exported/available");

// ==========================================
// 1. LUA LINTER TESTS
// ==========================================
console.log("Checking Lua Linter...");

// 1.1 Valid Lua
const luaValid = `
local function calculateBonus(score)
  if score > 100 then
    return score * 2
  else
    return score
  end
end
`;
const resLua1 = lintSourceDocument("bonus.lua", luaValid);
assert.strictEqual(resLua1.errors.length, 0, "Valid Lua should have 0 errors");
console.log("  [PASS] Valid Lua: 0 errors");

// 1.2 Unclosed function block
const luaUnclosedFunc = `
function spawnDude(x, y)
  local dude = {}
  dude.x = x
  dude.y = y
`;
const resLua2 = lintSourceDocument("spawn.lua", luaUnclosedFunc);
assert(resLua2.errors.length > 0, "Unclosed function should report error");
assert(resLua2.errors[0].message.includes("function"), "Error should mention unclosed function");
assert(resLua2.errors[0].explanation && resLua2.errors[0].explanation.length > 10, "Error must have explainable rationale");
assert(resLua2.errors[0].fixSuggestion, "Error must have actionable fix suggestion");
console.log("  [PASS] Lua unclosed function caught with explainable rationale");

// 1.3 Unclosed bracket / delimiter
const luaUnclosedBracket = `
local tbl = { 1, 2, 3
local x = 5
`;
const resLua3 = lintSourceDocument("syntax.lua", luaUnclosedBracket);
assert(resLua3.errors.length > 0, "Unclosed table brace should report error");
assert(resLua3.errors[0].message.includes("{"), "Error should report unclosed '{'");
console.log("  [PASS] Lua unclosed delimiter caught");

// 1.4 Lua Pitfall: != operator warning
const luaOperatorWarning = `
if a != b then
  print("not equal")
end
`;
const resLua4 = lintSourceDocument("compare.lua", luaOperatorWarning);
assert(resLua4.warnings.some(w => w.message.includes("~=")), "Should warn that Lua uses '~=' instead of '!='");
console.log("  [PASS] Lua '!=' operator warning caught");

// ==========================================
// 2. TOML LINTER TESTS
// ==========================================
console.log("Checking TOML Linter...");

// 2.1 Valid qm.toml
const tomlValid = `
[metadata]
name = "My Mod"
version = "1.0.0"

[[patches]]
target = "@game/main.lua"
pattern = "love.window.setTitle('Mud and Blood')"
replacement = "love.window.setTitle('Mud and Blood [Modded]')"
position = "at"
`;
const resToml1 = lintSourceDocument("qm.toml", tomlValid);
assert.strictEqual(resToml1.errors.length, 0, "Valid qm.toml should have 0 errors");
console.log("  [PASS] Valid TOML: 0 errors");

// 2.2 TOML Duplicate table header
const tomlDuplicateHeader = `
[metadata]
name = "Mod 1"

[metadata]
name = "Mod 2"
`;
const resToml2 = lintSourceDocument("config.toml", tomlDuplicateHeader);
assert(resToml2.errors.length > 0, "Duplicate table header should produce error");
assert(resToml2.errors[0].message.includes("Duplicate table header"), "Error message should note duplicate header");
console.log("  [PASS] TOML duplicate table header caught");

// 2.3 qm.toml patch missing target
const tomlMissingTarget = `
[[patches]]
pattern = "test"
replacement = "test2"
`;
const resToml3 = lintSourceDocument("qm.toml", tomlMissingTarget);
assert(resToml3.warnings.some(w => w.message.includes("target")), "Missing target in [[patches]] should produce warning/error");
console.log("  [PASS] qm.toml missing target warning caught");

// 2.4 qm.toml invalid position
const tomlInvalidPos = `
[[patches]]
target = "@game/main.lua"
pattern = "test"
replacement = "test2"
position = "sideways"
`;
const resToml4 = lintSourceDocument("qm.toml", tomlInvalidPos);
assert(resToml4.errors.some(e => e.message.includes("position")), "Invalid patch position should produce error");
console.log("  [PASS] qm.toml invalid position caught");

// ==========================================
// 3. JSON LINTER TESTS
// ==========================================
console.log("Checking JSON Linter...");

// 3.1 Valid mod.json
const jsonValid = JSON.stringify({
  id: "super_tactics",
  name: "Super Tactics",
  version: "1.2.0",
  load_priority: 50
}, null, 2);
const resJson1 = lintSourceDocument("mod.json", jsonValid);
assert.strictEqual(resJson1.errors.length, 0, "Valid mod.json should have 0 errors");
console.log("  [PASS] Valid mod.json: 0 errors");

// 3.2 Trailing comma in JSON
const jsonTrailingComma = `{
  "id": "my_mod",
  "name": "My Mod",
  "version": "1.0.0",
}`;
const resJson2 = lintSourceDocument("mod.json", jsonTrailingComma);
assert(resJson2.errors.length > 0, "Trailing comma should produce JSON error");
assert(resJson2.errors[0].explanation.includes("comma") || resJson2.errors[0].message.includes("comma") || resJson2.errors[0].message.includes("JSON"), "Should explain trailing comma / syntax issue");
console.log("  [PASS] JSON trailing comma caught with explanation");

// 3.3 Missing required 'id' in mod.json
const jsonMissingId = `{
  "name": "Nameless Mod",
  "version": "1.0.0"
}`;
const resJson3 = lintSourceDocument("mod.json", jsonMissingId);
assert(resJson3.errors.some(e => e.message.includes("'id'")), "Missing required id should produce error");
console.log("  [PASS] mod.json missing 'id' caught");

// ==========================================
// 4. MARKDOWN LINTER TESTS
// ==========================================
console.log("Checking Markdown Linter...");

// 4.1 Valid Markdown
const mdValid = `# Mod Documentation\n\n\`\`\`lua\nlocal x = 1\n\`\`\`\n\nClean text.`;
const resMd1 = lintSourceDocument("README.md", mdValid);
assert.strictEqual(resMd1.errors.length, 0, "Valid Markdown should have 0 errors");
console.log("  [PASS] Valid Markdown: 0 errors");

// 4.2 Unclosed code fence
const mdUnclosedFence = `# Mod Guide\n\n\`\`\`lua\nlocal x = 1\n\nUnclosed code block!`;
const resMd2 = lintSourceDocument("GUIDE.md", mdUnclosedFence);
assert(resMd2.errors.length > 0, "Unclosed code fence should produce error");
assert(resMd2.errors[0].message.includes("code fence"), "Error message should mention code fence");
assert(resMd2.errors[0].explanation, "Error must have explanation");
console.log("  [PASS] Markdown unclosed code fence caught with explanation");

console.log("=== ALL LINTER UNIT TESTS PASSED ===");
