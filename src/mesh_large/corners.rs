//! Each corner's welded depth: the least leaf depth over every corner at the same position
//! (f32 bits; capture canonicalises zeros), written in rank order.
//!
//! A position held by corners of two leaves lies inside both leaves' boxes (a leaf's box is
//! the union of its triangles' corners) and in an occupied cell of both leaves' occupancy
//! masks. So a corner whose position no other leaf's box and mask can hold is welded only
//! inside its own leaf, and its depth is its leaf's. Only the other corners, the candidates,
//! are sorted by position. Every corner at a shared position is itself a candidate, so each
//! position group of the sort holds every corner at that position, and its least depth is
//! the welded depth. Candidates whose position turns out to be held by one leaf only form
//! groups of one depth and change nothing.
use super::{
    attributes::FullTriangle,
    config::TreeConfig,
    full::CornerDepth,
    phases::{Step, StepProgress},
    records::{Bounds, NodeKind, NodeRecord, TriangleRecord},
};
use crate::{
    spill::{ExternalSorter, Layout, Record, Scratch, Sequence, SequenceWriter},
    JobError, JobErrorKind,
};
use rayon::prelude::*;

/// Occupancy mask resolution per axis of a leaf's box.
const SIDE: usize = 16;
const MASK_WORDS: usize = SIDE * SIDE * SIDE / 64;
/// Leaves are read in chunks of at most about this many triangles (plus one leaf), one chunk
/// per task, at least eight chunks per worker when the triangles allow.
const CHUNK_TRIANGLES: u64 = 1 << 14;

/// The most the join's chunk tasks hold at once beside its sorts, for `jobs` workers and
/// leaves of at most `leaf_triangles`: one wave of one chunk per worker, each holding one
/// leaf's triangles and their corners and either its candidates (every corner of the chunk
/// at worst) or, in the mask pass, a mask per leaf (a leaf per triangle at worst).
pub(crate) fn wave_bytes(jobs: usize, leaf_triangles: u64) -> Option<u64> {
    let chunk = CHUNK_TRIANGLES.checked_add(leaf_triangles)?;
    let candidates = chunk.checked_mul(3 * std::mem::size_of::<Candidate>() as u64)?;
    let masks = chunk.checked_mul(std::mem::size_of::<[u64; MASK_WORDS]>() as u64)?;
    let leaf = leaf_triangles.checked_mul(
        std::mem::size_of::<FullTriangle>() as u64
            + 3 * std::mem::size_of::<([u32; 3], u64, u8)>() as u64,
    )?;
    (jobs.max(1) as u64).checked_mul(candidates.max(masks).checked_add(leaf)?)
}

/// Test seam: how many leaves of the tree `nodes` each grid level lists, and how many are
/// tested for every position.
#[cfg(test)]
pub(crate) fn grid_levels(nodes: &Sequence<NodeRecord>) -> (Vec<usize>, usize) {
    let leaves: Vec<Leaf> = nodes
        .reader()
        .unwrap()
        .map(Result::unwrap)
        .filter(|n| n.kind == NodeKind::Leaf)
        .map(|n| Leaf {
            first: n.first,
            end: n.end,
            depth: n.depth() as u8,
            bounds: n.bounds,
        })
        .collect();
    let grid = Grid::new(&leaves, &mut || Ok(())).unwrap();
    let mut listed = vec![0; grid.levels.len()];
    for (i, level) in grid.levels.iter().enumerate() {
        let mut ids = level.ids.clone();
        ids.sort_unstable();
        ids.dedup();
        listed[i] = ids.len();
    }
    (listed, grid.huge.len())
}
/// A leaf box spanning more grid cells than this is tested for every position instead.
const LARGE_SPAN: usize = 64;

/// A rank-ordered triangle record with corner positions: capture's full triangles, or H1's
/// triangle records (unit tests).
pub(super) trait Corners: Record + Send + Sync {
    fn positions(&self) -> [[f32; 3]; 3];
}
impl Corners for FullTriangle {
    fn positions(&self) -> [[f32; 3]; 3] {
        self.triangle.positions
    }
}
impl Corners for TriangleRecord {
    fn positions(&self) -> [[f32; 3]; 3] {
        self.positions
    }
}

fn invariant(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}

#[derive(Clone, Copy)]
struct Leaf {
    first: u64,
    end: u64,
    depth: u8,
    bounds: Bounds,
}

