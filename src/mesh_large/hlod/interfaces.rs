//! Spillable source incidence: which decoded points and segments each leaf shares with
//! other leaves, and over which interval of DFS leaf indices.
//!
//! **Exact seams only** (owner decision, D1 amended). Vertices are welded by their
//! leaf-canonical decoded position (`LatticePlan::canonical`): the f32 coordinates every leaf
//! publishes, compared bit for bit, so two source vertices that decode to one lattice point
//! are one vertex. A triangle whose corners decode onto fewer than three points carries no
//! surface: it contributes nothing (as the decoded census skips it), so no item is ever a
//! degenerate edge.
//!
//! - A point (canonical key) or segment (unordered pair of distinct endpoint keys, an edge of
//!   a surface triangle) is an interface item of a tile iff it occurs in a leaf inside the
//!   tile and in one outside it. Nothing is inferred: a segment is shared only where another
//!   leaf holds that very edge, never because both of its endpoints are shared.
//! - Leaves are contiguous DFS intervals of the frozen tree, so a tile whose leaves are
//!   `[begin, end)` must keep every recorded item whose leaf interval `[lo, hi]` leaves its
//!   own. Rows are merged per item to the union of their intervals, which is exact: some
//!   holding leaf lies outside a tile iff the union does.
//! - No tolerance matching. Source T-junctions and unwelded seams (geometry meeting without
//!   a shared vertex or edge) are not locked and may crack at mixed depths there; they are
//!   counted instead, combinatorially (`IncidenceStats::source_open_edges`, ...).
//!
//! Work: one external sort of at most six items per triangle, one streaming reduce, and one
//! external sort and merge join of open-edge endpoints: O(n log n), nothing input-sized
//! resident (one leaf of items, one item group).
use crate::{
    mesh_large::{
        attributes::FullTriangle,
        config::{self, SortSpill},
        corners::{Holders, LeafIndex},
        full::CornerDepth,
        leaves::geometry::LatticePlan,
        phases::{Step, StepProgress},
        records::{NodeKind, NodeRecord},
        tasks::{run, Ran},
    },
    spill::{
        ExternalSorter, Record, Scratch, Sequence, SequenceWriter, SlotFile, SortConfig,
        SortedStream,
    },
    JobError, JobErrorKind,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

fn state(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.to_owned())
}

/// A canonical decoded position as f32 bit patterns: the weld key.
pub(crate) type Key = [u32; 3];

pub(crate) fn key(p: [f32; 3]) -> Key {
    p.map(f32::to_bits)
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

/// One decoded point (`a == b`) or segment (`a < b`) of a leaf, with the leaf's faces on it
/// (segments; a point's is zero) and whether it lies on the leaf's open border (a segment
/// held by other than two of the leaf's faces, or such a segment's endpoint).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Item {
    a: Key,
    b: Key,
    leaf: u32,
    open: bool,
    faces: u32,
}

impl Record for Item {
    const SIZE: usize = 33;
    fn encode(&self, out: &mut [u8]) {
        put(out, 0, self.a);
        put(out, 12, self.b);
        out[24..28].copy_from_slice(&self.leaf.to_le_bytes());
        out[28] = u8::from(self.open);
        out[29..33].copy_from_slice(&self.faces.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            a: get(bytes, 0),
            b: get(bytes, 12),
            leaf: u32_at(bytes, 24),
            open: bytes[28] != 0,
            faces: u32_at(bytes, 29),
        }
    }
}

/// A canonical key alone (shared points, open-edge endpoints).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KeyRecord(pub Key);

impl Record for KeyRecord {
    const SIZE: usize = 12;
    fn encode(&self, out: &mut [u8]) {
        put(out, 0, self.0);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self(get(bytes, 0))
    }
}

/// A decoded point (`a == b`) or segment (`a < b`) shared with the DFS leaves `lo..=hi`,
/// recorded for one holding `leaf`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub(crate) struct InterfaceRow {
    pub leaf: u32,
    pub a: Key,
    pub b: Key,
    pub lo: u32,
    pub hi: u32,
}

