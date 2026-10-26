"""D11 scalable borders and S1b audit receipts, re-derived from source and archive.

Source canonical incidence (``spatial.Store``: source manifest plus verified
leaves) defines each coarse tile's shared border: leaf edges and points with an
owner leaf inside its interval and one outside. Retained exact vertices split it
into arcs; every arc is delivered as exactly one chord whose directions are
witnessed by every source edge it replaces. A contracted vertex is placed on its
chord at its original edge ordinal; displacements and gaps are exact Fractions.

The S1b audit sidecar (``borders.jsonl``, schema 1) is an index only: payload
SHA-256 joins a record to its archive tile, and its logical depth, certificates
and receipts are claims. Depth labels are first validated against every relation
the tree proves (cell siblings equal, strictly deeper below a labelled cell).
Then, from geometry: E_d from the captured source extent, same-depth equality and
mixed-depth gaps at every shared source breakpoint, terminal contacts (a leaf not
proven deeper than the tile forces an exact contact), each certificate's hinted
chain re-walked through source incidence, epsilon against the exact arc
displacement, complete one-to-one arc coverage, and topology re-derived from the
parent job's decoded input: dropped components and their measured proxy distance,
closures and openings per boundary cluster (cycle rank) with measured loss, the
G contributions, contractedVertices and the complete omission frontier.

Not proven here: the history-dependent receipt counters listed in the result's
``uninspected``, a continuous source map behind licensed coincident faces, and
S2's sampled two-sided error.
"""
import collections
import hashlib
import json
import math
import struct
import time
from fractions import Fraction

from spatial import f32_point, WorkLimit, MAX_QUERY_CANDIDATES


def _key(a, b):
    return (a, b) if a < b else (b, a)


def _along(mask, x, y):
    """Mask bit 0 means lower id to higher; returned bit 0 means x -> y."""
    return mask & 3 if x < y else ((mask & 1) << 1) | ((mask >> 1) & 1)


def _fractions(p):
    return tuple(Fraction(v) for v in p)


def _lerp(a, b, i, m):
    t = Fraction(i, m)
    return tuple(x + (y - x) * t for x, y in zip(a, b))


def _distance2(p, q):
    return sum((x - y) ** 2 for x, y in zip(p, q))


# ---------------------------------------------------------------------------
# Per-tile arcs (called from Store.census, before any sidecar is read).

def prepare(store):
    store.db.executescript('''
      CREATE TABLE IF NOT EXISTS border_arcs(tile INTEGER,a INTEGER,b INTEGER,m INTEGER,
        path TEXT,d2 TEXT,PRIMARY KEY(tile,a,b)) WITHOUT ROWID;
      CREATE TABLE IF NOT EXISTS border_images(point INTEGER,tile INTEGER,lower INTEGER,
        upper INTEGER,i INTEGER,m INTEGER,PRIMARY KEY(point,tile)) WITHOUT ROWID;
    ''')


def tile_border(store, tile, vertices, directed, require, stats):
    """Arcs of one rendered coarse tile; refuses lost junctions, chords or directions.

    ``vertices`` holds the tile's rendered corner tuples and ``directed`` counts
    its nondegenerate directed edges by corner tuples."""
    started = time.process_time()
    try:
        return _tile_border(store, tile, vertices, directed, require, stats)
    finally:
        stats['arcCpuSeconds'] += time.process_time() - started


def _tile_border(store, tile, vertices, directed, require, stats):
    tid, lo, hi = int(tile['id']), tile['lo'], tile['hi']
    edges, points = {}, set()
    for kind, a, b in store.db.execute(
            'SELECT kind,a,b FROM requirements WHERE tile=? AND kind<2', (tid,)).fetchall():
        store.work.charge()
        if kind == 0:
            points.add(a)
        else:
            edges[a, b] = 0
    if not edges and not points:
        return {}
    for a, b in edges:
        store.work.charge()
        nf, nr = store.db.execute(
            'SELECT coalesce(sum(nf),0),coalesce(sum(nr),0) FROM canonical_edges '
            'WHERE a=? AND b=? AND leaf>=? AND leaf<?', (a, b, lo, hi)).fetchone()
        edges[a, b] = int(nf > 0) | int(nr > 0) << 1
    adjacent = collections.defaultdict(list)
    for a, b in edges:
        adjacent[a].append(b)
        adjacent[b].append(a)
    cache = {}

    def xyz(p):
        if p not in cache:
            cache[p] = store.canonical_xyz(p)
        return cache[p]

    def retained(p):
        return xyz(p) in vertices

    for p in points | set(adjacent):
        store.work.charge()
        require(len(adjacent.get(p, ())) == 2 or retained(p), 'interfacePoints',
                f'tile {tid}: decoded ancestor lost an exact border junction or point contact {xyz(p)}')
    visited, chords, contracted = set(), set(), {}
    arcs, images = [], []
    for start in sorted(adjacent):
        if not retained(start):
            continue
        for first in adjacent[start]:
            if _key(start, first) in visited:
                continue
            path, at, nxt = [start], start, first
            while True:
                store.work.charge()
                edge = _key(at, nxt)
                if edge in visited:
                    raise ValueError('border arc revisits an edge')
                visited.add(edge)
                path.append(nxt)
                if retained(nxt):
                    break
                around = adjacent[nxt]
                at, nxt = nxt, around[1] if around[0] == at else around[0]
            m = len(path) - 1
            a, b = path[0], path[-1]
            require(a != b, 'interfaceSegments',
                    f'tile {tid}: a closed shared leaf arc keeps one decoded vertex')
            chord = _key(a, b)
            require(chord not in chords, 'interfaceSegments',
                    f'tile {tid}: two shared leaf arcs share one decoded chord')
            chords.add(chord)
            along, against = directed.get((xyz(a), xyz(b)), 0), directed.get((xyz(b), xyz(a)), 0)
            require(along + against > 0, 'interfaceSegments',
                    f'tile {tid}: decoded ancestor lacks the chord of an exact shared leaf arc {xyz(a)} -> {xyz(b)}')
            allowed = 3
            for x, y in zip(path, path[1:]):
                allowed &= _along(edges[_key(x, y)], x, y)
            actual = int(along > 0) | int(against > 0) << 1
            require(not actual & ~allowed, 'interfaceWinding',
                    f'tile {tid}: chord direction {actual} is not witnessed along its source arc ({allowed})')
            A, B = _fractions(xyz(a)), _fractions(xyz(b))
            d2 = Fraction(0)
            lower, upper = chord
            for i in range(1, m):
                store.work.charge()
                d2 = max(d2, _distance2(_fractions(xyz(path[i])), _lerp(A, B, i, m)))
                images.append((path[i], tid, lower, upper, i if a == lower else m - i, m))
            contracted[chord] = path if a == lower else path[::-1]
            arcs.append((tid, lower, upper, m, json.dumps(path if a == lower else path[::-1]),
                         f'{d2.numerator}/{d2.denominator}'))
            if m > 1:
                stats['contractedBorderArcs'] += 1
                stats['contractedBorderVertices'] += m - 1
    require(len(visited) == len(edges), 'interfaceSegments',
            f'tile {tid}: decoded ancestor retains no vertex of a closed shared leaf arc')
    for p in points | set(adjacent):
        if retained(p):
            images.append((p, tid, None, None, 0, 0))
    store.db.executemany('INSERT INTO border_arcs VALUES (?,?,?,?,?,?)', arcs)
    store.db.executemany('INSERT INTO border_images VALUES (?,?,?,?,?,?)', images)
    stats['borderArcs'] += len(arcs)
    return contracted


