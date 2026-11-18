//! `mesh-inspect`'s coarse-omission forecast (#348): an estimate, not a bake.
//!
//! It reruns exactly what the bake computes before its walk (the incidence pass and the
//! shared-border contracts, [`borders::Borders::build`]) over the same captured tree, then
//! replays the walk's structural rules for every cell without building any geometry:
//!
//! - pass-through cells (a single coarse child whose contract collapses onto it) publish
//!   their child and share its fate, as `walk`'s `close` decides;
//! - a cell whose contract is past `6 × cap` items, or needs more than `cap` triangles on its
//!   own, is omitted for certain (`ContractLowerBound`): the bake never builds it;
//! - a cell with an omitted child is omitted without a build, unless it is the ownership
//!   root and its children's frontier is complete and bounded (at most
//!   [`recovery::INPUTS`] capped tiles), when the root is rebuilt from that frontier;
//! - an omitted cell hands its parent its children as a frontier when that frontier is
//!   complete and bounded, else nothing.
//!
//! Whether a cell's own build fails is the estimate. On measured bakes every primary
//! failure was a contraction stall or a stalled proposal, never a refused assessment: the
//! parent cannot remove the border points its children keep on faces inside it, or the
//! open boundaries it must keep fill the cap. The forecast scores each cell by
//! `load = (child border points + open-boundary estimate) / cap`, where the child border
//! points are the exact contract points of its children (every border point the merged
//! input holds) and the open-boundary estimate counts the source open edges whose loop
//! crosses the cell's border (what stage 3 fixes), resampled at the resolution of the
//! children's capped tiles. A load of at least 1 predicts an omission source.
use super::{
    admit_charged_scratch,
    borders::{self, Borders},
    interfaces, recovery,
    store::TileKey,
    Settings,
};
use crate::{
    mesh_large::{
        config::TreeConfig,
        full::FullCaptured,
        phases::StepProgress,
        records::{Bounds, NodeKind, NodeRecord},
    },
    JobError, JobErrorKind,
};
use serde::Serialize;

/// A load at or above this predicts an omission source.
pub(crate) const HIGH_LOAD: f64 = 1.0;
/// A load at or above this is reported as elevated risk.
pub(crate) const ELEVATED_LOAD: f64 = 0.5;
/// Depth bins of a leaf's crossing open edges (the admitted tree height is at most 32).
pub(crate) const DEPTHS: usize = 32;

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// Per-leaf source facts the topology pass supplies, indexed by DFS leaf.
#[derive(Clone, Debug)]
pub(crate) struct LeafFacts {
    /// Source surface area in the leaf (square metres).
    pub area: f64,
    /// Open edges in the leaf whose loop's deepest containing cell is at depth `k`, by `k`:
    /// such an edge crosses the border of every ancestor cell deeper than `k`.
    pub open: [u32; DEPTHS],
    /// Their lengths, by the same `k`.
    pub length: [f32; DEPTHS],
}
impl Default for LeafFacts {
    fn default() -> Self {
        Self {
            area: 0.,
            open: [0; DEPTHS],
            length: [0.; DEPTHS],
        }
    }
}

/// Resident bytes of one leaf's facts.
pub(crate) const LEAF_FACT_BYTES: u64 = std::mem::size_of::<LeafFacts>() as u64;

/// A cell's predicted risk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Risk {
    Low,
    Elevated,
    High,
    /// The contract alone cannot fit the cap: the bake omits the cell without a build.
    Certain,
    /// Publishes its single coarse child (no build of its own).
    PassThrough,
}

/// A cell's predicted outcome in the bake.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Built,
    /// Rebuilt from a bounded frontier after a descendant omission (the root only).
    Recovered,
    /// Omitted at its own build: an omission source.
    Source,
    /// Omitted because a descendant was.
    Descendant,
}
impl Outcome {
    pub(crate) fn omitted(self) -> bool {
        matches!(self, Self::Source | Self::Descendant)
    }
}

