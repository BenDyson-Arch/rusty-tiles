#!/usr/bin/env python3
"""Oracle self-test: meshopt golden vectors and exact-arithmetic identities.

    python3 -I oracle/self_test/run.py

Exits 0 when every check passes.
"""

import json
import os
import random
import struct
import sys
from fractions import Fraction

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

from c2o import meshopt  # noqa: E402
from c2o.gltf import LegacyContent, Payload, PayloadError  # noqa: E402
from c2o.check import level_operations  # noqa: E402
from c2o import spans as S  # noqa: E402
from c2o.numbers import JsonNumberError, Overflow, contains_overflow, f32_exact, f32_bits, load_json, lowbit_highbit, value_nodes  # noqa: E402
from c2o.scene import QUAT_WINDOW, Walk, mat_mul, node_matrix, node_visit_units, quaternion_excess  # noqa: E402
from c2o.tileset import Level, LevelVolume  # noqa: E402
from c2o.volumes import LevelTest, parse_volume  # noqa: E402

RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))


# ---------------------------------------------------------------- meshopt

def test_meshopt():
    with open(os.path.join(HERE, "meshopt_vectors.json")) as fh:
        vectors = json.load(fh)
    for case in vectors["triangles"]:
        got = meshopt.decode_triangles(bytes.fromhex(case["encoded"]), case["count"])
        check("meshopt TRIANGLES " + case["name"], got == case["decoded"], got)
    for case in vectors["indices"]:
        got = meshopt.decode_indices(bytes.fromhex(case["encoded"]), case["count"])
        check("meshopt INDICES " + case["name"], got == case["decoded"], got)
    for case in vectors["attributes"]:
        raw = meshopt.decode_attributes(bytes.fromhex(case["encoded"]), case["count"], case["stride"])
        got = [list(struct.unpack_from(case["struct"], raw, i * case["stride"]))
               for i in range(case["count"])]
        check("meshopt ATTRIBUTES " + case["name"], got == case["decoded"], got)
    ex = vectors["specExamples"]["fourBitSentinelGroup"]
    data = bytes.fromhex(ex["bytes"])
    values, pos = meshopt._attribute_group(data, 0, len(data), 2)
    deltas = [d - 256 if d > 127 else d for d in (meshopt._dezigzag8(v) for v in values)]
    check("meshopt spec 4-bit sentinel example", deltas == ex["deltas"] and pos == len(data), deltas)
    for hexbytes, value in vectors["specExamples"]["varint"]["cases"]:
        data = bytes.fromhex(hexbytes)
        got, pos = meshopt._varint(data, 0, len(data))
        check("meshopt varint %s" % hexbytes, got == value and pos == len(data), got)
    err = vectors["specExamples"]["varint"]["erratum"]
    data = bytes.fromhex(err["bytes"])
    got, _ = meshopt._varint(data, 0, len(data))
    check("meshopt varint erratum reads as LEB128", got == err["lebValue"], got)
    # A trivially encoded stream through every mode-3 group must round-trip.
    payload = bytes(range(32)) * 3
    stream = _encode_raw_attributes(payload, 6, 16)
    check("meshopt ATTRIBUTES raw-group round trip",
          meshopt.decode_attributes(stream, 6, 16) == payload)
    # Upstream decodeFilterOct8/Oct12 (pinned meshopt_decoder.test.js): within the
    # specified one-ULP output tolerance of the published values.
    for case in vectors["octahedral"]:
        stream = bytes.fromhex(case["encoded"])
        raw = meshopt.decode_attributes(stream, case["count"], case["stride"])
        fmt = "<%d%s" % (case["count"] * 4, "b" if case["stride"] == 4 else "h")
        got = struct.unpack(fmt, meshopt.octahedral(raw, case["count"], case["stride"]))
        want = [v - 65536 if case["stride"] == 8 and v > 32767 else v - 256 if case["stride"] == 4 and v > 127 else v
                for v in case["decoded"]]
        check("meshopt OCTAHEDRAL " + case["name"],
              all(abs(a - b) <= 1 for a, b in zip(got, want)) and len(got) == len(want), got)
    bad = struct.pack("<4h", 0, 1, 2048, 0)
    try:
        meshopt.octahedral(bad, 1, 8)
        check("meshopt OCTAHEDRAL refuses a non-(2^(K-1)-1) one", False)
    except meshopt.MeshoptError:
        check("meshopt OCTAHEDRAL refuses a non-(2^(K-1)-1) one", True)