/// The occupancy-mask bit of `p` in a box: the box's 16^3 cell holding `p`, or `None` when
/// `p` lies outside the box. The same function builds the masks and queries them, so a
/// corner of a leaf always finds its own bit.
fn mask_bit(bounds: &Bounds, p: [f32; 3]) -> Option<usize> {
    let mut cell = [0; 3];
    for axis in 0..3 {
        let (min, max) = (bounds.min[axis], bounds.max[axis]);
        if !(min <= p[axis] && p[axis] <= max) {
            return None;
        }
        cell[axis] = axis_cell(min, max, f64::from(p[axis]));
    }
    Some(cell[0] + SIDE * (cell[1] + SIDE * cell[2]))
}

/// The mask cell along one axis of a box `[min, max]` holding `x`. Non-decreasing in `x`
/// (correctly rounded arithmetic, then a floor), so the cells of the points of an interval lie
/// between the cells of its ends.
fn axis_cell(min: f32, max: f32, x: f64) -> usize {
    let extent = f64::from(max) - f64::from(min);
    if extent > 0. {
        let x = ((x - f64::from(min)) / extent * SIDE as f64).floor();
        (x.max(0.) as usize).min(SIDE - 1)
    } else {
        0
    }
}

/// Uniform grids over the leaves' boxes, each eight times coarser than the one below: a box
/// is listed in each cell of the finest level where it spans at most [`LARGE_SPAN`] cells, and
/// a box too large for every level is listed once, in `huge`, and tested for every position.
struct Grid {
    levels: Vec<Level>,
    huge: Vec<u32>,
}

/// One grid level: for each cell, the leaves whose box meets it.
struct Level {
    origin: [f64; 3],
    side: f64,
    dims: [usize; 3],
    offsets: Vec<usize>,
    ids: Vec<u32>,
}

/// Grid levels above the finest.
const COARSER_LEVELS: usize = 2;

impl Grid {
    fn new(
        leaves: &[Leaf],
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Self, JobError> {
        let mut all = Bounds::EMPTY;
        for leaf in leaves {
            all.union(&leaf.bounds);
        }
        let origin = all.min.map(f64::from);
        let extent: [f64; 3] = std::array::from_fn(|a| (f64::from(all.max[a]) - origin[a]).max(0.));
        let dims_at = |side: f64| -> [usize; 3] {
            std::array::from_fn(|a| (extent[a] / side).floor() as usize + 1)
        };
        let cells = |dims: [usize; 3]| dims.iter().product::<usize>();
        // About four cells per leaf: start from cubes that give at most that many along the
        // longest axis, then shrink while the count stays within twice it (flat sets).
        let target = leaves.len().max(1) * 4;
        let longest = extent.iter().copied().fold(0., f64::max);
        let mut side = if longest > 0. {
            longest / (target as f64).cbrt()
        } else {
            1.
        };
        for _ in 0..64 {
            let smaller = side * 0.8;
            if cells(dims_at(smaller)) > target * 2 {
                break;
            }
            side = smaller;
        }
        let mut members: Vec<u32> = (0..leaves.len() as u32).collect();
        let mut levels = Vec::new();
        for _ in 0..=COARSER_LEVELS {
            if members.is_empty() {
                break;
            }
            check()?;
            let (level, larger) = Level::new(leaves, &members, origin, side, dims_at(side));
            levels.push(level);
            members = larger;
            side *= 8.;
        }
        check()?;
        Ok(Self {
            levels,
            huge: members,
        })
    }

    /// The leaves whose box may hold `p`.
    fn near(&self, p: [f32; 3]) -> impl Iterator<Item = u32> + '_ {
        self.levels
            .iter()
            .flat_map(move |level| level.near(p))
            .chain(self.huge.iter().copied())
    }
}

impl Level {
    /// The level over `members` of `leaves`, and the members whose box spans more than
    /// [`LARGE_SPAN`] of its cells (left for coarser levels).
    fn new(
        leaves: &[Leaf],
        members: &[u32],
        origin: [f64; 3],
        side: f64,
        dims: [usize; 3],
    ) -> (Self, Vec<u32>) {
        let cells = dims.iter().product::<usize>();
        let mut level = Self {
            origin,
            side,
            dims,
            offsets: vec![0; cells + 1],
            ids: Vec::new(),
        };
        let mut placed = Vec::with_capacity(members.len());
        let mut larger = Vec::new();
        for &i in members {
            let bounds = &leaves[i as usize].bounds;
            let (lo, hi) = (level.cell(bounds.min), level.cell(bounds.max));
            let span: usize = (0..3).map(|a| hi[a] - lo[a] + 1).product();
            if span > LARGE_SPAN {
                larger.push(i);
            } else {
                placed.push((i, lo, hi));
            }
        }
        let each = |lo: [usize; 3], hi: [usize; 3], f: &mut dyn FnMut(usize)| {
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        f(x + dims[0] * (y + dims[1] * z));
                    }
                }
            }
        };
        for &(_, lo, hi) in &placed {
            each(lo, hi, &mut |c| level.offsets[c + 1] += 1);
        }
        for c in 0..cells {
            level.offsets[c + 1] += level.offsets[c];
        }
        let mut fill = level.offsets.clone();
        level.ids = vec![0; level.offsets[cells]];
        for &(i, lo, hi) in &placed {
            each(lo, hi, &mut |c| {
                level.ids[fill[c]] = i;
                fill[c] += 1;
            });
        }
        (level, larger)
    }

    /// The cell of `p`, clamped to the level. Monotone in each coordinate, so a point of a box
    /// lies in a cell of the box's span.
    fn cell(&self, p: [f32; 3]) -> [usize; 3] {
        std::array::from_fn(|a| {
            let x = ((f64::from(p[a]) - self.origin[a]) / self.side).floor();
            (x.max(0.) as usize).min(self.dims[a] - 1)
        })
    }

    fn near(&self, p: [f32; 3]) -> impl Iterator<Item = u32> + '_ {
        let [x, y, z] = self.cell(p);
        let c = x + self.dims[0] * (y + self.dims[1] * z);
        self.ids[self.offsets[c]..self.offsets[c + 1]]
            .iter()
            .copied()
    }
}

