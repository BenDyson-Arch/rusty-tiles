#!/usr/bin/env python3
"""Static independently authored GeoJSON corpus and root-controlled commands.

This script creates inputs/expectations only. It never launches a producer.
The 65537-ID case is separately requested and requires coordinator scheduling.
"""
import argparse, hashlib, json, shlex, struct
from pathlib import Path

CHECKOUT=Path('/tmp/rusty-tiles-vector-codec-foundation')
BUILD=Path('/tmp/rusty-tiles-payload-final-evidence/stride-final-build')
BINARIES={
 'portable':BUILD/'portable-cli-e4ec3cc-fbe4c46b578a54f5de0958b56aae8531ddd34b4c371040c18663df33199c03f5',
 'native':BUILD/'native-cli-e4ec3cc-bcdd0f3bcc0a504a20bf5f3a7653416bac75f9270233b1531a916b2423466dec'}
BIN_SHA={'portable':'fbe4c46b578a54f5de0958b56aae8531ddd34b4c371040c18663df33199c03f5',
 'native':'bcdd0f3bcc0a504a20bf5f3a7653416bac75f9270233b1531a916b2423466dec'}
HERE=Path(__file__).resolve().parent

def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(65536),b''):h.update(b)
 return h.hexdigest()
def feature(i,kind,coords,props=None):
 return {'type':'Feature','id':i,'properties':props or {},'geometry':{'type':kind,'coordinates':coords}}
def ring(x,y,w):return [[x,y,0],[x+w,y,0],[x+w,y+w,0],[x,y+w,0],[x,y,0]]

def corpus():
 # Explicit integer, UTF8 and list states are author truth, independent of
 # generator source/float conversions/producer document associations.
 ints=[-(1<<63), (1<<60)+3, 9007199254740993, -17, 0, 1, 65536, (1<<63)-1, 42]
 texts=['é','雪🙂','', 'a\\b', 'quote"', '__RUSTY_TILES_MISSING__', 'null', 'nul\u0000byte', 'end']
 rows=[]
 for i in range(9):
  rows.append(feature('p'+str(i),'Point',[i*2,i%3,0],
    {'ordinal':i,'flag':i%2==0,'integer':ints[i], 'real':None if i==4 else i+0.25,
     'text':None if i==3 else texts[i], 'nullable_integer':None if i==2 else ints[i],
     'list':[i,9007199254740993,18446744073709551615,None,'é',False,[3,2,1]],
     'empty_list':[]}))
 # Separate topology-only source avoids metadata schema promotion from making
 # expected integer truth contingent on an unrelated mixed schema.
 geometry=[feature('point','MultiPoint',[[0,0,0],[2,1,0]]),
  feature('line','LineString',[[0,3,0],[2,5,0],[5,3,0]]),
  feature('restart','MultiLineString',[[[0,7,0],[2,8,0]],[[4,8,0],[6,7,0],[8,9,0]]]),
  feature('hole','Polygon',[ring(10,0,8),list(reversed(ring(12,2,2)))]),
  feature('multiple','MultiPolygon',[[ring(22,0,3)],[ring(28,0,3)]])]
 # One polygon with a hole: ten authored coordinates exceed 8 vertices.
 # Unlike merely splitting MultiPolygon parts, the actual Polygon path must
 # partition triangulated surfaces into standard fill b3dm fragments
 # and separate source boundaries. This is an expected source path, not a
 # pre-execution assertion that the current producer is valid.
 fragmented=[feature('fill-source','Polygon',[ring(0,0,12),list(reversed(ring(4,4,4)))],
                     {'large':(1<<60)+3,'text':'fill é'})]
 return [('typed-metadata',rows,[]),('geometry',geometry,[]),
         ('fragmented-fill',fragmented,['--maxVertices','8'])]

def external_fixture(w):
 # Independent raw-preservation fixture, never a producer capability claim.
 arrays=[struct.pack('<3Q',0,(1<<63)+1,(1<<64)-1),
         struct.pack('<3q',2,-3,(1<<60)+3),struct.pack('<4I',0,0,2,3)]
 binary=b'';views=[]
 for value in arrays:
  binary+=b'\0'*(-len(binary)%8);views.append({'buffer':0,'byteOffset':len(binary),'byteLength':len(value)});binary+=value
 doc={'asset':{'version':'2.0'},'buffers':[{'byteLength':len(binary)}],'bufferViews':views,
  'extensionsUsed':['EXT_structural_metadata'],'extras':{'exactInteger':9007199254740993,'opaque':[None,False,'雪']},
  'extensions':{'EXT_structural_metadata':{'schema':{'id':'independent-external','classes':{'sample':{'properties':{
   'unsigned':{'type':'SCALAR','componentType':'UINT64'},
   'readings':{'type':'SCALAR','componentType':'INT64','array':True}}}}},
   'propertyTables':[{'class':'sample','count':3,'properties':{'unsigned':{'values':0},
    'readings':{'values':1,'arrayOffsets':2,'arrayOffsetType':'UINT32'}}}]}}}
 j=json.dumps(doc,separators=(',',':'),ensure_ascii=False).encode();j+=b' '*(-(20+len(j))%8);binary+=b'\0'*(-len(binary)%8)
 raw=struct.pack('<4sII',b'glTF',2,28+len(j)+len(binary))+struct.pack('<I4s',len(j),b'JSON')+j+struct.pack('<I4s',len(binary),b'BIN\0')+binary
 path=w/'independent-external-uint64-variable-arrays.glb';path.write_bytes(raw)
 return {'path':str(path),'sha256':sha(path),'scope':'independently authored external preservation only; not generated vector',
  'view_sha256':[hashlib.sha256(v).hexdigest() for v in arrays],
  'typed_values':{'unsigned':[0,(1<<63)+1,(1<<64)-1],'readings':[[],[2,-3],[(1<<60)+3]]}}

