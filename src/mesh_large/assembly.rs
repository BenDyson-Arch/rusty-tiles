//! Capture's rank-ordered full-triangle sequence, assembled directly: no corner sorts and no
//! triangle merge.
//!
//! 1. **Decode.** Per expanded primitive, the vertices its triangles use are decoded once (world
//!    position, source frame, canonical f32, the layout's attributes). They are kept in memory
//!    while the plan's allowance holds; otherwise they are written to a compressed scratch
//!    sequence and read back through a small block cache.
//! 2. **Histogram.** Every triangle's Morton key, counted per depth-[`PREFIX_DEPTH`] prefix.
//! 3. **Buckets.** Contiguous prefix ranges of at most the plan's bucket size; a single prefix
//!    cell over it is a bucket of its own, sorted externally.
//! 4. **Distribute.** Triangles are built in parallel and gathered per bucket; full buffers go
//!    to append-only staging files as compressed runs.
//! 5. **Sort.** Buckets are sorted in key order by the strict (key, provenance, ordinal) order,
//!    a window of them in parallel, and written in order.
//!
//! The sequence equals the merge-sorted one. Every record is built by the same arithmetic from
//! the same per-vertex values (decoding is a pure function of primitive and vertex); the order
//! is a strict total order; buckets are contiguous key ranges written in order; and inside a
//! bucket the whole order is applied.
use super::{
    attributes::{corner_attribute_bytes, flags, read_attributes, CornerAttributes, FullTriangle},
    capture::{canonical, source_position, triangle_bytes, vertices, SourceScan},
    input::StreamSource,
    phases::{Step, StepProgress},
    records::{Bounds, Cube, Provenance, TriangleRecord, KEY_DEPTH},
    source_crs::SourceFrame,
    subset::{Ranks, Selection},
    tasks::{run, Ran},
};
use crate::{
    gltf_source::source::ScenePrimitive,
    spill::{
        ByteFile, ByteWriter, ExternalSorter, Layout, RandomRecords, Record, Scratch, Sequence,
        SequenceWriter, SortConfig,
    },
    JobError, JobErrorKind,
};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
    Mutex,
};

type Result<T> = std::result::Result<T, JobError>;
fn invalid(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidInput, message)
}
fn invariant(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}
fn overflow() -> JobError {
    JobError::new(
        JobErrorKind::ResourceLimit,
        "assembly count or byte sum overflow",
    )
}

/// Depth of the histogram's key prefix: 8^7 cells.
pub(crate) const PREFIX_DEPTH: u8 = 7;
const CELLS: usize = 1 << (3 * PREFIX_DEPTH as usize);
/// Triangles one histogram or distribution task covers.
const CHUNK_TRIANGLES: usize = 1 << 14;
/// Vertices one decode task covers.
const CHUNK_VERTICES: usize = 1 << 16;
/// Staging files of ordinary buckets (consecutive bucket ranges); a heavy bucket has its own.
const STAGING_GROUPS: usize = 16;
/// Heavy buckets with a staging file of their own: those over 1/OWN_FILES of the records.
const OWN_FILES: usize = 64;
/// The most staging files open at once: the groups, the own files, and the group split by
/// each own file.
pub(crate) const STAGING_FILES: usize = STAGING_GROUPS + 2 * OWN_FILES;
/// Write buffer of a staging file.
const STAGING_BUFFER: usize = 64 << 10;
/// Smallest and largest gathering buffer per bucket.
const GATHER_MIN: u64 = 4 << 10;
const GATHER_MAX: u64 = 1 << 20;

/// Working sets of the assembly, fixed once by capture's plan and charged in its resident
/// bound ([`AssemblyPlan::memory_bytes`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AssemblyPlan {
    /// Decoded vertices held in memory, over all primitives; the rest spill.
    pub vertex_allowance_bytes: u64,
    /// Blocks each spilled-vertex reader keeps.
    pub cache_blocks: usize,
    /// Records of a bucket sorted in memory, in bytes; a larger bucket is sorted externally.
    pub bucket_bytes: u64,
    /// Gathering buffers over all buckets.
    pub gather_bytes: u64,
    /// Buckets sorted ahead of the writer.
    pub window: usize,
}

impl AssemblyPlan {
    /// The plan's fixed working sets for `workers`, from capture's sort allowance (the
    /// assembly runs before capture's sorts, so it may use theirs), with no vertex allowance
    /// yet ([`Self::with_vertex_allowance`]).
    pub(crate) fn fixed(workers: usize, sort_allowance: u64) -> Self {
        let window = 2 * workers.max(1);
        // The window (three copies of each bucket) takes half the allowance; an external
        // sort beside it takes the other half.
        Self {
            vertex_allowance_bytes: 0,
            cache_blocks: 64,
            bucket_bytes: (sort_allowance / (6 * window as u64)).clamp(BUCKET_FLOOR, 64 << 20),
            gather_bytes: 16 << 20,
            window,
        }
    }

    pub(crate) fn with_vertex_allowance(self, bytes: u64) -> Self {
        Self {
            vertex_allowance_bytes: bytes,
            ..self
        }
    }

    /// The resident bytes of the assembly beside capture's other terms, for `workers` tasks
    /// at once, `n` triangles of `layout` and readers of `buffer_bytes`; `sorter` is one
    /// external sort (a heavy bucket's, the run list's, or one of a routed primitive's two).
    /// The largest of its steps:
    /// - decoding and routing: the vertex allowance, a decode wave and two external sorts;
    /// - keying and distributing: the vertex allowance, each task's reader and rows, the
    ///   histogram and bucket tables, and the gathering buffers;
    /// - sorting: the window's buckets (records and sort keys) and one external sort.
    pub(crate) fn memory_bytes(
        &self,
        workers: usize,
        n: u64,
        layout: Layout,
        buffer_bytes: usize,
        sorter: SortConfig,
    ) -> Option<u64> {
        let w = workers.max(1) as u64;
        let vertex = DecodedVertex::size_in(layout) as u64;
        let record = FullTriangle::size_in(layout) as u64;
        let sort = sorter.memory_bound::<FullTriangle>();
        let reader = crate::spill::random_cache_bound(vertex as usize, self.cache_blocks)
            .checked_add(2 * buffer_bytes as u64)?;
        // A decode wave: two chunks per worker, encoded, beside the stores already decoded.
        let decode = (2 * w).checked_mul(CHUNK_VERTICES as u64 * (vertex + 12))?;
        let decoding = [
            self.vertex_allowance_bytes,
            decode,
            sort.checked_mul(2)?,
            reader,
        ]
        .iter()
        .try_fold(0u64, |sum, &b| sum.checked_add(b))?;
        // A distribution task: its rows, their encoded bytes and one compressed run.
        let task = (CHUNK_TRIANGLES as u64)
            .checked_mul(std::mem::size_of::<(u32, FullTriangle)>() as u64 + record)?
            .checked_add(2 * GATHER_MAX)?
            .checked_add(reader)?;
        // Histogram counts and the cell-to-bucket table.
        let tables = CELLS as u64 * 12;
        // Gathering buffers grow by doubling (up to twice their flush size), and one task can
        // add a whole chunk's records to one buffer before it flushes.
        let gathering = self
            .gather_bytes
            .max(Self::buckets_bound(n, record, self.bucket_bytes)?.checked_mul(GATHER_MIN)?)
            .checked_mul(2)?
            .checked_add(w.checked_mul(CHUNK_TRIANGLES as u64 * record)?)?;
        // Every staging file open at once with its write buffer, and the run list's writer.
        let staging = (STAGING_FILES * STAGING_BUFFER.min(buffer_bytes)) as u64;
        let distributing = [
            self.vertex_allowance_bytes,
            w.checked_mul(task)?,
            tables,
            gathering,
            staging,
            buffer_bytes as u64,
        ]
        .iter()
        .try_fold(0u64, |sum, &b| sum.checked_add(b))?;
        // The window's buckets: their records, a sorted copy and the sort keys.
        let window = (self.window as u64).checked_mul(self.bucket_bytes.checked_mul(3)?)?;
        let sorting = window
            .checked_add(sort)?
            .checked_add(4 * buffer_bytes as u64)?;
        Some(decoding.max(distributing).max(sorting))
    }

    /// At most this many buckets for `n` records of `record` bytes: ordinary buckets hold at
    /// least half the bucket size, except the one before each heavy bucket; heavy buckets
    /// hold more than it.
    fn buckets_bound(n: u64, record: u64, bucket_bytes: u64) -> Option<u64> {
        let per = (bucket_bytes / record).max(1);
        Some(n.checked_mul(2)? / per + 2)
    }
}

/// The assembly's scratch shape before it runs: the decoded vertices of every expanded
/// primitive at raw size (all of them may spill) and the files it may hold at once.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ScratchShape {
    pub vertex_bytes: u64,
    pub files: u64,
}

/// The most bytes the list of staged runs takes for `n` records under `layout`: every run
/// but each bucket's last holds at least the smallest gathering buffer.
pub(crate) fn run_list_bytes(n: u64, layout: Layout) -> Option<u64> {
    let record = FullTriangle::size_in(layout) as u64;
    let runs = n
        .checked_mul(record)?
        .div_ceil(GATHER_MIN)
        .checked_add(AssemblyPlan::buckets_bound(n, record, BUCKET_FLOOR)?)?;
    runs.checked_mul(StagedRun::SIZE as u64)
}

/// Bytes of one decoded vertex under `layout`.
pub(crate) fn vertex_record_bytes(layout: Layout) -> u64 {
    DecodedVertex::size_in(layout) as u64
}

