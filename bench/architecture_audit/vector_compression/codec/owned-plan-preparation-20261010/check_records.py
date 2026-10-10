"""Root-only independent checker for future actual prototype NDJSON; never run by author.
Invocation: python3 check_records.py FRESH_OUTPUT containing LABEL.stdout/LABEL.stderr.
No guessed target outputs are substituted. Faults modify copies of actual records and must
be rejected by this same unchanged checker; evidence remains specific to this prototype.
"""
from pathlib import Path
import json,copy,sys
HERE=Path(__file__).resolve().parent
F=json.loads((HERE/'fixtures.json').read_text());CASE={r['name']:r for r in F['cases']}
C=json.loads((HERE/'commands.json').read_text())['commands']
def one(rows,key):
    found=[r for r in rows if key in r];assert len(found)==1,(key,len(found));return found[0]
def stage(rows,name):return next(r for r in rows if r.get('stage')==name)
def trace(rows,s):
    name=s['stage'];events=[r for r in rows if r.get('allocationStage')==name]
    assert s['traceComplete'];assert s['events']==len(events)
    if not s['detailedTraceCaptured']:assert not events;return
    live=s['baselineRequested'];peak=live
    for n,e in enumerate(events):
        assert e['event']==n and e['liveBefore']==live,(name,n,'live')
        kind=e['kind'];old=e['oldBytes'];new=e['newBytes']
        if kind in (1,2):assert old==0;peak=max(peak,live+new);live+=new
        elif kind==3:assert new==0 and old<=live;live-=old
        elif kind==4:assert old<=live;peak=max(peak,live+new);live+=new-old
        else:raise AssertionError(kind)
    assert live==s['liveRequested'],(name,'retained')
    assert peak==s['peakRequestedOldPlusNew'],(name,'old+new peak',peak,s['peakRequestedOldPlusNew'])
def check(command,rows):
    if command['label']=='checked-capacity-controls':
        r=one(rows,'checkedCapacityControls');assert r['checkedCapacityControls']==4 and r['typedResourceLimit'] and r['sourceAllocationsOrNativeInvocations']==0;return
    case=CASE[command['case']];f=case['facts'];args=command['args'];role=args[3];fault=args[2]
    header=one(rows,'typeSizes');assert header['sourceCapacity']>=f['J'];types=header['typeSizes']
    for key,size in F['expectedTypeSizes'].items():assert types[key]==size,(key,types[key],size)
    for s in [r for r in rows if 'stage' in r]:trace(rows,s)
    admission=stage(rows,'accepted_pinned_json_admission_no_detailed_trace')
    ku=min(int(args[4]),f['J']);ejson=8*f['J']+256*ku+65536
    assert admission['peakRequestedOldPlusNew']-admission['baselineRequested']<=ejson
    if role=='admission':
        r=one(rows,'jsonAdmissionOutcome');expected='Accepted' if int(args[4])>=f['K'] else 'ResourceLimit'
        assert r['jsonAdmissionOutcome']==expected and r['noPlanAttempted']
        if expected=='Accepted':assert r['K']==f['K']
        assert not any(r.get('stage')=='concrete_plan_prepare' for r in rows);return
    s=stage(rows,'concrete_plan_prepare')
    assert s['peakRequestedOldPlusNew']-s['baselineRequested']<=f['planWork'],(command['label'],'construction estimate')
    if command['label'].endswith('one-below-work'):
        assert one(rows,'prepareOutcome')['prepareOutcome']=='ResourceLimit'
        assert s['events']==0 and s['liveRequested']==s['baselineRequested'];return
    if fault in ('arena','views','refs','owned_key'):
        assert one(rows,'prepareOutcome')['prepareOutcome']=='ResourceLimit';return
    if case['expected']=='Unsupported':
        assert one(rows,'prepareOutcome')['prepareOutcome']=='Unsupported'
        assert not any('exactOracleMatch' in r for r in rows);return
    p=one(rows,'planCapacities')['planCapacities']
    for key in ('arenaLen','ownedKeyBytes','borrowedKeys','K','V'):assert p[key]==f[key],(key,p[key],f[key])
    assert p['arena']==f['K'] and p['views']==f['V']
    assert p['planEstimateGate']==f['planWork']
    assert p['retainedRequested']==f['retainedRequested']
    assert s['liveRequested']-s['baselineRequested']==f['retainedRequested'],(command['label'],'retained owner exact')
    if case['name']=='shared-descriptor-four-references':
        r=one(rows,'sharedReferenceCount');assert r['sharedReferenceCount']==4 and r['physicalDescriptors']==1 and r['sameDescriptorIdentity']
    if role!='encode':
        r=one(rows,'fixtureSelectedRole');assert r['fixtureSelectedRole']==role and r['identityOracleMatch'] and not r['semanticClassifierOrNativeValidationProved']
        cp=stage(rows,'fixture_identity_role_plan_dropped_before_copy')
        assert cp['liveRequested']-cp['baselineRequested']==f['J']
        assert cp['peakRequestedOldPlusNew']-cp['baselineRequested']==f['J'];return
    ed=stage(rows,'exact_edit_allocation')
    if fault=='edits':assert one(rows,'encodePreparationOutcome')['encodePreparationOutcome']=='ResourceLimit';return
    assert ed['liveRequested']-ed['baselineRequested']==32*f['V']
    ct=stage(rows,'count_known_declarations_no_string_copy');assert ct['events']==0 and ct['liveRequested']==ct['baselineRequested']
    out=stage(rows,'direct_fixed_glb')
    if fault=='candidate':assert one(rows,'candidatePreparationOutcome')['candidatePreparationOutcome']=='ResourceLimit';return
    r=one(rows,'exactJsonBytes');ep=Path(case['path']);expected=(ep/'expected.glb').read_bytes()
    # Whole oracle is asserted by actual Rust run; independently check exact framing/count facts.
    assert r['exactOracleMatch'] and r['candidateCapacity']==len(expected) and r['editsCapacity']==f['V']
    j=int.from_bytes(expected[12:16],'little');actual_json=expected[20:20+j].rstrip(b' ')
    assert r['exactJsonBytes']==len(actual_json)
    assert out['peakRequestedOldPlusNew']-out['baselineRequested']==len(expected)
    assert out['liveRequested']-out['baselineRequested']==len(expected)
    cp=stage(rows,'plan_dropped_before_owned_identity_copy')
    assert cp['peakRequestedOldPlusNew']-cp['baselineRequested']==f['J']
    assert cp['liveRequested']-cp['baselineRequested']==f['J']

