#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const assert = require('assert');

console.log('[Test] Running Mod Engine Navigation Suite...');

const htmlPath = path.join(__dirname, '../ui/index.html');
const html = fs.readFileSync(htmlPath, 'utf8');

const jsPath = path.join(__dirname, '../ui/app.js');
const js = fs.readFileSync(jsPath, 'utf8');

// 1. Check Nav tab label
assert(
  html.includes('<button type="button" class="nav-tab" data-tab="devtools">\n        <span>Mod Engine</span>') ||
  html.includes('data-tab="devtools"><span>Mod Engine</span>') ||
  html.match(/data-tab="devtools"[\s\S]*?<span>Mod Engine<\/span>/),
  'Main navigation tab must display "Mod Engine"'
);

// 2. Check redundant panel header removed per user request
assert(
  !html.includes('<h2 class="panel-title">Mod Engine</h2>'),
  'Redundant panel header in #tab-devtools must be removed per user request'
);

// 3. Check Subtabs
assert(
  html.includes('data-modtab="studio"'),
  'Must include subtab for Patch Studio (data-modtab="studio")'
);
assert(
  html.includes('data-modtab="editor"'),
  'Must include subtab for Code Editor (data-modtab="editor")'
);
assert(
  html.includes('data-modtab="tutorials"'),
  'Must include subtab for Templates & Tutorials (data-modtab="tutorials")'
);

// 4. Check Subpanels
assert(
  html.includes('id="modtools-tab-studio"'),
  'Must contain #modtools-tab-studio panel'
);
assert(
  html.includes('id="modtools-tab-editor"'),
  'Must contain #modtools-tab-editor panel'
);
assert(
  html.includes('id="modtools-tab-tutorials"'),
  'Must contain #modtools-tab-tutorials panel'
);

// 5. Check app.js handles targetModTab === "studio" and "tutorials"
assert(
  js.includes('targetModTab === "studio"'),
  'app.js must handle targetModTab === "studio"'
);
assert(
  js.includes('targetModTab === "tutorials"'),
  'app.js must handle targetModTab === "tutorials"'
);

console.log('[Test] ALL Mod Engine Navigation tests PASSED!');
