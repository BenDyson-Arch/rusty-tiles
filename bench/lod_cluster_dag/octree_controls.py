#!/usr/bin/env python3
"""Small independent controls for interval ownership and GE-density packing."""
import ctypes as C,json,sys
from pathlib import Path
import numpy as np
from build import ptr

lib,out=sys.argv[1:];rng=np.random.default_rng(12);probes=0
# Compare sparse boundary union/min-max rule with a brute set oracle for >64 leaves,
# including vertices shared by nonconsecutive leaves and nonmanifold edge groups.
for leaves in [3,64,65,156,1959]:
    owners=[set(map(int,rng.integers(leaves,size=int(rng.integers(1,8))))) for _ in range(100)]
    low=np.array([min(x) for x in owners]);high=np.array([max(x) for x in owners])
    for _ in range(30):
        a,b=sorted(map(int,rng.integers(leaves+1,size=2)));region=set(range(a,b))
        incident=np.array([bool(region&x) for x in owners]);got=incident&((low<a)|(high>=b))
        wanted=np.array([bool(x&region) and bool(x-region) for x in owners]);assert np.array_equal(got,wanted);probes+=1
pos=np.array([[0,0,0],[1,0,0],[1,1,0],[0,1,0]],np.float32);idx=np.array([[0,1,2],[0,2,3]],np.uint32)
fn=C.CDLL(lib).unwrap_density;fn.argtypes=[C.c_void_p,C.c_void_p,C.c_int,C.c_int,C.c_void_p,C.c_void_p,C.c_void_p,C.c_double,C.c_void_p,C.c_void_p]
rows=[]
for spacing in [.1,.002,.0001]:
    cap=1024;uv=np.empty((2,3,2),np.float32);face=np.empty(cap*cap,np.int32);bc=np.empty((cap*cap,3),np.float32);size=C.c_int();actual=C.c_double()
    charts=fn(ptr(pos),ptr(idx),2,cap,ptr(uv),ptr(face),ptr(bc),spacing,C.byref(size),C.byref(actual));assert charts>0
    n=size.value;f=face[:n*n];b=bc[:n*n];mask=f>=0;yy,xx=np.divmod(np.flatnonzero(mask),n)
    reconstructed=(uv[f[mask]]*b[mask,:,None]).sum(1)*n
    err=float(np.abs(reconstructed-np.column_stack([xx+.5,yy+.5])).max());assert err<1e-3
    assert np.all((uv>=0)&(uv<=1));assert actual.value>=spacing*(1-1e-10)
    rows.append(dict(spacing=spacing,size=n,actual=actual.value,raster_inverse_error_pixels=err))
assert rows[0]['size']==128 and rows[1]['size']==512 and rows[2]['actual']>rows[2]['spacing']
record=dict(interval_boundary_probes=probes,density=rows);Path(out).write_text(json.dumps(record,indent=2));print(record)
