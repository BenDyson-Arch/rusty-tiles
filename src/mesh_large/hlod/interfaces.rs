//! Spillable source incidence: which decoded points and segments each leaf shares with
//! other leaves, and over which interval of DFS leaf indices.
//!
//! Vertices are welded by their leaf-canonical decoded position (`LatticePlan::canonical`):
//! the coordinates every leaf publishes, so two source vertices that decode to one lattice
//! point are one vertex, and a triangle whose corners decode onto fewer than three points
//! carries no surface (it is skipped, as the decoded census skips it).
//!
//! Two kinds of sharing are recorded:
//! - **exact**: a decoded point or segment that several leaves hold;
//! - **overlap** (T-junctions and partial collinear overlaps): an open-border point of one
//!   leaf lying on an open-border segment of another (within the leaves' decode tolerance).
//!   The segment's leaf must keep that segment (and so its endpoints), the point's leaf
//!   must keep the point; open edges between kept points are kept too (see `read_leaf`).
//!   Overlaps are found by an out-of-core kd join bounded by design (`kd_join`), which emits
//!   one row per matched segment or point with the union of its partners' intervals.
//!
//! Leaves are contiguous DFS intervals of the frozen tree, so a tile whose leaves are
//! `[begin, end)` must keep every recorded item whose leaf interval `[lo, hi]` leaves its
//! own. Rows are merged per (leaf, item) to the union of their intervals, which is exact:
//! some incident leaf lies outside a tile iff the union does. Nothing input-sized stays
//! resident (external sorts, one leaf of items, one group, a bounded set of held hosts).
use crate::{
    mesh_large::{
        attributes::FullTriangle,
        config::{self, SortSpill},
        full::CornerDepth,
        leaves::geometry::LatticePlan,
        records::{NodeKind, NodeRecord},
    },
    spill::{
        ExternalSorter, Record, Scratch, Sequence, SequenceWriter, SlotFile, SortConfig,
        SortedStream,
    },
    JobError, JobErrorKind,
};
use std::collections::HashMap;

fn state(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.to_owned())
}

/// A canonical decoded position as f32 bit patterns: the weld key.
pub(crate) type Key = [u32; 3];

pub(crate) fn key(p: [f32; 3]) -> Key {
    p.map(f32::to_bits)
}
fn point(k: Key) -> [f64; 3] {
    k.map(|b| f64::from(f32::from_bits(b)))
}

fn put(out: &mut [u8], at: usize, k: Key) {
    for (i, v) in k.iter().enumerate() {
        out[at + 4 * i..at + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
}
fn get(bytes: &[u8], at: usize) -> Key {
    std::array::from_fn(|i| {
        u32::from_le_bytes(bytes[at + 4 * i..at + 4 * i + 4].try_into().unwrap())
    })
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// One decoded point (`a == b`) or segment (`a < b`) of a leaf, with whether it lies on the
/// leaf's open border (a segment held by other than two of its faces, or such a segment's
/// endpoint) and the leaf's decode tolerance (f32 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Item {
    a: Key,
    b: Key,
    leaf: u32,
    open: bool,
    tol: u32,
}

impl Record for Item {
    const SIZE: usize = 33;
    fn encode(&self, out: &mut [u8]) {
        put(out, 0, self.a);
        put(out, 12, self.b);
        out[24..28].copy_from_slice(&self.leaf.to_le_bytes());
        out[28] = u8::from(self.open);
        out[29..33].copy_from_slice(&self.tol.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            a: get(bytes, 0),
            b: get(bytes, 12),
            leaf: u32_at(bytes, 24),
            open: bytes[28] != 0,
            tol: u32_at(bytes, 29),
        }
    }
}

/// A decoded point (`a == b`) or segment (`a < b`) shared with the DFS leaves `lo..=hi`,
/// recorded for one incident `leaf`.
///
/// Two intervals: `lo..=hi` locks (every partner leaf, certain or provisional) and
/// `clo..=chi` is certain (exact matches and shared items; `clo > chi` when none). They
/// differ only where an over-dense join region matched a superset (`kd_join`); such a row is
/// provisional, and a tile it keeps from its budget resolves it exactly
/// (`Fallback::resolve`).
///
/// `tol` is the largest decode tolerance (f32 bits) among the row's owners whose candidate
/// for this item such a region matched by a superset (`NO_TOL` when none). Rows of several
/// owners merge into one (`absorb`) without losing it, and the join's predicate only widens
/// with the tolerance, so re-testing the item once at `tol` finds exactly the union of those
/// owners' partners.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct InterfaceRow {
    pub leaf: u32,
    pub a: Key,
    pub b: Key,
    pub lo: u32,
    pub hi: u32,
    pub clo: u32,
    pub chi: u32,
    pub tol: u32,
}

/// No certain partner (an empty interval).
pub(crate) const NO_LEAF: (u32, u32) = (u32::MAX, 0);
/// No provisional candidate among a row's owners.
pub(crate) const NO_TOL: u32 = u32::MAX;

/// The larger of two row tolerances (`NO_TOL` is absent).
fn max_tol(a: u32, b: u32) -> u32 {
    match (a, b) {
        (NO_TOL, t) | (t, NO_TOL) => t,
        (a, b) => f32::from_bits(a).max(f32::from_bits(b)).to_bits(),
    }
}

impl InterfaceRow {
    /// A row whose whole interval is certain.
    pub(crate) fn certain(leaf: u32, a: Key, b: Key, lo: u32, hi: u32) -> Self {
        Self {
            leaf,
            a,
            b,
            lo,
            hi,
            clo: lo,
            chi: hi,
            tol: NO_TOL,
        }
    }
    /// Whether some of its lock interval is only provisional.
    pub(crate) fn provisional(&self) -> bool {
        (self.lo, self.hi) != (self.clo, self.chi)
    }
    /// Union with another row of the same item, channel by channel.
    pub(crate) fn absorb(&mut self, other: &Self) {
        self.lo = self.lo.min(other.lo);
        self.hi = self.hi.max(other.hi);
        self.clo = self.clo.min(other.clo);
        self.chi = self.chi.max(other.chi);
        self.tol = max_tol(self.tol, other.tol);
    }
    pub(crate) fn is_vertex(&self) -> bool {
        self.a == self.b
    }
    /// Whether a tile over leaves `[begin, end)` holding this row shares it with a tile
    /// outside, i.e. must retain it as an outer interface.
    pub(crate) fn outer(&self, begin: u32, end: u32) -> bool {
        self.lo < begin || self.hi >= end
    }
    /// The item, without its recording leaf, intervals or tolerance.
    pub(crate) fn item(&self) -> (Key, Key) {
        (self.a, self.b)
    }
}

/// Sort rows by item and merge each item's rows into one: intervals by union (exact: see the
/// module notes), the provisional tolerance by maximum (exact for resolution: see
/// `InterfaceRow`). The merged row's `leaf` is one of its owners.
pub(crate) fn merge_rows(rows: &mut Vec<InterfaceRow>) {
    rows.sort_unstable_by_key(|r| (r.a, r.b, r.lo, r.hi));
    let mut out: Vec<InterfaceRow> = Vec::with_capacity(rows.len());
    for r in rows.drain(..) {
        match out.last_mut() {
            Some(last) if last.item() == r.item() => last.absorb(&r),
            _ => out.push(r),
        }
    }
    out.shrink_to_fit();
    *rows = out;
}

impl Record for InterfaceRow {
    const SIZE: usize = 48;
    fn encode(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.leaf.to_le_bytes());
        put(out, 4, self.a);
        put(out, 16, self.b);
        out[28..32].copy_from_slice(&self.lo.to_le_bytes());
        out[32..36].copy_from_slice(&self.hi.to_le_bytes());
        out[36..40].copy_from_slice(&self.clo.to_le_bytes());
        out[40..44].copy_from_slice(&self.chi.to_le_bytes());
        out[44..48].copy_from_slice(&self.tol.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            leaf: u32_at(bytes, 0),
            a: get(bytes, 4),
            b: get(bytes, 16),
            lo: u32_at(bytes, 28),
            hi: u32_at(bytes, 32),
            clo: u32_at(bytes, 36),
            chi: u32_at(bytes, 40),
            tol: u32_at(bytes, 44),
        }
    }
}

/// An open-border segment (host) or point (guest, `a == b`) of a leaf, written during the
/// reduce for the overlap join.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Candidate {
    leaf: u32,
    a: Key,
    b: Key,
    tol: u32,
}

impl Record for Candidate {
    const SIZE: usize = 32;
    fn encode(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.leaf.to_le_bytes());
        put(out, 4, self.a);
        put(out, 16, self.b);
        out[28..32].copy_from_slice(&self.tol.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            leaf: u32_at(bytes, 0),
            a: get(bytes, 4),
            b: get(bytes, 16),
            tol: u32_at(bytes, 28),
        }
    }
}

/// What a sort of `records` records spills: nothing while they fit one run (the sorter keeps
/// them in memory), else every record (`config::sort_spill`).
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

/// What capture declares for the incidence pass up front: nothing beyond the files it
/// retains (its `leaf_plans` stage). Every phase of the pass is admitted as it starts, from
/// the counts known then (`ScratchPhases`), so no run is refused for a worst case it never
/// reaches.
pub(crate) fn scratch_need(_triangles: u64, _sorter: SortConfig) -> Option<u64> {
    Some(0)
}

/// The incidence pass's staged scratch declarations. Each is the bytes live when the pass
/// starts (capture's retained files) plus its phase's bound from the counts known when that
/// phase starts, and is admitted (free space preflighted) then. Phases never overlap, so the
/// pass declares their maximum, not their sum:
///
/// - items: the item sort, admitted one run ahead while items are pushed, then through
///   `finish` (from the items pushed);
/// - reduce: the sorted items being streamed, the exact rows pushed and the open candidates
///   written (from the item count: each item gives at most one exact row and one candidate);
/// - join: the rows (exact so far and at most one overlap row per candidate) and the overlap
///   join's allowance (`overlap_allowance`, from the candidate and group counts);
/// - walk: the row sort through `finish` and its stream beside the fallback index (from the
///   row count and the index's size); the candidates are gone by then.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScratchPhases {
    pub retained: u64,
    pub items: u64,
    pub reduce: u64,
    pub join: u64,
    pub walk: u64,
}

/// Phase bounds (without the retained files) from the counts known when each phase starts.
fn items_phase(items: u64, sorter: SortConfig) -> Option<u64> {
    Some(spill::<Item>(items, sorter)?.finish_peak)
}
/// The item stream, at most one exact row per item and one candidate per open item.
fn reduce_phase(items: u64, open: u64, sorter: SortConfig) -> Option<u64> {
    let item = spill::<Item>(items, sorter)?;
    item.bytes
        .checked_add(item.stream_slack)?
        .checked_add(spill::<InterfaceRow>(items, sorter)?.push_peak)?
        .checked_add(open.checked_mul(Candidate::SIZE as u64)?)
}
/// The rows so far (`rows`) plus at most one per candidate, and the join's allowance.
fn join_phase(
    rows: u64,
    candidates: u64,
    host_groups: u64,
    guest_groups: u64,
    sorter: SortConfig,
) -> Option<u64> {
    spill::<InterfaceRow>(rows.checked_add(candidates)?, sorter)?
        .push_peak
        .checked_add(overlap_allowance(
            candidates,
            host_groups,
            guest_groups,
            sorter,
        )?)
}
fn walk_phase(rows: u64, fallback: u64, sorter: SortConfig) -> Option<u64> {
    let row = spill::<InterfaceRow>(rows, sorter)?;
    row.finish_peak
        .checked_add(row.stream_slack)?
        .checked_add(fallback)
}

/// The pass's worst case for `triangles` source triangles, charged to memory when scratch is
/// memory-backed (staged admission cannot take memory nobody charged): a triangle gives at
/// most six items (three points, three segments), each at most one exact row, one candidate
/// (its own group) and one overlap row.
pub(crate) fn memory_backed_need(triangles: u64, sorter: SortConfig) -> Option<u64> {
    let items = triangles.checked_mul(6)?;
    let groups = triangles.checked_mul(3)?;
    let index = Index::bytes_for(groups)?.checked_mul(2)?;
    Some(
        items_phase(items, sorter)?
            .max(reduce_phase(items, items, sorter)?)
            .max(join_phase(items, items, groups, groups, sorter)?)
            .max(walk_phase(items.checked_mul(2)?, index, sorter)?),
    )
}

