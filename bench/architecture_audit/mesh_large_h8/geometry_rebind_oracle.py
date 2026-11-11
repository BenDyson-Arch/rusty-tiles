#!/usr/bin/env python3
"""Strict all-member geometry/provenance rebinding to accepted 10m payloads.
No GLB JSON exception. Entire BIN and whole GLB bytes must remain identical.
Only two explicitly audited allocation-accounting receipt fields may change.
Any unknown difference refuses rebinding; rerun the full source geometry oracle.
"""
import argparse
import copy
import hashlib
import json
import pathlib
import struct

ROOT=pathlib.Path(__file__).resolve().parents[3]
LEGACY_ORACLE='8fa2fd4ce55103f3e34e413a3f698a4ab72e4aa8b06736e871a882a5d880cf22'
LEGACY_DRIVER='d9df5d5223a4b560622ba0d7038ccde069f42f363302e1a9604d0773e0d54465'
MESHOPT_DECODER=pathlib.Path('/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/meshoptimizer/meshopt_decoder.cjs')


def require(value,message):
    if not value: raise AssertionError(message)


def sha(path):
    path=pathlib.Path(path)
    result=hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda:stream.read(1<<20),b''):result.update(block)
    return result.hexdigest()


def inventory(root):
    records={}
    for path in sorted(root.rglob('*')):
        require(not path.is_symlink(),'inventory symlink refused')
        if path.is_file():records[path.relative_to(root).as_posix()]={'bytes':path.stat().st_size,'sha256':sha(path)}
    return records


def differences(a,b,path=''):
    """Typed strict JSON identity; at most eight bounded diagnostic paths."""
    if type(a) is not type(b):return [path+':type']
    if isinstance(a,dict):
        if a.keys()!=b.keys():return [path+':keys']
        result=[]
        for key in sorted(a):
            result.extend(differences(a[key],b[key],path+'/'+key))
            if len(result)>=8:return result[:8]
        return result
    if isinstance(a,list):
        if len(a)!=len(b):return [path+':length']
        result=[]
        for i,(x,y) in enumerate(zip(a,b)):
            result.extend(differences(x,y,path+'/'+str(i)))
            if len(result)>=8:return result[:8]
        return result
    if isinstance(a,float):
        return [] if struct.pack('<d',a)==struct.pack('<d',b) else [path]
    return [] if a==b else [path]


def identity(a,b,label):
    delta=differences(a,b)
    require(not delta,label+' differs at '+','.join(delta))


def read_glb(path):
    require(20<=path.stat().st_size<=96<<20,'physical GLB96MiB cap')
    raw=path.read_bytes();require(raw[:4]==b'glTF' and struct.unpack_from('<II',raw,4)==(2,len(raw)),'GLB2 header')
    length,kind=struct.unpack_from('<II',raw,12);require(kind==0x4e4f534a and length%4==0,'GLB JSON chunk')
    require(length<=1<<20,'bounded accepted10m leaf JSON domain')
    end=20+length;require(end+8<=len(raw),'GLB JSON closure')
    doc=json.loads(raw[20:end]);length,kind=struct.unpack_from('<II',raw,end)
    require(kind==0x004e4942 and length%4==0 and end+8+length==len(raw),'GLB BIN closure')
    return raw,doc,memoryview(raw)[end+8:]


def payload_identity(old_doc,old_bin,new_doc,new_bin):
    # JSON includes every geometry/attribute/material/sampler/image/extension
    # declaration and codec/TRS parameter. No field is removed or ignored.
    identity(old_doc,new_doc,'entire GLB JSON')
    require(old_bin==new_bin,'entire GLB BIN byte difference')


