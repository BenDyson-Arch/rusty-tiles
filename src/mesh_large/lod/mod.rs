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
pub(in crate::mesh_large) mod codec;
mod geom;
pub(in crate::mesh_large) mod handoff;
pub(crate) mod incidence;
pub(in crate::mesh_large) mod mesh;
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

/// Proxy face cap `P` per tile (PROVISIONAL). Simplification aims at `P - P/16`, the relaxed
/// pass runs above `P`, and fills never take a proxy above `P`, so a proxy normally fits one
/// 8192-row `proxy_source` table (CesiumJS resolves picks through one table per model).
/// Tiles still above `P` are reported (`splitProvenanceTiles`); above `2P` over budget;
/// above `4P` Unsupported. Every property table stays within 8192 rows regardless.
pub(crate) const TARGET_FACES: usize = 8192;
/// Simplification stops at `P - P / FILL_HEADROOM` faces so accepted fills fit under `P`.
const FILL_HEADROOM: usize = 16;
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
/// Seed of the topology-cache binding over a tile's forced seam vertices.
const FORCED_BINDING: u64 = 0x4831_3746_4f52_4345;

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

/// One proxy's published row (fixed size, external-sorted into postorder). Equality
/// follows the sort order (`Ord`), so `Eq` and `Ord` agree even for NaN fields.
#[derive(Clone, Copy, Debug)]
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
impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
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

/// A seam record for the per-depth audit, grouped by `(lo, hi, kind, third)`:
///
/// - `kind` 0, a border edge with both endpoints cut at this depth (`third` zero), with the
///   direction its owner's face traverses it (`forward`: from `lo` to `hi`). A seam edge
///   between two consistently wound tiles is traversed once in each direction; two owners
///   traversing it the same way both cover the same side of it (overlapping coverage),
///   which an undirected owner count cannot see.
/// - `kind` 1, a face whose three vertices are all cut (keys sorted `lo < hi < third`).
///   Two tiles holding the same such face cover it twice even when every edge is
///   partnered (opposite windings: a fold over the seam). The source can also put one face
///   in two tiles: coincident triangles share a centroid key, but an over-budget key group
///   is split into rank-ordered runs, each its own leaf, so a double-sided face can straddle
///   a run boundary (and then sit in two proxies too, at the depths between). The pre-pass
///   marks coincident source faces special, and faces whose vertices are all special are
///   excepted; faces only leaves hold are excepted as well (leaf faces are source-exact,
///   and no leaf can re-run). What remains is simplification-made.
///
/// The owner is a depth-`d` proxy or a leaf at depth `<= d` (`leaf`): a leaf at depth `d`
/// has the same `owner_depth` as a proxy, but it is source and never re-runs, so its
/// records (logged once, in round 0) must never be superseded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SeamEdge {
    lo: Key,
    hi: Key,
    kind: u8,
    third: Key,
    forward: bool,
    owner: u64,
    owner_depth: u8,
    leaf: bool,
    lo_dec: [u32; 3],
    hi_dec: [u32; 3],
    special: bool,
    round: u8,
}
impl Record for SeamEdge {
    const SIZE: usize = 12 * 5 + 8 + 6;
    fn encode(&self, out: &mut [u8]) {
        let mut w = 0;
        for k in [self.lo, self.hi, self.lo_dec, self.hi_dec, self.third] {
            for v in k {
                out[w..w + 4].copy_from_slice(&v.to_le_bytes());
                w += 4;
            }
        }
        out[w..w + 8].copy_from_slice(&self.owner.to_le_bytes());
        out[w + 8] = self.owner_depth;
        out[w + 9] = u8::from(self.special);
        out[w + 10] = self.round;
        out[w + 11] = u8::from(self.forward);
        out[w + 12] = u8::from(self.leaf);
        out[w + 13] = self.kind;
    }
    fn decode(b: &[u8]) -> Self {
        let k = |i: usize| -> [u32; 3] {
            std::array::from_fn(|a| {
                u32::from_le_bytes(b[i + 4 * a..i + 4 * a + 4].try_into().unwrap())
            })
        };
        Self {
            lo: k(0),
            hi: k(12),
            lo_dec: k(24),
            hi_dec: k(36),
            third: k(48),
            owner: u64::from_le_bytes(b[60..68].try_into().unwrap()),
            owner_depth: b[68],
            special: b[69] != 0,
            round: b[70],
            forward: b[71] != 0,
            leaf: b[72] != 0,
            kind: b[73],
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
            key: std::array::from_fn(|i| {
                u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap())
            }),
            first: u64::from_le_bytes(b[12..20].try_into().unwrap()),
        }
    }
}

/// A maximal run of consecutive source ranks `[start, end)` of one source primitive: the
/// bake handoff's donor-range index (built once, read per tile by binary search).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct DonorRun {
    start: u64,
    end: u64,
    key: (u32, u32, u32),
}
impl Record for DonorRun {
    const SIZE: usize = 28;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.start.to_le_bytes());
        out[8..16].copy_from_slice(&self.end.to_le_bytes());
        out[16..20].copy_from_slice(&self.key.0.to_le_bytes());
        out[20..24].copy_from_slice(&self.key.1.to_le_bytes());
        out[24..28].copy_from_slice(&self.key.2.to_le_bytes());
    }
    fn decode(b: &[u8]) -> Self {
        let u = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        Self {
            start: u64::from_le_bytes(b[..8].try_into().unwrap()),
            end: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            key: (u(16), u(20), u(24)),
        }
    }
}

