#!/usr/bin/env python3
"""Self-test for the I1 oracle (stdlib only; deterministic).  Run: python3 -I self_test/test_oracle.py

Covers the contract's acceptance-plan A list: AVAILABILITY interleave examples, Morton round trips for level <= 4,
Fraction child-union identities, live-path sums, §2a source dedup on aliasing; plus rank and spread identities, the
§3 normalization exactness property (normalize once, substitute digits == uri(base, substituted raw)), the DC-2
forms, error halving and the subnormal boundary, and the tolerance arithmetic of T-1/T-2.
"""
import importlib.util
import io
import json
import os
import random
import struct
import sys
import tempfile
import zipfile
from fractions import Fraction

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("i1_oracle", os.path.join(os.path.dirname(HERE), "i1_oracle.py"))
O = importlib.util.module_from_spec(spec)
spec.loader.exec_module(O)

CHECKS = []


def check(name, cond):
    CHECKS.append((name, bool(cond)))
    if not cond:
        print("FAIL", name, file=sys.stderr)


def expect_outcome(fn, kind, reason=None):
    try:
        fn()
    except O.Outcome as o:
        return o.kind == kind and (reason is None or reason in o.reason)
    return False


# ---------------------------------------------------------------------------------------------------------------
def test_interleave():
    # AVAILABILITY.adoc "Converting from Tile Coordinates to Morton Index" examples.
    check("quad 0b11,0b00", O.interleave([0b11, 0b00]) == 0b0101)
    check("quad 0b1010,0b0011", O.interleave([0b1010, 0b0011]) == 0b01001110)
    check("quad 0b0110,0b0101", O.interleave([0b0110, 0b0101]) == 0b00110110)
    check("oct 0b001,0b010,0b100", O.interleave([0b001, 0b010, 0b100]) == 0b100010001)
    check("oct 0b111,0b000,0b111", O.interleave([0b111, 0b000, 0b111]) == 0b101101101)
    # Level offsets and bitstream lengths (AVAILABILITY.adoc tables).
    check("offsets quad", [O.level_offset(4, l) for l in range(5)] == [0, 1, 5, 21, 85])
    check("offsets oct", [O.level_offset(8, l) for l in range(4)] == [0, 1, 9, 73])


def test_morton_round_trips():
    ok = True
    for dims, n in ((2, 4), (3, 8)):
        for level in range(5):
            for m in range(n ** level):
                c = O.deinterleave(m, dims, level)
                ok &= O.interleave(c) == m and all(v < (1 << level) for v in c)
                if level < 4:
                    for k in range(n):
                        cc = O.child_coords(*(c + [0] * (3 - dims)), k)[:dims]
                        ok &= O.interleave(list(cc)) == m * n + k
    check("morton round trips and child index (level <= 4)", ok)
    # The P1 layout reference (tests/p1_point_oracle.py:279-287, read for layout only): template substitution of
    # (level, x, y, z) and a pre-order subtree walk; mirrored here as the global/local concatenation identity.
    ok = True
    for L in (1, 2, 3):
        for gx, gy, gz in ((0, 0, 0), (1, 0, 1), (3, 2, 1)):
            for ll in range(L):
                for lx in range(1 << ll):
                    x = (gx << ll) | lx
                    ok &= (x >> ll) == gx and (x & ((1 << ll) - 1)) == lx
    check("global = concat(subtree root, local)", ok)


