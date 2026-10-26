//! HLOD S1 acceptance: real runs on small analytic fixtures, checked on DECODED published
//! bytes by an independent census (its own GLB decoder; interfaces derived from the decoded
//! leaves alone, never from producer records), plus the controls that must fail.
mod border_census;
use super::{error::Schedule, Mutation, Settings};
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
    write_gltf_scaled(dir, meshes, &[])
}

/// `write_gltf` with a node scale per mesh (`[1; 3]` where `scales` is shorter).
fn write_gltf_scaled(dir: &Path, meshes: &[Mesh], scales: &[[f32; 3]]) -> PathBuf {
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
        nodes.push(json!({"mesh":i,"scale":scales.get(i).copied().unwrap_or([1.; 3])}));
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

/// The T-junction line: off every coarse tile lattice (50 + 2^-9), so a border vertex
/// snapped to a coarse lattice would leave it.
const T_LINE: f32 = 50. + 1. / 512.;

/// T-junctions: the right half is twice as fine, so along x = `T_LINE` every other right
/// vertex lies on a left edge without being one of its vertices.
fn t_junction() -> Vec<Mesh> {
    // Curved across the line but straight along it, so the T-junctions are exact.
    let ridge = |x: f32, _: f32| {
        let u = (x - 50.) / 30.;
        4. / (1. + u * u) + 0.01 * x
    };
    vec![
        grid(
            (20, 40),
            (0., T_LINE),
            (0., 100.),
            &ridge,
            [0.6, 0.5, 0.4, 1.],
        ),
        grid(
            (40, 80),
            (T_LINE, 100.),
            (0., 100.),
            &ridge,
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
    // Enough for synthetic depth 23 at 512 faces/leaf; the former 120x120 grid
    // spent most of its time on decoded degenerates and oversized test audit admission.
    let n = 48usize;
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
    contracted_source_segments: usize,
    contracted_source_vertices: usize,
    certificate_failures: usize,
    certificate_winding_failures: usize,
    max_same_depth_gap_m: f64,
    max_mixed_depth_gap_m: f64,
    missing_segments: usize,
    missing_vertices: usize,
    mixed_pairs: usize,
    same_depth_pairs: usize,
    max_coarse_triangles: usize,
    invalid_features: usize,
    packed_leaves: usize,
    /// Open-border points of one leaf strictly inside an open-border segment of another.
    overlaps: usize,
    missing_overlap_items: usize,
    /// Decoded open-border vertices of coarse tiles at no decoded vertex of their leaves.
    moved_border_vertices: usize,
    max_border_move_m: f64,
    /// Perpendicular displacement: (tile, interface point) pairs measured, where the point is
    /// shared with or lies on a leaf outside the tile.
    perpendicular_checked: usize,
    /// Pairs whose point lies farther from the tile's decoded edges than from its own inside
    /// leaf's (or beyond `PERPENDICULAR_RADIUS`).
    perpendicular_displaced: usize,
    /// Largest such excess, metres (`PERPENDICULAR_RADIUS` when nothing was within reach).
    max_perpendicular_excess_m: f64,
    /// Pairs of tiles on one root-to-leaf path with identical decoded content.
    duplicate_chain_tiles: usize,
    /// Coarse faces whose corners are exactly those of a descendant leaf face in reverse,
    /// with no descendant face of their own orientation (S5's source-orientation check).
    reversed_coarse_faces: usize,
}

/// Search radius of the perpendicular census, metres: a point farther than this from every
/// decoded edge of a tile counts as displaced by (at least) this much.
const PERPENDICULAR_RADIUS: f64 = 0.05;

/// Segments indexed for point queries within a radius, in bounded memory whatever their
/// length mix: each segment sits in the grid level whose power-of-two cell covers it and its
/// padding (at most two cells per axis, so at most eight entries), and a point meets every
/// segment within reach in its own cell of each level.
struct SegmentIndex {
    segments: Vec<([f64; 3], [f64; 3])>,
    levels: BTreeMap<i32, HashMap<[i64; 3], Vec<usize>>>,
}

impl SegmentIndex {
    /// `radius` is the query reach; every segment is also padded by a millionth of its
    /// extent, so points collinear within rounding are met.
    fn new(segments: Vec<([f64; 3], [f64; 3])>, radius: f64) -> Self {
        let mut levels: BTreeMap<i32, HashMap<[i64; 3], Vec<usize>>> = BTreeMap::new();
        for (k, (a, b)) in segments.iter().enumerate() {
            let span = (0..3).map(|i| (b[i] - a[i]).abs()).fold(0., f64::max);
            let pad = radius + 1e-6 * span;
            let extent = (span + 2. * pad).max(f64::MIN_POSITIVE);
            let level = extent.log2().ceil() as i32;
            let size = 2f64.powi(level);
            let lo: [i64; 3] =
                std::array::from_fn(|i| ((a[i].min(b[i]) - pad) / size).floor() as i64);
            let hi: [i64; 3] =
                std::array::from_fn(|i| ((a[i].max(b[i]) + pad) / size).floor() as i64);
            assert!(
                (0..3).all(|i| hi[i] - lo[i] <= 1),
                "segment index cell bound"
            );
            let grid = levels.entry(level).or_default();
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        grid.entry([x, y, z]).or_default().push(k);
                    }
                }
            }
        }
        Self { segments, levels }
    }

    /// Every segment whose padded cell holds `p` (a superset of those within reach).
    fn near(&self, p: [f64; 3]) -> impl Iterator<Item = usize> + '_ {
        self.levels.iter().flat_map(move |(level, grid)| {
            let size = 2f64.powi(*level);
            let cell = p.map(|v| (v / size).floor() as i64);
            grid.get(&cell).into_iter().flatten().copied()
        })
    }

    /// Grid entries held: at most eight per segment.
    fn entries(&self) -> usize {
        self.levels
            .values()
            .flat_map(|g| g.values())
            .map(Vec::len)
            .sum()
    }
}

/// Decoded edges of one tile, for point-to-edge distance within a radius.
struct EdgeIndex {
    index: SegmentIndex,
    radius: f64,
}

impl EdgeIndex {
    fn new(faces: &[[Point; 3]], radius: f64) -> Self {
        let f = |p: Point| p.map(|b| f64::from(f32::from_bits(b)));
        let mut unique: Vec<(Point, Point)> = faces
            .iter()
            .filter(|t| !degenerate(t))
            .flat_map(|t| (0..3).map(move |k| seg(t[k], t[(k + 1) % 3])))
            .collect();
        unique.sort_unstable();
        unique.dedup();
        let segments = unique.iter().map(|(a, b)| (f(*a), f(*b))).collect();
        Self {
            index: SegmentIndex::new(segments, radius),
            radius,
        }
    }

    /// Distance from `p` to the nearest edge, if one is within the radius.
    fn distance(&self, p: [f64; 3]) -> Option<f64> {
        let best = self
            .index
            .near(p)
            .map(|k| {
                let (a, b) = self.index.segments[k];
                point_segment(p, a, b)
            })
            .fold(f64::INFINITY, f64::min);
        (best <= self.radius).then_some(best)
    }
}

