// Data mapping and comparison only: every formatting result comes from core.
export async function loadEscape(load, digest) {
  const paths = ['corpus/conformance/cases/escape.jsonl', 'tests/escape-wire/supplemental.jsonl'];
  const manifest = JSON.parse(await load('corpus/conformance/manifest.json'));
  const provenance = JSON.parse(await load('tests/escape-wire/provenance.json'));
  const rows = [];
  const ids = new Set();
  const shells = new Map();
  const methods = new Set();
  const fail = message => { throw new Error(message); };
  for (const [index, path] of paths.entries()) {
    const text = await load(path);
    const hash = await digest(text);
    const expectedHash = index === 0 ? manifest.cases.files['cases/escape.jsonl'] : provenance.sha256;
    if (hash !== expectedHash) fail(`source hash mismatch: ${path}`);
    let count = 0;
    for (const line of text.split('\n')) {
      if (!line) continue;
      const row = JSON.parse(line);
      if (ids.has(row.id)) fail(`duplicate ID ${row.id}`);
      ids.add(row.id);
      if (index === 0) {
        if (row.kind !== 'escape' || row.applicability.requires.length || JSON.stringify(row.applicability.platforms) !== '["linux","macos","windows"]') fail(`unhandled applicability ${row.id}`);
        const method = row.op.startsWith('shell.') ? row.op.slice(6) : '';
        if (!['escape','quote','needs_quoting','format_path','env_var','join'].includes(method) || !Object.hasOwn(row.expect, 'ok')) fail(`unhandled source operation/error ${row.id}`);
        const {shell, ...input} = row.input;
        shells.set(shell, (shells.get(shell) ?? 0) + 1);
        methods.add(method);
        rows.push({id: row.id, request: {op:'escape', shell, operation:{method,...input}}, value:row.expect.ok});
      } else rows.push(row);
      count++;
    }
    if (count !== (index === 0 ? manifest.cases.by_kind.escape : provenance.rows)) fail(`missing source rows ${path}`);
  }
  if (shells.size !== 8 || methods.size !== 6 || [...shells.values()].some(n => n !== 1577)) fail('incomplete eight-shell/six-method coverage');
  return rows;
}

export function checkEscape(module, rows) {
  const failures = [];
  for (const row of rows) {
    const actual = JSON.parse(module.process(JSON.stringify(row.request)));
    if (actual.ok !== true || typeof actual.value !== typeof row.value || actual.value !== row.value) failures.push({id:row.id, actual, expected:row.value});
  }
  for (const shell of ['', 'unix', 'windows', 'pwsh', 'mybash', '/bin/mybash', 'BASH', 'bash.exe']) {
    const actual = JSON.parse(module.process(JSON.stringify({op:'escape',shell,operation:{method:'escape',string:'x'}})));
    const basename = shell.split('/').at(-1);
    const message = ['unix','windows'].includes(basename) ? `Shell platform group is not a concrete shell: ${basename}` : `Unknown shell: ${basename}`;
    if (actual.ok !== false || actual.error?.code !== 'UNKNOWN_SHELL' || actual.error?.source_class !== 'Ukiryu::Errors::UnknownShellError' || actual.error?.message !== message) failures.push({shell, actual});
  }
  let malformedControls = 0;
  for (const operation of [{method:'escape',string:false},{method:'join',executable:'x',args:[1]},{method:'quote',string:'x',extra:1},{method:'quote',string:'x',for_exe:null},{method:'execute',string:'x'},{method:'escape'}]) {
    const actual = JSON.parse(module.process(JSON.stringify({op:'escape',shell:'bash',operation})));
    if (actual.ok !== false || actual.error?.code !== 'INVALID_REQUEST') failures.push({operation,actual});
    malformedControls++;
  }
  for (const shell of ['bash','zsh','fish','sh','dash','tcsh','powershell','cmd']) {
    const row = rows.find(row => row.request.shell === shell && row.request.operation.method === 'quote');
    const actual = JSON.parse(module.process(JSON.stringify({...row.request,shell:'/tools/' + shell})));
    if (actual.ok !== true || actual.value !== row.value) failures.push({shell,actual,expected:row.value});
  }
  if (failures.length) throw new Error(`${failures.length} escape failures: ${JSON.stringify(failures.slice(0,12))}`);
  return {sourceCases:rows.length, unknownShellControls:8, malformedControls, basenameControls:8};
}
