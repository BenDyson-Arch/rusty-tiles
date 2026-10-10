#!/usr/bin/env python3
"""Bounded Linux F1d1 process-resource measurements of a frozen optimized CLI.

No Cargo operation, production import, generic scheduler or total-memory claim.
wait4 supplies process peak RSS; procfs and scratch peaks are sampled observations.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time
import zipfile

REPOSITORY=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(REPOSITORY/'tests'))
import f1d1_oracle as oracle


def sha256(path):
    result=hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda:stream.read(1024*1024),b''):result.update(block)
    return result.hexdigest()


def sample(pid,output_parent):
    result={'rss_kib':0,'threads':0,'file_descriptors':0,'scratch_files':0,'scratch_bytes':0}
    try:
        for line in (Path('/proc')/str(pid)/'status').read_text().splitlines():
            if line.startswith('VmRSS:'):result['rss_kib']=int(line.split()[1])
            if line.startswith('Threads:'):result['threads']=int(line.split()[1])
        result['file_descriptors']=len(list((Path('/proc')/str(pid)/'fd').iterdir()))
    except (FileNotFoundError,ProcessLookupError):pass
    if output_parent.exists():
        for path in output_parent.rglob('*'):
            if path.name=='result.3tz':continue
            try:
                if path.is_file():result['scratch_files']+=1;result['scratch_bytes']+=path.stat().st_size
            except FileNotFoundError:pass
    return result


def measure(binary,work,target,poll_ms,timeout):
    oracle.require(not work.exists(),'resource output directory must be fresh: '+str(work))
    work.mkdir(parents=True);source=work/'source.glb';source.write_bytes(oracle.fixture('grid',64))
    output=work/'output'/'result.3tz'
    command=[str(binary),'--json','mesh-local-to-3tz','-i',str(source),'-o',str(output),
        '--leaf-triangles','2048','--root-proxy-triangles',str(target),'--max-proxy-error-metres','100']
    peaks={key:0 for key in sample(-1,output.parent)};samples=0;usage=None;elapsed=None;timed_out=False
    with (work/'stdout.log').open('wb') as stdout,(work/'stderr.log').open('wb') as stderr:
        start=time.monotonic()
        process=subprocess.Popen(command,stdout=stdout,stderr=stderr,start_new_session=True)
        try:
            while True:
                observed=sample(process.pid,output.parent);samples+=1
                for key,value in observed.items():peaks[key]=max(peaks[key],value)
                pid,status,observed_usage=os.wait4(process.pid,os.WNOHANG)
                if pid:
                    process.returncode=os.waitstatus_to_exitcode(status);usage=observed_usage;elapsed=time.monotonic()-start;break
                if time.monotonic()-start>=timeout:
                    timed_out=True;os.killpg(process.pid,signal.SIGKILL)
                    _,status,usage=os.wait4(process.pid,0);process.returncode=os.waitstatus_to_exitcode(status);elapsed=time.monotonic()-start;break
                time.sleep(poll_ms/1000)
        finally:
            if process.returncode is None:
                try:os.killpg(process.pid,signal.SIGKILL)
                except ProcessLookupError:pass
                _,status,_=os.wait4(process.pid,0);process.returncode=os.waitstatus_to_exitcode(status)
    stdout=(work/'stdout.log').read_text();stderr=(work/'stderr.log').read_text()
    oracle.require(not timed_out,'bounded resource child timeout; killed and reaped: '+str(work))
    receipt={'command':command,'source_sha256':sha256(source),'source_bytes':source.stat().st_size,
        'triangle_limit':target,'exit_code':process.returncode,'elapsed_seconds':elapsed,
        'peak_rss_kib_wait4':usage.ru_maxrss,'user_cpu_seconds':usage.ru_utime,'system_cpu_seconds':usage.ru_stime,
        'poll_interval_ms':poll_ms,'samples':samples,'sampled_peaks':peaks,'stdout':stdout,'stderr':stderr,
        'timeout_seconds':timeout,'child_reaped':True}
    if target==1024:
        oracle.require(process.returncode==0,'near-limit accepted conversion '+stdout+stderr)
        with zipfile.ZipFile(output) as stream:
            oracle.require(len(stream.namelist())==len(set(stream.namelist())),'unique resource-case archive members')
            members={name:stream.read(name) for name in stream.namelist() if name!='@3dtilesIndex1@'}
        checked=oracle.inspect_members(source.read_bytes(),members,2048,target,100,exact_certificate=False)
        oracle.require(checked['source_triangles']==8192,'independently decoded source count')
        oracle.require(checked['comparison_work']==2*8192*checked['proxy_triangles'],'independent complete face-pair work')
        oracle.require(checked['comparison_work']>=.9*16777216,'accepted work close to declared ceiling')
        # The source covers a nonempty closed 64m square and every candidate
        # vertex belongs to it. Both nonempty supports lie in the same box.
        # Their Hausdorff distance is therefore at most its sqrt(8192) diameter.
        oracle.require(64**2+64**2<=100**2,'exact independent box-diameter bound within requested 100m')
        checked.update(independent_geometry_reference={'kind':'elementary_common_box_diameter','squared_diameter_metres2':8192,
            'requested_budget_squared_metres2':10000,'scope':'Actual source/candidate nonempty supports and original-position membership checked. Conservative whole-support distance <=sqrt(8192)<100m; detailed reported witness certificate not replayed at this size.'})
        receipt.update(artifact=checked,archive_sha256=sha256(output),archive_bytes=output.stat().st_size,
            comparison_ceiling_fraction=checked['comparison_work']/16777216,
            output_parent_entries_after_completion=sorted(path.name for path in output.parent.iterdir()))
        oracle.require(receipt['output_parent_entries_after_completion']==['result.3tz'],'no retained completed-case workspace/staging')
    else:
        oracle.require(process.returncode!=0,'pair-ceiling refusal required')
        response=json.loads(stdout)
        oracle.require(response['error'].get('kind',response['error'].get('code'))=='unsupported' and
            '16777216 complete face pairs' in response['error']['message'],'actual typed pair-admission refusal')
        oracle.require(not output.parent.exists(),'pair refusal before output parent/workspace/staging')
        receipt['output_parent_absent']=True
    return receipt


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,required=True);p.add_argument('--artifact-dir',type=Path,required=True)
    p.add_argument('--production-source',required=True);p.add_argument('--expected-binary-sha256',required=True)
    p.add_argument('--json-output',type=Path,required=True);p.add_argument('--poll-ms',type=int,default=10)
    p.add_argument('--timeout-seconds',type=int,default=180)
    a=p.parse_args();binary=a.binary.resolve(strict=True)
    oracle.require(platform.system()=='Linux','wait4/procfs evidence only supports Linux')
    oracle.require(1<=a.poll_ms<=100 and 1<=a.timeout_seconds<=180,'bounded resource measurement settings')
    actual_hash=sha256(binary);oracle.require(actual_hash==a.expected_binary_sha256,'frozen optimized binary identity')
    cases=[measure(binary,a.artifact_dir/'accepted-near-pair-ceiling',1024,a.poll_ms,a.timeout_seconds),
           measure(binary,a.artifact_dir/'refused-over-pair-ceiling',4096,a.poll_ms,a.timeout_seconds)]
    drivers=[Path(__file__),REPOSITORY/'tests/f1d1_oracle.py',REPOSITORY/'tests/f1c2_oracle.py']
    result={'production_source_commit':a.production_source,'binary_path':str(binary),'binary_sha256':actual_hash,
        'platform':platform.platform(),'python':sys.version,'cpu_count':os.cpu_count(),'invocation':[sys.executable,*sys.argv],
        'driver_sha256':{str(path.relative_to(REPOSITORY)):sha256(path) for path in drivers},'cases':cases,
        'wait4_peak_rss_semantics':'Raw child process ru_maxrss may include inherited Python launcher residency before exec. In particular, refusal launches after independent artifact decoding and can inherit a larger parent RSS. This is not pure steady-state converter RSS; procfs values remain sampled observations.',
        'limits':['One accepted and one refused process on this Linux host; no total-memory or cancellation-latency guarantee.',
            'wait4 peak RSS can include pre-exec inherited launcher memory; do not use it as pure converter allocation cost or compare accepted/refused values as such.',
            'wait4 process peak RSS is measured; descriptor/thread/scratch peaks sampled at stated interval and may miss short peaks.',
            'Detailed exact-rational face-pair witness certificate replay is omitted at this size; independent common-box diameter proves requested 100m geometric budget only.',
            'Measurements include source decode/candidate/certification/encoding/publication for this serial invocation; no constant-memory, streaming-total or performance guarantee.']}
    a.json_output.parent.mkdir(parents=True,exist_ok=True);a.json_output.write_text(json.dumps(result,indent=2,allow_nan=False)+'\n')


if __name__=='__main__':main()
