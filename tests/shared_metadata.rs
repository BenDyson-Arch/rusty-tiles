use rusty_tiles::{
    glb_write::{write_glb, TilePrimitive},
    metadata::*,
    tile::{mesh_to_3tz, MeshTo3tzOptions},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Read};

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

fn view<'a>(doc: &Value, binary: &'a [u8], index: usize) -> &'a [u8] {
    let v = &doc["bufferViews"][index];
    let at = v["byteOffset"].as_u64().unwrap_or(0) as usize;
    &binary[at..at + v["byteLength"].as_u64().unwrap() as usize]
}

#[test]
fn node_features_survive_instancing_partition_and_parent_lods() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("buildings.glb");
    let prim = TilePrimitive {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
        indices: vec![0, 1, 2],
        ..Default::default()
    };
    let bytes = write_glb(&[prim]).unwrap();
    let mut glb = gltf::Glb::from_slice(&bytes).unwrap();
    let mut doc: Value = serde_json::from_slice(&glb.json).unwrap();
    doc["nodes"] = json!([
        {"mesh":0,"name":"Building 🦉","translation":[-3,0,0]},
        {"mesh":0,"name":"Building 🦉","translation":[3,0,0]},
        {"name":"Group","children":[3]},
        {"mesh":0,"translation":[0,0,3]}
    ]);
    doc["scenes"][0]["nodes"] = json!([0, 1, 2]);
    glb.json = std::borrow::Cow::Owned(serde_json::to_vec(&doc).unwrap());
    std::fs::write(&input, glb.to_vec().unwrap()).unwrap();
    assert!(rusty_tiles::mesh::load(&input)
        .unwrap()
        .triangle_nodes
        .is_empty());
    assert_eq!(
        rusty_tiles::mesh::load_with_node_features(&input)
            .unwrap()
            .triangle_nodes,
        [0, 1, 3]
    );
    for explicit in [false, true] {
        for meshopt in [false, true] {
            for budget in [1, 20_000] {
                let output = work
                    .path()
                    .join(format!("{explicit}-{meshopt}-{budget}.3tz"));
                mesh_to_3tz(
                    &input,
                    &output,
                    &MeshTo3tzOptions {
                        explicit,
                        meshopt,
                        node_features: true,
                        max_triangles: budget,
                        ..Default::default()
                    },
                )
                .unwrap();
                rusty_tiles::validate::archive(&output, None).unwrap();
                let mut archive =
                    zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
                let files: Vec<_> = archive
                    .file_names()
                    .filter(|s| s.ends_with(".glb"))
                    .map(str::to_owned)
                    .collect();
                if budget == 1 {
                    assert!(files.len() > 1);
                } else {
                    assert_eq!(files.len(), 1);
                }
                let mut seen = std::collections::BTreeSet::new();
                for file in files {
                    let mut bytes = Vec::new();
                    archive
                        .by_name(&file)
                        .unwrap()
                        .read_to_end(&mut bytes)
                        .unwrap();
                    let glb = gltf::Glb::from_slice(&bytes).unwrap();
                    let doc: Value = serde_json::from_slice(&glb.json).unwrap();
                    let binary = glb.bin.unwrap();
                    let table = &doc["extensions"][STRUCTURAL_METADATA]["propertyTables"][0];
                    let props = &table["properties"];
                    assert_eq!(
                        doc["bufferViews"]
                            [props["node_index"]["values"].as_u64().unwrap() as usize]
                            ["byteOffset"]
                            .as_u64()
                            .unwrap()
                            % 8,
                        0
                    );
                    let source_ids: Vec<_> = view(
                        &doc,
                        &binary,
                        props["node_index"]["values"].as_u64().unwrap() as usize,
                    )
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| u32::from_le_bytes(*b))
                    .collect();
                    let names = view(
                        &doc,
                        &binary,
                        props["name"]["values"].as_u64().unwrap() as usize,
                    );
                    let offsets: Vec<_> = view(
                        &doc,
                        &binary,
                        props["name"]["stringOffsets"].as_u64().unwrap() as usize,
                    )
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| u32::from_le_bytes(*b) as usize)
                    .collect();
                    for primitive in doc["meshes"][0]["primitives"].as_array().unwrap() {
                        let feature = &primitive["extensions"][MESH_FEATURES]["featureIds"][0];
                        assert_eq!(feature["featureCount"], table["count"]);
                        let accessor = &doc["accessors"]
                            [primitive["attributes"]["_FEATURE_ID_0"].as_u64().unwrap() as usize];
                        let index = accessor["bufferView"].as_u64().unwrap() as usize;
                        let packed =
                            &doc["bufferViews"][index]["extensions"]["EXT_meshopt_compression"];
                        let count = accessor["count"].as_u64().unwrap() as usize;
                        let data = if meshopt {
                            assert_eq!(packed["mode"], "ATTRIBUTES");
                            let start = packed["byteOffset"].as_u64().unwrap() as usize;
                            let len = packed["byteLength"].as_u64().unwrap() as usize;
                            meshopt::encoding::decode_vertex_buffer::<[u8; 4]>(
                                &binary[start..start + len],
                                count,
                            )
                            .unwrap()
                            .concat()
                        } else {
                            view(&doc, &binary, index).to_vec()
                        };
                        let ids: Vec<_> = data
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|b| u16::from_le_bytes(b[..2].try_into().unwrap()) as usize)
                            .collect();
                        assert_eq!(ids.len(), count);
                        assert!(ids.iter().all(|id| *id == ids[0]));
                        let id = ids[0];
                        let source = source_ids[id];
                        let name =
                            std::str::from_utf8(&names[offsets[id]..offsets[id + 1]]).unwrap();
                        assert_eq!(
                            name,
                            if source == 3 {
                                "node_3"
                            } else {
                                "Building 🦉"
                            }
                        );
                        seen.insert(source);
                    }
                }
                assert_eq!(seen, [0, 1, 3].into_iter().collect());
                if budget == 1 {
                    if let Some(dir) = std::env::var_os("RUSTY_TILES_METADATA_ACCEPTANCE_DIR") {
                        let dir = std::path::PathBuf::from(dir)
                            .join(format!("mesh-{explicit}-{meshopt}"));
                        std::fs::create_dir_all(&dir).unwrap();
                        archive.extract(dir).unwrap();
                    }
                }
            }
        }
    }
}
