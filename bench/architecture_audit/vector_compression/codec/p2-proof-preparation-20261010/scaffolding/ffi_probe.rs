//! Isolated locked FFI probe: bounds/guards/bytes, not a production codec or conformance oracle.
use serde::Deserialize;
use std::path::Path;
#[derive(Deserialize)]#[serde(rename_all="camelCase")] struct Case{name:String,path:String,count:usize,stride:usize,expected_bound:usize}
#[derive(Deserialize)] struct Manifest{cases:Vec<Case>}
fn bound(n:usize,s:usize)->usize{assert!(n>0&&s>0&&s%4==0&&s<=256);let b=256.min(16*(8192/(16*s)));let c=n/b+usize::from(n%b!=0);let h=b/64+usize::from(b%64!=0);1+c*s*(s/4+h+b)+32.max(s+s/4)}
fn length_admitted(n:usize,b:usize)->bool{n>0&&n<=b}
fn decode(encoded:&[u8],n:usize,s:usize)->(i32,Vec<u8>){
    let d=n.checked_mul(s).unwrap();assert!(n>0&&s%4==0&&s<=256);let nwords=d/4+usize::from(d%4!=0);let mut words=vec![0x5a5a5a5au32;nwords+16];
    let code=unsafe{meshopt::ffi::meshopt_decodeVertexBuffer(words[8..].as_mut_ptr().cast(),n,s,encoded.as_ptr(),encoded.len())};
    assert!(words[..8].iter().chain(words[8+nwords..].iter()).all(|w|*w==0x5a5a5a5a),"decoder aligned guards changed");
    let bytes=words[8..8+nwords].iter().flat_map(|w|w.to_ne_bytes()).take(d).collect();(code,bytes)
}
fn main(){
    let arg=std::env::args_os().nth(1).expect("streams directory");let dir=Path::new(&arg);let manifest:Manifest=serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    for c in manifest.cases{
        let raw=std::fs::read(dir.join(&c.path)).unwrap();assert_eq!(raw.len(),c.count.checked_mul(c.stride).unwrap());
        let b=bound(c.count,c.stride);assert_eq!(b,c.expected_bound);let native=unsafe{meshopt::ffi::meshopt_encodeVertexBufferBound(c.count,c.stride)};assert_eq!(native,b);
        let mut source=vec![0x39u8];source.extend_from_slice(&raw); // Byte source starts at deliberately unaligned+1 address.
        let mut out=vec![0xc7u8;b+64];let n=unsafe{meshopt::ffi::meshopt_encodeVertexBuffer(out[32..].as_mut_ptr(),b,source[1..].as_ptr().cast(),c.count,c.stride)};
        assert!(length_admitted(n,b));assert!(!length_admitted(0,b));assert!(!length_admitted(b+1,b));
        assert!(out[..32].iter().chain(out[32+b..].iter()).all(|v|*v==0xc7),"bound destination guard changed");
        let(code,decoded)=decode(&out[32..32+n],c.count,c.stride);assert_eq!(code,0);assert_eq!(decoded,raw,"includes every padding byte");
        let mut tiny=vec![0xacu8;65];let rejected=unsafe{meshopt::ffi::meshopt_encodeVertexBuffer(tiny[32..].as_mut_ptr(),1,source[1..].as_ptr().cast(),c.count,c.stride)};assert_eq!(rejected,0);assert!(tiny[..32].iter().chain(tiny[33..].iter()).all(|v|*v==0xac));
        println!("{{\"name\":\"{}\",\"bound\":{},\"encodedLength\":{},\"sourceBytes\":{},\"guardsIntact\":true,\"sameKernelWholeByteMatch\":true,\"tinyDestinationRejected\":true}}",c.name,b,n,raw.len());
    }
    let encoded=std::fs::read(dir.join("golden-encoded.bin")).unwrap();let expected=std::fs::read(dir.join("golden-expected.bin")).unwrap();assert_eq!(encoded.len(),85);assert_eq!(expected.len(),48);
    let(code,bytes)=decode(&encoded,4,12);assert_eq!(code,0);assert_eq!(bytes,expected);
    let mut bad=encoded.clone();bad[0]=0;let(code,_)=decode(&bad,4,12);assert_eq!(code,-1,"pinned header mask rejects0");
    let(code,_)=decode(&encoded[..1],4,12);assert_eq!(code,-2,"pinned tail requirement rejects header-only input");
    for n in [3,5]{let(code,bytes)=decode(&encoded,n,12);if code==0{assert_ne!(bytes.len(),expected.len());}println!("{{\"goldenWrongCount\":{},\"decodeCode\":{},\"expected48ByteSpanMismatch\":true,\"claim\":\"characterization_not_universal_InvalidInput\"}}",n,code);}
    // A valid altered stream stays decodable but must fail its original independent byte oracle.
    let mut changed=expected.clone();changed[1]^=0x40;let b=bound(4,12);let mut out=vec![0u8;b];let n=unsafe{meshopt::ffi::meshopt_encodeVertexBuffer(out.as_mut_ptr(),b,changed.as_ptr().cast(),4,12)};assert!(length_admitted(n,b));let(code,actual)=decode(&out[..n],4,12);assert_eq!(code,0);assert_eq!(actual,changed);assert_ne!(actual,expected);
    println!("{{\"literalGoldenMatch\":true,\"provenMalformedHeaderAndHeaderOnlyRejected\":true,\"decodableByteMutationDetectedByOracle\":true,\"evidence\":\"locked_FFI_probe_only_not_format_admission_or_independent_kernel_conformance\"}}");
}
