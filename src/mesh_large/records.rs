//! Spill records of the capture, sort and cell-tree passes, with platform-independent
//! little-endian encodings. Every sort order is a strict total order on its key, so sorted
//! output never depends on input order, run size, fan-in or jobs.
use crate::spill::Record;
use serde::Serialize;
use std::cmp::Ordering;

/// Key depth: 21 bits per axis, 63-bit Morton key; cell depths run 0..=21 (design §2.4).
pub(crate) const KEY_DEPTH: u8 = 21;
const AXIS_CELLS: f64 = (1u64 << KEY_DEPTH) as f64;
const AXIS_MAX: u64 = (1 << KEY_DEPTH) - 1;

/// Spreads the low 21 bits of `v` to every third bit.
fn spread(v: u64) -> u64 {
    let mut x = v & AXIS_MAX;
    x = (x | x << 32) & 0x001f_0000_0000_ffff;
    x = (x | x << 16) & 0x001f_0000_ff00_00ff;
    x = (x | x << 8) & 0x100f_00f0_0f00_f00f;
    x = (x | x << 4) & 0x10c3_0c30_c30c_30c3;
    x = (x | x << 2) & 0x1249_2492_4924_9249;
    x
}

/// Morton key of axis cells: the octant at each depth is `x + 2y + 4z` of that depth's bits.
pub(crate) fn morton(cell: [u64; 3]) -> u64 {
    spread(cell[0]) | spread(cell[1]) << 1 | spread(cell[2]) << 2
}

/// The key prefix naming `key`'s cell at `depth`.
pub(crate) fn prefix(key: u64, depth: u8) -> u64 {
    key >> (3 * u32::from(KEY_DEPTH - depth))
}

/// The isotropic cube over the snapshot box: equal sides, so cells are cubes. A zero side
/// (every position equal) puts every triangle in key 0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct Cube {
    pub origin: [f64; 3],
    pub side: f64,
}

impl Cube {
    pub(crate) fn around(bounds: &Bounds) -> Self {
        let origin = bounds.min.map(f64::from);
        let side = (0..3)
            .map(|axis| f64::from(bounds.max[axis]) - origin[axis])
            .fold(0., f64::max);
        Self { origin, side }
    }

    /// Key of a triangle's centroid, evaluated in f64 from its f32 corners.
    pub(crate) fn key(&self, positions: &[[f32; 3]; 3]) -> u64 {
        if self.side == 0. {
            return 0;
        }
        let cell = std::array::from_fn(|axis| {
            let centroid = (f64::from(positions[0][axis])
                + f64::from(positions[1][axis])
                + f64::from(positions[2][axis]))
                / 3.;
            let scaled = ((centroid - self.origin[axis]) / self.side * AXIS_CELLS).floor();
            if scaled <= 0. {
                0
            } else {
                (scaled as u64).min(AXIS_MAX)
            }
        });
        morton(cell)
    }
}

/// An axis-aligned box of f32 positions; unions of f32 extrema are exact.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    pub(crate) const EMPTY: Self = Self {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };

    pub(crate) fn include(&mut self, point: [f32; 3]) {
        for axis in 0..3 {
            self.min[axis] = self.min[axis].min(point[axis]);
            self.max[axis] = self.max[axis].max(point[axis]);
        }
    }

    pub(crate) fn union(&mut self, other: &Self) {
        for axis in 0..3 {
            self.min[axis] = self.min[axis].min(other.min[axis]);
            self.max[axis] = self.max[axis].max(other.max[axis]);
        }
    }

    pub(crate) fn extent(&self) -> [f32; 3] {
        std::array::from_fn(|axis| (self.max[axis] - self.min[axis]).max(0.))
    }
}

struct Writer<'a>(&'a mut [u8], usize);
impl Writer<'_> {
    fn bytes(&mut self, value: &[u8]) {
        self.0[self.1..self.1 + value.len()].copy_from_slice(value);
        self.1 += value.len();
    }
    fn u8(&mut self, v: u8) {
        self.bytes(&[v]);
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32s(&mut self, v: &[f32]) {
        for x in v {
            self.bytes(&x.to_le_bytes());
        }
    }
}

struct Reader<'a>(&'a [u8], usize);
impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let out = self.0[self.1..self.1 + N].try_into().unwrap();
        self.1 += N;
        out
    }
    fn u8(&mut self) -> u8 {
        self.take::<1>()[0]
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }
    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take())
    }
    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.take())
    }
    fn vec3(&mut self) -> [f32; 3] {
        [self.f32(), self.f32(), self.f32()]
    }
}

/// Pass 1: one triangle corner's vertex reference. Sorted by (scene primitive, vertex) for the
/// sequential POSITION join.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CornerRef {
    /// Ordinal of the instanced primitive in scene traversal order.
    pub primitive: u32,
    pub vertex: u32,
    pub triangle: u32,
    pub corner: u8,
}

