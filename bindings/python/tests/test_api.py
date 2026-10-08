"""Exercise the installed native extension using only the Python standard library."""
from concurrent.futures import ThreadPoolExecutor
import importlib.metadata
import json
import os
from pathlib import Path
import shutil
import struct
import sys
import tempfile
import unittest
import zipfile

import rusty_tiles

ROOT = Path(sys.argv.pop(1))
EXAMPLE = ROOT / "tests/fixtures/example.gltf"


def write_las(path):
    """Invented LAS 1.2, format 0, with four local XYZ points."""
    points = [(0, 0, 0), (100, 0, 0), (0, 100, 0), (100, 100, 100)]
    header = bytearray(227)
    header[:4] = b"LASF"
    header[24:26] = bytes((1, 2))
    struct.pack_into("<HII", header, 94, 227, 227, 0)
    struct.pack_into("<BHI", header, 104, 0, 20, len(points))
    struct.pack_into("<I", header, 111, len(points))
    struct.pack_into("<3d", header, 131, 0.01, 0.01, 0.01)
    struct.pack_into("<6d", header, 179, 1, 0, 1, 0, 1, 0)
    with path.open("wb") as stream:
        stream.write(header)
        for x, y, z in points:
            stream.write(struct.pack("<iiiHBBbBH", x, y, z, 7, 9, 2, 0, 0, 0))


class WheelAPI(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.root = Path(self.work.name)

    def test_readme_example_in_clean_environment(self):
        self.assertEqual(os.environ["PATH"], "")
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
    unittest.main(verbosity=2)
