//! H17: certified geometry LOD — the coarse levels above H8's full-detail leaves.
//!
//! Runs inside the H8 producer after every leaf is published, while the captured baked
//! source is still in scratch (direct certificate terms need it; H16 never sees it). Every
//! internal tile, deepest depth first, gets a proxy built from its children's PUBLISHED
//! bytes: islands may vanish, seam chains are decimated identically on both sides, the
//! rest is simplified by half-edge collapse onto existing source vertices, quantized on the
//! frozen H8 lattice, encoded, decoded again, and certified both directions against the
//! source (composed through the children and direct, outward rounded, bounded work). Small
//! interior holes are filled only when the fill fits the already certified error. The
//! proxies and their rows (`proxies.jsonl`, postorder) are then published by H16.
//!
//! Proxies are untextured: flat material factors, smooth normals. Texture baking is H13.
mod certify;
mod codec;
mod geom;
pub(crate) mod incidence;
mod mesh;
#[cfg(test)]
mod tests;

pub(crate) use geom::Mutation;

use self::{
    codec::{ProxyPrimitive, RecordTable},
    geom::{Arith, Bvh, Limits, Tri},
    incidence::{LodSource, VertexInfo, NEVER},
    mesh::{Face, Key, KeyFace, KeyMesh, Label, Origin, Simplifier},
};
use super::{
    full::FullCaptured,
    leaves::geometry::LatticePlan,
    records::{NodeKind, NodeRecord},
};
use crate::{
    spill::{ExternalSorter, Record, Scratch, Sequence},
    JobError, JobErrorKind,
};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

/// Proxy face target `P` per tile. Above `2P` after simplification a tile is reported over
/// budget; above `4P` it is Unsupported. (Property tables stay within CesiumJS's 8192 rows
/// whatever the face count: each glTF primitive holds at most 8192 triangles.)
pub(crate) const TARGET_FACES: usize = 8192;
/// Most vertices of a fillable hole loop.
const FILL_LOOP: usize = 64;

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}
fn resource(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message.into())
}
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// One upward-rounded sum.
fn up_add(ar: Arith, a: f64, b: f64) -> f64 {
    ar.add_up(a, b)
}

/// Frozen hash of a welded key for seam retention (SplitMix64 finalizer).
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}
pub(crate) fn key_hash(key: Key) -> u64 {
    mix(mix(u64::from(key[0]) | (u64::from(key[1]) << 32)) ^ u64::from(key[2]))
}

/// Whether a cut vertex survives at depth `d` (identical on both sides of a seam, and
/// monotone: kept at `d` implies kept at every deeper depth).
pub(crate) fn kept(info: &VertexInfo, key: Key, d: u32, retention: u8) -> bool {
    u32::from(info.ell) <= d
        || (info.c3 != NEVER && u32::from(info.c3) <= d)
        || info.special
        || key_hash(key).trailing_zeros() >= u32::from(retention)
}

/// Retention exponent per depth: `k_d = round(log4(Sbar_d / P))` with `Sbar_d` the exact
/// mean source count of depth-`d` internal tiles, by integer comparisons
/// (`S >= 2^(2j+1) I P` for `j = 0, 1, ...`), then made monotone (`k_d >= k_{d+1}`).
pub(crate) fn retention(sums: &[(u128, u128)], target: u128) -> Vec<u8> {
    let mut k: Vec<u8> = sums
        .iter()
        .map(|&(sum, count)| {
            if count == 0 {
                return 0;
            }
            let mut j = 0u8;
            while j < 40 {
                let threshold = (1u128 << (2 * u32::from(j) + 1))
                    .checked_mul(count)
                    .and_then(|v| v.checked_mul(target));
                match threshold {
                    Some(t) if sum >= t => j += 1,
                    _ => break,
                }
            }
            j
        })
        .collect();
    for d in (0..k.len().saturating_sub(1)).rev() {
        k[d] = k[d].max(k[d + 1]);
    }
    k
}

/// One proxy's published row (fixed size, external-sorted into postorder).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Row {
    first: u64,
    end: u64,
    depth: u8,
    published: f64,
    to: f64,
    from: f64,
    composed_to: f64,
    composed_from: f64,
    direct_to: f64,
    direct_from: f64,
    child_max: f64,
    faces: u32,
    primitives: u32,
    vanished: u32,
    filled: u32,
    bytes: u64,
    sha256: [u8; 32],
    flags: u8,
}
const FLAG_COMPOSED_EXHAUSTED: u8 = 1;
const FLAG_DIRECT_EXHAUSTED: u8 = 2;
const FLAG_DIRECT_SKIPPED: u8 = 4;

impl Row {
    fn order(&self) -> (u64, u8, u64) {
        (self.end, u8::MAX - self.depth, self.first)
    }
}
impl Eq for Row {}
impl PartialOrd for Row {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Row {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.order().cmp(&other.order())
    }
}
impl Record for Row {
    const SIZE: usize = 8 * 2 + 1 + 8 * 8 + 4 * 4 + 8 + 32 + 1;
    fn encode(&self, out: &mut [u8]) {
        let mut w = 0;
        let mut put = |b: &[u8]| {
            out[w..w + b.len()].copy_from_slice(b);
            w += b.len();
        };
        put(&self.first.to_le_bytes());
        put(&self.end.to_le_bytes());
        put(&[self.depth]);
        for v in [
            self.published,
            self.to,
            self.from,
            self.composed_to,
            self.composed_from,
            self.direct_to,
            self.direct_from,
            self.child_max,
        ] {
            put(&v.to_le_bytes());
        }
        for v in [self.faces, self.primitives, self.vanished, self.filled] {
            put(&v.to_le_bytes());
        }
        put(&self.bytes.to_le_bytes());
        put(&self.sha256);
        put(&[self.flags]);
    }
    fn decode(b: &[u8]) -> Self {
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        let f = |i: usize| f64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        Self {
            first: u64_at(0),
            end: u64_at(8),
            depth: b[16],
            published: f(17),
            to: f(25),
            from: f(33),
            composed_to: f(41),
            composed_from: f(49),
            direct_to: f(57),
            direct_from: f(65),
            child_max: f(73),
            faces: u32_at(81),
            primitives: u32_at(85),
            vanished: u32_at(89),
            filled: u32_at(93),
            bytes: u64_at(97),
            sha256: b[105..137].try_into().unwrap(),
            flags: b[137],
        }
    }
}

/// A seam edge for the per-depth audit: both endpoints cut at this depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SeamEdge {
    lo: Key,
    hi: Key,
    owner: u64,
    owner_depth: u8,
    lo_dec: [u32; 3],
    hi_dec: [u32; 3],
    special: bool,
    round: u8,
}
impl Record for SeamEdge {
    const SIZE: usize = 12 * 4 + 8 + 1 + 1 + 1;
    fn encode(&self, out: &mut [u8]) {
        let mut w = 0;
        for k in [self.lo, self.hi, self.lo_dec, self.hi_dec] {
            for v in k {
                out[w..w + 4].copy_from_slice(&v.to_le_bytes());
                w += 4;
            }
        }
        out[w..w + 8].copy_from_slice(&self.owner.to_le_bytes());
        out[w + 8] = self.owner_depth;
        out[w + 9] = u8::from(self.special);
        out[w + 10] = self.round;
    }
    fn decode(b: &[u8]) -> Self {
        let k = |i: usize| -> [u32; 3] {
            std::array::from_fn(|a| u32::from_le_bytes(b[i + 4 * a..i + 4 * a + 4].try_into().unwrap()))
        };
        Self {
            lo: k(0),
            hi: k(12),
            lo_dec: k(24),
            hi_dec: k(36),
            owner: u64::from_le_bytes(b[48..56].try_into().unwrap()),
            owner_depth: b[56],
            special: b[57] != 0,
            round: b[58],
        }
    }
}

/// A finished tile of the current depth (its latest round).
struct Done(Outcome);

/// A seam vertex a tile removes (logged so a contested vertex re-runs every remover).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ForcedKey {
    key: Key,
    first: u64,
}
impl Record for ForcedKey {
    const SIZE: usize = 20;
    fn encode(&self, out: &mut [u8]) {
        for (i, v) in self.key.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        out[12..].copy_from_slice(&self.first.to_le_bytes());
    }
    fn decode(b: &[u8]) -> Self {
        Self {
            key: std::array::from_fn(|i| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap())),
            first: u64::from_le_bytes(b[12..20].try_into().unwrap()),
        }
    }
}

