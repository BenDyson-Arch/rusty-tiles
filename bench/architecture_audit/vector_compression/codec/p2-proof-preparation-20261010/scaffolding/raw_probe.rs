//! Isolated precode scaffold. No source graph classifier, codec owner, runtime or paths policy.
//! Fixed authored edits test actual borrowed RawValue serde map/count/GLB emission mechanics.
use serde::{Deserialize,Deserializer,Serialize,Serializer,de::{MapAccess,Visitor},ser::{SerializeMap,SerializeSeq}};
use serde_json::value::RawValue;
use std::{alloc::{GlobalAlloc,Layout,System},borrow::Cow,fmt,io::{self,Write},mem::size_of,sync::atomic::{AtomicUsize,Ordering::SeqCst}};
#[derive(Clone,Copy,Debug)] pub(crate) struct JsonLimits {pub bytes:usize,pub depth:u64,pub value_nodes:usize}
#[derive(Debug)] pub(crate) enum FormatError {InvalidInput(String),Unsupported(String),ResourceLimit(String)}
#[path="../inputs/source/src/content_integrity/json.rs"] mod json;
static LIVE:AtomicUsize=AtomicUsize::new(0); static PEAK:AtomicUsize=AtomicUsize::new(0);
static ALLOCS:AtomicUsize=AtomicUsize::new(0); static REALLOCS:AtomicUsize=AtomicUsize::new(0);
fn peak(n:usize) {PEAK.fetch_max(n,SeqCst);}
struct Meter;
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self,l:Layout)->*mut u8 {peak(LIVE.load(SeqCst).saturating_add(l.size()));let p=System.alloc(l);if !p.is_null(){LIVE.fetch_add(l.size(),SeqCst);ALLOCS.fetch_add(1,SeqCst);}p}
    unsafe fn alloc_zeroed(&self,l:Layout)->*mut u8 {peak(LIVE.load(SeqCst).saturating_add(l.size()));let p=System.alloc_zeroed(l);if !p.is_null(){LIVE.fetch_add(l.size(),SeqCst);ALLOCS.fetch_add(1,SeqCst);}p}
    unsafe fn dealloc(&self,p:*mut u8,l:Layout){System.dealloc(p,l);LIVE.fetch_sub(l.size(),SeqCst);}
    unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8 {
        // Conservative requested overlap BEFORE allocation; actual System may grow in place.
        peak(LIVE.load(SeqCst).saturating_add(n)); REALLOCS.fetch_add(1,SeqCst);
        let q=System.realloc(p,l,n);if !q.is_null(){LIVE.fetch_add(n,SeqCst);LIVE.fetch_sub(l.size(),SeqCst);}q
    }
}
#[global_allocator] static ALLOCATOR:Meter=Meter;
fn start()->(usize,usize,usize){let live=LIVE.load(SeqCst);PEAK.store(live,SeqCst);(live,ALLOCS.load(SeqCst),REALLOCS.load(SeqCst))}
fn report(stage:&str,s:(usize,usize,usize)) {let p=PEAK.load(SeqCst);let l=LIVE.load(SeqCst);let a=ALLOCS.load(SeqCst)-s.1;let r=REALLOCS.load(SeqCst)-s.2;println!("{{\"stage\":\"{}\",\"baselineRequested\":{},\"peakRequestedOldPlusNew\":{},\"liveRequested\":{},\"allocCalls\":{},\"reallocCalls\":{}}}",stage,s.0,p,l,a,r);}
struct Pairs<'a>(Vec<(String,&'a RawValue)>);
impl<'de> Deserialize<'de> for Pairs<'de> {
    fn deserialize<D:Deserializer<'de>>(d:D)->Result<Self,D::Error>{
        struct V;impl<'de> Visitor<'de> for V{type Value=Pairs<'de>;fn expecting(&self,f:&mut fmt::Formatter)->fmt::Result{f.write_str("object")}
        fn visit_map<M:MapAccess<'de>>(self,mut m:M)->Result<Self::Value,M::Error>{let mut p=Vec::new();while let Some(k)=m.next_key::<String>()?{let v=m.next_value::<&RawValue>()?;p.push((k,v));}Ok(Pairs(p))}}
        d.deserialize_map(V)
    }
}
fn pairs(raw:&RawValue)->Pairs<'_>{serde_json::from_str(raw.get()).unwrap()}
fn get<'a>(p:&Pairs<'a>,key:&str)->&'a RawValue{p.0.iter().find(|(k,_)|k==key).unwrap().1}
fn declarations<'a>(p:&Pairs<'a>,key:&str)->Vec<&'a RawValue>{p.0.iter().find(|(k,_)|k==key).map(|(_,v)|serde_json::from_str(v.get()).unwrap()).unwrap_or_default()}
#[derive(Deserialize)]#[serde(rename_all="camelCase")] struct Edit{bin_bytes:u64,fallback_bytes:u64,byte_offset:u64,byte_length:u64,byte_stride:u64,count:u64,metadata:bool}
struct Buffer<'a>{p:&'a Pairs<'a>,edit:&'a Edit}
impl Serialize for Buffer<'_>{fn serialize<S:Serializer>(&self,s:S)->Result<S::Ok,S::Error>{let mut m=s.serialize_map(None)?;for(k,v)in &self.p.0{if k=="byteLength"{m.serialize_entry(k,&self.edit.bin_bytes)?}else{m.serialize_entry(k,v)?}}m.end()}}
struct Fallback<'a>(&'a Edit);
impl Serialize for Fallback<'_>{fn serialize<S:Serializer>(&self,s:S)->Result<S::Ok,S::Error>{let mut m=s.serialize_map(Some(2))?;m.serialize_entry("byteLength",&self.0.fallback_bytes)?;m.serialize_entry("extensions",&serde_json::from_str::<&RawValue>(r#"{"EXT_meshopt_compression":{"fallback":true}}"#).unwrap())?;m.end()}}
struct Buffers<'a>{p:&'a Pairs<'a>,edit:&'a Edit}
impl Serialize for Buffers<'_>{fn serialize<S:Serializer>(&self,s:S)->Result<S::Ok,S::Error>{let mut q=s.serialize_seq(Some(2))?;q.serialize_element(&Buffer{p:self.p,edit:self.edit})?;q.serialize_element(&Fallback(self.edit))?;q.end()}}
struct CodecExtension<'a>(&'a Edit);
impl Serialize for CodecExtension<'_>{fn serialize<S:Serializer>(&self,s:S)->Result<S::Ok,S::Error>{let e=self.0;let mut m=s.serialize_map(Some(7))?;m.serialize_entry("buffer",&0u8)?;m.serialize_entry("byteOffset",&e.byte_offset)?;m.serialize_entry("byteLength",&e.byte_length)?;m.serialize_entry("byteStride",&e.byte_stride)?;m.serialize_entry("count",&e.count)?;m.serialize_entry("mode","ATTRIBUTES")?;m.serialize_entry("filter","NONE")?;m.end()}}
struct Extensions<'a>(&'a Edit);
impl Serialize for Extensions<'_>{fn serialize<S:Serializer>(&self,s:S)->Result<S::Ok,S::Error>{let mut m=s.serialize_map(Some(1))?;m.serialize_entry("EXT_meshopt_compression",&CodecExtension(self.0))?;m.end()}}
struct View<'a>{p:&'a Pairs<'a>,edit:&'a Edit}
impl Serialize for View<'_>{fn serialize<S:Serializer>(&self,s:S)->Result<S::Ok,S::Error>{let mut m=s.serialize_map(None)?;for(k,v)in &self.p.0{match k.as_str(){"buffer"=>m.serialize_entry(k,&1u8)?,"byteOffset"=>m.serialize_entry(k,&self.edit.byte_offset)?,_=>m.serialize_entry(k,v)?}}m.serialize_entry("extensions",&Extensions(self.edit))?;m.end()}}
struct Views<'a>{p:&'a[Pairs<'a>],edit:&'a Edit}
impl Serialize for Views<'_>{fn serialize<S:Serializer>(&self,s:S)->Result<S::Ok,S::Error>{let mut q=s.serialize_seq(Some(self.p.len()))?;for p in self.p{q.serialize_element(&View{p,edit:self.edit})?}q.end()}}
struct Decl<'a>(&'a[&'a RawValue]);
impl Serialize for Decl<'_>{fn serialize<S:Serializer>(&self,s:S)->Result<S::Ok,S::Error>{let mut q=s.serialize_seq(None)?;let mut found=false;for v in self.0{let name:String=serde_json::from_str(v.get()).unwrap();if name=="EXT_meshopt_compression"{found=true;}q.serialize_element(v)?;}if !found{q.serialize_element("EXT_meshopt_compression")?;}q.end()}}
struct Root<'a>{root:&'a Pairs<'a>,buffer:&'a Pairs<'a>,views:&'a[Pairs<'a>],used:&'a[&'a RawValue],required:&'a[&'a RawValue],edit:&'a Edit}
impl Serialize for Root<'_>{fn serialize<S:Serializer>(&self,s:S)->Result<S::Ok,S::Error>{let mut m=s.serialize_map(None)?;for(k,v)in &self.root.0{match k.as_str(){"buffers"=>m.serialize_entry(k,&Buffers{p:self.buffer,edit:self.edit})?,"bufferViews"=>m.serialize_entry(k,&Views{p:self.views,edit:self.edit})?,"extensionsUsed"=>m.serialize_entry(k,&Decl(self.used))?,"extensionsRequired"=>m.serialize_entry(k,&Decl(self.required))?,_=>m.serialize_entry(k,v)?}}if !self.root.0.iter().any(|(k,_)|k=="extensionsUsed"){m.serialize_entry("extensionsUsed",&Decl(self.used))?;}if !self.root.0.iter().any(|(k,_)|k=="extensionsRequired"){m.serialize_entry("extensionsRequired",&Decl(self.required))?;}m.end()}}
struct Count{n:usize,limit:usize}
impl Write for Count{fn write(&mut self,b:&[u8])->io::Result<usize>{self.n=self.n.checked_add(b.len()).filter(|n|*n<=self.limit).ok_or_else(||io::Error::other("count limit"))?;Ok(b.len())}fn flush(&mut self)->io::Result<()>{Ok(())}}
struct Fixed<'a>{out:&'a mut Vec<u8>,limit:usize}
impl Write for Fixed<'_>{fn write(&mut self,b:&[u8])->io::Result<usize>{let end=self.out.len().checked_add(b.len()).filter(|n|*n<=self.limit&&*n<=self.out.capacity()).ok_or_else(||io::Error::other("fixed sink capacity"))?;self.out.extend_from_slice(b);assert_eq!(end,self.out.len());Ok(b.len())}fn flush(&mut self)->io::Result<()>{Ok(())}}
fn align(n:usize,a:usize)->usize{n.checked_add(a-1).unwrap()/a*a}
fn main(){
    let path=std::path::PathBuf::from(std::env::args_os().nth(1).expect("case directory"));
    let input=std::fs::read(path.join("input.json")).unwrap();
    if path.join("expected-admission.json").exists(){
        #[derive(Deserialize)]#[serde(rename_all="camelCase")]struct Expect{bytes:usize,depth:u64,nodes:usize,category:String,expected_nodes:Option<usize>}
        let e:Expect=serde_json::from_slice(&std::fs::read(path.join("expected-admission.json")).unwrap()).unwrap();let s=start();let result=json::admit(&input,JsonLimits{bytes:e.bytes,depth:e.depth,value_nodes:e.nodes});
        let observed=match &result{Ok(doc)=>{if let Some(n)=e.expected_nodes{assert_eq!(doc.value_nodes(),n);} "admitted"},Err(FormatError::InvalidInput(_))=>"invalid_input",Err(FormatError::ResourceLimit(_))=>"resource_limit",Err(FormatError::Unsupported(_))=>"unsupported"};assert_eq!(observed,e.category);report("actual_pinned_admission_boundary",s);println!("{{\"observed\":\"{}\",\"match\":true,\"inputCapacity\":{}}}",observed,input.capacity());return;
    }
    let expected=std::fs::read(path.join("expected.glb")).unwrap();let binary=std::fs::read(path.join("binary.bin")).unwrap();let edit:Edit=serde_json::from_slice(&std::fs::read(path.join("edits.json")).unwrap()).unwrap();
    println!("{{\"typeSizes\":{{\"RawRef\":{},\"Slot\":{},\"CowKey\":{},\"OwnedPair\":{},\"Pairs\":{},\"Edit\":{},\"Root\":{},\"View\":{},\"PathBuf\":{}}},\"inputCapacity\":{},\"oracleCapacityExcludedFromPolicy\":{},\"binaryCapacity\":{}}}",size_of::<&RawValue>(),size_of::<(&RawValue,u64)>(),size_of::<Cow<'_,str>>(),size_of::<(String,&RawValue)>(),size_of::<Pairs<'_>>(),size_of::<Edit>(),size_of::<Root<'_>>(),size_of::<View<'_>>(),size_of::<std::path::PathBuf>(),input.capacity(),expected.capacity(),binary.capacity());
    let s=start();let admitted=json::admit(&input,JsonLimits{bytes:1048576,depth:64,value_nodes:65536}).unwrap();report("actual_pinned_json_admission",s);
    let s=start();let root=pairs(admitted.raw());let buffers:Vec<&RawValue>=serde_json::from_str(get(&root,"buffers").get()).unwrap();assert_eq!(buffers.len(),1);let buffer=pairs(buffers[0]);let refs:Vec<&RawValue>=serde_json::from_str(get(&root,"bufferViews").get()).unwrap();let views:Vec<Pairs<'_>>=refs.iter().map(|v|pairs(v)).collect();let used=declarations(&root,"extensionsUsed");let required=declarations(&root,"extensionsRequired");let out=Root{root:&root,buffer:&buffer,views:&views,used:&used,required:&required,edit:&edit};report("scaffold_owned_key_pairs_and_fixed_edit",s);
    println!("{{\"scaffoldCapacities\":{{\"rootPairSlots\":{},\"bufferPairSlots\":{},\"viewDescriptors\":{},\"viewPairSlots\":{},\"allKeyCapacity\":{},\"nodeCount\":{}}}}}",root.0.capacity(),buffer.0.capacity(),views.capacity(),views.iter().map(|p|p.0.capacity()).sum::<usize>(),root.0.iter().chain(buffer.0.iter()).chain(views.iter().flat_map(|p|p.0.iter())).map(|(k,_)|k.capacity()).sum::<usize>(),admitted.value_nodes());
    let bound=input.len().checked_add(512*views.len()).unwrap()+1024;let s=start();let mut count=Count{n:0,limit:bound};serde_json::to_writer(&mut count,&out).unwrap();report("count_writer",s);
    let mut too_small=Count{n:0,limit:count.n-1};assert!(serde_json::to_writer(&mut too_small,&out).is_err());
    let j=if edit.metadata{align(20+count.n,8)-20}else{align(count.n,4)};let u=align(binary.len(),if edit.metadata{8}else{4});let total=28+j+u;
    let s=start();let mut bytes=Vec::new();bytes.try_reserve_exact(total).unwrap();let actual_capacity=bytes.capacity();bytes.resize(20,0);serde_json::to_writer(Fixed{out:&mut bytes,limit:20+count.n},&out).unwrap();assert_eq!(bytes.len(),20+count.n);bytes.resize(20+j,b' ');bytes.extend_from_slice(&(u as u32).to_le_bytes());bytes.extend_from_slice(b"BIN\0");bytes.extend_from_slice(&binary);bytes.resize(total,0);bytes[..4].copy_from_slice(b"glTF");bytes[4..8].copy_from_slice(&2u32.to_le_bytes());bytes[8..12].copy_from_slice(&(total as u32).to_le_bytes());bytes[12..16].copy_from_slice(&(j as u32).to_le_bytes());bytes[16..20].copy_from_slice(b"JSON");assert_eq!(bytes.capacity(),actual_capacity);assert_eq!(bytes,expected,"independently frozen byte oracle");report("direct_exact_glb_no_output_json_vec",s);
    println!("{{\"exactJsonBytes\":{},\"deltaUpper\":{},\"candidateCapacity\":{},\"total\":{},\"binOrigin\":{},\"metadata\":{},\"exactOracleMatch\":true}}",count.n,bound,actual_capacity,total,28+j,edit.metadata);
    drop(out);drop(views);drop(buffer);drop(root);drop(bytes);drop(binary);drop(expected);drop(refs);drop(buffers);drop(used);drop(required); // No owned plan/BIN/decoder survives this separate identity probe.
    let s=start();let mut identity=Vec::new();identity.try_reserve_exact(input.len()).unwrap();identity.extend_from_slice(&input);assert_eq!(identity,input);report("snapshot_plus_distinct_owned_identity_candidate",s);
    assert!(!std::ptr::eq(identity.as_ptr(),input.as_ptr()));
}