fn point_segment(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let d: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
    let l2: f64 = d.iter().map(|v| v * v).sum();
    let t = if l2 > 0. {
        ((0..3).map(|i| (p[i] - a[i]) * d[i]).sum::<f64>() / l2).clamp(0., 1.)
    } else {
        0.
    };
    (0..3)
        .map(|i| (p[i] - a[i] - t * d[i]).powi(2))
        .sum::<f64>()
        .sqrt()
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
    // Overlaps (T-junctions, collinear partial overlaps): an open-border point of leaf Y
    // strictly inside an open-border segment of leaf X. A tile holding X but not Y keeps
    // the segment; one holding Y but not X keeps the point.
    let open = |faces: &[[Point; 3]]| -> Vec<(Point, Point)> {
        let mut count: HashMap<(Point, Point), usize> = HashMap::new();
        for f in faces.iter().filter(|f| !degenerate(f)) {
            for k in 0..3 {
                *count.entry(seg(f[k], f[(k + 1) % 3])).or_default() += 1;
            }
        }
        count
            .into_iter()
            .filter(|(_, n)| *n != 2)
            .map(|(e, _)| e)
            .collect()
    };
    let f = |p: Point| p.map(|b| f64::from(f32::from_bits(b)));
    let mut open_segments: Vec<(usize, Point, Point)> = Vec::new();
    let mut open_points: Vec<(usize, Point)> = Vec::new();
    for (i, leaf) in leaves.iter().enumerate() {
        let edges = open(&leaf.faces);
        let mut points: Vec<Point> = edges.iter().flat_map(|(a, b)| [*a, *b]).collect();
        points.sort_unstable();
        points.dedup();
        open_points.extend(points.into_iter().map(|p| (i, p)));
        open_segments.extend(edges.into_iter().map(|(a, b)| (i, a, b)));
    }
    let lengths: Vec<f64> = open_segments
        .iter()
        .map(|(_, a, b)| {
            let (a, b) = (f(*a), f(*b));
            (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
        })
        .collect();
    let grid = SegmentIndex::new(
        open_segments
            .iter()
            .map(|(_, a, b)| (f(*a), f(*b)))
            .collect(),
        0.,
    );
    let mut overlap_items: Vec<(usize, usize, Point, Point)> = Vec::new();
    // (host leaf, guest leaf, guest point, its perpendicular distance from the host segment)
    let mut overlap_points: Vec<(usize, usize, [f64; 3], f64)> = Vec::new();
    for (y, p) in &open_points {
        let q = f(*p);
        for k in grid.near(q) {
            let (x, a, b) = open_segments[k];
            if x == *y || *p == a || *p == b {
                continue;
            }
            let (pa, pb) = (f(a), f(b));
            let d: [f64; 3] = std::array::from_fn(|i| pb[i] - pa[i]);
            let l2: f64 = d.iter().map(|v| v * v).sum();
            let t = (0..3).map(|i| (q[i] - pa[i]) * d[i]).sum::<f64>() / l2;
            let perp2: f64 = (0..3).map(|i| (q[i] - pa[i] - t * d[i]).powi(2)).sum();
            if t > 0. && t < 1. && perp2 <= (1e-7 * lengths[k]).powi(2) {
                c.overlaps += 1;
                overlap_items.push((x, *y, a, b));
                overlap_items.push((*y, x, *p, *p));
                overlap_points.push((x, *y, q, perp2.sqrt()));
            }
        }
    }
    for t in tiles.iter() {
        let inside = |l: usize| (t.begin..t.end).contains(&l);
        let mut edges = HashSet::new();
        let mut points = HashSet::new();
        for face in &t.faces {
            for k in 0..3 {
                edges.insert(seg(face[k], face[(k + 1) % 3]));
                points.insert(face[k]);
            }
        }
        for (own, other, a, b) in &overlap_items {
            if inside(*own) && !inside(*other) {
                let kept = if a == b {
                    points.contains(a)
                } else {
                    edges.contains(&(*a, *b))
                };
                c.missing_overlap_items += usize::from(!kept);
            }
        }
        if !t.leaf {
            // No decoded open-border vertex of a coarse tile may leave the decoded vertices
            // of its own leaves: that is a perpendicular (or any) border displacement.
            let leaf_points: HashSet<Point> = leaves[t.begin..t.end]
                .iter()
                .flat_map(|l| l.faces.iter().flatten().copied())
                .collect();
            let mut moved: Vec<Point> = open(&t.faces)
                .into_iter()
                .flat_map(|(a, b)| [a, b])
                .filter(|p| !leaf_points.contains(p))
                .collect();
            moved.sort_unstable();
            moved.dedup();
            for p in &moved {
                let q = f(*p);
                let nearest = leaf_points
                    .iter()
                    .map(|l| {
                        let l = f(*l);
                        (0..3).map(|i| (l[i] - q[i]).powi(2)).sum::<f64>().sqrt()
                    })
                    .fold(f64::INFINITY, f64::min);
                c.max_border_move_m = c.max_border_move_m.max(nearest);
            }
            c.moved_border_vertices += moved.len();
        }
    }
    // Perpendicular displacement, the crack itself: every interface point between a tile's
    // leaves and the rest (a decoded point shared across the interval, or a T-junction point
    // on either side) must lie on the tile's decoded edges no farther than it lies from its
    // own inside leaf's, at leaf resolution (zero for shared and own points; the leaves'
    // own T-junction offset for a neighbour's point on a host segment). So any two disjoint
    // tiles meet wherever their leaves do, measured across the border, not along it.
    for t in tiles.iter() {
        let inside = |l: usize| (t.begin..t.end).contains(&l);
        let mut queries: Vec<([f64; 3], f64)> = point_leaves
            .iter()
            .filter(|(_, l)| l.iter().any(|x| inside(*x)) && l.iter().any(|x| !inside(*x)))
            .map(|(p, _)| (f(*p), 0.))
            .collect();
        for (host, guest, q, perp) in &overlap_points {
            match (inside(*host), inside(*guest)) {
                (true, false) => queries.push((*q, *perp)),
                (false, true) => queries.push((*q, 0.)),
                _ => {}
            }
        }
        if queries.is_empty() {
            continue;
        }
        let index = EdgeIndex::new(&t.faces, PERPENDICULAR_RADIUS);
        for (q, baseline) in queries {
            c.perpendicular_checked += 1;
            let excess = index
                .distance(q)
                .map_or(PERPENDICULAR_RADIUS, |d| (d - baseline).max(0.));
            if excess > 1e-9 {
                c.perpendicular_displaced += 1;
                c.max_perpendicular_excess_m = c.max_perpendicular_excess_m.max(excess);
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
    // No two tiles on one root-to-leaf path carry the same content (a pass-through chain
    // publishes its single child once): compared as decoded triangle sets.
    let contents: Vec<Vec<[Point; 3]>> = tiles
        .iter()
        .map(|t| {
            let mut faces = t.faces.clone();
            faces.sort_unstable();
            faces
        })
        .collect();
    let mut duplicates = BTreeSet::new();
    for chain in &chains {
        for (i, &x) in chain.iter().enumerate() {
            for &y in &chain[i + 1..] {
                if contents[x] == contents[y] {
                    duplicates.insert((x.min(y), x.max(y)));
                }
            }
        }
    }
    c.duplicate_chain_tiles = duplicates.len();
    // Source orientation: each oriented leaf face (rotated to its least corner) by leaf.
    let rotate = |f: [Point; 3]| {
        let k = (0..3).min_by_key(|&k| f[k]).unwrap();
        [f[k], f[(k + 1) % 3], f[(k + 2) % 3]]
    };
    let mut oriented: HashMap<[Point; 3], Vec<usize>> = HashMap::new();
    for (i, leaf) in leaves.iter().enumerate() {
        for f in leaf.faces.iter().filter(|f| !degenerate(f)) {
            oriented.entry(rotate(*f)).or_default().push(i);
        }
    }
    for t in tiles.iter().filter(|t| !t.leaf) {
        let within = |key: &[Point; 3]| {
            oriented
                .get(key)
                .is_some_and(|l| l.iter().any(|&i| (t.begin..t.end).contains(&i)))
        };
        for f in t.faces.iter().filter(|f| !degenerate(f)) {
            if !within(&rotate(*f)) && within(&rotate([f[0], f[2], f[1]])) {
                c.reversed_coarse_faces += 1;
            }
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
    border_census::check(
        dir,
        &tiles,
        &edge_leaves,
        &point_leaves,
        &edge_faces,
        &mut c,
    );
    c
}

// ---------------------------------------------------------------------------------------
// Runs.

struct Run {
    leaf_triangles: u64,
    cap: usize,
    jobs: usize,
    mutation: Mutation,
    budget: Option<u64>,
    profile: LeafProfile,
    /// S2's schedule: reduced unless a test exercises S2's measurement itself.
    sampling: Schedule,
    nearest_steps: u64,
}

/// S2's reduced sampling for tests that do not exercise its measurement. Production (and
/// every delivery outside library tests) uses `Schedule::PRODUCTION`: 200,000 per depth,
/// 2,048 per tile.
const TEST_SAMPLING: Schedule = Schedule {
    per_depth: 4096,
    per_tile: 64,
};
/// S2's nearest-search budget for those tests (production: `error::NEAREST_STEPS`). Its
/// controls (every source sliver vertex, every proxy vertex and edge) do not scale with the
/// schedule; a small budget keeps adversarial geometry cheap and only raises G.
const TEST_NEAREST_STEPS: u64 = 64;

/// Stage settings of the public-pipeline tests (reduced S2 work, see `TEST_SAMPLING`).
pub(super) fn test_settings(cap: usize) -> Settings {
    Settings {
        cap,
        sampling: TEST_SAMPLING,
        nearest_steps: TEST_NEAREST_STEPS,
        ..Settings::default()
    }
}

/// A run with S2's production schedule and budget, for tests of its measurement.
fn s2_run() -> Run {
    Run {
        sampling: Schedule::PRODUCTION,
        nearest_steps: super::error::NEAREST_STEPS,
        ..Run::default()
    }
}

impl Default for Run {
    fn default() -> Self {
        Self {
            leaf_triangles: 256,
            cap: 300,
            jobs: 2,
            mutation: Mutation::None,
            budget: None,
            profile: LeafProfile::Balanced,
            sampling: TEST_SAMPLING,
            nearest_steps: TEST_NEAREST_STEPS,
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
        .with_hlod_settings(Settings {
            audit_dir: Some(std::sync::Arc::new(out.with_extension("hlod-audit"))),
            cap: run.cap,
            mutation: run.mutation,
            nearest_steps: run.nearest_steps,
            sampling: run.sampling,
        });
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
    check_errors(&expand_archive(output).0);
}

/// The full acceptance on one fixture: zero missing decoded segments/points at every
/// tile, the cap, valid representatives, and a
/// validated archive.
fn accept(name: &str, source: &[Mesh], run: Run) -> Census {
    accept_with(name, source, run, false).0
}

/// `accept` for a fixture with or without source T-junctions or unwelded seams. Exact seams
/// lock only shared points and segments, so where the source has such defects the census
/// may find gaps along them: they are reported (the census's overlap and perpendicular
/// counts beside the manifest's source-defect counts), not asserted away. Returns the census
/// and the manifest's incidence report.
fn accept_with(name: &str, source: &[Mesh], run: Run, defects: bool) -> (Census, Value) {
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
    assert_eq!(census.certificate_failures, 0, "{name}: {census:?}");
    assert_eq!(census.certificate_winding_failures, 0, "{name}: {census:?}");
    assert_eq!(census.max_same_depth_gap_m, 0., "{name}: {census:?}");
    assert_eq!(census.invalid_features, 0, "{name}: {census:?}");
    assert_eq!(census.moved_border_vertices, 0, "{name}: {census:?}");
    assert!(census.perpendicular_checked > 0, "{name}: {census:?}");
    if !defects {
        assert_eq!(census.missing_overlap_items, 0, "{name}: {census:?}");
        assert_eq!(census.certificate_failures, 0, "{name}: {census:?}");
        assert_eq!(census.certificate_winding_failures, 0, "{name}: {census:?}");
        assert_eq!(census.max_same_depth_gap_m, 0., "{name}: {census:?}");
    }
    assert_eq!(census.duplicate_chain_tiles, 0, "{name}: {census:?}");
    assert_eq!(census.reversed_coarse_faces, 0, "{name}: {census:?}");
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
    archive(&dir, &work.path().join("out.3tz"));
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    (census, manifest["hlod"]["incidence"].clone())
}

/// A fixture's source defects: reported by the producer (combinatorially) and the census
/// (geometrically), with the largest gap the census measured.
fn reported(name: &str, census: &Census, incidence: &Value) {
    eprintln!(
        "{name}: source open edges {} (with shared endpoints {}); census overlaps {}, \
         unkept overlap items {}, perpendicular displaced {} (max {} m)",
        incidence["sourceOpenEdges"],
        incidence["sourceOpenEdgesWithSharedEndpoints"],
        census.overlaps,
        census.missing_overlap_items,
        census.perpendicular_displaced,
        census.max_perpendicular_excess_m
    );
    assert!(census.overlaps > 0, "{name}: {census:?}");
    assert!(
        incidence["sourceOpenEdgesWithSharedEndpoints"]
            .as_u64()
            .unwrap()
            > 0,
        "{name}: {incidence}"
    );
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

/// Leaves are H8's, byte for byte, whatever the coarse stage does: a different triangle cap
/// (so different coarse tiles) leaves every leaf member and manifest row unchanged.
#[test]
fn leaves_do_not_depend_on_the_coarse_stage() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let a = stage(&input, &work.path().join("a"), &Run::default()).unwrap();
    let b = stage(
        &input,
        &work.path().join("b"),
        &Run {
            cap: 2000,
            ..Run::default()
        },
    )
    .unwrap();
    assert!(files(&a, "proxies") != files(&b, "proxies"));
    let mut leaves_a = files(&a, "leaves");
    leaves_a.extend(files(&a, "leaves.jsonl"));
    let mut leaves_b = files(&b, "leaves");
    leaves_b.extend(files(&b, "leaves.jsonl"));
    assert!(leaves_a == leaves_b, "leaves differ between coarse caps");
}

/// The client-order decoder HLOD checks its own bytes with (`decode`) reads every leaf's
/// meshopt streams and its 64-bit provenance columns exactly: the decoded `source_triangle`
/// rows name every source triangle exactly once.
#[test]
fn decoder_reads_leaf_provenance_columns_exactly() {
    let work = tempfile::tempdir().unwrap();
    let source = sheet();
    let input = write_gltf(&work.path().join("src"), &source);
    let dir = stage(&input, &work.path().join("hlod"), &Run::default()).unwrap();
    let mut seen = BTreeMap::<(u64, u64, u64), BTreeSet<u64>>::new();
    let mut faces = 0usize;
    for (uri, bytes) in files(&dir, "leaves") {
        let decoded = super::decode::decode(&bytes).unwrap();
        for face in &decoded.faces {
            let (id, table) = face.feature(&decoded.tables, "source_triangle").unwrap();
            let t = &decoded.tables[table];
            assert_eq!(t.components["triangle_index"], "UINT64", "{uri}");
            let column = |name: &str| t.columns[name][id as usize];
            let primitive = (
                column("node_index"),
                column("mesh_index"),
                column("primitive_index"),
            );
            assert!(seen
                .entry(primitive)
                .or_default()
                .insert(column("triangle_index")));
            faces += 1;
        }
    }
    let expected: Vec<usize> = source.iter().map(|m| m.indices.len() / 3).collect();
    assert_eq!(faces, expected.iter().sum::<usize>());
    assert_eq!(seen.len(), expected.len());
    for (triangles, count) in seen.values().zip(&expected) {
        assert!(triangles.iter().copied().eq(0..*count as u64));
    }
}

/// Owner decision (exact seams): a source T-junction is not a shared edge. Along the line the
/// right side has extra vertices on the left side's edges; only the coinciding vertices are
/// locked. The defect is reported by the producer (open source edges whose endpoints meet
/// other geometry) and its gap measured by the census, not hidden.
#[test]
fn t_junction_cracks_are_reported_not_hidden() {
    let (census, incidence) = accept_with("t-junction", &t_junction(), Run::default(), true);
    reported("t-junction", &census, &incidence);
}

/// Three- and four-cell corners: sheets of different, non-nested resolutions meet along two
/// border lines (both off every coarse lattice) and at their common corner, so T-junction
/// runs end at a point shared by three or four meshes. Flat along both borders (exactly
/// collinear once decoded), curved inside.
fn corners(four: bool) -> Vec<Mesh> {
    let c = T_LINE;
    let height = move |x: f32, z: f32| {
        let w = |v: f32| (std::f32::consts::PI * (v - c) / 50.).sin().powi(2);
        2. + 3. * w(x) * w(z)
    };
    let colour = [0.5, 0.5, 0.5, 1.];
    if four {
        vec![
            grid((10, 10), (0., c), (0., c), &height, colour),
            grid((20, 20), (c, 100.), (0., c), &height, colour),
            grid((15, 15), (0., c), (c, 100.), &height, colour),
            grid((40, 40), (c, 100.), (c, 100.), &height, colour),
        ]
    } else {
        vec![
            grid((10, 20), (0., c), (0., 100.), &height, colour),
            grid((25, 12), (c, 100.), (0., c), &height, colour),
            grid((16, 30), (c, 100.), (c, 100.), &height, colour),
        ]
    }
}

#[test]
fn three_and_four_cell_corners_report_their_t_junctions() {
    for (name, four) in [("three-cell corner", false), ("four-cell corner", true)] {
        let (census, incidence) = accept_with(name, &corners(four), Run::default(), true);
        reported(name, &census, &incidence);
    }
}

/// A long collinear run: many short slivers laid end to end along one line in one leaf,
/// one long border segment of a neighbouring sheet along the same line, and the run's
/// joints all T-junctions on it.
#[test]
fn long_collinear_runs_report_their_t_junctions() {
    let c = T_LINE;
    let flat = |_: f32, _: f32| 2.;
    let source = vec![
        grid((1, 40), (0., c), (0., 100.), &flat, [0.6, 0.5, 0.4, 1.]),
        grid((30, 400), (c, 100.), (0., 100.), &flat, [0.3, 0.6, 0.3, 1.]),
    ];
    let (census, incidence) = accept_with("collinear run", &source, Run::default(), true);
    reported("collinear run", &census, &incidence);
}

/// Round 4: 64 separate ribbons (16,384 triangles) and 65 long diagonal slivers that never
/// come near them. The slivers are long against every region they cross; testing them
/// against every ribbon point (as an aggregate once did) locked all 16,512 ribbon vertices
/// and refused the parent at the cap. The exact join locks nothing here.
fn ribbons(n: usize) -> Mesh {
    let mut ribbons = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.6, 0.6, 0.6, 1.],
    };
    for i in 0..=n {
        for j in 0..=n {
            ribbons.positions.push([
                -90. + 80. * i as f32 / n as f32,
                0.,
                10. + 80. * j as f32 / n as f32,
            ]);
        }
    }
    for i in 0..n {
        for j in (0..n).step_by(2) {
            let a = (i * (n + 1) + j) as u32;
            let b = a + n as u32 + 1;
            ribbons.indices.extend([a, a + 1, b, b, a + 1, b + 1]);
        }
    }
    ribbons
}

#[test]
fn distant_long_slivers_lock_nothing_and_never_refuse() {
    let ribbons = ribbons(128);
    let mut slivers = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.8, 0.3, 0.3, 1.],
    };
    for k in 0..65i32 {
        let off = (k - 32) as f32 * 0.02;
        let base = slivers.positions.len() as u32;
        slivers.positions.extend([
            [-100., 0., -100. + off],
            [100., 0., 100. + off],
            [100., 0., 100. + off + 0.005],
        ]);
        slivers.indices.extend([base, base + 1, base + 2]);
    }
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &[ribbons, slivers]);
    let run = Run {
        leaf_triangles: crate::mesh_large::config::CONSUMER_TABLE_ROWS,
        cap: super::TRIANGLE_CAP,
        jobs: 4,
        ..Run::default()
    };
    let dir =
        stage(&input, &work.path().join("hlod"), &run).unwrap_or_else(|f| panic!("{}", f.error));
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let incidence = &manifest["hlod"]["incidence"];
    // Exact seams: rows only for coincident items, at most one per item.
    assert!(
        incidence["rows"].as_u64().unwrap() <= incidence["items"].as_u64().unwrap(),
        "{incidence}"
    );
    let census = census(&dir);
    assert_eq!(
        census.missing_segments + census.missing_vertices + census.certificate_failures,
        0,
        "{census:?}"
    );
}

/// The owner's former fallback case: 2,048 random chord slivers spanning two ribbon blocks.
/// Exact seams do no geometric matching, so the chords cost the pass only their items (at
/// most six per triangle) and lock nothing they do not share. Never refused, crack-free, and
/// identical (counters included) at one and four jobs.
/// 2,048 random chord slivers spanning two ribbon blocks, written under `dir`.
fn chords_input(dir: &Path) -> PathBuf {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f32 / (1u64 << 53) as f32
    };
    let mut chords = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.8, 0.3, 0.3, 1.],
    };
    for _ in 0..2048 {
        let mut p = || [-90. + 180. * next(), -2. + 4. * next(), 10. + 80. * next()];
        let (a, b) = (p(), p());
        let base = chords.positions.len() as u32;
        chords.positions.extend([a, b, [b[0], b[1], b[2] + 0.005]]);
        chords.indices.extend([base, base + 1, base + 2]);
    }
    let left = ribbons(128);
    let mut right = left.clone();
    for p in &mut right.positions {
        p[0] += 100.;
    }
    write_gltf(dir, &[left, right, chords])
}

#[test]
fn random_chords_cost_linear_work_and_never_refuse() {
    let work = tempfile::tempdir().unwrap();
    let input = chords_input(&work.path().join("src"));
    let run = |jobs: usize, name: &str| {
        let run = Run {
            leaf_triangles: crate::mesh_large::config::CONSUMER_TABLE_ROWS,
            cap: super::TRIANGLE_CAP,
            jobs,
            ..Run::default()
        };
        stage(&input, &work.path().join(name), &run).unwrap_or_else(|f| panic!("{}", f.error))
    };
    let one = run(1, "one");
    let four = run(4, "four");
    let manifest = |dir: &Path| -> Value {
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap()
    };
    let (m1, m4) = (manifest(&one), manifest(&four));
    let incidence = &m1["hlod"]["incidence"];
    eprintln!("CHORDS {incidence}");
    let triangles = 2 * 128 * 64 * 2 + 2048;
    assert!(
        incidence["items"].as_u64().unwrap() <= 6 * triangles,
        "{incidence}"
    );
    assert!(
        incidence["rows"].as_u64().unwrap() <= incidence["items"].as_u64().unwrap(),
        "{incidence}"
    );
    assert_eq!(m1["hlod"], m4["hlod"]);
    assert_eq!(published(&one), published(&four));
    let census = census(&one);
    assert_eq!(
        census.missing_segments + census.missing_vertices + census.certificate_failures,
        0,
        "{census:?}"
    );
}

