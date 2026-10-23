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
//!   The segment's leaf must keep that segment and its endpoints, the point's leaf must
//!   keep the point; open edges between kept points are kept too (see `HlodRead`).
//!   Overlaps are found by a streamed multi-level grid join: each open segment is a host in
//!   the cells of its own level (cell edge ≥ its length), each open point a guest in one
//!   cell of every level that has hosts.
//!
//! Leaves are contiguous DFS intervals of the frozen tree, so a tile whose leaves are
//! `[begin, end)` must keep every recorded item whose leaf interval `[lo, hi]` leaves its
//! own. Rows are merged per (leaf, item) to the union of their intervals, which is exact:
//! some incident leaf lies outside a tile iff the union does. Nothing input-sized stays
//! resident (external sorts, one leaf of items, one group, one grid cell's hosts at a time).
use crate::{
    mesh_large::{
        attributes::FullTriangle,
        config::{self, SortSpill},
        full::CornerDepth,
        leaves::geometry::LatticePlan,
        records::{Cube, NodeKind, NodeRecord},
    },
    spill::{ExternalSorter, Record, Scratch, Sequence, SequenceWriter, SortConfig, SortedStream},
    JobError, JobErrorKind,
};

use std::cell::Cell;

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

/// One open-border segment (host) or point (guest, `a == b`) of a leaf in one cell of one
/// grid level. Hosts sort before guests within a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Probe {
    level: u8,
    cell: [i32; 3],
    guest: bool,
    leaf: u32,
    a: Key,
    b: Key,
    tol: u32,
}

impl Record for Probe {
    const SIZE: usize = 46;
    fn encode(&self, out: &mut [u8]) {
        out[0] = self.level;
        for (i, c) in self.cell.iter().enumerate() {
            out[1 + 4 * i..5 + 4 * i].copy_from_slice(&c.to_le_bytes());
        }
        out[13] = u8::from(self.guest);
        out[14..18].copy_from_slice(&self.leaf.to_le_bytes());
        put(out, 18, self.a);
        put(out, 30, self.b);
        out[42..46].copy_from_slice(&self.tol.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            level: bytes[0],
            cell: std::array::from_fn(|i| u32_at(bytes, 1 + 4 * i) as i32),
            guest: bytes[13] != 0,
            leaf: u32_at(bytes, 14),
            a: get(bytes, 18),
            b: get(bytes, 30),
            tol: u32_at(bytes, 42),
        }
    }
}

/// An open-border candidate written during the reduce, expanded into probes once the host
/// levels are known.
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

/// Deepest grid level (cells of `side / 2^30`).
const MAX_LEVEL: u8 = 30;
/// Declared scratch allowance of the overlap join per source triangle (see
/// `overlap_allowance`); a run that needs more is refused (`resource_limit`).
const OVERLAP_BYTES_PER_TRIANGLE: u64 = 256;
const OVERLAP_FLOOR_BYTES: u64 = 64 << 20;

/// Worst-case spill of the exact pass plus the overlap allowance, for `triangles` source
/// triangles: every triangle contributes at most three point and three segment items, and
/// every item at most one row. Row sorts fill in bursts per group, so they are charged on
/// top of the whole item stream.
pub(crate) fn scratch_need(triangles: u64, sorter: SortConfig) -> Option<u64> {
    let items = triangles.checked_mul(6)?;
    let item: SortSpill = config::sort_spill::<Item>(items, sorter)?;
    let rows: SortSpill = config::sort_spill::<InterfaceRow>(items, sorter)?;
    let overlap = overlap_allowance(triangles);
    let reduce = item
        .bytes
        .checked_add(item.stream_slack)?
        .checked_add(rows.push_peak)?
        .checked_add(overlap)?;
    let walk = rows
        .finish_peak
        .checked_add(rows.stream_slack)?
        .checked_add(items.checked_mul(Candidate::SIZE as u64)?)?
        .checked_add(overlap)?;
    Some(item.finish_peak.max(reduce).max(walk))
}

