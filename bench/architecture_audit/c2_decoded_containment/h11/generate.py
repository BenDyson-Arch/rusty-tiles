"""H11 corner fast-path controls: fixtures written with the witness builder, verdicts from the oracle.

    python3 -I h11/generate.py [--check]

Run from `bench/architecture_audit/c2_decoded_containment`. Writes `h11/fixtures/*.3tz` and
`h11/report.json`: per fixture its sha256, the full oracle report (`oracle/c2_oracle.py --json`,
unmodified) and the expected outcome taken from that report. Nothing here reads `src/` or runs
rusty-tiles; production is compared by `acceptance/oracle_parity.rs`
(`production_matches_every_h11_fast_path_control`). `--check` regenerates in memory and exits 1
if any fixture hash or oracle outcome differs from the committed report.

Each pair is a Contained base and a mutated fixture with one change. The pairs are built so that a
broken fast path flips them: the mutants they target are listed per pair (`targets`), and
`acceptance/production-mutants/h11/mutants.py` applies those mutants to production. Coordinates are
written in the tile frame (`X, Y, Z`) and stored as glTF `(X, Z, -Y)`, so the y-up to z-up step `U`
maps them back. The fast path needs corner credit `T/64` per reference, so every single-reference
fixture has at least 128 drawn vertices (two corners) or 520 (eight corners).
"""
import hashlib
import json
import os
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(ROOT, "witnesses"))
import build as B  # noqa: E402

ORACLE = os.path.join(ROOT, "oracle", "c2_oracle.py")


def gltf(p):
    """Tile-frame point to its glTF y-up position."""
    x, y, z = p
    return [x, z, -y]


def f32_up(x):
    return struct.unpack("<f", struct.pack("<I", struct.unpack("<I", struct.pack("<f", x))[0] + 1))[0]


def lcg(seed):
    state = seed
    while True:
        state = (state * 6364136223846793005 + 1442695040888963407) % 2 ** 64
        yield state >> 11


def cloud(seed, count, half):
    """`count` tile-frame points on a 1/64 grid inside the cube of half-length `half` (exact in f32)."""
    r = lcg(seed)
    steps = int(half * 64)
    return [[(next(r) % (2 * steps + 1) - steps) / 64 for _ in range(3)] for _ in range(count)]


def payload(points, ctype=5126, normalized=False, viewless=False, declared=None, node=None):
    g = B.Gltf()
    pos = g.positions(points, ctype, normalized, viewless, declared)
    g.doc["meshes"] = [{"primitives": [{"attributes": {"POSITION": pos}, "mode": 0}]}]
    g.doc["nodes"] = [dict(node or {}, mesh=0)]
    g.doc["scenes"] = [{"nodes": [0]}]
    g.doc["scene"] = 0
    if ctype != 5126:
        g.doc["extensionsUsed"] = ["KHR_mesh_quantization"]
        g.doc["extensionsRequired"] = ["KHR_mesh_quantization"]
    return g.glb()


def archive(volume, glb):
    root = B.tile(volume, content="a.glb")
    return [("tileset.json", B.json_bytes(B.tileset(root))), ("a.glb", glb)]


def box(c, h):
    return {"box": [c[0], c[1], c[2], h[0], 0, 0, 0, h[1], 0, 0, 0, h[2]]}


