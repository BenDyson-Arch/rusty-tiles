//! P1: guard-by-construction half-edge collapse.
//!
//! One priority-queue pass that never produces what the fold guard refuses, instead of
//! meshopt followed by a whole-mesh guard and protect-and-retry rounds. Every collapse
//! `u -> v` (vertices never move; outputs are a subset of the merged keys) is checked
//! locally, in O(valence^2):
//! - link condition (no pinch, no hole closure, no component collapse);
//! - no face turns over (normal sign) or becomes zero-area;
//! - no directed edge is used by more faces than the source licenses (books);
//! - no coincident face, no face the source holds only reversed;
//! - no new edge between two shared (interface) points unless it is a source edge or a
//!   required contract segment; no new face on three shared points unless it is a source
//!   face; no required segment loses its last face.
//!
//! Border contraction is part of the same pass: a child border vertex the parent contract
//! removes (`Class::Remove`) may only collapse along its own arc, onto a vertex of the
//! same chord or a chord endpoint; it is collapsed with priority whenever valid, and the
//! pass fails if any remains. Open source boundaries collapse only along themselves.
use super::super::{
    interfaces::{InterfaceRow, Key},
    store::TileGeometry,
};
use super::{interface_points, Merged};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::{cmp::Ordering, collections::BinaryHeap};

/// Weight of the perpendicular boundary planes, times edge length squared.
const BOUNDARY_WEIGHT: f64 = 10.;

/// Steiner vertices with the removal vertex each replaced.
pub(crate) type Added = Vec<([f32; 3], u32)>;
/// Per merged vertex: its contract mapping (`Mapping`).
pub(crate) type Mapped = Vec<Option<(Key, Key, f64, u8)>>;

/// A vertex or face id. Ids index the merged children of one parent job (at most eight
/// capped tiles, so far below `u32::MAX`); a larger count saturates instead of wrapping.
fn id(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Class {
    Free,
    Locked,
    /// Must be removed onto its chord (endpoint vertex ids).
    Remove([u32; 2]),
}

pub(crate) struct Problem<'a> {
    pub positions: &'a [[f32; 3]],
    pub triangles: &'a [[u32; 3]],
    pub class: Vec<Class>,
    /// Shared (interface) points: new edges/faces between them need a licence.
    pub interface: Vec<bool>,
    /// Required segments (min, max) by vertex id.
    pub segments: HashSet<(u32, u32)>,
    pub target: usize,
    /// Accept a result up to this many faces when the target is not reached.
    pub limit: usize,
    /// Minimum cosine between a moved face's old and new normals.
    pub min_cos: f64,
    /// Seam-owner licences (N1): a new edge / face on shared points this cell owns.
    pub owns_segment: Option<&'a dyn Fn(u32, u32) -> bool>,
    pub owns_face: Option<&'a dyn Fn([u32; 3]) -> bool>,
    /// Canonical keys (for the exact side test of moved book wings).
    pub keys: &'a [Key],
    /// Rescue a removal vertex fanned only by shared points with a Steiner vertex.
    pub steiner: bool,
    /// Snaps a Steiner position (replacing the given vertex) onto its canonical lattice.
    pub snap: Option<&'a dyn Fn([f32; 3], u32) -> [f32; 3]>,
}

#[derive(Default, Debug, Clone, Copy, serde::Serialize)]
pub(crate) struct Stats {
    pub collapses: u64,
    pub checks: u64,
    pub refused: u64,
    pub removed_left: u64,
    pub faces: u64,
    pub rescues: u64,
    pub flips: u64,
    pub steiner: u64,
    /// Refusals by reason: 1 vanish, 2 link, 3 pinch, 4 zero area, 5 flip, 6 reversed,
    /// 7 shared face, 8 coincident, 9 shared edge, 10 book, 11 segment, 12 not an edge.
    pub why: [u64; 13],
}

#[derive(PartialEq)]
struct Entry {
    cost: f64,
    u: u32,
    stamp: u32,
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> Ordering {
        // min-heap on cost, then vertex for determinism
        o.cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
            .then(o.u.cmp(&self.u))
    }
}

type Quadric = [f64; 11];

fn plane_quadric(n: [f64; 3], d: f64, w: f64) -> Quadric {
    let [a, b, c] = n;
    [
        w * a * a,
        w * a * b,
        w * a * c,
        w * a * d,
        w * b * b,
        w * b * c,
        w * b * d,
        w * c * c,
        w * c * d,
        w * d * d,
        w,
    ]
}
fn add(q: &mut Quadric, r: &Quadric) {
    for i in 0..11 {
        q[i] += r[i];
    }
}
fn eval(q: &Quadric, p: [f64; 3]) -> f64 {
    let [x, y, z] = p;
    q[0] * x * x
        + 2. * q[1] * x * y
        + 2. * q[2] * x * z
        + 2. * q[3] * x
        + q[4] * y * y
        + 2. * q[5] * y * z
        + 2. * q[6] * y
        + q[7] * z * z
        + 2. * q[8] * z
        + q[9]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sorted(f: [u32; 3]) -> [u32; 3] {
    let mut s = f;
    s.sort_unstable();
    s
}

struct State<'a> {
    p: &'a Problem<'a>,
    class: Vec<Class>,
    iface: Vec<bool>,
    /// Steiner vertices added by the rescue (ids n, n+1, ...), with the vertex each replaced.
    added: Added,
    pos: Vec<[f64; 3]>,
    faces: Vec<[u32; 3]>,
    alive: Vec<bool>,
    vf: Vec<Vec<u32>>,
    dir: HashMap<(u32, u32), u32>,
    src_dir: HashMap<(u32, u32), u32>,
    src_face: HashMap<[u32; 3], [u32; 2]>,
    face_set: HashMap<[u32; 3], u32>,
    q: Vec<Quadric>,
    stamp: Vec<u32>,
    dead: Vec<bool>,
    alive_faces: usize,
    removes_left: usize,
    stats: Stats,
}

