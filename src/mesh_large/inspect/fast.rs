//! Default `mesh-inspect` (#437): the source's exact-position topology read straight from the
//! source, without capture. Its report equals the capture-based pass's ([`topology::run`])
//! field for field.
//!
//! **Positions.** Each corner's position is the one capture gives it: the POSITION element
//! after its node transform and any source CRS mapping, as canonical f32
//! (`capture::canonical`), or, with an opt-in weld, its class representative's.
//!
//! **Partitions.** One parallel pass streams every triangle (tasks of 2^16 triangles of one
//! primitive instance). Each used source vertex, once, goes to the partition of its position
//! key; each edge of a non-degenerate triangle (keys `a < b`, primitive instance, source
//! vertex indices) to the partition of `a`, as does each pair of distinct positions of a
//! degenerate triangle (for components only). Partitions are key ranges split at quantiles of
//! a sample of corner keys, so reading them in order reads every key in sorted order: a
//! position's id is its partition's offset plus its rank there, exactly as the capture-based
//! pass numbers positions, and edge groups come in the same order.
//!
//! **Counting.** Partitions are read, sorted and counted in memory, up to `jobs` at a time
//! (fewer when the largest would not fit). Every length sum is an f64 sum in edge-group
//! order: each partition returns its terms in order and they are added in partition order,
//! so each sum is the sequential one, bit for bit. Surface area is an [`ExactSum`].
//!
//! **Components.** One `u32` per position, as the capture-based pass holds. Each partition
//! joins the positions of its own edges; an edge whose second end lies in a later partition
//! is joined once every partition has been counted, its end's id found by key in one ordered
//! pass over the partitions' positions.
//!
//! **Loops.** Open edges and seams, in edge order, go to two sequences, joined into chains as
//! the capture-based pass joins them ([`topology::join_loops`]).
//!
//! Every partition file is compressed in scratch; the scratch bound is preflighted (and, on
//! memory-backed scratch, charged to memory) before the pass writes, every resident array is
//! admitted before it is allocated, and every step reports its total.
use super::{
    exact_sum::ExactSum,
    topology::{self, Admit, GeoLines, GeoRequest, Groups, KeptEdge, KeptEdges, TopologyReport},
};
use crate::{
    mesh_large::{
        assembly::decode_chunks,
        capture::{canonical, source_position, vertices},
        input::StreamSource,
        phases::{Step, StepProgress},
        source_crs::SourceFrame,
        tasks::{run as run_tasks, Ran},
        weld::{AuditFace, MovedTargets},
    },
    spill::{
        decode_block, encode_block, ByteFile, ByteWriter, Encoded, Layout, Record, Scratch,
        Sequence, SequenceWriter, SortConfig,
    },
    JobError, JobErrorKind,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
    Mutex,
};

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}
fn resource(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message.into())
}
/// `v` as an index (every count here fits the platform's memory, or allocation fails first).
fn to_index(v: u64) -> Result<usize, JobError> {
    usize::try_from(v).map_err(|_| resource("an index exceeds the platform's address space"))
}
/// `v` as a 32-bit id.
fn to_u32(v: usize) -> Result<u32, JobError> {
    u32::try_from(v).map_err(|_| resource("an id exceeds 2^32 - 1"))
}
fn stopped() -> JobError {
    JobError::new(JobErrorKind::Cancelled, "inspection stopped")
}

/// A position: its canonical f32 bits. Ordered as the capture-based pass orders positions.
type Key = [u32; 3];

fn put_u32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn get_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(crate::spill::field(bytes, at))
}
fn put_key(out: &mut [u8], at: usize, k: Key) {
    for (i, v) in k.iter().enumerate() {
        put_u32(out, at + 4 * i, *v);
    }
}
fn get_key(bytes: &[u8], at: usize) -> Key {
    std::array::from_fn(|i| get_u32(bytes, at + 4 * i))
}

/// A used source vertex: its position and global expanded index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VertexRec {
    key: Key,
    id: u64,
}
impl Record for VertexRec {
    const SIZE: usize = 20;
    fn encode(&self, out: &mut [u8]) {
        put_key(out, 0, self.key);
        out[12..20].copy_from_slice(&self.id.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            key: get_key(bytes, 0),
            id: u64::from_le_bytes(crate::spill::field(bytes, 12)),
        }
    }
}

/// `inst` of a pair that only joins components (two positions of a degenerate triangle).
const JOIN_ONLY: u32 = u32::MAX;

/// A triangle edge between positions `a < b`: its primitive instance (by `(node, primitive)`
/// order) and source vertex indices in `a`, `b` order. Sorted as the capture-based pass sorts
/// its edges; join-only pairs sort after a group's edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct EdgeRec {
    a: Key,
    b: Key,
    inst: u32,
    ia: u32,
    ib: u32,
}
impl Record for EdgeRec {
    const SIZE: usize = 36;
    fn encode(&self, out: &mut [u8]) {
        put_key(out, 0, self.a);
        put_key(out, 12, self.b);
        put_u32(out, 24, self.inst);
        put_u32(out, 28, self.ia);
        put_u32(out, 32, self.ib);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            a: get_key(bytes, 0),
            b: get_key(bytes, 12),
            inst: get_u32(bytes, 24),
            ia: get_u32(bytes, 28),
            ib: get_u32(bytes, 32),
        }
    }
}

/// A partition's distinct positions, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyRec(Key);
impl Record for KeyRec {
    const SIZE: usize = 12;
    fn encode(&self, out: &mut [u8]) {
        put_key(out, 0, self.0);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self(get_key(bytes, 0))
    }
}

/// A kept (open or seam) edge: position id `a`, and `b`'s id or, with `cross`, the index of
/// its key in the cross table (resolved before the joins).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeptRec {
    a: u32,
    b: u32,
    cross: bool,
    between: bool,
    /// In memory only: a seam (else an open edge). Not stored: each kind has its sequence.
    seam: bool,
    pa: Key,
    pb: Key,
}
impl Record for KeptRec {
    const SIZE: usize = 34;
    fn encode(&self, out: &mut [u8]) {
        put_u32(out, 0, self.a);
        put_u32(out, 4, self.b);
        out[8] = u8::from(self.cross);
        out[9] = u8::from(self.between);
        put_key(out, 10, self.pa);
        put_key(out, 22, self.pb);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            a: get_u32(bytes, 0),
            b: get_u32(bytes, 4),
            cross: bytes[8] != 0,
            between: bytes[9] != 0,
            seam: false,
            pa: get_key(bytes, 10),
            pb: get_key(bytes, 22),
        }
    }
}

/// Kept edges with their cross ends resolved.
struct Kept<'a> {
    edges: Sequence<KeptRec>,
    resolved: &'a [u32],
}
impl KeptEdges for Kept<'_> {
    fn count(&self) -> u64 {
        self.edges.len()
    }
    fn edges(&self) -> Result<Box<dyn Iterator<Item = Result<KeptEdge, JobError>> + '_>, JobError> {
        Ok(Box::new(self.edges.reader()?.map(|e| {
            let e = e?;
            let b = if e.cross {
                *self
                    .resolved
                    .get(e.b as usize)
                    .ok_or_else(|| state("a kept edge's end is not resolved"))?
            } else {
                e.b
            };
            Ok(KeptEdge {
                a: e.a,
                b,
                leaf: 0,
                between: e.between,
                pa: e.pa,
                pb: e.pb,
            })
        })))
    }
}

