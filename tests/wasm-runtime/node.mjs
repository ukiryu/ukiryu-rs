import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {createRequire} from 'node:module';
import {check, checkEmitter} from './cases.mjs';
import {createHash} from 'node:crypto';
import {loadEscape, checkEscape} from './escape.mjs';
const require = createRequire(import.meta.url);
for (const target of ['node', 'web']) {
  const bytes = await readFile(`wasm-${target}/ukiryu_bg.wasm`);
  const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes));
  console.log(target, JSON.stringify(imports));
  assert(imports.every(x => x.module === 'wbg' || x.module === '__wbindgen_placeholder__' || x.module.startsWith('./ukiryu')), 'unexpected host/C-runtime import');
}
const module = require('../../wasm-node/ukiryu.js');
console.log(`Node ${process.version}: ${check(module)} shared-wire checks passed`);

const escapeRows = await loadEscape(path => readFile(path, 'utf8'), text => createHash('sha256').update(text).digest('hex'));
console.log(`Node escape: ${JSON.stringify(checkEscape(module, escapeRows))}`);

const profile = await readFile('corpus/conformance/inputs/profiles/inkscape_1.0.yaml', 'utf8');
const golden = await readFile('ukiryu/tests/conformance/emitter/yaml.roundtrip.profiles-inkscape-1-0.yaml', 'utf8');
console.log(`Node emitter: ${checkEmitter(module, profile, golden)} checks passed`);

const {loadModelSnapshots, checkModelSnapshots} = await import('./model-snapshot.mjs');
const snapshots = await loadModelSnapshots(path => readFile(path, 'utf8'));
console.log(`Node model snapshots: ${checkModelSnapshots(module, snapshots)} frozen rows passed`);
