#!/usr/bin/env python3
"""Production ownership tree, exact leaves, local outer locks, GE-density rebaking."""
import ctypes as C
import copy,io,json,sys,time,shutil
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt
from build import GLB,Meshopt,Writer,ptr
from evaluate import Surface
from hlod import sample,closest
from octree_partition import partition


def edges(f,nv):
    e=np.sort(np.stack([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]],1).reshape(-1,2),axis=1).astype(np.uint64)
    return e[:,0]*np.uint64(nv)+e[:,1]


def leaf_writer(g,src,ids,mo,out,n):
    j=g.j;materials=[];textures=[];images=[];samplers=[];material_map={}
    for mi,pi in np.unique(src.primitive_identity[ids],axis=0):
        p=j['meshes'][mi]['primitives'][pi]
        if 'material' not in p or p['material'] in material_map:continue
        mid=p['material'];mat=copy.deepcopy(j['materials'][mid]);material_map[mid]=len(materials)
        assert not any(k in mat for k in ['normalTexture','occlusionTexture','emissiveTexture'])
        texinfo=mat.get('pbrMetallicRoughness',{}).get('baseColorTexture')
        if texinfo:
            tex=copy.deepcopy(j['textures'][texinfo['index']]);im=j['images'][tex['source']];iid=tex['source']
            name=f'leaf-{iid}'+('.jpg' if im['mimeType']=='image/jpeg' else '.png');path=out/'textures'/name
            if not path.exists():path.write_bytes(g.view(im['bufferView']))
            texinfo['index']=len(textures);tex['source']=len(images);images.append({'uri':'../textures/'+name})
            if 'sampler' in tex:
                sampler=j['samplers'][tex['sampler']];tex['sampler']=len(samplers);samplers.append(sampler)
            textures.append(tex)
        materials.append(mat)
    wr=Writer(materials,textures,images,samplers,mo)
    for identity in np.unique(src.primitive_identity[ids],axis=0):
        mi,pi=map(int,identity);rows=ids[np.all(src.primitive_identity[ids]==identity,axis=1)];provenance=src.provenance[rows]
        p=j['meshes'][mi]['primitives'][pi];vi=provenance[:,3:].reshape(-1)
        attrs={k:wr.acc(g.acc(ai)[vi],j['accessors'][ai],k=='POSITION') for k,ai in p['attributes'].items()}
        nt=len(rows);attrs['_FEATURE_ID_0']=wr.acc(np.repeat(np.arange(nt,dtype=np.float32),3))
        pt=wr.table('primitive',dict(mesh_index=[mi],primitive_index=[pi]))
        tt=wr.table('triangle',{k:provenance[:,i] for i,k in enumerate(['mesh_index','primitive_index','triangle_index','vertex_0','vertex_1','vertex_2'])})
        pr=dict(attributes=attrs,mode=4,extensions={'EXT_mesh_features':{'featureIds':[
            dict(featureCount=1,constant=0,propertyTable=pt,label='source_primitive'),dict(featureCount=nt,attribute=0,propertyTable=tt,label='source_triangle')]}})
        if 'material' in p:pr['material']=material_map[p['material']]
        wr.j['meshes'][0]['primitives'].append(pr)
    wr.save(out/n['file'])


