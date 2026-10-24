//! The LOD pre-pass: one external join over every source corner, by welded position.
//!
//! A welded vertex is the normalized f32 bit pattern of a baked source position, shared
//! across primitives and instances (the H1 weld). For each welded vertex this pass derives:
//!
//! - `c2`: the least tile depth at which its incident triangles lie in two or more different
//!   tiles (it is a CUT vertex of every depth-`d` tile with `d >= c2`); `NEVER` when all its
//!   triangles share one leaf;
//! - `c3`: the least depth with three or more incident tiles (a seam endpoint);
//! - `special`: incident to a degenerate triangle, a source boundary edge (one incident
//!   triangle), a non-manifold edge (three or more), an edge whose two triangles traverse
//!   it in the same direction (a winding flip or a same-winding duplicate), or two
//!   coincident triangles (a double-sided face), or its incidence overflowed the bounded
//!   group (then `c2 = c3 = 0`: kept at every depth).
//!
//! Tile identity at depth `d` is the length-`d` prefix of the leaf address (a shorter leaf
//! is its own tile), so for lexicographically sorted distinct leaf addresses the number of
//! depth-`d` tiles is one plus the number of adjacent pairs whose common prefix is shorter
//! than `d`. The result is joined back to rank order with H8's exact shallowest incident
//! leaf depth `l(v)` as the slim [`LodSource`] sequence every later stage reads.
//!
//! Welded vertices are named by capture's vertex ordinal (their index in the position-sorted
//! weld table, [`CornerVertices`]), a bijection with the welded position bits, so grouping
//! and every neighbour comparison are unchanged while the sort record stays 30 bytes.
use super::super::{
    attributes::FullTriangle,
    config::HEIGHT_CAP,
    full::{CornerDepth, CornerVertices, RANK_LIMIT},
    records::{NodeKind, NodeRecord},
};
use crate::{
    spill::{ExternalSorter, Record, Scratch, Sequence, SequenceWriter, SortConfig},
    JobError, JobErrorKind,
};

pub(crate) const NEVER: u8 = u8::MAX;
/// Deepest leaf address an [`Incidence`] stores: a tree is at most `HEIGHT_CAP` levels high,
/// so a leaf has at most `HEIGHT_CAP - 1` digits.
const MAX_ADDRESS_DIGITS: u8 = (HEIGHT_CAP - 1) as u8;
/// Unused low bits of an [`Address`] holding at most [`MAX_ADDRESS_DIGITS`] digits.
const ADDRESS_SHIFT: u32 = 128 - 3 * MAX_ADDRESS_DIGITS as u32;
// The packed address word (digits, length, corner, degenerate flag) fits its 13 bytes.
const _: () = assert!(3 * MAX_ADDRESS_DIGITS as u32 + 8 <= 104 && MAX_ADDRESS_DIGITS < 32);
/// Corners buffered per welded vertex before it is treated as an overflow.
const GROUP_CAP: usize = 4096;

/// Normalized f32 position bits (signed zero canonical).
pub(crate) fn weld(p: [f32; 3]) -> [u32; 3] {
    p.map(|x| if x == 0. { 0 } else { x.to_bits() })
}

/// A leaf address packed most significant digit first: digit `i` at bits `125 - 3i`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Address {
    pub digits: u128,
    pub len: u8,
}

impl Address {
    pub(crate) fn of(node: &NodeRecord) -> Self {
        let mut digits = 0u128;
        let mut len = 0u8;
        for (count, bits) in [(node.cell_depth, node.cell), (node.path_len, node.path)] {
            for level in (0..count).rev() {
                let digit = ((bits >> (3 * u32::from(level))) & 7) as u128;
                digits |= digit << (125 - 3 * u32::from(len));
                len += 1;
            }
        }
        Self { digits, len }
    }
    /// Length of the common prefix.
    pub(crate) fn common(self, other: Self) -> u8 {
        let x = self.digits ^ other.digits;
        let first = if x == 0 {
            32
        } else {
            (x.leading_zeros() / 3) as u8
        };
        first.min(self.len).min(other.len)
    }
}

