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
        if end - start <= BVH_LEAF || chi[axis].partial_cmp(&clo[axis]) != Some(Ordering::Greater) {
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
    /// An accepted fill: its unfiltered candidate is bounded by the fill's certified
    /// distance, so boxes are pruned against it too.
    fill: bool,
    /// A completed query of this same face (key, ranges, normal) found no triangle that
    /// faces the proxy, so `best` stays empty whatever the point: boxes are pruned
    /// against the unfiltered candidate alone.
    no_facing: bool,
}
impl Query<'_> {
    /// Whether a box at squared distance `lower` cannot beat the candidates so far.
    fn pruned(
        &self,
        lower: f64,
        slack: f64,
        best: Option<Witness>,
        unfiltered: Option<Witness>,
    ) -> bool {
        let beaten = |w: Option<Witness>| w.is_some_and(|b| lower > b.distance2 + slack);
        if self.no_facing {
            return beaten(unfiltered);
        }
        beaten(best) && (!self.fill || beaten(unfiltered))
    }
}
/// The face a completed query proved to have no facing donor. A face's normal and donor
/// ranges are fixed, so the proof holds for every later point of the same face.
struct NoFacing {
    key: SourceKey,
    ranges: Vec<std::ops::Range<u64>>,
    normal: [u64; 3],
}
impl NoFacing {
    fn matches(&self, key: SourceKey, ranges: &[std::ops::Range<u64>], normal: V3) -> bool {
        self.key == key && self.ranges == ranges && self.normal == normal.map(f64::to_bits)
    }
}
/// How far a donor may lie from a face with no facing donor, in longest face edges.
const FALLBACK_EDGES: f64 = 2.;
/// One candidate of a nearest-witness query, shared by both search paths: a triangle
/// of the face's own source primitive inside its donor ranges; `best` must also face
/// the proxy normal, `unfiltered` need not. Ties are broken by lineage, source triangle
/// and rank, so the winner is a strict minimum.
///
/// `unfiltered` is tracked for every face. A fill uses it beyond its facing bound; any
/// other face uses it only when no candidate faces the proxy at all (see
/// [`CapturedSource::nearest_result`]). Pruning consults it for a non-fill face only
/// once the face is known to have no facing candidate ([`Query::no_facing`]), so
/// tracking it never changes which facing witness wins.
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
        ..
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
        facing_fallback: false,
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
    if unfiltered.is_none_or(earlier) {
        *unfiltered = Some(witness);
    }
    if math::face_normal(p).is_some_and(|n| math::dot(n, normal) > 0.) && best.is_none_or(earlier) {
        *best = Some(witness);
    }
    Ok(())
}