def _encode_raw_attributes(data, count, stride):
    """Minimal encoder: one block, every group in mode 3, baseline = zeros."""
    out = bytearray([0xA0])
    groups = (count + 15) // 16
    for k in range(stride):
        out += bytes([0xFF] * ((groups + 3) // 4))
        prev = 0
        deltas = []
        for e in range(groups * 16):
            v = data[e * stride + k] if e < count else prev
            d = (v - prev) & 0xFF
            deltas.append(((d << 1) ^ (0xFF if d & 0x80 else 0)) & 0xFF)
            prev = v
        out += bytes(deltas)
    out += bytes(max(32, stride))
    return bytes(out)


# ---------------------------------------------------------------- spans

def test_worked_example():
    """Contract 'Worked example' (B1): sInst=5, level spans [23, 29]. Units per C2-WB 3.6:
    V = (20 + 25) + (30 + 25) = 100, vertex units 4 x 100 = 400, node units 256."""
    doc = {"scene": 0, "scenes": [{"nodes": [0]}], "meshes": [{}],
           "nodes": [{"translation": [10.5, 0, -3], "scale": [2, 4, 0.25], "mesh": 0}]}
    walk = Walk(doc, 0)
    inst = walk.instances[0]
    tr = S.Tracker()
    P, W = S.level0(inst.G_pairs, S.WORD_PAIR["u16"], S.k_pair(65535), tr)
    own = parse_volume({"box": [1.5, 0, 2, 100, 0, 0, 0, 0, 0, 0, 0, 50]}, "own")
    parent = parse_volume({"box": [0, 0, 0, 2048, 0, 0, 0, 2048, 0, 0, 0, 2048]}, "parent")
    l0 = Level("own")
    l0.volumes.append(LevelVolume("own", 0, own, "own"))
    l1 = Level("parent")
    l1.volumes.append(LevelVolume("ancestor", 1, parent, "parent"))
    transform = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1000.25, -20, 0, 1]
    l1.to_parent_pairs = [S.pair_of(Fraction(v)) for v in transform]
    spans = S.level_spans((P, W), tr.max, [l0, l1])
    check("worked example level-0 P pairs after U",
          P == [(-1, 20), (-2, 18), (2, 18)] and W == (0, 15), (P, W))
    check("worked example sInst == 5", inst.s_inst == 5, inst.s_inst)
    check("worked example level spans == [23, 29]", spans == [23, 29], spans)
    V, rows = level_operations([l0, l1], spans)
    check("C2-WB worked example V == (20 + 25) + (30 + 25) == 100", V == 100, (V, rows))
    check("C2-WB worked example vertex units == 4 x 100 == 400", 4 * V == 400, 4 * V)
    check("C2-WB worked example node units == max(256, 128) == 256", walk.node_units == 256, walk.node_units)


def test_wb_weights():
    """One identity per C2-WB weight (3.1, 3.2), on synthetic levels."""
    def level(volumes, step=False):
        lv = Level("t")
        for kind in volumes:
            vol = parse_volume({"box": [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]} if kind == "box"
                               else {"sphere": [0, 0, 0, 1]}, "t")
            lv.volumes.append(LevelVolume("own", 0, vol, "t"))
        if step:
            lv.to_parent_pairs = [S.ONE] * 16
        return lv
    small = 64  # q = 1: every floor applies
    for name, levels, spans, want in (
            ("level-0 base 20 + box 25", [level(["box"])], [small], 45),
            ("level-0 base 20 + sphere 32", [level(["sphere"])], [small], 52),
            ("level >= 1 step 30 + box 25", [level(["box"]), level(["box"], True)], [small, small], 45 + 55),
            ("level >= 1 without transform: box only", [level(["box"]), level(["box"])], [small, small], 45 + 25),
            ("two spheres + step at level 1", [level(["box"]), level(["sphere", "sphere"], True)], [small, small],
             45 + 30 + 64),
            ("q above every floor (s=577 -> q=100)", [level(["box", "sphere"]), level(["box", "sphere"], True)],
             [577, 577], 300 + 300),
            ("q between floors (s=320 -> q=25)", [level(["box", "sphere"]), level(["sphere"], True)],
             [320, 320], (25 + 25 + 32) + (30 + 32)),
            ("q = 0 (s = 0) uses the floors", [level(["box"])], [0], 45)):
        got, _ = level_operations(levels, spans)
        check("C2-WB V: " + name, got == want, (got, want))
    for s_step, want in ((0, 256), (64, 256), (65, 512), (128, 512), (129, 1152), (577, 12800)):
        check("C2-WB node visit units s_step=%d -> %d" % (s_step, want), node_visit_units(s_step) == want,
              node_visit_units(s_step))
    for text, want in (("{}", 1), ("[]", 1), ("0", 1), ('{"a":[1,2,{"b":null}],"c":"x"}', 7),
                       ("[[[[[1]]]]]", 6), ('{"a":{},"b":[]}', 3)):
        check("C2-WB value nodes %s == %d" % (text, want), value_nodes(load_json(text)) == want,
              value_nodes(load_json(text)))


def test_identity_equivalence():
    base = {"translation": [3.25, -7, 1e6], "rotation": [0.6, 0, 0, 0.8], "children": [1]}
    variants = [{"mesh": 0}, {"mesh": 0, "translation": [0, 0, 0], "rotation": [0, 0, 0, 1],
                              "scale": [1, 1, 1]},
                {"mesh": 0, "matrix": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]}]
    results = []
    for child in variants:
        doc = {"scene": 0, "scenes": [{"nodes": [0]}], "meshes": [{}], "nodes": [base, child]}
        inst = Walk(doc, 0).instances[0]
        tr = S.Tracker()
        P, W = S.level0(inst.G_pairs, S.WORD_PAIR["f32"], S.k_pair(1), tr)
        results.append((inst.G_pairs, P, W, tr.max, inst.s_inst))
    check("identity {mesh} == identity TRS == identity matrix (pairs and spans)",
          results[0] == results[1] == results[2], [r[3:] for r in results])


