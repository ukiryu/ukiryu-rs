// Shared by actual Node and Chromium consumers. No JS YAML/config implementation.
export function check(module) {
  let count = 0;
  const canonical = value => Array.isArray(value) ? value.map(canonical)
    : value !== null && typeof value === 'object'
      ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])]))
      : value;
  const equal = (actual, expected, label) => {
    count++;
    if (JSON.stringify(canonical(actual)) !== JSON.stringify(canonical(expected))) {
      throw new Error(`${label}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
    }
  };
  const call = request => JSON.parse(module.process(JSON.stringify(request)));
  equal(module.abi_version(), 1, 'ABI version');
  equal(call({op: 'version'}), {abi_version: 1, ok: true, version: module.core_version()}, 'shared wire');
  const yaml = 'message: "日本語 \\u2603"\nrange: [0.0, 1.0]\nflag: false\nempty: null\n';
  const parsed = call({op: 'parse_yaml', yaml});
  if (!parsed.ok) throw new Error(JSON.stringify(parsed));
  equal(parsed.value, {message: '日本語 ☃', range: [0, 1], flag: false, empty: null}, 'vendored YAML');
  const config = call({op: 'resolve_config', environment: {UKIRYU_FORMAT: 'json'}, cli: {format: 'raw'}});
  equal(config.ok, true, 'configuration result');
  equal(config.values.format, 'raw', 'CLI configuration precedence');
  const fromEnv = call({op: 'resolve_config', environment: {UKIRYU_FORMAT: 'json'}});
  equal(fromEnv.values.format, 'json', 'injected environment');
  for (const request of [
    {op: 'parse_yaml', yaml: 'invalid: [unclosed'},
    {op: 'parse_yaml', yaml: '.nan'},
    {op: 'parse_yaml', yaml: '1: value'},
    {op: 'parse_yaml', yaml: '!unregistered value'},
  ]) equal(call(request).ok, false, 'structured YAML error');
  equal(JSON.parse(module.process('{broken')).error.code, 'INVALID_REQUEST', 'invalid JSON');
  equal(call({op: 'execute', command: 'echo should-not-run'}).error.code, 'UNSUPPORTED_CAPABILITY', 'no subprocess capability');
  equal(call({op: 'register', path: '/etc'}).error.code, 'UNSUPPORTED_CAPABILITY', 'no filesystem capability');
  const large = 'λ'.repeat(2 * 1024 * 1024);
  const largeRequest = JSON.stringify({op: 'parse_yaml', yaml: JSON.stringify(large)});
  const result = JSON.parse(module.process(largeRequest));
  equal(result.ok, true, 'large allocation result');
  equal(result.value, large, 'large allocation survives memory growth');
  for (let i = 0; i < 100; i++) equal(call({op: 'parse_yaml', yaml: 'after: growth'}).value, {after: 'growth'}, 'repeated allocation');
  return count;
}