def _wedge(a, b, c, e):
    """Exact sign of |d|^2 (u.w) - (u.d)(w.d), d = b - a, u = c - a, w = e - a: the
    wings towards c and e (perpendicular to the edge) form an acute wedge (> 0), a
    right angle (0) or an obtuse one (< 0). With c = e it is |d x u|^2."""
    a, b, c, e = (_fractions(p) for p in (a, b, c, e))
    d = [b[k] - a[k] for k in range(3)]
    u = [c[k] - a[k] for k in range(3)]
    w = [e[k] - a[k] for k in range(3)]
    dot = lambda x, y: sum(x[k] * y[k] for k in range(3))
    value = dot(d, d) * dot(u, w) - dot(u, d) * dot(w, d)
    return (value > 0) - (value < 0)


def _distinct_sides(a, b, faces):
    """Same-direction faces on a -> b lie on distinct sides: every pair forms a right
    or obtuse wedge and none is collinear with the edge (exact)."""
    thirds = [f[(j + 2) % 3] for f in faces for j in range(3) if (f[j], f[(j + 1) % 3]) == (a, b)]
    return all(_wedge(a, b, c, c) > 0 and all(_wedge(a, b, c, e) <= 0 for e in thirds[i + 1:])
               for i, c in enumerate(thirds))


def _twins(store):
    """Coincident verified source faces: canonical corner set -> leaves (one entry
    per face), the corner -> sets index and connected twin regions (sets sharing a
    corner). Built once from the source manifest's canonical incidence."""
    if not hasattr(store, 'twin_sets'):
        sets = collections.defaultdict(list)
        for leaf, a, b, c in store.db.execute(
                'SELECT s.leaf,x.canonical,y.canonical,z.canonical FROM source_corners c '
                'JOIN source s ON s.id=c.source JOIN canonical_map x ON x.point=c.a '
                'JOIN canonical_map y ON y.point=c.b JOIN canonical_map z ON z.point=c.c'):
            store.work.charge()
            if len({a, b, c}) == 3:
                sets[tuple(sorted((a, b, c)))].append(leaf)
        sets = {k: v for k, v in sets.items() if len(v) > 1}
        corners = collections.defaultdict(list)
        region = _UnionFind()
        for k in sets:
            for v in k:
                corners[v].append(k)
                region.join(('v', v), ('s', k))
        store.twin_sets, store.twin_corners, store.twin_region = sets, corners, region
    return store.twin_sets, store.twin_corners, store.twin_region


def folds(store, tile, directed, arcs, require, faces=()):
    """Directed incidences are counted, never reduced to a mask (S1b review 1).

    A coarse edge used n >= 2 times in one direction needs n verified descendant
    source faces that way on every source edge it stands for (itself, or each edge
    of its contracted arc). Otherwise it must keep the other direction and those
    faces must lie on distinct sides (exact wedge sign): a simplified book spine's
    perpendicular pages or a collapsed plate's opposite sheets. A fold, duplicate or
    leaning flap is refused, balanced or not. Coincident coarse faces (n on one
    corner set, either orientation) need, at every corner, a descendant coincident
    source set of at least n faces, all in one connected twin region."""
    tid, lo, hi = int(tile['id']), tile['lo'], tile['hi']
    ids = {}

    def canonical(p):
        if p not in ids:
            row = store.db.execute('SELECT id FROM canonical_points WHERE xyz=?',
                                   (struct.pack('<3d', *(0.0 if x == 0 else x for x in p)),)).fetchone()
            ids[p] = row[0] if row else None
        return ids[p]

    def support(p, q):
        a, b = canonical(p), canonical(q)
        if a is None or b is None:
            return 0
        path = arcs.get(_key(a, b), [a, b])
        if path[0] != a:
            path = path[::-1]
        least = None
        for x, y in zip(path, path[1:]):
            store.work.charge()
            nf, nr = store.db.execute(
                'SELECT coalesce(sum(nf),0),coalesce(sum(nr),0) FROM canonical_edges '
                'WHERE a=? AND b=? AND leaf>=? AND leaf<?', (*_key(x, y), lo, hi)).fetchone()
            n = nf if x < y else nr
            least = n if least is None else min(least, n)
        return least or 0
    for (p, q), n in directed.items():
        store.work.charge()
        if n < 2 or support(p, q) >= n:
            continue
        store.work.charge(n * n)
        require(directed.get((q, p), 0) > 0 and _distinct_sides(p, q, faces), 'interfaceWinding',
                f'tile {tid}: coarse edge {p} -> {q} used {n} times one way, overlapping without '
                'that descendant source multiplicity')
    corners = collections.Counter(tuple(sorted(f)) for f in faces)
    if not any(n > 1 for n in corners.values()):
        return
    sets, index, region = _twins(store)
    for c, n in corners.items():
        if n < 2:
            continue
        regions = None
        for p in c:
            v = canonical(p)
            here = set()
            for k in index.get(v, ()) if v is not None else ():
                store.work.charge()
                if sum(lo <= leaf < hi for leaf in sets[k]) >= n:
                    here.add(region(('s', k)))
            regions = here if regions is None else regions & here
        require(bool(regions), 'interfaceWinding',
                f'tile {tid}: {n} coincident coarse faces {c} without a coincident source set of that '
                'multiplicity at every corner, in one twin region')


class _UnionFind:
    def __init__(self):
        self.parent = {}

    def __call__(self, p):
        self.parent.setdefault(p, p)
        while self.parent[p] != p:
            self.parent[p] = self.parent[self.parent[p]]
            p = self.parent[p]
        return p

    def join(self, a, b):
        a, b = self(a), self(b)
        if a != b:
            self.parent[max(a, b)] = min(a, b)


class _Near:
    """Exact membership, or a point within ``reach`` per axis (codec tolerance)."""
    def __init__(self, points, reach):
        self.points, self.reach = points, reach
        self.cell = [max(x, 1e-9) for x in reach]
        self.grid = collections.defaultdict(list)
        if any(reach):
            for p in points:
                self.grid[self._cell(p)].append(p)

    def _cell(self, p):
        return tuple(int(math.floor(p[k] / self.cell[k])) for k in range(3))

    def find(self, p):
        q = self.nearest(p)
        return None if q is None else 'exact' if q == p else 'tolerance'

    def nearest(self, p):
        """The point itself, or one within ``reach``, or None."""
        if p in self.points:
            return p
        if not any(self.reach):
            return None
        c = self._cell(p)
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for dz in (-1, 0, 1):
                    for q in self.grid.get((c[0] + dx, c[1] + dy, c[2] + dz), ()):
                        if all(abs(p[k] - q[k]) <= self.reach[k] for k in range(3)):
                            return q
        return None


