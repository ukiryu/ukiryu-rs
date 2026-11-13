#!/usr/bin/env python3
"""Carry and verify complete Onigmo notices in built runtime artifacts."""
import argparse
from functools import lru_cache
from pathlib import Path
import subprocess
import json


SOURCE = 'ukiryu/vendor/onigmo'
NAMES = ('NOTICE', 'LEGAL', 'COPYING', 'BSDL', 'PROVENANCE')
REQUIRED = {f'licenses/onigmo/{name}' for name in NAMES}


@lru_cache(maxsize=None)
def source_bytes(name):
    # Preserve the repository bytes on every runner, including Windows
    # checkouts with core.autocrlf. These are notices, not transformed data.
    return subprocess.check_output(['git', 'show', f'HEAD:{SOURCE}/{name}'])


def stage(folder):
    destination = folder / 'licenses' / 'onigmo'
    destination.mkdir(parents=True, exist_ok=False)
    for name in NAMES:
        (destination / name).write_bytes(source_bytes(name))
    package = folder / 'package.json'
    if package.exists():
        metadata = json.loads(package.read_text(encoding='utf-8'))
        # wasm-pack's explicit file list would otherwise exclude notices
        # from npm even though they are present in the GHA artifact.
        if 'files' in metadata and 'licenses' not in metadata['files']:
            metadata['files'].append('licenses')
            package.write_text(json.dumps(metadata, indent=2) + '\n', encoding='utf-8')


def verify_contents(contents):
    for name in NAMES:
        key = f'licenses/onigmo/{name}'
        if key not in contents:
            raise ValueError(f'missing required Onigmo notice: {key}')
        if contents[key] != source_bytes(name):
            raise ValueError(f'Onigmo notice differs from source: {key}')


def verify(folder):
    contents = {key: (folder / key).read_bytes() for key in REQUIRED}
    verify_contents(contents)
    # These controls exercise required-file and source-content checks on
    # the delivered bytes without modifying the downloaded artifact.
    missing = dict(contents)
    del missing['licenses/onigmo/NOTICE']
    damaged = dict(contents)
    damaged['licenses/onigmo/LEGAL'] += b'corruption'
    for label, candidate in [('missing NOTICE', missing), ('damaged LEGAL', damaged)]:
        try:
            verify_contents(candidate)
        except ValueError:
            continue
        raise ValueError(f'accepted negative license control: {label}')
    print(f'{folder}: all {len(NAMES)} Onigmo notice files match source; two negatives rejected')


def verify_npm(folder):
    packed = json.loads(subprocess.check_output(
        ['npm', 'pack', '--dry-run', '--json', '--ignore-scripts'], cwd=folder, text=True))
    paths = {entry['path'] for entry in packed[0]['files']}
    if not REQUIRED <= paths:
        raise ValueError(f'npm excludes required Onigmo notices: {REQUIRED - paths}')
    print(f'{folder}: npm dry run includes all {len(NAMES)} Onigmo notice files')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('stage', 'verify'))
    parser.add_argument('--npm', action='store_true', help='also audit npm pack file selection (GHA only)')
    parser.add_argument('directories', nargs='+', type=Path)
    args = parser.parse_args()
    for directory in args.directories:
        (stage if args.action == 'stage' else verify)(directory)
        if args.npm:
            verify_npm(directory)
