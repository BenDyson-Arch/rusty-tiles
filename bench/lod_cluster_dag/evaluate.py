#!/usr/bin/env python3
"""Decode-first geometric, colour, provenance and exhaustive interface oracle."""
import collections
import functools
import io
import json
import os
from pathlib import Path
import sys

import numpy as np
import point_cloud_utils as pcu
from PIL import Image
from scipy.spatial import cKDTree

from build import GLB


def stats(x):
    x=np.asarray(x)
    return dict(mean=float(x.mean()),p95=float(np.quantile(x,.95)),p99=float(np.quantile(x,.99)),max=float(x.max())) if len(x) else dict(mean=0.,p95=0.,p99=0.,max=0.)


class Surface:
    def __init__(self):
        self.xyz=[];self.uv=[];self.color=[];self.mat=[];self.materials=[];self.provenance=[];self.primitive_identity=[]

    def add(self, path, source=False):
        g=GLB(path); j=g.j; base=len(self.materials)
        for material in j.get('materials',[]):
            p=material.get('pbrMetallicRoughness',{}); image=None;wrap=(10497,10497)
            if 'baseColorTexture' in p:
                tex=j['textures'][p['baseColorTexture']['index']]
                sampler=j.get('samplers',[])[tex['sampler']] if 'sampler' in tex else {}
                wrap=(sampler.get('wrapS',10497),sampler.get('wrapT',10497))
                im=j['images'][tex.get('source',tex.get('extensions',{}).get('KHR_texture_basisu',{}).get('source'))]
                if 'uri' in im:
                    image=(str(g.path.parent/im['uri']),im.get('extras'))
                else: image=(g.view(im['bufferView']),None)
            self.materials.append((p.get('baseColorFactor',[1,1,1,1])[:3],image,wrap))
        default=len(self.materials);self.materials.append(([1,1,1],None,(10497,10497)))
        for node in j['nodes']:
            if 'mesh' not in node:continue
            assert not any(k in node for k in ['matrix','rotation','children'])
            for pi, pr in enumerate(j['meshes'][node['mesh']]['primitives']):
                pos=g.acc(pr['attributes']['POSITION'],True).astype(np.float64)
                pos=pos*np.array(node.get('scale',[1]*3))+np.array(node.get('translation',[0]*3))
                idx=g.acc(pr['indices']).reshape(-1,3) if 'indices' in pr else np.arange(len(pos)).reshape(-1,3)
                n=len(idx);self.xyz.append(pos[idx])
                self.uv.append(g.acc(pr['attributes']['TEXCOORD_0'],True)[idx] if 'TEXCOORD_0' in pr['attributes'] else np.zeros((n,3,2)))
                self.color.append(g.acc(pr['attributes']['COLOR_0'],True)[idx][...,:3] if 'COLOR_0' in pr['attributes'] else np.ones((n,3,3)))
                self.mat.append(np.full(n,base+pr['material'] if 'material' in pr else default,np.int32))
                if source:
                    ids=np.column_stack([np.full(n,node['mesh']),np.full(n,pi),np.arange(n),idx])
                    identity=ids[:,:2]
                else:
                    meta=j['extensions']['EXT_structural_metadata']
                    def feature_rows(fid):
                        if 'constant' in fid: rows=np.full(n,fid['constant'],np.int64)
                        else:
                            corners=g.acc(pr['attributes']['_FEATURE_ID_'+str(fid['attribute'])]).reshape(-1)[idx]
                            assert np.all(corners==corners[:,0,None]), 'feature ID varies within triangle'
                            assert np.all(corners==np.floor(corners)), 'feature IDs must be integers'
                            rows=corners[:,0].astype(np.int64)
                        assert rows.min()>=0 and rows.max()<fid['featureCount']
                        assert rows.max()<meta['propertyTables'][fid['propertyTable']]['count']
                        return rows
                    pf=next(f for f in pr['extensions']['EXT_mesh_features']['featureIds'] if f.get('label')=='source_primitive')
                    pt=meta['propertyTables'][pf['propertyTable']];pc=meta['schema']['classes'][pt['class']];rr=feature_rows(pf)
                    identity=np.column_stack([np.frombuffer(g.view(pt['properties'][name]['values']),
                        {'UINT32':'<u4','UINT64':'<u8'}[pc['properties'][name]['componentType']])[rr] for name in ['mesh_index','primitive_index']])
                    fid=next((f for f in pr.get('extensions',{}).get('EXT_mesh_features',{}).get('featureIds',[]) if f.get('label')=='source_triangle'),None)
                    if fid:
                        meta=j['extensions']['EXT_structural_metadata'];t=meta['propertyTables'][fid['propertyTable']];cls=meta['schema']['classes'][t['class']]
                        rows=feature_rows(fid)
                        cols=[]
                        for name in ['mesh_index','primitive_index','triangle_index','vertex_0','vertex_1','vertex_2']:
                            dt={'UINT32':'<u4','UINT64':'<u8'}[cls['properties'][name]['componentType']]
                            cols.append(np.frombuffer(g.view(t['properties'][name]['values']),dt)[rows])
                        ids=np.column_stack(cols)
                    else: ids=np.full((n,6),-1)
                self.provenance.append(ids)
                self.primitive_identity.append(identity)
        return self

    def finish(self):
        for k in ['xyz','uv','color','mat','provenance','primitive_identity']:setattr(self,k,np.concatenate(getattr(self,k)))
        self.vertices=np.ascontiguousarray(self.xyz.reshape(-1,3),np.float64)
        self.faces=np.arange(len(self.vertices),dtype=np.int32).reshape(-1,3)
        return self

    def sample(self, n, rng):
        area=np.linalg.norm(np.cross(self.xyz[:,1]-self.xyz[:,0],self.xyz[:,2]-self.xyz[:,0]),axis=1)
        fi=rng.choice(len(area),n,p=area/area.sum())
        r=rng.random((n,2));sr=np.sqrt(r[:,0]);bc=np.column_stack([1-sr,sr*(1-r[:,1]),sr*r[:,1]])
        return (self.xyz[fi]*bc[:,:,None]).sum(1),fi,bc

    def closest(self, q):
        return pcu.closest_points_on_mesh(np.ascontiguousarray(q,np.float64),self.vertices,self.faces)

    def rgb(self, fi, bc):
        uv=(self.uv[fi]*bc[:,:,None]).sum(1);vc=(self.color[fi]*bc[:,:,None]).sum(1)
        mi=self.mat[fi];rgb=np.zeros((len(fi),3))
        for m in np.unique(mi):
            mask=mi==m;factor,image,wrap=self.materials[m];v=vc[mask]*factor
            if image is not None:
                data,extras=image
                if isinstance(data,bytes):im=np.asarray(Image.open(io.BytesIO(data)).convert('RGB'))
                elif extras and data.endswith('.rgba'):im=np.fromfile(data,np.uint8).reshape(extras['height'],extras['width'],4)[:,:,:3]
                else:im=np.asarray(Image.open(data).convert('RGB'))
                h,w=im.shape[:2];tex=uv[mask]*[w,h]-.5;xy=np.floor(tex).astype(int);frac=tex-xy
                # Honor glTF sampler wrap modes; bilinear level 0, linear-light filtering.
                def address(x,size,mode):
                    if mode==33071:return np.clip(x,0,size-1)
                    if mode==10497:return x%size
                    assert mode==33648
                    u=x%(2*size);return np.where(u<size,u,2*size-1-u)
                out=np.zeros((mask.sum(),3))
                for dx,dy in [(0,0),(1,0),(0,1),(1,1)]:
                    c=im[address(xy[:,1]+dy,h,wrap[1]),address(xy[:,0]+dx,w,wrap[0]),:3]/255.
                    c=np.where(c<=.04045,c/12.92,((c+.055)/1.055)**2.4)
                    weight=(frac[:,0] if dx else 1-frac[:,0])*(frac[:,1] if dy else 1-frac[:,1])
                    out+=c*weight[:,None]
                v*=out
            rgb[mask]=v
        return rgb