/// The run-length index of source primitives over ranks (one pass, cancellable).
fn donor_runs(
    scratch: &Scratch,
    source: &Sequence<LodSource>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Sequence<DonorRun>, JobError> {
    let mut out =
        crate::spill::SequenceWriter::<DonorRun>::new(scratch, "lod-donor-runs", 1 << 16)?;
    let mut run: Option<DonorRun> = None;
    for (rank, s) in (0u64..).zip(source.reader()?) {
        let key = s?.primitive_key();
        if rank.is_multiple_of(1 << 16) {
            check()?;
        }
        match &mut run {
            Some(r) if r.key == key => r.end = rank + 1,
            _ => {
                if let Some(r) = run.take() {
                    out.push(&r)?;
                }
                run = Some(DonorRun {
                    start: rank,
                    end: rank + 1,
                    key,
                });
            }
        }
    }
    if let Some(r) = run {
        out.push(&r)?;
    }
    Ok(out.finish()?)
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// What the LOD stage reports into the leaf manifest.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LodReport {
    pub schema_version: u32,
    pub proxies: u64,
    pub proxy_faces: u64,
    pub proxy_bytes: u64,
    /// After a bake pass: the certified untextured proxies' bytes (`proxy_bytes` is then
    /// the published, baked total). Absent without a baker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub untextured_proxy_bytes: Option<u64>,
    /// Bytes of frozen bake-handoff packets written (absent without a baker).
    #[serde(skip_serializing_if = "is_zero")]
    pub handoff_bytes: u64,
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
    /// Fills left open because they would take the proxy above the 8192-face cap.
    pub fills_rejected_budget: u64,
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
    /// Tiles the audit found overlapping a neighbour's seam coverage, re-run under the
    /// seam guard (absent when none).
    #[serde(skip_serializing_if = "is_zero")]
    pub seam_overlap_tiles: u64,
    pub lattice_sha256: String,
    pub sidecar_peak_bytes: u64,
    pub relaxed_tiles: u64,
    /// Tiles above `2P` faces (each glTF primitive still holds at most 8192 triangles).
    pub over_budget_tiles: u64,
    /// Tiles above 8192 faces, whose proxy_source rows are split per glTF primitive
    /// (CesiumJS 1.146 then resolves picks only through the first table).
    pub split_provenance_tiles: u64,
    /// Run facts outside the deterministic manifest.
    #[serde(skip)]
    pub jobs: u64,
    #[serde(skip)]
    pub prepass_seconds: f64,
    /// Measured spill peak of the incidence pre-pass (retained capture files included).
    #[serde(skip)]
    pub prepass_scratch_peak_bytes: u64,
    #[serde(skip)]
    pub seconds: f64,
    #[serde(skip)]
    pub phase_seconds: [f64; 10],
    #[serde(skip)]
    pub audit_seconds: f64,
    /// Tile runs including seam-consensus re-runs.
    #[serde(skip)]
    pub tile_runs: u64,
    pub texture: &'static str,
}

/// A child reference while listing a depth.
#[derive(Clone, Copy, Debug)]
enum Child {
    Leaf {
        node: NodeRecord,
        index: u64,
        error: f64,
        density_drop: u32,
    },
    Proxy {
        node: NodeRecord,
    },
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
    /// Per material, the index of the first equal flat material: proxy faces sort by it so
    /// one glTF primitive carries every source primitive of one flat material.
    material_rank: &'a [u32],
    retention: &'a [u8],
    target: usize,
    mutation: Mutation,
    ar: Arith,
    check: &'a (dyn Fn() -> Result<(), JobError> + Sync),
    /// The bake handoff sink, when a bake pass will consume the tiles.
    handoff: Option<&'a handoff::HandoffSink>,
    /// Source-primitive runs over ranks (present with `handoff`).
    donor_runs: Option<&'a Sequence<DonorRun>>,
    profile: String,
    lattice_sha256: String,
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
fn decode_vertex(
    lattice: &LatticePlan,
    key: Key,
    ell: u8,
    depth: u32,
) -> Result<[f32; 3], JobError> {
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
            fs::create_dir_all(parent)
                .map_err(|e| JobError::io("create LOD sidecar folder", parent, e))?;
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
        // Each face's own source_triangle table (a leaf may hold several, one per glTF
        // primitive, each within the consumer row limit).
        let (row, table) = face
            .feature(&decoded.tables, "source_triangle")
            .ok_or_else(|| state("leaf triangle without a source_triangle feature ID"))?;
        let columns = &decoded.tables[table].columns;
        let column = |name: &str| -> Result<u64, JobError> {
            columns
                .get(name)
                .and_then(|c| c.get(row as usize))
                .copied()
                .ok_or_else(|| state("leaf feature ID outside its source_triangle table"))
        };
        let provenance = (
            column("node_index")? as u32,
            column("mesh_index")? as u32,
            column("primitive_index")? as u32,
            column("triangle_index")?,
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
                            .is_ok_and(|e| {
                                e.decoded[0].map(f32::to_bits) == face.p[c].map(f32::to_bits)
                            })
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
        for (slot, &v) in expected.iter_mut().zip(&face.v) {
            let (key, info) = sidecar.vertices[v as usize];
            *slot = match cache.get(&v) {
                Some(p) => *p,
                None => {
                    let p = decode_vertex(ctx.lattice, key, info.ell, depth)?;
                    cache.insert(v, p);
                    p
                }
            };
        }
        if !codec::same_face(&expected, &published.p) {
            return Err(state(
                "proxy GLB bytes decode differently from their sidecar",
            ));
        }
        let (label, _) = sidecar.labels[face.label as usize];
        if published
            .feature(&decoded.tables, "source_primitive")
            .map(|f| f.0)
            != Some(face.label)
        {
            return Err(state("proxy feature ID differs from its sidecar label"));
        }
        // Provenance: node, mesh and primitive from the face's set-0 row, the source
        // triangle from its per-triangle `proxy_source` row (fills carry no triangle).
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
        let triangle = if column("fill")? != 0 {
            u64::MAX
        } else {
            column("triangle_index")?
        };
        faces.push(KeyFace {
            keys: face.v.map(|v| sidecar.vertices[v as usize].0),
            info: face.v.map(|v| sidecar.vertices[v as usize].1),
            label,
            origin: Origin {
                node: label.key.0,
                mesh: label.key.1,
                primitive: label.key.2,
                triangle,
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
    fills_rejected_budget: u64,
    forced: u64,
    forced_unchecked: u64,
    sidecar_bytes: u64,
    direct_to_used: bool,
    direct_from_used: bool,
    relaxed: bool,
    /// Seconds: load children, islands/seams, simplify, encode+composed, direct,
    /// per-primitive+fills, final encode/records/sidecar.
    phases: [f64; 10],
    forced_keys: Vec<Key>,
    failed_keys: Vec<Key>,
    over_budget: bool,
    handoff_bytes: u64,
}

/// Faces of a mesh grouped by flat material, then label (stable): the GLB face order.
fn by_label(faces: &[Face], mesh: &KeyMesh, rank: &[u32]) -> Vec<Face> {
    let mut out = faces.to_vec();
    out.sort_by_key(|f| {
        let material = mesh.labels[f.label as usize].material as usize;
        (rank.get(material).copied().unwrap_or(u32::MAX), f.label)
    });
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

fn process(
    ctx: &Ctx,
    job: &Job,
    keep_extra: &BTreeSet<Key>,
    full: bool,
    strict_seams: bool,
) -> Result<Outcome, JobError> {
    (ctx.check)()?;
    let mut clock = std::time::Instant::now();
    let node = &job.node;
    let d = node.depth();
    let ar = ctx.ar;
    let mut children = Vec::with_capacity(job.children.len());
    for child in &job.children {
        children.push(match child {
            Child::Leaf {
                node, index, error, ..
            } => load_leaf(ctx, node, *index, *error)?,
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
    let mut phases = [0f64; 10];
    lap(&mut phases[0]);
    let mesh = KeyMesh::build(&key_faces)?;
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
    let mut stalled = Vec::new();
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
            && key_hash(v.key).is_multiple_of(3)
            && !v.info.special
        {
            keep = false;
        }
        // Stress (test-only): the first tile of each depth cannot remove a third of its
        // forced seam vertices, as if every collapse were invalid; seam consensus must keep
        // them in every tile sharing them (the neighbours re-run).
        if !keep
            && ctx.mutation == Mutation::ForcedFailures
            && node.first == 0
            && d >= 1
            && key_hash(v.key).is_multiple_of(3)
            && !v.info.special
        {
            locked[i] = true;
            stalled.push(v.key);
            continue;
        }
        if keep {
            locked[i] = true;
        } else {
            forced.push(i as u32);
        }
    }
    lap(&mut phases[1]);
    // The finishing pass reuses the simplification of the tile's last topology run (same
    // children, same keep set, so the same result); the topology pass computes and caches it.
    let cache = ctx
        .sidecars
        .join(format!("topo-d{d}"))
        .join(format!("f{}.bin", node.first));
    // The cache key binds every input of the simplification that can change between rounds
    // of one depth: the tile's children and target are fixed per depth, but the forced set
    // shrinks as consensus keeps vertices, and the seam guard switches on once the
    // audit finds the tile overlapping a neighbour. Folding the guard flag into the seed
    // (`FORCED_BINDING ^ strict`) and every forced key into the hash means a topology
    // cached under a different keep set or guard state never matches, so neither a later
    // round nor the finishing pass reuses stale faces. (The finishing pass reads with the
    // tile's final flag and forced set: exactly the run the audit accepted.)
    let binding = forced
        .iter()
        .fold(FORCED_BINDING ^ u64::from(strict_seams), |h, &i| {
            mix(h ^ key_hash(mesh.vertices[i as usize].key))
        });
    // The finishing pass publishes exactly the topology the seam audit checked: its cache
    // must exist and match the tile's forced set under the final keep set.
    let cached = if full {
        Some(read_topology(&cache, binding)?.ok_or_else(|| {
            state(format!(
                "coarse tile f{}-d{d}: the finishing pass has no audited topology for its \
                 forced seam set (a seam-consensus re-run was missed)",
                node.first
            ))
        })?)
    } else {
        None
    };
    let (s_faces, report, relaxed) = match cached {
        Some(c) => c,
        None => {
            let mut simplifier = Simplifier::new(&mesh, &kept_faces, locked);
            simplifier.strict_seams = strict_seams;
            let failed = simplifier.force(&forced);
            simplifier.set_seam(
                mesh.vertices
                    .iter()
                    .map(|v| v.info.cut(d) && !v.info.special)
                    .collect(),
                mesh.vertices.iter().map(|v| v.info.cut(d)).collect(),
            );
            out.forced_keys = forced
                .iter()
                .map(|&i| mesh.vertices[i as usize].key)
                .collect();
            out.failed_keys = failed
                .iter()
                .map(|&i| mesh.vertices[i as usize].key)
                .chain(stalled.iter().copied())
                .collect();
            // At most P - P/16 faces (the rest is headroom for hole fills under the P cap),
            // and at least a halving of the children (single-child chains and small
            // subtrees still coarsen level by level).
            let target = (ctx.target - ctx.target / FILL_HEADROOM)
                .min(kept_faces.len().div_ceil(2))
                .max(1);
            simplifier.simplify(target);
            // Above the face cap the relaxed pass runs, so the proxy fits one 8192-row
            // provenance table (CesiumJS resolves picks through one table per model).
            let relaxed = simplifier.live_faces() > ctx.target;
            if relaxed {
                simplifier.relax();
                simplifier.simplify(target);
            }
            let (s_faces, report) = simplifier.finish();
            if !full {
                write_topology(&cache, binding, &s_faces, &report, relaxed)?;
            }
            (s_faces, report, relaxed)
        }
    };
    out.relaxed = relaxed;
    out.forced = report.forced;
    out.forced_unchecked = report.forced_unchecked;
    out.over_budget = s_faces.len() > 2 * ctx.target;
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
        return Err(state(format!(
            "coarse tile f{}-d{d} simplified to nothing",
            node.first
        )));
    }
    if !full {
        // Topology pass: the seam edges are all the depth audit needs (fills never touch a
        // seam vertex, so the finishing pass publishes the same seam edges).
        out.seams = seam_edges(ctx, &mesh, &s_faces, node.first, d)?;
        // Control: the first tile of each depth claims one of its seam edges a second time
        // under a fictitious owner traversing it the same way, at every round (consensus
        // cannot resolve it).
        if ctx.mutation == Mutation::OverclaimSeam && node.first == 0 && d >= 1 {
            if let Some(mut e) = out
                .seams
                .iter()
                .find(|e| e.kind == 0 && !e.special)
                .copied()
            {
                e.owner = u64::MAX;
                out.seams.push(e);
            }
        }
        // Stress: until the seam guard is on for it, every proxy also claims the far side
        // of each of its seam edges, so the audit sees overlapping coverage on every seam,
        // including those it shares with leaves at this depth.
        if ctx.mutation == Mutation::OverlapUntilStrict && !strict_seams {
            let far: Vec<SeamEdge> = out
                .seams
                .iter()
                .filter(|e| e.kind == 0 && !e.special)
                .map(|e| SeamEdge {
                    forward: !e.forward,
                    ..*e
                })
                .collect();
            out.seams.extend(far);
        }
        lap(&mut phases[2]);
        out.phases = phases;
        return Ok(out);
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
    let s_order = by_label(&s_faces, &mesh, ctx.material_rank);
    let flat_records: Vec<RecordTable> = Vec::new();
    let unfilled = codec::encode(
        &primitives(&mesh, &s_order, &key_faces),
        ctx.materials,
        ctx.lattice,
        d,
        &flat_records,
    )?;
    let s_dec = codec::decode(&unfilled.glb)?;
    if s_dec.faces.len() != unfilled.faces.len()
        || !s_dec
            .faces
            .iter()
            .zip(&unfilled.faces)
            .all(|(a, b)| codec::same_face(&a.p, b))
    {
        return Err(state(
            "proxy bytes decode differently from the encoder's lattice",
        ));
    }
    lap(&mut phases[7]);
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
    lap(&mut phases[8]);
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
    let t_s = geom::directed(
        ar,
        &t_measured,
        &s_bvh,
        Limits::for_sizes(t_measured.len(), s_tris.len()),
    );
    lap(&mut phases[9]);
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
        skip_removed
            && s.positions
                .iter()
                .all(|p| removed_keys.contains(&incidence::weld(*p)))
    };
    let lo = node.bounds.min.map(f64::from);
    let hi = node.bounds.max.map(f64::from);
    // Direct terms where they can differ from the composed ones: above leaf-only children
    // T is the decoded source itself (up to the leaves' own snap bound).
    let leaf_parent = job.children.iter().all(|c| matches!(c, Child::Leaf { .. }));
    let work = 256 * (s_tris.len() as u64 + t_tris.len() as u64);
    lap(&mut phases[3]);
    let direct = certify::direct(
        ar,
        ctx.source,
        node.first,
        node.end,
        lo,
        hi,
        &s_tris,
        &s_bvh,
        work,
        !leaf_parent,
        &skip,
        ctx.check,
    )?;
    let direct_to = direct.proxy_to_source.unwrap_or(f64::INFINITY);
    let direct_from = direct.source_to_proxy;
    let to = composed_to.min(direct_to);
    let from = composed_from.min(direct_from);
    out.direct_to_used = direct_to < composed_to;
    out.direct_from_used = direct_from < composed_from;
    let open = to.max(from);
    if !open.is_finite() {
        return Err(state(format!(
            "coarse tile f{}-d{d}: certificate is not finite",
            node.first
        )));
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
    let any_loop = matches!(
        ctx.mutation,
        Mutation::CrackAsHole | Mutation::FillAcrossSeams
    );
    let loops = mesh::border_loops(
        &s_order,
        |v| any_loop || !mesh.vertices[v as usize].info.cut(d),
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
            for (v, p) in f.v.iter().zip(dec) {
                m.insert(*v, *p);
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
        // A fill never takes the proxy above the face cap (one provenance table per model).
        if s_order.len() + fill_faces.len() + fan.len() > ctx.target.max(s_order.len()) {
            out.fills_rejected_budget += 1;
            continue;
        }
        let tris: Vec<Tri> = fan_tris.iter().map(tri_of).collect();
        let bvh = label_bvh
            .entry(l.label)
            .or_insert_with(|| Bvh::new(&t_by_label[&l.label]));
        let h = geom::directed(ar, &tris, bvh, Limits::for_sizes(tris.len(), bvh.len()));
        let bound = up_add(ar, h.bound, label_child[&l.label]);
        // NaN-safe: a bound that is not at most `open` is rejected.
        if bound
            .partial_cmp(&open)
            .is_none_or(|o| o == std::cmp::Ordering::Greater)
        {
            out.fills_rejected_bound += 1;
            continue;
        }
        // Border-hole rule, checked again on the accepted loop: a fill never closes a loop
        // through a seam (cut) vertex, so it cannot cap a tile's own seam perimeter or a
        // hole a seam cuts (the C3 control relabels a crack on purpose and is audited).
        if ctx.mutation != Mutation::CrackAsHole
            && l.vertices.iter().any(|&v| {
                let c2 = mesh.vertices[v as usize].info.c2;
                c2 != NEVER && u32::from(c2) <= d
            })
        {
            return Err(state(format!(
                "coarse tile f{}-d{d}: a fill loop passes through a seam vertex (border-hole \
                 rule)",
                node.first
            )));
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
        // Fill faces carry their record index as `u32::MAX - index` (never a T face id).
        let fill_id = u32::MAX - fill_rows.len() as u32;
        fill_rows.push((
            l.label,
            min_key,
            l.vertices.len() as u32,
            fan.len() as u32,
            bound,
            mask,
        ));
        for t in fan {
            fill_faces.push(Face {
                v: t,
                label: l.label,
                id: fill_id,
            });
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
    let final_order = by_label(&all, &mesh, ctx.material_rank);
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
    let lost_rows: Vec<(u32, Key, u32, u8)> =
        lost.iter().map(|(l, (n, k, m))| (*l, *k, *n, *m)).collect();
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
                (
                    "node_index",
                    "UINT32",
                    fill_rows.iter().map(|r| u64::from(l(r).0)).collect(),
                ),
                (
                    "mesh_index",
                    "UINT32",
                    fill_rows.iter().map(|r| u64::from(l(r).1)).collect(),
                ),
                (
                    "primitive_index",
                    "UINT32",
                    fill_rows.iter().map(|r| u64::from(l(r).2)).collect(),
                ),
                (
                    "loop_x",
                    "UINT32",
                    fill_rows.iter().map(|r| u64::from(r.1[0])).collect(),
                ),
                (
                    "loop_y",
                    "UINT32",
                    fill_rows.iter().map(|r| u64::from(r.1[1])).collect(),
                ),
                (
                    "loop_z",
                    "UINT32",
                    fill_rows.iter().map(|r| u64::from(r.1[2])).collect(),
                ),
                (
                    "loop_vertices",
                    "UINT32",
                    fill_rows.iter().map(|r| u64::from(r.2)).collect(),
                ),
                (
                    "fan_faces",
                    "UINT32",
                    fill_rows.iter().map(|r| u64::from(r.3)).collect(),
                ),
                (
                    "bound",
                    "FLOAT64",
                    fill_rows.iter().map(|r| r.4.to_bits()).collect(),
                ),
                (
                    "reopen_children",
                    "UINT8",
                    fill_rows.iter().map(|r| u64::from(r.5)).collect(),
                ),
                ("depth", "UINT8", vec![u64::from(d); fill_rows.len()]),
            ],
        });
    }
    let encoded = codec::encode(
        &primitives(&mesh, &final_order, &key_faces),
        ctx.materials,
        ctx.lattice,
        d,
        &records,
    )?;
    let check = codec::decode(&encoded.glb)?;
    if check.faces.len() != encoded.faces.len()
        || !check
            .faces
            .iter()
            .zip(&encoded.faces)
            .all(|(a, b)| codec::same_face(&a.p, b))
    {
        return Err(state(
            "final proxy bytes decode differently from the encoder's lattice",
        ));
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
            return Err(state(
                "published proxy faces differ from the certified faces",
            ));
        }
    }

    if let Some(sink) = ctx.handoff {
        let packet = frozen_packet(
            ctx,
            job,
            &mesh,
            &key_faces,
            &final_order,
            &check,
            &encoded.glb,
            &label_error,
            &fill_rows,
            &v_rows,
            published,
            to_filled,
            from,
        )?;
        out.handoff_bytes = sink.write(&packet, ctx.check)?;
    }
    // Sidecar (key space, GLB face order) and the published member.
    let mut used: BTreeMap<u32, u32> = BTreeMap::new();
    let mut vertices = Vec::new();
    let mut labels_used: BTreeMap<u32, u32> = BTreeMap::new();
    let mut labels = Vec::new();
    let mut sidecar_faces = Vec::with_capacity(final_order.len());
    for f in &final_order {
        let label = *labels_used.entry(f.label).or_insert_with(|| {
            labels.push((
                mesh.labels[f.label as usize],
                label_error.get(&f.label).copied().unwrap_or(published),
            ));
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

/// The frozen bake packet of a finished tile, from its key-space mesh (original weld keys,
/// lineage, fills) and its DECODED published bytes (raw POSITION words, TRS, features,
/// tables). Faces are paired by published order and must decode bit-identically.
#[allow(clippy::too_many_arguments)]
fn frozen_packet(
    ctx: &Ctx,
    job: &Job,
    mesh: &KeyMesh,
    key_faces: &[KeyFace],
    faces: &[Face],
    published: &codec::Decoded,
    glb: &[u8],
    label_error: &BTreeMap<u32, f64>,
    fill_rows: &[(u32, Key, u32, u32, f64, u8)],
    v_rows: &[[u64; 9]],
    geometric_error: f64,
    proxy_to_source: f64,
    source_to_proxy: f64,
) -> Result<handoff::FrozenNode, JobError> {
    use handoff::*;
    let node = &job.node;
    if published.faces.len() != faces.len() {
        return Err(state(
            "frozen packet: published faces differ from the tile's faces",
        ));
    }
    // Source primitives in published order (the source_primitive rows).
    let mut order: BTreeMap<u32, u32> = BTreeMap::new();
    let mut primitives: Vec<FrozenPrimitive> = Vec::new();
    for f in faces {
        if let std::collections::btree_map::Entry::Vacant(slot) = order.entry(f.label) {
            let l = mesh.labels[f.label as usize];
            slot.insert(primitives.len() as u32);
            primitives.push(FrozenPrimitive {
                node: l.key.0,
                mesh: l.key.1,
                primitive: l.key.2,
                expanded_ordinal: l.expanded_ordinal,
                material: l.material,
                row: primitives.len() as u32,
                error: label_error
                    .get(&f.label)
                    .copied()
                    .unwrap_or(geometric_error),
                donor_ranges: Vec::new(),
            });
        }
    }
    let by_key: BTreeMap<(u32, u32, u32), u32> = primitives
        .iter()
        .enumerate()
        .map(|(i, p)| ((p.node, p.mesh, p.primitive), i as u32))
        .collect();
    // Donor ranges from the run index: binary search for the first run reaching past
    // `first`, then the runs overlapping `[first, end)` only (cost O(log R + runs in range),
    // cancellable), each clipped to the tile.
    let runs = ctx
        .donor_runs
        .ok_or_else(|| state("bake handoff without its donor-run index"))?;
    let run_at = |i: u64| -> Result<DonorRun, JobError> {
        runs.range(i, i + 1, DonorRun::SIZE)?
            .next()
            .ok_or_else(|| state("donor-run index ends early"))?
            .map_err(JobError::from)
    };
    let (mut lo, mut hi) = (0u64, runs.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if run_at(mid)?.end <= node.first {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    for (k, run) in (0u64..).zip(runs.range(lo, runs.len(), 1 << 16)?) {
        let run = run?;
        if run.start >= node.end {
            break;
        }
        if k.is_multiple_of(1 << 12) {
            (ctx.check)()?;
        }
        if let Some(&i) = by_key.get(&run.key) {
            let piece = run.start.max(node.first)..run.end.min(node.end);
            let ranges = &mut primitives[i as usize].donor_ranges;
            match ranges.last_mut() {
                Some(r) if r.end == piece.start => r.end = piece.end,
                _ => ranges.push(piece),
            }
        }
    }
    let fill_base = u32::MAX - fill_rows.len() as u32;
    let mut frozen_faces = Vec::with_capacity(faces.len());
    for (f, d) in faces.iter().zip(&published.faces) {
        let source = order[&f.label];
        let fill = (f.id > fill_base).then(|| u32::MAX - f.id);
        let origin = match key_faces.get(f.id as usize) {
            Some(k) if fill.is_none() => k.origin,
            _ => {
                let p = &primitives[source as usize];
                Origin {
                    node: p.node,
                    mesh: p.mesh,
                    primitive: p.primitive,
                    triangle: u64::MAX,
                }
            }
        };
        let corners = std::array::from_fn(|c| {
            let v = mesh.vertices[f.v[c] as usize];
            handoff::FrozenCorner {
                raw: d.raw[c],
                decoded: d.p[c].map(f32::to_bits),
                weld: v.key,
                ell: v.info.ell,
            }
        });
        frozen_faces.push(FrozenFace {
            gltf: d.primitive,
            source,
            corners,
            origin,
            fill,
            features: d
                .features
                .iter()
                .map(|(id, table)| (*id, table.map(|t| t as u32)))
                .collect(),
        });
    }
    // Every frozen corner decodes to the certified position of its weld key at this depth.
    let mut seen: BTreeMap<Key, [u32; 3]> = BTreeMap::new();
    for f in &frozen_faces {
        for c in &f.corners {
            let expected = match seen.get(&c.weld) {
                Some(p) => *p,
                None => {
                    let p =
                        decode_vertex(ctx.lattice, c.weld, c.ell, node.depth())?.map(f32::to_bits);
                    seen.insert(c.weld, p);
                    p
                }
            };
            if expected != c.decoded {
                return Err(state(
                    "frozen packet: a published corner is not its weld key's lattice point",
                ));
            }
        }
    }
    let mut gltf = Vec::with_capacity(published.primitives.len());
    let mut start = 0u32;
    for (i, p) in published.primitives.iter().enumerate() {
        let end = start
            + frozen_faces[start as usize..]
                .iter()
                .take_while(|f| f.gltf == i as u32)
                .count() as u32;
        let run = &frozen_faces[start as usize..end as usize];
        let (Some(first), Some(last)) = (run.first(), run.last()) else {
            return Err(state("frozen packet: an empty glTF primitive"));
        };
        gltf.push(FrozenGltfPrimitive {
            node: p.node,
            mesh: p.mesh,
            index: p.index,
            sources: first.source..last.source + 1,
            material: p.material,
            translation: p.translation,
            scale: p.scale,
            quantized: p.quantized,
            feature_ids: p.feature_ids.clone(),
            faces: start..end,
        });
        start = end;
    }
    let fills = fill_rows
        .iter()
        .map(|r| FrozenFill {
            source: order[&r.0],
            loop_key: r.1,
            loop_vertices: r.2,
            fan_faces: r.3,
            bound: r.4,
            reopen_children: r.5,
        })
        .collect();
    let vanished = v_rows
        .iter()
        .map(|r| FrozenVanished {
            node: r[0] as u32,
            mesh: r[1] as u32,
            primitive: r[2] as u32,
            component_key: [r[3] as u32, r[4] as u32, r[5] as u32],
            faces: r[6] as u32,
            reopen_children: r[7] as u8,
            reason: r[8] as u8,
        })
        .collect();
    let children = job
        .children
        .iter()
        .map(|c| {
            let n = c.node();
            FrozenChild {
                first: n.first,
                end: n.end,
                depth: n.depth(),
                slot: n.slot(),
                leaf: match c {
                    Child::Leaf {
                        index,
                        density_drop,
                        ..
                    } => Some(LeafChild {
                        index: *index,
                        density_drop: *density_drop,
                    }),
                    Child::Proxy { .. } => None,
                },
            }
        })
        .collect();
    let tables = published
        .tables
        .iter()
        .map(|t| FrozenTable {
            class: t.class.clone(),
            count: t.count as u64,
            columns: t
                .columns
                .iter()
                .map(|(name, values)| {
                    (
                        name.clone(),
                        t.components.get(name).cloned().unwrap_or_default(),
                        values.clone(),
                    )
                })
                .collect(),
        })
        .collect();
    Ok(FrozenNode {
        identity: NodeIdentity {
            cell_depth: node.cell_depth,
            cell: node.cell,
            path_len: node.path_len,
            path: node.path,
            last_octant: node.last_octant,
            depth: node.depth(),
            first: node.first,
            end: node.end,
        },
        profile: ctx.profile.clone(),
        geometric_error,
        proxy_to_source,
        source_to_proxy,
        lattice_sha256: ctx.lattice_sha256.clone(),
        glb_sha256: Sha256::digest(glb).into(),
        glb_bytes: glb.len() as u64,
        primitives,
        gltf,
        faces: frozen_faces,
        fills,
        vanished,
        children,
        tables,
    })
}

/// Cache of a tile's simplified faces (scratch), written by its latest topology run.
fn write_topology(
    path: &Path,
    binding: u64,
    faces: &[Face],
    report: &mesh::SimplifyReport,
    relaxed: bool,
) -> Result<(), JobError> {
    let mut b = Vec::with_capacity(faces.len() * 20 + 64);
    for v in [
        binding,
        report.forced,
        report.forced_unchecked,
        report.collapses,
        report.input_faces,
    ] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.push(u8::from(relaxed));
    b.extend_from_slice(&(faces.len() as u64).to_le_bytes());
    for f in faces {
        for v in [f.v[0], f.v[1], f.v[2], f.label, f.id] {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| JobError::io("create LOD topology cache", parent, e))?;
    }
    fs::write(path, b).map_err(|e| JobError::io("write LOD topology cache", path, e))
}

/// The cached run, or None when there is none or it was computed for another forced set.
fn read_topology(
    path: &Path,
    binding: u64,
) -> Result<Option<(Vec<Face>, mesh::SimplifyReport, bool)>, JobError> {
    let b = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(JobError::io("read LOD topology cache", path, e)),
    };
    let bad = || state("LOD topology cache is corrupt");
    let u64_at = |i: usize| -> Result<u64, JobError> {
        Ok(u64::from_le_bytes(
            b.get(i..i + 8).ok_or_else(bad)?.try_into().unwrap(),
        ))
    };
    if u64_at(0)? != binding {
        return Ok(None);
    }
    let report = mesh::SimplifyReport {
        forced: u64_at(8)?,
        forced_unchecked: u64_at(16)?,
        collapses: u64_at(24)?,
        input_faces: u64_at(32)?,
        ..Default::default()
    };
    let relaxed = *b.get(40).ok_or_else(bad)? != 0;
    let n = u64_at(41)? as usize;
    if b.len() != 49 + 20 * n {
        return Err(bad());
    }
    let u = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    let faces = (0..n)
        .map(|k| {
            let o = 49 + 20 * k;
            Face {
                v: [u(o), u(o + 4), u(o + 8)],
                label: u(o + 12),
                id: u(o + 16),
            }
        })
        .collect();
    Ok(Some((faces, report, relaxed)))
}

/// Border edges of a depth-`d` proxy with both endpoints seam vertices, for the audit,
/// each with the direction the proxy's one face on it traverses it.
fn seam_edges(
    ctx: &Ctx,
    mesh: &KeyMesh,
    faces: &[Face],
    owner: u64,
    d: u32,
) -> Result<Vec<SeamEdge>, JobError> {
    // (lo, hi) -> (faces on the edge, the last one's traversal from `a` to `b`)
    let mut count: BTreeMap<(u32, u32), (u32, (u32, u32))> = BTreeMap::new();
    for f in faces {
        for k in 0..3 {
            let (a, b) = (f.v[k], f.v[(k + 1) % 3]);
            if a != b {
                let e = count.entry((a.min(b), a.max(b))).or_default();
                *e = (e.0 + 1, (a, b));
            }
        }
    }
    let mut out = Vec::new();
    for ((a, b), (n, (from, _))) in count {
        let (va, vb) = (mesh.vertices[a as usize], mesh.vertices[b as usize]);
        if n != 1 || !va.info.cut(d) || !vb.info.cut(d) {
            continue;
        }
        let (lo, hi) = if va.key <= vb.key { (va, vb) } else { (vb, va) };
        let dec = |v: &mesh::Vertex| -> Result<[u32; 3], JobError> {
            Ok(decode_vertex(ctx.lattice, v.key, v.info.ell, d)?.map(f32::to_bits))
        };
        out.push(SeamEdge {
            lo: lo.key,
            hi: hi.key,
            kind: 0,
            third: [0; 3],
            forward: mesh.vertices[from as usize].key == lo.key,
            owner,
            owner_depth: d as u8,
            leaf: false,
            lo_dec: dec(&lo)?,
            hi_dec: dec(&hi)?,
            special: lo.info.special && hi.info.special,
            round: 0,
        });
    }
    // Faces of seam vertices only: no two frontier tiles may both hold one.
    for f in faces {
        let v = f.v.map(|i| mesh.vertices[i as usize]);
        if v.iter().all(|x| x.info.cut(d)) {
            let special = v.iter().all(|x| x.info.special);
            out.extend(seam_face(v.map(|x| x.key), special, owner, d as u8, false));
        }
    }
    Ok(out)
}

/// The audit record of a face whose three (distinct) vertices are seam vertices.
fn seam_face(
    keys: [Key; 3],
    special: bool,
    owner: u64,
    owner_depth: u8,
    leaf: bool,
) -> Option<SeamEdge> {
    let mut k = keys;
    k.sort_unstable();
    (k[0] != k[1] && k[1] != k[2]).then_some(SeamEdge {
        lo: k[0],
        hi: k[1],
        kind: 1,
        third: k[2],
        forward: false,
        owner,
        owner_depth,
        leaf,
        lo_dec: [0; 3],
        hi_dec: [0; 3],
        special,
        round: 0,
    })
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
    // (lo, hi) -> (triangles on the edge, whether the last one runs from lo to hi)
    let mut count: BTreeMap<(Key, Key), (u32, bool)> = BTreeMap::new();
    let mut info: BTreeMap<Key, VertexInfo> = BTreeMap::new();
    for s in source.range(leaf.first, leaf.end, 1 << 16)? {
        let s = s?;
        let keys = s.positions.map(incidence::weld);
        for c in 0..3 {
            info.insert(keys[c], s.info[c]);
            let (a, b) = (keys[c], keys[(c + 1) % 3]);
            if a != b {
                let e = count.entry((a.min(b), a.max(b))).or_default();
                *e = (e.0 + 1, a < b);
            }
        }
    }
    let mut n = 0;
    for ((a, b), (c, forward)) in count {
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
            kind: 0,
            third: [0; 3],
            forward,
            owner: leaf.first,
            owner_depth: leaf.depth() as u8,
            leaf: true,
            lo_dec: dec(a, ia)?,
            hi_dec: dec(b, ib)?,
            special: ia.special && ib.special,
            round: 0,
        })?;
        n += 1;
    }
    // The leaf's own faces of seam vertices only.
    for s in source.range(leaf.first, leaf.end, 1 << 16)? {
        let s = s?;
        if s.info.iter().all(|i| i.cut(d)) {
            let special = s.info.iter().all(|i| i.special);
            let keys = s.positions.map(incidence::weld);
            let depth = leaf.depth() as u8;
            if let Some(face) = seam_face(keys, special, leaf.first, depth, true) {
                out.push(&face)?;
                n += 1;
            }
        }
    }
    Ok(n)
}

#[derive(Default)]
struct AuditCounts {
    edges: u64,
    matched: u64,
    boundary: u64,
    relabelled: u64,
    /// Overlapping coverage: edges two owners traverse the same way (or more than two
    /// owners claim), and faces of seam vertices two owners hold.
    overlapping: u64,
    /// Frontier tiles with a seam defect, and what is wrong with each.
    unmatched: BTreeMap<(u64, u8), TileDefects>,
}

/// What is wrong with a seam edge group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Defect {
    /// A border edge only one frontier tile has (a crack).
    NoPartner,
    /// Overlapping coverage: `forward` owners traverse the edge from `lo` to `hi`,
    /// `backward` owners the other way; a manifold seam edge has one of each.
    Overlap { forward: usize, backward: usize },
    /// A face of seam vertices only (`lo`, `hi`, `third`) that two or more tiles hold (a
    /// fold or overlap over the seam).
    DuplicateFace { third: Key },
}

/// One seam defect of a tile: the edge and what is wrong with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SeamDefect {
    lo: Key,
    hi: Key,
    defect: Defect,
}

/// Every seam defect of one frontier tile, summarised. Each defect is recorded, not just
/// the tile's first: consensus treats a tile with any overlap differently from one with
/// cracks only, so a crack listed first must not hide a later overlap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TileDefects {
    /// The tile is a leaf (source: consensus cannot re-run it).
    leaf: bool,
    /// The tile's first crack (edge without a partner), in edge order.
    crack: Option<SeamDefect>,
    /// The tile's first overlap, in edge order.
    overlap: Option<SeamDefect>,
    /// All defects of the tile.
    count: u64,
}

impl TileDefects {
    fn add(&mut self, d: SeamDefect) {
        let slot = match d.defect {
            Defect::NoPartner => &mut self.crack,
            Defect::Overlap { .. } | Defect::DuplicateFace { .. } => &mut self.overlap,
        };
        slot.get_or_insert(d);
        self.count += 1;
    }

    /// The defect to report: an overlap before a crack (the crack is often its symptom).
    fn first(&self) -> Option<SeamDefect> {
        self.overlap.or(self.crack)
    }
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
        let owners: BTreeSet<(u64, u8)> = group.iter().map(|e| (e.owner, e.owner_depth)).collect();
        let leaves: BTreeSet<(u64, u8)> = group
            .iter()
            .filter(|e| e.leaf)
            .map(|e| (e.owner, e.owner_depth))
            .collect();
        let side = |forward: bool| -> BTreeSet<(u64, u8)> {
            group
                .iter()
                .filter(|e| e.forward == forward)
                .map(|e| (e.owner, e.owner_depth))
                .collect()
        };
        let (forward, backward) = (side(true), side(false));
        let flag = |defect: Defect, owners: &BTreeSet<(u64, u8)>, counts: &mut AuditCounts| {
            for owner in owners {
                let tile = counts.unmatched.entry(*owner).or_default();
                tile.leaf = leaves.contains(owner);
                tile.add(SeamDefect {
                    lo: first.lo,
                    hi: first.hi,
                    defect,
                });
            }
        };
        if first.kind == 1 {
            // A face of seam vertices two tiles both hold covers the seam twice, unless the
            // source itself repeats it there (its vertices are then special) or only leaves
            // hold it (source-exact faces; a leaf never re-runs).
            if owners.len() >= 2 && !first.special && owners != leaves {
                counts.overlapping += 1;
                flag(
                    Defect::DuplicateFace { third: first.third },
                    &owners,
                    counts,
                );
            }
            group.clear();
            return Ok(());
        }
        counts.edges += 1;
        if group
            .iter()
            .any(|e| e.lo_dec != first.lo_dec || e.hi_dec != first.hi_dec)
        {
            return Err(state(format!(
                "depth {d} seam: tiles decode a shared edge differently (crack)"
            )));
        }
        if owners.len() >= 2 && forward.len() == 1 && backward.len() == 1 {
            // One owner on each side: the manifold seam edge.
            counts.matched += 1;
        } else if owners.len() >= 2 {
            // Two owners on one side (or more than two owners): overlapping coverage, unless
            // the edge's endpoints are special in the source (non-manifold, boundary or
            // winding-flip edges, whose coverage the source itself does not orient).
            if first.special {
                counts.matched += 1;
            } else {
                counts.overlapping += 1;
                flag(
                    Defect::Overlap {
                        forward: forward.len(),
                        backward: backward.len(),
                    },
                    &owners,
                    counts,
                );
            }
        } else if first.special {
            counts.boundary += 1;
        } else if relabel {
            counts.relabelled += 1;
        } else {
            flag(Defect::NoPartner, &owners, counts);
        }
        group.clear();
        Ok(())
    };
    for e in sorted {
        let e = e?;
        if group
            .first()
            .is_some_and(|g| (g.lo, g.hi, g.kind, g.third) != (e.lo, e.hi, e.kind, e.third))
        {
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
    density_drop: u32,
}
impl Record for LeafFact {
    const SIZE: usize = 20;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.first.to_le_bytes());
        out[8..16].copy_from_slice(&self.error_bits.to_le_bytes());
        out[16..].copy_from_slice(&self.density_drop.to_le_bytes());
    }
    fn decode(b: &[u8]) -> Self {
        Self {
            first: u64::from_le_bytes(b[..8].try_into().unwrap()),
            error_bits: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            density_drop: u32::from_le_bytes(b[16..20].try_into().unwrap()),
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
    /// Write one frozen packet per final tile for the bake pass (H13).
    pub handoff: Option<&'a handoff::HandoffSink>,
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
        return Err(state(
            "the frozen H8 lattice differs from its recomputation",
        ));
    }
    report.lattice_sha256 = hex(Sha256::digest(&frozen));
    let proxies_dir = input.stage.join("proxies");
    fs::create_dir_all(&proxies_dir)
        .map_err(|e| JobError::io("create proxy folder", &proxies_dir, e))?;
    // Leaf facts in leaf order.
    let leaf_facts = {
        let path = input.stage.join("leaves.jsonl");
        let text = fs::File::open(&path).map_err(|e| JobError::io("read leaf rows", &path, e))?;
        let mut writer =
            crate::spill::SequenceWriter::<LeafFact>::new(input.scratch, "lod-leaves", 1 << 16)?;
        for line in std::io::BufRead::lines(std::io::BufReader::new(text)) {
            let line = line.map_err(|e| JobError::io("read leaf rows", &path, e))?;
            let row: Value = serde_json::from_str(&line).map_err(|e| state(e.to_string()))?;
            let error = row["geometryError"]
                .as_f64()
                .ok_or_else(|| state("leaf row without error"))?;
            writer.push(&LeafFact {
                first: row["first"]
                    .as_u64()
                    .ok_or_else(|| state("leaf row without first"))?,
                error_bits: error.to_bits(),
                density_drop: row["densityDrop"].as_u64().unwrap_or(0) as u32,
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
        incidence::scratch_need(captured.scan.triangles)
            .ok_or_else(|| resource("LOD scratch overflow"))?,
        crate::spill::SpaceRole::Scratch,
        crate::spill::PreflightStage::Lod,
    )?;
    let prepass = std::time::Instant::now();
    // The pre-pass is capture's declared `lod_prepass` stage (full::ScratchStages): measure it
    // and fail closed above its bound, like every capture stage.
    input.scratch.begin_stage();
    let source = incidence::build(
        input.scratch,
        input.sort,
        &captured.tree.nodes,
        &captured.triangles,
        &captured.corner_depths,
        check,
    )?;
    report.prepass_scratch_peak_bytes = input.scratch.stage_peak_bytes();
    let bound = captured.report.scratch.stages.lod_prepass;
    if report.prepass_scratch_peak_bytes > bound {
        return Err(resource(format!(
            "LOD pre-pass scratch {} bytes exceeded its declared stage bound {bound}",
            report.prepass_scratch_peak_bytes
        )));
    }
    report.prepass_seconds = prepass.elapsed().as_secs_f64();
    // Flat proxy materials without names, so equal flat materials share a glTF primitive.
    let materials: Vec<Value> = captured
        .materials
        .iter()
        .map(|m| {
            let mut value = m.flat_proxy();
            if let Some(object) = value.as_object_mut() {
                object.remove("name");
            }
            value
        })
        .collect();
    // H28: equal means the same material class (numbers by value, glTF defaults filled),
    // so flat materials written differently still share one glTF primitive and node.
    let keys: Vec<Value> = materials
        .iter()
        .map(|m| super::batching::material_key(m, false))
        .collect();
    let material_rank: Vec<u32> = (0..materials.len())
        .map(|i| keys.iter().position(|k| *k == keys[i]).unwrap_or(i) as u32)
        .collect();
    let sidecars = input.scratch.path().join("lod-sidecars");
    fs::create_dir_all(&sidecars).map_err(|e| JobError::io("create LOD sidecars", &sidecars, e))?;
    let runs = match input.handoff {
        Some(_) => Some(donor_runs(input.scratch, &source, check)?),
        None => None,
    };
    let ctx = Ctx {
        stage: input.stage,
        sidecars: &sidecars,
        source: &source,
        lattice: &captured.lattice,
        materials: &materials,
        material_rank: &material_rank,
        retention: &report.retention.clone(),
        target: input.target,
        mutation: input.mutation,
        ar: Arith::new(input.mutation),
        check,
        handoff: input.handoff,
        donor_runs: runs.as_ref(),
        profile: serde_json::to_value(captured.lattice.profile())
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        lattice_sha256: report.lattice_sha256.clone(),
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
        // Tiles whose seam overlapped another tile's (an edge claimed by more than two
        // tiles or two traversing it the same way, or a seam-vertex face two tiles hold):
        // they re-run with the seam guard.
        let mut strict: BTreeSet<u64> = BTreeSet::new();
        let mut newly_strict: BTreeSet<u64> = BTreeSet::new();
        let mut latest: BTreeMap<(u64, u8), u8> = BTreeMap::new();
        let mut failed: BTreeSet<Key> = BTreeSet::new();
        let window_size = 4 * input.jobs.max(1);
        let relabel = input.mutation == Mutation::CrackAsHole;
        let mut round = 0u8;
        let mut selected: Option<BTreeSet<u64>> = None;
        // Rounds run the topology pass only (children, islands, seams, simplification, seam
        // edges); once the depth agrees, one finishing pass certifies, fills, encodes and
        // writes every tile with the final keep set.
        let mut finishing = false;
        let mut final_counts: Option<AuditCounts> = None;
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
                       failed: &mut BTreeSet<Key>,
                       runs: &mut u64|
             -> Result<(), JobError> {
                if window.is_empty() {
                    return Ok(());
                }
                let outcomes: Vec<Result<Outcome, JobError>> = pool.install(|| {
                    use rayon::prelude::*;
                    window
                        .par_iter()
                        .map(|job| {
                            process(
                                &ctx,
                                job,
                                &keep,
                                finishing,
                                strict.contains(&job.node.first),
                            )
                        })
                        .collect()
                });
                for (job, outcome) in window.iter().zip(outcomes) {
                    let mut o = outcome?;
                    if finishing {
                        if !o.failed_keys.is_empty() {
                            return Err(state("a finishing tile disagreed with its topology pass"));
                        }
                        done.insert(job.node.first, Done(o));
                        continue;
                    }
                    for mut e in std::mem::take(&mut o.seams) {
                        e.round = round;
                        seam_log.push(&e)?;
                    }
                    if round == 0 {
                        for key in std::mem::take(&mut o.forced_keys) {
                            forced_log.push(&ForcedKey {
                                key,
                                first: job.node.first,
                            })?;
                        }
                    }
                    failed.extend(std::mem::take(&mut o.failed_keys));
                    *runs += 1;
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
                    if round == 0 && !finishing && n.depth() <= d {
                        leaf_seams(&source, &captured.lattice, &n, d, &mut seam_log)?;
                    }
                    if n.depth() == d + 1 {
                        if let Some(job) = current.as_mut() {
                            job.children.push(Child::Leaf {
                                node: n,
                                index: leaf_index,
                                error: f64::from_bits(fact.error_bits),
                                density_drop: fact.density_drop,
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
                            run(
                                &mut window,
                                &mut seam_log,
                                &mut forced_log,
                                &mut done,
                                &mut failed,
                                &mut report.tile_runs,
                            )?;
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
            run(
                &mut window,
                &mut seam_log,
                &mut forced_log,
                &mut done,
                &mut failed,
                &mut report.tile_runs,
            )?;
            if finishing {
                break final_counts
                    .take()
                    .ok_or_else(|| state("finished without an audit"))?;
            }
            // Seam consensus. A tile that cannot remove a seam vertex validly keeps it; a
            // seam edge without a partner keeps every seam vertex its tile removed. Kept
            // vertices are kept by every tile sharing them (each re-runs), which always
            // agrees, so this converges unless a tile ignores the keep set.
            let mut contested = std::mem::take(&mut failed);
            let mut audited = None;
            if contested.is_empty() {
                let log = std::mem::replace(
                    &mut seam_log,
                    crate::spill::SequenceWriter::<SeamEdge>::new(
                        input.scratch,
                        "lod-seams",
                        1 << 16,
                    )?,
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
                    final_counts = Some(counts);
                    finishing = true;
                    selected = None;
                    continue;
                }
                // Consensus acts on this depth's proxies only. A leaf at depth `d` is a
                // frontier owner with the same depth, but it is source: it has no forced
                // keys and never re-runs, and marking it for a re-run would supersede its
                // round-0 seam records (the only ones it ever logs), leaving every neighbour
                // along its seam without a partner.
                let proxies = || {
                    counts
                        .unmatched
                        .iter()
                        .filter(|((_, depth), t)| u32::from(*depth) == d && !t.leaf)
                };
                let owners: BTreeSet<u64> = proxies().map(|((first, _), _)| *first).collect();
                // Every proxy with an overlap among its defects, not just the proxies whose
                // first defect is one.
                newly_strict = proxies()
                    .filter(|((first, _), t)| t.overlap.is_some() && !strict.contains(first))
                    .map(|((first, _), _)| *first)
                    .collect();
                let log = std::mem::replace(
                    &mut forced_log,
                    crate::spill::SequenceWriter::<ForcedKey>::new(
                        input.scratch,
                        "lod-forced",
                        1 << 16,
                    )?,
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
            if (contested.is_empty() && newly_strict.is_empty()) || round >= 16 {
                let counts = audited.unwrap_or_default();
                let p = |k: Key| k.map(f32::from_bits);
                // Name an overlap if any tile has one (a crack is often its symptom).
                let named = counts
                    .unmatched
                    .iter()
                    .find(|(_, t)| t.overlap.is_some())
                    .or_else(|| counts.unmatched.iter().next())
                    .and_then(|(k, t)| Some((*k, t.first()?)));
                let ((first, depth), defect) = named.unwrap_or((
                    (0, 0),
                    SeamDefect {
                        lo: [0; 3],
                        hi: [0; 3],
                        defect: Defect::NoPartner,
                    },
                ));
                let what = match defect.defect {
                    Defect::NoPartner => "has no partner tile".to_string(),
                    Defect::DuplicateFace { third } => format!(
                        "and {:?} form a face of seam vertices that two tiles both hold \
                         (overlapping coverage)",
                        p(third)
                    ),
                    Defect::Overlap { forward, backward } => format!(
                        "is covered on the same side by two tiles ({forward} traverse it one way, \
                         {backward} the other: overlapping coverage)"
                    ),
                };
                let total: u64 = counts.unmatched.values().map(|t| t.count).sum();
                return Err(state(format!(
                    "depth {d} seam: edge {:?}-{:?} of tile f{first}-d{depth} {what}, and seam \
                     consensus cannot resolve it (crack; {total} seam defects in {} tiles); a \
                     crack is never published as a hole",
                    p(defect.lo),
                    p(defect.hi),
                    counts.unmatched.len()
                )));
            }
            round += 1;
            report.seam_rounds += 1;
            report.seam_kept_extra += contested.len() as u64;
            let log = std::mem::replace(
                &mut forced_log,
                crate::spill::SequenceWriter::<ForcedKey>::new(
                    input.scratch,
                    "lod-forced",
                    1 << 16,
                )?,
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
            report.seam_overlap_tiles += newly_strict.len() as u64;
            rerun.extend(newly_strict.iter().copied());
            strict.append(&mut newly_strict);
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
            let row = o
                .row
                .ok_or_else(|| state("a coarse tile produced no row"))?;
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
            report.fills_rejected_budget += o.fills_rejected_budget;
            report.handoff_bytes += o.handoff_bytes;
            report.forced_collapses += o.forced;
            report.forced_unchecked += o.forced_unchecked;
            report.relaxed_tiles += u64::from(o.relaxed);
            report.over_budget_tiles += u64::from(o.over_budget);
            report.split_provenance_tiles += u64::from(u64::from(row.faces) > 8192);
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
        let topo_dir = sidecars.join(format!("topo-d{d}"));
        if topo_dir.exists() {
            fs::remove_dir_all(&topo_dir)
                .map_err(|e| JobError::io("remove LOD topology cache", &topo_dir, e))?;
        }
        let child_dir = sidecars.join(format!("d{}", d + 1));
        if child_dir.exists() {
            fs::remove_dir_all(&child_dir)
                .map_err(|e| JobError::io("remove LOD sidecars", &child_dir, e))?;
            sidecar_live -= sidecar_bytes.remove(&(d + 1)).unwrap_or(0);
        }
    }
    let _ = fs::remove_dir_all(&sidecars);
    // Rows in postorder (end ascending, deeper first): H16's tile-closing order.
    let file = fs::File::create(&rows_path)
        .map_err(|e| JobError::io("create proxy rows", &rows_path, e))?;
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
