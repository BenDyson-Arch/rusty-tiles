#!/usr/bin/env python3
"""Independent tiny stored-3TZ physical-overlap control; no product imports."""
import hashlib,json,struct,subprocess,zlib
from pathlib import Path

def js(v):return json.dumps(v,separators=(',',':')).encode()
def local(name,data):
 n=name.encode();crc=zlib.crc32(data)
 return struct.pack('<IHHHHHIIIHH',0x04034b50,20,0x800,0,0,0,crc,len(data),len(data),len(n),0)+n+data
def central(name,data,offset):
 n=name.encode()
 return struct.pack('<IHHHHHHIIIHHHHHII',0x02014b50,20,20,0x800,0,0,0,zlib.crc32(data),len(data),len(data),len(n),0,0,0,0,0,offset)+n
def glb():
 b=struct.pack('<9f',0,0,0,1,0,0,0,1,0)
 d=js({'asset':{'version':'2.0'},'scene':0,'scenes':[{'nodes':[0]}],'nodes':[{'mesh':0}],'meshes':[{'primitives':[{'mode':0,'attributes':{'POSITION':0}}]}],'buffers':[{'byteLength':len(b)}],'bufferViews':[{'buffer':0,'byteLength':len(b)}],'accessors':[{'bufferView':0,'componentType':5126,'count':3,'type':'VEC3','min':[0,0,0],'max':[1,1,0]}]})
 d+=b' '*((-len(d))%4)
 return struct.pack('<4sII',b'glTF',2,28+len(d)+len(b))+struct.pack('<I4s',len(d),b'JSON')+d+struct.pack('<I4s',len(b),b'BIN\0')+b

def archive(overlap):
 p=glb();inner=local('points.glb',p);outer=inner if overlap else b'opaque historical bytes'
 manifest=js({'asset':{'version':'1.1'},'geometricError':0,'root':{'boundingVolume':{'box':[0,0,0,10,0,0,0,10,0,0,0,10]},'geometricError':0,'refine':'REPLACE','content':{'uri':'points.glb'}}})
 blob=bytearray();rows=[]
 def add(n,d):
  offset=len(blob);blob.extend(local(n,d));rows.append((n,d,offset));return offset
 add('tileset.json',manifest);add('conversion.json',js({'geometryReports':'outer.bin'}));o=add('outer.bin',outer)
 if overlap:rows.append(('points.glb',p,o+30+len('outer.bin')))
 else:add('points.glb',p)
 index=b''.join(h+struct.pack('<Q',offset) for h,offset in sorted(((hashlib.md5(n.encode()).digest(),o) for n,d,o in rows),key=lambda x:struct.unpack('<QQ',x[0])))
 add('@3dtilesIndex1@',index);cd=len(blob)
 for n,d,o in rows:blob.extend(central(n,d,o))
 size=len(blob)-cd;blob.extend(struct.pack('<IHHHHIIH',0x06054b50,0,0,len(rows),len(rows),size,cd,0))
 return bytes(blob)

def main():
 import argparse
 parser=argparse.ArgumentParser();parser.add_argument('binary');parser.add_argument('output');args=parser.parse_args()
 out=Path(args.output);out.mkdir(parents=True,exist_ok=False);records=[]
 for name,overlap in [('disjoint-control',False),('nested-physical-extents',True)]:
  data=archive(overlap);p=out/(name+'.3tz');p.write_bytes(data)
  command=['nice','-n','10',args.binary,'validate',str(p)]
  r=subprocess.run(command,capture_output=True,timeout=30)
  records.append({'case':name,'archive_bytes':len(data),'archive_sha256':hashlib.sha256(data).hexdigest(),'command':command,'returncode':r.returncode,'stdout':r.stdout.decode(),'stderr':r.stderr.decode(),'physical_extents_disjoint':not overlap})
 receipt={'scope':'two tiny stored archives, independently authored physical nesting control; not full ZIP admission acceptance','binary_sha256':hashlib.sha256(Path(args.binary).read_bytes()).hexdigest(),'driver_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'records':records}
 (out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2))
if __name__=='__main__':main()
