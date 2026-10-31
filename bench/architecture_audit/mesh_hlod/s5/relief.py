"""D11 border relief (amendment A1), re-derived from source incidence.

The producer certifies each shared arc at depth d at E_d/16 x 2^step, step in
[0, 8]. A coarse cell whose depth-d contract (chords and points) holds more than
85% of the hard cap requests the least step that brings it to 75%. An arc takes
its owners' largest request, and never less than its finer level's tolerance
(step >= finer step - 1). This module re-derives those steps independently.

From the verified leaves' canonical incidence only, for every depth from the
deepest up, it rebuilds:
- the shared border graph (items held by two or more leaves; owners in leaf order);
- live junctions: degree, owner changes, terminal leaves, source-open ends, face
  retention;
- the source chains in the producer's walk order, with cycle anchors, open-chain
  witnesses and, on relieved chains, isolated ears;
- the corridor greedy in IEEE double, as the producer computes it.

Then it counts each coarse cell's items and recomputes the trigger, requests and
monotone minimum. Every delivered tile stands for the frozen cells of its single-child
chain, from its audit depth to the depth it was built at (`encodedDepth`). An omitted
cell's chain comes from the omitted records over its leaf interval. A leaf lies one level
below its parent's chain. Each child's chain must start one level below its parent's,
from the root at 0. A link placeholder, or labels that do not fit, leave the steps
uninspected.
"""
import collections
import json
import math
import struct

TRIGGER, TARGET, STEPS = 85, 75, 8
EPS = 2.0 ** -52


def _bits(blob):
    return struct.unpack('<3I', blob)


def _rounding(a, b):
    return max([1.0] + [abs(x) for x in a] + [abs(x) for x in b]) * (128.0 * EPS)


def shortcuts(first, last, epsilon, point, retained, emit):
    """`curve::shortcuts`: greedy coordinate corridors over ordinal parameters."""
    guard = 0.0
    for i in range(first, last + 1):
        p = point(i)
        guard = max(guard, _rounding(p, p))
    start = first
    while start < last:
        a = point(start)
        lower = [-math.inf] * 3
        upper = [math.inf] * 3
        end, i = start, start + 1
        while i <= last:
            b = point(i)
            radius = max(epsilon - 2.0 * guard, 0.0) / math.sqrt(3.0)
            n = float(i - start)
            fits = epsilon >= 2.0 * guard
            for axis in range(3):
                delta = b[axis] - a[axis]
                lower[axis] = max(lower[axis], math.nextafter((delta - radius) / n, math.inf))
                upper[axis] = min(upper[axis], math.nextafter((delta + radius) / n, -math.inf))
                slope = delta / n
                fits = fits and lower[axis] <= slope <= upper[axis]
            if retained(i):
                if end != start and not fits:
                    break
                end = i
            i += 1
        if end == start:
            raise ValueError('source arc has no retained endpoint')
        emit(start, end)
        start = end


