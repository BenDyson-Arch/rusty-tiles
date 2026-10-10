#!/usr/bin/env python3
"""Root coordinator ONLY, after immutable source review. No author execution."""
from pathlib import Path
import argparse,hashlib,json,runpy,sys
HERE=Path(__file__).resolve().parent
OLD=Path('/tmp/rusty-tiles-next-vector-compression-audit/codec/p2-proof-preparation-20261010')
def sha(p):
    h=hashlib.sha256()
    with Path(p).open('rb') as s:
        for block in iter(lambda:s.read(65536),b''):h.update(block)
    return h.hexdigest()
def verify():
    for manifest,local in [('integrity.json',True),('input-pin.json',False)]:
        for name,pin in json.loads((HERE/manifest).read_text())['files'].items():
            p=HERE/name if local else Path(name)
            assert p.stat().st_size==pin['bytes'] and sha(p)==pin['sha256'],str(p)
    return sha(HERE/'integrity.json')
def main(out):
    integrity=verify();old=runpy.run_path(str(OLD/'coordinator_runner.py'))
    gate=json.loads(Path('/tmp/rusty-tiles-codec-p2-executions/accepted-c1-gate.json').read_text())
    tool=json.loads(Path('/tmp/rusty-tiles-codec-p2-executions/exact-artifact-receipt.json').read_text())
    assert gate['acceptanceStatus']=='accepted_merged_develop'
    assert gate['acceptedMergeCommit']==tool['acceptedMergeCommit']=='f5401dd120441b4f9aac5ad229dbfffe03fe4428'
    assert gate['acceptedJsonOwnerSha256']==sha(OLD/'inputs/source/src/content_integrity/json.rs')
    assert tool['stdSourceMatchedToCompiler'] and tool['dependencyClosureComplete']
    assert {'raw_value','float_roundtrip','unbounded_depth'}<=set(tool['serdeJsonFeatures'])
    artifact_pins={tool['rustc']:tool['rustcSha256'],**{p['path']:p['sha256'] for p in tool['artifacts'].values()}}
    for path,value in artifact_pins.items():assert sha(path)==value,path
    out=out.resolve();out.mkdir(parents=True,exist_ok=False);records=[]
    receipt={'status':'running_isolated_owned_plan','packageIntegritySha256':integrity,'evidenceScope':'this prototype only; production/full semantics/defaults held','commands':records}
    try:
        vv=old['execute']([tool['rustc'],'-vV'],out,'rustc-vV',records).stdout.decode();assert vv==tool['rustcVV']
        commands=json.loads((HERE/'commands.json').read_text());compile=commands['compile'];binary=out/'owned_plan_probe'
        args=[a.replace('<FRESH_OUTPUT>',str(out)) for a in compile['args']]
        old['execute']([compile['rustc'],*args],out,'compile-owned-plan',records);receipt['binarySha256']=sha(binary)
        for c in commands['commands']:old['execute']([str(binary),*c['args']],out,c['label'],records)
        old['execute']([sys.executable,str(HERE/'check_records.py'),str(out)],out,'independent-record-check',records)
        assert verify()==integrity
        for path,value in artifact_pins.items():assert sha(path)==value,path
        receipt['status']='actual_isolated_prototype_commands_and_independent_records_passed_not_production_acceptance'
    except Exception as error:
        receipt['status']='actual_isolated_prototype_failed';receipt['failure']=str(error);raise
    finally:(out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--out-dir',type=Path,required=True);main(p.parse_args().out_dir)
