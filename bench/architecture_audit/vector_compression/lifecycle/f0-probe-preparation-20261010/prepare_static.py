"""Static source copying/fingerprint closure preparation only; never builds or runs targets."""
from pathlib import Path
import hashlib,json,re,tomllib,shutil
OUT=Path(__file__).parent
SRC=Path('/tmp/rusty-tiles-vector-codec-foundation')
TARGET=Path('/tmp/rusty-tiles-f1d2-final-target/release')
ACCEPT=Path('/tmp/rusty-tiles-payload-final-evidence/remote-4dc8f0c')
PROOF=Path('/tmp/rusty-tiles-next-vector-compression-audit/lifecycle/proof-inputs-20261010-4dc')
sha=lambda b:hashlib.sha256(b).hexdigest()
pins={}
def pin(p,large=False):
 p=Path(p);r={'path':str(p),'bytes':p.stat().st_size,'resolvedPath':str(p.resolve())}
 if p.stat().st_size<=1048576 and not large:r['sha256']=sha(p.read_bytes())
 else:r.update(sha256=None,hashStatus='ROOT_COORDINATOR_HASH_AND_FREEZE_REQUIRED')
 pins[str(p)]=r;return r
accepted=json.loads((ACCEPT/'correction-source-pin.json').read_text());merge=json.loads((ACCEPT/'merge-source-verification.json').read_text())
assert accepted['source_commit']==merge['acceptedHead']=='4dc8f0c04dd5a98420768d05c6787f3baa49bb75'
assert merge['acceptedMergeCommit']=='f5401dd120441b4f9aac5ad229dbfffe03fe4428'
originals=['Cargo.lock','Cargo.toml','src/runtime.rs','src/runtime/tests.rs','src/runtime/directory.rs','src/runtime/directory_platform.rs','src/error.rs','src/main.rs','bindings/python/src/lib.rs','src/vector.rs','src/vector/pipeline/acceptance.rs','src/vector/pipeline/store.rs','src/vector/pipeline/encoding.rs']
for rel in originals:
 p=SRC/rel;raw=p.read_bytes();assert sha(raw)==accepted['production_sha256'][rel],rel
 dst=OUT/'inputs/accepted'/rel;dst.parent.mkdir(parents=True,exist_ok=True);dst.write_bytes(raw);pin(p)
for name in ['correction-source-pin.json','merge-source-verification.json']:
 raw=(ACCEPT/name).read_bytes();(OUT/'inputs'/name).write_bytes(raw);pin(ACCEPT/name)
for rel in ['src/runtime.rs','src/runtime/directory.rs','src/runtime/directory_platform.rs']:
 dst=OUT/'isolated'/rel;dst.parent.mkdir(parents=True,exist_ok=True);dst.write_bytes((SRC/rel).read_bytes())
