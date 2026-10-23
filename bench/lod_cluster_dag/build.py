#!/usr/bin/env python3
"""Spatially nested meshlet groups, bottom-up LockBorder LOD; standalone research."""
import ctypes as C
import copy
import io
import json
import os
from pathlib import Path
import struct
import sys
import time

import numpy as np
from PIL import Image


DT = {5120: 'i1', 5121: 'u1', 5122: '<i2', 5123: '<u2', 5125: '<u4', 5126: '<f4'}
NC = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}


class GLB:
    def __init__(self, path):
        self.path = Path(path)
        b = self.path.read_bytes()
        size = struct.unpack_from('<I', b, 12)[0]
        self.j = json.loads(b[20:20+size])
        self.b = b[28+size:]
        self.cache = {}

    def view(self, i):
        if i in self.cache: return self.cache[i]
        v = self.j['bufferViews'][i]
        ext = v.get('extensions', {}).get('EXT_meshopt_compression')
        if ext:
            lib = C.CDLL(os.environ.get('LIBMESHOPT', '/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag/libmeshopt.so'))
            fn = {'ATTRIBUTES': lib.meshopt_decodeVertexBuffer, 'TRIANGLES': lib.meshopt_decodeIndexBuffer, 'INDICES': lib.meshopt_decodeIndexSequence}[ext['mode']]
            assert ext.get('filter', 'NONE') == 'NONE', 'baseline filtered buffers require decode.mjs'
            fn.argtypes = [C.c_void_p, C.c_size_t, C.c_size_t, C.c_void_p, C.c_size_t]
            data = self.b[ext.get('byteOffset',0):ext.get('byteOffset',0)+ext['byteLength']]
            result = C.create_string_buffer(ext['count']*ext['byteStride'])
            assert fn(result,ext['count'],ext['byteStride'],data,len(data)) == 0
            self.cache[i] = result.raw
            return result.raw
        return self.b[v.get('byteOffset', 0):v.get('byteOffset', 0)+v['byteLength']]

    def acc(self, i, normalized=False):
        a = self.j['accessors'][i]
        assert 'sparse' not in a
        v = self.j['bufferViews'][a['bufferView']]
        dt = np.dtype(DT[a['componentType']]); nc = NC[a['type']]
        x = np.ndarray((a['count'], nc), dt, self.view(a['bufferView']),
                       offset=a.get('byteOffset', 0), strides=(v.get('byteStride', nc*dt.itemsize), dt.itemsize)).copy()
        if normalized and a.get('normalized'):
            x = np.maximum(x.astype(np.float32)/np.iinfo(dt).max, -1)
        return x


def ptr(a):
    return a.ctypes.data_as(C.c_void_p) if a is not None else None


