#!/usr/bin/env python3
"""Materialize every I1 witness, control and resource case the oracle owns (contract v3.1, acceptance plan B-E).

    python3 -I generate.py            # small fixtures -> ../controls/fixtures, manifest -> ../controls/manifest.json
    python3 -I generate.py --large    # also the large ones -> ../controls/large (gitignored)
    python3 -I generate.py --check    # regenerate in memory and compare every hash with the manifest

Stdlib only; byte-reproducible; never imports the oracle.  Expected verdicts come from the contract text, not from
running anything.  Inputs outside this directory are pinned by sha256 (see INPUTS).
"""
import base64
import hashlib
import json
import math
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from fixturelib import (Implicit, Slot, SubIR, cell, cjson, glb_tile_point, morton, offset, pad,  # noqa: E402
                        subtree_binary, tileset, write_3tz)

ROOT = os.path.dirname(HERE)
REPO = os.path.abspath(os.path.join(ROOT, "..", "..", ".."))
CONTRACT_SHA256 = "9674e60b959eea0a4a985b6beec7a278ec79c08579f6c19e58f5a070aee03496"
INPUTS = {
    "bench/architecture_audit/implicit_rewrite/a2_semantics/octree.json":
        "ec78244b10efb9ca564e037f51473b334ed4dbf4e4ece36feb918798e7fcee8b",
    "bench/architecture_audit/implicit_rewrite/a2_semantics/quadtree.json":
        "23d26bc05934a203da1505b07cf31a1fd896136bcacc482dd7eaa0dbd9665637",
}

CASES = []


def case(cid, members, expected, *, limits=None, pair=None, ref=None, note=None, large=False, question=None,
         production=None, facts=None, file=None):
    """expected: Accepted | InvalidInput | Unsupported | ResourceLimit:<limit>.
    facts: ("equal"|"differ", other case id) for Accepted controls.  production: the production expectation when
    it differs from the oracle's domain (C1 payload integrity is excluded from the oracle)."""
    CASES.append({"id": cid, "members": members, "expected": expected, "limits": limits or {}, "pair": pair,
                  "ref": ref, "note": note, "large": large, "question": question, "production": production,
                  "facts": facts, "file": file})


# ---------------------------------------------------------------------------------------------------------------
# Bases

BQ_BOX = [16.0, 8.0, 2.0, 16.0, 0.0, 0.0, 0.0, 8.0, 0.0, 0.0, 0.0, 2.0]
BQ_TILES = [(0, 0, 0, 0), (1, 0, 0, 0), (1, 1, 0, 0), (1, 0, 1, 0), (1, 1, 1, 0),
            (2, 0, 0, 0), (2, 1, 0, 0), (2, 1, 1, 0), (2, 3, 0, 0), (2, 2, 3, 0), (2, 3, 3, 0),
            (3, 2, 0, 0), (3, 3, 1, 0), (3, 7, 7, 0), (3, 6, 6, 0), (3, 1, 1, 0),
            (4, 4, 0, 0), (4, 5, 1, 0), (4, 15, 15, 0), (4, 14, 15, 0), (4, 3, 2, 0), (4, 2, 3, 0),
            (5, 9, 1, 0), (5, 29, 30, 0)]
BQ_EMPTY = {(1, 0, 1, 0), (2, 1, 1, 0), (4, 14, 15, 0)}
BQ_TEMPLATE = "content/{level}/{x}/{y}.glb"


def bq_row_box(t):
    c = cell(BQ_BOX, False, t[0], t[1], t[2], 0)
    c[11] = 2.0 - t[0] / 8.0          # the z half-extent narrows by level (quadtree keeps axis 2)
    return c


def bq_row_error(t):
    if t[0] == 0:
        return 1000.0
    k = morton([t[1], t[2]])
    return 1024.0 / (1 << t[0]) * (1.0 - k / (2.0 * 4 ** t[0]))


def BQ(**kw):
    tiles = kw.pop("tiles", BQ_TILES)
    content = kw.pop("content", None)
    if content is None:
        content = {t for t in tiles if t not in BQ_EMPTY}
    b = Implicit("QUADTREE", 3, kw.pop("A", 6), BQ_BOX, 1024.0, tiles,
                 [Slot(kw.pop("template", BQ_TEMPLATE), content, member_template=kw.pop("member_template", None))])
    b.row_box, b.row_error = bq_row_box, bq_row_error
    b.flag_column = True
    for k, v in kw.items():
        setattr(b, k, v)
    return b


BO_BOX = [100.0, 200.0, 300.0, 8.0, 0.0, 0.0, 0.0, 8.0, 0.0, 0.0, 0.0, 8.0]
BO_TRANSFORM = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1000, 2000, 0, 1]
BO_TILES = [(0, 0, 0, 0), (1, 0, 0, 0), (1, 1, 1, 0), (1, 1, 0, 1), (1, 0, 1, 1),
            (2, 3, 2, 0), (2, 3, 3, 1), (2, 0, 1, 0)]
BO_A = {t for t in BO_TILES if t != (1, 1, 0, 1)}
BO_B = {(0, 0, 0, 0), (2, 3, 2, 0), (2, 3, 3, 1)}


def bo_row_error(t):
    if t[0] == 0:
        return 60.0
    k = morton([t[1], t[2], t[3]])
    return 64.0 / (1 << t[0]) * (1.0 - k / (2.0 * 8 ** t[0]))


def BO(slot_a=None, slot_b=True, extra_slots=(), **kw):
    slots = [slot_a or Slot("o/a/{level}-{x}-{y}-{z}.glb", BO_A, metadata_class="cmeta")]
    if slot_b:
        slots.append(Slot("o/b/{level}-{x}-{y}-{z}.glb", BO_B, metadata_class="cmeta"))
    slots.extend(extra_slots)
    b = Implicit("OCTREE", 2, 3, BO_BOX, 64.0, kw.pop("tiles", BO_TILES), slots,
                 subtree_template="st/{level}.{x}.{y}.{z}.subtree", transform=BO_TRANSFORM)
    b.row_error = bo_row_error
    for k, v in kw.items():
        setattr(b, k, v)
    return b


def implicit_archive(b, classes_extra=None, error=None, mutate_root=None, mutate_doc=None, extra_members=None):
    root = b.root_tile()
    if mutate_root:
        mutate_root(root)
    classes = b.schema_classes()
    if classes_extra:
        classes_extra(classes)
    doc = tileset(root, classes, error=error)
    if mutate_doc:
        mutate_doc(doc)
    m = b.members()
    m["tileset.json"] = cjson(doc)
    if extra_members:
        m.update(extra_members)
    return m


def ir_hook(name, fn):
    def hook(irs):
        fn(irs[name])
    return hook


def bq_sub(level, x, y):
    return f"subtrees/{level}/{x}/{y}.subtree"


ROOT_SUB = bq_sub(0, 0, 0)


# ---------------------------------------------------------------------------------------------------------------
# A: availability

