//! H17 acceptance tests: real H8 runs on small fixtures, published through H16, checked by
//! an INDEPENDENT auditor (its own GLB decoder, brute-force sampled distances against the
//! fixture's exact source, frontier seams on decoded bytes, identity accounting), plus the
//! controls that must fail.
use super::{geom::Bvh, Mutation};
use crate::{
    content_integrity::meshopt::{self as codec, Mode},
    mesh_large::{
        delivery::{mesh_leaves_to_3tz, MeshTilesetRequest},
        leaves::producer::{mesh_to_leaf_payloads, LeafOutputRequest, LeafProfile},
    },
    validate::{inspect, ValidationRequest},
    JobErrorKind, RunControl,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

// ---------------------------------------------------------------------------------------
// Fixtures.

#[derive(Clone)]
struct Mesh {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    color: [f64; 4],
}

fn write_gltf(dir: &Path, meshes: &[Mesh]) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let mut bin = Vec::new();
    let (mut views, mut accessors, mut gltf_meshes, mut nodes, mut materials) =
        (vec![], vec![], vec![], vec![], vec![]);
    for (i, m) in meshes.iter().enumerate() {
        let start = bin.len();
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in &m.positions {
            for a in 0..3 {
                bin.extend(p[a].to_le_bytes());
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bin.len()-start}));
        accessors.push(json!({"bufferView":views.len()-1,"componentType":5126,"count":m.positions.len(),"type":"VEC3","min":lo,"max":hi}));
        let start = bin.len();
        for v in &m.indices {
            bin.extend(v.to_le_bytes());
        }
        views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bin.len()-start}));
        accessors.push(json!({"bufferView":views.len()-1,"componentType":5125,"count":m.indices.len(),"type":"SCALAR"}));
        gltf_meshes.push(json!({"primitives":[{"attributes":{"POSITION":accessors.len()-2},"indices":accessors.len()-1,"material":i}]}));
        nodes.push(json!({"mesh":i}));
        materials
            .push(json!({"pbrMetallicRoughness":{"baseColorFactor":m.color,"metallicFactor":0.0}}));
    }
    fs::write(dir.join("scene.bin"), &bin).unwrap();
    let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":(0..meshes.len()).collect::<Vec<_>>()}],
        "nodes":nodes,"meshes":gltf_meshes,"materials":materials,
        "buffers":[{"uri":"scene.bin","byteLength":bin.len()}],"bufferViews":views,"accessors":accessors});
    let path = dir.join("scene.gltf");
    fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    path
}

/// The fixture's surface. Platform `sin`/`cos` (libm) may differ by an ulp between
/// targets, which would change the INPUT bytes; this uses only IEEE `+ - * /` and `round`, so
/// the fixture (and the pinned output digest) is identical on every platform.
fn height(x: f32, z: f32) -> f32 {
    (3. * portable_sin(f64::from(x) / 7.) * portable_sin(f64::from(z) / 9. + FRAC_PI_2)) as f32
}

/// `sin` by range reduction to `[-pi, pi]` and a Taylor series to `x^25` (error below
/// 1e-15 there); exactly reproducible across platforms.
fn portable_sin(x: f64) -> f64 {
    let tau = 2. * std::f64::consts::PI;
    let r = x - (x / tau).round() * tau;
    let r2 = r * r;
    let (mut term, mut sum) = (r, r);
    for n in 1..13u32 {
        let k = f64::from(2 * n);
        term *= -r2 / (k * (k + 1.));
        sum += term;
    }
    sum
}
const FRAC_PI_2: f64 = std::f64::consts::FRAC_PI_2;

/// A wavy terrain over three primitives with shared seams, small interior holes (one
/// quad each, fillable), a larger hole, small islands floating 0.3 m above the surface, and
/// one island 3 m above it (vanishes at the root, where it is within the tile's spacing;
/// its distance must be certified).
fn terrain(g: usize) -> Vec<Mesh> {
    let step = 100. / g as f32;
    let holes: BTreeSet<(usize, usize)> = {
        let mut h = BTreeSet::new();
        for k in 0..12 {
            h.insert((7 + 13 * k % (g - 14), 9 + 29 * k % (g - 18)));
        }
        for i in 0..4 {
            for j in 0..4 {
                h.insert((g / 2 + i, g / 3 + j));
            }
        }
        h
    };
    let bands = [0, g * 2 / 5, g * 7 / 10, g];
    let colors = [
        [0.6, 0.5, 0.4, 1.],
        [0.3, 0.6, 0.3, 1.],
        [0.5, 0.5, 0.6, 1.],
    ];
    let mut meshes = Vec::new();
    for b in 0..3 {
        let mut m = Mesh {
            positions: vec![],
            indices: vec![],
            color: colors[b],
        };
        let (i0, i1) = (bands[b], bands[b + 1]);
        let w = g + 1;
        for i in i0..=i1 {
            for j in 0..=g {
                let (x, z) = (i as f32 * step, j as f32 * step);
                m.positions.push([x, height(x, z), z]);
            }
        }
        for i in i0..i1 {
            for j in 0..g {
                if holes.contains(&(i, j)) {
                    continue;
                }
                let v = |a: usize, c: usize| ((a - i0) * w + c) as u32;
                m.indices.extend([v(i, j), v(i, j + 1), v(i + 1, j)]);
                m.indices
                    .extend([v(i + 1, j), v(i, j + 1), v(i + 1, j + 1)]);
            }
        }
        meshes.push(m);
    }
    let mut islands = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.9, 0.1, 0.1, 1.],
    };
    let mut island = |x: f32, z: f32, lift: f32, size: f32| {
        let y = height(x, z) + lift;
        let base = islands.positions.len() as u32;
        islands.positions.extend([
            [x, y, z],
            [x + size, y, z],
            [x, y, z + size],
            [x + size, y, z + size],
        ]);
        islands
            .indices
            .extend([base, base + 2, base + 1, base + 1, base + 2, base + 3]);
    };
    for k in 0..40 {
        island(
            3. + (k as f32 * 37.) % 94.,
            5. + (k as f32 * 53.) % 90.,
            0.3,
            0.2,
        );
    }
    island(61.3, 27.9, 3., 0.2);
    meshes.push(islands);
    meshes
}

