"""Synthetic slice-3 controls (contract 9.C), built with the I1 fixture library.

`implicit_read/witnesses/fixturelib.py` is O's own writer; it shares no code
with `c2o` or O-implicit. Every expectation below is stated by construction:
the drawn point is placed on a chosen face, and a mutant moves exactly one
volume or transform by one ulp.
"""

import json
import math
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURELIB = os.path.normpath(os.path.join(HERE, "..", "..", "..", "implicit_read", "witnesses"))
sys.path.insert(0, FIXTURELIB)
from fixturelib import Implicit, Slot, cell, cjson, glb_tile_point, tileset, write_3tz  # noqa: E402

sys.path.pop(0)


def pred(x):
    return math.nextafter(x, -math.inf)


def succ(x):
    return math.nextafter(x, math.inf)


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def pred32(x):
    bits = struct.unpack("<I", struct.pack("<f", x))[0]
    assert x > 0
    return struct.unpack("<f", struct.pack("<I", bits - 1))[0]


def shortest32(x):
    """Shortest decimal text that reads back to the binary32 value x."""
    for p in range(1, 12):
        text = "%.*g" % (p, x)
        if f32(float(text)) == x:
            return float(text)
    raise AssertionError(x)


def implicit_members(b, root_mut=None, classes_extra=None, error=None):
    root = b.root_tile()
    if root_mut:
        root_mut(root)
    classes = b.schema_classes()
    if classes_extra:
        classes_extra(classes)
    m = b.members()
    m["tileset.json"] = cjson(tileset(root, classes, error=error))
    return m


def translation(t):
    return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, t[0], t[1], t[2], 1]


CASES = []


def case(cid, members, expected, control, role, *, limits=None, i1_limits=None, pair=None, what=None):
    CASES.append({"id": cid, "members": members, "expected": expected, "control": control, "role": role,
                  "limits": limits or {}, "i1Limits": i1_limits or {}, "pair": pair, "what": what})


def outside(volume, depth):
    return {"outcome": "Outside", "facts": {"volume": volume, "depth": depth}}


CONTAINED = {"outcome": "Contained"}


# ---------------------------------------------------------------------------------------------------------------
# Role family (S3-15, S3-17; synthetic S3-4, S3-5, S3-8).
#
# tileset.json: explicit root P (box = V_R + T), content ext/tileset.json with content box CV = V_R + T.
# ext/tileset.json: implicit root R, QUADTREE L=3 A=3, transform T = (8, 0, 0), rows = cells.
# Tiles I_0, (1,1,1), (2,3,3); only (2,3,3) has content: one point at R-frame (32, 28, 4), on the +x face of the
# leaf row, its parent's row, V_0, V_R, and (after T) of CV and P. depth(R) = 1, l = 2, so depth = 3, and the
# chain is [own, ancestor:1, ancestor:2 = I_0, ancestor:3 = V_R, externalContent:4 + ancestor:4 = P].

VR = [16.0, 16.0, 4.0, 16.0, 0.0, 0.0, 0.0, 16.0, 0.0, 0.0, 0.0, 4.0]
T = [8.0, 0.0, 0.0]
P_BOX = [24.0, 16.0, 4.0, 16.0, 0.0, 0.0, 0.0, 16.0, 0.0, 0.0, 0.0, 4.0]
ROLE_TILES = [(0, 0, 0, 0), (1, 1, 1, 0), (2, 3, 3, 0)]
ROLE_POINT = [32.0, 28.0, 4.0]


def shrink_x(box):
    out = list(box)
    out[3] = pred(out[3])
    return out


def role_archive(shrink=None, content_volume=True, t=None):
    b = Implicit("QUADTREE", 3, 3, VR, 50.0, ROLE_TILES,
                 [Slot("c/{level}-{x}-{y}.glb", {(2, 3, 3, 0)}, payload=lambda tile, c: glb_tile_point(ROLE_POINT))],
                 subtree_template="st/{level}-{x}-{y}.subtree")
    b.transform = translation(t or T)
    rows = {"own": (2, 3, 3, 0), "ancestor:1": (1, 1, 1, 0), "ancestor:2": (0, 0, 0, 0)}

    def row(tile):
        r = cell(VR, False, tile[0], tile[1], tile[2], 0)
        return shrink_x(r) if rows.get(shrink) == tile else r
    b.row_box = row

    def root_mut(root):
        if shrink == "ancestor:3":
            root["boundingVolume"]["box"] = shrink_x(VR)
    m = {}
    for k, v in implicit_members(b, root_mut).items():
        m["ext/" + k] = v
    top_content = {"uri": "ext/tileset.json"}
    if content_volume:
        top_content["boundingVolume"] = {"box": shrink_x(P_BOX) if shrink == "externalContent:4" else P_BOX}
    top = {"boundingVolume": {"box": shrink_x(P_BOX) if shrink == "ancestor:4" else P_BOX},
           "content": top_content, "geometricError": 100.0, "refine": "ADD"}
    m["tileset.json"] = cjson(tileset(top))
    return m


