#!/usr/bin/env python3
"""Standalone UV/chart-locked LOD experiment. See README.md for scope and run.sh."""
import os
for k in ('OMP_NUM_THREADS','OPENBLAS_NUM_THREADS','MKL_NUM_THREADS','NUMEXPR_NUM_THREADS'):
    os.environ[k]='2'
import argparse,ctypes as C,io,json,math,struct,subprocess,sys,time,resource,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.spatial import cKDTree

HERE=Path(__file__).resolve().parent
EVID=Path('/home/bend/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/uv-downscale')
SCR=Path('/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/uv-downscale')
BASE=Path('/home/bend/scratch/rusty-tiles-scratch/proxy-density/tex3/view')
INPUT=Path('/home/bend/.cache/rusty-tiles-evidence/h17-20261013/crop')
LIB=C.CDLL('/home/bend/.cache/rt-wt/target-lod-uv-downscale/libmeshopt.so')
ptr=lambda a:C.c_void_p(a.ctypes.data)
LIB.meshopt_simplifyWithAttributes.restype=C.c_size_t
LIB.meshopt_simplifyWithAttributes.argtypes=[C.c_void_p,C.c_void_p,C.c_size_t,C.c_void_p,C.c_size_t,C.c_size_t,C.c_void_p,C.c_size_t,C.c_void_p,C.c_size_t,C.c_void_p,C.c_size_t,C.c_float,C.c_uint,C.c_void_p]

def dump(p,x):
    Path(p).write_text(json.dumps(x,indent=2))

