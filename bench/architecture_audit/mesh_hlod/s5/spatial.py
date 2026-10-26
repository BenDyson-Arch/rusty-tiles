"""Independent bounded geometry indexes; SQLite owns archive-sized state.

No producer census, interface lock, sampled error or tile extras enter this index.
The R-trees are broad phases only: all geometric decisions use binary64 values.
"""
import collections
import itertools
import json
import math
from fractions import Fraction
import sqlite3
import struct

MAX_QUERY_CANDIDATES = 2_000_000


def packed_point(xyz):
    return struct.pack('<3d', *(0.0 if x == 0 else x for x in xyz))


def f32_point(xyz):
    """The published canonical domain, including positive zero."""
    return tuple(0.0 if x == 0 else x for x in struct.unpack('<3f', struct.pack('<3f', *xyz)))


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


def _cube_slack(p, radius):
    """Bound on the rounding of p[k] +/- radius and of stored f64 face boxes."""
    return 4*2.0**-52*(max(abs(x) for x in p)+radius)+1e-300


def box_distance(p, box):
    return sum(max(box[2*k]-p[k], 0, p[k]-box[2*k+1])**2 for k in range(3))


class Store:
    def __init__(self, path, work=None):
        self.db = sqlite3.connect(path)
        self.work = work or Work()
        self.source_bounds = {}
        self.leaf_boxes = {}
        self.captured = None
        self.db.executescript('''
          PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;
          PRAGMA temp_store=FILE; PRAGMA cache_size=-65536;
          CREATE TABLE faces(tile INTEGER, data TEXT);
          CREATE INDEX faces_tile ON faces(tile);
          CREATE TABLE source(id INTEGER PRIMARY KEY, leaf INTEGER, key TEXT UNIQUE, data TEXT);
          CREATE INDEX source_leaf ON source(leaf);
          CREATE VIRTUAL TABLE source_box USING rtree(id,x0,x1,y0,y1,z0,z1);
          CREATE TABLE points(id INTEGER PRIMARY KEY, xyz BLOB UNIQUE, rendered BLOB,
            tx REAL DEFAULT 0,ty REAL DEFAULT 0,tz REAL DEFAULT 0,
            x0 REAL,x1 REAL,y0 REAL,y1 REAL,z0 REAL,z1 REAL,conflict INTEGER DEFAULT 0);
          CREATE INDEX points_rendered ON points(rendered);
          CREATE TABLE observations(point INTEGER,leaf INTEGER,depth INTEGER,grid BLOB,
            unsupported INTEGER,xyz BLOB,PRIMARY KEY(point,leaf,grid,xyz)) WITHOUT ROWID;
          CREATE INDEX observations_depth ON observations(depth,grid);
          CREATE TABLE source_corners(source INTEGER PRIMARY KEY,a INTEGER,b INTEGER,c INTEGER);
          CREATE TABLE source_orientation(source INTEGER PRIMARY KEY,shape BLOB,sign INTEGER);
          CREATE TABLE canonical_points(id INTEGER PRIMARY KEY,key BLOB UNIQUE,xyz BLOB);
          CREATE INDEX canonical_xyz ON canonical_points(xyz);
          CREATE TABLE canonical_map(point INTEGER PRIMARY KEY,canonical INTEGER);
          CREATE TABLE canonical_owners(point INTEGER,leaf INTEGER,PRIMARY KEY(point,leaf)) WITHOUT ROWID;
          CREATE TABLE canonical_edges(a INTEGER,b INTEGER,leaf INTEGER,nf INTEGER,nr INTEGER,
            PRIMARY KEY(a,b,leaf)) WITHOUT ROWID;
          CREATE INDEX canonical_edges_b ON canonical_edges(b,a,leaf);
          CREATE TABLE incidence(a INTEGER,b INTEGER,faces INTEGER,leaf INTEGER,
            PRIMARY KEY(a,b)) WITHOUT ROWID;
          CREATE INDEX incidence_faces ON incidence(faces,a,b,leaf);
          CREATE TABLE exemptions(tile INTEGER,a INTEGER,b INTEGER,kind TEXT,
            PRIMARY KEY(tile,a,b,kind)) WITHOUT ROWID;
          CREATE TABLE source_shapes(shape BLOB,leaf INTEGER,sign INTEGER,
            PRIMARY KEY(shape,leaf,sign)) WITHOUT ROWID;
          CREATE TABLE requirements(tile INTEGER,kind INTEGER,a INTEGER,b INTEGER,
            PRIMARY KEY(tile,kind,a,b)) WITHOUT ROWID;
          CREATE TABLE pairs(a INTEGER,b INTEGER,PRIMARY KEY(a,b)) WITHOUT ROWID;
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
        return i

    def xyz(self, point):
        return struct.unpack('<3d', self.db.execute('SELECT xyz FROM points WHERE id=?', (point,)).fetchone()[0])

    def rendered_xyz(self, point):
        row=self.db.execute('SELECT rendered,xyz FROM points WHERE id=?',(point,)).fetchone()
        return struct.unpack('<3d',row[0] if row[0] is not None else row[1])

    def observe_point(self,point,xyz,tolerance):
        """Join by authoritative source XYZ, independent of original corner IDs.

        The same source point may cross primitive/UV seams or appear in leaves
        with different storage grids. Max incident intrinsic error is permitted
        by the shared canonical lattice; unrelated coarse leaves do not enter.
        """
        packed=struct.pack('<3d',*(0.0 if x==0 else x for x in xyz))
        row=self.db.execute('SELECT rendered FROM points WHERE id=?',(point,)).fetchone()
        if row[0] is None:
            self.db.execute('UPDATE points SET rendered=?,tx=?,ty=?,tz=?,x0=?,x1=?,y0=?,y1=?,z0=?,z1=? WHERE id=?',
                (packed,*tolerance,xyz[0],xyz[0],xyz[1],xyz[1],xyz[2],xyz[2],point))
        else:
            self.db.execute('UPDATE points SET tx=max(tx,?),ty=max(ty,?),tz=max(tz,?),'
                'x0=min(x0,?),x1=max(x1,?),y0=min(y0,?),y1=max(y1,?),z0=min(z0,?),z1=max(z1,?),'
                'conflict=max(conflict,?) WHERE id=?',
                (*tolerance,xyz[0],xyz[0],xyz[1],xyz[1],xyz[2],xyz[2],int(row[0]!=packed),point))

    def validate_leaf_points(self,require):
        checked=0
        for row in self.db.execute('SELECT id,xyz,tx,ty,tz,x0,x1,y0,y1,z0,z1,conflict FROM points WHERE rendered IS NOT NULL'):
            self.work.charge()
            point,packed,*values=row
            expected=struct.unpack('<3d',packed)
            tol=values[:3];ranges=values[3:9];conflict=values[9]
            require(all(expected[k]-tol[k]-1e-10<=ranges[2*k] and ranges[2*k+1]<=expected[k]+tol[k]+1e-10 for k in range(3)),
                    'leafGeometry',f'source point {point} {expected}: rendered range {ranges}, incident tolerance {tol}')
            require(not conflict,'interfacePoints',f'leaf decoded disagreement at source point {point} {expected}')
            checked+=1
        return checked

    def validate_leaf_agreement(self,require):
        """A source-point contradiction is provable without any grid inference."""
        for point,xyz,conflict in self.db.execute('SELECT id,xyz,conflict FROM points'):
            self.work.charge()
            require(not conflict,'interfacePoints',
                    f'leaf decoded disagreement at source point {point} {struct.unpack("<3d",xyz)}')

    def source_face(self, key, want, leaf, actual=None, depth=0):
        self.work.charge()
        try:
            i = self.db.execute('INSERT INTO source(leaf,key,data) VALUES (?,?,?)',
                (leaf,key,json.dumps(want,separators=(',',':')))).lastrowid
        except sqlite3.IntegrityError:
            return False
        self.db.execute('INSERT INTO source_box VALUES (?,?,?,?,?,?,?)', (i,*bounds(want['xyz'])))
        # Captured f32 positions bound the root cell; E_d derives from its extent.
        box=bounds([f32_point(p) for p in want['xyz']])
        self.captured=box if self.captured is None else tuple(
            min(a,b) if k%2==0 else max(a,b) for k,(a,b) in enumerate(zip(self.captured,box)))
        ids = [self.point(p) for p in want['xyz']]
        self.db.execute('INSERT INTO source_corners VALUES (?,?,?,?)',(i,*ids))
        if actual is not None:
            tolerance=actual.get('positionTolerance',[0.,0.,0.])
            step=actual.get('positionStep')
            grid=struct.pack('<3d',*step) if step is not None else b''
            for p,xyz in zip(ids,actual['xyz']):
                self.observe_point(p,xyz,tolerance)
                self.db.execute('INSERT OR IGNORE INTO observations VALUES (?,?,?,?,?,?)',
                    (p,leaf,depth,grid,int(actual.get('positionGridUnsupported',False)),packed_point(xyz)))
        from winding import shape
        key,sign=shape(actual['xyz'] if actual is not None else want['xyz'])
        if key is not None:
            self.db.execute('INSERT INTO source_orientation VALUES (?,?,?)',(i,key,sign))
        return True

    def finalize_canonical(self, require):
        """Derive source welds before looking at delivered coordinate aliases.

        A codec error bound is not a lattice. Only the independent integer
        accessor's axis-aligned grid is eligible for zero-phase snapping. An
        unchanged float source point needs no inferred grid. Changed float
        fallback points require a uniquely observed grid at their minimum
        incident depth; absence/ambiguity fails closed.
        """
        self.validate_leaf_agreement(require)
        stats=collections.Counter()
        depth_grids={}
        for depth,grid in self.db.execute('SELECT depth,grid FROM observations ORDER BY depth,grid'):
            self.work.charge()
            if grid:
                known=depth_grids.setdefault(depth,set())
                known.add(grid)
                if len(known)>4096:
                    raise WorkLimit('distinct source grids per depth exceeds 4096')
        for point,source,rendered in self.db.execute('SELECT id,xyz,rendered FROM points ORDER BY id'):
            self.work.charge()
            p=struct.unpack('<3d',source)
            actual=struct.unpack('<3d',rendered) if rendered is not None else p
            depth=self.db.execute('SELECT min(depth) FROM observations WHERE point=?',(point,)).fetchone()[0]
            grids=set()
            unsupported=False
            for grid,bad in self.db.execute('SELECT grid,unsupported FROM observations WHERE point=? AND depth=?',(point,depth)):
                self.work.charge()
                if grid:grids.add(grid)
                unsupported |= bool(bad)
            if not grids and not unsupported:
                grids=depth_grids.get(depth,set())
            if len(grids)==1 and not unsupported:
                step=struct.unpack('<3d',next(iter(grids)))
                require(all(math.isfinite(s) and s>0 for s in step),'oracleDomain','sourceCanonical: invalid independently decoded grid')
                canonical=f32_point([round(p[k]/step[k])*step[k] for k in range(3)])
                stats['gridSourcePoints']+=1
                for (observation,) in self.db.execute('SELECT xyz FROM observations WHERE point=?',(point,)):
                    self.work.charge()
                    delivered=struct.unpack('<3d',observation)
                    require(f32_point(delivered)==canonical,'leafGeometry',f'sourceCanonical point {point}: zero-phase canonical {canonical}, delivered {delivered}, step {step}')
            else:
                for (observation,) in self.db.execute('SELECT xyz FROM observations WHERE point=?',(point,)):
                    self.work.charge()
                    require(observation==source,'oracleDomain',f'sourceCanonical point {point}: changed source position without a unique supported minimum-depth grid')
                canonical=f32_point(p)
                stats['capturedSourcePoints']+=1
            key=struct.pack('<3f',*canonical)
            row=self.db.execute('SELECT id,xyz FROM canonical_points WHERE key=?',(key,)).fetchone()
            if row is None:
                identity=self.db.execute('INSERT INTO canonical_points(key,xyz) VALUES (?,?)',(key,packed_point(actual))).lastrowid
            else:
                identity,prior=row
                # Formula-confirmed source collisions are welds. Raw delivered
                # aliases may not erase a real displacement after that weld.
                require(prior==packed_point(actual),'interfacePoints',f'canonical source key {canonical}: raw delivered disagreement')
                stats['canonicalSourceCollisions']+=1
            self.db.execute('INSERT INTO canonical_map VALUES (?,?)',(point,identity))
        # Triangles with fewer than three keys contribute neither points nor
        # edges, but remain in source identity/fidelity and orientation checks.
        for leaf,a,b,c in self.db.execute('SELECT s.leaf,x.canonical,y.canonical,z.canonical FROM source_corners c '
                'JOIN source s ON s.id=c.source JOIN canonical_map x ON x.point=c.a '
                'JOIN canonical_map y ON y.point=c.b JOIN canonical_map z ON z.point=c.c'):
            self.work.charge()
            ids=(a,b,c)
            if len(set(ids))<3:
                stats['collapsedSourceTriangles']+=1
                continue
            self.db.executemany('INSERT OR IGNORE INTO canonical_owners VALUES (?,?)',((p,leaf) for p in ids))
            self.db.executemany('INSERT INTO canonical_edges VALUES (?,?,?,?,?) ON CONFLICT(a,b,leaf) '
                'DO UPDATE SET nf=nf+excluded.nf,nr=nr+excluded.nr',
                ((*sorted((ids[k],ids[(k+1)%3])),leaf,int(ids[k]<ids[(k+1)%3]),int(ids[k]>ids[(k+1)%3])) for k in range(3)))
        self.db.execute('INSERT INTO incidence SELECT a,b,sum(nf+nr),min(leaf) FROM canonical_edges GROUP BY a,b')
        for shape,leaf,sign in self.db.execute('SELECT o.shape,s.leaf,o.sign FROM source_orientation o '
                'JOIN source s ON s.id=o.source JOIN source_corners c ON c.source=o.source '
                'JOIN canonical_map x ON x.point=c.a JOIN canonical_map y ON y.point=c.b '
                'JOIN canonical_map z ON z.point=c.c WHERE x.canonical<>y.canonical '
                'AND x.canonical<>z.canonical AND y.canonical<>z.canonical'):
            self.work.charge()
            self.db.execute('INSERT OR IGNORE INTO source_shapes VALUES (?,?,?)',(shape,leaf,sign))
        self.canonical_stats=dict(stats)
        self.db.commit()

    def extent(self):
        """Maximum axis extent of the captured source (the root cell's E_0 basis)."""
        box=self.captured
        return max(box[2*k+1]-box[2*k] for k in range(3)) if box else 0.0

    def canonical_xyz(self, point):
        return struct.unpack('<3d',self.db.execute('SELECT xyz FROM canonical_points WHERE id=?',(point,)).fetchone()[0])

    def leaf_bounds(self, leaf):
        row = self.db.execute('SELECT min(x0),max(x1),min(y0),max(y1),min(z0),max(z1) '
                             'FROM source_box JOIN source USING(id) WHERE leaf=?', (leaf,)).fetchone()
        if row[0] is not None:
            self.leaf_boxes[leaf]=row

    def source_box(self, lo, hi):
        """Axis box of the verified source faces in a descendant leaf interval."""
        if (lo,hi) not in self.source_bounds:
            self.work.charge(hi-lo)
            boxes=[self.leaf_boxes[i] for i in range(lo,hi) if i in self.leaf_boxes]
            self.source_bounds[lo,hi]=(tuple(v for k in range(3) for v in
                (min(b[2*k] for b in boxes),max(b[2*k+1] for b in boxes))) if boxes else (None,)*6)
        return self.source_bounds[lo,hi]

    def within(self, p, lo, hi, distance2, radius, accept):
        """Nearest accepted descendant source face no farther than radius, or None.

        The cube is widened by the float rounding of its bounds, so every face
        within radius is a candidate; the caller decides on exact distances."""
        reach = radius + _cube_slack(p, radius)
        params = tuple(v for k in range(3) for v in (p[k]+reach,p[k]-reach))
        rows = self.db.execute('SELECT source.id,leaf,key,data FROM source_box JOIN source USING(id) '
            'WHERE x0<=? AND x1>=? AND y0<=? AND y1>=? AND z0<=? AND z1>=?', params)
        best, answer = math.inf, None
        for count,(i,leaf,key,data) in enumerate(rows):
            self.work.charge()
            if count>MAX_QUERY_CANDIDATES:
                raise WorkLimit('tie source query candidate limit')
            if not lo<=leaf<hi:
                continue
            want=json.loads(data);identity=tuple(map(int,key.split(',')))
            if not accept(identity,want):
                continue
            d=distance2(p,want['xyz'])
            if d<=Fraction(reach)**2 and d<best:
                best,answer=d,(identity,want)
        return answer

    def nearest(self, p, lo, hi, distance2):
        # Expanding cubes are complete once the best triangle lies closer than
        # the next cube boundary. Descendant filtering uses DFS leaf intervals.
        box = self.source_box(lo, hi)
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
            # Exact: complete once the best lies inside the cube shrunk by the
            # rounding of its float bounds.
            if answer is not None and best[0] <= Fraction(max(radius-_cube_slack(p,radius),0.0))**2:
                return answer
            radius *= 2
            if not math.isfinite(radius):
                raise ValueError('unbounded nearest source query')

    def census(self, tiles, leaves, require):
        if not hasattr(self,'canonical_stats'):
            self.finalize_canonical(require)
        chains = {}
        for leaf,t in enumerate(leaves):
            chain = []
            while t is not None:
                chain.append(t)
                t = t['parent']
            chains[leaf] = chain
        def inside(t, owner):
            return t['lo'] <= owner < t['hi']
        def owners(point):
            answer=set()
            for (owner,) in self.db.execute('SELECT leaf FROM canonical_owners WHERE point=?',(point,)):
                self.work.charge()
                answer.add(owner)
            return answer
        def demand(owners, kind, a, b=-1):
            low,high=min(owners),max(owners)
            for owner in owners:
                for t in chains[owner]:
                    self.work.charge()
                    if t['rendered'] and (low<t['lo'] or high>=t['hi']):
                        self.db.execute('INSERT OR IGNORE INTO requirements VALUES (?,?,?,?)', (int(t['id']),kind,a,b))
        def pairs(owners):
            for a,b in itertools.combinations(sorted(owners),2):
                for x,y in itertools.product(chains[a],chains[b]):
                    self.work.charge()
                    if x['hi']<=y['lo'] or y['hi']<=x['lo']:
                        self.db.execute('INSERT OR IGNORE INTO pairs VALUES (?,?)', tuple(sorted((int(x['id']),int(y['id'])))))
        shared_points=shared_segments=open_edges=open_shared=0
        for (p,) in self.db.execute('SELECT point FROM canonical_owners GROUP BY point HAVING count(*)>1'):
            own=owners(p)
            shared_points+=1
            demand(own,0,p)
            pairs(own)
        for a,b,incidence,leaf in self.db.execute('SELECT a,b,faces,leaf FROM incidence'):
            self.work.charge()
            own=set()
            for (o,) in self.db.execute('SELECT leaf FROM canonical_edges WHERE a=? AND b=?',(a,b)):
                self.work.charge();own.add(o)
            if len(own)>1:
                shared_segments+=1
                demand(own,1,a,b)
                pairs(own)
            if incidence==1:
                open_edges+=1
                left,right=owners(a),owners(b)
                if left-{leaf} and right-{leaf}:
                    open_shared+=1
                    low_a,high_a=min(left),max(left)
                    low_b,high_b=min(right),max(right)
                    # These are diagnostics, never chord/interface demands.
                    for t in chains[leaf]:
                        self.work.charge()
                        if t['rendered'] and (low_a<t['lo'] or high_a>=t['hi']) and (low_b<t['lo'] or high_b>=t['hi']):
                            self.db.execute('INSERT OR IGNORE INTO requirements VALUES (?,?,?,?)',(int(t['id']),2,a,b))
        self.db.execute('CREATE TABLE rendered_borders(tile INTEGER,a BLOB,b BLOB,PRIMARY KEY(tile,a,b)) WITHOUT ROWID')
        import borders
        borders.prepare(self)
        d11=collections.Counter()
        segments=points=exempt=max_triangles=0
        for t in tiles:
            if not t['rendered']:
                continue
            vertices=set();adjacency=collections.defaultdict(set)
            edge_counts=collections.Counter();directed=collections.Counter();triangles=0;surface=[]
            for f in self.faces(int(t['id'])):
                self.work.charge()
                triangles+=1
                if triangles>1_000_000:
                    raise WorkLimit('rendered tile exceeds 1,000,000 faces')
                ps=[tuple(p) for p in f['xyz']]
                if len(set(f32_point(p) for p in ps))<3:
                    continue
                vertices.update(ps)
                surface.append(ps)
                for k in range(3):
                    a,b=ps[k],ps[(k+1)%3]
                    adjacency[a].add(b);adjacency[b].add(a)
                    edge_counts[tuple(sorted((a,b)))]+=1
                    directed[a,b]+=1
            if t['children']:
                max_triangles=max(max_triangles,triangles)
            self.db.executemany('INSERT INTO rendered_borders VALUES (?,?,?)',
                ((int(t['id']),packed_point(a),packed_point(b)) for (a,b),n in edge_counts.items() if n==1))
            cache={}
            def canonical(p):
                if p not in cache:cache[p]=self.canonical_xyz(p)
                return cache[p]
            if t['children']:
                # D11: coarse borders are arcs between retained exact vertices.
                arcs=borders.tile_border(self,t,vertices,directed,require,d11)
                borders.folds(self,t,directed,arcs,require,surface)
            for kind,a,b in self.db.execute('SELECT kind,a,b FROM requirements WHERE tile=? ORDER BY kind DESC',(int(t['id']),)):
                self.work.charge()
                if t['children'] and kind<2:
                    points+=kind==0;segments+=kind==1
                elif kind==0:
                    points+=1
                    require(canonical(a) in vertices,'interfacePoints',t['id'])
                elif kind==1:
                    segments+=1
                    require(canonical(b) in adjacency.get(canonical(a),()),'interfaceSegments',t['id'])
                elif canonical(b) not in adjacency.get(canonical(a),()):
                    exempt+=1
                    self.db.execute('INSERT OR IGNORE INTO exemptions VALUES (?,?,?,?)',(int(t['id']),a,b,'open-edge-change'))
        depths={int(t['id']):t['depth'] for t in tiles}
        mixed=same=0
        for a,b in self.db.execute('SELECT a,b FROM pairs'):
            self.work.charge()
            if depths[a]==depths[b]:same+=1
            else:mixed+=1
        return dict(checkedSegments=segments,checkedPoints=points,
                    sharedPoints=shared_points,sharedSegments=shared_segments,
                    sharedSourcePoints=shared_points,sharedSourceSegments=shared_segments,
                    cornerContacts=shared_points,tContacts=0,
                    sourceOpenEdges=open_edges,sourceOpenEdgesWithSharedEndpoints=open_shared,
                    exemptOpenEdgeChanges=exempt,exemptCrackEvents=0,
                    mixedPairs=mixed,sameDepthPairs=same,
                    maxCoarseTriangles=max_triangles,
                    contractedBorderArcs=d11['contractedBorderArcs'],
                    contractedBorderVertices=d11['contractedBorderVertices'],
                    borderArcs=d11['borderArcs'],
                    borderArcCpuSeconds=d11['arcCpuSeconds'],
                    coarseTriangleCountBasis='expanded tile instances, summed across each tile contents; every payload is separately within the hard 8,192 (triangleCap)',
                    canonical=self.canonical_stats,candidateWork=self.work.used,workLimit=self.work.limit)
