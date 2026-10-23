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
//!   the cells of its own level (cell edge ≥ its length), each open point a guest in the bounded cells of its tolerance box at every host level.
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
/// Declared scratch allowance of the overlap join per source triangle; a run that needs
/// more is refused (`resource_limit`) before the join is written.
const OVERLAP_BYTES_PER_TRIANGLE: u64 = 256;
/// A highly shared exact item is refused before its fixed, charged buffer can grow.
const MAX_INCIDENT_LEAVES: usize = 1 << 16;
/// Most hosts one grid cell may hold at once; more is `unsupported`. A host's cell is under
/// twice its extent, so this many same-scale open segments in one cell is a pathological
/// heap, and the cap bounds both the cell's memory and each guest's tests.
pub(crate) const MAX_CELL_HOSTS: usize = 1 << 12;

/// Worst-case spill of the exact pass plus the overlap allowance, for `triangles` source
/// triangles: every triangle contributes at most three point and three segment items, and
/// every item at most one row. Row sorts fill in bursts per group, so they are charged on
/// top of the whole item stream.
pub(crate) fn scratch_need(triangles: u64, sorter: SortConfig) -> Option<u64> {
    let items = triangles.checked_mul(6)?;
    let item: SortSpill = config::sort_spill::<Item>(items, sorter)?;
    let rows: SortSpill = config::sort_spill::<InterfaceRow>(items, sorter)?;
    let overlap = triangles.checked_mul(OVERLAP_BYTES_PER_TRIANGLE)?;
    let reduce = item
        .bytes
        .checked_add(item.stream_slack)?
        .checked_add(rows.push_peak)?
        .checked_add(overlap)?;
    let walk = rows
        .finish_peak
        .checked_add(rows.stream_slack)?
        .checked_add(overlap)?;
    Some(item.finish_peak.max(reduce).max(walk))
}

/// Resident bound of the pass: two sorters live at once (items or probes streamed into
/// rows), one leaf's deduplicated items, and one grid cell's hosts.
pub(crate) fn memory_need(sorter: SortConfig, leaf_triangles: u64) -> u64 {
    sorter
        .memory_bound::<Probe>()
        .max(sorter.memory_bound::<Item>())
        .saturating_add(sorter.memory_bound::<InterfaceRow>())
        .saturating_add(leaf_triangles.saturating_mul(12 * std::mem::size_of::<Item>() as u64))
        .saturating_add((MAX_INCIDENT_LEAVES * std::mem::size_of::<Item>()) as u64)
        .saturating_add((MAX_CELL_HOSTS as u64).saturating_mul(std::mem::size_of::<Probe>() as u64))
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
    let mut group: Vec<Item> = Vec::with_capacity(MAX_INCIDENT_LEAVES);
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
        // Items are unique per leaf and sorted, so the group's leaves ascend strictly.
        push_group(&mut group, item)?;
    }
    if !group.is_empty() {
        flush(&mut group, &mut rows, &mut hosts, &mut guests, &mut stats)?;
    }
    let hosts = hosts.finish()?;
    let guests = guests.finish()?;
    overlaps(
        scratch,
        sorter,
        cube,
        &hosts,
        &guests,
        triangles.len(),
        &mut rows,
        &mut stats,
        check,
    )?;
    Ok((rows.finish()?, stats))
}

fn push_group(group: &mut Vec<Item>, item: Item) -> Result<(), JobError> {
    if group.len() == MAX_INCIDENT_LEAVES {
        return Err(JobError::new(
            JobErrorKind::ResourceLimit,
            format!("exact interface item exceeds {MAX_INCIDENT_LEAVES} incident leaves"),
        ));
    }
    group.push(item);
    Ok(())
}

