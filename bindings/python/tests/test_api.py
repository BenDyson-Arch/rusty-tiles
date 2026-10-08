"""Exercise the installed native extension using only the Python standard library."""
from concurrent.futures import ThreadPoolExecutor
import importlib.metadata
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


class WheelAPI(unittest.TestCase):
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

    def test_vector_geopackage_readonly_height_and_layer_selection(self):
        source = self.root / "survey.gpkg"
        with sqlite3.connect(source) as db:
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
        repacked = rusty_tiles.convert_to_3tz(extracted / "tileset.json", self.root / "repacked.3tz")
        self.assertTrue(rusty_tiles.validate(repacked.output)["ok"])
        self.assertIsNone(repacked.report)

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
        with self.assertRaises(rusty_tiles.DataError):
            rusty_tiles.convert_to_3tz(self.root, output)
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
        evidence["blender_build_hash"] = bpy.app.build_hash.decode("ascii")
    if report := os.environ.get("RUSTY_TILES_ACCEPTANCE_REPORT"):
        destination = Path(report)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(json.dumps(evidence, indent=2) + "\n")
    raise SystemExit(0 if result.wasSuccessful() else 1)
