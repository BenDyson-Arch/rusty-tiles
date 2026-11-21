//! D11 shared level contracts. One disk graph resolves all owners before parent jobs.
//! Source arcs are exact incidence paths, and every level shortcuts those source arcs
//! directly. Live junctions and terminal-leaf contacts are pinned. The same stored segment
//! is handed to every owner, so no tile-local choice can disagree with its neighbor.
mod curve;
mod disk;
mod graph;
pub(super) fn memory_need(sorter: SortConfig) -> u64 {
    sorter.memory_bound::<InterfaceRow>()
        + sorter.memory_bound::<disk::Words<7>>()
        + 8 * sorter.buffer_bytes as u64
        + (1 << 16)
        + 14 * disk::CACHE_BYTES
        + 8 * graph::FILTER_WORDS as u64
        // Chain reuse compares two live contracts before the geometry stage.
        + 2 * contract_workspace_bytes(6 * super::TRIANGLE_CAP as u64)
}

/// One list head of [`Borders::listed_items`]: next unread reference, node, items left.
type ListHead = (u64, u64, u64);

/// Resident bound of [`Borders::listed_items`] over a tree of `nodes` nodes: its sort and
/// stream, and one list head per node (reserved exactly, held until the lists drain).
pub(super) fn listed_items_memory(sorter: SortConfig, nodes: u64) -> u64 {
    sorter
        .memory_bound::<disk::Words<4>>()
        .saturating_add(nodes.saturating_mul(std::mem::size_of::<ListHead>() as u64))
}

/// Scratch bound of [`Borders::listed_items`] for `items` linked items.
pub(super) fn listed_items_scratch(items: u64, sorter: SortConfig) -> Option<u64> {
    let sort = config::sort_spill::<disk::Words<4>>(items, sorter)?;
    sort.finish_peak.checked_add(sort.stream_slack)
}

pub(super) const WALK_CACHE_BYTES: u64 = 8 * disk::CACHE_BYTES;

use self::{
    curve::{state, Ladder},
    disk::{Disk, NONE},
    graph::Source,
};
use super::{
    interfaces::{InterfaceRow, Key},
    scratch::{ComponentLease, ScratchLedger},
    store::TileKey,
};
use crate::{
    mesh_large::{
        config,
        phases::{Step, StepProgress},
        records::NodeRecord,
    },
    spill::{ExternalSorter, Scratch, Sequence, SortConfig, SortedStream},
    JobError,
};
use std::collections::BTreeMap;

fn resource(message: &str) -> JobError {
    JobError::new(crate::JobErrorKind::ResourceLimit, message)
}

/// Bytes per record of each store: `Disk<N>` words (8 bytes each) and the leaf rows.
const VERTEX: u64 = 64;
/// An edge, its finer relief (2 words) and its level relief (3 words).
const EDGE: u64 = 56 + 16 + 24;
const OWNER: u64 = 8;
const NODE: u64 = 64;
const LEAF: u64 = 264;
const LEAF_ROW: u64 = 36;
const MAP: u64 = 24;
const SEGMENT: u64 = 96;
const REF: u64 = 16;

/// Counts the borders' scratch is declared from: the incidence pass's rows, shared points and
/// shared segments, and the frozen tree's nodes, leaves and deepest depth.
#[derive(Clone, Copy, Debug)]
pub(super) struct ScratchCounts {
    pub rows: u64,
    pub points: u64,
    pub segments: u64,
    pub nodes: u64,
    pub leaves: u64,
    pub height: u32,
}

/// The graph's persistent stores and the relief state ([`Borders::bytes`] before any level).
fn graph_bytes(rows: u64, vertices: u64, edges: u64, nodes: u64, leaves: u64) -> Option<u64> {
    rows.checked_mul(LEAF_ROW + OWNER)?
        .checked_add(vertices.checked_mul(VERTEX)?)?
        .checked_add(edges.checked_mul(EDGE)?)?
        .checked_add(nodes.checked_mul(NODE)?)?
        .checked_add(leaves.checked_mul(LEAF)?)
}

/// Before the graph exists: its stores from the incidence counts, the row sort through its
/// stream and the build's transients (edge keys and adjacency).
fn build_need(counts: ScratchCounts, sorter: SortConfig) -> Option<u64> {
    let ScratchCounts {
        rows,
        points: vertices,
        segments: edges,
        nodes,
        leaves,
        ..
    } = counts;
    let sort = config::sort_spill::<disk::Words<7>>(rows, sorter)?;
    graph_bytes(rows, vertices, edges, nodes, leaves)?
        .checked_add(sort.finish_peak)?
        .checked_add(sort.stream_slack)?
        .checked_add(edges.checked_mul(72 + 8 + 96)?)?
        .checked_add(nodes.checked_mul(8)?)?
        .checked_add(vertices.checked_mul(24)?)?
        .checked_add(1 << 20)
}

/// The level stage beside the graph: every depth's vertex maps, the segments and references
/// all levels may emit ([`graph::LevelBounds`]), and one level's temporaries, the largest
/// over levels (live edges and vertices, point contacts and the ordered arc, at most two
/// entries per external edge and two per chain); they are released after each level.
fn levels_bytes(
    vertices: u64,
    edges: u64,
    nodes: u64,
    depths: u64,
    bounds: graph::LevelBounds,
) -> Option<u64> {
    let temporaries = edges
        .checked_mul(8)?
        .checked_add(vertices.checked_mul(24)?)?
        .checked_add(nodes.checked_mul(8)?)?
        .checked_add(bounds.active.checked_mul(4 * 24)?)?;
    vertices
        .checked_mul(depths)?
        .checked_mul(MAP)?
        .checked_add(bounds.segments.checked_mul(SEGMENT)?)?
        .checked_add(bounds.refs.checked_mul(REF)?)?
        .checked_add(temporaries)
}

