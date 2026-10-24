"""Exercise the installed native extension using only the Python standard library."""
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
import importlib.metadata
import hashlib
import json
import math
import os
from pathlib import Path
import inspect
import platform
import re
import shutil
import subprocess
import sqlite3
import struct
import sys
import tempfile
import unittest
import zipfile

import rusty_tiles

ROOT = Path(sys.argv.pop(1))
EXAMPLE = ROOT / "tests/fixtures/example.gltf"
BUDGET_MATRIX = json.loads((ROOT / "tests/fixtures/memory_budget_matrix.json").read_text())
# Optional: the CLI binary for the parity checks (the wheel runner keeps PATH empty).
CLI = os.environ.get("RUSTY_TILES_BIN") or None


def write_las(path, *, epsg=None):
    """Invented LAS 1.2, format 0, with four local XYZ points."""
    points = [(0, 0, 0), (100, 0, 0), (0, 100, 0), (100, 100, 100)]
    header = bytearray(227)
    header[:4] = b"LASF"
    header[24:26] = bytes((1, 2))
    vlr = b""
    if epsg is not None:
        keys = struct.pack("<12H", 1, 1, 0, 2, 1024, 0, 1, 2, 2048, 0, 1, epsg)
        vlr = struct.pack("<H16sHH32s", 0, b"LASF_Projection", 34735, len(keys), b"CRS") + keys
    struct.pack_into("<HII", header, 94, 227, 227 + len(vlr), bool(vlr))
    struct.pack_into("<BHI", header, 104, 0, 20, len(points))
    struct.pack_into("<I", header, 111, len(points))
    struct.pack_into("<3d", header, 131, 0.01, 0.01, 0.01)
    struct.pack_into("<6d", header, 179, 1, 0, 1, 0, 1, 0)
    with path.open("wb") as stream:
        stream.write(header)
        stream.write(vlr)
        for x, y, z in points:
            stream.write(struct.pack("<iiiHBBbBH", x, y, z, 7, 9, 2, 0, 0, 0))


def write_local_mesh(path, *, extras=False):
    """Hand-authored embedded GLB with three unindexed, untextured triangles."""
    points = [(x + dx, y, z) for dx in (0, 3, 6)
              for x, y, z in ((0, 0, 0), (1, 0, 0), (0, 1, 0))]
    payload = b"".join(struct.pack("<3f", *point) for point in points)
    document = {
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "translation": [10, 2, 3]}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "buffers": [{"byteLength": len(payload)}],
        "bufferViews": [{"buffer": 0, "byteLength": len(payload)}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 9,
                       "type": "VEC3", "min": [0, 0, 0], "max": [7, 1, 0]}],
    }
    if extras:
        document["extras"] = {"not_supported": True}
    encoded = json.dumps(document, separators=(",", ":")).encode()
    encoded += b" " * (-len(encoded) % 4)
    path.write_bytes(struct.pack("<III", 0x46546C67, 2, 28 + len(encoded) + len(payload))
                     + struct.pack("<II", len(encoded), 0x4E4F534A) + encoded
                     + struct.pack("<II", len(payload), 0x004E4942) + payload)


def write_grid_mesh(path, n=100):
    """Indexed, gently curved n x n grid (2 n^2 untextured triangles), enough for coarse HLOD tiles."""
    points = [struct.unpack("<3f", struct.pack("<3f", i * 0.5,
                                               math.sin(i * 0.5 * 0.31) + math.cos(j * 0.5 * 0.17),
                                               j * 0.5))
              for j in range(n + 1) for i in range(n + 1)]
    positions = b"".join(struct.pack("<3f", *point) for point in points)
    indices = bytearray()
    for j in range(n):
        for i in range(n):
            a = j * (n + 1) + i
            indices += struct.pack("<6I", a, a + n + 1, a + 1, a + 1, a + n + 1, a + n + 2)
    payload = positions + bytes(indices)
    low = [min(point[axis] for point in points) for axis in range(3)]
    high = [max(point[axis] for point in points) for axis in range(3)]
    document = {
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "buffers": [{"byteLength": len(payload)}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": len(positions)},
                        {"buffer": 0, "byteOffset": len(positions), "byteLength": len(indices)}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": (n + 1) ** 2, "type": "VEC3",
             "min": low, "max": high},
            {"bufferView": 1, "componentType": 5125, "count": 6 * n * n, "type": "SCALAR"}],
    }
    encoded = json.dumps(document, separators=(",", ":")).encode()
    encoded += b" " * (-len(encoded) % 4)
    path.write_bytes(struct.pack("<III", 0x46546C67, 2, 28 + len(encoded) + len(payload))
                     + struct.pack("<II", len(encoded), 0x4E4F534A) + encoded
                     + struct.pack("<II", len(payload), 0x004E4942) + payload)