/// Triangles one streaming task reads.
const STREAM_TRIANGLES: usize = 1 << 16;
/// The position reader's window (positions are read in index order, as the scan reads them).
const POSITION_WINDOW: usize = 64 << 10;
/// A smaller window for a task whose vertices are spread thinly over a large accessor.
const SPARSE_WINDOW: usize = 4 << 10;
/// Corner keys sampled for the partition split: [`SAMPLES_PER_PART`] per partition, between
/// these bounds.
const MIN_SAMPLES: u64 = 1 << 16;
const MAX_SAMPLES: u64 = 1 << 21;
const SAMPLES_PER_PART: u64 = 256;
/// At most this many partitions (their blocks share one scratch log).
const MAX_PARTS: u64 = 4096;
/// Resident bytes of an edge record while its partition is counted, beside its group's
/// outputs (at most one group per record): its length term, its kept edge, and an edge to a
/// later partition's key and join. Every vector is reserved exactly.
const EDGE_WORK_BYTES: u64 = (size_of::<EdgeRec>()
    + size_of::<Term>()
    + size_of::<KeptRec>()
    + size_of::<Key>()
    + size_of::<(u32, u32)>()) as u64;
/// Resident bytes of a vertex record while its partition is counted.
const VERTEX_WORK_BYTES: u64 = size_of::<VertexRec>() as u64;
/// Resident bytes per position of a partition's key table while its edges are counted.
const KEY_WORK_BYTES: u64 = size_of::<Key>() as u64;
/// Resident bytes per edge between partitions once every partition is counted: its key,
/// its join, its resolved id and its sorted copy.
const CROSS_BYTES: u64 =
    (size_of::<Key>() + size_of::<(u32, u32)>() + size_of::<u32>() + size_of::<(Key, u32)>())
        as u64;
/// Stream buffers of the partition store, the position tables and the kept edges.
const PART_BUFFER: usize = 32 << 10;
/// Resident bytes of one partition's block list per block, at twice its size (it grows).
const BLOCK_BYTES: u64 = 2 * size_of::<Block>() as u64;

/// Resident bytes a streaming task holds per triangle (every vector reserved exactly): its
/// three edges and, read in triangle order, its corners' vertices with their partitions,
/// their indices and the task's table of their positions; with a weld, its face; and the
/// encoded and compressed blocks it writes.
fn task_bytes(weld: bool) -> u64 {
    let records = 3 * size_of::<(usize, EdgeRec)>() + 3 * size_of::<(usize, VertexRec)>();
    let direct = size_of::<[u64; 3]>() + 3 * (size_of::<u64>() + size_of::<(Key, bool)>());
    let face = if weld {
        size_of::<AuditFace>() + AuditFace::SIZE
    } else {
        0
    };
    let blocks = 3 * 3 * (EdgeRec::SIZE + VertexRec::SIZE);
    (records + direct + face + blocks) as u64
}

/// What the pass reads.
pub(crate) struct Input<'s, 'a, 'p> {
    pub source: &'a StreamSource,
    pub frame: &'a mut SourceFrame,
    pub moved: Option<&'a MovedTargets<'a>>,
    /// With a weld, every face is also written here for its audit.
    pub audit: Option<&'a Mutex<SequenceWriter<AuditFace>>>,
    pub scratch: &'s Scratch,
    pub sorter: SortConfig,
    pub workers: usize,
    /// At least this many partitions (1 outside tests).
    pub min_parts: u64,
    /// Bytes of position tables at once, when not derived from the budget (tests: 0 reads
    /// every instance's corners in triangle order).
    pub tables: Option<u64>,
    /// Resident bytes the pass may hold besides the source and the run reserve.
    pub memory: u64,
    pub geo: Option<GeoRequest>,
    pub admit: &'a Admit<'a>,
    pub progress: &'s StepProgress<'p>,
}

/// The pass's report and selected GeoJSON lines, with its resident peak.
pub(crate) struct Fast<'s> {
    pub report: TopologyReport,
    pub geo: Option<GeoLines<'s>>,
    /// Partitions, and how many were counted at once.
    pub partitions: u64,
    pub workers: u64,
    /// The largest resident total of the streaming and counting steps.
    pub peak_bytes: u64,
}

/// Worst-case scratch of the pass for `triangles` triangles over `vertices` expanded source
/// vertices: every partition file, position table and kept edge at once (stored at most at
/// their raw size), and the GeoJSON sort.
pub(crate) fn scratch_need(triangles: u64, vertices: u64, sorter: SortConfig) -> Option<u64> {
    let corners = triangles.checked_mul(3)?;
    let used = vertices.min(corners);
    let geo = topology::spill::<topology::GeoSegment>(corners, sorter)?;
    corners
        .checked_mul(EdgeRec::SIZE as u64)?
        .checked_add(used.checked_mul((VertexRec::SIZE + KeyRec::SIZE) as u64)?)?
        .checked_add(corners.checked_mul(KeptRec::SIZE as u64)?)?
        .checked_add(geo.finish_peak)?
        .checked_add(geo.stream_slack)
}

/// Resident bytes of the streaming step besides its position tables and block lists:
/// `workers` tasks (records, readers and blocks), the used-vertex bitmap over `vertices`
/// expanded source vertices, the key sample and the store's buffer.
fn stream_bytes(vertices: u64, workers: usize, weld: bool) -> u64 {
    (workers as u64)
        .saturating_mul(
            (STREAM_TRIANGLES as u64)
                .saturating_mul(task_bytes(weld))
                .saturating_add(2 * POSITION_WINDOW as u64),
        )
        .saturating_add(vertices.div_ceil(64).saturating_mul(8))
        .saturating_add(MAX_SAMPLES * size_of::<Key>() as u64)
        .saturating_add(2 * PART_BUFFER as u64)
}

/// Resident bytes the pass needs at least for `workers` tasks over `triangles` triangles and
/// `vertices` expanded source vertices: its streaming step, and one position's group per
/// possible position beside a 64 MiB share for counting.
pub(crate) fn memory_floor(triangles: u64, vertices: u64, workers: usize, weld: bool) -> u64 {
    let used = vertices.min(triangles.saturating_mul(3));
    stream_bytes(vertices, workers, weld).max(used.saturating_mul(4).saturating_add(64 << 20))
}

/// One primitive instance: its scene index and instance ordinal.
struct Instance {
    scene: usize,
    ordinal: u32,
    /// Its first global expanded vertex.
    offset: u64,
}

