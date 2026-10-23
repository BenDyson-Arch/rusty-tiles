//! HLOD S1 acceptance: real runs on small analytic fixtures, checked on DECODED published
//! bytes by an independent census (its own GLB decoder; interfaces derived from the decoded
//! leaves alone, never from producer records), plus the controls that must fail.
use super::{Mutation, Settings, TilerLod};
use crate::{
    content_integrity::meshopt::{self as codec, Mode},
    mesh_large::{
        config::Profile,
        delivery::{mesh_leaves_to_3tz, MeshTilesetRequest},
        leaves::{
            geometry::LatticePlan,
            producer::{mesh_to_leaf_payloads, LeafOutputRequest, LeafProfile},
        },
        records::Bounds,
        TilerJobs, TilerRequest,
    },
    validate::{inspect, ValidationRequest},
    JobErrorKind, RunControl,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    io::Read,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

// ---------------------------------------------------------------------------------------
// Fixtures (IEEE + - * / only, so inputs are identical on every platform).

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

/// A smooth bump surface: rational, so exact on every platform.
fn bump(x: f32, z: f32) -> f32 {
    let (u, v) = ((x - 50.) / 30., (z - 50.) / 30.);
    4. / (1. + u * u + v * v) + 0.01 * x
}

/// A grid of `cells_x × cells_z` quads over `[x0, x1] × [z0, z1]` at height `y(x, z)`.
fn grid(
    cells: (usize, usize),
    x: (f32, f32),
    z: (f32, f32),
    y: &dyn Fn(f32, f32) -> f32,
    color: [f64; 4],
) -> Mesh {
    let (nx, nz) = cells;
    let mut m = Mesh {
        positions: vec![],
        indices: vec![],
        color,
    };
    for i in 0..=nx {
        for j in 0..=nz {
            let px = x.0 + (x.1 - x.0) * i as f32 / nx as f32;
            let pz = z.0 + (z.1 - z.0) * j as f32 / nz as f32;
            m.positions.push([px, y(px, pz), pz]);
        }
    }
    let w = nz + 1;
    for i in 0..nx {
        for j in 0..nz {
            let v = |a: usize, c: usize| (a * w + c) as u32;
            m.indices.extend([v(i, j), v(i, j + 1), v(i + 1, j)]);
            m.indices
                .extend([v(i + 1, j), v(i, j + 1), v(i + 1, j + 1)]);
        }
    }
    m
}

/// A bumpy sheet in two primitives (materials) meeting along a welded seam.
fn sheet() -> Vec<Mesh> {
    vec![
        grid((30, 60), (0., 50.), (0., 100.), &bump, [0.6, 0.5, 0.4, 1.]),
        grid(
            (30, 60),
            (50., 100.),
            (0., 100.),
            &bump,
            [0.3, 0.6, 0.3, 1.],
        ),
    ]
}

/// T-junctions: the right half is twice as fine, so along x = 50 every other right vertex
/// lies on a left edge without being one of its vertices.
fn t_junction() -> Vec<Mesh> {
    vec![
        grid((20, 40), (0., 50.), (0., 100.), &bump, [0.6, 0.5, 0.4, 1.]),
        grid(
            (40, 80),
            (50., 100.),
            (0., 100.),
            &bump,
            [0.3, 0.6, 0.3, 1.],
        ),
    ]
}

/// A nonmanifold "book": three sheets share the spine line x = 50, y = 0, so every spine
/// edge has three incident triangles, in leaves that differ.
fn book() -> Vec<Mesh> {
    let flat = |_: f32, _: f32| 0.;
    let left = grid((25, 50), (0., 50.), (0., 100.), &flat, [0.6, 0.5, 0.4, 1.]);
    let right = grid(
        (25, 50),
        (50., 100.),
        (0., 100.),
        &flat,
        [0.3, 0.6, 0.3, 1.],
    );
    // The vertical page: x = 50, y in [0, 40]; built as a grid in (y, z) then swapped.
    let mut page = grid((20, 50), (0., 40.), (0., 100.), &flat, [0.5, 0.5, 0.6, 1.]);
    for p in &mut page.positions {
        *p = [50., p[0], p[2]];
    }
    vec![left, right, page]
}

/// Thin islands: long 4 cm wide strips hovering over a sheet, crossing every cell, plus
/// single-triangle specks.
fn thin_islands() -> Vec<Mesh> {
    let mut out = vec![grid(
        (30, 30),
        (0., 100.),
        (0., 100.),
        &bump,
        [0.6, 0.5, 0.4, 1.],
    )];
    for k in 0..4 {
        let z0 = 12. + 25. * k as f32;
        let high = move |x: f32, _: f32| 6. + 0.02 * x;
        out.push(grid(
            (200, 1),
            (0., 100.),
            (z0, z0 + 0.04),
            &high,
            [0.9, 0.2, 0.2, 1.],
        ));
    }
    let mut specks = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.2, 0.2, 0.9, 1.],
    };
    for k in 0..40u32 {
        let (x, z) = (2.5 * k as f32 + 0.5, 97.5 - 2.4 * k as f32);
        let base = specks.positions.len() as u32;
        specks
            .positions
            .extend([[x, 8., z], [x + 0.01, 8., z], [x, 8., z + 0.01]]);
        specks.indices.extend([base, base + 1, base + 2]);
    }
    out.push(specks);
    out
}