/// Round 4: what one parent job allocates (meshopt's temporary arrays included, routed
/// through the counting allocator) stays within the job's charge, on adversarial children:
/// a fully locked disconnected soup (every vertex distinct, refused at the cap), grids whose
/// required border segments are not point-locked (protected retries), and grids with no
/// interface (free simplification); and the encode and decode check of a cap-sized tile
/// within the job's encode allowance.
#[test]
fn parent_jobs_allocate_within_their_charge() {
    use super::{encode, geometry, interfaces::key, interfaces::InterfaceRow, store::*};
    crate::test_alloc::route_meshopt();
    let bounds = Bounds {
        min: [-100.; 3],
        max: [100.; 3],
    };
    let lattice = LatticePlan::new(Profile::Balanced, bounds, &[255., 127., 63.]).unwrap();
    let cap = super::TRIANGLE_CAP;
    let n = 8192usize;
    let parent = TileKey {
        first: 0,
        end: 8 * n as u64,
        depth: 1,
        leaf_begin: 1,
        leaf_end: 9,
    };
    let outer = |slot: usize, a: [f32; 3], b: [f32; 3]| {
        InterfaceRow::new(
            slot as u32 + 1,
            key(a).min(key(b)),
            key(a).max(key(b)),
            0,
            slot as u32 + 1,
        )
    };
    let primitive = |t: usize| SourcePrimitive {
        node: 0,
        mesh: 0,
        primitive: t as u32,
        expanded_ordinal: t as u64,
        material: u32::MAX,
    };
    let child = |slot: usize, kind: u8| -> TileGeometry {
        let origin = [
            -90. + 90. * (slot & 1) as f32,
            -90. + 90. * ((slot >> 1) & 1) as f32,
            -90. + 90. * ((slot >> 2) & 1) as f32,
        ];
        let mut tile = TileGeometry {
            key: TileKey {
                first: (slot * n) as u64,
                end: ((slot + 1) * n) as u64,
                depth: 2,
                leaf_begin: slot as u32 + 1,
                leaf_end: slot as u32 + 2,
            },
            ells: Vec::new(),
            positions: Vec::new(),
            triangles: Vec::new(),
            representative: Vec::new(),
            primitives: Vec::new(),
            interface: Vec::new(),
            geometric_error: 0.,
            lease: None,
        };
        let at = |x: f32, z: f32, y: f32| {
            lattice
                .canonical([origin[0] + x, origin[1] + y, origin[2] + z], 2)
                .unwrap()
        };
        if kind == 0 || kind == 3 {
            // Disconnected soup, every segment required; point-locked too (kind 0, refused
            // at the cap), or not (kind 3: simplification deletes segments and retries).
            for t in 0..n {
                let (x, z) = ((t % 64) as f32 * 1.25, (t / 64) as f32 * 0.6);
                let c = [at(x, z, 0.), at(x + 1., z, 0.1), at(x, z + 0.5, 0.2)];
                let base = tile.positions.len() as u32;
                tile.positions.extend(c);
                tile.ells.extend([2; 3]);
                tile.triangles.push([base, base + 1, base + 2]);
                for k in 0..3 {
                    if kind == 0 {
                        tile.interface.push(outer(slot, c[k], c[k]));
                    }
                    tile.interface.push(outer(slot, c[k], c[(k + 1) % 3]));
                }
            }
        } else {
            // A 64 x 64 grid of quads (8,192 triangles), gently curved.
            let w = 65;
            for i in 0..w {
                for j in 0..w {
                    let (x, z) = (i as f32 * 1.25, j as f32 * 1.25);
                    tile.positions
                        .push(at(x, z, 0.02 * ((x * 0.3).sin() + (z * 0.2).cos())));
                    tile.ells.push(2);
                }
            }
            for i in 0..w - 1 {
                for j in 0..w - 1 {
                    let v = |a: usize, b: usize| (a * w + b) as u32;
                    tile.triangles.push([v(i, j), v(i, j + 1), v(i + 1, j)]);
                    tile.triangles
                        .push([v(i + 1, j), v(i, j + 1), v(i + 1, j + 1)]);
                }
            }
            if kind == 1 {
                // Required border segments without point locks: simplification deletes some
                // and the job protects and retries.
                for i in 0..w - 1 {
                    for (a, b) in [
                        ((i, 0), (i + 1, 0)),
                        ((i, w - 1), (i + 1, w - 1)),
                        ((0, i), (0, i + 1)),
                        ((w - 1, i), (w - 1, i + 1)),
                    ] {
                        let p = |(x, z): (usize, usize)| tile.positions[x * w + z];
                        tile.interface.push(outer(slot, p(a), p(b)));
                    }
                }
            }
        }
        tile.primitives = (0..tile.triangles.len()).map(primitive).collect();
        tile.representative = (0..tile.triangles.len() as u32).collect();
        tile.interface.sort_unstable_by_key(|r| (r.a, r.b));
        tile.interface.dedup_by_key(|r| (r.a, r.b));
        tile
    };
    let merged = (8 * n) as u64;
    let charge = merged * super::JOB_BYTES_PER_TRIANGLE;
    let mut grid = None;
    for kind in [0u8, 1, 3, 2] {
        let children: Vec<TileGeometry> = (0..8).map(|slot| child(slot, kind)).collect();
        let (built, peak) = crate::test_alloc::peak(|| {
            let mut interface = children
                .iter()
                .flat_map(|c| c.interface.iter())
                .filter(|r| r.outer(parent.leaf_begin, parent.leaf_end))
                .copied()
                .collect();
            super::interfaces::merge_rows(&mut interface);
            geometry::build_parent(
                parent,
                &children,
                &[cap],
                &super::borders::Contract {
                    depth: parent.depth,
                    interface,
                    certificates: Vec::new(),
                    terminal_vertices: Vec::new(),
                    scale: 0.,
                },
                &|_| Ok(None),
                &mut |_| Ok(true),
                &|| Ok(()),
            )
        });
        eprintln!(
            "JOB kind {kind}: peak {peak} bytes = {:.0} per merged triangle (charge {})",
            peak as f64 / merged as f64,
            super::JOB_BYTES_PER_TRIANGLE
        );
        assert!(peak as u64 <= charge, "kind {kind}: {peak} > {charge}");
        match (kind, built) {
            // Its whole interface cannot fit the cap: omitted (nothing published or held),
            // never refused.
            (0, built) => assert!(built.unwrap().built.is_none()),
            (2, built) => grid = Some(built.unwrap().built.unwrap()),
            // The retries ran (and fitted, or ran out and omitted it).
            (_, Ok(b)) => assert!(b.built.is_none_or(|b| b.retries > 0), "kind {kind}"),
            (_, Err(e)) => panic!("kind {kind}: {e}"),
        }
    }

    let built = grid.unwrap();
    let (checked, peak) = crate::test_alloc::peak(|| {
        let g = &built.geometry;
        let fixed: Vec<bool> = encode::open_border(&g.triangles, g.positions.len())
            .iter()
            .zip(&built.locked)
            .map(|(b, l)| *b || *l)
            .collect();
        let positions = lattice.encode_coarse(&g.positions, &g.ells, &fixed, g.key.depth)?;
        let encoded = encode::encode(
            g,
            &built.locked,
            positions,
            &plain_appearance(g),
            Mutation::None,
        )?;
        encode::check(g, &built.locked, &encoded)
    });
    checked.unwrap();
    eprintln!(
        "JOB encode+check peak {peak} (allowance {})",
        super::JOB_ENCODE_BYTES
    );
    assert!(peak as u64 <= super::JOB_ENCODE_BYTES, "{peak}");
}

/// The reviewer's encoder probe: steps 2^-7, 2^-8, 2^-9; a depth-1 coarse tile whose border
/// runs along x = 50 + 2^-9 with locked endpoints and an unmatched midpoint. Snapped to the
/// depth-1 lattice the midpoint would decode at x = 50 (a 2^-9 m gap); on the open border
/// it must keep its leaf-canonical decode, and the pre-publication check verifies that.
#[test]
fn border_midpoint_keeps_its_canonical_decode_regression() {
    use super::{encode, interfaces::InterfaceRow, store::*};
    let bounds = Bounds {
        min: [-100.; 3],
        max: [100.; 3],
    };
    let lattice = LatticePlan::new(Profile::Balanced, bounds, &[255., 127., 63.]).unwrap();
    assert_eq!(lattice.steps(), &[0.0078125, 0.00390625, 0.001953125]);
    let x = T_LINE;
    let (a, b, m, c) = ([x, 0., 0.], [x, 0., 10.], [x, 0., 5.], [55., 0., 5.]);
    // Without the border rule the coarse lattice moves the midpoint off the line.
    let snapped = lattice
        .encode_coarse(&[a, b, m, c], &[2; 4], &[true, true, false, false], 1)
        .unwrap();
    assert_eq!(snapped.decoded[2][0], 50., "probe must be sensitive");
    let key = super::interfaces::key;
    let row = |p: [f32; 3]| InterfaceRow::new(0, key(p), key(p), 0, 1);
    let tile = TileGeometry {
        key: TileKey {
            first: 0,
            end: 2,
            depth: 1,
            leaf_begin: 0,
            leaf_end: 1,
        },
        ells: vec![2; 4],
        positions: vec![a, b, m, c],
        triangles: vec![[0, 2, 3], [2, 1, 3]],
        representative: vec![0, 0],
        primitives: vec![SourcePrimitive {
            node: 0,
            mesh: 0,
            primitive: 0,
            expanded_ordinal: 0,
            material: u32::MAX,
        }],
        interface: vec![row(a), row(b)],
        geometric_error: 0.,
        lease: None,
    };
    let locked = [true, true, false, false];
    let appearance = plain_appearance(&tile);
    let fixed: Vec<bool> = encode::open_border(&tile.triangles, tile.positions.len())
        .iter()
        .zip(&locked)
        .map(|(b, l)| *b || *l)
        .collect();
    let positions = lattice
        .encode_coarse(&tile.positions, &tile.ells, &fixed, tile.key.depth)
        .unwrap();
    let encoded = encode::encode(&tile, &locked, positions, &appearance, Mutation::None).unwrap();
    encode::check(&tile, &locked, &encoded).unwrap();
    let decoded = decode(&encoded.glb);
    let mut seen = 0;
    for face in &decoded.faces {
        for p in face {
            if p[2] == 5. && p[0] != 55. {
                assert_eq!(p[0], x, "border midpoint decoded off the line");
                seen += 1;
            }
        }
    }
    assert_eq!(seen, 2);
}

/// A small sheet in one corner of a large frame: every cell above it holds one occupied
/// child, a pass-through chain. Each such cell publishes no content (the child stands for
/// it), so no two tiles on a root-to-leaf path are the same, the census stays crack-free,
/// and the public archive delivers the sparse coarse rows as content-less tiles.
#[test]
fn single_child_chains_publish_their_content_once() {
    let source = vec![
        grid(
            (24, 24),
            (0., 3.),
            (0., 3.),
            &|x, z| 0.05 * (x * z).sin(),
            [0.6, 0.5, 0.4, 1.],
        ),
        Mesh {
            positions: vec![[4000., 0., 4000.], [4001., 0., 4000.], [4000., 0., 4001.]],
            indices: vec![0, 2, 1],
            color: [0.5, 0.5, 0.5, 1.],
        },
    ];
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &source);
    let run = s2_run();
    let dir = stage(&input, &work.path().join("hlod"), &run).unwrap();
    let census = census(&dir);
    assert_eq!(census.duplicate_chain_tiles, 0, "{census:?}");
    assert_eq!(census.reversed_coarse_faces, 0, "{census:?}");
    assert_eq!(
        census.missing_segments + census.missing_vertices + census.certificate_failures,
        0,
        "{census:?}"
    );
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let passed = manifest["hlod"]["passThrough"].as_u64().unwrap();
    assert!(passed >= 3, "{}", manifest["hlod"]);
    assert!(census.coarse >= 1, "{census:?}");
    // S2 measures each encoded tile once: seeds and depth schedules follow its encoded
    // depth, not the top of the chain it is published at.
    let proxies = rows(&dir.join("proxies.jsonl"));
    assert!(proxies.iter().any(|r| r["encodedDepth"] != r["depth"]));
    let mut samples = [0u64; 32];
    for row in &proxies {
        let depth = row["encodedDepth"].as_u64().unwrap() as usize;
        let measurement = &row["measurement"];
        samples[depth] += measurement["areaSamplesPerDirection"].as_u64().unwrap();
        let key = super::store::TileKey {
            first: row["first"].as_u64().unwrap(),
            end: row["end"].as_u64().unwrap(),
            depth: depth as u32,
            leaf_begin: 0,
            leaf_end: 0,
        };
        assert_eq!(
            measurement["seed"].as_u64().unwrap(),
            super::error::seed(key)
        );
    }
    for (depth, n) in samples.iter().enumerate().filter(|(_, n)| **n > 0) {
        assert!(*n >= super::error::MIN_DEPTH);
        let summary = &manifest["hlod"]["errorMeasurement"]["byDepth"][depth];
        assert_eq!(summary["sourceAreaSamples"].as_u64(), Some(*n));
    }
    let archive = work.path().join("chain.3tz");
    let result = public(&input, &archive, None, 2).unwrap();
    assert!(result.diagnostics.hlod.tiles >= 1);
    // The chain cells are spliced out of the explicit hierarchy: no content-less tile below
    // content (Cesium would drop the coarse tile there before its replacements load), and
    // the coarse tile's children are its deeper renderable replacements.
    let shape = refinement_shape(&archive);
    assert_eq!(
        shape.empty_below_content + shape.error_inversions,
        0,
        "{shape:?}"
    );
    assert!(shape.skipped_levels > 0, "{shape:?}");
}