impl ScratchPhases {
    /// The pass's declared bound: its largest phase.
    pub(crate) fn declared(&self) -> u64 {
        self.retained
            .max(self.items)
            .max(self.reduce)
            .max(self.join)
            .max(self.walk)
    }
}

/// Resident bound of the pass: two sorters live at once (items streamed into rows, then the
/// rows beside one side's kept groups), one leaf's deduplicated items, one item group (at
/// most one item per leaf, in a vector at most twice that), and the overlap join's held
/// hosts and open sequence buffers.
pub(crate) fn memory_need(sorter: SortConfig, leaf_triangles: u64, leaves: u64) -> u64 {
    let item = std::mem::size_of::<Item>() as u64;
    sorter
        .memory_bound::<Item>()
        .saturating_add(sorter.memory_bound::<InterfaceRow>())
        // The kept groups' sort (one side at a time) replaces the consumed item sort.
        .saturating_add(
            sorter
                .memory_bound::<Placed>()
                .saturating_sub(sorter.memory_bound::<Item>()),
        )
        .saturating_add(leaf_triangles.saturating_mul(12 * item))
        .saturating_add(leaves.saturating_mul(2 * item))
        .saturating_add(((EXACT_HOSTS + SCAN_HOSTS) * std::mem::size_of::<Held>()) as u64)
        .saturating_add(KD_STREAMS.saturating_mul(sorter.buffer_bytes as u64))
}

/// Totals of the pass, for the report.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IncidenceStats {
    pub leaves: u64,
    pub items: u64,
    /// Source triangles whose corners decode onto fewer than three points.
    pub decoded_degenerate: u64,
    pub shared_points: u64,
    pub shared_segments: u64,
    /// Segments shared by three or more leaves.
    pub multi_leaf_segments: u64,
    pub open_segments: u64,
    pub open_points: u64,
    /// The overlap join's kd regions, splits, exact leaves, terminal regions no split could
    /// shorten (scanned exactly), splits whose long hosts were spilled, extra guest passes of
    /// chunked scans, and exact predicate tests.
    pub kd_regions: u64,
    pub kd_splits: u64,
    pub exact_leaves: u64,
    pub terminal_scans: u64,
    pub long_spills: u64,
    pub scan_passes: u64,
    pub tests: u64,
    /// Regions whose planned exact tests passed `BUDGET_TESTS_PER_RECORD`, matched by a
    /// superset (provisional rows, resolved exactly where a tile needs it).
    pub fallback_regions: u64,
    /// Candidates of the groups kept for that resolution, and the groups (each indexed once).
    pub fallback_candidates: u64,
    pub fallback_groups: u64,
    /// Rows emitted provisional (their lock wider than their certain interval).
    pub provisional_rows: u64,
    /// Distinct (item, tolerance) groups of open segments and of open points.
    pub host_groups: u64,
    pub guest_groups: u64,
    /// Hosts and guests given an overlap row.
    pub matched_hosts: u64,
    pub matched_guests: u64,
    /// Open points of one leaf found on an open segment of another (T-junctions and
    /// collinear overlaps).
    pub overlaps: u64,
    pub rows: u64,
    /// The pass's staged scratch declarations (run diagnostics, not the manifest).
    #[serde(skip)]
    pub scratch: ScratchPhases,
}

/// One triangle's canonical corners, or `None` when they decode onto fewer than three
/// points (no surface).
pub(crate) fn canonical_corners(
    lattice: &LatticePlan,
    t: &FullTriangle,
    ell: [u8; 3],
) -> Result<Option<[[f32; 3]; 3]>, JobError> {
    let mut c = [[0f32; 3]; 3];
    for k in 0..3 {
        c[k] = lattice.canonical(t.triangle.positions[k], ell[k])?;
    }
    let k = c.map(key);
    Ok((k[0] != k[1] && k[1] != k[2] && k[0] != k[2]).then_some(c))
}

/// How far a collinear neighbour's decode may sit off a segment because of the lattice: the
/// item's own snap step (zero in float profiles).
fn leaf_tolerance(lattice: &LatticePlan, ell: u8) -> f32 {
    if matches!(
        lattice.profile(),
        crate::mesh_large::config::Profile::Quality | crate::mesh_large::config::Profile::Lossless
    ) {
        0.
    } else {
        lattice.steps()[usize::from(ell)]
    }
}

/// A float-rounding floor local to the points involved: a few ulps of their largest
/// coordinate.
fn rounding(points: &[Key]) -> f64 {
    let magnitude = points
        .iter()
        .flat_map(|k| point(*k))
        .map(f64::abs)
        .fold(0., f64::max);
    magnitude * f64::from(f32::EPSILON) * 4.
}

/// Whether `p` lies on segment `ab` within `tol`, other than at an endpoint.
pub(crate) fn on_segment(a: Key, b: Key, p: Key, tol: f64) -> bool {
    if p == a || p == b {
        return false;
    }
    let (a, b, p) = (point(a), point(b), point(p));
    let d: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
    let length2: f64 = d.iter().map(|v| v * v).sum();
    if length2 == 0. {
        return false;
    }
    let t = (0..3).map(|i| (p[i] - a[i]) * d[i]).sum::<f64>() / length2;
    let length = length2.sqrt();
    if t * length < -tol || t * length > length + tol {
        return false;
    }
    let perpendicular2: f64 = (0..3).map(|i| (p[i] - a[i] - t * d[i]).powi(2)).sum();
    perpendicular2 <= tol * tol
}

/// Every leaf's interface rows (exact and overlap), sorted by (leaf, a, b).
///
/// `admit` preflights free space for a phase's whole declared bound (live bytes included)
/// before the phase writes (`ScratchPhases`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build<'s>(
    scratch: &'s Scratch,
    sorter: SortConfig,
    nodes: &Sequence<NodeRecord>,
    triangles: &Sequence<FullTriangle>,
    depths: &Sequence<CornerDepth>,
    lattice: &LatticePlan,
    admit: &dyn Fn(u64) -> Result<(), JobError>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(SortedStream<'s, InterfaceRow>, IncidenceStats, Fallback), JobError> {
    let overflow = || state("incidence scratch bound overflows");
    let mut stats = IncidenceStats::default();
    let retained = scratch.live_bytes();
    stats.scratch.retained = retained;
    let mut items = ExternalSorter::<Item>::new(scratch, sorter)?;
    // Items are admitted one run ahead of those pushed (a run spills only once full).
    let item_run = (sorter.run_bytes / std::mem::size_of::<Item>()).max(1) as u64;
    let mut admitted_items = item_run;
    // Open (leaf, item) pairs: exactly the candidates the reduce writes.
    let mut open_items = 0u64;
    let mut source = triangles.reader()?;
    let mut ells = depths.reader()?;
    let mut rank = 0u64;
    let mut leaf = 0u32;
    // (a, b, tol a, tol b): each endpoint's own snap step.
    let mut segments: Vec<(Key, Key, f32, f32)> = Vec::new();
    let mut buffer: Vec<Item> = Vec::new();
    for node in nodes.reader()? {
        let node = node?;
        if node.kind != NodeKind::Leaf {
            continue;
        }
        if node.first != rank {
            return Err(state("leaf ranges are not contiguous in DFS order"));
        }
        segments.clear();
        buffer.clear();
        for _ in node.first..node.end {
            if rank.is_multiple_of(4096) {
                check()?;
            }
            let t = source
                .next()
                .transpose()?
                .ok_or_else(|| state("source triangles end before the leaves"))?;
            let ell = ells
                .next()
                .transpose()?
                .ok_or_else(|| state("corner depths end before the leaves"))?
                .depths;
            rank += 1;
            let Some(c) = canonical_corners(lattice, &t, ell)? else {
                stats.decoded_degenerate += 1;
                continue;
            };
            let k = c.map(key);
            let t = ell.map(|e| leaf_tolerance(lattice, e));
            for i in 0..3 {
                let j = (i + 1) % 3;
                let (p, q, tp, tq) = if k[i] <= k[j] {
                    (k[i], k[j], t[i], t[j])
                } else {
                    (k[j], k[i], t[j], t[i])
                };
                segments.push((p, q, tp, tq));
            }
        }
        // Open border: segments held by other than exactly two of the leaf's faces.
        segments.sort_unstable_by_key(|x| (x.0, x.1));
        let mut i = 0;
        while i < segments.len() {
            let mut j = i + 1;
            let (a, b, mut ta, mut tb) = segments[i];
            while j < segments.len() && (segments[j].0, segments[j].1) == (a, b) {
                ta = ta.max(segments[j].2);
                tb = tb.max(segments[j].3);
                j += 1;
            }
            let open = j - i != 2;
            buffer.push(Item {
                a,
                b,
                leaf,
                open,
                tol: ta.max(tb).to_bits(),
            });
            for (p, tp) in [(a, ta), (b, tb)] {
                buffer.push(Item {
                    a: p,
                    b: p,
                    leaf,
                    open,
                    tol: tp.to_bits(),
                });
            }
            i = j;
        }
        // A point is open when any of its segments is.
        buffer.sort_unstable();
        buffer.dedup_by(|later, earlier| {
            if (later.a, later.b) == (earlier.a, earlier.b) {
                earlier.open |= later.open;
                earlier.tol = f32::from_bits(earlier.tol)
                    .max(f32::from_bits(later.tol))
                    .to_bits();
                true
            } else {
                false
            }
        });
        let after = stats.items + buffer.len() as u64;
        if after > admitted_items {
            admitted_items = after + item_run;
            let spilled = spill::<Item>(admitted_items, sorter).ok_or_else(overflow)?;
            admit(retained + spilled.push_peak)?;
        }
        stats.items = after;
        open_items += buffer.iter().filter(|i| i.open).count() as u64;
        for item in &buffer {
            items.push(*item)?;
        }
        leaf = leaf
            .checked_add(1)
            .ok_or_else(|| state("leaf index overflow"))?;
    }
    if source.next().transpose()?.is_some() {
        return Err(state("source triangles outlast the leaves"));
    }
    stats.leaves = u64::from(leaf);
    drop(buffer);
    drop(segments);
    stats.scratch.items = items_phase(stats.items, sorter)
        .and_then(|b| b.checked_add(retained))
        .ok_or_else(overflow)?;
    admit(stats.scratch.items)?;
    stats.scratch.reduce = reduce_phase(stats.items, open_items, sorter)
        .and_then(|b| b.checked_add(retained))
        .ok_or_else(overflow)?;
    admit(stats.scratch.reduce)?;
    let mut rows = ExternalSorter::<InterfaceRow>::new(scratch, sorter)?;
    let mut hosts = SequenceWriter::<Candidate>::new(scratch, "hlod-hosts", sorter.buffer_bytes)?;
    let mut guests = SequenceWriter::<Candidate>::new(scratch, "hlod-guests", sorter.buffer_bytes)?;
    let mut group: Vec<Item> = Vec::new();
    let leaves = u64::from(leaf);
    let flush = |group: &mut Vec<Item>,
                 rows: &mut ExternalSorter<'s, InterfaceRow>,
                 hosts: &mut SequenceWriter<Candidate>,
                 guests: &mut SequenceWriter<Candidate>,
                 stats: &mut IncidenceStats|
     -> Result<(), JobError> {
        let (a, b) = (group[0].a, group[0].b);
        if group.len() > 1 {
            let (lo, hi) = (group[0].leaf, group[group.len() - 1].leaf);
            if a == b {
                stats.shared_points += 1;
            } else {
                stats.shared_segments += 1;
                stats.multi_leaf_segments += u64::from(group.len() > 2);
            }
            for g in group.iter() {
                rows.push(InterfaceRow::certain(g.leaf, a, b, lo, hi))?;
                stats.rows += 1;
            }
        }
        // Open members by tolerance (then leaf): the join reads each run as one group.
        group.sort_unstable_by_key(|g| (g.tol, g.leaf));
        for g in group.iter().filter(|g| g.open) {
            let candidate = Candidate {
                leaf: g.leaf,
                a,
                b,
                tol: g.tol,
            };
            if a == b {
                guests.push(&candidate)?;
                stats.open_points += 1;
            } else {
                hosts.push(&candidate)?;
                stats.open_segments += 1;
            }
        }
        group.clear();
        Ok(())
    };
    let mut seen = 0u64;
    for item in items.finish()? {
        let item = item?;
        seen += 1;
        if seen.is_multiple_of(1 << 16) {
            check()?;
        }
        if group
            .first()
            .is_some_and(|g| (g.a, g.b) != (item.a, item.b))
        {
            flush(&mut group, &mut rows, &mut hosts, &mut guests, &mut stats)?;
        }
        // Items are unique per leaf and sorted, so the group's leaves ascend strictly and a
        // group holds at most one item per leaf (charged in `memory_need`).
        if group.len() as u64 >= leaves {
            return Err(state(
                "an interface item group holds more items than leaves",
            ));
        }
        group.push(item);
    }
    if !group.is_empty() {
        flush(&mut group, &mut rows, &mut hosts, &mut guests, &mut stats)?;
    }
    let hosts = hosts.finish()?;
    let guests = guests.finish()?;
    // Without candidates, the join phase is the rows alone.
    stats.scratch.join = spill::<InterfaceRow>(stats.rows, sorter)
        .and_then(|s| s.push_peak.checked_add(retained))
        .ok_or_else(overflow)?;
    let fallback = kd_join(
        scratch, sorter, &hosts, &guests, &mut rows, &mut stats, admit, check,
    )?;
    drop(hosts);
    drop(guests);
    stats.scratch.walk = walk_phase(stats.rows, fallback.bytes(), sorter)
        .and_then(|b| b.checked_add(retained))
        .ok_or_else(overflow)?;
    admit(stats.scratch.walk)?;
    Ok((rows.finish()?, stats, fallback))
}

