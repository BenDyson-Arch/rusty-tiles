"""Independent bounded geometry indexes; SQLite owns archive-sized state.

No producer census, interface lock, sampled error or tile extras enter this index.
The R-trees are broad phases only: all geometric decisions use binary64 values.
"""
import collections
import heapq
import itertools
import json
import math
import sqlite3
import struct

MAX_QUERY_CANDIDATES = 2_000_000
MAX_CONTACTS_PER_EDGE = 1_000_000


class WorkLimit(ValueError):
    pass


class Work:
    def __init__(self, limit=300_000_000):
        self.limit, self.used = limit, 0

    def charge(self, n=1):
        self.used += n
        if self.used > self.limit:
            raise WorkLimit(f'oracle candidate work {self.used} exceeds {self.limit}')


def bounds(points):
    return tuple(v for k in range(3) for v in
                 (min(p[k] for p in points), max(p[k] for p in points)))


def box_distance(p, box):
    return sum(max(box[2*k]-p[k], 0, p[k]-box[2*k+1])**2 for k in range(3))


class PointIndex:
    """Balanced BVH for one decoded tile; exact tolerant point matching."""
    def __init__(self, points, work):
        self.points, self.work = list(points), work
        def build(ids):
            box = bounds([self.points[i] for i in ids])
            if len(ids) <= 16:
                return box, ids, None
            axis = max(range(3), key=lambda k: box[2*k+1]-box[2*k])
            ids.sort(key=lambda i: self.points[i][axis])
            mid = len(ids)//2
            return box, build(ids[:mid]), build(ids[mid:])
        self.root = build(list(range(len(self.points)))) if self.points else None

    def matches(self, point, tolerance):
        pending = [self.root] if self.root else []
        radius2 = sum(x*x for x in tolerance)
        while pending:
            box, left, right = pending.pop()
            self.work.charge()
            if box_distance(point, box) > radius2:
                continue
            if right is None:
                for i in left:
                    self.work.charge()
                    if all(abs(point[k]-self.points[i][k]) <= tolerance[k] for k in range(3)):
                        yield i
            else:
                pending.extend((left, right))


