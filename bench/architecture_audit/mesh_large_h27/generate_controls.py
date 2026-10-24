#!/usr/bin/env python3
"""Stdlib-only independent immutable-geometry and appearance witnesses.

These are authored handoff/assembler fixtures, not executions of H17 or a
published geometric certificate. Their reference source is the exact decoded
oriented triangle, so the analytic correspondence bound is zero in both
directions. Real H17 sink/certificate integration remains a separate gate.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct


def f32(value):
    return struct.unpack("<f", struct.pack("<f", value))[0]


def bits(value):
    return struct.unpack("<I", struct.pack("<f", value))[0]


def unit(vector):
    norm = math.hypot(*vector)
    assert norm > 0 and math.isfinite(norm)
    return [value / norm for value in vector]


def mul(matrix, vector):
    return [sum(row[i] * vector[i] for i in range(3)) for row in matrix]


def solve(matrix, vector):
    """Independent pivoted elimination, not production's cofactor inverse."""
    rows = [list(row) + [vector[i]] for i, row in enumerate(matrix)]
    for column in range(3):
        pivot = max(range(column, 3), key=lambda row: abs(rows[row][column]))
        rows[column], rows[pivot] = rows[pivot], rows[column]
        assert rows[column][column] != 0
        divisor = rows[column][column]
        rows[column] = [value / divisor for value in rows[column]]
        for row in range(3):
            if row == column:
                continue
            factor = rows[row][column]
            rows[row] = [a - factor * b for a, b in zip(rows[row], rows[column])]
    return unit([row[3] for row in rows])


def angle(a, b):
    return math.degrees(math.acos(max(-1.0, min(1.0, sum(x*y for x, y in zip(unit(a), unit(b)))))))


def gamma(value):
    return value * 12.92 if value <= 0.0031308 else 1.055 * value**(1/2.4) - 0.055


class Wire:
    def __init__(self):
        self.data = bytearray()
        self.views = []
        self.accessors = []

    def view(self, data, *, stride=None, target=None):
        self.data.extend(bytes(-len(self.data) % 8))
        result = len(self.views)
        view = {"buffer": 0, "byteOffset": len(self.data), "byteLength": len(data)}
        if stride is not None:
            view["byteStride"] = stride
        if target is not None:
            view["target"] = target
        self.views.append(view)
        self.data.extend(data)
        return result

    def attribute(self, data, component, shape, count, *, stride=None, extrema=None):
        result = len(self.accessors)
        accessor = {"bufferView": self.view(data, stride=stride, target=34962),
                    "componentType": component, "type": shape, "count": count}
        if extrema:
            accessor["min"], accessor["max"] = extrema
        self.accessors.append(accessor)
        return result


def glb(doc, binary):
    text = json.dumps(doc, separators=(",", ":"), allow_nan=False).encode()
    text += b" " * (-len(text) % 4)
    binary += bytes(-len(binary) % 4)
    return (struct.pack("<III", 0x46546C67, 2, 28 + len(text) + len(binary))
            + struct.pack("<II", len(text), 0x4E4F534A) + text
            + struct.pack("<II", len(binary), 0x004E4942) + binary)


