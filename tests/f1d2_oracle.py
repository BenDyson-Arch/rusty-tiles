#!/usr/bin/env python3
"""Bounded F1d2 artifact execution and independent certificate sensitivity.

Source fixtures and all artifact geometry truth are authored independently.
No private Rust geometry or optimizer is called by the checker.
"""
import argparse
import copy
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import platform
import struct
import subprocess
import time
import zipfile

import f1d1_oracle as artifact
import f1d2_certificate as exact


def self_test():
    base=[[(0,0,0),(8,0,0),(0,8,0)]]
    quarter=list(exact.children(exact.exact_face(base[0])))
    old=artifact.ideal_face_certificate_squared(base,quarter)
    cases=[('equal-retriangulation',base,quarter,F(0))]
    shifted=[[(x,y,z+F(1,8)) for x,y,z in face] for face in quarter]
    cases.append(('positive-offset',base,shifted,F(1,8)))
    cases.append(('reordered-positive-offset',list(reversed(base)),list(reversed(shifted)),F(1,8)))
    cases.append(('spike',base,[[(0,0,0),(8,0,0),(0,8,100)]],F(100)))
    cases.append(('degenerate', [[(3,4,0)]*3],[[(0,0,0)]*3],F(5)))
    thin=[[(999999,0,0),(1000000,0,0),(999999,F(1,2**30),0)]]
    lifted=[[(x,y,z+F(1,2**30)) for x,y,z in f] for f in thin]
    cases.append(('mixed-thin',thin,lifted,F(1,2**30)))
    positives=[];controls=[]
    for label,left,right,bound in cases:
        proof=exact.certify_regions([(left,right)],bound)
        positives.append({'case':label,'certificate':str(bound),'proof':proof})
        if bound:
            try:exact.certify_regions([(left,right)],F(0))
            except exact.CertificateUnproven as error:controls.append({'control':label+'-false-zero','accepted':False,'classification':'unproven','reason':str(error)})
            else:raise AssertionError('insensitive false zero '+label)
    artifact.require(old==16 and positives[0]['proof']['ideal_cover_squared']=='0','historical 4m bound tightens to zero')
    for label,options in [('too-tight-positive',{'certificate':F(1,16)}),('work-cap',{'certificate':F(0),'max_tests':1}),('depth-cap',{'certificate':F(0),'max_depth':0}),('time-cap',{'certificate':F(0),'max_seconds':1e-12})]:
        try:exact.certify_regions([(base,shifted if label=='too-tight-positive' else quarter)],**options)
        except exact.CertificateUnproven as error:controls.append({'control':label,'accepted':False,'classification':'unproven','reason':str(error)})
        else:raise AssertionError('insensitive '+label)
    # Published-artifact hole control: source is a boundary ring, proxy fills it.
    # All original/proxy vertices are on the ring, so vertex-only zero passes.
    source=artifact.fixture('boundary-ring');members=artifact.synthetic_members(source)
    report=json.loads(members['conversion.json'])
    report['approximation']['certificate']={'error_metres':3,'patch_face_tests':96,'accepted_patches':16,'max_depth':1}
    members['conversion.json']=json.dumps(report).encode()
    positive=artifact.inspect_members(source,members,100,16,8)
    changed=dict(members);report['approximation']['certificate']['error_metres']=0
    changed['conversion.json']=json.dumps(report).encode()
    try:artifact.inspect_members(source,changed,100,16,8)
    except exact.CertificateUnproven as error:controls.append({'control':'published-hole-false-zero','accepted':False,'classification':'unproven','reason':str(error),'analytic_center_distance_metres':3})
    else:raise AssertionError('insensitive interior-hole zero')
    controls.extend(artifact.corruption_controls(source,members,100,16,8))
    published=positive['report'];stdout={'ok':True,'meshReport':copy.deepcopy(published)}
    artifact.inspect_cli_report(json.dumps(stdout),published)
    for label,mutate in [('stale-cli-certificate',lambda x:x['meshReport']['approximation']['certificate'].update(error_metres=0)),('missing-cli-report',lambda x:x.pop('meshReport')),('false-cli-success',lambda x:x.update(ok=False)),('wrong-cli-profile',lambda x:x['meshReport'].update(profile='f1d1-root-proxy-gltf-v1')),('boolean-cli-metric',lambda x:x['meshReport']['approximation']['certificate'].update(max_depth=True))]:
        changed=copy.deepcopy(stdout);mutate(changed)
        try:artifact.inspect_cli_report(json.dumps(changed),published)
        except artifact.leaf_oracle.OracleError as error:controls.append({'control':label,'accepted':False,'classification':'mismatched adapter result','reason':str(error)})
        else:raise AssertionError('insensitive CLI report '+label)

    return {'scope':'Independent exact-cover and synthetic artifact sensitivity only; no producer acceptance',
            'historical_retriangulation_squared':str(old),'exact_positive_cases':positives,
            'hole_artifact':positive,'controls':controls,'passed':True}


def archive_members(path):
    with zipfile.ZipFile(path) as stream:
        artifact.require(len(stream.namelist())==len(set(stream.namelist())),'unique archive members')
        return {n:stream.read(n) for n in stream.namelist() if n!='@3dtilesIndex1@'}