impl InterfaceRow {
    pub(crate) fn new(leaf: u32, a: Key, b: Key, lo: u32, hi: u32) -> Self {
        Self { leaf, a, b, lo, hi }
    }
    /// Union with another row of the same item.
    pub(crate) fn absorb(&mut self, other: &Self) {
        self.lo = self.lo.min(other.lo);
        self.hi = self.hi.max(other.hi);
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

/// Sort rows by item and merge each item's rows into one (intervals by union: exact, see
/// the module notes). The merged row's `leaf` is one of its holders.
pub(crate) fn merge_rows(rows: &mut Vec<InterfaceRow>) {
    rows.sort_unstable_by_key(|r| (r.a, r.b, r.lo, r.hi));
    let mut out: Vec<InterfaceRow> = Vec::with_capacity(rows.len());
    for r in rows.drain(..) {
        match out.last_mut() {
            Some(last) if last.item() == r.item() => last.absorb(&r),
            _ => out.push(r),
        }
    }
    *rows = out;
    // Release the drained input before a moving shrink allocates its replacement.
    // At most two row buffers then coexist, including allocator realloc transients.
    rows.shrink_to_fit();
}

impl Record for InterfaceRow {
    const SIZE: usize = 36;
    fn encode(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.leaf.to_le_bytes());
        put(out, 4, self.a);
        put(out, 16, self.b);
        out[28..32].copy_from_slice(&self.lo.to_le_bytes());
        out[32..36].copy_from_slice(&self.hi.to_le_bytes());
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
/// - reduce: the sorted items being streamed, the shared rows pushed (at most one per item)
///   and the shared points and open-edge endpoints written (at most one per item);
/// - ends: the rows, the shared points and the endpoints' sort and merge (from their counts);
/// - walk: the row sort through `finish` and its stream, consumed by the walk.
///
/// The pass checks its own peak against `declared` as it returns, before any downstream
/// stage writes (`HlodScratch`).
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScratchPhases {
    pub retained: u64,
    pub items: u64,
    pub reduce: u64,
    pub ends: u64,
    pub walk: u64,
}

/// The pass's two sorter configurations: `items` for the item and open-edge endpoint sorts,
/// whose streams end inside the pass (the stage's budget plan may grow it), and `rows` for
/// the row sort, whose stream the walk consumes after the pass (the request's floor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sorters {
    pub items: SortConfig,
    pub rows: SortConfig,
}
impl Sorters {
    pub(crate) fn uniform(sorter: SortConfig) -> Self {
        Self {
            items: sorter,
            rows: sorter,
        }
    }
}

/// Phase bounds (without the retained files) from the counts known when each phase starts.
/// At one fan-in each is non-increasing in the run buffer, so a grown `items` run never
/// declares more than the floor's.
fn items_phase(items: u64, sorters: Sorters) -> Option<u64> {
    Some(spill::<Item>(items, sorters.items)?.finish_peak)
}
fn reduce_phase(items: u64, sorters: Sorters) -> Option<u64> {
    let item = spill::<Item>(items, sorters.items)?;
    item.bytes
        .checked_add(item.stream_slack)?
        .checked_add(spill::<InterfaceRow>(items, sorters.rows)?.push_peak)?
        .checked_add(items.checked_mul(3 * KeyRecord::SIZE as u64 + Item::SIZE as u64)?)
}
fn ends_phase(rows: u64, points: u64, ends: u64, sorters: Sorters) -> Option<u64> {
    let sort = spill::<KeyRecord>(ends, sorters.items)?;
    spill::<InterfaceRow>(rows, sorters.rows)?
        .push_peak
        .checked_add(
            points
                .checked_add(ends)?
                .checked_mul(KeyRecord::SIZE as u64)?,
        )?
        .checked_add(sort.finish_peak)?
        .checked_add(sort.stream_slack)
}
fn walk_phase(rows: u64, sorters: Sorters) -> Option<u64> {
    let row = spill::<InterfaceRow>(rows, sorters.rows)?;
    row.finish_peak.checked_add(row.stream_slack)
}

/// The pass's worst case for `triangles` source triangles, charged to memory when scratch is
/// memory-backed (staged admission cannot take memory nobody charged): a triangle gives at
/// most six items (three points, three segments), each at most one row.
pub(crate) fn memory_backed_need(triangles: u64, sorter: SortConfig) -> Option<u64> {
    let sorter = Sorters::uniform(sorter);
    let items = triangles.checked_mul(6)?;
    Some(
        items_phase(items, sorter)?
            .max(reduce_phase(items, sorter)?)
            .max(ends_phase(items, items, items, sorter)?.checked_add(items.checked_mul(24)?)?)
            .max(walk_phase(items, sorter)?.checked_add(items.checked_mul(24)?)?),
    )
}

impl ScratchPhases {
    /// The pass's declared bound: its largest phase.
    pub(crate) fn declared(&self) -> u64 {
        self.retained
            .max(self.items)
            .max(self.reduce)
            .max(self.ends)
            .max(self.walk)
    }
}

/// Scratch files the pass holds at once besides sort runs (rounded as chunks): the shared
/// points and the open-edge endpoints.
pub(crate) const INCIDENCE_MAX_FILES: u64 = 4;

/// Resident bound of the pass with `workers` item workers: two sorters live at once (items
/// streamed into rows, then the rows beside the endpoints' sort), the item tasks' wave
/// ([`wave_bytes`]), the holders' per-leaf boxes and grid (the masks they read are captured
/// metadata), one item group (at most one item per leaf, in a vector at most twice that) and
/// two sequence writers.
pub(crate) fn memory_need(sorters: Sorters, leaf_triangles: u64, leaves: u64, workers: u64) -> u64 {
    let item = std::mem::size_of::<Item>() as u64;
    sorters
        .items
        .memory_bound::<Item>()
        .max(sorters.items.memory_bound::<KeyRecord>())
        .saturating_add(sorters.rows.memory_bound::<InterfaceRow>())
        .saturating_add(wave_bytes(
            workers.max(1),
            leaf_triangles,
            sorters.rows.buffer_bytes as u64,
        ))
        .saturating_add(leaves.saturating_mul(crate::mesh_large::corners::HOLDERS_LEAF_BYTES))
        .saturating_add(GROUP_MEMORY as u64 * item)
        .saturating_add(3 * sorters.rows.buffer_bytes as u64)
}

/// Each sort of the pass, by its worst-case count and resident record size, for the stage's
/// budget plan: at most six items per triangle, as many open-edge endpoints.
pub(crate) fn sort_loads(triangles: u64) -> [crate::mesh_large::capture_sort::SortLoad; 2] {
    let items = triangles.saturating_mul(6);
    [
        crate::mesh_large::capture_sort::SortLoad {
            records: items,
            record_bytes: std::mem::size_of::<Item>() as u64,
        },
        crate::mesh_large::capture_sort::SortLoad {
            records: items,
            record_bytes: std::mem::size_of::<KeyRecord>() as u64,
        },
    ]
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
    /// (leaf, segment) pairs on their leaf's open border.
    pub open_segments: u64,
    /// Source defects, counted after welding by canonical key and never locked: segments with
    /// exactly one incident surface face over all leaves (mesh borders, and the sides of
    /// T-junctions and unwelded seams), and those of them whose two endpoint keys each occur
    /// in a leaf besides the segment's own (candidate T-junctions and unwelded seams: their
    /// ends meet other geometry, the edge itself does not).
    pub source_open_edges: u64,
    pub source_open_edges_with_shared_endpoints: u64,
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

/// Every surface triangle's canonical corner keys in rank order: a second read of the
/// captured streams, holding nothing (S1b's face retention, `Borders::build`).
pub(crate) fn faces<'a>(
    triangles: &'a Sequence<FullTriangle>,
    depths: &'a Sequence<CornerDepth>,
    lattice: &'a LatticePlan,
) -> Result<impl Iterator<Item = Result<[Key; 3], JobError>> + 'a, JobError> {
    let mut ells = depths.reader()?;
    Ok(triangles.reader()?.filter_map(move |t| {
        let face = (|| {
            let t = t?;
            let ell = ells
                .next()
                .transpose()?
                .ok_or_else(|| state("corner depths end before the triangles"))?
                .depths;
            canonical_corners(lattice, &t, ell)
        })();
        face.map(|c| c.map(|c| c.map(key))).transpose()
    }))
}

/// One leaf's items from its surface triangles' canonical keys (`buffer` is replaced; the
/// faces' segments go through `segments`).
fn leaf_items(
    leaf: u32,
    faces: &[[Key; 3]],
    segments: &mut Vec<(Key, Key)>,
    buffer: &mut Vec<Item>,
) {
    segments.clear();
    buffer.clear();
    for k in faces {
        for i in 0..3 {
            let (p, q) = (k[i], k[(i + 1) % 3]);
            segments.push((p.min(q), p.max(q)));
        }
    }
    // Open border: segments held by other than exactly two of the leaf's faces.
    segments.sort_unstable();
    let mut i = 0;
    while i < segments.len() {
        let mut j = i + 1;
        while j < segments.len() && segments[j] == segments[i] {
            j += 1;
        }
        let (a, b) = segments[i];
        let faces = (j - i) as u32;
        let open = faces != 2;
        buffer.push(Item {
            a,
            b,
            leaf,
            open,
            faces,
        });
        for p in [a, b] {
            buffer.push(Item {
                a: p,
                b: p,
                leaf,
                open,
                faces: 0,
            });
        }
        i = j;
    }
    // A point is open when any of its segments is.
    buffer.sort_unstable();
    buffer.dedup_by(|later, earlier| {
        if (later.a, later.b) == (earlier.a, earlier.b) {
            earlier.open |= later.open;
            true
        } else {
            false
        }
    });
}

/// Leaves are read in chunks of at most about this many triangles (plus one leaf), one chunk
/// per task, at least eight chunks per worker when the triangles allow.
const CHUNK_TRIANGLES: u64 = 1 << 14;

/// Chunk tasks per worker in a wave: a wave's kept items are held until it ends.
const WAVE_CHUNKS: usize = 2;

/// The most the item tasks hold at once for `workers` workers and leaves of at most
/// `leaf_triangles`: a wave's chunks, each with one leaf's work (its faces, segments and
/// items) and its kept items (six per triangle at worst), and two stream buffers per worker.
pub(crate) fn wave_bytes(workers: u64, leaf_triangles: u64, buffer_bytes: u64) -> u64 {
    let item = std::mem::size_of::<Item>() as u64;
    let leaf = leaf_triangles.saturating_mul(12 * item);
    let kept = CHUNK_TRIANGLES
        .saturating_add(leaf_triangles)
        .saturating_mul(6 * item);
    let chunks = workers.saturating_mul(WAVE_CHUNKS as u64);
    chunks
        .saturating_mul(kept)
        .saturating_add(workers.saturating_mul(leaf))
        .saturating_add(workers.saturating_mul(2 * buffer_bytes))
}

#[cfg(test)]
thread_local! {
    /// Test seam: the pass on this thread sorts every item, as it did before filtering.
    pub(crate) static SORT_EVERY_ITEM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test seam: how many items the last pass on this thread sorted.
    pub(crate) static SORTED_ITEMS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One leaf of the pass: its triangle ranks.
struct Span {
    first: u64,
    end: u64,
}

/// What a chunk task found: its kept items in leaf order, and the counts of what it kept
/// back.
#[derive(Default)]
struct Part {
    kept: Vec<Item>,
    items: u64,
    open_segments: u64,
    degenerate: u64,
}

/// Every leaf's interface rows (exactly shared points and segments), sorted by
/// (leaf, a, b).
///
/// Items are generated by chunks of leaves on up to `workers` threads. An item another leaf
/// may hold ([`Holders`]) is sorted; every other item is its own leaf's alone, forms a group
/// of one in the reduce and writes no row, so the task accounts it: its count, and its open
/// flag when it is a segment. Source-open segments (one face in their leaf) are always
/// sorted, because the reduce derives open-edge endpoints from them in item order. The
/// reduce sees every holder of every shared item, so rows, counts and endpoints are those of
/// sorting every item.
///
/// `admit` preflights free space for a phase's whole declared bound (live bytes included)
/// before the phase writes (`ScratchPhases`). `progress` is the pass's cancellation check and
/// receives its steps (`hlod_incidence_items`, `_reduce`, `_ends`) and every sort checkpoint.
/// The first failing leaf in DFS order is reported, whatever order the workers met it in.
#[expect(
    clippy::too_many_arguments,
    reason = "the captured streams, the lattice, masks and the admission/progress hooks are separate inputs"
)]
pub(crate) fn build<'s>(
    scratch: &'s Scratch,
    sorters: Sorters,
    nodes: &Sequence<NodeRecord>,
    triangles: &Sequence<FullTriangle>,
    depths: &Sequence<CornerDepth>,
    lattice: &LatticePlan,
    index: &LeafIndex,
    workers: usize,
    admit: &dyn Fn(u64) -> Result<(), JobError>,
    progress: &'s StepProgress<'_>,
) -> Result<
    (
        SortedStream<'s, InterfaceRow>,
        IncidenceStats,
        Sequence<KeyRecord>,
    ),
    JobError,
