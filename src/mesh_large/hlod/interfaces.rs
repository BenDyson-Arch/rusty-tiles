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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct InterfaceRow {
    pub leaf: u32,
    pub a: Key,
    pub b: Key,
    pub lo: u32,
    pub hi: u32,
}

impl InterfaceRow {
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
            Some(last) if last.item() == r.item() => {
                last.lo = last.lo.min(r.lo);
                last.hi = last.hi.max(r.hi);
            }
            _ => out.push(r),
        }
    }
    out.shrink_to_fit();
    *rows = out;
}

impl Record for InterfaceRow {
    const SIZE: usize = 36;
    fn encode(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.leaf.to_le_bytes());
        put(out, 4, self.a);
        put(out, 16, self.b);
        out[28..32].copy_from_slice(&self.lo.to_le_bytes());
        out[32..].copy_from_slice(&self.hi.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            leaf: u32_at(bytes, 0),
            a: get(bytes, 4),
            b: get(bytes, 16),
            lo: u32_at(bytes, 28),
            hi: u32_at(bytes, 32),
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

/// Worst-case spill of the exact pass plus the overlap allowance, for `triangles` source
/// triangles: every triangle contributes at most three point and three segment items, so at
/// most six (leaf, item) pairs; each pair at most one exact row and one overlap row (the
/// join emits one row per matched host or guest). Row sorts fill in bursts per group, so
/// they are charged on top of the whole item stream.
pub(crate) fn scratch_need(triangles: u64, sorter: SortConfig) -> Option<u64> {
    let items = triangles.checked_mul(6)?;
    let item: SortSpill = config::sort_spill::<Item>(items, sorter)?;
    let rows: SortSpill = config::sort_spill::<InterfaceRow>(items.checked_mul(2)?, sorter)?;
    let overlap = overlap_allowance(triangles);
    let reduce = item
        .bytes
        .checked_add(item.stream_slack)?
        .checked_add(rows.push_peak)?
        .checked_add(overlap)?;
    let walk = rows.finish_peak.checked_add(rows.stream_slack)?;
    Some(item.finish_peak.max(reduce).max(walk))
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
        .saturating_add(((EXACT_HOSTS + LONG_HOSTS) * std::mem::size_of::<Held>()) as u64)
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
    /// The overlap join's kd regions, splits, exact leaves, tolerance-scale aggregated
    /// leaves, regions whose long hosts were aggregated, and exact predicate tests.
    pub kd_regions: u64,
    pub kd_splits: u64,
    pub exact_leaves: u64,
    pub conservative_leaves: u64,
    pub long_overflows: u64,
    pub tests: u64,
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
) -> Result<(SortedStream<'s, InterfaceRow>, IncidenceStats), JobError> {
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
                rows.push(InterfaceRow {
                    leaf: g.leaf,
                    a,
                    b,
                    lo,
                    hi,
                })?;
                stats.rows += 1;
            }
        }
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
    kd_join(
        scratch,
        sorter,
        triangles.len(),
        &hosts,
        &guests,
        &mut rows,
        &mut stats,
        check,
    )?;
    Ok((rows.finish()?, stats))
}

/// One host copy in the kd join: an open-border segment of `leaf`, its index among the hosts
/// (`id`, its aggregate slot), its tolerance (f32 bits) and the axes it has been cut along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KdHost {
    id: u32,
    leaf: u32,
    a: Key,
    b: Key,
    tol: u32,
    cut: u8,
}

impl Record for KdHost {
    const SIZE: usize = 37;
    fn encode(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.id.to_le_bytes());
        out[4..8].copy_from_slice(&self.leaf.to_le_bytes());
        put(out, 8, self.a);
        put(out, 20, self.b);
        out[32..36].copy_from_slice(&self.tol.to_le_bytes());
        out[36] = self.cut;
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            id: u32_at(bytes, 0),
            leaf: u32_at(bytes, 4),
            a: get(bytes, 8),
            b: get(bytes, 20),
            tol: u32_at(bytes, 32),
            cut: bytes[36],
        }
    }
}

