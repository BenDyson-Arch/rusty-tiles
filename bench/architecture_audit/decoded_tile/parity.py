"""Decoded-tile parity against the independent S5 published-byte decoder.

For every content tile of an archive, the published GLB (read with zipfile, not
rusty-tiles) and rusty-tiles' plain GLB view are both decoded by
`../mesh_hlod/s5/decode.mjs`, and `decode_tile(..., format="arrays")` is compared
with them:
- triangle corners equal decode.mjs's positions exactly, after the Y-up to Z-up
  turn (x, -z, y) the arrays document;
- corner order and source addresses (node, mesh, primitive, triangle) match;
- per-face feature IDs resolve to the same property-table rows;
- normals within decode.mjs's own per-face `normalTolerance` (the OCTAHEDRAL
  filter may round one code unit differently from the linked decoder) plus
  float32 storage, and UVs within 1e-7 (arrays store float32);
- the plain GLB decodes to the same faces as the published GLB, normals within
  the same tolerance.

Usage: python parity.py ARCHIVE.3tz [--limit N] [--report out.json]
Needs node, numpy and an installed rusty_tiles wheel.
"""
import argparse
import json
import math
from pathlib import Path
import subprocess
import sys
import zipfile

import numpy as np
import rusty_tiles

DECODER = Path(__file__).resolve().parents[1] / "mesh_hlod/s5/decode.mjs"


def oracle(glb):
    run = subprocess.run(["node", str(DECODER), "-"], input=glb, capture_output=True, check=False)
    if run.returncode:
        raise RuntimeError(run.stderr.decode())
    return json.loads(run.stdout)["faces"]


def z_up(v):
    return [v[0], -v[2], v[1]]


def compare(uri, decoded, faces):
    stats = {"faces": len(faces), "normal_max": 0.0, "normal_excess": 0.0, "uv_max": 0.0}
    k = 0
    for p in decoded["primitives"]:
        tables = decoded["property_tables"]
        for t, triangle in enumerate(p["indices"]):
            face = faces[k]
            k += 1
            assert face["source"] == [p["node"], p["mesh"], p["primitive"], t], (uri, face["source"])
            for c, corner in enumerate(triangle):
                expected = z_up(face["xyz"][c])
                got = p["positions"][corner].tolist()
                assert got == expected, (uri, t, c, got, expected)
                if p["normals"] is not None:
                    error = float(max(abs(a - b) for a, b in zip(p["normals"][corner], z_up(face["normals"][c]))))
                    stats["normal_max"] = max(stats["normal_max"], error)
                    excess = error - face["normalTolerance"] - 1e-6
                    stats["normal_excess"] = max(stats["normal_excess"], excess)
                else:
                    assert face["normals"] is None
                if p["uvs"]:
                    error = float(max(abs(a - b) for a, b in zip(p["uvs"][0][corner], face["uvs"][c])))
                    stats["uv_max"] = max(stats["uv_max"], error)
                else:
                    assert face["uvs"] is None
            for s in p["feature_ids"]:
                row = int(s["values"][triangle[0]])
                columns = tables[s["property_table"]]["properties"]
                got = {name: str(int(col[row])) if col.dtype == np.uint64 else int(col[row])
                       for name, col in columns.items()}
                assert got == face["ids"][s["label"]], (uri, t, got, face["ids"][s["label"]])
    assert k == len(faces), (uri, k, len(faces))
    assert stats["normal_excess"] <= 0 and stats["uv_max"] <= 1e-7, (uri, stats)
    return stats


def same_faces(plain, published):
    """Equal faces; normals within the oracle's tolerance. Returns the largest normal gap."""
    assert len(plain) == len(published)
    gap = 0.0
    for a, b in zip(plain, published):
        for key in ("xyz", "uvs", "ids", "source", "material"):
            assert a[key] == b[key], (key, a[key], b[key])
        if b["normals"] is None:
            assert a["normals"] is None
            continue
        error = max(abs(x - y) for p, q in zip(a["normals"], b["normals"]) for x, y in zip(p, q))
        assert error <= b["normalTolerance"], (error, b["normalTolerance"])
        gap = max(gap, error)
    return gap


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    tiles = rusty_tiles.list_tiles(args.archive)[: args.limit]
    report = {"archive": str(args.archive), "rusty_tiles": rusty_tiles.__version__, "tiles": []}
    with zipfile.ZipFile(args.archive) as archive:
        for tile in tiles:
            published = oracle(archive.read(tile["uri"]))
            plain = oracle(rusty_tiles.decode_tile(args.archive, tile["uri"]))
            plain_gap = same_faces(plain, published)
            decoded = rusty_tiles.decode_tile(args.archive, tile["uri"], format="arrays")
            stats = compare(tile["uri"], decoded, published)
            report["tiles"].append({"uri": tile["uri"], "role": tile["role"], "plain_normal_max": plain_gap, **stats})
    report["faces"] = sum(t["faces"] for t in report["tiles"])
    report["normal_max"] = max(t["normal_max"] for t in report["tiles"])
    report["uv_max"] = max(t["uv_max"] for t in report["tiles"])
    report["plain_normal_max"] = max(t["plain_normal_max"] for t in report["tiles"])
    report["leaves"] = sum(t["role"] == "leaf" for t in report["tiles"])
    text = json.dumps(report, indent=1)
    if args.report:
        args.report.write_text(text)
    print(json.dumps({k: v for k, v in report.items() if k != "tiles"} | {"tiles": len(report["tiles"])}))


if __name__ == "__main__":
    main()
