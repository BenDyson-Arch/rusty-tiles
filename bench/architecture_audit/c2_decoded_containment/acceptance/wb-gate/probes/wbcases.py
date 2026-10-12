#!/usr/bin/env python3
"""Build the C2-WB measurement matrix under WORK (default /tmp/c2wb-work) and write cases.json.

    python3 -I wbcases.py [WORK] [--only name,name]
"""
import json
import os
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import wbgen as G  # noqa: E402

FOREST = "/home/bend/.cache/forest-acc/work/head-b51f83d"

MATRIX = [
    # name, builder, expect
    # 1-limb operation classes (u8 raw words, small values), L = 16 chain levels
    ("box-L16", lambda: G.chain(16, "box", n=100000)),
    ("sph-L16", lambda: G.chain(16, "sph", n=100000)),
    ("boxT-L16", lambda: G.chain(16, "box", step=True, n=100000)),
    ("sphT-L16", lambda: G.chain(16, "sph", step=True, n=100000)),
    ("ext-L16", lambda: G.chain(16, "box", ext=True, n=100000)),
    ("extT-L16", lambda: G.chain(16, "box", step=True, ext=True, n=100000)),
    ("extS-L16", lambda: G.chain(16, "sph", ext=True, n=100000)),
    ("extTS-L16", lambda: G.chain(16, "sph", step=True, ext=True, n=100000)),
    ("box-L4", lambda: G.chain(4, "box", n=400000)),
    ("extTS-L32", lambda: G.chain(32, "sph", step=True, ext=True, n=50000)),
    ("soloB", lambda: G.chain(1, "box", n=1000000, content_volume=True)),
    ("soloS", lambda: G.chain(1, "sph", n=1000000, content_volume=True)),
    ("solo1B", lambda: G.chain(1, "box", n=1000000)),
    # FLOAT words (charged 5 limbs, small actual values): the producer class
    ("box-f32-L16", lambda: G.chain(16, "box", n=100000, word="f32")),
    ("boxT-f32-L16", lambda: G.chain(16, "box", step=True, n=100000, word="f32")),
    ("extTS-f32-L8", lambda: G.chain(8, "sph", step=True, ext=True, n=100000, word="f32")),
    # genuine large spans
    ("box-E120-L8", lambda: G.chain(8, "box", mag=("big", 120), n=100000)),
    ("box-E300-L8", lambda: G.chain(8, "box", mag=("big", 300), n=100000)),
    ("box-E500-L8", lambda: G.chain(8, "box", mag=("big", 500), n=100000)),
    ("boxT-E300-L8", lambda: G.chain(8, "box", step=True, mag=("big", 300), n=100000)),
    ("boxT-E500-L8", lambda: G.chain(8, "box", step=True, mag=("big", 500), n=100000)),
    ("sph-E300-L8", lambda: G.chain(8, "sph", mag=("big", 300), n=100000)),
    ("sph-E500-L8", lambda: G.chain(8, "sph", mag=("big", 500), n=40000)),
    ("sphT-E500-L8", lambda: G.chain(8, "sph", step=True, mag=("big", 500), n=40000)),
    ("extTS-E500-L8", lambda: G.chain(8, "sph", step=True, ext=True, mag=("big", 500), n=20000)),
    ("boxS-scale500-L8", lambda: G.chain(8, "box", mag=("scale", 500), n=10000)),
    ("boxS-scale1000-L4", lambda: G.chain(4, "box", mag=("scale", 1000), n=20000)),
    ("sphS-scale1000-L2", lambda: G.chain(2, "sph", mag=("scale", 1000), n=50000)),
    ("extS-scale1000-L4", lambda: G.chain(4, "box", ext=True, mag=("scale", 1000), n=10000)),
    # references
    ("refs-tiny-d1", lambda: G.refs(20, 1000, 5, depth=1)),
    ("refs-tiny-d100", lambda: G.refs(20, 1000, 5, depth=100)),
    ("refs-json64k", lambda: G.refs(4, 100, 10, extra_json=65536)),
    ("refs-json1m", lambda: G.refs(2, 50, 10, extra_json=1 << 20)),
    ("refs-bin256k", lambda: G.refs(4, 100, 10, extra_bin=262144)),
    ("refs-bin1m", lambda: G.refs(2, 50, 10, extra_bin=1 << 20)),
    ("precheck-d100", lambda: G.refs(20, 1000, 5, depth=100, overflow=1300000), "workLimit"),
    ("precheck-d6", lambda: G.refs(20, 1000, 5, depth=6, overflow=1300000), "workLimit"),
    ("refs-tiny-d6", lambda: G.refs(20, 1000, 5, depth=6)),
]

