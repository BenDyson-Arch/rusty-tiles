//! `mesh-inspect`'s source topology pass over the captured triangles (#348).
//!
//! Exact positions only: two corners are one position when their captured f32 coordinates
//! are bit-equal (`TriangleRecord::positions`, after node transforms and any source CRS
//! mapping; −0 is stored as +0). No tolerance. A source vertex is a primitive instance's
//! POSITION index (`(node, primitive, index)`).
//!
//! Work: one external sort of three corners per triangle (position ids, repeated positions),
//! one of position ids by corner, one of three edges per triangle, and two union-find
//! arrays' worth of one `u32` per position (components, then loops, then seams, reusing one
//! array). Open edges and seams are kept as sequences for the loop joins and the optional
//! GeoJSON. Every sort and sequence is admitted on the scratch before it is written; every
//! resident array is admitted against the memory budget before it is allocated.
use crate::{
    mesh_large::{
        attributes::FullTriangle,
        config::{self, SortSpill},
        hlod::forecast::{self, LeafFacts, DEPTHS},
        phases::{Step, StepProgress},
        records::{NodeKind, NodeRecord},
    },
    spill::{ExternalSorter, Record, Scratch, Sequence, SequenceWriter, SortConfig},
    JobError, JobErrorKind,
};
use serde::Serialize;
use std::collections::BTreeMap;

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}
fn resource(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message.into())
}

fn put_u32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn get_u32(bytes: &[u8], at: usize) -> u32 {
    let mut b = [0; 4];
    b.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(b)
}
fn put_u64(out: &mut [u8], at: usize, v: u64) {
    out[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn get_u64(bytes: &[u8], at: usize) -> u64 {
    let mut b = [0; 8];
    b.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(b)
}
fn put_key(out: &mut [u8], at: usize, k: [u32; 3]) {
    for (i, v) in k.iter().enumerate() {
        put_u32(out, at + 4 * i, *v);
    }
}
fn get_key(bytes: &[u8], at: usize) -> [u32; 3] {
    std::array::from_fn(|i| get_u32(bytes, at + 4 * i))
}

/// One triangle corner: its exact position, source vertex and `3 × rank + corner` slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Corner {
    position: [u32; 3],
    node: u32,
    primitive: u32,
    vertex: u32,
    slot: u64,
}
impl Record for Corner {
    const SIZE: usize = 32;
    fn encode(&self, out: &mut [u8]) {
        put_key(out, 0, self.position);
        put_u32(out, 12, self.node);
        put_u32(out, 16, self.primitive);
        put_u32(out, 20, self.vertex);
        put_u64(out, 24, self.slot);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            position: get_key(bytes, 0),
            node: get_u32(bytes, 12),
            primitive: get_u32(bytes, 16),
            vertex: get_u32(bytes, 20),
            slot: get_u64(bytes, 24),
        }
    }
}

/// A corner slot's position id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SlotId {
    slot: u64,
    id: u32,
}
impl Record for SlotId {
    const SIZE: usize = 12;
    fn encode(&self, out: &mut [u8]) {
        put_u64(out, 0, self.slot);
        put_u32(out, 8, self.id);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            slot: get_u64(bytes, 0),
            id: get_u32(bytes, 8),
        }
    }
}

/// One triangle edge between position ids `a < b`, with its primitive instance, source
/// vertex indices (in `a`, `b` order), leaf and endpoint positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    a: u32,
    b: u32,
    node: u32,
    primitive: u32,
    ia: u32,
    ib: u32,
    leaf: u32,
    pa: [u32; 3],
    pb: [u32; 3],
}
impl Record for Edge {
    const SIZE: usize = 52;
    fn encode(&self, out: &mut [u8]) {
        put_u32(out, 0, self.a);
        put_u32(out, 4, self.b);
        put_u32(out, 8, self.node);
        put_u32(out, 12, self.primitive);
        put_u32(out, 16, self.ia);
        put_u32(out, 20, self.ib);
        put_u32(out, 24, self.leaf);
        put_key(out, 28, self.pa);
        put_key(out, 40, self.pb);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            a: get_u32(bytes, 0),
            b: get_u32(bytes, 4),
            node: get_u32(bytes, 8),
            primitive: get_u32(bytes, 12),
            ia: get_u32(bytes, 16),
            ib: get_u32(bytes, 20),
            leaf: get_u32(bytes, 24),
            pa: get_key(bytes, 28),
            pb: get_key(bytes, 40),
        }
    }
}

