#!/usr/bin/env python3
"""Independent root-only comparison with the existing full-model baseline."""
import json,sys
from pathlib import Path
import numpy as np
from evaluate import Surface,stats,lab
source,decoded,output=sys.argv[1:]
a=Surface().add(source,True).finish();b=Surface().add(decoded).finish();rng=np.random.default_rng(20261024);rows={};distances=[];deltas=[]
for label,x,y in [('source_to_lod',a,b),('lod_to_source',b,a)]:
 q,fi,bc=x.sample(200000,rng);d,near,nbc=y.closest(q);assert np.isfinite(d).all() and np.isfinite(nbc).all()
 cx=x.rgb(fi,bc);cy=y.rgb(near,nbc);de=np.linalg.norm(lab(cx)-lab(cy),axis=1)
 srgb=lambda c:np.where(c<=.0031308,c*12.92,1.055*np.maximum(c,0)**(1/2.4)-.055)
 mse=np.mean((srgb(np.clip(cx,0,1))-srgb(np.clip(cy,0,1)))**2)
 rows[label]=dict(samples=len(q),geometry_m=stats(d),delta_e76=stats(de),psnr_srgb_db=float(-10*np.log10(max(mse,1e-30))))
 distances.append(d);deltas.append(de)
r=dict(triangles=len(b.xyz),geometry_m=stats(np.concatenate(distances)),delta_e76=stats(np.concatenate(deltas)),directions=rows)
Path(output).write_text(json.dumps(r,indent=2));print(json.dumps(r))