/// What the LOD stage reports into the leaf manifest.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LodReport {
    pub schema_version: u32,
    pub proxies: u64,
    pub proxy_faces: u64,
    pub proxy_bytes: u64,
    pub target_faces: u64,
    pub max_proxy_faces: u64,
    pub retention: Vec<u8>,
    pub max_error_by_depth: Vec<f64>,
    pub vanished_components: u64,
    pub vanished_faces: u64,
    pub fill_candidates: u64,
    pub fills: u64,
    pub fills_rejected_bound: u64,
    pub fills_rejected_topology: u64,
    pub forced_collapses: u64,
    pub forced_unchecked: u64,
    pub direct_to_used: u64,
    pub direct_from_used: u64,
    pub composed_exhausted: u64,
    pub direct_exhausted: u64,
    /// Tiles whose source range is too large for the direct budget (composed only).
    pub direct_skipped: u64,
    pub seam_edges: u64,
    pub seam_matched: u64,
    pub seam_unmatched_boundary: u64,
    pub seam_relabelled: u64,
    /// Seam consensus: extra rounds run and vertices kept because a tile could not
    /// remove them validly.
    pub seam_rounds: u64,
    pub seam_kept_extra: u64,
    pub lattice_sha256: String,
    pub sidecar_peak_bytes: u64,
    pub relaxed_tiles: u64,
    /// Tiles above `2P` faces (each glTF primitive still holds at most 8192 triangles).
    pub over_budget_tiles: u64,
    /// Run facts outside the deterministic manifest.
    #[serde(skip)]
    pub jobs: u64,
    #[serde(skip)]
    pub prepass_seconds: f64,
    #[serde(skip)]
    pub seconds: f64,
    #[serde(skip)]
    pub phase_seconds: [f64; 7],
    #[serde(skip)]
    pub audit_seconds: f64,
    pub texture: &'static str,
}

/// A child reference while listing a depth.
#[derive(Clone, Copy, Debug)]
enum Child {
    Leaf { node: NodeRecord, index: u64, error: f64 },
    Proxy { node: NodeRecord },
}
impl Child {
    fn node(&self) -> &NodeRecord {
        match self {
            Self::Leaf { node, .. } | Self::Proxy { node } => node,
        }
    }
}
struct Job {
    node: NodeRecord,
    children: Vec<Child>,
}

/// A child's geometry as its parent sees it.
struct ChildGeom {
    slot: u8,
    faces: Vec<KeyFace>,
    decoded: Vec<Tri>,
    to: f64,
    from: f64,
    published: f64,
    labels: BTreeMap<(u32, u32, u32), f64>,
}

/// Everything a node job reads.
struct Ctx<'a> {
    stage: &'a Path,
    sidecars: &'a Path,
    source: &'a Sequence<LodSource>,
    lattice: &'a LatticePlan,
    materials: &'a [Value],
    retention: &'a [u8],
    target: usize,
    mutation: Mutation,
    ar: Arith,
    check: &'a (dyn Fn() -> Result<(), JobError> + Sync),
}

fn slot_of(node: &NodeRecord) -> u8 {
    node.slot()
}

fn proxy_name(node: &NodeRecord) -> String {
    format!("proxies/f{}-d{}.glb", node.first, node.depth())
}
fn sidecar_path(dir: &Path, node: &NodeRecord) -> PathBuf {
    dir.join(format!("d{}", node.depth()))
        .join(format!("f{}.bin", node.first))
}

/// Decoded position of a source vertex at `depth` (exactly what any tile publishing it at
/// that depth decodes).
fn decode_vertex(lattice: &LatticePlan, key: Key, ell: u8, depth: u32) -> Result<[f32; 3], JobError> {
    let p = key.map(f32::from_bits);
    Ok(lattice.encode(&[p, p, p], &[ell; 3], depth)?.decoded[0])
}

fn tri_of(face: &[[f32; 3]; 3]) -> Tri {
    face.map(|p| p.map(f64::from))
}

// ---------------------------------------------------------------------------------------
// Sidecar: the key-space record of a proxy (scratch only), in its GLB face order.

struct Sidecar {
    to: f64,
    from: f64,
    published: f64,
    labels: Vec<(Label, f64)>,
    vertices: Vec<(Key, VertexInfo)>,
    faces: Vec<Face>,
}

impl Sidecar {
    fn write(&self, path: &Path) -> Result<u64, JobError> {
        let mut b: Vec<u8> = Vec::new();
        b.extend_from_slice(b"H17S\x01\0\0\0");
        for v in [self.to, self.from, self.published] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&(self.labels.len() as u64).to_le_bytes());
        for (l, e) in &self.labels {
            for v in [l.key.0, l.key.1, l.key.2, l.material] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&l.expanded_ordinal.to_le_bytes());
            b.extend_from_slice(&e.to_le_bytes());
        }
        b.extend_from_slice(&(self.vertices.len() as u64).to_le_bytes());
        for (k, i) in &self.vertices {
            for v in k {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&[i.ell, i.c2, i.c3, u8::from(i.special)]);
        }
        b.extend_from_slice(&(self.faces.len() as u64).to_le_bytes());
        for f in &self.faces {
            for v in f.v {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&f.label.to_le_bytes());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| JobError::io("create LOD sidecar folder", parent, e))?;
        }
        fs::write(path, &b).map_err(|e| JobError::io("write LOD sidecar", path, e))?;
        Ok(b.len() as u64)
    }

    fn read(path: &Path) -> Result<Self, JobError> {
        let b = fs::read(path).map_err(|e| JobError::io("read LOD sidecar", path, e))?;
        let bad = || state("LOD sidecar is corrupt");
        if b.get(..8) != Some(b"H17S\x01\0\0\0") {
            return Err(bad());
        }
        let mut at = 8usize;
        let mut take = |n: usize| -> Result<&[u8], JobError> {
            let s = b.get(at..at + n).ok_or_else(bad)?;
            at += n;
            Ok(s)
        };
        let f = |s: &[u8]| f64::from_le_bytes(s.try_into().unwrap());
        let u = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap());
        let l = |s: &[u8]| u64::from_le_bytes(s.try_into().unwrap());
        let to = f(take(8)?);
        let from = f(take(8)?);
        let published = f(take(8)?);
        let n = l(take(8)?) as usize;
        let mut labels = Vec::with_capacity(n.min(1 << 20));
        for _ in 0..n {
            let s = take(32)?;
            labels.push((
                Label {
                    key: (u(&s[0..4]), u(&s[4..8]), u(&s[8..12])),
                    material: u(&s[12..16]),
                    expanded_ordinal: l(&s[16..24]),
                },
                f(&s[24..32]),
            ));
        }
        let n = l(take(8)?) as usize;
        let mut vertices = Vec::with_capacity(n.min(1 << 24));
        for _ in 0..n {
            let s = take(16)?;
            vertices.push((
                [u(&s[0..4]), u(&s[4..8]), u(&s[8..12])],
                VertexInfo {
                    ell: s[12],
                    c2: s[13],
                    c3: s[14],
                    special: s[15] != 0,
                },
            ));
        }
        let n = l(take(8)?) as usize;
        let mut faces = Vec::with_capacity(n.min(1 << 24));
        for _ in 0..n {
            let s = take(16)?;
            faces.push(Face {
                v: [u(&s[0..4]), u(&s[4..8]), u(&s[8..12])],
                label: u(&s[12..16]),
                id: 0,
            });
        }
        Ok(Self {
            to,
            from,
            published,
            labels,
            vertices,
            faces,
        })
    }
}

// ---------------------------------------------------------------------------------------
// Children.

