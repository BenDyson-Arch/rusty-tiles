"""Independent bounded sampled boundary-distance diagnostic (Python stdlib).

``measure`` takes two COMPLETE, corresponding rendered traces as iterables of
segments, each a pair of XY or XYZ points. Caller owns correspondence, boundary
selection, placement and completeness: this module cannot infer them from a
chord, producer facts, or a proximity radius. Segment order is immaterial.
``trace`` can establish completeness of a selected boundary graph component.

Supported coordinates are finite binary64 values with abs(value) <= 1e100, or
integers exactly representable as such. Dimensions must agree. No epsilon joins
points. There are at most 1,000,000 segments per input, 2..33 equally spaced
samples per segment (including endpoints), and 0..65,536 BVH node visits per
nearest query. Degenerate segments are supported by measure; trace rejects
self-loops. Inputs are copied only up to the cap. BVH RAM is O(n), construction
is O(n log n): one comparison sort followed by balanced midpoint splits and
bottom-up AABBs. A single-axis ordering may reduce pruning quality, never truth.
Samples are streamed; no all-pairs table or archive-sized retained state exists.

Every sample and distance uses exact rational arithmetic on admitted binary64
coordinates. Sample interpolation is rational, so even a rounded midpoint cannot
leave its source segment. For an unfinished query the minimum of visited exact
segment distances AND unresolved subtree-box distances is a lower bound on the
true nearest distance. A visited min-so-far alone would be an UPPER bound.
Squared bounds remain rational until a downward-checked binary64 square root;
underflow can weaken the bound to zero. The maximum sampled bound is therefore
certified <= continuous symmetric Hausdorff distance, even for unfinished queries.
It is NEVER a certificate of the continuous maximum, even when all sampled
nearest queries finish. ``exact_nearest_sampled`` describes rational nearest
search completion, not exact representation by the reported binary64 distance.

Work objects need only ``charge(n=1)``; their exceptions propagate. All input
edges/coordinates, graph incidences, sort comparisons, build nodes, query nodes,
distance evaluations and sample visits are charged. A local 300M-unit cap also
applies without a supplied work object. These are operation counts, not CPU-time
or byte charges. Resource refusal raises WorkLimit (or the caller's exception),
never returns a partial success. Geometry/input refusals return an explicit
status and no numeric bound. This module does not perform I/O or producer imports.
"""

from fractions import Fraction
from functools import cmp_to_key
import heapq
import math


MAX_EDGES = 1_000_000
MAX_WORK = 300_000_000
MAX_QUERY_BUDGET = 65_536
MAX_SAMPLES_PER_SEGMENT = 33


class WorkLimit(ValueError):
    """The diagnostic's local work cap was exceeded."""


class _InputError(ValueError):
    def __init__(self, status, detail):
        self.status, self.detail = status, detail
        super().__init__(detail)


class _Work:
    def __init__(self, external):
        self.external = external
        self.used = 0

    def charge(self, n=1):
        self.used += n
        if self.used > MAX_WORK:
            raise WorkLimit(f'seam diagnostic work exceeds {MAX_WORK}')
        if self.external is not None:
            self.external.charge(n)


def _point(value, work, dimension=None):
    if not isinstance(value, (tuple, list)) or len(value) not in (2, 3):
        raise _InputError('invalid', 'points must be XY or XYZ tuples/lists')
    if dimension is not None and len(value) != dimension:
        raise _InputError('invalid', 'mixed coordinate dimensions')
    result = []
    for x in value:
        work.charge()
        if type(x) not in (int, float):
            raise _InputError('invalid', 'coordinates must be binary64 or exact integers')
        try:
            y = float(x)
        except OverflowError:
            raise _InputError('invalid', 'coordinate outside supported range') from None
        if not math.isfinite(y) or abs(y) > 1e100 or (type(x) is int and y != x):
            raise _InputError('invalid', 'coordinate outside supported exact binary64 domain')
        result.append(0.0 if y == 0 else y)
    return tuple(result)


