#!/usr/bin/env python3
"""Shipped audit: artifact inventory and version consistency, no network, no publishing.

Usage:
  shipped-audit.py                      repository checks (run on every PR)
  shipped-audit.py --version X.Y.Z      also require the workspace version to equal X.Y.Z
"""
import argparse
import json
import pathlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SEMVER = re.compile(r'^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$')
errors = []


def check(ok, msg):
    if not ok:
        errors.append(msg)
    print(('ok    ' if ok else 'FAIL  ') + msg)


def toml(path):
    return tomllib.loads((ROOT / path).read_text())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--version')
    args = ap.parse_args()

    ws = toml('Cargo.toml')
    version = ws['workspace']['package']['version']
    check(bool(SEMVER.match(version)), f'workspace version {version} is semver')
    if args.version:
        check(args.version == version, f'requested version {args.version} equals workspace version {version}')

    # Every member inherits the one workspace version.
    members = ws['workspace']['members']
    for m in members:
        pkg = toml(f'{m}/Cargo.toml')['package']
        check(pkg.get('version') == {'workspace': True}, f'{m} inherits the workspace version')
        check(pkg.get('publish') is False, f'{m} is publish = false (release-ready only, owner publishes)')

    # Lockfile agrees with the manifests.
    lock = toml('Cargo.lock')
    for m in members:
        p = next((p for p in lock['package'] if p['name'] == m), None)
        check(p is not None and p['version'] == version, f'Cargo.lock {m} version equals {version}')

    # Inventory manifest.
    train = json.loads((ROOT / 'release/train.json').read_text())
    ids = [a['id'] for a in train['artifacts']]
    check(len(ids) == len(set(ids)), 'artifact ids are unique')
    check(train['train_version'] is None or train['train_version'] == version,
          'train_version is undecided or equals the workspace version')
    for a in train['artifacts']:
        if a['repo'] == 'ukiryu/ukiryu-rs':
            check(a['source'] and (ROOT / a['source']).exists(), f"{a['id']} source exists")
            wf = a['built_by']
            check((ROOT / '.github/workflows' / wf).exists(), f"{a['id']} builder workflow {wf} exists")
        check(a['published'] is False, f"{a['id']} is not marked published")

    # CLI targets: manifest, packaging script and workflow matrix agree.
    script = (ROOT / 'scripts/cli-binary-artifacts.py').read_text()
    block = re.search(r'TARGETS = \{(.*?)\n\}', script, re.S).group(1)
    script_targets = set(re.findall(r"'([a-z0-9_]+-[a-z0-9_-]+)':", block))
    wf_targets = set(re.findall(r'target: (\S+)', (ROOT / '.github/workflows/cli-binaries.yml').read_text()))
    manifest_targets = set(train['cli_targets'])
    check(script_targets == wf_targets == manifest_targets,
          f'CLI targets agree (script {len(script_targets)}, workflow {len(wf_targets)}, manifest {len(manifest_targets)})')

    # Publication safety: nothing publishes on push/PR/merge.
    for wf in sorted((ROOT / '.github/workflows').glob('*.yml')):
        text = wf.read_text()
        is_train = wf.name == 'release-train.yml'
        publishes = re.findall(r'^\s*(?:run:\s*|-\s*run:\s*)?.*\b(?:cargo publish(?! --dry-run)|gem push|twine upload|npm publish|gh release create)\b', text, re.M)
        if not is_train:
            check(not publishes, f'{wf.name} has no publishing command')
            check('secrets.' not in text, f'{wf.name} uses no secrets')
            check(not re.search(r'contents:\s*write', text), f'{wf.name} has no contents: write')
        else:
            on = re.search(r'^on:\n((?:[ ]+.*\n|\n)+)', text, re.M).group(1)
            check(re.findall(r'^  (\w+):', on, re.M) == ['workflow_dispatch'],
                  'release-train.yml is triggered only by workflow_dispatch')
            check(not publishes, 'release-train.yml has no publishing command (owner adds it after first-release decisions)')
            check('secrets.' not in text, 'release-train.yml uses no secrets')
            check(not re.search(r'git tag|git push|gh release', text), 'release-train.yml creates no tags or releases')

    print()
    if errors:
        print(f'{len(errors)} shipped-audit check(s) failed')
        return 1
    print('shipped audit passed')
    return 0


sys.exit(main())