def main(source,baseline,output,meshopt,charts):
    begin=time.process_time();out=Path(output);(out/'content').mkdir(parents=True,exist_ok=True);(out/'textures').mkdir(exist_ok=True)
    g=GLB(source);assert g.j['nodes']==[{'mesh':i} for i in range(len(g.j['meshes']))], 'one identity node per mesh in matching order required'
    src=Surface().add(source,True).finish();mo=Meshopt(meshopt);nodes,owner=partition(g,baseline);nv0=len(src.xyz)
    pos,inv=np.unique(src.xyz.reshape(-1,3).astype(np.float32),axis=0,return_inverse=True);tri=inv.reshape(-1,3).astype(np.uint32);del inv
    nv=len(pos);ek,inverse=np.unique(edges(tri,nv),return_inverse=True)
    emin=np.full(len(ek),len(nodes),np.int32);emax=np.full(len(ek),-1,np.int32)
    np.minimum.at(emin,inverse,np.repeat(owner,3));np.maximum.at(emax,inverse,np.repeat(owner,3))
    # Sparse cross-leaf incidences, not a dense vertex-by-leaf bit matrix.
    cross=emin!=emax;selected=np.flatnonzero(cross[inverse]);cross_id=inverse[selected];cross_owner=owner[selected//3]
    vmin=np.full(nv,len(nodes),np.int32);vmax=np.full(nv,-1,np.int32)
    np.minimum.at(vmin,tri.ravel(),np.repeat(owner,3));np.maximum.at(vmax,tri.ravel(),np.repeat(owner,3))
    del inverse,selected,cross
    for n in nodes:
        if n['children']:continue
        ids=np.flatnonzero(owner==n['leaf_begin']);n['ids']=ids;n['idx']=tri[ids];n['ge']=0.;n['triangles']=len(ids)
        n['outer']=np.unique(cross_id[cross_owner==n['leaf_begin']]);leaf_writer(g,src,ids,mo,out,n)
    del owner,cross_id,cross_owner,tri,g
    for n in reversed(nodes):
        if n['children']:
            n['ids']=np.concatenate([nodes[c]['ids'] for c in n['children']])
            boundary=np.unique(np.concatenate([nodes[c]['outer'] for c in n['children']]))
            n['outer']=boundary[(emin[boundary]<n['leaf_begin'])|(emax[boundary]>=n['leaf_end'])]
    del emin,emax
    prep=time.process_time()-begin;start=time.process_time();stages=dict(source_image_decode=0.,simplify=0.,error=0.,charts=0.,bake=0.,write=0.)
    t=time.process_time();src.materials=[(factor,(np.asarray(Image.open(io.BytesIO(image[0])).convert('RGB')),None) if image else None,wrap) for factor,image,wrap in src.materials]
    stages['source_image_decode']=time.process_time()-t
    del src.provenance,src.faces,src.vertices
    lib=C.CDLL(charts);lib.unwrap_density.argtypes=[C.c_void_p,C.c_void_p,C.c_int,C.c_int,C.c_void_p,C.c_void_p,C.c_void_p,C.c_double,C.c_void_p,C.c_void_p]
    rng=np.random.default_rng(93210);perdepth={d:sum(n['depth']==d and bool(n['children']) for n in nodes) for d in {n['depth'] for n in nodes}}
    for n in reversed(nodes):
        original=src.xyz[n['ids']];n['lo']=original.min((0,1));n['hi']=original.max((0,1))
        if not n['children']:continue
        t=time.process_time();merged=np.concatenate([nodes[c]['idx'] for c in n['children']]);used,loc=np.unique(merged,return_inverse=True)
        localpos=np.ascontiguousarray(pos[used]);flat=np.ascontiguousarray(loc.reshape(-1),np.uint32);locks=np.ascontiguousarray((vmin[used]<n['leaf_begin'])|(vmax[used]>=n['leaf_end']),np.uint8)
        required=ek[n['outer']];result=np.empty_like(flat);err=C.c_float();target=min(8192,len(merged))
        if len(merged)<=8192:idx=merged.copy()
        else:
            count=mo.lib.meshopt_simplifyWithAttributes(ptr(result),ptr(flat),len(flat),ptr(localpos),len(localpos),12,None,0,None,0,ptr(locks),target*3,1e6,2|4,C.byref(err))
            idx=used[result[:count]].reshape(-1,3).astype(np.uint32)
        assert len(idx)>0
        # Some tiny/nonmanifold components may disappear even when their vertices
        # are locked. Protect the affected input faces and retry the remainder.
        missing=required[~np.isin(required,edges(idx,nv))];protected=np.zeros(len(merged),bool);retries=0
        while len(missing) and retries<3:
            protected|=np.isin(edges(merged,nv).reshape(-1,3),missing).any(1)
            keep=merged[protected];rest=merged[~protected];retries+=1
            if not len(rest):idx=keep.copy();break
            used,loc=np.unique(rest,return_inverse=True);localpos=np.ascontiguousarray(pos[used]);flat=np.ascontiguousarray(loc.reshape(-1),np.uint32);result=np.empty_like(flat)
            locks=np.ascontiguousarray((vmin[used]<n['leaf_begin'])|(vmax[used]>=n['leaf_end'])|np.isin(used,keep),np.uint8)
            count=mo.lib.meshopt_simplifyWithAttributes(ptr(result),ptr(flat),len(flat),ptr(localpos),len(localpos),12,None,0,None,0,ptr(locks),max(0,target-len(keep))*3,1e6,2|4,C.byref(err))
            idx=np.concatenate([used[result[:count]].reshape(-1,3).astype(np.uint32),keep])
            missing=required[~np.isin(required,edges(idx,nv))]
        retained=False
        if not np.isin(required,edges(idx,nv)).all():idx=merged.copy();retained=True
        assert np.isin(required,edges(idx,nv)).all()
        n.update(idx=idx,triangles=len(idx),input_triangles=len(merged),triangle_target=target,simplifier_error=float(err.value),
                 stopping_bound='input' if len(merged)<=8192 else ('triangle_budget' if len(idx)<=8192 else 'topology_constraints'),
                 protected_faces=int(protected.sum()),retained_children=retained,locked_interface_segments=len(required))
        xyz=pos[idx].astype(np.float64);stages['simplify']+=time.process_time()-t;t=time.process_time()
        ns=max(2048,(200000+perdepth[n['depth']]-1)//perdepth[n['depth']]);coarseq=sample(xyz,ns,rng);sourceq=sample(original,ns,rng)
        dist,near,bc=closest(np.concatenate([coarseq,xyz.mean(1)]),original);back,_,_=closest(sourceq,xyz)
        measured=max(float(dist[:ns].max()),float(back.max()));n['ge']=max(measured,max(nodes[c]['ge'] for c in n['children']))
        pid=src.primitive_identity[n['ids'][near[ns:]]];stages['error']+=time.process_time()-t;t=time.process_time()
        spacing=max(n['ge']/16,1e-5);cap=1024;size=C.c_int();actual=C.c_double();uv=np.empty((len(idx),3,2),np.float32)
        face=np.empty(cap*cap,np.int32);bary=np.empty((cap*cap,3),np.float32)
        nc=lib.unwrap_density(ptr(pos),ptr(idx),len(idx),cap,ptr(uv),ptr(face),ptr(bary),spacing,C.byref(size),C.byref(actual));assert nc>0
        size=size.value;face=face[:size*size];bary=bary[:size*size];covered=face>=0;pixels=np.flatnonzero(covered)
        q=(xyz[face[covered]]*bary[covered,:,None]).sum(1);stages['charts']+=time.process_time()-t;t=time.process_time()
        # Limit query memory independently of page cap.
        atlas=np.zeros((size*size,3),np.uint8)
        for first in range(0,len(q),262144):
            _,near,bc=closest(q[first:first+262144],original);colors=src.rgb(n['ids'][near],bc)
            srgb=np.where(colors<=.0031308,colors*12.92,1.055*np.maximum(colors,0)**(1/2.4)-.055)
            atlas[pixels[first:first+len(near)]]=np.rint(np.clip(srgb,0,1)*255).astype(np.uint8)
        if covered.any():
            nearest=distance_transform_edt(~covered.reshape(size,size),return_distances=False,return_indices=True)
            atlas=atlas.reshape(size,size,3)[nearest[0],nearest[1]]
        else:
            _,near,bc=closest(xyz.mean(1),original);colors=src.rgb(n['ids'][near],bc).mean(0)
            atlas[:]=np.rint(np.clip(np.where(colors<=.0031308,colors*12.92,1.055*np.maximum(colors,0)**(1/2.4)-.055),0,1)*255);atlas=atlas.reshape(size,size,3)
        Image.fromarray(atlas).save(out/'textures'/f'hlod-{n["id"]}.jpg',quality=90,subsampling=0)
        n.update(measured_ge=measured,error_samples_per_direction=ns,atlas_size=size,charts=nc,covered_texels=len(q),planned_spacing_m=spacing,actual_spacing_m=actual.value,density_capped=actual.value>spacing*1.00001)
        stages['bake']+=time.process_time()-t;t=time.process_time()
        wr=Writer([{'pbrMetallicRoughness':{'baseColorTexture':{'index':0},'metallicFactor':0.,'roughnessFactor':1.},'doubleSided':True}],
                  [{'source':0,'sampler':0}],[{'uri':f'../textures/hlod-{n["id"]}.jpg'}],[{'wrapS':33071,'wrapT':33071,'minFilter':9729,'magFilter':9729}],mo)
        unique,features=np.unique(pid,axis=0,return_inverse=True);tab=wr.table('primitive',dict(mesh_index=unique[:,0],primitive_index=unique[:,1]))
        attrs={'POSITION':wr.acc(xyz.reshape(-1,3).astype(np.float32),position=True),'TEXCOORD_0':wr.acc(uv.reshape(-1,2)),'_FEATURE_ID_0':wr.acc(np.repeat(features.astype(np.float32),3))}
        wr.j['meshes'][0]['primitives']=[dict(attributes=attrs,mode=4,material=0,extensions={'EXT_mesh_features':{'featureIds':[dict(featureCount=len(unique),attribute=0,propertyTable=tab,label='source_primitive')]}})]
        wr.save(out/n['file']);stages['write']+=time.process_time()-t
        for c in n['children']:
            for key in ['idx','ids','outer']:del nodes[c][key]
        print('tile',n['id'],'depth',n['depth'],'triangles',len(idx),'atlas',size,'ge',n['ge'],'cpu',time.process_time()-start,flush=True)
    def tile(n):
        center=((n['lo']+n['hi'])/2)[[0,2,1]]*[1,-1,1];half=((n['hi']-n['lo'])/2)[[0,2,1]]
        t=dict(refine='REPLACE',content={'uri':n['file']},geometricError=n['ge'],boundingVolume={'box':center.tolist()+[half[0],0,0,0,half[1],0,0,0,half[2]]})
        if n['children']:t['children']=[tile(nodes[c]) for c in n['children']]
        return t
    ts=dict(asset={'version':'1.1'},geometricError=float(np.linalg.norm(nodes[0]['hi']-nodes[0]['lo'])),root=tile(nodes[0]));ts['root']['transform']=json.loads((Path(baseline)/'tileset.json').read_text())['root']['transform']
    (out/'tileset.json').write_text(json.dumps(ts,separators=(',',':')))
    report=dict(source_triangles=nv0,baseline=str(baseline),exact_baseline_ownership=True,preparation_cpu_s=prep,coarse_cpu_s=time.process_time()-start,cpu_s=time.process_time()-begin,stage_cpu_s=stages,
                nodes=[{k:v for k,v in n.items() if k not in ['idx','ids','outer','lo','hi']} for n in nodes])
    (out/'build.json').write_text(json.dumps(report,indent=2))

if __name__=='__main__':main(*sys.argv[1:])