impl ScratchShape {
    /// For `source` under `layout`; in a subset bake, only the vertices its kept triangles
    /// use are stored, and a primitive without one has no store.
    pub(crate) fn of(
        source: &StreamSource,
        layout: Layout,
        subset: Option<&Selection>,
    ) -> Option<Self> {
        let size = DecodedVertex::size_in(layout) as u64;
        let mut vertex_bytes = 0u64;
        let mut primitives = 0u64;
        for (i, p) in source.document.scene_primitives().enumerate() {
            let stored = match subset {
                None => p.position.count as u64,
                Some(subset) => subset.primitives.get(i)?.vertices,
            };
            if stored == 0 && subset.is_some() {
                continue;
            }
            vertex_bytes = vertex_bytes.checked_add(stored.checked_mul(size)?)?;
            primitives += 1;
        }
        Some(Self {
            vertex_bytes,
            // Spilled vertex stores, staging files, the run list and one external sort's
            // runs (counted elsewhere).
            files: primitives
                .checked_add(STAGING_FILES as u64)?
                .checked_add(1)?,
        })
    }
}

/// The vertex allowance when available memory is unknown.
pub(crate) const DEFAULT_VERTEX_ALLOWANCE: u64 = 256 << 20;

/// The smallest in-memory bucket a plan uses.
pub(crate) const BUCKET_FLOOR: u64 = 8 << 20;

/// What the assembly reports in diagnostics.
#[derive(Clone, Debug, Default, Serialize)]
pub struct AssemblyReport {
    pub plan: Option<AssemblyPlan>,
    /// Expanded primitives whose decoded vertices were held in memory, and spilled.
    pub resident_primitives: u64,
    pub spilled_primitives: u64,
    /// Spilled primitives whose corners were placed by sorting (too little index locality for
    /// the block caches); included in `spilled_primitives`.
    pub routed_primitives: u64,
    /// Blocks the spilled-vertex readers decoded (a measure of index locality).
    pub spilled_blocks_decoded: u64,
    pub buckets: u64,
    /// Buckets over the bucket size (one prefix cell), sorted externally.
    pub heavy_buckets: u64,
    pub largest_bucket_records: u64,
    /// Compressed runs written to staging, and their bytes.
    pub staging_runs: u64,
    pub staging_bytes: u64,
    /// In a subset bake, expanded primitives with no captured triangle (nothing decoded).
    #[serde(skip_serializing_if = "is_zero")]
    pub excluded_primitives: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// One decoded vertex: its canonical position and its attributes under the run layout.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DecodedVertex {
    position: [f32; 3],
    attributes: CornerAttributes,
}
/// Records only need an order to be sortable; decoded vertices are never sorted. The order is
/// that of their bits, consistent with equality of bits.
impl DecodedVertex {
    fn bits(&self) -> [u32; 18] {
        let a = &self.attributes;
        let mut out = [0u32; 18];
        let values = self
            .position
            .iter()
            .chain(&a.normal)
            .chain(&a.tangent)
            .chain(a.texcoords.iter().flatten())
            .chain(&a.color);
        for (o, v) in out.iter_mut().zip(values) {
            *o = v.to_bits();
        }
        out
    }
}
impl PartialEq for DecodedVertex {
    fn eq(&self, other: &Self) -> bool {
        self.bits() == other.bits()
    }
}
impl Eq for DecodedVertex {}
impl PartialOrd for DecodedVertex {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for DecodedVertex {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.bits().cmp(&other.bits())
    }
}
impl Record for DecodedVertex {
    const SIZE: usize = 72;
    fn encode(&self, out: &mut [u8]) {
        self.encode_in(Layout::FULL, out);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self::decode_in(Layout::FULL, bytes)
    }
    fn size_in(layout: Layout) -> usize {
        12 + corner_attribute_bytes(layout)
    }
    fn encode_in(&self, layout: Layout, out: &mut [u8]) {
        for (a, v) in self.position.iter().enumerate() {
            out[4 * a..4 * a + 4].copy_from_slice(&v.to_le_bytes());
        }
        self.attributes.encode(layout, &mut out[12..]);
    }
    fn decode_in(layout: Layout, bytes: &[u8]) -> Self {
        Self {
            position: std::array::from_fn(|a| {
                f32::from_le_bytes(bytes[4 * a..4 * a + 4].try_into().unwrap())
            }),
            attributes: CornerAttributes::decode(layout, &bytes[12..]),
        }
    }
}

/// A spilled primitive is read through its block caches only when at least this share (in
/// 32nds) of its triangle chunks touch at most a cache's worth of vertex blocks; otherwise its
/// corners are placed by sorting ([`place_corners`]).
const LOCAL_CHUNKS_PER_32: u64 = 31;

/// One primitive's decoded vertices: in memory, spilled in vertex order (read through block
/// caches), or placed per triangle by sorting (read in triangle order).
enum Store {
    Memory(Vec<u8>),
    Spilled(Sequence<DecodedVertex>),
    Placed(Sequence<TriangleCorners>),
}

/// A subset bake's view of one primitive: its kept triangles and the vertices they use. Its
/// stores hold only those vertices, by rank, and a routed primitive's placed corners only
/// those triangles, by rank. Without one, every vertex and triangle has its own slot.
#[derive(Clone, Copy)]
struct Slots<'a> {
    triangles: &'a Ranks,
    vertices: &'a Ranks,
}

/// The store slot of vertex `v`.
fn vertex_slot(slots: Option<Slots<'_>>, v: u64) -> Result<u64> {
    match slots {
        None => Ok(v),
        Some(slots) => Ok(slots
            .vertices
            .rank(usize::try_from(v).map_err(|_| overflow())?) as u64),
    }
}

/// The placed-corner slot of triangle `t`.
fn triangle_slot(slots: Option<Slots<'_>>, t: usize) -> usize {
    slots.map_or(t, |slots| slots.triangles.rank(t))
}

/// Whether triangle `t` is captured.
fn kept(slots: Option<Slots<'_>>, t: usize) -> bool {
    slots.is_none_or(|slots| slots.triangles.contains(t))
}

/// Whether `range` holds no captured triangle.
fn none_kept(slots: Option<Slots<'_>>, range: &std::ops::Range<usize>) -> bool {
    slots.is_some_and(|slots| slots.triangles.rank(range.end) == slots.triangles.rank(range.start))
}

/// Per-task access to a [`Store`] for one range of triangles.
struct StoreReader<'a> {
    access: Access<'a>,
    slots: Option<Slots<'a>>,
}

enum Access<'a> {
    Memory(&'a [u8]),
    Spilled(RandomRecords<DecodedVertex>, Vec<u8>, Vec<DecodedVertex>),
    Placed(crate::spill::SequenceReader<'a, TriangleCorners>, usize),
}

impl StoreReader<'_> {
    /// The corners of triangle `t`, whose vertices are `verts`. A placed store is read in
    /// triangle order, so `t` must follow the previous (captured) triangle.
    fn corners(&mut self, layout: Layout, t: usize, verts: [u64; 3]) -> Result<[DecodedVertex; 3]> {
        let size = DecodedVertex::size_in(layout);
        let slots = self.slots;
        match &mut self.access {
            Access::Memory(bytes) => Ok(verts.map(|v| {
                let v = v as usize;
                let at = slots.map_or(v, |slots| slots.vertices.rank(v)) * size;
                DecodedVertex::decode_in(layout, &bytes[at..at + size])
            })),
            Access::Spilled(random, encoded, out) => {
                let mut corners = [DecodedVertex::default(); 3];
                for (c, v) in verts.iter().enumerate() {
                    random.read(vertex_slot(slots, *v)?, 1, encoded, out)?;
                    corners[c] = out[0];
                }
                Ok(corners)
            }
            Access::Placed(reader, next) => {
                let t = triangle_slot(slots, t);
                if t != *next {
                    return Err(invariant("placed corners read out of triangle order"));
                }
                *next += 1;
                let corners = reader
                    .next()
                    .transpose()?
                    .ok_or_else(|| invariant("placed corners end early"))?;
                Ok(corners.0)
            }
        }
    }
    fn decoded_blocks(&self) -> u64 {
        match &self.access {
            Access::Spilled(random, ..) => random.decoded_blocks(),
            _ => 0,
        }
    }
}

impl Store {
    /// A reader of triangles `range` (of the primitive's triangles) under `slots`.
    fn reader<'a>(
        &'a self,
        slots: Option<Slots<'a>>,
        cache_blocks: usize,
        range: std::ops::Range<usize>,
        buffer_bytes: usize,
    ) -> Result<StoreReader<'a>> {
        let access = match self {
            Self::Memory(bytes) => Access::Memory(bytes),
            Self::Spilled(sequence) => Access::Spilled(
                sequence.random_cached(cache_blocks)?,
                Vec::new(),
                Vec::new(),
            ),
            Self::Placed(sequence) => {
                let (start, end) = (
                    triangle_slot(slots, range.start),
                    triangle_slot(slots, range.end),
                );
                Access::Placed(
                    sequence.range(start as u64, end as u64, buffer_bytes)?,
                    start,
                )
            }
        };
        Ok(StoreReader { access, slots })
    }
}