def _value_pair_soundness(trials=300, seed=20261011):
    """Random chains: every exact quantity's set bits lie inside its pair."""
    rng = random.Random(seed)
    violations = 0
    checks = 0

    def rand_double():
        kind = rng.random()
        if kind < 0.15:
            return rng.choice([0.0, -0.0, 1.0, -1.0, 2.0, 0.5])
        if kind < 0.25:
            return rng.choice([5e-324, 2.0 ** 500, -(2.0 ** 1000) / 2 ** 400, 1e-300])
        return rng.uniform(-1, 1) * 2.0 ** rng.randint(-60, 60)

    def vp(x):
        x = Fraction(x)
        return (x, S.pair_of(x))

    def ok(v, p):
        nonlocal checks, violations
        checks += 1
        if v == 0:
            return
        if p is None:
            violations += 1
            return
        lo, hi = p
        low, high = lowbit_highbit(v)
        if low < lo or high > hi:
            violations += 1

    t = S.Tracker()

    def mul(a, b):
        r = (a[0] * b[0], S.mul(a[1], b[1], t))
        ok(*r)
        return r

    def fsum(terms):
        r = (sum(x[0] for x in terms), S.fsum([x[1] for x in terms], t))
        ok(*r)
        return r

    def neg(a):
        return (-a[0], a[1])

    for _ in range(trials):
        G = [[vp(int(i == j)) for j in range(4)] for i in range(4)]
        for _depth in range(rng.randint(1, 4)):
            q = [vp(rng.uniform(-1, 1)) for _ in range(4)]
            if rng.random() < 0.3:
                q[rng.randint(0, 3)] = vp(0.0)
            s3 = [vp(rand_double()) for _ in range(3)]
            t3 = [vp(rand_double()) for _ in range(3)]
            x, y, z, w = q
            xx, yy, zz, ww = mul(x, x), mul(y, y), mul(z, z), mul(w, w)
            n = fsum([xx, yy, zz, ww])
            two = vp(2)
            H = [[fsum([ww, xx, neg(yy), neg(zz)]), mul(two, fsum([mul(x, y), neg(mul(w, z))])),
                  mul(two, fsum([mul(x, z), mul(w, y)]))],
                 [mul(two, fsum([mul(x, y), mul(w, z)])), fsum([ww, neg(xx), yy, neg(zz)]),
                  mul(two, fsum([mul(y, z), neg(mul(w, x))]))],
                 [mul(two, fsum([mul(x, z), neg(mul(w, y))])), mul(two, fsum([mul(y, z), mul(w, x)])),
                  fsum([ww, neg(xx), neg(yy), zz])]]
            L = [[mul(H[r][c], s3[c]) for c in range(3)] + [mul(n, t3[r])] for r in range(3)]
            L.append([vp(0), vp(0), vp(0), n])
            G = [[fsum([mul(G[i][k], L[k][j]) for k in range(4)]) for j in range(4)] for i in range(4)]
        a = vp(f32_exact(rng.getrandbits(31)) or 0) if rng.random() < 0.8 else vp(0)
        a = (a[0], S.WORD_PAIR["f32"])
        k = vp(1)
        P = [fsum([mul(G[r][0], a), mul(G[r][1], a), mul(G[r][2], a), mul(G[r][3], k)]) for r in range(3)]
        W = mul(G[3][3], k)
        P = [P[0], neg(P[2]), P[1]]
        c = [vp(rand_double()) for _ in range(3)]
        for j in range(3):
            cw = mul(c[j], W)
            d = fsum([P[j], neg(cw)])
            mul(d, d)
    return checks, violations