def _boundary(faces):
    """One-sided edges of nondegenerate faces, grouped into connected components."""
    count = collections.Counter()
    for f in faces:
        if len(set(f)) < 3:
            continue
        for k in range(3):
            count[_key(f[k], f[(k + 1) % 3])] += 1
    edges = [e for e, n in count.items() if n == 1]
    root = _UnionFind()
    for a, b in edges:
        root.join(a, b)
    loops = collections.defaultdict(lambda: (set(), []))
    for a, b in edges:
        loops[root(a)][0].update((a, b))
        loops[root(a)][1].append((a, b))
    return list(loops.values())


def _samples(f):
    """S1b's fixed controls on one triangle: the quarter grid and the centroid."""
    out = [tuple((f[0][k] * a + f[1][k] * b + f[2][k] * (4 - a - b)) / 4 for k in range(3))
           for a in range(5) for b in range(5 - a)]
    out.append(tuple(sum(p[k] for p in f) / 3 for k in range(3)))
    return out


def _closest(p, f):
    """Closest point of triangle f to p and its feature: ('v', i), ('e', i) for edge
    f[i]-f[i+1], or ('f',). Float; a measurement witness, not a certificate."""
    a, b, c = f
    sub = lambda x, y: [x[k] - y[k] for k in range(3)]
    dot = lambda x, y: sum(x[k] * y[k] for k in range(3))
    at = lambda base, u, s: tuple(base[k] + u[k] * s for k in range(3))
    ab, ac, ap = sub(b, a), sub(c, a), sub(p, a)
    d1, d2 = dot(ab, ap), dot(ac, ap)
    if d1 <= 0 and d2 <= 0:
        return a, ('v', 0)
    bp = sub(p, b)
    d3, d4 = dot(ab, bp), dot(ac, bp)
    if d3 >= 0 and d4 <= d3:
        return b, ('v', 1)
    vc = d1 * d4 - d3 * d2
    if vc <= 0 and d1 >= 0 and d3 <= 0 and d1 - d3 != 0:
        return at(a, ab, d1 / (d1 - d3)), ('e', 0)
    cp = sub(p, c)
    d5, d6 = dot(ab, cp), dot(ac, cp)
    if d6 >= 0 and d5 <= d6:
        return c, ('v', 2)
    vb = d5 * d2 - d1 * d6
    if vb <= 0 and d2 >= 0 and d6 <= 0 and d2 - d6 != 0:
        return at(a, ac, d2 / (d2 - d6)), ('e', 2)
    va = d3 * d6 - d5 * d4
    if va <= 0 and d4 - d3 >= 0 and d5 - d6 >= 0 and (d4 - d3) + (d5 - d6) != 0:
        return at(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))), ('e', 1)
    denom = va + vb + vc
    if denom == 0:
        return a, ('v', 0)
    v, w = vb / denom, vc / denom
    return tuple(a[k] + ab[k] * v + ac[k] * w for k in range(3)), ('f',)


class _Mesh:
    """A tile-sized face set: exact distances, projections and boundary tests."""
    def __init__(self, faces):
        self.faces = [f for f in faces if len(set(f)) == 3]
        count = collections.Counter(_key(f[k], f[(k + 1) % 3]) for f in self.faces for k in range(3))
        self.open = {e for e, n in count.items() if n == 1}
        self.open_points = {p for e in self.open for p in e}
        # Uniform grid over face boxes: about one face per cell.
        self.grid = collections.defaultdict(list)
        if self.faces:
            lo = [min(q[k] for f in self.faces for q in f) for k in range(3)]
            hi = [max(q[k] for f in self.faces for q in f) for k in range(3)]
            span = max(max(hi[k] - lo[k] for k in range(3)), 1e-30)
            self.lo, self.cell = lo, span / max(1, round(len(self.faces) ** (1 / 3)))
            for i, f in enumerate(self.faces):
                a = self._cell([min(q[k] for q in f) for k in range(3)])
                b = self._cell([max(q[k] for q in f) for k in range(3)])
                for x in range(a[0], b[0] + 1):
                    for y in range(a[1], b[1] + 1):
                        for z in range(a[2], b[2] + 1):
                            self.grid[x, y, z].append(i)

    def _cell(self, p):
        return tuple(int(math.floor((p[k] - self.lo[k]) / self.cell)) for k in range(3))

    def _ranked(self, p):
        """Candidate faces by increasing float distance; rings grow until the best
        lies within the searched cube (complete for the nearest face)."""
        if not self.faces:
            return []
        c, seen, out, ring = self._cell(p), set(), [], 0
        limit = max(len(self.grid), 1)
        while True:
            for x in range(c[0] - ring, c[0] + ring + 1):
                for y in range(c[1] - ring, c[1] + ring + 1):
                    for z in range(c[2] - ring, c[2] + ring + 1):
                        if max(abs(x - c[0]), abs(y - c[1]), abs(z - c[2])) != ring:
                            continue
                        for i in self.grid.get((x, y, z), ()):
                            if i in seen:
                                continue
                            seen.add(i)
                            f = self.faces[i]
                            q, feature = _closest(p, f)
                            out.append((sum((p[k] - q[k]) ** 2 for k in range(3)), q, feature, f))
            out.sort(key=lambda r: r[0])
            reach = ring * self.cell
            if out and out[0][0] <= reach * reach * 0.999:
                return out
            if len(seen) == len(self.faces) or ring > limit:
                return out
            ring += 1

    def distance(self, p):
        """Exact distance from p to the face set (float candidates, exact minimum)."""
        from oracle import distance2
        ranked = self._ranked(p)
        if not ranked:
            return math.inf
        bound = ranked[0][0] * (1 + 1e-9) + 1e-300
        best = min(distance2(p, f) for d, _, _, f in ranked if d <= bound * 4 + 1e-24)
        return math.sqrt(best)

    def project(self, p):
        ranked = self._ranked(p)
        return ranked[0][1] if ranked else p

    def over_boundary(self, p):
        """Whether p's nearest point on the face set is on a one-sided edge or vertex."""
        ranked = self._ranked(p)
        if not ranked:
            return False
        _, _, feature, f = ranked[0]
        if feature[0] == 'v':
            return f[feature[1]] in self.open_points
        if feature[0] == 'e':
            i = feature[1]
            return _key(f[i], f[(i + 1) % 3]) in self.open
        return False

    def near(self, points):
        """Faces with a corner within the bounding box of `points`, doubled."""
        if not points:
            return []
        lo = [min(p[k] for p in points) for k in range(3)]
        hi = [max(p[k] for p in points) for k in range(3)]
        pad = [max(hi[k] - lo[k], 1e-12) / 2 for k in range(3)]
        inside = lambda q: all(lo[k] - pad[k] <= q[k] <= hi[k] + pad[k] for k in range(3))
        return [f for f in self.faces if any(inside(q) for q in f)]