def lab(linear):
    xyz=linear@np.array([[.4124564,.3575761,.1804375],[.2126729,.7151522,.0721750],[.0193339,.1191920,.9503041]]).T
    xyz/=[.95047,1,1.08883];d=6/29
    f=np.where(xyz>d**3,np.cbrt(xyz),xyz/(3*d*d)+4/29)
    return np.column_stack([116*f[:,1]-16,500*(f[:,0]-f[:,1]),200*(f[:,1]-f[:,2])])


def topology(xyz):
    pos,w=np.unique(xyz.reshape(-1,3),axis=0,return_inverse=True);w=w.reshape(-1,3).astype(np.int64)
    a=w[:,[0,1,2]].ravel();b=w[:,[1,2,0]].ravel();e=np.sort(np.column_stack([a,b]),axis=1)
    key=e[:,0]*len(pos)+e[:,1];order=np.argsort(key,kind='stable');ks=key[order]
    starts=np.r_[0,np.flatnonzero(np.diff(ks))+1];counts=np.diff(np.r_[starts,len(ks)])
    return pos,e,order,starts,counts


def cracks(src, root, nodes, out, baseline=False):
    """All disjoint topologically adjacent tree pairs, all depth differences.

    Canonical source interface samples projected to each decoded OPEN boundary;
    their separation is a conservative correspondence-based boundary discrepancy.
    Exact segment retention is checked separately and proves zero width on candidates.
    """
    pos,edges,order,starts,counts=topology(src.xyz)
    # Retain manifold shared edges; non-manifold incidences checked separately below.
    edgefaces=[];edgeends=[]
    for s,c in zip(starts[counts>=2],counts[counts>=2]):
        faces=order[s:s+c]//3
        for i in range(c):
            for k in range(i+1,c):
                edgefaces.append([faces[i],faces[k]]);edgeends.append(edges[order[s]])
    edgefaces=np.array(edgefaces);edgeends=np.array(edgeends)
    lookup={tuple(map(int,p[:3])):i for i,p in enumerate(src.provenance)}
    owner=np.full(len(src.xyz),-1,np.int32);surfaces={};leaf_exact=True;leaf_pos_max=0.;leaf_attr_max=0.
    known_primitives=set(map(tuple,src.primitive_identity))
    for nid,n in nodes.items():
        surf=Surface().add(root/n['file']).finish();surfaces[nid]=surf
        assert set(map(tuple,surf.primitive_identity))<=known_primitives
        if not n['children']:
            original=np.array([lookup[tuple(map(int,p[:3]))] for p in surf.provenance])
            assert len(np.unique(original))==len(original), 'duplicate triangle within leaf'
            assert np.all(owner[original]==-1);owner[original]=nid
            assert np.array_equal(src.provenance[original],surf.provenance)
            assert np.array_equal(surf.primitive_identity,surf.provenance[:,:2])
            leaf_exact &= np.array_equal(src.xyz[original],surf.xyz)
            leaf_pos_max=max(leaf_pos_max,float(np.abs(src.xyz[original]-surf.xyz).max()))
            leaf_attr_max=max(leaf_attr_max,float(np.abs(src.uv[original]-surf.uv).max()),float(np.abs(src.color[original]-surf.color).max()))
    assert np.all(owner>=0)
    ancestors={}
    for nid in nodes:
        a=[];q=nid
        while q is not None:a.append(q);q=nodes[q]['parent']
        ancestors[nid]=a
    pair_edges=collections.defaultdict(list)
    for e,(fa,fb) in enumerate(edgefaces):
        a,b=int(owner[fa]),int(owner[fb])
        if a==b:continue
        aa=set(ancestors[a]);bb=set(ancestors[b])
        for x in aa-bb:
            for y in bb-aa:pair_edges[tuple(sorted((x,y)))].append(e)
    needed=collections.defaultdict(set)
    for (a,b),es in pair_edges.items():needed[a].update(es);needed[b].update(es)
    projections={};retention={};source_tree=cKDTree(pos)
    for nid,es in sorted(needed.items()):
        surf=surfaces[nid];pp,ee,oo,ss,cc=topology(surf.xyz)
        # Include all edges with odd/unpaired incidence; exact retained edges can also
        # be interior to a non-manifold source region, so test against all edges.
        unique_edges=ee[oo[ss]];border=ee[oo[ss[cc==1]]]
        es=np.array(sorted(es));ends=edgeends[es];aa=pos[ends[:,0]];bb=pos[ends[:,1]]
        t=np.linspace(0,1,5);q=(aa[:,None,:]*(1-t[None,:,None])+bb[:,None,:]*t[None,:,None]).reshape(-1,3)
        d,w=source_tree.query(pp,workers=1)
        valid=d==0
        mapped=np.sort(w[unique_edges],axis=1)
        keys=set((mapped[:,0]*len(pos)+mapped[:,1])[valid[unique_edges].all(1)].tolist())
        expected=ends[:,0]*len(pos)+ends[:,1]
        retained=np.array([int(e) in keys for e in expected])
        retention[nid]=dict(interface_edges=len(es),missing_exact_segments=int((~retained).sum()))
        if retained.all():projected=q.copy()
        else:
            assert len(border)>0
            vf=np.column_stack([border[:,0],border[:,1],border[:,1]]).astype(np.int32)
            # PCU finds the exact closest degenerate primitive, but its returned
            # barycentrics are undefined for segments. Reconstruct analytically.
            _,fi,_=pcu.closest_points_on_mesh(np.ascontiguousarray(q),np.ascontiguousarray(pp),vf)
            a=pp[border[fi,0]]; b=pp[border[fi,1]]; ab=b-a
            den=(ab*ab).sum(1); t=np.divide(((q-a)*ab).sum(1),den,out=np.zeros(len(q)),where=den>0)
            projected=a+np.clip(t,0,1)[:,None]*ab
            projected.reshape(-1,5,3)[retained]=q.reshape(-1,5,3)[retained]
        projections[nid]=(es,projected.reshape(-1,5,3))
    rows=[];mixed=[];same=[];ratios=[];surface_mixed=[];surface_same=[];surface_ratios=[]
    for (a,b),es in sorted(pair_edges.items()):
        es=np.array(es);ea,pa=projections[a];eb,pb=projections[b]
        qa=pa[np.searchsorted(ea,es)].reshape(-1,3);qb=pb[np.searchsorted(eb,es)].reshape(-1,3)
        width=np.linalg.norm(qa-qb,axis=1)
        assert np.isfinite(width).all()
        if width.max()==0: opening=width
        else:
            dab,_,_=surfaces[b].closest(qa);dba,_,_=surfaces[a].closest(qb)
            opening=np.maximum(dab,dba)
            assert np.isfinite(opening).all()
        da,db=nodes[a]['depth'],nodes[b]['depth'];coarse=a if da<db else b;ge=nodes[coarse]['ge']
        row=dict(a=a,b=b,depth_a=da,depth_b=db,source_edges=len(es),samples=len(width),width_m=stats(width),surface_opening_m=stats(opening),coarse_ge=ge)
        if da!=db:
            assert ge>0 or width.max()==0, 'positive crack with zero geometricError'
            mixed.append(width);r=width/ge if ge>0 else np.zeros_like(width);ratios.append(r);row['fraction_ge']=stats(r)
            surface_mixed.append(opening);sr=opening/ge if ge>0 else np.zeros_like(opening);surface_ratios.append(sr);row['surface_fraction_ge']=stats(sr)
        else:same.append(width);surface_same.append(opening)
        rows.append(row)
    result=dict(leaf_exact_positions=leaf_exact,leaf_max_position_delta=leaf_pos_max,leaf_max_uv_color_delta=leaf_attr_max,
        leaf_provenance_exact=True,all_corner_feature_ids_valid=True,proxy_primitive_ids_valid=True,
        source_triangles=len(src.xyz),source_nonmanifold_edges=int((counts>2).sum()),
        mixed_pairs=sum(r['depth_a']!=r['depth_b'] for r in rows),same_depth_pairs=sum(r['depth_a']==r['depth_b'] for r in rows),
        mixed_width_m=stats(np.concatenate(mixed)),mixed_fraction_ge=stats(np.concatenate(ratios)),
        mixed_surface_opening_m=stats(np.concatenate(surface_mixed)),mixed_surface_fraction_ge=stats(np.concatenate(surface_ratios)),
        same_depth_surface_opening_m=stats(np.concatenate(surface_same)),
        same_depth_width_m=stats(np.concatenate(same)),retention=retention,pairs=rows)
    Path(out).write_text(json.dumps(result,indent=2));print('cracks',json.dumps({k:v for k,v in result.items() if k not in ['pairs','retention']}),flush=True)


