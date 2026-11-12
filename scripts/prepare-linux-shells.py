#!/usr/bin/env python3
"""Provision optional CI shells in RUNNER_TEMP without a system install."""
import hashlib
import json
import os
import pathlib
import subprocess

root = pathlib.Path(os.environ['RUNNER_TEMP']) / 'ukiryu-shells'
packages = root / 'packages'
prefix = root / 'root'
packages.mkdir(parents=True, exist_ok=True)
prefix.mkdir(parents=True, exist_ok=True)
# apt download only fetches archives. dpkg-deb extracts into this job's private
# prefix; neither command installs packages or runs package maintainer scripts.
subprocess.run(['apt-get', 'download', 'fish', 'tcsh', 'zsh', 'zsh-common',
                'libpcre2-32-0', 'libpcre2-8-0', 'libtinfo6', 'libncurses6',
                'libcrypt1'], cwd=packages, check=True)
receipts = []
for archive in sorted(packages.glob('*.deb')):
    subprocess.run(['dpkg-deb', '--extract', str(archive), str(prefix)], check=True)
    receipts.append({'archive': archive.name,
                     'sha256': hashlib.sha256(archive.read_bytes()).hexdigest()})
libpaths = [str(path) for path in (prefix / 'usr/lib/x86_64-linux-gnu',
                                 prefix / 'lib/x86_64-linux-gnu') if path.is_dir()]
env = dict(os.environ)
env['PATH'] = str(prefix / 'usr/bin') + os.pathsep + env['PATH']
env['LD_LIBRARY_PATH'] = os.pathsep.join(libpaths + [env.get('LD_LIBRARY_PATH', '')])
for shell in ['bash', 'dash', 'zsh', 'fish', 'tcsh']:
    # Command execution proves this private prefix's binaries can actually load.
    subprocess.run([shell, '-c', 'echo ukiryu-shell-ready'], env=env, check=True)
with open(os.environ['GITHUB_PATH'], 'a') as stream:
    stream.write(str(prefix / 'usr/bin') + '\n')
with open(os.environ['GITHUB_ENV'], 'a') as stream:
    stream.write('LD_LIBRARY_PATH=' + env['LD_LIBRARY_PATH'] + '\n')
    stream.write('UKIRYU_REQUIRE_SHELL_TESTS=1\n')
(root / 'packages.json').write_text(json.dumps(receipts, indent=2) + '\n')
print(json.dumps({'prefix': str(prefix), 'packages': receipts,
                  'shell_execution_checks': ['bash', 'dash', 'zsh', 'fish', 'tcsh']}))