class Store:
    def __init__(self, path, work=None):
        self.db = sqlite3.connect(path)
        self.work = work or Work()
        self.source_bounds = {}
        self.leaf_boxes = {}
        self.db.executescript('''
          PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;
          PRAGMA temp_store=FILE; PRAGMA cache_size=-65536;
          CREATE TABLE faces(tile INTEGER, data TEXT);
          CREATE INDEX faces_tile ON faces(tile);
          CREATE TABLE source(id INTEGER PRIMARY KEY, leaf INTEGER, key TEXT UNIQUE, data TEXT);
          CREATE INDEX source_leaf ON source(leaf);
          CREATE VIRTUAL TABLE source_box USING rtree(id,x0,x1,y0,y1,z0,z1);
          CREATE TABLE points(id INTEGER PRIMARY KEY, xyz BLOB UNIQUE);
          CREATE TABLE owners(point INTEGER, leaf INTEGER, PRIMARY KEY(point,leaf)) WITHOUT ROWID;
          CREATE VIRTUAL TABLE point_box USING rtree(id,x0,x1,y0,y1,z0,z1);
          CREATE TABLE edges(a INTEGER,b INTEGER,leaf INTEGER,PRIMARY KEY(a,b,leaf)) WITHOUT ROWID;
          CREATE INDEX edges_b ON edges(b,a,leaf);
          CREATE TABLE requirements(tile INTEGER,kind INTEGER,a INTEGER,b INTEGER,
            PRIMARY KEY(tile,kind,a,b)) WITHOUT ROWID;
          CREATE TABLE pairs(a INTEGER,b INTEGER,PRIMARY KEY(a,b)) WITHOUT ROWID;
          CREATE VIRTUAL TABLE leaf_box USING rtree(id,x0,x1,y0,y1,z0,z1);
        ''')

    def close(self):
        self.db.close()

    def faces(self, tile):
        return (json.loads(r[0]) for r in self.db.execute('SELECT data FROM faces WHERE tile=?', (tile,)))

    def put_faces(self, tile, faces):
        self.db.executemany('INSERT INTO faces VALUES (?,?)',
                            ((tile,json.dumps(f,separators=(',',':'))) for f in faces))

    def point(self, xyz):
        packed = struct.pack('<3d', *(0.0 if x == 0 else x for x in xyz))
        row = self.db.execute('SELECT id FROM points WHERE xyz=?', (packed,)).fetchone()
        if row:
            return row[0]
        i = self.db.execute('INSERT INTO points(xyz) VALUES (?)', (packed,)).lastrowid
        self.db.execute('INSERT INTO point_box VALUES (?,?,?,?,?,?,?)',
                        (i,xyz[0],xyz[0],xyz[1],xyz[1],xyz[2],xyz[2]))
        return i

    def xyz(self, point):
        return struct.unpack('<3d', self.db.execute('SELECT xyz FROM points WHERE id=?', (point,)).fetchone()[0])

    def owners(self, point):
        return {r[0] for r in self.db.execute('SELECT leaf FROM owners WHERE point=?', (point,))}

    def source_face(self, key, want, leaf):
        self.work.charge()
        try:
            i = self.db.execute('INSERT INTO source(leaf,key,data) VALUES (?,?,?)',
                (leaf,key,json.dumps(want,separators=(',',':')))).lastrowid
        except sqlite3.IntegrityError:
            return False
        self.db.execute('INSERT INTO source_box VALUES (?,?,?,?,?,?,?)', (i,*bounds(want['xyz'])))
        ids = [self.point(p) for p in want['xyz']]
        self.db.executemany('INSERT OR IGNORE INTO owners VALUES (?,?)', ((p,leaf) for p in ids))
        self.db.executemany('INSERT OR IGNORE INTO edges VALUES (?,?,?)',
                            ((*sorted((ids[k],ids[(k+1)%3])),leaf) for k in range(3)))
        return True

    def leaf_bounds(self, leaf):
        row = self.db.execute('SELECT min(x0),max(x1),min(y0),max(y1),min(z0),max(z1) '
                             'FROM source_box JOIN source USING(id) WHERE leaf=?', (leaf,)).fetchone()
        if row[0] is not None:
            self.db.execute('INSERT INTO leaf_box VALUES (?,?,?,?,?,?,?)', (leaf,*row))
            self.leaf_boxes[leaf]=row

    def nearest(self, p, lo, hi, distance2):
        # Expanding cubes are complete once the best triangle lies closer than
        # the next cube boundary. Descendant filtering uses DFS leaf intervals.
        if (lo,hi) not in self.source_bounds:
            self.work.charge(hi-lo)
            boxes=[self.leaf_boxes[i] for i in range(lo,hi) if i in self.leaf_boxes]
            self.source_bounds[lo,hi]=(tuple(v for k in range(3) for v in
                (min(b[2*k] for b in boxes),max(b[2*k+1] for b in boxes))) if boxes else (None,)*6)
        box = self.source_bounds[lo,hi]
        if box[0] is None:
            raise ValueError('coarse content has no verified descendant source faces')
        extent = max(box[2*k+1]-box[2*k] for k in range(3))
        radius = max(math.sqrt(box_distance(p,box)), extent/1024, 1e-12)
        seen, best, answer = set(), math.inf, None
        candidates=0
        while True:
            params = tuple(v for k in range(3) for v in (p[k]+radius,p[k]-radius))
            rows = self.db.execute('SELECT source.id,leaf,key,data FROM source_box JOIN source USING(id) '
                'WHERE x0<=? AND x1>=? AND y0<=? AND y1>=? AND z0<=? AND z1>=?', params)
            for i,leaf,key,data in rows:
                self.work.charge()
                candidates+=1
                if candidates>MAX_QUERY_CANDIDATES:
                    raise WorkLimit('nearest source broad-phase candidate limit')
                # Charge even excluded descendant ranges. Filtering in SQL
                # hid arbitrary overlap work from both work/candidate caps.
                if not lo<=leaf<hi:
                    continue
                if i in seen:
                    continue
                seen.add(i)
                if len(seen)>MAX_QUERY_CANDIDATES:
                    raise WorkLimit('nearest source query candidate limit')
                want = json.loads(data)
                d = distance2(p,want['xyz'])
                # Generator writes source_order; analytic manifests preserve
                # original source order through expanded ordinal and triangle.
                identity = tuple(map(int,key.split(',')))
                rank = (want['expanded_ordinal'],identity[3],want.get('source_rank',0))
                candidate = d,rank
                if answer is None or candidate < best:
                    best, answer = candidate,(identity,want)
            if answer is not None and best[0] <= radius*radius:
                return answer
            radius *= 2
            if not math.isfinite(radius):
                raise ValueError('unbounded nearest source query')

    def census(self, tiles, leaves, require):
        chains = {}
        for leaf,t in enumerate(leaves):
            chain = []
            while t is not None:
                chain.append(t)
                t = t['parent']
            chains[leaf] = chain
        def inside(t, owner):
            return t['lo'] <= owner < t['hi']
        def demand(owners, kind, a, b=-1):
            for owner in owners:
                for t in chains[owner]:
                    self.work.charge()
                    if t['rendered'] and any(not inside(t,o) for o in owners):
                        self.db.execute('INSERT OR IGNORE INTO requirements VALUES (?,?,?,?)', (int(t['id']),kind,a,b))
        def pairs(owners):
            for a,b in itertools.combinations(sorted(owners),2):
                for x,y in itertools.product(chains[a],chains[b]):
                    self.work.charge()
                    if x['hi']<=y['lo'] or y['hi']<=x['lo']:
                        self.db.execute('INSERT OR IGNORE INTO pairs VALUES (?,?)', tuple(sorted((int(x['id']),int(y['id'])))))
        corners = contacts = 0
        for (p,) in self.db.execute('SELECT point FROM owners GROUP BY point HAVING count(*)>1'):
            owners = self.owners(p)
            corners += 1
            demand(owners,0,p)
            pairs(owners)
        for a,b,owner_text in self.db.execute('SELECT a,b,group_concat(leaf) FROM edges GROUP BY a,b'):
            self.work.charge()
            owners = set(map(int,owner_text.split(',')))
            if len(owners)>1:
                demand(owners,1,a,b)
                pairs(owners)
            x,y = self.xyz(a),self.xyz(b)
            d = [y[k]-x[k] for k in range(3)]
            l2 = sum(v*v for v in d)
            if not l2:
                continue
            eps = 1e-7*math.sqrt(l2)
            box = tuple(v for k in range(3) for v in (min(x[k],y[k])-eps,max(x[k],y[k])+eps))
            params = tuple(v for k in range(3) for v in (box[2*k+1],box[2*k]))
            nearby=set()
            for (leaf,) in self.db.execute('SELECT id FROM leaf_box WHERE x0<=? AND x1>=? AND y0<=? AND y1>=? AND z0<=? AND z1>=?',params):
                self.work.charge()
                if len(nearby)>=MAX_QUERY_CANDIDATES:
                    raise WorkLimit('edge leaf-box query candidate limit')
                nearby.add(leaf)
            if not nearby-owners:
                continue
            split = [(0,a,self.owners(a)),(1,b,self.owners(b))]
            candidates = 0
            for (p,) in self.db.execute('SELECT id FROM point_box WHERE x0<=? AND x1>=? AND y0<=? AND y1>=? AND z0<=? AND z1>=?', params):
                self.work.charge()
                candidates += 1
                if candidates>MAX_QUERY_CANDIDATES:
                    raise WorkLimit('edge spatial query candidate limit')
                if p in (a,b):
                    continue
                other = self.owners(p)
                if not other-owners:
                    continue
                z = self.xyz(p)
                u = sum((z[k]-x[k])*d[k] for k in range(3))/l2
                if 0<u<1 and sum((z[k]-x[k]-u*d[k])**2 for k in range(3)) <= 1e-14*l2:
                    contacts += 1
                    split.append((u,p,other))
                    if len(split)>MAX_CONTACTS_PER_EDGE:
                        raise WorkLimit('collinear contacts per edge limit')
                    union = owners|other
                    for owner in owners:
                        for t in chains[owner]:
                            if t['rendered'] and any(not inside(t,o) for o in other):
                                self.db.execute('INSERT OR IGNORE INTO requirements VALUES (?,?,?,?)',(int(t['id']),1,a,b))
                    for owner in other:
                        for t in chains[owner]:
                            if t['rendered'] and any(not inside(t,o) for o in owners):
                                self.db.execute('INSERT OR IGNORE INTO requirements VALUES (?,?,?,?)',(int(t['id']),0,p,-1))
                    pairs(union)
            if len(split)>2:
                # Preserve every original split-side edge in the span. Testing
                # only consecutive global contact points misses A-T when an
                # unrelated S lies between A and T but belongs to this same
                # ancestor: A-S/S-T need not be edges in that source partition.
                point_ids={p for _,p,_ in split}
                for p in point_ids:
                    for x,y,owner in self.db.execute('SELECT a,b,leaf FROM edges WHERE a=? OR b=?',(p,p)):
                        self.work.charge()
                        if x!=p or y not in point_ids or owner in owners:
                            continue
                        for t in chains[owner]:
                            if t['rendered'] and any(not inside(t,o) for o in owners):
                                self.db.execute('INSERT OR IGNORE INTO requirements VALUES (?,?,?,?)', (int(t['id']),1,x,y))
        segments = points = 0
        for t in tiles:
            if not t['rendered']:
                continue
            vertices = set()
            adjacency=collections.defaultdict(set)
            tol = [0.,0.,0.]
            for f in self.faces(int(t['id'])):
                ps = [tuple(p) for p in f['xyz']]
                vertices.update(ps)
                for k in range(3):
                    a,b=ps[k],ps[(k+1)%3]
                    adjacency[a].add(b);adjacency[b].add(a)
                ft = f.get('positionTolerance',[0.,0.,0.])
                tol = [max(tol[k],ft[k]) for k in range(3)]
            # Arithmetic slack only; quantization tolerance is decoder-derived.
            tol = [v+1e-10 for v in tol]
            index = PointIndex(vertices,self.work)
            cache = {}
            def matches(p):
                if p not in cache:
                    cache[p] = list(index.matches(self.xyz(p),tol))
                return cache[p]
            for kind,a,b in self.db.execute('SELECT kind,a,b FROM requirements WHERE tile=? ORDER BY kind DESC',(int(t['id']),)):
                if kind == 0:
                    points += 1
                    require(bool(matches(a)),'interfacePoints',t['id'])
                else:
                    segments += 1
                    targets={index.points[y] for y in matches(b)}
                    found=False
                    for x in matches(a):
                        neighbors=adjacency[index.points[x]]
                        self.work.charge(len(neighbors)+1)
                        if not neighbors.isdisjoint(targets):
                            found=True;break
                    require(found,'interfaceSegments',t['id'])
        depths = {int(t['id']):t['depth'] for t in tiles}
        mixed = same = 0
        for a,b in self.db.execute('SELECT a,b FROM pairs'):
            if depths[a] == depths[b]: same += 1
            else: mixed += 1
        return dict(checkedSegments=segments,checkedPoints=points,cornerContacts=corners,
                    tContacts=contacts,mixedPairs=mixed,sameDepthPairs=same,
                    candidateWork=self.work.used,workLimit=self.work.limit)
