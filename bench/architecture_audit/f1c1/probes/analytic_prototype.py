#!/usr/bin/env python3
"""Audit-only independent WGS84 references. No Rust/oracle/converter imports.

Decimal(80) Taylor sin/cos creates frozen references from decimal requests;
float candidate uses math trig, while quaternion truth is rational or Hamilton.
This is a specification prototype, NOT a production acceptance execution.
"""
from decimal import Decimal as D, localcontext
from fractions import Fraction as F
from pathlib import Path
import hashlib, json, math, struct
PI=D('3.141592653589793238462643383279502884197169399375105820974944592307816406286208998628')
A=D('6378137'); INVF=D('298.257223563')
CASES=[('origin','0','0','0'),('east90','90','0','0'),('west90','-90','0','100'),('antimeridian+','180','0','0'),('antimeridian-','-180','0','0'),('north0','0','90','0'),('north137','137','90','100'),('south0','0','-90','0'),('south-79','-79','-90','-100'),('brisbane','153','-27','130'),('oblique-high','37','48','1000000'),('oblique-low','-123','-52','-1000000'),('near-north','179.999999','89.999999','1000'),('near-south','-179.999999','-89.999999','-1000'),('near-antimeridian+','179.999999','23','123'),('near-antimeridian-','-179.999999','23','123'),('far-positive-height','37','48','50000000'),('far-negative-height','-123','-52','-50000000'),('earth-centre-forward','137','0','-6378137'),('near-earth-centre-forward','137','0','-6378136.999999'),('near-pole-centre-forward','79','90','-6356752.314245')]

def sincos_degrees(degrees):
    d=D(degrees)
    # Exact cardinal cases prove a pole meridian convention without libm noise.
    if d%90 == 0:
        return [(D(0),D(1)),(D(1),D(0)),(D(0),D(-1)),(D(-1),D(0))][int(d/90)%4]
    x=d*PI/180; s=x; c=D(1); st=x; ct=D(1)
    for k in range(1,200):
        st *= -x*x/D((2*k)*(2*k+1)); ct *= -x*x/D((2*k-1)*(2*k))
        s+=st; c+=ct
        if abs(st)<D('1e-85') and abs(ct)<D('1e-85'): break
    else: raise AssertionError('Taylor did not converge')
    return s,c

def frame_decimal(lon,lat,height):
    sl,cl=sincos_degrees(lon); sp,cp=sincos_degrees(lat); h=D(height)
    f=1/INVF; e2=f*(2-f); n=A/(1-e2*sp*sp).sqrt()
    origin=((n+h)*cp*cl,(n+h)*cp*sl,(n*(1-e2)+h)*sp)
    columns=((-sl,cl,D(0)),(-sp*cl,-sp*sl,cp),(cp*cl,cp*sl,sp))
    return origin,columns

def frame_float(lon,lat,height):
    l=math.radians(float(lon)); p=math.radians(float(lat)); h=float(height)
    sl,cl,sp,cp=math.sin(l),math.cos(l),math.sin(p),math.cos(p)
    f=1/float(INVF); e2=f*(2-f); n=float(A)/math.sqrt(1-e2*sp*sp)
    return ((n+h)*cp*cl,(n+h)*cp*sl,(n*(1-e2)+h)*sp),((-sl,cl,0),(-sp*cl,-sp*sl,cp),(cp*cl,cp*sl,sp))

def dot(a,b): return sum(x*y for x,y in zip(a,b))
def cross(a,b): return (a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0])
def matapply(columns,p): return tuple(sum(columns[c][r]*p[c] for c in range(3)) for r in range(3))
def caxis(p): return (p[0],-p[2],p[1])
def ham(a,b):
    av,bv=a[:3],b[:3]; v=cross(av,bv)
    return tuple(a[3]*bv[i]+b[3]*av[i]+v[i] for i in range(3))+(a[3]*b[3]-dot(av,bv),)
def quat(q,p):
    v=ham(ham(q,tuple(p)+(0,)),tuple(-x for x in q[:3])+(q[3],))
    return v[:3]
def world(frame,p):
    o,b=frame; v=matapply(b,p)
    return tuple(x+y for x,y in zip(o,v))
def distance(a,b): return math.sqrt(sum(float(x-y)**2 for x,y in zip(a,b)))
def f32(v): return struct.unpack('<f',struct.pack('<f',v))[0]

