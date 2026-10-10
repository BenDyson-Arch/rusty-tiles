#!/usr/bin/env python3
"""Nonauthor probes: no production/test-helper imports or generated oracle truth."""
import collections
from decimal import Decimal as D, getcontext
from fractions import Fraction
import hashlib
import itertools
import json
import math
from pathlib import Path
import random
import struct
import subprocess
import sys
import zipfile

getcontext().prec = 90
ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
PIN = '3bd86c352c0a430731528040379ce1765fc11e67'
SHA = 'b0f2031f3f951018f6e19e682d62e86e1699dd15e382ca2d275300d30c7f99e5'
PI = D('3.141592653589793238462643383279502884197169399375105820974944592307816406286208998628034825342117')
IDENTITY = [1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.]
CAP = 58998676.19243112
def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def dec(x): return D.from_float(float(x))
def f32(x): return struct.unpack('<f', struct.pack('<f', x))[0]
def sincos(degrees):
    x = dec(degrees)*PI/180
    sinterm, costerm = x, D(1)
    s, c = sinterm, costerm
    for k in range(1, 130):
        sinterm *= -x*x/D(2*k*(2*k+1))
        costerm *= -x*x/D((2*k-1)*(2*k))
        s += sinterm
        c += costerm
    return s,c
def reference(anchor, quaternion, offset):
    lon,lat,h = anchor
    sl,cl = sincos(lon)
    sp,cp = sincos(lat)
    a = D(6378137)
    f = D(1)/D('298.257223563')
    e = f*(2-f)
    n = a/(1-e*sp*sp).sqrt()
    origin = [(n+dec(h))*cp*cl,(n+dec(h))*cp*sl,(n*(1-e)+dec(h))*sp]
    norm = sum(dec(v)**2 for v in quaternion).sqrt()
    x,y,z,w = [dec(v)/norm for v in quaternion]
    # Rotate the independent ENU basis vectors using Hamilton q v q^-1.
    qvec = [x,y,z]
    def rotate(v):
        cross = [qvec[1]*v[2]-qvec[2]*v[1],qvec[2]*v[0]-qvec[0]*v[2],qvec[0]*v[1]-qvec[1]*v[0]]
        dot = sum(qvec[i]*v[i] for i in range(3))
        return [(w*w-sum(t*t for t in qvec))*v[i]+2*dot*qvec[i]+2*w*cross[i] for i in range(3)]
    def world_vector(v):
        e,n,u = rotate(v)
        return [-sl*e-sp*cl*n+cp*cl*u,cl*e-sp*sl*n+cp*sl*u,cp*n+sp*u]
    columns = [world_vector([D(int(i==j)) for i in range(3)]) for j in range(3)]
    delta = world_vector([dec(offset[0]),-dec(offset[2]),dec(offset[1])])
    return [columns[j][i] if i<3 and j<3 else (origin[i]+delta[i] if j==3 and i<3 else D(int(i==j))) for j in range(4) for i in range(4)], [float(dec(v)/norm) for v in quaternion]
def apply(matrix, point):
    return [sum(matrix[c*4+r]*dec(point[c]) for c in range(3))+matrix[12+r] for r in range(3)]
def apply_float(matrix, point):
    return [sum(float(matrix[c*4+r])*point[c] for c in range(3))+float(matrix[12+r]) for r in range(3)]
def make_fixture(name, positions, nodes=False):
    positions = [tuple(f32(x) for x in p) for p in positions]
    data = b''.join(struct.pack('<3f',*p) for p in positions)
    doc={'asset':{'version':'2.0'},'buffers':[{'byteLength':len(data)}], 'bufferViews':[{'buffer':0,'byteOffset':0,'byteLength':len(data)}], 'accessors':[{'bufferView':0,'componentType':5126,'count':len(positions),'type':'VEC3'}], 'meshes':[{'primitives':[{'attributes':{'POSITION':0},'mode':4}]}], 'nodes':[{'mesh':0}], 'scenes':[{'nodes':[0]}], 'scene':0}
    doc['accessors'][0]['min']=[min(p[i] for p in positions) for i in range(3)]
    doc['accessors'][0]['max']=[max(p[i] for p in positions) for i in range(3)]
    expected=positions
    if nodes:
        doc['nodes']=[{'translation':[100,-200,300],'children':[1]}, {'matrix':[0,3,0,0,-2,0,0,0,0,0,.5,0,7,-11,13,1],'mesh':0}]
        expected=[tuple(f32(t) for t in (-2*y+107,3*x-211,.5*z+313)) for x,y,z in positions]
    encoded=json.dumps(doc,separators=(',',':')).encode()
    encoded += b' '*((-len(encoded))%4)
    data += b'\0'*((-len(data))%4)
    blob=struct.pack('<4sII',b'glTF',2,28+len(encoded)+len(data))+struct.pack('<I4s',len(encoded),b'JSON')+encoded+struct.pack('<I4s',len(data),b'BIN\0')+data
    path=HERE/'fixtures'/f'{name}.glb'
    path.parent.mkdir(exist_ok=True)
    path.write_bytes(blob)
    return path,expected
