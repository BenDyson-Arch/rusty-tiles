"""Read-only independent identity and retained-record checks; no target invocation."""
from pathlib import Path
import json,hashlib,re,struct
OUT=Path('/tmp/rusty-tiles-vector-f0-actual-review')
ORIGINAL=Path('/tmp/rusty-tiles-vector-f0-probe-preparation')
REVISION=Path('/tmp/rusty-tiles-vector-f0-module-path-revision')
EXEC=Path('/tmp/rusty-tiles-vector-f0-module-path-execution')
pins={}
def read(p):
 p=Path(p);b=p.read_bytes();pins[str(p)]={'sha256':hashlib.sha256(b).hexdigest(),'bytes':len(b)};return b
def sha(p):
 p=Path(p)
 if str(p) not in pins:read(p)
 return pins[str(p)]['sha256']
def js(p):return json.loads(read(p))
def main():
 r=js(EXEC/'receipt.json');assert sha(EXEC/'receipt.json')=='b52f4fa688734cd728f76250eaa532812b058e847557add3edee4c8dd3712260'
 before=js(EXEC/'before-artifact-binding.json');assert r['artifactPins']==before['artifactPins'] and r['sourceAndArtifactsUnchanged']
 original=js(ORIGINAL/'integrity.json');revision=js(REVISION/'integrity.json');mapping=js(REVISION/'revision.json')
 assert len(original['files'])==37 and len(revision['files'])==9
 for root,d in [(ORIGINAL,original),(REVISION,revision)]:
  for name,p in d['files'].items():assert sha(root/name)==p['sha256'] and pins[str(root/name)]['bytes']==p['bytes']
 assert sha(ORIGINAL/'integrity.json')==r['packageIntegritySha256']==before['originalPackageIntegritySha256']
 assert sha(REVISION/'integrity.json')==r['revisionIntegritySha256']==before['revisionIntegritySha256']
 for new,old in mapping['copies'].items():assert read(REVISION/new)==read(ORIGINAL/old)
 assert read(REVISION/'isolated/f0_probe.rs')==read(ORIGINAL/'isolated/f0_probe.rs').replace(b'src/runtime.rs',b'src/runtime/mod.rs')
 for name in ['runtime.rs','runtime/directory.rs','runtime/directory_platform.rs']:
  assert read(ORIGINAL/'isolated/src'/name)==read(ORIGINAL/'inputs/accepted/src'/name)==read(Path('/tmp/rusty-tiles-vector-codec-foundation/src')/name)
 source_diff=js(ORIGINAL/'source-differences.json');helper=read(ORIGINAL/'inputs/accepted/src/runtime/tests.rs')[source_diff['candidateHelperSourceByteStart']:source_diff['candidateHelperSourceByteEnd']]
 assert hashlib.sha256(helper).hexdigest()==source_diff['candidateHelperSha256'] and helper in read(ORIGINAL/'isolated/src/runtime/tests.rs')
 for p,h in r['artifactPins'].items():assert sha(p)==h,p
 for p,h in r['outputHashes'].items():assert sha(EXEC/p)==h,p
 assert sha('/tmp/rusty-tiles-run-f0-probes-module-path.py')==r['coordinatorSourceSha256']==before['coordinatorSourceSha256']
 assert js('/tmp/rusty-tiles-codec-p2-executions/exact-artifact-receipt.json') and sha('/tmp/rusty-tiles-codec-p2-executions/exact-artifact-receipt.json')==r['baseToolchainReceiptSha256']
 commands=js(EXEC/'commands.json');assert commands==r['commands'] and all(c['exitCode']==0 for c in commands)
 assert r['compilerVV']==read(EXEC/'compiler-vV.stdout').decode() and 'host: x86_64-unknown-linux-gnu' in r['compilerVV']
 stdout=read(EXEC/'focused-tests.stdout').decode();names=re.findall(r'^test (\S+) \.\.\. ok$',stdout,re.M)
 assert names==r['selectedTests']==js(REVISION/'compile-recipe.json')['linuxExpectedTests'] and len(names)==2
 assert '2 passed; 0 failed; 0 ignored; 0 measured; 32 filtered out' in stdout and read(EXEC/'focused-tests.stderr')==b''
 binary=read(EXEC/'vector_f0_probe');assert binary[:4]==b'\x7fELF' and struct.unpack_from('<H',binary,18)[0]==62 and sha(EXEC/'vector_f0_probe')==r['executableSha256']=='465d71090d4c3b07e5845f167d881306f28e998b695c79ee5c57be75b89d1c01'
 closure=js(ORIGINAL/'closure-recommendation.json');assert len(closure['compiled'])==10 and not closure['unresolvedEdges']
 for unit in closure['compiled']:
  for a in unit['artifacts']:assert read(EXEC/'deps'/Path(a['path']).name)==read(a['path'])
 resolved={str(Path(p).resolve()):h for p,h in r['artifactPins'].items()};missing=[];generated=[];consumed=[]
 for line in read(EXEC/'link-trace/link-1533011.stdout').decode().splitlines():
  if not line.startswith('/'):continue
  name=line.split('(',1)[0];p=Path(name);identity=str(p.resolve())
  if identity in resolved:consumed.append({'trace_path':name,'resolved_path':identity,'sha256':resolved[identity]})
  elif p.is_file():missing.append(name)
  elif name.startswith(str(EXEC)):generated.append(name)
  else:missing.append(name)
 assert not missing and len(set(generated))==8
 link=js(EXEC/'link-trace/link-1533011-command.json');assert link['actualCommand'][0]=='/usr/bin/cc' and '-Wl,--trace' in link['actualCommand']
 assert 'libsame_file' not in '\n'.join(link['argv']) and 'libmeshopt' not in '\n'.join(link['argv'])
 libmvec=r['artifactPins']['/usr/lib/libmvec.so.1'];assert sha('/usr/lib/libmvec.so.1')==libmvec
 failure=read('/tmp/rusty-tiles-vector-f0-execution/compile.stderr').decode();assert failure.count('error[E0583]')==2 and 'mod tests' in failure and 'mod directory' in failure
 for p in Path('/tmp/rusty-tiles-vector-f0-execution').rglob('*'):
  if p.is_file():read(p)
 for p in ['/tmp/rusty-tiles-run-f0-probes.py','/tmp/rusty-tiles-archive-read-final-evidence/run-controlled.py',ORIGINAL/'inputs/dependencies/tempfile-3.27.0/src/file/imp/unix.rs',ORIGINAL/'inputs/dependencies/tempfile-3.27.0/src/file/imp/windows.rs']:read(p)
 result={'status':'READ_ONLY_SOURCE_ARTIFACT_AND_RETAINED_EXECUTION_RECORD_VERIFICATION_PASS','original_declared_files':37,'revision_declared_files':9,'module_copies_byte_identical':True,'entrypoint_path_change_only':True,'production_runtime_directory_bytes_unchanged':True,'accepted_candidate_helper_exact':True,'root_before_after_pins_equal':True,'artifact_pins_current_hashes_verified':len(r['artifactPins']),'executable_sha256':r['executableSha256'],'selected_linux_tests':names,'actual_persistent_link_inputs_missing_pins':missing,'consumed_traced_inputs':consumed,'unretained_generated_compiler_objects':sorted(set(generated)),'libmvec_explicitly_bound':libmvec,'same_file_not_in_link_argv':True,'original_compile_failure_retained':True,'reviewer_correction':'Initial libmvec omission suspicion was caused by a truncated displayed list; complete dictionary enumeration shows the file bound before/after. No extra execution required.','scope':'Existing primitive Linux Replace non-CAS seam and explicitly copied Unix0640 mode only; Windows/capture/resource/producer/new codec held.'}
 (OUT/'verification.json').write_text(json.dumps(result,indent=2)+'\n')
 (OUT/'input-pin.json').write_text(json.dumps({'files':pins,'count':len(pins)},indent=2)+'\n')
 print(json.dumps({'status':result['status'],'artifact_pins':len(r['artifactPins']),'pinned_files':len(pins),'missing_link_input_pins':0,'linux_tests':2}))
if __name__=='__main__':main()