/// One coarse cell of the tree and its forecast.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct CellForecast {
    pub first: u64,
    pub end: u64,
    pub depth: u32,
    pub leaf_begin: u32,
    pub leaf_end: u32,
    #[serde(skip)]
    pub bounds: Bounds,
    pub triangles: u64,
    pub children: u32,
    /// The cell's own contract: points and segments, or `None` past `6 × cap` items.
    pub contract_points: Option<u64>,
    pub contract_segments: Option<u64>,
    /// Linked contract items, reported also past the limit.
    pub contract_items: u64,
    pub terminal_locks: u64,
    /// The exact contract points of its children: every border point the merged input holds.
    pub child_points: u64,
    /// Source open edges in the cell whose loop crosses its border, and their length.
    pub crossing_open_edges: u64,
    pub crossing_open_length: f64,
    /// Those edges resampled at the children's capped resolution.
    pub open_estimate: f64,
    pub load: f64,
    pub risk: Risk,
    pub outcome: Outcome,
}

/// What a finished cell or leaf hands its parent.
#[derive(Clone)]
struct Handed {
    coarse: bool,
    /// Contract points of the child (its own node's), sorted and distinct; empty past the
    /// contract limit, when `points` counts its linked items instead.
    keys: Vec<interfaces::Key>,
    points: u64,
    omitted: bool,
    /// Its frontier: capped tiles it hands up (1 when built), `None` when unavailable.
    tiles: Option<u32>,
    /// Whether every handed tile is within the cap (leaves may exceed it in tests).
    capped: bool,
    /// The cell's index in the forecast, for a coarse child.
    cell: Option<usize>,
}

struct Frame {
    node: NodeRecord,
    leaf_begin: u32,
    started: usize,
    children: Vec<Handed>,
    area: f64,
    open: [u64; DEPTHS],
    length: [f64; DEPTHS],
}

/// The forecast of every coarse cell, in the walk's close order (postorder), and the
/// incidence pass's totals.
pub(crate) struct Forecast {
    pub cells: Vec<CellForecast>,
    pub incidence: interfaces::IncidenceStats,
    pub cap: usize,
    pub incidence_scratch_peak_bytes: u64,
}

/// Resident bound of the forecast stage besides the captured metadata: the incidence pass
/// or border build (whichever is larger), the border walk cache, two contracts, the stream
/// buffers and the cell records.
pub(crate) fn memory_need(captured: &FullCaptured, config: &TreeConfig, cap: usize) -> u64 {
    let sorter = config.sorter(1);
    let leaves = captured.tree.stats.leaves;
    let buffer = config.sort.buffer_bytes as u64;
    interfaces::memory_need(
        interfaces::Sorters::uniform(sorter),
        config.leaf_triangles(),
        leaves,
    )
    .max(borders::memory_need(sorter))
    .saturating_add(borders::WALK_CACHE_BYTES)
    .saturating_add(
        borders::contract_workspace_bytes((cap as u64).saturating_mul(6)).saturating_mul(2),
    )
    .saturating_add(buffer.saturating_mul(5 + config.sort.fan_in as u64 + 1))
    .saturating_add(
        captured
            .tree
            .stats
            .internal
            .saturating_mul(std::mem::size_of::<CellForecast>() as u64),
    )
    .saturating_add(u64::from(captured.tree.stats.height).saturating_mul(
        std::mem::size_of::<Frame>() as u64
            + 8 * (std::mem::size_of::<Handed>() as u64 + child_key_bytes(cap)),
    ))
    // One parent's union of its children's points.
    .saturating_add(8 * child_key_bytes(cap))
}

/// The most a child's contract points occupy: a contract within the limit has at most
/// `6 × cap` items, each with at most two points.
fn child_key_bytes(cap: usize) -> u64 {
    (cap as u64)
        .saturating_mul(12)
        .saturating_mul(std::mem::size_of::<interfaces::Key>() as u64)
}