/// Where the layout cannot skip a cell (implicit trees and document roots, forced here by
/// tight fragment caps), a chain cell repeats its coarse tile instead of publishing empty;
/// under the default caps every chain cell is spliced out and nothing repeats.
#[test]
fn chain_cells_repeat_coarse_content_where_they_cannot_be_spliced() {
    use crate::mesh_large::delivery::{self, plan::Limits};
    use sha2::{Digest, Sha256};
    let source = vec![
        grid(
            (24, 24),
            (0., 3.),
            (0., 3.),
            &|x, z| 0.05 * (x * z).sin(),
            [0.6, 0.5, 0.4, 1.],
        ),
        Mesh {
            positions: vec![[4000., 0., 4000.], [4001., 0., 4000.], [4000., 0., 4001.]],
            indices: vec![0, 2, 1],
            color: [0.5, 0.5, 0.5, 1.],
        },
    ];
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &source);
    let dir = stage(&input, &work.path().join("hlod"), &Run::default()).unwrap();
    let repeats = |archive: &Path| {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
        let mut seen: HashMap<Vec<u8>, usize> = HashMap::new();
        for i in 0..zip.len() {
            let mut bytes = Vec::new();
            zip.by_index(i).unwrap().read_to_end(&mut bytes).unwrap();
            if bytes.starts_with(b"glTF") {
                *seen.entry(Sha256::digest(&bytes).to_vec()).or_default() += 1;
            }
        }
        seen.values().filter(|n| **n > 1).count()
    };
    let publish = |name: &str, limits: Limits| {
        let output = work.path().join(name);
        let mut file = fs::File::create(&output).unwrap();
        let (report, _) = delivery::publish_with_limits(
            &MeshTilesetRequest::new(&dir, &output),
            limits,
            &mut file,
            work.path(),
        )
        .unwrap();
        drop(file);
        let checked = inspect(ValidationRequest::new(&output)).unwrap();
        assert!(checked
            .checks
            .iter()
            .any(|c| c == "decodedContentContainment"));
        check_errors(&expand_archive(&output).0);
        (output, report)
    };
    let (spliced, report) = publish("spliced.3tz", delivery::plan::LIMITS);
    assert_eq!(repeats(&spliced), 0, "{report:?}");
    assert_eq!(report.implicit_roots, 0, "{report:?}");
    assert_eq!(refinement_shape(&spliced).empty_below_content, 0);
    let tight = Limits {
        objects: 3,
        records: 6,
        levels: 2,
    };
    let (repeated, report) = publish("repeated.3tz", tight);
    assert!(report.implicit_roots + report.documents > 1, "{report:?}");
    assert!(repeats(&repeated) > 0, "{report:?}");
    let shape = refinement_shape(&repeated);
    assert_eq!(
        shape.empty_below_content + shape.error_inversions,
        0,
        "{shape:?}"
    );
}

/// The published hierarchy's refinement structure, over every document of an archive.
#[derive(Debug, Default)]
struct RefinementShape {
    tiles: usize,
    /// Tiles without content (neither geometry nor a document link nor implicit tiling)
    /// below a tile with content.
    empty_below_content: usize,
    /// Explicit parent/child pairs more than one octree level apart (spliced chains).
    skipped_levels: usize,
    /// Child geometric error above its parent's.
    error_inversions: usize,
}

fn refinement_shape(archive: &Path) -> RefinementShape {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let mut docs: HashMap<String, Value> = HashMap::new();
    for i in 0..zip.len() {
        let mut member = zip.by_index(i).unwrap();
        let name = member.name().to_owned();
        if name.ends_with(".json") {
            let mut bytes = Vec::new();
            member.read_to_end(&mut bytes).unwrap();
            if let Ok(doc) = serde_json::from_slice::<Value>(&bytes) {
                docs.insert(name, doc);
            }
        }
    }
    fn visit(
        tile: &Value,
        docs: &HashMap<String, Value>,
        above: bool,
        parent: Option<(u64, f64)>,
        shape: &mut RefinementShape,
    ) {
        shape.tiles += 1;
        let uris: Vec<&str> = tile["content"]["uri"]
            .as_str()
            .into_iter()
            .chain(
                tile["contents"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c["uri"].as_str()),
            )
            .collect();
        let link = uris.iter().find(|u| u.ends_with(".json"));
        let geometry = uris.iter().any(|u| !u.ends_with(".json"));
        let implicit = tile.get("implicitTiling").is_some();
        let error = tile["geometricError"].as_f64().unwrap();
        let depth = tile["extras"]["depth"].as_u64();
        if let Some((parent_depth, parent_error)) = parent {
            if depth.is_some_and(|d| d > parent_depth + 1) {
                shape.skipped_levels += 1;
            }
            shape.error_inversions += usize::from(error > parent_error);
        }
        if above && !geometry && link.is_none() && !implicit {
            shape.empty_below_content += 1;
        }
        let here = depth.map(|d| (d, error));
        if let Some(uri) = link {
            let doc = &docs[*uri];
            visit(&doc["root"], docs, above || geometry, parent, shape);
        }
        for child in tile["children"].as_array().into_iter().flatten() {
            visit(child, docs, above || geometry, here, shape);
        }
    }
    let mut shape = RefinementShape::default();
    visit(
        &docs["tileset.json"]["root"],
        &docs,
        false,
        None,
        &mut shape,
    );
    shape
}

/// The adversarial interface inputs (sub-lattice micro geometry meeting long frame edges, a
/// duplicated triangle soup with every point and segment shared) run within a small
/// declared budget: the walk's leases stay within its ledger, and the overlap join's tests
/// and rows stay within their by-design bounds.
#[test]
fn adversarial_interfaces_stay_within_a_small_budget() {
    let mut soup = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.7, 0.7, 0.7, 1.],
    };
    for i in 0..40u32 {
        for j in 0..40u32 {
            let (x, z) = (i as f32 * 1.5, j as f32 * 1.5);
            let base = soup.positions.len() as u32;
            soup.positions
                .extend([[x, 0., z], [x + 1., 0., z], [x, 0., z + 1.]]);
            soup.indices.extend([base, base + 1, base + 2]);
        }
    }
    let cases = [
        ("cluster", cluster(), 512),
        ("soup", vec![soup.clone(), soup], 256),
    ];
    for (name, source, leaf_triangles) in cases {
        let work = tempfile::tempdir().unwrap();
        let input = write_gltf(&work.path().join("src"), &source);
        let dir = stage(
            &input,
            &work.path().join("hlod"),
            &Run {
                leaf_triangles,
                cap: super::TRIANGLE_CAP,
                budget: Some(2 << 30),
                ..Run::default()
            },
        )
        .unwrap_or_else(|f| panic!("{name}: {}", f.error));
        let resources: Value =
            serde_json::from_slice(&fs::read(dir.join("resources.json")).unwrap()).unwrap();
        let d = &resources["hlod"]["diagnostics"];
        let (peak, ledger) = (
            d["lease_peak_bytes"].as_u64().unwrap(),
            d["ledger_bytes"].as_u64().unwrap(),
        );
        assert!(peak <= ledger, "{name}: {d}");
        let manifest: Value =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let inc = &manifest["hlod"]["incidence"];
        eprintln!("ADVERSARIAL {name}: {inc}");
        assert!(
            inc["rows"].as_u64().unwrap() <= inc["items"].as_u64().unwrap(),
            "{name}: {inc}"
        );
        let census = census(&dir);
        assert_eq!(
            census.missing_segments + census.missing_vertices + census.certificate_failures,
            0,
            "{name}: {census:?}"
        );
    }
}

/// The round-2 dense fixture: 2,100 long slivers packed in one block plus a connected
/// grid (9,300 triangles, two leaves). Their long open edges crowd one region far past the
/// exact leaf size; the join splits them apart instead of refusing.
#[test]
fn dense_sliver_borders_are_not_refused() {
    let mut slivers = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.8, 0.4, 0.4, 1.],
    };
    for i in 0..2100u32 {
        let (x, y) = (2. + (i % 50) as f32 * 0.2, 1. + (i / 50) as f32 * 0.2);
        let base = slivers.positions.len() as u32;
        slivers
            .positions
            .extend([[x, y, 2.], [x + 0.05, y, 2.], [x + 0.02, y, 12.]]);
        slivers.indices.extend([base, base + 1, base + 2]);
    }
    let source = vec![
        grid((60, 60), (20., 80.), (0., 60.), &bump, [0.4, 0.6, 0.4, 1.]),
        slivers,
    ];
    let run = Run {
        leaf_triangles: crate::mesh_large::config::CONSUMER_TABLE_ROWS,
        cap: super::TRIANGLE_CAP,
        ..Run::default()
    };
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &source);
    let dir =
        stage(&input, &work.path().join("hlod"), &run).unwrap_or_else(|f| panic!("{}", f.error));
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let incidence = &manifest["hlod"]["incidence"];
    assert!(
        incidence["rows"].as_u64().unwrap() <= incidence["items"].as_u64().unwrap(),
        "{incidence}"
    );
    let census = census(&dir);
    assert!(census.leaves >= 2, "{census:?}");
    assert_eq!(
        census.missing_segments
            + census.missing_vertices
            + census.missing_overlap_items
            + census.perpendicular_displaced
            + census.duplicate_chain_tiles
            + census.reversed_coarse_faces,
        0,
        "{census:?}"
    );
}

/// Many leaves share one point (a fan of thin triangles about a common apex): its item
/// group holds one item per leaf, within the stage's charged bound, and stays crack-free.
#[test]
fn a_point_shared_by_every_leaf_is_charged_and_crack_free() {
    let mut fan = Mesh {
        positions: vec![[50., 1., 50.]],
        indices: vec![],
        color: [0.6, 0.6, 0.6, 1.],
    };
    let n = 1200u32;
    for i in 0..=n {
        let a = std::f32::consts::TAU * i as f32 / n as f32;
        fan.positions
            .push([50. + 40. * a.cos(), 0., 50. + 40. * a.sin()]);
    }
    for i in 0..n {
        fan.indices.extend([0, i + 2, i + 1]);
    }
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &[fan]);
    let dir = stage(
        &input,
        &work.path().join("hlod"),
        &Run {
            leaf_triangles: 8,
            budget: Some(1 << 30),
            ..Run::default()
        },
    )
    .unwrap_or_else(|f| panic!("{}", f.error));
    let census = census(&dir);
    assert!(census.leaves >= 100, "{census:?}");
    assert_eq!(
        census.missing_segments + census.missing_vertices + census.certificate_failures,
        0,
        "{census:?}"
    );
}

/// The census's segment index holds at most eight entries per segment whatever the length
/// mix (a uniform grid sized to short segments explodes on long ones).
#[test]
fn segment_index_is_bounded_for_mixed_lengths() {
    let mut segments = Vec::new();
    for i in 0..2000 {
        let x = i as f64 * 5e-7;
        segments.push(([x, 0., 0.], [x + 5e-7, 0., 0.]));
    }
    segments.push(([-500., -1., -500.], [500., 2., 500.]));
    let n = segments.len();
    let index = SegmentIndex::new(segments, 0.);
    assert!(index.entries() <= 8 * n, "{}", index.entries());
    assert!(index.near([1e-4, 0., 0.]).count() >= 1);
}

/// The adversarial held tile: 8,192 disconnected triangles whose every point and segment
/// is shared (a duplicate in another leaf), each with its own source primitive. Its actual
/// footprint at exact capacity stays within the proven bound the walk admits.
#[test]
fn adversarial_held_tile_fits_its_admitted_charge() {
    use super::{interfaces::InterfaceRow, store::*};
    let n = 8192usize;
    let mut tile = TileGeometry {
        key: TileKey {
            first: 0,
            end: n as u64,
            depth: 3,
            leaf_begin: 0,
            leaf_end: 1,
        },
        ells: Vec::with_capacity(3 * n),
        positions: Vec::with_capacity(3 * n),
        triangles: Vec::with_capacity(n),
        representative: Vec::with_capacity(n),
        primitives: Vec::with_capacity(n),
        interface: Vec::with_capacity(6 * n),
        geometric_error: 0.,
        lease: None,
    };
    for t in 0..n {
        let base = tile.positions.len() as u32;
        let corners = [0f32, 1., 2.].map(|k| [t as f32, k, 0.]);
        tile.positions.extend(corners);
        tile.ells.extend([3; 3]);
        tile.triangles.push([base, base + 1, base + 2]);
        tile.representative.push(t as u32);
        tile.primitives.push(SourcePrimitive {
            node: 0,
            mesh: 0,
            primitive: t as u32,
            expanded_ordinal: t as u64,
            material: 0,
        });
        let k = corners.map(super::interfaces::key);
        for i in 0..3 {
            let (p, q) = (k[i], k[(i + 1) % 3]);
            for (a, b) in [(p, p), (p.min(q), p.max(q))] {
                tile.interface.push(InterfaceRow::new(0, a, b, 0, 1));
            }
        }
    }
    let bytes = tile.bytes();
    assert!(bytes >= 2_220_032, "{bytes}");
    assert!(
        bytes <= TileGeometry::charge(n as u64),
        "{bytes} > {}",
        TileGeometry::charge(n as u64)
    );
    tile.check_charge().unwrap();
}

