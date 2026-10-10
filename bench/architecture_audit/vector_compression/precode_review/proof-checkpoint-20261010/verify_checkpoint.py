#!/usr/bin/env python3
"""Independent read/hash/retention checks; no authored checker or target execution."""
import pathlib, json, hashlib, gzip, tarfile, subprocess, collections, re

ROOT = pathlib.Path('/tmp/rusty-tiles-vector-codec-foundation')
OUT = pathlib.Path('/tmp/rusty-tiles-vector-proof-checkpoint-review')
PIN = pathlib.Path('/tmp/rusty-tiles-vector-proof-checkpoint-20261010/candidate-pin.json')
PREFIX = 'bench/architecture_audit/vector_compression/'
def sha(b): return hashlib.sha256(b).hexdigest()
def read(p): return pathlib.Path(p).read_bytes()
def load(p): return json.loads(read(p))
def git(*args): return subprocess.check_output(['git', '-C', str(ROOT), *args])
def write(name, value): (OUT/name).write_text(json.dumps(value, indent=2)+'\n')
d = load(PIN)
assert sha(read(PIN)) == '80c3a25ccd65d2528520225b26b30a40ac8d8f13dcba9ebc49559f93c30f8420'
assert len(d['files']) == 402
inputs = {str(PIN): {'bytes':len(read(PIN)), 'sha256':sha(read(PIN))}}
def pin(path):
    b=read(path); inputs[str(path)]={'bytes':len(b),'sha256':sha(b)}; return b
for p, expected in d['files'].items():
    b=pin(ROOT/p)
    assert len(b)==expected['bytes'] and sha(b)==expected['sha256'], p
source = load(d['sourcePin'])
assert sha(pin(d['sourcePin']))==d['sourcePinSha256']
prod=source['production_sha256']
assert len(prod)==97
for p, expected in prod.items():
    b=pin(ROOT/p)
    digest=expected if isinstance(expected,str) else expected['sha256']
    assert sha(b)==digest,p
    assert b==git('show',d['acceptedDevelopBase']+':'+p),p
changes=git('diff','--name-status',d['baseHead']).decode().splitlines()
changed={line.split('\t')[1]:line.split('\t')[0] for line in changes}
assert set(changed)==set(d['files'])
assert changed['docs/architecture/vector-compression-foundation.md']=='M'
assert all(status=='A' for p,status in changed.items() if p.startswith(PREFIX))
assert not git('diff','--name-only'), 'unstaged changes'

mapping={
'codec/allocation-adjudication-20261010':'/tmp/rusty-tiles-next-vector-compression-audit/codec/allocation-adjudication-20261010',
'codec/p2-domain-oracle-revision-20261010':'/tmp/rusty-tiles-next-vector-compression-audit/codec/p2-domain-oracle-revision-20261010',
'codec/p3-generated-preparation-20261010':'/tmp/rusty-tiles-vector-p3-generated-preparation',
'codec/p3-sensitive-preparation-20261010':'/tmp/rusty-tiles-vector-p3-sensitive-preparation',
'lifecycle/f0-module-path-revision-20261010':'/tmp/rusty-tiles-vector-f0-module-path-revision',
'lifecycle/f0-probe-preparation-20261010':'/tmp/rusty-tiles-vector-f0-probe-preparation',
'lifecycle/proof-inputs-20261010-4dc':'/tmp/rusty-tiles-next-vector-compression-audit/lifecycle/proof-inputs-20261010-4dc',
'precode_review/actual-p2-20261010':'/tmp/rusty-tiles-codec-p2-actual-review',
'precode_review/allocation-20261010':'/tmp/rusty-tiles-codec-p2-allocation-review',
'precode_review/f0-actual-20261010':'/tmp/rusty-tiles-vector-f0-actual-review',
'precode_review/p3-actual-20261010':'/tmp/rusty-tiles-vector-p3-actual-review',
'precode_review/p3-preexecution-20261010':'/tmp/rusty-tiles-vector-p3-preexecution-review',
'precode_review/parser-coefficients-20261010':'/tmp/rusty-tiles-codec-parser-coefficient-review',
}
copies=collections.Counter()
for p in d['files']:
    for rel, original in mapping.items():
        stem=PREFIX+rel+'/'
        if p.startswith(stem):
            assert read(ROOT/p)==pin(pathlib.Path(original)/p[len(stem):]),p
            copies[rel]+=1

