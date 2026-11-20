"""Focused checks for the published-artifact boundary; no engine build."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("published_audit", ROOT / "scripts/published-audit.py")
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class PublishedConsumerTest(unittest.TestCase):
    def test_consumer_copies_only_tests_and_pins_registry_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            destination = Path(tmp) / "consumer"
            audit.prepare_core(destination, "0.1.0")
            manifest = tomllib.loads((destination / "Cargo.toml").read_text())
            self.assertEqual(manifest["dependencies"]["ukiryu"], "=0.1.0")
            self.assertFalse((destination / "src").exists())
            self.assertEqual((destination / "tests/conformance.rs").read_bytes(),
                             (ROOT / "ukiryu/tests/conformance.rs").read_bytes())
            with self.assertRaises(FileExistsError):
                audit.prepare_core(destination, "0.1.0")

    def test_version_rejects_ranges_and_manifest_injection(self):
        for value in ("latest", "^0.1.0", "01.1.0", '0.1.0"\npath="/tmp/core"', "0.1.0/../../core"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                audit.version(value)

    def test_input_revisions_are_immutable(self):
        current = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        audit.validate_inputs(current, {"core": "a" * 40})
        for value in ("main", "abc123", "A" * 40, "a" * 39):
            with self.subTest(value=value), self.assertRaises(ValueError):
                audit.validate_inputs(current, {"core": value})

    def test_lock_rejects_source_substitution_or_version_drift(self):
        with tempfile.TemporaryDirectory() as tmp:
            lock = Path(tmp) / "Cargo.lock"
            for source, release, checksum in (
                ("registry+https://github.com/rust-lang/crates.io-index", "0.1.0", "a" * 64),
                ("git+https://github.com/ukiryu/ukiryu-rs", "0.1.0", "a" * 64),
                ("", "0.1.0", "a" * 64),
                ("registry+https://github.com/rust-lang/crates.io-index", "0.2.0", "a" * 64),
                ("registry+https://github.com/rust-lang/crates.io-index", "0.1.0", ""),
            ):
                lock.write_text(f'[[package]]\nname="ukiryu"\nversion="{release}"\nsource="{source}"\nchecksum="{checksum}"\n')
                if source.startswith("registry+") and release == "0.1.0" and checksum:
                    audit.verify_core_lock(lock, "0.1.0")
                else:
                    with self.subTest(source=source, release=release), self.assertRaises(ValueError):
                        audit.verify_core_lock(lock, "0.1.0")

    def test_empty_cli_release_cannot_pass(self):
        sys.path.insert(0, str(ROOT / "scripts"))
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, "matrix incomplete"):
                audit.cli_archives(Path(tmp), "0.1.0")


if __name__ == "__main__":
    unittest.main()