def decode(blob):
    jsize=struct.unpack_from('<I',blob,12)[0]
    doc=json.loads(blob[20:20+jsize])
    bstart=28+jsize
    points=[]
    for mesh in doc['meshes']:
        for prim in mesh['primitives']:
            accessor=doc['accessors'][prim['attributes']['POSITION']]
            view=doc['bufferViews'][accessor['bufferView']]
            start=bstart+view.get('byteOffset',0)+accessor.get('byteOffset',0)
            points.extend(struct.unpack_from('<3f',blob,start+i*view.get('byteStride',12)) for i in range(accessor['count']))
    return points
def box_check(box, points):
    # Emitted local boxes are axis aligned; exact rational arithmetic allows zero extents.
    for p in points:
        b=(p[0],-p[2],p[1])
        for i in range(3):
            assert abs(Fraction(b[i])-Fraction(box[i])) <= Fraction(box[3+4*i]), ('local containment',box,p,i)
def verify_world_boxes(root, matrix, ref, archive):
    for child in root['children']:
        assert 'transform' not in child
        points=decode(archive[child['content']['uri']])
        for box in [root['boundingVolume']['box'],child['boundingVolume']['box']]:
            box_check(box, points)
            center=apply(matrix,box[:3])
            half=[box[3],box[7],box[11]]
            # High-precision independent target, compared to transformed oriented box axes.
            for p in points:
                target=apply(ref,[p[0],-p[2],p[1]])
                delta=[target[i]-center[i] for i in range(3)]
                for axis in range(3):
                    projection=sum(delta[r]*matrix[axis*4+r] for r in range(3))
                    assert abs(projection)<=dec(half[axis])+D('0.000001'), ('world OBB',axis,p)