impl Record for CornerRef {
    const SIZE: usize = 13;
    fn encode(&self, out: &mut [u8]) {
        let mut w = Writer(out, 0);
        w.u32(self.primitive);
        w.u32(self.vertex);
        w.u32(self.triangle);
        w.u8(self.corner);
    }
    fn decode(bytes: &[u8]) -> Self {
        let mut r = Reader(bytes, 0);
        Self {
            primitive: r.u32(),
            vertex: r.u32(),
            triangle: r.u32(),
            corner: r.u8(),
        }
    }
}

/// Pass 2: a corner with its baked position, sorted back by (primitive, triangle, corner).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CornerPosition {
    pub primitive: u32,
    pub triangle: u32,
    pub corner: u8,
    pub vertex: u32,
    pub position: [f32; 3],
}

impl CornerPosition {
    fn key(&self) -> (u32, u32, u8) {
        (self.primitive, self.triangle, self.corner)
    }
}
impl PartialEq for CornerPosition {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}
impl Eq for CornerPosition {}
impl PartialOrd for CornerPosition {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for CornerPosition {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
    }
}

impl Record for CornerPosition {
    const SIZE: usize = 25;
    fn encode(&self, out: &mut [u8]) {
        let mut w = Writer(out, 0);
        w.u32(self.primitive);
        w.u32(self.triangle);
        w.u8(self.corner);
        w.u32(self.vertex);
        w.f32s(&self.position);
    }
    fn decode(bytes: &[u8]) -> Self {
        let mut r = Reader(bytes, 0);
        Self {
            primitive: r.u32(),
            triangle: r.u32(),
            corner: r.u8(),
            vertex: r.u32(),
            position: r.vec3(),
        }
    }
}

/// Exact per-triangle source provenance (F1c2 identity): node, mesh, primitive and the
/// triangle's index within its primitive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub(crate) struct Provenance {
    pub node: u32,
    pub mesh: u32,
    pub primitive: u32,
    pub triangle: u32,
}

/// Passes 3–5a: a self-contained triangle. Sorted by (Morton key, provenance); the rank of a
/// record in that order is its position in the cell tree's rank space.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TriangleRecord {
    pub key: u64,
    pub source: Provenance,
    /// Authored POSITION accessor indices, in the reflected corner order of `positions`.
    pub vertices: [u32; 3],
    /// Positions after the node bake, in local metres; −0 is stored as +0, so f32-bit equality
    /// (the weld key) is value equality.
    pub positions: [[f32; 3]; 3],
    /// Projected tile bytes of this triangle (§2.4); the leaf byte budget sums them.
    pub bytes: u32,
    /// Two corners share a welded (f32-bit) position. The triangle stays in its leaf for exact
    /// provenance; adjacency passes skip it.
    pub degenerate: bool,
}

impl TriangleRecord {
    pub(crate) fn bounds(&self) -> Bounds {
        let mut bounds = Bounds::EMPTY;
        for p in self.positions {
            bounds.include(p);
        }
        bounds
    }
}

impl PartialEq for TriangleRecord {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for TriangleRecord {}
impl PartialOrd for TriangleRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TriangleRecord {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.key, self.source).cmp(&(other.key, other.source))
    }
}