def main():
 p=argparse.ArgumentParser();p.add_argument('--work',required=True,type=Path)
 p.add_argument('--include-large-ids',action='store_true');a=p.parse_args()
 w=a.work.resolve();assert w.parent==Path('/tmp') and w.name.startswith('rusty-tiles-vector-p3-inputs-')
 w.mkdir(mode=0o700)
 source_pin=json.loads((BUILD/'source-pin.json').read_bytes())
 actual={n:sha(CHECKOUT/n) for n in source_pin['production_sha256']}
 assert actual==source_pin['production_sha256'], 'Accepted 97 production source bytes changed'
 assert len(actual)==97
 for mode,b in BINARIES.items():assert sha(b)==BIN_SHA[mode]
 cases=corpus()
 if a.include_large_ids:
  cases.append(('large-ids-65537',[feature(i,'Point',[0,0,0]) for i in range(65537)],
                ['--maxFeatures','70000','--maxVertices','70000','--maxBytes','33554432']))
 manifest={'status':'PREPARATION_ONLY_NO_PRODUCER_EXECUTION',
  'prepare_sha256':sha(Path(__file__)), 'accepted_source_pin_sha256':sha(BUILD/'source-pin.json'),
  'accepted_production_sha256':actual,'binary_sha256':BIN_SHA,'cases':[],
  'independent_external_preservation':[external_fixture(w)]}
 commands=[]
 for label,rows,extra in cases:
  path=w/(label+'.geojson');path.write_text(json.dumps({'type':'FeatureCollection','features':rows},
    separators=(',',':'),ensure_ascii=False,allow_nan=False)+'\n')
  expectation={'source_rows':{json.dumps(r['id'],separators=(',',':'),ensure_ascii=False):r['properties'] for r in rows},
   'source_geometry':{json.dumps(r['id'],separators=(',',':'),ensure_ascii=False):r['geometry'] for r in rows},
   'list_properties_are_JSON_strings':['list','empty_list'] if label=='typed-metadata' else [],
   'expected_position_domains':['FLOAT_VEC3_stride12','normalized_U16_VEC3_stride8_zero_padding'],
   'feature_id_domain':'FLOAT_SCALAR_stride4' if label.startswith('large-ids') else 'U16_SCALAR_stride4_zero_padding',
   'requires_fill_b3dm':label=='fragmented-fill','expected_point_id_row_count':65537 if label.startswith('large-ids') else None,
   'expected_primitive_modes':[0,3,4] if label=='geometry' else ([3,4] if label=='fragmented-fill' else [0]),
   'requires_line_restart':label=='geometry','requires_polygon_extension':label=='geometry',
   'property_kinds':({'ordinal':'INT64','flag':'BOOLEAN','integer':'INT64','real':'FLOAT64','text':'STRING',
                      'nullable_integer':'INT64','list':'STRING','empty_list':'STRING'} if label=='typed-metadata'
                     else ({'large':'INT64','text':'STRING'} if label=='fragmented-fill' else {}))}
  expected=w/(label+'.expected.json');expected.write_text(json.dumps(expectation,indent=2,ensure_ascii=False)+'\n')
  manifest['cases'].append({'label':label,'input':str(path),'sha256':sha(path),
    'expected':str(expected),'expected_sha256':sha(expected),'features':len(rows),'extra_flags':extra,
    'large_case_requires_separate_root_schedule':label.startswith('large-ids')})
  for mode,binary in BINARIES.items():
   for variant,flags in [('raw',[]),('quantized',['--quantize']),('meshopt',['--meshopt']),('quantized-meshopt',['--quantize','--meshopt'])]:
    # Large-ID raw+meshopt suffice for the actual f32 ID branch. Position
    # quantization is independently covered by all small cases.
    if label.startswith('large-ids') and variant.startswith('quantized'):continue
    out=w/(label+'-'+mode+'-'+variant+'.3tz')
    command=['env','RAYON_NUM_THREADS=2','PROJ_NETWORK=OFF','nice','-n','10',str(binary),'--json','vector',
      '-i',str(path),'-o',str(out),'--explicit','--reproducible','--sourceCrs','local','--jobs','2',
      '--lodLevels','1','--listFields','json',*extra,*flags]
    commands.append(shlex.join(command))
    manifest['cases'][-1].setdefault('runs',[]).append({'mode':mode,'variant':variant,'output':str(out),'command':command,
     'extract_command':['python3','-B',str(HERE/'extract.py'),'--archive',str(out),'--out',str(out)+'.extracted','--expected',str(expected)]})
 (w/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 (w/'root-producer-commands.sh').write_text('# Coordinator schedule only: serial taskset<=2, no parallel sh execution.\n'+'\n'.join(commands)+'\n')
 print(json.dumps({'manifest':str(w/'manifest.json'),'cases':len(cases),'commands':len(commands),'producer_executions':0}))

if __name__=='__main__':main()