/// A dense micro-cluster (synthetic run depths > 21) inside a large sparse frame.
fn cluster() -> Vec<Mesh> {
    let mut dense = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.4, 0.4, 0.8, 1.],
    };
    let n = 200usize;
    for i in 0..=n {
        for j in 0..=n {
            let (x, z) = (i as f32 * 5e-7, j as f32 * 5e-7);
            dense.positions.push([x, 1e-7 * ((i * j) % 7) as f32, z]);
        }
    }
    for i in 0..n {
        for j in 0..n {
            let v = |a: usize, c: usize| (a * (n + 1) + c) as u32;
            dense.indices.extend([v(i, j), v(i, j + 1), v(i + 1, j)]);
            dense
                .indices
                .extend([v(i + 1, j), v(i, j + 1), v(i + 1, j + 1)]);
        }
    }
    let frame = Mesh {
        positions: vec![
            [-500., -1., -500.],
            [500., -1., -500.],
            [-500., 1., 500.],
            [500., 2., 500.],
        ],
        indices: vec![0, 2, 1, 1, 2, 3],
        color: [0.5, 0.5, 0.5, 1.],
    };
    vec![dense, frame]
}

// ---------------------------------------------------------------------------------------
// Independent GLB decoding (the auditor does not use the producer's decoder).

#[derive(Default)]
struct Glb {
    faces: Vec<[[f32; 3]; 3]>,
    features: Vec<[u32; 2]>,
    /// Property table of each face's second feature-ID set.
    second_table: Vec<usize>,
    tables: Vec<(String, BTreeMap<String, Vec<u64>>)>,
    /// glTF primitives over all scene meshes, and declared materials.
    gltf_primitives: usize,
    materials: usize,
}