class Relief:
    def __init__(self, store, tiles, leaves, cap, extent, require, stats):
        self.store, self.require, self.stats = store, require, stats
        self.cap, self.extent = cap, extent
        self.tiles, self.leaves = tiles, leaves

    # -- the frozen tree from the delivered one --------------------------------
    def domain(self, records, omitted):
        """Frozen depths [top, bottom] of every delivered tile (its single-child chain).
        False when a link placeholder hides the tree or the labels do not fit."""
        labels = frozen(self.tiles, records, omitted)
        if labels is None:
            return False
        self.top, self.bottom = labels
        return True

    def cells(self):
        """Per leaf index: its ancestor-or-self tile at every frozen depth (the leaf itself
        at and below its own depth, as the producer's leaf table)."""
        self.height = max(self.bottom.values())
        self.cell = {}
        self.leaf_depth = {}
        self.coarse = {}
        for leaf in self.leaves:
            chain, t = [], leaf
            while t is not None:
                chain.append(t)
                t = t['parent']
            ids = [None] * (self.height + 1)
            for t in chain:
                tid = int(t['id'])
                for d in range(self.top[tid], self.bottom[tid] + 1):
                    ids[d] = tid
            depth = self.top[int(leaf['id'])]
            self.leaf_depth[leaf['lo']] = depth
            self.cell[leaf['lo']] = [ids[min(d, depth)] for d in range(self.height + 1)]
        for t in self.tiles:
            self.coarse[int(t['id'])] = bool(t['children'])

    # -- shared incidence ------------------------------------------------------
    def graph(self):
        db = self.store.db
        owners = collections.defaultdict(list)
        for point, leaf in db.execute('SELECT point,leaf FROM canonical_owners ORDER BY point,leaf'):
            owners[point].append(leaf)
        keys = {p: _bits(k) for p, k in db.execute('SELECT id,key FROM canonical_points')}
        xyz = {p: struct.unpack('<3f', k) for p, k in db.execute('SELECT id,key FROM canonical_points')}
        edge_leaves = collections.defaultdict(set)
        faces_on = collections.Counter()
        for a, b, leaf, nf, nr in db.execute('SELECT a,b,leaf,nf,nr FROM canonical_edges'):
            edge = (min(a, b), max(a, b))
            edge_leaves[edge].add(leaf)
            faces_on[edge] += nf + nr
        shared_points = sorted((p for p, ls in owners.items() if len(set(ls)) >= 2), key=lambda p: keys[p])
        self.vid = {p: i for i, p in enumerate(shared_points)}
        self.point = [tuple(float(x) for x in xyz[p]) for p in shared_points]
        self.vowners = [sorted(set(owners[p])) for p in shared_points]
        self.vterminal = [min(self.leaf_depth[l] for l in o) for o in self.vowners]
        edges = []
        for (a, b), ls in edge_leaves.items():
            if len(ls) >= 2 and a in self.vid and b in self.vid:
                va, vb = self.vid[a], self.vid[b]
                edges.append((min(va, vb), max(va, vb), sorted(ls)))
        edges.sort(key=lambda e: (e[0], e[1]))
        self.edges = edges
        self.eid = {(a, b): i for i, (a, b, _) in enumerate(edges)}
        self.eterminal = [min(self.leaf_depth[l] for l in e[2]) for e in edges]
        # Adjacency in the producer's linked-list order: the latest edge first.
        self.adjacent = [[] for _ in shared_points]
        for i, (a, b, _) in enumerate(edges):
            self.adjacent[a].insert(0, i)
            self.adjacent[b].insert(0, i)
        # Source-open edge endpoints (one face over all leaves).
        self.open = [False] * len(shared_points)
        for (a, b), n in faces_on.items():
            if n == 1:
                for p in (a, b):
                    if p in self.vid:
                        self.open[self.vid[p]] = True
        self.retain_faces()

    def external_run(self, owners, depth):
        return self.cell[owners[0]][depth] != self.cell[owners[-1]][depth]

    def run(self, owners, depth):
        out = []
        for leaf in owners:
            c = self.cell[leaf][depth]
            if not out or out[-1] != c:
                out.append(c)
        return out

    def retain_faces(self):
        """Face retention (bits 40..48) and isolated ears (48..56): shallowest depth."""
        self.face_from = [0] * len(self.point)
        self.ear_from = [0] * len(self.point)
        for a, b, c in self.store.db.execute('SELECT a,b,c FROM source_corners'):
            ids = [self.vid.get(p) for p in (a, b, c)]
            if any(i is None for i in ids):
                continue
            depth = max(next(d for d in range(self.height + 1) if self.external_run(self.vowners[i], d))
                        for i in ids)
            shared = []
            for x, y in ((ids[0], ids[1]), (ids[1], ids[2]), (ids[2], ids[0])):
                e = self.eid.get((min(x, y), max(x, y)))
                shared.append(e is not None and self.external_run(self.edges[e][2], depth))
            for k, v in enumerate(ids):
                if shared[k] or shared[(k + 2) % 3]:
                    opposite = (k + 1) % 3
                    ear = shared[k] and shared[(k + 2) % 3] and (
                        shared[opposite] or (self.open[ids[opposite]] and self.open[ids[(k + 2) % 3]]))
                    if ear and (self.ear_from[v] == 0 or depth + 1 < self.ear_from[v]):
                        self.ear_from[v] = depth + 1
                    continue
                if self.face_from[v] == 0 or depth + 1 < self.face_from[v]:
                    self.face_from[v] = depth + 1

    # -- one level ------------------------------------------------------------------
    def level(self, depth, requests, count, finer_step, retained_finer):
        """Walk one level. `requests`: node -> step. With `count` (node -> counts per
        step), count chords of chains with a counted owner; otherwise emit. Returns
        (items per node, retained vertex set, per-edge step) when emitting."""
        n = len(self.point)
        active = [1 if self.external_run(e[2], depth) else 0 for e in self.edges]
        live = []
        items = collections.Counter()
        for v in range(n):
            external = self.external_run(self.vowners[v], depth)
            degree, two, terminal = 0, [None, None], False
            for e in self.adjacent[v]:
                if active[e]:
                    if degree < 2:
                        two[degree] = e
                    degree += 1
                    terminal = terminal or depth >= self.eterminal[e]
            kind = int(external)
            face = self.face_from[v] != 0 and depth + 1 >= self.face_from[v]
            if degree == 2 and not terminal and not self.open[v] and not face:
                ra, rb = self.run(self.edges[two[0]][2], depth), self.run(self.edges[two[1]][2], depth)
                if ra == rb and ra == self.run(self.vowners[v], depth):
                    kind = 2
            live.append((two, kind))
            if count is None and external and degree > 0 and kind == 1:
                reached = set()
                for e in self.adjacent[v]:
                    if active[e]:
                        reached.update(self.run(self.edges[e][2], depth))
                for o in self.run(self.vowners[v], depth):
                    if o not in reached:
                        items[o] += 1
            if count is None and external and degree == 0:
                for o in self.run(self.vowners[v], depth):
                    items[o] += 1
        retained = set(range(n)) if retained_finer is None else set()
        edge_step = {}
        moved = set()

        def is_retained(v):
            return retained_finer is None or v in retained_finer

        def chain(start, first_edge):
            arc, at, edge = [], start, first_edge
            while True:
                arc.append((at, edge))
                active[edge] = 2
                a, b, _ = self.edges[edge]
                at = b if a == at else a
                two, kind = live[at]
                if at == start or kind != 2:
                    arc.append((at, None))
                    break
                edge = two[1] if two[0] == edge else two[0]
                if active[edge] != 1:
                    raise ValueError('source arc revisits an edge before its endpoint')
            begin, end = 0, len(arc) - 1
            pieces = [begin, end]
            if at == start:
                count_ = end - begin
                kept = [i for i in range(begin, end) if is_retained(arc[i][0])]
                if len(kept) < 3:
                    raise ValueError('a shared cycle has fewer than three retained vertices')
                origin = kept[0]
                arc = [arc[(origin + i) % count_] for i in range(count_)] + [(arc[origin][0], None)]
                begin, end = 0, count_
                pieces, ordinal = [begin], 0
                for i in range(begin, end):
                    if is_retained(arc[i][0]):
                        if ordinal == len(kept) // 3 or ordinal == 2 * len(kept) // 3:
                            pieces.append(i)
                        ordinal += 1
                pieces.append(end)
            elif end > begin + 1:
                A, B = self.point[arc[0][0]], self.point[arc[end][0]]
                direction = [B[k] - A[k] for k in range(3)]
                length2 = sum(x * x for x in direction)
                middle = begin + (end - begin) // 2
                chosen = None
                for i in range(begin + 1, end):
                    if not is_retained(arc[i][0]):
                        continue
                    p = self.point[arc[i][0]]
                    t = sum((p[k] - A[k]) * direction[k] for k in range(3)) / length2
                    d2 = sum((p[k] - A[k] - t * direction[k]) ** 2 for k in range(3))
                    if chosen is None or d2 > chosen[0] or (d2 == chosen[0] and abs(i - middle) < abs(chosen[1] - middle)):
                        chosen = (d2, i)
                if chosen is not None:
                    pieces.insert(1, chosen[1])
            least = 0
            if finer_step is not None:
                for v, e in arc:
                    if e is not None:
                        least = max(least, finer_step.get(e, 0) - 1)
            a0, b0, owners_ = self.edges[first_edge]
            owners = self.run(owners_, depth)

            def pinned(step):
                p = list(pieces)
                if step > 0:
                    for i in range(begin + 1, end):
                        v = arc[i][0]
                        if is_retained(v) and self.ear_from[v] != 0 and depth + 1 >= self.ear_from[v]:
                            p.append(i)
                    p = sorted(set(p))
                return p

            def chords(step):
                epsilon = self.extent / 1024.0 * 2.0 ** -depth / 16.0 * 2.0 ** step
                out = []
                ps = pinned(step)
                for x, y in zip(ps, ps[1:]):
                    shortcuts(x, y, epsilon, lambda i: self.point[arc[i][0]],
                              lambda i: is_retained(arc[i][0]), lambda s, e: out.append((s, e)))
                return out

            if count is not None:
                if not any(o in count for o in owners):
                    return
                per = [len(chords(max(least, j))) for j in range(STEPS + 1)]
                for o in owners:
                    if o in count:
                        for j in range(STEPS + 1):
                            count[o][j] += per[j]
                return
            step = max([least] + [requests[o] for o in owners if o in requests])
            for s, e in chords(step):
                for o in owners:
                    items[o] += 1
                retained.update((arc[s][0], arc[e][0]))
                for i in range(s + 1, e):
                    moved.add(arc[i][0])
                for i in range(s, e):
                    edge_step[arc[i][1]] = step

        for v in range(n):
            if live[v][1] != 1:
                continue
            for e in self.adjacent[v]:
                if active[e] == 1:
                    chain(v, e)
        for e in range(len(self.edges)):
            if active[e] == 1:
                chain(self.edges[e][0], e)
        if count is not None:
            return None
        # Retained at this depth: everything not moved by a chord.
        retained = {v for v in range(n) if v not in moved}
        return items, retained, edge_step

    def rederive(self):
        """Per depth: (triggered cells, per-edge step)."""
        out = {}
        finer, retained = None, None
        for depth in range(self.height, -1, -1):
            self.store.work.charge()
            items, kept, steps = self.level(depth, {}, None, finer, retained)
            triggered = {o: c for o, c in items.items()
                         if self.coarse.get(o) and self.top[o] <= depth <= self.bottom[o]
                         and c * 100 > TRIGGER * self.cap}
            if triggered:
                counts = {o: [0] * (STEPS + 1) for o in triggered}
                self.level(depth, {}, counts, finer, retained)
                requests = {}
                for o, c in counts.items():
                    at = lambda j: max(triggered[o] + c[j] - c[0], 0)
                    requests[o] = next((j for j in range(1, STEPS + 1) if at(j) * 100 <= TARGET * self.cap), STEPS)
                items, kept, steps = self.level(depth, requests, None, finer, retained)
                self.stats['reliefCellsRederived'] += len(requests)
            out[depth] = (set(triggered), steps)
            finer, retained = steps, kept
        return out



