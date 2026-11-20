// Runs the qualified consumer through the INSTALLED npm package, including
// its shipped Node and browser WASM. No source build or checkout dist/.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFile} from 'node:fs/promises';
import {resolve, sep} from 'node:path';
import {pathToFileURL} from 'node:url';

const [packageDir, consumerDir, corpusDir, version, coreRef] = process.argv.slice(2);
const pkg = resolve(packageDir);
const readJson = async path => JSON.parse(await readFile(path, 'utf8'));
assert.equal((await readJson(resolve(pkg, 'package.json'))).version, version);
const lockHash = createHash('sha256').update(await readFile('Cargo.lock')).digest('hex');
for (const name of ['ukiryu-wasm', 'ukiryu-web']) {
  const vendor = resolve(pkg, 'vendor', name);
  const marker = await readJson(resolve(vendor, 'BUILT_FROM'));
  assert.equal(marker.core_commit, coreRef, `${name} core revision`);
  assert.equal(marker.crate_version, version, `${name} core version`);
  assert.equal(marker.lock_sha256, lockHash, `${name} core lock`);
  for (const [file, expected] of Object.entries(marker.files)) {
    // npm omits .gitignore. Every runtime payload and notice must be present.
    if (file === '.gitignore') continue;
    const target = resolve(vendor, file);
    assert(target.startsWith(vendor + sep), `unsafe marker path: ${file}`);
    assert.equal(createHash('sha256').update(await readFile(target)).digest('hex'), expected, file);
  }
}
const api = await import(pathToFileURL(resolve(pkg, 'dist/node.js')));
const {runCorpus} = await import(pathToFileURL(resolve(consumerDir, 'test/conformance/corpus.mjs')));
const result = await runCorpus({
  engine: await api.load(), api,
  load: file => readFile(resolve(corpusDir, file), 'utf8'),
  digest: async text => createHash('sha256').update(text, 'utf8').digest('hex'),
});
console.log(`Published npm ${version} (Node WASM): ${JSON.stringify(result)}`);
// Exercise the independently shipped browser WASM without a source build.
const browser = await import(pathToFileURL(resolve(pkg, 'dist/web.js')));
const browserResult = await runCorpus({
  engine: await browser.load({wasm: await readFile(resolve(pkg, 'vendor/ukiryu-web/ukiryu_bg.wasm'))}),
  api: browser,
  load: file => readFile(resolve(corpusDir, file), 'utf8'),
  digest: async text => createHash('sha256').update(text, 'utf8').digest('hex'),
});
console.log(`Published npm ${version} (browser WASM in Node): ${JSON.stringify(browserResult)}`);
