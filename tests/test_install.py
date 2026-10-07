"""Exercise release selection and verify-before-install without network access."""
import hashlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import textwrap
import unittest


INSTALLER = Path(__file__).resolve().parents[1] / "scripts" / "install.sh"


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="rusty-tiles-install-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.downloads = self.root / "downloads"
        self.downloads.mkdir()
        self.destination = self.root / "bin with spaces"
        self.env = dict(os.environ, PATH=f"{self.tools}:{os.environ['PATH']}",
                        INSTALL_TEST_DOWNLOADS=str(self.downloads),
                        INSTALL_TEST_REQUESTS=str(self.root / "requests"),
                        INSTALL_TEST_PLATFORM="Linux", INSTALL_TEST_ARCH="x86_64",
                        INSTALL_TEST_MUSL="0", INSTALL_TEST_LATEST="v0.3.0")
        self.tool("uname", 'case "$1" in -m) echo "$INSTALL_TEST_ARCH";; -s) echo "$INSTALL_TEST_PLATFORM";; esac')
        self.tool("getconf", '[ "$INSTALL_TEST_MUSL" = 0 ] && echo "glibc 2.35"')
        self.tool("curl", textwrap.dedent('''\
            out=
            latest=0
            while [ "$#" -gt 0 ]; do
                case "$1" in
                    -o|--output) out=$2; shift 2 ;;
                    --write-out) latest=1; shift 2 ;;
                    --retry|--proto|--proto-redir) shift 2 ;;
                    --*) shift ;;
                    *) url=$1; shift ;;
                esac
            done
            printf '%s\\n' "$url" >> "$INSTALL_TEST_REQUESTS"
            if [ "$latest" = 1 ]; then
                printf 'https://github.com/BenDyson-Arch/rusty-tiles/releases/tag/%s' "$INSTALL_TEST_LATEST"
            else
                cp "$INSTALL_TEST_DOWNLOADS/${url##*/}" "$out"
            fi
        '''))

    def tool(self, name, script):
        path = self.tools / name
        path.write_text("#!/bin/sh\nset -eu\n" + script + "\n")
        path.chmod(0o755)

    def release(self, version="0.3.0", target="x86_64-unknown-linux-gnu", binary=None, checksum=None):
        payload = binary or b"#!/bin/sh\necho 'rusty-tiles 0.3.0'\n"
        name = f"rusty-tiles-{version}-{target}.tgz"
        path = self.downloads / name
        with tarfile.open(path, "w:gz") as archive:
            info = tarfile.TarInfo("rusty-tiles")
            info.size = len(payload)
            info.mode = 0o755
            archive.addfile(info, io.BytesIO(payload))
        digest = checksum or hashlib.sha256(path.read_bytes()).hexdigest()
        (self.downloads / "SHA256SUMS").write_text(f"{digest}  {name}\n")

    def run_installer(self, *args):
        return subprocess.run(["sh", str(INSTALLER), "--prefix", str(self.destination), *args],
                              env=self.env, capture_output=True, text=True)

    def preserve_old_binary(self):
        self.destination.mkdir()
        (self.destination / "rusty-tiles").write_text("existing binary")

    def test_latest_pins_downloads_to_resolved_version(self):
        self.release()
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        requests = (self.root / "requests").read_text().splitlines()
        self.assertEqual(len(requests), 3)
        self.assertTrue(requests[0].endswith("/releases/latest"))
        self.assertTrue(all("/download/v0.3.0/" in url for url in requests[1:]))
        installed = self.destination / "rusty-tiles"
        self.assertTrue(os.access(installed, os.X_OK))
        self.assertIn("Installed", result.stdout)

    def test_pinned_prerelease_on_apple_silicon(self):
        self.env.update(INSTALL_TEST_PLATFORM="Darwin", INSTALL_TEST_ARCH="arm64")
        self.release(version="0.3.0-rc.1", target="aarch64-apple-darwin")
        result = self.run_installer("--version", "v0.3.0-rc.1")
        self.assertEqual(result.returncode, 0, result.stderr)
        requests = (self.root / "requests").read_text()
        self.assertNotIn("/latest", requests)
        self.assertIn("aarch64-apple-darwin.tgz", requests)

    def test_arm_linux_and_intel_macos_assets(self):
        for platform, arch, target in (("Linux", "aarch64", "aarch64-unknown-linux-gnu"),
                                       ("Darwin", "x86_64", "x86_64-apple-darwin")):
            with self.subTest(target=target):
                self.env.update(INSTALL_TEST_PLATFORM=platform, INSTALL_TEST_ARCH=arch)
                self.release(target=target)
                result = self.run_installer("--version", "0.3.0")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(target, (self.root / "requests").read_text())

    def test_shasum_fallback(self):
        self.release()
        # Restrict PATH so a Linux host exercises the checksum tool used on macOS.
        for tool in ("sh", "tar", "gzip", "mktemp", "install", "awk", "chmod", "cp", "rm", "mkdir", "shasum"):
            executable = shutil.which(tool)
            if executable is None:
                self.skipTest(f"{tool} is unavailable")
            (self.tools / tool).symlink_to(executable)
        self.env["PATH"] = str(self.tools)
        result = self.run_installer("--version", "0.3.0")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_checksum_mismatch_never_executes_or_replaces_binary(self):
        marker = self.root / "executed"
        self.release(binary=f"#!/bin/sh\ntouch '{marker}'\n".encode(), checksum="0" * 64)
        self.preserve_old_binary()
        result = self.run_installer("--version", "0.3.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertFalse(marker.exists())
        self.assertEqual((self.destination / "rusty-tiles").read_text(), "existing binary")

    def test_missing_checksum_preserves_existing_binary(self):
        self.release()
        (self.downloads / "SHA256SUMS").write_text("0" * 64 + "  wrong-asset.tgz\n")
        self.preserve_old_binary()
        result = self.run_installer("--version", "0.3.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing or invalid checksum", result.stderr)
        self.assertEqual((self.destination / "rusty-tiles").read_text(), "existing binary")

    def test_binary_runtime_failure_preserves_existing_binary(self):
        self.release(binary=b"#!/bin/sh\nexit 127\n")
        self.preserve_old_binary()
        result = self.run_installer("--version", "0.3.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("binary cannot run", result.stderr)
        self.assertEqual((self.destination / "rusty-tiles").read_text(), "existing binary")

    def test_source_only_release_has_actionable_error(self):
        result = self.run_installer("--version", "0.2.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("older releases are source-only", result.stderr)
        self.assertFalse(self.destination.exists())

    def test_unsupported_platform_and_musl_fail_before_download(self):
        for setting, value in (("INSTALL_TEST_PLATFORM", "Windows_NT"),
                               ("INSTALL_TEST_ARCH", "riscv64"), ("INSTALL_TEST_MUSL", "1")):
            with self.subTest(setting=setting):
                previous = self.env[setting]
                self.env[setting] = value
                result = self.run_installer("--version", "0.3.0")
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.root / "requests").exists())
                self.env[setting] = previous

    def test_rejects_bad_arguments_before_download(self):
        for args in (("--version", "../../escape"), ("--version",), ("--unknown",)):
            with self.subTest(args=args):
                result = self.run_installer(*args)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.root / "requests").exists())


if __name__ == "__main__":
    unittest.main()
