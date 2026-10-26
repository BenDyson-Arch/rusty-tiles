"""D11 scalable borders and S1b audit receipts, re-derived from source and archive.

Source canonical incidence (``spatial.Store``: source manifest plus verified
leaves) defines each coarse tile's shared border: leaf edges and points with an
owner leaf inside its interval and one outside. Retained exact vertices split it
into arcs; every arc is delivered as exactly one chord whose directions are
witnessed by every source edge it replaces. A contracted vertex is placed on its
chord at its original edge ordinal; displacements and gaps are exact Fractions.

The S1b audit sidecar (``borders.jsonl``, schema 1) is an index only: payload
SHA-256 joins a record to its archive tile, and its logical depth, certificates
and receipts are claims. Every claim is recomputed from geometry and compared:
E_d from the captured source extent, each certificate's hinted chain re-walked
through source incidence, epsilon against the exact arc displacement, complete
one-to-one arc coverage, terminal contacts the tree proves, same-depth equality
and mixed-depth gaps at every shared source breakpoint, dropped components,
hole closures/openings, their G contributions and omitted cells.

Not proven here: a terminal contact with a leaf whose logical depth neither the
tree nor a labelled sibling fixes (counted), hole-loss witnesses beyond the
patch/rim/lost-face controls named below, and S2's sampled two-sided error.
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


def folds(store, tile, directed, arcs, require):
    """Directed incidences are counted, never reduced to a mask (S1b review 1).

    A coarse edge used n >= 2 times in one direction (a fold or an overlap on one
    side) needs n verified descendant source faces in that direction on every
    source edge it stands for: itself, or each edge of its contracted arc."""
    tid, lo, hi = int(tile['id']), tile['lo'], tile['hi']
    ids = {}

    def canonical(p):
        if p not in ids:
            row = store.db.execute('SELECT id FROM canonical_points WHERE xyz=?',
                                   (struct.pack('<3d', *(0.0 if x == 0 else x for x in p)),)).fetchone()
            ids[p] = row[0] if row else None
        return ids[p]
    for (p, q), n in directed.items():
        store.work.charge()
        if n < 2:
            continue
        a, b = canonical(p), canonical(q)
        path = None if a is None or b is None else arcs.get(_key(a, b), [min(a, b), max(a, b)])
        supported = path is not None
        if supported:
            if path[0] != a:
                path = path[::-1]
            for x, y in zip(path, path[1:]):
                store.work.charge()
                nf, nr = store.db.execute(
                    'SELECT coalesce(sum(nf),0),coalesce(sum(nr),0) FROM canonical_edges '
                    'WHERE a=? AND b=? AND leaf>=? AND leaf<?', (*_key(x, y), lo, hi)).fetchone()
                supported &= (nf if x < y else nr) >= n
        require(supported, 'interfaceWinding',
                f'tile {tid}: coarse edge {p} -> {q} used {n} times in one direction without that '
                'descendant source multiplicity')


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
        if p in self.points:
            return 'exact'
        if not any(self.reach):
            return None
        c = self._cell(p)
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for dz in (-1, 0, 1):
                    for q in self.grid.get((c[0] + dx, c[1] + dy, c[2] + dz), ()):
                        if all(abs(p[k] - q[k]) <= self.reach[k] for k in range(3)):
                            return 'tolerance'
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
        # Logical depth increases strictly down the tree, except a repeated chain payload.
        for t in self.tiles:
            r = self.records.get(int(t['id']))
            p = t['parent']
            while r and p is not None and int(p['id']) not in self.records:
                p = p['parent']
            if r and p is not None:
                q = self.records[int(p['id'])]
                require(q['contract']['depth'] < r['contract']['depth'] or q is r, 'borderAudit',
                        f'tile {t["id"]}: logical depth does not increase below tile {p["id"]}')
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
        """Exact or bounded logical depth of every tile, from labels and the tree."""
        exact, lower = {}, {}
        omitted_depths = collections.defaultdict(list)
        for r in omitted:
            omitted_depths[r['key']['leafBegin'], r['key']['leafEnd']].append(r['key']['depth'])
        for t in self.tiles:
            tid = int(t['id'])
            if tid in self.records:
                exact[tid] = self.records[tid]['contract']['depth']
        for t in self.tiles:
            tid = int(t['id'])
            if tid in exact:
                continue
            parent = self.cell_parent(t)
            if parent is not None:
                siblings = [c for c in self.cell_children(parent) if int(c['id']) in self.records]
                labels = {self.records[int(c['id'])]['contract']['depth'] for c in siblings}
                if labels:
                    self.require(len(labels) == 1, 'borderAudit',
                                 f'tiles below {parent["id"]} have distinct logical depths')
                    exact[tid] = labels.pop()
                    continue
                pid = int(parent['id'])
                if pid in exact:
                    lower[tid] = exact[pid] + 1
                elif (parent['lo'], parent['hi']) in omitted_depths:
                    lower[tid] = min(omitted_depths[parent['lo'], parent['hi']]) + 1
                else:
                    lower[tid] = lower.get(pid, 0) + 1
            else:
                lower[tid] = 0
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

        self.unclaimed = {}
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
        # Claims are compared only after every geometric terminal contact passed.
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
                    self.stats['terminalContactsUndetermined'] += 1
                    continue
                if terminal:
                    self.stats['terminalContacts'] += 1
                    require(lower_ is None, 'terminalBorderLock',
                            f'tile {t["id"]} contracts a border vertex shared with no-deeper leaf {leaf["id"]}')
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
        """Independent dropped-component, closure and opening census of one parent job.

        The job input is the union of the cell children's decoded payloads, welded by
        f32 key. Lossy coarse interiors may be re-quantized, so an input vertex
        survives at an output vertex within both payloads' codec tolerances."""
        require, store = self.require, self.store
        tid = int(t['id'])
        border = {f32_point(store.canonical_xyz(p)) for (p,) in store.db.execute(
            'SELECT point FROM border_images WHERE tile=?', (tid,))}
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
        root = _UnionFind()
        for f in source:
            for p in f[1:]:
                root.join(f[0], p)
        groups = collections.defaultdict(set)
        for p in list(root.parent):
            groups[root(p)].add(p)
        out_points = _Near({p for f in output for p in f}, reach)
        dropped, ambiguous, lost = 0, False, set()
        floor = max(e, child_g)
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
            slack = 2 * math.sqrt(sum(x * x for x in reach))
            require(diameter < floor + slack and diameter < t['g'] + slack, 'droppedComponent',
                    f'tile {tid}: dropped component diameter {diameter} not below max(E, child G) {floor}')
            self.stats['maxDroppedDiameterOverFloor'] = max(self.stats['maxDroppedDiameterOverFloor'],
                                                            diameter / floor if floor else math.inf)
        self.stats['droppedComponents'] += dropped
        if dropped != published['droppedComponents']:
            require(ambiguous and dropped < published['droppedComponents'], 'topologyReceipt',
                    f'tile {tid}: {dropped} dropped components re-derived, audit reports {published["droppedComponents"]}')
            self.stats['droppedComponentsWithinTolerance'] += published['droppedComponents'] - dropped
        self.holes(t, source, output, border, published, e, reach, lambda p: root(p) in lost)

    def holes(self, t, source, output, border, published, e, reach, dropped):
        """Interior boundary cycles lost (closed) or added (opened) between input and output.

        A boundary component's cycle rank (edges - vertices + 1) counts its loops, so
        one closure and one opening cannot cancel. Patch faces spanning only a closed
        cycle's corners and every opened rim edge are measured against exact source."""
        require = self.require
        tid = int(t['id'])
        before, after = _boundary(source), _boundary(output)
        after_near = _Near({p for points, _ in after for p in points}, reach)
        before_near = _Near({p for points, _ in before for p in points}, reach)
        closed = opened = 0
        for points, edges in before:
            touches = bool(points & border)
            kept = any(after_near.find(p) for p in points)
            if touches:
                require(kept, 'holeClosure', f'tile {tid}: a boundary cycle touching a delivered border was closed')
                continue
            rank = len(edges) - len(points) + 1
            if kept or dropped(next(iter(points))):
                continue
            if not rank:
                # A boundary path, not a hole (two open runs sewn or simplified away).
                self.stats['vanishedBoundaryPaths'] += 1
                continue
            closed += rank
            witnessed = False
            for f in output:
                if len(set(f)) == 3 and all(p in points for p in f):
                    witnessed = True
                    c = tuple(sum(p[k] for p in f) / 3 for k in range(3))
                    d = self.source_distance(t, c)
                    self.stats['maxClosureLossOverE'] = max(self.stats['maxClosureLossOverE'], d / e)
                    require(d <= e * (1 + 1e-9), 'holeClosure',
                            f'tile {tid}: closed hole patch is {d} from source, above E = {e}')
            self.stats['closuresWithoutPatchWitness'] += not witnessed
        for points, edges in after:
            if points & border or any(before_near.find(p) for p in points):
                continue
            rank = len(edges) - len(points) + 1
            if not rank:
                self.stats['newBoundaryPaths'] += 1
                continue
            opened += rank
            for a, b in edges:
                m = tuple((a[k] + b[k]) / 2 for k in range(3))
                d = self.source_distance(t, m)
                self.stats['maxOpeningRimOverE'] = max(self.stats['maxOpeningRimOverE'], d / e)
                require(d <= e * (1 + 1e-9), 'holeOpening',
                        f'tile {tid}: opened hole rim is {d} from source, above E = {e}')
        self.stats['closedHoles'] += closed
        self.stats['openedHoles'] += opened
        require(closed == published['closedHoles'] and opened == published['openedHoles'], 'topologyReceipt',
                f'tile {tid}: re-derived {closed} closed / {opened} opened holes, audit reports '
                f'{published["closedHoles"]} / {published["openedHoles"]}')

    def source_distance(self, t, p):
        from oracle import distance2
        _, want = self.store.nearest(p, t['lo'], t['hi'], distance2)
        return math.sqrt(distance2(p, want['xyz']))

    # -- omission --------------------------------------------------------------
    def omission(self, omitted):
        require = self.require
        cells = collections.Counter()
        sources = collections.Counter()
        frontier = collections.defaultdict(set)
        for t in self.tiles:
            if t['contents'] or t['external']:
                continue
            key = (t['lo'], t['hi'])
            cells[key] += 1
            rendered = [c for c in self.cell_children(t) if c['contents']]
            children = self.cell_children(t)
            sources[key] += all(c['contents'] for c in children)
            frontier[key] |= {(c['lo'], c['hi']) for c in rendered}
        claimed = collections.Counter()
        claimed_sources = collections.Counter()
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
            for f in r['frontier']:
                require((f['leafBegin'], f['leafEnd']) in frontier[key], 'omissionReport',
                        f'omission frontier {f} is no rendered cell child of the omitted cell')
        require(claimed == cells, 'omissionReport',
                f'audit omitted cells {sorted(claimed.items())} != archive {sorted(cells.items())}')
        require(claimed_sources == sources, 'omissionReport', 'audit omission sources differ from the archive')
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
