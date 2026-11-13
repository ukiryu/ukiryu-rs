#!/usr/bin/env python3
"""Verify the delivered artifact, including generated hidden files, not build dirs."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import artifact_licenses

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('directory', type=Path)
args = parser.parse_args()
for target in ('node', 'web'):
    folder = args.directory / f'wasm-{target}'
    marker = json.loads((folder / 'BUILT_FROM').read_text())
    assert re.fullmatch(r'[0-9a-f]{40}', marker['core_commit']), 'invalid source commit'
    assert re.fullmatch(r'[0-9a-f]{64}', marker['lock_sha256']), 'invalid lock hash'
    required = {'LICENSE', 'package.json', 'ukiryu.js', 'ukiryu.d.ts', 'ukiryu_bg.wasm'}
    required |= artifact_licenses.REQUIRED
    expected = marker['files']
    assert required <= expected.keys(), f'{target}: missing required manifest entries'
    actual = {str(p.relative_to(folder)) for p in folder.rglob('*') if p.is_file()}
    assert actual == set(expected) | {'BUILT_FROM'}, f'{target}: artifact/manifest file mismatch'
    for name, digest in expected.items():
        path = folder / name
        assert path.resolve().is_relative_to(folder.resolve()), 'invalid artifact path'
        assert hashlib.sha256(path.read_bytes()).hexdigest() == digest, f'{target}: {name} hash mismatch'
    artifact_licenses.verify(folder)
    print(f'{target}: {len(expected)} delivered files verified at {marker["core_commit"]}')