def test_soundness():
    checks, violations = _value_pair_soundness()
    check("pair soundness over random chains (%d quantities)" % checks, violations == 0, violations)


# ---------------------------------------------------------------- exact identities

def test_identities():
    for bits in (0x00000001, 0x007FFFFF, 0x00800000, 0x3F800000, 0x7F7FFFFF, 0xBE99999A, 0x80000000):
        expect = Fraction(struct.unpack("<f", struct.pack("<I", bits))[0])
        check("f32 exact decode %08x" % bits, f32_exact(bits) == expect, f32_exact(bits))
    check("f32 NaN/Inf rejected", f32_exact(0x7F800000) is None and f32_exact(0x7FC00000) is None)
    check("pair of -0.0 is ZERO", S.pair_of(Fraction(-0.0)) is None)
    check("pair of 10.5 is (-1, 3)", S.pair_of(Fraction(10.5)) == (-1, 3))
    check("pair of 1000.25 is (-2, 9)", S.pair_of(Fraction(1000.25)) == (-2, 9))
    check("k pairs", [S.k_pair(k) for k in (1, 127, 255, 32767, 65535)]
          == [(0, 0), (0, 6), (0, 7), (0, 14), (0, 15)])
    check("JSON overflow literal becomes a non-number marker",
          isinstance(load_json(b'{"a": 1e999}')["a"], Overflow) and contains_overflow(load_json(b'[[1e400]]')))
    try:
        load_json(b'[NaN]')
        check("JSON NaN token rejected", False)
    except JsonNumberError:
        check("JSON NaN token rejected", True)
    reps = load_json(b'[1.0, 1e0, 10e-1, 1000.000e-3, -0.0, 0e999999999999999999999999999999, 1.5, 1e-999]')
    check("exact integer representations read as integers",
          reps[:6] == [1, 1, 1, 1, 0, 0] and all(type(v) is int for v in reps[:6])
          and reps[6] == 1.5 and type(reps[7]) is float, reps)
    check("JSON number is nearest binary64", load_json(b"[0.1]")[0] == 0.1)
    rng = random.Random(7)
    for _ in range(20):
        q = [rng.uniform(-1, 1) for _ in range(4)]
        R = node_matrix({"rotation": q})
        RT = [[R[j][i] for j in range(4)] for i in range(4)]
        I = mat_mul(R, RT)
        ident = all(I[i][j] == (1 if i == j else 0) for i in range(4) for j in range(4))
        if not ident:
            break
    check("R(q) = H(q)/|q|^2 is exactly orthogonal for arbitrary q", ident)
    x = 2.0 ** -9
    check("quaternion window: |q|^2-1 == 2^-18 is admitted",
          quaternion_excess([x, 0, 0, 1]) == QUAT_WINDOW)
    beyond = struct.unpack("<d", struct.pack("<Q", struct.unpack("<Q", struct.pack("<d", x))[0] + 1))[0]
    check("quaternion window: next representable beyond is excluded",
          quaternion_excess([beyond, 0, 0, 1]) > QUAT_WINDOW)
    # U applied once: W09's root matrix cancels it.
    G = node_matrix({"matrix": [1, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0, 0, 0, 0, 1]})
    y = [sum(G[r][c] * v for c, v in enumerate([1, 2, 3, 1])) for r in range(3)]
    check("W09 node frame then U", (y[0], -y[2], y[1]) == (1, 2, 3), y)


