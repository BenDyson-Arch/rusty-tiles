//! The exact source graph and live ownership. Vertex/edge owner lists are complete disk
//! runs. Owner equality is a streamed comparison of coarsened runs, never a hash or a
//! Cartesian owner-pair join. Adjacency is a disk linked list with two links per edge.
use super::{
    curve::state,
    disk::{Disk, Words, NONE},
};
use crate::{
    mesh_large::{
        hlod::{
            interfaces::{InterfaceRow, Key},
            store::TileKey,
        },
        records::{NodeKind, NodeRecord},
    },
    spill::{ExternalSorter, Scratch, Sequence, SequenceWriter, SortConfig},
    JobError,
};

pub(super) struct Source {
    /// key x/y/z, owner first/end, incident head/count, minimum terminal leaf depth.
    pub(super) vertices: Disk<8>,
    /// endpoints, owner first/end, next at a/b, minimum terminal leaf depth.
    pub(super) edges: Disk<7>,
    pub(super) owners: Disk<1>,
    /// first/end source rank, depth, first/end leaf, kind, contract head/count.
    pub(super) nodes: Disk<8>,
    /// terminal depth, ancestor node IDs for every logical depth (leaf repeated below it).
    pub(super) leaves: Disk<33>,
    pub(super) depth: u32,
    pub(super) leaf_rows: Sequence<InterfaceRow>,
}

fn build_tree(
    scratch: &Scratch,
    tree: &Sequence<NodeRecord>,
) -> Result<(Disk<8>, Disk<33>, u32), JobError> {
    let mut nodes = Disk::new(scratch, "border-nodes")?;
    let mut leaves = Disk::new(scratch, "border-leaves")?;
    let mut ancestors = [0u64; 32];
    let mut stack: Vec<(u64, NodeRecord)> = Vec::new();
    let mut depth = 0;
    let close = |id, nodes: &mut Disk<8>, end| -> Result<(), JobError> {
        let mut n = nodes.get(id)?;
        n[4] = end;
        nodes.set(id, n)
    };
    for node in tree.reader()? {
        let n = node?;
        while stack.last().is_some_and(|(_, p)| p.depth() >= n.depth()) {
            close(stack.pop().unwrap().0, &mut nodes, leaves.len)?;
        }
        let d = n.depth() as usize;
        if d >= 32 {
            return Err(state("source tree exceeds shared-border height"));
        }
        depth = depth.max(d as u32);
        let id = nodes.push([
            n.first,
            n.end,
            d as u64,
            leaves.len,
            leaves.len,
            u64::from(n.kind == NodeKind::Leaf),
            NONE,
            0,
        ])?;
        ancestors[d..].fill(id);
        if n.kind == NodeKind::Leaf {
            let mut row = [0; 33];
            row[0] = d as u64;
            row[1..].copy_from_slice(&ancestors);
            leaves.push(row)?;
            close(id, &mut nodes, leaves.len)?;
        } else {
            stack.push((id, n));
        }
    }
    while let Some((id, _)) = stack.pop() {
        close(id, &mut nodes, leaves.len)?;
    }
    Ok((nodes, leaves, depth))
}