def producer_payload(archive, member):
    with zipfile.ZipFile(os.path.join(FOREST, archive + ".3tz")) as z:
        return z.read(member)


MATRIX += [
    ("refs-w6d", lambda: G.refs(4, 1000, 5, payload=producer_payload("W6d-explicit", "t/0.glb"), half=1e7)),
    ("refs-w6dn", lambda: G.refs(4, 1000, 5, payload=producer_payload("W6d-nomeshopt-explicit", "t/0.glb"), half=1e7)),
    ("refs-w3", lambda: G.refs(4, 1000, 5, payload=producer_payload("W3-pc-deep-explicit", "t/1001.glb"), half=1e7)),
    ("refs-w5", lambda: G.refs(2, 100, 5, payload=producer_payload(
        "W5-vec-perf-explicit", "t/0209ef9d5e56a9a60226ad911f6601828f45fee77d60d73db662852e12b4a3fc.glb"), half=1e7)),
]

MATRIX += [
    ("nodes-flat", lambda: G.refs(1, 2, 5, payload=G.glb_nodes(8000, 1), half=1e3)),
    ("nodes-deep", lambda: G.refs(1, 1, 1, payload=G.glb_nodes(4000, 4), half=1e3)),
    ("nodes-flat-T", lambda: G.refs(1, 4, 5, payload=G.glb_nodes(8000, 1, rotate=False), half=1e3)),
]

V2_C1 = [  # (shape, references): the delta review's C1-structure shapes, plus the linear `shared` class
    ("json-flat", 50), ("json-nums-1", 50), ("json-nums-30", 50), ("json-nums-62", 50), ("json-deep-60", 50),
    ("b3dm-nums-1", 50), ("b3dm-nums-60", 50), ("b3dm-deep-60", 50), ("unselected-60000", 20),
    ("skipped-30000", 20), ("quad-6000-20", 1), ("quadB-5000-16000-40", 1), ("shared-16000-40", 1),
    ("shared-16000-200", 1), ("extbuf-2000", 20), ("extbuf-20000", 20),
]
for _shape, _k in V2_C1:
    def _b(shape=_shape, k=_k):
        member, payload, extra = G.c1_shape(shape)
        return G.refs_named(1, k, 1, member, payload, extra)
    MATRIX.append(("c1-" + _shape, _b))
MATRIX.append(("e2e-quadB-5000-15500-500", lambda: G.refs_named(1, 1, 1, *G.c1_shape("quadB-5000-15500-500"))))

PRODUCER = ["W1-explicit", "W2-explicit", "W3-pc-deep-explicit", "W5-vec-perf-explicit",
            "W5-vec-perf-quantize-explicit", "W5-vec-perf-meshopt-explicit", "W6d-explicit", "W6d-nomeshopt-explicit"]


def main(argv):
    work = argv[0] if argv and not argv[0].startswith("--") else "/tmp/c2wb-work"
    only = None
    if "--only" in argv:
        only = set(argv[argv.index("--only") + 1].split(","))
    os.makedirs(work, exist_ok=True)
    cases = []
    for entry in MATRIX:
        name, build = entry[0], entry[1]
        expect = entry[2] if len(entry) > 2 else "contained"
        if only and name not in only:
            continue
        m = build()
        a = os.path.join(work, name + ".3tz")
        b = os.path.join(work, name + "-Z.3tz")
        G.write_3tz(m, a)
        G.write_3tz(G.twin(m), b)
        cases.append({"name": name, "a": a, "b": b, "expect": expect})
    for name in PRODUCER:
        if only and name not in only:
            continue
        src = os.path.join(FOREST, name + ".3tz")
        with zipfile.ZipFile(src) as z:
            m = {i.filename: z.read(i) for i in z.infolist() if i.filename != G.INDEX}
        b = os.path.join(work, name + "-Z.3tz")
        G.write_3tz(G.twin(m), b)
        cases.append({"name": name, "a": src, "b": b, "expect": "contained"})
    with open(os.path.join(work, "cases.json"), "w") as fh:
        json.dump(cases, fh, indent=1)
    print(len(cases), "cases")


if __name__ == "__main__":
    main(sys.argv[1:])
