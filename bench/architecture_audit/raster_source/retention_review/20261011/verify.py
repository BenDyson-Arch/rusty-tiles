import gzip,hashlib,json,os,pathlib
os.setpriority(os.PRIO_PROCESS,0,10);os.sched_setaffinity(0,{0,1})
P=pathlib.Path;out=P(__file__).parent;p=P('/tmp/rusty-tiles-r2-foundation/bench/architecture_audit/raster_source/precode_evidence/20261011-followup/index.json')
raw=p.read_bytes();assert hashlib.sha256(raw).hexdigest()=='b6eae403f591e082e57e2ce1231e65ba0be90e43a29719d84cef8bbe38e10a87';j=json.loads(raw)
assert len(j['records'])==47 and len(j['roots'])==6 and j['prior_index_sha256']=='68de2ab0125f8b7140c1dc769bfc9640e90e2eccd8e8030075ac85f2aeb08b6a'
assert j['base_source_head']=='480a73eb4fa81b1e237c48f4a365ba78d8e30832' and j['develop_baseline']=='6c5ea2acac9fc403bfabfa9581464223b52259d7'
blobs={};keys=set();external=[];n=0
for r in j['records']:
 k=(r['epoch'],r['relative_path']);assert k not in keys;keys.add(k);assert r['kind']=='file'
 original=P(r['original_path']).read_bytes();assert len(original)==r['bytes'] and hashlib.sha256(original).hexdigest()==r['sha256']
 if r['storage']=='binary-metadata-only':
  assert 'blob' not in r;external.append({'path':r['original_path'],'bytes':r['bytes'],'sha256':r['sha256']});continue
 assert r['storage']=='lossless-gzip';gz=(p.parent/r['blob']).read_bytes();assert len(gz)==r['compressed_bytes'] and hashlib.sha256(gz).hexdigest()==r['compressed_sha256'] and gz[4:8]==bytes(4)
 assert gzip.decompress(gz)==original;n+=1;blobs[r['blob']]=len(gz)
assert n==44 and len(external)==3 and len(blobs)==39 and sum(blobs.values())==148070
README=P('/tmp/rusty-tiles-r2-foundation/bench/architecture_audit/raster_source/README.md');b=README.read_bytes()
v={'status':'PASS_FULL_FOLLOWUP_RETENTION_IDENTITY','index_sha256':hashlib.sha256(raw).hexdigest(),'records':47,'roots_retained':6,'lossless_records':44,'unique_blobs':39,'compressed_bytes':148070,'external_metadata_only':external,'all_existing_originals_equal':True,'gzip_mtime_zero':True,'prior_index_sha256':j['prior_index_sha256'],'README':{'path':str(README),'bytes':len(b),'sha256':hashlib.sha256(b).hexdigest()},'nice':os.getpriority(os.PRIO_PROCESS,0),'affinity':sorted(os.sched_getaffinity(0)),'scope':'Proof checkpoint docs/evidence retention only. 74 roots scanned by helper is not 74 retained roots. Exactly six retained here;44 byte-retained files and3 external identity records. No R2 source/product/merge/release acceptance; implementation authoring already permitted by separate gate09cb.'}
(out/'verification.json').write_text(json.dumps(v,indent=2)+'\n');print(json.dumps(v))