def fixture():
    wire = Wire()
    cases = [
        {"name": "float", "component": 5126,
         "words": [[-0.0, 0.0, 0.25], [2.0, 0.0, 0.25], [0.0, 1.0, 0.25]],
         "translation": [10.0, -2.0, 0.0], "scale": [0.5, 2.0, 1.0]},
        {"name": "u16", "component": 5123,
         "words": [[0, 0, 0], [16, 0, 0], [0, 8, 0]],
         "translation": [-3.0, 5.0, -1.0], "scale": [0.125, 0.25, 0.5]},
    ]
    meshes, nodes, faces = [], [], []
    for row, case in enumerate(cases):
        words = case["words"]
        if case["component"] == 5126:
            position = b"".join(struct.pack("<3f", *word) for word in words)
            raw_words = [[bits(value) for value in word] for word in words]
            stride = 12
        else:
            position = b"".join(struct.pack("<3H", *word) + b"\x00\x00" for word in words)
            raw_words = words
            stride = 8
        extrema = [[min(word[a] for word in words) for a in range(3)],
                   [max(word[a] for word in words) for a in range(3)]]
        pos = wire.attribute(position, case["component"], "VEC3", 3, stride=stride, extrema=extrema)
        attrs, feature_ids = {"POSITION": pos}, []
        # Attribute numbers intentionally differ from feature-set array order.
        for table, attribute_number, label in [(0, 0, "source_primitive"),
                                               (1, 3, "source_triangle"),
                                               (2, 7, "independent_class")]:
            attrs[f"_FEATURE_ID_{attribute_number}"] = wire.attribute(struct.pack("<3f", *([float(row)]*3)), 5126, "SCALAR", 3)
            feature_ids.append({"featureCount": 2, "attribute": attribute_number,
                                "propertyTable": table, "label": label})
        # Opposite cyclic orders distinguish reordering from reversal.
        indices = [0, 1, 2] if row == 0 else [1, 2, 0]
        index = len(wire.accessors)
        wire.accessors.append({"bufferView": wire.view(struct.pack("<3H", *indices), target=34963),
                               "componentType": 5123, "type": "SCALAR", "count": 3})
        meshes.append({"primitives": [{"attributes": attrs, "indices": index,
                        "extensions": {"EXT_mesh_features": {"featureIds": feature_ids}}}]})
        nodes.append({"mesh": row, "translation": case["translation"], "scale": case["scale"]})
        decoded = [[f32(f32(f32(float(word[a])) * case["scale"][a]) + case["translation"][a])
                    for a in range(3)] for word in words]
        source = {"node": 5, "mesh": 11, "primitive": 7 + row*2, "instance": 12345 + row}
        faces.append({"ordinal": row, "source": source, "source_triangle": row,
                      "material": 0xFFFFFFFF, "position_component": case["component"],
                      "words": [raw_words[i] for i in indices],
                      "transform": {"translation": [bits(v) for v in case["translation"]],
                                    "scale": [bits(v) for v in case["scale"]]},
                      "decoded_positions": [decoded[i] for i in indices],
                      "position_bits": [[bits(v) for v in decoded[i]] for i in indices],
                      "welds": [[bits(v) if v != 0 else 0 for v in decoded[i]] for i in indices],
                      "features": [{"set": attribute, "feature_entry": n, "table": n, "value": row}
                                   for n, attribute in enumerate((0, 3, 7))],
                      "donor_ranges": [[row, row+1]],
                      "fill_bound": None})
    classes, tables, table_reference = {}, [], []
    table_inputs = [
        ("source_primitive", {"node_index": ("UINT32", [5, 5]),
          "mesh_index": ("UINT32", [11, 11]), "primitive_index": ("UINT32", [7, 9]),
          "expanded_primitive": ("UINT64", [12345, 12346]),
          "e_A": ("FLOAT64", [0.0, 0.0]), "V": ("UINT32", [0, 0]), "F": ("UINT32", [0, 0])}),
        ("source_triangle", {"triangle_index": ("UINT64", [0, 1]),
          "primitive_table_row": ("UINT32", [0, 1])}),
        ("independent_class", {"business_id": ("UINT64", [2**53+7, 2**63+11]),
          "literal_factor": ("FLOAT64", [0.25, 0.75])}),
        # Unreferenced metadata still belongs to immutable provenance ownership.
        ("proxy_regions", {"V": ("UINT32", [0, 0]), "F": ("UINT32", [0, 0]),
          "source_region": ("UINT64", [17, 19])}),
    ]
    format_codes = {"UINT32": "I", "UINT64": "Q", "FLOAT64": "d"}
    for name, columns in table_inputs:
        schema, properties = {}, {}
        for key, (component, values) in columns.items():
            schema[key] = {"type": "SCALAR", "componentType": component, "required": True}
            encoded = struct.pack("<" + format_codes[component] * len(values), *values)
            properties[key] = {"values": wire.view(encoded)}
        classes[name] = {"properties": schema}
        tables.append({"class": name, "count": 2, "properties": properties})
        table_reference.append({"class": name, "count": 2,
                                "columns": {k: {"component": t, "values": v} for k, (t, v) in columns.items()}})
    doc = {"asset": {"version": "2.0", "generator": "H27 independent immutable fixture"},
           "scene": 0, "scenes": [{"nodes": [0, 1]}], "nodes": nodes, "meshes": meshes,
           "accessors": wire.accessors, "bufferViews": wire.views,
           "buffers": [{"byteLength": len(wire.data)}],
           "extensionsUsed": ["KHR_mesh_quantization", "EXT_mesh_features", "EXT_structural_metadata"],
           "extensionsRequired": ["KHR_mesh_quantization"],
           "extensions": {"EXT_structural_metadata": {
               "schema": {"id": "h27_independent_all_tables_v1", "classes": classes},
               "propertyTables": tables}}}
    binary = glb(doc, bytes(wire.data))
    reference = {"status": "independently authored immutable handoff fixture; not actual H17 output",
                 "glb_sha256": hashlib.sha256(binary).hexdigest(),
                 "glb_bytes": len(binary), "geometric_error": 0.0,
                 "geometry_reference": "source faces equal decoded proxy faces exactly, both directions",
                 "face_order": faces, "tables": table_reference,
                 "all_feature_sets": 3, "all_property_tables": 4,
                 "required_preservation": ["oriented faces", "raw POSITION words and TRS bits",
                                           "all feature sets/table links", "all source metadata columns",
                                           "original weld bits", "source/material/origin labels"]}
    return binary, reference