def family_a():
    case("BQ", implicit_archive(BQ()), "Accepted", ref="base (QUADTREE L=3 A=6, rows, one slot, bitstreams)")
    case("BO", implicit_archive(BO()), "Accepted", ref="base (OCTREE L=2 A=3, transform, two slots, contentMetadata)")

    # A-1: clear (1,1,0), which has the available child (2,3,0) in the same subtree; co-adjusted by construction.
    case("A-1", implicit_archive(BQ(tiles=[t for t in BQ_TILES if t != (1, 1, 0, 0)])), "InvalidInput",
         pair="BQ", ref="A-1", note="co-adjusted: availableCount, row removed, content bit cleared, payload deleted")
    # A-2: set a leaf (4,1,2) under the available (3,1,1... no: (4,2,2)'s parent is (3,1,1)); row = parent row.
    leaf = (4, 2, 2, 0)
    b = BQ(tiles=BQ_TILES + [leaf])
    b.row_box = lambda t: bq_row_box((3, 1, 1, 0)) if t == leaf else bq_row_box(t)
    b.row_error = lambda t: bq_row_error((3, 1, 1, 0)) if t == leaf else bq_row_error(t)
    case("A-2", implicit_archive(b), "Accepted", pair="BQ", facts=("differ", "BQ"), ref="A-2",
         note="+1 tile; row copied from the parent; no content")

    # A-3: every availability bitstream rewritten MSB-first.
    def msb(ir):
        for entry in [ir.tile, ir.child] + ir.content:
            if entry[0] == "bits":
                data = entry[1]
                for i, v in enumerate(data):
                    data[i] = int(f"{v:08b}"[::-1], 2)

    case("A-3", implicit_archive(BQ(subtree_hooks=[lambda irs: [msb(ir) for ir in irs.values()]])),
         "InvalidInput", pair="BQ", ref="A-3 (InvalidInput or facts; the oracle decides InvalidInput)")

    def count_delta(d):
        def f(ir):
            e = ir.tile
            ir.tile = (e[0], e[1], e[2] + d)
        return f
    case("A-4+1", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, count_delta(+1))])), "InvalidInput",
         pair="BQ", ref="A-4")
    case("A-4-1", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, count_delta(-1))])), "InvalidInput",
         pair="BQ", ref="A-4")

    def set_bit(which, index, slot=0):
        def f(ir):
            e = ir.tile if which == "tile" else ir.child if which == "child" else ir.content[slot]
            e[1][index >> 3] |= 1 << (index & 7)
        return f
    # A-5: root subtree T = 21 bits; bit 23 is a trailing bit.
    case("A-5", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, set_bit("tile", 23))])), "InvalidInput",
         pair="BQ", ref="A-5", note="one bit")
    # A-6: content bit at (2,0,1), whose tile bit is 0 (local level 2, morton 2 -> index 5 + 2).
    # Post-freeze amendment 5: ship the member the drop-content-implies-tile mutant names (R's report: the stray
    # bit is indexed to the empty neighbour (2,1,1)), so only content => tile refuses the archive.
    m = implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, set_bit("content", 5 + morton([0, 1])))]))
    m["content/2/1/1.glb"] = glb_tile_point(cell(BQ_BOX, False, 2, 1, 1, 0)[0:3])
    case("A-6", m, "InvalidInput", pair="BQ", ref="A-6",
         note="one content bit; ships content/2/1/1.glb (amendment 5) so the mutant reaches no missing entry")
    # A-7: child bit (3,0,2): its parent (2,0,1) is unavailable.
    case("A-7", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, set_bit("child", morton([0, 2])))])),
         "InvalidInput", pair="BQ", ref="A-7", note="one bit")
    # A-8: child bit under the available (5,9,1) in subtree (3,2,0): g + L = 6 >= A = 6.
    # Post-freeze amendment 5: the child bit under (5,9,1) plus the subtree it names (6,18,2), with its tile, row,
    # content bit and payload, so the ge-vs-gt-on-A mutant reaches no missing entry; only the A rule refuses.
    case("A-8", implicit_archive(BQ(tiles=BQ_TILES + [(6, 18, 2, 0)])), "InvalidInput", pair="BQ", ref="A-8",
         note="child bit (morton 12) in subtree (3,2,0) under the available (5,9,1); ships subtrees/6/18/2.subtree "
              "and content/6/18/2.glb (amendment 5)")
    case("A-9.32", implicit_archive(BQ(A=32)), "Accepted", pair="BQ", facts=("equal", "BQ"), ref="A-9 (looser A, OD-8)")
    case("A-9.33", implicit_archive(BQ(A=33)), "Unsupported", pair="A-9.32", ref="A-9")

    def drop(ir):
        ir.drop = True
    case("A-10", implicit_archive(BQ(subtree_hooks=[ir_hook(bq_sub(3, 6, 6), drop)])), "InvalidInput",
         pair="BQ", ref="A-10")

    # A-11: two implicit roots under an explicit root; the second resolves the same subtree members.
    def two_roots(same):
        r1 = BQ(template="r1/{level}/{x}/{y}.glb")
        r2 = BQ(template="r2/{level}/{x}/{y}.glb")
        if not same:
            r2.subtree_template = "subtrees2/{level}/{x}/{y}.subtree"
        m = r1.members()
        m.update(r2.members())
        top = {"boundingVolume": {"box": BQ_BOX}, "children": [r1.root_tile(), r2.root_tile()],
               "geometricError": 2048.0, "refine": "REPLACE"}
        m["tileset.json"] = cjson(tileset(top, r1.schema_classes()))
        return m
    case("A-11.base", two_roots(False), "Accepted", ref="A-11 base (distinct subtree members)")
    case("A-11", two_roots(True), "Unsupported", pair="A-11.base", ref="A-11 (OD-7)")

    a12(set_bit)

    # A-13: three slots aliasing one view vs three equal views.
    def bq3(alias):
        content = {t for t in BQ_TILES if t not in BQ_EMPTY}
        b = Implicit("QUADTREE", 3, 6, BQ_BOX, 1024.0, BQ_TILES,
                     [Slot(f"k{i}/{{level}}/{{x}}/{{y}}.glb", content) for i in range(3)])
        b.row_box, b.row_error, b.flag_column, b.alias_content = bq_row_box, bq_row_error, True, alias
        return implicit_archive(b)
    case("A-13.base", bq3(False), "Accepted", ref="A-13 unaliased form")
    case("A-13", bq3(True), "Accepted", pair="A-13.base", facts=("equal", "A-13.base"), ref="A-13 (K = 3 aliases)")

    def child_const1(ir):
        ir.child = ("const", 1, None)
    case("A-14", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, child_const1)])), "InvalidInput",
         pair="BQ", ref="A-14 (closed form)")


def a12(set_bit):
    """Double-defect controls pinning the step order (contract precedence; A-12)."""
    # A-12a: JSON content template (root admission) + subtree version 2 (§1): Unsupported first.
    def v2(ir):
        ir.header = lambda m: m[:4] + struct.pack("<I", 2) + m[8:]
    m = implicit_archive(BQ(template="content/{level}/{x}/{y}.json", subtree_hooks=[ir_hook(ROOT_SUB, v2)]))
    case("A-12a", m, "Unsupported", pair="BQ", ref="A-12 root admission before §1",
         note="defects: JSON template (Unsupported) and subtree version 2 (InvalidInput)")
    # A-12b: trailing bit (§2a.3, InvalidInput) + visits short of the root reservation (§2a.9, ResourceLimit).
    m = implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, set_bit("tile", 23))]))
    case("A-12b", m, "InvalidInput", limits={"hierarchy_visits": 12}, pair="A-5",
         ref="A-12 §2a.3 before §2a.9", note="Tileset + Node = 2 visits, root P_t = 11 > 10 remaining")
    # A-12c: Scan charge (§2a.2, ResourceLimit) before the trailing-bit pass (§2a.3, InvalidInput).
    case("A-12c", a12c(trailing=True), "ResourceLimit:total_bytes_read", limits={"total_bytes_read": 1_000_000},
         pair="A-12c.base", ref="A-12 §2a.2 before §2a.3",
         note="Scan = 1,001 distinct overlapping views x 4,682 B ~ 4.69 MB; all member bytes < 0.4 MB")
    case("A-12c.base", a12c(trailing=False), "Accepted", ref="A-12c without its two defects")
    case("A-12c.trailing", a12c(trailing=True), "InvalidInput", pair="A-12c.base",
         ref="A-12c with default limits: the trailing bit alone")
    # A-12d: content-index reservation (§2a.10, ResourceLimit) before the property table count (§5, InvalidInput).
    def count_plus(ir):
        cls, cnt, props = ir.tables[0]
        ir.tables[0] = (cls, cnt + 1, props)
    m = implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, count_plus)]))
    case("A-12d", m, "ResourceLimit:references", limits={"references": 9}, pair="B-8+1",
         ref="A-12 §2a.10 before §5", note="root subtree member = 1 reference; E = 9 > 8 remaining")


def a12c(trailing):
    """OCTREE L=6 A=6: tile bit 0 only; slot 0 aliases the tile view; slots 1..1000 name distinct zero views."""
    n, L = 8, 6
    T = offset(n, L)
    nb = (T + 7) // 8
    K0 = 1000
    tile = bytearray(nb)
    tile[0] = 1
    if trailing:
        tile[-1] |= 0x80
    zero_off = nb + (-nb % 8)
    buf = bytes(tile) + b"\x00" * (zero_off - nb) + b"\x00" * (nb + 8 * K0)
    views = [{"buffer": 0, "byteLength": nb, "byteOffset": 0}]
    views += [{"buffer": 0, "byteLength": nb, "byteOffset": zero_off + 8 * k} for k in range(K0)]
    js = {"buffers": [{"byteLength": len(buf)}], "bufferViews": views,
          "tileAvailability": {"bitstream": 0},
          "contentAvailability": [{"bitstream": 0}] + [{"bitstream": 1 + k} for k in range(K0)],
          "childSubtreeAvailability": {"constant": 0}}
    box = [0.0, 0.0, 0.0, 32.0, 0.0, 0.0, 0.0, 32.0, 0.0, 0.0, 0.0, 32.0]
    root = {"boundingVolume": {"box": box},
            "contents": [{"uri": f"s{k}/{{level}}-{{x}}-{{y}}-{{z}}.glb"} for k in range(K0 + 1)],
            "geometricError": 64.0,
            "implicitTiling": {"availableLevels": 6, "subdivisionScheme": "OCTREE", "subtreeLevels": 6,
                               "subtrees": {"uri": "st/{level}-{x}-{y}-{z}.subtree"}},
            "refine": "ADD"}
    return {"tileset.json": cjson(tileset(root)), "st/0-0-0-0.subtree": subtree_binary(cjson(js), buf),
            "s0/0-0-0-0.glb": glb_tile_point([0.0, 0.0, 0.0])}


# ---------------------------------------------------------------------------------------------------------------
# B: rows and metadata

