"""UNEXECUTED preparation input generator; coordinator runs only after P0/P2.

Standalone-independent tiny raw GLBs; selected metadata framing is a DRAFT
interpretation to adjudicate against pinned primary/consumer, not a proof.
No repository import, codec call, subprocess, external write or large corpus.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path


def align(n, a):
    return ((n + a - 1) // a) * a


def raw_document(doc):
    # Every controlled dimension is an exact small integer. Opaque numeric
    # lexemes are deliberately authored as bytes after ordinary bookkeeping.
    data = json.dumps(doc, ensure_ascii=False, separators=(",", ":")).encode()
    marker = b'"OPAQUE_NUMERIC_SENTINEL"'
    assert data.count(marker) == 1
    return data.replace(marker, b'{"integer":18446744073709551615,"exponent":1e400,"negativeZero":-0.0}')


def frame(raw, binary, metadata):
    j = align(20 + len(raw), 8) - 20 if metadata else align(len(raw), 4)
    b = align(len(binary), 8 if metadata else 4)
    total = 28 + j + b
    return (struct.pack("<4sII", b"glTF", 2, total)
            + struct.pack("<I4s", j, b"JSON") + raw + b" " * (j - len(raw))
            + struct.pack("<I4s", b, b"BIN\0") + binary + b"\0" * (b - len(binary)))


def positions(quantized=False, metadata=False, tail=0):
    binary = bytearray()
    views = []
    expected = []

    def add(name, payload, stride=None):
        binary.extend(b"\0" * (align(len(binary), 8) - len(binary)))
        offset = len(binary)
        binary.extend(payload)
        view = {"buffer": 0, "byteOffset": offset, "byteLength": len(payload)}
        if stride is not None:
            view["byteStride"] = stride
        views.append(view)
        expected.append({"view": len(views) - 1, "role": name, "hex": payload.hex()})
        return len(views) - 1

    p = (struct.pack("<4H", 0, 32768, 65535, 0xA55A)
         + struct.pack("<4H", 65535, 0, 32768, 0x5AA5)) if quantized else struct.pack("<6f", 0., .5, 1., 1., 0., .5)
    add("POSITION whole records including nonzero padding", p, 8 if quantized else 12)
    add("padded U16 feature IDs", struct.pack("<4H", 0, 0xBEEF, 1, 0xCAFE), 4)
    accessors = [{"bufferView": 0, "componentType": 5123 if quantized else 5126,
                  "count": 2, "type": "VEC3", "min": [0, 0, 32768] if quantized else [0, 0, .5], "max": [65535, 32768, 65535] if quantized else [1, .5, 1]},
                 {"bufferView": 1, "componentType": 5123, "count": 2, "type": "SCALAR"}]
    if quantized:
        accessors[0]["normalized"] = True
    primitive = {"attributes": {"POSITION": 0, "_FEATURE_ID_0": 1}, "mode": 0,
                 "extensions": {"EXT_mesh_features": {"featureIds": [{"featureCount": 2, "attribute": 0}]}}}
    doc = {"asset": {"version": "2.0"}, "bufferViews": views, "accessors": accessors,
           "meshes": [{"primitives": [primitive]}], "nodes": [{"mesh": 0}],
           "scenes": [{"nodes": [0]}], "scene": 0,
           "extensionsUsed": ["EXT_mesh_features"], "extras": {"opaque": "OPAQUE_NUMERIC_SENTINEL", "tag": "x" * tail}}
    if quantized:
        doc["extensionsUsed"].append("KHR_mesh_quantization")
        doc["extensionsRequired"] = ["KHR_mesh_quantization"]
        doc["nodes"][0].update(translation=[10, 20, 30], scale=[2, 3, 4])
    if metadata:
        props = {}
        schema = {}
        values = [("enabled", {"type": "BOOLEAN"}, b"\x03"),
                  ("integer", {"type": "SCALAR", "componentType": "INT64"}, struct.pack("<2q", -(2**60 + 3), 2**60 + 3)),
                  ("real", {"type": "SCALAR", "componentType": "FLOAT64"}, struct.pack("<2d", .125, -.25))]
        for name, definition, payload in values:
            props[name] = {"values": add(name, payload)}
            schema[name] = definition
        # Last raw payload is variable-sized to independently vary BIN slack.
        for name, strings in [("name", ["zero", "rocket🚀"]),
                              ("list", ["[9007199254740993,null,2]", "[2,1]" + " " * tail])]:
            encoded = [s.encode("utf8") for s in strings]
            props[name] = {"values": add(name + " UTF8", b"".join(encoded)),
                           "stringOffsets": add(name + " offsets", struct.pack("<3I", 0, len(encoded[0]), sum(map(len, encoded))))}
            schema[name] = {"type": "STRING"}
        doc["extensionsUsed"].append("EXT_structural_metadata")
        doc["extensions"] = {"EXT_structural_metadata": {
            "schema": {"id": "independent-proof", "classes": {"row": {"properties": schema}}},
            "propertyTables": [{"class": "row", "count": 2, "properties": props}]}}
        primitive["extensions"]["EXT_mesh_features"]["featureIds"][0]["propertyTable"] = 0
        # Valid unused raw view varies logical BIN residue without changing
        # metadata table semantics; preserves every physical view object.
        add("unused raw tail for slack residues", b"q" * (tail + 1))
    doc["buffers"] = [{"byteLength": len(binary)}]
    return raw_document(doc), bytes(binary), expected


def raw_matrix():
    # Unused MAT2 U8 accessor occupies six bytes: two padded columns, no
    # required last-column padding. Validity and RAW eligibility need review.
    binary = bytes([1, 2, 0xA5, 0x5A, 3, 4])
    doc = {"asset": {"version": "2.0"}, "buffers": [{"byteLength": 6}],
           "bufferViews": [{"buffer": 0, "byteLength": 6}],
           "accessors": [{"bufferView": 0, "componentType": 5121, "count": 1, "type": "MAT2"}],
           "extras": {"opaque": "OPAQUE_NUMERIC_SENTINEL"}}
    return raw_document(doc), binary, [{"view": 0, "role": "partial final matrix padding RAW", "hex": binary.hex()}]


def generate(output):
    output.mkdir(parents=True, exist_ok=False)
    records = []

    def save(name, data, expectation, views=()):
        (output / name).write_bytes(data)
        records.append({"file": name, "sha256": hashlib.sha256(data).hexdigest(),
                        "bytes": len(data), "draft_expected": expectation,
                        "expected_source_views": list(views),
                        "opaque_numeric_lexemes": ["18446744073709551615", "1e400", "-0.0"]})

    for quantized in [False, True]:
        raw, binary, views = positions(quantized)
        save("quantized.glb" if quantized else "float.glb", frame(raw, binary, False), "admit and losslessly repack", views)
    for residue in range(8):
        raw, binary, views = positions(metadata=True, tail=residue)
        good = frame(raw, binary, True)
        save(f"metadata-{residue}.glb", good, "selected metadata framing candidate; normative/consumer gate HELD", views)
        core = frame(raw, binary, False)
        j = struct.unpack_from("<I", core, 12)[0]
        if (20 + j) % 8 or len(core) % 8:
            save(f"metadata-core4-{residue}.glb", core, "selected metadata envelope InvalidInput; interpretation gate HELD")
        # Bad zero-slack control is produced only if actual slack exists.
        bin_origin = 28 + struct.unpack_from("<I", good, 12)[0]
        if len(good) > bin_origin + len(binary):
            corrupt = bytearray(good)
            corrupt[-1] = 0xA5
            save(f"metadata-slack-nonzero-{residue}.glb", bytes(corrupt), "InvalidInput if selected slack rule is independently accepted")
    raw, binary, views = raw_matrix()
    save("all-raw-matrix.glb", frame(raw, binary, False), "valid partial padded matrix; all-raw byte-identical owned candidate", views)
    (output / "manifest.json").write_text(json.dumps({"status": "Materialized independent DRAFT inputs, not target acceptance", "records": records}, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("new_output_directory", type=Path)
    generate(parser.parse_args().new_output_directory)
