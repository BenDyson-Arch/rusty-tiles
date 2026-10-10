#!/usr/bin/env python3
"""Additive argv-only correction for genuine 65537 f32 generated feature IDs.

Original source/expectation bytes and failed original run stay unchanged.
Only the existing producer maxBytes option increases; no codec/default/source
rewriting or producer launch occurs in this input-receipt preparation script.
"""
import argparse,hashlib,json,shlex
from pathlib import Path
BUILD=Path('/tmp/rusty-tiles-payload-final-evidence/stride-final-build')
SOURCE=Path('/tmp/rusty-tiles-vector-codec-foundation')
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(65536),b''):h.update(b)
 return h.hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,required=True);p.add_argument('--expected',type=Path,required=True)
 p.add_argument('--work',type=Path,required=True);a=p.parse_args();w=a.work.resolve()
 assert w.parent==Path('/tmp') and w.name.startswith('rusty-tiles-vector-p3-large-ids-revision-');w.mkdir(mode=0o700)
 data=json.loads(a.source.read_bytes());rows=data['features'];assert len(rows)==65537
 for i,f in enumerate(rows):
  assert f['id']==i and f['properties']=={} and f['geometry']=={'type':'Point','coordinates':[0,0,0]}
 expected=json.loads(a.expected.read_bytes());assert expected['expected_point_id_row_count']==65537
 assert expected['feature_id_domain']=='FLOAT_SCALAR_stride4' and len(expected['source_rows'])==65537
 # Source-layer spelling is bounded for this literal filename. The source
 # profile has only generated _source_layer/_source_id metadata, so a 256-byte
 # property encoding bound leaves ample room above the exact compact length.
 layer=a.source.stem;assert len(layer.encode('utf8'))<=128
 max_props=max(len(json.dumps({'_source_id':str(i),'_source_layer':layer},separators=(',',':'),ensure_ascii=False).encode()) for i in range(65537))
 assert max_props<=256
 pin=json.loads((BUILD/'source-pin.json').read_bytes())
 for n in ['src/vector/model.rs','src/vector/pipeline/store.rs','src/vector/pipeline/encoding.rs','src/vector/pipeline/source_native.rs','src/vector/portable.rs']:
  assert sha(SOURCE/n)==pin['production_sha256'][n]
 paths={mode:next(BUILD.glob(mode+'-cli-e4ec3cc-*')) for mode in ['portable','native']}
 expected_binary={'portable':'fbe4c46b578a54f5de0958b56aae8531ddd34b4c371040c18663df33199c03f5',
                  'native':'bcdd0f3bcc0a504a20bf5f3a7653416bac75f9270233b1531a916b2423466dec'}
 for mode,path in paths.items():assert sha(path)==expected_binary[mode]
 runs=[]
 for mode,binary in paths.items():
  for variant,flags in [('raw',[]),('meshopt',['--meshopt'])]:
   out=w/('large-ids-65537-'+mode+'-'+variant+'.3tz')
   argv=['env','RAYON_NUM_THREADS=2','PROJ_NETWORK=OFF','nice','-n','10',str(binary),'--json','vector',
    '-i',str(a.source.resolve()),'-o',str(out),'--explicit','--reproducible','--sourceCrs','local','--jobs','2',
    '--lodLevels','1','--listFields','json','--maxFeatures','70000','--maxVertices','70000','--maxBytes','134217728',*flags]
   runs.append({'mode':mode,'variant':variant,'output':str(out),'argv':argv,'binary_sha256':expected_binary[mode]})
 result={'status':'ARGV_ONLY_PREPARATION_NO_PRODUCER_EXECUTION','source':str(a.source.resolve()),'source_sha256':sha(a.source),
  'expected':str(a.expected.resolve()),'expected_sha256':sha(a.expected),'driver_sha256':sha(Path(__file__)),
  'changed_option':{'name':'producer --maxBytes','before':33554432,'after':134217728},
  'bound':{'features':65537,'rendered_points_per_feature':1,'conservative_intrinsic_points_per_feature':1,
   'property_bytes_upper_per_feature':256,'observed_literal_compact_properties_max_bytes':max_props,
   'estimate_formula':'32*(rendered+intrinsic)+property_json_bytes+2048',
   'aggregate_estimate_lower':65537*2048,'aggregate_estimate_upper':65537*(64+256+2048),
   'original_partition_guard':2*33554432,'revised_partition_guard':2*134217728,
   'reason':'2048*65537 alone exceeds original guard and even2*64MiB by2048;128MiB chosen existing option clears finite literal source upper bound',
   'scope':'producer partition estimate only, not codec source/working/default limits, actual output bytes or universal RAM/time acceptance'},
  'source_hashes':{n:pin['production_sha256'][n] for n in ['src/vector/model.rs','src/vector/pipeline/store.rs','src/vector/pipeline/encoding.rs']},
  'retained_original_failure':'root observed original targets pass but4 U16 leafpayloads16385/16384/16384/16384; original expectedf32 extraction failed. No source/old receipt repair.',
  'runs':runs,'required_execution_gate':'Root must observe a real65537-row one-leaf POINTS primitive with actual f32 IDs0..65536, metadata exact identities, then exact raw/meshopt logicalview/address preservation; options alone do not prove this'}
 (w/'recipe-revision.json').write_text(json.dumps(result,indent=2)+'\n')
 (w/'root-producer-commands.sh').write_text('# Root scheduled taskset2 serial execution only\n'+'\n'.join(shlex.join(r['argv']) for r in runs)+'\n')
 print(json.dumps({'receipt':str(w/'recipe-revision.json'),'commands':len(runs),'producer_executions':0}))
if __name__=='__main__':main()
