#!/usr/bin/env python3
"""Independently check the default combined scale archive, including all CRCs.

Streams hashing and padded members; availability is read from each raw subtree.
This does not certify validator traversal or browser acceptance.
"""
import argparse,hashlib,json,pathlib,struct,sys,unittest,zipfile
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--input',type=pathlib.Path,required=True,help='Default combined generator output directory')
parser.add_argument('--report-json',type=pathlib.Path,required=True)
args=parser.parse_args()
root=pathlib.Path(__file__).resolve().parents[2]; out=args.input.resolve(); path=out/'archive.3tz'
if args.report_json.resolve() in {path.resolve(),(out/'manifest.json').resolve()}:parser.error('report must differ from archive and manifest')
sys.path.insert(0,str(root/'bench/scale_gates')); import test_scale_gates as oracle
m=json.loads((out/'manifest.json').read_text()); before=path.stat(); digest=hashlib.sha256()
with path.open('rb') as f:
 while data:=f.read(1<<20):digest.update(data)
sha=digest.hexdigest()
assert m['files']['archive.3tz']=={'bytes':before.st_size,'sha256':sha}
with zipfile.ZipFile(path) as z:
 doc=json.loads(z.read('tileset.json')); impl=doc['root']['implicitTiling']; A=impl['availableLevels']; L=impl['subtreeLevels']; assert (A,L)==(12,6)
 assert impl['subdivisionScheme']=='QUADTREE' and impl['subtrees']['uri']=='subtrees/{level}/{x}/{y}.subtree'
 assert doc['root']['content']['uri']=='content/{level}/{x}/{y}.glb'
 tiles=oracle.walk_implicit(z,L,A); oracle.check_index(unittest.TestCase(),path)
 infos=z.infolist(); by_offset={i.header_offset:i.filename for i in infos[:-1]}; index=z.read('@3dtilesIndex1@')
 actual=[(index[i:i+16],struct.unpack_from('<Q',index,i+16)[0]) for i in range(0,len(index),24)]
 for info in infos:
  if info.filename.endswith('.subtree') and info.filename!='subtrees/0/0/0.subtree':
   raw=z.read(info.filename); _,_,jl,_=struct.unpack_from('<4sIQQ',raw); assert json.loads(raw[24:24+jl])['contentAvailability']==[{'constant':0}]
 expected=sorted([(hashlib.md5(name.encode()).digest(),offset) for offset,name in by_offset.items()],key=lambda row:struct.unpack('<QQ',row[0])); assert actual==expected
 raw=z.read('subtrees/0/0/0.subtree'); _,_,jl,bl=struct.unpack_from('<4sIQQ',raw); sd=json.loads(raw[24:24+jl]); view=sd['bufferViews'][0]; bits=raw[24+jl+view['byteOffset']:24+jl+view['byteOffset']+view['byteLength']]; contents=sum(b.bit_count() for b in bits); assert contents==85==sd['contentAvailability'][0]['availableCount']
 assert all(bool(bits[i//8]&(1<<(i%8)))==(i<85) for i in range(((1<<(2*L))-1)//3))
 assert bits[-1]>>((((1<<(2*L))-1)//3)%8)==0
 expected_contents={f'content/{l}/{x}/{y}.glb' for l in range(4) for y in range(1<<l) for x in range(1<<l)}; actual_contents={i.filename for i in infos if i.filename.endswith('.glb')}; assert actual_contents==expected_contents
 half=doc['root']['boundingVolume']['box'][3]; hz=doc['root']['boundingVolume']['box'][11]
 for name in sorted(actual_contents):
  info=z.getinfo(name); assert info.file_size==56*(1<<20)
  with z.open(name) as f: head=f.read(4096)
  magic,version,total=struct.unpack_from('<4sII',head); assert (magic,version,total)==(b'glTF',2,info.file_size)
  jl,kind=struct.unpack_from('<I4s',head,12); assert kind==b'JSON'; gd=json.loads(head[20:20+jl]); bl,bkind=struct.unpack_from('<I4s',head,20+jl); assert bkind==b'BIN\0' and 28+jl+bl==total
  assert gd['accessors'][0]['componentType']==5126 and gd['accessors'][0]['count']==3
  points=struct.unpack_from('<9f',head,28+jl); _,lv,x,y=name[:-4].split('/'); lv,x,y=map(int,(lv,x,y)); size=2*half/(1<<lv); lo=(-half+x*size,-half+y*size,-hz); hi=(lo[0]+size,lo[1]+size,hz)
  for j in range(0,9,3):
   point=(points[j],-points[j+2],points[j+1]); assert all(lo[a]<=point[a]<=hi[a] for a in range(3))
 assert z.testzip() is None
 assert any(i.header_offset>=(1<<32)-1 for i in infos)
 assert len(infos)==4184 and sum(i.filename.endswith('.subtree') for i in infos)==4097
assert before.st_size>4*(1<<30) and tiles>=1<<22
assert m['truth']['tiles']==tiles and m['truth']['archive_bytes']==before.st_size and m['truth']['combined_scale_met'] is True
assert (path.stat().st_size,path.stat().st_mtime_ns)==(before.st_size,before.st_mtime_ns)
r={'passed':True,'scope':'Independent full sparse archive tile/count/content/CRC/index/ZIP64/bounds inventory, not successful validator traversal or browser acceptance','archive':str(path),'sha256':sha,'tiles':tiles,'archive_bytes':before.st_size,'allocated_bytes':before.st_blocks*512,'members':4184,'subtrees':4097,'contents':contents,'max_member_bytes':56*(1<<20),'both_scale_thresholds':True}; args.report_json.write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r))