def test_integer_predicate():
    """Integer LevelTest agrees with the slow Fraction predicate."""
    rng = random.Random(11)
    mismatches = 0
    for trial in range(400):
        scale = rng.choice([Fraction(1, 2 ** 149), Fraction(1, 65535), Fraction(1, 127), Fraction(1)])
        A = [[Fraction(rng.randint(-8, 8), rng.choice([1, 3, 65535, 2 ** 20])) for _ in range(3)]
             for _ in range(3)]
        b = [Fraction(rng.randint(-100, 100), rng.choice([1, 7, 2 ** 10])) for _ in range(3)]
        if trial % 2:
            vol = parse_volume({"box": [rng.uniform(-5, 5) for _ in range(3)]
                                + [rng.uniform(0, 30), 0, 0, 0, 0, -rng.uniform(0, 30), 0,
                                   rng.uniform(0, 30), 0]}, "t")
        else:
            vol = parse_volume({"sphere": [rng.uniform(-5, 5) for _ in range(3)]
                                + [rng.uniform(0, 40)]}, "t")
        test = LevelTest(vol, A, b, scale)
        for _ in range(20):
            if scale == Fraction(1, 2 ** 149):
                v = [rng.randint(-2 ** 150, 2 ** 150) for _ in range(3)]
            else:
                v = [rng.randint(-70000, 70000) for _ in range(3)]
            x = [c * scale for c in v]
            p = [sum(A[r][c] * x[c] for c in range(3)) + b[r] for r in range(3)]
            if test.inside(v) != vol.contains(p):
                mismatches += 1
    check("integer predicate == Fraction predicate (8000 cases)", mismatches == 0, mismatches)
    # Signed permutation box: rows covered by permuted columns, rotated box detected.
    vol = parse_volume({"box": [0, 0, 0, 0, -2, 0, 0, 0, 3, 5, 0, 0]}, "t")
    check("signed-permutation half-lengths", vol.kind == "box" and vol.h == [5, 2, 3], vol.h)
    vol = parse_volume({"box": [0, 0, 0, 3, 4, 0, -4, 3, 0, 0, 0, 5]}, "t")
    check("non-permutation box is rotatedBox", vol.kind == "rotatedBox")