> {
    let overflow = || state("incidence scratch bound overflows");
    let mut stats = IncidenceStats::default();
    let retained = scratch.live_bytes();
    stats.scratch.retained = retained;
    let mut spans = Vec::new();
    let mut boxes = Vec::new();
    let mut rank = 0u64;
    for node in nodes.reader()? {
        let node = node?;
        if node.kind != NodeKind::Leaf {
            continue;
        }
        if node.first != rank {
            return Err(state("leaf ranges are not contiguous in DFS order"));
        }
        rank = node.end;
        spans.push(Span {
            first: node.first,
            end: node.end,
        });
        boxes.push(node.bounds);
    }
    if rank < triangles.len() {
        return Err(state("source triangles outlast the leaves"));
    }
    if rank > triangles.len() {
        return Err(state("source triangles end before the leaves"));
    }
    stats.leaves = u32::try_from(spans.len())
        .map_err(|_| state("leaf index overflow"))?
        .into();
    let holders = Holders::new(index, &boxes, lattice.snapped().then(|| lattice.steps()))?;
    drop(boxes);
    progress.begin(Step::IncidenceItems, triangles.len())?;
    let mut items = ExternalSorter::<Item>::new(scratch, sorters.items)?.observed(progress);
    // Items are admitted ahead of those pushed. The lookahead is the floor's run (the row
    // sorter's), not a grown item run: a grown run spills later, never more, so with the
    // same lookahead no admission declares more than the floor's would.
    let item_run = (sorters.items.run_bytes.min(sorters.rows.run_bytes)
        / std::mem::size_of::<Item>())
    .max(1) as u64;
    let mut admitted_items = item_run;
    let mut sorted = 0u64;
    let workers = workers.max(1);
    let size = (triangles.len() / (workers as u64 * 8)).clamp(1, CHUNK_TRIANGLES);
    let chunks = chunks(&spans, size);
    let read_buffer = sorters.rows.buffer_bytes;
    #[cfg(test)]
    let every = SORT_EVERY_ITEM.with(std::cell::Cell::get);
    #[cfg(not(test))]
    let every = false;
    let done = AtomicU64::new(0);
    // One chunk's items, checking `stop` and the run's stop before each leaf.
    let chunk_items =
        |range: &std::ops::Range<usize>, stop: &AtomicBool| -> Result<Part, JobError> {
            let (first, end) = (spans[range.start].first, spans[range.end - 1].end);
            let mut source = triangles.range(first, end, read_buffer)?;
            let mut ells = depths.range(first, end, read_buffer)?;
            let mut part = Part::default();
            let (mut faces, mut segments, mut buffer) = (Vec::new(), Vec::new(), Vec::new());
            let mut points: Vec<(Key, bool)> = Vec::new();
            for leaf in range.clone() {
                crate::test_hooks::pace("mesh.incidence-items");
                if stop.load(Relaxed) {
                    return Err(JobError::new(JobErrorKind::Cancelled, "incidence stopped"));
                }
                scratch.check_stop()?;
                let span = &spans[leaf];
                faces.clear();
                for _ in span.first..span.end {
                    let t = source
                        .next()
                        .transpose()?
                        .ok_or_else(|| state("source triangles end before the leaves"))?;
                    let ell = ells
                        .next()
                        .transpose()?
                        .ok_or_else(|| state("corner depths end before the leaves"))?
                        .depths;
                    match canonical_corners(lattice, &t, ell)? {
                        Some(c) => faces.push(c.map(key)),
                        None => part.degenerate += 1,
                    }
                }
                let leaf = u32::try_from(leaf).map_err(|_| state("leaf index overflow"))?;
                leaf_items(leaf, &faces, &mut segments, &mut buffer);
                part.items += buffer.len() as u64;
                // Each point once: whether another leaf may hold it. The items are sorted and
                // every edge's endpoints are among them, so the points are in key order.
                points.clear();
                points.extend(
                    buffer
                        .iter()
                        .filter(|item| item.a == item.b)
                        .map(|item| (item.a, holders.other(leaf, item.a.map(f32::from_bits)))),
                );
                let other = |k: Key| {
                    points
                        .binary_search_by_key(&k, |p| p.0)
                        .map_or(true, |i| points[i].1)
                };
                for item in &buffer {
                    let segment = item.a != item.b;
                    if every
                        || (segment && item.faces == 1)
                        || (other(item.a) && (!segment || other(item.b)))
                    {
                        part.kept.push(*item);
                    } else if segment && item.open {
                        part.open_segments += 1;
                    }
                }
                done.fetch_add(span.end - span.first, Relaxed);
            }
            Ok(part)
        };
    for wave in chunks.chunks(WAVE_CHUNKS * workers) {
        let ran = run(
            wave.len(),
            workers,
            &mut || progress.advance(done.load(Relaxed)),
            |i, stop| chunk_items(&wave[i], stop),
        )?;
        let parts = match ran {
            Ran::All(parts) => parts,
            // The first failure in leaf order: the wave's chunks again, in order (every
            // earlier wave succeeded).
            Ran::Failed(error) => {
                let stop = AtomicBool::new(false);
                for range in wave {
                    progress.check()?;
                    chunk_items(range, &stop)?;
                }
                return Err(error);
            }
        };
        for part in parts {
            stats.items += part.items;
            stats.open_segments += part.open_segments;
            stats.decoded_degenerate += part.degenerate;
            let after = sorted + part.kept.len() as u64;
            if after > admitted_items {
                admitted_items = after + item_run;
                let spilled = spill::<Item>(admitted_items, sorters.items).ok_or_else(overflow)?;
                admit(retained + spilled.push_peak)?;
            }
            sorted = after;
            for item in part.kept {
                items.push(item)?;
            }
        }
        progress.advance(done.load(Relaxed))?;
    }
    drop(holders);
    #[cfg(test)]
    SORTED_ITEMS.with(|n| n.set(sorted));
    stats.scratch.items = items_phase(sorted, sorters)
        .and_then(|b| b.checked_add(retained))
        .ok_or_else(overflow)?;
    admit(stats.scratch.items)?;
    // Finishing waits for run writers and may merge: still inside the step.
    let sorted_items = items.finish()?;
    progress.end()?;
    let (rows, open_ends) = reduce(
        scratch,
        sorters,
        (sorted_items, sorted),
        &mut stats,
        retained,
        admit,
        progress,
    )?;
    Ok((rows, stats, open_ends))
}