def role_family():
    case("S3-17.base", role_archive(), CONTAINED, "S3-17", "base",
         what="external implicit root, l = 2; the point lies on every chain volume's +x face")
    for vol in ("own", "ancestor:1", "ancestor:2", "ancestor:3", "externalContent:4", "ancestor:4"):
        cid = "S3-17." + vol.replace(":", "")
        control = {"ancestor:2": "S3-17/S3-4", "ancestor:3": "S3-17/S3-5",
                   "externalContent:4": "S3-17/S3-15"}.get(vol, "S3-17")
        case(cid, role_archive(shrink=vol), outside(vol, 3), control, "mutant", pair="S3-17.base",
             what="the %s volume's x half-length one ulp smaller" % vol)
    # S3-8 (synthetic): no content volume; R.transform's x translation one ulp toward P's +x face.
    case("S3-8.syn.base", role_archive(content_volume=False), CONTAINED, "S3-8", "base",
         what="role base without the referencing content volume")
    case("S3-8.syn", role_archive(content_volume=False, t=[succ(T[0]), 0.0, 0.0]), outside("ancestor:4", 3),
         "S3-8", "mutant", pair="S3-8.syn.base", what="R.transform x translation + 1 ulp")


# ---------------------------------------------------------------------------------------------------------------
# Small quadtree SQ: V_R = [0,32]x[0,32]x[0,8], L = 2, A = 2, tiles I_0, (1,0,0), (1,1,1), every tile with content
# at its cell centre.

SQ_TILES = [(0, 0, 0, 0), (1, 0, 0, 0), (1, 1, 1, 0)]


def sq(content=None, slots=None, **kw):
    b = Implicit("QUADTREE", 2, 2, VR, 50.0, SQ_TILES,
                 slots or [Slot("c/{level}-{x}-{y}.glb", set(SQ_TILES) if content is None else content)],
                 subtree_template="st/{level}-{x}-{y}.subtree")
    for k, v in kw.items():
        setattr(b, k, v)
    return b


def single(row=None, point=None, rows_box=True, vr=None, comp="FLOAT64", error_row=None):
    vr = vr or VR
    b = Implicit("QUADTREE", 1, 1, vr, 50.0, [(0, 0, 0, 0)],
                 [Slot("c/{level}-{x}-{y}.glb", {(0, 0, 0, 0)},
                       payload=(lambda tile, c: glb_tile_point(point)) if point else None)],
                 subtree_template="st/{level}-{x}-{y}.subtree")
    b.rows_box = rows_box
    b.comp = comp
    if row:
        b.row_box = lambda tile: list(row)
    if error_row is not None:
        b.row_error = lambda tile: error_row
    return b


def derived_family():
    case("S3-9.base", implicit_members(sq()), CONTAINED, "S3-9", "base", what="SQ with rows")
    case("S3-9", implicit_members(sq(rows_box=False)), {"outcome": "NotInspected", "reasons": ["derivedImplicitVolume"]},
         "S3-9", "mutant", pair="S3-9.base", what="TILE_BOUNDING_BOX column removed; content at level 1")
    case("S3-10", implicit_members(sq(rows_box=False, content={(0, 0, 0, 0)})), CONTAINED, "S3-10", "control",
         pair="S3-9", what="rowless; only I_0 (DerivedRoot) has content, so no chain holds a derived tile")
    point = [32.0, 16.0, 4.0]
    case("S3-11.base", implicit_members(single(rows_box=False, point=point)), CONTAINED, "S3-11", "base",
         what="rowless single tile (DerivedRoot); the point is on V_R's +x face")
    case("S3-11", implicit_members(single(rows_box=False, point=point),
                                   root_mut=lambda r: r["boundingVolume"].__setitem__("box", shrink_x(VR))),
         outside("own", 0), "S3-11", "mutant", pair="S3-11.base",
         what="V_R x half-length one ulp smaller (V_0 = V_R, tested first: own)")


