//! Native, archive-only HLOD delivery checks. C1 owns admission; this module owns
//! the decoded leaf contact/ancestor interface relation, never producer facts.
//!
//! A payload is decoded once at its catalog rank. Only its open/nonmanifold
//! edges survive the bounded per-payload reduction. A disk R-tree joins leaf
//! contacts; each contact is checked along depth-bounded ancestor paths. No
//! archive-sized triangle/point vector or ancestor cross-product is retained.
//! SQLite pages are capped; its temporary sorts are disk backed. Candidate and
//! corner work uses the remaining payload-element budget (including one unit
//! per 10,000 SQLite VM operations). Dense/pathological spatial joins therefore
//! refuse with a resource limit rather than silently truncating the census.
//!
//! Proof limits: original rows certify internal archive correspondence, not an
//! unavailable source document. Fresh coarse UV charts cannot be certified from
//! leaf UVs. Winding is checked on decoded interface incidences and coincident
//! exterior segments, not as a source-distance or whole-surface certificate.
use super::{identity, invalid, spill_failure, Budget, Error, ScratchRefusal};
use crate::spill::{Scratch, SlotFile};
use rusqlite::{params, Connection};
use serde_json::{value::RawValue, Value};
use std::{
    collections::BTreeMap,
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
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: Point, b: Point) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}
fn point(m: Matrix, p: Point) -> Point {
    std::array::from_fn(|i| m[i] * p[0] + m[4 + i] * p[1] + m[8 + i] * p[2] + m[12 + i])
}
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|i| (0..4).map(|k| a[k * 4 + i % 4] * b[i / 4 * 4 + k]).sum())
}
fn scale(m: Matrix) -> f64 {
    (0..3)
        .map(|i| (0..3).map(|k| m[k * 4 + i].abs()).sum::<f64>())
        .fold(0., f64::max)
}
fn normal(m: Matrix, n: Point) -> Point {
    let x = [m[0], m[1], m[2]];
    let y = [m[4], m[5], m[6]];
    let z = [m[8], m[9], m[10]];
    let (a, b, c) = (cross(y, z), cross(z, x), cross(x, y));
    std::array::from_fn(|i| a[i] * n[0] + b[i] * n[1] + c[i] * n[2])
}
fn roundoff(p: Point) -> f64 {
    p.into_iter().map(f64::abs).fold(1., f64::max) * f64::EPSILON * 64.
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
    fn position(&self, i: usize, m: Matrix) -> Result<(Point, f64), Error> {
        if self.components != 3 {
            return Err(bad("interfaceSegments", "POSITION must be VEC3"));
        }
        let p = [self.at(i, 0)?, self.at(i, 1)?, self.at(i, 2)?];
        let p = point(m, p);
        finite(&p)?;
        // Half a source lattice step propagated through the scene's linear map.
        // Float32 uses its actual exponent, rather than the producer's reported error.
        let step = match self.component {
            5126 => {
                (0..3)
                    .map(|c| self.at(i, c).unwrap_or(0.).abs())
                    .fold(f32::MIN_POSITIVE as f64, f64::max)
                    * f32::EPSILON as f64
            }
            5120 if self.normalized => 1. / 127.,
            5121 if self.normalized => 1. / 255.,
            5122 if self.normalized => 1. / 32767.,
            5123 if self.normalized => 1. / 65535.,
            _ => 1.,
        };
        Ok((p, step * scale(m) * 0.5 + roundoff(p)))
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

#[derive(Clone)]
struct Edge {
    a: Point,
    b: Point,
    n: Point,
    tolerance: f64,
    incidence: i32,
    count: usize,
    original: Option<(Source, [u64; 2])>,
}
impl Edge {
    fn bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(80);
        for p in [self.a, self.b, self.n] {
            v.extend(p);
        }
        v.push(self.tolerance);
        encode(&v)
    }
    fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let v = decode::<10>(bytes)?;
        Ok(Self {
            a: v[0..3].try_into().unwrap(),
            b: v[3..6].try_into().unwrap(),
            n: v[6..9].try_into().unwrap(),
            tolerance: v[9],
            incidence: 1,
            count: 1,
            original: None,
        })
    }
    fn placed(&self, m: Matrix) -> Result<Self, Error> {
        let mut e = self.clone();
        e.a = point(m, e.a);
        e.b = point(m, e.b);
        e.n = normal(m, e.n);
        finite(&e.a)?;
        finite(&e.b)?;
        finite(&e.n)?;
        e.tolerance = e.tolerance * scale(m) + roundoff(e.a).max(roundoff(e.b));
        Ok(e)
    }
    fn bounds(&self) -> [f64; 6] {
        [
            self.a[0].min(self.b[0]) - self.tolerance,
            self.a[0].max(self.b[0]) + self.tolerance,
            self.a[1].min(self.b[1]) - self.tolerance,
            self.a[1].max(self.b[1]) + self.tolerance,
            self.a[2].min(self.b[2]) - self.tolerance,
            self.a[2].max(self.b[2]) + self.tolerance,
        ]
    }
}
fn point_key(p: Point) -> [u64; 3] {
    p.map(|v| if v == 0. { 0 } else { v.to_bits() })
}
fn on_segment(p: Point, a: Point, b: Point, tolerance: f64) -> Option<f64> {
    let d = sub(b, a);
    let l2 = dot(d, d);
    if l2 == 0. {
        return (norm(sub(p, a)) <= tolerance).then_some(0.);
    }
    let t = dot(sub(p, a), d) / l2;
    let nearest = std::array::from_fn(|i| a[i] + t.clamp(0., 1.) * d[i]);
    (norm(sub(p, nearest)) <= tolerance).then_some(t.clamp(0., 1.))
}
fn overlap(a: &Edge, b: &Edge) -> Option<(f64, f64)> {
    let tolerance = a.tolerance + b.tolerance;
    // Both endpoints must lie on the same infinite line, then clip intervals.
    let d = sub(a.b, a.a);
    let length = norm(d);
    if length <= tolerance {
        return None;
    }
    let l2 = dot(d, d);
    let project = |p: Point| {
        let t = dot(sub(p, a.a), d) / l2;
        let q = std::array::from_fn(|i| a.a[i] + t * d[i]);
        (t, norm(sub(p, q)))
    };
    let (x, dx) = project(b.a);
    let (y, dy) = project(b.b);
    if dx > tolerance || dy > tolerance {
        return None;
    }
    let lo = x.min(y).max(0.);
    let hi = x.max(y).min(1.);
    (hi - lo > tolerance / length).then_some((lo, hi))
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
            CREATE TABLE payloads(rank INTEGER PRIMARY KEY,identity_present INTEGER NOT NULL,triangle_present INTEGER NOT NULL);
            CREATE TABLE raw_edges(rank INTEGER NOT NULL,data BLOB NOT NULL); CREATE INDEX raw_rank ON raw_edges(rank);
            CREATE TABLE original_vertices(rank INTEGER NOT NULL,key BLOB NOT NULL,point BLOB NOT NULL,tol REAL NOT NULL);
            CREATE INDEX originals ON original_vertices(key,rank);
            CREATE TABLE edges(id INTEGER PRIMARY KEY,tile INTEGER NOT NULL,data BLOB NOT NULL,leaf INTEGER NOT NULL);
            CREATE INDEX edges_tile ON edges(tile);
            CREATE VIRTUAL TABLE spatial USING rtree(id,x0,x1,y0,y1,z0,z1);
            BEGIN;").map_err(sql)?;
        Ok(Self {
            connection,
            _file: file,
            _anchor: anchor,
            _scratch: scratch,
            checked: false,
        })
    }
    pub(super) fn tile(
        &mut self,
        id: u64,
        parent: Option<u64>,
        local: Matrix,
    ) -> Result<(), Error> {
        finite(&local)?;
        let world = if let Some(p) = parent {
            let bytes: Vec<u8> = self
                .connection
                .query_row("SELECT world FROM tiles WHERE id=?", [integer(p)?], |r| {
                    r.get(0)
                })
                .map_err(sql)?;
            multiply(decode::<16>(&bytes)?, local)
        } else {
            local
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
        let mut work = identity::Work::new(budget, already, decoded_limit);
        let mut edges = BTreeMap::<([u64; 3], [u64; 3]), Edge>::new();
        let mut originals = BTreeMap::<(Source, u64), (Point, f64)>::new();
        let tables = doc["extensions"]["EXT_structural_metadata"]["propertyTables"]
            .as_array()
            .ok_or_else(|| bad("hlodFeatureIds", "missing tables"))?;
        let mut used: Vec<Vec<bool>> = Vec::with_capacity(tables.len());
        for t in tables {
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
                        let mut tolerance: f64 = 0.;
                        for j in 0..3 {
                            let (p, t) = positions.position(ids[j], m)?;
                            xyz[j] = p;
                            tolerance = tolerance.max(t);
                        }
                        let (_, source) = primitive.row(ids[0], &mut work)?;
                        let mut original = None;
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
                                if let Some((p, t)) = originals.get(&key) {
                                    if norm(sub(*p, xyz[j])) > *t + tolerance {
                                        return Err(bad("sourceIdentity","original vertex maps to inconsistent decoded leaf positions"));
                                    }
                                } else {
                                    if originals.len().saturating_mul(144) > decoded_limit {
                                        return Err(Error::limit("sourceIdentity: original vertex workspace exceeds decoded document budget"));
                                    }
                                    originals.insert(key, (xyz[j], tolerance));
                                }
                            }
                            original = Some((source, vertices));
                        }
                        let normal = cross(sub(xyz[1], xyz[0]), sub(xyz[2], xyz[0]));
                        for j in 0..3 {
                            let k = (j + 1) % 3;
                            let a = point_key(xyz[j]);
                            let b = point_key(xyz[k]);
                            if a == b {
                                continue;
                            }
                            let forward = a < b;
                            let key = if forward { (a, b) } else { (b, a) };
                            let incidence = if forward { 1 } else { -1 };
                            if let Some(e) = edges.get_mut(&key) {
                                e.count += 1;
                                e.incidence += incidence;
                                e.tolerance = e.tolerance.max(tolerance);
                            } else {
                                if edges.len().saturating_mul(256) > decoded_limit {
                                    return Err(Error::limit("interfaceSegments: edge workspace exceeds decoded document budget"));
                                }
                                edges.insert(
                                    key,
                                    Edge {
                                        a: xyz[j],
                                        b: xyz[k],
                                        n: normal,
                                        tolerance,
                                        incidence,
                                        count: 1,
                                        original: original.map(|(s, v)| (s, [v[j], v[k]])),
                                    },
                                );
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
        let mut insert = self
            .connection
            .prepare_cached("INSERT INTO raw_edges VALUES(?,?)")
            .map_err(sql)?;
        let mut vertex = self
            .connection
            .prepare_cached("INSERT INTO original_vertices VALUES(?,?,?,?)")
            .map_err(sql)?;
        for e in edges.values() {
            if e.count == 2 && e.incidence == 0 {
                continue;
            }
            if !has_triangles && e.count == 2 && e.incidence.abs() == 2 {
                return Err(bad(
                    "interfaceWinding",
                    "coarse shared edge has equal directed incidences",
                ));
            }
            work.charge(1)?;
            insert
                .execute(params![integer(rank)?, e.bytes()])
                .map_err(sql)?;
            if let Some((source, vs)) = e.original {
                for (i, p) in [e.a, e.b].into_iter().enumerate() {
                    let mut key = Vec::with_capacity(48);
                    for n in source.into_iter().chain([vs[i]]) {
                        key.extend(n.to_le_bytes());
                    }
                    vertex
                        .execute(params![integer(rank)?, key, encode(&p), e.tolerance])
                        .map_err(sql)?;
                }
            }
        }
        Ok(work.used)
    }
    fn payload(&self, rank: u64, identity: bool, triangle: bool) -> Result<(), Error> {
        self.connection
            .execute(
                "INSERT INTO payloads VALUES(?,?,?)",
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

    /// An owned archive/report label can establish expectation even when every
    /// payload's identity was stripped. Call before finish; traversal order is immaterial.
    pub(super) fn expect_identity(&mut self) {
        self.checked = true;
    }

    pub(super) fn finish(&mut self, budget: Budget, already: u64) -> Result<u64, Error> {
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
        let result = self.finish_inner(&effort);
        self.connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(sql)?;
        let count = used.load(Ordering::Relaxed);
        budget.charge(already, count, || {
            "interfaceSegments: native delivery work exceeds payload element budget".into()
        })?;
        result?;
        self.connection.execute_batch("COMMIT;").map_err(sql)?;
        Ok(count)
    }
    fn finish_inner(&mut self, effort: &Effort) -> Result<(), Error> {
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
            return Ok(());
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
        self.originals(effort)?;
        self.place_edges(effort)?;
        self.contacts(effort)?;
        self.exterior_winding(effort)?;
        Ok(())
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
    fn originals(&self, effort: &Effort) -> Result<(), Error> {
        // Only boundary source vertices reach this disk join. Interior original-row
        // correspondence was checked inside the bounded ingest workspace.
        let mut query = self
            .connection
            .prepare("SELECT key,point,tol FROM original_vertices ORDER BY key,rank")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut previous: Option<(Vec<u8>, Point, f64)> = None;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let key: Vec<u8> = row.get(0).map_err(sql)?;
            let bytes: Vec<u8> = row.get(1).map_err(sql)?;
            let p = decode::<3>(&bytes)?;
            let tolerance: f64 = row.get(2).map_err(sql)?;
            if let Some((k, q, t)) = &previous {
                if *k == key && norm(sub(p, *q)) > tolerance + *t {
                    return Err(bad(
                        "sourceIdentity",
                        "same original boundary vertex disagrees between decoded descendant leaves",
                    ));
                }
            }
            previous = Some((key, p, tolerance));
        }
        Ok(())
    }
    fn place_edges(&self, effort: &Effort) -> Result<(), Error> {
        let mut query=self.connection.prepare("SELECT r.tile,t.world,t.leaf,e.data FROM refs r JOIN tiles t ON t.id=r.tile JOIN raw_edges e ON e.rank=r.rank ORDER BY r.tile").map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut edge = self
            .connection
            .prepare_cached("INSERT INTO edges(tile,data,leaf) VALUES(?,?,?)")
            .map_err(sql)?;
        let mut spatial = self
            .connection
            .prepare_cached("INSERT INTO spatial VALUES(?,?,?,?,?,?,?)")
            .map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let tile: i64 = row.get(0).map_err(sql)?;
            let m: Vec<u8> = row.get(1).map_err(sql)?;
            let leaf: bool = row.get(2).map_err(sql)?;
            let bytes: Vec<u8> = row.get(3).map_err(sql)?;
            let e = Edge::from_bytes(&bytes)?.placed(decode::<16>(&m)?)?;
            edge.execute(params![tile, e.bytes(), leaf]).map_err(sql)?;
            let id = self.connection.last_insert_rowid();
            let b = e.bounds();
            if b.iter()
                .any(|v| !v.is_finite() || v.abs() > f32::MAX as f64)
            {
                return Err(Error::unsupported(
                    "interfaceSegments: transformed geometry exceeds spatial index finite range",
                ));
            }
            spatial
                .execute(params![id, b[0], b[1], b[2], b[3], b[4], b[5]])
                .map_err(sql)?;
        }
        Ok(())
    }
    fn contacts(&self, effort: &Effort) -> Result<(), Error> {
        let mut outer = self
            .connection
            .prepare("SELECT id,tile,data FROM edges WHERE leaf=1 ORDER BY id")
            .map_err(sql)?;
        let mut rows = outer.query([]).map_err(sql)?;
        let mut nearby = self
            .connection
            .prepare(
                "SELECT e.tile,e.data FROM spatial s JOIN edges e ON e.id=s.id
            WHERE s.x0<=?1 AND s.x1>=?2 AND s.y0<=?3 AND s.y1>=?4 AND s.z0<=?5 AND s.z1>=?6
                AND e.leaf=1 AND e.id>?7 AND e.tile<>?8",
            )
            .map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let id: i64 = row.get(0).map_err(sql)?;
            let a_tile: i64 = row.get(1).map_err(sql)?;
            let bytes: Vec<u8> = row.get(2).map_err(sql)?;
            let a = Edge::from_bytes(&bytes)?;
            let b = a.bounds();
            let mut candidates = nearby
                .query(params![b[1], b[0], b[3], b[2], b[5], b[4], id, a_tile])
                .map_err(sql)?;
            while let Some(other) = candidates.next().map_err(sql)? {
                effort.charge(1)?;
                let b_tile: i64 = other.get(0).map_err(sql)?;
                let bytes: Vec<u8> = other.get(1).map_err(sql)?;
                let b = Edge::from_bytes(&bytes)?;
                if overlap(&a, &b).is_some() {
                    self.ancestors(a_tile, b_tile, Some(&a), None, effort)?;
                    self.ancestors(b_tile, a_tile, Some(&b), None, effort)?;
                }
                let tol = a.tolerance + b.tolerance;
                for p in [a.a, a.b] {
                    if let Some(u) = on_segment(p, b.a, b.b, tol) {
                        self.ancestors(a_tile, b_tile, None, Some((p, a.tolerance)), effort)?;
                        // An endpoint contact locks the point, not every edge
                        // incident there. Only a strict T contact locks its long side.
                        let margin = tol / norm(sub(b.b, b.a));
                        if u > margin && u < 1. - margin {
                            self.ancestors(b_tile, a_tile, Some(&b), None, effort)?;
                        }
                    }
                }
                for p in [b.a, b.b] {
                    if let Some(u) = on_segment(p, a.a, a.b, tol) {
                        self.ancestors(b_tile, a_tile, None, Some((p, b.tolerance)), effort)?;
                        let margin = tol / norm(sub(a.b, a.a));
                        if u > margin && u < 1. - margin {
                            self.ancestors(a_tile, b_tile, Some(&a), None, effort)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn ancestors(
        &self,
        mut tile: i64,
        other: i64,
        segment: Option<&Edge>,
        p: Option<(Point, f64)>,
        effort: &Effort,
    ) -> Result<(), Error> {
        let mut lookup = self
            .connection
            .prepare_cached("SELECT parent,last FROM tiles WHERE id=?")
            .map_err(sql)?;
        loop {
            effort.charge(1)?;
            let (parent, last): (Option<i64>, i64) = lookup
                .query_row([tile], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(sql)?;
            if other >= tile && other <= last {
                break;
            }
            if let Some(e) = segment {
                self.require_segment(tile, e, effort)?;
            }
            if let Some((p, t)) = p {
                self.require_point(tile, p, t, effort)?;
            }
            let Some(parent) = parent else {
                break;
            };
            tile = parent;
        }
        Ok(())
    }
    fn require_point(
        &self,
        tile: i64,
        p: Point,
        tolerance: f64,
        effort: &Effort,
    ) -> Result<(), Error> {
        let mut query=self.connection.prepare_cached("SELECT e.data FROM spatial s JOIN edges e ON e.id=s.id
            WHERE e.tile=?1 AND s.x0<=?2 AND s.x1>=?3 AND s.y0<=?4 AND s.y1>=?5 AND s.z0<=?6 AND s.z1>=?7").map_err(sql)?;
        let mut rows = query
            .query(params![
                tile,
                p[0] + tolerance,
                p[0] - tolerance,
                p[1] + tolerance,
                p[1] - tolerance,
                p[2] + tolerance,
                p[2] - tolerance
            ])
            .map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let bytes: Vec<u8> = row.get(0).map_err(sql)?;
            let e = Edge::from_bytes(&bytes)?;
            if norm(sub(p, e.a)) <= tolerance + e.tolerance
                || norm(sub(p, e.b)) <= tolerance + e.tolerance
            {
                return Ok(());
            }
        }
        Err(bad(
            "interfacePoints",
            "decoded ancestor lost a leaf contact point",
        ))
    }
    fn require_segment(&self, tile: i64, want: &Edge, effort: &Effort) -> Result<(), Error> {
        let mut query=self.connection.prepare_cached("SELECT e.data FROM spatial s JOIN edges e ON e.id=s.id
            WHERE e.tile=?1 AND s.x0<=?2 AND s.x1>=?3 AND s.y0<=?4 AND s.y1>=?5 AND s.z0<=?6 AND s.z1>=?7").map_err(sql)?;
        let b = want.bounds();
        let mut rows = query
            .query(params![tile, b[1], b[0], b[3], b[2], b[5], b[4]])
            .map_err(sql)?;
        let mut intervals = Vec::<(f64, f64, bool, f64)>::new();
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let bytes: Vec<u8> = row.get(0).map_err(sql)?;
            let e = Edge::from_bytes(&bytes)?;
            if let Some((lo, hi)) = overlap(want, &e) {
                if intervals.len() >= 65_536 {
                    return Err(Error::limit(
                        "interfaceSegments: one interface exceeds bounded interval workspace",
                    ));
                }
                let winding = dot(want.n, e.n) >= -1e-12 * norm(want.n) * norm(e.n);
                intervals.push((
                    lo,
                    hi,
                    winding,
                    (want.tolerance + e.tolerance) / norm(sub(want.b, want.a)),
                ));
            }
        }
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let covered = |oriented: bool| {
            let mut end: f64 = 0.;
            let mut tolerance: f64 = 0.;
            for &(lo, hi, winding, t) in &intervals {
                if oriented && !winding {
                    continue;
                }
                if lo > end + t {
                    return false;
                }
                end = end.max(hi);
                tolerance = tolerance.max(t);
                if end >= 1. - tolerance {
                    return true;
                }
            }
            false
        };
        if !covered(false) {
            return Err(bad(
                "interfaceSegments",
                "decoded ancestor lacks consecutive subsegment edges of a leaf interface",
            ));
        }
        if !covered(true) {
            return Err(bad(
                "interfaceWinding",
                "decoded ancestor reverses a leaf interface incidence",
            ));
        }
        Ok(())
    }
    fn exterior_winding(&self, effort: &Effort) -> Result<(), Error> {
        // Roots contain every leaf, so the outside-owner rule has no required
        // interfaces there. Coincident exterior edges still carry an independently
        // decoded orientation witness; this also detects an entirely flipped root.
        let mut outer=self.connection.prepare("SELECT e.tile,e.data,t.last FROM edges e JOIN tiles t ON t.id=e.tile WHERE e.leaf=0").map_err(sql)?;
        let mut rows = outer.query([]).map_err(sql)?;
        let mut nearby = self
            .connection
            .prepare(
                "SELECT e.data FROM spatial s JOIN edges e ON e.id=s.id
            WHERE s.x0<=?1 AND s.x1>=?2 AND s.y0<=?3 AND s.y1>=?4 AND s.z0<=?5 AND s.z1>=?6
                AND e.leaf=1 AND e.tile>?7 AND e.tile<=?8",
            )
            .map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let tile: i64 = row.get(0).map_err(sql)?;
            let bytes: Vec<u8> = row.get(1).map_err(sql)?;
            let last: i64 = row.get(2).map_err(sql)?;
            let a = Edge::from_bytes(&bytes)?;
            let bounds = a.bounds();
            let mut candidates = nearby
                .query(params![
                    bounds[1], bounds[0], bounds[3], bounds[2], bounds[5], bounds[4], tile, last
                ])
                .map_err(sql)?;
            let mut positive = false;
            let mut negative = false;
            while let Some(other) = candidates.next().map_err(sql)? {
                effort.charge(1)?;
                let bytes: Vec<u8> = other.get(0).map_err(sql)?;
                let b = Edge::from_bytes(&bytes)?;
                if overlap(&a, &b).is_some() {
                    if dot(a.n, b.n) < -1e-12 * norm(a.n) * norm(b.n) {
                        negative = true;
                    } else {
                        positive = true;
                    }
                }
            }
            if negative && !positive {
                return Err(bad(
                    "interfaceWinding",
                    "coarse exterior incidence reverses decoded descendant leaf geometry",
                ));
            }
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
    fn edge(a: Point, b: Point) -> Edge {
        Edge {
            a,
            b,
            n: [0., 0., 1.],
            tolerance: 1e-12,
            incidence: 1,
            count: 1,
            original: None,
        }
    }
    fn placed(store: &Store, tile: i64, e: &Edge, leaf: bool) {
        store
            .connection
            .execute(
                "INSERT INTO edges(tile,data,leaf) VALUES(?,?,?)",
                params![tile, e.bytes(), leaf],
            )
            .unwrap();
        let id = store.connection.last_insert_rowid();
        let b = e.bounds();
        store
            .connection
            .execute(
                "INSERT INTO spatial VALUES(?,?,?,?,?,?,?)",
                params![id, b[0], b[1], b[2], b[3], b[4], b[5]],
            )
            .unwrap();
    }
    #[test]
    fn contact_point_does_not_prove_consecutive_subsegment_delivery() {
        let s = store();
        let required = edge([0., 0., 0.], [2., 0., 0.]);
        placed(&s, 0, &edge([0., 0., 0.], [1., 0., 0.]), false);
        placed(&s, 0, &edge([1., 0., 0.], [1., 1., 0.]), false);
        placed(&s, 0, &edge([1., 1., 0.], [2., 0., 0.]), false);
        s.require_point(0, [1., 0., 0.], 1e-12, &effort()).unwrap();
        assert!(s
            .require_segment(0, &required, &effort())
            .unwrap_err()
            .to_string()
            .contains("interfaceSegments"));
    }
    #[test]
    fn collinear_subdivision_passes_but_reversed_incidence_fails() {
        let s = store();
        let required = edge([0., 0., 0.], [2., 0., 0.]);
        placed(&s, 0, &edge([0., 0., 0.], [1., 0., 0.]), false);
        placed(&s, 0, &edge([1., 0., 0.], [2., 0., 0.]), false);
        s.require_segment(0, &required, &effort()).unwrap();
        let mut reversed = required.clone();
        reversed.n = [0., 0., -1.];
        assert!(s
            .require_segment(0, &reversed, &effort())
            .unwrap_err()
            .to_string()
            .contains("interfaceWinding"));
    }
    #[test]
    fn reflected_scene_changes_position_and_oriented_area_together() {
        let mut m = IDENTITY;
        m[0] = -2.;
        m[5] = 4.;
        m[12] = 8.;
        let e = edge([0., 0., 0.], [1., 0., 0.]).placed(m).unwrap();
        assert_eq!(e.a, [8., 0., 0.]);
        assert_eq!(e.b, [6., 0., 0.]);
        assert_eq!(e.n, [0., 0., -8.]);
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
            let result = s.finish(budget(), 0);
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
                .finish(budget(), 0)
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
            .finish(budget(), 0)
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
            let result = s.finish(budget(), 0);
            if same {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains("D4 coarse"));
            }
        }
    }
    #[test]
    fn root_exterior_winding_has_a_descendant_witness_without_outside_owner() {
        let mut s = store();
        s.tile(0, None, IDENTITY).unwrap();
        s.tile(1, Some(0), IDENTITY).unwrap();
        s.hierarchy(&effort()).unwrap();
        let good = edge([0., 0., 0.], [2., 0., 0.]);
        let mut flipped = good.clone();
        flipped.n = [0., 0., -1.];
        placed(&s, 0, &flipped, false);
        placed(&s, 1, &good, true);
        assert!(s
            .exterior_winding(&effort())
            .unwrap_err()
            .to_string()
            .contains("interfaceWinding"));
    }
    #[test]
    fn shared_corner_does_not_lock_a_canceled_internal_edge() {
        let mut s = store();
        s.tile(0, None, IDENTITY).unwrap();
        s.tile(1, Some(0), IDENTITY).unwrap();
        s.tile(2, Some(1), IDENTITY).unwrap();
        s.tile(3, Some(1), IDENTITY).unwrap();
        s.tile(4, Some(0), IDENTITY).unwrap();
        for tile in [1, 2, 3, 4] {
            s.reference(tile, tile).unwrap();
        }
        s.hierarchy(&effort()).unwrap();
        // A--B is internal to tile 1, so its proxy may cancel it. The
        // outside leaf touches A only; A remains on a different proxy edge.
        let (a, b, d, c) = ([0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [-1., -1., 0.]);
        placed(&s, 1, &edge(a, d), false);
        placed(&s, 1, &edge(d, b), false);
        placed(&s, 2, &edge(a, b), true);
        placed(&s, 3, &edge(b, a), true);
        placed(&s, 4, &edge(a, c), true);
        s.contacts(&effort()).unwrap();
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
            .finish(budget(), 0)
            .unwrap_err()
            .to_string()
            .contains("replaceContentContinuity"));
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
            )
            .unwrap_err();
        assert!(error.exceeded().is_some());
    }
}
