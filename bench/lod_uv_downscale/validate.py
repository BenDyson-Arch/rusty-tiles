"""Independent, bounded acceptance controls and crop3 leaf attribute comparison."""
import os
os.environ['OMP_NUM_THREADS']='2';os.environ['OPENBLAS_NUM_THREADS']='2'
import json,hashlib
import numpy as np
from run import INPUT,SCR,EVID,BASE,load_glb,read_mesh,dump,downscale
from metrics import border_scene,project,nearest,scene,colour,srgb
from PIL import Image

# Sensitive geometric controls: a common moved border must have zero gap; separate borders
# must retain their known separation. Degenerate line BVHs must project endpoints correctly.
tri=np.array([[[0.,0.,0.],[1.,0.,0.],[0.,1.,0.]]]);sc,org=border_scene(tri)
q=np.array([[.3,-1,0.],[2.,-1,0.]])
p=project(sc,org,q);assert np.allclose(p,[[.3,0,0],[1,0,0]],atol=2e-7)
a=scene(tri);b=scene(tri+[0,0,.05]);d,_,_=nearest(*b,project(*border_scene(tri),np.array([[.2,0,0.]])))
assert abs(d[0]-.05)<2e-7
same,_,_=nearest(*a,project(*border_scene(tri),np.array([[.2,0,0.]])));assert same.max()<2e-7
image=SCR/'linear-filter-control.png';Image.fromarray(np.array([[[0,0,0],[255,255,255]]],np.uint8)).save(image)
test={'c':np.ones((1,3,3)),'uv':np.full((1,3,2),.5),'im':np.zeros(1,int)}
co=colour(test,[str(image)],np.array([0]),np.array([[1.,0.,0.]]));assert np.allclose(co,srgb(np.array([.5,.5,.5])),atol=2e-7)
assert abs(np.array(downscale(Image.open(image)))[0,0,0]/255-srgb(np.array(.5)))<1/255
tex_tree=json.loads((SCR/'tex3'/'built-tree.json').read_text());leaf_hashes={}
for f,t in tex_tree.items():
 if t['kind']!='leaf':continue
 digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
 h=digest(BASE/f);assert h==digest(SCR/'tex3'/'output'/(t['key']+'.glb'));leaf_hashes[f]=h
dump(EVID/'tex3-leaf-copied-sha256.json',leaf_hashes)

root=SCR/'crop3';tree=json.loads((root/'built-tree.json').read_text());sj,sv,sa=load_glb(INPUT/'crop3.glb');source={}
for mi,mesh in enumerate(sj['meshes']):
 for pi,pr in enumerate(mesh['primitives']):
  ix=sa(pr['indices']).ravel();attrs={k:sa(ai)[ix].reshape(-1,3,sa(ai).shape[1]) for k,ai in pr['attributes'].items()}
  if 'COLOR_0' in attrs:assert np.all(attrs['COLOR_0'][...,3]==1)
  source[mi*1024+pi]=attrs
n=nt=0
for f,t in tree.items():
 if t['kind']!='leaf':continue
 path=root/'output'/(t['key']+'.glb');m,_,_=read_mesh(path,root/'decoded');j,v,acc=load_glb(path)
 for pr in j['meshes'][0]['primitives']:
  row=pr['extensions']['EXT_mesh_features']['featureIds'][0]['constant']
  meta=j['extensions']['EXT_structural_metadata'];pt=meta['propertyTables'][0]
  mi=np.frombuffer(v(pt['properties']['mesh_index']['values']),'<u4')[row];pi=np.frombuffer(v(pt['properties']['primitive_index']['values']),'<u4')[row];pid=int(mi)*1024+int(pi)
  fids=acc(pr['attributes']['_FEATURE_ID_1']).astype(int).ravel()[::3];tt=meta['propertyTables'][1]
  tids=np.frombuffer(v(tt['properties']['triangle_index']['values']),'<u4')[fids]
  for key in ['COLOR_0','NORMAL']:
   if key in source[pid]:
    got=acc(pr['attributes'][key]).reshape(-1,3,source[pid][key].shape[2]);assert np.array_equal(got,source[pid][key][tids]),(f,key)
   elif key=='NORMAL':assert key not in pr['attributes']
   else:assert np.all(acc(pr['attributes'][key])==1) # glTF's default vertex colour
  # Full-detail source triangle ordering, coordinate values and primitive IDs.
  assert np.array_equal(m['p'][m['pid']==pid],source[pid]['POSITION'][tids]),f
  nt+=len(tids)
 n+=1
assert nt==788425
r={'sensitive_controls':'passed: projection endpoints, zero shared border, known 0.05 m gap, linear-light filtering and downscale','tex3_leaf_sha256_equal_count':len(leaf_hashes),'crop3_leaf_tiles':n,'crop3_source_triangles':nt,'positions_normals_rgba_primitive_and_triangle_identity_exact':True,'source_alpha_all_one':True}
dump(EVID/'validation.json',r);print(r)