def test_repeated_resource_bytes():
    """B and Res count every buffer/image entry, also when two entries name one member: C1
    payload contract (c1-payload-integrity-contract.md:330-331), "Repeated resource
    declarations charge repeated actual bytes; no digest/URI deduplication changes that
    meaning"."""
    class Members:
        def __init__(self, members):
            self.members = members
            self.infos = members

        def read(self, name):
            return self.members[name]
    doc = {"asset": {"version": "2.0"},
           "buffers": [{"uri": "b.bin", "byteLength": 12}, {"uri": "./b.bin", "byteLength": 12},
                       {"uri": "b.bin", "byteLength": 12}],
           "images": [{"uri": "b.bin"}]}
    gltf = json.dumps(doc).encode()
    p = Payload(Members({"g.gltf": gltf, "b.bin": bytes(12)}), "g.gltf")
    check("C2-WB B counts a member once per resolved entry (3 buffers + 1 image naming b.bin)",
          p.bytes_read == len(gltf) + 4 * 12, (p.bytes_read, len(gltf) + 4 * 12))
    check("C2-WB Res counts entries, aliases included", p.external_resources == 4, p.external_resources)


def test_legacy_content():
    """H7 (PROPOSED): b3dm/i3dm/pnts/cmpt are C1 Unsupported, by final suffix before any read (L1)
    or by the first four bytes (L2); existence first (L0); nothing after the magic is read."""
    class Members:
        def __init__(self, members):
            self.infos = members
            self.reads = []

        def read(self, name):
            self.reads.append(name)
            return self.infos[name]

    def outcome(name, data):
        archive = Members({name: data})
        try:
            Payload(archive, name)
        except LegacyContent as err:
            return ("Unsupported", err.fmt, err.rule, archive.reads)
        except PayloadError:
            return ("Undecided", None, None, archive.reads)
        return ("decoded", None, None, archive.reads)
    for name, fmt in (("t.b3dm", "b3dm"), ("t.i3dm", "i3dm"), ("t.pnts", "pnts"), ("t.cmpt", "cmpt"),
                      ("d/T.B3DM", "b3dm"), ("t.PnTs", "pnts")):
        # Suffix verdicts never read the member: corrupt or empty bytes give the same outcome.
        for data in (b"", b"glTF", fmt.encode(), b"\xff" * 64):
            got = outcome(name, data)
            check("L1 %s %r" % (name, data[:4]), got == ("Unsupported", fmt, "suffix", []), got)
    for fmt in ("b3dm", "i3dm", "pnts", "cmpt"):
        for name in ("t.glb", "t.gltf", "T.GLB"):
            got = outcome(name, fmt.encode() + bytes(60))
            check("L2 %s in %s" % (fmt, name), got[:3] == ("Unsupported", fmt, "magic"), got)
    for name in ("b3dm/t.glb", "t.b3dm.glb", "t.cmpt/x.glb"):
        got = outcome(name, b"{\"asset\":{\"version\":\"2.0\"},\"generator\":\"b3dm\"}")
        check("legacy words outside the final suffix and magic: %s" % name, got[0] == "decoded", got)
    for name in ("t.bin", "t", "t.vctr", "t.b3dmx", "t.glb3dm"):
        got = outcome(name, b"b3dm" + bytes(28))
        check("non-admitted suffix %s is Undecided, not legacy" % name, got == ("Undecided", None, None, []), got)
    archive = Members({})
    try:
        Payload(archive, "missing.b3dm")
        got = "decoded"
    except LegacyContent:
        got = "Unsupported"
    except PayloadError:
        got = "Undecided"
    check("L0: a missing .b3dm member is not classified", got == "Undecided", got)


def main():
    for test in (test_meshopt, test_worked_example, test_wb_weights, test_repeated_resource_bytes, test_legacy_content,
                 test_identity_equivalence, test_soundness,
                 test_identities, test_integer_predicate):
        try:
            test()
        except Exception as err:  # a crash is a failed check, never a pass
            check(test.__name__ + " raised", False, repr(err))
    failed = [r for r in RESULTS if not r[1]]
    for name, ok, detail in RESULTS:
        print("%s  %s%s" % ("PASS" if ok else "FAIL", name, "" if ok else "  -> %s" % (detail,)))
    print("%d checks, %d failed" % (len(RESULTS), len(failed)))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