def _segments(values, work, dimension=None, cap=MAX_EDGES):
    if values is None:
        raise _InputError('missing', 'segment input is missing')
    try:
        iterator = iter(values)
    except TypeError:
        raise _InputError('invalid', 'segments must be iterable') from None
    result = []
    for edge in iterator:
        work.charge()
        if len(result) >= cap:
            raise _InputError('limit', f'edge cap {cap} exceeded')
        if not isinstance(edge, (tuple, list)) or len(edge) != 2:
            raise _InputError('invalid', 'each segment must have two endpoints')
        a = _point(edge[0], work, dimension)
        dimension = len(a)
        b = _point(edge[1], work, dimension)
        result.append((a, b))
    if not result:
        raise _InputError('empty', 'segment input is empty')
    return result


def _refusal(error, work):
    return dict(status=error.status, detail=error.detail, work=work.used)


def trace(edges, start, end, work=None, max_edges=MAX_EDGES):
    """Return the COMPLETE simple chain in start's boundary-graph component.

    Start/end are exact canonical coordinate points, not inferred projections.
    The entire start-connected component must be a simple path whose degree-one
    endpoints are start/end. Branches (even dead ends), duplicate undirected
    edges, self-loops and cycles refuse as ``ambiguous``. Missing endpoints or
    disconnected end refuse as ``missing``. Other disconnected components are
    ignored explicitly via ``ignored_edges``; caller selects the candidate edge
    scope and owns whether those components belong to another interface.
    Success returns ordered ``points`` and ``segments``; refusals return neither.
    """
    meter = _Work(work)
    try:
        if type(max_edges) is not int or not 1 <= max_edges <= MAX_EDGES:
            raise _InputError('invalid', 'max_edges must be in 1..1000000')
        if start is None or end is None:
            raise _InputError('missing', 'canonical endpoints are missing')
        start = _point(start, meter)
        end = _point(end, meter, len(start))
        if start == end:
            raise _InputError('ambiguous', 'equal endpoints do not define a simple chain')
        segments = _segments(edges, meter, len(start), max_edges)
        adjacency = {}
        for a, b in segments:
            meter.charge(2)
            adjacency.setdefault(a, []).append(b)
            adjacency.setdefault(b, []).append(a)
        if start not in adjacency or end not in adjacency:
            raise _InputError('missing', 'canonical endpoint absent from boundary graph')
        # Inspect the whole component, including branches beyond the requested
        # endpoint. Stopping the walk on first reaching end would miss ambiguity.
        seen, pending, incidences = {start}, [start], 0
        while pending:
            p = pending.pop()
            neighbors = adjacency[p]
            meter.charge()
            if len(neighbors) > 2 or p in neighbors or len(set(neighbors)) != len(neighbors):
                raise _InputError('ambiguous', 'branch, duplicate edge or cycle in component')
            for q in neighbors:
                meter.charge()
                incidences += 1
                if q not in seen:
                    seen.add(q)
                    pending.append(q)
        if end not in seen:
            raise _InputError('missing', 'canonical endpoints are disconnected')
        meter.charge(len(seen))
        if any(len(adjacency[p]) != (1 if p in (start, end) else 2) for p in seen):
            raise _InputError('ambiguous', 'component is not a path with the requested endpoints')
        points, previous, current = [start], None, start
        while current != end:
            meter.charge()
            following = next(q for q in adjacency[current] if q != previous)
            points.append(following)
            previous, current = current, following
        if len(points) != len(seen):
            raise _InputError('ambiguous', 'component contains edges outside the chain')
        return dict(status='ok', complete=True, points=points,
                    segments=list(zip(points, points[1:])),
                    edge_count=len(points)-1, ignored_edges=len(segments)-incidences//2,
                    work=meter.used)
    except _InputError as error:
        return dict(_refusal(error, meter), complete=False)


def _box(edge):
    a, b = edge
    return tuple((min(x, y), max(x, y)) for x, y in zip(a, b))