def execute(binary,root,variant,limit,budget,*,n=4,expected='success'):
    root.mkdir(parents=True,exist_ok=True)
    source=artifact.fixture(variant,n);input_path=root/'source.glb';input_path.write_bytes(source)
    output=root/'result.3tz' if expected=='success' else root/'absent-parent'/'result.3tz'
    command=[str(binary),'--json','mesh-local-to-3tz','-i',str(input_path),'-o',str(output),
             '--leaf-triangles','16','--root-proxy-triangles',str(limit),'--max-proxy-error-metres',str(budget)]
    started=time.monotonic();process=subprocess.run(command,capture_output=True,text=True,timeout=120)
    receipt={'command':command,'exit_code':process.returncode,'stdout':process.stdout,'stderr':process.stderr,
             'source_sha256':artifact.digest(source),'elapsed_seconds':round(time.monotonic()-started,6)}
    if expected=='success':
        artifact.require(process.returncode==0,'F1d2 positive '+process.stdout+process.stderr)
        members=archive_members(output);proof=artifact.inspect_members(source,members,16,limit,budget)
        cli_report=artifact.inspect_cli_report(process.stdout,proof['report'])
        receipt.update(artifact=proof,archive_sha256=artifact.digest(output.read_bytes()),cli_report=cli_report,
                       controls=artifact.corruption_controls(source,members,16,limit,budget))
    else:
        artifact.require(process.returncode!=0,'refusal required '+variant)
        response=json.loads(process.stdout)
        artifact.require(response['error'].get('kind',response['error'].get('code'))=='unsupported','typed bounded refusal')
        artifact.require(not output.parent.exists(),'refusal before workspace/output parent')
        receipt['output_parent_absent']=True
        existing=root/'preserved.3tz';sentinel=b'authored existing destination: preserve on bounded refusal\n'
        existing.write_bytes(sentinel)
        replaced=list(command);replaced[replaced.index('-o')+1]=str(existing);replaced.append('--force')
        before={p.name for p in root.iterdir()}
        second=subprocess.run(replaced,capture_output=True,text=True,timeout=120)
        second_response=json.loads(second.stdout)
        artifact.require(second.returncode!=0 and second_response.get('ok') is False,'Replace result is a refusal')
        artifact.require(second_response['error'].get('kind',second_response['error'].get('code'))=='unsupported','Replace refusal has same Unsupported kind')
        artifact.require(second_response['error']['message']==response['error']['message'],'Replace refusal exercises the same certificate gate')
        artifact.require(existing.read_bytes()==sentinel,'Replace refusal preserves existing destination')
        artifact.require({p.name for p in root.iterdir()}==before,'Replace refusal leaves no staging files')
        receipt['replace_preservation']={'command':replaced,'exit_code':second.returncode,'stdout':second.stdout,'stderr':second.stderr,'destination_sha256':artifact.digest(sentinel),'directory_unchanged':True}
    return receipt


def run(binary,artifacts):
    binary=binary.resolve(strict=True)
    broad=artifact.run(binary,artifacts/'baseline')
    tight=execute(binary,artifacts/'tight-grid','grid',16,.5)
    actual=tight['artifact']['report']['approximation']['certificate']['error_metres']
    old=F(tight['artifact']['historical_whole_face_squared'])
    artifact.require(F(.5)**2<old and F(actual)**2<old,'useful actual artifact succeeds below independent old whole-face certificate')
    # A small bump can be discarded by an untrusted proposal; the subsequent
    # exact artifact checker must establish its real nonzero distance bound.
    nonzero=execute(binary,artifacts/'nonzero-bump','bump',2,.5)
    artifact.require(nonzero['artifact']['report']['approximation']['certificate']['error_metres']>0,'nonzero emitted bound fixture')
    budget_refusal=execute(binary,artifacts/'refused-nonzero-proposal','bump',2,.125,expected='refusal')
    artifact.require('proposal did not achieve the requested actual reduction' in json.loads(budget_refusal['stdout'])['error']['message'],'exercise actual budget-sensitive proposal refusal')
    budget_refusal['observed_gate']='budget-sensitive candidate proposal; no depth/work certificate gate claimed'
    return {'binary_path':str(binary),'binary_sha256':artifact.digest(binary.read_bytes()),
            'baseline':broad,'useful_tightening':tight,'nonzero_artifact':nonzero,'nonzero_budget_refusal':budget_refusal,
            'scope':'Actual bounded F1d2 reductions independently decoded and re-certified; no topology/appearance/world-distance/recursive acceptance'}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--self-test',action='store_true');p.add_argument('--binary',type=Path)
    p.add_argument('--artifact-dir',type=Path);p.add_argument('--json-output',type=Path)
    args=p.parse_args();result={}
    if args.self_test:result['self_test']=self_test()
    if args.binary:
        artifact.require(args.artifact_dir,'artifact dir required')
        result['execution']=run(args.binary,args.artifact_dir)
    paths=[Path(__file__),Path(artifact.__file__),Path(exact.__file__),Path(artifact.leaf_oracle.__file__)]
    result['driver_sha256']={x.name:artifact.digest(x.read_bytes()) for x in paths}
    result['python']=platform.python_version()
    text=json.dumps(result,indent=2,allow_nan=False)+'\n'
    if args.json_output:args.json_output.parent.mkdir(parents=True,exist_ok=True);args.json_output.write_text(text)
    else:print(text,end='')


if __name__=='__main__':main()
