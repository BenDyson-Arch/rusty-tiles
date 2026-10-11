//! Exact decoded-content containment: the normative static span bound, quaternion admission,
//! the node composite and the per-vertex frame walk (C2 contract v3.2, "Exact evaluation").
// Removed when F wires the operation into `validate.rs`.
#![allow(dead_code)]
mod dyadic;
use dyadic::{Dyadics, D};
use std::cmp::Ordering;

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
/// Declaration order is precedence: shape exclusions are reported before `NumericSpan`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Exclusion {
    RegionVolume,
    RotatedBox,
    NumericSpan,
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

// ---- static bound: the normative pair arithmetic ----

const ONE: Bound = Bound::P(Pair(0, 0));
const TWO: Bound = Bound::P(Pair(1, 1));
/// A pair that is over every cap.
const SATURATED: Bound = Bound::P(Pair(i32::MIN, i32::MAX));

impl Bound {
    /// The exact pair of a binary64 input: `(lowbit(x), ⌊log2|x|⌋)`, and ZERO for `±0`.
    pub(crate) fn of(x: f64) -> Bound {
        if x == 0.0 {
            return Bound::Zero;
        }
        if !x.is_finite() {
            return SATURATED;
        }
        let bits = x.to_bits();
        let (e, f) = (((bits >> 52) & 0x7ff) as i32, bits & ((1 << 52) - 1));
        let (m, e) = if e == 0 {
            (f, -1074)
        } else {
            (f | 1 << 52, e - 1075)
        };
        Bound::P(Pair(
            e + m.trailing_zeros() as i32,
            e + 63 - m.leading_zeros() as i32,
        ))
    }
    /// `hi − lo + 1`, ZERO is 0, and a saturated pair is `u32::MAX` (over every cap).
    pub(crate) fn span(self) -> u32 {
        match self {
            Bound::Zero => 0,
            Bound::P(Pair(lo, hi)) if lo == i32::MIN || hi == i32::MAX => u32::MAX,
            Bound::P(Pair(lo, hi)) => {
                u32::try_from(i64::from(hi) - i64::from(lo) + 1).unwrap_or(u32::MAX)
            }
        }
    }
    fn mul(self, b: Bound) -> Bound {
        match (self, b) {
            (Bound::Zero, _) | (_, Bound::Zero) => Bound::Zero,
            (ONE, p) | (p, ONE) => p,
            (Bound::P(Pair(la, ha)), Bound::P(Pair(lb, hb))) => Bound::P(Pair(
                la.saturating_add(lb),
                ha.saturating_add(hb).saturating_add(1),
            )),
        }
    }
    /// Entrywise hull; ZERO ∪ p = p.
    fn hull(self, b: Bound) -> Bound {
        match (self, b) {
            (Bound::Zero, p) | (p, Bound::Zero) => p,
            (Bound::P(Pair(la, ha)), Bound::P(Pair(lb, hb))) => {
                Bound::P(Pair(la.min(lb), ha.max(hb)))
            }
        }
    }
}
/// Flat sum over the written terms (negation leaves a pair unchanged).
fn flat_sum(terms: &[Bound]) -> Bound {
    let mut m = 0u32;
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for t in terms {
        if let Bound::P(Pair(l, h)) = *t {
            (m, lo, hi) = (m + 1, lo.min(l), hi.max(h));
        }
    }
    match m {
        0 => Bound::Zero,
        1 => Bound::P(Pair(lo, hi)),
        m => Bound::P(Pair(
            lo,
            hi.saturating_add((u32::BITS - (m - 1).leading_zeros()) as i32),
        )),
    }
}
/// Records the span of every counted product and flat-sum result.
struct Spans(u32);
impl Spans {
    fn mul(&mut self, a: Bound, b: Bound) -> Bound {
        self.count(a.mul(b))
    }
    fn sum(&mut self, terms: &[Bound]) -> Bound {
        self.count(flat_sum(terms))
    }
    fn count(&mut self, b: Bound) -> Bound {
        self.0 = self.0.max(b.span());
        b
    }
}
fn over(s: u32, span_bits: u64) -> bool {
    s == u32::MAX || u64::from(s) > span_bits
}