/// A dense micro-cluster (synthetic run depths > 21, packed leaves) in a sparse frame.
fn cluster() -> Vec<Mesh> {
    let mut dense = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.4, 0.4, 0.8, 1.],
    };
    let n = 120usize;
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

/// A dense bumpy patch filling one octant of the root cube, and a few specks in each other
/// octant: the sparse siblings pack into shared leaves.
fn packed() -> Vec<Mesh> {
    let mut out = vec![grid(
        (24, 24),
        (0., 48.),
        (0., 48.),
        &|x, z| bump(x, z) * 0.2,
        [0.6, 0.5, 0.4, 1.],
    )];
    let mut specks = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.2, 0.2, 0.9, 1.],
    };
    for (k, (x, y, z)) in [
        (75., 2., 10.),
        (10., 2., 75.),
        (75., 2., 75.),
        (10., 70., 10.),
        (75., 70., 10.),
        (10., 70., 75.),
        (75., 70., 75.),
        (90., 95., 90.),
    ]
    .into_iter()
    .enumerate()
    {
        for j in 0..(3 + k as u32) {
            let base = specks.positions.len() as u32;
            let o = j as f32 * 1.5;
            specks
                .positions
                .extend([[x + o, y, z], [x + o + 1., y, z], [x + o, y + 0.5, z + 1.]]);
            specks.indices.extend([base, base + 1, base + 2]);
        }
    }
    out.push(specks);
    out
}

// ---------------------------------------------------------------------------------------
// Independent decoding of published bytes.

struct Decoded {
    faces: Vec<[[f32; 3]; 3]>,
    /// Feature ID of set 0 per face (u32::MAX when the corners disagree).
    features: Vec<u32>,
    table_rows: usize,
}

fn decode(bytes: &[u8]) -> Decoded {
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
                    "TRIANGLES" => Mode::Triangles,
                    _ => Mode::Indices,
                };
                let mut out = codec::decode(
                    &bin[s..s + e["byteLength"].as_u64().unwrap() as usize],
                    e["count"].as_u64().unwrap() as usize,
                    e["byteStride"].as_u64().unwrap() as usize,
                    mode,
                )
                .unwrap();
                if e["filter"] == "OCTAHEDRAL" {
                    out.clear(); // Normals are not needed here.
                }
                out
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
        let size = match ct {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            _ => 4,
        };
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
        let offset = acc["byteOffset"].as_u64().unwrap_or(0) as usize;
        (
            (0..n)
                .map(|i| data[offset + i * stride..offset + i * stride + size * width].to_vec())
                .collect(),
            ct,
        )
    };
    let scalar = |e: &[u8], ct: u64| -> u32 {
        match ct {
            5121 => u32::from(e[0]),
            5123 => u32::from(u16::from_le_bytes([e[0], e[1]])),
            5125 => u32::from_le_bytes(e[..4].try_into().unwrap()),
            _ => f32::from_le_bytes(e[..4].try_into().unwrap()) as u32,
        }
    };
    let mut out = Decoded {
        faces: vec![],
        features: vec![],
        table_rows: 0,
    };
    for root in doc["scenes"][0]["nodes"].as_array().unwrap() {
        let node = &doc["nodes"][root.as_u64().unwrap() as usize];
        let read = |key: &str, default: f32| -> Vec<f32> {
            node.get(key).map_or(vec![default; 3], |v| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_f64().unwrap() as f32)
                    .collect()
            })
        };
        let (t, s) = (read("translation", 0.), read("scale", 1.));
        for prim in doc["meshes"][node["mesh"].as_u64().unwrap() as usize]["primitives"]
            .as_array()
            .unwrap()
        {
            let (pos, ct) = elements(prim["attributes"]["POSITION"].as_u64().unwrap() as usize);
            // Client order: u * s + t in f32, unfused.
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
            let feature: Vec<u32> =
                prim["attributes"]
                    .get("_FEATURE_ID_0")
                    .map_or(vec![u32::MAX; points.len()], |a| {
                        let (e, ct) = elements(a.as_u64().unwrap() as usize);
                        e.iter().map(|b| scalar(b, ct)).collect()
                    });
            let index: Vec<usize> = match prim.get("indices") {
                Some(i) => {
                    let (e, ct) = elements(i.as_u64().unwrap() as usize);
                    e.iter().map(|b| scalar(b, ct) as usize).collect()
                }
                None => (0..points.len()).collect(),
            };
            for c in index.chunks(3) {
                out.faces.push([points[c[0]], points[c[1]], points[c[2]]]);
                let f = feature[c[0]];
                out.features
                    .push(if feature[c[1]] == f && feature[c[2]] == f {
                        f
                    } else {
                        u32::MAX
                    });
            }
        }
    }
    out.table_rows = doc["extensions"]["EXT_structural_metadata"]["propertyTables"][0]["count"]
        .as_u64()
        .unwrap_or(0) as usize;
    out
}