/// One guest in the kd join with its running aggregate: the least and greatest other leaf
/// whose host it was found on so far (`lo > hi` while unmatched).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KdGuest {
    leaf: u32,
    p: Key,
    tol: u32,
    lo: u32,
    hi: u32,
}

impl Record for KdGuest {
    const SIZE: usize = 28;
    fn encode(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.leaf.to_le_bytes());
        put(out, 4, self.p);
        out[16..20].copy_from_slice(&self.tol.to_le_bytes());
        out[20..24].copy_from_slice(&self.lo.to_le_bytes());
        out[24..28].copy_from_slice(&self.hi.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            leaf: u32_at(bytes, 0),
            p: get(bytes, 4),
            tol: u32_at(bytes, 16),
            lo: u32_at(bytes, 20),
            hi: u32_at(bytes, 24),
        }
    }
}

/// Hosts of a region tested exactly against each of its guests (a kd leaf).
pub(crate) const EXACT_HOSTS: usize = 64;
/// Long hosts one region tests exactly against its guests; more are aggregated instead.
pub(crate) const LONG_HOSTS: usize = 64;
/// Cuts a short host may take; each along a distinct axis, so at most `2^MAX_CUTS` copies.
const MAX_CUTS: u32 = 2;
/// A guard on splits along one path; geometry ends every path first (see `kd_join`).
const MAX_DEPTH: u32 = 64;
/// One host's aggregate slot: least other leaf, then greatest other leaf + 1 (0: none).
const SLOT: usize = 8;
/// Sequence buffers open at once: a region's host or guest reader, two child writers and a
/// long-host spill.
const KD_STREAMS: u64 = 4;
/// Scratch files the kd join holds at once (for the scratch admission's per-file rounding):
/// a pending right child's hosts and guests per split on the path, the region being split
/// with its four child writers and long spill, the slots, and the two candidate sequences.
pub(crate) const KD_MAX_FILES: u64 = 2 * (MAX_DEPTH as u64 + 1) + 2 + 5 + 1 + 2;

/// Proven scratch bound of the overlap join per source triangle. A triangle adds at most
/// three open (leaf, segment) pairs (hosts) and three open (leaf, point) pairs (guests). Live
/// at once, per host: its candidate (kept for the final rows), its aggregate slot and at most
/// `2^MAX_CUTS + 2` kd copies (a host branches only where cut, so at most `2^MAX_CUTS`
/// branches each hold one copy, plus the copy being read and the two being written in the
/// region being split); per guest: its candidate and at most two kd copies (in the region
/// being read and the child being written). The join's rows go to the row sorter, charged
/// there (`scratch_need`).
const OVERLAP_BYTES_PER_TRIANGLE: u64 = 3
    * (Candidate::SIZE as u64 + SLOT as u64 + ((1 << MAX_CUTS) + 2) * KdHost::SIZE as u64)
    + 3 * (Candidate::SIZE as u64 + 2 * KdGuest::SIZE as u64);
/// The join's fixed allowance below which small inputs never fall.
const OVERLAP_FLOOR_BYTES: u64 = 64 << 20;

/// The declared scratch allowance of the overlap join: the proven per-triangle bound, at
/// least 64 MiB. Enforced while writing (`resource_limit`, which the proof rules out).
fn overlap_allowance(triangles: u64) -> u64 {
    triangles
        .saturating_mul(OVERLAP_BYTES_PER_TRIANGLE)
        .max(OVERLAP_FLOOR_BYTES)
}

/// The two least and two greatest distinct leaves seen: enough to give the least and the
/// greatest leaf other than any one leaf.
#[derive(Clone, Copy, Debug, Default)]
struct Extremes {
    min: [Option<u32>; 2],
    max: [Option<u32>; 2],
}