impl UriBounds {
    /// The hull identity: `hull(&UriBounds::identity(), b) == b`.
    pub(crate) fn identity() -> Self {
        Self {
            pair0: [Bound::Zero; 4],
            s_inst: 0,
            s_pos: 0,
        }
    }
}
/// The composite before the first non-skipped node: diagonal `(0, 0)`, ZERO elsewhere.
pub(crate) fn composite_identity() -> Composite {
    std::array::from_fn(|i| std::array::from_fn(|j| if i == j { ONE } else { Bound::Zero }))
}
fn trs_bound(s: &mut Spans, t: [f64; 3], q: [f64; 4], sc: [f64; 3]) -> Composite {
    let [x, y, z, w] = q.map(Bound::of);
    let (xx, yy, zz, ww) = (s.mul(x, x), s.mul(y, y), s.mul(z, z), s.mul(w, w));
    let n = s.sum(&[xx, yy, zz, ww]);
    // H00, H11 and H22 are flat sums of the same four pairs (negation leaves a pair unchanged).
    let diag = s.sum(&[ww, xx, yy, zz]);
    let mut off = |a: (Bound, Bound), b: (Bound, Bound)| {
        let terms = [s.mul(a.0, a.1), s.mul(b.0, b.1)];
        let sum = s.sum(&terms);
        s.mul(TWO, sum)
    };
    let h = [
        [diag, off((x, y), (w, z)), off((x, z), (w, y))],
        [off((x, y), (w, z)), diag, off((y, z), (w, x))],
        [off((x, z), (w, y)), off((y, z), (w, x)), diag],
    ];
    let mut l = [[Bound::Zero; 4]; 4];
    for r in 0..3 {
        for c in 0..3 {
            l[r][c] = s.mul(h[r][c], Bound::of(sc[c]));
        }
        l[r][3] = s.mul(n, Bound::of(t[r]));
    }
    l[3][3] = n;
    l
}
fn matrix_bound(m: [f64; 16]) -> Composite {
    let mut l = [[Bound::Zero; 4]; 4];
    for (r, row) in l.iter_mut().take(3).enumerate() {
        for (c, e) in row.iter_mut().enumerate() {
            *e = Bound::of(m[4 * c + r]);
        }
    }
    l[3][3] = ONE;
    l
}
/// `G ← G·L` with `s_step` over the node's L quantities, its `G·L` products and the new `G`.
/// `Skip` returns `G` unchanged and `s_step = 0`.
pub(crate) fn node_step_bound(g: &Composite, l: &Local) -> (Composite, u32) {
    let mut s = Spans(0);
    let l = match *l {
        Local::Skip => return (*g, 0),
        Local::Matrix(m) => matrix_bound(m),
        Local::Trs { t, q, s: sc } => trs_bound(&mut s, t, q, sc),
    };
    l.iter().flatten().for_each(|&e| {
        s.count(e);
    });
    let mut out = [[Bound::Zero; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, e) in row.iter_mut().enumerate() {
            let terms: [Bound; 4] = std::array::from_fn(|k| s.mul(g[i][k], l[k][j]));
            *e = s.sum(&terms);
        }
    }
    (out, s.0)
}
fn word_and_k(n: Normalization) -> (Bound, Bound, i32, bool) {
    let p = |lo, hi| Bound::P(Pair(lo, hi));
    match n {
        Normalization::None(Word::F32) => (p(-149, 127), ONE, 1, false),
        Normalization::None(Word::Int8) => (p(0, 7), ONE, 1, false),
        Normalization::None(Word::Int16) => (p(0, 15), ONE, 1, false),
        Normalization::I8 => (p(0, 7), p(0, 6), 127, true),
        Normalization::U8 => (p(0, 7), p(0, 7), 255, false),
        Normalization::I16 => (p(0, 15), p(0, 14), 32767, true),
        Normalization::U16 => (p(0, 15), p(0, 15), 65535, false),
    }
}
/// Level 0: `P_r = Σ[G[r][0]·a, G[r][1]·a, G[r][2]·a, G[r][3]·k]`, `W = G[3][3]·k`, then U.
/// Returns `[P₀, P₁, P₂, W]` after U and their span (`sPos`).
pub(crate) fn position_bound(g: &Composite, n: Normalization) -> ([Bound; 4], u32) {
    let (a, k, _, _) = word_and_k(n);
    let mut s = Spans(0);
    let mut p = [Bound::Zero; 3];
    for (r, pr) in p.iter_mut().enumerate() {
        let terms = [
            s.mul(g[r][0], a),
            s.mul(g[r][1], a),
            s.mul(g[r][2], a),
            s.mul(g[r][3], k),
        ];
        *pr = s.sum(&terms);
    }
    let w = s.mul(g[3][3], k);
    ([p[0], p[2], p[1], w], s.0)
}
pub(crate) fn hull(a: &UriBounds, b: &UriBounds) -> UriBounds {
    UriBounds {
        pair0: std::array::from_fn(|i| a.pair0[i].hull(b.pair0[i])),
        s_inst: a.s_inst.max(b.s_inst),
        s_pos: a.s_pos.max(b.s_pos),
    }
}
/// Row `j`'s half-length: the covering signed-permutation entry, or 0.
fn half(b: &[f64; 12], j: usize) -> f64 {
    (0..3)
        .map(|i| b[3 + 3 * i + j])
        .find(|&e| e != 0.0)
        .unwrap_or(0.0)
}
/// The single definition of the slice-1 volume shapes: regions are excluded, and a box is
/// admitted only when each half-axis column has at most one nonzero entry, in distinct rows.
pub(crate) fn volume_exclusion(v: &VolumeFacts) -> Option<Exclusion> {
    match v {
        VolumeFacts::Region => Some(Exclusion::RegionVolume),
        VolumeFacts::Sphere(_) => None,
        VolumeFacts::Box(b) => {
            let mut rows = 0u8;
            for i in 0..3 {
                let column = &b[3 + 3 * i..6 + 3 * i];
                let mut nonzero = (0..3).filter(|&j| column[j] != 0.0);
                if let Some(j) = nonzero.next() {
                    if nonzero.next().is_some() || rows & 1 << j != 0 {
                        return Some(Exclusion::RotatedBox);
                    }
                    rows |= 1 << j;
                }
            }
            None
        }
    }
}
fn levels_volumes(levels: &[LevelFacts]) -> impl Iterator<Item = &VolumeFacts> {
    levels
        .iter()
        .flat_map(|l| l.volumes.iter().flatten().map(|(_, v)| v))
}
/// One level's span: its tile step (`l > 0`), `W`, and its volume quantities.
fn level_span(p: &mut [Bound; 3], w: Bound, level: &LevelFacts, step: bool) -> u32 {
    let mut s = Spans(0);
    s.count(w);
    if let Some(t) = level.to_parent.filter(|_| step) {
        let q = *p;
        for (r, pr) in p.iter_mut().enumerate() {
            let terms = [
                s.mul(Bound::of(t[r]), q[0]),
                s.mul(Bound::of(t[4 + r]), q[1]),
                s.mul(Bound::of(t[8 + r]), q[2]),
                s.mul(Bound::of(t[12 + r]), w),
            ];
            *pr = s.sum(&terms);
        }
    }
    for (_, v) in level.volumes.iter().flatten() {
        let c = match v {
            VolumeFacts::Box(b) => &b[..3],
            VolumeFacts::Sphere(b) => &b[..3],
            VolumeFacts::Region => continue,
        };
        let mut q = [Bound::Zero; 3];
        for j in 0..3 {
            let cw = s.mul(Bound::of(c[j]), w);
            let d = s.sum(&[p[j], cw]);
            match v {
                VolumeFacts::Box(b) => {
                    s.mul(Bound::of(half(b, j)), w);
                }
                _ => q[j] = s.mul(d, d),
            }
        }
        if let VolumeFacts::Sphere(sp) = v {
            s.sum(&q);
            let (r2, w2) = (s.mul(Bound::of(sp[3]), Bound::of(sp[3])), s.mul(w, w));
            s.mul(r2, w2);
        }
    }
    s.0
}
/// Non-allocating view of `s_0 … s_L`, recomputed from the hull on iteration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SmallSpanList<'a> {
    u: UriBounds,
    levels: &'a [LevelFacts],
}
impl SmallSpanList<'_> {
    pub(crate) fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        let [p0, p1, p2, w] = self.u.pair0;
        let mut p = [p0, p1, p2];
        let s_pos = self.u.s_pos;
        self.levels.iter().enumerate().map(move |(l, level)| {
            let s = level_span(&mut p, w, level, l > 0);
            if l == 0 {
                s.max(s_pos)
            } else {
                s
            }
        })
    }
}
/// Region and rotated-box exclusions first, then `numericSpan` over `sInst` and `s_0 … s_L`.
pub(crate) fn level_spans<'a>(
    u: &UriBounds,
    levels: &'a [LevelFacts],
    caps: SpanCaps,
) -> Result<SmallSpanList<'a>, Exclusion> {
    if let Some(e) = levels_volumes(levels).filter_map(volume_exclusion).min() {
        return Err(e);
    }
    let spans = SmallSpanList { u: *u, levels };
    if over(u.s_inst, caps.span_bits) || spans.iter().any(|s| over(s, caps.span_bits)) {
        return Err(Exclusion::NumericSpan);
    }
    Ok(spans)
}
fn limbs_squared(s: u32) -> u64 {
    u64::from(s).div_ceil(64).pow(2)
}
/// `T·Σ_ℓ ⌈s_ℓ/64⌉²`, checked.
pub(crate) fn vertex_units(t_count: u64, spans: &SmallSpanList) -> Option<u64> {
    spans
        .iter()
        .try_fold(0u64, |a, s| a.checked_add(limbs_squared(s)))?
        .checked_mul(t_count)
}
/// 1 for a skipped node (`s_step = 0`), otherwise `128·⌈s_step/64⌉²`.
pub(crate) fn node_units(s_step: u32) -> u64 {
    match s_step {
        0 => 1,
        s => 128 * limbs_squared(s),
    }
}

// ---- exact evaluation ----