/// Worst-case scratch of the forecast stage, for a memory-backed scratch charge: the
/// incidence pass's worst case beside the border build's.
pub(crate) fn scratch_need(captured: &FullCaptured, config: &TreeConfig) -> Option<u64> {
    let n = captured.triangles.len();
    let sorter = config.sorter(1);
    interfaces::memory_backed_need(n, sorter)?.checked_add(borders::scratch_need(
        n.checked_mul(6)?,
        n.checked_mul(3)?,
        n.checked_mul(3)?,
        captured.tree.stats.nodes,
        captured.tree.stats.leaves,
        captured.tree.stats.height.saturating_sub(1),
        sorter,
    )?)
}

/// Run the bake's incidence pass and border contracts over `captured`, then forecast every
/// coarse cell. `charged` is the memory charged for memory-backed scratch (0 on disk).
pub(crate) fn forecast(
    captured: &FullCaptured,
    config: &TreeConfig,
    settings: &Settings,
    leaves: &[LeafFacts],
    charged: u64,
    progress: &StepProgress<'_>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Forecast, JobError> {
    let cap = settings.cap;
    if leaves.len() as u64 != captured.tree.stats.leaves {
        return Err(state("leaf facts differ from the tree's leaves"));
    }
    captured.scratch.begin_stage();
    let admit = |bound: u64| admit_charged_scratch(captured, charged, bound);
    let sorter = config.sorter(1);
    let (rows, stats, source_open_ends) = interfaces::build(
        &captured.scratch,
        interfaces::Sorters::uniform(sorter),
        &captured.tree.nodes,
        &captured.triangles,
        &captured.corner_depths,
        &captured.lattice,
        &admit,
        progress,
    )?;
    let incidence_scratch_peak_bytes = captured.scratch.stage_peak_bytes();
    if incidence_scratch_peak_bytes > stats.scratch.declared() {
        return Err(JobError::new(
            JobErrorKind::ResourceLimit,
            format!(
                "incidence scratch {incidence_scratch_peak_bytes} bytes exceeded its declared \
                 bound {}",
                stats.scratch.declared()
            ),
        ));
    }
    if captured.tree.stats.internal == 0 {
        // A single leaf: the incidence totals, and no coarse cell.
        return Ok(Forecast {
            cells: Vec::new(),
            incidence: stats,
            cap,
            incidence_scratch_peak_bytes,
        });
    }
    let ledger = super::scratch::ScratchLedger::new(&captured.scratch, stats.scratch.walk, &admit);
    let border_lease = ledger.admit(
        Borders::COMPONENT,
        borders::scratch_need(
            stats.rows,
            stats.shared_points,
            stats.shared_segments,
            captured.tree.stats.nodes,
            captured.tree.stats.leaves,
            captured.tree.stats.height - 1,
            sorter,
        )
        .ok_or_else(|| {
            JobError::new(
                JobErrorKind::ResourceLimit,
                "shared-border scratch bound overflows",
            )
        })?,
    )?;
    let borders = Borders::build(
        &captured.scratch,
        sorter,
        &captured.tree.nodes,
        rows,
        source_open_ends.reader()?,
        interfaces::faces(
            &captured.triangles,
            &captured.corner_depths,
            &captured.lattice,
        )?,
        captured.tree.stats.per_depth[0].maximum_extent,
        settings.mutation.relief_cap(cap),
        &border_lease,
        check,
    )?;
    drop(source_open_ends);
    let cells = walk(captured, &borders, settings, leaves, check)?;
    ledger.check()?;
    drop(borders);
    drop(border_lease);
    Ok(Forecast {
        cells,
        incidence: stats,
        cap,
        incidence_scratch_peak_bytes,
    })
}