/// A duplicated triangle soup: every point and segment shared across leaves, the worst
/// case for held interface rows, runs within its admitted charges and stays crack-free.
#[test]
fn duplicated_soup_is_admitted_and_crack_free() {
    let mut soup = Mesh {
        positions: vec![],
        indices: vec![],
        color: [0.7, 0.7, 0.7, 1.],
    };
    for i in 0..60u32 {
        for j in 0..60u32 {
            let (x, z) = (i as f32 * 1.5, j as f32 * 1.5);
            let base = soup.positions.len() as u32;
            let y = bump(x, z);
            soup.positions
                .extend([[x, y, z], [x + 1., y, z], [x, y, z + 1.]]);
            soup.indices.extend([base, base + 1, base + 2]);
        }
    }
    let source = [soup.clone(), soup];
    // Every coarse tile is the union of its children here (cap above the input), so the
    // held charges of the leaves (every point and segment held twice) are what is exercised.
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &source);
    let dir = stage(
        &input,
        &work.path().join("union"),
        &Run {
            cap: 8192,
            ..Run::default()
        },
    )
    .unwrap();
    let census = census(&dir);
    assert!(census.coarse >= 3, "{census:?}");
    assert_eq!(
        census.missing_segments + census.missing_vertices + census.moved_border_vertices,
        0,
        "{census:?}"
    );
    // D11 permits dropping small wholly interior components. These formerly omitted
    // ancestors now fit; S2 measures the removed surface and the receipt counts it.
    let dir = stage(&input, &work.path().join("capped"), &Run::default()).unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["hlod"]["omittedProxies"], 0);
    let dropped: u64 = manifest["hlod"]["bordersByDepth"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["topology"]["droppedComponents"].as_u64().unwrap())
        .sum();
    assert!(dropped > 0);
    for row in rows(&dir.join("proxies.jsonl")) {
        if row["topology"]["droppedComponents"].as_u64().unwrap() > 0 {
            assert!(
                row["geometricError"].as_f64().unwrap()
                    >= row["topologyErrorFloor"].as_f64().unwrap()
            );
            assert!(row["geometricError"].as_f64().unwrap() > 1.);
        }
    }
    // Sol round 8: nothing compounds; every job merges at most eight capped children.
    assert_bounded_jobs(&dir, 300);
    for row in rows(&dir.join("proxies.jsonl")) {
        assert!(row["triangles"].as_u64().unwrap() <= 300, "{row}");
    }
    cover(&dir);
    let capped = super::tests::census(&dir);
    assert_eq!(
        capped.missing_segments + capped.missing_vertices + capped.reversed_coarse_faces,
        0,
        "{capped:?}"
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
    let (census, incidence) = accept_with(
        "cluster",
        &cluster(),
        // Its sub-lattice micro geometry decodes onto many exactly collinear lattice
        // points, all kept by the overlap rule: it runs at the production cap.
        Run {
            leaf_triangles: 512,
            cap: super::TRIANGLE_CAP,
            ..Run::default()
        },
        true,
    );
    // Leaf quantization creates collinear partial overlaps here. D11 does not
    // tolerance-match those source defects; keep their complete census visible.
    reported("cluster", &census, &incidence);
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

/// S5 round 5: a thin two-sided plate (an up-facing sheet a millimetre above a down-facing
/// one, joined along one edge: a fold) simplifies across the fold, where a collapsed face
/// can land on an opposite-facing source face. Every coarse face keeps its source
/// orientation (the census's exact reversal check, independent of the producer's).
#[test]
fn back_to_back_sheets_keep_their_source_orientation() {
    let top = grid(
        (40, 80),
        (0., 100.),
        (0., 100.),
        &|x, z| bump(x, z) + 1e-3 * (x / 100.),
        [0.6, 0.5, 0.4, 1.],
    );
    let mut bottom = grid((40, 80), (0., 100.), (0., 100.), &bump, [0.3, 0.6, 0.3, 1.]);
    for t in bottom.indices.chunks_mut(3) {
        t.swap(1, 2);
    }
    let census = accept("back-to-back plate", &[top, bottom], Run::default());
    assert_eq!(census.reversed_coarse_faces, 0, "{census:?}");
}

/// Mirrored nodes (negative-determinant scale) face where their leaves face at every depth:
/// capture reverses their corners once, and coarse tiles inherit them. Every coarse face of
/// a bumpy sheet points to the side its leaves' faces point to on average, and mirroring the
/// sheet turns that side over.
#[test]
fn mirrored_nodes_keep_their_orientation_in_coarse_tiles() {
    let normal = |f: &[Point; 3]| {
        let p = f.map(|v| v.map(|b| f64::from(f32::from_bits(b))));
        let (u, w): ([f64; 3], [f64; 3]) = (
            std::array::from_fn(|i| p[1][i] - p[0][i]),
            std::array::from_fn(|i| p[2][i] - p[0][i]),
        );
        [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| (0..3).map(|i| a[i] * b[i]).sum::<f64>();
    let mut sides = Vec::new();
    for mirrored in [false, true] {
        let work = tempfile::tempdir().unwrap();
        let mut meshes = sheet();
        let mut scales = Vec::new();
        if mirrored {
            for m in &mut meshes {
                for p in &mut m.positions {
                    p[0] = -p[0];
                }
                scales.push([-1., 1., 1.]);
            }
        }
        let input = write_gltf_scaled(&work.path().join("src"), &meshes, &scales);
        let dir = stage(&input, &work.path().join("hlod"), &Run::default()).unwrap();
        let census = census(&dir);
        assert_eq!(
            census.missing_segments + census.reversed_coarse_faces,
            0,
            "{census:?}"
        );
        let tiles = load(&dir);
        let mut side = [0f64; 3];
        for f in tiles.iter().filter(|t| t.leaf).flat_map(|t| &t.faces) {
            let n = normal(f);
            side = std::array::from_fn(|i| side[i] + n[i]);
        }
        let coarse: Vec<&[Point; 3]> = tiles
            .iter()
            .filter(|t| !t.leaf)
            .flat_map(|t| &t.faces)
            .filter(|f| !degenerate(f))
            .collect();
        assert!(!coarse.is_empty());
        let against = coarse.iter().filter(|f| dot(normal(f), side) <= 0.).count();
        assert_eq!(
            against,
            0,
            "mirrored {mirrored}: {against} of {}",
            coarse.len()
        );
        sides.push(side);
    }
    assert!(dot(sides[0], sides[1]) < 0., "{sides:?}");
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
        census.missing_segments > 0
            && census.missing_vertices > 0
            && census.perpendicular_displaced > 0,
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

/// The coarsest published cover: each leaf's shallowest published tile. Its tiles' leaf
/// ranges must cover every leaf exactly once (no gap, no double coverage); returns their
/// count and bytes (what a first view of the whole model loads).
fn cover(dir: &Path) -> (usize, u64) {
    let tiles = load(dir);
    let leaves = tiles.iter().filter(|t| t.leaf).count();
    let mut chosen: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for i in 0..leaves {
        let t = tiles
            .iter()
            .filter(|t| t.begin <= i && i < t.end)
            .min_by_key(|t| t.depth)
            .unwrap();
        chosen.insert(t.uri.clone(), (t.begin, t.end));
    }
    let mut ranges: Vec<(usize, usize)> = chosen.values().copied().collect();
    ranges.sort_unstable();
    let mut next = 0;
    for (begin, end) in &ranges {
        assert_eq!(*begin, next, "cover gap or double coverage: {ranges:?}");
        next = *end;
    }
    assert_eq!(next, leaves);
    let bytes = chosen
        .keys()
        .map(|u| fs::metadata(dir.join(u)).unwrap().len())
        .sum();
    (chosen.len(), bytes)
}

/// Bounded jobs: every parent job merges at most eight capped children.
fn assert_bounded_jobs(dir: &Path, cap: u64) {
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let leaf_cap = rows(&dir.join("leaves.jsonl"))
        .iter()
        .map(|r| r["end"].as_u64().unwrap() - r["first"].as_u64().unwrap())
        .max()
        .unwrap();
    let input = manifest["hlod"]["maxJobInputTriangles"].as_u64().unwrap();
    let bound = 8 * leaf_cap.max(cap);
    assert!(input > 0 && input <= bound, "{input} > {bound}");
}

/// Round 8 (owner): an exact interface that cannot fit the cap omits that cell's coarse tile
/// and, by upward propagation, every ancestor's: omitted cells publish no content and hold
/// nothing, so no job merges more than eight capped children and no content-less cell has a
/// content ancestor (Cesium's empty traversal never meets one below content). The capped
/// tiles below the omitted cap cover every leaf exactly once; the first view loads them.
#[test]
fn a_parent_whose_interface_cannot_fit_is_omitted_not_refused() {
    use crate::mesh_large::delivery::{self, plan::Limits};
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let dir = stage(
        &input,
        &work.path().join("capped"),
        &Run {
            cap: 12,
            ..Run::default()
        },
    )
    .unwrap_or_else(|f| panic!("{}", f.error));
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let hlod = &manifest["hlod"];
    let omitted = hlod["omittedProxies"].as_u64().unwrap();
    assert!(omitted > 0, "{hlod}");
    let by_depth: Vec<u64> = serde_json::from_value(hlod["omittedByDepth"].clone()).unwrap();
    assert_eq!(by_depth.iter().sum::<u64>(), omitted);
    // Omission reaches the root.
    assert_eq!(by_depth[0], 1, "{hlod}");
    let sources = hlod["omissionSources"].as_u64().unwrap();
    assert!(sources > 0 && sources <= omitted, "{hlod}");
    assert_bounded_jobs(&dir, 12);
    for row in rows(&dir.join("proxies.jsonl")) {
        assert!(row["triangles"].as_u64().unwrap() <= 12, "{row}");
    }
    let census = census(&dir);
    assert_eq!(
        census.missing_segments + census.missing_vertices + census.reversed_coarse_faces,
        0,
        "{census:?}"
    );
    for (name, limits) in [
        ("default.3tz", delivery::plan::LIMITS),
        (
            "tight.3tz",
            Limits {
                objects: 3,
                records: 6,
                levels: 2,
            },
        ),
    ] {
        let output = work.path().join(name);
        let mut file = fs::File::create(&output).unwrap();
        let (report, _) = delivery::publish_with_limits(
            &MeshTilesetRequest::new(&dir, &output),
            limits,
            &mut file,
            work.path(),
        )
        .unwrap();
        drop(file);
        inspect(ValidationRequest::new(&output)).unwrap();
        let shape = refinement_shape(&output);
        assert_eq!(
            shape.empty_below_content + shape.error_inversions,
            0,
            "{name}: {shape:?} {report:?}"
        );
    }
    let (tiles, bytes) = cover(&dir);
    eprintln!(
        "OMITTED {omitted} ({sources} sources) by depth {by_depth:?}; the first view loads \
         {tiles} capped tiles ({bytes} bytes), each leaf covered once"
    );
}

/// One tile of an archive expanded into one explicit hierarchy (`implicit::expand_tileset`:
/// external documents followed, implicit subtrees with their tile metadata), in the
/// archive's frame.
struct Sim {
    center: [f64; 3],
    half: [f64; 3],
    error: f64,
    content: bool,
    parent: Option<usize>,
    children: Vec<usize>,
    /// The leaf tiles (no children) at or below it.
    leaves: Vec<usize>,
}

/// The expanded tiles of `archive` (index 0 the root) and the tileset's geometric error.
fn expand_archive(archive: &Path) -> (Vec<Sim>, f64) {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let mut members: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..zip.len() {
        let mut member = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        members.insert(member.name().to_owned(), bytes);
    }
    let manifest: Value = serde_json::from_slice(&members["tileset.json"]).unwrap();
    let expanded = crate::implicit::expand_tileset(&manifest, |name| {
        members
            .get(name)
            .cloned()
            .ok_or_else(|| crate::Error::Message(format!("missing member {name}")))
    })
    .unwrap();
    fn build(v: &Value, parent: Option<usize>, offset: [f64; 3], out: &mut Vec<Sim>) -> usize {
        let mut offset = offset;
        if let Some(t) = v["transform"].as_array() {
            let t: Vec<f64> = t.iter().map(|x| x.as_f64().unwrap()).collect();
            if parent.is_some() {
                // Below the root, transforms are translations only.
                for (i, want) in [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.]
                    .iter()
                    .enumerate()
                {
                    assert_eq!(t[i], *want, "{t:?}");
                }
            }
            for a in 0..3 {
                offset[a] += t[12 + a];
            }
        }
        let b: Vec<f64> = v["boundingVolume"]["box"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        let center = std::array::from_fn(|a| b[a] + offset[a]);
        let half = std::array::from_fn(|a| b[3 + a].abs() + b[6 + a].abs() + b[9 + a].abs());
        let content =
            v.get("content").is_some() || v["contents"].as_array().is_some_and(|c| !c.is_empty());
        let id = out.len();
        out.push(Sim {
            center,
            half,
            error: v["geometricError"].as_f64().unwrap(),
            content,
            parent,
            children: Vec::new(),
            leaves: Vec::new(),
        });
        for c in v["children"].as_array().into_iter().flatten() {
            let child = build(c, Some(id), offset, out);
            out[id].children.push(child);
        }
        id
    }
    let mut tiles = Vec::new();
    build(&expanded["root"], None, [0.; 3], &mut tiles);
    for i in (0..tiles.len()).rev() {
        if tiles[i].children.is_empty() {
            assert!(tiles[i].content, "an empty leaf tile");
            tiles[i].leaves = vec![i];
        } else {
            let leaves: Vec<usize> = tiles[i]
                .children
                .iter()
                .flat_map(|&c| tiles[c].leaves.clone())
                .collect();
            tiles[i].leaves = leaves;
        }
    }
    (tiles, manifest["geometricError"].as_f64().unwrap())
}

/// Round 9 invariant over a written tileset: every content-less tile's geometric error is at
/// least `EMPTY_PARENT_SCALE` times its box diagonal (it refines whenever it is visible),
/// and no tile's error exceeds its parent's. Returns the content-less tiles.
fn check_errors(tiles: &[Sim]) -> usize {
    let scale = crate::mesh_large::delivery::plan::EMPTY_PARENT_SCALE;
    let mut empty = 0;
    for t in tiles {
        if !t.content {
            empty += 1;
            let diagonal = 2. * t.half.iter().map(|h| h * h).sum::<f64>().sqrt();
            assert!(
                t.error >= scale * diagonal * (1. - 1e-9),
                "content-less tile error {} below {scale} x diagonal {diagonal}",
                t.error
            );
        }
        if let Some(p) = t.parent {
            assert!(tiles[p].error >= t.error, "error above its parent's");
        }
    }
    empty
}

/// A camera sweep over `tiles`, as CesiumJS 1.146's base traversal selects them with every
/// tile loaded and visible (`Cesium3DTilesetBaseTraversal`): the root is skipped unless the
/// tileset's error projects past the maximum screen-space error (16 px at 1080 px, 60°
/// vertical field of view); a tile refines (its children are traversed) iff it has children,
/// its parent refines and its own error projects past the maximum; a tile that stops refining
/// is drawn iff it has content (an empty tile that stops draws nothing). Returns (frames in
/// which the tileset is drawn, of them frames with a leaf drawn by no selected tile, frames
/// with a leaf drawn twice).
fn traverse(tiles: &[Sim], tileset_error: f64) -> (usize, usize, usize) {
    let sse = |t: &Sim, error: f64, cam: [f64; 3]| {
        if error == 0. {
            return 0.;
        }
        let d = (0..3)
            .map(|a| ((cam[a] - t.center[a]).abs() - t.half[a]).max(0.).powi(2))
            .sum::<f64>()
            .sqrt()
            .max(1e-7);
        error * 1080. / (d * 2. * (std::f64::consts::PI / 6.).tan())
    };
    let root = &tiles[0];
    let extent = 2. * root.half.iter().copied().fold(0., f64::max);
    let leaves: Vec<usize> = root.leaves.clone();
    let (mut drawn, mut holes, mut doubles) = (0, 0, 0);
    for lateral in [-0.5, -0.25, 0., 0.25, 0.5] {
        for k in 0..=60 {
            let d = extent * 0.05 * 1.15f64.powi(k);
            let cam = [
                root.center[0] + lateral * extent,
                root.center[1],
                root.center[2] + root.half[2] + d,
            ];
            if sse(root, tileset_error, cam) <= 16. {
                continue;
            }
            drawn += 1;
            let mut covered: HashMap<usize, usize> = HashMap::new();
            let mut stack = vec![0usize];
            while let Some(i) = stack.pop() {
                let t = &tiles[i];
                if !t.children.is_empty() && sse(t, t.error, cam) > 16. {
                    stack.extend(&t.children);
                } else if t.content {
                    for &l in &t.leaves {
                        *covered.entry(l).or_default() += 1;
                    }
                }
            }
            holes += usize::from(leaves.iter().any(|l| !covered.contains_key(l)));
            doubles += usize::from(covered.values().any(|n| *n > 1));
        }
    }
    (drawn, holes, doubles)
}

/// Content-less tiles inside implicit subtrees of `archive` (available tiles without
/// content), read from the subtree files themselves.
fn implicit_empty_tiles(archive: &Path) -> usize {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let mut members: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..zip.len() {
        let mut member = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        members.insert(member.name().to_owned(), bytes);
    }
    let mut empty = 0;
    for (name, bytes) in &members {
        if !name.ends_with(".json") {
            continue;
        }
        let Ok(doc) = serde_json::from_slice::<Value>(bytes) else {
            continue;
        };
        let mut stack = vec![doc["root"].clone()];
        while let Some(tile) = stack.pop() {
            if let Some(tiling) = tile.get("implicitTiling") {
                let uri = tiling["subtrees"]["uri"]
                    .as_str()
                    .unwrap()
                    .replace("{level}", "0")
                    .replace("{x}", "0")
                    .replace("{y}", "0")
                    .replace("{z}", "0");
                let levels = tiling["subtreeLevels"].as_u64().unwrap() as u32;
                let (available, contents) =
                    crate::mesh_large::delivery::tests::subtree_availability(
                        &members[&uri],
                        levels,
                    );
                empty += available.len() - contents.len();
            }
            stack.extend(tile["children"].as_array().into_iter().flatten().cloned());
        }
    }
    empty
}

/// Round 9 (P1): omitted cells are content-less and have no content ancestor, so they must
/// refine whenever they are visible; with their children's (sub-millimetre) error Cesium
/// stopped at them and drew nothing (the cap-12 sheet was invisible at every distance, the
/// cap-30 sheet had holes). Every content-less tile now takes the empty-parent error, in
/// explicit output, inside implicit subtrees and as a document's root: over a camera sweep
/// the traversal draws every leaf exactly once in every frame, and the published content-less
/// tiles are exactly the omitted cells reported.
#[test]
fn omitted_cells_refine_whenever_they_are_visible() {
    use crate::mesh_large::delivery::{self, plan::Limits};
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let tight = Limits {
        objects: 3,
        records: 6,
        levels: 2,
    };
    let mut implicit_empty = 0;
    for cap in [12, 30] {
        let dir = stage(
            &input,
            &work.path().join(format!("cap{cap}")),
            &Run {
                cap,
                ..Run::default()
            },
        )
        .unwrap_or_else(|f| panic!("{}", f.error));
        let manifest: Value =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let omitted = manifest["hlod"]["omittedProxies"].as_u64().unwrap() as usize;
        assert!(omitted > 0, "cap {cap}: {}", manifest["hlod"]);
        for (name, limits) in [("default", delivery::plan::LIMITS), ("tight", tight)] {
            let output = work.path().join(format!("cap{cap}-{name}.3tz"));
            let mut file = fs::File::create(&output).unwrap();
            let (report, _) = delivery::publish_with_limits(
                &MeshTilesetRequest::new(&dir, &output),
                limits,
                &mut file,
                work.path(),
            )
            .unwrap();
            drop(file);
            inspect(ValidationRequest::new(&output)).unwrap();
            let (tiles, tileset_error) = expand_archive(&output);
            let empty = check_errors(&tiles);
            assert_eq!(empty, omitted, "cap {cap} {name}: {report:?}");
            let (drawn, holes, doubles) = traverse(&tiles, tileset_error);
            eprintln!(
                "TRAVERSAL cap {cap} {name}: {} tiles, {empty} content-less, {drawn} frames \
                 drawn, {holes} with holes, {doubles} with double draws; {report:?}",
                tiles.len()
            );
            assert!(drawn > 0);
            assert_eq!((holes, doubles), (0, 0), "cap {cap} {name}");
            if name == "tight" {
                implicit_empty += implicit_empty_tiles(&output);
            }
        }
    }
    // The tight caps put omitted cells inside implicit subtrees, where the same rule applies.
    assert!(implicit_empty > 0);
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
    let spilled = stage(
        &input,
        &work.path().join("spilled"),
        &Run {
            jobs: 2,
            mutation: Mutation::ForceSpill,
            ..Run::default()
        },
    )
    .unwrap();
    let resources: Value =
        serde_json::from_slice(&fs::read(spilled.join("resources.json")).unwrap()).unwrap();
    assert!(
        resources["hlod"]["diagnostics"]["held_spill"]["tiles"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(published(&one) == published(&spilled));
}

fn public(
    input: &Path,
    output: &Path,
    budget: Option<u64>,
    jobs: usize,
) -> Result<crate::TilerResult, crate::JobFailure> {
    let mut request = TilerRequest::new(input, output)
        .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(jobs).unwrap()))
        .with_test_leaf_triangles(256)
        .with_hlod_settings(test_settings(300))
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
    let memory = first.diagnostics.memory.clone();
    assert_eq!(memory.budget.resolved_bytes, 64 << 30);
    assert_eq!(
        first.diagnostics.leaf_delivery.page_bytes, 0,
        "no texture workspace"
    );
    assert_eq!(first.diagnostics.leaf_delivery.cache_bytes, 0);
    assert_eq!(memory.hlod.texture_workspace_bytes, 0);
    let hlod = first.diagnostics.hlod.clone();
    let public_summary = serde_json::to_value(&hlod).unwrap();
    assert_eq!(public_summary["omittedProxies"], 0);
    assert_eq!(public_summary["omissionSources"], 0);
    let depths = public_summary["omittedByDepth"].as_array().unwrap();
    assert!(!depths.is_empty());
    assert!(depths.iter().all(|count| count == 0));
    assert_eq!(depths.len(), hlod.borders_by_depth.len());
    assert!(hlod.tiles >= 3 && hlod.lease_peak_bytes <= memory.hlod.ledger_bytes);
    let minimum = memory.minimum_bytes;
    let exact = public(&input, &work.path().join("b.3tz"), Some(minimum), 3).unwrap();
    assert_eq!(exact.diagnostics.memory.minimum_bytes, minimum);
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
    // Zero is refused.
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

/// S6: the public operation's default path is HLOD, with nothing selected. A plain request
/// (only smaller leaves, so the fixture has coarse levels) resolves the automatic budget,
/// runs the HLOD phases and publishes a validated archive whose coarse members carry only
/// the representative `source_primitive` table (D4): no `proxy_source`, vanish or fill
/// records, no certificate fields, and monotone errors with content-less cells refining.
#[test]
fn default_public_path_publishes_hlod_coarse_tiles_without_h17_records() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let output = work.path().join("default.3tz");
    let result = crate::mesh_large::mesh_to_tileset(
        TilerRequest::new(&input, &output).with_test_leaf_triangles(256),
        &RunControl::default(),
    )
    .unwrap();
    let d = &result.diagnostics;
    assert_ne!(
        d.memory.budget.source,
        crate::mesh_large::BudgetSource::Explicit
    );
    assert!(d.hlod.tiles >= 3, "{:?}", d.hlod.tiles);
    assert!(d.phase_seconds.hlod_geometry > 0.);
    let checked = inspect(ValidationRequest::new(&output)).unwrap();
    assert!(checked
        .checks
        .iter()
        .any(|c| c == "decodedContentContainment"));
    check_errors(&expand_archive(&output).0);
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    let (mut coarse, mut leaves) = (0u64, 0u64);
    for i in 0..zip.len() {
        let mut member = zip.by_index(i).unwrap();
        let name = member.name().to_owned();
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        for retired in [
            &b"proxy_source"[..],
            b"lod_vanished",
            b"lod_filled",
            b"certif",
        ] {
            assert!(
                !bytes.windows(retired.len()).any(|w| w == retired),
                "{name} holds {}",
                String::from_utf8_lossy(retired)
            );
        }
        if !name.ends_with(".glb") {
            continue;
        }
        let decoded = super::decode::decode(&bytes).unwrap();
        let classes: Vec<_> = decoded.tables.iter().map(|t| t.class.as_str()).collect();
        if classes.contains(&"source_triangle") {
            leaves += 1;
        } else {
            assert_eq!(classes, ["source_primitive"], "{name}");
            assert!(decoded.faces.iter().all(|f| f.uniform_features), "{name}");
            coarse += 1;
        }
    }
    assert_eq!(coarse, d.hlod.tiles);
    assert_eq!(leaves, result.report.leaves);
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
            .with_test_leaf_triangles(256)
            .with_hlod_settings(test_settings(300))
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
/// coarse cap, balanced, two jobs, optional `RT_HLOD_EVIDENCE_BUDGET`) of
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
            jobs: 2,
            sampling: Schedule::PRODUCTION,
            nearest_steps: super::error::NEAREST_STEPS,
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
        "contractedSourceSegments": census.contracted_source_segments,
        "contractedSourceVertices": census.contracted_source_vertices,
        "certificateFailures": census.certificate_failures,
        "certificateWindingFailures": census.certificate_winding_failures,
        "maxSameDepthGapM": census.max_same_depth_gap_m,
        "maxMixedDepthGapM": census.max_mixed_depth_gap_m,
        "missingSegments": census.missing_segments, "missingPoints": census.missing_vertices,
        "mixedPairs": census.mixed_pairs, "sameDepthPairs": census.same_depth_pairs,
        "maxCoarseTriangles": census.max_coarse_triangles, "invalidFeatures": census.invalid_features,
        "overlaps": census.overlaps, "missingOverlapItems": census.missing_overlap_items,
        "movedBorderVertices": census.moved_border_vertices, "maxBorderMoveM": census.max_border_move_m,
        "perpendicularChecked": census.perpendicular_checked,
        "perpendicularDisplaced": census.perpendicular_displaced,
        "maxPerpendicularExcessM": census.max_perpendicular_excess_m,
        "duplicateChainTiles": census.duplicate_chain_tiles,
        "reversedCoarseFaces": census.reversed_coarse_faces,
        "hlod": manifest["hlod"], "resources": resources["hlod"], "members": members,
    });
    fs::write(
        out.join("census.json"),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    eprintln!("{census:?}");
    assert_eq!(
        census.missing_segments
            + census.missing_vertices
            + census.moved_border_vertices
            + census.duplicate_chain_tiles
            + census.reversed_coarse_faces
            + census.certificate_failures,
        0
    );
    assert_eq!(census.max_same_depth_gap_m, 0.);
    assert_eq!(census.certificate_winding_failures, 0);
    // Exact seams: source T-junctions and unwelded seams are reported (the census's
    // overlap and perpendicular counts above, the manifest's source-defect counts), not
    // asserted; omitted coarse tiles are reported by `omittedProxies`.
}

/// One flat untextured group holding every face of `tile` (S1's default material).
fn plain_appearance(tile: &super::store::TileGeometry) -> super::appearance::Appearance {
    super::appearance::Appearance {
        groups: vec![super::appearance::Group {
            class: 0,
            faces: (0..tile.triangles.len()).collect(),
            material: json!({"pbrMetallicRoughness":{"metallicFactor":0.0,"roughnessFactor":1.0}}),
            uvs: None,
            tangents: None,
            colors: None,
            pages: Vec::new(),
        }],
        normals: None,
        facts: Default::default(),
        timing: Default::default(),
    }
}

// ---------------------------------------------------------------------------------------
// S2: sampled two-sided source error.

/// A source index of `captured` admitted through a walk-style scratch ledger (S2 writes no
/// scratch it has not admitted).
fn with_source_index<R>(
    captured: &crate::mesh_large::full::FullCaptured,
    f: impl FnOnce(&super::source_index::SourceIndex) -> R,
) -> R {
    use super::{scratch::ScratchLedger, source_index};
    let admit = |_: u64| Ok(());
    let ledger = ScratchLedger::new(&captured.scratch, captured.scratch.live_bytes(), &admit);
    let need = source_index::scratch_need(captured.triangles.len()).unwrap();
    let lease = ledger
        .admit(source_index::SourceIndex::COMPONENT, need)
        .unwrap();
    let index = source_index::SourceIndex::build(captured, &lease, &|| Ok(())).unwrap();
    ledger.check().unwrap();
    f(&index)
}

fn capture_hlod(input: &Path, leaf_triangles: u64) -> crate::mesh_large::full::FullCaptured {
    use crate::mesh_large::{
        config::{Jobs, TreeConfig},
        full,
    };
    let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(leaf_triangles);
    config.jobs = Jobs::Fixed(NonZeroUsize::new(1).unwrap());
    full::capture(input, &config, &mut || Ok(())).unwrap()
}

#[test]
fn measured_positions_must_match_decoded_glb_bits() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let failure = stage(
        &input,
        &work.path().join("wrong-measurement"),
        &Run {
            mutation: Mutation::WrongMeasuredPosition,
            ..Run::default()
        },
    )
    .unwrap_err();
    assert!(
        failure
            .error
            .to_string()
            .contains("measured positions differ from decoded GLB bits"),
        "{}",
        failure.error
    );
}

/// Disk/index nearest against exhaustive triangle arithmetic, including clipped ranges
/// through block boundaries, deterministic source-order ties and original donor rows; the
/// budgeted production search agrees with it on this ordinary source, unexhausted.
#[test]
fn source_index_matches_exhaustive_with_range_clipping_and_ties() {
    use super::{
        error::Nearest,
        source_index::{bary, closest, distance2, Hit, Triangle},
    };
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &thin_islands());
    let captured = capture_hlod(&input, 256);
    let source: Vec<_> = captured
        .triangles
        .reader()
        .unwrap()
        .map(Result::unwrap)
        .collect();
    with_source_index(&captured, |index| {
        let mut random = 0x7159ac23u32;
        for (first, end) in [
            (0, source.len()),
            (17, 300),
            (250, 520),
            (source.len() - 13, source.len()),
        ] {
            let key = super::store::TileKey {
                first: first as u64,
                end: end as u64,
                depth: 1,
                leaf_begin: 0,
                leaf_end: 1,
            };
            let mut query = index.query(&captured, key).unwrap();
            let mut paged = index.paged_query(&captured, key).unwrap();
            let mut budgeted = index.query(&captured, key).unwrap();
            let mut nearest = Nearest::new();
            let streamed: Vec<_> = query.surface().unwrap().map(|t| t.unwrap().rank).collect();
            let mut streamed_sorted = streamed.clone();
            streamed_sorted.sort_unstable();
            assert_eq!(
                streamed_sorted,
                (first as u64..end as u64).collect::<Vec<_>>()
            );
            for _ in 0..40 {
                let mut unit = || {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    random as f64 / u32::MAX as f64
                };
                let p = [unit() * 110., unit() * 12., unit() * 110.];
                let actual = query.nearest(p).unwrap();
                let mut expected = Hit::empty();
                for (i, t) in source[first..end].iter().enumerate() {
                    let points = Triangle::new(t.triangle.positions, 0).positions();
                    let w = closest(p, points);
                    let d = distance2(p, bary(points, w));
                    let rank = (first + i) as u64;
                    let order = (t.triangle.source, t.expanded_ordinal);
                    if d < expected.distance2
                        || (d == expected.distance2
                            && (order, rank) < (expected.order, expected.rank))
                    {
                        expected.rank = rank;
                        expected.bary = w;
                        expected.distance2 = d;
                        expected.order = order;
                    }
                }
                let disk = paged.nearest(p).unwrap();
                assert_eq!(disk.rank, expected.rank);
                assert_eq!(disk.distance2, expected.distance2);
                assert_eq!(actual.rank, expected.rank);
                assert!((actual.distance2 - expected.distance2).abs() < 1e-10);
                let bounded = nearest.source(&mut budgeted, p).unwrap();
                assert_eq!(bounded.rank, expected.rank);
                assert!((bounded.distance2 - expected.distance2).abs() < 1e-10);
                let t = query.triangle(actual).unwrap();
                let expected_triangle = source[actual.rank as usize];
                assert_eq!(t, expected_triangle);
                assert_eq!(t.corners, expected_triangle.corners);
                assert_eq!(t.flags, expected_triangle.flags);
                assert_eq!(t.material, expected_triangle.material);
                assert!(actual.bary.iter().all(|v| v.is_finite() && *v >= 0.));
            }
            assert_eq!(nearest.stats.exhausted, 0, "{:?}", nearest.stats);
        }
    });
    captured.close().unwrap();
}

/// The source index writes no scratch it has not admitted: under-admitted, its first write
/// is refused (a resource limit) rather than worked around.
#[test]
fn source_index_refuses_scratch_past_its_admission() {
    use super::{scratch::ScratchLedger, source_index::SourceIndex};
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let captured = capture_hlod(&input, 256);
    let admit = |_: u64| Ok(());
    let ledger = ScratchLedger::new(&captured.scratch, captured.scratch.live_bytes(), &admit);
    let lease = ledger.admit(SourceIndex::COMPONENT, 4096).unwrap();
    let before = captured.scratch.live_bytes();
    let error = SourceIndex::build(&captured, &lease, &|| Ok(()))
        .err()
        .unwrap();
    assert_eq!(error.kind(), JobErrorKind::ResourceLimit, "{error}");
    assert!(captured.scratch.live_bytes() <= before + 4096);
    drop(lease);
    captured.close().unwrap();
}

/// The review's adversary (round 3 of #278): R distinct parallel oblique triangles whose
/// boxes all contain every query, while every nearest distance is positive. Exact search
/// tests every triangle per query, warm hints included (32 queries at R = 32,768 took
/// 1,048,607 `closest` calls). S2's budgeted search stays at O(B + depth) per query
/// whatever R, on the resident proxy path and on the paged source path, returns a finite
/// distance no smaller than the exact one (conservative for G) and counts its exhaustions.
#[test]
fn nearest_searches_are_bounded_on_oblique_adversaries() {
    use super::{
        error::{Nearest, NEAREST_STEPS},
        source_index::{Hit, Surface, Triangle, BLOCK, CLOSEST_CALLS},
    };
    let oblique = |r: u64| -> Vec<[[f32; 3]; 3]> {
        (0..r)
            .map(|i| {
                let d = i as f32 / 100_000.;
                [[3. + d, -1., -1.], [-1., 3. + d, -1.], [-1., -1., 3. + d]]
            })
            .collect()
    };
    let calls = |f: &mut dyn FnMut()| {
        CLOSEST_CALLS.with(|c| c.set(0));
        f();
        CLOSEST_CALLS.with(|c| c.get())
    };
    let queries: Vec<[f64; 3]> = (0..32).map(|j| [j as f64 / 1000., 0., 0.]).collect();
    let q = queries.len() as u64;
    // Resident surface (the decoded coarse tile of source-to-proxy samples).
    for r in [32_768u64, 131_072] {
        let surface = Surface::new(
            oblique(r)
                .into_iter()
                .enumerate()
                .map(|(i, t)| Triangle::new(t, i as u64))
                .collect(),
        )
        .unwrap();
        let key = super::store::TileKey {
            first: 0,
            end: u64::MAX,
            depth: 0,
            leaf_begin: 0,
            leaf_end: 0,
        };
        let mut exact = Vec::new();
        let exact_calls = calls(&mut || {
            let mut hint = None;
            for p in &queries {
                exact.push(surface.nearest_hint(*p, key, &mut hint));
            }
        });
        assert_eq!(exact_calls, q * r + q - 1, "the probe's Θ(Q·R)");
        let mut nearest = Nearest::new();
        let mut found: Vec<Hit> = Vec::new();
        let used = calls(&mut || {
            let mut hint = None;
            for p in &queries {
                found.push(nearest.surface(&surface, *p, &mut hint));
            }
        });
        // One hint test plus the budget per query (no cold seed after the first).
        assert!(used <= q * (NEAREST_STEPS + 1) + 64, "R={r}: {used} calls");
        assert_eq!(nearest.stats.searches, q);
        assert!(nearest.stats.exhausted > 0, "{:?}", nearest.stats);
        for (a, e) in found.iter().zip(&exact) {
            assert!(a.distance2.is_finite() && a.distance2 >= e.distance2);
        }
        eprintln!(
            "OBLIQUE resident R={r} Q={q}: exact {exact_calls} closest calls, bounded {used} \
             ({:?})",
            nearest.stats
        );
    }
    // Paged source index (the proxy-to-source samples and representatives).
    let r = 32_768u64;
    let mesh = Mesh {
        positions: oblique(r).into_iter().flatten().collect(),
        indices: (0..3 * r as u32).collect(),
        color: [0.5, 0.5, 0.5, 1.],
    };
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &[mesh]);
    let captured = capture_hlod(&input, 8192);
    let n = captured.triangles.len();
    assert_eq!(n, r);
    with_source_index(&captured, |index| {
        let key = super::store::TileKey {
            first: 0,
            end: n,
            depth: 0,
            leaf_begin: 0,
            leaf_end: 1,
        };
        let mut exact = Vec::new();
        let exact_calls = calls(&mut || {
            let mut query = index.query(&captured, key).unwrap();
            for p in &queries {
                exact.push(query.nearest(*p).unwrap());
            }
        });
        assert!(exact_calls >= q * r, "{exact_calls}");
        let mut nearest = Nearest::new();
        let mut found = Vec::new();
        let used = calls(&mut || {
            let mut query = index.query(&captured, key).unwrap();
            for p in &queries {
                found.push(nearest.source(&mut query, *p).unwrap());
            }
        });
        // Per query: the hint, the budget, and at most one cold seed's block.
        assert!(
            used <= q * (NEAREST_STEPS + 1) + BLOCK as u64,
            "{used} calls"
        );
        assert!(nearest.stats.exhausted > 0, "{:?}", nearest.stats);
        for (a, e) in found.iter().zip(&exact) {
            assert!(a.distance2.is_finite() && a.distance2 >= e.distance2);
        }
        eprintln!(
            "OBLIQUE paged R={r} Q={q}: exact {exact_calls} closest calls, bounded {used} ({:?})",
            nearest.stats
        );
    });
    captured.close().unwrap();
}

/// D1 omission: omitted cells hold no content, so S2 measures, samples and stores nothing
/// for them. At cap 12 every coarse tile of the sheet is omitted (nothing is measured); at
/// cap 30 some are published above omitted-free subtrees, each measured once, its G at
/// least every published descendant's, and delivery floors the content-less cells above.
#[test]
fn omitted_cells_are_not_measured_and_errors_stay_monotone() {
    use crate::mesh_large::delivery::{self, plan::LIMITS};
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    for cap in [12, 30] {
        let dir = stage(
            &input,
            &work.path().join(format!("cap{cap}")),
            &Run {
                cap,
                ..Run::default()
            },
        )
        .unwrap_or_else(|f| panic!("{}", f.error));
        let manifest: Value =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let hlod = &manifest["hlod"];
        assert!(hlod["omittedProxies"].as_u64().unwrap() > 0, "{hlod}");
        let proxies = rows(&dir.join("proxies.jsonl"));
        assert_eq!(proxies.is_empty(), cap == 12, "{hlod}");
        let measurement = &hlod["errorMeasurement"];
        let measured: u64 = measurement["byDepth"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["tiles"].as_u64().unwrap())
            .sum();
        assert_eq!(measured, proxies.len() as u64, "one measurement per row");
        assert_eq!(hlod["tiles"].as_u64(), Some(measured));
        let searches: u64 = proxies
            .iter()
            .map(|r| r["nearestSearch"]["searches"].as_u64().unwrap())
            .sum();
        assert_eq!(searches > 0, cap == 30);
        assert_eq!(measurement["nearest"]["searches"].as_u64(), Some(searches));
        for row in &proxies {
            let g = row["geometricError"].as_f64().unwrap();
            let m = &row["measurement"];
            assert!(g >= 1.05 * m["sampledMax"].as_f64().unwrap());
            assert!(g >= row["childrenError"].as_f64().unwrap());
            // Published descendants (deeper, inside its source range) never exceed it.
            let (first, end) = (row["first"].as_u64().unwrap(), row["end"].as_u64().unwrap());
            for other in &proxies {
                let (f, e) = (
                    other["first"].as_u64().unwrap(),
                    other["end"].as_u64().unwrap(),
                );
                if other["depth"].as_u64() > row["depth"].as_u64() && first <= f && e <= end {
                    assert!(other["geometricError"].as_f64().unwrap() <= g);
                }
            }
        }
        let output = work.path().join(format!("cap{cap}.3tz"));
        let mut file = fs::File::create(&output).unwrap();
        delivery::publish_with_limits(
            &MeshTilesetRequest::new(&dir, &output),
            LIMITS,
            &mut file,
            work.path(),
        )
        .unwrap();
        drop(file);
        inspect(ValidationRequest::new(&output)).unwrap();
        let shape = refinement_shape(&output);
        assert_eq!(
            shape.empty_below_content + shape.error_inversions,
            0,
            "{shape:?}"
        );
        // The total alone cannot authorize omissions at other depths. Move one reported
        // omission while preserving the sum; the intact stage must now be refused.
        let mut damaged_manifest = manifest.clone();
        let counts = damaged_manifest["hlod"]["omittedByDepth"]
            .as_array_mut()
            .unwrap();
        let from = counts.iter().position(|v| v.as_u64().unwrap() > 0).unwrap();
        let to = (from + 1) % counts.len();
        counts[from] = json!(counts[from].as_u64().unwrap() - 1);
        counts[to] = json!(counts[to].as_u64().unwrap() + 1);
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&damaged_manifest).unwrap(),
        )
        .unwrap();
        let output = work.path().join(format!("bad-omissions-{cap}.3tz"));
        let failure = mesh_leaves_to_3tz(
            MeshTilesetRequest::new(&dir, &output),
            &RunControl::default(),
        )
        .unwrap_err();
        assert!(
            failure.error.to_string().contains("omission counts differ"),
            "{}",
            failure.error
        );
        assert!(!output.exists());
    }
}

