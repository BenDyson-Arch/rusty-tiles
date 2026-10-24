//! Native archive-only HLOD identity, continuity and exact delivery checks.
//! D1 interface keys are common-frame canonical f32 triples (+0); R9 original
//! aliases retain independent exact placed f64 keys. Collapsed canonical faces
//! contribute no interface items; three-distinct collinear faces still do.
//!
//! C1/identity admission runs once per payload. Bounded geometry replay emits
//! only compact point/edge keys and per-leaf face incidence into disk SQLite.
//! Ordered key scans find actual outside-leaf owners and stage sparse ancestor
//! obligations. No chords, T contacts, distance joins or tolerant adjacency are
//! inferred. Every directed incident owner survives local balanced cancellation.
//! Original-ID interval joins independently replay even disjoint placements.
//! No triangle soup or archive-sized geometry/alias vector is retained.
//! Decoding, key work, scans, owner walks and queries consume the existing work
//! budget, including one unit per 10,000 SQLite VM operations. SQLite's page
//! cache is bounded; temporary sorts are disk backed and transient sort sizes
//! are outside disk_bytes(), which counts the monotonic main database only.
//!
//! Census basis is decoded leaf canonical keys, not unavailable original source
//! coordinates. Fresh coarse UV charts, source distance and coarse orientation
//! outside required interfaces/local contradictory pairs remain uninspected.
use super::{identity, invalid, spill_failure, Budget, Error, ScratchRefusal};
use crate::spill::{Scratch, SlotFile};
use rusqlite::{params, Connection};
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
type OriginalMap = BTreeMap<(Source, u64), [u64; 3]>;

