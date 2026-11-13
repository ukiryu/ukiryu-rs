import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {createRequire} from 'node:module';
import {check, parseData} from './check.mjs';
const require = createRequire(import.meta.url);
const read = path => readFile(path, 'utf8');
for (const target of ['node', 'web']) {
  const bytes = await readFile(`regexp-wasm-${target}/ukiryu_regexp_onigmo_wasm_bg.wasm`);
  const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes));
  console.log(target, JSON.stringify(imports));
  assert(imports.every(x => x.module === 'wbg' || x.module === '__wbindgen_placeholder__' || x.module.startsWith('./ukiryu')), 'unexpected host/C-runtime import');
}
const data = parseData(
  await read('ukiryu/tests/regexp_onigmo_data/cases.jsonl'),
  await read('ukiryu/tests/types_corpus/regexp_match.jsonl'),
  await read('tests/regexp-onigmo-wasm/date_parse.json'));
const module = require('../../regexp-wasm-node/ukiryu_regexp_onigmo_wasm.js');
const count = check(module, data);
assert.equal(check(module, data), count);
console.log(`Node ${process.version}: ${count} Onigmo checks passed`);
