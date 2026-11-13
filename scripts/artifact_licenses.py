#!/usr/bin/env python3
"""Carry and verify complete Onigmo notices in built runtime artifacts."""
import argparse
from functools import lru_cache
from pathlib import Path
import subprocess


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


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('stage', 'verify'))
    parser.add_argument('directories', nargs='+', type=Path)
    args = parser.parse_args()
    for directory in args.directories:
        (stage if args.action == 'stage' else verify)(directory)