// ---------------------------------------------------------------------------------------
// The census.

type Point = [u32; 3];
fn bits(p: [f32; 3]) -> Point {
    p.map(f32::to_bits)
}
fn seg(a: Point, b: Point) -> (Point, Point) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}
fn degenerate(f: &[Point; 3]) -> bool {
    f[0] == f[1] || f[1] == f[2] || f[0] == f[2]
}

struct Tile {
    uri: String,
    depth: u32,
    begin: usize,
    end: usize,
    faces: Vec<[Point; 3]>,
    leaf: bool,
}

#[derive(Debug, Default)]
struct Census {
    leaves: usize,
    coarse: usize,
    max_depth: u32,
    shared_edges: usize,
    shared_vertices: usize,
    /// Shared edges with three or more incident decoded triangles.
    nonmanifold_edges: usize,
    checked_segments: usize,
    missing_segments: usize,
    missing_vertices: usize,
    mixed_pairs: usize,
    same_depth_pairs: usize,
    max_coarse_triangles: usize,
    invalid_features: usize,
    packed_leaves: usize,
}

fn rows(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn load(dir: &Path) -> Vec<Tile> {
    let leaf_rows = rows(&dir.join("leaves.jsonl"));
    let firsts: Vec<(u64, u64)> = leaf_rows
        .iter()
        .map(|r| (r["first"].as_u64().unwrap(), r["end"].as_u64().unwrap()))
        .collect();
    let mut tiles = Vec::new();
    let mut add = |r: &Value, leaf: bool| {
        let (first, end) = (r["first"].as_u64().unwrap(), r["end"].as_u64().unwrap());
        let begin = firsts.iter().position(|f| f.0 == first).unwrap();
        let stop = firsts.iter().position(|f| f.1 == end).unwrap() + 1;
        let bytes = fs::read(dir.join(r["uri"].as_str().unwrap())).unwrap();
        let decoded = decode(&bytes);
        tiles.push((
            Tile {
                uri: r["uri"].as_str().unwrap().to_owned(),
                depth: r["depth"].as_u64().unwrap() as u32,
                begin,
                end: stop,
                faces: decoded.faces.iter().map(|f| f.map(bits)).collect(),
                leaf,
            },
            decoded,
        ));
    };
    for r in &leaf_rows {
        add(r, true);
    }
    for r in rows(&dir.join("proxies.jsonl")) {
        add(&r, false);
    }
    tiles
        .into_iter()
        .map(|(mut t, d)| {
            if !t.leaf {
                // Coarse feature IDs: one per triangle, inside the table.
                let bad = d
                    .features
                    .iter()
                    .filter(|f| **f == u32::MAX || **f as usize >= d.table_rows)
                    .count();
                if bad > 0 {
                    t.uri = format!("{}#invalid-features={bad}", t.uri);
                }
            }
            t
        })
        .collect()
}

/// Interfaces come from the decoded leaves alone: decoded edges and points shared by
/// different leaves (nondegenerate faces). Every tile must hold, at identical decoded
/// coordinates, each shared segment/point one of its leaves shares with a leaf outside it.
/// Pairs are all disjoint (independently refinable) tile pairs over a shared interface.
fn census(dir: &Path) -> Census {
    let tiles = load(dir);
    let leaves: Vec<&Tile> = tiles.iter().filter(|t| t.leaf).collect();
    let mut c = Census {
        leaves: leaves.len(),
        coarse: tiles.len() - leaves.len(),
        packed_leaves: rows(&dir.join("leaves.jsonl"))
            .iter()
            .filter(|r| r["packed"] == true)
            .count(),
        ..Default::default()
    };
    let mut edge_leaves: HashMap<(Point, Point), BTreeSet<usize>> = HashMap::new();
    let mut edge_faces: HashMap<(Point, Point), usize> = HashMap::new();
    let mut point_leaves: HashMap<Point, BTreeSet<usize>> = HashMap::new();
    for (i, leaf) in leaves.iter().enumerate() {
        assert_eq!((leaf.begin, leaf.end), (i, i + 1));
        for f in leaf.faces.iter().filter(|f| !degenerate(f)) {
            for k in 0..3 {
                let e = seg(f[k], f[(k + 1) % 3]);
                edge_leaves.entry(e).or_default().insert(i);
                *edge_faces.entry(e).or_default() += 1;
                point_leaves.entry(f[k]).or_default().insert(i);
            }
        }
    }
    edge_leaves.retain(|_, l| l.len() > 1);
    point_leaves.retain(|_, l| l.len() > 1);
    c.shared_edges = edge_leaves.len();
    c.shared_vertices = point_leaves.len();
    c.nonmanifold_edges = edge_leaves.keys().filter(|e| edge_faces[*e] > 2).count();
    for t in &tiles {
        c.max_depth = c.max_depth.max(t.depth);
        if t.uri.contains("#invalid-features") {
            c.invalid_features += 1;
        }
        if !t.leaf {
            c.max_coarse_triangles = c.max_coarse_triangles.max(t.faces.len());
        }
        let mut edges = HashSet::new();
        let mut points = HashSet::new();
        for f in &t.faces {
            for k in 0..3 {
                edges.insert(seg(f[k], f[(k + 1) % 3]));
                points.insert(f[k]);
            }
        }
        let crosses = |l: &BTreeSet<usize>| {
            l.iter().any(|x| (t.begin..t.end).contains(x))
                && l.iter().any(|x| !(t.begin..t.end).contains(x))
        };
        for (e, l) in &edge_leaves {
            if crosses(l) {
                c.checked_segments += 1;
                if !edges.contains(e) {
                    c.missing_segments += 1;
                }
            }
        }
        for (p, l) in &point_leaves {
            if crosses(l) && !points.contains(p) {
                c.missing_vertices += 1;
            }
        }
    }
    // Every tile containing each leaf (its ancestors and itself).
    let mut chains: Vec<Vec<usize>> = vec![Vec::new(); leaves.len()];
    for (i, t) in tiles.iter().enumerate() {
        for chain in &mut chains[t.begin..t.end] {
            chain.push(i);
        }
    }
    let mut adjacent: BTreeSet<(usize, usize)> = BTreeSet::new();
    for l in edge_leaves.values().chain(point_leaves.values()) {
        let l: Vec<usize> = l.iter().copied().collect();
        for a in 0..l.len() {
            for b in a + 1..l.len() {
                adjacent.insert((l[a], l[b]));
            }
        }
    }
    let mut pairs = BTreeSet::new();
    for (a, b) in adjacent {
        for &x in &chains[a] {
            for &y in &chains[b] {
                let (tx, ty) = (&tiles[x], &tiles[y]);
                if tx.end <= ty.begin || ty.end <= tx.begin {
                    pairs.insert((x.min(y), x.max(y)));
                }
            }
        }
    }
    for (x, y) in pairs {
        if tiles[x].depth == tiles[y].depth {
            c.same_depth_pairs += 1;
        } else {
            c.mixed_pairs += 1;
        }
    }
    c
}

// ---------------------------------------------------------------------------------------
// Runs.

struct Run {
    leaf_triangles: u64,
    cap: usize,
    jobs: usize,
    lod: TilerLod,
    mutation: Mutation,
    budget: Option<u64>,
    profile: LeafProfile,
}

impl Default for Run {
    fn default() -> Self {
        Self {
            leaf_triangles: 256,
            cap: 300,
            jobs: 2,
            lod: TilerLod::Hlod,
            mutation: Mutation::None,
            budget: None,
            profile: LeafProfile::Balanced,
        }
    }
}

fn stage(input: &Path, out: &Path, run: &Run) -> Result<PathBuf, crate::JobFailure> {
    let scratch = out.with_extension("scratch");
    fs::create_dir_all(&scratch).unwrap();
    let mut request = LeafOutputRequest::new(input, out, run.profile)
        .with_jobs(NonZeroUsize::new(run.jobs).unwrap())
        .with_scratch_dir(&scratch)
        .with_leaf_triangles(run.leaf_triangles)
        .with_lod(run.lod)
        .with_hlod_settings(Settings {
            cap: run.cap,
            mutation: run.mutation,
        });
    if run.lod == TilerLod::H17 {
        request = request.with_lod_target(run.cap);
    }
    if let Some(budget) = run.budget {
        request = request.with_memory_budget(budget);
    }
    let result = mesh_to_leaf_payloads(request, &RunControl::default()).map(|r| r.output);
    assert_eq!(
        fs::read_dir(&scratch).unwrap().count(),
        0,
        "scratch is cleaned up"
    );
    result
}

fn files(dir: &Path, sub: &str) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let path = dir.join(sub);
    if path.is_file() {
        out.insert(sub.to_owned(), fs::read(&path).unwrap());
        return out;
    }
    for entry in fs::read_dir(&path).unwrap() {
        let entry = entry.unwrap();
        out.insert(
            format!("{sub}/{}", entry.file_name().to_string_lossy()),
            fs::read(entry.path()).unwrap(),
        );
    }
    out
}