/// The used vertices of `p` (of its `kept` triangles in a subset bake) as a bitmap, and how
/// many of its triangle chunks touch at most `cache_blocks` distinct blocks of `per_block`
/// vertices (indices were checked by payload validation; reading them again checks nothing
/// new). A subset bake's locality is counted over store slots instead ([`slot_locality`]).
fn used_vertices(
    source: &StreamSource,
    p: &ScenePrimitive<'_>,
    buffer_bytes: usize,
    workers: usize,
    (per_block, cache_blocks): (u64, usize),
    kept: Option<&Ranks>,
    poll: &mut dyn FnMut() -> Result<()>,
) -> Result<std::result::Result<(Vec<AtomicU64>, u64), JobError>> {
    let used: Vec<AtomicU64> = (0..p.position.count.div_ceil(64))
        .map(|_| AtomicU64::new(0))
        .collect();
    let local = AtomicU64::new(0);
    let marked = run(
        p.triangles.div_ceil(CHUNK_TRIANGLES),
        workers,
        poll,
        |chunk, stop| {
            let mut indices = p.indices.map(|v| source.reader(&v, buffer_bytes));
            let start = chunk * CHUNK_TRIANGLES;
            let (mut word, mut bits) = (usize::MAX, 0u64);
            let mut blocks = Vec::with_capacity(3 * CHUNK_TRIANGLES);
            for triangle in start..(start + CHUNK_TRIANGLES).min(p.triangles) {
                if triangle.is_multiple_of(1024) && stop.load(Relaxed) {
                    return Err(JobError::new(JobErrorKind::Cancelled, "assembly stopped"));
                }
                if kept.is_some_and(|kept| !kept.contains(triangle)) {
                    continue;
                }
                for vertex in vertices(p, &mut indices, triangle)? {
                    blocks.push(vertex / per_block);
                    let w = (vertex / 64) as usize;
                    if w != word {
                        if bits != 0 {
                            used[word].fetch_or(bits, Relaxed);
                        }
                        (word, bits) = (w, 0);
                    }
                    bits |= 1 << (vertex % 64);
                }
            }
            if bits != 0 {
                used[word].fetch_or(bits, Relaxed);
            }
            blocks.sort_unstable();
            blocks.dedup();
            if blocks.len() <= cache_blocks {
                local.fetch_add(1, Relaxed);
            }
            Ok(())
        },
    )?;
    Ok(match marked {
        Ran::All(_) => Ok((used, local.into_inner())),
        Ran::Failed(failure) => Err(failure),
    })
}

/// In a subset bake: how many of `p`'s triangle chunks holding a captured triangle touch at
/// most `cache_blocks` distinct blocks of `per_block` store slots, and how many chunks hold
/// one.
fn slot_locality(
    source: &StreamSource,
    p: &ScenePrimitive<'_>,
    buffer_bytes: usize,
    workers: usize,
    (per_block, cache_blocks): (u64, usize),
    slots: Slots<'_>,
    poll: &mut dyn FnMut() -> Result<()>,
) -> Result<std::result::Result<(u64, u64), JobError>> {
    let counted = run(
        p.triangles.div_ceil(CHUNK_TRIANGLES),
        workers,
        poll,
        |chunk, stop| {
            let start = chunk * CHUNK_TRIANGLES;
            let range = start..(start + CHUNK_TRIANGLES).min(p.triangles);
            if none_kept(Some(slots), &range) {
                return Ok((0, 0));
            }
            let mut indices = p.indices.map(|v| source.reader(&v, buffer_bytes));
            let mut blocks = Vec::with_capacity(3 * CHUNK_TRIANGLES);
            for triangle in range {
                if triangle.is_multiple_of(1024) && stop.load(Relaxed) {
                    return Err(JobError::new(JobErrorKind::Cancelled, "assembly stopped"));
                }
                if !slots.triangles.contains(triangle) {
                    continue;
                }
                for vertex in vertices(p, &mut indices, triangle)? {
                    blocks.push(vertex_slot(Some(slots), vertex)? / per_block);
                }
            }
            blocks.sort_unstable();
            blocks.dedup();
            Ok((u64::from(blocks.len() <= cache_blocks), 1))
        },
    )?;
    Ok(match counted {
        Ran::All(counts) => Ok(counts
            .iter()
            .fold((0, 0), |(l, a), (local, active)| (l + local, a + active))),
        Ran::Failed(failure) => Err(failure),
    })
}

/// Vertices decoded exactly as the corner join decoded them.
struct Decoder<'a> {
    source: &'a StreamSource,
    p: &'a ScenePrimitive<'a>,
    frame: &'a mut SourceFrame,
    buffer_bytes: usize,
}

/// Runs `task(chunk, frame, check)` over `chunks` with a source frame; a task calls `check`
/// as it goes and stops on its error. A local or portable frame is copied for each task
/// ([`SourceFrame::portable`]), so the chunks run on up to `workers` threads and `check`
/// reads their stop flag. A native frame holds GDAL/PROJ handles that must stay on the
/// thread that made them, so its chunks run in order on the calling thread with that frame,
/// and `check` is `poll` itself: a `poll` error is returned as such, not as a task failure.
/// Either way every vertex is mapped by the same deterministic arithmetic.
pub(super) fn decode_chunks<T: Send>(
    frame: &mut SourceFrame,
    chunks: std::ops::Range<usize>,
    workers: usize,
    poll: &mut dyn FnMut() -> Result<()>,
    task: impl Fn(usize, &mut SourceFrame, &mut dyn FnMut() -> Result<()>) -> Result<T> + Sync,
) -> Result<Ran<T>> {
    let first = chunks.start;
    if let Some(portable) = frame.portable() {
        return run(chunks.len(), workers, poll, |i, stop| {
            task(first + i, &mut portable.frame(), &mut || {
                if stop.load(Relaxed) {
                    return Err(stopped());
                }
                Ok(())
            })
        });
    }
    decode_inline(frame, chunks, poll, task)
}

/// [`decode_chunks`] for a native frame: the chunks in order on the calling thread, paced as
/// [`run`] paces its tasks, with `poll` as each task's `check`.
fn decode_inline<T>(
    frame: &mut SourceFrame,
    chunks: std::ops::Range<usize>,
    poll: &mut dyn FnMut() -> Result<()>,
    task: impl Fn(usize, &mut SourceFrame, &mut dyn FnMut() -> Result<()>) -> Result<T>,
) -> Result<Ran<T>> {
    let mut found = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        poll()?;
        crate::test_hooks::pace("mesh.capture-task");
        let mut polled = None;
        let result = task(chunk, frame, &mut || {
            poll().map_err(|error| {
                polled = Some(error);
                stopped()
            })
        });
        if let Some(error) = polled {
            return Err(error);
        }
        match result {
            Ok(value) => found.push(value),
            Err(error) => return Ok(Ran::Failed(error)),
        }
    }
    poll()?;
    Ok(Ran::All(found))
}

fn stopped() -> JobError {
    JobError::new(JobErrorKind::Cancelled, "assembly stopped")
}

/// The boxes of a decoded range: mapped f32 and f64 source.
#[derive(Clone, Copy)]
struct Boxes {
    mapped: Bounds,
    source: [[f64; 3]; 2],
}
impl Boxes {
    const EMPTY: Self = Self {
        mapped: Bounds::EMPTY,
        source: [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]],
    };
    fn union(&mut self, other: &Self) {
        self.mapped.union(&other.mapped);
        for axis in 0..3 {
            self.source[0][axis] = self.source[0][axis].min(other.source[0][axis]);
            self.source[1][axis] = self.source[1][axis].max(other.source[1][axis]);
        }
    }
}

impl Decoder<'_> {
    /// Decodes the used vertices of `range` into `out` (records under `layout`, unused ones
    /// zero), running `check` every 4,096 vertices and stopping on its error. With `ranks`
    /// (a subset bake), `out` holds the used vertices only, at their ranks from the range's
    /// first.
    fn decode(
        &mut self,
        used: &[AtomicU64],
        range: std::ops::Range<usize>,
        layout: Layout,
        out: &mut [u8],
        ranks: Option<&Ranks>,
        check: &mut dyn FnMut() -> Result<()>,
    ) -> Result<Boxes> {
        let p = self.p;
        let source = self.source;
        let size = DecodedVertex::size_in(layout);
        let mut positions = source.reader(&p.position, self.buffer_bytes);
        let mut normals = p.normal.map(|v| source.reader(&v, self.buffer_bytes));
        let mut tangents = p.tangent.map(|v| source.reader(&v, self.buffer_bytes));
        let mut uvs = p
            .texcoords
            .map(|v| v.map(|v| source.reader(&v, self.buffer_bytes)));
        let mut colors = p.color.map(|v| source.reader(&v, self.buffer_bytes));
        let want_jacobian = normals.is_some() || tangents.is_some();
        let mut boxes = Boxes::EMPTY;
        let slot = |v: usize| ranks.map_or(v, |ranks| ranks.rank(v));
        let start = slot(range.start);
        for vertex in range {
            if vertex.is_multiple_of(4096) {
                check()?;
            }
            if used[vertex / 64].load(Relaxed) & (1 << (vertex % 64)) == 0 {
                continue;
            }
            let world = source_position(p, &mut positions, vertex as u64)?;
            let (position, jacobian) = self.frame.vertex(world, want_jacobian)?;
            let position = canonical(position);
            boxes.mapped.include(position);
            for axis in 0..3 {
                boxes.source[0][axis] = boxes.source[0][axis].min(world[axis]);
                boxes.source[1][axis] = boxes.source[1][axis].max(world[axis]);
            }
            let attributes = read_attributes(
                p,
                vertex as u64,
                jacobian.as_ref(),
                &mut normals,
                &mut tangents,
                &mut uvs,
                &mut colors,
            )?;
            let at = (slot(vertex) - start) * size;
            DecodedVertex {
                position,
                attributes,
            }
            .encode_in(layout, &mut out[at..at + size]);
        }
        Ok(boxes)
    }
}

/// A decoded primitive: its store, its boxes and, in a subset bake, its kept-vertex ranks.
type Decoded = (Store, Boxes, Option<Ranks>);

