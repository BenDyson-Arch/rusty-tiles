#!/usr/bin/env python3
"""C2 slice-3 controls (contract 9.C, 9.D) and the S3-19 retained fixtures.

    python3 -I controls/slice3/generate.py [--forest DIR] [--pre-amendment DIR] [--evidence DIR] [--check]

- Synthetic controls (`synthetic.py`) are always built; they are small and committed under `fixtures/`.
- `--forest DIR`: R's forest producer archives (`W1.3tz`, `W6.3tz`, ...; pinned in `forest.INPUTS`). Derived
  controls up to SMALL bytes are committed under `fixtures/`; larger ones are written to `--evidence`
  (default /home/bend/.cache/c2s3-evidence/slice3) and listed by sha256 only.
- `--pre-amendment DIR`: the I1 controls before amendment 6 (`fixtures/`, `large/`, as generated at 92e8c04).
  Each S3-19 fixture is checked against its old sha256 in `implicit_read/controls/amendment6.json`.
- `--check`: rebuild in memory and compare every hash with `manifest.json`.

Without `--forest` / `--pre-amendment`, the forest and S3-19 rows of the existing manifest are kept.
Expectations are stated by construction; `check.py` runs the oracle and records its results.
"""

import hashlib
import json
import os
import re
import sys
from fractions import Fraction

HERE = os.path.dirname(os.path.abspath(__file__))
TOP = os.path.dirname(os.path.dirname(HERE))  # c2_decoded_containment
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(TOP, "oracle"))
import forest as FO  # noqa: E402
import synthetic as SY  # noqa: E402

SMALL = 120_000
I1_ROOT = os.path.normpath(os.path.join(TOP, "..", "implicit_read"))
DEFAULT_EVIDENCE = "/home/bend/.cache/c2s3-evidence/slice3"
CONTRACT_SHA256 = "8bc534290c5a90ace963e345336dea1e04c45fafb3c869672ee4af1ba03331d6"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def outside(volume, depth):
    return {"outcome": "Outside", "facts": {"volume": volume, "depth": depth}}


CONTAINED = {"outcome": "Contained"}


class Built:
    def __init__(self):
        self.rows = []
        self.blobs = {}

    def add(self, cid, data, expected, control, role, *, limits=None, i1_limits=None, pair=None, what=None,
            source=None, record=None):
        self.rows.append({"id": cid, "sha256": sha(data), "bytes": len(data), "expected": expected,
                          "control": control, "role": role, "limits": limits or {}, "i1Limits": i1_limits or {},
                          "pair": pair, "what": what, "source": source, "construction": record})
        self.blobs[cid] = data


# ---------------------------------------------------------------------------------------------------------------
# Forest-derived controls

def load_input(directory, wid):
    path = os.path.join(directory, wid + ".3tz")
    with open(path, "rb") as fh:
        data = fh.read()
    if sha(data) != FO.INPUTS[wid]:
        raise SystemExit("%s: sha256 %s != pinned %s" % (path, sha(data), FO.INPUTS[wid]))
    return path, data


def zero_slack_face(tile):
    """First (axis, sign) whose own-content extreme lies exactly on the tile's effective-volume face."""
    v = tile["volume"]
    for axis in range(3):
        for sign in (1, -1):
            face = v.c[axis] + sign * v.h[axis]
            ext = tile["own"][axis][1] if sign > 0 else tile["own"][axis][0]
            if ext == face:
                return axis, sign
    return None


def shrink_row(row, axis):
    out = list(row)
    out[3 + 4 * axis] = FO.pred(out[3 + 4 * axis])
    return out


def walk_order(tiles):
    return sorted(tiles, key=lambda k: min(tiles[k]["refs"]))


def root_box_text_edit(text, axis, new_value):
    return FO.replace_json_number(text, r'"root":\s*\{\s*"boundingVolume":\s*\{\s*"box":\s*\[([^\]]*)\]',
                                  3 + 4 * axis, new_value)


