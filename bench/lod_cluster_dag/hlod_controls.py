#!/usr/bin/env python3
"""Analytic controls for interior seam release, outer edges and chart raster."""
import ctypes as C
import json
from pathlib import Path
import sys
import numpy as np
from build import Meshopt,ptr

lib,chartlib,output=sys.argv[1:];mo=Meshopt(lib)
x,y=np.meshgrid(np.linspace(0,1,21),np.linspace(0,1,21));pos=np.column_stack([x.ravel(),y.ravel(),np.zeros(x.size)]).astype(np.float32)
idx=[]
for j in range(20):
    for i in range(20):
        a=j*21+i;idx.extend([[a,a+1,a+22],[a,a+22,a+21]])
idx=np.asarray(idx,np.uint32)
# The old two-child interface is x=.5. Only the parent's square perimeter is locked.
lock=np.ascontiguousarray((pos[:,:2]==0).any(1)|(pos[:,:2]==1).any(1),np.uint8)
flat=idx.ravel();dest=np.empty_like(flat);error=C.c_float()
n=mo.lib.meshopt_simplifyWithAttributes(ptr(dest),ptr(flat),len(flat),ptr(pos),len(pos),12,None,0,None,0,ptr(lock),80*3,1e6,1|2|4,C.byref(error))
f=dest[:n].reshape(-1,3)
def edges(f):return set(map(tuple,np.sort(np.concatenate([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]]),axis=1)))
border={e for e in edges(idx) if lock[list(e)].all() and np.linalg.norm(pos[e[0]]-pos[e[1]])<.051}
assert border<=edges(f)
interior=np.flatnonzero((pos[:,0]==.5)&~lock.astype(bool));removed=len(set(interior)-set(f.ravel()))
assert removed>0 and len(f)<len(idx)//2
chart=C.CDLL(chartlib);chart.unwrap.argtypes=[C.c_void_p,C.c_void_p,C.c_int,C.c_int,C.c_void_p,C.c_void_p,C.c_void_p]
size=64;uv=np.zeros((len(f),3,2),np.float32);fi=np.empty(size*size,np.int32);bc=np.empty((size*size,3),np.float32)
nc=chart.unwrap(ptr(pos),ptr(f),len(f),size,ptr(uv),ptr(fi),ptr(bc));assert nc>0
mask=fi>=0;assert mask.any();assert np.all(uv>=0) and np.all(uv<=1)
yy,xx=np.divmod(np.flatnonzero(mask),size);reconstructed=(uv[fi[mask]]*bc[mask,:,None]).sum(1)*size
err=float(np.abs(reconstructed-np.column_stack([xx+.5,yy+.5])).max());assert err<1e-4
# Sensitive negative control: removing a triangle carrying a perimeter segment
# must make the exact interface set check fail.
mutant=None
for i,t in enumerate(f):
    if any(tuple(sorted((int(t[k]),int(t[(k+1)%3])))) in border for k in range(3)):
        mutant=np.delete(f,i,axis=0);break
assert mutant is not None and not border<=edges(mutant)
result=dict(input_triangles=len(idx),coarse_triangles=len(f),released_child_interface_vertices=removed,
            outer_segments_retained=len(border),missing_segment_mutant_rejected=True,charts=nc,
            chart_raster_inverse_error_pixels=err)
Path(output).write_text(json.dumps(result,indent=2));print(result)
