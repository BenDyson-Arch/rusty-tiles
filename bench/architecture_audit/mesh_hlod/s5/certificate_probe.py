#!/usr/bin/env python3
"""Extract N3 certificate workloads from published bytes, or build a source-exact kernel probe.
No producer imports. Bulk corpus/binaries belong in the external evidence directory.
Usage: certificate_probe.py corpus ARCHIVE OUT; certificate_probe.py harness SOURCE_ROOT OUT.rs
The harness replaces only Effort/error/hash-map plumbing and times Grid::covers,
not archive decode or grid construction. It counts original distance calls or
new fast/slow calls, and supplies a separate per-test microbenchmark.
"""
import sys,struct,json,hashlib
from pathlib import Path
from archive import Archive,expand
from oracle import decode

def corpus(path,out):
    cases=[]; cache={}
    with Archive(path) as a:
        tiles=expand(a)
        def keys(t):
            faces=[]
            for uri in t['contents']:
                data=a.read(uri); h=hashlib.sha256(data).digest()
                if h not in cache:
                    # Coarse faces retain decode order/multiplicity; leaves group sorted keys.
                    fs=[]
                    for f in decode(data)['faces']:
                        ks=tuple(tuple(0 if v==0 else struct.unpack('<I',struct.pack('<f',v))[0] for v in p) for p in f['xyz'])
                        if len(set(ks))==3: fs.append(ks)
                    cache[h]=fs
                faces.extend(cache[h])
            if t['children']: return faces
            return sorted({tuple(sorted(f)) for f in faces},key=lambda f:struct.pack('<9I',*(v for p in f for v in p)))
        def digest(t): return tuple(sorted(hashlib.sha256(a.read(u)).digest() for u in t['contents']))
        def children(t,d):
            result=[]
            for c in t['children']:
                if not c['contents'] or digest(c)==d: result.extend(children(c,d))
                else: result.append(c)
            return result
        for t in tiles:
            if not t['children'] or not t['contents']: continue
            assert t['node'].get('transform') is None, 'relative transforms require explicit canonical-frame conversion'
            stages=[]
            for uri in t['contents']:
                d=a.read(uri); doc=json.loads(d[20:20+int.from_bytes(d[12:16],'little')]);stages.append(doc.get('asset',{}).get('extras',{}).get('rustyTiles',{}).get('lastResort',{}).get('stage'))
            if 3 not in stages: continue
            parent=keys(t); kept={p for f in parent for p in f}; roots={}
            def find(p):
                while roots[p]!=p:
                    roots[p]=roots[roots[p]];p=roots[p]
                return p
            cs=children(t,digest(t)); child=[f for c in cs for f in keys(c)]
            for f in child:
                for p in f: roots.setdefault(p,p)
                for p in f[1:]:
                    x,y=find(f[0]),find(p);roots[max(x,y)]=min(x,y)
            present={find(p) for p in roots if p in kept}
            dropped=[f for f in child if find(f[0]) not in present]
            if dropped: cases.append((t['g'],max(c['g'] for c in cs),parent,dropped))
    with open(out,'wb') as f:
        f.write(struct.pack('<I',len(cases)))
        for g,slack,parents,drops in cases:
            f.write(struct.pack('<ddII',g,slack,len(parents),len(drops)))
            for face in parents+drops:f.write(struct.pack('<9I',*(v for p in face for v in p)))
    print(json.dumps({'archiveSha256':hashlib.sha256(Path(path).read_bytes()).hexdigest(),'cases':len(cases),'parents':sum(len(c[2]) for c in cases),'droppedTriangles':sum(len(c[3]) for c in cases),'corpusSha256':hashlib.sha256(Path(out).read_bytes()).hexdigest()}))

