#!/usr/bin/env python3
"""Pair producer content by independently read authored tileset slot addresses.

No hashed-filename identity or byte-based match selector. The source archives
are transport/document truth; supplied independently decoded views are values
at those addresses. This does not certify world accuracy or general 3TZ input.
"""
import argparse,binascii,hashlib,json,posixpath,zipfile
from pathlib import Path
from urllib.parse import urlsplit
def sha(b):return hashlib.sha256(b).hexdigest()
def catalog(archive):
 with zipfile.ZipFile(archive) as z:
  infos=z.infolist();assert len(infos)==len({i.filename for i in infos})
  files={}
  for i in infos:
   b=z.read(i);assert binascii.crc32(b)&0xffffffff==i.CRC;files[i.filename]=b
 return files
def resource(base,uri):
 u=urlsplit(uri);assert not u.scheme and not u.netloc and not u.query and not u.fragment
 assert '%' not in uri and '\\' not in uri and not uri.startswith('/')
 p=posixpath.normpath(posixpath.join(posixpath.dirname(base),uri));assert p!='..' and not p.startswith('../')
 return p
def addresses(files):
 slots={};frames={};active=set();visits=0
 def document(member,address):
  nonlocal visits
  assert member not in active,'external tileset cycle';active.add(member)
  d=json.loads(files[member]);assert isinstance(d.get('root'),dict)
  frames[address+'/document']={'geometricError':d.get('geometricError'),
    'gltfUpAxis':d.get('asset',{}).get('gltfUpAxis')}
  tile(d['root'],member,address+'/root');active.remove(member)
 def tile(t,base,address):
  nonlocal visits
  visits+=1;assert visits<=65536
  frames[address]={k:t[k] for k in ['boundingVolume','transform','geometricError','refine'] if k in t}
  assert not ('content' in t and 'contents' in t)
  content=[('content',t['content'])] if 'content' in t else [ ('contents/'+str(i),c) for i,c in enumerate(t.get('contents',[]))]
  for slot,c in content:
   at=address+'/'+slot;assert at not in slots and at not in frames
   frames[at]={k:v for k,v in c.items() if k!='uri'}
   member=resource(base,c['uri']);assert member in files
   raw=files[member]
   if raw[:4] in [b'glTF',b'b3dm']:slots[at]=member
   else:
    assert member.endswith('.json'),'outside generated finite content profile'
    document(member,at+'/external')
  for i,c in enumerate(t.get('children',[])):tile(c,base,address+'/children/'+str(i))
 document('tileset.json','tileset')
 return slots,frames
def extracted(root):
 receipt=json.loads((root/'receipt.json').read_bytes());members={}
 for record in receipt['records']:
  member=record['member'];assert member not in members
  folder=root/record['folder'];report=json.loads((folder/'extraction.json').read_bytes())
  assert not report['missing_consumer_decoded_views'],'consumer decoding incomplete'
  assert record['member_sha256']
  values=[(folder/('view-%04d.bin'%v['index'])).read_bytes() for v in report['views']]
  members[member]={'associations':report['associations'],'raw_association_values':report['raw_association_values'],
    'wrapper_kind':'b3dm' if report['framing_observation']['wrapper'] else 'glb', 'views':values,
    'member_sha256':record['member_sha256']}
 return receipt,members
def compare(left_slots,left_frames,left,right_slots,right_frames,right):
 assert left_frames==right_frames,'authored hierarchy/frame or content-slot facts changed'
 assert set(left_slots)==set(right_slots),'content slot duplicate/omission/address change'
 assert set(left_slots.values())==set(left),'left extraction does not exactly cover referenced payloads'
 assert set(right_slots.values())==set(right),'right extraction does not exactly cover referenced payloads'
 result=[]
 for address in sorted(left_slots):
  lm=left_slots[address];rm=right_slots[address];l=left[lm];r=right[rm]
  assert l['wrapper_kind']==r['wrapper_kind'],('wrapper changed at authored slot',address)
  assert l['associations']==r['associations'],('association graph changed at authored slot',address)
  assert l['raw_association_values']==r['raw_association_values'],('raw association lexemes changed',address)
  assert len(l['views'])==len(r['views']),('view inventory changed',address)
  for i,(lb,rb) in enumerate(zip(l['views'],r['views'])):
   assert lb==rb,('logical view bytes changed at authored slot',address,i)
  result.append({'address':address,'left_member':lm,'right_member':rm,'views':len(l['views']),
                 'logical_view_sha256':[sha(v) for v in l['views']]})
 return result
