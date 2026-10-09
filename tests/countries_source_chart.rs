//! Original country rings must produce fills in both vector backends.
//! The Sudan and Antarctica fixture rings are unchanged extracts from Natural
//! Earth's public-domain 1:110m Admin 0 Countries dataset, pinned in
//! `bench/demodata_manifest.json` (ne_110m_admin_0_countries.zip).
use serde_json::Value;
use std::{collections::BTreeSet, fs, io::Read, path::Path, process::Command};

fn view<'a>(doc: &Value, bin: &'a [u8], index: &Value) -> &'a [u8] {
    let v = &doc["bufferViews"][index.as_u64().unwrap() as usize];
    let start = v["byteOffset"].as_u64().unwrap_or(0) as usize;
    &bin[start..start + v["byteLength"].as_u64().unwrap() as usize]
}
fn ids(doc: &Value, bin: &[u8], table: usize) -> Vec<String> {
    let t = &doc["extensions"]["EXT_structural_metadata"]["propertyTables"][table];
    let col = &t["properties"]["_source_id"];
    let values = view(doc, bin, &col["values"]);
    let offsets = view(doc, bin, &col["stringOffsets"]);
    (0..t["count"].as_u64().unwrap() as usize)
        .map(|i| {
            let a = u32::from_le_bytes(offsets[i * 4..i * 4 + 4].try_into().unwrap()) as usize;
            let b = u32::from_le_bytes(offsets[i * 4 + 4..i * 4 + 8].try_into().unwrap()) as usize;
            std::str::from_utf8(&values[a..b]).unwrap().to_owned()
        })
        .collect()
}
fn filled_ids(archive: &Path) -> BTreeSet<String> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let mut filled = BTreeSet::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        if !entry.name().ends_with(".glb") {
            continue;
        }
        let mut bytes = vec![];
        entry.read_to_end(&mut bytes).unwrap();
        let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let doc: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
        let bin = &bytes[28 + length..];
        for mesh in doc["meshes"].as_array().unwrap() {
            for p in mesh["primitives"].as_array().unwrap() {
                if p["mode"].as_u64().unwrap_or(4) != 4 {
                    continue;
                }
                let feature = &p["extensions"]["EXT_mesh_features"]["featureIds"][0];
                let rows = ids(
                    &doc,
                    bin,
                    feature["propertyTable"].as_u64().unwrap() as usize,
                );
                let accessor =
                    &doc["accessors"][p["attributes"]["_FEATURE_ID_0"].as_u64().unwrap() as usize];
                let data = view(&doc, bin, &accessor["bufferView"]);
                let offset = accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
                let component = accessor["componentType"].as_u64().unwrap();
                let size = match component {
                    5121 => 1,
                    5123 => 2,
                    5125 => 4,
                    _ => panic!("feature component {component}"),
                };
                for n in 0..accessor["count"].as_u64().unwrap() as usize {
                    let stride = doc["bufferViews"]
                        [accessor["bufferView"].as_u64().unwrap() as usize]["byteStride"]
                        .as_u64()
                        .unwrap_or(size as u64) as usize;
                    let data = &data[offset + n * stride..offset + n * stride + size];
                    let row = match size {
                        1 => data[0] as usize,
                        2 => u16::from_le_bytes(data.try_into().unwrap()) as usize,
                        4 => u32::from_le_bytes(data.try_into().unwrap()) as usize,
                        _ => unreachable!(),
                    };
                    filled.insert(rows[row].clone());
                }
            }
        }
    }
    filled
}
#[test]
fn countries_emit_filled_ids_without_skip_invalid_even_when_fragmented() {
    let temp = tempfile::tempdir().unwrap();
    let input =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/countries-source.geojson");
    for (name, args) in [
        ("normal", vec![]),
        ("fragmented", vec!["--maxVertices", "256"]),
        ("explicit", vec!["--explicit"]),
    ] {
        let archive = temp.path().join(format!("{name}.3tz"));
        let output = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
            .args(["vector", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(&archive)
            .args(["--json", "--reproducible"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let mut zip = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
        let report: Value =
            serde_json::from_reader(zip.by_name("conversion.json").unwrap()).unwrap();
        assert_eq!(report["features"], 2, "{name}: {report}");
        assert_eq!(report["skippedFeatures"], 0, "{name}: {report}");
        if name == "fragmented" {
            assert!(report["fragmentedPolygons"].as_u64().unwrap() > 0);
        }
        assert_eq!(
            filled_ids(&archive),
            BTreeSet::from(["15".into(), "160".into()])
        );
        assert_eq!(
            rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(&archive))
                .unwrap()
                .ok,
            true
        );
    }
}