/// Each leaf's occupancy mask: the cells of its box that hold one of its corners.
struct Index<'a> {
    leaves: &'a [Leaf],
    masks: Vec<[u64; MASK_WORDS]>,
    grid: Grid,
}

impl Index<'_> {
    /// Whether a leaf other than `own` has a corner in the mask cell of its box holding `p`.
    fn shared(&self, own: u32, p: [f32; 3]) -> bool {
        self.grid.near(p).any(|i| {
            i != own
                && mask_bit(&self.leaves[i as usize].bounds, p)
                    .is_some_and(|bit| self.masks[i as usize][bit / 64] & (1 << (bit % 64)) != 0)
        })
    }
}

/// A corner at a position another leaf may hold, ordered by position and then depth, so the
/// first record of each position group carries the group's least depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Candidate {
    position: [u32; 3],
    depth: u8,
    rank: u64,
    corner: u8,
}
impl Record for Candidate {
    const SIZE: usize = 18;
    fn encode(&self, out: &mut [u8]) {
        debug_assert!(self.rank < super::full::RANK_LIMIT && self.corner < 4 && self.depth < 64);
        for a in 0..3 {
            out[4 * a..4 * a + 4].copy_from_slice(&self.position[a].to_le_bytes());
        }
        out[12..17].copy_from_slice(&self.rank.to_le_bytes()[..5]);
        out[17] = self.corner | self.depth << 2;
    }
    fn decode(bytes: &[u8]) -> Self {
        let mut rank = [0u8; 8];
        rank[..5].copy_from_slice(&bytes[12..17]);
        Self {
            position: std::array::from_fn(|a| {
                u32::from_le_bytes(bytes[4 * a..4 * a + 4].try_into().unwrap())
            }),
            rank: u64::from_le_bytes(rank),
            corner: bytes[17] & 3,
            depth: bytes[17] >> 2,
        }
    }
}

/// A corner whose welded depth is below its leaf's, sorted back to rank order. Encoded in 6
/// bytes: the 40-bit rank and `corner | depth << 2` (a depth is below the tree height, at
/// most 32).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct RankDepth {
    rank: u64,
    corner: u8,
    depth: u8,
}
impl Record for RankDepth {
    const SIZE: usize = 6;
    fn encode(&self, out: &mut [u8]) {
        debug_assert!(self.rank < super::full::RANK_LIMIT && self.corner < 4 && self.depth < 64);
        out[..5].copy_from_slice(&self.rank.to_le_bytes()[..5]);
        out[5] = self.corner | self.depth << 2;
    }
    fn decode(bytes: &[u8]) -> Self {
        let mut rank = [0u8; 8];
        rank[..5].copy_from_slice(&bytes[..5]);
        Self {
            rank: u64::from_le_bytes(rank),
            corner: bytes[5] & 3,
            depth: bytes[5] >> 2,
        }
    }
}

/// Leaf chunks of about `size` triangles each (at least one leaf).
fn chunks(leaves: &[Leaf], size: u64) -> Vec<std::ops::Range<usize>> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < leaves.len() {
        let (mut end, mut triangles) = (start, 0);
        while end < leaves.len() && (end == start || triangles < size) {
            triangles += leaves[end].end - leaves[end].first;
            end += 1;
        }
        chunks.push(start..end);
        start = end;
    }
    chunks
}

