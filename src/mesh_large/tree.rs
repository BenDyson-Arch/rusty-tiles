//! Streaming cell tree, octant packing, degenerate run hierarchy and exact vertex leaf depth.
//! No tile payload or approximation is produced here.
use super::{
    config::TreeConfig,
    records::{prefix, Bounds, NodeKind, NodeRecord, Span, TriangleRecord, KEY_DEPTH},
};
use crate::{
    spill::{
        ExternalSorter, Record, Scratch, Sequence, SequenceWriter, SortObserver, SortedStream,
    },
    JobError, JobErrorKind,
};
use serde::Serialize;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct WeldDepth {
    /// Normalized f32 POSITION bits, shared across primitives and instances.
    pub position: [u32; 3],
    pub depth: u8,
}
impl Record for WeldDepth {
    const SIZE: usize = 13;
    fn encode(&self, out: &mut [u8]) {
        for (i, v) in self.position.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        out[12] = self.depth;
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            position: std::array::from_fn(|i| {
                u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap())
            }),
            depth: bytes[12],
        }
    }
}

/// Sort by welded position, with a unique rank/corner tie-breaker; reduction reads ALL corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct VertexDepth {
    position: [u32; 3],
    rank: u64,
    corner: u8,
    depth: u8,
}
impl Record for VertexDepth {
    const SIZE: usize = 22;
    fn encode(&self, out: &mut [u8]) {
        for (i, v) in self.position.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        out[12..20].copy_from_slice(&self.rank.to_le_bytes());
        out[20] = self.corner;
        out[21] = self.depth;
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            position: std::array::from_fn(|i| {
                u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap())
            }),
            rank: u64::from_le_bytes(bytes[12..20].try_into().unwrap()),
            corner: bytes[20],
            depth: bytes[21],
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DepthBounds {
    pub depth: u32,
    pub bounds: Bounds,
    /// Largest component extent of any individual tile at this depth, evaluated in f64.
    pub maximum_extent: f64,
}
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct TreeStats {
    pub triangles: u64,
    pub nodes: u64,
    pub leaves: u64,
    pub internal: u64,
    /// Number of levels, including the root.
    pub height: u32,
    pub welded_vertices: u64,
    pub degenerate_cells: u64,
    pub per_depth: Vec<DepthBounds>,
}
#[cfg_attr(not(test), allow(dead_code))] // Retained for bounded unit-test fixtures; production uses the streamed facade.
pub(crate) struct TreeOutput {
    pub triangles: Sequence<TriangleRecord>,
    /// Preorder, sorted by (first rank, depth).
    pub nodes: Sequence<NodeRecord>,
    pub weld_depths: Sequence<WeldDepth>,
    pub stats: TreeStats,
}

/// The tree a full capture retains for leaves and coarse levels. It stores no triangles: it is
/// built over the rank-ordered full-triangle sequence ([`build_ranked`]), which embeds every
/// [`TriangleRecord`] field and which every later stage reads.
pub(crate) struct RetainedTree {
    /// Preorder, sorted by (first rank, depth).
    pub nodes: Sequence<NodeRecord>,
    pub weld_depths: Sequence<WeldDepth>,
    pub stats: TreeStats,
}

/// Rank-ordered triangles that can be read from the first rank any number of times: the cell
/// pass reads them once and the weld pass again. H1 builds over the copy it stores; full
/// capture builds over its retained full-triangle sequence and stores no projection.
pub(crate) trait RankedTriangles: Sync {
    fn ranked(&self) -> Result<RankedReader<'_>, JobError>;
    /// Ranks `[first, end)` only.
    fn range(&self, first: u64, end: u64) -> Result<RankedReader<'_>, JobError>;
    /// The Morton key of the triangle at `rank`.
    fn key_at(&self, rank: u64) -> Result<u64, JobError>;
    fn count(&self) -> u64;
}
pub(crate) type RankedReader<'a> = Box<dyn Iterator<Item = Result<TriangleRecord, JobError>> + 'a>;
impl RankedTriangles for Sequence<TriangleRecord> {
    fn ranked(&self) -> Result<RankedReader<'_>, JobError> {
        Ok(Box::new(self.reader()?.map(|row| row.map_err(Into::into))))
    }
    fn range(&self, first: u64, end: u64) -> Result<RankedReader<'_>, JobError> {
        let rows = Sequence::range(self, first, end, self.buffer_bytes())?;
        Ok(Box::new(rows.map(|row| row.map_err(Into::into))))
    }
    fn key_at(&self, rank: u64) -> Result<u64, JobError> {
        let (mut encoded, mut out) = (Vec::new(), Vec::new());
        self.random()?.read(rank, 1, &mut encoded, &mut out)?;
        Ok(out[0].key)
    }
    fn count(&self) -> u64 {
        self.len()
    }
}

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}
fn unsupported(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::Unsupported, message.into())
}
fn fits(span: &Span, config: &TreeConfig) -> bool {
    span.count <= config.leaf_triangles() && span.bytes <= config.tile_bytes()
}
fn node(
    span: Span,
    depth: u8,
    cell: u64,
    path_len: u8,
    path: u64,
    kind: NodeKind,
    children: u8,
) -> NodeRecord {
    NodeRecord {
        cell_depth: depth,
        cell,
        path_len,
        path,
        last_octant: if path_len > 0 {
            (path & 7) as u8
        } else {
            (cell & 7) as u8
        },
        kind,
        children,
        first: span.first,
        end: span.end,
        bytes: span.bytes,
        bounds: span.bounds,
    }
}
fn span_of(node: &NodeRecord) -> Span {
    Span {
        count: node.triangles(),
        bytes: node.bytes,
        first: node.first,
        end: node.end,
        bounds: node.bounds,
    }
}

