//! Bounded source-surface index shared by S2 measurement and S3 appearance.
//! Source ranks refer to captured (pre-leaf-quantization) triangles. Queries are clipped
//! to the tile's descendant range, with distance then source-rank ties; no facing or
//! material restrictions. Disk sequences use capture's scratch owner. Each query owns
//! a fixed cache; no allocation scales with the source or with the tile's source range.
use super::{
    resource, state,
    store::{SourcePrimitive, TileKey},
};
use crate::{
    mesh_large::{
        attributes::FullTriangle,
        full::FullCaptured,
        records::{Bounds, Provenance},
    },
    spill::{RandomRecords, Record, Sequence, SequenceWriter},
    JobError,
};
use rustc_hash::FxHashMap as HashMap;
use std::collections::VecDeque;

pub(crate) const BLOCK: usize = 256;
const CACHE_BLOCKS: usize = 64;
const CACHE_NODES: usize = 16384;
/// Includes both caches, compact and full-row I/O, build workspace, stacks and slack.
pub(crate) const QUERY_BYTES: u64 = 8 << 20;
pub(crate) const BUILD_BYTES: u64 = 2 << 20;
/// Compact triangles plus <= one fine node per two triangles, upper nodes and slack.
pub(crate) fn scratch_need(n: u64) -> Option<u64> {
    n.checked_mul(160)?.checked_add(1 << 20)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Triangle {
    pub bits: [[u32; 3]; 3],
    pub rank: u64,
    pub order: (Provenance, u64),
    pub material: u32,
    pub controls: u32,
    area_bits: u64,
}
impl Triangle {
    pub(crate) fn new(p: [[f32; 3]; 3], rank: u64) -> Self {
        let q = p.map(|p| p.map(f64::from));
        let n = cross(sub(q[1], q[0]), sub(q[2], q[0]));
        Self {
            material: u32::MAX,
            controls: 0,
            area_bits: (0.5 * dot(n, n).sqrt()).to_bits(),
            bits: p.map(|p| p.map(f32::to_bits)),
            rank,
            order: (
                Provenance {
                    triangle: rank,
                    ..Default::default()
                },
                0,
            ),
        }
    }
    pub(crate) fn positions(self) -> [V3; 3] {
        self.bits.map(|p| p.map(|x| f64::from(f32::from_bits(x))))
    }
    pub(crate) fn area(self) -> f64 {
        f64::from_bits(self.area_bits)
    }
}
/// Source controls for one bounded patch: its boundary vertices, one vertex per
/// edge-connected component, and all sliver vertices. Patch boundaries are a
/// conservative superset of true source boundaries; no global connectivity mesh.
pub(crate) fn mark_controls(triangles: &mut [Triangle]) {
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..triangles.len()).collect();
    let mut edges = HashMap::<([u32; 3], [u32; 3]), (usize, usize, usize)>::default();
    for (i, t) in triangles.iter_mut().enumerate() {
        t.controls = 0;
        let p = t.positions();
        let max_edge = (0..3)
            .map(|k| distance2(p[k], p[(k + 1) % 3]))
            .fold(0., f64::max);
        if t.area() <= max_edge * 1e-4 {
            t.controls = 7;
        }
        for k in 0..3 {
            let (a, b) = (t.bits[k], t.bits[(k + 1) % 3]);
            let entry = edges.entry((a.min(b), a.max(b))).or_insert((i, k, 0));
            entry.2 += 1;
            let a = root(&mut parent, i);
            let b = root(&mut parent, entry.0);
            parent[a.max(b)] = a.min(b);
        }
    }
    for &(i, k, count) in edges.values() {
        if count == 1 {
            triangles[i].controls |= (1 << k) | (1 << ((k + 1) % 3));
        }
    }
    for (i, t) in triangles.iter_mut().enumerate() {
        if root(&mut parent, i) == i {
            t.controls |= 1;
        }
    }
}

