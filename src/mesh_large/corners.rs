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
/// Leaves are read in chunks of at most about this many triangles, one chunk per task (and
/// at least eight chunks per worker when the triangles allow).
const CHUNK_TRIANGLES: u64 = 1 << 16;
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
        let extent = f64::from(max) - f64::from(min);
        if extent > 0. {
            let x = ((f64::from(p[axis]) - f64::from(min)) / extent * SIDE as f64).floor();
            cell[axis] = (x.max(0.) as usize).min(SIDE - 1);
        }
    }
    Some(cell[0] + SIDE * (cell[1] + SIDE * cell[2]))
}

/// A uniform grid over the leaves' boxes, listing for each cell the leaves whose box meets
/// it; a box spanning more than [`LARGE_SPAN`] cells is listed once, in `large`.
struct Grid {
    origin: [f64; 3],
    side: f64,
    dims: [usize; 3],
    offsets: Vec<usize>,
    ids: Vec<u32>,
    large: Vec<u32>,
}

impl Grid {
    fn new(leaves: &[Leaf]) -> Self {
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
        let dims = dims_at(side);
        let mut grid = Self {
            origin,
            side,
            dims,
            offsets: vec![0; cells(dims) + 1],
            ids: Vec::new(),
            large: Vec::new(),
        };
        let mut spans = Vec::with_capacity(leaves.len());
        for (i, leaf) in leaves.iter().enumerate() {
            let (lo, hi) = (grid.cell(leaf.bounds.min), grid.cell(leaf.bounds.max));
            let span: usize = (0..3).map(|a| hi[a] - lo[a] + 1).product();
            if span > LARGE_SPAN {
                grid.large.push(i as u32);
                spans.push(None);
            } else {
                spans.push(Some((lo, hi)));
            }
        }
        let each = |span: ([usize; 3], [usize; 3]), f: &mut dyn FnMut(usize)| {
            let (lo, hi) = span;
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        f(x + dims[0] * (y + dims[1] * z));
                    }
                }
            }
        };
        for span in spans.iter().flatten() {
            each(*span, &mut |c| grid.offsets[c + 1] += 1);
        }
        for c in 0..cells(dims) {
            grid.offsets[c + 1] += grid.offsets[c];
        }
        let mut fill = grid.offsets.clone();
        grid.ids = vec![0; grid.offsets[cells(dims)]];
        for (i, span) in spans.iter().enumerate() {
            if let Some(span) = span {
                each(*span, &mut |c| {
                    grid.ids[fill[c]] = i as u32;
                    fill[c] += 1;
                });
            }
        }
        grid
    }

    /// The grid cell of `p`, clamped to the grid. Monotone in each coordinate, so a point of
    /// a box lies in a cell of the box's span.
    fn cell(&self, p: [f32; 3]) -> [usize; 3] {
        std::array::from_fn(|a| {
            let x = ((f64::from(p[a]) - self.origin[a]) / self.side).floor();
            (x.max(0.) as usize).min(self.dims[a] - 1)
        })
    }

    /// The leaves whose box may hold `p`.
    fn near(&self, p: [f32; 3]) -> impl Iterator<Item = u32> + '_ {
        let [x, y, z] = self.cell(p);
        let c = x + self.dims[0] * (y + self.dims[1] * z);
        self.ids[self.offsets[c]..self.offsets[c + 1]]
            .iter()
            .chain(&self.large)
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
) -> Result<Sequence<CornerDepth>, JobError> {
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
    let wave = jobs.max(1) * 2;
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
        grid: Grid::new(&leaves),
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
    for leaf in &leaves {
        for rank in leaf.first..leaf.end {
            let mut depths = [leaf.depth; 3];
            while let Some(patch) = patches.next_if(|p| p.as_ref().is_ok_and(|p| p.rank == rank)) {
                let patch = patch?;
                depths[patch.corner as usize] = patch.depth;
            }
            writer.push(&CornerDepth { depths })?;
            if rank % 4096 == 0 {
                progress.advance(2 * count + rank)?;
            }
        }
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
    Ok(written)
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
        let grid = Grid::new(&leaves);
        assert!(grid.offsets.len() <= leaves.len() * 8 + 2);
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
}