tests=(SRC/'src/runtime/tests.rs').read_text();start=tests.index("fn candidate<'a>");end=tests.index('\nfn sealed',start);helper=tests[start:end]
testbody='use super::*;\nuse std::io::Write;\n\n'+helper+'\n\ninclude!("../../probes/p5_f0_seams.rs");\n'
(OUT/'isolated/src/runtime/tests.rs').write_text(testbody)
probe=(PROOF/'p5_f0_seams.rs.in').read_bytes();prepared=probe.decode()
prepared=prepared.replace('struct CompetitorAfterFinalCheck;','struct CompetitorAfterFinalCheck { reached: std::cell::Cell<bool> }')
prepared=prepared.replace('assert_eq!(policy, OutputPolicy::Replace);','assert_eq!(policy, OutputPolicy::Replace);\n            self.reached.set(true);')
prepared=prepared.replace('fs::write(output, b"competitor after source recheck").unwrap();','fs::write(output, b"competitor after source recheck").unwrap();\n            assert_eq!(fs::read(output).unwrap(), b"competitor after source recheck");')
prepared=prepared.replace('let published = staged.seal()', 'let files = CompetitorAfterFinalCheck { reached: std::cell::Cell::new(false) };\n    let published = staged.seal()')
prepared=prepared.replace('&output, OutputPolicy::Replace, &CompetitorAfterFinalCheck,','&output, OutputPolicy::Replace, &files,')
prepared=prepared.replace('assert_eq!(published.output, output);','assert!(files.reached.get(), "install interference seam must be reached");\n    assert_eq!(published.output, output);')
prepared=prepared.replace('staged.writer().set_permissions(permissions).unwrap();','assert_eq!(staged.writer().metadata().unwrap().permissions().mode() & 0o077, 0);\n    staged.writer().set_permissions(permissions).unwrap();\n    assert_eq!(staged.writer().metadata().unwrap().permissions().mode() & 0o777, 0o640);')
prepared=prepared.replace('staged.writer().set_permissions(readonly).unwrap();','staged.writer().set_permissions(readonly).unwrap();\n    assert!(staged.writer().metadata().unwrap().permissions().readonly());')
(OUT/'isolated/probes').mkdir(exist_ok=True);(OUT/'isolated/probes/p5_f0_seams.rs').write_text(prepared)
(OUT/'inputs/p5_f0_seams.rs.in').write_bytes(probe);pin(PROOF/'p5_f0_seams.rs.in')
(OUT/'isolated/f0_probe.rs').write_text('#![allow(dead_code)]\n#[path="src/runtime.rs"]\nmod runtime;\npub use runtime::{CancellationHandle, RunControl, RunEvent};\n')
(OUT/'source-differences.json').write_text(json.dumps({'acceptedMerge':merge['acceptedMergeCommit'],'unchangedRuntimeAndDirectory':['isolated/src/runtime.rs','isolated/src/runtime/directory.rs','isolated/src/runtime/directory_platform.rs'],'originalTestsRetained':'inputs/accepted/src/runtime/tests.rs','isolatedTests':'only accepted candidate helper + imports + original3 P5 prepared tests with explicit reached/precondition assertions; accepted inline directory tests compile but are filtered out of actual run','probeAdaptations':['Cell reached flag + read competing bytes inside install and assert seam reached after publication','Unix candidate initially no group/other bits then copied0640 assertion before seal','Windows candidate readonly assertion before seal; original target remains writable'],'candidateHelperSourceByteStart':start,'candidateHelperSourceByteEnd':end,'candidateHelperSha256':sha(helper.encode()),'p5InputSha256':sha(probe),'preparedProbeSha256':sha(prepared.encode()),'productionEdits':False,'targetExecutions':0},indent=2)+'\n')
lock=tomllib.loads((SRC/'Cargo.lock').read_text());locks={(p['name'],p['version']):p for p in lock['package']}
units={};idx={}
for p in sorted((TARGET/'.fingerprint').glob('*/*.json')):
 f=p.with_suffix('')
 if not f.exists():continue
 try:integer=int.from_bytes(bytes.fromhex(f.read_text().strip()),'little')
 except ValueError:continue
 units[str(p)]={'p':p,'value':json.loads(p.read_text()),'integer':integer};idx.setdefault(integer,[]).append(str(p))
root=TARGET/'.fingerprint/rusty-tiles-fe8339c670388461/lib-rusty_tiles.json';rootv=json.loads(root.read_text());pin(root)
roots={name:idx[number] for _,name,_,number in rootv['deps']if name in ['tempfile','same_file','libc']}
assert all(len(v)==1 for v in roots.values())and len(roots)==3
visited={};unresolved=[]
def visit(name,role):
 if name in visited:
  if role=='compiled':visited[name]['role']='compiled'
  return
 u=units[name];p=u['p'];v=u['value'];pkg,suffix=p.parent.name.rsplit('-',1)
 n={'unit':str(p),'role':role,'fingerprintInteger':u['integer'],'fingerprintFile':pin(p.with_suffix('')),'fingerprintJson':pin(p),'features':json.loads(v.get('features')or'[]'),'rustcId':v.get('rustc'),'profile':v.get('profile'),'rustflags':v.get('rustflags'),'config':v.get('config'),'edges':[]};visited[name]=n
 if p.stem.startswith('lib-'):
  crate=p.stem.removeprefix('lib-');arts=[q for q in [TARGET/'deps'/f'lib{crate}-{suffix}.rlib',TARGET/'deps'/f'lib{crate}-{suffix}.so']if q.exists()];n['artifacts']=[pin(a,True)for a in arts];assert len(arts)==1
  dep=TARGET/'deps'/f'{crate}-{suffix}.d';n['depInfo']=pin(dep)
  dirs=set(re.findall(r'(/[\S:]+/registry/src/[^/]+/[^/]+)/',dep.read_text()));assert len(dirs)==1,dirs
  src=Path(next(iter(dirs)));cargo=tomllib.loads((src/'Cargo.toml').read_text());namever=(cargo['package']['name'],cargo['package']['version']);locked=locks[namever]
  cached=Path('/home/bend/.cargo/registry/cache/index.crates.io-1949cf8c6b5b557f')/(namever[0]+'-'+namever[1]+'.crate');registry=pin(cached);registry['sha256']=sha(cached.read_bytes());registry.pop('hashStatus',None);assert registry['sha256']==locked['checksum']
  n['package']={'name':namever[0],'version':namever[1],'lockChecksum':locked['checksum'],'cachedArchive':registry,'cargoToml':pin(src/'Cargo.toml'),'depInfoSourceDirectory':str(src),'proofLimit':'Current registry metadata/dep-info, not historical extracted build-source immutability'}
 for packageid,depname,public,depid in v.get('deps',[]):
  hits=idx.get(depid,[]);e={'name':depname,'integer':depid,'candidateUnits':hits,'resolutionUnique':len(hits)==1};n['edges'].append(e)
  if len(hits)!=1:unresolved.append(e)
  else:visit(hits[0],'historical_build_provenance'if role=='historical_build_provenance'or depname=='build_script_build'else'compiled')
