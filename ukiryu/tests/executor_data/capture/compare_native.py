#!/usr/bin/env python3
"""Check a fresh Ruby capture against the committed golden file.

Rows must be identical (every field, in order). The oracle block is
reported, not compared, except for the pinned source SHA.
Usage: compare_native.py FRESH.json GOLDEN.json
"""
import json
import sys

fresh = json.load(open(sys.argv[1], encoding='utf-8'))
print(json.dumps(fresh['oracle'], indent=2))
try:
    golden = json.load(open(sys.argv[2], encoding='utf-8'))
except FileNotFoundError:
    sys.exit(f'golden file {sys.argv[2]} is missing; commit the fresh capture')
if fresh['oracle']['sha'] != golden['oracle']['sha']:
    sys.exit('oracle SHA differs')
if fresh['rows'] != golden['rows']:
    for a, b in zip(fresh['rows'], golden['rows']):
        if a != b:
            print('fresh :', json.dumps(a))
            print('golden:', json.dumps(b))
    sys.exit(f"rows differ ({len(fresh['rows'])} fresh, {len(golden['rows'])} golden)")
print(f"{len(fresh['rows'])} rows identical to {sys.argv[2]}")