def _bvh(segments, work):
    boxes = [_box(edge) for edge in segments]
    dimension = len(segments[0][0])
    work.charge(len(segments) * dimension)
    low = [min(box[k][0] for box in boxes) for k in range(dimension)]
    high = [max(box[k][1] for box in boxes) for k in range(dimension)]
    work.charge(2 * len(boxes) * dimension)
    axis = max(range(dimension), key=lambda k: high[k]-low[k])

    def compare(i, j):
        work.charge()
        x, y = boxes[i][axis], boxes[j][axis]
        return (x > y) - (x < y)

    order = sorted(range(len(segments)), key=cmp_to_key(compare))
    nodes = []

    def build(lo, hi):
        work.charge()
        if hi-lo == 1:
            index = order[lo]
            nodes.append((boxes[index], index, None, None))
        else:
            middle = (lo+hi)//2
            left, right = build(lo, middle), build(middle, hi)
            a, b = nodes[left][0], nodes[right][0]
            work.charge(dimension)
            box = tuple((min(x[0], y[0]), max(x[1], y[1])) for x, y in zip(a, b))
            nodes.append((box, None, left, right))
        return len(nodes)-1

    root = build(0, len(order))
    return nodes, root


def _rational(point):
    return tuple(Fraction(x) for x in point)


def _box_squared(p, box, work):
    work.charge(len(p))
    return sum((max(Fraction(lo)-x, 0, x-Fraction(hi)))**2
               for x, (lo, hi) in zip(p, box))


def _segment_squared(p, edge, work):
    work.charge(1 + len(p))
    a, b = map(_rational, edge)
    d = tuple(y-x for x, y in zip(a, b))
    v = tuple(x-y for x, y in zip(p, a))
    length = sum(x*x for x in d)
    t = max(0, min(1, sum(x*y for x, y in zip(v, d))/length)) if length else 0
    return sum((x-t*y)**2 for x, y in zip(v, d))


def _nearest(p, segments, tree, budget, work):
    nodes, root = tree
    frontier = [(_box_squared(p, nodes[root][0], work), root)]
    best, visited = None, 0
    while frontier:
        bound, index = frontier[0]
        if best is not None and bound >= best:
            # Heap minimum proves ALL remaining subtrees cannot improve best.
            return best, True, visited
        if visited >= budget:
            # Each subtree's box contains all its segments. The minimum here
            # covers every segment, including those never evaluated exactly.
            return min(best, bound) if best is not None else bound, False, visited
        heapq.heappop(frontier)
        visited += 1
        work.charge()
        _, leaf, left, right = nodes[index]
        if leaf is not None:
            distance = _segment_squared(p, segments[leaf], work)
            best = distance if best is None else min(best, distance)
        else:
            for child in (left, right):
                distance = _box_squared(p, nodes[child][0], work)
                if best is None or distance < best:
                    heapq.heappush(frontier, (distance, child))
    return best, True, visited


def _sqrt_lower(squared):
    value = math.sqrt(float(squared))
    while Fraction(value)**2 > squared:
        value = math.nextafter(value, 0.0)
    return value


def _direction(source, target, tree, budget, samples, work):
    maximum, unresolved, queries, visits = Fraction(0), 0, 0, 0
    for edge in source:
        work.charge()
        a, b = map(_rational, edge)
        for i in range(samples):
            work.charge()
            t = Fraction(i, samples-1)
            p = tuple(x+t*(y-x) for x, y in zip(a, b))
            squared, complete, used = _nearest(p, target, tree, budget, work)
            maximum = max(maximum, squared)
            unresolved += not complete
            queries += 1
            visits += used
    return maximum, dict(sampled_lower_bound=_sqrt_lower(maximum),
                         sampled_lower_bound_certified=True,
                         exact_nearest_sampled=unresolved == 0,
                         unresolved_queries=unresolved, queries=queries,
                         node_visits=visits)


