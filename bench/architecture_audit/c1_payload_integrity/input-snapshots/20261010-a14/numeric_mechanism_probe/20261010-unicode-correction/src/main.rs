use serde::Deserialize;
use serde_json::{Value, value::RawValue};
use std::collections::BTreeMap;

#[derive(Debug)]
enum ValidationFailure { InvalidInput(String), ResourceLimit(String) }
struct ValidationLimits { json_bytes:u64, json_depth:u64, document_items:u64 }
impl Default for ValidationLimits {
    fn default()->Self { Self {json_bytes:8*1024*1024,json_depth:64,document_items:65536} }
}
#[path="original-json.rs"]
mod bounded;
mod raw_admission;

#[derive(Deserialize)]
struct Records<'a> {
    #[serde(borrow)]
    accessors:Vec<&'a RawValue>,
}
#[derive(Debug,PartialEq)]
enum ExactUnsigned { Value(u64), Overflow, Invalid }
// This is a finite candidate arithmetic probe, not production implementation.
// RawValue and the first bounded parse establish number grammar before entry.
fn exact(raw:&str)->ExactUnsigned {
    if !matches!(raw.as_bytes().first(),Some(b'0'..=b'9'|b'-')) {return ExactUnsigned::Invalid;}
    let negative=raw.starts_with('-');
    let raw=raw.strip_prefix('-').unwrap_or(raw);
    let (mantissa,exponent)=raw.split_once(['e','E']).unwrap_or((raw,"0"));
    let mut digits=0usize;let mut leading=0usize;let mut trailing=0usize;let mut nonzero=false;
    for b in mantissa.bytes().filter(|b|*b!=b'.') {
        digits+=1;
        if b==b'0' { if !nonzero {leading+=1;} trailing+=1; }
        else {nonzero=true;trailing=0;}
    }
    if !nonzero {return ExactUnsigned::Value(0);}
    if negative {return ExactUnsigned::Invalid;}
    let frac=mantissa.split_once('.').map_or(0,|(_,f)|f.len()) as i64;
    let exp_negative=exponent.starts_with('-');
    let exp=exponent.trim_start_matches(['-','+']);
    let ceiling=raw.len() as i64+32;
    let mut magnitude=0i64;
    for b in exp.bytes() {magnitude=(magnitude*10+i64::from(b-b'0')).min(ceiling);}
    let exponent=if exp_negative {-magnitude} else {magnitude};
    let scale=exponent-frac+trailing as i64;
    if scale<0 {return ExactUnsigned::Invalid;}
    let significant=digits-leading-trailing;
    if significant as i64+scale>20 {return ExactUnsigned::Overflow;}
    let mut value=0u64;
    for b in mantissa.bytes().filter(|b|*b!=b'.').skip(leading).take(significant) {
        value=match value.checked_mul(10).and_then(|v|v.checked_add(u64::from(b-b'0'))) {Some(v)=>v,None=>return ExactUnsigned::Overflow};
    }
    for _ in 0..scale {value=match value.checked_mul(10){Some(v)=>v,None=>return ExactUnsigned::Overflow};}
    ExactUnsigned::Value(value)
}
fn main() {
    let cases=[("3.0",ExactUnsigned::Value(3)),("3e0",ExactUnsigned::Value(3)),("300e-2",ExactUnsigned::Value(3)),("3.0000000000000001",ExactUnsigned::Invalid),("30000000000000001e-16",ExactUnsigned::Invalid),("9007199254740993.0",ExactUnsigned::Value(9007199254740993)),("18446744073709551615.0",ExactUnsigned::Value(u64::MAX)),("18446744073709551616.0",ExactUnsigned::Overflow),("18446744073709551614.9",ExactUnsigned::Invalid),("-0e999",ExactUnsigned::Value(0)),("-1.0",ExactUnsigned::Invalid),("1e-999",ExactUnsigned::Invalid),("1e100",ExactUnsigned::Overflow),("0.0000",ExactUnsigned::Value(0)),("34963e0",ExactUnsigned::Value(34963))];
    let mut rows=Vec::new();
    for (token,wanted) in cases {
        let bytes=format!(r#"{{"accessors":[{{"count":{token},"extras":{{"$serde_json::private::Number":"1"}}}}]}}"#);
        let document=bounded::parse(bytes.as_bytes()).unwrap();
        assert!(document["accessors"][0]["extras"].is_object());
        let borrowed:Records<'_>=serde_json::from_str(&bytes).unwrap();
        let record:BTreeMap<String,&RawValue>=serde_json::from_str(borrowed.accessors[0].get()).unwrap();
        let scalar=record["count"].get();
        let got=exact(scalar);assert_eq!(got,wanted,"{token}");
        let start=bytes.as_ptr() as usize;let end=start+bytes.len();let p=scalar.as_ptr() as usize;
        assert!(p>=start && p+scalar.len()<=end);
        rows.push(serde_json::json!({"token":token,"exact":format!("{got:?}"),"borrowed_source":true,"original_owned_number":document["accessors"][0]["count"]}));
    }
    for (name,input,resource) in [("duplicate",r#"{"a":1,"a":2}"#.to_owned(),false),("malformed","{\"a\":01}".to_owned(),false),("trailing","{} {}".to_owned(),false),("nonfinite","1e999".to_owned(),false),("depth65",format!("{}0{}","[".repeat(65),"]".repeat(65)),true),("deep100000",format!("{}0{}","[".repeat(100000),"]".repeat(100000)),true),("nodes65537",format!("[{}]",vec!["0";65536].join(",")),true)] {
        let error=bounded::parse(input.as_bytes()).unwrap_err();
        assert_eq!(matches!(error,ValidationFailure::ResourceLimit(_)),resource,"{name}: {error:?}");
        rows.push(serde_json::json!({"negative":name,"error":format!("{error:?}"),"borrowed_scan_performed":false}));
    }
    let depth64=format!("{}0{}","[".repeat(64),"]".repeat(64));assert!(bounded::parse(depth64.as_bytes()).is_ok());
    let nodes65536=format!("[{}]",vec!["0";65535].join(","));assert!(bounded::parse(nodes65536.as_bytes()).is_ok());
    let plain:Value=serde_json::from_str(r#"{"$serde_json::private::Number":"1"}"#).unwrap();assert!(plain.is_object());
    let mut raw_rows=Vec::new();
    for (token,wanted) in [("3.0",ExactUnsigned::Value(3)),("1e400",ExactUnsigned::Overflow),("1e-400",ExactUnsigned::Invalid),("-0e999",ExactUnsigned::Value(0)),("3.0000000000000001",ExactUnsigned::Invalid),("18446744073709551615.0",ExactUnsigned::Value(u64::MAX)),("18446744073709551616.0",ExactUnsigned::Overflow)] {
        let bytes=format!(r#"{{"accessors":[{{"count":{token},"extras":{{"$serde_json::private::Number":"1"}}}}]}}"#);
        let (root,nodes,peak)=raw_admission::validate(bytes.as_bytes(),8*1024*1024,64,65536).unwrap();
        let borrowed:Records<'_>=serde_json::from_str(root.get()).unwrap();
        let record:BTreeMap<String,&RawValue>=serde_json::from_str(borrowed.accessors[0].get()).unwrap();
        let scalar=record["count"].get();let got=exact(scalar);
        assert_eq!(got,wanted,"{token}");
        let source_start=bytes.as_ptr() as usize;let p=scalar.as_ptr() as usize;
        assert!(p>=source_start&&p+scalar.len()<=source_start+bytes.len());
        assert_eq!(nodes,6); // root, array, record, count, extras, magic-key string
        raw_rows.push(serde_json::json!({"token":token,"exact":format!("{got:?}"),"nodes":nodes,"pending_peak":peak,"borrowed_source":true}));
    }
    for scalar in ["true","false","null",r#""3""#,"[3]","{}"] {
        let (raw,_,_)=raw_admission::validate(scalar.as_bytes(),8*1024*1024,64,65536).unwrap();assert_eq!(exact(raw.get()),ExactUnsigned::Invalid);raw_rows.push(serde_json::json!({"non_number":scalar,"unsigned":"Invalid"}));
    }
    for (name,input,wanted) in [("duplicate",r#"{"a":1,"a":2}"#.to_owned(),"InvalidInput"),("escaped_duplicate",r#"{"a":1,"\u0061":2}"#.to_owned(),"InvalidInput"),("deepduplicate",r#"{"a":[{"b":1,"b":2}]}"#.to_owned(),"InvalidInput"),("duplicate_before_deep_admission",format!("{{\"a\":{}0{},\"a\":0}}","[".repeat(65),"]".repeat(65)),"InvalidInput"),("malformed","{\"a\":01}".to_owned(),"InvalidInput"),("trailing","{} {}".to_owned(),"InvalidInput"),("nan","NaN".to_owned(),"InvalidInput"),("bad_exponent","1e+".to_owned(),"InvalidInput"),("depth65",format!("{}0{}","[".repeat(65),"]".repeat(65)),"ResourceLimit"),("deep100000",format!("{}0{}","[".repeat(100000),"]".repeat(100000)),"ResourceLimit"),("nodes65537",format!("[{}]",vec!["0";65536].join(",")),"ResourceLimit")] {
        let error=raw_admission::validate(input.as_bytes(),8*1024*1024,64,65536).unwrap_err();assert!(error.starts_with(wanted),"{name}: {error}");raw_rows.push(serde_json::json!({"negative":name,"error":error}));
    }
    assert_eq!(raw_admission::validate(depth64.as_bytes(),8*1024*1024,64,65536).unwrap().1,65);
    assert_eq!(raw_admission::validate(nodes65536.as_bytes(),8*1024*1024,64,65536).unwrap().1,65536);
    assert!(raw_admission::validate(br#""abcdefgh""#,10,64,65536).is_ok());
    assert!(raw_admission::validate(br#""abcdefgh""#,9,64,65536).unwrap_err().starts_with("ResourceLimit"));
    for token in [r#""\uD800""#,r#""\uDC00""#,r#""\uD800\u0041""#] {
        let bytes=format!(r#"{{"extras":{token}}}"#);
        let error=raw_admission::validate(bytes.as_bytes(),8*1024*1024,64,65536).unwrap_err();
        assert!(error.starts_with("InvalidInput"));raw_rows.push(serde_json::json!({"invalid_scalar_string":token,"error":error}));
    }
    for token in [r#""\uD83D\uDE00""#,r#""\"\\\n\t\u0041""#] {
        let bytes=format!(r#"{{"extras":{token}}}"#);
        assert_eq!(raw_admission::validate(bytes.as_bytes(),8*1024*1024,64,65536).unwrap().1,2);
        raw_rows.push(serde_json::json!({"valid_scalar_string":token,"nodes":2}));
    }
    // Syntax/depth/node admission has succeeded first. A subsequent owned Value
    // failure is a representation refusal, not malformed source syntax.
    for token in ["1e400", "-1e400"] {
        let (raw,_,_)=raw_admission::validate(token.as_bytes(),8*1024*1024,64,65536).unwrap();
        assert!(serde_json::from_str::<Value>(raw.get()).is_err());
        raw_rows.push(serde_json::json!({"owned_adapter_token":token,"raw_grammar":"valid","owned_adapter":"Unsupported finite owned document representation"}));
    }
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"classification":"Actual pinned serde raw_value-only finite first-Value and iterative borrowed structural-admission exploratory mechanisms; no production acceptance","rows":rows,"raw_rows":raw_rows,"depth_equality":64,"node_equality":65536,"opaque_global_map_preserved":true,"no_arbitrary_precision_feature":true})).unwrap());
}
