#!/usr/bin/env python3
"""Published-byte/source oracle for directories and .3tz, including implicit forests.

Archive-sized state is disk backed. No producer facts or interface locks are read.
This checks topology/identity/attributes, not a source-distance certificate.
Generate independent source expectations with source_manifest.py.
"""
import argparse
import collections
import hashlib
import itertools
import json
import math
from fractions import Fraction
from pathlib import Path
import subprocess
import sqlite3
import tempfile
import zipfile
from archive import Archive, expand
from continuity import check_tiles
from spatial import Store, Work, WorkLimit
from seam_diagnostic import WorkLimit as SeamWorkLimit

ROOT = Path(__file__).parent

class Failure(Exception):
    def __init__(self, check, detail):
        self.check, self.detail = check, detail
        super().__init__(f'{check}: {detail}')

def require(ok, check, detail):
    if not ok:
        raise Failure(check, detail)

def decode(path):
    binary = isinstance(path,(bytes,bytearray))
    r = subprocess.run(['node', '--max-old-space-size=2048', str(ROOT / 'decode.mjs'), '-' if binary else str(path)],
                       input=bytes(path) if binary else None,capture_output=True,timeout=180)
    stderr=r.stderr.decode(errors='replace')
    require(r.returncode == 0, 'codecStructure' if 'featureIdentity' not in stderr and 'metadataIntegrity' not in stderr else 'featureIdentity', stderr[-1500:])
    decoded = json.loads(r.stdout)
    # UINT64 columns cross JSON as decimal strings; Python integers retain every bit.
    for face in decoded['faces']:
        for row in face['ids'].values():
            for key, value in row.items():
                if isinstance(value, str):
                    row[key] = int(value)
    return decoded

def distance2(p, face):
    """Exact squared distance from p to the closed triangle, as a Fraction.

    Binary float inputs are exact dyadic rationals: put all twelve coordinates
    on one power-of-two scale as integers. Region tests are signs of integer
    numerators (no division, no rounding), so a skinny f32 triangle whose float
    Gram determinant would round to zero keeps its interior (review r12).
    """
    ratios=[float(v).as_integer_ratio() for q in (p,*face) for v in q]
    shift=max(d.bit_length()-1 for _,d in ratios)
    n=[r<<(shift-d.bit_length()+1) for r,d in ratios]
    P,A,B,C=n[0:3],n[3:6],n[6:9],n[9:12]
    sub=lambda x,y:[x[0]-y[0],x[1]-y[1],x[2]-y[2]]
    dot=lambda x,y:x[0]*y[0]+x[1]*y[1]+x[2]*y[2]
    best=None  # (numerator, denominator) at scale 2**(2*shift)
    u,v,w=sub(B,A),sub(C,A),sub(P,A)
    uu,uv,vv,wu,wv=dot(u,u),dot(u,v),dot(v,v),dot(w,u),dot(w,v)
    det=uu*vv-uv*uv
    if det>0:
        x,y=wu*vv-wv*uv,wv*uu-wu*uv  # barycentric numerators over det
        if x>=0 and y>=0 and x+y<=det:
            # |w - (x u + y v)/det|^2 = |w|^2 - (x wu + y wv)/det at the projection.
            best=(dot(w,w)*det-(x*wu+y*wv),det)
    for s0,s1 in ((A,B),(B,C),(C,A)):
        d,w=sub(s1,s0),sub(P,s0)
        dd,wd=dot(d,d),dot(w,d)
        if dd==0 or wd<=0:
            candidate=(dot(w,w),1)
        elif wd>=dd:
            e=sub(w,d);candidate=(dot(e,e),1)
        else:
            candidate=(dot(w,w)*dd-wd*wd,dd)
        if best is None or candidate[0]*best[1]<best[0]*candidate[1]:
            best=candidate
    return Fraction(best[0],best[1]<<(2*shift))

def material_class(m):
    m=m or {}
    mode=m.get('alphaMode','OPAQUE')
    return (mode,m.get('alphaCutoff',.5) if mode=='MASK' else None,
            m.get('doubleSided',False),'KHR_materials_unlit' in m.get('extensions',{}))