/// Where closed tree nodes go: the node sort, or one parallel subtree's vector.
pub(crate) trait NodeSink {
    fn push(&mut self, node: NodeRecord) -> Result<(), JobError>;
}
impl NodeSink for ExternalSorter<'_, NodeRecord> {
    fn push(&mut self, node: NodeRecord) -> Result<(), JobError> {
        ExternalSorter::push(self, node).map_err(Into::into)
    }
}
impl NodeSink for Vec<NodeRecord> {
    fn push(&mut self, node: NodeRecord) -> Result<(), JobError> {
        Vec::push(self, node);
        Ok(())
    }
}

struct OpenCell {
    depth: u8,
    cell: u64,
    span: Span,
    children: [Option<NodeRecord>; 8],
}
impl OpenCell {
    fn new(depth: u8, cell: u64, first: u64) -> Self {
        Self {
            depth,
            cell,
            span: Span::empty(first),
            children: [None; 8],
        }
    }
    fn append(&mut self, child: NodeRecord) -> Result<(), JobError> {
        let slot = child.slot() as usize;
        if self.children[slot].is_some() {
            return Err(invalid("duplicate child slot in cell tree"));
        }
        self.span.extend(&span_of(&child));
        self.children[slot] = Some(child);
        Ok(())
    }
    fn close(self, config: &TreeConfig, out: &mut dyn NodeSink) -> Result<NodeRecord, JobError> {
        if fits(&self.span, config) {
            return Ok(node(
                self.span,
                self.depth,
                self.cell,
                0,
                0,
                NodeKind::Leaf,
                0,
            ));
        }
        let mut packed: Option<NodeRecord> = None;
        let mut children = 0u8;
        for child in self.children.into_iter().flatten() {
            if child.kind == NodeKind::Internal {
                if let Some(run) = packed.take() {
                    out.push(run)?;
                    children += 1;
                }
                children += 1;
                continue;
            }
            if let Some(run) = packed.as_mut() {
                let mut combined = span_of(run);
                combined.extend(&span_of(&child));
                if fits(&combined, config) {
                    run.end = combined.end;
                    run.bytes = combined.bytes;
                    run.bounds = combined.bounds;
                    run.last_octant = child.slot();
                    continue;
                }
            }
            if let Some(run) = packed.replace(child) {
                out.push(run)?;
                children += 1;
            }
        }
        if let Some(run) = packed {
            out.push(run)?;
            children += 1;
        }
        let closed = node(
            self.span,
            self.depth,
            self.cell,
            0,
            0,
            NodeKind::Internal,
            children,
        );
        out.push(closed)?;
        Ok(closed)
    }
}

