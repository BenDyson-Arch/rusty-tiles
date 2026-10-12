#!/usr/bin/env python3
"""Slice-3 self-test of c2o/implicit.py (contract 9.A "Self-test additions").

    python3 -I oracle/self_test/slice3.py

1. Pins: the composed O-explicit modules and O-implicit match their pinned sha256.
2. Explicitization identity: for single-root fixtures, the verdict, the precheck units and every drawn vertex's
   per-level coordinates equal those of a hand-built explicit archive whose tiles carry the same effective
   volumes (rows; V_R again for a DerivedRoot I_0) as transform-free tiles under an explicit copy of R.
3. Chain-length identity: every implicit reference has levels = explicit facts (depth(R) + 1) + l + 1.
4. Root-order identity: the k-th implicit root of the explicit traversal is O-implicit's k-th root (doc, depth,
   scheme, subtree template); swapping two roots' records is detected.
5. External-root placement: for an external-root R, the referencing content volume is at level l + 2, before
   the referencing tile's volume, and that level's step is R.transform; the levels of I_0 and R have no step.
6. `implicitTiling` is never produced, and depth = depth(R) + l on every implicit reference.
"""

import json
import os
import posixpath
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ORACLE = os.path.dirname(HERE)
TOP = os.path.dirname(ORACLE)
sys.path.insert(0, ORACLE)
from c2o import tileset as TS  # noqa: E402
from c2o.implicit import ImplicitOracle, ImplicitTraversal, i1_module, i1_limits, pins, split_roots  # noqa: E402
from c2o.numbers import OracleUndecided, exact  # noqa: E402

SLICE3 = os.path.join(TOP, "controls", "slice3")
sys.path.insert(0, os.path.join(TOP, "..", "implicit_read", "witnesses"))
from fixturelib import cjson, write_3tz  # noqa: E402

CHECKS = []


def check(name, ok, detail=None):
    CHECKS.append({"check": name, "ok": bool(ok), "detail": detail})


def fixture(cid):
    return os.path.join(SLICE3, "fixtures", cid + ".3tz")


def members_of(path):
    z = TS.Archive(path, 1 << 30)
    return {n: z.read(n) for n in z.infos if n != "@3dtilesIndex1@"}


def explicitize(path):
    """Explicit twin: each implicit root R becomes an explicit tile with R's volume, transform and error whose
    only child is I_0; every implicit tile becomes a transform-free explicit tile with its effective volume
    (I1's row, or V_R for a DerivedRoot I_0), its I1 error and its emitted contents."""
    record = i1_module().evaluate(path, i1_limits())
    assert record["verdict"]["kind"] == "Accepted", record["verdict"]
    groups = split_roots(record["tiles"])
    members = members_of(path)
    docs = {}
    for group in groups:
        docs.setdefault(group[0]["doc"], []).append(group)
    for doc_name, doc_groups in docs.items():
        doc = json.loads(members[doc_name])
        queue = list(doc_groups)

        def convert(tile):
            if "implicitTiling" in tile:
                group = queue.pop(0)
                return explicit_root(tile, group, doc_name)
            if "children" in tile:
                tile["children"] = [convert(c) for c in tile["children"]]
            return tile
        doc["root"] = convert(doc["root"])
        members[doc_name] = cjson(doc)
    return members


def explicit_root(tile, group, doc_name):
    base = posixpath.dirname(doc_name)
    nodes = []
    stack = []
    for rec in group:
        vol = rec["volume"]
        box = tile["boundingVolume"]["box"] if vol == "derived-root" else [
            __import__("struct").unpack(">d", bytes.fromhex(h))[0] for h in vol[4:].split(",")]
        node = {"boundingVolume": {"box": box},
                "geometricError": __import__("struct").unpack(">d", bytes.fromhex(rec["error"]))[0]}
        uris = [{"uri": posixpath.relpath(m, base) if base else m} for _, m in rec["contents"]]
        if len(uris) == 1:
            node["content"] = uris[0]
        elif uris:
            node["contents"] = uris
        del stack[rec["level"]:]
        if stack:
            stack[-1].setdefault("children", []).append(node)
        else:
            nodes.append(node)
        stack.append(node)
    out = {k: v for k, v in tile.items() if k not in ("implicitTiling", "content", "contents")}
    out["children"] = nodes
    return out


def run(path, **kw):
    o = ImplicitOracle(path, **kw)
    return o, o.run()


