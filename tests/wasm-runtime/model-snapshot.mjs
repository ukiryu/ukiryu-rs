// Frozen corpus expectations, supplied to the WASM core as text by each host.
export async function loadModelSnapshots(read) {
  const rows = (await read('corpus/conformance/cases/yaml.jsonl')).trim().split('\n').map(JSON.parse);
  return Promise.all(['Interface', 'ImplementationVersion', 'ImplementationIndex'].map(async model => {
    const row = rows.find(r => r.op === 'yaml.model_load' && r.input.model === model);
    if (!row || !row.expect.ok) throw new Error(`missing frozen ${model} snapshot`);
    return {row, yaml: await read(`corpus/conformance/inputs/${row.input.file}`)};
  }));
}

export function checkModelSnapshots(module, examples) {
  if (examples.length !== 3) throw new Error('three model classes must be checked');
  const canonical = value => Array.isArray(value) ? value.map(canonical)
    : value !== null && typeof value === 'object'
      ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])]))
      : value;
  for (const {row, yaml} of examples) {
    const result = JSON.parse(module.process(JSON.stringify({
      op: 'yaml', method: 'model_snapshot', model: row.input.model, yaml,
    })));
    const expected = {ok: true, value: row.expect.ok};
    if (JSON.stringify(canonical(result)) !== JSON.stringify(canonical(expected))) {
      throw new Error(`${row.id}: ${JSON.stringify(result)} != ${JSON.stringify(expected)}`);
    }
  }
  return examples.length;
}