pub(crate) fn run<'s>(input: Input<'s, '_, '_>) -> Result<Fast<'s>, JobError> {
    let Input {
        source,
        frame,
        moved,
        audit,
        scratch,
        sorter,
        workers,
        min_parts,
        tables,
        memory,
        geo,
        admit,
        progress,
    } = input;
    let workers = workers.max(1);
    let primitives: Vec<_> = source.document.scene_primitives().collect();
    let n: u64 = primitives.iter().map(|p| p.triangles as u64).sum();
    let vertices_total: u64 = primitives.iter().map(|p| p.position.count as u64).sum();
    // Instance ordinals in `(node, primitive)` order; offsets in scene order.
    let mut order: Vec<(u32, u32, usize)> = primitives
        .iter()
        .enumerate()
        .map(|(i, p)| (p.node, p.primitive, i))
        .collect();
    order.sort_unstable();
    let mut instances: Vec<Instance> = primitives
        .iter()
        .enumerate()
        .map(|(i, _)| Instance {
            scene: i,
            ordinal: 0,
            offset: 0,
        })
        .collect();
    for (ordinal, &(_, _, i)) in order.iter().enumerate() {
        instances[i].ordinal =
            u32::try_from(ordinal).map_err(|_| resource("too many primitive instances"))?;
    }
    let mut offset = 0u64;
    for (inst, p) in instances.iter_mut().zip(&primitives) {
        inst.offset = offset;
        offset += p.position.count as u64;
    }
    let mut report = TopologyReport {
        triangles: n,
        open: topology::Loops::new(),
        seams: topology::Loops::new(),
        ..Default::default()
    };
    report.primitive_instances = {
        let mut used: Vec<(u32, u32)> = primitives
            .iter()
            .filter(|p| p.triangles > 0)
            .map(|p| (p.node, p.primitive))
            .collect();
        used.sort_unstable();
        used.dedup();
        used.len() as u64
    };

    // Partitions: a sample of corner keys and the budget's share per worker. The position
    // groups (one `u32` per possible position) are held beside the counting.
    let weld = moved.is_some();
    let floor = memory_floor(n, vertices_total, workers, weld);
    if memory < floor {
        return Err(resource(format!(
            "mesh-inspect needs at least {floor} bytes for its partitions and position groups \
             but the memory budget leaves {memory}"
        )));
    }
    let used_bound = vertices_total.min(n.saturating_mul(3));
    let edge_bytes = n.saturating_mul(3).saturating_mul(EDGE_WORK_BYTES);
    // Block lists, held from the stream until every partition is counted: each task writes
    // at most one block per partition and record kind, and never more blocks than records.
    let tasks = (n + vertices_total).div_ceil(STREAM_TRIANGLES as u64) + primitives.len() as u64;
    let block_bound = |parts: u64| {
        tasks
            .saturating_mul(2 * parts)
            .min(n.saturating_mul(3).saturating_add(vertices_total))
            .saturating_mul(BLOCK_BYTES)
    };
    // Partitions of about half a worker's share of the counting pool, so a sample's error
    // still fits; the pool is what the groups and the block lists leave.
    let groups_pool = memory.saturating_sub(used_bound.saturating_mul(4));
    let parts_for = |pool: u64| {
        edge_bytes
            .div_ceil((pool / workers as u64 / 2).max(1))
            .max(workers as u64)
            .min(MAX_PARTS)
            .min(n.max(1))
            .max(min_parts)
            .max(1)
    };
    let first_parts = parts_for(groups_pool);
    let parts = parts_for(groups_pool.saturating_sub(block_bound(first_parts))).max(first_parts);
    let pool = groups_pool.saturating_sub(block_bound(parts));
    // Refused now, not after the stream, when even an even split cannot be counted.
    let expected = edge_bytes.div_ceil(parts);
    if expected > pool {
        return Err(resource(format!(
            "mesh-inspect needs about {expected} bytes to count each of its {parts} partitions \
             but the memory budget leaves {pool}; raise --memory-budget"
        )));
    }
    let splitters = sample(source, frame, &primitives, n, parts, progress)?;
    let parts = splitters.len() + 1;
    let blocks = block_bound(parts as u64) / BLOCK_BYTES;
    let streaming =
        stream_bytes(vertices_total, workers, weld).saturating_add(block_bound(parts as u64));

    // 1. Stream every triangle into the partitions' blocks, in one scratch log.
    let store = Mutex::new(Store::new(scratch, parts)?);
    let used: Vec<AtomicU64> = (0..vertices_total.div_ceil(64))
        .map(|_| AtomicU64::new(0))
        .collect();
    let sink = Sink {
        store: &store,
        splitters: &splitters,
        audit,
    };
    let stream = Stream {
        source,
        primitives: &primitives,
        instances: &instances,
        moved,
        used: &used,
        sink: &sink,
        workers,
        done: AtomicU64::new(0),
    };
    // Whole primitive instances at a time, as many as their position tables fit: each
    // table holds the key of every used vertex, decoded once in vertex order.
    let tables = tables.unwrap_or(memory.saturating_sub(streaming));
    let mut table_peak = 0u64;
    progress.begin(Step::InspectStream, n)?;
    let mut surface = ExactSum::default();
    let mut first = 0;
    while first < primitives.len() {
        let mut last = first;
        let mut held = 0u64;
        while let Some(p) = primitives.get(last) {
            let bytes = (p.position.count as u64).saturating_mul(TABLE_BYTES);
            if held.saturating_add(bytes) > tables {
                break;
            }
            held += bytes;
            last += 1;
        }
        let streamed = if last == first {
            // One instance past the allowance: each task decodes its own corners.
            last = first + 1;
            stream.direct(first, frame, progress)
        } else {
            table_peak = table_peak.max(held);
            stream.batch(first..last, frame, progress)
        };
        let (area, degenerate) = match streamed {
            Ok(found) => found,
            // The first failing vertex in capture's order, not the first in time; any
            // other failure (a stop, a read or a resource) as it is.
            Err(error) if is_source_error(&error) => {
                return Err(verdict(source, frame, &primitives, progress)?.unwrap_or(error))
            }
            Err(error) => return Err(error),
        };
        surface.merge(&area);
        report.degenerate_triangles += degenerate;
        first = last;
    }
    report.surface_area_m2 = surface.value();
    source.verify_unchanged()?;
    drop(used);
    let store = store
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .finish()?;
    let streaming = streaming.saturating_add(table_peak);
    if store.blocks() > blocks {
        return Err(state(
            "the partition store holds more blocks than its bound",
        ));
    }
    // The counting pool is what the groups and the block lists, as allocated, leave.
    let lists = store.list_bytes();
    let pool = groups_pool.saturating_sub(lists);
    progress.end()?;

    // How many partitions are counted at once: as many as fit the budget, at most `workers`.
    let largest = (0..parts)
        .map(|p| {
            let (v, e) = (store.vertex_records[p], store.edge_records[p]);
            let blocks = 2 * store.largest_block(p);
            v.saturating_mul(VERTEX_WORK_BYTES)
                .max(
                    e.saturating_mul(EDGE_WORK_BYTES)
                        .saturating_add(v.saturating_mul(KEY_WORK_BYTES)),
                )
                .saturating_add(blocks)
        })
        .max()
        .unwrap_or(0);
    if largest > pool {
        return Err(resource(format!(
            "mesh-inspect needs {largest} bytes to count its largest partition but the memory \
             budget leaves {pool}; raise --memory-budget"
        )));
    }
    let wave = usize::try_from(pool / largest.max(1))
        .unwrap_or(usize::MAX)
        .clamp(1, workers);
    let counting = largest
        .saturating_mul(wave as u64)
        .saturating_add(used_bound.saturating_mul(4))
        .saturating_add(lists);

    // 2. Positions: counts, repeated positions and each partition's position table.
    let vertex_records: u64 = store.vertex_records.iter().sum();
    progress.begin(Step::InspectVertices, vertex_records)?;
    let done = AtomicU64::new(0);
    let counted = run_tasks(
        parts,
        wave,
        &mut || progress.advance(done.load(Relaxed)),
        |p, stop| {
            let mut recs: Vec<VertexRec> = Vec::new();
            recs.try_reserve_exact(to_index(store.vertex_records[p])?)
                .map_err(|_| resource("partition allocation failed"))?;
            store.read(&store.vertices[p], &mut recs, stop)?;
            recs.sort_unstable();
            recs.dedup();
            let mut keys = SequenceWriter::compressed(
                scratch,
                &format!("inspect-keys-{p}"),
                PART_BUFFER,
                Layout::FULL,
            )?;
            let mut c = VertexCounts::default();
            let mut i = 0;
            while i < recs.len() {
                if i % 65_536 == 0 && stop.load(Relaxed) {
                    return Err(stopped());
                }
                let key = recs[i].key;
                let mut j = i;
                while j < recs.len() && recs[j].key == key {
                    j += 1;
                }
                let here = (j - i) as u64;
                c.positions += 1;
                c.source_vertices += here;
                if here > 1 {
                    c.repeated += 1;
                    c.extra += here - 1;
                }
                keys.push(&KeyRec(key))?;
                i = j;
            }
            done.fetch_add(store.vertex_records[p], Relaxed);
            c.keys = Some(keys.finish()?);
            Ok(c)
        },
    )?;
    let counted = match counted {
        Ran::Failed(error) => return Err(error),
        Ran::All(c) => c,
    };
    let mut offsets = Vec::with_capacity(parts + 1);
    let mut positions = 0u64;
    let mut key_files = Vec::with_capacity(parts);
    for mut c in counted {
        offsets.push(positions);
        positions += c.positions;
        report.source_vertices += c.source_vertices;
        report.repeated_positions.positions += c.repeated;
        report.repeated_positions.extra_vertices += c.extra;
        key_files.push(
            c.keys
                .take()
                .ok_or_else(|| state("a partition without its positions"))?,
        );
    }
    offsets.push(positions);
    report.positions = positions;
    let position_count = u32::try_from(positions).map_err(|_| {
        JobError::new(
            JobErrorKind::Unsupported,
            "more than 2^32 distinct positions",
        )
    })?;
    progress.end()?;

    // 3. Edge groups, in partition waves: counts and length terms in order, kept edges, and
    // each partition's own joins.
    (admit.memory)("position_groups", u64::from(position_count) * 4)?;
    progress.check()?;
    let mut groups = Groups::new(position_count)?;
    progress.check()?;
    let mut open = SequenceWriter::<KeptRec>::compressed(
        scratch,
        "inspect-open-edges",
        PART_BUFFER,
        Layout::FULL,
    )?;
    let mut seams =
        SequenceWriter::<KeptRec>::compressed(scratch, "inspect-seams", PART_BUFFER, Layout::FULL)?;
    let mut cross: Vec<Key> = Vec::new();
    let mut cross_joins: Vec<(u32, u32)> = Vec::new();
    let mut sums = Sums::default();
    let edge_records: u64 = store.edge_records.iter().sum();
    progress.begin(Step::InspectEdges, edge_records)?;
    let done = AtomicU64::new(0);
    let mut first = 0;
    while first < parts {
        let last = (first + wave).min(parts);
        // Each partition of the wave joins its own slice of the groups.
        let slices: Vec<Mutex<&mut [u32]>> = {
            let mut rest = &mut groups.slice_mut()[to_index(offsets[first])?..];
            let mut out = Vec::with_capacity(last - first);
            for p in first..last {
                let (own, tail) = rest.split_at_mut(to_index(offsets[p + 1] - offsets[p])?);
                out.push(Mutex::new(own));
                rest = tail;
            }
            out
        };
        let found = run_tasks(
            last - first,
            wave,
            &mut || progress.advance(done.load(Relaxed)),
            |k, stop| {
                let p = first + k;
                let mut slice = slices[k].lock().unwrap_or_else(|e| e.into_inner());
                let out = count_edges(
                    &key_files[p],
                    &store,
                    p,
                    offsets[p],
                    &mut slice,
                    &|key| splitters.partition_point(|s| s <= key) == p,
                    stop,
                )?;
                done.fetch_add(store.edge_records[p], Relaxed);
                Ok(out)
            },
        )?;
        drop(slices);
        let found = match found {
            Ran::Failed(error) => return Err(error),
            Ran::All(found) => found,
        };
        for out in found {
            progress.check()?;
            out.counts.add_to(&mut report);
            for t in &out.terms {
                sums.add(t);
            }
            let base = u32::try_from(cross.len()).map_err(|_| resource("too many cross edges"))?;
            for mut e in out.kept {
                if e.cross {
                    e.b += base;
                }
                if e.seam {
                    seams.push(&e)?;
                } else {
                    open.push(&e)?;
                }
            }
            // The cross table and its joins, reserved exactly, beside the groups and a wave.
            let held = ((cross.len() + out.cross.len()) as u64)
                .saturating_mul(CROSS_BYTES)
                .saturating_add(counting);
            if held > memory {
                return Err(resource(format!(
                    "mesh-inspect needs {held} bytes for edges between partitions but the \
                     memory budget leaves {memory}"
                )));
            }
            cross
                .try_reserve_exact(out.cross.len())
                .and_then(|()| cross_joins.try_reserve_exact(out.cross_joins.len()))
                .map_err(|_| resource("cross edge allocation failed"))?;
            cross_joins.extend(out.cross_joins.iter().map(|&(a, c)| (a, c + base)));
            cross.extend(out.cross);
        }
        first = last;
    }
    let peak =
        streaming.max(counting.saturating_add((cross.len() as u64).saturating_mul(CROSS_BYTES)));
    drop(store);
    sums.write(&mut report);
    let open = open.finish()?;
    let seams = seams.finish()?;
    progress.end()?;

    // 4. Components: every cross end found by key, then joined.
    progress.begin(Step::InspectComponents, cross.len() as u64)?;
    (admit.memory)(
        "cross_edges",
        (cross.len() as u64).saturating_mul(CROSS_BYTES),
    )?;
    let resolved = resolve(&cross, &key_files, progress)?;
    drop(cross);
    drop(key_files);
    for (i, &(a, c)) in cross_joins.iter().enumerate() {
        if i % 65_536 == 0 {
            progress.advance(i as u64)?;
        }
        groups.join(a, resolved[c as usize]);
    }
    drop(cross_joins);
    let mut components = 0u64;
    for i in 0..position_count {
        if i % (1 << 20) == 0 {
            progress.check()?;
        }
        components += u64::from(groups.find(i) == i);
    }
    report.components = components;
    progress.end()?;

    // 5. Loops and seam chains.
    let open = Kept {
        edges: open,
        resolved: &resolved,
    };
    let seams = Kept {
        edges: seams,
        resolved: &resolved,
    };
    let geo = topology::join_loops(topology::Loops4 {
        open: &open,
        seams: &seams,
        groups: &mut groups,
        report: &mut report,
        geo,
        scratch,
        sorter,
        admit,
        progress,
        open_loops: &mut |_, _| Ok(()),
    })?;
    Ok(Fast {
        report,
        geo,
        partitions: parts as u64,
        workers: wave as u64,
        peak_bytes: peak,
    })
}