def pairs():
    """(pair id, targets, note, base members, mutated members)."""
    out = []

    # F1, F3, F6: a sphere needs every corner. The escaping vertex is an off-diagonal corner of the
    # raw (glTF) box; both raw diagonal corners lie inside the sphere. Raw (x, y, z) is tile (x, -z, y),
    # so the raw diagonal runs along tile X = -Y.
    inner = cloud(11, 520, 0.25)
    inner[100] = [-0.6, 0.6, 0.0]
    inner[200] = [0.6, -0.6, 0.0]
    sphere = {"sphere": [0, 0, 0, 1]}
    escape = [p[:] for p in inner]
    escape[400] = [-0.75, -0.75, 0.0]
    out.append(("h11-sphere-offdiagonal", ["two-diagonal-corners", "skip-fallback", "report-first-marked-vertex"],
                "sphere r=1; base corners at distance <= 0.89; tile vertex 400 at (-0.75, -0.75, 0) is outside "
                "while the raw diagonal corners, tile (-0.75, 0.6, -0.25) and (0.6, -0.75, 0.25), are inside",
                archive(sphere, payload([gltf(p) for p in inner])),
                archive(sphere, payload([gltf(p) for p in escape]))))

    # F1 with a rotated node: a box chain, but the composite is not a signed permutation.
    # q = (0, 0, 0.6, 0.8) about glTF z: x' = 0.28x - 0.96y, y' = 0.96x + 0.28y (tile X and Z).
    rotation = {"rotation": [0, 0, 0.6, 0.8]}
    small = [[(next(r) % 25 - 12) / 64, (next(r) % 25 - 12) / 64, 0.0]
             for r in [lcg(13)] for _ in range(520)]
    base = [p[:] for p in small]
    base[300] = [0.3, -0.9, 0.0]
    moved = [p[:] for p in small]
    moved[300] = [0.3, -1.0, 0.0]
    unit = box([0, 0, 0], [1, 1, 1])
    out.append(("h11-rotated-box", ["two-diagonal-corners", "skip-fallback", "report-first-marked-vertex"],
                "node rotation (0, 0, 0.6, 0.8), unit box; glTF vertex 300 (0.3, -1, 0) maps to tile X = 1.044; "
                "the diagonal raw corners map inside",
                archive(unit, payload(base, node=rotation)),
                archive(unit, payload(moved, node=rotation))))

    # F2, F5, F3, F6: the unique maximum is the last vertex, one f32 ulp past the face.
    line = cloud(17, 200, 0.5)
    on_face = [p[:] for p in line]
    on_face[199] = [1.0, 0.0, 0.0]
    past = [p[:] for p in line]
    past[199] = [f32_up(1.0), 0.0, 0.0]
    out.append(("h11-ulp-last-vertex", ["inward-corner", "box-skips-last-vertex", "skip-fallback",
                                        "report-first-marked-vertex"],
                "unit box; vertex 199 at X = 1 (base) or the next f32 above 1 (mutated)",
                archive(unit, payload([gltf(p) for p in on_face])),
                archive(unit, payload([gltf(p) for p in past]))))

    # F2 on integer words: u16 normalized, the maximum raw word 65535 decodes to exactly 1.
    words = [[next(r) % 30000, 0, next(r) % 30000] for r in [lcg(19)] for _ in range(200)]
    at_cap = [w[:] for w in words]
    at_cap[150] = [65534, 0, 0]
    over = [w[:] for w in words]
    over[150] = [65535, 0, 0]
    narrow = box([0, 0, 0], [0.99999, 1, 1])
    out.append(("h11-u16-maximum", ["inward-corner", "skip-fallback"],
                "u16 normalized; box half-length 0.99999 in X holds 65534/65535 but not 65535/65535",
                archive(narrow, payload(at_cap, 5123, True)),
                archive(narrow, payload(over, 5123, True))))

    # F4: declared bounds are not data. Viewless POSITION words are zero whatever min/max say.
    declared = ([0.5, 0.5, 0.5], [0.6, 0.6, 0.6])
    zeros = [[0, 0, 0]] * 520
    around = box([0.55, -0.55, 0.55], [0.6, 0.6, 0.6])
    away = box([0.55, -0.55, 0.55], [0.1, 0.1, 0.1])
    out.append(("h11-declared-bounds", ["trust-declared-bounds", "skip-fallback"],
                "viewless POSITION (all words zero) declaring glTF min/max [0.5, 0.6]^3; the mutated box holds "
                "the declared region but not the origin",
                archive(around, payload(zeros, viewless=True, declared=declared)),
                archive(away, payload(zeros, viewless=True, declared=declared))))

    # F8: chunks start at their own bitmap words. u8 normalized, 2,500 vertices (three chunks).
    grid = [[next(r) % 101, next(r) % 101, next(r) % 101] for r in [lcg(23)] for _ in range(2500)]
    late_in = [w[:] for w in grid]
    late_in[2100] = [150, 0, 0]
    late_out = [w[:] for w in grid]
    late_out[2100] = [255, 0, 0]
    cube = box([0.4, -0.5, 0.5], [0.4, 0.5, 0.5])
    out.append(("h11-third-chunk", ["chunk-offset-ignored", "skip-fallback"],
                "u8 normalized, 2,500 vertices; vertex 2100 (third chunk) at X = 150/255 (base) or 1 (mutated) "
                "against X in [0, 0.8]",
                archive(cube, payload(late_in, 5121, True)),
                archive(cube, payload(late_out, 5121, True))))

    # F6 and the inconclusive-but-contained path: axis points on the unit sphere, corners outside.
    axes = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]]
    ring = [axes[i % 6] for i in range(520)]
    pushed = [p[:] for p in ring]
    pushed[300] = [f32_up(1.0), 0, 0]
    out.append(("h11-inconclusive-sphere", ["report-first-marked-vertex", "skip-fallback"],
                "520 axis points on the unit sphere (every corner test inconclusive, all contained); vertex 300 "
                "moved one f32 ulp outward",
                archive(sphere, payload([gltf(p) for p in ring])),
                archive(sphere, payload([gltf(p) for p in pushed]))))
    return out