/// Runs `task` over `chunks` on `pool`, a wave of `wave` chunks at a time, handing each
/// result to `sink` in chunk order. `progress` checks between waves; tasks check the run's
/// stop themselves.
fn in_waves<T: Send>(
    pool: &rayon::ThreadPool,
    chunks: &[std::ops::Range<usize>],
    wave: usize,
    task: impl Fn(std::ops::Range<usize>) -> Result<T, JobError> + Sync,
    mut sink: impl FnMut(&std::ops::Range<usize>, T) -> Result<(), JobError>,
) -> Result<(), JobError> {
    for wave in chunks.chunks(wave.max(1)) {
        let done: Vec<T> = pool.install(|| {
            wave.par_iter()
                .map(|range| task(range.clone()))
                .collect::<Result<_, _>>()
        })?;
        for (range, result) in wave.iter().zip(done) {
            sink(range, result)?;
        }
    }
    Ok(())
}

/// A visit of one leaf (its index and its triangles) during a chunk read.
type LeafVisit<'a, T> = dyn FnMut(usize, &[T]) -> Result<(), JobError> + 'a;

/// Each corner's welded depth, in rank order, for the tree whose preorder `nodes` cover
/// `triangles` (rank order) with leaves.
pub(super) fn corner_depths<T: Corners>(
    scratch: &Scratch,
    nodes: &Sequence<NodeRecord>,
    height: u32,
    triangles: &Sequence<T>,
    config: &TreeConfig,
    jobs: usize,
    progress: &StepProgress<'_>,
) -> Result<(Sequence<CornerDepth>, LeafIndex), JobError> {
    let count = triangles.len();
    let mut leaves = Vec::new();
    for node in nodes.reader()? {
        let node = node?;
        if node.kind != NodeKind::Leaf {
            continue;
        }
        if node.first != leaves.last().map_or(0, |l: &Leaf| l.end) || node.end <= node.first {
            return Err(invariant("leaf rank ranges are not contiguous"));
        }
        if node.depth() >= height {
            return Err(invariant("leaf depth exceeds the tree height"));
        }
        leaves.push(Leaf {
            first: node.first,
            end: node.end,
            depth: node.depth() as u8,
            bounds: node.bounds,
        });
    }
    if leaves.last().map_or(0, |l| l.end) != count || leaves.len() > u32::MAX as usize {
        return Err(invariant("leaves do not cover exactly the triangle ranks"));
    }
    // One unit per triangle for each of the mask, candidate and write passes.
    progress.begin(Step::CornerDepths, count.saturating_mul(3))?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.max(1))
        .thread_name(|i| format!("rusty-tiles-corners-{i}"))
        .build()
        .map_err(|e| JobError::new(JobErrorKind::ResourceLimit, format!("corner pool: {e}")))?;
    let size = (count / (jobs.max(1) as u64 * 8)).clamp(1, CHUNK_TRIANGLES);
    let chunks = chunks(&leaves, size);
    // One chunk per worker per wave ([`wave_bytes`]).
    let wave = jobs.max(1);
    let buffer = config.sort.buffer_bytes;
    // Reads a chunk's leaves, one leaf's triangles at a time, checking the run's stop.
    let read =
        |range: &std::ops::Range<usize>, each: &mut LeafVisit<'_, T>| -> Result<(), JobError> {
            let (first, end) = (leaves[range.start].first, leaves[range.end - 1].end);
            let mut reader = triangles.range(first, end, buffer)?;
            let mut leaf = Vec::new();
            for i in range.clone() {
                crate::test_hooks::pace("mesh.corner-candidates");
                scratch.check_stop()?;
                leaf.clear();
                for _ in leaves[i].first..leaves[i].end {
                    leaf.push(
                        reader
                            .next()
                            .transpose()?
                            .ok_or_else(|| invariant("triangles end before the leaves"))?,
                    );
                }
                each(i, &leaf)?;
            }
            Ok(())
        };
    // Masks.
    let mut masks = Vec::with_capacity(leaves.len());
    in_waves(
        &pool,
        &chunks,
        wave,
        |range| {
            let mut out = Vec::with_capacity(range.len());
            read(&range, &mut |i, triangles| {
                let mut mask = [0u64; MASK_WORDS];
                for p in triangles.iter().flat_map(Corners::positions) {
                    let bit = mask_bit(&leaves[i].bounds, p)
                        .ok_or_else(|| invariant("corner outside its leaf's bounds"))?;
                    mask[bit / 64] |= 1 << (bit % 64);
                }
                out.push(mask);
                Ok(())
            })?;
            Ok(out)
        },
        |range, found| {
            masks.extend(found);
            progress.advance(leaves[range.end - 1].end)
        },
    )?;
    let index = Index {
        grid: Grid::new(&leaves, &mut || progress.check())?,
        leaves: &leaves,
        masks,
    };
    // Candidates: one query per distinct position of a leaf.
    let mut candidates =
        ExternalSorter::<Candidate>::new(scratch, config.sorter(jobs))?.observed(progress);
    in_waves(
        &pool,
        &chunks,
        wave,
        |range| {
            let mut out = Vec::new();
            let mut corners: Vec<([u32; 3], u64, u8)> = Vec::new();
            read(&range, &mut |i, triangles| {
                let leaf = &leaves[i];
                corners.clear();
                for (rank, t) in (leaf.first..).zip(triangles) {
                    for (corner, p) in t.positions().iter().enumerate() {
                        corners.push((p.map(f32::to_bits), rank, corner as u8));
                    }
                }
                corners.sort_unstable();
                for group in corners.chunk_by(|a, b| a.0 == b.0) {
                    if index.shared(i as u32, group[0].0.map(f32::from_bits)) {
                        out.extend(group.iter().map(|&(position, rank, corner)| Candidate {
                            position,
                            depth: leaf.depth,
                            rank,
                            corner,
                        }));
                    }
                }
                Ok(())
            })?;
            Ok(out)
        },
        |range, found| {
            for candidate in found {
                candidates.push(candidate)?;
            }
            progress.advance(count + leaves[range.end - 1].end)
        },
    )?;
    drop(pool);
    // Join: every candidate deeper than its position's least depth is patched to it.
    let mut patches =
        ExternalSorter::<RankDepth>::new(scratch, config.sorter(jobs))?.observed(progress);
    let mut least: Option<Candidate> = None;
    for (i, candidate) in candidates.finish()?.enumerate() {
        let candidate = candidate?;
        match least {
            Some(l) if l.position == candidate.position => {
                if candidate.depth > l.depth {
                    patches.push(RankDepth {
                        rank: candidate.rank,
                        corner: candidate.corner,
                        depth: l.depth,
                    })?;
                }
            }
            _ => least = Some(candidate),
        }
        if i % 4096 == 0 {
            progress.check()?;
        }
    }
    let mut patches = patches.finish()?.peekable();
    // Every corner's depth in rank order: its leaf's, unless patched.
    let mut writer = SequenceWriter::<CornerDepth>::compressed(
        scratch,
        "corner-depths",
        config.sort.buffer_bytes,
        Layout::FULL,
    )?;
    let mut least_depths = Vec::with_capacity(leaves.len());
    for leaf in &leaves {
        let mut least = leaf.depth;
        for rank in leaf.first..leaf.end {
            let mut depths = [leaf.depth; 3];
            while let Some(patch) = patches.next_if(|p| p.as_ref().is_ok_and(|p| p.rank == rank)) {
                let patch = patch?;
                depths[patch.corner as usize] = patch.depth;
                least = least.min(patch.depth);
            }
            writer.push(&CornerDepth { depths })?;
            if rank % 4096 == 0 {
                progress.advance(2 * count + rank)?;
            }
        }
        least_depths.push(least);
        let stuck = match patches.peek() {
            Some(Ok(p)) => p.rank < leaf.end,
            Some(Err(_)) => true,
            None => false,
        };
        if stuck {
            return Err(match patches.next() {
                Some(Err(error)) => error.into(),
                _ => invariant("corner depth patches are not in rank order"),
            });
        }
    }
    if patches.next().is_some() {
        return Err(invariant("corner depth patch past the last rank"));
    }
    let written = writer.finish()?;
    progress.end()?;
    let Index { masks, .. } = index;
    Ok((
        written,
        LeafIndex {
            masks,
            least_depths,
        },
    ))
}