def frozen(tiles, records, omitted):
    """Frozen depths [top, bottom] of every delivered tile, by tile id: a coarse record
    covers its contract depth through its `encodedDepth`; an omitted cell its omitted
    depths from its own on, unless the delivered tree continues the chain below it; a
    leaf lies one level below its parent's chain. None when a link placeholder hides the
    tree or the labels do not fit (each chain starts one level below its parent's)."""
    claimed = collections.defaultdict(set)
    for r in omitted:
        claimed[r['key']['leafBegin'], r['key']['leafEnd']].add(r['key']['depth'])
    top, bottom = {}, {}
    for t in tiles:
        if t.get('external'):
            return None
        tid, parent = int(t['id']), t['parent']
        first = bottom[int(parent['id'])] + 1 if parent is not None else 0
        if not t['children']:
            low = high = first
        elif tid in records:
            r = records[tid]
            low, high = r['contract']['depth'], r['tile'].get('encodedDepth')
            if type(low) is not int or type(high) is not int:
                return None
        else:
            depths = claimed.get((t['lo'], t['hi']), set())
            low = high = first
            if low not in depths:
                return None
            if not any((c['lo'], c['hi']) == (t['lo'], t['hi']) for c in t['children']):
                while high + 1 in depths:
                    high += 1
        if low != first or high < low:
            return None
        top[tid], bottom[tid] = low, high
    return top, bottom


