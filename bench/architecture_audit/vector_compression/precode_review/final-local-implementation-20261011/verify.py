from pathlib import Path
import os,json,hashlib,gzip,subprocess
os.setpriority(os.PRIO_PROCESS,0,10);os.sched_setaffinity(0,{0,1})
root=Path('/tmp/rusty-tiles-vector-codec-foundation');out=Path('/tmp/rusty-tiles-vector-final-local-review-20261011');cache={}
def sha(p):
 p=Path(p)
 if str(p) in cache:return cache[str(p)]
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
 cache[str(p)]=h.hexdigest();return h.hexdigest()
def bind(p,h):
 assert sha(p)==h,(str(p),sha(p),h)
 return {'path':str(p),'sha256':h,'bytes':Path(p).stat().st_size}
def load(p):return json.loads(Path(p).read_text())
def git(*args):return subprocess.check_output(['git',*args],cwd=root)
p=Path('/tmp/rusty-tiles-vector-final-local-source-package-20261011/package.json');j=load(p);r={'setup':{'nice':os.getpriority(os.PRIO_PROCESS,0),'affinity':sorted(os.sched_getaffinity(0))},'package':bind(p,'224d3d96f644b7da21f8753d005da285c756d2ed17fc13ace02565b190c98460')}
assert git('rev-parse','HEAD').decode().strip()==j['head']=='a60c216caef35cca68a5c36969457c83c08d41e9';tree=git('write-tree').decode().strip();assert tree=='e47d63e78ede054b6853ddde5af48f9f67e8471c' and git('diff','--name-only')==b''
r['stagedTree']=tree;r['head']=j['head'];r['packageFiles']=[bind(root/path,h) for path,h in j['files'].items()];assert len(j['files'])==111
prod=load(j['productionPin']);bind(j['productionPin'],j['productionPinSha256']);assert len(prod['production_sha256'])==108
for k,h in prod['production_sha256'].items():assert j['files'][k]==h
names=git('diff','--cached','--name-only','-z').decode().rstrip('\0').split('\0');changedprod=[n for n in names if n in prod['production_sha256']];assert len(changedprod)==27 and set(changedprod)==set(prod['changedProductionPaths'])
assert all(n in changedprod or n.startswith('bench/architecture_audit/vector_compression/') or n in ['docs/architecture/vector-compression-foundation.md','docs/architecture/public-surface-inventory.md'] for n in names)
r['stagedCounts']={'all':len(names),'production':27,'docs':2,'evidence':len(names)-29};r['stagedPathsSha256']=hashlib.sha256('\n'.join(names).encode()).hexdigest()
# Bind each staged object to current filesystem, including the blob gzip bytes and
# precode records outside the implementation index. Read-only Git objects only.
for n in names:
 b=git('show',':'+n);assert hashlib.sha256(b).hexdigest()==sha(root/n)
idx=root/'bench/architecture_audit/vector_compression/candidate_evidence/local-implementation-20261011/index.json';i=load(idx);r['index']=bind(idx,'b1086f6d56def7e3d1e61f292bb719448df4816ee50e7587a4e8ace6bc0ed3fd');assert i['productionPinSha256']==j['productionPinSha256'] and i['baseCommit']==j['head'] and len(i['records'])==5062 and len(i['externalCompiledArtifacts'])==341
assert len({x['sourcePath'] for x in i['records']})==5062
blobs={};totalbytes=0
for x in i['records']:
 bp=idx.parent/x['file'];assert x['file']=='blobs/'+x['sha256']+'.gz';bind(bp,x['gzipSha256']);assert bp.stat().st_size==x['gzipBytes'];v=bind(x['sourcePath'],x['sha256']);assert v['bytes']==x['bytes'];decoded=gzip.decompress(bp.read_bytes());assert len(decoded)==x['bytes'] and hashlib.sha256(decoded).hexdigest()==x['sha256'] and decoded==Path(x['sourcePath']).read_bytes()
 if x['file'] not in blobs:blobs[x['file']]={'sha256':x['gzipSha256'],'bytes':x['gzipBytes']};totalbytes+=x['gzipBytes']
 else:assert blobs[x['file']]=={'sha256':x['gzipSha256'],'bytes':x['gzipBytes']}
assert len(blobs)==i['uniqueBlobs']==1743 and totalbytes==i['uniqueCompressedBytes']==13613074
assert set(x.name for x in (idx.parent/'blobs').iterdir())=={Path(k).name for k in blobs}
for x in i['externalCompiledArtifacts']:
 v=bind(x['sourcePath'],x['sha256']);assert v['bytes']==x['bytes'] and 'bytes not bundled' in x['scope']
r['retention']={'fullOriginalByteRecords':5062,'uniqueGzipBlobs':1743,'gzipBytes':13613074,'externalCompiledMetadata':341,'allOriginalIdentityAndGzipChecksPass':True}
script=Path('/tmp/rusty-tiles-vector-retain-local-implementation-20261011.py');r['retentionScript']=bind(script,sha(script));assert str(script) in {x['sourcePath'] for x in i['records']}
r['priorIndependentReviews']=[]
for name,h in [('correction6','b4df089b7bc727eefd26d85f9a6c6e8f4c6010b3bd8cf5177357c943c6a16a13'),('actual-regression1','50f991482bb924b89d0d25d90c26e806aa9b8e4f6df14f4e5de9e86fd3aa96f2'),('actual-operation-limits1','be3ea12a95a1acddc9c77be68598816f896ebad74739baec98defb1271bced35'),('actual-native-producer1','12c7a28d811d1168cfe8fe1dd54a4e1650db8e21e76b015a3d00ad18868e404b')]:
 p=Path('/tmp/rusty-tiles-vector-integrated-source-review')/name/'integrity.json';r['priorIndependentReviews'].append(bind(p,h));inte=load(p)
 for fn,x in inte['files'].items():bind(p.parent/fn,x['sha256'])
# Every selected expected artifact/data path in supplied binding manifests is
# retained losslessly or explicitly listed external, never silently omitted.
paths={x['sourcePath'] for x in i['records']}|{x['sourcePath'] for x in i['externalCompiledArtifacts']}
for m in ['/tmp/rusty-tiles-vector-acceptance-artifact-binding-revision3/manifest.json','/tmp/rusty-tiles-vector-regression-inputs-20261011/manifest.json']:
 for x in load(m)['files']:
  q=Path(x['path']);q=q if q.is_absolute() else root/q;assert str(q) in paths
r['hashPaths']=len(cache);assert git('write-tree').decode().strip()==tree and git('diff','--name-only')==b''
(out/'verification.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps({'staged':r['stagedCounts'],'retention':r['retention'],'hashPaths':len(cache),'tree':tree}));print('FINAL_LOCAL_PACKAGE_ALL_HASH_AND_BYTEIDENTITY_CHECKS_PASS')