/// Leaf chunks of about `size` triangles each (at least one leaf).
fn chunks(spans: &[Span], size: u64) -> Vec<std::ops::Range<usize>> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < spans.len() {
        let (mut end, mut triangles) = (start, 0);
        while end < spans.len() && (end == start || triangles < size) {
            triangles += spans[end].end - spans[end].first;
            end += 1;
        }
        chunks.push(start..end);
        start = end;
    }
    chunks
}

/// A high-valence item has a fixed resident prefix; every remaining owner spills to one
/// reusable positioned file. The stream is consumed twice, never paired with itself.
const GROUP_MEMORY: usize = 4096;
struct OwnerGroup {
    memory: Vec<Item>,
    spill: SlotFile,
    len: usize,
    open: u64,
    faces: u64,
}
impl OwnerGroup {
    fn new(scratch: &Scratch) -> Result<Self, JobError> {
        Ok(Self {
            memory: Vec::with_capacity(GROUP_MEMORY),
            spill: SlotFile::new(scratch, "interface-owner-group", Item::SIZE)?,
            len: 0,
            open: 0,
            faces: 0,
        })
    }
    fn push(&mut self, item: Item) -> Result<(), JobError> {
        if self.len < GROUP_MEMORY {
            self.memory.push(item);
        } else {
            let mut bytes = [0; Item::SIZE];
            item.encode(&mut bytes);
            self.spill.write((self.len - GROUP_MEMORY) as u64, &bytes)?;
        }
        self.len += 1;
        self.open += u64::from(item.open);
        self.faces += u64::from(item.faces);
        Ok(())
    }
    fn get(&self, i: usize) -> Result<Item, JobError> {
        if i < GROUP_MEMORY {
            return Ok(self.memory[i]);
        }
        let mut bytes = [0; Item::SIZE];
        self.spill.read((i - GROUP_MEMORY) as u64, &mut bytes)?;
        Ok(Item::decode(&bytes))
    }
    fn clear(&mut self) {
        self.memory.clear();
        self.len = 0;
        self.open = 0;
        self.faces = 0;
    }
}