/// One corner seen from its welded vertex: the sort record of the pre-pass.
///
/// Encoded in 30 bytes (it was 63 with welded positions and a 17-byte address):
///
/// | bytes | field |
/// |---|---|
/// | 0..4 | `vertex`, the welded vertex ordinal (the group key) |
/// | 4..12 | `others`, the next and previous corners' vertex ordinals |
/// | 12..17 | `rank`, 40 bits |
/// | 17..30 | `address.digits >> ADDRESS_SHIFT` (93 bits) `<< 8`, then `address.len` (5 bits) `<< 3`, `corner` (2 bits) `<< 1` and `degenerate` |
///
/// [`Incidence::new`] refuses values outside these ranges; encoding never truncates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Incidence {
    vertex: u32,
    rank: u64,
    corner: u8,
    address: Address,
    others: [u32; 2],
    degenerate: bool,
}

impl Incidence {
    fn new(
        vertex: u32,
        rank: u64,
        corner: u8,
        address: Address,
        others: [u32; 2],
        degenerate: bool,
    ) -> Result<Self, JobError> {
        if rank >= RANK_LIMIT {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                format!("LOD pre-pass: triangle rank {rank} exceeds the 40-bit incidence rank"),
            ));
        }
        if address.len > MAX_ADDRESS_DIGITS || address.digits << (3 * u32::from(address.len)) != 0 {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                format!(
                    "LOD pre-pass: a {}-digit leaf address exceeds the {MAX_ADDRESS_DIGITS}-digit incidence address",
                    address.len
                ),
            ));
        }
        if corner > 2 {
            return Err(state("LOD pre-pass: corner index out of range"));
        }
        Ok(Self {
            vertex,
            rank,
            corner,
            address,
            others,
            degenerate,
        })
    }
}

impl Record for Incidence {
    const SIZE: usize = 30;
    fn encode(&self, out: &mut [u8]) {
        debug_assert!(self.rank < RANK_LIMIT && self.corner < 3);
        debug_assert!(self.address.len <= MAX_ADDRESS_DIGITS);
        out[..4].copy_from_slice(&self.vertex.to_le_bytes());
        out[4..8].copy_from_slice(&self.others[0].to_le_bytes());
        out[8..12].copy_from_slice(&self.others[1].to_le_bytes());
        out[12..17].copy_from_slice(&self.rank.to_le_bytes()[..5]);
        let word = (self.address.digits >> ADDRESS_SHIFT) << 8
            | u128::from(self.address.len) << 3
            | u128::from(self.corner) << 1
            | u128::from(self.degenerate);
        out[17..30].copy_from_slice(&word.to_le_bytes()[..13]);
    }
    fn decode(bytes: &[u8]) -> Self {
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let mut rank = [0u8; 8];
        rank[..5].copy_from_slice(&bytes[12..17]);
        let mut word = [0u8; 16];
        word[..13].copy_from_slice(&bytes[17..30]);
        let word = u128::from_le_bytes(word);
        Self {
            vertex: u32_at(0),
            rank: u64::from_le_bytes(rank),
            corner: ((word >> 1) & 3) as u8,
            address: Address {
                digits: (word >> 8) << ADDRESS_SHIFT,
                len: ((word >> 3) & 31) as u8,
            },
            others: [u32_at(4), u32_at(8)],
            degenerate: word & 1 != 0,
        }
    }
}

/// A corner's facts, back in rank order. Encoded in 8 bytes: the 40-bit rank, the corner
/// with the special flag (`corner | special << 2`), `c2` and `c3`. Its ranks come from
/// [`Incidence`] records, so they are within the same 40-bit bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CornerInfo {
    rank: u64,
    corner: u8,
    c2: u8,
    c3: u8,
    special: bool,
}