/// The least two and greatest two distinct leaves of a set: enough to give, for any one
/// leaf, the least and greatest *other* leaf of the set. Merging two sets' extremes gives
/// their union's. `u32::MAX` encodes an absent entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Extremes {
    min: [u32; 2],
    max: [u32; 2],
}

const NONE: u32 = u32::MAX;

impl Extremes {
    const EMPTY: Self = Self {
        min: [NONE; 2],
        max: [NONE; 2],
    };

    fn is_empty(&self) -> bool {
        self.min[0] == NONE
    }

    fn add(&mut self, leaf: u32) {
        let [a, b] = self.min;
        if a == NONE {
            self.min = [leaf, NONE];
        } else if leaf < a {
            self.min = [leaf, a];
        } else if leaf > a && (b == NONE || leaf < b) {
            self.min[1] = leaf;
        }
        let [a, b] = self.max;
        if a == NONE {
            self.max = [leaf, NONE];
        } else if leaf > a {
            self.max = [leaf, a];
        } else if leaf < a && (b == NONE || leaf > b) {
            self.max[1] = leaf;
        }
    }

    fn merge(&mut self, other: &Self) {
        for leaf in other.min.into_iter().chain(other.max) {
            if leaf != NONE {
                self.add(leaf);
            }
        }
    }

    /// The least and greatest leaf of the set other than `own`, if any.
    fn others(&self, own: u32) -> Option<(u32, u32)> {
        let pick = |v: [u32; 2]| {
            let x = if v[0] == own { v[1] } else { v[0] };
            (x != NONE).then_some(x)
        };
        Some((pick(self.min)?, pick(self.max)?))
    }

    /// Whether some leaf of `self` differs from some leaf of `other` (both nonempty).
    fn differ(&self, other: &Self) -> bool {
        !(self.min[1] == NONE && other.min[1] == NONE && self.min[0] == other.min[0])
    }

    fn put(&self, out: &mut [u8]) {
        for (i, v) in self.min.into_iter().chain(self.max).enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
    }

    fn get(bytes: &[u8]) -> Self {
        Self {
            min: [u32_at(bytes, 0), u32_at(bytes, 4)],
            max: [u32_at(bytes, 8), u32_at(bytes, 12)],
        }
    }
}

/// Extremes in one half of a result slot (an unwritten slot reads as zeros: none yet).
fn slot_extremes(bytes: &[u8]) -> Extremes {
    if bytes.iter().all(|b| *b == 0) {
        Extremes::EMPTY
    } else {
        Extremes::get(bytes)
    }
}

/// A host group id (an over-budget region's hosts past those held).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct HostId(u32);

impl Record for HostId {
    const SIZE: usize = 4;
    fn encode(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.0.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self(u32_at(bytes, 0))
    }
}

/// One host group copy in the kd join: the distinct open segment `ab` at tolerance `tol`
/// (f32 bits) held open by the leaves summarized in `leaves`, its group index (`id`, its
/// result slot) and the axes it has been cut along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KdHost {
    id: u32,
    a: Key,
    b: Key,
    tol: u32,
    leaves: Extremes,
    cut: u8,
}

impl Record for KdHost {
    const SIZE: usize = 49;
    fn encode(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.id.to_le_bytes());
        put(out, 4, self.a);
        put(out, 16, self.b);
        out[28..32].copy_from_slice(&self.tol.to_le_bytes());
        self.leaves.put(&mut out[32..48]);
        out[48] = self.cut;
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            id: u32_at(bytes, 0),
            a: get(bytes, 4),
            b: get(bytes, 16),
            tol: u32_at(bytes, 28),
            leaves: Extremes::get(&bytes[32..48]),
            cut: bytes[48],
        }
    }
}

/// One guest group in the kd join: the distinct open point `p` at tolerance `tol` held by
/// the leaves summarized in `leaves`, and its group index (`id`, its result slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KdGuest {
    id: u32,
    p: Key,
    tol: u32,
    leaves: Extremes,
}

impl Record for KdGuest {
    const SIZE: usize = 36;
    fn encode(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.id.to_le_bytes());
        put(out, 4, self.p);
        out[16..20].copy_from_slice(&self.tol.to_le_bytes());
        self.leaves.put(&mut out[20..36]);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            id: u32_at(bytes, 0),
            p: get(bytes, 4),
            tol: u32_at(bytes, 16),
            leaves: Extremes::get(&bytes[20..36]),
        }
    }
}

/// Hosts of a region tested exactly against each of its guests (a kd leaf).
pub(crate) const EXACT_HOSTS: usize = 64;
/// Hosts held at once while a region's long hosts (or a terminal region's hosts) are tested
/// exactly against its guests; more are tested in further chunks, each one more pass over
/// the region's guests.
pub(crate) const SCAN_HOSTS: usize = 4096;
/// Cuts a short host may take; each along a distinct axis, so at most `2^MAX_CUTS` copies.
const MAX_CUTS: u32 = 2;
/// A guard on splits along one path; geometry ends every path first (see `kd_join`).
const MAX_DEPTH: u32 = 64;
/// One group's result slot: the extremes of the leaves of its exactly matched partners, then
/// of its provisionally matched ones (an over-budget region's superset).
const SLOT: usize = 32;
/// Planned exact tests per record a region may spend on its long (or terminal) hosts before
/// it matches them by a superset instead: one budget, deterministic in the region's counts.
/// A few hundred long hosts per region (sliver bundles, the round-4 ribbons case) stay exact.
pub(crate) const BUDGET_TESTS_PER_RECORD: u64 = 128;
/// Sequence buffers open at once: a region's host or guest reader, two child writers and a
/// long-host spill or staged-guest writer.
const KD_STREAMS: u64 = 4;
/// Scratch files the kd join holds at once (for the scratch admission's per-file rounding):
/// a pending right child's hosts and guests per split on the path, the region being split
/// with its child writers, long spill and staged guests, the two slot files and both sides'
/// fallback indexes (entries and node levels; sort runs are rounded as chunks).
pub(crate) const KD_MAX_FILES: u64 =
    2 * (MAX_DEPTH as u64 + 1) + 2 + 6 + 2 + 2 * (1 + INDEX_MAX_LEVELS as u64);

/// Proven live scratch of the overlap join per host group: its result slot, at most
/// `2^MAX_CUTS + 2` kd copies and, in an over-budget region, its id past the held ones. A
/// host branches only where cut, so at most `2^MAX_CUTS` branches each hold one copy; in the
/// region being split it also has the copy being read and either two being written (cut) or
/// one in the long spill. 330 bytes.
pub(crate) const OVERLAP_BYTES_PER_HOST_GROUP: u64 =
    SLOT as u64 + ((1 << MAX_CUTS) + 2) * KdHost::SIZE as u64 + HostId::SIZE as u64;
/// Per guest group: its result slot and at most two kd copies (the one being read and the one
/// being written: a child's, or a chunked scan's staged copy, each pass releasing the one
/// before). 104 bytes.
pub(crate) const OVERLAP_BYTES_PER_GUEST_GROUP: u64 = SLOT as u64 + 2 * KdGuest::SIZE as u64;

/// The scratch allowance of the overlap join for `candidates` members in `host_groups` and
/// `guest_groups` groups: the larger of its two phases (counted, so no floor; file rounding
/// is admitted beside it).
///
/// - regions: the candidates (kept from the reduce for the final rows) and the proven live
///   bytes of the kd split per group (`OVERLAP_BYTES_PER_HOST_GROUP`, `..._GUEST_GROUP`);
/// - fallback: the candidates and result slots while the rows are emitted, the provisional
///   groups' sort (`Placed`, one side at a time: at most every group of that side) and both
///   sides' finished indexes (`Index`, the hosts' kept while the guests' is built).
///
/// Admitted from the counted inputs when the join starts, and its writes are counted against
/// it while they happen (`resource_limit`, which the bound rules out). File rounding is in
/// the admission (`KD_MAX_FILES`).
fn overlap_allowance(
    candidates: u64,
    host_groups: u64,
    guest_groups: u64,
    sorter: SortConfig,
) -> Option<u64> {
    let held = candidates.checked_mul(Candidate::SIZE as u64)?;
    let regions = held
        .checked_add(host_groups.checked_mul(OVERLAP_BYTES_PER_HOST_GROUP)?)?
        .checked_add(guest_groups.checked_mul(OVERLAP_BYTES_PER_GUEST_GROUP)?)?;
    let sort = |groups: u64| -> Option<u64> {
        let s = spill::<Placed>(groups, sorter)?;
        s.finish_peak.checked_add(s.stream_slack)
    };
    let fallback = held
        .checked_add(
            host_groups
                .checked_add(guest_groups)?
                .checked_mul(SLOT as u64)?,
        )?
        .checked_add(sort(host_groups)?.max(sort(guest_groups)?))?
        .checked_add(Index::bytes_for(host_groups)?)?
        .checked_add(Index::bytes_for(guest_groups)?)?;
    Some(regions.max(fallback))
}

/// A region of the kd join: an axis-aligned box whose faces never cut extend without bound,
/// so the split planes partition every point (a guest lies in exactly one child).
#[derive(Clone, Copy, Debug)]
struct Region {
    lo: [f64; 3],
    hi: [f64; 3],
    cut_lo: [bool; 3],
    cut_hi: [bool; 3],
    /// The largest tolerances of the region's hosts and guests: only its guests pair with its
    /// hosts, so its padding (and the scale it stops splitting at) follows its own geometry.
    host_tol: f32,
    guest_tol: f32,
}

impl Region {
    fn mid(&self, axis: usize) -> f64 {
        (self.lo[axis] + self.hi[axis]) / 2.
    }
    fn half(&self, axis: usize, right: bool) -> Self {
        let mut r = *self;
        if right {
            r.lo[axis] = self.mid(axis);
            r.cut_lo[axis] = true;
        } else {
            r.hi[axis] = self.mid(axis);
            r.cut_hi[axis] = true;
        }
        r
    }
    /// Whether segment `ab` meets the region grown by `pad` (slab clipping, closed bounds).
    fn meets(&self, a: [f64; 3], b: [f64; 3], pad: f64) -> bool {
        let (mut t0, mut t1) = (0f64, 1f64);
        for i in 0..3 {
            let lo = if self.cut_lo[i] {
                self.lo[i] - pad
            } else {
                f64::NEG_INFINITY
            };
            let hi = if self.cut_hi[i] {
                self.hi[i] + pad
            } else {
                f64::INFINITY
            };
            let d = b[i] - a[i];
            if d == 0. {
                if a[i] < lo || a[i] > hi {
                    return false;
                }
                continue;
            }
            let (u, v) = ((lo - a[i]) / d, (hi - a[i]) / d);
            t0 = t0.max(u.min(v));
            t1 = t1.min(u.max(v));
            if t0 > t1 {
                return false;
            }
        }
        true
    }
}

/// Hosts or guests of one kd region: the join's input candidates (the root, read as groups)
/// or a spill.
enum Source<'a, T: Record> {
    Root(&'a Sequence<Candidate>, u64),
    Spill(Sequence<T>),
}

