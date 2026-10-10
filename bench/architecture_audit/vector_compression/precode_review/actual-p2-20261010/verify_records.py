"""Independent lightweight retained-record/hash arithmetic review; never executes probes."""
import ast, hashlib, json, pathlib, struct, tomllib
OUT=pathlib.Path(__file__).parent
BASE=pathlib.Path('/tmp/rusty-tiles-codec-p2-executions')
PKG=pathlib.Path('/tmp/rusty-tiles-next-vector-compression-audit/codec/p2-proof-preparation-20261010')
REV=PKG.parent/'p2-domain-oracle-revision-20261010'
AUD=pathlib.Path('/tmp/rusty-tiles-codec-toolchain-audit/closure-candidates.json')
pins={}
def sha(p): return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
def pin(p):
 p=pathlib.Path(p); r={'sha256':sha(p),'bytes':p.stat().st_size}; pins[str(p)]=r; return r['sha256']
def read(p): pin(p); return json.loads(pathlib.Path(p).read_text())
def package(p):
 d=read(p/'integrity.json')
 for n,r in d['files'].items():
  f=p/n; assert f.stat().st_size==r['bytes'] and pin(f)==r['sha256'],n
 return len(d['files'])
counts={'originalPackageFiles':package(PKG),'additiveDomainFiles':package(REV)}
gate=read(BASE/'accepted-c1-gate.json'); assert gate['acceptedMergeCommit']=='f5401dd120441b4f9aac5ad229dbfffe03fe4428'
for k in ['acceptanceReceipt','independentRemoteReviewReceipt']: assert pin(gate[k]['path'])==gate[k]['sha256']
assert pin(PKG/'inputs/source/src/content_integrity/json.rs')==gate['acceptedJsonOwnerSha256']
art=read(BASE/'exact-artifact-receipt.json'); audit=read(AUD)
assert len(art['artifacts'])==205 and art['stdSourceMatchedToCompiler'] and art['dependencyClosureComplete']
assert art['cargoLockSha256']==pin(PKG/'inputs/source/Cargo.lock')
assert art['acceptedMergeCommit']==gate['acceptedMergeCommit'] and audit['graphUnresolved']==[]
for r in audit['frozenInputByteComparisons']:
 assert r['byteEqual'] and pin(r['frozen']['path'])==pin(r['actual']['path'])==r['frozen']['sha256']==r['actual']['sha256']
counts['frozenSourceComparisons']=len(audit['frozenInputByteComparisons'])
manifest=audit['installedChannelManifest']; assert pin(manifest['path'])==manifest['sha256']
channel=tomllib.loads(pathlib.Path(manifest['path']).read_text())
assert all(channel['pkg'][n]['version']=='1.98.0 (88d9e12ae 2026-08-18)' for n in ['rustc','rust-std','rust-src'])
assert 'commit-hash: 88d9e12ae178fab0fb5cc050a94da85685d449ea' in art['rustcVV']
by_unit={r['fingerprintInteger']:r for r in audit['recordedCompiledClosure']}
assert len(by_unit)==18
closure=[]
for u in by_unit.values():
 assert pin(u['fingerprintFile']['path'])==u['fingerprintFile']['sha256']
 assert int.from_bytes(bytes.fromhex(pathlib.Path(u['fingerprintFile']['path']).read_text()),'little')==u['fingerprintInteger']
 assert pin(u['fingerprintJson']['path'])==u['fingerprintJson']['sha256']
 f=read(u['fingerprintJson']['path'])
 assert json.loads(f['features'])==u['features']
 for e in u['edges']:
  assert e['resolutionUnique'] and len(e['candidateUnits'])==1
  fp=pathlib.Path(e['candidateUnits'][0]); pin(fp)
  assert int.from_bytes(bytes.fromhex(fp.with_suffix('').read_text()),'little')==e['dependencyFingerprintInteger']
 a=u['artifacts'][0]; dest=pathlib.Path(art['dependencySearchDir'])/pathlib.Path(a['path']).name
 assert pin(dest)==art['artifacts'][str(dest)]['sha256']==art['artifacts'][a['path']]['sha256']
 closure.append({'name':u['package']['name'],'features':u['features'],'copiedArtifactSha256':sha(dest)})
counts['compiledClosureArtifactsIndependentlyRehashed']=len(closure)
# Independently inspect archive bytes; no ar/native command.
mesh=pathlib.Path(art['crates']['meshopt']['path']).read_bytes(); assert mesh[:8]==b'!<arch>\n'; pos=8; members={}; longnames=b''
while pos<len(mesh):
 h=mesh[pos:pos+60]; assert h[58:60]==b'`\n'; n=h[:16].decode().strip(); size=int(h[48:58]); b=mesh[pos+60:pos+60+size];pos+=60+size+(size%2)
 if n=='//':longnames=b;continue
 if n.startswith('/') and n[1:].isdigit():n=longnames[int(n[1:]):].split(b'/\n')[0].decode()
 else:n=n.rstrip('/')
 members[n]={'bytes':len(b),'sha256':hashlib.sha256(b).hexdigest()}