for name,values in roots.items():visit(values[0],'compiled')
compiled=[v for v in visited.values()if v['role']=='compiled'];assert not unresolved
for n in compiled:
 if n['package']['name']in ['tempfile','same-file','libc']:assert n['package']['version']=={'tempfile':'3.27.0','same-file':'1.0.6','libc':'0.2.189'}[n['package']['name']]
closure={'status':'STATIC_MATCHED_ARTIFACT_CLOSURE_RECOMMENDATION_NOT_EXECUTED','rootFingerprint':pin(root),'rootEdges':roots,'compiled':compiled,'historicalBuildProvenance':[v for v in visited.values()if v['role']!='compiled'],'unresolvedEdges':unresolved,'directNeed':{'tempfile':'Actual runtime and file probe dependency','libc':'Unchanged accepted inline Linux directory module compiles under cfg(test)','same_file':'Matched1.0.6 identity dependency prepared for future captured-source/handle work; current two file publication probes do not use it, so no same-file primitive execution acceptance follows'},'compilerBinding':'Repin selected rustc/std/driver/LLVM and actual linker/loader as in exact P2 receipt; augment with these compiled artifacts before/after. No new dependency closure complete flag from this static package. Linux compiled artifacts are not Windows artifacts.','nativeMeaning':'Native-host x86_64 Linux crate closure includes rustix/linux_raw_sys/libc filesystem interfaces; no meshopt C++ dependency in this probe.','historicalBuildLimit':'Build script/run records identify original compilation provenance. Actual root rustc consumes rlibs and their bundled code; current extracted source agreement does not prove original source immutability.'}
(OUT/'closure-recommendation.json').write_text(json.dumps(closure,indent=2)+'\n')
(OUT/'root-artifact-hash-candidates.txt').write_text(''.join(a['path']+'\n'for n in compiled for a in n['artifacts']))
registry=Path('/home/bend/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f')
for rel in ['tempfile-3.27.0/Cargo.toml','tempfile-3.27.0/src/file/imp/windows.rs','tempfile-3.27.0/src/file/imp/unix.rs','same-file-1.0.6/Cargo.toml','same-file-1.0.6/src/lib.rs','same-file-1.0.6/src/unix.rs','same-file-1.0.6/src/win.rs']:
 p=registry/rel;dst=OUT/'inputs/dependencies'/rel;dst.parent.mkdir(parents=True,exist_ok=True);dst.write_bytes(p.read_bytes());pin(p)
(OUT/'input-pins.json').write_text(json.dumps({'source':'accepted merged C1 production bytes read-only','files':pins},indent=2)+'\n')
print(json.dumps({'compiledUnits':len(compiled),'historicalBuildUnits':len(visited)-len(compiled),'unresolvedEdges':len(unresolved),'roots':roots,'sourceEdits':False,'probeExecutions':0},indent=2))
