import json, sys, base64, subprocess, numpy as np, os
TOOLS=os.path.dirname(os.path.abspath(__file__))
path=sys.argv[1]
rec=json.loads(subprocess.run(['node',os.path.join(TOOLS,'decode_many.cjs')],input=path,capture_output=True,text=True,check=True).stdout)
doc=rec['document']
def acc(i):
    a=doc['accessors'][i]; raw=np.frombuffer(base64.b64decode(rec['views'][a['bufferView']]),dtype=np.float32)
    w={'VEC2':2,'VEC3':3,'SCALAR':1}[a['type']]; return raw.reshape(-1,w)
for m in doc['meshes']:
  p=m['primitives'][0]; uv=acc(p['attributes']['TEXCOORD_0']).astype(np.float64)
  tex=doc['textures'][doc['materials'][p['material']]['pbrMetallicRoughness']['baseColorTexture']['index']]
  print('tris',len(uv)//3)
  T=uv.reshape(-1,3,2)
  def clip(poly,a,b):
    out=[];n=len(poly)
    for i in range(n):
      p,q=poly[i],poly[(i+1)%n]
      sp=(b[0]-a[0])*(p[1]-a[1])-(b[1]-a[1])*(p[0]-a[0]); sq=(b[0]-a[0])*(q[1]-a[1])-(b[1]-a[1])*(q[0]-a[0])
      if sp>=0: out.append(p)
      if (sp>=0)!=(sq>=0):
        t=sp/(sp-sq); out.append(p+(q-p)*t)
    return out
  def area(poly):
    return 0.5*abs(sum(poly[i][0]*poly[(i+1)%len(poly)][1]-poly[(i+1)%len(poly)][0]*poly[i][1] for i in range(len(poly)))) if len(poly)>2 else 0
  def orient(t): return (t[1][0]-t[0][0])*(t[2][1]-t[0][1])-(t[1][1]-t[0][1])*(t[2][0]-t[0][0])
  lo=T.min(1);hi=T.max(1); order=np.argsort(lo[:,0]); act=[]; res=[]
  for i in order:
    act=[j for j in act if hi[j,0]>lo[i,0]]
    for j in act:
      if hi[j,1]<=lo[i,1] or hi[i,1]<=lo[j,1]: continue
      a=T[i] if orient(T[i])>0 else T[i][::-1]; b=T[j] if orient(T[j])>0 else T[j][::-1]
      poly=list(a)
      for k in range(3):
        poly=clip(poly,b[k],b[(k+1)%3])
        if not poly: break
      ar=area(poly)
      res.append((ar, area(list(a)), area(list(b)), i, j))
    act.append(i)
  res.sort(reverse=True)
  W=4096
  print('candidate pairs',len(res),'pairs with area>0',sum(1 for r in res if r[0]>0))
  for r in res[:8]: print('overlap uv^2 %.3e tri areas %.3e %.3e (in 4096^2 texels: %.4f)'%(r[0],r[1],r[2],r[0]*W*W), r[3], r[4], T[r[3]].tolist(), T[r[4]].tolist())
