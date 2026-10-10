#!/usr/bin/env python3
"""Read/hash-only evidence identity checks for additive whole precode decision."""
import pathlib, hashlib, json
OUT=pathlib.Path('/tmp/rusty-tiles-vector-whole-precode-review')
ROOT=pathlib.Path('/tmp/rusty-tiles-vector-codec-foundation')
pins={}
def sha(b):return hashlib.sha256(b).hexdigest()
def pin(p):
    p=pathlib.Path(p);b=p.read_bytes();pins[str(p)]={'bytes':len(b),'sha256':sha(b)};return b
def load(p):return json.loads(pin(p))
for p in [
ROOT/'AGENTS.md', ROOT/'docs/architecture/README.md',
ROOT/'docs/architecture/vector-compression-foundation.md',
pathlib.Path('/tmp/rusty-tiles-next-vector-compression-audit/codec/contract-draft.md'),
pathlib.Path('/tmp/rusty-tiles-next-vector-compression-audit/lifecycle/revision-20261010/contract-draft.md'),
pathlib.Path('/tmp/rusty-tiles-next-vector-compression-audit/precode_review/revision-20261010/checklist.md'),
pathlib.Path('/tmp/rusty-tiles-vector-proof-checkpoint-review/receipt.json'),
pathlib.Path('/tmp/rusty-tiles-vector-proof-checkpoint-review/verification.json'),
]:pin(p)
for parent in ['rusty-tiles-codec-p2-actual-review','rusty-tiles-codec-p2-allocation-review',
               'rusty-tiles-codec-parser-coefficient-review','rusty-tiles-vector-p3-actual-review',
               'rusty-tiles-vector-f0-actual-review','rusty-tiles-codec-owned-plan-precode-review',
               'rusty-tiles-codec-owned-plan-actual-review']:
    folder=pathlib.Path('/tmp')/parent
    for p in folder.iterdir():
        if p.is_file():pin(p)
for folder in ['rusty-tiles-codec-owned-plan-preparation','rusty-tiles-codec-owned-plan-context-revision']:
    for p in (pathlib.Path('/tmp')/folder).rglob('*'):
        if p.is_file():pin(p)
before=load('/tmp/rusty-tiles-codec-owned-plan-execution-binding-20261010/before.json')
after=load('/tmp/rusty-tiles-codec-owned-plan-execution-binding-20261010/after.json')
assert sha(pin('/tmp/rusty-tiles-codec-owned-plan-execution-binding-20261010/after.json'))=='ea5fe7429ce44417a3889cd2ea52e10f66435acce1d0baf385e7e5210b46c3bd'
assert after['artifactPinsUnchanged'] and after['sourceUnchanged'] and after['exitCode']==0
assert sha(pin('/tmp/rusty-tiles-codec-owned-plan-execution-binding-20261010/before.json'))==after['beforeSha256']
execution=pathlib.Path('/tmp/rusty-tiles-codec-owned-plan-execution-20261010')
for n,expected in after['outputHashes'].items():assert sha(pin(execution/n))==expected,n
receipt=load(execution/'receipt.json')
assert len(receipt['commands'])==55
assert all(c['exitCode']==0 for c in receipt['commands'])
for c in receipt['commands']:
    assert sha(pin(execution/(c['label']+'.stdout')))==c['stdoutSha256']
    assert sha(pin(execution/(c['label']+'.stderr')))==c['stderrSha256']
record_check=load(execution/'independent-record-check.stdout')
assert record_check['acceptedCommands']==52 and record_check['sensitiveChangedRecordRejections']==18
source=load('/tmp/rusty-tiles-payload-final-evidence/stride-final-build/source-pin.json')
assert len(source['production_sha256'])==97
for rel,expected in source['production_sha256'].items():assert sha(pin(ROOT/rel))==expected,rel
(OUT/'input-pin.json').write_text(json.dumps(pins,indent=2)+'\n')
(OUT/'verification.json').write_text(json.dumps({
'status':'PASS_READ_HASH_IDENTITIES_ONLY', 'inputFiles':len(pins),
'rootPlanReceiptCommands':55,'rootActualProbeCommands':52,'rootChangedRecordRejections':18,
'allRecordedExitCodesZero':True,'productionHashesUnchanged':97,
'afterBindingSha256':sha(pin('/tmp/rusty-tiles-codec-owned-plan-execution-binding-20261010/after.json')),
'scope':'All recorded output hashes checked; separate actual reviewer owns allocation-event/source/ABI acceptance. This reviewer does not rehash the large compiler inventory or execute any target/author checker.',
'compilerCargoNativeProducerDecoderAuthorCheckerOrGitInvocations':0,'productionEdits':0
},indent=2)+'\n')
print(json.dumps({'status':'PASS','inputs':len(pins),'recordedCommands':55,'actualProbeCommands':52,'changedRecordControls':18}))
