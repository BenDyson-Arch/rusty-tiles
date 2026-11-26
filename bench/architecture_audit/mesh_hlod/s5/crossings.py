"""Coarse self-intersections (#420, owner decision 2026-11-20), counted from published bytes.

Two distinct faces of one coarse tile may meet only in the hull of their common corners
(equal f32 keys): nowhere with none, at the vertex with one, along the edge with two.
Each pair whose closed hulls meet anywhere else is counted, and split into *present*
pairs (both faces are faces of the tile's immediate children; leaf children are the
source) and *new* ones. Faces have three distinct keys; three collinear keys are their
segment; faces are distinct by corner set (a repeated or reversed triangle, or a placement
repeated at the same position, is one face), and two faces with the same three keys are a
fold, not counted here. "Present" is judged against the children together. The census
reports and never refuses.

This is an independent construction of the native census: the intersection of the two
closed hulls is clipped exactly (Fractions of the f32 values) as a convex vertex set, and
every vertex must lie in the hull of the common corners. The native validator decides the
same relation with orientation predicates and a case analysis.

Each tile has its own work budget (a fixed allowance and a share per face, as natively);
a tile that exhausts it is counted as incomplete, without partial counts.
"""
import hashlib
import struct
from fractions import Fraction

LISTED = 16
# The native census's budget formula and weights. The two find candidates differently
# (native: a grid; here: a sweep along x), so near the budget their incomplete tiles can
# differ; the controls stay far below it. Native also bounds each tile's workspace (half
# the 128 MiB decoded-document allowance, about 28,600 face entries); the oracle has no
# such bound.
TILE_FLOOR = 1 << 24
FACE_UNITS = 1 << 15
CANDIDATE_UNITS = 1
EXACT_UNITS = 64


def key(p):
    return tuple(0.0 if v == 0 else v for v in struct.unpack('<3f', struct.pack('<3f', *p)))


def _sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def _dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def _cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def _clip(points, normal, offset):
    """Sutherland-Hodgman: the cyclic vertex list of a convex set clipped to
    normal . x <= offset (closed), exact."""
    out = []
    n = len(points)
    for i in range(n):
        p, q = points[i], points[(i + 1) % n]
        dp, dq = _dot(normal, p) - offset, _dot(normal, q) - offset
        if dp <= 0:
            out.append(p)
        if (dp < 0 < dq) or (dq < 0 < dp):
            t = dp / (dp - dq)
            out.append(tuple(a + t * (b - a) for a, b in zip(p, q)))
    return out


def _halfspaces(corners):
    """Closed half-spaces (normal, offset) whose intersection is the hull of `corners`
    (three distinct points: a triangle, or a collinear segment)."""
    a, b, c = corners
    n = _cross(_sub(b, a), _sub(c, a))
    if any(n):
        out = [(n, _dot(n, a)), (tuple(-x for x in n), -_dot(n, a))]
        for p, q, r in ((a, b, c), (b, c, a), (c, a, b)):
            m = _cross(n, _sub(q, p))  # in-plane, perpendicular to pq
            if _dot(m, _sub(r, p)) > 0:
                m = tuple(-x for x in m)
            out.append((m, _dot(m, p)))
        return out
    # Collinear: the line through the outer two, and the slab between them.
    d = next(_sub(q, p) for p, q in ((a, b), (a, c)) if any(_sub(q, p)))
    axis = min(range(3), key=lambda k: abs(d[k]))
    e = tuple(Fraction(int(k == axis)) for k in range(3))
    n1 = _cross(d, e)
    n2 = _cross(d, n1)
    out = []
    for m in (n1, n2):
        out += [(m, _dot(m, a)), (tuple(-x for x in m), -_dot(m, a))]
    values = [_dot(d, p) for p in corners]
    out += [(d, max(values)), (tuple(-x for x in d), -min(values))]
    return out


def _in_hull(p, common):
    if not common:
        return False
    if len(common) == 1:
        return p == common[0]
    u, v = common
    if any(_cross(_sub(v, u), _sub(p, u))):
        return False
    t = _dot(_sub(p, u), _sub(v, u))
    return 0 <= t <= _dot(_sub(v, u), _sub(v, u))