def _cycle_centroids(components):
    """Vertex centroids of a fundamental cycle basis of boundary components."""
    out = []
    for points, edges in components:
        adjacency = collections.defaultdict(list)
        for a, b in edges:
            adjacency[a].append(b)
            adjacency[b].append(a)
        parent, depth, order = {}, {}, []
        for start in points:
            if start in parent:
                continue
            parent[start], depth[start] = None, 0
            stack = [start]
            while stack:
                v = stack.pop()
                order.append(v)
                for w in adjacency[v]:
                    if w not in parent:
                        parent[w], depth[w] = v, depth[v] + 1
                        stack.append(w)
        tree = {_key(v, parent[v]) for v in parent if parent[v] is not None}
        for a, b in edges:
            if _key(a, b) in tree:
                continue
            path_a, path_b = [a], [b]
            while path_a[-1] != path_b[-1]:
                if depth[path_a[-1]] >= depth[path_b[-1]]:
                    path_a.append(parent[path_a[-1]])
                else:
                    path_b.append(parent[path_b[-1]])
            cycle = path_a + path_b[-2::-1]
            out.append(tuple(sum(p[k] for p in cycle) / len(cycle) for k in range(3)))
    return out


# ---------------------------------------------------------------------------
# Sidecar comparison.

def _bits(key):
    return struct.pack('<3f', *(struct.unpack('<f', struct.pack('<I', int(v)))[0] for v in key))