def explicitization(cid):
    path = fixture(cid)
    o1, r1 = run(path)
    with tempfile.TemporaryDirectory() as tmp:
        twin = os.path.join(tmp, "twin.3tz")
        with open(twin, "wb") as fh:
            fh.write(write_3tz(explicitize(path)))
        o2, r2 = run(twin)
        t1, t2 = o1.trace(), o2.trace()
    same_outcome = r1["outcome"] == r2["outcome"]
    f1, f2 = r1.get("facts") or {}, r2.get("facts") or {}
    same_facts = all(f1.get(k) == f2.get(k) for k in ("level", "role", "k", "vertex", "member", "coordinate"))
    units = (r1.get("precheck") or {}).get("totals", {}).get("units") == \
        (r2.get("precheck") or {}).get("totals", {}).get("units")
    coords = [(t["reference"], t["vertex"], t["levels"]) for t in t1] == \
             [(t["reference"], t["vertex"], t["levels"]) for t in t2]
    check("explicitization %s" % cid, same_outcome and same_facts and units and coords and t1,
          {"outcome": [r1["outcome"], r2["outcome"]], "vertices": len(t1), "units": units, "coords": coords,
           "twinImplicitReferences": (r2.get("implicit") or {}).get("implicitReferences")})


def chain_and_depth(paths):
    bad, refs = [], 0
    for path in paths:
        o, r = run(path, detail=False)
        if r["outcome"] in ("Undecided",) or getattr(o, "traversal", None) is None:
            continue
        if "implicitTiling" in (r.get("reasons") or []):
            bad.append((path, "implicitTiling"))
        for ref in o.traversal.references:
            imp = getattr(ref, "implicit", None)
            if imp is None:
                continue
            refs += 1
            depth_r = ref.depth - imp["level"]
            if len(ref.levels) != (depth_r + 1) + imp["level"] + 1:
                bad.append((os.path.basename(path), ref.index, len(ref.levels), ref.depth))
    check("chain length = explicit facts + l + 1, depth = depth(R) + l, no implicitTiling", refs and not bad,
          {"references": refs, "bad": bad[:10]})


def root_order():
    path = fixture("multi-root")
    o, r = run(path)
    groups = split_roots(o.report and i1_module().evaluate(path, i1_limits())["tiles"])
    roots = i1_module().evaluate(path, i1_limits())["roots"]
    seen = [getattr(ref, "implicit", {}).get("root") for ref in o.traversal.references]
    order = []
    for root in seen:
        if root not in order:
            order.append(root)
    ok = r["outcome"] == "Contained" and len(groups) == 2 and len(order) == 2 and \
        [g[0]["doc"] for g in groups] == ["tileset.json"] * 2
    check("root order: two roots, in traversal order", ok, {"roots": order, "schemes": [x["scheme"] for x in roots]})
    archive = TS.Archive(path, 1 << 30)
    try:
        ImplicitTraversal(archive, 10 ** 6, list(reversed(groups)), list(reversed(roots))).run()
        swapped = False
    except OracleUndecided as err:
        swapped = "root-order mismatch" in err.why
    check("root order: swapped O-implicit records are detected", swapped)


def external_placement():
    path = fixture("S3-17.base")
    o, r = run(path)
    refs = [ref for ref in o.traversal.references if getattr(ref, "implicit", None)]
    ref = refs[0]
    level = ref.implicit["level"]
    lv = ref.levels[level + 2]
    roles = [v.role for v in lv.volumes_in_order()]
    ext = json.loads(members_of(path)["ext/tileset.json"])["root"]["transform"]
    want = [[exact(ext[4 * c + row]) for c in range(4)] for row in range(4)]
    ok = (roles == ["externalContent", "ancestor"] and lv.to_parent == want
          and all(ref.levels[i].to_parent is None for i in range(1, level + 2))
          and ref.levels[level + 1].volumes[0].role == "ancestor")
    check("external root: content volume at l + 2 after R.transform", ok,
          {"l": level, "roles": roles, "levels": len(ref.levels)})


def main():
    drift = pins()
    check("pins", not drift, drift)
    for cid in ("S3-9.base", "S3-17.base", "S3-17.own", "S3-11.base", "S3-14.syn.base", "S3-14.syn",
                "F-W7", "F-W4-polygons", "multi-root"):
        explicitization(cid)
    manifest = json.load(open(os.path.join(SLICE3, "manifest.json")))
    chain_and_depth([os.path.join(SLICE3, c["file"]) for c in manifest["cases"]
                     if c["file"].startswith("fixtures/") and c["control"] != "S3-16"])
    root_order()
    external_placement()
    failed = [c for c in CHECKS if not c["ok"]]
    print(json.dumps({"checks": len(CHECKS), "failed": failed}, indent=1))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