#[test]
fn delivery_refuses_understated_and_incomplete_sampled_error() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &sheet());
    let dir = stage(
        &input,
        &work.path().join("stage"),
        &Run {
            leaf_triangles: 1024,
            cap: 1500,
            ..s2_run()
        },
    )
    .unwrap();
    let original = rows(&dir.join("proxies.jsonl"));
    for mutation in 0..12 {
        let mut damaged = original.clone();
        let row = damaged.last_mut().unwrap();
        match mutation {
            0 => row["geometricError"] = json!(0.),
            1 => row["measurement"]["sourceToProxy"]["area"]["samples"] = json!(1),
            2 => row["measurement"]["q"] = json!(12345.),
            3 => row["measurement"]["margin"] = json!(0.),
            4 => {
                row.as_object_mut().unwrap().remove("errorPolicy");
                row.as_object_mut().unwrap().remove("measurement");
            }
            6 => {
                row.as_object_mut().unwrap().remove("encodedDepth");
            }
            7 => row["encodedDepth"] = json!(31),
            8 => row["encodedDepth"] = json!(row["encodedDepth"].as_u64().unwrap() + 1),
            // Every row's share looks complete against an inflated depth schedule, but the
            // published rows (nothing omitted) do not cover it.
            9 => {
                for row in &mut damaged {
                    let m = &mut row["measurement"];
                    m["depthSourceTriangles"] =
                        json!(m["depthSourceTriangles"].as_u64().unwrap() * 1000);
                }
            }
            // S2 review r9: a missing whole depth (or the entire coarse stream) must
            // not silently become a frontier when the producer reported no omissions.
            10 => damaged.retain(|r| r["depth"].as_u64().unwrap() != 0),
            11 => damaged.clear(),
            _ => {
                for row in damaged
                    .iter_mut()
                    .filter(|r| r["encodedDepth"].as_u64().unwrap() > 0)
                {
                    let m = &mut row["measurement"];
                    m["areaSamplesPerDirection"] = json!(2048);
                    m["sourceToProxy"]["area"]["samples"] = json!(2048);
                    m["proxyToSource"]["area"]["samples"] = json!(2048);
                    let maximum = m["sampledMax"].as_f64().unwrap();
                    let margin = maximum * 0.05;
                    m["margin"] = json!(margin);
                    row["geometricError"] =
                        json!(row["geometricError"].as_f64().unwrap().max(maximum * 1.05));
                }
            }
        };
        if mutation == 5 {
            let mut maximum = 0f64;
            for row in &mut damaged {
                maximum = maximum.max(row["geometricError"].as_f64().unwrap());
                row["geometricError"] = json!(maximum);
            }
        }
        let data = damaged
            .iter()
            .map(|r| format!("{}\n", serde_json::to_string(r).unwrap()))
            .collect::<String>();
        fs::write(dir.join("proxies.jsonl"), data).unwrap();
        let output = work.path().join(format!("bad-{mutation}.3tz"));
        let failure = mesh_leaves_to_3tz(
            MeshTilesetRequest::new(&dir, &output),
            &RunControl::default(),
        )
        .unwrap_err();
        let message = failure.error.to_string();
        match mutation {
            5 => assert!(message.contains("invalid S2 sample counts"), "{message}"),
            9 => assert!(message.contains("source triangles scheduled"), "{message}"),
            10 | 11 => assert!(message.contains("omission counts differ"), "{message}"),
            _ => {}
        }
        assert!(!output.exists());
    }
}

