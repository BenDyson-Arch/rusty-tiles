#!/usr/bin/env python3
"""Read-only audit: independent byte fixtures, current CLI; results are observations."""
import argparse, hashlib, importlib.util, json, pathlib, struct, subprocess, sys, tempfile
sys.dont_write_bytecode=True
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary',required=True,type=pathlib.Path,help='Already-built CLI; this probe does not compile')
parser.add_argument('--repo',type=pathlib.Path,default=next((p for p in pathlib.Path(__file__).resolve().parents if (p/'bench/architecture_audit/api/probe.py').is_file()),pathlib.Path.cwd()),help='Source checkout (default: ancestor of this script, otherwise cwd)')
parser.add_argument('--output',type=pathlib.Path,help='Write JSON to this path; otherwise print JSON')
parser.add_argument('--build-source-revision',default='unknown',help='Supplied build source revision; not verified from binary')
parser.add_argument('--build-features',default='unknown',help='Supplied build features; not inferred from binary')
parser.add_argument('--build-provenance',default='unknown',help='Supplied toolchain/build details; not verified from binary')
args=parser.parse_args()
ROOT=args.repo.resolve();BIN=args.binary.resolve()
spec=importlib.util.spec_from_file_location('fixture',ROOT/'bench/architecture_audit/api/probe.py')
fixture=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixture)
result={'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
        'binary_sha256':hashlib.sha256(BIN.read_bytes()).hexdigest(),
        'binary':str(BIN),'supplied_build_metadata':{'source_revision':args.build_source_revision,'features':args.build_features,'provenance':args.build_provenance},
        'fixture_helper':{'path':'bench/architecture_audit/api/probe.py','sha256':hashlib.sha256((ROOT/'bench/architecture_audit/api/probe.py').read_bytes()).hexdigest(),'provenance':'standard-library independent GLB byte fixture generator retained from #113 baseline API audit'},
        'cases':{}}
def run(args):
    p=subprocess.run([str(BIN),*map(str,args)],text=True,capture_output=True)
    return {'arguments':list(map(str,args)),'exit':p.returncode,'stdout':p.stdout,'stderr':p.stderr}
with tempfile.TemporaryDirectory(prefix='f1-evidence-') as temporary:
    work=pathlib.Path(temporary)
    base=fixture.glb([[0,0,0],[1,0,0],[0,1,0]])
    size=struct.unpack_from('<I',base,12)[0]
    doc=json.loads(base[20:20+size]);document=base[20:20+size]
    # Accessor and bufferView require 36 bytes but BIN chunk contains zero.
    truncated=(struct.pack('<4sII',b'glTF',2,28+size)+struct.pack('<II',size,0x4e4f534a)+document+struct.pack('<II',0,0x004e4942))
    inputs={'valid_control':base,'missing_binary_payload':truncated,
            'geometry_outside_bounds':fixture.glb([[1000,1000,1000],[1001,1000,1000],[1000,1001,1000]])}
    manifest={'asset':{'version':'1.1'},'geometricError':0,'root':{'boundingVolume':{'box':[0.5,0.5,0,0.5,0,0,0,0.5,0,0,0,0.5]},'geometricError':0,'content':{'uri':'triangle.glb'}}}
    for name,data in inputs.items():
        source=work/name;source.mkdir();(source/'tileset.json').write_text(json.dumps(manifest));(source/'triangle.glb').write_bytes(data)
        archive=work/(name+'.3tz')
        case={'fixture_sha256':hashlib.sha256(data).hexdigest(),'pack':run(['--json','convert','-i',source,'-o',archive])}
        case['validate']=run(['--json','validate',archive])
        case['independent_fact']= {'valid_control':'all 3 source positions lie inside declared box; BIN contains 36 bytes',
                                  'missing_binary_payload':'declared bufferView byteLength=36, actual BIN byteLength=0; required position data absent',
                                  'geometry_outside_bounds':'all source position coordinates >=1000; declared box ranges x/y 0..1, z -0.5..0.5'}[name]
        result['cases'][name]=case
    # Current legacy mesh frontend probes, using identical source bytes plus CLI-only options.
    source=work/'geographic.glb';source.write_bytes(fixture.glb([[153,10,27],[153.0001,10,27],[153,11,27.0001]]))
    result['cases']['legacy_geographic_offset_cli']=run(['--json','meshTo3tz','-i',source,'-o',work/'offset.3tz','--sourceCrs','geographic','--sourceOffset','1000','2000','300'])
    for case in result['cases'].values():
        for key in ['pack','validate']:
            if key in case:
                case[key]['arguments']=[s.replace(str(work),'<work>') for s in case[key]['arguments']]
                for f in ['stdout','stderr']:case[key][f]=case[key][f].replace(str(work),'<work>')
        if 'arguments' in case:case['arguments']=[s.replace(str(work),'<work>') for s in case['arguments']]
        for f in ['stdout','stderr']:
            if f in case:case[f]=case[f].replace(str(work),'<work>')
encoded=json.dumps(result,indent=2)+'\n'
if args.output:
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(encoded)
else:
    print(encoded,end='')