def harness(root,out):
    root=Path(root); s=(root/'drops.rs').read_text(); cert=(root/'certificate.rs').read_text()
    grid=s[s.index('struct Grid {'):s.index('#[cfg(test)]\nmod tests')]
    modern='filtered_upper' in grid
    if not modern: grid=grid.replace('distance2_upper(center, face)', '{ effort.certificate_fast.fetch_add(1,Ordering::Relaxed); distance2_upper(center, face) }').replace('distance2_upper(p, face)', '{ effort.certificate_fast.fetch_add(1,Ordering::Relaxed); distance2_upper(p, face) }')
    prefix='''#![allow(dead_code)]
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64,Ordering};
type CanonicalPoint=[u32;3]; type Error=String;
const GRID:f64=32.; const TOLERANCE:f64=1e-6;
#[derive(Default)] struct Effort { certificate_fast:AtomicU64,certificate_slow:AtomicU64 }
impl Effort { fn charge(&self,_n:usize)->Result<(),Error>{Ok(())} }
fn point(p:CanonicalPoint)->[f64;3]{p.map(|x|f64::from(f32::from_bits(x)))}
'''
    main='''
#[repr(C)] struct Timespec {sec:i64,nsec:i64}
unsafe extern "C" {fn clock_gettime(clock:i32,t:*mut Timespec)->i32;}
fn cpu()->u128{let mut t=Timespec{sec:0,nsec:0};assert_eq!(unsafe{clock_gettime(2,&mut t)},0);t.sec as u128*1_000_000_000+t.nsec as u128}
fn main(){
 let data=std::fs::read(std::env::args().nth(1).unwrap()).unwrap(); let mut cursor=0;
 fn u32at(d:&[u8],c:&mut usize)->u32{let v=u32::from_le_bytes(d[*c..*c+4].try_into().unwrap());*c+=4;v}
 fn f64at(d:&[u8],c:&mut usize)->f64{let v=f64::from_le_bytes(d[*c..*c+8].try_into().unwrap());*c+=8;v}
 let cases=u32at(&data,&mut cursor);let effort=Effort::default();let mut ns=0u128;
 for _ in 0..cases {
  let g=f64at(&data,&mut cursor);let slack=f64at(&data,&mut cursor);
  let n=u32at(&data,&mut cursor);let m=u32at(&data,&mut cursor);
  let mut read=|n| (0..n).map(|_| std::array::from_fn(|_|std::array::from_fn(|_|u32at(&data,&mut cursor)))).collect::<Vec<[[u32;3];3]>>();
  let faces=read(n);let drops=read(m);let grid=Grid::new(&faces,g,slack,&effort).unwrap();
  let start=cpu();
  for t in drops { assert!(grid.covers(t.map(point),&faces,&effort).unwrap()); }
  ns+=cpu()-start;
 }
 let fast=effort.certificate_fast.load(Ordering::Relaxed);let slow=effort.certificate_slow.load(Ordering::Relaxed);
 println!("cases={} certificate_ns={} fast={} slow={} ns_per_test={:.3}",cases,ns,fast,slow,ns as f64/fast as f64);
}
'''
    if modern:
        main=main.replace('fn main(){', 'fn main(){if std::env::args().nth(2).as_deref()==Some("micro"){micro();return;}')
        main += '''fn micro(){
 let n=300_000;
 for (name,p,r) in [("box",[100.,100.,100.],1.),("filter",[0.5,0.5,1.],1.1),("interval",[0.5,0.5,1.],1.)] {
  let slow=AtomicU64::new(0);let start=cpu();
  for _ in 0..n {
   std::hint::black_box(certificate::filtered_upper(std::hint::black_box(p.map(certificate::Interval::point)),std::hint::black_box([[0.,0.,0.],[2.,0.,0.],[0.,2.,0.]]),std::hint::black_box(r),std::hint::black_box(r),&slow));
  }
  println!("{} tests={} slow={} cpu_ns={} ns_per_test={:.3}",name,n,slow.load(Ordering::Relaxed),cpu()-start,(cpu()-start) as f64/n as f64);
 }
}
'''
    Path(out).write_text(prefix+'mod certificate {\n'+cert+'\n}\n'+grid+main)
if __name__=='__main__':
    if sys.argv[1]=='corpus':corpus(*sys.argv[2:])
    else:harness(*sys.argv[2:])