def test_cells():
    rnd = random.Random(1501)
    ok_union = ok_direct = True
    for octree in (False, True):
        for _ in range(6):
            root = [float(rnd.randint(-50, 50)) for _ in range(3)] + [float(rnd.randint(-9, 9)) / 4 for _ in range(9)]
            for level in range(3):
                for x in range(1 << level):
                    for y in range(1 << level):
                        for z in (range(1 << level) if octree else [0]):
                            pc, ph = O.derived_cell(root, level, (x, y, z), octree)
                            kids = [O.derived_cell(root, level + 1, O.child_coords(x, y, z, k), octree)
                                    for k in range(8 if octree else 4)]
                            # Child half-axes are half the parent's on subdivided axes; QUADTREE keeps axis 2.
                            for kc, kh in kids:
                                for k in range(3):
                                    want = [v / 2 for v in ph[k]] if (k < 2 or octree) else ph[k]
                                    ok_union &= kh[k] == want
                            # The children's centres average to the parent's centre; each parent corner is a
                            # corner of exactly one child (union identity, exact).
                            avg = [sum(kc[j] for kc, _ in kids) / len(kids) for j in range(3)]
                            ok_union &= avg == pc
                            corners = set()
                            for kc, kh in kids:
                                for s in range(8):
                                    p = tuple(kc[j] + sum((1 if (s >> k) & 1 else -1) * kh[k][j] for k in range(3))
                                              for j in range(3))
                                    corners.add(p)
                            for s in range(8):
                                p = tuple(pc[j] + sum((1 if (s >> k) & 1 else -1) * ph[k][j] for k in range(3))
                                          for j in range(3))
                                ok_union &= p in corners
                            # Direct evaluation at level+1 equals progressive halving of the parent (exact).
                            for k_idx, (kc, kh) in enumerate(kids):
                                cx, cy, cz = O.child_coords(x, y, z, k_idx)
                                sub = [cx & 1, cy & 1, cz & 1]
                                prog = [pc[j] + sum(((2 * sub[k] + 1) * Fraction(1, 2) - 1) * ph[k][j]
                                                    for k in range(3 if octree else 2)) for j in range(3)]
                                ok_direct &= prog == kc
    check("Fraction child-union identities", ok_union)
    check("direct cell == progressive subdivision (exact)", ok_direct)