/// What the corner join leaves for the incidence pass, per leaf in DFS order: its occupancy
/// mask and the least welded depth of its corners (the coarsest lattice step they snap to).
pub(crate) struct LeafIndex {
    masks: Vec<[u64; MASK_WORDS]>,
    least_depths: Vec<u8>,
}

impl LeafIndex {
    /// Resident bytes per leaf.
    pub(crate) const LEAF_BYTES: u64 = (std::mem::size_of::<[u64; MASK_WORDS]>() + 1) as u64;

    pub(crate) fn len(&self) -> usize {
        self.masks.len()
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.len() as u64 * Self::LEAF_BYTES
    }

    /// Test seam: each leaf's least welded depth.
    #[cfg(test)]
    pub(crate) fn least_depths(&self) -> &[u8] {
        &self.least_depths
    }
}

/// Bytes per leaf of [`Holders`] beside the [`LeafIndex`] it borrows: the leaf's box, its
/// reach, and its grid entries (at most [`LARGE_SPAN`] ids, and about eight cell offsets).
pub(crate) const HOLDERS_LEAF_BYTES: u64 = (std::mem::size_of::<Leaf>()
    + std::mem::size_of::<f64>()
    + LARGE_SPAN * std::mem::size_of::<u32>()
    + 8 * std::mem::size_of::<usize>()) as u64;