/// The longest edge of a proxy face.
fn longest_edge(face: &FrozenFace) -> f64 {
    let p = face.positions.map(|p| p.map(f64::from));
    (0..3)
        .map(|i| math::distance2(p[i], p[(i + 1) % 3]).sqrt())
        .fold(0., f64::max)
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
    /// No same-primitive candidate faces the proxy: this is the nearest one regardless
    /// of orientation.
    pub facing_fallback: bool,
}
/// One decoded source image (with its sampling mip chain) in a [`SourceImages`] cache.
/// Its bytes count against the cache from its decode until its last holder drops it.
pub(super) struct CachedImage {
    chain: mips::Chain,
    bytes: u64,
    live: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl Drop for CachedImage {
    fn drop(&mut self) {
        self.live
            .fetch_sub(self.bytes, std::sync::atomic::Ordering::Relaxed);
    }
}
type ImageKey = (usize, mips::Class);
struct ImageEntry {
    image: std::sync::Arc<CachedImage>,
    last: u64,
}
struct ImageState {
    capacity: u64,
    entries: std::collections::HashMap<ImageKey, ImageEntry>,
    tick: u64,
    /// Decode peaks reserved by workers decoding now.
    reserved: u64,
}
impl ImageState {
    /// Drop least recently used entries (never `keep`) while `live + extra` exceeds the
    /// capacity. An entry a worker still holds stays counted until that worker drops it.
    fn evict(&mut self, live: &std::sync::atomic::AtomicU64, extra: u64, keep: Option<ImageKey>) {
        let over = |state: &Self| {
            live.load(std::sync::atomic::Ordering::Relaxed) + state.reserved + extra
                > state.capacity
        };
        while over(self) {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(k, _)| Some(**k) != keep)
                .min_by_key(|(_, e)| e.last)
                .map(|(k, _)| *k)
            else {
                return;
            };
            self.entries.remove(&oldest);
        }
    }
}
/// The decoded source images of a bake, shared by its workers: a least recently used
/// cache of decoded images and their sampling mip chains, bounded by bytes.
///
/// Each worker reserves `cache_limit` bytes of the bake lease for source images (its
/// tile charge), and the cache's capacity is the sum of the reservations of the workers
/// running. A worker holds at most one image outside the cache and drops it before it
/// decodes another, and every image's decode peak is within its worker's reservation,
/// so once the unheld entries are evicted, every worker's held image or decode peak
/// fits its own reservation. Which images are resident never changes a sample.
pub(crate) struct SourceImages {
    state: std::sync::Mutex<ImageState>,
    /// Bytes of decoded images alive, cached or held.
    live: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Images decoded, and the seconds they took (diagnostics).
    decodes: std::sync::atomic::AtomicU64,
}
impl SourceImages {
    pub(crate) fn new(capacity: u64) -> Self {
        Self {
            state: std::sync::Mutex::new(ImageState {
                capacity,
                entries: Default::default(),
                tick: 0,
                reserved: 0,
            }),
            live: Default::default(),
            decodes: Default::default(),
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, ImageState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Set the capacity (the reservations of the workers about to run), evicting least
    /// recently used images down to it.
    pub(crate) fn set_capacity(&self, capacity: u64) {
        let mut state = self.lock();
        state.capacity = capacity;
        state.evict(&self.live, 0, None);
    }
    /// Bytes of decoded images alive now.
    #[cfg(test)]
    pub(crate) fn live_bytes(&self) -> u64 {
        self.live.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// Images decoded so far.
    pub(crate) fn decodes(&self) -> u64 {
        self.decodes.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// The cached image `key`, else `decode` it (outside the lock) within `peak`
    /// reserved bytes and cache it. Returns the image and whether it was decoded now.
    fn get(
        &self,
        key: ImageKey,
        peak: u64,
        decode: impl FnOnce() -> Result<mips::Chain, JobError>,
    ) -> Result<(std::sync::Arc<CachedImage>, bool), JobError> {
        {
            let mut state = self.lock();
            state.tick += 1;
            let tick = state.tick;
            if let Some(e) = state.entries.get_mut(&key) {
                e.last = tick;
                return Ok((e.image.clone(), false));
            }
            state.evict(&self.live, peak, None);
            state.reserved += peak;
        }
        let decoded = decode();
        let mut state = self.lock();
        state.reserved -= peak;
        let chain = decoded?;
        self.decodes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bytes = chain
            .levels
            .iter()
            .map(|l| l.rgba.capacity() as u64)
            .sum::<u64>()
            .max(chain.charged_bytes as u64);
        self.live
            .fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
        let image = std::sync::Arc::new(CachedImage {
            chain,
            bytes,
            live: self.live.clone(),
        });
        state.tick += 1;
        let tick = state.tick;
        // A sibling may have decoded the same image meanwhile: keep the cached one.
        let image = match state.entries.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                e.get_mut().last = tick;
                e.get().image.clone()
            }
            std::collections::hash_map::Entry::Vacant(e) => e
                .insert(ImageEntry {
                    image: image.clone(),
                    last: tick,
                })
                .image
                .clone(),
        };
        state.evict(&self.live, 0, Some(key));
        Ok((image, true))
    }
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
    /// The latest face found to have no facing donor in the current node.
    no_facing: Option<NoFacing>,
    /// Attribute flags of each source primitive in the node, built on first use.
    primitive_flags: Option<BTreeMap<SourceKey, u8>>,
    /// Fine hierarchy over `resident`, when its bytes fit beside them.
    bvh: Option<Bvh>,
    bvh_stack: Vec<(u32, f64)>,
    root: u64,
    first: u64,
    end: u64,
    work: Cell<u64>,
    limit: u64,
    cache_limit: u64,
    /// The bake's decoded source images, shared by its workers.
    images: std::sync::Arc<SourceImages>,
    /// The image this worker sampled last (it holds at most this one outside the cache).
    current: Option<(ImageKey, std::sync::Arc<CachedImage>)>,
    /// Images this worker decoded so far, and the seconds they took (diagnostics).
    decodes: u64,
    decode_seconds: f64,
}
impl<'a> CapturedSource<'a> {
    /// A source with a decoded-image cache of its own, of `cache_limit` bytes.
    pub(crate) fn new(
        captured: &'a FullCaptured,
        cache_limit: u64,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Self, JobError> {
        Self::shared(
            captured,
            cache_limit,
            std::sync::Arc::new(SourceImages::new(cache_limit)),
            check,
        )
    }
    /// A worker's source: `cache_limit` bytes of the lease are its reservation in the
    /// bake's shared decoded-image cache `images`.
    pub(crate) fn shared(
        captured: &'a FullCaptured,
        cache_limit: u64,
        images: std::sync::Arc<SourceImages>,
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
            no_facing: None,
            primitive_flags: None,
            bvh: None,
            bvh_stack: Vec::new(),
            root: root.ok_or_else(|| invalid("empty captured hierarchy"))?,
            first: 0,
            end: captured.triangles.len(),
            work: Cell::new(0),
            limit: 0,
            cache_limit,
            images,
            current: None,
            decodes: 0,
            decode_seconds: 0.,
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
        self.primitive_flags = None;
        self.no_facing = None;
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
    /// The published attribute flags (NORMAL, TANGENT, COLOR, ...) of source primitive
    /// `k` in the current node: what its leaves carry. 0 when the node has none of it.
    pub(super) fn primitive_flags(
        &mut self,
        k: SourceKey,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<u8, JobError> {
        if self.primitive_flags.is_none() {
            let mut map = BTreeMap::new();
            for (i, t) in self.tris(self.first, self.end)?.enumerate() {
                if i.is_multiple_of(1 << 14) {
                    check()?;
                }
                let t = t?;
                *map.entry(key(&t)).or_insert(0u8) |= t.flags;
            }
            self.primitive_flags = Some(map);
        }
        Ok(self
            .primitive_flags
            .as_ref()
            .and_then(|m| m.get(&k).copied())
            .unwrap_or(0))
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
    /// Research-only broad phase: one box query per proxy face, followed by drawing.
    pub(super) fn projection_candidates(
        &self,
        face: &FrozenFace,
        pad: f64,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Vec<Witness>, JobError> {
        let Some(bvh) = &self.bvh else {
            return Ok(Vec::new());
        };
        let p = face.positions.map(|p| p.map(f64::from));
        let lo: V3 =
            std::array::from_fn(|a| p.iter().map(|p| p[a]).fold(f64::INFINITY, f64::min) - pad);
        let hi: V3 =
            std::array::from_fn(|a| p.iter().map(|p| p[a]).fold(f64::NEG_INFINITY, f64::max) + pad);
        let normal = math::face_normal(p).unwrap();
        let mut out = Vec::new();
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            check()?;
            let n = bvh.nodes[i];
            if (0..3).any(|a| n.hi[a] < lo[a] || n.lo[a] > hi[a]) {
                continue;
            }
            if n.len == 0 {
                stack.push(n.start as usize);
                stack.push(n.start as usize + 1);
                continue;
            }
            for &local in &bvh.order[n.start as usize..(n.start + n.len) as usize] {
                let rank = self.resident_first + u64::from(local);
                let t = self.resident[local as usize];
                if key(&t) != face.source || !face.donor_ranges.iter().any(|r| r.contains(&rank)) {
                    continue;
                }
                if !math::face_normal(t.triangle.positions.map(|p| p.map(f64::from)))
                    .is_some_and(|n| math::dot(n, normal) > 0.)
                {
                    continue;
                }
                out.push(Witness {
                    rank,
                    triangle: t,
                    bary: [1., 0., 0.],
                    distance2: 0.,
                    fill_fallback: false,
                    facing_fallback: false,
                });
            }
        }
        Ok(out)
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
            no_facing: fill.is_none()
                && self
                    .no_facing
                    .as_ref()
                    .is_some_and(|m| m.matches(k, ranges, normal)),
        };
        let limit = FALLBACK_EDGES * longest_edge(face);
        if self.bvh.is_some() {
            self.nearest_bvh(&query, &mut best, &mut unfiltered, check)?;
            self.remember_no_facing(&query, best);
            return self.nearest_result(fill, best, unfiltered, limit);
        }
        // Nearest-first depth-first search: children are pushed farthest first, so the
        // closest box is searched first and the distance bound tightens early. The
        // result does not depend on visit order: candidates are compared by a strict
        // total order (distance, then lineage, triangle and rank), and a box is pruned
        // only when its lower distance bound exceeds the best distance by more than the
        // arithmetic slack of a computed distance (as in the BVH path).
        let slack = 1e-12 * (1. + math::dot(q, q));
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
            if query.pruned(lower, slack, best, unfiltered) {
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
        self.remember_no_facing(&query, best);
        self.nearest_result(fill, best, unfiltered, limit)
    }
    /// A finished query that found no facing triangle visited every candidate of the
    /// face (nothing is pruned while `best` is empty), so no triangle of this face's
    /// key and ranges faces its normal. Later points of the face then prune against the
    /// nearest candidate alone and select the same unfiltered witness.
    fn remember_no_facing(&mut self, query: &Query<'_>, best: Option<Witness>) {
        if best.is_some() || query.fill || query.no_facing {
            return;
        }
        self.no_facing = Some(NoFacing {
            key: query.key,
            ranges: query.ranges.to_vec(),
            normal: query.normal.map(f64::to_bits),
        });
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
            if query.pruned(lower, slack, *best, *unfiltered) {
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
        limit: f64,
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
        if let Some(b) = best {
            return Ok(b);
        }
        // No triangle of the face's own primitive in its donor ranges faces the proxy:
        // simplification turned the face away from every same-primitive donor (a flipped
        // sliver or fold). Its appearance is then the nearest same-primitive surface
        // point regardless of orientation: the same material and texture, the surface the
        // face approximates, chosen by the same strict order. Both searches prune only
        // once a facing witness exists, so with none this is the exhaustive nearest over
        // the face's candidates, within the same query work budget.
        let mut w =
            unfiltered.ok_or_else(|| invalid("proxy face has no same-primitive source donor"))?;
        // A donor far beyond the face's own size is no longer the surface the face
        // approximates; refuse it instead of baking an unrelated appearance. A NaN
        // distance or limit refuses too.
        let distance = w.distance2.sqrt();
        if distance.is_nan() || limit.is_nan() || distance > limit {
            return Err(unsupported(format!(
                "proxy has no facing same-primitive source appearance donor; the nearest \
                 same-primitive triangle is {distance:.3e} away, beyond the {limit:.3e} \
                 limit of {FALLBACK_EDGES} face edges"
            )));
        }
        w.facing_fallback = true;
        Ok(w)
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
        // Two deliberate differences from the full search: it charges no query work,
        // and on malformed input where the full search would find no same-primitive
        // corner at all (and refuse) it returns white. A proxy face's own source
        // triangle lies in the node, so admitted input always has such a corner.
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
    /// The H28 material class representative of a source material (`u32::MAX` stays).
    pub(super) fn class(&self, material: u32) -> u32 {
        self.captured.material_classes.of(material)
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
            facing_fallback: false,
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
        let key = (image, class);
        if self.current.as_ref().is_none_or(|(k, _)| *k != key) {
            // Hold no image while fetching another (see [`SourceImages`]).
            self.current = None;
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
            // Every image switch is admitted against this worker's reservation, cached
            // or not, so admission does not depend on what is resident.
            if peak > self.cache_limit {
                return Err(resource("source image decode/cache budget"));
            }
            let started = std::time::Instant::now();
            let cache_limit = self.cache_limit;
            let (cached, decoded) = self.images.get(key, peak, || {
                let encoded = snapshot.reconstruct(check)?;
                let (_, rgba) = source::decode_image_admitted(
                    &encoded,
                    snapshot.identity.mime.as_deref(),
                    source::DecodeAdmission::LARGE_MESH,
                    &mut || check(),
                )?;
                drop(encoded);
                mips::build(
                    info.width,
                    info.height,
                    rgba,
                    class,
                    None,
                    usize::try_from(cache_limit).map_err(|_| resource("source cache budget"))?,
                )
            })?;
            if decoded {
                self.decodes += 1;
                self.decode_seconds += started.elapsed().as_secs_f64();
            }
            self.current = Some((key, cached));
        }
        let c = &self.current.as_ref().unwrap().1.chain;
        let uv: [f64; 2] = std::array::from_fn(|a| {
            (0..3)
                .map(|i| {
                    f64::from(w.triangle.corners[i].texcoords[binding.texcoord][a]) * w.bary[i]
                })
                .sum()
        });
        Ok(Some((
            sample(&c.levels, c.class, sampler, uv, lod)?,
            binding.texcoord,
        )))
    }
    pub(super) fn work(&self) -> u64 {
        self.work.get()
    }
    pub(super) fn cache_limit(&self) -> u64 {
        self.cache_limit
    }
    /// Images decoded into the cache so far, and their seconds (diagnostics).
    pub(super) fn decodes(&self) -> (u64, f64) {
        (self.decodes, self.decode_seconds)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::{
        config::{Profile, TreeConfig},
        full,
    };
    use std::fs;

    /// Deterministic xorshift in [0, 1).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// `count` random triangles in a 4 m box (every seventh an exact copy of the one
    /// before, for distance ties), random per-vertex COLOR_0 or none, captured with
    /// sixteen-triangle tree leaves so the streamed search has many boxes to prune.
    fn capture(count: usize, color: bool, seed: u64) -> (tempfile::TempDir, full::FullCaptured) {
        let work = tempfile::tempdir().unwrap();
        let input = full::tests::fixture(work.path(), count);
        let mut rng = Rng(seed);
        let bin = input.parent().unwrap().join("attributes.bin");
        let mut bytes = fs::read(&bin).unwrap();
        let mut positions = Vec::with_capacity(count * 9);
        for t in 0..count {
            if t % 7 == 6 {
                let previous: Vec<f32> = positions[(t - 1) * 9..t * 9].to_vec();
                positions.extend(previous);
            } else {
                positions.extend((0..9).map(|_| (rng.next() * 4.) as f32));
            }
        }
        for (i, v) in positions.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        // COLOR_0 is the last attribute: count * 3 corners of four floats.
        let colors = bytes.len() - count * 48;
        for i in 0..count * 12 {
            let v = rng.next() as f32;
            bytes[colors + i * 4..colors + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        fs::write(&bin, &bytes).unwrap();
        let mut doc: serde_json::Value =
            serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
        let extreme = |pick: fn(f32, f32) -> f32| -> Vec<f64> {
            (0..3)
                .map(|a| {
                    f64::from(
                        positions
                            .iter()
                            .skip(a)
                            .step_by(3)
                            .copied()
                            .reduce(pick)
                            .unwrap(),
                    )
                })
                .collect()
        };
        doc["accessors"][0]["min"] = serde_json::json!(extreme(f32::min));
        doc["accessors"][0]["max"] = serde_json::json!(extreme(f32::max));
        if !color {
            doc["meshes"][0]["primitives"][0]["attributes"]
                .as_object_mut()
                .unwrap()
                .remove("COLOR_0");
        }
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let mut config = TreeConfig::new(Profile::Balanced);
        config.jobs =
            crate::mesh_large::config::Jobs::Fixed(std::num::NonZeroUsize::new(1).unwrap());
        config.leaf_triangles = 16;
        config.sort.run_bytes = 1 << 16;
        config.scratch_dir = Some(work.path().join("scratch"));
        fs::create_dir_all(config.scratch_dir.as_ref().unwrap()).unwrap();
        let captured = full::capture(&input, &config, &mut || Ok(())).unwrap();
        (work, captured)
    }

    fn root(c: &full::FullCaptured) -> NodeRecord {
        let n = c.triangles.len();
        c.tree
            .nodes
            .reader()
            .unwrap()
            .map(Result::unwrap)
            .find(|r| r.first == 0 && r.end == n)
            .unwrap()
    }

    /// The exhaustive answer, written independently of both search paths.
    fn brute(
        tris: &[FullTriangle],
        q: V3,
        face: &FrozenFace,
        normal: V3,
    ) -> (Option<Witness>, Option<Witness>) {
        let mut best: Option<Witness> = None;
        let mut unfiltered: Option<Witness> = None;
        let order = |w: &Witness| {
            (
                w.distance2,
                w.triangle.triangle.source.triangle != face.source_triangle,
                w.triangle.triangle.source.triangle,
                w.rank,
            )
        };
        let less = |a: &Witness, b: &Witness| {
            let (x, y) = (order(a), order(b));
            x.0 < y.0 || (x.0 == y.0 && (x.1, x.2, x.3) < (y.1, y.2, y.3))
        };
        for (rank, t) in tris.iter().enumerate() {
            let rank = rank as u64;
            if !face.donor_ranges.iter().any(|r| r.contains(&rank)) || key(t) != face.source {
                continue;
            }
            let p = t.triangle.positions.map(|p| p.map(f64::from));
            let bary = math::closest(q, p);
            let w = Witness {
                rank,
                triangle: *t,
                bary,
                distance2: math::distance2(q, math::bary(p, bary)),
                fill_fallback: false,
                facing_fallback: false,
            };
            if unfiltered.as_ref().is_none_or(|u| less(&w, u)) {
                unfiltered = Some(w);
            }
            if math::face_normal(p).is_some_and(|n| math::dot(n, normal) > 0.)
                && best.as_ref().is_none_or(|b| less(&w, b))
            {
                best = Some(w);
            }
        }
        (best, unfiltered)
    }

    fn same(a: &Result<Witness, JobError>, b: &Result<Witness, JobError>) -> bool {
        match (a, b) {
            (Ok(a), Ok(b)) => {
                a.rank == b.rank
                    && a.distance2.to_bits() == b.distance2.to_bits()
                    && a.bary.map(f64::to_bits) == b.bary.map(f64::to_bits)
                    && a.fill_fallback == b.fill_fallback
                    && a.facing_fallback == b.facing_fallback
            }
            (Err(a), Err(b)) => a.kind() == b.kind() && a.message() == b.message(),
            _ => false,
        }
    }

    fn unit_normal(t: &FullTriangle) -> Option<V3> {
        math::face_normal(t.triangle.positions.map(|p| p.map(f64::from)))
    }

    /// Both searches against the brute-force answer for one query, returning the result.
    fn check_query(
        fast: &mut CapturedSource<'_>,
        slow: &mut CapturedSource<'_>,
        tris: &[FullTriangle],
        face: &FrozenFace,
        q: V3,
        normal: V3,
        what: &str,
    ) -> Result<Witness, JobError> {
        let ok = &mut || Ok(());
        let (best, unfiltered) = brute(tris, q, face, normal);
        let limit = FALLBACK_EDGES * longest_edge(face);
        let expected = fast.nearest_result(face.fill_bound, best, unfiltered, limit);
        let a = fast.nearest(q, face, normal, ok);
        let b = slow.nearest(q, face, normal, ok);
        assert!(same(&a, &expected), "BVH {what}");
        assert!(same(&b, &expected), "streamed {what}");
        expected
    }

    /// Many points of one face whose donors none faces: after the first, the memo prunes
    /// against the nearest donor alone, yet every point selects exactly the donor the
    /// exhaustive scan does, with less query work.
    fn repeated_faces_agree_with_the_exhaustive_scan(
        fast: &mut CapturedSource<'_>,
        slow: &mut CapturedSource<'_>,
        tris: &[FullTriangle],
        base: &FrozenFace,
        seed: u64,
    ) {
        let mut rng = Rng(seed ^ 0x1234_5678);
        let (mut faces, mut cheaper, mut accepted, mut refused) = (0, 0, 0, 0);
        for f in 0..60 {
            let mut face = base.clone();
            // Alternate faces too small for their donors (refused beyond the limit) and
            // large ones (always accepted).
            let size = if f % 3 == 0 { 0.05 } else { 40. };
            face.positions = [[0., 0., 0.], [size, 0., 0.], [0., size, 0.]];
            let k = (f * 11 % 300) as u64;
            face.donor_ranges = match f % 3 {
                0 => std::iter::once(k..k + 1).collect(),
                1 => vec![k..k + 2, k + 5..k + 9],
                _ => std::iter::once(10..120).collect(),
            };
            // A normal that no donor of the face faces.
            let probe: V3 = [1., 1., 1.];
            let normal = (0..4000).find_map(|_| {
                let n = math::unit(std::array::from_fn(|_| rng.next() - 0.5))?;
                brute(tris, probe, &face, n).0.is_none().then_some(n)
            });
            let Some(normal) = normal else { continue };
            faces += 1;
            let mut works = Vec::new();
            for i in 0..14 {
                let q: V3 = std::array::from_fn(|_| rng.next() * 6. - 1.);
                let before = fast.work.get();
                let r = check_query(fast, slow, tris, &face, q, normal, &format!("face {f}/{i}"));
                works.push(fast.work.get() - before);
                match r {
                    Ok(w) => {
                        assert!(w.facing_fallback);
                        accepted += 1;
                    }
                    Err(e) => {
                        assert_eq!(e.kind(), crate::JobErrorKind::Unsupported);
                        assert!(
                            e.message().contains(" away, beyond the "),
                            "{}",
                            e.message()
                        );
                        refused += 1;
                    }
                }
            }
            // The first query paid for every candidate; later ones never more, and on
            // the larger donor sets less in total.
            assert!(
                works[1..].iter().all(|&w| w <= works[0]),
                "face {f}: {works:?}"
            );
            cheaper += usize::from(works[1..].iter().sum::<u64>() < works[0] * 13);
        }
        assert!(faces > 30, "{faces} faces with a non-facing normal");
        assert!(
            accepted > 100 && refused > 20,
            "{accepted} accepted, {refused} refused"
        );
        assert!(
            cheaper * 3 > faces,
            "{cheaper} of {faces} faces got cheaper"
        );
    }

    /// One face with donors of varying normals, queried with a normal that some donor
    /// faces and one that none does, alternately: only the second falls back, and a
    /// proof recorded for it never leaks to the first.
    fn varying_normals_on_one_face_do_not_share_the_no_facing_proof(
        fast: &mut CapturedSource<'_>,
        slow: &mut CapturedSource<'_>,
        tris: &[FullTriangle],
        base: &FrozenFace,
        seed: u64,
    ) {
        let mut rng = Rng(seed ^ 0x77);
        let mut mixed = 0;
        for start in 0..40u64 {
            let mut face = base.clone();
            face.positions = [[0., 0., 0.], [40., 0., 0.], [0., 40., 0.]];
            face.donor_ranges = std::iter::once(start * 7..start * 7 + 4).collect();
            let donors: Vec<V3> = (start * 7..start * 7 + 4)
                .filter(|&r| key(&tris[r as usize]) == face.source)
                .filter_map(|r| unit_normal(&tris[r as usize]))
                .collect();
            let Some(&facing) = donors.first() else {
                continue;
            };
            // Normals of the donors differ, yet some normal turns every one away.
            let probe: V3 = [1., 1., 1.];
            let away = (0..4000).find_map(|_| {
                let n = math::unit(std::array::from_fn(|_| rng.next() - 0.5))?;
                brute(tris, probe, &face, n).0.is_none().then_some(n)
            });
            let Some(away) = away else { continue };
            if donors.iter().all(|d| math::dot(*d, facing) > 0.99) {
                continue;
            }
            mixed += 1;
            for i in 0..12 {
                let q: V3 = std::array::from_fn(|_| rng.next() * 6. - 1.);
                let toward = i % 2 == 0;
                let r = check_query(
                    fast,
                    slow,
                    tris,
                    &face,
                    q,
                    if toward { facing } else { away },
                    &format!("mixed face {start}/{i}"),
                );
                let w = r.expect("size 40 accepts every donor");
                assert_eq!(w.facing_fallback, !toward, "face {start}/{i}");
            }
        }
        assert!(mixed > 5, "{mixed} faces with varying donor normals");
    }

    #[test]
    fn bvh_streamed_and_exhaustive_nearest_witnesses_agree_including_ties() {
        let ok = &mut || Ok(());
        for (seed, color) in [
            (0x9e37_79b9_7f4a_7c15, true),
            (0x2545_f491_4f6c_dd1d, false),
        ] {
            let (_work, c) = capture(320, color, seed);
            let node = root(&c);
            let tris: Vec<FullTriangle> =
                c.triangles.reader().unwrap().map(Result::unwrap).collect();
            assert_eq!(tris.len(), 320);
            let mut fast = CapturedSource::new(&c, 1 << 30, ok).unwrap();
            fast.begin(node, u64::MAX / 4).unwrap();
            assert!(fast.make_resident(1 << 30, ok).unwrap() > 0);
            assert!(fast.bvh.as_ref().is_some_and(|b| b.nodes.len() > 8));
            let mut slow = CapturedSource::new(&c, 1 << 30, ok).unwrap();
            slow.begin(node, u64::MAX / 4).unwrap();
            assert!(slow.bvh.is_none() && slow.resident.is_empty());
            let mut rng = Rng(seed ^ 0xabcdef);
            let base = FrozenFace {
                ordinal: 0,
                source: key(&tris[0]),
                source_triangle: tris[40].triangle.source.triangle,
                material: tris[0].material,
                positions: tris[0].triangle.positions,
                words: [crate::mesh_large::bake::types::PositionWord::Float([0; 3]); 3],
                transform: Default::default(),
                welds: [[0; 3]; 3],
                features: Vec::new(),
                fill_bound: None,
                donor_ranges: std::iter::once(0..320).collect(),
            };
            let mut checked = 0;
            let mut facing_fallbacks = 0;
            for i in 0..600 {
                let mut face = base.clone();
                // Exact vertices and centroids of duplicated triangles tie on distance.
                let q: V3 = match i % 3 {
                    0 => tris[(i * 7 + 6) % 320].triangle.positions[i % 3].map(f64::from),
                    1 => {
                        let p = tris[(i * 7 + 5) % 320].triangle.positions;
                        std::array::from_fn(|a| {
                            (f64::from(p[0][a]) + f64::from(p[1][a]) + f64::from(p[2][a])) / 3.
                        })
                    }
                    _ => std::array::from_fn(|_| rng.next() * 6. - 1.),
                };
                let normal = math::unit(std::array::from_fn(|_| rng.next() - 0.5)).unwrap();
                if i % 4 == 1 {
                    face.donor_ranges = vec![10..90, 150..151, 200..300];
                }
                // One or two candidates: often none faces the proxy (the facing fallback).
                if i % 7 == 3 {
                    let k = ((i * 13) % 318) as u64;
                    face.donor_ranges = vec![k..k + 1, k + 2..k + 3];
                }
                if i % 5 == 2 {
                    face.fill_bound = Some(rng.next() * 2.);
                }
                let (best, unfiltered) = brute(&tris, q, &face, normal);
                let limit = FALLBACK_EDGES * longest_edge(&face);
                let expected = fast.nearest_result(face.fill_bound, best, unfiltered, limit);
                let a = fast.nearest(q, &face, normal, ok);
                let b = slow.nearest(q, &face, normal, ok);
                assert!(same(&a, &expected), "BVH query {i}");
                assert!(same(&b, &expected), "streamed query {i}");
                checked += usize::from(expected.is_ok());
                facing_fallbacks += usize::from(expected.is_ok_and(|w| w.facing_fallback));
            }
            assert!(checked > 300);
            assert!(facing_fallbacks > 10, "{facing_fallbacks} facing fallbacks");
            repeated_faces_agree_with_the_exhaustive_scan(&mut fast, &mut slow, &tris, &base, seed);
            varying_normals_on_one_face_do_not_share_the_no_facing_proof(
                &mut fast, &mut slow, &tris, &base, seed,
            );
            // Corner colour: the resident shortcut agrees with the full search, both on
            // every actual corner (weld matches) and off-vertex (nearest fallback).
            assert_eq!(fast.colorless, !color);
            for (i, t) in tris.iter().enumerate().step_by(3) {
                for corner in 0..3 {
                    let mut weld =
                        t.triangle.positions[corner].map(|v| if v == 0. { 0 } else { v.to_bits() });
                    if i % 2 == 1 {
                        weld[0] = (f32::from_bits(weld[0]) + 0.013).to_bits();
                    }
                    let own = t.triangle.source.triangle;
                    let a = fast.color(key(t), own, weld, ok).unwrap();
                    let b = slow.color(key(t), own, weld, ok).unwrap();
                    assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits), "triangle {i}");
                    if !color {
                        assert_eq!(a, [1.; 4]);
                    }
                }
            }
        }
    }

    /// A distinct 16x16 chain for image `i` (5,456 bytes of levels).
    fn chain(i: usize) -> mips::Chain {
        let rgba = (0..16 * 16 * 4).map(|k| (k * 7 + i * 31) as u8).collect();
        mips::build(16, 16, rgba, mips::Class::Colour, None, 1 << 20).unwrap()
    }
    fn chain_bytes() -> u64 {
        chain(0)
            .levels
            .iter()
            .map(|l| l.rgba.capacity() as u64)
            .sum::<u64>()
            .max(chain(0).charged_bytes as u64)
    }

    #[test]
    fn shared_images_hit_evict_least_recent_and_count_held_images() {
        let bytes = chain_bytes();
        let peak = bytes;
        // Room for two images, or one beside a decode.
        let images = SourceImages::new(2 * bytes);
        let key = |i: usize| (i, mips::Class::Colour);
        let get = |i: usize| images.get(key(i), peak, || Ok(chain(i))).unwrap();
        for i in 0..2 {
            let (image, decoded) = get(i);
            assert!(decoded);
            assert_eq!(image.chain.levels[0].rgba, chain(i).levels[0].rgba);
        }
        // A hit decodes nothing and refreshes the entry.
        assert!(!get(0).1);
        // Image 2 needs its decode peak beside the two cached: the least recent (1) goes.
        assert!(get(2).1);
        assert_eq!(images.decodes(), 3);
        assert!(!get(0).1 && !get(2).1);
        assert!(get(1).1);
        assert!(images.live_bytes() <= 2 * bytes);
        // A held image stays counted after its eviction, until its holder drops it.
        let held = get(3).0;
        images.set_capacity(0);
        assert_eq!(images.live_bytes(), bytes);
        drop(held);
        assert_eq!(images.live_bytes(), 0);
        // A failed decode releases its reservation and caches nothing.
        images.set_capacity(2 * bytes);
        assert!(images
            .get(key(9), peak, || Err(invalid("decode failure")))
            .is_err());
        assert!(get(9).1);
    }

    #[test]
    fn shared_images_across_workers_return_exact_chains_within_the_reservations() {
        let bytes = chain_bytes();
        // Four workers, each reserving one decode peak (here one image).
        let images = SourceImages::new(4 * bytes);
        std::thread::scope(|scope| {
            for w in 0..4usize {
                let images = &images;
                scope.spawn(move || {
                    let mut held: Option<std::sync::Arc<CachedImage>> = None;
                    for step in 0..400usize {
                        let i = (step * 7 + w * 3) % 11;
                        // A worker drops its image before it fetches another.
                        drop(held.take());
                        let (image, _) = images
                            .get((i, mips::Class::Colour), bytes, || Ok(chain(i)))
                            .unwrap();
                        assert_eq!(image.chain.levels[0].rgba, chain(i).levels[0].rgba);
                        assert!(images.live_bytes() <= 4 * bytes);
                        held = Some(image);
                    }
                    drop(held);
                });
            }
        });
        assert!(images.decodes() >= 11);
        images.set_capacity(0);
        assert_eq!(images.live_bytes(), 0);
    }
}
