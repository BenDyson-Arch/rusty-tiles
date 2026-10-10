#!/usr/bin/env python3
"""Independent design arithmetic only: imports no production Rust/test code."""
from decimal import Decimal as D, getcontext
import math, json, struct, hashlib, subprocess
from pathlib import Path
getcontext().prec=65
PI=D('3.141592653589793238462643383279502884197169399375105820974944592307816')
A=D('6378137'); F=1/D('298.257223563'); E2=F*(2-F)
def dsin(x):
    term=x; total=x; n=1
    while abs(term)>D('1e-68'):
        term *= -x*x / ((2*n)*(2*n+1)); total+=term; n+=1
    return total
def dcos(x): return dsin(PI/2-x)
def decimal_origin(lon,lat,h):
    lon=D(str(lon))*PI/180; lat=D(str(lat))*PI/180; h=D(str(h))
    s,c=dsin(lat),dcos(lat); n=A/(1-E2*s*s).sqrt()
    return [(n+h)*c*dcos(lon),(n+h)*c*dsin(lon),(n*(1-E2)+h)*s]
def frame(lon,lat,h):
    lon,lat=math.radians(lon),math.radians(lat); sl,cl=math.sin(lon),math.cos(lon); sp,cp=math.sin(lat),math.cos(lat)
    e2=float(F*(2-F)); n=float(A)/math.sqrt(1-e2*sp*sp)
    return [[-sl,-sp*cl,cp*cl],[cl,-sp*sl,cp*sl],[0,cp,sp]],[(n+h)*cp*cl,(n+h)*cp*sl,(n*(1-e2)+h)*sp]
def mv(m,v): return [sum(x*y for x,y in zip(row,v)) for row in m]
def add(a,b): return [x+y for x,y in zip(a,b)]
def cross(a,b): return [a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]
def quat_rotate(q,v):
    # Quaternion vector identity, distinct from the conventional matrix encoding.
    xyz=q[:3]; t=[2*x for x in cross(xyz,v)]
    return add(v,add([q[3]*x for x in t],cross(xyz,t)))
def y_to_z(v): return [v[0],-v[2],v[1]]
def f32(v): return struct.unpack('<f',struct.pack('<f',v))[0]
anchors=[(0,0,0),(90,0,0),(-180,0,125),(180,0,125),(153,-27,42),(23,52,-500),(37,90,0),(-74,-90,25),(179.999999,89.999999,35786000),(-179.999999,-89.999999,-1000000),(123,45,100000000)]
qs=[[0,0,0,1],[0,0,math.sqrt(.5),math.sqrt(.5)],[.2,-.3,.4,math.sqrt(.71)]]
results=[]
for lon,lat,h in anchors:
    basis,origin=frame(lon,lat,h); exact=decimal_origin(lon,lat,h)
    result={'anchor':[lon,lat,h],'origin':origin,'ecef_vs_decimal_max_m':max(abs(D(x)-y) for x,y in zip(origin,exact)), 'basis':basis}
    result['ecef_vs_decimal_max_m']=float(result['ecef_vs_decimal_max_m'])
    result['orthonormal_max']=max(abs(sum(basis[k][r]*basis[k][c] for k in range(3))-(r==c)) for r in range(3) for c in range(3))
    result['placed_axes']=[add(origin,mv(basis,y_to_z(v))) for v in ([1,0,0],[0,1,0],[0,0,-1])]
    result['height_delta_10m']=mv(basis,[0,0,10])
    results.append(result)
# Hand-solvable cardinal frame axes and active right-handed ENU z rotation.
assert frame(0,0,0)[0]==[[0.0,0.0,1.0],[1.0,0.0,0.0],[0,1.0,0.0]]
assert max(abs(x-y) for x,y in zip(quat_rotate(qs[1],[1,0,0]),[0,1,0]))<1e-15
# Root representation rounding versus direct Earth-f32 bake.
point=[.125,.25,-.375]; basis,origin=frame(153,-27,42); world=add(origin,mv(basis,y_to_z(point)))
precision={'small_local_point':point,'world':world,'ecef_f32_bake_max_m':max(abs(f32(x)-x) for x in world),'earth_f32_spacing_m':struct.unpack('<f',struct.pack('<I',struct.unpack('<I',struct.pack('<f',6378137.0))[0]+1))[0]-6378137.0,'local_f32_rounding_worst_component_at_limit_m':.03125,'local_f32_rounding_worst_euclidean_at_limit_m':math.sqrt(3)*.03125}
# Independent box proof for emitted stored f32 points. Boxes only tested locally;
# injectivity of any nonsingular affine placement preserves enclosure exactly.
points=[[-1e-38,0,0],[1e6,0,0],[1,0,0],[-1e6,-1e6,-1e6],[1e6,1e6,1e6]]
box_probes=[]
for points_case in (points[:3],points[3:],[[0,0,0]],[[.125,.25,-.375],[.125,.25,-.375]]):
    values=[y_to_z([f32(x) for x in p]) for p in points_case]
    lo=[min(p[i] for p in values) for i in range(3)]; hi=[max(p[i] for p in values) for i in range(3)]
    center=[(l+h)/2 for l,h in zip(lo,hi)]
    half=[0 if l==h else math.nextafter(max(c-l,h-c),math.inf) for l,h,c in zip(lo,hi,center)]
    contains=all(c-h<=x<=c+h for p in values for x,c,h in zip(p,center,half)); assert contains
    box_probes.append({'stored_points':values,'center':center,'half':half,'exact_local_enclosure':contains})