native=[m for m in audit['meshoptNativeBundling']['members'] if m['name'].startswith('55d')]
assert len(native)==16
for m in native:assert members[m['name']]=={k:m[k] for k in ['bytes','sha256']}
counts['nativeObjectsIndependentlyMatchedInConsumedRlib']=16
outputs=0
for lane in ['raw','ffi','models','domain-revision-20261010']:
 receipt=read(BASE/lane/'receipt.json')
 if lane=='models':assert [c['exitCode'] for c in receipt['commands']]==[0,0,1]
 if lane=='domain-revision-20261010':assert [c['exitCode'] for c in receipt['commands']]==[0]
 for command in receipt['commands']:
  for kind in ['stdout','stderr']:assert pin(BASE/lane/(command['label']+'.'+kind))==command[kind+'Sha256']
  outputs+=2
 if lane in ['raw','ffi']:
  assert receipt['artifactReceipt']==art and receipt['artifactReceiptSha256']==sha(BASE/'exact-artifact-receipt.json')
  assert receipt['packageUnchanged'] and receipt['artifactsUnchanged'] and all(c['exitCode']==0 for c in receipt['commands'])
  assert pin(BASE/lane/(lane+'_probe'))==receipt['probeBinarySha256']
  assert receipt['acceptedC1GateSha256']==sha(BASE/'accepted-c1-gate.json')
counts['commandOutputFilesIndependentlyRehashed']=outputs
admissions=read(PKG/'fixtures/admission/manifest.json')['cases']
for c in admissions:
 rows=[json.loads(x) for x in (BASE/'raw'/('admission-'+c['name']+'.stdout')).read_text().splitlines()]
 assert rows[-1]['observed']==c['expected']['category'] and rows[-1]['match']