impl<T: Record> Source<'_, T> {
    fn len(&self) -> u64 {
        match self {
            Source::Root(_, groups) => *groups,
            Source::Spill(s) => s.len(),
        }
    }
}

type Stream<'a, T> = Box<dyn Iterator<Item = Result<T, JobError>> + 'a>;

/// Candidates as groups: consecutive candidates of one item and tolerance (the reduce writes
/// each item's open leaves sorted by tolerance), with the extremes of their leaves.
fn groups<'a>(
    seq: &'a Sequence<Candidate>,
) -> Result<impl Iterator<Item = Result<(Candidate, Extremes), JobError>> + 'a, JobError> {
    let mut reader = seq.reader()?.peekable();
    Ok(std::iter::from_fn(move || {
        let first = match reader.next()? {
            Ok(c) => c,
            Err(e) => return Some(Err(e.into())),
        };
        let mut leaves = Extremes::EMPTY;
        leaves.add(first.leaf);
        while let Some(Ok(next)) = reader
            .next_if(|c| matches!(c, Ok(c) if (c.a, c.b, c.tol) == (first.a, first.b, first.tol)))
        {
            leaves.add(next.leaf);
        }
        Some(Ok((first, leaves)))
    }))
}

fn host_stream<'a>(s: &'a Source<'_, KdHost>) -> Result<Stream<'a, KdHost>, JobError> {
    Ok(match s {
        Source::Root(seq, _) => Box::new(groups(seq)?.enumerate().map(|(i, g)| {
            let (c, leaves) = g?;
            Ok(KdHost {
                id: i as u32,
                a: c.a,
                b: c.b,
                tol: c.tol,
                leaves,
                cut: 0,
            })
        })),
        Source::Spill(seq) => Box::new(seq.reader()?.map(|h| Ok(h?))),
    })
}

fn guest_stream<'a>(s: &'a Source<'_, KdGuest>) -> Result<Stream<'a, KdGuest>, JobError> {
    Ok(match s {
        Source::Root(seq, _) => Box::new(groups(seq)?.enumerate().map(|(i, g)| {
            let (c, leaves) = g?;
            Ok(KdGuest {
                id: i as u32,
                p: c.a,
                tol: c.tol,
                leaves,
            })
        })),
        Source::Spill(seq) => Box::new(seq.reader()?.map(|g| Ok(g?))),
    })
}

/// How a host copy meets a split.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    /// Cut: a copy goes to each side.
    Both,
    /// Kept by this region and tested against all of its guests.
    Long,
}

/// A host group held in memory with the extremes of its partners' leaves found so far.
type Held = (KdHost, Extremes);

/// The kd join's state: scratch, the result slots, the live-byte ledger and totals.
struct Kd<'c, 's> {
    scratch: &'s Scratch,
    buffer: usize,
    floor: f64,
    host_slots: SlotFile,
    guest_slots: SlotFile,
    live: u64,
    budget: u64,
    stats: &'c mut IncidenceStats,
    check: &'c (dyn Fn() -> Result<(), JobError> + Sync),
}

impl<'s> Kd<'_, 's> {
    /// The index padding of a host in `region`. The predicate accepts a guest only within
    /// `tau = max(host tol, guest tol) + rounding` of the segment's line, projecting at most
    /// `tau` beyond an endpoint. So the guest lies within `sqrt(2) * tau` of the segment
    /// itself (beyond an endpoint, the along-line and perpendicular offsets are orthogonal
    /// and each at most `tau`), hence within that of some segment point in every coordinate,
    /// and the segment meets any region holding the guest grown by it. Only the region's
    /// guests can pair with its hosts (a child's guests are a subset of its parent's), so
    /// `tau` is bounded by the region's largest guest tolerance and the rounding floor of the
    /// largest coordinate; the factor `1 + 2^-20` absorbs the f64 rounding of the clipping.
    fn pad(&self, tol: u32, region: &Region) -> f64 {
        std::f64::consts::SQRT_2
            * (f64::from(f32::from_bits(tol).max(region.guest_tol)) + self.floor)
            * (1. + 1. / f64::from(1u32 << 20))
    }

    fn side(&self, region: &Region, axis: usize, h: &KdHost) -> Side {
        let pad = self.pad(h.tol, region);
        let (a, b) = (point(h.a), point(h.b));
        let span = (a[axis] - b[axis]).abs() + 2. * pad;
        let extent = region.hi[axis] - region.lo[axis];
        match (
            region.half(axis, false).meets(a, b, pad),
            region.half(axis, true).meets(a, b, pad),
        ) {
            (true, false) => Side::Left,
            (false, true) => Side::Right,
            // A short host crosses at most one split per axis: having crossed one, its
            // padded span reaches every later plane of that axis only at half the region.
            (true, true)
                if 2. * span < extent
                    && h.cut & (1 << axis) == 0
                    && u32::from(h.cut).count_ones() < MAX_CUTS =>
            {
                Side::Both
            }
            _ => Side::Long,
        }
    }

    fn account(&mut self, bytes: u64) -> Result<(), JobError> {
        self.live += bytes;
        if self.live > self.budget {
            return Err(JobError::new(
                JobErrorKind::ResourceLimit,
                format!(
                    "the overlap join's live scratch passed its declared {} bytes",
                    self.budget
                ),
            ));
        }
        Ok(())
    }

    fn push<T: Record>(&mut self, w: &mut SequenceWriter<T>, r: &T) -> Result<(), JobError> {
        self.account(T::SIZE as u64)?;
        w.push(r)?;
        Ok(())
    }

    fn writer<T: Record>(&self, stem: &str) -> Result<SequenceWriter<T>, JobError> {
        Ok(SequenceWriter::<T>::new(self.scratch, stem, self.buffer)?)
    }

    fn release<T: Record>(&mut self, s: Source<'_, T>) {
        if let Source::Spill(seq) = s {
            self.live -= seq.len() * T::SIZE as u64;
        }
    }

    /// Merge `found` into half `half` of group `id`'s slot (0 exact, 1 provisional).
    fn merge_slot(
        slots: &mut SlotFile,
        id: u32,
        found: &Extremes,
        half: usize,
    ) -> Result<(), JobError> {
        if found.is_empty() {
            return Ok(());
        }
        let mut bytes = [0u8; SLOT];
        slots.read(u64::from(id), &mut bytes)?;
        let part = &mut bytes[16 * half..16 * half + 16];
        let mut all = slot_extremes(part);
        all.merge(found);
        all.put(part);
        slots.write(u64::from(id), &bytes)?;
        Ok(())
    }

    fn flush(&mut self, held: &[Held]) -> Result<(), JobError> {
        for (h, found) in held {
            Self::merge_slot(&mut self.host_slots, h.id, found, 0)?;
        }
        Ok(())
    }

    /// Whether testing `hosts` long (or terminal) hosts exactly against `guests` guests
    /// passes the region's budget of `BUDGET_TESTS_PER_RECORD` per record of the region.
    fn over_budget(hosts: u64, guests: u64, region_hosts: u64) -> bool {
        hosts.saturating_mul(guests)
            > BUDGET_TESTS_PER_RECORD.saturating_mul(region_hosts.saturating_add(guests))
    }

    /// The superset match of an over-budget region: every host of `hosts` with every guest
    /// of `guests` (each records the other side's leaves as provisional; a member drops its
    /// own leaf later), so no accepted pair is missed. The groups are kept (`Fallback`) for
    /// exact resolution where a tile needs it. `guests` are streamed through `each`.
    fn superset<'g>(
        &mut self,
        hosts: Stream<'_, KdHost>,
        guests: Stream<'g, KdGuest>,
        mut each: impl FnMut(&mut Self, KdGuest) -> Result<(), JobError>,
        held_ids: &mut Vec<u32>,
    ) -> Result<(), JobError> {
        self.stats.fallback_regions += 1;
        let mut host_leaves = Extremes::EMPTY;
        let mut ids =
            SequenceWriter::<HostId>::new(self.scratch, "hlod-kd-fallback-ids", self.buffer)?;
        for h in hosts {
            let h = h?;
            host_leaves.merge(&h.leaves);
            if held_ids.len() < SCAN_HOSTS {
                held_ids.push(h.id);
            } else {
                self.account(HostId::SIZE as u64)?;
                ids.push(&HostId(h.id))?;
            }
        }
        let mut guest_leaves = Extremes::EMPTY;
        for (k, g) in guests.enumerate() {
            if k.is_multiple_of(1 << 12) {
                (self.check)()?;
            }
            let g = g?;
            guest_leaves.merge(&g.leaves);
            Self::merge_slot(&mut self.guest_slots, g.id, &host_leaves, 1)?;
            each(self, g)?;
        }
        for &id in held_ids.iter() {
            Self::merge_slot(&mut self.host_slots, id, &guest_leaves, 1)?;
        }
        let ids = ids.finish()?;
        for id in ids.reader()? {
            Self::merge_slot(&mut self.host_slots, id?.0, &guest_leaves, 1)?;
        }
        self.live -= ids.len() * HostId::SIZE as u64;
        held_ids.clear();
        Ok(())
    }

    /// Test guest group `g` against host group `h` with the exact predicate; on a match, each
    /// side records the other's leaves (a member later excludes its own).
    fn test(&mut self, (host, found): &mut Held, g: &KdGuest, guest_found: &mut Extremes) {
        if !host.leaves.differ(&g.leaves) {
            return;
        }
        self.stats.tests += 1;
        let tol = f64::from(f32::from_bits(host.tol).max(f32::from_bits(g.tol)))
            + rounding(&[host.a, host.b, g.p]);
        if !on_segment(host.a, host.b, g.p, tol) {
            return;
        }
        self.stats.overlaps += 1;
        found.merge(&g.leaves);
        guest_found.merge(&host.leaves);
    }

    /// One guest against `held`, its finds recorded in its slot.
    fn guest(&mut self, held: &mut [Held], g: &KdGuest) -> Result<(), JobError> {
        let mut found = Extremes::EMPTY;
        for h in held.iter_mut() {
            self.test(h, g, &mut found);
        }
        Self::merge_slot(&mut self.guest_slots, g.id, &found, 0)
    }

    /// Test every guest of `guests` exactly against `hosts`, holding `SCAN_HOSTS` at a time:
    /// every chunk but the last in one pass over the guests; the last chunk is returned held,
    /// for the caller's final guest pass.
    fn scan(
        &mut self,
        hosts: Stream<'_, KdHost>,
        guests: &Source<'_, KdGuest>,
    ) -> Result<Vec<Held>, JobError> {
        let mut chunk: Vec<Held> = Vec::with_capacity(SCAN_HOSTS);
        for h in hosts {
            if chunk.len() == SCAN_HOSTS {
                self.stats.scan_passes += 1;
                for (k, g) in guest_stream(guests)?.enumerate() {
                    if k.is_multiple_of(1 << 12) {
                        (self.check)()?;
                    }
                    self.guest(&mut chunk, &g?)?;
                }
                self.flush(&chunk)?;
                chunk.clear();
            }
            chunk.push((h?, Extremes::EMPTY));
        }
        Ok(chunk)
    }

    /// Join one region (see `kd_join`).
    fn region(
        &mut self,
        region: Region,
        hosts: Source<'_, KdHost>,
        guests: Source<'_, KdGuest>,
        depth: u32,
    ) -> Result<(), JobError> {
        (self.check)()?;
        self.stats.kd_regions += 1;
        if guests.len() == 0 || hosts.len() == 0 {
            self.release(hosts);
            self.release(guests);
            return Ok(());
        }
        let extent: [f64; 3] = std::array::from_fn(|i| region.hi[i] - region.lo[i]);
        let pad_max = self.pad(region.host_tol.to_bits(), &region);
        let eligible: Vec<usize> = (0..3).filter(|&i| extent[i] > 8. * pad_max).collect();
        let terminal = depth >= MAX_DEPTH || eligible.is_empty();
        if terminal && Self::over_budget(hosts.len(), guests.len(), hosts.len()) {
            self.stats.terminal_scans += 1;
            let mut held = Vec::new();
            self.superset(
                host_stream(&hosts)?,
                guest_stream(&guests)?,
                |_, _| Ok(()),
                &mut held,
            )?;
            self.release(hosts);
            self.release(guests);
            return Ok(());
        }
        if hosts.len() <= EXACT_HOSTS as u64 || terminal {
            // A leaf (few hosts), or a region no split can shorten: test exactly.
            if terminal {
                self.stats.terminal_scans += 1;
            } else {
                self.stats.exact_leaves += 1;
            }
            let mut last = self.scan(host_stream(&hosts)?, &guests)?;
            self.release(hosts);
            for (k, g) in guest_stream(&guests)?.enumerate() {
                if k.is_multiple_of(1 << 12) {
                    (self.check)()?;
                }
                self.guest(&mut last, &g?)?;
            }
            self.flush(&last)?;
            self.release(guests);
            return Ok(());
        }
        // The axis keeping the most hosts whole: fewest cut or long copies.
        let mut cost = [0u64; 3];
        for h in host_stream(&hosts)? {
            let h = h?;
            for &axis in &eligible {
                if matches!(self.side(&region, axis, &h), Side::Both | Side::Long) {
                    cost[axis] += 1;
                }
            }
        }
        let axis = *eligible
            .iter()
            .min_by(|&&x, &&y| cost[x].cmp(&cost[y]).then(extent[y].total_cmp(&extent[x])))
            .unwrap();
        self.stats.kd_splits += 1;
        let (mut left_hosts, mut right_hosts) = (
            self.writer::<KdHost>("hlod-kd-hosts")?,
            self.writer::<KdHost>("hlod-kd-hosts")?,
        );
        let mut long: Vec<Held> = Vec::new();
        let mut long_spill: Option<SequenceWriter<KdHost>> = None;
        let (mut left, mut right) = (region.half(axis, false), region.half(axis, true));
        (
            left.host_tol,
            right.host_tol,
            left.guest_tol,
            right.guest_tol,
        ) = (0., 0., 0., 0.);
        for h in host_stream(&hosts)? {
            let h = h?;
            let tol = f32::from_bits(h.tol);
            match self.side(&region, axis, &h) {
                Side::Left => {
                    left.host_tol = left.host_tol.max(tol);
                    self.push(&mut left_hosts, &h)?;
                }
                Side::Right => {
                    right.host_tol = right.host_tol.max(tol);
                    self.push(&mut right_hosts, &h)?;
                }
                Side::Both => {
                    let h = KdHost {
                        cut: h.cut | (1 << axis),
                        ..h
                    };
                    left.host_tol = left.host_tol.max(tol);
                    right.host_tol = right.host_tol.max(tol);
                    self.push(&mut left_hosts, &h)?;
                    self.push(&mut right_hosts, &h)?;
                }
                Side::Long => {
                    if let Some(spill) = long_spill.as_mut() {
                        self.account(KdHost::SIZE as u64)?;
                        spill.push(&h)?;
                    } else if long.len() < SCAN_HOSTS {
                        long.push((h, Extremes::EMPTY));
                    } else {
                        let mut spill = self.writer::<KdHost>("hlod-kd-long")?;
                        for (held, _) in long.drain(..) {
                            self.push(&mut spill, &held)?;
                        }
                        self.push(&mut spill, &h)?;
                        long_spill = Some(spill);
                    }
                }
            }
        }
        let region_hosts = hosts.len();
        self.release(hosts);
        let long_count = long.len() as u64 + long_spill.as_ref().map_or(0, |w| w.len());
        let mid = region.mid(axis);
        let (mut left_guests, mut right_guests) = (
            self.writer::<KdGuest>("hlod-kd-guests")?,
            self.writer::<KdGuest>("hlod-kd-guests")?,
        );
        let mut route = |kd: &mut Self, g: KdGuest| -> Result<(), JobError> {
            let tol = f32::from_bits(g.tol);
            if point(g.p)[axis] < mid {
                left.guest_tol = left.guest_tol.max(tol);
                kd.push(&mut left_guests, &g)
            } else {
                right.guest_tol = right.guest_tol.max(tol);
                kd.push(&mut right_guests, &g)
            }
        };
        if long_count > 0 && Self::over_budget(long_count, guests.len(), region_hosts) {
            // Over budget: the long hosts by a superset, the guests on to their sides.
            let spill = match long_spill {
                Some(spill) => Some(Source::Spill(spill.finish()?)),
                None => None,
            };
            let held: Box<dyn Iterator<Item = Result<KdHost, JobError>>> =
                Box::new(long.iter().map(|(h, _)| Ok(*h)));
            let stream: Stream<'_, KdHost> = match &spill {
                Some(spill) => Box::new(held.chain(host_stream(spill)?)),
                None => held,
            };
            let mut ids = Vec::new();
            self.superset(stream, guest_stream(&guests)?, &mut route, &mut ids)?;
            if let Some(spill) = spill {
                self.release(spill);
            }
            self.release(guests);
            let (left_hosts, right_hosts) = (left_hosts.finish()?, right_hosts.finish()?);
            let (left_guests, right_guests) = (left_guests.finish()?, right_guests.finish()?);
            self.region(
                left,
                Source::Spill(left_hosts),
                Source::Spill(left_guests),
                depth + 1,
            )?;
            return self.region(
                right,
                Source::Spill(right_hosts),
                Source::Spill(right_guests),
                depth + 1,
            );
        }
        // More long hosts than one chunk: all but the last chunk in passes over the guests.
        let mut long = match long_spill {
            Some(spill) => {
                self.stats.long_spills += 1;
                let spill = Source::Spill(spill.finish()?);
                let last = self.scan(host_stream(&spill)?, &guests)?;
                self.release(spill);
                last
            }
            None => long,
        };
        // The guests: against the (last chunk of) long hosts, then to their side of the plane.
        for (k, g) in guest_stream(&guests)?.enumerate() {
            if k.is_multiple_of(1 << 12) {
                (self.check)()?;
            }
            let g = g?;
            if !long.is_empty() {
                self.guest(&mut long, &g)?;
            }
            route(self, g)?;
        }
        self.flush(&long)?;
        self.release(guests);
        let (left_hosts, right_hosts) = (left_hosts.finish()?, right_hosts.finish()?);
        let (left_guests, right_guests) = (left_guests.finish()?, right_guests.finish()?);
        // The right child waits, closed, while the left is joined.
        self.region(
            left,
            Source::Spill(left_hosts),
            Source::Spill(left_guests),
            depth + 1,
        )?;
        self.region(
            right,
            Source::Spill(right_hosts),
            Source::Spill(right_guests),
            depth + 1,
        )
    }
}

