#!/usr/bin/env node
const assert = require('assert');
const fs = require('fs');
const path = require('path');

console.log('[Test] Running Latest Fixes & User Feedback Test Suite...');

// 1. Check index.html UI cleanup (zero redundant headers)
const htmlPath = path.join(__dirname, '../ui/index.html');
const html = fs.readFileSync(htmlPath, 'utf8');

assert(!html.includes('id="tab-devtools"') || !html.match(/<div class="panel-header">\s*<h2 class="panel-title">Mod Engine<\/h2>/),
  'Mod Engine tab must NOT have redundant yellow header');

assert(!html.includes('id="tab-library"') || !html.match(/<div class="panel-header">\s*<h2 class="panel-title">Mod Library &amp; Updates<\/h2>/),
  'Library & Updates tab must NOT have redundant yellow header');

// 2. Load app.js in a mocked environment
const jsPath = path.join(__dirname, '../ui/app.js');
const jsCode = fs.readFileSync(jsPath, 'utf8');

const mockWindow = {
  addEventListener: () => {},
  document: {
    addEventListener: () => {},
    getElementById: () => null,
    querySelectorAll: () => []
  }
};
global.window = mockWindow;
global.document = mockWindow.document;

eval(jsCode);

// 3. Test serializePatchToToml schema compliance
const testPatch = {
  id: "tactical_cqc_boost",
  target: "main.lua",
  pattern: "player.cqc = 10",
  position: "after",
  payload: "player.cqc = 25\nplayer.morale = 100",
  matchIndent: true,
  description: "Boosts close quarters combat",
  requires: ["core_lib", "ai_overhaul"]
};

const tomlOutput = global.serializePatchToToml(testPatch);

// Verify triple single quotes
assert(tomlOutput.includes("pattern = '''\nplayer.cqc = 10\n'''"), "Pattern must use triple single quotes");
assert(tomlOutput.includes("payload = '''\nplayer.cqc = 25\nplayer.morale = 100\n'''"), "Payload must use triple single quotes");

// Verify match_indent is strictly after payload
const payloadIndex = tomlOutput.indexOf("payload = '''");
const matchIndentIndex = tomlOutput.indexOf("match_indent = true");
assert(matchIndentIndex > payloadIndex, "match_indent must be positioned after payload");

// Verify requires and description
assert(tomlOutput.includes('id = "tactical_cqc_boost"'), "Must include patch id");
assert(tomlOutput.includes('description = "Boosts close quarters combat"'), "Must include description");
assert(tomlOutput.includes('requires = ["core_lib", "ai_overhaul"]'), "Must include requires array");

// Test regex patch serialization
const regexPatch = {
  id: "regex_test",
  target: "conf.lua",
  pattern: "t\\.window\\.title\\s*=",
  position: "regex",
  payload: 't.window.title = "Mud & Blood Enhanced"',
  matchIndent: false
};
const regexToml = global.serializePatchToToml(regexPatch);
assert(regexToml.includes("[patches.regex]"), "Regex mode must serialize [patches.regex]");

// 4. Test parsePatchesFromQmToml
const parsed = global.parsePatchesFromQmToml(tomlOutput, "TestMod");
assert.strictEqual(parsed.length, 1, "Should parse 1 patch from toml");
assert.strictEqual(parsed[0].id, "tactical_cqc_boost");
assert.strictEqual(parsed[0].target, "main.lua");
assert.strictEqual(parsed[0].position, "after");
assert.deepStrictEqual(parsed[0].requires, ["core_lib", "ai_overhaul"]);

// 5. Test filterSchemaCompliantPatches
const testCandidates = [
  { id: "valid_patch_1", target: "main.lua", pattern: "abc", position: "after" },
  { id: "valid-patch-2", target: "conf.lua", pattern: "xyz", position: "at" },
  { id: "", target: "main.lua", pattern: "abc", position: "at" }, // Missing id
  { id: "invalid spaces", target: "main.lua", pattern: "abc", position: "at" }, // Bad id format
  { id: "missing_target", pattern: "abc", position: "at" }, // Missing target
  { id: "missing_pattern", target: "main.lua", position: "at" }, // Missing pattern
  { id: "bad_pos", target: "main.lua", pattern: "abc", position: "invalid_position" } // Bad position
];

const compliant = global.filterSchemaCompliantPatches(testCandidates);
assert.strictEqual(compliant.length, 2, "Only valid_patch_1 and valid-patch-2 should be compliant");
assert.strictEqual(compliant[0].id, "valid_patch_1");
assert.strictEqual(compliant[1].id, "valid-patch-2");

// 6. Test Editor Anchor Visibility (Line 18 in screenshot)
const dummyLines = [
  '-- line 1',
  '-- line 2',
  '-- line 3',
  '-- line 4',
  '-- line 5',
  '-- line 6',
  '-- line 7',
  '-- line 8',
  '-- line 9',
  '-- line 10',
  '-- line 11',
  '-- line 12',
  '-- line 13',
  '-- line 14',
  '-- line 15',
  '-- line 16',
  '-- line 17',
  'require "includes/menu"', // Line 18
  '-- line 19',
  '-- line 20',
  '-- line 21',
  '-- line 22'
];

global.state.currentSourceLines = dummyLines;
global.state.patchTargetRange = [18, 18];
global.state.patchContextLines = 4;
global.state.patchWizardData = {
  patchId: "test_anchor_visibility",
  position: "after",
  payload: 'print("menu loaded")'
};

const fakePane = { innerHTML: "" };
global.document.getElementById = (id) => {
  if (id === "studio-right-pane") return fakePane;
  return null;
};

global.renderLivePatchEditor();

// Verify Line 18 is present in editor HTML and has ANCHOR badge
assert(fakePane.innerHTML.includes('18</span>'), "Line 18 must be rendered in editor");
assert(fakePane.innerHTML.includes('require &quot;includes/menu&quot;'), "Line 18 content must be rendered");
assert(fakePane.innerHTML.includes('ANCHOR'), "Line 18 must have ANCHOR badge");

// Verify Line 18 appears BEFORE payload input for "after" mode
const anchorPos = fakePane.innerHTML.indexOf('require &quot;includes/menu&quot;');
const inputPos = fakePane.innerHTML.indexOf('id="patch-payload-input"');
assert(anchorPos < inputPos, "For 'after' mode, anchor line 18 must be placed ABOVE the payload textarea");

// Test "before" mode
global.state.patchWizardData.position = "before";
global.renderLivePatchEditor();
const anchorPosBefore = fakePane.innerHTML.indexOf('require &quot;includes/menu&quot;');
const inputPosBefore = fakePane.innerHTML.indexOf('id="patch-payload-input"');
assert(inputPosBefore < anchorPosBefore, "For 'before' mode, payload textarea must be placed ABOVE anchor line 18");

// Test "at" mode
global.state.patchWizardData.position = "at";
global.renderLivePatchEditor();
assert(fakePane.innerHTML.includes('REPLACING'), "For 'at' mode, target line must show REPLACING badge");

// 7. Test Diff Rendering Helper
const diffItems = [
  { type: "context", lineNo: 18, content: 'require "includes/menu"' },
  { type: "add", lineNo: 19, content: 'print("injected")' }
];
const diffHtml = global.renderLiveDiffRowsHtml(diffItems);
assert(diffHtml.includes('ANCHOR'), "Context line in diff must have ANCHOR badge");
assert(diffHtml.includes('diff-add'), "Add line in diff must have diff-add class");
assert(diffHtml.includes('ADDED'), "Add line in diff must have ADDED badge");

console.log('[Test] ALL Latest Fixes & User Feedback tests PASSED!');