impl Record for CornerInfo {
    const SIZE: usize = 8;
    fn encode(&self, out: &mut [u8]) {
        debug_assert!(self.rank < RANK_LIMIT && self.corner < 3);
        out[..5].copy_from_slice(&self.rank.to_le_bytes()[..5]);
        out[5] = self.corner | u8::from(self.special) << 2;
        out[6] = self.c2;
        out[7] = self.c3;
    }
    fn decode(bytes: &[u8]) -> Self {
        let mut rank = [0u8; 8];
        rank[..5].copy_from_slice(&bytes[..5]);
        Self {
            rank: u64::from_le_bytes(rank),
            corner: bytes[5] & 3,
            c2: bytes[6],
            c3: bytes[7],
            special: bytes[5] & 4 != 0,
        }
    }
}

/// Per-corner vertex facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct VertexInfo {
    /// Exact shallowest incident leaf depth (H8 `CornerDepth`).
    pub ell: u8,
    pub c2: u8,
    pub c3: u8,
    pub special: bool,
}

impl VertexInfo {
    /// A cut vertex of a depth-`d` tile: shared with another depth-`d` tile.
    pub(crate) fn cut(&self, d: u32) -> bool {
        self.c2 != NEVER && u32::from(self.c2) <= d
    }
}

/// One source triangle as the LOD stages read it, in rank order.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LodSource {
    pub positions: [[f32; 3]; 3],
    pub node: u32,
    pub mesh: u32,
    pub primitive: u32,
    pub triangle: u64,
    pub expanded_ordinal: u64,
    pub material: u32,
    pub info: [VertexInfo; 3],
    pub degenerate: bool,
}

/// Identity order (provenance); the sequence itself is in rank order.
impl PartialEq for LodSource {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}
impl Eq for LodSource {}
impl PartialOrd for LodSource {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for LodSource {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.node,
            self.mesh,
            self.primitive,
            self.triangle,
            self.expanded_ordinal,
        )
            .cmp(&(
                other.node,
                other.mesh,
                other.primitive,
                other.triangle,
                other.expanded_ordinal,
            ))
    }
}

impl LodSource {
    pub(crate) fn primitive_key(&self) -> (u32, u32, u32) {
        (self.node, self.mesh, self.primitive)
    }
}

impl Record for LodSource {
    const SIZE: usize = 81;
    fn encode(&self, out: &mut [u8]) {
        let mut w = 0;
        let mut put = |bytes: &[u8]| {
            out[w..w + bytes.len()].copy_from_slice(bytes);
            w += bytes.len();
        };
        for p in self.positions {
            for v in p {
                put(&v.to_le_bytes());
            }
        }
        put(&self.node.to_le_bytes());
        put(&self.mesh.to_le_bytes());
        put(&self.primitive.to_le_bytes());
        put(&self.triangle.to_le_bytes());
        put(&self.expanded_ordinal.to_le_bytes());
        put(&self.material.to_le_bytes());
        for i in &self.info {
            put(&[i.ell, i.c2, i.c3, u8::from(i.special)]);
        }
        put(&[u8::from(self.degenerate)]);
    }
    fn decode(bytes: &[u8]) -> Self {
        let f = |i: usize| f32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let u = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let l = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
        let info = |i: usize| VertexInfo {
            ell: bytes[i],
            c2: bytes[i + 1],
            c3: bytes[i + 2],
            special: bytes[i + 3] != 0,
        };
        Self {
            positions: std::array::from_fn(|c| std::array::from_fn(|a| f(12 * c + 4 * a))),
            node: u(36),
            mesh: u(40),
            primitive: u(44),
            triangle: l(48),
            expanded_ordinal: l(56),
            material: u(64),
            info: [info(68), info(72), info(76)],
            degenerate: bytes[80] != 0,
        }
    }
}

fn state(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}

