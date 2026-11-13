#!/usr/bin/env python3
"""Compare a fresh capture with the committed golden file.

The oracle header's host fields (Ruby build string, host, bash version)
describe the capturing machine and are not compared.

Usage: compare.py FRESH.json COMMITTED.json
"""
import json
import sys

HOST_FIELDS = ('ruby', 'host', 'bash')


def load(path):
    with open(path) as f:
        doc = json.load(f)
    for field in HOST_FIELDS:
        doc.get('oracle', {}).pop(field, None)
    return doc


def first_difference(a, b, path='$'):
    if type(a) is not type(b):
        return path
    if isinstance(a, dict):
        for key in sorted(set(a) | set(b)):
            if key not in a or key not in b:
                return f'{path}.{key}'
            found = first_difference(a[key], b[key], f'{path}.{key}')
            if found:
                return found
        return None
    if isinstance(a, list):
        if len(a) != len(b):
            return f'{path} (length {len(a)} != {len(b)})'
        for i, (x, y) in enumerate(zip(a, b)):
            found = first_difference(x, y, f'{path}[{i}]')
            if found:
                return found
        return None
    return None if a == b else path


fresh, committed = load(sys.argv[1]), load(sys.argv[2])
diff = first_difference(fresh, committed)
if diff:
    print(f'capture differs from {sys.argv[2]} at {diff}')
    sys.exit(1)
print(f'capture matches {sys.argv[2]}')