/// One depth-21 key. A single fitting span needs no extra spill file. On overflow the prior
/// maximal run is spilled, and every later run is handled with constant resident state.
struct KeyGroup<'a> {
    key: u64,
    total: Span,
    current: Span,
    runs: Option<SequenceWriter<Span>>,
    scratch: &'a Scratch,
    buffer: usize,
}
impl<'a> KeyGroup<'a> {
    fn new(key: u64, first: u64, scratch: &'a Scratch, buffer: usize) -> Self {
        Self {
            key,
            total: Span::empty(first),
            current: Span::empty(first),
            runs: None,
            scratch,
            buffer,
        }
    }
    fn push(
        &mut self,
        triangle: &TriangleRecord,
        rank: u64,
        config: &TreeConfig,
    ) -> Result<(), JobError> {
        if u64::from(triangle.bytes) > config.tile_bytes() {
            return Err(unsupported(format!(
                "single triangle exceeds projected tile byte budget: {:?}, {} > {}",
                triangle.source,
                triangle.bytes,
                config.tile_bytes()
            )));
        }
        let next = Span {
            count: 1,
            bytes: u64::from(triangle.bytes),
            first: rank,
            end: rank + 1,
            bounds: triangle.bounds(),
        };
        let mut combined = self.current;
        combined.extend(&next);
        if !fits(&combined, config) {
            if self.runs.is_none() {
                self.runs = Some(SequenceWriter::new(self.scratch, "cell-runs", self.buffer)?);
            }
            self.runs.as_mut().unwrap().push(&self.current)?;
            self.current = next;
        } else {
            self.current = combined;
        }
        self.total.extend(&next);
        Ok(())
    }
    fn close(
        mut self,
        config: &TreeConfig,
        out: &mut dyn NodeSink,
        stats: &mut TreeStats,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<NodeRecord, JobError> {
        check()?;
        let Some(mut writer) = self.runs.take() else {
            return Ok(node(
                self.total,
                KEY_DEPTH,
                self.key,
                0,
                0,
                NodeKind::Leaf,
                0,
            ));
        };
        writer.push(&self.current)?;
        let runs = writer.finish()?;
        stats.degenerate_cells += 1;
        let mut levels = 1u8;
        let mut capacity = 8u64;
        while capacity < runs.len() {
            capacity = capacity
                .checked_mul(8)
                .ok_or_else(|| unsupported("run hierarchy exceeds representable height"))?;
            levels += 1;
        }
        let height = u32::from(KEY_DEPTH) + u32::from(levels) + 1;
        if height > config.height_cap {
            return Err(unsupported(format!(
                "hierarchy height {height} > {}; degenerate cell box {:?}",
                config.height_cap, self.total.bounds
            )));
        }
        // All leaves have `levels` base-8 digits. The tail is left-filled and each synthetic
        // node is the recursive group of up to eight nodes of the preceding level.
        let mut stack: Vec<RunOpen> = Vec::with_capacity(levels as usize);
        for length in 0..levels {
            stack.push(RunOpen::new(length, 0, self.total.first));
        }
        let mut previous = 0u64;
        for (index, span) in runs.reader()?.enumerate() {
            let index = index as u64;
            let span = span?;
            if index > 0 {
                let common = common_run_prefix(previous, index, levels);
                while stack.len() > common as usize + 1 {
                    let closed = stack.pop().unwrap().close(self.key, out)?;
                    stack.last_mut().unwrap().append(closed);
                }
                for length in common + 1..levels {
                    stack.push(RunOpen::new(
                        length,
                        index >> (3 * (levels - length)),
                        span.first,
                    ));
                }
            }
            let leaf = node(span, KEY_DEPTH, self.key, levels, index, NodeKind::Leaf, 0);
            out.push(leaf)?;
            stack.last_mut().unwrap().append(leaf);
            previous = index;
            if index.is_multiple_of(2048) {
                check()?;
            }
        }
        while stack.len() > 1 {
            let closed = stack.pop().unwrap().close(self.key, out)?;
            stack.last_mut().unwrap().append(closed);
        }
        stack.pop().unwrap().close(self.key, out)
    }
}
struct RunOpen {
    length: u8,
    path: u64,
    span: Span,
    children: u8,
}
impl RunOpen {
    fn new(length: u8, path: u64, first: u64) -> Self {
        Self {
            length,
            path,
            span: Span::empty(first),
            children: 0,
        }
    }
    fn append(&mut self, child: NodeRecord) {
        self.span.extend(&span_of(&child));
        self.children += 1;
        debug_assert!(self.children <= 8);
    }
    fn close(self, cell: u64, out: &mut dyn NodeSink) -> Result<NodeRecord, JobError> {
        let record = node(
            self.span,
            KEY_DEPTH,
            cell,
            self.length,
            self.path,
            NodeKind::Internal,
            self.children,
        );
        out.push(record)?;
        Ok(record)
    }
}
fn common_run_prefix(a: u64, b: u64, length: u8) -> u8 {
    (0..length)
        .take_while(|d| a >> (3 * (length - d)) == b >> (3 * (length - d)))
        .count()
        .saturating_sub(1) as u8
}
fn common_cell_prefix(a: u64, b: u64) -> u8 {
    (0..=KEY_DEPTH)
        .take_while(|d| prefix(a, *d) == prefix(b, *d))
        .last()
        .unwrap_or(0)
}

#[cfg_attr(not(test), allow(dead_code))] // Retained for bounded unit-test fixtures; production uses the streamed facade.
/// H1's tree over a sorted stream; the rank-ordered triangles are stored and kept.
pub(crate) fn build(
    scratch: &Scratch,
    triangles: SortedStream<'_, TriangleRecord>,
    config: &TreeConfig,
    jobs: usize,
    weld_stage_need_bytes: Option<u64>,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<TreeOutput, JobError> {
    let stored = SequenceWriter::new(scratch, "triangles", config.sort.buffer_bytes)?;
    let mut input = triangles.map(|row| row.map_err(Into::into));
    let (triangles, nodes, weld_depths, stats) = build_in(
        scratch,
        &mut input,
        Some(stored),
        None,
        config,
        jobs,
        weld_stage_need_bytes,
        None,
        check,
    )?;
    Ok(TreeOutput {
        triangles: triangles.ok_or_else(|| invalid("H1 tree stored no triangles"))?,
        nodes,
        weld_depths,
        stats,
    })
}

/// The tree over `ranked`, which must be in strict Morton/provenance order (checked); no
/// copy of the triangles is stored. `observer` receives both sorts' checkpoints. With more
/// than one job, the cell pass runs as parallel subtrees ([`parallel_cells`]).
pub(crate) fn build_ranked(
    scratch: &Scratch,
    ranked: &dyn RankedTriangles,
    config: &TreeConfig,
    jobs: usize,
    weld_stage_need_bytes: Option<u64>,
    observer: Option<&dyn SortObserver>,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<RetainedTree, JobError> {
    let mut input = ranked.ranked()?;
    let (_, nodes, weld_depths, stats) = build_in(
        scratch,
        &mut input,
        None,
        Some(ranked),
        config,
        jobs,
        weld_stage_need_bytes,
        observer,
        check,
    )?;
    Ok(RetainedTree {
        nodes,
        weld_depths,
        stats,
    })
}

fn observe<'s, T: Record>(
    sorter: ExternalSorter<'s, T>,
    observer: Option<&'s dyn SortObserver>,
) -> ExternalSorter<'s, T> {
    match observer {
        Some(observer) => sorter.observed(observer),
        None => sorter,
    }
}

type Built = (
    Option<Sequence<TriangleRecord>>,
    Sequence<NodeRecord>,
    Sequence<WeldDepth>,
    TreeStats,
);
/// Cells over `triangles`, written to `stored` when given; the weld pass then rereads
/// `stored`, or `ranked` (the same triangles) when nothing is stored.
#[allow(clippy::too_many_arguments)]
fn build_in(
    scratch: &Scratch,
    triangles: &mut dyn Iterator<Item = Result<TriangleRecord, JobError>>,
    mut stored: Option<SequenceWriter<TriangleRecord>>,
    ranked: Option<&dyn RankedTriangles>,
    config: &TreeConfig,
    jobs: usize,
    weld_stage_need_bytes: Option<u64>,
    observer: Option<&dyn SortObserver>,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Built, JobError> {
    check()?;
    if config.leaf_triangles() == 0 || config.tile_bytes() == 0 || config.height_cap == 0 {
        return Err(JobError::new(
            JobErrorKind::InvalidRequest,
            "tree budgets must be nonzero",
        ));
    }
    let mut nodes = observe(ExternalSorter::new(scratch, config.sorter(jobs))?, observer);
    let mut stats = TreeStats::default();
    // Over a re-readable rank order (full capture), the cell pass runs as parallel subtrees.
    let root = match ranked {
        Some(ranked) if jobs > 1 => parallel_cells(
            scratch,
            ranked,
            config,
            jobs,
            FRONTIER_SPLIT,
            &mut nodes,
            &mut stats,
            check,
        )?,
        _ => {
            let mut cells = CellBuilder::new(scratch, config, 0, 0);
            for triangle in triangles {
                let triangle = triangle?;
                cells.push(&triangle, &mut nodes, check)?;
                if let Some(stored) = stored.as_mut() {
                    stored.push(&triangle)?;
                }
            }
            let (root, built) = cells.finish(&mut nodes, check)?;
            stats.triangles = built.triangles;
            stats.degenerate_cells = built.degenerate_cells;
            root
        }
    };
    if root.kind == NodeKind::Leaf {
        nodes.push(root)?;
    }
    let stored = stored.map(SequenceWriter::finish).transpose()?;
    check()?;
    let mut ordered_nodes = SequenceWriter::new(scratch, "tree", config.sort.buffer_bytes)?;
    for record in nodes.finish()? {
        let record = record?;
        let depth = record.depth();
        if depth + 1 > config.height_cap {
            return Err(unsupported(format!(
                "hierarchy height {} > {}; outlier box {:?}",
                depth + 1,
                config.height_cap,
                root.bounds
            )));
        }
        stats.nodes += 1;
        stats.height = stats.height.max(depth + 1);
        match record.kind {
            NodeKind::Leaf => stats.leaves += 1,
            NodeKind::Internal => stats.internal += 1,
        };
        while stats.per_depth.len() <= depth as usize {
            let d = stats.per_depth.len() as u32;
            stats.per_depth.push(DepthBounds {
                depth: d,
                bounds: Bounds::EMPTY,
                maximum_extent: 0.,
            });
        }
        let summary = &mut stats.per_depth[depth as usize];
        summary.bounds.union(&record.bounds);
        for axis in 0..3 {
            summary.maximum_extent = summary
                .maximum_extent
                .max(f64::from(record.bounds.max[axis]) - f64::from(record.bounds.min[axis]));
        }
        ordered_nodes.push(&record)?;
        if stats.nodes.is_multiple_of(4096) {
            check()?;
        }
    }
    let nodes = ordered_nodes.finish()?;
    check()?;
    // Stage-entry recheck of the weld pass. A caller sharing the Scratch (H8 full capture)
    // passes that stage's declared working set, which includes its retained files;
    // standalone H1 rechecks its own whole need.
    let stage_need = match weld_stage_need_bytes {
        Some(need) => need,
        None => super::config::scratch_need(
            stats.triangles,
            root.bytes,
            config,
            scratch.allocation_unit_bytes(),
        )
        .ok_or_else(|| {
            JobError::new(JobErrorKind::ResourceLimit, "H1 scratch bound exceeds u64")
        })?,
    };
    super::config::preflight_remaining(scratch, stage_need)?;
    let ranked: &dyn RankedTriangles = match (&stored, ranked) {
        (Some(stored), _) => stored,
        (None, Some(ranked)) => ranked,
        (None, None) => return Err(invalid("tree weld pass has no ranked triangles")),
    };
    let weld_depths = vertex_depths(
        scratch,
        ranked,
        stats.triangles,
        &nodes,
        config,
        jobs,
        observer,
        check,
    )?;
    stats.welded_vertices = weld_depths.len();
    check()?;
    Ok((stored, nodes, weld_depths, stats))
}

/// What a cell pass built besides its nodes.
struct CellTotals {
    triangles: u64,
    degenerate_cells: u64,
}

/// The cells over a run of triangles in strict order that all lie in one cell at `base` (the
/// whole tree for `base == 0`): the grouping by key, degenerate runs and octant packing of the
/// cell tree. Ranks count from `first`.
struct CellBuilder<'a> {
    scratch: &'a Scratch,
    config: &'a TreeConfig,
    base: u8,
    rank: u64,
    group: Option<KeyGroup<'a>>,
    cells: Vec<OpenCell>,
    previous: Option<TriangleRecord>,
    stats: TreeStats,
}

impl<'a> CellBuilder<'a> {
    fn new(scratch: &'a Scratch, config: &'a TreeConfig, base: u8, first: u64) -> Self {
        Self {
            scratch,
            config,
            base,
            rank: first,
            group: None,
            cells: Vec::with_capacity(KEY_DEPTH as usize),
            previous: None,
            stats: TreeStats::default(),
        }
    }