/// Which leaves may hold a point that their corners reach once snapped.
///
/// A leaf's corner `q` of welded depth `d` is published at `c = canonical(q, d)`: on each
/// axis `k·s` for the lattice step `s` of `d` and `k = round(q / s)`, with no rounding (a
/// power-of-two `s` below `q`'s f32 spacing divides `q`, so `c = q`; otherwise `k·s` is a
/// multiple of `s` within `s / 2` of `q`, which f32 holds). So `|c - q| <= s / 2 <= reach`,
/// the half step of the leaf's least welded depth (steps shrink with depth). In float
/// profiles `c` equals `q` and the reach is zero.
///
/// A leaf holding `c` therefore has a corner in its box within `reach` of `c`: its box grown
/// by `reach` holds `c`, and its mask has a set cell between the cells of `c - reach` and
/// `c + reach`. Both tests stay conservative in floating point: rounding is monotone, so
/// `q >= c - reach` gives `q >= fl(c - reach)`, grown boxes are rounded outward to f32, and
/// mask cells are monotone in the coordinate. A leaf failing either test does not hold `c`.
pub(crate) struct Holders<'a> {
    leaves: Vec<Leaf>,
    /// Half the lattice step of each leaf's least welded depth; zero in float profiles,
    /// where the canonical value equals the source value.
    half_steps: Vec<f64>,
    grid: Grid,
    index: &'a LeafIndex,
}

