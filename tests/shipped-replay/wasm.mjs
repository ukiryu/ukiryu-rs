// Replay the corpus escape rows through the DOWNLOADED wasm package.
// Usage: node tests/shipped-replay/wasm.mjs <downloaded-wasm-dir>
// The module under test is <dir>/wasm-node/ukiryu.js from the artifact; the
// corpus and the row mapping come from the checkout. Counts are pinned and
// there is no skip path: loadEscape fails on any unhandled row.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {createRequire} from 'node:module';
import {resolve} from 'node:path';
import {createHash} from 'node:crypto';
import {check} from '../wasm-runtime/cases.mjs';
import {loadEscape, checkEscape} from '../wasm-runtime/escape.mjs';

const PINNED_ESCAPE_ROWS = 12616 + 5984; // corpus escape + supplemental
const dir = resolve(process.argv[2]);
const module = createRequire(import.meta.url)(resolve(dir, 'wasm-node/ukiryu.js'));
for (const target of ['node', 'web']) {
  const imports = WebAssembly.Module.imports(new WebAssembly.Module(await readFile(resolve(dir, `wasm-${target}/ukiryu_bg.wasm`))));
  assert(imports.every(x => x.module === 'wbg' || x.module === '__wbindgen_placeholder__' || x.module.startsWith('./ukiryu')), `unexpected import in wasm-${target}`);
}
console.log(`shared-wire checks: ${check(module)}`);
const rows = await loadEscape(path => readFile(path, 'utf8'), text => createHash('sha256').update(text).digest('hex'));
const result = checkEscape(module, rows);
assert.equal(result.sourceCases, PINNED_ESCAPE_ROWS, 'escape row count differs from the pin');
console.log(`wasm replay: ${JSON.stringify(result)} against ${dir}`);
