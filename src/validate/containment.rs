//! Exact decoded-content containment: the normative static span bound, quaternion admission,
//! the node composite and the per-vertex frame walk (C2 contract v3.2, "Exact evaluation").
// Removed when F wires the operation into `validate.rs`.
#![allow(dead_code)]
mod dyadic;

/// Raw POSITION word type; it selects the span-table pair even for unnormalized words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Word {
    F32,
    Int8,
    Int16,
}
/// `k = 1, 127, 255, 32767, 65535`; signed variants clamp raw values at or below `−k` to `−1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Normalization {
    None(Word),
    I8,
    U8,
    I16,
    U16,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RawVertex {
    F32([f32; 3]),
    Int([i32; 3]),
}
/// A selected node's local transform; `Skip` has none of `matrix`, `translation`, `rotation`, `scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Local {
    Skip,
    Matrix([f64; 16]),
    Trs {
        t: [f64; 3],
        q: [f64; 4],
        s: [f64; 3],
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum VolumeFacts {
    Box([f64; 12]),
    Sphere([f64; 4]),
    Region,
}
/// `levels[l].to_parent` steps from frame `l − 1` into frame `l`; `levels[0].to_parent` is `None`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LevelFacts {
    pub(crate) to_parent: Option<[f64; 16]>,
    pub(crate) volumes: [Option<(Role, VolumeFacts)>; 3],
}
/// Test order within a level is the declaration order; `k` is the level index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Role {
    Own,
    Content,
    ExternalContent(u32),
    Ancestor(u32),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpanCaps {
    pub(crate) span_bits: u64,
    pub(crate) node_depth: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exclusion {
    NumericSpan,
    RegionVolume,
    RotatedBox,
}

/// Exponent bounds `(lo, hi)` of the set bits of a nonzero value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pair(pub(crate) i32, pub(crate) i32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bound {
    Zero,
    P(Pair),
}
pub(crate) type Composite = [[Bound; 4]; 4];
/// Per-URI hull (D4): level-0 `P₀, P₁, P₂` after U and `W`, plus `sInst` and `sPos`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UriBounds {
    pub(crate) pair0: [Bound; 4],
    pub(crate) s_inst: u32,
    pub(crate) s_pos: u32,
}
impl UriBounds {
    /// The hull identity: `hull(&UriBounds::identity(), b) == b`.
    pub(crate) fn identity() -> Self {
        todo!()
    }
}
/// The composite before the first non-skipped node.
pub(crate) fn composite_identity() -> Composite {
    todo!()
}
pub(crate) fn node_step_bound(g: &Composite, l: &Local) -> (Composite, u32) {
    let _ = (g, l);
    todo!()
}
pub(crate) fn position_bound(g: &Composite, n: Normalization) -> ([Bound; 4], u32) {
    let _ = (g, n);
    todo!()
}
pub(crate) fn hull(a: &UriBounds, b: &UriBounds) -> UriBounds {
    let _ = (a, b);
    todo!()
}
pub(crate) fn volume_exclusion(v: &VolumeFacts) -> Option<Exclusion> {
    let _ = v;
    todo!()
}
/// Non-allocating view of `s_0 … s_L`, recomputed from the hull on iteration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SmallSpanList<'a> {
    u: UriBounds,
    levels: &'a [LevelFacts],
}
impl SmallSpanList<'_> {
    pub(crate) fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        std::iter::empty()
    }
}
pub(crate) fn level_spans<'a>(
    u: &UriBounds,
    levels: &'a [LevelFacts],
    caps: SpanCaps,
) -> Result<SmallSpanList<'a>, Exclusion> {
    let _ = (u, levels, caps);
    todo!()
}
pub(crate) fn vertex_units(t_count: u64, spans: &SmallSpanList) -> Option<u64> {
    let _ = (t_count, spans);
    todo!()
}
pub(crate) fn node_units(s_step: u32) -> u64 {
    let _ = s_step;
    todo!()
}

/// Host refusal of the dyadic arena reservation; F maps it to ResourceLimit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AllocRefused;
pub(crate) struct Arena {
    d: dyadic::Dyadics,
}
impl Arena {
    /// `16·(node_depth + 1) + 32` values of `⌈(span_bits + 64)/64⌉` limbs plus the admission region.
    pub(crate) fn reserve(caps: SpanCaps) -> Result<Arena, AllocRefused> {
        let _ = caps;
        todo!()
    }
    /// Sticky capacity-invariant breach; F maps it to InvalidState, never to a verdict.
    pub(crate) fn fault(&self) -> bool {
        todo!()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RotationOutcome {
    Ok,
    InvalidZeroOrRange,
    QuaternionWindow,
}
pub(crate) fn admit_rotation(q: [f64; 4], arena: &mut Arena) -> RotationOutcome {
    let _ = (q, arena);
    todo!()
}
pub(crate) fn push_node(arena: &mut Arena, l: &Local) {
    let _ = (arena, l);
    todo!()
}
pub(crate) fn pop_node(arena: &mut Arena) {
    let _ = arena;
    todo!()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Contained,
    Outside { role: Role, level: u32 },
}
pub(crate) fn walk(
    arena: &mut Arena,
    n: Normalization,
    raw: RawVertex,
    levels: &[LevelFacts],
) -> Verdict {
    let _ = (arena, n, raw, levels);
    todo!()
}