/// Resident bound of the pass: two sorters live at once (items or probes streamed into
/// rows), one leaf's deduplicated items, one item group (at most one item per leaf, in a
/// vector plus its reallocation transient (at most three times that)), one grid cell's hosts (twice: held and a dense cell's scan
/// chunk) and the sequences a dense cell's split keeps open.
pub(crate) fn memory_need(sorter: SortConfig, leaf_triangles: u64, leaves: u64) -> u64 {
    let item = std::mem::size_of::<Item>() as u64;
    sorter
        .memory_bound::<Probe>()
        .max(sorter.memory_bound::<Item>())
        .saturating_add(sorter.memory_bound::<InterfaceRow>())
        .saturating_add(leaf_triangles.saturating_mul(12 * item))
        .saturating_add(leaves.saturating_mul(3 * item))
        .saturating_add(2 * (CELL_HOSTS as u64) * std::mem::size_of::<Probe>() as u64)
        .saturating_add(SPLIT_STREAMS.saturating_mul(sorter.buffer_bytes as u64))
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
    pub probes: u64,
    /// Grid cells denser than `CELL_HOSTS` hosts: spilled, split and scanned in chunks.
    pub dense_cells: u64,
    pub dense_splits: u64,
    pub dense_chunks: u64,
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

/// Grid level of a host: the deepest whose cell edge covers the segment and its tolerance,
/// so the host meets at most two cells per axis.
fn host_level(cube: Cube, a: Key, b: Key, tol: f64) -> u8 {
    let (a, b) = (point(a), point(b));
    let extent = (0..3).map(|i| (b[i] - a[i]).abs()).fold(0., f64::max) + 2. * tol;
    let mut level = 0u8;
    while level < MAX_LEVEL && cube.side / f64::from(1u32 << (level + 1)) >= extent {
        level += 1;
    }
    level
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
    cube: Cube,
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
    let mut guest_tol = 0f32;
    let mut group: Vec<Item> = Vec::new();
    let leaves = u64::from(leaf);
    let mut flush = |group: &mut Vec<Item>,
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
                guest_tol = guest_tol.max(f32::from_bits(g.tol));
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
    // The rounding floor of any point in (or near) the cube.
    let magnitude = (0..3)
        .map(|i| cube.origin[i].abs().max((cube.origin[i] + cube.side).abs()))
        .fold(0., f64::max);
    let join = Join {
        cube,
        guest_tol: f64::from(guest_tol),
        floor: magnitude * f64::from(f32::EPSILON) * 8.,
    };
    overlaps(
        scratch,
        sorter,
        join,
        triangles.len(),
        &hosts,
        &guests,
        &mut rows,
        &mut stats,
        check,
    )?;
    Ok((rows.finish()?, stats))
}

fn cell(cube: Cube, level: u8, p: [f64; 3]) -> [i32; 3] {
    let size = cube.side / f64::from(1u32 << level);
    std::array::from_fn(|i| {
        let c = ((p[i] - cube.origin[i]) / size).floor();
        c.clamp(-1., f64::from(i32::MAX)) as i32
    })
}

/// The overlap join's index geometry: every host is padded by the largest tolerance any
/// pair may use (its own or any guest's, plus a rounding floor for the whole cube), so a
/// guest within a pair's tolerance of a host always lies in one of the host's cells.
#[derive(Clone, Copy)]
struct Join {
    cube: Cube,
    guest_tol: f64,
    floor: f64,
}

impl Join {
    fn pad(&self, h: &Candidate) -> f64 {
        // The predicate allows independent axial and perpendicular offsets <=tol.
        // Their combined displacement is <=sqrt(2)*tol; twice the radius covers it.
        2. * (f64::from(f32::from_bits(h.tol)).max(self.guest_tol)
            + rounding(&[h.a, h.b]).max(self.floor))
    }
    /// The host's level and its cell range there (at most two cells per axis).
    fn cells(&self, h: &Candidate) -> Result<(u8, [i32; 3], [i32; 3]), JobError> {
        let pad = self.pad(h);
        let level = host_level(self.cube, h.a, h.b, pad);
        let (a, b) = (point(h.a), point(h.b));
        let lo = cell(
            self.cube,
            level,
            std::array::from_fn(|i| a[i].min(b[i]) - pad),
        );
        let hi = cell(
            self.cube,
            level,
            std::array::from_fn(|i| a[i].max(b[i]) + pad),
        );
        if level > 0 && (0..3).any(|i| hi[i] - lo[i] > 1) {
            return Err(state("an overlap host meets more than two cells per axis"));
        }
        Ok((level, lo, hi))
    }
}

/// Hosts of one cell held in memory; a denser cell is spilled and split (below).
pub(crate) const CELL_HOSTS: usize = 1 << 12;
/// Octant splits of a spilled dense cell before its chunked scan.
const SPLIT_DEPTH: u32 = 4;
/// Sequences open at once while a dense cell is split: a host and a guest stream per level
/// being read, and the pair being written.
const SPLIT_STREAMS: u64 = 2 * (SPLIT_DEPTH as u64 + 1) + 2;

/// The declared scratch allowance of the overlap join (probe sort and dense-cell spills):
/// 256 bytes per source triangle, at least 64 MiB. A run past it is `resource_limit`.
fn overlap_allowance(triangles: u64) -> u64 {
    triangles
        .saturating_mul(OVERLAP_BYTES_PER_TRIANGLE)
        .max(OVERLAP_FLOOR_BYTES)
}

/// Shared prospective charge for every live join stream. Exact-item files have
/// been consumed, so their admitted space is reusable. Probe generations stay
/// conservatively charged in full while row bursts and dense spills grow.
struct JoinCharge {
    limit: u64,
    candidates: u64,
    probe_sort: u64,
    sorter: SortConfig,
    rows: Cell<u64>,
    live: Cell<u64>,
}
impl JoinCharge {
    fn check(&self, rows: u64, live: u64) -> Result<(), JobError> {
        let row_sort = config::sort_spill::<InterfaceRow>(rows, self.sorter)
            .and_then(|s| s.finish_peak.checked_add(s.stream_slack))
            .ok_or_else(|| {
                JobError::new(JobErrorKind::ResourceLimit, "interface row spill overflow")
            })?;
        let need = self
            .candidates
            .saturating_add(self.probe_sort)
            .saturating_add(row_sort)
            .saturating_add(live);
        if need > self.limit {
            return Err(JobError::new(
                JobErrorKind::ResourceLimit,
                format!(
                    "overlap join needs {need} scratch bytes, above its admitted {}",
                    self.limit
                ),
            ));
        }
        Ok(())
    }
}

/// The grid join of open points (guests) against open segments of other leaves (hosts):
/// streamed in sorted cells, each cell's hosts in memory up to [`CELL_HOSTS`]; a denser
/// cell is spilled, split into octants to a bounded depth and finished by a chunked scan,
/// so no input is refused for its density and memory stays bounded.
#[allow(clippy::too_many_arguments)]
fn overlaps<'s>(
    scratch: &'s Scratch,
    sorter: SortConfig,
    join: Join,
    triangles: u64,
    hosts: &Sequence<Candidate>,
    guests: &Sequence<Candidate>,
    rows: &mut ExternalSorter<'s, InterfaceRow>,
    stats: &mut IncidenceStats,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(), JobError> {
    if hosts.is_empty() || guests.is_empty() || join.cube.side <= 0. {
        return Ok(());
    }
    // Exact probe count and host levels before anything is written.
    let mut host_levels = 0u32;
    let mut probes_count = 0u64;
    for (i, host) in hosts.reader()?.enumerate() {
        if i.is_multiple_of(1024) {
            check()?;
        }
        let (level, lo, hi) = join.cells(&host?)?;
        host_levels |= 1 << level;
        probes_count += (0..3).map(|i| (hi[i] - lo[i] + 1) as u64).product::<u64>();
    }
    let levels = u64::from(host_levels.count_ones());
    probes_count += guests.len() * levels;
    let sort = config::sort_spill::<Probe>(probes_count, sorter)
        .map(|s| s.finish_peak.saturating_add(s.stream_slack))
        .ok_or_else(|| state("overlap join arithmetic overflow"))?;
    let allowance = overlap_allowance(triangles);
    if sort > allowance {
        return Err(JobError::new(
            JobErrorKind::ResourceLimit,
            format!(
                "the overlap join needs {sort} bytes of scratch for {probes_count} probes, past \
                 its declared {allowance}"
            ),
        ));
    }
    crate::spill::preflight(
        scratch.path(),
        allowance,
        crate::spill::SpaceRole::Scratch,
        crate::spill::PreflightStage::Lod,
    )?;
    let charge = JoinCharge {
        limit: scratch_need(triangles, sorter).ok_or_else(|| {
            JobError::new(
                JobErrorKind::ResourceLimit,
                "interface stage spill overflow",
            )
        })?,
        candidates: hosts
            .len()
            .saturating_add(guests.len())
            .saturating_mul(Candidate::SIZE as u64),
        probe_sort: sort,
        sorter,
        rows: Cell::new(stats.rows),
        live: Cell::new(0),
    };
    charge.check(stats.rows, 0)?;
    let mut probes = ExternalSorter::<Probe>::new(scratch, sorter)?;
    for (i, host) in hosts.reader()?.enumerate() {
        if i.is_multiple_of(1024) {
            check()?;
        }
        let h = host?;
        let (level, lo, hi) = join.cells(&h)?;
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    probes.push(Probe {
                        level,
                        cell: [x, y, z],
                        guest: false,
                        leaf: h.leaf,
                        a: h.a,
                        b: h.b,
                        tol: h.tol,
                    })?;
                    stats.probes += 1;
                }
            }
        }
    }
    for (i, guest) in guests.reader()?.enumerate() {
        if i.is_multiple_of(1024) {
            check()?;
        }
        let g = guest?;
        for level in (0..=MAX_LEVEL).filter(|l| host_levels & (1 << l) != 0) {
            probes.push(Probe {
                level,
                cell: cell(join.cube, level, point(g.a)),
                guest: true,
                leaf: g.leaf,
                a: g.a,
                b: g.b,
                tol: g.tol,
            })?;
            stats.probes += 1;
        }
    }
    let mut dense = Dense {
        scratch,
        sorter,
        join,
        budget: allowance - sort,
        charge: &charge,
        check,
    };
    let mut pairs = 0u64;
    let mut emit = |h: &Probe, g: &Probe, stats: &mut IncidenceStats| -> Result<(), JobError> {
        pairs += 1;
        if pairs.is_multiple_of(256) {
            check()?;
        }
        if h.leaf == g.leaf {
            return Ok(());
        }
        let tol = f64::from(f32::from_bits(h.tol).max(f32::from_bits(g.tol)))
            + rounding(&[h.a, h.b, g.a]);
        if !on_segment(h.a, h.b, g.a, tol) {
            return Ok(());
        }
        charge.check(stats.rows.saturating_add(4), charge.live.get())?;
        stats.overlaps += 1;
        let (lo, hi) = (h.leaf.min(g.leaf), h.leaf.max(g.leaf));
        for (leaf, a, b) in [
            (h.leaf, h.a, h.b),
            (h.leaf, h.a, h.a),
            (h.leaf, h.b, h.b),
            (g.leaf, g.a, g.a),
        ] {
            rows.push(InterfaceRow { leaf, a, b, lo, hi })?;
            stats.rows += 1;
        }
        charge.rows.set(stats.rows);
        Ok(())
    };
    let mut cell_hosts: Vec<Probe> = Vec::with_capacity(CELL_HOSTS);
    // A dense cell's hosts and guests, spilled once its hosts pass `CELL_HOSTS`.
    let mut spilled: Option<(SequenceWriter<Probe>, SequenceWriter<Probe>)> = None;
    let mut current: Option<(u8, [i32; 3])> = None;
    let mut seen = 0u64;
    let finish_dense =
        |spilled: &mut Option<(SequenceWriter<Probe>, SequenceWriter<Probe>)>,
         current: Option<(u8, [i32; 3])>,
         dense: &mut Dense<'_, 's>,
         emit: &mut dyn FnMut(&Probe, &Probe, &mut IncidenceStats) -> Result<(), JobError>,
         stats: &mut IncidenceStats|
         -> Result<(), JobError> {
            let Some((h, g)) = spilled.take() else {
                return Ok(());
            };
            let (level, c) = current.ok_or_else(|| state("a dense overlap cell without a key"))?;
            let (h, g) = (h.finish()?, g.finish()?);
            stats.dense_cells += 1;
            let size = join.cube.side / f64::from(1u32 << level);
            let lo: [f64; 3] =
                std::array::from_fn(|i| join.cube.origin[i] + f64::from(c[i]) * size);
            let region = Region {
                lo,
                hi: std::array::from_fn(|i| lo[i] + size),
                cut_lo: [false; 3],
                cut_hi: [false; 3],
            };
            dense.join_spilled(&h, &g, region, 0, emit, stats)?;
            dense.release(dense.bytes(&h) + dense.bytes(&g));
            Ok(())
        };
    for probe in probes.finish()? {
        let p = probe?;
        seen += 1;
        if seen.is_multiple_of(1 << 16) {
            check()?;
        }
        if current != Some((p.level, p.cell)) {
            finish_dense(&mut spilled, current, &mut dense, &mut emit, stats)?;
            cell_hosts.clear();
            current = Some((p.level, p.cell));
        }
        match (&mut spilled, p.guest) {
            (Some((h, _)), false) => dense.push(h, &p)?,
            (Some((_, g)), true) => dense.push(g, &p)?,
            (None, false) if cell_hosts.len() == CELL_HOSTS => {
                let mut h =
                    SequenceWriter::<Probe>::new(scratch, "hlod-dense-hosts", sorter.buffer_bytes)?;
                let g = SequenceWriter::<Probe>::new(
                    scratch,
                    "hlod-dense-guests",
                    sorter.buffer_bytes,
                )?;
                for q in cell_hosts.drain(..) {
                    dense.push(&mut h, &q)?;
                }
                dense.push(&mut h, &p)?;
                spilled = Some((h, g));
            }
            (None, false) => cell_hosts.push(p),
            (None, true) => {
                for h in &cell_hosts {
                    emit(h, &p, stats)?;
                }
            }
        }
    }
    finish_dense(&mut spilled, current, &mut dense, &mut emit, stats)?;
    Ok(())
}

