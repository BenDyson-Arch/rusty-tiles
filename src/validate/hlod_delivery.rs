//! Native archive-only HLOD identity, continuity and exact delivery checks.
//! D1 interface keys are common-frame canonical f32 triples (+0); R9 original
//! aliases retain independent exact placed f64 keys. Collapsed canonical faces
//! contribute no interface items; three-distinct collinear faces still do.
//!
//! C1/identity admission runs once per payload. Bounded geometry replay emits
//! only compact point/edge keys and per-leaf face incidence into disk SQLite.
//! Ordered key scans find actual outside-leaf owners and stage sparse ancestor
//! obligations. No T contacts, distance joins or tolerant adjacency are inferred.
//! Every directed incident owner survives local balanced cancellation.
//! Original IDs recorded once at ingest are checked by one ordered key scan,
//! including disjoint placements and repeated references.
//! No triangle soup or archive-sized geometry/alias vector is retained.
//! Decoding, key work, scans, owner walks and queries consume the existing work
//! budget, including one unit per 10,000 SQLite VM operations. SQLite's page
//! cache is bounded; temporary sorts are disk backed and transient sort sizes
//! are outside disk_bytes(), which counts the monotonic main database only.
//!
//! D11 (`borders`): an ancestor may contract its shared leaf border to chords
//! between retained exact vertices, one witnessed chord per arc, each contracted
//! vertex within G/16 of its chord at its edge ordinal; junctions and point
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
mod drops;
use crate::spill::{Scratch, SlotFile};
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
    fn position(&self, i: usize, m: Matrix) -> Result<Point, Error> {
        if self.components != 3 {
            return Err(bad("interfaceSegments", "POSITION must be VEC3"));
        }
        let raw = [self.at(i, 0)?, self.at(i, 1)?, self.at(i, 2)?];
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

fn replay_document<'a>(
    bytes: &'a [u8],
    cap: usize,
    effort: &Effort,
) -> Result<(Value, &'a [u8], identity::Work), Error> {
    let length = u32::from_le_bytes(identity::slice(bytes, 12, 4)?.try_into().unwrap()) as usize;
    let mut work = identity::Work::new(
        effort.budget,
        effort
            .base
            .saturating_add(effort.used.load(Ordering::Relaxed)),
        cap,
    );
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
fn replay_surface(
    bytes: &[u8],
    frame: Matrix,
    cap: usize,
    effort: &Effort,
) -> Result<Surface, Error> {
    let (doc, bin, mut work) = replay_document(bytes, cap, effort)?;
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
                for corner in (0..count).step_by(3) {
                    let mut xyz = [[0.; 3]; 3];
                    for (j, p) in xyz.iter_mut().enumerate() {
                        let id = match &indices {
                            Some(i) => i.index(corner + j)?,
                            None => corner + j,
                        };
                        // Preserve scene-then-reference f64 evaluation order.
                        *p = point(frame, positions.position(id, m)?);
                        finite(p)?;
                    }
                    surface.triangle(xyz, &mut work, cap)?;
                }
            }
        }
        if let Some(children) = node["children"].as_array() {
            stack.push((children.as_slice(), 0, m));
        }
    }
    effort.charge(usize::try_from(work.used).map_err(|_| super::refused())?)?;
    Ok(surface)
}