def measure(left_segments, right_segments, work=None, query_budget=256,
            samples_per_segment=3):
    """Bound continuous symmetric Hausdorff from below using fixed samples.

    ``status`` is ok, unresolved, missing, empty, invalid or limit. Valid results
    have two ``directions`` (left-to-right, right-to-left), unresolved query count
    and ``sampled_lower_bound_certified=True``. Refusals have a None bound and
    None query counts, with ``sampled_lower_bound_certified=False``.
    A budget of zero is valid: root-box bounds remain certified, all queries are
    unresolved. Each query has its own budget. ``work`` exhaustion always raises.
    ``continuous_maximum_certified`` is always False, including exact sample
    completion. No upper bound on the continuous distance is advertised.
    """
    meter = _Work(work)
    base = dict(continuous_maximum_certified=False, sampled_lower_bound=None,
                sampled_lower_bound_certified=False,
                exact_nearest_sampled=False, unresolved_queries=None, queries=None)
    try:
        if type(query_budget) is not int or not 0 <= query_budget <= MAX_QUERY_BUDGET:
            raise _InputError('invalid', 'query_budget must be in 0..65536')
        if (type(samples_per_segment) is not int or
                not 2 <= samples_per_segment <= MAX_SAMPLES_PER_SEGMENT):
            raise _InputError('invalid', 'samples_per_segment must be in 2..33')
        left = _segments(left_segments, meter)
        right = _segments(right_segments, meter, len(left[0][0]))
        left_tree, right_tree = _bvh(left, meter), _bvh(right, meter)
        a, forward = _direction(left, right, right_tree, query_budget, samples_per_segment, meter)
        b, reverse = _direction(right, left, left_tree, query_budget, samples_per_segment, meter)
        unresolved = forward['unresolved_queries'] + reverse['unresolved_queries']
        return dict(base, status='unresolved' if unresolved else 'ok',
                    sampled_lower_bound=_sqrt_lower(max(a, b)),
                    sampled_lower_bound_certified=True,
                    exact_nearest_sampled=unresolved == 0, unresolved_queries=unresolved,
                    queries=forward['queries'] + reverse['queries'],
                    directions=[forward, reverse], left_edges=len(left), right_edges=len(right),
                    samples_per_segment=samples_per_segment, query_budget=query_budget,
                    work=meter.used)
    except _InputError as error:
        return dict(base, **_refusal(error, meter))