class Audit:
    def __init__(self, store, tiles, leaves, archive, require, extent):
        self.store, self.tiles, self.leaves = store, tiles, leaves
        self.archive, self.require, self.extent = archive, require, extent
        self.ids = {}
        self.stats = collections.Counter()

    def canonical(self, key):
        packed = _bits(key)
        if packed not in self.ids:
            row = self.store.db.execute('SELECT id FROM canonical_points WHERE key=?', (packed,)).fetchone()
            self.ids[packed] = row[0] if row else None
        return self.ids[packed]

    def scale(self, depth):
        return self.extent / 1024. * 2.0 ** -depth

    # -- joins ---------------------------------------------------------------
    def join(self, rows):
        require = self.require
        by_sha, omitted = {}, []
        for r in rows:
            require(r.get('schemaVersion') == 1, 'borderAudit', 'unknown audit schema')
            if r.get('kind') == 'tile':
                sha = r['tile']['sha256']
                require(sha not in by_sha, 'borderAudit', f'two audit records for payload {sha}')
                by_sha[sha] = r
            elif r.get('kind') in ('topologyAttempt', 'topologyConstruction'):
                # Additive location hints (audit v1): operations are re-derived from
                # the delivered surfaces and source, never read from these.
                self.stats[r['kind'] + 'Hints'] += 1
            else:
                require(r.get('kind') == 'omitted', 'borderAudit', f'unknown audit record kind {r.get("kind")!r}')
                omitted.append(r)
        used = set()
        self.records = {}
        for t in self.tiles:
            if not t['children'] or not t['contents']:
                continue
            require(len(t['contents']) == 1, 'oracleDomain', f'tile {t["id"]}: D11 audit needs one coarse payload')
            sha = hashlib.sha256(self.archive.read(t['contents'][0])).hexdigest()
            r = by_sha.get(sha)
            require(r is not None, 'borderAudit', f'tile {t["id"]}: no audit record for its payload')
            row = r['tile']
            require((row['leafBegin'], row['leafEnd']) == (t['lo'], t['hi']), 'borderAudit',
                    f'tile {t["id"]}: audit leaf interval {row["leafBegin"]}..{row["leafEnd"]} != {t["lo"]}..{t["hi"]}')
            require(row['geometricError'] == t['g'], 'borderAudit',
                    f'tile {t["id"]}: audit G {row["geometricError"]} != published {t["g"]}')
            require(row['depth'] == r['contract']['depth'], 'borderAudit', f'tile {t["id"]}: contract depth')
            self.records[int(t['id'])] = r
            used.add(sha)
        require(used == set(by_sha), 'borderAudit',
                f'{len(set(by_sha) - used)} audit records name no delivered coarse payload')
        return omitted

    # -- per tile ------------------------------------------------------------
    def tile(self, t, r):
        require, store = self.require, self.store
        tid = int(t['id'])
        contract, row = r['contract'], r['tile']
        depth = contract['depth']
        e = self.scale(depth)
        require(contract['scale'] == e, 'borderScale',
                f'tile {tid}: audit scale {contract["scale"]} != source E_{depth} = {e}')
        require(t['g'] >= e, 'borderScale', f'tile {tid}: G {t["g"]} below E_{depth} = {e}')
        # Terminal claims are needed by the pairwise rules, which run before any
        # certificate is trusted or compared.
        claims = set()
        for key in contract['terminalVertices']:
            claims.add(self.canonical(key))
        for c in contract['certificates']:
            if c.get('terminalLock'):
                claims.update(self.canonical(k) for k in c['endpoints'])
        self.terminal_claims[tid] = claims
        return depth, e

    def certify(self, t, r, e):
        """Interface items and certificates against the re-derived arcs."""
        require, store = self.require, self.store
        tid = int(t['id'])
        contract = r['contract']
        depth = contract['depth']
        arcs = {}
        for a, b, m, path, d2 in store.db.execute(
                'SELECT a,b,m,path,d2 FROM border_arcs WHERE tile=?', (tid,)).fetchall():
            arcs[a, b] = (m, json.loads(path), Fraction(d2))
        retained = {p for (p,) in store.db.execute(
            'SELECT point FROM border_images WHERE tile=? AND lower IS NULL', (tid,))}
        adjacent = collections.defaultdict(list)
        for m, path, _ in arcs.values():
            for x, y in zip(path, path[1:]):
                adjacent[x].append(y)
                adjacent[y].append(x)
        # Interface items: delivered chords and retained border points.
        items = set()
        for row_ in contract['interface']:
            a, b = self.canonical(row_['a']), self.canonical(row_['b'])
            require(a is not None and b is not None, 'borderCertificate',
                    f'tile {tid}: interface item is not a source canonical key')
            items.add(_key(a, b))
        expected = set(arcs) | {(p, p) for p in retained}
        require(items == expected, 'borderCertificate',
                f'tile {tid}: interface items differ from re-derived chords/points '
                f'({len(items - expected)} extra, {len(expected - items)} missing)')
        chains, covered = {}, set()
        epsilon_max = 0.0
        for c in contract['certificates']:
            self.stats['certificates'] += 1
            a, b = (self.canonical(k) for k in c['endpoints'])
            require(a is not None and b is not None, 'borderCertificate', f'tile {tid}: endpoint is no source key')
            eps = c['epsilon']
            require(type(eps) in (int, float) and math.isfinite(eps) and eps >= 0, 'borderCertificate',
                    f'tile {tid}: invalid epsilon {eps!r}')
            require(Fraction(eps) * 16 <= Fraction(e), 'borderCertificate',
                    f'tile {tid}: epsilon {eps} above E_{depth}/16 = {e / 16}')
            epsilon_max = max(epsilon_max, eps)
            if a == b:
                # A point-only contact: a claim, re-derived from incidence above.
                require(a in retained, 'borderCertificate',
                        f'tile {tid}: point certificate is no retained border point')
                continue
            chord = _key(a, b)
            require(chord in arcs and chord not in covered, 'borderCertificate',
                    f'tile {tid}: certificate names no re-derived arc, or one arc twice')
            covered.add(chord)
            m, path, d2 = arcs[chord]
            require(d2 <= Fraction(eps) ** 2, 'borderCertificate',
                    f'tile {tid}: arc displacement {math.sqrt(d2)} exceeds certified epsilon {eps}')
            if c['terminalLock']:
                require(m == 1, 'terminalBorderLock', f'tile {tid}: a terminal-locked arc is contracted')
            # Re-walk the hinted chain through source incidence.
            origin = self.canonical(c['origin'])
            first_edge = tuple(self.canonical(k) for k in c['chain'])
            hint = (origin, _key(*first_edge))
            if hint not in chains:
                chains[hint] = self.walk(tid, origin, first_edge, adjacent)
            walk = chains[hint]
            lo_, hi_ = c['first'], c['end']
            require(isinstance(lo_, int) and isinstance(hi_, int) and 0 <= lo_ < hi_ < len(walk),
                    'borderCertificate', f'tile {tid}: hinted ordinals {lo_}..{hi_} outside the source chain')
            sub = walk[lo_:hi_ + 1]
            require((sub[0], sub[-1]) in ((a, b),), 'borderCertificate',
                    f'tile {tid}: certificate endpoints are not the hinted ordinals')
            require(sub == path or sub == path[::-1], 'borderCertificate',
                    f'tile {tid}: certified source subarc differs from the delivered arc')
        require(covered == set(arcs), 'borderCertificate',
                f'tile {tid}: {len(set(arcs) - covered)} delivered arcs without a certificate')
        for key in contract['terminalVertices']:
            p = self.canonical(key)
            require(p in retained, 'terminalBorderLock', f'tile {tid}: claimed terminal vertex is not exact')
        self.stats['maxEpsilon'] = max(self.stats['maxEpsilon'], epsilon_max)
        return retained

    def walk(self, tid, origin, first_edge, adjacent):
        require = self.require
        require(origin in first_edge, 'borderCertificate', f'tile {tid}: chain origin is not on its first edge')
        at = origin
        nxt = first_edge[1] if first_edge[0] == origin else first_edge[0]
        require(nxt in adjacent.get(at, ()), 'borderCertificate', f'tile {tid}: hinted edge is not on the border')
        walk, seen = [at], set()
        while True:
            self.store.work.charge()
            edge = _key(at, nxt)
            if edge in seen:
                break
            seen.add(edge)
            walk.append(nxt)
            if nxt == origin or len(adjacent[nxt]) != 2:
                break
            around = adjacent[nxt]
            at, nxt = nxt, around[1] if around[0] == at else around[0]
            if len(walk) > MAX_QUERY_CANDIDATES:
                raise WorkLimit('hinted border chain length limit')
        return walk

    # -- depth relations -------------------------------------------------------
    def cell_parent(self, t):
        p = t['parent']
        while p is not None and p['external']:
            p = p['parent']
        return p

    def depths(self, omitted):
        """Logical depth labels (coarse and omitted records), validated against every
        relation the delivered tree proves before any is used: cell siblings share
        one depth, and a labelled cell is strictly deeper than its labelled cell
        ancestors (a repeated chain payload excepted). Returns exact depths (labels,
        and leaves with a labelled sibling) and proven lower bounds for the rest."""
        require = self.require
        labels, source = {}, {}
        for t in self.tiles:
            tid = int(t['id'])
            if tid in self.records:
                labels[tid], source[tid] = self.records[tid]['contract']['depth'], id(self.records[tid])
        # Omitted cells with one leaf interval form a chain: shallower cells first.
        claimed = collections.defaultdict(list)
        for r in omitted:
            claimed[r['key']['leafBegin'], r['key']['leafEnd']].append(r['key']['depth'])
        cells = collections.defaultdict(list)
        for t in self.tiles:
            if not t['contents'] and not t['external']:
                cells[t['lo'], t['hi']].append(int(t['id']))
        for key, ids in cells.items():
            for tid, depth in zip(ids, sorted(claimed.get(key, ()))):
                labels[tid], source[tid] = depth, ('omitted', tid)
        exact, lower = dict(labels), {}
        for t in self.tiles:
            tid = int(t['id'])
            parent, steps = self.cell_parent(t), 1
            while parent is not None and int(parent['id']) not in labels:
                parent, steps = self.cell_parent(parent), steps + 1
            if parent is not None and tid in labels:
                pid = int(parent['id'])
                require(labels[pid] < labels[tid] or source[pid] == source[tid], 'borderDepth',
                        f'tile {tid}: logical depth {labels[tid]} is not below cell ancestor {pid} '
                        f'({labels[pid]})')
            if tid not in labels:
                lower[tid] = labels[int(parent['id'])] + steps if parent is not None else 0
        for t in self.tiles:
            children = [int(c['id']) for c in self.cell_children(t)]
            found = {labels[c] for c in children if c in labels}
            require(len(found) <= 1, 'borderDepth',
                    f'cell children of tile {t["id"]} claim distinct logical depths {sorted(found)}')
            for c in children:
                if found and c not in labels:
                    exact[c] = next(iter(found))
                    lower[c] = exact[c]
        self.stats['depthLabels'] = len(labels)
        return exact, lower

    def cell_children(self, t):
        out, pending = [], list(t['children'])
        while pending:
            c = pending.pop()
            if c['external']:
                pending.extend(c['children'])
            else:
                out.append(c)
        return out

    # -- pairwise and terminal rules -------------------------------------------
    def pairs(self, exact, lower):
        require, store = self.require, self.store
        by_id = {int(t['id']): t for t in self.tiles}
        leaf_tile = {t['lo']: t for t in self.leaves}
        position = {}

        def image(point, lower_, upper, i, m):
            if lower_ is None:
                return _fractions(store.canonical_xyz(point))
            if (lower_, upper) not in position:
                position[lower_, upper] = (_fractions(store.canonical_xyz(lower_)),
                                           _fractions(store.canonical_xyz(upper)))
            a, b = position[lower_, upper]
            return _lerp(a, b, i, m)

        def ancestor(a, t):
            while t is not None:
                if t is a:
                    return True
                t = t['parent']
            return False

        self.unclaimed, self.locks = {}, []
        current, group = None, []
        rows = store.db.execute('SELECT point,tile,lower,upper,i,m FROM border_images ORDER BY point,tile')
        for row in list(rows) + [(None,) * 6]:
            store.work.charge()
            if row[0] != current:
                if group:
                    self.vertex(current, group, by_id, leaf_tile, exact, lower, image, ancestor)
                current, group = row[0], []
            if row[0] is not None:
                group.append(row)
        self.stats['position'] = len(position)
        # Terminal locks after every pairwise gap; claims after every lock.
        require(not self.locks, 'terminalBorderLock', self.locks[0] if self.locks else '')
        for tile, point in self.unclaimed.items():
            require(False, 'terminalBorderLock',
                    f'tile {tile}: proven terminal contact {store.canonical_xyz(point)} missing from its audit claims')

    def vertex(self, point, group, by_id, leaf_tile, exact, lower, image, ancestor):
        require, store = self.require, self.store
        coarse = [(by_id[tile], lower_, upper, i, m) for _, tile, lower_, upper, i, m in group]
        for x in range(len(coarse)):
            tx, lx, ux, ix, mx = coarse[x]
            for y in range(x + 1, len(coarse)):
                store.work.charge()
                ty, ly, uy, iy, my = coarse[y]
                if not (tx['hi'] <= ty['lo'] or ty['hi'] <= tx['lo']):
                    continue
                dx, dy = exact[int(tx['id'])], exact[int(ty['id'])]
                same = (lx, ux, Fraction(ix, mx) if mx else 0) == (ly, uy, Fraction(iy, my) if my else 0)
                if dx == dy:
                    self.stats['sameDepthPairs'] += 1
                    require(same or image(point, lx, ux, ix, mx) == image(point, ly, uy, iy, my),
                            'sameDepthBorderGap',
                            f'tiles {tx["id"]} and {ty["id"]} at depth {dx} disagree at a shared source vertex')
                    continue
                self.stats['mixedDepthPairs'] += 1
                if same:
                    continue
                (tc, dc), (tf, df) = sorted(((tx, dx), (ty, dy)), key=lambda v: v[1])
                gap2 = _distance2(image(point, lx, ux, ix, mx), image(point, ly, uy, iy, my))
                limit = min(Fraction(self.scale(dc)), Fraction(tc['g'])) / 8
                self.stats['maxMixedGapOverE'] = max(self.stats['maxMixedGapOverE'],
                                                     math.sqrt(gap2) / (self.scale(dc) / 8))
                require(gap2 <= limit * limit, 'mixedDepthBorderGap',
                        f'tiles {tc["id"]} (depth {dc}) and {tf["id"]} (depth {df}): gap {math.sqrt(gap2)} '
                        f'above min(E, G)/8 = {float(limit)}')
        # Terminal contacts: a leaf no deeper than a coarse owner locks it exactly.
        owners = [l for (l,) in store.db.execute('SELECT leaf FROM canonical_owners WHERE point=?', (point,))]
        for t, lower_, *_ in coarse:
            d = exact[int(t['id'])]
            for l in owners:
                store.work.charge()
                if t['lo'] <= l < t['hi']:
                    continue
                leaf = leaf_tile[l]
                lid = int(leaf['id'])
                cell = self.cell_parent(leaf)
                if lid in exact:
                    terminal = exact[lid] <= d
                elif cell is not None and ancestor(cell, t):
                    terminal = True
                elif lower.get(lid, 0) > d:
                    terminal = False
                else:
                    # Unresolved: the leaf may be no deeper than the tile, so its
                    # contact must be exact; a contraction here is never accepted.
                    self.stats['terminalContactsUnresolvedExact'] += 1
                    if lower_ is not None:
                        self.locks.append(f'tile {t["id"]} contracts a border vertex shared with leaf {leaf["id"]}, '
                                          f'whose depth (at least {lower.get(lid, 0)}) is not proven deeper than {d}')
                    continue
                if terminal:
                    self.stats['terminalContacts'] += 1
                    if lower_ is not None:
                        self.locks.append(f'tile {t["id"]} contracts a border vertex shared with no-deeper leaf {leaf["id"]}')
                    if point not in self.terminal_claims.get(int(t['id']), set()):
                        self.unclaimed.setdefault(int(t['id']), point)

    # -- topology receipts -----------------------------------------------------
    def topology(self, t, r, e, retained):
        require, store = self.require, self.store
        tid = int(t['id'])
        published, attempted = r['tile']['topology'], r['tile']['attemptedTopology']
        children = self.cell_children(t)
        while len(children) == 1 and self.records.get(int(children[0]['id'])) is r:
            children = self.cell_children(children[0])
        child_g = max((c['g'] for c in children if c['contents']), default=0.0)
        floor = max(e, child_g)
        g = t['g']
        for kind, count, local, increase, final in (
                ('closedHoles', 'closedHoles', ('holePatchToSource', 'holeBoundaryToPatch'), 'holeGIncrease', 'closedHoleFinalG'),
                ('openedHoles', 'openedHoles', ('openedHoleToProxy', 'openedHoleBoundaryToSource'), 'openedHoleGIncrease', 'openedHoleFinalG'),
                ('droppedComponents', 'droppedComponents', ('droppedComponentToProxy',), 'droppedComponentGIncrease', 'droppedComponentFinalG')):
            n = published[count]
            require(isinstance(n, int) and 0 <= n <= attempted[count], 'topologyReceipt',
                    f'tile {tid}: published {count} {n} exceeds attempted {attempted[count]}')
            values = [published[k] for k in local] + [published[increase], published[final]]
            require(all(type(v) in (int, float) and math.isfinite(v) and v >= 0 for v in values),
                    'topologyReceipt', f'tile {tid}: invalid {kind} distances')
            require(published[final] == (g if n else 0.0), 'topologyReceipt',
                    f'tile {tid}: {final} {published[final]} is not the published G {g} of its {n} {kind}')
            if not n:
                require(all(published[k] == 0 for k in local) and published[increase] == 0, 'topologyReceipt',
                        f'tile {tid}: {kind} distances without a published operation')
                continue
            require(g >= floor + published[increase] * (1 - 1e-12) - 1e-300, 'topologyReceipt',
                    f'tile {tid}: G {g} omits its {increase} {published[increase]} above floor {floor}')
            if kind == 'droppedComponents':
                expected = max(1.05 * published['droppedComponentToProxy'] - floor, 0.0)
                require(abs(published[increase] - expected) <= 1e-12 * max(1.0, expected), 'topologyReceipt',
                        f'tile {tid}: {increase} {published[increase]} != 1.05 x proxy distance - floor = {expected}')
            else:
                # Admitted closures and openings fit E with S2's 5% margin.
                for k in local:
                    require(1.05 * published[k] <= e * (1 + 1e-12), 'topologyReceipt',
                            f'tile {tid}: published {k} {published[k]} above E/1.05')
                require(published[increase] == 0, 'topologyReceipt',
                        f'tile {tid}: an admitted {kind} contributes {published[increase]} to G')
        self.components(t, children, e, child_g, retained, published)

    def components(self, t, children, e, child_g, retained, published):
        """Independent dropped-component, closure and opening replay of one parent job.

        The job input is the union of the cell children's decoded payloads, welded by
        f32 key. Lossy coarse interiors may be re-quantized, so an input vertex
        survives at an output vertex within both payloads' codec tolerances (`slack`
        widens every comparison with a producer distance by the same amount)."""
        require, store = self.require, self.store
        tid = int(t['id'])
        border = {f32_point(store.canonical_xyz(p)) for (p,) in store.db.execute(
            'SELECT point FROM border_images WHERE tile=?', (tid,))}
        contracted = {f32_point(store.canonical_xyz(p)) for (p,) in store.db.execute(
            'SELECT point FROM border_images WHERE tile=? AND lower IS NOT NULL', (tid,))}
        tolerance = [0.0] * 3

        def faces(tile):
            out = []
            for f in store.faces(int(tile['id'])):
                store.work.charge()
                for k in range(3):
                    tolerance[k] = max(tolerance[k], f.get('positionTolerance', [0, 0, 0])[k])
                out.append([f32_point(p) for p in f['xyz']])
            return out

        source = [f for c in children if c['contents'] for f in faces(c)]
        output = faces(t)
        reach = [2 * x for x in tolerance]
        slack = 2 * math.sqrt(sum(x * x for x in reach))
        # contractedVertices: the job's input vertices this tile contracts onto chords.
        derived = len({p for f in source for p in f} & contracted)
        require(published['contractedVertices'] == derived, 'topologyReceipt',
                f'tile {tid}: audit contractedVertices {published["contractedVertices"]}, '
                f're-derived {derived} input vertices on contracted arcs')
        root = _UnionFind()
        for f in source:
            for p in f[1:]:
                root.join(f[0], p)
        groups = collections.defaultdict(set)
        faces_of = collections.defaultdict(list)
        for p in list(root.parent):
            groups[root(p)].add(p)
        for f in source:
            faces_of[root(f[0])].append(f)
        out_points = _Near({p for f in output for p in f}, reach)
        mesh = _Mesh(output)
        dropped, ambiguous, lost = 0, False, set()
        floor = max(e, child_g)
        loss = diameter_max = 0.0
        for key, members in groups.items():
            store.work.charge(len(members))
            hits = [out_points.find(p) for p in members]
            if any(hits):
                ambiguous |= not any(h == 'exact' for h in hits)
                continue
            dropped += 1
            lost.add(key)
            require(not members & border, 'droppedComponent',
                    f'tile {tid}: a dropped input component touches a delivered border')
            lo = [min(p[k] for p in members) for k in range(3)]
            hi = [max(p[k] for p in members) for k in range(3)]
            diameter = math.sqrt(sum((hi[k] - lo[k]) ** 2 for k in range(3)))
            diameter_max = max(diameter_max, diameter)
            require(diameter < floor + slack and diameter < t['g'] + slack, 'droppedComponent',
                    f'tile {tid}: dropped component diameter {diameter} not below max(E, child G) {floor}')
            # Its loss: S1b's fixed controls on every face, exactly to the delivered proxy.
            for f in faces_of[key]:
                for q in _samples(f):
                    store.work.charge(len(output))
                    loss = max(loss, mesh.distance(q))
        self.stats['droppedComponents'] += dropped
        self.stats['maxDroppedLossOverE'] = max(self.stats['maxDroppedLossOverE'], loss / e)
        if dropped:
            require(published['droppedComponentToProxy'] >= loss - slack, 'topologyReceipt',
                    f'tile {tid}: audit droppedComponentToProxy {published["droppedComponentToProxy"]} '
                    f'under the measured {loss}')
            require(t['g'] >= max(floor, 1.05 * (loss - slack)) * (1 - 1e-12), 'droppedComponent',
                    f'tile {tid}: G {t["g"]} omits the measured drop loss {loss} (x1.05, floor {floor})')
            require(abs(published['maxDroppedComponentDiameter'] - diameter_max) <= slack + 1e-12 * diameter_max,
                    'topologyReceipt', f'tile {tid}: audit maxDroppedComponentDiameter '
                    f'{published["maxDroppedComponentDiameter"]} != re-derived {diameter_max}')
        if dropped != published['droppedComponents']:
            require(ambiguous and dropped < published['droppedComponents'], 'topologyReceipt',
                    f'tile {tid}: {dropped} dropped components re-derived, audit reports {published["droppedComponents"]}')
            self.stats['droppedComponentsWithinTolerance'] += published['droppedComponents'] - dropped
        self.holes(t, source, output, border, published, e, reach, slack, lambda p: root(p) in lost)

    def holes(self, t, source, output, border, published, e, reach, slack, dropped):
        """Boundary changes between the job's input and output, per region.

        Boundary components of input and output are joined into clusters when they
        share (near) vertices. A cluster's cycle-rank drop counts its closures and
        its rise its openings; the audit may report more (a closure and an opening in
        one cluster) but never fewer. Wherever an input boundary vertex became
        interior, or the output has boundary vertices the input lacked, the loss is
        measured: centroids of every fundamental cycle projected across the change,
        and fixed controls of faces lying over the old boundary, exactly against
        source (closures) or the delivered proxy (openings). Both must fit E; a
        change with no witness is refused."""
        require, store = self.require, self.store
        tid = int(t['id'])
        before, after = _boundary(source), _boundary(output)
        before_near = _Near({p for points, _ in before for p in points}, reach)
        after_near = _Near({p for points, _ in after for p in points}, reach)
        out_points = _Near({p for f in output for p in f}, reach)
        in_points = _Near({p for f in source for p in f}, reach)
        # Clusters: input and output boundary components sharing a near vertex.
        cluster = _UnionFind()
        owner = {}
        for i, (points, _) in enumerate(before):
            cluster(('in', i))
            for p in points:
                owner[p] = ('in', i)
        for j, (points, _) in enumerate(after):
            cluster(('out', j))
            for p in points:
                hit = before_near.nearest(p)
                if hit is not None:
                    cluster.join(('out', j), owner[hit])
        members = collections.defaultdict(lambda: ([], []))
        for i in range(len(before)):
            members[cluster(('in', i))][0].append(i)
        for j in range(len(after)):
            members[cluster(('out', j))][1].append(j)
        rank = lambda c: len(c[1]) - len(c[0]) + 1
        input_mesh, output_mesh = _Mesh(source), _Mesh(output)
        closed = opened = 0
        closure_loss = opening_loss = rim = 0.0
        for ins, outs in members.values():
            points_in = set().union(*(before[i][0] for i in ins)) if ins else set()
            points_out = set().union(*(after[j][0] for j in outs)) if outs else set()
            if ins and all(dropped(next(iter(before[i][0]))) for i in ins) and not outs:
                continue
            r_in = sum(rank(before[i]) for i in ins)
            r_out = sum(rank(after[j]) for j in outs)
            touches = bool((points_in | points_out) & border)
            # A covered input boundary vertex, or a boundary vertex the input lacked.
            covered = [p for p in points_in if out_points.find(p) and not after_near.find(p)]
            new = [p for p in points_out if not before_near.find(p)]
            if touches:
                require(r_out >= r_in, 'holeClosure',
                        f'tile {tid}: a boundary cycle touching a delivered border was closed')
                require(r_out <= r_in and not new, 'holeOpening',
                        f'tile {tid}: an opened boundary touches a delivered border')
            closed += max(r_in - r_out, 0)
            opened += max(r_out - r_in, 0)
            if covered or r_in > r_out:
                witnesses = [output_mesh.project(c) for c in _cycle_centroids([before[i] for i in ins])]
                for f in output_mesh.near(points_in):
                    for q in _samples(f):
                        store.work.charge(len(source))
                        if input_mesh.over_boundary(q):
                            witnesses.append(q)
                require(bool(witnesses), 'holeClosure', f'tile {tid}: a closure has no measurable patch')
                for q in witnesses:
                    store.work.charge()
                    closure_loss = max(closure_loss, self.source_distance(t, q))
            if new or r_out > r_in:
                witnesses = [input_mesh.project(c) for c in _cycle_centroids([after[j] for j in outs])]
                for f in input_mesh.near(points_out):
                    for q in _samples(f):
                        store.work.charge(len(output))
                        if output_mesh.over_boundary(q):
                            witnesses.append(q)
                require(bool(witnesses), 'holeOpening', f'tile {tid}: an opening has no measurable lost surface')
                for q in witnesses:
                    store.work.charge(len(output))
                    opening_loss = max(opening_loss, output_mesh.distance(q))
                for j in outs:
                    for a, b in after[j][1]:
                        m = tuple((a[k] + b[k]) / 2 for k in range(3))
                        d = self.source_distance(t, m)
                        rim = max(rim, d)
                        self.stats['maxOpeningRimOverE'] = max(self.stats['maxOpeningRimOverE'], d / e)
                        require(d <= e * (1 + 1e-9) + slack, 'holeOpening',
                                f'tile {tid}: opened hole rim is {d} from source, above E = {e}')
        self.stats['maxClosureLossOverE'] = max(self.stats['maxClosureLossOverE'], closure_loss / e)
        self.stats['maxOpeningLossOverE'] = max(self.stats['maxOpeningLossOverE'], opening_loss / e)
        require(closure_loss <= e * (1 + 1e-9) + slack, 'holeClosure',
                f'tile {tid}: a closed or covered hole is {closure_loss} from source, above E = {e}')
        require(opening_loss <= e * (1 + 1e-9) + slack, 'holeOpening',
                f'tile {tid}: an opened hole lost surface {opening_loss} from the proxy, above E = {e}')
        self.stats['closedHoles'] += closed
        self.stats['openedHoles'] += opened
        require(published['closedHoles'] >= closed and published['openedHoles'] >= opened, 'topologyReceipt',
                f'tile {tid}: re-derived {closed} closed / {opened} opened holes, audit reports '
                f'{published["closedHoles"]} / {published["openedHoles"]}')
        require((published['closedHoles'] == 0 or closure_loss > 0 or closed) and
                (published['openedHoles'] == 0 or opening_loss > 0 or opened), 'topologyReceipt',
                f'tile {tid}: audit reports holes the delivered boundaries do not show')
        if published['closedHoles']:
            require(published['holePatchToSource'] >= closure_loss - slack, 'topologyReceipt',
                    f'tile {tid}: audit holePatchToSource {published["holePatchToSource"]} under the measured {closure_loss}')
        if published['openedHoles']:
            require(published['openedHoleToProxy'] >= opening_loss - slack, 'topologyReceipt',
                    f'tile {tid}: audit openedHoleToProxy {published["openedHoleToProxy"]} under the measured {opening_loss}')
            require(published['openedHoleBoundaryToSource'] >= rim - slack, 'topologyReceipt',
                    f'tile {tid}: audit openedHoleBoundaryToSource {published["openedHoleBoundaryToSource"]} under the measured {rim}')

    def source_distance(self, t, p):
        from oracle import distance2
        _, want = self.store.nearest(p, t['lo'], t['hi'], distance2)
        return math.sqrt(distance2(p, want['xyz']))

    # -- omission --------------------------------------------------------------
    def omission(self, omitted):
        require = self.require
        cells = collections.Counter()
        sources = collections.Counter()
        frontier = collections.defaultdict(collections.Counter)
        for t in self.tiles:
            if t['contents'] or t['external']:
                continue
            key = (t['lo'], t['hi'])
            cells[key] += 1
            rendered = [c for c in self.cell_children(t) if c['contents']]
            children = self.cell_children(t)
            sources[key] += all(c['contents'] for c in children)
            frontier[key].update((c['lo'], c['hi']) for c in rendered)
        claimed = collections.Counter()
        claimed_sources = collections.Counter()
        claimed_frontier = collections.defaultdict(collections.Counter)
        for r in omitted:
            k = r['key']
            key = (k['leafBegin'], k['leafEnd'])
            claimed[key] += 1
            reason = r['reason']
            require(reason in ('contract_lower_bound', 'remesh_stalled', 'descendant_omission'), 'omissionReport',
                    f'unknown omission reason {reason!r}')
            require(r['source'] == (reason != 'descendant_omission'), 'omissionReport',
                    f'omission source flag disagrees with reason {reason}')
            claimed_sources[key] += r['source']
            claimed_frontier[key].update((f['leafBegin'], f['leafEnd']) for f in r['frontier'])
        require(claimed == cells, 'omissionReport',
                f'audit omitted cells {sorted(claimed.items())} != archive {sorted(cells.items())}')
        require(claimed_sources == sources, 'omissionReport', 'audit omission sources differ from the archive')
        # The complete frontier, with multiplicity: every rendered cell child of the
        # omitted cells on one leaf interval (a chain reports it once, below).
        for key in cells:
            require(claimed_frontier[key] == frontier[key], 'omissionReport',
                    f'omitted cells over leaves {key}: audit frontier {sorted(claimed_frontier[key].elements())} '
                    f'!= rendered cell children {sorted(frontier[key].elements())}')
        self.stats['omittedCells'] = sum(cells.values())