def metadata_family():
    s = Slot("c/{level}-{x}-{y}.glb", set(SQ_TILES), header={"metadata": {"class": "hmeta", "properties": {"g": 0}}})

    def hmeta(classes):
        classes["hmeta"] = {"properties": {"g": {"componentType": "UINT32", "type": "SCALAR"}}}
    case("S3-12.header", implicit_members(sq(slots=[s]), classes_extra=hmeta),
         {"outcome": "NotInspected", "reasons": ["tileMetadata"]}, "S3-12", "mutant", pair="S3-9.base",
         what="template header metadata (I6, F step 7)")
    case("S3-12.subtree", implicit_members(sq(flag_column=True)), CONTAINED, "S3-12", "control", pair="S3-9.base",
         what="an extra opaque BOOLEAN tileMetadata column in the subtree (I6: not tileMetadata)")


def rotated_family():
    aligned = [16.0, 16.0, 4.0, 4.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 4.0]
    rotated = [16.0, 16.0, 4.0, 2.0, 2.0, 0.0, -2.0, 2.0, 0.0, 0.0, 0.0, 4.0]
    case("S3-13.base", implicit_members(single(row=aligned)), CONTAINED, "S3-13", "base",
         what="axis-aligned I_0 row")
    case("S3-13", implicit_members(single(row=rotated)), {"outcome": "NotInspected", "reasons": ["rotatedBox"]},
         "S3-13", "mutant", pair="S3-13.base", what="I_0 row half-axes rotated 45 degrees about z")


def float32_family():
    """S3-14: FLOAT32 rows, tight at a leaf face (binary32), mutant pred in binary32.

    The leaf half-length h is chosen so that the shortest decimal of h reads below h: a reader that rounds
    FLOAT32 rows through their shortest decimal text would see the base escape too."""
    k = next(k for k in range(2, 4096, 2)
             if shortest32(f32(1.0 + k * 2.0 ** -23)) < 1.0 + k * 2.0 ** -23
             and f32(2.0 + k * 2.0 ** -23) == 2.0 + k * 2.0 ** -23)
    h = 1.0 + k * 2.0 ** -23
    face = 1.0 + h
    vr = [2.0, 2.0, 2.0, 2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0]

    def build(hx):
        b = Implicit("OCTREE", 2, 2, vr, 8.0, [(0, 0, 0, 0), (1, 0, 0, 0)],
                     [Slot("c/{level}-{x}-{y}-{z}.glb", {(1, 0, 0, 0)},
                           payload=lambda tile, c: glb_tile_point([face, 1.0, 1.0]))],
                     subtree_template="st/{level}-{x}-{y}-{z}.subtree")
        b.comp = "FLOAT32"
        b.row_box = lambda tile: (list(vr) if tile[0] == 0 else [1.0, 1.0, 1.0, hx, 0.0, 0.0, 0.0, 1.0, 0.0,
                                                                 0.0, 0.0, 1.0])
        return b
    case("S3-14.syn.base", implicit_members(build(h)), CONTAINED, "S3-14", "base",
         what="FLOAT32 rows; leaf x face 1 + h = %r is tight to the point (k = %d)" % (face, k))
    case("S3-14.syn", implicit_members(build(pred32(h))), outside("own", 1), "S3-14", "mutant",
         pair="S3-14.syn.base", what="leaf x half-length pred in binary32")


def depth_family():
    """S3-16: a chain of hierarchy_depth + 2 levels (hierarchy_depth lowered to 12)."""
    depth = 12
    box = [4096.0, 4096.0, 1.0, 4096.0, 0.0, 0.0, 0.0, 4096.0, 0.0, 0.0, 0.0, 1.0]
    tiles = [(level, 0, 0, 0) for level in range(depth + 1)]
    b = Implicit("QUADTREE", 4, depth + 1, box, 4096.0, tiles,
                 [Slot("c/{level}-{x}-{y}.glb", {(depth, 0, 0, 0)})], subtree_template="st/{level}-{x}-{y}.subtree")
    m = implicit_members(b)
    case("S3-16", m, CONTAINED, "S3-16", "control", i1_limits={"hierarchy_depth": depth},
         what="content at level 12 under a tileset root: depth 12, chain 14 levels = hierarchy_depth + 2")
    case("S3-16.depth-1", m, {"outcome": "ResourceLimit", "limit": "hierarchy_depth"}, "S3-16", "mutant",
         i1_limits={"hierarchy_depth": depth - 1}, pair="S3-16", what="hierarchy_depth 11: refused in C1 (I1)")


