#!/usr/bin/env python3
"""Write source/toolchain/lock and artifact hashes after an actual GHA build."""
import hashlib
import json
from pathlib import Path
import subprocess


def output(*args):
    return subprocess.check_output(args, text=True).strip()


for folder in (Path('wasm-node'), Path('wasm-web')):
    provenance = {
        'core_commit': output('git', 'rev-parse', 'HEAD'),
        'crate_version': next(p['version'] for p in json.loads(output('cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'))['packages'] if p['name'] == 'ukiryu'),
        'rust': output('rustc', '--version', '--verbose'),
        'wasm_pack': output('wasm-pack', '--version'),
        'wasm_bindgen': '0.2.105',
        'lock_sha256': hashlib.sha256(Path('Cargo.lock').read_bytes()).hexdigest(),
        'files': {str(p.relative_to(folder)): hashlib.sha256(p.read_bytes()).hexdigest()
                  for p in sorted(folder.rglob('*')) if p.is_file() and p.name != 'BUILT_FROM'},
    }
    assert any(name.endswith('.wasm') for name in provenance['files'])
    assert any(name.endswith('.d.ts') for name in provenance['files'])
    (folder / 'BUILT_FROM').write_text(json.dumps(provenance, indent=2) + '\n')