/// The deterministic published stage: leaves, coarse tiles, their rows and the manifest.
fn published(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = files(dir, "leaves");
    for sub in ["proxies", "leaves.jsonl", "proxies.jsonl", "manifest.json"] {
        out.extend(files(dir, sub));
    }
    out
}

fn archive(dir: &Path, output: &Path) {
    mesh_leaves_to_3tz(MeshTilesetRequest::new(dir, output), &RunControl::default()).unwrap();
    let checked = inspect(ValidationRequest::new(output)).unwrap();
    assert!(
        checked
            .checks
            .iter()
            .any(|c| c == "decodedContentContainment"),
        "{:?}",
        checked.not_inspected
    );
}

/// The full acceptance on one fixture: zero missing decoded segments/points at every
/// tile, leaves byte-identical to the H17 run, the cap, valid representatives, and a
/// validated archive.
fn accept(name: &str, source: &[Mesh], run: Run) -> Census {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), source);
    let dir = stage(&input, &work.path().join("hlod"), &run).unwrap();
    let census = census(&dir);
    eprintln!("{name}: {census:?}");
    assert!(
        census.coarse >= 3,
        "{name}: fixture must build coarse levels"
    );
    assert_eq!(census.missing_segments, 0, "{name}: {census:?}");
    assert_eq!(census.missing_vertices, 0, "{name}: {census:?}");
    assert_eq!(census.invalid_features, 0, "{name}: {census:?}");
    assert!(census.max_coarse_triangles <= run.cap, "{name}: {census:?}");
    assert!(
        census.mixed_pairs > 0 && census.same_depth_pairs > 0,
        "{name}: {census:?}"
    );
    // Every row's monotone error, cap and reduction record.
    let leaf_error: f64 = rows(&dir.join("leaves.jsonl"))
        .iter()
        .map(|r| r["geometryError"].as_f64().unwrap())
        .fold(0., f64::max);
    for row in rows(&dir.join("proxies.jsonl")) {
        let g = row["geometricError"].as_f64().unwrap();
        assert!(g.is_finite() && g >= 0.);
        assert!(row["triangles"].as_u64().unwrap() as usize <= run.cap);
        assert!(row["childrenError"].as_f64().unwrap() <= g);
        if row["depth"].as_u64().unwrap() == 0 {
            assert!(g >= leaf_error);
        }
    }
    // Leaves are H8's, byte for byte, whichever coarse method runs.
    let h17 = stage(
        &input,
        &work.path().join("h17"),
        &Run {
            lod: TilerLod::H17,
            leaf_triangles: run.leaf_triangles,
            profile: run.profile,
            ..Run::default()
        },
    )
    .unwrap();
    let mut a = files(&dir, "leaves");
    a.extend(files(&dir, "leaves.jsonl"));
    let mut b = files(&h17, "leaves");
    b.extend(files(&h17, "leaves.jsonl"));
    assert!(a == b, "{name}: HLOD leaves differ from H17's");
    archive(&dir, &work.path().join("out.3tz"));
    census
}

