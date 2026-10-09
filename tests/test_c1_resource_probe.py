"""Exercise descriptor-observation races without weakening validator checks."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location(
    "c1_resource_probe",
    Path(__file__).resolve().parents[1] / "bench/architecture_audit/c1/resource_probe.py",
)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class DescriptorObservationTests(unittest.TestCase):
    def observe(self, observation):
        usage = SimpleNamespace(ru_maxrss=123, ru_utime=0.1, ru_stime=0.2)
        child = SimpleNamespace(pid=999999, returncode=None)
        with tempfile.TemporaryDirectory() as temporary:
            config = Path(temporary) / "config.json"
            config.write_text(json.dumps({"command": ["unused"]}))
            output = io.StringIO()
            with (
                mock.patch.object(probe.subprocess, "Popen", return_value=child),
                mock.patch.object(probe.os, "wait4", side_effect=[(0, 0, usage), (child.pid, 3 << 8, usage)]),
                mock.patch.object(probe.pathlib.Path, "iterdir", side_effect=observation),
                mock.patch.object(probe.time, "sleep"),
                contextlib.redirect_stdout(output),
            ):
                probe.worker(str(config))
            return json.loads(output.getvalue())

    def test_permission_denial_keeps_exit_status_and_resource_observations(self):
        report = self.observe(PermissionError("process is not inspectable"))
        self.assertEqual(report["exitCode"], 3)
        self.assertEqual(report["maximumRssKiB"], 123)
        self.assertEqual(report["descriptorPermissionDenials"], 1)
        self.assertEqual(report["descriptorSamples"], 0)
        self.assertIsNone(report["peakDescriptors"])

    def test_exited_process_does_not_become_a_zero_descriptor_measurement(self):
        report = self.observe(FileNotFoundError("process exited"))
        self.assertEqual(report["exitCode"], 3)
        self.assertEqual(report["descriptorPermissionDenials"], 0)
        self.assertEqual(report["descriptorSamples"], 0)
        self.assertIsNone(report["peakDescriptors"])

    def test_available_sample_is_recorded(self):
        report = self.observe([[Path("0"), Path("1")]])
        self.assertEqual(report["exitCode"], 3)
        self.assertEqual(report["descriptorSamples"], 1)
        self.assertEqual(report["peakDescriptors"], 2)


if __name__ == "__main__":
    unittest.main()
