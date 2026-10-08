"""Acceptance runners must reject missing or stale completion evidence."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


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

    @unittest.skipIf(os.name == "nt", "The test launcher uses a POSIX executable script")
    def test_current_failure_report_is_preserved(self):
        launcher = self.root / "blender"
        launcher.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\nfrom pathlib import Path\n"
            "if sys.argv[sys.argv.index('--') + 1] == 'test':\n"
            "    Path(os.environ['RUSTY_TILES_ACCEPTANCE_REPORT']).write_text("
            "json.dumps({'ok': False, 'tests_run': 12, "
            "'wheel_sha256': os.environ['RUSTY_TILES_ACCEPTANCE_WHEEL_SHA256']}))\n"
            "    sys.exit(1)\n"
        )
        launcher.chmod(0o755)
        result = self.run_runner("test_blender_wheel.py", "--blender", str(launcher))
        self.assertNotEqual(result.returncode, 0)
        evidence = json.loads(self.report.read_text())
        self.assertIs(evidence["ok"], False)
        self.assertEqual(evidence["tests_run"], 12)


if __name__ == "__main__":
    unittest.main()