impl Extremes {
    fn add(&mut self, leaf: u32) {
        match self.min {
            [None, _] => self.min[0] = Some(leaf),
            [Some(a), _] if leaf < a => self.min = [Some(leaf), Some(a)],
            [Some(a), b] if leaf > a && b.is_none_or(|b| leaf < b) => self.min[1] = Some(leaf),
            _ => {}
        }
        match self.max {
            [None, _] => self.max[0] = Some(leaf),
            [Some(a), _] if leaf > a => self.max = [Some(leaf), Some(a)],
            [Some(a), b] if leaf < a && b.is_none_or(|b| leaf > b) => self.max[1] = Some(leaf),
            _ => {}
        }
    }
    /// The least and greatest leaf other than `own`, if any.
    fn others(&self, own: u32) -> Option<(u32, u32)> {
        let pick = |v: [Option<u32>; 2]| if v[0] == Some(own) { v[1] } else { v[0] };
        Some((pick(self.min)?, pick(self.max)?))
    }
}

/// A region of the kd join: an axis-aligned box whose faces never cut extend without bound,
/// so the split planes partition every point (a guest lies in exactly one child).
#[derive(Clone, Copy, Debug)]
struct Region {
    lo: [f64; 3],
    hi: [f64; 3],
    cut_lo: [bool; 3],
    cut_hi: [bool; 3],
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

/// Hosts or guests of one kd region: the join's input candidates (the root) or a spill.
enum Source<'a, T: Record> {
    Root(&'a Sequence<Candidate>),
    Spill(Sequence<T>),
}

impl<T: Record> Source<'_, T> {
    fn len(&self) -> u64 {
        match self {
            Source::Root(s) => s.len(),
            Source::Spill(s) => s.len(),
        }
    }
}

type Stream<'a, T> = Box<dyn Iterator<Item = Result<T, JobError>> + 'a>;

fn host_stream<'a>(s: &'a Source<'_, KdHost>) -> Result<Stream<'a, KdHost>, JobError> {
    Ok(match s {
        Source::Root(seq) => Box::new(seq.reader()?.enumerate().map(|(i, c)| {
            let c = c?;
            Ok(KdHost {
                id: i as u32,
                leaf: c.leaf,
                a: c.a,
                b: c.b,
                tol: c.tol,
                cut: 0,
            })
        })),
        Source::Spill(seq) => Box::new(seq.reader()?.map(|h| Ok(h?))),
    })
}

