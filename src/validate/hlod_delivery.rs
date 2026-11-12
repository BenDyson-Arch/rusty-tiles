//! Native archive-only HLOD identity, continuity and exact delivery checks.
//! D1 interface keys are common-frame canonical f32 triples (+0); R9 original
//! aliases retain independent exact placed f64 keys. Collapsed canonical faces
//! contribute no interface items; three-distinct collinear faces still do.
//!
//! C1/identity admission runs once per payload. Bounded geometry replay emits
//! only compact point/edge keys and per-leaf face incidence into disk SQLite,
//! through bounded external sorts so that every row is appended in key order
//! (`bulk`). Ordered key scans find actual outside-leaf owners and stage sparse
//! ancestor obligations. No T contacts, distance joins or tolerant adjacency are inferred.
//! Every directed incident owner survives local balanced cancellation.
//! Original IDs recorded once at ingest (an appended spill sequence) are checked
//! by one bounded external sort of their exact placed keys, including disjoint
//! placements and repeated references.
//! No triangle soup or archive-sized geometry/alias vector is retained.
//! Decoding, key work, scans, owner walks and queries consume the existing work
//! budget, including one unit per 10,000 SQLite VM operations. SQLite's page
//! cache is bounded; temporary sorts are disk backed and transient sort sizes
//! are outside disk_bytes(), which counts the monotonic main database only. The
//! original-vertex sequence and the alias sort's runs are scratch files counted
//! in the scratch peak; the sort holds at most the catalog's side-sort memory.
//!
//! D11 (`borders`): an ancestor may contract its shared leaf border to chords
//! between retained exact vertices, one witnessed chord per arc, each contracted
//! vertex within G/8 of its chord at its edge ordinal (A1); junctions and point
//! contacts stay exact. Siblings agree exactly, a shallower leaf locks deeper
//! neighbours, and other provably no-deeper pairs stay within G/8. Directed
//! incidences are counted: unsupported same-direction multiplicity must lie on
//! distinct sides (exact wedge sign) and coincident faces need coincident source.
//!
//! Census basis is decoded leaf canonical keys, not unavailable original source
//! coordinates. Uninspected here (source oracle and audit sidecar): contraction
//! against source arcs and producer certificates (epsilon, E_d, terminal-lock
//! claims), logical depth (so same-depth exactness between cousins and G/8 where
//! the finer neighbour has the larger G), topology receipts, fresh coarse UV
//! charts, source distance, and coarse orientation outside required
//! interfaces/local contradictory pairs.
use super::{identity, invalid, spill_failure, Budget, Error, ScratchRefusal};
mod borders;
mod bulk;
mod drops;
pub(super) mod recovery;
mod replay;
#[cfg(test)]
pub(super) use replay::JOB_DECODE_BYTES;
pub(super) use replay::{Fetch, Payloads};
pub(super) mod texture;
mod ticks;
mod vfs;
use crate::archive3tz::CatalogConfig;
use crate::spill::{
    ExternalSorter, Record, Scratch, Sequence, SequenceWriter, SlotFile, SortConfig,
};
use bulk::Sorter;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{value::RawValue, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

type Point = [f64; 3];
type Matrix = [f64; 16];
type Source = [u64; 5];
const IDENTITY: Matrix = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];
// glTF Y-up -> 3D Tiles Z-up; compare in the common root's glTF frame.
const Y_UP: Matrix = [
    1., 0., 0., 0., 0., 0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1.,
];
const INVERSE_Y_UP: Matrix = [
    1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1.,
];
fn delivery_frame(relative: Matrix) -> Matrix {
    multiply(multiply(INVERSE_Y_UP, relative), Y_UP)
}
const SOURCE_COLUMNS: [&str; 5] = [
    "node_index",
    "mesh_index",
    "primitive_index",
    "expanded_ordinal",
    "material_index",
];

fn bad(label: &str, detail: &str) -> Error {
    invalid(format!("{label}: {detail}"))
}
fn sql(error: rusqlite::Error) -> Error {
    Error::Scratch(ScratchRefusal::Io(std::io::Error::other(format!(
        "HLOD delivery database: {error}"
    ))))
}
fn integer(n: u64) -> Result<i64, Error> {
    i64::try_from(n).map_err(|_| Error::limit("HLOD delivery index exceeds disk index range"))
}
fn encode(values: &[f64]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn decode<const N: usize>(bytes: &[u8]) -> Result<[f64; N], Error> {
    if bytes.len() != N * 8 {
        return Err(Error::InvalidState("corrupt HLOD geometry scratch".into()));
    }
    Ok(std::array::from_fn(|i| {
        f64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap())
    }))
}
fn point(m: Matrix, p: Point) -> Point {
    std::array::from_fn(|i| m[i] * p[0] + m[4 + i] * p[1] + m[8 + i] * p[2] + m[12 + i])
}
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|i| (0..4).map(|k| a[k * 4 + i % 4] * b[i / 4 * 4 + k]).sum())
}
fn nonsingular(m: Matrix) -> bool {
    let mut rows: [[f64; 3]; 3] = std::array::from_fn(|r| std::array::from_fn(|c| m[c * 4 + r]));
    for row in &mut rows {
        let max = row.iter().copied().map(f64::abs).fold(0., f64::max);
        if max == 0. {
            return false;
        }
        for v in row {
            *v /= max;
        }
    }
    for c in 0..3 {
        let pivot = (c..3)
            .max_by(|&a, &b| rows[a][c].abs().total_cmp(&rows[b][c].abs()))
            .unwrap();
        rows.swap(c, pivot);
        if rows[c][c] == 0. {
            return false;
        }
        let pivot_row = rows[c];
        for row in rows.iter_mut().skip(c + 1) {
            let f = row[c] / pivot_row[c];
            for (value, pivot) in row.iter_mut().zip(pivot_row).skip(c + 1) {
                *value -= f * pivot;
            }
        }
    }
    true
}

fn finite(p: &[f64]) -> Result<(), Error> {
    if p.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(bad("interfaceSegments", "non-finite transformed geometry"))
    }
}
fn node_matrix(node: &Value) -> Result<Matrix, Error> {
    let values = |key: &str, default: &[f64]| -> Result<Vec<f64>, Error> {
        match node.get(key) {
            Some(v) => {
                let a = v
                    .as_array()
                    .ok_or_else(|| bad("interfaceSegments", "invalid scene transform"))?;
                if a.len() != default.len() {
                    return Err(bad("interfaceSegments", "invalid scene transform"));
                }
                a.iter()
                    .map(|v| {
                        v.as_f64()
                            .filter(|n| n.is_finite())
                            .ok_or_else(|| bad("interfaceSegments", "invalid scene transform"))
                    })
                    .collect()
            }
            None => Ok(default.to_vec()),
        }
    };
    if node.get("matrix").is_some() {
        return Ok(values("matrix", &IDENTITY)?.try_into().unwrap());
    }
    let t = values("translation", &[0.; 3])?;
    let s = values("scale", &[1.; 3])?;
    let q = values("rotation", &[0., 0., 0., 1.])?;
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    Ok([
        (1. - 2. * (y * y + z * z)) * s[0],
        2. * (x * y + z * w) * s[0],
        2. * (x * z - y * w) * s[0],
        0.,
        2. * (x * y - z * w) * s[1],
        (1. - 2. * (x * x + z * z)) * s[1],
        2. * (y * z + x * w) * s[1],
        0.,
        2. * (x * z + y * w) * s[2],
        2. * (y * z - x * w) * s[2],
        (1. - 2. * (x * x + y * y)) * s[2],
        0.,
        t[0],
        t[1],
        t[2],
        1.,
    ])
}

/// Inspect raw ownership before projecting fields. Opaque metadata/extension
/// values never pass through serde_json::Value or acquire floating-point meaning.
fn admit_scene_profile(raw: &RawValue, work: &mut identity::Work) -> Result<(), Error> {
    fn array(raw: Option<&RawValue>) -> Result<Vec<&RawValue>, Error> {
        raw.map(|r| serde_json::from_str(r.get()).map_err(Into::into))
            .unwrap_or_else(|| Ok(Vec::new()))
    }
    fn known(name: &str) -> bool {
        matches!(
            name,
            "KHR_mesh_quantization"
                | "EXT_meshopt_compression"
                | "EXT_mesh_features"
                | "EXT_structural_metadata"
                | "KHR_materials_unlit"
                | "KHR_materials_ior"
                | "KHR_materials_clearcoat"
                | "KHR_materials_transmission"
                | "KHR_materials_volume"
                | "KHR_materials_sheen"
                | "KHR_materials_specular"
                | "KHR_materials_emissive_strength"
                | "KHR_materials_iridescence"
                | "KHR_materials_anisotropy"
                | "KHR_materials_dispersion"
                | "KHR_materials_pbrSpecularGlossiness"
                | "KHR_materials_variants"
                | "KHR_texture_transform"
                | "KHR_texture_basisu"
                | "EXT_texture_webp"
                | "EXT_texture_avif"
                | "MSFT_texture_dds"
                | "KHR_lights_punctual"
        )
    }
    fn container(raw: &RawValue, work: &mut identity::Work) -> Result<(), Error> {
        work.charge(1)?;
        if let Some(extensions) = identity::field(raw, "extensions") {
            let fields: BTreeMap<String, &RawValue> = serde_json::from_str(extensions.get())?;
            work.charge(fields.len())?;
            for name in fields.keys() {
                if !known(name) {
                    return Err(Error::unsupported(format!(
                        "interfaceSegments: unsupported rendered scene extension {name}"
                    )));
                }
            }
        }
        if ["skin", "weights", "targets"]
            .iter()
            .any(|k| identity::field(raw, k).is_some())
        {
            return Err(Error::unsupported("interfaceSegments: skin or morph deformation is outside the rendered scene profile"));
        }
        Ok(())
    }
    container(raw, work)?;
    for field in ["extensionsUsed", "extensionsRequired"] {
        if let Some(names) = identity::field(raw, field) {
            for name in serde_json::from_str::<Vec<String>>(names.get())? {
                work.charge(1)?;
                if !known(&name) {
                    return Err(Error::unsupported(format!(
                        "interfaceSegments: unsupported rendered scene extension {name}"
                    )));
                }
            }
        }
    }
    if !array(identity::field(raw, "animations"))?.is_empty() {
        return Err(Error::unsupported(
            "interfaceSegments: animation is outside the rendered scene profile",
        ));
    }
    let scenes = array(identity::field(raw, "scenes"))?;
    if scenes.is_empty() || (identity::field(raw, "scene").is_none() && scenes.len() != 1) {
        return Err(Error::unsupported(
            "interfaceSegments: a unique selected glTF scene is required",
        ));
    }
    for scene in scenes {
        container(scene, work)?;
    }
    for name in ["nodes", "meshes", "materials", "textures"] {
        for object in array(identity::field(raw, name))? {
            container(object, work)?;
            if name == "meshes" {
                for primitive in array(identity::field(object, "primitives"))? {
                    container(primitive, work)?;
                }
            }
        }
    }
    Ok(())
}

/// Streaming accessor: decoded views are shared by `identity::Work`; no expanded
/// archive triangle soup, and no conversion of UINT64 source columns to f64.
struct Accessor {
    bytes: Rc<Vec<u8>>,
    offset: usize,
    stride: usize,
    count: usize,
    width: usize,
    components: usize,
    component: u64,
    normalized: bool,
}
impl Accessor {
    fn new(doc: &Value, bin: &[u8], id: usize, work: &mut identity::Work) -> Result<Self, Error> {
        let a = &doc["accessors"][id];
        let components = match a["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            _ => {
                return Err(bad(
                    "interfaceSegments",
                    "unsupported geometry accessor shape",
                ))
            }
        };
        if a.get("sparse").is_some() {
            return Err(Error::unsupported(
                "interfaceSegments: sparse HLOD geometry accessor",
            ));
        }
        let component = a["componentType"]
            .as_u64()
            .ok_or_else(|| bad("interfaceSegments", "invalid component"))?;
        let width = match component {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(bad("interfaceSegments", "invalid component")),
        };
        let vi = identity::uint(&a["bufferView"])?;
        Ok(Self {
            bytes: identity::view(doc, bin, vi, work)?,
            offset: a["byteOffset"].as_u64().unwrap_or(0) as usize,
            stride: doc["bufferViews"][vi]["byteStride"]
                .as_u64()
                .unwrap_or((width * components) as u64) as usize,
            count: identity::uint(&a["count"])?,
            width,
            components,
            component,
            normalized: a["normalized"] == true,
        })
    }
    fn at(&self, index: usize, c: usize) -> Result<f64, Error> {
        if index >= self.count || c >= self.components {
            return Err(bad(
                "interfaceSegments",
                "accessor index outside decoded geometry",
            ));
        }
        let start = index
            .checked_mul(self.stride)
            .and_then(|n| n.checked_add(self.offset))
            .and_then(|n| n.checked_add(c * self.width))
            .ok_or_else(|| bad("interfaceSegments", "accessor offset overflow"))?;
        let b = identity::slice(&self.bytes, start, self.width)?;
        let n = match self.component {
            5120 => (b[0] as i8) as f64,
            5121 => b[0] as f64,
            5122 => i16::from_le_bytes(b.try_into().unwrap()) as f64,
            5123 => u16::from_le_bytes(b.try_into().unwrap()) as f64,
            5125 => u32::from_le_bytes(b.try_into().unwrap()) as f64,
            _ => f32::from_le_bytes(b.try_into().unwrap()) as f64,
        };
        let n = if self.normalized {
            match self.component {
                5120 => (n / 127.).max(-1.),
                5121 => n / 255.,
                5122 => (n / 32767.).max(-1.),
                5123 => n / 65535.,
                _ => return Err(bad("interfaceSegments", "invalid normalized component")),
            }
        } else {
            n
        };
        if n.is_finite() {
            Ok(n)
        } else {
            Err(bad("interfaceSegments", "non-finite decoded geometry"))
        }
    }
    fn index(&self, i: usize) -> Result<usize, Error> {
        let n = self.at(i, 0)?;
        if self.components != 1
            || self.normalized
            || n < 0.
            || n.fract() != 0.
            || n > u32::MAX as f64
        {
            return Err(bad("sourceIdentity", "invalid decoded index"));
        }
        Ok(n as usize)
    }
    /// A VEC3 element as stored (normalization applied, no node transform).
    fn vec3(&self, i: usize) -> Result<Point, Error> {
        Ok([self.at(i, 0)?, self.at(i, 1)?, self.at(i, 2)?])
    }
    fn position(&self, i: usize, m: Matrix) -> Result<Point, Error> {
        if self.components != 3 {
            return Err(bad("interfaceSegments", "POSITION must be VEC3"));
        }
        let raw = self.vec3(i)?;
        let p = point(m, raw);
        finite(&p)?;
        Ok(p)
    }
}

struct Column {
    bytes: Rc<Vec<u8>>,
    width: usize,
    count: usize,
}
impl Column {
    fn new(
        doc: &Value,
        bin: &[u8],
        table: usize,
        key: &str,
        work: &mut identity::Work,
    ) -> Result<Self, Error> {
        let metadata = &doc["extensions"]["EXT_structural_metadata"];
        let t = &metadata["propertyTables"][table];
        let schema = &metadata["schema"]["classes"][t["class"]
            .as_str()
            .ok_or_else(|| bad("sourceIdentity", "missing class"))?]["properties"][key];
        let width = match schema["componentType"].as_str() {
            Some("UINT8") => 1,
            Some("UINT32") => 4,
            Some("UINT64") => 8,
            _ => return Err(bad("sourceIdentity", "invalid original-row column")),
        };
        Ok(Self {
            bytes: identity::view(
                doc,
                bin,
                identity::uint(&t["properties"][key]["values"])?,
                work,
            )?,
            width,
            count: identity::uint(&t["count"])?,
        })
    }
    fn at(&self, i: usize) -> Result<u64, Error> {
        if i >= self.count {
            return Err(bad("sourceIdentity", "original row outside table"));
        }
        let b = identity::slice(
            &self.bytes,
            i.checked_mul(self.width)
                .ok_or_else(|| bad("sourceIdentity", "column overflow"))?,
            self.width,
        )?;
        Ok(match self.width {
            1 => u64::from(b[0]),
            4 => u64::from(u32::from_le_bytes(b.try_into().unwrap())),
            _ => u64::from_le_bytes(b.try_into().unwrap()),
        })
    }
}
struct Feature {
    ids: Accessor,
    columns: Vec<Column>,
    table: usize,
}
impl Feature {
    fn new(
        doc: &Value,
        bin: &[u8],
        p: &Value,
        label: &str,
        work: &mut identity::Work,
    ) -> Result<Option<Self>, Error> {
        let set = p["extensions"]["EXT_mesh_features"]["featureIds"]
            .as_array()
            .and_then(|xs| xs.iter().find(|s| s["label"] == label));
        let Some(set) = set else { return Ok(None) };
        let table = identity::uint(&set["propertyTable"])?;
        let id = identity::uint(
            &p["attributes"][format!("_FEATURE_ID_{}", identity::uint(&set["attribute"])?)],
        )?;
        let mut keys = SOURCE_COLUMNS.to_vec();
        if label == "source_triangle" {
            keys.extend(["triangle_index", "vertex_0", "vertex_1", "vertex_2"]);
        }
        let columns = keys
            .iter()
            .map(|k| Column::new(doc, bin, table, k, work))
            .collect::<Result<_, _>>()?;
        Ok(Some(Self {
            ids: Accessor::new(doc, bin, id, work)?,
            columns,
            table,
        }))
    }
    fn row(&self, vertex: usize, work: &mut identity::Work) -> Result<(usize, Source), Error> {
        work.charge(self.columns.len() + 1)?;
        let row = self.ids.index(vertex)?;
        let mut key = [0; 5];
        for (i, v) in key.iter_mut().enumerate() {
            *v = self.columns[i].at(row)?;
        }
        Ok((row, key))
    }
}

// R9 identity uses exact f64 placed coordinates, independently of D1's f32 keys.
fn point_key(p: Point) -> [u64; 3] {
    p.map(|v| if v == 0. { 0 } else { v.to_bits() })
}

fn replay_document(
    bytes: &[u8],
    cap: usize,
    budget: Budget,
    base: u64,
) -> Result<(Value, &[u8], identity::Work), Error> {
    let length = u32::from_le_bytes(identity::slice(bytes, 12, 4)?.try_into().unwrap()) as usize;
    let mut work = identity::Work::new(budget, base, cap);
    work.charge(length.div_ceil(4))?;
    let raw: &RawValue = serde_json::from_slice(identity::slice(bytes, 20, length)?)?;
    let doc = identity::project(raw, "doc")?;
    let bin = identity::slice(
        bytes,
        28 + length,
        u32::from_le_bytes(identity::slice(bytes, 20 + length, 4)?.try_into().unwrap()) as usize,
    )?;
    Ok((doc, bin, work))
}