counts['actualAdmissionCases']=len(admissions)
serial=read(PKG/'fixtures/serializer/manifest.json')['cases'];cal=read(BASE/'raw-calibration/calibration.stdout');findings=[]
for c,calrow in zip(serial,cal['cases'],strict=True):
 rows=[json.loads(x) for x in (BASE/'raw'/('serializer-'+c['name']+'.stdout')).read_text().splitlines()]; stages={r['stage']:r for r in rows if 'stage'in r}; facts=next(r for r in rows if 'exactJsonBytes'in r)
 assert facts['exactJsonBytes']==c['expectedJsonBytes'] and facts['exactOracleMatch'] and facts['deltaUpper']==c['deltaBound'] and facts['total']==c['expectedTotal']
 assert facts['candidateCapacity']==facts['total']
 j=c['expectedJsonBytes']; u=(PKG/'fixtures/serializer'/c['path']/'binary.bin').stat().st_size;a=8 if c['metadata']else 4
 jc=((20+j+a-1)//a)*a-20 if c['metadata']else ((j+3)//4)*4
 assert facts['total']==28+jc+((u+a-1)//a)*a and facts['binOrigin']==28+jc
 if c['metadata']:assert (20+jc)%8==0 and facts['binOrigin']%8==0
 parser=stages['actual_pinned_json_admission'];plan=stages['scaffold_owned_key_pairs_and_fixed_edit'];pd=parser['peakRequestedOldPlusNew']-parser['baselineRequested'];qd=plan['peakRequestedOldPlusNew']-plan['baselineRequested']
 ej=8*c['inputBytes']+256*c['nodes']+65536;ep=2*c['inputBytes']+128*c['nodes']+256*c['views']
 assert calrow['name']==c['name'] and (pd,qd,ej,ep)==(calrow['scaffoldParserIncrementalPeak'],calrow['scaffoldPairStageIncrementalPeak'],calrow['draftEjson'],calrow['draftEplanA0'])
 if pd>ej or qd>ep:findings.append(calrow)
assert findings==cal['findings'] and len(findings)==1 and findings[0]['name']=='long-root-key-late-escape'
counts['actualExactSerializerCases']=len(serial);counts['metadataResiduePairs']=len({(c['jsonResidue'],c['binResidue']) for c in serial if c['metadata']});assert counts['metadataResiduePairs']==64
cr=read(BASE/'raw-calibration/receipt.json'); assert cr['exitCode']==2
for kind in ['stdout','stderr']:assert pin(BASE/'raw-calibration'/('calibration.'+kind))==cr[kind+'Sha256']
streams=read(PKG/'fixtures/streams/manifest.json')['cases'];ffirows=[json.loads(x)for x in (BASE/'ffi/ffi-cases.stdout').read_text().splitlines()]
for c,r in zip(streams,ffirows[:len(streams)],strict=True):
 s=c['stride'];n=c['count'];b=min(256,16*(8192//(16*s))); blocks=(n+b-1)//b;head=(b+63)//64;bound=1+blocks*s*(s//4+head+b)+max(s+s//4,32)
 assert r['name']==c['name'] and bound==c['expectedBound']==r['bound'] and 0<r['encodedLength']<=bound and r['sourceBytes']==n*s
 assert all(r[k] for k in ['guardsIntact','sameKernelWholeByteMatch','tinyDestinationRejected'])
assert ffirows[-1]['literalGoldenMatch'] and ffirows[-1]['provenMalformedHeaderAndHeaderOnlyRejected'] and ffirows[-1]['decodableByteMutationDetectedByOracle']
counts['actualFfiCases']=len(streams);counts['allPositiveEligibleStrides']=len({c['stride'] for c in streams})
def function(p,name):
 text=pathlib.Path(p).read_text();node=next(n for n in ast.parse(text).body if isinstance(n,ast.FunctionDef)and n.name==name);return ast.get_source_segment(text,node)
assert function(PKG/'domain_oracle.py','check')==function(REV/'domain_oracle.py','check')
domain=read(BASE/'domain-revision-20261010/domain_oracle.stdout');expect=read(REV/'expected-selftest-records.json')
assert domain==expect['expectedOutput']
counts['domainBaselinesAndRestored']=len(domain['cases']);counts['domainFaultyRejections']=sum(len(c['sensitiveControls']) for c in domain['cases'])
assert counts['domainBaselinesAndRestored']==15 and counts['domainFaultyRejections']==33
for lane in ['artifact-binding','artifact-binding-absolute-argv0','artifact-binding-traced-link']:
 r=read(BASE/lane/'binding-receipt.json')
 for name in ['compile.stdout','compile.stderr','rustc-vV.stdout']:pin(BASE/lane/name)
 if lane=='artifact-binding':assert r['exitCode']!=0
 else:
  assert r['exitCode']==r['probeExitCode']==0 and r['afterPinsEqual']
  assert pin(BASE/lane/'binding_probe')==r['probeBinarySha256'] and pin(BASE/lane/'run.stdout')==r['probeStdoutSha256'];pin(BASE/lane/'run.stderr')
for lane in ['raw-link-trace','ffi-link-trace','artifact-binding-traced-link']:
 for f in (BASE/lane).glob('link-*'):pin(f)
 for f in (BASE/lane).glob('link-*-command.json'):assert read(f)['actualCommand'][:3]==['/usr/bin/cc','-v','-Wl,--trace']
for name in ['binding_probe.rs','bind-artifacts.py','bind-artifacts-absolute-argv0.py','bind-artifacts-traced-link.py','finish-artifact-receipt.py','raw-launch.json','ffi-launch.json']:pin(BASE/name)
for f in (BASE/'artifact-binding-traced-link').glob('link-*.stdout'):
 assert '/usr/lib/gcc/x86_64-pc-linux-gnu/16/libstdc++.so' in f.read_text()
for f in (BASE/'ffi-link-trace').glob('link-*.stdout'):
 assert '55d8142df00c0e5b-vertexcodec.o' in f.read_text() and '/usr/lib/gcc/x86_64-pc-linux-gnu/16/libstdc++.so' in f.read_text()
cpp=pathlib.Path('/usr/lib/gcc/x86_64-pc-linux-gnu/16/libstdc++.so').resolve();assert str(cpp)=='/usr/lib/libstdc++.so.6.0.36';assert pin(cpp)==art['artifacts'][str(cpp)]['sha256']
for p,r in art['artifacts'].items():assert pathlib.Path(p).is_file()
result={'status':'READ_ONLY_RETAINED_RECORD_VERIFICATION_PASSED','counts':counts,'calibrationFindings':findings,'compiledClosure':closure,'scope':'Independent current small-source/package/output/18copiedcompiledartifact/probebinary/native-member hashes and integer comparisons. Did not execute any author script, model, Rust/native target, Cargo or Git. Did not independently rehash all205 large compiler/sysroot/environment artifacts; their before/after hash equality is retained coordinator evidence corroborated by inspected runner source and exact receipt identity. No historical build-source immutability/full OS trace/production/default/whole-operation/RSS claim.'}
(OUT/'verification.json').write_text(json.dumps(result,indent=2)+'\n')
(OUT/'input-pin.json').write_text(json.dumps({'status':'review_epoch_pins','files':pins},indent=2)+'\n')
print(json.dumps({'counts':counts,'inputPins':len(pins),'finding':findings[0]},indent=2))