/// An open edge or a seam edge, kept for the loop joins and the GeoJSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Kept {
    a: u32,
    b: u32,
    leaf: u32,
    /// Seams: the edge's faces come from more than one primitive instance.
    between: bool,
    pa: [u32; 3],
    pb: [u32; 3],
}
impl Record for Kept {
    const SIZE: usize = 37;
    fn encode(&self, out: &mut [u8]) {
        put_u32(out, 0, self.a);
        put_u32(out, 4, self.b);
        put_u32(out, 8, self.leaf);
        out[12] = u8::from(self.between);
        put_key(out, 13, self.pa);
        put_key(out, 25, self.pb);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            a: get_u32(bytes, 0),
            b: get_u32(bytes, 4),
            leaf: get_u32(bytes, 8),
            between: bytes[12] != 0,
            pa: get_key(bytes, 13),
            pb: get_key(bytes, 25),
        }
    }
}

/// A GeoJSON segment: its feature's kind and selection order, then its endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GeoSegment {
    pub feature: u32,
    pub pa: [u32; 3],
    pub pb: [u32; 3],
}
impl Record for GeoSegment {
    const SIZE: usize = 28;
    fn encode(&self, out: &mut [u8]) {
        put_u32(out, 0, self.feature);
        put_key(out, 4, self.pa);
        put_key(out, 16, self.pb);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            feature: get_u32(bytes, 0),
            pa: get_key(bytes, 4),
            pb: get_key(bytes, 16),
        }
    }
}

/// What a sort of `records` spills: nothing while they fit one run, else every record.
fn spill<T: Record>(records: u64, sorter: SortConfig) -> Option<SortSpill> {
    let run = (sorter.run_bytes / std::mem::size_of::<T>().max(1)).max(1) as u64;
    if records <= run {
        return Some(SortSpill {
            bytes: 0,
            push_peak: 0,
            finish_peak: 0,
            stream_slack: 0,
        });
    }
    config::sort_spill::<T>(records, sorter)
}