/// Orientation bit of a face relative to its sorted triple (0: even permutation).
fn parity(f: [u32; 3]) -> usize {
    let s = sorted(f);
    // rotate f so it starts at s[0]
    let k = f.iter().position(|&x| x == s[0]).unwrap_or(0);
    usize::from(f[(k + 1) % 3] != s[1])
}

impl<'a> State<'a> {
    /// One use fewer of directed edge `e` (dropped at zero).
    fn dec_dir(&mut self, e: (u32, u32)) {
        if let Some(c) = self.dir.get_mut(&e) {
            *c -= 1;
            if *c == 0 {
                self.dir.remove(&e);
            }
        }
    }

    /// One face fewer on sorted triple `s` (dropped at zero).
    fn dec_face(&mut self, s: [u32; 3]) {
        if let Some(c) = self.face_set.get_mut(&s) {
            *c -= 1;
            if *c == 0 {
                self.face_set.remove(&s);
            }
        }
    }

    fn normal(&self, f: [u32; 3]) -> [f64; 3] {
        let [a, b, c] = f.map(|v| self.pos[v as usize]);
        cross(sub(b, a), sub(c, a))
    }

    fn live_faces(&self, v: u32) -> impl Iterator<Item = u32> + '_ {
        self.vf[v as usize]
            .iter()
            .copied()
            .filter(move |&f| self.alive[f as usize] && self.faces[f as usize].contains(&v))
    }

    fn neighbours(&self, v: u32, out: &mut Vec<u32>) {
        out.clear();
        for f in self.live_faces(v) {
            for &w in &self.faces[f as usize] {
                if w != v && !out.contains(&w) {
                    out.push(w);
                }
            }
        }
    }

    fn undirected(&self, a: u32, b: u32) -> u32 {
        self.dir.get(&(a, b)).copied().unwrap_or(0) + self.dir.get(&(b, a)).copied().unwrap_or(0)
    }

    fn is_boundary_edge(&self, a: u32, b: u32) -> bool {
        self.undirected(a, b) == 1
    }

    fn on_boundary(&self, v: u32, nb: &[u32]) -> bool {
        nb.iter().any(|&w| self.undirected(v, w) == 1)
    }

    fn src_edge(&self, a: u32, b: u32) -> bool {
        self.src_dir.contains_key(&(a, b)) || self.src_dir.contains_key(&(b, a))
    }

    fn allowed_target(&self, u: u32, v: u32, u_boundary: bool) -> bool {
        match self.class[u as usize] {
            Class::Locked => false,
            Class::Remove(chord) => {
                v == chord[0] || v == chord[1] || self.class[v as usize] == Class::Remove(chord)
            }
            Class::Free => {
                if matches!(self.class[v as usize], Class::Remove(_)) {
                    // never pull a free vertex onto a border vertex that will move
                    return false;
                }
                !u_boundary || self.is_boundary_edge(u, v)
            }
        }
    }

    /// Candidate targets of `u`, cheapest first.
    fn candidates(&self, u: u32) -> Vec<(f64, u32)> {
        if self.dead[u as usize] || self.class[u as usize] == Class::Locked {
            return Vec::new();
        }
        let mut nb = Vec::new();
        self.neighbours(u, &mut nb);
        let ub = self.on_boundary(u, &nb);
        let remove = matches!(self.class[u as usize], Class::Remove(_));
        let mut out: Vec<(f64, u32)> = nb
            .iter()
            .filter(|&&v| self.allowed_target(u, v, ub))
            .map(|&v| {
                let mut q = self.q[u as usize];
                add(&mut q, &self.q[v as usize]);
                let c = eval(&q, self.pos[v as usize]).max(0.);
                // Border removals go first whenever valid.
                (if remove { c * 1e-12 - 1e30 } else { c }, v)
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        out
    }

    fn push(&mut self, heap: &mut BinaryHeap<Entry>, u: u32) {
        self.stamp[u as usize] += 1;
        if let Some(&(cost, _)) = self.candidates(u).first() {
            heap.push(Entry {
                cost,
                u,
                stamp: self.stamp[u as usize],
            });
        }
    }

    /// Whether `u -> v` keeps every guard property.
    fn valid(&mut self, u: u32, v: u32) -> bool {
        match self.check(u, v) {
            0 => true,
            r => {
                self.stats.why[r as usize] += 1;
                false
            }
        }
    }

    /// 0 when valid, else the refusal reason (`Stats::why` index).
    fn check(&mut self, u: u32, v: u32) -> u8 {
        self.stats.checks += 1;
        let star_u: Vec<u32> = self.live_faces(u).collect();
        let (shared, moved): (Vec<u32>, Vec<u32>) = star_u
            .iter()
            .partition(|&&f| self.faces[f as usize].contains(&v));
        if shared.is_empty() {
            return 12;
        }
        let star_v = self.live_faces(v).count();
        if moved.is_empty() && star_v == shared.len() {
            return 1; // the component would vanish
        }
        // Link condition.
        let (mut nu, mut nv) = (Vec::new(), Vec::new());
        self.neighbours(u, &mut nu);
        self.neighbours(v, &mut nv);
        let expected: Vec<u32> = shared
            .iter()
            .map(|&f| {
                self.faces[f as usize]
                    .iter()
                    .copied()
                    .find(|&w| w != u && w != v)
                    .unwrap_or(u32::MAX)
            })
            .collect();
        for &w in &nu {
            if w != v && nv.contains(&w) && !expected.contains(&w) {
                return 2;
            }
        }
        // Boundary pinch: two boundary vertices joined by an interior edge.
        if !self.is_boundary_edge(u, v) && self.on_boundary(u, &nu) && self.on_boundary(v, &nv) {
            return 3;
        }
        // Local directed-edge accounting.
        let mut delta: HashMap<(u32, u32), i32> = HashMap::default();
        let mut new_faces: Vec<[u32; 3]> = Vec::with_capacity(moved.len());
        for &f in &shared {
            let t = self.faces[f as usize];
            for j in 0..3 {
                *delta.entry((t[j], t[(j + 1) % 3])).or_default() -= 1;
            }
        }
        let iface = &self.iface;
        for &f in &moved {
            let t = self.faces[f as usize];
            let g = t.map(|w| if w == u { v } else { w });
            let (n0, n1) = (self.normal(t), self.normal(g));
            let (l0, l1) = (dot(n0, n0).sqrt(), dot(n1, n1).sqrt());
            if l1 <= 1e-12 * l0.max(1e-30) || l1 == 0. {
                return 4;
            }
            if l0 > 0. && dot(n0, n1) < self.p.min_cos * l0 * l1 {
                return 5;
            }
            let s = sorted(g);
            let src = self.src_face.get(&s).copied().unwrap_or([0, 0]);
            let d = parity(g);
            if src[d] == 0 && src[1 - d] != 0 {
                return 6; // reversed against the source
            }
            if src == [0, 0]
                && g.iter().all(|&w| iface[w as usize])
                && !self.p.owns_face.is_some_and(|own| own(g))
            {
                return 7; // new face on three shared points
            }
            if self.face_set.contains_key(&s) || new_faces.iter().any(|h| sorted(*h) == s) {
                return 8; // coincident
            }
            for j in 0..3 {
                *delta.entry((t[j], t[(j + 1) % 3])).or_default() -= 1;
                *delta.entry((g[j], g[(j + 1) % 3])).or_default() += 1;
            }
            // new edges incident to v
            for &w in &g {
                if w != v
                    && iface[v as usize]
                    && iface[w as usize]
                    && !self.src_edge(v, w)
                    && !self.p.segments.contains(&(v.min(w), v.max(w)))
                    && !self.p.owns_segment.is_some_and(|own| own(v, w))
                {
                    return 9;
                }
            }
            new_faces.push(g);
        }
        let mut deltas: Vec<((u32, u32), i32)> = delta.iter().map(|(&e, &d)| (e, d)).collect();
        deltas.sort_unstable();
        for ((a, b), d) in deltas {
            if d > 0 {
                let now = i64::from(self.dir.get(&(a, b)).copied().unwrap_or(0)) + i64::from(d);
                let src = self.src_dir.get(&(a, b)).copied().unwrap_or(0);
                // Between two shared points (not a required chord) the directed count
                // may not exceed the source's (the guard's boundary rule).
                let shared = iface[a as usize]
                    && iface[b as usize]
                    && !self.p.segments.contains(&(a.min(b), a.max(b)))
                    && !(src == 0
                        && !self.src_dir.contains_key(&(b, a))
                        && self.p.owns_segment.is_some_and(|own| own(a, b)));
                let lic = i64::from(if shared { src } else { src.max(1) });
                if now > lic {
                    return 10; // book
                }
            }
        }
        // Required segments keep a face.
        let mut touched: Vec<(u32, u32)> = delta
            .keys()
            .map(|&(a, b)| (a.min(b), a.max(b)))
            .filter(|e| self.p.segments.contains(e))
            .collect();
        touched.sort_unstable();
        touched.dedup();
        for (a, b) in touched {
            let after = i64::from(self.undirected(a, b))
                + i64::from(delta.get(&(a, b)).copied().unwrap_or(0))
                + i64::from(delta.get(&(b, a)).copied().unwrap_or(0));
            if after <= 0 {
                return 11;
            }
        }
        0
    }

    /// Flip edge (r, x) when exactly two faces hold it and the flip keeps every guard
    /// property. Returns whether it flipped.
    fn flip(&mut self, r: u32, x: u32) -> bool {
        if self.p.segments.contains(&(r.min(x), r.max(x))) {
            return false;
        }
        let both: Vec<u32> = self
            .live_faces(r)
            .filter(|&f| self.faces[f as usize].contains(&x))
            .collect();
        if both.len() != 2 {
            return false;
        }
        // f1 cyclic (r, x, p); f2 cyclic (x, r, q)
        let rot = |t: [u32; 3], a: u32| {
            let k = t.iter().position(|&w| w == a).unwrap_or(0);
            [t[k], t[(k + 1) % 3], t[(k + 2) % 3]]
        };
        let (mut f1, mut f2) = (both[0], both[1]);
        if rot(self.faces[f1 as usize], r)[1] != x {
            std::mem::swap(&mut f1, &mut f2);
        }
        let t1 = rot(self.faces[f1 as usize], r);
        let t2 = rot(self.faces[f2 as usize], x);
        if t1[1] != x || t2[1] != r {
            return false;
        }
        let (p, q) = (t1[2], t2[2]);
        if p == q || self.undirected(p, q) > 0 {
            return false;
        }
        let g1 = [r, q, p];
        let g2 = [q, x, p];
        let iface = &self.iface;
        if iface[p as usize]
            && iface[q as usize]
            && !self.src_edge(p, q)
            && !self.p.owns_segment.is_some_and(|own| own(p, q))
        {
            return false;
        }
        let quad = {
            let (a, b) = (self.normal(t1), self.normal(t2));
            [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
        };
        for g in [g1, g2] {
            let n = self.normal(g);
            if dot(n, n) <= 0. || dot(n, quad) <= 0. {
                return false;
            }
            let s = sorted(g);
            if self.face_set.contains_key(&s) {
                return false;
            }
            let src = self.src_face.get(&s).copied().unwrap_or([0, 0]);
            let d = parity(g);
            if src[d] == 0 && src[1 - d] != 0 {
                return false;
            }
            if src == [0, 0]
                && g.iter().all(|&w| iface[w as usize])
                && !self.p.owns_face.is_some_and(|own| own(g))
            {
                return false;
            }
        }
        for (a, b) in [(p, q), (q, p)] {
            let now = self.dir.get(&(a, b)).copied().unwrap_or(0) + 1;
            if now > self.src_dir.get(&(a, b)).copied().unwrap_or(0).max(1) {
                return false;
            }
        }
        // apply
        for (f, g) in [(f1, g1), (f2, g2)] {
            let t = self.faces[f as usize];
            for j in 0..3 {
                let e = (t[j], t[(j + 1) % 3]);
                self.dec_dir(e);
            }
            let s = sorted(t);
            self.dec_face(s);
            self.faces[f as usize] = g;
            for j in 0..3 {
                *self.dir.entry((g[j], g[(j + 1) % 3])).or_default() += 1;
            }
            *self.face_set.entry(sorted(g)).or_default() += 1;
        }
        self.vf[p as usize].push(f2);
        self.vf[q as usize].push(f1);
        self.stats.flips += 1;
        true
    }

    /// Rescue of a removal vertex `r` stuck between shared points: replace it by a new
    /// interior (Steiner) vertex `s` just inside the arc and close the boundary with the
    /// sliver face (s, x, y) on its arc neighbours, so no face spans three shared points.
    fn steiner(&mut self, r: u32) -> bool {
        if !matches!(self.class[r as usize], Class::Remove(_)) || self.dead[r as usize] {
            return false;
        }
        let star: Vec<u32> = self.live_faces(r).collect();
        let mut nb = Vec::new();
        self.neighbours(r, &mut nb);
        let arc: Vec<u32> = nb
            .iter()
            .copied()
            .filter(|&w| self.is_boundary_edge(r, w))
            .collect();
        if arc.len() != 2 {
            return false;
        }
        // x -> r and r -> y are the existing boundary directions
        let (x, y) = if self.dir.contains_key(&(arc[0], r)) && self.dir.contains_key(&(r, arc[1])) {
            (arc[0], arc[1])
        } else if self.dir.contains_key(&(arc[1], r)) && self.dir.contains_key(&(r, arc[0])) {
            (arc[1], arc[0])
        } else {
            return false;
        };
        if self.undirected(x, y) > 0 {
            return false;
        }
        let interior: Vec<u32> = nb.iter().copied().filter(|&w| w != x && w != y).collect();
        if interior.is_empty() {
            return false;
        }
        let pr = self.pos[r as usize];
        let mut c = [0f64; 3];
        for &w in &interior {
            for i in 0..3 {
                c[i] += self.pos[w as usize][i] / interior.len() as f64;
            }
        }
        // f32 (lattice) position strictly inside the chord: from the chord midpoint toward
        // the fan's interior (any arc bend), else from r; the first candidate whose closing
        // face and moved faces keep the fan's orientation, distinct from every live neighbour.
        let mut fan = [0f64; 3];
        for &f in &star {
            let m = self.normal(self.faces[f as usize]);
            for i in 0..3 {
                fan[i] += m[i];
            }
        }
        let (px, py) = (self.pos[x as usize], self.pos[y as usize]);
        let mid = [
            (px[0] + py[0]) / 2.,
            (px[1] + py[1]) / 2.,
            (px[2] + py[2]) / 2.,
        ];
        let sid = id(self.pos.len());
        let new_face = [sid, x, y];
        let mut chosen = None;
        let chord = dot(sub(px, py), sub(px, py)).sqrt();
        'search: for base in [mid, pr] {
            let toward = sub(c, base);
            let reach = dot(toward, toward).sqrt();
            if reach <= 0. {
                continue;
            }
            // absolute offsets on the chord's scale first, then fractions of the reach
            let steps = [0.05, 0.1, 0.2, 0.4]
                .map(|k| (k * chord / reach).min(0.45))
                .into_iter()
                .chain([0.1, 0.2, 0.3, 0.45]);
            for frac in steps {
                let mut ps = [0f32; 3];
                for i in 0..3 {
                    ps[i] = (base[i] + frac * toward[i]) as f32;
                }
                if let Some(snap) = self.p.snap {
                    ps = snap(ps, r);
                }
                let q = ps.map(f64::from);
                if q == pr || nb.iter().any(|&w| self.pos[w as usize] == q) {
                    continue;
                }
                let [a, b, cc] = new_face.map(|v| if v == sid { q } else { self.pos[v as usize] });
                let n = cross(sub(b, a), sub(cc, a));
                if dot(n, n) <= 0. || dot(n, fan) <= 0. {
                    continue;
                }
                for &f in &star {
                    let t = self.faces[f as usize];
                    let g = t.map(|w| if w == r { q } else { self.pos[w as usize] });
                    let (n0, n1) = (self.normal(t), cross(sub(g[1], g[0]), sub(g[2], g[0])));
                    if dot(n1, n1) <= 0. || dot(n0, n1) <= 0. {
                        continue 'search;
                    }
                }
                chosen = Some((ps, q));
                break 'search;
            }
        }
        let Some((ps32, psd)) = chosen else {
            return false;
        };
        let lic = self.src_dir.get(&(x, y)).copied().unwrap_or(0).max(1);
        if self.dir.get(&(x, y)).copied().unwrap_or(0) + 1 > lic {
            return false;
        }
        if self.iface[x as usize]
            && self.iface[y as usize]
            && !self.src_edge(x, y)
            && !self.p.segments.contains(&(x.min(y), x.max(y)))
        {
            return false;
        }
        // apply: new vertex
        self.pos.push(psd);
        self.added.push((ps32, r));
        self.class.push(Class::Free);
        self.iface.push(false);
        self.vf.push(Vec::new());
        let qr = self.q[r as usize];
        self.q.push(qr);
        self.stamp.push(0);
        self.dead.push(false);
        for f in star {
            let t = self.faces[f as usize];
            for j in 0..3 {
                let e = (t[j], t[(j + 1) % 3]);
                self.dec_dir(e);
            }
            let so = sorted(t);
            self.dec_face(so);
            let g = t.map(|w| if w == r { sid } else { w });
            self.faces[f as usize] = g;
            for j in 0..3 {
                *self.dir.entry((g[j], g[(j + 1) % 3])).or_default() += 1;
            }
            *self.face_set.entry(sorted(g)).or_default() += 1;
            self.vf[sid as usize].push(f);
        }
        let fid = id(self.faces.len());
        self.faces.push(new_face);
        self.alive.push(true);
        self.alive_faces += 1;
        for j in 0..3 {
            *self
                .dir
                .entry((new_face[j], new_face[(j + 1) % 3]))
                .or_default() += 1;
        }
        *self.face_set.entry(sorted(new_face)).or_default() += 1;
        for v in new_face {
            self.vf[v as usize].push(fid);
        }
        self.dead[r as usize] = true;
        self.removes_left -= 1;
        self.stats.steiner += 1;
        true
    }

    fn apply(&mut self, u: u32, v: u32) {
        self.stats.collapses += 1;
        let star_u: Vec<u32> = self.live_faces(u).collect();
        for f in star_u {
            let t = self.faces[f as usize];
            for j in 0..3 {
                let e = (t[j], t[(j + 1) % 3]);
                self.dec_dir(e);
            }
            let s = sorted(t);
            self.dec_face(s);
            if t.contains(&v) {
                self.alive[f as usize] = false;
                self.alive_faces -= 1;
            } else {
                let g = t.map(|w| if w == u { v } else { w });
                self.faces[f as usize] = g;
                for j in 0..3 {
                    *self.dir.entry((g[j], g[(j + 1) % 3])).or_default() += 1;
                }
                *self.face_set.entry(sorted(g)).or_default() += 1;
                self.vf[v as usize].push(f);
            }
        }
        self.dead[u as usize] = true;
        if matches!(self.class[u as usize], Class::Remove(_)) {
            self.removes_left -= 1;
        }
        let qu = self.q[u as usize];
        add(&mut self.q[v as usize], &qu);
        // compact v's face list
        let mut list = std::mem::take(&mut self.vf[v as usize]);
        list.retain(|&f| self.alive[f as usize] && self.faces[f as usize].contains(&v));
        list.sort_unstable();
        list.dedup();
        self.vf[v as usize] = list;
    }
}

/// Runs the pass. Returns the surviving faces (merged vertex ids) and stats; `None` when
/// a removal vertex is left or the target is not reached.
pub(crate) fn simplify(p: &Problem<'_>) -> (Option<Vec<[u32; 3]>>, Stats) {
    let (out, stats, _) = simplify_added(p);
    (out, stats)
}

/// `simplify`, also returning Steiner vertices (ids from `p.positions.len()` on) and the
/// removed vertex each replaced.
pub(crate) fn simplify_added(p: &Problem<'_>) -> (Option<Vec<[u32; 3]>>, Stats, Added) {
    let n = p.positions.len();
    let pos: Vec<[f64; 3]> = p.positions.iter().map(|q| q.map(f64::from)).collect();
    let mut st = State {
        p,
        class: p.class.clone(),
        iface: p.interface.clone(),
        added: Vec::new(),
        pos,
        faces: p.triangles.to_vec(),
        alive: vec![true; p.triangles.len()],
        vf: vec![Vec::new(); n],
        dir: HashMap::default(),
        src_dir: HashMap::default(),
        src_face: HashMap::default(),
        face_set: HashMap::default(),
        q: vec![[0.; 11]; n],
        stamp: vec![0; n],
        dead: vec![false; n],
        alive_faces: p.triangles.len(),
        removes_left: p
            .class
            .iter()
            .filter(|c| matches!(c, Class::Remove(_)))
            .count(),
        stats: Stats::default(),
    };
    for (i, &t) in p.triangles.iter().enumerate() {
        for j in 0..3 {
            *st.dir.entry((t[j], t[(j + 1) % 3])).or_default() += 1;
            st.vf[t[j] as usize].push(id(i));
        }
        *st.face_set.entry(sorted(t)).or_default() += 1;
        st.src_face.entry(sorted(t)).or_default()[parity(t)] += 1;
        let nrm = st.normal(t);
        let l = dot(nrm, nrm).sqrt();
        if l > 0. {
            let n1 = nrm.map(|x| x / l);
            let d = -dot(n1, st.pos[t[0] as usize]);
            let q = plane_quadric(n1, d, l * 0.5);
            for &v in &t {
                add(&mut st.q[v as usize], &q);
            }
        }
    }
    st.src_dir = st.dir.clone();
    // Boundary edges: perpendicular planes, heavily weighted.
    for (i, &t) in p.triangles.iter().enumerate() {
        let _ = i;
        for j in 0..3 {
            let (a, b) = (t[j], t[(j + 1) % 3]);
            if st.undirected(a, b) == 1 {
                let e = sub(st.pos[b as usize], st.pos[a as usize]);
                let nrm = st.normal(t);
                let pn = cross(e, nrm);
                let l = dot(pn, pn).sqrt();
                if l > 0. {
                    let n1 = pn.map(|x| x / l);
                    let d = -dot(n1, st.pos[a as usize]);
                    let w = dot(e, e) * BOUNDARY_WEIGHT;
                    let q = plane_quadric(n1, d, w);
                    add(&mut st.q[a as usize], &q);
                    add(&mut st.q[b as usize], &q);
                }
            }
        }
    }
    let mut nb = Vec::new();
    for _outer in 0..4 {
        let before = (
            st.stats.collapses,
            st.stats.steiner,
            st.stats.flips,
            st.stats.rescues,
        );
        let mut heap = BinaryHeap::new();
        for u in 0..id(st.pos.len()) {
            if !st.dead[u as usize] && !st.vf[u as usize].is_empty() {
                st.push(&mut heap, u);
            }
        }
        while let Some(e) = heap.pop() {
            if st.alive_faces <= p.target && st.removes_left == 0 {
                break;
            }
            let u = e.u;
            if st.dead[u as usize] || e.stamp != st.stamp[u as usize] {
                continue;
            }
            if st.alive_faces <= p.target && !matches!(st.class[u as usize], Class::Remove(_)) {
                continue;
            }
            let mut done = false;
            for (_, v) in st.candidates(u) {
                if st.valid(u, v) {
                    st.apply(u, v);
                    st.neighbours(v, &mut nb);
                    let list = nb.clone();
                    st.push(&mut heap, v);
                    for w in list {
                        st.push(&mut heap, w);
                    }
                    done = true;
                    break;
                }
                st.stats.refused += 1;
            }
            if !done {
                // stuck until a neighbour changes (it is pushed again then)
                st.stamp[u as usize] += 1;
            }
        }
        // Rescue: a border vertex left on its arc. Simplify the free vertices around it
        // (cheapest valid collapse first), then retry it; a bounded number of rounds.
        let mut rounds = 0;
        while st.removes_left > 0 && rounds < 64 {
            rounds += 1;
            let left: Vec<u32> = (0..id(n))
                .filter(|&v| {
                    !st.dead[v as usize] && matches!(st.class[v as usize], Class::Remove(_))
                })
                .collect();
            let mut progress = false;
            for r in left {
                if st.dead[r as usize] {
                    continue;
                }
                if let Some(v) = st
                    .candidates(r)
                    .into_iter()
                    .map(|(_, v)| v)
                    .find(|&v| st.valid(r, v))
                {
                    st.apply(r, v);
                    progress = true;
                    continue;
                }
                // flip an edge from r to a shared point so r gains a free apex
                let mut ring = Vec::new();
                st.neighbours(r, &mut ring);
                let mut flipped = false;
                for &x in &ring {
                    if st.iface[x as usize] && st.flip(r, x) {
                        flipped = true;
                        break;
                    }
                }
                if flipped {
                    progress = true;
                    continue;
                }
                if p.steiner && st.steiner(r) {
                    progress = true;
                    continue;
                }
                // free vertices in r's 2-ring
                let mut ring2 = ring.clone();
                for &w in &ring {
                    st.neighbours(w, &mut nb);
                    for &x in &nb {
                        if x != r && !ring2.contains(&x) {
                            ring2.push(x);
                        }
                    }
                }
                let mut best: Option<(f64, u32, u32)> = None;
                for &w in &ring2 {
                    if st.dead[w as usize] || st.class[w as usize] != Class::Free {
                        continue;
                    }
                    for (c, v) in st.candidates(w) {
                        if best.is_some_and(|b| b.0 <= c) {
                            break;
                        }
                        if st.valid(w, v) {
                            best = Some((c, w, v));
                            break;
                        }
                    }
                }
                if let Some((_, w, v)) = best {
                    st.apply(w, v);
                    st.stats.rescues += 1;
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        let after = (
            st.stats.collapses,
            st.stats.steiner,
            st.stats.flips,
            st.stats.rescues,
        );
        if st.alive_faces <= p.target
            || after == before
            || (st.removes_left > 0 && after.1 == before.1 && after.2 == before.2)
        {
            break;
        }
    }
    st.stats.removed_left = st.removes_left as u64;
    st.stats.faces = st.alive_faces as u64;
    let out: Vec<[u32; 3]> = st
        .faces
        .iter()
        .zip(&st.alive)
        .filter(|(_, a)| **a)
        .map(|(t, _)| *t)
        .collect();
    let ok = st.removes_left == 0 && out.len() <= p.target.max(p.limit);
    (ok.then_some(out), st.stats, st.added)
}

/// Keys helper for callers.
pub(crate) fn key_index(keys: &[Key]) -> HashMap<Key, u32> {
    keys.iter().enumerate().map(|(i, k)| (*k, id(i))).collect()
}

/// Builds the collapse problem for merged children under the parent contract.
pub(crate) fn problem<'a>(
    m: &'a Merged,
    children: &[TileGeometry],
    interface: &[InterfaceRow],
    mapping: &Mapping<'_>,
    target: usize,
    min_cos: f64,
) -> (Problem<'a>, Mapped, Vec<InterfaceRow>) {
    let ids = key_index(&m.keys);
    let points = interface_points(interface);
    let mut previous: Vec<InterfaceRow> = children
        .iter()
        .flat_map(|c| c.interface.iter().copied())
        .collect();
    super::super::interfaces::merge_rows(&mut previous);
    let child_border = interface_points(&previous);
    let n = m.keys.len();
    let mut class = vec![Class::Free; n];
    let mut iface = vec![false; n];
    let mut mapped = vec![None; n];
    for (v, k) in m.keys.iter().enumerate() {
        if points.contains(k) {
            class[v] = Class::Locked;
            iface[v] = true;
            continue;
        }
        if child_border.contains(k) {
            let mm = mapping(*k);
            mapped[v] = mm;
            if let Some((a, b, _, _)) = mm {
                if let (Some(&a), Some(&b)) = (ids.get(&a), ids.get(&b)) {
                    if a != id(v) && b != id(v) {
                        class[v] = Class::Remove([a, b]);
                    }
                }
            }
        }
    }
    // Nonmanifold edges and pinch vertices stay (conservative).
    let mut und: HashMap<(u32, u32), u32> = HashMap::default();
    for t in &m.triangles {
        for j in 0..3 {
            let (a, b) = (t[j], t[(j + 1) % 3]);
            *und.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    // Inherited folds (a book edge or a coincident group) keep every face unchanged.
    let mut dir: HashMap<(u32, u32), u32> = HashMap::default();
    let mut groups: HashMap<[u32; 3], u32> = HashMap::default();
    for t in &m.triangles {
        for j in 0..3 {
            *dir.entry((t[j], t[(j + 1) % 3])).or_default() += 1;
        }
        let mut s = *t;
        s.sort_unstable();
        *groups.entry(s).or_default() += 1;
    }
    for t in &m.triangles {
        let mut s = *t;
        s.sort_unstable();
        let fold = groups[&s] > 1 || (0..3).any(|j| dir[&(t[j], t[(j + 1) % 3])] > 1);
        if fold {
            for &v in t {
                if class[v as usize] == Class::Free {
                    class[v as usize] = Class::Locked;
                }
            }
        }
    }
    let mut boundary_degree = vec![0u32; n];
    let mut edges: Vec<((u32, u32), u32)> = und.iter().map(|(&e, &c)| (e, c)).collect();
    edges.sort_unstable();
    for ((a, b), c) in edges {
        if c > 2 {
            for v in [a, b] {
                if class[v as usize] == Class::Free {
                    class[v as usize] = Class::Locked;
                }
            }
        }
        if c == 1 {
            boundary_degree[a as usize] += 1;
            boundary_degree[b as usize] += 1;
        }
    }
    for v in 0..n {
        if boundary_degree[v] > 2 && class[v] == Class::Free {
            class[v] = Class::Locked;
        }
    }
    let segments: HashSet<(u32, u32)> = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .filter_map(|r| Some((*ids.get(&r.a)?, *ids.get(&r.b)?)))
        .map(|(a, b)| (a.min(b), a.max(b)))
        .collect();
    (
        Problem {
            positions: &m.positions,
            triangles: &m.triangles,
            class,
            interface: iface,
            segments,
            target,
            limit: target,
            keys: &m.keys,
            steiner: false,
            snap: None,
            min_cos,
            owns_segment: None,
            owns_face: None,
        },
        mapped,
        previous,
    )
}

pub(crate) type Mapping<'a> = dyn Fn(Key) -> Option<(Key, Key, f64, u8)> + 'a;

/// `m` with Steiner vertices appended (keys from their f32 positions, ells of the vertex
/// each replaced); `mapped` grows with `None`.
pub(crate) fn extend_added(m: &mut Merged, mapped: &mut Mapped, added: &[([f32; 3], u32)]) {
    for &(p, r) in added {
        m.keys.push(super::key(p));
        m.ells.push(m.ells[r as usize]);
        m.positions.push(p);
        mapped.push(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::hlod::interfaces::key;

    /// A border vertex `r` (to be removed onto chord a–b) whose only faces have the shared
    /// point `x` as apex: removing it by collapse makes the face (a, b, x) on three shared
    /// points, which the seam rule refuses. Vertices: a, r, b, x.
    fn fan_of_shared_points() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        (
            vec![[0., 0., 0.], [1., 0., 0.], [2., 0., 0.], [1., 1., 0.]],
            vec![[0, 1, 3], [1, 2, 3]],
        )
    }

    fn problem<'a>(
        positions: &'a [[f32; 3]],
        triangles: &'a [[u32; 3]],
        keys: &'a [Key],
        steiner: bool,
    ) -> Problem<'a> {
        Problem {
            positions,
            triangles,
            class: vec![
                Class::Locked,
                Class::Remove([0, 2]),
                Class::Locked,
                Class::Locked,
            ],
            interface: vec![true, false, true, true],
            segments: [(0, 2)].into_iter().collect(),
            target: 1,
            limit: 4,
            min_cos: 0.,
            owns_segment: None,
            owns_face: None,
            keys,
            steiner,
            snap: None,
        }
    }

    fn up(positions: &[[f32; 3]], added: &[([f32; 3], u32)], f: [u32; 3]) -> f64 {
        let at = |v: u32| {
            let v = v as usize;
            positions
                .get(v)
                .copied()
                .unwrap_or_else(|| added[v - positions.len()].0)
                .map(f64::from)
        };
        let [a, b, c] = f.map(at);
        cross(sub(b, a), sub(c, a))[2]
    }

    #[test]
    fn a_removal_never_makes_a_face_on_three_shared_points() {
        let (positions, triangles) = fan_of_shared_points();
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let (out, stats) = simplify(&problem(&positions, &triangles, &keys, false));
        assert!(out.is_none(), "{stats:?}");
        assert_eq!(stats.removed_left, 1);
    }

    #[test]
    fn steiner_rescues_a_removal_fanned_only_by_shared_points() {
        let (positions, triangles) = fan_of_shared_points();
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let (out, stats, added) = simplify_added(&problem(&positions, &triangles, &keys, true));
        let faces = out.unwrap_or_else(|| panic!("rescued: {stats:?}"));
        assert_eq!(stats.removed_left, 0);
        assert_eq!(added.len(), 1, "one Steiner vertex");
        assert_eq!(added[0].1, 1, "it replaces the removed vertex");
        let s = added[0].0;
        assert!(s[1] > 0. && s[1] < 1., "strictly inside the fan: {s:?}");
        for f in &faces {
            assert!(!f.contains(&1), "the removed vertex is gone: {faces:?}");
            let shared = f.iter().filter(|&&v| v == 0 || v == 2 || v == 3).count();
            assert!(shared < 3, "no face on three shared points: {faces:?}");
            assert!(
                up(&positions, &added, *f) > 0.,
                "orientation kept: {faces:?}"
            );
        }
        // The required chord a–b still has a face.
        assert!(faces.iter().any(|f| (0..3)
            .any(|j| (f[j], f[(j + 1) % 3]) == (0, 2) || (f[j], f[(j + 1) % 3]) == (2, 0))));
    }

    #[test]
    fn a_removal_collapses_along_its_chord() {
        // a, r, b on the border with an interior free vertex i above r and a top row.
        let positions = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [2., 0., 0.],
            [1., 1., 0.],
            [0., 2., 0.],
            [2., 2., 0.],
        ];
        let triangles = vec![[0, 1, 3], [1, 2, 3], [0, 3, 4], [3, 2, 5], [4, 3, 5]];
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let mut p = problem(&positions, &triangles, &keys, false);
        p.class = vec![
            Class::Locked,
            Class::Remove([0, 2]),
            Class::Locked,
            Class::Free,
            Class::Locked,
            Class::Locked,
        ];
        p.interface = vec![true, false, true, false, true, true];
        p.target = 3;
        let (out, stats) = simplify(&p);
        let faces = out.unwrap_or_else(|| panic!("{stats:?}"));
        assert!(faces.iter().all(|f| !f.contains(&1)), "{faces:?}");
        assert!(
            faces.iter().all(|f| up(&positions, &[], *f) > 0.),
            "{faces:?}"
        );
    }
}