/// Decodes `p`'s used vertices into a store: in memory when `memory` allows, else spilled.
/// Returns the first failure (in time) of a parallel task as the inner error: the caller
/// reruns serially for the first failing vertex in order.
#[allow(clippy::too_many_arguments)]
fn decode_primitive(
    source: &StreamSource,
    p: &ScenePrimitive<'_>,
    frame: &mut SourceFrame,
    scratch: &Scratch,
    layout: Layout,
    buffer_bytes: usize,
    workers: usize,
    in_memory: bool,
    (cache_blocks, sorter, progress): (usize, SortConfig, &StepProgress<'_>),
    kept: Option<&Ranks>,
    poll: &mut dyn FnMut() -> Result<()>,
) -> Result<std::result::Result<Decoded, JobError>> {
    let size = DecodedVertex::size_in(layout);
    let per_block = crate::spill::block_records(size, buffer_bytes);
    let (used, local) = match used_vertices(
        source,
        p,
        buffer_bytes,
        workers,
        (per_block, cache_blocks),
        kept,
        poll,
    )? {
        Ok(found) => found,
        Err(failure) => return Ok(Err(failure)),
    };
    // A subset bake stores the used vertices only, by rank.
    let ranks = kept.map(|_| Ranks::new(used.iter().map(|w| w.load(Relaxed)).collect()));
    let slot = |v: usize| ranks.as_ref().map_or(v, |ranks| ranks.rank(v));
    let count = p.position.count;
    let chunks = count.div_ceil(CHUNK_VERTICES);
    // Store slots of chunk `chunk`'s vertices.
    let span = |chunk: usize| {
        let start = chunk * CHUNK_VERTICES;
        slot((start + CHUNK_VERTICES).min(count)) - slot(start)
    };
    let mut boxes = Boxes::EMPTY;
    let decode_chunk = |chunk: usize,
                        frame: &mut SourceFrame,
                        out: &mut [u8],
                        check: &mut dyn FnMut() -> Result<()>|
     -> Result<Boxes> {
        let start = chunk * CHUNK_VERTICES;
        let end = (start + CHUNK_VERTICES).min(count);
        let mut decoder = Decoder {
            source,
            p,
            frame,
            buffer_bytes,
        };
        decoder.decode(&used, start..end, layout, out, ranks.as_ref(), check)
    };
    if in_memory {
        let mut bytes = vec![0u8; slot(count).checked_mul(size).ok_or_else(overflow)?];
        let mut rest = &mut bytes[..];
        let mut slots: Vec<Mutex<&mut [u8]>> = Vec::with_capacity(chunks);
        for chunk in 0..chunks {
            let (head, tail) = std::mem::take(&mut rest).split_at_mut(span(chunk) * size);
            slots.push(Mutex::new(head));
            rest = tail;
        }
        let ran = decode_chunks(frame, 0..chunks, workers, poll, |chunk, frame, check| {
            let mut slot = slots[chunk].lock().unwrap_or_else(|e| e.into_inner());
            decode_chunk(chunk, frame, &mut slot, check)
        })?;
        drop(slots);
        let found = match ran {
            Ran::All(found) => found,
            Ran::Failed(failure) => return Ok(Err(failure)),
        };
        found.iter().for_each(|b| boxes.union(b));
        return Ok(Ok((Store::Memory(bytes), boxes, ranks)));
    }
    // Spilled: waves of two chunks per worker, written in order.
    let mut writer = SequenceWriter::<DecodedVertex>::compressed(
        scratch,
        "decoded-vertices",
        buffer_bytes,
        layout,
    )?;
    let wave = 2 * workers.max(1);
    for first in (0..chunks).step_by(wave) {
        let last = (first + wave).min(chunks);
        let ran = decode_chunks(frame, first..last, workers, poll, |chunk, frame, check| {
            let len = span(chunk) * size;
            let mut out = vec![0u8; len];
            let found = decode_chunk(chunk, frame, &mut out, check)?;
            Ok((out, found))
        })?;
        let found = match ran {
            Ran::All(found) => found,
            Ran::Failed(failure) => return Ok(Err(failure)),
        };
        for (bytes, found) in found {
            boxes.union(&found);
            for record in bytes.chunks_exact(size) {
                writer.push_encoded(record)?;
            }
        }
    }
    let spilled = writer.finish()?;
    let slots = kept.zip(ranks.as_ref()).map(|(triangles, vertices)| Slots {
        triangles,
        vertices,
    });
    let (local, chunks) = match slots {
        None => (local, p.triangles.div_ceil(CHUNK_TRIANGLES) as u64),
        Some(slots) => match slot_locality(
            source,
            p,
            buffer_bytes,
            workers,
            (per_block, cache_blocks),
            slots,
            poll,
        )? {
            Ok(counted) => counted,
            Err(failure) => return Ok(Err(failure)),
        },
    };
    // Too little locality for the block caches: place every corner by sorting instead.
    if needs_route(local, chunks) {
        let placed = place_corners(
            source, p, &spilled, scratch, layout, sorter, progress, slots, poll,
        )?;
        return Ok(Ok((Store::Placed(placed), boxes, ranks)));
    }
    Ok(Ok((Store::Spilled(spilled), boxes, ranks)))
}

/// Test seam: faults injected into the assembly of captures whose scratch lies under a
/// registered directory.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fault {
    /// Writing a staged run fails.
    Staging,
    /// Sorting bucket 1 fails, while the worker holding bucket 0 leaves at the stop without
    /// sorting it: the writer, waiting on bucket 0, must report bucket 1's error.
    Bucket,
}
#[cfg(test)]
pub(crate) static FAULTS: Mutex<Vec<(std::path::PathBuf, Fault)>> = Mutex::new(Vec::new());
#[cfg(test)]
fn injected(scratch: &Scratch, fault: Fault) -> Result<()> {
    let faults = FAULTS.lock().unwrap_or_else(|e| e.into_inner());
    if faults
        .iter()
        .any(|(dir, f)| *f == fault && scratch.path().starts_with(dir))
    {
        return Err(JobError::new(
            JobErrorKind::ResourceLimit,
            format!("injected {fault:?} failure"),
        ));
    }
    Ok(())
}

/// Whether a spilled primitive's corners are placed by sorting: fewer than
/// [`LOCAL_CHUNKS_PER_32`] in 32 of its `chunks` triangle chunks touch at most a cache's worth
/// of vertex blocks (`local` of them do).
fn needs_route(local: u64, chunks: u64) -> bool {
    local.saturating_mul(32) < chunks.saturating_mul(LOCAL_CHUNKS_PER_32)
}

/// A corner of a routed primitive, sorted by vertex to meet the decoded vertices in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CornerVertex {
    vertex: u64,
    triangle: u64,
    corner: u8,
}
impl Record for CornerVertex {
    const SIZE: usize = 17;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.vertex.to_le_bytes());
        out[8..16].copy_from_slice(&self.triangle.to_le_bytes());
        out[16] = self.corner;
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            vertex: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            triangle: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            corner: bytes[16],
        }
    }
}

