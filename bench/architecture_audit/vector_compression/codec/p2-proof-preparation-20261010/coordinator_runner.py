#!/usr/bin/env python3
"""Coordinator ONLY. All native/build execution requires an accepted C1 merge and exact supplied pins."""
import argparse,hashlib,json,os,re,subprocess,time,tomllib
from pathlib import Path
HERE=Path(__file__).resolve().parent
sha=lambda raw:hashlib.sha256(raw).hexdigest()
def file_sha(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as source:
        for block in iter(lambda:source.read(65536),b''):h.update(block)
    return h.hexdigest()
def verify_package():
    record=json.loads((HERE/'integrity.json').read_text())
    for name,pin in record['files'].items():
        raw=(HERE/name).read_bytes();assert len(raw)==pin['bytes'] and sha(raw)==pin['sha256'],name
    return sha((HERE/'integrity.json').read_bytes())
def controlled():
    os.setpriority(os.PRIO_PROCESS,0,10);os.sched_setaffinity(0,set(sorted(os.sched_getaffinity(0))[:2]))
def execute(command,directory,label,records):
    env=dict(os.environ,CARGO_BUILD_JOBS='2',RAYON_NUM_THREADS='2',RUST_TEST_THREADS='2');started=time.monotonic()
    result=subprocess.run(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env,preexec_fn=controlled,timeout=60,check=False)
    (directory/(label+'.stdout')).write_bytes(result.stdout);(directory/(label+'.stderr')).write_bytes(result.stderr)
    record=dict(label=label,command=command,exitCode=result.returncode,elapsedSeconds=time.monotonic()-started,stdoutSha256=sha(result.stdout),stderrSha256=sha(result.stderr));records.append(record)
    if result.returncode:raise RuntimeError(json.dumps(record))
    return result

def main(args):
    integrity=verify_package();gate_raw=args.c1_gate.read_bytes();gate=json.loads(gate_raw)
    assert gate['acceptanceStatus']=='accepted_merged_develop','P0 accepted C1 merge not supplied'
    assert re.fullmatch('[0-9a-f]{40}',gate['acceptedMergeCommit'])
    assert file_sha(gate['acceptanceReceipt']['path'])==gate['acceptanceReceipt']['sha256'],'Accepted C1 evidence receipt identity mismatch'
    helper=HERE/'inputs/source/src/content_integrity/json.rs'
    assert gate['acceptedJsonOwnerSha256']==sha(helper.read_bytes()),'Accepted JSON source changed: hold and separately repin/review; never silently regenerate predictions'
    directory=args.out_dir.resolve();directory.mkdir(parents=True,exist_ok=False);records=[];tool=None;artifact_pins={}
    receipt=dict(status='running_coordinator_owned_isolated_scaffold',acceptedC1Gate=gate,acceptedC1GateSha256=sha(gate_raw),packageIntegritySha256=integrity,lane=args.lane,evidenceScope='scaffolding only; no production codec/default/RSS/conformance acceptance',commands=records)
    try:
        if args.lane=='models':
            for source in ['phase_model.py','frame_oracle.py','domain_oracle.py']:
                execute([args.python,str(HERE/source)],directory,source.removesuffix('.py'),records)
        else:
            assert args.artifacts,'Explicit artifact/toolchain closure receipt required'
            tool=json.loads(args.artifacts.read_text());receipt['artifactReceiptSha256']=sha(args.artifacts.read_bytes());receipt['artifactReceipt']=tool
            assert tool['acceptedMergeCommit']==gate['acceptedMergeCommit']
            assert tool['cargoLockSha256']==sha((HERE/'inputs/source/Cargo.lock').read_bytes())
            assert tool['stdSourceMatchedToCompiler'] is True,'std HashSet/allocator backing must match actual compiler'
            assert {'raw_value','float_roundtrip','unbounded_depth'}<=set(tool['serdeJsonFeatures'])
            assert tool['dependencyClosureComplete'] is True,'Coordinator must pin every consumed dependency/proc-macro artifact and toolchain closure'
            lock=tomllib.loads((HERE/'inputs/source/Cargo.lock').read_text())
            expected={p['name']:p for p in lock['package'] if p['name'] in ['serde','serde_json','meshopt']}
            for name in ['serde','serde_json','meshopt']:
                assert tool['crates'][name]['version']==expected[name]['version'] and tool['crates'][name]['registryChecksum']==expected[name]['checksum']
            artifact_paths=[tool['rustc']]+[v['path']for v in tool['artifacts'].values()]
            artifact_hashes={tool['rustc']:tool['rustcSha256'],**{v['path']:v['sha256']for v in tool['artifacts'].values()}}
            for name in ['serde','serde_json','meshopt']:assert tool['crates'][name]['path'] in artifact_hashes,'Direct artifact missing from pinned closure'
            for path in artifact_paths:
                actual=file_sha(path);assert actual==artifact_hashes[path],path;artifact_pins[path]=actual
            vv=execute([tool['rustc'],'-vV'],directory,'rustc-vV',records).stdout.decode();assert vv==tool['rustcVV'];receipt['rustcVV']=vv
            probe='raw_probe' if args.lane=='raw' else 'ffi_probe';binary=directory/probe
            extern=['serde','serde_json']+(['meshopt'] if args.lane=='ffi' else [])
            command=[tool['rustc'],'--edition=2021','--crate-name',probe,'-C','opt-level=1','-L','dependency='+tool['dependencySearchDir']]
            for name in extern:command+=['--extern',name+'='+tool['crates'][name]['path']]
            command+=[str(HERE/'scaffolding'/(probe+'.rs')),'-o',str(binary)]
            execute(command,directory,'compile-'+probe,records);receipt['probeBinarySha256']=file_sha(binary)
            if args.lane=='raw':
                for family in ['admission','serializer']:
                    manifest=json.loads((HERE/'fixtures'/family/'manifest.json').read_text())
                    for case in manifest['cases']:execute([str(binary),str(HERE/'fixtures'/family/case['path'])],directory,family+'-'+case['name'],records)
            else:execute([str(binary),str(HERE/'fixtures/streams')],directory,'ffi-cases',records)
        receipt['packageUnchanged']=integrity==verify_package();receipt['artifactsUnchanged']=all(file_sha(path)==value for path,value in artifact_pins.items())
        assert receipt['packageUnchanged'] and receipt['artifactsUnchanged'];receipt['status']='isolated_scaffold_commands_passed_not_production_acceptance'
    except Exception as error:
        receipt['status']='isolated_scaffold_failed';receipt['failure']=str(error);raise
    finally:(directory/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--lane',choices=['models','raw','ffi'],required=True);p.add_argument('--c1-gate',type=Path,required=True);p.add_argument('--artifacts',type=Path);p.add_argument('--python',default='python3');p.add_argument('--out-dir',type=Path,required=True);main(p.parse_args())