def improper(f, g):
    """Faces f, g (three distinct f32 keys each) meet outside the hull of their common keys."""
    common = [k for k in f if k in g]
    if len(common) == 3:
        return False
    exact = lambda face: [tuple(Fraction(x) for x in p) for p in face]
    shape = exact(f)
    for normal, offset in _halfspaces(exact(g)):
        shape = _clip(shape, normal, offset)
        if not shape:
            return False
    return any(not _in_hull(p, exact(common)) for p in shape)


def _box(face):
    return [min(p[k] for p in face) for k in range(3)], [max(p[k] for p in face) for k in range(3)]


def pairs(faces, budget):
    """Improper pairs (i, j), i < j, or None past `budget` work units. Candidates are the
    pairs whose boxes overlap, found by a sweep along x."""
    boxes = [_box(f) for f in faces]
    order = sorted(range(len(faces)), key=lambda i: boxes[i][0][0])
    work, found = 0, []
    for n, i in enumerate(order):
        for j in order[n + 1:]:
            if boxes[j][0][0] > boxes[i][1][0]:
                break
            work += CANDIDATE_UNITS
            if work > budget:
                return None
            if all(boxes[i][0][k] <= boxes[j][1][k] and boxes[j][0][k] <= boxes[i][1][k] for k in range(3)):
                work += EXACT_UNITS
                if work > budget:
                    return None
                if improper(faces[i], faces[j]):
                    found.append((min(i, j), max(i, j)))
    return sorted(found)


def inspect(store, tiles, archive):
    digests = {t['id']: tuple(sorted(hashlib.sha256(archive.read(u)).digest() for u in t['contents']))
               for t in tiles}

    def children(t, digest):
        out = []
        for c in t['children']:
            if not c['contents'] or digests[c['id']] == digest:
                out.extend(children(c, digest))
            else:
                out.append(c)
        return out

    def faces_of(t):
        """Distinct faces by corner set, in first-entry order: a repeated placement or a
        repeated or reversed triangle is one face."""
        out, seen = [], set()
        for f in store.faces(int(t['id'])):
            ks = tuple(key(v) for v in f['xyz'])
            if len(set(ks)) == 3 and tuple(sorted(ks)) not in seen:
                seen.add(tuple(sorted(ks)))
                out.append(ks)
        return out

    def content_below(t):
        return any(c['contents'] or content_below(c) for c in t['children'])

    result = dict(tiles=0, faces=0, incompleteTiles=0, crossingTiles=0, newTiles=0, presentPairs=0,
                  newPairs=0, presentPairsByDepth=[], newPairsByDepth=[], listedTiles=[])
    listed = []
    for t in tiles:
        # A coarse tile has content below it (native's leaf rule).
        if not t['contents'] or not content_below(t):
            continue
        faces = faces_of(t)
        found = pairs(faces, TILE_FLOOR + FACE_UNITS * len(faces))
        if found is None:
            result['incompleteTiles'] += 1
            continue
        result['tiles'] += 1
        result['faces'] += len(faces)
        # The children together: the coarse tile replaces them together, and leaf children
        # together are the source.
        child = {tuple(sorted(f)) for c in children(t, digests[t['id']]) for f in faces_of(c)}
        present = sum(1 for i, j in found
                      if tuple(sorted(faces[i])) in child and tuple(sorted(faces[j])) in child)
        new = len(found) - present
        if not found:
            continue
        result['crossingTiles'] += 1
        result['newTiles'] += new > 0
        depth = t['depth']
        for name in ('presentPairsByDepth', 'newPairsByDepth'):
            result[name] += [0] * (depth + 1 - len(result[name]))
        result['presentPairsByDepth'][depth] += present
        result['newPairsByDepth'][depth] += new
        result['presentPairs'] += present
        result['newPairs'] += new
        listed.append(dict(tile=int(t['id']), depth=depth, faces=len(faces), geometricError=t['g'],
                           presentPairs=present, newPairs=new))
    listed.sort(key=lambda d: (-d['newPairs'], -d['presentPairs'], d['tile']))
    result['listedTiles'] = listed[:LISTED]
    return result