// ---------------------------------------------------------------------------------------
// Tests.

#[test]
fn canonical_decode_is_every_leaf_decode_and_survives_coarse_encoding() {
    let bounds = Bounds {
        min: [-1000.; 3],
        max: [1000.; 3],
    };
    let extents: Vec<f64> = (0..12).map(|d| 2000. / f64::from(1u32 << d)).collect();
    for profile in [Profile::Balanced, Profile::Fast, Profile::Lossless] {
        let lattice = LatticePlan::new(profile, bounds, &extents).unwrap();
        // Within a few metres, so even depth-9 leaves (3.9 m extent) hold them.
        let points: Vec<[f32; 3]> = (0..300)
            .map(|i| {
                let t = i as f32;
                [
                    123.4567 + (t * 0.0123457).fract(),
                    -5.678 + (t * 0.731).fract() * 0.9,
                    -78.9012 + (t * 0.0198765).fract(),
                ]
            })
            .collect();
        for ell in [2u8, 5, 9] {
            let ells = vec![ell; points.len()];
            let canonical: Vec<[f32; 3]> = points
                .iter()
                .map(|p| lattice.canonical(*p, ell).unwrap())
                .collect();
            // Every leaf depth at or below the vertex's minimum leaf depth decodes it so.
            for depth in u32::from(ell)..10 {
                let leaf = lattice.encode(&points, &ells, depth).unwrap();
                assert_eq!(
                    leaf.decoded.iter().map(|p| bits(*p)).collect::<Vec<_>>(),
                    canonical.iter().map(|p| bits(*p)).collect::<Vec<_>>(),
                    "{profile:?} ell {ell} leaf depth {depth}"
                );
            }
            // Locked coarse vertices decode exactly there at every coarse depth.
            let locked: Vec<bool> = (0..points.len()).map(|i| i % 3 == 0).collect();
            for depth in 0..=u32::from(ell) {
                let coarse = lattice
                    .encode_coarse(&canonical, &ells, &locked, depth)
                    .unwrap();
                for (i, p) in coarse.decoded.iter().enumerate() {
                    if locked[i] {
                        assert_eq!(bits(*p), bits(canonical[i]));
                    }
                }
            }
        }
    }
}