/// Worst case of the borders' two admissions ([`Borders::build`]), for plans made before the
/// incidence pass and the graph exist (memory-backed scratch, `mesh-inspect`): the build, or
/// the graph with every edge and vertex external at every depth and every row referenced
/// at every depth.
pub(super) fn scratch_need(counts: ScratchCounts, sorter: SortConfig) -> Option<u64> {
    let depths = u64::from(counts.height) + 1;
    let worst = graph::LevelBounds {
        segments: counts
            .points
            .checked_add(counts.segments)?
            .checked_mul(depths)?,
        refs: counts.rows.checked_mul(depths)?,
        active: counts.segments,
    };
    let levels = graph_bytes(
        counts.rows,
        counts.points,
        counts.segments,
        counts.nodes,
        counts.leaves,
    )?
    .checked_add(levels_bytes(
        counts.points,
        counts.segments,
        counts.nodes,
        depths,
        worst,
    )?)?;
    Some(build_need(counts, sorter)?.max(levels))
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DepthReceipt {
    pub depth: u32,
    pub scale: f64,
    pub tiles: u64,
    pub segments: u64,
    pub terminal_locks: u64,
    pub max_epsilon: f64,
    pub topology: super::geometry::TopologyReceipt,
    pub attempted_topology: super::geometry::TopologyReceipt,
    pub omission_sources: u64,
    pub omitted_cells: u64,
    pub frontier_tiles: u64,
    /// Border relief (A1): cells whose contract exceeded the trigger at this depth, shared
    /// segments certified at a ladder step above zero, the largest step, and the largest
    /// published relief floor.
    #[serde(skip_serializing_if = "is_zero")]
    pub relief_cells: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub relieved_segments: u64,
    #[serde(skip_serializing_if = "is_zero_step")]
    pub max_relief_step: u32,
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub max_relief_floor: f64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}
fn is_zero_step(n: &u32) -> bool {
    *n == 0
}
fn is_zero_f64(n: &f64) -> bool {
    *n == 0.
}

/// Feasibility border relief (A1). A cell whose depth-d contract holds more than 85%
/// of the hard cap in items requests the least ladder step that brings it to 75%,
/// or step 8 if none does. A shared arc takes the largest
/// request of its owners, and never less than its finer level's tolerance. Relief
/// certificates stay exact and shared.
pub(super) const RELIEF_STEPS: u32 = 8;

/// One level's relief: cells that triggered, segments above step zero, largest step.
#[derive(Clone, Copy, Default)]
pub(super) struct ReliefLevel {
    pub cells: u64,
    pub segments: u64,
    pub max_step: u32,
}

type Counts = [u64; RELIEF_STEPS as usize + 1];

/// What one level walk does with each source chain.
enum Mode<'a> {
    /// Emit chords. A chain's step is its owners' largest request (`requests`, by node),
    /// and at least its finer level's.
    Emit(&'a BTreeMap<u64, u32>),
    /// Count, for every chain with a triggered owner, its chords at each ladder step.
    Count(&'a mut BTreeMap<u64, Counts>),
}

pub(super) struct Borders {
    source: Source,
    /// Per (logical depth, source vertex): retained endpoint pair and source parameter.
    maps: Disk<3>,
    /// endpoints, chain source edge, origin, first/end source ordinal, epsilon bits, depth,
    /// the complete source-owner run whose depth-coarsened owners consume this segment,
    /// the largest finer certified displacement on its subarc (bits) and its ladder step.
    segments: Disk<12>,
    /// Per node: linked references to common segments (head/count stored in Source::nodes).
    refs: Disk<2>,
    ladder: Ladder,
    /// Hard payload cap the relief trigger is a share of.
    cap: u64,
    /// Per source edge: largest certified displacement over finer levels, and the ladder
    /// step of the finest level so far (A1 floor and monotone tolerance).
    edge_finer: Disk<2>,
    /// Per source edge, this level: certified displacement, step and depth + 1.
    edge_level: Disk<3>,
    /// Test seam: the emission bounds the borders were admitted with.
    #[cfg(test)]
    bounds: graph::LevelBounds,
    relief: Vec<ReliefLevel>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Certificate {
    pub endpoints: [Key; 2],
    pub chain: [Key; 2],
    pub origin: Key,
    pub first: u64,
    pub end: u64,
    pub epsilon: f64,
    pub terminal_lock: bool,
    /// Border relief ladder step (A1); 0 is D11's E_d/16.
    #[serde(skip_serializing_if = "is_zero_step")]
    pub step: u32,
}

/// A contract point and the least and greatest DFS leaf holding it anywhere in the source:
/// its whole owner run, the same for every cell and depth (unlike an interface row's
/// interval, which spans only the cell's own arcs at the point). The seam owner
/// (`geometry::folds::Policy`) is derived from it, as the validators derive it from the
/// leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Holders {
    pub key: Key,
    pub lo: u32,
    pub hi: u32,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Contract {
    pub depth: u32,
    pub interface: Vec<InterfaceRow>,
    pub certificates: Vec<Certificate>,
    /// Exact vertex contacts with a terminal leaf, including contacts with no shared edge.
    pub terminal_vertices: Vec<Key>,
    /// Every interface point's global holding leaves, sorted by key.
    pub holders: Vec<Holders>,
    pub scale: f64,
    /// Border relief floor (A1): 8 x the largest sum of a certificate's displacement and
    /// the largest finer certified displacement on its subarc, over every member of its
    /// single-child chain. Published G is at least this; it is 0 without relief.
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub relief_floor: f64,
}

/// Peak storage of contract construction for at most `limit` linked items. Each
/// item emits at most one certificate, two terminal keys, two point holders and three
/// interface rows.
/// `merge_rows` briefly holds both row vectors. Its retired row copy also covers
/// stable certificate sorting's scratch (one certificate is smaller than three
/// rows). Exact reservations below prevent growth transients. This allowance stays live
/// through geometry, S2 and audit publication, independently of geometry workspace.
pub(super) fn contract_workspace_bytes(limit: u64) -> u64 {
    limit.saturating_mul(
        (std::mem::size_of::<Certificate>()
            + 2 * std::mem::size_of::<Key>()
            + 2 * std::mem::size_of::<Holders>()
            + 6 * std::mem::size_of::<InterfaceRow>()) as u64,
    )
}

/// Test fixtures: point holders from vertex rows whose intervals stand for the global holders.
#[cfg(test)]
pub(crate) fn holders_from_rows(interface: &[InterfaceRow]) -> Vec<Holders> {
    let mut holders: Vec<_> = interface
        .iter()
        .filter(|r| r.is_vertex())
        .map(|r| Holders {
            key: r.a,
            lo: r.lo,
            hi: r.hi,
        })
        .collect();
    holders.sort_unstable_by_key(|h| h.key);
    holders.dedup_by(|b, a| {
        let same = a.key == b.key;
        if same {
            (a.lo, a.hi) = (a.lo.min(b.lo), a.hi.max(b.hi));
        }
        same
    });
    holders
}

/// A contract as `mesh-inspect`'s forecast counts it ([`Borders::contract_counts`]).
pub(super) struct ContractCounts {
    /// Its distinct points, as sorted source vertex ids.
    pub points: Vec<u64>,
    /// Its distinct segments (the interface's edge rows).
    pub segments: u64,
    /// [`Contract::terminal_locks`].
    pub terminal_locks: u64,
}
impl ContractCounts {
    /// [`Contract::minimum_triangles`].
    pub(super) fn minimum_triangles(&self) -> u64 {
        (self.points.len() as u64).max(self.segments).div_ceil(3)
    }
}

/// [`Borders::listed_items`]: listed nodes' items, taken in ascending node order.
pub(super) struct ListedItems<'s, 'p> {
    stream: std::iter::Peekable<SortedStream<'s, disk::Words<4>>>,
    /// Cancellation, checked as items are taken.
    progress: &'s StepProgress<'p>,
}
impl ListedItems<'_, '_> {
    /// The linked items of `node`'s own contract (empty for an empty list). Nodes must be
    /// taken in ascending order; the items of nodes skipped over are dropped.
    pub(super) fn take(&mut self, node: u64) -> Result<Vec<[u64; 3]>, JobError> {
        let mut items: Vec<[u64; 3]> = Vec::new();
        let mut read = 0u64;
        // Errors are taken too, and returned.
        while let Some(item) = self
            .stream
            .next_if(|r| r.as_ref().map_or(true, |w| w.0[0] <= node))
        {
            if read.is_multiple_of(4096) {
                self.progress.check()?;
            }
            read += 1;
            let [at, a, b, edge] = item?.0;
            if at == node {
                items.push([a, b, edge]);
            }
        }
        Ok(items)
    }
}

impl Contract {
    /// Whether any certificate is relieved (A1).
    pub(super) fn relieved(&self) -> bool {
        self.certificates.iter().any(|c| c.step > 0)
    }
    pub(super) fn terminal_locks(&self) -> u64 {
        self.terminal_vertices.len() as u64
            + self.certificates.iter().filter(|c| c.terminal_lock).count() as u64
    }
    /// Every required point and edge must occur in a triangle. This necessary lower bound
    /// is independent of remeshing; exceeding it makes the capped frontier unavoidable.
    pub(super) fn minimum_triangles(&self) -> usize {
        let points = self.interface.iter().filter(|r| r.is_vertex()).count();
        points.max(self.interface.len() - points).div_ceil(3)
    }
}

impl Borders {
    pub(super) const COMPONENT: &'static str = "s1b-shared-borders";
    #[expect(
        clippy::too_many_arguments,
        reason = "predates the lint policy; scripts/lint_debt.py keeps the count from growing"
    )]
    /// The borders of the frozen tree, and their scratch admission, which the walk holds
    /// until it drops them.
    ///
    /// The borders admit their scratch twice, under one name. Before the graph exists, the
    /// build is admitted from the incidence counts ([`build_need`]). Once it is built, that
    /// admission is released and the borders are admitted once more, with free space
    /// preflighted, for everything they will hold: the graph as built and every level's
    /// emission, bounded from the graph ([`graph::Source::level_bounds`]). A shortfall is
    /// refused there, before any level runs; nothing is admitted later.
    pub(super) fn build<'l, 'a>(
        scratch: &Scratch,
        sorter: SortConfig,
        nodes: &Sequence<NodeRecord>,
        rows: impl Iterator<Item = Result<InterfaceRow, crate::spill::SpillError>>,
        open_ends: impl Iterator<Item = Result<super::interfaces::KeyRecord, crate::spill::SpillError>>,
        faces: impl Iterator<Item = Result<[Key; 3], JobError>>,
        triangles: u64,
        (extent, cap, counts): (f64, usize, ScratchCounts),
        ledger: &'l ScratchLedger<'a>,
        progress: &StepProgress<'_>,
    ) -> Result<(Self, ComponentLease<'l, 'a>), JobError> {
        let check = || progress.check();
        let check = &check;
        let overflow = || resource("shared-border scratch bound overflows");
        let building = ledger.admit(
            Self::COMPONENT,
            build_need(counts, sorter).ok_or_else(overflow)?,
        )?;
        progress.begin(Step::BordersRows, counts.rows)?;
        let mut source = Source::build(
            scratch,
            sorter,
            nodes,
            rows,
            open_ends,
            &|| building.charge(0),
            progress,
        )?;
        source.retain_faces(faces, triangles, progress)?;
        let bounds = source.level_bounds(check)?;
        let need = graph_bytes(
            source.owners.len.max(source.leaf_rows.len()),
            source.vertices.len,
            source.edges.len,
            source.nodes.len,
            source.leaves.len,
        )
        .and_then(|graph| {
            graph.checked_add(levels_bytes(
                source.vertices.len,
                source.edges.len,
                source.nodes.len,
                u64::from(source.depth) + 1,
                bounds,
            )?)
        })
        .ok_or_else(overflow)?;
        drop(building);
        let lease = ledger.admit(Self::COMPONENT, need)?;
        let mut borders = Self {
            maps: Disk::new(scratch, "border-maps")?,
            segments: Disk::new(scratch, "border-segments")?,
            refs: Disk::new(scratch, "border-contracts")?,
            ladder: Ladder::new(extent)?,
            cap: cap as u64,
            edge_finer: Disk::new(scratch, "border-relief-finer")?,
            edge_level: Disk::new(scratch, "border-relief-level")?,
            relief: vec![ReliefLevel::default(); source.depth as usize + 1],
            #[cfg(test)]
            bounds,
            source,
        };
        if borders.source.edges.len > 0 {
            let last = borders.source.edges.len - 1;
            borders.edge_finer.set(last, [0, 0])?;
            borders.edge_level.set(last, [0, 0, 0])?;
        }
        // Source arcs are traversed one level at a time; their temporary ordered points
        // are discarded after certificates and source-vertex maps have been materialized.
        let levels = u64::from(borders.source.depth) + 1;
        progress.begin(Step::BordersLevels, levels)?;
        for (done, depth) in (0..=borders.source.depth).rev().enumerate() {
            progress.advance(done as u64)?;
            borders.level(scratch, depth, check)?;
            lease.charge(borders.bytes())?;
        }
        progress.end()?;
        #[cfg(test)]
        borders.check_retention()?;
        let mut ancestors = [(0u64, 0u64, 0u32); 32];
        progress.begin(Step::BordersPublish, borders.source.nodes.len)?;
        for id in 0..borders.source.nodes.len {
            if id.is_multiple_of(4096) {
                progress.advance(id)?;
            }
            let mut node = borders.source.nodes.get(id)?;
            let depth = node[2] as u32;
            let mut published = depth;
            if node[5] == 0 && depth > 0 {
                let parent = ancestors[depth as usize - 1];
                if (parent.0, parent.1) == (node[0], node[1])
                    && borders.contracts_equal(node[0], node[1], depth - 1, super::TRIANGLE_CAP)?
                {
                    published = parent.2;
                }
            }
            node[5] |= u64::from(published) << 1;
            borders.source.nodes.set(id, node)?;
            ancestors[depth as usize] = (node[0], node[1], published);
        }
        borders.source.flush()?;
        borders.maps.flush()?;
        borders.segments.flush()?;
        borders.refs.flush()?;
        lease.charge(borders.bytes())?;
        progress.end()?;
        Ok((borders, lease))
    }
    /// Test builds: an external vertex retained at a depth is retained one level finer
    /// (nested endpoints), and face retention keeps its vertex at and below its depth.
    #[cfg(test)]
    fn check_retention(&self) -> Result<(), JobError> {
        let n = self.source.vertices.len;
        for v in 0..n {
            let r = self.source.vertices.get(v)?;
            for depth in 0..=self.source.depth {
                let m = self.maps.get(u64::from(depth) * n + v)?;
                let retained = m[0] == m[1];
                if Source::face_retained(&r, depth) && !retained {
                    return Err(state("a face-retained vertex was contracted"));
                }
                if depth < self.source.depth
                    && retained
                    && self.source.external(r[3], r[4], depth)?
                {
                    let finer = self.maps.get(u64::from(depth + 1) * n + v)?;
                    if finer[0] != finer[1] {
                        return Err(state("retained border vertices are not nested"));
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn audit_bound(&self, source_triangle_depths: u64) -> Option<u64> {
        super::audit::bound(self.refs.len, self.source.nodes.len, source_triangle_depths)
    }
    fn bytes(&self) -> u64 {
        self.source.vertices.len * VERTEX
            + self.source.edges.len * EDGE
            + self.source.owners.len * OWNER
            + self.source.nodes.len * NODE
            + self.source.leaves.len * LEAF
            + self.source.leaf_rows.len() * LEAF_ROW
            + self.maps.len * MAP
            + self.segments.len * SEGMENT
            + self.refs.len * REF
    }
    pub(super) fn leaf_rows(&self) -> &Sequence<InterfaceRow> {
        &self.source.leaf_rows
    }
    /// A chain cell may reuse its child's payload only when the complete decoded border
    /// contract agrees. Comparison is bounded by the maximum cap-sized interface.
    fn contracts_equal(
        &self,
        first: u64,
        end: u64,
        depth: u32,
        cap: usize,
    ) -> Result<bool, JobError> {
        let key = |depth| TileKey {
            first,
            end,
            depth,
            leaf_begin: 0,
            leaf_end: 0,
        };
        let Some(a) = self.own_contract(key(depth), 6 * cap)? else {
            return Ok(false);
        };
        let Some(b) = self.own_contract(key(depth + 1), 6 * cap)? else {
            return Ok(false);
        };
        Ok(a.interface
            .iter()
            .map(InterfaceRow::item)
            .eq(b.interface.iter().map(InterfaceRow::item)))
    }
    pub(super) fn publication_depth(&self, key: TileKey) -> Result<u32, JobError> {
        Ok((self.source.nodes.get(self.source.node(key)?)?[5] >> 1) as u32)
    }
    pub(super) fn can_collapse(&self, first: u64, end: u64, depth: u32) -> Result<bool, JobError> {
        Ok(self.publication_depth(TileKey {
            first,
            end,
            depth: depth + 1,
            leaf_begin: 0,
            leaf_end: 0,
        })? <= depth)
    }
    pub(super) fn scale(&self, depth: u32) -> f64 {
        self.ladder.error(depth)
    }

    /// Border relief at this depth (A1).
    pub(super) fn relief(&self, depth: u32) -> ReliefLevel {
        self.relief.get(depth as usize).copied().unwrap_or_default()
    }

    /// One level: emitted as D11 with each chain at its finer level's minimum step. Only
    /// when a cell's contract then exceeds the trigger are its chains' chords counted at
    /// every ladder step, the level rewound and emitted again with the requested steps.
    /// Without a trigger (or finer relief) this is D11's level, byte for byte.
    fn level(
        &mut self,
        scratch: &Scratch,
        depth: u32,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<(), JobError> {
        let mark = (self.segments.len, self.refs.len);
        self.walk(scratch, depth, &mut Mode::Emit(&BTreeMap::new()), check)?;
        let mut counts = BTreeMap::new();
        let mut items = BTreeMap::new();
        for id in 0..self.source.nodes.len {
            let n = self.source.nodes.get(id)?;
            // Only coarse cells build from a contract; leaves are published exactly.
            // Widen before scaling, including for the no-relief cap sentinel.
            if n[2] == u64::from(depth)
                && n[5] & 1 == 0
                && u128::from(n[7]) * 1000 > 850 * u128::from(self.cap)
            {
                counts.insert(id, [0; RELIEF_STEPS as usize + 1]);
                items.insert(id, n[7]);
            }
        }
        if !counts.is_empty() {
            check()?;
            self.walk(scratch, depth, &mut Mode::Count(&mut counts), check)?;
            let requests: BTreeMap<u64, u32> = counts
                .iter()
                .map(|(&id, c)| {
                    let at = |j: usize| (items[&id] + c[j]).saturating_sub(c[0]);
                    let step = (1..=RELIEF_STEPS)
                        .find(|&j| u128::from(at(j as usize)) * 1000 <= 750 * u128::from(self.cap))
                        .unwrap_or(RELIEF_STEPS);
                    (id, step)
                })
                .collect();
            self.rewind(mark, depth)?;
            self.walk(scratch, depth, &mut Mode::Emit(&requests), check)?;
            self.relief[depth as usize].cells = requests.len() as u64;
        }
        // The finer state the next (coarser) level reads: this level's steps, and the
        // largest certified displacement so far on each external edge.
        for e in 0..self.source.edges.len {
            if e.is_multiple_of(4096) {
                check()?;
            }
            let r = self.edge_level.get(e)?;
            if r[2] == u64::from(depth) + 1 {
                let f = self.edge_finer.get(e)?;
                let bound = f64::from_bits(f[0]).max(f64::from_bits(r[0]));
                self.edge_finer.set(e, [bound.to_bits(), r[1]])?;
            }
        }
        Ok(())
    }

    /// Drop this level's emission: segments and references from `mark` on. Every owner's
    /// list is unlinked back to its first reference before `mark`: a leaf shallower than
    /// this depth is its own owner here, so its list also holds deeper levels' items.
    fn rewind(&mut self, mark: (u64, u64), depth: u32) -> Result<(), JobError> {
        for id in 0..self.source.nodes.len {
            let mut n = self.source.nodes.get(id)?;
            if n[6] == NONE || n[6] < mark.1 {
                continue;
            }
            while n[6] != NONE && n[6] >= mark.1 {
                n[6] = self.refs.get(n[6])?[0];
                n[7] -= 1;
            }
            self.source.nodes.set(id, n)?;
        }
        self.segments.truncate(mark.0);
        self.refs.truncate(mark.1);
        self.relief[depth as usize] = ReliefLevel::default();
        Ok(())
    }

    fn walk(
        &mut self,
        scratch: &Scratch,
        depth: u32,
        mode: &mut Mode<'_>,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<(), JobError> {
        let emits = matches!(mode, Mode::Emit(_));
        let mut active = Disk::<1>::new(scratch, "border-live-edges")?;
        for i in 0..self.source.edges.len {
            let e = self.source.edges.get(i)?;
            active.push([u64::from(self.source.external(e[2], e[3], depth)?)])?;
        }
        // State 0: internal; 1: live junction/point; 2: removable degree-two source vertex.
        let mut live = Disk::<3>::new(scratch, "border-live-vertices")?;
        // Per node: 1 + the last vertex one of whose live edges it holds (sparse, zero).
        let mut reached = Disk::<1>::new(scratch, "border-point-contacts")?;
        if self.source.nodes.len > 0 {
            reached.set(self.source.nodes.len - 1, [0])?;
        }
        for v in 0..self.source.vertices.len {
            if v.is_multiple_of(4096) {
                check()?;
            }
            self.maps
                .set(u64::from(depth) * self.source.vertices.len + v, [v, v, 0])?;
            let vertex = self.source.vertices.get(v)?;
            let external = self.source.external(vertex[3], vertex[4], depth)?;
            let (mut edge, mut degree, mut two) = (vertex[5], 0, [NONE; 2]);
            let mut terminal = false;
            while edge != NONE {
                let e = self.source.edges.get(edge)?;
                if active.get(edge)?[0] != 0 {
                    if degree < 2 {
                        two[degree] = edge;
                    }
                    degree += 1;
                    terminal |= u64::from(depth) >= e[6];
                }
                edge = if e[0] == v { e[4] } else { e[5] };
            }
            let mut kind = u64::from(external);
            if degree == 2
                && !terminal
                && vertex[7] >> 32 & 1 == 0
                && !Source::face_retained(&vertex, depth)
            {
                let a = self.source.edges.get(two[0])?;
                let b = self.source.edges.get(two[1])?;
                if self.source.same_owners([a[2], a[3]], [b[2], b[3]], depth)?
                    && self
                        .source
                        .same_owners([a[2], a[3]], [vertex[3], vertex[4]], depth)?
                {
                    kind = 2;
                }
            }
            live.push([two[0], two[1], kind])?;
            if emits && external && degree > 0 && kind == 1 {
                self.point_contacts(v, &vertex, depth, &active, &mut reached)?;
            }
            if emits && external && degree == 0 {
                self.emit_segment([
                    v,
                    v,
                    NONE,
                    v,
                    0,
                    0,
                    0,
                    u64::from(depth),
                    vertex[3],
                    vertex[4],
                    0,
                    0,
                ])?;
            }
        }
        let mut arc = Disk::<3>::new(scratch, "border-ordered-arc")?;
        // Junction-started paths first. Adjacency has one link per incidence, so even a
        // high-valence vertex visits each incident edge once rather than pairing owners.
        for v in 0..self.source.vertices.len {
            if live.get(v)?[2] != 1 {
                continue;
            }
            let mut edge = self.source.vertices.get(v)?[5];
            while edge != NONE {
                let e = self.source.edges.get(edge)?;
                if active.get(edge)?[0] == 1 {
                    self.chain(v, edge, depth, &live, &mut active, &mut arc, mode, check)?;
                }
                edge = if e[0] == v { e[4] } else { e[5] };
            }
        }
        // Remaining edges are cycles. Source edge order chooses the least edge and its
        // least endpoint as the canonical origin/direction, independent of sort buffers.
        for edge in 0..self.source.edges.len {
            if active.get(edge)?[0] == 1 {
                let e = self.source.edges.get(edge)?;
                self.chain(e[0], edge, depth, &live, &mut active, &mut arc, mode, check)?;
            }
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "predates the lint policy; scripts/lint_debt.py keeps the count from growing"
    )]
    fn chain(
        &mut self,
        start: u64,
        first_edge: u64,
        depth: u32,
        live: &Disk<3>,
        active: &mut Disk<1>,
        arc: &mut Disk<3>,
        mode: &mut Mode<'_>,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<(), JobError> {
        let mut begin = arc.len;
        let retained = |v| -> Result<u64, JobError> {
            if depth == self.source.depth {
                return Ok(1);
            }
            let m = self
                .maps
                .get(u64::from(depth + 1) * self.source.vertices.len + v)?;
            Ok(u64::from(m[0] == m[1]))
        };
        let (mut at, mut edge) = (start, first_edge);
        loop {
            if (arc.len - begin).is_multiple_of(4096) {
                check()?;
            }
            arc.push([at, edge, retained(at)?])?;
            active.set(edge, [2])?;
            let e = self.source.edges.get(edge)?;
            at = if e[0] == at { e[1] } else { e[0] };
            let next = live.get(at)?;
            if at == start || next[2] != 2 {
                arc.push([at, NONE, retained(at)?])?;
                break;
            }
            edge = if next[0] == edge { next[1] } else { next[0] };
            if active.get(edge)?[0] != 1 {
                return Err(state("source arc revisits an edge before its endpoint"));
            }
        }
        let mut end = arc.len - 1;
        let mut first_edge = first_edge;
        let mut start = start;
        let e = self.source.edges.get(first_edge)?;
        let mut pieces = vec![begin, end];
        if at == start {
            let count = end - begin;
            let mut kept = 0;
            let mut origin = NONE;
            for i in begin..end {
                if arc.get(i)?[2] != 0 {
                    kept += 1;
                    if origin == NONE {
                        origin = i;
                    }
                }
            }
            if kept < 3 {
                return Err(state(
                    "a shared cycle has fewer than three retained vertices",
                ));
            }
            let rotated = arc.len;
            for i in 0..=count {
                let mut p = arc.get(begin + (origin - begin + i) % count)?;
                if i == count {
                    p[1] = NONE;
                }
                arc.push(p)?;
            }
            begin = rotated;
            end = begin + count;
            first_edge = arc.get(begin)?[1];
            start = arc.get(begin)?[0];
            pieces = vec![begin];
            let mut ordinal = 0;
            for i in begin..end {
                if arc.get(i)?[2] != 0 {
                    if ordinal == kept / 3 || ordinal == 2 * kept / 3 {
                        pieces.push(i);
                    }
                    ordinal += 1;
                }
            }
            pieces.push(end);
        } else if end > begin + 1 {
            // An open shared path may complete a boundary loop through a nonshared
            // source edge. Keep an interior witness so contraction cannot identify that
            // loop with its chord. Prefer the furthest retained point from the endpoint
            // line; straight paths use the middle retained point. This costs one extra
            // vertex per chain, independent of its source sampling density.
            let a = self
                .source
                .key(start)?
                .map(|x| f64::from(f32::from_bits(x)));
            let b = self.source.key(at)?.map(|x| f64::from(f32::from_bits(x)));
            let direction: [f64; 3] = std::array::from_fn(|k| b[k] - a[k]);
            let length2 = direction.iter().map(|x| x * x).sum::<f64>();
            let middle = begin + (end - begin) / 2;
            let mut chosen: Option<(f64, u64)> = None;
            for i in begin + 1..end {
                let p = arc.get(i)?;
                if p[2] == 0 {
                    continue;
                }
                let p = self.source.key(p[0])?.map(|x| f64::from(f32::from_bits(x)));
                let t = (0..3).map(|k| (p[k] - a[k]) * direction[k]).sum::<f64>() / length2;
                let distance2 = (0..3)
                    .map(|k| (p[k] - a[k] - t * direction[k]).powi(2))
                    .sum::<f64>();
                if chosen.is_none_or(|(distance, old)| {
                    distance2 > distance
                        || (distance2 == distance && i.abs_diff(middle) < old.abs_diff(middle))
                }) {
                    chosen = Some((distance2, i));
                }
            }
            if let Some((_, witness)) = chosen {
                pieces.insert(1, witness);
            }
        }
        // A1: the least step whose tolerance covers every finer chord on this arc.
        let mut least = 0;
        if depth < self.source.depth {
            for i in begin..end {
                let edge = arc.get(i)?[1];
                if edge != NONE {
                    least = least.max(self.edge_finer.get(edge)?[1].saturating_sub(1) as u32);
                }
            }
        }
        // Relieved pieces also end at isolated ears still retained one level finer.
        let pinned = |this: &Self, step: u32| -> Result<Vec<u64>, JobError> {
            let mut p = pieces.clone();
            if step > 0 {
                for i in begin + 1..end {
                    let a = arc.get(i)?;
                    if a[2] != 0 && Source::ear_pinned(&this.source.vertices.get(a[0])?, depth) {
                        p.push(i);
                    }
                }
                p.sort_unstable();
                p.dedup();
            }
            Ok(p)
        };
        let mut owners = Vec::new();
        let mut at = e[2];
        while at < e[3] {
            let owner = self.source.owner(at, depth)?;
            owners.push(owner);
            let end = self.source.nodes.get(owner)?[4];
            at = graph::next_owner(&self.source.owners, at, e[3], end)?;
        }
        // Chords this arc emits at `step`. A relieved step is taken only where it emits
        // fewer chords than the least step; isolated ear pins can make it stricter.
        let chords = |this: &Self, step: u32| -> Result<u64, JobError> {
            let epsilon = this.ladder.relief(depth, step);
            let mut n = 0;
            for pair in pinned(this, step)?.windows(2) {
                n += this.count(arc, pair[0], pair[1], epsilon)?;
            }
            Ok(n)
        };
        match mode {
            Mode::Emit(requests) => {
                let mut step = owners
                    .iter()
                    .filter_map(|o| requests.get(o).copied())
                    .fold(least, u32::max);
                if step > least && chords(self, step)? >= chords(self, least)? {
                    step = least;
                }
                let epsilon = self.ladder.relief(depth, step);
                for pair in pinned(self, step)?.windows(2) {
                    self.shortcuts(
                        arc,
                        pair[0],
                        pair[1],
                        begin,
                        first_edge,
                        start,
                        depth,
                        [e[2], e[3]],
                        epsilon,
                        step,
                    )?;
                }
            }
            Mode::Count(counts) => {
                if !owners.iter().any(|o| counts.contains_key(o)) {
                    return Ok(());
                }
                let base = chords(self, least)?;
                let mut per = [base; RELIEF_STEPS as usize + 1];
                for (j, n) in per.iter_mut().enumerate().skip(1) {
                    if j as u32 > least {
                        *n = chords(self, j as u32)?.min(base);
                    }
                }
                for o in &owners {
                    if let Some(c) = counts.get_mut(o) {
                        for (c, n) in c.iter_mut().zip(per) {
                            *c += n;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Chords one piece would emit at `epsilon` (the relief census; nothing is written).
    fn count(&self, arc: &Disk<3>, first: u64, last: u64, epsilon: f64) -> Result<u64, JobError> {
        let vertices = &self.source.vertices;
        let mut n = 0;
        curve::shortcuts(
            first,
            last,
            epsilon,
            &mut |i| {
                let v = vertices.get(arc.get(i)?[0])?;
                Ok([v[0] as u32, v[1] as u32, v[2] as u32].map(f32::from_bits))
            },
            &mut |i| Ok(arc.get(i)?[2] != 0),
            &mut |_, _, _| {
                n += 1;
                Ok(())
            },
        )?;
        Ok(n)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "predates the lint policy; scripts/lint_debt.py keeps the count from growing"
    )]
    fn shortcuts(
        &mut self,
        arc: &Disk<3>,
        first: u64,
        last: u64,
        origin_at: u64,
        chain: u64,
        origin: u64,
        depth: u32,
        owners: [u64; 2],
        epsilon: f64,
        step: u32,
    ) -> Result<(), JobError> {
        // The callback's mutation touches only output stores. Source reads and node-owner
        // traversal use immutable input stores; no source-sized chord vector is retained.
        let Source {
            vertices,
            owners: owner_store,
            leaves,
            nodes,
            ..
        } = &mut self.source;
        let vertex_count = vertices.len;
        let maps = &mut self.maps;
        let segments = &mut self.segments;
        let refs = &mut self.refs;
        let edge_finer = &self.edge_finer;
        let edge_level = &mut self.edge_level;
        let relief = &mut self.relief[depth as usize];
        curve::shortcuts(
            first,
            last,
            epsilon,
            &mut |i| {
                let v = vertices.get(arc.get(i)?[0])?;
                Ok([v[0] as u32, v[1] as u32, v[2] as u32].map(f32::from_bits))
            },
            &mut |i| Ok(arc.get(i)?[2] != 0),
            &mut |a, b, bound| {
                let va = arc.get(a)?[0];
                let vb = arc.get(b)?[0];
                // A1: finer certified displacement on this subarc, and this level's.
                let mut finer = 0f64;
                for i in a..b {
                    let edge = arc.get(i)?[1];
                    finer = finer.max(f64::from_bits(edge_finer.get(edge)?[0]));
                    edge_level.set(
                        edge,
                        [bound.to_bits(), u64::from(step), u64::from(depth) + 1],
                    )?;
                }
                if step > 0 {
                    relief.segments += 1;
                    relief.max_step = relief.max_step.max(step);
                }
                let segment = [
                    va,
                    vb,
                    chain,
                    origin,
                    a - origin_at,
                    b - origin_at,
                    bound.to_bits(),
                    u64::from(depth),
                    owners[0],
                    owners[1],
                    finer.to_bits(),
                    u64::from(step),
                ];
                let id = segments.push(segment)?;
                for i in a..=b {
                    let vertex = arc.get(i)?[0];
                    let map = if i == a || i == b {
                        [vertex, vertex, 0]
                    } else {
                        [va, vb, ((i - a) as f64 / (b - a) as f64).to_bits()]
                    };
                    maps.set(u64::from(depth) * vertex_count + vertex, map)?;
                }
                let mut at = owners[0];
                while at < owners[1] {
                    let leaf = owner_store.get(at)?[0];
                    let owner = leaves.get(leaf)?[1 + depth as usize];
                    let mut node = nodes.get(owner)?;
                    node[6] = refs.push([node[6], id])?;
                    node[7] += 1;
                    nodes.set(owner, node)?;
                    // A fan can have arbitrarily many leaves per coarse owner. Visit
                    // distinct owners in output order, with O(log rows) reads per ref.
                    at = graph::next_owner(owner_store, at, owners[1], node[4])?;
                }
                Ok(())
            },
        )?;
        Ok(())
    }
    /// Owners of a live vertex holding none of its live edges get the point, as for a
    /// degree-zero contact. Live edges stamp their depth owners first: one owner read
    /// per run, as segment emission costs.
    fn point_contacts(
        &mut self,
        v: u64,
        vertex: &[u64; 8],
        depth: u32,
        active: &Disk<1>,
        reached: &mut Disk<1>,
    ) -> Result<(), JobError> {
        let mut edge = vertex[5];
        while edge != NONE {
            let e = self.source.edges.get(edge)?;
            let mut at = if active.get(edge)?[0] != 0 {
                e[2]
            } else {
                e[3]
            };
            while at < e[3] {
                let owner = self.source.owner(at, depth)?;
                reached.set(owner, [v + 1])?;
                let end = self.source.nodes.get(owner)?[4];
                at = graph::next_owner(&self.source.owners, at, e[3], end)?;
            }
            edge = if e[0] == v { e[4] } else { e[5] };
        }
        let (mut at, mut segment) = (vertex[3], NONE);
        while at < vertex[4] {
            let owner = self.source.owner(at, depth)?;
            let mut node = self.source.nodes.get(owner)?;
            if reached.get(owner)?[0] != v + 1 {
                if segment == NONE {
                    let s = [
                        v,
                        v,
                        NONE,
                        v,
                        0,
                        0,
                        0,
                        u64::from(depth),
                        vertex[3],
                        vertex[4],
                        0,
                        0,
                    ];
                    segment = self.segments.push(s)?;
                }
                node[6] = self.refs.push([node[6], segment])?;
                node[7] += 1;
                self.source.nodes.set(owner, node)?;
            }
            at = graph::next_owner(&self.source.owners, at, vertex[4], node[4])?;
        }
        Ok(())
    }
    fn emit_segment(&mut self, segment: [u64; 12]) -> Result<(), JobError> {
        let id = self.segments.push(segment)?;
        let mut at = segment[8];
        while at < segment[9] {
            let owner = self.source.owner(at, segment[7] as u32)?;
            let mut node = self.source.nodes.get(owner)?;
            node[6] = self.refs.push([node[6], id])?;
            node[7] += 1;
            self.source.nodes.set(owner, node)?;
            at = graph::next_owner(&self.source.owners, at, segment[9], node[4])?;
        }
        Ok(())
    }

    /// The contract of `key`. A member of a single-child chain is published once, at the
    /// chain's top, for every member: its relief floor is the largest over the chain.
    pub(super) fn contract(
        &self,
        key: TileKey,
        limit: usize,
    ) -> Result<Option<Contract>, JobError> {
        let Some(mut contract) = self.own_contract(key, limit)? else {
            return Ok(None);
        };
        // Chain members are consecutive in the frozen tree's preorder.
        let top = self.publication_depth(key)?;
        let first = self.source.node(TileKey { depth: top, ..key })?;
        for (id, depth) in (first..self.source.nodes.len).zip(top..) {
            let n = self.source.nodes.get(id)?;
            if (n[0], n[1], n[2]) != (key.first, key.end, u64::from(depth))
                || n[5] & 1 == 1
                || (n[5] >> 1) as u32 != top
            {
                break;
            }
            if depth != key.depth {
                contract.relief_floor = contract.relief_floor.max(self.relief_floor(n)?);
            }
        }
        Ok(Some(contract))
    }

    /// One node's relief floor (A1): 8 x its largest relieved certificate's own plus finer
    /// certified displacement.
    fn relief_floor(&self, n: [u64; 8]) -> Result<f64, JobError> {
        let (mut at, mut floor) = (n[6], 0f64);
        while at != NONE {
            let link = self.refs.get(at)?;
            let s = self.segments.get(link[1])?;
            if s[0] != s[1] && s[11] > 0 {
                floor = floor.max(8. * (f64::from_bits(s[6]) + f64::from_bits(s[10])));
            }
            at = link[0];
        }
        Ok(floor)
    }

    /// The number of linked contract items of `key`'s own node, read without materializing
    /// them: what `contract` compares against its limit (`mesh-inspect` reports it for a
    /// contract past the limit).
    pub(super) fn contract_items(&self, key: TileKey) -> Result<u64, JobError> {
        Ok(self.source.nodes.get(self.source.node(key)?)?[7])
    }

    /// The contract of `key`'s own node.
    fn own_contract(&self, key: TileKey, limit: usize) -> Result<Option<Contract>, JobError> {
        let n = self.source.nodes.get(self.source.node(key)?)?;
        if n[7] > limit as u64 {
            return Ok(None);
        }
        let mut at = n[6];
        let count = n[7] as usize;
        let mut interface = Vec::with_capacity(3 * count);
        let mut certificates = Vec::with_capacity(count);
        let mut terminal_vertices = Vec::with_capacity(2 * count);
        let mut holders = Vec::with_capacity(2 * count);
        let mut relief_floor = 0f64;
        while at != NONE {
            let link = self.refs.get(at)?;
            let s = self.segments.get(link[1])?;
            let va = self.source.vertices.get(s[0])?;
            let vb = self.source.vertices.get(s[1])?;
            let a = [va[0] as u32, va[1] as u32, va[2] as u32];
            let b = [vb[0] as u32, vb[1] as u32, vb[2] as u32];
            for (p, v) in [(a, va), (b, vb)] {
                if v[7] as u32 <= key.depth {
                    terminal_vertices.push(p);
                }
                holders.push(Holders {
                    key: p,
                    lo: self.source.owners.get(v[3])?[0] as u32,
                    hi: self.source.owners.get(v[4] - 1)?[0] as u32,
                });
            }
            let lo = self.source.owners.get(s[8])?[0] as u32;
            let hi = self.source.owners.get(s[9] - 1)?[0] as u32;
            for p in [a, b] {
                interface.push(InterfaceRow::new(key.leaf_begin, p, p, lo, hi));
            }
            if a != b {
                interface.push(InterfaceRow::new(
                    key.leaf_begin,
                    a.min(b),
                    a.max(b),
                    lo,
                    hi,
                ));
                let edge = self.source.edges.get(s[2])?;
                certificates.push(Certificate {
                    endpoints: [a, b],
                    chain: [self.source.key(edge[0])?, self.source.key(edge[1])?],
                    origin: self.source.key(s[3])?,
                    first: s[4],
                    end: s[5],
                    epsilon: f64::from_bits(s[6]),
                    terminal_lock: edge[6] <= u64::from(key.depth),
                    step: s[11] as u32,
                });
                if s[11] > 0 {
                    relief_floor =
                        relief_floor.max(8. * (f64::from_bits(s[6]) + f64::from_bits(s[10])));
                }
            }
            at = link[0];
        }
        super::interfaces::merge_rows(&mut interface);
        certificates.sort_by_key(|c| (c.endpoints, c.chain, c.first));
        terminal_vertices.sort_unstable();
        terminal_vertices.dedup();
        holders.sort_unstable_by_key(|h| h.key);
        holders.dedup();
        Ok(Some(Contract {
            depth: key.depth,
            interface,
            certificates,
            terminal_vertices,
            holders,
            scale: self.scale(self.publication_depth(key)?),
            relief_floor,
        }))
    }
    /// The tree node of `key` (its index in the frozen tree's preorder).
    pub(super) fn node_id(&self, key: TileKey) -> Result<u64, JobError> {
        self.source.node(key)
    }

    /// The linked items of every node whose own contract list holds at most `limit` items.
    pub(super) fn listed_item_count(&self, limit: usize) -> Result<u64, JobError> {
        let mut total = 0u64;
        for id in 0..self.source.nodes.len {
            let n = self.source.nodes.get(id)?[7];
            if n <= limit as u64 {
                total = total.saturating_add(n);
            }
        }
        Ok(total)
    }

    /// Every linked item of the nodes whose own contract list holds at most `limit` items,
    /// as `[node, first endpoint, second endpoint, chain source edge]` sorted by node, in
    /// one pass (`mesh-inspect`'s forecast). A list links each reference to a strictly
    /// smaller index, and every reference belongs to exactly one list, so taking the
    /// greatest unread reference of all lists at a time reads the reference store once in
    /// descending order; segments are emitted just before their references, so they are
    /// read in nearly the same order. Following one list at a time instead reads a page
    /// per item, each page about once per list that crosses it. `progress` advances by
    /// item; the caller begins its step with [`Self::listed_item_count`].
    pub(super) fn listed_items<'s, 'p>(
        &self,
        scratch: &'s Scratch,
        sorter: SortConfig,
        limit: usize,
        progress: &'s StepProgress<'p>,
    ) -> Result<ListedItems<'s, 'p>, JobError> {
        let mut sorted = ExternalSorter::<disk::Words<4>>::new(scratch, sorter)?.observed(progress);
        // At most one head per node, as `listed_items_memory` charges; a head is popped
        // before its successor is pushed, so the heap never grows past it.
        let mut heads: Vec<ListHead> = Vec::new();
        heads
            .try_reserve_exact(usize::try_from(self.source.nodes.len).unwrap_or(usize::MAX))
            .map_err(|_| {
                JobError::new(
                    crate::JobErrorKind::ResourceLimit,
                    "contract list heads allocation failed",
                )
            })?;
        for id in 0..self.source.nodes.len {
            if id.is_multiple_of(4096) {
                progress.check()?;
            }
            let n = self.source.nodes.get(id)?;
            if n[7] <= limit as u64 && n[6] != NONE {
                heads.push((n[6], id, n[7]));
            }
        }
        let mut heads = std::collections::BinaryHeap::from(heads);
        let mut done = 0u64;
        while let Some((at, node, left)) = heads.pop() {
            if done.is_multiple_of(4096) {
                progress.advance(done)?;
            }
            done += 1;
            let link = self.refs.get(at)?;
            let s = self.segments.get(link[1])?;
            sorted.push(disk::Words([node, s[0], s[1], s[2]]))?;
            match (link[0], left) {
                (NONE, 1) => {}
                (NONE, _) | (_, 0 | 1) => {
                    return Err(state("a contract list differs from its item count"))
                }
                (next, _) => heads.push((next, node, left - 1)),
            }
        }
        Ok(ListedItems {
            stream: sorted.finish()?.peekable(),
            progress,
        })
    }

    /// What `mesh-inspect`'s forecast reads of a contract at `depth` with these linked
    /// items (`[first endpoint, second endpoint, chain source edge]`), equal to the
    /// corresponding values of [`Self::contract`]; its rows, certificates and holders are
    /// not built.
    pub(super) fn contract_counts(
        &self,
        depth: u32,
        items: &[[u64; 3]],
    ) -> Result<ContractCounts, JobError> {
        let points = Self::contract_points(items);
        // Edge rows: one per distinct endpoint pair (ids order as keys do).
        let mut edges: Vec<[u64; 2]> = items
            .iter()
            .filter(|[a, b, _]| a != b)
            .map(|&[a, b, _]| [a.min(b), a.max(b)])
            .collect();
        edges.sort_unstable();
        edges.dedup();
        let depth = u64::from(depth);
        let mut terminal_locks = 0u64;
        for &p in &points {
            // The low half holds the minimum terminal leaf depth (as `own_contract` reads it).
            if self.source.vertices.get(p)?[7] & u64::from(u32::MAX) <= depth {
                terminal_locks += 1;
            }
        }
        // One certificate per linked chord, locked when its source edge is terminal.
        let mut chains: Vec<u64> = items
            .iter()
            .filter(|[a, b, _]| a != b)
            .map(|&[_, _, edge]| edge)
            .collect();
        chains.sort_unstable();
        for edge in chains {
            if self.source.edges.get(edge)?[6] <= depth {
                terminal_locks += 1;
            }
        }
        Ok(ContractCounts {
            segments: edges.len() as u64,
            points,
            terminal_locks,
        })
    }

    /// The points of a contract with these linked items, as sorted, distinct source vertex
    /// ids. Vertex ids follow key order, so they count and compare as the contract's point
    /// keys do.
    pub(super) fn contract_points(items: &[[u64; 3]]) -> Vec<u64> {
        let mut points: Vec<u64> = items.iter().flat_map(|&[a, b, _]| [a, b]).collect();
        points.sort_unstable();
        points.dedup();
        points
    }

    pub(super) fn mapping(
        &self,
        depth: u32,
        key: Key,
    ) -> Result<Option<(Key, Key, f64, u8)>, JobError> {
        let Some(vertex) = self.source.vertex(key)? else {
            return Ok(None);
        };
        let m = self
            .maps
            .get(u64::from(depth) * self.source.vertices.len + vertex)?;
        Ok(Some((
            self.source.key(m[0])?,
            self.source.key(m[1])?,
            f64::from_bits(m[2]),
            self.source.vertices.get(vertex)?[7] as u8,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        mesh_large::{
            hlod::scratch::ScratchLedger,
            records::{Bounds, NodeKind},
        },
        spill::SequenceWriter,
    };
    fn k(x: f32, y: f32) -> Key {
        [x, y, 0.].map(f32::to_bits)
    }
    #[test]
    fn contract_normalization_at_link_limit_fits_its_own_allowance() {
        // Model the constructor's maximum linked-item emission, then execute its
        // actual normalization operations. Each disjoint triangle has terminal
        // one-edge arcs: nine emitted rows merge to six. This includes the moving
        // shrink counted by test_alloc, with all certificate/terminal buffers live.
        // Disk graph caches have their own fixed allowance and are not modeled here.
        let limit = 6 * super::super::TRIANGLE_CAP;
        let ((rows, certificates, terminals), peak) = crate::test_alloc::peak(|| {
            let mut rows = Vec::with_capacity(3 * limit);
            let mut certificates = Vec::with_capacity(limit);
            let mut terminals = Vec::with_capacity(2 * limit);
            for i in 0..limit / 3 {
                let x = 4. * i as f32;
                let points = [k(x, 0.), k(x + 1., 0.), k(x, 1.)];
                for j in 0..3 {
                    let (a, b) = (points[j], points[(j + 1) % 3]);
                    for p in [a, b] {
                        rows.push(InterfaceRow::new(0, p, p, 0, 1));
                        terminals.push(p);
                    }
                    rows.push(InterfaceRow::new(0, a.min(b), a.max(b), 0, 1));
                    certificates.push(Certificate {
                        endpoints: [a, b],
                        chain: [a, b],
                        origin: a,
                        first: 0,
                        end: 1,
                        epsilon: 0.,
                        terminal_lock: true,
                        step: 0,
                    });
                }
            }
            super::super::interfaces::merge_rows(&mut rows);
            certificates.sort_by_key(|c| (c.endpoints, c.chain, c.first));
            terminals.sort_unstable();
            terminals.dedup();
            (rows, certificates, terminals)
        });
        assert_eq!(rows.len(), 2 * limit);
        assert_eq!(certificates.len(), limit);
        assert_eq!(terminals.len(), limit);
        let charge = contract_workspace_bytes(limit as u64);
        eprintln!("CONTRACT CONSTRUCTION: links={limit}, peak={peak}, charge={charge}");
        assert!(peak as u64 <= charge);
    }
    /// The borders' scratch, temporaries included, never passes what they were admitted.
    /// The emission stays within its bounds, and the borders' scratch, temporaries
    /// included, within their admission.
    fn within_admission(
        scratch: &Scratch,
        before: u64,
        lease: &ComponentLease<'_, '_>,
        borders: &Borders,
    ) {
        let peak = scratch.stage_peak_bytes().saturating_sub(before);
        assert!(peak <= lease.bytes(), "{peak} past {}", lease.bytes());
        assert!(
            borders.segments.len <= borders.bounds.segments
                && borders.refs.len <= borders.bounds.refs,
            "{} segments and {} references past {:?}",
            borders.segments.len,
            borders.refs.len,
            borders.bounds
        );
    }

    /// Counts for the build admission of small test graphs (the test ledger admits all).
    fn test_counts() -> ScratchCounts {
        ScratchCounts {
            rows: 1 << 20,
            points: 1 << 20,
            segments: 1 << 20,
            nodes: 1 << 10,
            leaves: 1 << 10,
            height: 31,
        }
    }

    #[test]
    fn large_source_scratch_admission_is_checked_and_incidence_bounded() {
        let sorter = SortConfig {
            run_bytes: 1 << 20,
            fan_in: 8,
            buffer_bytes: 1 << 16,
            jobs: 1,
            writers: 0,
        };
        for n in [500_000_000u64, 1_000_000_000] {
            // The production caller's worst-case interface counts, with a full-height
            // tree. The envelope covers all persistent levels and the external sort.
            let need = scratch_need(
                ScratchCounts {
                    rows: 6 * n,
                    points: 3 * n,
                    segments: 3 * n,
                    nodes: 2 * n / 8192,
                    leaves: n / 8192,
                    height: 31,
                },
                sorter,
            )
            .unwrap();
            assert!((20_000 * n..=32_768 * n).contains(&need));
        }
        for field in 0..5 {
            let mut counts = [1; 5];
            counts[field] = u64::MAX;
            let counts = ScratchCounts {
                rows: counts[0],
                points: counts[1],
                segments: counts[2],
                nodes: counts[3],
                leaves: counts[4],
                height: 31,
            };
            assert!(scratch_need(counts, sorter).is_none());
        }
    }

    #[test]
    fn fan_owner_reads_scale_with_rows_and_height_not_chords_times_fan() {
        fn subtree(tree: &mut SequenceWriter<NodeRecord>, first: u64, end: u64, depth: u8) {
            let leaf = end - first == 1;
            tree.push(&NodeRecord {
                first,
                end,
                cell_depth: depth,
                cell: 0,
                path_len: 0,
                path: 0,
                last_octant: 0,
                kind: if leaf {
                    NodeKind::Leaf
                } else {
                    NodeKind::Internal
                },
                children: if leaf { 0 } else { 2 },
                bytes: 0,
                bounds: Bounds::EMPTY,
            })
            .unwrap();
            if !leaf {
                let middle = first + (end - first) / 2;
                subtree(tree, first, middle, depth + 1);
                subtree(tree, middle, end, depth + 1);
            }
        }
        let mut previous_reads = None;
        for exponent in [7u32, 9] {
            // One fan with R owners followed by L zigzag edges, R=L. At depth one
            // every edge has the same two owners, and every zigzag emits a chord.
            let r = 1u64 << exponent;
            let work = tempfile::tempdir().unwrap();
            let scratch = Scratch::create(Some(work.path()), 0).unwrap();
            let sorter = SortConfig {
                run_bytes: 1 << 20,
                fan_in: 8,
                buffer_bytes: 1 << 16,
                jobs: 1,
                writers: 0,
            };
            let mut tree = SequenceWriter::new(&scratch, "tree", 1 << 16).unwrap();
            subtree(&mut tree, 0, 2 * r, 0);
            let tree = tree.finish().unwrap();
            let mut rows = Vec::new();
            for i in 0..r {
                let a = k(i as f32, ((i % 2) * 10) as f32);
                let b = k((i + 1) as f32, (((i + 1) % 2) * 10) as f32);
                let owners: Vec<u32> = if i == 0 {
                    (0..r as u32).chain([r as u32]).collect()
                } else {
                    vec![(i % r) as u32, (r + i % r) as u32]
                };
                for owner in owners {
                    rows.push(InterfaceRow::new(owner, a.min(b), a.max(b), 0, 1));
                    rows.push(InterfaceRow::new(owner, a, a, 0, 1));
                    rows.push(InterfaceRow::new(owner, b, b, 0, 1));
                }
            }
            rows.sort_unstable();
            rows.dedup();
            let row_count = rows.len() as u64;
            let height = exponent + 1;
            let ledger = ScratchLedger::new(&scratch, scratch.live_bytes(), &|_| Ok(()));
            scratch.begin_stage();
            let before = scratch.live_bytes();
            let (borders, lease) = Borders::build(
                &scratch,
                sorter,
                &tree,
                rows.into_iter().map(Ok),
                std::iter::empty(),
                std::iter::empty(),
                0,
                (4096., super::super::TRIANGLE_CAP, test_counts()),
                &ledger,
                crate::mesh_large::phases::StepProgress::silent(),
            )
            .unwrap();
            within_admission(&scratch, before, &lease, &borders);
            let node = borders
                .source
                .node(TileKey {
                    first: 0,
                    end: r,
                    depth: 1,
                    leaf_begin: 0,
                    leaf_end: r as u32,
                })
                .unwrap();
            assert_eq!(borders.source.nodes.get(node).unwrap()[7], r);
            // Count all positioned owner-row reads, including classification and owner
            // comparisons, not just emission. The original raw-run emission violates
            // both this height-scaled ceiling and the fourfold-input growth check.
            let reads = borders.source.owners.read_count();
            assert!(
                reads <= 4 * row_count * u64::from(height + 1),
                "R={r}, rows={row_count}, owner reads={reads}"
            );
            if let Some(previous) = previous_reads {
                assert!(
                    reads <= 6 * previous,
                    "owner reads grew from {previous} to {reads}"
                );
            }
            previous_reads = Some(reads);
            ledger.check().unwrap();
        }
    }

    #[test]
    fn more_than_24576_mandatory_junctions_cannot_fit_the_hard_cap() {
        let interface = (0..24_577)
            .map(|i| {
                let p = k(i as f32, 0.);
                InterfaceRow::new(0, p, p, 0, 1)
            })
            .collect();
        let c = Contract {
            depth: 0,
            interface,
            certificates: vec![],
            terminal_vertices: vec![],
            holders: vec![],
            scale: 1.,
            relief_floor: 0.,
        };
        assert_eq!(c.minimum_triangles(), 8193);
        assert!(c.minimum_triangles() > super::super::TRIANGLE_CAP);
    }
    /// A one-bay wall strip between two depth-1 border lines L and R, split along its
    /// diagonals between the two leaves of cell A: the diagonals are leaf-level borders,
    /// interior at depth 1. Removing L1 there leaves faces of interface points only (no
    /// realization), so face retention must keep it although its face edges are shared at
    /// a deeper depth.
    #[test]
    fn a_strip_between_two_borders_retains_its_run_vertex_across_depths() {
        let work = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(work.path()), 0).unwrap();
        let sorter = SortConfig {
            run_bytes: 4096,
            fan_in: 2,
            buffer_bytes: 1024,
            jobs: 1,
            writers: 0,
        };
        let mut tree = SequenceWriter::new(&scratch, "tree", 1024).unwrap();
        // Cells A and B are chains to depth 2, each with two depth-3 leaves; A's split
        // the strip.
        for (first, end, depth, internal, children) in [
            (0, 4, 0, true, 2),
            (0, 2, 1, true, 1),
            (0, 2, 2, true, 2),
            (0, 1, 3, false, 0),
            (1, 2, 3, false, 0),
            (2, 4, 1, true, 1),
            (2, 4, 2, true, 2),
            (2, 3, 3, false, 0),
            (3, 4, 3, false, 0),
        ] {
            tree.push(&NodeRecord {
                first,
                end,
                cell_depth: depth,
                cell: 0,
                path_len: 0,
                path: 0,
                last_octant: 0,
                kind: if internal {
                    NodeKind::Internal
                } else {
                    NodeKind::Leaf
                },
                children,
                bytes: 0,
                bounds: Bounds::EMPTY,
            })
            .unwrap();
        }
        let tree = tree.finish().unwrap();
        // Four floors: lines L and R (the strip), X left of L, Y right of R.
        let line = |x: f32| -> Vec<Key> { (0..5).map(|j| k(x, j as f32)).collect() };
        let (l, r, x, y) = (line(0.), line(1.), line(-1.), line(2.));
        let mut leaves: [Vec<[Key; 3]>; 4] = Default::default();
        for j in 0..4 {
            // Leaves 0 and 1 (cell A) split each strip quad on its diagonal; leaf 2 (cell
            // B) is left of L, leaf 3 (cell B) right of R.
            leaves[0].push([l[j], r[j], l[j + 1]]);
            leaves[1].push([r[j], r[j + 1], l[j + 1]]);
            leaves[2].extend([[x[j], l[j], x[j + 1]], [l[j], l[j + 1], x[j + 1]]]);
            leaves[3].extend([[r[j], y[j], r[j + 1]], [y[j], y[j + 1], r[j + 1]]]);
        }
        // Every point and edge held by more than one leaf, with its leaf span.
        let mut held: std::collections::BTreeMap<(Key, Key), Vec<u32>> = Default::default();
        for (leaf, faces) in leaves.iter().enumerate() {
            let mut items = std::collections::BTreeSet::new();
            for f in faces {
                for i in 0..3 {
                    let (a, b) = (f[i], f[(i + 1) % 3]);
                    items.insert((a, a));
                    items.insert((a.min(b), a.max(b)));
                }
            }
            for item in items {
                held.entry(item).or_default().push(leaf as u32);
            }
        }
        let mut rows = Vec::new();
        for ((a, b), holders) in &held {
            if holders.len() > 1 {
                for &leaf in holders {
                    rows.push(InterfaceRow::new(
                        leaf,
                        *a,
                        *b,
                        holders[0],
                        *holders.last().unwrap(),
                    ));
                }
            }
        }
        rows.sort_unstable();
        let faces: Vec<[Key; 3]> = leaves.iter().flatten().copied().collect();
        let ledger = ScratchLedger::new(&scratch, scratch.live_bytes(), &|_| Ok(()));
        scratch.begin_stage();
        let before = scratch.live_bytes();
        let (borders, lease) = Borders::build(
            &scratch,
            sorter,
            &tree,
            rows.clone().into_iter().map(Ok),
            std::iter::empty(),
            faces.clone().into_iter().map(Ok),
            0,
            (4096., super::super::TRIANGLE_CAP, test_counts()),
            &ledger,
            crate::mesh_large::phases::StepProgress::silent(),
        )
        .unwrap();
        within_admission(&scratch, before, &lease, &borders);
        assert_counts_match_contracts(&borders, &scratch, sorter);
        let cell_a = borders
            .contract(
                TileKey {
                    first: 0,
                    end: 2,
                    depth: 2,
                    leaf_begin: 0,
                    leaf_end: 2,
                },
                100,
            )
            .unwrap()
            .unwrap();
        let points: std::collections::BTreeSet<Key> =
            cell_a.interface.iter().flat_map(|r| [r.a, r.b]).collect();
        // L2 is the open path's witness; L1 and L3 have only interface neighbours in A.
        for v in [&l[1], &l[3]] {
            assert!(points.contains(v), "{v:?} contracted away: {points:?}");
        }
        drop((borders, lease));
        // Too little space once the graph is built: refused at that admission, the second
        // and last, before any level runs; the build's admission is released.
        let admissions = std::sync::atomic::AtomicU64::new(0);
        let refuse_second = |_: u64| {
            use std::sync::atomic::Ordering::Relaxed;
            if admissions.fetch_add(1, Relaxed) + 1 == 2 {
                return Err(JobError::new(
                    crate::JobErrorKind::ResourceLimit,
                    "no space",
                ));
            }
            Ok(())
        };
        let base = scratch.live_bytes();
        let ledger = ScratchLedger::new(&scratch, base, &refuse_second);
        let error = Borders::build(
            &scratch,
            sorter,
            &tree,
            rows.into_iter().map(Ok),
            std::iter::empty(),
            faces.into_iter().map(Ok),
            0,
            (4096., super::super::TRIANGLE_CAP, test_counts()),
            &ledger,
            crate::mesh_large::phases::StepProgress::silent(),
        )
        .err()
        .expect("the post-graph admission is refused");
        assert_eq!(error.kind(), crate::JobErrorKind::ResourceLimit);
        assert!(error.to_string().contains("no space"), "{error}");
        assert_eq!(admissions.load(std::sync::atomic::Ordering::Relaxed), 2);
        // Nothing stays admitted: the build's lease was released before the refused one.
        assert_eq!(ledger.bound(), base);
    }
    #[test]
    fn all_owners_share_certified_chords_and_terminal_leaves_stay_exact() {
        let work = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(work.path()), 0).unwrap();
        let sorter = SortConfig {
            run_bytes: 4096,
            fan_in: 2,
            buffer_bytes: 1024,
            jobs: 1,
            writers: 0,
        };
        let mut tree = SequenceWriter::new(&scratch, "tree", 1024).unwrap();
        for (first, end, depth, internal, children) in [
            (0, 4, 0, true, 2),
            (0, 2, 1, true, 2),
            (0, 1, 2, false, 0),
            (1, 2, 2, false, 0),
            (2, 4, 1, true, 2),
            (2, 3, 2, false, 0),
            (3, 4, 2, false, 0),
        ] {
            tree.push(&NodeRecord {
                first,
                end,
                cell_depth: depth,
                cell: 0,
                path_len: 0,
                path: 0,
                last_octant: 0,
                kind: if internal {
                    NodeKind::Internal
                } else {
                    NodeKind::Leaf
                },
                children,
                bytes: 0,
                bounds: Bounds::EMPTY,
            })
            .unwrap();
        }
        let tree = tree.finish().unwrap();
        let points = [k(0., 0.), k(1., 1.), k(2., 0.), k(3., 0.), k(4., 0.)];
        let mut rows = Vec::new();
        let isolated = k(9., 9.);
        for leaf in 0..4 {
            rows.push(InterfaceRow::new(leaf, isolated, isolated, 0, 3));
            let start = if leaf % 2 == 0 { 0 } else { 2 };
            for i in start..start + 2 {
                let (a, b) = (points[i].min(points[i + 1]), points[i].max(points[i + 1]));
                let lo = if i < 2 { 0 } else { 1 };
                rows.push(InterfaceRow::new(leaf, a, b, lo, lo + 2));
            }
            for &p in &points[start..=start + 2] {
                rows.push(InterfaceRow::new(leaf, p, p, 0, 3));
            }
        }
        rows.sort_unstable();
        let ledger = ScratchLedger::new(&scratch, scratch.live_bytes(), &|_| Ok(()));
        scratch.begin_stage();
        let before = scratch.live_bytes();
        let (borders, lease) = Borders::build(
            &scratch,
            sorter,
            &tree,
            rows.iter().copied().map(Ok),
            std::iter::empty(),
            std::iter::empty(),
            0,
            (4096., super::super::TRIANGLE_CAP, test_counts()),
            &ledger,
            crate::mesh_large::phases::StepProgress::silent(),
        )
        .unwrap();
        within_admission(&scratch, before, &lease, &borders);
        assert_counts_match_contracts(&borders, &scratch, sorter);
        let tile = |first, end, depth, leaf_begin, leaf_end| TileKey {
            first,
            end,
            depth,
            leaf_begin,
            leaf_end,
        };
        let left = borders.contract(tile(0, 2, 1, 0, 2), 100).unwrap().unwrap();
        let right = borders.contract(tile(2, 4, 1, 2, 4), 100).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&left.certificates).unwrap(),
            serde_json::to_value(&right.certificates).unwrap()
        );
        assert_eq!(
            left.certificates.len(),
            3,
            "curved source arc retained, straight tail simplified"
        );
        assert!(left
            .certificates
            .iter()
            .all(|c| c.epsilon <= left.scale / 16.));
        let middle = borders.mapping(1, points[3]).unwrap().unwrap();
        assert_eq!((middle.0, middle.1), (points[2], points[4]));
        assert_eq!(middle.2, 0.5);
        let leaf = borders.contract(tile(1, 2, 2, 1, 2), 100).unwrap().unwrap();
        assert_eq!(leaf.certificates.len(), 2);
        assert_eq!(leaf.terminal_vertices.len(), 4);
        assert!(leaf.terminal_vertices.contains(&isolated));
        assert_eq!(leaf.terminal_locks(), 6);
        assert_eq!(left.terminal_locks(), 0);
        assert!(leaf
            .certificates
            .iter()
            .all(|c| c.epsilon == 0. && c.end - c.first == 1));
        assert!(borders
            .contract(tile(0, 4, 0, 0, 4), 100)
            .unwrap()
            .unwrap()
            .interface
            .is_empty());
        drop((borders, lease));
        // A nonshared source boundary meeting point 2 can complete a hole through
        // the straight tail. Its separate open chain retains an interior witness.
        scratch.begin_stage();
        let before = scratch.live_bytes();
        let (pinned, lease) = Borders::build(
            &scratch,
            sorter,
            &tree,
            rows.into_iter().map(Ok),
            [Ok(super::super::interfaces::KeyRecord(points[2]))].into_iter(),
            std::iter::empty(),
            0,
            (4096., super::super::TRIANGLE_CAP, test_counts()),
            &ledger,
            crate::mesh_large::phases::StepProgress::silent(),
        )
        .unwrap();
        within_admission(&scratch, before, &lease, &pinned);
        assert_counts_match_contracts(&pinned, &scratch, sorter);
        let middle = pinned.mapping(1, points[3]).unwrap().unwrap();
        assert_eq!((middle.0, middle.1), (points[3], points[3]));
        assert_eq!(
            pinned
                .contract(tile(0, 2, 1, 0, 2), 100)
                .unwrap()
                .unwrap()
                .certificates
                .len(),
            4
        );
        ledger.check().unwrap();
    }

    /// Borders over a frozen tree of (first, end, depth, internal, children) rows, with
    /// noisy 41-point arcs (x, owning leaves) along y, at `cap`.
    fn noisy_borders<R>(
        nodes: &[(u64, u64, u8, bool, u8)],
        arcs: &[(f32, [u32; 2])],
        cap: usize,
        read: impl FnOnce(&Borders) -> R,
    ) -> R {
        let work = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(work.path()), 0).unwrap();
        let sorter = SortConfig {
            run_bytes: 4096,
            fan_in: 2,
            buffer_bytes: 1024,
            jobs: 1,
            writers: 0,
        };
        let mut tree = SequenceWriter::new(&scratch, "tree", 1024).unwrap();
        for &(first, end, depth, internal, children) in nodes {
            tree.push(&NodeRecord {
                first,
                end,
                cell_depth: depth,
                cell: 0,
                path_len: 0,
                path: 0,
                last_octant: 0,
                kind: if internal {
                    NodeKind::Internal
                } else {
                    NodeKind::Leaf
                },
                children,
                bytes: 0,
                bounds: Bounds::EMPTY,
            })
            .unwrap();
        }
        let tree = tree.finish().unwrap();
        let mut rows = Vec::new();
        for &(x, leaves) in arcs {
            let p: Vec<Key> = (0..=40)
                .map(|i| k(x + if i % 2 == 0 { 0.2 } else { -0.2 }, i as f32))
                .collect();
            for i in 0..=40 {
                for leaf in leaves {
                    rows.push(InterfaceRow::new(leaf, p[i], p[i], leaves[0], leaves[1]));
                }
                if i < 40 {
                    let (a, b) = (p[i].min(p[i + 1]), p[i].max(p[i + 1]));
                    for leaf in leaves {
                        rows.push(InterfaceRow::new(leaf, a, b, leaves[0], leaves[1]));
                    }
                }
            }
        }
        rows.sort_unstable();
        let ledger = ScratchLedger::new(&scratch, scratch.live_bytes(), &|_| Ok(()));
        scratch.begin_stage();
        let before = scratch.live_bytes();
        let (borders, lease) = Borders::build(
            &scratch,
            sorter,
            &tree,
            rows.into_iter().map(Ok),
            std::iter::empty(),
            std::iter::empty(),
            0,
            (4096., cap, test_counts()),
            &ledger,
            crate::mesh_large::phases::StepProgress::silent(),
        )
        .unwrap();
        within_admission(&scratch, before, &lease, &borders);
        assert_counts_match_contracts(&borders, &scratch, sorter);
        read(&borders)
    }

    /// The gathered items give every node exactly what its full contract gives
    /// `mesh-inspect`'s forecast, with every list or only the shorter ones listed.
    fn assert_counts_match_contracts(borders: &Borders, scratch: &Scratch, sorter: SortConfig) {
        let lengths: Vec<u64> = (0..borders.source.nodes.len)
            .map(|id| borders.source.nodes.get(id).unwrap()[7])
            .collect();
        let mut sorted = lengths.clone();
        sorted.sort_unstable();
        let median = sorted[sorted.len() / 2] as usize;
        // A cancelled run stops before it reads a list.
        let stop = || Err(JobError::new(crate::JobErrorKind::Cancelled, "stopped"));
        let cancelled = crate::mesh_large::phases::StepProgress::new(&stop, None);
        let refs = borders.refs.read_count();
        assert!(borders
            .listed_items(scratch, sorter, 6 * super::super::TRIANGLE_CAP, &cancelled)
            .is_err());
        assert_eq!(
            borders.refs.read_count(),
            refs,
            "no list read once cancelled"
        );
        for limit in [6 * super::super::TRIANGLE_CAP, median] {
            let listed: u64 = lengths.iter().filter(|&&n| n <= limit as u64).sum();
            assert_eq!(borders.listed_item_count(limit).unwrap(), listed);
            let progress = crate::mesh_large::phases::StepProgress::silent();
            let mut items = borders
                .listed_items(scratch, sorter, limit, progress)
                .unwrap();
            for (id, &n) in lengths.iter().enumerate() {
                let id = id as u64;
                let r = borders.source.nodes.get(id).unwrap();
                let key = tile(r[0], r[1], r[2] as u32, r[3] as u32, r[4] as u32);
                assert_eq!(borders.node_id(key).unwrap(), id);
                let full = borders.own_contract(key, limit).unwrap();
                let Some(full) = full else {
                    assert!(
                        n > limit as u64,
                        "node {id}: only a list past its limit is absent"
                    );
                    continue;
                };
                let taken = items.take(id).unwrap();
                assert_eq!(taken.len() as u64, n, "node {id} items");
                let counts = borders.contract_counts(key.depth, &taken).unwrap();
                let mut keys: Vec<Key> = full
                    .interface
                    .iter()
                    .filter(|r| r.is_vertex())
                    .map(|r| r.a)
                    .collect();
                keys.sort_unstable();
                keys.dedup();
                let ids = |p: &[u64]| -> Vec<Key> {
                    p.iter().map(|&v| borders.source.key(v).unwrap()).collect()
                };
                assert_eq!(
                    ids(&Borders::contract_points(&taken)),
                    keys,
                    "node {id} points"
                );
                assert_eq!(ids(&counts.points), keys, "node {id} counted points");
                assert_eq!(
                    counts.segments,
                    (full.interface.len() - keys.len()) as u64,
                    "node {id} segments"
                );
                assert_eq!(
                    counts.terminal_locks,
                    full.terminal_locks(),
                    "node {id} locks"
                );
                assert_eq!(
                    counts.minimum_triangles(),
                    full.minimum_triangles() as u64,
                    "node {id} minimum"
                );
            }
        }
    }

    fn tile(first: u64, end: u64, depth: u32, leaf_begin: u32, leaf_end: u32) -> TileKey {
        TileKey {
            first,
            end,
            depth,
            leaf_begin,
            leaf_end,
        }
    }

    /// Review 1 (P1): X1 -> X2 is a single-child chain beside Y1 -> Y2, a noisy arc
    /// between their leaves, relieved at depths 1 and 2. X2 is built and published in
    /// X1's slot, where Y2 (inside X's cell parent, not a sibling) delivers the same
    /// chord: G must cover 8 x (own + finer) = 16 x own, X1's floor, not X2's 8 x own.
    #[test]
    fn relief_floor_covers_every_member_of_a_single_child_chain() {
        let nodes = [
            (0, 5, 0, true, 2),
            (0, 2, 1, true, 1),
            (0, 2, 2, true, 2),
            (0, 1, 3, false, 0),
            (1, 2, 3, false, 0),
            (2, 5, 1, true, 2),
            (2, 4, 2, true, 2),
            (2, 3, 3, false, 0),
            (3, 4, 3, false, 0),
            (4, 5, 2, false, 0),
        ];
        noisy_borders(&nodes, &[(0., [0, 2])], 40, |borders| {
            let (x1, x2) = (tile(0, 2, 1, 0, 2), tile(0, 2, 2, 0, 2));
            assert_eq!(borders.publication_depth(x2).unwrap(), 1);
            assert!(borders.relief(1).segments > 0 && borders.relief(2).cells > 0);
            let top = borders.contract(x1, 1000).unwrap().unwrap();
            let built = borders.contract(x2, 1000).unwrap().unwrap();
            let own = built
                .certificates
                .iter()
                .filter(|c| c.step > 0)
                .map(|c| c.epsilon)
                .fold(0., f64::max);
            assert!(own > 0.);
            assert!(
                built.relief_floor >= 16. * own,
                "{} < 16 x {own}",
                built.relief_floor
            );
            assert_eq!(built.relief_floor, top.relief_floor);
            // Below its chain, Y2 is its own top: its floor is its own.
            let y2 = borders
                .contract(tile(2, 4, 2, 2, 4), 1000)
                .unwrap()
                .unwrap();
            assert!(y2.relief_floor < built.relief_floor);
        });
    }

    /// Review 1 (P3-2): relief triggers at depth 2 beside a depth-1 leaf A, which owns its
    /// depth-2 items itself. The level's rewind must unlink A's list too: every node's
    /// list ends after exactly its count. A's arc is pinned at every point (A is published
    /// exactly), so its owner's request would emit no fewer chords there: it stays at
    /// step 0, while the owner's other arc is relieved from 40 chords to 2.
    #[test]
    fn relief_rewind_unlinks_shallower_leaves_owning_deeper_items() {
        let nodes = [
            (0, 5, 0, true, 2),
            (0, 1, 1, false, 0),
            (1, 5, 1, true, 2),
            (1, 3, 2, true, 2),
            (1, 2, 3, false, 0),
            (2, 3, 3, false, 0),
            (3, 5, 2, true, 2),
            (3, 4, 3, false, 0),
            (4, 5, 3, false, 0),
        ];
        noisy_borders(&nodes, &[(0., [0, 1]), (10., [1, 3])], 60, |borders| {
            assert!(borders.relief(2).cells > 0);
            for id in 0..borders.source.nodes.len {
                let n = borders.source.nodes.get(id).unwrap();
                let (mut at, mut links) = (n[6], 0);
                while at != NONE && links <= n[7] {
                    assert!(at < borders.refs.len, "node {id}: reference {at} truncated");
                    at = borders.refs.get(at).unwrap()[0];
                    links += 1;
                }
                assert_eq!((at, links), (NONE, n[7]), "node {id}: list does not end");
            }
            let a = borders
                .contract(tile(0, 1, 1, 0, 1), 100_000)
                .unwrap()
                .unwrap();
            assert!(!a.certificates.is_empty());
            assert!(a.certificates.iter().all(|c| c.step == 0));
            let owner = borders
                .contract(tile(1, 3, 2, 1, 3), 100_000)
                .unwrap()
                .unwrap();
            assert_eq!(owner.certificates.iter().filter(|c| c.step > 0).count(), 2);
        });
    }
}