def readback(binary):
    """A second parsing route independently audits stored geometry/metadata."""
    assert struct.unpack_from("<III", binary) == (0x46546C67, 2, len(binary))
    size = struct.unpack_from("<I", binary, 12)[0]
    doc = json.loads(binary[20:20+size])
    start = 20 + size
    assert binary[start+4:start+8] == b"BIN\0"
    data = binary[start+8:]

    def values(index):
        accessor = doc["accessors"][index]
        view = doc["bufferViews"][accessor["bufferView"]]
        component = {5123: "H", 5126: "f"}[accessor["componentType"]]
        width = {"SCALAR": 1, "VEC3": 3}[accessor["type"]]
        fmt = "<" + component * width
        stride = view.get("byteStride", struct.calcsize(fmt))
        return [struct.unpack_from(fmt, data, view.get("byteOffset", 0) + n*stride)
                for n in range(accessor["count"])]

    faces = []
    for node in doc["nodes"]:
        for primitive in doc["meshes"][node["mesh"]]["primitives"]:
            p = values(primitive["attributes"]["POSITION"])
            p = [[f32(f32(v[a] * node["scale"][a]) + node["translation"][a]) for a in range(3)] for v in p]
            i = [v[0] for v in values(primitive["indices"])]
            features = []
            for set_ in primitive["extensions"]["EXT_mesh_features"]["featureIds"]:
                ids = values(primitive["attributes"][f"_FEATURE_ID_{set_['attribute']}"])
                assert len({ids[j][0] for j in i}) == 1
                features.append((int(ids[i[0]][0]), set_["propertyTable"]))
            faces.append({"positions": [p[j] for j in i], "features": features})
    metadata = doc["extensions"]["EXT_structural_metadata"]
    tables = []
    for table in metadata["propertyTables"]:
        properties = metadata["schema"]["classes"][table["class"]]["properties"]
        columns = {}
        for key, column in table["properties"].items():
            view = doc["bufferViews"][column["values"]]
            component = properties[key]["componentType"]
            code = {"UINT32": "I", "UINT64": "Q", "FLOAT64": "d"}[component]
            values_ = list(struct.unpack_from("<" + code*table["count"], data, view["byteOffset"]))
            columns[key] = {"component": component, "values": values_}
        tables.append({"class": table["class"], "count": table["count"], "columns": columns})
    return faces, tables


