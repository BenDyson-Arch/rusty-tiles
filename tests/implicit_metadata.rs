use rusty_tiles::implicit::{
    expand_tileset, Coordinates, SubdivisionScheme, Subtree, TileMetadata,
};
use serde_json::{json, Value};

fn fixture() -> (Value, Vec<u8>) {
    let mut tree = Subtree::new(SubdivisionScheme::Quadtree, 3, 1).unwrap();
    // Insert in reverse order to distinguish available-tile rank from tile index.
    for (level, x, error) in [(2, 3, 0.), (1, 1, 4.), (0, 0, 8.)] {
        let c = Coordinates {
            level,
            x,
            y: 0,
            z: 0,
        };
        tree.set_tile(c, &[true]).unwrap();
        tree.set_metadata(
            c,
            TileMetadata {
                bounding_box: [x as f64, 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1.],
                geometric_error: error,
                extras: json!({"identity":format!("{level}:{x}")}),
            },
        )
        .unwrap();
    }
    let manifest = json!({"asset":{"version":"1.1"},"geometricError":16,
        "schema":{"id":"test","classes":{"rustyTile":{"properties":{
            "boundingBox":{"type":"SCALAR","componentType":"FLOAT64","array":true,"count":12,"semantic":"TILE_BOUNDING_BOX"},
            "geometricError":{"type":"SCALAR","componentType":"FLOAT64","semantic":"TILE_GEOMETRIC_ERROR"},"extras":{"type":"STRING"}}}}},
        "root":{"boundingVolume":{"box":[0,0,0,4,0,0,0,4,0,0,0,4]},"geometricError":8,
            "content":{"uri":"content/{level}-{x}-{y}.glb"},
            "implicitTiling":{"subdivisionScheme":"QUADTREE","subtreeLevels":3,"availableLevels":3,"subtrees":{"uri":"root.subtree"}}}});
    (manifest, tree.to_bytes().unwrap())
}

fn parts(bytes: &[u8]) -> (Value, Vec<u8>) {
    let length = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
    (
        serde_json::from_slice(&bytes[24..24 + length]).unwrap(),
        bytes[24 + length..].to_vec(),
    )
}

fn encode(doc: Value, binary: Vec<u8>) -> Vec<u8> {
    let mut json = serde_json::to_vec(&doc).unwrap();
    json.resize(json.len().next_multiple_of(8), b' ');
    let mut out = b"subt".to_vec();
    out.extend(1u32.to_le_bytes());
    out.extend((json.len() as u64).to_le_bytes());
    out.extend((binary.len() as u64).to_le_bytes());
    out.extend(json);
    out.extend(binary);
    out
}

#[test]
fn metadata_uses_available_morton_rank_and_preserves_measured_errors_and_extras() {
    let (manifest, bytes) = fixture();
    let (doc, binary) = parts(&bytes);
    let props = &doc["propertyTables"][0]["properties"];
    let view = &doc["bufferViews"][props["boundingBox"]["values"].as_u64().unwrap() as usize];
    let start = view["byteOffset"].as_u64().unwrap() as usize;
    for (row, expected) in [0., 1., 3.].into_iter().enumerate() {
        assert_eq!(
            f64::from_le_bytes(
                binary[start + row * 96..start + row * 96 + 8]
                    .try_into()
                    .unwrap()
            ),
            expected
        );
    }
    let expanded = expand_tileset(&manifest, |_| Ok(bytes.clone())).unwrap();
    let mut tile = &expanded["root"];
    for (level, x, error) in [(0, 0, 8.), (1, 1, 4.), (2, 3, 0.)] {
        assert_eq!(tile["geometricError"], error);
        assert_eq!(tile["boundingVolume"]["box"][0], x as f64);
        assert_eq!(tile["extras"]["identity"], format!("{level}:{x}"));
        assert_eq!(tile["content"]["uri"], format!("content/{level}-{x}-0.glb"));
        if level < 2 {
            tile = &tile["children"][0];
        }
    }
    assert!(tile.get("children").is_none());
}

#[test]
fn malformed_subtrees_and_metadata_fail_without_panics() {
    let (manifest, bytes) = fixture();
    for case in [
        "length",
        "count",
        "missingMetadata",
        "view",
        "schema",
        "error",
        "offsets",
        "bits",
        "levels",
        "content",
    ] {
        let mut manifest = manifest.clone();
        let (mut doc, mut binary) = parts(&bytes);
        match case {
            "length" => {
                binary.pop();
            }
            "count" => doc["tileAvailability"]["availableCount"] = json!(99),
            "missingMetadata" => {
                doc.as_object_mut().unwrap().remove("tileMetadata");
            }
            "view" => doc["bufferViews"][0]["byteOffset"] = json!(1),
            "schema" => {
                manifest["schema"]["classes"]["rustyTile"]["properties"]["boundingBox"]
                    ["semantic"] = json!("CONTENT_BOUNDING_BOX")
            }
            "error" => {
                let index = doc["propertyTables"][0]["properties"]["geometricError"]["values"]
                    .as_u64()
                    .unwrap() as usize;
                let at = doc["bufferViews"][index]["byteOffset"].as_u64().unwrap() as usize;
                binary[at..at + 8].copy_from_slice(&f64::NAN.to_le_bytes());
            }
            "offsets" => {
                doc["propertyTables"][0]["properties"]["extras"]["stringOffsetType"] =
                    json!("UINT64")
            }
            "bits" => {
                let index = doc["tileAvailability"]["bitstream"].as_u64().unwrap() as usize;
                let at = doc["bufferViews"][index]["byteOffset"].as_u64().unwrap() as usize;
                binary[at + 2] |= 0x80;
            }
            "levels" => manifest["root"]["implicitTiling"]["availableLevels"] = json!(1),
            "content" => doc["contentAvailability"] = json!([]),
            _ => unreachable!(),
        }
        let bytes = encode(doc, binary);
        assert!(
            expand_tileset(&manifest, |_| Ok(bytes.clone())).is_err(),
            "{case}"
        );
    }
    assert!(expand_tileset(&manifest, |_| Err(rusty_tiles::Error::Data(
        "missing subtree".into()
    )))
    .is_err());
}