fn read_glb(bytes: &[u8]) -> Glb {
    let u = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    assert_eq!(&bytes[..4], b"glTF");
    let jl = u(12);
    let doc: Value = serde_json::from_slice(&bytes[20..20 + jl]).unwrap();
    let bin = &bytes[28 + jl..28 + jl + u(20 + jl)];
    let view = |i: usize| -> Vec<u8> {
        let v = &doc["bufferViews"][i];
        match v["extensions"].get("EXT_meshopt_compression") {
            Some(e) => {
                let s = e["byteOffset"].as_u64().unwrap_or(0) as usize;
                let mode = match e["mode"].as_str().unwrap() {
                    "ATTRIBUTES" => Mode::Attributes,
                    _ => Mode::Triangles,
                };
                codec::decode(
                    &bin[s..s + e["byteLength"].as_u64().unwrap() as usize],
                    e["count"].as_u64().unwrap() as usize,
                    e["byteStride"].as_u64().unwrap() as usize,
                    mode,
                )
                .unwrap()
            }
            None => {
                let s = v["byteOffset"].as_u64().unwrap_or(0) as usize;
                bin[s..s + v["byteLength"].as_u64().unwrap() as usize].to_vec()
            }
        }
    };
    let elements = |a: usize| -> (Vec<Vec<u8>>, u64) {
        let acc = &doc["accessors"][a];
        let n = acc["count"].as_u64().unwrap() as usize;
        let ct = acc["componentType"].as_u64().unwrap();
        let size = if ct == 5123 { 2 } else { 4 };
        let width = match acc["type"].as_str().unwrap() {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            _ => 4,
        };
        let vi = acc["bufferView"].as_u64().unwrap() as usize;
        let data = view(vi);
        let stride = doc["bufferViews"][vi]["byteStride"]
            .as_u64()
            .map_or(size * width, |s| s as usize);
        (
            (0..n)
                .map(|i| data[i * stride..i * stride + size * width].to_vec())
                .collect(),
            ct,
        )
    };
    let mut out = Glb::default();
    for root in doc["scenes"][0]["nodes"].as_array().unwrap() {
        let node = &doc["nodes"][root.as_u64().unwrap() as usize];
        let t: Vec<f32> = node.get("translation").map_or(vec![0.; 3], |v| {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap() as f32)
                .collect()
        });
        let s: Vec<f32> = node.get("scale").map_or(vec![1.; 3], |v| {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap() as f32)
                .collect()
        });
        for prim in doc["meshes"][node["mesh"].as_u64().unwrap() as usize]["primitives"]
            .as_array()
            .unwrap()
        {
            out.gltf_primitives += 1;
            let (pos, ct) = elements(prim["attributes"]["POSITION"].as_u64().unwrap() as usize);
            let points: Vec<[f32; 3]> = pos
                .iter()
                .map(|e| {
                    std::array::from_fn(|a| {
                        let raw = if ct == 5123 {
                            f32::from(u16::from_le_bytes([e[2 * a], e[2 * a + 1]]))
                        } else {
                            f32::from_le_bytes(e[4 * a..4 * a + 4].try_into().unwrap())
                        };
                        let p = raw * s[a] + t[a];
                        if p == 0. {
                            0.
                        } else {
                            p
                        }
                    })
                })
                .collect();
            let feature = |name: &str| -> Vec<u32> {
                prim["attributes"]
                    .get(name)
                    .map_or(vec![u32::MAX; points.len()], |a| {
                        elements(a.as_u64().unwrap() as usize)
                            .0
                            .iter()
                            .map(|e| f32::from_le_bytes(e[..4].try_into().unwrap()) as u32)
                            .collect()
                    })
            };
            let (f0, f1) = (feature("_FEATURE_ID_0"), feature("_FEATURE_ID_1"));
            let second = prim["extensions"]["EXT_mesh_features"]["featureIds"][1]["propertyTable"]
                .as_u64()
                .map_or(usize::MAX, |t| t as usize);
            let index: Vec<usize> = match prim.get("indices") {
                Some(i) => {
                    let (e, ct) = elements(i.as_u64().unwrap() as usize);
                    e.iter()
                        .map(|b| {
                            if ct == 5123 {
                                usize::from(u16::from_le_bytes([b[0], b[1]]))
                            } else {
                                u32::from_le_bytes(b[..4].try_into().unwrap()) as usize
                            }
                        })
                        .collect()
                }
                None => (0..points.len()).collect(),
            };
            for t in index.chunks(3) {
                out.faces.push([points[t[0]], points[t[1]], points[t[2]]]);
                out.features.push([f0[t[0]], f1[t[0]]]);
                out.second_table.push(second);
            }
        }
    }
    out.materials = doc["materials"].as_array().map_or(0, Vec::len);
    let meta = &doc["extensions"]["EXT_structural_metadata"];
    for table in meta["propertyTables"].as_array().unwrap() {
        let class = table["class"].as_str().unwrap().to_owned();
        let count = table["count"].as_u64().unwrap() as usize;
        let mut columns = BTreeMap::new();
        for (name, p) in table["properties"].as_object().unwrap() {
            let ct = meta["schema"]["classes"][&class]["properties"][name]["componentType"]
                .as_str()
                .unwrap();
            let w = match ct {
                "UINT8" => 1,
                "UINT32" => 4,
                _ => 8,
            };
            let data = view(p["values"].as_u64().unwrap() as usize);
            let values = (0..count)
                .map(|i| {
                    let mut b = [0u8; 8];
                    b[..w].copy_from_slice(&data[i * w..i * w + w]);
                    u64::from_le_bytes(b)
                })
                .collect();
            columns.insert(name.clone(), values);
        }
        out.tables.push((class, columns));
    }
    out
}

fn rows(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

// ---------------------------------------------------------------------------------------
// Runs.

fn leaves(
    input: &Path,
    out: &Path,
    jobs: usize,
    mutation: Mutation,
    target: usize,
) -> Result<PathBuf, crate::JobFailure> {
    let scratch = out.with_extension("scratch");
    fs::create_dir_all(&scratch).unwrap();
    mesh_to_leaf_payloads(
        LeafOutputRequest::new(input, out, LeafProfile::Balanced)
            .with_jobs(NonZeroUsize::new(jobs).unwrap())
            .with_scratch_dir(&scratch)
            .with_lod_mutation(mutation)
            .with_lod_target(target),
        &RunControl::default(),
    )
    .map(|r| r.output)
}

fn archive(
    leaves: &Path,
    output: &Path,
) -> (
    crate::mesh_large::delivery::MeshTilesetReport,
    BTreeMap<String, Vec<u8>>,
) {
    let result = mesh_leaves_to_3tz(
        MeshTilesetRequest::new(leaves, output),
        &RunControl::default(),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    let mut members = BTreeMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        members.insert(entry.name().to_owned(), bytes);
    }
    (result.report, members)
}

// ---------------------------------------------------------------------------------------
// The auditor.

type Tri = [[f64; 3]; 3];
fn f64s(t: &[[f32; 3]; 3]) -> Tri {
    t.map(|p| p.map(f64::from))
}
fn samples(t: &Tri, k: usize) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    for i in 0..=k {
        for j in 0..=k - i {
            let (u, v) = (i as f64 / k as f64, j as f64 / k as f64);
            out.push(std::array::from_fn(|a| {
                t[0][a] + u * (t[1][a] - t[0][a]) + v * (t[2][a] - t[0][a])
            }));
        }
    }
    out
}

#[derive(Debug, Default)]
struct Audit {
    proxies: usize,
    worst_ratio: f64,
    worst_gap: f64,
    cracks: usize,
    boundary_edges: usize,
    matched_edges: usize,
    fills: usize,
    vanished: usize,
    fill_faces: usize,
    /// Fill-face corners on a seam vertex of their depth (the border-hole rule forbids).
    seam_fill_vertices: usize,
    max_depth: u64,
}