/// Loop length size classes (#348): `< 1 m`, `1–5 m`, `5–20 m`, `> 20 m`.
pub(crate) const CLASSES: [&str; 4] = ["under_1m", "1m_to_5m", "5m_to_20m", "over_20m"];
fn class(length: f64) -> usize {
    if length < 1. {
        0
    } else if length < 5. {
        1
    } else if length <= 20. {
        2
    } else {
        3
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Measure {
    pub edges: u64,
    pub length_m: f64,
}
impl Measure {
    fn add(&mut self, edges: u64, length: f64) {
        self.edges += edges;
        self.length_m += length;
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SizeClass {
    pub class: &'static str,
    pub loops: u64,
    pub edges: u64,
    pub length_m: f64,
}

/// Open loops or seams: their edges and length, how many connected chains they form and
/// those chains by length class.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Loops {
    pub edges: u64,
    pub length_m: f64,
    pub loops: u64,
    pub by_size_class: Vec<SizeClass>,
}
impl Loops {
    fn new() -> Self {
        Self {
            by_size_class: CLASSES
                .iter()
                .map(|class| SizeClass {
                    class,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }
}

/// Edges open within their own primitive (by source vertex index): where primitives or
/// unwelded vertices meet, and true borders.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PrimitiveBoundaries {
    /// Every edge used by exactly one face of its primitive instance with those vertices.
    pub edges: Measure,
    /// Of them, edges exactly coincident (both end positions) with another primitive
    /// instance's edge.
    pub coincide_with_other_primitive: Measure,
    /// Of them, edges coincident only with another face of the same primitive (unwelded
    /// vertices inside one primitive).
    pub unwelded_within_primitive: Measure,
    /// Of them, edges no other face shares at all (open after welding positions).
    pub open: Measure,
}

/// The topology pass's report (exact positions, see the module notes).
#[derive(Clone, Debug, Default, Serialize)]
pub struct TopologyReport {
    pub triangles: u64,
    /// Triangles with two corners at one position (no surface; no edges counted).
    pub degenerate_triangles: u64,
    /// Distinct source vertices (`(node, primitive, index)`) the triangles use.
    pub source_vertices: u64,
    /// Distinct exact positions.
    pub positions: u64,
    /// Primitive instances (`(node, primitive)`) with at least one triangle.
    pub primitive_instances: u64,
    /// Connected components after welding exact positions.
    pub components: u64,
    pub surface_area_m2: f64,
    /// Position edges with more than two faces.
    pub nonmanifold_edges: u64,
    /// Open edges after welding exact positions (one face), their loops and size classes.
    pub open: Loops,
    pub primitive_boundaries: PrimitiveBoundaries,
    pub repeated_positions: RepeatedPositions,
    /// Unwelded seams: edges open by index but closed by position, joined into chains.
    pub seams: Loops,
}

/// Positions held by more than one source vertex: the HLOD welds them, a source viewer
/// does not.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RepeatedPositions {
    pub positions: u64,
    /// Source vertices beyond the first at those positions.
    pub extra_vertices: u64,
    /// Position edges that welding repeated positions closes (each had at least one
    /// primitive-open edge and is shared by two or more faces by position).
    pub closes_open_edges: Measure,
}

/// One GeoJSON line feature: an open loop or a seam chain at or above the length threshold.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct LineFeature {
    pub kind: &'static str,
    pub edges: u64,
    pub length_m: f64,
    pub size_class: &'static str,
    /// Seams: whether any of its edges lies between primitive instances.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub between_primitives: Option<bool>,
}

/// GeoJSON request of the pass.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GeoRequest {
    pub min_length: f64,
    pub max_segments: u64,
}

/// The selected line features and their segments, sorted by feature.
pub(crate) struct GeoLines<'s> {
    pub features: Vec<LineFeature>,
    pub segments: crate::spill::SortedStream<'s, GeoSegment>,
    /// Features at or above the threshold left out by the segment budget.
    pub omitted: u64,
}

pub(crate) struct Topology<'s> {
    pub report: TopologyReport,
    pub leaves: Vec<LeafFacts>,
    pub geo: Option<GeoLines<'s>>,
}

/// Admission callbacks of the pass.
pub(crate) struct Admit<'a> {
    /// A resident allocation of `bytes` under `name`, before it is made.
    pub memory: &'a dyn Fn(&'static str, u64) -> Result<(), JobError>,
    /// A scratch bound (live bytes included), before the phase writes.
    pub scratch: &'a dyn Fn(u64) -> Result<(), JobError>,
}

/// Union-find over `n` position ids: the smaller root wins, so roots are deterministic.
struct Groups(Vec<u32>);
impl Groups {
    fn new(n: u32) -> Result<Self, JobError> {
        let mut v = Vec::new();
        v.try_reserve_exact(n as usize)
            .map_err(|_| resource("position union-find allocation failed"))?;
        v.extend(0..n);
        Ok(Self(v))
    }
    fn reset(&mut self) {
        for (i, p) in self.0.iter_mut().enumerate() {
            *p = i as u32;
        }
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            let up = self.0[self.0[x as usize] as usize];
            self.0[x as usize] = up;
            x = up;
        }
        x
    }
    fn join(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            let (lo, hi) = (a.min(b), a.max(b));
            self.0[hi as usize] = lo;
        }
    }
}

fn length(pa: [u32; 3], pb: [u32; 3]) -> f64 {
    (0..3)
        .map(|i| {
            let d = f64::from(f32::from_bits(pa[i])) - f64::from(f32::from_bits(pb[i]));
            d * d
        })
        .sum::<f64>()
        .sqrt()
}