def test_rank_spread():
    rnd = random.Random(7)
    ok = True
    for nbits in (1, 7, 8, 9, 63, 64, 65, 4097, 9000):
        data = bytearray(rnd.getrandbits(8) for _ in range((nbits + 7) // 8))
        if nbits % 8:
            data[-1] &= (1 << (nbits % 8)) - 1
        s = O.Source("V", nbits, lambda d=bytes(data): d, 0)
        bitsl = [(data[i >> 3] >> (i & 7)) & 1 for i in range(nbits)]
        for i in sorted(set([0, nbits - 1, nbits // 2] + [rnd.randrange(nbits) for _ in range(40)])):
            ok &= s.rank(i) == sum(bitsl[:i])
            ok &= s.bit(i) == bool(bitsl[i])
        ok &= sorted(s.setbits()) == [i for i in range(nbits) if bitsl[i]]
    check("strict rank, bit and set-bit identities", ok)
    ok = True
    for n in (4, 8):
        for count in (1, 5, 8, 17):
            v = rnd.getrandbits(count)
            want = 0
            for i in range(count):
                if (v >> i) & 1:
                    want |= ((1 << n) - 1) << (i * n)
            ok &= O.spread(v, count, n) == want
    check("spread replicates each bit N times", ok)


def test_templates():
    base = []
    # DC-2 forms (delta_attacks.out.json resolves each to a .json member).
    for t in ("c/{level}-{x}-{y}-{z}.json", "c/{level}-{x}-{y}-{z}.json/.", "c/{level}-{x}-{y}-{z}.json/",
              "c/x.json/{level}-{x}-{y}-{z}/..", "c/{level}-{x}-{y}-{z}.JSON//./", "c/{level}-{x}-{y}-{z}.js%6Fn",
              "c/{level}-{x}-{y}-{z}.JsOn"):
        check(f"JSON detection {t}", expect_outcome(lambda t=t: O.normalize_template(t, base, True, True, "t"),
                                                    "Unsupported", "external tileset"))
    check("escape guard last segment", expect_outcome(
        lambda: O.normalize_template("c/{level}-{x}-{y}-{z}.j%7{x}on", base, True, True, "t"),
        "Unsupported", "escape adjacent"))
    check("escape guard non-last segment", expect_outcome(
        lambda: O.normalize_template("c%2{x}/{level}-{y}-{z}.glb", base, True, True, "t"), "Unsupported",
        "escape adjacent"))
    check("complete escape before a variable is not guarded",
          O.normalize_template("c%41{x}/{level}-{y}-{z}.glb", base, True, True, "t").text == "cA{x}/{level}-{y}-{z}.glb")
    check("v3.2: escape guard is per segment ('c%/{level}' is a malformed escape)", expect_outcome(
        lambda: O.normalize_template("c%/{level}/{x}-{y}-{z}.glb", base, True, True, "t"), "InvalidInput",
        "malformed"))
    check("variables eliminated", expect_outcome(
        lambda: O.normalize_template("c/{level}/../{x}/../{y}/../{z}/../a.glb", base, True, True, "t"),
        "Unsupported", "eliminated"))
    t = O.normalize_template("./c//{level}/./{x}-{y}-{z}.glb", base, True, True, "t")
    check("normalization exactness C-7d", t.text == "c/{level}/{x}-{y}-{z}.glb")
    check("subtree template is not JSON-detected",
          O.normalize_template("s/{level}/{x}/{y}.json", base, False, False, "t").text == "s/{level}/{x}/{y}.json")
    check("missing variable", expect_outcome(lambda: O.normalize_template("c/{level}/{x}.glb", base, False, True, "t"),
                                             "InvalidInput"))
    check("{z} in QUADTREE", expect_outcome(
        lambda: O.normalize_template("c/{level}/{x}/{y}/{z}.glb", base, False, True, "t"), "InvalidInput"))
    check("query is Unsupported", expect_outcome(
        lambda: O.normalize_template("c/{level}/{x}/{y}.glb?v=1", base, False, True, "t"), "Unsupported"))
    check("backslash is InvalidInput", expect_outcome(
        lambda: O.normalize_template("c\\{level}/{x}/{y}.glb", base, False, True, "t"), "InvalidInput"))
    check("decoded slash is InvalidInput", expect_outcome(
        lambda: O.normalize_template("c%2F{level}/{x}/{y}.glb", base, False, True, "t"), "InvalidInput"))
    check("escape past the root", expect_outcome(
        lambda: O.normalize_template("../{level}/{x}/{y}.glb", base, False, True, "t"), "InvalidInput"))
    check("relative to the document directory",
          O.normalize_template("../c/{level}/{x}/{y}.glb", ["ext", "a"], False, True, "t").text ==
          "ext/c/{level}/{x}/{y}.glb")
    for k, want in ((1024, None), (1025, "Unsupported")):
        tail = "/{level}-{x}-{y}.glb"
        raw = "c/" + "a" * (k - 2 - len(tail)) + tail
        if want:
            check(f"cap {k}", expect_outcome(lambda raw=raw: O.normalize_template(raw, base, False, True, "t"), want))
        else:
            check(f"cap {k}", O.normalize_template(raw, base, False, True, "t").length == k)
    # Decoded braces are literal (structural normalization).
    t = O.normalize_template("c/%7Bx%7D-{level}-{x}-{y}.glb", base, False, True, "t")
    check("decoded brace is literal", t.substitute(2, 3, 1, 0) == "c/{x}-2-3-1.glb")
    # Exactness property (OD-9): normalize once + substitute == uri(base, substituted raw), for random templates.
    rnd = random.Random(2026)
    pieces = [".", "..", "", "a", "b.c", "%41", "%2e", "%2E%2E", "{level}", "{x}", "y{y}z", "{z}", "q{x}{y}",
              "%7Bx%7D", "x.json", "%C3%A9"]
    agree = total = 0
    for _ in range(4000):
        segs = [rnd.choice(pieces) for _ in range(rnd.randint(1, 7))]
        raw = "/".join(segs) + "/{level}{x}{y}{z}.glb" * rnd.randint(0, 1)
        bdir = rnd.choice([[], ["d"], ["d", "e"]])
        try:
            nt = O.normalize_template(raw, bdir, True, False, "t")
        except O.Outcome as o:
            if o.kind != "InvalidInput" or "variable" in o.reason:
                continue
            # A static InvalidInput must also fail uri() for every substitution.
            total += 1
            sub = raw.replace("{level}", "3").replace("{x}", "5").replace("{y}", "0").replace("{z}", "7")
            agree += expect_outcome(lambda: O.resolve_uri(bdir, sub, "t"), "InvalidInput")
            continue
        for lv, x, y, z in ((0, 0, 0, 0), (3, 5, 0, 7), (12, 4095, 17, 1)):
            sub = raw.replace("{level}", str(lv)).replace("{x}", str(x)).replace("{y}", str(y)).replace("{z}", str(z))
            total += 1
            try:
                agree += O.resolve_uri(bdir, sub, "t") == nt.substitute(lv, x, y, z)
            except O.Outcome:
                pass
    check(f"normalize-once exactness ({agree}/{total})", total > 1000 and agree == total)


def test_errors_and_tolerance():
    ok = True
    for e in (0.0, 1.0, 1024.0, 0.1, 2.0 ** -1000):
        for l in range(32):
            ex = Fraction(e) / (1 << l)
            if ex == 0 or ex >= Fraction(sys.float_info.min):
                ok &= Fraction(float(ex)) == ex
    check("derived error e_R 2^-l exact when normal", ok)
    tol = 1e-6 * (1 + 300 ** 0.5)
    outer = O.exact_box([0.0, 0, 0, 10.0, 0, 0, 0, 10.0, 0, 0, 0, 10.0])
    inner = lambda d: O.exact_box([0.0, 0, 0, 10.0 + d, 0, 0, 0, 10.0, 0, 0, 0, 10.0])
    check("T-1 2e-6 inside the scaled tolerance", O.contains(outer, inner(2e-6), "t"))
    check("T-1 2*tol outside", not O.contains(outer, inner(2 * tol), "t"))
    check("T-2 relative error tolerance", O.error_ok(100.0 * (1 + 1e-9), 100.0)
          and not O.error_ok(100.0 + 2 * 1e-8 * 101.0, 100.0))
    # A rotated container (non-axis-aligned) in the binary64 path.
    c, s = 0.6, 0.8
    rot = O.exact_box([0.0, 0, 0, 10 * c, 10 * s, 0, -10 * s, 10 * c, 0, 0, 0, 10.0])
    small = O.exact_box([1.0, 1.0, 0, 1.0, 0, 0, 0, 1.0, 0, 0, 0, 1.0])
    far = O.exact_box([9.0, 9.0, 0, 1.0, 0, 0, 0, 1.0, 0, 0, 0, 1.0])
    check("rotated container", O.contains(rot, small, "t") and not O.contains(rot, far, "t"))


# ---------------------------------------------------------------------------------------------------------------
# Tiny archives built in memory (independent of witnesses/fixturelib.py)

def subtree(js, binchunk=b""):
    j = json.dumps(js, separators=(",", ":")).encode()
    j += b" " * (-len(j) % 8)
    binchunk += b"\x00" * (-len(binchunk) % 8)
    return struct.pack("<4sIQQ", b"subt", 1, len(j), len(binchunk)) + j + binchunk


def archive(members):
    f = tempfile.NamedTemporaryFile(suffix=".3tz", delete=False)
    with zipfile.ZipFile(f, "w", zipfile.ZIP_STORED) as z:
        for k, v in members.items():
            z.writestr(k, v)
    f.close()
    return f.name


def implicit_root(scheme, L, A, sub_uri, contents=None):
    r = {"boundingVolume": {"box": [0.0, 0, 0, 8.0, 0, 0, 0, 8.0, 0, 0, 0, 8.0]}, "geometricError": 8.0,
         "implicitTiling": {"availableLevels": A, "subdivisionScheme": scheme, "subtreeLevels": L,
                            "subtrees": {"uri": sub_uri}}, "refine": "ADD"}
    if contents:
        r["contents"] = [{"uri": u} for u in contents]
    return {"asset": {"version": "1.1"}, "geometricError": 8.0, "root": r}


def test_live_path_and_dedup():
    # Chain of D single-tile QUADTREE subtrees (L = 1): live sum = D x S, cumulative = (2^D - 1) x S for branching 2.
    D = 4
    members = {}
    pad_view = b"\x00" * 1000
    for level in range(D):
        for x in range(1 << level):
            kids = bits = 0
            if level < D - 1:
                bits = 0b0011  # children 0 and 1
            js = {"buffers": [{"byteLength": 8 + len(pad_view)}],
                  "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 1},
                                  {"buffer": 0, "byteOffset": 8, "byteLength": len(pad_view)}],
                  "tileAvailability": {"constant": 1}, "childSubtreeAvailability": {"bitstream": 0}}
            members[f"s/{level}/{x}/0.st"] = subtree(js, bytes([bits]) + b"\x00" * 7 + pad_view)
    members["tileset.json"] = json.dumps(implicit_root("QUADTREE", 1, D, "s/{level}/{x}/{y}.st")).encode()
    path = archive(members)
    S = len(members["s/0/0/0.st"])
    rec = O.evaluate(path)
    check("live path max = D x S", rec["counts"]["maxLivePathBytes"] == D * S)
    check("cumulative = (2^D - 1) x S", rec["counts"]["cumulativeSubtreeBytes"] == (2 ** D - 1) * S)
    check("visits = Tileset + Node + P_t sum + Exit", rec["counts"]["visits"] == 3 + (2 ** D - 1))
    rl = O.evaluate(path, {"document_decoded_bytes": D * S - 1})
    check("live cap - 1 refuses", rl["verdict"]["limit"] == "document_decoded_bytes")
    check("live cap accepts", O.evaluate(path, {"document_decoded_bytes": D * S})["verdict"]["kind"] == "Accepted")
    os.unlink(path)
    # §2a dedup: three layers on one view, plus the tile on the same view -> one View source.
    T = O.level_offset(4, 2)
    tb = bytes([0b00011])  # bits 0,1 (root and first child)
    js = {"buffers": [{"byteLength": 1}], "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 1}],
          "tileAvailability": {"bitstream": 0}, "contentAvailability": [{"bitstream": 0}] * 3,
          "childSubtreeAvailability": {"constant": 0}}
    m = {"tileset.json": json.dumps(implicit_root("QUADTREE", 2, 2, "s/{level}/{x}/{y}.st",
                                                  [f"c{k}/{{level}}/{{x}}/{{y}}.glb" for k in range(3)])).encode(),
         "s/0/0/0.st": subtree(js, tb)}
    for k in range(3):
        m[f"c{k}/0/0/0.glb"] = b"glb"
        m[f"c{k}/1/0/0.glb"] = b"glb"
    path = archive(m)
    rec = O.evaluate(path)
    check("dedup: Scan counts one distinct view", rec["counts"]["scanBytes"] == (T + 7) // 8)
    check("dedup: E counts aliases once per slot", rec["counts"]["emittedContents"] == 6
          and rec["counts"]["referencesCharged"] == 1 + 6)
    os.unlink(path)


def main():
    test_interleave()
    test_morton_round_trips()
    test_cells()
    test_rank_spread()
    test_templates()
    test_errors_and_tolerance()
    test_live_path_and_dedup()
    failed = [n for n, ok in CHECKS if not ok]
    print(json.dumps({"checks": len(CHECKS), "failed": failed}))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
