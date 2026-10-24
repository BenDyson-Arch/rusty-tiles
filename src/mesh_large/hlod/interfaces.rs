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
/// provisional, and a tile it keeps from its budget resolves it exactly (`Resolver`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct InterfaceRow {
    pub leaf: u32,
    pub a: Key,
    pub b: Key,
    pub lo: u32,
    pub hi: u32,
    pub clo: u32,
    pub chi: u32,
}

/// No certain partner (an empty interval).
pub(crate) const NO_LEAF: (u32, u32) = (u32::MAX, 0);

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
    }
    pub(crate) fn is_vertex(&self) -> bool {
        self.a == self.b
    }
    /// Whether a tile over leaves `[begin, end)` holding this row shares it with a tile
    /// outside, i.e. must retain it as an outer interface.
    pub(crate) fn outer(&self, begin: u32, end: u32) -> bool {
        self.lo < begin || self.hi >= end
    }
    /// The item, without its recording leaf or interval.
    pub(crate) fn item(&self) -> (Key, Key) {
        (self.a, self.b)
    }
}

/// Sort rows by item and merge each item's intervals (exact: see the module notes).
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
    const SIZE: usize = 44;
    fn encode(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.leaf.to_le_bytes());
        put(out, 4, self.a);
        put(out, 16, self.b);
        out[28..32].copy_from_slice(&self.lo.to_le_bytes());
        out[32..36].copy_from_slice(&self.hi.to_le_bytes());
        out[36..40].copy_from_slice(&self.clo.to_le_bytes());
        out[40..44].copy_from_slice(&self.chi.to_le_bytes());
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

/// Worst-case scratch of the incidence pass for `triangles` source triangles, the largest of
/// its phases (each phase's live files, not a sum across phases that never overlap):
///
/// - items: the item sort through `finish`;
/// - reduce: the sorted items being streamed, the exact rows pushed so far and the open
///   candidates being written;
/// - join: the rows pushed so far (exact and overlap) and the overlap join's live bytes,
///   candidates included (`overlap_allowance`); the item runs are gone by then;
/// - walk: the row sort through `finish` and its stream, beside the fallback store.
///
/// A triangle contributes at most three point and three segment items, so at most six
/// (leaf, item) pairs; a pair gets at most one exact row and one overlap row (the join
/// emits one row per matched host or guest), and at most one candidate of `Candidate::SIZE`.
pub(crate) fn scratch_need(triangles: u64, sorter: SortConfig) -> Option<u64> {
    let items = triangles.checked_mul(6)?;
    let item: SortSpill = config::sort_spill::<Item>(items, sorter)?;
    let exact: SortSpill = config::sort_spill::<InterfaceRow>(items, sorter)?;
    let rows: SortSpill = config::sort_spill::<InterfaceRow>(items.checked_mul(2)?, sorter)?;
    let candidates = items.checked_mul(Candidate::SIZE as u64)?;
    let reduce = item
        .bytes
        .checked_add(item.stream_slack)?
        .checked_add(exact.push_peak)?
        .checked_add(candidates)?;
    let join = rows.push_peak.checked_add(overlap_allowance(
        items,
        triangles.checked_mul(3)?,
        triangles.checked_mul(3)?,
    ))?;
    // The fallback store (at most one copy of each candidate) lives through the walk.
    let walk = rows
        .finish_peak
        .checked_add(rows.stream_slack)?
        .checked_add(candidates)?;
    Some(item.finish_peak.max(reduce).max(join).max(walk))
}