def run():
    fixtures=[]; max_origin=0; max_axes=0
    with localcontext() as ctx:
        ctx.prec=80
        for name,lon,lat,height in CASES:
            o,b=frame_decimal(lon,lat,height); of,bf=frame_float(lon,lat,height)
            err=max(abs(float(x)-y) for x,y in zip(o,of)); max_origin=max(max_origin,err)
            max_axes=max(max_axes,max(abs(float(x)-y) for a,c in zip(b,bf) for x,y in zip(a,c)))
            assert err<=max(3e-9,2*math.ulp(max(abs(x) for x in of)))
            for i in range(3):
                for j in range(3): assert abs(dot(b[i],b[j])-(i==j))<D('1e-74')
            assert all(abs(x-y)<D('1e-74') for x,y in zip(cross(b[0],b[1]),b[2]))
            fixtures.append({'name':name,'request':{'longitude_degrees':lon,'latitude_degrees':lat,'ellipsoidal_height_metres':height},'origin_ecef_decimal':[str(x) for x in o],'enu_columns_decimal':[[str(x) for x in a] for a in b],'f64_origin_max_absolute_difference_metres':err})
    # Literal coordinate identities, independent of inverse round trips.
    assert list(map(D,fixtures[0]['origin_ecef_decimal']))==[D('6378137'),D(0),D(0)]
    north=fixtures[5]; assert [[D(x) for x in c] for c in north['enu_columns_decimal']]==[[0,1,0],[-1,0,0],[0,0,1]]
    south=fixtures[7]; assert [[D(x) for x in c] for c in south['enu_columns_decimal']]==[[0,1,0],[1,0,0],[0,0,-1]]
    # Noncommuting orientation with rational matrix coefficients.
    rows=((F(2,15),F(-2,3),F(11,15)),(F(14,15),F(1,3),F(2,15)),(F(-1,3),F(2,3),F(2,3)))
    q=tuple(i/math.sqrt(30) for i in (1,2,3,4)); p=(7,-11,13)
    literal=tuple(dot(r,p) for r in rows); actual=quat(q,p)
    assert max(abs(float(x)-y) for x,y in zip(literal,actual))<1e-14
    assert max(abs(x-y) for x,y in zip(quat(q,p),quat(tuple(-x for x in q),p)))<1e-14
    # Source-node translation/scaling -> post-node source offset -> C -> Q -> ENU.
    point=(1,2,3); node=(2*point[0]+3,2*point[1]-5,2*point[2]+7); off=(11,13,17)
    local=tuple(x+y for x,y in zip(node,off)); placed=quat(q,caxis(local)); anchor=frame_float('153','-27','130'); expected=world(anchor,placed)
    controls={
      'omit-yup-conversion':world(anchor,quat(q,local)),
      'double-yup-conversion':world(anchor,quat(q,caxis(caxis(local)))),
      'rotation-before-yup-conversion':world(anchor,caxis(quat(q,local))),
      'source-offset-before-node':world(anchor,quat(q,caxis(tuple(2*(point[i]+off[i])+(3,-5,7)[i] for i in range(3))))),
      'quaternion-wxyz-misread':world(anchor,quat((q[3],q[0],q[1],q[2]),caxis(local))),
      'wrong-anchor-height':world(frame_float('153','-27','0'),placed),
      'degrees-as-radians':world(frame_float(str(153*180/math.pi),str(-27*180/math.pi),'130'),placed),
      'geocentric-latitude':world(frame_float('153',str(math.degrees(math.atan2(anchor[0][2],math.hypot(anchor[0][0],anchor[0][1])))),'130'),placed),
    }
    sensitivities={k:distance(expected,v) for k,v in controls.items()}
    assert min(sensitivities.values())>1
    # Absolute-ECEF f32 bake has decimetre quantization; local f32 keeps tiny detail.
    detail=(.01,.02,.03); ecef=world(anchor,detail)
    precision={'direct_ecef_f32_error_metres':distance(ecef,tuple(f32(x) for x in ecef)), 'local_f32_error_metres':distance(detail,tuple(f32(x) for x in detail))}
    assert precision['direct_ecef_f32_error_metres']>.01 and precision['local_f32_error_metres']<1e-8
    # Complete-chain synthetic control: local OBB center/axes transformed separately
    # from world vertex. Zero half axes are tested by zero coordinate components.
    box_center=(16.,-30.,12.); half=(4.,8.,0.)
    corners=[tuple(box_center[i]+(-1 if bits&(1<<i) else 1)*half[i] for i in range(3)) for bits in range(8)]
    axes=tuple(tuple(anchor[1][i][r]*half[i] for r in range(3)) for i in range(3)); center=world(anchor,box_center)
    containment=[]
    for localp in corners:
        w=world(anchor,localp); delta=tuple(w[i]-center[i] for i in range(3)); projected=tuple(dot(delta,axis) for axis in anchor[1]);
        assert all(abs(projected[i])<=half[i]+2e-9 for i in range(3)); containment.append(list(w))
    return {'status':'prototype-self-test-passed','not_executed':'No production converter, Cargo, acceptance archive reader, renderer or installed API was run.','provenance':'Independent author; stdlib Decimal80 Taylor trig; NGA WGS84 constants; literal cardinals and rational Hamilton rotation; spec sources pinned separately.','anchor_cases':fixtures,'rational_orientation_rows':[[str(x) for x in r] for r in rows],'max_f64_origin_difference_metres':max_origin,'max_f64_axis_difference':max_axes,'sensitive_wrong_formula_controls_metres':sensitivities,'precision_representation_probe':precision,'synthetic_world_obb':{'center':center,'half_axis_columns':axes,'world_corners':containment}}

if __name__=='__main__':
    output=run(); root=Path(__file__).resolve().parent
    (root/'frozen-analytic-fixtures.json').write_text(json.dumps(output,indent=2)+'\n')
    summary={k:v for k,v in output.items() if k not in ('anchor_cases','synthetic_world_obb')}
    summary.update(script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),fixture_sha256=hashlib.sha256((root/'frozen-analytic-fixtures.json').read_bytes()).hexdigest(),anchor_case_count=len(CASES))
    (root/'prototype-execution.json').write_text(json.dumps(summary,indent=2)+'\n'); print(json.dumps(summary,indent=2))
