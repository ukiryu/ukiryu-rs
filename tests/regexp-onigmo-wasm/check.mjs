// Shared by the Node and Chromium runs: every expectation was captured from
// Ruby 4.0.1 (see ukiryu/tests/regexp_onigmo.rs and date_parse.json).
export function check(module, data) {
  const failures = [];
  let count = 0;
  const canonical = value => Array.isArray(value) ? value.map(canonical)
    : value !== null && typeof value === 'object'
      ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])]))
      : value;
  const compare = (id, actual, expected) => {
    count++;
    if (JSON.stringify(canonical(actual)) !== JSON.stringify(canonical(expected))) failures.push(`${id}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
  };
  for (const row of data.cases) {
    const {pattern, options, string} = row.input;
    compare(row.id, JSON.parse(module.regexp_match(pattern, options, string)), row.expect);
  }
  // Type.validate(value, :string, pattern:) rows: ok when String#=~ matches.
  for (const row of data.typeRows) {
    const [source, options] = row.input.pattern.$re;
    const result = JSON.parse(module.regexp_match(source, options, row.input.value));
    compare(row.id, 'match' in result ? result.match !== null : result, 'ok' in row.expect);
  }
  for (const row of data.dates.rows) compare(`Date._parse(${row.input})`, JSON.parse(module.date_parse(row.input)), row.expect);
  const bytes = value => [...new TextEncoder().encode(value)].map(b => b.toString(16).padStart(2,'0')).join('');
  if (data.versions.length !== 663) throw new Error('missing captured version encoding cases');
  for (const row of data.versions) compare(row.id, JSON.parse(module.extract_version(JSON.stringify(row.input))), row.expect);
  let extraction = 0, binary = 0;
  for (const row of data.resolve) {
    if (row.op !== 'resolve.extract_version') continue;
    if (row.applicability.requires.length || !Object.hasOwn(row.expect,'ok')) throw new Error('unhandled version applicability/error');
    const output = row.input.output == null ? null : typeof row.input.output === 'string' ? {hex:bytes(row.input.output),encoding:'UTF-8'} : {hex:row.input.output.$bytes,encoding:'ASCII-8BIT'};
    if (output?.encoding === 'ASCII-8BIT') binary++;
    const value = row.expect.ok;
    const expected = {ok:value == null ? null : typeof value === 'string' ? {hex:bytes(value),encoding:'UTF-8'} : {hex:value.$bytes,encoding:'ASCII-8BIT'}};
    compare(row.id, JSON.parse(module.extract_version(JSON.stringify({output,pattern:row.input.pattern}))), expected);
    extraction++;
  }
  if (extraction !== 372 || binary !== 31) throw new Error(`missing source extraction rows ${extraction}/${binary}`);
  if (failures.length) throw new Error(`${failures.length} of ${count} checks failed:\n${failures.join('\n')}`);
  return count;
}

export function parseData(cases, typeRows, dates, versions, resolve) {
  const lines = text => text.split('\n').filter(Boolean).map(l => JSON.parse(l));
  return {cases: lines(cases), typeRows: lines(typeRows), dates: JSON.parse(dates), versions:JSON.parse(versions), resolve:lines(resolve)};
}
