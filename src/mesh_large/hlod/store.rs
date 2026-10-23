//! The S1 → S2/S3 handoff: tile identity, final decoded geometry, representative source
//! primitives and the published error, with no certificates. Also the resident form in
//! which a finished tile waits for its parent; it is leased against the run's memory
//! budget and dropped (lease released) as soon as the parent is built.
use super::interfaces::InterfaceRow;
use crate::mesh_large::{records::Bounds, resources::Lease};
use serde::Serialize;

/// One tile of the frozen ownership tree: its source rank range, tile depth and DFS leaf
/// interval. `[first, end)` is also the descendant source range S2 samples against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TileKey {
    pub first: u64,
    pub end: u64,
    pub depth: u32,
    pub leaf_begin: u32,
    pub leaf_end: u32,
}

impl TileKey {
    /// The archive-stage member name (`delivery::ProxyRow` checks this form).
    pub(crate) fn uri(&self) -> String {
        format!("proxies/f{}-d{}.glb", self.first, self.depth)
    }
}

/// A source primitive (D4): the identity row a coarse triangle's single `source_primitive`
/// feature ID points at. Leaves carry their own exact rows; coarse triangles carry a
/// representative, not ancestry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub(crate) struct SourcePrimitive {
    pub node: u32,
    pub mesh: u32,
    pub primitive: u32,
    pub expanded_ordinal: u64,
    /// Source material ID; `u32::MAX` means none.
    pub material: u32,
}

/// What stopped a parent's reduction (D1 reports the binding constraint).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoppingBound {
    /// The merged children already fit the cap; the parent is their union.
    Input,
    /// Reduced to the triangle cap (the budget-led target).
    TriangleBudget,
}

/// A finished tile held for its parent: welded vertices with their leaf-canonical decode,
/// triangles over them, one representative source primitive per triangle and the outer
/// interface rows the parent must filter.
#[derive(Debug)]
pub(crate) struct TileGeometry {
    pub key: TileKey,
    /// Per vertex: the finest minimum incident leaf depth among the source vertices welded
    /// into it; its position is a point of that depth's lattice.
    pub ells: Vec<u8>,
    /// Leaf-canonical decode per vertex, unique within the tile (the weld key): what every
    /// leaf holding the vertex publishes.
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// Per triangle: index into `primitives`.
    pub representative: Vec<u32>,
    pub primitives: Vec<SourcePrimitive>,
    /// Outer interface of this tile, one row per shared vertex/edge (`leaf` arbitrary).
    pub interface: Vec<InterfaceRow>,
    /// Published geometric error of this tile (leaves: their encoding error).
    pub geometric_error: f64,
    /// Admission of this resident record against the stage ledger.
    pub lease: Option<Lease>,
}

/// Resident bytes charged per triangle of a held tile: vertices (≤ 3 per triangle at 21
/// bytes), triangles, representatives, primitive rows (≤ 1 per triangle at 32 bytes) and
/// interface rows (≤ 6 per triangle at 20 bytes), with allocation slack.
pub(crate) const HELD_BYTES_PER_TRIANGLE: u64 = 256;

impl TileGeometry {
    /// The admitted charge for a held tile of `triangles` triangles.
    pub(crate) fn charge(triangles: u64) -> u64 {
        triangles
            .max(1)
            .saturating_mul(HELD_BYTES_PER_TRIANGLE)
            .saturating_add(1024)
    }
}

/// The handoff of one coarse tile to the error (S2) and appearance (S3) stages: its final
/// decoded geometry (positions exactly as published; S3 charts must leave them fixed),
/// which vertices are outer interface (never moved or removed by any later stage),
/// representative identities and the errors S1 knows.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CoarseTile<'a> {
    pub key: TileKey,
    /// Indexed, final decoded (published) positions.
    pub decoded: &'a [[f32; 3]],
    pub triangles: &'a [[u32; 3]],
    pub locked: &'a [bool],
    pub representative: &'a [u32],
    pub primitives: &'a [SourcePrimitive],
    /// Decoded bounds of `decoded`.
    pub bounds: Bounds,
    /// Largest published error among the children.
    pub children_error: f64,
    /// meshopt's absolute error estimate of the reduction (0 for an unreduced union).
    pub simplifier_error: f64,
    /// Largest move of any vertex from its leaf-canonical decode by the coarse lattice.
    pub encode_displacement: f64,
    pub stopping: StoppingBound,
}

/// The published error of one coarse tile and the policy that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TileError {
    pub geometric_error: f64,
    pub policy: &'static str,
}

/// The persisted handoff row of one coarse tile (`proxies.jsonl`, postorder). Delivery
/// reads `uri`, `sha256`, `bytes`, `first`, `end`, `depth`, `geometricError` and
/// `primitives`; the rest is the S1 record S2/S3 extend.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HlodRow {
    pub schema_version: u32,
    pub uri: String,
    pub sha256: String,
    pub bytes: u64,
    pub first: u64,
    pub end: u64,
    pub depth: u32,
    pub leaf_begin: u32,
    pub leaf_end: u32,
    pub geometric_error: f64,
    pub error_policy: &'static str,
    pub measurement: super::error::Summary,
    #[serde(skip)]
    pub error_cpu_seconds: f64,
    #[serde(skip)]
    pub error_cpu: super::error::CpuReport,
    pub children_error: f64,
    pub simplifier_error: f64,
    pub encode_displacement: f64,
    /// Rows of the `source_primitive` property table (representatives).
    pub primitives: u32,
    pub gltf_primitives: u32,
    pub triangles: u32,
    pub input_triangles: u32,
    pub stopping_bound: StoppingBound,
    pub protected_faces: u32,
    pub retries: u32,
    pub interface_vertices: u32,
    pub interface_segments: u32,
    pub quantized: bool,
    pub bounds: BoundsRow,
    pub appearance: super::appearance::TileAppearance,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct BoundsRow {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl From<Bounds> for BoundsRow {
    fn from(b: Bounds) -> Self {
        Self {
            min: b.min.map(f64::from),
            max: b.max.map(f64::from),
        }
    }
}