/// Review regression (S3 donor choice): two parallel sheets 0.01 m apart baked at 0.1 m
/// spacing, points alternating between them. Every point takes its own sheet's nearest
/// triangle, and the resident local surface returns exactly S2's paged search hits.
#[test]
fn bake_donors_are_exact_across_close_parallel_sheets() {
    use super::bake::{resolve, Queries, Searcher};
    use super::source_index::SourceIndex;
    use crate::mesh_large::{
        config::{Profile, TreeConfig},
        full,
    };
    let work = tempfile::tempdir().unwrap();
    let flat = |_: f32, _: f32| 0.;
    let lifted = |_: f32, _: f32| 0.01;
    let sheets = vec![
        grid((10, 10), (0., 1.), (0., 1.), &flat, [0.9, 0.1, 0.1, 1.]),
        grid((10, 10), (0., 1.), (0., 1.), &lifted, [0.1, 0.1, 0.9, 1.]),
    ];
    let input = write_gltf(&work.path().join("src"), &sheets);
    let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(256);
    config.jobs = crate::mesh_large::config::Jobs::Fixed(NonZeroUsize::new(1).unwrap());
    let captured = full::capture(&input, &config, &mut || Ok(())).unwrap();
    let key = super::store::TileKey {
        first: 0,
        end: captured.scan.triangles,
        depth: 0,
        leaf_begin: 0,
        leaf_end: 1,
    };
    let mut points = Vec::new();
    for row in 0..10 {
        for col in 0..10 {
            let (x, z) = (0.05 + 0.1 * col as f32, 0.05 + 0.1 * row as f32);
            let y = if (row + col) % 2 == 0 { 0. } else { 0.01 };
            points.push([x, y, z]);
        }
    }
    let run = |index: &SourceIndex, local: bool| {
        let mut query = index.query(&captured, key).unwrap();
        let mut searcher = if local {
            Searcher::new(&mut query).unwrap()
        } else {
            Searcher::paged(&mut query)
        };
        let mut q = [Queries::new(0, points.clone(), false)];
        resolve(&mut searcher, &mut q, &|| Ok(())).unwrap();
        let [q] = q;
        q.hits
    };
    let (paged, local) =
        with_source_index(&captured, |index| (run(index, false), run(index, true)));
    for (i, (a, b)) in paged.iter().zip(&local).enumerate() {
        assert_eq!(
            (a.rank, a.material, a.d2.to_bits(), a.uv),
            (b.rank, b.material, b.d2.to_bits(), b.uv),
            "point {i}"
        );
        let on_b = points[i][1] > 0.;
        assert_eq!(
            b.material,
            u32::from(on_b),
            "point {i} took the other sheet's donor"
        );
        assert!(b.d2 < 1e-12, "point {i} lies on its donor");
    }
}