/// A half-open split region of a dense cell: faces never cut extend without bound, so every
/// guest the cell holds falls in exactly one octant whatever its rounding.
#[derive(Clone, Copy)]
struct Region {
    lo: [f64; 3],
    hi: [f64; 3],
    cut_lo: [bool; 3],
    cut_hi: [bool; 3],
}

impl Region {
    fn octant(&self, o: usize) -> Self {
        let mut r = *self;
        for i in 0..3 {
            let mid = (self.lo[i] + self.hi[i]) / 2.;
            if (o >> i) & 1 == 0 {
                r.hi[i] = mid;
                r.cut_hi[i] = true;
            } else {
                r.lo[i] = mid;
                r.cut_lo[i] = true;
            }
        }
        r
    }
    fn holds(&self, p: [f64; 3]) -> bool {
        (0..3).all(|i| {
            (!self.cut_lo[i] || p[i] >= self.lo[i]) && (!self.cut_hi[i] || p[i] < self.hi[i])
        })
    }
    /// Whether segment `ab` meets the region grown by `pad` (slab clipping).
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

/// Spills of dense cells, within what the allowance leaves after the probe sort.
struct Dense<'c, 's> {
    scratch: &'s Scratch,
    sorter: SortConfig,
    join: Join,
    budget: u64,
    charge: &'c JoinCharge,
    check: &'c (dyn Fn() -> Result<(), JobError> + Sync),
}

impl<'s> Dense<'_, 's> {
    fn bytes(&self, s: &Sequence<Probe>) -> u64 {
        s.len() * Probe::SIZE as u64
    }
    fn push(&mut self, w: &mut SequenceWriter<Probe>, p: &Probe) -> Result<(), JobError> {
        let live = self.charge.live.get().saturating_add(Probe::SIZE as u64);
        if live > self.budget {
            return Err(JobError::new(
                JobErrorKind::ResourceLimit,
                format!(
                    "dense overlap cells need more than the {} bytes of scratch the join's \
                     allowance leaves",
                    self.budget
                ),
            ));
        }
        self.charge.check(self.charge.rows.get(), live)?;
        self.charge.live.set(live);
        w.push(p)?;
        Ok(())
    }
    fn release(&self, bytes: u64) {
        self.charge.live.set(self.charge.live.get() - bytes);
    }
    fn writer(&self, stem: &str) -> Result<SequenceWriter<Probe>, JobError> {
        Ok(SequenceWriter::<Probe>::new(
            self.scratch,
            stem,
            self.sorter.buffer_bytes,
        )?)
    }