/// Whether `error` is about the source's content (a failing vertex), as opposed to a stop,
/// a read failure or a resource limit.
fn is_source_error(error: &JobError) -> bool {
    matches!(
        error.kind(),
        JobErrorKind::InvalidInput | JobErrorKind::Unsupported
    )
}

/// Capture's verdict on a failing source: instances in scene order, each one's used vertices
/// in vertex order, the first that does not decode (as capture's decoding reports it, after
/// its scan has found every index and position valid). `None` when every vertex decodes.
fn verdict(
    source: &StreamSource,
    frame: &mut SourceFrame,
    primitives: &[crate::gltf_source::source::ScenePrimitive<'_>],
    progress: &StepProgress<'_>,
) -> Result<Option<JobError>, JobError> {
    for p in primitives {
        let mut used = vec![0u64; p.position.count.div_ceil(64)];
        let mut indices = p.indices.map(|v| source.reader(&v, PART_BUFFER));
        for triangle in 0..p.triangles {
            if triangle.is_multiple_of(4096) {
                progress.check()?;
            }
            match vertices(p, &mut indices, triangle) {
                Ok(vs) => {
                    for v in vs {
                        used[to_index(v / 64)?] |= 1 << (v % 64);
                    }
                }
                Err(error) => return Ok(Some(error)),
            }
        }
        let mut positions = source.reader(&p.position, POSITION_WINDOW);
        for v in 0..p.position.count {
            if v.is_multiple_of(4096) {
                progress.check()?;
            }
            if used[v / 64] & (1 << (v % 64)) == 0 {
                continue;
            }
            let decoded = source_position(p, &mut positions, v as u64)
                .and_then(|world| frame.vertex(world, false));
            if let Err(error) = decoded {
                return Ok(Some(error));
            }
        }
    }
    Ok(None)
}

/// Resident bytes per vertex of a position table: its key and its moved bit (rounded up).
const TABLE_BYTES: u64 = 13;
/// Vertices one decoding task covers.
const DECODE_VERTICES: usize = 1 << 16;

/// One compressed block of a partition's records in the [`Store`].
#[derive(Clone, Copy)]
struct Block {
    offset: u64,
    bytes: u32,
    records: u32,
    stored: bool,
}

/// Every partition's vertex and edge records, as compressed blocks appended to one scratch
/// log: one open file however many partitions there are.
struct Store {
    log: ByteWriter,
    vertices: Vec<Vec<Block>>,
    edges: Vec<Vec<Block>>,
    vertex_records: Vec<u64>,
    edge_records: Vec<u64>,
}
impl Store {
    fn new(scratch: &Scratch, parts: usize) -> Result<Self, JobError> {
        Ok(Self {
            log: ByteWriter::new(scratch, "inspect-partitions", PART_BUFFER)?,
            vertices: vec![Vec::new(); parts],
            edges: vec![Vec::new(); parts],
            vertex_records: vec![0; parts],
            edge_records: vec![0; parts],
        })
    }
    fn append(
        &mut self,
        edges: bool,
        part: usize,
        block: &Encoded,
        records: u32,
    ) -> Result<(), JobError> {
        let at = Block {
            offset: self.log.len(),
            bytes: u32::try_from(block.bytes.len()).map_err(|_| state("block overflows"))?,
            records,
            stored: block.stored,
        };
        self.log.append(&block.bytes)?;
        if edges {
            self.edges[part].push(at);
            self.edge_records[part] += u64::from(records);
        } else {
            self.vertices[part].push(at);
            self.vertex_records[part] += u64::from(records);
        }
        Ok(())
    }
    fn finish(self) -> Result<Stored, JobError> {
        Ok(Stored {
            log: self.log.finish()?,
            vertices: self.vertices,
            edges: self.edges,
            vertex_records: self.vertex_records,
            edge_records: self.edge_records,
        })
    }
}

/// The finished [`Store`], read one partition at a time.
struct Stored {
    log: ByteFile,
    vertices: Vec<Vec<Block>>,
    edges: Vec<Vec<Block>>,
    vertex_records: Vec<u64>,
    edge_records: Vec<u64>,
}
impl Stored {
    /// Resident bytes of the block lists, as allocated.
    fn list_bytes(&self) -> u64 {
        let lists = self.vertices.iter().chain(&self.edges);
        lists
            .map(|b| b.capacity() as u64 * size_of::<Block>() as u64)
            .sum::<u64>()
            .saturating_add(
                (self.vertices.capacity() + self.edges.capacity()) as u64
                    * size_of::<Vec<Block>>() as u64,
            )
    }
    fn blocks(&self) -> u64 {
        self.vertices
            .iter()
            .chain(&self.edges)
            .map(|b| b.len() as u64)
            .sum()
    }
    /// The largest block of partition `p`, raw: what reading it holds beside its records.
    fn largest_block(&self, p: usize) -> u64 {
        let raw = |blocks: &[Block], size: usize| {
            blocks
                .iter()
                .map(|b| u64::from(b.records) * size as u64)
                .max()
                .unwrap_or(0)
        };
        raw(&self.vertices[p], VertexRec::SIZE).max(raw(&self.edges[p], EdgeRec::SIZE))
    }
    /// Appends the records of `blocks` to `out` (reserved by the caller).
    fn read<T: Record>(
        &self,
        blocks: &[Block],
        out: &mut Vec<T>,
        stop: &AtomicBool,
    ) -> Result<(), JobError> {
        let (mut stored, mut raw) = (Vec::new(), Vec::new());
        for b in blocks {
            if stop.load(Relaxed) {
                return Err(stopped());
            }
            stored.resize(b.bytes as usize, 0);
            self.log.read_at(b.offset, &mut stored)?;
            raw.resize(b.records as usize * T::SIZE, 0);
            decode_block(&stored, b.stored, &mut raw)
                .map_err(|e| state(format!("inspection partition block: {e}")))?;
            out.extend(raw.chunks_exact(T::SIZE).map(T::decode));
        }
        Ok(())
    }
}

/// Where streamed records go: the partitions' store and, with a weld, the audit's faces.
struct Sink<'a> {
    store: &'a Mutex<Store>,
    splitters: &'a [Key],
    audit: Option<&'a Mutex<SequenceWriter<AuditFace>>>,
}
impl Sink<'_> {
    fn part(&self, k: &Key) -> usize {
        self.splitters.partition_point(|s| s <= k)
    }
    /// Writes a task's records to their partitions: one compressed block per partition it
    /// touches, appended under one lock.
    fn flush(&self, out: &mut TaskOut) -> Result<(), JobError> {
        fn blocks<T: Record>(
            runs: &mut [(usize, T)],
            edges: bool,
            into: &mut Vec<(bool, usize, Encoded, u32)>,
        ) -> Result<(), JobError> {
            runs.sort_unstable_by_key(|(p, _)| *p);
            let (mut raw, mut scratch) = (Vec::new(), Vec::new());
            for run in runs.chunk_by(|a, b| a.0 == b.0) {
                raw.clear();
                raw.resize(run.len() * T::SIZE, 0);
                for ((_, r), out) in run.iter().zip(raw.chunks_exact_mut(T::SIZE)) {
                    r.encode(out);
                }
                let records = u32::try_from(run.len()).map_err(|_| state("block overflows"))?;
                into.push((edges, run[0].0, encode_block(&raw, &mut scratch), records));
            }
            Ok(())
        }
        let mut encoded = Vec::new();
        blocks(&mut out.verts, false, &mut encoded)?;
        blocks(&mut out.edges, true, &mut encoded)?;
        {
            let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            for (edges, part, block, records) in &encoded {
                store.append(*edges, *part, block, *records)?;
            }
        }
        out.verts.clear();
        out.edges.clear();
        if let Some(audit) = self.audit {
            let mut w = audit.lock().unwrap_or_else(|e| e.into_inner());
            for f in &out.faces {
                w.push(f)?;
            }
        }
        out.faces.clear();
        Ok(())
    }
}