fn load_leaf(ctx: &Ctx, node: &NodeRecord, index: u64, error: f64) -> Result<ChildGeom, JobError> {
    let path = ctx.stage.join("leaves").join(format!("{index:012}.glb"));
    let bytes = fs::read(&path).map_err(|e| JobError::io("read leaf for LOD", &path, e))?;
    let decoded = codec::decode(&bytes)?;
    drop(bytes);
    let table = decoded
        .tables
        .iter()
        .find(|t| t.class == "source_triangle")
        .ok_or_else(|| state("leaf without a source_triangle table"))?;
    let column = |name: &str| {
        table
            .columns
            .get(name)
            .ok_or_else(|| state(format!("leaf source_triangle table without {name}")))
    };
    let (nodes, meshes, prims, tris) = (
        column("node_index")?,
        column("mesh_index")?,
        column("primitive_index")?,
        column("triangle_index")?,
    );
    let mut by_provenance: BTreeMap<(u32, u32, u32, u64), LodSource> = BTreeMap::new();
    for s in ctx.source.range(node.first, node.end, 1 << 16)? {
        let s = s?;
        if by_provenance
            .insert((s.node, s.mesh, s.primitive, s.triangle), s)
            .is_some()
        {
            return Err(state("duplicate source provenance in a leaf range"));
        }
    }
    if decoded.faces.len() as u64 != node.triangles() {
        return Err(state("leaf GLB triangle count differs from its rank range"));
    }
    let depth = node.depth();
    let mut faces = Vec::with_capacity(decoded.faces.len());
    let mut tris_out = Vec::with_capacity(decoded.faces.len());
    let mut labels = BTreeMap::new();
    for face in &decoded.faces {
        let row = face
            .feature(&decoded.tables, "source_triangle")
            .ok_or_else(|| state("leaf triangle without a source_triangle feature ID"))?
            .0 as usize;
        if row >= table.count {
            return Err(state("leaf feature ID outside its source_triangle table"));
        }
        let provenance = (
            nodes[row] as u32,
            meshes[row] as u32,
            prims[row] as u32,
            tris[row],
        );
        let s = by_provenance
            .get(&provenance)
            .ok_or_else(|| state("leaf triangle provenance is not in its rank range"))?;
        // The published bytes must decode to the frozen lattice point of each source
        // corner at the leaf's depth (H8's contract, re-checked on bytes).
        for c in 0..3 {
            let expected = ctx
                .lattice
                .encode(&[s.positions[c]; 3], &[s.info[c].ell; 3], depth)?
                .decoded[0];
            if expected.map(f32::to_bits) != face.p[c].map(f32::to_bits) {
                let matching: Vec<u8> = (0..ctx.lattice.steps().len() as u8)
                    .filter(|&ell| {
                        ctx.lattice
                            .encode(&[s.positions[c]; 3], &[ell; 3], depth)
                            .is_ok_and(|e| e.decoded[0].map(f32::to_bits) == face.p[c].map(f32::to_bits))
                    })
                    .collect();
                return Err(state(format!(
                    "leaf {index}: decoded corner {:?} of source {:?} differs from its frozen \
                     lattice point {:?} (corner depth {}, leaf depth {depth}; depths that \
                     decode to it: {matching:?}; provenance {:?})",
                    face.p[c], s.positions[c], expected, s.info[c].ell, provenance
                )));
            }
        }
        let label = Label {
            key: s.primitive_key(),
            expanded_ordinal: s.expanded_ordinal,
            material: s.material,
        };
        labels.insert(label.key, error);
        faces.push(KeyFace {
            keys: s.positions.map(incidence::weld),
            info: s.info,
            label,
            origin: Origin {
                node: s.node,
                mesh: s.mesh,
                primitive: s.primitive,
                triangle: s.triangle,
            },
        });
        tris_out.push(tri_of(&face.p));
    }
    Ok(ChildGeom {
        slot: slot_of(node),
        faces,
        decoded: tris_out,
        to: error,
        from: error,
        published: error,
        labels,
    })
}

fn load_proxy(ctx: &Ctx, node: &NodeRecord) -> Result<ChildGeom, JobError> {
    let sidecar = Sidecar::read(&sidecar_path(ctx.sidecars, node))?;
    let path = ctx.stage.join(proxy_name(node));
    let bytes = fs::read(&path).map_err(|e| JobError::io("read proxy for LOD", &path, e))?;
    let decoded = codec::decode(&bytes)?;
    if decoded.faces.len() != sidecar.faces.len() {
        return Err(state("proxy GLB face count differs from its sidecar"));
    }
    let depth = node.depth();
    let mut cache: BTreeMap<u32, [f32; 3]> = BTreeMap::new();
    let mut faces = Vec::with_capacity(sidecar.faces.len());
    let mut tris = Vec::with_capacity(sidecar.faces.len());
    for (face, published) in sidecar.faces.iter().zip(&decoded.faces) {
        let mut expected = [[0f32; 3]; 3];
        for c in 0..3 {
            let v = face.v[c];
            let (key, info) = sidecar.vertices[v as usize];
            expected[c] = match cache.get(&v) {
                Some(p) => *p,
                None => {
                    let p = decode_vertex(ctx.lattice, key, info.ell, depth)?;
                    cache.insert(v, p);
                    p
                }
            };
        }
        if !codec::same_face(&expected, &published.p) {
            return Err(state("proxy GLB bytes decode differently from their sidecar"));
        }
        let (label, _) = sidecar.labels[face.label as usize];
        if published.feature(&decoded.tables, "source_primitive").map(|f| f.0) != Some(face.label) {
            return Err(state("proxy feature ID differs from its sidecar label"));
        }
        // Provenance from the published per-triangle table.
        let (row, table) = published
            .feature(&decoded.tables, "proxy_source")
            .ok_or_else(|| state("proxy triangle without a proxy_source feature ID"))?;
        let columns = &decoded.tables[table].columns;
        let column = |name: &str| -> Result<u64, JobError> {
            columns
                .get(name)
                .and_then(|c| c.get(row as usize))
                .copied()
                .ok_or_else(|| state("proxy_source table without its row"))
        };
        faces.push(KeyFace {
            keys: face.v.map(|v| sidecar.vertices[v as usize].0),
            info: face.v.map(|v| sidecar.vertices[v as usize].1),
            label,
            origin: Origin {
                node: column("node_index")? as u32,
                mesh: column("mesh_index")? as u32,
                primitive: column("primitive_index")? as u32,
                triangle: column("triangle_index")?,
            },
        });
        tris.push(tri_of(&published.p));
    }
    Ok(ChildGeom {
        slot: slot_of(node),
        faces,
        decoded: tris,
        to: sidecar.to,
        from: sidecar.from,
        published: sidecar.published,
        labels: sidecar.labels.iter().map(|(l, e)| (l.key, *e)).collect(),
    })
}

// ---------------------------------------------------------------------------------------
// One node.

#[derive(Default)]
struct Outcome {
    row: Option<Row>,
    seams: Vec<SeamEdge>,
    vanished_components: u64,
    vanished_faces: u64,
    fill_candidates: u64,
    fills_rejected_bound: u64,
    fills_rejected_topology: u64,
    forced: u64,
    forced_unchecked: u64,
    sidecar_bytes: u64,
    direct_to_used: bool,
    direct_from_used: bool,
    relaxed: bool,
    /// Seconds: load children, islands/seams, simplify, encode+composed, direct,
    /// per-primitive+fills, final encode/records/sidecar.
    phases: [f64; 7],
    forced_keys: Vec<Key>,
    failed_keys: Vec<Key>,
    over_budget: bool,
}

/// Faces of a mesh grouped by label (stable), the GLB face order.
fn by_label(faces: &[Face]) -> Vec<Face> {
    let mut out = faces.to_vec();
    out.sort_by_key(|f| f.label);
    out
}

/// Build the per-primitive proxy geometry for `faces` (already label-grouped).
fn primitives(mesh: &KeyMesh, faces: &[Face], origins: &[KeyFace]) -> Vec<ProxyPrimitive> {
    let mut out: Vec<ProxyPrimitive> = Vec::new();
    let mut local: BTreeMap<u32, u32> = BTreeMap::new();
    let mut current = u32::MAX;
    for f in faces {
        if f.label != current {
            current = f.label;
            local.clear();
            let l = mesh.labels[f.label as usize];
            out.push(ProxyPrimitive {
                key: l.key,
                expanded_ordinal: l.expanded_ordinal,
                material: l.material,
                positions: Vec::new(),
                ell: Vec::new(),
                faces: Vec::new(),
                origins: Vec::new(),
            });
        }
        let p = out.last_mut().unwrap();
        p.origins.push(match origins.get(f.id as usize) {
            Some(k) => k.origin,
            None => {
                let l = mesh.labels[f.label as usize].key;
                Origin {
                    node: l.0,
                    mesh: l.1,
                    primitive: l.2,
                    triangle: u64::MAX,
                }
            }
        });
        let tri = f.v.map(|v| {
            *local.entry(v).or_insert_with(|| {
                let vertex = mesh.vertices[v as usize];
                p.positions.push(vertex.key.map(f32::from_bits));
                p.ell.push(vertex.info.ell);
                (p.positions.len() - 1) as u32
            })
        });
        p.faces.push(tri);
    }
    out
}

