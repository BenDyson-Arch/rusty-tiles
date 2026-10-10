#!/usr/bin/env python3
"""Raw-role/association oracle self-test or compare independently extracted future candidate files."""
from pathlib import Path
import argparse,json,hashlib
HERE=Path(__file__).resolve().parent
ROOT=HERE/'fixtures/generated-domain'
def check(case,dir):
    for name,expected in case['files'].items():
        raw=(dir/name).read_bytes()
        if name.endswith('.bin'): assert len(raw)==expected['bytes'] and hashlib.sha256(raw).hexdigest()==expected['sha256'],name
    assert json.loads((dir/'associations.json').read_text())==case['associations']
def selftest(cases):
    rows=[]
    for case in cases:
        dir=ROOT/case['path'];check(case,dir)
        mutations=[]
        for name,expected in case['files'].items():
            raw=(dir/name).read_bytes()
            if name.endswith('.bin') and raw:
                changed=bytes([raw[0]^1])+raw[1:];assert hashlib.sha256(changed).hexdigest()!=expected['sha256'];mutations.append('raw_'+name)
        altered=dict(case['associations'])
        key=next((k for k in ['propertyTable','valuesView','componentType','mode','nodeRaw','extrasRaw','sharedRefs'] if k in altered))
        value=altered[key]
        if isinstance(value,int): altered[key]=value+1
        elif isinstance(value,list): altered[key]=list(reversed(value))
        else: altered[key]=value.replace('9007199254740993','9007199254740992') if '9007199254740993' in value else value+' changed'
        assert altered!=case['associations'];mutations.append('semantic_association_'+key)
        rows.append(dict(name=case['name'],sensitiveControls=mutations))
    return dict(evidence='MODEL_ORACLE_SELFTEST_ONLY; no codec/producer/consumer execution',cases=rows)
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--candidate-extracted-dir',type=Path);a=p.parse_args();cases=json.loads((ROOT/'manifest.json').read_text())['cases']
    if a.candidate_extracted_dir:
        for c in cases:check(c,a.candidate_extracted_dir/c['path'])
        print(json.dumps(dict(status='all_raw_role_and_association_oracles_match',scope='Supplied independently extracted bytes only; decoder/extractor/source/artifact lineage must be recorded separately')))
    else:print(json.dumps(selftest(cases),indent=2))
