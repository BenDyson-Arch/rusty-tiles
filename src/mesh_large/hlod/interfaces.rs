//! Spillable source incidence: which decoded points and segments each leaf shares with
//! other leaves, and over which interval of DFS leaf indices.
//!
//! Vertices are welded by their leaf-canonical decoded position (`LatticePlan::canonical`):
//! the coordinates every leaf publishes, so two source vertices that decode to one lattice
//! point are one vertex, and a triangle whose corners decode onto fewer than three points
//! carries no surface (it is skipped, as the decoded census skips it).
//!
//! Leaves are contiguous DFS intervals of the frozen tree, so a tile whose leaves are
//! `[begin, end)` must keep, as complete decoded segments, exactly the shared points and
//! segments whose incident-leaf interval `[lo, hi]` leaves its own. Children's rows are
//! merged and filtered upward; nothing input-sized stays resident (two external sorts, one
//! leaf of deduplicated items at a time, one group of incident leaves at a time).
use crate::{
    mesh_large::{
        attributes::FullTriangle,
        config::{self, SortSpill},
        full::CornerDepth,
        leaves::geometry::LatticePlan,
        records::{NodeKind, NodeRecord},
    },
    spill::{ExternalSorter, Record, Scratch, Sequence, SortConfig, SortedStream},
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

/// One decoded point (`a == b`) or segment (`a < b`) incidence of a leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Item {
    a: Key,
    b: Key,
    leaf: u32,
}

impl Record for Item {
    const SIZE: usize = 28;
    fn encode(&self, out: &mut [u8]) {
        put(out, 0, self.a);
        put(out, 12, self.b);
        out[24..].copy_from_slice(&self.leaf.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            a: get(bytes, 0),
            b: get(bytes, 12),
            leaf: u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
        }
    }
}

/// A decoded point (`a == b`) or segment (`a < b`) shared by the DFS leaves `lo..=hi`,
/// recorded once for each incident `leaf`.
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
    /// Identity without the recording leaf: equal for every incident leaf.
    pub(crate) fn key(&self) -> (Key, Key, u32, u32) {
        (self.a, self.b, self.lo, self.hi)
    }
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
        let u = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        Self {
            leaf: u(0),
            a: get(bytes, 4),
            b: get(bytes, 16),
            lo: u(28),
            hi: u(32),
        }
    }
}

/// Worst-case spill of the incidence pass for `triangles` source triangles: every
/// triangle contributes at most three point and three segment items, and every item at
/// most one row. The row sort fills in bursts per group, so it is charged on top of the
/// whole item stream.
pub(crate) fn scratch_need(triangles: u64, sorter: SortConfig) -> Option<u64> {
    let items = triangles.checked_mul(6)?;
    let item: SortSpill = config::sort_spill::<Item>(items, sorter)?;
    let rows: SortSpill = config::sort_spill::<InterfaceRow>(items, sorter)?;
    let reduce = item
        .bytes
        .checked_add(item.stream_slack)?
        .checked_add(rows.push_peak)?;
    let walk = rows.finish_peak.checked_add(rows.stream_slack)?;
    Some(item.finish_peak.max(reduce).max(walk))
}

/// Resident bound of the pass: both sorters live while the item stream is reduced, plus
/// one leaf's deduplicated items.
pub(crate) fn memory_need(sorter: SortConfig, leaf_triangles: u64) -> u64 {
    sorter
        .memory_bound::<Item>()
        .saturating_add(sorter.memory_bound::<InterfaceRow>())
        .saturating_add(leaf_triangles.saturating_mul(6 * std::mem::size_of::<Item>() as u64))
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

/// Every leaf's shared-point and shared-segment rows, sorted by (leaf, a, b).
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
    let mut buffer: Vec<Item> = Vec::new();
    for node in nodes.reader()? {
        let node = node?;
        if node.kind != NodeKind::Leaf {
            continue;
        }
        if node.first != rank {
            return Err(state("leaf ranges are not contiguous in DFS order"));
        }
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
            for i in 0..3 {
                let (p, q) = (k[i], k[(i + 1) % 3]);
                buffer.push(Item { a: p, b: p, leaf });
                buffer.push(Item {
                    a: p.min(q),
                    b: p.max(q),
                    leaf,
                });
            }
        }
        buffer.sort_unstable();
        buffer.dedup();
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
    let mut rows = ExternalSorter::<InterfaceRow>::new(scratch, sorter)?;
    let mut group: Vec<u32> = Vec::new();
    let mut current: Option<(Key, Key)> = None;
    let flush = |key: (Key, Key),
                 group: &mut Vec<u32>,
                 rows: &mut ExternalSorter<'s, InterfaceRow>,
                 stats: &mut IncidenceStats|
     -> Result<(), JobError> {
        if group.len() > 1 {
            let (lo, hi) = (group[0], group[group.len() - 1]);
            if key.0 == key.1 {
                stats.shared_points += 1;
            } else {
                stats.shared_segments += 1;
                stats.multi_leaf_segments += u64::from(group.len() > 2);
            }
            for &leaf in group.iter() {
                rows.push(InterfaceRow {
                    leaf,
                    a: key.0,
                    b: key.1,
                    lo,
                    hi,
                })?;
                stats.rows += 1;
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
        if current != Some((item.a, item.b)) {
            if let Some(k) = current {
                flush(k, &mut group, &mut rows, &mut stats)?;
            }
            current = Some((item.a, item.b));
        }
        // Items are unique per leaf and sorted, so the group's leaves ascend strictly.
        group.push(item.leaf);
    }
    if let Some(k) = current {
        flush(k, &mut group, &mut rows, &mut stats)?;
    }
    Ok((rows.finish()?, stats))
}