def main():
    manifest=json.loads(Path(sys.argv[1]).read_text())
    binary=Path(manifest['portable_binary']['path'])
    assert manifest['source_commit']==PIN
    assert manifest['portable_binary']['sha256']==SHA==sha(binary)
    assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()==PIN
    for path,digest in manifest['compiled_inputs_sha256'].items():
        assert sha(ROOT/path)==digest, ('worktree drift',path)
        committed=subprocess.check_output(['git','show',f'{PIN}:{path}'],cwd=ROOT)
        assert hashlib.sha256(committed).hexdigest()==digest, ('commit drift',path)
    real_budget=D(2)**26-D(6378137)-D(3).sqrt()*D(1000000)
    assert dec(CAP)<=real_budget<dec(math.nextafter(CAP,math.inf))
    results={'source_commit':PIN,'binary_sha256':SHA,'compiled_inputs_verified':len(manifest['compiled_inputs_sha256']),'executions':[],'findings':[],'limits':['Portable CLI artifact only; no native, installed Python, browser, Blender or non-Unix execution here.','Own fixture has POSITION only; authored PBR/resource truth requires separate evidence.']}
    runs=HERE/'runs'
    runs.mkdir(exist_ok=True)
    def invoke(name, source, extra=(), extension='.3tz', expected_kind=None):
        output=runs/(name+extension)
        output.unlink(missing_ok=True)
        command=[str(binary),'--json','mesh-local-to-3tz','-i',str(source),'-o',str(output),'--leaf-triangles','1',*map(str,extra)]
        completed=subprocess.run(command,text=True,capture_output=True,timeout=20)
        parsed=json.loads(completed.stdout)
        record={'name':name,'argv':command,'exit':completed.returncode,'result':parsed}
        results['executions'].append(record)
        if expected_kind:
            assert completed.returncode != 0 and parsed['error']['kind']==expected_kind,record
            assert not output.exists(),record
            return parsed
        assert completed.returncode==0,record
        with zipfile.ZipFile(output) as z: archive={p:z.read(p) for p in z.namelist()}
        return parsed,archive
    regular,truth=make_fixture('hierarchical',[(0,0,0),(1,2,3),(-4,5,-6),(123456,-234567,345678),(5,-9,31),(9,4,-3),(20,7,8),(1,-5,3),(-12,14,9),(17,21,-31),(-4,7,9),(1,2,3)],True)
    degenerate,dtruth=make_fixture('line',[(x,7,-9) for x in (-1e-38,1e6,0,1e-38,33,1,0,0,0)])
    results['fixtures']=[{'name':'hierarchical','path':str(regular.relative_to(HERE)),'sha256':sha(regular),'expected_baked_positions':truth}, {'name':'line','path':str(degenerate.relative_to(HERE)),'sha256':sha(degenerate),'expected_baked_positions':dtruth}]
    results['cap_floor_verification']={'real_budget_decimal90':str(real_budget),'operative_cap_exact_decimal':str(dec(CAP)),'next_f64_exact_decimal':str(dec(math.nextafter(CAP,math.inf)))}
    local,localmembers=invoke('local',regular)
    assert local['meshReport']['root_transform']==IDENTITY
    lp=json.loads(localmembers['tileset.json'])
    assert lp['root']['transform']==IDENTITY
    assert collections.Counter(p for c in lp['root']['children'] for p in decode(localmembers[c['content']['uri']]))==collections.Counter(truth)
    cases=[('equator',[0,0,0],[0,0,0,1],[0,0,0]),('northpole',[37,90,12345],[.5,.5,.5,.5],[7,-11,13]),('southpole',[-123,-90,-6000000],[-.5,.5,.5,.5],[-3,8,-17]),('eastanti',[180,17,58000000],[.5,-.5,.5,.5],[1,-2,3]),('westanti',[-180,-31,-6378137],[0,0,0,1],[2,3,5]),('center',[0,0,-6378137],[.5,.5,.5,.5],[0,0,0]),('oblique',[153.02,-27.48,57200000],[.18257418583505536,.3651483716701107,.5477225575051661,.7302967433402214],[300,-200,100]),('nearunit',[12.75,71.5,-1234],[0,0,0,1+5e-13],[17,19,-23])]
    cases.extend([('height-cap',[37.1,-55.4,CAP],[.5,.5,.5,.5],[0,0,0]),('offset-cap',[-161.2,83.7,0],[.5,-.5,.5,.5],[0,-CAP,0]),('negative-q',[153.02,-27.48,57200000],[-.18257418583505536,-.3651483716701107,-.5477225575051661,-.7302967433402214],[300,-200,100])])
    maximum_error=D(0)
    maximum_evaluation_error=D(0)
    control_results=[]
    for fname,source,expected in [('hierarchy',regular,truth),('degenerate',degenerate,dtruth)]:
        baseline=localmembers if fname=='hierarchy' else invoke('line-local',source)[1]
        for name,anchor,q,offset in cases:
            extra=['--anchor',*anchor,'--orientation-xyzw',*q,'--scene-offset',*offset]
            parsed,archive=invoke(fname+'-'+name,source,extra)
            doc=json.loads(archive['tileset.json'])
            report=json.loads(archive['conversion.json'])
            assert report==parsed['meshReport']
            assert report['root_transform']==doc['root']['transform']
            assert report['schema_version']==4 and report['profile']=='f1c1-placed-gltf-v1'
            assert report['source_coordinates']=='local-gltf' and report['coordinates']=='wgs84-ecef'
            ref,nq=reference(anchor,q,offset)
            assert max(abs(x-y) for x,y in zip(nq,report['placement']['orientation_xyzw']))<3e-16
            matrix=list(map(dec,doc['root']['transform']))
            for p in expected:
                point=[p[0],-p[2],p[1]]
                error=max(abs(x-y) for x,y in zip(apply(matrix,point),apply(ref,point)))
                maximum_error=max(error,maximum_error)
                assert error<=D('0.000001'),(fname,name,p,error)
                evaluation_error=max(abs(dec(x)-y) for x,y in zip(apply_float(matrix,point),apply(ref,point)))
                maximum_evaluation_error=max(evaluation_error,maximum_evaluation_error)
                assert evaluation_error<=D('0.000001'),(fname,name,p,evaluation_error)
            for member in baseline:
                if member.endswith('.glb'): assert archive[member]==baseline[member],(name,member)
            assert doc['root']['geometricError']==json.loads(baseline['tileset.json'])['root']['geometricError']
            verify_world_boxes(doc['root'],matrix,ref,archive)
            if name=='oblique' and fname=='hierarchy':
                _,zipped=invoke('zip-placed',source,extra,'.3dtiles.zip')
                assert archive==zipped
                # Deliberate world-chain errors must exceed the independent accuracy check.
                for control in ('translation','transpose','double-B'):
                    bad=matrix.copy()
                    if control=='translation': bad[12]+=1
                    elif control=='transpose':
                        bad=[matrix[r*4+c] if r<3 and c<3 else matrix[c*4+r] for c in range(4) for r in range(4)]
                    else:
                        bad[4:7]=matrix[8:11]
                        bad[8:11]=[-v for v in matrix[4:7]]
                    worst=max(abs(x-y) for p in expected for x,y in zip(apply(bad,[p[0],-p[2],p[1]]),apply(ref,[p[0],-p[2],p[1]])))
                    assert worst>D('0.000001'),(control,worst)
                    control_results.append({'control':control,'detected_error_metres':str(worst)})
                # A zero-extent violation and a shrunken local box must be rejected exactly.
                first=doc['root']['children'][0]
                box=first['boundingVolume']['box'].copy()
                box[3]=0
                try: box_check(box,decode(archive[first['content']['uri']]))
                except AssertionError: control_results.append({'control':'undersized-local-box','detected':True})
                else: raise AssertionError('insensitive undersized-local-box control')
    # Exact Fraction admission truth, including tiny positive residuals rounded away by naive sum.
    boundary_cases=[(CAP,[0,0,0]),(CAP,[5e-324,0,0]),(CAP,[-5e-324,0,0]),(math.nextafter(CAP,-math.inf),[1e-10,1e-10,1e-10]),(math.nextafter(CAP,-math.inf),[1e-8,0,0]),(0,[CAP,0,5e-324])]
    rng=random.Random(142)
    for i in range(35):
        offset=[rng.choice([0,5e-324,1e-100,1e-12,1e-9,3e-9,7e-9,1e-8])*rng.choice([-1,1]) for _ in range(3)]
        boundary_cases.append((math.nextafter(CAP,-math.inf) if i%2 else CAP,offset))
    for i in range(35):
        # Spread the near-cap sum over every component, with several exponent scales.
        a=rng.uniform(0, CAP/3)
        b=rng.uniform(0, CAP/3)
        c=rng.choice([5e-324,1e-100,1e-8,1,1e6])
        d=CAP-a-b-c
        parts=[a,b,c,math.nextafter(d,math.inf if i%2 else -math.inf)]
        rng.shuffle(parts)
        boundary_cases.append((rng.choice([-1,1])*parts[0],[rng.choice([-1,1])*v for v in parts[1:]]))
    for i,(h,d) in enumerate(boundary_cases):
        exact=Fraction(abs(h))+sum(Fraction(abs(v)) for v in d)
        supported=exact<=Fraction(CAP)
        missing=HERE/'never-created-source.glb'
        invoke(f'budget-{i}',missing,['--anchor',0,0,h,'--scene-offset',*d],expected_kind='io' if supported else 'unsupported')
    for i,args in enumerate([['--anchor',181,0,0],['--anchor',0,-91,0],['--anchor',0,0,'NaN'],['--anchor',0,0,0,'--orientation-xyzw',0,0,0,0],['--orientation-xyzw',0,0,0,1],['--scene-offset',0,0,0],['--anchor',0,0,0,'--scene-offset','inf',0,0]]):
        invoke(f'invalid-{i}',HERE/'never-created-source.glb',args,expected_kind='invalid_request')
    # Source/output identity refusal preserves the source even under requested replacement.
    alias=HERE/'fixtures'/'same.3tz'
    alias.write_bytes(regular.read_bytes())
    before=alias.read_bytes()
    p=subprocess.run([str(binary),'--json','mesh-local-to-3tz','-i',str(alias),'-o',str(alias),'--leaf-triangles','1','--force','--anchor','1','2','3'],text=True,capture_output=True,timeout=20)
    obj=json.loads(p.stdout)
    assert p.returncode and obj['error']['kind']=='invalid_request' and alias.read_bytes()==before,obj
    results['executions'].append({'name':'same-file-alias','exit':p.returncode,'result':obj})
    assert not list(runs.glob('.mesh-work-*')) and not list(runs.glob('.rusty-tiles-*'))
    assert sha(binary)==SHA
    results['maximum_decimal_world_coordinate_error_metres']=str(maximum_error)
    results['maximum_f64_world_evaluation_error_metres']=str(maximum_evaluation_error)
    results['boundary_cases']=len(boundary_cases)
    results['sensitivity_controls']=control_results
    results['status']='passed'
    results['probe_sha256']=sha(Path(__file__))
    (HERE/'fresh-review-results.json').write_text(json.dumps(results,indent=2)+'\n')
    print(json.dumps({'status':'passed','executions':len(results['executions']),'boundary_cases':len(boundary_cases),'max_world_error_metres':str(maximum_error)}))
if __name__=='__main__': main()
