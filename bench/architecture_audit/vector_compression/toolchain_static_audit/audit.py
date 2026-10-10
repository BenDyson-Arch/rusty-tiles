#!/usr/bin/env python3
"""Static artifact inspection only; does not invoke Cargo, rustc, or any probe."""
from pathlib import Path
import hashlib,json,tomllib,re,subprocess,os,tarfile
OUT=Path(__file__).resolve().parent
P2=Path('/tmp/rusty-tiles-next-vector-compression-audit/codec/p2-proof-preparation-20261010')
TARGET=Path('/tmp/rusty-tiles-f1d2-final-target/release')
SYS=Path('/home/bend/.rustup/toolchains/stable-x86_64-unknown-linux-gnu')
C1=Path('/tmp/rusty-tiles-c1-payload-foundation')
sha=lambda b:hashlib.sha256(b).hexdigest()
def pin(p,hash_small=True):
 p=Path(p);r={'path':str(p),'exists':p.exists()}
 if p.exists():
  r.update(bytes=p.stat().st_size,resolvedPath=str(p.resolve()),isSymlink=p.is_symlink())
  r['sha256']=sha(p.read_bytes()) if hash_small and p.stat().st_size<=1024*1024 else None
  if r['sha256'] is None:r['hashStatus']='ROOT_COORDINATOR_HASH_REQUIRED'
 return r
lockfile=P2/'inputs/source/Cargo.lock';lock=tomllib.loads(lockfile.read_text())
locks={(p['name'],p['version']):p for p in lock['package']}
idx={};units={}
for p in sorted((TARGET/'.fingerprint').glob('*/*.json')):
 f=p.with_suffix('')
 if not f.exists():continue
 try:integer=int.from_bytes(bytes.fromhex(f.read_text().strip()),'little')
 except ValueError:continue
 units[str(p)]={'path':p,'value':json.loads(p.read_text()),'integer':integer}
 idx.setdefault(integer,[]).append(str(p))
roots=[TARGET/'.fingerprint'/x for x in ['serde_json-60507217a1b3d3c8/lib-serde_json.json','serde-aa8e5dbbc29a15a8/lib-serde.json','meshopt-5887a2583d9f345c/lib-meshopt.json']]
visited={};unresolved=[]
def visit(p,role):
 key=str(p)
 if key in visited:
  if role=='recordedCompiledClosure':visited[key]['role']=role
  return
 u=units[key];v=u['value'];dname=p.parent.name;pkg,suffix=dname.rsplit('-',1)
 name=p.stem.removeprefix('lib-')
 node={'unit':dname+'/'+p.stem,'role':role,'fingerprintInteger':u['integer'],'fingerprintFile':pin(p.with_suffix('')),'fingerprintJson':pin(p),'fingerprintRustcId':v.get('rustc'),'features':json.loads(v['features']) if v.get('features') else [],'profile':v.get('profile'),'rustflags':v.get('rustflags'),'config':v.get('config'),'compileKind':v.get('compile_kind'),'edges':[]}
 visited[key]=node
 if p.stem.startswith('lib-'):
  artifacts=[q for q in [TARGET/'deps'/f'lib{name}-{suffix}.rlib',TARGET/'deps'/f'lib{name}-{suffix}.so'] if q.exists()]
  node['artifacts']=[pin(q,False) for q in artifacts]
  if len(artifacts)!=1:unresolved.append({'unit':node['unit'],'reason':'Expected exactly one actual rlib or proc-macro shared artifact','candidates':[str(q) for q in artifacts]})
  node['artifactKind']='procMacroSharedLibrary' if artifacts and artifacts[0].suffix=='.so' else 'rlib'
  dep=TARGET/'deps'/f'{name}-{suffix}.d';node['depInfo']=pin(dep)
  if dep.exists():
   dirs=sorted(set(re.findall(r'(/[^\s:]+/registry/src/[^/]+/[^/]+)/',dep.read_text())))
   node['recordedRegistrySourceDirectories']=dirs
   if len(dirs)==1:
    src=Path(dirs[0]);cargo=src/'Cargo.toml';data=tomllib.loads(cargo.read_text());n=data['package']['name'];ver=data['package']['version'];lp=locks.get((n,ver));checksum=src/'.cargo-checksum.json';cs=json.loads(checksum.read_text()) if checksum.exists() else {}
    crate=Path('/home/bend/.cargo/registry/cache/index.crates.io-1949cf8c6b5b557f')/f'{n}-{ver}.crate';cratepin=pin(crate);packagehash=cratepin.get('sha256') or cs.get('package')
    node['package']={'name':n,'version':ver,'cargoToml':pin(cargo),'registryChecksumFile':pin(checksum),'registryChecksum':packagehash,'cachedRegistryArchive':cratepin,'lockRegistryChecksum':lp.get('checksum') if lp else None,'matchesFrozenLockRegistryChecksum':bool(lp and packagehash==lp.get('checksum')),'proofLimit':'Registry package checksum metadata and dep-info provenance; does not prove all extracted source bytes were unmodified at original compilation.'}
    if not node['package']['matchesFrozenLockRegistryChecksum']:unresolved.append({'unit':node['unit'],'reason':'Registry checksum not resolved/matched to frozen lock'})
 elif p.stem.startswith('run-build'):
  b=TARGET/'build'/dname;node['buildProvenance']=[pin(q) for q in sorted(b.glob('*')) if q.is_file()];node['generatedOutputInventory']=[pin(q) for q in sorted((b/'out').rglob('*')) if q.is_file()]
 elif p.stem.startswith('build-script'):
  b=TARGET/'build'/dname;node['buildScriptArtifacts']=[pin(q,False) for q in sorted(b.glob('*')) if q.is_file()]
 for packageid,depname,public,depid in v.get('deps',[]):
  hits=idx.get(depid,[]);edge={'dependencyName':depname,'dependencyFingerprintInteger':depid,'candidateUnits':hits,'resolutionUnique':len(hits)==1,'cargoPackageIdHash':packageid,'public':public};node['edges'].append(edge)
  if len(hits)!=1:unresolved.append({'unit':node['unit'],'edge':edge})
  else:
   nextrole='historicalBuildProvenance' if role=='historicalBuildProvenance' or depname=='build_script_build' else 'recordedCompiledClosure'
   visit(Path(hits[0]),nextrole)
