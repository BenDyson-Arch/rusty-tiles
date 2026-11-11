#!/usr/bin/env python3
"""Only run coordinator-supplied exact hash-pinned CLI; never build or mutate fixtures."""
import argparse, copy, hashlib, json, os, subprocess, time
from pathlib import Path
HERE=Path(__file__).resolve().parent
FIXTURES=HERE/'fixtures'
def sha(raw): return hashlib.sha256(raw).hexdigest()
def snapshot(root): return {str(p.relative_to(root)):sha(p.read_bytes()) for p in sorted(root.rglob('*')) if p.is_file()}
def controlled_launch():
    os.setpriority(os.PRIO_PROCESS,0,10)
    os.sched_setaffinity(0,set(sorted(os.sched_getaffinity(0))[:2]))
def run(args):
    binary=args.binary.resolve(); pin=args.source_pin.resolve()
    assert sha(binary.read_bytes())==args.binary_sha256,'binary hash mismatch'
    assert sha(pin.read_bytes())==args.source_pin_sha256,'source pin hash mismatch'
    manifest=json.loads((FIXTURES/'manifest.json').read_text())
    assert manifest['runnerSha256']==sha(Path(__file__).read_bytes()),'runner hash mismatch'
    assert manifest['generatorSha256']==sha((HERE/'generator.py').read_bytes()),'generator hash mismatch'
    for ref in manifest['referencePins']:
        raw=(HERE/'primary'/ref['file']).read_bytes(); assert sha(raw)==ref['sha256'] and len(raw)==ref['bytes']
    before=snapshot(FIXTURES); references_before=snapshot(HERE/'primary')
    args.run_dir.mkdir(parents=True,exist_ok=False); records=[]
    env=dict(os.environ,RAYON_NUM_THREADS='2',RUST_TEST_THREADS='2',CARGO_BUILD_JOBS='2')
    for case in manifest['cases']:
        path=(FIXTURES/case['path']).resolve(); assert sha(path.read_bytes())==case['sha256']
        command=[str(binary),'--json','validate',str(path)]; started=time.monotonic()
        result=subprocess.run(command,env=env,preexec_fn=controlled_launch,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=20,check=False)
        (args.run_dir/(case['name']+'.stdout')).write_bytes(result.stdout); (args.run_dir/(case['name']+'.stderr')).write_bytes(result.stderr)
        try: response=json.loads(result.stdout)
        except (ValueError,UnicodeError): response=None
        observed='admitted' if result.returncode==0 else (response or {}).get('error',{}).get('code','unknown')
        expected=copy.deepcopy(case['expectedReport'])
        if expected: expected['archive']=str(path)
        passed=result.returncode==case['expectedExitCode'] and observed==case['expectedCategory'] and not result.stderr and (expected is None or expected==response)
        records.append(dict(name=case['name'],passed=passed,expectedCategory=case['expectedCategory'],observedCategory=observed,expectedReport=expected,response=response,exitCode=result.returncode,command=command,elapsedSeconds=time.monotonic()-started,stdoutSha256=sha(result.stdout),stderrSha256=sha(result.stderr),archiveSha256=case['sha256']))
        print(json.dumps(dict(name=case['name'],passed=passed,expected=case['expectedCategory'],observed=observed)),flush=True)
    immutable=before==snapshot(FIXTURES) and references_before==snapshot(HERE/'primary')
    identities=sha(binary.read_bytes())==args.binary_sha256 and sha(pin.read_bytes())==args.source_pin_sha256
    receipt=dict(schemaVersion=1,binary=str(binary),binarySha256=args.binary_sha256,sourcePinSha256=args.source_pin_sha256,sourcePin=json.loads(pin.read_text()),manifestSha256=sha((FIXTURES/'manifest.json').read_bytes()),runnerSha256=sha(Path(__file__).read_bytes()),fixturesBefore=before,fixturesAndReferencesUnchanged=immutable,artifactIdentitiesUnchanged=identities,cases=records,passes=sum(r['passed'] for r in records),failures=sum(not r['passed'] for r in records),executionPolicy=dict(nice=10,workerEnv=2,affinity='first two allowed CPUs'))
    (args.run_dir/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
    return 0 if immutable and identities and all(r['passed'] for r in records) else 1
if __name__=='__main__':
    p=argparse.ArgumentParser(); p.add_argument('--binary',type=Path,required=True); p.add_argument('--binary-sha256',required=True); p.add_argument('--source-pin',type=Path,required=True); p.add_argument('--source-pin-sha256',required=True); p.add_argument('--run-dir',type=Path,required=True)
    raise SystemExit(run(p.parse_args()))