def inspect_open_edges(store, tiles, leaves, *, pair_limit=4096):
    """Source-aware, exact-anchor correspondence; never proximity matching.

    Candidate supports are incidence-one source edges whose two keys both
    occur outside a rendered ownership cut. Partner cuts must be disjoint and
    contain both exact endpoint keys. They are candidates, not asserted cracks.
    On each side, remove edges touching OTHER shared source anchors, then require
    a complete unique source-open boundary chain AND a complete unique delivered
    boundary chain. This defines the measured interval between consecutive shared
    anchors. Cycles, alternative paths, branches and missing anchors are explicitly
    unmeasured. No chord is inserted and no other geometric neighbor is guessed.

    The sampled bound covers only these established correspondences, never all
    open edges, arbitrary T contacts or unassociated source boundaries. A null
    maximum means no measured correspondence, not a zero crack width. Pair cap
    omissions, ambiguous traces and unfinished BVH queries have separate counts.
    SQLite owns pair deduplication; only two bounded trace graphs are in RAM.
    """
    import struct
    from spatial import MAX_QUERY_CANDIDATES, WorkLimit
    if type(pair_limit) is not int or not 0 <= pair_limit <= 4096:
        raise ValueError('seam diagnostic pair_limit must be in 0..4096')
    db=store.db
    db.executescript('''
      CREATE TABLE diagnostic_pairs(x INTEGER,y INTEGER,a INTEGER,b INTEGER,
        PRIMARY KEY(x,y,a,b)) WITHOUT ROWID;
      CREATE TABLE shared_anchors(point INTEGER PRIMARY KEY);
      INSERT INTO shared_anchors SELECT point FROM canonical_owners GROUP BY point HAVING count(*)>1;
    ''')
    byid={int(t['id']):t for t in tiles}
    def ancestors(point):
        result=set()
        for (leaf,) in db.execute('SELECT leaf FROM canonical_owners WHERE point=?',(point,)):
            store.work.charge()
            t=leaves[leaf]
            while t is not None:
                store.work.charge()
                if t['rendered']:result.add(int(t['id']))
                t=t['parent']
        return result
    for tile,a,b in db.execute('SELECT tile,a,b FROM requirements WHERE kind=2'):
        store.work.charge()
        t=byid[tile]
        for partner in ancestors(a)&ancestors(b):
            store.work.charge()
            other=byid[partner]
            if t['hi']<=other['lo'] or other['hi']<=t['lo']:
                db.execute('INSERT OR IGNORE INTO diagnostic_pairs VALUES (?,?,?,?)',(*sorted((tile,partner)),a,b))
    stats=dict(candidatePairs=0,measuredPairs=0,ambiguousTracePairs=0,
               positiveSampledWidthPairs=0,
               unmeasuredTracePairs=0,pairLimitUnmeasured=0,unresolvedQueries=0,
               maximumExemptCrackWidthSampledLowerBound=None,
               sampledLowerBoundCertified=False,continuousMaximumCertified=False,
               completeSampledNearest=True,pairLimit=pair_limit,
               samplesPerSegment=3,queryNodeBudget=256,
               scope='symmetric Hausdorff lower bounds on complete exact-anchor source-open boundary correspondences only; no near-neighbor or arbitrary T-contact census')
    def graph(tile,a,b,delivered):
        # The graph boundary is cut only at independently established shared
        # source anchors. Cut endpoints remain; no inferred chord is added.
        endpoints={store.canonical_xyz(a),store.canonical_xyz(b)}
        anchor_cache={}
        def other_anchor(p):
            if p in endpoints:return False
            if p not in anchor_cache:
                store.work.charge()
                packed=struct.pack('<3d',*p)
                anchor_cache[p]=db.execute('SELECT 1 FROM canonical_points p JOIN shared_anchors s '
                    'ON s.point=p.id WHERE p.xyz=? LIMIT 1',(packed,)).fetchone() is not None
            return anchor_cache[p]
        if delivered:
            rows=db.execute('SELECT a,b FROM rendered_borders WHERE tile=?',(int(tile['id']),))
        else:
            # Read every open row before descendant filtering, so broad-phase
            # exclusions cannot hide work. incidence=1 has exactly one owner.
            rows=db.execute('SELECT p.xyz,q.xyz,i.leaf FROM incidence i '
                'JOIN canonical_points p ON p.id=i.a JOIN canonical_points q ON q.id=i.b WHERE i.faces=1')
        result=[]
        for count,row in enumerate(rows,1):
            store.work.charge()
            if count>MAX_QUERY_CANDIDATES:
                raise WorkLimit('seam trace candidate limit')
            if not delivered and not tile['lo']<=row[2]<tile['hi']:continue
            x,y=(struct.unpack('<3d',v) for v in row[:2])
            if not other_anchor(x) and not other_anchor(y):result.append((x,y))
            if len(result)>MAX_EDGES:
                raise WorkLimit('seam trace edge limit')
        return trace(result,*tuple(store.canonical_xyz(p) for p in (a,b)),work=store.work)
    for x,y,a,b in db.execute('SELECT x,y,a,b FROM diagnostic_pairs'):
        store.work.charge()
        stats['candidatePairs']+=1
        if stats['candidatePairs']>pair_limit:
            stats['pairLimitUnmeasured']+=1
            continue
        traces=[]
        for tile in (byid[x],byid[y]):
            original=graph(tile,a,b,False)
            actual=graph(tile,a,b,True) if original['status']=='ok' else original
            traces.append(actual)
        if any(t['status']=='ambiguous' for t in traces):
            stats['ambiguousTracePairs']+=1
            continue
        if any(t['status']!='ok' for t in traces):
            stats['unmeasuredTracePairs']+=1
            continue
        result=measure(traces[0]['segments'],traces[1]['segments'],work=store.work)
        if result['status'] not in ('ok','unresolved'):
            stats['unmeasuredTracePairs']+=1
            continue
        stats['measuredPairs']+=1
        stats['unresolvedQueries']+=result['unresolved_queries']
        stats['completeSampledNearest'] &= result['exact_nearest_sampled']
        value=result['sampled_lower_bound']
        prior=stats['maximumExemptCrackWidthSampledLowerBound']
        stats['maximumExemptCrackWidthSampledLowerBound']=value if prior is None else max(prior,value)
        stats['sampledLowerBoundCertified']=True
        if value>0:
            stats['positiveSampledWidthPairs']+=1
            for tile in (x,y):
                db.execute('INSERT OR IGNORE INTO exemptions VALUES (?,?,?,?)',(tile,a,b,'crack'))
    stats['traceMeasurementStatus']=('unmeasured' if not stats['measuredPairs'] else
        'partial' if stats['ambiguousTracePairs']+stats['unmeasuredTracePairs']+stats['pairLimitUnmeasured'] else 'sampled')
    return stats