# Frozen independent Cesium reference, separate from proposed contract. Pole
# conventions may intentionally differ and must be visible, not normalized away.
node=Path('/home/bend/.cache/rusty-tiles-f1c1-spatial-probes/reference.mjs')
node.write_text('import {Cartesian3, Matrix4, Transforms} from "/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/@cesium/engine/index.js";\nconst cases='+json.dumps(anchors)+'; console.log(JSON.stringify(cases.map(([lon,lat,h])=>({anchor:[lon,lat,h],origin:Cartesian3.fromDegrees(lon,lat,h),matrix:Array.from(Transforms.eastNorthUpToFixedFrame(Cartesian3.fromDegrees(lon,lat,h)))}))));\n')
run=subprocess.run(['node',str(node)],text=True,capture_output=True,check=True)
cesium=json.loads(run.stdout)
for own,other in zip(results,cesium):
    matrix=other['matrix']; own['cesium_origin_max_m']=max(abs(own['origin'][i]-matrix[12+i]) for i in range(3))
    own['cesium_basis_max']=max(abs(own['basis'][r][c]-matrix[c*4+r]) for r in range(3) for c in range(3))
# Numeric-budget boundary: high height, large scene offset and local corners.
# Decimal reference keeps exact f64 quaternion inputs, independently normalizes,
# and rotates via vector identity rather than matrix multiplication.
precision_bound = 2**26
q=qs[2]; qnorm=math.sqrt(sum(v*v for v in q)); q=[v/qnorm for v in q]
dq=[D(v) for v in q]; dnorm=sum(v*v for v in dq).sqrt(); dq=[v/dnorm for v in dq]
columns=[quat_rotate(q,v) for v in ([1,0,0],[0,1,0],[0,0,1])]
rot=[[columns[c][r] for c in range(3)] for r in range(3)]
o=[4000000.125,-2000000.25,1000000.375]
h=precision_bound-float(A)-sum(abs(v) for v in o)-math.sqrt(3)*1e6
E,C=frame(123,45,h); ER=[[sum(E[r][k]*rot[k][c] for k in range(3)) for c in range(3)] for r in range(3)]
root_t=add(C,mv(ER,y_to_z(o)))
dlon=D('123')*PI/180; dlat=D('45')*PI/180
Es=[[-dsin(dlon),-dsin(dlat)*dcos(dlon),dcos(dlat)*dcos(dlon)], [dcos(dlon),-dsin(dlat)*dsin(dlon),dcos(dlat)*dsin(dlon)], [D(0),dcos(dlat),dsin(dlat)]]
# Decimal height uses exact binary64 h because the request arrives as binary64.
dC=decimal_origin(123,45,D(h))
# decimal_origin parses D via str, retaining exact D(h) expansion.
world_errors=[]
for p in ([1e6,-1e6,1e6],[-1e6,1e6,-1e6],[.125,.25,-.375]):
    world=add(root_t,mv(ER,y_to_z(p)))
    dp=[D(x)+D(y) for x,y in zip(p,o)]
    exact=add(dC,mv(Es,quat_rotate(dq,y_to_z(dp))))
    world_errors.append(max(float(abs(D(x)-y)) for x,y in zip(world,exact)))
precision['proposed_budget_m']=precision_bound
precision['proposed_budget_case']={'anchor':[123,45,h],'offset':o,'quaternion':q,'world_max_error_vs_decimal_m':max(world_errors),'loose_64u_budget_m':64*2**-53*precision_bound}
precision['offset_order_control']={'quaternion':qs[1],'offset_yup':[1,2,3],'correct_world_offset_at_equator':mv(frame(0,0,0)[0],quat_rotate(qs[1],y_to_z([1,2,3]))),'wrong_unrotated_offset':mv(frame(0,0,0)[0],y_to_z([1,2,3]))}
assert max(world_errors)<1e-6
output={'provenance' :{'python':'decimal precision 65; independent decimal Taylor sine; quaternion vector identity; no repo imports','cesium_engine_version':'26.4.0','reference_entry_sha256':hashlib.sha256(Path('/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/@cesium/core/Source/FixedFrameTransforms.js').read_bytes()).hexdigest()},'scope':'Design arithmetic only, not candidate implementation, browser acceptance, or Cargo evidence','anchors':results,'precision':precision,'boxes':box_probes,'cesium_frozen':cesium}
Path('/home/bend/.cache/rusty-tiles-f1c1-spatial-probes/results.json').write_text(json.dumps(output,indent=2)+'\n')
print(json.dumps({'decimal_origin_max_m':max(r['ecef_vs_decimal_max_m'] for r in results),'orthonormal_max':max(r['orthonormal_max'] for r in results),'cesium_origin_max_m':max(r['cesium_origin_max_m'] for r in results),'cesium_basis_max':max(r['cesium_basis_max'] for r in results),'precision':precision,'local_box_cases':len(box_probes)},indent=2))
