import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {chromium} from 'playwright';
import assert from 'node:assert/strict';
const files = new Map([
  ['/probe.js', ['regexp-wasm-web/ukiryu_regexp_onigmo_wasm.js', 'text/javascript']],
  ['/ukiryu_regexp_onigmo_wasm_bg.wasm', ['regexp-wasm-web/ukiryu_regexp_onigmo_wasm_bg.wasm', 'application/wasm']],
  ['/check.mjs', ['tests/regexp-onigmo-wasm/check.mjs', 'text/javascript']],
  ['/cases.jsonl', ['ukiryu/tests/regexp_onigmo_data/cases.jsonl', 'text/plain; charset=utf-8']],
  ['/types.jsonl', ['ukiryu/tests/types_corpus/regexp_match.jsonl', 'text/plain; charset=utf-8']],
  ['/versions.json', ['ukiryu/tests/version_detection_bytes/cases.json', 'application/json']],
  ['/resolve.jsonl', ['corpus/conformance/cases/resolve.jsonl', 'text/plain; charset=utf-8']],
  ['/dates.json', ['tests/regexp-onigmo-wasm/date_parse.json', 'application/json']],
]);
const server = createServer(async (req, res) => {
  try {
    if (req.url === '/') {res.end('<!doctype html><title>Ukiryu Onigmo WASM</title>'); return;}
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
    const module = await import('/probe.js');
    await module.default();
    const {check, parseData} = await import('/check.mjs');
    const text = async path => (await fetch(path)).text();
    const data = parseData(await text('/cases.jsonl'), await text('/types.jsonl'), await text('/dates.json'), await text('/versions.json'), await text('/resolve.jsonl'));
    return {count: check(module, data), repeated: check(module, data)};
  });
  assert.equal(result.count, result.repeated);
  console.log(`Chromium ${browser.version()}: ${result.count} Onigmo checks passed`);
} finally {
  await browser?.close();
  server.close();
}
