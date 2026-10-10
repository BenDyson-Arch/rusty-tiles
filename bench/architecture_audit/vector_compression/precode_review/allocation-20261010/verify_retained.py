"""Independent read/hash and integer adjudication; never runs author model/target."""
from pathlib import Path
import hashlib,json
OUT=Path(__file__).parent
PKG=Path('/tmp/rusty-tiles-next-vector-compression-audit/codec/allocation-adjudication-20261010')
RUN=Path('/tmp/rusty-tiles-codec-p2-executions/allocation-adjudication-model')
pins={}
def pin(p):
 p=Path(p);b=p.read_bytes();r={'bytes':len(b),'sha256':hashlib.sha256(b).hexdigest()};pins[str(p)]=r;return r
def read(p):pin(p);return json.loads(Path(p).read_text())
i=read(PKG/'integrity.json');assert pin(PKG/'integrity.json')['sha256']=='1ee2185020b67803b10e394da54379727fe7de24467b6d44ab11a78e8397a89a'
for rel,r in i['files'].items():assert pin(PKG/rel)==r,rel
for p,r in i['externalInputs'].items():assert pin(p)==r,p
receipt=read(RUN/'receipt.json');actual=read(RUN/'stdout.json')
assert receipt['exitCode']==0 and receipt['sourceIntegritySha256']==pin(PKG/'integrity.json')['sha256']
for n,s in receipt['outputs'].items():assert pin(RUN/n)['sha256']==s
assert pin(RUN/'stdout.json')['sha256']=='2dc7910366aa456124cc033361a5f3e6399ac6bdf0d7df43f1ead21b717d8912'
assert receipt['argv'][:3]==['nice','-n','10'] and 'run-controlled.py' in receipt['argv'][4]
pin(receipt['argv'][4])
facts=read(PKG/'source-ledger.json');fixture=read(facts['fixtureInput']);lengths=[len(k.encode())for k in fixture]
assert lengths==facts['literalDecodedRootKeyLengths']==[5,7,11,14,18,65537,3]
assert sum(len(k.encode())for obj in [fixture,*fixture['buffers'],*fixture['bufferViews']]for k in obj)==facts['allOwnedKeyCapacities']==65660
rows=[json.loads(l)for l in Path(facts['actualRawStdout']).read_text().splitlines()];stages={r['stage']:r for r in rows if 'stage'in r};plan=stages['scaffold_owned_key_pairs_and_fixed_edit'];parser=stages['actual_pinned_json_admission']
assert plan['peakRequestedOldPlusNew']-plan['baselineRequested']==196987
assert plan['liveRequested']-plan['baselineRequested']==66676
assert parser['peakRequestedOldPlusNew']-parser['baselineRequested']==197041
assert 131072+sum(lengths)+8*40==196987 and 65660+40*(8+4+8)+24+16*(4+4+4)==66676
for e in facts['events']:assert sum(e['terms'].values())==e['expectedIncrementalRequested']
assert facts['proposedEbuild']==3*facts['J']+128*facts['K']+256*(facts['V']+facts['A'])==200434
assert facts['proposedEretain']==facts['J']+128*facts['K']+256*(facts['V']+facts['A'])==68774
expected=read(PKG/'expected-controls.json');evaluated=[];faults=[]
for c in expected['cases']:
 required=max(sum(v.values())for v in c['branches'].values())+c['outsidePhaseBytes'];assert required==c['expectedRequiredBytes']
 category='admitted'if required<=c['configuredLimit']else'resource_limit';assert category==c['expectedCategory']
 evaluated.append({'name':c['name'],'requiredBytes':required,'configuredLimit':c['configuredLimit'],'category':category})
for c in expected['faultyLedgers']:
 claimed=sum(c['terms'].values());assert claimed==c['predictedWrongBytes']and claimed!=c['independentCorrectBytes']
 faults.append({'name':c['name'],'claimedBytes':claimed,'expectedBytes':c['independentCorrectBytes'],'status':'rejected_by_independent_source_ledger'})
assert actual['cases']==evaluated and actual['faultyLedgers']==faults and len(evaluated)==12 and len(faults)==11
assert actual['productionProof']is False and actual['wholeOperationRssProof']is False
for p in ['/tmp/rusty-tiles-c1-payload-foundation/AGENTS.md','/tmp/rusty-tiles-c1-payload-foundation/docs/architecture/README.md','/tmp/rusty-tiles-next-vector-compression-audit/lifecycle/revision-20261010/contract-draft.md','/tmp/rusty-tiles-next-vector-compression-audit/lifecycle/proof-inputs-20261010-4dc/README.md','/tmp/rusty-tiles-vector-p3-inputs-small/manifest.json']:
 pin(p)
old=Path('/tmp/rusty-tiles-codec-p2-actual-review');oldint=read(old/'integrity.json')
for n,r in oldint['files'].items():assert pin(old/n)==r
result={'status':'INDEPENDENT_RETAINED_ALLOCATION_RECORD_HASH_AND_INTEGER_CHECKS_PASS','packageFiles':len(i['files']),'externalInputs':len(i['externalInputs']),'actualModelExit':0,'exactOneBelowCases':12,'faultyLedgerRejections':11,'recordedConstructionMaximum':196987,'recordedRetainedLive':66676,'sourceDerivedIntermediateEventsSeparatelyTraced':False,'old2JStillRejected':True,'universal3JApproved':False,'productionCoefficientsApproved':False,'executionScope':'Only this independent read/hash/integer verifier executed. No author model, Cargo, Git, native, Rust or producer target was run by reviewer.'}
(OUT/'verification.json').write_text(json.dumps(result,indent=2)+'\n')
(OUT/'input-pin.json').write_text(json.dumps({'files':pins},indent=2)+'\n')
print(json.dumps(result,indent=2))