#[test]
fn sheet_is_crack_free_at_every_depth_mix_with_exact_leaves() {
    let census = accept("sheet", &sheet(), Run::default());
    assert!(census.shared_edges > 100, "{census:?}");
}

/// T-junction interfaces are not shared source edges: along x = 50 the right side has
/// extra vertices on the left side's edges. Every tile on one side must still cover
/// exactly the decoded line extent its own leaves cover (no gap opens along the line).
#[test]
fn t_junctions_keep_their_shared_vertices_and_segments() {
    accept("t-junction", &t_junction(), Run::default());
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &t_junction());
    let dir = stage(&input, &work.path().join("hlod"), &Run::default()).unwrap();
    let tiles = load(&dir);
    let line = 50f32.to_bits();
    let coverage = |faces: &[[Point; 3]]| -> Vec<(u32, u32)> {
        let mut spans: Vec<(f32, f32)> = Vec::new();
        for f in faces.iter().filter(|f| !degenerate(f)) {
            for k in 0..3 {
                let (a, b) = (f[k], f[(k + 1) % 3]);
                if a[0] == line && b[0] == line {
                    let (za, zb) = (f32::from_bits(a[2]), f32::from_bits(b[2]));
                    spans.push((za.min(zb), za.max(zb)));
                }
            }
        }
        spans.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let mut union: Vec<(f32, f32)> = Vec::new();
        for (lo, hi) in spans {
            match union.last_mut() {
                Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
                _ => union.push((lo, hi)),
            }
        }
        union
            .iter()
            .map(|(a, b)| (a.to_bits(), b.to_bits()))
            .collect()
    };
    let side = |t: &Tile| -> Option<bool> {
        let xs: Vec<f32> = t
            .faces
            .iter()
            .filter(|f| !degenerate(f))
            .map(|f| {
                (f32::from_bits(f[0][0]) + f32::from_bits(f[1][0]) + f32::from_bits(f[2][0])) / 3.
            })
            .collect();
        if xs.iter().all(|x| *x < 50.) {
            Some(false)
        } else if xs.iter().all(|x| *x > 50.) {
            Some(true)
        } else {
            None
        }
    };
    let leaves: Vec<&Tile> = tiles.iter().filter(|t| t.leaf).collect();
    let mut checked = 0;
    for t in tiles.iter().filter(|t| !t.leaf) {
        let mine = &leaves[t.begin..t.end];
        let Some(s) = side(t) else { continue };
        if mine.iter().any(|l| side(l) != Some(s)) {
            continue;
        }
        let from_leaves: Vec<[Point; 3]> =
            mine.iter().flat_map(|l| l.faces.iter().copied()).collect();
        let expected = coverage(&from_leaves);
        if expected.is_empty() {
            continue;
        }
        assert_eq!(coverage(&t.faces), expected, "{}", t.uri);
        checked += 1;
    }
    assert!(
        checked >= 4,
        "{checked} one-sided coarse tiles touch the T-junction line"
    );
}

#[test]
fn nonmanifold_shared_edges_keep_every_owner_pair() {
    let census = accept("book", &book(), Run::default());
    assert!(census.nonmanifold_edges > 0, "{census:?}");
}

#[test]
fn thin_islands_crossing_cells_keep_their_interfaces() {
    accept("thin islands", &thin_islands(), Run::default());
}

#[test]
fn packed_leaves_and_synthetic_depths_are_crack_free() {
    let census = accept(
        "cluster",
        &cluster(),
        Run {
            leaf_triangles: 512,
            ..Run::default()
        },
    );
    assert!(census.max_depth > 21, "{census:?}");
    let census = accept(
        "packed",
        &packed(),
        Run {
            leaf_triangles: 200,
            ..Run::default()
        },
    );
    assert!(census.packed_leaves > 0, "{census:?}");
}

#[test]
fn float_profiles_are_crack_free() {
    accept(
        "sheet lossless",
        &sheet(),
        Run {
            profile: LeafProfile::Lossless,
            ..Run::default()
        },
    );
}

/// Control: a moved interface vertex that bypasses the producer's check is found by the
/// independent census; a dropped segment is refused before publication.
#[test]
fn moved_or_dropped_segments_fail() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let dir = stage(
        &input,
        &work.path().join("moved"),
        &Run {
            mutation: Mutation::MoveVertexUnchecked,
            ..Run::default()
        },
    )
    .unwrap();
    let census = census(&dir);
    assert!(
        census.missing_segments > 0 && census.missing_vertices > 0,
        "{census:?}"
    );
    let failure = stage(
        &input,
        &work.path().join("dropped"),
        &Run {
            mutation: Mutation::DropSegment,
            ..Run::default()
        },
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidState);
    assert!(
        failure.error.to_string().contains("lost interface"),
        "{}",
        failure.error
    );
    assert!(!work.path().join("dropped").exists());
}

