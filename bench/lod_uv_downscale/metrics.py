"""Independent decoded-output measurements; Open3D BVH supplies exact nearest triangles."""
import os,json,math,hashlib,itertools,gc
from pathlib import Path
import numpy as np
import open3d as o3d
from run import SCR,EVID,BASE,INPUT,read_mesh,get_mesh,dump,image_read

N=200_000

def scene(p):
    # Translate before float32 conversion: metric precision stays below a few micrometres.
    origin=np.floor(p.reshape(-1,3).min(0)/64)*64
    vertices=np.ascontiguousarray((p-origin).reshape(-1,3),np.float32)
    sc=o3d.t.geometry.RaycastingScene(nthreads=2)
    sc.add_triangles(o3d.core.Tensor(vertices),o3d.core.Tensor(np.arange(len(vertices),dtype=np.uint32).reshape(-1,3)))
    return sc,origin

def nearest(sc,origin,q):
    r=sc.compute_closest_points(o3d.core.Tensor(np.ascontiguousarray(q-origin,np.float32)),nthreads=2)
    pt=r['points'].numpy().astype(float)+origin;uv=r['primitive_uvs'].numpy().astype(float)
    return np.linalg.norm(pt-q,axis=1),r['primitive_ids'].numpy().astype(int),np.column_stack([1-uv.sum(1),uv])

def project(sc,origin,q):
    r=sc.compute_closest_points(o3d.core.Tensor(np.ascontiguousarray(q-origin,np.float32)),nthreads=2)
    return r['points'].numpy().astype(float)+origin

def border_scene(p):
    _,w=np.unique(p.reshape(-1,3),axis=0,return_inverse=True);w=w.reshape(-1,3)
    edges=np.sort(np.column_stack([w.ravel(),w[:,[1,2,0]].ravel()]),axis=1)
    _,ix,count=np.unique(edges,axis=0,return_index=True,return_counts=True);ix=ix[count==1]
    a=p.reshape(-1,3)[ix];b=p[:,[1,2,0]].reshape(-1,3)[ix]
    assert len(a), 'tile without an open border at a source seam'
    # Degenerate triangles represent exact line segments to the BVH closest-point query.
    return scene(np.stack([a,b,b],axis=1))

def samples(m,rng,n=N):
    p=m['p'];area=np.linalg.norm(np.cross(p[:,1]-p[:,0],p[:,2]-p[:,0]),axis=1)
    ix=rng.choice(len(p),n,p=area/area.sum());u=rng.random(n);v=rng.random(n);su=np.sqrt(u);b=np.column_stack([1-su,su*(1-v),su*v])
    return np.einsum('ni,nij->nj',b,p[ix]),ix,b

def srgb(linear):return np.where(linear<=.0031308,12.92*linear,1.055*np.maximum(linear,0)**(1/2.4)-.055)
def linear(s):return np.where(s<=.04045,s/12.92,((s+.055)/1.055)**2.4)
def lab(rgb):
    xyz=linear(np.clip(rgb,0,1))@np.array([[.4124564,.2126729,.0193339],[.3575761,.7151522,.119192],[.1804375,.0721750,.9503041]])
    xyz/=np.array([.95047,1,1.08883]);f=np.where(xyz>(6/29)**3,np.cbrt(xyz),xyz/(3*(6/29)**2)+4/29)
    return np.column_stack([116*f[:,1]-16,500*(f[:,0]-f[:,1]),200*(f[:,1]-f[:,2])])

