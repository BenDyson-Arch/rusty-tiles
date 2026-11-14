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
        for bad in ("0", "-5", "soon", "nan"):
            for module, call in ((oracle, lambda: oracle.decode(b"glTF")),
                                 (source_manifest, source_manifest._decode_timeout)):
                with self.subTest(bad=bad, module=module.__name__), \
                        mock.patch.dict(os.environ, {"RUSTY_TILES_ORACLE_TIMEOUT": bad}):
                    with self.assertRaisesRegex(module.OracleConfigError, "positive number of seconds"):
                        call()

    def test_a_timeout_or_bad_setting_is_not_a_refusal(self):
        for module in (oracle, source_manifest):
            for error in (module.DecodeTimeout, module.OracleConfigError):
                with self.subTest(module=module.__name__, error=error.__name__):
                    self.assertFalse(issubclass(error, REFUSALS))
                    self.assertFalse(issubclass(error, oracle.Failure))
                    self.assertFalse(issubclass(error, source_manifest.ManifestError))

    def test_meshopt_decode_shares_the_limit(self):
        glb = source_manifest._GLB.__new__(source_manifest._GLB)
        glb.baseline, glb.views, glb.bin_start, glb.bin_length = True, {}, 0, 16
        glb.data = bytes(16)
        glb.doc = {"bufferViews": [{"buffer": 0, "byteLength": 16, "extensions": {"EXT_meshopt_compression": {
            "buffer": 0, "byteOffset": 0, "byteLength": 16, "count": 4, "byteStride": 4, "mode": "ATTRIBUTES"}}}]}
        with mock.patch.dict(os.environ, {"RUSTY_TILES_ORACLE_TIMEOUT": "45"}):
            limit, message = self.run_timing_out(source_manifest, lambda: glb.view(0))
        self.assertEqual(limit, 45)
        self.assertIn("meshopt.py did not finish within 45 s for bufferView 0", message)
        self.assertIn("RUSTY_TILES_ORACLE_TIMEOUT", message)

    def test_meshopt_refusals_are_unchanged_without_a_timeout(self):
        # The real meshopt.py subprocess, on a stream that is not meshopt data: still the
        # ordinary manifest refusal.
        glb = source_manifest._GLB.__new__(source_manifest._GLB)
        glb.baseline, glb.views, glb.bin_start, glb.bin_length = True, {}, 0, 16
        glb.data = bytes(16)
        glb.doc = {"bufferViews": [{"buffer": 0, "byteLength": 16, "extensions": {"EXT_meshopt_compression": {
            "buffer": 0, "byteOffset": 0, "byteLength": 16, "count": 4, "byteStride": 4, "mode": "ATTRIBUTES"}}}]}
        with self.assertRaisesRegex(source_manifest.ManifestError, "independent meshopt decode failed"):
            glb.view(0)

    def test_baseline_decode_shares_the_limit(self):
        with tempfile.NamedTemporaryFile(suffix=".glb") as glb, \
                mock.patch.dict(os.environ, {"RUSTY_TILES_ORACLE_TIMEOUT": "750"}):
            limit, message = self.run_timing_out(source_manifest, lambda: source_manifest._baseline_faces(glb.name))
        self.assertEqual(limit, 750)
        self.assertIn("within 750 s", message)
        self.assertIn("RUSTY_TILES_ORACLE_TIMEOUT", message)


if __name__ == "__main__":
    unittest.main()
