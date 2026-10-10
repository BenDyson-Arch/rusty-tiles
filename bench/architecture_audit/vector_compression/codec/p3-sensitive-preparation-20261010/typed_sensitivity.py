#!/usr/bin/env python3
"""Root-run sensitivity checks against actual typed raw producer GLB bytes.

No producer/consumer/process launches. Reuses the frozen independent typed
oracle, not product validation. This does not establish codec rejection of
these source semantic mutations; preservation compares must catch changed bits.
"""
import argparse,copy,hashlib,importlib.util,json,struct
from pathlib import Path
FROZEN=Path('/tmp/rusty-tiles-vector-p3-generated-preparation/extract.py')
PIN='5c6f86a1a9a47ecd9dd9189bf8de1fb22a34ffd4c70eaf14bcf3e278fad4653f'
def sha(b):return hashlib.sha256(b).hexdigest()
def envelope(doc,binary):
 # Keep the original current core4 mechanism deliberately. These are typed
 # oracle controls, not selected metadata8 profile admission controls.
 j=json.dumps(doc,separators=(',',':'),ensure_ascii=False).encode();j+=b' '*(-len(j)%4)
 binary+=b'\0'*(-len(binary)%4)
 return struct.pack('<4sII',b'glTF',2,28+len(j)+len(binary))+struct.pack('<I4s',len(j),b'JSON')+j+struct.pack('<I4s',len(binary),b'BIN\0')+binary
def main():
 p=argparse.ArgumentParser();p.add_argument('--input',type=Path,required=True);p.add_argument('--expected',type=Path,required=True)
 p.add_argument('--work',type=Path,required=True);a=p.parse_args();w=a.work.resolve()
 assert w.parent==Path('/tmp') and w.name.startswith('rusty-tiles-vector-p3-typed-controls-');w.mkdir(mode=0o700)
 assert sha(FROZEN.read_bytes())==PIN
 spec=importlib.util.spec_from_file_location('frozen_independent_typed_oracle',FROZEN);oracle=importlib.util.module_from_spec(spec);spec.loader.exec_module(oracle)
 raw=a.input.read_bytes();before=sha(raw);jl=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+jl]);binary=raw[28+jl:]
 assert raw[:4]==b'glTF' and all('EXT_meshopt_compression' not in v.get('extensions',{}) for v in doc['bufferViews'])
 expected=json.loads(a.expected.read_bytes());props=doc['extensions']['EXT_structural_metadata']['propertyTables'][0]['properties']
 def view_start(d,i):return d['bufferViews'][i].get('byteOffset',0)
 def prop_start(d,key,role='values'):return view_start(d,props[key][role])
 controls=[]
 controls.append(('int64_low_bit',lambda d,b:b.__setitem__(prop_start(d,'integer'),b[prop_start(d,'integer')]^1)))
 count=doc['extensions']['EXT_structural_metadata']['propertyTables'][0]['count'];assert count==9
 controls.append(('boolean_unused_tail_bit',lambda d,b:b.__setitem__(prop_start(d,'flag')+1,b[prop_start(d,'flag')+1]|0x80)))
 controls.append(('utf8_string_offset_shift',lambda d,b:struct.pack_into('<I',b,prop_start(d,'text','stringOffsets')+4,3)))
 def list_integer(d,b):
  start=prop_start(d,'list');length=d['bufferViews'][props['list']['values']]['byteLength'];data=bytes(b[start:start+length])
  needle=b'9007199254740993';at=data.index(needle);b[start+at+len(needle)-1]=ord('2')
 controls.append(('list_large_integer_rounding',list_integer))
 def source_id(d,b):
  start=prop_start(d,'_source_id');length=d['bufferViews'][props['_source_id']['values']]['byteLength'];data=bytes(b[start:start+length])
  at=data.index(b'p0');b[start+at]=ord('q')
 controls.append(('source_identity_changed',source_id))
 def padding(d,b):
  primitive=d['meshes'][0]['primitives'][0];ac=d['accessors'][primitive['attributes']['_FEATURE_ID_0']]
  start=view_start(d,ac['bufferView'])+ac.get('byteOffset',0);b[start+2]=1
 controls.append(('feature_id_padding_bit',padding))
 controls.append(('wrong_property_table',lambda d,b:d['meshes'][0]['primitives'][0]['extensions']['EXT_mesh_features']['featureIds'][0].update(propertyTable=1)))
 records=[]
 oracle.inspect(raw,w/'baseline',None,expected)
 for name,change in controls:
  d=copy.deepcopy(doc);b=bytearray(binary);change(d,b);changed=envelope(d,bytes(b));(w/(name+'.glb')).write_bytes(changed)
  try:oracle.inspect(changed,w/(name+'-extracted'),None,expected)
  except (AssertionError,ValueError,UnicodeDecodeError,KeyError,struct.error) as e:
   records.append({'control':name,'input_sha256':sha(changed),'outcome':'independent_typed_oracle_rejected',
    'cause_type':type(e).__name__,'cause':str(e)[:512]})
  else:raise AssertionError('insensitive typed control '+name)
 assert sha(a.input.read_bytes())==before
 result={'status':'ROOT_EXECUTED_ORACLE_CONTROLS_NOT_PRODUCER_OR_CODEC_ACCEPTANCE','source_input_sha256':before,
  'frozen_typed_oracle_sha256':PIN,'probe_sha256':sha(Path(__file__).read_bytes()),'records':records,
  'scope':'actual input-derived sensitive controls plus authored typed expectations; no new converter/decoder/codec execution, no quantization accuracy claim'}
 (w/'receipt.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
if __name__=='__main__':main()