/// The overlap join of open points (guests) against open segments of other leaves (hosts):
/// an out-of-core kd split of both sets, without a sort, exact within its budget and a
/// superset (provisional, kept for exact resolution) past it.
///
/// - Groups: candidates of one item and tolerance (consecutive, as the reduce writes them)
///   are one group carrying the extremes of its leaves, so coincident geometry from many
///   leaves (collapsed sub-lattice detail) is tested once; a member's result is the extremes
///   of its group's partners' leaves, its own leaf excluded, which is exact.
/// - Index: a host copy goes to every child its padded segment meets (`Kd::pad` proves the
///   padding covers the predicate's whole acceptance region), a guest to the one child
///   holding it, so every accepted pair meets in some region.
/// - Copies: a short host is cut at most once per axis and at most `MAX_CUTS` times; a host
///   long against its region (or past its cuts) stays there and is tested against all of the
///   region's guests. A host has at most `2^MAX_CUTS` copies, a guest one.
/// - Work: a region of at most `EXACT_HOSTS` hosts is a leaf (at most that many tests per
///   guest). A split tests each guest against the region's long hosts; a region no split can
///   shorten (every axis within eight paddings, or the depth guard) tests all of its hosts.
///   Both are exact, in chunks of `SCAN_HOSTS` held hosts, one guest pass per chunk. Splits
///   halve one axis longer than eight paddings, so a path has at most `3 log2(extent / 8
///   padding)` splits (under 57 with the rounding floor), each moving every record O(1)
///   times: O((hosts + guests) * depth), plus the long and terminal tests. Those are the
///   pairs of distinct geometry no axis split separates (hosts crossing the region's best
///   axis at half its extent, or within eight paddings of each other): exact point-on-segment
///   incidence is at least as hard as Hopcroft's problem, so no exact method bounds them by
///   O(n log n) in general; the split axis is chosen to keep them fewest, and a terminal
///   region of quantized geometry holds a bounded number of distinct lattice points. Nothing
///   is aggregated, so no lock is ever false.
/// - Memory: at most `EXACT_HOSTS + SCAN_HOSTS` held hosts and `KD_STREAMS` buffers.
/// - Output: one row per matched (leaf, item), with the exact union of its partners' leaf
///   intervals.
/// - Scratch: live bytes are counted against `overlap_allowance` (proof there), admitted
///   with the rows into `stats.scratch.join`.
#[allow(clippy::too_many_arguments)]
fn kd_join<'s>(
    scratch: &'s Scratch,
    sorter: SortConfig,
    hosts: &Sequence<Candidate>,
    guests: &Sequence<Candidate>,
    rows: &mut ExternalSorter<'s, InterfaceRow>,
    stats: &mut IncidenceStats,
    admit: &dyn Fn(u64) -> Result<(), JobError>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Fallback, JobError> {
    if hosts.is_empty() || guests.is_empty() {
        return Ok(Fallback::default());
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let (mut magnitude, mut guest_tol, mut host_tol) = (0f64, 0f32, 0f32);
    let mut extend = |k: Key| {
        let p = point(k);
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
            magnitude = magnitude.max(p[i].abs());
        }
    };
    let mut host_groups = 0u64;
    for g in groups(hosts)? {
        let (h, _) = g?;
        extend(h.a);
        extend(h.b);
        host_tol = host_tol.max(f32::from_bits(h.tol));
        host_groups += 1;
    }
    let mut guest_groups = 0u64;
    for g in groups(guests)? {
        let (g, _) = g?;
        extend(g.a);
        guest_tol = guest_tol.max(f32::from_bits(g.tol));
        guest_groups += 1;
    }
    stats.host_groups = host_groups;
    stats.guest_groups = guest_groups;
    let candidates = hosts.len() + guests.len();
    let overflow = || state("overlap join allowance overflows");
    let budget =
        overlap_allowance(candidates, host_groups, guest_groups, sorter).ok_or_else(overflow)?;
    // The rows so far, at most one more per candidate, and the allowance.
    stats.scratch.join = join_phase(stats.rows, candidates, host_groups, guest_groups, sorter)
        .and_then(|b| b.checked_add(stats.scratch.retained))
        .ok_or_else(overflow)?;
    admit(stats.scratch.join)?;
    let bounds = (lo, hi);
    let mut host_slots = SlotFile::new(scratch, "hlod-kd-host-slots", SLOT)?;
    host_slots.write(host_groups - 1, &[0; SLOT])?;
    let mut guest_slots = SlotFile::new(scratch, "hlod-kd-guest-slots", SLOT)?;
    guest_slots.write(guest_groups - 1, &[0; SLOT])?;
    // `rounding` of any three of these points is at most this floor.
    let floor = magnitude * f64::from(f32::EPSILON) * 4.;
    let mut kd = Kd {
        scratch,
        buffer: sorter.buffer_bytes,
        floor,
        host_slots,
        guest_slots,
        live: (hosts.len() + guests.len()) * Candidate::SIZE as u64
            + (host_groups + guest_groups) * SLOT as u64,
        budget,
        stats,
        check,
    };
    kd.account(0)?;
    let root = Region {
        lo,
        hi,
        cut_lo: [false; 3],
        cut_hi: [false; 3],
        host_tol,
        guest_tol,
    };
    kd.region(
        root,
        Source::Root(hosts, host_groups),
        Source::Root(guests, guest_groups),
        0,
    )?;
    // Each member of a matched group: its partners' leaves other than its own, certain and
    // locked (certain or provisional). A group matched provisionally is kept once, with its
    // members' leaves, in its side's index for exact resolution; its members' rows carry its
    // tolerance.
    let mut fallback = Fallback::default();
    for (side, seq) in [(0, hosts), (1, guests)] {
        let mut placed = ExternalSorter::<Placed>::new(scratch, sorter)?;
        let mut group = u64::MAX;
        let (mut exact, mut provisional) = (Extremes::EMPTY, Extremes::EMPTY);
        let mut kept: Option<Kept> = None;
        let mut last: Option<(Key, Key, u32)> = None;
        for (n, c) in seq.reader()?.enumerate() {
            if n.is_multiple_of(1 << 16) {
                (kd.check)()?;
            }
            let c = c?;
            if last != Some((c.a, c.b, c.tol)) {
                if let Some(k) = kept.take() {
                    placed.push(Placed::new(k, &bounds))?;
                    kd.stats.fallback_groups += 1;
                }
                last = Some((c.a, c.b, c.tol));
                group = group.wrapping_add(1);
                let mut bytes = [0u8; SLOT];
                let slots = if side == 0 {
                    &kd.host_slots
                } else {
                    &kd.guest_slots
                };
                slots.read(group, &mut bytes)?;
                exact = slot_extremes(&bytes[..16]);
                provisional = slot_extremes(&bytes[16..]);
                if !provisional.is_empty() {
                    kept = Some(Kept {
                        a: c.a,
                        b: c.b,
                        tol: c.tol,
                        leaves: Extremes::EMPTY,
                    });
                }
            }
            if let Some(k) = kept.as_mut() {
                k.leaves.add(c.leaf);
                kd.stats.fallback_candidates += 1;
            }
            let mut lock = exact;
            lock.merge(&provisional);
            if let Some((lo, hi)) = lock.others(c.leaf) {
                if side == 0 {
                    kd.stats.matched_hosts += 1;
                } else {
                    kd.stats.matched_guests += 1;
                }
                let (clo, chi) = exact
                    .others(c.leaf)
                    .map_or(NO_LEAF, |(lo, hi)| (lo.min(c.leaf), hi.max(c.leaf)));
                let row = InterfaceRow {
                    leaf: c.leaf,
                    a: c.a,
                    b: c.b,
                    lo: lo.min(c.leaf),
                    hi: hi.max(c.leaf),
                    clo,
                    chi,
                    tol: if provisional.is_empty() {
                        NO_TOL
                    } else {
                        c.tol
                    },
                };
                kd.stats.provisional_rows += u64::from(row.provisional());
                rows.push(row)?;
                kd.stats.rows += 1;
            }
        }
        if let Some(k) = kept.take() {
            placed.push(Placed::new(k, &bounds))?;
            kd.stats.fallback_groups += 1;
        }
        let index = Index::build(&mut kd, placed.finish()?)?;
        if side == 0 {
            fallback.hosts = index;
        } else {
            fallback.guests = index;
        }
    }
    Ok(fallback)
}

