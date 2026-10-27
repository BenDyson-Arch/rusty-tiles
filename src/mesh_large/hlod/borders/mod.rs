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
        + 12 * disk::CACHE_BYTES
        // Chain reuse compares two live contracts before the geometry stage.
        + 2 * contract_workspace_bytes(6 * super::TRIANGLE_CAP as u64)
}

pub(super) const WALK_CACHE_BYTES: u64 = 8 * disk::CACHE_BYTES;

use self::{
    curve::{state, Ladder},
    disk::{Disk, NONE},
    graph::Source,
};
use super::{
    interfaces::{InterfaceRow, Key},
    scratch::ComponentLease,
    store::TileKey,
};
use crate::{
    mesh_large::{config, records::NodeRecord},
    spill::{Scratch, Sequence, SortConfig},
    JobError,
};

/// Persistent bound for graph, ownership, source arcs, level correspondence and contracts.
/// Counts are source interface counts, not an assumed manifold/valence bound. The source
/// item sort is a separate transient peak. Height is bounded by the admitted tree (<=32).
pub(super) fn scratch_need(
    rows: u64,
    vertices: u64,
    edges: u64,
    nodes: u64,
    leaves: u64,
    height: u32,
    sorter: SortConfig,
) -> Option<u64> {
    let h = u64::from(height) + 1;
    let persistent = rows
        .checked_mul(8 + 36)?
        .checked_add(vertices.checked_mul(64)?)?
        .checked_add(edges.checked_mul(56)?)?
        .checked_add(nodes.checked_mul(64)?)?
        .checked_add(leaves.checked_mul(264)?)?
        .checked_add(
            h.checked_mul(
                vertices
                    .checked_mul(104)?
                    .checked_add(edges.checked_mul(80)?)?
                    .checked_add(rows.checked_mul(16)?)?,
            )?,
        )?;
    let sort = config::sort_spill::<disk::Words<7>>(rows, sorter)?;
    persistent
        .checked_add(sort.finish_peak)?
        .checked_add(sort.stream_slack)?
        .checked_add(edges.checked_mul(72 + 8 + 96)?)?
        .checked_add(vertices.checked_mul(24)?)
        .and_then(|v| v.checked_add(1 << 20))
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
}