fn area(p: &[[f32; 3]; 3]) -> f64 {
    let q = p.map(|v| v.map(f64::from));
    let u = [q[1][0] - q[0][0], q[1][1] - q[0][1], q[1][2] - q[0][2]];
    let w = [q[2][0] - q[0][0], q[2][1] - q[0][1], q[2][2] - q[0][2]];
    let c = [
        u[1] * w[2] - u[2] * w[1],
        u[2] * w[0] - u[0] * w[2],
        u[0] * w[1] - u[1] * w[0],
    ];
    0.5 * (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt()
}

/// Leaf boundaries in rank order: the DFS leaf of each rank as triangles stream by.
struct LeafCursor<'a> {
    nodes: crate::spill::SequenceReader<'a, NodeRecord>,
    leaf: Option<u32>,
    end: u64,
}
impl<'a> LeafCursor<'a> {
    fn new(nodes: &'a Sequence<NodeRecord>) -> Result<Self, JobError> {
        Ok(Self {
            nodes: nodes.reader()?,
            leaf: None,
            end: 0,
        })
    }
    fn leaf_of(&mut self, rank: u64) -> Result<u32, JobError> {
        while self.leaf.is_none() || rank >= self.end {
            let node = self
                .nodes
                .next()
                .transpose()?
                .ok_or_else(|| state("triangles outlast the leaves"))?;
            if node.kind != NodeKind::Leaf {
                continue;
            }
            if node.first != rank {
                return Err(state("leaf ranges are not contiguous in rank order"));
            }
            self.leaf = Some(self.leaf.map_or(0, |l| l + 1));
            self.end = node.end;
        }
        self.leaf.ok_or_else(|| state("no leaf"))
    }
}

/// Resident bytes of a chain table for at most `edges` kept edges (at most one chain each;
/// an ordered map's nodes hold at most about twice their entries).
fn chain_table_bytes(edges: u64) -> u64 {
    edges.saturating_mul(2 * std::mem::size_of::<(u32, Chain)>() as u64 + 16)
}

#[derive(Clone, Copy, Debug)]
struct Chain {
    edges: u64,
    length: f64,
    lo: u32,
    hi: u32,
    between: bool,
}

/// Every chain (connected open edges or seam edges) by its root, from `kept`.
fn chains(
    kept: &Sequence<Kept>,
    groups: &mut Groups,
    admit: &Admit<'_>,
    name: &'static str,
    progress: &StepProgress<'_>,
) -> Result<BTreeMap<u32, Chain>, JobError> {
    groups.reset();
    for (i, e) in kept.reader()?.enumerate() {
        if i % 65_536 == 0 {
            progress.check()?;
        }
        let e = e?;
        groups.join(e.a, e.b);
    }
    (admit.memory)(name, chain_table_bytes(kept.len()))?;
    let mut table: BTreeMap<u32, Chain> = BTreeMap::new();
    for (i, e) in kept.reader()?.enumerate() {
        if i % 65_536 == 0 {
            progress.check()?;
        }
        let e = e?;
        let root = groups.find(e.a);
        let l = length(e.pa, e.pb);
        let c = table.entry(root).or_insert(Chain {
            edges: 0,
            length: 0.,
            lo: e.leaf,
            hi: e.leaf,
            between: false,
        });
        c.edges += 1;
        c.length += l;
        c.lo = c.lo.min(e.leaf);
        c.hi = c.hi.max(e.leaf);
        c.between |= e.between;
    }
    Ok(table)
}

/// Chains by size class, summed in root order (deterministic).
fn tally(table: &BTreeMap<u32, Chain>, loops: &mut Loops) {
    for c in table.values() {
        let s = &mut loops.by_size_class[class(c.length)];
        s.loops += 1;
        s.edges += c.edges;
        s.length_m += c.length;
    }
    loops.loops = table.len() as u64;
    loops.length_m = loops.by_size_class.iter().map(|s| s.length_m).sum();
}