/// A kept group: a distinct open point (`a == b`) or segment at tolerance `tol` (f32 bits)
/// that an over-budget region matched by a superset, with the extremes of the leaves holding
/// it open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Kept {
    a: Key,
    b: Key,
    tol: u32,
    leaves: Extremes,
}

impl Record for Kept {
    const SIZE: usize = 44;
    fn encode(&self, out: &mut [u8]) {
        put(out, 0, self.a);
        put(out, 12, self.b);
        out[24..28].copy_from_slice(&self.tol.to_le_bytes());
        self.leaves.put(&mut out[28..44]);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            a: get(bytes, 0),
            b: get(bytes, 12),
            tol: u32_at(bytes, 24),
            leaves: Extremes::get(&bytes[28..44]),
        }
    }
}

/// A kept group with its index order: the Morton code of its box centre in the join's bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Placed {
    code: u64,
    kept: Kept,
}

impl Placed {
    fn new(kept: Kept, (lo, hi): &([f64; 3], [f64; 3])) -> Self {
        let (a, b) = (point(kept.a), point(kept.b));
        let mut code = 0u64;
        for axis in 0..3 {
            let extent = hi[axis] - lo[axis];
            let centre = (a[axis] + b[axis]) / 2.;
            let q = if extent > 0. {
                (((centre - lo[axis]) / extent) * f64::from((1u32 << 21) - 1))
                    .clamp(0., f64::from((1u32 << 21) - 1)) as u64
            } else {
                0
            };
            for bit in 0..21 {
                code |= ((q >> bit) & 1) << (3 * bit + axis);
            }
        }
        Self { code, kept }
    }
}

impl Record for Placed {
    const SIZE: usize = 8 + Kept::SIZE;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.code.to_le_bytes());
        self.kept.encode(&mut out[8..]);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            code: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            kept: Kept::decode(&bytes[8..]),
        }
    }
}

/// One node of an `Index`: the box (f32 bits per coordinate), largest tolerance and least
/// and greatest leaf of the groups below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Node {
    lo: Key,
    hi: Key,
    tol: u32,
    least: u32,
    greatest: u32,
}

impl Node {
    fn of(k: &Kept) -> Self {
        let (a, b) = (k.a.map(f32::from_bits), k.b.map(f32::from_bits));
        Self {
            lo: std::array::from_fn(|i| a[i].min(b[i]).to_bits()),
            hi: std::array::from_fn(|i| a[i].max(b[i]).to_bits()),
            tol: k.tol,
            least: k.leaves.min[0],
            greatest: k.leaves.max[0],
        }
    }
    fn union(&self, other: &Self) -> Self {
        let pick = |x: u32, y: u32, low: bool| {
            let (x, y) = (f32::from_bits(x), f32::from_bits(y));
            if low { x.min(y) } else { x.max(y) }.to_bits()
        };
        Self {
            lo: std::array::from_fn(|i| pick(self.lo[i], other.lo[i], true)),
            hi: std::array::from_fn(|i| pick(self.hi[i], other.hi[i], false)),
            tol: pick(self.tol, other.tol, false),
            least: self.least.min(other.least),
            greatest: self.greatest.max(other.greatest),
        }
    }
    /// Whether some point of the box lies within `pad(tol)` of the query box in every
    /// coordinate.
    fn near(&self, lo: &[f64; 3], hi: &[f64; 3], pad: f64) -> bool {
        (0..3).all(|i| {
            f64::from(f32::from_bits(self.lo[i])) <= hi[i] + pad
                && f64::from(f32::from_bits(self.hi[i])) >= lo[i] - pad
        })
    }
}

impl Record for Node {
    const SIZE: usize = 36;
    fn encode(&self, out: &mut [u8]) {
        put(out, 0, self.lo);
        put(out, 12, self.hi);
        out[24..28].copy_from_slice(&self.tol.to_le_bytes());
        out[28..32].copy_from_slice(&self.least.to_le_bytes());
        out[32..36].copy_from_slice(&self.greatest.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            lo: get(bytes, 0),
            hi: get(bytes, 12),
            tol: u32_at(bytes, 24),
            least: u32_at(bytes, 28),
            greatest: u32_at(bytes, 32),
        }
    }
}

/// Children per index node.
const FANOUT: usize = 16;
/// Node levels of an index of at most `u32::MAX` groups.
const INDEX_MAX_LEVELS: usize = 8;

/// A packed spatial index of one side's kept groups, built once after the join: the groups
/// in Morton order of their box centres, `FANOUT` to a node, nodes `FANOUT` to a parent, up
/// to one root. A query descends only into nodes whose box (grown by the predicate's padding
/// at the node's largest tolerance) meets the query box.
pub(crate) struct Index {
    entries: Sequence<Kept>,
    /// Node levels from the entries' parents up to the root.
    levels: Vec<Sequence<Node>>,
    /// The largest coordinate magnitude of the groups (their rounding floor).
    magnitude: f64,
}

/// Work of one resolution: index records read and exact predicate tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// Provisional rows re-tested, and rows whose lock was finalized (all of the tile's).
    pub tested: u64,
    pub rows: u64,
    pub visits: u64,
    pub tests: u64,
}

impl Index {
    /// Scratch bytes of an index of `groups` groups.
    fn bytes_for(groups: u64) -> Option<u64> {
        let mut bytes = groups.checked_mul(Kept::SIZE as u64)?;
        let mut level = groups;
        while level > 1 {
            level = level.div_ceil(FANOUT as u64);
            bytes = bytes.checked_add(level.checked_mul(Node::SIZE as u64)?)?;
        }
        Some(bytes)
    }

    fn bytes(&self) -> u64 {
        self.entries.len() * Kept::SIZE as u64
            + self
                .levels
                .iter()
                .map(|l| l.len() * Node::SIZE as u64)
                .sum::<u64>()
    }

    /// Pack `sorted` groups, counting every byte written against the join's allowance.
    fn build(
        kd: &mut Kd<'_, '_>,
        sorted: SortedStream<'_, Placed>,
    ) -> Result<Option<Self>, JobError> {
        let mut entries = kd.writer::<Kept>("hlod-fallback-entries")?;
        let mut level = kd.writer::<Node>("hlod-fallback-nodes")?;
        let mut node: Option<Node> = None;
        let mut count = 0;
        let mut magnitude = 0f64;
        for p in sorted {
            let k = p?.kept;
            kd.push(&mut entries, &k)?;
            for v in point(k.a).into_iter().chain(point(k.b)) {
                magnitude = magnitude.max(v.abs());
            }
            let n = Node::of(&k);
            node = Some(node.map_or(n, |m| m.union(&n)));
            count += 1;
            if count == FANOUT {
                kd.push(&mut level, &node.take().unwrap())?;
                count = 0;
            }
        }
        if let Some(n) = node.take() {
            kd.push(&mut level, &n)?;
        }
        let entries = entries.finish()?;
        if entries.is_empty() {
            return Ok(None);
        }
        let mut levels = vec![level.finish()?];
        while levels[levels.len() - 1].len() > 1 {
            if levels.len() == INDEX_MAX_LEVELS {
                return Err(state("fallback index deeper than its level bound"));
            }
            let mut up = kd.writer::<Node>("hlod-fallback-nodes")?;
            let mut node: Option<Node> = None;
            let mut count = 0;
            for n in levels[levels.len() - 1].reader()? {
                let n = n?;
                node = Some(node.map_or(n, |m| m.union(&n)));
                count += 1;
                if count == FANOUT {
                    kd.push(&mut up, &node.take().unwrap())?;
                    count = 0;
                }
            }
            if let Some(n) = node.take() {
                kd.push(&mut up, &n)?;
            }
            levels.push(up.finish()?);
        }
        Ok(Some(Self {
            entries,
            levels,
            magnitude,
        }))
    }

    /// Every group within the predicate's reach of the box `lo..=hi` of an item at
    /// tolerance `tol`: a group can pair with the item only if its box lies within
    /// `sqrt(2) * tau` of the item's in every coordinate (`tau` as in `Kd::pad`, at the
    /// larger of the two tolerances), so a node is entered only if its box, grown by that at
    /// its largest tolerance, meets the item's, and only if `wanted` its least and greatest
    /// leaf (a node or group that cannot change the caller's answer is skipped). Each record
    /// read counts a visit.
    fn query(
        &self,
        lo: &[f64; 3],
        hi: &[f64; 3],
        tol: f32,
        cost: &mut Resolved,
        wanted: impl Fn(u32, u32) -> bool,
        mut each: impl FnMut(&Kept),
    ) -> Result<(), JobError> {
        let magnitude = lo
            .iter()
            .chain(hi)
            .fold(self.magnitude, |m, v| m.max(v.abs()));
        let floor = magnitude * f64::from(f32::EPSILON) * 4.;
        let pad = |t: u32| {
            std::f64::consts::SQRT_2
                * (f64::from(tol.max(f32::from_bits(t))) + floor)
                * (1. + 1. / f64::from(1u32 << 20))
        };
        let entries = self.entries.random()?;
        let levels = self
            .levels
            .iter()
            .map(Sequence::random)
            .collect::<Result<Vec<_>, _>>()?;
        let (mut encoded, mut nodes, mut kept) = (Vec::new(), Vec::new(), Vec::new());
        // (level, first, count, least, greatest): a node's children (level 0 is the entries,
        // level `k` is `levels[k - 1]`) with the node's leaves, checked again when popped
        // (an answer found meanwhile may make them unwanted).
        let mut stack = vec![(self.levels.len(), 0u64, 1usize, 0u32, NONE - 1)];
        while let Some((level, first, count, least, greatest)) = stack.pop() {
            if level < self.levels.len() && !wanted(least, greatest) {
                continue;
            }
            cost.visits += count as u64;
            if level == 0 {
                entries.read(first, count, &mut encoded, &mut kept)?;
                for k in &kept {
                    let n = Node::of(k);
                    if wanted(n.least, n.greatest) && n.near(lo, hi, pad(k.tol)) {
                        each(k);
                    }
                }
                continue;
            }
            levels[level - 1].read(first, count, &mut encoded, &mut nodes)?;
            let below = if level == 1 {
                self.entries.len()
            } else {
                self.levels[level - 2].len()
            };
            for (i, n) in nodes.iter().enumerate() {
                if wanted(n.least, n.greatest) && n.near(lo, hi, pad(n.tol)) {
                    let child = (first + i as u64) * FANOUT as u64;
                    let count = (below - child).min(FANOUT as u64) as usize;
                    stack.push((level - 1, child, count, n.least, n.greatest));
                }
            }
        }
        Ok(())
    }
}