// Interface canonicalization is f32. Original alias identity deliberately keeps
// the separate exact f64 point_key above; casting must never weaken R9.
type CanonicalPoint = [u32; 3];
type CanonicalEdge = (CanonicalPoint, CanonicalPoint);
fn canonical(p: Point) -> Result<CanonicalPoint, Error> {
    let p = p.map(|v| v as f32);
    if p.iter().any(|v| !v.is_finite()) {
        return Err(Error::unsupported(
            "interfaceSegments: common-frame position exceeds canonical f32 range",
        ));
    }
    Ok(p.map(|v| if v == 0. { 0 } else { v.to_bits() }))
}
fn point_bytes(p: CanonicalPoint) -> Vec<u8> {
    p.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn edge_bytes(e: CanonicalEdge) -> Vec<u8> {
    e.0.into_iter()
        .chain(e.1)
        .flat_map(u32::to_le_bytes)
        .collect()
}
fn read_point(bytes: &[u8]) -> Result<CanonicalPoint, Error> {
    if bytes.len() != 12 {
        return Err(Error::InvalidState(
            "corrupt canonical point scratch".into(),
        ));
    }
    Ok(std::array::from_fn(|i| {
        u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap())
    }))
}
fn read_edge(bytes: &[u8]) -> Result<CanonicalEdge, Error> {
    if bytes.len() != 24 {
        return Err(Error::InvalidState("corrupt canonical edge scratch".into()));
    }
    Ok((read_point(&bytes[..12])?, read_point(&bytes[12..])?))
}
/// A coarse payload's declared count of dropped child components (D11 coarse drops),
/// `asset.extras.rustyTiles.droppedComponents`; absent is zero.
fn declared_drops(raw: &RawValue) -> Result<u64, Error> {
    let value = identity::field(raw, "asset")
        .and_then(|r| identity::field(r, "extras"))
        .and_then(|r| identity::field(r, "rustyTiles"))
        .and_then(|r| identity::field(r, "droppedComponents"));
    match value {
        None => Ok(0),
        Some(v) => serde_json::from_str::<u64>(v.get()).map_err(|_| {
            bad(
                "coarseDrops",
                "droppedComponents is not an unsigned integer",
            )
        }),
    }
}
/// A coarse payload's last-resort declaration (D1/D11 amendment A2),
/// `asset.extras.rustyTiles.lastResort` {stage 0-3, foldFaces}; absent is none. Stage 0
/// is a tile that keeps folds admitted below it.
fn declared_last_resort(raw: &RawValue) -> Result<Option<(u8, u64)>, Error> {
    let value = identity::field(raw, "asset")
        .and_then(|r| identity::field(r, "extras"))
        .and_then(|r| identity::field(r, "rustyTiles"))
        .and_then(|r| identity::field(r, "lastResort"));
    let Some(v) = value else { return Ok(None) };
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct LastResort {
        stage: u8,
        fold_faces: u64,
    }
    match serde_json::from_str::<LastResort>(v.get()) {
        Ok(d) if d.stage <= 3 => Ok(Some((d.stage, d.fold_faces))),
        _ => Err(bad(
            "lastResort",
            "lastResort is not {stage: 0-3, foldFaces: unsigned integer}",
        )),
    }
}
/// Faces on a fold, from the decoded surface alone (the producer's
/// `folds::fold_faces`): faces of a coincident group, and faces using an edge that two
/// or more faces use in the same direction.
fn fold_faces(surface: &Surface) -> u64 {
    let mut groups = BTreeMap::<[CanonicalPoint; 3], u64>::new();
    for f in &surface.faces {
        let mut k = *f;
        k.sort_unstable();
        *groups.entry(k).or_default() += 1;
    }
    let directed = |a: CanonicalPoint, b: CanonicalPoint| {
        let e = &surface.edges[&if a < b { (a, b) } else { (b, a) }];
        if a < b {
            e.forward
        } else {
            e.reverse
        }
    };
    surface
        .faces
        .iter()
        .filter(|f| {
            let mut k = **f;
            k.sort_unstable();
            groups[&k] > 1 || (0..3).any(|j| directed(f[j], f[(j + 1) % 3]) > 1)
        })
        .count() as u64
}
fn add_count(value: &mut u64, n: u64) -> Result<(), Error> {
    *value = value
        .checked_add(n)
        .ok_or_else(|| Error::limit("interfaceSegments: incidence counter exceeds native range"))?;
    Ok(())
}
#[derive(Clone, Copy, Debug, Default)]
struct Incidence {
    forward: u64,
    reverse: u64,
}
impl Incidence {
    fn directions(self) -> i64 {
        i64::from(self.forward != 0) | (i64::from(self.reverse != 0) << 1)
    }
}
#[derive(Default, Debug)]
struct Surface {
    points: BTreeSet<CanonicalPoint>,
    edges: BTreeMap<CanonicalEdge, Incidence>,
    /// Nondegenerate faces by key (at most the capped payload's triangles), read
    /// only to place same-direction faces on the sides of an edge.
    faces: Vec<[CanonicalPoint; 3]>,
    triangles: u64,
    /// Coarse replay only ([`normal_inversion`]): nondegenerate faces whose primitive
    /// publishes NORMAL, and those with a corner NORMAL more than 90° from the face.
    normal_faces: u64,
    inverted_faces: u64,
    /// Why some primitive's NORMAL was not compared ([`normal_skip`] bits, OR'd).
    normal_skips: u8,
}
impl Surface {
    fn reserve(&self, bytes: usize, cap: usize) -> Result<(), Error> {
        if self
            .points
            .len()
            .saturating_mul(64)
            .saturating_add(self.edges.len().saturating_mul(128))
            .saturating_add(self.faces.len().saturating_mul(36))
            .saturating_add(bytes)
            > cap
        {
            return Err(Error::limit(
                "interfaceSegments: canonical replay workspace exceeds decoded document budget",
            ));
        }
        Ok(())
    }
    /// Another placement of the same tile: union of points, summed incidences.
    fn merge(&mut self, other: Surface, cap: usize) -> Result<(), Error> {
        for p in other.points {
            if self.points.insert(p) {
                self.reserve(64, cap)?;
            }
        }
        for (key, e) in other.edges {
            if !self.edges.contains_key(&key) {
                self.reserve(128, cap)?;
            }
            let mine = self.edges.entry(key).or_default();
            add_count(&mut mine.forward, e.forward)?;
            add_count(&mut mine.reverse, e.reverse)?;
        }
        self.reserve(other.faces.len().saturating_mul(36), cap)?;
        self.faces.extend(other.faces);
        add_count(&mut self.normal_faces, other.normal_faces)?;
        add_count(&mut self.inverted_faces, other.inverted_faces)?;
        self.normal_skips |= other.normal_skips;
        add_count(&mut self.triangles, other.triangles)
    }
    fn triangle(
        &mut self,
        xyz: [Point; 3],
        work: &mut identity::Work,
        cap: usize,
    ) -> Result<(), Error> {
        work.charge(12)?;
        add_count(&mut self.triangles, 1)?;
        let p = [canonical(xyz[0])?, canonical(xyz[1])?, canonical(xyz[2])?];
        // Three distinct collinear keys ARE a surface under the published
        // combinatorial contract. Only repeated canonical corners are excluded.
        if p[0] == p[1] || p[1] == p[2] || p[2] == p[0] {
            return Ok(());
        }
        self.reserve(36, cap)?;
        self.faces.push(p);
        for key in p {
            work.charge(2 * (self.points.len().max(2).ilog2() as usize + 1))?;
            if !self.points.contains(&key) {
                self.reserve(64, cap)?;
                self.points.insert(key);
            }
        }
        for j in 0..3 {
            let (a, b) = (p[j], p[(j + 1) % 3]);
            let forward = a < b;
            let key = if forward { (a, b) } else { (b, a) };
            work.charge(2 * (self.edges.len().max(2).ilog2() as usize + 1))?;
            if !self.edges.contains_key(&key) {
                self.reserve(128, cap)?;
                self.edges.insert(key, Incidence::default());
            }
            let e = self.edges.get_mut(&key).unwrap();
            add_count(
                if forward {
                    &mut e.forward
                } else {
                    &mut e.reverse
                },
                1,
            )?;
        }
        Ok(())
    }
}
/// Counts use decoded leaf canonical f32 keys, not unavailable source positions.
/// Each distinct shared key is counted once globally; face incidence is never
/// cancelled. Omission and frontier counts are derived from the delivered
/// hierarchy alone (never from producer reports); every payload is within the
/// hard triangle cap, so only the largest is reported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DecodedInterfaceCensus {
    pub(super) shared_points: u64,
    pub(super) shared_segments: u64,
    pub(super) source_open_edges: u64,
    pub(super) source_open_edges_with_shared_endpoints: u64,
    pub(super) max_payload_triangles: u64,
    pub(super) omission: Omission,
    /// D11 arcs delivered as one chord over two or more leaf edges, counted per
    /// tile, and the leaf border vertices they contract.
    pub(super) contracted_border_arcs: u64,
    pub(super) contracted_border_vertices: u64,
    /// D11 coarse drops: child components a coarse payload no longer holds, recounted
    /// and checked against its declaration and geometricError, by cell depth.
    pub(super) dropped_by_depth: Vec<u64>,
    /// D5 amendment A3: refinable content tiles, and those whose G is within 1% of the
    /// texture-implied error (0 without a declaration).
    pub(super) texture_refinable_tiles: u64,
    pub(super) texture_bound_tiles: u64,
    /// Coarse NORMAL census (a warning, never a failure).
    pub(super) coarse_normals: CoarseNormals,
}
/// Coarse tiles publishing NORMAL, and those with faces some corner NORMAL of which points
/// more than 90° from the face (the face is shaded against its own winding), by cell depth.
/// Producers since #336 keep every corner within 60°; earlier coarse tiles averaged every
/// face at a vertex. Counted over all placements of a tile, like its triangles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CoarseNormals {
    pub(super) tiles: u64,
    pub(super) faces: u64,
    /// Coarse tiles with a NORMAL this comparison does not read ([`normal_skip`]), and
    /// the reasons met (bits of [`NORMAL_SKIPS`]).
    pub(super) skipped_tiles: u64,
    pub(super) skip_reasons: u8,
    pub(super) inverted_tiles_by_depth: Vec<u64>,
    pub(super) inverted_faces_by_depth: Vec<u64>,
}
/// Payload-element units charged for each scratch-store row a validation writes (sorted and
/// appended in key order, `bulk`), and for each ancestor-walk or placement-frame lookup. A row
/// costs far more than an ordinary payload unit (about 6 ns), so it is weighted to keep every
/// work class within the stated time per unit (`docs/architecture/validator-limits-amendment.md`,
/// time statement).
pub(crate) const ROW_UNITS: usize = 128;
/// Units for each seek of a keyed lookup into a complete leaf table (descendant incidences,
/// edge faces and coincident faces of a coarse tile, a coarse drop's border contacts). A seek
/// reads pages the cache may not hold: 1.7 µs at 1 M rows, 3.0 µs at 64 M, about 3.4 µs
/// extrapolated to the 600 M rows a 1 GB archive's budget can write.
pub(super) const SEEK_UNITS: usize = 320;
/// Units for each further row a keyed range query reads after its seek (charged as a row),
/// where the same rows can be read again: descendant leaf rows are read once per coarse
/// ancestor.
pub(super) const READ_UNITS: usize = 32;
/// Units for each open segment's shared-endpoint check: a merge step, and for a shared lower
/// endpoint one sequence record, sorted and merged once (about 190 ns measured).
pub(super) const OPEN_UNITS: usize = 32;
/// Whether a tile has a descendant declared at a fold stage: one seek past it in
/// `fold_stages`, bounded by its preorder `last`.
const FOLD_STAGE_BELOW: &str = "SELECT EXISTS(SELECT 1 FROM tiles t,fold_stages f
     WHERE t.id=?1 AND f.tile>t.id AND f.tile<=t.last)";
/// The three geometry replay queries (each also run as its own lookahead, `replay`).
const LEAVES: &str = "SELECT tile,rank,frame FROM placements WHERE leaf=1 ORDER BY id";
const EDGE_LEAVES: &str = "SELECT p.tile,p.rank,p.frame FROM placements p WHERE p.leaf=1 AND p.tile IN (SELECT tile FROM replay_tiles) ORDER BY p.id";
const COARSE: &str = "SELECT tile,rank,frame FROM placements WHERE leaf=0 ORDER BY tile,id";
/// Units for each original vertex row appended to the spill sequence at ingest.
pub(super) const ORIGINAL_UNITS: usize = 16;
/// Units for each placed original key: placement, external sort and ordered scan.
pub(super) const ALIAS_UNITS: usize = 32;
/// Leaf placement frames of one payload held at once by the alias pass; a payload with
/// more leaf placements re-reads its original rows once per further chunk.
const ALIAS_FRAMES: usize = 64;

/// One original vertex of one payload in its payload frame (f64 bits), recorded at ingest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Original {
    rank: u64,
    source: Source,
    vertex: u64,
    point: [u64; 3],
}
/// One original vertex at one leaf placement. Sorted by (source, vertex, exact), so
/// distinct exact placed positions of one vertex are adjacent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Alias {
    source: Source,
    vertex: u64,
    exact: [u64; 3],
}
fn put(out: &mut [u8], values: impl IntoIterator<Item = u64>) {
    for (chunk, v) in out.as_chunks_mut::<8>().0.iter_mut().zip(values) {
        *chunk = v.to_le_bytes();
    }
}
fn take<const N: usize>(bytes: &[u8]) -> [u64; N] {
    let (words, _) = bytes.as_chunks::<8>();
    std::array::from_fn(|i| u64::from_le_bytes(words[i]))
}
impl Record for Original {
    const SIZE: usize = 80;
    fn encode(&self, out: &mut [u8]) {
        put(
            out,
            std::iter::once(self.rank)
                .chain(self.source)
                .chain([self.vertex])
                .chain(self.point),
        );
    }
    fn decode(bytes: &[u8]) -> Self {
        let v = take::<10>(bytes);
        Self {
            rank: v[0],
            source: [v[1], v[2], v[3], v[4], v[5]],
            vertex: v[6],
            point: [v[7], v[8], v[9]],
        }
    }
}
impl Record for Alias {
    const SIZE: usize = 72;
    fn encode(&self, out: &mut [u8]) {
        put(
            out,
            self.source
                .into_iter()
                .chain([self.vertex])
                .chain(self.exact),
        );
    }
    fn decode(bytes: &[u8]) -> Self {
        let v = take::<9>(bytes);
        Self {
            source: [v[0], v[1], v[2], v[3], v[4]],
            vertex: v[5],
            exact: [v[6], v[7], v[8]],
        }
    }
}
/// Hard published triangle cap of every HLOD payload (D1), leaf or coarse.
pub(super) const TRIANGLE_CAP: u64 = 8192;
/// A content-less cell's geometric error is at least this multiple of its box
/// diagonal, so CesiumJS (maximum SSE 16) refines it while it covers a pixel.
pub(super) const EMPTY_PARENT_SCALE: f64 = 16.;
/// Content-less cells of the delivered octree. External-link and implicit-root
/// placeholders are transparent: they are not cells and add no cell depth. A
/// source is an omitted cell whose cell children all render; the rest are
/// omitted only because a cell child is. The first renderable cover holds every
/// content tile without a content ancestor; each renderable leaf lies below
/// exactly one of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Omission {
    pub(super) omitted_cells: u64,
    pub(super) omission_sources: u64,
    pub(super) omitted_by_depth: Vec<u64>,
    pub(super) frontier_tiles: u64,
    pub(super) frontier_leaves: u64,
}
/// Units for each face whose corner normals are compared with it (nine decoded components
/// and a few products; [`normal_inversion`]).
const NORMAL_UNITS: usize = 6;
/// The cofactor of a node chain's linear part (columns `c1×c2, c2×c0, c0×c1`): for any
/// nonsingular `L`, `(La)×(Lb) = cof(L)(a×b)` and the rendered normal is parallel to
/// `cof(L)·n`, so the sign of their dot product is the rendered one, reflections included.
fn cofactor(m: Matrix) -> [Point; 3] {
    let c: [Point; 3] = std::array::from_fn(|k| [m[4 * k], m[4 * k + 1], m[4 * k + 2]]);
    [cross(c[1], c[2]), cross(c[2], c[0]), cross(c[0], c[1])]
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn apply(columns: &[Point; 3], v: Point) -> Point {
    std::array::from_fn(|i| columns[0][i] * v[0] + columns[1][i] * v[1] + columns[2][i] * v[2])
}
fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
/// Whether some corner NORMAL of a face points more than 90° from the face's own normal
/// (wound by the stored corners): the face would be shaded against its winding. `None` for
/// a face without a normal (zero area). Corners with a zero NORMAL are not compared.
fn normal_inversion(cof: &[Point; 3], xyz: [Point; 3], normals: [Point; 3]) -> Option<bool> {
    let e = |k: usize| std::array::from_fn(|i| xyz[k][i] - xyz[0][i]);
    let face = apply(cof, cross(e(1), e(2)));
    (dot(face, face) > 0.).then(|| normals.iter().any(|n| dot(face, apply(cof, *n)) < 0.))
}
/// Reasons a coarse NORMAL is not compared, as bits, with their `notInspected` suffixes
/// (`coarseNormalInversion.<suffix>`).
pub(super) const NORMAL_SKIPS: [(u8, &str); 4] = [
    (1, "meshoptFilteredNormal"),
    (2, "sparseNormal"),
    (4, "viewlessNormal"),
    (8, "unsupportedNormalAccessor"),
];
/// The reason (a [`NORMAL_SKIPS`] bit) a coarse NORMAL accessor is outside what the
/// comparison reads exactly, or 0 when it is compared: a VEC3 of f32, or of normalized
/// i8/i16, one per vertex, in a view without a meshopt filter (this decoder undoes none).
/// Decided from the document alone, before any decode, so this warning-only check never
/// refuses a payload that the earlier checks admit (for example viewless zero storage).
fn normal_skip(doc: &Value, attribute: &Value, vertices: usize) -> u8 {
    let Some(a) = attribute.as_u64().map(|i| &doc["accessors"][i as usize]) else {
        return 8;
    };
    if a.get("sparse").is_some() {
        return 2;
    }
    let Some(view) = a.get("bufferView").and_then(Value::as_u64) else {
        return 4;
    };
    let normalized = a["normalized"] == true;
    let component = match a["componentType"].as_u64() {
        Some(5126) => !normalized,
        Some(5120 | 5122) => normalized,
        _ => false,
    };
    if a["type"] != "VEC3" || !component || a["count"].as_u64() != Some(vertices as u64) {
        return 8;
    }
    let filter =
        &doc["bufferViews"][view as usize]["extensions"]["EXT_meshopt_compression"]["filter"];
    if filter.as_str().is_some_and(|f| f != "NONE") {
        return 1;
    }
    0
}
/// `normals`: also compare each face's corner NORMALs with the face (coarse payloads only;
/// leaves carry source normals as authored).
fn replay_surface(
    bytes: &[u8],
    frame: Matrix,
    cap: usize,
    effort: &Effort,
    normals: bool,
) -> Result<Surface, Error> {
    let base = effort
        .base
        .saturating_add(effort.used.load(Ordering::Relaxed));
    let (surface, used) = replay_units(bytes, frame, cap, effort.budget, base, normals)?;
    effort.charge(usize::try_from(used).map_err(|_| super::refused())?)?;
    Ok(surface)
}
/// [`replay_surface`] on a worker (#342): the surface and its work units, charged from zero;
/// the pass charges them in order ([`replay::Lookahead`]).
fn replay_surface_free(
    bytes: &[u8],
    frame: Matrix,
    cap: usize,
    budget: Budget,
    normals: bool,
) -> Result<(Surface, u64), Error> {
    replay_units(bytes, frame, cap, budget, 0, normals)
}
/// The replayed surface and the work units of its document, charged from `base`.
fn replay_units(
    bytes: &[u8],
    frame: Matrix,
    cap: usize,
    budget: Budget,
    base: u64,
    normals: bool,
) -> Result<(Surface, u64), Error> {
    let (doc, bin, mut work) = replay_document(bytes, cap, budget, base)?;
    let nodes = doc["nodes"]
        .as_array()
        .ok_or_else(|| bad("interfaceSegments", "missing replay nodes"))?;
    let scene = doc["scene"].as_u64().unwrap_or(0) as usize;
    let roots = doc["scenes"][scene]["nodes"]
        .as_array()
        .ok_or_else(|| bad("interfaceSegments", "missing replay scene"))?;
    let mut surface = Surface::default();
    let mut stack = vec![(roots.as_slice(), 0usize, IDENTITY)];
    while let Some((children, next, parent)) = stack.last_mut() {
        if *next == children.len() {
            stack.pop();
            continue;
        }
        let id = identity::uint(&children[*next])?;
        *next += 1;
        work.charge(1)?;
        let node = nodes
            .get(id)
            .ok_or_else(|| bad("interfaceSegments", "replay node outside document"))?;
        let m = multiply(*parent, node_matrix(node)?);
        if let Some(mesh) = node.get("mesh") {
            for p in doc["meshes"][identity::uint(mesh)?]["primitives"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let positions = Accessor::new(
                    &doc,
                    bin,
                    identity::uint(&p["attributes"]["POSITION"])?,
                    &mut work,
                )?;
                let indices = p
                    .get("indices")
                    .map(|i| Accessor::new(&doc, bin, identity::uint(i)?, &mut work))
                    .transpose()?;
                let count = indices.as_ref().map_or(positions.count, |i| i.count);
                let normal = match p["attributes"].get("NORMAL") {
                    Some(n) if normals => match normal_skip(&doc, n, positions.count) {
                        0 => {
                            let a = Accessor::new(&doc, bin, identity::uint(n)?, &mut work)?;
                            Some((a, cofactor(m)))
                        }
                        reason => {
                            surface.normal_skips |= reason;
                            None
                        }
                    },
                    _ => None,
                };
                for corner in (0..count).step_by(3) {
                    let mut xyz = [[0.; 3]; 3];
                    let mut ids = [0; 3];
                    for (j, p) in xyz.iter_mut().enumerate() {
                        let id = match &indices {
                            Some(i) => i.index(corner + j)?,
                            None => corner + j,
                        };
                        ids[j] = id;
                        // Preserve scene-then-reference f64 evaluation order.
                        *p = point(frame, positions.position(id, m)?);
                        finite(p)?;
                    }
                    surface.triangle(xyz, &mut work, cap)?;
                    if let Some((n, cof)) = &normal {
                        work.charge(NORMAL_UNITS)?;
                        let local = [
                            positions.vec3(ids[0])?,
                            positions.vec3(ids[1])?,
                            positions.vec3(ids[2])?,
                        ];
                        let corners = [n.vec3(ids[0])?, n.vec3(ids[1])?, n.vec3(ids[2])?];
                        if let Some(inverted) = normal_inversion(cof, local, corners) {
                            add_count(&mut surface.normal_faces, 1)?;
                            add_count(&mut surface.inverted_faces, u64::from(inverted))?;
                        }
                    }
                }
            }
        }
        if let Some(children) = node["children"].as_array() {
            stack.push((children.as_slice(), 0, m));
        }
    }
    Ok((surface, work.used))
}

struct Effort {
    // Unweighted counts: all filter attempts and the subset reaching intervals.
    // Kept separate so validator work calibration can price the two paths.
    certificate_fast: AtomicU64,
    certificate_slow: AtomicU64,
    budget: Budget,
    base: u64,
    used: Arc<AtomicU64>,
    /// Progress and cancellation checkpoints: every charge is one (#342).
    ticks: ticks::Ticks,
}
impl Effort {
    fn charge(&self, n: usize) -> Result<(), Error> {
        let old = self.used.fetch_add(n as u64, Ordering::Relaxed);
        self.budget
            .charge(self.base.saturating_add(old), n as u64, || {
                "interfaceSegments: native delivery work exceeds payload element budget".into()
            })?;
        self.ticks.check()
    }
}

/// The disk store once it exists; every query path runs after `materialize`.
struct Db(Option<Connection>);
impl std::ops::Deref for Db {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.0
            .as_ref()
            .expect("HLOD delivery store is used before it is materialized")
    }
}
/// Rows registered before any HLOD identity is known. Archives without it never
/// create the disk store or report its scratch; a bounded buffer otherwise moves
/// to disk at the first identity payload, expectation or overflow.
#[derive(Default)]
struct Pending {
    tiles: Vec<PendingTile>,
    index: std::collections::HashMap<u64, usize>,
    refs: Vec<(u64, u64)>,
    payloads: Vec<(u64, bool, bool, Vec<u8>)>,
}
struct PendingTile {
    id: u64,
    parent: Option<u64>,
    world: Matrix,
    refine: Option<bool>,
    placeholder: bool,
    error: Option<f64>,
    diagonal: Option<f64>,
    metadata: Option<recovery::Metadata>,
}
/// Buffered rows (tiles, references and payloads) before the store moves to disk.
const PENDING_ROWS: usize = 16_384;

