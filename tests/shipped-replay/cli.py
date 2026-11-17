#!/usr/bin/env python3
"""Replay the corpus `cli-output` rows against a built CLI binary.

The binary is the executable extracted from the downloaded release archive,
never a cargo build of the source tree. Every row must match exactly (stdout,
stderr, exit status) after the corpus host substitutions. There is no skip
path: an unexpected row shape, a count that differs from the pin or a single
mismatch fails the run.

One row is compared against a different expectation than the captured one:
`cli-output.version` prints the implementation's own version. The captured
row keeps the Ruby gem's version; the replay expects the Rust workspace
version instead. See DIVERGENCES.md next to this script.
"""
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

PINNED_ROWS = 21  # cli-output.jsonl: every row is Linux, cli+process+fs.
VERSION_ROW = 'cli-output.version'
VERSION_LINE = re.compile(r'\x1b\[36mUkiryu version (\S+)\x1b\[0m\n')


def workspace_version(root):
    """The `[workspace.package] version` of the Cargo workspace the binary was built from."""
    text = (root / 'Cargo.toml').read_text()
    section = re.search(r'^\[workspace\.package\]\n(.*?)(?=^\[|\Z)', text, re.M | re.S)
    assert section, 'Cargo.toml has no [workspace.package] section'
    found = re.search(r'^version\s*=\s*"([^"]+)"', section.group(1), re.M)
    assert found, 'Cargo.toml [workspace.package] has no version'
    return found.group(1)


def expected(row, version):
    """The row's captured expectation, with the version row re-pointed at the Rust version."""
    want = dict(row['expect']['ok'])
    if row['id'] == VERSION_ROW:
        captured = VERSION_LINE.fullmatch(want['stdout'])
        assert captured, f'{VERSION_ROW}: captured stdout has an unexpected shape'
        want['stdout'] = want['stdout'].replace(captured.group(1), version)
    return want


def main(binary, corpus):
    binary, corpus = Path(binary).resolve(), Path(corpus).resolve()
    version = workspace_version(Path(__file__).resolve().parents[2])
    manifest = json.loads((corpus / 'manifest.json').read_text())
    assert manifest['cases']['by_kind']['cli-output'] == PINNED_ROWS, 'manifest count differs from the pin'
    rows = [json.loads(l) for l in (corpus / 'cases/cli-output.jsonl').read_text().splitlines() if l.strip()]
    assert len(rows) == PINNED_ROWS, f'{len(rows)} rows, pinned {PINNED_ROWS}'
    assert len({r['id'] for r in rows}) == len(rows), 'duplicate row id'
    assert VERSION_ROW in {r['id'] for r in rows}, f'{VERSION_ROW} missing'
    inputs = str(corpus / 'inputs')
    failures = []
    for row in rows:
        assert row['op'] == 'cli.run' and set(row['input']) == {'argv'} and set(row['expect']) == {'ok'}, row['id']
        assert row['applicability'] == {'platforms': ['linux'], 'requires': ['cli', 'process', 'fs']}, row['id']
        with tempfile.TemporaryDirectory() as home:
            env = {
                'PATH': os.environ['PATH'], 'HOME': home, 'TZ': 'UTC', 'LANG': 'C.UTF-8', 'LC_ALL': 'C.UTF-8',
                'SHELL': '/bin/bash', 'UKIRYU_REGISTER': f'{inputs}/register', 'THOR_COLUMNS': '100',
                'COLUMNS': '100', 'TERM': 'dumb',
            }
            argv = [a.replace('${INPUTS}', inputs) for a in row['input']['argv']]
            done = subprocess.run([str(binary), *argv], env=env, capture_output=True, timeout=60, cwd=home)

            def norm(data):
                return data.decode('utf-8').replace(home, '${HOME}').replace(inputs, '${INPUTS}')

            want = expected(row, version)
            got = {'stdout': norm(done.stdout), 'stderr': norm(done.stderr), 'exit': done.returncode}
            if got != {'stdout': want['stdout'], 'stderr': want['stderr'], 'exit': want['exit']}:
                failures.append((row['id'], want, got))
    for id_, want, got in failures:
        print(f'MISMATCH {id_}\n  want {json.dumps(want)[:400]}\n  got  {json.dumps(got)[:400]}')
    print(f'cli replay: {len(rows) - len(failures)}/{len(rows)} rows exact against {binary.name}')
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main(*sys.argv[1:3]))
