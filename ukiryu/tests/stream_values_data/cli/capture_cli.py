#!/usr/bin/env python3
"""Real `ukiryu exec` runs of the pinned Ruby CLI (ukiryu/ukiryu@ab6df353)
for the stream payloads of capture_stream_values.rb.

The tool is the `emit` register in this directory (bin/emit on PATH writes
its first two arguments, after a leading X, as printf escapes to stdout and stderr and exits
with the third). Each payload runs once on stdout and once on stderr, in
--raw mode (with and without --output FILE) and in --format json, where
only the parsed "output" section is kept (the rest carries timestamps).

Usage: capture_cli.py OUT.json COMMAND...
COMMAND is the CLI prefix, for example
`bundle exec ukiryu exec --register=<this dir>/register emit run`, run under
the locked oracle bundle (corpus/oracle-ruby) with <this dir>/bin on PATH.
"""
import json
import os
import subprocess
import sys
import tempfile

# Messages that name the tool's path show this directory as <CLI>.
HERE = os.path.dirname(os.path.abspath(__file__)).encode()

PAYLOADS = {
    'empty': b'',
    'newline': b'\n',
    'crlf_lines': b'a\r\nb\r\n',
    'whitespace_only': b' \t\n',
    'nul_middle': b'a\x00b',
    'nul_edges': b'\x00 a \x00',
    'utf8_text': 'héllo ✓ \U0001d11e\n'.encode(),
    'utf8_edges': ' é \n'.encode(),
    'all_bytes': bytes(range(256)),
    'invalid_middle': b'a\xffb',
    'invalid_after_space': b' \xffa',
    'invalid_truncated_end': b'ok\xe2\x82',
    'invalid_before_space': b'ok\xff \n',
    'invalid_mixed': b'\xe2\x82A\xc0\x80\xed\xa0\x80\xf4\x90\x80\x80\xf0\x9f\x98B\x80\xfe\xc3\xa9',
    'invalid_lines': b'x\xff\ny\r\nz',
    'invalid_after_newline': b'\n\x80',
}


def fmt(data):
    # The tool's arguments must not be empty (ValidationError): X marks the start.
    return 'X' + ''.join('\\x%02x' % b for b in data)


def run(prefix, out, err, code, extra):
    args = prefix + ['out=' + fmt(out), 'err=' + fmt(err), 'code=%d' % code] + extra
    p = subprocess.run(args, capture_output=True, env={**os.environ, 'NO_COLOR': '1'})
    p.stdout = p.stdout.replace(HERE, b'<CLI>')
    p.stderr = p.stderr.replace(HERE, b'<CLI>')
    return p


def case(run_sh, cid, out, err, code):
    row = {'id': cid, 'stdout_payload_hex': out.hex(), 'stderr_payload_hex': err.hex(), 'status': code}
    p = run(run_sh, out, err, code, ['--raw'])
    row['raw'] = {'exit': p.returncode, 'stdout_hex': p.stdout.hex(), 'stderr_hex': p.stderr.hex()}
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, 'out.bin')
        p = run(run_sh, out, err, code, ['--raw', '--output', path])
        row['raw_file'] = {
            'exit': p.returncode,
            'stdout_hex': p.stdout.hex(),
            'stderr_hex': p.stderr.hex(),
            'file_hex': open(path, 'rb').read().hex() if os.path.exists(path) else None,
        }
    p = run(run_sh, out, err, code, ['--format', 'json'])
    doc = json.loads(p.stdout)
    keep = {k: doc[k] for k in ('status', 'error', 'output') if k in doc}
    row['json'] = {'exit': p.returncode, 'stderr_hex': p.stderr.hex(),
                   'response': {k: (v if k != 'output' else {kk: vv.encode().hex() for kk, vv in v.items()})
                                for k, v in keep.items()}}
    return row


def main():
    out_path, run_sh = sys.argv[1], sys.argv[2:]
    rows = []
    for name, data in PAYLOADS.items():
        rows.append(case(run_sh, 'stdout_' + name, data, b'', 0))
        rows.append(case(run_sh, 'stderr_' + name, b'', data, 0))
    rows.append(case(run_sh, 'both_source_fixture', b'A\x00\xff\x80B', b'C\xfeD', 0))
    rows.append(case(run_sh, 'failed_valid', b'o\x00k\n', b'bad\n', 3))
    rows.append(case(run_sh, 'failed_stdout_invalid', b'a\xffb', b'bad\n', 3))
    rows.append(case(run_sh, 'failed_stdout_invalid_after_space', b' \xffa', b'', 3))
    with open(out_path, 'w') as f:
        json.dump({'rows': rows}, f, indent=2)
        f.write('\n')
    print(len(rows), 'rows')


main()
