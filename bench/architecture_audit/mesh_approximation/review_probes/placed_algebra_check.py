import json,gzip,math,hashlib,sys,zipfile
from pathlib import Path
r=Path(__file__).resolve().parents[4]
q=json.loads(gzip.decompress((r/'bench/architecture_audit/mesh_approximation/placed-browser.json.gz').read_bytes()))
lon,lat,height=q['placement_request']['anchor'];lon,lat=math.radians(lon),math.radians(lat)
a=6378137.;b=a*(1-1/298.257223563)
sl,cl,sp,cp=math.sin(lon),math.cos(lon),math.sin(lat),math.cos(lat)
# Algebraically alternate radius form using the two ellipsoid semiaxes.
n=a*a/math.sqrt((a*cp)**2+(b*sp)**2)
origin=[(n+height)*cp*cl,(n+height)*cp*sl,((b/a)**2*n+height)*sp]
columns=[[-sl,cl,0.],[-sp*cl,-sp*sl,cp],[cp*cl,cp*sl,sp]]
reference=[value for col in columns for value in col+[0.]]+origin+[1.]
errors=[abs(x-y) for x,y in zip(reference,q['emitted_root_transform'])]
assert max(errors[:12])<=2e-15 and max(errors[12:15])<=1e-8
sys.path.insert(0,str(r/'tests'));import f1d1_oracle as oracle;import f1d1_viewer as viewer
source=Path(q['source']).read_bytes();plan=viewer.plan(source)
def world(v,vector=False):return [sum(columns[c][axis]*v[c] for c in range(3))+(0. if vector else origin[axis]) for axis in range(3)]
camera_errors=[]
for label,camera in plan['camera'].items():
 for field,value in camera.items():
  wanted=world(value,field!='destination');actual=q['browser']['camera'][label][field]
  camera_errors.extend(abs(x-y) for x,y in zip(wanted,actual))
assert max(camera_errors)<=1e-8
point_errors=[]
for run in q['browser']['successful_queries']:
 for target,sample in zip(plan['targets'],run['samples']):
  point_errors.extend(abs(x-y) for x,y in zip(world(target['world']),sample['world']))
assert max(point_errors)<=1e-8
print(json.dumps({'scope':'Independent alternate-semiaxis double-precision algebra crosscheck; Decimal80 source audit remains the stricter accuracy reference', 'placed_archive_sha256':q['archive_sha256'],'basis_difference_max':max(errors[:12]),'origin_difference_metres_max':max(errors[12:15]),'camera_difference_max':max(camera_errors),'target_difference_metres_max':max(point_errors)},indent=2))
