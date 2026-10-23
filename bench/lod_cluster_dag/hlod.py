#!/usr/bin/env python3
"""Classic HLOD over exact seed leaves, welded geometry and fresh source bake."""
import ctypes as C
import copy
import json
import io
import os
from pathlib import Path
import shutil
import sys
import time
import numpy as np
import point_cloud_utils as pcu
from PIL import Image
from scipy.ndimage import distance_transform_edt
from build import GLB, Meshopt, Writer, ptr
from evaluate import Surface


def sample(xyz, count, rng):
    area=np.linalg.norm(np.cross(xyz[:,1]-xyz[:,0],xyz[:,2]-xyz[:,0]),axis=1)
    fi=rng.choice(len(xyz),count,p=area/area.sum());r=rng.random((count,2));s=np.sqrt(r[:,0])
    bc=np.column_stack([1-s,s*(1-r[:,1]),s*r[:,1]])
    return (xyz[fi]*bc[:,:,None]).sum(1)


def closest(q, xyz):
    return pcu.closest_points_on_mesh(np.ascontiguousarray(q,np.float64),np.ascontiguousarray(xyz.reshape(-1,3),np.float64),np.arange(xyz.size//3,dtype=np.int32).reshape(-1,3))


def main(source, seed, output, meshopt, charts):
    started=time.process_time();seed=Path(seed);out=Path(output);out.mkdir(parents=True,exist_ok=True)
    (out/'content').mkdir(exist_ok=True);(out/'textures').mkdir(exist_ok=True)
    assert all(set(n)=={'mesh'} for n in GLB(source).j['nodes']), 'only identity mesh nodes supported'
    src=Surface().add(source,True).finish();mo=Meshopt(meshopt);lib=C.CDLL(charts)
    # Decode each source image once; parent raster queries reuse these pixels.
    cache_started=time.process_time()
    src.materials=[(factor,(np.asarray(Image.open(io.BytesIO(image[0])).convert('RGB')),None) if image else None,wrap) for factor,image,wrap in src.materials]
    cache_cpu=time.process_time()-cache_started
    lib.unwrap.argtypes=[C.c_void_p,C.c_void_p,C.c_int,C.c_int,C.c_void_p,C.c_void_p,C.c_void_p]
    nodes=copy.deepcopy(json.loads((seed/'build.json').read_text())['nodes']);byid={n['id']:n for n in nodes}
    # Reuse leaf ownership, never seed proxies. Nonmanifold edge incidences
    # are indivisible ownership units: splitting one sheet junction between
    # cells can let vertex-locked tiny components disappear during simplification.
    lookup={tuple(p[:3]):i for i,p in enumerate(src.provenance)}
    pos,weld=np.unique(src.xyz.reshape(-1,3).astype(np.float32),axis=0,return_inverse=True)
    triangles=weld.reshape(-1,3).astype(np.uint32);incidence=np.zeros(len(pos),np.uint64)
    leaves=[n for n in nodes if not n['children']];owner=np.full(len(triangles),-1,np.int32)
    for n in leaves:
        surf=Surface().add(seed/n['file']).finish()
        ids=np.array([lookup[tuple(p[:3])] for p in surf.provenance]);owner[ids]=n['id']
    assert np.all(owner>=0)
    original_owner=owner.copy()
    def edge_keys(f):
        e=np.sort(np.stack([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]],axis=1).reshape(-1,2),axis=1).astype(np.uint64)
        return e[:,0]*np.uint64(len(pos))+e[:,1]
    source_edges,inverse,counts=np.unique(edge_keys(triangles),return_inverse=True,return_counts=True)
    junctions=[np.unique(np.flatnonzero(inverse==e)//3) for e in np.flatnonzero(counts>2)]
    # Iterate the finite minimum-label union to cover connected junction edges.
    changed=True
    while changed:
        changed=False
        for faces in junctions:
            target=int(owner[faces].min())
            if np.any(owner[faces]!=target):owner[faces]=target;changed=True
    reassigned=int(np.count_nonzero(owner!=original_owner))
    face_masks=np.zeros(len(triangles),np.uint64);g=GLB(source);j=g.j
    images=[{'uri':f'../textures/leaf-{i}'+('.jpg' if im['mimeType']=='image/jpeg' else '.png')} for i,im in enumerate(j.get('images',[]))]
    for p in (seed/'textures').glob('leaf-*'):shutil.copyfile(p,out/'textures'/p.name)
    for n in leaves:
        ids=np.flatnonzero(owner==n['id']);assert len(ids)>0
        n['ids']=ids;n['idx']=triangles[ids];n['ge']=0.;n['triangles']=len(ids)
        np.bitwise_or.at(incidence,n['idx'].ravel(),np.uint64(n['mask']));face_masks[ids]=np.uint64(n['mask'])
        wr=Writer(j.get('materials',[]),j.get('textures',[]),images,j.get('samplers',[]),mo)
        for identity in np.unique(src.primitive_identity[ids],axis=0):
            mi,pi=map(int,identity);rows=ids[np.all(src.primitive_identity[ids]==identity,axis=1)]
            provenance=src.provenance[rows];p=j['meshes'][mi]['primitives'][pi];vi=provenance[:,3:].reshape(-1)
            attrs={k:wr.acc(g.acc(ai)[vi],j['accessors'][ai],k=='POSITION') for k,ai in p['attributes'].items()}
            nt=len(rows);attrs['_FEATURE_ID_0']=wr.acc(np.repeat(np.arange(nt,dtype=np.float32),3))
            pt=wr.table('primitive',dict(mesh_index=[mi],primitive_index=[pi]))
            tt=wr.table('triangle',{k:provenance[:,i] for i,k in enumerate(['mesh_index','primitive_index','triangle_index','vertex_0','vertex_1','vertex_2'])})
            pr=dict(attributes=attrs,mode=4,extensions={'EXT_mesh_features':{'featureIds':[
                dict(featureCount=1,constant=0,propertyTable=pt,label='source_primitive'),
                dict(featureCount=nt,attribute=0,propertyTable=tt,label='source_triangle')]}})
            if 'material' in p:pr['material']=p['material']
            wr.j['meshes'][0]['primitives'].append(pr)
        wr.save(out/n['file']);n['bytes']=(out/n['file']).stat().st_size
    edge_masks=np.zeros(len(source_edges),np.uint64);np.bitwise_or.at(edge_masks,inverse,np.repeat(face_masks,3))
    for n in reversed(nodes):
        if n['children']:n['ids']=np.concatenate([byid[c]['ids'] for c in n['children']])
    prep_cpu=time.process_time()-started-cache_cpu;coarse_started=time.process_time();rng=np.random.default_rng(93210)
    stages=dict(source_image_decode=cache_cpu,simplify=0.,charts=0.,bake_and_error=0.,write=0.)
    for n in reversed(nodes):
        original=src.xyz[n['ids']];n['lo']=original.min((0,1));n['hi']=original.max((0,1))
        if not n['children']:continue
        t=time.process_time();merged=np.concatenate([byid[c]['idx'] for c in n['children']]);mask=np.uint64(n['mask'])
        locks=np.ascontiguousarray(((incidence&mask)!=0)&((incidence&~mask)!=0),np.uint8)
        # No attributes, material splits or UV seam vertices enter simplification.
        flat=np.ascontiguousarray(merged.reshape(-1));result=np.empty_like(flat);err=C.c_float()
        target=max(512,min(16000,len(merged)//2))
        count=mo.lib.meshopt_simplifyWithAttributes(ptr(result),ptr(flat),len(flat),ptr(pos),len(pos),12,None,0,None,0,ptr(locks),target*3,1e6,1|2|4,C.byref(err))
        idx=result[:count].reshape(-1,3).copy()
        required=source_edges[((edge_masks&mask)!=0)&((edge_masks&~mask)!=0)]
        assert np.isin(required,edge_keys(idx)).all(), 'simplifier removed an outer interface segment'
        n['locked_interface_segments']=len(required)
        n.pop('meshlets',None)
        n['idx']=idx;xyz=pos[idx].astype(np.float64);n['triangles']=len(idx)
        stages['simplify']+=time.process_time()-t;t=time.process_time()
        size=int(os.environ.get('HLOD_ATLAS_SIZE','1024' if n['depth']<=1 else '512'));uv=np.empty((len(idx),3,2),np.float32)
        face=np.empty(size*size,np.int32);bc=np.empty((size*size,3),np.float32)
        nc=lib.unwrap(ptr(pos),ptr(idx),len(idx),size,ptr(uv),ptr(face),ptr(bc));assert nc>0,'chart packing failed'
        covered=face>=0;pixels=np.flatnonzero(covered)
        q=(xyz[face[covered]]*bc[covered,:,None]).sum(1)
        stages['charts']+=time.process_time()-t;t=time.process_time()
        ns=max(2048,200000//(2**n['depth']));coarseq=sample(xyz,ns,rng);sourceq=sample(original,ns,rng)
        # One source BVH construction per parent serves raster, centroid identity and error.
        queries=np.concatenate([q,xyz.mean(1),coarseq]);query_start=time.process_time();dist,near,nbc=closest(queries,original);query_cpu=time.process_time()-query_start
        color_start=time.process_time()
        assert np.isfinite(dist).all() and np.isfinite(nbc).all()
        colors=src.rgb(n['ids'][near[:len(q)]],nbc[:len(q)])
        srgb=np.where(colors<=.0031308,colors*12.92,1.055*np.maximum(colors,0)**(1/2.4)-.055)
        atlas=np.zeros((size*size,3),np.uint8);atlas[pixels]=np.rint(np.clip(srgb,0,1)*255).astype(np.uint8)
        # Fill empty texels from the nearest covered texel, including chart gutters.
        nearest=distance_transform_edt(~covered.reshape(size,size),return_distances=False,return_indices=True)
        atlas=atlas.reshape(size,size,3)[nearest[0],nearest[1]]
        Image.fromarray(atlas).save(out/'textures'/f'hlod-{n["id"]}.jpg',quality=90,subsampling=0)
        color_cpu=time.process_time()-color_start
        pid=src.primitive_identity[n['ids'][near[len(q):len(q)+len(idx)]]]
        back,_,_=closest(sourceq,xyz);measured=max(float(back.max()),float(dist[-ns:].max()))
        n['ge']=max(measured,max(byid[c]['ge'] for c in n['children']))
        n['query_cpu_s']=query_cpu;n['color_cpu_s']=color_cpu;n['measured_ge']=measured;n['error_samples_per_direction']=ns;n['atlas_size']=size;n['charts']=nc;n['covered_texels']=len(q)
        stages['bake_and_error']+=time.process_time()-t;t=time.process_time()
        wr=Writer([{'pbrMetallicRoughness':{'baseColorTexture':{'index':0},'metallicFactor':0.,'roughnessFactor':1.},'doubleSided':True}],
                  [{'source':0,'sampler':0}],[{'uri':f'../textures/hlod-{n["id"]}.jpg'}],[{'wrapS':33071,'wrapT':33071,'minFilter':9729,'magFilter':9729}],mo)
        # Geometry can cross primitive boundaries; representative face identity is
        # stored per triangle and is constant across all three corners.
        unique,features=np.unique(pid,axis=0,return_inverse=True)
        tab=wr.table('primitive',dict(mesh_index=unique[:,0],primitive_index=unique[:,1]))
        attrs={'POSITION':wr.acc(xyz.reshape(-1,3).astype(np.float32),position=True),'TEXCOORD_0':wr.acc(uv.reshape(-1,2)),
               '_FEATURE_ID_0':wr.acc(np.repeat(features.astype(np.float32),3))}
        wr.j['meshes'][0]['primitives']=[dict(attributes=attrs,mode=4,material=0,extensions={'EXT_mesh_features':{'featureIds':[
            dict(featureCount=len(unique),attribute=0,propertyTable=tab,label='source_primitive')]}})]
        wr.save(out/n['file']);n['bytes']=(out/n['file']).stat().st_size
        stages['write']+=time.process_time()-t
        print('tile',n['id'],'depth',n['depth'],'triangles',len(idx),'charts',nc,'ge',n['ge'],'cpu',time.process_time()-coarse_started,flush=True)
    ts=json.loads((seed/'tileset.json').read_text())
    def walk(t):
        n=next(n for n in nodes if n['file']==t['content']['uri']);t['geometricError']=n['ge']
        center=((n['lo']+n['hi'])/2)[[0,2,1]]*[1,-1,1];half=((n['hi']-n['lo'])/2)[[0,2,1]]
        t['boundingVolume']={'box':center.tolist()+[half[0],0,0,0,half[1],0,0,0,half[2]]}
        for c in t.get('children',[]):walk(c)
    walk(ts['root']);(out/'tileset.json').write_text(json.dumps(ts,separators=(',',':')))
    report=dict(source_triangles=len(src.xyz),nonmanifold_reassigned_triangles=reassigned,preparation_cpu_s=prep_cpu,coarse_cpu_s=time.process_time()-coarse_started+cache_cpu,
        cpu_s=time.process_time()-started,stage_cpu_s=stages,atlas_sizes_by_depth=[1024,1024,512,512,512],
        nodes=[{k:v for k,v in n.items() if k not in ('idx','ids','lo','hi')} for n in nodes])
    (out/'build.json').write_text(json.dumps(report,indent=2))

if __name__=='__main__':main(*sys.argv[1:])
