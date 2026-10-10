#!/usr/bin/env python3
"""Frozen F1d1 candidate feasibility only: never F1d2 producer acceptance."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import zipfile

REPO=Path(__file__).resolve().parents[4]
sys.path.insert(0,str(REPO/'tests'))
import f1d1_oracle as oracle
import f1d2_certificate as exact

binary=Path('/home/bend/.cache/rusty-tiles-f1d-evidence/rusty-tiles-native-0eeb906')
expected='81fca6a80c1e64cfe6cec953838eba8cce6b39cadcd45a193e4a1096c8e44965'
assert hashlib.sha256(binary.read_bytes()).hexdigest()==expected
work=Path('/home/bend/.cache/rusty-tiles-f1d2-evidence-feasibility');work.mkdir(exist_ok=True)
receipts=[]
for variant,limit in [('grid',16),('bump',8),('bump',2),('boundary-ring',2)]:
    source=oracle.fixture(variant);case=work/f'{variant}-{limit}';case.mkdir(exist_ok=True)
    inp=case/'source.glb';inp.write_bytes(source);output=case/'result.3tz'
    command=[str(binary),'--json','mesh-local-to-3tz','-i',str(inp),'-o',str(output),'--force','--leaf-triangles','16','--root-proxy-triangles',str(limit),'--max-proxy-error-metres','8']
    result=subprocess.run(command,capture_output=True,text=True,timeout=60)
    item={'variant':variant,'triangle_limit':limit,'command':command,'source_sha256':oracle.digest(source),'exit_code':result.returncode,'stdout':result.stdout,'stderr':result.stderr}
    if result.returncode==0:
        with zipfile.ZipFile(output) as z:members={n:z.read(n) for n in z.namelist() if n!='@3dtilesIndex1@'}
        root=json.loads(members['tileset.json'])['root']['content']['uri'];doc,raw,_=oracle.decode(members[root]);proxy=[]
        for primitive in doc['meshes'][0]['primitives']:
            points=oracle.accessor(doc,raw,primitive['attributes']['POSITION'])
            indices=oracle.accessor(doc,raw,primitive['indices']) if 'indices' in primitive else list(range(len(points)))
            proxy.extend([[points[i] for i in indices[start:start+3]] for start in range(0,len(indices),3)])
        truth,_=oracle.leaf_oracle.expected_source(source);original=[x['positions'] for x in truth.values()]
        item.update(archive_sha256=oracle.digest(output.read_bytes()),proxy_triangles=len(proxy),historical_squared=str(oracle.ideal_face_certificate_squared(original,proxy)),source_vertex_gap_squared=str(max(min(oracle.point_triangle_squared(p,t) for t in proxy) for f in original for p in f)))
        try:item['independent_prospective_half_metre_cover']=exact.certify_regions([(original,proxy)],.5)
        except exact.CertificateUnproven as error:item['independent_prospective_half_metre_cover']={'proved':False,'reason':str(error)}
    receipts.append(item)
paths=[Path(__file__),Path(oracle.__file__),Path(exact.__file__),Path(oracle.leaf_oracle.__file__)]
receipt={'scope':'Frozen F1d1 optimizer-candidate feasibility only; no F1d2 producer/profile/certificate acceptance','binary_sha256':expected,'cases':receipts,'driver_hashes':{p.name:oracle.digest(p.read_bytes()) for p in paths}}
(REPO/'bench/architecture_audit/mesh_approximation_f1d2/old-cli-feasibility.json').write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps({'cases':len(receipts),'driver_sha256':oracle.digest(Path(__file__).read_bytes())}))