/// Host refusal of the dyadic arena reservation; F maps it to ResourceLimit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AllocRefused;
/// Walk scratch values per arena, including the dyadic operation scratch.
const SCRATCH: usize = 32;
/// The composite stack (one 16-value frame per node depth plus the identity) and walk scratch.
pub(crate) struct Arena {
    d: Dyadics,
    frames: usize,
    top: usize,
}
impl Arena {
    /// `16·(node_depth + 1) + 32` values of `⌈(span_bits + 64)/64⌉` limbs plus the admission region.
    pub(crate) fn reserve(caps: SpanCaps) -> Result<Arena, AllocRefused> {
        Self::reserve_within(caps, usize::MAX)
    }
    fn reserve_within(caps: SpanCaps, max_limbs: usize) -> Result<Arena, AllocRefused> {
        let frames = usize::try_from(caps.node_depth)
            .ok()
            .and_then(|d| d.checked_add(1));
        let n = frames.and_then(|f| f.checked_mul(16)?.checked_add(SCRATCH));
        let cap = caps
            .span_bits
            .checked_add(64)
            .and_then(|b| usize::try_from(b.div_ceil(64)).ok());
        let (Some(frames), Some(n), Some(cap)) = (frames, n, cap) else {
            return Err(AllocRefused);
        };
        let mut d = Dyadics::reserve(n, cap, max_limbs).ok_or(AllocRefused)?;
        for i in 0..4 {
            let e = d.walk(5 * i);
            d.int(e, 1);
        }
        Ok(Arena { d, frames, top: 0 })
    }
    /// Sticky capacity-invariant breach; F maps it to InvalidState, never to a verdict.
    pub(crate) fn fault(&self) -> bool {
        self.d.fault()
    }
    fn g(&self, frame: usize, i: usize, j: usize) -> D {
        self.d.walk(16 * frame + 4 * i + j)
    }
    fn s(&self, k: usize) -> D {
        self.d.walk(16 * self.frames + k)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RotationOutcome {
    Ok,
    InvalidZeroOrRange,
    QuaternionWindow,
}
/// Node rotation admission: finite components in [−1, 1], not all zero, and `|Σq² − 1| ≤ 2^-18`
/// evaluated exactly in the arena's admission region.
pub(crate) fn admit_rotation(q: [f64; 4], arena: &mut Arena) -> RotationOutcome {
    if q.iter().any(|c| !c.is_finite() || c.abs() > 1.0) || q.iter().all(|&c| c == 0.0) {
        return RotationOutcome::InvalidZeroOrRange;
    }
    let d = &mut arena.d;
    let (s, t) = (d.admit(0), d.admit(1));
    d.zero(s);
    for c in q {
        d.float(t, c);
        d.mul(t, t, t);
        d.add(s, s, t);
    }
    d.int(t, 1);
    d.sub(s, s, t);
    d.pow2(t, -18);
    match d.cmp_abs(s, t) {
        Ordering::Greater => RotationOutcome::QuaternionWindow,
        _ => RotationOutcome::Ok,
    }
}
/// `L` for a TRS node, homogeneous: `[H·S | n·t; 0 0 0 n]` with `H = n·R(q)`, in scratch 0..16.
fn trs_local(a: &mut Arena, t: [f64; 3], q: [f64; 4], sc: [f64; 3]) -> bool {
    let l: [D; 16] = std::array::from_fn(|k| a.s(k));
    let [x, y, z, w] = [16, 17, 18, 19].map(|k| a.s(k));
    let [xx, yy, zz, ww] = [20, 21, 22, 23].map(|k| a.s(k));
    let [n, p1, p2, input] = [24, 25, 26, 27].map(|k| a.s(k));
    let d = &mut a.d;
    let mut finite = true;
    for (h, c) in [x, y, z, w].into_iter().zip(q) {
        finite &= d.float(h, c);
    }
    for (sq, v) in [(xx, x), (yy, y), (zz, z), (ww, w)] {
        d.mul(sq, v, v);
    }
    d.add(n, xx, yy);
    d.add(n, n, zz);
    d.add(n, n, ww);
    // H00 = Σ[w·w, x·x, −y·y, −z·z], H11 = Σ[w·w, −x·x, y·y, −z·z], H22 = Σ[w·w, −x·x, −y·y, z·z].
    for (r, signs) in [
        [true, false, false],
        [false, true, false],
        [false, false, true],
    ]
    .into_iter()
    .enumerate()
    {
        let h = l[5 * r];
        d.copy(h, ww);
        for (v, plus) in [xx, yy, zz].into_iter().zip(signs) {
            if plus {
                d.add(h, h, v);
            } else {
                d.sub(h, h, v);
            }
        }
    }
    // H[r][c] = 2·Σ[a·b, ±e·f], as written in the contract.
    for (rc, (a0, a1), (b0, b1), plus) in [
        (1, (x, y), (w, z), false),
        (2, (x, z), (w, y), true),
        (4, (x, y), (w, z), true),
        (6, (y, z), (w, x), false),
        (8, (x, z), (w, y), false),
        (9, (y, z), (w, x), true),
    ] {
        d.mul(p1, a0, a1);
        d.mul(p2, b0, b1);
        if plus {
            d.add(l[rc], p1, p2);
        } else {
            d.sub(l[rc], p1, p2);
        }
        d.dbl(l[rc]);
    }
    for c in 0..3 {
        finite &= d.float(input, sc[c]);
        for r in 0..3 {
            d.mul(l[4 * r + c], l[4 * r + c], input);
        }
    }
    for r in 0..3 {
        finite &= d.float(input, t[r]);
        d.mul(l[4 * r + 3], n, input);
        d.zero(l[12 + r]);
    }
    d.copy(l[15], n);
    finite
}
fn matrix_local(a: &mut Arena, m: [f64; 16]) -> bool {
    let l: [D; 16] = std::array::from_fn(|k| a.s(k));
    let mut finite = true;
    for r in 0..3 {
        for c in 0..4 {
            finite &= a.d.float(l[4 * r + c], m[4 * c + r]);
        }
        a.d.zero(l[12 + r]);
    }
    a.d.int(l[15], 1);
    finite
}
/// Pushes `G·L` onto the composite stack (root-first, parent·local); `Skip` pushes nothing.
pub(crate) fn push_node(arena: &mut Arena, l: &Local) {
    let finite = match *l {
        Local::Skip => return,
        Local::Matrix(m) => matrix_local(arena, m),
        Local::Trs { t, q, s } => trs_local(arena, t, q, s),
    };
    if !finite || arena.top + 1 >= arena.frames {
        return arena.d.breach();
    }
    let (top, tmp) = (arena.top, arena.s(28));
    for i in 0..4 {
        for j in 0..4 {
            let (dst, g, l) = (arena.g(top + 1, i, j), arena.g(top, i, 0), arena.s(j));
            arena.d.mul(dst, g, l);
            for k in 1..4 {
                let (g, l) = (arena.g(top, i, k), arena.s(4 * k + j));
                arena.d.mul(tmp, g, l);
                arena.d.add(dst, dst, tmp);
            }
        }
    }
    arena.top += 1;
}
pub(crate) fn pop_node(arena: &mut Arena) {
    match arena.top.checked_sub(1) {
        Some(top) => arena.top = top,
        None => arena.d.breach(),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Contained,
    Outside { role: Role, level: u32 },
}
fn rank(r: Role) -> u8 {
    match r {
        Role::Own => 0,
        Role::Content => 1,
        Role::ExternalContent(_) => 2,
        Role::Ancestor(_) => 3,
    }
}
/// Homogeneous membership of `P/W` (W > 0): `|P_j − c_j·W| ≤ h_j·W`, or
/// `Σ(P_j − c_j·W)² ≤ r²·W²`.
fn inside(a: &mut Arena, p: [D; 3], w: D, v: &VolumeFacts) -> bool {
    let [input, cw, dd, hw, q, lhs, r2, w2] = [14, 15, 16, 17, 18, 19, 20, 21].map(|k| a.s(k));
    let d = &mut a.d;
    let (c, sphere) = match v {
        VolumeFacts::Box(b) if volume_exclusion(v).is_none() => (&b[..3], None),
        VolumeFacts::Sphere(s) => (&s[..3], Some(s[3])),
        _ => {
            d.breach();
            return true;
        }
    };
    d.zero(lhs);
    for j in 0..3 {
        d.float(input, c[j]);
        d.mul(cw, input, w);
        d.sub(dd, p[j], cw);
        match (v, sphere) {
            (VolumeFacts::Box(b), _) => {
                d.float(input, half(b, j).abs());
                d.mul(hw, input, w);
                if d.cmp_abs(dd, hw) == Ordering::Greater {
                    return false;
                }
            }
            _ => {
                d.mul(q, dd, dd);
                d.add(lhs, lhs, q);
            }
        }
    }
    let Some(r) = sphere else {
        return true;
    };
    d.float(input, r);
    d.mul(r2, input, input);
    d.mul(w2, w, w);
    d.mul(q, r2, w2);
    d.cmp(lhs, q) != Ordering::Greater
}
/// Applies the top composite to the raw vertex (signed clamp first, homogeneous `k`), then U
/// once, then for each level `l`: `levels[l].to_parent` when `l > 0`, and that level's volumes
/// in Role order. Never allocates; on a capacity breach the verdict is meaningless and `fault()`
/// is set.
pub(crate) fn walk(
    arena: &mut Arena,
    n: Normalization,
    raw: RawVertex,
    levels: &[LevelFacts],
) -> Verdict {
    debug_assert!(levels.first().is_none_or(|l| l.to_parent.is_none()));
    let (_, _, k, signed) = word_and_k(n);
    let top = arena.top;
    let x: [D; 4] = std::array::from_fn(|c| arena.s(c));
    let [p0, p1, p2, w] = [4, 5, 6, 7].map(|i| arena.s(i));
    let mut next = [8, 9, 10].map(|i| arena.s(i));
    let (tmp, input) = (arena.s(11), arena.s(12));
    let mut finite = true;
    for c in 0..3 {
        match raw {
            RawVertex::F32(v) => finite &= arena.d.float(x[c], f64::from(v[c])),
            RawVertex::Int(v) if signed && v[c] <= -k => arena.d.int(x[c], i64::from(-k)),
            RawVertex::Int(v) => arena.d.int(x[c], i64::from(v[c])),
        }
    }
    arena.d.int(x[3], i64::from(k));
    for (r, pr) in [p0, p1, p2].into_iter().enumerate() {
        let g = arena.g(top, r, 0);
        arena.d.mul(pr, g, x[0]);
        for (c, &xc) in x.iter().enumerate().skip(1) {
            let g = arena.g(top, r, c);
            arena.d.mul(tmp, g, xc);
            arena.d.add(pr, pr, tmp);
        }
    }
    let g33 = arena.g(top, 3, 3);
    arena.d.mul(w, g33, x[3]);
    if !finite || arena.d.is_zero(w) || arena.d.is_negative(w) {
        arena.d.breach();
        return Verdict::Contained;
    }
    // U(x, y, z) = (x, −z, y), applied exactly once.
    arena.d.neg(p2);
    let mut p = [p0, p2, p1];
    for (l, level) in levels.iter().enumerate() {
        if let Some(t) = level.to_parent.filter(|_| l > 0) {
            for (r, &nr) in next.iter().enumerate() {
                arena.d.float(input, t[r]);
                arena.d.mul(nr, input, p[0]);
                for (c, &operand) in [p[1], p[2], w].iter().enumerate() {
                    arena.d.float(input, t[4 * (c + 1) + r]);
                    arena.d.mul(tmp, input, operand);
                    arena.d.add(nr, nr, tmp);
                }
            }
            std::mem::swap(&mut p, &mut next);
        }
        for want in 0..4 {
            for (role, v) in level.volumes.iter().flatten() {
                if rank(*role) == want && !inside(arena, p, w, v) {
                    return Verdict::Outside {
                        role: *role,
                        level: l as u32,
                    };
                }
            }
        }
    }
    Verdict::Contained
}

#[cfg(test)]
mod tests {
    use super::*;
    use Bound::Zero as Z;

    fn pair(lo: i32, hi: i32) -> Bound {
        Bound::P(Pair(lo, hi))
    }
    fn caps(span_bits: u64) -> SpanCaps {
        SpanCaps {
            span_bits,
            node_depth: 64,
        }
    }
    /// Box with half-axis column `i` = `h_i·e_i`.
    fn bx(c: [f64; 3], h: [f64; 3]) -> VolumeFacts {
        VolumeFacts::Box([
            c[0], c[1], c[2], h[0], 0.0, 0.0, 0.0, h[1], 0.0, 0.0, 0.0, h[2],
        ])
    }
    fn level(to_parent: Option<[f64; 16]>, vols: &[(Role, VolumeFacts)]) -> LevelFacts {
        let mut volumes = [None; 3];
        for (slot, v) in volumes.iter_mut().zip(vols) {
            *slot = Some(*v);
        }
        LevelFacts { to_parent, volumes }
    }
    fn affine(diag: [f64; 3], t: [f64; 3]) -> [f64; 16] {
        let mut m = [0.0; 16];
        (m[0], m[5], m[10], m[15]) = (diag[0], diag[1], diag[2], 1.0);
        (m[12], m[13], m[14]) = (t[0], t[1], t[2]);
        m
    }
    fn prev(x: f64) -> f64 {
        f64::from_bits(x.to_bits() - 1)
    }
    fn succ(x: f64) -> f64 {
        f64::from_bits(x.to_bits() + 1)
    }
    const IDENTITY_TRS: Local = Local::Trs {
        t: [0.0; 3],
        q: [0.0, 0.0, 0.0, 1.0],
        s: [1.0; 3],
    };
    const IDENTITY_MATRIX: Local = Local::Matrix([
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);
    /// The per-URI facts F derives for a single instance path.
    fn uri(chain: &[Local], n: Normalization) -> UriBounds {
        let (mut g, mut s_inst) = (composite_identity(), 0);
        for l in chain {
            let (next, s) = node_step_bound(&g, l);
            (g, s_inst) = (next, s_inst.max(s));
        }
        let (pair0, s_pos) = position_bound(&g, n);
        UriBounds {
            pair0,
            s_inst,
            s_pos,
        }
    }
    fn arena_with(span_bits: u64, chain: &[Local]) -> Arena {
        let mut a = Arena::reserve(SpanCaps {
            span_bits,
            node_depth: chain.len() as u64,
        })
        .unwrap();
        chain.iter().for_each(|l| push_node(&mut a, l));
        a
    }
    fn spans(u: &UriBounds, levels: &[LevelFacts], bits: u64) -> Vec<u32> {
        level_spans(u, levels, caps(bits)).unwrap().iter().collect()
    }
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    // ---- the worked example (contract "Worked example", B1) ----

    const WORKED_NODE: Local = Local::Trs {
        t: [10.5, 0.0, -3.0],
        q: [0.0, 0.0, 0.0, 1.0],
        s: [2.0, 4.0, 0.25],
    };
    /// The contract's literal own box: row 1 is the plane −z = 0, which no vertex reaches.
    const LITERAL_CENTRE: [f64; 3] = [1.5, 0.0, 2.0];
    /// O's adjusted fixture (v3.3): row-1 centre 3, so vertices with raw z = 0 lie on the plane.
    const ADJUSTED_CENTRE: [f64; 3] = [1.5, 3.0, 2.0];
    fn worked_levels(own_centre: [f64; 3]) -> [LevelFacts; 2] {
        [
            level(None, &[(Role::Own, bx(own_centre, [100.0, 0.0, 50.0]))]),
            level(
                Some(affine([1.0; 3], [1000.25, -20.0, 0.0])),
                &[(Role::Ancestor(1), bx([0.0; 3], [2048.0; 3]))],
            ),
        ]
    }

    #[test]
    fn worked_example_is_bit_exact() {
        let (g, s_step) = node_step_bound(&composite_identity(), &WORKED_NODE);
        assert_eq!(s_step, 5);
        assert_eq!(
            g,
            [
                [pair(1, 1), Z, Z, pair(-1, 3)],
                [Z, pair(2, 2), Z, Z],
                [Z, Z, pair(-2, -2), pair(0, 1)],
                [Z, Z, Z, ONE],
            ]
        );
        let (pair0, s_pos) = position_bound(&g, Normalization::U16);
        assert_eq!(
            pair0,
            [pair(-1, 20), pair(-2, 18), pair(2, 18), pair(0, 15)]
        );
        assert_eq!(s_pos, 22);
        let u = uri(&[WORKED_NODE], Normalization::U16);
        assert_eq!((u.pair0, u.s_inst, u.s_pos), (pair0, 5, 22));
        let levels = worked_levels(LITERAL_CENTRE);
        let at_cap = level_spans(&u, &levels, caps(29)).unwrap();
        assert_eq!(at_cap.iter().collect::<Vec<_>>(), [23, 29]);
        assert_eq!(vertex_units(4, &at_cap), Some(8));
        assert_eq!(node_units(s_step), 128);
        assert_eq!(
            level_spans(&u, &levels, caps(28)).err(),
            Some(Exclusion::NumericSpan)
        );
    }

    #[test]
    fn worked_example_verdicts_at_cap_and_one_over() {
        let n = Normalization::U16;
        let u = uri(&[WORKED_NODE], n);
        let own = Verdict::Outside {
            role: Role::Own,
            level: 0,
        };
        let mut rng = Rng(7);
        let mut vertices: Vec<[i32; 3]> = (0..8)
            .map(|i| [i & 1, i >> 1 & 1, i >> 2 & 1].map(|b| b * 65535))
            .collect();
        vertices.extend((0..56).map(|_| [0; 3].map(|_| rng.below(65536) as i32)));
        for centre in [LITERAL_CENTRE, ADJUSTED_CENTRE] {
            let levels = worked_levels(centre);
            assert_eq!(spans(&u, &levels, 29), [23, 29]);
            assert_eq!(
                level_spans(&u, &levels, caps(28)).err(),
                Some(Exclusion::NumericSpan)
            );
        }
        let mut a = arena_with(29, &[WORKED_NODE]);
        // Literal: after U, row 1 is 3 − a₂/(4·65535) ∈ [2.75, 3], never on the plane −z = 0.
        let literal = worked_levels(LITERAL_CENTRE);
        for &v in &vertices {
            assert_eq!(walk(&mut a, n, RawVertex::Int(v), &literal), own);
        }
        // Adjusted: raw z = 0 lies exactly on the row-1 plane at 3; raw z = 1 is off it.
        let adjusted = worked_levels(ADJUSTED_CENTRE);
        for &[x, y, z] in &vertices {
            let on = RawVertex::Int([x, y, 0]);
            assert_eq!(walk(&mut a, n, on, &adjusted), Verdict::Contained);
            let off = RawVertex::Int([x, y, z.max(1)]);
            assert_eq!(walk(&mut a, n, off, &adjusted), own);
        }
        assert!(!a.fault());
        // x = 2·a₀/65535 + 10.5 + 1000.25 reaches 1012.75 exactly at a₀ = 65535.
        let parent = |h: f64| {
            [
                adjusted[0],
                level(
                    adjusted[1].to_parent,
                    &[(Role::Ancestor(1), bx([0.0; 3], [h, 2048.0, 2048.0]))],
                ),
            ]
        };
        let corner = RawVertex::Int([65535, 0, 0]);
        assert_eq!(
            walk(&mut a, n, corner, &parent(1012.75)),
            Verdict::Contained
        );
        assert_eq!(
            walk(&mut a, n, corner, &parent(prev(1012.75))),
            Verdict::Outside {
                role: Role::Ancestor(1),
                level: 1
            }
        );
        assert_eq!(spans(&u, &parent(1012.75), 29), [23, 29]);
        assert!(!a.fault());
    }

    // ---- pair arithmetic ----

    #[test]
    fn flat_sum_adds_ceil_log2_m() {
        let t = [pair(0, 3), pair(-2, 1), pair(1, 5), pair(4, 4)];
        assert_eq!(flat_sum(&t[..1]), pair(0, 3));
        assert_eq!(flat_sum(&t[..2]), pair(-2, 4));
        assert_eq!(flat_sum(&t[..3]), pair(-2, 7));
        assert_eq!(flat_sum(&t[..4]), pair(-2, 7));
        // ZERO terms are dropped before m is counted.
        assert_eq!(flat_sum(&[Z, pair(0, 3), Z]), pair(0, 3));
        assert_eq!(flat_sum(&[Z, pair(0, 3), Z, pair(-2, 1)]), pair(-2, 4));
        assert_eq!(flat_sum(&[Z, Z]), Z);
        assert_eq!(flat_sum(&[]), Z);
        // n = Σ[x·x, y·y, z·z, w·w] for q = (0.5, 0.5, 0.5, 0.5): four (−2, −1) terms.
        let q = Local::Trs {
            t: [0.0; 3],
            q: [0.5; 4],
            s: [1.0; 3],
        };
        assert_eq!(
            node_step_bound(&composite_identity(), &q).0[3][3],
            pair(-2, 1)
        );
    }

    #[test]
    fn unit_pair_passes_through_and_zero_absorbs() {
        assert_eq!(ONE.mul(pair(-3, 7)), pair(-3, 7));
        assert_eq!(pair(-3, 7).mul(ONE), pair(-3, 7));
        assert_eq!(ONE.mul(ONE), ONE);
        assert_eq!(pair(0, 1).mul(pair(2, 3)), pair(2, 5));
        assert_eq!(pair(-1, -1).mul(pair(-1, -1)), pair(-2, -1));
        assert_eq!(Z.mul(ONE), Z);
        assert_eq!(pair(5, 9).mul(Z), Z);
        assert_eq!(TWO.mul(pair(-1000, -1000)), pair(-999, -998));
        // Saturation is over every cap.
        let big = pair(i32::MIN + 5, 0).mul(pair(-10, 0));
        assert_eq!(big.span(), u32::MAX);
        assert!(over(big.span(), u64::MAX));
    }

    #[test]
    fn negative_zero_is_zero_everywhere() {
        assert_eq!(Bound::of(-0.0), Z);
        assert_eq!(Bound::of(0.0), Z);
        assert_eq!(Bound::of(1.5), pair(-1, 0));
        assert_eq!(Bound::of(-3.0), pair(0, 1));
        assert_eq!(Bound::of(1000.25), pair(-2, 9));
        assert_eq!(Bound::of(f64::from_bits(1)), pair(-1074, -1074));
        assert_eq!(Bound::of(f64::MAX), pair(971, 1023));
        let trs = |t0| Local::Trs {
            t: [t0, 1.0, 1.0],
            q: [0.0, -0.0, 0.0, 1.0],
            s: [1.0; 3],
        };
        let (g, _) = node_step_bound(&composite_identity(), &trs(-0.0));
        assert_eq!((g[0][3], g[0][1]), (Z, Z));
        assert_eq!(g, node_step_bound(&composite_identity(), &trs(0.0)).0);
        let mut m = [0.0; 16];
        (m[0], m[5], m[10], m[15], m[12]) = (1.0, 1.0, 1.0, 1.0, -0.0);
        assert_eq!(
            node_step_bound(&composite_identity(), &Local::Matrix(m)).0,
            composite_identity()
        );
        // A −0.0 centre gives the same spans and verdicts as +0.0.
        let u = uri(&[], Normalization::None(Word::F32));
        let at = |c: f64| {
            [level(
                None,
                &[(Role::Own, bx([c, 0.0, c], [0.0, 1.0, 0.0]))],
            )]
        };
        assert_eq!(spans(&u, &at(-0.0), 4096), spans(&u, &at(0.0), 4096));
        let mut a = arena_with(4096, &[]);
        for c in [0.0, -0.0] {
            let n = Normalization::None(Word::F32);
            let origin = RawVertex::F32([0.0, 0.0, -0.0]);
            assert_eq!(walk(&mut a, n, origin, &at(c)), Verdict::Contained);
        }
    }

    #[test]
    fn hull_is_entrywise_with_zero_identity() {
        let a = UriBounds {
            pair0: [pair(-1, 20), Z, pair(2, 18), pair(0, 15)],
            s_inst: 5,
            s_pos: 22,
        };
        let b = UriBounds {
            pair0: [pair(0, 25), pair(3, 4), Z, pair(0, 15)],
            s_inst: 9,
            s_pos: 7,
        };
        assert_eq!(hull(&UriBounds::identity(), &a), a);
        assert_eq!(hull(&a, &UriBounds::identity()), a);
        let h = hull(&a, &b);
        assert_eq!(
            h.pair0,
            [pair(-1, 25), pair(3, 4), pair(2, 18), pair(0, 15)]
        );
        assert_eq!((h.s_inst, h.s_pos), (9, 22));
        assert_eq!(hull(&b, &a), h);
        // Two instances each within the cap whose hull is not (delta2 H1).
        let near = UriBounds {
            pair0: [pair(0, 20), Z, Z, ONE],
            s_inst: 1,
            s_pos: 21,
        };
        let far = UriBounds {
            pair0: [pair(-20, 0), Z, Z, ONE],
            s_inst: 1,
            s_pos: 21,
        };
        let own = [level(None, &[(Role::Own, bx([0.0; 3], [1.0; 3]))])];
        assert!(level_spans(&near, &own, caps(22)).is_ok());
        assert!(level_spans(&far, &own, caps(22)).is_ok());
        assert_eq!(
            level_spans(&hull(&near, &far), &own, caps(22)).err(),
            Some(Exclusion::NumericSpan)
        );
    }

    #[test]
    fn identity_nodes_are_equivalent_to_skip() {
        let (_, s) = node_step_bound(&composite_identity(), &Local::Skip);
        assert_eq!(s, 0);
        for id in [IDENTITY_TRS, IDENTITY_MATRIX] {
            assert_eq!(
                node_step_bound(&composite_identity(), &id),
                (composite_identity(), 1)
            );
        }
        let real = Local::Trs {
            t: [3.5, -2.0, 0.75],
            q: [0.5, -0.5, 0.5, 0.5],
            s: [2.0, 0.5, 3.0],
        };
        let n = Normalization::I16;
        let levels = [
            // Centred on U(t) so that some vertices fall outside.
            level(None, &[(Role::Own, bx([3.5, -0.75, -2.0], [1.5; 3]))]),
            level(
                Some(affine([2.0, 1.0, 1.0], [0.5, 0.0, -1.0])),
                &[
                    (Role::Ancestor(1), bx([7.0, 0.0, -4.0], [16.0; 3])),
                    (
                        Role::ExternalContent(1),
                        VolumeFacts::Sphere([7.0, 0.0, -4.0, 20.0]),
                    ),
                ],
            ),
        ];
        let skip = uri(&[Local::Skip, real], n);
        let mut s = None;
        for id in [IDENTITY_TRS, IDENTITY_MATRIX] {
            let u = uri(&[id, real], n);
            assert_eq!(
                (u.pair0, u.s_inst, u.s_pos),
                (skip.pair0, skip.s_inst, skip.s_pos)
            );
            assert_eq!(spans(&u, &levels, 4096), spans(&skip, &levels, 4096));
            s.get_or_insert(spans(&u, &levels, 4096));
        }
        let mut arenas = [
            arena_with(4096, &[Local::Skip, real]),
            arena_with(4096, &[IDENTITY_TRS, real]),
            arena_with(4096, &[IDENTITY_MATRIX, real]),
        ];
        let mut rng = Rng(11);
        let (mut inside, mut outside) = (0, 0);
        for _ in 0..300 {
            let raw = RawVertex::Int([0; 3].map(|_| rng.below(65536) as i32 - 32768));
            let v = arenas.each_mut().map(|a| walk(a, n, raw, &levels));
            assert!(v.iter().all(|x| *x == v[0]));
            match v[0] {
                Verdict::Contained => inside += 1,
                _ => outside += 1,
            }
        }
        assert!(inside > 0 && outside > 0);
        assert!(arenas.iter().all(|a| !a.fault()));
    }

    // ---- decode: denominators and the signed clamp ----

    #[test]
    fn k_denominators_and_word_pairs() {
        let g = composite_identity();
        for (n, a, k) in [
            (Normalization::None(Word::F32), pair(-149, 127), ONE),
            (Normalization::None(Word::Int8), pair(0, 7), ONE),
            (Normalization::None(Word::Int16), pair(0, 15), ONE),
            (Normalization::I8, pair(0, 7), pair(0, 6)),
            (Normalization::U8, pair(0, 7), pair(0, 7)),
            (Normalization::I16, pair(0, 15), pair(0, 14)),
            (Normalization::U16, pair(0, 15), pair(0, 15)),
        ] {
            let (p, s) = position_bound(&g, n);
            assert_eq!(p, [a, a, a, k]);
            assert_eq!(s, a.span().max(k.span()));
        }
        // The F32 identity level 0 is the probe's producer level-0 span.
        assert_eq!(position_bound(&g, Normalization::None(Word::F32)).1, 277);
        // Raw k is exactly 1 and raw a is exactly a/k: the smallest f64 at or above a/k is
        // inside and the next one down is outside.
        let mut arena = arena_with(4096, &[]);
        for (n, k, raws) in [
            (Normalization::I8, 127, [1i32, 100, -5, 127]),
            (Normalization::U8, 255, [1, 3, 254, 255]),
            (Normalization::I16, 32767, [1, -7, 32766, 32767]),
            (Normalization::U16, 65535, [1, 3, 65534, 65535]),
            (Normalization::None(Word::Int16), 1, [1, -7, 300, 65535]),
        ] {
            for a in raws {
                let hi = exact_ceiling(a.abs(), k);
                let vol = |h| [level(None, &[(Role::Own, bx([0.0; 3], [h, 0.0, 0.0]))])];
                let raw = RawVertex::Int([a, 0, 0]);
                assert_eq!(walk(&mut arena, n, raw, &vol(hi)), Verdict::Contained);
                assert!(walk(&mut arena, n, raw, &vol(prev(hi))) != Verdict::Contained);
            }
        }
        assert!(!arena.fault());
    }
    /// The smallest f64 `h ≥ a/k`, decided with integers.
    fn exact_ceiling(a: i32, k: i32) -> f64 {
        let h = f64::from(a) / f64::from(k);
        let bits = h.to_bits();
        let (m, e) = (
            (bits & ((1 << 52) - 1)) | 1 << 52,
            ((bits >> 52) & 0x7ff) as i32 - 1075,
        );
        assert!((-100..=0).contains(&e));
        let at_least = i128::from(m) * i128::from(k) >= i128::from(a) << -e;
        if at_least {
            h
        } else {
            succ(h)
        }
    }

    #[test]
    fn signed_clamp_precedes_the_rational() {
        let mut arena = arena_with(4096, &[]);
        for (n, k) in [(Normalization::I8, 127), (Normalization::I16, 32767)] {
            let minus_one = [level(None, &[(Role::Own, bx([-1.0, 0.0, 0.0], [0.0; 3]))])];
            for raw in [-k - 1, -k] {
                let raw = RawVertex::Int([raw, 0, 0]);
                assert_eq!(walk(&mut arena, n, raw, &minus_one), Verdict::Contained);
            }
            let raw = RawVertex::Int([-k + 1, 0, 0]);
            assert!(walk(&mut arena, n, raw, &minus_one) != Verdict::Contained);
            // Unsigned and unnormalized words are not clamped.
            let raw = RawVertex::Int([-k - 1, 0, 0]);
            let n = Normalization::None(if k == 127 { Word::Int8 } else { Word::Int16 });
            assert!(walk(&mut arena, n, raw, &minus_one) != Verdict::Contained);
        }
        // The clamped word stays within its raw pair.
        let (p, _) = position_bound(&composite_identity(), Normalization::I8);
        assert_eq!(
            p[0],
            Bound::of(-128.0).hull(Bound::of(127.0)).hull(pair(0, 7))
        );
        assert!(!arena.fault());
    }

    // ---- volumes ----

    #[test]
    fn sphere_squares_the_difference() {
        let u = uri(&[WORKED_NODE], Normalization::U16);
        let sphere = |l0: VolumeFacts| [level(None, &[(Role::Own, l0)])];
        assert_eq!(
            spans(&u, &sphere(bx([1.5, 0.0, 2.0], [100.0, 0.0, 50.0])), 4096),
            [23]
        );
        // Q = D·D: (−2,43), (−4,37), (2,39); LHS = (−4,45); RHS = (4,13)·(0,31) = (4,45).
        let s = spans(
            &u,
            &sphere(VolumeFacts::Sphere([1.5, 0.0, 2.0, 100.0])),
            4096,
        );
        assert_eq!(s, [50]);
        assert!(s[0] >= 2 * 23);
        let at = |h: f64| {
            [level(
                None,
                &[(Role::Own, VolumeFacts::Sphere([0.0, 0.0, 0.0, h]))],
            )]
        };
        // (3, 4, 0) after U is at distance exactly 5.
        let mut arena = arena_with(4096, &[]);
        let n = Normalization::None(Word::F32);
        let raw = RawVertex::F32([3.0, 0.0, -4.0]);
        assert_eq!(walk(&mut arena, n, raw, &at(5.0)), Verdict::Contained);
        assert!(walk(&mut arena, n, raw, &at(prev(5.0))) != Verdict::Contained);
        assert_eq!(
            walk(&mut arena, n, RawVertex::F32([0.0; 3]), &at(0.0)),
            Verdict::Contained
        );
        assert!(!arena.fault());
    }

    #[test]
    fn zero_half_lengths_are_exact_planes_lines_and_points() {
        let n = Normalization::None(Word::F32);
        let p = 0.1f32;
        let c = f64::from(p);
        let point = [level(None, &[(Role::Own, bx([c, 0.0, 0.0], [0.0; 3]))])];
        let mut arena = arena_with(4096, &[]);
        assert_eq!(
            walk(&mut arena, n, RawVertex::F32([p, 0.0, 0.0]), &point),
            Verdict::Contained
        );
        let next = f32::from_bits(p.to_bits() + 1);
        assert!(
            walk(&mut arena, n, RawVertex::F32([next, 0.0, 0.0]), &point) != Verdict::Contained
        );
        let line = [level(None, &[(Role::Own, bx([0.0; 3], [0.0, 1.0, 0.0]))])];
        assert_eq!(
            walk(&mut arena, n, RawVertex::F32([0.0, 0.0, -0.5]), &line),
            Verdict::Contained
        );
        assert!(walk(&mut arena, n, RawVertex::F32([0.0, 0.5, 0.0]), &line) != Verdict::Contained);
        // HW is ZERO for a zero half-length and D is P alone for a zero centre.
        let u = uri(&[], n);
        assert_eq!(spans(&u, &line, 4096), [277]);
        assert!(!arena.fault());
    }

    #[test]
    fn signed_permutation_boxes_only() {
        let perm = VolumeFacts::Box([0.0, 0.0, 0.0, 0.0, -2.0, 0.0, 0.0, 0.0, 3.0, 4.0, 0.0, 0.0]);
        assert_eq!(volume_exclusion(&perm), None);
        assert_eq!(volume_exclusion(&bx([0.0; 3], [0.0; 3])), None);
        let rotated =
            VolumeFacts::Box([0.0, 0.0, 0.0, 1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(volume_exclusion(&rotated), Some(Exclusion::RotatedBox));
        let same_row =
            VolumeFacts::Box([0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(volume_exclusion(&same_row), Some(Exclusion::RotatedBox));
        assert_eq!(
            volume_exclusion(&VolumeFacts::Region),
            Some(Exclusion::RegionVolume)
        );
        assert_eq!(
            volume_exclusion(&VolumeFacts::Sphere([0.0, 0.0, 0.0, 1.0])),
            None
        );
        // Row j takes |entry| from its covering column: here rows (1, 2, 0) = (2, 3, 4).
        let mut arena = arena_with(4096, &[]);
        let n = Normalization::None(Word::F32);
        let own = [level(None, &[(Role::Own, perm)])];
        let raw = RawVertex::F32([4.0, -3.0, -2.0]);
        assert_eq!(walk(&mut arena, n, raw, &own), Verdict::Contained);
        let raw = RawVertex::F32([4.0, 3.0, -2.5]);
        assert!(walk(&mut arena, n, raw, &own) != Verdict::Contained);
        // Shape exclusions take precedence over numericSpan, region before rotated.
        let u = uri(&[], n);
        let both = [
            level(None, &[(Role::Own, rotated)]),
            level(None, &[(Role::Ancestor(1), VolumeFacts::Region)]),
        ];
        assert_eq!(
            level_spans(&u, &both, caps(0)).err(),
            Some(Exclusion::RegionVolume)
        );
        assert_eq!(
            level_spans(&u, &both[..1], caps(0)).err(),
            Some(Exclusion::RotatedBox)
        );
        assert_eq!(
            level_spans(&u, &own, caps(276)).err(),
            Some(Exclusion::NumericSpan)
        );
        assert!(!arena.fault());
    }

    // ---- frame order ----

    #[test]
    fn up_axis_is_applied_exactly_once() {
        let mut arena = arena_with(4096, &[]);
        let n = Normalization::None(Word::F32);
        let raw = RawVertex::F32([0.0, 0.0, 1.0]);
        let at = |c| [level(None, &[(Role::Own, bx(c, [0.0; 3]))])];
        assert_eq!(
            walk(&mut arena, n, raw, &at([0.0, -1.0, 0.0])),
            Verdict::Contained
        );
        for wrong in [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0]] {
            assert!(walk(&mut arena, n, raw, &at(wrong)) != Verdict::Contained);
        }
        assert!(!arena.fault());
    }

    #[test]
    fn composite_is_root_first_parent_times_local() {
        let root = Local::Trs {
            t: [1.0, 0.0, 0.0],
            q: [0.0, 0.0, 0.0, 1.0],
            s: [1.0; 3],
        };
        // A half turn about z, then a quarter turn with a non-unit quaternion (R = H/‖q‖²).
        let half = Local::Trs {
            t: [0.0; 3],
            q: [0.0, 0.0, 1.0, 0.0],
            s: [1.0; 3],
        };
        let quarter = Local::Trs {
            t: [0.0; 3],
            q: [0.0, 0.0, 0.5, 0.5],
            s: [1.0; 3],
        };
        let n = Normalization::None(Word::F32);
        let at = |c| [level(None, &[(Role::Own, bx(c, [0.0; 3]))])];
        let raw = RawVertex::F32([1.0, 0.0, 0.0]);
        let mut arena = arena_with(4096, &[root, half]);
        // T·R·x = (1,0,0) + (−1,0,0); R·T·x would be (−2, 0, 0).
        assert_eq!(walk(&mut arena, n, raw, &at([0.0; 3])), Verdict::Contained);
        assert!(walk(&mut arena, n, raw, &at([-2.0, 0.0, 0.0])) != Verdict::Contained);
        // Pop the half turn and push the quarter turn: T·Rq·x = (1, 1, 0) → U → (1, 0, 1).
        pop_node(&mut arena);
        push_node(&mut arena, &quarter);
        assert_eq!(
            walk(&mut arena, n, raw, &at([1.0, 0.0, 1.0])),
            Verdict::Contained
        );
        // Skip pushes nothing, so one pop returns to the root.
        push_node(&mut arena, &Local::Skip);
        pop_node(&mut arena);
        assert_eq!(
            walk(&mut arena, n, raw, &at([2.0, 0.0, 0.0])),
            Verdict::Contained
        );
        assert!(!arena.fault());
    }

    #[test]
    fn tile_steps_apply_before_their_level_in_order() {
        let n = Normalization::None(Word::F32);
        let raw = RawVertex::F32([1.0, 0.0, 0.0]);
        let mut arena = arena_with(4096, &[]);
        let chain = |c1: f64, c2: f64| {
            [
                level(None, &[(Role::Own, bx([1.0, 0.0, 0.0], [0.0; 3]))]),
                level(
                    Some(affine([2.0, 1.0, 1.0], [0.0; 3])),
                    &[(Role::Ancestor(1), bx([c1, 0.0, 0.0], [0.0; 3]))],
                ),
                level(
                    Some(affine([1.0; 3], [1.0, 0.0, 0.0])),
                    &[(Role::Ancestor(2), bx([c2, 0.0, 0.0], [0.0; 3]))],
                ),
            ]
        };
        assert_eq!(
            walk(&mut arena, n, raw, &chain(2.0, 3.0)),
            Verdict::Contained
        );
        // Reversed tile order would reach 4 at the top level.
        assert_eq!(
            walk(&mut arena, n, raw, &chain(2.0, 4.0)),
            Verdict::Outside {
                role: Role::Ancestor(2),
                level: 2
            }
        );
        // An ancestor's own transform is not applied before testing it.
        assert_eq!(
            walk(&mut arena, n, raw, &chain(1.0, 3.0)),
            Verdict::Outside {
                role: Role::Ancestor(1),
                level: 1
            }
        );
        // A transform on level 0 is not a step (levels[0].to_parent is None by contract).
        assert!(!arena.fault());
    }

    #[test]
    fn volumes_are_tested_in_role_order() {
        let n = Normalization::None(Word::F32);
        let raw = RawVertex::F32([5.0, 0.0, 0.0]);
        let miss = bx([0.0; 3], [1.0; 3]);
        let hit = bx([0.0; 3], [8.0; 3]);
        let mut arena = arena_with(4096, &[]);
        let own = level(None, &[(Role::Content, miss), (Role::Own, miss)]);
        assert_eq!(
            walk(&mut arena, n, raw, &[own]),
            Verdict::Outside {
                role: Role::Own,
                level: 0
            }
        );
        let up = [
            level(None, &[(Role::Own, hit), (Role::Content, hit)]),
            level(
                None,
                &[(Role::Ancestor(1), miss), (Role::ExternalContent(1), miss)],
            ),
        ];
        assert_eq!(
            walk(&mut arena, n, raw, &up),
            Verdict::Outside {
                role: Role::ExternalContent(1),
                level: 1
            }
        );
        let up = [
            up[0],
            level(
                None,
                &[(Role::Ancestor(1), miss), (Role::ExternalContent(1), hit)],
            ),
        ];
        assert_eq!(
            walk(&mut arena, n, raw, &up),
            Verdict::Outside {
                role: Role::Ancestor(1),
                level: 1
            }
        );
        assert!(!arena.fault());
    }

    #[test]
    fn singular_transforms_need_no_inverse() {
        let n = Normalization::None(Word::F32);
        let mut arena = arena_with(4096, &[]);
        let mut collapse = [0.0; 16];
        (collapse[12], collapse[13], collapse[14], collapse[15]) = (5.0, 6.0, 7.0, 1.0);
        let mut rank1 = collapse;
        (rank1[0], rank1[1]) = (1.0, 2.0);
        let up = |t: [f64; 16], c: [f64; 3], h: f64| {
            [
                level(None, &[]),
                level(Some(t), &[(Role::Ancestor(1), bx(c, [h, 0.0, 0.0]))]),
            ]
        };
        for raw in [[0.0, 0.0, 0.0], [3.5, -2.0, 1e3], [-7.0, 0.25, 0.0]] {
            let raw = RawVertex::F32(raw);
            assert_eq!(
                walk(&mut arena, n, raw, &up(collapse, [5.0, 6.0, 7.0], 0.0)),
                Verdict::Contained
            );
        }
        // rank 1: (x, y', z') ↦ (5 + x, 6 + 2x, 7).
        let raw = RawVertex::F32([2.0, 0.0, 0.0]);
        assert_eq!(
            walk(&mut arena, n, raw, &up(rank1, [7.0, 10.0, 7.0], 0.0)),
            Verdict::Contained
        );
        assert!(walk(&mut arena, n, raw, &up(rank1, [7.0, 9.0, 7.0], 0.0)) != Verdict::Contained);
        let u = uri(&[], n);
        let s = spans(&u, &up(collapse, [5.0, 6.0, 7.0], 0.0), 4096);
        // Level 1: P_r = T[r][3]·W only, with W = (0, 0); D₀ = Σ[(0, 2), (0, 2)] = (0, 3).
        assert_eq!(s[1], 4);
        // A zero node scale (W10) zeroes a composite column; the next matrix's raw L entries
        // still count toward s_step (accepted reading 9).
        let flat = Local::Trs {
            t: [0.0; 3],
            q: [0.0, 0.0, 0.0, 1.0],
            s: [0.0, 1.0, 1.0],
        };
        let mut m = [0.0; 16];
        (m[0], m[4], m[5], m[10], m[15]) = (1.0, succ(1.0), 1.0, 1.0, 1.0);
        let (g, _) = node_step_bound(&composite_identity(), &flat);
        assert!((0..3).all(|r| g[r][0] == Z));
        assert_eq!(node_step_bound(&g, &Local::Matrix(m)).1, 53);
        let mut arena = arena_with(4096, &[flat, Local::Matrix(m)]);
        let raw = RawVertex::F32([9.0, 0.0, 0.0]);
        assert_eq!(
            walk(&mut arena, n, raw, &up(collapse, [5.0, 6.0, 7.0], 0.0)),
            Verdict::Contained
        );
        let own = [level(None, &[(Role::Own, bx([0.0; 3], [0.0; 3]))])];
        assert_eq!(walk(&mut arena, n, raw, &own), Verdict::Contained);
        assert!(!arena.fault());
    }

    // ---- quaternion admission and tiny-quaternion spans ----

    #[test]
    fn quaternion_window_is_exact() {
        let mut arena = Arena::reserve(SpanCaps {
            span_bits: 28,
            node_depth: 1,
        })
        .unwrap();
        let p = |e| 2f64.powi(e);
        for (q, want) in [
            ([0.0, 0.0, 0.0, 1.0], RotationOutcome::Ok),
            ([0.5, 0.5, 0.5, 0.5], RotationOutcome::Ok),
            // Σq² − 1 = 2^-18 exactly, then one ulp beyond.
            ([p(-9), 0.0, 0.0, 1.0], RotationOutcome::Ok),
            (
                [succ(p(-9)), 0.0, 0.0, 1.0],
                RotationOutcome::QuaternionWindow,
            ),
            // A subnormal component pushes the boundary case out by 2^-2148.
            (
                [p(-9), f64::from_bits(1), 0.0, 1.0],
                RotationOutcome::QuaternionWindow,
            ),
            (
                [p(-9), -f64::from_bits(1), -0.0, -1.0],
                RotationOutcome::QuaternionWindow,
            ),
            // w = 1 − a: 2a − a² below and above 2^-18.
            ([0.0, 0.0, 0.0, 1.0 - p(-19)], RotationOutcome::Ok),
            (
                [0.0, 0.0, 0.0, 1.0 - p(-19) - p(-37)],
                RotationOutcome::QuaternionWindow,
            ),
            ([0.0, 0.0, 0.0, 0.5], RotationOutcome::QuaternionWindow),
            ([0.0, -0.0, 0.0, -0.0], RotationOutcome::InvalidZeroOrRange),
            (
                [0.0, 0.0, 0.0, succ(1.0)],
                RotationOutcome::InvalidZeroOrRange,
            ),
            (
                [0.0, 0.0, f64::NAN, 1.0],
                RotationOutcome::InvalidZeroOrRange,
            ),
            (
                [0.0, f64::INFINITY, 0.0, 0.0],
                RotationOutcome::InvalidZeroOrRange,
            ),
        ] {
            assert_eq!(admit_rotation(q, &mut arena), want, "{q:?}");
        }
        assert!(!arena.fault());
    }

    #[test]
    fn tiny_quaternion_component_is_numeric_span_under_a_sphere() {
        let tiny = Local::Trs {
            t: [1.0, 2.0, 3.0],
            q: [2f64.powi(-1000), 0.0, 0.0, 1.0],
            s: [1.0; 3],
        };
        let mut arena = Arena::reserve(caps(4096)).unwrap();
        assert_eq!(
            admit_rotation([2f64.powi(-1000), 0.0, 0.0, 1.0], &mut arena),
            RotationOutcome::Ok
        );
        let n = Normalization::None(Word::F32);
        let u = uri(&[tiny], n);
        assert!(u.s_inst > 2000);
        let boxed = [level(
            None,
            &[(Role::Own, bx([0.0; 3], [2f64.powi(130); 3]))],
        )];
        let sphere = [level(
            None,
            &[(
                Role::Own,
                VolumeFacts::Sphere([0.0, 0.0, 0.0, 2f64.powi(130)]),
            )],
        )];
        let s = spans(&u, &boxed, 4096);
        assert!(s[0] > 2000 && s[0] <= 4096);
        assert_eq!(
            level_spans(&u, &sphere, caps(4096)).err(),
            Some(Exclusion::NumericSpan)
        );
        assert!(spans(&u, &sphere, 1 << 20)[0] > 4096);
        // The box chain walks within the arena sized from exactly the admitted span.
        let cap = u64::from(s[0].max(u.s_inst));
        let mut arena = arena_with(cap, &[tiny]);
        let mut rng = Rng(5);
        for _ in 0..200 {
            let raw = random_raw(&mut rng, n);
            walk(&mut arena, n, raw, &boxed);
        }
        assert!(!arena.fault());
    }

    // ---- reservation ----

    #[test]
    fn reservation_size_and_refusal() {
        let a = Arena::reserve(caps(4096)).unwrap();
        // (16·65 + 32) values × (65 + 2) limbs, plus 4 × (35 + 2) admission limbs.
        assert_eq!(a.d.limbs(), 1072 * 67 + 4 * 37);
        assert!(!a.fault());
        let refused = |c: SpanCaps, limit: usize| Arena::reserve_within(c, limit).err();
        assert_eq!(refused(caps(4096), 1000), Some(AllocRefused));
        assert_eq!(
            refused(caps(4096), 1072 * 67 + 4 * 37 - 1),
            Some(AllocRefused)
        );
        assert!(Arena::reserve_within(caps(4096), 1072 * 67 + 4 * 37).is_ok());
        let huge = |span_bits, node_depth| {
            Arena::reserve(SpanCaps {
                span_bits,
                node_depth,
            })
            .err()
        };
        assert_eq!(huge(u64::MAX, 64), Some(AllocRefused));
        assert_eq!(huge(4096, u64::MAX), Some(AllocRefused));
        assert_eq!(huge(1 << 40, 1 << 20), Some(AllocRefused));
    }

    #[test]
    fn units_are_checked() {
        assert_eq!(node_units(0), 1);
        assert_eq!(node_units(1), 128);
        assert_eq!(node_units(64), 128);
        assert_eq!(node_units(65), 512);
        assert_eq!(node_units(u32::MAX), 128 * (1 << 26) * (1 << 26));
        let u = uri(&[], Normalization::None(Word::F32));
        let levels = [level(None, &[(Role::Own, bx([0.0; 3], [1.0; 3]))])];
        let s = level_spans(&u, &levels, caps(4096)).unwrap();
        assert_eq!(s.iter().collect::<Vec<_>>(), [277]);
        assert_eq!(vertex_units(3, &s), Some(3 * 25));
        assert_eq!(vertex_units(u64::MAX, &s), None);
        assert_eq!(vertex_units(0, &s), Some(0));
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "capacity exceeded"))]
    fn arena_below_the_bound_faults_instead_of_wrapping() {
        let u = uri(&[WORKED_NODE], Normalization::U16);
        let levels = worked_levels(ADJUSTED_CENTRE);
        assert_eq!(spans(&u, &levels, 29), [23, 29]);
        // One limb holds a 0-bit cap only; the walk must breach, never wrap.
        let mut a = arena_with(0, &[WORKED_NODE]);
        walk(
            &mut a,
            Normalization::U16,
            RawVertex::Int([65535; 3]),
            &levels,
        );
        assert!(a.fault());
    }

    // ---- the static bound covers every runtime value at exactly its cap ----

    fn random_f64(rng: &mut Rng) -> f64 {
        let sign = if rng.below(2) == 0 { 1.0 } else { -1.0 };
        let m = (rng.next() >> 11) as f64;
        sign * match rng.below(10) {
            0 => 0.0,
            1 => -0.0,
            2 => 1.0,
            3 => 0.5,
            4 => f64::from_bits(1 + rng.below(1 << 20)),
            5 => m * 2f64.powi(-(rng.below(300) as i32) - 53),
            _ => m * 2f64.powi(rng.below(80) as i32 - 93),
        }
    }
    fn random_local(rng: &mut Rng) -> Local {
        match rng.below(4) {
            0 => Local::Skip,
            1 => {
                let mut m: [f64; 16] = std::array::from_fn(|_| random_f64(rng));
                (m[3], m[7], m[11], m[15]) = (0.0, 0.0, 0.0, 1.0);
                Local::Matrix(m)
            }
            _ => {
                let mut q = [0.0; 4].map(|_| random_f64(rng).clamp(-1.0, 1.0));
                q[3] = q[3].abs().max(0.25);
                Local::Trs {
                    t: [0.0; 3].map(|_| random_f64(rng)),
                    q,
                    s: [0.0; 3].map(|_| random_f64(rng)),
                }
            }
        }
    }
    fn random_raw(rng: &mut Rng, n: Normalization) -> RawVertex {
        let int =
            |rng: &mut Rng, lo: i64, hi: i64| (lo + rng.below((hi - lo + 1) as u64) as i64) as i32;
        let range = match n {
            Normalization::None(Word::F32) => {
                return RawVertex::F32([0; 3].map(|_| loop {
                    let x = f32::from_bits(rng.next() as u32);
                    if x.is_finite() {
                        break x;
                    }
                }))
            }
            Normalization::None(Word::Int8) => (-128, 255),
            Normalization::None(Word::Int16) => (-32768, 65535),
            Normalization::I8 => (-128, 127),
            Normalization::U8 => (0, 255),
            Normalization::I16 => (-32768, 32767),
            Normalization::U16 => (0, 65535),
        };
        RawVertex::Int([0; 3].map(|_| int(rng, range.0, range.1)))
    }
    fn random_volume(rng: &mut Rng) -> VolumeFacts {
        let big = 2f64.powi(rng.below(900) as i32);
        let c = [0.0; 3].map(|_| random_f64(rng));
        if rng.below(2) == 0 {
            VolumeFacts::Sphere([c[0], c[1], c[2], big])
        } else {
            let mut b = [0.0; 12];
            b[..3].copy_from_slice(&c);
            let rows = [[0, 1, 2], [1, 2, 0], [2, 0, 1], [0, 2, 1]][rng.below(4) as usize];
            for (i, &j) in rows.iter().enumerate() {
                b[3 + 3 * i + j] = if rng.below(5) == 0 { 0.0 } else { -big };
            }
            VolumeFacts::Box(b)
        }
    }

    #[test]
    fn static_bound_covers_runtime_at_exact_cap() {
        let norms = [
            Normalization::None(Word::F32),
            Normalization::None(Word::Int8),
            Normalization::None(Word::Int16),
            Normalization::I8,
            Normalization::U8,
            Normalization::I16,
            Normalization::U16,
        ];
        let mut rng = Rng(0x0123_4567_89ab_cdef);
        let (mut tested, mut contained) = (0, 0);
        for _ in 0..120 {
            let chain: Vec<Local> = (0..1 + rng.below(4))
                .map(|_| random_local(&mut rng))
                .collect();
            let n = norms[rng.below(7) as usize];
            let u = uri(&chain, n);
            let levels: Vec<LevelFacts> = (0..1 + rng.below(4))
                .map(|l| {
                    let t = (l > 0 && rng.below(4) != 0).then(|| {
                        let mut m: [f64; 16] = std::array::from_fn(|_| random_f64(&mut rng));
                        (m[3], m[7], m[11], m[15]) = (0.0, 0.0, 0.0, 1.0);
                        m
                    });
                    let role = if l == 0 {
                        Role::Own
                    } else {
                        Role::Ancestor(l as u32)
                    };
                    let mut v = vec![(role, random_volume(&mut rng))];
                    if rng.below(2) == 0 {
                        let second = if l == 0 {
                            Role::Content
                        } else {
                            Role::ExternalContent(l as u32)
                        };
                        v.push((second, random_volume(&mut rng)));
                    }
                    level(t, &v)
                })
                .collect();
            let s = spans(&u, &levels, u64::from(u32::MAX - 1));
            let cap = s.iter().copied().fold(u.s_inst, u32::max);
            assert!(level_spans(&u, &levels, caps(u64::from(cap))).is_ok());
            assert_eq!(
                level_spans(&u, &levels, caps(u64::from(cap) - 1)).err(),
                Some(Exclusion::NumericSpan)
            );
            let mut arena = arena_with(u64::from(cap), &chain);
            for _ in 0..12 {
                let raw = random_raw(&mut rng, n);
                tested += 1;
                if walk(&mut arena, n, raw, &levels) == Verdict::Contained {
                    contained += 1;
                }
            }
            assert!(!arena.fault());
        }
        assert!(contained > tested / 4 && contained < tested);
    }
}