def family_b():
    a, b_ = (4, 3, 2, 0), (4, 2, 3, 0)

    def swapped(t):
        return bq_row_box(b_ if t == a else a if t == b_ else t)

    def swapped_e(t):
        return bq_row_error(b_ if t == a else a if t == b_ else t)
    m = BQ()
    m.row_box, m.row_error = swapped, swapped_e
    # Amendment 6: each payload sits at the centre of its tile's (swapped) row, so C2 slice 3 finds it inside its
    # own effective volume; at the cell centre it would be Outside (own) and production would refuse an I1 control.
    m.slots[0].payload = lambda t, c: glb_tile_point(swapped(t)[0:3])
    case("B-1", implicit_archive(m), "Accepted", pair="BQ", facts=("differ", "BQ"), ref="B-1 (leaf siblings swapped)")

    def ulp(t):
        r = bq_row_box(t)
        if t == a:
            r[0] = math.nextafter(r[0], math.inf)
        return r
    m = BQ()
    m.row_box = ulp
    case("B-2", implicit_archive(m), "Accepted", pair="BQ", facts=("differ", "BQ"), ref="B-2 (+1 ulp)",
         note="one ulp on a centre component")
    case("B-3", implicit_archive(BQ(tile_class="renamedClass", box_prop="hull", err_prop="gerr")), "Accepted",
         pair="BQ", facts=("equal", "BQ"), ref="B-3 (property and class renamed)")

    def drop_semantic(prop):
        def f(classes):
            del classes["tile"]["properties"][prop]["semantic"]
        return f
    case("B-4", implicit_archive(BQ(), classes_extra=drop_semantic("bounds")), "Accepted", pair="BQ",
         facts=("differ", "BQ"), ref="B-4 (box semantic removed: derived, derived-root at level 0)")
    case("B-4e", implicit_archive(BQ(), classes_extra=drop_semantic("error")), "Accepted", pair="BQ",
         facts=("differ", "BQ"), ref="B-4 (error semantic removed: e_R 2^-l)")

    def dup(classes):
        classes["tile"]["properties"]["bounds2"] = dict(classes["tile"]["properties"]["bounds"])
    case("B-5", implicit_archive(BQ(), classes_extra=dup), "InvalidInput", pair="BQ", ref="B-5")
    case("B-6", implicit_archive(BQ(comp="FLOAT32")), "Accepted", pair="BQ", facts=("equal", "BQ"),
         ref="B-6 (FLOAT32 rows widen exactly)")

    def scale(classes):
        classes["tile"]["properties"]["bounds"]["scale"] = [1.0] * 12
    case("B-7", implicit_archive(BQ(), classes_extra=scale), "Unsupported", pair="BQ", ref="B-7 (class scale)")

    def table_scale(ir):
        ir.tables[0][2]["bounds"]["scale"] = [1.0] * 12
    case("B-7t", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, table_scale)])), "Unsupported", pair="BQ",
         ref="B-7 (table-level scale; O-extra)")

    def count(d):
        def f(ir):
            cls, cnt, props = ir.tables[0]
            ir.tables[0] = (cls, cnt + d, props)
        return f
    case("B-8+1", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, count(+1))])), "InvalidInput", pair="BQ",
         ref="B-8")
    case("B-8-1", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, count(-1))])), "InvalidInput", pair="BQ",
         ref="B-8")
    for tag, v in (("neg", -1.0), ("nan", float("nan"))):
        m = BQ()
        m.row_error = (lambda v: lambda t: v if t == a else bq_row_error(t))(v)
        case(f"B-9.{tag}", implicit_archive(m), "InvalidInput", pair="BQ", ref="B-9")
    # B-10: derived errors; e_R = 2^-1020 makes level 3 subnormal; 2^-1019 keeps it normal (one exponent bit).
    for tag, e, exp in (("subnormal", math.nextafter(2.0 ** -1017, 0.0), "Unsupported"),
                        ("normal", 2.0 ** -1017, "Accepted")):
        m = BQ()
        m.error = e
        m.rows_error = False
        case(f"B-10.{tag}", implicit_archive(m), exp, pair="B-10.normal" if tag == "subnormal" else None,
             ref="B-10", note="TILE_GEOMETRIC_ERROR absent; derived e_R 2^-l reaches level 5; one ulp apart")
    # v3.2: the subnormal rule applies at level >= 1; level 0 is e_R itself.
    for tag, tiles, exp in (("root", [(0, 0, 0, 0)], "Accepted"), ("l1", [(0, 0, 0, 0), (1, 1, 0, 0)], "Unsupported")):
        m = Implicit("QUADTREE", 2, 2, BQ_BOX, 2.0 ** -1030, tiles)
        m.rows_error = False
        case(f"B-10.{tag}", implicit_archive(m), exp, pair="B-10.root" if tag == "l1" else None,
             ref="B-10 (v3.2: subnormal e_R at level 0 is e_R; level 1 derives a subnormal)")
    # B-11/B-12: the error column omitted from the table; required vs optional.
    b12 = BQ(table_error=False)
    case("B-12", implicit_archive(b12), "Accepted", pair="BQ", facts=("differ", "BQ"),
         ref="B-12 (optional omitted: e_R 2^-l)")

    def required(classes):
        classes["tile"]["properties"]["error"]["required"] = True
    case("B-11", implicit_archive(BQ(table_error=False), classes_extra=required), "InvalidInput", pair="B-12",
         ref="B-11 (required omitted)")
    b12b = BQ(table_box=False)
    case("B-12b", implicit_archive(b12b), "Accepted", pair="BQ", facts=("differ", "BQ"),
         ref="B-12 (optional box omitted: derived)")


# ---------------------------------------------------------------------------------------------------------------
# C: addressing, templates, placement, semantics

def json_doc():
    return cjson({"asset": {"version": "1.1"}, "geometricError": 1.0,
                  "root": {"boundingVolume": {"box": BO_BOX}, "geometricError": 0.0, "refine": "ADD"}})