def forest_controls(built, directory, ImplicitOracle):
    # ---- W7: single tile, zero-slack mesh (S3-4, S3-5 at l = 0) and W6 (S3-1, S3-4, S3-5)
    for wid in ("W7", "W6"):
        path, data = load_input(directory, wid)
        zipf = FO.Zip(data)
        _, report, tiles = FO.analyse(path, ImplicitOracle)
        i0 = next(k for k in tiles if k[1] == 0)
        axis, sign = zero_slack_face(tiles[i0])
        v0 = FO.row_values(tiles[i0]["volume"])
        built.add("F-%s" % wid, data, CONTAINED, "witness", "base", source=wid,
                  what="producer archive, unchanged")
        built.add("S3-4.%s" % wid, FO.apply_rows(zipf, [(v0, shrink_row(v0, axis))]),
                  outside("own", tiles[i0]["depth"]), "S3-4", "mutant", pair="F-%s" % wid, source=wid,
                  what="V_0 (I_0 row) half-length on axis %d one ulp smaller; V_R untouched" % axis,
                  record={"tile": FO.describe(i0), "axis": axis, "sign": sign,
                          "note": "I_0's own content attains the face, so the first Outside is own (l = 0)"})
        text = zipf.read("tileset.json").decode("utf-8")
        vr_h = float(Fraction(tiles[i0]["volume"].h[axis]))
        new_text = root_box_text_edit(text, axis, FO.pred(vr_h))
        built.add("S3-5.%s" % wid, zipf.replace({"tileset.json": new_text.encode("utf-8")}),
                  outside("ancestor:1", tiles[i0]["depth"]), "S3-5", "mutant", pair="F-%s" % wid, source=wid,
                  what="V_R half-length on axis %d one ulp smaller in tileset.json; V_0 untouched" % axis,
                  record={"axis": axis, "VR": "V_0 = V_R in this witness"})
        if wid == "W6":
            kids = FO.children_of(tiles)
            leaves = [k for k in walk_order(tiles) if not kids[k] and tiles[k]["own"] is not None]
            leaf = max(leaves, key=lambda k: k[1])
            face = zero_slack_face(tiles[leaf])
            row = FO.row_values(tiles[leaf]["volume"])
            built.add("S3-1.W6", FO.apply_rows(zipf, [(row, shrink_row(row, face[0]))]),
                      outside("own", tiles[leaf]["depth"]), "S3-1", "mutant", pair="F-W6", source=wid,
                      what="zero-slack leaf row half-length on axis %d one ulp smaller" % face[0],
                      record={"tile": FO.describe(leaf), "axis": face[0], "sign": face[1]})

    # ---- whole-subtree-tight bases (S3-2 leaf, S3-3 ancestor, S3-6 delta on W1)
    for wid in ("W4-polygons", "W1", "W5-vec-perf-quantize"):
        path, data = load_input(directory, wid)
        zipf = FO.Zip(data)
        _, report, tiles = FO.analyse(path, ImplicitOracle)
        kids = FO.children_of(tiles)
        order = walk_order(tiles)
        leaves = [k for k in order if not kids[k] and tiles[k]["own"] is not None]
        leaves.sort(key=lambda k: (-k[1], order.index(k)))
        x, axis, sign = _leaf_face(zipf, tiles, kids, leaves, wid == "W1")
        changes = FO.tight_subtree(tiles, kids, x, axis, sign)
        old, new = changes[x]
        base = FO.apply_rows(zipf, list(changes.values()))
        mut = FO.apply_rows(FO.Zip(base), [(new, shrink_row(new, axis))])
        e = FO.subtree_extreme(tiles, kids, x, axis, sign)
        rec = {"X": FO.describe(x), "axis": axis, "sign": sign, "e": str(e), "eApprox": float(e),
               "c'": new[axis], "h'": new[3 + 4 * axis], "rows": len(changes)}
        if wid != "W1":
            built.add("F-%s" % wid, data, CONTAINED, "witness", "base", source=wid, what="producer archive, unchanged")
        built.add("S3-2.%s.base" % wid, base, CONTAINED, "S3-2", "base", source=wid, record=rec,
                  what="whole-subtree-tight base at leaf %s, axis %d sign %+d" % (FO.describe(x), axis, sign))
        built.add("S3-2.%s" % wid, mut, outside("own", tiles[x]["depth"]), "S3-2", "mutant",
                  pair="S3-2.%s.base" % wid, source=wid, record=rec, what="pred(h') on X")
        if wid == "W5-vec-perf-quantize":
            built.add("S3-7.%s" % wid, _swizzle(zipf, tiles, order), {"outcome": "Outside"}, "S3-7", "mutant",
                      pair="F-%s" % wid, source=wid,
                      what="first implicit payload with a root translation: (d0, d1, d2) written instead of "
                           "(d0, d2, -d1)")
        if wid == "W1":
            built.add("F-W1", data, CONTAINED, "witness", "base", source=wid, what="producer archive, unchanged")
            member, text, gi, new_t = _delta_edit(FO.Zip(base), tiles, x, axis, sign)
            built.add("S3-6.W1", FO.Zip(base).replace({member: new_t}), outside("own", tiles[x]["depth"]),
                      "S3-6", "mutant", pair="S3-2.W1.base", source=wid,
                      record=dict(rec, member=member, translationIndex=gi),
                      what="X's payload root translation moved one ulp toward the tight face (JSON text edit)")
            _ancestor_control(built, zipf, tiles, kids, order, wid)