def receipt_identity(old,new):
    a,b=copy.deepcopy(old),copy.deepcopy(new);allowed=[]
    # Allocation lease only: does not alter any payload, range, error, density,
    # position bound, UV mapping, format floor, or provenance certificate.
    x,y=a['plannedFootprint']['planningOwnedBytes'],b['plannedFootprint']['planningOwnedBytes']
    require(type(x) is int and type(y) is int and 0<=x<=y<1<<64,'planned allocation lease shape/growth')
    if x!=y:
        allowed.append({'path':'/plannedFootprint/planningOwnedBytes','old':x,'new':y,'meaning':'explicit metadata/sample allocation lease growth'})
        b['plannedFootprint']['planningOwnedBytes']=x
    for i,(old_row,new_row) in enumerate(zip(a['appearance'],b['appearance'])):
        if 'quality' not in old_row or 'quality' not in new_row:continue
        old_quality,new_quality=old_row['quality'],new_row['quality']
        require(len(old_quality)==len(new_quality),'quality receipt inventory changed')
        # Only the LAST exact fixed-schema workspace accounting object. Never
        # remove numeric quality metrics, semantic MASK gap or source checks.
        accounting={'caller_levels','charged_bytes','effort','native_workspace_provisional','scalar_rrr'}
        old_charge,new_charge=old_quality[-1],new_quality[-1]
        require(set(old_charge)==accounting and set(new_charge)==accounting,'fixed workspace receipt schema')
        if old_charge['charged_bytes']!=new_charge['charged_bytes']:
            w,h=old_row['width'],old_row['height'];base=w*h;texels=0;levels=0
            while True:
                texels+=w*h;levels+=1
                if w==h==1:break
                w,h=max(1,w//2),max(1,h//2)
            require(old_charge['caller_levels']==new_charge['caller_levels']==old_row['levels']==levels,'workspace mip accounting')
            require(old_charge['charged_bytes']==128*texels+(64<<20)+8*base
                    and new_charge['charged_bytes']==132*texels+(64<<20)+8*base,'only known decoded-chain workspace correction permitted')
            allowed.append({'path':f'/appearance/{i}/quality/{len(old_quality)-1}/charged_bytes','old':old_charge['charged_bytes'],'new':new_charge['charged_bytes'],'meaning':'128→132 per complete-chain texel workspace allocation charge'})
            new_charge['charged_bytes']=old_charge['charged_bytes']
    identity(a,b,'leaf certificate/receipt semantic fields')
    return allowed


def rows(path):
    expected=0;uris=set()
    with path.open() as stream:
        for line in stream:
            row=json.loads(line);uri=row['uri'];parts=pathlib.PurePosixPath(uri).parts
            require(parts and parts[0]=='leaves' and '..'not in parts and not uri.startswith('/') and uri.endswith('.glb'),'member URI domain')
            require(uri not in uris and row['first']==expected and row['end']>row['first'],'member provenance/rank closure')
            uris.add(uri);expected=row['end'];yield row
    require(expected==10_000_000 and len(uris)==1024,'required accepted10m/1024member closure')


def controls(doc,bin_data,receipt):
    mutant=bytearray(bin_data);require(mutant,'BIN control nonempty');mutant[0]^=1
    try:payload_identity(doc,bin_data,doc,mutant)
    except AssertionError:pass
    else:raise AssertionError('BIN mutation insensitive')
    altered=copy.deepcopy(doc);accessor=altered['meshes'][0]['primitives'][0]['attributes']['POSITION']
    altered['accessors'][accessor]['count']+=1
    try:payload_identity(doc,bin_data,altered,bin_data)
    except AssertionError:pass
    else:raise AssertionError('POSITION metadata mutation insensitive')
    altered=copy.deepcopy(receipt);altered['geometryError']+=1
    try:receipt_identity(receipt,altered)
    except AssertionError:pass
    else:raise AssertionError('geometry certificate mutation insensitive')
    return ['BIN-byte','POSITION-accessor-metadata','geometryError-receipt']


def verify(truth_path,old,new,old_oracle_path,old_resource_path,new_resource_path,run_controls):
    truth=json.loads(truth_path.read_text());accepted=json.loads(old_oracle_path.read_text())
    require(accepted['verdict']=='PASS' and accepted['controlsExecuted'] is True and accepted['triangles']==10_000_000 and accepted['leaves']==1024,'accepted old geometry oracle shape')
    require(accepted['oracleSha256']==LEGACY_ORACLE and accepted['driverSha256']==LEGACY_DRIVER,'pinned accepted source geometry oracle/driver')
    require(sha(MESHOPT_DECODER)==accepted['meshoptDecoderSha256'],'same pinned lossless meshopt decoder semantics')
    old_payload={k:v for k,v in accepted.items() if k!='passHash'}
    require(hashlib.sha256(json.dumps(old_payload,sort_keys=True,separators=(',',':')).encode()).hexdigest()==accepted['passHash'],'accepted old oracle passHash')
    require(sha(truth_path)==accepted['truthSha256'] and sha(old/'manifest.json')==accepted['manifestSha256'] and sha(old/'leaves.jsonl')==accepted['leafManifestSha256'],'old accepted source/artifact inventory binding')
    source_inventory=inventory(truth_path.parent)
    require(sha(truth_path.parent/'source.gltf')==truth['inputSha256'],'current exact source glTF')
    for name,digest in truth['assets'].items():require(source_inventory[name]['sha256']==digest,'current exact source payload/image')
    resources=[]
    for path in (old_resource_path,new_resource_path):
        resource=json.loads(path.read_text());identity(resource['inputs_before'],source_inventory,'actualrun source before inventory');identity(resource['inputs_after'],source_inventory,'actualrun source after inventory')
        require(resource['binary_unchanged'] is True and resource['inputs_unchanged'] is True and resource['exit_code']==0,'actualrun immutable source/binary success')
        resources.append({'receiptSha256':sha(path),'binarySha256':resource['binary_sha256_before'],'resourceGatePassed':resource['passed']})
    # Old resource runner had a known unrelated-sibling FAIL. Rebinding does not
    # turn that receipt into a resource PASS or accept final performance here.
    old_manifest=json.loads((old/'manifest.json').read_text());new_manifest=json.loads((new/'manifest.json').read_text());identity(old_manifest,new_manifest,'entire payload manifest')
    old_inventory=inventory(old);new_inventory=inventory(new);identity(sorted(old_inventory),sorted(new_inventory),'complete output file inventory')
    members=[];accounting=[];proof_controls=[];total_bytes=0
    old_rows,new_rows=iter(rows(old/'leaves.jsonl')),iter(rows(new/'leaves.jsonl'))
    for ordinal in range(1024):
        a,b=next(old_rows),next(new_rows);require(a['uri']==b['uri'],'same allmember URI/rank order')
        uri=a['uri'];old_path,new_path=old/uri,new/uri
        for path,row,items in ((old_path,a,old_inventory),(new_path,b,new_inventory)):
            require(items[uri]=={'bytes':row['bytes'],'sha256':row['sha256']},'actual member hash/bytes certificate')
        accounting.extend({'uri':uri,**delta} for delta in receipt_identity(a,b))
        old_raw,old_doc,old_bin=read_glb(old_path);new_raw,new_doc,new_bin=read_glb(new_path)
        payload_identity(old_doc,old_bin,new_doc,new_bin)
        require(old_raw==new_raw,'unknown GLB serialization difference; strict rebinding refuses')
        if run_controls and not proof_controls:proof_controls=controls(old_doc,old_bin,a)
        total_bytes+=len(new_raw);members.append({'uri':uri,'oldSha256':a['sha256'],'newSha256':b['sha256'],'bytes':len(new_raw),'binSha256':hashlib.sha256(new_bin).hexdigest(),'decodedJSONSha256':hashlib.sha256(json.dumps(new_doc,sort_keys=True,separators=(',',':')).encode()).hexdigest()})
        del old_raw,old_doc,old_bin,new_raw,new_doc,new_bin
    require(next(old_rows,None)is None and next(new_rows,None)is None,'both complete member EOF')
    allowed_files={'manifest.json','leaves.jsonl','resources.json',*(member['uri'] for member in members)}
    require(set(old_inventory)==set(new_inventory)==allowed_files,'only declared complete leafpayload/debug files')
    require(total_bytes==new_manifest['report']['glb_bytes'],'allmember totalbyte report')
    result={'verdict':'PASS','scope':'Strict complete geometry/attributes/provenance payload rebinding only; retains accepted old limitations','triangles':10_000_000,'leaves':1024,'glbBytes':total_bytes,'controls':proof_controls,'oldAcceptedOracleSha256':sha(old_oracle_path),'oldAcceptedPassHash':accepted['passHash'],'sourceInventory':source_inventory,'runSourceBindings':resources,'oldArtifactInventory':old_inventory,'newArtifactInventory':new_inventory,'members':members,'explicitAccountingDifferences':accounting,'oracleSha256':sha(__file__),'rendererAcceptance':False,'decodedTextureAcceptance':False,'scaleSeamScope':accepted['scaleSeamScope'],'sourceEffectiveNormalAcceptance':False,'oldResourceFailurePreserved':not resources[0]['resourceGatePassed']}
    result['passHash']=hashlib.sha256(json.dumps(result,sort_keys=True,separators=(',',':')).encode()).hexdigest();return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for field in ('truth','old_output','new_output','old_oracle','old_resource','new_resource'):parser.add_argument(field,type=pathlib.Path)
    parser.add_argument('--controls',action='store_true');parser.add_argument('--report',type=pathlib.Path,required=True);args=parser.parse_args()
    require(not args.report.resolve().is_relative_to(ROOT),'evidence report cache-only')
    result=verify(args.truth,args.old_output,args.new_output,args.old_oracle,args.old_resource,args.new_resource,args.controls)
    args.report.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:result[k] for k in ('verdict','triangles','leaves','glbBytes','controls','passHash')},sort_keys=True))


if __name__=='__main__':main()