def colour(m,images,ix,b):
    co=np.einsum('ni,nij->nj',b,m['c'][ix]);result=srgb(co);uv=np.einsum('ni,nij->nj',b,m['uv'][ix]);ii=m['im'][ix]
    for im in np.unique(ii):
        if im<0:continue
        mask=ii==im;a=linear(np.array(image_read(images[im]),dtype=np.float32)/255);h,w=a.shape[:2];xy=uv[mask]*[w,h]-.5
        x=np.clip(xy[:,0],0,w-1);y=np.clip(xy[:,1],0,h-1);x0=x.astype(int);y0=y.astype(int);x1=np.minimum(x0+1,w-1);y1=np.minimum(y0+1,h-1);fx=(x-x0)[:,None];fy=(y-y0)[:,None]
        c=(a[y0,x0]*(1-fx)+a[y0,x1]*fx)*(1-fy)+(a[y1,x0]*(1-fx)+a[y1,x1]*fx)*fy
        result[mask]=srgb(c*co[mask])
    return np.clip(result,0,1)

def merge(files,meshes,ims):
    out=[];images=[]
    for f in files:
        m={k:v.copy() for k,v in meshes[f].items()};mask=m['im']>=0;m['im'][mask]+=len(images);images+=ims[f];out.append(m)
    return {k:np.concatenate([x[k] for x in out]) for k in out[0]},images

def stats(d):return dict(mean=float(np.mean(d)),p95=float(np.quantile(d,.95)),p99=float(np.quantile(d,.99)),max=float(np.max(d)))

def metrics(name):
    root=SCR/name;target=root/'output';tree=json.loads((root/'built-tree.json').read_text());source=get_mesh(root/'source.npz');si=json.loads((root/'source-images.json').read_text());dec=root/'decoded';dec.mkdir(exist_ok=True)
    children={}
    for f,t in tree.items():children.setdefault(t['parent'],[]).append(f)
    leaves=[f for f,t in tree.items() if t['kind']=='leaf'];proxies=[f for f,t in tree.items() if t['kind']=='proxy'];roots=children[None]
    candidate={};ci={};baseline={};bi={};roundtrip=[]
    for f,t in tree.items():
        m,im,k=read_mesh(target/(t['key']+'.glb'),dec);candidate[f]=m;ci[f]=im
        original=get_mesh(target/(t['key']+'.npz'))
        # Writer groups by primitive. Compare sorted triangle records including identity.
        def digest(m):
            a=np.column_stack([m['pid'],m['p'].reshape(-1,9)]);return a[np.lexsort(a.T[::-1])]
        assert np.array_equal(digest(m),digest(original)),('decoded geometry mismatch',f)
        if name=='tex3':baseline[f]=get_mesh(root/(t['key']+'.npz'));bi[f]=t['images']
        else:baseline[f]=m;bi[f]=im
        roundtrip.append(f)
    # All candidate leaf KTX2 images were already decoded during preparation.
    for f in leaves:
        if name=='tex3':ci[f]=tree[f]['images']
    srcscene,so=scene(source['p']);rng=np.random.default_rng(20261024);q,qi,qb=samples(source,rng);sourcecol=colour(source,si,qi,qb)
    maxdepth=max(t['depth'] for t in tree.values());rows=[]
    def frontier(d):
        stack=list(roots);out=[]
        while stack:
            f=stack.pop()
            if tree[f]['depth']>=d or tree[f]['kind']=='leaf':out.append(f)
            else:stack+=children[f]
        return out
    for variant,meshes,images in [('candidate',candidate,ci)]+([('baseline',baseline,bi)] if name=='tex3' else []):
        for depth in range(maxdepth+1):
            fs=frontier(depth);m,ims=merge(fs,meshes,images);sc,org=scene(m['p']);forward,ix,bary=nearest(sc,org,q);cc=colour(m,ims,ix,bary)
            qq,ii,bb=samples(m,rng);reverse,si2,sb=nearest(srcscene,so,qq)
            err=np.r_[forward,reverse];delta=np.linalg.norm(lab(cc)-lab(sourcecol),axis=1);mse=np.mean((cc-sourcecol)**2)
            ps=[f for f in fs if tree[f]['kind']=='proxy'];own=[f for f in proxies if tree[f]['depth']==depth]
            glb_bytes=sum((target/(tree[f]['key']+'.glb')).stat().st_size for f in own) if variant=='candidate' else sum(tree[f]['bytes'] for f in own)
            leafbytes=sum((target/(tree[f]['key']+'.glb')).stat().st_size if variant=='candidate' else tree[f]['bytes'] for f in fs if tree[f]['kind']=='leaf')
            row=dict(variant=variant,depth=depth,frontier_tiles=len(fs),proxy_tiles=len(own),proxy_triangles=sum(len(meshes[f]['p']) for f in own),frontier_triangles=len(m['p']),proxy_glb_bytes=glb_bytes,frontier_glb_bytes=leafbytes+sum((target/(tree[f]['key']+'.glb')).stat().st_size if variant=='candidate' else tree[f]['bytes'] for f in ps),geometry_two_sided_m=stats(err),source_to_proxy_m=stats(forward),proxy_to_source_m=stats(reverse),mean_deltaE76=float(delta.mean()),p95_deltaE76=float(np.quantile(delta,.95)),psnr_db=float(-10*np.log10(max(mse,1e-30))),samples_each_direction=N)
            own_all=[f for f,t in tree.items() if t['depth']==depth]
            row.update(level_tiles=len(own_all),level_triangles=sum(len(meshes[f]['p']) for f in own_all),level_glb_bytes=sum((target/(tree[f]['key']+'.glb')).stat().st_size if variant=='candidate' else tree[f]['bytes'] for f in own_all))
            if variant=='candidate':
                row['input_triangles']=sum(tree[f].get('input_triangles',0) for f in own);row['geometry_control_triangles']=sum(tree[f].get('geometry_control_triangles',0) for f in own);row['uv_control_triangles']=sum(tree[f].get('uv_control_triangles',0) for f in own)
            rows.append(row);print(name,variant,depth,row,flush=True);del sc,m;gc.collect()
    dump(EVID/(name+'-levels.json'),rows)
    assign_ge(tree,rows)
    dump(root/'built-tree.json',tree)
    crackrows=cracks(name,tree,children,source,candidate,baseline,leaves,proxies)
    # No tileset is built: output is standalone GLBs indexed by source tree JSON.
    manifest={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(target.glob('*.glb'))}
    dump(EVID/(name+'-artifact-sha256.json'),manifest);dump(EVID/(name+'-build.json'),json.loads((root/'build.json').read_text()))
    dump(EVID/(name+'-tree.json'),tree);dump(EVID/(name+'-decoded-check.json'),{'tiles':len(roundtrip),'geometry_and_primitive_ids_exact':True,'leaf_bytes_unchanged':name=='tex3','final_shared_scratch_occupancy_bytes':sum(p.stat().st_size for p in SCR.rglob('*') if p.is_file())})