/// Resident bound of the pass: two sorters live at once (items streamed into rows), one
/// leaf's deduplicated items, one item group (at most one item per leaf, in a vector at most
/// twice that), and the overlap join's held hosts and open sequence buffers.
pub(crate) fn memory_need(sorter: SortConfig, leaf_triangles: u64, leaves: u64) -> u64 {
    let item = std::mem::size_of::<Item>() as u64;
    sorter
        .memory_bound::<Item>()
        .saturating_add(sorter.memory_bound::<InterfaceRow>())
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
    /// Candidates kept for that resolution.
    pub fallback_candidates: u64,
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn build<'s>(
    scratch: &'s Scratch,
    sorter: SortConfig,
    nodes: &Sequence<NodeRecord>,
    triangles: &Sequence<FullTriangle>,
    depths: &Sequence<CornerDepth>,
    lattice: &LatticePlan,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(SortedStream<'s, InterfaceRow>, IncidenceStats, Fallback), JobError> {
    let mut stats = IncidenceStats::default();
    let mut items = ExternalSorter::<Item>::new(scratch, sorter)?;
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
        stats.items += buffer.len() as u64;
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
    let fallback = kd_join(
        scratch, sorter, &hosts, &guests, &mut rows, &mut stats, check,
    )?;
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
/// with its child writers, long spill and staged guests, the two slot files and the two
/// candidate sequences.
pub(crate) const KD_MAX_FILES: u64 = 2 * (MAX_DEPTH as u64 + 1) + 2 + 6 + 2 + 2;

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
/// The join's fixed allowance below which small inputs never fall.
const OVERLAP_FLOOR_BYTES: u64 = 64 << 20;

/// The scratch allowance of the overlap join: its `candidates` twice (kept from the reduce for
/// the final rows, and at most once more in the `Fallback` store) plus the proven live bytes
/// of its host and guest groups, at least 64 MiB.
/// Admitted from the counted inputs when the join starts (the capture declares it for at
/// most three host and three guest candidates per triangle, each its own group), and
/// enforced while writing (`resource_limit`, which the proof rules out). File rounding is in
/// the scratch admission (`KD_MAX_FILES`).
fn overlap_allowance(candidates: u64, host_groups: u64, guest_groups: u64) -> u64 {
    candidates
        .saturating_mul(2 * Candidate::SIZE as u64)
        .saturating_add(host_groups.saturating_mul(OVERLAP_BYTES_PER_HOST_GROUP))
        .saturating_add(guest_groups.saturating_mul(OVERLAP_BYTES_PER_GUEST_GROUP))
        .max(OVERLAP_FLOOR_BYTES)
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
/// an out-of-core kd split of both sets, without a sort, and exact throughout.
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
/// - Scratch: live bytes are counted against `overlap_allowance` (proof there).
#[allow(clippy::too_many_arguments)]
fn kd_join<'s>(
    scratch: &'s Scratch,
    sorter: SortConfig,
    hosts: &Sequence<Candidate>,
    guests: &Sequence<Candidate>,
    rows: &mut ExternalSorter<'s, InterfaceRow>,
    stats: &mut IncidenceStats,
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
    let budget = overlap_allowance(hosts.len() + guests.len(), host_groups, guest_groups);
    crate::spill::preflight(
        scratch.path(),
        budget,
        crate::spill::SpaceRole::Scratch,
        crate::spill::PreflightStage::Lod,
    )?;
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
    // locked (certain or provisional); a group matched provisionally is kept for resolution.
    let mut kept = [
        SequenceWriter::<Candidate>::new(scratch, "hlod-fallback-hosts", sorter.buffer_bytes)?,
        SequenceWriter::<Candidate>::new(scratch, "hlod-fallback-guests", sorter.buffer_bytes)?,
    ];
    for (side, seq) in [(0, hosts), (1, guests)] {
        let mut group = u64::MAX;
        let (mut exact, mut provisional) = (Extremes::EMPTY, Extremes::EMPTY);
        let mut last: Option<(Key, Key, u32)> = None;
        for c in seq.reader()? {
            let c = c?;
            if last != Some((c.a, c.b, c.tol)) {
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
            }
            if !provisional.is_empty() {
                kd.account(Candidate::SIZE as u64)?;
                kept[side].push(&c)?;
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
                rows.push(InterfaceRow {
                    leaf: c.leaf,
                    a: c.a,
                    b: c.b,
                    lo: lo.min(c.leaf),
                    hi: hi.max(c.leaf),
                    clo,
                    chi,
                })?;
                kd.stats.rows += 1;
            }
        }
    }
    let [kept_hosts, kept_guests] = kept;
    Ok(Fallback {
        hosts: Some(kept_hosts.finish()?),
        guests: Some(kept_guests.finish()?),
    })
}

/// The candidates of every group an over-budget region matched by a superset (provisional
/// rows), kept through the walk so a tile those rows keep from its budget can resolve them
/// exactly. At most one copy of each candidate.
#[derive(Default)]
pub(crate) struct Fallback {
    hosts: Option<Sequence<Candidate>>,
    guests: Option<Sequence<Candidate>>,
}

impl Fallback {
    /// Whether nothing was matched by a superset.
    pub(crate) fn is_empty(&self) -> bool {
        self.hosts.as_ref().is_none_or(|s| s.is_empty())
            && self.guests.as_ref().is_none_or(|s| s.is_empty())
    }

    /// Scratch bytes it holds.
    pub(crate) fn bytes(&self) -> u64 {
        [&self.hosts, &self.guests]
            .iter()
            .map(|s| s.as_ref().map_or(0, |s| s.len() * Candidate::SIZE as u64))
            .sum()
    }

    /// Resolve a tile's provisional rows exactly: each provisional point or segment is tested
    /// against the kept candidates of the other kind with the join's exact predicate, its
    /// certain interval gains the leaves it truly meets, and the rows derived from points and
    /// segments (segment endpoints, open edges between kept points, as `read_leaf` derives
    /// them) are recomputed from the resolved intervals. Every row then locks only its
    /// certain interval; a row left with none is dropped. Returns the provisional rows found.
    ///
    /// Work: the kept candidates are streamed once per kind against the tile's provisional
    /// rows of the other kind (held, at most the tile's rows): the tile's provisional rows
    /// times the kept candidates, paid only by a tile that cannot reach its budget otherwise.
    pub(crate) fn resolve(
        &self,
        rows: &mut Vec<InterfaceRow>,
        check: &(dyn Fn() -> Result<(), JobError> + Sync),
    ) -> Result<u64, JobError> {
        let provisional: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].provisional()).collect();
        if provisional.is_empty() {
            return Ok(0);
        }
        let wanted: HashMap<(u32, Key, Key), usize> = provisional
            .iter()
            .map(|&i| ((rows[i].leaf, rows[i].a, rows[i].b), i))
            .collect();
        let mut found: HashMap<usize, Extremes> = HashMap::with_capacity(provisional.len());
        // The tile's own provisional candidates of each kind, as the join saw them.
        let mut mine: [Vec<(Candidate, usize)>; 2] = [Vec::new(), Vec::new()];
        for (side, seq) in [(0usize, &self.hosts), (1, &self.guests)] {
            let Some(seq) = seq else { continue };
            for c in seq.reader()? {
                let c = c?;
                if let Some(&i) = wanted.get(&(c.leaf, c.a, c.b)) {
                    mine[side].push((c, i));
                }
            }
        }
        let test = |h: &Candidate, g: &Candidate| {
            h.leaf != g.leaf && {
                let tol = f64::from(f32::from_bits(h.tol).max(f32::from_bits(g.tol)))
                    + rounding(&[h.a, h.b, g.a]);
                on_segment(h.a, h.b, g.a, tol)
            }
        };
        // Provisional points against every kept host; provisional segments against every
        // kept guest.
        for (side, other) in [(1usize, &self.hosts), (0usize, &self.guests)] {
            let Some(other) = other else { continue };
            if mine[side].is_empty() {
                continue;
            }
            for (k, c) in other.reader()?.enumerate() {
                if k.is_multiple_of(1 << 12) {
                    check()?;
                }
                let c = c?;
                for (m, i) in &mine[side] {
                    let hit = if side == 1 { test(&c, m) } else { test(m, &c) };
                    if hit {
                        found.entry(*i).or_insert(Extremes::EMPTY).add(c.leaf);
                    }
                }
            }
        }
        drop(mine);
        drop(wanted);
        for (i, leaves) in found {
            let r = &mut rows[i];
            if let Some((lo, hi)) = leaves.others(r.leaf) {
                r.clo = r.clo.min(lo.min(r.leaf));
                r.chi = r.chi.max(hi.max(r.leaf));
            }
        }
        // Derived rows, recomputed from the resolved certain intervals: endpoints from their
        // segments, then open edges from their endpoints (an existing row only gains).
        let index: HashMap<(u32, Key, Key), usize> = (0..rows.len())
            .map(|i| ((rows[i].leaf, rows[i].a, rows[i].b), i))
            .collect();
        for i in 0..rows.len() {
            let r = rows[i];
            if r.is_vertex() || r.clo > r.chi {
                continue;
            }
            for p in [r.a, r.b] {
                if let Some(&j) = index.get(&(r.leaf, p, p)) {
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
            let ends = (
                index.get(&(r.leaf, r.a, r.a)),
                index.get(&(r.leaf, r.b, r.b)),
            );
            if let (Some(&ja), Some(&jb)) = ends {
                let (a, b) = (rows[ja], rows[jb]);
                let (lo, hi) = (a.clo.max(b.clo), a.chi.min(b.chi));
                if lo <= hi {
                    rows[i].clo = rows[i].clo.min(lo);
                    rows[i].chi = rows[i].chi.max(hi);
                }
            }
        }
        for r in rows.iter_mut() {
            r.lo = r.clo;
            r.hi = r.chi;
        }
        rows.retain(|r| r.lo <= r.hi);
        Ok(provisional.len() as u64)
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
        let fallback = kd_join(&scratch, SORT, &h, &g, &mut rows, &mut stats, &|| Ok(())).unwrap();
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

    /// Within budget the join's rows equal the brute-force predicate's exactly. Past it, the
    /// locks cover every true pair (no miss), the certain intervals claim no false one, and
    /// resolving the provisional rows recovers the truth exactly.
    fn agrees(hosts: &[Host], guests: &[Guest]) -> IncidenceStats {
        let (rows, stats, resolved) = join_kept(hosts, guests, |rows, fallback| {
            fallback.resolve(rows, &|| Ok(())).unwrap();
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