impl Record for Triangle {
    const SIZE: usize = 88;
    fn encode(&self, b: &mut [u8]) {
        for (v, o) in self
            .bits
            .iter()
            .flatten()
            .zip(b[..36].as_chunks_mut::<4>().0.iter_mut())
        {
            o.copy_from_slice(&v.to_le_bytes());
        }
        b[36..44].copy_from_slice(&self.rank.to_le_bytes());
        for (x, b) in [self.order.0.node, self.order.0.mesh, self.order.0.primitive]
            .iter()
            .zip(b[44..56].as_chunks_mut::<4>().0.iter_mut())
        {
            b.copy_from_slice(&x.to_le_bytes());
        }
        b[56..64].copy_from_slice(&self.order.0.triangle.to_le_bytes());
        b[64..72].copy_from_slice(&self.order.1.to_le_bytes());
        b[72..76].copy_from_slice(&self.material.to_le_bytes());
        b[76..80].copy_from_slice(&self.controls.to_le_bytes());
        b[80..88].copy_from_slice(&self.area_bits.to_le_bytes());
    }
    fn decode(b: &[u8]) -> Self {
        Self {
            material: u32::from_le_bytes(b[72..76].try_into().unwrap()),
            controls: u32::from_le_bytes(b[76..80].try_into().unwrap()),
            area_bits: u64::from_le_bytes(b[80..88].try_into().unwrap()),
            bits: std::array::from_fn(|i| {
                std::array::from_fn(|j| {
                    u32::from_le_bytes(b[(i * 3 + j) * 4..(i * 3 + j + 1) * 4].try_into().unwrap())
                })
            }),
            rank: u64::from_le_bytes(b[36..44].try_into().unwrap()),
            order: (
                Provenance {
                    node: u32::from_le_bytes(b[44..48].try_into().unwrap()),
                    mesh: u32::from_le_bytes(b[48..52].try_into().unwrap()),
                    primitive: u32::from_le_bytes(b[52..56].try_into().unwrap()),
                    triangle: u64::from_le_bytes(b[56..64].try_into().unwrap()),
                },
                u64::from_le_bytes(b[64..72].try_into().unwrap()),
            ),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Node {
    lo: [u32; 3],
    hi: [u32; 3],
    first: u64,
    end: u64,
    left: u64,
    right: u64,
    start: u32,
    count: u32,
    area_bits: u64,
}
impl Node {
    fn area(&self) -> f64 {
        f64::from_bits(self.area_bits)
    }
    fn bounds(&self) -> Bounds {
        Bounds {
            min: self.lo.map(f32::from_bits),
            max: self.hi.map(f32::from_bits),
        }
    }
    fn distance(&self, p: V3) -> f64 {
        let b = self.bounds();
        (0..3)
            .map(|i| {
                let d = (f64::from(b.min[i]) - p[i])
                    .max(p[i] - f64::from(b.max[i]))
                    .max(0.);
                d * d
            })
            .sum()
    }
    fn intersects(&self, k: TileKey) -> bool {
        self.first < k.end && k.first < self.end
    }
}
impl Record for Node {
    const SIZE: usize = 72;
    fn encode(&self, b: &mut [u8]) {
        for (v, o) in self
            .lo
            .iter()
            .chain(&self.hi)
            .zip(b[..24].as_chunks_mut::<4>().0.iter_mut())
        {
            o.copy_from_slice(&v.to_le_bytes());
        }
        for (v, o) in [self.first, self.end, self.left, self.right]
            .iter()
            .zip(b[24..56].as_chunks_mut::<8>().0.iter_mut())
        {
            o.copy_from_slice(&v.to_le_bytes());
        }
        b[56..60].copy_from_slice(&self.start.to_le_bytes());
        b[60..64].copy_from_slice(&self.count.to_le_bytes());
        b[64..72].copy_from_slice(&self.area_bits.to_le_bytes());
    }
    fn decode(b: &[u8]) -> Self {
        let u32at = |i| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let u64at = |i| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        Self {
            lo: std::array::from_fn(|i| u32at(i * 4)),
            hi: std::array::from_fn(|i| u32at(12 + i * 4)),
            first: u64at(24),
            end: u64at(32),
            left: u64at(40),
            right: u64at(48),
            start: u32at(56),
            count: u32at(60),
            area_bits: u64at(64),
        }
    }
}

/// Resident BVH, bounded to one source block or one <=8192-face decoded coarse tile.
/// Ordinary f64 nearest-triangle arithmetic, reused from the former baker's geometric
/// witness math; this is sampling, with no interval/certification machinery.
pub(crate) struct Surface {
    pub triangles: Vec<Triangle>,
    nodes: Vec<Node>,
}
impl Surface {
    pub(crate) fn new(mut triangles: Vec<Triangle>) -> Result<Self, JobError> {
        if triangles.is_empty() {
            return Err(state("empty sampled surface"));
        }
        let mut nodes = Vec::with_capacity(triangles.len().div_ceil(4) * 2 + 1);
        fn build(t: &mut [Triangle], base: usize, nodes: &mut Vec<Node>) -> usize {
            let mut b = Bounds::EMPTY;
            let (mut first, mut end) = (u64::MAX, 0);
            for t in t.iter() {
                for p in t.bits {
                    b.include(p.map(f32::from_bits));
                }
                first = first.min(t.rank);
                end = end.max(t.rank + 1);
            }
            let at = nodes.len();
            nodes.push(Node {
                lo: b.min.map(f32::to_bits),
                hi: b.max.map(f32::to_bits),
                first,
                end,
                left: 0,
                right: 0,
                start: base as u32,
                count: t.len() as u32,
                area_bits: t.iter().map(|t| t.area()).sum::<f64>().to_bits(),
            });
            if t.len() > 4 {
                let ext = b.extent();
                let axis = (0..3).max_by(|&a, &b| ext[a].total_cmp(&ext[b])).unwrap();
                let mid = t.len() / 2;
                let centre = |t: &Triangle| t.positions().iter().map(|p| p[axis]).sum::<f64>();
                t.select_nth_unstable_by(mid, |a, b| {
                    centre(a).total_cmp(&centre(b)).then(a.rank.cmp(&b.rank))
                });
                let (a, b) = t.split_at_mut(mid);
                let left = build(a, base, nodes);
                let right = build(b, base + mid, nodes);
                nodes[at].left = left as u64;
                nodes[at].right = right as u64;
                nodes[at].count = 0;
            }
            at
        }
        build(&mut triangles, 0, &mut nodes);
        Ok(Self { triangles, nodes })
    }
    /// Reuse the previous witness only as an initial upper distance. The exact
    /// nearest search/tie rule still runs; spatially coherent samples prune earlier.
    pub(crate) fn charge(&self) -> usize {
        self.triangles.capacity() * std::mem::size_of::<Triangle>()
            + self.nodes.capacity() * std::mem::size_of::<Node>()
    }
    pub(crate) fn nearest_hint(&self, p: V3, key: TileKey, hint: &mut Option<usize>) -> Hit {
        let mut hit = hint
            .and_then(|i| {
                self.triangles
                    .get(i)
                    .filter(|t| t.rank >= key.first && t.rank < key.end)
                    .map(|&t| make_hit(p, t, i))
            })
            .unwrap_or_else(Hit::empty);
        self.nearest(p, key, &mut hit);
        *hint = Some(hit.index);
        hit
    }
    /// Budgeted best-first [`Surface::nearest`]: nodes in order of box distance, stopping
    /// when the nearest unexplored box is farther than `best` (then exact: the same winner
    /// and tie rule) or when `budget` runs out (then the best candidate so far). Continues
    /// `best`; true when it changed (`best.index` then names a triangle of this surface).
    pub(crate) fn nearest_budget(
        &self,
        p: V3,
        key: TileKey,
        best: &mut Hit,
        budget: &mut Budget,
        frontier: &mut Frontier,
    ) -> bool {
        use std::cmp::Reverse;
        let mut changed = false;
        frontier.clear();
        frontier.push(Reverse((self.nodes[0].distance(p).to_bits(), 0)));
        while let Some(Reverse((d, i))) = frontier.pop() {
            if f64::from_bits(d) > best.distance2 || !budget.spend() {
                break;
            }
            let n = &self.nodes[i as usize];
            if !n.intersects(key) {
                continue;
            }
            if n.count > 0 {
                for (offset, t) in self.triangles[n.start as usize..(n.start + n.count) as usize]
                    .iter()
                    .enumerate()
                {
                    if t.rank < key.first || t.rank >= key.end {
                        continue;
                    }
                    if !budget.spend() {
                        return changed;
                    }
                    let points = t.positions();
                    let weights = closest(p, points);
                    let d = distance2(p, bary(points, weights));
                    if d < best.distance2
                        || (d == best.distance2 && (t.order, t.rank) < (best.order, best.rank))
                    {
                        *best = Hit {
                            rank: t.rank,
                            index: n.start as usize + offset,
                            order: t.order,
                            bary: weights,
                            distance2: d,
                            material: t.material,
                        };
                        changed = true;
                    }
                }
            } else {
                for c in [n.left, n.right] {
                    let child = &self.nodes[c as usize];
                    if child.intersects(key) {
                        frontier.push(Reverse((child.distance(p).to_bits(), c)));
                    }
                }
            }
        }
        changed
    }
    /// [`Surface::nearest_budget`] seeded by the previous winner `hint` (one step).
    pub(crate) fn nearest_budget_hint(
        &self,
        p: V3,
        key: TileKey,
        hint: &mut Option<usize>,
        budget: &mut Budget,
        frontier: &mut Frontier,
    ) -> Hit {
        let mut best = hint
            .and_then(|i| {
                self.triangles
                    .get(i)
                    .filter(|t| t.rank >= key.first && t.rank < key.end)
                    .map(|&t| make_hit(p, t, i))
            })
            .unwrap_or_else(Hit::empty);
        if best.distance2.is_finite() {
            budget.spend();
        }
        self.nearest_budget(p, key, &mut best, budget, frontier);
        if best.distance2.is_finite() {
            *hint = Some(best.index);
        }
        best
    }
    pub(crate) fn nearest(&self, p: V3, key: TileKey, best: &mut Hit) {
        let mut stack = [0usize; 64];
        let mut len = 1;
        while len > 0 {
            len -= 1;
            let n = &self.nodes[stack[len]];
            if !n.intersects(key) || n.distance(p) > best.distance2 {
                continue;
            }
            if n.count > 0 {
                for (offset, t) in self.triangles[n.start as usize..(n.start + n.count) as usize]
                    .iter()
                    .enumerate()
                {
                    if t.rank < key.first || t.rank >= key.end {
                        continue;
                    }
                    let points = t.positions();
                    let weights = closest(p, points);
                    let d = distance2(p, bary(points, weights));
                    if d < best.distance2
                        || (d == best.distance2 && (t.order, t.rank) < (best.order, best.rank))
                    {
                        *best = Hit {
                            rank: t.rank,
                            index: n.start as usize + offset,
                            order: t.order,
                            bary: weights,
                            distance2: d,
                            material: t.material,
                        };
                    }
                }
            } else {
                let (a, b) = (n.left as usize, n.right as usize);
                let (near, far) = if self.nodes[a].distance(p) <= self.nodes[b].distance(p) {
                    (a, b)
                } else {
                    (b, a)
                };
                stack[len] = far;
                stack[len + 1] = near;
                len += 2;
            }
        }
    }
}

/// Work budget of one budgeted nearest search (the bake's donor search): one step per
/// node visit and per triangle test. Exhaustion returns the best candidate found so far.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Budget {
    left: u64,
    pub steps: u64,
    pub exhausted: bool,
}
impl Budget {
    pub(crate) fn new(steps: u64) -> Self {
        Self {
            left: steps,
            steps: 0,
            exhausted: false,
        }
    }
    fn spend(&mut self) -> bool {
        if self.left == 0 {
            self.exhausted = true;
            return false;
        }
        self.left -= 1;
        self.steps += 1;
        true
    }
}

/// Best-first frontier: (squared box distance bits, node or entry index); ties in index
/// order, so the search order is a pure function of the query point.
pub(crate) type Frontier = std::collections::BinaryHeap<std::cmp::Reverse<(u64, u64)>>;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub rank: u64,
    index: usize,
    pub bary: V3,
    pub order: (Provenance, u64),
    pub distance2: f64,
    material: u32,
}
impl Hit {
    pub(crate) fn empty() -> Self {
        Self {
            rank: u64::MAX,
            index: usize::MAX,
            order: (
                Provenance {
                    node: u32::MAX,
                    mesh: u32::MAX,
                    primitive: u32::MAX,
                    triangle: u64::MAX,
                },
                u64::MAX,
            ),
            bary: [0.; 3],
            distance2: f64::INFINITY,
            material: u32::MAX,
        }
    }
}
fn make_hit(p: V3, t: Triangle, index: usize) -> Hit {
    let positions = t.positions();
    let weights = closest(p, positions);
    Hit {
        rank: t.rank,
        index,
        order: t.order,
        bary: weights,
        distance2: distance2(p, bary(positions, weights)),
        material: t.material,
    }
}
/// Immutable disk index. Construction streams rank-ordered blocks, holding O(log N)
/// upper nodes and one 256-triangle fine BVH. Readers/caches belong to each admitted job.
pub(crate) struct SourceIndex {
    nodes: Sequence<Node>,
    triangles: Sequence<Triangle>,
    root: u64,
}
impl SourceIndex {
    pub(crate) fn build(
        captured: &FullCaptured,
        check: &(dyn Fn() -> Result<(), JobError> + Sync),
    ) -> Result<Self, JobError> {
        let mut nw = SequenceWriter::new(&captured.scratch, "hlod-source-nodes", 65536)?;
        let mut tw = SequenceWriter::new(&captured.scratch, "hlod-source-triangles", 65536)?;
        let mut reader = captured
            .triangles
            .range(0, captured.triangles.len(), 65536)?;
        fn build(
            count: u64,
            rank: &mut u64,
            reader: &mut impl Iterator<Item = Result<FullTriangle, crate::spill::SpillError>>,
            nw: &mut SequenceWriter<Node>,
            tw: &mut SequenceWriter<Triangle>,
            check: &(dyn Fn() -> Result<(), JobError> + Sync),
        ) -> Result<(u64, Node), JobError> {
            if count <= BLOCK as u64 {
                check()?;
                let first = *rank;
                let mut tris = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let t = reader
                        .next()
                        .transpose()?
                        .ok_or_else(|| state("truncated source index"))?;
                    let mut triangle = Triangle::new(t.triangle.positions, *rank);
                    triangle.order = (t.triangle.source, t.expanded_ordinal);
                    triangle.material = t.material;
                    tris.push(triangle);
                    *rank += 1;
                }
                mark_controls(&mut tris);
                let s = Surface::new(tris)?;
                let offset = nw.len();
                for n in &s.nodes {
                    nw.push(n)?;
                }
                for t in &s.triangles {
                    tw.push(t)?;
                }
                let mut node = s.nodes[0];
                node.first = first;
                node.end = *rank;
                node.left = offset;
                node.right = s.nodes.len() as u64;
                node.start = 0;
                node.count = count as u32;
                let id = nw.len();
                nw.push(&node)?;
                return Ok((id, node));
            }
            // Split on a whole-block boundary; leaves have 256 entries except the last.
            let left_count = (count.div_ceil(BLOCK as u64) / 2) * BLOCK as u64;
            let (a, na) = build(left_count, rank, reader, nw, tw, check)?;
            let (b, nb) = build(count - left_count, rank, reader, nw, tw, check)?;
            let mut bounds = na.bounds();
            bounds.union(&nb.bounds());
            let n = Node {
                lo: bounds.min.map(f32::to_bits),
                hi: bounds.max.map(f32::to_bits),
                first: na.first,
                end: nb.end,
                left: a,
                right: b,
                start: 0,
                count: 0,
                area_bits: (na.area() + nb.area()).to_bits(),
            };
            let id = nw.len();
            nw.push(&n)?;
            Ok((id, n))
        }
        let (root, _) = build(
            captured.triangles.len(),
            &mut 0,
            &mut reader,
            &mut nw,
            &mut tw,
            check,
        )?;
        Ok(Self {
            nodes: nw.finish()?,
            triangles: tw.finish()?,
            root,
        })
    }
    pub(crate) fn query<'a>(
        &'a self,
        captured: &'a FullCaptured,
        key: TileKey,
    ) -> Result<SourceQuery<'a>, JobError> {
        self.query_depth(captured, key, 12)
    }
    #[cfg(test)]
    pub(crate) fn paged_query<'a>(
        &'a self,
        captured: &'a FullCaptured,
        key: TileKey,
    ) -> Result<SourceQuery<'a>, JobError> {
        self.query_depth(captured, key, 0)
    }
    fn query_depth<'a>(
        &'a self,
        captured: &'a FullCaptured,
        key: TileKey,
        limit: u32,
    ) -> Result<SourceQuery<'a>, JobError> {
        if key.first >= key.end || key.end > captured.triangles.len() {
            return Err(state("source query range outside capture"));
        }
        let mut query = SourceQuery {
            captured,
            key,
            nodes: self.nodes.random()?,
            triangles: self.triangles.random()?,
            full: captured.triangles.random()?,
            surface: &self.triangles,
            node_cache: HashMap::with_capacity_and_hasher(CACHE_NODES, Default::default()),
            node_fifo: VecDeque::with_capacity(CACHE_NODES),
            blocks: HashMap::with_capacity_and_hasher(CACHE_BLOCKS, Default::default()),
            block_fifo: VecDeque::with_capacity(CACHE_BLOCKS),
            encoded: Vec::new(),
            node_buf: Vec::new(),
            tri_buf: Vec::new(),
            full_buf: Vec::new(),
            block_bytes: 0,
            region: Vec::new(),
            region_root: 0,
            block_limit: CACHE_BLOCKS,
            last: None,
        };
        query.region_root = query.build_region(self.root, 0, limit)?;
        if query
            .region
            .iter()
            .all(|r| r.children.is_some() || r.node.count > 0)
        {
            // Complete resident query hierarchy: drop the paged node cache before
            // admitting twice as many fine blocks. Still inside QUERY_BYTES.
            query.node_cache = HashMap::default();
            query.node_fifo = VecDeque::new();
            query.block_limit = 192;
        }
        query.check_charge()?;
        Ok(query)
    }
}
#[derive(Clone, Copy)]
struct Region {
    node: Node,
    children: Option<[u32; 2]>,
}
#[derive(Clone, Copy)]
enum Visit {
    Resident(u32),
    Disk(u64),
}
/// S3 calls nearest() for rank/bary/distance, then triangle() only for donors actually
/// shaded. FullTriangle supplies original UVs, colours, normals, flags and material;
/// image decoding/caches remain S3's separately charged responsibility.
pub(crate) struct SourceQuery<'a> {
    pub(crate) captured: &'a FullCaptured,
    pub(crate) key: TileKey,
    nodes: RandomRecords<Node>,
    triangles: RandomRecords<Triangle>,
    full: RandomRecords<FullTriangle>,
    surface: &'a Sequence<Triangle>,
    node_cache: HashMap<u64, Node>,
    node_fifo: VecDeque<u64>,
    blocks: HashMap<u64, Surface>,
    block_fifo: VecDeque<u64>,
    encoded: Vec<u8>,
    node_buf: Vec<Node>,
    tri_buf: Vec<Triangle>,
    full_buf: Vec<FullTriangle>,
    block_bytes: usize,
    region: Vec<Region>,
    region_root: u32,
    block_limit: usize,
    last: Option<Triangle>,
}
impl SourceQuery<'_> {
    fn node(&mut self, id: u64) -> Result<Node, JobError> {
        if let Some(n) = self.node_cache.get(&id) {
            return Ok(*n);
        }
        self.nodes
            .read(id, 1, &mut self.encoded, &mut self.node_buf)?;
        let n = self.node_buf[0];
        if self.node_cache.len() == CACHE_NODES {
            self.node_cache.remove(&self.node_fifo.pop_front().unwrap());
        }
        self.node_fifo.push_back(id);
        self.node_cache.insert(id, n);
        Ok(n)
    }
    /// Captured source positions only, streamed from the compact index. Expand to
    /// complete disk blocks, then clip by stored source rank: BVH order within a
    /// block differs from capture order but is deterministic for all jobs/budgets.
    pub(crate) fn surface(
        &self,
    ) -> Result<impl Iterator<Item = Result<Triangle, JobError>> + '_, JobError> {
        let key = self.key;
        let first = key.first / BLOCK as u64 * BLOCK as u64;
        let end = key
            .end
            .div_ceil(BLOCK as u64)
            .saturating_mul(BLOCK as u64)
            .min(self.surface.len());
        Ok(self
            .surface
            .range(first, end, 65536)?
            .filter_map(move |t| match t {
                Ok(mut t) if t.rank >= key.first && t.rank < key.end => {
                    let block_first = t.rank / BLOCK as u64 * BLOCK as u64;
                    if key.first > block_first || key.end < block_first + BLOCK as u64 {
                        t.controls = 7;
                    }
                    Some(Ok(t))
                }
                Ok(_) => None,
                Err(e) => Some(Err(e.into())),
            }))
    }
    // Trim the global rank BVH once per tile, collapse unary range paths, and keep
    // at most 8191 query nodes resident. Deeper branches retain disk node IDs.
    // Small/medium tiles then perform pointer-only traversal, with no cache lookup
    // or I/O for the upper hierarchy. Large tiles use the same bounded paged path.
    fn build_region(&mut self, id: u64, depth: u32, limit: u32) -> Result<u32, JobError> {
        let node = self.node(id)?;
        let children = if node.count == 0 {
            let a = self.node(node.left)?;
            let b = self.node(node.right)?;
            match (a.intersects(self.key), b.intersects(self.key)) {
                (true, false) => return self.build_region(node.left, depth, limit),
                (false, true) => return self.build_region(node.right, depth, limit),
                (false, false) => return Err(state("empty source query hierarchy")),
                (true, true) if depth < limit => Some([
                    self.build_region(node.left, depth + 1, limit)?,
                    self.build_region(node.right, depth + 1, limit)?,
                ]),
                _ => None,
            }
        } else {
            None
        };
        let at = self.region.len() as u32;
        self.region.push(Region { node, children });
        Ok(at)
    }
    // Charge actual vector capacities and conservative hash allocation bounds,
    // plus 128 KiB for handles, allocator headers and an in-flight replacement block.
    // Refuse rather than letting a cache/backend change exceed the admitted workspace.
    fn check_charge(&self) -> Result<(), JobError> {
        use std::mem::size_of;
        let bytes = self.node_cache.capacity() * 2 * (size_of::<(u64, Node)>() + 1)
            + self.blocks.capacity() * 2 * (size_of::<(u64, Surface)>() + 1)
            + (self.node_fifo.capacity() + self.block_fifo.capacity()) * size_of::<u64>()
            + self.region.capacity() * size_of::<Region>()
            + self.encoded.capacity()
            + self.node_buf.capacity() * size_of::<Node>()
            + self.tri_buf.capacity() * size_of::<Triangle>()
            + self.full_buf.capacity() * size_of::<FullTriangle>()
            + self.block_bytes
            + (128 << 10);
        if bytes as u64 > QUERY_BYTES {
            return Err(resource(format!(
                "source query cache needs {bytes} bytes, admitted {QUERY_BYTES}"
            )));
        }
        Ok(())
    }
    fn visit(&mut self, v: Visit) -> Result<(Node, Option<[Visit; 2]>), JobError> {
        match v {
            Visit::Resident(i) => {
                let r = self.region[i as usize];
                let children = match r.children {
                    Some([a, b]) => Some([Visit::Resident(a), Visit::Resident(b)]),
                    None if r.node.count == 0 => {
                        Some([Visit::Disk(r.node.left), Visit::Disk(r.node.right)])
                    }
                    None => None,
                };
                Ok((r.node, children))
            }
            Visit::Disk(id) => {
                let n = self.node(id)?;
                let children =
                    (n.count == 0).then_some([Visit::Disk(n.left), Visit::Disk(n.right)]);
                Ok((n, children))
            }
        }
    }
    pub(crate) fn nearest(&mut self, p: V3) -> Result<Hit, JobError> {
        let mut winning = self
            .last
            .filter(|t| t.rank >= self.key.first && t.rank < self.key.end);
        let mut best = winning
            .map(|t| make_hit(p, t, usize::MAX))
            .unwrap_or_else(Hit::empty);
        let root = self.visit(Visit::Resident(self.region_root))?;
        let mut stack = [root; 64];
        let mut len = 1;
        while len > 0 {
            len -= 1;
            let (n, children) = stack[len];
            if !n.intersects(self.key) || n.distance(p) > best.distance2 {
                continue;
            }
            if n.count > 0 {
                if !self.blocks.contains_key(&n.first) {
                    self.nodes.read(
                        n.left,
                        n.right as usize,
                        &mut self.encoded,
                        &mut self.node_buf,
                    )?;
                    self.triangles.read(
                        n.first,
                        n.count as usize,
                        &mut self.encoded,
                        &mut self.tri_buf,
                    )?;
                    let surface = Surface {
                        nodes: self.node_buf.clone(),
                        triangles: self.tri_buf.clone(),
                    };
                    if self.blocks.len() == self.block_limit {
                        let old = self
                            .blocks
                            .remove(&self.block_fifo.pop_front().unwrap())
                            .unwrap();
                        self.block_bytes -= old.charge();
                    }
                    self.block_fifo.push_back(n.first);
                    self.block_bytes += surface.charge();
                    self.blocks.insert(n.first, surface);
                    self.check_charge()?;
                }
                let previous = best.rank;
                let surface = &self.blocks[&n.first];
                surface.nearest(p, self.key, &mut best);
                if best.rank != previous {
                    winning = Some(surface.triangles[best.index]);
                }
            } else {
                let [left, right] =
                    children.ok_or_else(|| state("source branch without children"))?;
                let a = self.visit(left)?;
                let b = self.visit(right)?;
                let (near, far) = if a.0.distance(p) <= b.0.distance(p) {
                    (a, b)
                } else {
                    (b, a)
                };
                stack[len] = far;
                stack[len + 1] = near;
                len += 2;
            }
        }
        if !best.distance2.is_finite() {
            return Err(state("source nearest query has no finite witness"));
        }
        self.last = winning;
        Ok(best)
    }
    /// Full records `first..first + count` into caller-owned (and caller-charged)
    /// buffers, with one positioned read.
    pub(crate) fn triangles(
        &self,
        first: u64,
        count: usize,
        encoded: &mut Vec<u8>,
        out: &mut Vec<FullTriangle>,
    ) -> Result<(), JobError> {
        Ok(self.full.read(first, count, encoded, out)?)
    }
    /// Make the fine block below leaf node `n` resident (the bounded FIFO of
    /// [`SourceQuery::nearest`]).
    fn load_block(&mut self, n: &Node) -> Result<(), JobError> {
        if self.blocks.contains_key(&n.first) {
            return Ok(());
        }
        self.nodes
            .read(n.left, n.right as usize, &mut self.encoded, &mut self.node_buf)?;
        self.triangles
            .read(n.first, n.count as usize, &mut self.encoded, &mut self.tri_buf)?;
        let surface = Surface {
            nodes: self.node_buf.clone(),
            triangles: self.tri_buf.clone(),
        };
        if self.blocks.len() == self.block_limit {
            let old = self
                .blocks
                .remove(&self.block_fifo.pop_front().unwrap())
                .unwrap();
            self.block_bytes -= old.charge();
        }
        self.block_fifo.push_back(n.first);
        self.block_bytes += surface.charge();
        self.blocks.insert(n.first, surface);
        self.check_charge()
    }
    /// Budgeted best-first [`SourceQuery::nearest`] over the paged hierarchy and its fine
    /// blocks (see [`Surface::nearest_budget`]): exact unless `budget` runs out, then the
    /// best candidate so far; deterministic per query point and hint.
    pub(crate) fn nearest_budget(
        &mut self,
        p: V3,
        budget: &mut Budget,
        frontier: &mut Frontier,
        inner: &mut Frontier,
    ) -> Result<Hit, JobError> {
        use std::cmp::Reverse;
        let mut winning = self
            .last
            .filter(|t| t.rank >= self.key.first && t.rank < self.key.end);
        let mut best = winning
            .map(|t| make_hit(p, t, usize::MAX))
            .unwrap_or_else(Hit::empty);
        if best.distance2.is_finite() {
            budget.spend();
        }
        let mut entries: Vec<(Node, Option<[Visit; 2]>)> = Vec::new();
        entries.push(self.visit(Visit::Resident(self.region_root))?);
        frontier.clear();
        frontier.push(Reverse((entries[0].0.distance(p).to_bits(), 0)));
        while let Some(Reverse((d, e))) = frontier.pop() {
            if f64::from_bits(d) > best.distance2 || !budget.spend() {
                break;
            }
            let (n, children) = entries[e as usize];
            if !n.intersects(self.key) {
                continue;
            }
            if n.count > 0 {
                self.load_block(&n)?;
                let surface = &self.blocks[&n.first];
                if surface.nearest_budget(p, self.key, &mut best, budget, inner) {
                    winning = Some(surface.triangles[best.index]);
                }
                if budget.exhausted {
                    break;
                }
            } else {
                let [left, right] =
                    children.ok_or_else(|| state("source branch without children"))?;
                for v in [left, right] {
                    let child = self.visit(v)?;
                    if child.0.intersects(self.key) {
                        frontier.push(Reverse((child.0.distance(p).to_bits(), entries.len() as u64)));
                        entries.push(child);
                    }
                }
            }
        }
        if !best.distance2.is_finite() {
            return Err(state("source nearest query has no finite witness"));
        }
        self.last = winning;
        Ok(best)
    }
    pub(crate) fn triangle(&mut self, hit: Hit) -> Result<FullTriangle, JobError> {
        self.full
            .read(hit.rank, 1, &mut self.encoded, &mut self.full_buf)?;
        Ok(self.full_buf[0])
    }
    pub(crate) fn primitive(&self, hit: Hit) -> SourcePrimitive {
        SourcePrimitive {
            node: hit.order.0.node,
            mesh: hit.order.0.mesh,
            primitive: hit.order.0.primitive,
            expanded_ordinal: hit.order.1,
            material: hit.material,
        }
    }
    /// Sum complete stored subtrees; only the two clipped boundary blocks scan triangles.
    pub(crate) fn area(&mut self) -> Result<f64, JobError> {
        let mut stack = vec![Visit::Resident(self.region_root)];
        let mut area = 0.;
        while let Some(v) = stack.pop() {
            let (node, children) = self.visit(v)?;
            if !node.intersects(self.key) {
                continue;
            }
            if node.first >= self.key.first && node.end <= self.key.end {
                area += node.area();
            } else if let Some(children) = children {
                stack.extend(children);
            } else {
                self.triangles.read(
                    node.first,
                    node.count as usize,
                    &mut self.encoded,
                    &mut self.tri_buf,
                )?;
                area += self
                    .tri_buf
                    .iter()
                    .filter(|t| t.rank >= self.key.first && t.rank < self.key.end)
                    .map(|t| t.area())
                    .sum::<f64>();
            }
        }
        Ok(area)
    }
}

