//! Ignored, external-fixture replay of omitted real parents. This exercises geometry
//! only: candidate assessment always succeeds, and no S2, encoding or appearance runs.
use super::{
    build_parent, merge, Contract, HashMap, InterfaceRow, SourcePrimitive, TileGeometry, TileKey,
};
use crate::mesh_large::hlod::decode;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn u32_at(v: &Value) -> u32 {
    u32::try_from(v.as_u64().unwrap()).unwrap()
}
fn vertex(v: &Value) -> [u32; 3] {
    std::array::from_fn(|i| u32_at(&v[i]))
}
fn tile_key(v: &Value) -> TileKey {
    TileKey {
        first: v["first"].as_u64().unwrap(),
        end: v["end"].as_u64().unwrap(),
        depth: u32_at(&v["depth"]),
        leaf_begin: u32_at(&v["leafBegin"]),
        leaf_end: u32_at(&v["leafEnd"]),
    }
}
fn interface(v: &Value) -> Vec<InterfaceRow> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            InterfaceRow::new(
                u32_at(&r["leaf"]),
                vertex(&r["a"]),
                vertex(&r["b"]),
                u32_at(&r["lo"]),
                u32_at(&r["hi"]),
            )
        })
        .collect()
}
fn contract(v: &Value) -> Contract {
    use crate::mesh_large::hlod::borders::Certificate;
    let certificates = v["certificates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let epsilon = c["epsilon"].as_f64().unwrap();
            assert_eq!(
                epsilon, 0.,
                "mapping None is only valid for these exact real borders"
            );
            Certificate {
                endpoints: [vertex(&c["endpoints"][0]), vertex(&c["endpoints"][1])],
                chain: [vertex(&c["chain"][0]), vertex(&c["chain"][1])],
                origin: vertex(&c["origin"]),
                first: c["first"].as_u64().unwrap(),
                end: c["end"].as_u64().unwrap(),
                epsilon,
                terminal_lock: c["terminalLock"].as_bool().unwrap(),
            }
        })
        .collect();
    Contract {
        depth: u32_at(&v["depth"]),
        interface: interface(&v["interface"]),
        certificates,
        terminal_vertices: v["terminalVertices"]
            .as_array()
            .unwrap()
            .iter()
            .map(vertex)
            .collect(),
        scale: v["scale"].as_f64().unwrap(),
    }
}
fn child(dir: &Path, v: &Value) -> TileGeometry {
    let (positions, triangles) = if let Some(p) = v.get("positions") {
        (
            serde_json::from_value::<Vec<[f32; 3]>>(p.clone()).unwrap(),
            serde_json::from_value::<Vec<[u32; 3]>>(v["triangles"].clone()).unwrap(),
        )
    } else {
        let bytes = fs::read(dir.join(v["file"].as_str().unwrap())).unwrap();
        assert_eq!(bytes.len() as u64, v["bytes"].as_u64().unwrap());
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            v["sha256"].as_str().unwrap()
        );
        let decoded = decode::decode(&bytes).unwrap();
        let mut ids = HashMap::default();
        let mut positions = Vec::new();
        let mut triangles = Vec::with_capacity(decoded.faces.len());
        for face in decoded.faces {
            triangles.push(face.p.map(|p| {
                let next = positions.len() as u32;
                let id = *ids.entry(super::key(p)).or_insert(next);
                if id == next {
                    positions.push(p);
                }
                id
            }));
        }
        (positions, triangles)
    };
    assert!(!triangles.is_empty());
    let key = tile_key(&v["key"]);
    TileGeometry {
        key,
        ells: if v["ells"].is_array() {
            serde_json::from_value(v["ells"].clone()).unwrap()
        } else {
            vec![u8::try_from(u32_at(&v["ells"])).unwrap(); positions.len()]
        },
        positions,
        representative: vec![0; triangles.len()],
        triangles,
        primitives: vec![SourcePrimitive {
            node: 0,
            mesh: 0,
            primitive: 0,
            expanded_ordinal: 0,
            material: u32::MAX,
        }],
        interface: interface(&v["interface"]),
        geometric_error: v["geometricError"].as_f64().unwrap(),
        lease: None,
    }
}
fn replay(path: &Path) {
    let dir = path.parent().unwrap();
    let input_bytes = fs::read(path).unwrap();
    let hashes: Value =
        serde_json::from_slice(&fs::read(dir.join("hashes.json")).unwrap()).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&input_bytes)),
        hashes["input.json"].as_str().unwrap()
    );
    let input: Value = serde_json::from_slice(&input_bytes).unwrap();
    assert_eq!(input["schemaVersion"], 1);
    let key = tile_key(&input["parentKey"]);
    let contract = contract(&input["contract"]);
    assert_eq!(contract.depth, key.depth);
    let children: Vec<_> = input["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| child(dir, v))
        .collect();
    assert!(children.len() <= 8);
    let caps: Vec<_> = input["caps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| usize::try_from(v.as_u64().unwrap()).unwrap())
        .collect();
    assert_eq!(caps, [8192]);
    let merged = merge(&children);
    let (locked, loops) = super::topology::protected(&merged, &contract.interface, true);
    eprintln!("fold replay input={} parent={key:?} children={} merged_vertices={} merged_faces={} interface={} initial_boundary_pins={} boundary_loops={} caps={caps:?}",
        path.display(), children.len(), merged.positions.len(), merged.triangles.len(),
        contract.interface.len(), locked.iter().filter(|&&v| v).count(), loops);
    drop(merged);
    let mut assessed = 0;
    let outcome = build_parent(key, &children, &caps, &contract, &|_| Ok(None), &mut |built| {
        assessed += 1;
        eprintln!("fold replay assessed={assessed} faces={} protected={} retries={} stopping={:?} topology={:?}",
            built.geometry.triangles.len(), built.protected_faces, built.retries, built.stopping, built.topology);
        Ok(std::env::var_os("RT_HLOD_FOLD_REJECT_HOLES").is_none()
            || (built.change.receipt.closed_holes == 0 && built.change.receipt.opened_holes == 0))
    }, &|| Ok(())).unwrap();
    if let Some(built) = outcome.built {
        eprintln!("fold replay result parent={key:?} omitted=false output={} protected={} retries={} assessed={assessed}",
            built.geometry.triangles.len(), built.protected_faces, built.retries);
    } else {
        eprintln!("fold replay result parent={key:?} omitted=true assessed={assessed} attempted={:?} construction_dropped_faces={}",
            outcome.attempted_topology, outcome.construction_dropped.len());
        panic!("the isolated parent must fit without omissions");
    }
}

#[test]
#[ignore = "external immutable real-parent fixture; set RT_HLOD_FOLD_REPLAY"]
fn replay_failed_real_parent() {
    let path = std::path::PathBuf::from(
        std::env::var_os("RT_HLOD_FOLD_REPLAY")
            .expect("set RT_HLOD_FOLD_REPLAY to input.json, a case directory, or the replay root"),
    );
    if path.is_file() {
        replay(&path);
    } else if path.join("input.json").is_file() {
        replay(&path.join("input.json"));
    } else {
        let mut cases: Vec<_> = fs::read_dir(&path)
            .unwrap()
            .map(|e| e.unwrap().path().join("input.json"))
            .filter(|p| p.is_file())
            .collect();
        cases.sort();
        assert!(
            !cases.is_empty(),
            "no replay cases under {}",
            path.display()
        );
        for case in cases {
            replay(&case);
        }
    }
}

/// Capture exact held order in a bounded test log, before appearance reorders GLB
/// faces. No production branch or scratch file is added; the evidence runner owns
/// its log. Only one selected eight-child parent can be emitted per bake.
pub(super) fn capture(
    tile: TileKey,
    children: &[TileGeometry],
    contract: &Contract,
    caps: &[usize],
) {
    use std::io::Write;
    if tile.first != 1 || tile.depth != 4 || std::env::var_os("RT_HLOD_FOLD_CAPTURE").is_none() {
        return;
    }
    let children: Vec<_> = children
        .iter()
        .map(|c| {
            serde_json::json!({
                "key": c.key, "positions": c.positions, "triangles": c.triangles,
                "ells": c.ells, "interface": c.interface, "geometricError": c.geometric_error
            })
        })
        .collect();
    let record = serde_json::json!({"schemaVersion": 1, "parentKey": tile,
        "contract": contract, "caps": caps.iter().copied().collect::<std::collections::BTreeSet<_>>(),
        "children": children});
    let mut stderr = std::io::stderr().lock();
    write!(stderr, "FOLD_INPUT ").unwrap();
    serde_json::to_writer(&mut stderr, &record).unwrap();
    writeln!(stderr).unwrap();
}
