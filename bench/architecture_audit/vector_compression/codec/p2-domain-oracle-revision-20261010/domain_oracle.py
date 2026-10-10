#!/usr/bin/env python3
"""Additive oracle sensitivity revision; unchanged frozen fixture acceptance checks."""
from pathlib import Path
import argparse,json,hashlib,tempfile
HERE=Path(__file__).resolve().parent
ORIGINAL=Path('/tmp/rusty-tiles-next-vector-compression-audit/codec/p2-proof-preparation-20261010')
ROOT=ORIGINAL/'fixtures/generated-domain'
def check(case,dir):
    for name,expected in case['files'].items():
        raw=(dir/name).read_bytes()
        if name.endswith('.bin'): assert len(raw)==expected['bytes'] and hashlib.sha256(raw).hexdigest()==expected['sha256'],name
    assert json.loads((dir/'associations.json').read_text())==case['associations']
def rejected(case,directory,expected_reason):
    try:check(case,directory)
    except AssertionError as error:
        assert str(error)==expected_reason,('unexpected rejecting predicate',str(error),expected_reason)
        return 'rejected_by_unchanged_checker'
    raise AssertionError('faulty control was admitted by unchanged checker')
def association_mutation(associations):
    altered=dict(associations)
    key=next(k for k in ['propertyTable','valuesView','componentType','mode','nodeRaw','extrasRaw','sharedRefs','featureTable'] if k in altered)
    value=altered[key]
    if isinstance(value,bool):altered[key]=not value
    elif isinstance(value,int):altered[key]=value+1
    elif isinstance(value,list):altered[key]=value+[0]
    elif isinstance(value,str):altered[key]=value.replace('9007199254740993','9007199254740992') if '9007199254740993' in value else value+' changed'
    else:raise AssertionError(('unsupported frozen association type',key,type(value).__name__))
    assert altered!=associations,'association mutation must change the expected object'
    return key,altered
def selftest(cases):
    rows=[]
    for case in cases:
        original=ROOT/case['path'];check(case,original)
        controls=[]
        with tempfile.TemporaryDirectory(prefix='domain-oracle-faulty-control-') as temporary:
            directory=Path(temporary)
            originals={name:(original/name).read_bytes() for name in case['files']}
            for name,raw in originals.items():(directory/name).write_bytes(raw)
            check(case,directory)
            for name,raw in originals.items():
                if name.endswith('.bin') and raw:
                    changed=bytes([raw[0]^1])+raw[1:]
                    assert changed!=raw and len(changed)==len(raw)
                    assert hashlib.sha256(changed).hexdigest()!=case['files'][name]['sha256']
                    (directory/name).write_bytes(changed)
                    status=rejected(case,directory,name)
                    controls.append(dict(name='raw_'+name,mutationSha256=hashlib.sha256(changed).hexdigest(),status=status,rejectingPredicate='binary_length_and_sha256',assertionMessage=name))
                    (directory/name).write_bytes(raw)
            key,altered=association_mutation(case['associations'])
            changed=(json.dumps(altered,ensure_ascii=False,separators=(',',':'))+'\n').encode('utf-8')
            (directory/'associations.json').write_bytes(changed)
            status=rejected(case,directory,'')
            controls.append(dict(name='semantic_association_'+key,mutationSha256=hashlib.sha256(changed).hexdigest(),status=status,rejectingPredicate='association_object_equality',assertionMessage=''))
            (directory/'associations.json').write_bytes(originals['associations.json'])
            check(case,directory)
        rows.append(dict(name=case['name'],baseline='admitted_by_unchanged_checker',restored='admitted_by_unchanged_checker',sensitiveControls=controls))
    return dict(evidence='MODEL_ORACLE_SELFTEST_ONLY; no codec/producer/consumer execution',cases=rows)
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--candidate-extracted-dir',type=Path);a=p.parse_args();cases=json.loads((ROOT/'manifest.json').read_text())['cases']
    if a.candidate_extracted_dir:
        for c in cases:check(c,a.candidate_extracted_dir/c['path'])
        print(json.dumps(dict(status='all_raw_role_and_association_oracles_match',scope='Supplied independently extracted bytes only; decoder/extractor/source/artifact lineage must be recorded separately')))
    else:print(json.dumps(selftest(cases),indent=2))