/// One streaming task's records and sums.
#[derive(Default)]
struct TaskOut {
    verts: Vec<(usize, VertexRec)>,
    edges: Vec<(usize, EdgeRec)>,
    faces: Vec<AuditFace>,
    area: ExactSum,
    degenerate: u64,
}
impl TaskOut {
    /// Reserved exactly for `vertices` vertex records, `triangles` triangles' edges and, with
    /// a weld, their faces (as [`task_bytes`] charges).
    fn with_capacity(vertices: usize, triangles: usize, weld: bool) -> Result<Self, JobError> {
        let mut out = Self::default();
        out.verts
            .try_reserve_exact(vertices)
            .and_then(|()| out.edges.try_reserve_exact(3 * triangles))
            .and_then(|()| {
                out.faces
                    .try_reserve_exact(if weld { triangles } else { 0 })
            })
            .map_err(|_| resource("streaming task allocation failed"))?;
        Ok(out)
    }
    /// One triangle with corner positions `k`, source vertices `v` of instance `ordinal`.
    fn triangle(
        &mut self,
        sink: &Sink<'_>,
        k: [Key; 3],
        v: [u64; 3],
        ordinal: u32,
        moved: bool,
    ) -> Result<(), JobError> {
        let index = |v: u64| {
            u32::try_from(v).map_err(|_| {
                JobError::new(
                    JobErrorKind::Unsupported,
                    "a source vertex index exceeds 2^32 - 1",
                )
            })
        };
        let corners = k.map(|c| c.map(f32::from_bits));
        self.area.add(topology::area(&corners));
        let degenerate = k[0] == k[1] || k[1] == k[2] || k[0] == k[2];
        if sink.audit.is_some() {
            self.faces.push(AuditFace {
                corners: k,
                degenerate,
                moved,
            });
        }
        if degenerate {
            self.degenerate += 1;
            // The capture-based pass joins corners 0-1 and 1-2 of every triangle.
            for (x, y) in [(k[0], k[1]), (k[1], k[2])] {
                if x != y {
                    let (a, b) = (x.min(y), x.max(y));
                    self.edges.push((
                        sink.part(&a),
                        EdgeRec {
                            a,
                            b,
                            inst: JOIN_ONLY,
                            ia: 0,
                            ib: 0,
                        },
                    ));
                }
            }
            return Ok(());
        }
        for c in 0..3 {
            let j = (c + 1) % 3;
            let (mut a, mut b) = (k[c], k[j]);
            let (mut ia, mut ib) = (index(v[c])?, index(v[j])?);
            if a > b {
                std::mem::swap(&mut a, &mut b);
                std::mem::swap(&mut ia, &mut ib);
            }
            self.edges.push((
                sink.part(&a),
                EdgeRec {
                    a,
                    b,
                    inst: ordinal,
                    ia,
                    ib,
                },
            ));
        }
        Ok(())
    }
}

