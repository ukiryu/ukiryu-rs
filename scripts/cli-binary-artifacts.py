#!/usr/bin/env python3
"""Package GHA-built CLI candidates and verify the actual downloaded archives."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import struct
import subprocess
import tarfile
import tempfile
import zipfile


TARGETS = {
    'x86_64-unknown-linux-gnu': ('elf', 62),
    'aarch64-unknown-linux-gnu': ('elf', 183),
    'x86_64-apple-darwin': ('macho', 0x01000007),
    'aarch64-apple-darwin': ('macho', 0x0100000C),
    'x86_64-pc-windows-msvc': ('pe', 0x8664),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def output(*args):
    return subprocess.check_output(args, text=True).strip()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def binary_target(data, target):
    kind, machine = TARGETS[target]
    if kind == 'elf':
        require(data[:6] == b'\x7fELF\x02\x01', 'not a 64-bit little-endian ELF')
        actual = struct.unpack_from('<H', data, 18)[0]
    elif kind == 'macho':
        require(data[:4] == b'\xcf\xfa\xed\xfe', 'not a 64-bit little-endian Mach-O')
        actual = struct.unpack_from('<I', data, 4)[0]
    else:
        require(data[:2] == b'MZ', 'missing DOS signature')
        offset = struct.unpack_from('<I', data, 0x3C)[0]
        require(data[offset:offset + 4] == b'PE\0\0', 'missing PE signature')
        actual = struct.unpack_from('<H', data, offset + 4)[0]
    require(actual == machine, f'binary architecture differs from {target}')


def stage(directory, target):
    directory.mkdir(parents=True, exist_ok=False)
    metadata = json.loads(output('cargo', 'metadata', '--locked', '--format-version', '1'))
    core = next(p for p in metadata['packages'] if p['name'] == 'ukiryu')
    engine = next(p for p in metadata['packages'] if p['name'] == 'yeptris')
    name = 'ukiryu.exe' if target.endswith('msvc') else 'ukiryu'
    binary = Path('target') / target / 'release' / name
    binary_target(binary.read_bytes(), target)
    with tempfile.TemporaryDirectory() as temporary:
        payload = Path(temporary) / f'ukiryu-{core["version"]}-{target}'
        payload.mkdir()
        shutil.copy2(binary, payload / name)
        for filename in ('LICENSE', 'README.md'):
            shutil.copy2(filename, payload / filename)
        dependencies = []
        for package in metadata['packages']:
            if not package['source']:
                continue
            root = Path(package['manifest_path']).parent
            licenses = [p for p in root.iterdir() if p.is_file()
                        and p.name.lower().startswith(('license', 'copying', 'copyright'))]
            destination = payload / 'licenses' / f'{package["name"]}-{package["version"]}'
            if licenses:
                destination.mkdir(parents=True)
                for path in licenses:
                    shutil.copy2(path, destination / path.name)
            dependencies.append({'name': package['name'], 'version': package['version'],
                                 'source': package['source'], 'license': package['license']})
        engine_root = Path(engine['manifest_path']).parent / 'vendor/yeptris'
        (payload / 'licenses').mkdir(exist_ok=True)
        shutil.copy2(engine_root / 'LICENSE.md', payload / 'licenses/yeptris-engine-LICENSE.md')
        marker = {
            'core_commit': output('git', 'rev-parse', 'HEAD'),
            'crate_version': core['version'], 'target': target, 'binary': name,
            'profile': 'release', 'rust': output('rustc', '--version', '--verbose'),
            'lock_sha256': digest(Path('Cargo.lock').read_bytes()),
            'yeptris_source': engine['source'],
            'engine_ref': (engine_root / 'ENGINE_REF').read_text().strip(),
            'dependencies': dependencies,
            'qualification': 'CLI version/help smoke only; full CLI and release-train gates remain separate',
            'compatibility': 'Built and run on the recorded native GHA runner; no older OS baseline asserted',
            'files': {p.relative_to(payload).as_posix(): digest(p.read_bytes())
                      for p in sorted(payload.rglob('*')) if p.is_file()},
        }
        (payload / 'BUILT_FROM').write_text(json.dumps(marker, indent=2) + '\n', encoding='utf-8')
        if target.endswith('msvc'):
            archive = directory / f'{payload.name}.zip'
            with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as handle:
                for path in sorted(payload.rglob('*')):
                    if path.is_file():
                        handle.write(path, f'{payload.name}/{path.relative_to(payload).as_posix()}')
        else:
            archive = directory / f'{payload.name}.tar.gz'
            with tarfile.open(archive, 'w:gz') as handle:
                handle.add(payload, arcname=payload.name)
        (directory / 'SHA256SUMS').write_text(f'{digest(archive.read_bytes())}  {archive.name}\n', encoding='ascii')
        print(archive)


def read_archive(archive):
    members = {}

    def add(name, data):
        path = PurePosixPath(name)
        require(not path.is_absolute() and '..' not in path.parts and '\\' not in name,
                'unsafe archive member')
        require(len(path.parts) >= 2, 'archive files must share a root directory')
        require(name not in members, 'duplicate archive member')
        members[name] = data

    if archive.suffix == '.zip':
        with zipfile.ZipFile(archive) as handle:
            for member in handle.infolist():
                require(not member.is_dir(), 'unexpected zip directory member')
                require((member.external_attr >> 16) & 0o170000 != 0o120000, 'archive symlink')
                add(member.filename, handle.read(member))
    else:
        with tarfile.open(archive, 'r:gz') as handle:
            for member in handle:
                if member.isdir():
                    continue
                require(member.isfile(), 'non-regular archive member')
                add(member.name, handle.extractfile(member).read())
    require(bool(members), 'empty archive')
    roots = {PurePosixPath(name).parts[0] for name in members}
    require(len(roots) == 1, 'multiple archive roots')
    root = roots.pop()
    return root, {name[len(root) + 1:]: data for name, data in members.items()}


def verify_payload(root, members, expected_commit):
    marker = json.loads(members['BUILT_FROM'])
    target = marker['target']
    require(target in TARGETS, 'unknown target')
    require(marker['core_commit'] == expected_commit, 'source commit differs from checkout')
    require(re.fullmatch(r'[0-9a-f]{64}', marker['lock_sha256']), 'invalid lock digest')
    require(marker['lock_sha256'] == digest(Path('Cargo.lock').read_bytes()), 'lock differs from checkout')
    require(marker['profile'] == 'release', 'not a release build')
    require(root == f'ukiryu-{marker["crate_version"]}-{target}', 'archive name/target mismatch')
    require(marker['binary'] == ('ukiryu.exe' if target.endswith('msvc') else 'ukiryu'), 'wrong binary name')
    required = {marker['binary'], 'LICENSE', 'README.md', 'licenses/yeptris-engine-LICENSE.md'}
    require(required <= marker['files'].keys(), 'missing required file')
    require(set(members) == set(marker['files']) | {'BUILT_FROM'}, 'archive file set differs')
    for name, expected in marker['files'].items():
        require(digest(members[name]) == expected, f'file digest differs: {name}')
    binary_target(members[marker['binary']], target)
    return marker


def negative_controls(root, members, commit):
    marker = json.loads(members['BUILT_FROM'])
    damaged = dict(members)
    damaged[marker['binary']] += b'corruption'
    missing = dict(members)
    del missing['licenses/yeptris-engine-LICENSE.md']
    relabeled = dict(members)
    altered = dict(marker)
    altered['target'] = next(t for t in TARGETS if TARGETS[t] != TARGETS[marker['target']])
    relabeled['BUILT_FROM'] = json.dumps(altered).encode()
    for label, candidate in [('damaged binary', damaged), ('missing license', missing),
                             ('relabeled target', relabeled)]:
        try:
            verify_payload(root, candidate, commit)
        except (ValueError, KeyError):
            print(f'rejected {label}')
        else:
            raise ValueError(f'negative control accepted: {label}')


def verify(directory, run, all_targets, negatives):
    folders = sorted(directory.iterdir()) if all_targets else [directory]
    require(len(folders) == (len(TARGETS) if all_targets else 1), 'incorrect artifact count')
    seen = set()
    commit = output('git', 'rev-parse', 'HEAD')
    for folder in folders:
        checksum = (folder / 'SHA256SUMS').read_text(encoding='ascii').strip()
        expected, name = checksum.split('  ')
        require(Path(name).name == name, 'unsafe archive filename')
        require({p.name for p in folder.iterdir()} == {'SHA256SUMS', name}, 'unexpected artifact files')
        archive = folder / name
        require(digest(archive.read_bytes()) == expected, 'archive checksum differs')
        root, members = read_archive(archive)
        marker = verify_payload(root, members, commit)
        require(marker['target'] not in seen, 'duplicate target')
        seen.add(marker['target'])
        if negatives:
            negative_controls(root, members, commit)
        if run:
            with tempfile.TemporaryDirectory() as temporary:
                executable = Path(temporary) / marker['binary']
                executable.write_bytes(members[marker['binary']])
                executable.chmod(0o755)
                version = subprocess.run([str(executable), '--version'], check=True, capture_output=True, timeout=20)
                require(version.stdout == f'ukiryu {marker["crate_version"]}\n'.encode(), 'unexpected CLI version')
                help_result = subprocess.run([str(executable), '--help'], check=True, capture_output=True, timeout=20)
                require(b'Usage:' in help_result.stdout and b'Commands:' in help_result.stdout, 'missing CLI help')
        print(f'{name}: {len(marker["files"])} files verified at {commit}')
    if all_targets:
        require(seen == set(TARGETS), 'target matrix incomplete')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('stage', 'verify'))
    parser.add_argument('--directory', required=True, type=Path)
    parser.add_argument('--target', choices=TARGETS)
    parser.add_argument('--run', action='store_true')
    parser.add_argument('--all-targets', action='store_true')
    parser.add_argument('--negative-controls', action='store_true')
    args = parser.parse_args()
    if args.action == 'stage':
        require(args.target, 'stage requires --target')
        stage(args.directory, args.target)
    else:
        verify(args.directory, args.run, args.all_targets, args.negative_controls)


if __name__ == '__main__':
    main()