def references():
    donor = [[0.8, 0, 0], [0, 1, 0], [0.6, 0, 1]]
    effective = unit([1, 0, 1])
    correct = unit(mul(donor, effective))
    repaired = unit([1, 0, 1])
    # Nonorthogonal proxy too: its true inverse and transpose differ visibly.
    proxy = [[0.8, 0, 0], [0, 1, 0], [0.6, 0, 1]]
    world = [0.0, 0.0, 1.0]
    inverse = solve(proxy, world)
    transpose = unit([sum(proxy[j][i]*world[j] for j in range(3)) for i in range(3)])
    colour = round(255 * gamma(0.5))
    tuples = [(1, 9, 0, 255), (1, 9, 0, 255), (2, 8, 0, 255), (3, 8, 0, 255)]
    return {"scope": "independent analytic semantic references; no production execution",
            "authored_skew": {"donor_columns": [[0.8, 0, 0.6], [0, 1, 0], [0, 0, 1]],
                "rgb": [1, 0.5, 1], "normal_scale": 1,
                "expected_world": correct, "wrong_orthogonalized_world": repaired,
                "wrong_angle_degrees": angle(correct, repaired)},
            "full_inverse": {"proxy_columns": [[0.8, 0, 0.6], [0, 1, 0], [0, 0, 1]],
                "world": world, "expected_tangent": inverse, "wrong_transpose": transpose,
                "wrong_angle_degrees": angle(inverse, transpose)},
            "gamma": {"black_white_srgb_byte": colour, "linear_data_byte": 128,
                      "semantic_wrong_class_error_bytes": colour-128},
            "id_tuple": {"contributors": tuples, "support": [2, 2, 3, 3],
                         "expected": tuples[0], "wrong_channel_modes": [1, 8, 0, 255],
                         "wrong_is_member": (1, 8, 0, 255) in tuples},
            "limits": ["must still execute sensitive production mutants", "not codec-floor acceptance",
                       "not H17 handoff integration or a consumer proof"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output-dir", type=Path, required=True,
        help="Persistent cache directory for generated fixtures and manifest.",
    )
    args = parser.parse_args()
    output_dir = args.output_dir.expanduser().resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    binary, reference = fixture()
    faces, tables = readback(binary)
    assert tables == reference["tables"]
    for actual, expected in zip(faces, reference["face_order"]):
        assert actual["positions"] == expected["decoded_positions"]
        assert actual["features"] == [(f["value"], f["table"]) for f in expected["features"]]
    assert reference["face_order"][0]["words"][0][0] == 0x80000000
    sensitivity = references()
    assert sensitivity["authored_skew"]["wrong_angle_degrees"] > 12
    assert sensitivity["full_inverse"]["wrong_angle_degrees"] > 12
    assert sensitivity["gamma"]["black_white_srgb_byte"] == 188
    assert not sensitivity["id_tuple"]["wrong_is_member"]
    outputs = {"immutable-three-sets.glb": binary,
               "immutable-three-sets.json": (json.dumps(reference, indent=2) + "\n").encode(),
               "appearance-reference.json": (json.dumps(sensitivity, indent=2) + "\n").encode()}
    for name, content in outputs.items():
        (output_dir / name).write_bytes(content)
    manifest = {"generator": Path(__file__).name, "scope": reference["status"],
                "files": {name: {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
                          for name, data in outputs.items()}}
    (output_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"glb_bytes": len(binary), "sha256": reference["glb_sha256"],
                      "faces": len(faces), "feature_sets": 3, "tables": len(tables),
                      "self_checks": "passed"}))


if __name__ == "__main__":
    main()
