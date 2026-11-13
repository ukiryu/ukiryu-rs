#!/usr/bin/env python3
"""Stage or verify a GHA-built native C library, header, licenses and provenance."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import artifact_licenses


def output(*args):
    return subprocess.check_output(args, text=True).strip()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stage(folder):
    # Never remove/replace an existing output directory.
    folder.mkdir(parents=True, exist_ok=False)
    names = {'win32': 'ukiryu_ffi.dll', 'darwin': 'libukiryu_ffi.dylib', 'linux': 'libukiryu_ffi.so'}
    name = names[sys.platform]
    shutil.copy2(Path('target/release') / name, folder / name)
    shutil.copy2('ukiryu-ffi/include/ukiryu.h', folder / 'ukiryu.h')
    shutil.copy2('LICENSE', folder / 'LICENSE')
    artifact_licenses.stage(folder)
    metadata = json.loads(output('cargo', 'metadata', '--locked', '--format-version', '1'))
    packages = metadata['packages']
    core = next(p for p in packages if p['name'] == 'ukiryu')
    engine = next(p for p in packages if p['name'] == 'yeptris')
    engine_root = Path(engine['manifest_path']).parent
    ref = engine_root / 'vendor/yeptris/ENGINE_REF'
    # Include source-package license texts rather than relying on SPDX labels.
    dependencies = []
    for package in packages:
        if not package['source']:
            continue
        root = Path(package['manifest_path']).parent
        license_paths = [p for p in root.iterdir() if p.is_file()
                         and p.name.lower().startswith(('license', 'copying', 'copyright'))]
        destination = folder / 'licenses' / f'{package["name"]}-{package["version"]}'
        if license_paths:
            destination.mkdir(parents=True)
            for path in license_paths:
                shutil.copy2(path, destination / path.name)
        dependencies.append({'name': package['name'], 'version': package['version'],
                             'source': package['source'], 'license': package['license']})
    vendor_license = engine_root / 'vendor/yeptris/LICENSE.md'
    shutil.copy2(vendor_license, folder / 'licenses' / 'yeptris-engine-LICENSE.md')
    marker = {
        'core_commit': output('git', 'rev-parse', 'HEAD'),
        'crate_version': core['version'],
        'abi_version': 1,
        'profile': 'release',
        'library': name,
        'rust': output('rustc', '--version', '--verbose'),
        'build_platform': platform.platform(),
        'machine': platform.machine(),
        'compatibility': 'Tested on the recorded GHA runner; no wheel/gem platform compatibility tag asserted',
        'lock_sha256': digest(Path('Cargo.lock')),
        'yeptris_source': engine['source'],
        'engine_ref': ref.read_text().strip(),
        'engine_ref_sha256': digest(ref),
        'dependencies': dependencies,
        'files': {str(p.relative_to(folder)).replace('\\', '/'): digest(p)
                  for p in sorted(folder.rglob('*')) if p.is_file()},
    }
    (folder / 'BUILT_FROM').write_text(json.dumps(marker, indent=2) + '\n', encoding='utf-8')
    print(folder / name)


def verify(folder):
    marker = json.loads((folder / 'BUILT_FROM').read_text(encoding='utf-8'))
    assert re.fullmatch(r'[0-9a-f]{40}', marker['core_commit']), 'invalid source SHA'
    assert re.fullmatch(r'[0-9a-f]{64}', marker['lock_sha256']), 'invalid lock hash'
    assert marker['abi_version'] == 1 and marker['profile'] == 'release'
    required = {marker['library'], 'ukiryu.h', 'LICENSE', 'licenses/yeptris-engine-LICENSE.md'}
    required |= artifact_licenses.REQUIRED
    assert required <= marker['files'].keys(), 'required artifact file missing from manifest'
    actual = {str(p.relative_to(folder)).replace('\\', '/') for p in folder.rglob('*') if p.is_file()}
    assert actual == set(marker['files']) | {'BUILT_FROM'}, 'artifact file set differs from manifest'
    for name, expected in marker['files'].items():
        path = folder / name
        assert path.resolve().is_relative_to(folder.resolve()), 'unsafe manifest path'
        assert digest(path) == expected, f'hash mismatch: {name}'
    artifact_licenses.verify(folder)
    print(f'{folder}: {len(marker["files"])} files verified at {marker["core_commit"]}')


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('action', choices=('stage', 'verify'))
parser.add_argument('directory', type=Path)
parser.add_argument('--expected-count', type=int)
args = parser.parse_args()
if args.action == 'stage':
    stage(args.directory)
else:
    # A glob of downloaded per-platform directories may be passed as their parent.
    folders = [args.directory] if (args.directory / 'BUILT_FROM').exists() else sorted(args.directory.iterdir())
    assert folders, 'no artifacts found'
    if args.expected_count is not None:
        assert len(folders) == args.expected_count, f'expected {args.expected_count} artifacts, got {len(folders)}'
    for folder in folders:
        assert folder.is_dir(), f'unexpected download entry: {folder}'
        verify(folder)
