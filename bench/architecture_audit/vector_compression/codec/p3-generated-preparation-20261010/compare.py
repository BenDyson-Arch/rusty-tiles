#!/usr/bin/env python3
"""Pair raw/current compressed producer payloads by preserved associations.

Never use container/member names or generated JSON serialization as expected
bit truth. Independently extracted physical view bytes must match by index.
"""
import argparse,json,hashlib
from pathlib import Path
def load(root):
 return [(p,json.loads(p.read_text())) for p in root.glob('payload-*/extraction.json')]
def main():
 p=argparse.ArgumentParser();p.add_argument('raw',type=Path);p.add_argument('compressed',type=Path);a=p.parse_args()
 left=load(a.raw);right=load(a.compressed);used=set();records=[]
 for lp,l in left:
  matches=[(i,rp,r) for i,(rp,r) in enumerate(right) if i not in used and r['associations']==l['associations']]
  assert len(matches)==1,('nonunique association match',lp,len(matches));i,rp,r=matches[0];used.add(i)
  assert not l['missing_consumer_decoded_views'] and not r['missing_consumer_decoded_views']
  assert l['raw_association_values']==r['raw_association_values'],'authored association value lexemes changed'
  assert len(l['views'])==len(r['views'])
  for lv,rv in zip(l['views'],r['views']):
   lb=(lp.parent/('view-%04d.bin'%lv['index'])).read_bytes();rb=(rp.parent/('view-%04d.bin'%rv['index'])).read_bytes()
   assert lb==rb,('physical view bytes changed',lv['index'])
  records.append({'raw_payload':str(lp.parent),'compressed_payload':str(rp.parent),'views':len(l['views'])})
 assert len(used)==len(right)
 print(json.dumps({'status':'paired_exact_logical_view_bytes_and_associations_equal','records':records,
  'compare_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
  'scope':'compression preservation only; actual source geometry/format conformance separate'}))
if __name__=='__main__':main()
