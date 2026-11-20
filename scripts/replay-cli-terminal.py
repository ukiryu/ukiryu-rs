#!/usr/bin/env python3
"""Run the CLI on a real PTY; return raw terminal bytes and process status."""
import json
import os
import pty
import select
import subprocess
import sys

master, slave = pty.openpty()
process = subprocess.Popen(sys.argv[1:], stdout=slave, stderr=slave, stdin=subprocess.PIPE)
os.close(slave)
process.stdin.close()
chunks = []
try:
    while True:
        if not select.select([master], [], [], 30)[0]:
            process.kill()
            raise RuntimeError('CLI terminal replay timed out')
        try:
            chunk = os.read(master, 65536)
        except OSError:
            break
        if not chunk:
            break
        chunks.append(chunk)
finally:
    os.close(master)
status = process.wait(timeout=10)
print(json.dumps({'output': b''.join(chunks).decode('utf-8'), 'exit': status}))
