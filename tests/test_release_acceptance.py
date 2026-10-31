"""A required browser gate must fail before building when tools are absent."""
import json
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


def doctor_report(commands, exit_code=None):
    """A doctor --json report in the T1 refusal model."""
    entries, refusals = {}, []
    for name, refusal in sorted(commands.items()):
        entry = {"status": "ready" if refusal is None else "unavailable", "ready": refusal is None}
        if refusal is not None:
            entry["refusal"] = {"code": refusal[0], "exitCode": refusal[1]}
            refusals.append(refusal)
        entries[name] = entry
    report = {"commands": entries, "selectedCommands": list(commands), "ready": not refusals,
              "ok": not refusals, "status": "unavailable" if refusals else "ready",
              "probes": ["proj-directory-listing"], "proj": {"networkEnabled": False}}
    if refusals:
        report["error"] = {"code": refusals[0][0], "message": "stub"}
        report["exitCode"] = refusals[0][1] if exit_code is None else exit_code
    return report


@unittest.skipIf(os.name == "nt" or shutil.which("bash") is None, "POSIX acceptance script")
class DoctorContractStep(unittest.TestCase):
    """Step 2 checks the selected binary's doctor exit against its refusals."""

    def run_with_stub(self, process_exit):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            readme = doctor_report({"mesh": None, "validate": None, "preview": None})
            full = doctor_report({"convert": None, "raster": ("unsupported", 2)})
            binary = root / "rusty-tiles"
            # The harness runs the binary with an empty PATH: builtins only.
            binary.write_text(
                "#!/bin/sh\n"
                f"if [ \"$1\" = doctor ] && [ \"$2\" = --command ]; then printf '%s\\n' '{json.dumps(readme)}'; exit 0; fi\n"
                f"if [ \"$1\" = doctor ]; then printf '%s\\n' '{json.dumps(full)}'; exit {process_exit}; fi\n"
                "exit 1\n")
            binary.chmod(0o755)
            return subprocess.run(
                ["bash", str(ROOT / "scripts/release_acceptance.sh")],
                env=dict(os.environ, ACCEPTANCE_REQUIRE_BROWSER="0", RUSTY_TILES_BIN=str(binary),
                         ACCEPTANCE_WORK=str(root / "work"), PYTHON=sys.executable),
                capture_output=True, text=True, timeout=60,
            )

    def test_exit_matching_the_first_refusal_passes_step_2(self):
        result = self.run_with_stub(2)
        self.assertIn("ok: doctor", result.stdout, result.stderr)
        self.assertIn("FAIL: convert", result.stderr)

    def test_exit_disagreeing_with_the_refusal_fails_step_2(self):
        result = self.run_with_stub(4)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("ok: doctor", result.stdout)
        self.assertIn("violates the status/exit contract", result.stderr)


if __name__ == "__main__":
    unittest.main()
