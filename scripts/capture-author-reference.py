#!/usr/bin/env python3
"""Capture the original Ruby author contract without repaired command classes.

Run in the pinned Ruby 4.0.1 environment: mise x -- python3 <script> <oracle>.
Fresh subprocesses prevent json/fileutils loaded by this generator from repairing
Ruby's missing requires. Standalone lint/docs are explicitly required and their
exceptions rendered by the original FriendlyCLI error handler. The sole stub is
the existing offline-schema condition: a failed default remote schema download.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import shutil
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[1]
DATA = REPO / 'ukiryu-cli/tests/author_data'
SHA = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
BOOT = '''require 'bundler/setup'; require 'ukiryu'; require 'ukiryu/cli'
Ukiryu::Definition::DefinitionValidator.singleton_class.send(:define_method, :download_and_load_schema) { nil }
'''
STANDALONE = '''require 'ukiryu/cli_commands/{runner}_command'
begin
  Ukiryu::CliCommands::{klass}Command.start(ARGV)
rescue Exception => e
  Ukiryu::Cli.send(:handle_exception_on_start, e, {{shell: Thor::Base.shell.new}})
end
'''


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def scrub(value, root):
    if isinstance(value, str):
        return value.replace(str(root), '{root}')
    if isinstance(value, list):
        return [scrub(v, root) for v in value]
    if isinstance(value, dict):
        return {scrub(k, root): scrub(v, root) for k, v in value.items()}
    return value


def setup(case, root, mtime):
    shutil.copytree(DATA / 'tree', root, dirs_exist_ok=True)
    for path in root.rglob('*'):
        if path.is_file():
            os.utime(path, (mtime, mtime))
    (root / 'home').mkdir(exist_ok=True)
    (root / 'bin').mkdir(exist_ok=True)
    if case['id'].startswith('lint_all_empty_tools'):
        (root / 'emptyreg/tools').mkdir(parents=True, exist_ok=True)
    if case['id'].startswith(('definitions_path_bundled', 'definitions_list_bundled')):
        (root / 'tooldir/bin').mkdir(parents=True, exist_ok=True)
        target = root / 'tooldir/share/ukiryu/zip/9.20.yaml'
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(root / 'defs/digit.yaml', target)
        os.utime(target, (mtime, mtime))


def capture(case, oracle, mtime):
    with tempfile.TemporaryDirectory(prefix='ukiryu-author-reference-') as tmp:
        root = Path(tmp)
        setup(case, root, mtime)
        env = {'HOME': str(root / 'home'), 'XDG_DATA_HOME': str(root / 'xdg'),
               'XDG_DATA_DIRS': str(root / 'sysdata'), 'PATH': str(root / 'bin'),
               'UKIRYU_REGISTER': str(root / 'register'), 'TERM': 'xterm' if case['pty'] else 'dumb',
               'TZ': 'UTC', 'LANG': 'C.UTF-8', 'LC_ALL': 'C.UTF-8'}
        if case['args'][0] == 'extract':
            env['SHELL'] = '/bin/bash'
        if case.get('suite') == 'tools':
            env.update(PATH=f'{root}/tbin:/usr/bin:/bin', UKIRYU_REGISTER=str(root / 'regx'), SHELL='/bin/bash')
        for key, value in case['env'].items():
            if value is None:
                env.pop(key, None)
            else:
                env[key] = value.replace('{root}', str(root))
        env['UKIRYU_CAPTURE_GEMS'] = str(root / 'gems.json')
        env['BUNDLE_GEMFILE'] = str(oracle / 'Gemfile')
        env['BUNDLE_FROZEN'] = 'true'
        runner = case['runner']
        code = "at_exit { require 'json'; File.write(ENV.fetch('UKIRYU_CAPTURE_GEMS'), JSON.generate(Gem.loaded_specs.transform_values { |s| s.version.to_s })) }\n" + BOOT + ('Ukiryu::Cli.start(ARGV)' if runner == 'exe' else
                       STANDALONE.format(runner=runner, klass=runner.title()))
        args = [v.replace('{root}', str(root)) for v in case['args']]
        cmd = [RUBY, '-I', str(oracle / 'lib'), '-e', code, '--', *args]
        if case['pty']:
            master, slave = pty.openpty()
            proc = subprocess.Popen(cmd, env=env, cwd=root, stdout=slave, stderr=slave,
                                    stdin=subprocess.PIPE)
            os.close(slave)
            if case['stdin']:
                proc.stdin.write(case['stdin'].encode())
            proc.stdin.close()
            chunks = []
            while True:
                if select.select([master], [], [], 60)[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError:
                        break
                    if not chunk:
                        break
                    chunks.append(chunk)
                else:
                    proc.kill()
                    raise RuntimeError('oracle timed out: ' + case['id'])
            os.close(master)
            code = proc.wait(timeout=10)
            output = b''.join(chunks).decode()
            expect = {'stdout': '', 'stderr': '', 'exit': code}
        else:
            proc = subprocess.run(cmd, env=env, cwd=root, input=case['stdin'],
                                  text=True, capture_output=True, timeout=60)
            expect = {'stdout': proc.stdout, 'stderr': proc.stderr, 'exit': proc.returncode}
        for relative in ('defs/invalid_yaml.yaml', 'defs/list.yaml'):
            digest = hashlib.sha256(str(root / relative).encode()).hexdigest()
            expect = json.loads(json.dumps(expect).replace(digest, '{sha256:' + relative + '}'))
        opts = case['args']
        destination = root / ('outdocs' if '--output-dir' in opts else
                              'out.md' if '--output' in opts else
                              'out.yaml' if '-o' in opts else 'xdg')
        paths = ([destination] if destination.is_file() else
                 sorted(p for p in destination.rglob('*') if p.is_file()))
        expect['files'] = [str(p) for p in paths]
        expect['file_contents'] = {str(p.relative_to(root)): p.read_text() for p in paths}
        row = {**case, 'mode': 'tty' if case['pty'] else 'exact',
               'expect': scrub(expect, root)}
        if case['pty']:
            row['tty_output'] = scrub(output, root)
        if case['mode'] in ('lines', 'yaml_data'):
            row['mode'] = case['mode']
        row['runtime_gems'] = json.loads((root / 'gems.json').read_text())
        return row


def fixture_capture(oracle):
    corpus = REPO / 'corpus/conformance/inputs'
    files = sorted(p for folder in ('register', 'profiles')
                   for p in (oracle / 'spec/fixtures' / folder).rglob('*.yaml'))
    assert len(files) == 64, len(files)
    for source in files:
        relative = source.relative_to(oracle / 'spec/fixtures')
        assert sha(source) == sha(corpus / relative), relative
    # A separate fresh interpreter loads JSON only after requiring the original
    # gem. No command implementation is patched; the same remote-failure stub
    # is shared with CLI captures.
    ruby = BOOT + '''require 'json'; require 'digest'
root = ARGV.fetch(0)
rows = ARGV.drop(1).map do |rel|
  path = File.join(root, rel)
  outcome = begin
    r = Ukiryu::Definition::DefinitionLinter.lint_file(path)
    {kind: 'result', to_h: r.to_h, to_s: r.to_s}
  rescue Exception => e
    {kind: 'raised', class: e.class.name, message: e.message}
  end
  validation = Ukiryu::Definition::DefinitionValidator.validate_file(path).to_h
  {path: rel, sha256: Digest::SHA256.file(path).hexdigest, lint: outcome, validation: validation}
end
puts JSON.generate(rows)
'''
    relatives = [str(p.relative_to(oracle / 'spec/fixtures')) for p in files]
    result = subprocess.run([RUBY, '-I', str(oracle / 'lib'), '-e', ruby, str(corpus), *relatives],
                            text=True, capture_output=True, check=True)
    rows = json.loads(result.stdout)
    # Lint source paths are the only substitution; code/message bytes remain.
    for row in rows:
        row['lint'] = scrub(row['lint'], corpus)
    return rows


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('oracle', type=Path)
    args = parser.parse_args()
    oracle = args.oracle.resolve()
    RUBY = shutil.which('ruby')
    assert subprocess.check_output(['git', '-C', str(oracle), 'rev-parse', 'HEAD'], text=True).strip() == SHA
    assert not subprocess.check_output(['git', '-C', str(oracle), 'status', '--porcelain'], text=True).strip()
    os.environ['BUNDLE_GEMFILE'] = str(oracle / 'Gemfile')
    os.environ['BUNDLE_FROZEN'] = 'true'
    assert (oracle / 'Gemfile.lock').read_bytes() == (DATA / 'oracle.Gemfile.lock').read_bytes()
    subprocess.run([RUBY, '-I', str(oracle / 'lib'), '-e', BOOT], check=True)
    source_paths = [str(Path(__file__).relative_to(REPO)),
                    'ukiryu-cli/tests/author_data/case_inputs.json',
                    'ukiryu-cli/tests/author_data/tools_case_inputs.json',
                    'ukiryu-cli/tests/author_data/oracle.Gemfile.lock',
                    'ukiryu-cli/tests/author_data/tree']
    subprocess.run(['git', '-C', str(REPO), 'diff', '--exit-code', 'HEAD', '--', *source_paths], check=True, stdout=subprocess.DEVNULL)
    generator_git = subprocess.check_output(['git', '-C', str(REPO), 'log', '-1', '--format=%H', '--', *source_paths], text=True).strip()
    inputs = json.loads((DATA / 'case_inputs.json').read_text())
    assert inputs['cases']
    assert all(c['mode'] not in ('frozen', 'gated', 'prefix') for c in inputs['cases'])
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        cases = list(pool.map(lambda c: capture(c, oracle, inputs['fixed_mtime_unix']), inputs['cases']))
    tools_inputs = json.loads((DATA / 'tools_case_inputs.json').read_text())
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        tools_cases = list(pool.map(lambda c: capture(c, oracle, inputs['fixed_mtime_unix']), tools_inputs['cases']))
    gem_sets = {json.dumps(c.pop('runtime_gems'), sort_keys=True) for c in cases + tools_cases}
    (DATA / 'tools_cases.json').write_text(json.dumps({'oracle': SHA, 'fixed_mtime_unix': inputs['fixed_mtime_unix'], 'cases': tools_cases}, indent=1, ensure_ascii=False) + '\n')
    payload = {'oracle': SHA, 'fixed_mtime_unix': inputs['fixed_mtime_unix'], 'cases': cases}
    (DATA / 'cases.json').write_text(json.dumps(payload, indent=1, ensure_ascii=False) + '\n')
    fixtures = fixture_capture(oracle)
    (DATA / 'fixture_cases.json').write_text(json.dumps({'oracle': SHA, 'cases': fixtures}, indent=1, ensure_ascii=False) + '\n')
    provenance = {'oracle': SHA, 'oracle_clean': True,
                  'oracle_lock_sha256': sha(oracle / 'Gemfile.lock'),
                  'bundler_frozen': True,
                  'ruby': subprocess.check_output([RUBY, '-e', 'print RUBY_VERSION'], text=True),
                  'ruby_engine': subprocess.check_output([RUBY, '-e', 'print RUBY_ENGINE'], text=True),
                  'locale': 'C.UTF-8', 'timezone': 'UTC',
                  'gems': [json.loads(g) for g in sorted(gem_sets)],
                  'generator_git': generator_git, 'generator_sources_clean': True,
                  'generator_sha256': sha(Path(__file__)), 'inputs_sha256': sha(DATA / 'case_inputs.json'),
                  'author_sha256': sha(DATA / 'cases.json'), 'fixtures_sha256': sha(DATA / 'fixture_cases.json'),
                  'tools_sha256': sha(DATA / 'tools_cases.json'),
                  'tools_inputs_sha256': sha(DATA / 'tools_case_inputs.json'),
                  'remote_schema': 'default download stubbed as unavailable; HTTP parity is a separate task',
                  'standalone': 'lint/docs explicitly required; exceptions rendered by original FriendlyCLI; no repaired methods'}
    (DATA / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    print(json.dumps({'author_cases': len(cases), 'fixture_cases': len(fixtures), 'tool_cases': len(tools_cases)}))