pub(crate) type V3 = [f64; 3];
pub(crate) fn add(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] + b[i])
}
pub(crate) fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(crate) fn mul(a: V3, k: f64) -> V3 {
    a.map(|v| v * k)
}
pub(crate) fn dot(a: V3, b: V3) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// Rescaling handles every finite magnitude without squaring into overflow.
pub(crate) fn unit(a: V3) -> Option<V3> {
    if a.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let scale = a.iter().map(|v| v.abs()).fold(0.0, f64::max);
    if scale == 0.0 {
        return None;
    }
    let q = a.map(|v| v / scale);
    let n = dot(q, q).sqrt();
    (n > 0.0 && n.is_finite()).then(|| q.map(|v| v / n))
}
pub(crate) fn bary(p: [V3; 3], w: V3) -> V3 {
    std::array::from_fn(|i| p[0][i] * w[0] + p[1][i] * w[1] + p[2][i] * w[2])
}
fn segment(q: V3, a: V3, b: V3) -> (V3, f64) {
    let e = sub(b, a);
    let n = dot(e, e);
    let t = if n == 0.0 {
        0.0
    } else {
        (dot(sub(q, a), e) / n).clamp(0.0, 1.0)
    };
    (add(a, mul(e, t)), t)
}
/// Returns an actual barycentric source-surface witness, including degenerates.
/// Its distance is appearance correspondence, never a geometric certificate.
pub(crate) fn closest(q: V3, p: [V3; 3]) -> V3 {
    let ab = sub(p[1], p[0]);
    let ac = sub(p[2], p[0]);
    let ap = sub(q, p[0]);
    // Gram determinant / Voronoi products lose the narrow dimension on slivers.
    // Cross-product barycentrics preserve it (including the 1e8:1 analytic control).
    let n = cross(ab, ac);
    let n2 = dot(n, n);
    if n2 <= 1e-8 * dot(ab, ab) * dot(ac, ac) {
        if n2 > 0. {
            let v = dot(cross(ap, ac), n) / n2;
            let w = dot(cross(ab, ap), n) / n2;
            if v >= 0. && w >= 0. && v + w <= 1. {
                return [1. - v - w, v, w];
            }
        }
        return closest_edges(q, p);
    }
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let bp = sub(q, p[1]);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return [0.0, 1.0, 0.0];
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 && d1 != d3 {
        let v = d1 / (d1 - d3);
        return [1.0 - v, v, 0.0];
    }
    let cp = sub(q, p[2]);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return [0.0, 0.0, 1.0];
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 && d2 != d6 {
        let w = d2 / (d2 - d6);
        return [1.0 - w, 0.0, w];
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 >= d3 && d5 >= d6 && (d4 - d3) + (d5 - d6) != 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return [0.0, 1.0 - w, w];
    }
    let den = va + vb + vc;
    if den > 0.0 && den.is_finite() {
        let v = (vb / den).clamp(0.0, 1.0);
        let w = (vc / den).clamp(0.0, 1.0 - v);
        return [1.0 - v - w, v, w];
    }
    closest_edges(q, p)
}
fn closest_edges(q: V3, p: [V3; 3]) -> V3 {
    let mut best = (f64::INFINITY, [1.0, 0.0, 0.0]);
    for (a, b) in [(0, 1), (1, 2), (2, 0)] {
        let (s, t) = segment(q, p[a], p[b]);
        let d = distance2(q, s);
        if d < best.0 {
            let mut w = [0.0; 3];
            w[a] = 1.0 - t;
            w[b] = t;
            best = (d, w);
        }
    }
    best.1
}
pub(crate) fn distance2(a: V3, b: V3) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_controls_skip_interior_vertices_and_cover_boundaries() {
        let mut triangles = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let p = |dx, dy| [(x + dx) as f32, (y + dy) as f32, 0.];
                for face in [[p(0, 0), p(1, 0), p(0, 1)], [p(1, 0), p(1, 1), p(0, 1)]] {
                    triangles.push(Triangle::new(face, triangles.len() as u64));
                }
            }
        }
        mark_controls(&mut triangles);
        let mut checks = rustc_hash::FxHashSet::default();
        for t in triangles {
            for (corner, p) in t.bits.into_iter().enumerate() {
                if t.controls & (1 << corner) != 0 {
                    checks.insert(p);
                }
            }
        }
        assert_eq!(checks.len(), 32);
        assert!(!checks.contains(&[4f32.to_bits(), 4f32.to_bits(), 0f32.to_bits()]));
    }
    #[test]
    fn analytic_extreme_sliver_keeps_its_interior_dimension() {
        let t = [[0., 0., 0.], [1e8, 0., 0.], [1e8, 1., 0.]];
        for (p, truth) in [
            ([5e7, 0.25, 0.], 0.),
            ([5e7, 0.25, 2.], 2.),
            ([5e7, -0.25, 0.], 0.25),
        ] {
            let weights = closest(p, t);
            let distance = distance2(p, bary(t, weights)).sqrt();
            assert!((distance - truth).abs() < 1e-12, "{weights:?}, {distance}");
        }
    }
    #[test]
    fn ties_use_source_provenance_before_morton_rank() {
        let mut a = Triangle::new([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], 0);
        a.order.0.triangle = 100;
        let mut b = Triangle::new([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], 1);
        b.order.0.triangle = 2;
        let surface = Surface::new(vec![a, b]).unwrap();
        let key = TileKey {
            first: 0,
            end: 2,
            depth: 0,
            leaf_begin: 0,
            leaf_end: 1,
        };
        let mut hit = Hit::empty();
        surface.nearest([0.2, 0.2, 1.], key, &mut hit);
        assert_eq!(hit.rank, 1);
        let mut hit = Hit::empty();
        surface.nearest([0.2, 0.2, 1.], TileKey { end: 1, ..key }, &mut hit);
        assert_eq!(hit.rank, 0);
    }
}
