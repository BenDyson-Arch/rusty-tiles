"""Exact, independent winding witnesses from verified descendant leaves.

Retain directed published border incidences and exact coincident triangles;
replace nearest-source area-dot tests, which do not survive folded surfaces.
The caller owns source-ID/leaf-order verification and builds the reference
tables only from those verified leaves. Shader NORMAL and producer census
facts are not orientation evidence. A border witnesses only its own direction:
the interior of every face without a unique coincident witness stays explicitly
uninspected. No continuous source-surface mapping is asserted.

Acceptance is bounded literal geometry, including reversal, aliasing, root
borders, descendant exclusion and ambiguous coincidence. Full archive audits
and source-table construction belong to the caller.
"""
import collections
import math
import struct
from spatial import f32_point


def _point(xyz):
    p = tuple(0.0 if x == 0 else float(x) for x in xyz)
    if len(p) != 3 or not all(map(math.isfinite, p)):
        raise ValueError('winding requires finite three-dimensional coordinates')
    return p


def shape(xyz):
    """Return sorted packed binary64 corners and permutation parity.

    Collinearity is tested with exact integers representing binary64 values,
    avoiding underflow, overflow and rounded cross-product cancellation.
    Cyclic corner permutations preserve sign; exchanging two reverses it.
    """
    points = tuple(_point(p) for p in xyz)
    if len(points) != 3:
        raise ValueError('winding requires three corners')
    ordered = sorted(points)
    if ordered[0] == ordered[1] or ordered[1] == ordered[2]:
        return None, 0
    u, v = [], []
    for k in range(3):
        ratios = [p[k].as_integer_ratio() for p in points]
        denominator = max(d for _, d in ratios)
        a, b, c = [n * (denominator // d) for n, d in ratios]
        u.append(b - a)
        v.append(c - a)
    if all(u[i] * v[j] == u[j] * v[i]
           for i, j in ((0, 1), (0, 2), (1, 2))):
        return None, 0
    inversions = sum(points[i] > points[j]
                     for i, j in ((0, 1), (0, 2), (1, 2)))
    return (b''.join(struct.pack('<3d', *p) for p in ordered),
            -1 if inversions % 2 else 1)


def _rows(store, sql, parameters):
    # spatial imports shape while constructing its source index. Keep this
    # dependency lazy, including the independent audit's work-limit exception.
    from spatial import MAX_QUERY_CANDIDATES, WorkLimit
    for count, row in enumerate(store.db.execute(sql, parameters), 1):
        store.work.charge()
        if count > MAX_QUERY_CANDIDATES:
            raise WorkLimit('winding query candidate limit')
        yield row


def _point_ids(store, point):
    packed = struct.pack('<3d', *point)
    # The source formula has already established these welds. An accidental
    # delivered-coordinate alias never creates a source topology group.
    return {row[0] for row in _rows(store,
        'SELECT id FROM canonical_points WHERE xyz=?', (packed,))}


def _source_border(store, low, high, lo, hi, *, details=False):
    """Sum incidences after rendered aliases, without endpoint products."""
    from spatial import MAX_QUERY_CANDIDATES, WorkLimit
    left, right = _point_ids(store, low), _point_ids(store, high)
    if not left or not right:
        return (0,0,False,[]) if details else (0,0)
    endpoints, others = (left, right) if len(left) <= len(right) else (right, left)
    seen = set()
    forward = reverse = candidates = 0
    supports={}
    for endpoint in endpoints:
        for a, b, leaf, nf, nr, incidence in _rows(store,
                'SELECT e.a,e.b,e.leaf,e.nf,e.nr,i.faces FROM canonical_edges e '
                'JOIN incidence i ON i.a=e.a AND i.b=e.b WHERE e.a=? UNION ALL '
                'SELECT e.a,e.b,e.leaf,e.nf,e.nr,i.faces FROM canonical_edges e '
                'JOIN incidence i ON i.a=e.a AND i.b=e.b WHERE e.b=?',
                (endpoint, endpoint)):
            # Charge and cap every returned row, including duplicate rows and
            # other descendants, across all endpoint probes for this edge.
            candidates += 1
            if candidates > MAX_QUERY_CANDIDATES:
                raise WorkLimit('winding edge candidate limit')
            other = b if a == endpoint else a
            if other not in others or not lo <= leaf < hi:
                continue
            key = a, b, leaf
            if key in seen:
                continue
            seen.add(key)
            supports[a,b]=incidence
            if a in left:
                forward += nf
                reverse += nr
            else:
                forward += nr
                reverse += nf
    if details:
        return forward,reverse,bool(supports) and all(n==1 for n in supports.values()),list(supports)
    return forward,reverse


def inspect(store, tiles, require):
    """Check exact border/coincident witnesses for all rendered coarse tiles.

    Counts of borders concern edges; coincidence/uninspected counts concern
    faces. Ambiguous coincident signs permit either sign, certify neither, and
    remain in uninspectedInteriorOrientation. Border witnesses never remove a
    face from that count. Degenerate faces have no orientation witness.
    """
    if not hasattr(store,'canonical_stats'):
        store.finalize_canonical(require)
    stats = dict(checkedTiles=0, checkedFaces=0, borderedEdges=0,
                 checkedBorders=0, unwitnessedBorders=0,
                 sourceOpenBordersExempt=0, exemptWindingEvents=0,
                 checkedDirectedEdges=0,
                 coincidentFaces=0, ambiguousCoincidentFaces=0,
                 degenerateFaces=0, uninspectedFaces=0,
                 uninspectedBorderFaces=0, uninspectedInteriorFaces=0,
                 uninspectedInteriorOrientation=0)
    for tile in tiles:
        if not tile['children'] or not tile['rendered']:
            continue
        stats['checkedTiles'] += 1
        tileid, lo, hi = int(tile['id']), tile['lo'], tile['hi']
        edges = collections.defaultdict(lambda: [0, 0])
        for face in store.faces(tileid):
            store.work.charge()
            points = [_point(p) for p in face['xyz']]
            if len(points) != 3:
                raise ValueError('winding requires three corners')
            if len(set(f32_point(p) for p in points))<3:
                continue
            for k in range(3):
                store.work.charge()
                a, b = points[k], points[(k + 1) % 3]
                key = (a, b) if a < b else (b, a)
                entry = edges[key]
                entry[0] += 1
                entry[1] += 1 if a < b else -1 if a > b else 0
        borders = set()
        for (a, b), (count, direction) in edges.items():
            store.work.charge()
            if a == b:
                continue
            border=count==1
            if border:
                borders.add((a,b))
                stats['borderedEdges']+=1
            nf,nr,all_open,supports = _source_border(store,a,b,lo,hi,details=True)
            if not nf+nr:
                stats['unwitnessedBorders']+=int(border)
                continue
            allowed=int(nf>0)|int(nr>0)<<1
            actual=int(count+direction>0)|int(count-direction>0)<<1
            if all_open:
                stats['sourceOpenBordersExempt']+=int(border)
                if actual & ~allowed:
                    for x,y in supports:
                        changed=store.db.execute('INSERT OR IGNORE INTO exemptions VALUES (?,?,?,?)',(tileid,x,y,'winding')).rowcount
                        stats['exemptWindingEvents']+=changed
                continue
            stats['checkedDirectedEdges']+=1
            stats['checkedBorders']+=int(border)
            # Direction is an allowance, never a multiplicity or retention
            # requirement. Source mask 3 permits actual 1,2 or3; mask1 may
            # not acquire mask2, including an extra reversed interior face.
            require(not actual & ~allowed,'interfaceWinding',
                    f'tile {tile["id"]}: disallowed exact edge direction {a} -> {b}; source mask {allowed}, actual mask {actual}')
        # Stream again rather than retaining all face coordinates with edges.
        for face in store.faces(tileid):
            store.work.charge()
            stats['checkedFaces'] += 1
            packed, sign = shape(face['xyz']) if len(set(f32_point(p) for p in face['xyz']))==3 else (None,0)
            signs = set()
            if packed is not None:
                for (source_sign,) in _rows(store,
                        'SELECT sign FROM source_shapes '
                        'WHERE shape=? AND leaf>=? AND leaf<?',
                        (packed, lo, hi)):
                    if source_sign not in (-1, 1):
                        raise ValueError('invalid verified source orientation')
                    signs.add(source_sign)
            else:
                stats['degenerateFaces'] += 1
            if signs:
                stats['coincidentFaces'] += 1
                require(sign in signs, 'interfaceWinding',
                        f'tile {tile["id"]}: reversed exact coincident triangle')
                if len(signs) == 1:
                    continue
                stats['ambiguousCoincidentFaces'] += 1
            stats['uninspectedFaces'] += 1
            stats['uninspectedInteriorOrientation'] += 1
            points = [_point(p) for p in face['xyz']]
            on_border = any(tuple(sorted((points[k], points[(k + 1) % 3])))
                            in borders for k in range(3))
            stats['uninspectedBorderFaces' if on_border
                  else 'uninspectedInteriorFaces'] += 1
    stats['uninspected'] = [
        'isolated interior orientation without unique exact coincident '
        'descendant-leaf geometry is uninspected; directed border witnesses '
        'certify only retained edges, without a continuous source-surface mapping'
    ]
    return stats