/// One validation's disk hierarchy and decoded boundary index. Field order closes
/// SQLite before removing its private file and the existing scratch anchor.
pub(super) struct Store {
    // Registered cardinalities bound recovery SQL before it runs, including refusals.
    recovery_rows: (usize, usize),
    connection: Db,
    /// Places SQLite's temporary files in the scratch directory; dropped after `connection`.
    _vfs: Option<vfs::ScratchVfs>,
    /// Why it could not, reported once as a warning event by `finish`.
    vfs_fallback: Option<String>,
    _file: Option<tempfile::NamedTempFile>,
    _anchor: Option<SlotFile>,
    _scratch: Arc<Scratch>,
    pending: Pending,
    pub(super) checked: bool,
    common_root: Option<Matrix>,
    profile_exclusion: Option<&'static str>,
    /// The declared texture-error rule (D5 amendment A3).
    texture: texture::Declaration,
    decoded_limit: usize,
    census: Option<DecodedInterfaceCensus>,
    /// Original vertex rows, appended per payload at ingest; the alias pass consumes them.
    originals: Option<SequenceWriter<Original>>,
    sort: SortConfig,
    buffer_bytes: usize,
    resources: Resources,
    pool: replay::Pool,
}
/// The sizes of one store, from the validation's memory plan (#342).
#[derive(Clone, Copy, Debug)]
pub(super) struct Resources {
    /// The store's external sorts; `None` keeps the catalog's side sorts.
    pub(super) sort: Option<SortConfig>,
    /// SQLite page cache; the sorter's in-memory list is bounded by the same size.
    pub(super) cache_bytes: u64,
    /// SQLite temporary tables and sorts in memory: only when their row bound fits the plan.
    pub(super) temp_in_memory: bool,
    /// Geometry replay workers (1: sequential) and their in-flight stored bytes.
    pub(super) workers: usize,
    pub(super) window_bytes: u64,
}
impl Resources {
    /// The 0.4.0 sizes: an 8 MiB cache, a disk temporary store and one worker.
    #[cfg(test)]
    pub(super) const FIXED: Self = Self {
        sort: None,
        cache_bytes: 8 << 20,
        temp_in_memory: false,
        workers: 1,
        window_bytes: 0,
    };
}
/// The open-segment endpoint merge of `segment_requirements`.
struct OpenEnds<'a> {
    lower: bulk::LowerEnds<'a>,
    higher: SequenceWriter<bulk::End>,
}
impl Store {
    pub(super) fn new(
        scratch: Arc<Scratch>,
        config: &CatalogConfig,
        resources: Resources,
    ) -> Result<Self, Error> {
        Ok(Self {
            recovery_rows: (0, 0),
            connection: Db(None),
            _vfs: None,
            vfs_fallback: None,
            _file: None,
            _anchor: None,
            _scratch: scratch,
            pending: Pending::default(),
            checked: false,
            common_root: None,
            profile_exclusion: None,
            texture: texture::Declaration::Absent,
            decoded_limit: 0,
            census: None,
            originals: None,
            sort: resources.sort.unwrap_or(config.side_sort),
            buffer_bytes: config.buffer_bytes,
            pool: replay::Pool::new(resources.workers, resources.window_bytes),
            resources,
        })
    }
    /// Create the disk store and replay the pending rows into it, once.
    fn materialize(&mut self) -> Result<(), Error> {
        if self.connection.0.is_some() {
            return Ok(());
        }
        let anchor = SlotFile::new(&self._scratch, "hlod-delivery", 1).map_err(spill_failure)?;
        let file = tempfile::Builder::new()
            .prefix("hlod-delivery-")
            .tempfile_in(self._scratch.path())
            .map_err(|e| Error::Scratch(ScratchRefusal::Io(e)))?;
        // SQLite's sorter runs, temporary tables and journals go to the scratch directory too,
        // the volume the start-of-run bound checked (review P2-2).
        let scratch_vfs = vfs::ScratchVfs::new(self._scratch.path());
        let connection = match &scratch_vfs {
            Some(v) => Connection::open_with_flags_and_vfs(
                file.path(),
                rusqlite::OpenFlags::default(),
                v.name(),
            ),
            None => Connection::open(file.path()),
        }
        .map_err(sql)?;
        connection
            .execute_batch(&format!(
                "PRAGMA cache_size=-{}; PRAGMA temp_store={};",
                (self.resources.cache_bytes >> 10).max(1),
                if self.resources.temp_in_memory {
                    "MEMORY"
                } else {
                    "FILE"
                }
            ))
            .map_err(sql)?;
        connection.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;
            CREATE TABLE tiles(id INTEGER PRIMARY KEY,parent INTEGER,world BLOB NOT NULL,refine INTEGER,placeholder INTEGER NOT NULL DEFAULT 0,last INTEGER,leaf INTEGER,error REAL,diagonal REAL,cell INTEGER,rank_first INTEGER,ownership_depth INTEGER,ownership_invalid INTEGER NOT NULL DEFAULT 0,implicit_root INTEGER NOT NULL DEFAULT 0,recovery TEXT);
            CREATE TABLE recovery_cells(id INTEGER PRIMARY KEY,first INTEGER NOT NULL,last INTEGER NOT NULL);
            CREATE TABLE refs(tile INTEGER NOT NULL,rank INTEGER NOT NULL);
            CREATE TABLE payloads(rank INTEGER PRIMARY KEY,identity_present INTEGER NOT NULL,triangle_present INTEGER NOT NULL,primitive_rows INTEGER NOT NULL DEFAULT 0,triangles INTEGER NOT NULL DEFAULT 0,digest BLOB NOT NULL,dropped INTEGER NOT NULL DEFAULT 0,last_stage INTEGER NOT NULL DEFAULT 0,fold_faces INTEGER NOT NULL DEFAULT 0,recovery INTEGER NOT NULL DEFAULT 0,texture_area REAL NOT NULL DEFAULT 0,texture_texels REAL NOT NULL DEFAULT 0);
            CREATE TABLE placements(id INTEGER PRIMARY KEY,tile INTEGER NOT NULL,rank INTEGER NOT NULL,frame BLOB NOT NULL,leaf INTEGER NOT NULL);
            CREATE TABLE leaf_points(key BLOB NOT NULL,tile INTEGER NOT NULL,PRIMARY KEY(key,tile)) WITHOUT ROWID;
            CREATE TABLE leaf_segments(key BLOB NOT NULL,tile INTEGER NOT NULL,nf INTEGER NOT NULL,nr INTEGER NOT NULL,PRIMARY KEY(key,tile)) WITHOUT ROWID;
            CREATE TABLE shared_point_keys(key BLOB PRIMARY KEY) WITHOUT ROWID;
            CREATE TABLE requirements(tile INTEGER NOT NULL,kind INTEGER NOT NULL,key BLOB NOT NULL,directions INTEGER NOT NULL,PRIMARY KEY(tile,kind,key)) WITHOUT ROWID;
            CREATE TABLE leaf_faces(corners BLOB NOT NULL,tile INTEGER NOT NULL,n INTEGER NOT NULL,PRIMARY KEY(corners,tile)) WITHOUT ROWID;
            CREATE TABLE coarse_faces(tile INTEGER NOT NULL,corners BLOB NOT NULL);
            CREATE INDEX coarse_faces_tile ON coarse_faces(tile);
            CREATE TABLE multi_edges(key BLOB PRIMARY KEY) WITHOUT ROWID;
            CREATE TABLE edge_faces(key BLOB NOT NULL,tile INTEGER NOT NULL,third BLOB NOT NULL,dir INTEGER NOT NULL);
            CREATE INDEX edge_faces_key ON edge_faces(key,dir,tile,third);
            CREATE TABLE twin_points(key BLOB NOT NULL,corners BLOB NOT NULL,PRIMARY KEY(key,corners)) WITHOUT ROWID;
            CREATE TABLE border_map(key BLOB NOT NULL,tile INTEGER NOT NULL,image BLOB NOT NULL,d REAL NOT NULL,PRIMARY KEY(key,tile)) WITHOUT ROWID;
            CREATE TABLE fold_stages(tile INTEGER PRIMARY KEY);
            BEGIN;").map_err(sql)?;
        let pending = std::mem::take(&mut self.pending);
        for t in &pending.tiles {
            connection
                .execute(
                    "INSERT INTO tiles(id,parent,world,refine,placeholder,error,diagonal,rank_first,ownership_depth,ownership_invalid,implicit_root,recovery) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                    params![
                        integer(t.id)?,
                        t.parent.map(integer).transpose()?,
                        encode(&t.world),
                        t.refine,
                        t.placeholder,
                        t.error,
                        t.diagonal,
                        t.metadata.as_ref().and_then(|m| m.owner.map(|k| k.0)).map(integer).transpose()?,
                        t.metadata.as_ref().and_then(|m| m.owner.map(|k| k.1)),
                        t.metadata.as_ref().is_some_and(|m| m.invalid_owner),
                        t.metadata.as_ref().is_some_and(|m| m.implicit_root),
                        t.metadata.as_ref().and_then(|m| m.recovery.as_deref())
                    ],
                )
                .map_err(sql)?;
        }
        for &(tile, rank) in &pending.refs {
            connection
                .execute(
                    "INSERT INTO refs VALUES(?,?)",
                    params![integer(tile)?, integer(rank)?],
                )
                .map_err(sql)?;
        }
        for (rank, identity, triangle, digest) in &pending.payloads {
            connection
                .execute(
                    "INSERT INTO payloads(rank,identity_present,triangle_present,digest) VALUES(?,?,?,?)",
                    params![integer(*rank)?, identity, triangle, digest],
                )
                .map_err(sql)?;
        }
        self.connection = Db(Some(connection));
        if scratch_vfs.is_none() {
            self.vfs_fallback = Some(vfs::ScratchVfs::fallback_reason(self._scratch.path()));
        }
        self._vfs = scratch_vfs;
        self._file = Some(file);
        self._anchor = Some(anchor);
        Ok(())
    }
    /// Move to disk when the bounded buffer is full.
    fn bound_pending(&mut self) -> Result<(), Error> {
        let p = &self.pending;
        if p.tiles.len() + p.refs.len() + p.payloads.len() >= PENDING_ROWS {
            self.materialize()?;
        }
        Ok(())
    }
    pub(super) fn tile(
        &mut self,
        id: u64,
        parent: Option<u64>,
        local: Matrix,
    ) -> Result<(), Error> {
        self.recovery_rows.0 = self.recovery_rows.0.saturating_add(1);
        finite(&local)?;
        if !nonsingular(local) {
            self.exclude_scene("singular tile placement is outside the decoded interface profile");
        }
        let world = if let Some(p) = parent {
            let world = match self.pending.index.get(&p) {
                Some(&i) if self.connection.0.is_none() => self.pending.tiles[i].world,
                _ => {
                    let bytes: Vec<u8> = self
                        .connection
                        .query_row("SELECT world FROM tiles WHERE id=?", [integer(p)?], |r| {
                            r.get(0)
                        })
                        .map_err(sql)?;
                    decode::<16>(&bytes)?
                }
            };
            multiply(world, local)
        } else {
            if self.common_root.is_some_and(|m| m != local) {
                self.exclude_scene(
                    "distinct root placements have no common decoded interface frame",
                );
            }
            self.common_root = Some(local);
            // Drop one shared invertible placement before any arithmetic. This
            // avoids ECEF translation erasing a small decoded gap by cancellation.
            IDENTITY
        };
        finite(&world)?;
        if self.connection.0.is_none() {
            self.pending.index.insert(id, self.pending.tiles.len());
            self.pending.tiles.push(PendingTile {
                id,
                parent,
                world,
                refine: None,
                placeholder: false,
                error: None,
                diagonal: None,
                metadata: None,
            });
            return self.bound_pending();
        }
        self.connection
            .execute(
                "INSERT INTO tiles(id,parent,world) VALUES(?,?,?)",
                params![
                    integer(id)?,
                    parent.map(integer).transpose()?,
                    encode(&world)
                ],
            )
            .map_err(sql)?;
        Ok(())
    }
    pub(super) fn kind(
        &mut self,
        id: u64,
        refine: Option<bool>,
        placeholder: bool,
        error: f64,
        diagonal: Option<f64>,
    ) -> Result<(), Error> {
        if self.connection.0.is_none() {
            let i = *self
                .pending
                .index
                .get(&id)
                .ok_or_else(|| Error::InvalidState("delivery kind before its tile".into()))?;
            let t = &mut self.pending.tiles[i];
            (t.refine, t.placeholder, t.error, t.diagonal) =
                (refine, placeholder, Some(error), diagonal);
            return Ok(());
        }
        self.connection
            .execute(
                "UPDATE tiles SET refine=?,placeholder=?,error=?,diagonal=? WHERE id=?",
                params![refine, placeholder, error, diagonal, integer(id)?],
            )
            .map_err(sql)?;
        Ok(())
    }
    pub(super) fn reference(&mut self, id: u64, rank: u64) -> Result<(), Error> {
        self.recovery_rows.1 = self.recovery_rows.1.saturating_add(1);
        if self.connection.0.is_none() {
            self.pending.refs.push((id, rank));
            return self.bound_pending();
        }
        self.connection
            .execute(
                "INSERT INTO refs VALUES(?,?)",
                params![integer(id)?, integer(rank)?],
            )
            .map_err(sql)?;
        Ok(())
    }
    pub(super) fn ingest(
        &mut self,
        rank: u64,
        bytes: &[u8],
        budget: Budget,
        already: u64,
        decoded_limit: usize,
    ) -> Result<u64, Error> {
        self.decoded_limit = self.decoded_limit.max(decoded_limit);
        // Byte identity: a payload copied under another member name is the same
        // payload for the single-cover check.
        let digest = {
            use sha2::Digest;
            sha2::Sha256::digest(bytes).to_vec()
        };
        let embedded = bytes.get(..4) == Some(b"glTF");
        let length = if embedded {
            u32::from_le_bytes(identity::slice(bytes, 12, 4)?.try_into().unwrap()) as usize
        } else {
            0
        };
        // Non-GLB content is recorded as lacking identity. The final archive-wide
        // expectation cannot depend on whether a stripped payload arrived first.
        let raw = if embedded {
            serde_json::from_slice::<&RawValue>(identity::slice(bytes, 20, length)?)?
        } else if bytes
            .first()
            .is_some_and(|b| matches!(b, b'{' | b' ' | b'\n' | b'\r' | b'\t'))
        {
            match serde_json::from_slice::<&RawValue>(bytes) {
                Ok(r) => r,
                Err(_) => {
                    self.payload_bytes(rank, false, false, &digest)?;
                    return Ok(0);
                }
            }
        } else {
            self.payload_bytes(rank, false, false, &digest)?;
            return Ok(0);
        };
        if !identity::declares(raw) {
            self.payload_bytes(rank, false, false, &digest)?;
            return Ok(0);
        }
        // Source identity is now expected: the hierarchy and payload rows go to disk.
        self.materialize()?;
        if !embedded {
            return Err(Error::unsupported(
                "hlodFeatureIds: native delivery identity requires embedded GLB",
            ));
        }
        let mut work = identity::Work::new(budget, already, decoded_limit);
        admit_scene_profile(raw, &mut work)?;
        let doc = identity::project(raw, "doc")?;
        let has_triangles = doc["extensions"]["EXT_structural_metadata"]["propertyTables"]
            .as_array()
            .is_some_and(|ts| ts.iter().any(|t| t["class"] == "source_triangle"));
        self.payload_bytes(rank, true, has_triangles, &digest)?;
        if let Some((stage, folds)) = declared_last_resort(raw)? {
            self.connection
                .execute(
                    "UPDATE payloads SET last_stage=?,fold_faces=? WHERE rank=?",
                    params![i64::from(stage) + 1, integer(folds)?, integer(rank)?],
                )
                .map_err(sql)?;
        }
        // Four nested raw-field scans, each bounded by the admitted JSON length.
        work.charge(
            raw.get()
                .len()
                .saturating_mul(4 * recovery::JSON_BYTE_UNITS),
        )?;
        let recovery = recovery::payload_marker(raw)?;
        if recovery {
            work.charge(recovery::METADATA_UNITS)?;
            self.connection
                .execute(
                    "UPDATE payloads SET recovery=1 WHERE rank=?",
                    [integer(rank)?],
                )
                .map_err(sql)?;
        }
        let dropped = declared_drops(raw)?;
        if dropped > 0 {
            self.connection
                .execute(
                    "UPDATE payloads SET dropped=? WHERE rank=?",
                    params![integer(dropped)?, integer(rank)?],
                )
                .map_err(sql)?;
        }
        let bin = identity::slice(
            bytes,
            28 + length,
            u32::from_le_bytes(identity::slice(bytes, 20 + length, 4)?.try_into().unwrap())
                as usize,
        )?;
        let mut originals = BTreeMap::<(Source, u64), Point>::new();
        // A3: base-colour pages and the payload's textured surface and texel areas.
        let pages = match self.texture {
            texture::Declaration::Rule(_) => texture::pages(&doc, bin, &mut work)?,
            texture::Declaration::Fast => Vec::new(),
            texture::Declaration::Absent => {
                if texture::textured(&doc, &mut work)? {
                    return Err(bad(
                        "textureError",
                        "a textured HLOD payload needs the tileset's textureError declaration",
                    ));
                }
                Vec::new()
            }
        };
        let (mut texture_area, mut texture_texels) = (0f64, 0f64);
        let tables = doc["extensions"]["EXT_structural_metadata"]["propertyTables"]
            .as_array()
            .ok_or_else(|| bad("hlodFeatureIds", "missing tables"))?;
        let mut used: Vec<Vec<bool>> = Vec::with_capacity(tables.len());
        let mut primitive_rows = 0usize;
        let mut triangles = 0u64;
        for t in tables {
            work.charge(1)?;
            if t["class"] == "source_primitive" {
                primitive_rows = primitive_rows.max(identity::uint(&t["count"])?);
            }
            let n = if t["class"] == "source_triangle" {
                identity::uint(&t["count"])?
            } else {
                0
            };
            work.charge(n)?;
            let mut v = Vec::new();
            v.try_reserve_exact(n).map_err(|_| super::refused())?;
            v.resize(n, false);
            used.push(v);
        }
        self.connection
            .execute(
                "UPDATE payloads SET primitive_rows=? WHERE rank=?",
                params![integer(primitive_rows as u64)?, integer(rank)?],
            )
            .map_err(sql)?;
        let nodes = doc["nodes"]
            .as_array()
            .ok_or_else(|| bad("interfaceSegments", "missing scene nodes"))?;
        let scene = doc["scene"].as_u64().unwrap_or(0) as usize;
        let roots = doc["scenes"][scene]["nodes"]
            .as_array()
            .ok_or_else(|| bad("interfaceSegments", "missing scene roots"))?;
        // Frames hold iterators into the admitted document, not copied subtree lists.
        let mut stack = vec![(roots.as_slice(), 0usize, IDENTITY)];
        while let Some((children, next, parent)) = stack.last_mut() {
            if *next == children.len() {
                stack.pop();
                continue;
            }
            let id = identity::uint(&children[*next])?;
            *next += 1;
            work.charge(1)?;
            let n = nodes
                .get(id)
                .ok_or_else(|| bad("interfaceSegments", "scene node outside document"))?;
            if n.get("skin").is_some()
                || n.get("weights").is_some()
                || n["extensions"].get("EXT_mesh_gpu_instancing").is_some()
            {
                return Err(Error::unsupported(
                    "interfaceSegments: deformed or instanced HLOD identity geometry",
                ));
            }
            let m = multiply(*parent, node_matrix(n)?);
            finite(&m)?;
            if let Some(mesh) = n.get("mesh") {
                let mesh = &doc["meshes"][identity::uint(mesh)?];
                if mesh.get("weights").is_some() {
                    return Err(Error::unsupported(
                        "interfaceSegments: morphed HLOD identity geometry",
                    ));
                }
                for p in mesh["primitives"].as_array().into_iter().flatten() {
                    if p.get("targets").is_some() {
                        return Err(Error::unsupported(
                            "interfaceSegments: morphed HLOD identity geometry",
                        ));
                    }
                    let positions = Accessor::new(
                        &doc,
                        bin,
                        identity::uint(&p["attributes"]["POSITION"])?,
                        &mut work,
                    )?;
                    let indices = p
                        .get("indices")
                        .map(|id| Accessor::new(&doc, bin, identity::uint(id)?, &mut work))
                        .transpose()?;
                    let primitive = Feature::new(&doc, bin, p, "source_primitive", &mut work)?
                        .ok_or_else(|| bad("hlodFeatureIds", "missing primitive identity"))?;
                    let triangle = Feature::new(&doc, bin, p, "source_triangle", &mut work)?;
                    let page = p["material"]
                        .as_u64()
                        .and_then(|m| pages.get(m as usize).copied().flatten());
                    let mut textured = match page {
                        Some((texels, set)) => {
                            let uv = Accessor::new(
                                &doc,
                                bin,
                                identity::uint(&p["attributes"][format!("TEXCOORD_{set}")])?,
                                &mut work,
                            )?;
                            if uv.components != 2 || uv.count != positions.count {
                                return Err(bad("textureError", "base-colour TEXCOORD shape"));
                            }
                            Some(texture::Sums {
                                uv,
                                texels,
                                area: &mut texture_area,
                                texel_area: &mut texture_texels,
                            })
                        }
                        None => None,
                    };
                    let count = indices.as_ref().map_or(positions.count, |a| a.count);
                    if count % 3 != 0 || p["mode"].as_u64().unwrap_or(4) != 4 {
                        return Err(bad("hlodFeatureIds", "identity primitive is not triangles"));
                    }
                    // Every scene instance of the primitive is published geometry.
                    add_count(&mut triangles, (count / 3) as u64)?;
                    for corner in (0..count).step_by(3) {
                        work.charge(12)?;
                        let mut ids = [0usize; 3];
                        for (j, id) in ids.iter_mut().enumerate() {
                            *id = match &indices {
                                Some(a) => a.index(corner + j)?,
                                None => corner + j,
                            };
                        }
                        let mut xyz = [[0.; 3]; 3];
                        for (j, p) in xyz.iter_mut().enumerate() {
                            *p = positions.position(ids[j], m)?;
                        }
                        if let Some(textured) = &mut textured {
                            textured.triangle(ids, xyz, &mut work)?;
                        }
                        let (_, source) = primitive.row(ids[0], &mut work)?;
                        if let Some(triangle) = &triangle {
                            let (row, key) = triangle.row(ids[0], &mut work)?;
                            if key != source {
                                return Err(bad(
                                    "sourceIdentity",
                                    "triangle original row disagrees with its source primitive",
                                ));
                            }
                            let seen = used
                                .get_mut(triangle.table)
                                .and_then(|v| v.get_mut(row))
                                .ok_or_else(|| {
                                    bad("sourceIdentity", "triangle row outside table")
                                })?;
                            if *seen {
                                return Err(bad("sourceIdentity","original triangle row is used by more than one decoded triangle"));
                            }
                            *seen = true;
                            let mut vertices = [0; 3];
                            for j in 0..3 {
                                vertices[j] = triangle.columns[6 + j].at(row)?;
                                let key = (source, vertices[j]);
                                if let Some(p) = originals.get(&key) {
                                    if point_key(*p) != point_key(xyz[j]) {
                                        return Err(bad("sourceIdentity","original vertex maps to inconsistent decoded leaf positions"));
                                    }
                                } else {
                                    if originals.len().saturating_mul(144) > decoded_limit {
                                        return Err(Error::limit("sourceIdentity: original vertex workspace exceeds decoded document budget"));
                                    }
                                    originals.insert(key, xyz[j]);
                                }
                            }
                        }
                    }
                }
            }
            if let Some(children) = n["children"].as_array() {
                stack.push((children.as_slice(), 0, m));
            }
        }
        self.connection
            .execute(
                "UPDATE payloads SET triangles=?,texture_area=?,texture_texels=? WHERE rank=?",
                params![
                    integer(triangles)?,
                    texture_area,
                    texture_texels,
                    integer(rank)?
                ],
            )
            .map_err(sql)?;
        if used.iter().any(|v| v.iter().any(|used| !*used)) {
            return Err(bad(
                "sourceIdentity",
                "original triangle table has unreferenced rows",
            ));
        }
        // One row per original vertex in the payload frame, contiguous per payload;
        // placement is applied by the alias pass.
        for (&(source, vertex), p) in &originals {
            work.charge(ORIGINAL_UNITS)?;
            self.original(Original {
                rank,
                source,
                vertex,
                point: p.map(f64::to_bits),
            })?;
        }
        Ok(work.used)
    }
    /// Analytic payload rows: distinct ranks have distinct bytes.
    #[cfg(test)]
    fn payload(&mut self, rank: u64, identity: bool, triangle: bool) -> Result<(), Error> {
        self.payload_bytes(rank, identity, triangle, &rank.to_be_bytes())
    }
    fn payload_bytes(
        &mut self,
        rank: u64,
        identity: bool,
        triangle: bool,
        digest: &[u8],
    ) -> Result<(), Error> {
        if self.connection.0.is_none() {
            self.pending
                .payloads
                .push((rank, identity, triangle, digest.to_vec()));
            return self.bound_pending();
        }
        self.connection
            .execute(
                "INSERT INTO payloads(rank,identity_present,triangle_present,digest) VALUES(?,?,?,?)",
                params![integer(rank)?, identity, triangle, digest],
            )
            .map_err(sql)?;
        Ok(())
    }
    /// Monotonic main-database length, for the report after successful finish.
    /// Add to managed scratch peak for a conservative combined peak. SQLite's
    /// transient sort files are outside this count; its page cache is separately capped.
    pub(super) fn disk_bytes(&self) -> u64 {
        self._file.as_ref().map_or(0, |file| {
            file.as_file()
                .metadata()
                .map_or(0, |metadata| metadata.len())
        })
    }

    /// A traversal-owned placement semantic (for example an up-axis override)
    /// can exclude interface coverage while ordinary archive checks still run.
    pub(super) fn exclude_scene(&mut self, reason: &'static str) {
        self.profile_exclusion.get_or_insert(reason);
    }

    /// An owned archive/report label can establish expectation even when every
    /// payload's identity was stripped. Call before finish; traversal order is immaterial.
    pub(super) fn expect_identity(&mut self) {
        self.checked = true;
    }
    /// Hold every content tile's G to the declared texture term (D5 amendment A3).
    pub(super) fn expect_texture(&mut self, declaration: texture::Declaration) {
        self.texture = declaration;
    }

    /// [`Store::finish_with`] over a plain read closure (unit tests).
    #[cfg(test)]
    pub(super) fn finish(
        &mut self,
        budget: Budget,
        already: u64,
        probe: Option<crate::runtime::AttemptProbe>,
        read: &mut dyn FnMut(u64) -> Result<Vec<u8>, Error>,
    ) -> Result<u64, Error> {
        self.finish_with(budget, already, probe, &mut replay::Plain(read))
    }
    pub(super) fn finish_with(
        &mut self,
        budget: Budget,
        already: u64,
        probe: Option<crate::runtime::AttemptProbe>,
        read: &mut dyn replay::Payloads,
    ) -> Result<u64, Error> {
        // Without HLOD identity nothing below applies: no store, no scratch.
        if self.connection.0.is_none() {
            if !self.checked && !self.pending.payloads.iter().any(|p| p.1) {
                return Ok(0);
            }
            self.materialize()?;
        }
        // Secondary indexes of rows inserted in traversal order are built once, by sort.
        self.connection
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS tile_parent ON tiles(parent);
                 CREATE INDEX IF NOT EXISTS refs_tile ON refs(tile);
                 CREATE INDEX IF NOT EXISTS refs_rank ON refs(rank);",
            )
            .map_err(sql)?;
        let used = Arc::new(AtomicU64::new(0));
        let effort = Effort {
            certificate_fast: AtomicU64::new(0),
            certificate_slow: AtomicU64::new(0),
            budget,
            base: already,
            used: used.clone(),
            ticks: ticks::Ticks::new(probe),
        };
        if let Some(reason) = self.vfs_fallback.take() {
            effort.ticks.warn("sqlite_temp_dir", &reason)?;
        }
        let hook = used.clone();
        let remaining = budget.value.saturating_sub(already);
        let mut heartbeat = effort.ticks.heartbeat();
        self.connection
            .progress_handler(
                10_000,
                Some(move || {
                    // One work unit per call, whatever the control's state; progress totals
                    // are not work.
                    let over =
                        heartbeat.charged() && hook.fetch_add(1, Ordering::Relaxed) >= remaining;
                    heartbeat.interrupt() || over
                }),
            )
            .map_err(sql)?;
        let result = self.finish_inner(&effort, read);
        self.connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(sql)?;
        if let Some(stopped) = effort.ticks.stopped() {
            return Err(stopped);
        }
        let count = used.load(Ordering::Relaxed);
        budget.charge(already, count, || {
            "interfaceSegments: native delivery work exceeds payload element budget".into()
        })?;
        let census = result?;
        self.connection.execute_batch("COMMIT;").map_err(sql)?;
        self.census = census;
        Ok(count)
    }
    fn finish_inner(
        &mut self,
        effort: &Effort,
        read: &mut dyn replay::Payloads,
    ) -> Result<Option<DecodedInterfaceCensus>, Error> {
        let declared: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM payloads WHERE identity_present=1)",
                [],
                |r| r.get(0),
            )
            .map_err(sql)?;
        self.checked |= declared;
        if !self.checked {
            return Ok(None);
        }
        // REPLACE continuity is an HLOD delivery property: other tilesets may keep
        // content-less intermediate tiles (the I1/C2 owners accept them).
        effort.ticks.pass()?;
        self.hierarchy(effort)?;
        if let Some(reason) = self.profile_exclusion {
            return Err(Error::unsupported(format!("interfaceSegments: {reason}")));
        }
        let stripped:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM refs r LEFT JOIN payloads p ON p.rank=r.rank WHERE p.identity_present IS NOT 1)",[],|r|r.get(0)).map_err(sql)?;
        if stripped {
            return Err(bad(
                "hlodFeatureIds",
                "archive expects source identity on every referenced payload",
            ));
        }
        effort.ticks.pass()?;
        let (coarse_triangle, coarse_rows) = self.coarse_identity(effort)?;
        if coarse_triangle {
            return Err(bad(
                "hlodFeatureIds",
                "D4 coarse reference carries forbidden source_triangle identity",
            ));
        }
        let missing_triangle:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM refs r JOIN tiles t ON t.id=r.tile JOIN payloads p ON p.rank=r.rank WHERE t.leaf=1 AND p.triangle_present=0)",[],|r|r.get(0)).map_err(sql)?;
        if missing_triangle {
            return Err(bad(
                "sourceIdentity",
                "descendant leaf has no original triangle rows",
            ));
        }
        if coarse_rows {
            return Err(bad(
                "hlodFeatureIds",
                "D4 coarse source_primitive table exceeds 8192 rows",
            ));
        }
        let mut census = DecodedInterfaceCensus::default();
        effort.ticks.pass()?;
        self.triangle_cap(&mut census)?;
        effort.ticks.pass()?;
        if let texture::Declaration::Rule(rule) = self.texture {
            (census.texture_refinable_tiles, census.texture_bound_tiles) =
                self.texture_error(rule, effort)?;
        }
        effort.ticks.pass()?;
        self.recovery(effort)?;
        effort.ticks.pass()?;
        census.omission = self.omission(effort)?;
        effort.ticks.pass()?;
        self.single_cover(effort)?;
        effort.ticks.pass()?;
        self.place_references(effort)?;
        effort.ticks.pass()?;
        self.replay_original_aliases(effort)?;
        effort.ticks.pass()?;
        self.collect_leaves(effort, read)?;
        effort.ticks.pass()?;
        self.load_twins()?;
        effort.ticks.pass()?;
        self.edge_faces(effort, read)?;
        let mut needs = self.sorter()?;
        effort.ticks.pass()?;
        self.point_requirements(&mut census, &mut needs, effort)?;
        effort.ticks.pass()?;
        let ends = self.segment_requirements(&mut census, &mut needs, effort)?;
        effort.ticks.pass()?;
        self.load_requirements(needs)?;
        effort.ticks.pass()?;
        self.count_shared_ends(&mut census, ends)?;
        effort.ticks.pass()?;
        self.resolve_last_resorts(effort)?;
        let mut images = self.sorter()?;
        effort.ticks.pass()?;
        self.check_coarse(&mut census, &mut images, effort, read)?;
        effort.ticks.pass()?;
        self.coarse_drops(&mut census.dropped_by_depth, effort)?;
        effort.ticks.pass()?;
        self.border_pairs(images, effort)?;
        Ok(Some(census))
    }
    fn hierarchy(&self, effort: &Effort) -> Result<(), Error> {
        struct Frame {
            id: i64,
            replace: bool,
            renderable: bool,
            barrier: bool,
            placeholder: bool,
        }
        let mut stack = Vec::<Frame>::new();
        let mut previous = -1i64;
        let mut query=self.connection.prepare("SELECT t.id,t.parent,t.refine,t.placeholder,EXISTS(SELECT 1 FROM refs r WHERE r.tile=t.id) FROM tiles t ORDER BY t.id").map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut end = self
            .connection
            .prepare_cached("UPDATE tiles SET last=? WHERE id=?")
            .map_err(sql)?;
        let mut cell = self
            .connection
            .prepare_cached("UPDATE tiles SET cell=? WHERE id=?")
            .map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let id: i64 = row.get(0).map_err(sql)?;
            let parent: Option<i64> = row.get(1).map_err(sql)?;
            while stack.last().map(|f| f.id) != parent {
                let frame = stack.pop().ok_or_else(|| {
                    Error::InvalidState("HLOD hierarchy is not in preorder".into())
                })?;
                end.execute(params![previous, frame.id]).map_err(sql)?;
            }
            let inherited = stack.last();
            let refine: Option<bool> = row.get(2).map_err(sql)?;
            let placeholder: bool = row.get(3).map_err(sql)?;
            let content: bool = row.get(4).map_err(sql)?;
            let replace = refine.unwrap_or_else(|| inherited.is_none_or(|p| p.replace));
            // The nearest renderable ancestor owns its refinement. An empty
            // ADD node cannot undo a REPLACE loading barrier already above it.
            let renderable = inherited.is_some_and(|p| p.renderable);
            let barrier = inherited.is_some_and(|p| p.barrier);
            if content && barrier {
                return Err(bad(
                    "replaceContentContinuity",
                    "contentless REPLACE tile separates renderable ancestor and descendant",
                ));
            }
            // The cell parent (D11 depth order) is the nearest non-placeholder
            // ancestor: links and implicit roots are not cells.
            if let Some(parent) = stack.iter().rev().find(|f| !f.placeholder) {
                cell.execute(params![parent.id, id]).map_err(sql)?;
            }
            stack.push(Frame {
                id,
                replace,
                renderable: if content { replace } else { renderable },
                barrier: !content && (barrier || (!placeholder && renderable)),
                placeholder,
            });
            previous = id;
        }
        for frame in stack.into_iter().rev() {
            end.execute(params![previous, frame.id]).map_err(sql)?;
        }
        // refs_tile supports the range existence probe; no transitive closure table.
        self.connection.execute_batch("UPDATE tiles SET leaf=NOT EXISTS(SELECT 1 FROM refs r WHERE r.tile>tiles.id AND r.tile<=tiles.last);").map_err(sql)?;
        Ok(())
    }
    /// D4 coarse references: whether some reference with `source_triangle` rows, and some
    /// with more than 8192 `source_primitive` rows, lies on a tile above a leaf reference to
    /// another payload. One preorder pass keeps each open tile's descendant leaf ranks as a
    /// (lowest, highest) pair, so every reference is read once: a rank differs from some
    /// descendant leaf rank unless both bounds equal it.
    fn coarse_identity(&self, effort: &Effort) -> Result<(bool, bool), Error> {
        type Range = Option<(i64, i64)>;
        fn join(a: Range, b: Range) -> Range {
            match (a, b) {
                (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
                (a, b) => a.or(b),
            }
        }
        /// The tile's first two distinct ranks of one kind: enough to find one that differs.
        fn note(ranks: &mut [Option<i64>; 2], rank: i64) {
            if ranks[0].is_none_or(|r| r == rank) {
                ranks[0] = Some(rank);
            } else if ranks[1].is_none() {
                ranks[1] = Some(rank);
            }
        }
        fn differs(below: Range, ranks: &[Option<i64>; 2]) -> bool {
            below.is_some_and(|(lo, hi)| ranks.iter().flatten().any(|&r| lo != r || hi != r))
        }
        struct Frame {
            id: i64,
            below: Range,
            leaf: Range,
            triangle: [Option<i64>; 2],
            rows: [Option<i64>; 2],
        }
        let mut found = (false, false);
        let mut close = |frame: Frame, stack: &mut Vec<Frame>| {
            found.0 |= differs(frame.below, &frame.triangle);
            found.1 |= differs(frame.below, &frame.rows);
            if let Some(parent) = stack.last_mut() {
                parent.below = join(parent.below, join(frame.below, frame.leaf));
            }
        };
        let mut query = self
            .connection
            .prepare("SELECT t.id,t.parent,t.leaf,r.rank,p.triangle_present,p.primitive_rows FROM tiles t LEFT JOIN refs r ON r.tile=t.id LEFT JOIN payloads p ON p.rank=r.rank ORDER BY t.id")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut stack = Vec::<Frame>::new();
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let id: i64 = row.get(0).map_err(sql)?;
            if stack.last().map(|f| f.id) != Some(id) {
                let parent: Option<i64> = row.get(1).map_err(sql)?;
                while stack.last().map(|f| f.id) != parent {
                    let frame = stack.pop().ok_or_else(|| {
                        Error::InvalidState("HLOD hierarchy is not in preorder".into())
                    })?;
                    close(frame, &mut stack);
                }
                stack.push(Frame {
                    id,
                    below: None,
                    leaf: None,
                    triangle: [None; 2],
                    rows: [None; 2],
                });
            }
            let Some(rank) = row.get::<_, Option<i64>>(3).map_err(sql)? else {
                continue;
            };
            let frame = stack.last_mut().expect("frame pushed above");
            if row.get::<_, Option<bool>>(2).map_err(sql)? == Some(true) {
                frame.leaf = join(frame.leaf, Some((rank, rank)));
            }
            if row.get::<_, Option<bool>>(4).map_err(sql)? == Some(true) {
                note(&mut frame.triangle, rank);
            }
            if row
                .get::<_, Option<i64>>(5)
                .map_err(sql)?
                .is_some_and(|n| n > 8192)
            {
                note(&mut frame.rows, rank);
            }
        }
        while let Some(frame) = stack.pop() {
            close(frame, &mut stack);
        }
        Ok(found)
    }
    /// Append one original vertex row to the ingest sequence.
    fn original(&mut self, row: Original) -> Result<(), Error> {
        if self.originals.is_none() {
            self.originals = Some(
                SequenceWriter::new(&self._scratch, "hlod-originals", self.buffer_bytes)
                    .map_err(spill_failure)?,
            );
        }
        let writer = self.originals.as_mut().expect("writer created above");
        writer.push(&row).map_err(spill_failure)
    }
    /// Leaf placement frames of `rank` after placement id `after`, at most `ALIAS_FRAMES`.
    fn leaf_frames(&self, rank: u64, after: i64) -> Result<Vec<(i64, Matrix)>, Error> {
        let mut query = self
            .connection
            .prepare_cached("SELECT id,frame FROM placements WHERE rank=? AND leaf=1 AND id>? ORDER BY id LIMIT ?")
            .map_err(sql)?;
        let mut rows = query
            .query(params![integer(rank)?, after, ALIAS_FRAMES as i64])
            .map_err(sql)?;
        let mut frames = Vec::with_capacity(ALIAS_FRAMES);
        while let Some(row) = rows.next().map_err(sql)? {
            frames.push((
                row.get(0).map_err(sql)?,
                decode::<16>(&row.get::<_, Vec<u8>>(1).map_err(sql)?)?,
            ));
        }
        Ok(frames)
    }
    /// Every original vertex shared by leaf placements, including repeated
    /// references to one payload and disjoint geometry, must have one exact placed
    /// f64 position (normalized zero). The rows recorded at ingest are read once per
    /// payload block (and once more per further chunk of `ALIAS_FRAMES` leaf frames),
    /// each placed key goes to one bounded external sort by (source, vertex, exact),
    /// and one ordered scan compares neighbours: each payload is decoded once, never
    /// per pair, and resident memory is the sort configuration plus one frame chunk.
    fn replay_original_aliases(&mut self, effort: &Effort) -> Result<(), Error> {
        // The pass owns the sequence: its spill file is removed when the pass ends.
        let Some(writer) = self.originals.take() else {
            return Ok(());
        };
        let recorded = writer.finish().map_err(spill_failure)?;
        let mut sorter =
            ExternalSorter::<Alias>::new(&self._scratch, self.sort).map_err(spill_failure)?;
        let mut emit = |row: &Original, frames: &[(i64, Matrix)]| -> Result<(), Error> {
            let local = row.point.map(f64::from_bits);
            for (_, frame) in frames {
                effort.charge(ALIAS_UNITS)?;
                let placed = point(*frame, local);
                // Checked while keys are emitted, before the sorted scan: an archive with both
                // a non-finite placement and an alias conflict reports the non-finite one.
                finite(&placed)?;
                sorter
                    .push(Alias {
                        source: row.source,
                        vertex: row.vertex,
                        exact: point_key(placed),
                    })
                    .map_err(spill_failure)?;
            }
            Ok(())
        };
        let mut rows = recorded.reader().map_err(spill_failure)?;
        let mut next = rows.next().transpose().map_err(spill_failure)?;
        let mut index = 0u64;
        while let Some(first) = next {
            // One payload's rows are contiguous: its first frame chunk is applied while
            // the block is read, further chunks by re-reading the block's range.
            effort.charge(ROW_UNITS)?;
            let start = index;
            let mut frames = self.leaf_frames(first.rank, -1)?;
            next = Some(first);
            while let Some(row) = next.filter(|row| row.rank == first.rank) {
                emit(&row, &frames)?;
                index += 1;
                next = rows.next().transpose().map_err(spill_failure)?;
            }
            while frames.len() == ALIAS_FRAMES {
                effort.charge(ROW_UNITS)?;
                frames = self.leaf_frames(first.rank, frames[ALIAS_FRAMES - 1].0)?;
                if frames.is_empty() {
                    break;
                }
                for row in recorded
                    .range(start, index, self.buffer_bytes)
                    .map_err(spill_failure)?
                {
                    emit(&row.map_err(spill_failure)?, &frames)?;
                }
            }
        }
        let mut previous: Option<Alias> = None;
        for alias in sorter.finish().map_err(spill_failure)? {
            let alias = alias.map_err(spill_failure)?;
            if let Some(p) = previous {
                if (p.source, p.vertex) == (alias.source, alias.vertex) && p.exact != alias.exact {
                    return Err(bad(
                        "sourceIdentity",
                        "original vertex has distinct exact decoded placed positions",
                    ));
                }
            }
            previous = Some(alias);
        }
        Ok(())
    }

    pub(super) fn census(&self) -> Option<DecodedInterfaceCensus> {
        self.census.clone()
    }
    /// D1's hard cap applies to every referenced payload, counted per payload
    /// over all its scene instances, never per reference.
    fn triangle_cap(&self, census: &mut DecodedInterfaceCensus) -> Result<(), Error> {
        let (rank, most): (Option<i64>, i64) = self
            .connection
            .query_row(
                "SELECT p.rank,max(p.triangles) FROM payloads p WHERE EXISTS(SELECT 1 FROM refs r WHERE r.rank=p.rank)",
                [],
                |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
            )
            .map_err(sql)?;
        let most = u64::try_from(most).map_err(|_| super::refused())?;
        if most > TRIANGLE_CAP {
            return Err(bad(
                "triangleCap",
                &format!(
                    "payload rank {} publishes {most} triangles, above the hard {TRIANGLE_CAP}",
                    rank.unwrap_or(-1)
                ),
            ));
        }
        census.max_payload_triangles = most;
        Ok(())
    }
    /// Upward omission and the first renderable cover, from the delivered
    /// hierarchy alone. A content-less cell never has a content ancestor, never
    /// ends a branch, and ADD content never stays drawn over its refinement.
    fn omission(&self, effort: &Effort) -> Result<Omission, Error> {
        struct Frame {
            id: i64,
            placeholder: bool,
            empty: bool,
            depth: usize,
            replace: bool,
            content_above: bool,
            empty_child: bool,
        }
        let mut omission = Omission::default();
        let mut stack = Vec::<Frame>::new();
        let mut query=self.connection.prepare("SELECT t.id,t.parent,t.refine,t.placeholder,t.leaf,EXISTS(SELECT 1 FROM refs r WHERE r.tile=t.id),EXISTS(SELECT 1 FROM tiles c WHERE c.parent=t.id),t.error,t.diagonal FROM tiles t ORDER BY t.id").map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let close = |frame: Frame, omission: &mut Omission| {
            if frame.empty && !frame.empty_child {
                omission.omission_sources += 1;
            }
        };
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let id: i64 = row.get(0).map_err(sql)?;
            let parent: Option<i64> = row.get(1).map_err(sql)?;
            while stack.last().map(|f| f.id) != parent {
                let frame = stack.pop().ok_or_else(|| {
                    Error::InvalidState("HLOD hierarchy is not in preorder".into())
                })?;
                close(frame, &mut omission);
            }
            let refine: Option<bool> = row.get(2).map_err(sql)?;
            let placeholder: bool = row.get(3).map_err(sql)?;
            let leaf: bool = row.get(4).map_err(sql)?;
            let content: bool = row.get(5).map_err(sql)?;
            let children: bool = row.get(6).map_err(sql)?;
            let error: Option<f64> = row.get(7).map_err(sql)?;
            let diagonal: Option<f64> = row.get(8).map_err(sql)?;
            let inherited = stack.last();
            let replace = refine.unwrap_or_else(|| inherited.is_none_or(|p| p.replace));
            let content_above = inherited.is_some_and(|p| p.content_above);
            let depth = inherited.map_or(0, |p| p.depth + usize::from(!p.placeholder));
            let empty = !placeholder && !content;
            if empty && content_above {
                return Err(bad(
                    "omissionPropagation",
                    &format!("content-less cell {id} has a content ancestor: omission must reach the root"),
                ));
            }
            if !children && !content {
                return Err(bad(
                    "frontierCoverage",
                    &format!("terminal tile {id} publishes no content: the cover is incomplete"),
                ));
            }
            if empty {
                // Cesium's base traversal descends an empty tile only while its SSE
                // exceeds the maximum (default 16): with G >= 16 x box diagonal that
                // holds until the whole box projects below about one pixel. Children's
                // G alone (a leaf's sub-millimetre error) stops it at once, drawing
                // nothing, since an omitted cell has no content ancestor to hold.
                // Links and implicit roots always traverse and are exempt.
                let floor = diagonal.map(|d| EMPTY_PARENT_SCALE * d);
                match (error, floor) {
                    (Some(g), Some(floor)) if g >= floor * (1. - 1e-12) => {}
                    _ => {
                        return Err(bad(
                            "emptyCellRefinement",
                            &format!(
                                "content-less cell {id} has geometricError {error:?} below \
                                 {EMPTY_PARENT_SCALE} x its box diagonal {diagonal:?}: \
                                 a viewer never refines it"
                            ),
                        ))
                    }
                }
                omission.omitted_cells += 1;
                if omission.omitted_by_depth.len() <= depth {
                    omission.omitted_by_depth.resize(depth + 1, 0);
                }
                omission.omitted_by_depth[depth] += 1;
            }
            if content {
                if !content_above {
                    omission.frontier_tiles += 1;
                }
                if leaf {
                    omission.frontier_leaves += 1;
                } else if !replace {
                    return Err(bad(
                        "frontierCoverage",
                        &format!(
                            "ADD content of tile {id} stays drawn over its renderable descendants"
                        ),
                    ));
                }
            }
            if !placeholder {
                // The cell parent is the nearest non-placeholder ancestor.
                if let Some(cell) = stack.iter_mut().rev().find(|f| !f.placeholder) {
                    cell.empty_child |= empty;
                }
            }
            stack.push(Frame {
                id,
                placeholder,
                empty,
                depth,
                replace,
                content_above: content_above || content,
                empty_child: false,
            });
        }
        while let Some(frame) = stack.pop() {
            close(frame, &mut omission);
        }
        Ok(omission)
    }
    /// The first renderable cover renders every leaf once: the same payload
    /// bytes may repeat only down a single-child chain (a cell standing for its
    /// only child), never twice in one tile and never across branches.
    fn single_cover(&self, effort: &Effort) -> Result<(), Error> {
        let twice: Option<i64> = self
            .connection
            .query_row(
                "SELECT r.tile FROM refs r JOIN payloads p ON p.rank=r.rank GROUP BY r.tile,p.digest HAVING count(*)>1 LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?;
        if let Some(tile) = twice {
            return Err(bad(
                "frontierCoverage",
                &format!("tile {tile} references the same payload bytes twice"),
            ));
        }
        let mut query = self
            .connection
            .prepare("SELECT p.digest,r.tile,t.last FROM refs r JOIN payloads p ON p.rank=r.rank JOIN tiles t ON t.id=r.tile ORDER BY p.digest,r.tile")
            .map_err(sql)?;
        let mut path = self
            .connection
            .prepare_cached("SELECT count(*) FROM tiles WHERE id>? AND id<=? AND parent=id-1")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut previous: Option<(Vec<u8>, i64, i64)> = None;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let digest: Vec<u8> = row.get(0).map_err(sql)?;
            let tile: i64 = row.get(1).map_err(sql)?;
            let last: i64 = row.get(2).map_err(sql)?;
            if let Some((d, above, above_last)) = &previous {
                if *d == digest {
                    let span = tile - above;
                    effort.charge(usize::try_from(span).map_err(|_| super::refused())?)?;
                    let chained: i64 = path
                        .query_row(params![above, tile], |r| r.get(0))
                        .map_err(sql)?;
                    if *above_last != last || chained != span {
                        return Err(bad(
                            "frontierCoverage",
                            &format!("payload bytes of tile {above} repeat at tile {tile} outside a single-child chain"),
                        ));
                    }
                }
            }
            previous = Some((digest, tile, last));
        }
        Ok(())
    }
    fn place_references(&self, effort: &Effort) -> Result<(), Error> {
        let mut query = self
            .connection
            .prepare("SELECT r.tile,r.rank,t.world,t.leaf FROM refs r JOIN tiles t ON t.id=r.tile")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(ROW_UNITS)?;
            let world: Vec<u8> = row.get(2).map_err(sql)?;
            self.connection
                .execute(
                    "INSERT INTO placements(tile,rank,frame,leaf) VALUES(?,?,?,?)",
                    params![
                        row.get::<_, i64>(0).map_err(sql)?,
                        row.get::<_, i64>(1).map_err(sql)?,
                        encode(&delivery_frame(decode::<16>(&world)?)),
                        row.get::<_, bool>(3).map_err(sql)?
                    ],
                )
                .map_err(sql)?;
        }
        self.connection
            .execute_batch("CREATE INDEX IF NOT EXISTS placement_rank ON placements(rank,leaf,id)")
            .map_err(sql)
    }
    /// The lookahead copy of a replay query, prepared uncharged; none without workers.
    fn lookahead_query(
        &self,
        query: &str,
        effort: &Effort,
    ) -> Result<Option<rusqlite::Statement<'_>>, Error> {
        if !self.pool.parallel() {
            return Ok(None);
        }
        effort
            .ticks
            .uncharged(|| self.connection.prepare(query))
            .map(Some)
            .map_err(sql)
    }
    fn lookahead_rows<'s>(
        &self,
        query: Option<&'s mut rusqlite::Statement<'_>>,
        effort: &Effort,
    ) -> Result<Option<rusqlite::Rows<'s>>, Error> {
        query
            .map(|q| effort.ticks.uncharged(|| q.query([])))
            .transpose()
            .map_err(sql)
    }
    /// One leaf placement's points and segments, bound for the bulk load (`bulk`).
    fn record_leaf(
        &self,
        tile: i64,
        surface: &Surface,
        rows: &mut Sorter<'_, 61>,
        effort: &Effort,
    ) -> Result<(), Error> {
        for p in &surface.points {
            effort.charge(ROW_UNITS)?;
            rows.push(bulk::point_row(&point_bytes(*p), tile))
                .map_err(spill_failure)?;
        }
        for (key, e) in &surface.edges {
            effort.charge(ROW_UNITS)?;
            rows.push(bulk::segment_row(
                &edge_bytes(*key),
                tile,
                integer(e.forward)?,
                integer(e.reverse)?,
            ))
            .map_err(spill_failure)?;
        }
        Ok(())
    }
    fn collect_leaves(
        &self,
        effort: &Effort,
        read: &mut dyn replay::Payloads,
    ) -> Result<(), Error> {
        let mut query = self.connection.prepare(LEAVES).map_err(sql)?;
        let total = effort.ticks.total(|| {
            self.connection
                .query_row("SELECT count(*) FROM placements WHERE leaf=1", [], |r| {
                    r.get::<_, i64>(0)
                })
                .map(|n| n as u64)
                .map_err(sql)
        })?;
        effort.ticks.replay("validate.delivery.leaves", total)?;
        let mut ahead_query = self.lookahead_query(LEAVES, effort)?;
        let mut ahead_rows = self.lookahead_rows(ahead_query.as_mut(), effort)?;
        let mut ahead =
            replay::Lookahead::new(self, &self.pool, ahead_rows.as_mut(), (1, 2), effort, false);
        let mut rows = query.query([]).map_err(sql)?;
        let mut leaves = self.sorter()?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.ticks.step()?;
            effort.charge(1)?;
            let surface = ahead.surface(
                row.get(1).map_err(sql)?,
                &row.get::<_, Vec<u8>>(2).map_err(sql)?,
                effort,
                read,
            )?;
            let tile: i64 = row.get(0).map_err(sql)?;
            self.record_leaf(tile, &surface, &mut leaves, effort)?;
            // Every leaf face by corner set, so coincident source groups are found
            // across leaves too (they license coincident coarse faces above).
            for f in &surface.faces {
                effort.charge(ROW_UNITS)?;
                let mut k = *f;
                k.sort_unstable();
                let bytes: Vec<u8> = k.iter().flat_map(|p| point_bytes(*p)).collect();
                leaves
                    .push(bulk::face_row(&bytes, tile))
                    .map_err(spill_failure)?;
            }
        }
        drop(rows);
        // The lookahead's statement is released before the load writes.
        drop(ahead);
        drop(ahead_rows);
        drop(ahead_query);
        self.load_leaves(leaves)
    }
    /// Leaf faces on every edge some direction of which carries two or more
    /// descendant faces: their third corners and directions, so coarse multiplicity
    /// is matched to source faces on its own side rather than counted. A second,
    /// bounded replay of only the leaves holding such edges.
    fn edge_faces(&self, effort: &Effort, read: &mut dyn replay::Payloads) -> Result<(), Error> {
        self.connection
            .execute_batch(
                "INSERT INTO multi_edges SELECT key FROM leaf_segments GROUP BY key HAVING sum(nf)>=2 OR sum(nr)>=2",
            )
            .map_err(sql)?;
        // The leaves to replay, found once through the (key, tile) index from the
        // multiplicity keys; placements are then read in one ordered pass.
        self.connection
            .execute_batch(
                "CREATE TEMP TABLE replay_tiles(tile INTEGER PRIMARY KEY);
                 INSERT OR IGNORE INTO replay_tiles SELECT s.tile FROM multi_edges m JOIN leaf_segments s ON s.key=m.key;",
            )
            .map_err(sql)?;
        let mut query = self.connection.prepare(EDGE_LEAVES).map_err(sql)?;
        let total = effort.ticks.total(|| {
            self.connection
                .query_row(
                    "SELECT count(*) FROM placements p WHERE p.leaf=1 AND p.tile IN (SELECT tile FROM replay_tiles)",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n as u64)
                .map_err(sql)
        })?;
        effort.ticks.replay("validate.delivery.edges", total)?;
        let mut ahead_query = self.lookahead_query(EDGE_LEAVES, effort)?;
        let mut ahead_rows = self.lookahead_rows(ahead_query.as_mut(), effort)?;
        let mut ahead =
            replay::Lookahead::new(self, &self.pool, ahead_rows.as_mut(), (1, 2), effort, false);
        let mut rows = query.query([]).map_err(sql)?;
        // Every face side of the replayed leaves, kept where its segment is a multi edge by
        // the merge join of the bulk load.
        let mut faces = self.sorter()?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.ticks.step()?;
            effort.charge(1)?;
            let tile: i64 = row.get(0).map_err(sql)?;
            let surface = ahead.surface(
                row.get(1).map_err(sql)?,
                &row.get::<_, Vec<u8>>(2).map_err(sql)?,
                effort,
                read,
            )?;
            for f in &surface.faces {
                for j in 0..3 {
                    effort.charge(ROW_UNITS)?;
                    let (x, y, c) = (f[j], f[(j + 1) % 3], f[(j + 2) % 3]);
                    let key = edge_bytes(if x < y { (x, y) } else { (y, x) });
                    faces
                        .push(bulk::edge_face(&key, tile, &point_bytes(c), x > y))
                        .map_err(spill_failure)?;
                }
            }
        }
        drop(rows);
        // The lookahead's statement is released before the load writes.
        drop(ahead);
        drop(ahead_rows);
        drop(ahead_query);
        self.load_edge_faces(faces)
    }
    #[allow(clippy::too_many_arguments)]
    fn require_ancestors(
        &self,
        mut tile: i64,
        owners: (i64, i64),
        kind: i64,
        key: &[u8],
        directions: i64,
        needs: &mut Sorter<'_, 41>,
        effort: &Effort,
    ) -> Result<(), Error> {
        let owner = tile;
        let (first, last) = owners;
        let mut lookup=self.connection.prepare_cached("SELECT parent,last,EXISTS(SELECT 1 FROM refs r WHERE r.tile=tiles.id) FROM tiles WHERE id=?").map_err(sql)?;
        loop {
            effort.charge(ROW_UNITS)?;
            let (parent, end, content): (Option<i64>, i64, bool) = lookup
                .query_row([tile], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(sql)?;
            if first >= tile && last <= end {
                break;
            }
            if tile != owner && content {
                needs
                    .push(bulk::need(tile, kind, key, directions))
                    .map_err(spill_failure)?;
            }
            let Some(parent) = parent else {
                break;
            };
            tile = parent;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn shared_point(
        &self,
        key: &[u8],
        first: i64,
        last: i64,
        census: &mut DecodedInterfaceCensus,
        needs: &mut Sorter<'_, 41>,
        effort: &Effort,
    ) -> Result<(), Error> {
        if first == last {
            return Ok(());
        }
        effort.charge(ROW_UNITS)?;
        add_count(&mut census.shared_points, 1)?;
        self.connection
            .execute("INSERT INTO shared_point_keys VALUES(?)", [key])
            .map_err(sql)?;
        let mut query = self
            .connection
            .prepare_cached("SELECT tile FROM leaf_points WHERE key=? ORDER BY tile")
            .map_err(sql)?;
        let mut rows = query.query([key]).map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            self.require_ancestors(
                row.get(0).map_err(sql)?,
                (first, last),
                0,
                key,
                0,
                needs,
                effort,
            )?;
        }
        Ok(())
    }
    fn point_requirements(
        &self,
        census: &mut DecodedInterfaceCensus,
        needs: &mut Sorter<'_, 41>,
        effort: &Effort,
    ) -> Result<(), Error> {
        let mut query = self
            .connection
            .prepare("SELECT key,tile FROM leaf_points ORDER BY key,tile")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut group: Option<(Vec<u8>, i64, i64)> = None;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let key: Vec<u8> = row.get(0).map_err(sql)?;
            let tile: i64 = row.get(1).map_err(sql)?;
            if let Some((k, _, last)) = &mut group {
                if *k == key {
                    *last = tile;
                    continue;
                }
            }
            if let Some((k, first, last)) = group.take() {
                self.shared_point(&k, first, last, census, needs, effort)?;
            }
            group = Some((key, tile, tile));
        }
        if let Some((key, first, last)) = group {
            self.shared_point(&key, first, last, census, needs, effort)?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn segment_group(
        &self,
        key: &[u8],
        (first, last): (i64, i64),
        faces: u64,
        census: &mut DecodedInterfaceCensus,
        open: &mut OpenEnds<'_>,
        needs: &mut Sorter<'_, 41>,
        effort: &Effort,
    ) -> Result<(), Error> {
        effort.charge(1)?;
        if faces == 1 {
            add_count(&mut census.source_open_edges, 1)?;
            // Both endpoints must also occur in another leaf. This is an exact
            // key join, not a chord inference or a distance/near-seam query:
            // lower endpoints merge in key order, higher ones after a sort.
            read_edge(key)?;
            effort.charge(OPEN_UNITS)?;
            if open.lower.shared(&key[..12])? {
                open.higher
                    .push(&bulk::end(&key[12..]))
                    .map_err(spill_failure)?;
            }
        }
        if first == last {
            return Ok(());
        }
        add_count(&mut census.shared_segments, 1)?;
        let mut query = self
            .connection
            .prepare_cached("SELECT tile,nf,nr FROM leaf_segments WHERE key=? ORDER BY tile")
            .map_err(sql)?;
        let mut rows = query.query([key]).map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let directions = i64::from(row.get::<_, i64>(1).map_err(sql)? > 0)
                | (i64::from(row.get::<_, i64>(2).map_err(sql)? > 0) << 1);
            self.require_ancestors(
                row.get(0).map_err(sql)?,
                (first, last),
                1,
                key,
                directions,
                needs,
                effort,
            )?;
        }
        Ok(())
    }
    /// Shared segments' ancestor requirements and the open-segment census. Returns the
    /// higher endpoints of open segments whose lower endpoint is shared (`count_shared_ends`).
    fn segment_requirements(
        &self,
        census: &mut DecodedInterfaceCensus,
        needs: &mut Sorter<'_, 41>,
        effort: &Effort,
    ) -> Result<Sequence<bulk::End>, Error> {
        let mut query = self
            .connection
            .prepare("SELECT key,tile,nf,nr FROM leaf_segments ORDER BY key,tile")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut shared = self
            .connection
            .prepare("SELECT key FROM shared_point_keys ORDER BY key")
            .map_err(sql)?;
        let mut open = OpenEnds {
            lower: bulk::LowerEnds::new(&mut shared)?,
            higher: SequenceWriter::new(&self._scratch, "hlod-open-ends", self.buffer_bytes)
                .map_err(spill_failure)?,
        };
        let mut group: Option<(Vec<u8>, i64, i64, u64)> = None;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let key: Vec<u8> = row.get(0).map_err(sql)?;
            let tile: i64 = row.get(1).map_err(sql)?;
            let mut faces =
                u64::try_from(row.get::<_, i64>(2).map_err(sql)?).map_err(|_| super::refused())?;
            add_count(
                &mut faces,
                u64::try_from(row.get::<_, i64>(3).map_err(sql)?).map_err(|_| super::refused())?,
            )?;
            if let Some((k, _, last, n)) = &mut group {
                if *k == key {
                    *last = tile;
                    add_count(n, faces)?;
                    continue;
                }
            }
            if let Some((k, first, last, n)) = group.take() {
                self.segment_group(&k, (first, last), n, census, &mut open, needs, effort)?;
            }
            group = Some((key, tile, tile, faces));
        }
        if let Some((key, first, last, n)) = group {
            self.segment_group(&key, (first, last), n, census, &mut open, needs, effort)?;
        }
        open.higher.finish().map_err(spill_failure)
    }
    /// Counts the open segments whose higher endpoint is shared too, by a sort of those
    /// endpoints merged with `shared_point_keys`.
    fn count_shared_ends(
        &self,
        census: &mut DecodedInterfaceCensus,
        ends: Sequence<bulk::End>,
    ) -> Result<(), Error> {
        let mut sorted = self.sorter()?;
        for e in ends.reader().map_err(spill_failure)? {
            sorted
                .push(e.map_err(spill_failure)?)
                .map_err(spill_failure)?;
        }
        drop(ends);
        let shared = self.shared_ends(sorted)?;
        add_count(&mut census.source_open_edges_with_shared_endpoints, shared)
    }
    /// Resolve declarations once, before any fold or drop exemption. Undeclared
    /// payloads do not contradict a declaration (the reference uses the same rule).
    fn resolve_last_resorts(&self, effort: &Effort) -> Result<(), Error> {
        let mut query = self
            .connection
            .prepare(
                "SELECT r.tile,t.leaf,min(p.last_stage),max(p.last_stage),
                    min(p.fold_faces),max(p.fold_faces),count(*)
             FROM refs r JOIN payloads p ON p.rank=r.rank JOIN tiles t ON t.id=r.tile
             WHERE p.last_stage>0 GROUP BY r.tile ORDER BY r.tile",
            )
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(
                usize::try_from(row.get::<_, i64>(6).map_err(sql)?)
                    .map_err(|_| super::refused())?
                    .saturating_mul(ROW_UNITS),
            )?;
            let tile: i64 = row.get(0).map_err(sql)?;
            if row.get::<_, bool>(1).map_err(sql)? {
                return Err(bad(
                    "lastResort",
                    &format!("leaf tile {tile} declares a last resort"),
                ));
            }
            let stage: i64 = row.get(2).map_err(sql)?;
            let folds: i64 = row.get(4).map_err(sql)?;
            if stage != row.get::<_, i64>(3).map_err(sql)?
                || folds != row.get::<_, i64>(5).map_err(sql)?
            {
                return Err(bad(
                    "lastResort",
                    &format!("tile {tile}: payloads disagree"),
                ));
            }
        }
        // Every declaration is now unique per tile, so its consumers (the coarse check and the
        // recount) read any one payload's stage by a seek on the tile's references.
        Ok(())
    }

    /// Records one coarse tile's NORMAL census; an inverted tile's cell depth is found by
    /// one seek per ancestor (links and implicit roots are not cells and add no depth).
    fn coarse_normals(
        &self,
        tile: i64,
        surface: &Surface,
        census: &mut CoarseNormals,
        effort: &Effort,
    ) -> Result<(), Error> {
        if surface.normal_skips != 0 {
            add_count(&mut census.skipped_tiles, 1)?;
            census.skip_reasons |= surface.normal_skips;
        }
        if surface.normal_faces == 0 {
            return Ok(());
        }
        add_count(&mut census.tiles, 1)?;
        add_count(&mut census.faces, surface.normal_faces)?;
        if surface.inverted_faces == 0 {
            return Ok(());
        }
        // Cell depth as `omission` counts it: ancestors that are not placeholders (the
        // `cell` column is rewritten by descendant recovery, so the parent chain is read).
        let mut depth = 0;
        let mut at = tile;
        let mut parent = self
            .connection
            .prepare_cached(
                "SELECT t.parent,p.placeholder FROM tiles t LEFT JOIN tiles p ON p.id=t.parent WHERE t.id=?1",
            )
            .map_err(sql)?;
        loop {
            effort.charge(SEEK_UNITS)?;
            let (up, placeholder): (Option<i64>, Option<bool>) = parent
                .query_row([at], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(sql)?;
            let Some(up) = up else { break };
            depth += usize::from(placeholder == Some(false));
            at = up;
        }
        for v in [
            &mut census.inverted_tiles_by_depth,
            &mut census.inverted_faces_by_depth,
        ] {
            if v.len() <= depth {
                v.resize(depth + 1, 0);
            }
        }
        add_count(&mut census.inverted_tiles_by_depth[depth], 1)?;
        add_count(
            &mut census.inverted_faces_by_depth[depth],
            surface.inverted_faces,
        )
    }
    /// Every coarse tile's decoded surface (all its placements) against its D11
    /// border: exact junctions, one witnessed chord per shared arc, displacement.
    fn check_coarse(
        &self,
        census: &mut DecodedInterfaceCensus,
        images: &mut Sorter<'_, 69>,
        effort: &Effort,
        read: &mut dyn replay::Payloads,
    ) -> Result<(), Error> {
        // The tiles referencing a payload declared at a fold stage (2 or 3), once: each
        // declared coarse tile then finds one below it by a single seek, not by a scan of
        // its descendants' references at every ancestor.
        self.connection
            .execute_batch(
                "INSERT OR IGNORE INTO fold_stages SELECT r.tile FROM payloads p
                 JOIN refs r ON r.rank=p.rank WHERE p.last_stage>=3",
            )
            .map_err(sql)?;
        let mut query = self.connection.prepare(COARSE).map_err(sql)?;
        let total = effort.ticks.total(|| {
            self.connection
                .query_row("SELECT count(*) FROM placements WHERE leaf=0", [], |r| {
                    r.get::<_, i64>(0)
                })
                .map(|n| n as u64)
                .map_err(sql)
        })?;
        effort.ticks.replay("validate.delivery.coarse", total)?;
        let mut ahead_query = self.lookahead_query(COARSE, effort)?;
        let mut ahead_rows = self.lookahead_rows(ahead_query.as_mut(), effort)?;
        let mut ahead =
            replay::Lookahead::new(self, &self.pool, ahead_rows.as_mut(), (1, 2), effort, true);
        let mut rows = query.query([]).map_err(sql)?;
        let mut current: Option<(i64, Surface)> = None;
        loop {
            let row = rows.next().map_err(sql)?;
            let tile: Option<i64> = row.map(|r| r.get(0)).transpose().map_err(sql)?;
            if current.as_ref().map(|c| c.0) != tile {
                if let Some((tile, surface)) = current.take() {
                    // The merged coarse faces, for the coarse-drop recount (drops.rs).
                    let mut face = self
                        .connection
                        .prepare_cached("INSERT INTO coarse_faces VALUES(?,?)")
                        .map_err(sql)?;
                    for f in &surface.faces {
                        effort.charge(ROW_UNITS)?;
                        let bytes: Vec<u8> = f.iter().flat_map(|p| point_bytes(*p)).collect();
                        face.execute(params![tile, bytes]).map_err(sql)?;
                    }
                    let (contracted, shared) = self.border(tile, &surface, images, effort)?;
                    // A2: a declared tile holds exactly its declared fold faces. Folds are
                    // admitted from stage 2, and in a declared tile above such a stage
                    // (it keeps them as input); otherwise every fold needs source.
                    // `last_stage` stores stage + 1 (0: undeclared).
                    let (stored, declared): (i64, i64) = self
                        .connection
                        .query_row(
                            "SELECT coalesce(max(p.last_stage),0),coalesce(max(p.fold_faces),0)
                             FROM refs r JOIN payloads p ON p.rank=r.rank WHERE r.tile=?1",
                            [tile],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .map_err(sql)?;
                    let stage = stored - 1;
                    // A declared fold stage below: one seek into `fold_stages`.
                    let below = stored > 0 && {
                        effort.charge(SEEK_UNITS)?;
                        self.connection
                            .prepare_cached(FOLD_STAGE_BELOW)
                            .map_err(sql)?
                            .query_row([tile], |r| r.get(0))
                            .map_err(sql)?
                    };
                    if stored > 0 && i64::try_from(fold_faces(&surface)).ok() != Some(declared) {
                        return Err(bad(
                            "lastResort",
                            &format!(
                                "declared {declared} fold faces, decoded {}: tile={tile}",
                                fold_faces(&surface)
                            ),
                        ));
                    }
                    if stage == 0 && !below {
                        return Err(bad(
                            "lastResort",
                            &format!("stage 0 declares folds without a declared fold stage below: tile={tile}"),
                        ));
                    }
                    if !(stage >= 2 || (stored > 0 && below)) {
                        self.equal_directions(tile, &surface, &contracted, effort)?;
                    }
                    self.seam_owner(tile, &surface, &contracted, &shared, effort)?;
                    self.coarse_normals(tile, &surface, &mut census.coarse_normals, effort)?;
                    for path in contracted.values() {
                        add_count(&mut census.contracted_border_arcs, 1)?;
                        add_count(
                            &mut census.contracted_border_vertices,
                            path.len() as u64 - 2,
                        )?;
                    }
                }
            }
            let (Some(row), Some(tile)) = (row, tile) else {
                break;
            };
            effort.ticks.step()?;
            effort.charge(1)?;
            let surface = ahead.surface(
                row.get(1).map_err(sql)?,
                &row.get::<_, Vec<u8>>(2).map_err(sql)?,
                effort,
                read,
            )?;
            match &mut current {
                Some((_, merged)) => merged.merge(surface, self.decoded_limit)?,
                None => current = Some((tile, surface)),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn budget() -> Budget {
        Budget {
            key: super::super::LimitKey::TotalPayloadElements,
            value: 10_000_000,
            source: super::super::limits::Derivation::Explicit,
        }
    }
    pub(super) fn effort_for_tests() -> Effort {
        effort()
    }
    fn effort() -> Effort {
        Effort {
            certificate_fast: AtomicU64::new(0),
            certificate_slow: AtomicU64::new(0),
            budget: budget(),
            base: 0,
            used: Arc::new(AtomicU64::new(0)),
            ticks: Default::default(),
        }
    }
    /// A store with this build's catalog profile: the forced-spill CI run sorts every bulk
    /// load in runs of a record or two.
    pub(super) fn store() -> Store {
        store_with(&CatalogConfig::selected())
    }
    pub(super) fn store_with(config: &CatalogConfig) -> Store {
        let scratch = Scratch::create(None, 4096).unwrap();
        Store::new(Arc::new(scratch), config, Resources::FIXED).unwrap()
    }
    #[test]
    fn continuity_is_hlod_only_and_respects_link_placeholders() {
        for (hlod, placeholder) in [(false, false), (true, false), (true, true)] {
            let mut s = store();
            s.tile(0, None, IDENTITY).unwrap();
            s.tile(1, Some(0), IDENTITY).unwrap();
            s.tile(2, Some(1), IDENTITY).unwrap();
            s.kind(1, None, placeholder, 0., None).unwrap();
            s.reference(0, 0).unwrap();
            s.reference(2, 2).unwrap();
            s.payload(0, false, false).unwrap();
            s.payload(2, false, false).unwrap();
            if hlod {
                s.expect_identity();
            }
            let result = s.finish(budget(), 0, None, &mut |_| panic!("unexpected replay"));
            match (hlod, placeholder) {
                // Other tilesets may keep empty intermediates; no disk store exists.
                (false, _) => {
                    result.unwrap();
                    assert!(s._file.is_none() && s.census().is_none());
                }
                // The link passes continuity; the stripped payloads then refuse.
                (true, true) => assert!(result.unwrap_err().to_string().contains("hlodFeatureIds")),
                (true, false) => assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("replaceContentContinuity")),
            }
        }
    }
    #[test]
    fn partial_strip_is_order_independent_and_report_expectation_is_sufficient() {
        for declared_first in [false, true] {
            let mut s = store();
            s.tile(0, None, IDENTITY).unwrap();
            s.tile(1, Some(0), IDENTITY).unwrap();
            s.reference(0, 0).unwrap();
            s.reference(1, 1).unwrap();
            if declared_first {
                s.payload(1, true, true).unwrap();
                s.payload(0, false, false).unwrap();
            } else {
                s.payload(0, false, false).unwrap();
                s.payload(1, true, true).unwrap();
            }
            assert!(s
                .finish(budget(), 0, None, &mut |_| panic!("unexpected replay"))
                .unwrap_err()
                .to_string()
                .contains("hlodFeatureIds"));
        }
        let mut s = store();
        s.tile(0, None, IDENTITY).unwrap();
        s.reference(0, 0).unwrap();
        s.payload(0, false, false).unwrap();
        s.expect_identity();
        assert!(s
            .finish(budget(), 0, None, &mut |_| panic!("unexpected replay"))
            .unwrap_err()
            .to_string()
            .contains("hlodFeatureIds"));
    }
    #[test]
    fn d4_uses_references_and_allows_unchanged_same_payload_cells() {
        for same in [false, true] {
            let mut s = store();
            s.tile(0, None, IDENTITY).unwrap();
            s.tile(1, Some(0), IDENTITY).unwrap();
            s.reference(0, 0).unwrap();
            s.reference(1, if same { 0 } else { 1 }).unwrap();
            s.payload(0, true, true).unwrap();
            if !same {
                s.payload(1, true, true).unwrap();
            }
            let result = s.finish(budget(), 0, None, &mut |_| Ok(replay_fixture(&[])));
            if same {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains("D4 coarse"));
            }
        }
    }
    #[test]
    fn d4_reads_each_reference_against_its_own_descendant_leaves() {
        // Tile 1's references and its leaves' ranks; a sibling leaf of tile 1 with rank 5.
        for (coarse, leaves, refused) in [
            (&[0][..], &[0, 0][..], false),
            (&[0, 1], &[0, 0], true),
            (&[0], &[0, 7], true),
            (&[7, 0], &[0, 7], true),
        ] {
            let mut s = store();
            s.tile(0, None, IDENTITY).unwrap();
            s.tile(1, Some(0), IDENTITY).unwrap();
            let sibling = 2 + leaves.len() as u64;
            for id in 2..sibling {
                s.tile(id, Some(1), IDENTITY).unwrap();
            }
            s.tile(sibling, Some(0), IDENTITY).unwrap();
            for &rank in coarse {
                s.reference(1, rank).unwrap();
            }
            for (i, &rank) in leaves.iter().enumerate() {
                s.reference(2 + i as u64, rank).unwrap();
            }
            s.reference(sibling, 5).unwrap();
            for rank in [0, 1, 5, 7] {
                s.payload(rank, true, true).unwrap();
            }
            let result = s.finish(budget(), 0, None, &mut |_| Ok(replay_fixture(&[])));
            let d4 = result.is_err_and(|e| e.to_string().contains("D4 coarse"));
            assert_eq!(d4, refused, "{coarse:?} over {leaves:?}");
        }
        // Two levels down: tile 2 repeats its leaf's payload, tile 1 does not.
        let mut s = store();
        for (id, parent, rank) in [
            (0, None, None),
            (1, Some(0), Some(0)),
            (2, Some(1), Some(7)),
            (3, Some(2), Some(7)),
        ] {
            s.tile(id, parent, IDENTITY).unwrap();
            if let Some(rank) = rank {
                s.reference(id, rank).unwrap();
            }
        }
        for rank in [0, 7] {
            s.payload(rank, true, true).unwrap();
        }
        let result = s.finish(budget(), 0, None, &mut |_| Ok(replay_fixture(&[])));
        assert!(result.unwrap_err().to_string().contains("D4 coarse"));
    }
    #[test]
    fn empty_add_override_does_not_clear_a_replace_ancestor_barrier() {
        let mut s = store();
        s.tile(0, None, IDENTITY).unwrap();
        s.tile(1, Some(0), IDENTITY).unwrap();
        s.tile(2, Some(1), IDENTITY).unwrap();
        s.kind(0, Some(true), false, 0., None).unwrap();
        s.kind(1, Some(false), false, 0., None).unwrap();
        s.reference(0, 0).unwrap();
        s.reference(2, 2).unwrap();
        s.payload(0, false, false).unwrap();
        s.payload(2, false, false).unwrap();
        s.expect_identity();
        assert!(s
            .finish(budget(), 0, None, &mut |_| panic!("unexpected replay"))
            .unwrap_err()
            .to_string()
            .contains("replaceContentContinuity"));
    }
    #[test]
    fn raw_scene_profile_refuses_uncovered_motion_and_admits_neutral_materials() {
        use serde_json::json;
        let base =
            json!({"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{}]}]});
        let check = |value: &Value| {
            let text = serde_json::to_string(value).unwrap();
            let raw: &RawValue = serde_json::from_str(&text).unwrap();
            admit_scene_profile(raw, &mut identity::Work::new(budget(), 0, 1 << 20))
        };
        check(&base).unwrap();
        let mut neutral = base.clone();
        neutral["extensionsUsed"] = json!(["KHR_materials_ior"]);
        neutral["materials"] = json!([{"extensions":{"KHR_materials_ior":{"ior":1.4}}}]);
        check(&neutral).unwrap();
        let mut animation = base.clone();
        animation["animations"] = json!([{}]);
        let mut ambiguous = base.clone();
        ambiguous["scenes"] = json!([{"nodes":[0]},{"nodes":[0]}]);
        let mut rtc = base.clone();
        rtc["extensions"] = json!({"CESIUM_RTC":{"center":[1,0,0]}});
        let mut node = base.clone();
        node["nodes"][0]["extensions"] = json!({"EXT_mesh_gpu_instancing":{}});
        let mut mesh = base.clone();
        mesh["meshes"][0]["extensions"] = json!({"UNKNOWN_motion":{}});
        let mut primitive = base.clone();
        primitive["meshes"][0]["primitives"][0]["extensions"] = json!({"UNKNOWN_motion":{}});
        for value in [animation, ambiguous, rtc, node, mesh, primitive] {
            assert!(matches!(check(&value), Err(Error::Unsupported(_))));
        }
        ambiguous = base.clone();
        ambiguous["scenes"] = json!([{"nodes":[0]},{"nodes":[0]}]);
        ambiguous["scene"] = json!(0);
        check(&ambiguous).unwrap();
        let opaque:&RawValue=serde_json::from_str(r#"{"scenes":[{"nodes":[]}],"extras":{"opaque":1e400},"materials":[{"extensions":{"KHR_materials_ior":{"opaque":1e400}}}]}"#).unwrap();
        admit_scene_profile(opaque, &mut identity::Work::new(budget(), 0, 1 << 20)).unwrap();
    }
    #[test]
    fn traversal_scene_exclusion_cannot_advertise_identity_interface_coverage() {
        for identity in [false, true] {
            let mut s = store();
            s.tile(0, None, IDENTITY).unwrap();
            s.reference(0, 0).unwrap();
            s.payload(0, identity, identity).unwrap();
            s.exclude_scene("tileset glTF up-axis override is outside the Y-up profile");
            let result = s.finish(budget(), 0, None, &mut |_| panic!("unexpected replay"));
            if identity {
                assert!(matches!(result, Err(Error::Unsupported(_))));
            } else {
                result.unwrap();
            }
        }
    }
    pub(super) fn replay_fixture(faces: &[[Point; 3]]) -> Vec<u8> {
        let bin: Vec<u8> = faces
            .iter()
            .flatten()
            .flatten()
            .flat_map(|v| (*v as f32).to_le_bytes())
            .collect();
        let doc = serde_json::json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":faces.len()*3}],"bufferViews":[{"buffer":0,"byteLength":bin.len()}],"buffers":[{"byteLength":bin.len()}]});
        pack_fixture(doc, bin)
    }
    fn pack_fixture(doc: Value, mut bin: Vec<u8>) -> Vec<u8> {
        let mut json = serde_json::to_vec(&doc).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let mut bytes = b"glTF".to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend(((28 + json.len() + bin.len()) as u32).to_le_bytes());
        bytes.extend((json.len() as u32).to_le_bytes());
        bytes.extend(b"JSON");
        bytes.extend(json);
        bytes.extend((bin.len() as u32).to_le_bytes());
        bytes.extend(b"BIN\0");
        bytes.extend(bin);
        bytes
    }
    #[test]
    fn original_replay_compares_closed_aliases_after_exact_reference_placement() {
        // Rows: (rank, payload-frame x of vertex 0, placement x shift). A rebased
        // payload placed back exactly agrees; any gap, or a repeated reference to the
        // same payload moved by a gap, refuses.
        fn aliases(items: &[(i64, f64, f64)]) -> Result<(), Error> {
            let mut s = store();
            s.materialize()?;
            let mut seen = BTreeSet::new();
            for (id, &(rank, x, shift)) in items.iter().enumerate() {
                if seen.insert(rank) {
                    s.original(Original {
                        rank: rank as u64,
                        source: [0; 5],
                        vertex: 0,
                        point: [x, 1., 2.].map(f64::to_bits),
                    })?;
                }
                let mut frame = IDENTITY;
                frame[12] = shift;
                s.connection
                    .execute(
                        "INSERT INTO placements(id,tile,rank,frame,leaf) VALUES(?,?,?,?,1)",
                        params![id as i64, id as i64, rank, encode(&frame)],
                    )
                    .map_err(sql)?;
            }
            s.replay_original_aliases(&effort())
        }
        aliases(&[(0, 0., 0.), (1, -10., 10.)]).unwrap();
        aliases(&[(0, 0., 0.), (0, 0., 0.)]).unwrap();
        // One ulp at 10: exact identity, never a tolerance.
        let ulp = f64::from_bits(10f64.to_bits() + 1) - 10.;
        for gap in [0.125, 10., ulp] {
            assert!(aliases(&[(0, 0., 0.), (1, -10., 10. + gap)])
                .unwrap_err()
                .to_string()
                .contains("sourceIdentity"));
            assert!(aliases(&[(0, 0., 0.), (0, 0., gap)])
                .unwrap_err()
                .to_string()
                .contains("sourceIdentity"));
        }
    }
    /// Literal original rows (vertex, payload-frame point), one leaf placement each.
    fn original_rows(rows: &[(u64, Point)]) -> Store {
        let mut s = store();
        s.materialize().unwrap();
        for (rank, &(vertex, p)) in rows.iter().enumerate() {
            s.original(Original {
                rank: rank as u64,
                source: [0; 5],
                vertex,
                point: p.map(f64::to_bits),
            })
            .unwrap();
            s.connection
                .execute(
                    "INSERT INTO placements(id,tile,rank,frame,leaf) VALUES(?,?,?,?,1)",
                    params![rank as i64, rank as i64, rank as i64, encode(&IDENTITY)],
                )
                .unwrap();
        }
        s
    }
    #[test]
    fn original_keys_are_exact_full_width_uint64() {
        let (p, q) = ([1., 2., 3.], [1., 2., 3.5]);
        for (a, b, compared) in [
            (i64::MAX as u64, i64::MAX as u64, true),
            (u64::MAX, u64::MAX, true),
            (1u64 << 63, (1u64 << 63) - 1, false),
            (u64::MAX, u64::MAX - 1, false),
            (0, u64::MAX, false),
        ] {
            let result = original_rows(&[(a, p), (b, q)]).replay_original_aliases(&effort());
            if compared {
                assert!(result.unwrap_err().to_string().contains("sourceIdentity"));
            } else {
                result.unwrap();
            }
            original_rows(&[(a, p), (b, p)])
                .replay_original_aliases(&effort())
                .unwrap();
        }
    }
    /// Negative control of the sorted alias pass: a forced-spill sort (runs of four keys,
    /// fan-in two) and a payload with more leaf placements than one frame chunk still
    /// refuse one vertex whose exact placed position differs only at its last placement,
    /// between payloads recorded far apart, and accept the same rows without the gap.
    #[test]
    fn sorted_alias_pass_catches_violations_across_runs_and_frame_chunks() {
        let ulp = f64::from_bits(10f64.to_bits() + 1) - 10.;
        for config in [CatalogConfig::DEFAULT, CatalogConfig::FORCED_SPILL] {
            for gap in [0., ulp] {
                let scratch = Scratch::create(None, 4096).unwrap();
                let mut s = Store::new(Arc::new(scratch), &config, Resources::FIXED).unwrap();
                s.materialize().unwrap();
                let placements = ALIAS_FRAMES as i64 * 2 + 1;
                // Payload 0: many distinct vertices and `placements` identical frames but
                // the last, which moves vertex 7 when the gap is nonzero; then 40 filler
                // payloads; then payload 41 holding vertex 7 at its rebased position.
                for v in 0..20u64 {
                    let x = if v == 7 { 0. } else { v as f64 + 0.5 };
                    s.original(Original {
                        rank: 0,
                        source: [1, 2, 3, 4, 5],
                        vertex: v,
                        point: [x, 1., 2.].map(f64::to_bits),
                    })
                    .unwrap();
                }
                for rank in 1..=40u64 {
                    s.original(Original {
                        rank,
                        source: [9; 5],
                        vertex: rank,
                        point: [0.; 3].map(f64::to_bits),
                    })
                    .unwrap();
                }
                s.original(Original {
                    rank: 41,
                    source: [1, 2, 3, 4, 5],
                    vertex: 7,
                    point: [-10., 1., 2.].map(f64::to_bits),
                })
                .unwrap();
                let mut id = 0i64;
                let mut place = |s: &Store, rank: i64, shift: f64| {
                    let mut frame = IDENTITY;
                    frame[12] = shift;
                    s.connection
                        .execute(
                            "INSERT INTO placements(id,tile,rank,frame,leaf) VALUES(?,?,?,?,1)",
                            params![id, id, rank, encode(&frame)],
                        )
                        .unwrap();
                    id += 1;
                };
                for k in 0..placements {
                    place(&s, 0, if k == placements - 1 { gap } else { 0. });
                }
                for rank in 1..=40 {
                    place(&s, rank, 0.);
                }
                place(&s, 41, 10.);
                let result = s.replay_original_aliases(&effort());
                if gap == 0. {
                    result.unwrap();
                } else {
                    assert!(result
                        .unwrap_err()
                        .to_string()
                        .contains("distinct exact decoded placed positions"));
                }
            }
        }
    }
    #[test]
    fn original_scan_charges_every_row() {
        let rows: Vec<(u64, Point)> = (0..4).map(|v| (v, [v as f64, 0., 0.])).collect();
        let mut s = original_rows(&rows);
        let mut work = effort();
        work.budget.value = 3;
        assert!(s
            .replay_original_aliases(&work)
            .unwrap_err()
            .exceeded()
            .is_some());
        original_rows(&rows)
            .replay_original_aliases(&effort())
            .unwrap();
    }

    fn delivery_case(
        coarse: &[[Point; 3]],
        left: &[[Point; 3]],
        right: &[[Point; 3]],
    ) -> Result<DecodedInterfaceCensus, Error> {
        let payloads = [
            replay_fixture(coarse),
            replay_fixture(left),
            replay_fixture(right),
        ];
        let mut s = store();
        s.decoded_limit = 1 << 24;
        for (id, parent) in [(0, None), (1, Some(0)), (2, Some(1)), (3, Some(0))] {
            s.tile(id, parent, IDENTITY)?;
        }
        // The omitted root refines (G = 16 x its unit diagonal).
        s.kind(0, None, false, 16., Some(1.))?;
        for (tile, rank) in [(1, 0), (2, 1), (3, 2)] {
            s.reference(tile, rank)?;
            s.payload(rank, true, rank != 0)?;
        }
        assert!(s.census().is_none());
        s.finish(budget(), 0, None, &mut |rank| {
            Ok(payloads[rank as usize].clone())
        })?;
        let mut census = s.census().unwrap();
        // The fixed hierarchy: an omitted root above two rendered cells.
        assert_eq!(
            std::mem::take(&mut census.omission),
            Omission {
                omitted_cells: 1,
                omission_sources: 1,
                omitted_by_depth: vec![1],
                frontier_tiles: 2,
                frontier_leaves: 2,
            }
        );
        Ok(census)
    }
    #[test]
    fn exact_shared_edge_census_and_direction_are_literal() {
        let (a, b, c, d) = ([0., 0., 0.], [2., 0., 0.], [0., 2., 0.], [2., -2., 0.]);
        let left = [[a, b, c]];
        let right = [[b, a, d]];
        assert_eq!(
            delivery_case(&left, &left, &right).unwrap(),
            DecodedInterfaceCensus {
                shared_points: 2,
                shared_segments: 1,
                source_open_edges: 4,
                ..Default::default()
            }
        );
        let t = [1., 0., 0.];
        // Keeping the endpoints and a collinear chain does not keep edge AB.
        let split = [[a, t, c], [t, b, c]];
        assert!(delivery_case(&split, &left, &right)
            .unwrap_err()
            .to_string()
            .contains("interfaceSegments"));
        assert!(delivery_case(&[[b, a, c]], &left, &right)
            .unwrap_err()
            .to_string()
            .contains("interfaceWinding"));
    }
    #[test]
    fn balanced_source_directions_allow_one_retained_coarse_incidence() {
        let (a, b, c, d, e) = (
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [2., -2., 0.],
            [0., 0., 2.],
        );
        // AB has both source directions inside the left subtree. Keeping
        // source face ABC preserves the edge without retaining source BAD.
        let census = delivery_case(&[[a, b, c]], &[[a, b, c], [b, a, d]], &[[a, b, e]]).unwrap();
        assert_eq!(census.shared_segments, 1);
    }
    #[test]
    fn one_source_direction_rejects_a_reversed_coarse_incidence() {
        let (a, b, c, d) = ([0., 0., 0.], [2., 0., 0.], [0., 2., 0.], [2., -2., 0.]);
        assert!(delivery_case(&[[b, a, c]], &[[a, b, c]], &[[b, a, d]])
            .unwrap_err()
            .to_string()
            .contains("interfaceWinding"));
    }
    #[test]
    fn one_source_direction_rejects_an_extra_opposite_coarse_incidence() {
        let (a, b, c, d, e) = (
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [2., -2., 0.],
            [0., 0., 2.],
        );
        // The valid ABC face cannot mask the unsupported opposite AB direction.
        assert!(
            delivery_case(&[[a, b, c], [b, a, e]], &[[a, b, c]], &[[b, a, d]])
                .unwrap_err()
                .to_string()
                .contains("interfaceWinding")
        );
    }
    #[test]
    fn t_junction_and_partial_overlap_create_no_interface_obligation() {
        let left = [[[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]]];
        // The coarse tile keeps its leaf: a lost child component is a declared drop.
        let coarse = left;
        for right in [
            [[[1., 0., 0.], [1., -1., 1.], [1., -1., -1.]]],
            [[[0.5, 0., 0.], [1.5, 0., 0.], [1., -1., 0.]]],
        ] {
            let census = delivery_case(&coarse, &left, &right).unwrap();
            assert_eq!(census.shared_points, 0);
            assert_eq!(census.shared_segments, 0);
            assert_eq!(census.source_open_edges, 6);
            assert_eq!(census.source_open_edges_with_shared_endpoints, 0);
        }
    }
    #[test]
    fn open_chord_is_counted_but_shared_points_remain_strict() {
        let (a, b, c, d) = ([0., 0., 0.], [2., 0., 0.], [0., 2., 0.], [1., 1., 1.]);
        let left = [[a, b, c]];
        let right = [
            [a, [-2., 0., 1.], [-2., 1., 1.]],
            [b, [4., 0., 1.], [4., 1., 1.]],
        ];
        let without_chord = [[a, d, c], [b, c, d]];
        let census = delivery_case(&without_chord, &left, &right).unwrap();
        assert_eq!(census.shared_points, 2);
        assert_eq!(census.shared_segments, 0);
        assert_eq!(census.source_open_edges, 9);
        assert_eq!(census.source_open_edges_with_shared_endpoints, 1);
        assert!(delivery_case(&[[b, c, d]], &left, &right)
            .unwrap_err()
            .to_string()
            .contains("interfacePoints"));
    }
    #[test]
    fn balanced_and_nonmanifold_incident_owners_are_never_cancelled() {
        let (a, b, c, d, e) = (
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [2., -2., 0.],
            [0., 0., 2.],
        );
        let left = [[a, b, c], [b, a, d]];
        let right = [[a, b, e]];
        let census = delivery_case(&left, &left, &right).unwrap();
        assert_eq!(census.shared_segments, 1);
        assert_eq!(census.source_open_edges, 6);
        let lost = [[a, d, c], [b, c, d]];
        assert!(delivery_case(&lost, &left, &right)
            .unwrap_err()
            .to_string()
            .contains("interfaceSegments"));
        let f = [0., 0., -2.];
        let closed_left = [[a, b, c], [b, a, e], [a, c, e], [b, e, c]];
        let closed_right = [[a, b, d], [b, a, f], [a, d, f], [b, f, d]];
        let census = delivery_case(&closed_left, &closed_left, &closed_right).unwrap();
        assert_eq!(census.shared_points, 2);
        assert_eq!(census.shared_segments, 1);
        assert_eq!(census.source_open_edges, 0);
        let lost = [[a, e, c], [b, c, e], [a, c, e], [b, e, c]];
        assert!(delivery_case(&lost, &closed_left, &closed_right)
            .unwrap_err()
            .to_string()
            .contains("interfaceSegments"));
    }
    #[test]
    fn canonical_degeneracy_collinearity_and_original_identity_stay_distinct() {
        let mut work = identity::Work::new(budget(), 0, 1 << 20);
        let mut s = Surface::default();
        s.triangle(
            [[0., 0., 0.], [-0., 0., 0.], [9., 9., 9.]],
            &mut work,
            1 << 20,
        )
        .unwrap();
        assert_eq!(s.triangles, 1);
        assert!(s.points.is_empty());
        assert!(s.edges.is_empty());
        s.triangle(
            [[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]],
            &mut work,
            1 << 20,
        )
        .unwrap();
        assert_eq!(s.points.len(), 3);
        assert_eq!(s.edges.len(), 3);
        assert!(canonical([f64::MAX, 0., 0.]).is_err());
        assert_eq!(
            canonical([-0., 0., 0.]).unwrap(),
            canonical([0.; 3]).unwrap()
        );
        let a = [1., 0., 0.];
        let b = [f64::from_bits(1f64.to_bits() + 1), 0., 0.];
        assert_eq!(canonical(a).unwrap(), canonical(b).unwrap());
        // Original identity keeps the exact f64 bits the f32 seam key merges.
        let mut s = original_rows(&[(0, a), (0, b)]);
        assert!(s
            .replay_original_aliases(&effort())
            .unwrap_err()
            .to_string()
            .contains("sourceIdentity"));
    }
    #[test]
    fn every_incident_face_and_repeated_reference_contributes_to_open_count() {
        let mut s = store();
        s.materialize().unwrap();
        let face = [[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]];
        let surface =
            replay_surface(&replay_fixture(&face), IDENTITY, 1 << 20, &effort(), false).unwrap();
        let mut rows = s.sorter().unwrap();
        s.record_leaf(7, &surface, &mut rows, &effort()).unwrap();
        s.record_leaf(7, &surface, &mut rows, &effort()).unwrap();
        s.load_leaves(rows).unwrap();
        let mut census = DecodedInterfaceCensus::default();
        let mut needs = s.sorter().unwrap();
        s.point_requirements(&mut census, &mut needs, &effort())
            .unwrap();
        s.segment_requirements(&mut census, &mut needs, &effort())
            .unwrap();
        assert_eq!(census.source_open_edges, 0);
        assert_eq!(census.shared_segments, 0);
        let incidences: i64 = s
            .connection
            .query_row("SELECT sum(nf+nr) FROM leaf_segments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(incidences, 6);
    }
    #[test]
    fn root_local_winding_check_survives_without_exterior_heuristics() {
        let (a, b, c, d) = ([0., 0., 0.], [2., 0., 0.], [0., 2., 0.], [2., -2., 0.]);
        let payloads = [
            replay_fixture(&[[a, b, c], [a, b, d]]),
            replay_fixture(&[[a, b, c]]),
            replay_fixture(&[[b, a, d]]),
        ];
        let mut s = store();
        s.decoded_limit = 1 << 20;
        s.tile(0, None, IDENTITY).unwrap();
        s.tile(1, Some(0), IDENTITY).unwrap();
        s.tile(2, Some(0), IDENTITY).unwrap();
        for rank in 0..3 {
            s.reference(rank, rank).unwrap();
            s.payload(rank, true, rank != 0).unwrap();
        }
        assert!(s
            .finish(budget(), 0, None, &mut |rank| Ok(
                payloads[rank as usize].clone()
            ))
            .unwrap_err()
            .to_string()
            .contains("interfaceWinding"));
        assert!(s.census().is_none());
    }
    /// A book's spine: two source pages traverse AB forward and a third backward.
    /// The coarse pair of forward faces is retained source multiplicity; with only
    /// one forward source face (the test above) the same coarse pair is refused.
    #[test]
    fn nonmanifold_source_multiplicity_admits_equal_coarse_directions() {
        let (a, b, c, d, e) = (
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [2., -2., 0.],
            [0., 0., 2.],
        );
        let payloads = [
            replay_fixture(&[[a, b, c], [a, b, d]]),
            replay_fixture(&[[a, b, c]]),
            replay_fixture(&[[a, b, d]]),
            replay_fixture(&[[b, a, e]]),
        ];
        let mut s = store();
        s.decoded_limit = 1 << 20;
        s.tile(0, None, IDENTITY).unwrap();
        for leaf in 1..4 {
            s.tile(leaf, Some(0), IDENTITY).unwrap();
        }
        for rank in 0..4 {
            s.reference(rank, rank).unwrap();
            s.payload(rank, true, rank != 0).unwrap();
        }
        s.finish(budget(), 0, None, &mut |rank| {
            Ok(payloads[rank as usize].clone())
        })
        .unwrap();
        assert_eq!(s.census().unwrap().shared_segments, 1);
    }
    #[test]
    fn hard_cap_refuses_any_payload_above_8192_while_d4_property_rows_remain_bounded() {
        // Leaf or coarse, the cap is per payload; 8,192 exactly is admitted.
        for (tile, triangles, ok) in [(0, 8192, true), (0, 8193, false), (1, 8193, false)] {
            let mut s = store();
            s.tile(0, None, IDENTITY).unwrap();
            s.tile(1, Some(0), IDENTITY).unwrap();
            for rank in [0, 1] {
                s.reference(rank, rank).unwrap();
                s.payload(rank, true, rank == 1).unwrap();
            }
            s.materialize().unwrap();
            s.connection
                .execute(
                    "UPDATE payloads SET triangles=? WHERE rank=?",
                    params![triangles, tile],
                )
                .unwrap();
            let result = s.finish(budget(), 0, None, &mut |_| Ok(replay_fixture(&[])));
            if ok {
                result.unwrap();
                assert_eq!(s.census().unwrap().max_payload_triangles, 8192);
            } else {
                let error = result.unwrap_err().to_string();
                assert!(
                    error.contains("triangleCap") && error.contains("8193"),
                    "{error}"
                );
            }
        }
        for same in [false, true] {
            let mut s = store();
            s.decoded_limit = 1 << 20;
            s.tile(0, None, IDENTITY).unwrap();
            s.tile(1, Some(0), IDENTITY).unwrap();
            s.reference(0, 0).unwrap();
            s.reference(1, if same { 0 } else { 1 }).unwrap();
            s.payload(0, true, same).unwrap();
            if !same {
                s.payload(1, true, true).unwrap();
            }
            s.materialize().unwrap();
            s.connection
                .execute("UPDATE payloads SET primitive_rows=8193 WHERE rank=0", [])
                .unwrap();
            let result = s.finish(budget(), 0, None, &mut |_| Ok(replay_fixture(&[])));
            if same {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains("hlodFeatureIds"));
            }
        }
    }
    /// One delivered tile in preorder: (parent, placeholder, refine, payload ranks).
    /// Ranks on childless tiles are leaf payloads (with original triangle rows).
    type Row<'a> = (Option<u64>, bool, Option<bool>, &'a [u64]);
    fn omission_case(tiles: &[Row<'_>], digests: &[(u64, u64)]) -> Result<Omission, Error> {
        omission_errors(tiles, digests, &[])
    }
    /// `errors` overrides a tile's (geometricError, box diagonal); the default is the
    /// refinable floor 16 over a unit diagonal.
    fn omission_errors(
        tiles: &[Row<'_>],
        digests: &[(u64, u64)],
        errors: &[(u64, f64, Option<f64>)],
    ) -> Result<Omission, Error> {
        let mut s = store();
        s.decoded_limit = 1 << 20;
        let mut seen = BTreeSet::new();
        for (id, &(parent, placeholder, refine, ranks)) in tiles.iter().enumerate() {
            let id = id as u64;
            s.tile(id, parent, IDENTITY)?;
            let (_, error, diagonal) =
                errors
                    .iter()
                    .copied()
                    .find(|e| e.0 == id)
                    .unwrap_or((id, 16., Some(1.)));
            s.kind(id, refine, placeholder, error, diagonal)?;
            let leaf = !tiles.iter().any(|t| t.0 == Some(id));
            for &rank in ranks {
                s.reference(id, rank)?;
                if seen.insert(rank) {
                    let digest = digests.iter().find(|d| d.0 == rank).map_or(rank, |d| d.1);
                    s.payload_bytes(rank, true, leaf, &digest.to_be_bytes())?;
                }
            }
        }
        s.finish(budget(), 0, None, &mut |_| Ok(replay_fixture(&[])))?;
        Ok(s.census().unwrap().omission)
    }
    fn refusal(result: Result<Omission, Error>, check: &str) {
        let error = result.unwrap_err().to_string();
        assert!(error.contains(check), "{check}: {error}");
    }
    /// Either barrier owner: REPLACE continuity (all archives) runs first.
    fn below_content(result: Result<Omission, Error>) {
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("omissionPropagation") || error.contains("replaceContentContinuity"),
            "{error}"
        );
    }
    #[test]
    fn omission_is_counted_by_cell_depth_through_placeholders() {
        const N: &[u64] = &[];
        // 0 omitted root; 1 an external link to 2 (the same cell's document root,
        // omitted); 3 a leaf; 4 an omission source over leaves 5 and 6; 7 a coarse
        // frontier tile over leaf 8.
        let tiles = [
            (None, false, None, N),
            (Some(0), true, None, N),
            (Some(1), false, None, N),
            (Some(2), false, None, &[3][..]),
            (Some(2), false, None, N),
            (Some(4), false, None, &[5][..]),
            (Some(4), false, None, &[6][..]),
            (Some(0), false, None, &[7][..]),
            (Some(7), false, None, &[8][..]),
        ];
        assert_eq!(
            omission_case(&tiles, &[]).unwrap(),
            Omission {
                omitted_cells: 3,
                omission_sources: 1,
                omitted_by_depth: vec![1, 1, 1],
                frontier_tiles: 4,
                frontier_leaves: 4,
            }
        );
        // Sensitivity: a non-placeholder link adds a cell (depth 2 holds two).
        let mut deeper = tiles;
        deeper[1].1 = false;
        assert_eq!(
            omission_case(&deeper, &[]).unwrap().omitted_by_depth,
            vec![1, 1, 1, 1]
        );
        // All-omitted chain to the root above one source.
        let chain = [
            (None, false, None, N),
            (Some(0), false, None, N),
            (Some(1), false, None, &[1][..]),
            (Some(1), false, None, &[2][..]),
        ];
        let omission = omission_case(&chain, &[]).unwrap();
        assert_eq!(
            (
                omission.omitted_cells,
                omission.omission_sources,
                omission.omitted_by_depth
            ),
            (2, 1, vec![1, 1])
        );
    }
    #[test]
    fn content_above_an_omitted_cell_is_refused_in_every_subtree_form() {
        const N: &[u64] = &[];
        // Root content over an omitted cell over two leaves.
        let mut tiles = vec![
            (None, false, None, &[9][..]),
            (Some(0), false, None, N),
            (Some(1), false, None, &[1][..]),
            (Some(1), false, None, &[2][..]),
        ];
        below_content(omission_case(&tiles, &[]));
        // Control: the same tree with the root omitted too is admitted.
        tiles[0].3 = N;
        omission_case(&tiles, &[]).unwrap();
        // External or implicit placeholder between the content and the omitted root.
        let linked = [
            (None, false, None, &[9][..]),
            (Some(0), true, None, N),
            (Some(1), false, None, N),
            (Some(2), false, None, &[1][..]),
            (Some(2), false, None, &[2][..]),
        ];
        below_content(omission_case(&linked, &[]));
        // Content below an omitted cell's empty sibling path: deep descendant.
        let deep = [
            (None, false, None, N),
            (Some(0), false, None, &[9][..]),
            (Some(1), false, None, &[8][..]),
            (Some(2), false, None, N),
            (Some(3), false, None, &[1][..]),
        ];
        below_content(omission_case(&deep, &[]));
        // Only the omission owner sees an empty branch end below content (no
        // renderable descendant, so no REPLACE barrier), even past a placeholder.
        for placeholder in [false, true] {
            let mut ends = vec![
                (None, false, None, &[9][..]),
                (Some(0), false, None, &[1][..]),
                (Some(0), placeholder, None, N),
            ];
            if placeholder {
                ends.push((Some(2), false, None, N));
            }
            refusal(omission_case(&ends, &[]), "omissionPropagation");
        }
    }
    /// R9 (S1 756283e2): an omitted cell carrying its children's error is never
    /// refined by CesiumJS, so nothing below it draws.
    #[test]
    fn content_less_cells_must_refine_under_cesium_sse() {
        const N: &[u64] = &[];
        let tiles = [
            (None, false, None, N),
            (Some(0), true, None, N),
            (Some(1), false, None, N),
            (Some(2), false, None, &[1][..]),
            (Some(2), false, None, &[2][..]),
            (Some(0), false, None, &[3][..]),
        ];
        omission_errors(&tiles, &[], &[]).unwrap();
        // Exactly the floor passes; children's leaf error at the root, or one ulp
        // below the floor at the external document's root, is refused.
        omission_errors(&tiles, &[], &[(0, 160., Some(10.))]).unwrap();
        for errors in [
            [(0, 0.000671, Some(141.5 / 16.))],
            [(2, 16. * (1. - 1e-9), Some(1.))],
            [(2, 16., None)],
        ] {
            refusal(omission_errors(&tiles, &[], &errors), "emptyCellRefinement");
        }
        // A link placeholder always traverses: its own error is not a cell's.
        omission_errors(&tiles, &[], &[(1, 0., Some(1.))]).unwrap();
    }
    #[test]
    fn the_first_renderable_cover_is_complete_and_single() {
        const N: &[u64] = &[];
        // A terminal content-less cell leaves its branch uncovered.
        let open = [
            (None, false, None, N),
            (Some(0), false, None, &[1][..]),
            (Some(0), false, None, N),
        ];
        refusal(omission_case(&open, &[]), "frontierCoverage");
        let dangling = [
            (None, false, None, N),
            (Some(0), false, None, &[1][..]),
            (Some(0), true, None, N),
        ];
        refusal(omission_case(&dangling, &[]), "frontierCoverage");
        // ADD content stays drawn over its leaves.
        let add = [
            (None, false, Some(false), &[9][..]),
            (Some(0), false, None, &[1][..]),
            (Some(0), false, None, &[2][..]),
        ];
        refusal(omission_case(&add, &[]), "frontierCoverage");
        let mut replace = add;
        replace[0].2 = Some(true);
        omission_case(&replace, &[]).unwrap();
        // A coarse payload standing for its only child down a chain is one cover.
        let chain = [
            (None, false, None, N),
            (Some(0), false, None, &[9][..]),
            (Some(1), false, None, &[9][..]),
            (Some(2), false, None, &[1][..]),
            (Some(2), false, None, &[2][..]),
        ];
        let omission = omission_case(&chain, &[]).unwrap();
        assert_eq!((omission.frontier_tiles, omission.frontier_leaves), (1, 2));
        // The same coarse payload in two branches, by reference or by copied bytes.
        let across = [
            (None, false, None, N),
            (Some(0), false, None, &[9][..]),
            (Some(1), false, None, &[1][..]),
            (Some(1), false, None, &[2][..]),
            (Some(0), false, None, &[10][..]),
            (Some(4), false, None, &[3][..]),
            (Some(4), false, None, &[4][..]),
        ];
        omission_case(&across, &[]).unwrap();
        refusal(omission_case(&across, &[(10, 9)]), "frontierCoverage");
        let mut shared = across;
        shared[4].3 = &[9];
        refusal(omission_case(&shared, &[]), "frontierCoverage");
        // A repeat that skips a branching cell is not a chain.
        let skip = [
            (None, false, None, N),
            (Some(0), false, None, &[9][..]),
            (Some(1), false, None, &[1][..]),
            (Some(1), false, None, &[10][..]),
            (Some(3), false, None, &[2][..]),
        ];
        omission_case(&skip, &[]).unwrap();
        refusal(omission_case(&skip, &[(10, 9)]), "frontierCoverage");
        // The same payload twice in one tile renders it twice.
        let twice = [
            (None, false, None, N),
            (Some(0), false, None, &[1, 1][..]),
            (Some(0), false, None, &[2][..]),
        ];
        refusal(omission_case(&twice, &[]), "frontierCoverage");
        let copied = [
            (None, false, None, N),
            (Some(0), false, None, &[1, 3][..]),
            (Some(0), false, None, &[2][..]),
        ];
        omission_case(&copied, &[]).unwrap();
        refusal(omission_case(&copied, &[(3, 1)]), "frontierCoverage");
    }
    #[test]
    fn common_root_and_y_up_frame_are_applied_before_canonicalization() {
        let mut s = store();
        let mut root = IDENTITY;
        root[12] = 6_000_000.;
        root[13] = -4_000_000.;
        s.tile(0, None, root).unwrap();
        let mut local = IDENTITY;
        local[14] = -10.;
        s.tile(1, Some(0), local).unwrap();
        s.reference(1, 0).unwrap();
        s.materialize().unwrap();
        s.hierarchy(&effort()).unwrap();
        s.place_references(&effort()).unwrap();
        let frame: Vec<u8> = s
            .connection
            .query_row("SELECT frame FROM placements", [], |r| r.get(0))
            .unwrap();
        let frame = decode::<16>(&frame).unwrap();
        let face = [[[0., 10., 0.], [1., 10., 0.], [0., 11., 0.]]];
        let surface =
            replay_surface(&replay_fixture(&face), frame, 1 << 20, &effort(), false).unwrap();
        assert!(surface.points.contains(&canonical([0., 0., 0.]).unwrap()));
        assert!(!surface
            .points
            .contains(&canonical(point(local, [0., 10., 0.])).unwrap()));
    }

    /// One face per entry, with its three corner NORMALs as snorm8 (`KHR_mesh_quantization`
    /// style) and an optional node scale.
    fn normal_fixture(faces: &[([Point; 3], [Point; 3])], scale: [f64; 3]) -> Vec<u8> {
        let mut bin: Vec<u8> = faces
            .iter()
            .flat_map(|f| f.0)
            .flatten()
            .flat_map(|v| (v as f32).to_le_bytes())
            .collect();
        let offset = bin.len();
        for n in faces.iter().flat_map(|f| f.1) {
            bin.extend(n.map(|v| ((v * 127.).round() as i8).to_le_bytes()[0]));
            bin.push(0);
        }
        let count = faces.len() * 3;
        let doc = serde_json::json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0,"scale":scale}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1}}]}],
            "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":count},
                {"bufferView":1,"componentType":5120,"normalized":true,"type":"VEC3","count":count}],
            "bufferViews":[{"buffer":0,"byteLength":offset},
                {"buffer":0,"byteOffset":offset,"byteLength":bin.len() - offset,"byteStride":4}],
            "buffers":[{"byteLength":bin.len()}]});
        pack_fixture(doc, bin)
    }
    /// A corner NORMAL more than 90° from its face (wound by its corners) counts the face
    /// once; 89° does not; zero-area faces and zero normals are not compared; leaves
    /// (`normals: false`) are never compared.
    #[test]
    fn coarse_replay_counts_faces_shaded_against_their_winding() {
        let tri = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let up = [0., 0., 1.];
        let tilt = |deg: f64| {
            let r = deg.to_radians();
            [r.sin(), 0., r.cos()]
        };
        let faces = [
            (tri, [up; 3]),
            (tri, [up, tilt(89.), up]),
            (tri, [up, up, tilt(91.)]),
            (tri, [[0., 0., -1.]; 3]),
            (tri, [[0.; 3], up, up]),
            (
                [[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]],
                [[0., 0., -1.]; 3],
            ),
        ];
        let surface = replay_surface(
            &normal_fixture(&faces, [1.; 3]),
            IDENTITY,
            1 << 20,
            &effort(),
            true,
        )
        .unwrap();
        assert_eq!((surface.normal_faces, surface.inverted_faces), (5, 2));
        let surface = replay_surface(
            &normal_fixture(&faces, [1.; 3]),
            IDENTITY,
            1 << 20,
            &effort(),
            false,
        )
        .unwrap();
        assert_eq!((surface.normal_faces, surface.inverted_faces), (0, 0));
        // A NORMAL behind a meshopt filter is skipped, not decoded (and not refused).
        let mut doc: Value = {
            let b = normal_fixture(&faces, [1.; 3]);
            let l = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
            serde_json::from_slice(&b[20..20 + l]).unwrap()
        };
        doc["bufferViews"][1]["extensions"] = serde_json::json!({"EXT_meshopt_compression":
            {"buffer":0,"byteLength":1,"byteStride":4,"count":18,"mode":"ATTRIBUTES","filter":"OCTAHEDRAL"}});
        let bin = {
            let b = normal_fixture(&faces, [1.; 3]);
            let l = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
            b[28 + l..].to_vec()
        };
        let surface = replay_surface(
            &pack_fixture(doc.clone(), bin.clone()),
            IDENTITY,
            1 << 20,
            &effort(),
            true,
        )
        .unwrap();
        assert_eq!(
            (
                surface.normal_faces,
                surface.inverted_faces,
                surface.normal_skips
            ),
            (0, 0, 1)
        );
        // Mutants the comparison does not read are skipped with their reason, never
        // refused: sparse, viewless (zero storage), and each unsupported shape.
        doc["bufferViews"][1]
            .as_object_mut()
            .unwrap()
            .remove("extensions");
        let base = doc.clone();
        type Mutant = (&'static str, fn(&mut Value), u8);
        let mutants: [Mutant; 6] = [
            (
                "sparse",
                |d| d["accessors"][1]["sparse"] = serde_json::json!({"count":1}),
                2,
            ),
            (
                "viewless",
                |d| {
                    d["accessors"][1]
                        .as_object_mut()
                        .unwrap()
                        .remove("bufferView");
                },
                4,
            ),
            (
                "u8",
                |d| d["accessors"][1]["componentType"] = 5121.into(),
                8,
            ),
            (
                "unnormalized",
                |d| d["accessors"][1]["normalized"] = false.into(),
                8,
            ),
            ("vec4", |d| d["accessors"][1]["type"] = "VEC4".into(), 8),
            ("short", |d| d["accessors"][1]["count"] = 3.into(), 8),
        ];
        for (name, mutate, reason) in mutants {
            let mut d = base.clone();
            mutate(&mut d);
            let surface = replay_surface(
                &pack_fixture(d, bin.clone()),
                IDENTITY,
                1 << 20,
                &effort(),
                true,
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                (surface.normal_faces, surface.normal_skips),
                (0, reason),
                "{name}"
            );
        }
        // The unmutated document (no filter) is compared.
        let surface =
            replay_surface(&pack_fixture(base, bin), IDENTITY, 1 << 20, &effort(), true).unwrap();
        assert_eq!((surface.normal_faces, surface.normal_skips), (5, 0));
        // A reflecting, non-uniform node scale reverses the rendered winding and
        // transforms normals by its inverse transpose: the verdicts do not change.
        let surface = replay_surface(
            &normal_fixture(&faces, [-2., 0.5, 3.]),
            IDENTITY,
            1 << 20,
            &effort(),
            true,
        )
        .unwrap();
        assert_eq!((surface.normal_faces, surface.inverted_faces), (5, 2));
    }
    /// The verdict is the world-space one for any nonsingular node transform: the
    /// rendered normal is `L^-T n` and the face normal `(La)×(Lb)`, flipped when
    /// `det L < 0` (glTF reverses the winding).
    #[test]
    fn normal_inversion_matches_world_space_under_shear_and_reflection() {
        let local = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let shear: Matrix = [
            1., 0., 0., 0., 2., -1., 0., 0., 0.5, 0., 3., 0., 0., 0., 0., 1.,
        ];
        let l: [Point; 3] =
            std::array::from_fn(|k| [shear[4 * k], shear[4 * k + 1], shear[4 * k + 2]]);
        let det = dot(l[0], cross(l[1], l[2]));
        assert!(det < 0.);
        let world = local.map(|p| point(shear, p));
        let e = |k: usize| std::array::from_fn(|i| world[k][i] - world[0][i]);
        let face = cross(e(1), e(2)).map(|v| v * det.signum());
        let cof = cofactor(shear);
        for k in 0..64 {
            let a = f64::from(k) * 0.1;
            let n = [a.cos() * 0.6, a.sin() * 0.6, (a * 1.7).sin()];
            // L^-T n = cof(L) n / det.
            let rendered = apply(&cof, n).map(|v| v / det);
            let expected = dot(face, rendered) < 0.;
            assert_eq!(
                normal_inversion(&cof, local, [n; 3]),
                Some(expected),
                "{n:?}"
            );
        }
    }
    #[test]
    fn replay_charges_json_and_decode_work_before_reduction() {
        let bytes = replay_fixture(&[[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]]);
        let mut e = effort();
        e.budget.value = 1;
        assert!(replay_surface(&bytes, IDENTITY, 1 << 20, &e, false)
            .err()
            .unwrap()
            .exceeded()
            .is_some());
    }

    /// The pending buffer is bounded: a long identity-free prefix moves to disk, and a
    /// later HLOD expectation still sees every registered row.
    #[test]
    fn pending_rows_move_to_disk_when_the_buffer_fills() {
        let mut s = store();
        let n = PENDING_ROWS as u64;
        s.tile(0, None, IDENTITY).unwrap();
        for id in 1..n {
            s.tile(id, Some(id - 1), IDENTITY).unwrap();
            assert_eq!(s._file.is_some(), id + 1 >= n, "{id}");
        }
        s.expect_identity();
        let error = s
            .finish(budget(), 0, None, &mut |_| panic!("unexpected replay"))
            .unwrap_err();
        // The replayed chain is checked: its content-less cells have no G.
        assert!(error.to_string().contains("emptyCellRefinement"), "{error}");
        let tiles: i64 = s
            .connection
            .query_row("SELECT count(*) FROM tiles", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tiles as u64, n);
    }
    #[test]
    fn final_work_has_a_structured_budget_refusal() {
        let mut s = store();
        s.tile(0, None, IDENTITY).unwrap();
        s.tile(1, Some(0), IDENTITY).unwrap();
        // Only an HLOD expectation creates delivery work.
        s.expect_identity();
        let error = s
            .finish(
                Budget {
                    value: 1,
                    ..budget()
                },
                0,
                None,
                &mut |_| panic!("unexpected replay"),
            )
            .unwrap_err();
        assert!(error.exceeded().is_some());
    }

    /// The parallel-replay fixture (#342): `copies` disjoint copies of the `delivery_case`
    /// hierarchy (an omitted root over a coarse tile with one leaf below and a sibling leaf),
    /// every third copy with a doubled leaf face so its edges carry two faces in one
    /// direction (an edge-face replay). Returns the outcome, the census and the work units
    /// charged when each replayed placement started.
    fn parallel_case<P: replay::Payloads>(
        workers: usize,
        value: u64,
        payloads: impl FnOnce(Arc<Vec<Vec<u8>>>) -> P,
    ) -> (String, String, Vec<u64>) {
        let copies = 24u64;
        let mut s = Store::new(
            Arc::new(Scratch::create(None, 4096).unwrap()),
            &CatalogConfig::selected(),
            Resources {
                workers,
                window_bytes: 1 << 26,
                ..Resources::FIXED
            },
        )
        .unwrap();
        s.decoded_limit = 1 << 24;
        s.tile(0, None, IDENTITY).unwrap();
        s.kind(0, None, false, 16. * copies as f64, Some(copies as f64))
            .unwrap();
        let mut bytes = Vec::new();
        for k in 0..copies {
            let x = 10. * k as f64;
            let (a, b, c, d) = (
                [x, 0., 0.],
                [x + 2., 0., 0.],
                [x, 2., 0.],
                [x + 2., -2., 0.],
            );
            let left = if k % 3 == 0 {
                replay_fixture(&[[a, b, c], [a, b, c]])
            } else {
                replay_fixture(&[[a, b, c]])
            };
            let (coarse, leaf, sibling) = (1 + 3 * k, 2 + 3 * k, 3 + 3 * k);
            s.tile(coarse, Some(0), IDENTITY).unwrap();
            s.tile(leaf, Some(coarse), IDENTITY).unwrap();
            s.tile(sibling, Some(0), IDENTITY).unwrap();
            // Even copies' coarse payloads publish NORMAL (#371), every fourth with one
            // corner turned against the face, so worker replays count normals too.
            let up = [0., 0., 1.];
            let coarse_payload = match k % 4 {
                0 => normal_fixture(&[([a, b, c], [up, up, [0., 0., -1.]])], [1.; 3]),
                2 => normal_fixture(&[([a, b, c], [up; 3])], [1.; 3]),
                _ => replay_fixture(&[[a, b, c]]),
            };
            for (tile, payload) in [
                (coarse, coarse_payload),
                (leaf, left),
                (sibling, replay_fixture(&[[b, a, d]])),
            ] {
                let rank = bytes.len() as u64;
                bytes.push(payload);
                s.reference(tile, rank).unwrap();
                s.payload(rank, true, tile != coarse).unwrap();
            }
        }
        replay::BOUNDARIES.with(|b| b.borrow_mut().clear());
        let result = s.finish_with(
            Budget { value, ..budget() },
            0,
            None,
            &mut payloads(Arc::new(bytes)),
        );
        let boundaries = replay::BOUNDARIES.with(|b| std::mem::take(&mut *b.borrow_mut()));
        (
            format!("{result:?}"),
            format!("{:?}", s.census()),
            boundaries,
        )
    }

    fn shared(payloads: Arc<Vec<Vec<u8>>>) -> replay::Shared {
        replay::Shared {
            payloads,
            ..Default::default()
        }
    }

    /// #342 (review P3): the lookahead replay on worker threads gives the sequential outcome
    /// (result, work count, census, error and refusal) at every budget that lands on a
    /// placement boundary or one unit either side of it, through the leaf, edge-face and
    /// coarse passes; with worker panics, worker read failures, an admission refusal mid-pass
    /// and a lookahead that disagrees with its pass.
    #[test]
    fn parallel_replay_matches_the_sequential_replay_at_every_budget() {
        let (full, census, boundaries) = parallel_case(1, budget().value, shared);
        assert!(full.starts_with("Ok("), "{full}");
        assert!(census.starts_with("Some("), "{census}");
        // Twelve coarse tiles publish NORMAL; six have an inverted face.
        assert!(
            census.contains("coarse_normals: CoarseNormals { tiles: 12, faces: 12"),
            "{census}"
        );
        assert!(
            census.contains("inverted_tiles_by_depth: [0, 6]"),
            "{census}"
        );
        // Leaf, edge-face and coarse placements were all replayed.
        assert!(boundaries.len() >= 3 * 24 + 8, "{}", boundaries.len());
        let work: u64 = full
            .trim_start_matches("Ok(")
            .trim_end_matches(')')
            .parse()
            .unwrap();
        let mut budgets: Vec<u64> = boundaries
            .iter()
            .flat_map(|&b| [b.saturating_sub(1), b, b + 1])
            .chain([1, 100, work - 1, work, work + 1, budget().value])
            .collect();
        budgets.sort_unstable();
        budgets.dedup();
        for &value in &budgets {
            let sequential = parallel_case(1, value, shared);
            for workers in [2, 4] {
                let parallel = parallel_case(workers, value, shared);
                assert_eq!(
                    (&parallel.0, &parallel.1),
                    (&sequential.0, &sequential.1),
                    "budget {value}, {workers} workers"
                );
            }
        }
        // Worker panics and failed worker reads replay sequentially (review P2-4); an
        // admission refusal stops the pass where one worker stops; a lookahead that skips a
        // row disagrees with its pass and the rest of the pass runs sequentially.
        type Variant = fn(Arc<Vec<Vec<u8>>>) -> replay::Shared;
        let variants: [(Variant, Option<i64>); 4] = [
            (
                |p| replay::Shared {
                    payloads: p,
                    panic: vec![4, 31, 55],
                    ..Default::default()
                },
                None,
            ),
            (
                |p| replay::Shared {
                    payloads: p,
                    corrupt: vec![1, 7, 40],
                    ..Default::default()
                },
                None,
            ),
            (
                |p| replay::Shared {
                    payloads: p,
                    refuse: Some(28),
                    ..Default::default()
                },
                None,
            ),
            (shared, Some(13)),
        ];
        for (index, (variant, skip)) in variants.into_iter().enumerate() {
            for value in [budget().value, work / 2, boundaries[boundaries.len() / 3]] {
                let sequential = parallel_case(1, value, variant);
                for workers in [2, 4] {
                    replay::SKIP_AHEAD.with(|s| s.set(skip));
                    let parallel = parallel_case(workers, value, variant);
                    replay::SKIP_AHEAD.with(|s| s.set(None));
                    assert_eq!(
                        (&parallel.0, &parallel.1),
                        (&sequential.0, &sequential.1),
                        "variant {index}, budget {value}, {workers} workers"
                    );
                }
            }
        }
        // The refusal variant is a refusal, not a pass.
        assert!(parallel_case(4, budget().value, |p| replay::Shared {
            payloads: p,
            refuse: Some(28),
            ..Default::default()
        })
        .0
        .contains("member reads exceed work limit"));
    }

    /// A worker panic replays its placement sequentially; a panic there unwinds normally.
    #[test]
    fn a_panicking_sequential_replay_unwinds_instead_of_aborting() {
        // Only the worker panics: the sequential replay gives the ordinary result.
        struct WorkerOnly(Arc<Vec<Vec<u8>>>, bool);
        impl replay::Payloads for WorkerOnly {
            fn admit(&mut self, _: u64) -> Result<(), Error> {
                Ok(())
            }
            fn fetch(&mut self, rank: u64) -> Result<Vec<u8>, Error> {
                assert!(!self.1 || rank != 31, "injected sequential panic");
                Ok(self.0[rank as usize].clone())
            }
            fn locate(&mut self, _: u64) -> Option<(u64, replay::Fetch)> {
                Some((1, Box::new(|| panic!("injected worker panic"))))
            }
        }
        let expected = parallel_case(1, budget().value, shared);
        let got = parallel_case(4, budget().value, |p| WorkerOnly(p, false));
        assert_eq!((got.0, got.1), (expected.0, expected.1));
        // The same placement panics sequentially too: an unwind, caught here, not an abort.
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parallel_case(4, budget().value, |p| WorkerOnly(p, true))
        }));
        assert!(unwound.is_err());
    }

    /// Review P2-2: SQLite's anonymous temporary files (here a sorter that spills past its
    /// 8 MiB list) are opened inside the scratch directory, not in SQLite's own temp dir.
    #[test]
    fn sqlite_temporary_files_live_in_the_scratch_directory() {
        let mut s = store();
        s.materialize().unwrap();
        s.connection
            .execute_batch(
                "CREATE TEMP TABLE spill(v BLOB);
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<200000)
                 INSERT INTO spill SELECT randomblob(100) FROM n;",
            )
            .unwrap();
        let mut query = s
            .connection
            .prepare("SELECT v FROM spill ORDER BY v")
            .unwrap();
        let mut rows = query.query([]).unwrap();
        assert!(rows.next().unwrap().is_some());
        let opened = s._vfs.as_ref().unwrap().opened();
        assert!(opened > 0, "the sort and the temporary table opened files");
        #[cfg(target_os = "linux")]
        {
            let dir = s._scratch.path().to_owned();
            let inside = std::fs::read_dir("/proc/self/fd")
                .unwrap()
                .filter_map(|fd| std::fs::read_link(fd.ok()?.path()).ok())
                .filter(|link| {
                    link.starts_with(&dir)
                        && link
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with("sqlite-"))
                })
                .count();
            assert!(
                inside > 0,
                "no SQLite temporary file is open in {}",
                dir.display()
            );
        }
        drop(rows);
        drop(query);
        drop(s);
    }

    /// One progress event: phase, done, total.
    type Seen = Vec<(String, u64, Option<u64>)>;
    /// Records every event; cancels its control at the `at`-th event.
    struct Events {
        at: usize,
        seen: std::sync::Mutex<Seen>,
        handle: std::sync::OnceLock<crate::CancellationHandle>,
    }
    impl crate::Observer for Events {
        fn observe(&self, event: &crate::RunEvent<'_>) -> Result<(), crate::JobError> {
            if let crate::RunEvent::Progress { phase, done, total } = event {
                let mut seen = self.seen.lock().unwrap();
                seen.push(((*phase).into(), *done, *total));
                if seen.len() == self.at {
                    self.handle.get().unwrap().cancel();
                }
            }
            Ok(())
        }
    }
    /// The `delivery_case` fixture's store, finished under a run control cancelled at the
    /// `at`-th event; its events and the work count (or the error).
    fn delivery_events(at: Option<usize>) -> (Seen, Result<u64, Error>) {
        let (a, b, c, d) = ([0., 0., 0.], [2., 0., 0.], [0., 2., 0.], [2., -2., 0.]);
        let payloads = [
            replay_fixture(&[[a, b, c]]),
            replay_fixture(&[[a, b, c]]),
            replay_fixture(&[[b, a, d]]),
        ];
        let mut s = store();
        s.decoded_limit = 1 << 24;
        for (id, parent) in [(0, None), (1, Some(0)), (2, Some(1)), (3, Some(0))] {
            s.tile(id, parent, IDENTITY).unwrap();
        }
        s.kind(0, None, false, 16., Some(1.)).unwrap();
        for (tile, rank) in [(1, 0), (2, 1), (3, 2)] {
            s.reference(tile, rank).unwrap();
            s.payload(rank, true, rank != 0).unwrap();
        }
        let mut read = |rank: u64| Ok(payloads[rank as usize].clone());
        let Some(at) = at else {
            return (Vec::new(), s.finish(budget(), 0, None, &mut read));
        };
        let observer = Arc::new(Events {
            at,
            seen: Default::default(),
            handle: Default::default(),
        });
        let control = crate::RunControl::new(Some(observer.clone()));
        observer.handle.set(control.cancellation_handle()).ok();
        let attempt = control.begin().map_err(|f| f.error).unwrap();
        let result = s.finish(budget(), 0, Some(attempt.probe()), &mut read);
        drop(attempt);
        let seen = observer.seen.lock().unwrap().clone();
        (seen, result)
    }
    /// #342: the delivery stage reports every pass of `validate.delivery` in order and each
    /// geometry replay with its placement total; counting the totals charges no work, and a
    /// cancellation requested at any event stops the stage there.
    #[test]
    fn delivery_progress_has_totals_and_every_event_is_a_checkpoint() {
        let (events, work) = delivery_events(Some(usize::MAX));
        let work = work.unwrap();
        let passes: Vec<u64> = events
            .iter()
            .filter(|e| e.0 == ticks::DELIVERY)
            .map(|e| e.1)
            .collect();
        assert_eq!(passes, (0..ticks::PASSES).collect::<Vec<_>>());
        assert!(events
            .iter()
            .filter(|e| e.0 == ticks::DELIVERY)
            .all(|e| e.2 == Some(ticks::PASSES)));
        let replay = |phase: &str| -> Vec<(u64, Option<u64>)> {
            events
                .iter()
                .filter(|e| e.0 == phase)
                .map(|e| (e.1, e.2))
                .collect()
        };
        // Two leaf placements and one coarse placement, each phase from 0 to its total.
        assert_eq!(
            replay("validate.delivery.leaves"),
            [(0, Some(2)), (2, Some(2))]
        );
        assert_eq!(
            replay("validate.delivery.coarse"),
            [(0, Some(1)), (1, Some(1))]
        );
        assert_eq!(replay("validate.delivery.edges"), [(0, Some(0))]);
        // The same work without a run control: counting the totals charges nothing.
        assert_eq!(delivery_events(None).1.unwrap(), work);
        for at in 1..=events.len() {
            let (seen, result) = delivery_events(Some(at));
            assert!(matches!(result, Err(Error::Cancelled)), "{at}: {result:?}");
            assert_eq!(seen.len(), at, "stopped at the event that cancelled");
        }
        assert!(work > 0);
    }
}