impl<'a> Holders<'a> {
    /// The holders over `boxes` (each leaf's box, in DFS order) and `steps[d]`, the lattice
    /// step of depth `d` (`None` in float profiles).
    pub(crate) fn new(
        index: &'a LeafIndex,
        boxes: &[Bounds],
        steps: Option<&[f32]>,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Self, JobError> {
        if boxes.len() != index.len() || index.least_depths.len() != index.len() {
            return Err(invariant("leaf masks do not match the leaves"));
        }
        let mut leaves = Vec::with_capacity(boxes.len());
        let mut half_steps = Vec::with_capacity(boxes.len());
        let mut grown = Vec::with_capacity(boxes.len());
        for (i, bounds) in boxes.iter().enumerate() {
            if i % 4096 == 0 {
                check()?;
            }
            // Half a power-of-two step: exact in f32.
            let half = match steps {
                Some(steps) => {
                    let step = steps
                        .get(usize::from(index.least_depths[i]))
                        .ok_or_else(|| invariant("leaf depth outside the lattice"))?;
                    step / 2.
                }
                None => 0.,
            };
            let leaf = Leaf {
                first: 0,
                end: 0,
                depth: 0,
                bounds: *bounds,
            };
            // The grown box, rounded outward: every canonical value of the leaf lies inside
            // it. A rounded f32 sum is within half a unit of the exact one, so one step
            // outward bounds it.
            let mut out = *bounds;
            for axis in 0..3 {
                out.min[axis] = (bounds.min[axis] - half).next_down();
                out.max[axis] = (bounds.max[axis] + half).next_up();
            }
            grown.push(Leaf {
                bounds: out,
                ..leaf
            });
            leaves.push(leaf);
            half_steps.push(f64::from(half));
        }
        Ok(Self {
            grid: Grid::new(&grown, check)?,
            leaves,
            half_steps,
            index,
        })
    }

    /// Whether a leaf other than `own` may hold the canonical point `c`.
    pub(crate) fn other(&self, own: u32, c: [f32; 3]) -> bool {
        self.grid
            .near(c)
            .any(|i| i != own && self.may_hold(i as usize, c))
    }

    /// Whether leaf `i` has an occupied mask cell within its reach of `c`.
    fn may_hold(&self, i: usize, c: [f32; 3]) -> bool {
        let bounds = &self.leaves[i].bounds;
        let mut lo = [0; 3];
        let mut hi = [0; 3];
        for axis in 0..3 {
            let x = f64::from(c[axis]);
            let reach = self.half_steps[i];
            let (min, max) = (bounds.min[axis], bounds.max[axis]);
            let (from, to) = (x - reach, x + reach);
            if to < f64::from(min) || from > f64::from(max) {
                return false;
            }
            lo[axis] = axis_cell(min, max, from.max(f64::from(min)));
            hi[axis] = axis_cell(min, max, to.min(f64::from(max)));
        }
        let mask = &self.index.masks[i];
        (lo[2]..=hi[2]).any(|z| {
            (lo[1]..=hi[1]).any(|y| {
                (lo[0]..=hi[0]).any(|x| {
                    let bit = x + SIDE * (y + SIDE * z);
                    mask[bit / 64] & (1 << (bit % 64)) != 0
                })
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_bits_cover_their_boxes() {
        let bounds = Bounds {
            min: [0., -1., 5.],
            max: [1., 1., 5.],
        };
        assert_eq!(mask_bit(&bounds, [0., -1., 5.]), Some(0));
        assert_eq!(
            mask_bit(&bounds, [1., 1., 5.]),
            Some(SIDE - 1 + SIDE * (SIDE - 1))
        );
        assert_eq!(mask_bit(&bounds, [1.5, 0., 5.]), None);
        assert_eq!(mask_bit(&bounds, [0.5, 0., 5.0001]), None);
    }

    /// The packed records keep their limits: the largest rank, every corner and depths up to
    /// the tree height cap.
    #[test]
    fn packed_records_round_trip_at_their_limits() {
        let top = (super::super::config::HEIGHT_CAP - 1) as u8;
        for rank in [
            0,
            1,
            u64::from(u32::MAX) + 7,
            super::super::full::RANK_LIMIT - 1,
        ] {
            for corner in 0..3 {
                for depth in [0, 1, 20, top] {
                    let row = RankDepth {
                        rank,
                        corner,
                        depth,
                    };
                    let mut bytes = [0u8; RankDepth::SIZE];
                    row.encode(&mut bytes);
                    assert_eq!(RankDepth::decode(&bytes), row);
                    for position in [[0; 3], [1, u32::MAX, 0x8000_0000]] {
                        let row = Candidate {
                            position,
                            depth,
                            rank,
                            corner,
                        };
                        let mut bytes = [0u8; Candidate::SIZE];
                        row.encode(&mut bytes);
                        assert_eq!(Candidate::decode(&bytes), row);
                    }
                }
            }
        }
    }

    #[test]
    fn grids_list_every_box_holding_a_point() {
        // Boxes of very different sizes, a flat set, coincident boxes and a point box.
        let mut leaves = Vec::new();
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32
        };
        for i in 0..400 {
            let size = if i % 50 == 0 { 0.9 } else { 0.01 * next() };
            let min = [next(), next(), if i % 3 == 0 { 0. } else { next() }];
            leaves.push(Leaf {
                first: i,
                end: i + 1,
                depth: 0,
                bounds: Bounds {
                    min,
                    max: min.map(|v| v + size),
                },
            });
        }
        leaves.push(leaves[7]);
        let point = Bounds {
            min: [0.5; 3],
            max: [0.5; 3],
        };
        leaves.push(Leaf {
            bounds: point,
            ..leaves[0]
        });
        let grid = Grid::new(&leaves, &mut || Ok(())).unwrap();
        // The finest level has about four cells per leaf; coarser ones fewer.
        assert!(grid.levels[0].offsets.len() <= leaves.len() * 8 + 2);
        assert!(grid
            .levels
            .iter()
            .skip(1)
            .all(|l| l.offsets.len() <= grid.levels[0].offsets.len()));
        // The eight 0.9-wide boxes are too wide for the finest level but not for every level.
        assert!(
            grid.levels.len() > 1 && grid.huge.len() < 8,
            "{}",
            grid.huge.len()
        );
        for _ in 0..20_000 {
            let p = [next() * 1.2 - 0.1, next() * 1.2 - 0.1, next() * 1.2 - 0.1];
            let holders: Vec<u32> = (0..leaves.len() as u32)
                .filter(|&i| mask_bit(&leaves[i as usize].bounds, p).is_some())
                .collect();
            let near: std::collections::BTreeSet<u32> = grid.near(p).collect();
            assert!(holders.iter().all(|i| near.contains(i)), "{p:?}");
        }
        for leaf in &leaves {
            assert!(grid.near(leaf.bounds.min).count() > 0);
            assert!(grid.near(leaf.bounds.max).count() > 0);
        }
    }

    /// Every leaf publishing a canonical point is found by the holders, whichever other leaf
    /// asks. Corners sit on half lattice steps (ties), on mask-cell and box faces, at -0 and
    /// +0, far from the origin and in flat boxes, at every welded depth from their leaf's
    /// least to its own; in snapped and float profiles. The filter is also sensitive: points
    /// only the asking leaf holds are often refused.
    #[test]
    fn holders_find_every_leaf_holding_a_canonical_point() {
        use super::super::{config::Profile, leaves::geometry::LatticePlan};
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for profile in [Profile::Balanced, Profile::Quality] {
            for offset in [0f32, 3000.] {
                let source = Bounds {
                    min: [offset - 600.; 3],
                    max: [offset + 600.; 3],
                };
                let extents = [1200., 300., 75., 19., 5., 1.25];
                let lattice = LatticePlan::new(profile, source, &extents).unwrap();
                let steps = lattice.steps().to_vec();
                let mut boxes = Vec::new();
                let mut corners: Vec<Vec<([f32; 3], u8)>> = Vec::new();
                for leaf in 0..80 {
                    let own = 1 + (next() % 5) as u8;
                    let least = (next() % u64::from(own + 1)) as u8;
                    let s = steps[usize::from(least)];
                    // A box on the coarse lattice, some of it flat, near its neighbours.
                    let origin: [f32; 3] =
                        std::array::from_fn(|_| offset + (next() % 64) as f32 * s - 32. * s);
                    let size: [f32; 3] = std::array::from_fn(|a| {
                        if leaf % 7 == 0 && a == 1 {
                            0.
                        } else {
                            (1 + next() % 6) as f32 * s * 1.5
                        }
                    });
                    let mut list = Vec::new();
                    let depth = |next: &mut dyn FnMut() -> u64| {
                        least + (next() % u64::from(own - least + 1)) as u8
                    };
                    // Box corners, then points on mask-cell faces and half-step ties.
                    for corner in 0..8 {
                        let p = std::array::from_fn(|a| {
                            origin[a] + if corner >> a & 1 == 1 { size[a] } else { 0. }
                        });
                        list.push((p, if corner == 0 { least } else { depth(&mut next) }));
                    }
                    for _ in 0..24 {
                        let p = std::array::from_fn(|a| {
                            let j = (next() % 17) as f32;
                            match next() % 3 {
                                0 => origin[a] + size[a] * j / 16.,
                                1 => {
                                    let t = steps[(next() % steps.len() as u64) as usize];
                                    let k = ((size[a] / t).floor() as u64).max(1);
                                    (origin[a] + (next() % k) as f32 * t + t / 2.)
                                        .min(origin[a] + size[a])
                                }
                                _ => origin[a] + size[a] * (next() % 1000) as f32 / 999.,
                            }
                        });
                        list.push((p, depth(&mut next)));
                    }
                    if offset == 0. && leaf % 5 == 0 {
                        list.push(([-0., 0., -0.], depth(&mut next)));
                        list.push(([0., -0., 0.], depth(&mut next)));
                    }
                    let mut bounds = Bounds::EMPTY;
                    for (p, _) in &list {
                        bounds.include(*p);
                    }
                    boxes.push(bounds);
                    corners.push(list);
                }
                let masks = boxes
                    .iter()
                    .zip(&corners)
                    .map(|(bounds, list)| {
                        let mut mask = [0u64; MASK_WORDS];
                        for (p, _) in list {
                            let bit = mask_bit(bounds, *p).unwrap();
                            mask[bit / 64] |= 1 << (bit % 64);
                        }
                        mask
                    })
                    .collect();
                let least_depths = corners
                    .iter()
                    .map(|list| list.iter().map(|c| c.1).min().unwrap())
                    .collect();
                let index = LeafIndex {
                    masks,
                    least_depths,
                };
                let snapped = lattice.snapped().then_some(&steps[..]);
                let holders = Holders::new(&index, &boxes, snapped, &mut || Ok(())).unwrap();
                let held: Vec<Vec<[u32; 3]>> = corners
                    .iter()
                    .map(|list| {
                        list.iter()
                            .map(|&(q, ell)| {
                                let c = lattice.canonical(q, ell).unwrap();
                                // The canonical value moves at most half its step, exactly.
                                let half = if lattice.snapped() {
                                    f64::from(steps[usize::from(ell)]) / 2.
                                } else {
                                    0.
                                };
                                for a in 0..3 {
                                    assert!((f64::from(c[a]) - f64::from(q[a])).abs() <= half);
                                }
                                c.map(f32::to_bits)
                            })
                            .collect()
                    })
                    .collect();
                let (mut refused, mut lonely) = (0, 0);
                for (b, points) in held.iter().enumerate() {
                    for c in points {
                        let holders_of: Vec<usize> =
                            (0..held.len()).filter(|&l| held[l].contains(c)).collect();
                        for asking in [0, b, (b + 1) % held.len(), held.len() - 1] {
                            let other = holders.other(asking as u32, c.map(f32::from_bits));
                            if holders_of.iter().any(|&l| l != asking) {
                                assert!(other, "{profile:?} {offset}: leaf {b} holds {c:?}");
                            } else {
                                lonely += 1;
                                refused += usize::from(!other);
                            }
                        }
                    }
                }
                assert!(
                    refused * 4 > lonely,
                    "{profile:?} {offset}: {refused} of {lonely}"
                );
            }
        }
    }
}