def load_nodes(root, baseline_original=None):
    if baseline_original:
        sys.path.insert(0,'/home/bend/.cache/rusty-tiles-evidence/leaf-gaps-20261024')
        from tile_tree import tree
        tt=tree(baseline_original);ids={f:i for i,f in enumerate(tt)}
        nodes={ids[f]:dict(file=f,depth=v['depth'],ge=v['ge'],parent=ids.get(v['parent']),children=[]) for f,v in tt.items()}
        for nid,n in nodes.items():
            if n['parent'] is not None:nodes[n['parent']]['children'].append(nid)
    else:nodes={n['id']:n for n in json.loads((Path(root)/'build.json').read_text())['nodes']}
    return nodes


def run(source, root, output, baseline_original=None):
    root=Path(root);output=Path(output);output.mkdir(exist_ok=True,parents=True)
    src=Surface().add(source,True).finish();nodes=load_nodes(root,baseline_original)
    rows=[];rng=np.random.default_rng(20261024)
    for depth in sorted(set(n['depth'] for n in nodes.values())):
        # Complete same-depth frontier: include any earlier terminal leaves.
        chosen=[n for n in nodes.values() if n['depth']==depth or (not n['children'] and n['depth']<depth)]
        surface=Surface()
        for n in chosen:surface.add(root/n['file'])
        surface.finish();distances=[];deltas=[];directions={}
        for label,a,b in [('source_to_lod',src,surface),('lod_to_source',surface,src)]:
            q,fi,bc=a.sample(200_000,rng);d,near,nbc=b.closest(q);distances.append(d)
            delta=np.linalg.norm(lab(a.rgb(fi,bc))-lab(b.rgb(near,nbc)),axis=1);deltas.append(delta)
            directions[label]=dict(samples=len(q),geometry_m=stats(d),delta_e76=stats(delta))
        row=dict(depth=depth,tiles=len(chosen),triangles=len(surface.xyz),geometry_m=stats(np.concatenate(distances)),delta_e76=stats(np.concatenate(deltas)),directions=directions)
        rows.append(row);print(json.dumps(row),flush=True)
        (output/'quality.json').write_text(json.dumps(rows,indent=2))
        del surface
    cracks(src,root,nodes,output/'cracks.json',bool(baseline_original))


if __name__=='__main__':run(*sys.argv[1:])