pub(super) struct Borders {
    source: Source,
    /// Per (logical depth, source vertex): retained endpoint pair and source parameter.
    maps: Disk<3>,
    /// endpoints, chain source edge, origin, first/end source ordinal, epsilon bits, depth,
    /// the complete source-owner run whose depth-coarsened owners consume this segment.
    segments: Disk<10>,
    /// Per node: linked references to common segments (head/count stored in Source::nodes).
    refs: Disk<2>,
    ladder: Ladder,
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
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Contract {
    pub depth: u32,
    pub interface: Vec<InterfaceRow>,
    pub certificates: Vec<Certificate>,
    /// Exact vertex contacts with a terminal leaf, including contacts with no shared edge.
    pub terminal_vertices: Vec<Key>,
    pub scale: f64,
}

/// Peak storage of contract construction for at most `limit` linked items. Each
/// item emits at most one certificate, two terminal keys and three interface rows.
/// `merge_rows` briefly holds both row vectors. Its retired row copy also covers
/// stable certificate sorting's scratch (one certificate is smaller than three
/// rows). Exact reservations below prevent growth transients. This allowance stays live
/// through geometry, S2 and audit publication, independently of geometry workspace.
pub(super) fn contract_workspace_bytes(limit: u64) -> u64 {
    limit.saturating_mul(
        (std::mem::size_of::<Certificate>()
            + 2 * std::mem::size_of::<Key>()
            + 6 * std::mem::size_of::<InterfaceRow>()) as u64,
    )
}

impl Contract {
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
    pub(super) fn build(
        scratch: &Scratch,
        sorter: SortConfig,
        nodes: &Sequence<NodeRecord>,
        rows: impl Iterator<Item = Result<InterfaceRow, crate::spill::SpillError>>,
        open_ends: impl Iterator<Item = Result<super::interfaces::KeyRecord, crate::spill::SpillError>>,
        extent: f64,
        lease: &ComponentLease<'_, '_>,
        check: &(dyn Fn() -> Result<(), JobError> + Sync),
    ) -> Result<Self, JobError> {
        let source = Source::build(
            scratch,
            sorter,
            nodes,
            rows,
            open_ends,
            &|| lease.charge(0),
            check,
        )?;
        let mut borders = Self {
            source,
            maps: Disk::new(scratch, "border-maps")?,
            segments: Disk::new(scratch, "border-segments")?,
            refs: Disk::new(scratch, "border-contracts")?,
            ladder: Ladder::new(extent)?,
        };
        // Source arcs are traversed one level at a time; their temporary ordered points
        // are discarded after certificates and source-vertex maps have been materialized.
        for depth in (0..=borders.source.depth).rev() {
            check()?;
            borders.level(scratch, depth, check)?;
            lease.charge(borders.bytes())?;
        }
        let mut ancestors = [(0u64, 0u64, 0u32); 32];
        for id in 0..borders.source.nodes.len {
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
        Ok(borders)
    }
    pub(super) fn audit_bound(&self, source_triangle_depths: u64) -> Option<u64> {
        super::audit::bound(self.refs.len, self.source.nodes.len, source_triangle_depths)
    }
    fn bytes(&self) -> u64 {
        self.source.vertices.len * 64
            + self.source.edges.len * 56
            + self.source.owners.len * 8
            + self.source.nodes.len * 64
            + self.source.leaves.len * 264
            + self.source.leaf_rows.len() * 36
            + self.maps.len * 24
            + self.segments.len * 80
            + self.refs.len * 16
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
        let Some(a) = self.contract(key(depth), 6 * cap)? else {
            return Ok(false);
        };
        let Some(b) = self.contract(key(depth + 1), 6 * cap)? else {
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

    fn level(
        &mut self,
        scratch: &Scratch,
        depth: u32,
        check: &(dyn Fn() -> Result<(), JobError> + Sync),
    ) -> Result<(), JobError> {
        let mut active = Disk::<1>::new(scratch, "border-live-edges")?;
        for i in 0..self.source.edges.len {
            let e = self.source.edges.get(i)?;
            active.push([u64::from(self.source.external(e[2], e[3], depth)?)])?;
        }
        // State 0: internal; 1: live junction/point; 2: removable degree-two source vertex.
        let mut live = Disk::<3>::new(scratch, "border-live-vertices")?;
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
            if degree == 2 && !terminal && vertex[7] >> 32 == 0 {
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
            if external && degree == 0 {
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
                    self.chain(v, edge, depth, &live, &mut active, &mut arc, check)?;
                }
                edge = if e[0] == v { e[4] } else { e[5] };
            }
        }
        // Remaining edges are cycles. Source edge order chooses the least edge and its
        // least endpoint as the canonical origin/direction, independent of sort buffers.
        for edge in 0..self.source.edges.len {
            if active.get(edge)?[0] == 1 {
                let e = self.source.edges.get(edge)?;
                self.chain(e[0], edge, depth, &live, &mut active, &mut arc, check)?;
            }
        }
        Ok(())
    }

    fn chain(
        &mut self,
        start: u64,
        first_edge: u64,
        depth: u32,
        live: &Disk<3>,
        active: &mut Disk<1>,
        arc: &mut Disk<3>,
        check: &(dyn Fn() -> Result<(), JobError> + Sync),
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
        let epsilon = self.ladder.epsilon(depth);
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
        for pair in pieces.windows(2) {
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
            )?;
        }
        Ok(())
    }

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
    fn emit_segment(&mut self, segment: [u64; 10]) -> Result<(), JobError> {
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

    pub(super) fn contract(
        &self,
        key: TileKey,
        limit: usize,
    ) -> Result<Option<Contract>, JobError> {
        let n = self.source.nodes.get(self.source.node(key)?)?;
        if n[7] > limit as u64 {
            return Ok(None);
        }
        let mut at = n[6];
        let count = n[7] as usize;
        let mut interface = Vec::with_capacity(3 * count);
        let mut certificates = Vec::with_capacity(count);
        let mut terminal_vertices = Vec::with_capacity(2 * count);
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
                });
            }
            at = link[0];
        }
        super::interfaces::merge_rows(&mut interface);
        certificates.sort_by_key(|c| (c.endpoints, c.chain, c.first));
        terminal_vertices.sort_unstable();
        terminal_vertices.dedup();
        Ok(Some(Contract {
            depth: key.depth,
            interface,
            certificates,
            terminal_vertices,
            scale: self.scale(self.publication_depth(key)?),
        }))
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
    fn large_source_scratch_admission_is_checked_and_incidence_bounded() {
        let sorter = SortConfig {
            run_bytes: 1 << 20,
            fan_in: 8,
            buffer_bytes: 1 << 16,
            jobs: 1,
        };
        for n in [500_000_000u64, 1_000_000_000] {
            // The production caller's worst-case interface counts, with a full-height
            // tree. The envelope covers all persistent levels and the external sort.
            let need =
                scratch_need(6 * n, 3 * n, 3 * n, 2 * n / 8192, n / 8192, 31, sorter).unwrap();
            assert!((20_000 * n..=32_768 * n).contains(&need));
        }
        for field in 0..5 {
            let mut counts = [1; 5];
            counts[field] = u64::MAX;
            assert!(scratch_need(
                counts[0], counts[1], counts[2], counts[3], counts[4], 31, sorter,
            )
            .is_none());
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
            let need = scratch_need(row_count, r + 1, r, 4 * r - 1, 2 * r, height, sorter).unwrap();
            let lease = ledger.admit(Borders::COMPONENT, need).unwrap();
            let borders = Borders::build(
                &scratch,
                sorter,
                &tree,
                rows.into_iter().map(Ok),
                std::iter::empty(),
                4096.,
                &lease,
                &|| Ok(()),
            )
            .unwrap();
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
            scale: 1.,
        };
        assert_eq!(c.minimum_triangles(), 8193);
        assert!(c.minimum_triangles() > super::super::TRIANGLE_CAP);
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
        let lease = ledger.admit(Borders::COMPONENT, 8 << 20).unwrap();
        let borders = Borders::build(
            &scratch,
            sorter,
            &tree,
            rows.iter().copied().map(Ok),
            std::iter::empty(),
            4096.,
            &lease,
            &|| Ok(()),
        )
        .unwrap();
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
        drop(borders);
        // A nonshared source boundary meeting point 2 can complete a hole through
        // the straight tail. Its separate open chain retains an interior witness.
        let pinned = Borders::build(
            &scratch,
            sorter,
            &tree,
            rows.into_iter().map(Ok),
            [Ok(super::super::interfaces::KeyRecord(points[2]))].into_iter(),
            4096.,
            &lease,
            &|| Ok(()),
        )
        .unwrap();
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
}
