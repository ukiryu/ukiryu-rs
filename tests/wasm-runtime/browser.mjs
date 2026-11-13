import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {chromium} from 'playwright';
import assert from 'node:assert/strict';
const files = new Map([
  ['/ukiryu.js', ['wasm-web/ukiryu.js', 'text/javascript']],
  ['/ukiryu_bg.wasm', ['wasm-web/ukiryu_bg.wasm', 'application/wasm']],
  ['/escape.mjs', ['tests/wasm-runtime/escape.mjs', 'text/javascript']],
  ...['corpus/conformance/cases/escape.jsonl','corpus/conformance/manifest.json','tests/escape-wire/supplemental.jsonl','tests/escape-wire/provenance.json'].map(path => ['/' + path, [path, 'application/json']]),
  ['/cases.mjs', ['tests/wasm-runtime/cases.mjs', 'text/javascript']],
]);
const server = createServer(async (req, res) => {
  try {
    if (req.url === '/') {res.end('<!doctype html><title>Ukiryu WASM</title>'); return;}
    const file = files.get(req.url);
    if (!file) {res.writeHead(404); res.end(); return;}
    res.setHeader('Content-Type', file[1]);
    res.end(await readFile(file[0]));
  } catch (error) {res.writeHead(500); res.end(String(error));}
});
server.listen(0, '127.0.0.1');
await new Promise(resolve => server.once('listening', resolve));
let browser;
try {
  browser = await chromium.launch({headless: true});
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  const result = await page.evaluate(async () => {
    const module = await import('/ukiryu.js');
    const wasm = await module.default();
    const {check} = await import('/cases.mjs');
    const {loadEscape, checkEscape} = await import('/escape.mjs');
    const rows = await loadEscape(async path => { const response = await fetch('/' + path); if (!response.ok) throw new Error(path); return response.text(); }, async text => [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text)))].map(x => x.toString(16).padStart(2,'0')).join(''));
    const escape = checkEscape(module, rows);
    const before = wasm.memory.buffer.byteLength;
    const count = check(module);
    const after = wasm.memory.buffer.byteLength;
    return {before, after, count, repeated: check(module), escape, escapeRepeated: checkEscape(module, rows)};
  });
  assert(result.after > result.before, 'large YAML must exercise WebAssembly memory growth');
  assert.equal(result.count, result.repeated);
  assert.deepEqual(result.escape, result.escapeRepeated);
  console.log(`Chromium ${browser.version()}: ${JSON.stringify(result)}`);
} finally {
  await browser?.close();
  server.close();
}
