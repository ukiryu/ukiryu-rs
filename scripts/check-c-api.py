#!/usr/bin/env python3
"""Load the actual produced library and exercise its C ABI as a clean consumer."""
import argparse
import ctypes
import json
import pathlib
import sys

parser = argparse.ArgumentParser()
parser.add_argument('--library', type=pathlib.Path)
args = parser.parse_args()
filename = ('ukiryu_ffi.dll' if sys.platform == 'win32' else
            'libukiryu_ffi.dylib' if sys.platform == 'darwin' else 'libukiryu_ffi.so')
path = (args.library or pathlib.Path('target/debug') / filename).resolve()
library = ctypes.CDLL(str(path))
library.ukiryu_abi_version.restype = ctypes.c_uint32
library.ukiryu_json.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
library.ukiryu_json.restype = ctypes.c_void_p
library.ukiryu_free.argtypes = [ctypes.c_void_p]
library.ukiryu_free.restype = None
assert library.ukiryu_abi_version() == 1
calls = 0
frees = 0


def request(raw):
    global calls, frees
    pointer = library.ukiryu_json(raw, len(raw))
    calls += 1
    assert pointer, 'missing owned reply'
    try:
        return json.loads(ctypes.string_at(pointer))
    finally:
        library.ukiryu_free(pointer)
        frees += 1


assert request(b'{"op":"version"}') == {'ok': True, 'version': '0.1.0', 'abi_version': 1}
assert request(b'broken')['error']['code'] == 'INVALID_REQUEST'
assert request(b'\xff')['error']['code'] == 'INVALID_REQUEST'
assert request(b'{"op":"parse_yaml","yaml":"name: [broken"}')['error']['code'] == 'YAML_PARSE_ERROR'
value = request(json.dumps({'op': 'parse_yaml', 'yaml': 'version: "1.0"\nflag: false\nrange: [0.0, 1.0]\n'}).encode())
assert value['ok'] and value['value']['version'] == '1.0'
assert value['value']['flag'] is False
assert all(type(x) is float for x in value['value']['range'])
for yaml in ['.nan', '.inf', '-.inf', 'nested: [.nan]', '1: number-key', 'true: bool-key']:
    rejected = request(json.dumps({'op': 'parse_yaml', 'yaml': yaml}).encode())
    assert rejected['ok'] is False, (yaml, rejected)
    assert rejected['error']['code'] == 'YAML_NOT_JSON_COMPATIBLE', (yaml, rejected)
quoted = request(json.dumps({'op': 'parse_yaml', 'yaml': "literal: '.nan'"}).encode())
assert quoted['ok'] and quoted['value']['literal'] == '.nan'
configuration = {
    'op': 'resolve_config',
    'environment': {'UKIRYU_TIMEOUT': '999999999999999999999999999999999999999',
                    'UKIRYU_DEBUG': 'true', 'UKIRYU_USE_COLOR': 'true', 'NO_COLOR': ''},
    'programmatic': {'debug': None, 'output': 'Ω\x00value'},
    'cli': {'debug': False},
}
raw = json.dumps(configuration, ensure_ascii=False).encode()
for _ in range(1000):
    value = request(raw)
    assert value['ok'] and value['values']['debug'] is False
    assert value['values']['use_color'] is False
    assert value['values']['output'] == 'Ω\x00value'
    assert value['values']['timeout'] == 999999999999999999999999999999999999999
    assert value['sources']['debug'] == 'cli'
library.ukiryu_free(None)
assert calls == frees
print(json.dumps({'abi_version': 1, 'calls': calls, 'owned_replies_freed': frees,
                  'library': str(path), 'status': 'passed'}))
