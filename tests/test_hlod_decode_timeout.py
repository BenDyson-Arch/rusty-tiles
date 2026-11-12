"""The S5 HLOD oracles' decode.mjs limit: long enough for slow runners, overridable, and a
timeout is reported as an environment limit, never as an oracle verdict."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


S5 = Path(__file__).resolve().parents[1] / "bench" / "architecture_audit" / "mesh_hlod" / "s5"
sys.path.insert(0, str(S5))
import oracle
import source_manifest

# The exceptions oracle.py's main() reports as an `oracleDomain` refusal.
REFUSALS = (ValueError, OSError, subprocess.SubprocessError, KeyError, TypeError, IndexError)


class DecodeTimeoutTests(unittest.TestCase):
    def run_timing_out(self, module, call):
        seen = {}

        def fake_run(command, **kwargs):
            seen["timeout"] = kwargs["timeout"]
            raise subprocess.TimeoutExpired(command, kwargs["timeout"])

        with mock.patch.object(module.subprocess, "run", side_effect=fake_run):
            with self.assertRaises(module.DecodeTimeout) as caught:
                call()
        return seen["timeout"], str(caught.exception)

    def test_default_limit_is_600_seconds_and_named(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("RUSTY_TILES_ORACLE_TIMEOUT", None)
            limit, message = self.run_timing_out(oracle, lambda: oracle.decode(b"glTF"))
        self.assertEqual(limit, 600)
        self.assertIn("within 600 s", message)
        self.assertIn("RUSTY_TILES_ORACLE_TIMEOUT", message)

    def test_environment_overrides_the_limit(self):
        with mock.patch.dict(os.environ, {"RUSTY_TILES_ORACLE_TIMEOUT": "900"}):
            limit, message = self.run_timing_out(oracle, lambda: oracle.decode(Path("x.glb")))
            self.assertEqual(limit, 900)
            self.assertIn("within 900 s for x.glb", message)
        for bad in ("0", "-5", "soon"):
            with self.subTest(bad=bad), mock.patch.dict(os.environ, {"RUSTY_TILES_ORACLE_TIMEOUT": bad}):
                with self.assertRaisesRegex(ValueError, "positive number of seconds"):
                    oracle.decode(b"glTF")

    def test_a_timeout_is_not_a_refusal(self):
        for module in (oracle, source_manifest):
            with self.subTest(module=module.__name__):
                self.assertFalse(issubclass(module.DecodeTimeout, REFUSALS))
                self.assertFalse(issubclass(module.DecodeTimeout, oracle.Failure))

    def test_baseline_decode_shares_the_limit(self):
        with tempfile.NamedTemporaryFile(suffix=".glb") as glb, \
                mock.patch.dict(os.environ, {"RUSTY_TILES_ORACLE_TIMEOUT": "750"}):
            limit, message = self.run_timing_out(source_manifest, lambda: source_manifest._baseline_faces(glb.name))
        self.assertEqual(limit, 750)
        self.assertIn("within 750 s", message)
        self.assertIn("RUSTY_TILES_ORACLE_TIMEOUT", message)


if __name__ == "__main__":
    unittest.main()
