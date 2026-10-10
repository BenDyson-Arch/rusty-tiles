"""Independent bounded reads/hashes/NDJSON arithmetic, never executes a target."""
from pathlib import Path
import hashlib,json

HERE=Path(__file__).resolve().parent
P=Path('/tmp/rusty-tiles-codec-owned-plan-preparation')
C=Path('/tmp/rusty-tiles-codec-owned-plan-context-revision')
E=Path('/tmp/rusty-tiles-codec-owned-plan-execution-20261010')
B=Path('/tmp/rusty-tiles-codec-owned-plan-execution-binding-20261010')
T=Path('/tmp/rusty-tiles-codec-owned-plan-link-trace-20261010')
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def load(p):return json.loads(p.read_text())
def find(rows,key):
    values=[r[key] for r in rows if key in r]
    assert len(values)==1,(key,len(values))
    return values[0]
def get_stage(rows,name):return next(r for r in rows if r.get('stage')==name)
def replay(rows,s):
    events=[r for r in rows if r.get('allocationStage')==s['stage']]
    assert s['traceComplete'] and len(events)==s['events']
    if not s['detailedTraceCaptured']:
        assert not events
        return 0
    current=highest=s['baselineRequested']
    for index,a in enumerate(events):
        assert a['event']==index and a['liveBefore']==current
        old,new=a['oldBytes'],a['newBytes']
        assert old>=0 and new>=0
        if a['kind'] in (1,2):
            assert old==0
            highest=max(highest,current+new);current+=new
        elif a['kind']==3:
            assert new==0 and old<=current
            current-=old
        elif a['kind']==4:
            assert old<=current
            highest=max(highest,current+new);current+=new-old
        else:raise AssertionError(a)
    assert current==s['liveRequested'] and highest==s['peakRequestedOldPlusNew']
    return len(events)

before,after,receipt=load(B/'before.json'),load(B/'after.json'),load(E/'receipt.json')
assert sha(B/'after.json')=='ea5fe7429ce44417a3889cd2ea52e10f66435acce1d0baf385e7e5210b46c3bd'
assert sha(B/'before.json')==after['beforeSha256']
assert after['exitCode']==0 and after['artifactPinsUnchanged'] and after['sourceUnchanged']
assert before['cpuAffinity']==[0,1] and before['nice']==10
assert before['sourceSha256']==sha(P/'owned_plan_probe.rs')
assert before['coordinatorSourceSha256']==sha(Path('/tmp/rusty-tiles-run-owned-plan-context.py'))
assert before['contextRunnerSha256']==sha(C/'root_runner.py')
assert before['packageIntegritySha256']==sha(P/'integrity.json')
assert before['contextRevisionIntegritySha256']==sha(C/'integrity.json')
assert receipt['packageIntegritySha256']==before['packageIntegritySha256']
assert receipt['binarySha256']==sha(E/'owned_plan_probe')
for name,value in after['outputHashes'].items():assert sha(E/name)==value,name
for name,value in after['linkTraceHashes'].items():assert sha(T/name)==value,name
for name,value in [('runner.stdout',after['runnerStdoutSha256']),('runner.stderr',after['runnerStderrSha256'])]:
    assert sha(B/name)==value
for root in (P,C):
    for name,pin in load(root/'integrity.json')['files'].items():
        q=root/name;assert q.stat().st_size==pin['bytes'] and sha(q)==pin['sha256']
for name,pin in load(C/'input-pin.json')['files'].items():
    q=Path(name);assert q.stat().st_size==pin['bytes'] and sha(q)==pin['sha256']
commands=load(P/'commands.json')['commands']
assert len(commands)==52 and len(receipt['commands'])==55
for command in receipt['commands']:
    assert command['exitCode']==0,command['label']
    for suffix in ('stdout','stderr'):
        assert sha(E/(command['label']+'.'+suffix))==command[suffix+'Sha256']
assert [r['label'] for r in receipt['commands'][2:-1]]==[r['label'] for r in commands]
base=load(Path('/tmp/rusty-tiles-codec-p2-executions/exact-artifact-receipt.json'))
assert (E/'rustc-vV.stdout').read_text()==base['rustcVV']
check=load(E/'independent-record-check.stdout')
assert check['acceptedCommands']==52 and check['sensitiveChangedRecordRejections']==18
assert check['labels']==[r['label'] for r in commands] and not check['productionOrSemanticDomainAcceptance']