/// The streaming step's shared state.
struct Stream<'a, 'd> {
    source: &'a StreamSource,
    primitives: &'a [crate::gltf_source::source::ScenePrimitive<'d>],
    instances: &'a [Instance],
    moved: Option<&'a MovedTargets<'a>>,
    /// Every expanded vertex a triangle uses (a vertex's record is written once).
    used: &'a [AtomicU64],
    sink: &'a Sink<'a>,
    workers: usize,
    /// Triangles streamed.
    done: AtomicU64,
}

impl Stream<'_, '_> {
    fn poll<'p>(
        &'p self,
        progress: &'p StepProgress<'_>,
    ) -> impl FnMut() -> Result<(), JobError> + 'p {
        move || progress.advance(self.done.load(Relaxed))
    }

    /// The position of vertex `v` of instance `i`, as capture decodes it, and whether a weld
    /// moved it.
    fn position(
        &self,
        frame: &mut SourceFrame,
        i: usize,
        positions: &mut crate::mesh_large::input::ElementReader<'_>,
        v: u64,
    ) -> Result<(Key, bool), JobError> {
        let p = &self.primitives[i];
        let world = source_position(p, positions, v)?;
        let bits = canonical(frame.vertex(world, false)?.0).map(f32::to_bits);
        match self.moved {
            Some(moved) => Ok(match moved.target(self.instances[i].scene, v)? {
                Some(target) => (target, true),
                None => (bits, false),
            }),
            None => Ok((bits, false)),
        }
    }

    /// Triangle tasks of instances `prims`.
    fn triangle_tasks(
        &self,
        prims: std::ops::Range<usize>,
    ) -> Vec<(usize, std::ops::Range<usize>)> {
        let mut tasks = Vec::new();
        for i in prims {
            let count = self.primitives[i].triangles;
            for start in (0..count).step_by(STREAM_TRIANGLES) {
                tasks.push((i, start..(start + STREAM_TRIANGLES).min(count)));
            }
        }
        tasks
    }

    /// Instances `prims`: their used vertices marked, decoded once into a position table in
    /// vertex order (each written to its partition), then their triangles read against it.
    fn batch(
        &self,
        prims: std::ops::Range<usize>,
        frame: &mut SourceFrame,
        progress: &StepProgress<'_>,
    ) -> Result<(ExactSum, u64), JobError> {
        let base = self.instances[prims.start].offset;
        let total: usize = prims
            .clone()
            .map(|i| self.primitives[i].position.count)
            .sum();
        let tasks = self.triangle_tasks(prims.clone());
        // 1. Used vertices, from every index (checked).
        let marked = run_tasks(
            tasks.len(),
            self.workers,
            &mut self.poll(progress),
            |t, stop| {
                let (i, range) = &tasks[t];
                let p = &self.primitives[*i];
                let offset = self.instances[*i].offset;
                let mut indices = p.indices.map(|v| self.source.reader(&v, PART_BUFFER));
                for triangle in range.clone() {
                    if triangle.is_multiple_of(4096) && stop.load(Relaxed) {
                        return Err(stopped());
                    }
                    for v in vertices(p, &mut indices, triangle)? {
                        let id = offset + v;
                        self.used[(id / 64) as usize].fetch_or(1 << (id % 64), Relaxed);
                    }
                }
                Ok(())
            },
        )?;
        if let Ran::Failed(error) = marked {
            return Err(error);
        }
        // 2. Used positions in vertex order, into the table.
        let mut table: Vec<Key> = Vec::new();
        table
            .try_reserve_exact(total)
            .map_err(|_| resource("position table allocation failed"))?;
        table.resize(total, [0; 3]);
        let moved_bits: Vec<AtomicU64> =
            (0..total.div_ceil(64)).map(|_| AtomicU64::new(0)).collect();
        let mut chunks: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
        for i in prims.clone() {
            let count = self.primitives[i].position.count;
            for start in (0..count).step_by(DECODE_VERTICES) {
                chunks.push((i, start..(start + DECODE_VERTICES).min(count)));
            }
        }
        let slices: Vec<Mutex<&mut [Key]>> = {
            let mut rest = table.as_mut_slice();
            let mut out = Vec::with_capacity(chunks.len());
            for (_, range) in &chunks {
                let (own, tail) = rest.split_at_mut(range.len());
                out.push(Mutex::new(own));
                rest = tail;
            }
            out
        };
        let decoded = decode_chunks(
            frame,
            0..chunks.len(),
            self.workers,
            &mut self.poll(progress),
            |c, frame, check| {
                let (i, range) = &chunks[c];
                let p = &self.primitives[*i];
                let offset = self.instances[*i].offset;
                let mut slice = slices[c].lock().unwrap_or_else(|e| e.into_inner());
                let mut positions = self.source.reader(&p.position, POSITION_WINDOW);
                let mut out = TaskOut::with_capacity(range.len(), 0, false)?;
                for v in range.clone() {
                    if v.is_multiple_of(4096) {
                        check()?;
                    }
                    let id = offset + v as u64;
                    if self.used[(id / 64) as usize].load(Relaxed) & (1 << (id % 64)) == 0 {
                        continue;
                    }
                    let (key, moved) = self.position(frame, *i, &mut positions, v as u64)?;
                    slice[v - range.start] = key;
                    if moved {
                        let local = id - base;
                        moved_bits[(local / 64) as usize].fetch_or(1 << (local % 64), Relaxed);
                    }
                    out.verts
                        .push((self.sink.part(&key), VertexRec { key, id }));
                }
                self.sink.flush(&mut out)
            },
        )?;
        drop(slices);
        if let Ran::Failed(error) = decoded {
            return Err(error);
        }
        // 3. Triangles against the table.
        let table = &table;
        let streamed = run_tasks(
            tasks.len(),
            self.workers,
            &mut self.poll(progress),
            |t, stop| {
                let (i, range) = &tasks[t];
                let p = &self.primitives[*i];
                let inst = &self.instances[*i];
                let local = to_index(inst.offset - base)?;
                let mut indices = p.indices.map(|v| self.source.reader(&v, PART_BUFFER));
                let mut out = TaskOut::with_capacity(0, range.len(), self.moved.is_some())?;
                let mut counted = range.start;
                for triangle in range.clone() {
                    if triangle.is_multiple_of(1024) {
                        if stop.load(Relaxed) {
                            return Err(stopped());
                        }
                        self.done.fetch_add((triangle - counted) as u64, Relaxed);
                        counted = triangle;
                    }
                    let v = vertices(p, &mut indices, triangle)?;
                    let mut k = [[0u32; 3]; 3];
                    let mut moved = false;
                    for c in 0..3 {
                        let at = local + to_index(v[c])?;
                        k[c] = table[at];
                        moved |= moved_bits[at / 64].load(Relaxed) & (1 << (at % 64)) != 0;
                    }
                    out.triangle(self.sink, k, v, inst.ordinal, moved)?;
                }
                self.done.fetch_add((range.end - counted) as u64, Relaxed);
                self.sink.flush(&mut out)?;
                Ok((std::mem::take(&mut out.area), out.degenerate))
            },
        )?;
        Self::sum(streamed)
    }

    /// Instance `i` alone (its table would not fit). Each task reads its triangles' vertex
    /// indices, decodes their distinct vertices once in vertex order into a table of its own,
    /// then reads its triangles against it.
    fn direct(
        &self,
        i: usize,
        frame: &mut SourceFrame,
        progress: &StepProgress<'_>,
    ) -> Result<(ExactSum, u64), JobError> {
        let tasks = self.triangle_tasks(i..i + 1);
        let weld = self.moved.is_some();
        let streamed = decode_chunks(
            frame,
            0..tasks.len(),
            self.workers,
            &mut self.poll(progress),
            |t, frame, check| {
                let (i, range) = &tasks[t];
                let p = &self.primitives[*i];
                let inst = &self.instances[*i];
                let mut indices = p.indices.map(|v| self.source.reader(&v, PART_BUFFER));
                let mut corners: Vec<[u64; 3]> = Vec::new();
                let mut ids: Vec<u64> = Vec::new();
                corners
                    .try_reserve_exact(range.len())
                    .and_then(|()| ids.try_reserve_exact(3 * range.len()))
                    .map_err(|_| resource("streaming task allocation failed"))?;
                for triangle in range.clone() {
                    if triangle.is_multiple_of(4096) {
                        check()?;
                    }
                    let v = vertices(p, &mut indices, triangle)?;
                    corners.push(v);
                    ids.extend(v);
                }
                ids.sort_unstable();
                ids.dedup();
                // Distinct vertices in vertex order: a thin spread reads a small window each.
                let span = ids.last().zip(ids.first()).map_or(0, |(hi, lo)| hi - lo);
                let window = if span <= 16 * ids.len() as u64 {
                    POSITION_WINDOW
                } else {
                    SPARSE_WINDOW
                };
                let mut positions = self.source.reader(&p.position, window);
                let mut out = TaskOut::with_capacity(ids.len(), range.len(), weld)?;
                let mut table: Vec<(Key, bool)> = Vec::new();
                table
                    .try_reserve_exact(ids.len())
                    .map_err(|_| resource("streaming task allocation failed"))?;
                for (n, &v) in ids.iter().enumerate() {
                    if n.is_multiple_of(4096) {
                        check()?;
                    }
                    let (key, moved) = self.position(frame, *i, &mut positions, v)?;
                    table.push((key, moved));
                    let id = inst.offset + v;
                    let bit = 1u64 << (id % 64);
                    if self.used[to_index(id / 64)?].fetch_or(bit, Relaxed) & bit == 0 {
                        out.verts
                            .push((self.sink.part(&key), VertexRec { key, id }));
                    }
                }
                let mut counted = range.start;
                for (triangle, v) in range.clone().zip(&corners) {
                    if triangle.is_multiple_of(1024) {
                        check()?;
                        self.done.fetch_add((triangle - counted) as u64, Relaxed);
                        counted = triangle;
                    }
                    let mut k = [[0u32; 3]; 3];
                    let mut moved = false;
                    for c in 0..3 {
                        let at = ids
                            .binary_search(&v[c])
                            .map_err(|_| state("a corner without its vertex"))?;
                        k[c] = table[at].0;
                        moved |= table[at].1;
                    }
                    out.triangle(self.sink, k, *v, inst.ordinal, moved)?;
                }
                self.done.fetch_add((range.end - counted) as u64, Relaxed);
                self.sink.flush(&mut out)?;
                Ok((std::mem::take(&mut out.area), out.degenerate))
            },
        )?;
        Self::sum(streamed)
    }

    fn sum(streamed: Ran<(ExactSum, u64)>) -> Result<(ExactSum, u64), JobError> {
        match streamed {
            Ran::Failed(error) => Err(error),
            Ran::All(found) => {
                let mut area = ExactSum::default();
                let mut degenerate = 0;
                for (a, d) in found {
                    area.merge(&a);
                    degenerate += d;
                }
                Ok((area, degenerate))
            }
        }
    }
}

