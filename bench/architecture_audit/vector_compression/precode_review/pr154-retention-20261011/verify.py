import pathlib, hashlib, gzip, json, os, subprocess

os.setpriority(os.PRIO_PROCESS,0,10)
os.sched_setaffinity(0,{0,1})
OUT=pathlib.Path(__file__).resolve().parent
ROOT=pathlib.Path('/tmp/rusty-tiles-r2-foundation')
BUNDLE=ROOT/'bench/architecture_audit/vector_compression/candidate_evidence/pr154-remote-accepted-20261011'
REMOTE=pathlib.Path('/tmp/rusty-tiles-vector-final-remote-evidence-20261011-e9fb073')
HEAD='e9fb073184ad25a1f5c7839ea5d1cd9e02830887'
BASE='f5401dd120441b4f9aac5ad229dbfffe03fe4428'
MERGE='6c5ea2acac9fc403bfabfa9581464223b52259d7'
TREE='e395a4c22e8b0e86d3c72169c332fbea1e5d8eb5'
def sha(b): return hashlib.sha256(b).hexdigest()
def load(p): return json.loads(p.read_bytes())
def gh(endpoint,name):
    b=subprocess.check_output(['gh','api','repos/BenDyson-Arch/rusty-tiles/'+endpoint])
    (OUT/name).write_bytes(b)
    return json.loads(b)
index_bytes=(BUNDLE/'index.json').read_bytes()
assert sha(index_bytes)=='7199bf434519aebd237fe30f943004368a19e8f7b7d292f60a7b7b82f873c123'
index=json.loads(index_bytes)
assert index['head']==HEAD and index['reviewedTree']==TREE and index['developMerge']==MERGE
assert index['finalReviewIntegritySha256']=='65309b72dd9155bbd9def16fb7e112974095e5af99708c89f8dd735ea9e03978'
assert len(index['records'])==234 and len(index['externalDownloadedBinaries'])==20
records={r['sourcePath']:r for r in index['records']}
assert len(records)==234
blobs={}
for rec in index['records']:
    assert rec['file']=='blobs/'+rec['sha256']+'.gz'
    original=pathlib.Path(rec['sourcePath']).read_bytes()
    compressed=(BUNDLE/rec['file']).read_bytes()
    assert len(original)==rec['bytes'] and sha(original)==rec['sha256']
    assert len(compressed)==rec['gzipBytes'] and sha(compressed)==rec['gzipSha256']
    assert gzip.decompress(compressed)==original
    blobs[rec['file']]=rec['gzipBytes']
assert len(blobs)==index['uniqueBlobs']==165
assert sum(blobs.values())==index['uniqueCompressedBytes']==1420841
assert {str(p.relative_to(BUNDLE)) for p in (BUNDLE/'blobs').glob('*')}==set(blobs)
roots=[REMOTE,pathlib.Path('/tmp/rusty-tiles-vector-pr154-source-review-20261011'),pathlib.Path('/tmp/rusty-tiles-vector-pr154-final-remote-review-20261011')]
expected={str(p) for r in roots for p in r.rglob('*') if p.is_file() and p.suffix not in ['.zip','.whl']}
for p in ['rusty-tiles-vector-capture-remote-154.py','rusty-tiles-vector-collect-pr154-artifacts.py','rusty-tiles-vector-collect-pr154-job-logs.py','rusty-tiles-vector-retain-pr154-accepted-20261011.py']: expected.add('/tmp/'+p)
assert set(records)==expected
external={r['sourcePath']:r for r in index['externalDownloadedBinaries']}
assert len(external)==20 and not(set(external)&set(records))
expected_external={}
for folder in (REMOTE/'artifacts').iterdir():
    rec=load(folder/'receipt.json')
    expected_external[str(folder/'artifact.zip')]={'bytes':rec['archiveBytes'],'sha256':rec['archiveSha256']}
    for member in rec['members']:
        if member['path'].endswith('.whl'): expected_external[str(folder/'files'/member['path'])]={'bytes':member['bytes'],'sha256':member['sha256']}
assert set(external)==set(expected_external)
for path,facts in expected_external.items():
    assert {k:external[path][k] for k in ['bytes','sha256']}==facts
    assert pathlib.Path(path).stat().st_size==facts['bytes']
    assert 'external' in external[path]['scope'] and 'file' not in external[path]
assert sha(pathlib.Path('/tmp/rusty-tiles-vector-pr154-final-remote-review-20261011/integrity.json').read_bytes())==index['finalReviewIntegritySha256']
before=load(REMOTE/'merge/pr-immediate-before.json')
assert before['head']['sha']==HEAD and before['base']['sha']==BASE and before['state']=='open'
command=load(REMOTE/'merge/command.json')
assert command['returncode']==0 and command['argv']==['gh','pr','merge','154','--repo','BenDyson-Arch/rusty-tiles','--merge','--match-head-commit',HEAD]
pr=gh('pulls/154','merged-pr-live.json')
commit=gh('git/commits/'+MERGE,'merge-commit-live.json')
develop=gh('branches/develop','develop-live.json')
assert pr['merged'] is True and pr['state']=='closed' and pr['head']['sha']==HEAD and pr['merge_commit_sha']==MERGE
assert commit['tree']['sha']==TREE and [p['sha'] for p in commit['parents']]==[BASE,HEAD]
assert develop['commit']['sha']==MERGE
local=subprocess.check_output(['git','show','--no-patch','--format=%T%n%P',MERGE],cwd=ROOT).decode().splitlines()
assert local==[TREE,BASE+' '+HEAD]
result={'status':'PASS_LOSSLESS_PR154_RETENTION_AND_ACCEPTED_MERGE_IDENTITY','indexSha256':sha(index_bytes),'head':HEAD,'base':BASE,'merge':MERGE,'tree':TREE,'fullOriginalRecordsCompared':234,'uniqueGzipBlobsCompared':165,'uniqueCompressedBytes':1420841,'sourceDirectoriesCompletelyRepresented':True,'externalMetadataMatchedAcceptedArtifactReceipts':20,'externalBinaryBytesRetained':False,'externalBinaryHashReplay':False,'actualMergedAt':pr['merged_at'],'developAtAcceptedMerge':True,'reviewerNice':os.getpriority(os.PRIO_PROCESS,0),'reviewerAffinity':sorted(os.sched_getaffinity(0)),'scope':index['scope'],'newProductOrPlatformExecution':False,'r2SourceOrPolicyAdjudicated':False}
(OUT/'verification.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
