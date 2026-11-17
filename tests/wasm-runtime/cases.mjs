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
  count += checkDocuments(module);
  return count;
}

// The document operations that take their text in the request. Expected
// values are the core's corpus results (lint.synthetic.invalid-yaml).
export function checkDocuments(module) {
  let count = 0;
  const call = request => JSON.parse(module.process(JSON.stringify(request)));
  const equal = (actual, expected, label) => {
    count++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      throw new Error(`${label}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
    }
  };
  const sym = name => ({$sym: name});
  equal(call({op: 'lint', method: 'string', yaml: 'invalid: yaml: content: [unclosed'}), {ok: true, value: {$map: [
    [sym('issues'), [{$map: [
      [sym('severity'), sym('error')],
      [sym('severity_string'), 'ERROR'],
      [sym('message'), 'Invalid YAML: (<unknown>): mapping values are not allowed in this context at line 1 column 14'],
      [sym('location'), null],
      [sym('suggestion'), null],
      [sym('rule_id'), null],
    ]}]],
    [sym('total_count'), 1],
    [sym('error_count'), 1],
    [sym('warning_count'), 0],
    [sym('info_count'), 0],
    [sym('style_count'), 0],
  ]}}, 'lint string');
  equal(call({op: 'yaml', method: 'safe_load', yaml: ':sym: 1.5\n2: [.nan, ~]\nplain: text\n'}), {ok: true, value: {$map: [
    [sym('sym'), {$float: '1.5'}],
    [2, [{$float: 'NaN'}, null]],
    ['plain', 'text'],
  ]}}, 'safe_load typed values');
  const broken = call({op: 'yaml', method: 'safe_load', yaml: 'a: [1'});
  equal([broken.ok, broken.error.source_class, broken.error.detail],
    [false, 'Psych::SyntaxError', {category: 'YamlSyntax', stage: 'document'}], 'safe_load reader error');
  equal(call({op: 'yaml', method: 'model_load', model: 'Interface', path: '/etc/passwd'}).error.code,
    'UNSUPPORTED_CAPABILITY', 'model_load reads files');
  return count;
}

// `model_to_yaml` emits the pinned Yeptris golden of the corpus row
// yaml.roundtrip.profiles-inkscape-1-0, and re-emitting it is stable.
export function checkEmitter(module, profile, golden) {
  const call = request => JSON.parse(module.process(JSON.stringify(request)));
  const first = call({op: 'yaml', method: 'model_to_yaml', model: 'ToolDefinition', yaml: profile});
  if (!first.ok) throw new Error(`model_to_yaml: ${JSON.stringify(first)}`);
  if (first.value !== golden) throw new Error(`model_to_yaml differs from the golden: ${JSON.stringify(first.value)}`);
  const again = call({op: 'yaml', method: 'model_to_yaml', model: 'ToolDefinition', yaml: first.value});
  if (again.value !== golden) throw new Error(`model_to_yaml re-emission differs: ${JSON.stringify(again)}`);
  return 2;
}