/// The groups an over-budget region matched by a superset (provisional rows), indexed per
/// side and kept through the walk so a tile those rows keep from its budget can resolve them
/// exactly. Each group once.
#[derive(Default)]
pub(crate) struct Fallback {
    hosts: Option<Index>,
    guests: Option<Index>,
}

impl Fallback {
    /// Whether nothing was matched by a superset.
    pub(crate) fn is_empty(&self) -> bool {
        self.hosts.is_none() && self.guests.is_none()
    }

    /// Scratch bytes it holds.
    pub(crate) fn bytes(&self) -> u64 {
        [&self.hosts, &self.guests]
            .iter()
            .map(|s| s.as_ref().map_or(0, Index::bytes))
            .sum()
    }

    /// Resolve the provisional rows of the tile over leaves `[begin, end)` exactly, then lock
    /// every row only by its certain interval (a row left with none is dropped). Rows are
    /// first merged per item, as a parent holds them.
    ///
    /// - Re-test: a provisional row with a provisional owner (`tol`) is tested against the
    ///   other side's kept groups the index finds near it, at its tolerance (the union of its
    ///   owners' partners: `InterfaceRow`). Only partners outside the tile matter to it and
    ///   to every ancestor (whose leaves contain the tile's), so a group counts by its least
    ///   leaf below `begin` and its greatest at or above `end`; its own owners lie inside.
    ///   Any group meeting the predicate there is a true partner, and every true partner of a
    ///   provisional owner not already certain shared a superset region with it, so was kept.
    /// - Derive: rows implied by others (`read_leaf`) are recomputed from the resolved
    ///   certain intervals: an endpoint gains its segments' (each owner of a segment holds
    ///   its endpoints, so the merged endpoint row includes theirs: exact), then a segment
    ///   gains the narrower of its endpoints' within its previous lock (an open edge between
    ///   kept points: exact for one owner, and never past the superset for several).
    ///
    /// Work: one index query per re-tested row, `O(log C)` node levels plus the groups near
    /// it, each tested once; a row is resolved at most once on its path to the root (its
    /// lock is final afterwards). Counted in the result.
    pub(crate) fn resolve(
        &self,
        rows: &mut Vec<InterfaceRow>,
        begin: u32,
        end: u32,
        check: &(dyn Fn() -> Result<(), JobError> + Sync),
    ) -> Result<Resolved, JobError> {
        self.resolve_with(rows, begin, end, true, check)
    }