/// The allowance pays for retained candidates, all probe spill/merge generations,
/// and row-sort growth above the already admitted six rows per source triangle.
/// Check the prospective record count BEFORE pushing, including compaction jumps.
struct JoinBudget {
    limit: u64,
    candidates: u64,
    base_rows: u64,
    sorter: SortConfig,
}
impl JoinBudget {
    fn new(n: u64, candidates: u64, sorter: SortConfig) -> Result<Self, JobError> {
        Ok(Self {
            limit: n.saturating_mul(OVERLAP_BYTES_PER_TRIANGLE),
            candidates: candidates.saturating_mul(Candidate::SIZE as u64),
            base_rows: Self::spill::<InterfaceRow>(n.saturating_mul(6), sorter)?,
            sorter,
        })
    }
    fn spill<T: Record>(n: u64, sorter: SortConfig) -> Result<u64, JobError> {
        config::sort_spill::<T>(n, sorter)
            .and_then(|s| s.finish_peak.checked_add(s.stream_slack))
            .ok_or_else(|| {
                JobError::new(
                    JobErrorKind::ResourceLimit,
                    "overlap spill arithmetic overflow",
                )
            })
    }
    fn check(&self, probes: u64, rows: u64) -> Result<(), JobError> {
        let extra_rows =
            Self::spill::<InterfaceRow>(rows, self.sorter)?.saturating_sub(self.base_rows);
        let need = self
            .candidates
            .saturating_add(Self::spill::<Probe>(probes, self.sorter)?)
            .saturating_add(extra_rows);
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

fn cell(cube: Cube, level: u8, p: [f64; 3]) -> [i32; 3] {
    let size = cube.side / f64::from(1u32 << level);
    std::array::from_fn(|i| {
        let c = ((p[i] - cube.origin[i]) / size).floor();
        c.clamp(-1., f64::from(i32::MAX)) as i32
    })
}

/// The grid join of open points (guests) against open segments of other leaves (hosts).
#[allow(clippy::too_many_arguments)]
fn overlaps<'s>(
    scratch: &'s Scratch,
    sorter: SortConfig,
    cube: Cube,
    hosts: &Sequence<Candidate>,
    guests: &Sequence<Candidate>,
    triangles: u64,
    rows: &mut ExternalSorter<'s, InterfaceRow>,
    stats: &mut IncidenceStats,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(), JobError> {
    if hosts.is_empty() || cube.side <= 0. {
        return Ok(());
    }
    let budget = JoinBudget::new(triangles, hosts.len().saturating_add(guests.len()), sorter)?;
    budget.check(0, stats.rows)?;
    // Pay every pair's floating tolerance in the host box. This streamed maximum
    // also covers canonical vertices just outside the source cube after quantization.
    let mut rounding_floor = 0f64;
    for (i, candidate) in hosts.reader()?.chain(guests.reader()?).enumerate() {
        if i.is_multiple_of(1024) {
            check()?;
        }
        let c = candidate?;
        rounding_floor = rounding_floor.max(rounding(&[c.a, c.b]));
    }
    let mut host_levels = 0u32;
    let mut minimum_host_tol = [f64::INFINITY; 31];
    crate::spill::preflight(
        scratch.path(),
        budget.limit,
        crate::spill::SpaceRole::Scratch,
        crate::spill::PreflightStage::Lod,
    )?;
    let mut probes = ExternalSorter::<Probe>::new(scratch, sorter)?;
    for (i, host) in hosts.reader()?.enumerate() {
        if i.is_multiple_of(1024) {
            check()?;
        }
        let h = host?;
        let snap = f64::from(f32::from_bits(h.tol));
        // on_segment permits both axial and perpendicular offsets <= tolerance.
        // Their combined displacement is <=sqrt(2)*tolerance; 2 is conservative.
        let tol = 2. * (snap + rounding_floor);
        let level = host_level(cube, h.a, h.b, tol);
        host_levels |= 1 << level;
        minimum_host_tol[level as usize] = minimum_host_tol[level as usize].min(snap);
        let (a, b) = (point(h.a), point(h.b));
        let lo = cell(cube, level, std::array::from_fn(|i| a[i].min(b[i]) - tol));
        let hi = cell(cube, level, std::array::from_fn(|i| a[i].max(b[i]) + tol));
        if level > 0 && (0..3).any(|i| hi[i] - lo[i] > 1) {
            return Err(state("an overlap host meets more than two cells per axis"));
        }
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    if stats.probes.is_multiple_of(1024) {
                        check()?;
                    }
                    budget.check(stats.probes.saturating_add(1), stats.rows)?;
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
            // The predicate accepts max(host_tol, guest_tol). Expanding both boxes
            // covers that radius, even across a cell boundary at mixed lattice steps.
            let tol =
                2. * (f64::from(f32::from_bits(g.tol)) - minimum_host_tol[level as usize]).max(0.);
            let p = point(g.a);
            let lo = cell(cube, level, p.map(|v| v - tol));
            let hi = cell(cube, level, p.map(|v| v + tol));
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        if stats.probes.is_multiple_of(1024) {
                            check()?;
                        }
                        budget.check(stats.probes.saturating_add(1), stats.rows)?;
                        probes.push(Probe {
                            level,
                            cell: [x, y, z],
                            guest: true,
                            leaf: g.leaf,
                            a: g.a,
                            b: g.b,
                            tol: g.tol,
                        })?;
                        stats.probes += 1;
                    }
                }
            }
        }
    }
    let mut cell_hosts: Vec<Probe> = Vec::new();
    let mut current: Option<(u8, [i32; 3])> = None;
    let mut seen = 0u64;
    for probe in probes.finish()? {
        let p = probe?;
        seen += 1;
        if seen.is_multiple_of(1 << 16) {
            check()?;
        }
        if current != Some((p.level, p.cell)) {
            cell_hosts.clear();
            current = Some((p.level, p.cell));
        }
        if !p.guest {
            if cell_hosts.len() == MAX_CELL_HOSTS {
                return Err(JobError::new(
                    JobErrorKind::Unsupported,
                    format!(
                        "more than {MAX_CELL_HOSTS} open border segments of one scale meet in \
                         one grid cell; their overlaps cannot be bounded"
                    ),
                ));
            }
            cell_hosts.push(p);
            continue;
        }
        for (i, h) in cell_hosts.iter().filter(|h| h.leaf != p.leaf).enumerate() {
            if i.is_multiple_of(256) {
                check()?;
            }
            // Expanded boxes can meet in several cells. Evaluate each pair once,
            // in the lexicographically first cell of their intersection.
            let (a, b) = (point(h.a), point(h.b));
            let host_tol = 2. * (f64::from(f32::from_bits(h.tol)) + rounding_floor);
            let guest_tol = 2.
                * (f64::from(f32::from_bits(p.tol)) - minimum_host_tol[p.level as usize]).max(0.);
            let host_lo = cell(
                cube,
                p.level,
                std::array::from_fn(|i| a[i].min(b[i]) - host_tol),
            );
            let guest_lo = cell(cube, p.level, point(p.a).map(|v| v - guest_tol));
            if p.cell != std::array::from_fn(|i| host_lo[i].max(guest_lo[i])) {
                continue;
            }
            let tol = f64::from(f32::from_bits(h.tol).max(f32::from_bits(p.tol)))
                + rounding(&[h.a, h.b, p.a]);
            if !on_segment(h.a, h.b, p.a, tol) {
                continue;
            }
            budget.check(stats.probes, stats.rows.saturating_add(4))?;
            stats.overlaps += 1;
            let (lo, hi) = (h.leaf.min(p.leaf), h.leaf.max(p.leaf));
            for (leaf, a, b) in [
                (h.leaf, h.a, h.b),
                (h.leaf, h.a, h.a),
                (h.leaf, h.b, h.b),
                (p.leaf, p.a, p.a),
            ] {
                rows.push(InterfaceRow { leaf, a, b, lo, hi })?;
                stats.rows += 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod bounded_join_tests {
    use super::*;

    fn config() -> SortConfig {
        SortConfig {
            run_bytes: 8192,
            fan_in: 2,
            buffer_bytes: 64,
            jobs: 1,
        }
    }
    fn sequence(scratch: &Scratch, stem: &str, rows: &[Candidate]) -> Sequence<Candidate> {
        let mut out = SequenceWriter::new(scratch, stem, 64).unwrap();
        for row in rows {
            out.push(row).unwrap();
        }
        out.finish().unwrap()
    }
    #[test]
    fn mixed_step_and_diagonal_endpoint_guests_are_matched_once() {
        for (cube, a, b, p, host_tol) in [
            (
                Cube {
                    origin: [0.; 3],
                    side: 32768.,
                },
                [0., 0.75, 0.],
                [0.75, 0.75, 0.],
                [0.5, 1., 0.],
                1f32 / 128.,
            ),
            (
                Cube {
                    origin: [0.; 3],
                    side: 16.,
                },
                [2.3125, 3., 0.],
                [3.8125, 4.5, 0.],
                [4.5, 4.5, 0.],
                1f32 / 16.,
            ),
        ] {
            let parent = tempfile::tempdir().unwrap();
            let scratch = Scratch::create(Some(parent.path()), 1 << 20).unwrap();
            let (a, b, p) = (key(a), key(b), key(p));
            assert!(on_segment(a, b, p, 0.5));
            let hosts = sequence(
                &scratch,
                "hosts",
                &[Candidate {
                    leaf: 0,
                    a,
                    b,
                    tol: host_tol.to_bits(),
                }],
            );
            let guests = sequence(
                &scratch,
                "guests",
                &[Candidate {
                    leaf: 1,
                    a: p,
                    b: p,
                    tol: 0.5f32.to_bits(),
                }],
            );
            let mut rows = ExternalSorter::new(&scratch, config()).unwrap();
            let mut stats = IncidenceStats::default();
            overlaps(
                &scratch,
                config(),
                cube,
                &hosts,
                &guests,
                1000,
                &mut rows,
                &mut stats,
                &|| Ok(()),
            )
            .unwrap();
            let rows: Vec<_> = rows.finish().unwrap().map(Result::unwrap).collect();
            assert_eq!(stats.overlaps, 1);
            assert_eq!(rows.len(), 4);
            assert!(rows.iter().any(|r| r.leaf == 1 && r.a == p && r.b == p));
        }
    }
    #[test]
    fn dense_overlap_refuses_before_unpaid_row_spill() {
        let parent = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(parent.path()), 1 << 20).unwrap();
        scratch.set_chunk_bytes(256);
        let a = key([0., 0.75, 0.]);
        let b = key([0.75, 0.75, 0.]);
        let p = key([0.5, 0.75, 0.]);
        let hosts: Vec<_> = (0..40)
            .map(|leaf| Candidate { leaf, a, b, tol: 0 })
            .collect();
        let guests: Vec<_> = (40..80)
            .map(|leaf| Candidate {
                leaf,
                a: p,
                b: p,
                tol: 0,
            })
            .collect();
        let hosts = sequence(&scratch, "hosts", &hosts);
        let guests = sequence(&scratch, "guests", &guests);
        let mut rows = ExternalSorter::new(&scratch, config()).unwrap();
        let mut stats = IncidenceStats::default();
        let error = overlaps(
            &scratch,
            config(),
            Cube {
                origin: [0.; 3],
                side: 32768.,
            },
            &hosts,
            &guests,
            200,
            &mut rows,
            &mut stats,
            &|| Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::ResourceLimit);
        assert!(
            stats.probes > 0 && stats.rows >= 1200 && stats.rows < 6400,
            "{stats:?}"
        );
        assert!(scratch.peak_bytes() <= scratch_need(200, config()).unwrap());
    }
    #[test]
    fn exact_incident_group_refuses_before_charged_capacity_grows() {
        let mut group = Vec::with_capacity(MAX_INCIDENT_LEAVES);
        let item = Item {
            a: [0; 3],
            b: [0; 3],
            leaf: 0,
            open: false,
            tol: 0,
        };
        for leaf in 0..MAX_INCIDENT_LEAVES {
            push_group(
                &mut group,
                Item {
                    leaf: leaf as u32,
                    ..item
                },
            )
            .unwrap();
        }
        let capacity = group.capacity();
        let error = push_group(&mut group, item).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::ResourceLimit);
        assert_eq!(group.capacity(), capacity);
        assert_eq!(group.len(), MAX_INCIDENT_LEAVES);
        assert!(memory_need(config(), 0) >= (capacity * std::mem::size_of::<Item>()) as u64);
    }
}