/// An interface that cannot fit the cap is `unsupported` with tile/count/constraint
/// diagnostics, never an over-cap or interface-breaking tile.
#[test]
fn interface_that_cannot_fit_the_cap_is_unsupported() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let failure = stage(
        &input,
        &work.path().join("capped"),
        &Run {
            cap: 12,
            ..Run::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        failure.error.kind(),
        JobErrorKind::Unsupported,
        "{}",
        failure.error
    );
    let message = failure.error.to_string();
    assert!(
        message.contains("coarse tile f") && message.contains("cap"),
        "{message}"
    );
    assert!(!work.path().join("capped").exists());
}

/// Output is identical for any job count, any admitted budget and spilled sorts.
#[test]
fn output_is_identical_across_jobs_and_budgets() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &thin_islands());
    let one = stage(
        &input,
        &work.path().join("one"),
        &Run {
            jobs: 1,
            ..Run::default()
        },
    )
    .unwrap();
    let many = stage(
        &input,
        &work.path().join("many"),
        &Run {
            jobs: 4,
            budget: Some(64 << 30),
            ..Run::default()
        },
    )
    .unwrap();
    assert!(published(&one) == published(&many));
}

fn public(
    input: &Path,
    output: &Path,
    budget: Option<u64>,
    jobs: usize,
) -> Result<crate::TilerResult, crate::JobFailure> {
    let mut request = TilerRequest::new(input, output)
        .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(jobs).unwrap()))
        .with_lod(TilerLod::Hlod)
        .with_test_leaf_triangles(256)
        .with_hlod_settings(Settings {
            cap: 300,
            mutation: Mutation::None,
        })
        .with_test_spill(
            crate::spill::SortConfig {
                run_bytes: 1 << 16,
                fan_in: 4,
                buffer_bytes: 1 << 12,
                jobs: 1,
            },
            1 << 14,
        );
    if let Some(budget) = budget {
        request = request.with_memory_budget(budget);
    }
    crate::mesh_large::mesh_to_tileset(request, &RunControl::default())
}

/// D8 through the public operation: the reported minimum budget is admitted and gives the
/// same archive; one byte less is a resource refusal before the binding stage allocates;
/// untextured input reserves no texture workspace; spilled sorts give the same bytes.
#[test]
fn budget_equality_admits_and_one_below_refuses() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let first = public(&input, &work.path().join("a.3tz"), Some(64 << 30), 3).unwrap();
    let memory = first.diagnostics.memory.clone().unwrap();
    assert_eq!(memory.budget.resolved_bytes, 64 << 30);
    assert_eq!(
        first.diagnostics.leaf_delivery.page_bytes, 0,
        "no texture workspace"
    );
    assert_eq!(first.diagnostics.leaf_delivery.cache_bytes, 0);
    assert_eq!(memory.hlod.texture_workspace_bytes, 0);
    let hlod = first.diagnostics.hlod.clone().unwrap();
    assert!(hlod.tiles >= 3 && hlod.lease_peak_bytes <= memory.hlod.ledger_bytes);
    let minimum = memory.minimum_bytes;
    let exact = public(&input, &work.path().join("b.3tz"), Some(minimum), 3).unwrap();
    assert_eq!(exact.diagnostics.memory.unwrap().minimum_bytes, minimum);
    assert!(
        fs::read(work.path().join("a.3tz")).unwrap()
            == fs::read(work.path().join("b.3tz")).unwrap()
    );
    let below = public(&input, &work.path().join("c.3tz"), Some(minimum - 1), 3).unwrap_err();
    assert_eq!(
        below.error.kind(),
        JobErrorKind::ResourceLimit,
        "{}",
        below.error
    );
    assert!(!work.path().join("c.3tz").exists());
    // A budget names the HLOD path only; zero is refused.
    let h17 = crate::mesh_large::mesh_to_tileset(
        TilerRequest::new(&input, work.path().join("d.3tz"))
            .with_test_leaf_triangles(256)
            .with_memory_budget(1 << 30),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(h17.error.kind(), JobErrorKind::InvalidRequest);
    let zero = public(&input, &work.path().join("e.3tz"), Some(0), 1).unwrap_err();
    assert_eq!(zero.error.kind(), JobErrorKind::InvalidRequest);
    // The public archive's coarse members are the HLOD members.
    let mut zip = zip::ZipArchive::new(fs::File::open(work.path().join("a.3tz")).unwrap()).unwrap();
    let mut coarse = 0;
    for i in 0..zip.len() {
        let mut bytes = Vec::new();
        zip.by_index(i).unwrap().read_to_end(&mut bytes).unwrap();
        coarse += usize::from(bytes.windows(25).any(|w| w == b"rusty_tiles_large_hlod_v1"));
    }
    assert_eq!(coarse as u64, hlod.tiles);
}