def _translation_index(axis, sign):
    """glTF translation component and direction that moves the tile-frame point by +sign on `axis`:
    tile = U(gltf) = (gx, -gz, gy)."""
    return {0: (0, sign), 1: (2, -sign), 2: (1, sign)}[axis]


def escapes(row, axis, sign, e):
    """True when pred(h') on the tightened row leaves e outside. c' is fixed by the rule, so when the opposite
    face constraint binds h', pred(h') can still reach e; such faces are skipped (contract 9.C reading)."""
    new, c2, h2 = FO.tighten(row, axis, sign, e)
    return (Fraction(c2) + sign * Fraction(FO.pred(h2)) - e) * sign < 0


def _leaf_face(zipf, tiles, kids, leaves, need_delta):
    for x in leaves:
        faces = [_delta_face(zipf, tiles, x)] if need_delta else [(a, s) for a in range(3) for s in (1, -1)]
        for face in faces:
            if face is None:
                continue
            axis, sign = face
            e = FO.subtree_extreme(tiles, kids, x, axis, sign)
            if escapes(FO.row_values(tiles[x]["volume"]), axis, sign, e):
                return x, axis, sign
    raise RuntimeError("no leaf face where pred(h') escapes")


def _delta_face(zipf, tiles, x):
    """Choose the face of X for S3-6: the face whose tightening slack is smallest relative to one ulp of the
    translation component that moves along it."""
    best = None
    member = tiles[x]["member"]
    text, _ = FO.glb_json(zipf.read(member))
    node = json.loads(text)["nodes"][0]
    t = node["translation"]
    for axis in range(3):
        for sign in (1, -1):
            e = FO.Fraction(tiles[x]["own"][axis][1] if sign > 0 else tiles[x]["own"][axis][0])
            row = FO.row_values(tiles[x]["volume"])
            new, c2, h2 = FO.tighten(row, axis, sign, e)
            face = Fraction(c2) + sign * Fraction(h2)
            slack = (face - e) * sign
            gi, d = _translation_index(axis, sign)
            moved = Fraction(FO.succ(t[gi]) if d > 0 else FO.pred(t[gi])) - Fraction(t[gi])
            margin = abs(moved) - slack
            if margin > 0 and escapes(row, axis, sign, e) and (best is None or margin > best[0]):
                best = (margin, axis, sign)
    if best is None:
        return None
    return best[1], best[2]


def _delta_edit(zipf, tiles, x, axis, sign):
    member = tiles[x]["member"]
    data = zipf.read(member)
    text, _ = FO.glb_json(data)
    t = json.loads(text)["nodes"][0]["translation"]
    gi, d = _translation_index(axis, sign)
    value = FO.succ(t[gi]) if d > 0 else FO.pred(t[gi])
    new_text = FO.replace_json_number(text, r'"translation":\s*\[([^\]]*)\]', gi, value)
    return member, new_text, gi, FO.glb_with_json(data, new_text)


def _swizzle(zipf, tiles, order):
    for key in order:
        member = tiles[key]["member"]
        data = zipf.read(member)
        text, _ = FO.glb_json(data)
        node = json.loads(text)["nodes"][0]
        if "translation" in node:
            s0, s1, s2 = node["translation"]
            body = ",".join(FO.number_text(v) for v in (s0, -s2, s1))
            m = re.search(r'"translation":\s*\[([^\]]*)\]', text)
            new_text = text[:m.start(1)] + body + text[m.end(1):]
            return zipf.replace({member: FO.glb_with_json(data, new_text)})
    raise RuntimeError("no payload with a translation")


