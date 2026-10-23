use rusty_tiles::metadata::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[test]
fn public_api_attaches_lossless_tables_to_json_and_typed_gltf() {
    let mut builder = MetadataGlb::new("test");
    let rows = [
        BTreeMap::from([
            ("id".into(), json!(9_007_199_254_740_993i64)),
            ("name".into(), json!("楼 🦉")),
        ]),
        BTreeMap::from([("id".into(), json!(i64::MIN)), ("name".into(), Value::Null)]),
    ];
    encode_property_table(
        &mut builder,
        &rows.iter().collect::<Vec<_>>(),
        &BTreeMap::new(),
        "test",
        "building",
        "buildings",
    )
    .unwrap();
    let metadata: StructuralMetadata =
        serde_json::from_value(builder.document["extensions"][STRUCTURAL_METADATA].clone())
            .unwrap();
    assert_eq!(
        metadata.schema.as_ref().unwrap().classes["building"].properties["id"]
            .component_type
            .as_deref(),
        Some("INT64")
    );
    let (doc, binary) = builder.into_parts();
    let view = &doc["bufferViews"][metadata.property_tables[0].properties["id"].values];
    let at = view["byteOffset"].as_u64().unwrap() as usize;
    assert_eq!(
        i64::from_le_bytes(binary[at..at + 8].try_into().unwrap()),
        9_007_199_254_740_993
    );
    assert_eq!(
        i64::from_le_bytes(binary[at + 8..at + 16].try_into().unwrap()),
        i64::MIN
    );
    assert!(
        metadata.schema.as_ref().unwrap().classes["building"].properties["name"]
            .no_data
            .is_some()
    );
    let mut root = gltf_json::Root::default();
    metadata.attach_gltf(&mut root).unwrap();
    metadata.attach_gltf(&mut root).unwrap();
    assert_eq!(root.extensions_used, [STRUCTURAL_METADATA]);
    assert_eq!(
        serde_json::to_value(root).unwrap()["extensions"][STRUCTURAL_METADATA],
        doc["extensions"][STRUCTURAL_METADATA]
    );
    assert_eq!(
        tile_schema().classes["rustyTile"].properties["boundingBox"]
            .semantic
            .as_deref(),
        Some(TILE_BOUNDING_BOX)
    );
    assert_eq!(
        tile_schema().classes["rustyTile"].properties["geometricError"]
            .semantic
            .as_deref(),
        Some(TILE_GEOMETRIC_ERROR)
    );
}