def leaf_depths(tiles, audit_path):
    """Frozen depth per leaf index from the audit sidecar, before the D11 audit joins it:
    a leaf below a collapsed chain is quantized on its frozen depth's grid. Coarse tiles
    carry the sha256 of each payload (`oracle.audit`). None when a coarse payload has no
    unique record or the labels do not fit; the audit then refuses or leaves relief
    uninspected, and delivered depths stand."""
    by_sha, omitted = collections.defaultdict(list), []
    try:
        with open(audit_path, 'rb') as f:
            for line in f:
                if not line.strip():
                    continue
                r = json.loads(line)
                if r.get('kind') == 'tile':
                    by_sha[r['tile']['sha256']].append(r)
                elif r.get('kind') == 'omitted':
                    omitted.append(r)
        records = {}
        for t in tiles:
            if t['children'] and t['contents']:
                shas = t.get('sha256', [])
                if len(shas) != 1 or len(by_sha.get(shas[0], ())) != 1:
                    return None
                records[int(t['id'])] = by_sha[shas[0]][0]
        labels = frozen(tiles, records, omitted)
    except (KeyError, TypeError, ValueError, AttributeError):
        return None
    if labels is None:
        return None
    return {t['lo']: labels[0][int(t['id'])] for t in tiles if not t['children']}


def inspect(store, tiles, leaves, records, omitted, arcs_of, cap, extent, require, stats):
    """Compare every delivered certificate's declared step with the re-derived step of
    the source chain holding its first edge. Returns False when outside the domain."""
    relief = Relief(store, tiles, leaves, cap, extent, require, stats)
    if not relief.domain(records, omitted):
        return False
    stats['reliefChainLevels'] += sum(relief.bottom[i] - relief.top[i] for i in relief.top)
    relief.cells()
    relief.graph()
    derived = relief.rederive()
    for t in tiles:
        tid = int(t['id'])
        r = records.get(tid)
        if r is None:
            continue
        depth = r['contract']['depth']
        triggered, steps = derived[depth]
        for c in r['contract']['certificates']:
            declared = c.get('step', 0)
            if c['endpoints'][0] == c['endpoints'][1]:
                continue
            path = arcs_of(tid, c)
            first = (min(relief.vid[path[0]], relief.vid[path[1]]), max(relief.vid[path[0]], relief.vid[path[1]]))
            want = steps.get(relief.eid[first], 0)
            stats['reliefStepsChecked'] += 1
            require(declared == want, 'borderReliefStep',
                    f'tile {tid}: certificate declares relief step {declared}, source re-derivation gives {want}')
    return True