    fn push(
        &mut self,
        triangle: &TriangleRecord,
        out: &mut dyn NodeSink,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<(), JobError> {
        if self
            .previous
            .as_ref()
            .is_some_and(|old| old.cmp(triangle) != Ordering::Less)
        {
            return Err(invalid(
                "triangle order is not strict Morton/provenance order",
            ));
        }
        if self.group.as_ref().is_some_and(|g| g.key != triangle.key) {
            let old = self.group.take().unwrap();
            let old_key = old.key;
            let closed = old.close(self.config, out, &mut self.stats, check)?;
            self.cells.last_mut().unwrap().append(closed)?;
            let common = common_cell_prefix(old_key, triangle.key);
            while usize::from(self.base) + self.cells.len() > common as usize + 1 {
                let closed = self.cells.pop().unwrap().close(self.config, out)?;
                self.cells.last_mut().unwrap().append(closed)?;
            }
        }
        if self.group.is_none() {
            for depth in self.base + self.cells.len() as u8..KEY_DEPTH {
                self.cells
                    .push(OpenCell::new(depth, prefix(triangle.key, depth), self.rank));
            }
            self.group = Some(KeyGroup::new(
                triangle.key,
                self.rank,
                self.scratch,
                self.config.sort.buffer_bytes,
            ));
        }
        self.group
            .as_mut()
            .unwrap()
            .push(triangle, self.rank, self.config)?;
        self.rank = self.rank.checked_add(1).ok_or_else(|| {
            JobError::new(JobErrorKind::ResourceLimit, "triangle count overflows u64")
        })?;
        self.previous = Some(*triangle);
        if self.rank.is_multiple_of(4096) {
            check()?;
        }
        Ok(())
    }

    /// Closes every open cell; the returned node is the cell at `base` (the root for 0).
    fn finish(
        mut self,
        out: &mut dyn NodeSink,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<(NodeRecord, CellTotals), JobError> {
        let Some(group) = self.group.take() else {
            return Err(JobError::new(
                JobErrorKind::InvalidInput,
                "scene contains no triangles",
            ));
        };
        let closed = group.close(self.config, out, &mut self.stats, check)?;
        self.cells.last_mut().unwrap().append(closed)?;
        while self.cells.len() > 1 {
            let closed = self.cells.pop().unwrap().close(self.config, out)?;
            self.cells.last_mut().unwrap().append(closed)?;
        }
        let top = self.cells.pop().unwrap().close(self.config, out)?;
        Ok((
            top,
            CellTotals {
                triangles: self.rank,
                degenerate_cells: self.stats.degenerate_cells,
            },
        ))
    }
}

/// Deepest cell the parallel frontier splits to, and the smallest cell it splits.
const FRONTIER_DEPTH: u8 = 18;
const FRONTIER_SPLIT: u64 = 1 << 16;

/// The cell pass over `ranked` as parallel subtrees. A frontier of disjoint octree cells
/// covering the rank space (the largest cells split until there are about 16 per worker,
/// found by binary search on the sorted keys) is built cell by cell on `jobs` threads with
/// [`CellBuilder`]; the levels above the frontier then receive the closed frontier cells in
/// rank order and close exactly as one pass would. Every node record is the one the serial
/// pass writes, so the sorted node stream is identical. `check` runs on the calling thread
/// while the subtrees build; the workers stop at the scratch's stop probe or a failure.
#[allow(clippy::too_many_arguments)]
fn parallel_cells(
    scratch: &Scratch,
    ranked: &dyn RankedTriangles,
    config: &TreeConfig,
    jobs: usize,
    split: u64,
    nodes: &mut ExternalSorter<'_, NodeRecord>,
    stats: &mut TreeStats,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<NodeRecord, JobError> {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
        Mutex,
    };
    let count = ranked.count();
    if count == 0 {
        return Err(JobError::new(
            JobErrorKind::InvalidInput,
            "scene contains no triangles",
        ));
    }
    // First rank in [lo, hi) whose key has a depth-`depth` prefix of at least `cell`.
    let lower = |depth: u8, cell: u64, mut lo: u64, mut hi: u64| -> Result<u64, JobError> {
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if prefix(ranked.key_at(mid)?, depth) < cell {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    };
    // (depth, cell, first, end), in rank order.
    let mut frontier: Vec<(u8, u64, u64, u64)> = vec![(0, 0, 0, count)];
    let target = jobs.saturating_mul(16);
    while frontier.len() < target {
        let Some(at) = frontier
            .iter()
            .enumerate()
            .filter(|(_, c)| c.0 < FRONTIER_DEPTH && c.3 - c.2 > split)
            .max_by_key(|(_, c)| c.3 - c.2)
            .map(|(at, _)| at)
        else {
            break;
        };
        check()?;
        let (depth, cell, first, end) = frontier[at];
        let mut children = Vec::with_capacity(8);
        let mut start = first;
        for octant in 0..8u64 {
            let child = cell << 3 | octant;
            let stop = if octant == 7 {
                end
            } else {
                lower(depth + 1, child + 1, start, end)?
            };
            if stop > start {
                children.push((depth + 1, child, start, stop));
            }
            start = stop;
        }
        frontier.splice(at..at + 1, children);
    }
    type Job = Result<(NodeRecord, Vec<NodeRecord>, CellTotals), JobError>;
    let results: Vec<Mutex<Option<Job>>> = frontier.iter().map(|_| Mutex::new(None)).collect();
    let (next, stop) = (AtomicUsize::new(0), AtomicBool::new(false));
    let stopped = std::thread::scope(|scope| {
        let worker = || loop {
            let task = next.fetch_add(1, Relaxed);
            if task >= frontier.len() || stop.load(Relaxed) {
                break;
            }
            let (depth, _, first, end) = frontier[task];
            let job = (|| -> Job {
                let mut out = Vec::new();
                let mut cells = CellBuilder::new(scratch, config, depth, first);
                let mut halt = || -> Result<(), JobError> {
                    crate::test_hooks::pace("mesh.tree-subtree");
                    if stop.load(Relaxed) {
                        return Err(JobError::new(JobErrorKind::Cancelled, "tree stopped"));
                    }
                    scratch.check_stop().map_err(Into::into)
                };
                for triangle in ranked.range(first, end)? {
                    cells.push(&triangle?, &mut out, &mut halt)?;
                }
                let (top, built) = cells.finish(&mut out, &mut halt)?;
                if built.triangles != end {
                    return Err(invalid("tree subtree read the wrong number of triangles"));
                }
                Ok((top, out, built))
            })();
            if job.is_err() {
                stop.store(true, Relaxed);
            }
            *results[task].lock().unwrap_or_else(|e| e.into_inner()) = Some(job);
        };
        let mut threads = Vec::new();
        let mut stopped = None;
        for _ in 0..jobs.clamp(1, frontier.len()) {
            match std::thread::Builder::new()
                .name("rusty-tiles-tree".into())
                .spawn_scoped(scope, worker)
            {
                Ok(thread) => threads.push(thread),
                Err(error) => {
                    stop.store(true, Relaxed);
                    stopped = Some(JobError::new(
                        JobErrorKind::ResourceLimit,
                        format!("start tree worker: {error}"),
                    ));
                    break;
                }
            }
        }
        while stopped.is_none() && !threads.iter().all(|t| t.is_finished()) {
            if let Err(error) = check() {
                stop.store(true, Relaxed);
                stopped = Some(error);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        for thread in threads {
            if let Err(panic) = thread.join() {
                std::panic::resume_unwind(panic);
            }
        }
        stopped
    });
    if let Some(error) = stopped {
        return Err(error);
    }
    // The levels above the frontier, closed in rank order as one pass closes them.
    let mut top: Vec<OpenCell> = Vec::new();
    let mut last_key: Option<u64> = None;
    let mut root = None;
    for (result, &(depth, cell, first, _)) in results.into_iter().zip(&frontier) {
        let (closed, out, built) = result
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
            .ok_or_else(|| invalid("tree subtree did not run"))??;
        stats.degenerate_cells += built.degenerate_cells;
        for node in out {
            nodes.push(node)?;
        }
        if depth == 0 {
            root = Some(closed);
            continue;
        }
        let key = cell << (3 * u32::from(KEY_DEPTH - depth));
        if let Some(last) = last_key {
            let common = common_cell_prefix(last, key);
            while top.len() > common as usize + 1 {
                let c = top.pop().unwrap().close(config, nodes)?;
                top.last_mut().unwrap().append(c)?;
            }
        }
        for d in top.len() as u8..depth {
            top.push(OpenCell::new(d, prefix(key, d), first));
        }
        top.last_mut().unwrap().append(closed)?;
        last_key = Some(key);
    }
    stats.triangles = count;
    match root {
        Some(root) => Ok(root),
        None => {
            while top.len() > 1 {
                let c = top.pop().unwrap().close(config, nodes)?;
                top.last_mut().unwrap().append(c)?;
            }
            top.pop()
                .ok_or_else(|| invalid("tree has no root"))?
                .close(config, nodes)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn vertex_depths(
    scratch: &Scratch,
    triangles: &dyn RankedTriangles,
    count: u64,
    nodes: &Sequence<NodeRecord>,
    config: &TreeConfig,
    jobs: usize,
    observer: Option<&dyn SortObserver>,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Sequence<WeldDepth>, JobError> {
    let mut leaves = nodes.reader()?.filter_map(|n| match n {
        Ok(n) if n.kind == NodeKind::Leaf => Some(Ok(n)),
        Ok(_) => None,
        Err(e) => Some(Err(e)),
    });
    let mut leaf = leaves
        .next()
        .transpose()?
        .ok_or_else(|| invalid("tree has no leaves"))?;
    let mut sorter = observe(ExternalSorter::new(scratch, config.sorter(jobs))?, observer);
    let mut read = 0_u64;
    for (rank, triangle) in triangles.ranked()?.enumerate() {
        let rank = rank as u64;
        let triangle = triangle?;
        if rank >= leaf.end {
            leaf = leaves
                .next()
                .transpose()?
                .ok_or_else(|| invalid("leaves do not cover triangle ranks"))?;
        }
        if rank < leaf.first || rank >= leaf.end {
            return Err(invalid("leaf rank ranges are not contiguous"));
        }
        for (corner, position) in triangle.positions.iter().enumerate() {
            sorter.push(VertexDepth {
                position: position.map(f32::to_bits),
                rank,
                corner: corner as u8,
                depth: leaf.depth() as u8,
            })?;
        }
        if rank.is_multiple_of(1365) {
            check()?;
        }
        read = rank + 1;
    }
    if read != count || leaf.end != count || leaves.next().transpose()?.is_some() {
        return Err(invalid("leaves do not cover exactly the triangle ranks"));
    }
    check()?;
    let mut out = SequenceWriter::new(scratch, "weld-depth", config.sort.buffer_bytes)?;
    let mut current: Option<WeldDepth> = None;
    for (i, record) in sorter.finish()?.enumerate() {
        let record = record?;
        match current.as_mut() {
            Some(weld) if weld.position == record.position => {
                weld.depth = weld.depth.min(record.depth)
            }
            _ => {
                if let Some(weld) = current.replace(WeldDepth {
                    position: record.position,
                    depth: record.depth,
                }) {
                    out.push(&weld)?;
                }
            }
        }
        if i.is_multiple_of(4096) {
            check()?;
        }
    }
    if let Some(weld) = current {
        out.push(&weld)?;
    }
    Ok(out.finish()?)
}

#[cfg(test)]
mod tests {
    use super::super::{config::Profile, records::Provenance};
    use super::*;
    use std::collections::BTreeMap;

    fn triangle(key: u64, id: u64) -> TriangleRecord {
        TriangleRecord {
            key,
            source: Provenance {
                triangle: id,
                ..Default::default()
            },
            vertices: [0, 1, 2],
            positions: [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            bytes: 56,
            degenerate: false,
        }
    }
    fn inspect(
        input: Vec<TriangleRecord>,
        config: &TreeConfig,
        jobs: usize,
    ) -> (Vec<NodeRecord>, Vec<WeldDepth>) {
        let scratch = Scratch::create(None, 0).unwrap();
        let mut sorter = ExternalSorter::new(&scratch, config.sorter(jobs)).unwrap();
        sorter.extend(input).unwrap();
        let output = build(
            &scratch,
            sorter.finish().unwrap(),
            config,
            jobs,
            None,
            &mut || Ok(()),
        )
        .unwrap();
        let nodes = output
            .nodes
            .reader()
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let weld = output
            .weld_depths
            .reader()
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        drop(output);
        assert_eq!(scratch.live_bytes(), 0);
        (nodes, weld)
    }
    /// The parallel cell pass writes the serial pass's node records, for any frontier and job
    /// count: clustered keys, coincident-key degenerate cells, packing across siblings, empty
    /// octants and keys spread over the whole range.
    #[test]
    fn parallel_cells_equal_the_serial_pass() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut input: Vec<TriangleRecord> = Vec::new();
        for id in 0..6000u64 {
            let key = match id % 5 {
                // Clusters under a few depth-3 cells.
                0 | 1 => ((next() % 4) << 54) | (next() >> 12) & ((1 << 54) - 1),
                // Many triangles sharing one key: a degenerate run hierarchy.
                2 => 7 << 60,
                // Spread over the whole key range.
                _ => next() >> 1,
            };
            input.push(triangle(key, id));
        }
        input.sort();
        for leaf_triangles in [3, 40, 700] {
            let config = TreeConfig::new(Profile::Quality).with_leaf_triangles(leaf_triangles);
            let scratch = Scratch::create(None, 0).unwrap();
            let mut writer = SequenceWriter::new(&scratch, "ranked", 1 << 12).unwrap();
            for t in &input {
                writer.push(t).unwrap();
            }
            let ranked = writer.finish().unwrap();
            let collect = |nodes: ExternalSorter<'_, NodeRecord>| -> Vec<NodeRecord> {
                nodes.finish().unwrap().map(Result::unwrap).collect()
            };
            let mut serial_nodes = ExternalSorter::new(&scratch, config.sorter(1)).unwrap();
            let mut cells = CellBuilder::new(&scratch, &config, 0, 0);
            for t in &input {
                cells.push(t, &mut serial_nodes, &mut || Ok(())).unwrap();
            }
            let (root, built) = cells.finish(&mut serial_nodes, &mut || Ok(())).unwrap();
            if root.kind == NodeKind::Leaf {
                serial_nodes.push(root).unwrap();
            }
            let serial = collect(serial_nodes);
            for (jobs, split) in [(2, 1), (3, 50), (8, 1), (8, 1 << 20)] {
                let mut nodes = ExternalSorter::new(&scratch, config.sorter(jobs)).unwrap();
                let mut stats = TreeStats::default();
                let root = parallel_cells(
                    &scratch,
                    &ranked,
                    &config,
                    jobs,
                    split,
                    &mut nodes,
                    &mut stats,
                    &mut || Ok(()),
                )
                .unwrap();
                if root.kind == NodeKind::Leaf {
                    nodes.push(root).unwrap();
                }
                assert_eq!(
                    collect(nodes),
                    serial,
                    "leaf {leaf_triangles} jobs {jobs} split {split}"
                );
                assert_eq!(stats.triangles, built.triangles);
                assert_eq!(stats.degenerate_cells, built.degenerate_cells);
            }
        }
    }

    #[test]
    fn packing_stops_at_an_internal_octant_and_crosses_empty_slots() {
        let config = TreeConfig::new(Profile::Quality).with_leaf_triangles(2);
        let input = vec![
            triangle(0, 0),
            triangle(1 << 60, 1),
            triangle(2 << 60, 2),
            triangle((2 << 60) | (1 << 57), 3),
            triangle((2 << 60) | (2 << 57), 4),
            triangle(3 << 60, 5),
            triangle(5 << 60, 6),
        ];
        let (nodes, weld) = inspect(input, &config, 1);
        let children: Vec<_> = nodes
            .iter()
            .filter(|n| n.parent() == Some((0, 0, 0, 0)))
            .collect();
        assert_eq!(children.len(), 3);
        assert_eq!(
            (
                children[0].slot(),
                children[0].last_octant,
                children[0].triangles()
            ),
            (0, 1, 2)
        );
        assert_eq!(children[1].kind, NodeKind::Internal);
        assert_eq!(
            (
                children[2].slot(),
                children[2].last_octant,
                children[2].triangles()
            ),
            (3, 5, 2)
        );
        assert!(weld.iter().all(|v| v.depth == 1));
    }
    #[test]
    fn coincident_keys_have_unique_eight_ary_paths_and_exact_rank_coverage() {
        let config = TreeConfig::new(Profile::Quality).with_leaf_triangles(1);
        let input = (0..85).map(|id| triangle(0, id)).collect();
        let (nodes, weld) = inspect(input, &config, 1);
        let mut identities = BTreeMap::new();
        let mut ranks = Vec::new();
        for node in &nodes {
            assert!(identities.insert(node.identity(), node).is_none());
            if node.kind == NodeKind::Leaf {
                ranks.extend(node.first..node.end);
            }
        }
        assert_eq!(ranks, (0..85).collect::<Vec<_>>());
        for node in &nodes {
            if node.kind == NodeKind::Internal {
                let children: Vec<_> = nodes
                    .iter()
                    .filter(|child| child.parent() == Some(node.identity()))
                    .collect();
                assert_eq!(children.len(), node.children as usize);
                assert!(children.len() <= 8);
                let mut slots: Vec<_> = children.iter().map(|child| child.slot()).collect();
                slots.sort();
                slots.dedup();
                assert_eq!(slots.len(), children.len());
            }
        }
        assert_eq!(nodes.iter().map(NodeRecord::depth).max(), Some(24));
        assert!(weld.iter().all(|v| v.depth == 24));
    }
    #[test]
    fn node_and_exact_weld_outputs_ignore_run_size_fan_in_and_jobs() {
        let input: Vec<_> = (0..40)
            .rev()
            .map(|id| triangle(if id % 3 == 0 { 0 } else { 1 << 60 }, id))
            .collect();
        let base = TreeConfig::new(Profile::Quality).with_leaf_triangles(2);
        let expected = inspect(input.clone(), &base, 1);
        for (bytes, fan, jobs) in [(1024, 2, 1), (2048, 3, 2), (4096, 8, 2)] {
            let mut config = base.clone();
            config.sort.run_bytes = bytes;
            config.sort.fan_in = fan;
            config.sort.buffer_bytes = 31;
            assert_eq!(inspect(input.clone(), &config, jobs), expected);
        }
    }
    #[test]
    fn hierarchy_refusal_drops_all_candidate_files() {
        let config = TreeConfig::new(Profile::Quality)
            .with_leaf_triangles(1)
            .with_height_cap(22);
        let scratch = Scratch::create(None, 0).unwrap();
        let mut sorter = ExternalSorter::new(&scratch, config.sorter(1)).unwrap();
        sorter.extend((0..9).map(|id| triangle(0, id))).unwrap();
        let err = match build(
            &scratch,
            sorter.finish().unwrap(),
            &config,
            1,
            None,
            &mut || Ok(()),
        ) {
            Ok(_) => panic!("expected refusal"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), JobErrorKind::Unsupported);
        assert!(err.to_string().contains("hierarchy height 24 > 22"));
        assert_eq!(scratch.live_bytes(), 0);
    }
}