    /// Join a spilled dense cell: in memory once its hosts fit, else split into octants
    /// (each written, joined and released in turn) to `SPLIT_DEPTH`, then a chunked scan.
    #[allow(clippy::too_many_arguments)]
    fn join_spilled(
        &mut self,
        hosts: &Sequence<Probe>,
        guests: &Sequence<Probe>,
        region: Region,
        depth: u32,
        emit: &mut dyn FnMut(&Probe, &Probe, &mut IncidenceStats) -> Result<(), JobError>,
        stats: &mut IncidenceStats,
    ) -> Result<(), JobError> {
        if hosts.is_empty() || guests.is_empty() {
            return Ok(());
        }
        if hosts.len() as usize > CELL_HOSTS && depth < SPLIT_DEPTH {
            for o in 0..8 {
                (self.check)()?;
                let octant = region.octant(o);
                let mut h = self.writer("hlod-dense-hosts")?;
                for (i, q) in hosts.reader()?.enumerate() {
                    if i.is_multiple_of(256) {
                        (self.check)()?;
                    }
                    let q = q?;
                    let pad = self.join.pad(&Candidate {
                        leaf: q.leaf,
                        a: q.a,
                        b: q.b,
                        tol: q.tol,
                    }) * (1. + 1e-9);
                    if octant.meets(point(q.a), point(q.b), pad) {
                        self.push(&mut h, &q)?;
                    }
                }
                let mut g = self.writer("hlod-dense-guests")?;
                for (i, q) in guests.reader()?.enumerate() {
                    if i.is_multiple_of(256) {
                        (self.check)()?;
                    }
                    let q = q?;
                    if octant.holds(point(q.a)) {
                        self.push(&mut g, &q)?;
                    }
                }
                let (h, g) = (h.finish()?, g.finish()?);
                stats.dense_splits += 1;
                self.join_spilled(&h, &g, octant, depth + 1, emit, stats)?;
                self.release(self.bytes(&h) + self.bytes(&g));
            }
            return Ok(());
        }
        // In memory, or a chunked scan of the hosts against every guest.
        let mut chunk: Vec<Probe> = Vec::with_capacity(CELL_HOSTS);
        let mut reader = hosts.reader()?;
        loop {
            chunk.clear();
            for q in reader.by_ref().take(CELL_HOSTS) {
                chunk.push(q?);
            }
            if chunk.is_empty() {
                return Ok(());
            }
            stats.dense_chunks += 1;
            for (k, g) in guests.reader()?.enumerate() {
                if k.is_multiple_of(1 << 12) {
                    (self.check)()?;
                }
                let g = g?;
                for h in &chunk {
                    emit(h, &g, stats)?;
                }
            }
        }
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

    /// Run the streamed join on `hosts` and `guests` (leaf, a, b, tolerance) in `cube`; the
    /// overlap rows and stats.
    fn join(
        cube: Cube,
        hosts: &[(u32, Key, Key, f32)],
        guests: &[(u32, Key, f32)],
    ) -> (Vec<InterfaceRow>, IncidenceStats) {
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
        let mut guest_tol = 0f32;
        for &(leaf, a, tol) in guests {
            guest_tol = guest_tol.max(tol);
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
        let magnitude = (0..3)
            .map(|i| cube.origin[i].abs().max((cube.origin[i] + cube.side).abs()))
            .fold(0., f64::max);
        let joined = Join {
            cube,
            guest_tol: f64::from(guest_tol),
            floor: magnitude * f64::from(f32::EPSILON) * 8.,
        };
        overlaps(
            &scratch,
            SORT,
            joined,
            1,
            &h,
            &g,
            &mut rows,
            &mut stats,
            &|| Ok(()),
        )
        .unwrap();
        let rows = rows.finish().unwrap().map(Result::unwrap).collect();
        (rows, stats)
    }

    /// The round-2 probe: the host is indexed by its own small tolerance, but the pair
    /// predicate accepts the guest's larger one. Index padding covers any guest's.
    #[test]
    fn a_guest_with_a_larger_tolerance_meets_its_host() {
        let unit = Cube {
            origin: [0.; 3],
            side: 1.,
        };
        let x = 0.5 - 2f64.powi(-12);
        let host = (
            0,
            k([x, 0.125, 0.125]),
            k([x, 0.125, 0.375]),
            2f32.powi(-13),
        );
        let guest = (1, k([0.5, 0.125, 0.25]), 2f32.powi(-10));
        let (rows, stats) = join(unit, &[host], &[guest]);
        assert_eq!(stats.overlaps, 1, "{stats:?}");
        assert!(rows
            .iter()
            .any(|r| r.leaf == 1 && r.a == guest.1 && r.b == guest.1));
        assert!(rows
            .iter()
            .any(|r| r.leaf == 0 && (r.a, r.b) == (host.1, host.2)));
    }

    /// A cell far denser than `CELL_HOSTS` is spilled, split and scanned in chunks, and finds
    /// exactly the brute-force matches.
    #[test]
    fn dense_cells_are_joined_exactly_in_bounded_memory() {
        let cube = Cube {
            origin: [0.; 3],
            side: 16.,
        };
        // Parallel long hosts in a slab (one scale, a few cells), guests on every 7th.
        let n = 3 * CELL_HOSTS;
        let mut hosts = Vec::new();
        let mut guests = Vec::new();
        for i in 0..n {
            let (x, y) = (
                1. + (i % 64) as f64 * 0.03125,
                1. + (i / 64) as f64 * 0.0625,
            );
            hosts.push((i as u32, k([x, y, 1.]), k([x, y, 7.]), 1e-4));
            if i % 7 == 0 {
                guests.push((n as u32 + i as u32, k([x, y, 1. + (i % 5) as f64]), 1e-4));
            }
        }
        // And a guest off every host.
        guests.push((u32::MAX, k([0.5, 0.5, 0.5]), 1e-4));
        let (_, stats) = join(cube, &hosts, &guests);
        let brute = guests
            .iter()
            .map(|g| {
                hosts
                    .iter()
                    .filter(|h| {
                        h.0 != g.0 && on_segment(h.1, h.2, g.1, 1e-4 + rounding(&[h.1, h.2, g.1]))
                    })
                    .count() as u64
            })
            .sum::<u64>();
        assert!(
            stats.dense_cells >= 1 && stats.dense_splits >= 8,
            "{stats:?}"
        );
        assert_eq!(stats.overlaps, brute, "{stats:?}");
        assert!(brute > 0);
    }

    #[test]
    fn dense_partition_cancels_even_when_every_candidate_is_pruned() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Entry plus ceil((CELL_HOSTS + 1) / 256) host checks precede the
        // guest scan. Interrupt each scan before its first partition completes.
        let host_checks = (CELL_HOSTS + 1).div_ceil(256);
        for stop in [3, 1 + host_checks + 2] {
            let dir = tempfile::tempdir().unwrap();
            let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
            let mut h = SequenceWriter::<Probe>::new(&scratch, "hosts", 4096).unwrap();
            let mut g = SequenceWriter::<Probe>::new(&scratch, "guests", 4096).unwrap();
            for _ in 0..=CELL_HOSTS {
                let p = Probe {
                    level: 0,
                    cell: [0; 3],
                    guest: false,
                    leaf: 0,
                    a: k([10., 10., 10.]),
                    b: k([11., 10., 10.]),
                    tol: 0f32.to_bits(),
                };
                h.push(&p).unwrap();
                g.push(&p).unwrap();
            }
            let (h, g) = (h.finish().unwrap(), g.finish().unwrap());
            let charge = JoinCharge {
                limit: 64 << 20,
                candidates: 0,
                probe_sort: 0,
                sorter: SORT,
                rows: Cell::new(0),
                live: Cell::new((h.len() + g.len()) * Probe::SIZE as u64),
            };
            let initial_live = charge.live.get();
            let calls = AtomicUsize::new(0);
            let check = || {
                if calls.fetch_add(1, Ordering::Relaxed) + 1 == stop {
                    Err(JobError::new(JobErrorKind::Cancelled, "partition control"))
                } else {
                    Ok(())
                }
            };
            let mut dense = Dense {
                scratch: &scratch,
                sorter: SORT,
                join: Join {
                    cube: Cube {
                        origin: [0.; 3],
                        side: 1.,
                    },
                    guest_tol: 0.,
                    floor: 0.,
                },
                budget: 64 << 20,
                charge: &charge,
                check: &check,
            };
            let mut stats = IncidenceStats::default();
            let error = dense
                .join_spilled(
                    &h,
                    &g,
                    Region {
                        lo: [0.; 3],
                        hi: [1.; 3],
                        cut_lo: [true; 3],
                        cut_hi: [true; 3],
                    },
                    0,
                    &mut |_, _, _| panic!("all candidates must be pruned"),
                    &mut stats,
                )
                .unwrap_err();
            assert_eq!(error.kind(), JobErrorKind::Cancelled);
            assert_eq!(stats.dense_splits, 0, "scan completed before cancellation");
            assert_eq!(charge.live.get(), initial_live);
        }
    }

    /// A group of one item per leaf is charged in the stage's resident bound.
    #[test]
    fn item_groups_are_charged_per_leaf() {
        let base = memory_need(SORT, 256, 0);
        let more = memory_need(SORT, 256, 1 << 20);
        assert!(more - base >= (1 << 20) * std::mem::size_of::<Item>() as u64);
    }
    #[test]
    fn diagonal_endpoint_offset_is_inside_the_candidate_box() {
        let cube = Cube {
            origin: [0.; 3],
            side: 16.,
        };
        let host = (0, k([2.3125, 3., 0.]), k([3.8125, 4.5, 0.]), 1f32 / 16.);
        let guest = (1, k([4.5, 4.5, 0.]), 0.5f32);
        assert!(on_segment(host.1, host.2, guest.1, 0.5));
        let (rows, stats) = join(cube, &[host], &[guest]);
        assert_eq!(stats.overlaps, 1);
        assert_eq!(rows.len(), 4);
    }
    #[test]
    fn row_spill_refuses_before_the_admitted_prefix_is_exceeded() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 4096).unwrap();
        scratch.set_chunk_bytes(64);
        let sorter = SortConfig {
            run_bytes: 256,
            fan_in: 2,
            buffer_bytes: 16,
            jobs: 1,
        };
        let charge = JoinCharge {
            limit: 4096,
            candidates: 128,
            probe_sort: 256,
            sorter,
            rows: Cell::new(0),
            live: Cell::new(0),
        };
        let mut rows = ExternalSorter::new(&scratch, sorter).unwrap();
        let mut count = 0;
        loop {
            if let Err(error) = charge.check(count + 1, 0) {
                assert_eq!(error.kind(), JobErrorKind::ResourceLimit);
                break;
            }
            rows.push(InterfaceRow {
                leaf: count as u32,
                a: [0; 3],
                b: [0; 3],
                lo: 0,
                hi: 1,
            })
            .unwrap();
            count += 1;
            charge.rows.set(count);
        }
        assert!(count > 0 && count < 1000);
        assert!(scratch.peak_bytes() <= 4096);
        assert!(charge.check(count, 4096).is_err());
    }
}
