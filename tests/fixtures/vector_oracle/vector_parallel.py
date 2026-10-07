"""One deterministic tile candidate per process; spool access is read-only."""
import hashlib
import json
import math
import os
import pathlib
import sqlite3
import struct
import subprocess
import uuid
import numpy as np
from vector_source import coordinate_list


def encode(task, writer=None):
    if writer is None:import vector as writer
    spool,prefix,center,level,args,schemas,output = task
    center=np.asarray(center);output=pathlib.Path(output)
    tolerance=getattr(args,'lod_tolerance',.1);max_vertices=getattr(args,'max_vertices',65536);max_bytes=getattr(args,'max_bytes',4194304)
    max_parent_features=getattr(args,'max_parent_features',4096)
    reports=[]
    # Retain only budgeted simplified geometry and one source feature at a time.
    db=sqlite3.connect(pathlib.Path(spool).as_uri()+'?mode=ro',uri=True)
    try:
        cap=max_parent_features if level else args.max_features
        count=db.execute('SELECT COUNT(*) FROM (SELECT 1 FROM features WHERE path>=? AND path<? LIMIT ?)',
            (prefix,prefix+'~',cap+1)).fetchone()[0]
        if count>cap:
            return None,reports,'parentFeatures' if level else 'features',os.getpid()
        def features():
            for fid,data in db.execute('SELECT id,data FROM features WHERE path>=? AND path<? ORDER BY id',
                    (prefix,prefix+'~')):
                feature=json.loads(data)
                locked={tuple(p) for p in coordinate_list(feature['geometry']) if feature.get('_surface_fragment') or db.execute(
                    'SELECT shared FROM vertices WHERE x=? AND y=? AND z=?',p).fetchone()[0]}
                yield feature,locked
        def size(feature):return len(coordinate_list(feature['geometry']))
        def estimate(feature):return size(feature)*32+len(json.dumps(feature['properties']).encode())+2048
        result=_encode(features(),center,level,args,schemas,output,reports,tolerance,max_vertices,max_bytes,writer,size,estimate)
        return (*result,os.getpid())
    finally: db.close()


def _encode(features,center,level,args,schemas,output,reports,tolerance,max_vertices,max_bytes,writer,size,estimate):
    last_budget_reason=None
    items=[]; error=0.; vertices=0; approximate_bytes=0
    for feature,locked in features:
        if level:
            fallback=[]
            feature,e=writer.simplify_feature(feature,tolerance*2**(level-1),locked,fallback,
                parent_repair=getattr(args,'parent_repair',False))
            for value in fallback:
                reports.append(value)
            error=max(error,e)
        vertices+=size(feature); approximate_bytes+=estimate(feature)-(2048 if level else 0)
        if vertices>max_vertices:
            last_budget_reason='vertices'
            return None,reports,last_budget_reason
        # Metadata is irreducible; stop excessive accumulation before encoding.
        if approximate_bytes>max_bytes*2:
            last_budget_reason='estimatedBytes'
            return None,reports,last_budget_reason
        items.append(feature)
    serial=uuid.uuid4().hex
    groups=[]
    fills=[f for f in items if f.get('_surface_fragment')]
    vectors=[f for f in items if not f.get('_surface_fragment')]
    for feature in fills:
        boundary=[edge for triangle in feature['_triangle_boundaries'] for edge in triangle]
        if boundary:
            vectors.append(dict(properties=feature['properties'],geometry=dict(type='MultiLineString',coordinates=boundary)))
    if fills:groups.append(('fill',fills,True))
    if vectors:groups.append(('vector',vectors,False))
    contents=[];files=[];primitive_count=0;vertex_count=0;byte_count=0;rounding=0.;quantization=0.;before_bytes=0;polygon_reports=[]
    for role,features,fill_only in groups:
        uri=f't/{serial}-{role}.glb';file=output/uri;encoding={}
        writer.emit(features,file,lambda p:np.asarray(p,dtype=float)-center,
            getattr(args,'repair',False),polygon_reports if not level else None,
            getattr(args,'ambiguous_outlines',False),encoding,schemas,fill_only=fill_only,quantize=getattr(args,'quantize',False))
        primitive_count+=encoding['primitives']
        before_bytes+=encoding['beforeBytes']
        quantization=max(quantization,encoding['quantizationError'])
        helper=getattr(args,'meshopt_helper',None)
        if helper:
            compressed=subprocess.run([helper,'encode-vector-content','--input',str(file)],capture_output=True,text=True)
            if compressed.returncode:raise ValueError('meshopt encoding failed: '+compressed.stderr.strip())
        if fill_only:
            # Cesium 1.143 selects the draft vector GLB decoder at tileset
            # scope. A standard b3dm wrapper routes only fills to its model
            # decoder; the embedded GLB still uses modern feature metadata.
            glb=file.read_bytes();table=b'{"BATCH_LENGTH":0}'
            table+=b' '*(-(28+len(table))%8)
            before_bytes+=28+len(table)
            wrapped=struct.pack('<4s6I',b'b3dm',1,28+len(table)+len(glb),len(table),0,0,0)+table+glb
            file.unlink();uri=f't/{serial}-{role}.b3dm';file=output/uri;file.write_bytes(wrapped)
        uri='t/'+hashlib.sha256(file.read_bytes()).hexdigest()+file.suffix
        target=output/uri
        if target.exists():file.unlink()
        else:file.replace(target)
        file=target
        files.append(file);byte_count+=file.stat().st_size;vertex_count+=encoding['vertices']
        rounding=max(rounding,encoding['rounding'])
        content=dict(uri=uri)
        if not fill_only:content['extensions']={'3DTILES_content_gltf_vector':dict(vector=True)}
        contents.append(content)
    if vertex_count>max_vertices or byte_count>max_bytes:
        last_budget_reason='vertices' if vertex_count>max_vertices else 'bytes'
        return None,reports,last_budget_reason  # unused immutable candidates are pruned at publication
    if not level:
        reports.extend(polygon_reports)
    node=dict(extras=dict(featureFragments=len(items),vertices=vertex_count,primitives=primitive_count,encodedBytes=byte_count,geometryErrorMetres=error,
            positionRoundingMetres=rounding,quantizationErrorMetres=quantization,uncompressedBytes=before_bytes,toleranceMetres=tolerance*2**(level-1) if level else 0),
            geometricError=error+rounding+quantization if level or getattr(args,'quantize',False) else 0)
    if len(contents)==1:node['content']=contents[0]
    else:node['contents']=contents
    return node,reports,last_budget_reason