def cracks(name,tree,children,source,candidate,baseline,leaves,proxies):
    """Every source-edge adjacent tile pair, all depth combinations, decoded geometry.

    Query both sides at shared leaf edge endpoints and midpoint. For candidate, also
    demand the WHOLE shared edge exists exactly as an output triangle edge in both tiles.
    This strengthens zero-distance samples into continuous edge agreement.
    """
    src=source['p'];_,w=np.unique(src.reshape(-1,3),axis=0,return_inverse=True);w=w.reshape(-1,3);nt=len(src)
    a=w.ravel();b=w[:,[1,2,0]].ravel();edge=np.sort(np.column_stack([a,b]),axis=1);_,inv,counts=np.unique(edge,axis=0,return_inverse=True,return_counts=True);order=np.argsort(inv,kind='stable');starts=np.r_[0,np.cumsum(counts)[:-1]]
    source_keys=source['pid']*(1<<32)+source['tid'];keyorder=np.argsort(source_keys);keys=source_keys[keyorder];owner=np.empty(nt,int)
    leafid={f:i for i,f in enumerate(leaves)};leafmeshes={f:candidate[f] for f in leaves}
    for f,m in leafmeshes.items():
        k=m['pid']*(1<<32)+m['tid'];si=keyorder[np.searchsorted(keys,k)];assert np.array_equal(source_keys[si],k);owner[si]=leafid[f]
    pairs=[];elist=[];nonmanifold=0
    for size in np.unique(counts[counts>=2]):
        st=starts[counts==size]
        if size>2:nonmanifold+=len(st)
        incidents=order[st[:,None]+np.arange(size)]
        for i,j in itertools.combinations(range(size),2):
            aa=incidents[:,i];bb=incidents[:,j];ta=aa//3;tb=bb//3;mask=owner[ta]!=owner[tb]
            pairs.append(np.column_stack([owner[ta[mask]],owner[tb[mask]]]));elist.append(aa[mask])
    pairs=np.concatenate(pairs);elist=np.concatenate(elist);print('cross-leaf edges',len(pairs),'nonmanifold',nonmanifold,flush=True)
    # Shared decoded leaf edge, not the source quantization positions.
    source_decoded=np.empty_like(src)
    for f,m in leafmeshes.items():
        si=keyorder[np.searchsorted(keys,m['pid']*(1<<32)+m['tid'])];source_decoded[si]=m['p']
    p=source_decoded.reshape(-1,3)[elist];pp=source_decoded[:,[1,2,0]].reshape(-1,3)[elist]
    ancestor={}
    for f in tree:
        chain=[];x=f
        while x is not None:chain.append(x);x=tree[x]['parent']
        ancestor[f]=chain
    records=[];distributions={}
    # For each coarser tile, collect every adjacent finer tile at every depth.
    for variant,meshes in [('candidate',candidate)]+([('baseline',baseline)] if name=='tex3' else []):
        scene_cache={};edge_cache={};border_cache={}
        def surface(f):
            if f not in scene_cache:scene_cache[f]=scene(meshes[f]['p'])
            return scene_cache[f]
        def border(f):
            if f not in border_cache:border_cache[f]=border_scene(meshes[f]['p'])
            return border_cache[f]
        def edge_set(f):
            if f not in edge_cache:
                t=meshes[f]['p'];a=t.reshape(-1,3);b=t[:,[1,2,0]].reshape(-1,3);s=np.concatenate([np.minimum(a,b),np.maximum(a,b)],axis=1) # replaced below by lexicographic canonicalization
                flip=(a[:,0]>b[:,0])|((a[:,0]==b[:,0])&(a[:,1]>b[:,1]))|((a[:,0]==b[:,0])&(a[:,1]==b[:,1])&(a[:,2]>b[:,2]));aa=np.where(flip[:,None],b,a);bb=np.where(flip[:,None],a,b);s=np.ascontiguousarray(np.concatenate([aa,bb],axis=1));edge_cache[f]=set(s.view('V48').ravel().tolist())
            return edge_cache[f]
        pair_edges={}
        for ei,(la,lb) in enumerate(pairs):
            for fa in ancestor[leaves[la]]:
                for fb in ancestor[leaves[lb]]:
                    # Ancestor and descendant overlap; REPLACE never displays them as
                    # adjacent regions in one frontier. Their internal edges are irrelevant.
                    if fa in ancestor[fb] or fb in ancestor[fa]:continue
                    da,db=tree[fa]['depth'],tree[fb]['depth']
                    key=(fa,fb) if da<db or (da==db and fa<fb) else (fb,fa)
                    pair_edges.setdefault(key,[]).append(ei)
        for (fa,fb),es in pair_edges.items():
            es=np.unique(es);A=p[es];B=pp[es];q=np.concatenate([A,B,(A+B)/2]);da,db=tree[fa]['depth'],tree[fb]['depth'];ge=max(float(tree[fa].get('candidate_ge',tree[fa]['ge']) if variant=='candidate' else tree[fa]['ge']),1e-12)
            if variant=='candidate':
                flip=(A[:,0]>B[:,0])|((A[:,0]==B[:,0])&(A[:,1]>B[:,1]))|((A[:,0]==B[:,0])&(A[:,1]==B[:,1])&(A[:,2]>B[:,2]));aa=np.where(flip[:,None],B,A);bb=np.where(flip[:,None],A,B);keys_edge=np.ascontiguousarray(np.concatenate([aa,bb],axis=1)).view('V48').ravel().tolist();s1=edge_set(fa);s2=edge_set(fb);missing=sum(k not in s1 or k not in s2 for k in keys_edge)
                if missing:
                    d1,*_=nearest(*surface(fa),q);d2,*_=nearest(*surface(fb),q);distance=d1+d2
                else:distance=np.zeros(len(q))
            else:
                # Project source seam anchors onto each DECODED open border, then measure
                # its distance to the other tile's decoded surface in both directions.
                # A common simplified seam may move away from source without opening a gap.
                missing=None;qa=project(*border(fa),q);qb=project(*border(fb),q)
                d1,*_=nearest(*surface(fb),qa);d2,*_=nearest(*surface(fa),qb);distance=np.maximum(d1,d2)
            rec=dict(variant=variant,a=fa,b=fb,depth_a=da,depth_b=db,depth_gap=db-da,edges=len(es),samples=len(q),max_m=float(distance.max()),p99_m=float(np.quantile(distance,.99)),coarse_ge_m=ge,ge_basis='monotone level sampled max (experimental)' if variant=='candidate' else 'published certified GE',max_fraction_ge=float(distance.max()/ge),missing_exact_edges=missing)
            records.append(rec)
            group=(variant,da,db)
            distributions.setdefault(group,[]).append((distance,distance/ge))
        # Aggregate weighted sample p99 by reconstructing pair measurements above: keep scalar
        # maxima and pair p99 separate, never mislabel p99-of-p99 as sample p99.
        print(name,variant,'crack pairs',len(pair_edges),'max',max(r['max_m'] for r in records if r['variant']==variant),flush=True)
    dump(EVID/(name+'-crack-pairs.json'),records)
    summary=[]
    for (variant,da,db),parts in sorted(distributions.items()):
        d=np.concatenate([a for a,b in parts]);frac=np.concatenate([b for a,b in parts]);rs=[r for r in records if r['variant']==variant and r['depth_a']==da and r['depth_b']==db]
        summary.append(dict(variant=variant,depth_a=da,depth_b=db,pairs=len(parts),samples=len(d),max_m=float(d.max()),p99_m=float(np.quantile(d,.99)),max_fraction_ge=float(frac.max()),p99_fraction_ge=float(np.quantile(frac,.99)),missing_exact_edges=sum(r['missing_exact_edges'] or 0 for r in rs)))
    dump(EVID/(name+'-crack-summary.json'),summary)
    return records

def cracks_only(name):
    root=SCR/name;target=root/'output';tree=json.loads((root/'built-tree.json').read_text());dec=root/'decoded';dec.mkdir(exist_ok=True)
    assign_ge(tree,json.loads((EVID/(name+'-levels.json')).read_text()))
    dump(root/'built-tree.json',tree)
    children={};candidate={};baseline={}
    for f,t in tree.items():
        children.setdefault(t['parent'],[]).append(f)
        candidate[f]=read_mesh(target/(t['key']+'.glb'),dec)[0]
        if name=='tex3':baseline[f]=get_mesh(root/(t['key']+'.npz'))
    cracks(name,tree,children,get_mesh(root/'source.npz'),candidate,baseline,[f for f,t in tree.items() if t['kind']=='leaf'],[f for f,t in tree.items() if t['kind']=='proxy'])

def assign_ge(tree,rows):
    ge={};mx=1e-12
    for r in sorted([r for r in rows if r['variant']=='candidate'],key=lambda r:-r['depth']):
        mx=max(mx,r['geometry_two_sided_m']['max']);ge[r['depth']]=mx
    for t in tree.values():t['candidate_ge']=ge[t['depth']]
