"""Exercise the installed native extension using only the Python standard library."""
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
from fractions import Fraction
import importlib.metadata
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import sqlite3
import struct
import sys
import tempfile
import unittest
import zipfile

import rusty_tiles

ROOT = Path(sys.argv.pop(1))
EXAMPLE = ROOT / "tests/fixtures/example.gltf"


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


class WheelAPI(unittest.TestCase):
    def test_certified_root_proxy_installed_api(self):
        sys.path.insert(0, str(ROOT / "tests"))
        try:
            import f1d1_oracle as oracle
        finally:
            sys.path.pop(0)
        payload = oracle.fixture("grid")
        source = self.root / "proxy-grid.glb"
        source.write_bytes(payload)
        output = self.root / "proxy-grid.3tz"
        result = rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=16,
            root_proxy_triangles=8, max_proxy_error_metres=10.0)
        with zipfile.ZipFile(result.output) as stream:
            members = {name: stream.read(name) for name in stream.namelist()
                       if name != "@3dtilesIndex1@"}
        inspected = oracle.inspect_members(payload, members, 16, 8, 10.0)
        self.assertEqual(result.report, json.loads(members["conversion.json"]))
        self.assertLess(result.report["approximation"]["triangles"], result.report["triangles"])
        self.assertTrue(inspected)
        for kwargs in ({"root_proxy_triangles": 8}, {"max_proxy_error_metres": 10.0},
                       {"root_proxy_triangles": 0, "max_proxy_error_metres": 10.0},
                       {"root_proxy_triangles": 8, "max_proxy_error_metres": float("nan")}):
            absent = self.root / "proxy-invalid" / "out.3tz"
            with self.assertRaises(rusty_tiles.InvalidRequestError):
                rusty_tiles.mesh_local_to_3tz(source, absent, leaf_triangles=16, **kwargs)
            self.assertFalse(absent.parent.exists())

    def test_adaptive_certificate_installed_api(self):
        sys.path.insert(0, str(ROOT / "tests"))
        try:
            import f1d1_oracle as oracle
        finally:
            sys.path.pop(0)
        # Decoded artifacts must prove tighter coverage; the report counters
        # cannot supply the geometry truth for the independent exact checker.
        for variant, limit in (("grid", 16), ("bump", 2)):
            with self.subTest(variant=variant):
                payload = oracle.fixture(variant, 4)
                source = self.root / ("adaptive-" + variant + ".glb")
                output = source.with_suffix(".3tz")
                source.write_bytes(payload)
                result = rusty_tiles.mesh_local_to_3tz(source, output,
                    leaf_triangles=16, root_proxy_triangles=limit,
                    max_proxy_error_metres=0.5)
                with zipfile.ZipFile(result.output) as stream:
                    members = {name: stream.read(name) for name in stream.namelist()
                               if name != "@3dtilesIndex1@"}
                checked = oracle.inspect_members(payload, members, 16, limit, 0.5)
                self.assertEqual(result.report, json.loads(members["conversion.json"]))
                certificate = result.report["approximation"]["certificate"]
                self.assertLessEqual(certificate["error_metres"], 0.5)
                self.assertGreater(certificate["max_depth"], 0)
                if variant == "grid":
                    self.assertGreater(Fraction(checked["historical_whole_face_squared"]), Fraction(1, 4))
                else:
                    self.assertGreater(certificate["error_metres"], 0)

    def test_source_identity_metadata_independent_installed_api(self):
        sys.path.insert(0, str(ROOT / "tests"))
        try:
            import f1c2_oracle as oracle
        finally:
            sys.path.pop(0)
        payload = oracle.fixture("instances")
        source = self.root / "identity.glb"
        source.write_bytes(payload)
        for limit in (1, 5, 100):
            output = self.root / ("identity-" + str(limit) + ".3tz")
            result = rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=limit)
            with zipfile.ZipFile(result.output) as stream:
                members = {name: stream.read(name) for name in stream.namelist()
                           if name != "@3dtilesIndex1@"}
            inspected = oracle.inspect_members(payload, members, limit)
            self.assertEqual(result.report, json.loads(members["conversion.json"]))
            self.assertEqual(inspected["triangles"], 32)

    def test_explicit_mesh_placement_independent_world_contract(self):
        sys.path.insert(0, str(ROOT / "tests"))
        try:
            import f1c1_oracle as oracle
        finally:
            sys.path.pop(0)
        # Independently cover every placement and every admitted source frame.
        scenarios = [("all-slots", name, limit, False)
                     for name in oracle.PLACEMENTS for limit in (1, 1000)]
        scenarios += [(variant, "mixed-offset", limit, variant in oracle.EXTERNAL_VARIANTS)
                      for variant in oracle.VARIANTS for limit in (1, 1000)]
        sources = {}
        for index, (variant, name, limit, external) in enumerate(scenarios):
            key = (variant, external)
            if key not in sources:
                sources[key] = oracle.write_fixture(self.root / (variant + str(external)),
                                                     variant, external)
            source = sources[key]
            output = self.root / f"{index}-{variant}-{name}-{limit}.3tz"
            placement = oracle.PLACEMENTS[name]
            with self.subTest(variant=variant, placement=name, leaf_limit=limit):
                result = rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=limit,
                                                       **{key: tuple(value) for key, value in placement.items()})
                self.assertEqual(result.report, oracle.inspect(source, output, limit, placement)["report"])
                self.assertFalse(result.cleanup_diagnostics)
        errors = {"invalid_request": rusty_tiles.InvalidRequestError,
                  "unsupported": rusty_tiles.UnsupportedError}
        for name, arguments, kind in oracle.refusal_cases():
            if name.startswith("partial-"):
                continue  # PyO3 rejects tuple arity before entering the operation.
            parameters = {}
            cursor = 0
            while cursor < len(arguments):
                flag = arguments[cursor]
                length = 4 if flag == "--orientation-xyzw" else 3
                parameters[flag[2:].replace("-", "_")] = tuple(
                    float(value) for value in arguments[cursor + 1:cursor + 1 + length])
                cursor += length + 1
            output = self.root / "absent" / (name + ".3tz")
            with self.subTest(refusal=name):
                with self.assertRaises(errors[kind]) as caught:
                    rusty_tiles.mesh_local_to_3tz(self.root / "missing.glb", output,
                                                  leaf_triangles=1, **parameters)
                self.assertEqual(caught.exception.kind, kind)
                self.assertFalse(output.parent.exists())
        for parameters in ({"anchor": (0, 0)}, {"anchor": (0, 0, 0), "orientation_xyzw": (0, 0, 1)},
                           {"anchor": (0, 0, 0), "scene_offset": (0, 0)}):
            with self.assertRaises((TypeError, ValueError)):
                rusty_tiles.mesh_local_to_3tz(self.root / "missing.glb", self.root / "absent/out.3tz",
                                              leaf_triangles=1, **parameters)
            self.assertFalse((self.root / "absent").exists())

    def test_local_resource_mesh_independent_profile_oracle(self):
        sys.path.insert(0, str(ROOT / "tests"))
        try:
            import f1b2_oracle as oracle
        finally:
            sys.path.pop(0)
        for variant in oracle.VARIANTS:
            directory = self.root / variant
            directory.mkdir()
            source = oracle.write_fixture(directory, variant)
            for limit in (1, 1000):
                with self.subTest(variant=variant, leaf_triangles=limit):
                    output = directory / (str(limit) + ".3tz")
                    result = rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=limit)
                    self.assertEqual(result.report, oracle.inspect(source, output, limit)["report"])
        errors = {"invalid_input": rusty_tiles.DataError, "unsupported": rusty_tiles.UnsupportedError,
                  "io": rusty_tiles.TilesIOError, "invalid_request": rusty_tiles.InvalidRequestError}
        for name, bundle, kind in oracle.refusal_bundles():
            directory = self.root / ("refusal-" + name)
            directory.mkdir()
            source = oracle.write_bundle(directory, bundle)
            output = directory / "absent" / "out.3tz"
            with self.subTest(refusal=name):
                with self.assertRaises(errors[kind]) as caught:
                    rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=1)
                self.assertEqual(caught.exception.kind, kind)
                self.assertFalse(output.parent.exists())

    def test_local_external_mesh_snapshot_and_typed_overlap(self):
        embedded = self.root / "embedded.glb"
        write_local_mesh(embedded)
        raw = embedded.read_bytes()
        json_length = struct.unpack_from("<I", raw, 12)[0]
        document = json.loads(raw[20:20 + json_length])
        resource = self.root / "geometry.bin"
        resource.write_bytes(raw[28 + json_length:])
        document["buffers"][0]["uri"] = "geometry.bin"
        source = self.root / "external.gltf"
        original = json.dumps(document).encode() + b"\n\t"
        source.write_bytes(original)
        alias = self.root / "alias.3tz"
        os.link(resource, alias)
        events = []
        with self.assertRaises(rusty_tiles.InvalidRequestError) as caught:
            rusty_tiles.mesh_local_to_3tz(source, alias, leaf_triangles=1,
                                         force=True, callback=events.append)
        self.assertEqual(caught.exception.kind, "invalid_request")
        self.assertEqual(events, [])
        self.assertEqual(alias.read_bytes(), resource.read_bytes())

        def observe(event):
            if not events:
                source.unlink()
                resource.unlink()
            events.append(event)

        output = self.root / "external.3tz"
        result = rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=1,
                                              callback=observe)
        self.assertEqual(result.report["schema_version"], 7)
        self.assertEqual(result.report["profile"], "f1d2-adaptive-root-proxy-gltf-v1")
        self.assertEqual(result.report["source_bytes"], len(original))
        self.assertEqual(result.report["external_files"], 1)
        self.assertEqual(result.report["external_bytes"], 108)
        self.assertEqual(result.report["triangles"], 3)
        self.assertEqual(result.report["leaf_tiles"], 3)
        self.assertFalse(source.exists())
        self.assertFalse(resource.exists())
        with zipfile.ZipFile(output) as archive:
            self.assertEqual(json.loads(archive.read("conversion.json")), result.report)

    def test_local_textured_mesh_independent_profile_oracle(self):
        # Loading an independently authored fixture/reader is allowed here;
        # rusty_tiles itself has already been imported from the installed wheel.
        sys.path.insert(0, str(ROOT / "tests"))
        try:
            import f1b_oracle as oracle
        finally:
            sys.path.pop(0)
        for variant in oracle.VARIANTS:
            source = self.root / (variant + ".glb")
            source.write_bytes(oracle.fixture(variant=variant))
            before = hashlib.sha256(source.read_bytes()).hexdigest()
            for limit in (1, 3, 1000):
                with self.subTest(variant=variant, leaf_triangles=limit):
                    output = self.root / (variant + "-" + str(limit) + ".3tz")
                    value = rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=limit)
                    inspected = oracle.inspect(source, output, limit)
                    self.assertEqual(value.report, inspected["report"])
                    self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), before)
        for name, data, kind in oracle.rejection_fixtures():
            source = self.root / (name + ".glb")
            source.write_bytes(data)
            expected = rusty_tiles.UnsupportedError if kind == "unsupported" else rusty_tiles.DataError
            for limit in (1, 1000):
                with self.subTest(refusal=name, leaf_triangles=limit):
                    output = self.root / (name + "-" + str(limit)) / "out.3tz"
                    with self.assertRaises(expected) as caught:
                        rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=limit)
                    self.assertEqual(caught.exception.kind, kind)
                    self.assertFalse(output.parent.exists())
                    self.assertEqual(source.read_bytes(), data)

    def test_core_pbr_mesh_independent_corner_and_resource_oracle(self):
        sys.path.insert(0, str(ROOT / "tests"))
        try:
            import f1b3_oracle as oracle
        finally:
            sys.path.pop(0)
        for external, variants, limits in (
            (False, oracle.VARIANTS, (1, 3, 1000)),
            (True, oracle.EXTERNAL_VARIANTS, (1, 1000)),
        ):
            for variant in variants:
                name = ("external-" if external else "embedded-") + variant
                source = oracle.write_fixture(self.root / name, variant, external=external)
                before = {p.relative_to(source.parent): p.read_bytes()
                          for p in source.parent.rglob("*") if p.is_file()}
                for limit in limits:
                    with self.subTest(variant=variant, external=external, leaf_triangles=limit):
                        output = self.root / (name + "-" + str(limit) + ".3tz")
                        value = rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=limit)
                        self.assertEqual(value.report, oracle.inspect(source, output, limit)["report"])
                        self.assertEqual(value.report["profile"], "f1d2-adaptive-root-proxy-gltf-v1")
                self.assertEqual(before, {p.relative_to(source.parent): p.read_bytes()
                                         for p in source.parent.rglob("*") if p.is_file()})
        for name, bundle, kind in oracle.refusal_bundles():
            source = oracle.binding.write_bundle(self.root / "refusal-sources" / name, bundle)
            before = {p.relative_to(source.parent): p.read_bytes()
                      for p in source.parent.rglob("*") if p.is_file()}
            for limit in (1, 1000):
                with self.subTest(refusal=name, leaf_triangles=limit):
                    output = self.root / "absent-output" / name / (str(limit) + ".3tz")
                    expected = {"unsupported": rusty_tiles.UnsupportedError,
                                "invalid_input": rusty_tiles.DataError,
                                "io": rusty_tiles.TilesIOError}[kind]
                    with self.assertRaises(expected) as caught:
                        rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=limit)
                    self.assertEqual(caught.exception.kind, kind)
                    self.assertFalse(output.parent.exists())
                    self.assertEqual(before, {p.relative_to(source.parent): p.read_bytes()
                                             for p in source.parent.rglob("*") if p.is_file()})

    def test_local_textured_mesh_forwards_one_prepared_image(self):
        source = self.root / "textured.glb"
        original = (ROOT / "tests/fixtures/f1b/basecolor.glb").read_bytes()
        expected_image = (ROOT / "tests/fixtures/f1b/source-rgba.png").read_bytes()
        source.write_bytes(original)
        output = self.root / "textured.3tz"
        events = []

        def observe(event):
            self.assertFalse(output.exists())
            events.append(event)
            if event.get("phase") == "mesh_leaves" and event.get("done") == 0:
                source.write_bytes(b"changed after preparation")

        result = rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=1, callback=observe)
        self.assertEqual(result.report["schema_version"], 7)
        self.assertEqual(result.report["profile"], "f1d2-adaptive-root-proxy-gltf-v1")
        self.assertEqual(result.report["source_bytes"], len(original))
        self.assertEqual(result.report["triangles"], 8)
        self.assertEqual(result.report["leaf_tiles"], 8)
        self.assertEqual(result.report["images"], 1)
        self.assertEqual(result.report["image_bytes"], len(expected_image))
        self.assertEqual(result.report["image_pixels"], 6)
        self.assertTrue(events)
        with zipfile.ZipFile(output) as archive:
            self.assertEqual(archive.read("textures/0.png"), expected_image)
            self.assertNotIn("textures/1.png", archive.namelist())
            self.assertEqual(len(archive.namelist()), 12)
            self.assertEqual(json.loads(archive.read("conversion.json")), result.report)

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

    def test_local_mesh_explicit_limit_report_and_precommit_events(self):
        source = self.root / "local.glb"
        write_local_mesh(source)
        output = self.root / "local.3tz"
        events = []

        def observe(event):
            self.assertFalse(output.exists(), "mesh callback ran after installation")
            events.append(event)

        result = rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=1,
                                               callback=observe)
        self.assertIsInstance(result, rusty_tiles.MeshResult)
        self.assertTrue(result.output.is_absolute())
        self.assertTrue(result.output.samefile(output))
        self.assertEqual(result.report["triangles"], 3)
        self.assertEqual(result.report["leaf_tiles"], 3)
        self.assertEqual(result.report["leaf_triangles"], 1)
        self.assertEqual(result.report["coordinates"], "local-gltf")
        self.assertEqual(result.report["profile"], "f1d2-adaptive-root-proxy-gltf-v1")
        self.assertEqual(result.cleanup_diagnostics, [])
        self.assertTrue(events)
        self.assertTrue(any(event.get("phase") == "ready_to_publish" for event in events))
        with zipfile.ZipFile(output) as archive:
            self.assertEqual(json.loads(archive.read("conversion.json")), result.report)
            manifest = json.loads(archive.read("tileset.json"))
            children = manifest["root"]["children"]
            self.assertEqual(len(children), 3)
            content_names = [child["content"]["uri"] for child in children]
            self.assertEqual(set(archive.namelist()),
                             {"tileset.json", "conversion.json", "@3dtilesIndex1@", *content_names})

    def test_local_mesh_validation_and_unsupported_source_create_no_work(self):
        output = self.root / "absent" / "local.3tz"
        with self.assertRaises(rusty_tiles.InvalidRequestError) as caught:
            rusty_tiles.mesh_local_to_3tz(self.root / "missing.glb", output, leaf_triangles=0)
        self.assertEqual(caught.exception.kind, "invalid_request")
        self.assertFalse(output.parent.exists())
        source = self.root / "unsupported.glb"
        write_local_mesh(source, extras=True)
        with self.assertRaises(rusty_tiles.UnsupportedError) as caught:
            rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=1)
        self.assertEqual(caught.exception.kind, "unsupported")
        self.assertFalse(output.parent.exists())
        with self.assertRaises(TypeError):
            rusty_tiles.mesh_local_to_3tz(source, output)
        with self.assertRaises(TypeError):
            rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=1, callback=42)

    def test_local_mesh_callbacks_abort_preserve_original_and_destination(self):
        source = self.root / "callback.glb"
        write_local_mesh(source)
        for final in (False, True):
            for force in (False, True):
                with self.subTest(final=final, force=force):
                    output = self.root / f"callback-{final}-{force}.3tz"
                    if force:
                        output.write_bytes(b"previous destination")
                    original = RuntimeError("selected mesh observer failure")

                    def observe(event):
                        if not final or event.get("phase") == "ready_to_publish":
                            raise original

                    with self.assertRaises(RuntimeError) as caught:
                        rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=1,
                                                     force=force, callback=observe)
                    self.assertIs(caught.exception, original)
                    if force:
                        self.assertEqual(output.read_bytes(), b"previous destination")
                    else:
                        self.assertFalse(output.exists())
        self.assertFalse(any(path.name.startswith((".tiles-", ".mesh-work-"))
                             for path in self.root.iterdir()))

    def test_local_mesh_callback_cwd_change_cannot_redirect_relative_output(self):
        source = self.root / "cwd.glb"
        write_local_mesh(source)
        initial = self.root / "initial"
        redirected = self.root / "redirected"
        initial.mkdir()
        redirected.mkdir()
        source, initial, redirected = source.resolve(), initial.resolve(), redirected.resolve()
        original_cwd = Path.cwd()

        def observe(event):
            os.chdir(redirected)

        # The unittest harness runs cases sequentially. Restore process-global
        # CWD even when conversion/assertions fail, without launching a second
        # interpreter that would lose the installed-wheel import environment.
        try:
            os.chdir(initial)
            result = rusty_tiles.mesh_local_to_3tz(source, "bound.3tz", leaf_triangles=1,
                                                  callback=observe)
            self.assertTrue(result.output.is_absolute())
            self.assertTrue(result.output.samefile(initial / "bound.3tz"))
            self.assertTrue((initial / "bound.3tz").exists())
            self.assertFalse((redirected / "bound.3tz").exists())
        finally:
            os.chdir(original_cwd)

    def test_local_mesh_nested_and_concurrent_jobs_have_independent_results(self):
        source = self.root / "independent.glb"
        write_local_mesh(source)
        nested = []

        def observe(event):
            if not nested:
                nested.append(rusty_tiles.mesh_local_to_3tz(
                    source, self.root / "nested-local.3tz", leaf_triangles=3,
                    anchor=(73, 90, 42), orientation_xyzw=(1, 0, 0, 0)))

        outer = rusty_tiles.mesh_local_to_3tz(source, self.root / "outer-local.3tz",
                                              leaf_triangles=1, anchor=(153, -27, 42), callback=observe)
        self.assertNotEqual(outer.report["root_transform"], nested[0].report["root_transform"])
        self.assertEqual(outer.report["leaf_tiles"], 3)
        self.assertEqual(nested[0].report["leaf_tiles"], 1)
        with ThreadPoolExecutor(max_workers=3) as executor:
            results = list(executor.map(lambda i: rusty_tiles.mesh_local_to_3tz(
                source, self.root / f"concurrent-local-{i}.3tz", leaf_triangles=1,
                anchor=(i * 37, i * 17, i * 19)), range(3)))
        self.assertEqual(len({result.output for result in results}), 3)
        self.assertEqual(len({tuple(result.report["root_transform"]) for result in results}), 3)

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
            source, output, layers=["survey"], height_offset=7, explicit=True,
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
        shutil.copy(EXAMPLE, self.root / "example.gltf")
        previous = Path.cwd()
        try:
            os.chdir(self.root)
            exec(example, {})
        finally:
            os.chdir(previous)
        with zipfile.ZipFile(self.root / "example.3tz") as archive:
            self.assertIn("tileset.json", archive.namelist())

    def test_mesh_events_and_report(self):
        events = []
        result = rusty_tiles.mesh_to_3tz(
            EXAMPLE, self.root / "mesh.3tz", max_triangles=2,
            tile_size=64, texture_format="jpeg", callback=events.append,
        )
        self.assertIsInstance(result, rusty_tiles.ConversionResult)
        self.assertTrue(result.archive)
        self.assertIsInstance(result.output, Path)
        self.assertIsInstance(result.report, dict)
        self.assertTrue(any(event["event"] == "log" for event in events))
        self.assertTrue(rusty_tiles.validate(result.output)["ok"])

    def test_general_mesh_crs_requires_axes_and_height_and_places_output(self):
        output = self.root / "projected-mesh.3tz"
        result = rusty_tiles.mesh_to_3tz(
            EXAMPLE, output, source_crs="EPSG:32632", source_axes="xyz",
            height_offset=0, source_offset=(500000.123456789, 0, 100),
            tile_size=64, max_triangles=2,
        )
        self.assertTrue(rusty_tiles.validate(result.output)["ok"])
        with zipfile.ZipFile(output) as archive:
            root = json.loads(archive.read("tileset.json"))["root"]
            self.assertEqual(len(root["transform"]), 16)
            self.assertGreater(abs(root["transform"][12]), 6000000)
        for kwargs in [{"height_offset": 0}, {"source_axes": "xyz"}]:
            with self.assertRaises(rusty_tiles.DataError):
                rusty_tiles.mesh_to_3tz(
                    EXAMPLE, output, force=True, source_crs="EPSG:32632", **kwargs,
                )
            self.assertTrue(rusty_tiles.validate(output)["ok"])
        with self.assertRaises(ValueError):
            rusty_tiles.mesh_to_3tz(
                EXAMPLE, self.root / "bad-axes.3tz", source_crs="EPSG:32632",
                source_axes="guess", height_offset=0,
            )

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

    def test_convert_to_implicit_preserves_payload_and_force_semantics(self):
        source = self.root / "source.las"
        write_las(source)
        explicit = rusty_tiles.point_cloud_to_3tz(source, self.root / "explicit.3tz", explicit=True, source_crs="local")
        converted = rusty_tiles.convert_to_implicit(explicit.output, self.root / "implicit.3tz")
        self.assertTrue(rusty_tiles.validate(converted.output)["ok"])
        self.assertTrue(converted.report["contentBytesPreserved"])
        with zipfile.ZipFile(explicit.output) as before, zipfile.ZipFile(converted.output) as after:
            for name in before.namelist():
                if name.endswith(".glb"):
                    self.assertEqual(before.read(name), after.read(name))
        with self.assertRaises(rusty_tiles.OutputExistsError):
            rusty_tiles.convert_to_implicit(explicit.output, converted.output)
        rusty_tiles.convert_to_implicit(explicit.output, converted.output, force=True)

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
            explicit=True,
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

    def test_mesh_node_features_are_available_from_python(self):
        source = self.root / "buildings.gltf"
        document = json.loads(EXAMPLE.read_text())
        document["nodes"] = [
            {"mesh": 0, "name": "Building A", "translation": [-2, 0, 0]},
            {"mesh": 0, "name": "Building B", "translation": [2, 0, 0]},
        ]
        document["scenes"][document.get("scene", 0)]["nodes"] = [0, 1]
        source.write_text(json.dumps(document))
        for explicit in (False, True):
            output = self.root / f"buildings-{explicit}.3tz"
            rusty_tiles.mesh_to_3tz(source, output, node_features=True, explicit=explicit)
            self.assertTrue(rusty_tiles.validate(output)["ok"])
            names = set()
            with zipfile.ZipFile(output) as archive:
                for entry in archive.namelist():
                    if not entry.endswith(".glb"):
                        continue
                    payload = archive.read(entry)
                    length = struct.unpack_from("<I", payload, 12)[0]
                    gltf = json.loads(payload[20:20 + length])
                    binary = payload[28 + length:]
                    metadata = gltf["extensions"]["EXT_structural_metadata"]
                    table = metadata["propertyTables"][0]
                    prop = table["properties"]["name"]
                    def view(index):
                        description = gltf["bufferViews"][index]
                        start = description.get("byteOffset", 0)
                        return binary[start:start + description["byteLength"]]
                    strings = view(prop["values"])
                    offsets = [row[0] for row in struct.iter_unpack("<I", view(prop["stringOffsets"]))]
                    names.update(strings[start:end].decode("utf-8") for start, end in zip(offsets, offsets[1:]))
                    for primitive in gltf["meshes"][0]["primitives"]:
                        self.assertIn("_FEATURE_ID_0", primitive["attributes"])
                        feature = primitive["extensions"]["EXT_mesh_features"]["featureIds"][0]
                        self.assertEqual(feature["propertyTable"], 0)
                        self.assertEqual(feature["featureCount"], table["count"])
            self.assertEqual(names, {"Building A", "Building B"})

    def test_point_metadata_attributes_are_available_from_python(self):
        source = self.root / "points.las"
        write_las(source)
        for explicit in (False, True):
            output = self.root / f"attributes-{explicit}.3tz"
            rusty_tiles.point_cloud_to_3tz(source, output, max_points=2,
                                         metadata_attributes=True, explicit=explicit, source_crs="local")
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
        with self.assertRaises(ValueError):
            rusty_tiles.mesh_to_3tz(EXAMPLE, output, texture_format="uastc")
        with self.assertRaises(ValueError):
            rusty_tiles.mesh_to_3tz(EXAMPLE, output, cartographic=(0, 100, 0))
        with self.assertRaises(TypeError):
            rusty_tiles.mesh_to_3tz(EXAMPLE, output, callback=42)
        self.assertFalse(output.exists())

    def test_callback_exception_is_preserved(self):
        error = RuntimeError("callback failed")
        calls = []

        def fail(event):
            calls.append(event)
            raise error

        with self.assertRaises(RuntimeError) as caught:
            rusty_tiles.mesh_to_3tz(
                EXAMPLE, self.root / "callback.3tz", max_triangles=2,
                tile_size=64, texture_format="jpeg", callback=fail,
            )
        self.assertIs(caught.exception, error)
        self.assertEqual(len(calls), 1)

    def test_concurrent_calls_and_callback_reentry(self):
        def convert(index):
            def callback(event):
                self.assertTrue(rusty_tiles.validate(self.root / "base.3tz")["ok"])

            return rusty_tiles.mesh_to_3tz(
                EXAMPLE, self.root / f"thread-{index}.3tz", max_triangles=2,
                tile_size=64, texture_format="jpeg", callback=callback,
            )

        source = self.root / "model.glb"
        write_local_mesh(source)
        rusty_tiles.glb_to_3tz(source, self.root / "base.3tz")
        with ThreadPoolExecutor(max_workers=2) as pool:
            results = list(pool.map(convert, range(2)))
        self.assertEqual(len(results), 2)
        for result in results:
            self.assertTrue(rusty_tiles.validate(result.output)["ok"])


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