fn guest_stream<'a>(s: &'a Source<'_, KdGuest>) -> Result<Stream<'a, KdGuest>, JobError> {
    Ok(match s {
        Source::Root(seq) => Box::new(seq.reader()?.map(|c| {
            let c = c?;
            Ok(KdGuest {
                leaf: c.leaf,
                p: c.a,
                tol: c.tol,
                lo: u32::MAX,
                hi: 0,
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

/// A host held in memory with its aggregate over this region's guests.
type Held = (KdHost, u32, u32);

/// The kd join's state: scratch, the host aggregate slots, the live-byte ledger and totals.
struct Kd<'c, 's> {
    scratch: &'s Scratch,
    buffer: usize,
    guest_tol: f64,
    floor: f64,
    pad_max: f64,
    slots: SlotFile,
    live: u64,
    budget: u64,
    stats: &'c mut IncidenceStats,
    rows: &'c mut ExternalSorter<'s, InterfaceRow>,
    check: &'c (dyn Fn() -> Result<(), JobError> + Sync),
}

impl<'s> Kd<'_, 's> {
    /// The index padding of a host. The predicate accepts a guest only within
    /// `tau = max(host tol, guest tol) + rounding` of the segment's line, projecting at most
    /// `tau` beyond an endpoint. So the guest lies within `sqrt(2) * tau` of the segment
    /// itself (beyond an endpoint, the along-line and perpendicular offsets are orthogonal
    /// and each at most `tau`), hence within that of some segment point in every coordinate,
    /// and the segment meets any region holding the guest grown by it. `tau` is bounded for
    /// every guest by the largest guest tolerance and the rounding floor of the largest
    /// coordinate; the factor `1 + 2^-20` absorbs the f64 rounding of the clipping.
    fn pad(&self, tol: u32) -> f64 {
        std::f64::consts::SQRT_2
            * (f64::from(f32::from_bits(tol)).max(self.guest_tol) + self.floor)
            * (1. + 1. / f64::from(1u32 << 20))
    }

    fn side(&self, region: &Region, axis: usize, h: &KdHost) -> Side {
        let pad = self.pad(h.tol);
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

    /// A guest's row once it has passed every region on its path.
    fn finish_guest(&mut self, g: &KdGuest) -> Result<(), JobError> {
        if g.lo <= g.hi {
            self.stats.matched_guests += 1;
            self.rows.push(InterfaceRow {
                leaf: g.leaf,
                a: g.p,
                b: g.p,
                lo: g.lo.min(g.leaf),
                hi: g.hi.max(g.leaf),
            })?;
            self.stats.rows += 1;
        }
        Ok(())
    }

    fn merge_slot(&mut self, id: u32, lo: u32, hi: u32) -> Result<(), JobError> {
        let mut bytes = [0u8; SLOT];
        self.slots.read(u64::from(id), &mut bytes)?;
        let (old_lo, old_hi) = (u32_at(&bytes, 0), u32_at(&bytes, 4));
        let (lo, hi) = if old_hi == 0 {
            (lo, hi)
        } else {
            (old_lo.min(lo), (old_hi - 1).max(hi))
        };
        bytes[..4].copy_from_slice(&lo.to_le_bytes());
        bytes[4..].copy_from_slice(&(hi + 1).to_le_bytes());
        self.slots.write(u64::from(id), &bytes)?;
        Ok(())
    }

    fn flush(&mut self, held: &[Held]) -> Result<(), JobError> {
        for (h, lo, hi) in held {
            if lo <= hi {
                self.merge_slot(h.id, *lo, *hi)?;
            }
        }
        Ok(())
    }

    /// Test `g` against `h` with the exact predicate, aggregating both sides on a match.
    fn test(&mut self, (host, lo, hi): &mut Held, g: &mut KdGuest) {
        if host.leaf == g.leaf {
            return;
        }
        self.stats.tests += 1;
        let tol = f64::from(f32::from_bits(host.tol).max(f32::from_bits(g.tol)))
            + rounding(&[host.a, host.b, g.p]);
        if !on_segment(host.a, host.b, g.p, tol) {
            return;
        }
        self.stats.overlaps += 1;
        *lo = (*lo).min(g.leaf);
        *hi = (*hi).max(g.leaf);
        g.lo = g.lo.min(host.leaf);
        g.hi = g.hi.max(host.leaf);
    }

    /// The tolerance-scale leaf: every host meeting the region pairs with every guest in it,
    /// a superset of the predicate within `8 sqrt(3) + 1` paddings, in one pass per side.
    fn conservative(
        &mut self,
        hosts: &Source<'_, KdHost>,
        guests: &Source<'_, KdGuest>,
    ) -> Result<(), JobError> {
        self.stats.conservative_leaves += 1;
        let mut host_leaves = Extremes::default();
        for h in host_stream(hosts)? {
            host_leaves.add(h?.leaf);
        }
        let mut guest_leaves = Extremes::default();
        for g in guest_stream(guests)? {
            let mut g = g?;
            guest_leaves.add(g.leaf);
            if let Some((lo, hi)) = host_leaves.others(g.leaf) {
                g.lo = g.lo.min(lo);
                g.hi = g.hi.max(hi);
            }
            self.finish_guest(&g)?;
        }
        for h in host_stream(hosts)? {
            let h = h?;
            if let Some((lo, hi)) = guest_leaves.others(h.leaf) {
                self.merge_slot(h.id, lo, hi)?;
            }
        }
        Ok(())
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
        if guests.len() == 0 {
            self.release(hosts);
            self.release(guests);
            return Ok(());
        }
        if hosts.len() == 0 {
            for g in guest_stream(&guests)? {
                self.finish_guest(&g?)?;
            }
            self.release(hosts);
            self.release(guests);
            return Ok(());
        }
        if hosts.len() <= EXACT_HOSTS as u64 {
            self.stats.exact_leaves += 1;
            let mut held: Vec<Held> = Vec::with_capacity(EXACT_HOSTS);
            for h in host_stream(&hosts)? {
                held.push((h?, u32::MAX, 0));
            }
            for (k, g) in guest_stream(&guests)?.enumerate() {
                if k.is_multiple_of(1 << 12) {
                    (self.check)()?;
                }
                let mut g = g?;
                for h in held.iter_mut() {
                    self.test(h, &mut g);
                }
                self.finish_guest(&g)?;
            }
            self.flush(&held)?;
            self.release(hosts);
            self.release(guests);
            return Ok(());
        }
        let extent: [f64; 3] = std::array::from_fn(|i| region.hi[i] - region.lo[i]);
        let eligible: Vec<usize> = (0..3).filter(|&i| extent[i] > 8. * self.pad_max).collect();
        if depth >= MAX_DEPTH || eligible.is_empty() {
            self.conservative(&hosts, &guests)?;
            self.release(hosts);
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
        let mut long_leaves = Extremes::default();
        for h in host_stream(&hosts)? {
            let h = h?;
            match self.side(&region, axis, &h) {
                Side::Left => self.push(&mut left_hosts, &h)?,
                Side::Right => self.push(&mut right_hosts, &h)?,
                Side::Both => {
                    let h = KdHost {
                        cut: h.cut | (1 << axis),
                        ..h
                    };
                    self.push(&mut left_hosts, &h)?;
                    self.push(&mut right_hosts, &h)?;
                }
                Side::Long => {
                    long_leaves.add(h.leaf);
                    if let Some(spill) = long_spill.as_mut() {
                        self.account(KdHost::SIZE as u64)?;
                        spill.push(&h)?;
                    } else if long.len() < LONG_HOSTS {
                        long.push((h, u32::MAX, 0));
                    } else {
                        let mut spill = self.writer::<KdHost>("hlod-kd-long")?;
                        for (held, _, _) in long.drain(..) {
                            self.push(&mut spill, &held)?;
                        }
                        self.push(&mut spill, &h)?;
                        long_spill = Some(spill);
                    }
                }
            }
        }
        // Guests: against the long hosts (exactly, or aggregated once they overflow), then on
        // to their side of the plane.
        let mid = region.mid(axis);
        let (mut left_guests, mut right_guests) = (
            self.writer::<KdGuest>("hlod-kd-guests")?,
            self.writer::<KdGuest>("hlod-kd-guests")?,
        );
        let mut guest_leaves = Extremes::default();
        for (k, g) in guest_stream(&guests)?.enumerate() {
            if k.is_multiple_of(1 << 12) {
                (self.check)()?;
            }
            let mut g = g?;
            if long_spill.is_some() {
                guest_leaves.add(g.leaf);
                if let Some((lo, hi)) = long_leaves.others(g.leaf) {
                    g.lo = g.lo.min(lo);
                    g.hi = g.hi.max(hi);
                }
            } else {
                for h in long.iter_mut() {
                    self.test(h, &mut g);
                }
            }
            if point(g.p)[axis] < mid {
                self.push(&mut left_guests, &g)?;
            } else {
                self.push(&mut right_guests, &g)?;
            }
        }
        match long_spill {
            Some(spill) => {
                self.stats.long_overflows += 1;
                let spill = spill.finish()?;
                for h in spill.reader()? {
                    let h = h?;
                    if let Some((lo, hi)) = guest_leaves.others(h.leaf) {
                        self.merge_slot(h.id, lo, hi)?;
                    }
                }
                self.live -= spill.len() * KdHost::SIZE as u64;
            }
            None => self.flush(&long)?,
        }
        self.release(hosts);
        self.release(guests);
        let (left_hosts, right_hosts) = (left_hosts.finish()?, right_hosts.finish()?);
        let (left_guests, right_guests) = (left_guests.finish()?, right_guests.finish()?);
        // The right child waits, closed, while the left is joined.
        self.region(
            region.half(axis, false),
            Source::Spill(left_hosts),
            Source::Spill(left_guests),
            depth + 1,
        )?;
        self.region(
            region.half(axis, true),
            Source::Spill(right_hosts),
            Source::Spill(right_guests),
            depth + 1,
        )
    }
}

/// The overlap join of open points (guests) against open segments of other leaves (hosts),
/// bounded by design: an out-of-core kd split of both sets, without a sort.
///
/// - Index: a host copy goes to every child its padded segment meets (`Kd::pad` proves the
///   padding covers the predicate's whole acceptance region), a guest to the one child
///   holding it, so every accepted pair meets in some region.
/// - Copies: a short host is cut at most once per axis and at most `MAX_CUTS` times; a host
///   long against its region (or past its cuts) stays there and is tested against all of the
///   region's guests. A host has at most `2^MAX_CUTS` copies, a guest one.
/// - Work: a region of at most `EXACT_HOSTS` hosts is a leaf, tested exactly (at most that
///   many tests per guest); a split tests each guest against at most `LONG_HOSTS` long hosts,
///   aggregating instead when there are more. Splits halve one axis longer than eight
///   paddings, so a path has at most `3 log2(extent / 8 padding)` splits (under 57 with the
///   rounding floor); a region no split can shorten is aggregated at tolerance scale. Every
///   record is moved O(1) times per level: O((hosts + guests) * depth) moves and tests.
/// - Memory: at most `EXACT_HOSTS + LONG_HOSTS` held hosts and `KD_STREAMS` buffers.
/// - Output: one row per matched guest and one per matched host, each with the exact union of
///   its partners' leaf intervals (aggregation is a superset only where it is used).
/// - Scratch: live bytes are counted against `overlap_allowance` (proof there).
#[allow(clippy::too_many_arguments)]
fn kd_join<'s>(
    scratch: &'s Scratch,
    sorter: SortConfig,
    triangles: u64,
    hosts: &Sequence<Candidate>,
    guests: &Sequence<Candidate>,
    rows: &mut ExternalSorter<'s, InterfaceRow>,
    stats: &mut IncidenceStats,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(), JobError> {
    if hosts.is_empty() || guests.is_empty() {
        return Ok(());
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
    for h in hosts.reader()? {
        let h = h?;
        extend(h.a);
        extend(h.b);
        host_tol = host_tol.max(f32::from_bits(h.tol));
    }
    for g in guests.reader()? {
        let g = g?;
        extend(g.a);
        guest_tol = guest_tol.max(f32::from_bits(g.tol));
    }
    let budget = overlap_allowance(triangles);
    crate::spill::preflight(
        scratch.path(),
        budget,
        crate::spill::SpaceRole::Scratch,
        crate::spill::PreflightStage::Lod,
    )?;
    let mut slots = SlotFile::new(scratch, "hlod-kd-slots", SLOT)?;
    slots.write(hosts.len() - 1, &[0; SLOT])?;
    // `rounding` of any three of these points is at most this floor.
    let floor = magnitude * f64::from(f32::EPSILON) * 4.;
    let mut kd = Kd {
        scratch,
        buffer: sorter.buffer_bytes,
        guest_tol: f64::from(guest_tol),
        floor,
        pad_max: 0.,
        slots,
        live: (hosts.len() + guests.len()) * Candidate::SIZE as u64 + hosts.len() * SLOT as u64,
        budget,
        stats,
        rows,
        check,
    };
    kd.pad_max = kd.pad(host_tol.to_bits());
    kd.account(0)?;
    let root = Region {
        lo,
        hi,
        cut_lo: [false; 3],
        cut_hi: [false; 3],
    };
    kd.region(root, Source::Root(hosts), Source::Root(guests), 0)?;
    for (i, h) in hosts.reader()?.enumerate() {
        let h = h?;
        let mut bytes = [0u8; SLOT];
        kd.slots.read(i as u64, &mut bytes)?;
        let (lo, hi) = (u32_at(&bytes, 0), u32_at(&bytes, 4));
        if hi != 0 {
            kd.stats.matched_hosts += 1;
            kd.rows.push(InterfaceRow {
                leaf: h.leaf,
                a: h.a,
                b: h.b,
                lo: lo.min(h.leaf),
                hi: (hi - 1).max(h.leaf),
            })?;
            kd.stats.rows += 1;
        }
    }
    Ok(())
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
        kd_join(&scratch, SORT, 1, &h, &g, &mut rows, &mut stats, &|| Ok(())).unwrap();
        let mut rows: Vec<InterfaceRow> = rows.finish().unwrap().map(Result::unwrap).collect();
        merge_by_leaf(&mut rows);
        (rows, stats)
    }

    fn merge_by_leaf(rows: &mut Vec<InterfaceRow>) {
        rows.sort_unstable();
        rows.dedup_by(|later, earlier| {
            if (later.leaf, later.a, later.b) == (earlier.leaf, earlier.a, earlier.b) {
                earlier.lo = earlier.lo.min(later.lo);
                earlier.hi = earlier.hi.max(later.hi);
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
                    rows.push(InterfaceRow {
                        leaf: h.0,
                        a: h.1,
                        b: h.2,
                        lo,
                        hi,
                    });
                    rows.push(InterfaceRow {
                        leaf: g.0,
                        a: g.1,
                        b: g.1,
                        lo,
                        hi,
                    });
                }
            }
        }
        merge_by_leaf(&mut rows);
        rows
    }

    /// Exact where nothing was aggregated; a superset (each brute row covered) where it was.
    fn agrees(hosts: &[Host], guests: &[Guest]) -> IncidenceStats {
        let (rows, stats) = join(hosts, guests);
        let truth = brute(hosts, guests);
        if stats.conservative_leaves + stats.long_overflows == 0 {
            assert_eq!(rows, truth, "{stats:?}");
        } else {
            for t in &truth {
                assert!(
                    rows.iter().any(|r| (r.leaf, r.a, r.b) == (t.leaf, t.a, t.b)
                        && r.lo <= t.lo
                        && r.hi >= t.hi),
                    "{t:?} {stats:?}"
                );
            }
        }
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

    /// The reviewer's dense case: 8,192 long hosts in random directions in one cell against
    /// 8,192 guests is bounded work, not 6.7e7 tests.
    #[test]
    fn dense_crossing_hosts_take_bounded_work() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let n = 8192u32;
        let mut hosts = Vec::new();
        for i in 0..n {
            let a = [next(), next(), next()];
            let b = [next(), next(), next()];
            let (a, b) = if k(a) < k(b) { (a, b) } else { (b, a) };
            hosts.push((i % 97, k(a), k(b), 1e-3));
        }
        let guests: Vec<Guest> = (0..n)
            .map(|i| (100 + i % 89, k([next(), next(), next()]), 1e-3))
            .collect();
        let (_, stats) = join(&hosts, &guests);
        let bound = u64::from(n) * (EXACT_HOSTS + LONG_HOSTS * 57) as u64;
        assert!(stats.tests <= bound, "{stats:?}");
        assert!(stats.tests < u64::from(n) * u64::from(n) / 8, "{stats:?}");
    }

    /// A group of one item per leaf is charged in the stage's resident bound.
    #[test]
    fn item_groups_are_charged_per_leaf() {
        let base = memory_need(SORT, 256, 0);
        let more = memory_need(SORT, 256, 1 << 20);
        assert!(more - base >= (1 << 20) * std::mem::size_of::<Item>() as u64);
    }
}
