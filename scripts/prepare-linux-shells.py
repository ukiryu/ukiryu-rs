#!/usr/bin/env python3
"""Provision optional CI shells in RUNNER_TEMP without a system install."""
import hashlib
import json
import os
import getpass
import pathlib
import subprocess

root = pathlib.Path(os.environ['RUNNER_TEMP']) / 'ukiryu-shells'
packages = root / 'packages'
prefix = root / 'root'
packages.mkdir(parents=True, exist_ok=True)
prefix.mkdir(parents=True, exist_ok=True)
# Runner images can have stale package indexes (yielding archive 404s). Refresh
# signed Ubuntu indexes in a private state/cache, without root or global hooks.
state = root / 'apt-state'
cache = root / 'apt-cache'
for directory in [state / 'lists/partial', cache / 'archives/partial', root / 'apt-log']:
    directory.mkdir(parents=True, exist_ok=True)
sources = root / 'sources.list'
sources.write_text(''.join(
    f'deb [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] {url} {suite} main universe\n'
    for url, suite in [('https://archive.ubuntu.com/ubuntu', 'noble'),
                       ('https://archive.ubuntu.com/ubuntu', 'noble-updates'),
                       ('https://security.ubuntu.com/ubuntu', 'noble-security')]))
apt = ['apt-get', '-o', 'Dir::Etc::main=/dev/null', '-o', 'Dir::Etc::parts=-',
       '-o', f'Dir::Etc::sourcelist={sources}', '-o', 'Dir::Etc::sourceparts=-',
       '-o', f'Dir::State={state}', '-o', 'Dir::State::status=/var/lib/dpkg/status',
       '-o', f'Dir::Cache={cache}', '-o', f'Dir::Log={root / "apt-log"}',
       '-o', f'APT::Sandbox::User={getpass.getuser()}']
subprocess.run(apt + ['update'], check=True)
# apt download only fetches archives. dpkg-deb extracts into this job's private
# prefix; neither command installs packages or runs package maintainer scripts.
subprocess.run(apt + ['download', 'fish', 'tcsh', 'zsh', 'zsh-common',
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
binpaths = [str(prefix / 'usr/bin'), str(prefix / 'bin')]
env['PATH'] = os.pathsep.join(binpaths + [env['PATH']])
env['LD_LIBRARY_PATH'] = os.pathsep.join(libpaths + ([env['LD_LIBRARY_PATH']] if env.get('LD_LIBRARY_PATH') else []))
for shell in ['bash', 'dash', 'zsh', 'fish', 'tcsh']:
    # Command execution proves this private prefix's binaries can actually load.
    subprocess.run([shell, '-c', 'echo ukiryu-shell-ready'], env=env, check=True)
with open(os.environ['GITHUB_PATH'], 'a') as stream:
    for directory in binpaths:
        stream.write(directory + '\n')
with open(os.environ['GITHUB_ENV'], 'a') as stream:
    stream.write('LD_LIBRARY_PATH=' + env['LD_LIBRARY_PATH'] + '\n')
    stream.write('UKIRYU_REQUIRE_SHELL_TESTS=1\n')
(root / 'packages.json').write_text(json.dumps(receipts, indent=2) + '\n')
print(json.dumps({'prefix': str(prefix), 'packages': receipts,
                  'shell_execution_checks': ['bash', 'dash', 'zsh', 'fish', 'tcsh']}))
