#!/usr/bin/env python3
"""Create missing Ukiryu sibling checkouts; never move or update existing ones."""
import argparse
from pathlib import Path
import subprocess

REPOS = ("ukiryu-rs", "ukiryu-ruby", "ukiryu-python", "ukiryu-ts", "ukiryu", "register", "ukiryu.github.io")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.home() / "src/ukiryu")
    parser.add_argument("--clone", action="store_true", help="create missing checkouts and worktree directory")
    args = parser.parse_args()
    root = args.root.expanduser().resolve()
    if args.clone:
        root.mkdir(parents=True, exist_ok=True)
    missing = []
    for name in REPOS:
        destination = root / name
        if not destination.exists() and args.clone:
            subprocess.run(["git", "clone", "--filter=blob:none", f"https://github.com/ukiryu/{name}.git", str(destination)], check=True)
        if destination.exists():
            probe = subprocess.run(["git", "-C", str(destination), "rev-parse", "--show-toplevel"],
                                   text=True, capture_output=True)
            if probe.returncode or Path(probe.stdout.strip()).resolve() != destination.resolve():
                raise ValueError(f"existing {destination} is not an independent checkout; left untouched")
            print(f"PRESENT {name} (existing contents unchanged)", flush=True)
        else:
            missing.append(name)
            print(f"MISSING {name}", flush=True)
    farm = root / "worktrees"
    if args.clone:
        farm.mkdir(exist_ok=True)
    if not farm.is_dir():
        missing.append("worktrees")
    print(f"sibling checkouts: {len(REPOS) - sum(name in REPOS for name in missing)}/{len(REPOS)}; worktrees: {farm.is_dir()}")
    return int(bool(missing))


if __name__ == "__main__":
    raise SystemExit(main())