struct Effort {
    budget: Budget,
    base: u64,
    used: Arc<AtomicU64>,
}
impl Effort {
    fn charge(&self, n: usize) -> Result<(), Error> {
        let old = self.used.fetch_add(n as u64, Ordering::Relaxed);
        self.budget
            .charge(self.base.saturating_add(old), n as u64, || {
                "interfaceSegments: native delivery work exceeds payload element budget".into()
            })
            .map(|_| ())
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
}
/// Buffered rows (tiles, references and payloads) before the store moves to disk.
const PENDING_ROWS: usize = 16_384;

/// One validation's disk hierarchy and decoded boundary index. Field order closes
/// SQLite before removing its private file and the existing scratch anchor.
pub(super) struct Store {
    connection: Db,
    _file: Option<tempfile::NamedTempFile>,
    _anchor: Option<SlotFile>,
    _scratch: Arc<Scratch>,
    pending: Pending,
    pub(super) checked: bool,
    common_root: Option<Matrix>,
    profile_exclusion: Option<&'static str>,
    decoded_limit: usize,
    census: Option<DecodedInterfaceCensus>,
}
impl Store {
    pub(super) fn new(scratch: Arc<Scratch>) -> Result<Self, Error> {
        Ok(Self {
            connection: Db(None),
            _file: None,
            _anchor: None,
            _scratch: scratch,
            pending: Pending::default(),
            checked: false,
            common_root: None,
            profile_exclusion: None,
            decoded_limit: 0,
            census: None,
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
        let connection = Connection::open(file.path()).map_err(sql)?;
        connection.execute_batch("PRAGMA cache_size=-8192; PRAGMA temp_store=FILE; PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;
            CREATE TABLE tiles(id INTEGER PRIMARY KEY,parent INTEGER,world BLOB NOT NULL,refine INTEGER,placeholder INTEGER NOT NULL DEFAULT 0,last INTEGER,leaf INTEGER,error REAL,diagonal REAL,cell INTEGER);
            CREATE INDEX tile_parent ON tiles(parent);
            CREATE TABLE refs(tile INTEGER NOT NULL,rank INTEGER NOT NULL); CREATE INDEX refs_tile ON refs(tile); CREATE INDEX refs_rank ON refs(rank);
            CREATE TABLE payloads(rank INTEGER PRIMARY KEY,identity_present INTEGER NOT NULL,triangle_present INTEGER NOT NULL,primitive_rows INTEGER NOT NULL DEFAULT 0,triangles INTEGER NOT NULL DEFAULT 0,digest BLOB NOT NULL,dropped INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE placements(id INTEGER PRIMARY KEY,tile INTEGER NOT NULL,rank INTEGER NOT NULL,frame BLOB NOT NULL,leaf INTEGER NOT NULL);
            CREATE INDEX placement_rank ON placements(rank,leaf,id);
            CREATE TABLE originals(source BLOB NOT NULL,vertex BLOB NOT NULL,rank INTEGER NOT NULL,point BLOB NOT NULL,PRIMARY KEY(source,vertex,rank)) WITHOUT ROWID;
            CREATE TABLE leaf_points(key BLOB NOT NULL,tile INTEGER NOT NULL,PRIMARY KEY(key,tile)) WITHOUT ROWID;
            CREATE TABLE leaf_segments(key BLOB NOT NULL,tile INTEGER NOT NULL,nf INTEGER NOT NULL,nr INTEGER NOT NULL,PRIMARY KEY(key,tile)) WITHOUT ROWID;
            CREATE TABLE shared_point_keys(key BLOB PRIMARY KEY) WITHOUT ROWID;
            CREATE TABLE requirements(tile INTEGER NOT NULL,kind INTEGER NOT NULL,key BLOB NOT NULL,directions INTEGER NOT NULL,PRIMARY KEY(tile,kind,key)) WITHOUT ROWID;
            CREATE TABLE leaf_faces(corners BLOB NOT NULL,tile INTEGER NOT NULL,n INTEGER NOT NULL,PRIMARY KEY(corners,tile)) WITHOUT ROWID;
            CREATE TABLE coarse_faces(tile INTEGER NOT NULL,corners BLOB NOT NULL);
            CREATE INDEX coarse_faces_tile ON coarse_faces(tile);
            CREATE TABLE multi_edges(key BLOB PRIMARY KEY) WITHOUT ROWID;
            CREATE TABLE edge_faces(key BLOB NOT NULL,tile INTEGER NOT NULL,third BLOB NOT NULL,dir INTEGER NOT NULL);
            CREATE INDEX edge_faces_key ON edge_faces(key,tile);
            CREATE TABLE twin_points(key BLOB NOT NULL,corners BLOB NOT NULL,PRIMARY KEY(key,corners)) WITHOUT ROWID;
            CREATE TABLE border_map(key BLOB NOT NULL,tile INTEGER NOT NULL,image BLOB NOT NULL,d REAL NOT NULL,PRIMARY KEY(key,tile)) WITHOUT ROWID;
            BEGIN;").map_err(sql)?;
        let pending = std::mem::take(&mut self.pending);
        for t in &pending.tiles {
            connection
                .execute(
                    "INSERT INTO tiles(id,parent,world,refine,placeholder,error,diagonal) VALUES(?,?,?,?,?,?,?)",
                    params![
                        integer(t.id)?,
                        t.parent.map(integer).transpose()?,
                        encode(&t.world),
                        t.refine,
                        t.placeholder,
                        t.error,
                        t.diagonal
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
                "UPDATE payloads SET triangles=? WHERE rank=?",
                params![integer(triangles)?, integer(rank)?],
            )
            .map_err(sql)?;
        if used.iter().any(|v| v.iter().any(|used| !*used)) {
            return Err(bad(
                "sourceIdentity",
                "original triangle table has unreferenced rows",
            ));
        }
        // One row per original vertex in the payload frame; placement is applied by
        // the alias scan, in the same scene-then-reference order a replay used.
        let mut insert = self
            .connection
            .prepare_cached("INSERT INTO originals VALUES(?,?,?,?)")
            .map_err(sql)?;
        for (&(source, vertex), p) in &originals {
            work.charge(1)?;
            let source: Vec<u8> = source.into_iter().flat_map(u64::to_be_bytes).collect();
            insert
                .execute(params![
                    source,
                    vertex.to_be_bytes().to_vec(),
                    integer(rank)?,
                    encode(p)
                ])
                .map_err(sql)?;
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

    pub(super) fn finish(
        &mut self,
        budget: Budget,
        already: u64,
        read: &mut dyn FnMut(u64) -> Result<Vec<u8>, Error>,
    ) -> Result<u64, Error> {
        // Without HLOD identity nothing below applies: no store, no scratch.
        if self.connection.0.is_none() {
            if !self.checked && !self.pending.payloads.iter().any(|p| p.1) {
                return Ok(0);
            }
            self.materialize()?;
        }
        let used = Arc::new(AtomicU64::new(0));
        let effort = Effort {
            budget,
            base: already,
            used: used.clone(),
        };
        let hook = used.clone();
        let remaining = budget.value.saturating_sub(already);
        self.connection
            .progress_handler(
                10_000,
                Some(move || hook.fetch_add(1, Ordering::Relaxed) >= remaining),
            )
            .map_err(sql)?;
        let result = self.finish_inner(&effort, read);
        self.connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(sql)?;
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
        read: &mut dyn FnMut(u64) -> Result<Vec<u8>, Error>,
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
        let coarse_triangle: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(
            SELECT 1 FROM refs r JOIN tiles t ON t.id=r.tile JOIN payloads p ON p.rank=r.rank
            WHERE p.triangle_present=1 AND EXISTS(
                SELECT 1 FROM refs d JOIN tiles l ON l.id=d.tile
                WHERE d.tile>t.id AND d.tile<=t.last AND l.leaf=1 AND d.rank<>r.rank))",
                [],
                |r| r.get(0),
            )
            .map_err(sql)?;
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
        let coarse_rows:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM refs r JOIN tiles t ON t.id=r.tile JOIN payloads p ON p.rank=r.rank WHERE p.primitive_rows>8192 AND EXISTS(SELECT 1 FROM refs d JOIN tiles l ON l.id=d.tile WHERE d.tile>t.id AND d.tile<=t.last AND l.leaf=1 AND d.rank<>r.rank))",[],|r|r.get(0)).map_err(sql)?;
        if coarse_rows {
            return Err(bad(
                "hlodFeatureIds",
                "D4 coarse source_primitive table exceeds 8192 rows",
            ));
        }
        let mut census = DecodedInterfaceCensus::default();
        self.triangle_cap(&mut census)?;
        census.omission = self.omission(effort)?;
        self.single_cover(effort)?;
        self.place_references(effort)?;
        self.replay_original_aliases(effort)?;
        self.collect_leaves(effort, read)?;
        self.connection
            .execute_batch(
                "INSERT INTO twin_points SELECT substr(corners,1+12*k.i,12),corners FROM
                 (SELECT corners FROM leaf_faces GROUP BY corners HAVING sum(n)>1),
                 (SELECT 0 AS i UNION ALL SELECT 1 UNION ALL SELECT 2) k WHERE 1 ON CONFLICT DO NOTHING",
            )
            .map_err(sql)?;
        self.edge_faces(effort, read)?;
        self.point_requirements(&mut census, effort)?;
        self.segment_requirements(&mut census, effort)?;
        self.check_coarse(&mut census, effort, read)?;
        self.coarse_drops(&mut census.dropped_by_depth, effort)?;
        self.border_pairs(effort)?;
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
    /// Every original vertex shared by leaf placements, including repeated
    /// references to one payload and disjoint geometry, must have one exact placed
    /// f64 position (normalized zero). One ordered scan over (source, vertex) rows
    /// recorded at ingest: each payload is decoded once, never per pair.
    fn replay_original_aliases(&self, effort: &Effort) -> Result<(), Error> {
        let mut query = self
            .connection
            .prepare("SELECT o.source,o.vertex,o.point,p.frame FROM originals o JOIN placements p ON p.rank=o.rank WHERE p.leaf=1 ORDER BY o.source,o.vertex")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut group: Option<(Vec<u8>, Vec<u8>, [u64; 3])> = None;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let source: Vec<u8> = row.get(0).map_err(sql)?;
            let vertex: Vec<u8> = row.get(1).map_err(sql)?;
            let local = decode::<3>(&row.get::<_, Vec<u8>>(2).map_err(sql)?)?;
            let frame = decode::<16>(&row.get::<_, Vec<u8>>(3).map_err(sql)?)?;
            let placed = point(frame, local);
            finite(&placed)?;
            let exact = point_key(placed);
            match &group {
                Some((s, v, first)) if *s == source && *v == vertex => {
                    if *first != exact {
                        return Err(bad(
                            "sourceIdentity",
                            "original vertex has distinct exact decoded placed positions",
                        ));
                    }
                }
                _ => group = Some((source, vertex, exact)),
            }
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
            effort.charge(1)?;
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
        Ok(())
    }
    fn surface(
        &self,
        rank: i64,
        frame: &[u8],
        effort: &Effort,
        read: &mut dyn FnMut(u64) -> Result<Vec<u8>, Error>,
    ) -> Result<Surface, Error> {
        effort.charge(1)?;
        let bytes = read(u64::try_from(rank).map_err(|_| super::refused())?)?;
        replay_surface(&bytes, decode::<16>(frame)?, self.decoded_limit, effort)
    }
    fn record_leaf(&self, tile: i64, surface: &Surface, effort: &Effort) -> Result<(), Error> {
        let mut point = self
            .connection
            .prepare_cached("INSERT OR IGNORE INTO leaf_points VALUES(?,?)")
            .map_err(sql)?;
        let mut edge=self.connection.prepare_cached("INSERT INTO leaf_segments VALUES(?,?,?,?) ON CONFLICT(key,tile) DO UPDATE SET nf=leaf_segments.nf+excluded.nf,nr=leaf_segments.nr+excluded.nr WHERE leaf_segments.nf<=9223372036854775807-excluded.nf AND leaf_segments.nr<=9223372036854775807-excluded.nr").map_err(sql)?;
        for p in &surface.points {
            effort.charge(1)?;
            point.execute(params![point_bytes(*p), tile]).map_err(sql)?;
        }
        for (key, e) in &surface.edges {
            effort.charge(1)?;
            if edge
                .execute(params![
                    edge_bytes(*key),
                    tile,
                    integer(e.forward)?,
                    integer(e.reverse)?
                ])
                .map_err(sql)?
                == 0
            {
                return Err(Error::limit(
                    "interfaceSegments: per-leaf face incidence exceeds disk counter range",
                ));
            }
        }
        Ok(())
    }
    fn collect_leaves(
        &self,
        effort: &Effort,
        read: &mut dyn FnMut(u64) -> Result<Vec<u8>, Error>,
    ) -> Result<(), Error> {
        let mut query = self
            .connection
            .prepare("SELECT tile,rank,frame FROM placements WHERE leaf=1 ORDER BY id")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let surface = self.surface(
                row.get(1).map_err(sql)?,
                &row.get::<_, Vec<u8>>(2).map_err(sql)?,
                effort,
                read,
            )?;
            let tile: i64 = row.get(0).map_err(sql)?;
            self.record_leaf(tile, &surface, effort)?;
            // Every leaf face by corner set, so coincident source groups are found
            // across leaves too (they license coincident coarse faces above).
            let mut face = self
                .connection
                .prepare_cached("INSERT INTO leaf_faces VALUES(?,?,1) ON CONFLICT(corners,tile) DO UPDATE SET n=n+1")
                .map_err(sql)?;
            for f in &surface.faces {
                effort.charge(1)?;
                let mut k = *f;
                k.sort_unstable();
                let bytes: Vec<u8> = k.iter().flat_map(|p| point_bytes(*p)).collect();
                face.execute(params![bytes, tile]).map_err(sql)?;
            }
        }
        Ok(())
    }
    /// Leaf faces on every edge some direction of which carries two or more
    /// descendant faces: their third corners and directions, so coarse multiplicity
    /// is matched to source faces on its own side rather than counted. A second,
    /// bounded replay of only the leaves holding such edges.
    fn edge_faces(
        &self,
        effort: &Effort,
        read: &mut dyn FnMut(u64) -> Result<Vec<u8>, Error>,
    ) -> Result<(), Error> {
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
        let mut query = self
            .connection
            .prepare(
                "SELECT p.tile,p.rank,p.frame FROM placements p WHERE p.leaf=1 AND p.tile IN (SELECT tile FROM replay_tiles) ORDER BY p.id",
            )
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut multi = self
            .connection
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM multi_edges WHERE key=?)")
            .map_err(sql)?;
        let mut insert = self
            .connection
            .prepare_cached("INSERT INTO edge_faces VALUES(?,?,?,?)")
            .map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let tile: i64 = row.get(0).map_err(sql)?;
            let surface = self.surface(
                row.get(1).map_err(sql)?,
                &row.get::<_, Vec<u8>>(2).map_err(sql)?,
                effort,
                read,
            )?;
            for f in &surface.faces {
                for j in 0..3 {
                    effort.charge(2)?;
                    let (x, y, c) = (f[j], f[(j + 1) % 3], f[(j + 2) % 3]);
                    let key = edge_bytes(if x < y { (x, y) } else { (y, x) });
                    if multi
                        .query_row([&key], |r| r.get::<_, bool>(0))
                        .map_err(sql)?
                    {
                        insert
                            .execute(params![key, tile, point_bytes(c), i64::from(x > y)])
                            .map_err(sql)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn require_ancestors(
        &self,
        mut tile: i64,
        owners: (i64, i64),
        kind: i64,
        key: &[u8],
        directions: i64,
        effort: &Effort,
    ) -> Result<(), Error> {
        let owner = tile;
        let (first, last) = owners;
        let mut lookup=self.connection.prepare_cached("SELECT parent,last,EXISTS(SELECT 1 FROM refs r WHERE r.tile=tiles.id) FROM tiles WHERE id=?").map_err(sql)?;
        loop {
            effort.charge(1)?;
            let (parent, end, content): (Option<i64>, i64, bool) = lookup
                .query_row([tile], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(sql)?;
            if first >= tile && last <= end {
                break;
            }
            if tile != owner && content {
                self.connection.execute("INSERT INTO requirements VALUES(?,?,?,?) ON CONFLICT(tile,kind,key) DO UPDATE SET directions=requirements.directions|excluded.directions",params![tile,kind,key,directions]).map_err(sql)?;
            }
            let Some(parent) = parent else {
                break;
            };
            tile = parent;
        }
        Ok(())
    }
    fn shared_point(
        &self,
        key: &[u8],
        first: i64,
        last: i64,
        census: &mut DecodedInterfaceCensus,
        effort: &Effort,
    ) -> Result<(), Error> {
        if first == last {
            return Ok(());
        }
        effort.charge(1)?;
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
            self.require_ancestors(row.get(0).map_err(sql)?, (first, last), 0, key, 0, effort)?;
        }
        Ok(())
    }
    fn point_requirements(
        &self,
        census: &mut DecodedInterfaceCensus,
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
                self.shared_point(&k, first, last, census, effort)?;
            }
            group = Some((key, tile, tile));
        }
        if let Some((key, first, last)) = group {
            self.shared_point(&key, first, last, census, effort)?;
        }
        Ok(())
    }
    fn segment_group(
        &self,
        key: &[u8],
        first: i64,
        last: i64,
        faces: u64,
        census: &mut DecodedInterfaceCensus,
        effort: &Effort,
    ) -> Result<(), Error> {
        effort.charge(1)?;
        if faces == 1 {
            add_count(&mut census.source_open_edges, 1)?;
            // Both endpoints must also occur in another leaf. This is an exact
            // key join, not a chord inference or a distance/near-seam query.
            read_edge(key)?;
            effort.charge(2)?;
            let shared:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM shared_point_keys WHERE key=?1) AND EXISTS(SELECT 1 FROM shared_point_keys WHERE key=?2)",params![&key[..12],&key[12..]],|r|r.get(0)).map_err(sql)?;
            if shared {
                add_count(&mut census.source_open_edges_with_shared_endpoints, 1)?;
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
                effort,
            )?;
        }
        Ok(())
    }
    fn segment_requirements(
        &self,
        census: &mut DecodedInterfaceCensus,
        effort: &Effort,
    ) -> Result<(), Error> {
        let mut query = self
            .connection
            .prepare("SELECT key,tile,nf,nr FROM leaf_segments ORDER BY key,tile")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
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
                self.segment_group(&k, first, last, n, census, effort)?;
            }
            group = Some((key, tile, tile, faces));
        }
        if let Some((key, first, last, n)) = group {
            self.segment_group(&key, first, last, n, census, effort)?;
        }
        Ok(())
    }
    /// Every coarse tile's decoded surface (all its placements) against its D11
    /// border: exact junctions, one witnessed chord per shared arc, displacement.
    fn check_coarse(
        &self,
        census: &mut DecodedInterfaceCensus,
        effort: &Effort,
        read: &mut dyn FnMut(u64) -> Result<Vec<u8>, Error>,
    ) -> Result<(), Error> {
        let mut query = self
            .connection
            .prepare("SELECT tile,rank,frame FROM placements WHERE leaf=0 ORDER BY tile,id")
            .map_err(sql)?;
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
                        effort.charge(1)?;
                        let bytes: Vec<u8> = f.iter().flat_map(|p| point_bytes(*p)).collect();
                        face.execute(params![tile, bytes]).map_err(sql)?;
                    }
                    let contracted = self.border(tile, &surface, effort)?;
                    self.equal_directions(tile, &surface, &contracted, effort)?;
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
            effort.charge(1)?;
            let surface = self.surface(
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
            budget: budget(),
            base: 0,
            used: Arc::new(AtomicU64::new(0)),
        }
    }
    pub(super) fn store() -> Store {
        let scratch = Scratch::create(None, 4096).unwrap();
        Store::new(Arc::new(scratch)).unwrap()
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
            let result = s.finish(budget(), 0, &mut |_| panic!("unexpected replay"));
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
                .finish(budget(), 0, &mut |_| panic!("unexpected replay"))
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
            .finish(budget(), 0, &mut |_| panic!("unexpected replay"))
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
            let result = s.finish(budget(), 0, &mut |_| Ok(replay_fixture(&[])));
            if same {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains("D4 coarse"));
            }
        }
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
            .finish(budget(), 0, &mut |_| panic!("unexpected replay"))
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
            let result = s.finish(budget(), 0, &mut |_| panic!("unexpected replay"));
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
            for (id, &(rank, x, shift)) in items.iter().enumerate() {
                s.connection
                    .execute(
                        "INSERT OR IGNORE INTO originals VALUES(?,?,?,?)",
                        params![
                            vec![0u8; 40],
                            0u64.to_be_bytes().to_vec(),
                            rank,
                            encode(&[x, 1., 2.])
                        ],
                    )
                    .map_err(sql)?;
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
            s.connection
                .execute(
                    "INSERT INTO originals VALUES(?,?,?,?)",
                    params![
                        vec![0u8; 40],
                        vertex.to_be_bytes().to_vec(),
                        rank as i64,
                        encode(&p)
                    ],
                )
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
    #[test]
    fn original_scan_charges_every_row() {
        let rows: Vec<(u64, Point)> = (0..4).map(|v| (v, [v as f64, 0., 0.])).collect();
        let s = original_rows(&rows);
        let mut work = effort();
        work.budget.value = 3;
        assert!(s
            .replay_original_aliases(&work)
            .unwrap_err()
            .exceeded()
            .is_some());
        s.replay_original_aliases(&effort()).unwrap();
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
        s.finish(budget(), 0, &mut |rank| Ok(payloads[rank as usize].clone()))?;
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
        let s = original_rows(&[(0, a), (0, b)]);
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
        let surface = replay_surface(&replay_fixture(&face), IDENTITY, 1 << 20, &effort()).unwrap();
        s.record_leaf(7, &surface, &effort()).unwrap();
        s.record_leaf(7, &surface, &effort()).unwrap();
        let mut census = DecodedInterfaceCensus::default();
        s.point_requirements(&mut census, &effort()).unwrap();
        s.segment_requirements(&mut census, &effort()).unwrap();
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
            .finish(budget(), 0, &mut |rank| Ok(payloads[rank as usize].clone()))
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
        s.finish(budget(), 0, &mut |rank| Ok(payloads[rank as usize].clone()))
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
            let result = s.finish(budget(), 0, &mut |_| Ok(replay_fixture(&[])));
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
            let result = s.finish(budget(), 0, &mut |_| Ok(replay_fixture(&[])));
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
        s.finish(budget(), 0, &mut |_| Ok(replay_fixture(&[])))?;
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
        let surface = replay_surface(&replay_fixture(&face), frame, 1 << 20, &effort()).unwrap();
        assert!(surface.points.contains(&canonical([0., 0., 0.]).unwrap()));
        assert!(!surface
            .points
            .contains(&canonical(point(local, [0., 10., 0.])).unwrap()));
    }

    #[test]
    fn replay_charges_json_and_decode_work_before_reduction() {
        let bytes = replay_fixture(&[[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]]);
        let mut e = effort();
        e.budget.value = 1;
        assert!(replay_surface(&bytes, IDENTITY, 1 << 20, &e)
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
            .finish(budget(), 0, &mut |_| panic!("unexpected replay"))
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
                &mut |_| panic!("unexpected replay"),
            )
            .unwrap_err();
        assert!(error.exceeded().is_some());
    }
}