def precedence_family():
    """S3-20: an explicit reference first pushes the precheck over the cap; a later reference goes through a
    rowless subtree. Exclusions override the precheck reason."""
    def build(rows_box):
        b = sq(rows_box=rows_box)
        b.slots[0].template = "r/c/{level}-{x}-{y}.glb"
        b.slots[0].member_template = "r/c/{level}-{x}-{y}.glb"
        b.subtree_template = "r/st/{level}-{x}-{y}.subtree"
        r = b.root_tile()
        r["transform"] = translation([0.0, 0.0, 16.0])
        m = b.members()
        leaf = {"boundingVolume": {"box": VR}, "content": {"uri": "a.glb"}, "geometricError": 1.0}
        top = {"boundingVolume": {"box": [16.0, 16.0, 12.0, 16.0, 0.0, 0.0, 0.0, 16.0, 0.0, 0.0, 0.0, 12.0]},
               "children": [leaf, r], "geometricError": 100.0, "refine": "ADD"}
        m["a.glb"] = glb_tile_point([16.0, 16.0, 4.0])
        m["tileset.json"] = cjson(tileset(top, b.schema_classes()))
        return m
    cap = {"containmentWorkUnits": 1}
    case("S3-20.base", build(True), {"outcome": "NotInspected", "reasons": ["workLimit"]}, "S3-20", "base",
         limits=cap, what="explicit a.glb first, then an implicit root with rows; cap 1 unit")
    case("S3-20", build(False), {"outcome": "NotInspected", "reasons": ["derivedImplicitVolume"]}, "S3-20",
         "mutant", limits=cap, pair="S3-20.base",
         what="same, the implicit root rowless: derivedImplicitVolume, not workLimit")


def repeated_uri_base():
    """R9 shape at K = 3: three identical slots, so each tile emits the same member three times."""
    slots = [Slot("c/{level}-{x}-{y}.glb", set(SQ_TILES)) for _ in range(3)]
    return implicit_members(sq(slots=slots))


def multi_root():
    """Two implicit roots under one explicit tile (root-order identity, self-test 3)."""
    a = sq()
    a.slots[0].template = a.slots[0].member_template = "a/c/{level}-{x}-{y}.glb"
    a.subtree_template = "a/st/{level}-{x}-{y}.subtree"
    b = Implicit("OCTREE", 2, 2, [16.0, 16.0, 12.0, 16.0, 0.0, 0.0, 0.0, 16.0, 0.0, 0.0, 0.0, 4.0], 40.0,
                 [(0, 0, 0, 0), (1, 1, 0, 1)], [Slot("b/c/{level}-{x}-{y}-{z}.glb", {(0, 0, 0, 0), (1, 1, 0, 1)})],
                 subtree_template="b/st/{level}-{x}-{y}-{z}.subtree")
    m = a.members()
    m.update(b.members())
    top = {"boundingVolume": {"box": [16.0, 16.0, 8.0, 16.0, 0.0, 0.0, 0.0, 16.0, 0.0, 0.0, 0.0, 8.0]},
           "children": [a.root_tile(), b.root_tile()], "geometricError": 100.0, "refine": "ADD"}
    classes = a.schema_classes()
    m["tileset.json"] = cjson(tileset(top, classes))
    return m


def build():
    CASES.clear()
    role_family()
    derived_family()
    metadata_family()
    rotated_family()
    float32_family()
    depth_family()
    precedence_family()
    case("repeated-uri.base", repeated_uri_base(), CONTAINED, "9.D repeated URI", "base",
         what="R9 shape, K = 3 identical slots; at-cap pair from O's total")
    case("multi-root", multi_root(), CONTAINED, "self-test 3", "control",
         what="two implicit roots (QUADTREE then OCTREE) under one explicit tile")
    return CASES


def archive(members):
    return write_3tz(members)


if __name__ == "__main__":
    print(json.dumps([c["id"] for c in build()]))
