"""The rolling dev build's version resolution and uncommitted version stamp."""
import contextlib
from datetime import datetime, timedelta, timezone
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import dev_version


MANIFEST = '''[package]
name = "rusty-tiles"
version.workspace = true

[workspace]
members = ["bindings/python"]

[workspace.package]
version = "0.4.0"

[dependencies]
serde = { version = "1" }
'''

LOCK = '''version = 4

[[package]]
name = "rusty-tiles"
version = "0.4.0"
dependencies = [
 "serde",
]

[[package]]
name = "rusty-tiles-python"
version = "0.4.0"
dependencies = [
 "rusty-tiles",
]

[[package]]
name = "serde"
version = "0.4.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "00"
'''


class DevelopmentVersion(unittest.TestCase):
    def test_next_patch_only_after_the_workspace_version_is_tagged(self):
        committed = datetime(2026, 11, 12, 17, 34, 59, tzinfo=timezone.utc)
        self.assertEqual(dev_version.development_version((0, 4, 0), True, committed),
                         ("0.4.1-dev.202611121734", "0.4.1.dev202611121734"))
        self.assertEqual(dev_version.development_version((0, 4, 0), False, committed),
                         ("0.4.0-dev.202611121734", "0.4.0.dev202611121734"))

    def test_development_number_uses_utc(self):
        brisbane = timezone(timedelta(hours=10))
        committed = datetime(2026, 11, 13, 3, 34, tzinfo=brisbane)
        self.assertEqual(dev_version.development_version((1, 2, 3), True, committed)[0],
                         "1.2.4-dev.202611121734")

    def test_release_core_drops_a_prerelease_and_rejects_non_semver(self):
        self.assertEqual(dev_version.release_core("0.5.0-rc.1"), (0, 5, 0))
        for version in ("0.5", "v0.5.0", "0.05.0"):
            with self.subTest(version=version), self.assertRaises(SystemExit):
                dev_version.release_core(version)


class Checkout(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="rusty-tiles-dev-version-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        patcher = mock.patch.object(dev_version, "ROOT", self.root)
        patcher.start()
        self.addCleanup(patcher.stop)

    def write(self, manifest=MANIFEST, lock=LOCK, newline="\n"):
        for name, text in (("Cargo.toml", manifest), ("Cargo.lock", lock)):
            with (self.root / name).open("w", encoding="utf-8", newline=newline) as output:
                output.write(text)

    def stamp(self, version):
        with contextlib.redirect_stdout(io.StringIO()):
            dev_version.stamp(version)

    def test_stamp_changes_only_workspace_versions(self):
        self.write()
        self.stamp("0.4.1-dev.202611121734")
        self.assertEqual((self.root / "Cargo.toml").read_text(),
                         MANIFEST.replace('version = "0.4.0"', 'version = "0.4.1-dev.202611121734"'))
        expected = LOCK.replace('rusty-tiles"\nversion = "0.4.0"', 'rusty-tiles"\nversion = "0.4.1-dev.202611121734"')
        expected = expected.replace('rusty-tiles-python"\nversion = "0.4.0"',
                                    'rusty-tiles-python"\nversion = "0.4.1-dev.202611121734"')
        self.assertEqual((self.root / "Cargo.lock").read_text(), expected)
        self.assertIn('name = "serde"\nversion = "0.4.0"\nsource', expected)

    def test_stamp_keeps_crlf_checkouts(self):
        self.write(newline="\r\n")
        self.stamp("0.4.1-dev.1")
        for name in ("Cargo.toml", "Cargo.lock"):
            raw = (self.root / name).read_bytes()
            self.assertIn(b"0.4.1-dev.1", raw)
            self.assertEqual(raw.count(b"\n"), raw.count(b"\r\n"))

    def test_stamp_refuses_inconsistent_or_invalid_input(self):
        cases = (
            (MANIFEST, LOCK.replace('python"\nversion = "0.4.0"', 'python"\nversion = "0.3.0"'), "0.4.1",
             "differ from version 0.4.0"),
            (MANIFEST.replace("[workspace.package]", "[workspace.metadata]"), LOCK, "0.4.1", "no [workspace.package]"),
            (MANIFEST.replace('version = "0.4.0"', 'version = "0.4.0"\nversion = "0.4.0"'), LOCK, "0.4.1",
             "exactly one version"),
            (MANIFEST, LOCK, "0.4.1.dev1", "not a SemVer"),
        )
        for manifest, lock, version, message in cases:
            with self.subTest(message=message):
                self.write(manifest, lock)
                with self.assertRaises(SystemExit) as caught:
                    self.stamp(version)
                self.assertIn(message, str(caught.exception))
                self.assertEqual((self.root / "Cargo.toml").read_text(), manifest)
                self.assertEqual((self.root / "Cargo.lock").read_text(), lock)

    def test_resolve_reads_the_tag_and_commit_time(self):
        self.write()
        environment = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
                           GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.invalid",
                           GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@example.invalid",
                           GIT_COMMITTER_DATE="2026-11-12T17:34:00Z", GIT_AUTHOR_DATE="2026-11-12T17:34:00Z")

        def git(*arguments):
            subprocess.run(["git", "-C", str(self.root), *arguments], check=True, env=environment,
                           capture_output=True)

        def resolve():
            output = io.StringIO()
            with mock.patch.dict(os.environ, environment), contextlib.redirect_stdout(output):
                dev_version.resolve()
            return output.getvalue().splitlines()

        git("init", "-q")
        git("add", "Cargo.toml", "Cargo.lock")
        git("commit", "-q", "-m", "base")
        self.assertEqual(resolve(), ["version=0.4.0-dev.202611121734", "python-version=0.4.0.dev202611121734"])
        git("tag", "v0.4.0")
        self.assertEqual(resolve(), ["version=0.4.1-dev.202611121734", "python-version=0.4.1.dev202611121734"])


if __name__ == "__main__":
    unittest.main()