def _ancestor_control(built, zipf, tiles, kids, order, wid):
    """S3-3: whole-subtree-tight base at an implicit ancestor X whose own content stays inside pred(h'), so the
    first Outside is a descendant's content at ancestor:k."""
    for x in order:
        if not kids[x] or x[1] == 0:
            continue
        for axis in range(3):
            for sign in (1, -1):
                changes = FO.tight_subtree(tiles, kids, x, axis, sign)
                old, new = changes[x]
                hp = FO.pred(new[3 + 4 * axis])
                face = Fraction(new[axis]) + sign * Fraction(hp)
                own = tiles[x]["own"]
                if own is not None and (own[axis][1] > face if sign > 0 else own[axis][0] < face):
                    continue
                first = None
                for k in order:
                    if k == x or not _below(k, x):
                        continue
                    o = tiles[k]["own"]
                    if o is not None and (o[axis][1] > face if sign > 0 else o[axis][0] < face):
                        first = k
                        break
                if first is None:
                    continue
                base = FO.apply_rows(zipf, list(changes.values()))
                mut = FO.apply_rows(FO.Zip(base), [(new, shrink_row(new, axis))])
                rec = {"X": FO.describe(x), "axis": axis, "sign": sign, "firstOutside": FO.describe(first),
                       "rows": len(changes)}
                k = first[1] - x[1]
                built.add("S3-3.%s.base" % wid, base, CONTAINED, "S3-3", "base", source=wid, record=rec,
                          what="whole-subtree-tight base at ancestor %s" % FO.describe(x))
                built.add("S3-3.%s" % wid, mut, outside("ancestor:%d" % k, tiles[first]["depth"]), "S3-3",
                          "mutant", pair="S3-3.%s.base" % wid, source=wid, record=rec,
                          what="pred(h') on X; X's own content stays inside, %s's does not" % FO.describe(first))
                return
    raise RuntimeError("no ancestor X fits S3-3 in %s" % wid)


def _below(k, x):
    if k[0] != x[0] or k[1] <= x[1]:
        return False
    d = k[1] - x[1]
    return (k[2] >> d, k[3] >> d, k[4] >> d) == x[2:]


def repeated_uri_pair(built):
    """Contract 9.D repeated-URI pair: the cap is O's precheck total (units are charged per reference)."""
    import tempfile
    from c2o.implicit import ImplicitOracle
    data = built.blobs["repeated-uri.base"]
    with tempfile.NamedTemporaryFile(suffix=".3tz") as fh:
        fh.write(data)
        fh.flush()
        report = ImplicitOracle(fh.name, precheck_only=True).run()
    units = report["precheck"]["totals"]["units"]
    refs = report["references"]
    built.add("repeated-uri.at-cap", data, CONTAINED, "9.D repeated URI", "pair",
              limits={"containmentWorkUnits": units}, pair="repeated-uri.base",
              what="containmentWorkUnits = O's total %d over %d references (3 per tile)" % (units, refs))
    built.add("repeated-uri.cap-1", data, {"outcome": "NotInspected", "reasons": ["workLimit"]},
              "9.D repeated URI", "pair", limits={"containmentWorkUnits": units - 1}, pair="repeated-uri.at-cap",
              what="one unit under O's total")


# ---------------------------------------------------------------------------------------------------------------
# S3-19: pre-amendment (unrotated) I1 fixtures retained as C2 controls