for p in roots:visit(p,'recordedCompiledClosure')
# Source equality only, never execute the frozen helpers/probes.
comparisons=[]
frozenpins=json.loads((P2/'inputs/pins.json').read_text())
originals={str(Path(x['file'])):Path(x['original']) for x in frozenpins['files']}
for prefix,actual in [('toolchain',SYS),('source',C1)]:
 for p in sorted((P2/'inputs'/prefix).rglob('*')):
  if p.is_file():
   q=originals.get(str(p.relative_to(P2/'inputs')),actual/p.relative_to(P2/'inputs'/prefix))
   comparisons.append({'kind':prefix,'frozen':pin(p),'actual':pin(q),'byteEqual':q.exists() and p.read_bytes()==q.read_bytes()})
for p in sorted((P2/'inputs/dependencies').rglob('*')):
 if p.is_file():
  rel=p.relative_to(P2/'inputs/dependencies');srcroot=Path('/home/bend/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f');q=srcroot/rel
  crate=Path('/home/bend/.cargo/registry/cache/index.crates.io-1949cf8c6b5b557f')/(rel.parts[0]+'.crate')
  with tarfile.open(crate,'r:gz') as tar:
   registry_raw=tar.extractfile(str(rel)).read()
  registry_expected=sha(registry_raw);actual_sha=sha(q.read_bytes()) if q.exists() else None
  comparisons.append({'kind':'dependencies','frozen':pin(p),'actual':pin(q),'byteEqual':q.exists() and p.read_bytes()==q.read_bytes(),'actualMatchesRegistryFileChecksum':registry_expected==actual_sha,'registryFileSha256':registry_expected})
manifest=SYS/'lib/rustlib/multirust-channel-manifest.toml';mf=tomllib.loads(manifest.read_text());host='x86_64-unknown-linux-gnu'
manifestpins={n:{'version':mf['pkg'][n]['version'],'distribution':mf['pkg'][n]['target']['*' if n=='rust-src' else host]} for n in ['rustc','rust-std','rust-src']}
# Inspect archives by GNU ar container bytes, not by executing native objects.
def members(path):
 raw=Path(path).read_bytes();assert raw[:8]==b'!<arch>\n';at=8;table=b'';out=[]
 while at<len(raw):
  h=raw[at:at+60];assert h[58:60]==b'`\n';sz=int(h[48:58]);data=raw[at+60:at+60+sz];name=h[:16].decode().strip()
  if name=='//':table=data
  elif name not in ['/', '/SYM64/']:
   if name.startswith('/') and name[1:].isdigit():name=table[int(name[1:]):].split(b'/\n',1)[0].decode()
   else:name=name.rstrip('/')
   out.append({'name':name,'bytes':len(data),'sha256':sha(data) if len(data)<=1024*1024 else None})
  at+=60+sz+(sz%2)
 return out