def oracle(path):
    out = subprocess.run([sys.executable, "-I", ORACLE, path, "--json"], capture_output=True, text=True,
                         check=True).stdout
    return json.loads(out)


def expected(report):
    e = {"outcome": report["outcome"]}
    if report["outcome"] == "Outside":
        e["facts"] = {k: report["facts"][k] for k in ("depth", "level", "role", "node")}
    if report["outcome"] == "NotInspected":
        e["reasons"] = report["reasons"]
    return e


def main(argv):
    check = "--check" in argv
    fixtures = os.path.join(HERE, "fixtures")
    rows, listing = [], []
    with tempfile.TemporaryDirectory() as tmp:
        for pid, targets, note, base, mutated in pairs():
            for side, members in (("base", base), ("mutated", mutated)):
                cid = "%s.%s" % (pid, side)
                rel = "h11/fixtures/%s.3tz" % cid
                path = os.path.join(tmp if check else fixtures, cid + ".3tz")
                os.makedirs(os.path.dirname(path), exist_ok=True)
                digest = B.write_3tz(path, members)
                report = oracle(path)
                report["archive"] = rel
                rows.append({"id": cid, "pair": pid, "side": side, "file": rel, "sha256": digest,
                             "targets": targets, "note": note, "expected": expected(report), "oracle": report})
                listing.append((cid, report["outcome"], report.get("facts", {}).get("vertex")))
    doc = {"generator": "h11/generate.py", "oracle": "oracle/c2_oracle.py (unmodified)", "rows": rows}
    path = os.path.join(HERE, "report.json")
    if check:
        old = json.load(open(path))
        want = {r["id"]: (r["sha256"], r["expected"]) for r in old["rows"]}
        got = {r["id"]: (r["sha256"], r["expected"]) for r in rows}
        bad = sorted(k for k in set(want) | set(got) if want.get(k) != got.get(k))
        print(json.dumps({"rows": len(rows), "mismatch": bad}))
        return 1 if bad else 0
    with open(path, "w") as fh:
        json.dump(doc, fh, indent=1, sort_keys=True)
        fh.write("\n")
    for row in listing:
        print(*row)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
