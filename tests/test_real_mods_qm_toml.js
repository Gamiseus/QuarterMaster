const assert = require('assert');
const path = require('path');
const fs = require('fs');
const vm = require('vm');

console.log("=== RUNNING REAL MODS QM.TOML EMPIRICAL LINTER TEST ===");

const APP_JS_PATH = path.resolve(__dirname, '..', 'ui', 'app.js');
const appJsSource = fs.readFileSync(APP_JS_PATH, 'utf8');

// Ensure Zero Emojis
assert(!/\p{Extended_Pictographic}/u.test(appJsSource), "app.js must contain 0 emojis");

const sandbox = {
  console,
  document: { getElementById: () => null, querySelectorAll: () => [], addEventListener: () => {} },
  window: { addEventListener: () => {} },
  state: { editorActiveFilePath: null }
};

const vmContext = vm.createContext(sandbox);
vm.runInContext(appJsSource, vmContext);

const lintSourceDocument = sandbox.lintSourceDocument;
assert(typeof lintSourceDocument === 'function', "lintSourceDocument must be available");

const modsDir = path.resolve(__dirname, '..', '..', 'MnB-Mods');
const modFolders = fs.readdirSync(modsDir);

let totalTested = 0;
for (const folder of modFolders) {
  const qmPath = path.join(modsDir, folder, 'qm.toml');
  if (fs.existsSync(qmPath)) {
    totalTested++;
    const content = fs.readFileSync(qmPath, 'utf8');
    const res = lintSourceDocument('qm.toml', content);

    console.log(`Testing ${folder}/qm.toml:`);
    if (res.errors.length > 0) {
      console.error(`  [FAIL] ${res.errors.length} errors found:`);
      res.errors.forEach(e => console.error(`    Line ${e.line}: ${e.message}`));
    }
    if (res.warnings.length > 0) {
      console.warn(`  [WARN] ${res.warnings.length} warnings found:`);
      res.warnings.forEach(w => console.warn(`    Line ${w.line}: ${w.message}`));
    }

    assert.strictEqual(res.errors.length, 0, `${folder}/qm.toml must have 0 errors`);
    assert.strictEqual(res.warnings.length, 0, `${folder}/qm.toml must have 0 warnings`);
    console.log(`  [PASS] ${folder}/qm.toml verified with 0 errors and 0 warnings`);
  }
}

assert(totalTested >= 7, `Expected at least 7 real mods tested, got ${totalTested}`);
console.log(`=== ALL ${totalTested} REAL MOD QM.TOML FILES PASSED VALIDATION CLEANLY ===`);
