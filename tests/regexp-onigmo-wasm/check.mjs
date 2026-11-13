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
  if (failures.length) throw new Error(`${failures.length} of ${count} checks failed:\n${failures.join('\n')}`);
  return count;
}

export function parseData(cases, typeRows, dates) {
  const lines = text => text.split('\n').filter(Boolean).map(l => JSON.parse(l));
  return {cases: lines(cases), typeRows: lines(typeRows), dates: JSON.parse(dates)};
}
