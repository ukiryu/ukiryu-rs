#!/usr/bin/env python3
"""Capture CLI lookup output from a clean ukiryu ab6df353 checkout.

Usage: capture-cli-lookup.py ORACLE --ruby RUBY [--output PATH]
RUBY may be a session-local wrapper for the pinned Ruby 4.0.1 bundle.
Only temporary fixtures and the requested output are written.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

PIN = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
ROOT = Path(__file__).resolve().parents[1]
TESTS = ROOT / 'ukiryu-cli/tests'
CASES = [
    ('which_exact', ['which', 'beta'], ['beta'], {}),
    ('which_unavailable', ['which', 'zeta'], [], {}),
    ('which_alias', ['which', 'zt'], ['zeta'], {}),
    ('which_alias_without_profiles', ['which', 'zt'], ['zeta'], {}),
    ('which_interface_without_profiles', ['which', 'reader/1.0'], ['zeta'], {'zeta': 'reader/1.0'}),
    ('which_interface', ['which', 'reader/1.0'], ['zeta'], {'zeta': 'reader/1.0'}),
    ('which_prefers_available', ['which', 'reader/1.0'], ['zeta'], {'beta': 'reader/1.0', 'zeta': 'reader/1.0'}),
    ('which_no_available', ['which', 'reader/1.0'], [], {'beta': 'reader/1.0', 'zeta': 'reader/1.0'}),
    ('which_missing', ['which', 'missing'], [], {}),
    ('which_platform', ['which', 'zeta', '--platform', 'windows'], [], {}),
    ('which_shell', ['which', 'zeta', '--shell', 'sh'], ['zeta'], {}),
    ('which_unknown_platform', ['which', 'zeta', '--platform', 'unknown'], [], {}),
    ('which_unknown_shell', ['which', 'zeta', '--shell', 'unknown'], [], {}),
    ('list_orphan', ['list'], ['beta', 'zeta'], {}),
    ('list_interface_fallback', ['list'], ['beta', 'zeta'], {'beta': 'zeta/1.0'}),
    ('exec_help_missing', ['exec', 'missing', 'help'], [], {}),
    ('exec_default_missing', ['exec', 'missing'], [], {}),
]

def prepare(root, row):
    fixture = 'execute_data/fixture' if row['id'].startswith('exec_') else 'inspect_data'
    if fixture == 'inspect_data':
        shutil.copytree(TESTS / fixture / 'register', root / 'register')
    else:
        shutil.copytree(TESTS / fixture, root, dirs_exist_ok=True)
    for directory in ['home', 'xdg', 'sysdata', 'bin']:
        (root / directory).mkdir(exist_ok=True)
    for name in row['executables']:
        path = root / 'bin' / name
        path.write_text('#!/bin/sh\n')
        path.chmod(0o755)
    for name, interface in row['implements'].items():
        path = next((root / 'register/tools' / name / 'default').glob('*.yaml'))
        path.write_text(path.read_text() + '\nimplements: ' + interface + '\n')
    for name in row['profiles']:
        path = next((root / 'register/tools' / name / 'default').glob('*.yaml'))
        yaml = path.read_text()
        path.write_text(yaml + '\nprofiles:' + yaml.split('execution_profiles:', 1)[1])
    if row['id'].startswith('list_'):
        (root / 'register/tools/zeta/index.yaml').write_text(
            "---\nukiryu_schema: '1.0'\nname: zeta\ninterface: zeta\nimplementations: []\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('oracle', type=Path)
    parser.add_argument('--ruby', default='ruby')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    oracle = args.oracle.resolve()
    git = lambda *words: subprocess.check_output(['git', *words], cwd=oracle, text=True).strip()
    assert git('rev-parse', 'HEAD') == PIN, 'wrong oracle revision'
    assert not git('status', '--porcelain'), 'oracle checkout is dirty'
    version = subprocess.check_output([args.ruby, '--version'], text=True).strip()
    assert version.startswith('ruby 4.0.1 '), version
    rows = []
    for name, argv, executables, implements in CASES:
        profiles = ['beta', 'zeta'] if name in ('which_alias', 'which_interface', 'which_prefers_available', 'which_no_available') else []
        row = dict(id=name, args=argv, executables=executables, implements=implements, profiles=profiles)
        with tempfile.TemporaryDirectory(prefix='ukiryu-lookup-') as temporary:
            root = Path(temporary)
            prepare(root, row)
            env = {key: value for key, value in os.environ.items() if key.startswith(('BUNDLE_', 'GEM_'))}
            env.update(HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'xdg'),
                       XDG_DATA_DIRS=str(root / 'sysdata'), PATH=f'{root}/bin:/usr/bin:/bin',
                       SHELL='/bin/bash', UKIRYU_REGISTER=str(root / 'register'),
                       TERM='dumb', TZ='UTC', LANG='C.UTF-8')
            result = subprocess.run([args.ruby, '-I' + str(oracle / 'lib'), str(oracle / 'exe/ukiryu'), *argv],
                                    env=env, cwd=root, text=True, input='', capture_output=True, timeout=40)
            assert 'bundler/' not in result.stderr, result.stderr
            row.update(status=result.returncode, stdout=result.stdout.replace(str(root), '{root}'),
                       stderr=result.stderr.replace(str(root), '{root}'))
            rows.append(row)
    assert rows
    # Metadata has no clock or machine path: the payload must survive a new
    # wall clock, locale, cwd and temporary directory unchanged.
    source = dict(repository='ukiryu/ukiryu', revision=PIN, ruby=version.split(' (')[0],
                  lockfile_sha256=hashlib.sha256((oracle / 'Gemfile.lock').read_bytes()).hexdigest(),
                  generator_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())
    args.output.write_text(json.dumps(dict(source=source, cases=rows), indent=2) + '\n')
    print(f'captured {len(rows)} lookup rows')

if __name__ == '__main__':
    main()
