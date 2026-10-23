"""Recover source triangle identity for exact legacy leaves; refuse non-exact matches."""
import pathlib,json,numpy as np,sys
from scipy.spatial import cKDTree
def canonical_vertices(p):
    ix=np.lexsort((p[:,:,2],p[:,:,1],p[:,:,0]),axis=1)
    return np.take_along_axis(p,ix[:,:,None],axis=1)

def main():
    S=pathlib.Path('/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/astra-keep-h17');w=S/'legacy-anchor-tex3';D=w/'decoded';sd=S/'source-decoded'
    src=np.fromfile(sd/'0.tri.f64',np.float64).reshape(-1,27)[:,:9].reshape(-1,3,3);sr=json.load(open(sd/'index.json'))[0]
    keys=np.zeros((len(src),6),np.uint32)
    for g in sr['surfaces']:
     sl=slice(g['first'],g['first']+g['count']);keys[sl,0]=g['mesh'];keys[sl,2]=np.arange(g['count'])
    kt=cKDTree(src.mean(1));rows=json.load(open(D/'index.json'));tree=json.load(open(D/'tree.json'));parents={v['parent'] for v in tree.values()};tiles=[];out={};maxerr=0.;seen=[]
    for r in rows:
     kind='proxy' if r['file'] in parents else 'leaf';a=np.fromfile(D/f"{r['id']}.tri.f64",np.float64).reshape(-1,27)[:,:9].copy().reshape(-1,3,3)
     if kind=='leaf':
      dist,idx=kt.query(a.mean(1));err=np.max(np.abs(canonical_vertices(src[idx])-canonical_vertices(a)));maxerr=max(maxerr,float(err));assert err==0.,err;seen.extend(idx.tolist());k=keys[idx];a=src[idx].copy() # reorder identical decoded corners for source-corner census
     else:k=np.zeros((len(a),6),np.uint32)
     if kind not in out:out[kind]=[open(str(w/'seams')+f'.{kind}.{suffix}','wb') for suffix in ['pos.f64','tile.i32','src.u32']]
     a.tofile(out[kind][0]);np.full(len(a),r['id'],np.int32).tofile(out[kind][1]);k.tofile(out[kind][2]);tiles.append(dict(id=r['id'],file=r['file'],kind=kind,triangles=len(a)))
    assert len(set(seen))==len(src)==len(seen)
    for handles in out.values():
     for f in handles:f.close()
    (w/'seams.tiles.json').write_text(json.dumps(tiles));print({'leaf_source_triangle_matches':len(seen),'max_corner_difference':maxerr})

if __name__ == "__main__":
    main()