fn process(ctx: &Ctx, job: &Job, keep_extra: &BTreeSet<Key>) -> Result<Outcome, JobError> {
    (ctx.check)()?;
    let mut clock = std::time::Instant::now();
    let node = &job.node;
    let d = node.depth();
    let ar = ctx.ar;
    let mut children = Vec::with_capacity(job.children.len());
    for child in &job.children {
        children.push(match child {
            Child::Leaf { node, index, error } => load_leaf(ctx, node, *index, *error)?,
            Child::Proxy { node } => load_proxy(ctx, node)?,
        });
    }
    let mut key_faces = Vec::new();
    let mut t_tris: Vec<Tri> = Vec::new();
    let mut t_slot: Vec<u8> = Vec::new();
    for c in &children {
        key_faces.extend_from_slice(&c.faces);
        t_tris.extend_from_slice(&c.decoded);
        t_slot.extend(std::iter::repeat_n(c.slot, c.faces.len()));
    }
    let mut lap = |slot: &mut f64| {
        let now = std::time::Instant::now();
        *slot += (now - clock).as_secs_f64();
        clock = now;
    };
    let mut phases = [0f64; 7];
    lap(&mut phases[0]);
    let mesh = KeyMesh::build(&key_faces);
    let mut out = Outcome::default();

    // Islands: whole components with no cut vertex and a zero area-share target.
    let comps = mesh::components(&mesh, d);
    let total: f64 = comps.iter().map(|c| c.area).sum();
    let mut vanish: Vec<bool> = comps
        .iter()
        .map(|c| !c.cut && c.area * ctx.target as f64 <= total && total > 0.)
        .collect();
    if vanish.iter().all(|v| *v) {
        let keep = (0..comps.len())
            .max_by(|&a, &b| {
                comps[a]
                    .area
                    .total_cmp(&comps[b].area)
                    .then(comps[b].min_key.cmp(&comps[a].min_key))
            })
            .unwrap();
        vanish[keep] = false;
    }
    // Policy (not soundness: the certificate measures whatever vanishes): an island may
    // vanish only when it lies within the tile's natural spacing, diagonal / sqrt(P), of
    // the surface that stays, so one far floater cannot dominate a tile's error.
    if vanish.iter().any(|v| *v) {
        let lo = node.bounds.min.map(f64::from);
        let hi = node.bounds.max.map(f64::from);
        let diagonal = ((0..3).map(|a| (hi[a] - lo[a]).powi(2)).sum::<f64>()).sqrt();
        let spacing = diagonal / (ctx.target as f64).sqrt();
        let staying: Vec<Tri> = comps
            .iter()
            .zip(&vanish)
            .filter(|(_, v)| !**v)
            .flat_map(|(c, _)| c.faces.iter().map(|&f| t_tris[f as usize]))
            .collect();
        let bvh = Bvh::new(&staying);
        let mut evals = 0;
        for (c, v) in comps.iter().zip(vanish.iter_mut()) {
            if !*v {
                continue;
            }
            let far = c.faces.iter().any(|&f| {
                t_tris[f as usize]
                    .iter()
                    .any(|p| bvh.nearest(*p, 64, &mut evals).sqrt() > spacing)
            });
            if far {
                *v = false;
            }
        }
    }
    let mut removed_face = vec![false; mesh.faces.len()];
    let mut vanished_rows: Vec<(u32, Key, u32, u8)> = Vec::new(); // label, key, faces, mask
    for (c, gone) in comps.iter().zip(&vanish) {
        if !*gone {
            continue;
        }
        out.vanished_components += 1;
        let mut per_label: BTreeMap<u32, (u32, u8)> = BTreeMap::new();
        for &f in &c.faces {
            removed_face[f as usize] = true;
            let e = per_label.entry(mesh.faces[f as usize].label).or_default();
            e.0 += 1;
            e.1 |= 1 << t_slot[f as usize];
        }
        out.vanished_faces += c.faces.len() as u64;
        for (label, (count, mask)) in per_label {
            vanished_rows.push((label, c.min_key, count, mask));
        }
    }
    let kept_faces: Vec<Face> = mesh
        .faces
        .iter()
        .zip(&removed_face)
        .filter(|(_, r)| !**r)
        .map(|(f, _)| *f)
        .collect();

    // Seam chains, then simplification.
    let k = ctx.retention.get(d as usize).copied().unwrap_or(0);
    let mut locked = vec![false; mesh.vertices.len()];
    let mut forced = Vec::new();
    for (i, v) in mesh.vertices.iter().enumerate() {
        if !v.info.cut(d) {
            continue;
        }
        let mut keep = kept(&v.info, v.key, d, k) || keep_extra.contains(&v.key);
        // Control: the first tile of each depth removes a third of the seam vertices every
        // other tile keeps, ignoring the keep set (a crack consensus cannot heal).
        if matches!(ctx.mutation, Mutation::InjectCrack | Mutation::CrackAsHole)
            && node.first == 0
            && d >= 1
            && key_hash(v.key) % 3 == 0
            && !v.info.special
        {
            keep = false;
        }
        if keep {
            locked[i] = true;
        } else {
            forced.push(i as u32);
        }
    }
    lap(&mut phases[1]);
    let mut simplifier = Simplifier::new(&mesh, &kept_faces, locked);
    let failed = simplifier.force(&forced);
    simplifier.set_seam(
        mesh.vertices
            .iter()
            .map(|v| v.info.cut(d) && !v.info.special)
            .collect(),
        mesh.vertices.iter().map(|v| v.info.cut(d)).collect(),
    );
    out.forced_keys = forced.iter().map(|&i| mesh.vertices[i as usize].key).collect();
    out.failed_keys = failed.iter().map(|&i| mesh.vertices[i as usize].key).collect();
    // At most P faces, and at least a halving of the children (single-child chains and
    // small subtrees still coarsen level by level).
    let target = ctx.target.min(kept_faces.len().div_ceil(2)).max(1);
    simplifier.simplify(target);
    let strict_faces = simplifier.live_faces();
    let relaxed = strict_faces > 2 * ctx.target;
    if relaxed {
        simplifier.relax();
        simplifier.simplify(target);
    }
    let (s_faces, report) = simplifier.finish();
    out.relaxed = relaxed;
    out.forced = report.forced;
    out.forced_unchecked = report.forced_unchecked;
    out.over_budget = s_faces.len() > 2 * ctx.target;
    if std::env::var_os("RT_LOD_BUDGET_DEBUG").is_some() && out.over_budget {
        let cut = mesh.vertices.iter().filter(|v| v.info.cut(d)).count();
        let special = mesh.vertices.iter().filter(|v| v.info.special).count();
        let used: BTreeSet<u32> = s_faces.iter().flat_map(|f| f.v).collect();
        let used_cut = used.iter().filter(|&&v| mesh.vertices[v as usize].info.cut(d)).count();
        let used_special = used.iter().filter(|&&v| mesh.vertices[v as usize].info.special).count();
        eprintln!("BUDGET f{}-d{d}: T faces {} verts {} cut {cut} special {special} kept-extra {} forced {} failed {}; strict {strict_faces} final {} used verts {} (cut {used_cut}, special {used_special}) report {:?}",
            node.first, mesh.faces.len(), mesh.vertices.len(), keep_extra.len(), forced.len(), failed.len(), s_faces.len(), used.len(), report);
    }
    if s_faces.len() > 4 * ctx.target {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            format!(
                "coarse tile f{}-d{d}: {} faces remain after simplification, above the \
                 {} limit (locked seam or non-manifold vertices)",
                node.first,
                s_faces.len(),
                4 * ctx.target
            ),
        ));
    }
    if s_faces.is_empty() {
        return Err(state(format!("coarse tile f{}-d{d} simplified to nothing", node.first)));
    }
    // Accounting: every label of T is present in S or recorded as vanished.
    let present: BTreeSet<u32> = s_faces.iter().map(|f| f.label).collect();
    let recorded: BTreeSet<u32> = vanished_rows.iter().map(|r| r.0).collect();
    let mut lost: BTreeMap<u32, (u32, Key, u8)> = BTreeMap::new();
    for (i, f) in mesh.faces.iter().enumerate() {
        if !present.contains(&f.label) && !recorded.contains(&f.label) {
            let e = lost.entry(f.label).or_insert((0, [u32::MAX; 3], 0));
            e.0 += 1;
            for &v in &f.v {
                e.1 = e.1.min(mesh.vertices[v as usize].key);
            }
            e.2 |= 1 << t_slot[i];
        }
    }

    // Encode the unfilled proxy, decode its bytes, certify.
    lap(&mut phases[2]);
    let s_order = by_label(&s_faces);
    let flat_records: Vec<RecordTable> = Vec::new();
    let unfilled = codec::encode(&primitives(&mesh, &s_order, &key_faces), ctx.materials, ctx.lattice, d, &flat_records)?;
    let s_dec = codec::decode(&unfilled.glb)?;
    if s_dec.faces.len() != unfilled.faces.len()
        || !s_dec
            .faces
            .iter()
            .zip(&unfilled.faces)
            .all(|(a, b)| codec::same_face(&a.p, b))
    {
        return Err(state("proxy bytes decode differently from the encoder's lattice"));
    }
    let s_tris: Vec<Tri> = s_dec.faces.iter().map(|f| tri_of(&f.p)).collect();
    let s_bvh = Bvh::new(&s_tris);
    let t_bvh = Bvh::new(&t_tris);
    let child_to = children.iter().map(|c| c.to).fold(0f64, f64::max);
    let child_from = children.iter().map(|c| c.from).fold(0f64, f64::max);
    let child_max = children.iter().map(|c| c.published).fold(0f64, f64::max);
    let s_labels: Vec<u32> = s_order.iter().map(|f| f.label).collect();
    let t_labels: Vec<u32> = mesh.faces.iter().map(|f| f.label).collect();
    let s_t = geom::directed_labelled(
        ar,
        &s_tris,
        &t_bvh,
        Limits::for_sizes(s_tris.len(), t_tris.len()),
        Some((&s_labels, &t_labels)),
    );
    let skip_removed = ctx.mutation == Mutation::SkipRemoved;
    let t_measured: Vec<Tri> = if skip_removed {
        t_tris
            .iter()
            .zip(&removed_face)
            .filter(|(_, r)| !**r)
            .map(|(t, _)| *t)
            .collect()
    } else {
        t_tris.clone()
    };
    let t_s = geom::directed(ar, &t_measured, &s_bvh, Limits::for_sizes(t_measured.len(), s_tris.len()));
    let composed_to = up_add(ar, s_t.bound, child_to);
    let composed_from = up_add(ar, child_from, t_s.bound);
    let removed_keys: BTreeSet<Key> = if skip_removed {
        mesh.faces
            .iter()
            .zip(&removed_face)
            .filter(|(_, r)| **r)
            .flat_map(|(f, _)| f.v.map(|v| mesh.vertices[v as usize].key))
            .collect()
    } else {
        BTreeSet::new()
    };
    let skip = |s: &LodSource| {
        skip_removed && s.positions.iter().all(|p| removed_keys.contains(&incidence::weld(*p)))
    };
    let lo = node.bounds.min.map(f64::from);
    let hi = node.bounds.max.map(f64::from);
    // Direct terms where they can differ from the composed ones: above leaf-only children
    // T is the decoded source itself (up to the leaves' own snap bound).
    let leaf_parent = job.children.iter().all(|c| matches!(c, Child::Leaf { .. }));
    let work = if leaf_parent {
        0
    } else {
        256 * (s_tris.len() as u64 + t_tris.len() as u64)
    };
    lap(&mut phases[3]);
    let direct = certify::direct(
        ar, ctx.source, node.first, node.end, lo, hi, &s_tris, &s_bvh, work, &skip, ctx.check,
    )?;
    let direct_to = direct.proxy_to_source.unwrap_or(f64::INFINITY);
    let direct_from = direct.source_to_proxy;
    let to = composed_to.min(direct_to);
    let from = composed_from.min(direct_from);
    out.direct_to_used = direct_to < composed_to;
    out.direct_from_used = direct_from < composed_from;
    let open = to.max(from);
    if !open.is_finite() {
        return Err(state(format!("coarse tile f{}-d{d}: certificate is not finite", node.first)));
    }

    lap(&mut phases[4]);
    // Per-primitive proxy -> source bounds (composed through each child's own bound).
    let mut t_by_label: BTreeMap<u32, Vec<Tri>> = BTreeMap::new();
    for (f, t) in mesh.faces.iter().zip(&t_tris) {
        t_by_label.entry(f.label).or_default().push(*t);
    }
    let label_child: BTreeMap<u32, f64> = mesh
        .labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let e = children
                .iter()
                .filter_map(|c| c.labels.get(&l.key))
                .copied()
                .fold(0f64, f64::max);
            (i as u32, e)
        })
        .collect();
    let mut label_bvh: BTreeMap<u32, Bvh> = BTreeMap::new();
    let mut label_error: BTreeMap<u32, f64> = BTreeMap::new();
    {
        // Same-primitive witnesses come from the S -> T search; a face whose search met no
        // same-primitive target gets one filtered query.
        let mut per_label: BTreeMap<u32, f64> = BTreeMap::new();
        let mut evals = 0u64;
        for ((f, t), &same) in s_order.iter().zip(&s_tris).zip(&s_t.same) {
            let bound = if same.is_finite() {
                same
            } else {
                let bvh = label_bvh
                    .entry(f.label)
                    .or_insert_with(|| Bvh::new(&t_by_label[&f.label]));
                bvh.best_target(t, 64, &mut evals)
                    .map_or(f64::INFINITY, |(k, _)| ar.patch(t, bvh.tri(k), 0.))
            };
            let e = per_label.entry(f.label).or_insert(0.);
            *e = e.max(bound);
        }
        for (label, h) in per_label {
            label_error.insert(label, up_add(ar, h, label_child[&label]));
        }
    }

    // Fills: simple interior loops, accepted only within the certified error.
    let crack_as_hole = ctx.mutation == Mutation::CrackAsHole;
    let loops = mesh::border_loops(
        &s_order,
        |v| crack_as_hole || !mesh.vertices[v as usize].info.cut(d),
        FILL_LOOP,
    );
    let mut edges: BTreeSet<(u32, u32)> = BTreeSet::new();
    for f in &s_order {
        for k in 0..3 {
            let (a, b) = (f.v[k], f.v[(k + 1) % 3]);
            edges.insert((a.min(b), a.max(b)));
        }
    }
    let decoded_of: BTreeMap<u32, [f32; 3]> = {
        let mut m = BTreeMap::new();
        for (f, dec) in s_order.iter().zip(&unfilled.faces) {
            for c in 0..3 {
                m.insert(f.v[c], dec[c]);
            }
        }
        m
    };
    let mut fill_faces: Vec<Face> = Vec::new();
    let mut fill_rows: Vec<(u32, Key, u32, u32, f64, u8)> = Vec::new();
    let mut to_filled = to;
    for l in &loops {
        out.fill_candidates += 1;
        let fan = mesh::fan(l);
        let loop_edges: BTreeSet<(u32, u32)> = (0..l.vertices.len())
            .map(|i| {
                let (a, b) = (l.vertices[i], l.vertices[(i + 1) % l.vertices.len()]);
                (a.min(b), a.max(b))
            })
            .collect();
        let diagonal_exists = fan.iter().any(|t| {
            (0..3).any(|k| {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let e = (a.min(b), a.max(b));
                !loop_edges.contains(&e) && edges.contains(&e)
            })
        });
        let fan_tris: Vec<[[f32; 3]; 3]> = fan.iter().map(|t| t.map(|v| decoded_of[&v])).collect();
        let degenerate = fan_tris.iter().any(|t| {
            let b = t.map(|p| p.map(f32::to_bits));
            b[0] == b[1] || b[1] == b[2] || b[0] == b[2] || {
                let n = mesh::normal(tri_of(t));
                n == [0.; 3]
            }
        });
        if diagonal_exists || degenerate {
            out.fills_rejected_topology += 1;
            continue;
        }
        let tris: Vec<Tri> = fan_tris.iter().map(tri_of).collect();
        let bvh = label_bvh
            .entry(l.label)
            .or_insert_with(|| Bvh::new(&t_by_label[&l.label]));
        let h = geom::directed(ar, &tris, bvh, Limits::for_sizes(tris.len(), bvh.len()));
        let bound = up_add(ar, h.bound, label_child[&l.label]);
        if !(bound <= open) {
            out.fills_rejected_bound += 1;
            continue;
        }
        to_filled = to_filled.max(bound);
        let e = label_error.entry(l.label).or_insert(0.);
        *e = e.max(bound);
        let min_key = l
            .vertices
            .iter()
            .map(|&v| mesh.vertices[v as usize].key)
            .min()
            .unwrap();
        let mut mask = 0u8;
        for (f, slot) in mesh.faces.iter().zip(&t_slot) {
            if f.v.iter().any(|v| l.vertices.contains(v)) {
                mask |= 1 << slot;
            }
        }
        fill_rows.push((l.label, min_key, l.vertices.len() as u32, fan.len() as u32, bound, mask));
        for t in fan {
            fill_faces.push(Face { v: t, label: l.label, id: u32::MAX });
        }
    }
    let raw = to_filled.max(from);
    if raw > open {
        return Err(state("an accepted fill raised the certified error"));
    }
    let published = raw.max(child_max);

    lap(&mut phases[5]);
    // Final encoding with fills and records; decode and compare once more.
    let mut all = s_order.clone();
    all.extend_from_slice(&fill_faces);
    let final_order = by_label(&all);
    let label_rows = |rows: &[(u32, Key, u32, u8)], reason: u64| -> Vec<[u64; 9]> {
        rows.iter()
            .map(|(label, key, faces, mask)| {
                let l = mesh.labels[*label as usize].key;
                [
                    u64::from(l.0),
                    u64::from(l.1),
                    u64::from(l.2),
                    u64::from(key[0]),
                    u64::from(key[1]),
                    u64::from(key[2]),
                    u64::from(*faces),
                    u64::from(*mask),
                    reason,
                ]
            })
            .collect()
    };
    let mut v_rows = label_rows(&vanished_rows, 0);
    let lost_rows: Vec<(u32, Key, u32, u8)> = lost.iter().map(|(l, (n, k, m))| (*l, *k, *n, *m)).collect();
    v_rows.extend(label_rows(&lost_rows, 1));
    let mut records = Vec::new();
    if !v_rows.is_empty() {
        let col = |i: usize| v_rows.iter().map(|r| r[i]).collect::<Vec<_>>();
        records.push(RecordTable {
            class: "lod_vanished",
            columns: vec![
                ("node_index", "UINT32", col(0)),
                ("mesh_index", "UINT32", col(1)),
                ("primitive_index", "UINT32", col(2)),
                ("component_x", "UINT32", col(3)),
                ("component_y", "UINT32", col(4)),
                ("component_z", "UINT32", col(5)),
                ("faces", "UINT32", col(6)),
                ("reopen_children", "UINT8", col(7)),
                ("reason", "UINT8", col(8)),
                ("depth", "UINT8", vec![u64::from(d); v_rows.len()]),
            ],
        });
    }
    if !fill_rows.is_empty() {
        let l = |r: &(u32, Key, u32, u32, f64, u8)| mesh.labels[r.0 as usize].key;
        records.push(RecordTable {
            class: "lod_filled",
            columns: vec![
                ("node_index", "UINT32", fill_rows.iter().map(|r| u64::from(l(r).0)).collect()),
                ("mesh_index", "UINT32", fill_rows.iter().map(|r| u64::from(l(r).1)).collect()),
                ("primitive_index", "UINT32", fill_rows.iter().map(|r| u64::from(l(r).2)).collect()),
                ("loop_x", "UINT32", fill_rows.iter().map(|r| u64::from(r.1[0])).collect()),
                ("loop_y", "UINT32", fill_rows.iter().map(|r| u64::from(r.1[1])).collect()),
                ("loop_z", "UINT32", fill_rows.iter().map(|r| u64::from(r.1[2])).collect()),
                ("loop_vertices", "UINT32", fill_rows.iter().map(|r| u64::from(r.2)).collect()),
                ("fan_faces", "UINT32", fill_rows.iter().map(|r| u64::from(r.3)).collect()),
                ("bound", "FLOAT64", fill_rows.iter().map(|r| r.4.to_bits()).collect()),
                ("reopen_children", "UINT8", fill_rows.iter().map(|r| u64::from(r.5)).collect()),
                ("depth", "UINT8", vec![u64::from(d); fill_rows.len()]),
            ],
        });
    }
    let encoded = codec::encode(&primitives(&mesh, &final_order, &key_faces), ctx.materials, ctx.lattice, d, &records)?;
    let check = codec::decode(&encoded.glb)?;
    if check.faces.len() != encoded.faces.len()
        || !check
            .faces
            .iter()
            .zip(&encoded.faces)
            .all(|(a, b)| codec::same_face(&a.p, b))
    {
        return Err(state("final proxy bytes decode differently from the encoder's lattice"));
    }
    // Every certified face is in the published bytes (fills added, nothing else changed).
    {
        let bits = |t: &[[f32; 3]; 3]| {
            let mut b = t.map(|p| p.map(f32::to_bits));
            b.sort_unstable();
            b
        };
        let mut published_faces: Vec<_> = check.faces.iter().map(|f| bits(&f.p)).collect();
        let mut certified: Vec<_> = unfilled.faces.iter().map(bits).collect();
        for f in &fill_faces {
            certified.push(bits(&f.v.map(|v| decoded_of[&v])));
        }
        published_faces.sort_unstable();
        certified.sort_unstable();
        if published_faces != certified {
            return Err(state("published proxy faces differ from the certified faces"));
        }
    }

    // Seam edges for the depth audit: border edges with both endpoints cut.
    let mut count: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for f in &final_order {
        for k in 0..3 {
            let (a, b) = (f.v[k], f.v[(k + 1) % 3]);
            if a != b {
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
    }
    for ((a, b), n) in count {
        let (va, vb) = (mesh.vertices[a as usize], mesh.vertices[b as usize]);
        if n != 1 || !va.info.cut(d) || !vb.info.cut(d) {
            continue;
        }
        let (lo, hi) = if va.key <= vb.key { (va, vb) } else { (vb, va) };
        let dec = |v: &mesh::Vertex| -> Result<[u32; 3], JobError> {
            Ok(decode_vertex(ctx.lattice, v.key, v.info.ell, d)?.map(f32::to_bits))
        };
        out.seams.push(SeamEdge {
            lo: lo.key,
            hi: hi.key,
            owner: node.first,
            owner_depth: d as u8,
            lo_dec: dec(&lo)?,
            hi_dec: dec(&hi)?,
            special: lo.info.special && hi.info.special,
            round: 0,
        });
    }

    // Sidecar (key space, GLB face order) and the published member.
    let mut used: BTreeMap<u32, u32> = BTreeMap::new();
    let mut vertices = Vec::new();
    let mut labels_used: BTreeMap<u32, u32> = BTreeMap::new();
    let mut labels = Vec::new();
    let mut sidecar_faces = Vec::with_capacity(final_order.len());
    for f in &final_order {
        let label = *labels_used.entry(f.label).or_insert_with(|| {
            labels.push((mesh.labels[f.label as usize], label_error.get(&f.label).copied().unwrap_or(published)));
            (labels.len() - 1) as u32
        });
        let v = f.v.map(|v| {
            *used.entry(v).or_insert_with(|| {
                let x = mesh.vertices[v as usize];
                vertices.push((x.key, x.info));
                (vertices.len() - 1) as u32
            })
        });
        sidecar_faces.push(Face { v, label, id: 0 });
    }
    out.sidecar_bytes = Sidecar {
        to: to_filled,
        from,
        published,
        labels,
        vertices,
        faces: sidecar_faces,
    }
    .write(&sidecar_path(ctx.sidecars, node))?;
    let member = ctx.stage.join(proxy_name(node));
    fs::write(&member, &encoded.glb).map_err(|e| JobError::io("write proxy", &member, e))?;
    let flags = if s_t.exhausted || t_s.exhausted {
        FLAG_COMPOSED_EXHAUSTED
    } else {
        0
    } | if direct.source_to_proxy_exhausted || direct.proxy_to_source_exhausted {
        FLAG_DIRECT_EXHAUSTED
    } else {
        0
    } | if direct.skipped {
        FLAG_DIRECT_SKIPPED
    } else {
        0
    };
    out.row = Some(Row {
        first: node.first,
        end: node.end,
        depth: d as u8,
        published,
        to: to_filled,
        from,
        composed_to,
        composed_from,
        direct_to,
        direct_from,
        child_max,
        faces: final_order.len() as u32,
        primitives: labels_used.len() as u32,
        vanished: v_rows.len() as u32,
        filled: fill_rows.len() as u32,
        bytes: encoded.glb.len() as u64,
        sha256: Sha256::digest(&encoded.glb).into(),
        flags,
    });
    lap(&mut phases[6]);
    out.phases = phases;
    Ok(out)
}

// ---------------------------------------------------------------------------------------
// Leaf seam edges for the frontier audit (leaves at depth <= d).

fn leaf_seams(
    source: &Sequence<LodSource>,
    lattice: &LatticePlan,
    leaf: &NodeRecord,
    d: u32,
    out: &mut crate::spill::SequenceWriter<SeamEdge>,
) -> Result<u64, JobError> {
    let mut count: BTreeMap<(Key, Key), u32> = BTreeMap::new();
    let mut info: BTreeMap<Key, VertexInfo> = BTreeMap::new();
    for s in source.range(leaf.first, leaf.end, 1 << 16)? {
        let s = s?;
        let keys = s.positions.map(incidence::weld);
        for c in 0..3 {
            info.insert(keys[c], s.info[c]);
            let (a, b) = (keys[c], keys[(c + 1) % 3]);
            if a != b {
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
    }
    let mut n = 0;
    for ((a, b), c) in count {
        let (ia, ib) = (info[&a], info[&b]);
        if c != 1 || !ia.cut(d) || !ib.cut(d) {
            continue;
        }
        let dec = |k: Key, i: VertexInfo| -> Result<[u32; 3], JobError> {
            Ok(decode_vertex(lattice, k, i.ell, leaf.depth())?.map(f32::to_bits))
        };
        out.push(&SeamEdge {
            lo: a,
            hi: b,
            owner: leaf.first,
            owner_depth: leaf.depth() as u8,
            lo_dec: dec(a, ia)?,
            hi_dec: dec(b, ib)?,
            special: ia.special && ib.special,
            round: 0,
        })?;
        n += 1;
    }
    Ok(n)
}

#[derive(Default)]
struct AuditCounts {
    edges: u64,
    matched: u64,
    boundary: u64,
    relabelled: u64,
    /// Proxy tiles with an edge no other frontier tile has, and one such edge.
    unmatched: BTreeMap<(u64, u8), (Key, Key)>,
}

fn audit(
    sorted: impl Iterator<Item = Result<SeamEdge, crate::spill::SpillError>>,
    d: u32,
    relabel: bool,
) -> Result<AuditCounts, JobError> {
    let mut counts = AuditCounts::default();
    let mut group: Vec<SeamEdge> = Vec::new();
    let finish = |group: &mut Vec<SeamEdge>, counts: &mut AuditCounts| -> Result<(), JobError> {
        let Some(first) = group.first().copied() else {
            return Ok(());
        };
        counts.edges += 1;
        let owners: BTreeSet<(u64, u8)> = group.iter().map(|e| (e.owner, e.owner_depth)).collect();
        if group
            .iter()
            .any(|e| e.lo_dec != first.lo_dec || e.hi_dec != first.hi_dec)
        {
            return Err(state(format!(
                "depth {d} seam: tiles decode a shared edge differently (crack)"
            )));
        }
        if owners.len() >= 2 {
            counts.matched += 1;
        } else if first.special {
            counts.boundary += 1;
        } else if relabel {
            counts.relabelled += 1;
        } else {
            counts
                .unmatched
                .entry((first.owner, first.owner_depth))
                .or_insert((first.lo, first.hi));
        }
        group.clear();
        Ok(())
    };
    for e in sorted {
        let e = e?;
        if group.first().is_some_and(|g| (g.lo, g.hi) != (e.lo, e.hi)) {
            finish(&mut group, &mut counts)?;
        }
        group.push(e);
    }
    finish(&mut group, &mut counts)?;
    Ok(counts)
}

// ---------------------------------------------------------------------------------------
// The stage.

/// Per-leaf facts parsed once from `leaves.jsonl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LeafFact {
    first: u64,
    error_bits: u64,
}
impl Record for LeafFact {
    const SIZE: usize = 16;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.first.to_le_bytes());
        out[8..].copy_from_slice(&self.error_bits.to_le_bytes());
    }
    fn decode(b: &[u8]) -> Self {
        Self {
            first: u64::from_le_bytes(b[..8].try_into().unwrap()),
            error_bits: u64::from_le_bytes(b[8..].try_into().unwrap()),
        }
    }
}