#[derive(Default)]
struct VertexCounts {
    positions: u64,
    source_vertices: u64,
    repeated: u64,
    extra: u64,
    keys: Option<Sequence<KeyRec>>,
}

/// One edge group's length terms, as the capture-based pass adds them.
#[derive(Clone, Copy)]
struct Term {
    length: f64,
    primitive_open: u64,
    /// 0: open, 1: closed between primitive instances, 2: closed within one, 3: neither.
    kind: u8,
}

/// The length sums, added in edge-group order.
#[derive(Default)]
struct Sums {
    edges: f64,
    open: f64,
    coincide: f64,
    unwelded: f64,
    closes: f64,
}
impl Sums {
    fn add(&mut self, t: &Term) {
        let po = t.primitive_open as f64;
        if t.primitive_open > 0 {
            self.edges += t.length * po;
        }
        match t.kind {
            0 => self.open += t.length,
            1 => {
                self.coincide += t.length * po;
                self.closes += t.length;
            }
            2 => {
                self.unwelded += t.length * po;
                self.closes += t.length;
            }
            _ => {}
        }
    }
    fn write(&self, r: &mut TopologyReport) {
        let b = &mut r.primitive_boundaries;
        b.edges.length_m = self.edges;
        b.open.length_m = self.open;
        b.coincide_with_other_primitive.length_m = self.coincide;
        b.unwelded_within_primitive.length_m = self.unwelded;
        r.repeated_positions.closes_open_edges.length_m = self.closes;
    }
}

/// A partition's edge counts.
#[derive(Default)]
struct EdgeCounts {
    nonmanifold: u64,
    boundary: u64,
    open: u64,
    coincide: u64,
    unwelded: u64,
    closes: u64,
}
impl EdgeCounts {
    fn add_to(&self, r: &mut TopologyReport) {
        r.nonmanifold_edges += self.nonmanifold;
        let b = &mut r.primitive_boundaries;
        b.edges.edges += self.boundary;
        b.open.edges += self.open;
        b.coincide_with_other_primitive.edges += self.coincide;
        b.unwelded_within_primitive.edges += self.unwelded;
        r.repeated_positions.closes_open_edges.edges += self.closes;
    }
}

struct EdgeOut {
    counts: EdgeCounts,
    terms: Vec<Term>,
    /// Open edges and seams, in edge order.
    kept: Vec<KeptRec>,
    /// Second ends in a later partition, by key, and the joins that wait for them.
    cross: Vec<Key>,
    cross_joins: Vec<(u32, u32)>,
}

/// Union-find over one partition's slice of the groups (ids `offset..`); the smaller root
/// wins, as [`Groups`].
fn find(slice: &mut [u32], offset: u32, mut x: u32) -> u32 {
    while slice[(x - offset) as usize] != x {
        let up = slice[(slice[(x - offset) as usize] - offset) as usize];
        slice[(x - offset) as usize] = up;
        x = up;
    }
    x
}