evidence=ROOT/PREFIX/'candidate_evidence'
records={}
for name, count in [('p2-actual-20261010',250),('f0-module-path-original-failure',6),('f0-module-path-actual-20261010',15)]:
    index=load(evidence/name/'index.json')
    assert len(index['records'])==count
    for r in index['records']:
        packed=read(evidence/name/r['retainedFile']);b=gzip.decompress(packed)
        assert sha(packed)==r['gzipSha256'] and len(b)==r['bytes'] and sha(b)==r['sha256']
        if 'gzipBytes' in r: assert len(packed)==r['gzipBytes']
        if 'sourcePath' in r: original=pathlib.Path(r['sourcePath'])
        elif name=='f0-module-path-original-failure': original=pathlib.Path('/tmp/rusty-tiles-vector-f0-execution')/r['sourceRelativePath']
        else: original=pathlib.Path(index['sourceRoot'])/r['sourceRelativePath']
        assert b==pin(original),str(original)
        records[(name,r.get('sourceRelativePath',r.get('sourcePath')))]=b

p3=evidence/'p3-existing-producer-20261010';index=load(p3/'index.json')
assert sha(read(p3/'index.json'))=='7da201fc5d2e95785a76899b20dc18e199bed4aff26fadab437290a40573ef1e'
assert len(index['records'])==2603 and index['uniqueBlobs']==687
assert sha(read(p3/index['bundle']))==index['bundleSha256']=='f1396f4a0bd870e1ec8d40cdf1e7c4ef79df66f29800189fd5cdf1004702a924'
blobs={}
with tarfile.open(p3/index['bundle'],'r:gz') as tf:
    for member in tf:
        assert member.isfile(),member.name
        b=tf.extractfile(member).read()
        assert member.name=='sha256/'+sha(b)
        assert member.name not in blobs
        blobs[member.name]=b
assert len(blobs)==687
for r in index['records']:
    b=blobs[r['blobMember']]
    assert len(b)==r['bytes'] and sha(b)==r['sha256']
    assert b==pin(r['sourcePath']),r['sourcePath']
assert set(blobs)=={r['blobMember'] for r in index['records']}
v=load(ROOT/PREFIX/'precode_review/p3-actual-20261010/verification.json')
assert len(v['pairs'])==8
assert sum(p['slots'] for p in v['pairs'])==38
assert sum(p['views'] for p in v['pairs'])==402
for p in v['pairs']:
    if p['fill_wrapper_slots']:
        for f in p['framing_and_wrapper']:
            if f['wrapper'] is None: continue
            for key in ['raw','meshopt']:
                assert f[key]['json_absolute_end']%8!=0 and f[key]['bin_start']%8!=0
for p in v['pairs']:
    if p['large_id']: assert p['large_id']['rows']==65537
doc=read(ROOT/'docs/architecture/vector-compression-foundation.md').decode()
for target in re.findall(r'\]\(([^)]+)\)',doc):
    if not target.startswith('http'): assert (ROOT/'docs/architecture'/target).exists(),target
write('input-pin.json',inputs)
write('verification.json',{'status':'PASS_EXACT_CANDIDATE_DOCS_EVIDENCE_RETENTION_ONLY','candidateSha256':sha(read(PIN)),
    'candidateFiles':402,'candidateBytes':sum(p['bytes'] for p in d['files'].values()),'productionFilesMatchAcceptedMerge':97,
    'historicalBaseHead':d['baseHead'],'allExistingHistoricalFilesUnchangedExceptAuthoredCheckpoint':True,
    'originalPackageByteExactCopies':dict(copies),'originalPackageFiles':sum(copies.values()),
    'gzipRecords':{'p2':250,'f0OriginalFailure':6,'f0Actual':15},
    'p3Retention':{'indexSha256':sha(read(p3/'index.json')),'bundleSha256':index['bundleSha256'],'records':2603,'blobs':687,'everyBlobAndSourceByteIndependentlyVerified':True},
    'p3ReviewCrosschecks':{'pairs':8,'slots':38,'views':402,'fillAbsolute8JsonEndBinStartFailuresConfirmed':True},
    'compilerCargoNativeProducerDecoderOrAuthorCheckerInvocations':0,'gitMutations':0,'repositoryEdits':0,
    'scope':'read/hash/decompress/static documentation comparison and reused bounded prior independent review; no broad semantic execution or final future commit acceptance'})
print(json.dumps({'status':'PASS','candidateFiles':402,'originalCopies':sum(copies.values()),'gzipRecords':271,'p3Records':2603,'p3Blobs':687}))
