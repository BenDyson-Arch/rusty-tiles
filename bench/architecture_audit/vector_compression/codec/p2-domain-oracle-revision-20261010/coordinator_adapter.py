#!/usr/bin/env python3
"""Root-only additive model runner. Does not build or execute codec/native targets."""
import argparse,hashlib,json,re,runpy
from pathlib import Path
HERE=Path(__file__).resolve().parent
ORIGINAL=Path('/tmp/rusty-tiles-next-vector-compression-audit/codec/p2-proof-preparation-20261010')
sha=lambda raw:hashlib.sha256(raw).hexdigest()
def verify_revision():
    record=json.loads((HERE/'integrity.json').read_text())
    for name,pin in record['files'].items():
        raw=(HERE/name).read_bytes();assert len(raw)==pin['bytes'] and sha(raw)==pin['sha256'],name
    for path,pin in record['externalInputs'].items():
        raw=Path(path).read_bytes();assert len(raw)==pin['bytes'] and sha(raw)==pin['sha256'],path
    return sha((HERE/'integrity.json').read_bytes())
def main(args):
    revision=verify_revision()
    # run_path does not create __pycache__ or run the original CLI entrypoint.
    original=runpy.run_path(str(ORIGINAL/'coordinator_runner.py'))
    package=original['verify_package']()
    delta=json.loads((HERE/'revision.json').read_text());assert package==delta['originalPackageIntegritySha256']
    gate_raw=args.c1_gate.read_bytes();gate=json.loads(gate_raw)
    assert gate['acceptanceStatus']=='accepted_merged_develop'
    assert re.fullmatch('[0-9a-f]{40}',gate['acceptedMergeCommit'])
    assert gate['acceptedMergeCommit']==delta['acceptedC1MergeCommit']
    assert original['file_sha'](gate['acceptanceReceipt']['path'])==gate['acceptanceReceipt']['sha256']
    assert gate['acceptedJsonOwnerSha256']==sha((ORIGINAL/'inputs/source/src/content_integrity/json.rs').read_bytes())
    directory=args.out_dir.resolve();directory.mkdir(parents=True,exist_ok=False);records=[]
    receipt=dict(status='running_coordinator_owned_additive_model_revision',acceptedC1Gate=gate,acceptedC1GateSha256=sha(gate_raw),originalPackageIntegritySha256=package,revisionIntegritySha256=revision,lane=args.lane,evidenceScope='oracle scaffold sensitivity only; P2-P5 remain HELD; no production/default/RSS/conformance acceptance',commands=records)
    try:
        if args.lane=='models':
            for name in ['phase_model','frame_oracle']:original['execute']([args.python,str(ORIGINAL/(name+'.py'))],directory,name,records)
        result=original['execute']([args.python,str(HERE/'domain_oracle.py')],directory,'domain_oracle',records)
        actual=json.loads(result.stdout);expected=json.loads((HERE/'expected-selftest-records.json').read_text())
        assert actual==expected['expectedOutput'],'Output does not match separately frozen expectations'
        receipt['originalPackageUnchanged']=package==original['verify_package']()
        receipt['revisionUnchanged']=revision==verify_revision()
        assert receipt['originalPackageUnchanged'] and receipt['revisionUnchanged']
        receipt['status']='additive_oracle_scaffold_passed_not_production_acceptance'
    except Exception as error:
        receipt['status']='additive_oracle_scaffold_failed';receipt['failure']=str(error);raise
    finally:(directory/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--lane',choices=['domain','models'],default='domain');p.add_argument('--c1-gate',type=Path,required=True);p.add_argument('--python',default='python3');p.add_argument('--out-dir',type=Path,required=True);main(p.parse_args())