/// A routed corner with its decoded vertex, sorted back to (triangle, corner).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlacedCorner {
    triangle: u64,
    corner: u8,
    vertex: DecodedVertex,
}
impl PartialEq for PlacedCorner {
    fn eq(&self, other: &Self) -> bool {
        (self.triangle, self.corner) == (other.triangle, other.corner)
    }
}
impl Eq for PlacedCorner {}
impl PartialOrd for PlacedCorner {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PlacedCorner {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.triangle, self.corner).cmp(&(other.triangle, other.corner))
    }
}
impl Record for PlacedCorner {
    const SIZE: usize = 9 + DecodedVertex::SIZE;
    fn encode(&self, out: &mut [u8]) {
        self.encode_in(Layout::FULL, out);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self::decode_in(Layout::FULL, bytes)
    }
    fn size_in(layout: Layout) -> usize {
        9 + DecodedVertex::size_in(layout)
    }
    fn encode_in(&self, layout: Layout, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.triangle.to_le_bytes());
        out[8] = self.corner;
        self.vertex.encode_in(layout, &mut out[9..]);
    }
    fn decode_in(layout: Layout, bytes: &[u8]) -> Self {
        Self {
            triangle: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            corner: bytes[8],
            vertex: DecodedVertex::decode_in(layout, &bytes[9..]),
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Test seam: set on the capturing thread once a primitive's corners are being routed.
    pub(crate) static ROUTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A routed triangle's three decoded corners, in triangle order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TriangleCorners([DecodedVertex; 3]);
impl Record for TriangleCorners {
    const SIZE: usize = 3 * DecodedVertex::SIZE;
    fn encode(&self, out: &mut [u8]) {
        self.encode_in(Layout::FULL, out);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self::decode_in(Layout::FULL, bytes)
    }
    fn size_in(layout: Layout) -> usize {
        3 * DecodedVertex::size_in(layout)
    }
    fn encode_in(&self, layout: Layout, out: &mut [u8]) {
        let size = DecodedVertex::size_in(layout);
        for (c, v) in self.0.iter().enumerate() {
            v.encode_in(layout, &mut out[c * size..(c + 1) * size]);
        }
    }
    fn decode_in(layout: Layout, bytes: &[u8]) -> Self {
        let size = DecodedVertex::size_in(layout);
        Self(std::array::from_fn(|c| {
            DecodedVertex::decode_in(layout, &bytes[c * size..(c + 1) * size])
        }))
    }
}

/// The routed placement of `p`'s corners: every corner sorted by vertex, joined in order with
/// the decoded vertices, sorted back to (triangle, corner) and written per triangle. Two
/// external sorts of the primitive's corners, like the corner join the assembly replaced.
#[allow(clippy::too_many_arguments)]
fn place_corners(
    source: &StreamSource,
    p: &ScenePrimitive<'_>,
    decoded: &Sequence<DecodedVertex>,
    scratch: &Scratch,
    layout: Layout,
    sorter: SortConfig,
    progress: &StepProgress<'_>,
    slots: Option<Slots<'_>>,
    poll: &mut dyn FnMut() -> Result<()>,
) -> Result<Sequence<TriangleCorners>> {
    #[cfg(test)]
    ROUTING.with(|routing| routing.set(true));
    let mut by_vertex = ExternalSorter::<CornerVertex>::new(scratch, sorter)?.observed(progress);
    let mut indices = p.indices.map(|v| source.reader(&v, sorter.buffer_bytes));
    for triangle in 0..p.triangles {
        if triangle.is_multiple_of(4096) {
            crate::test_hooks::pace("mesh.assembly-route");
            poll()?;
        }
        if !kept(slots, triangle) {
            continue;
        }
        let slot = triangle_slot(slots, triangle);
        for (corner, vertex) in vertices(p, &mut indices, triangle)?.into_iter().enumerate() {
            by_vertex.push(CornerVertex {
                vertex: vertex_slot(slots, vertex)?,
                triangle: slot as u64,
                corner: corner as u8,
            })?;
        }
    }
    let mut placed =
        ExternalSorter::<PlacedCorner>::with_layout(scratch, sorter, layout)?.observed(progress);
    let mut vertices_in_order = decoded.reader()?;
    let (mut at, mut current) = (0u64, None::<DecodedVertex>);
    for (i, corner) in by_vertex.finish()?.enumerate() {
        let corner = corner?;
        if i.is_multiple_of(4096) {
            poll()?;
        }
        while current.is_none() || at <= corner.vertex {
            current = Some(
                vertices_in_order
                    .next()
                    .transpose()?
                    .ok_or_else(|| invariant("routed corner past the decoded vertices"))?,
            );
            at += 1;
        }
        if at != corner.vertex + 1 {
            return Err(invariant("routed corners out of vertex order"));
        }
        placed.push(PlacedCorner {
            triangle: corner.triangle,
            corner: corner.corner,
            vertex: current.expect("a vertex was read"),
        })?;
    }
    drop(vertices_in_order);
    let mut writer = SequenceWriter::<TriangleCorners>::compressed(
        scratch,
        "placed-corners",
        sorter.buffer_bytes,
        layout,
    )?;
    let mut corners = [DecodedVertex::default(); 3];
    let mut expected = (0u64, 0u8);
    for (i, row) in placed.finish()?.enumerate() {
        let row = row?;
        if i.is_multiple_of(4096) {
            poll()?;
        }
        if (row.triangle, row.corner) != expected {
            return Err(invariant(
                "routed corners do not cover every triangle corner",
            ));
        }
        corners[row.corner as usize] = row.vertex;
        expected = if row.corner == 2 {
            writer.push(&TriangleCorners(corners))?;
            (row.triangle + 1, 0)
        } else {
            (row.triangle, row.corner + 1)
        };
    }
    if expected != (triangle_slot(slots, p.triangles) as u64, 0) {
        return Err(invariant(
            "routed corners do not cover every triangle corner",
        ));
    }
    Ok(writer.finish()?)
}

/// The first failing vertex of `p` in vertex order, as the corner join reported it.
/// The first failing vertex of `p` in vertex order, as the corner join reported it, or
/// `None` when every vertex decodes alone. Checks `poll` as it goes.
fn first_vertex_failure(
    source: &StreamSource,
    p: &ScenePrimitive<'_>,
    frame: &mut SourceFrame,
    layout: Layout,
    buffer_bytes: usize,
    kept: Option<&Ranks>,
    poll: &mut dyn FnMut() -> Result<()>,
) -> Result<Option<JobError>> {
    let used: Vec<AtomicU64> = (0..p.position.count.div_ceil(64))
        .map(|_| AtomicU64::new(0))
        .collect();
    let mut indices = p.indices.map(|v| source.reader(&v, buffer_bytes));
    for triangle in 0..p.triangles {
        if triangle.is_multiple_of(4096) {
            poll()?;
        }
        if kept.is_some_and(|kept| !kept.contains(triangle)) {
            continue;
        }
        match vertices(p, &mut indices, triangle) {
            Ok(vs) => vs.iter().for_each(|&v| {
                used[(v / 64) as usize].fetch_or(1 << (v % 64), Relaxed);
            }),
            Err(error) => return Ok(Some(error)),
        }
    }
    let size = DecodedVertex::size_in(layout);
    let mut decoder = Decoder {
        source,
        p,
        frame,
        buffer_bytes,
    };
    let mut out = vec![0u8; size];
    for vertex in 0..p.position.count {
        if vertex.is_multiple_of(4096) {
            poll()?;
        }
        if let Err(error) = decoder.decode(
            &used,
            vertex..vertex + 1,
            layout,
            &mut out,
            None,
            &mut || Ok(()),
        ) {
            return Ok(Some(error));
        }
    }
    Ok(None)
}

/// The prefix cell of a Morton key.
fn cell(key: u64) -> usize {
    (key >> (3 * u32::from(KEY_DEPTH - PREFIX_DEPTH))) as usize
}

/// Morton key, node, mesh, primitive, triangle and expanded ordinal.
type SortKey = (u64, u32, u32, u32, u64, u64);

/// Sort key of an encoded record: Morton key, provenance, expanded ordinal
/// ([`FullTriangle`]'s order).
fn sort_key(bytes: &[u8]) -> SortKey {
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    (
        u64_at(0),
        u32_at(8),
        u32_at(12),
        u32_at(16),
        u64_at(20),
        u64_at(93),
    )
}

/// One triangle, built exactly as the corner-sort capture built it.
#[allow(clippy::too_many_arguments)]
fn triangle(
    p: &ScenePrimitive<'_>,
    ordinal: u64,
    bytes: u32,
    store: &mut StoreReader<'_>,
    layout: Layout,
    cube: &Cube,
    indices: &mut Option<crate::mesh_large::input::ElementReader<'_>>,
    t: usize,
) -> Result<FullTriangle> {
    let verts = vertices(p, indices, t)?;
    let mut positions = [[0f32; 3]; 3];
    let mut corners = [CornerAttributes::default(); 3];
    for (c, v) in store.corners(layout, t, verts)?.iter().enumerate() {
        positions[c] = v.position;
        corners[c] = v.attributes;
    }
    let degenerate = positions[0] == positions[1]
        || positions[0] == positions[2]
        || positions[1] == positions[2];
    Ok(FullTriangle {
        triangle: TriangleRecord {
            key: cube.key(&positions),
            source: Provenance {
                node: p.node,
                mesh: p.mesh,
                primitive: p.primitive,
                triangle: t as u64,
            },
            vertices: verts,
            positions,
            bytes,
            degenerate,
        },
        expanded_ordinal: ordinal,
        material: p.material.unwrap_or(u32::MAX),
        flags: flags(p),
        corners,
    })
}

/// A compressed run of one bucket's records in a staging file. Runs are listed in scratch
/// (their number grows with the records) and sorted by bucket before the sort step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct StagedRun {
    bucket: u32,
    offset: u64,
    group: u32,
    stored_bytes: u32,
    records: u32,
    stored: bool,
}
impl Record for StagedRun {
    const SIZE: usize = 25;
    fn encode(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.bucket.to_le_bytes());
        out[4..12].copy_from_slice(&self.offset.to_le_bytes());
        out[12..16].copy_from_slice(&self.group.to_le_bytes());
        out[16..20].copy_from_slice(&self.stored_bytes.to_le_bytes());
        out[20..24].copy_from_slice(&self.records.to_le_bytes());
        out[24] = u8::from(self.stored);
    }
    fn decode(bytes: &[u8]) -> Self {
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        Self {
            bucket: u32_at(0),
            offset: u64::from_le_bytes(bytes[4..12].try_into().unwrap()),
            group: u32_at(12),
            stored_bytes: u32_at(16),
            records: u32_at(20),
            stored: bytes[24] != 0,
        }
    }
}

