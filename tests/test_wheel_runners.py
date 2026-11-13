"""Acceptance runners must reject missing or stale completion evidence."""
import json
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import zipfile


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import download_blender


class WheelRunnerEvidence(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.root = Path(self.work.name)
        self.wheel = self.root / "rusty_tiles-0.0.0-py3-none-any.whl"
        self.wheel.write_bytes(b"invalid wheel")
        self.report = self.root / "report.json"
        self.report.write_text(json.dumps({"ok": True, "tests_run": 12}))

    def run_runner(self, script, *arguments):
        return subprocess.run(
            [sys.executable, str(ROOT / "scripts" / script), str(self.wheel),
             "--report-json", str(self.report), *arguments],
            capture_output=True,
            text=True,
            timeout=120,
        )

    def test_failed_wheel_install_clears_previous_success(self):
        result = self.run_runner("test_python_wheel.py")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is invalid", result.stderr)
        self.assertFalse(self.report.exists())

    def test_missing_wheel_clears_previous_success(self):
        self.wheel.unlink()
        for runner in ("test_python_wheel.py", "test_blender_wheel.py"):
            with self.subTest(runner=runner):
                self.report.write_text('{"ok": true}')
                result = self.run_runner(runner)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.report.exists())

    def test_missing_blender_clears_previous_success(self):
        result = self.run_runner("test_blender_wheel.py", "--blender", str(self.root / "missing-blender"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Blender executable not found", result.stderr)
        self.assertFalse(self.report.exists())

    def test_report_path_cannot_replace_input_wheel(self):
        before = self.wheel.read_bytes()
        self.report = self.wheel
        for runner in ("test_python_wheel.py", "test_blender_wheel.py"):
            with self.subTest(runner=runner):
                result = self.run_runner(runner)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("report path must differ", result.stderr)
                self.assertEqual(self.wheel.read_bytes(), before)

    @unittest.skipIf(os.name == "nt", "The test launcher uses a POSIX executable script")
    def test_blender_requires_current_successful_suite_evidence(self):
        # Simulate a launcher that exits successfully without running the suite,
        # then one that supplies a report for a different wheel.
        launcher = self.root / "blender"
        for body, expected in (
            ("pass\n", "no completion evidence"),
            ("import json, os\nfrom pathlib import Path\n"
             "Path(os.environ['RUSTY_TILES_ACCEPTANCE_REPORT']).write_text("
             "json.dumps({'ok': True, 'tests_run': 12, 'wheel_sha256': 'different-wheel'}))\n",
             "did not confirm successful acceptance"),
            ("import json, os\nfrom pathlib import Path\n"
             "Path(os.environ['RUSTY_TILES_ACCEPTANCE_REPORT']).write_text("
             "json.dumps({'ok': True, 'tests_run': 12, "
             "'wheel_sha256': os.environ['RUSTY_TILES_ACCEPTANCE_WHEEL_SHA256']}))\n",
             "did not provide Blender runtime identity"),
            ("import json, os\nfrom pathlib import Path\n"
             "Path(os.environ['RUSTY_TILES_ACCEPTANCE_REPORT']).write_text("
             "json.dumps({'ok': True, 'tests_run': 0, 'blender': 'test-launcher', "
             "'wheel_sha256': os.environ['RUSTY_TILES_ACCEPTANCE_WHEEL_SHA256']}))\n",
             "did not confirm successful acceptance"),
        ):
            with self.subTest(expected=expected):
                launcher.write_text(f"#!{sys.executable}\n" + body)
                launcher.chmod(0o755)
                self.report.write_text('{"ok": true}')
                result = self.run_runner("test_blender_wheel.py", "--blender", str(launcher))
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(expected, result.stderr)
                if expected == "no completion evidence":
                    self.assertFalse(self.report.exists())
                else:
                    evidence = json.loads(self.report.read_text())
                    self.assertIs(evidence["ok"], False)
                    self.assertIn(expected, evidence["runner_error"])

    @unittest.skipIf(os.name == "nt", "The test launcher uses a POSIX executable script")
    def test_current_failure_report_is_preserved(self):
        launcher = self.root / "blender"
        for claimed_success in (False, True):
            with self.subTest(claimed_success=claimed_success):
                launcher.write_text(
                    f"#!{sys.executable}\n"
                    "import json, os, sys\nfrom pathlib import Path\n"
                    "if sys.argv[sys.argv.index('--') + 1] == 'test':\n"
                    "    Path(os.environ['RUSTY_TILES_ACCEPTANCE_REPORT']).write_text("
                    f"json.dumps({{'ok': {claimed_success!r}, 'tests_run': 12, "
                    "'wheel_sha256': os.environ['RUSTY_TILES_ACCEPTANCE_WHEEL_SHA256']}))\n"
                    "    sys.exit(1)\n"
                )
                launcher.chmod(0o755)
                result = self.run_runner("test_blender_wheel.py", "--blender", str(launcher))
                self.assertNotEqual(result.returncode, 0)
                evidence = json.loads(self.report.read_text())
                self.assertIs(evidence["ok"], False)
                self.assertEqual(evidence["tests_run"], 12)
                self.assertEqual(evidence["runner_error"], "Acceptance subprocess exited with status 1")

    @unittest.skipIf(os.name == "nt", "The test launcher uses a POSIX executable script")
    def test_official_bundle_requires_pinned_runtime_version_and_architecture(self):
        launcher = self.root / "blender"
        manifest = json.loads(download_blender.MANIFEST.read_text())
        for series, release in manifest["releases"].items():
            spec = release["platforms"]["linux-x64"]
            pinned_version = [int(part) for part in release["version"].split(".")]
            pinned_python = f"{release['python']}.0 (main) [test]"
            other_python = f"{int(release['python'].split('.')[0]) + 1}.0.0 (main) [test]"
            for version, python, machine, expected in (
                ([0, 0, 0], pinned_python, "x86_64", "different Blender version"),
                (pinned_version, other_python, "x86_64", "different Python version"),
                (pinned_version, pinned_python, "aarch64", "different architecture"),
                (pinned_version, pinned_python, "x86_64", None),
            ):
                with self.subTest(release=series, expected=expected):
                    launcher.write_text(
                        f"#!{sys.executable}\n"
                        "import json, os\nfrom pathlib import Path\n"
                        "Path(os.environ['RUSTY_TILES_ACCEPTANCE_REPORT']).write_text("
                        "json.dumps({'ok': True, 'tests_run': 12, 'blender': 'test-launcher', "
                        f"'blender_version': {version!r}, 'python': {python!r}, 'machine': {machine!r}, "
                        "'wheel_sha256': os.environ['RUSTY_TILES_ACCEPTANCE_WHEEL_SHA256']}))\n"
                    )
                    launcher.chmod(0o755)
                    distribution = self.root / "distribution.json"
                    distribution.write_text(json.dumps({
                        "release": series, "distribution_platform": "linux-x64",
                        "version": release["version"],
                        "archive_url": release["base_url"] + spec["archive"],
                        "archive_sha256": spec["sha256"], "executable": str(launcher),
                        "executable_sha256": download_blender.sha256_file(launcher),
                    }))
                    result = self.run_runner("test_blender_wheel.py", "--blender", str(launcher),
                                             "--distribution-json", str(distribution))
                    evidence = json.loads(self.report.read_text())
                    if expected is None:
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertIs(evidence["ok"], True)
                        self.assertEqual(evidence["blender_distribution"], json.loads(distribution.read_text()))
                    else:
                        self.assertNotEqual(result.returncode, 0)
                        self.assertIs(evidence["ok"], False)
                        self.assertIn(expected, evidence["runner_error"])

    @unittest.skipIf(os.name == "nt", "The test launcher uses a POSIX executable script")
    def test_required_gltf_import_must_not_be_skipped(self):
        launcher = self.root / "blender"
        skip = ("__main__.WheelAPI.test_blender_imports_decoded_plain_glb: "
                "this Blender's Python lacks numpy, which its glTF importer needs")
        for skips, required, expected in (
            ([skip], True, "glTF import of decoded tiles did not run"),
            ([skip], False, None),
            (["__main__.WheelAPI.test_other: reason"], True, None),
        ):
            with self.subTest(skips=skips, required=required):
                launcher.write_text(
                    f"#!{sys.executable}\n"
                    "import json, os\nfrom pathlib import Path\n"
                    "Path(os.environ['RUSTY_TILES_ACCEPTANCE_REPORT']).write_text("
                    "json.dumps({'ok': True, 'tests_run': 12, 'blender': 'test-launcher', "
                    f"'skipped': 1, 'skips': {skips!r}, "
                    "'wheel_sha256': os.environ['RUSTY_TILES_ACCEPTANCE_WHEEL_SHA256']}))\n"
                )
                launcher.chmod(0o755)
                flags = ["--require-gltf-import"] if required else []
                result = self.run_runner("test_blender_wheel.py", "--blender", str(launcher), *flags)
                evidence = json.loads(self.report.read_text())
                if expected is None:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIs(evidence["ok"], True)
                else:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIs(evidence["ok"], False)
                    self.assertIn(expected, evidence["runner_error"])


class OfficialBlenderIntegrity(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.root = Path(self.work.name)

    def test_download_rejects_corrupted_archive_and_removes_it(self):
        source = self.root / "source.zip"
        source.write_bytes(b"corrupted archive")
        destination = self.root / "download.zip"
        with self.assertRaisesRegex(RuntimeError, "SHA-256 mismatch"):
            download_blender.download_verified(source.as_uri(), destination,
                                               hashlib.sha256(b"original archive").hexdigest())
        self.assertFalse(destination.exists())
        # The same local transfer succeeds only with its exact digest.
        download_blender.download_verified(source.as_uri(), destination,
                                           download_blender.sha256_file(source))
        self.assertEqual(destination.read_bytes(), source.read_bytes())

    def test_windows_archive_cannot_write_outside_bundle(self):
        archive = self.root / "blender.zip"
        with zipfile.ZipFile(archive, "w") as output:
            output.writestr("../escaped.exe", b"payload")
        with self.assertRaisesRegex(ValueError, "Unsafe Blender archive path"):
            download_blender.extract_bundle(archive, self.root / "bundle")
        self.assertFalse((self.root / "escaped.exe").exists())

    def test_macos_mount_detaches_when_application_copy_fails(self):
        error = subprocess.CalledProcessError(1, "ditto")
        with mock.patch.object(download_blender.subprocess, "run", side_effect=[None, error, None]) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                download_blender.extract_bundle(self.root / "blender.dmg", self.root / "bundle")
        self.assertEqual(run.call_args_list[-1].args[0][:2], ["hdiutil", "detach"])

    def test_changed_executable_cannot_reuse_verified_distribution_evidence(self):
        manifest = json.loads(download_blender.MANIFEST.read_text())
        release = manifest["releases"]["4.5"]
        spec = release["platforms"]["linux-x64"]
        executable = self.root / "blender"
        executable.write_bytes(b"original executable")
        record = {
            "release": "4.5", "distribution_platform": "linux-x64", "version": release["version"],
            "archive_url": release["base_url"] + spec["archive"],
            "archive_sha256": spec["sha256"], "executable": str(executable),
            "executable_sha256": download_blender.sha256_file(executable),
        }
        download_blender.validate_distribution(record, executable)
        executable.write_bytes(b"substituted executable")
        with self.assertRaisesRegex(RuntimeError, "does not match the verified distribution"):
            download_blender.validate_distribution(record, executable)

    def test_evidence_cannot_claim_another_release_or_platform(self):
        manifest = json.loads(download_blender.MANIFEST.read_text())
        release = manifest["releases"]["4.5"]
        spec = release["platforms"]["linux-x64"]
        executable = self.root / "blender"
        executable.write_bytes(b"executable")
        record = {
            "release": "4.5", "distribution_platform": "linux-x64", "version": release["version"],
            "archive_url": release["base_url"] + spec["archive"],
            "archive_sha256": spec["sha256"], "executable": str(executable),
            "executable_sha256": download_blender.sha256_file(executable),
        }
        download_blender.validate_distribution(record, executable)
        # 5.2 has no Intel macOS bundle, and 4.5 archive evidence cannot pass as 5.2.
        for changes in ({"release": "5.2"}, {"release": None}, {"release": "5.2", "distribution_platform": "macos-x64"}):
            with self.subTest(changes=changes):
                with self.assertRaisesRegex(RuntimeError, "does not match the pinned official bundle"):
                    download_blender.validate_distribution({**record, **changes}, executable)

    def test_manifest_pins_are_internally_consistent(self):
        manifest = json.loads(download_blender.MANIFEST.read_text())
        self.assertIn("4.5", manifest["releases"])
        self.assertIn("5.2", manifest["releases"])
        for series, release in manifest["releases"].items():
            with self.subTest(release=series):
                version = release["version"]
                self.assertTrue(version.startswith(series + "."))
                self.assertEqual(release["base_url"], f"https://download.blender.org/release/Blender{series}/")
                self.assertEqual(release["checksum_url"], f"{release['base_url']}blender-{version}.sha256")
                self.assertRegex(release["python"], r"^3\.[0-9]+$")
                for name, spec in release["platforms"].items():
                    self.assertTrue(spec["archive"].startswith(f"blender-{version}-{name}."))
                    self.assertRegex(spec["sha256"], "^[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