impl Source {
    /// `admitted` is the component lease's byte check. The caller admits the entire graph
    /// bound before entering; the check is repeated after materialized phases.
    pub(super) fn build(
        scratch: &Scratch,
        sorter: SortConfig,
        tree: &Sequence<NodeRecord>,
        rows: impl Iterator<Item = Result<InterfaceRow, crate::spill::SpillError>>,
        open_ends: impl Iterator<
            Item = Result<super::super::interfaces::KeyRecord, crate::spill::SpillError>,
        >,
        admitted: &dyn Fn() -> Result<(), JobError>,
        check: &(dyn Fn() -> Result<(), JobError> + Sync),
    ) -> Result<Self, JobError> {
        let (nodes, leaves, depth) = build_tree(scratch, tree)?;
        let mut leaf_rows = SequenceWriter::new(scratch, "border-leaf-rows", sorter.buffer_bytes)?;
        let mut sorted = ExternalSorter::<Words<7>>::new(scratch, sorter)?;
        for (i, row) in rows.enumerate() {
            if i.is_multiple_of(4096) {
                check()?;
            }
            let r = row?;
            leaf_rows.push(&r)?;
            sorted.push(Words([
                u64::from(r.a[0]),
                u64::from(r.a[1]),
                u64::from(r.a[2]),
                u64::from(r.b[0]),
                u64::from(r.b[1]),
                u64::from(r.b[2]),
                u64::from(r.leaf),
            ]))?;
        }
        let leaf_rows = leaf_rows.finish()?;
        let mut vertices = Disk::new(scratch, "border-vertices")?;
        let mut raw_edges = Disk::<9>::new(scratch, "border-edge-keys")?;
        let mut owners = Disk::new(scratch, "border-owners")?;
        // One group's summary, regardless of the number of incident leaves.
        let mut item: Option<[u64; 6]> = None;
        let mut begin = 0;
        let mut terminal = 31;
        let flush = |k: [u64; 6],
                     begin,
                     end,
                     terminal,
                     vertices: &mut Disk<8>,
                     raw: &mut Disk<9>|
         -> Result<(), JobError> {
            if k[..3] == k[3..] {
                vertices.push([k[0], k[1], k[2], begin, end, NONE, 0, terminal])?;
            } else {
                raw.push([k[0], k[1], k[2], k[3], k[4], k[5], begin, end, terminal])?;
            }
            Ok(())
        };
        for row in sorted.finish()? {
            let r = row?.0;
            let k: [u64; 6] = r[..6].try_into().unwrap();
            if item.is_some_and(|old| old != k) {
                flush(
                    item.unwrap(),
                    begin,
                    owners.len,
                    terminal,
                    &mut vertices,
                    &mut raw_edges,
                )?;
                begin = owners.len;
                terminal = 31;
            }
            terminal = terminal.min(leaves.get(r[6])?[0]);
            owners.push([r[6]])?;
            item = Some(k);
        }
        if let Some(k) = item {
            flush(
                k,
                begin,
                owners.len,
                terminal,
                &mut vertices,
                &mut raw_edges,
            )?;
        }
        admitted()?;
        let mut source = Self {
            vertices,
            edges: Disk::new(scratch, "border-edges")?,
            owners,
            nodes,
            leaves,
            depth,
            leaf_rows,
        };
        for i in 0..raw_edges.len {
            if i.is_multiple_of(4096) {
                check()?;
            }
            let e = raw_edges.get(i)?;
            let a = source
                .vertex([e[0] as u32, e[1] as u32, e[2] as u32])?
                .ok_or_else(|| state("shared edge has no first source vertex"))?;
            let b = source
                .vertex([e[3] as u32, e[4] as u32, e[5] as u32])?
                .ok_or_else(|| state("shared edge has no second source vertex"))?;
            let mut va = source.vertices.get(a)?;
            let mut vb = source.vertices.get(b)?;
            let id = source.edges.push([a, b, e[6], e[7], va[5], vb[5], e[8]])?;
            va[5] = id;
            va[6] += 1;
            vb[5] = id;
            vb[6] += 1;
            source.vertices.set(a, va)?;
            source.vertices.set(b, vb)?;
        }
        drop(raw_edges);
        if source.vertices.len > 0 {
            for (i, p) in open_ends.enumerate() {
                if i.is_multiple_of(4096) {
                    check()?;
                }
                if let Some(id) = source.vertex(p?.0)? {
                    let mut v = source.vertices.get(id)?;
                    v[7] |= 1 << 32;
                    source.vertices.set(id, v)?;
                }
            }
        }
        admitted()?;
        Ok(source)
    }

    pub(super) fn vertex(&self, key: Key) -> Result<Option<u64>, JobError> {
        let (mut lo, mut hi) = (0, self.vertices.len);
        let want = key.map(u64::from);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let v = self.vertices.get(mid)?;
            match v[..3].cmp(&want) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Ok(Some(mid)),
            }
        }
        Ok(None)
    }
    pub(super) fn key(&self, id: u64) -> Result<Key, JobError> {
        let v = self.vertices.get(id)?;
        Ok([v[0] as u32, v[1] as u32, v[2] as u32])
    }
    pub(super) fn point(&self, id: u64) -> Result<[f32; 3], JobError> {
        Ok(self.key(id)?.map(f32::from_bits))
    }
    pub(super) fn node(&self, tile: TileKey) -> Result<u64, JobError> {
        let (mut lo, mut hi) = (0, self.nodes.len);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let n = self.nodes.get(mid)?;
            match (n[0], n[2]).cmp(&(tile.first, u64::from(tile.depth))) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal if n[1] == tile.end => return Ok(mid),
                _ => return Err(state("shared-border node differs from the frozen tree")),
            }
        }
        Err(state("shared-border node is absent from the frozen tree"))
    }
    pub(super) fn owner(&self, at: u64, depth: u32) -> Result<u64, JobError> {
        let leaf = self.owners.get(at)?[0];
        Ok(self.leaves.get(leaf)?[1 + depth as usize])
    }
    pub(super) fn external(&self, begin: u64, end: u64, depth: u32) -> Result<bool, JobError> {
        Ok(self.owner(begin, depth)? != self.owner(end - 1, depth)?)
    }
    pub(super) fn owner_list(
        &self,
        begin: u64,
        end: u64,
        depth: u32,
        emit: &mut impl FnMut(u64) -> Result<(), JobError>,
    ) -> Result<(), JobError> {
        let mut last = NONE;
        for at in begin..end {
            let owner = self.owner(at, depth)?;
            if owner != last {
                emit(owner)?;
                last = owner;
            }
        }
        Ok(())
    }
    pub(super) fn same_owners(
        &self,
        a: [u64; 2],
        b: [u64; 2],
        depth: u32,
    ) -> Result<bool, JobError> {
        let (mut i, mut j) = (a[0], b[0]);
        while i < a[1] && j < b[1] {
            let owner = self.owner(i, depth)?;
            if owner != self.owner(j, depth)? {
                return Ok(false);
            }
            while i < a[1] && self.owner(i, depth)? == owner {
                i += 1;
            }
            while j < b[1] && self.owner(j, depth)? == owner {
                j += 1;
            }
        }
        Ok(i == a[1] && j == b[1])
    }
}