def family_c():
    # C-1: member names written with x and y swapped.
    case("C-1", implicit_archive(BQ(member_template="content/{level}/{y}/{x}.glb",
                                    subtree_member_template="subtrees/{level}/{y}/{x}.subtree")),
         "InvalidInput", pair="BQ", ref="C-1 (InvalidInput or facts; asymmetric set gives InvalidInput)")
    case("C-2", implicit_archive(BQ(template="content/{level}/{x}.glb", member_template=BQ_TEMPLATE)),
         "InvalidInput", pair="BQ", ref="C-2")
    case("C-3", implicit_archive(BQ(template="content/{level}/{x}/{y}/{z}.glb", member_template=BQ_TEMPLATE)),
         "InvalidInput", pair="BQ", ref="C-3")

    level0 = {(0, 0, 0, 0)}
    deeper = {t for t in BO_TILES if t[0] >= 1}

    def third(template, avail, json_member=True):
        payload = (lambda t, c: json_doc()) if json_member else None
        return BO(extra_slots=[Slot(template, avail, payload=payload)])
    case("C-4.base", implicit_archive(third("o/c/{level}-{x}-{y}-{z}.glb", set())), "Accepted",
         ref="C-4 base (third slot never available)")
    case("C-4", implicit_archive(third("o/c/{level}-{x}-{y}-{z}.json", set())), "Unsupported", pair="C-4.base",
         ref="C-4")
    case("C-5.base", implicit_archive(third("o/c/{level}-{x}-{y}-{z}.glb", level0, False)), "Accepted",
         ref="C-5 base")
    case("C-5", implicit_archive(third("o/c/{level}-{x}-{y}-{z}.json", level0)), "Unsupported", pair="C-5.base",
         ref="C-5 (RC-3)")
    case("C-6.base", implicit_archive(third("o/c/{level}-{x}-{y}-{z}.glb", deeper, False)), "Accepted",
         ref="C-6 base")
    case("C-6", implicit_archive(third("o/c/{level}-{x}-{y}-{z}.json", deeper)), "Unsupported", pair="C-6.base",
         ref="C-6 (producer pattern)")
    forms = {
        "upper": ("o/c/{level}-{x}-{y}-{z}.JSON", "o/c/{level}-{x}-{y}-{z}.JSON"),
        "escaped": ("o/c/{level}-{x}-{y}-{z}.js%6Fn", "o/c/{level}-{x}-{y}-{z}.json"),
        "guard": ("o/c/{level}-{x}-{y}-{z}.j%7{x}on", "o/c/{level}-{x}-{y}-{z}.j%7{x}on"),
        "query": ("o/c/{level}-{x}-{y}-{z}.json?v=1", "o/c/{level}-{x}-{y}-{z}.json"),
        "dot": ("o/c/{level}-{x}-{y}-{z}.json/.", "o/c/{level}-{x}-{y}-{z}.json"),
        "slash": ("o/c/{level}-{x}-{y}-{z}.json/", "o/c/{level}-{x}-{y}-{z}.json"),
        "popped": ("o/c/x.json/{level}-{x}-{y}-{z}/..", "o/c/x.json"),
        "upperdot": ("o/c/{level}-{x}-{y}-{z}.JSON//./", "o/c/{level}-{x}-{y}-{z}.JSON"),
    }
    for tag, (template, member) in forms.items():
        for avail_tag, avail in (("avail", deeper), ("never", set())):
            s = Slot(template, avail, member_template=member, payload=lambda t, c: json_doc())
            case(f"C-7.{tag}.{avail_tag}", implicit_archive(BO(extra_slots=[s])), "Unsupported",
                 pair="C-6.base", ref="C-7 (DC-2 forms; also never available)")
    # C-7b/c/d replace slot a.
    def slot_a(template, member):
        return Slot(template, BO_A, member_template=member, metadata_class="cmeta")
    case("C-7b.base", implicit_archive(BO(slot_a("c2{x}/{level}-{y}-{z}.glb", None))), "Accepted", ref="C-7b base")
    case("C-7b", implicit_archive(BO(slot_a("c%2{x}/{level}-{y}-{z}.glb", "c2{x}/{level}-{y}-{z}.glb"))),
         "Unsupported", pair="C-7b.base", ref="C-7b (escape guard, non-last segment)")
    case("C-7c.base", implicit_archive(BO(slot_a("c/{level}/{x}/{y}/{z}/a.glb", None))), "Accepted",
         ref="C-7c base")
    case("C-7c", implicit_archive(BO(slot_a("c/{level}/../{x}/../{y}/../{z}/../a.glb", "c/a.glb"))),
         "Unsupported", pair="C-7c.base", ref="C-7c (variables eliminated)")
    case("C-7d.base", implicit_archive(BO(slot_a("c/{level}/{x}-{y}-{z}.glb", None))), "Accepted", ref="C-7d base")
    case("C-7d", implicit_archive(BO(slot_a("./c//{level}/./{x}-{y}-{z}.glb", "c/{level}/{x}-{y}-{z}.glb"))),
         "Accepted", pair="C-7d.base", facts=("equal", "C-7d.base"),
         ref="C-7d (normalization exactness: same members and volumes)")
    # O-extra: a decoded brace in a static part is literal text, never a variable.
    case("C-7e", implicit_archive(BO(slot_a("c/%7Bx%7D-{level}-{x}-{y}-{z}.glb",
                                            lambda t: "c/{x}-%d-%d-%d-%d.glb" % t))),
         "Accepted", ref="O-extra: %7Bx%7D decodes to the literal '{x}' (normalization is structural)")

    # C-8: implicit root on a non-root explicit tile.
    r = BQ()
    m = r.members()
    leaf = {"boundingVolume": {"box": [40.0, 8.0, 2.0, 8.0, 0, 0, 0, 8.0, 0, 0, 0, 2.0]}, "content": {"uri": "leaf.glb"},
            "geometricError": 0.0}
    top = {"boundingVolume": {"box": [24.0, 8.0, 2.0, 24.0, 0, 0, 0, 8.0, 0, 0, 0, 2.0]},
           "children": [r.root_tile(), leaf], "geometricError": 2048.0, "refine": "ADD"}
    m["leaf.glb"] = glb_tile_point([40.0, 8.0, 2.0])  # post-freeze amendment 2
    m["tileset.json"] = cjson(tileset(top, r.schema_classes()))
    case("C-8", m, "Accepted", ref="C-8 (X-8)", note="R (refine REPLACE) is the first child of an ADD root")

    # C-9 root volume kinds and scheme.
    def region(root):
        root["boundingVolume"] = {"region": [-1.3, 0.69, -1.29, 0.7, 0.0, 20.0]}
    def sphere(root):
        root["boundingVolume"] = {"sphere": [16.0, 8.0, 2.0, 20.0]}
    def hextree(root):
        root["implicitTiling"]["subdivisionScheme"] = "HEXTREE"
    case("C-9.region", implicit_archive(BQ(), mutate_root=region), "Unsupported", pair="BQ", ref="C-9")
    case("C-9.sphere", implicit_archive(BQ(), mutate_root=sphere), "InvalidInput", pair="BQ", ref="C-9")
    case("C-9.hextree", implicit_archive(BQ(), mutate_root=hextree), "Unsupported", pair="BQ", ref="C-9")

    m = implicit_archive(BQ())
    m["content/0/0/0.glb"] = json_doc()
    case("C-10", m, "Accepted", pair="BQ", production="InvalidInput",
         ref="C-10 (C1 magic; the oracle treats payloads as opaque)")

    # C-11: explicit external tileset whose root is implicit, with a transform.
    case("C-11", external(BO(), [1000.0, 2000.0, 0.0], 30.0, 400.0), "Accepted", ref="C-11")

    # C-12/C-13: contentMetadata classes with CONTENT_URI / CONTENT_GROUP_ID.
    def cm_column(semantic, kind):
        def classes_fn(classes):
            p = {"type": "STRING"} if kind == "string" else {"componentType": "UINT32", "type": "SCALAR"}
            if semantic:
                p["semantic"] = semantic
            classes["cmeta"]["properties"]["extra"] = p

        def hook(irs):
            for ir in irs.values():
                if ir.content_metadata is None:
                    continue
                for ti in ir.content_metadata:
                    cls, cnt, props = ir.tables[ti]
                    if kind == "string":
                        text = b"x.json" * cnt
                        offs = b"".join(struct.pack("<I", 6 * i) for i in range(cnt + 1))
                        props["extra"] = {"values": text, "stringOffsets": offs}
                    else:
                        props["extra"] = {"values": b"".join(struct.pack("<I", 7) for _ in range(cnt))}
        return classes_fn, hook
    for cid, sem, kind in (("C-12", "CONTENT_URI", "string"), ("C-13", "CONTENT_GROUP_ID", "u32")):
        fn0, hook0 = cm_column(None, kind)
        case(f"{cid}.base", implicit_archive(BO(subtree_hooks=[hook0]), classes_extra=fn0), "Accepted",
             ref=f"{cid} base (same column, no semantic)")
        fn1, hook1 = cm_column(sem, kind)
        case(cid, implicit_archive(BO(subtree_hooks=[hook1]), classes_extra=fn1), "Unsupported",
             pair=f"{cid}.base", ref=f"{cid} (RC-1)", note="the CONTENT_URI value names x.json" if kind == "string"
             else None)
    # C-12b: template header metadata classes.
    for tag, sem, prop, value, p in (
            ("uri", "CONTENT_URI", "u", "x.json", {"type": "STRING"}),
            ("group", "CONTENT_GROUP_ID", "g", 0, {"componentType": "UINT32", "type": "SCALAR"})):
        for with_sem in (False, True):
            s = Slot("o/a/{level}-{x}-{y}-{z}.glb", BO_A, metadata_class="cmeta",
                     header={"metadata": {"class": "hmeta", "properties": {prop: value}}})
            pp = dict(p)
            if with_sem:
                pp["semantic"] = sem

            def fn(classes, pp=pp, prop=prop):
                classes["hmeta"] = {"properties": {prop: pp}}
            cid = f"C-12b.{tag}" + ("" if with_sem else ".base")
            case(cid, implicit_archive(BO(slot_a=s), classes_extra=fn), "Unsupported" if with_sem else "Accepted",
                 pair=None if not with_sem else f"C-12b.{tag}.base", ref="C-12b (DC-5, at root admission)")

    def const0(irs):
        for ir in irs.values():
            ir.content = [("const", 0, None) for _ in ir.content]
    case("C-14", implicit_archive(BQ(content=set(), subtree_hooks=[const0])), "InvalidInput", pair="BQ",
         ref="C-14 (post-walk :299)", note="every content layer Const0; no payload members")
    # O-extra: implicit root without refine inherits ADD (W7 pairs with the "refine forced to REPLACE" mutant).
    r = BQ(refine=None)
    m = r.members()
    top = {"boundingVolume": {"box": BQ_BOX}, "children": [r.root_tile()], "geometricError": 2048.0, "refine": "ADD"}
    m["tileset.json"] = cjson(tileset(top, r.schema_classes()))
    case("C-15", m, "Accepted", ref="O-extra: inherited refine ADD (paired with 'refine forced to REPLACE')")

    # O-extra single-defect controls for exit rows not named in C (X-3, X-4, X-13, root shape).
    def s2(root):
        root["boundingVolume"]["extensions"] = {"3DTILES_bounding_volume_S2": {
            "token": "3", "minimumHeight": 0.0, "maximumHeight": 10.0}}
    case("C-9.s2", implicit_archive(BQ(), mutate_root=s2), "Unsupported", pair="BQ", ref="O-extra: S2 root (X-3)")

    def v10(root):
        root["extensions"] = {"3DTILES_implicit_tiling": root.pop("implicitTiling")}
    case("C-16", implicit_archive(BQ(), mutate_root=v10,
                                  mutate_doc=lambda d: d.update({"extensionsUsed": ["3DTILES_implicit_tiling"]})),
         "Unsupported", pair="BQ", ref="O-extra: 3DTILES_implicit_tiling 1.0 (X-13)")

    def with_children(root):
        root["children"] = [{"boundingVolume": {"box": BQ_BOX}, "geometricError": 0.0}]

    def with_metadata(root):
        root["metadata"] = {"class": "tile"}

    def with_cbv(root):
        root["content"]["boundingVolume"] = {"box": BQ_BOX}
    case("C-17.children", implicit_archive(BQ(), mutate_root=with_children), "InvalidInput", pair="BQ",
         ref="O-extra: implicit root with children (:92-97)")
    case("C-17.metadata", implicit_archive(BQ(), mutate_root=with_metadata), "InvalidInput", pair="BQ",
         ref="O-extra: implicit root with metadata (:92-97)")
    case("C-17.contentbv", implicit_archive(BQ(), mutate_root=with_cbv), "InvalidInput", pair="BQ",
         ref="O-extra: content template with boundingVolume (:92-97)")

    def levels(field, value):
        def f(root):
            root["implicitTiling"][field] = value
        return f
    case("A-9b", implicit_archive(BQ(), mutate_root=levels("subtreeLevels", 33)), "Unsupported", pair="BQ",
         ref="O-extra: subtreeLevels 33 (X-4)")
    case("A-9c", implicit_archive(BQ(), mutate_root=levels("availableLevels", 0)), "InvalidInput", pair="BQ",
         ref="O-extra: availableLevels 0")

    def offset_on_error(classes):
        classes["tile"]["properties"]["error"]["offset"] = 0.0
    for tag, text in (("float", b"6.0"), ("exp", b"6e0")):
        m = implicit_archive(BQ())
        m["tileset.json"] = m["tileset.json"].replace(b'"availableLevels":6', b'"availableLevels":' + text, 1)
        case(f"A-9d.{tag}", m, "InvalidInput", pair="BQ", ref="v3.2: implicit integer fields are JSON integers")

    def dup_opaque(classes):
        classes["tile"]["properties"]["flag"]["semantic"] = "NAME"
        classes["tile"]["properties"]["flag2"] = {"type": "BOOLEAN", "semantic": "NAME"}
    case("B-5b", implicit_archive(BQ(), classes_extra=dup_opaque), "InvalidInput", pair="BQ",
         ref="v3.2: a duplicated semantic of any kind in one class")

    def ext_twice():
        m = external(BO(), [1000.0, 2000.0, 0.0], 30.0, 400.0)
        top = json.loads(m["tileset.json"])
        child = top["root"]
        second = dict(child, geometricError=child["geometricError"] - 1.0)  # post-freeze amendment 1: uniqueItems
        top["root"] = {"boundingVolume": child["boundingVolume"], "children": [dict(child), second],
                       "geometricError": 500.0, "refine": "REPLACE"}
        top["geometricError"] = 500.0  # post-freeze amendment 4: root error <= document geometricError
        m["tileset.json"] = cjson(top)
        return m
    case("C-11b", ext_twice(), "Unsupported", pair="C-11",
         ref="v3.2: OD-7 validation-wide (one implicit-root external tileset referenced twice)")
    case("C-7f", implicit_archive(BO(slot_a("c%/{level}/{x}-{y}-{z}.glb", "c/{level}/{x}-{y}-{z}.glb"))),
         "InvalidInput", pair="C-7d.base", ref="v3.2: escape guard per segment; 'c%' is a malformed escape")

    case("B-7o", implicit_archive(BQ(), classes_extra=offset_on_error), "Unsupported", pair="BQ",
         ref="O-extra: offset on TILE_GEOMETRIC_ERROR")


def external(b, translation, half, top_error, implicit_half=None, row0_half=None):
    """tileset.json (explicit root P, content ext/tileset.json) -> ext/tileset.json with implicit root R."""
    b.transform = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0] + list(translation) + [1]
    root = b.root_tile()
    if implicit_half is not None:
        root["boundingVolume"]["box"] = [b.box[0], b.box[1], b.box[2], implicit_half, 0, 0, 0, implicit_half, 0,
                                         0, 0, implicit_half]
    doc = tileset(root, b.schema_classes())
    m = {}
    for k, v in b.members().items():
        m["ext/" + k] = v
    m["ext/tileset.json"] = cjson(doc)
    cx = translation[0] + b.box[0]
    cy = translation[1] + b.box[1]
    cz = translation[2] + b.box[2]
    top = {"boundingVolume": {"box": [cx, cy, cz, half, 0, 0, 0, half, 0, 0, 0, half]},
           "content": {"uri": "ext/tileset.json"}, "geometricError": top_error, "refine": "REPLACE"}
    m["tileset.json"] = cjson(tileset(top))
    return m