/// Scratch bytes the pre-pass needs at its peak (both sorts' spilled runs plus the slim
/// source), for a stage preflight.
pub(crate) fn scratch_need(triangles: u64) -> Option<u64> {
    let corners = triangles.checked_mul(3)?;
    let sort1 = SortConfig::scratch_need::<Incidence>(corners).ok()?;
    let sort2 = SortConfig::scratch_need::<CornerInfo>(corners).ok()?;
    let slim = triangles.checked_mul(LodSource::SIZE as u64)?;
    sort1.max(sort2.checked_add(slim)?).checked_add(slim)
}

/// Facts of one welded vertex from its (bounded) group of incidences.
fn reduce(key: u32, group: &[Incidence]) -> (u8, u8, bool) {
    let mut special = group.iter().any(|g| g.degenerate);
    // Each neighbour with the direction of the edge to it: `others` is (next, previous) in
    // the triangle's winding, so the edge runs out to the next and in from the previous.
    let mut others: Vec<(u32, bool)> = Vec::with_capacity(group.len() * 2);
    for g in group {
        for (o, out) in g.others.into_iter().zip([true, false]) {
            if o == key {
                special = true;
            }
            others.push((o, out));
        }
    }
    others.sort_unstable();
    let mut i = 0;
    while i < others.len() {
        let mut j = i + 1;
        while j < others.len() && others[j].0 == others[i].0 {
            j += 1;
        }
        // A manifold edge has two triangles traversing it in opposite directions; two that
        // traverse it the same way (a winding flip, or a same-winding duplicate face) leave
        // the edge's side unoriented, so the directed seam audit treats it like a
        // non-manifold edge.
        if j - i != 2 || others[i].1 == others[i + 1].1 {
            special = true;
        }
        i = j;
    }
    // Coincident faces (two triangles on the same three vertices, either winding: a
    // double-sided face). The tree may put them in different leaves (an over-budget key
    // group is split into rank-ordered runs, and both sides share a key), and then two
    // frontier tiles hold the same face as source; the seam audit's shared-face rule
    // excepts faces whose vertices are all special.
    let mut faces: Vec<[u32; 2]> = group
        .iter()
        .map(|g| {
            let [a, b] = g.others;
            [a.min(b), a.max(b)]
        })
        .collect();
    faces.sort_unstable();
    if faces.windows(2).any(|w| w[0] == w[1]) {
        special = true;
    }
    let mut addresses: Vec<Address> = group.iter().map(|g| g.address).collect();
    addresses.sort_unstable();
    addresses.dedup();
    let mut commons: Vec<u8> = addresses.windows(2).map(|w| w[0].common(w[1])).collect();
    commons.sort_unstable();
    let c2 = commons.first().map_or(NEVER, |c| c + 1);
    let c3 = commons.get(1).map_or(NEVER, |c| c + 1);
    (c2, c3, special)
}

