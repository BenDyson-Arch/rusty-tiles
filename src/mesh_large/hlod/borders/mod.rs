//! D11 shared level contracts. One disk graph resolves all owners before parent jobs.
//! Source arcs are exact incidence paths, and every level shortcuts those source arcs
//! directly. Live junctions and terminal-leaf contacts are pinned. The same stored segment
//! is handed to every owner, so no tile-local choice can disagree with its neighbor.
mod curve;
mod disk;
mod graph;

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
                    .checked_add(edges.checked_mul(112)?)?
                    .checked_add(rows.checked_mul(16)?)?,
            )?,
        )?;
    let sort = config::sort_spill::<disk::Words<7>>(rows, sorter)?;
    persistent
        .checked_add(sort.finish_peak)?
        .checked_add(sort.stream_slack)?
        .checked_add(edges.checked_mul(72 + 8)?)?
        .checked_add(vertices.checked_mul(24)?)
        .and_then(|v| v.checked_add(1 << 20))
}

pub(super) struct Borders {
    pub(super) source: Source,
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
}

pub(super) struct Contract {
    pub interface: Vec<InterfaceRow>,
    pub certificates: Vec<Certificate>,
    pub scale: f64,
}

impl Borders {
    pub(super) const COMPONENT: &'static str = "s1b-shared-borders";
    pub(super) fn build(
        scratch: &Scratch,
        sorter: SortConfig,
        nodes: &Sequence<NodeRecord>,
        rows: impl Iterator<Item = Result<InterfaceRow, crate::spill::SpillError>>,
        extent: f64,
        lease: &ComponentLease<'_, '_>,
        check: &(dyn Fn() -> Result<(), JobError> + Sync),
    ) -> Result<Self, JobError> {
        let source = Source::build(scratch, sorter, nodes, rows, &|| lease.charge(0), check)?;
        let mut borders = Self {
            source,
            maps: Disk::new(scratch, "border-maps")?,
            segments: Disk::new(scratch, "border-segments")?,
            refs: Disk::new(scratch, "border-contracts")?,
            ladder: Ladder::new(extent)?,
        };
        // Source arcs are traversed one level at a time; their temporary ordered points
        // are discarded after certificates and source-vertex maps have been materialized.
        for depth in 0..=borders.source.depth {
            check()?;
            borders.level(scratch, depth, check)?;
            lease.charge(borders.bytes())?;
        }
        Ok(borders)
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
            if degree == 2 && !terminal {
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
        let mut arc = Disk::<2>::new(scratch, "border-ordered-arc")?;
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
        arc: &mut Disk<2>,
        check: &(dyn Fn() -> Result<(), JobError> + Sync),
    ) -> Result<(), JobError> {
        let begin = arc.len;
        let (mut at, mut edge) = (start, first_edge);
        loop {
            if (arc.len - begin).is_multiple_of(4096) {
                check()?;
            }
            arc.push([at, edge])?;
            active.set(edge, [2])?;
            let e = self.source.edges.get(edge)?;
            at = if e[0] == at { e[1] } else { e[0] };
            let next = live.get(at)?;
            if at == start || next[2] != 2 {
                arc.push([at, NONE])?;
                break;
            }
            edge = if next[0] == edge { next[1] } else { next[0] };
            if active.get(edge)?[0] != 1 {
                return Err(state("source arc revisits an edge before its endpoint"));
            }
        }
        let end = arc.len - 1;
        let e = self.source.edges.get(first_edge)?;
        let epsilon = self.ladder.epsilon(depth);
        let mut pieces = vec![begin, end];
        if at == start {
            let count = end - begin;
            if count < 3 {
                return Err(state(
                    "a shared cycle has fewer than three distinct source edges",
                ));
            }
            pieces = vec![begin, begin + count / 3, begin + 2 * count / 3, end];
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
        arc: &Disk<2>,
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
                let mut previous = NONE;
                for at in owners[0]..owners[1] {
                    let leaf = owner_store.get(at)?[0];
                    let owner = leaves.get(leaf)?[1 + depth as usize];
                    if owner == previous {
                        continue;
                    }
                    let mut node = nodes.get(owner)?;
                    node[6] = refs.push([node[6], id])?;
                    node[7] += 1;
                    nodes.set(owner, node)?;
                    previous = owner;
                }
                Ok(())
            },
        )?;
        Ok(())
    }
    fn emit_segment(&mut self, segment: [u64; 10]) -> Result<(), JobError> {
        let id = self.segments.push(segment)?;
        let mut previous = NONE;
        for at in segment[8]..segment[9] {
            let owner = self.source.owner(at, segment[7] as u32)?;
            if owner == previous {
                continue;
            }
            let mut node = self.source.nodes.get(owner)?;
            node[6] = self.refs.push([node[6], id])?;
            node[7] += 1;
            self.source.nodes.set(owner, node)?;
            previous = owner;
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
        let mut interface = Vec::new();
        let mut certificates = Vec::new();
        while at != NONE {
            let link = self.refs.get(at)?;
            let s = self.segments.get(link[1])?;
            let a = self.source.key(s[0])?;
            let b = self.source.key(s[1])?;
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
                });
            }
            at = link[0];
        }
        super::interfaces::merge_rows(&mut interface);
        certificates.sort_by_key(|c| (c.endpoints, c.chain, c.first));
        Ok(Some(Contract {
            interface,
            certificates,
            scale: self.scale(key.depth),
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
        for leaf in 0..4 {
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
            rows.into_iter().map(Ok),
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
        ledger.check().unwrap();
    }
}