def selftest():
 # Literal independent address model: congruent association graphs but unequal
 # geometry/padding bits, exactly the selector ambiguity exposed by fill.
 frames={'tileset/root':{'boundingVolume':{'box':[0]*12}},
         'tileset/root/children/0':{},'tileset/root/children/1':{}}
 slots={'tileset/root/children/0/content':'old-A.glb','tileset/root/children/1/content':'old-B.glb'}
 def payload(bits):return {'wrapper_kind':'glb','associations':{'same':'graph'},'raw_association_values':{'same':'1'},'views':[bits]}
 left={'old-A.glb':payload(b'\x01\x00\x00\x00'),'old-B.glb':payload(b'\x02\x00\x00\x00')}
 right={'new-X.glb':payload(b'\x01\x00\x00\x00'),'new-Y.glb':payload(b'\x02\x00\x00\x00')}
 target=dict(zip(slots,['new-X.glb','new-Y.glb']));compare(slots,frames,left,target,frames,right)
 controls={}
 def rejected(name,s,f,r):
  try:compare(slots,frames,left,s,f,r)
  except AssertionError as e:controls[name]=str(e)
  else:raise AssertionError('insensitive control '+name)
 rejected('omitted_slot',{next(iter(target)): 'new-X.glb'},frames,right)
 rejected('duplicated_alias_replaces_second_payload',dict(zip(slots,['new-X.glb','new-X.glb'])),frames,{'new-X.glb':right['new-X.glb']})
 rejected('swapped_congruent_graph_payloads',dict(zip(slots,['new-Y.glb','new-X.glb'])),frames,right)
 changed=dict(right);changed['new-X.glb']=payload(b'\x01\x00\x00\x80');rejected('padding_bit_changed',target,frames,changed)
 changed=dict(right);changed['new-X.glb']=dict(right['new-X.glb'],associations={'same':'other'});rejected('wrong_association',target,frames,changed)
 changed=dict(frames);changed['tileset/root/children/0']={'transform':[2]*16};rejected('changed_child_frame',target,changed,right)
 rejected('unreferenced_extra_payload',target,frames,{**right,'extra.glb':payload(b'x')})
 return {'status':'MODEL_SELFTEST_ONLY_NOT_TARGET_ACCEPTANCE','controls':controls}
def main():
 p=argparse.ArgumentParser();p.add_argument('--selftest',action='store_true')
 for k in ['raw-archive','compressed-archive','raw-extracted','compressed-extracted']:p.add_argument('--'+k,type=Path)
 p.add_argument('--receipt',type=Path);a=p.parse_args()
 if a.selftest:result=selftest()
 else:
  assert all([a.raw_archive,a.compressed_archive,a.raw_extracted,a.compressed_extracted])
  lf=catalog(a.raw_archive);rf=catalog(a.compressed_archive);ls,la=addresses(lf);rs,ra=addresses(rf)
  lr,l=extracted(a.raw_extracted);rr,r=extracted(a.compressed_extracted)
  assert lr['archive_sha256']==sha(a.raw_archive.read_bytes()) and rr['archive_sha256']==sha(a.compressed_archive.read_bytes())
  for name,v in l.items():assert v['member_sha256']==sha(lf[name])
  for name,v in r.items():assert v['member_sha256']==sha(rf[name])
  result={'status':'authored_slot_exact_view_and_association_preservation_pass','slots':compare(ls,la,l,rs,ra,r),
   'raw_archive_sha256':sha(a.raw_archive.read_bytes()),'compressed_archive_sha256':sha(a.compressed_archive.read_bytes()),
   'scope':'finite explicit hierarchy/content-slot/frame association and decoded logical byte preservation; no world geometry accuracy/quantization or arbitrary external URI conformance'}
 result['probe_sha256']=sha(Path(__file__).read_bytes());text=json.dumps(result,indent=2)+'\n'
 if a.receipt:
  with a.receipt.open('x') as f:f.write(text)
 print(text,end='')
if __name__=='__main__':main()