/// Build the slim source: every source triangle in rank order with its corners' facts.
/// Consumes the corner vertex ordinals: their file is removed once the incidences are pushed.
pub(crate) fn build(
    scratch: &Scratch,
    sort: SortConfig,
    nodes: &Sequence<NodeRecord>,
    triangles: &Sequence<FullTriangle>,
    depths: &Sequence<CornerDepth>,
    vertices: Sequence<CornerVertices>,
    check: &dyn Fn() -> Result<(), JobError>,
) -> Result<Sequence<LodSource>, JobError> {
    let mut incidences = ExternalSorter::<Incidence>::new(scratch, sort)?;
    {
        let mut vertex_rows = vertices.reader()?;
        let mut leaves = nodes
            .reader()?
            .filter(|n| n.as_ref().map_or(true, |node| node.kind == NodeKind::Leaf));
        let mut leaf: Option<NodeRecord> = None;
        for (rank, triangle) in triangles.reader()?.enumerate() {
            let triangle = triangle?;
            let rank = rank as u64;
            if rank.is_multiple_of(1 << 16) {
                check()?;
            }
            while leaf.is_none_or(|l| l.end <= rank) {
                leaf = Some(
                    leaves
                        .next()
                        .transpose()?
                        .ok_or_else(|| state("LOD pre-pass: triangles outlive the leaves"))?,
                );
            }
            let leaf = leaf.unwrap();
            if leaf.first > rank {
                return Err(state("LOD pre-pass: leaf ranges skip a triangle"));
            }
            let address = Address::of(&leaf);
            let keys = vertex_rows
                .next()
                .transpose()?
                .ok_or_else(|| state("LOD pre-pass: corner vertices end early"))?
                .vertices;
            for corner in 0..3 {
                incidences.push(Incidence::new(
                    keys[corner],
                    rank,
                    corner as u8,
                    address,
                    [keys[(corner + 1) % 3], keys[(corner + 2) % 3]],
                    triangle.triangle.degenerate,
                )?)?;
            }
        }
        if vertex_rows.next().is_some() {
            return Err(state("LOD pre-pass: corner vertices outlive the triangles"));
        }
    }
    drop(vertices);
    let mut infos = ExternalSorter::<CornerInfo>::new(scratch, sort)?;
    {
        let mut group: Vec<Incidence> = Vec::new();
        let mut overflow = false;
        let mut current: Option<u32> = None;
        let flush = |group: &mut Vec<Incidence>,
                     overflow: bool,
                     infos: &mut ExternalSorter<CornerInfo>|
         -> Result<(), JobError> {
            let Some(first) = group.first() else {
                return Ok(());
            };
            let (c2, c3, special) = if overflow {
                (0, 0, true)
            } else {
                reduce(first.vertex, group)
            };
            for g in group.drain(..) {
                infos.push(CornerInfo {
                    rank: g.rank,
                    corner: g.corner,
                    c2,
                    c3,
                    special,
                })?;
            }
            Ok(())
        };
        for (index, incidence) in incidences.finish()?.enumerate() {
            let incidence = incidence?;
            if index.is_multiple_of(1 << 16) {
                check()?;
            }
            if current != Some(incidence.vertex) {
                flush(&mut group, overflow, &mut infos)?;
                overflow = false;
                current = Some(incidence.vertex);
            }
            group.push(incidence);
            if group.len() > GROUP_CAP {
                overflow = true;
                flush(&mut group, overflow, &mut infos)?;
            }
        }
        flush(&mut group, overflow, &mut infos)?;
    }
    let mut writer = SequenceWriter::<LodSource>::new(scratch, "lod-source", 1 << 20)?;
    {
        let mut infos = infos.finish()?;
        let mut depths = depths.reader()?;
        for (rank, triangle) in triangles.reader()?.enumerate() {
            let triangle = triangle?;
            let rank = rank as u64;
            if rank.is_multiple_of(1 << 16) {
                check()?;
            }
            let depth = depths
                .next()
                .transpose()?
                .ok_or_else(|| state("LOD pre-pass: corner depths end early"))?;
            let mut info = [VertexInfo::default(); 3];
            for (corner, slot) in info.iter_mut().enumerate() {
                let c = infos
                    .next()
                    .transpose()?
                    .ok_or_else(|| state("LOD pre-pass: corner facts end early"))?;
                if c.rank != rank || usize::from(c.corner) != corner {
                    return Err(state("LOD pre-pass: corner facts out of rank order"));
                }
                *slot = VertexInfo {
                    ell: depth.depths[corner],
                    c2: c.c2,
                    c3: c.c3,
                    special: c.special,
                };
            }
            let source = triangle.triangle.source;
            writer.push(&LodSource {
                positions: triangle.triangle.positions,
                node: source.node,
                mesh: source.mesh,
                primitive: source.primitive,
                triangle: source.triangle,
                expanded_ordinal: triangle.expanded_ordinal,
                material: triangle.material,
                info,
                degenerate: triangle.triangle.degenerate,
            })?;
        }
        if infos.next().is_some() || depths.next().is_some() {
            return Err(state("LOD pre-pass: corner facts outlive the triangles"));
        }
    }
    Ok(writer.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(digits: &[u8]) -> Address {
        let mut a = Address { digits: 0, len: 0 };
        for &d in digits {
            a.digits |= u128::from(d) << (125 - 3 * u32::from(a.len));
            a.len += 1;
        }
        a
    }
    fn inc(vertex: u32, others: [u32; 2], digits: &[u8], rank: u64) -> Incidence {
        Incidence {
            vertex,
            rank,
            corner: 0,
            address: address(digits),
            others,
            degenerate: false,
        }
    }

    #[test]
    fn common_prefix_counts_tiles_per_depth() {
        assert_eq!(address(&[1, 2, 3]).common(address(&[1, 2, 4])), 2);
        assert_eq!(address(&[1, 2]).common(address(&[1, 2, 0])), 2);
        assert_eq!(address(&[7; 31]).common(address(&[7; 31])), 31);
        assert_eq!(address(&[0]).common(address(&[1])), 0);
        // A vertex shared by leaves 1.2.3, 1.2.4 and 1.5: two tiles from depth 2, three
        // from depth 3.
        let k = 9;
        let ring = |a: u32, b: u32| [a, b];
        let (a, b, c, d) = (1, 2, 3, 4);
        let group = [
            inc(k, ring(a, b), &[1, 2, 3], 0),
            inc(k, ring(b, c), &[1, 2, 4], 1),
            inc(k, ring(c, d), &[1, 5], 2),
            inc(k, ring(d, a), &[1, 5], 3),
        ];
        let (c2, c3, special) = reduce(k, &group);
        assert_eq!((c2, c3, special), (2, 3, false));
        // An open fan: a and d are boundary edges.
        let (_, _, special) = reduce(k, &group[..3]);
        assert!(special);
        // One leaf only: never cut.
        let solo = [inc(k, ring(a, b), &[4], 0), inc(k, ring(b, a), &[4], 1)];
        assert_eq!(reduce(k, &solo).0, NEVER);
    }

    /// A closed fan whose triangles disagree in winding across one edge: every edge still
    /// has two triangles, but two traverse k-c the same way, so the vertex is special (the
    /// directed seam audit cannot orient that edge's side). A double-sided face (two
    /// coincident triangles, opposite windings) is consistently wound but special too: its
    /// two sides may lie in different leaves, and then two tiles hold the same source face.
    #[test]
    fn winding_flips_are_special() {
        let k = 9;
        let ring = |a: u32, b: u32| [a, b];
        let (a, b, c, d) = (1, 2, 3, 4);
        let consistent = [
            inc(k, ring(a, b), &[1, 2], 0),
            inc(k, ring(b, c), &[1, 2], 1),
            inc(k, ring(c, d), &[1, 3], 2),
            inc(k, ring(d, a), &[1, 3], 3),
        ];
        assert!(!reduce(k, &consistent).2);
        // (k, c, d) flipped to (k, d, c): k-c is now traversed k->c twice, k-d d->k twice.
        let mut flipped = consistent;
        flipped[2] = inc(k, ring(d, c), &[1, 3], 2);
        let (c2, _, special) = reduce(k, &flipped);
        assert!(special);
        assert_eq!(c2, 2, "cut depths are unchanged");
        // A double-sided triangle split over two leaves: consistently wound, but coincident.
        let pair = [
            inc(k, ring(a, b), &[1, 2], 0),
            inc(k, ring(b, a), &[1, 3], 1),
        ];
        assert!(reduce(k, &pair).2);
    }

    #[test]
    fn slim_source_round_trips() {
        let s = LodSource {
            positions: [[1., -0., 3.5], [f32::MIN_POSITIVE, 2., 1e30], [-7., 8., 9.]],
            node: 1,
            mesh: u32::MAX,
            primitive: 3,
            triangle: 1 << 60,
            expanded_ordinal: 5,
            material: u32::MAX,
            info: [
                VertexInfo {
                    ell: 3,
                    c2: 2,
                    c3: NEVER,
                    special: true,
                },
                VertexInfo::default(),
                VertexInfo {
                    ell: 31,
                    c2: 0,
                    c3: 0,
                    special: false,
                },
            ],
            degenerate: true,
        };
        let mut bytes = [0u8; LodSource::SIZE];
        s.encode(&mut bytes);
        let back = LodSource::decode(&bytes);
        assert_eq!(
            back.positions.map(|p| p.map(f32::to_bits)),
            s.positions.map(|p| p.map(f32::to_bits))
        );
        assert_eq!(back.info, s.info);
        assert_eq!(
            (back.triangle, back.mesh, back.degenerate),
            (s.triangle, s.mesh, true)
        );
    }

    fn round_trip<T: Record + std::fmt::Debug>(record: T) {
        let mut bytes = vec![0u8; T::SIZE];
        record.encode(&mut bytes);
        assert_eq!(T::decode(&bytes), record);
    }

    /// The 30-byte incidence and 8-byte corner facts keep every field at its packed limits:
    /// the largest vertex ordinals and rank, the deepest all-sevens address (31 digits) and
    /// the empty one, every corner and both flags.
    #[test]
    fn slim_records_round_trip_at_their_limits() {
        let deepest = address(&[7; MAX_ADDRESS_DIGITS as usize]);
        assert_eq!(deepest.len, 31);
        let addresses = [
            deepest,
            address(&[]),
            address(&[1]),
            address(&[0; MAX_ADDRESS_DIGITS as usize]),
            address(&[
                5, 0, 7, 1, 2, 3, 4, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3,
            ]),
        ];
        for address in addresses {
            for rank in [0, 1, 99, u64::from(u32::MAX) + 1, RANK_LIMIT - 1] {
                for corner in 0..3 {
                    for degenerate in [false, true] {
                        for (vertex, others) in [(0, [0, 0]), (u32::MAX, [u32::MAX, 1 << 31])] {
                            let i =
                                Incidence::new(vertex, rank, corner, address, others, degenerate)
                                    .unwrap();
                            round_trip(i);
                        }
                        for (c2, c3) in [(0, 0), (31, NEVER), (NEVER, NEVER), (1, 2)] {
                            round_trip(CornerInfo {
                                rank,
                                corner,
                                c2,
                                c3,
                                special: degenerate,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Values beyond the packed ranges are refused with a typed error, never truncated.
    #[test]
    fn incidence_refuses_values_beyond_its_packed_range() {
        let ok = address(&[3; 4]);
        let refused = |r: Result<Incidence, JobError>| r.unwrap_err().kind();
        assert_eq!(
            refused(Incidence::new(1, RANK_LIMIT, 0, ok, [2, 3], false)),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            refused(Incidence::new(1, u64::MAX, 0, ok, [2, 3], false)),
            JobErrorKind::Unsupported
        );
        let too_deep = address(&[7; MAX_ADDRESS_DIGITS as usize + 1]);
        assert_eq!(
            refused(Incidence::new(1, 0, 0, too_deep, [2, 3], false)),
            JobErrorKind::Unsupported
        );
        // Digits past the stated length are not an address this record can carry.
        let stray = Address {
            digits: 1 << 40,
            len: 2,
        };
        assert_eq!(
            refused(Incidence::new(1, 0, 0, stray, [2, 3], false)),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            refused(Incidence::new(1, 0, 3, ok, [2, 3], false)),
            JobErrorKind::InvalidState
        );
    }

    #[test]
    fn vertex_ordinals_refuse_past_32_bits() {
        use super::super::super::full::vertex_ordinal;
        assert_eq!(vertex_ordinal(u64::from(u32::MAX)).unwrap(), u32::MAX);
        assert_eq!(
            vertex_ordinal(u64::from(u32::MAX) + 1).unwrap_err().kind(),
            JobErrorKind::Unsupported
        );
    }
}