fixtures=load(P/'fixtures.json');cases={r['name']:r for r in fixtures['cases']}
types_seen=[];events=0;stages=0;prepare_facts=[];failures=[];successful_oracles=0;gates=0
all_rows={}
for command in commands:
    label=command['label'];rows=[json.loads(line) for line in (E/(label+'.stdout')).read_text().splitlines()]
    all_rows[label]=rows
    if label=='checked-capacity-controls':
        assert find(rows,'checkedCapacityControls')==4 and find(rows,'typedResourceLimit')
        continue
    f=cases[command['case']]['facts'];args=command['args'];role,fault=args[3],args[2]
    types=find(rows,'typeSizes');types_seen.append(types)
    for name,size in fixtures['expectedTypeSizes'].items():assert types[name]==size
    assert find(rows,'sourceCapacity')==f['J']
    for s in (r for r in rows if 'stage' in r):events+=replay(rows,s);stages+=1
    adm=get_stage(rows,'accepted_pinned_json_admission_no_detailed_trace')
    assert adm['peakRequestedOldPlusNew']-adm['baselineRequested']<=8*f['J']+256*min(int(args[4]),f['J'])+65536
    if role=='admission':
        outcome=find(rows,'jsonAdmissionOutcome')
        assert outcome==('Accepted' if int(args[4])>=f['K'] else 'ResourceLimit')
        if outcome=='Accepted':assert find(rows,'K')==f['K']
        assert not any(r.get('stage')=='concrete_plan_prepare' for r in rows)
        continue
    prep=get_stage(rows,'concrete_plan_prepare')
    peak=prep['peakRequestedOldPlusNew']-prep['baselineRequested']
    assert peak<=f['planWork']
    if label.endswith('one-below-work'):
        assert find(rows,'prepareOutcome')=='ResourceLimit' and prep['events']==0
        assert prep['liveRequested']==prep['baselineRequested'];gates+=1;continue
    if any('prepareOutcome' in r for r in rows):
        outcome=find(rows,'prepareOutcome');assert outcome==('ResourceLimit' if fault!='none' else 'Unsupported')
        assert prep['liveRequested']==prep['baselineRequested'];failures.append([label,outcome,peak]);continue
    a=find(rows,'planCapacities')
    assert a['arena']==f['K'] and a['arenaLen']==f['arenaLen']<=f['K']
    assert a['views']==f['V'] and a['ownedKeyBytes']==f['ownedKeyBytes']
    assert a['borrowedKeys']==f['borrowedKeys'] and a['planEstimateGate']==f['planWork']
    retained=40*f['K']+96*f['V']+f['ownedKeyBytes']
    assert a['retainedRequested']==retained==prep['liveRequested']-prep['baselineRequested']
    prepare_facts.append({'label':label,'J':f['J'],'K':f['K'],'V':f['V'],'peak':peak,'retained':retained,'estimate':f['planWork'],'events':prep['events']})
    if role!='encode':
        assert find(rows,'identityOracleMatch') and not find(rows,'semanticClassifierOrNativeValidationProved')
        cp=get_stage(rows,'fixture_identity_role_plan_dropped_before_copy')
        assert cp['baselineRequested']==prep['baselineRequested'],(label,'Plan owners released before role copy')
        assert cp['peakRequestedOldPlusNew']-cp['baselineRequested']==f['J']==cp['liveRequested']-cp['baselineRequested'];continue
    edits=get_stage(rows,'exact_edit_allocation')
    if fault=='edits':assert find(rows,'encodePreparationOutcome')=='ResourceLimit';continue
    assert edits['liveRequested']-edits['baselineRequested']==32*f['V']
    count=get_stage(rows,'count_known_declarations_no_string_copy')
    assert count['events']==0 and count['liveRequested']==count['baselineRequested']
    negative=get_stage(rows,'count_one_below_negative')
    assert negative['liveRequested']==negative['baselineRequested']
    out=get_stage(rows,'direct_fixed_glb')
    if fault=='candidate':assert find(rows,'candidatePreparationOutcome')=='ResourceLimit' and out['events']==0;continue
    expected=(Path(cases[command['case']]['path'])/'expected.glb').read_bytes()
    assert find(rows,'exactOracleMatch') and find(rows,'candidateCapacity')==len(expected)
    assert out['peakRequestedOldPlusNew']-out['baselineRequested']==len(expected)==out['liveRequested']-out['baselineRequested']
    cp=get_stage(rows,'plan_dropped_before_owned_identity_copy')
    assert cp['baselineRequested']==prep['baselineRequested']-find(rows,'oracleCapacityExcluded')-find(rows,'fixtureBinaryCapacityExcluded'),(label,'Plan/edit/candidate owners released before copy')
    assert cp['peakRequestedOldPlusNew']-cp['baselineRequested']==f['J']==cp['liveRequested']-cp['baselineRequested']
    successful_oracles+=1
assert all(t==types_seen[0] for t in types_seen)
long=all_rows['long-root-key-late-escape'];f=cases['long-root-key-late-escape']['facts'];prep=get_stage(long,'concrete_plan_prepare')
assert prep['peakRequestedOldPlusNew']-prep['baselineRequested']>2*f['J']+128*f['K']+256*f['V']
alloc=[r for r in long if r.get('allocationStage')=='concrete_plan_prepare']
for size in (16,65537,131072):assert any(r['newBytes']==size and r['kind'] in (1,2,4) for r in alloc)
assert any(r['kind']==4 and r['oldBytes']==65536 and r['newBytes']==131072 for r in alloc)
result={'status':'INDEPENDENT_BOUNDED_ACTUAL_RECORD_REPLAY_PASS','evidenceClass':'read/hash/arithmetic only; no reviewer target/compiler execution','commands':52,'commandReceiptRecords':55,'outputHashesVerified':len(after['outputHashes']),'linkTraceHashesVerified':len(after['linkTraceHashes']),'types':types_seen[0],'stagesReplayed':stages,'allocationEventsReplayed':events,'wholeOracleAssertionsObserved':successful_oracles,'oneBelowPlanGatesNoAllocation':gates,'authorActualChangedRecordRejections':18,'sensitivitySelectorsObservedInActualTrace':True,'artifactPinsRootBeforeAfterVerified':len(before['artifactPins']),'largeArtifactRehashByReviewer':False,'prepareFacts':prepare_facts,'typedPrepareFailureFacts':failures}
(HERE/'verification.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items() if k not in ('prepareFacts','typedPrepareFailureFacts')},indent=2))
