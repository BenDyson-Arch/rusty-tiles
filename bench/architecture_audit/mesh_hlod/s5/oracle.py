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
from pathlib import Path
import subprocess
import sqlite3
import tempfile
import zipfile
from archive import Archive, expand
from continuity import check_tiles
from spatial import Store, Work, WorkLimit

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
    """Orthogonal triangle projection plus three closed segments, independent of S2."""
    a,b,c=face
    sub=lambda x,y:[x[k]-y[k] for k in range(3)]
    dot=lambda x,y:sum(u*v for u,v in zip(x,y))
    u,v,w=sub(b,a),sub(c,a),sub(p,a)
    uu,uv,vv,wu,wv=dot(u,u),dot(u,v),dot(v,v),dot(w,u),dot(w,v)
    det=uu*vv-uv*uv
    best=math.inf
    if det>0:
        x,y=(wu*vv-wv*uv)/det,(wv*uu-wu*uv)/det
        if x>=0 and y>=0 and x+y<=1:
            best=sum((w[k]-x*u[k]-y*v[k])**2 for k in range(3))
    for a,b in ((a,b),(b,c),(c,a)):
        d,w=sub(b,a),sub(p,a)
        length=dot(d,d)
        t=max(0,min(1,dot(w,d)/length)) if length else 0
        best=min(best,sum((w[k]-t*d[k])**2 for k in range(3)))
    return best

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


def audit(root,expected,*,scratch=None,work_limit=300_000_000,coordinate_frame='root',profile='hlod'):
    """Check every published cell. Only one payload/tile is decoded at a time.

    root frame removes the archive's common placement, conjugating relative tile
    transforms by standard glTF Y-up -> 3D Tiles Z-up. Source expectations stay
    in authored glTF coordinates. world frame retains placement and applies that
    conversion, so its source manifest must independently use the same frame.
    Legacy H17 explicitly relaxes only D4 coarse identity; geometry/interfaces,
    leaves, rendered attributes and winding remain checked.
    """
    from source_manifest import load_manifest
    with load_manifest(expected) as manifest, Archive(root) as archive, tempfile.TemporaryDirectory(prefix='s5-oracle-',dir=scratch) as temporary:
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
        leaf_count=representatives=legacy=0
        attributes=collections.Counter()
        try:
            for t in tiles:
                if coordinate_frame=='root':
                    relative=t['relative']
                    matrix=IDENTITY if relative==IDENTITY else multiply(inverse(yup),multiply(relative,yup))
                else:
                    matrix=multiply(t['transform'],yup)
                for uri in t['contents']:
                    decoded=decode(archive.read(uri))
                    faces=decoded['faces']
                    require(len(faces)<=1_000_000,'oracleWorkLimit','one payload exceeds 1,000,000 faces')
                    for f in faces:
                        structural_face(f)
                        transform_face(f,matrix)
                        ids=f['ids']
                        # Every HLOD payload participates, even if its identity
                        # declarations were completely removed by a mutant.
                        if profile=='hlod':
                            require('source_primitive' in ids,'featureIdentity',uri)
                            require(not t['children'] or 'source_triangle' not in ids,'featureIdentity','coarse source_triangle '+uri)
                        t['rendered']=True
                        if not t['children']:
                            key,want=check_leaf(f,manifest,attributes,geometry=False)
                            require(store.source_face(key,want,t['lo'],actual=f),'leafCoverage','duplicate '+key)
                            leaf_count+=1
                    store.put_faces(int(t['id']),faces)
                if not t['children']:
                    # Empty terminal availability cells contain no rendered
                    # source. Exact global multiplicity below decides coverage.
                    store.leaf_bounds(t['lo'])
                store.db.commit()
            require(leaf_count==len(manifest['triangles']),'leafCoverage',f'{leaf_count} != {len(manifest["triangles"])}')
            source_points=store.validate_leaf_points(require)
            result=store.census(tiles,leaves,require)
            from winding import inspect as inspect_winding
            result['winding']=inspect_winding(store,tiles,require)
            for t in tiles:
                if not t['children']: continue
                for f in store.faces(int(t['id'])):
                    p=[sum(v[k] for v in f['xyz'])/3 for k in range(3)]
                    nearest,want=store.nearest(p,t['lo'],t['hi'],distance2)
                    actual=f['ids'].get('source_primitive',{})
                    if profile=='hlod':
                        require(all(actual.get(k)==want[k] for k in ('expanded_ordinal','material_index')),'representativeIdentity',t['id'])
                        require(tuple(actual.get(k) for k in KEYS[:3])==nearest[:3],'representativeIdentity',t['id'])
                        representatives+=1
                    else: legacy+=1
                    require(material_class(f['material'])==material_class(want['material']),'representativeMaterialClass',t['id'])
            result.update(ok=True,leafTriangles=leaf_count,tiles=len(tiles),representatives=representatives,
                          legacyCoarseTriangles=legacy,coordinateFrame=coordinate_frame,profile=profile,
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
    p.add_argument('--profile',choices=('hlod','h17'),default='hlod')
    a=p.parse_args()
    try:
        print(json.dumps(audit(a.root,a.expected,scratch=a.scratch,work_limit=a.work_limit,
                              coordinate_frame=a.coordinate_frame,profile=a.profile),sort_keys=True))
    except Failure as e:
        print(json.dumps(dict(ok=False,check=e.check,detail=e.detail)))
        raise SystemExit(1)
    except (ValueError,OSError,subprocess.SubprocessError,sqlite3.Error,zipfile.BadZipFile,KeyError,TypeError,IndexError) as e:
        print(json.dumps(dict(ok=False,check='oracleWorkLimit' if isinstance(e,WorkLimit) else 'oracleDomain',detail=str(e))))
        raise SystemExit(1)