def load_glb(p):
    b=Path(p).read_bytes();n=struct.unpack_from('<I',b,12)[0];j=json.loads(b[20:20+n]);binary=b[28+n:];cache={}
    def view(i):
        if i in cache:return cache[i]
        v=j['bufferViews'][i];e=v.get('extensions',{}).get('EXT_meshopt_compression')
        if e:
            src=binary[e.get('byteOffset',0):e.get('byteOffset',0)+e['byteLength']];out=np.empty(e['count']*e['byteStride'],np.uint8)
            fn={'ATTRIBUTES':'meshopt_decodeVertexBuffer','TRIANGLES':'meshopt_decodeIndexBuffer','INDICES':'meshopt_decodeIndexSequence'}[e['mode']]
            f=getattr(LIB,fn);f.argtypes=[C.c_void_p,C.c_size_t,C.c_size_t,C.c_char_p,C.c_size_t]
            assert f(ptr(out),e['count'],e['byteStride'],src,len(src))==0
            fl=e.get('filter','NONE')
            if fl!='NONE':
                f=getattr(LIB,{'OCTAHEDRAL':'meshopt_decodeFilterOct','QUATERNION':'meshopt_decodeFilterQuat','EXPONENTIAL':'meshopt_decodeFilterExp'}[fl]);f.argtypes=[C.c_void_p,C.c_size_t,C.c_size_t];f(ptr(out),e['count'],e['byteStride'])
            raw=out.tobytes()
        else:raw=binary[v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']]
        cache[i]=raw;return raw
    def acc(i):
        a=j['accessors'][i];v=j['bufferViews'][a['bufferView']];dt=np.dtype({5120:'i1',5121:'u1',5122:'<i2',5123:'<u2',5125:'<u4',5126:'<f4'}[a['componentType']]);nc={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']]
        x=np.ndarray((a['count'],nc),dt,buffer=view(a['bufferView']),offset=a.get('byteOffset',0),strides=(v.get('byteStride',dt.itemsize*nc),dt.itemsize)).copy()
        if a.get('normalized'):x=np.maximum(x.astype(float)/np.iinfo(dt).max,-1)
        return x
    return j,view,acc

def read_mesh(p,texdir=None):
    j,view,acc=load_glb(p);ps=[];uvs=[];cs=[];ids=[];ts=[];ims=[];images=[];kind=None;tg=0
    for i,im in enumerate(j.get('images',[])):
        raw=view(im['bufferView']) if 'bufferView' in im else (Path(p).parent/im['uri']).read_bytes()
        if raw[:2]==b'\xabK':
            q=texdir/(Path(p).stem+f'-{i}.ktx2');q.write_bytes(raw);images.append(str(q))
        else:
            q=texdir/(Path(p).stem+f'-{i}.png');Image.open(io.BytesIO(raw)).convert('RGB').save(q);images.append(str(q))
    meta=j.get('extensions',{}).get('EXT_structural_metadata')
    def col(table,key):
        pt=meta['propertyTables'][table];ty=meta['schema']['classes'][pt['class']]['properties'][key]['componentType'];return np.frombuffer(view(pt['properties'][key]['values']),{'UINT8':'u1','UINT16':'<u2','UINT32':'<u4','UINT64':'<u8'}[ty])
    for node in j['nodes']:
        if 'mesh' not in node:continue
        assert not any(k in node for k in ('matrix','rotation'))
        mi=node['mesh'];s=np.array(node.get('scale',[1]*3));tr=np.array(node.get('translation',[0]*3))
        for pi,pr in enumerate(j['meshes'][mi]['primitives']):
            pos=acc(pr['attributes']['POSITION']).astype(float)*s+tr;idx=acc(pr['indices']).ravel() if 'indices' in pr else np.arange(len(pos));nf=len(idx)//3
            uv=acc(pr['attributes']['TEXCOORD_0']) if 'TEXCOORD_0' in pr['attributes'] else np.zeros((len(pos),2))
            color=acc(pr['attributes']['COLOR_0'])[:,:3] if 'COLOR_0' in pr['attributes'] else np.ones((len(pos),3))
            mat=j.get('materials',[{}])[pr.get('material',0)];factor=np.array(mat.get('pbrMetallicRoughness',{}).get('baseColorFactor',[1]*4))[:3];color=color*factor
            feats=pr.get('extensions',{}).get('EXT_mesh_features',{}).get('featureIds',[])
            if feats:
                f=next(f for f in feats if f['label']=='source_primitive');fi=np.full(nf,f['constant'],int) if 'constant' in f else acc(pr['attributes']['_FEATURE_ID_'+str(f['attribute'])]).astype(int).ravel()[idx[::3]]
                pid=col(f['propertyTable'],'mesh_index')[fi].astype(np.int64)*1024+col(f['propertyTable'],'primitive_index')[fi]
                f=next((f for f in feats if f['label'] in ('source_triangle','proxy_source')),None)
                if f:
                    kind='leaf' if f['label']=='source_triangle' else 'proxy';fi=acc(pr['attributes']['_FEATURE_ID_'+str(f['attribute'])]).astype(int).ravel()[idx[::3]]
                    tid=col(f['propertyTable'],'triangle_index')[fi].astype(np.int64)
                else:kind='proxy';tid=np.full(nf,-1,np.int64)
            else:pid=np.full(nf,mi*1024+pi,np.int64);tid=np.arange(nf);kind='source'
            tex=mat.get('pbrMetallicRoughness',{}).get('baseColorTexture');im=-1
            if tex:
                t=j['textures'][tex['index']];im=t.get('source',t.get('extensions',{}).get('KHR_texture_basisu',{}).get('source'))
            ps.append(pos[idx].reshape(-1,3,3));uvs.append(uv[idx].reshape(-1,3,2));cs.append(color[idx].reshape(-1,3,3));ids.append(pid);ts.append(tid);ims.append(np.full(nf,im))
    return dict(p=np.concatenate(ps),uv=np.concatenate(uvs),c=np.concatenate(cs),pid=np.concatenate(ids),tid=np.concatenate(ts),im=np.concatenate(ims)),images,kind

def save_mesh(p,m):np.savez(p,**m)
def get_mesh(p):return dict(np.load(p))
def image_read(p):
    p=Path(p)
    if p.suffix=='.ktx2':
        w,h=json.loads(Path(str(p)+'.json').read_text());return Image.frombytes('RGBA',(w,h),Path(str(p)+'.rgba').read_bytes()).convert('RGB')
    return Image.open(p).convert('RGB')

def downscale(im):
    a=np.asarray(im,dtype=np.float32)/255
    a=np.where(a<=.04045,a/12.92,((a+.055)/1.055)**2.4)
    size=(max(1,im.width//2),max(1,im.height//2))
    a=np.stack([np.asarray(Image.fromarray(a[...,c]).resize(size,Image.Resampling.BOX)) for c in range(3)],axis=2)
    a=np.where(a<=.0031308,12.92*a,1.055*np.maximum(a,0)**(1/2.4)-.055)
    return Image.fromarray(np.clip(np.rint(a*255),0,255).astype(np.uint8))

def scratch_bytes(path):return sum(p.stat().st_size for p in Path(path).rglob('*') if p.is_file())

def prep(name):
    out=SCR/name;out.mkdir(parents=True,exist_ok=True);tex=out/'textures';tex.mkdir(exist_ok=True)
    src,ims,_=read_mesh(INPUT/(name+'.glb'),tex);save_mesh(out/'source.npz',src);dump(out/'source-images.json',ims)
    tree={};allimages=list(ims)
    if name=='tex3':
        sys.path.insert(0,'/home/bend/.cache/rusty-tiles-evidence/leaf-gaps-20261024');from tile_tree import tree as readtree
        tree=readtree(str(BASE))
        for i,(f,t) in enumerate(tree.items()):
            m,images,kind=read_mesh(BASE/f,tex);t.update(kind=kind,images=images,key=str(i),file=f,bytes=(BASE/f).stat().st_size)
            save_mesh(out/(str(i)+'.npz'),m);allimages+=images
    else:
        # Equivalent centroid octree, same full-detail triangle ownership; no splitting faces.
        def node(ix,parent,depth):
            key=str(len(tree));m={k:v[ix] for k,v in src.items()};tree[key]=dict(parent=parent,depth=depth,ge=0.,kind='leaf',key=key,file=key+'.glb',images=[],bytes=0)
            if len(ix)>8192:
                tree[key]['kind']='proxy';cen=m['p'].mean(1);mid=(cen.min(0)+cen.max(0))/2;octant=((cen>mid)*[1,2,4]).sum(1)
                for o in np.unique(octant):node(ix[octant==o],key,depth+1)
            else:save_mesh(out/(key+'.npz'),m)
            return key
        node(np.arange(len(src['p'])),None,0)
    dump(out/'tree.json',tree);ktx=list(dict.fromkeys(p for p in allimages if p.endswith('.ktx2')));dump(out/'ktx.json',ktx)
    if ktx:subprocess.run(['node',str(HERE/'decode_basis.cjs'),str(out/'ktx.json')],check=True)
    print('prepared',name,len(tree),flush=True)

def pack(meshes,images):
    # Repack whole child atlas rectangles (a conservative superset of their charts).
    rect=[]
    for ci,imlist in enumerate(images):
        for ii,p in enumerate(imlist):
            im=downscale(image_read(p));rect.append((ci,ii,im))
    if not rect:return None
    area=sum((im.width+4)*(im.height+4) for _,_,im in rect);width=2**math.ceil(math.log2(max(max(im.width+4 for _,_,im in rect),math.sqrt(area))))
    x=y=row=0;placed=[]
    for ci,ii,im in sorted(rect,key=lambda r:-r[2].height):
        w,h=im.size
        if x+w+4>width:x=0;y+=row;row=0
        placed.append((ci,ii,im,x+2,y+2));x+=w+4;row=max(row,h+4)
    height=2**math.ceil(math.log2(max(1,y+row)));atlas=Image.new('RGB',(width,height))
    for ci,ii,im,x,y in placed:
        # Extruded two-pixel rectangle gutters; charts' original gutters downscale too.
        a=np.array(im);a=np.pad(a,((2,2),(2,2),(0,0)),mode='edge');atlas.paste(Image.fromarray(a),(x-2,y-2))
        m=meshes[ci];mask=m['im']==ii;m['uv'][mask]=(m['uv'][mask]*im.size+[x,y])/[width,height]
    for m in meshes:m['im'][:]=0
    return atlas

def dedup(p,attr):
    x=np.ascontiguousarray(np.concatenate([p,attr],axis=1));_,keep,inv=np.unique(x,axis=0,return_index=True,return_inverse=True)
    return keep,inv.astype(np.uint32).reshape(-1,3)

def simplify(m,mode='locked'):
    outputs=[];unconstrained=0;locked_vertices=0
    groups=np.unique(m['pid']) if mode!='geometry' else [None]
    for pid in groups:
        sel=np.arange(len(m['p'])) if pid is None else np.flatnonzero(m['pid']==pid)
        p=m['p'][sel].reshape(-1,3);uv=m['uv'][sel].reshape(-1,2);co=m['c'][sel].reshape(-1,3)
        attr=np.zeros((len(p),1)) if mode=='geometry' else np.column_stack([uv,co])
        keep,ind=dedup(p,attr);pos=p[keep];attributes=np.ascontiguousarray(attr[keep],np.float32)
        a=ind.ravel();b=ind[:,[1,2,0]].ravel();edges=np.sort(np.column_stack([a,b]),axis=1);ue,count=np.unique(edges,axis=0,return_counts=True)
        locks=np.zeros(len(pos),np.uint8)
        if mode=='locked':locks[np.unique(ue[count==1])]=1
        locked_vertices+=int(locks.sum())
        pp=np.ascontiguousarray(pos-pos.mean(0),np.float32);weights=np.ones(attributes.shape[1],np.float32);weights[:2]=1.0
        # Allocate the 8192 budget across primitive groups in proportion to input tris.
        target=max(1,int(8192*len(sel)/len(m['p']))) if mode!='floor' else 1
        target=min(target,len(ind));out=np.empty(ind.size,np.uint32);err=C.c_float()
        n=LIB.meshopt_simplifyWithAttributes(ptr(out),ptr(ind),ind.size,ptr(pp),len(pp),12,ptr(attributes),attributes.shape[1]*4,ptr(weights),len(weights),ptr(locks),target*3,1.,1 if mode=='locked' else 0,C.byref(err))
        oi=out[:n].reshape(-1,3);soup=keep[oi]
        # Exact original positions and attrs; no relocation or quantization.
        outm=dict(p=p[soup],uv=uv[soup],c=co[soup],pid=np.full(len(oi),pid or 0,np.int64),tid=np.full(len(oi),-1,np.int64),im=np.full(len(oi),0 if np.any(m['im']>=0) else -1,np.int64))
        outputs.append(outm)
    return {k:np.concatenate([v[k] for v in outputs]) for k in m},locked_vertices

def write_glb(p,m,image=None,leaf=False,normals=None):
    data=bytearray();views=[];acc=[]
    def view(raw):
        while len(data)%4:data.append(0)
        v=len(views);views.append(dict(buffer=0,byteOffset=len(data),byteLength=len(raw)));data.extend(raw);return v
    def accessor(a,typ,ct=5126,normalized=False):
        a=np.ascontiguousarray(a,{5126:'<f4',5125:'<u4',5121:'u1'}[ct]);v=view(a.tobytes());i=len(acc);acc.append(dict(bufferView=v,componentType=ct,count=len(a),type=typ))
        if normalized:acc[-1]['normalized']=True
        return i
    origin=np.floor(m['p'].reshape(-1,3).min(0)/64)*64
    for axis in range(3):
        if not np.array_equal((m['p'][...,axis]-origin[axis]).astype(np.float32).astype(float)+origin[axis],m['p'][...,axis]):origin[axis]=0
    local=(m['p']-origin).astype(np.float32)
    assert np.array_equal(local.astype(float)+origin,m['p']),('coordinate roundtrip',p)
    prim=[];pids=np.unique(m['pid']);pt=[]
    for row,pid in enumerate(pids):
        ix=np.flatnonzero(m['pid']==pid);pa=accessor(local[ix].reshape(-1,3),'VEC3');acc[pa]['min']=local[ix].reshape(-1,3).min(0).astype(float).tolist();acc[pa]['max']=local[ix].reshape(-1,3).max(0).astype(float).tolist()
        at={'POSITION':pa};feat=[dict(featureCount=len(pids),constant=row,label='source_primitive',propertyTable=0)]
        if normals is not None and np.isfinite(normals[ix]).all():at['NORMAL']=accessor(normals[ix].reshape(-1,3),'VEC3')
        if image:at['TEXCOORD_0']=accessor(m['uv'][ix].reshape(-1,2),'VEC2')
        else:
            co=m['c'][ix].reshape(-1,3)
            if leaf:
                c8=np.rint(co*255).astype(np.uint8);assert np.array_equal(c8.astype(float)/255,co)
                at['COLOR_0']=accessor(np.column_stack([c8,np.full(len(c8),255,np.uint8)]),'VEC4',5121,True)
            else:at['COLOR_0']=accessor(co,'VEC3')
        if leaf:
            # Leaf source identity is per triangle and cannot vary within a face.
            at['_FEATURE_ID_1']=accessor(np.repeat(ix.astype(np.uint32),3)[:,None],'SCALAR',5125);feat.append(dict(featureCount=len(m['p']),attribute=1,label='source_triangle',propertyTable=1))
        prim.append(dict(attributes=at,mode=4,material=0,extensions={'EXT_mesh_features':{'featureIds':feat}}))
    def prop(a):return {'values':view(np.asarray(a,dtype='<u4').tobytes())}
    props={k:dict(type='SCALAR',componentType='UINT32') for k in ('mesh_index','primitive_index')}
    classes={'primitive':{'properties':props}};tables=[dict(class_='primitive',count=len(pids),properties={'mesh_index':prop(pids//1024),'primitive_index':prop(pids%1024)})];tables[0]['class']=tables[0].pop('class_')
    if leaf:
        fields={'mesh_index':m['pid']//1024,'primitive_index':m['pid']%1024,'triangle_index':m['tid']}
        classes['triangle']={'properties':{k:dict(type='SCALAR',componentType='UINT32') for k in fields}}
        tables.append({'class':'triangle','count':len(m['p']),'properties':{k:prop(v) for k,v in fields.items()}})
    j=dict(asset={'version':'2.0'},buffers=[],bufferViews=views,accessors=acc,meshes=[{'primitives':prim}],nodes=[dict(mesh=0,translation=origin.tolist())],scenes=[{'nodes':[0]}],scene=0,materials=[{'pbrMetallicRoughness':{'metallicFactor':0.,'roughnessFactor':1.}}],extensionsUsed=['EXT_mesh_features','EXT_structural_metadata'],extensions={'EXT_structural_metadata':{'schema':{'id':'uv-downscale','classes':classes},'propertyTables':tables}})
    if image:
        j['images']=[{'bufferView':view(Path(image).read_bytes()),'mimeType':'image/png'}];j['textures']=[{'source':0,'sampler':0}];j['samplers']=[{'magFilter':9729,'minFilter':9729,'wrapS':33071,'wrapT':33071}];j['materials'][0]['pbrMetallicRoughness']['baseColorTexture']={'index':0}
    j['buffers']=[{'byteLength':len(data)}];jb=json.dumps(j,separators=(',',':')).encode();jb+=b' '*((-len(jb))%4);data+=b'\x00'*((-len(data))%4)
    Path(p).write_bytes(struct.pack('<III',0x46546c67,2,28+len(jb)+len(data))+struct.pack('<II',len(jb),0x4e4f534a)+jb+struct.pack('<II',len(data),0x004e4942)+data)

def build(name):
    out=SCR/name;target=out/'output';target.mkdir(exist_ok=True);tree=json.loads((out/'tree.json').read_text());children={}
    for f,t in tree.items():children.setdefault(t['parent'],[]).append(f)
    # Leaf preparation is outside the measured coarse build. Preserve source normals too.
    normal_map={}
    if name=='crop3':
        sj,sv,sa=load_glb(INPUT/'crop3.glb')
        for mi,mesh in enumerate(sj['meshes']):
            for pi,pr in enumerate(mesh['primitives']):
                if 'NORMAL' in pr['attributes']:
                    ni=sa(pr['indices']).ravel() if 'indices' in pr else np.arange(len(sa(pr['attributes']['NORMAL'])))
                    normal_map[mi*1024+pi]=sa(pr['attributes']['NORMAL'])[ni].reshape(-1,3,3)
    for f,t in tree.items():
        if t['kind']=='leaf':
            t['built_images']=t['images'];m=get_mesh(out/(t['key']+'.npz'));save_mesh(target/(t['key']+'.npz'),m)
            if name=='tex3':(target/(t['key']+'.glb')).write_bytes((BASE/f).read_bytes())
            else:
                normals=np.full_like(m['p'],np.nan) if normal_map else None
                if normals is not None:
                    for pid in np.unique(m['pid']):
                        if int(pid) in normal_map:
                            ix=m['pid']==pid;normals[ix]=normal_map[int(pid)][m['tid'][ix]]
                write_glb(target/(t['key']+'.glb'),m,leaf=True,normals=normals)
    start=resource.getrusage(resource.RUSAGE_SELF);child_start=resource.getrusage(resource.RUSAGE_CHILDREN);tm=time.monotonic();rows=[]
    # Decode the supplied leaf GLBs inside the coarse timer, including meshopt and IDs.
    # The resulting arrays are the ones consumed by every subsequent parent merge.
    decode_start=resource.getrusage(resource.RUSAGE_SELF)
    for f,t in tree.items():
        if t['kind']=='leaf':
            supplied=BASE/f if name=='tex3' else target/(t['key']+'.glb')
            decoded,_,_=read_mesh(supplied,out/'textures')
            save_mesh(target/(t['key']+'.npz'),decoded)
    decode_end=resource.getrusage(resource.RUSAGE_SELF)
    leaf_mesh_decode_cpu=decode_end.ru_utime+decode_end.ru_stime-decode_start.ru_utime-decode_start.ru_stime
    # Published leaves are supplied as compressed GLBs. Include their KTX2 transcode CPU.
    leafktx=list(dict.fromkeys(p for t in tree.values() if t['kind']=='leaf' for p in t['images'] if p.endswith('.ktx2')))
    if leafktx:
        dump(out/'leaf-ktx.json',leafktx)
        subprocess.run(['node',str(HERE/'decode_basis.cjs'),str(out/'leaf-ktx.json')],check=True)
    for f,t in sorted(tree.items(),key=lambda x:-x[1]['depth']):
        if t['kind']=='leaf':continue
        ch=children[f];meshes=[get_mesh(target/(tree[c]['key']+'.npz')) for c in ch]
        atlas=pack(meshes,[tree[c]['built_images'] for c in ch]);m={k:np.concatenate([v[k] for v in meshes]) for k in meshes[0]}
        # Controls timed separately below; main build includes only the proposed method.
        simple,nlock=simplify(m);image=None
        if atlas:
            image=target/(t['key']+'.png');atlas.save(image);t['built_images']=[str(image)]
        else:t['built_images']=[]
        save_mesh(target/(t['key']+'.npz'),simple);write_glb(target/(t['key']+'.glb'),simple,image)
        t['candidate_triangles']=len(simple['p']);t['input_triangles']=len(m['p']);t['locked_vertices']=nlock
        print(name,'depth',t['depth'],'tile',t['key'],'tris',len(m['p']),'->',len(simple['p']),flush=True)
    end=resource.getrusage(resource.RUSAGE_SELF);child_end=resource.getrusage(resource.RUSAGE_CHILDREN);ns=len(get_mesh(out/'source.npz')['p'])
    child_cpu=child_end.ru_utime+child_end.ru_stime-child_start.ru_utime-child_start.ru_stime
    r=dict(cpu_s=end.ru_utime+end.ru_stime-start.ru_utime-start.ru_stime+child_cpu,leaf_mesh_decode_cpu_s=leaf_mesh_decode_cpu,child_texture_decode_cpu_s=child_cpu,wall_s=time.monotonic()-tm,peak_rss_bytes=max(end.ru_maxrss,child_end.ru_maxrss)*1024,peak_scratch_bytes=scratch_bytes(out),scratch_scope='per dataset prep+build; excludes venv, other dataset, metric output; fresh output grows monotonically',source_triangles=ns,codec='PNG lossless, uncompressed FLOAT geometry; baseline KTX2+meshopt; byte comparison is codec-confounded')
    r['cpu_s_per_mtri']=r['cpu_s']/(ns/1e6);dump(out/'build.json',r);dump(out/'built-tree.json',tree)
    print('BUILD',name,r,flush=True)

def controls(name):
    out=SCR/name;target=out/'output';tree=json.loads((out/'built-tree.json').read_text());children={}
    for f,t in tree.items():children.setdefault(t['parent'],[]).append(f)
    # Paired ablations on exactly the same merged child geometry, separate process.
    for f,t in tree.items():
        if t['kind']=='leaf':continue
        meshes=[get_mesh(target/(tree[c]['key']+'.npz')) for c in children[f]];pack(meshes,[tree[c]['built_images'] for c in children[f]]);m={k:np.concatenate([v[k] for v in meshes]) for k in meshes[0]}
        for mode in ('geometry','uv'):
            sm,_=simplify(m,mode);t[mode+'_control_triangles']=len(sm['p'])
    dump(out/'built-tree.json',tree)

if __name__=='__main__':
    a=argparse.ArgumentParser();a.add_argument('phase',choices=['prep','build','controls','metrics','cracks']);a.add_argument('name',choices=['tex3','crop3']);args=a.parse_args()
    if args.phase=='prep':prep(args.name)
    elif args.phase=='build':build(args.name)
    elif args.phase=='controls':controls(args.name)
    elif args.phase=='cracks':
        from metrics import cracks_only
        cracks_only(args.name)
    else:
        from metrics import metrics
        metrics(args.name)