meshlib=TARGET/'deps/libmeshopt-5887a2583d9f345c.rlib';archivepaths=[TARGET/'build/meshopt-5643d4cfd1ac4a85/out/libmeshopt_cpp.a',Path('/home/bend/.cache/rusty-tiles-115-target/release/build/meshopt-5643d4cfd1ac4a85/out/libmeshopt_cpp.a')]
meshmembers=members(meshlib);archives=[]
for q in archivepaths:
 ms=members(q);archives.append({'archive':pin(q),'members':ms,'allMembersByteIdenticalInRlib':all(any(n['name']==m['name'] and n['sha256']==m['sha256'] and n['sha256'] is not None for n in meshmembers) for m in ms)})
# Static ELF DT_NEEDED inventory. Candidate paths are not dynamic-loader execution receipts.
# Conservative hash set for sysroot and possible linker closure, with exact selection held for coordinator.
stdmanifest=SYS/'lib/rustlib/manifest-rust-std-x86_64-unknown-linux-gnu'
stdpaths=[SYS/line.removeprefix('file:') for line in stdmanifest.read_text().splitlines() if line.startswith('file:')]
stdpaths=[p for p in stdpaths if p.suffix in ['.rlib','.rmeta','.so','.o','.a']]
linkcandidates=[Path('/usr/bin/cc'),Path('/usr/bin/gcc'),Path('/usr/bin/collect2'),Path('/usr/bin/ld'),SYS/'lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld',SYS/'lib/rustlib/x86_64-unknown-linux-gnu/bin/gcc-ld/ld.lld']
for base in [Path('/usr/lib'),Path('/usr/lib/gcc')]:
 for pattern in ['crt*.o','Scrt1.o','crti.o','crtn.o','libc.so','libc_nonshared.a','libgcc.a','libgcc_eh.a','libgcc_s.so','libstdc++.so','specs','collect2']:
  linkcandidates.extend(base.glob(pattern) if base==Path('/usr/lib') else base.glob('**/'+pattern))
linkcandidates=sorted(set(p for p in linkcandidates if p.exists()))
compiler=[SYS/'bin/rustc',SYS/'lib/librustc_driver-28a98848f7a7c026.so',SYS/'lib/libLLVM.so.22.1-rust-1.98.0-stable',SYS/'lib/libLLVM-22-rust-1.98.0-stable.so']
elfqueue=compiler+linkcandidates+[Path(a['path']) for n in visited.values() for a in n.get('artifacts',[]) if n.get('artifactKind')=='procMacroSharedLibrary']
elfrecords={}
while elfqueue:
 p=elfqueue.pop(0);key=str(p.resolve())
 if key in elfrecords or not p.exists():continue
 result=subprocess.run(['/usr/bin/readelf','-d',str(p)],capture_output=True,text=True,check=False)
 need=re.findall(r'Shared library: \[([^]]+)\]',result.stdout);paths=re.findall(r'(?:runpath|rpath): \[([^]]+)\]',result.stdout,re.I)
 rec={'artifact':pin(p,False),'needed':need,'readelfExitCode':result.returncode,'rpathOrRunpath':paths,'staticInspectionTool':'/usr/bin/readelf -d','candidateDependencies':[]};elfrecords[key]=rec
 for n in need:
  qs=[q for q in [p.parent/n,SYS/'lib'/n,Path('/usr/lib')/n] if q.exists()];dedup={str(q.resolve()):q for q in qs};qs=list(dedup.values())
  rec['candidateDependencies'].append({'soname':n,'candidates':[pin(q,False) for q in qs],'dynamicLoaderSelectionVerified':False})
  elfqueue.extend(qs)
rootbindings=[]
for d in sorted((TARGET/'.fingerprint').glob('rusty-tiles-*')):
 for p in d.glob('*.json'):
  v=json.loads(p.read_text())
  if v.get('features')=='[]' and any(x[1]=='serde_json' and x[3]==406739774006881114 for x in v.get('deps',[])):
   rootbindings.append({'fingerprintJson':pin(p),'features':[],'directEdges':[x for x in v['deps'] if x[1] in ['serde_json','serde','meshopt']]})