def s3_19(built, pre_dir):
    """Every amendment-6 row that O-implicit accepts at its manifest limits, that production does not refuse in
    C1, whose content-bearing tiles all have a row (or are DerivedRoot) and that has no template-header metadata.
    Contract 9.C S3-19: Outside(own), as C2 controls only (the unrotated point leaves its own row)."""
    from c2o.implicit import i1_module
    i1_defaults = dict(i1_module().DEFAULT_LIMITS)  # I1 controls run at O-implicit's defaults + manifest limits
    with open(os.path.join(I1_ROOT, "controls", "amendment6.json")) as fh:
        amend = json.load(fh)
    i1_manifest = {c["id"]: c for c in json.load(open(os.path.join(I1_ROOT, "controls", "manifest.json")))["cases"]}
    for row in amend["rows"]:
        if row["i1Verdict"] != "Accepted" or i1_manifest[row["id"]].get("production"):
            continue  # not accepted, or refused by production's C1 (C-10, R1b.full): not C2 controls
        path = os.path.join(pre_dir, row["file"])
        with open(path, "rb") as fh:
            data = fh.read()
        if sha(data) != row["oldSha256"]:
            raise SystemExit("%s: not the pre-amendment fixture" % path)
        record = i1_module().evaluate(path, dict(i1_defaults, **row["limits"]))
        content_tiles = [t for t in record["tiles"] if t["contents"]]
        if any(not (t["volume"].startswith("row:") or t["volume"] == "derived-root") for t in content_tiles):
            continue  # a derived tile bears content: derivedImplicitVolume (S3-18 lists it)
        if b'"metadata"' in FO.Zip(data).read("tileset.json"):
            continue  # template-header metadata: tileMetadata (S3-18 lists it)
        built.add("S3-19.%s" % row["id"], data, {"outcome": "Outside", "facts": {"volume": "own"}}, "S3-19", "control", i1_limits=dict(i1_defaults, **row["limits"]),
                  source="implicit_read pre-amendment-6 %s" % row["file"],
                  what="unrotated implicit payloads (I1 control before amendment 6), as a C2 control only")


# ---------------------------------------------------------------------------------------------------------------

def main(argv):
    def opt(name, default=None):
        return argv[argv.index(name) + 1] if name in argv else default
    forest_dir = opt("--forest")
    pre_dir = opt("--pre-amendment")
    evidence = opt("--evidence", DEFAULT_EVIDENCE)
    check = "--check" in argv
    built = Built()
    for c in SY.build():
        built.add(c["id"], SY.archive(c["members"]), c["expected"], c["control"], c["role"], limits=c["limits"],
                  i1_limits=c["i1Limits"], pair=c["pair"], what=c["what"], source="synthetic")
    repeated_uri_pair(built)
    if forest_dir:
        from c2o.implicit import ImplicitOracle
        forest_controls(built, forest_dir, ImplicitOracle)
    if pre_dir:
        s3_19(built, pre_dir)
    mpath = os.path.join(HERE, "manifest.json")
    old = json.load(open(mpath)) if os.path.exists(mpath) else {"cases": []}
    have = {r["id"] for r in built.rows}
    for r in old["cases"]:
        keep = (r.get("source") or "").startswith(("W", "implicit_read")) and r["id"] not in have
        if keep and ((not forest_dir and r["control"] != "S3-19") or (not pre_dir and r["control"] == "S3-19")):
            built.rows.append({k: v for k, v in r.items() if k != "file"} | {"file": r["file"]})
    for r in built.rows:
        if "file" in r:
            continue
        small = r["bytes"] <= SMALL
        r["file"] = ("fixtures/%s.3tz" % r["id"]) if small else ("evidence:%s.3tz" % r["id"])
    if check:
        want = {r["id"]: r["sha256"] for r in old["cases"]}
        bad = [r["id"] for r in built.rows if r["id"] in want and want[r["id"]] != r["sha256"]]
        print(json.dumps({"checked": len(built.rows), "mismatch": bad}))
        return 1 if bad else 0
    os.makedirs(os.path.join(HERE, "fixtures"), exist_ok=True)
    for r in built.rows:
        if r["id"] not in built.blobs:
            continue
        data = built.blobs[r["id"]]
        if r["file"].startswith("fixtures/"):
            target = os.path.join(HERE, r["file"])
        else:
            os.makedirs(evidence, exist_ok=True)
            target = os.path.join(evidence, r["file"][len("evidence:"):])
        with open(target, "wb") as fh:
            fh.write(data)
    manifest = {"generator": "controls/slice3/generate.py", "contract": CONTRACT_SHA256,
                "forestInputs": FO.INPUTS, "evidenceDir": DEFAULT_EVIDENCE, "small": SMALL,
                "cases": sorted(built.rows, key=lambda r: r["id"])}
    with open(mpath, "w") as fh:
        json.dump(manifest, fh, indent=1, sort_keys=True)
        fh.write("\n")
    print(json.dumps({"cases": len(manifest["cases"]),
                      "committed": sum(r["file"].startswith("fixtures/") for r in manifest["cases"])}))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