# ---------------------------------------------------------------------------------------------------------------
# D: container

def family_d():
    def trunc(ir):
        ir.header = lambda m: m[:-8]
    case("D-1", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, trunc)])), "InvalidInput", pair="BQ",
         ref="D-1 (truncated BIN)")

    def view_offset(delta, absent=False):
        def f(js):
            v = js["bufferViews"][-1]
            if absent:
                del v["byteOffset"]
            else:
                v["byteOffset"] += delta
        return f
    case("D-2+1", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, lambda ir: ir.post_js.append(
        view_offset(1)))])), "InvalidInput", pair="BQ", ref="D-2 (byteOffset +1)")
    case("D-2.absent", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, lambda ir: ir.post_js.append(
        view_offset(0, True)))])), "InvalidInput", pair="BQ", ref="D-2 (byteOffset absent)")

    def padding(binchunk, js):
        binchunk[-1] = 1
    case("D-3", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, lambda ir: ir.post_bin.append(padding))])),
         "InvalidInput", pair="BQ", ref="D-3", note="one bit in the BIN padding")

    def version2(ir):
        ir.header = lambda m: m[:4] + struct.pack("<I", 2) + m[8:]

    def magic(ir):
        ir.header = lambda m: b"subT" + m[4:]
    case("D-4.version", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, version2)])), "InvalidInput",
         pair="BQ", ref="D-4")
    case("D-4.magic", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, magic)])), "InvalidInput", pair="BQ",
         ref="D-4 (bad magic; v3.2 ruling 1)")

    def ext(uri):
        def f(ir):
            ir.external = (bq_sub(0, 0, 0)[:-len(".subtree")] + ".bin", uri)
        return f
    case("D-6", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, ext("0.bin"))])), "Accepted", pair="BQ",
         facts=("equal", "BQ"), ref="D-6 (valid external buffer, OD-6)")
    m = implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, ext("data:application/octet-stream;base64,AAAA"))]))
    m.pop("subtrees/0/0/0.bin")
    case("D-5", m, "InvalidInput", pair="D-6", ref="D-5 (data URI)")

    def to_json_format(ir):
        ir.header = lambda m: cjson({"tileAvailability": {"constant": 1},
                                     "childSubtreeAvailability": {"constant": 0}})
    case("D-7", implicit_archive(BQ(subtree_hooks=[ir_hook(bq_sub(3, 6, 6), to_json_format)])), "Unsupported",
         pair="BQ", ref="D-7 (JSON-format subtree)")

    def bom_json(ir):
        ir.header = lambda m: b"\xef\xbb\xbf \n\t" + cjson({"tileAvailability": {"constant": 1},
                                                          "childSubtreeAvailability": {"constant": 0}})
    case("D-7b", implicit_archive(BQ(subtree_hooks=[ir_hook(bq_sub(3, 6, 6), bom_json)])), "Unsupported",
         pair="D-7", ref="D-7 with a UTF-8 BOM and leading whitespace (v3.2 ruling 1)")

    def newline(ir):
        def text(js):
            body = cjson(js)
            k = (-(len(body) + 1)) % 8
            return b"{" + b" " * k + body[1:] + b"\n"
        ir.json_text = text
    case("D-8", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, newline)])), "Accepted", pair="BQ",
         facts=("equal", "BQ"), ref="D-8 (RC-5: JSON ends in \\n, no padding)")

    def ext_key(ir):
        ir.extra_json["extensions"] = {}
    case("D-11", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, ext_key)])), "Unsupported", pair="BQ",
         ref="O-extra: extensions in subtree JSON (X-6)")

    def extras_ext(ir):
        ir.extra_json["extras"] = {"extensions": {"x": 1}, "note": {"extensions": []}}
    case("D-11b", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, extras_ext)])), "Accepted", pair="D-11",
         facts=("equal", "BQ"), ref="v3.2: 'extensions' inside extras is not an extension")

    def float_offset(ir):
        ir.json_text = lambda js: cjson(js).replace(b'"byteOffset":0}', b'"byteOffset":0.0}', 1)
    case("D-2.float", implicit_archive(BQ(subtree_hooks=[ir_hook(ROOT_SUB, float_offset)])), "InvalidInput",
         pair="BQ", ref="v3.2: integer fields are JSON integers (byteOffset 0.0)")

    # D-9 on a one-tile base (no views needed).
    for tag, extra, exp in (("base", {}, "Accepted"), ("", {"bufferViews": []}, "InvalidInput")):
        b = Implicit("QUADTREE", 1, 1, BQ_BOX, 8.0, [(0, 0, 0, 0)])
        b.rows_box = b.rows_error = False
        b.force_bitstream = False
        b.subtree_hooks = [lambda irs, extra=extra: irs[ROOT_SUB].extra_json.update(extra)]
        case("D-9" + (".base" if tag else ""), implicit_archive(b), exp, pair=None if tag else "D-9.base",
             ref="D-9 (RC-15)")

    # D-10: contentMetadata in the subtree whose slot b is empty.
    def d10(count):
        def hook(irs):
            ir = irs["st/2.0.1.0.subtree"]
            ir.content_metadata = []
            for k in range(2):
                ir.content_metadata.append(len(ir.tables))
                ir.tables.append(("cmeta", 1 if k == 0 else count, {"weight": {"values": bytes([1, 2])[:max(
                    1, 1 if k == 0 else count)]}}))
        return hook
    case("D-10a", implicit_archive(BO(subtree_hooks=[d10(0)])), "InvalidInput", pair="BO", ref="D-10a (count 0)")
    case("D-10b", implicit_archive(BO(subtree_hooks=[d10(1)])), "InvalidInput", pair="BO", ref="D-10b (count 1)")


# ---------------------------------------------------------------------------------------------------------------
# T: tolerances

T_BOX = [0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 10.0]
T_TOL = 1e-6 * (1.0 + math.sqrt(300.0))


def single(row_half_x=10.0, e_r=100.0, e_0=100.0, box=None, half=10.0):
    b = Implicit("QUADTREE", 1, 1, box or T_BOX, e_r, [(0, 0, 0, 0)])
    b.force_bitstream = False
    b.row_box = lambda t: [0.0, 0.0, 0.0, row_half_x, 0.0, 0.0, 0.0, half, 0.0, 0.0, 0.0, half]
    b.row_error = lambda t: e_0
    return b


def family_t():
    case("T-0", implicit_archive(single()), "Accepted", ref="T base (V_0 = V_R, e_0 = e_R)")
    case("T-1a", implicit_archive(single(row_half_x=10.0 + 2e-6)), "Accepted", pair="T-0",
         ref="T-1 (outside by 2e-6, ||h|| = 17.3 >= 10)")
    case("T-1b", implicit_archive(single(row_half_x=10.0 + 2 * T_TOL)), "InvalidInput", pair="T-1a",
         ref="T-1 (outside by 2 x 1e-6(1 + ||h||))")
    case("T-2a", implicit_archive(single(e_0=100.0 * (1 + 1e-9))), "Accepted", pair="T-0", ref="T-2")
    case("T-2b", implicit_archive(single(e_0=100.0 + 2 * 1e-8 * (1 + 100.0))), "InvalidInput", pair="T-2a",
         ref="T-2")

    def ext_t(implicit_half):
        b = single(row_half_x=9.0, half=9.0, e_0=90.0)
        return external(b, [5.0, 5.0, 5.0], 10.0, 200.0, implicit_half=implicit_half)
    # Post-freeze amendment 4: a document root's error must not exceed its document's geometricError.
    case("T-4", implicit_archive(single(), error=100.0 - 2 * 1e-8 * 101.0), "InvalidInput", pair="T-0",
         ref="O-extra: root geometricError above its document's geometricError (pre-I1 rule)")
    case("T-3.base", ext_t(10.0), "Accepted", ref="T-3 base (V_R = referencing tile)")
    case("T-3", ext_t(12.0), "InvalidInput", pair="T-3.base", ref="T-3 (V_R escapes P; V_0 does not; X-11)")


# ---------------------------------------------------------------------------------------------------------------
# R: resources

def tiny_tree(scheme, L, A, tile_entry, child_entry, box=None, error=64.0, buf=b"", views=None, templates=()):
    js = {}
    if buf:
        js["buffers"] = [{"byteLength": len(buf)}]
        js["bufferViews"] = views
    js["tileAvailability"] = tile_entry
    if templates:
        js["contentAvailability"] = [{"constant": 0}] * len(templates)
    js["childSubtreeAvailability"] = child_entry
    root = {"boundingVolume": {"box": box or [0.0, 0.0, 0.0, 64.0, 0, 0, 0, 64.0, 0, 0, 0, 64.0]},
            "geometricError": error,
            "implicitTiling": {"availableLevels": A, "subdivisionScheme": scheme, "subtreeLevels": L,
                               "subtrees": {"uri": "st/{level}-{x}-{y}" + ("-{z}" if scheme == "OCTREE" else "")
                                                   + ".subtree"}},
            "refine": "ADD"}
    name = "st/0-0-0" + ("-0" if scheme == "OCTREE" else "") + ".subtree"
    return {"tileset.json": cjson(tileset(root)), name: subtree_binary(cjson(js), buf)}