/// The range index is conservative: every exact original alias in a leaf is
/// replayed, including aliases absent from the geometric boundary reduction.
fn replay_originals(
    bytes: &[u8],
    frame: Matrix,
    cap: usize,
    effort: &Effort,
) -> Result<OriginalMap, Error> {
    let (doc, bin, mut work) = replay_document(bytes, cap, effort)?;
    let nodes = doc["nodes"]
        .as_array()
        .ok_or_else(|| bad("sourceIdentity", "missing replay nodes"))?;
    let scene = doc["scene"].as_u64().unwrap_or(0) as usize;
    let roots = doc["scenes"][scene]["nodes"]
        .as_array()
        .ok_or_else(|| bad("sourceIdentity", "missing replay scene"))?;
    let mut originals = OriginalMap::new();
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
            .ok_or_else(|| bad("sourceIdentity", "replay node outside document"))?;
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
                let triangle = Feature::new(&doc, bin, p, "source_triangle", &mut work)?
                    .ok_or_else(|| bad("sourceIdentity", "missing leaf original rows in replay"))?;
                let count = indices.as_ref().map_or(positions.count, |i| i.count);
                for corner in (0..count).step_by(3) {
                    work.charge(12)?;
                    let first = match &indices {
                        Some(i) => i.index(corner)?,
                        None => corner,
                    };
                    let (row, source) = triangle.row(first, &mut work)?;
                    for j in 0..3 {
                        work.charge(originals.len().max(2).ilog2() as usize + 1)?;
                        let id = match &indices {
                            Some(i) => i.index(corner + j)?,
                            None => corner + j,
                        };
                        let p = positions.position(id, m)?;
                        // Match ingest scene evaluation followed by reference
                        // placement; folding matrices changes exact f64 results.
                        let p = point(frame, p);
                        finite(&p)?;
                        let exact = point_key(p);
                        let key = (source, triangle.columns[6 + j].at(row)?);
                        if let Some(previous) = originals.get(&key) {
                            if *previous != exact {
                                return Err(bad(
                                    "sourceIdentity",
                                    "original vertex has distinct exact decoded placed positions",
                                ));
                            }
                        } else {
                            if originals.len().saturating_mul(144) >= cap {
                                return Err(Error::limit("sourceIdentity: replay original workspace exceeds decoded document budget"));
                            }
                            originals.insert(key, exact);
                        }
                    }
                }
            }
        }
        if let Some(children) = node["children"].as_array() {
            stack.push((children.as_slice(), 0, m));
        }
    }
    effort.charge(usize::try_from(work.used).map_err(|_| super::refused())?)?;
    Ok(originals)
}
fn compare_originals(
    left: &OriginalMap,
    right: &OriginalMap,
    effort: &Effort,
) -> Result<(), Error> {
    let (mut l, mut r) = (left.iter(), right.iter());
    let (mut a, mut b) = (l.next(), r.next());
    while let (Some((ka, pa)), Some((kb, pb))) = (a, b) {
        effort.charge(1)?;
        match ka.cmp(kb) {
            std::cmp::Ordering::Less => a = l.next(),
            std::cmp::Ordering::Greater => b = r.next(),
            std::cmp::Ordering::Equal => {
                if pa != pb {
                    return Err(bad(
                        "sourceIdentity",
                        "same original vertex has distinct exact decoded placed positions",
                    ));
                }
                a = l.next();
                b = r.next();
            }
        }
    }
    Ok(())
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
    triangles: u64,
}
impl Surface {
    fn reserve(&self, bytes: usize, cap: usize) -> Result<(), Error> {
        if self
            .points
            .len()
            .saturating_mul(64)
            .saturating_add(self.edges.len().saturating_mul(128))
            .saturating_add(bytes)
            > cap
        {
            return Err(Error::limit(
                "interfaceSegments: canonical replay workspace exceeds decoded document budget",
            ));
        }
        Ok(())
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
/// cancelled. Above-cap counts are decoded nonleaf content references.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct DecodedInterfaceCensus {
    pub(super) shared_points: u64,
    pub(super) shared_segments: u64,
    pub(super) source_open_edges: u64,
    pub(super) source_open_edges_with_shared_endpoints: u64,
    pub(super) stopped_above_cap: u64,
    pub(super) max_above_cap_triangles: u64,
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

/// One validation's disk hierarchy and decoded boundary index. Field order closes
/// SQLite before removing its private file and the existing scratch anchor.
pub(super) struct Store {
    connection: Connection,
    _file: tempfile::NamedTempFile,
    _anchor: SlotFile,
    _scratch: Arc<Scratch>,
    pub(super) checked: bool,
    common_root: Option<Matrix>,
    profile_exclusion: Option<&'static str>,
    decoded_limit: usize,
    census: Option<DecodedInterfaceCensus>,
}
impl Store {
    pub(super) fn new(scratch: Arc<Scratch>) -> Result<Self, Error> {
        let anchor = SlotFile::new(&scratch, "hlod-delivery", 1).map_err(spill_failure)?;
        let file = tempfile::Builder::new()
            .prefix("hlod-delivery-")
            .tempfile_in(scratch.path())
            .map_err(|e| Error::Scratch(ScratchRefusal::Io(e)))?;
        let connection = Connection::open(file.path()).map_err(sql)?;
        connection.execute_batch("PRAGMA cache_size=-8192; PRAGMA temp_store=FILE; PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;
            CREATE TABLE tiles(id INTEGER PRIMARY KEY,parent INTEGER,world BLOB NOT NULL,refine INTEGER,placeholder INTEGER NOT NULL DEFAULT 0,last INTEGER,leaf INTEGER);
            CREATE INDEX tile_parent ON tiles(parent);
            CREATE TABLE refs(tile INTEGER NOT NULL,rank INTEGER NOT NULL); CREATE INDEX refs_tile ON refs(tile); CREATE INDEX refs_rank ON refs(rank);
            CREATE TABLE payloads(rank INTEGER PRIMARY KEY,identity_present INTEGER NOT NULL,triangle_present INTEGER NOT NULL,primitive_rows INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE placements(id INTEGER PRIMARY KEY,tile INTEGER NOT NULL,rank INTEGER NOT NULL,frame BLOB NOT NULL,leaf INTEGER NOT NULL);
            CREATE INDEX placement_rank ON placements(rank,leaf,id);
            CREATE TABLE original_domains(rank INTEGER NOT NULL,source BLOB NOT NULL,lo BLOB NOT NULL,hi BLOB NOT NULL,PRIMARY KEY(rank,source));
            CREATE INDEX original_domain_source ON original_domains(source,lo,rank,hi);
            CREATE TABLE leaf_points(key BLOB NOT NULL,tile INTEGER NOT NULL,PRIMARY KEY(key,tile)) WITHOUT ROWID;
            CREATE TABLE leaf_segments(key BLOB NOT NULL,tile INTEGER NOT NULL,nf INTEGER NOT NULL,nr INTEGER NOT NULL,PRIMARY KEY(key,tile)) WITHOUT ROWID;
            CREATE TABLE shared_point_keys(key BLOB PRIMARY KEY) WITHOUT ROWID;
            CREATE TABLE requirements(tile INTEGER NOT NULL,kind INTEGER NOT NULL,key BLOB NOT NULL,directions INTEGER NOT NULL,PRIMARY KEY(tile,kind,key)) WITHOUT ROWID;
            CREATE TABLE fulfilled(tile INTEGER NOT NULL,kind INTEGER NOT NULL,key BLOB NOT NULL,directions INTEGER NOT NULL,PRIMARY KEY(tile,kind,key)) WITHOUT ROWID;
            BEGIN;").map_err(sql)?;
        Ok(Self {
            connection,
            _file: file,
            _anchor: anchor,
            _scratch: scratch,
            checked: false,
            common_root: None,
            profile_exclusion: None,
            decoded_limit: 0,
            census: None,
        })
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
            let bytes: Vec<u8> = self
                .connection
                .query_row("SELECT world FROM tiles WHERE id=?", [integer(p)?], |r| {
                    r.get(0)
                })
                .map_err(sql)?;
            multiply(decode::<16>(&bytes)?, local)
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
    ) -> Result<(), Error> {
        self.connection
            .execute(
                "UPDATE tiles SET refine=?,placeholder=? WHERE id=?",
                params![refine, placeholder, integer(id)?],
            )
            .map_err(sql)?;
        Ok(())
    }
    pub(super) fn reference(&mut self, id: u64, rank: u64) -> Result<(), Error> {
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
                    self.payload(rank, false, false)?;
                    return Ok(0);
                }
            }
        } else {
            self.payload(rank, false, false)?;
            return Ok(0);
        };
        if !identity::declares(raw) {
            self.payload(rank, false, false)?;
            return Ok(0);
        }
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
        self.payload(rank, true, has_triangles)?;
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
        if used.iter().any(|v| v.iter().any(|used| !*used)) {
            return Err(bad(
                "sourceIdentity",
                "original triangle table has unreferenced rows",
            ));
        }
        let mut domain: Option<(Source, u64, u64)> = None;
        let mut insert_domain = self
            .connection
            .prepare_cached("INSERT INTO original_domains VALUES(?,?,?,?)")
            .map_err(sql)?;
        let flush = |d: (Source, u64, u64),
                     stmt: &mut rusqlite::CachedStatement<'_>|
         -> Result<(), Error> {
            let source: Vec<u8> = d.0.into_iter().flat_map(u64::to_be_bytes).collect();
            stmt.execute(params![
                integer(rank)?,
                source,
                d.1.to_be_bytes().to_vec(),
                d.2.to_be_bytes().to_vec()
            ])
            .map_err(sql)?;
            Ok(())
        };
        for &(source, vertex) in originals.keys() {
            work.charge(1)?;
            if let Some(d) = &mut domain {
                if d.0 == source {
                    d.2 = vertex;
                    continue;
                }
                flush(*d, &mut insert_domain)?;
            }
            domain = Some((source, vertex, vertex));
        }
        if let Some(d) = domain {
            flush(d, &mut insert_domain)?;
        }
        Ok(work.used)
    }
    fn payload(&self, rank: u64, identity: bool, triangle: bool) -> Result<(), Error> {
        self.connection
            .execute(
                "INSERT INTO payloads(rank,identity_present,triangle_present) VALUES(?,?,?)",
                params![integer(rank)?, identity, triangle],
            )
            .map_err(sql)?;
        Ok(())
    }
    /// Monotonic main-database length, for the report after successful finish.
    /// Add to managed scratch peak for a conservative combined peak. SQLite's
    /// transient sort files are outside this count; its page cache is separately capped.
    pub(super) fn disk_bytes(&self) -> u64 {
        self._file
            .as_file()
            .metadata()
            .map_or(0, |metadata| metadata.len())
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
        self.hierarchy(effort)?;
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
        self.place_references(effort)?;
        self.replay_original_aliases(effort, read)?;
        let mut census = DecodedInterfaceCensus::default();
        self.collect_leaves(effort, read)?;
        self.point_requirements(&mut census, effort)?;
        self.segment_requirements(&mut census, effort)?;
        self.check_coarse(&mut census, effort, read)?;
        Ok(Some(census))
    }
    fn hierarchy(&self, effort: &Effort) -> Result<(), Error> {
        struct Frame {
            id: i64,
            replace: bool,
            renderable: bool,
            barrier: bool,
        }
        let mut stack = Vec::<Frame>::new();
        let mut previous = -1i64;
        let mut query=self.connection.prepare("SELECT t.id,t.parent,t.refine,t.placeholder,EXISTS(SELECT 1 FROM refs r WHERE r.tile=t.id) FROM tiles t ORDER BY t.id").map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut end = self
            .connection
            .prepare_cached("UPDATE tiles SET last=? WHERE id=?")
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
            stack.push(Frame {
                id,
                replace,
                renderable: if content { replace } else { renderable },
                barrier: !content && (barrier || (!placeholder && renderable)),
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
    fn replay_original_aliases(
        &self,
        effort: &Effort,
        read: &mut dyn FnMut(u64) -> Result<Vec<u8>, Error>,
    ) -> Result<(), Error> {
        let mut outer = self
            .connection
            .prepare("SELECT id,rank,frame FROM placements WHERE leaf=1 ORDER BY id")
            .map_err(sql)?;
        let mut rows = outer.query([]).map_err(sql)?;
        let mut domains = self
            .connection
            .prepare("SELECT source,lo,hi FROM original_domains WHERE rank=?")
            .map_err(sql)?;
        // Source/lo and rank/leaf/id are indexed. Enumerate and charge every
        // returned range candidate before testing its upper bound or aliases.
        // A disjoint geometry box never excludes an identity candidate.
        let mut candidates = self.connection.prepare("SELECT p.id,p.rank,p.frame,d.hi FROM original_domains d INDEXED BY original_domain_source CROSS JOIN placements p INDEXED BY placement_rank ON p.rank=d.rank WHERE d.source=?1 AND d.lo<=?2 AND p.leaf=1 AND p.id>?3").map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let id: i64 = row.get(0).map_err(sql)?;
            let rank: i64 = row.get(1).map_err(sql)?;
            let frame: Vec<u8> = row.get(2).map_err(sql)?;
            let mut local = domains.query([rank]).map_err(sql)?;
            let mut seen = BTreeSet::new();
            let mut left = None;
            while let Some(domain) = local.next().map_err(sql)? {
                effort.charge(1)?;
                let source: Vec<u8> = domain.get(0).map_err(sql)?;
                let lo: Vec<u8> = domain.get(1).map_err(sql)?;
                let hi: Vec<u8> = domain.get(2).map_err(sql)?;
                let mut nearby = candidates.query(params![source, hi, id]).map_err(sql)?;
                while let Some(other) = nearby.next().map_err(sql)? {
                    effort.charge(1)?;
                    let high: Vec<u8> = other.get(3).map_err(sql)?;
                    if high < lo {
                        continue;
                    }
                    let other_id: i64 = other.get(0).map_err(sql)?;
                    effort.charge(seen.len().max(2).ilog2() as usize + 1)?;
                    if seen.contains(&other_id) {
                        continue;
                    }
                    if seen.len().saturating_mul(48) >= self.decoded_limit {
                        return Err(Error::limit(
                            "sourceIdentity: replay pair workspace exceeds decoded document budget",
                        ));
                    }
                    seen.insert(other_id);
                    if left.is_none() {
                        effort.charge(1)?;
                        let bytes = read(u64::try_from(rank).map_err(|_| super::refused())?)?;
                        left = Some(replay_originals(
                            &bytes,
                            decode::<16>(&frame)?,
                            self.decoded_limit,
                            effort,
                        )?);
                    }
                    let other_rank: i64 = other.get(1).map_err(sql)?;
                    let other_frame: Vec<u8> = other.get(2).map_err(sql)?;
                    effort.charge(1)?;
                    let bytes = read(u64::try_from(other_rank).map_err(|_| super::refused())?)?;
                    let right = replay_originals(
                        &bytes,
                        decode::<16>(&other_frame)?,
                        self.decoded_limit,
                        effort,
                    )?;
                    compare_originals(left.as_ref().unwrap(), &right, effort)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn census(&self) -> Option<DecodedInterfaceCensus> {
        self.census
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
            self.record_leaf(row.get(0).map_err(sql)?, &surface, effort)?;
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
    fn match_surface(&self, tile: i64, surface: &Surface, effort: &Effort) -> Result<(), Error> {
        let mut query = self
            .connection
            .prepare("SELECT kind,key FROM requirements WHERE tile=?")
            .map_err(sql)?;
        let mut rows = query.query([tile]).map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            let kind: i64 = row.get(0).map_err(sql)?;
            let key: Vec<u8> = row.get(1).map_err(sql)?;
            let size = if kind == 0 {
                surface.points.len()
            } else {
                surface.edges.len()
            };
            effort.charge(size.max(2).ilog2() as usize + 1)?;
            let directions = if kind == 0 {
                surface.points.contains(&read_point(&key)?).then_some(0)
            } else {
                surface.edges.get(&read_edge(&key)?).map(|e| e.directions())
            };
            if let Some(directions) = directions {
                self.connection.execute("INSERT INTO fulfilled VALUES(?,?,?,?) ON CONFLICT(tile,kind,key) DO UPDATE SET directions=fulfilled.directions|excluded.directions",params![tile,kind,key,directions]).map_err(sql)?;
            }
        }
        Ok(())
    }
    fn check_coarse(
        &self,
        census: &mut DecodedInterfaceCensus,
        effort: &Effort,
        read: &mut dyn FnMut(u64) -> Result<Vec<u8>, Error>,
    ) -> Result<(), Error> {
        let mut query=self.connection.prepare("SELECT p.tile,p.rank,p.frame,g.triangle_present FROM placements p JOIN payloads g ON g.rank=p.rank WHERE p.leaf=0 ORDER BY p.id").map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let tile: i64 = row.get(0).map_err(sql)?;
            let surface = self.surface(
                row.get(1).map_err(sql)?,
                &row.get::<_, Vec<u8>>(2).map_err(sql)?,
                effort,
                read,
            )?;
            if surface.triangles > 8192 {
                add_count(&mut census.stopped_above_cap, 1)?;
                census.max_above_cap_triangles =
                    census.max_above_cap_triangles.max(surface.triangles);
            }
            if !row.get::<_, bool>(3).map_err(sql)? {
                for e in surface.edges.values() {
                    effort.charge(1)?;
                    if (e.forward == 2 && e.reverse == 0) || (e.reverse == 2 && e.forward == 0) {
                        return Err(bad(
                            "interfaceWinding",
                            "coarse shared edge has equal directed incidences",
                        ));
                    }
                }
            }
            self.match_surface(tile, &surface, effort)?;
        }
        let mut query=self.connection.prepare("SELECT r.kind,r.directions,f.directions,r.tile FROM requirements r LEFT JOIN fulfilled f USING(tile,kind,key) ORDER BY r.kind,r.tile,r.key").map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let kind: i64 = row.get(0).map_err(sql)?;
            let allowed: i64 = row.get(1).map_err(sql)?;
            let actual: Option<i64> = row.get(2).map_err(sql)?;
            // D1 preserves the unordered edge, not source face multiplicity.
            // Every actual coarse direction needs a descendant source witness;
            // ordinary simplification may remove other incident source faces.
            if actual.is_some_and(|mask| (kind == 0 || mask != 0) && mask & !allowed == 0) {
                continue;
            }
            let tile: i64 = row.get(3).map_err(sql)?;
            let (label, detail) = if kind == 0 {
                (
                    "interfacePoints",
                    "decoded ancestor lost an exact shared leaf point",
                )
            } else if actual.is_some() {
                (
                    "interfaceWinding",
                    "decoded ancestor reverses a required directed leaf edge",
                )
            } else {
                (
                    "interfaceSegments",
                    "decoded ancestor lacks an exact shared leaf edge",
                )
            };
            return Err(bad(label, &format!("{detail}: tile={tile}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> Budget {
        Budget {
            key: super::super::LimitKey::TotalPayloadElements,
            value: 10_000_000,
            source: super::super::limits::Derivation::Explicit,
        }
    }
    fn effort() -> Effort {
        Effort {
            budget: budget(),
            base: 0,
            used: Arc::new(AtomicU64::new(0)),
        }
    }
    fn store() -> Store {
        let scratch = Scratch::create(None, 4096).unwrap();
        Store::new(Arc::new(scratch)).unwrap()
    }
    #[test]
    fn continuity_runs_without_identity_and_respects_link_placeholders() {
        for placeholder in [false, true] {
            let mut s = store();
            s.tile(0, None, IDENTITY).unwrap();
            s.tile(1, Some(0), IDENTITY).unwrap();
            s.tile(2, Some(1), IDENTITY).unwrap();
            s.kind(1, None, placeholder).unwrap();
            s.reference(0, 0).unwrap();
            s.reference(2, 2).unwrap();
            s.payload(0, false, false).unwrap();
            s.payload(2, false, false).unwrap();
            let result = s.finish(budget(), 0, &mut |_| panic!("unexpected replay"));
            if placeholder {
                result.unwrap();
            } else {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("replaceContentContinuity"));
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
        s.kind(0, Some(true), false).unwrap();
        s.kind(1, Some(false), false).unwrap();
        s.reference(0, 0).unwrap();
        s.reference(2, 2).unwrap();
        s.payload(0, false, false).unwrap();
        s.payload(2, false, false).unwrap();
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
    fn replay_fixture(faces: &[[Point; 3]]) -> Vec<u8> {
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
    fn original_fixture(offset: f64, inconsistent: bool) -> Vec<u8> {
        use serde_json::json;
        let vertices = [
            [offset, 0., 0.],
            [offset + 2., 0., 0.],
            [offset, 2., 0.],
            [offset, 0., 2.],
        ];
        let ids = [[0usize, 1, 2], [1, 0, 3], [0, 2, 3], [1, 3, 2]];
        let mut faces = ids.map(|face| face.map(|i| vertices[i]));
        if inconsistent {
            faces[0][0][0] += 0.125;
        }
        let bytes = replay_fixture(&faces);
        let n = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let mut doc: Value = serde_json::from_slice(&bytes[20..20 + n]).unwrap();
        let mut bin = bytes[28 + n..].to_vec();
        let mut schema = serde_json::Map::new();
        let mut properties = serde_json::Map::new();
        for (column, key) in SOURCE_COLUMNS
            .into_iter()
            .chain(["triangle_index", "vertex_0", "vertex_1", "vertex_2"])
            .enumerate()
        {
            let view = doc["bufferViews"].as_array().unwrap().len();
            doc["bufferViews"]
                .as_array_mut()
                .unwrap()
                .push(json!({"buffer":0,"byteOffset":bin.len(),"byteLength":32}));
            for (row, face) in ids.iter().enumerate() {
                let value = match column {
                    0..=4 => 0,
                    5 => row as u64,
                    _ => face[column - 6] as u64,
                };
                bin.extend(value.to_le_bytes());
            }
            schema.insert(key.into(), json!({"componentType":"UINT64"}));
            properties.insert(key.into(), json!({"values":view}));
        }
        let view = doc["bufferViews"].as_array().unwrap().len();
        doc["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({"buffer":0,"byteOffset":bin.len(),"byteLength":24}));
        for row in 0u16..4 {
            for _ in 0..3 {
                bin.extend(row.to_le_bytes());
            }
        }
        doc["accessors"]
            .as_array_mut()
            .unwrap()
            .push(json!({"bufferView":view,"componentType":5123,"type":"SCALAR","count":12}));
        let primitive = &mut doc["meshes"][0]["primitives"][0];
        primitive["attributes"]["_FEATURE_ID_0"] = json!(1);
        primitive["extensions"] = json!({"EXT_mesh_features":{"featureIds":[{"label":"source_triangle","attribute":0,"propertyTable":0,"featureCount":4}]}});
        doc["extensions"] = json!({"EXT_structural_metadata":{"schema":{"classes":{"source_triangle":{"properties":schema}}},"propertyTables":[{"class":"source_triangle","count":4,"properties":properties}]}});
        doc["buffers"][0]["byteLength"] = json!(bin.len());
        pack_fixture(doc, bin)
    }
    #[test]
    fn original_replay_compares_closed_aliases_after_exact_reference_placement() {
        let source =
            replay_originals(&original_fixture(0., false), IDENTITY, 1 << 20, &effort()).unwrap();
        assert_eq!(source.len(), 4);
        let mut frame = IDENTITY;
        frame[12] = 10.;
        let rebased =
            replay_originals(&original_fixture(-10., false), frame, 1 << 20, &effort()).unwrap();
        compare_originals(&source, &rebased, &effort()).unwrap();
        for gap in [0.125, 10.] {
            frame[12] = 10. + gap;
            let moved = replay_originals(&original_fixture(-10., false), frame, 1 << 20, &effort())
                .unwrap();
            assert!(compare_originals(&source, &moved, &effort())
                .unwrap_err()
                .to_string()
                .contains("sourceIdentity"));
        }
        assert!(
            replay_originals(&original_fixture(0., true), IDENTITY, 1 << 20, &effort())
                .unwrap_err()
                .to_string()
                .contains("sourceIdentity")
        );
    }
    fn domain_pair(a: (u64, u64), b: (u64, u64)) -> Store {
        let mut s = store();
        s.decoded_limit = 1 << 20;
        for (rank, (lo, hi)) in [a, b].into_iter().enumerate() {
            s.connection
                .execute(
                    "INSERT INTO original_domains VALUES(?,?,?,?)",
                    params![
                        rank as i64,
                        vec![0u8; 40],
                        lo.to_be_bytes().to_vec(),
                        hi.to_be_bytes().to_vec()
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
    fn original_domain_ranges_include_equal_endpoints_and_full_uint64() {
        for (a, b, overlap) in [
            ((0, i64::MAX as u64), (i64::MAX as u64, u64::MAX), true),
            ((1u64 << 63, u64::MAX), (u64::MAX, u64::MAX), true),
            ((0, 0), (u64::MAX, u64::MAX), false),
            ((u64::MAX, u64::MAX), (0, u64::MAX - 1), false),
        ] {
            let s = domain_pair(a, b);
            let mut reads = 0;
            let result = s.replay_original_aliases(&effort(), &mut |_| {
                reads += 1;
                Err(bad("sourceIdentity", "selected identity pair"))
            });
            if overlap {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("selected identity pair"));
                assert_eq!(reads, 1);
            } else {
                result.unwrap();
                assert_eq!(reads, 0);
            }
        }
    }
    #[test]
    fn identity_candidates_charge_before_interval_filter_and_repeat_placements() {
        let s = domain_pair((u64::MAX, u64::MAX), (0, u64::MAX - 1));
        let mut work = effort();
        work.budget.value = 2;
        assert!(s
            .replay_original_aliases(&work, &mut |_| panic!("disjoint intervals"))
            .unwrap_err()
            .exceeded()
            .is_some());
        let bytes = original_fixture(0., false);
        for gap in [0., 0.125, 10.] {
            let s = domain_pair((0, 3), (0, 3));
            // Two references to one payload still have distinct placed meanings.
            s.connection
                .execute("UPDATE placements SET rank=0 WHERE id=1", [])
                .unwrap();
            let mut frame = IDENTITY;
            frame[12] = gap;
            s.connection
                .execute("UPDATE placements SET frame=? WHERE id=1", [encode(&frame)])
                .unwrap();
            let mut reads = 0;
            let result = s.replay_original_aliases(&effort(), &mut |rank| {
                assert_eq!(rank, 0);
                reads += 1;
                Ok(bytes.clone())
            });
            assert_eq!(reads, 2);
            if gap == 0. {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains("sourceIdentity"));
            }
        }
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
        for (tile, rank) in [(1, 0), (2, 1), (3, 2)] {
            s.reference(tile, rank)?;
            s.payload(rank, true, rank != 0)?;
        }
        assert!(s.census().is_none());
        s.finish(budget(), 0, &mut |rank| Ok(payloads[rank as usize].clone()))?;
        Ok(s.census().unwrap())
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
        let coarse = [[[20., 20., 0.], [22., 20., 0.], [20., 22., 0.]]];
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
        let mut left = OriginalMap::new();
        left.insert(([0; 5], 0), point_key(a));
        let mut right = OriginalMap::new();
        right.insert(([0; 5], 0), point_key(b));
        assert!(compare_originals(&left, &right, &effort())
            .unwrap_err()
            .to_string()
            .contains("sourceIdentity"));
    }
    #[test]
    fn every_incident_face_and_repeated_reference_contributes_to_open_count() {
        let s = store();
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
    #[test]
    fn above_cap_is_counted_while_d4_property_rows_remain_bounded() {
        let triangle = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let above = vec![triangle; 8193];
        let census = delivery_case(&above, &[triangle], &[]).unwrap();
        assert_eq!(census.stopped_above_cap, 1);
        assert_eq!(census.max_above_cap_triangles, 8193);
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

    #[test]
    fn final_work_has_a_structured_budget_refusal() {
        let mut s = store();
        s.tile(0, None, IDENTITY).unwrap();
        s.tile(1, Some(0), IDENTITY).unwrap();
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