/// Reduce sorted items to rows and the source-defect counts (see `build`).
fn reduce<'s>(
    scratch: &'s Scratch,
    sorters: Sorters,
    (items, sorted): (
        impl Iterator<Item = Result<Item, crate::spill::SpillError>>,
        u64,
    ),
    stats: &mut IncidenceStats,
    retained: u64,
    admit: &dyn Fn(u64) -> Result<(), JobError>,
    progress: &'s StepProgress<'_>,
) -> Result<(SortedStream<'s, InterfaceRow>, Sequence<KeyRecord>), JobError> {
    let overflow = || state("incidence scratch bound overflows");
    let sorter = sorters.rows;
    stats.scratch.reduce = reduce_phase(sorted, sorters)
        .and_then(|b| b.checked_add(retained))
        .ok_or_else(overflow)?;
    admit(stats.scratch.reduce)?;
    progress.begin(Step::IncidenceReduce, sorted)?;
    let mut rows = ExternalSorter::<InterfaceRow>::new(scratch, sorter)?.observed(progress);
    let mut shared =
        SequenceWriter::<KeyRecord>::new(scratch, "hlod-shared-points", sorter.buffer_bytes)?;
    let mut ends =
        SequenceWriter::<KeyRecord>::new(scratch, "hlod-open-ends", sorter.buffer_bytes)?;
    let mut open_ends =
        SequenceWriter::<KeyRecord>::new(scratch, "hlod-source-open-ends", sorter.buffer_bytes)?;
    let mut group = OwnerGroup::new(scratch)?;
    // The point group read last and whether it was shared: a segment `(a, b)` sorts after
    // the point `(a, a)` and before any later point, so this is its first endpoint's.
    let mut point: Option<(Key, bool)> = None;
    let leaves = stats.leaves;
    let mut flush = |group: &mut OwnerGroup,
                     stats: &mut IncidenceStats,
                     point: &mut Option<(Key, bool)>|
     -> Result<(), JobError> {
        let first = group.get(0)?;
        let (a, b) = (first.a, first.b);
        let held = group.len > 1;
        if held {
            let (lo, hi) = (first.leaf, group.get(group.len - 1)?.leaf);
            if a == b {
                stats.shared_points += 1;
            } else {
                stats.shared_segments += 1;
                stats.multi_leaf_segments += u64::from(group.len > 2);
            }
            for i in 0..group.len {
                let g = group.get(i)?;
                rows.push(InterfaceRow::new(g.leaf, a, b, lo, hi))?;
                stats.rows += 1;
            }
        }
        if a == b {
            *point = Some((a, held));
            if held {
                shared.push(&KeyRecord(a))?;
            }
        } else {
            stats.open_segments += group.open;
            if group.faces == 1 {
                stats.source_open_edges += 1;
                open_ends.push(&KeyRecord(a))?;
                open_ends.push(&KeyRecord(b))?;
                if *point == Some((a, true)) {
                    ends.push(&KeyRecord(b))?;
                }
            }
        }
        group.clear();
        Ok(())
    };
    let mut seen = 0u64;
    for item in items {
        let item = item?;
        seen += 1;
        if seen.is_multiple_of(4096) {
            progress.advance(seen)?;
        }
        if group
            .memory
            .first()
            .is_some_and(|g| (g.a, g.b) != (item.a, item.b))
        {
            flush(&mut group, stats, &mut point)?;
        }
        // Items are unique per leaf and sorted, so the group's leaves ascend strictly and a
        // group holds at most one item per leaf (charged in `memory_need`).
        if group.len as u64 >= leaves {
            return Err(state(
                "an interface item group holds more items than leaves",
            ));
        }
        group.push(item)?;
    }
    if group.len != 0 {
        flush(&mut group, stats, &mut point)?;
    }
    drop(group);
    progress.end()?;
    let (shared, ends, open_ends) = (shared.finish()?, ends.finish()?, open_ends.finish()?);
    let open_bytes = open_ends.len() * KeyRecord::SIZE as u64;
    // Second endpoints: the open edges whose first endpoint is shared, sorted by their second
    // and merged against the shared points (both ascending).
    stats.scratch.ends = ends_phase(stats.rows, shared.len(), ends.len(), sorters)
        .and_then(|b| b.checked_add(retained))
        .and_then(|b| b.checked_add(open_bytes))
        .ok_or_else(overflow)?;
    admit(stats.scratch.ends)?;
    let endpoints = ends.len();
    progress.begin(Step::IncidenceEnds, endpoints.saturating_mul(2))?;
    let mut sorted = ExternalSorter::<KeyRecord>::new(scratch, sorters.items)?.observed(progress);
    let mut done = 0u64;
    for e in ends.reader()? {
        sorted.push(e?)?;
        done += 1;
        if done.is_multiple_of(4096) {
            progress.advance(done)?;
        }
    }
    drop(ends);
    let mut points = shared.reader()?.peekable();
    for e in sorted.finish()? {
        let e = e?;
        done += 1;
        if done.is_multiple_of(4096) {
            progress.advance(done)?;
        }
        while points
            .next_if(|p| matches!(p, Ok(p) if p.0 < e.0))
            .is_some()
        {}
        if let Some(p) = points.peek() {
            match p {
                Ok(p) if p.0 == e.0 => stats.source_open_edges_with_shared_endpoints += 1,
                Ok(_) => {}
                Err(_) => return Err(points.next().unwrap().unwrap_err().into()),
            }
        }
    }
    drop(points);
    drop(shared);
    stats.scratch.walk = walk_phase(stats.rows, sorters)
        .and_then(|b| b.checked_add(retained))
        .and_then(|b| b.checked_add(open_bytes))
        .ok_or_else(overflow)?;
    admit(stats.scratch.walk)?;
    // The rows sort finishes (and may merge) inside the last step.
    let rows = rows.finish()?;
    progress.end()?;
    Ok((rows, open_ends))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SORT: SortConfig = SortConfig {
        run_bytes: 1 << 16,
        fan_in: 4,
        buffer_bytes: 1 << 12,
        jobs: 1,
        writers: 0,
    };

    fn k(p: [f64; 3]) -> Key {
        key(p.map(|v| v as f32))
    }

    /// The rows and stats of the exact pass over leaves given as triangles of keys.
    fn incidence(leaves: &[Vec<[Key; 3]>]) -> (Vec<InterfaceRow>, IncidenceStats) {
        let (rows, stats, _) = incidence_with(leaves, Sorters::uniform(SORT));
        (rows, stats)
    }

    /// Rows, counts and the source open-edge endpoints the borders read.
    fn incidence_with(
        leaves: &[Vec<[Key; 3]>],
        sorters: Sorters,
    ) -> (Vec<InterfaceRow>, IncidenceStats, Vec<KeyRecord>) {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        scratch.set_chunk_bytes(1 << 10);
        let mut sorter = ExternalSorter::<Item>::new(&scratch, sorters.items).unwrap();
        let mut stats = IncidenceStats {
            leaves: leaves.len() as u64,
            ..Default::default()
        };
        let (mut segments, mut buffer) = (Vec::new(), Vec::new());
        for (leaf, faces) in leaves.iter().enumerate() {
            leaf_items(leaf as u32, faces, &mut segments, &mut buffer);
            stats.items += buffer.len() as u64;
            for i in &buffer {
                sorter.push(*i).unwrap();
            }
        }
        let sorted = stats.items;
        let (rows, open_ends) = reduce(
            &scratch,
            sorters,
            (sorter.finish().unwrap(), sorted),
            &mut stats,
            0,
            &|_| Ok(()),
            StepProgress::silent(),
        )
        .unwrap();
        let mut rows: Vec<InterfaceRow> = rows.map(Result::unwrap).collect();
        rows.sort_unstable();
        let ends: Vec<KeyRecord> = open_ends.reader().unwrap().map(Result::unwrap).collect();
        (rows, stats, ends)
    }

    fn rows_of(rows: &[InterfaceRow], leaf: u32) -> Vec<(Key, Key)> {
        rows.iter()
            .filter(|r| r.leaf == leaf)
            .map(InterfaceRow::item)
            .collect()
    }

    /// Owner decision: a chord between two shared points is not shared unless another leaf
    /// holds that edge. Leaf 0 is a quad split along its diagonal; leaf 1 holds the quad's
    /// corners in a fan from a centre point, so it shares both diagonal endpoints with leaf 0
    /// but not the diagonal itself.
    #[test]
    fn a_chord_between_shared_points_is_not_locked() {
        let (p, q, r, s, c) = (
            k([0., 0., 0.]),
            k([1., 0., 0.]),
            k([1., 1., 0.]),
            k([0., 1., 0.]),
            k([0.5, 0.5, 1.]),
        );
        let leaf0 = vec![[p, q, r], [p, r, s]];
        let leaf1 = vec![[p, c, q], [q, c, r], [r, c, s], [s, c, p]];
        let (rows, stats) = incidence(&[leaf0, leaf1]);
        let locked = rows_of(&rows, 0);
        let diagonal = (p.min(r), p.max(r));
        assert!(!locked.contains(&diagonal), "{locked:?}");
        for e in [(p, q), (q, r), (r, s), (s, p)] {
            assert!(locked.contains(&(e.0.min(e.1), e.0.max(e.1))), "{locked:?}");
        }
        for v in [p, q, r, s] {
            assert!(locked.contains(&(v, v)));
        }
        assert_eq!(stats.shared_segments, 4, "{stats:?}");
        assert_eq!(stats.shared_points, 4, "{stats:?}");
    }

    /// A source T-junction is reported, not locked: leaf 1's vertex `m` lies on leaf 0's edge
    /// `pq` without being one of its vertices. Nothing is shared across it; the two halves of
    /// leaf 1's side and leaf 0's long edge are open source edges whose endpoints meet other
    /// geometry.
    #[test]
    fn a_source_t_junction_is_counted_not_locked() {
        let (p, q, m) = (k([0., 0., 0.]), k([2., 0., 0.]), k([1., 0., 0.]));
        let (a, b, c) = (k([1., -1., 0.]), k([0., 1., 0.]), k([2., 1., 0.]));
        let leaf0 = vec![[p, a, q]];
        let leaf1 = vec![[p, m, b], [m, c, b], [m, q, c]];
        let (rows, stats) = incidence(&[leaf0, leaf1]);
        assert!(rows_of(&rows, 0).iter().all(|i| i.0 == i.1), "{rows:?}");
        assert!(!rows.iter().any(|r| r.a == m), "{rows:?}");
        assert_eq!(stats.shared_segments, 0, "{stats:?}");
        // pq (leaf 0) and pm, mq (leaf 1): each with one face, ends p and q shared.
        assert!(stats.source_open_edges >= 3, "{stats:?}");
        assert_eq!(
            stats.source_open_edges_with_shared_endpoints, 1,
            "{stats:?}"
        );
    }

    /// Sol's tolerance adversaries (dense false boxes, ribbons beside slivers, random chords,
    /// slabs, mixed tolerances) as plain geometry: the exact pass does no geometric work, only
    /// sorts of at most six items per triangle, so its rows and items grow linearly however
    /// close the geometry sits and nothing near-but-unshared is locked.
    #[test]
    fn tolerance_adversaries_cost_linear_work_and_lock_nothing_unshared() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for n in [512usize, 1024, 2048, 4096] {
            let mut leaves: Vec<Vec<[Key; 3]>> = vec![Vec::new(); 8];
            for i in 0..n {
                // Dense false boxes and slabs: slivers a hair apart, one per leaf in turn.
                let y = i as f64 * 1e-6;
                leaves[i % 4].push([k([0., y, 0.]), k([1., y, 0.]), k([0.5, y, 1e-6])]);
                // Chords and mixed tolerances: random long triangles crossing every region.
                let t = [
                    [next(), next(), next()],
                    [next(), next(), next()],
                    [next(), next(), next()],
                ];
                leaves[4 + i % 4].push(t.map(k));
            }
            let (rows, stats) = incidence(&leaves);
            assert!(stats.items <= 6 * 2 * n as u64, "{stats:?}");
            assert!(stats.rows <= stats.items, "{stats:?}");
            // Only coincident geometry is shared: these slivers and chords share nothing.
            assert_eq!(stats.shared_segments, 0, "n {n}: {stats:?}");
            assert!(
                rows.is_empty() || rows.iter().all(|r| r.is_vertex()),
                "n {n}"
            );
        }
    }

    /// Owners of one item merge to the union of their intervals, whatever their number.
    #[test]
    fn a_point_shared_by_many_leaves_is_one_row_per_leaf_with_the_full_interval() {
        let c = k([0., 0., 0.]);
        let leaves: Vec<Vec<[Key; 3]>> = (0..GROUP_MEMORY + 1)
            .map(|i| {
                let a = i as f64 / (GROUP_MEMORY + 1) as f64 * std::f64::consts::TAU;
                let b = (i + 1) as f64 / (GROUP_MEMORY + 1) as f64 * std::f64::consts::TAU;
                vec![[c, k([a.cos(), a.sin(), 0.]), k([b.cos(), b.sin(), 0.])]]
            })
            .collect();
        let (rows, stats) = incidence(&leaves);
        let centre: Vec<_> = rows.iter().filter(|r| r.a == c && r.b == c).collect();
        assert_eq!(centre.len(), GROUP_MEMORY + 1);
        assert!(centre
            .iter()
            .all(|r| (r.lo, r.hi) == (0, GROUP_MEMORY as u32)));
        assert!(stats.shared_segments >= GROUP_MEMORY as u64, "{stats:?}");
    }

    /// A group of one item per leaf is charged at a fixed size: per leaf, the bound grows by
    /// the holders' box and grid entries only. Each item worker adds its share of the wave.
    #[test]
    fn item_groups_have_fixed_memory_independent_of_leaf_count() {
        let base = memory_need(Sorters::uniform(SORT), 256, 0, 1);
        let more = memory_need(Sorters::uniform(SORT), 256, 1 << 20, 1);
        assert_eq!(
            more - base,
            (1 << 20) * crate::mesh_large::corners::HOLDERS_LEAF_BYTES
        );
        let wave = |w| wave_bytes(w, 256, SORT.buffer_bytes as u64);
        assert_eq!(
            memory_need(Sorters::uniform(SORT), 256, 0, 4) - base,
            wave(4) - wave(1)
        );
    }

    /// A strip of `n` unit squares split into leaves of `per` squares: every inner edge and
    /// its endpoints are shared between neighbouring leaves.
    fn strip(n: usize, per: usize) -> Vec<Vec<[Key; 3]>> {
        let p = |x: usize, y: usize| k([x as f64, y as f64, 0.]);
        (0..n)
            .collect::<Vec<_>>()
            .chunks(per)
            .map(|xs| {
                xs.iter()
                    .flat_map(|&x| {
                        [
                            [p(x, 0), p(x + 1, 0), p(x + 1, 1)],
                            [p(x, 0), p(x + 1, 1), p(x, 1)],
                        ]
                    })
                    .collect()
            })
            .collect()
    }

    /// The pass's rows and counts are those of the floor for any item-sort run size, fan-in
    /// and writer count, from one-record runs merged twice to background writers.
    #[test]
    fn rows_and_stats_are_identical_for_every_item_sort_plan() {
        let leaves = strip(3_000, 7);
        let (rows, stats, ends) = incidence_with(&leaves, Sorters::uniform(SORT));
        assert!(
            stats.items > 15_000 && stats.shared_segments > 400 && ends.len() > 1_000,
            "{stats:?}"
        );
        let tiny = |records: usize, fan_in, writers| SortConfig {
            run_bytes: records * std::mem::size_of::<Item>(),
            fan_in,
            writers,
            ..SORT
        };
        // Spilled runs with and without writers, multi-pass merges, and one in-memory run.
        for items in [
            tiny(1, 2, 0),
            tiny(100, 3, 2),
            tiny(997, 4, 3),
            tiny(1 << 15, 4, 7),
        ] {
            let (planned_rows, planned, planned_ends) =
                incidence_with(&leaves, Sorters { items, rows: SORT });
            assert_eq!(planned_rows, rows, "{items:?}");
            assert_eq!(planned_ends, ends, "{items:?}");
            assert_eq!(
                serde_json::to_value(planned).unwrap(),
                serde_json::to_value(stats).unwrap(),
                "{items:?}"
            );
        }
    }

    /// At the floor's fan-in, a larger item run never declares a larger phase, for any
    /// counts: the stage plan may grow the item sort without new scratch refusals.
    #[test]
    fn grown_item_runs_never_declare_more_scratch_at_one_fan_in() {
        let floor = Sorters::uniform(SORT);
        for scale in [1u64, 2, 3, 8, 64, 1 << 10, 1 << 16] {
            let grown = Sorters {
                items: SortConfig {
                    run_bytes: SORT.run_bytes * scale as usize,
                    writers: 3,
                    ..SORT
                },
                rows: SORT,
            };
            for items in [0u64, 1, 1_000, 1 << 20, 1 << 26, 1 << 32] {
                let rows = items / 3;
                assert!(items_phase(items, grown) <= items_phase(items, floor));
                assert!(reduce_phase(items, grown) <= reduce_phase(items, floor));
                assert!(
                    ends_phase(rows, rows, items, grown) <= ends_phase(rows, rows, items, floor)
                );
                assert_eq!(walk_phase(rows, grown), walk_phase(rows, floor));
            }
        }
    }
}
