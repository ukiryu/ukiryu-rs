#!/usr/bin/env python3
"""Exercise all sealed escape rows through an actual C library, freeing each reply."""
import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import sys

parser = argparse.ArgumentParser()
parser.add_argument('--library', type=Path)
args = parser.parse_args()
filename = ('ukiryu_ffi.dll' if sys.platform == 'win32' else
            'libukiryu_ffi.dylib' if sys.platform == 'darwin' else 'libukiryu_ffi.so')
path = (args.library or Path('target/debug') / filename).resolve()
library = ctypes.CDLL(str(path))
library.ukiryu_json.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
library.ukiryu_json.restype = ctypes.c_void_p
library.ukiryu_free.argtypes = [ctypes.c_void_p]
library.ukiryu_free.restype = None
calls = frees = 0


def request(value):
    global calls, frees
    raw = json.dumps(value, ensure_ascii=False).encode('utf-8')
    pointer = library.ukiryu_json(raw, len(raw))
    calls += 1
    assert pointer, 'missing owned reply'
    try:
        return json.loads(ctypes.string_at(pointer))
    finally:
        library.ukiryu_free(pointer)
        frees += 1


manifest = json.loads(Path('corpus/conformance/manifest.json').read_bytes())
provenance = json.loads(Path('tests/escape-wire/provenance.json').read_bytes())
ids = set()
failures = []
coverage = {}
methods = set()
for index, source in enumerate(['corpus/conformance/cases/escape.jsonl',
                                'tests/escape-wire/supplemental.jsonl']):
    raw = Path(source).read_bytes()
    expected_hash = (manifest['cases']['files']['cases/escape.jsonl'] if index == 0
                     else provenance['sha256'])
    assert hashlib.sha256(raw).hexdigest() == expected_hash, source
    count = 0
    # Split only LF: Unicode line separators inside JSON strings are data.
    for line in raw.split(b'\n'):
        if not line:
            continue
        row = json.loads(line)
        assert row['id'] not in ids, row['id']
        ids.add(row['id'])
        if index == 0:
            assert row['kind'] == 'escape'
            assert row['applicability'] == {'platforms': ['linux', 'macos', 'windows'], 'requires': []}
            assert row['op'].startswith('shell.')
            method = row['op'][6:]
            assert method in ['escape', 'quote', 'needs_quoting', 'format_path', 'env_var', 'join']
            methods.add(method)
            operation = dict(row['input'])
            shell = operation.pop('shell')
            coverage[shell] = coverage.get(shell, 0) + 1
            operation['method'] = method
            value = {'op': 'escape', 'shell': shell, 'operation': operation}
            assert set(row['expect']) == {'ok'}, row['id']
            expected = row['expect']['ok']
        else:
            value = row['request']
            expected = row['value']
        actual = request(value)
        if (actual.get('ok') is not True or type(actual.get('value')) is not type(expected)
                or actual.get('value') != expected):
            failures.append({'id': row['id'], 'actual': actual, 'expected': expected})
        count += 1
    assert count == (manifest['cases']['by_kind']['escape'] if index == 0 else provenance['rows'])
assert len(coverage) == 8 and set(coverage.values()) == {1577}
assert len(methods) == 6
for shell in ['', 'unix', 'windows', 'pwsh', 'mybash', '/bin/mybash', 'BASH', 'bash.exe']:
    actual = request({'op': 'escape', 'shell': shell, 'operation': {'method': 'escape', 'string': 'x'}})
    assert actual['ok'] is False and actual['error']['code'] == 'UNKNOWN_SHELL', actual
    assert actual['error']['source_class'] == 'Ukiryu::Errors::UnknownShellError', actual
assert calls == frees
print(json.dumps({'source_cases': len(ids), 'coverage': coverage, 'methods': sorted(methods),
                  'calls': calls, 'owned_replies_freed': frees, 'library': str(path),
                  'failures': len(failures), 'first_failures': failures[:12]}, ensure_ascii=True))
assert not failures, f'{len(failures)} escape failures'
