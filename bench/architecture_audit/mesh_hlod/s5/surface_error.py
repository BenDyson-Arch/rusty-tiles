"""Independent exact, two-sided source check on the producer's captured f32 frame.

A whole panel passes when all corners are within G of one closed target
triangle. Its G-neighbourhood is convex. Every failed corner/centroid search
exhausts an exact broad phase and is a genuine lower witness. A panel whose
proof runs out of work is unresolved and refuses the audit.
"""
import json
from fractions import Fraction
from spatial import WorkLimit, f32_point

POLICY = 'hlod-certified-decoded-surface-v1'
MAX_FACES = 131072
MAX_DEPTH = 32


def bounds(points):
    points = iter(points)
    first = next(points)
    lo, hi = list(first), list(first)
    for p in points:
        for k in range(3):
            lo[k] = min(lo[k], p[k])
            hi[k] = max(hi[k], p[k])
    return tuple(lo), tuple(hi)


def union(a, b):
    if a is None:
        return b
    if b is None:
        return a
    return bounds((*a, *b))


def captured(face):
    return tuple(f32_point(p) for p in face)


def source_rows(store, lo, hi):
    for data, in store.db.execute('SELECT data FROM source WHERE leaf>=? AND leaf<?', (lo, hi)):
        store.work.charge()
        yield captured(json.loads(data)['xyz'])


def exact(face):
    return tuple(tuple(Fraction(v) for v in p) for p in face)


class Index:
    def __init__(self, faces, work):
        self.work = work
        self.shapes = {tuple(sorted(f)) for f in faces}
        items = [(bounds(f), f) for f in faces]
        self.root = self.build(items)

    def build(self, items):
        self.work.charge(len(items))
        extent = bounds(p for box, _ in items for p in box)
        if len(items) <= 8:
            return extent, items, ()
        lo, hi = extent
        axis = max(range(3), key=lambda k: hi[k] - lo[k])
        items.sort(key=lambda item: item[0][0][axis] + item[0][1][axis])
        middle = len(items) // 2
        return extent, (), (self.build(items[:middle]), self.build(items[middle:]))

    def candidates(self, point, threshold):
        def possible(box):
            lo, hi = box
            return sum(max(0, lo[k]-point[k], point[k]-hi[k])**2 for k in range(3)) <= threshold
        pending = [self.root]
        while pending:
            box, items, children = pending.pop()
            self.work.charge()
            if possible(box):
                for box, face in items:
                    self.work.charge()
                    if possible(box):
                        yield face
                pending.extend(children)


def directed(domain, target, g, work, distance2, require, label):
    if not domain or not target:
        require(False, 'sourceSurfaceError', label + ' empty support')
    target = Index(target, work)
    threshold = Fraction(g)**2
    covered = 0
    for triangle in domain:
        work.charge()
        if tuple(sorted(triangle)) in target.shapes:
            covered += 1
            continue
        todo = [(triangle, 0)]
        while todo:
            face, depth = todo.pop()
            work.charge()
            covered_face = False
            # Try each target face at corner 0. A failed corner 0 search proves
            # an actual violation regardless of the chosen subdivision.
            first_hit = False
            for other in target.candidates(face[0], threshold):
                work.charge()
                if distance2(face[0], other) <= threshold:
                    first_hit = True
                    work.charge(2)
                    if all(distance2(p, other) <= threshold for p in face[1:]):
                        covered_face = True
                        break
            require(first_hit, 'sourceSurfaceError', f'{label} outside G at {face[0]}')
            if covered_face:
                covered += 1
                continue
            center = tuple(sum(p[k] for p in face)/3 for k in range(3))
            for p in (*face[1:], center):
                hit = False
                for other in target.candidates(p, threshold):
                    work.charge()
                    if distance2(p, other) <= threshold:
                        hit = True
                        break
                require(hit, 'sourceSurfaceError', f'{label} outside G at {p}')
            if depth == MAX_DEPTH:
                raise WorkLimit(f'{label} continuous source-error cover unresolved at depth {depth}')
            # Longest-edge bisection covers the closed panel, including a line or point.
            i = max(range(3), key=lambda j: sum((a-b)**2 for a,b in
                        zip(face[j], face[(j+1)%3])))
            a,b,c = face[i],face[(i+1)%3],face[(i+2)%3]
            m = tuple((x+y)/2 for x,y in zip(a,b))
            todo.extend([((a,m,c),depth+1), ((m,b,c),depth+1)])
    return covered


def inspect(store, tiles, exact_cover, distance2, require, identity):
    # This source-aware mode explicitly prices the local-metre domain. The
    # existing S5 source-material and root-frame admission precede it.
    require(all(t['relative'] == identity for t in tiles), 'oracleDomain',
            'surface-error verification requires the common local capture frame')
    extrema = panels = direct = 0
    below = {}
    for t in reversed(tiles):
        key = t['id']
        if not t['children']:
            source_box = None
            for f in source_rows(store, t['lo'], t['hi']):
                source_box = union(source_box, bounds(f))
        else:
            source_box = None
            for child in t['children']:
                source_box = union(source_box, below.pop(child['id']))
        below[key] = source_box
        if not t['rendered'] or not t['children']:
            continue
        proxy_box = None
        proxy = []
        for row in store.faces(int(key)):
            store.work.charge()
            face = captured(row['xyz'])
            proxy_box = union(proxy_box, bounds(face))
            if exact_cover:
                if len(proxy) == MAX_FACES:
                    raise WorkLimit(f'tile {key} exceeds {MAX_FACES} decoded faces')
                proxy.append(exact(face))
        require(proxy_box is not None and source_box is not None, 'sourceSurfaceError',
                f'empty support {key}')
        g = Fraction(t['g'])
        for a,b in zip((*source_box[0],*source_box[1]), (*proxy_box[0],*proxy_box[1])):
            require(abs(Fraction(a)-Fraction(b)) <= g, 'sourceSurfaceError',
                    f'tile {key} exceeds G at a support extremum')
        extrema += 1
        if exact_cover:
            count = store.db.execute('SELECT count(*) FROM source WHERE leaf>=? AND leaf<?',
                                     (t['lo'],t['hi'])).fetchone()[0]
            if count > MAX_FACES:
                raise WorkLimit(f'tile {key} exceeds {MAX_FACES} faces per exact reference')
            source = [exact(f) for f in source_rows(store, t['lo'], t['hi'])]
            for a,b,direction in [(source,proxy,'source to proxy'),(proxy,source,'proxy to source')]:
                panels += directed(a,b,g,store.work,distance2,require,f'tile {key} {direction}')
            direct += 1
    return dict(extremaTiles=extrema, completeTwoSidedTiles=direct, completePanels=panels)