/// Review round 3 probe 3: 32,768 distinct oblique parallel triangles whose boxes all
/// contain every query (exact search is Θ(R) per query, warm hints included). The bake's
/// budgeted best-first search stays within its fixed per-texel budget, deterministically.
#[test]
fn donor_search_is_bounded_on_oblique_adversaries() {
    use super::source_index::{Budget, Frontier, Hit, Surface, Triangle};
    let r = 32_768u64;
    let triangles: Vec<Triangle> = (0..r)
        .map(|i| {
            let d = i as f32 / 100_000.;
            Triangle::new(
                [[3. + d, -1., -1.], [-1., 3. + d, -1.], [-1., -1., 3. + d]],
                i,
            )
        })
        .collect();
    let surface = Surface::new(triangles).unwrap();
    let key = super::store::TileKey {
        first: 0,
        end: r,
        depth: 0,
        leaf_begin: 0,
        leaf_end: 1,
    };
    let budget_steps = 256;
    let mut frontier = Frontier::new();
    let run = |frontier: &mut Frontier| {
        let mut hint = None;
        (0..32)
            .map(|j| {
                let mut budget = Budget::new(budget_steps);
                let hit: Hit = surface.nearest_budget_hint(
                    [j as f64 / 1000., 0., 0.],
                    key,
                    &mut hint,
                    &mut budget,
                    frontier,
                );
                // The budget, plus a cold search's greedy seed (depth + one leaf).
                assert!(
                    budget.steps <= budget_steps + 64,
                    "query {j}: {} steps",
                    budget.steps
                );
                assert!(hit.distance2.is_finite());
                (hit.rank, budget.exhausted)
            })
            .collect::<Vec<_>>()
    };
    let first = run(&mut frontier);
    assert!(
        first.iter().all(|q| q.1),
        "the adversary exhausts the budget"
    );
    assert_eq!(first, run(&mut frontier), "deterministic");
}

/// Review round 4 defect 2: a cold search (no candidate yet) whose budget runs out before
/// it reaches any triangle (here B = 2 against a paged hierarchy many levels deep; the
/// review's adversary was 4,096 overlapping 256-triangle blocks against B = 4096) still
/// returns a finite donor: its greedy seed. Budgets bound improvement, never existence;
/// seeded and exhausted searches are counted, and an ample budget stays exact.
#[test]
fn cold_donor_search_always_has_a_finite_witness() {
    use super::source_index::{Budget, Frontier, Hit, Surface, Triangle};
    use crate::mesh_large::{
        config::{Profile, TreeConfig},
        full,
    };
    let work = tempfile::tempdir().unwrap();
    let wave = |x: f32, z: f32| 0.05 * (7. * x).sin() * (5. * z).cos();
    let input = write_gltf(
        &work.path().join("src"),
        &[grid(
            (64, 64),
            (0., 1.),
            (0., 1.),
            &wave,
            [0.5, 0.5, 0.5, 1.],
        )],
    );
    let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(256);
    config.jobs = crate::mesh_large::config::Jobs::Fixed(NonZeroUsize::new(1).unwrap());
    let captured = full::capture(&input, &config, &mut || Ok(())).unwrap();
    let key = super::store::TileKey {
        first: 0,
        end: captured.scan.triangles,
        depth: 0,
        leaf_begin: 0,
        leaf_end: 1,
    };
    with_source_index(&captured, |index| {
        cold_witness_checks(index, &captured, key)
    });
    // The resident surface: a budget of one step still yields the seed.
    let r = 4096u64;
    let triangles: Vec<Triangle> = (0..r)
        .map(|i| {
            let d = i as f32 / 100_000.;
            Triangle::new(
                [[3. + d, -1., -1.], [-1., 3. + d, -1.], [-1., -1., 3. + d]],
                i,
            )
        })
        .collect();
    let surface = Surface::new(triangles).unwrap();
    let key = super::store::TileKey {
        first: 0,
        end: r,
        depth: 0,
        leaf_begin: 0,
        leaf_end: 1,
    };
    let mut budget = Budget::new(1);
    let hit: Hit = surface.nearest_budget_hint(
        [0.5, 0., 0.],
        key,
        &mut None,
        &mut budget,
        &mut Frontier::new(),
    );
    assert!(hit.distance2.is_finite());
    assert!(budget.seeded && budget.exhausted);
}

fn cold_witness_checks(
    index: &super::source_index::SourceIndex,
    captured: &crate::mesh_large::full::FullCaptured,
    key: super::store::TileKey,
) {
    use super::bake::Searcher;
    let points: Vec<[f64; 3]> = (0..16)
        .map(|i| [0.03 + 0.06 * i as f64, 0.2, 0.97 - 0.05 * i as f64])
        .collect();
    for p in &points {
        let exact = index.query(captured, key).unwrap().nearest(*p).unwrap();
        let cold = |steps: u64| {
            let mut query = index.query(captured, key).unwrap();
            let mut searcher = Searcher::paged_with_budget(&mut query, steps);
            let hit = searcher.search(*p).unwrap();
            (hit, searcher.exhausted, searcher.seeded)
        };
        let (starved, exhausted, seeded) = cold(2);
        assert!(starved.distance2.is_finite(), "{p:?}: no finite witness");
        assert_eq!((exhausted, seeded), (1, 1));
        assert!(starved.distance2 >= exact.distance2);
        let again = cold(2).0;
        assert_eq!(
            (again.rank, again.distance2.to_bits()),
            (starved.rank, starved.distance2.to_bits()),
            "deterministic"
        );
        let (ample, exhausted, seeded) = cold(1 << 20);
        assert_eq!(
            (ample.rank, ample.distance2.to_bits()),
            (exact.rank, exact.distance2.to_bits())
        );
        assert_eq!((exhausted, seeded), (0, 1));
    }
}

/// Evidence lane, not CI: S2's cost on the adversarial chords input (every box overlaps
/// almost every query) at production settings (8,192-triangle leaves and cap, one job),
/// for each budget in `RT_HLOD_EVIDENCE_STEPS` (comma-separated, default `4096,1024,256`)
/// and both schedules. Writes `chords-s2.json` under `RT_HLOD_EVIDENCE_DIR`: S2 CPU,
/// searches, steps and exhaustions by encoded depth, and G by depth.
#[test]
#[ignore = "evidence lane: needs RT_HLOD_EVIDENCE_DIR"]
fn evidence_chords_s2_cost() {
    let out = PathBuf::from(std::env::var("RT_HLOD_EVIDENCE_DIR").unwrap());
    fs::create_dir_all(&out).unwrap();
    let input = chords_input(&out.join("src"));
    let budgets: Vec<u64> = std::env::var("RT_HLOD_EVIDENCE_STEPS")
        .unwrap_or_else(|_| "4096,1024,256".into())
        .split(',')
        .map(|b| b.parse().unwrap())
        .collect();
    let mut records = Vec::new();
    for (label, sampling) in [
        ("production", Schedule::PRODUCTION),
        ("test", TEST_SAMPLING),
    ] {
        for &steps in &budgets {
            let output = out.join(format!("chords-{label}-{steps}.3tz"));
            let started = std::time::Instant::now();
            let result = crate::mesh_large::mesh_to_tileset(
                TilerRequest::new(&input, &output)
                    .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(1).unwrap()))
                    .with_test_leaf_triangles(crate::mesh_large::config::CONSUMER_TABLE_ROWS)
                    .with_hlod_settings(Settings {
                        nearest_steps: steps,
                        sampling,
                        ..Settings::default()
                    }),
                &RunControl::default(),
            )
            .unwrap_or_else(|f| panic!("{}", f.error));
            let wall = started.elapsed().as_secs_f64();
            let h = result.diagnostics.hlod;
            let m = &h.error_measurement;
            let depths: Vec<Value> = m
                .by_depth
                .iter()
                .enumerate()
                .filter(|(_, d)| d.tiles > 0)
                .map(|(i, d)| {
                    json!({
                        "encodedDepth": i, "tiles": d.tiles, "g": d.geometric_error,
                        "sampledMax": d.sampled_max,
                        "s2CpuSeconds": h.error_cpu_by_depth.get(i).copied().unwrap_or(0.),
                        "searches": d.nearest.searches, "steps": d.nearest.steps,
                        "exhausted": d.nearest.exhausted, "maxSteps": d.nearest.max_steps,
                        "sourceAreaSamples": d.source_area_samples,
                        "sourceControls": d.source_controls, "proxyControls": d.proxy_controls,
                    })
                })
                .collect();
            let record = json!({
                "schedule": label, "nearestSteps": steps, "wallSeconds": wall,
                "stageCpuSeconds": h.cpu_seconds, "s2CpuSeconds": h.error_cpu_seconds,
                "s2Cpu": h.error_cpu, "nearest": m.nearest, "byDepth": depths,
            });
            eprintln!("CHORDS-S2 {record}");
            records.push(record);
            fs::remove_file(&output).ok();
        }
    }
    fs::write(
        out.join("chords-s2.json"),
        serde_json::to_vec_pretty(&records).unwrap(),
    )
    .unwrap();
}

/// S2's whole measurement (representatives, both directed passes and their controls) on
/// an adversarial source through the real stage, at production's nearest budget: every
/// search stays within its budget plus one seed, some exhaust, and G still covers the
/// sampled error and the children's.
#[test]
fn s2_measurement_is_bounded_on_an_oblique_source() {
    use super::{error::NEAREST_STEPS, source_index::BLOCK};
    let r = 1024u32;
    let mesh = Mesh {
        positions: (0..r)
            .flat_map(|i| {
                let d = i as f32 / 10_000.;
                [[3. + d, -1., -1.], [-1., 3. + d, -1.], [-1., -1., 3. + d]]
            })
            .collect(),
        indices: (0..3 * r).collect(),
        color: [0.5, 0.5, 0.5, 1.],
    };
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &[mesh]);
    let dir = stage(
        &input,
        &work.path().join("oblique"),
        // Disjoint triangles do not simplify: a cap above the whole source keeps parents.
        &Run {
            cap: 2048,
            nearest_steps: NEAREST_STEPS,
            ..Run::default()
        },
    )
    .unwrap_or_else(|f| panic!("{}", f.error));
    let proxies = rows(&dir.join("proxies.jsonl"));
    assert!(!proxies.is_empty());
    let bound = NEAREST_STEPS + BLOCK as u64 + 64;
    let mut exhausted = 0;
    for row in &proxies {
        let s = &row["nearestSearch"];
        let (searches, steps) = (
            s["searches"].as_u64().unwrap(),
            s["steps"].as_u64().unwrap(),
        );
        assert!(s["maxSteps"].as_u64().unwrap() <= bound, "{s}");
        assert!(steps <= searches * bound, "{s}");
        exhausted += s["exhausted"].as_u64().unwrap();
        let g = row["geometricError"].as_f64().unwrap();
        assert!(g >= 1.05 * row["measurement"]["sampledMax"].as_f64().unwrap());
        assert!(g >= row["childrenError"].as_f64().unwrap());
    }
    assert!(exhausted > 0, "the adversary exhausts some searches");
}

/// Replays bounded parent jobs from immutable archive evidence; never performs a bake.
#[test]
#[ignore = "evidence lane: RT_HLOD_REPLAY_DIR"]
fn evidence_replay_topology() {
    use super::{borders::Contract, interfaces::InterfaceRow, store::*};
    let root = PathBuf::from(std::env::var("RT_HLOD_REPLAY_DIR").unwrap());
    let old = rows(&root.join("candidate-c053-tex3/out.hlod-audit/borders.jsonl"));
    let current = rows(&root.join("candidate-bbbe-tex3/out.hlod-audit/borders.jsonl"));
    let mut archive =
        zip::ZipArchive::new(fs::File::open(root.join("candidate-bbbe-tex3/out.3tz")).unwrap())
            .unwrap();
    let point = |v: &Value| std::array::from_fn(|i| v[i].as_u64().unwrap() as u32);
    let key = |v: &Value| TileKey {
        first: v["first"].as_u64().unwrap(),
        end: v["end"].as_u64().unwrap(),
        depth: v["depth"].as_u64().unwrap() as u32,
        leaf_begin: v["leafBegin"].as_u64().unwrap() as u32,
        leaf_end: v["leafEnd"].as_u64().unwrap() as u32,
    };
    for omitted in current
        .iter()
        .filter(|r| r["kind"] == "omitted" && r["source"] == true)
    {
        let tile = key(&omitted["key"]);
        let row = old
            .iter()
            .find(|r| r["tile"]["first"] == tile.first && r["tile"]["depth"] == tile.depth)
            .unwrap();
        let c = &row["contract"];
        let contract = Contract {
            depth: tile.depth,
            scale: c["scale"].as_f64().unwrap(),
            certificates: vec![],
            terminal_vertices: vec![],
            interface: c["interface"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    InterfaceRow::new(
                        r["leaf"].as_u64().unwrap() as u32,
                        point(&r["a"]),
                        point(&r["b"]),
                        r["lo"].as_u64().unwrap() as u32,
                        r["hi"].as_u64().unwrap() as u32,
                    )
                })
                .collect(),
        };
        let children: Vec<_> = omitted["frontier"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let k = key(v);
                let mut bytes = Vec::new();
                archive
                    .by_name(&format!("content/f{}-d{}.glb", k.first, k.depth))
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .unwrap();
                let faces = decode(&bytes).faces;
                let n = faces.len();
                TileGeometry {
                    key: k,
                    positions: faces.into_iter().flatten().collect(),
                    ells: vec![k.depth as u8; n * 3],
                    triangles: (0..n as u32)
                        .map(|i| [i * 3, i * 3 + 1, i * 3 + 2])
                        .collect(),
                    representative: vec![0; n],
                    primitives: vec![SourcePrimitive {
                        node: 0,
                        mesh: 0,
                        primitive: 0,
                        expanded_ordinal: 0,
                        material: u32::MAX,
                    }],
                    interface: vec![],
                    geometric_error: 0.,
                    lease: None,
                }
            })
            .collect();
        let built = super::geometry::build_parent(
            tile,
            &children,
            &[8192],
            &contract,
            &|_| Ok(None),
            &mut |_| Ok(true),
            &|| Ok(()),
        )
        .unwrap();
        eprintln!(
            "REPLAY {}: built={} attempts={:?}",
            tile.first,
            built.built.is_some(),
            built.attempted_topology
        );
        assert!(
            built.built.is_some(),
            "bounded real parent {} must fit",
            tile.first
        );
    }
}
