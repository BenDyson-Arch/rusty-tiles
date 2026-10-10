#!/usr/bin/env python3
"""Bounded old-converter observations; never production validation as the oracle.

Usage: python3 replay_baseline.py FROZEN_CLI EMPTY_OUTPUT_DIRECTORY
The CLI must match the recorded baseline hash. No Cargo/build or source mutation.
Only the authored 8-point, depth-one QUADTREE fixture is an acceptance target here.
"""
import copy
import hashlib
import json
import pathlib
import struct
import subprocess
import sys
import zipfile

PIN = "6f26363b4f9fd5270a5faf186fbaab8b3b139ebf13c9c788bb52d368eb1ea4f1"
IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def members(path):
    with zipfile.ZipFile(path) as archive:
        return {name: archive.read(name) for name in archive.namelist()
                if name != "@3dtilesIndex1@"}


def parse_tree(data):
    assert data[:4] == b"subt" and struct.unpack_from("<I", data, 4)[0] == 1
    js, binary = struct.unpack_from("<QQ", data, 8)
    assert js % 8 == binary % 8 == 0 and len(data) == 24 + js + binary
    doc = json.loads(data[24:24 + js])
    buf = data[24 + js:]

    def view(index):
        value = doc["bufferViews"][index]
        offset, length = value.get("byteOffset", 0), value["byteLength"]
        assert value["buffer"] == 0 and offset % 8 == 0 and offset + length <= len(buf)
        return buf[offset:offset + length]

    def availability(value, count):
        if "constant" in value:
            assert value["constant"] in (0, 1)
            return [bool(value["constant"])] * count
        raw = view(value["bitstream"])
        assert len(raw) == (count + 7) // 8
        bits = [bool(raw[i // 8] & (1 << (i % 8))) for i in range(count)]
        assert not any(raw[i // 8] & (1 << (i % 8)) for i in range(count, len(raw) * 8))
        if "availableCount" in value:
            assert value["availableCount"] == sum(bits)
        return bits

    return doc, view, availability


def mul(a, b):
    return [sum(a[k * 4 + i % 4] * b[i // 4 * 4 + k] for k in range(4))
            for i in range(16)]


def positions(data, matrix):
    js = struct.unpack_from("<I", data, 12)[0]
    doc = json.loads(data[20:20 + js])
    binary = data[28 + js:]
    assert len(doc["nodes"]) == 1 and doc["nodes"][0] == {"mesh": 0}
    result = []
    for primitive in doc["meshes"][0]["primitives"]:
        assert primitive["mode"] == 0
        accessor = doc["accessors"][primitive["attributes"]["POSITION"]]
        assert accessor["componentType"] == 5126 and accessor["type"] == "VEC3"
        view = doc["bufferViews"][accessor["bufferView"]]
        start = accessor.get("byteOffset", 0) + view.get("byteOffset", 0)
        for index in range(accessor["count"]):
            x, y, z = struct.unpack_from("<3f", binary, start + index * view.get("byteStride", 12))
            local = [x, -z, y]
            result.append([matrix[12 + i] + sum(matrix[k * 4 + i] * local[k] for k in range(3))
                           for i in range(3)])
    return result


def oracle(source, output, authored):
    for name, data in source.items():
        if name not in ("tileset.json", "conversion.json"):
            assert output[name] == data
    src = json.loads(source["tileset.json"])
    dst = json.loads(output["tileset.json"])
    root = src["root"]
    proxy = dst["root"]
    for key, value in src.items():
        if key not in ("root", "schema"):
            assert dst[key] == value
    for key, value in root.items():
        if key not in ("children", "content", "contents"):
            assert proxy[key] == value
    tiling = proxy["implicitTiling"]
    assert tiling["subdivisionScheme"] == "QUADTREE" and tiling["subtreeLevels"] == 2
    subtree_name = tiling["subtrees"]["uri"]
    for variable in ("level", "x", "y"):
        subtree_name = subtree_name.replace("{" + variable + "}", "0")
    doc, view, bits = parse_tree(output[subtree_name])
    available = bits(doc["tileAvailability"], 5)
    slots = sorted((int(child["transform"][12] >= 0) + 2 * int(child["transform"][13] >= 0), child)
                   for child in root["children"])
    expected_indices = [0] + [1 + slot for slot, _ in slots]
    assert [i for i, yes in enumerate(available) if yes] == expected_indices
    assert bits(doc["contentAvailability"][0], 5) == [i in expected_indices[1:] for i in range(5)]
    assert not any(bits(doc["childSubtreeAvailability"], 16))
    table = doc["propertyTables"][doc["tileMetadata"]]
    assert table["count"] == 3
    props = table["properties"]
    boxes = list(struct.iter_unpack("<12d", view(props["boundingBox"]["values"])))
    errors = [x[0] for x in struct.iter_unpack("<d", view(props["geometricError"]["values"]))]
    expected_boxes = [root["boundingVolume"]["box"]]
    for _, child in slots:
        box = child["boundingVolume"]["box"].copy()
        for axis in range(3):
            box[axis] += child["transform"][12 + axis]
        expected_boxes.append(box)
    assert boxes == [tuple(x) for x in expected_boxes]
    assert errors == [root["geometricError"]] + [child["geometricError"] for _, child in slots]
    actual = []
    template = proxy["content"]["uri"]
    for slot, original in slots:
        name = template.replace("{level}", "1").replace("{x}", str(slot & 1)).replace("{y}", str(slot >> 1))
        owned = json.loads(output[name])["root"]
        for key, value in original.items():
            if key not in ("content", "children"):
                assert owned[key] == value
        for key, value in original["content"].items():
            if key != "uri":
                assert owned["content"][key] == value
        leaf_tiling = owned["implicitTiling"]
        assert leaf_tiling["subdivisionScheme"] == "QUADTREE"
        assert leaf_tiling["availableLevels"] == leaf_tiling["subtreeLevels"] == 1
        leaf_uri = leaf_tiling["subtrees"]["uri"]
        for variable in ("level", "x", "y"):
            leaf_uri = leaf_uri.replace("{" + variable + "}", "0")
        leaf_doc, leaf_view, leaf_bits = parse_tree(output[leaf_uri])
        assert leaf_bits(leaf_doc["tileAvailability"], 1) == [True]
        assert leaf_bits(leaf_doc["contentAvailability"][0], 1) == [True]
        assert leaf_bits(leaf_doc["childSubtreeAvailability"], 4) == [False] * 4
        leaf_table = leaf_doc["propertyTables"][leaf_doc["tileMetadata"]]
        assert leaf_table["count"] == 1
        leaf_props = leaf_table["properties"]
        assert struct.unpack("<12d", leaf_view(leaf_props["boundingBox"]["values"])) == tuple(original["boundingVolume"]["box"])
        assert struct.unpack("<d", leaf_view(leaf_props["geometricError"]["values"]))[0] == original["geometricError"]
        alias = owned["content"]["uri"]
        for variable in ("level", "x", "y"):
            alias = alias.replace("{" + variable + "}", "0")
        assert output[alias] == source[original["content"]["uri"]]
        actual += positions(output[alias], mul(proxy.get("transform", IDENTITY), owned.get("transform", IDENTITY)))
    expected = sorted(feature["geometry"]["coordinates"] for feature in authored["features"])
    actual.sort()
    assert len(actual) == len(expected) == 8
    worst = max(abs(a - b) for x, y in zip(actual, expected) for a, b in zip(x, y))
    assert worst < 2e-8
    return {"availableIndices": expected_indices, "points": len(actual), "maxSourcePositionError": worst}


def encode_glb(doc, binary):
    js = json.dumps(doc, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    return (struct.pack("<4sII", b"glTF", 2, 28 + len(js) + len(binary))
            + struct.pack("<I4s", len(js), b"JSON") + js
            + struct.pack("<I4s", len(binary), b"BIN\0") + binary)


def main():
    cli, work = pathlib.Path(sys.argv[1]).resolve(), pathlib.Path(sys.argv[2]).resolve()
    assert sha(cli.read_bytes()) == PIN
    work.mkdir(parents=True, exist_ok=False)
    fixture = pathlib.Path(__file__).with_name("points.geojson")
    authored = json.loads(fixture.read_bytes())
    commands = []

    def run(args):
        result = subprocess.run([str(cli), "--json"] + args, capture_output=True, text=True)
        commands.append({"args": args, "exit": result.returncode, "stdout": result.stdout, "stderr": result.stderr})
        return result

    def package_files(files, label):
        directory = work / label
        directory.mkdir()
        for name, data in files.items():
            path = directory / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        result = run(["convert", "-i", str(directory), "-o", str(work / (label + ".3tz"))])
        assert result.returncode == 0
        return work / (label + ".3tz")

    explicit, implicit = work / "explicit.3tz", work / "implicit.3tz"
    assert run(["vector", "-i", str(fixture), "-o", str(explicit), "--source-crs", "local", "--explicit",
                "--max-features", "4", "--max-parent-features", "4", "--lod-levels", "1", "--reproducible"]).returncode == 0
    source_hash = sha(explicit.read_bytes())
    assert run(["convert-to-implicit", "-i", str(explicit), "-o", str(implicit)]).returncode == 0
    source, output = members(explicit), members(implicit)
    positive = oracle(source, output, authored)
    sensitivity = {}
    rootdoc = json.loads(output["tileset.json"])
    subtree_name = rootdoc["root"]["implicitTiling"]["subtrees"]["uri"]
    for variable in ("level", "x", "y"):
        subtree_name = subtree_name.replace("{" + variable + "}", "0")
    h, _, _ = parse_tree(output[subtree_name])
    json_length = struct.unpack_from("<Q", output[subtree_name], 8)[0]
    binary_start = 24 + json_length
    for label in ("availability_clear", "availability_swap", "box_wrong", "error_wrong", "external_transform_wrong", "payload_flip"):
        changed = copy.deepcopy(output)
        if label.startswith("availability"):
            view = h["bufferViews"][h["tileAvailability"]["bitstream"]]
            raw = bytearray(changed[subtree_name])
            offset = binary_start + view["byteOffset"]
            raw[offset] ^= 1 << 1
            if label.endswith("swap"):
                raw[offset] ^= 1 << 2
            changed[subtree_name] = bytes(raw)
        elif label in ("box_wrong", "error_wrong"):
            prop = h["propertyTables"][0]["properties"]["boundingBox" if label == "box_wrong" else "geometricError"]
            view = h["bufferViews"][prop["values"]]
            raw = bytearray(changed[subtree_name]); offset = binary_start + view["byteOffset"]
            struct.pack_into("<d", raw, offset, struct.unpack_from("<d", raw, offset)[0] + 1)
            changed[subtree_name] = bytes(raw)
        else:
            name = next(name for name in changed if name.startswith("implicit-owned-"))
            doc = json.loads(changed[name])
            if label == "external_transform_wrong":
                doc["root"]["transform"][12] += 1; changed[name] = json.dumps(doc).encode()
            else:
                alias = doc["root"]["content"]["uri"]
                for variable in ("level", "x", "y"):
                    alias = alias.replace("{" + variable + "}", "0")
                raw = bytearray(changed[alias]); raw[-1] ^= 1; changed[alias] = bytes(raw)
        try:
            oracle(source, changed, authored)
        except AssertionError:
            sensitivity[label] = "rejected"
        else:
            raise AssertionError("insensitive control: " + label)
    version_source = copy.deepcopy(source)
    manifest = json.loads(version_source["tileset.json"]); manifest["asset"]["version"] = "1.0"
    version_source["tileset.json"] = json.dumps(manifest).encode()
    version_input = package_files(version_source, "version-1.0")
    version_before = sha(version_input.read_bytes())
    version_output = work / "version-1.0-implicit.3tz"; version_output.write_bytes(b"prior output")
    version_run = run(["convert-to-implicit", "-i", str(version_input), "-o", str(version_output), "--force"])
    version = {"exit": version_run.returncode, "inputUntouched": sha(version_input.read_bytes()) == version_before,
               "priorReplaced": version_output.read_bytes() != b"prior output",
               "outputAssetVersion": json.loads(members(version_output)["tileset.json"])["asset"]["version"],
               "interpretation": "Admission/declaration observation; does not establish mutated native-vector source as valid 3D Tiles 1.0."}
    ancestor = copy.deepcopy(source); manifest = json.loads(ancestor["tileset.json"])
    schema = None
    for child in manifest["root"]["children"]:
        old = child["content"]["uri"]; data = ancestor.pop(old); js = struct.unpack_from("<I", data, 12)[0]
        doc = json.loads(data[20:20 + js]); metadata = doc["extensions"]["EXT_structural_metadata"]
        candidate_schema = metadata.pop("schema"); assert schema is None or schema == candidate_schema
        schema = candidate_schema; metadata["schemaUri"] = "../subtrees"
        encoded = encode_glb(doc, data[28 + js:]); new = "t/" + sha(encoded) + ".glb"
        ancestor[new] = encoded; child["content"]["uri"] = new; child["extras"]["encodedBytes"] = len(encoded)
    ancestor["subtrees"] = json.dumps(schema).encode()
    ancestor["tileset.json"] = json.dumps(manifest).encode()
    ancestor_input = package_files(ancestor, "reserved-ancestor")
    ancestor_hash = sha(ancestor_input.read_bytes())
    # A production inspect pass is only source admission evidence, never the oracle.
    admitted = run(["validate", str(ancestor_input)])
    ancestor_output = work / "reserved-ancestor-output.3tz"; ancestor_output.write_bytes(b"prior output")
    failed = run(["convert-to-implicit", "-i", str(ancestor_input), "-o", str(ancestor_output), "--force"])
    receipt = {"scope": "One authored fixture feasibility plus sensitive controls; no broader converter acceptance.",
               "baseline": {"productionSource": "4e1bda6", "mergedAuditTree": "e3d222a", "note": "Production sources unchanged by the merge; supplied frozen CLI artifact.", "binarySha256": PIN},
               "authoredFixtureSha256": sha(fixture.read_bytes()), "explicitSha256": source_hash,
               "implicitSha256": sha(implicit.read_bytes()), "sourceUntouched": sha(explicit.read_bytes()) == source_hash,
               "positive": positive, "sensitivity": sensitivity, "versionMutation": version,
               "reservedAncestor": {"sourceAdmitted": admitted.returncode == 0, "sourceSha256": ancestor_hash,
                                    "exit": failed.returncode, "stdout": failed.stdout, "stderr": failed.stderr,
                                    "sourceUntouched": sha(ancestor_input.read_bytes()) == ancestor_hash,
                                    "priorOutputUntouched": ancestor_output.read_bytes() == b"prior output",
                                    "workDirsRemaining": [p.name for p in work.glob(".tiles-work-*")]},
               "commands": commands}
    (work / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({k: v for k, v in receipt.items() if k != "commands"}, indent=2))


if __name__ == "__main__":
    main()