/// The rank-ordered full-triangle sequence of `source` (see the module notes). `scan` is the
/// preflight scan; with a source CRS its box is replaced by the mapped one, as the corner join
/// did. Validation has run.
///
/// With a `subset` (a subset bake), only its kept triangles are captured and only the
/// vertices they use are decoded and stored; `scan` already counts the kept triangles and
/// holds the whole source's box, which keys them as a full bake would.
#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble(
    source: &StreamSource,
    frame: &mut SourceFrame,
    scratch: &Scratch,
    layout: Layout,
    scan: &mut SourceScan,
    plan: AssemblyPlan,
    workers: usize,
    sorter: SortConfig,
    progress: &StepProgress<'_>,
    subset: Option<&Selection>,
) -> Result<(Sequence<FullTriangle>, AssemblyReport)> {
    let buffer_bytes = sorter.buffer_bytes;
    let mut report = AssemblyReport {
        plan: Some(plan),
        ..AssemblyReport::default()
    };
    let primitives: Vec<ScenePrimitive<'_>> = source.document.scene_primitives().collect();
    // Only a primitive's own attributes are ever nonzero, so a layout holding them all drops
    // nothing but zeros. Checked once per primitive, never per record.
    if primitives.iter().any(|p| flags(p) & !layout.0 != 0) {
        return Err(invariant(
            "attribute layout omits a selected primitive's attribute",
        ));
    }
    let n = scan.triangles;
    let stop_check = || -> Result<()> {
        crate::test_hooks::pace("mesh.assembly");
        scratch.check_stop().map_err(Into::into)
    };
    // 1. Decode, primitive by primitive.
    let total_vertices: u64 = primitives.iter().map(|p| p.position.count as u64).sum();
    progress.begin(Step::Corners, total_vertices)?;
    let size = DecodedVertex::size_in(layout) as u64;
    let mut allowance = plan.vertex_allowance_bytes;
    let mut stores = Vec::with_capacity(primitives.len());
    // A subset bake's kept-vertex ranks, per primitive.
    let mut vertex_ranks = Vec::with_capacity(subset.map_or(0, |_| primitives.len()));
    let mut boxes = Boxes::EMPTY;
    let mut done = 0u64;
    for (i, p) in primitives.iter().enumerate() {
        let kept = subset.map(|s| &s.primitives[i]);
        if kept.is_some_and(|kept| kept.vertices == 0) {
            // No captured triangle: nothing to decode or store.
            stores.push(Store::Memory(Vec::new()));
            vertex_ranks.push(Ranks::new(Vec::new()));
            report.excluded_primitives += 1;
            done += p.position.count as u64;
            progress.advance(done)?;
            continue;
        }
        let bytes = kept
            .map_or(p.position.count as u64, |kept| kept.vertices)
            .checked_mul(size)
            .ok_or_else(overflow)?;
        let in_memory = bytes <= allowance;
        let decoded = decode_primitive(
            source,
            p,
            frame,
            scratch,
            layout,
            buffer_bytes,
            workers,
            in_memory,
            (plan.cache_blocks, sorter, progress),
            kept.map(|kept| &kept.triangles),
            &mut || {
                stop_check()?;
                progress.advance(done)
            },
        )?;
        let (store, found, ranks) = match decoded {
            Ok(decoded) => decoded,
            // The first failing vertex in order, as before; otherwise (a read or scratch
            // error) the task's own failure.
            Err(failure) => {
                let first = first_vertex_failure(
                    source,
                    p,
                    frame,
                    layout,
                    buffer_bytes,
                    kept.map(|kept| &kept.triangles),
                    &mut || {
                        stop_check()?;
                        progress.check()
                    },
                )?;
                return Err(first.unwrap_or(failure));
            }
        };
        vertex_ranks.extend(ranks);
        if in_memory {
            allowance -= bytes;
            report.resident_primitives += 1;
        } else {
            report.spilled_primitives += 1;
            if matches!(store, Store::Placed(_)) {
                report.routed_primitives += 1;
            }
        }
        boxes.union(&found);
        stores.push(store);
        done += p.position.count as u64;
        progress.advance(done)?;
    }
    progress.end()?;
    if vertex_ranks.len() != subset.map_or(0, |_| primitives.len()) {
        return Err(invariant(
            "subset vertex ranks disagree with the primitives",
        ));
    }
    // A subset's view of primitive `i`.
    let slots_of = |i: usize| {
        subset
            .zip(vertex_ranks.get(i))
            .map(|(subset, vertices)| Slots {
                triangles: &subset.primitives[i].triangles,
                vertices,
            })
    };
    let expected = frame.source_bounds();
    match subset {
        // The kept vertices are the selection's, and the cube stays the whole source's.
        Some(subset) => subset.check_decoded(boxes.source, boxes.mapped)?,
        None => {
            if let Some(expected) = expected {
                if boxes.source != expected {
                    return Err(invalid(
                        "mesh source geometry changed between preflight and capture",
                    ));
                }
                scan.bounds = boxes.mapped;
            }
        }
    }
    let cube = Cube::around(&scan.bounds);
    // Tasks over every (primitive, chunk), in input order.
    let tasks: Vec<(usize, usize)> = primitives
        .iter()
        .enumerate()
        .flat_map(|(i, p)| (0..p.triangles.div_ceil(CHUNK_TRIANGLES)).map(move |c| (i, c)))
        .collect();
    let range = |&(i, c): &(usize, usize)| {
        let start = c * CHUNK_TRIANGLES;
        start..(start + CHUNK_TRIANGLES).min(primitives[i].triangles)
    };
    let spilled_blocks = AtomicU64::new(0);
    // 2. Histogram, and the checks the corner-sort capture made at its end.
    progress.begin(Step::Positions, n)?;
    let counts: Vec<AtomicU64> = (0..CELLS).map(|_| AtomicU64::new(0)).collect();
    let counted = AtomicU64::new(0);
    let ran = run(
        tasks.len(),
        workers,
        &mut || {
            stop_check()?;
            progress.advance(counted.load(Relaxed))
        },
        |k, stop| {
            if stop.load(Relaxed) {
                return Err(JobError::new(JobErrorKind::Cancelled, "assembly stopped"));
            }
            let (i, _) = tasks[k];
            let p = &primitives[i];
            let slots = slots_of(i);
            if none_kept(slots, &range(&tasks[k])) {
                return Ok(Bounds::EMPTY);
            }
            let mut store =
                stores[i].reader(slots, plan.cache_blocks, range(&tasks[k]), buffer_bytes)?;
            let mut indices = p.indices.map(|v| source.reader(&v, buffer_bytes));
            let mut bounds = Bounds::EMPTY;
            let mut cells = Vec::with_capacity(CHUNK_TRIANGLES);
            for t in range(&tasks[k]) {
                if !kept(slots, t) {
                    continue;
                }
                let verts = vertices(p, &mut indices, t)?;
                let mut positions = [[0f32; 3]; 3];
                for (c, v) in store.corners(layout, t, verts)?.iter().enumerate() {
                    positions[c] = v.position;
                    bounds.include(positions[c]);
                }
                cells.push(cell(cube.key(&positions)));
            }
            cells.sort_unstable();
            for same in cells.chunk_by(|a, b| a == b) {
                counts[same[0]].fetch_add(same.len() as u64, Relaxed);
            }
            spilled_blocks.fetch_add(store.decoded_blocks(), Relaxed);
            counted.fetch_add(cells.len() as u64, Relaxed);
            Ok(bounds)
        },
    )?;
    let found = match ran {
        Ran::All(found) => found,
        Ran::Failed(failure) => return Err(failure),
    };
    let mut bounds = Bounds::EMPTY;
    found.iter().for_each(|b| bounds.union(b));
    let mut count = 0u64;
    let mut geometry_bytes = 0u64;
    for (i, p) in primitives.iter().enumerate() {
        let triangles = subset.map_or(p.triangles, |s| s.primitives[i].triangles.ones()) as u64;
        count = count.checked_add(triangles).ok_or_else(overflow)?;
        geometry_bytes = geometry_bytes
            .checked_add(
                triangles
                    .checked_mul(u64::from(triangle_bytes(p)))
                    .ok_or_else(overflow)?,
            )
            .ok_or_else(overflow)?;
    }
    progress.end()?;
    source.verify_unchanged()?;
    let expected = subset.map_or(scan.bounds, |subset| subset.bounds);
    if count != scan.triangles || geometry_bytes != scan.geometry_bytes || bounds != expected {
        return Err(invalid(
            "mesh source geometry changed between preflight and capture",
        ));
    }
    // 3. Buckets: contiguous cell ranges of at most `cap` records; a heavier cell is its own.
    let record = FullTriangle::size_in(layout);
    let cap = (plan.bucket_bytes / record as u64).max(1);
    let mut bucket_of = vec![0u32; CELLS];
    let mut starts: Vec<u64> = vec![0];
    let (mut filled, mut total) = (0u64, 0u64);
    for (c, k) in counts.iter().enumerate() {
        let k = k.load(Relaxed);
        if k == 0 {
            bucket_of[c] = (starts.len() - 1) as u32;
            continue;
        }
        if filled > 0 && (filled + k > cap || k > cap) {
            starts.push(total);
            filled = 0;
        }
        bucket_of[c] = (starts.len() - 1) as u32;
        filled += k;
        total += k;
    }
    drop(counts);
    if total != n {
        return Err(invariant(
            "assembly histogram does not count every triangle",
        ));
    }
    starts.push(total);
    let buckets = starts.len() - 1;
    let records_of = |b: usize| starts[b + 1] - starts[b];
    let heavy = |b: usize| records_of(b) > cap;
    report.buckets = buckets as u64;
    report.heavy_buckets = (0..buckets).filter(|&b| heavy(b)).count() as u64;
    report.largest_bucket_records = (0..buckets).map(records_of).max().unwrap_or(0);
    // Staging files: STAGING_GROUPS of consecutive buckets, and one of its own for each
    // heavy bucket holding more than 1/OWN_FILES of the records (at most OWN_FILES of them),
    // removed as soon as its external sort holds its records.
    let own_file = |b: usize| heavy(b) && records_of(b) > n / OWN_FILES as u64;
    let mut group_of = vec![0usize; buckets];
    let mut groups = 0usize;
    let ordinary = (0..buckets).filter(|&b| !own_file(b)).count().max(1);
    let mut seen = 0usize;
    let mut last_group = usize::MAX;
    for (b, group) in group_of.iter_mut().enumerate() {
        if own_file(b) {
            *group = groups;
            groups += 1;
            last_group = usize::MAX;
        } else {
            let g = seen * STAGING_GROUPS / ordinary;
            seen += 1;
            if g != last_group {
                last_group = g;
                groups += 1;
            }
            *group = groups - 1;
        }
    }
    let stagings: Vec<Mutex<Option<ByteWriter>>> = (0..groups)
        .map(|_| {
            ByteWriter::new(
                scratch,
                "assembly-staging",
                buffer_bytes.min(STAGING_BUFFER),
            )
            .map(|w| Mutex::new(Some(w)))
        })
        .collect::<std::result::Result<_, _>>()?;
    // 4. Distribute.
    progress.begin(Step::Triangles, n)?;
    let flush = (plan.gather_bytes / buckets as u64).clamp(GATHER_MIN, GATHER_MAX) as usize;
    let gathering: Vec<Mutex<Vec<u8>>> = (0..buckets).map(|_| Mutex::new(Vec::new())).collect();
    let runs = Mutex::new(SequenceWriter::<StagedRun>::new(
        scratch,
        "staging-runs",
        buffer_bytes,
    )?);
    let staged = |b: usize, raw: &[u8], scratch_buffer: &mut Vec<u8>| -> Result<()> {
        #[cfg(test)]
        injected(scratch, Fault::Staging)?;
        let encoded = crate::spill::encode_block(raw, scratch_buffer);
        let group = group_of[b];
        let mut writer = stagings[group].lock().unwrap_or_else(|e| e.into_inner());
        let writer = writer
            .as_mut()
            .ok_or_else(|| invariant("staging file closed"))?;
        let offset = writer.len();
        writer.append(&encoded.bytes)?;
        runs.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(&StagedRun {
                bucket: b as u32,
                offset,
                group: group as u32,
                stored_bytes: encoded.bytes.len() as u32,
                records: (raw.len() / record) as u32,
                stored: encoded.stored,
            })?;
        Ok(())
    };
    let distributed = AtomicU64::new(0);
    let ran = run(
        tasks.len(),
        workers,
        &mut || {
            stop_check()?;
            progress.advance(distributed.load(Relaxed))
        },
        |k, stop| {
            if stop.load(Relaxed) {
                return Err(JobError::new(JobErrorKind::Cancelled, "assembly stopped"));
            }
            let (i, _) = tasks[k];
            let p = &primitives[i];
            let ordinal = i as u64;
            let bytes = triangle_bytes(p);
            let slots = slots_of(i);
            if none_kept(slots, &range(&tasks[k])) {
                return Ok(());
            }
            let mut store =
                stores[i].reader(slots, plan.cache_blocks, range(&tasks[k]), buffer_bytes)?;
            let mut indices = p.indices.map(|v| source.reader(&v, buffer_bytes));
            let mut rows: Vec<(u32, FullTriangle)> = Vec::with_capacity(CHUNK_TRIANGLES);
            for t in range(&tasks[k]) {
                if !kept(slots, t) {
                    continue;
                }
                let row = triangle(
                    p,
                    ordinal,
                    bytes,
                    &mut store,
                    layout,
                    &cube,
                    &mut indices,
                    t,
                )?;
                rows.push((bucket_of[cell(row.triangle.key)], row));
            }
            rows.sort_by_key(|r| r.0);
            let mut encoded = vec![0u8; record];
            let mut compressed = Vec::new();
            for group in rows.chunk_by(|a, b| a.0 == b.0) {
                let b = group[0].0 as usize;
                let full = {
                    let mut buffer = gathering[b].lock().unwrap_or_else(|e| e.into_inner());
                    for (_, row) in group {
                        row.encode_in(layout, &mut encoded);
                        buffer.extend_from_slice(&encoded);
                    }
                    (buffer.len() >= flush).then(|| std::mem::take(&mut *buffer))
                };
                if let Some(raw) = full {
                    staged(b, &raw, &mut compressed)?;
                }
            }
            spilled_blocks.fetch_add(store.decoded_blocks(), Relaxed);
            distributed.fetch_add(rows.len() as u64, Relaxed);
            Ok(())
        },
    )?;
    if let Ran::Failed(failure) = ran {
        return Err(failure);
    }
    let mut compressed = Vec::new();
    for (b, buffer) in gathering.into_iter().enumerate() {
        let raw = buffer.into_inner().unwrap_or_else(|e| e.into_inner());
        if !raw.is_empty() {
            staged(b, &raw, &mut compressed)?;
        }
    }
    drop(stores);
    report.spilled_blocks_decoded = spilled_blocks.load(Relaxed);
    let files: Vec<Mutex<Option<ByteFile>>> = stagings
        .into_iter()
        .map(|w| {
            w.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .map(ByteWriter::finish)
                .transpose()
                .map(Mutex::new)
        })
        .collect::<std::result::Result<_, _>>()?;
    // The run list, sorted by (bucket, offset).
    let listed = runs
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .finish()?;
    let mut by_bucket = ExternalSorter::<StagedRun>::new(scratch, sorter)?.observed(progress);
    for (i, r) in listed.reader()?.enumerate() {
        let r = r?;
        report.staging_runs += 1;
        report.staging_bytes += u64::from(r.stored_bytes);
        by_bucket.push(r)?;
        if i.is_multiple_of(4096) {
            progress.check()?;
        }
    }
    drop(listed);
    let mut sorted_runs = by_bucket.finish()?.peekable();
    progress.end()?;
    // 5. Sort and write, buckets in order.
    let mut writer = SequenceWriter::<FullTriangle>::compressed(
        scratch,
        "full-triangles",
        buffer_bytes,
        layout,
    )?;
    // Each staged run of `runs` decoded, in order, handed to `each`.
    let read_runs = |runs: &mut dyn Iterator<Item = Result<StagedRun>>,
                     each: &mut dyn FnMut(&[u8]) -> Result<()>|
     -> Result<u64> {
        let (mut compressed, mut raw) = (Vec::new(), Vec::new());
        let mut records = 0u64;
        for r in runs {
            let r = r?;
            compressed.resize(r.stored_bytes as usize, 0);
            {
                let file = files[r.group as usize]
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                file.as_ref()
                    .ok_or_else(|| invariant("staging file removed early"))?
                    .read_at(r.offset, &mut compressed)?;
            }
            raw.resize(r.records as usize * record, 0);
            crate::spill::decode_block(&compressed, r.stored, &mut raw)
                .map_err(|e| invariant(&format!("staged run: {e}")))?;
            each(&raw)?;
            records += u64::from(r.records);
        }
        Ok(records)
    };
    // Sorts an ordinary bucket in memory: its records' bytes in the strict order.
    let sort_bucket = |b: usize, runs: Vec<StagedRun>| -> Result<Vec<u8>> {
        #[cfg(test)]
        if b == 1 {
            injected(scratch, Fault::Bucket)?;
        }
        let mut raw = Vec::new();
        let records = read_runs(&mut runs.into_iter().map(Ok), &mut |run| {
            raw.extend_from_slice(run);
            Ok(())
        })?;
        if records != records_of(b) {
            return Err(invariant("staged runs do not hold their bucket"));
        }
        let mut order: Vec<(SortKey, u32)> = raw
            .chunks_exact(record)
            .enumerate()
            .map(|(i, r)| (sort_key(r), i as u32))
            .collect();
        order.sort_unstable();
        let mut sorted = Vec::with_capacity(raw.len());
        for (_, i) in &order {
            sorted.extend_from_slice(&raw[*i as usize * record..(*i as usize + 1) * record]);
        }
        Ok(sorted)
    };
    progress.begin(Step::Sequence, n)?;
    let window = plan.window.max(1);
    // Run lists of the buckets handed out so far (ordinary buckets only), and their results.
    let lists: Vec<Mutex<Option<Vec<StagedRun>>>> =
        (0..buckets).map(|_| Mutex::new(None)).collect();
    let slots: Vec<Mutex<Option<Result<Vec<u8>>>>> =
        (0..buckets).map(|_| Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let consumed = std::sync::atomic::AtomicUsize::new(0);
    let dispatched = std::sync::atomic::AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let failed = std::sync::atomic::AtomicUsize::new(usize::MAX);
    // Workers and the writer wait on one gate (a condition variable needs one mutex).
    let gate = Mutex::new(());
    let ready = std::sync::Condvar::new();
    let wait = |millis: u64| {
        let guard = gate.lock().unwrap_or_else(|e| e.into_inner());
        drop(ready.wait_timeout(guard, std::time::Duration::from_millis(millis)));
    };
    let last_of_group = {
        let mut last = vec![0usize; groups];
        for b in 0..buckets {
            last[group_of[b]] = b;
        }
        last
    };
    let mut previous: Option<TriangleRecord> = None;
    let mut written = 0u64;
    let outcome = std::thread::scope(|scope| -> Result<()> {
        // Sets the stop flag when a worker unwinds, so the writer stops waiting and joins.
        struct Unwind<'a>(&'a AtomicBool, &'a std::sync::Condvar);
        impl Drop for Unwind<'_> {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    self.0.store(true, Relaxed);
                    self.1.notify_all();
                }
            }
        }
        let worker = || {
            let _unwind = Unwind(&stop, &ready);
            loop {
                let b = next.fetch_add(1, Relaxed);
                // Test seam: with Fault::Bucket, the worker holding bucket 0 waits for the
                // stop (bucket 1 fails) and leaves without filling the writer's slot.
                #[cfg(test)]
                if b == 0 && injected(scratch, Fault::Bucket).is_err() {
                    let started = std::time::Instant::now();
                    while !stop.load(Relaxed) && started.elapsed().as_secs() < 30 {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
                if b >= buckets || stop.load(Relaxed) {
                    break;
                }
                // Stay within the window ahead of the writer, and wait for the bucket's runs.
                while (b >= consumed.load(Relaxed) + window || b >= dispatched.load(Relaxed))
                    && !stop.load(Relaxed)
                {
                    wait(5);
                }
                if stop.load(Relaxed) {
                    break;
                }
                // Heavy buckets are sorted externally by the writer itself.
                let result = if heavy(b) {
                    Ok(Vec::new())
                } else {
                    let runs = lists[b].lock().unwrap_or_else(|e| e.into_inner()).take();
                    stop_check().and_then(|()| {
                        sort_bucket(
                            b,
                            runs.ok_or_else(|| invariant("bucket runs not handed out"))?,
                        )
                    })
                };
                let failed_here = result.is_err();
                *slots[b].lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                if failed_here {
                    // The lowest failed bucket's error is the pass's error (stored before the stop).
                    failed.fetch_min(b, Relaxed);
                    stop.store(true, Relaxed);
                }
                ready.notify_all();
            }
        };
        let mut threads = Vec::new();
        for _ in 0..workers.clamp(1, buckets.max(1)) {
            match std::thread::Builder::new()
                .name("rusty-tiles-assembly".into())
                .spawn_scoped(scope, worker)
            {
                Ok(thread) => threads.push(thread),
                Err(error) => {
                    stop.store(true, Relaxed);
                    return Err(JobError::new(
                        JobErrorKind::ResourceLimit,
                        format!("start assembly worker: {error}"),
                    ));
                }
            }
        }
        // Hands out the run lists of ordinary buckets below `limit`, stopping at a heavy
        // bucket (its runs are read by the writer, in order, when its turn comes).
        type Runs<'r> = std::iter::Peekable<crate::spill::SortedStream<'r, StagedRun>>;
        let dispatch = |sorted_runs: &mut Runs<'_>, limit: usize| -> Result<()> {
            let mut b = dispatched.load(Relaxed);
            while b < limit.min(buckets) && !heavy(b) {
                let mut list = Vec::new();
                while let Some(r) =
                    sorted_runs.next_if(|r| r.as_ref().is_ok_and(|r| r.bucket as usize == b))
                {
                    list.push(r?);
                }
                if let Some(Err(_)) = sorted_runs.peek() {
                    return Err(sorted_runs.next().unwrap().unwrap_err().into());
                }
                *lists[b].lock().unwrap_or_else(|e| e.into_inner()) = Some(list);
                b += 1;
                dispatched.store(b, Relaxed);
            }
            ready.notify_all();
            Ok(())
        };
        let mut write = |bytes: &[u8]| -> Result<()> {
            // The merge's check: each triangle record strictly after the previous one.
            let row = TriangleRecord::decode(&bytes[..TriangleRecord::SIZE]);
            if previous
                .as_ref()
                .is_some_and(|p: &TriangleRecord| p.cmp(&row) != std::cmp::Ordering::Less)
            {
                return Err(invariant(
                    "full source provenance projection is not unique strict order",
                ));
            }
            previous = Some(row);
            writer.push_encoded(bytes)?;
            written += 1;
            if written.is_multiple_of(1024) {
                progress.advance(written)?;
            }
            Ok(())
        };
        let mut result = Ok(());
        'buckets: for b in 0..buckets {
            if let Err(error) = dispatch(&mut sorted_runs, b + window) {
                result = Err(error);
                break;
            }
            let wrote = if heavy(b) {
                // One prefix cell over the bucket size: an external sort of its records, read
                // run by run. Its staging file goes as soon as the sorter holds them.
                (|| -> Result<()> {
                    let mut sort =
                        ExternalSorter::<FullTriangle>::with_layout(scratch, sorter, layout)?
                            .observed(progress);
                    let mut runs = std::iter::from_fn(|| {
                        sorted_runs
                            .next_if(|r| r.as_ref().map_or(true, |r| r.bucket as usize == b))
                            .map(|r| r.map_err(JobError::from))
                    });
                    let records = read_runs(&mut runs, &mut |run| {
                        for r in run.chunks_exact(record) {
                            sort.push(FullTriangle::decode_in(layout, r))?;
                        }
                        Ok(())
                    })?;
                    if records != records_of(b) {
                        return Err(invariant("staged runs do not hold their bucket"));
                    }
                    if own_file(b) {
                        *files[group_of[b]].lock().unwrap_or_else(|e| e.into_inner()) = None;
                    }
                    dispatched.store(b + 1, Relaxed);
                    let mut encoded = vec![0u8; record];
                    for row in sort.finish()? {
                        row?.encode_in(layout, &mut encoded);
                        write(&encoded)?;
                    }
                    Ok(())
                })()
            } else {
                let done = loop {
                    if let Some(done) = slots[b].lock().unwrap_or_else(|e| e.into_inner()).take() {
                        break done;
                    }
                    if stop.load(Relaxed) && threads.iter().all(|t| t.is_finished()) {
                        // Every worker left: this bucket's own result if it came in meanwhile,
                        // else the failed bucket's error, else (a panic) the joins below.
                        let own = slots[b].lock().unwrap_or_else(|e| e.into_inner()).take();
                        if let Some(done) = own {
                            break done;
                        }
                        let f = failed.load(Relaxed);
                        let theirs = (f < buckets)
                            .then(|| slots[f].lock().unwrap_or_else(|e| e.into_inner()).take())
                            .flatten();
                        result = match theirs {
                            Some(Err(error)) => Err(error),
                            _ => Err(invariant("assembly bucket did not run")),
                        };
                        break 'buckets;
                    }
                    wait(25);
                    if let Err(error) = progress.check() {
                        result = Err(error);
                        break 'buckets;
                    }
                };
                done.and_then(|sorted| sorted.chunks_exact(record).try_for_each(&mut write))
            };
            if let Err(error) = wrote {
                result = Err(error);
                break;
            }
            consumed.store(b + 1, Relaxed);
            ready.notify_all();
            if last_of_group[group_of[b]] == b {
                *files[group_of[b]].lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
        }
        stop.store(true, Relaxed);
        ready.notify_all();
        for thread in threads {
            if let Err(panic) = thread.join() {
                std::panic::resume_unwind(panic);
            }
        }
        result
    });
    outcome?;
    if written != n {
        return Err(invariant("full capture rank count differs from scan"));
    }
    progress.end()?;
    Ok((writer.finish()?, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spilled primitive keeps its block caches with at least 31 in 32 local chunks and is
    /// routed below that, exactly at the boundary.
    #[test]
    fn the_route_starts_below_31_in_32_local_chunks() {
        for (local, chunks, routed) in [
            (31, 32, false),
            (30, 32, true),
            (32, 32, false),
            (0, 1, true),
            (1, 1, false),
            (62, 64, false),
            (61, 64, true),
            (969, 1000, false),
            (968, 1000, true),
            (0, 0, false),
        ] {
            assert_eq!(needs_route(local, chunks), routed, "{local} of {chunks}");
        }
    }

    type Mapped = ([u32; 3], Option<[u64; 9]>, std::thread::ThreadId);

    /// A world position per chunk, mapped with its Jacobian; bits, and the thread that ran it.
    fn map_chunk(chunk: usize, frame: &mut SourceFrame) -> Result<Mapped> {
        let world = [chunk as f64 * 0.7 - 5., 3. - chunk as f64 * 0.3, 1.5];
        let (position, jacobian) = frame.vertex(world, true)?;
        Ok((
            position.map(f32::to_bits),
            jacobian.map(|j| {
                let flat: Vec<u64> = j.0.iter().flatten().map(|v| v.to_bits()).collect();
                flat.try_into().unwrap()
            }),
            std::thread::current().id(),
        ))
    }

    fn resolved(definition: &str, offset: [f64; 2]) -> SourceFrame {
        let spec = super::super::source_crs::MeshSourceCrs::from_parameters(
            Some(definition),
            Some(crate::georef::SourceAxes::Xyz),
            Some(&offset),
            None,
            None,
        )
        .unwrap()
        .unwrap();
        SourceFrame::resolve(&spec, [[-10., -10., 0.], [10., 10., 5.]]).unwrap()
    }

    /// Local and portable CRS frames decode on the workers, each task with a copy that maps
    /// every position and Jacobian to the same bits as the frame itself. A native frame
    /// decodes on the calling thread.
    #[test]
    fn portable_frames_decode_on_the_workers_with_identical_mapping() {
        let caller = std::thread::current().id();
        let mut frames = vec![
            (SourceFrame::Local, true),
            (resolved("EPSG:32756", [502_000., 6_961_000.]), true),
        ];
        // The allow-listed GDA2020 code is portable in the standard build, native PROJ in
        // the native one.
        frames.push((
            resolved("EPSG:7856", [502_000., 6_961_000.]),
            cfg!(not(feature = "native-geospatial")),
        ));
        for (mut frame, portable) in frames {
            assert_eq!(frame.portable().is_some(), portable);
            let want: Vec<_> = (0..24)
                .map(|chunk| map_chunk(chunk, &mut frame).unwrap())
                .collect();
            super::super::tasks::SPAWNED.with(|n| n.set(0));
            let Ran::All(got) = decode_chunks(
                &mut frame,
                0..24,
                4,
                &mut || Ok(()),
                |chunk, frame, check| {
                    check()?;
                    map_chunk(chunk, frame)
                },
            )
            .unwrap() else {
                panic!("no chunk fails");
            };
            let spawned = super::super::tasks::SPAWNED.with(|n| n.get());
            assert_eq!(spawned > 0, portable, "{spawned} workers");
            for (want, got) in want.iter().zip(&got) {
                assert_eq!((want.0, want.1), (got.0, got.1));
            }
            assert_eq!(got.iter().all(|m| m.2 == caller), !portable);
        }
    }

    /// On the calling thread a task's `check` is `poll`: a `poll` error ends the decode as that
    /// error, at the check that saw it, while a task's own error is a task failure.
    #[test]
    fn inline_decode_returns_poll_errors_and_task_failures_apart() {
        let task = |chunk: usize, _: &mut SourceFrame, check: &mut dyn FnMut() -> Result<()>| {
            for _ in 0..10 {
                check()?;
            }
            if chunk == 3 {
                return Err(JobError::new(JobErrorKind::InvalidInput, "bad vertex"));
            }
            Ok(chunk)
        };
        // Chunk 0 takes a poll and 10 checks; chunk 1 a poll (the 12th) and fails at its 3rd.
        let mut polls = 0;
        let error = decode_inline(
            &mut SourceFrame::Local,
            0..5,
            &mut || {
                polls += 1;
                if polls == 15 {
                    return Err(JobError::new(JobErrorKind::Cancelled, "run stopped"));
                }
                Ok(())
            },
            task,
        )
        .err()
        .expect("the poll error");
        assert_eq!(error.kind(), JobErrorKind::Cancelled);
        assert!(error.to_string().contains("run stopped"), "{error}");
        assert_eq!(polls, 15);
        match decode_inline(&mut SourceFrame::Local, 0..5, &mut || Ok(()), task).unwrap() {
            Ran::Failed(error) => assert_eq!(error.kind(), JobErrorKind::InvalidInput),
            Ran::All(_) => panic!("chunk 3 fails"),
        }
    }
}
