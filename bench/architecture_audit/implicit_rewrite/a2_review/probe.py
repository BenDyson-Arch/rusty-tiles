#!/usr/bin/env python3
"""Nonauthor bounded review: models, mandatory rows, ancestor coherence, pins.

No Cargo, converter execution, Git mutation or production change. Copies the
authored semantic model into a fresh temporary skeleton before its replay.
"""
import base64
import copy
from fractions import Fraction as F
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import urllib.request

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
AUDIT = HERE.parent


def sha(data):
    return hashlib.sha256(data).hexdigest()


def load_module(path):
    spec = importlib.util.spec_from_file_location('reviewed_model', path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def multiply(a, b):
    return [sum(F(a[r+4*k])*F(b[k+4*c]) for k in range(4))
            for c in range(4) for r in range(4)]


def inverse(m):
    rows = [[F(m[r+4*c]) for c in range(4)] + [F(r == c) for c in range(4)] for r in range(4)]
    for c in range(4):
        pivot = next(r for r in range(c,4) if rows[r][c])
        rows[c],rows[pivot] = rows[pivot],rows[c]
        d = rows[c][c]; rows[c] = [x/d for x in rows[c]]
        for r in range(4):
            if r != c:
                d = rows[r][c]; rows[r] = [x-d*y for x,y in zip(rows[r],rows[c])]
    return [rows[r][4+c] for c in range(4) for r in range(4)]


def point(m,p):
    return [sum(F(m[r+4*k])*F((list(p)+[1])[k]) for k in range(4)) for r in range(3)]


def coherence(source):
    """Separate exact oracle for these authored axis-aligned local fixtures."""
    nodes = {n['id']:n for n in source['explicit_nodes']}; failures=[]; checked=0
    identity = [int(i%5==0) for i in range(16)]
    def visit(nid,parent,ancestors):
        nonlocal checked
        n = nodes[nid]; frame = multiply(parent,n['transform'])
        chain = ancestors + [(n,frame)]
        for pid in n['payloads']:
            world = point(frame,source['points'][pid])
            for owner,m in chain:
                local = point(inverse(m),world); b = owner['bounds']; checked += 1
                if any(abs(local[a]-F(b[a])) > F(b[3+4*a]) for a in range(3)):
                    failures.append({'payload':pid,'owner':owner['id']})
        for child in n['children']: visit(child,frame,chain)
    visit('n0',identity,[])
    return {'containment_checks':checked,'violations':failures}


def review_models(work):
    skeleton=work/'skeleton'; target=skeleton/'bench/architecture_audit/implicit_rewrite/a2_semantics'
    shutil.copytree(AUDIT/'a2_semantics',target,ignore=shutil.ignore_patterns('__pycache__'))
    records=json.loads((target/'results.json').read_bytes())
    for name,digest in records['inspected_source_sha256'].items():
        data=(ROOT/name).read_bytes(); assert sha(data)==digest
        dst=skeleton/name; dst.parent.mkdir(parents=True,exist_ok=True); dst.write_bytes(data)
    result=subprocess.run(['nice','-n','10',sys.executable,'-B',str(target/'probe.py')],capture_output=True,text=True,timeout=20)
    assert result.returncode==0,result.stderr
    capture=subprocess.run(['nice','-n','10',sys.executable,'-B',str(AUDIT/'a2_capture_resources/probe.py'),str(work/'capture')],capture_output=True,text=True,timeout=20)
    assert capture.returncode==0,capture.stderr
    cap=json.loads((work/'capture/receipt.json').read_bytes())
    oldcap=json.loads((AUDIT/'a2_capture_resources/results.json').read_bytes())
    # Runtime measurements/paths differ; deterministic evidence must agree.
    for key in ('driver_sha256','fixture_sha256'): assert cap[key]==oldcap[key]
    stable=lambda rows:[{k:v for k,v in x.items() if not k.startswith('tracemalloc_')} for x in rows]
    assert stable(cap['cases'])==stable(oldcap['cases'])
    m=load_module(target/'probe.py'); supplementary=[]
    for scheme in ('QUADTREE','OCTREE'):
        source,members=m.fixture(scheme,'source-child-order')
        good=coherence(source); assert not good['violations']
        supplementary.append({'scheme':scheme,'case':'all_payloads_in_every_ancestor','result':good})
        for property_name in ('bounds','error'):
            changed=copy.deepcopy(members)
            def remove(doc,binary): del doc['propertyTables'][0]['properties'][property_name]
            m.mutate_subtree(changed,'n0-0-0-0'+('-0' if scheme=='OCTREE' else '')+'.subtree',remove)
            try: m.oracle(source,changed)
            except (KeyError,ValueError) as error:
                supplementary.append({'scheme':scheme,'case':'missing_authoritative_'+property_name,'rejected':True,'cause':str(error)})
            else: raise AssertionError('missing row accepted')
        # Preserve source/output equality while moving one descendant outside
        # its ancestor. This demonstrates the semantic model's declared limit.
        badsource=copy.deepcopy(source); badmembers=copy.deepcopy(members)
        badsource['explicit_nodes'][2]['transform'][12] += 4
        before=members['n2.json']; after=json.loads(before); after['root']['transform'][12] += 4
        for name,data in list(badmembers.items()):
            if data==before: badmembers[name]=m.json_bytes(after)
        def move(doc,binary):
            view=doc['bufferViews'][doc['propertyTables'][0]['properties']['bounds']['values']]
            at=view['byteOffset']+12*8
            struct.pack_into('<d',binary,at,struct.unpack_from('<d',binary,at)[0]+4)
        m.mutate_subtree(badmembers,'n1-0-0-0'+('-0' if scheme=='OCTREE' else '')+'.subtree',move)
        m.oracle(badsource,badmembers)
        bad=coherence(badsource); assert bad['violations']
        supplementary.append({'scheme':scheme,'case':'self_consistent_descendant_outside_ancestor','authored_pair_oracle':'passes','separate_coherence_oracle':'rejects','result':bad})
    return {'semantic_replay':json.loads(result.stdout),'capture_replay_cases':len(cap['cases']),
            'capture_receipt_sha256':sha((work/'capture/receipt.json').read_bytes()),'supplementary':supplementary}


def archive_preservation():
    directory=ROOT/'bench/architecture_audit/mesh_approximation_f1d2/remote_acceptance'
    index=json.loads((directory/'storage-index.json').read_bytes()); checked=[]
    for entry in index['entries']:
        data=(directory/entry['stored_path']).read_bytes(); raw=gzip.decompress(data)
        assert len(data)==entry['stored_bytes'] and sha(data)==entry['stored_sha256']
        assert len(raw)==entry['original_bytes'] and sha(raw)==entry['original_sha256']
        original=Path(entry['original_path']); assert original.read_bytes()==raw
        checked.append(entry['stored_path'])
    return {'entries_verified':len(checked),'index_sha256':sha((directory/'storage-index.json').read_bytes()),'stored_and_expanded_bytes_agree':True}


def primary_pins(work):
    pins=json.loads((AUDIT/'a2_semantics/sources.json').read_bytes()); checked=[]
    for i,entry in enumerate(pins['sources']):
        with urllib.request.urlopen(entry['url'],timeout=20) as response: raw=response.read(200001)
        assert len(raw)==entry['bytes'] and sha(raw)==entry['sha256'],entry['url']
        (work/f'primary-{i}.txt').write_bytes(raw)
        checked.append({'url':entry['url'],'bytes':len(raw),'sha256':sha(raw)})
    return checked


def main():
    assert not sys.flags.optimize
    work=Path(sys.argv[1]).resolve(); assert str(work).startswith('/tmp/')
    work.mkdir(exist_ok=False)
    result={'evidence':'independent nonauthor bounded model replay and byte-binding review; no production execution',
            'driver_sha256':sha(Path(__file__).read_bytes()),'models':review_models(work),
            'f1d2_preservation':archive_preservation(),'primary_sources':primary_pins(work)}
    (work/'receipt.json').write_text(json.dumps(result,indent=2,sort_keys=True)+'\n')
    print(json.dumps({'receipt':str(work/'receipt.json'),'semantic_fixtures':4,'authored_controls':22,'capture_cases':41,'f1d2_entries':result['f1d2_preservation']['entries_verified']}))


if __name__=='__main__': main()