fn welded(p: [f32; 3]) -> [u32; 3] {
    p.map(|x| if x == 0. { 0 } else { x.to_bits() })
}

/// Everything checkable from the leaf directory against the exact source.
fn audit(source: &[Mesh], dir: &Path) -> Audit {
    let mut audit = Audit::default();
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let steps: Vec<f64> = manifest["lattice"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let leaf_rows = rows(&dir.join("leaves.jsonl"));
    let proxy_rows = rows(&dir.join("proxies.jsonl"));
    audit.proxies = proxy_rows.len();
    let source_tri = |mesh: usize, t: usize| -> Tri {
        let m = &source[mesh];
        std::array::from_fn(|c| m.positions[m.indices[3 * t + c] as usize].map(f64::from))
    };
    // Source boundary vertices (welded across meshes).
    let mut edge_count: BTreeMap<([u32; 3], [u32; 3]), u32> = BTreeMap::new();
    for m in source {
        for t in m.indices.chunks(3) {
            for k in 0..3 {
                let (a, b) = (
                    welded(m.positions[t[k] as usize]),
                    welded(m.positions[t[(k + 1) % 3] as usize]),
                );
                *edge_count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
    }
    let boundary: Vec<[f64; 3]> = edge_count
        .iter()
        .filter(|(_, c)| **c != 2)
        .flat_map(|((a, b), _)| [*a, *b])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|k| k.map(|b| f64::from(f32::from_bits(b))))
        .collect();
    // Leaves: decoded faces, source triangles, present primitives.
    struct Tile {
        first: u64,
        end: u64,
        depth: u64,
        error: f64,
        faces: Vec<[[f32; 3]; 3]>,
        source: Vec<Tri>,
        provenance: Vec<(u64, u64)>,
        present: BTreeSet<(u64, u64, u64)>,
        vanished: Vec<(u64, u64, u64)>,
        fills: Vec<([u64; 3], f64)>,
        /// Decoded fill faces (proxies).
        filled: Vec<[[f32; 3]; 3]>,
    }
    let primitives = |g: &Glb| -> BTreeSet<(u64, u64, u64)> {
        let t = &g
            .tables
            .iter()
            .find(|t| t.0 == "source_primitive")
            .unwrap()
            .1;
        (0..t["node_index"].len())
            .map(|i| {
                (
                    t["node_index"][i],
                    t["mesh_index"][i],
                    t["primitive_index"][i],
                )
            })
            .collect()
    };
    let mut tiles: Vec<Tile> = Vec::new();
    for r in &leaf_rows {
        let g = read_glb(&fs::read(dir.join(r["uri"].as_str().unwrap())).unwrap());
        let t = &g
            .tables
            .iter()
            .find(|t| t.0 == "source_triangle")
            .unwrap()
            .1;
        let source_tris = g
            .features
            .iter()
            .map(|f| {
                source_tri(
                    t["mesh_index"][f[1] as usize] as usize,
                    t["triangle_index"][f[1] as usize] as usize,
                )
            })
            .collect();
        let provenance = g
            .features
            .iter()
            .map(|f| {
                (
                    t["mesh_index"][f[1] as usize],
                    t["triangle_index"][f[1] as usize],
                )
            })
            .collect();
        tiles.push(Tile {
            first: r["first"].as_u64().unwrap(),
            end: r["end"].as_u64().unwrap(),
            depth: r["depth"].as_u64().unwrap(),
            error: r["geometryError"].as_f64().unwrap(),
            present: primitives(&g),
            faces: g.faces,
            source: source_tris,
            provenance,
            vanished: vec![],
            fills: vec![],
            filled: vec![],
        });
    }
    let leaf_count = tiles.len();
    for r in &proxy_rows {
        let g = read_glb(&fs::read(dir.join(r["uri"].as_str().unwrap())).unwrap());
        let (first, end) = (r["first"].as_u64().unwrap(), r["end"].as_u64().unwrap());
        let source: Vec<Tri> = tiles[..leaf_count]
            .iter()
            .filter(|l| l.first >= first && l.end <= end)
            .flat_map(|l| l.source.iter().copied())
            .collect();
        // Provenance: every proxy triangle names a source triangle of its own primitive in
        // the tile's range, or is a fill.
        let in_range: BTreeSet<(u64, u64)> = tiles[..leaf_count]
            .iter()
            .filter(|l| l.first >= first && l.end <= end)
            .flat_map(|l| l.provenance.iter().copied())
            .collect();
        let present_rows = &g
            .tables
            .iter()
            .find(|t| t.0 == "source_primitive")
            .unwrap()
            .1;
        if g.faces.len() <= 8192 {
            // One provenance table per proxy (CesiumJS picks through one table per model)
            // and one glTF primitive per distinct flat material.
            assert_eq!(g.tables.iter().filter(|t| t.0 == "proxy_source").count(), 1);
            assert_eq!(
                g.gltf_primitives, g.materials,
                "one glTF primitive per material"
            );
        }
        let mut filled = vec![];
        for ((f, table), face) in g.features.iter().zip(&g.second_table).zip(&g.faces) {
            let (class, t) = &g.tables[*table];
            assert_eq!(class, "proxy_source");
            let row = f[1] as usize;
            assert!(!t.contains_key("mesh_index"), "slim provenance");
            let mesh = present_rows["mesh_index"][f[0] as usize];
            if t["fill"][row] == 0 {
                assert!(
                    in_range.contains(&(mesh, t["triangle_index"][row])),
                    "provenance outside the tile"
                );
            } else {
                audit.fill_faces += 1;
                filled.push(*face);
            }
        }
        let mut vanished = vec![];
        let mut fills = vec![];
        for (class, t) in &g.tables {
            #[allow(clippy::needless_range_loop)] // several columns share the row index
            for i in 0..t.values().next().map_or(0, Vec::len) {
                if class == "lod_vanished" {
                    vanished.push((
                        t["node_index"][i],
                        t["mesh_index"][i],
                        t["primitive_index"][i],
                    ));
                    assert_ne!(
                        t["reopen_children"][i], 0,
                        "a vanished record names no child"
                    );
                }
                if class == "lod_filled" {
                    fills.push((
                        [t["loop_x"][i], t["loop_y"][i], t["loop_z"][i]],
                        f64::from_bits(t["bound"][i]),
                    ));
                }
            }
        }
        tiles.push(Tile {
            first,
            end,
            depth: r["depth"].as_u64().unwrap(),
            error: r["geometricError"].as_f64().unwrap(),
            present: primitives(&g),
            faces: g.faces,
            source,
            provenance: vec![],
            vanished,
            fills,
            filled,
        });
    }
    for p in &tiles[leaf_count..] {
        audit.max_depth = audit.max_depth.max(p.depth);
        // Children: tiles one level deeper inside the range.
        let children: Vec<&Tile> = tiles
            .iter()
            .filter(|c| c.depth == p.depth + 1 && c.first >= p.first && c.end <= p.end)
            .collect();
        assert!(!children.is_empty());
        assert_eq!(
            children.iter().map(|c| c.end - c.first).sum::<u64>(),
            p.end - p.first,
            "children cover the range"
        );
        for c in &children {
            assert!(p.error >= c.error, "error not monotone");
            for prim in &c.present {
                assert!(
                    p.present.contains(prim) || p.vanished.contains(prim),
                    "primitive {prim:?} unaccounted"
                );
            }
        }
        for v in &p.vanished {
            assert!(
                children.iter().any(|c| c.present.contains(v)),
                "vanished primitive does not reopen"
            );
        }
        audit.vanished += p.vanished.len();
        for (key, bound) in &p.fills {
            assert!(*bound <= p.error);
            let k = key.map(|b| f64::from(f32::from_bits(b as u32)));
            assert!(
                boundary.contains(&k),
                "a fill closes something that is not a source hole"
            );
        }
        audit.fills += p.fills.len();
        // Two-directional sampled distances (lower bounds of the Hausdorff distance).
        let proxy: Vec<Tri> = p.faces.iter().map(f64s).collect();
        let pb = Bvh::new(&proxy);
        let sb = Bvh::new(&p.source);
        let mut evals = 0;
        let mut worst = 0f64;
        for t in &p.source {
            for s in samples(t, 2) {
                worst = worst.max(pb.nearest(s, u32::MAX, &mut evals).sqrt());
            }
        }
        for t in &proxy {
            for s in samples(t, 4) {
                worst = worst.max(sb.nearest(s, u32::MAX, &mut evals).sqrt());
            }
        }
        assert!(
            worst <= p.error * (1. + 1e-12),
            "tile f{}-d{}: sampled distance {worst} exceeds published error {}",
            p.first,
            p.depth,
            p.error
        );
        if p.error > 0. {
            audit.worst_ratio = audit.worst_ratio.max(worst / p.error);
        }
        audit.worst_gap = audit.worst_gap.max(worst);
    }
    // Frontier seams per proxy depth, on decoded bytes.
    let depths: BTreeSet<u64> = tiles[leaf_count..].iter().map(|t| t.depth).collect();
    for d in depths {
        let mut count: BTreeMap<([u32; 3], [u32; 3]), u32> = BTreeMap::new();
        for (i, t) in tiles.iter().enumerate() {
            let frontier = if i < leaf_count {
                t.depth <= d
            } else {
                t.depth == d
            };
            if !frontier {
                continue;
            }
            for f in &t.faces {
                for k in 0..3 {
                    let (a, b) = (welded(f[k]), welded(f[(k + 1) % 3]));
                    if a != b {
                        *count.entry((a.min(b), a.max(b))).or_default() += 1;
                    }
                }
            }
        }
        // Border-hole rule: no fill at depth d touches a seam vertex, a decoded position
        // shared by two frontier tiles.
        let mut owners: BTreeMap<[u32; 3], BTreeSet<usize>> = BTreeMap::new();
        for (i, t) in tiles.iter().enumerate() {
            if (i < leaf_count && t.depth <= d) || (i >= leaf_count && t.depth == d) {
                for p in t.faces.iter().flatten() {
                    owners.entry(welded(*p)).or_default().insert(i);
                }
            }
        }
        for t in tiles[leaf_count..].iter().filter(|t| t.depth == d) {
            for p in t.filled.iter().flatten() {
                if owners.get(&welded(*p)).is_some_and(|o| o.len() > 1) {
                    audit.seam_fill_vertices += 1;
                }
            }
        }
        let tol = steps[d as usize] * 1.5 + 1e-9;
        let near = |k: [u32; 3]| {
            let p = k.map(|b| f64::from(f32::from_bits(b)));
            boundary
                .iter()
                .any(|b| (0..3).all(|a| (b[a] - p[a]).abs() <= tol))
        };
        for ((a, b), c) in count {
            if c == 1 {
                if near(a) && near(b) {
                    audit.boundary_edges += 1;
                } else {
                    audit.cracks += 1;
                }
            } else {
                audit.matched_edges += 1;
            }
        }
    }
    audit
}

// ---------------------------------------------------------------------------------------
// Tests.

/// The terrain through H8 + H17 + H16: every internal tile has certified content, the
/// independent auditor finds no crack and no sampled distance above a published error,
/// islands vanish and holes are filled with records that reopen, the archive validates
/// with decoded containment, and two worker counts give identical archive bytes.
#[test]
fn terrain_lod_is_certified_seamless_accounted_and_deterministic() {
    let work = tempfile::tempdir().unwrap();
    let source = terrain(150);
    let input = write_gltf(&work.path().join("src"), &source);
    let mut archives = Vec::new();
    for jobs in [1, 2] {
        let dir = leaves(
            &input,
            &work.path().join(format!("leaves{jobs}")),
            jobs,
            Mutation::None,
            1024,
        )
        .unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["proxyManifest"], "proxies.jsonl");
        let lod = &manifest["lod"];
        let (report, members) = archive(&dir, &work.path().join(format!("t{jobs}.3tz")));
        // The second worker count only has to reproduce the first's bytes (checked below);
        // the audit and validation of those bytes run once (debug CI runs are serial).
        if jobs == 1 {
            let audit = audit(&source, &dir);
            eprintln!("terrain jobs {jobs}: {audit:?}\nlod {lod}");
            assert!(audit.proxies >= 3 && audit.max_depth >= 1, "{audit:?}");
            assert_eq!(audit.cracks, 0, "{audit:?}");
            assert_eq!(audit.seam_fill_vertices, 0, "{audit:?}");
            assert!(audit.matched_edges > 0);
            assert!(audit.vanished > 0, "islands should vanish at the root");
            assert!(
                audit.fills > 0,
                "small holes should be filled at a coarse depth"
            );
            assert!(lod["seamMatched"].as_u64().unwrap() > 0);
            assert_eq!(report.proxies as usize, audit.proxies);
            for r in rows(&dir.join("proxies.jsonl")) {
                let bytes = fs::read(dir.join(r["uri"].as_str().unwrap())).unwrap();
                assert!(
                    members.values().any(|m| *m == bytes),
                    "proxy {} not published",
                    r["uri"]
                );
            }
            let checked = inspect(ValidationRequest::new(
                work.path().join(format!("t{jobs}.3tz")),
            ))
            .unwrap();
            assert!(
                checked
                    .checks
                    .iter()
                    .any(|c| c == "decodedContentContainment"),
                "{:?}",
                checked.not_inspected
            );
        }
        archives.push(fs::read(work.path().join(format!("t{jobs}.3tz"))).unwrap());
    }
    assert!(archives[0] == archives[1], "archives differ across jobs");
}

/// Synthetic run depths (> 21) and packed leaves: proxies there decode on the frozen
/// lattice, seams match on decoded bytes and the certificate holds.
#[test]
fn synthetic_depth_lod_is_certified_and_seamless() {
    let work = tempfile::tempdir().unwrap();
    let source = cluster();
    let input = write_gltf(&work.path().join("src"), &source);
    let dir = leaves(&input, &work.path().join("leaves"), 2, Mutation::None, 512).unwrap();
    let audit = audit(&source, &dir);
    eprintln!("cluster: {audit:?}");
    assert!(audit.max_depth > 21, "{audit:?}");
    assert_eq!(audit.cracks, 0);
    assert_eq!(audit.seam_fill_vertices, 0, "{audit:?}");
    let leaf_rows = rows(&dir.join("leaves.jsonl"));
    assert!(leaf_rows.iter().any(|r| r["depth"].as_u64().unwrap() > 21));
    archive(&dir, &work.path().join("c.3tz"));
}

/// Control C2: dropping the removed-region terms understates the error; the auditor's
/// sampled distance on the vanished 3 m island exceeds the published value.
#[test]
fn control_skipped_removed_region_term_is_caught() {
    let work = tempfile::tempdir().unwrap();
    let source = terrain(150);
    let input = write_gltf(&work.path().join("src"), &source);
    let dir = leaves(
        &input,
        &work.path().join("leaves"),
        2,
        Mutation::SkipRemoved,
        1024,
    )
    .unwrap();
    let caught = std::panic::catch_unwind(|| audit(&source, &dir));
    let message = caught
        .err()
        .map(|e| e.downcast::<String>().map(|s| *s).unwrap_or_default());
    assert!(
        message
            .as_deref()
            .is_some_and(|m| m.contains("exceeds published error")),
        "the skipped removed-region term must be caught: {message:?}"
    );
}

/// Control C3: a real crack (one side keeps seam vertices the other removes) is refused by
/// the production seam audit; relabelling it a hole publishes it, and the independent
/// auditor finds the crack in the decoded bytes.
#[test]
fn control_crack_relabelled_as_hole_is_caught() {
    let work = tempfile::tempdir().unwrap();
    let source = terrain(150);
    let input = write_gltf(&work.path().join("src"), &source);
    let refused = leaves(
        &input,
        &work.path().join("refused"),
        2,
        Mutation::InjectCrack,
        1024,
    )
    .unwrap_err();
    assert_eq!(refused.error.kind(), JobErrorKind::InvalidState);
    assert!(
        refused.error.message().contains("crack"),
        "{}",
        refused.error.message()
    );
    let dir = leaves(
        &input,
        &work.path().join("relabelled"),
        2,
        Mutation::CrackAsHole,
        1024,
    )
    .unwrap();
    // Either an unmatched frontier edge, or a fill over a loop that is not a source hole.
    match std::panic::catch_unwind(|| audit(&source, &dir)) {
        Ok(audit) => assert!(
            audit.cracks > 0,
            "the relabelled crack must be found: {audit:?}"
        ),
        Err(e) => {
            let message = e.downcast::<String>().map(|s| *s).unwrap_or_default();
            assert!(message.contains("not a source hole"), "{message}");
        }
    }
}

/// Seam consensus under stress: the first tile of each depth cannot remove a third of its
/// forced seam vertices, so every neighbour that removed them re-runs with them kept. The
/// published proxies (finishing pass over the audited topology caches) must still be
/// crack-free on decoded bytes and certified, with identical bytes across worker counts.
#[test]
fn seam_consensus_reruns_are_audited() {
    let work = tempfile::tempdir().unwrap();
    let source = terrain(150);
    let input = write_gltf(&work.path().join("src"), &source);
    let mut proxies = Vec::new();
    for jobs in [1, 2] {
        let dir = leaves(
            &input,
            &work.path().join(format!("leaves{jobs}")),
            jobs,
            Mutation::ForcedFailures,
            1024,
        )
        .unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let lod = &manifest["lod"];
        assert!(lod["seamRounds"].as_u64().unwrap() > 0, "{lod}");
        assert!(lod["seamKeptExtra"].as_u64().unwrap() > 0, "{lod}");
        // Audited once; the other worker count must reproduce the same bytes.
        if jobs == 1 {
            let audit = audit(&source, &dir);
            eprintln!("consensus jobs {jobs}: {audit:?}\nlod {lod}");
            assert_eq!(audit.cracks, 0, "{audit:?}");
            assert_eq!(audit.seam_fill_vertices, 0, "{audit:?}");
            assert!(audit.matched_edges > 0);
        }
        let mut bytes = Vec::new();
        for r in rows(&dir.join("proxies.jsonl")) {
            bytes.push(fs::read(dir.join(r["uri"].as_str().unwrap())).unwrap());
        }
        proxies.push(bytes);
    }
    assert!(proxies[0] == proxies[1], "proxies differ across jobs");
}

/// Control C4: with the fill eligibility opened to seam (cut) vertices, tiles of the deep
/// cluster cap their own seam perimeters; the production border-hole check refuses the run.
/// (Published outputs are also audited: `seam_fill_vertices` counts fill corners on seams.)
#[test]
fn control_fill_across_seams_is_caught() {
    let work = tempfile::tempdir().unwrap();
    let source = cluster();
    let input = write_gltf(&work.path().join("src"), &source);
    let refused = leaves(
        &input,
        &work.path().join("leaves"),
        2,
        Mutation::FillAcrossSeams,
        512,
    )
    .unwrap_err();
    assert_eq!(refused.error.kind(), JobErrorKind::InvalidState);
    assert!(
        refused.error.message().contains("border-hole rule"),
        "{}",
        refused.error.message()
    );
}

/// Every published file of a leaf directory except the timing record, hashed in name order.
fn tree_digest(dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                // Names joined with `/` on every platform (not the OS separator).
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                if name != "resources.json" {
                    out.push((name, fs::read(&path).unwrap()));
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files);
    files.sort();
    let mut hash = Sha256::new();
    for (name, bytes) in &files {
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The no-baker output is pinned: leaves, proxies, rows and manifest of the terrain run are
/// byte-identical to the H17 branch without the bake handoff (golden taken there).
#[test]
fn no_baker_output_matches_the_pre_handoff_golden() {
    let work = tempfile::tempdir().unwrap();
    let source = terrain(150);
    let input = write_gltf(&work.path().join("src"), &source);
    let dir = leaves(&input, &work.path().join("leaves"), 2, Mutation::None, 1024).unwrap();
    let digest = tree_digest(&dir);
    eprintln!("no-baker digest {digest}");
    assert_eq!(digest, NO_BAKER_GOLDEN);
}

/// `tree_digest` of the terrain run on develop at 0072977 (#209, before the bake handoff),
/// with the platform-independent fixture. Regenerate it only for an intended change of the
/// LOD or leaf output. Regenerated for the H8 gate escalation report: the untextured
/// terrain's `manifest.json` leaf report gained `escalated_pages`,
/// `lossless_fallback_pages`, `escalation_diagnostics` and `escalation_diagnostics_omitted`,
/// then `escalation_reasons` (codec verification escalation); regenerated again by #226,
/// whose leaf and proxy property tables are meshopt streams (decoded values unchanged).
const NO_BAKER_GOLDEN: &str = "8535a539236f33e85be879ae974845eae6567df1c1353be9e9fab4e4d32832fd";

/// The public `mesh_to_tileset` path carries the certified coarse tiles: the published
/// archive holds one member per proxy row beside the leaves and validates.
#[test]
fn public_mesh_to_tileset_publishes_certified_coarse_tiles() {
    let work = tempfile::tempdir().unwrap();
    let source = terrain(90);
    let input = write_gltf(&work.path().join("src"), &source);
    let output = work.path().join("public.3tz");
    crate::mesh_large::mesh_to_tileset(
        crate::mesh_large::TilerRequest::new(&input, &output).with_jobs(
            crate::mesh_large::TilerJobs::Maximum(NonZeroUsize::new(2).unwrap()),
        ),
        &RunControl::default(),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    let mut members = Vec::new();
    for i in 0..zip.len() {
        let mut bytes = Vec::new();
        zip.by_index(i).unwrap().read_to_end(&mut bytes).unwrap();
        members.push(bytes);
    }
    // Coarse members are recognised by their proxy_source tables.
    let coarse = members
        .iter()
        .filter(|m| m.starts_with(b"glTF") && m.windows(12).any(|w| w == b"proxy_source"))
        .count();
    assert!(coarse >= 1, "coarse tiles published");
    let checked = inspect(ValidationRequest::new(&output)).unwrap();
    assert!(
        checked
            .checks
            .iter()
            .any(|c| c == "decodedContentContainment"),
        "{:?}",
        checked.not_inspected
    );
}

/// The seam audit from the composite crack's geometry (mgal x10, depth 6): tiles
/// f88245879 and f89031639 both held the seam-vertex face L-H-P after their forced seam
/// removals, so at depth 5 their parents disagreed about edge L-H. The audit now flags
/// such a face held by two tiles, and an edge claimed by more than two tiles, for both
/// owners; consensus then re-keeps their removed seam vertices (or fails typed).
#[test]
fn seam_audit_flags_duplicate_seam_faces_and_overclaimed_edges() {
    use super::{audit, seam_face, Defect, SeamEdge};
    let key = |p: [f32; 3]| super::incidence::weld(p);
    let l = key([703.64294, 28.421944, 204.25375]);
    let h = key([703.6449, 28.462814, 204.25786]);
    let p = key([703.6427, 28.492601, 204.25209]);
    let q = key([703.6497, 28.519394, 204.1919]);
    let edge = |a: [u32; 3], b: [u32; 3], owner: u64| SeamEdge {
        lo: a.min(b),
        hi: a.max(b),
        kind: 0,
        third: [0; 3],
        owner,
        owner_depth: 6,
        lo_dec: [1; 3],
        hi_dec: [2; 3],
        special: false,
        round: 0,
    };
    let (a, b, c) = (88_245_879, 89_031_639, 89_523_094);
    let mut records = vec![
        seam_face([l, h, p], false, a, 6).unwrap(),
        seam_face([p, l, h], false, b, 6).unwrap(),
        edge(l, h, a),
        edge(l, h, b),
        edge(l, h, c),
        edge(h, p, a),
        edge(h, p, b),
        edge(p, q, c),
    ];
    records.sort();
    let counts = audit(records.into_iter().map(Ok), 6, false).unwrap();
    assert_eq!(
        counts.overlapping, 2,
        "one duplicate face, one overclaimed edge"
    );
    assert_eq!(counts.matched, 1, "H-P is an ordinary two-tile seam edge");
    let defect = |owner: u64| counts.unmatched.get(&(owner, 6)).map(|v| v.2);
    assert!(
        matches!(defect(a), Some(Defect::DuplicateFace(_))),
        "{:?}",
        counts.unmatched
    );
    assert!(
        matches!(defect(b), Some(Defect::DuplicateFace(_))),
        "{:?}",
        counts.unmatched
    );
    // c's overclaimed L-H (or its partnerless P-Q) is reported too.
    assert!(defect(c).is_some());
    // A face whose vertices are all special (a repeated source face) is not flagged.
    let mut special = vec![
        seam_face([l, h, p], true, a, 6).unwrap(),
        seam_face([l, h, p], true, b, 6).unwrap(),
    ];
    special.sort();
    let counts = audit(special.into_iter().map(Ok), 6, false).unwrap();
    assert!(counts.unmatched.is_empty() && counts.overlapping == 0);
}

/// Overlapping seam coverage never passes silently: with the first tile of each depth
/// claiming a seam edge a second time, consensus cannot resolve it and the run fails
/// with a typed error naming the overclaimed edge.
#[test]
fn control_overclaimed_seam_edge_is_refused() {
    let work = tempfile::tempdir().unwrap();
    let source = terrain(150);
    let input = write_gltf(&work.path().join("src"), &source);
    let refused = leaves(
        &input,
        &work.path().join("leaves"),
        2,
        Mutation::OverclaimSeam,
        1024,
    )
    .unwrap_err();
    assert_eq!(refused.error.kind(), JobErrorKind::InvalidState);
    assert!(
        refused.error.message().contains("claimed by"),
        "{}",
        refused.error.message()
    );
}