/// Cancellation inside the HLOD stage leaves no output and no scratch.
#[test]
fn cancellation_during_hlod_cleans_up() {
    struct CancelOn(std::sync::OnceLock<crate::CancellationHandle>);
    impl crate::Observer for CancelOn {
        fn observe(&self, event: &crate::RunEvent<'_>) -> Result<(), crate::JobError> {
            if let crate::RunEvent::Progress {
                phase: "hlod_geometry",
                done,
                ..
            } = event
            {
                if *done >= 1 {
                    self.0.get().unwrap().cancel();
                }
            }
            Ok(())
        }
    }
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let scratch = work.path().join("scratch");
    fs::create_dir_all(&scratch).unwrap();
    let observer = std::sync::Arc::new(CancelOn(std::sync::OnceLock::new()));
    let control = RunControl::new(Some(observer.clone()));
    observer.0.set(control.cancellation_handle()).ok().unwrap();
    let output = work.path().join("cancelled.3tz");
    let failure = crate::mesh_large::mesh_to_tileset(
        TilerRequest::new(&input, &output)
            .with_lod(TilerLod::Hlod)
            .with_test_leaf_triangles(256)
            .with_hlod_settings(Settings {
                cap: 300,
                mutation: Mutation::None,
            })
            .with_scratch_dir(&scratch),
        &control,
    )
    .unwrap_err();
    assert_eq!(
        failure.error.kind(),
        JobErrorKind::Cancelled,
        "{}",
        failure.error
    );
    assert!(!output.exists());
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    assert!(failure.retained_paths.is_empty());
}

/// Evidence lane, not CI: the production-settings HLOD stage (8,192-triangle leaves and
/// coarse cap, balanced, four jobs, optional `RT_HLOD_EVIDENCE_BUDGET`) of
/// `RT_HLOD_EVIDENCE_INPUT`, written under `RT_HLOD_EVIDENCE_DIR`, censused on its decoded bytes. Writes `census.json` with every
/// member's SHA-256 so the CLI archive's members can be matched to the censused bytes.
#[test]
#[ignore = "evidence lane: needs RT_HLOD_EVIDENCE_INPUT and RT_HLOD_EVIDENCE_DIR"]
fn evidence_census() {
    let input = PathBuf::from(std::env::var("RT_HLOD_EVIDENCE_INPUT").unwrap());
    let out = PathBuf::from(std::env::var("RT_HLOD_EVIDENCE_DIR").unwrap());
    fs::create_dir_all(&out).unwrap();
    let started = std::time::Instant::now();
    let dir = stage(
        &input,
        &out.join("stage"),
        &Run {
            leaf_triangles: crate::mesh_large::config::CONSUMER_TABLE_ROWS,
            cap: super::TRIANGLE_CAP,
            jobs: 4,
            budget: std::env::var("RT_HLOD_EVIDENCE_BUDGET")
                .ok()
                .map(|b| crate::mesh_large::resources::parse_memory_budget(&b).unwrap()),
            ..Run::default()
        },
    )
    .unwrap();
    let stage_seconds = started.elapsed().as_secs_f64();
    let census = census(&dir);
    let members: BTreeMap<String, String> = rows(&dir.join("leaves.jsonl"))
        .into_iter()
        .chain(rows(&dir.join("proxies.jsonl")))
        .map(|r| {
            (
                r["uri"].as_str().unwrap().to_owned(),
                r["sha256"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let resources: Value =
        serde_json::from_slice(&fs::read(dir.join("resources.json")).unwrap()).unwrap();
    let record = json!({
        "input": input, "stageSeconds": stage_seconds,
        "census": format!("{census:?}"),
        "leaves": census.leaves, "coarse": census.coarse, "maxDepth": census.max_depth,
        "sharedSegments": census.shared_edges, "sharedPoints": census.shared_vertices,
        "nonmanifoldSegments": census.nonmanifold_edges, "checkedSegments": census.checked_segments,
        "missingSegments": census.missing_segments, "missingPoints": census.missing_vertices,
        "mixedPairs": census.mixed_pairs, "sameDepthPairs": census.same_depth_pairs,
        "maxCoarseTriangles": census.max_coarse_triangles, "invalidFeatures": census.invalid_features,
        "hlod": manifest["hlod"], "resources": resources["hlod"], "members": members,
    });
    fs::write(
        out.join("census.json"),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    eprintln!("{census:?}");
    assert_eq!(census.missing_segments + census.missing_vertices, 0);
    assert!(census.max_coarse_triangles <= super::TRIANGLE_CAP);
}