pub(crate) struct LodInput<'a> {
    pub stage: &'a Path,
    pub captured: &'a FullCaptured,
    pub scratch: &'a Scratch,
    pub sort: crate::spill::SortConfig,
    pub jobs: usize,
    pub mutation: Mutation,
    /// Proxy face target `P` ([`TARGET_FACES`] in production).
    pub target: usize,
}

/// Build, certify and write every coarse tile; returns the report. Writes `proxies/` and
/// `proxies.jsonl` (postorder) into the leaf directory `stage`.
pub(crate) fn produce(
    input: LodInput<'_>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<LodReport, JobError> {
    let started = std::time::Instant::now();
    let captured = input.captured;
    let mut report = LodReport {
        schema_version: 1,
        target_faces: input.target as u64,
        texture: "untextured: flat material factors and smooth normals; texture baking is H13",
        jobs: input.jobs as u64,
        ..Default::default()
    };
    // The frozen lattice: recompute from the same frozen inputs, require equality, bind.
    let height = captured.tree.stats.height as usize;
    let mut extents = vec![0.0; height];
    for s in &captured.tree.stats.per_depth {
        extents[s.depth as usize] = s.maximum_extent;
    }
    let recomputed = LatticePlan::new(captured.lattice.profile(), captured.scan.bounds, &extents)?;
    let frozen = serde_json::to_vec(&captured.lattice).map_err(|e| state(e.to_string()))?;
    if serde_json::to_vec(&recomputed).map_err(|e| state(e.to_string()))? != frozen {
        return Err(state("the frozen H8 lattice differs from its recomputation"));
    }
    report.lattice_sha256 = hex(Sha256::digest(&frozen));
    let proxies_dir = input.stage.join("proxies");
    fs::create_dir_all(&proxies_dir).map_err(|e| JobError::io("create proxy folder", &proxies_dir, e))?;
    // Leaf facts in leaf order.
    let leaf_facts = {
        let path = input.stage.join("leaves.jsonl");
        let text = fs::File::open(&path).map_err(|e| JobError::io("read leaf rows", &path, e))?;
        let mut writer = crate::spill::SequenceWriter::<LeafFact>::new(input.scratch, "lod-leaves", 1 << 16)?;
        for line in std::io::BufRead::lines(std::io::BufReader::new(text)) {
            let line = line.map_err(|e| JobError::io("read leaf rows", &path, e))?;
            let row: Value = serde_json::from_str(&line).map_err(|e| state(e.to_string()))?;
            let error = row["geometryError"].as_f64().ok_or_else(|| state("leaf row without error"))?;
            writer.push(&LeafFact {
                first: row["first"].as_u64().ok_or_else(|| state("leaf row without first"))?,
                error_bits: error.to_bits(),
            })?;
        }
        writer.finish()?
    };
    // Depth statistics for the seam retention exponents.
    let mut sums = vec![(0u128, 0u128); height];
    let mut internal = 0u64;
    for n in captured.tree.nodes.reader()? {
        let n = n?;
        if n.kind == NodeKind::Internal {
            sums[n.depth() as usize].0 += u128::from(n.triangles());
            sums[n.depth() as usize].1 += 1;
            internal += 1;
        }
    }
    report.retention = retention(&sums, input.target as u128);
    let rows_path = input.stage.join("proxies.jsonl");
    if internal == 0 {
        fs::write(&rows_path, b"").map_err(|e| JobError::io("write proxy rows", &rows_path, e))?;
        report.seconds = started.elapsed().as_secs_f64();
        return Ok(report);
    }
    crate::spill::preflight(
        input.scratch.path(),
        incidence::scratch_need(captured.scan.triangles).ok_or_else(|| resource("LOD scratch overflow"))?,
        crate::spill::SpaceRole::Scratch,
        crate::spill::PreflightStage::Lod,
    )?;
    let prepass = std::time::Instant::now();
    let source = incidence::build(
        input.scratch,
        input.sort,
        &captured.tree.nodes,
        &captured.triangles,
        &captured.corner_depths,
        check,
    )?;
    report.prepass_seconds = prepass.elapsed().as_secs_f64();
    let materials: Vec<Value> = captured.materials.iter().map(|m| m.flat_proxy()).collect();
    let sidecars = input.scratch.path().join("lod-sidecars");
    fs::create_dir_all(&sidecars).map_err(|e| JobError::io("create LOD sidecars", &sidecars, e))?;
    let ctx = Ctx {
        stage: input.stage,
        sidecars: &sidecars,
        source: &source,
        lattice: &captured.lattice,
        materials: &materials,
        retention: &report.retention.clone(),
        target: input.target,
        mutation: input.mutation,
        ar: Arith::new(input.mutation),
        check,
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(input.jobs.max(1))
        .thread_name(|i| format!("rt-lod-{i}"))
        .build()
        .map_err(|e| resource(format!("LOD worker pool: {e}")))?;
    let mut rows = ExternalSorter::<Row>::new(input.scratch, input.sort)?;
    let deepest = sums.iter().rposition(|s| s.1 > 0).unwrap_or(0);
    report.max_error_by_depth = vec![0.; deepest + 1];
    let mut sidecar_live = 0u64;
    let mut sidecar_bytes: BTreeMap<u32, u64> = BTreeMap::new();
    for d in (0..=deepest as u32).rev() {
        check()?;
        let mut seam_log =
            crate::spill::SequenceWriter::<SeamEdge>::new(input.scratch, "lod-seams", 1 << 16)?;
        let mut forced_log =
            crate::spill::SequenceWriter::<ForcedKey>::new(input.scratch, "lod-forced", 1 << 16)?;
        let mut done: BTreeMap<u64, Done> = BTreeMap::new();
        let mut keep: BTreeSet<Key> = BTreeSet::new();
        let mut latest: BTreeMap<(u64, u8), u8> = BTreeMap::new();
        let mut failed: BTreeSet<Key> = BTreeSet::new();
        let window_size = 4 * input.jobs.max(1);
        let relabel = input.mutation == Mutation::CrackAsHole;
        let mut round = 0u8;
        let mut selected: Option<BTreeSet<u64>> = None;
        let counts = loop {
            // List this depth's internal tiles (all, or the re-run selection) and their
            // children from the preorder stream, and process them in windows.
            let mut nodes = captured.tree.nodes.reader()?;
            let mut leaves = leaf_facts.reader()?;
            let mut leaf_index = 0u64;
            let mut window: Vec<Job> = Vec::new();
            let mut current: Option<Job> = None;
            let wanted = |n: &NodeRecord| selected.as_ref().is_none_or(|s| s.contains(&n.first));
            let run = |window: &mut Vec<Job>,
                       seam_log: &mut crate::spill::SequenceWriter<SeamEdge>,
                       forced_log: &mut crate::spill::SequenceWriter<ForcedKey>,
                       done: &mut BTreeMap<u64, Done>,
                       failed: &mut BTreeSet<Key>|
             -> Result<(), JobError> {
                if window.is_empty() {
                    return Ok(());
                }
                let outcomes: Vec<Result<Outcome, JobError>> = pool.install(|| {
                    use rayon::prelude::*;
                    window.par_iter().map(|job| process(&ctx, job, &keep)).collect()
                });
                for (job, outcome) in window.iter().zip(outcomes) {
                    let mut o = outcome?;
                    for mut e in std::mem::take(&mut o.seams) {
                        e.round = round;
                        seam_log.push(&e)?;
                    }
                    if round == 0 {
                        for key in std::mem::take(&mut o.forced_keys) {
                            forced_log.push(&ForcedKey { key, first: job.node.first })?;
                        }
                    }
                    failed.extend(std::mem::take(&mut o.failed_keys));
                    done.insert(job.node.first, Done(o));
                }
                window.clear();
                Ok(())
            };
            while let Some(n) = nodes.next().transpose()? {
                if n.kind == NodeKind::Leaf {
                    let fact = leaves
                        .next()
                        .transpose()?
                        .ok_or_else(|| state("leaf facts end before the tree"))?;
                    if fact.first != n.first {
                        return Err(state("leaf facts out of tree order"));
                    }
                    if round == 0 && n.depth() <= d {
                        leaf_seams(&source, &captured.lattice, &n, d, &mut seam_log)?;
                    }
                    if n.depth() == d + 1 {
                        if let Some(job) = current.as_mut() {
                            job.children.push(Child::Leaf {
                                node: n,
                                index: leaf_index,
                                error: f64::from_bits(fact.error_bits),
                            });
                        }
                    }
                    leaf_index += 1;
                    continue;
                }
                if n.depth() <= d {
                    if let Some(job) = current.take() {
                        window.push(job);
                        if window.len() >= window_size {
                            run(&mut window, &mut seam_log, &mut forced_log, &mut done, &mut failed)?;
                        }
                    }
                }
                if n.depth() == d {
                    current = wanted(&n).then(|| Job {
                        node: n,
                        children: Vec::new(),
                    });
                } else if n.depth() == d + 1 {
                    if let Some(job) = current.as_mut() {
                        job.children.push(Child::Proxy { node: n });
                    }
                }
            }
            if let Some(job) = current.take() {
                window.push(job);
            }
            run(&mut window, &mut seam_log, &mut forced_log, &mut done, &mut failed)?;
            // Seam consensus. A tile that cannot remove a seam vertex validly keeps it; a
            // seam edge without a partner keeps every seam vertex its tile removed. Kept
            // vertices are kept by every tile sharing them (each re-runs), which always
            // agrees, so this converges unless a tile ignores the keep set.
            let mut contested = std::mem::take(&mut failed);
            let mut audited = None;
            if contested.is_empty() {
                let log = std::mem::replace(
                    &mut seam_log,
                    crate::spill::SequenceWriter::<SeamEdge>::new(input.scratch, "lod-seams", 1 << 16)?,
                )
                .finish()?;
                let mut sorter = ExternalSorter::<SeamEdge>::new(input.scratch, input.sort)?;
                for e in log.reader()? {
                    let e = e?;
                    seam_log.push(&e)?;
                    if latest.get(&(e.owner, e.owner_depth)).copied().unwrap_or(0) == e.round {
                        sorter.push(e)?;
                    }
                }
                let audit_started = std::time::Instant::now();
                let counts = audit(sorter.finish()?, d, relabel)?;
                report.audit_seconds += audit_started.elapsed().as_secs_f64();
                if counts.unmatched.is_empty() {
                    break counts;
                }
                let owners: BTreeSet<u64> = counts
                    .unmatched
                    .keys()
                    .filter(|(_, depth)| u32::from(*depth) == d)
                    .map(|(first, _)| *first)
                    .collect();
                let log = std::mem::replace(
                    &mut forced_log,
                    crate::spill::SequenceWriter::<ForcedKey>::new(input.scratch, "lod-forced", 1 << 16)?,
                )
                .finish()?;
                for f in log.reader()? {
                    let f = f?;
                    if owners.contains(&f.first) && !keep.contains(&f.key) {
                        contested.insert(f.key);
                    }
                    forced_log.push(&f)?;
                }
                audited = Some(counts);
            }
            if contested.is_empty() || round >= 16 {
                let counts = audited.unwrap_or_default();
                let p = |k: Key| k.map(f32::from_bits);
                let ((first, depth), (lo, hi)) = counts
                    .unmatched
                    .iter()
                    .next()
                    .map(|(k, v)| (*k, *v))
                    .unwrap_or(((0, 0), ([0; 3], [0; 3])));
                return Err(state(format!(
                    "depth {d} seam: edge {:?}-{:?} of tile f{first}-d{depth} has no partner \
                     tile and seam consensus cannot resolve it (crack); a crack is never \
                     published as a hole",
                    p(lo),
                    p(hi)
                )));
            }
            round += 1;
            report.seam_rounds += 1;
            report.seam_kept_extra += contested.len() as u64;
            let log = std::mem::replace(
                &mut forced_log,
                crate::spill::SequenceWriter::<ForcedKey>::new(input.scratch, "lod-forced", 1 << 16)?,
            )
            .finish()?;
            let mut rerun = BTreeSet::new();
            for f in log.reader()? {
                let f = f?;
                if contested.contains(&f.key) {
                    rerun.insert(f.first);
                }
                forced_log.push(&f)?;
            }
            for first in &rerun {
                latest.insert((*first, d as u8), round);
            }
            keep.append(&mut contested);
            selected = Some(rerun);
        };
        drop(forced_log);
        drop(seam_log);
        let mut written = 0u64;
        for Done(o) in done.into_values() {
            let row = o.row.ok_or_else(|| state("a coarse tile produced no row"))?;
            report.proxies += 1;
            report.proxy_faces += u64::from(row.faces);
            report.max_proxy_faces = report.max_proxy_faces.max(u64::from(row.faces));
            report.proxy_bytes += row.bytes;
            let e = &mut report.max_error_by_depth[row.depth as usize];
            *e = e.max(row.published);
            report.vanished_components += o.vanished_components;
            report.vanished_faces += o.vanished_faces;
            report.fill_candidates += o.fill_candidates;
            report.fills += u64::from(row.filled);
            report.fills_rejected_bound += o.fills_rejected_bound;
            report.fills_rejected_topology += o.fills_rejected_topology;
            report.forced_collapses += o.forced;
            report.forced_unchecked += o.forced_unchecked;
            report.relaxed_tiles += u64::from(o.relaxed);
            report.over_budget_tiles += u64::from(o.over_budget);
            for (a, b) in report.phase_seconds.iter_mut().zip(o.phases) {
                *a += b;
            }
            report.direct_to_used += u64::from(o.direct_to_used);
            report.direct_from_used += u64::from(o.direct_from_used);
            report.composed_exhausted += u64::from(row.flags & FLAG_COMPOSED_EXHAUSTED != 0);
            report.direct_exhausted += u64::from(row.flags & FLAG_DIRECT_EXHAUSTED != 0);
            report.direct_skipped += u64::from(row.flags & FLAG_DIRECT_SKIPPED != 0);
            written += o.sidecar_bytes;
            rows.push(row)?;
        }
        sidecar_bytes.insert(d, written);
        sidecar_live += written;
        report.sidecar_peak_bytes = report.sidecar_peak_bytes.max(sidecar_live);
        report.seam_edges += counts.edges;
        report.seam_matched += counts.matched;
        report.seam_unmatched_boundary += counts.boundary;
        report.seam_relabelled += counts.relabelled;
        let child_dir = sidecars.join(format!("d{}", d + 1));
        if child_dir.exists() {
            fs::remove_dir_all(&child_dir).map_err(|e| JobError::io("remove LOD sidecars", &child_dir, e))?;
            sidecar_live -= sidecar_bytes.remove(&(d + 1)).unwrap_or(0);
        }
    }
    let _ = fs::remove_dir_all(&sidecars);
    // Rows in postorder (end ascending, deeper first): H16's tile-closing order.
    let file = fs::File::create(&rows_path).map_err(|e| JobError::io("create proxy rows", &rows_path, e))?;
    let mut writer = BufWriter::new(file);
    for row in rows.finish()? {
        let r = row?;
        let finite = |v: f64| if v.is_finite() { json!(v) } else { Value::Null };
        let line = json!({"schemaVersion":1,"uri":format!("proxies/f{}-d{}.glb", r.first, r.depth),
            "sha256":hex(r.sha256),"bytes":r.bytes,"first":r.first,"end":r.end,"depth":r.depth,
            "geometricError":r.published,"proxyToSource":r.to,"sourceToProxy":r.from,
            "composed":{"proxyToSource":r.composed_to,"sourceToProxy":r.composed_from},
            "direct":{"proxyToSource":finite(r.direct_to),"sourceToProxy":finite(r.direct_from)},
            "childMaximum":r.child_max,"faces":r.faces,"primitives":r.primitives,
            "vanishedRecords":r.vanished,"filledRecords":r.filled,
            "composedExhausted":r.flags & FLAG_COMPOSED_EXHAUSTED != 0,
            "directExhausted":r.flags & FLAG_DIRECT_EXHAUSTED != 0,
            "directSkipped":r.flags & FLAG_DIRECT_SKIPPED != 0,
            "latticeSha256":report.lattice_sha256});
        serde_json::to_writer(&mut writer, &line).map_err(|e| state(e.to_string()))?;
        writer
            .write_all(b"\n")
            .map_err(|e| JobError::io("write proxy rows", &rows_path, e))?;
    }
    writer
        .flush()
        .and_then(|_| writer.get_ref().sync_all())
        .map_err(|e| JobError::io("write proxy rows", &rows_path, e))?;
    report.seconds = started.elapsed().as_secs_f64();
    Ok(report)
}