class WheelAPI(unittest.TestCase):
    def test_tiler_installed_result_matches_archive_report_and_keeps_diagnostics(self):
        source = self.root / "tiler.glb"
        write_local_mesh(source)
        output = self.root / "tiler.3tz"
        events = []

        def observe(event):
            self.assertFalse(output.exists(), "required callback ran after installation")
            events.append(event)

        result = rusty_tiles.mesh_to_tileset(source, output, jobs=1, callback=observe)
        self.assertIsInstance(result, rusty_tiles.TilerResult)
        self.assertTrue(result.output.samefile(output))
        self.assertTrue(result.archive)
        self.assertEqual(result.cleanup_diagnostics, [])
        with zipfile.ZipFile(output) as archive:
            self.assertEqual(result.report, json.loads(archive.read("conversion.json")))
            self.assertIn("tileset.json", archive.namelist())
        report = result.report
        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(report["operation"], "mesh_tiler")
        self.assertEqual((report["tiles_version"], report["gltf_version"]), ("1.1", "2.0"))
        self.assertEqual(report["triangles"], 3)
        self.assertEqual(report["profile"], "balanced")
        self.assertEqual(report["source_coordinates"], "local-gltf")
        self.assertEqual(report["coordinates"], "local-gltf")
        self.assertIsNone(report["root_transform"])
        self.assertNotIn("diagnostics", report)
        self.assertNotIn("requested_jobs", report)
        self.assertEqual(result.diagnostics["leaf_delivery"]["jobs"], 1)
        self.assertEqual(result.diagnostics["archive_bytes"], output.stat().st_size)
        self.assertTrue(any(event.get("phase") == "ready_to_publish" for event in events))
        with self.assertRaises(AttributeError):
            result.output = self.root / "redirected.3tz"

    def test_tiler_provisional_profiles_and_worker_scratch_choices_preserve_bytes(self):
        source = self.root / "profiles.glb"
        write_local_mesh(source)
        for profile in ("fast", "balanced", "quality", "lossless"):
            with self.subTest(profile=profile):
                result = rusty_tiles.mesh_to_tileset(
                    source, self.root / f"profile-{profile}.3tz", profile=profile, jobs=1,
                )
                self.assertEqual(result.report["profile"], profile)
                self.assertEqual(result.report["triangles"], 3)
        first = self.root / "one-worker.3tz"
        second = self.root / "auto-workers.3tz"
        scratch = self.root / "scratch"
        scratch.mkdir()
        a = rusty_tiles.mesh_to_tileset(source, first, jobs=1)
        b = rusty_tiles.mesh_to_tileset(source, second, jobs=None, scratch_dir=scratch)
        self.assertEqual(first.read_bytes(), second.read_bytes())
        self.assertEqual(a.report, b.report)
        self.assertEqual(list(scratch.iterdir()), [])
        self.assertGreaterEqual(b.diagnostics["leaf_delivery"]["jobs"], 1)

    def test_tiler_source_crs_matches_the_cli_contract(self):
        source = self.root / "projected.glb"
        write_local_mesh(source)
        outputs = []
        for offset in ((14812000.0, -1384000.0, 100.0), [14812000, -1384000, 100]):
            output = self.root / f"crs-{len(outputs)}.3tz"
            result = rusty_tiles.mesh_to_tileset(
                source, output, jobs=1, source_crs="EPSG:3857", source_axes="y-up",
                source_offset=offset, height_offset=0.0,
            )
            report = result.report
            self.assertEqual(report["source_coordinates"], "source-crs")
            self.assertEqual(report["coordinates"], "wgs84-ecef")
            self.assertEqual(report["placement"]["kind"], "wgs84")
            self.assertEqual(report["source_crs"]["definition"], "EPSG:3857")
            self.assertEqual(report["source_crs"]["axes"], "y-up")
            self.assertEqual(report["source_crs"]["source_offset"], [14812000.0, -1384000.0, 100.0])
            self.assertEqual(len(report["root_transform"]), 16)
            outputs.append(output.read_bytes())
        self.assertEqual(outputs[0], outputs[1])
        offset_file = self.root / "projected_offset.txt"
        offset_file.write_text("E: 14812000\nN: -1384000\nA: 100\n")
        defaults = rusty_tiles.mesh_to_tileset(
            source, self.root / "crs-file.3tz", jobs=1, source_crs="EPSG:3857",
            source_offset_file=offset_file,
        )
        self.assertEqual(defaults.report["source_crs"]["axes"], "y-up")
        self.assertEqual(defaults.report["source_crs"]["height_offset_metres"], 0.0)
        self.assertEqual((self.root / "crs-file.3tz").read_bytes(), outputs[0])
        offset_file.write_text("E: 1\nN: 2\n")
        with self.assertRaises(rusty_tiles.InvalidRequestError):
            rusty_tiles.mesh_to_tileset(source, self.root / "bad.3tz", source_crs="EPSG:3857",
                                        source_offset_file=offset_file)
        local = rusty_tiles.mesh_to_tileset(source, self.root / "local.3tz", source_crs="local")
        self.assertNotIn("source_crs", local.report)

    def test_tiler_python_request_errors_precede_source_io_and_callbacks(self):
        source = self.root / "missing.glb"
        output = self.root / "invalid.3tz"
        output.write_bytes(b"previous")
        events = []
        for options in (
            {"profile": "unknown"}, {"jobs": 0}, {"jobs": -1}, {"jobs": 2**128},
            {"orientation_xyzw": (0, 0, 0, 1)}, {"scene_offset": (0, 0, 0)},
            {"anchor": (153, -27, 0), "orientation_xyzw": (0, 0, 0, 2)},
            {"source_crs": "auto", "source_axes": "y-up", "height_offset": 0.0},
            {"source_crs": "EPSG:3857", "source_offset": (1.0, 2.0),
             "source_offset_file": "offset.txt"},
            {"source_offset_file": "offset.txt"},
            {"source_crs": "EPSG:3857", "source_axes": "z-up", "height_offset": 0.0},
            {"source_crs": "EPSG:3857", "source_axes": "y-up", "height_offset": 0.0,
             "source_offset": (1.0,)},
            {"source_axes": "xyz"}, {"source_offset": (1.0, 2.0)}, {"height_offset": 0.0},
            {"source_crs": "EPSG:3857", "source_axes": "y-up", "height_offset": 0.0,
             "anchor": (153, -27, 0)},
        ):
            with self.subTest(options=options):
                with self.assertRaises(rusty_tiles.InvalidRequestError) as caught:
                    rusty_tiles.mesh_to_tileset(source, output, force=True,
                                                callback=events.append, **options)
                self.assertEqual(caught.exception.kind, "invalid_request")
                self.assertEqual(output.read_bytes(), b"previous")
                self.assertEqual(events, [])
        for jobs in (True, False, "auto", 1.0):
            with self.subTest(jobs=jobs), self.assertRaises(TypeError):
                rusty_tiles.mesh_to_tileset(source, output, jobs=jobs)
        for keyword, value in (
            ("leaf_triangles", 1), ("root_proxy_triangles", 1),
            ("max_proxy_error_metres", 1), ("source_offset", "1 2"),
            ("texture_format", "jpeg"), ("explicit", True),
        ):
            with self.subTest(keyword=keyword), self.assertRaises(TypeError):
                rusty_tiles.mesh_to_tileset(source, output, **{keyword: value})
        with self.assertRaises(TypeError):
            rusty_tiles.mesh_to_tileset(source, output, callback=42)

    def test_tiler_output_is_3tz_only_and_preserves_existing_entry(self):
        source = self.root / "missing.glb"
        for suffix in (".zip", ".3dtiles.zip", ".glb", ""):
            output = self.root / ("invalid" + suffix)
            output.write_bytes(b"prior")
            with self.subTest(suffix=suffix), self.assertRaises(rusty_tiles.InvalidRequestError) as caught:
                rusty_tiles.mesh_to_tileset(source, output, force=True)
            self.assertEqual(caught.exception.kind, "invalid_request")
            self.assertEqual(output.read_bytes(), b"prior")

    def test_tiler_memory_budget_matrix_matches_the_core_rules_before_source_io(self):
        source = self.root / "missing.glb"
        output = self.root / "budget.3tz"
        output.write_bytes(b"previous")
        events = []
        integers = lambda row: re.fullmatch(r"-?\d+", row["text"])
        invalid = BUDGET_MATRIX["invalid"]
        self.assertGreaterEqual(len([row for row in invalid if integers(row)]), 4)
        for row in invalid:
            text = row["text"]
            for lod in ("hlod", "h17"):
                with self.subTest(text=text, lod=lod):
                    if integers(row):
                        # Zero, negative and overflowing integers are core refusals.
                        with self.assertRaises(rusty_tiles.InvalidRequestError) as caught:
                            rusty_tiles.mesh_to_tileset(
                                source, output, force=True, callback=events.append,
                                memory_budget_bytes=int(text), _lod=lod)
                        self.assertEqual(caught.exception.kind, "invalid_request")
                        self.assertIn("memory budget", str(caught.exception))
                    else:
                        # Python takes integers only: suffix text is a type error.
                        with self.assertRaises(TypeError):
                            rusty_tiles.mesh_to_tileset(
                                source, output, force=True, callback=events.append,
                                memory_budget_bytes=text, _lod=lod)
                    self.assertEqual(output.read_bytes(), b"previous")
                    self.assertEqual(events, [])
        for value in (True, False, 1.0, 2.5, b"1", [1], "1"):
            with self.subTest(value=value), self.assertRaises(TypeError):
                rusty_tiles.mesh_to_tileset(source, output, memory_budget_bytes=value)
        for row in BUDGET_MATRIX["valid"]:
            if not re.fullmatch(r"\d+", row["text"]):
                continue
            with self.subTest(valid=row["text"]):
                # Accepted: the run proceeds to the source, which is missing.
                with self.assertRaises(rusty_tiles.TilesIOError):
                    rusty_tiles.mesh_to_tileset(
                        source, output, force=True, memory_budget_bytes=int(row["bytes"]),
                        _lod="hlod")
        # The legacy default method does not enforce a budget, so it refuses one, ahead of
        # the (missing) source and an output that fails its own rules.
        for budget in (2 << 30, 4 << 30):
            with self.subTest(default_method=budget), \
                    self.assertRaises(rusty_tiles.InvalidRequestError) as caught:
                rusty_tiles.mesh_to_tileset(source, self.root / "x.txt",
                                            memory_budget_bytes=budget)
            self.assertIn("memory budget", str(caught.exception))
        with self.assertRaises(rusty_tiles.InvalidRequestError):
            rusty_tiles.mesh_to_tileset(source, output, _lod="octree")

    def test_tiler_hlod_installed_run_reports_budget_and_keeps_callback_semantics(self):
        source = self.root / "grid.glb"
        write_grid_mesh(source)
        events = []
        output = self.root / "hlod.3tz"

        def observe(event):
            self.assertFalse(output.exists(), "required callback ran after installation")
            events.append(event)

        result = rusty_tiles.mesh_to_tileset(
            source, output, jobs=1, memory_budget_bytes=2 << 30, callback=observe, _lod="hlod")
        budget = result.diagnostics["memory"]["budget"]
        self.assertEqual((budget["requested_bytes"], budget["resolved_bytes"], budget["source"]),
                         (2 << 30, 2 << 30, "explicit"))
        self.assertGreater(result.diagnostics["memory"]["minimum_bytes"], 0)
        self.assertGreater(result.diagnostics["hlod"]["tiles"], 0)
        self.assertGreater(result.report["tiles"], result.report["leaves"])
        with zipfile.ZipFile(output) as archive:
            self.assertEqual(result.report, json.loads(archive.read("conversion.json")))
        self.assertTrue(any(event.get("phase") == "hlod_geometry" for event in events))
        self.assertTrue(any(event.get("phase") == "ready_to_publish" for event in events))
        # An unset budget resolves the documented default and never exceeds 4 GiB.
        defaulted = rusty_tiles.mesh_to_tileset(
            source, self.root / "default.3tz", jobs=1, _lod="hlod")
        default_budget = defaulted.diagnostics["memory"]["budget"]
        self.assertIsNone(default_budget["requested_bytes"])
        self.assertLessEqual(default_budget["resolved_bytes"], 4 << 30)
        self.assertIn(default_budget["source"],
                      ("default_ceiling", "default_headroom", "default_undetected"))
        # A budget is a ceiling on admission, never an input to the output.
        self.assertEqual(output.read_bytes(), (self.root / "default.3tz").read_bytes())
        # A callback exception still wins before publication, and nothing is installed.
        failure = RuntimeError("hlod callback")

        def fail(event):
            if event.get("phase") == "hlod_geometry":
                raise failure

        aborted = self.root / "aborted.3tz"
        with self.assertRaises(RuntimeError) as caught:
            rusty_tiles.mesh_to_tileset(source, aborted, jobs=1, callback=fail, _lod="hlod")
        self.assertIs(caught.exception, failure)
        self.assertFalse(aborted.exists())
        # Below the stage minimum the run is a typed resource refusal that publishes nothing.
        refused = self.root / "refused.3tz"
        with self.assertRaises(rusty_tiles.ResourceLimitError) as caught:
            rusty_tiles.mesh_to_tileset(source, refused, jobs=1, memory_budget_bytes=1 << 20,
                                        _lod="hlod")
        self.assertEqual(caught.exception.kind, "resource_limit")
        self.assertFalse(refused.exists())
        self.assertFalse(any(path.name.startswith(".tiles-") for path in self.root.iterdir()))

    @unittest.skipUnless(CLI, "set RUSTY_TILES_BIN for CLI/Python parity")
    def test_tiler_cli_and_python_agree_on_hlod_archive_and_reports(self):
        source = self.root / "parity.glb"
        write_grid_mesh(source)
        cli_output = self.root / "cli.3tz"
        completed = subprocess.run(
            [CLI, "--json", "mesh", "-i", str(source), "-o", str(cli_output), "--lod", "hlod",
             "--jobs", "1", "--memory-budget", "2GiB"],
            capture_output=True, check=True, timeout=300)
        envelope = json.loads(completed.stdout)
        python = rusty_tiles.mesh_to_tileset(
            source, self.root / "python.3tz", jobs=1, memory_budget_bytes=2 << 30, _lod="hlod")
        self.assertEqual((self.root / "python.3tz").read_bytes(), cli_output.read_bytes())
        self.assertEqual(python.report, envelope["tilerReport"])
        with zipfile.ZipFile(cli_output) as archive:
            self.assertEqual(envelope["tilerReport"], json.loads(archive.read("conversion.json")))
            members = sorted(archive.namelist())
        with zipfile.ZipFile(self.root / "python.3tz") as archive:
            self.assertEqual(sorted(archive.namelist()), members)
        self.assertTrue(any(name.endswith(".glb") for name in members))
        # Diagnostics are runtime facts (times, scratch paths); the budget and the stage plan
        # they report are not.
        for key in ("budget", "minimum_bytes", "capture_minimum_bytes", "leaf_minimum_bytes",
                    "archive_minimum_bytes"):
            self.assertEqual(python.diagnostics["memory"][key], envelope["diagnostics"]["memory"][key], key)
        self.assertEqual(python.diagnostics["memory"]["hlod"]["minimum_bytes"],
                         envelope["diagnostics"]["memory"]["hlod"]["minimum_bytes"])
        self.assertEqual(python.diagnostics["hlod"]["tiles"], envelope["diagnostics"]["hlod"]["tiles"])
        self.assertEqual(python.diagnostics["archive_bytes"], envelope["diagnostics"]["archive_bytes"])

    def test_tiler_documented_signatures_name_only_installed_parameters(self):
        installed = set(inspect.signature(rusty_tiles.mesh_to_tileset).parameters)
        documented = {"input", "output", "profile", "jobs", "memory_budget_bytes", "scratch_dir",
                      "anchor", "orientation_xyzw", "scene_offset", "source_crs", "source_axes",
                      "source_offset", "source_offset_file", "height_offset", "force", "callback"}
        # The private selector is installed but is never documented.
        self.assertEqual(installed - documented, {"_lod"})
        self.assertLessEqual(documented, installed)
        for name in ("docs/MESH.md", "bindings/python/README.md"):
            text = (ROOT / name).read_text()
            self.assertNotIn("_lod", text, name)
            signature = re.search(r"mesh_to_tileset\(input, output, \*,([^)]*)\)", text, re.S)
            self.assertIsNotNone(signature, name)
            names = re.findall(r"(\w+)=", signature.group(1))
            self.assertEqual(set(names), documented - {"input", "output"}, name)

    def test_tiler_typed_source_failures_and_conflict_preserve_destination(self):
        source = self.root / "source.glb"
        output = self.root / "existing.3tz"
        output.write_bytes(b"previous")
        with self.assertRaises(rusty_tiles.TilesIOError) as caught:
            rusty_tiles.mesh_to_tileset(source, output, force=True, jobs=1)
        self.assertEqual(caught.exception.kind, "io")
        source.write_bytes(b"invalid glTF")
        with self.assertRaises(rusty_tiles.DataError) as caught:
            rusty_tiles.mesh_to_tileset(source, output, force=True, jobs=1)
        self.assertEqual(caught.exception.kind, "invalid_input")
        write_local_mesh(source, extras=True)
        with self.assertRaises(rusty_tiles.UnsupportedError) as caught:
            rusty_tiles.mesh_to_tileset(source, output, force=True, jobs=1)
        self.assertEqual(caught.exception.kind, "unsupported")
        write_local_mesh(source)
        with self.assertRaises(rusty_tiles.OutputExistsError) as caught:
            rusty_tiles.mesh_to_tileset(source, output, jobs=1)
        self.assertEqual(caught.exception.kind, "output_conflict")
        self.assertEqual(output.read_bytes(), b"previous")
        result = rusty_tiles.mesh_to_tileset(source, output, force=True, jobs=1)
        with zipfile.ZipFile(result.output) as archive:
            self.assertEqual(result.report, json.loads(archive.read("conversion.json")))

    def test_tiler_callbacks_keep_the_original_winning_exception_before_publication(self):
        source = self.root / "callback.glb"
        write_local_mesh(source)
        observed = []
        rusty_tiles.mesh_to_tileset(source, self.root / "baseline.3tz", jobs=1,
                                    callback=observed.append)
        self.assertTrue(observed)
        self.assertTrue(any(event.get("phase") == "ready_to_publish" for event in observed))
        for final in (False, True):
            for replace in (False, True):
                output = self.root / f"abort-{final}-{replace}.3tz"
                if replace:
                    output.write_bytes(b"original")
                failure = RuntimeError(f"tiler callback {final}/{replace}")

                def fail(event):
                    if not final or event.get("phase") == "ready_to_publish":
                        raise failure

                with self.assertRaises(RuntimeError) as caught:
                    rusty_tiles.mesh_to_tileset(source, output, jobs=1, force=replace,
                                                callback=fail)
                self.assertIs(caught.exception, failure)
                self.assertEqual(output.read_bytes() if replace else output.exists(),
                                 b"original" if replace else False)
        self.assertFalse(any(path.name.startswith(".tiles-") for path in self.root.iterdir()))

    def test_tiler_placement_and_callback_cwd_changes_use_bound_output(self):
        source = self.root / "placed.glb"
        write_local_mesh(source)
        other = self.root / "other"
        other.mkdir()
        scratch = self.root / "scratch"
        scratch.mkdir()
        original_cwd = Path.cwd()
        try:
            os.chdir(self.root)

            def move_cwd(event):
                os.chdir(other)

            result = rusty_tiles.mesh_to_tileset(
                Path("placed.glb"), Path("bound.3tz"), jobs=1,
                scratch_dir=Path("scratch"),
                anchor=(153, -27, 25), orientation_xyzw=(0, 0, 0, 1 + 1e-13),
                scene_offset=(10, 2, -5), callback=move_cwd,
            )
        finally:
            os.chdir(original_cwd)
        self.assertTrue(result.output.samefile(self.root / "bound.3tz"))
        self.assertFalse((other / "bound.3tz").exists())
        self.assertEqual(list(scratch.iterdir()), [])
        placement = result.report["placement"]
        self.assertEqual(placement["kind"], "wgs84")
        self.assertEqual(placement["anchor_degrees_metres"], [153, -27, 25])
        self.assertEqual(placement["orientation_xyzw"], [0, 0, 0, 1])
        self.assertEqual(placement["scene_offset_metres"], [10, 2, -5])
        with zipfile.ZipFile(result.output) as archive:
            transform = json.loads(archive.read("tileset.json"))["root"]["transform"]
            self.assertEqual(len(transform), 16)
            self.assertEqual(result.report["root_transform"], transform)
        self.assertEqual(result.report["coordinates"], "wgs84-ecef")

    def test_d1_native_capability_is_explicit_and_leaves_no_output(self):
        output = self.root / "d1-raster"
        events = []
        with self.assertRaises(rusty_tiles.UnsupportedError) as caught:
            rusty_tiles.raster_tile_to_directory(
                ROOT / "tests/fixtures/d1-rgb.tif", output,
                zoom=3, x=5, y=2, callback=events.append,
            )
        self.assertEqual(caught.exception.kind, "unsupported")
        self.assertIn("native-geospatial", str(caught.exception))
        self.assertIsNone(caught.exception.recovery)
        self.assertFalse(output.exists())
        self.assertEqual(events, [])

    def test_directory_replace_default_wheel_preserves_old_entry(self):
        output = self.root / "existing-raster"
        output.mkdir()
        (output / "old").write_bytes(b"original")
        with self.assertRaises(rusty_tiles.UnsupportedError) as caught:
            rusty_tiles.raster_tile_to_directory(
                ROOT / "tests/fixtures/d1-rgb.tif", output,
                zoom=3, x=5, y=2, force=True,
            )
        self.assertIsNone(caught.exception.recovery)
        self.assertEqual((output / "old").read_bytes(), b"original")
        self.assertEqual(list(output.iterdir()), [output / "old"])

    def test_d1_invalid_request_precedes_capability(self):
        with self.assertRaises(rusty_tiles.InvalidRequestError) as caught:
            rusty_tiles.raster_tile_to_directory(
                ROOT / "tests/fixtures/d1-rgb.tif", self.root / "invalid-d1",
                zoom=25, x=0, y=0,
            )
        self.assertEqual(caught.exception.kind, "invalid_request")

    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.root = Path(self.work.name)

    def test_vector_geojson_options_progress_reuse_and_atomic_failure(self):
        source = self.root / "features.geojson"
        source.write_text(json.dumps({"type": "FeatureCollection", "features": [
            {"type": "Feature", "id": i, "properties": {"keep": i, "name": str(i)},
             "geometry": {"type": "Point", "coordinates": [i, i, 7]}}
            for i in range(4)
        ]}))
        events = []
        output = self.root / "vectors.3tz"
        options = dict(source_crs="local", where_clause="keep >= 1", fields=["name"],
                       max_features=1, lod_levels=1, reproducible=True, jobs=2,
                       quantize=True, meshopt=True)
        result = rusty_tiles.vector_to_3tz(source, output, callback=events.append, **options)
        self.assertEqual(result.report["features"], 3)
        self.assertTrue(rusty_tiles.validate(output)["ok"])
        self.assertTrue(any(e["event"] == "progress" for e in events))
        reused = rusty_tiles.vector_to_3tz(
            source, self.root / "reused.3tz", reuse_tileset=output, **options,
        )
        self.assertEqual(reused.report["reuse"]["rebuiltContents"], 0)
        self.assertGreater(reused.report["reuse"]["reusedContents"], 0)
        self.assertTrue(rusty_tiles.validate(reused.output)["ok"])
        before = output.read_bytes()
        with self.assertRaises(rusty_tiles.OutputExistsError):
            rusty_tiles.vector_to_3tz(source, output, **options)
        source.write_text('{"type":"FeatureCollection","features":[')
        with self.assertRaises(rusty_tiles.DataError):
            rusty_tiles.vector_to_3tz(source, output, force=True, **options)
        self.assertEqual(output.read_bytes(), before)

    def test_vector_callback_failure_aborts_skip_invalid_and_preserves_output(self):
        source = self.root / "callback.geojson"
        source.write_text(json.dumps({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {},
             "geometry": {"type": "Point", "coordinates": [1, 2, 3]}}
        ]}))
        output = self.root / "callback.3tz"
        output.write_bytes(b"existing output")
        marker = RuntimeError("vector observer fault")

        def fail(_event):
            raise marker

        with self.assertRaises(RuntimeError) as raised:
            rusty_tiles.vector_to_3tz(source, output, source_crs="local",
                                     skip_invalid=True, force=True, callback=fail)
        self.assertIs(raised.exception, marker)
        self.assertEqual(output.read_bytes(), b"existing output")
        self.assertEqual(list(self.root.glob(".vector-work-*")), [])

    def test_vector_geopackage_readonly_height_and_layer_selection(self):
        source = self.root / "survey.gpkg"
        with closing(sqlite3.connect(source)) as db, db:
            db.executescript("""
                PRAGMA application_id=1196444487;
                CREATE TABLE gpkg_spatial_ref_sys(srs_name TEXT,srs_id INTEGER PRIMARY KEY,
                  organization TEXT,organization_coordsys_id INTEGER,definition TEXT,description TEXT);
                INSERT INTO gpkg_spatial_ref_sys VALUES('WGS 84',4326,'EPSG',4326,
                  'GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]','');
                CREATE TABLE gpkg_contents(table_name TEXT PRIMARY KEY,data_type TEXT,
                  identifier TEXT,description TEXT,last_change TEXT,min_x REAL,min_y REAL,
                  max_x REAL,max_y REAL,srs_id INTEGER);
                INSERT INTO gpkg_contents(table_name,data_type,identifier,srs_id)
                  VALUES('survey','features','survey',4326);
                CREATE TABLE gpkg_geometry_columns(table_name TEXT,column_name TEXT,
                  geometry_type_name TEXT,srs_id INTEGER,z INTEGER,m INTEGER);
                INSERT INTO gpkg_geometry_columns VALUES('survey','geom','POINT',4326,1,0);
                CREATE TABLE survey(fid INTEGER PRIMARY KEY,geom BLOB,name TEXT);
            """)
            geometry = b"GP\x00\x01" + struct.pack("<iBI3d", 4326, 1, 1001, 0, 0, 123)
            db.execute("INSERT INTO survey VALUES(7,?,'height')", (geometry,))
        before = source.read_bytes()
        output = self.root / "survey.3tz"
        result = rusty_tiles.vector_to_3tz(
            source, output, layers=["survey"], height_offset=7,
        )
        self.assertEqual(result.report["features"], 1)
        self.assertEqual(result.report["layers"][0]["heightMode"], "explicit offset")
        self.assertTrue(rusty_tiles.validate(output)["ok"])
        with zipfile.ZipFile(output) as archive:
            tileset = json.loads(archive.read("tileset.json"))
        self.assertAlmostEqual(tileset["root"]["transform"][12], 6378137 + 130, delta=0.001)
        self.assertEqual(source.read_bytes(), before)
        self.assertFalse(Path(str(source) + "-wal").exists())

    def test_vector_unsupported_crs_and_height_refusals(self):
        source = self.root / "points.geojson"
        source.write_text(json.dumps({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {},
             "geometry": {"type": "Point", "coordinates": [0, 0, 0]}}
        ]}))
        output = self.root / "rejected.3tz"
        with self.assertRaisesRegex(rusty_tiles.UnsupportedError, "native-geospatial"):
            rusty_tiles.vector_to_3tz(source, output, source_crs="EPSG:26910", height_offset=0)
        self.assertFalse(output.exists())
        with self.assertRaises(rusty_tiles.DataError):
            rusty_tiles.vector_to_3tz(source, output, source_crs="local", height_offset=7)
        self.assertFalse(output.exists())

    def test_readme_example_in_clean_environment(self):
        self.assertEqual(os.environ["PATH"], "")
        installed_root = Path(os.environ["RUSTY_TILES_ACCEPTANCE_PACKAGE_ROOT"]).resolve()
        self.assertTrue(Path(rusty_tiles.__file__).resolve().is_relative_to(installed_root))
        self.assertEqual(rusty_tiles.__version__, importlib.metadata.version("rusty-tiles"))
        self.assertFalse(importlib.metadata.requires("rusty-tiles"))
        readme = (ROOT / "bindings/python/README.md").read_text()
        example = readme.split("```python\n", 1)[1].split("```", 1)[0]
        write_local_mesh(self.root / "example.glb")
        previous = Path.cwd()
        try:
            os.chdir(self.root)
            exec(example, {})
        finally:
            os.chdir(previous)
        with zipfile.ZipFile(self.root / "example.3tz") as archive:
            self.assertIn("tileset.json", archive.namelist())

    def test_pack_and_convert(self):
        source = self.root / "model.glb"
        write_local_mesh(source)
        result = rusty_tiles.glb_to_3tz(
            str(source), self.root / "wrapped.3tz", anchor=(153.02, -27.47, 0),
            orientation_xyzw=(0, 0, 0, 1),
        )
        self.assertIsInstance(result, rusty_tiles.ModelWrapResult)
        self.assertEqual(result.report["profile"], "w1-static-model-v1")
        with zipfile.ZipFile(result.output) as archive:
            self.assertEqual(archive.read("model/source.glb"), source.read_bytes())
            self.assertEqual(json.loads(archive.read("conversion.json")), result.report)
        check = rusty_tiles.validate(result.output)
        self.assertEqual(check["contentReferences"], 1)
        extracted = self.root / "extracted"
        with zipfile.ZipFile(result.output) as archive:
            archive.extractall(extracted)
        # Generated index entries are not ordinary source members. Repacking
        # explicitly selects the extracted resources rather than a stale index.
        (extracted / "@3dtilesIndex1@").unlink()
        repacked = rusty_tiles.convert_to_3tz(extracted / "tileset.json", self.root / "repacked.3tz")
        self.assertTrue(rusty_tiles.validate(repacked.output)["ok"])
        self.assertIsInstance(repacked, rusty_tiles.PackageResult)
        self.assertTrue(repacked.archive)

    def test_model_products_capture_publication_and_reference_authority(self):
        source = self.root / "model space.glb"
        write_local_mesh(source)
        original = source.read_bytes()
        manifest = rusty_tiles.model_to_manifest(source)
        self.assertIsInstance(manifest, rusty_tiles.ModelManifestResult)
        self.assertEqual(manifest.output.name, "tileset.json")
        self.assertTrue(manifest.output.parent.samefile(source.parent))
        self.assertTrue(manifest.output.samefile(self.root / "tileset.json"))
        document = json.loads(manifest.output.read_bytes())
        self.assertEqual(document["root"]["content"]["uri"], "model%20space.glb")
        self.assertEqual(document["root"]["geometricError"], 0)
        self.assertGreaterEqual(document["geometricError"], 1)
        self.assertEqual(document["root"]["refine"], "REPLACE")
        self.assertEqual(manifest.report["root_geometric_error_metres"], 0)
        self.assertEqual(manifest.report["tileset_geometric_error_metres"], document["geometricError"])
        self.assertEqual(source.read_bytes(), original)
        before = manifest.output.read_bytes()
        error = RuntimeError("stop before model publication")

        def fail(event):
            if event.get("phase") == "ready_to_publish":
                raise error

        with self.assertRaises(RuntimeError) as caught:
            rusty_tiles.model_to_manifest(source, force=True, callback=fail)
        self.assertIs(caught.exception, error)
        self.assertEqual(manifest.output.read_bytes(), before)
        archive = self.root / "captured.3tz"

        def remove_captured(event):
            if event.get("phase") == "model_capture":
                source.unlink()

        result = rusty_tiles.glb_to_3tz(source, archive, callback=remove_captured)
        with zipfile.ZipFile(result.output) as package:
            self.assertEqual(package.read("model/source.glb"), original)
        self.assertFalse(source.exists())
        self.assertFalse(any(path.name.startswith((".model-work-", ".mesh-work-", ".rusty-tiles-"))
                             for path in self.root.iterdir()))

    def test_wrapper_domain_reductions_and_pure_placement_refusals(self):
        output = self.root / "absent" / "out.3tz"
        with self.assertRaises(rusty_tiles.UnsupportedError):
            rusty_tiles.glb_to_3tz(EXAMPLE, output)
        self.assertFalse(output.parent.exists())
        with self.assertRaises(rusty_tiles.InvalidRequestError):
            rusty_tiles.glb_to_3tz(self.root / "missing.glb", output,
                                  orientation_xyzw=(0, 0, 0, 1))
        self.assertFalse(output.parent.exists())
        with self.assertRaises(TypeError):
            rusty_tiles.glb_to_3tz(EXAMPLE, output, cartographic=(0, 0, 0))

    def package_source(self, name="source"):
        source = self.root / name
        source.mkdir()
        members = {
            "tileset.json": b'{"asset":{"version":"1.1"},"root":{}}\n',
            "opaque.bin": bytes(range(256)),
            "conversion.json": b'  {"producer":"source","custom":17}\n',
        }
        for member, payload in members.items():
            (source / member).write_bytes(payload)
        return source, members

    def test_package_receipt_members_and_precommit_events(self):
        source, members = self.package_source()
        output = self.root / "package.3tz"
        events = []

        def observe(event):
            self.assertFalse(output.exists(), "pack callback ran after installation")
            events.append(event)

        result = rusty_tiles.convert_to_3tz(source, output, callback=observe)
        self.assertIsInstance(result, rusty_tiles.PackageResult)
        self.assertTrue(result.archive)
        self.assertTrue(result.output.is_absolute())
        self.assertTrue(result.output.samefile(output))
        self.assertIsInstance(result.receipt, rusty_tiles.PackageReceipt)
        self.assertEqual(result.receipt.member_count, len(members))
        self.assertEqual(result.receipt.source_bytes, sum(map(len, members.values())))
        self.assertEqual(result.receipt.archive_bytes, output.stat().st_size)
        self.assertEqual(result.cleanup_diagnostics, [])
        self.assertGreaterEqual(len(events), 2)
        with zipfile.ZipFile(output) as archive:
            self.assertEqual(set(archive.namelist()), {*members, "@3dtilesIndex1@"})
            for member, payload in members.items():
                self.assertEqual(archive.read(member), payload)
        self.assert_package_index(output, members)

    def assert_package_index(self, output, members):
        """Independent stdlib reader: 24-byte MD5/uint64 records and ZIP offsets."""
        data = output.read_bytes()
        with zipfile.ZipFile(output) as archive:
            infos = archive.infolist()
            self.assertEqual(infos[0].filename, "tileset.json")
            self.assertEqual(infos[-1].filename, "@3dtilesIndex1@")
            self.assertTrue(all(info.compress_type == zipfile.ZIP_STORED for info in infos))
            index = archive.read("@3dtilesIndex1@")
            self.assertEqual(len(index), 24 * len(members))
            expected = {hashlib.md5(info.filename.encode("utf-8")).digest(): info
                        for info in infos[:-1]}
            self.assertEqual(len(expected), len(members))
            previous = None
            for low, high, offset in struct.iter_unpack("<QQQ", index):
                key = (low, high)
                if previous is not None:
                    self.assertGreater(key, previous, "index hashes are not strictly ordered")
                previous = key
                digest = struct.pack("<QQ", low, high)
                self.assertIn(digest, expected, "index hash does not identify a selected member")
                info = expected.pop(digest)
                self.assertEqual(offset, info.header_offset, "index offset differs from ZIP directory")
                self.assertEqual(data[offset:offset + 4], b"PK\x03\x04")
                name_length, extra_length = struct.unpack_from("<HH", data, offset + 26)
                name = data[offset + 30:offset + 30 + name_length]
                self.assertEqual(name.decode("utf-8"), info.filename)
                self.assertEqual(hashlib.md5(name).digest(), digest)
                payload_offset = offset + 30 + name_length + extra_length
                self.assertEqual(data[payload_offset:payload_offset + info.file_size], members[info.filename])
            self.assertEqual(expected, {})

    def test_package_independent_index_reader_detects_corrupt_hash_order_and_offset(self):
        source, members = self.package_source()
        output = self.root / "control-index.3tz"
        rusty_tiles.convert_to_3tz(source, output)
        self.assert_package_index(output, members)
        with zipfile.ZipFile(output) as archive:
            order = [info.filename for info in archive.infolist()[:-1]]
        for name, expected_error in [
            ("hash", "index hash does not identify"),
            ("order", "index hashes are not strictly ordered"),
            ("offset", "index offset differs"),
        ]:
            with self.subTest(corruption=name):
                candidate = self.root / f"bad-index-{name}.3tz"
                with zipfile.ZipFile(candidate, "w", compression=zipfile.ZIP_STORED) as archive:
                    for member in order:
                        archive.writestr(member, members[member])
                    # Correct offsets for this independently written ZIP keep
                    # each negative control isolated to its designated property.
                    records = sorted((*struct.unpack("<QQ", hashlib.md5(info.filename.encode()).digest()), info.header_offset)
                                     for info in archive.infolist())
                    index = b"".join(struct.pack("<QQQ", *record) for record in records)
                    if name == "hash":
                        index = b"\0" * 16 + index[16:]
                    elif name == "order":
                        index = index[24:48] + index[:24] + index[48:]
                    else:
                        index = index[:16] + struct.pack("<Q", 1) + index[24:]
                    archive.writestr("@3dtilesIndex1@", index)
                with self.assertRaisesRegex(AssertionError, expected_error):
                    self.assert_package_index(candidate, members)

    def test_package_early_and_final_callback_errors_preserve_exception_and_output(self):
        source, _ = self.package_source()
        observed = []
        rusty_tiles.convert_to_3tz(source, self.root / "control.3tz", callback=observed.append)
        self.assertGreaterEqual(len(observed), 2)
        for point in ("early", "final"):
            for force in (False, True):
                with self.subTest(point=point, force=force):
                    output = self.root / f"{point}-{force}.3tz"
                    original = b"previous destination bytes"
                    if force:
                        output.write_bytes(original)
                    failure = RuntimeError(f"pack observer failed at {point}")
                    calls = []

                    def fail(event):
                        if force:
                            self.assertEqual(output.read_bytes(), original)
                        else:
                            self.assertFalse(output.exists())
                        calls.append(event)
                        if point == "early" or event == observed[-1]:
                            raise failure

                    before = set(self.root.iterdir())
                    with self.assertRaises(RuntimeError) as caught:
                        rusty_tiles.convert_to_3tz(source, output, force=force, callback=fail)
                    self.assertIs(caught.exception, failure)
                    self.assertEqual(set(self.root.iterdir()), before)
                    self.assertEqual(len(calls), 1 if point == "early" else len(observed))
                    if force:
                        self.assertEqual(output.read_bytes(), original)
                    else:
                        self.assertFalse(output.exists())

    def test_package_nested_repeated_and_concurrent_calls_are_independent(self):
        source, members = self.package_source()
        nested = []
        outer_events = []
        outer_output = self.root / "outer.3tz"

        def observe(event):
            self.assertFalse(outer_output.exists())
            outer_events.append(event)
            if not nested:
                nested.append(rusty_tiles.convert_to_3tz(source, self.root / "nested.3tz"))

        outer = rusty_tiles.convert_to_3tz(source, outer_output, callback=observe)
        self.assertEqual(len(nested), 1)
        self.assertTrue(outer_events)
        failure = RuntimeError("independent failed invocation")

        def fail(event):
            raise failure

        with self.assertRaises(RuntimeError) as caught:
            rusty_tiles.convert_to_3tz(source, self.root / "failed.3tz", callback=fail)
        self.assertIs(caught.exception, failure)

        def convert(index):
            events = []
            output = self.root / f"parallel-package-{index}.3tz"

            def callback(event):
                self.assertFalse(output.exists())
                events.append(event)

            result = rusty_tiles.convert_to_3tz(source, output, callback=callback)
            return result, events

        with ThreadPoolExecutor(max_workers=2) as pool:
            concurrent = list(pool.map(convert, range(4)))
        self.assertEqual(len({result.output for result, _ in concurrent}), 4)
        for result in [outer, *nested, *(result for result, _ in concurrent)]:
            self.assertEqual(result.receipt.member_count, len(members))
            with zipfile.ZipFile(result.output) as archive:
                self.assertEqual(archive.read("conversion.json"), members["conversion.json"])
        for _, events in concurrent:
            self.assertEqual(events, outer_events)

    def test_package_invalid_requests_source_errors_and_output_conflicts(self):
        source, _ = self.package_source()
        output = self.root / "output.3tz"
        with self.assertRaises(TypeError):
            rusty_tiles.convert_to_3tz(source, output, callback=42)
        self.assertFalse(output.exists())
        with self.assertRaises(rusty_tiles.TilesIOError) as caught:
            rusty_tiles.convert_to_3tz(self.root / "missing", self.root / "not-created" / "output.3tz")
        self.assertEqual(caught.exception.kind, "io")
        self.assertFalse((self.root / "not-created").exists())
        (source / "@3dtilesIndex1@").write_bytes(b"stale generated index")
        with self.assertRaises(rusty_tiles.InvalidRequestError) as caught:
            rusty_tiles.convert_to_3tz(source, self.root / "not-created" / "output.3tz")
        self.assertEqual(caught.exception.kind, "invalid_request")
        self.assertFalse((self.root / "not-created").exists())
        (source / "@3dtilesIndex1@").unlink()
        rusty_tiles.convert_to_3tz(source, output)
        before = output.read_bytes()
        with self.assertRaises(rusty_tiles.OutputExistsError) as caught:
            rusty_tiles.convert_to_3tz(source, output)
        self.assertEqual(caught.exception.kind, "output_conflict")
        self.assertEqual(output.read_bytes(), before)
        with self.assertRaises(rusty_tiles.InvalidRequestError):
            rusty_tiles.convert_to_3tz(source, source / "overlap.3tz", force=True)
        self.assertFalse((source / "overlap.3tz").exists())
        with self.assertRaises(rusty_tiles.InvalidRequestError):
            rusty_tiles.convert_to_3tz(source, self.root / "bad-extension.zip")
        self.assertFalse((self.root / "bad-extension.zip").exists())

    def test_local_points_progress_and_force(self):
        source = self.root / "points.las"
        write_las(source)
        events = []
        output = self.root / "points.3tz"
        result = rusty_tiles.point_cloud_to_3tz(
            source, output, max_points=2, chunk_points=1, callback=events.append,
         source_crs="local",)
        self.assertEqual(result.report["points"], 4)
        self.assertTrue(rusty_tiles.validate(output)["ok"])
        phases = {e["phase"] for e in events if e["event"] == "progress"}
        self.assertTrue({"ingestion", "tiling"}.issubset(phases))
        before = output.read_bytes()
        with self.assertRaises(rusty_tiles.OutputExistsError):
            rusty_tiles.point_cloud_to_3tz(source, output, source_crs="local")
        self.assertEqual(output.read_bytes(), before)
        rusty_tiles.point_cloud_to_3tz(source, output, force=True, source_crs="local")
        self.assertTrue(rusty_tiles.validate(output)["ok"])

    def test_point_coordinates_are_required_before_callbacks(self):
        source = self.root / "coordinates.las"
        write_las(source)
        output = self.root / "coordinates.3tz"
        events = []
        with self.assertRaises(TypeError):
            rusty_tiles.point_cloud_to_3tz(source, output, callback=events.append)
        with self.assertRaises(rusty_tiles.InvalidRequestError):
            rusty_tiles.point_cloud_to_3tz(source, output, source_crs="local",
                                         height_offset=0, callback=events.append)
        self.assertEqual(events, [])
        self.assertFalse(output.exists())

    def test_retired_explicit_keyword_is_an_unknown_argument(self):
        # Explicit vector and point-cloud output retired in 0.4.0 (H6); the
        # keyword is rejected before any input or output work.
        source = self.root / "retired.las"
        write_las(source)
        output = self.root / "retired.3tz"
        events = []
        for value in (True, False):
            with self.assertRaisesRegex(TypeError, "explicit"):
                rusty_tiles.point_cloud_to_3tz(source, output, source_crs="local",
                                             explicit=value, callback=events.append)
            with self.assertRaisesRegex(TypeError, "explicit"):
                rusty_tiles.vector_to_3tz(self.root / "absent.geojson", output,
                                          source_crs="local", explicit=value,
                                          callback=events.append)
        self.assertEqual(events, [])
        self.assertFalse(output.exists())

    def test_point_callback_abort_preserves_output_and_reentry_is_isolated(self):
        source = self.root / "callback-points.las"
        write_las(source)
        output = self.root / "callback-points.3tz"
        rusty_tiles.point_cloud_to_3tz(source, output, source_crs="local")
        original = output.read_bytes()
        for exception in (RuntimeError("point observer failed"), KeyboardInterrupt()):
            def fail(event):
                raise exception
            with self.assertRaises(type(exception)) as caught:
                rusty_tiles.point_cloud_to_3tz(source, output, source_crs="local",
                                             force=True, callback=fail)
            self.assertIs(caught.exception, exception)
            self.assertEqual(output.read_bytes(), original)
        nested = self.root / "nested-points.3tz"
        nested_results = []
        def reenter(event):
            if not nested_results:
                nested_results.append(None)
                nested_results[0] = rusty_tiles.point_cloud_to_3tz(
                    source, nested, source_crs="local", max_points=1)
        result = rusty_tiles.point_cloud_to_3tz(source, output, source_crs="local",
                                               force=True, callback=reenter, max_points=2)
        self.assertEqual(result.report["points"], 4)
        self.assertEqual(nested_results[0].report["points"], 4)
        self.assertTrue(rusty_tiles.validate(output)["ok"])
        self.assertTrue(rusty_tiles.validate(nested)["ok"])
        self.assertEqual(result.cleanup_diagnostics, [])

    def test_georeferenced_points_and_unsupported_datum(self):
        source = self.root / "points.las"
        write_las(source)
        output = self.root / "globe.3tz"
        result = rusty_tiles.point_cloud_to_3tz(
            source, output, source_crs="EPSG:4326", height_offset=7,
        )
        self.assertEqual(result.report["resolvedCrs"], "EPSG:4326")
        self.assertTrue(rusty_tiles.validate(output)["ok"])
        with zipfile.ZipFile(output) as archive:
            tileset = json.loads(archive.read("tileset.json"))
        # Independently calculate the ECEF bounding-box centre used by the root.
        positions = []
        a = 6378137.0
        e2 = 1 - (1 - 1 / 298.257223563) ** 2
        for lon, lat, height in [(0, 0, 7), (1, 0, 7), (0, 1, 7), (1, 1, 8)]:
            lon, lat = math.radians(lon), math.radians(lat)
            n = a / math.sqrt(1 - e2 * math.sin(lat) ** 2)
            positions.append(((n + height) * math.cos(lat) * math.cos(lon),
                              (n + height) * math.cos(lat) * math.sin(lon),
                              (n * (1 - e2) + height) * math.sin(lat)))
        expected = [(min(p[i] for p in positions) + max(p[i] for p in positions)) / 2
                    for i in range(3)]
        for actual, expected in zip(tileset["root"]["transform"][12:15], expected):
            self.assertAlmostEqual(actual, expected, delta=0.001)
        rejected = self.root / "rejected.3tz"
        with self.assertRaisesRegex(rusty_tiles.UnsupportedError, "native-geospatial"):
            rusty_tiles.point_cloud_to_3tz(
                source, rejected, source_crs="EPSG:26910", height_offset=0,
            )
        self.assertFalse(rejected.exists())

    def test_las_header_crs_matches_explicit_crs(self):
        source = self.root / "header.las"
        write_las(source, epsg=4326)
        documents = []
        for crs in ("header", "EPSG:4326"):
            output = self.root / f"{crs.replace(':', '-')}.3tz"
            result = rusty_tiles.point_cloud_to_3tz(
                source, output, source_crs=crs, height_offset=7,
            )
            self.assertEqual(result.report["resolvedCrs"], "EPSG:4326")
            self.assertEqual(result.report["heightOffset"], 7)
            self.assertTrue(rusty_tiles.validate(output)["ok"])
            with zipfile.ZipFile(output) as archive:
                documents.append(json.loads(archive.read("tileset.json")))
        self.assertEqual(documents[0], documents[1])

    def test_georeferenced_points_require_height_decision(self):
        source = self.root / "points.las"
        write_las(source)
        output = self.root / "rejected.3tz"
        for height in (None, float("nan"), float("inf")):
            with self.subTest(height=height):
                with self.assertRaisesRegex(rusty_tiles.InvalidRequestError, "heightOffset"):
                    rusty_tiles.point_cloud_to_3tz(
                        source, output, source_crs="EPSG:4326", height_offset=height,
                    )
                self.assertFalse(output.exists())
        with self.assertRaisesRegex(rusty_tiles.DataError, "no CRS"):
            rusty_tiles.point_cloud_to_3tz(source, output, source_crs="header", height_offset=0)
        self.assertFalse(output.exists())

    def test_point_metadata_attributes_are_available_from_python(self):
        source = self.root / "points.las"
        write_las(source)
        output = self.root / "attributes.3tz"
        rusty_tiles.point_cloud_to_3tz(source, output, max_points=2,
                                     metadata_attributes=True, source_crs="local")
        self.assertTrue(rusty_tiles.validate(output)["ok"])
        with zipfile.ZipFile(output) as archive:
            for entry in archive.namelist():
                if not entry.endswith(".glb"):
                    continue
                payload = archive.read(entry)
                length = struct.unpack_from("<I", payload, 12)[0]
                gltf = json.loads(payload[20:20 + length])
                binary = payload[28 + length:]
                metadata = gltf["extensions"]["EXT_structural_metadata"]
                primitive = gltf["meshes"][0]["primitives"][0]
                self.assertEqual(primitive["extensions"]["EXT_structural_metadata"]["propertyAttributes"], [0])
                for name, expected, kind in (("classification", 2, "B"),
                                             ("intensity", 7, "H"),
                                             ("return_number", 1, "B")):
                    self.assertIn(name, metadata["propertyTables"][0]["properties"])
                    prop = metadata["propertyAttributes"][0]["properties"][f"vertex_{name}"]
                    accessor = gltf["accessors"][primitive["attributes"][prop["attribute"]]]
                    view = gltf["bufferViews"][accessor["bufferView"]]
                    for index in range(accessor["count"]):
                        at = view.get("byteOffset", 0) + index * view["byteStride"]
                        self.assertEqual(struct.unpack_from("<" + kind, binary, at)[0], expected)

    def test_typed_errors_and_failed_publication(self):
        for error in (rusty_tiles.DataError, rusty_tiles.EnvironmentError,
                      rusty_tiles.OutputExistsError, rusty_tiles.TilesIOError, rusty_tiles.UnsupportedError):
            self.assertTrue(issubclass(error, rusty_tiles.TilesError))
        output = self.root / "missing.3tz"
        with self.assertRaises(rusty_tiles.TilesIOError):
            rusty_tiles.point_cloud_to_3tz(self.root / "missing.las", output, source_crs="local")
        self.assertFalse(output.exists())
        bad = self.root / "bad.3tz"
        bad.write_bytes(b"invalid ZIP")
        with self.assertRaises(rusty_tiles.DataError):
            rusty_tiles.validate(bad)
        empty_package = self.root / "empty-package"
        empty_package.mkdir()
        with self.assertRaises(rusty_tiles.DataError):
            rusty_tiles.convert_to_3tz(empty_package, output)
        for removed in ("mesh_to_3tz", "mesh_local_to_3tz", "MeshResult", "ConversionResult"):
            self.assertFalse(hasattr(rusty_tiles, removed), removed)
        self.assertFalse(output.exists())

    def test_validate_reports_decoded_content_containment(self):
        def package(name, half, position, node):
            data = self.root / name
            data.mkdir()
            payload = struct.pack("<3f", *position)
            bound = list(struct.unpack("<3f", payload))
            document = json.dumps({
                "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
                "nodes": [node], "buffers": [{"byteLength": 12}],
                "bufferViews": [{"buffer": 0, "byteLength": 12}],
                "accessors": [{"bufferView": 0, "componentType": 5126, "type": "VEC3",
                               "count": 1, "min": bound, "max": bound}],
                "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}]}],
            }).encode()
            document += b" " * (-len(document) % 4)
            glb = (struct.pack("<4sII", b"glTF", 2, 28 + len(document) + 12)
                   + struct.pack("<I4s", len(document), b"JSON") + document
                   + struct.pack("<I4s", 12, b"BIN\0") + payload)
            (data / "a.glb").write_bytes(glb)
            (data / "tileset.json").write_text(json.dumps({
                "asset": {"version": "1.1"}, "geometricError": 0,
                "root": {"boundingVolume": {"box": [0, 0, 0, half[0], 0, 0, 0, half[1], 0, 0, 0, half[2]]},
                         "geometricError": 0, "content": {"uri": "a.glb"}},
            }))
            return rusty_tiles.convert_to_3tz(data, self.root / f"{name}.3tz").output

        # glTF (1, 2, 3) is (1, -3, 2) in the z-up tile frame: on the box faces.
        report = rusty_tiles.validate(package("inside", (1, 3, 2), (1, 2, 3), {"mesh": 0}))
        self.assertIn("decodedContentContainment", report["checks"])
        self.assertNotIn("decodedContentBounds", report["notInspected"])
        self.assertNotIn("scene transforms", report["notInspected"])
        self.assertIn("skins, animations, morph targets and rendered extent", report["notInspected"])
        # VS-A: payload rows are a list by default, so the report is plain JSON data; with
        # lazy_payloads=True they are a read-only sequence read on demand. The report keeps
        # the CLI's sorted key order and states no scratch when nothing spilled.
        self.assertEqual(list(report), sorted(report))
        self.assertNotIn("scratch", report)
        self.assertIsInstance(report["payloads"], list)
        self.assertEqual(json.loads(json.dumps(report)), report)
        [row] = report["payloads"]
        self.assertEqual(row["uri"], "a.glb")
        self.assertEqual(sorted(row), ["accessorsChecked", "primitivesChecked", "uri", "vertices"])
        lazy = rusty_tiles.validate(package("inside-lazy", (1, 3, 2), (1, 2, 3), {"mesh": 0}),
                                    lazy_payloads=True)
        rows = lazy["payloads"]
        self.assertEqual(type(rows).__name__, "ValidationPayloads")
        self.assertEqual(list(lazy), list(report))
        self.assertEqual(len(rows), 1)
        self.assertEqual((rows[0], rows[-1], rows[0:5], list(rows), list(rows)),
                         (row, row, [row], [row], [row]))
        for index in (1, -2):
            with self.assertRaises(IndexError):
                rows[index]
        with self.assertRaises(TypeError):
            json.dumps(lazy)
        self.assertEqual(json.loads(json.dumps({**lazy, "payloads": list(rows)}))["payloads"], [row])
        self.assertEqual(
            {key: report["limits"][key] for key in (
                "containmentSpanBits", "containmentWorkUnits", "containmentBytesRead",
                "containmentDecodedBytes", "containmentNodeDepth")},
            {"containmentSpanBits": 4096, "containmentWorkUnits": 2**29,
             "containmentBytesRead": 2**31, "containmentDecodedBytes": 2**31,
             "containmentNodeDepth": 64},
        )
        with self.assertRaises(rusty_tiles.DataError) as caught:
            rusty_tiles.validate(package("outside", (1, 3, 1.9999999999999998), (1, 2, 3), {"mesh": 0}))
        self.assertEqual(
            str(caught.exception),
            "decoded content outside bounding volume: content=a.glb depth=0 volume=own "
            "node=0 mesh=0 primitive=0 vertex=0",
        )
        report = rusty_tiles.validate(package("skinned", (1, 1, 1), (9, 9, 9), {"mesh": 0, "skin": 0}))
        self.assertNotIn("decodedContentContainment", report["checks"])
        self.assertIn("decodedContentBounds", report["notInspected"])
        self.assertIn("decodedContentBounds.skin", report["notInspected"])
        self.assertIn("scene transforms", report["notInspected"])

    def test_validate_limit_surface(self):
        # VS-C: opt-in caps are None, overrides are validated before any I/O, a limit refusal
        # carries its structured cause, and the callback sees progress and can stop the run.
        data = self.root / "limits"
        data.mkdir()
        payload = struct.pack("<3f", 0, 0, 0)
        document = json.dumps({
            "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0}], "buffers": [{"byteLength": 12}],
            "bufferViews": [{"buffer": 0, "byteLength": 12}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "type": "VEC3",
                           "count": 1, "min": [0, 0, 0], "max": [0, 0, 0]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}]}],
        }).encode()
        document += b" " * (-len(document) % 4)
        (data / "a.glb").write_bytes(
            struct.pack("<4sII", b"glTF", 2, 28 + len(document) + 12)
            + struct.pack("<I4s", len(document), b"JSON") + document
            + struct.pack("<I4s", 12, b"BIN\0") + payload)
        (data / "tileset.json").write_text(json.dumps({
            "asset": {"version": "1.1"}, "geometricError": 0,
            "root": {"boundingVolume": {"box": [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]},
                     "geometricError": 0, "content": {"uri": "a.glb"}},
        }))
        archive = rusty_tiles.convert_to_3tz(data, self.root / "limits.3tz").output
        with zipfile.ZipFile(archive) as opened:
            stored = sum(info.compress_size for info in opened.infolist())
        report = rusty_tiles.validate(archive)
        self.assertEqual(report["storedBytes"], stored)
        for key in ("sourceArchiveBytes", "archiveEntries", "centralDirectoryBytes",
                    "archiveStoredBytes"):
            self.assertIsNone(report["limits"][key])
        self.assertEqual(report["limits"]["liveDocumentBytes"], 2**28)
        self.assertEqual(report["limitDerivation"]["model"], 1)
        self.assertEqual(report["limitDerivation"]["budgets"]["hierarchyVisits"],
                         {"floor": 2**18, "perStoredByte": [1, 8], "source": "floor"})
        capped = rusty_tiles.validate(archive, max_volume={"archiveStoredBytes": stored,
                                                           "references": 2**64 - 1})
        self.assertEqual(capped["limits"]["archiveStoredBytes"], stored)
        self.assertEqual(capped["limitDerivation"]["budgets"]["references"]["source"], "explicit")
        missing = self.root / "absent.3tz"
        for bad in ({"jsonBytes": 1}, {"unknown": 1}, {"references": True},
                    {"references": -1}, {"references": 2**64}, {"references": 1.0},
                    {"references": "1"}, {1: 1}, [("references", 1)]):
            with self.assertRaises(rusty_tiles.InvalidRequestError, msg=repr(bad)):
                rusty_tiles.validate(missing, max_volume=bad)
        with self.assertRaises(rusty_tiles.InvalidRequestError):
            rusty_tiles.validate(missing, scratch_dir=self.root / "no-such-dir")
        with self.assertRaises(rusty_tiles.ResourceLimitError) as caught:
            rusty_tiles.validate(archive, max_volume={"archiveStoredBytes": stored - 1})
        self.assertEqual(caught.exception.limit, {"limit": "archiveStoredBytes",
                                                  "value": stored - 1, "observed": stored,
                                                  "derivation": "cap"})
        self.assertTrue(str(caught.exception).endswith(
            f"(limit archiveStoredBytes={stored - 1}, cap)"))
        events = []
        rusty_tiles.validate(archive, callback=events.append)
        self.assertEqual([event["phase"] for event in events],
                         ["validate.hash", "validate.traverse", "validate.stream",
                          "validate.rows"])

        def interrupt(event):
            raise KeyboardInterrupt

        with self.assertRaises(KeyboardInterrupt):
            rusty_tiles.validate(archive, callback=interrupt)

    def test_validate_reads_implicit_roots_with_rust_parity(self):
        """I1 parity with tests/i1_implicit_read.rs: labels, counts and categories."""
        def point_glb():
            document = json.dumps({
                "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
                "nodes": [{"mesh": 0}], "buffers": [{"byteLength": 12}],
                "bufferViews": [{"buffer": 0, "byteLength": 12}],
                "accessors": [{"bufferView": 0, "componentType": 5126, "type": "VEC3",
                               "count": 1, "min": [0, 0, 0], "max": [0, 0, 0]}],
                "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}]}],
            }).encode()
            document += b" " * (-len(document) % 4)
            return (struct.pack("<4sII", b"glTF", 2, 28 + len(document) + 12)
                    + struct.pack("<I4s", len(document), b"JSON") + document
                    + struct.pack("<I4s", 12, b"BIN\0") + bytes(12))

        def package(name, template):
            # QUADTREE L = 2, A = 2: five constant-available tiles with content.
            data = self.root / name
            (data / "s/0/0").mkdir(parents=True)
            subtree = json.dumps({"tileAvailability": {"constant": 1},
                                  "contentAvailability": [{"constant": 1}],
                                  "childSubtreeAvailability": {"constant": 0}}).encode()
            subtree += b" " * (-len(subtree) % 8)
            (data / "s/0/0/0.subtree").write_bytes(
                b"subt" + struct.pack("<IQQ", 1, len(subtree), 0) + subtree)
            for level, x, y in ((0, 0, 0), (1, 0, 0), (1, 1, 0), (1, 0, 1), (1, 1, 1)):
                (data / f"c/{level}/{x}").mkdir(parents=True, exist_ok=True)
                (data / f"c/{level}/{x}/{y}.glb").write_bytes(point_glb())
            (data / "tileset.json").write_text(json.dumps({
                "asset": {"version": "1.1"}, "geometricError": 16,
                "root": {"boundingVolume": {"box": [0, 0, 0, 8, 0, 0, 0, 8, 0, 0, 0, 8]},
                         "geometricError": 16, "refine": "REPLACE", "content": {"uri": template},
                         "implicitTiling": {"subdivisionScheme": "QUADTREE", "subtreeLevels": 2,
                                            "availableLevels": 2,
                                            "subtrees": {"uri": "s/{level}/{x}/{y}.subtree"}}},
            }))
            return rusty_tiles.convert_to_3tz(data, self.root / f"{name}.3tz").output

        report = rusty_tiles.validate(package("implicit", "c/{level}/{x}/{y}.glb"))
        self.assertEqual((report["tiles"], report["contentReferences"]), (5, 5))
        checks = report["checks"]
        self.assertEqual(checks.index("implicitAddressingAndAvailability"),
                         checks.index("hierarchyBounds") + 1)
        for label in ("recordedBudgets.implicitTileExtras", "decodedContentBounds.derivedImplicitVolume"):
            self.assertIn(label, report["notInspected"])
        self.assertNotIn("implicitAddressingAndAvailability", report["notInspected"])
        # X-1: a JSON content template is Unsupported at root admission.
        with self.assertRaises(rusty_tiles.UnsupportedError) as caught:
            rusty_tiles.validate(package("json-template", "c/{level}/{x}/{y}.json"))
        self.assertEqual(str(caught.exception),
                         "implicit: unsupported external tileset content template in tileset.json")

if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(WheelAPI)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    evidence = {
        "python": sys.version,
        "platform": platform.platform(),
        "machine": platform.machine(),
        "package_version": rusty_tiles.__version__,
        "package_file": rusty_tiles.__file__,
        "wheel_sha256": os.environ.get("RUSTY_TILES_ACCEPTANCE_WHEEL_SHA256"),
        "empty_path": os.environ.get("PATH") == "",
        "tests_run": result.testsRun,
        "failures": len(result.failures),
        "errors": len(result.errors),
        "skipped": len(result.skipped),
        "ok": result.wasSuccessful(),
    }
    if "bpy" in sys.modules:
        import bpy
        evidence["blender"] = bpy.app.version_string
        evidence["blender_version"] = list(bpy.app.version)
        evidence["blender_build_hash"] = bpy.app.build_hash.decode("ascii")
    if report := os.environ.get("RUSTY_TILES_ACCEPTANCE_REPORT"):
        destination = Path(report)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(json.dumps(evidence, indent=2) + "\n")
    raise SystemExit(0 if result.wasSuccessful() else 1)
