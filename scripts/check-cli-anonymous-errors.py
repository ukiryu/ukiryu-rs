#!/usr/bin/env python3
"""Re-measure the four anonymous-options CLI errors with the pinned Ruby oracle.

Usage: check-cli-anonymous-errors.py ORACLE --ruby RUBY --output PATH
RUBY may be a session-local wrapper selecting Ruby 4.0.1 and its bundle.
Raw observations/provenance go to PATH; tracked expectations are never written.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

PIN = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
IDS = {'exec_register_unknown_param', 'runfile_execute',
       'runfile_execute_json', 'runfile_execute_table'}
ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / 'ukiryu-cli/tests/execute_data'


def canonical(text):
    # Exactly the anonymous-class inspect token, not arbitrary hexadecimal.
    return re.sub(r'#<Class:0x[0-9a-fA-F]+>', '#<Class:0x<address>>', text)


def stdout_value(text):
    text = canonical(text)
    # JSON whitespace has no meaning. The YAML observations retain Psych's
    # original formatting; this script compares the same Ruby emitter.
    if text.startswith('{'):
        return json.loads(text)
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('oracle', type=Path)
    parser.add_argument('--ruby', default='ruby')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    oracle = args.oracle.resolve()
    git = lambda *words: subprocess.check_output(['git', *words], cwd=oracle, text=True).strip()
    assert git('rev-parse', 'HEAD') == PIN, 'wrong oracle revision'
    assert not git('status', '--porcelain'), 'dirty oracle'
    version = subprocess.check_output([args.ruby, '--version'], text=True).strip()
    assert version.startswith('ruby 4.0.1 '), version
    cases = json.loads((DATA / 'cases.json').read_text())['cases']
    selected = [case for case in cases if case['id'] in IDS]
    assert len(selected) == len(IDS)
    rows = []
    for case in selected:
        # Different processes, roots, timezone and ambient locale; CLI locale
        # remains its captured UTF-8 contract, and all address bytes stay raw.
        for timezone, locale in [('UTC', 'C.UTF-8'), ('Asia/Tokyo', 'C')]:
            with tempfile.TemporaryDirectory(prefix='ukiryu-anonymous-') as temporary:
                root = Path(temporary)
                shutil.copytree(DATA / 'fixture', root, dirs_exist_ok=True)
                for name in ['home', 'xdg', 'sysdata']:
                    (root / name).mkdir(exist_ok=True)
                env = {k: v for k, v in os.environ.items() if k.startswith(('BUNDLE_', 'GEM_'))}
                env.update(HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'xdg'),
                           XDG_DATA_DIRS=str(root / 'sysdata'), PATH='/usr/bin:/bin',
                           SHELL='/bin/bash', UKIRYU_REGISTER=str(root / 'register'),
                           TERM='dumb', TZ=timezone, LANG='C.UTF-8', LC_ALL=locale)
                env.update(case['env'])
                env = {k: v for k, v in env.items() if v is not None}
                result = subprocess.run([args.ruby, '-I' + str(oracle / 'lib'),
                                         str(oracle / 'exe/ukiryu'), *case['args']],
                                        cwd=root, env=env, input=case['stdin'] or '',
                                        text=True, capture_output=True, timeout=40)
                actual = dict(status=result.returncode,
                              stdout=result.stdout.replace(str(root), '{root}'),
                              stderr=result.stderr.replace(str(root), '{root}'))
                matches = (actual['status'] == case['status']
                           and stdout_value(actual['stdout']) == stdout_value(case['stdout'])
                           and canonical(actual['stderr']) == canonical(case['stderr']))
                rows.append(dict(id=case['id'], timezone=timezone, locale=locale,
                                 **actual, matches=matches))
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    receipt = dict(oracle=PIN, ruby=version,
                   lockfile_sha256=digest(oracle / 'Gemfile.lock'),
                   generator_sha256=digest(Path(__file__)),
                   cases_sha256=digest(DATA / 'cases.json'),
                   observations=rows)
    args.output.write_text(json.dumps(receipt, indent=2) + '\n')
    mismatches = [row['id'] for row in rows if not row['matches']]
    print(f'{len(rows)} observations, {len(mismatches)} mismatches: {mismatches}')
    return bool(mismatches)


if __name__ == '__main__':
    raise SystemExit(main())
