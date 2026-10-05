"""Disk-backed, budgeted OGR vector hierarchy; original identities survive fragments."""
import contextlib
import hashlib
import json
import math
import pathlib
import sqlite3
import struct
import sys
import tempfile

import numpy as np
from vector_source import Reader, coordinate_list
from vector_reuse import Reuse, canonical, contents


def run(args, writer):
    tolerance = getattr(args, 'lod_tolerance', .1)
    levels = getattr(args, 'lod_levels', 3)
    max_vertices = getattr(args, 'max_vertices', 65536)
    max_bytes = getattr(args, 'max_bytes', 4194304)
    max_tiles = getattr(args, 'max_tiles', 100000)
    if args.max_features < 1 or not math.isfinite(tolerance) or tolerance <= 0 or not 1 <= levels <= 16:
        raise ValueError('positive feature budget/tolerance and 1..16 LOD levels required')
    if max_vertices < 4 or max_bytes < 4096 or max_tiles < 1:
        raise ValueError('maxVertices >= 4, maxBytes >= 4096 and positive maxTiles required')
    output = pathlib.Path(args.output)
    output.mkdir(exist_ok=True)
    (output/'t').mkdir(exist_ok=True)
    reuse=Reuse(args,output,writer.encoder_digest)
    reader = Reader(args)
    counters = dict(features=0, fragments=0, leafTiles=0, tiles=0, routingTiles=0,
                    maximumTileVertices=0, maximumTileBytes=0, fragmentedPolygons=0)
    reports = []
    report_count = 0
    with contextlib.ExitStack() as cleanup, tempfile.TemporaryDirectory(prefix='.vector-', dir=output.parent) as scratch:
        db = sqlite3.connect(str(pathlib.Path(scratch)/'features.sqlite'))
        cleanup.callback(db.close)
        db.executescript('''PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=FILE;
        PRAGMA cache_size=-32768;
        CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,n INTEGER,estimate INTEGER,
          x REAL,y REAL,z REAL,lx REAL,ly REAL,lz REAL,hx REAL,hy REAL,hz REAL);
        ALTER TABLE features ADD COLUMN fingerprint TEXT;
        ALTER TABLE features ADD COLUMN sortkey TEXT;
        CREATE INDEX paths ON features(path);
        CREATE TABLE vertices(x REAL,y REAL,z REAL,owner INTEGER,shared INTEGER DEFAULT 0,PRIMARY KEY(x,y,z)) WITHOUT ROWID;
        ''')
        report_file = cleanup.enter_context((output/'geometry-reports.jsonl').open('w'))
        def report(value):
            nonlocal report_count
            report_count += 1
            report_file.write(json.dumps(value,allow_nan=False)+'\n')
            if len(reports)<100:
                reports.append(value)
        def size(feature):
            return len(coordinate_list(feature['geometry']))
        def estimate(feature):
            # A sizing hint only. Publication always checks actual encoded GLB bytes.
            return size(feature)*32+len(json.dumps(feature['properties']).encode())+2048
        def split(feature):
            g = feature['geometry']; kind, c = g['type'], g['coordinates']
            if kind == 'LineString' and len(c)>2:
                middle = len(c)//2
                parts = [dict(type=kind,coordinates=c[:middle+1]),dict(type=kind,coordinates=c[middle:])]
            elif kind in ('MultiPoint','MultiLineString','MultiPolygon') and len(c)>1:
                middle = len(c)//2
                parts = [dict(type=kind,coordinates=c[:middle]),dict(type=kind,coordinates=c[middle:])]
            elif kind in ('MultiLineString','MultiPolygon'):
                if feature.get('_surface_fragment'):
                    raise ValueError('indivisible polygon fill/boundary or metadata exceeds tile budget')
                return split(dict(feature,geometry=dict(type=kind[5:],coordinates=c[0])))
            elif kind == 'Polygon':
                # Preserve source triangulated surfaces, separating their real
                # boundaries from fill geometry to avoid artificial fragment edges.
                details = {}
                p, indices, *_ = writer.polygon(c,getattr(args,'repair',False),details)
                tri_indices=np.asarray(indices).reshape(-1,3).tolist()
                if len(tri_indices)<=1:
                    raise ValueError('one triangle or its metadata exceeds tile budget')
                edges={}
                for triangle in tri_indices:
                    for a,b in zip(triangle,triangle[1:]+triangle[:1]):
                        key=tuple(sorted((a,b)))
                        edges[key]=edges.get(key,0)+1
                triangles=[]; boundaries=[]
                for triangle in tri_indices:
                    triangles.append([np.asarray(p[i]).tolist() for i in triangle]+[np.asarray(p[triangle[0]]).tolist()])
                    boundaries.append([[np.asarray(p[a]).tolist(),np.asarray(p[b]).tolist()]
                        for a,b in zip(triangle,triangle[1:]+triangle[:1]) if edges[tuple(sorted((a,b)))]==1])
                counters['fragmentedPolygons'] += 1
                report(dict(sourceId=feature['properties']['_source_id'],sourceLayer=feature['properties']['_source_layer'],
                    reason='oversized polygon surfaces partitioned; original boundary rendered separately without internal edges',**details))
                middle=len(triangles)//2
                return [dict(feature,geometry=dict(type='MultiPolygon',coordinates=[[t] for t in triangles[a:b]]),
                             _surface_fragment=True,_triangle_boundaries=boundaries[a:b])
                        for a,b in ((0,middle),(middle,len(triangles)))]
            else:
                raise ValueError('indivisible geometry or its metadata exceeds tile budget; raise maxVertices/maxBytes')
            if feature.get('_surface_fragment') and kind=='MultiPolygon':
                middle=len(c)//2
                return [dict(feature,geometry=part,_triangle_boundaries=feature['_triangle_boundaries'][a:b])
                        for part,(a,b) in zip(parts,((0,middle),(middle,len(c))))]
            return [dict(feature,geometry=part) for part in parts]
        def insert(feature,path='', enforce_hint=True):
            if enforce_hint and (size(feature)>max_vertices or estimate(feature)>max_bytes):
                for index,part in enumerate(split(feature)):
                    insert(dict(part,_fragment_path=feature.get('_fragment_path','')+str(index)),path)
                return
            p = np.asarray(coordinate_list(feature['geometry']))
            lo,hi=p.min(axis=0),p.max(axis=0); center=(lo+hi)/2
            cursor=db.execute('INSERT INTO features(path,data,n,estimate,x,y,z,lx,ly,lz,hx,hy,hz) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)',
                (path,canonical(feature).decode(),len(p),estimate(feature),*center.tolist(),*lo.tolist(),*hi.tolist()))
            owner=cursor.lastrowid
            db.execute('UPDATE features SET sortkey=? WHERE id=?',
                (feature['properties']['_source_layer']+'\0'+feature['properties']['_source_id']+'\0'+feature.get('_fragment_path',''),owner))
            db.executemany('INSERT INTO vertices(x,y,z,owner) VALUES(?,?,?,?) ON CONFLICT(x,y,z) DO UPDATE SET shared=shared OR owner!=excluded.owner',
                           ((*point,owner) for point in p.tolist()))
        failures = 0
        first_failure = None
        accepted = {}
        rejected = {}
        def failure(value):
            nonlocal failures, first_failure
            failures += 1
            if first_failure is None:first_failure=value
            name=value['sourceLayer'];rejected[name]=rejected.get(name,0)+1
            value['outcome']='skipped' if getattr(args,'skip_invalid',False) else 'invalid'
            report(value)
            print(f"layer {name!r}, feature {value['sourceId']}: {value['reason']}",file=sys.stderr)
        reader.on_missing_geometry=report
        reader.on_feature_error=failure
        for feature in reader:
            db.execute('SAVEPOINT feature')
            try:
                writer.validate_feature(feature,getattr(args,'repair',False),getattr(args,'ambiguous_outlines',False))
                insert(feature)
            except (ValueError,RuntimeError,TypeError) as error:
                db.execute('ROLLBACK TO feature')
                failure(dict(sourceLayer=feature['properties']['_source_layer'],sourceId=feature['properties']['_source_id'],reason=str(error)))
            else:
                counters['features'] += 1
                name=feature['properties']['_source_layer'];accepted[name]=accepted.get(name,0)+1
            finally:
                db.execute('RELEASE feature')
            if counters['features']%1000==0:
                db.commit()
        for layer in reader.layer_reports:
            layer['features']=accepted.get(layer['name'],0)
            layer['invalidFeatures']=rejected.get(layer['name'],0)
        if failures and not getattr(args,'skip_invalid',False):
            raise ValueError(f"{failures} unconvertible feature(s); first failure: {first_failure['reason']}. No tileset published. Fix the reported features or explicitly use --skipInvalid.")
        counters['skippedFeatures']=failures
        db.commit()
        counters['featuresWithoutGeometry']=reader.features_without_geometry
        if not counters['features'] and not reuse.previous:
            if not reader.features_without_geometry:raise ValueError('empty selected layers')
            reader.anchor=np.zeros(3);reader.frame=np.eye(3)
        reuse.configure(args,reader)
        counters['fragments']=db.execute('SELECT COUNT(*) FROM features').fetchone()[0]
        def where(prefix):
            return (prefix,prefix+'~')
        def rows(prefix):
            # Independent cursor allows shared-coordinate lookups during iteration.
            for value, in db.execute('SELECT data FROM features WHERE path>=? AND path<? ORDER BY id',where(prefix)):
                yield json.loads(value)
        def stats(prefix):
            return db.execute('SELECT COUNT(*),SUM(n),SUM(estimate),MIN(lx),MIN(ly),MIN(lz),MAX(hx),MAX(hy),MAX(hz) FROM features WHERE path>=? AND path<?',where(prefix)).fetchone()
        def locks(feature):
            if feature.get('_surface_fragment'):
                return {tuple(p) for p in coordinate_list(feature['geometry'])}
            return {tuple(p) for p in coordinate_list(feature['geometry']) if db.execute(
                'SELECT shared FROM vertices WHERE x=? AND y=? AND z=?',p).fetchone()[0]}
        def fingerprint(fid,value):
            feature=json.loads(value)
            locked=locks(feature)
            mask=np.packbits([tuple(p) in locked for p in coordinate_list(feature['geometry'])]).tobytes()
            db.execute('UPDATE features SET fingerprint=? WHERE id=?',(hashlib.sha256(value.encode()+mask).hexdigest(),fid))
        for fid,value in db.execute('SELECT id,data FROM features'):
            fingerprint(fid,value)
        serial=0
        def encoded(prefix,center,level):
            nonlocal serial
            items=[]; error=0.; vertices=0; approximate_bytes=0
            for feature in rows(prefix):
                if len(items)>=args.max_features:
                    return None
                if level:
                    fallback=[]
                    feature,e=writer.simplify_feature(feature,tolerance*2**(level-1),locks(feature),fallback)
                    for value in fallback:
                        report(value)
                    error=max(error,e)
                vertices+=size(feature); approximate_bytes+=estimate(feature)
                if vertices>max_vertices:
                    return None
                # Metadata is irreducible; stop excessive accumulation before encoding.
                if approximate_bytes>max_bytes*2:
                    return None
                items.append(feature)
            serial+=1
            groups=[]
            fills=[f for f in items if f.get('_surface_fragment')]
            vectors=[f for f in items if not f.get('_surface_fragment')]
            for feature in fills:
                boundary=[edge for triangle in feature['_triangle_boundaries'] for edge in triangle]
                if boundary:
                    vectors.append(dict(properties=feature['properties'],geometry=dict(type='MultiLineString',coordinates=boundary)))
            if fills:groups.append(('fill',fills,True))
            if vectors:groups.append(('vector',vectors,False))
            contents=[];files=[];vertex_count=0;byte_count=0;rounding=0.;polygon_reports=[]
            for role,features,fill_only in groups:
                uri=f't/{serial}-{role}.glb';file=output/uri;encoding={}
                writer.emit(features,file,lambda p:np.asarray(p,dtype=float)-center,
                    getattr(args,'repair',False),polygon_reports if not level else None,
                    getattr(args,'ambiguous_outlines',False),encoding,reader.schemas,fill_only=fill_only)
                if fill_only:
                    # Cesium 1.143 selects the draft vector GLB decoder at tileset
                    # scope. A standard b3dm wrapper routes only fills to its model
                    # decoder; the embedded GLB still uses modern feature metadata.
                    glb=file.read_bytes();table=b'{"BATCH_LENGTH":0}'
                    table+=b' '*(-(28+len(table))%8)
                    wrapped=struct.pack('<4s6I',b'b3dm',1,28+len(table)+len(glb),len(table),0,0,0)+table+glb
                    file.unlink();uri=f't/{serial}-{role}.b3dm';file=output/uri;file.write_bytes(wrapped)
                uri='t/'+hashlib.sha256(file.read_bytes()).hexdigest()+file.suffix
                target=output/uri
                if target.exists():file.unlink()
                else:file.rename(target)
                file=target
                files.append(file);byte_count+=file.stat().st_size;vertex_count+=encoding['vertices']
                rounding=max(rounding,encoding['rounding'])
                content=dict(uri=uri)
                if not fill_only:content['extensions']={'3DTILES_content_gltf_vector':dict(vector=True)}
                contents.append(content)
            if vertex_count>max_vertices or byte_count>max_bytes:
                return None  # unused immutable candidates are pruned at publication
            if not level:
                for value in polygon_reports:report(value)
            node=dict(extras=dict(vertices=vertex_count,encodedBytes=byte_count,geometryErrorMetres=error,
                    positionRoundingMetres=rounding,toleranceMetres=tolerance*2**(level-1) if level else 0),
                    geometricError=error+rounding if level else 0)
            if len(contents)==1:node['content']=contents[0]
            else:node['contents']=contents
            return node
        def attach(node,lo,hi,center,children=()):
            # Node and GLB have the same local origin; children translate relative to it.
            zcenter=center[[0,2,1]]*[1,-1,1]
            half=np.maximum((hi-lo)[[0,2,1]]/2,.001)
            rounding=node.get('extras',{}).get('positionRoundingMetres',0.)
            for child in children:
                rounding=max(rounding,child['_padding'])
                delta=child['_center']-zcenter
                child['transform']=[1,0,0,0,0,1,0,0,0,0,1,0,*delta.tolist(),1]
            half+=rounding
            node.update(boundingVolume=dict(box=[0,0,0,half[0],0,0,0,half[1],0,0,0,half[2]]),refine='REPLACE',
                        _center=zcenter,_padding=rounding)
            if children:
                node['children']=list(children)
                node['geometricError']=max(node['geometricError'],*(c['geometricError'] for c in children))
            counters['tiles']+=1
            if counters['tiles']>max_tiles:
                raise ValueError('hierarchy exceeds maxTiles; raise budgets or maxTiles explicitly')
            if 'content' in node or 'contents' in node:
                counters['maximumTileVertices']=max(counters['maximumTileVertices'],node['extras']['vertices'])
                counters['maximumTileBytes']=max(counters['maximumTileBytes'],node['extras']['encodedBytes'])
            return node
        def build(prefix,depth=0):
            signature=reuse.signature(db,prefix)
            n=stats(prefix)[0]
            restored=reuse.restore(prefix,signature,counters,n,max_tiles)
            if restored is not None:return restored
            node=build_uncached(prefix,depth)
            reuse.remember(prefix,signature,node,stats(prefix)[0],reuse.cuts.get(prefix))
            return node
        def build_uncached(prefix,depth=0):
            if depth>64:
                raise ValueError('partition depth exceeds 64')
            n,vertices,est,*bounds=stats(prefix)
            lo,hi=np.asarray(bounds[:3]),np.asarray(bounds[3:]); center=(lo+hi)/2
            leaf=None
            if n<=args.max_features and vertices<=max_vertices and est<=max_bytes*2:
                leaf=encoded(prefix,center,0)
            if leaf:
                counters['leafTiles']+=1
                node=attach(leaf,lo,hi,center)
                for level in range(1,levels+1):
                    coarse=encoded(prefix,center,level)
                    if not coarse:
                        continue
                    if coarse['extras']['vertices']>=node['extras']['vertices']:
                        # Immutable candidates are pruned after the manifest is complete.
                        continue
                    node=attach(coarse,lo,hi,center,[node])
                return node
            if n==1:
                fid,value=db.execute('SELECT id,data FROM features WHERE path>=? AND path<?',where(prefix)).fetchone()
                parts=split(json.loads(value))
                db.execute('DELETE FROM features WHERE id=?',(fid,))
                for index,part in enumerate(parts):
                    insert(dict(part,_fragment_path=json.loads(value).get('_fragment_path','')+str(index)),prefix,enforce_hint=False)
                counters['fragments']+=len(parts)-1
                for fid,value in db.execute('SELECT id,data FROM features WHERE path>=? AND path<?',where(prefix)):
                    fingerprint(fid,value)
                return build_uncached(prefix,depth+1)
            cut=reuse.cuts.get(prefix)
            if cut is None:
                axis=('x','y','z')[int(np.argmax(hi-lo))]
                value,key=db.execute(f'SELECT {axis},sortkey FROM features WHERE path>=? AND path<? ORDER BY {axis},sortkey LIMIT 1 OFFSET ?',(*where(prefix),n//2-1)).fetchone()
                cut=dict(axis=axis,value=value,key=key)
                reuse.cuts[prefix]=cut
            axis=cut['axis']
            if axis not in ('x','y','z'):raise ValueError('invalid previous spatial partition axis')
            db.execute(f'UPDATE features SET path=? WHERE path=? AND ({axis}<? OR ({axis}=? AND sortkey<=?))',
                       (prefix+'0',prefix,cut['value'],cut['value'],cut['key']))
            db.execute('UPDATE features SET path=? WHERE path=?',(prefix+'1',prefix))
            children=[build(prefix+suffix,depth+1) for suffix in ('0','1') if stats(prefix+suffix)[0]]
            level=levels+int(math.ceil(math.log2(n/args.max_features)))
            coarse=encoded(prefix,center,max(1,level)) if n<=args.max_features else None
            if coarse is None:
                counters['routingTiles']+=1
                coarse=dict(geometricError=float(np.linalg.norm(hi-lo)),extras=dict(routing=True))
            return attach(coarse,lo,hi,center,children)
        if counters['features']:
            root=build('')
        else:
            root=dict(boundingVolume=dict(box=[0,0,0,.001,0,0,0,.001,0,0,0,.001]),geometricError=0,
                      refine='REPLACE',_center=np.zeros(3),_padding=0.)
            counters['tiles']=1
            reuse.remember('',reuse.signature(db,''),root,0)

        basis=reader.frame.T
        anchor=reader.anchor+basis@root['_center']
        root['transform']=[*basis[:,0].tolist(),0,*basis[:,1].tolist(),0,*basis[:,2].tolist(),0,*anchor.tolist(),1]
        def clean(node):
            node.pop('_center',None);node.pop('_padding',None)
            for child in node.get('children',[]):
                clean(child)
        clean(root)
        b=root['boundingVolume']['box']
        manifest=dict(asset=dict(version='1.1'),extensionsUsed=['3DTILES_content_gltf_vector'],
                      geometricError=max(1.,root['geometricError'],2*float(np.linalg.norm([b[3],b[7],b[11]]))),root=root)
        reuse_report=reuse.publish(manifest,reader)
        (output/'tileset.json').write_text(json.dumps(manifest,allow_nan=False))
        report_file.close()
        shared=db.execute('SELECT COUNT(*) FROM vertices WHERE shared=1').fetchone()[0]
        db.close()
    (output/'conversion.json').write_text(json.dumps(dict(**counters,inputDriver=reader.driver,layers=reader.layer_reports,
        budgets=dict(features=args.max_features,vertices=max_vertices,bytes=max_bytes,tiles=max_tiles),
        metadata=dict(listFields=getattr(args,'list_fields','error'),fields=getattr(args,'fields',[]) or [],dropFields=getattr(args,'drop_fields',[]) or []),
        skipInvalidEnabled=getattr(args,'skip_invalid',False),repairEnabled=getattr(args,'repair',False),lodToleranceMetres=tolerance,lodLevels=levels,reuse=reuse_report,
        lodFallbacks=reports,geometryReportCount=report_count,geometryReports='geometry-reports.jsonl',geometryReportsScope='current ingestion and newly encoded geometry; previous content reports remain in the prior archive',
        lockedSharedVertices=shared,pointPolicy='retain every semantic point feature; oversized parents route without content',
        polygonFragmentPolicy='standard glTF fills plus vector source boundaries; no internal fragment outlines',
        errorPolicy='direct original-to-parent distance plus float32 rounding; all source bounds retained'),indent=2))
    print(f"vector: {counters['features']} source features, {counters['fragments']} fragments, {counters['leafTiles']} leaves, {counters['tiles']} tiles")