historical=[pin(Path('/tmp/rusty-tiles-payload-final-evidence/stride-final-build')/n) for n in ['library-receipt.json','portable-cli-receipt.json','native-cli-receipt.json']]
result={'status':'STATIC_CLOSURE_AUDIT_ONLY_ROOT_EXECUTION_REQUIRED','compilerExecutedByAuditor':False,'nativeOrModelProbeExecutedByAuditor':False,'dependencyClosureComplete':False,'stdSourceMatchedToCompiler':False,'acceptedMergeCommit':None,'frozenCargoLock':pin(lockfile),'dependencySearchDir':str(TARGET/'deps'),'resolutionMethod':'Exact ASCII hexadecimal fingerprint bytes interpreted little-endian as Cargo dependency integer; all recursive edge candidates retained; no filename-random selection.','directRootUnits':[str(p) for p in roots],'portableRootFingerprintBindings':rootbindings,'historicalRootExecutionReceiptFiles':historical,'cachedRustcInfo':{'file':pin(TARGET.parent/'.rustc_info.json'),'record':json.loads((TARGET.parent/'.rustc_info.json').read_text()),'proofLimit':'Cargo cached output, not a compiler execution by this auditor; coordinator must re-execute selected rustc.'},'recordedCompiledClosure':[n for n in visited.values() if n['role']=='recordedCompiledClosure'],'historicalBuildProvenance':[n for n in visited.values() if n['role']=='historicalBuildProvenance'],'graphUnresolved':unresolved,'frozenInputByteComparisons':comparisons,'installedChannelManifest':pin(manifest),'installedComponentManifestPins':manifestpins,'manifestVersionsAgree':len({x['version'] for x in manifestpins.values()})==1,'staticFrozenStdSourceByteMatch':all(x['byteEqual'] for x in comparisons if x['kind']=='toolchain'),'compilerCommitFromCoordinatorPriorExecution':'88d9e12ae178fab0fb5cc050a94da85685d449ea','compilerArtifacts':[pin(p,False) for p in compiler],'conservativeStdInstalledComponentHashSet':[pin(p,False) for p in sorted(stdpaths)],'stdInventoryProofLimit':'Installed manifest artifacts are a conservative superset, not an observed consumed std set. Coordinator must bind exact sysroot/target and hash this set or trace actual compiler opens.','staticElfDependencyCandidates':list(elfrecords.values()),'possibleLinkerAndStartupArtifacts':[pin(p,False) for p in linkcandidates],'linkerSelectionVerified':False,'meshoptNativeBundling':{'rlib':pin(meshlib,False),'members':meshmembers,'historicalNativeArchives':archives,'interpretation':'All native archive members are present byte-identically inside the selected meshopt rlib when flags say true. External build-script output directory is historical provenance; isolated compiler consumes bundled native objects via the rlib. stdc++ remains an external link dependency.'},'openCoordinatorChecks':['Accepted exact C1 develop merge and accepted frozen JSON helper equality, with independent acceptance receipt.','Run exact selected compiler -vV and record full output; compare compiler commit/version/host with installed rustc/std/src manifest.','Hash all selected compiled libraries/proc-macro dependencies, compiler driver/LLVM, std component conservative set, system loader/shared libraries and actual linker/startup/library selection before and after isolated compilation.','Record dynamic-loader and linker resolution rather than treating static /usr/lib candidates as actual consumed paths; preserve PATH, LD_LIBRARY_PATH, LD_PRELOAD, RUSTFLAGS/config and selected linker environment.','Use exact isolated command/root-owned execution to establish rlib compiler compatibility; fingerprint rustc numeric IDs are not a full compiler-version receipt.','Retain registry metadata and byte comparison proof limits; original historical build source immutability cannot be newly inferred from current extracted source.']}
(OUT/'closure-candidates.json').write_text(json.dumps(result,indent=2)+'\n')
hashpaths=set()
for n in result['recordedCompiledClosure']:
 for a in n.get('artifacts',[]):hashpaths.add(a['path'])
for section in ['compilerArtifacts','conservativeStdInstalledComponentHashSet','possibleLinkerAndStartupArtifacts']:
 for a in result[section]:hashpaths.add(a['path'])
for n in result['staticElfDependencyCandidates']:
 hashpaths.add(n['artifact']['path'])
 for d in n['candidateDependencies']:
  for a in d['candidates']:hashpaths.add(a['path'])
(OUT/'root-hash-candidates.txt').write_text(''.join(p+'\n' for p in sorted(hashpaths)))
print(json.dumps({'compiledNodes':len(result['recordedCompiledClosure']),'provenanceNodes':len(result['historicalBuildProvenance']),'unresolvedGraphEdges':len(unresolved),'byteComparisons':len(comparisons),'byteComparisonMismatches':[x['frozen']['path'] for x in comparisons if not x['byteEqual']],'registryMismatches':[n['unit'] for n in visited.values() if n.get('package') and not n['package']['matchesFrozenLockRegistryChecksum']],'nativeBundlingMatches':[a['allMembersByteIdenticalInRlib'] for a in archives],'hashCandidates':len(hashpaths)},indent=2))