/// Run the pass. `cells` are the internal cells' intervals (`forecast::cell_intervals`).
#[expect(
    clippy::too_many_arguments,
    reason = "the captured stores, the admissions and the run hooks are separate owners"
)]
pub(crate) fn run<'s>(
    scratch: &'s Scratch,
    sorter: SortConfig,
    triangles: &Sequence<FullTriangle>,
    nodes: &Sequence<NodeRecord>,
    leaves: u64,
    cells: &[(u32, u32, u8, u32)],
    geo: Option<GeoRequest>,
    admit: &Admit<'_>,
    progress: &'s StepProgress<'_>,
) -> Result<Topology<'s>, JobError> {
    let n = triangles.len();
    let corners = n
        .checked_mul(3)
        .ok_or_else(|| resource("corner count overflows"))?;
    let overflow = || resource("inspection scratch bound overflows");
    let retained = scratch.live_bytes();
    let mut report = TopologyReport {
        triangles: n,
        open: Loops::new(),
        seams: Loops::new(),
        ..Default::default()
    };
    (admit.memory)(
        "leaf_facts",
        leaves.saturating_mul(forecast::LEAF_FACT_BYTES),
    )?;
    let mut facts: Vec<LeafFacts> = Vec::new();
    facts
        .try_reserve_exact(usize::try_from(leaves).map_err(|_| overflow())?)
        .map_err(|_| resource("leaf facts allocation failed"))?;
    facts.resize(leaves as usize, LeafFacts::default());

    // 1. Corners by exact position: position ids, source vertices and repeated positions.
    let corner_spill = spill::<Corner>(corners, sorter).ok_or_else(overflow)?;
    (admit.scratch)(
        retained
            .checked_add(corner_spill.finish_peak)
            .ok_or_else(overflow)?,
    )?;
    progress.begin(Step::InspectVertices, n)?;
    let mut sorted = ExternalSorter::<Corner>::new(scratch, sorter)?.observed(progress);
    let mut instances: std::collections::BTreeSet<(u32, u32)> = Default::default();
    let mut recent: Option<(u32, u32)> = None;
    {
        let mut cursor = LeafCursor::new(nodes)?;
        for (rank, t) in triangles.reader()?.enumerate() {
            let rank = rank as u64;
            if rank.is_multiple_of(4096) {
                progress.advance(rank)?;
            }
            let t = t?.triangle;
            let leaf = cursor.leaf_of(rank)?;
            facts[leaf as usize].area += area(&t.positions);
            if t.degenerate {
                report.degenerate_triangles += 1;
            }
            let instance = (t.source.node, t.source.primitive);
            if recent != Some(instance) {
                recent = Some(instance);
                // Primitive instances are bounded by the document.
                if !instances.contains(&instance) {
                    // An ordered set's node share per entry, generously.
                    (admit.memory)("primitive_instance", 32)?;
                    instances.insert(instance);
                }
            }
            for k in 0..3 {
                let vertex = u32::try_from(t.vertices[k]).map_err(|_| {
                    JobError::new(
                        JobErrorKind::Unsupported,
                        "a source vertex index exceeds 2^32 - 1",
                    )
                })?;
                sorted.push(Corner {
                    position: t.positions[k].map(f32::to_bits),
                    node: t.source.node,
                    primitive: t.source.primitive,
                    vertex,
                    slot: 3 * rank + k as u64,
                })?;
            }
        }
    }
    report.primitive_instances = instances.len() as u64;
    drop(instances);
    report.surface_area_m2 = facts.iter().map(|f| f.area).sum();
    let sorted = sorted.finish()?;
    progress.end()?;
    let id_spill = spill::<SlotId>(corners, sorter).ok_or_else(overflow)?;
    (admit.scratch)(
        retained
            .checked_add(corner_spill.bytes)
            .and_then(|b| b.checked_add(corner_spill.stream_slack))
            .and_then(|b| b.checked_add(id_spill.finish_peak))
            .ok_or_else(overflow)?,
    )?;
    let mut ids = ExternalSorter::<SlotId>::new(scratch, sorter)?.observed(progress);
    let mut next: u64 = 0;
    let mut last: Option<Corner> = None;
    // Distinct source vertices at the current position.
    let mut here = 0u64;
    let close_position = |here: u64, report: &mut TopologyReport| {
        if here > 1 {
            report.repeated_positions.positions += 1;
            report.repeated_positions.extra_vertices += here - 1;
        }
    };
    for (i, c) in sorted.enumerate() {
        if i % 65_536 == 0 {
            progress.check()?;
        }
        let c = c?;
        let new_position = last.is_none_or(|l| l.position != c.position);
        if new_position {
            if last.is_some() {
                close_position(here, &mut report);
            }
            here = 0;
            next += 1;
        }
        if new_position
            || last
                .is_some_and(|l| (l.node, l.primitive, l.vertex) != (c.node, c.primitive, c.vertex))
        {
            here += 1;
            report.source_vertices += 1;
        }
        let id = u32::try_from(next - 1).map_err(|_| {
            JobError::new(
                JobErrorKind::Unsupported,
                "more than 2^32 distinct positions",
            )
        })?;
        ids.push(SlotId { slot: c.slot, id })?;
        last = Some(c);
    }
    if last.is_some() {
        close_position(here, &mut report);
    }
    report.positions = next;
    let ids = ids.finish()?;
    let positions = u32::try_from(next).map_err(|_| {
        JobError::new(
            JobErrorKind::Unsupported,
            "more than 2^32 distinct positions",
        )
    })?;

    // 2. Edges by position ids; components over every triangle.
    (admit.memory)("position_groups", u64::from(positions) * 4)?;
    let mut groups = Groups::new(positions)?;
    let edge_spill = spill::<Edge>(corners, sorter).ok_or_else(overflow)?;
    (admit.scratch)(
        retained
            .checked_add(id_spill.bytes)
            .and_then(|b| b.checked_add(id_spill.stream_slack))
            .and_then(|b| b.checked_add(edge_spill.finish_peak))
            .ok_or_else(overflow)?,
    )?;
    progress.begin(Step::InspectEdges, n)?;
    let mut edges = ExternalSorter::<Edge>::new(scratch, sorter)?.observed(progress);
    {
        let mut ids = ids;
        let mut cursor = LeafCursor::new(nodes)?;
        for (rank, t) in triangles.reader()?.enumerate() {
            let rank = rank as u64;
            if rank.is_multiple_of(4096) {
                progress.advance(rank)?;
            }
            let t = t?.triangle;
            let leaf = cursor.leaf_of(rank)?;
            let mut id = [0u32; 3];
            for (k, slot) in id.iter_mut().enumerate() {
                let s = ids
                    .next()
                    .transpose()?
                    .ok_or_else(|| state("position ids end before the triangles"))?;
                if s.slot != 3 * rank + k as u64 {
                    return Err(state("position ids are not in corner order"));
                }
                *slot = s.id;
            }
            groups.join(id[0], id[1]);
            groups.join(id[1], id[2]);
            if id[0] == id[1] || id[1] == id[2] || id[0] == id[2] {
                continue;
            }
            for k in 0..3 {
                let j = (k + 1) % 3;
                let (mut a, mut b) = (id[k], id[j]);
                let (mut pa, mut pb) = (
                    t.positions[k].map(f32::to_bits),
                    t.positions[j].map(f32::to_bits),
                );
                let index = |v: u64| {
                    u32::try_from(v).map_err(|_| state("a source vertex index changed width"))
                };
                let (mut ia, mut ib) = (index(t.vertices[k])?, index(t.vertices[j])?);
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                    std::mem::swap(&mut pa, &mut pb);
                    std::mem::swap(&mut ia, &mut ib);
                }
                edges.push(Edge {
                    a,
                    b,
                    node: t.source.node,
                    primitive: t.source.primitive,
                    ia,
                    ib,
                    leaf,
                    pa,
                    pb,
                })?;
            }
        }
        if ids.next().is_some() {
            return Err(state("position ids outlast the triangles"));
        }
    }
    report.components = (0..positions).filter(|&i| groups.find(i) == i).count() as u64;
    let edges = edges.finish()?;
    progress.end()?;

    // 3. Edge groups: open edges, primitive boundaries, seams.
    let kept_bound = corners.saturating_mul(Kept::SIZE as u64);
    (admit.scratch)(
        retained
            .checked_add(edge_spill.bytes)
            .and_then(|b| b.checked_add(edge_spill.stream_slack))
            .and_then(|b| b.checked_add(kept_bound))
            .ok_or_else(overflow)?,
    )?;
    let mut open = SequenceWriter::<Kept>::new(scratch, "inspect-open-edges", sorter.buffer_bytes)?;
    let mut seams = SequenceWriter::<Kept>::new(scratch, "inspect-seams", sorter.buffer_bytes)?;
    {
        // The current position edge: its first record, size, last primitive instance and
        // primitive-open entries; the current index edge's first record and size.
        struct Group {
            first: Edge,
            faces: u64,
            last_instance: (u32, u32),
            primitive_open: u64,
        }
        let mut group: Option<Group> = None;
        let mut sub: Option<(Edge, u64)> = None;
        let flush_sub = |sub: Option<(Edge, u64)>, group: &mut Option<Group>| {
            if let (Some((_, 1)), Some(g)) = (sub, group.as_mut()) {
                g.primitive_open += 1;
            }
        };
        let flush = |g: Group,
                     report: &mut TopologyReport,
                     open: &mut SequenceWriter<Kept>,
                     seams: &mut SequenceWriter<Kept>|
         -> Result<(), JobError> {
            let e = g.first;
            let l = length(e.pa, e.pb);
            let between = g.last_instance != (e.node, e.primitive);
            let kept = Kept {
                a: e.a,
                b: e.b,
                leaf: e.leaf,
                between,
                pa: e.pa,
                pb: e.pb,
            };
            if g.faces > 2 {
                report.nonmanifold_edges += 1;
            }
            let b = &mut report.primitive_boundaries;
            b.edges.add(g.primitive_open, l * g.primitive_open as f64);
            if g.faces == 1 {
                report.open.edges += 1;
                b.open.add(1, l);
                open.push(&kept)?;
            } else if g.primitive_open > 0 {
                if between {
                    b.coincide_with_other_primitive
                        .add(g.primitive_open, l * g.primitive_open as f64);
                } else {
                    b.unwelded_within_primitive
                        .add(g.primitive_open, l * g.primitive_open as f64);
                }
                report.repeated_positions.closes_open_edges.add(1, l);
                report.seams.edges += 1;
                seams.push(&kept)?;
            }
            Ok(())
        };
        for (i, e) in edges.enumerate() {
            if i % 65_536 == 0 {
                progress.check()?;
            }
            let e = e?;
            let same_edge = group
                .as_ref()
                .is_some_and(|g| (g.first.a, g.first.b) == (e.a, e.b));
            let same_sub = same_edge
                && sub.is_some_and(|(s, _)| {
                    (s.node, s.primitive, s.ia, s.ib) == (e.node, e.primitive, e.ia, e.ib)
                });
            if !same_sub {
                flush_sub(sub.take(), &mut group);
            }
            if !same_edge {
                if let Some(g) = group.take() {
                    flush(g, &mut report, &mut open, &mut seams)?;
                }
                group = Some(Group {
                    first: e,
                    faces: 0,
                    last_instance: (e.node, e.primitive),
                    primitive_open: 0,
                });
            }
            let g = group.as_mut().ok_or_else(|| state("no edge group"))?;
            g.faces += 1;
            g.last_instance = (e.node, e.primitive);
            sub = Some(match sub {
                Some((s, m)) if same_sub => (s, m + 1),
                _ => (e, 1),
            });
        }
        flush_sub(sub.take(), &mut group);
        if let Some(g) = group.take() {
            flush(g, &mut report, &mut open, &mut seams)?;
        }
    }
    let open = open.finish()?;
    let seams = seams.finish()?;

    // 4. Loops and seam chains; each open edge's crossing depth for the forecast.
    progress.begin(Step::InspectLoops, 3)?;
    let loops = chains(&open, &mut groups, admit, "open_loops", progress)?;
    tally(&loops, &mut report.open);
    report.open.edges = open.len();
    for (i, e) in open.reader()?.enumerate() {
        if i % 65_536 == 0 {
            progress.check()?;
        }
        let e = e?;
        let root = groups.find(e.a);
        let c = loops
            .get(&root)
            .ok_or_else(|| state("an open edge without its loop"))?;
        let k = (forecast::common_depth(cells, c.lo, c.hi) as usize).min(DEPTHS - 1);
        let f = facts
            .get_mut(e.leaf as usize)
            .ok_or_else(|| state("an open edge outside the leaves"))?;
        f.open[k] = f.open[k].saturating_add(1);
        f.length[k] += length(e.pa, e.pb) as f32;
    }
    progress.advance(1)?;
    // Selected open loops by root (each root is its component's smallest position id,
    // whatever the join order, so a later join of the same edges finds the same roots).
    let mut selected: Vec<(f64, u32, bool, Chain)> = Vec::new();
    if let Some(g) = geo {
        selected.extend(
            loops
                .iter()
                .filter(|(_, c)| c.length >= g.min_length)
                .map(|(r, c)| (c.length, *r, false, *c)),
        );
    }
    drop(loops);
    let seam_chains = chains(&seams, &mut groups, admit, "seam_chains", progress)?;
    tally(&seam_chains, &mut report.seams);
    report.seams.edges = seams.len();
    progress.advance(2)?;
    let geo = match geo {
        None => {
            progress.end()?;
            None
        }
        Some(g) => {
            selected.extend(
                seam_chains
                    .iter()
                    .filter(|(_, c)| c.length >= g.min_length)
                    .map(|(r, c)| (c.length, *r, true, *c)),
            );
            drop(seam_chains);
            // Longest first; ties by kind, then root (deterministic).
            selected.sort_by(|a, b| {
                b.0.total_cmp(&a.0)
                    .then_with(|| a.2.cmp(&b.2))
                    .then_with(|| a.1.cmp(&b.1))
            });
            let mut budget = g.max_segments;
            let mut features = Vec::new();
            let mut omitted = 0u64;
            // Feature number by root, per kind.
            let mut chosen: [BTreeMap<u32, u32>; 2] = [BTreeMap::new(), BTreeMap::new()];
            for (len, root, seam, c) in selected {
                if c.edges > budget {
                    omitted += 1;
                    continue;
                }
                budget -= c.edges;
                chosen[usize::from(seam)].insert(root, features.len() as u32);
                features.push(LineFeature {
                    kind: if seam { "seam" } else { "open_loop" },
                    edges: c.edges,
                    length_m: len,
                    size_class: CLASSES[class(len)],
                    between_primitives: seam.then_some(c.between),
                });
            }
            let segments = g.max_segments - budget;
            let geo_spill = spill::<GeoSegment>(segments, sorter).ok_or_else(overflow)?;
            (admit.scratch)(
                scratch
                    .live_bytes()
                    .checked_add(geo_spill.finish_peak)
                    .and_then(|b| b.checked_add(geo_spill.stream_slack))
                    .ok_or_else(overflow)?,
            )?;
            let mut sorted = ExternalSorter::<GeoSegment>::new(scratch, sorter)?.observed(progress);
            // The groups hold the seam join; the open loops are joined again after.
            for (seam, kept) in [(true, &seams), (false, &open)] {
                if !seam {
                    groups.reset();
                    for e in open.reader()? {
                        let e = e?;
                        groups.join(e.a, e.b);
                    }
                }
                for (i, e) in kept.reader()?.enumerate() {
                    if i % 65_536 == 0 {
                        progress.check()?;
                    }
                    let e = e?;
                    if let Some(&feature) = chosen[usize::from(seam)].get(&groups.find(e.a)) {
                        sorted.push(GeoSegment {
                            feature,
                            pa: e.pa,
                            pb: e.pb,
                        })?;
                    }
                }
            }
            let segments = sorted.finish()?;
            progress.end()?;
            Some(GeoLines {
                features,
                segments,
                omitted,
            })
        }
    };
    drop(groups);
    drop(open);
    drop(seams);
    Ok(Topology {
        report,
        leaves: facts,
        geo,
    })
}

