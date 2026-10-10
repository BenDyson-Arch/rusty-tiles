#!/usr/bin/env python3
"""Nonauthor byte bindings and read-only replay of final real-source decoders.

Requires externally retained exact artifacts; launches only small reference
meshopt decoding, never Rust/CLI production. Not a second general glTF parser.
"""
from fractions import Fraction as F
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import sys
import zipfile

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[3]
REAL=HERE.parent/'a2_real_sources'


def sha(data): return hashlib.sha256(data).hexdigest()


def independent_coherence(nodes):
    checks=0
    for n in nodes:
        for owner in nodes:
            if n['path'][:len(owner['path'])]!=owner['path']: continue
            b=owner['node']['boundingVolume']['box']; m=owner['matrix']
            assert m[:12]==[F(i%5==0) for i in range(12)]
            for world in n['positions']:
                local=[F(world[a])-F(m[12+a]) for a in range(3)]
                assert all(abs(local[a]-F(b[a]))<=F(b[3+4*a]) for a in range(3))
                checks+=1
    return checks


def stored(path):
    with zipfile.ZipFile(path) as z:
        assert len(z.namelist())==len(set(z.namelist())) and z.testzip() is None
        assert all(x.compress_type==zipfile.ZIP_STORED for x in z.infolist())
        return {n:z.read(n) for n in z.namelist() if n!='@3dtilesIndex1@'}


def main():
    assert not sys.flags.optimize
    out=Path(sys.argv[1]); assert str(out.resolve()).startswith('/tmp/') and not out.exists()
    summaries=json.loads((REAL/'results.json').read_bytes()); runs={}
    spec=importlib.util.spec_from_file_location('real_model',REAL/'probe.py')
    model=importlib.util.module_from_spec(spec); spec.loader.exec_module(model)
    for mode,arg in zip(('portable','native'),sys.argv[2:]):
        directory=Path(arg); raw=(directory/'receipt.json').read_bytes(); receipt=json.loads(raw)
        summary=summaries['runs'][mode]; compressed=(REAL/summary['raw_receipt']).read_bytes()
        assert gzip.decompress(compressed)==raw
        assert sha(compressed)==summary['raw_gzip_sha256'] and sha(raw)==summary['raw_receipt_sha256']
        pins=receipt['pins']; assert sha((REAL/'probe.py').read_bytes())==pins['probe_sha256']==summary['probe_sha256']
        assert sha((REAL/'meshopt_decode.mjs').read_bytes())==pins['wrapper_sha256']
        assert sha(Path(pins['binary']).read_bytes())==pins['binary_sha256']
        assert sha(Path(pins['codec']).read_bytes())==pins['codec_sha256']
        for name,digest in pins['production_sha256'].items(): assert sha((ROOT/name).read_bytes())==digest
        for name,digest in receipt['generated_artifacts'].items(): assert sha((directory/name).read_bytes())==digest
        decoder=model.Decoder(Path(pins['codec'])); cases=[]; b3dm=[]
        for c in receipt['cases']:
            label=c['label']; source_path=directory/(label+'.3tz'); source=stored(source_path)
            assert sha(source_path.read_bytes())==c['source_sha256']
            assert {n:sha(b) for n,b in source.items()}==c['source_members']
            for name,data in source.items():
                if data[:4]==b'b3dm':
                    version,length,fj,fb,bj,bb=struct.unpack_from('<6I',data,4)
                    ft=json.loads(data[28:28+fj]); assert ft=={'BATCH_LENGTH':0} and fb==bj==bb==0
                    b3dm.append({'source':label,'member':name,'feature_table':ft,'RTC_CENTER':False,'batch_table':False})
            nodes,leaves=model.source_geometry(source,decoder)
            checks=independent_coherence(nodes)
            assert len(leaves)==c['source_leaf_rendered_count']
            assert model.ordinal_plan(nodes,c['scheme'])==c['ordinal_design_plan']
            assert [n['metadata_tables'] for n in nodes]==c['feature_metadata_tables']
            aliases=0
            if c['legacy_exit']==0:
                output_path=directory/(label+'-implicit.3tz'); output=stored(output_path)
                assert sha(output_path.read_bytes())==c['output_sha256']
                observed=model.owned_oracle(source,output,c['scheme'])
                assert json.loads(json.dumps(observed))==c['owned_oracle']
                aliases=observed['alias_count']
                for name,data in source.items():
                    if name not in ('tileset.json','conversion.json'): assert output[name]==data
                assert model.closure(source,[h['uri'] for n in nodes for h in model.headers(n['node'])])==c['source_resource_closure']
                assert model.closure(output,[a[1] for a in observed['aliases']])==c['alias_resource_closure']
            cases.append({'label':label,'legacy_exit':c['legacy_exit'],'nodes':len(nodes),'rendered_leaf_positions':len(leaves),'independent_ancestor_checks':checks,'aliases':aliases})
        assert len(receipt['controls'])==len(summary['controls'])==19
        assert len(receipt['commands'])==summary['commands']
        runs[mode]={'raw_receipt_sha256':sha(raw),'driver_sha256':pins['probe_sha256'],
                    'production_inputs':len(pins['production_sha256']), 'generated_artifacts':len(receipt['generated_artifacts']),
                    'cases':cases,'controls':19,'b3dm_profile':b3dm,'reference_codec_calls':decoder.codec_calls,
                    'limitations':'Authored decoder replay plus separate exact ancestor containment over decoded coordinates; no independent general codec, replacement artifact, 3TZ index or viewer acceptance.'}
    result={'evidence':'nonauthor final real-source byte bindings; read-only model replay',
            'driver_sha256':sha(Path(__file__).read_bytes()),'runs':runs}
    out.write_text(json.dumps(result,indent=2,sort_keys=True)+'\n')
    print(json.dumps({'receipt':str(out),'runs':{m:{'cases':len(r['cases']),'successes':sum(c['legacy_exit']==0 for c in r['cases'])} for m,r in runs.items()}}))


if __name__=='__main__': main()