def family_r(large):
    # R1b: the last subtree (3,7,7) in DFS order has 4 tiles and a corrupt payload in its first tile.
    m = implicit_archive(BQ())
    glb = bytearray(m["content/3/7/7.glb"])
    glb[0:4] = b"glTX"
    m["content/3/7/7.glb"] = bytes(glb)
    full_through_last = 2 + len(BQ_TILES)          # Tileset + Node(R) + all P_t; ExitTileset adds one more
    case("R1b", m, "ResourceLimit:hierarchy_visits", limits={"hierarchy_visits": full_through_last - 1},
         ref="R1b (reservation at the last subtree's admission)", question="Q2",
         note="limit = visits needed through the last reservation minus one (26 - 1); per-visit charging would "
              "reach the corrupt payload first (InvalidInput)", file="R1b")
    case("R1b.full", m, "Accepted", limits={"hierarchy_visits": full_through_last + 1}, production="InvalidInput",
         ref="R1b with the full count: C1 refuses the corrupt payload", file="R1b")
    # R4: depth exact (max depth 5) versus one less.
    case("R4", implicit_archive(BQ()), "Accepted", limits={"hierarchy_depth": 5}, ref="R4 exact", question="Q3",
         file="BQ")
    case("R4-1", implicit_archive(BQ()), "ResourceLimit:hierarchy_depth", limits={"hierarchy_depth": 4},
         ref="R4 one less", question="Q3", file="BQ")
    # R3: OCTREE tile Const1, child Const0.
    case("R3.L8", tiny_tree("OCTREE", 8, 8, {"constant": 1}, {"constant": 0}), "ResourceLimit:hierarchy_visits",
         ref="R3 (L = A = 8, 2,396,745 tiles, step 9)")
    case("R3.L32", tiny_tree("OCTREE", 32, 32, {"constant": 1}, {"constant": 0}), "ResourceLimit:hierarchy_visits",
         ref="R3 (L = A = 32)")
    # R5b: a bitstream tile source whose popcount exceeds the remaining visits.
    T7 = offset(8, 7)
    nb = (T7 + 7) // 8
    data = bytearray(b"\xff" * nb)
    if T7 % 8:
        data[-1] = (1 << (T7 % 8)) - 1
    case("R5b", tiny_tree("OCTREE", 7, 7, {"bitstream": 0}, {"constant": 0}, buf=bytes(data),
                          views=[{"buffer": 0, "byteLength": nb, "byteOffset": 0}]),
         "ResourceLimit:hierarchy_visits", ref="R5b (bitstream P_t = 299,593)")
    case("R5", implicit_archive(BQ()), "Accepted", ref="R5 (fixture for the injected rank-table refusal; "
         "production expects ResourceLimit under injection only)", production="ResourceLimit (injected)", file="BQ")
    # R9: K = 2,500 identical Const1 slots; the first child subtree's step 10 refuses.
    case("R9", r9(), "ResourceLimit:references", ref="R9 (DC-3: 182,500 reserved + 182,500 > 262,144)")
    case("R8b.1024", r8b(1024), "Accepted", ref="R8b (normalized length 1,024 B)")
    case("R8b.1025", r8b(1025), "Unsupported", pair="R8b.1024", ref="R8b (1,025 B)")
    if large:
        family_r_large()


def r9():
    n, L = 8, 3
    K = 2500
    js = {"tileAvailability": {"constant": 1}, "contentAvailability": [{"constant": 1}] * K,
          "childSubtreeAvailability": {"constant": 1}}
    child = {"tileAvailability": {"constant": 1}, "contentAvailability": [{"constant": 1}] * K,
             "childSubtreeAvailability": {"constant": 0}}
    box = [0.0, 0.0, 0.0, 64.0, 0, 0, 0, 64.0, 0, 0, 0, 64.0]
    root = {"boundingVolume": {"box": box}, "contents": [{"uri": "c/{level}-{x}-{y}-{z}.glb"}] * K,
            "geometricError": 64.0,
            "implicitTiling": {"availableLevels": 6, "subdivisionScheme": "OCTREE", "subtreeLevels": L,
                               "subtrees": {"uri": "st/{level}-{x}-{y}-{z}.subtree"}}, "refine": "ADD"}
    m = {"tileset.json": cjson(tileset(root)), "st/0-0-0-0.subtree": subtree_binary(cjson(js), b""),
         "st/3-0-0-0.subtree": subtree_binary(cjson(child), b"")}
    for level in range(L):
        for x in range(1 << level):
            for y in range(1 << level):
                for z in range(1 << level):
                    c = cell(box, True, level, x, y, z)[0:3]
                    m[f"c/{level}-{x}-{y}-{z}.glb"] = glb_tile_point(c)
    return m


def r8b(length):
    """Normalized template of exactly `length` bytes: static directory segments (<= 200 B) then the variables."""
    tail = "/{level}-{x}-{y}.glb"
    rest = length - 1 - len(tail)
    segs = []
    while rest > 0:
        chunk = min(201, rest)
        if rest - chunk == 1:
            chunk -= 1
        segs.append("a" * (chunk - 1))
        rest -= chunk
    template = "c" + "".join("/" + s for s in segs) + tail
    assert len(template.encode()) == length, (len(template), length)
    b = Implicit("QUADTREE", 1, 1, BQ_BOX, 8.0, [(0, 0, 0, 0)], [Slot(template, {(0, 0, 0, 0)})])
    b.rows_box = b.rows_error = False
    b.force_bitstream = False
    return implicit_archive(b)


