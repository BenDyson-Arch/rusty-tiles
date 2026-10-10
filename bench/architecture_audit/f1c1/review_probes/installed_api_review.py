#!/usr/bin/env python3
"""Handcrafted installed-API review, independent from the shared wheel runners."""
import collections
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import zipfile

sys.dont_write_bytecode=True
import fresh_review as own

EXT_SHA='c6b17e6b58b5d2824ed5cfdb80d790f4a1c4c702575ed2619c80e3b052825b86'
WHEEL_SHA='74306bb22bf1f11e22af4fc0f71745e46a19a4a3c625f9235ab741c9beee05a9'
HERE=Path(__file__).resolve().parent
def main():
    manifest=json.loads(Path(sys.argv[1]).read_text())
    wheel=manifest['wheel']
    extension=Path(wheel['extension_path'])
    assert own.sha(extension)==wheel['extension_sha256']==EXT_SHA
    assert own.sha(Path(wheel['path']))==wheel['sha256']==WHEEL_SHA
    with zipfile.ZipFile(wheel['path']) as archive:
        names=[n for n in archive.namelist() if n.endswith('rusty_tiles.abi3.so')]
        assert len(names)==1 and hashlib.sha256(archive.read(names[0])).hexdigest()==EXT_SHA
    for name,digest in manifest['compiled_inputs_sha256'].items():
        assert own.sha(own.ROOT/name)==digest
        committed=subprocess.check_output(['git','show',f'{own.PIN}:{name}'],cwd=own.ROOT)
        assert hashlib.sha256(committed).hexdigest()==digest
    os.environ['PATH']=''
    sys.path.insert(0,str(extension.parent.parent))
    import rusty_tiles as api
    imported=Path(sys.modules['rusty_tiles.rusty_tiles'].__file__).resolve()
    assert imported==extension.resolve()
    outputdir=HERE/'api-runs'
    outputdir.mkdir(exist_ok=True)
    source=HERE/'fixtures'/'hierarchical.glb'
    prior_truth=json.loads((HERE/'fresh-review-results.json').read_text())['fixtures'][0]['expected_baked_positions']
    params={'anchor':(153.02,-27.48,57200000.),'orientation_xyzw':(.18257418583505536,.3651483716701107,.5477225575051661,.7302967433402214),'scene_offset':(300.,-200.,100.)}
    report={'source_commit':own.PIN,'extension_sha256':EXT_SHA,'wheel_sha256':WHEEL_SHA,'interpreter':sys.version,'module_path':str(imported),'PATH_during_api_calls':os.environ['PATH'],'compiled_inputs_verified':len(manifest['compiled_inputs_sha256']),'executions':[],'findings':[]}
    def output(name, prior=False):
        p=outputdir/(name+'.3tz')
        p.unlink(missing_ok=True)
        if prior: p.write_bytes(b'previous artifact')
        return p
    def inspect(path,result):
        with zipfile.ZipFile(path) as stream: members={n:stream.read(n) for n in stream.namelist()}
        doc=json.loads(members['tileset.json'])
        stored=json.loads(members['conversion.json'])
        assert result.report==stored and stored['root_transform']==doc['root']['transform']
        points=[p for c in doc['root']['children'] for p in own.decode(members[c['content']['uri']])]
        assert collections.Counter(points)==collections.Counter(map(tuple,prior_truth))
        ref,_=own.reference(params['anchor'],params['orientation_xyzw'],params['scene_offset'])
        matrix=list(map(own.dec,doc['root']['transform']))
        worst=max(abs(own.dec(a)-b) for p in points for a,b in zip(own.apply_float(matrix,[p[0],-p[2],p[1]]),own.apply(ref,[p[0],-p[2],p[1]])))
        assert worst<=own.D('0.000001')
        own.verify_world_boxes(doc['root'],matrix,ref,members)
        assert not result.cleanup_diagnostics
        return str(worst)
    events=[]
    p=output('placed-callback-success')
    result=api.mesh_local_to_3tz(source,p,leaf_triangles=1,callback=events.append,**params)
    assert events[0]['phase']=='mesh_leaves' and events[0]['done']==0
    assert events[-1]['phase']=='ready_to_publish'
    report['executions'].append({'name':'placed-callback-success','status':'passed','events':events,'f64_world_error_metres':inspect(p,result)})
    # Exact callback exception objects must survive cleanup at three lifecycle boundaries.
    for target in ('initial','leaf-written','ready'):
        p=output('exception-'+target,True)
        marker=RuntimeError('independent reviewer marker '+target)
        seen=[]
        def callback(event):
            seen.append(event)
            match=(target=='initial' and event['phase']=='mesh_leaves' and event['done']==0) or (target=='leaf-written' and event['phase']=='mesh_leaves' and event['done']==1) or (target=='ready' and event['phase']=='ready_to_publish')
            if match: raise marker
        try: api.mesh_local_to_3tz(source,p,leaf_triangles=1,force=True,callback=callback,**params)
        except RuntimeError as error: assert error is marker
        else: raise AssertionError('callback exception swallowed '+target)
        assert p.read_bytes()==b'previous artifact'
        assert not any(q.name.startswith(('.mesh-work-','.tiles-stage-')) for q in outputdir.rglob('*'))
        report['executions'].append({'name':'callback-exception-'+target,'status':'passed','original_exception_identity':True,'prior_preserved':True,'events':seen})
    p=output('keyboardinterrupt-cancel',True)
    marker=KeyboardInterrupt('independent reviewer interruption')
    def interrupt(event):
        if event['phase']=='mesh_leaves' and event['done']==1: raise marker
    try: api.mesh_local_to_3tz(source,p,leaf_triangles=1,force=True,callback=interrupt,**params)
    except KeyboardInterrupt as error: assert error is marker
    else: raise AssertionError('KeyboardInterrupt swallowed')
    assert p.read_bytes()==b'previous artifact'
    report['executions'].append({'name':'callback-KeyboardInterrupt-cancel','status':'passed','original_exception_identity':True,'prior_preserved':True})
    # Actual Python SIGINT delivery at an admitted callback must abort publication.
    p=output('sigint-cancel',True)
    def send_sigint(event):
        if event['phase']=='mesh_leaves' and event['done']==1: signal.raise_signal(signal.SIGINT)
    try: api.mesh_local_to_3tz(source,p,leaf_triangles=1,force=True,callback=send_sigint,**params)
    except KeyboardInterrupt: pass
    else: raise AssertionError('SIGINT did not interrupt API call')
    assert p.read_bytes()==b'previous artifact'
    report['executions'].append({'name':'actual-SIGINT-cancel','status':'passed','prior_preserved':True})
    # Public adapter creates an independent RunControl for each nested call.
    outer=output('nested-outer')
    inner=output('nested-inner')
    nested=[]
    def reenter(event):
        if not nested:
            nested.append({'state':'started'})
            nested_events=[]
            second=api.mesh_local_to_3tz(source,inner,leaf_triangles=1,callback=nested_events.append,**params)
            nested[0]={'state':'completed','events':nested_events,'f64_world_error_metres':inspect(inner,second)}
    first=api.mesh_local_to_3tz(source,outer,leaf_triangles=1,callback=reenter,**params)
    assert nested[0]['state']=='completed'
    report['executions'].append({'name':'nested-independent-API-calls','status':'passed','nested':nested,'outer_f64_world_error_metres':inspect(outer,first),'api_call_count':2})
    # Bad nested request must not poison the enclosing operation.
    p=output('nested-invalid-request')
    nested_errors=[]
    def nested_invalid(event):
        if not nested_errors:
            try: api.mesh_local_to_3tz(HERE/'missing-source.glb',outputdir/'never'/'invalid.3tz',leaf_triangles=1,anchor=(181,0,0))
            except api.InvalidRequestError as error: nested_errors.append(error.kind)
            else: raise AssertionError('nested invalid admitted')
    result=api.mesh_local_to_3tz(source,p,leaf_triangles=1,callback=nested_invalid,**params)
    assert nested_errors==['invalid_request']
    report['executions'].append({'name':'nested-request-refusal-isolation','status':'passed','nested_kind':nested_errors,'outer_f64_world_error_metres':inspect(p,result),'api_call_count':2})
    invalid=[('longitude',{'anchor':(181,0,0)},'invalid_request'),('latitude',{'anchor':(0,-91,0)},'invalid_request'),('nan-anchor',{'anchor':(0,0,float('nan'))},'invalid_request'),('zero-quaternion',{'anchor':(0,0,0),'orientation_xyzw':(0,0,0,0)},'invalid_request'),('orientation-no-anchor',{'orientation_xyzw':(0,0,0,1)},'invalid_request'),('offset-no-anchor',{'scene_offset':(0,0,0)},'invalid_request'),('infinite-offset',{'anchor':(0,0,0),'scene_offset':(float('inf'),0,0)},'invalid_request'),('cap-plus-subnormal',{'anchor':(0,0,own.CAP),'scene_offset':(5e-324,0,0)},'unsupported'),('distributed-budget',{'anchor':(0,0,30e6),'scene_offset':(30e6,-1,0)},'unsupported')]
    for name,kwargs,kind in invalid:
        for prior in (False,True):
            p=output(name+'-prior',True) if prior else outputdir/'uncreated'/name/'out.3tz'
            observed=[]
            expected=api.InvalidRequestError if kind=='invalid_request' else api.UnsupportedError
            try: api.mesh_local_to_3tz(HERE/'missing-source.glb',p,leaf_triangles=1,force=prior,callback=observed.append,**kwargs)
            except expected as error:
                assert error.kind==kind and error.retained_paths==[] and error.secondary_diagnostics==[] and error.recovery is None
            else: raise AssertionError('request admitted '+name)
            assert not observed
            assert p.read_bytes()==b'previous artifact' if prior else not p.parent.exists()
            report['executions'].append({'name':'preIO-'+name,'prior_output':prior,'status':'passed','kind':kind,'callback_events':observed})
    p=output('exact-cap-admitted')
    try: api.mesh_local_to_3tz(HERE/'missing-source.glb',p,leaf_triangles=1,anchor=(0,0,own.CAP))
    except api.TilesIOError as error: assert error.kind=='io'
    else: raise AssertionError('missing source admitted')
    report['executions'].append({'name':'exact-cap-admitted-before-missing-source','status':'passed','kind':'io'})
    assert not any(q.name.startswith(('.mesh-work-','.tiles-stage-')) for q in outputdir.rglob('*'))
    assert own.sha(extension)==EXT_SHA
    report['status']='passed'
    report['api_call_count']=sum(e.get('api_call_count',1) for e in report['executions'])
    report['probe_sha256']=own.sha(Path(__file__))
    report['limits']=['Linux installed CPython3.14.7 ABI3 extension only.','Python cancellation evidence uses KeyboardInterrupt/SIGINT delivered during callbacks; public Python API does not expose direct RunControl reuse.','Nested calls prove separate adapter-created controls, not reuse of one Rust RunControl.','No platform/native/renderer/Blender execution in this installed-API extension review.']
    (HERE/'installed-api-results.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'status':report['status'],'api_calls':report['api_call_count'],'extension_sha256':EXT_SHA}))
if __name__=='__main__': main()
