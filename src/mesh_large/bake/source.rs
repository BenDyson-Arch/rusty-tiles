//! Concrete captured-source queries. The hierarchy is streamed once into an
//! immutable postorder spill; a query owns only its bounded traversal stack.
use super::{
    frames::Frame,
    invalid,
    math::{self, V3},
    resource,
    types::{FrozenFace, SourceKey},
    unsupported,
};
use crate::{
    gltf_source::source::{self, TextureSlot},
    mesh_large::{
        attributes::{FullTriangle, COLOR, NORMAL, TANGENT},
        full::FullCaptured,
        leaves::{crops, mips},
        records::{NodeKind, NodeRecord},
    },
    spill::{Record, Sequence, SequenceWriter},
    JobError,
};
use std::{cell::Cell, cmp::Ordering, collections::BTreeMap};

/// A fine bounding-volume hierarchy over the resident triangles (indices into the
/// resident slice), at most [`BVH_LEAF`] triangles per leaf. The captured-tree index
/// stops at tile-sized leaves of thousands of triangles; this one is what makes a
/// nearest-witness query logarithmic once the tile's triangles are resident.
struct Bvh {
    nodes: Vec<BvhNode>,
    order: Vec<u32>,
}
#[derive(Clone, Copy)]
struct BvhNode {
    lo: V3,
    hi: V3,
    /// Leaf: `order[start..start + len]`. Inner (`len == 0`): children `start` and
    /// `start + 1`.
    start: u32,
    len: u32,
}
const BVH_LEAF: usize = 8;
/// Bytes a [`Bvh`] over `n` triangles may hold, including its build workspace.
fn bvh_bytes(n: u64) -> u64 {
    n.saturating_mul(4 + 24).saturating_add(
        (2 * n.div_ceil(4) + 1).saturating_mul(std::mem::size_of::<BvhNode>() as u64),
    )
}
fn build_bvh(
    tris: &[FullTriangle],
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Bvh, JobError> {
    let n = tris.len();
    let points = |i: u32| {
        tris[i as usize]
            .triangle
            .positions
            .map(|p| p.map(f64::from))
    };
    let mut centres = Vec::new();
    centres
        .try_reserve_exact(n)
        .map_err(|_| resource("source BVH centres"))?;
    centres.extend((0..n as u32).map(|i| {
        let p = points(i);
        std::array::from_fn::<f64, 3, _>(|a| (p[0][a] + p[1][a] + p[2][a]) / 3.)
    }));
    let mut order: Vec<u32> = Vec::new();
    order
        .try_reserve_exact(n)
        .map_err(|_| resource("source BVH order"))?;
    order.extend(0..n as u32);
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(2 * n.div_ceil(4) + 1)
        .map_err(|_| resource("source BVH nodes"))?;
    let empty = BvhNode {
        lo: [0.; 3],
        hi: [0.; 3],
        start: 0,
        len: 0,
    };
    nodes.push(empty);
    let mut pending = vec![(0usize, 0usize, n)];
    while let Some((at, start, end)) = pending.pop() {
        check()?;
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut clo = [f64::INFINITY; 3];
        let mut chi = [f64::NEG_INFINITY; 3];
        for &i in &order[start..end] {
            for p in points(i) {
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
            for a in 0..3 {
                clo[a] = clo[a].min(centres[i as usize][a]);
                chi[a] = chi[a].max(centres[i as usize][a]);
            }
        }
        let axis = (0..3)
            .max_by(|&a, &b| (chi[a] - clo[a]).total_cmp(&(chi[b] - clo[b])))
            .unwrap();
        if end - start <= BVH_LEAF || !(chi[axis] > clo[axis]) {
            nodes[at] = BvhNode {
                lo,
                hi,
                start: start as u32,
                len: (end - start) as u32,
            };
            continue;
        }
        let mid = start + (end - start) / 2;
        order[start..end].select_nth_unstable_by(mid - start, |&a, &b| {
            centres[a as usize][axis]
                .total_cmp(&centres[b as usize][axis])
                .then(a.cmp(&b))
        });
        let left = nodes.len();
        if left + 2 > nodes.capacity() {
            return Err(resource("source BVH node bound"));
        }
        nodes.push(empty);
        nodes.push(empty);
        nodes[at] = BvhNode {
            lo,
            hi,
            start: left as u32,
            len: 0,
        };
        pending.push((left, start, mid));
        pending.push((left + 1, mid, end));
    }
    Ok(Bvh { nodes, order })
}

/// The fixed inputs of one nearest-witness query.
struct Query<'q> {
    q: V3,
    normal: V3,
    key: SourceKey,
    lineage: u64,
    ranges: &'q [std::ops::Range<u64>],
    fill: bool,
}
/// One candidate of a nearest-witness query, shared by both search paths: a triangle
/// of the face's own source primitive inside its donor ranges; `best` must also face
/// the proxy normal, `unfiltered` (fills only) need not. Ties are broken by lineage,
/// source triangle and rank, so the winner is a strict minimum.
fn consider(
    query: &Query<'_>,
    rank: u64,
    t: FullTriangle,
    best: &mut Option<Witness>,
    unfiltered: &mut Option<Witness>,
) -> Result<(), JobError> {
    let Query {
        q,
        normal,
        key: k,
        lineage,
        ranges,
        fill,
    } = *query;
    if !(ranges.iter().any(|r| r.contains(&rank)) && key(&t) == k) {
        return Ok(());
    }
    let p = t.triangle.positions.map(|p| p.map(f64::from));
    let bary = math::closest(q, p);
    let d = math::distance2(q, math::bary(p, bary));
    if !d.is_finite() {
        return Err(invalid("nonfinite source witness distance"));
    }
    let witness = Witness {
        rank,
        triangle: t,
        bary,
        distance2: d,
        fill_fallback: false,
    };
    let earlier = |b: Witness| {
        d < b.distance2
            || (d == b.distance2
                && (
                    t.triangle.source.triangle != lineage,
                    t.triangle.source.triangle,
                    rank,
                ) < (
                    b.triangle.triangle.source.triangle != lineage,
                    b.triangle.triangle.source.triangle,
                    b.rank,
                ))
    };
    if fill && unfiltered.is_none_or(earlier) {
        *unfiltered = Some(witness);
    }
    if math::face_normal(p).is_some_and(|n| math::dot(n, normal) > 0.) && best.is_none_or(earlier) {
        *best = Some(witness);
    }
    Ok(())
}

/// Source triangles of a rank range: a resident slice or a spill stream.
enum Tris<'a> {
    Resident(std::slice::Iter<'a, FullTriangle>),
    Stream(crate::spill::SequenceReader<'a, FullTriangle>),
}
impl Iterator for Tris<'_> {
    type Item = Result<FullTriangle, JobError>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Tris::Resident(i) => i.next().map(|t| Ok(*t)),
            Tris::Stream(r) => r.next().map(|t| t.map_err(JobError::from)),
        }
    }
}
#[derive(Clone, Copy)]
struct QueryNode {
    first: u64,
    end: u64,
    lo: [u32; 3],
    hi: [u32; 3],
    children: [u64; 8],
    count: u8,
}
impl PartialEq for QueryNode {
    fn eq(&self, b: &Self) -> bool {
        (self.first, self.end) == (b.first, b.end)
    }
}
impl Eq for QueryNode {}
impl Ord for QueryNode {
    fn cmp(&self, b: &Self) -> Ordering {
        (self.first, self.end).cmp(&(b.first, b.end))
    }
}
impl PartialOrd for QueryNode {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Record for QueryNode {
    const SIZE: usize = 105;
    fn encode(&self, b: &mut [u8]) {
        b[..8].copy_from_slice(&self.first.to_le_bytes());
        b[8..16].copy_from_slice(&self.end.to_le_bytes());
        for (i, x) in self.lo.iter().chain(self.hi.iter()).enumerate() {
            b[16 + i * 4..20 + i * 4].copy_from_slice(&x.to_le_bytes());
        }
        for (i, x) in self.children.iter().enumerate() {
            b[40 + i * 8..48 + i * 8].copy_from_slice(&x.to_le_bytes());
        }
        b[104] = self.count;
    }
    fn decode(b: &[u8]) -> Self {
        Self {
            first: u64::from_le_bytes(b[..8].try_into().unwrap()),
            end: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            lo: std::array::from_fn(|i| {
                u32::from_le_bytes(b[16 + i * 4..20 + i * 4].try_into().unwrap())
            }),
            hi: std::array::from_fn(|i| {
                u32::from_le_bytes(b[28 + i * 4..32 + i * 4].try_into().unwrap())
            }),
            children: std::array::from_fn(|i| {
                u64::from_le_bytes(b[40 + i * 8..48 + i * 8].try_into().unwrap())
            }),
            count: b[104],
        }
    }
}
struct Pending {
    node: NodeRecord,
    children: [u64; 8],
    count: u8,
}
fn finish(p: Pending, w: &mut SequenceWriter<QueryNode>) -> Result<u64, JobError> {
    if (p.node.kind == NodeKind::Internal && p.count != p.node.children)
        || (p.node.kind == NodeKind::Leaf && p.count != 0)
    {
        return Err(invalid("source hierarchy child closure"));
    }
    let i = w.len();
    w.push(&QueryNode {
        first: p.node.first,
        end: p.node.end,
        lo: p.node.bounds.min.map(f32::to_bits),
        hi: p.node.bounds.max.map(f32::to_bits),
        children: p.children,
        count: p.count,
    })?;
    Ok(i)
}
pub(super) fn key(t: &FullTriangle) -> SourceKey {
    SourceKey {
        node: t.triangle.source.node,
        mesh: t.triangle.source.mesh,
        primitive: t.triangle.source.primitive,
        instance: t.expanded_ordinal,
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Witness {
    pub rank: u64,
    pub triangle: FullTriangle,
    pub bary: V3,
    pub distance2: f64,
    pub fill_fallback: bool,
}
struct Cache {
    image: usize,
    class: mips::Class,
    chain: mips::Chain,
    sampler: crops::SourceSampler,
}
pub(crate) struct CapturedSource<'a> {
    pub(super) captured: &'a FullCaptured,
    index: Sequence<QueryNode>,
    /// The whole query index, resident when it fits a quarter of the cache lease.
    nodes: Vec<QueryNode>,
    /// The current node's source triangles `[resident_first, ..)`, when the bake granted
    /// their residency (see [`CapturedSource::make_resident`]); else they are streamed.
    resident: Vec<FullTriangle>,
    resident_first: u64,
    /// True when the resident triangles carry no COLOR_0: every corner colour is then
    /// the default white, so colour queries need no search.
    colorless: bool,
    /// Reused depth-first stack of the nearest-witness query.
    nearest_stack: Vec<(QueryNode, f64)>,
    /// Fine hierarchy over `resident`, when its bytes fit beside them.
    bvh: Option<Bvh>,
    bvh_stack: Vec<(u32, f64)>,
    root: u64,
    first: u64,
    end: u64,
    work: Cell<u64>,
    limit: u64,
    cache_limit: u64,
    cache: Option<Cache>,
}
impl<'a> CapturedSource<'a> {
    pub(crate) fn new(
        captured: &'a FullCaptured,
        cache_limit: u64,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Self, JobError> {
        let mut writer = SequenceWriter::new(&captured.scratch, "proxy-source-index", 4096)?;
        let mut reader = captured.tree.nodes.reader()?;
        let mut stack: Vec<Pending> = Vec::new();
        stack
            .try_reserve_exact(65)
            .map_err(|_| resource("source index stack"))?;
        let mut root = None;
        for next in &mut reader {
            check()?;
            let node = next?;
            while stack.last().is_some_and(|p| p.node.depth() >= node.depth()) {
                let i = finish(stack.pop().unwrap(), &mut writer)?;
                if let Some(p) = stack.last_mut() {
                    if p.count >= 8 {
                        return Err(invalid("source index fanout"));
                    }
                    p.children[p.count as usize] = i;
                    p.count += 1;
                } else {
                    if root.replace(i).is_some() {
                        return Err(invalid("multiple source roots"));
                    }
                }
            }
            if stack.len() >= 65 {
                return Err(resource("source hierarchy depth"));
            }
            if let Some(p) = stack.last() {
                if node.parent() != Some(p.node.identity())
                    || node.first < p.node.first
                    || node.end > p.node.end
                {
                    return Err(invalid("source index preorder"));
                }
            }
            stack.push(Pending {
                node,
                children: [0; 8],
                count: 0,
            });
        }
        while let Some(p) = stack.pop() {
            let i = finish(p, &mut writer)?;
            if let Some(p) = stack.last_mut() {
                if p.count >= 8 {
                    return Err(invalid("source index fanout"));
                }
                p.children[p.count as usize] = i;
                p.count += 1;
            } else {
                if root.replace(i).is_some() {
                    return Err(invalid("multiple source roots"));
                }
            }
        }
        let index = writer.finish()?;
        let mut nodes = Vec::new();
        if index
            .len()
            .checked_mul(std::mem::size_of::<QueryNode>() as u64)
            .is_some_and(|b| b <= cache_limit / 4)
        {
            nodes
                .try_reserve_exact(index.len() as usize)
                .map_err(|_| resource("resident source index"))?;
            for n in index.range(0, index.len(), 1 << 16)? {
                check()?;
                nodes.push(n?);
            }
        }
        Ok(Self {
            captured,
            index,
            nodes,
            resident: Vec::new(),
            resident_first: 0,
            colorless: false,
            nearest_stack: Vec::new(),
            bvh: None,
            bvh_stack: Vec::new(),
            root: root.ok_or_else(|| invalid("empty captured hierarchy"))?,
            first: 0,
            end: captured.triangles.len(),
            work: Cell::new(0),
            limit: 0,
            cache_limit,
            cache: None,
        })
    }
    pub(super) fn begin(&mut self, node: NodeRecord, limit: u64) -> Result<(), JobError> {
        if node.first >= node.end || node.end > self.captured.triangles.len() || limit == 0 {
            return Err(invalid("proxy source query range/budget"));
        }
        self.first = node.first;
        self.end = node.end;
        self.work.set(0);
        self.limit = limit;
        self.resident = Vec::new();
        self.bvh = None;
        self.colorless = false;
        Ok(())
    }
    /// Hold the current node's source triangles in memory when they fit `budget` bytes,
    /// so correspondence queries read memory instead of the spill, plus a fine search
    /// hierarchy over them when that also fits. Returns the bytes held (0 when the
    /// triangles do not fit; queries then stream as before, same results).
    pub(super) fn make_resident(
        &mut self,
        budget: u64,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<u64, JobError> {
        self.resident = Vec::new();
        self.bvh = None;
        let count = self.end - self.first;
        let bytes = count.saturating_mul(std::mem::size_of::<FullTriangle>() as u64);
        if bytes > budget {
            return Ok(0);
        }
        let mut resident = Vec::new();
        resident
            .try_reserve_exact(count as usize)
            .map_err(|_| resource("resident proxy source triangles"))?;
        for (k, t) in self
            .captured
            .triangles
            .range(self.first, self.end, 1 << 16)?
            .enumerate()
        {
            if k.is_multiple_of(1 << 14) {
                check()?;
            }
            resident.push(t?);
        }
        self.colorless = !resident.iter().any(|t| t.flags & COLOR != 0);
        let tree = bvh_bytes(count);
        let held = if bytes.saturating_add(tree) <= budget && count <= u64::from(u32::MAX) {
            self.bvh = Some(build_bvh(&resident, check)?);
            bytes + tree
        } else {
            bytes
        };
        self.resident = resident;
        self.resident_first = self.first;
        Ok(held)
    }
    fn node(&self, i: u64) -> Result<QueryNode, JobError> {
        if let Some(n) = self.nodes.get(i as usize) {
            return Ok(*n);
        }
        self.index
            .range(i, i + 1, 4096)?
            .next()
            .transpose()?
            .ok_or_else(|| invalid("missing source query node"))
    }
    /// Source triangles `[first, end)` in rank order, from memory when resident.
    fn tris(&self, first: u64, end: u64) -> Result<Tris<'_>, JobError> {
        let offset = first.wrapping_sub(self.resident_first);
        if !self.resident.is_empty()
            && first >= self.resident_first
            && end - self.resident_first <= self.resident.len() as u64
        {
            let n = (end - first) as usize;
            return Ok(Tris::Resident(
                self.resident[offset as usize..offset as usize + n].iter(),
            ));
        }
        Ok(Tris::Stream(
            self.captured.triangles.range(first, end, 4096)?,
        ))
    }
    fn charge(&self) -> Result<(), JobError> {
        let work = self
            .work
            .get()
            .checked_add(1)
            .ok_or_else(|| resource("source query work overflow"))?;
        self.work.set(work);
        if work > self.limit {
            return Err(resource("source correspondence work budget"));
        }
        Ok(())
    }
    /// Stable nearest actual surface witness. Distance never participates in H17
    /// acceptance: source appearance may move arbitrarily on a certified proxy.
    pub(super) fn nearest(
        &mut self,
        q: V3,
        face: &FrozenFace,
        normal: V3,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Witness, JobError> {
        let k = face.source;
        let lineage = face.source_triangle;
        let fill = face.fill_bound;
        let ranges = &face.donor_ranges;
        let mut best: Option<Witness> = None;
        let mut unfiltered: Option<Witness> = None;
        let query = Query {
            q,
            normal,
            key: k,
            lineage,
            ranges,
            fill: fill.is_some(),
        };
        if self.bvh.is_some() {
            self.nearest_bvh(&query, &mut best, &mut unfiltered, check)?;
            return self.nearest_result(fill, best, unfiltered);
        }
        // Nearest-first depth-first search: children are pushed farthest first, so the
        // closest box is searched first and the distance bound tightens early. The
        // result does not depend on visit order: candidates are compared by a strict
        // total order (distance, then lineage, triangle and rank), and a box is pruned
        // only when its lower distance bound strictly exceeds the best distance.
        let mut stack = std::mem::take(&mut self.nearest_stack);
        stack.clear();
        if stack.capacity() < 8 * 65 {
            stack
                .try_reserve_exact(8 * 65)
                .map_err(|_| resource("source query stack"))?;
        }
        let bound = |n: &QueryNode| {
            math::box_distance_lower(
                q,
                n.lo.map(|v| f64::from(f32::from_bits(v))),
                n.hi.map(|v| f64::from(f32::from_bits(v))),
            )
        };
        let root = self.node(self.root)?;
        stack.push((root, bound(&root)));
        while let Some((n, lower)) = stack.pop() {
            check()?;
            self.charge()?;
            if n.end <= self.first
                || n.first >= self.end
                || !ranges.iter().any(|r| n.first < r.end && n.end > r.start)
            {
                continue;
            }
            if best.is_some_and(|b| lower > b.distance2)
                && (fill.is_none() || unfiltered.is_some_and(|b| lower > b.distance2))
            {
                continue;
            }
            if n.count > 0 {
                let mut kids = [(root, 0f64); 8];
                for (slot, &child) in kids.iter_mut().zip(&n.children[..n.count as usize]) {
                    let c = self.node(child)?;
                    *slot = (c, bound(&c));
                }
                let kids = &mut kids[..n.count as usize];
                // Farthest first; equal bounds keep the original (last-first) push order.
                kids.reverse();
                kids.sort_by(|a, b| b.1.total_cmp(&a.1));
                for &kid in kids.iter() {
                    if stack.len() >= 8 * 65 {
                        return Err(resource("source query stack bound"));
                    }
                    stack.push(kid);
                }
                continue;
            }
            let first = n.first.max(self.first);
            let end = n.end.min(self.end);
            let mut r = self.tris(first, end)?;
            let mut rank = first;
            while let Some(t) = r.next().transpose()? {
                self.charge()?;
                consider(&query, rank, t, &mut best, &mut unfiltered)?;
                rank += 1;
                check()?;
            }
        }
        // An early error return drops the buffer; the next query reallocates it.
        self.nearest_stack = stack;
        self.nearest_result(fill, best, unfiltered)
    }
    /// [`Self::nearest`] over the fine hierarchy of the resident triangles. A box is
    /// pruned only when its lower bound exceeds the best distance by more than the
    /// arithmetic slack of a computed distance, so the answer is the exhaustive one
    /// over the same candidates, whatever the tree.
    fn nearest_bvh(
        &mut self,
        query: &Query<'_>,
        best: &mut Option<Witness>,
        unfiltered: &mut Option<Witness>,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<(), JobError> {
        let q = query.q;
        let slack = 1e-12 * (1. + math::dot(q, q));
        let mut stack = std::mem::take(&mut self.bvh_stack);
        stack.clear();
        let bvh = self
            .bvh
            .as_ref()
            .ok_or_else(|| invalid("source BVH missing"))?;
        let bound = |n: &BvhNode| math::box_distance_lower(q, n.lo, n.hi);
        stack.push((0, bound(&bvh.nodes[0])));
        while let Some((i, lower)) = stack.pop() {
            check()?;
            self.charge()?;
            if best.is_some_and(|b| lower > b.distance2 + slack)
                && (!query.fill || unfiltered.is_some_and(|b| lower > b.distance2 + slack))
            {
                continue;
            }
            let n = bvh.nodes[i as usize];
            if n.len == 0 {
                let (a, b) = (n.start, n.start + 1);
                let (la, lb) = (bound(&bvh.nodes[a as usize]), bound(&bvh.nodes[b as usize]));
                // Nearer child last, so it is searched first.
                if la <= lb {
                    stack.push((b, lb));
                    stack.push((a, la));
                } else {
                    stack.push((a, la));
                    stack.push((b, lb));
                }
                continue;
            }
            for &local in &bvh.order[n.start as usize..(n.start + n.len) as usize] {
                self.charge()?;
                let rank = self.resident_first + u64::from(local);
                if rank < self.first || rank >= self.end {
                    continue;
                }
                consider(query, rank, self.resident[local as usize], best, unfiltered)?;
            }
        }
        self.bvh_stack = stack;
        Ok(())
    }
    /// The query's answer from its best facing and unfiltered candidates.
    fn nearest_result(
        &self,
        fill: Option<f64>,
        best: Option<Witness>,
        unfiltered: Option<Witness>,
    ) -> Result<Witness, JobError> {
        if let Some(bound) = fill {
            if best.is_none_or(|b| {
                (if b.distance2 == 0. {
                    0.
                } else {
                    b.distance2.sqrt().next_up()
                }) > bound
            }) {
                let mut w = unfiltered
                    .ok_or_else(|| invalid("accepted fill has no same-primitive donor"))?;
                // Allow finite arithmetic outward error, never a new appearance bound.
                if w.distance2.sqrt() > bound.next_up() {
                    return Err(invalid(
                        "accepted fill donor violates immutable eligibility bound",
                    ));
                }
                w.fill_fallback = true;
                return Ok(w);
            }
        }
        best.ok_or_else(|| {
            unsupported("proxy has no facing same-primitive source appearance donor")
        })
    }
    /// The weld key, not the replacement face's Origin, selects canonical colour.
    pub(super) fn color(
        &mut self,
        k: SourceKey,
        own_triangle: u64,
        weld: [u32; 3],
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<[f32; 4], JobError> {
        // Both the weld match and the nearest-corner fallback return white for a
        // triangle without COLOR_0; when no resident triangle has it, skip the search.
        if self.colorless && !self.resident.is_empty() {
            return Ok([1.; 4]);
        }
        let q = weld.map(|v| f64::from(f32::from_bits(v)));
        let mut stack = vec![self.root];
        let mut best: Option<((bool, u64, u8), [f32; 4])> = None;
        while let Some(i) = stack.pop() {
            self.charge()?;
            check()?;
            let n = self.node(i)?;
            if n.end <= self.first
                || n.first >= self.end
                || math::box_distance_lower(
                    q,
                    n.lo.map(|v| f64::from(f32::from_bits(v))),
                    n.hi.map(|v| f64::from(f32::from_bits(v))),
                ) > 0.
            {
                continue;
            }
            if n.count > 0 {
                for &c in n.children[..n.count as usize].iter().rev() {
                    if stack.len() >= 8 * 65 {
                        return Err(resource("colour query stack"));
                    }
                    stack.push(c);
                }
                continue;
            }
            let mut r = self.tris(n.first.max(self.first), n.end.min(self.end))?;
            while let Some(t) = r.next().transpose()? {
                self.charge()?;
                check()?;
                if key(&t) != k {
                    continue;
                }
                for c in 0..3 {
                    let bits =
                        t.triangle.positions[c].map(|v| if v == 0. { 0 } else { v.to_bits() });
                    if bits != weld {
                        continue;
                    }
                    let order = (
                        t.triangle.source.triangle != own_triangle,
                        t.triangle.source.triangle,
                        c as u8,
                    );
                    if best.as_ref().is_none_or(|(b, _)| order < *b) {
                        best = Some((
                            order,
                            if t.flags & COLOR == 0 {
                                [1.; 4]
                            } else {
                                t.corners[c].color
                            },
                        ));
                    }
                }
            }
        }
        if let Some((_, c)) = best {
            return Ok(c);
        }
        // Simplification welds by position across primitive boundaries, so a proxy
        // corner can sit on a vertex its own primitive does not have. Its colour is then
        // the nearest same-primitive source corner in the node (deterministic order).
        self.nearest_color(k, q, check)
    }
    fn nearest_color(
        &mut self,
        k: SourceKey,
        q: V3,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<[f32; 4], JobError> {
        let mut stack = vec![self.root];
        let mut best: Option<((f64, u64, u8), [f32; 4])> = None;
        while let Some(i) = stack.pop() {
            self.charge()?;
            check()?;
            let n = self.node(i)?;
            if n.end <= self.first
                || n.first >= self.end
                || best.as_ref().is_some_and(|((d, _, _), _)| {
                    math::box_distance_lower(
                        q,
                        n.lo.map(|v| f64::from(f32::from_bits(v))),
                        n.hi.map(|v| f64::from(f32::from_bits(v))),
                    ) > *d
                })
            {
                continue;
            }
            if n.count > 0 {
                for &c in n.children[..n.count as usize].iter().rev() {
                    if stack.len() >= 8 * 65 {
                        return Err(resource("colour query stack"));
                    }
                    stack.push(c);
                }
                continue;
            }
            let mut r = self.tris(n.first.max(self.first), n.end.min(self.end))?;
            while let Some(t) = r.next().transpose()? {
                self.charge()?;
                check()?;
                if key(&t) != k {
                    continue;
                }
                for c in 0..3 {
                    let d = math::distance2(q, t.triangle.positions[c].map(f64::from));
                    let order = (d, t.triangle.source.triangle, c as u8);
                    if best.as_ref().is_none_or(|(b, _)| order < *b) {
                        best = Some((
                            order,
                            if t.flags & COLOR == 0 {
                                [1.; 4]
                            } else {
                                t.corners[c].color
                            },
                        ));
                    }
                }
            }
        }
        best.map(|(_, c)| c)
            .ok_or_else(|| invalid("proxy colour key has no same-primitive source corner"))
    }
    pub(super) fn material(&self, index: u32) -> Result<serde_json::Value, JobError> {
        if index == u32::MAX {
            return Ok(serde_json::json!({}));
        }
        let m = self
            .captured
            .materials
            .get(index as usize)
            .ok_or_else(|| invalid("proxy material index"))?;
        m.remap(
            &m.bindings()
                .map(|b| (b.texture, b.texture))
                .collect::<BTreeMap<_, _>>(),
        )
    }
    pub(super) fn record(&self, rank: u64, bary: V3) -> Result<Witness, JobError> {
        if rank >= self.captured.triangles.len()
            || bary.iter().any(|v| !v.is_finite() || *v < 0. || *v > 1.)
            || (bary.iter().sum::<f64>() - 1.).abs() > 1e-8
        {
            return Err(invalid("terminal source witness"));
        }
        let triangle = self
            .tris(rank, rank + 1)?
            .next()
            .transpose()?
            .ok_or_else(|| invalid("terminal source rank"))?;
        Ok(Witness {
            rank,
            triangle,
            bary,
            distance2: 0.,
            fill_fallback: false,
        })
    }
    pub(super) fn lod(&self, w: Witness, slot: TextureSlot, texel: f64) -> Result<f64, JobError> {
        if w.triangle.material == u32::MAX {
            return Ok(0.);
        }
        let Some(binding) = self.captured.materials[w.triangle.material as usize]
            .bindings()
            .find(|b| b.slot == slot)
        else {
            return Ok(0.);
        };
        let image = self.captured.textures[binding.texture]["source"]
            .as_u64()
            .ok_or_else(|| invalid("footprint image"))? as usize;
        let info = self
            .captured
            .image_info
            .get(image)
            .ok_or_else(|| invalid("footprint image metadata"))?;
        let p = w.triangle.triangle.positions.map(|p| p.map(f64::from));
        let uv = w.triangle.corners.map(|c| {
            [
                f64::from(c.texcoords[binding.texcoord][0]) * f64::from(info.width),
                f64::from(c.texcoords[binding.texcoord][1]) * f64::from(info.height),
            ]
        });
        let sigma = math::pixel_gradient(p, uv).unwrap_or(0.);
        let rho = texel * sigma;
        if !rho.is_finite() {
            return Err(resource("source footprint overflow"));
        }
        Ok(if rho > 0. { rho.log2() } else { 0. })
    }
    /// Conservative source UV-plane box of the actual finite isotropic filter.
    /// It may include points outside the face; the separately stored barycentric
    /// centre remains the actual source-surface witness.
    pub(super) fn footprint(
        &self,
        w: Witness,
        slot: TextureSlot,
        lod: f64,
    ) -> Result<(V3, V3), JobError> {
        let p = w.triangle.triangle.positions.map(|p| p.map(f64::from));
        let q = math::bary(p, w.bary);
        if w.triangle.material == u32::MAX {
            return Ok((q, q));
        }
        let Some(binding) = self.captured.materials[w.triangle.material as usize]
            .bindings()
            .find(|b| b.slot == slot)
        else {
            return Ok((q, q));
        };
        let texture = &self.captured.textures[binding.texture];
        let image = texture["source"]
            .as_u64()
            .ok_or_else(|| invalid("support image"))? as usize;
        let info = &self.captured.image_info[image];
        let s = crops::resolve(
            texture
                .get("sampler")
                .and_then(serde_json::Value::as_u64)
                .and_then(|n| self.captured.samplers.get(n as usize)),
        )?;
        let filter = if lod <= 0. {
            s.mag_filter
        } else {
            s.min_filter
        };
        let maxlevel = (u32::BITS - info.width.max(info.height).leading_zeros() - 1) as f64;
        let mip = if matches!(filter, 9984..=9987) {
            if matches!(filter, 9986 | 9987) {
                lod.max(0.).ceil()
            } else {
                lod.max(0.).round()
            }
        } else {
            0.
        }
        .min(maxlevel) as u32;
        let size = [(info.width >> mip).max(1), (info.height >> mip).max(1)];
        let radius = if matches!(filter, 9728 | 9984 | 9986) {
            0.5
        } else {
            1.5
        };
        let uv = w
            .triangle
            .corners
            .map(|c| c.texcoords[binding.texcoord].map(f64::from));
        let a = [uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]];
        let b = [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]];
        let det = a[0] * b[1] - a[1] * b[0];
        if det == 0. {
            return Ok((q, q));
        }
        let e = math::sub(p[1], p[0]);
        let f = math::sub(p[2], p[0]);
        let du = math::mul(
            math::sub(math::mul(e, b[1]), math::mul(f, a[1])),
            radius / (det * f64::from(size[0])),
        );
        let dv = math::mul(
            math::sub(math::mul(f, a[0]), math::mul(e, b[0])),
            radius / (det * f64::from(size[1])),
        );
        let r: V3 = std::array::from_fn(|a| du[a].abs() + dv[a].abs());
        if r.iter().any(|v| !v.is_finite()) {
            return Err(resource("source support extent overflow"));
        }
        Ok((
            std::array::from_fn(|a| (q[a] - r[a]).next_down()),
            std::array::from_fn(|a| (q[a] + r[a]).next_up()),
        ))
    }
    /// Original normal-map input admission requires authored NORMAL+TANGENT.
    /// A missing flag is a broken captured handoff, never a new derivative mode.
    pub(super) fn frame(w: Witness) -> Result<Frame, JobError> {
        let t = w.triangle;
        if t.flags & (NORMAL | TANGENT) != (NORMAL | TANGENT) {
            return Err(invalid(
                "admitted source normal binding lacks authored NORMAL/TANGENT",
            ));
        }
        Frame::donor(
            t.corners.map(|c| c.normal.map(f64::from)),
            t.corners.map(|c| c.tangent.map(f64::from)),
            w.bary,
        )
    }
    pub(super) fn signal(
        &mut self,
        w: Witness,
        slot: TextureSlot,
        lod: f64,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Option<([f64; 4], usize)>, JobError> {
        if w.triangle.material == u32::MAX {
            return Ok(None);
        }
        let m = self
            .captured
            .materials
            .get(w.triangle.material as usize)
            .ok_or_else(|| invalid("source material"))?;
        let Some(binding) = m.bindings().find(|b| b.slot == slot).copied() else {
            return Ok(None);
        };
        let tex = self
            .captured
            .textures
            .get(binding.texture)
            .ok_or_else(|| invalid("source texture"))?;
        let image = tex
            .get("source")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| invalid("source texture image"))?;
        let sampler = crops::resolve(
            tex.get("sampler")
                .and_then(serde_json::Value::as_u64)
                .and_then(|n| self.captured.samplers.get(n as usize)),
        )?;
        let class = match slot {
            TextureSlot::BaseColor | TextureSlot::Emissive => mips::Class::Colour,
            // Original normal images are raw LINEAR texture components: the
            // viewer filters unequal-length vectors BEFORE normalScale/TBN and
            // normalization. Normalizing source base pixels does not commute.
            // Source minification uses raw linear component reduction too;
            // proxy-authored normals separately own common-frame unit mips.
            TextureSlot::Normal => mips::Class::Data,
            _ => mips::Class::Data,
        };
        if self
            .cache
            .as_ref()
            .is_none_or(|c| c.image != image || c.class != class)
        {
            self.cache = None;
            let snapshot = self
                .captured
                .images
                .get(image)
                .ok_or_else(|| invalid("source image snapshot"))?;
            let info = self
                .captured
                .image_info
                .get(image)
                .ok_or_else(|| invalid("source image metadata"))?;
            let pixels = u64::from(info.width)
                .checked_mul(u64::from(info.height))
                .ok_or_else(|| resource("source decode pixels"))?;
            let peak = pixels
                .checked_mul(24)
                .and_then(|n| n.checked_add(snapshot.identity.encoded_bytes))
                .and_then(|n| n.checked_add(64 << 20))
                .ok_or_else(|| resource("source decode charge"))?;
            if peak > self.cache_limit {
                return Err(resource("source image decode/cache budget"));
            }
            let encoded = snapshot.reconstruct(check)?;
            let (_, rgba) = source::decode_image_admitted(
                &encoded,
                snapshot.identity.mime.as_deref(),
                source::DecodeAdmission::LARGE_MESH,
                &mut || check(),
            )?;
            drop(encoded);
            let chain = mips::build(
                info.width,
                info.height,
                rgba,
                class,
                None,
                usize::try_from(self.cache_limit).map_err(|_| resource("source cache budget"))?,
            )?;
            self.cache = Some(Cache {
                image,
                class,
                chain,
                sampler,
            });
        }
        let c = self.cache.as_mut().unwrap();
        c.sampler = sampler;
        let uv: [f64; 2] = std::array::from_fn(|a| {
            (0..3)
                .map(|i| {
                    f64::from(w.triangle.corners[i].texcoords[binding.texcoord][a]) * w.bary[i]
                })
                .sum()
        });
        Ok(Some((
            sample(&c.chain.levels, c.class, sampler, uv, lod)?,
            binding.texcoord,
        )))
    }
    pub(super) fn work(&self) -> u64 {
        self.work.get()
    }
    pub(super) fn cache_limit(&self) -> u64 {
        self.cache_limit
    }
    pub(super) fn density_floor(
        &mut self,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<f64, JobError> {
        let captured = self.captured;
        let mut reader = captured.triangles.range(self.first, self.end, 4096)?;
        let mut minimum = f64::INFINITY;
        let edge = self
            .captured
            .image_info
            .iter()
            .map(|i| i.width.max(i.height))
            .max()
            .unwrap_or(1)
            .max(1);
        while let Some(t) = reader.next().transpose()? {
            check()?;
            self.charge()?;
            let p = t.triangle.positions.map(|p| p.map(f64::from));
            // This heuristic is only for a zero prescribed raster density;
            // retained no-raster geometry supplies no sampling density.
            if math::face_normal(p).is_none() {
                continue;
            }
            for e in 0..3 {
                let d = math::distance2(p[e], p[(e + 1) % 3]).sqrt();
                if d > 0. {
                    minimum = minimum.min(d / f64::from(edge));
                }
            }
        }
        // Point-only geometry has no raster area. Its finite fallback does not
        // manufacture texture coverage or geometric error.
        Ok(if minimum.is_finite() && minimum > 0. {
            minimum
        } else {
            f64::from(f32::MIN_POSITIVE)
        })
    }
}
fn wrap(i: i64, n: u32, mode: u32) -> usize {
    let n = i64::from(n);
    match mode {
        33071 => i.clamp(0, n - 1) as usize,
        33648 => {
            let k = i.rem_euclid(2 * n);
            if k < n {
                k as usize
            } else {
                (2 * n - 1 - k) as usize
            }
        }
        _ => i.rem_euclid(n) as usize,
    }
}
fn linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(v: f64) -> f64 {
    if v == 0. || v == 1. {
        return v;
    }
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }
}
/// The admitted taps and fraction are finite and bounded. Nested interpolation
/// keeps constants exact; endpoint bounds contain only floating point rounding
/// of the mathematical convex combination, never an external normal input.
fn convex(a: f64, b: f64, t: f64) -> f64 {
    if t == 0. {
        a
    } else if t == 1. {
        b
    } else {
        (a + (b - a) * t).clamp(a.min(b), a.max(b))
    }
}
pub(super) fn sample(
    levels: &[mips::Level],
    class: mips::Class,
    s: crops::SourceSampler,
    uv: [f64; 2],
    lod: f64,
) -> Result<[f64; 4], JobError> {
    if levels.is_empty() || uv.iter().any(|v| !v.is_finite()) || !lod.is_finite() {
        return Err(invalid("source sampling coordinates"));
    }
    let filter = if lod <= 0. {
        s.mag_filter
    } else {
        s.min_filter
    };
    let mip = matches!(filter, 9984..=9987);
    let blend = matches!(filter, 9986 | 9987);
    let l = if mip {
        lod.max(0.).min((levels.len() - 1) as f64)
    } else {
        0.
    };
    let a = if blend {
        l.floor() as usize
    } else {
        l.round() as usize
    };
    let b = if blend {
        (a + 1).min(levels.len() - 1)
    } else {
        a
    };
    let f = if blend { l - a as f64 } else { 0. };
    let fetch = |level: &mips::Level| -> Result<[f64; 4], JobError> {
        // Reduce UV before integer conversion, including huge finite repeats.
        let uv = std::array::from_fn::<_, 2, _>(|i| match [s.wrap_s, s.wrap_t][i] {
            33071 => uv[i].clamp(0., 1.),
            33648 => {
                let q = uv[i].rem_euclid(2.);
                if q <= 1. {
                    q
                } else {
                    2. - q
                }
            }
            _ => uv[i].rem_euclid(1.),
        });
        let xy = [
            uv[0] * f64::from(level.width) - 0.5,
            uv[1] * f64::from(level.height) - 0.5,
        ];
        let get = |x: i64, y: i64| {
            let i = (wrap(y, level.height, s.wrap_t) * level.width as usize
                + wrap(x, level.width, s.wrap_s))
                * 4;
            std::array::from_fn::<_, 4, _>(|a| {
                let v = f64::from(level.rgba[i + a]) / 255.;
                if class == mips::Class::Colour && a < 3 {
                    linear(v)
                } else {
                    v
                }
            })
        };
        if matches!(filter, 9728 | 9984 | 9986) {
            return Ok(get(
                (xy[0] + 0.5).floor() as i64,
                (xy[1] + 0.5).floor() as i64,
            ));
        }
        let x = xy[0].floor() as i64;
        let y = xy[1].floor() as i64;
        let f = [xy[0] - x as f64, xy[1] - y as f64];
        let p = [get(x, y), get(x + 1, y), get(x, y + 1), get(x + 1, y + 1)];
        Ok(std::array::from_fn(|a| {
            convex(
                convex(p[0][a], p[1][a], f[0]),
                convex(p[2][a], p[3][a], f[0]),
                f[1],
            )
        }))
    };
    let p = fetch(&levels[a])?;
    let q = fetch(&levels[b])?;
    Ok(std::array::from_fn(|i| {
        let v = convex(p[i], q[i], f);
        if class == mips::Class::Colour && i < 3 {
            srgb(v)
        } else {
            v
        }
    }))
}
