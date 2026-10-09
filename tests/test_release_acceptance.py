"""A required browser gate must fail before building when tools are absent."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipIf(os.name == "nt" or shutil.which("bash") is None, "POSIX acceptance script")
class RequiredBrowserAcceptance(unittest.TestCase):
    def run_acceptance(self, **changes):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            environment = dict(
                os.environ,
                ACCEPTANCE_REQUIRE_BROWSER="1",
                ACCEPTANCE_WORK=str(work / "acceptance"),
                CESIUM_DIR=str(work / "missing-runtime"),
                CHROMIUM=str(work / "missing-chromium"),
                PYTHON=sys.executable,
                # This would fail at the binary step if preflight were bypassed.
                RUSTY_TILES_BIN=str(work / "missing-cli"),
            )
            environment.update(changes)
            return subprocess.run(
                ["bash", str(ROOT / "scripts/release_acceptance.sh")],
                env=environment, capture_output=True, text=True, timeout=30,
            )

    def test_missing_browser_tools_cannot_be_skipped_in_release_mode(self):
        result = self.run_acceptance()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("required browser acceptance prerequisites", result.stderr)
        self.assertIn("Cesium runtime", result.stderr)
        self.assertIn("Chromium", result.stderr)
        self.assertNotIn("not an executable", result.stderr)
        self.assertNotIn("skipped: browser", result.stdout)

    def test_invalid_strict_setting_is_not_treated_as_optional(self):
        result = self.run_acceptance(ACCEPTANCE_REQUIRE_BROWSER="true")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ACCEPTANCE_REQUIRE_BROWSER must be 0 or 1", result.stderr)

    def test_optional_mode_remains_available_for_default_builds(self):
        result = self.run_acceptance(ACCEPTANCE_REQUIRE_BROWSER="0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("RUSTY_TILES_BIN is not an executable", result.stderr)
        self.assertNotIn("required browser acceptance prerequisites", result.stderr)

    def test_selected_binary_without_native_converters_fails_required_gate(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tools = root / "tools"
            tools.mkdir()
            # Isolate browser-tool readiness from the selected CLI's exit code.
            for name in ("node", "python", "chromium"):
                tool = tools / name
                tool.write_text("#!/bin/sh\nexit 0\n")
                tool.chmod(0o755)
            binary = tools / "rusty-tiles"
            binary.write_text("#!/bin/sh\nprintf '%s\\n' '{\"ok\":false}'\nexit 4\n")
            binary.chmod(0o755)
            runtime = root / "runtime"
            runtime.mkdir()
            (runtime / "Cesium.js").write_text("// stand-in for prerequisite check\n")
            result = self.run_acceptance(
                PATH=str(tools) + os.pathsep + os.environ.get("PATH", ""),
                PYTHON=str(tools / "python"), CHROMIUM=str(tools / "chromium"),
                CESIUM_DIR=str(runtime), RUSTY_TILES_BIN=str(binary),
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("required browser acceptance prerequisites", result.stderr)
            self.assertIn("selected binary's", result.stderr)
            self.assertNotIn("doctor for README commands", result.stderr)


if __name__ == "__main__":
    unittest.main()