class Meshopt:
    def __init__(self, lib):
        self.lib = C.CDLL(lib)
        self.lib.meshopt_buildMeshletsBound.argtypes = [C.c_size_t]*3
        self.lib.meshopt_buildMeshletsBound.restype = C.c_size_t
        self.lib.meshopt_buildMeshlets.argtypes = [C.c_void_p]*4 + [C.c_size_t, C.c_void_p] + [C.c_size_t]*4 + [C.c_float]
        self.lib.meshopt_buildMeshlets.restype = C.c_size_t
        self.lib.meshopt_simplifyWithAttributes.argtypes = [C.c_void_p, C.c_void_p, C.c_size_t, C.c_void_p, C.c_size_t, C.c_size_t, C.c_void_p, C.c_size_t, C.c_void_p, C.c_size_t, C.c_void_p, C.c_size_t, C.c_float, C.c_uint, C.c_void_p]
        self.lib.meshopt_simplifyWithAttributes.restype = C.c_size_t

    def clusters(self, idx, pos):
        idx = np.ascontiguousarray(idx, np.uint32).reshape(-1)
        bound = self.lib.meshopt_buildMeshletsBound(len(idx), 64, 124)
        ml = np.zeros((bound, 4), np.uint32)
        vi = np.empty(bound*64, np.uint32); ti = np.empty(bound*124*3, np.uint8)
        n = self.lib.meshopt_buildMeshlets(ptr(ml), ptr(vi), ptr(ti), ptr(idx), len(idx), ptr(pos), len(pos), 12, 64, 124, 0.)
        return [vi[vo:vo+vc][ti[to:to+tc*3]].reshape(-1, 3).copy() for vo, to, vc, tc in ml[:n]]

    def simplify(self, idx, pos, attr, locks):
        idx = np.ascontiguousarray(idx, np.uint32).reshape(-1)
        out = np.empty_like(idx); err = C.c_float()
        weights = np.full(attr.shape[1], float(os.environ.get('CLUSTER_ATTRIBUTE_WEIGHT','1')), np.float32) if attr is not None else np.empty(0, np.float32)
        flags = 1|2|4|(32 if os.environ.get('CLUSTER_PERMISSIVE','0')=='1' else 0)
        n = self.lib.meshopt_simplifyWithAttributes(ptr(out), ptr(idx), len(idx), ptr(pos), len(pos), 12,
             ptr(attr), attr.strides[0] if attr is not None else 0, ptr(weights), len(weights), ptr(locks),
             (len(idx)//6)*3, 1e6, flags, C.byref(err))
        return out[:n].reshape(-1, 3).copy(), err.value


class Writer:
    def __init__(self, materials, textures, images, samplers, meshopt):
        self.j = dict(asset={'version': '2.0'}, buffers=[], bufferViews=[], accessors=[],
            scenes=[{'nodes': [0]}], scene=0, nodes=[{'mesh': 0}], meshes=[{'primitives': []}],
            materials=materials, textures=textures, images=images, samplers=samplers,
            extensionsUsed=['EXT_mesh_features', 'EXT_structural_metadata'])
        self.data = bytearray()
        self.tables = []
        self.meshopt = meshopt
        self.compress = os.environ.get('CLUSTER_COMPRESS','1')=='1'
        self.decoded_size = 0
        if self.compress:
            self.j['extensionsUsed'].append('EXT_meshopt_compression')
            self.j['extensionsRequired'] = ['EXT_meshopt_compression']

    def view(self, data, stride=4, mode='ATTRIBUTES'):
        while len(self.data)%4: self.data.append(0)
        i = len(self.j['bufferViews'])
        if self.compress:
            assert stride%4==0 and len(data)%stride==0
            lib=self.meshopt.lib;count=len(data)//stride
            if mode=='ATTRIBUTES':
                bound=lib.meshopt_encodeVertexBufferBound;encode=lib.meshopt_encodeVertexBuffer
                bound.argtypes=[C.c_size_t,C.c_size_t];bound.restype=C.c_size_t
                size=bound(count,stride);encode.argtypes=[C.c_void_p,C.c_size_t,C.c_void_p,C.c_size_t,C.c_size_t];encode.restype=C.c_size_t
                encoded=C.create_string_buffer(size);length=encode(encoded,size,data,count,stride)
            else:
                bound=lib.meshopt_encodeIndexBufferBound;encode=lib.meshopt_encodeIndexBuffer
                bound.argtypes=[C.c_size_t,C.c_size_t];bound.restype=C.c_size_t
                size=bound(count,int(np.frombuffer(data,'<u4').max())+1)
                encode.argtypes=[C.c_void_p,C.c_size_t,C.c_void_p,C.c_size_t];encode.restype=C.c_size_t
                encoded=C.create_string_buffer(size);length=encode(encoded,size,data,count)
            assert length>0
            self.j['bufferViews'].append(dict(buffer=1,byteOffset=self.decoded_size,byteLength=len(data),
                extensions={'EXT_meshopt_compression':dict(buffer=0,byteOffset=len(self.data),byteLength=length,byteStride=stride,count=count,mode=mode,filter='NONE')}))
            self.decoded_size+=len(data)
            self.data.extend(encoded.raw[:length])
        else:
            self.j['bufferViews'].append(dict(buffer=0, byteOffset=len(self.data), byteLength=len(data)))
            self.data.extend(data)
        return i

    def acc(self, x, original=None, position=False):
        x = np.ascontiguousarray(x)
        ct = {np.dtype(v): k for k, v in DT.items()}[x.dtype]
        n = x.shape[1] if x.ndim == 2 else 1
        a = dict(bufferView=self.view(x.tobytes(),x.dtype.itemsize*n,'TRIANGLES' if ct==5125 else 'ATTRIBUTES'), componentType=ct, count=len(x), type={v:k for k,v in NC.items()}[n])
        self.j['bufferViews'][a['bufferView']]['target'] = 34963 if ct==5125 else 34962
        if original and original.get('normalized'): a['normalized'] = True
        if position: a.update(min=x.min(0).tolist(), max=x.max(0).tolist())
        self.j['accessors'].append(a)
        return len(self.j['accessors'])-1

    def table(self, cls, cols):
        self.tables.append(dict(name=cls, **{'class': cls}, count=len(next(iter(cols.values()))),
            properties={k: {'values': self.view(np.asarray(v, '<u4').tobytes())} for k,v in cols.items()}))
        return len(self.tables)-1

    def save(self, path):
        # Every tile refers only to resources used by its own primitives.
        # Resource IDs are local glTF indices; source identity lives in metadata.
        primitives = self.j['meshes'][0]['primitives']
        mids = sorted({p['material'] for p in primitives if 'material' in p})
        materials = [copy.deepcopy(self.j['materials'][i]) for i in mids]
        tids = sorted({m['pbrMetallicRoughness']['baseColorTexture']['index'] for m in materials
                       if 'baseColorTexture' in m.get('pbrMetallicRoughness', {})})
        textures = [copy.deepcopy(self.j['textures'][i]) for i in tids]
        iids = sorted({t['source'] for t in textures})
        sids = sorted({t['sampler'] for t in textures if 'sampler' in t})
        for p in primitives:
            if 'material' in p: p['material'] = mids.index(p['material'])
        for m in materials:
            if 'baseColorTexture' in m.get('pbrMetallicRoughness', {}):
                t = m['pbrMetallicRoughness']['baseColorTexture']; t['index'] = tids.index(t['index'])
        for t in textures:
            t['source'] = iids.index(t['source'])
            if 'sampler' in t: t['sampler'] = sids.index(t['sampler'])
        self.j.update(materials=materials, textures=textures, images=[self.j['images'][i] for i in iids],
                      samplers=[self.j['samplers'][i] for i in sids])
        for name in ['materials', 'textures', 'images', 'samplers']:
            if not self.j[name]: del self.j[name]
        classes = {'primitive': ['mesh_index', 'primitive_index'],
                   'triangle': ['mesh_index', 'primitive_index', 'triangle_index', 'vertex_0', 'vertex_1', 'vertex_2']}
        self.j['extensions'] = {'EXT_structural_metadata': dict(schema={'id': 'cluster_tree', 'classes': {
            k: {'properties': {c: {'type': 'SCALAR', 'componentType': 'UINT32'} for c in cs}} for k,cs in classes.items()}},
            propertyTables=self.tables)}
        self.j['buffers'] = [{'byteLength': len(self.data)}]
        if self.compress:
            self.j['buffers'].append({'byteLength': self.decoded_size, 'extensions': {'EXT_meshopt_compression': {'fallback': True}}})
        js = json.dumps(self.j, separators=(',', ':')).encode()
        js += b' '*((-len(js))%4); self.data.extend(b'\0'*((-len(self.data))%4))
        path.write_bytes(struct.pack('<4sII', b'glTF', 2, 28+len(js)+len(self.data)) +
            struct.pack('<I4s', len(js), b'JSON') + js + struct.pack('<I4s', len(self.data), b'BIN\0') + self.data)


def build(source, output, lib, placement_tileset=None):
    start = time.process_time(); wall = time.monotonic()
    src = GLB(source); j = src.j; out = Path(output); out.mkdir(parents=True, exist_ok=True)
    (out/'content').mkdir(exist_ok=True); (out/'textures').mkdir(exist_ok=True)
    assert all(set(n) == {'mesh'} for n in j['nodes'])
    assert sorted(n['mesh'] for n in j['nodes']) == list(range(len(j['meshes'])))
    mo = Meshopt(lib); prims = []; clusters = []; source_triangles = 0
    for mi, mesh in enumerate(j['meshes']):
        for pi, pr in enumerate(mesh['primitives']):
            assert pr.get('mode',4)==4
            attrs = {k: src.acc(v) for k,v in pr['attributes'].items()}
            pos = attrs['POSITION']; assert pos.dtype == np.float32
            idx = src.acc(pr['indices']).astype(np.uint32).reshape(-1,3)
            key = 'TEXCOORD_0' if 'TEXCOORD_0' in attrs else 'COLOR_0' if 'COLOR_0' in attrs else None
            attr = np.ascontiguousarray(src.acc(pr['attributes'][key], True), np.float32) if key else None
            p = dict(mi=mi, pi=pi, attrs=attrs, idx=idx, pr=pr, attr=attr)
            pid = len(prims); prims.append(p)
            # Match unordered corner IDs, retaining original order in exact leaves.
            keys = {tuple(sorted(map(int,t))): i for i,t in enumerate(idx)}
            assert len(keys)==len(idx), 'duplicate triangles need multimap provenance'
            for c in mo.clusters(idx, pos):
                ids = np.array([keys[tuple(sorted(map(int,t)))] for t in c], np.uint32)
                clusters.append(dict(pid=pid, ids=ids, center=pos[c].mean((0,1))))
            source_triangles += len(idx)
    print('source', source_triangles, 'meshlets', len(clusters), flush=True)

    images = {'leaf': [], 'proxy': []}
    used_materials = {p['pr']['material'] for p in prims if 'material' in p['pr']}
    used_textures = {j['materials'][m]['pbrMetallicRoughness']['baseColorTexture']['index'] for m in used_materials
                     if 'baseColorTexture' in j['materials'][m].get('pbrMetallicRoughness', {})}
    used_images = {j['textures'][t]['source'] for t in used_textures}
    image_uv = {}; image_wrap = {}
    for p in prims:
        if 'TEXCOORD_0' not in p['attrs'] or 'material' not in p['pr']: continue
        material=j['materials'][p['pr']['material']].get('pbrMetallicRoughness',{})
        if 'baseColorTexture' not in material: continue
        texture=j['textures'][material['baseColorTexture']['index']]
        image_id=texture['source']
        sampler=j.get('samplers',[])[texture['sampler']] if 'sampler' in texture else {}
        wrap=(sampler.get('wrapS',10497),sampler.get('wrapT',10497))
        assert image_id not in image_wrap or image_wrap[image_id]==wrap
        image_wrap[image_id]=wrap
        image_uv.setdefault(image_id,[]).append(p['attrs']['TEXCOORD_0'])
        p['image_id']=image_id
    image_transforms={}
    for i, im in enumerate(j.get('images', [])):
        raw = src.view(im['bufferView']); suffix = '.jpg' if im['mimeType']=='image/jpeg' else '.png'
        if i in used_images:
            (out/'textures'/f'leaf-{i}{suffix}').write_bytes(raw)
            size=int(os.environ.get('CLUSTER_TEXTURE_SIZE','1024'))
            image = Image.open(io.BytesIO(raw)).convert('RGB');wh=np.array(image.size)
            uv=np.concatenate(image_uv[i]);low=np.zeros(2,dtype=int);high=wh
            if os.environ.get('CLUSTER_CROP_TEXTURE','1')=='1' and uv.min()>=0 and uv.max()<=1:
                low=np.floor(uv.min(0)*wh).astype(int)-2
                high=np.ceil(uv.max(0)*wh).astype(int)+2
            image_transforms[i]=(wh,low,high-low)
            # Include a real two-pixel apron across original texture edges;
            # clipping the crop at UV 0/1 would change repeat/clamp sampling.
            def address(a,extent,mode):
                if mode==33071:return np.clip(a,0,extent-1)
                if mode==10497:return a%extent
                assert mode==33648
                b=a%(2*extent);return np.where(b<extent,b,2*extent-1-b)
            x=address(np.arange(low[0],high[0]),wh[0],image_wrap[i][0])
            y=address(np.arange(low[1],high[1]),wh[1],image_wrap[i][1])
            image=Image.fromarray(np.asarray(image)[y[:,None],x[None,:]])
            image.thumbnail((size,size), Image.Resampling.LANCZOS)
            image.save(out/'textures'/f'proxy-{i}.jpg', quality=int(os.environ.get('CLUSTER_JPEG_QUALITY','85')))
        images['leaf'].append({'uri': f'../textures/leaf-{i}{suffix}'})
        images['proxy'].append({'uri': f'../textures/proxy-{i}.jpg'})
    nodes = []; levels = {}

    def split(cs, depth, parent):
        nid = len(nodes); node = dict(id=nid, depth=depth, parent=parent, children=[])
        nodes.append(node)
        if depth < 5 and len(cs)>1:
            centers = np.array([clusters[c]['center'] for c in cs]); axis = np.ptp(centers,0).argmax()
            order = np.argsort(centers[:,axis], kind='stable'); mid = len(cs)//2
            node['children'] = [split([cs[i] for i in order[:mid]], depth+1, nid), split([cs[i] for i in order[mid:]], depth+1, nid)]
        else:
            node['source'] = {}
            for c in cs:
                q = clusters[c]; node['source'].setdefault(q['pid'], []).append(q['ids'])
            node['source'] = {k:np.concatenate(v) for k,v in node['source'].items()}
        return nid

    split(list(range(len(clusters))), 0, None)
    # Lock original ownership interfaces explicitly, including non-manifold
    # junctions which the manifold-only LockBorder test cannot identify.
    allpos = np.concatenate([p['attrs']['POSITION'] for p in prims])
    _, weld = np.unique(allpos, axis=0, return_inverse=True)
    weld = weld.ravel(); offset = 0
    for p in prims:
        p['weld'] = weld[offset:offset+len(p['attrs']['POSITION'])]
        offset += len(p['attrs']['POSITION'])
    incidence = np.zeros(int(weld.max())+1, np.uint64)
    leaves = [n for n in nodes if not n['children']]
    assert len(leaves) <= 64
    for bit, n in enumerate(leaves):
        n['mask'] = 1 << bit
        for pid, ids in n['source'].items():
            p = prims[pid]
            np.bitwise_or.at(incidence, p['weld'][p['idx'][ids].ravel()], np.uint64(n['mask']))
    for n in reversed(nodes):
        if n['children']: n['mask'] = sum(nodes[c]['mask'] for c in n['children'])
    for node in reversed(nodes):
        leaf = not node['children']; ge = 0.; groups = {}; meshlets = 0
        if leaf:
            groups = {k:prims[k]['idx'][ids] for k,ids in node['source'].items()}
        else:
            kids = [nodes[c] for c in node['children']]
            for kid in kids:
                for pid, idx in kid['groups'].items(): groups.setdefault(pid, []).append(idx)
            maxerr = 0.
            mask = np.uint64(node['mask'])
            boundary = ((incidence & mask) != 0) & ((incidence & ~mask) != 0)
            for pid, arrays in groups.items():
                p = prims[pid]; merged = np.concatenate(arrays)
                locks = np.ascontiguousarray(boundary[p['weld']], np.uint8)
                simplified, err = mo.simplify(merged, p['attrs']['POSITION'], p['attr'], locks)
                groups[pid] = simplified; maxerr = max(maxerr, err)
                meshlets += len(mo.clusters(simplified, p['attrs']['POSITION']))
            ge = max(k['ge'] for k in kids) + maxerr
        node.update(groups=groups, ge=ge, meshlets=meshlets)
        kind = 'leaf' if leaf else 'proxy'
        wr = Writer(j.get('materials',[]), j.get('textures',[]), images[kind], j.get('samplers',[]), mo)
        lo = np.full(3,np.inf); hi = -lo; ntri = 0
        for pid, idx in sorted(groups.items()):
            p = prims[pid]; nt = len(idx); ntri += nt
            if leaf:
                used = idx.ravel(); indices = None
            else:
                used, inverse = np.unique(idx, return_inverse=True)
                indices = inverse.astype(np.uint32).reshape(-1)
            attrs = {}
            for k,a in p['attrs'].items():
                values=a[used]
                if not leaf and k=='TEXCOORD_0' and 'image_id' in p:
                    wh,low,size=image_transforms[p['image_id']]
                    values=np.asarray((values.astype(np.float64)*wh-low)/size,np.float32)
                attrs[k]=wr.acc(values,j['accessors'][p['pr']['attributes'][k]],k=='POSITION')
            pp = p['attrs']['POSITION'][used]; lo = np.minimum(lo,pp.min(0)); hi=np.maximum(hi,pp.max(0))
            pr = dict(attributes=attrs, mode=4)
            if 'material' in p['pr']: pr['material']=p['pr']['material']
            if indices is not None: pr['indices']=wr.acc(indices)
            tab = wr.table('primitive', dict(mesh_index=[p['mi']], primitive_index=[p['pi']]))
            fids = [dict(featureCount=1, constant=0, propertyTable=tab, label='source_primitive')]
            if leaf:
                ids = node['source'][pid]
                tab = wr.table('triangle', dict(mesh_index=np.full(nt,p['mi']), primitive_index=np.full(nt,p['pi']),
                    triangle_index=ids, vertex_0=idx[:,0], vertex_1=idx[:,1], vertex_2=idx[:,2]))
                attrs['_FEATURE_ID_0'] = wr.acc(np.repeat(np.arange(nt,dtype=np.float32),3))
                fids.append(dict(featureCount=nt, attribute=0, propertyTable=tab, label='source_triangle'))
            pr['extensions']={'EXT_mesh_features': {'featureIds': fids}}
            wr.j['meshes'][0]['primitives'].append(pr)
        if not leaf:
            lo = np.minimum(lo, np.min([nodes[c]['lo'] for c in node['children']],0))
            hi = np.maximum(hi, np.max([nodes[c]['hi'] for c in node['children']],0))
        node.update(lo=lo, hi=hi)
        filename=f'content/{node["id"]}.glb'; wr.save(out/filename)
        node.update(file=filename, triangles=ntri, bytes=(out/filename).stat().st_size)
        levels.setdefault(node['depth'], dict(tiles=0, triangles=0, glb_bytes=0, meshlets=0))
        lv=levels[node['depth']]; lv['tiles']+=1; lv['triangles']+=ntri; lv['glb_bytes']+=node['bytes']; lv['meshlets']+=meshlets
        print('tile',node['id'],'depth',node['depth'],'triangles',ntri,'ge',ge,flush=True)

    def tile(nid):
        n=nodes[nid]; center=(n['lo']+n['hi'])/2; half=(n['hi']-n['lo'])/2
        # glTF Y-up -> 3D Tiles Z-up (same convention as the baseline).
        center=center[[0,2,1]]*np.array([1,-1,1]); half=half[[0,2,1]]
        r=dict(boundingVolume={'box':center.tolist()+[half[0],0,0,0,half[1],0,0,0,half[2]]},
               geometricError=n['ge'], refine='REPLACE', content={'uri':n['file']})
        if n['children']: r['children']=[tile(c) for c in n['children']]
        return r
    # The document error measures omitting the entire asset, not root-vs-source
    # error. Otherwise Cesium can correctly cull the whole tileset in a wide view.
    tileset=dict(asset={'version':'1.1'}, geometricError=float(np.linalg.norm(nodes[0]['hi']-nodes[0]['lo'])), root=tile(0))
    if placement_tileset:
        placement = json.loads(Path(placement_tileset).read_text())['root'].get('transform')
        if placement: tileset['root']['transform'] = placement
    (out/'tileset.json').write_text(json.dumps(tileset,separators=(',',':')))
    for depth, lv in levels.items():
        kind='leaf' if depth==5 else 'proxy'
        lv['texture_bytes']=sum(p.stat().st_size for p in (out/'textures').glob(kind+'-*'))
        lv['total_bytes']=lv['glb_bytes']+lv['texture_bytes']
    report=dict(source_triangles=source_triangles, initial_meshlets=len(clusters), levels=levels,
        options={k:os.environ.get(k,v) for k,v in dict(CLUSTER_PERMISSIVE='0',CLUSTER_COMPRESS='1',CLUSTER_TEXTURE_SIZE='1024',CLUSTER_ATTRIBUTE_WEIGHT='1',CLUSTER_CROP_TEXTURE='1',CLUSTER_JPEG_QUALITY='85').items()},
        cpu_s=time.process_time()-start, wall_s=time.monotonic()-wall,
        output_bytes=sum(p.stat().st_size for p in out.rglob('*') if p.is_file()),
        nodes=[{k:v for k,v in n.items() if k not in ('groups','source','lo','hi')} for n in nodes])
    (out/'build.json').write_text(json.dumps(report,indent=2))


if __name__=='__main__':
    build(*sys.argv[1:])