def material_semantics(material):
    """PBR constants and texture-slot presence, independent of resource indices.

    Texture pixels/crop transforms are a separate source-correspondence proof;
    source GLB material indices are local to that GLB and cannot be compared to
    the copied/cropped leaf's indices.
    """
    m=material or {};p=m.get('pbrMetallicRoughness',{})
    require(set(m.get('extensions',{}))<={'KHR_materials_unlit'},'oracleDomain','unsupported source material extension')
    slots=(p.get('baseColorTexture'),p.get('metallicRoughnessTexture'),m.get('normalTexture'),m.get('occlusionTexture'),m.get('emissiveTexture'))
    return (material_class(m),p.get('baseColorFactor',[1,1,1,1]),p.get('metallicFactor',1),
            p.get('roughnessFactor',1),m.get('emissiveFactor',[0,0,0]),
            (m.get('normalTexture') or {}).get('scale',1),
            (m.get('occlusionTexture') or {}).get('strength',1),tuple(v is not None for v in slots))


def textured(material):
    m=material or {};p=m.get('pbrMetallicRoughness',{})
    return any(key in owner for owner,keys in ((p,('baseColorTexture','metallicRoughnessTexture')),
        (m,('normalTexture','occlusionTexture','emissiveTexture'))) for key in keys)


IDENTITY = [1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]
KEYS = ('node_index','mesh_index','primitive_index','triangle_index')
# D4 ties (owner, 2026-10-26): this oracle's distances are exact, so tau covers
# only the producer's f64 arithmetic. It is a stated acceptance policy, not a
# universal forward-error bound: it is sized for a well-conditioned evaluation
# near the source (<= 8 roundings of magnitude M = |centroid| + descendant source
# diagonal, doubled), and does not bound cancellation, ill-conditioned
# projections or distances to a remote compact box (r13). A producer pick that
# misses by more than tau is refused as a producer defect, never widened.
TIE_ULPS = 16


def tie_excess(named, minimum):
    """sqrt(named) - sqrt(minimum) from exact squared distances, for reports.

    The difference of squares is exact; dividing by the sum of the roots avoids the
    cancellation that rounds a sub-ulp positive excess to zero (review r13). It is
    zero only for an exact tie."""
    difference=named-minimum
    if difference<=0:
        return 0.0
    roots=Fraction(math.sqrt(named))+Fraction(math.sqrt(minimum))
    # Underflowed roots: still report a positive, nonzero excess.
    return float(difference/roots) or math.ulp(0.0) if roots else math.ulp(0.0)


def within_tau(named, minimum, tau):
    """Exact sqrt(named) - sqrt(minimum) <= tau for squared distances."""
    tau=Fraction(tau)
    gap=named-minimum-tau*tau
    return gap<=0 or gap*gap<=4*tau*tau*minimum