def family_r_large():
    # R2: QUADTREE L=1 A=6, branching 2 (children 0 and 3), no content, 64 KiB unused view per subtree.
    pad_view = b"\x00" * 65536
    tiles = [(0, 0, 0, 0)]
    frontier = [(0, 0, 0, 0)]
    for level in range(1, 6):
        nxt = []
        for t in frontier:
            for c in (0, 3):
                nxt.append((level, 2 * t[1] + (c & 1), 2 * t[2] + (c >> 1), 0))
        tiles += nxt
        frontier = nxt
    b = Implicit("QUADTREE", 1, 6, BQ_BOX, 64.0, tiles)
    b.rows_box = b.rows_error = False
    b.force_bitstream = True
    b.tile_count = False
    b.subtree_hooks = [lambda irs: [ir.extra_views.append(pad_view) for ir in irs.values()]]
    m = implicit_archive(b)
    sizes = sorted({len(v) for k, v in m.items() if k.endswith(".subtree")})
    assert len(sizes) == 1
    S = sizes[0]
    # VS-B-8 (amendment 7): each subtree also holds its rank table, 4 * ceil(T/64) = 4 B (T = 1, tile availability
    # a bitstream, no content so E = 0 and no content tables).  The table is charged after the member read, so the
    # sixth subtree's charge, not its read, is the first to cross cap - 1.
    RANK = 4 * ((1 + 63) // 64)
    live, cum = 6 * (S + RANK), 63 * S
    note = (f"S = {S} B per subtree plus a {RANK} B rank table; maximum live sum 6(S + {RANK}) = {live}; "
            f"cumulative members 63S = {cum}")
    case("R2", m, "Accepted", limits={"document_decoded_bytes": live}, large=True, ref="R2 (cap = maximum live sum)",
         note=note, file="R2")
    case("R2-1", m, "ResourceLimit:document_decoded_bytes", limits={"document_decoded_bytes": live - 1}, large=True,
         ref="R2 (one less: the sixth subtree's table charge is refused)", note=note, file="R2")
    case("R2b.mid", m, "Accepted", limits={"document_decoded_bytes": (live + cum) // 2}, large=True,
         ref="R2b (between live and cumulative)", note=note, file="R2")
    case("R2b.cum-1", m, "Accepted", limits={"document_decoded_bytes": cum - 1}, large=True,
         ref="R2b (cumulative - 1; flips the cumulative mutant)", note=note, file="R2")
    # R6: subtree member at the member_bytes boundary (child bitstream C = 8^8 bits, all zero).
    n, L = 8, 8
    T, C = offset(n, L), n ** L
    tb = bytearray((T + 7) // 8)
    tb[0] = 1
    cb = bytes(C // 8)
    toff = 0
    coff = len(tb) + (-len(tb) % 8)
    buf = bytes(tb) + b"\x00" * (coff - len(tb)) + cb
    m6 = tiny_tree("OCTREE", 8, 8, {"bitstream": 0}, {"bitstream": 1}, buf=buf,
                   views=[{"buffer": 0, "byteLength": len(tb), "byteOffset": toff},
                          {"buffer": 0, "byteLength": len(cb), "byteOffset": coff}])
    size = len(m6["st/0-0-0-0.subtree"])
    case("R6", m6, "Accepted", limits={"member_bytes": size}, large=True, ref=f"R6 (member_bytes = {size})", file="R6")
    case("R6-1", m6, "ResourceLimit:member_bytes", limits={"member_bytes": size - 1}, large=True,
         ref="R6 (member_bytes - 1)", file="R6")
    # R7 / R7a: OCTREE L=10 A=1; 19,173,962 B tile view with bit 0; K aliasing layers.
    n, L = 8, 10
    T = offset(n, L)
    nb = (T + 7) // 8
    for cid, extra in (("R7", False), ("R7a", True)):
        K = 20000
        tile = bytearray(nb)
        tile[0] = 1
        second = nb + (-nb % 8)
        buf = bytes(tile) + b"\x00" * (second - nb) + bytes(nb)
        views = [{"buffer": 0, "byteLength": nb, "byteOffset": 0},
                 {"buffer": 0, "byteLength": nb, "byteOffset": second}]
        layers = [{"bitstream": 1, "availableCount": 0}] * K
        templates = [{"uri": "c/{level}-{x}-{y}-{z}.glb"}] * K
        if extra:
            layers = layers + [{"bitstream": 0, "availableCount": 1}]
            templates = templates + [{"uri": "a/{level}-{x}-{y}-{z}.glb"}]
        js = {"buffers": [{"byteLength": len(buf)}], "bufferViews": views,
              "tileAvailability": {"bitstream": 0, "availableCount": 1}, "contentAvailability": layers,
              "childSubtreeAvailability": {"constant": 0}}
        root = {"boundingVolume": {"box": [0.0, 0.0, 0.0, 64.0, 0, 0, 0, 64.0, 0, 0, 0, 64.0]},
                "contents": templates, "geometricError": 64.0,
                "implicitTiling": {"availableLevels": 1, "subdivisionScheme": "OCTREE", "subtreeLevels": L,
                                   "subtrees": {"uri": "st/{level}-{x}-{y}-{z}.subtree"}}, "refine": "ADD"}
        mm = {"tileset.json": cjson(tileset(root)), "st/0-0-0-0.subtree": subtree_binary(cjson(js), buf)}
        if extra:
            mm["a/0-0-0-0.glb"] = glb_tile_point([0.0, 0.0, 0.0])
        case(cid, mm, "Accepted" if extra else "InvalidInput", large=True,
             ref="R7a (one extra layer aliasing the tile view)" if extra else "R7 (post-walk content rule)",
             note="Scan = 2 x 19,173,962 B (tile view + one shared zero view)")
    # R7b: K = 10,000 layers, each its own view at offsets 8, 16, ... overlapping the tile view at 0 (DC-6).
    K = 10000
    tile = bytearray(nb + 8 * K)
    tile[0] = 1
    buf = bytes(tile)
    views = [{"buffer": 0, "byteLength": nb, "byteOffset": 0}]
    views += [{"buffer": 0, "byteLength": nb, "byteOffset": 8 * (k + 1)} for k in range(K)]
    js = {"buffers": [{"byteLength": len(buf)}], "bufferViews": views, "tileAvailability": {"bitstream": 0},
          "contentAvailability": [{"bitstream": 1 + k} for k in range(K)], "childSubtreeAvailability": {"constant": 0}}
    root = {"boundingVolume": {"box": [0.0, 0.0, 0.0, 64.0, 0, 0, 0, 64.0, 0, 0, 0, 64.0]},
            "contents": [{"uri": "c/{level}-{x}-{y}-{z}.glb"}] * K, "geometricError": 64.0,
            "implicitTiling": {"availableLevels": 1, "subdivisionScheme": "OCTREE", "subtreeLevels": L,
                               "subtrees": {"uri": "st/{level}-{x}-{y}-{z}.subtree"}}, "refine": "ADD"}
    case("R7b", {"tileset.json": cjson(tileset(root)), "st/0-0-0-0.subtree": subtree_binary(cjson(js), buf)},
         "ResourceLimit:total_bytes_read", large=True, ref="R7b (DC-6: Scan ~ 10,001 x 19.17 MB at step 2)",
         note="one 19,253,962 B buffer; the tile view at 0 and 10,000 content views at 8, 16, ... (all zero)")
    # R7c: OCTREE L=1 A=6, 37,449 single-tile subtrees, one opaque BOOLEAN column; a 32,668-property class.
    props = {f"p{i}": {"type": "BOOLEAN"} for i in range(32668)}
    root = {"boundingVolume": {"box": [0.0, 0.0, 0.0, 64.0, 0, 0, 0, 64.0, 0, 0, 0, 64.0]}, "geometricError": 64.0,
            "implicitTiling": {"availableLevels": 6, "subdivisionScheme": "OCTREE", "subtreeLevels": 1,
                               "subtrees": {"uri": "st/{level}-{x}-{y}-{z}.subtree"}}, "refine": "ADD"}
    mm = {"tileset.json": cjson(tileset(root, {"c": {"properties": props}}))}
    for level in range(6):
        child = {"constant": 1 if level < 5 else 0}
        js = {"buffers": [{"byteLength": 1}], "bufferViews": [{"buffer": 0, "byteLength": 1, "byteOffset": 0}],
              "propertyTables": [{"class": "c", "count": 1, "properties": {"p0": {"values": 0}}}],
              "tileAvailability": {"constant": 1}, "childSubtreeAvailability": child, "tileMetadata": 0}
        blob = subtree_binary(cjson(js), b"\x01")
        for x in range(1 << level):
            for y in range(1 << level):
                for z in range(1 << level):
                    mm[f"st/{level}-{x}-{y}-{z}.subtree"] = blob
    case("R7c", mm, "Accepted", large=True, ref="R7c (DC-4: once-per-class classification)")
    # R8 / R8a / R8a.nopad: OCTREE L=6 Const1, one Const1 slot, the reviewer's padding.
    padding = "./" * 4_000_000
    for cid, template, exp in (("R8", padding + "c/{level}/../{x}/../{y}/../{z}/../a.glb", "Unsupported"),
                               ("R8a", padding + "c/{level}-{x}-{y}-{z}.glb", "Accepted"),
                               ("R8a.nopad", "c/{level}-{x}-{y}-{z}.glb", "Accepted")):
        js = {"tileAvailability": {"constant": 1}, "contentAvailability": [{"constant": 1}],
              "childSubtreeAvailability": {"constant": 0}}
        box = [0.0, 0.0, 0.0, 64.0, 0, 0, 0, 64.0, 0, 0, 0, 64.0]
        root = {"boundingVolume": {"box": box}, "content": {"uri": template}, "geometricError": 64.0,
                "implicitTiling": {"availableLevels": 6, "subdivisionScheme": "OCTREE", "subtreeLevels": 6,
                                   "subtrees": {"uri": "st/{level}-{x}-{y}-{z}.subtree"}}, "refine": "ADD"}
        mm = {"tileset.json": cjson(tileset(root)), "st/0-0-0-0.subtree": subtree_binary(cjson(js), b"")}
        if cid == "R8":
            mm["c/a.glb"] = glb_tile_point([0.0, 0.0, 0.0])
        else:
            for level in range(6):
                for x in range(1 << level):
                    for y in range(1 << level):
                        for z in range(1 << level):
                            mm[f"c/{level}-{x}-{y}-{z}.glb"] = glb_tile_point(cell(box, True, level, x, y, z)[0:3])
        case(cid, mm, exp, large=True,
             ref={"R8": "R8 (DC-1: variables eliminated, at root admission)",
                  "R8a": "R8a (variables survive; normalized length 25 B)",
                  "R8a.nopad": "R8a timing companion without padding"}[cid],
             facts=("equal", "R8a.nopad") if cid == "R8a" else None,
             question="Q4" if cid != "R8" else None)


# ---------------------------------------------------------------------------------------------------------------
# W9: #151 fixtures (schema.id corrected) and O-authored link-free variants

def load_151(scheme):
    rel = f"bench/architecture_audit/implicit_rewrite/a2_semantics/{scheme}.json"
    raw = open(os.path.join(REPO, rel), "rb").read()
    assert hashlib.sha256(raw).hexdigest() == INPUTS[rel], rel
    return {k: base64.b64decode(v) for k, v in json.loads(raw)["members_base64"].items()}


def fix_schema_id(members):
    """The coordinator's correction (probes/materialize_a2_fixture.py): schema.id = "a2"; n0.json -> tileset.json."""
    out = dict(members)
    out["tileset.json"] = out.pop("n0.json")
    for name in list(out):
        if name.endswith(".json"):
            doc = json.loads(out[name])
            if isinstance(doc.get("schema"), dict) and "id" not in doc["schema"]:
                doc["schema"]["id"] = "a2"
                out[name] = json.dumps(doc, separators=(",", ":")).encode()
    return out


def rename_class(members):
    out = {}
    for name, data in members.items():
        if name.endswith(".json"):
            data = data.replace(b'"a2Tile"', b'"rustyTile"')
        elif name.endswith(".subtree"):
            jlen = int.from_bytes(data[8:16], "little")
            js = data[24:24 + jlen].replace(b'"a2Tile"', b'"rustyTile"')
            js = js + b" " * (jlen - len(js))
            data = data[:24] + js + data[24 + jlen:]
        out[name] = data
    return out


def relayout(member, drop_layers=(), to_f32=False):
    """Re-encode a #151 binary subtree: drop content layers, optionally convert the a2Tile columns to FLOAT32."""
    jlen = int.from_bytes(member[8:16], "little")
    js = json.loads(member[24:24 + jlen])
    binc = member[24 + jlen:]
    views = [binc[v["byteOffset"]:v["byteOffset"] + v["byteLength"]] for v in js["bufferViews"]]
    if to_f32:
        for t in js.get("propertyTables", []):
            for col in t["properties"].values():
                data = views[col["values"]]
                vals = struct.unpack("<%dd" % (len(data) // 8), data)
                assert all(struct.unpack("<f", struct.pack("<f", v))[0] == v for v in vals)
                views[col["values"]] = struct.pack("<%df" % len(vals), *vals)
    ca = [c for i, c in enumerate(js.get("contentAvailability", [])) if i not in drop_layers]
    if "contentAvailability" in js:
        if ca:
            js["contentAvailability"] = ca
        else:
            del js["contentAvailability"]
    buf = bytearray()
    for i, v in enumerate(views):
        off = len(buf) + (-len(buf) % 8)
        buf.extend(b"\x00" * (off - len(buf)))
        js["bufferViews"][i]["byteOffset"] = off
        js["bufferViews"][i]["byteLength"] = len(v)
        buf.extend(v)
    js["buffers"][0]["byteLength"] = len(buf)
    return subtree_binary(cjson(js), bytes(buf))


def family_w():
    for scheme in ("octree", "quadtree"):
        base = fix_schema_id(load_151(scheme))
        case(f"W9u.{scheme}-none", dict(base), "Unsupported", ref="W9u (#151 fixture, schema.id corrected)")
        case(f"W9u.{scheme}-rename-class", rename_class(base), "Unsupported", ref="W9u (#151, class renamed)")
        # Link-free variants: each node document as a standalone tileset, the JSON slot removed.
        raw = fix_schema_id(load_151(scheme))
        docs = {"n0": raw["tileset.json"]}
        for k in ("n1", "n2", "n3", "n4"):
            docs[k] = raw[f"{k}.json"]
        for k, data in docs.items():
            for f32_tag in ("", ".f32"):
                doc = json.loads(data)
                if "id" not in doc.get("schema", {}):
                    doc["schema"]["id"] = "a2"
                root = doc["root"]
                slots = root.pop("contents", None) or [root.pop("content")]
                drop = [i for i, c in enumerate(slots) if c["uri"].endswith(".json")]
                keep = [c for i, c in enumerate(slots) if i not in drop]
                if len(keep) > 1:
                    root["contents"] = keep
                elif keep:
                    root["content"] = keep[0]
                if f32_tag:
                    for p in doc["schema"]["classes"]["a2Tile"]["properties"].values():
                        p["componentType"] = "FLOAT32"
                members = {"tileset.json": cjson(doc)}
                sub_t = root["implicitTiling"]["subtrees"]["uri"]
                for name, blob in raw.items():
                    if name.endswith(".subtree") and name.startswith(sub_t.split("-{")[0] + "-"):
                        members[name] = relayout(blob, drop, bool(f32_tag))
                for c in keep:
                    for name, blob in raw.items():
                        if name.endswith(".glb") and name.startswith(c["uri"].split("-{")[0] + "-"):
                            members[name] = blob
                case(f"W9'.{scheme}-{k}{f32_tag}", members, "Accepted",
                     facts=("equal", f"W9'.{scheme}-{k}") if f32_tag else None,
                     ref="W9' (O-authored link-free a2Tile variant" + (", FLOAT32)" if f32_tag else ")"))


# ---------------------------------------------------------------------------------------------------------------
# VS-B-8 (amendment 7): the availability index tables are charged to the live-path limit.  Each pair sets
# document_decoded_bytes to exactly (subtree member bytes + table charge): the cap accepts, cap - 1 is refused at the
# charge (the member read itself still fits).  The charge is derived from the formula, never from running anything:
#   4 * ceil(T/64)                                   when the tile availability is a bitstream (rank table)
#   + 4 * (P_t + 1) + 2 * E + 4 * sum(popcount(v))   when E > 0, over distinct bitstream content sources v
# A 4 KiB unused view pads the member so that the cap stays above any one C1 payload view budget.

def vsb_archive(scheme, L, tiles, slots, force_bitstream):
    pad_view = b"\x00" * 4096
    b = Implicit(scheme, L, L, BQ_BOX, 64.0, tiles, slots)
    b.rows_box = b.rows_error = False
    b.force_bitstream = force_bitstream
    b.tile_count = False
    b.alias_content = True            # slots with equal bit sets share one bufferView (one source)
    b.subtree_hooks = [lambda irs: [ir.extra_views.append(pad_view) for ir in irs.values()]]
    return implicit_archive(b)


def family_vsb():
    def pair(cid, m, charge, what, note):
        member = len(m[ROOT_SUB])
        cap = member + charge
        full = f"{note}; member {member} B + tables {charge} B = cap {cap}"
        case(cid, m, "Accepted", limits={"document_decoded_bytes": cap}, ref=f"VS-B-8 {what} (cap = member + tables)",
             note=full, file=cid)
        case(f"{cid}-1", m, "ResourceLimit:document_decoded_bytes", limits={"document_decoded_bytes": cap - 1},
             ref=f"VS-B-8 {what} (one less: the table charge is refused)", note=full, file=cid)

    # VS-B-8.1: bitstream tile availability, no content: rank table only.  QUADTREE L = 4: T = 85, 4 * ceil(85/64) = 8.
    T = offset(4, 4)
    assert T == 85
    tiles = [(0, 0, 0, 0), (1, 0, 0, 0), (1, 1, 0, 0), (2, 0, 0, 0), (3, 0, 0, 0)]
    pair("VSB8.rank", vsb_archive("QUADTREE", 4, tiles, [], True), 4 * ((T + 63) // 64),
         "rank table only", "QUADTREE L=4 A=4, T=85, E=0, bitstream tiles")

    # VS-B-8.2: content slots 1 and 2 alias one bitstream source (X, popcount 3), slot 3 uses another (Y, popcount 2).
    # QUADTREE L = 3: T = 21 (rank 4), P_t = 6, E = 3 + 3 + 2 = 8, distinct popcounts 3 + 2 = 5:
    # 4 + 4 * 7 + 2 * 8 + 4 * 5 = 68.  Counting the aliased slot twice would charge 4 * 8 = 32 for positions (+12).
    T = offset(4, 3)
    assert T == 21
    tiles = [(0, 0, 0, 0), (1, 0, 0, 0), (1, 1, 0, 0), (1, 0, 1, 0), (2, 0, 0, 0), (2, 1, 0, 0)]
    X = {(0, 0, 0, 0), (1, 0, 0, 0), (2, 1, 0, 0)}
    Y = {(1, 1, 0, 0), (2, 0, 0, 0)}
    slots = [Slot("c/a/{level}/{x}/{y}.glb", X), Slot("c/b/{level}/{x}/{y}.glb", X),
             Slot("c/c/{level}/{x}/{y}.glb", Y)]
    P_t, E, pops = len(tiles), len(X) + len(X) + len(Y), len(X) + len(Y)
    assert (P_t, E, pops) == (6, 8, 5)
    pair("VSB8.alias", vsb_archive("QUADTREE", 3, tiles, slots, True),
         4 * ((T + 63) // 64) + 4 * (P_t + 1) + 2 * E + 4 * pops,
         "offsets, slots and deduplicated positions", "QUADTREE L=3 A=3, T=21, P_t=6, E=8, slots 1-2 alias X(3), slot 3 Y(2)")

    # VS-B-8.3: constant-1 tile availability (every tile available) with a bitstream content source: no rank table.
    # QUADTREE L = 2: T = 5 = P_t, one slot with 3 content tiles: 4 * 6 + 2 * 3 + 4 * 3 = 42.
    T = offset(4, 2)
    assert T == 5
    tiles = [(0, 0, 0, 0), (1, 0, 0, 0), (1, 1, 0, 0), (1, 0, 1, 0), (1, 1, 1, 0)]
    X = {(0, 0, 0, 0), (1, 0, 0, 0), (1, 1, 1, 0)}
    P_t, E, pops = len(tiles), len(X), len(X)
    assert P_t == T
    pair("VSB8.const", vsb_archive("QUADTREE", 2, tiles, [Slot("c/{level}/{x}/{y}.glb", X)], False),
         4 * (P_t + 1) + 2 * E + 4 * pops, "constant tiles, content bitstream (no rank table)",
         "QUADTREE L=2 A=2, tiles constant 1, P_t=5, E=3")


# ---------------------------------------------------------------------------------------------------------------
# Sanity: an explicit-only archive with the O payload (outside I1; checks that the payload passes C1)

def family_z():
    doc = tileset({"boundingVolume": {"box": BQ_BOX}, "content": {"uri": "a.glb"}, "geometricError": 0.0,
                   "refine": "ADD"})
    case("Z-payload", {"tileset.json": cjson(doc), "a.glb": glb_tile_point([16.0, 8.0, 2.0])}, "Undecided",
         production="Accepted", ref="O-extra: explicit-only archive carrying the oracle's minimal GLB "
         "(production must accept it before any control relies on payload validity)")


# ---------------------------------------------------------------------------------------------------------------

def diff(a, b):
    """Member-level difference between two archives: changed members and the bit Hamming distance when equal-size."""
    out = {"added": sorted(set(b) - set(a)), "removed": sorted(set(a) - set(b)), "changed": {}}
    for k in sorted(set(a) & set(b)):
        if a[k] != b[k]:
            if len(a[k]) == len(b[k]):
                out["changed"][k] = sum(bin(x ^ y).count("1") for x, y in zip(a[k], b[k]))
            else:
                out["changed"][k] = f"length {len(a[k])} -> {len(b[k])}"
    return out


def build(large):
    CASES.clear()
    family_a()
    family_b()
    family_c()
    family_d()
    family_t()
    family_r(large)
    family_vsb()
    family_w()
    family_z()
    by_id = {c["id"]: c for c in CASES}
    assert len(by_id) == len(CASES), "duplicate case ids"
    files = {}
    for c in CASES:
        fid = c["file"] or c["id"]
        if fid in files:
            assert files[fid]["members"] == c["members"], fid
            c["archive"] = files[fid]["archive"]
        else:
            data = write_3tz(c["members"])
            files[fid] = {"members": c["members"], "archive": data}
            c["archive"] = data
        c["fileId"] = fid
        if c["pair"] and c["pair"] in by_id:
            c["diff"] = diff(by_id[c["pair"]]["members"], c["members"])
    return files


def main(argv):
    large = "--large" in argv
    check = "--check" in argv
    files = build(large)
    out_small = os.path.join(ROOT, "controls", "fixtures")
    out_large = os.path.join(ROOT, "controls", "large")
    manifest = {"generator": "witnesses/generate.py", "contract": CONTRACT_SHA256, "inputs": INPUTS,
                "large": large, "cases": []}
    for c in CASES:
        subdir = "large" if c["large"] else "fixtures"
        path = f"{subdir}/{c['fileId']}.3tz"
        entry = {"id": c["id"], "file": path, "sha256": hashlib.sha256(c["archive"]).hexdigest(),
                 "bytes": len(c["archive"]), "members": len(c["members"]), "limits": c["limits"],
                 "expected": c["expected"], "ref": c["ref"]}
        for k in ("pair", "note", "question", "production", "facts", "diff"):
            if c.get(k):
                entry[k] = c[k]
        manifest["cases"].append(entry)
    mpath = os.path.join(ROOT, "controls", "manifest.json")
    if check:
        old = json.load(open(mpath))
        want = {e["id"]: e["sha256"] for e in old["cases"]}
        got = {e["id"]: e["sha256"] for e in manifest["cases"]}
        bad = [k for k in got if k in want and want[k] != got[k]]
        missing = [k for k in want if k not in got and (large or not k.startswith("R7"))]
        print(json.dumps({"checked": len(got), "mismatch": bad, "missing": missing}))
        return 1 if bad else 0
    os.makedirs(out_small, exist_ok=True)
    if large:
        os.makedirs(out_large, exist_ok=True)
    written = set()
    for c in CASES:
        d = out_large if c["large"] else out_small
        p = os.path.join(d, c["fileId"] + ".3tz")
        if p in written:
            continue
        written.add(p)
        with open(p, "wb") as f:
            f.write(c["archive"])
    if not large and os.path.exists(mpath):
        # Keep the recorded large entries when regenerating only the small set.
        old = json.load(open(mpath))
        manifest["cases"] += [e for e in old["cases"] if e["file"].startswith("large/")]
        manifest["large"] = old.get("large", False)
    with open(mpath, "w") as f:
        json.dump(manifest, f, indent=1, sort_keys=True)
        f.write("\n")
    print(json.dumps({"cases": len(manifest["cases"]), "files": len(written)}))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