/// Worst-case scratch of the pass beside its retained files, for a memory-backed scratch
/// charge: the largest of its phases with every corner kept.
pub(crate) fn scratch_need(triangles: u64, sorter: SortConfig) -> Option<u64> {
    let corners = triangles.checked_mul(3)?;
    let corner = spill::<Corner>(corners, sorter)?;
    let id = spill::<SlotId>(corners, sorter)?;
    let edge = spill::<Edge>(corners, sorter)?;
    let kept = corners.checked_mul(2 * Kept::SIZE as u64)?;
    let geo = spill::<GeoSegment>(corners, sorter)?;
    Some(
        corner
            .finish_peak
            .max(corner.bytes + corner.stream_slack + id.finish_peak)
            .max(id.bytes + id.stream_slack + edge.finish_peak)
            .max(edge.bytes + edge.stream_slack + kept)
            .max(kept + geo.finish_peak + geo.stream_slack),
    )
}

/// Resident bound of the pass besides its admitted arrays: the sorters live at once (one
/// streaming into the next) and the sequence writers and readers.
pub(crate) fn memory_need(sorter: SortConfig) -> u64 {
    sorter
        .memory_bound::<Corner>()
        .max(sorter.memory_bound::<Edge>())
        .saturating_add(
            sorter
                .memory_bound::<SlotId>()
                .max(sorter.memory_bound::<GeoSegment>()),
        )
        .saturating_add(6 * sorter.buffer_bytes as u64)
}