def reject(command,rows,mutate):
    bad=copy.deepcopy(rows);mutate(bad)
    assert bad!=rows,'fault must actually change a selected association/owner/event'
    try:check(command,bad)
    except (AssertionError,StopIteration,KeyError):return 1
    raise AssertionError('unchanged checker admitted faulty record')

def faults(command,rows):
    if command.get('case')!='long-root-key-late-escape' or command['label']!='long-root-key-late-escape':return 0
    count=reject(command,rows,lambda x:one(x,'typeSizes').__setitem__('sourceCapacity',0))
    # Concrete retained owners have exact predicted identity, so omissions and duplicate ownership fail.
    for category,amount in [('arena',40*CASE[command['case']]['facts']['K']),('view',96),('escaped-key',65544)]:
        for sign in (-1,1):
            def m(x,amount=amount,sign=sign):one(x,'planCapacities')['planCapacities']['retainedRequested']+=sign*amount
            count+=reject(command,rows,m)
    count+=reject(command,rows,lambda x:one(x,'planCapacities')['planCapacities'].__setitem__('arena',65536))
    count+=reject(command,rows,lambda x:one(x,'planCapacities')['planCapacities'].__setitem__('arenaLen',2*CASE[command['case']]['facts']['arenaLen']))
    # Exact source-bound old estimate is known to fail for this mechanism; observe actual trace first.
    f=CASE[command['case']]['facts'];p=stage(rows,'concrete_plan_prepare')
    assert p['peakRequestedOldPlusNew']-p['baselineRequested']>2*f['J']+128*f['K']+256*f['V']
    # Each removed temporary ref/scratch/reallocation request is selected from actual complete events.
    es=[r for r in rows if r.get('allocationStage')=='concrete_plan_prepare']
    for wanted in (16,65537,131072):
        e=next(r for r in es if r['newBytes']==wanted and r['kind'] in (1,2,4))
        def m(x,n=e['event']):
            z=next(r for r in x if r.get('allocationStage')=='concrete_plan_prepare' and r['event']==n);z['newBytes']=0
        count+=reject(command,rows,m)
    realloc=next(r for r in es if r['kind']==4 and r['oldBytes']>0)
    def m(x):next(r for r in x if r.get('allocationStage')=='concrete_plan_prepare' and r['event']==realloc['event'])['oldBytes']=0
    count+=reject(command,rows,m)
    # Peak undercount independently fails event replay; duplicate transfer independently fails live replay.
    for stage_name in ('concrete_plan_prepare','exact_edit_allocation','direct_fixed_glb','plan_dropped_before_owned_identity_copy'):
        count+=reject(command,rows,lambda x,n=stage_name:stage(x,n).__setitem__('peakRequestedOldPlusNew',stage(x,n)['peakRequestedOldPlusNew']-1))
    count+=reject(command,rows,lambda x:stage(x,'concrete_plan_prepare').__setitem__('liveRequested',stage(x,'concrete_plan_prepare')['liveRequested']+65537))
    return count

def main():
    output=Path(sys.argv[1]);accepted=[];n=0
    for command in C:
        path=output/(command['label']+'.stdout')
        rows=[json.loads(line) for line in path.read_text().splitlines() if line.strip()]
        check(command,rows);n+=faults(command,rows);accepted.append(command['label'])
    print(json.dumps({'status':'ACTUAL records checked for this isolated prototype only','acceptedCommands':len(accepted),'sensitiveChangedRecordRejections':n,'labels':accepted,'productionOrSemanticDomainAcceptance':False},indent=2))
if __name__=='__main__':main()
