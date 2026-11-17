"""scripts/sha256s.py: reused CI archives and wheels must match the hashes CI recorded."""
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "sha256s.py"


def run(*args):
    return subprocess.run([sys.executable, "-I", str(SCRIPT), *args], capture_output=True, text=True)


class Sha256sTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="rusty-tiles-sha256s-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.built = self.root / "built"
        self.built.mkdir()
        for name in ("rusty_tiles-0.4.1-cp310-abi3-win_amd64.whl", "rusty_tiles-0.4.1-cp310-abi3-macosx_15_0_arm64.whl"):
            (self.built / name).write_bytes(name.encode() * 1000)
        # As CI does: one sums file per producing job.
        for path in sorted(self.built.iterdir()):
            result = run("write", str(self.root / "sums" / f"{path.name[:20]}-{len(path.name)}.sha256"), str(path))
            self.assertEqual(result.returncode, 0, result.stderr)
        self.sums = sorted(str(path) for path in (self.root / "sums").iterdir())
        self.downloaded = self.root / "dist"
        self.downloaded.mkdir()
        for path in self.built.iterdir():
            (self.downloaded / path.name).write_bytes(path.read_bytes())

    def check(self):
        return run("check", "--dir", str(self.downloaded), *self.sums)

    def test_write_uses_sha256sum_format(self):
        text = Path(self.sums[0]).read_text()
        names = [line.split("  ")[1] for line in Path(self.sums[0]).read_text().splitlines()]
        self.assertEqual(len(names), 1)
        self.assertRegex(text, r"^[0-9a-f]{64}  \S+\n$")
        self.assertIn(hashlib.sha256((self.built / names[0]).read_bytes()).hexdigest(), text)

    def test_identical_files_pass(self):
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.count(": OK "), 2)

    def test_a_changed_byte_fails(self):
        victim = next(self.downloaded.iterdir())
        data = bytearray(victim.read_bytes())
        data[0] ^= 1
        victim.write_bytes(bytes(data))
        result = self.check()
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"{victim.name}: SHA-256", result.stderr)
        self.assertIn("differs from CI's", result.stderr)

    def test_missing_and_extra_files_fail(self):
        victim = next(self.downloaded.iterdir())
        victim.unlink()
        (self.downloaded / "SHA256SUMS").write_text("stray\n")
        result = self.check()
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"{victim.name}: listed but not downloaded", result.stderr)
        self.assertIn("SHA256SUMS: downloaded but not recorded by CI", result.stderr)

    def test_conflicting_or_malformed_sums_fail(self):
        name = next(self.downloaded.iterdir()).name
        (self.root / "sums" / "conflict.sha256").write_text(f"{'0' * 64}  {name}\n")
        self.sums.append(str(self.root / "sums" / "conflict.sha256"))
        self.assertIn("listed with two different hashes", self.check().stderr)
        Path(self.sums[-1]).write_text("not a hash line\n")
        self.assertIn("not a '<sha256>  <name>' line", self.check().stderr)

    def test_no_hashes_fails(self):
        empty = self.root / "empty.sha256"
        empty.write_text("")
        for path in self.downloaded.iterdir():
            path.unlink()
        result = run("check", "--dir", str(self.downloaded), str(empty))
        self.assertEqual(result.returncode, 1)
        self.assertIn("no hashes to check", result.stderr)


if __name__ == "__main__":
    unittest.main()