/// The points of a contract: sorted and distinct (the rows are merged per item).
fn point_keys(contract: &borders::Contract) -> Vec<interfaces::Key> {
    let mut keys: Vec<_> = contract
        .interface
        .iter()
        .filter(|r| r.is_vertex())
        .map(|r| r.a)
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// The border points a parent's merged input holds: the distinct union of its children's
/// contract points (a point on a face between two children is in both). A child past the
/// contract limit adds its linked item count.
fn union_points(children: &[Handed]) -> u64 {
    let mut keys: Vec<interfaces::Key> = children
        .iter()
        .flat_map(|c| c.keys.iter().copied())
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let unlisted: u64 = children
        .iter()
        .filter(|c| c.keys.is_empty())
        .map(|c| c.points)
        .sum();
    keys.len() as u64 + unlisted
}

/// Replays `walk`'s frames and `close` over the frozen tree.
fn walk(
    captured: &FullCaptured,
    borders: &Borders,
    settings: &Settings,
    leaves: &[LeafFacts],
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Vec<CellForecast>, JobError> {
    let cap = settings.cap;
    let mut cells: Vec<CellForecast> = Vec::new();
    cells
        .try_reserve_exact(usize::try_from(captured.tree.stats.internal).unwrap_or(usize::MAX))
        .map_err(|_| {
            JobError::new(
                JobErrorKind::ResourceLimit,
                "forecast cell allocation failed",
            )
        })?;
    let mut stack: Vec<Frame> = Vec::new();
    let mut leaf = 0u32;
    let close = |stack: &mut Vec<Frame>,
                 cells: &mut Vec<CellForecast>,
                 leaf: u32|
     -> Result<(), JobError> {
        let frame = stack.pop().ok_or_else(|| state("no open frame"))?;
        let parent_depth_open = !stack.is_empty();
        let node = frame.node;
        let depth = node.depth();
        let key = TileKey {
            first: node.first,
            end: node.end,
            depth,
            leaf_begin: frame.leaf_begin,
            leaf_end: leaf,
        };
        let crossing = (0..(depth as usize).min(DEPTHS)).fold((0u64, 0f64), |(n, l), k| {
            (n + frame.open[k], l + frame.length[k])
        });
        let pass_through = match (&frame.children[..], frame.started) {
            ([only], 1) => only.coarse && borders.can_collapse(node.first, node.end, depth)?,
            _ => false,
        };
        let mut frame = frame;
        let handed = if pass_through {
            let only = frame
                .children
                .pop()
                .ok_or_else(|| state("a pass-through cell without its child"))?;
            // Pass-through: the child is published here and shares its fate.
            let child = only.cell.and_then(|i| cells.get(i)).map(|c| c.outcome);
            cells.push(CellForecast {
                first: key.first,
                end: key.end,
                depth,
                leaf_begin: key.leaf_begin,
                leaf_end: key.leaf_end,
                bounds: node.bounds,
                triangles: node.triangles(),
                children: 1,
                contract_points: None,
                contract_segments: None,
                contract_items: borders.contract_items(key)?,
                terminal_locks: 0,
                child_points: only.points,
                crossing_open_edges: crossing.0,
                crossing_open_length: crossing.1,
                open_estimate: 0.,
                load: 0.,
                risk: Risk::PassThrough,
                outcome: match child {
                    Some(o) if o.omitted() => Outcome::Descendant,
                    Some(o) => o,
                    None => Outcome::Built,
                },
            });
            Handed {
                cell: Some(cells.len() - 1),
                ..only
            }
        } else {
            let contract = borders.contract(key, 6 * cap)?;
            let items = borders.contract_items(key)?;
            let (points, segments, minimum, locks) = match &contract {
                Some(c) => {
                    let points = c.interface.iter().filter(|r| r.is_vertex()).count() as u64;
                    (
                        Some(points),
                        Some(c.interface.len() as u64 - points),
                        c.minimum_triangles() as u64,
                        c.terminal_locks(),
                    )
                }
                None => (None, None, u64::MAX, 0),
            };
            let child_points = union_points(&frame.children);
            let children = frame.children.len().max(1) as f64;
            let triangles = node.triangles();
            // The children's capped tiles: their mean edge length over the cell's surface.
            let tiles = (triangles as f64).min(children * cap as f64).max(1.);
            let edge = (4. * frame.area / (3f64.sqrt() * tiles)).sqrt();
            let open_estimate = if edge > 0. && edge.is_finite() {
                (crossing.0 as f64).min(crossing.1 / edge)
            } else {
                crossing.0 as f64
            };
            let load = (child_points as f64 + open_estimate) / cap as f64;
            let risk = if contract.is_none() || minimum > cap as u64 {
                Risk::Certain
            } else if load >= HIGH_LOAD {
                Risk::High
            } else if load >= ELEVATED_LOAD {
                Risk::Elevated
            } else {
                Risk::Low
            };
            let has_omission = frame.children.iter().any(|c| c.omitted);
            let frontier = frame
                .children
                .iter()
                .try_fold((0u32, true), |(n, capped), c| {
                    c.tiles.map(|t| (n.saturating_add(t), capped && c.capped))
                })
                .filter(|&(n, capped)| capped && n > 0 && n as usize <= recovery::INPUTS)
                .map(|(n, _)| n);
            let root = !parent_depth_open || borders.publication_depth(key)? == 0;
            let recovery_allowed = settings.mutation.recovery();
            let outcome = if has_omission && !(root && recovery_allowed && frontier.is_some()) {
                Outcome::Descendant
            } else if matches!(risk, Risk::Certain | Risk::High) {
                if has_omission {
                    Outcome::Descendant
                } else {
                    Outcome::Source
                }
            } else if has_omission {
                Outcome::Recovered
            } else {
                Outcome::Built
            };
            cells.push(CellForecast {
                first: key.first,
                end: key.end,
                depth,
                leaf_begin: key.leaf_begin,
                leaf_end: key.leaf_end,
                bounds: node.bounds,
                triangles,
                children: frame.children.len() as u32,
                contract_points: points,
                contract_segments: segments,
                contract_items: items,
                terminal_locks: locks,
                child_points,
                crossing_open_edges: crossing.0,
                crossing_open_length: crossing.1,
                open_estimate,
                load,
                risk,
                outcome,
            });
            let omitted = outcome.omitted();
            Handed {
                coarse: true,
                keys: contract.as_ref().map(point_keys).unwrap_or_default(),
                points: points.unwrap_or(items),
                omitted,
                tiles: if omitted { frontier } else { Some(1) },
                capped: true,
                cell: Some(cells.len() - 1),
            }
        };
        if let Some(parent) = stack.last_mut() {
            parent.children.push(handed);
            parent.area += frame.area;
            for k in 0..DEPTHS {
                parent.open[k] += frame.open[k];
                parent.length[k] += frame.length[k];
            }
        }
        Ok(())
    };
    for node in captured.tree.nodes.reader()? {
        let node = node?;
        check()?;
        while stack.last().is_some_and(|f| f.node.end <= node.first) {
            close(&mut stack, &mut cells, leaf)?;
        }
        if let Some(parent) = stack.last_mut() {
            parent.started += 1;
        }
        if node.kind == NodeKind::Internal {
            stack.push(Frame {
                node,
                leaf_begin: leaf,
                started: 0,
                children: Vec::with_capacity(8),
                area: 0.,
                open: [0; DEPTHS],
                length: [0.; DEPTHS],
            });
            continue;
        }
        let facts = leaves
            .get(leaf as usize)
            .ok_or_else(|| state("a leaf without its facts"))?;
        let key = TileKey {
            first: node.first,
            end: node.end,
            depth: node.depth(),
            leaf_begin: leaf,
            leaf_end: leaf + 1,
        };
        let (keys, points) = match borders.contract(key, 6 * cap)? {
            Some(c) => {
                let keys = point_keys(&c);
                let n = keys.len() as u64;
                (keys, n)
            }
            None => (Vec::new(), borders.contract_items(key)?),
        };
        let parent = stack
            .last_mut()
            .ok_or_else(|| state("a leaf without a parent in a tree with internal tiles"))?;
        parent.children.push(Handed {
            coarse: false,
            keys,
            points,
            omitted: false,
            tiles: Some(1),
            capped: node.triangles() <= cap as u64,
            cell: None,
        });
        parent.area += facts.area;
        for k in 0..DEPTHS {
            parent.open[k] += u64::from(facts.open[k]);
            parent.length[k] += f64::from(facts.length[k]);
        }
        leaf = leaf
            .checked_add(1)
            .ok_or_else(|| state("leaf index overflow"))?;
    }
    while !stack.is_empty() {
        close(&mut stack, &mut cells, leaf)?;
    }
    if leaf as usize != leaves.len() {
        return Err(state("the forecast walk did not visit every leaf"));
    }
    Ok(cells)
}

/// The deepest coarse cell holding both DFS leaves `lo` and `hi` (`lo <= hi`), by depth: a
/// loop whose edges lie in those leaves crosses the border of every ancestor cell deeper.
/// `cells` are the internal cells' `(leaf_begin, leaf_end, depth, parent)` in preorder.
pub(crate) fn common_depth(cells: &[(u32, u32, u8, u32)], lo: u32, hi: u32) -> u8 {
    // The last cell beginning at or before `lo`; its ancestor chain holds every cell
    // containing `lo` (preorder intervals nest).
    let mut at = match cells.partition_point(|c| c.0 <= lo) {
        0 => return 0,
        n => n - 1,
    };
    loop {
        let c = cells[at];
        if c.0 <= lo && hi < c.1 {
            return c.2;
        }
        if c.3 == u32::MAX {
            return 0;
        }
        at = c.3 as usize;
    }
}

/// The internal cells of the tree as `(leaf_begin, leaf_end, depth, parent index)` in
/// preorder (`u32::MAX` for the root), for [`common_depth`].
pub(crate) fn cell_intervals(
    nodes: &crate::spill::Sequence<NodeRecord>,
    internal: u64,
) -> Result<Vec<(u32, u32, u8, u32)>, JobError> {
    let mut out: Vec<(u32, u32, u8, u32)> = Vec::new();
    out.try_reserve_exact(usize::try_from(internal).unwrap_or(usize::MAX))
        .map_err(|_| {
            JobError::new(
                JobErrorKind::ResourceLimit,
                "forecast interval allocation failed",
            )
        })?;
    // Open internal cells: (index in `out`, rank end).
    let mut stack: Vec<(usize, u64)> = Vec::new();
    let mut leaf = 0u32;
    for node in nodes.reader()? {
        let node = node?;
        while let Some(&(i, end)) = stack.last() {
            if end > node.first {
                break;
            }
            out[i].1 = leaf;
            stack.pop();
        }
        if node.kind == NodeKind::Internal {
            let parent = stack.last().map_or(u32::MAX, |&(i, _)| i as u32);
            let depth = u8::try_from(node.depth()).map_err(|_| state("cell depth overflow"))?;
            out.push((leaf, leaf, depth, parent));
            stack.push((out.len() - 1, node.end));
        } else {
            leaf = leaf
                .checked_add(1)
                .ok_or_else(|| state("leaf index overflow"))?;
        }
    }
    while let Some((i, _)) = stack.pop() {
        out[i].1 = leaf;
    }
    Ok(out)
}

/// Resident bytes of [`cell_intervals`] for `internal` cells.
pub(crate) fn interval_bytes(internal: u64) -> u64 {
    internal.saturating_mul(std::mem::size_of::<(u32, u32, u8, u32)>() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Preorder nesting: root [0, 4) with children [0, 2) and [2, 4); leaves 0..4.
    #[test]
    fn common_depth_is_the_deepest_cell_holding_both_leaves() {
        let cells = vec![
            (0, 4, 0, u32::MAX),
            (0, 2, 1, 0),
            (2, 4, 1, 0),
            (2, 3, 2, 2),
        ];
        assert_eq!(common_depth(&cells, 0, 1), 1);
        assert_eq!(common_depth(&cells, 0, 3), 0);
        assert_eq!(common_depth(&cells, 2, 2), 2);
        assert_eq!(common_depth(&cells, 2, 3), 1);
        assert_eq!(common_depth(&cells, 3, 3), 1);
    }
}