def inspect(store, tiles, leaves, archive, require, audit_path=None, extent=None, census=None):
    """D11 audit after the census. Without a sidecar, contraction is refused."""
    census = census or {}
    started = time.process_time()
    if audit_path is None:
        require(not census.get('contractedBorderArcs'), 'borderAudit',
                'contracted borders cannot be certified without the S1b audit sidecar')
        return dict(audited=False, uninspected=['D11 certificates, E/G inequalities, logical-depth gaps, '
                    'terminal locks and topology receipts need the S1b audit sidecar'])
    data = open(audit_path, 'rb').read()
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    audit = Audit(store, tiles, leaves, archive, require, extent)
    omitted = audit.join(rows)
    audit.terminal_claims = {}
    contexts = {}
    for t in tiles:
        r = audit.records.get(int(t['id']))
        if r is None:
            continue
        depth, e = audit.tile(t, r)
        # A repeated chain payload is checked once, below its deepest repeat.
        contexts[id(r)] = (t, r, e)
    # Geometry first: D11 gaps and terminal contacts from images alone, then the
    # producer's certificates, then topology receipts and omission records.
    exact, lower = audit.depths(omitted)
    audit.pairs(exact, lower)
    for t, r, e in contexts.values():
        retained = audit.certify(t, r, e)
        audit.topology(t, r, e, retained)
    audit.omission(omitted)
    stats = dict(audit.stats)
    stats['uninspected'] = [
        'history-dependent receipt counters (reconstructedFaces, duplicateFaces, remeshedVertices, remeshRounds, '
        'constrainedRemeshes, rejectedRemeshRounds, rejectedHoleClosures, rejectedHoleOpenings) and every '
        'attemptedTopology distance: rejected proposals leave no delivered geometry',
        'holeBoundaryToPatch beyond the E/1.05 receipt bound; closure loss is measured patch-to-source',
        'topologyAttempt/topologyConstruction location hints: operations are re-derived, never read',
        'a contact with a leaf whose logical depth the tree does not fix is required exact, not claimed',
        'coincident coarse faces are licensed per corner and twin region, not by a continuous source map']
    stats.update(auditCpuSeconds=time.process_time() - started, audited=True, sidecarSha256=hashlib.sha256(data).hexdigest(), sidecarBytes=len(data),
                 records=len(audit.records), omittedRecords=len(omitted), extent=extent)
    return stats


def compare(result, report):
    """Compare a producer report's audit receipt and per-depth summaries."""
    hlod = report.get('diagnostics', report).get('hlod', report)
    receipt = hlod.get('audit') or {}
    differences = {}
    if receipt.get('sha256') != result['sidecarSha256'] or receipt.get('bytes') != result['sidecarBytes']:
        differences['audit'] = dict(reported=receipt, derived=dict(sha256=result['sidecarSha256'],
                                                                  bytes=result['sidecarBytes']))
    return dict(ok=not differences, differences=differences)