    fn resolve_with(
        &self,
        rows: &mut Vec<InterfaceRow>,
        begin: u32,
        end: u32,
        derive: bool,
        check: &(dyn Fn() -> Result<(), JobError> + Sync),
    ) -> Result<Resolved, JobError> {
        let mut cost = Resolved::default();
        if !rows.iter().any(InterfaceRow::provisional) {
            return Ok(cost);
        }
        merge_rows(rows);
        let lock: Vec<(u32, u32)> = rows.iter().map(|r| (r.lo, r.hi)).collect();
        for r in rows.iter_mut() {
            if !r.provisional() || r.tol == NO_TOL {
                continue;
            }
            let index = if r.is_vertex() {
                &self.hosts
            } else {
                &self.guests
            };
            let Some(index) = index else { continue };
            if cost.tested.is_multiple_of(256) {
                check()?;
            }
            cost.tested += 1;
            let (a, b) = (point(r.a), point(r.b));
            let lo: [f64; 3] = std::array::from_fn(|i| a[i].min(b[i]));
            let hi: [f64; 3] = std::array::from_fn(|i| a[i].max(b[i]));
            let tol = f32::from_bits(r.tol);
            // The least partner below the tile and the greatest at or above its end found so
            // far: a group or node whose leaves cannot extend either is skipped.
            let below = std::cell::Cell::new(begin);
            let above = std::cell::Cell::new(end);
            let wanted = |least: u32, greatest: u32| {
                least < below.get() || (greatest != NONE && greatest >= above.get())
            };
            let (mut tests, mut found_above) = (0, false);
            index.query(&lo, &hi, tol, &mut cost, wanted, |k| {
                let (least, greatest) = (k.leaves.min[0], k.leaves.max[0]);
                tests += 1;
                let t = f64::from(tol.max(f32::from_bits(k.tol)));
                let hit = if r.a == r.b {
                    on_segment(k.a, k.b, r.a, t + rounding(&[k.a, k.b, r.a]))
                } else {
                    on_segment(r.a, r.b, k.a, t + rounding(&[r.a, r.b, k.a]))
                };
                if hit {
                    below.set(below.get().min(least));
                    if greatest >= above.get() {
                        above.set(greatest + 1);
                        found_above = true;
                    }
                }
            })?;
            cost.tests += tests;
            let (below, above) = (below.get(), above.get());
            for found in [
                (below < begin).then_some(below),
                found_above.then_some(above - 1),
            ]
            .into_iter()
            .flatten()
            {
                r.clo = r.clo.min(found.min(r.leaf));
                r.chi = r.chi.max(found.max(r.leaf));
            }
        }
        if derive {
            let index: HashMap<(Key, Key), usize> =
                (0..rows.len()).map(|i| (rows[i].item(), i)).collect();
            for i in 0..rows.len() {
                let r = rows[i];
                if r.is_vertex() || r.clo > r.chi {
                    continue;
                }
                for p in [r.a, r.b] {
                    if let Some(&j) = index.get(&(p, p)) {
                        rows[j].clo = rows[j].clo.min(r.clo);
                        rows[j].chi = rows[j].chi.max(r.chi);
                    }
                }
            }
            for i in 0..rows.len() {
                let r = rows[i];
                if r.is_vertex() {
                    continue;
                }
                if let (Some(&ja), Some(&jb)) = (index.get(&(r.a, r.a)), index.get(&(r.b, r.b))) {
                    let lo = rows[ja].clo.max(rows[jb].clo).max(lock[i].0);
                    let hi = rows[ja].chi.min(rows[jb].chi).min(lock[i].1);
                    if lo <= hi {
                        rows[i].clo = rows[i].clo.min(lo);
                        rows[i].chi = rows[i].chi.max(hi);
                    }
                }
            }
        }
        cost.rows = rows.len() as u64;
        for r in rows.iter_mut() {
            r.lo = r.clo;
            r.hi = r.chi;
            r.tol = NO_TOL;
        }
        rows.retain(|r| r.lo <= r.hi);
        Ok(cost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SORT: SortConfig = SortConfig {
        run_bytes: 1 << 16,
        fan_in: 4,
        buffer_bytes: 1 << 12,
        jobs: 1,
    };

    fn k(p: [f64; 3]) -> Key {
        key(p.map(|v| v as f32))
    }

    type Host = (u32, Key, Key, f32);
    type Guest = (u32, Key, f32);

    /// The overlap rows (merged per leaf and item) and stats of the real join.
    fn join(hosts: &[Host], guests: &[Guest]) -> (Vec<InterfaceRow>, IncidenceStats) {
        let (rows, stats, _) = join_kept(hosts, guests, |_, _| ());
        (rows, stats)
    }

    /// The join, with `then` given its rows and fallback store before the scratch goes.
    fn join_kept(
        hosts: &[Host],
        guests: &[Guest],
        then: impl FnOnce(&mut Vec<InterfaceRow>, &Fallback),
    ) -> (Vec<InterfaceRow>, IncidenceStats, Vec<InterfaceRow>) {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let mut h = SequenceWriter::<Candidate>::new(&scratch, "h", 1 << 12).unwrap();
        for &(leaf, a, b, tol) in hosts {
            h.push(&Candidate {
                leaf,
                a,
                b,
                tol: tol.to_bits(),
            })
            .unwrap();
        }
        let mut g = SequenceWriter::<Candidate>::new(&scratch, "g", 1 << 12).unwrap();
        for &(leaf, a, tol) in guests {
            g.push(&Candidate {
                leaf,
                a,
                b: a,
                tol: tol.to_bits(),
            })
            .unwrap();
        }
        let (h, g) = (h.finish().unwrap(), g.finish().unwrap());
        let mut rows = ExternalSorter::<InterfaceRow>::new(&scratch, SORT).unwrap();
        let mut stats = IncidenceStats::default();
        let fallback = kd_join(
            &scratch,
            SORT,
            &h,
            &g,
            &mut rows,
            &mut stats,
            &|_| Ok(()),
            &|| Ok(()),
        )
        .unwrap();
        let mut rows: Vec<InterfaceRow> = rows.finish().unwrap().map(Result::unwrap).collect();
        merge_by_leaf(&mut rows);
        let mut after = rows.clone();
        then(&mut after, &fallback);
        (rows, stats, after)
    }

    fn merge_by_leaf(rows: &mut Vec<InterfaceRow>) {
        rows.sort_unstable();
        rows.dedup_by(|later, earlier| {
            if (later.leaf, later.a, later.b) == (earlier.leaf, earlier.a, earlier.b) {
                earlier.absorb(later);
                true
            } else {
                false
            }
        });
    }

    /// The predicate over every pair: the rows an exact join emits.
    fn brute(hosts: &[Host], guests: &[Guest]) -> Vec<InterfaceRow> {
        let mut rows = Vec::new();
        for h in hosts {
            for g in guests {
                if h.0 == g.0 {
                    continue;
                }
                let tol = f64::from(h.3.max(g.2)) + rounding(&[h.1, h.2, g.1]);
                if on_segment(h.1, h.2, g.1, tol) {
                    let (lo, hi) = (h.0.min(g.0), h.0.max(g.0));
                    rows.push(InterfaceRow::certain(h.0, h.1, h.2, lo, hi));
                    rows.push(InterfaceRow::certain(g.0, g.1, g.1, lo, hi));
                }
            }
        }
        merge_by_leaf(&mut rows);
        rows
    }

    /// Each leaf's rows resolved as its own tile (re-test only: these rows imply no others).
    fn resolve_per_leaf(rows: &mut Vec<InterfaceRow>, fallback: &Fallback) -> Resolved {
        let leaves: std::collections::BTreeSet<u32> = rows.iter().map(|r| r.leaf).collect();
        let mut out = Vec::new();
        let mut total = Resolved::default();
        for leaf in leaves {
            let mut tile: Vec<_> = rows.iter().filter(|r| r.leaf == leaf).copied().collect();
            let cost = fallback
                .resolve_with(&mut tile, leaf, leaf + 1, false, &|| Ok(()))
                .unwrap();
            total.tested += cost.tested;
            total.visits += cost.visits;
            total.tests += cost.tests;
            out.extend(tile);
        }
        for r in &mut out {
            r.tol = NO_TOL;
        }
        *rows = out;
        total
    }

    /// Within budget the join's rows equal the brute-force predicate's exactly. Past it, the
    /// locks cover every true pair (no miss), the certain intervals claim no false one, and
    /// resolving the provisional rows recovers the truth exactly.
    fn agrees(hosts: &[Host], guests: &[Guest]) -> IncidenceStats {
        let (rows, stats, resolved) = join_kept(hosts, guests, |rows, fallback| {
            resolve_per_leaf(rows, fallback);
        });
        let truth = brute(hosts, guests);
        if stats.fallback_regions == 0 {
            assert_eq!(rows, truth, "{stats:?}");
            return stats;
        }
        for t in &truth {
            let r = rows
                .iter()
                .find(|r| (r.leaf, r.a, r.b) == (t.leaf, t.a, t.b))
                .unwrap_or_else(|| panic!("missed {t:?} {stats:?}"));
            assert!(r.lo <= t.lo && r.hi >= t.hi, "{r:?} {t:?}");
        }
        for r in rows.iter().filter(|r| r.clo <= r.chi) {
            let t = truth
                .iter()
                .find(|t| (t.leaf, t.a, t.b) == (r.leaf, r.a, r.b))
                .unwrap_or_else(|| panic!("false certain {r:?}"));
            assert!(r.clo >= t.lo && r.chi <= t.hi, "{r:?} {t:?}");
        }
        let mut resolved = resolved;
        merge_by_leaf(&mut resolved);
        assert_eq!(resolved, truth, "{stats:?}");
        stats
    }

    /// Round 5: a parent merges its leaves' rows of one item into one row. Leaf 2 meets the
    /// host block at its large tolerance, leaf 1 (same point, small tolerance) does not; the
    /// merged row keeps the larger tolerance, so resolution still finds leaf 2's partner.
    /// Controls: leaf 2 alone finds it, leaf 1 alone does not.
    #[test]
    fn merged_owners_keep_the_partners_of_every_owner() {
        let hosts: Vec<Host> = (0..600u32)
            .map(|i| {
                let y = f64::from(i) / 256.;
                (0, k([0., y, 0.]), k([1., y, 0.]), 1. / 256.)
            })
            .collect();
        let target = k([0.5, 2.5, 0.]);
        let mut guests: Vec<Guest> = (0..512u32)
            .map(|i| {
                let c = |v: u32| f64::from(v % 8) / 2.;
                (3 + i, k([c(i), c(i / 8), c(i / 64)]), 0.5)
            })
            .collect();
        guests.extend([(1, target, 1. / 256.), (2, target, 0.5)]);
        let truth = brute(&hosts, &guests);
        assert!(truth
            .iter()
            .any(|r| r.leaf == 2 && r.a == target && r.lo == 0));
        assert!(!truth.iter().any(|r| r.leaf == 1 && r.a == target));
        let (_, stats, _) = join_kept(&hosts, &guests, |rows, fallback| {
            let tile = |leaves: &[u32]| -> Vec<InterfaceRow> {
                rows.iter()
                    .filter(|r| leaves.contains(&r.leaf) && r.a == target)
                    .copied()
                    .collect()
            };
            let mut control = tile(&[2]);
            fallback.resolve(&mut control, 2, 3, &|| Ok(())).unwrap();
            assert!(control.iter().any(|r| r.lo == 0), "{control:?}");
            let mut alone = tile(&[1]);
            fallback.resolve(&mut alone, 1, 2, &|| Ok(())).unwrap();
            assert!(alone.is_empty(), "{alone:?}");
            let mut parent = tile(&[1, 2]);
            assert_eq!(parent.len(), 2);
            merge_rows(&mut parent);
            assert_eq!(parent.len(), 1);
            assert_eq!(parent[0].tol, 0.5f32.to_bits());
            fallback.resolve(&mut parent, 1, 3, &|| Ok(())).unwrap();
            assert!(parent.iter().any(|r| r.lo == 0), "{parent:?}");
        });
        assert!(stats.fallback_regions > 0, "{stats:?}");
    }

    /// Round 5: resolution queries the index near each row, not every kept group. A slab of
    /// hosts with guests off it (all false) or on it (all true), every region a superset:
    /// doubling the kept groups doubles the work (a global scan quadruples it: 4.19M
    /// comparisons at 2,048), because a query enters only nodes near its row whose leaves
    /// could still widen its answer.
    #[test]
    fn resolution_work_grows_linearly_with_the_kept_groups() {
        for (y, z, hit) in [(0.2, 0.3, false), (0.002, 0., true)] {
            let mut costs = Vec::new();
            for n in [512u32, 1024, 2048, 4096] {
                let hosts: Vec<Host> = (0..n)
                    .map(|i| {
                        let y = f64::from(i) * 1e-6;
                        (0, k([0., y, 0.]), k([1., y, 0.]), 0.1)
                    })
                    .collect();
                let guests: Vec<Guest> = (0..n)
                    .map(|i| (1 + i / 64, k([f64::from(i) / f64::from(n), y, z]), 0.1))
                    .collect();
                let (_, stats, _) = join_kept(&hosts, &guests, |rows, fallback| {
                    let mut total = 0;
                    for leaf in 1..=n / 64 {
                        let mut tile: Vec<_> =
                            rows.iter().filter(|r| r.leaf == leaf).copied().collect();
                        assert_eq!(tile.len(), 64);
                        let cost = fallback
                            .resolve(&mut tile, leaf, leaf + 1, &|| Ok(()))
                            .unwrap();
                        assert_eq!(cost.tested, 64, "{cost:?}");
                        assert_eq!(tile.len(), if hit { 64 } else { 0 });
                        assert!(tile.iter().all(|r| r.lo == 0 && r.hi == leaf));
                        total += cost.visits + cost.tests;
                    }
                    costs.push(total);
                });
                assert!(stats.fallback_regions > 0, "{stats:?}");
            }
            // Per row: at most the node levels' children of one path, plus its first hit.
            for (&cost, n) in costs.iter().zip([512u64, 1024, 2048, 4096]) {
                let levels = u64::from(n.ilog2()).div_ceil(FANOUT.ilog2().into());
                assert!(cost <= n * (FANOUT as u64 + 1) * (levels + 1), "{costs:?}");
            }
            for w in costs.windows(2) {
                assert!(w[1] <= 5 * w[0] / 2, "{costs:?}");
            }
        }
    }

    /// Decoy hosts pinning the root to x in [0, 1] and making x its split axis (short
    /// decoys cross the y mid, none the z mid or x = 1/2), so the root splits at x = 1/2.
    fn decoys(tol: f32) -> Vec<Host> {
        let mut hosts = Vec::new();
        for i in 0..200u32 {
            let x = if i % 2 == 0 {
                f64::from(i) / 500.
            } else {
                1. - f64::from(i) / 500.
            };
            let z = 0.1 + f64::from(i % 10) * 0.01;
            hosts.push((2 + i, k([x, 0.24, z]), k([x, 0.26, z]), tol));
        }
        hosts.push((300, k([0., 0.1, 0.1]), k([0., 0.1, 0.11]), tol));
        hosts.push((301, k([1., 0.4, 0.39]), k([1., 0.4, 0.4]), tol));
        hosts
    }

    /// Round 2: the host is indexed by its own small tolerance, but the pair predicate takes
    /// the guest's larger one; the root split at x = 1/2 separates them unless the padding
    /// covers the guest's tolerance.
    #[test]
    fn a_guest_with_a_larger_tolerance_meets_its_host() {
        let x = 0.5 - 2f64.powi(-12);
        let host = (
            0,
            k([x, 0.125, 0.125]),
            k([x, 0.125, 0.375]),
            2f32.powi(-13),
        );
        let guest = (1, k([0.5, 0.125, 0.25]), 2f32.powi(-10));
        let (rows, stats) = join(&[host], &[guest]);
        assert_eq!(stats.overlaps, 1, "{stats:?}");
        assert_eq!(rows, brute(&[host], &[guest]));
        assert_eq!(rows.len(), 2);
        let mut hosts = vec![host];
        hosts.extend(decoys(2f32.powi(-13)));
        let stats = agrees(&hosts, &[guest]);
        assert!(stats.kd_splits > 0 && stats.overlaps == 1, "{stats:?}");
    }

    /// Round 3: a guest the predicate accepts beyond an endpoint lies up to sqrt(2) tau from
    /// the segment, outside a tau-grown box. Decoys pin the root to x in [0, 1] and make x
    /// its split axis, so the split plane x = 1/2 falls between the endpoint and the guest:
    /// only the sqrt(2) padding sends the host to the guest's side.
    #[test]
    fn a_guest_beyond_an_endpoint_meets_its_host() {
        let t = 2f64.powi(-10);
        let b = [0.5 - 1.25 * t, 0.125, 0.375];
        let a = [b[0] - 0.125, 0.125, b[2] - 0.125];
        let host = (0, k(a), k(b), (t / 4.) as f32);
        let guest = (1, k([0.5, 0.125, 0.375]), t as f32);
        let mut hosts = vec![host];
        hosts.extend(decoys((t / 4.) as f32));
        let truth = brute(&hosts, &[guest]);
        assert!(
            truth.iter().any(|r| r.leaf == 1),
            "the predicate accepts the probe"
        );
        let stats = agrees(&hosts, &[guest]);
        assert!(stats.kd_splits > 0, "{stats:?}");
    }

    /// Random near-degenerate configurations: bundles of nearly collinear hosts, guests on,
    /// near and just beyond them (and past the endpoint caps), mixed tolerances.
    #[test]
    fn the_join_matches_brute_force_on_random_near_degenerate_inputs() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for round in 0..40 {
            let mut hosts: Vec<Host> = Vec::new();
            let mut guests: Vec<Guest> = Vec::new();
            let lines = 1 + (next() * 6.) as usize;
            let per = [1usize, 3, 40, 150][round % 4];
            for _ in 0..lines {
                let a = [next(), next(), next()];
                let d = [next() - 0.5, next() - 0.5, next() - 0.5];
                let n = (d.iter().map(|v| v * v).sum::<f64>()).sqrt().max(1e-9);
                let d = d.map(|v| v / n * (0.05 + next() * 0.5));
                let step = 2f64.powi(-(8 + (next() * 4.) as i32));
                for _ in 0..per {
                    let leaf = (next() * 12.) as u32;
                    let jitter = |s: f64, r: &mut dyn FnMut() -> f64| (r() - 0.5) * s;
                    let (s0, s1) = (next() * 0.6, 0.4 + next() * 0.6);
                    let off: [f64; 3] = std::array::from_fn(|_| jitter(step, &mut next));
                    let pa: [f64; 3] = std::array::from_fn(|i| a[i] + d[i] * s0 + off[i]);
                    let pb: [f64; 3] = std::array::from_fn(|i| a[i] + d[i] * s1 + off[i]);
                    if k(pa) != k(pb) {
                        let (pa, pb) = if k(pa) < k(pb) { (pa, pb) } else { (pb, pa) };
                        hosts.push((leaf, k(pa), k(pb), (step * next()) as f32));
                    }
                    let s = -0.1 + next() * 1.2;
                    let off: [f64; 3] = std::array::from_fn(|_| jitter(step * 3., &mut next));
                    let p: [f64; 3] = std::array::from_fn(|i| a[i] + d[i] * s + off[i]);
                    guests.push(((next() * 12.) as u32, k(p), (step * next()) as f32));
                }
            }
            if hosts.is_empty() {
                continue;
            }
            agrees(&hosts, &guests);
        }
    }

    /// Output is one row per matched host and guest whatever the matches: 1,000 collinear
    /// hosts and 1,000 guests on all of them give at most 2,000 rows, not 4,000,000.
    #[test]
    fn rows_are_bounded_by_hosts_and_guests() {
        let hosts: Vec<Host> = (0..1000u32)
            .map(|i| (i, k([0., 0.5, 0.5]), k([1., 0.5, 0.5]), 1e-4))
            .collect();
        let guests: Vec<Guest> = (0..1000u32)
            .map(|i| {
                (
                    1000 + i,
                    k([0.0005 + f64::from(i) * 0.000999, 0.5, 0.5]),
                    1e-4,
                )
            })
            .collect();
        let (rows, stats) = join(&hosts, &guests);
        assert!(stats.rows <= 2000 && rows.len() <= 2000, "{stats:?}");
        assert!(stats.matched_guests == 1000, "{stats:?}");
    }

    /// 2,048 random chords of one cell (long against every region, the pairs no split
    /// separates) against 2,048 guests, half on a chord: the join falls back to its superset
    /// where the exact tests pass the budget, so its tests stay within the budget times the
    /// records it moves, it misses no pair, and resolution recovers the exact truth.
    #[test]
    fn random_chords_fall_back_within_budget_and_resolve_exactly() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let n = 2048u32;
        let mut hosts = Vec::new();
        for i in 0..n {
            let a = [next(), next(), next()];
            let b = [next(), next(), next()];
            let (a, b) = if k(a) < k(b) { (a, b) } else { (b, a) };
            hosts.push((i % 97, k(a), k(b), 1e-3));
        }
        // Half the guests on a host, half anywhere.
        let mut guests: Vec<Guest> = Vec::new();
        for i in 0..n {
            let p = if i % 2 == 0 {
                let h = &hosts[(i as usize * 7) % hosts.len()];
                let (a, b) = (point(h.1), point(h.2));
                let t = next();
                std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t)
            } else {
                [next(), next(), next()]
            };
            guests.push((100 + i % 89, k(p), 1e-3));
        }
        let stats = agrees(&hosts, &guests);
        assert!(stats.fallback_regions > 0, "{stats:?}");
        let records = 2 * u64::from(n);
        assert!(
            stats.tests <= BUDGET_TESTS_PER_RECORD * records * u64::from(MAX_DEPTH),
            "{stats:?}"
        );
        assert!(stats.tests < records * records / 8, "{stats:?}");
    }

    /// A group of one item per leaf is charged in the stage's resident bound.
    #[test]
    fn item_groups_are_charged_per_leaf() {
        let base = memory_need(SORT, 256, 0);
        let more = memory_need(SORT, 256, 1 << 20);
        assert!(more - base >= (1 << 20) * std::mem::size_of::<Item>() as u64);
    }
}
