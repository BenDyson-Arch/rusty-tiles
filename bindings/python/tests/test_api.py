"""Exercise the installed native extension using only the Python standard library."""
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
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
import subprocess
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
        self.assertEqual(result.report["profile"], "f1a-local-static-glb-v1")
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
        script = """
import os, sys
from pathlib import Path
import rusty_tiles
source, initial, redirected = map(Path, sys.argv[1:])
os.chdir(initial)
def observe(event):
    os.chdir(redirected)
result = rusty_tiles.mesh_local_to_3tz(source, "bound.3tz", leaf_triangles=1, callback=observe)
assert result.output.is_absolute()
assert result.output.samefile(initial / "bound.3tz")
assert (initial / "bound.3tz").exists()
assert not (redirected / "bound.3tz").exists()
"""
        completed = subprocess.run([sys.executable, "-c", script, str(source.resolve()),
                                    str(initial.resolve()), str(redirected.resolve())],
                                   capture_output=True, text=True)
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)

    def test_local_mesh_nested_and_concurrent_jobs_have_independent_results(self):
        source = self.root / "independent.glb"
        write_local_mesh(source)
        nested = []

        def observe(event):
            if not nested:
                nested.append(rusty_tiles.mesh_local_to_3tz(
                    source, self.root / "nested-local.3tz", leaf_triangles=3))

        outer = rusty_tiles.mesh_local_to_3tz(source, self.root / "outer-local.3tz",
                                              leaf_triangles=1, callback=observe)
        self.assertEqual(outer.report["leaf_tiles"], 3)
        self.assertEqual(nested[0].report["leaf_tiles"], 1)
        with ThreadPoolExecutor(max_workers=3) as executor:
            results = list(executor.map(lambda i: rusty_tiles.mesh_local_to_3tz(
                source, self.root / f"concurrent-local-{i}.3tz", leaf_triangles=1), range(3)))
        self.assertEqual(len({result.output for result in results}), 3)
        self.assertTrue(all(result.report == results[0].report for result in results))

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
        with self.assertRaisesRegex(rusty_tiles.EnvironmentError, "native-geospatial"):
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
        result = rusty_tiles.glb_to_3tz(
            str(EXAMPLE), self.root / "wrapped.3tz", cartographic=(153.02, -27.47, 0),
            rotation=(5, 0, 0),
        )
        self.assertIsNone(result.report)
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
        explicit = rusty_tiles.point_cloud_to_3tz(source, self.root / "explicit.3tz", explicit=True)
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
        )
        self.assertEqual(result.report["points"], 4)
        self.assertTrue(rusty_tiles.validate(output)["ok"])
        phases = {e["phase"] for e in events if e["event"] == "progress"}
        self.assertTrue({"ingestion", "tiling"}.issubset(phases))
        before = output.read_bytes()
        with self.assertRaises(rusty_tiles.OutputExistsError):
            rusty_tiles.point_cloud_to_3tz(source, output)
        self.assertEqual(output.read_bytes(), before)
        rusty_tiles.point_cloud_to_3tz(source, output, force=True)
        self.assertTrue(rusty_tiles.validate(output)["ok"])

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
        with self.assertRaisesRegex(rusty_tiles.EnvironmentError, "native-geospatial"):
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
                with self.assertRaisesRegex(rusty_tiles.DataError, "heightOffset"):
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
                                         metadata_attributes=True, explicit=explicit)
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
            rusty_tiles.point_cloud_to_3tz(self.root / "missing.las", output)
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

        rusty_tiles.glb_to_3tz(EXAMPLE, self.root / "base.3tz")
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