def multiply(a,b):
    return [sum(a[i%4+k*4]*b[k+(i//4)*4] for k in range(4)) for i in range(16)]


def inverse(m):
    rows = [[float(m[r+c*4]) for c in range(4)]+[float(r==c) for c in range(4)] for r in range(4)]
    for c in range(4):
        pivot=max(range(c,4),key=lambda r:abs(rows[r][c]))
        require(abs(rows[pivot][c])>1e-30,'oracleDomain','singular tile transform')
        rows[c],rows[pivot]=rows[pivot],rows[c]
        scale=rows[c][c];rows[c]=[x/scale for x in rows[c]]
        for r in range(4):
            if r!=c:
                scale=rows[r][c];rows[r]=[x-scale*y for x,y in zip(rows[r],rows[c])]
    return [rows[r][c+4] for c in range(4) for r in range(4)]


def transform_face(f,m):
    if m == IDENTITY:
        return
    f['xyz']=[[sum(m[a+k*4]*p[k] for k in range(3))+m[a+12] for a in range(3)] for p in f['xyz']]
    tol=f.get('positionTolerance',[0,0,0])
    f['positionTolerance']=[sum(abs(m[a+k*4])*tol[k] for k in range(3))+1e-12 for a in range(3)]
    if f.get('positionStep') is not None:
        linear=[[m[a+k*4] for k in range(3)] for a in range(3)]
        axis_aligned=all(sum(v!=0 for v in row)==1 for row in linear) and all(sum(linear[a][k]!=0 for a in range(3))==1 for k in range(3))
        if axis_aligned:
            f['positionStep']=[sum(abs(linear[a][k])*f['positionStep'][k] for k in range(3)) for a in range(3)]
        else:
            f['positionStep']=None
            f['positionGridUnsupported']=True
    if f.get('normals') is not None:
        inv=inverse(m)
        ns=[[sum(inv[k+a*4]*n[k] for k in range(3)) for a in range(3)] for n in f['normals']]
        frobenius=math.sqrt(sum(inv[k+a*4]**2 for a in range(3) for k in range(3)))
        if f.get('normalTolerance') is not None:
            conditioning=max(2*frobenius/math.sqrt(sum(v*v for v in n)) for n in ns)
            f['normalTolerance']=min(2,conditioning*f['normalTolerance']+1e-12)
        f['normals']=[[x/math.sqrt(sum(v*v for v in n)) for x in n] for n in ns]


def area_vector(face):
    a,b,c=face
    u=[b[k]-a[k] for k in range(3)];v=[c[k]-a[k] for k in range(3)]
    return [u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]]


def structural_face(f):
    for name,width in (('xyz',3),('normals',3),('uvs',2)):
        values=f.get(name)
        if values is None and name!='xyz':
            continue
        require(isinstance(values,list) and len(values)==3 and
                all(isinstance(v,list) and len(v)==width and
                    all(type(x) in (int,float) and math.isfinite(x) for x in v) for v in values),
                'codecStructure',f'{name} rendered triangle shape/values')
    if f.get('normals') is not None:
        require(all(abs(sum(x*x for x in n)-1)<2e-3 for n in f['normals']),
                'codecStructure','rendered NORMAL must be unit length')
    tol=f.get('positionTolerance',[0,0,0])
    require(isinstance(tol,list) and len(tol)==3 and all(type(x) in (int,float) and math.isfinite(x) and x>=0 for x in tol),
            'codecStructure','invalid decoded position tolerance')


def check_leaf(f,manifest,stats=None,*,geometry=True):
    row=f['ids'].get('source_triangle',{})
    key=tuple(row.get(k) for k in KEYS)
    want=manifest['triangles'].get(','.join(map(str,key)))
    require(want is not None,'sourceIdentity',str(key))
    primitive=f['ids'].get('source_primitive',{})
    require(all(primitive.get(k)==row.get(k) for k in KEYS[:3]+('expanded_ordinal','material_index')),'sourceIdentity',str(key))
    require(all(row.get(k)==want[k] for k in ('expanded_ordinal','material_index')),'sourceIdentity',str(key))
    require([row.get(f'vertex_{k}') for k in range(3)]==want['vertices'],'sourceIdentity',str(key))
    tolerance=f.get('positionTolerance',[0,0,0])
    if geometry:
        require(all(abs(f['xyz'][i][k]-want['xyz'][i][k])<=tolerance[k]+1e-10 for i in range(3) for k in range(3)),
                'leafGeometry',str(key))
    source_glb=manifest.get('metadata',{}).get('sourceKind')=='sourceGLB'
    if source_glb:
        require(material_semantics(f['material'])==material_semantics(want['material']),'leafMaterial',str(key))
    else:
        require(f['material']==want['material'],'leafMaterial',str(key))
    texcoord_sets=want.get('texcoord_sets',{})
    if isinstance(want.get('texcoords'),dict):texcoord_sets=want['texcoords']
    require(not any(str(k)!='0' for k in texcoord_sets),'oracleDomain','source TEXCOORD sets beyond 0 are unsupported')
    for actual,expected,check in (('uvs',want.get('uvs',want.get('texcoords')),'sourceTexcoords'),
                                  ('normals',want.get('normals'),'sourceNormals')):
        if actual=='uvs' and isinstance(expected,dict):
            expected=expected.get('0',expected.get(0))
        if expected is not None:
            values=f.get(actual)
            if actual=='uvs' and source_glb and textured(want['material']):
                # Cropped leaf coordinates need not equal authored UVs. Without
                # independently decoded texture pixels, their physical mapping
                # is explicitly uninspected, never accepted by a wide epsilon.
                require(values is not None,'sourceTexcoords',str(key)+' missing rendered TEXCOORD_0')
                if stats is not None:stats['leafTexcoordCorrespondenceUninspected']+=1
                continue
            tolerance=f.get('uvTolerance' if actual=='uvs' else 'normalTolerance',2e-4)
            tolerances=tolerance if isinstance(tolerance,list) else [tolerance]*(2 if actual=='uvs' else 3)
            require(values is not None and len(values)==len(expected) and
                all(len(a)==len(b) and all(abs(x-y)<=tolerances[k]+1e-10 for k,(x,y) in enumerate(zip(a,b))) for a,b in zip(values,expected)),check,str(key))
            if stats is not None:stats['leafTexcoordsChecked' if actual=='uvs' else 'leafNormalsChecked']+=1
    return ','.join(map(str,key)),want


def audit(root,expected,*,scratch=None,work_limit=300_000_000,coordinate_frame='root',profile='hlod',producer_manifest=None,
          border_audit=None,producer_report=None,triangle_cap=8192):
    """Check every published cell. Only one payload/tile is decoded at a time.

    root frame removes the archive's common placement, conjugating relative tile
    transforms by standard glTF Y-up -> 3D Tiles Z-up. Source expectations stay
    in authored glTF coordinates. world frame retains placement and applies that
    conversion, so its source manifest must independently use the same frame.
    HLOD is the only supported acceptance profile; profile selection cannot
    weaken identity or representative checks.
    """
    require(profile=='hlod','oracleDomain','unsupported acceptance profile: '+str(profile))
    from source_manifest import load_manifest
    with load_manifest(expected) as manifest, Archive(root) as archive, tempfile.TemporaryDirectory(prefix='s5-oracle-',dir=scratch) as temporary:
        metadata=manifest.get('metadata',{})
        require(coordinate_frame=='root' and metadata.get('coordinateFrame','root')=='root'
                and metadata.get('transform16',IDENTITY)==IDENTITY,
                'oracleDomain','sourceCanonical: exact D1 source topology requires root-frame capture coordinates without post-capture transform16; world placement after f32 capture is not a canonical-key frame')
        try:
            tiles=expand(archive)
        except ValueError as e:
            if any(s in str(e) for s in ('geometricError','nonfinite JSON')):
                raise Failure('geometricErrorFinite',str(e)) from e
            raise Failure('oracleDomain',str(e)) from e
        require(bool(tiles),'leafCoverage','empty hierarchy')
        barrier=check_tiles(tiles)
        require(barrier['ok'],'replaceContentContinuity',str(barrier['violations']))
        doc=archive.json(archive.root_name)
        g=doc.get('geometricError')
        require(type(g) in (int,float) and math.isfinite(g) and g>=0,'geometricErrorFinite','tileset G')
        for name in {name for t in tiles for name in t.get('external_uris',[])}:
            external=archive.json(name)
            g=external.get('geometricError')
            require(type(g) in (int,float) and math.isfinite(g) and g>=0,'geometricErrorFinite',name)
            require(external['root']['geometricError']<=g+1e-8,'geometricErrorOrder',name)
        leaves=[]
        def intervals(t):
            t['lo']=len(leaves)
            if not t['children']:
                leaves.append(t)
            else:
                for c in t['children']: intervals(c)
            t['hi']=len(leaves)
        intervals(tiles[0])
        for t in tiles:
            t['rendered']=False
            g=t['g'];parent=t['parent']
            # Compose below the placement boundary directly. Inverting a large
            # ECEF transform and multiplying it back loses small translations.
            t['relative']=multiply(parent['relative'],t['node'].get('transform',IDENTITY)) if parent else IDENTITY
            require(type(g) in (int,float) and math.isfinite(g) and g>=0,'geometricErrorFinite',str(g))
            require(g <= (parent['g'] if parent else doc['geometricError'])+1e-8,'geometricErrorOrder',str(g))
        yup=[1,0,0,0,0,0,1,0,0,-1,0,0,0,0,0,1]
        # An identity placement is exact; avoid inverse/multiply cancellation in
        # the overwhelmingly common root-only placement case.
        work=Work(work_limit)
        store=Store(Path(temporary)/'census.sqlite',work)
        leaf_count=representatives=0
        attributes=collections.Counter()
        try:
            for t in tiles:
                if coordinate_frame=='root':
                    relative=t['relative']
                    matrix=IDENTITY if relative==IDENTITY else multiply(inverse(yup),multiply(relative,yup))
                else:
                    matrix=multiply(t['transform'],yup)
                for uri in t['contents']:
                    data=archive.read(uri)
                    if t['children'] and border_audit is not None:
                        t.setdefault('sha256',[]).append(hashlib.sha256(data).hexdigest())
                    decoded=decode(data)
                    faces=decoded['faces']
                    require(len(faces)<=1_000_000,'oracleWorkLimit','one payload exceeds 1,000,000 faces')
                    # D1's hard cap, per payload, from this decoder's own faces.
                    require(len(faces)<=8192,'triangleCap',f'{uri} publishes {len(faces)} triangles, above the hard 8192')
                    for f in faces:
                        structural_face(f)
                        transform_face(f,matrix)
                        ids=f['ids']
                        # Every HLOD payload participates, even if its identity
                        # declarations were completely removed by a mutant.
                        require('source_primitive' in ids,'featureIdentity',uri)
                        require(not t['children'] or 'source_triangle' not in ids,'featureIdentity','coarse source_triangle '+uri)
                        t['rendered']=True
                        if not t['children']:
                            key,want=check_leaf(f,manifest,attributes,geometry=False)
                            require(store.source_face(key,want,t['lo'],actual=f,depth=t['depth']),'leafCoverage','duplicate '+key)
                            leaf_count+=1
                    store.put_faces(int(t['id']),faces)
                if not t['children']:
                    # Empty terminal availability cells contain no rendered
                    # source. Exact global multiplicity below decides coverage.
                    store.leaf_bounds(t['lo'])
                store.db.commit()
            require(leaf_count==len(manifest['triangles']),'leafCoverage',f'{leaf_count} != {len(manifest["triangles"])}')
            # A leaf below a collapsed single-child chain is quantized on its frozen
            # depth's grid, not its delivered depth's: group its observations there.
            chain_leaves=0
            if border_audit is not None:
                import relief
                frozen=relief.leaf_depths(tiles,border_audit)
                if frozen:
                    deeper={t['lo']:frozen[t['lo']] for t in leaves if frozen[t['lo']]!=t['depth']}
                    store.relabel(deeper)
                    chain_leaves=len(deeper)
            store.finalize_canonical(require)
            source_points=store.validate_leaf_points(require)
            result=store.census(tiles,leaves,require)
            result['chainLeaves']=chain_leaves
            from winding import inspect as inspect_winding
            result['winding']=inspect_winding(store,tiles,require)
            from seam_diagnostic import inspect_open_edges
            result['seamDiagnostics']=inspect_open_edges(store,tiles,leaves)
            result['exemptCrackEvents']=store.db.execute("SELECT count(*) FROM exemptions WHERE kind='crack'").fetchone()[0]
            result['exemptCrackEventBasis']='deduplicated (tile,source-open-edge) positive sampled-width witnesses only; changed open edges are candidates, and unmeasured correspondences are not zero-gap proofs'
            result['exemptWindingEvents']=result['winding']['exemptWindingEvents']
            # D11: the S1b audit sidecar is an index; every claim is re-derived.
            import borders
            result['borders']=borders.inspect(store,tiles,leaves,archive,require,border_audit,
                                              extent=store.extent(),census=result,cap=triangle_cap)
            if producer_report is not None:
                require(result['borders'].get('audited'),'borderAudit','a producer report needs the audit sidecar')
                with open(producer_report) as f:
                    result['borders']['producerReport']=borders.compare(result['borders'],json.load(f))
                require(result['borders']['producerReport']['ok'],'borderAudit',
                        json.dumps(result['borders']['producerReport']['differences'],sort_keys=True))
            ties=dict(accepted=0,exactTies=0,maxExcess=0.0,maxTau=0.0,maxExcessOverTau=0.0,
                      tolerance='exact oracle distances; tau = 16 * 2**-52 * (|centroid| + descendant source box diagonal) for producer arithmetic')
            for t in tiles:
                if not t['children']: continue
                for f in store.faces(int(t['id'])):
                    p=[sum(v[k] for v in f['xyz'])/3 for k in range(3)]
                    nearest,want=store.nearest(p,t['lo'],t['hi'],distance2)
                    actual=f['ids'].get('source_primitive',{})
                    primitive=tuple(actual.get(k) for k in KEYS[:3])
                    named=(primitive,actual.get('expanded_ordinal'),actual.get('material_index'))
                    if named!=(tuple(nearest[:3]),want['expanded_ordinal'],want['material_index']):
                        # D4 tie: the producer breaks ties on its own arithmetic. Accept
                        # the named primitive only within tau of the minimum distance.
                        box=store.source_box(t['lo'],t['hi'])
                        tau=TIE_ULPS*2.0**-52*(math.sqrt(sum(x*x for x in p))+
                            math.sqrt(sum((box[2*k+1]-box[2*k])**2 for k in range(3))))
                        minimum=distance2(p,want['xyz'])
                        hit=store.within(p,t['lo'],t['hi'],distance2,math.sqrt(minimum)+tau,
                            lambda identity,w:(tuple(identity[:3]),w['expanded_ordinal'],w['material_index'])==named)
                        require(hit is not None,'representativeIdentity',t['id'])
                        named_distance=distance2(p,hit[1]['xyz'])
                        require(within_tau(named_distance,minimum,tau),'representativeIdentity',t['id'])
                        # Reporting only; the decision above is exact.
                        excess=tie_excess(named_distance,minimum)
                        ties['accepted']+=1
                        ties['exactTies']+=named_distance==minimum
                        if excess>=ties['maxExcess']:
                            ties.update(maxExcess=excess,maxTau=tau)
                        # Zero tau (all-zero source and centroid) admits only exact ties.
                        ratio=excess/tau if tau>0 else (0.0 if excess==0 else math.inf)
                        ties['maxExcessOverTau']=max(ties['maxExcessOverTau'],ratio)
                        want=hit[1]
                    representatives+=1
                    require(material_class(f['material'])==material_class(want['material']),'representativeMaterialClass',t['id'])
            # D11 coarse drops: declared, recounted and within geometricError (drops.py).
            import drops
            result['coarseDrops']=drops.inspect(store,tiles,archive,require,distance2)
            # Upward omission, single first-renderable cover and empty-cell
            # refinement, from the same independent expansion (omission.py).
            from omission import inspect as inspect_omission, Failure as OmissionFailure
            try:
                result['omission']=inspect_omission(tiles,archive)
            except OmissionFailure as e:
                raise Failure(e.check,e.detail) from e
            if producer_manifest is not None:
                from omission import compare
                with open(producer_manifest) as f:
                    result['omission']['producerReport']=compare(result['omission'],json.load(f))
                require(result['omission']['producerReport']['ok'],'omissionReport',
                        json.dumps(result['omission']['producerReport']['differences'],sort_keys=True))
            result['representativeTies']=ties
            result.update(ok=True,leafTriangles=leaf_count,tiles=len(tiles),representatives=representatives,
                          coordinateFrame=coordinate_frame,profile=profile,
                          leafSourcePointsChecked=source_points,
                          candidateWork=work.used,sourceManifestSha256=file_hash(expected))
            result['attributes']=dict(attributes)
            result['uninspected']=(['textured leaf UV/source-pixel correspondence: crop/repack transforms require independent image-content proof']
                                   if attributes['leafTexcoordCorrespondenceUninspected'] else [])
            result['uninspected'].append('coarse face-interior orientation without a unique exact coincident descendant-leaf face is uninspected; directed retained-border witnesses certify only edge direction, not face-interior orientation; no continuous source-surface mapping is certified')
            return result
        finally:
            store.close()


def file_hash(path):
    digest=hashlib.sha256()
    with open(path,'rb') as f:
        for chunk in iter(lambda:f.read(1024*1024),b''):digest.update(chunk)
    return digest.hexdigest()


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('root');p.add_argument('expected')
    p.add_argument('--scratch');p.add_argument('--work-limit',type=int,default=300_000_000)
    p.add_argument('--coordinate-frame',choices=('root','world'),default='root')
    p.add_argument('--profile',choices=('hlod',),default='hlod')
    p.add_argument('--producer-manifest',help='S1 stage manifest.json; compared only after independent derivation')
    p.add_argument('--border-audit',help='S1b borders.jsonl audit sidecar: an index whose claims are re-derived')
    p.add_argument('--producer-report',help='producer run report naming the audit sidecar hash/bytes')
    p.add_argument('--triangle-cap',type=int,default=8192,help='hard payload cap the A1 relief trigger is a share of')
    a=p.parse_args()
    try:
        print(json.dumps(audit(a.root,a.expected,scratch=a.scratch,work_limit=a.work_limit,
                              coordinate_frame=a.coordinate_frame,profile=a.profile,producer_manifest=a.producer_manifest,
                              border_audit=a.border_audit,producer_report=a.producer_report,
                              triangle_cap=a.triangle_cap),sort_keys=True))
    except Failure as e:
        print(json.dumps(dict(ok=False,check=e.check,detail=e.detail)))
        raise SystemExit(1)
    except (ValueError,OSError,subprocess.SubprocessError,sqlite3.Error,zipfile.BadZipFile,KeyError,TypeError,IndexError) as e:
        print(json.dumps(dict(ok=False,check='oracleWorkLimit' if isinstance(e,(WorkLimit,SeamWorkLimit)) else 'oracleDomain',detail=str(e))))
        raise SystemExit(1)
