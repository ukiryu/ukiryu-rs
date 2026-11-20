#!/usr/bin/env python3
"""Capture exact malformed-definition CLI output from the clean Ruby oracle.

Run with Ruby 4.0.1 on PATH, e.g. mise x -- python3 this-script ORACLE.
Reuses the original-command boot and offline-schema policy of the author
capture. No CLI source is patched. Captures LF/CRLF, validate/lint and text/JSON.
"""
import argparse
import concurrent.futures
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess

REPO = Path(__file__).resolve().parents[1]
OUTPUT = REPO / 'ukiryu-cli/tests/definition_diagnostics'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('oracle', type=Path)
    args = parser.parse_args()
    oracle = args.oracle.resolve()
    spec = importlib.util.spec_from_file_location('author', REPO / 'scripts/capture-author-reference.py')
    author = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(author)
    author.RUBY = shutil.which('ruby')
    assert author.RUBY, 'Ruby missing'
    git = lambda *args: subprocess.check_output(['git', '-C', str(oracle), *args], text=True).strip()
    assert git('rev-parse', 'HEAD') == author.SHA, 'wrong oracle'
    assert not git('status', '--porcelain'), 'dirty oracle'
    expected_gems = json.loads((author.DATA / 'provenance.json').read_text())['gems'][0]
    # Generated output/provenance may differ; its generator and shared boot may not.
    source_paths = ['scripts/capture-definition-diagnostics.py', 'scripts/capture-author-reference.py']
    subprocess.run(['git', '-C', str(REPO), 'diff', '--exit-code', 'HEAD', '--', *source_paths], check=True)
    source_commit = subprocess.check_output(['git', '-C', str(REPO), 'log', '-1', '--format=%H', '--', *source_paths], text=True).strip()
    os.environ.update(BUNDLE_GEMFILE=str(oracle / 'Gemfile'), BUNDLE_FROZEN='true')
    ruby = subprocess.check_output([author.RUBY, '-e', 'print RUBY_VERSION'], text=True)
    assert ruby == '4.0.1', ruby
    psych = subprocess.check_output([author.RUBY, '-ryaml', '-e', 'print Psych::VERSION'], text=True)
    assert psych == '5.3.1', psych
    original_setup = author.setup
    def setup(case, root, mtime):
        original_setup(case, root, mtime)
        (root / 'defs/diagnostic.yaml').write_bytes(case['yaml'].encode())
    author.setup = setup
    inputs = []
    for name, source in [
        ('flow', 'name: [broken\n'),
        ('indent', 'name: test\n  bad: indent\n'),
        ('colon', 'name: : test\n'),
    ]:
        for newline in ['lf', 'crlf']:
            yaml = source if newline == 'lf' else source.replace('\n', '\r\n')
            for verb in ['validate', 'lint']:
                for fmt in ['text', 'json']:
                    case = {'id': f'{name}-{newline}-{verb}-{fmt}',
                            'runner': 'exe' if verb == 'validate' else 'lint',
                            'args': ([verb] if verb == 'validate' else []) + ['file', 'defs/diagnostic.yaml', '--format', fmt],
                            'yaml': yaml,
                            'env': {}, 'stdin': None, 'pty': False, 'mode': 'exact'}
                    inputs.append(case)
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        results = list(pool.map(lambda c: author.capture(c, oracle, 1767323045), inputs))
    gems = [r['runtime_gems'] for r in results]
    rows = [{'id': r['id'], 'yaml': r['yaml'],
             'args': (['lint'] if r['runner'] == 'lint' else []) + r['args'],
             'expect': {k: r['expect'][k] for k in ['stdout', 'stderr', 'exit']}} for r in results]
    assert len(rows) == 24 and len({r['id'] for r in rows}) == 24
    assert all(row['expect']['exit'] == 1 for row in rows), 'oracle failed unexpectedly'
    assert all(g == gems[0] for g in gems), 'dependency drift between probes'
    assert gems[0] == expected_gems, 'loaded dependency versions differ from author oracle'
    OUTPUT.mkdir(parents=True, exist_ok=True)
    payload = json.dumps({'oracle': author.SHA, 'cases': rows}, indent=2, ensure_ascii=False) + '\n'
    (OUTPUT / 'cases.json').write_text(payload)
    provenance = {'oracle': author.SHA, 'oracle_clean': True, 'source_commit': source_commit,
                  'ruby': ruby, 'psych': psych, 'timezone': 'UTC', 'locale': 'C.UTF-8',
                  'oracle_lock_sha256': digest(oracle / 'Gemfile.lock'), 'bundler_frozen': True,
                  'generator_sha256': digest(Path(__file__)),
                  'author_generator_sha256': digest(REPO / 'scripts/capture-author-reference.py'),
                  'payload_sha256': hashlib.sha256(payload.encode()).hexdigest()}
    (OUTPUT / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    print(f'Captured {len(rows)} exact diagnostic CLI cases')


if __name__ == '__main__':
    main()
