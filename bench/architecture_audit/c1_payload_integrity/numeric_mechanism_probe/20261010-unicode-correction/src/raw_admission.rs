use serde::{Deserialize,de::{self,DeserializeSeed,Visitor,MapAccess,SeqAccess}};
use serde_json::value::RawValue;
use std::{cell::Cell,collections::BTreeSet};

struct State {nodes:Cell<usize>,limited:Cell<bool>,max_nodes:usize,max_depth:usize}
#[derive(Clone,Copy)]
struct Child<'a>{state:&'a State,depth:usize}
struct ScalarString;
impl<'de> Visitor<'de> for ScalarString {
    type Value=();
    fn expecting(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str("valid Unicode JSON string")}
    fn visit_str<E:de::Error>(self,_:&str)->Result<(),E>{Ok(())}
}
impl Child<'_> {
    fn charge<E:de::Error>(self)->Result<(),E>{
        if self.depth>self.state.max_depth || self.state.nodes.get()>=self.state.max_nodes {
            self.state.limited.set(true);return Err(E::custom("JSON depth/items admission"));
        }
        self.state.nodes.set(self.state.nodes.get()+1);Ok(())
    }
}
impl<'de> DeserializeSeed<'de> for Child<'_>{
    type Value=&'de RawValue;
    fn deserialize<D:de::Deserializer<'de>>(self,d:D)->Result<Self::Value,D::Error>{self.charge()?; <&RawValue>::deserialize(d)}
}
impl<'de> Visitor<'de> for Child<'_>{
    type Value=Vec<(&'de RawValue,usize)>;
    fn expecting(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str("borrowed bounded JSON container")}
    fn visit_seq<A:SeqAccess<'de>>(self,mut access:A)->Result<Self::Value,A::Error>{
        let mut children=Vec::new();
        while let Some(raw)=access.next_element_seed(self)? {children.push((raw,self.depth));}
        Ok(children)
    }
    fn visit_map<A:MapAccess<'de>>(self,mut access:A)->Result<Self::Value,A::Error>{
        let mut keys=BTreeSet::new();let mut children=Vec::new();
        while let Some(key)=access.next_key::<String>()? {
            if !keys.insert(key) {return Err(de::Error::custom("duplicate JSON key"));}
            let raw=access.next_value_seed(self)?;children.push((raw,self.depth));
        }
        Ok(children)
    }
}
pub fn validate(bytes:&[u8],max_bytes:usize,max_depth:usize,max_nodes:usize)->Result<(&RawValue,usize,usize),String>{
    if bytes.len()>max_bytes {return Err("ResourceLimit bytes".into());}
    let state=State{nodes:Cell::new(0),limited:Cell::new(false),max_nodes,max_depth};
    Child{state:&state,depth:0}.charge::<serde_json::Error>().map_err(|e|format!("ResourceLimit {e}"))?;
    // One source-bounded lexical skip. This decoder is dropped before traversal.
    let root:&RawValue=serde_json::from_slice(bytes).map_err(|e|format!("InvalidInput {e}"))?;
    let mut pending=vec![(root,0usize)];let mut peak=1;
    while let Some((raw,depth))=pending.pop(){
        if matches!(raw.get().as_bytes()[0],b'['|b'{') {
            let children={
                use serde::Deserializer;
                let mut d=serde_json::Deserializer::from_str(raw.get());
                let children=d.deserialize_any(Child{state:&state,depth:depth+1})
                    .map_err(|e|format!("{} {e}",if state.limited.get(){"ResourceLimit"}else{"InvalidInput"}))?;
                d.end().map_err(|e|format!("InvalidInput {e}"))?;
                children
            }; // Container decoder scratch freed before processing children.
            pending.extend(children.into_iter().rev());peak=peak.max(pending.len());
        } else if raw.get().as_bytes()[0]==b'"' {
            use serde::Deserializer;
            let mut d=serde_json::Deserializer::from_str(raw.get());
            d.deserialize_str(ScalarString).map_err(|e|format!("InvalidInput {e}"))?;
            d.end().map_err(|e|format!("InvalidInput {e}"))?;
        }
    }
    Ok((root,state.nodes.get(),peak))
}
