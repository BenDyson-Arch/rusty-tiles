#!/usr/bin/env python3
"""Independent exact-rational checks of the native static distance enclosure.
Build a tiny stdin kernel with rustc; no Cargo/producer dependency. OUT is external
scratch/evidence. Seeded f32, skinny, degenerate, translated and scaled cases.
"""
import argparse,json,random,struct,subprocess,sys
from pathlib import Path
from fractions import Fraction
from oracle import distance2

def main():
    p=argparse.ArgumentParser();p.add_argument('out',type=Path);a=p.parse_args();a.out.mkdir(exist_ok=True,parents=True)
    src=Path(__file__).resolve().parents[4]/'src/validate/hlod_delivery/drops/certificate.rs'
    # Include the unchanged production module, including interval fallback.
    code='#![allow(dead_code)]\n#[path='+json.dumps(str(src))+'] mod certificate;\n'+r'''
use std::io::{self,BufRead};
fn main(){for line in io::stdin().lock().lines(){let l=line.unwrap();let v:Vec<f64>=l.split_whitespace().map(|x|x.parse().unwrap()).collect();
let mut p=std::array::from_fn(|k| certificate::Interval::point(v[k]));
let t=std::array::from_fn(|i|std::array::from_fn(|k|v[3+3*i+k]));
if v.len()>12 { for _ in 0..32 {p=std::array::from_fn(|k|p[k].add(certificate::Interval::point(t[0][k])).div(certificate::Interval::point(2.)));} }
match certificate::filter(p,t){Some((lo,hi))=>println!("{lo:.17e} {hi:.17e}"),None=>println!("none")}
}}
'''
    (a.out/'filter.rs').write_text(code)
    subprocess.run(['rustc','--edition=2021','-O',str(a.out/'filter.rs'),'-o',str(a.out/'filter')],check=True)
    r=random.Random(313);cases=[]
    f32=lambda x:struct.unpack('<f',struct.pack('<f',x))[0]
    for _ in range(2000):
        scale=2.**r.randint(-120,120)
        t=[[f32(r.uniform(-3,3)*scale) for k in range(3)] for i in range(3)]
        if r.randrange(3)==0:t[2]=[f32(t[0][k]+(t[1][k]-t[0][k])*0.5) for k in range(3)]
        if r.randrange(7)==0:t[2]=t[1][:]
        p=[f32(r.uniform(-3,3)*scale) for k in range(3)]
        cases.append((p,t,r.randrange(2)==0))
    for e in [-330,-300,-150,150,300,330]:
        for _ in range(20):
            scale=2.**e
            t=[[r.uniform(-3,3)*scale for k in range(3)] for i in range(3)]
            cases.append(([r.uniform(-3,3)*scale for k in range(3)],t,False))
    # Original numeric-bound witness, plus power-of-two scalings.
    t=[[-1.4724074926764578e23,2.9648180383498556e22,-7.774404609302878e22],[5.001019053693828e22,1.1372596964707108e23,-1.23504860697962e23],[-3.7499307865693355e22,7.642523899974737e22,-1.032032911033214e23]]
    p=[2.9580527309896196e22,1.0501786345078302e23,-1.1876531748611357e23]
    for e in [-100,-40,0,40]:cases.append(([x*2.**e for x in p],[[x*2.**e for x in v] for v in t],False))
    lines=[' '.join(map(repr,p+[x for v in t for x in v]))+(' 1' if sub else '') for p,t,sub in cases]
    run=subprocess.run([str(a.out/'filter')],input='\n'.join(lines),text=True,capture_output=True,check=True)
    count=0
    for (p,t,sub),line in zip(cases,run.stdout.splitlines(),strict=True):
        if sub:
            p=[Fraction(x) for x in p]
            for _ in range(32):p=[(p[k]+Fraction(t[0][k]))/2 for k in range(3)]
        exact=distance2(p,t)
        assert line!='none'
        lo,hi=map(Fraction,map(float,line.split()))
        assert lo*lo<=exact<=hi*hi,(p,t,line,exact)
        count+=1
    (a.out/'result.json').write_text(json.dumps({'seed':313,'exactEnclosures':count,'depth32Cases':sum(c[2] for c in cases)},indent=2)+'\n')
    print((a.out/'result.json').read_text())
if __name__=='__main__':main()
