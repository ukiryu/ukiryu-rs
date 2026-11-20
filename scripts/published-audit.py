#!/usr/bin/env python3
"""Prepare registry-only consumers and verify published CLI archives.

The checkout supplies the sealed corpus and its consumers, never the engine
under test. Publication and version selection remain release-owner actions.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def version(value):
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?", value):
        raise ValueError("version must be an explicit X.Y.Z release (optionally a prerelease)")
    return value


def validate_inputs(release, refs):
    version(release)
    for name, ref in refs.items():
        if not re.fullmatch(r"[0-9a-f]{40}", ref):
            raise ValueError(f"{name} must be a full lowercase commit SHA")
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
    if workspace["workspace"]["package"]["version"] != release:
        raise ValueError("the qualified core checkout must have the requested train version")


def prepare_core(destination, release):
    """Copy only the test consumer; Cargo resolves ukiryu from crates.io."""
    version(release)
    destination.mkdir(parents=True, exist_ok=False)
    (destination / "tests").mkdir()
    shutil.copy2(ROOT / "ukiryu/tests/conformance.rs", destination / "tests")
    shutil.copytree(ROOT / "ukiryu/tests/conformance", destination / "tests/conformance")
    (destination / "Cargo.toml").write_text(f'''[package]
name = "ukiryu-published-audit"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
ukiryu = "={release}"
serde = {{ version = "1", features = ["derive"] }}
serde_json = {{ version = "1", features = ["arbitrary_precision", "preserve_order"] }}
num-bigint = "0.4"
''')


def verify_core_lock(lockfile, release):
    version(release)
    packages = tomllib.loads(lockfile.read_text())["package"]
    core = [p for p in packages if p["name"] == "ukiryu"]
    if len(core) != 1 or core[0]["version"] != release:
        raise ValueError("audit must resolve exactly the requested core version")
    package = core[0]
    if package.get("source") != "registry+https://github.com/rust-lang/crates.io-index":
        raise ValueError("core must come from crates.io, never a checkout or git dependency")
    if not re.fullmatch(r"[0-9a-f]{64}", package.get("checksum", "")):
        raise ValueError("registry core must carry its published checksum")
    print(json.dumps(package, sort_keys=True))


def cli_archives(directory, release):
    """Verify all published targets, then replay the Linux CLI corpus."""
    version(release)
    spec = importlib.util.spec_from_file_location("cli_artifacts", ROOT / "scripts/cli-binary-artifacts.py")
    artifacts = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(artifacts)
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    seen = set()
    # The archive verifier resolves license and lock paths relative to ROOT.
    for archive in sorted(directory.iterdir()):
        root, members = artifacts.read_archive(archive)
        marker = artifacts.verify_payload(root, members, commit)
        if marker["crate_version"] != release or marker["target"] in seen:
            raise ValueError("duplicate target or CLI archive version differs from the train")
        seen.add(marker["target"])
        if marker["target"] == "x86_64-unknown-linux-gnu":
            binary = directory.parent / "audited-cli"
            with binary.open("xb") as handle:
                handle.write(members[marker["binary"]])
            binary.chmod(0o755)
            subprocess.run([
                "python3", str(ROOT / "tests/shipped-replay/cli.py"),
                str(binary.resolve()), str(ROOT / "corpus/conformance"),
            ], check=True, cwd=ROOT)
        print(f"published CLI: {archive.name} {marker['core_commit']}")
    if seen != set(artifacts.TARGETS):
        raise ValueError(f"published CLI target matrix incomplete: {sorted(seen)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("inputs", "prepare-core", "verify-core-lock", "cli"))
    parser.add_argument("--version", required=True)
    parser.add_argument("--path", type=Path)
    for name in ("core", "ruby", "python", "ts"):
        parser.add_argument(f"--{name}-ref")
    args = parser.parse_args()
    if args.action == "inputs":
        validate_inputs(args.version, {name: getattr(args, f"{name}_ref") or ""
                                       for name in ("core", "ruby", "python", "ts")})
    elif args.path is None:
        parser.error("--path is required for this action")
    elif args.action == "prepare-core":
        prepare_core(args.path, args.version)
    elif args.action == "verify-core-lock":
        verify_core_lock(args.path, args.version)
    else:
        cli_archives(args.path, args.version)


if __name__ == "__main__":
    main()