/// Counts partition `edges` (its positions `keys`, numbered from `offset`): the capture-based
/// pass's step 3 over these groups, in order.
fn count_edges(
    keys: &Sequence<KeyRec>,
    store: &Stored,
    part: usize,
    offset: u64,
    slice: &mut [u32],
    own: &dyn Fn(&Key) -> bool,
    stop: &AtomicBool,
) -> Result<EdgeOut, JobError> {
    let offset = u32::try_from(offset).map_err(|_| state("position offset overflows"))?;
    let mut table: Vec<Key> = Vec::new();
    table
        .try_reserve_exact(to_index(keys.len())?)
        .map_err(|_| resource("position table allocation failed"))?;
    for k in keys.reader()? {
        table.push(k?.0);
    }
    let mut recs: Vec<EdgeRec> = Vec::new();
    recs.try_reserve_exact(to_index(store.edge_records[part])?)
        .map_err(|_| resource("partition allocation failed"))?;
    store.read(&store.edges[part], &mut recs, stop)?;
    recs.sort_unstable();
    // Every output is reserved exactly for at most one entry per edge group, as
    // `EDGE_WORK_BYTES` charges.
    let groups = recs.chunk_by(|x, y| (x.a, x.b) == (y.a, y.b)).count();
    let mut out = EdgeOut {
        counts: EdgeCounts::default(),
        terms: Vec::new(),
        kept: Vec::new(),
        cross: Vec::new(),
        cross_joins: Vec::new(),
    };
    out.terms
        .try_reserve_exact(groups)
        .and_then(|()| out.kept.try_reserve_exact(groups))
        .and_then(|()| out.cross.try_reserve_exact(groups))
        .and_then(|()| out.cross_joins.try_reserve_exact(groups))
        .map_err(|_| resource("partition allocation failed"))?;
    let mut at = 0usize;
    let mut i = 0;
    while i < recs.len() {
        if i % 65_536 == 0 && stop.load(Relaxed) {
            return Err(stopped());
        }
        let (a, b) = (recs[i].a, recs[i].b);
        let mut j = i;
        while j < recs.len() && recs[j].a == a && recs[j].b == b {
            j += 1;
        }
        while table.get(at).is_some_and(|k| *k < a) {
            at += 1;
        }
        if table.get(at) != Some(&a) {
            return Err(state("an edge end without its position"));
        }
        let ia = offset + to_u32(at)?;
        let (ib, cross) = if own(&b) {
            let local = table
                .binary_search(&b)
                .map_err(|_| state("an edge end without its position"))?;
            let ib = offset + to_u32(local)?;
            let (ra, rb) = (find(slice, offset, ia), find(slice, offset, ib));
            if ra != rb {
                let (lo, hi) = (ra.min(rb), ra.max(rb));
                slice[(hi - offset) as usize] = lo;
            }
            (ib, false)
        } else {
            let c = u32::try_from(out.cross.len()).map_err(|_| resource("too many cross edges"))?;
            out.cross.push(b);
            out.cross_joins.push((ia, c));
            (c, true)
        };
        let group = &recs[i..j];
        i = j;
        let real = group.iter().take_while(|r| r.inst != JOIN_ONLY).count();
        if real == 0 {
            continue;
        }
        let group = &group[..real];
        // Index edges used by exactly one face of their primitive instance.
        let mut primitive_open = 0u64;
        let mut s = 0;
        while s < group.len() {
            let mut t = s;
            let id = |r: &EdgeRec| (r.inst, r.ia, r.ib);
            while t < group.len() && id(&group[t]) == id(&group[s]) {
                t += 1;
            }
            if t - s == 1 {
                primitive_open += 1;
            }
            s = t;
        }
        let faces = group.len() as u64;
        let between = group[group.len() - 1].inst != group[0].inst;
        let l = topology::length(a, b);
        let kept = KeptRec {
            a: ia,
            b: ib,
            cross,
            between,
            seam: false,
            pa: a,
            pb: b,
        };
        let c = &mut out.counts;
        if faces > 2 {
            c.nonmanifold += 1;
        }
        c.boundary += primitive_open;
        let kind = if faces == 1 {
            c.open += 1;
            out.kept.push(kept);
            0
        } else if primitive_open > 0 {
            if between {
                c.coincide += primitive_open;
            } else {
                c.unwelded += primitive_open;
            }
            c.closes += 1;
            out.kept.push(KeptRec { seam: true, ..kept });
            if between {
                1
            } else {
                2
            }
        } else {
            3
        };
        if primitive_open > 0 || kind != 3 {
            out.terms.push(Term {
                length: l,
                primitive_open,
                kind,
            });
        }
    }
    Ok(out)
}

/// The position id of every cross key, in one ordered pass over the partitions' positions.
fn resolve(
    cross: &[Key],
    keys: &[Sequence<KeyRec>],
    progress: &StepProgress<'_>,
) -> Result<Vec<u32>, JobError> {
    let mut wanted: Vec<(Key, u32)> = Vec::new();
    wanted
        .try_reserve_exact(cross.len())
        .map_err(|_| resource("cross table allocation failed"))?;
    for (i, k) in cross.iter().enumerate() {
        wanted.push((*k, to_u32(i)?));
    }
    wanted.sort_unstable();
    let mut resolved = vec![u32::MAX; cross.len()];
    let mut next = wanted.iter().peekable();
    let mut id = 0u32;
    'parts: for part in keys {
        if next.peek().is_none() {
            break;
        }
        for (i, k) in part.reader()?.enumerate() {
            if i % 65_536 == 0 {
                progress.check()?;
            }
            let k = k?.0;
            while let Some(&&(want, c)) = next.peek() {
                if want < k {
                    return Err(state("a cross edge end without its position"));
                }
                if want > k {
                    break;
                }
                resolved[c as usize] = id;
                next.next();
            }
            id = id.wrapping_add(1);
            if next.peek().is_none() {
                break 'parts;
            }
        }
    }
    if next.peek().is_some() {
        return Err(state("a cross edge end without its position"));
    }
    Ok(resolved)
}

/// SplitMix64's finaliser: a fixed 64-bit mix (the sample's jitter).
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Splitters of `parts` key ranges: quantiles of a sample of corner keys, one corner from each
/// of [`SAMPLES_PER_PART`] per partition (within bounds) equal strata of the triangles, at a
/// fixed pseudo-random triangle and corner in each, so a regular grid cannot alias it. The
/// keys are positions before any weld, and a corner that does not decode is left out: the
/// split only balances the partitions, and the stream reports failures.
fn sample(
    source: &StreamSource,
    frame: &mut SourceFrame,
    primitives: &[crate::gltf_source::source::ScenePrimitive<'_>],
    n: u64,
    parts: u64,
    progress: &StepProgress<'_>,
) -> Result<Vec<Key>, JobError> {
    let count = parts
        .saturating_mul(SAMPLES_PER_PART)
        .clamp(MIN_SAMPLES, MAX_SAMPLES)
        .min(n);
    progress.begin(Step::InspectSample, count)?;
    let mut keys: Vec<Key> = Vec::with_capacity(to_index(count)?);
    let mut first = 0u64;
    let mut j = 0u64;
    for p in primitives {
        let end = first + p.triangles as u64;
        let mut positions = source.reader(&p.position, SPARSE_WINDOW);
        let mut indices = p.indices.map(|v| source.reader(&v, SPARSE_WINDOW));
        while j < count {
            let h = mix(j);
            // Stratum `j` of `count`, at a jitter of `h` within it.
            let at = (u128::from(j) << 32 | u128::from(h >> 32)) * u128::from(n)
                / (u128::from(count) << 32);
            let t = u64::try_from(at).map_err(|_| state("sample index overflows"))?;
            if t >= end {
                break;
            }
            if j.is_multiple_of(1024) {
                progress.advance(j)?;
            }
            j += 1;
            let corner = to_index(h % 3)?;
            let key = vertices(p, &mut indices, to_index(t - first)?)
                .and_then(|v| source_position(p, &mut positions, v[corner]))
                .and_then(|world| frame.vertex(world, false));
            if let Ok((position, _)) = key {
                keys.push(canonical(position).map(f32::to_bits));
            }
        }
        first = end;
    }
    keys.sort_unstable();
    let parts = parts.max(1).min(keys.len().max(1) as u64);
    let mut splitters: Vec<Key> = Vec::new();
    for q in 1..parts {
        splitters.push(keys[to_index(q * keys.len() as u64 / parts)?]);
    }
    splitters.dedup();
    progress.end()?;
    Ok(splitters)
}