impl Record for TriangleRecord {
    const SIZE: usize = 77;
    fn encode(&self, out: &mut [u8]) {
        let mut w = Writer(out, 0);
        w.u64(self.key);
        for v in [
            self.source.node,
            self.source.mesh,
            self.source.primitive,
            self.source.triangle,
        ] {
            w.u32(v);
        }
        for v in self.vertices {
            w.u32(v);
        }
        for p in &self.positions {
            w.f32s(p);
        }
        w.u32(self.bytes);
        w.u8(u8::from(self.degenerate));
    }
    fn decode(bytes: &[u8]) -> Self {
        let mut r = Reader(bytes, 0);
        Self {
            key: r.u64(),
            source: Provenance {
                node: r.u32(),
                mesh: r.u32(),
                primitive: r.u32(),
                triangle: r.u32(),
            },
            vertices: [r.u32(), r.u32(), r.u32()],
            positions: [r.vec3(), r.vec3(), r.vec3()],
            bytes: r.u32(),
            degenerate: r.u8() != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NodeKind {
    Internal,
    Leaf,
}

/// One tile of the cell tree.
///
/// Identity (design §2.4, N4):
/// - a cell: `(cell_depth, cell)` with `path_len = 0`;
/// - a packed leaf: `(parent cell, first octant)`, stored as its first octant's cell, with
///   `last_octant` the last octant it covers; an unpacked leaf has `last_octant == slot`;
/// - a run sub-hierarchy node: `(degenerate cell, run path)`, the path being `path_len` 3-bit
///   synthetic slots, first slot most significant.
///
/// Every node covers the contiguous rank range `[first, end)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NodeRecord {
    pub cell_depth: u8,
    pub cell: u64,
    pub path_len: u8,
    pub path: u64,
    pub last_octant: u8,
    pub kind: NodeKind,
    pub children: u8,
    pub first: u64,
    pub end: u64,
    pub bytes: u64,
    pub bounds: Bounds,
}

impl NodeRecord {
    /// Tile depth: the LOD step and lattice depth (`21 + level` inside run sub-hierarchies).
    pub(crate) fn depth(&self) -> u32 {
        u32::from(self.cell_depth) + u32::from(self.path_len)
    }

    /// Slot under its parent: the (first) octant, or the last synthetic run slot.
    pub(crate) fn slot(&self) -> u8 {
        if self.path_len > 0 {
            (self.path & 7) as u8
        } else if self.cell_depth == 0 {
            0
        } else {
            (self.cell & 7) as u8
        }
    }

    pub(crate) fn triangles(&self) -> u64 {
        self.end - self.first
    }

    pub(crate) fn is_packed(&self) -> bool {
        self.kind == NodeKind::Leaf && self.path_len == 0 && self.last_octant != self.slot()
    }

    /// Identity of the parent tile, or `None` for the root.
    pub(crate) fn parent(&self) -> Option<(u8, u64, u8, u64)> {
        if self.path_len > 0 {
            return Some((self.cell_depth, self.cell, self.path_len - 1, self.path >> 3));
        }
        (self.cell_depth > 0).then(|| (self.cell_depth - 1, self.cell >> 3, 0, 0))
    }

    pub(crate) fn identity(&self) -> (u8, u64, u8, u64) {
        (self.cell_depth, self.cell, self.path_len, self.path)
    }
}

/// Preorder: an ancestor starts at or before its descendants' first rank and is shallower;
/// siblings have disjoint ranges. `(first, depth)` is therefore unique and orders a preorder.
impl Eq for NodeRecord {}
impl PartialOrd for NodeRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for NodeRecord {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.first, self.depth()).cmp(&(other.first, other.depth()))
    }
}

impl Record for NodeRecord {
    const SIZE: usize = 72;
    fn encode(&self, out: &mut [u8]) {
        let mut w = Writer(out, 0);
        w.u8(self.cell_depth);
        w.u64(self.cell);
        w.u8(self.path_len);
        w.u64(self.path);
        w.u8(self.last_octant);
        w.u8(match self.kind {
            NodeKind::Internal => 0,
            NodeKind::Leaf => 1,
        });
        w.u8(self.children);
        w.u64(self.first);
        w.u64(self.end);
        w.u64(self.bytes);
        w.f32s(&self.bounds.min);
        w.f32s(&self.bounds.max);
        w.bytes(&[0; 3]);
    }
    fn decode(bytes: &[u8]) -> Self {
        let mut r = Reader(bytes, 0);
        Self {
            cell_depth: r.u8(),
            cell: r.u64(),
            path_len: r.u8(),
            path: r.u64(),
            last_octant: r.u8(),
            kind: if r.u8() == 0 {
                NodeKind::Internal
            } else {
                NodeKind::Leaf
            },
            children: r.u8(),
            first: r.u64(),
            end: r.u64(),
            bytes: r.u64(),
            bounds: Bounds {
                min: r.vec3(),
                max: r.vec3(),
            },
        }
    }
}

/// Totals of a contiguous rank range: a cell, a candidate, a packed run or a degenerate run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Span {
    pub count: u64,
    pub bytes: u64,
    pub first: u64,
    pub end: u64,
    pub bounds: Bounds,
}

impl Span {
    pub(crate) fn empty(first: u64) -> Self {
        Self {
            count: 0,
            bytes: 0,
            first,
            end: first,
            bounds: Bounds::EMPTY,
        }
    }

    /// Appends the following contiguous span.
    pub(crate) fn extend(&mut self, next: &Self) {
        debug_assert_eq!(self.end, next.first);
        self.count += next.count;
        self.bytes += next.bytes;
        self.end = next.end;
        self.bounds.union(&next.bounds);
    }
}

impl Eq for Span {}
impl PartialOrd for Span {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Span {
    fn cmp(&self, other: &Self) -> Ordering {
        self.first.cmp(&other.first)
    }
}

impl Record for Span {
    const SIZE: usize = 56;
    fn encode(&self, out: &mut [u8]) {
        let mut w = Writer(out, 0);
        w.u64(self.count);
        w.u64(self.bytes);
        w.u64(self.first);
        w.u64(self.end);
        w.f32s(&self.bounds.min);
        w.f32s(&self.bounds.max);
    }
    fn decode(bytes: &[u8]) -> Self {
        let mut r = Reader(bytes, 0);
        Self {
            count: r.u64(),
            bytes: r.u64(),
            first: r.u64(),
            end: r.u64(),
            bounds: Bounds {
                min: r.vec3(),
                max: r.vec3(),
            },
        }
    }
}
