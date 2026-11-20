#!/usr/bin/env python3
"""Replay the frozen Linux terminal captures against the compiled CLI.

stdout and stderr share a real PTY, as in the Ruby capture. Disable only
terminal newline translation so the expected LF bytes need no output mask.
"""

import argparse
import errno
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import subprocess
import tempfile
import termios
import time


ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "ukiryu-cli/tests/execute_data"
ORACLE = "ukiryu/ukiryu ab6df353c6dc2d7faf7817a86aafcafa82579896"


def terminal(binary, args, env, cwd, stdin):
    master, slave = pty.openpty()
    try:
        settings = termios.tcgetattr(slave)
        settings[1] &= ~termios.ONLCR
        termios.tcsetattr(slave, termios.TCSANOW, settings)
        child = subprocess.Popen(
            [str(binary), *args], cwd=cwd, env=env, stdin=subprocess.PIPE,
            stdout=slave, stderr=slave, start_new_session=True,
        )
    finally:
        os.close(slave)
    chunks = []
    try:
        try:
            child.stdin.write((stdin or "").encode())
        except BrokenPipeError:
            pass
        child.stdin.close()
        deadline = time.monotonic() + 15
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"CLI terminal timeout: {args}")
            if not select.select([master], [], [], remaining)[0]:
                raise TimeoutError(f"CLI terminal timeout: {args}")
            try:
                chunk = os.read(master, 65536)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                break  # Linux PTYs signal EOF with EIO after the slave closes.
            if not chunk:
                break
            chunks.append(chunk)
        return b"".join(chunks), child.wait(timeout=max(0.01, deadline - time.monotonic()))
    finally:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
        os.close(master)


def matches(row, output, status):
    return output == row["stdout"].encode() and status == row["status"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    data = json.loads((DATA / "cases.json").read_text())
    assert data["oracle"] == ORACLE, "terminal captures must use the pinned oracle"
    rows = [row for row in data["cases"] if row.get("pty") is True]
    assert len(rows) == 12, f"expected all 12 terminal rows, got {len(rows)}"
    failures = []
    for row in rows:
        assert row["stderr"] is None, "terminal capture merges stderr into stdout"
        with tempfile.TemporaryDirectory(prefix="ukiryu-cli-terminal-") as scratch:
            root = Path(scratch) / "fixture"
            shutil.copytree(DATA / "fixture", root)
            env = {
                "HOME": str(root / "home"), "XDG_DATA_HOME": str(root / "xdg"),
                "XDG_DATA_DIRS": str(root / "sysdata"), "PATH": "/usr/bin:/bin",
                "SHELL": "/bin/bash", "UKIRYU_REGISTER": str(root / "register"),
                "TERM": "dumb", "TZ": "UTC", "LANG": "C.UTF-8",
            }
            for key, value in row["env"].items():
                if value is None:
                    env.pop(key, None)
                else:
                    env[key] = value
            output, status = terminal(binary, row["args"], env, root, row["stdin"])
            if not matches(row, output, status):
                failures.append(
                    f"{row['id']}: exit {status} (expected {row['status']}), "
                    f"output {output!r} (expected {row['stdout'].encode()!r})"
                )
            assert not matches(row, output + b"changed", status), "output mismatch accepted"
            assert not matches(row, output, status + 1), "exit status mismatch accepted"
            # The captures currently write no files; detect unexpected writes.
            expected = {
                path.relative_to(DATA / "fixture"): path.read_bytes()
                for path in (DATA / "fixture").rglob("*") if path.is_file()
            }
            actual = {
                path.relative_to(root): path.read_bytes()
                for path in root.rglob("*") if path.is_file()
            }
            if expected != actual:
                failures.append(f"{row['id']}: unexpected fixture file changes")
    assert not failures, "\n".join(failures)
    print(f"terminal CLI captures: {len(rows)} passed; output/status negative controls passed")


if __name__ == "__main__":
    main()
