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
use super::{
    embedding::{self, Point},
    folds, interface_points, Merged,
};
use crate::JobError;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    collections::BinaryHeap,
};

/// Weight of the perpendicular boundary planes, times edge length squared.
const BOUNDARY_WEIGHT: f64 = 10.;

/// Steiner vertices with the removal vertex each replaced.
pub(crate) type Added = Vec<([f32; 3], u32)>;
/// Snaps a Steiner position (replacing the given vertex) to its canonical lattice point,
/// or refuses it.
pub(crate) type Snap<'a> = dyn Fn([f32; 3], u32) -> Option<[f32; 3]> + 'a;
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
    /// Snaps a Steiner position (replacing the given vertex) onto its canonical lattice;
    /// `None` refuses the position (no canonical point at the inherited depth).
    pub snap: Option<&'a Snap<'a>>,
    /// Directed counts of edges between two shared points that the contraction fold guard
    /// licenses (`folds::Source::boundary_edges`, by vertex id); absent pairs have none.
    /// Without it (unit controls) the input counts by id, a required segment at least once.
    pub shared: Option<HashMap<(u32, u32), u32>>,
    /// Whether a canonical key is a protected border key elsewhere in the source, which
    /// a Steiner vertex may not take.
    pub protected: Option<&'a dyn Fn([f32; 3]) -> bool>,
    /// The pass gives up (no result) once it has spent this many work units: faces and
    /// neighbours visited by its adjacency scans and checks. Deterministic, unlike time.
    pub work_limit: u64,
    /// Polled every `POLL_WORK` work units; `true` stops the pass (no result) as an
    /// exhausted budget does (`Stats::cancelled`), for the job's cancellation check.
    pub cancel: Option<&'a dyn Fn() -> bool>,
}

/// Work units between polls of `Problem::cancel`.
const POLL_WORK: u64 = 1 << 16;

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
    /// 7 shared face, 8 coincident, 9 shared edge, 10 book, 11 segment, 12 not an edge,
    /// 13 touching boundary.
    /// Each refused check counts once, under the first test that refused it; that test
    /// depends on the order they run in. A vertex above `RETRY_VALENCE` tests a sample of
    /// its moved faces first, so it may report a different reason than testing every face
    /// in turn would (its accept or refuse decision is the same).
    pub why: [u64; 14],
    /// Work units spent (`State::work`), and whether `Problem::work_limit` stopped the pass.
    pub work: u64,
    pub exhausted: bool,
    /// Whether `Problem::cancel` stopped the pass.
    pub cancelled: bool,
    /// Steiner rescues refused, by the furthest test any position reached: 0 link not one
    /// simple path, 1 an arc end off the vertex's contraction, 2 chord edge exists or has no
    /// licence, 3 no canonical private position, 4 orientation, 5 contact with another face,
    /// 6 a moved face on a book edge (`State::books_hold`).
    pub steiner_why: [u64; 7],
    /// Of `work`: units spent in the rescue rounds, and of those in Steiner rescues.
    pub rescue_work: u64,
    pub steiner_work: u64,
    /// Of `work`: the main loop's units by site (`SiteWork`). With `rescue_work` they sum
    /// to `work`.
    pub site: SiteWork,
}

/// Where the main loop spends its work units, to find a superlinear site in a cell that
/// uses much of its budget. `queue`: the initial queueing of every vertex, once per outer
/// pass; `candidates`: the target scan of each pop; `checks`: the guard checks of each
/// tried target, of which `refused_checks` failed; `apply`: applying a collapse and
/// rescanning the target's neighbours; `requeue`: queueing the target again; `neighbours`:
/// queueing its neighbours again. Counts: `pops` (entries acted on), `checks_run`, and the
/// largest single check and candidate scan, which name a high-valence vertex when one
/// dominates.
#[derive(Default, Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SiteWork {
    pub queue: u64,
    pub candidates: u64,
    pub checks: u64,
    pub refused_checks: u64,
    pub apply: u64,
    pub requeue: u64,
    pub neighbours: u64,
    pub pops: u64,
    pub checks_run: u64,
    pub max_check: u64,
    pub max_candidates: u64,
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

/// A position's weld key with signed zero normalized.
fn private_key(p: [f32; 3]) -> Key {
    p.map(|x| if x == 0. { 0f32 } else { x }.to_bits())
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
    /// Per vertex: the scan epoch that last saw it (`neighbours` deduplicates in O(1)).
    mark: RefCell<Vec<u32>>,
    epoch: Cell<u32>,
    /// Work units so far: faces and neighbours visited by adjacency scans and checks.
    work: Cell<u64>,
    /// Keys a Steiner vertex may not take (`State::taken`).
    taken: Option<HashSet<Key>>,
    /// Per vertex: neighbourhood changes to skip before a stuck vertex is tried again
    /// (`retry_wait`).
    retry_after: Vec<u32>,
    /// Whether `Problem::cancel` returned true, and the work count of its next poll.
    cancelled: Cell<bool>,
    next_poll: Cell<u64>,
    /// Mesh changes so far (collapses, flips, Steiner rescues): a cached star is current
    /// while this is unchanged.
    changes: u64,
    /// The last checked vertex's live faces, with `changes` when they were collected: a
    /// pop checks one vertex against up to `RETRY_VALENCE` targets with no change between.
    star_cache: Option<(u32, u64, Vec<u32>)>,
    /// The directed edges live faces use twice or more (`books_hold` can refuse only a
    /// change that touches one, or makes one).
    books: HashSet<(u32, u32)>,
    /// Per vertex: on a boundary (edges one face uses) whose component reaches a contract
    /// point, in the input; a collapse passes it on to its target (`touching_changed`).
    touching: Vec<bool>,
}

/// Moved faces a high-valence vertex's check samples first (`State::check_star`).
const PROBE: usize = 32;

/// Targets a vertex tries per pop (its cheapest), and the candidate count above which a
/// vertex is not requeued after every change around it.
const RETRY_VALENCE: usize = 64;

/// Changes around a vertex with `candidates` targets before it is requeued or, when stuck,
/// tried again. A fan apex of valence k was requeued after each of its k neighbours'
/// collapses, each requeue scanning all k targets (O(k²) work), and when stuck it also
/// checked every target at O(k) each (O(k³)). With at most `RETRY_VALENCE` targets per
/// pop and a wait of k/16 changes, both cost a bounded amount of work per change.
/// Ordinary vertices (at most `RETRY_VALENCE` candidates) behave as before.
fn retry_wait(tried: usize) -> u32 {
    if tried > RETRY_VALENCE {
        u32::try_from(tried / 16).unwrap_or(u32::MAX)
    } else {
        0
    }
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
            if *c == 1 {
                self.books.remove(&e);
            }
            if *c == 0 {
                self.dir.remove(&e);
            }
        }
    }

    /// One use more of directed edge `e`.
    fn inc_dir(&mut self, e: (u32, u32)) {
        let c = self.dir.entry(e).or_default();
        *c += 1;
        if *c == 2 {
            self.books.insert(e);
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

    /// The live neighbours of `v` in first-seen order. A high-valence vertex (a fan apex)
    /// is scanned in linear time: deduplication marks each neighbour with the scan's epoch.
    fn neighbours(&self, v: u32, out: &mut Vec<u32>) {
        out.clear();
        let epoch = self.epoch.get().wrapping_add(1);
        self.epoch.set(epoch);
        let mut mark = self.mark.borrow_mut();
        if epoch == 0 {
            mark.iter_mut().for_each(|m| *m = 0);
            self.epoch.set(1);
        }
        let epoch = self.epoch.get();
        if mark.len() < self.pos.len() {
            mark.resize(self.pos.len(), 0);
        }
        let mut scanned = 0u64;
        for f in self.live_faces(v) {
            scanned += 1;
            for &w in &self.faces[f as usize] {
                if w != v && mark[w as usize] != epoch {
                    mark[w as usize] = epoch;
                    out.push(w);
                }
            }
        }
        self.work.set(self.work.get().saturating_add(scanned));
    }

    /// Whether the pass must stop: its work budget is spent or the job was cancelled (polled
    /// every `POLL_WORK` units).
    fn exhausted(&self) -> bool {
        if self.cancelled.get() {
            return true;
        }
        let work = self.work.get();
        if work > self.p.work_limit {
            return true;
        }
        if let Some(cancel) = self.p.cancel {
            if work >= self.next_poll.get() {
                self.next_poll.set(work.saturating_add(POLL_WORK));
                if cancel() {
                    self.cancelled.set(true);
                    return true;
                }
            }
        }
        false
    }

    fn undirected(&self, a: u32, b: u32) -> u32 {
        self.dir.get(&(a, b)).copied().unwrap_or(0) + self.dir.get(&(b, a)).copied().unwrap_or(0)
    }

    fn is_boundary_edge(&self, a: u32, b: u32) -> bool {
        self.undirected(a, b) == 1
    }

    /// The vertices on a boundary (edges one live face uses) whose component of boundary
    /// edges holds a contract point.
    fn touching_boundaries(&self) -> Vec<bool> {
        let n = self.vf.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn root(parent: &mut [usize], mut a: usize) -> usize {
            while parent[a] != a {
                parent[a] = parent[parent[a]];
                a = parent[a];
            }
            a
        }
        let mut on = vec![false; n];
        let mut edges: Vec<(u32, u32)> = self
            .dir
            .keys()
            .map(|&(a, b)| (a.min(b), a.max(b)))
            .filter(|&(a, b)| a != b && self.undirected(a, b) == 1)
            .collect();
        edges.sort_unstable();
        edges.dedup();
        for (a, b) in edges {
            on[a as usize] = true;
            on[b as usize] = true;
            let (x, y) = (root(&mut parent, a as usize), root(&mut parent, b as usize));
            parent[x] = y;
        }
        let mut reached = vec![false; n];
        for (v, _) in on.iter().enumerate().filter(|&(_, &o)| o) {
            if self.iface.get(v).copied().unwrap_or(false) {
                let r = root(&mut parent, v);
                reached[r] = true;
            }
        }
        (0..n)
            .map(|v| on[v] && reached[root(&mut parent, v)])
            .collect()
    }

    /// Whether a collapse of `u` onto `v` with directed edge changes `delta` changes a
    /// touching boundary (`touching`) other than by merging `u` into `v`: up to that merge,
    /// every boundary edge there stays one and no edge becomes one. The topology floor
    /// refuses any change to such a boundary's graph (`topology::changes_noting`), and a
    /// collapse never undoes one, so a pass that made one could not be proposed.
    fn touching_changed(&self, u: u32, v: u32, delta: &HashMap<(u32, u32), i32>) -> bool {
        let t = |x: u32| self.touching.get(x as usize).copied().unwrap_or(false);
        let near = t(u) || t(v);
        let mut edges: Vec<(u32, u32)> = delta.keys().map(|&(a, b)| (a.min(b), a.max(b))).collect();
        edges.sort_unstable();
        edges.dedup();
        let merged = |x: u32| if x == u { v } else { x };
        let (mut before, mut after) = (Vec::new(), Vec::new());
        for (a, b) in edges {
            let ends = [a, b].iter().any(|&x| x == u || x == v);
            if !(t(a) || t(b) || (near && ends)) {
                continue;
            }
            let old = i64::from(self.undirected(a, b));
            let new = old
                + i64::from(delta.get(&(a, b)).copied().unwrap_or(0))
                + i64::from(delta.get(&(b, a)).copied().unwrap_or(0));
            if old == 1 {
                let (p, q) = (merged(a), merged(b));
                if p != q {
                    before.push((p.min(q), p.max(q)));
                }
            }
            if new == 1 {
                after.push((a, b));
            }
        }
        before.sort_unstable();
        after.sort_unstable();
        before != after
    }

    fn on_boundary(&self, v: u32, nb: &[u32]) -> bool {
        nb.iter().any(|&w| self.undirected(v, w) == 1)
    }

    fn src_edge(&self, a: u32, b: u32) -> bool {
        self.src_dir.contains_key(&(a, b)) || self.src_dir.contains_key(&(b, a))
    }

    /// Whether every edge a change touches still passes the contraction fold guard's book
    /// rule (`folds::book_refused`, the guard's own predicate) once the change is made:
    /// faces `gone` die, faces `changed` take new corners and faces `added` are created;
    /// `pending` is a vertex the change adds (a Steiner vertex), with its position. An edge
    /// between two shared points is the guard's shared-edge rule instead (the licences
    /// above), unless it is a new segment this cell's seam owner licenses (stage 1), which
    /// the guard treats as any other edge. Only an edge left with two or more faces in one
    /// direction can be refused, so the rest cost a lookup each.
    fn books_hold(
        &self,
        gone: &[u32],
        changed: &[(u32, [u32; 3])],
        added: &[[u32; 3]],
        pending: Option<(u32, [f32; 3])>,
    ) -> bool {
        let mut delta: HashMap<(u32, u32), i32> = HashMap::default();
        let mut bump = |t: [u32; 3], by: i32| {
            for j in 0..3 {
                *delta.entry((t[j], t[(j + 1) % 3])).or_default() += by;
            }
        };
        for &f in gone {
            bump(self.faces[f as usize], -1);
        }
        for &(f, g) in changed {
            bump(self.faces[f as usize], -1);
            bump(g, 1);
        }
        for &t in added {
            bump(t, 1);
        }
        let after = |a: u32, b: u32| {
            i64::from(self.dir.get(&(a, b)).copied().unwrap_or(0))
                + i64::from(delta.get(&(a, b)).copied().unwrap_or(0))
        };
        let mut books: Vec<(u32, u32)> = delta
            .keys()
            .map(|&(a, b)| (a.min(b), a.max(b)))
            .filter(|&(a, b)| after(a, b) >= 2 || after(b, a) >= 2)
            .collect();
        if books.is_empty() {
            return true;
        }
        books.sort_unstable();
        books.dedup();
        let key_of = |v: u32| -> Key {
            match pending {
                Some((w, p)) if w == v => super::key(p),
                _ => match self.p.keys.get(v as usize) {
                    Some(&k) => k,
                    None => super::key(self.added[v as usize - self.p.positions.len()].0),
                },
            }
        };
        let rewritten: HashMap<u32, [u32; 3]> = changed.iter().copied().collect();
        let dead: HashSet<u32> = gone.iter().copied().collect();
        for (x, y) in books {
            // A vertex the change adds (`pending`, not yet in `iface`) is never shared.
            let shared = |v: u32| self.iface.get(v as usize).copied().unwrap_or(false);
            if shared(x) && shared(y) && self.shared_licence(x, y).is_some() {
                continue;
            }
            let (kx, ky) = (key_of(x), key_of(y));
            // The faces on x–y after the change: x's live faces and the rewritten ones (a
            // rewritten face may newly hold x), then the added faces.
            let mut seen: HashSet<u32> = HashSet::default();
            let mut wings: Vec<(usize, Key, bool)> = Vec::new();
            let mut wing = |t: [u32; 3]| {
                let Some(j) = (0..3).find(|&j| {
                    let (a, b) = (t[j], t[(j + 1) % 3]);
                    (a, b) == (x, y) || (a, b) == (y, x)
                }) else {
                    return;
                };
                let (from, third) = (t[j], t[(j + 2) % 3]);
                let d = usize::from(key_of(from) > key_of(if from == x { y } else { x }));
                let original = self
                    .src_face
                    .get(&sorted(t))
                    .is_some_and(|c| c[parity(t)] > 0);
                wings.push((d, key_of(third), original));
            };
            let faces: Vec<u32> = self
                .live_faces(x)
                .chain(changed.iter().map(|&(f, _)| f))
                .collect();
            self.work.set(
                self.work
                    .get()
                    .saturating_add(u64::try_from(faces.len()).unwrap_or(u64::MAX)),
            );
            for f in faces {
                if dead.contains(&f) || !seen.insert(f) {
                    continue;
                }
                wing(rewritten.get(&f).copied().unwrap_or(self.faces[f as usize]));
            }
            for &t in added {
                wing(t);
            }
            // The guard orders the edge by key; direction 0 runs from the lesser key.
            let edge = (kx.min(ky), kx.max(ky));
            let counts = [0, 1].map(|d| wings.iter().filter(|w| w.0 == d).count() as u32);
            let (lo, hi) = if kx < ky { (x, y) } else { (y, x) };
            let source = [(lo, hi), (hi, lo)].map(|e| self.src_dir.get(&e).copied().unwrap_or(0));
            if folds::book_refused(edge, counts, source, |d| {
                wings.iter().filter(move |w| w.0 == d).map(|w| (w.1, w.2))
            }) {
                return false;
            }
        }
        true
    }

    /// The guard's licence for directed edge `a -> b` between two shared points: its
    /// count in the contracted source (`Problem::shared`), or none when the source has the
    /// edge in neither direction and this cell owns the new segment (N1, stage 1).
    fn shared_licence(&self, a: u32, b: u32) -> Option<u32> {
        let count = |a: u32, b: u32| match &self.p.shared {
            Some(shared) => shared.get(&(a, b)).copied().unwrap_or(0),
            None => self
                .src_dir
                .get(&(a, b))
                .copied()
                .unwrap_or(0)
                .max(u32::from(self.p.segments.contains(&(a.min(b), a.max(b))))),
        };
        let (ab, ba) = (count(a, b), count(b, a));
        if ab == 0 && ba == 0 && self.p.owns_segment.is_some_and(|own| own(a, b)) {
            None
        } else {
            Some(ab)
        }
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

    /// Queues `u` at its cheapest target's cost; returns its candidate count.
    fn push(&mut self, heap: &mut BinaryHeap<Entry>, u: u32) -> usize {
        self.stamp[u as usize] += 1;
        let candidates = self.candidates(u);
        if let Some(&(cost, _)) = candidates.first() {
            heap.push(Entry {
                cost,
                u,
                stamp: self.stamp[u as usize],
            });
        }
        candidates.len()
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

    /// 0 when valid, else the refusal reason (`Stats::why` index). It costs u's star, not
    /// v's (a rim vertex tried against a fan apex of valence k no longer scans the apex).
    fn check(&mut self, u: u32, v: u32) -> u8 {
        self.stats.checks += 1;
        // u's star, collected once per pop: the targets a pop tries change nothing between.
        let star_u = match self.star_cache.take() {
            Some((w, at, star)) if w == u && at == self.changes => star,
            _ => {
                let star: Vec<u32> = self.live_faces(u).collect();
                self.work.set(
                    self.work
                        .get()
                        .saturating_add(u64::try_from(star.len()).unwrap_or(u64::MAX)),
                );
                star
            }
        };
        let why = self.check_star(u, v, &star_u);
        self.star_cache = Some((u, self.changes, star_u));
        why
    }

    /// The tests of one moved face `t` (u moving to v) that depend on nothing else the
    /// check accumulates: 4 degenerate, 5 turned over, 6 reversed against the source, 7 a new
    /// face on three shared points, 8 coincident with a live face, 9 a new unlicensed edge
    /// between shared points. 0 when it passes them.
    fn face_test(&self, t: [u32; 3], u: u32, v: u32) -> u8 {
        let iface = &self.iface;
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
        if self.face_set.contains_key(&s) {
            return 8; // coincident
        }
        // new edges incident to v
        for &w in &g {
            if w != v
                && iface[v as usize]
                && iface[w as usize]
                && self.shared_licence(v, w) == Some(0)
                && self.shared_licence(w, v) == Some(0)
            {
                return 9;
            }
        }
        0
    }

    /// `check` on u's live faces `star_u`.
    fn check_star(&mut self, u: u32, v: u32, star_u: &[u32]) -> u8 {
        // The faces u and v share, from the smaller of the two stars: u's is in hand, and
        // either may be a fan apex of thousands.
        let shared: Vec<u32> = if star_u.len() <= self.vf[v as usize].len() {
            self.work.set(
                self.work
                    .get()
                    .saturating_add(u64::try_from(star_u.len()).unwrap_or(u64::MAX)),
            );
            star_u
                .iter()
                .copied()
                .filter(|&f| self.faces[f as usize].contains(&v))
                .collect()
        } else {
            let mut seen = 0u64;
            let shared = self
                .live_faces(v)
                .filter(|&f| {
                    seen += 1;
                    self.faces[f as usize].contains(&u)
                })
                .collect();
            self.work.set(self.work.get().saturating_add(seen));
            shared
        };
        if shared.is_empty() {
            return 12;
        }
        let moved: Vec<u32> = if star_u.len() > RETRY_VALENCE {
            // A high-valence u (a fan apex) is refused against most targets by one of its
            // moved faces alone. Test a spread sample of them first, by the same face tests
            // as below: a refusal here is one the full check makes too, so the decision is
            // unchanged; only its cost falls.
            let step = (star_u.len() / PROBE).max(1);
            let mut probed = 0u64;
            for &f in star_u.iter().step_by(step).take(PROBE) {
                let t = self.faces[f as usize];
                if t.contains(&v) {
                    continue;
                }
                probed += 1;
                let why = self.face_test(t, u, v);
                if why != 0 {
                    self.work.set(self.work.get().saturating_add(probed));
                    return why;
                }
            }
            self.work.set(self.work.get().saturating_add(probed));
            star_u
                .iter()
                .copied()
                .filter(|&f| !self.faces[f as usize].contains(&v))
                .collect()
        } else {
            star_u
                .iter()
                .copied()
                .filter(|&f| !self.faces[f as usize].contains(&v))
                .collect()
        };
        if moved.is_empty() {
            // The component vanishes unless v keeps a face without u (stops at the first).
            let mut seen = 0u64;
            let keeps = self.live_faces(v).any(|f| {
                seen += 1;
                !self.faces[f as usize].contains(&u)
            });
            self.work.set(self.work.get().saturating_add(seen));
            if !keeps {
                return 1;
            }
        }
        // Link condition: a neighbour of u that is also v's (an edge to v in the live
        // directed counts, which is sharing a face) must be the apex of a shared face.
        let mut nu = Vec::new();
        self.neighbours(u, &mut nu);
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
            if w != v && self.undirected(w, v) > 0 && !expected.contains(&w) {
                return 2;
            }
        }
        // Boundary pinch: two boundary vertices joined by an interior edge. v's neighbours
        // are scanned only when the rest holds.
        if !self.is_boundary_edge(u, v) && self.on_boundary(u, &nu) {
            let mut nv = Vec::new();
            self.neighbours(v, &mut nv);
            if self.on_boundary(v, &nv) {
                return 3;
            }
        }
        // Local directed-edge accounting.
        let mut delta: HashMap<(u32, u32), i32> = HashMap::default();
        // The moved faces' sorted triples so far (coincidence among them in O(1) each).
        let mut new_faces: HashSet<[u32; 3]> = HashSet::default();
        for &f in &shared {
            let t = self.faces[f as usize];
            for j in 0..3 {
                *delta.entry((t[j], t[(j + 1) % 3])).or_default() -= 1;
            }
        }
        let iface = &self.iface;
        for &f in &moved {
            let t = self.faces[f as usize];
            let why = self.face_test(t, u, v);
            if why != 0 {
                return why;
            }
            let g = t.map(|w| if w == u { v } else { w });
            let s = sorted(g);
            if new_faces.contains(&s) {
                return 8; // coincident with another moved face
            }
            for j in 0..3 {
                *delta.entry((t[j], t[(j + 1) % 3])).or_default() -= 1;
                *delta.entry((g[j], g[(j + 1) % 3])).or_default() += 1;
            }
            new_faces.insert(s);
        }
        let mut deltas: Vec<((u32, u32), i32)> = delta.iter().map(|(&e, &d)| (e, d)).collect();
        deltas.sort_unstable();
        for ((a, b), d) in deltas {
            if d > 0 {
                let now = i64::from(self.dir.get(&(a, b)).copied().unwrap_or(0)) + i64::from(d);
                let src = self.src_dir.get(&(a, b)).copied().unwrap_or(0);
                // Between two shared points the directed count may not exceed the
                // guard's contracted source count (its boundary rule).
                let shared = if iface[a as usize] && iface[b as usize] {
                    self.shared_licence(a, b)
                } else {
                    None
                };
                let lic = i64::from(shared.unwrap_or(src.max(1)));
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
        // A boundary reaching a contract point keeps its shape (the topology floor's
        // touching-boundary rule, `topology::changes_noting`).
        if self.touching_changed(u, v, &delta) {
            return 13;
        }
        // The guard's book rule on every edge the collapse touches: a moved wing of a
        // source book keeps the edge's counts but not its faces. Only an edge used twice or
        // more in one direction, before or after, can be refused: one already a book
        // (`books`), or one this collapse makes so.
        let touches_book =
            |&(a, b): &(u32, u32)| self.books.contains(&(a, b)) || self.books.contains(&(b, a));
        let makes_book = |(e, &d): (&(u32, u32), &i32)| {
            d > 0 && i64::from(self.dir.get(e).copied().unwrap_or(0)) + i64::from(d) >= 2
        };
        if (!self.books.is_empty() && delta.keys().any(touches_book))
            || delta.iter().any(makes_book)
        {
            let rewritten: Vec<(u32, [u32; 3])> = moved
                .iter()
                .map(|&f| {
                    (
                        f,
                        self.faces[f as usize].map(|w| if w == u { v } else { w }),
                    )
                })
                .collect();
            if !self.books_hold(&shared, &rewritten, &[], None) {
                return 10;
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
        // The flip makes p–q in both directions: between two shared points each needs the
        // guard's licence.
        if iface[p as usize]
            && iface[q as usize]
            && (self.shared_licence(p, q).is_some_and(|n| n < 1)
                || self.shared_licence(q, p).is_some_and(|n| n < 1))
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
        if !self.books_hold(&[], &[(f1, g1), (f2, g2)], &[], None) {
            return false;
        }
        // apply
        self.changes += 1;
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
                self.inc_dir((g[j], g[(j + 1) % 3]));
            }
            *self.face_set.entry(sorted(g)).or_default() += 1;
        }
        // A vertex can re-enter a face it held before (two flips through the same slots):
        // its stale entry would make `live_faces` yield the face twice.
        if !self.vf[p as usize].contains(&f2) {
            self.vf[p as usize].push(f2);
        }
        if !self.vf[q as usize].contains(&f1) {
            self.vf[q as usize].push(f1);
        }
        self.stats.flips += 1;
        true
    }

    /// The ends `(x, y)` of `r`'s link when it is exactly one simple oriented path from `y`
    /// to `x` over at least two faces: each face about `r`, rotated to `(r, p, q)`, gives
    /// the link edge `p -> q`; every vertex starts and ends at most one edge, one vertex
    /// starts none, and the walk from it consumes every face. So `x -> r` and `r -> y` are
    /// the only edges at `r` with one face, and no cycle (a pinch) is left over.
    fn simple_link(&self, r: u32, star: &[u32]) -> Option<(u32, u32)> {
        if star.len() < 2 {
            return None;
        }
        let mut edges: Vec<(u32, u32)> = Vec::with_capacity(star.len());
        for &f in star {
            let t = self.faces[f as usize];
            let k = t.iter().position(|&w| w == r)?;
            let (p, q) = (t[(k + 1) % 3], t[(k + 2) % 3]);
            if p == r || q == r || p == q {
                return None;
            }
            edges.push((p, q));
        }
        edges.sort_unstable();
        let mut ends: Vec<u32> = edges.iter().map(|e| e.1).collect();
        ends.sort_unstable();
        if edges.windows(2).any(|w| w[0].0 == w[1].0) || ends.windows(2).any(|w| w[0] == w[1]) {
            return None;
        }
        let mut starts = edges
            .iter()
            .map(|e| e.0)
            .filter(|p| ends.binary_search(p).is_err());
        let y = starts.next()?;
        if starts.next().is_some() {
            return None;
        }
        let (mut at, mut steps) = (y, 0usize);
        while let Ok(i) = edges.binary_search_by(|e| e.0.cmp(&at)) {
            at = edges[i].1;
            steps += 1;
            if steps > edges.len() {
                return None;
            }
        }
        (steps == edges.len()).then_some((at, y))
    }

    /// The canonical keys of every input vertex and every Steiner vertex so far (built on
    /// the first rescue): a Steiner vertex takes none of them.
    fn taken(&mut self) -> &mut HashSet<Key> {
        let (positions, added) = (self.p.positions, &self.added);
        let work = &self.work;
        self.taken.get_or_insert_with(|| {
            work.set(
                work.get()
                    .saturating_add(u64::try_from(positions.len()).unwrap_or(u64::MAX)),
            );
            positions
                .iter()
                .chain(added.iter().map(|a| &a.0))
                .map(|&p| private_key(p))
                .collect()
        })
    }

    /// Rescue of a removal vertex `r` stuck between shared points: replace it by a new
    /// interior (Steiner) vertex `s` and close its arc with the bridge face `(s, x, y)`, so
    /// no face spans three shared points. Admitted only when (`Stats::steiner_why`):
    /// - `r`'s link is one simple path from `y` to `x` (`simple_link`), `x` and `y` lie on
    ///   `r`'s own contraction (its chord's endpoints or removals of the same chord), there
    ///   is no `x`–`y` edge yet and the edge has a licence;
    /// - `s` is a canonical lattice point (the snap succeeds and is idempotent) whose key no
    ///   input vertex, Steiner vertex or protected border key elsewhere holds;
    /// - every moved face keeps a certified positive orientation against its old normal,
    ///   and the bridge against the sum of the old normals (`embedding::positive`);
    /// - each changed face meets every other live face only in their common vertices
    ///   (`embedding::permitted`, exact).
    ///
    /// Positions are tried in a fixed order (from the chord midpoint toward the fan's
    /// interior, then from `r`); one that fails a test is skipped for the next. The mesh
    /// changes only once a position passes every test.
    fn steiner(&mut self, r: u32) -> bool {
        let Class::Remove(chord) = self.class[r as usize] else {
            return false;
        };
        if self.dead[r as usize] {
            return false;
        }
        let star: Vec<u32> = self.live_faces(r).collect();
        let Some((x, y)) = self.simple_link(r, &star) else {
            self.stats.steiner_why[0] += 1;
            return false;
        };
        let on_chord = |v: u32| {
            v == chord[0] || v == chord[1] || self.class[v as usize] == Class::Remove(chord)
        };
        if !on_chord(x) || !on_chord(y) {
            self.stats.steiner_why[1] += 1;
            return false;
        }
        let lic = if self.iface[x as usize] && self.iface[y as usize] {
            self.shared_licence(x, y)
        } else {
            None
        }
        .unwrap_or(self.src_dir.get(&(x, y)).copied().unwrap_or(0).max(1));
        if self.undirected(x, y) > 0 || self.dir.get(&(x, y)).copied().unwrap_or(0) + 1 > lic {
            self.stats.steiner_why[2] += 1;
            return false;
        }
        let mut nb = Vec::new();
        self.neighbours(r, &mut nb);
        let interior: Vec<u32> = nb.iter().copied().filter(|&w| w != x && w != y).collect();
        let pr = self.pos[r as usize];
        let mut c = [0f64; 3];
        for &w in &interior {
            for i in 0..3 {
                c[i] += self.pos[w as usize][i] / interior.len() as f64;
            }
        }
        // Old faces rotated to (r, a, b) with their normals; the bridge's reference is
        // their sum, accumulated with one rounding per further face.
        let old: Vec<(u32, [u32; 2], (Point, Point))> = star
            .iter()
            .map(|&f| {
                let t = self.faces[f as usize];
                let k = t.iter().position(|&w| w == r).unwrap_or(0);
                let n = embedding::normal(t.map(|v| self.pos[v as usize]));
                (f, [t[(k + 1) % 3], t[(k + 2) % 3]], n)
            })
            .collect();
        let (mut fan, mut fan_abs) = ([0f64; 3], [0f64; 3]);
        for (_, _, (n, a)) in &old {
            for i in 0..3 {
                fan[i] += n[i];
                fan_abs[i] += a[i];
            }
        }
        let fan_depth = 4 + u32::try_from(old.len()).unwrap_or(u32::MAX);
        let (px, py) = (self.pos[x as usize], self.pos[y as usize]);
        let mid = [
            (px[0] + py[0]) / 2.,
            (px[1] + py[1]) / 2.,
            (px[2] + py[2]) / 2.,
        ];
        let chord_length = dot(sub(px, py), sub(px, py)).sqrt();
        // Distinct canonical proposals in order: absolute offsets on the chord's scale
        // first, then fractions of the reach, from the midpoint and then from r.
        let mut proposals: Vec<[f32; 3]> = Vec::with_capacity(16);
        for base in [mid, pr] {
            let toward = sub(c, base);
            let reach = dot(toward, toward).sqrt();
            if reach <= 0. {
                continue;
            }
            let steps = [0.05, 0.1, 0.2, 0.4]
                .map(|k| (k * chord_length / reach).min(0.45))
                .into_iter()
                .chain([0.1, 0.2, 0.3, 0.45]);
            for frac in steps {
                let raw: [f32; 3] = std::array::from_fn(|i| (base[i] + frac * toward[i]) as f32);
                let ps = match self.p.snap {
                    Some(snap) => match snap(raw, r) {
                        Some(ps) if snap(ps, r).map(private_key) == Some(private_key(ps)) => ps,
                        // no canonical point, or not a fixed point of its own lattice
                        _ => continue,
                    },
                    None => raw,
                };
                if ps.iter().all(|v| v.is_finite())
                    && !proposals.iter().any(|&p| private_key(p) == private_key(ps))
                {
                    proposals.push(ps);
                }
            }
        }
        let sid = id(self.pos.len());
        let bridge = [sid, x, y];
        let in_star: HashSet<u32> = star.iter().copied().collect();
        let mut nearby: Option<Vec<u32>> = None;
        let mut stage = 3;
        let mut chosen = None;
        for ps in proposals.iter().copied() {
            self.work.set(
                self.work
                    .get()
                    .saturating_add(u64::try_from(star.len()).unwrap_or(u64::MAX)),
            );
            if self.exhausted() {
                return false;
            }
            if self.taken().contains(&private_key(ps)) || self.p.protected.is_some_and(|f| f(ps)) {
                continue;
            }
            let q = ps.map(f64::from);
            if !embedding::positive(fan, fan_abs, fan_depth, px, py, q)
                || !old.iter().all(|&(_, [a, b], (n, n_abs))| {
                    embedding::positive(n, n_abs, 4, self.pos[a as usize], self.pos[b as usize], q)
                })
            {
                stage = stage.max(4);
                continue;
            }
            stage = 5;
            let at = |v: u32| if v == sid { q } else { self.pos[v as usize] };
            let changed: Vec<[u32; 3]> = old
                .iter()
                .map(|&(_, [a, b], _)| [sid, a, b])
                .chain([bridge])
                .collect();
            let boxes: Vec<[Point; 2]> = changed
                .iter()
                .map(|t| embedding::bounds(t.map(at)))
                .collect();
            let near = nearby.get_or_insert_with(|| {
                // Live faces outside the star within the bounds of the star and every
                // proposal: no changed face of any proposal reaches beyond them.
                let mut all = embedding::bounds([pr, px, py]);
                let mut include = |p: Point| {
                    for k in 0..3 {
                        all[0][k] = all[0][k].min(p[k]);
                        all[1][k] = all[1][k].max(p[k]);
                    }
                };
                for (_, [a, b], _) in &old {
                    include(self.pos[*a as usize]);
                    include(self.pos[*b as usize]);
                }
                for p in &proposals {
                    include(p.map(f64::from));
                }
                self.work.set(
                    self.work
                        .get()
                        .saturating_add(u64::try_from(self.faces.len()).unwrap_or(u64::MAX)),
                );
                (0..id(self.faces.len()))
                    .filter(|&f| {
                        self.alive[f as usize]
                            && !in_star.contains(&f)
                            && embedding::overlap(
                                all,
                                embedding::bounds(
                                    self.faces[f as usize].map(|v| self.pos[v as usize]),
                                ),
                            )
                    })
                    .collect()
            });
            // Each changed face is tested against the others and the nearby faces.
            let tests = changed
                .len()
                .saturating_mul(changed.len().saturating_add(near.len()));
            self.work.set(
                self.work
                    .get()
                    .saturating_add(u64::try_from(tests).unwrap_or(u64::MAX)),
            );
            let apart = changed.iter().enumerate().all(|(i, &t)| {
                changed[i + 1..]
                    .iter()
                    .zip(&boxes[i + 1..])
                    .all(|(&u, &b)| {
                        !embedding::overlap(boxes[i], b) || embedding::permitted(t, u, at)
                    })
                    && near.iter().all(|&f| {
                        let u = self.faces[f as usize];
                        !embedding::overlap(boxes[i], embedding::bounds(u.map(at)))
                            || embedding::permitted(t, u, at)
                    })
            });
            if apart {
                let rewritten: Vec<(u32, [u32; 3])> = star
                    .iter()
                    .map(|&f| {
                        (
                            f,
                            self.faces[f as usize].map(|w| if w == r { sid } else { w }),
                        )
                    })
                    .collect();
                if self.books_hold(&[], &rewritten, &[bridge], Some((sid, ps))) {
                    chosen = Some((ps, q));
                    break;
                }
                stage = 6;
            }
        }
        let Some((ps32, psd)) = chosen else {
            self.stats.steiner_why[stage] += 1;
            return false;
        };
        // apply: new vertex
        self.changes += 1;
        self.taken().insert(private_key(ps32));
        self.pos.push(psd);
        self.added.push((ps32, r));
        self.class.push(Class::Free);
        self.retry_after.push(0);
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
                self.inc_dir((g[j], g[(j + 1) % 3]));
            }
            *self.face_set.entry(sorted(g)).or_default() += 1;
            self.vf[sid as usize].push(f);
        }
        let fid = id(self.faces.len());
        self.faces.push(bridge);
        self.alive.push(true);
        self.alive_faces += 1;
        for j in 0..3 {
            self.inc_dir((bridge[j], bridge[(j + 1) % 3]));
        }
        *self.face_set.entry(sorted(bridge)).or_default() += 1;
        for v in bridge {
            self.vf[v as usize].push(fid);
        }
        self.dead[r as usize] = true;
        self.removes_left -= 1;
        self.stats.steiner += 1;
        true
    }

    fn apply(&mut self, u: u32, v: u32) {
        self.stats.collapses += 1;
        self.changes += 1;
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
                    self.inc_dir((g[j], g[(j + 1) % 3]));
                }
                *self.face_set.entry(sorted(g)).or_default() += 1;
                self.vf[v as usize].push(f);
            }
        }
        self.dead[u as usize] = true;
        if self.touching.get(u as usize).copied().unwrap_or(false) {
            if let Some(t) = self.touching.get_mut(v as usize) {
                *t = true;
            }
        }
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

impl<'a> State<'a> {
    /// The pass's state over the problem's input: adjacency, directed edge and face counts,
    /// the source's counts and the quadrics.
    fn new(p: &'a Problem<'a>) -> Self {
        let n = p.positions.len();
        let pos: Vec<[f64; 3]> = p.positions.iter().map(|q| q.map(f64::from)).collect();
        let mut st = Self {
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
            mark: RefCell::new(vec![0; n]),
            epoch: Cell::new(0),
            work: Cell::new(0),
            taken: None,
            retry_after: vec![0; n],
            cancelled: Cell::new(false),
            next_poll: Cell::new(0),
            changes: 0,
            star_cache: None,
            books: HashSet::default(),
            touching: Vec::new(),
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
        st.touching = st.touching_boundaries();
        st.books = st
            .dir
            .iter()
            .filter(|&(_, &c)| c >= 2)
            .map(|(&e, _)| e)
            .collect();
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
        st
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
    let (out, stats, added, _) = simplify_left(p);
    (out, stats, added)
}

/// `simplify_added`, also returning the removal vertices the pass left (`Class::Remove`,
/// still live), in id order: the border vertices D may keep instead.
pub(crate) fn simplify_left(p: &Problem<'_>) -> (Option<Vec<[u32; 3]>>, Stats, Added, Vec<u32>) {
    let n = p.positions.len();
    let mut st = State::new(p);
    let mut nb = Vec::new();
    for _outer in 0..4 {
        if st.exhausted() {
            break;
        }
        let before = (
            st.stats.collapses,
            st.stats.steiner,
            st.stats.flips,
            st.stats.rescues,
        );
        let mut heap = BinaryHeap::new();
        let w0 = st.work.get();
        for u in 0..id(st.pos.len()) {
            if !st.dead[u as usize] && !st.vf[u as usize].is_empty() {
                st.push(&mut heap, u);
            }
        }
        st.stats.site.queue += st.work.get() - w0;
        while let Some(e) = heap.pop() {
            if (st.alive_faces <= p.target && st.removes_left == 0) || st.exhausted() {
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
            st.stats.site.pops += 1;
            let w0 = st.work.get();
            let candidates = st.candidates(u);
            let spent = st.work.get() - w0;
            st.stats.site.candidates += spent;
            st.stats.site.max_candidates = st.stats.site.max_candidates.max(spent);
            let tried = candidates.len();
            // A high-valence vertex tries its cheapest `RETRY_VALENCE` targets: each check
            // moves all its faces, so trying every target would cost its valence squared.
            for (_, v) in candidates.into_iter().take(RETRY_VALENCE) {
                let w0 = st.work.get();
                let valid = st.valid(u, v);
                let spent = st.work.get() - w0;
                let site = &mut st.stats.site;
                site.checks_run += 1;
                site.checks += spent;
                site.max_check = site.max_check.max(spent);
                if !valid {
                    site.refused_checks += spent;
                }
                if valid {
                    let w0 = st.work.get();
                    st.apply(u, v);
                    st.neighbours(v, &mut nb);
                    let list = nb.clone();
                    let w1 = st.work.get();
                    let n = st.push(&mut heap, v);
                    st.retry_after[v as usize] = retry_wait(n);
                    let w2 = st.work.get();
                    st.stats.site.apply += w1 - w0;
                    st.stats.site.requeue += w2 - w1;
                    for w in list {
                        // A high-valence neighbour is queued again only after a share of its
                        // neighbourhood has changed, not after every change: each requeue
                        // scans all its targets (`retry_wait`). Its queued entry stays valid
                        // meanwhile, and a pop recomputes its targets.
                        let wait = &mut st.retry_after[w as usize];
                        if *wait > 0 {
                            *wait -= 1;
                            continue;
                        }
                        let n = st.push(&mut heap, w);
                        st.retry_after[w as usize] = retry_wait(n);
                    }
                    st.stats.site.neighbours += st.work.get() - w2;
                    done = true;
                    break;
                }
                st.stats.refused += 1;
            }
            if !done {
                // stuck until a neighbour changes (it is pushed again then)
                st.stamp[u as usize] += 1;
                st.retry_after[u as usize] = retry_wait(tried);
            }
        }
        // Rescue: a border vertex left on its arc. Simplify the free vertices around it
        // (cheapest valid collapse first), then retry it; a bounded number of rounds.
        let mut rounds = 0;
        let rescue_start = st.work.get();
        while st.removes_left > 0 && rounds < 64 && !st.exhausted() {
            rounds += 1;
            let left: Vec<u32> = (0..id(n))
                .filter(|&v| {
                    !st.dead[v as usize] && matches!(st.class[v as usize], Class::Remove(_))
                })
                .collect();
            let mut progress = false;
            for r in left {
                if st.exhausted() {
                    break;
                }
                if st.dead[r as usize] {
                    continue;
                }
                if let Some(v) = st
                    .candidates(r)
                    .into_iter()
                    .take(RETRY_VALENCE)
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
                let steiner_start = st.work.get();
                let rescued = p.steiner && st.steiner(r);
                st.stats.steiner_work += st.work.get().saturating_sub(steiner_start);
                if rescued {
                    progress = true;
                    continue;
                }
                // Free vertices in r's 2-ring, in first-seen order. Not through a
                // high-valence neighbour: the apex of a large fan (a triangulated base cap,
                // say) would bring its whole rim, which is not local to r, into every
                // round's candidate search for every stuck removal.
                let mut seen: HashSet<u32> = ring.iter().copied().collect();
                let mut ring2 = ring.clone();
                for &w in &ring {
                    st.neighbours(w, &mut nb);
                    if nb.len() > RETRY_VALENCE {
                        continue;
                    }
                    for &x in &nb {
                        if x != r && seen.insert(x) {
                            ring2.push(x);
                        }
                    }
                }
                let mut best: Option<(f64, u32, u32)> = None;
                for &w in &ring2 {
                    if st.exhausted() {
                        break;
                    }
                    if st.dead[w as usize] || st.class[w as usize] != Class::Free {
                        continue;
                    }
                    for (c, v) in st.candidates(w).into_iter().take(RETRY_VALENCE) {
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
        st.stats.rescue_work += st.work.get().saturating_sub(rescue_start);
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
    st.stats.work = st.work.get();
    st.stats.exhausted = st.exhausted();
    st.stats.cancelled = st.cancelled.get();
    let ok = !st.stats.exhausted && st.removes_left == 0 && out.len() <= p.target.max(p.limit);
    let left: Vec<u32> = (0..id(n))
        .filter(|&v| !st.dead[v as usize] && matches!(st.class[v as usize], Class::Remove(_)))
        .collect();
    (ok.then_some(out), st.stats, st.added, left)
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
) -> Result<(Problem<'a>, Mapped, Vec<InterfaceRow>), JobError> {
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
            let mm = mapping(*k)?;
            mapped[v] = mm;
            if let Some((a, b, _, _)) = mm {
                // As the contraction (`topology::contract_from`): a chord endpoint missing
                // from the merged children is an error, not a free vertex.
                let (Some(&a), Some(&b)) = (ids.get(&a), ids.get(&b)) else {
                    return Err(super::state(
                        "a nested border chord endpoint is absent from its merged children",
                    ));
                };
                if a != id(v) && b != id(v) {
                    class[v] = Class::Remove([a, b]);
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
    Ok((
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
            protected: None,
            shared: None,
            work_limit: u64::MAX,
            cancel: None,
            min_cos,
            owns_segment: None,
            owns_face: None,
        },
        mapped,
        previous,
    ))
}

/// A child border key's image at the parent depth (`borders::Borders::mapping`).
pub(crate) type Mapping<'a> = dyn Fn(Key) -> Result<Option<(Key, Key, f64, u8)>, JobError> + 'a;

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
            protected: None,
            shared: None,
            work_limit: u64::MAX,
            cancel: None,
        }
    }

    /// Positions, triangles, classes and shared points of a collapse fixture.
    type Fixture = (Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<Class>, Vec<bool>);

    /// A source book: two faces use the interior edge p→q in the same direction (no face
    /// uses q→p), and the third corner of one is r, a border vertex to be removed onto the
    /// chord a–b. Collapsing r onto a or b keeps p→q's count at its source count but moves
    /// that wing, which the contraction fold guard refuses (its book rule admits two
    /// same-direction faces only unchanged, or with a reverse face and on distinct sides).
    /// p, q and w are locked, as `problem` locks every free corner of a fold face; x is the
    /// strip's free apex below the chord. Vertices: a, r, b, p, q, w, x.
    fn moved_book_wing() -> Fixture {
        (
            vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [2., 0., 0.],
                [1., 1., 0.],
                [1., 1., 1.],
                [1., 2., 0.],
                [1., -1., 0.],
            ],
            vec![[0, 6, 1], [1, 6, 2], [3, 4, 1], [3, 4, 5]],
            vec![
                Class::Locked,
                Class::Remove([0, 2]),
                Class::Locked,
                Class::Locked,
                Class::Locked,
                Class::Locked,
                Class::Free,
            ],
            vec![true, false, true, false, false, false, false],
        )
    }

    /// The contraction fold guard's refusals of `faces` (`folds::Refusal`) for the fixture
    /// above: the parent contract is a, b and the chord a–b, the children's border the path
    /// a, r, b, and r maps to the chord's middle (the guard licenses a–b from that path).
    fn book_guard_refusals(
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        faces: &[[u32; 3]],
    ) -> Vec<u8> {
        let k: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let row = |a: Key, b: Key| InterfaceRow::new(0, a.min(b), a.max(b), 0, 1);
        let interface = vec![row(k[0], k[0]), row(k[2], k[2]), row(k[0], k[2])];
        let previous = vec![
            row(k[0], k[0]),
            row(k[1], k[1]),
            row(k[2], k[2]),
            row(k[0], k[1]),
            row(k[1], k[2]),
        ];
        let mut mapping = vec![None; positions.len()];
        mapping[0] = Some((k[0], k[0], 0., 0));
        mapping[2] = Some((k[2], k[2], 0., 0));
        mapping[1] = Some((k[0], k[2], 0.5, 0));
        super::super::folds::Source::for_contraction(
            positions,
            triangles,
            &interface,
            &previous,
            [0, 1],
            &mapping,
        )
        .refusals(positions, faces)
    }

    #[test]
    fn a_book_wing_does_not_move() {
        let (positions, triangles, class, shared) = moved_book_wing();
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        // The source itself passes the guard.
        assert!(book_guard_refusals(&positions, &triangles, &triangles)
            .iter()
            .all(|&r| r == 0));
        // Collapsing r onto a keeps p→q at two faces and every other count licensed: the
        // guard refuses exactly the moved wing (p, q, a) and its unchanged partner (both
        // faces of the edge), by the book rule.
        let collapsed = [[0, 6, 2], [3, 4, 0], [3, 4, 5]];
        assert_eq!(
            book_guard_refusals(&positions, &triangles, &collapsed),
            vec![0, folds::Refusal::Book as u8, folds::Refusal::Book as u8]
        );
        let p = Problem {
            positions: &positions,
            triangles: &triangles,
            class,
            interface: shared.clone(),
            segments: [(0, 2)].into_iter().collect(),
            target: 1,
            limit: 4,
            min_cos: 0.,
            owns_segment: None,
            owns_face: None,
            keys: &keys,
            steiner: false,
            snap: None,
            protected: None,
            shared: None,
            work_limit: u64::MAX,
            cancel: None,
        };
        let (out, stats) = simplify(&p);
        if let Some(out) = &out {
            let refusals = book_guard_refusals(&positions, &triangles, out);
            assert!(
                refusals.iter().all(|&r| r == 0),
                "P1 proposed {out:?}, which the guard refuses: {refusals:?} ({stats:?})"
            );
        }
        // Removing r needs the wing to move: r stays.
        assert!(out.is_none(), "{out:?}");
        assert_eq!(stats.removed_left, 1, "{stats:?}");
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

    /// The removal `r` of chord `ring[0]`–`ring[last]` with shared `ring` vertices and free
    /// others, at a face cap `cap`, snapped to the unit grid (the review's synthetic
    /// controls, `steiner/probe/fixtures.py`).
    fn rescue(
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        ring: &[u32],
        r: u32,
        cap: usize,
        snap: Option<&Snap<'_>>,
    ) -> (Option<Vec<[u32; 3]>>, Stats, Added) {
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let (x, y) = (ring[0], ring[ring.len() - 1]);
        let class = (0..id(positions.len()))
            .map(|v| {
                if v == r {
                    Class::Remove([x, y])
                } else if ring.contains(&v) {
                    Class::Locked
                } else {
                    Class::Free
                }
            })
            .collect();
        let p = Problem {
            positions,
            triangles,
            class,
            interface: (0..id(positions.len()))
                .map(|v| ring.contains(&v))
                .collect(),
            segments: [(x.min(y), x.max(y))].into_iter().collect(),
            target: cap,
            limit: cap,
            min_cos: 0.,
            owns_segment: None,
            owns_face: None,
            keys: &keys,
            steiner: true,
            snap,
            protected: None,
            shared: None,
            work_limit: u64::MAX,
            cancel: None,
        };
        simplify_added(&p)
    }

    fn unit(p: [f32; 3], _: u32) -> Option<[f32; 3]> {
        Some(p.map(f32::round))
    }

    /// x, r, y and the fan's far apex w (`crossing` without its spectator is `clear`).
    fn crossing_fan() -> Vec<[f32; 3]> {
        vec![[0., 0., 0.], [32., 16., 0.], [64., 0., 0.], [32., 64., 0.]]
    }

    #[test]
    fn steiner_closes_a_clear_fan_on_the_lattice() {
        let positions = crossing_fan();
        let (out, stats, added) = rescue(
            &positions,
            &[[0, 1, 3], [3, 1, 2]],
            &[0, 3, 2],
            1,
            3,
            Some(&unit),
        );
        assert!(out.is_some(), "{stats:?}");
        assert_eq!(added, vec![([32., 3., 0.], 1)]);
    }

    /// A flip at r–x would rewrite the face (x, r, q) into (r, q, p), a new face on r→q,
    /// which the source face (r, q, w) also uses that way: a moved book wing, refused as the
    /// fold guard refuses it. Without the book face the same flip is made.
    #[test]
    fn a_flip_does_not_move_a_book_wing() {
        let positions = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [0.5, 1., 0.],
            [0.5, -1., 0.],
            [0.25, -0.5, 1.],
        ];
        let (r, x) = (0, 1);
        for book in [false, true] {
            let mut triangles = vec![[0, 1, 2], [1, 0, 3]];
            if book {
                triangles.push([0, 3, 4]);
            }
            let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
            let n = positions.len();
            let p = Problem {
                positions: &positions,
                triangles: &triangles,
                class: vec![Class::Free; n],
                interface: vec![false; n],
                segments: HashSet::default(),
                target: 1,
                limit: 4,
                min_cos: 0.,
                owns_segment: None,
                owns_face: None,
                keys: &keys,
                steiner: false,
                snap: None,
                protected: None,
                shared: None,
                work_limit: u64::MAX,
                cancel: None,
            };
            let mut st = State::new(&p);
            assert_eq!(st.flip(r, x), !book, "book {book}");
        }
    }

    /// Removing r (onto the chord x–y, shared points) needs (x, r, w) to change, and its
    /// edge w→x is a book: the source face (w, x, z) uses it the same way, and w is not
    /// shared, so the guard's book rule applies. Collapsing r onto x or y and every Steiner
    /// position move that wing: all refused (the rescue as `steiner_why[6]`), and r stays.
    /// Without the book face, r collapses.
    #[test]
    fn a_steiner_position_does_not_move_a_book_wing() {
        let mut positions = crossing_fan();
        positions.push([16., 32., 20.]);
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let faces = [[0, 1, 3], [3, 1, 2], [3, 0, 4]];
        for book in [true, false] {
            let triangles = &faces[..if book { 3 } else { 2 }];
            let p = Problem {
                positions: &positions,
                triangles,
                // z, the book face's free corner, is locked as `problem` locks every
                // free corner of a fold face.
                class: vec![
                    Class::Locked,
                    Class::Remove([0, 2]),
                    Class::Locked,
                    Class::Locked,
                    Class::Locked,
                ],
                interface: vec![true, false, true, false, false],
                segments: [(0, 2)].into_iter().collect(),
                target: 4,
                limit: 4,
                min_cos: 0.,
                owns_segment: None,
                owns_face: None,
                keys: &keys,
                steiner: true,
                snap: Some(&unit),
                protected: None,
                shared: None,
                work_limit: u64::MAX,
                cancel: None,
            };
            let (out, stats, added) = simplify_added(&p);
            if book {
                assert!(out.is_none(), "{stats:?}");
                assert!(added.is_empty(), "{added:?}");
                assert_eq!(stats.removed_left, 1, "{stats:?}");
                assert!(stats.steiner_why[6] > 0, "{stats:?}");
            } else {
                assert!(out.is_some(), "{stats:?}");
            }
        }
    }

    /// A change adding two same-direction faces on an edge from a shared point to the
    /// vertex it adds (`pending`, not yet in the state's tables) is judged, not indexed out
    /// of bounds: the vertex it adds is never shared, and the two new faces are a book.
    #[test]
    fn a_pending_vertex_is_not_shared() {
        let positions = vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 1.]];
        let triangles = [[0, 1, 2], [0, 2, 3]];
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let p = Problem {
            positions: &positions,
            triangles: &triangles,
            class: vec![Class::Free; 4],
            interface: vec![true, false, false, false],
            segments: HashSet::default(),
            target: 1,
            limit: 4,
            min_cos: 0.,
            owns_segment: None,
            owns_face: None,
            keys: &keys,
            steiner: false,
            snap: None,
            protected: None,
            shared: None,
            work_limit: u64::MAX,
            cancel: None,
        };
        let st = State::new(&p);
        let s = 4;
        let added = [[0, s, 1], [0, s, 2]];
        assert!(!st.books_hold(&[], &[], &added, Some((s, [0.5, 0.5, -1.]))));
    }

    /// The review's three-face counterexample: the bridge (s, x, y) of every position on
    /// the fan's axis would pierce a spectator triangle that the old fan never touched. The
    /// rescue is refused (contact), so the removal stays and P1 builds nothing.
    #[test]
    fn steiner_never_pierces_a_disjoint_triangle() {
        let mut positions = crossing_fan();
        positions.extend([[32., 1., -1.], [32., 1., 1.], [32., 2., 0.]]);
        let faces = [[0, 1, 3], [3, 1, 2], [4, 5, 6]];
        let (out, stats, added) = rescue(&positions, &faces, &[0, 3, 2], 1, 4, Some(&unit));
        assert!(out.is_none() && added.is_empty(), "{stats:?} {added:?}");
        assert_eq!(stats.removed_left, 1);
        assert!(stats.steiner_why[5] > 0, "{stats:?}");
    }

    /// A position whose key an unrelated vertex holds, then positions touching that
    /// triangle's corner: refused.
    #[test]
    fn steiner_takes_no_key_another_vertex_holds() {
        let mut positions = crossing_fan();
        positions.extend([[32., 3., 0.], [32., 3., 1.], [33., 3., 1.]]);
        let faces = [[0, 1, 3], [3, 1, 2], [4, 5, 6]];
        let (out, stats, added) = rescue(&positions, &faces, &[0, 3, 2], 1, 4, Some(&unit));
        assert!(out.is_none() && added.is_empty(), "{stats:?} {added:?}");
    }

    /// r's link is a path and a disjoint cycle (a pinch): capping it would leave s with two
    /// link cycles. Refused before any position is tried.
    #[test]
    fn steiner_needs_one_simple_link_path() {
        let mut positions = crossing_fan();
        positions.extend([[-200., -100., 10.], [0., 300., 10.], [200., -100., 10.]]);
        let faces = [[0, 1, 3], [3, 1, 2], [4, 1, 5], [5, 1, 6], [6, 1, 4]];
        let (out, stats, added) = rescue(&positions, &faces, &[0, 3, 2], 1, 6, Some(&unit));
        assert!(out.is_none() && added.is_empty(), "{stats:?}");
        assert!(stats.steiner_why[0] > 0, "{stats:?}");
    }

    /// A moved face fails at the first distinct midpoint positions; the next position on
    /// the same ray, (48, 1, 0), closes a valid five-face disk (the review's `skip`).
    #[test]
    fn steiner_tries_the_next_position_after_a_failed_face() {
        let positions = vec![
            [0., 0., 0.],
            [0., 7., 0.],
            [9., 6., 0.],
            [93., 5., 0.],
            [100., 0., 0.],
            [23., -47., 0.],
        ];
        let faces: Vec<[u32; 3]> = (0..4).map(|i| [i, 5, i + 1]).collect();
        let (out, stats, added) = rescue(&positions, &faces, &[0, 1, 2, 3, 4], 5, 5, Some(&unit));
        let faces = out.unwrap_or_else(|| panic!("{stats:?}"));
        assert_eq!(added, vec![([48., 1., 0.], 5)]);
        assert_eq!(faces.len(), 5);
        for f in &faces {
            assert!(up(&positions, &added, *f) > 0., "{faces:?}");
        }
    }

    /// No canonical point (a refused snap) is no position: the raw proposal is never used.
    #[test]
    fn steiner_refuses_a_failed_snap() {
        let (positions, triangles) = fan_of_shared_points();
        let none = |_: [f32; 3], _: u32| None;
        let (out, stats, added) = rescue(&positions, &triangles, &[0, 3, 2], 1, 4, Some(&none));
        assert!(out.is_none() && added.is_empty(), "{stats:?}");
        assert!(stats.steiner_why[3] > 0, "{stats:?}");
        // A snap that is not idempotent is refused too.
        let drift = |p: [f32; 3], _: u32| Some([p[0], p[1] + 0.25, p[2]]);
        let (out, _, added) = rescue(&positions, &triangles, &[0, 3, 2], 1, 4, Some(&drift));
        assert!(out.is_none() && added.is_empty());
    }

    /// An arc end that is neither an endpoint of r's chord nor a removal of the same chord
    /// would make a border edge the contract does not have: refused.
    #[test]
    fn steiner_shortcuts_only_along_its_own_chord() {
        let (positions, triangles) = fan_of_shared_points();
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let mut p = problem(&positions, &triangles, &keys, true);
        p.class[1] = Class::Remove([0, 3]);
        let (out, stats, added) = simplify_added(&p);
        assert!(out.is_none() && added.is_empty(), "{stats:?}");
        assert!(stats.steiner_why[1] > 0, "{stats:?}");
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

    /// Two flips through the same face slots bring vertices back into faces they held. Their
    /// stale face-list entries must not make `live_faces` yield a face twice: `apply` would
    /// then move a face and kill it again, leaving a hole.
    #[test]
    fn flips_through_the_same_slots_keep_face_lists_distinct() {
        let positions = vec![[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
        let triangles = vec![[0, 1, 2], [0, 2, 3]];
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let mut p = problem(&positions, &triangles, &keys, false);
        p.class = vec![Class::Free; 4];
        p.interface = vec![false; 4];
        p.segments = HashSet::default();
        let mut st = State::new(&p);
        assert!(st.flip(0, 2), "diagonal 0-2 flips to 1-3");
        assert!(st.flip(1, 3), "and back to 0-2");
        for v in 0..4 {
            let faces: Vec<u32> = st.live_faces(v).collect();
            let mut distinct = faces.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(faces.len(), distinct.len(), "vertex {v}: {faces:?}");
        }
        // collapsing 0 onto 1 removes the face on edge 0-1 and keeps the other
        st.apply(0, 1);
        let alive: Vec<[u32; 3]> = (0..st.faces.len())
            .filter(|&f| st.alive[f])
            .map(|f| st.faces[f])
            .collect();
        assert_eq!(alive.len(), 1, "{alive:?}");
        assert_eq!(st.alive_faces, 1);
        assert!(alive[0].contains(&1) && !alive[0].contains(&0), "{alive:?}");
    }

    /// The job's cancellation is polled inside the pass: a cancelled pass stops at its first
    /// poll with no result, and records it.
    #[test]
    fn cancellation_stops_the_pass_without_a_result() {
        let (positions, triangles, class) = grid(32);
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let n = positions.len();
        let polls = Cell::new(0u32);
        let cancel = || {
            polls.set(polls.get() + 1);
            true
        };
        let p = Problem {
            positions: &positions,
            triangles: &triangles,
            class,
            interface: vec![false; n],
            segments: HashSet::default(),
            target: triangles.len() / 8,
            limit: triangles.len() / 8,
            min_cos: 0.,
            owns_segment: None,
            owns_face: None,
            keys: &keys,
            steiner: false,
            snap: None,
            protected: None,
            shared: None,
            work_limit: u64::MAX,
            cancel: Some(&cancel),
        };
        let (out, stats) = simplify(&p);
        assert!(out.is_none());
        assert!(stats.cancelled && stats.collapses == 0, "{stats:?}");
        assert_eq!(polls.get(), 1);
    }

    /// A stuck removal r whose shared apex x is also the apex of a large open fan: the
    /// rescue rounds search r's 2-ring for free collapses, which used to take in the fan's
    /// whole rim (k vertices) every round. The rim is not local to r and is now skipped:
    /// no rescue collapse, and rescue work independent of k.
    #[test]
    fn rescue_does_not_search_through_a_high_valence_neighbour() {
        let k = 1000u32;
        let (mut positions, mut triangles) = fan_of_shared_points();
        for i in 0..=k {
            let t = std::f32::consts::PI * i as f32 / k as f32;
            positions.push([1. + t.cos(), 1. + t.sin(), 0.]);
        }
        triangles.extend((0..k).map(|i| [3, 4 + i, 5 + i]));
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let n = positions.len();
        let mut p = problem(&positions, &triangles, &keys, false);
        p.class.resize(n, Class::Free);
        p.interface.resize(n, false);
        p.target = triangles.len();
        p.limit = triangles.len();
        let (out, stats) = simplify(&p);
        assert!(out.is_none(), "r stays stuck without a Steiner rescue");
        assert_eq!(stats.rescues, 0, "{stats:?}");
        assert!(stats.rescue_work < 16 * u64::from(k), "{stats:?}");
    }

    /// A planar grid of `n`×`n` cells, its outer ring locked, no interface.
    fn grid(n: u32) -> (Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<Class>) {
        let w = n + 1;
        let mut positions = Vec::new();
        let mut class = Vec::new();
        for j in 0..w {
            for i in 0..w {
                positions.push([i as f32, j as f32, 0.]);
                let edge = i == 0 || j == 0 || i == n || j == n;
                class.push(if edge { Class::Locked } else { Class::Free });
            }
        }
        let mut triangles = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let v = j * w + i;
                triangles.push([v, v + 1, v + w + 1]);
                triangles.push([v, v + w + 1, v + w]);
            }
        }
        (positions, triangles, class)
    }

    /// A raised, non-convex fan: one interior apex joined to every vertex of a star-shaped
    /// rim. Collapsing the apex onto any rim vertex turns faces over, and every rim
    /// collapse along the boundary changes the apex's neighbours: without a work bound the
    /// apex is re-checked against all k candidates after each of k collapses.
    fn fan(k: u32) -> (Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<Class>) {
        let mut positions = vec![[0., 0., 1.]];
        for i in 0..k {
            let a = std::f32::consts::TAU * i as f32 / k as f32;
            let r = if i % 2 == 0 { 1. } else { 0.5 };
            positions.push([r * a.cos(), r * a.sin(), 0.]);
        }
        let triangles = (0..k).map(|i| [0, 1 + i, 1 + (i + 1) % k]).collect();
        (positions, triangles, vec![Class::Free; k as usize + 1])
    }

    fn free(
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        class: Vec<Class>,
        keys: &[Key],
        target: usize,
        work_limit: u64,
    ) -> Stats {
        let n = positions.len();
        let p = Problem {
            positions,
            triangles,
            class,
            interface: vec![false; n],
            segments: HashSet::default(),
            target,
            limit: target,
            min_cos: 0.,
            owns_segment: None,
            owns_face: None,
            keys,
            steiner: false,
            snap: None,
            protected: None,
            shared: None,
            work_limit,
            cancel: None,
        };
        let (out, stats) = simplify(&p);
        assert!(
            !stats.exhausted || out.is_none(),
            "an exhausted pass has no result"
        );
        stats
    }

    /// The main loop's work by site and the rescue rounds account for all the pass's work.
    #[test]
    fn site_work_accounts_for_the_pass() {
        for (positions, triangles, class, target) in [
            {
                let (p, t, c) = grid(64);
                let n = t.len() / 8;
                (p, t, c, n)
            },
            {
                let (p, t, c) = fan(4000);
                (p, t, c, 16)
            },
        ] {
            let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
            let stats = free(&positions, &triangles, class, &keys, target, u64::MAX);
            let s = stats.site;
            let main = s.queue + s.candidates + s.checks + s.apply + s.requeue + s.neighbours;
            assert_eq!(main + stats.rescue_work, stats.work, "{stats:?}");
            assert!(
                s.refused_checks <= s.checks && s.pops > 0 && s.checks_run > 0,
                "{s:?}"
            );
            assert!(
                s.max_check <= s.checks && s.max_candidates <= s.candidates,
                "{s:?}"
            );
        }
    }

    /// A closed prism: two k-vertex cap fans (apexes 0 and 1 at the cap centres) joined by a
    /// wall strip, 4k faces; the rims jittered so the walls are not planar.
    fn prism(k: u32) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let mut positions = vec![[0f32, 0., 0.], [0., 0., 1.]];
        for ring in 0..2u32 {
            for i in 0..k {
                let a = std::f32::consts::TAU * i as f32 / k as f32;
                let r = 1. + 0.01 * ((i * 7 + ring * 3) % 5) as f32;
                positions.push([r * a.cos(), r * a.sin(), ring as f32]);
            }
        }
        let (b, t) = (|i: u32| 2 + i % k, |i: u32| 2 + k + i % k);
        let mut triangles = Vec::new();
        for i in 0..k {
            triangles.push([0, b(i + 1), b(i)]);
            triangles.push([1, t(i), t(i + 1)]);
            triangles.push([b(i), b(i + 1), t(i + 1)]);
            triangles.push([b(i), t(i + 1), t(i)]);
        }
        (positions, triangles)
    }

    impl State<'_> {
        /// The check as it was before the star cache, the probe and the target-side
        /// shortcuts: u's and v's stars and neighbours collected afresh, every moved face
        /// tested in turn, and the fold guard's book rule on every touched edge
        /// (`books_hold`, unfiltered). No cache, for the differential test.
        fn reference_check(&self, u: u32, v: u32) -> u8 {
            let star_u: Vec<u32> = self.live_faces(u).collect();
            let (shared, moved): (Vec<u32>, Vec<u32>) = star_u
                .iter()
                .partition(|&&f| self.faces[f as usize].contains(&v));
            if shared.is_empty() {
                return 12;
            }
            let star_v = self.live_faces(v).count();
            if moved.is_empty() && star_v == shared.len() {
                return 1;
            }
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
            let of_v: HashSet<u32> = nv.iter().copied().collect();
            if nu
                .iter()
                .any(|&w| w != v && of_v.contains(&w) && !expected.contains(&w))
            {
                return 2;
            }
            if !self.is_boundary_edge(u, v) && self.on_boundary(u, &nu) && self.on_boundary(v, &nv)
            {
                return 3;
            }
            let mut delta: HashMap<(u32, u32), i32> = HashMap::default();
            let mut new_faces: HashSet<[u32; 3]> = HashSet::default();
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
                    return 6;
                }
                if src == [0, 0]
                    && g.iter().all(|&w| iface[w as usize])
                    && !self.p.owns_face.is_some_and(|own| own(g))
                {
                    return 7;
                }
                if self.face_set.contains_key(&s) || new_faces.contains(&s) {
                    return 8;
                }
                for j in 0..3 {
                    *delta.entry((t[j], t[(j + 1) % 3])).or_default() -= 1;
                    *delta.entry((g[j], g[(j + 1) % 3])).or_default() += 1;
                }
                for &w in &g {
                    if w != v
                        && iface[v as usize]
                        && iface[w as usize]
                        && self.shared_licence(v, w) == Some(0)
                        && self.shared_licence(w, v) == Some(0)
                    {
                        return 9;
                    }
                }
                new_faces.insert(s);
            }
            for (&(a, b), &d) in &delta {
                if d > 0 {
                    let now = i64::from(self.dir.get(&(a, b)).copied().unwrap_or(0)) + i64::from(d);
                    let src = self.src_dir.get(&(a, b)).copied().unwrap_or(0);
                    let shared = if iface[a as usize] && iface[b as usize] {
                        self.shared_licence(a, b)
                    } else {
                        None
                    };
                    if now > i64::from(shared.unwrap_or(src.max(1))) {
                        return 10;
                    }
                }
            }
            for &(a, b) in delta.keys() {
                let e = (a.min(b), a.max(b));
                if self.p.segments.contains(&e) {
                    let after = i64::from(self.undirected(e.0, e.1))
                        + i64::from(delta.get(&e).copied().unwrap_or(0))
                        + i64::from(delta.get(&(e.1, e.0)).copied().unwrap_or(0));
                    if after <= 0 {
                        return 11;
                    }
                }
            }
            // The touching-boundary rule, then the contraction fold guard's book rule (the
            // rules the check gained since).
            if self.touching_changed(u, v, &delta) {
                return 13;
            }
            let rewritten: Vec<(u32, [u32; 3])> = moved
                .iter()
                .map(|&f| {
                    (
                        f,
                        self.faces[f as usize].map(|w| if w == u { v } else { w }),
                    )
                })
                .collect();
            if !self.books_hold(&shared, &rewritten, &[], None) {
                return 10;
            }
            0
        }
    }

    /// Every live face's boundary edges (used by one face) with an endpoint on a touching
    /// boundary (`State::touching`), with `u` merged into `v`, as a sorted list.
    fn touching_edges(st: &State<'_>, merge: Option<(u32, u32)>) -> Vec<(u32, u32)> {
        let mut faces = HashSet::default();
        for v in 0..st.vf.len() as u32 {
            faces.extend(st.live_faces(v));
        }
        let mut und: HashMap<(u32, u32), u32> = HashMap::default();
        for &f in &faces {
            let t = st.faces[f as usize];
            for j in 0..3 {
                let (a, b) = (t[j], t[(j + 1) % 3]);
                *und.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let m = |x: u32| match merge {
            Some((u, v)) if x == u => v,
            _ => x,
        };
        // With a merge, u and v are both on a touching boundary if either is (the collapse
        // passes the flag on).
        let near = merge.is_some_and(|(u, v)| st.touching[u as usize] || st.touching[v as usize]);
        let on = |x: u32| {
            st.touching[x as usize] || (near && merge.is_some_and(|(u, v)| x == u || x == v))
        };
        let mut out: Vec<(u32, u32)> = und
            .into_iter()
            .filter(|&((a, b), c)| c == 1 && (on(a) || on(b)))
            .map(|((a, b), _)| (m(a).min(m(b)), m(a).max(m(b))))
            .filter(|&(a, b)| a != b)
            .collect();
        out.sort_unstable();
        out
    }

    /// The topology floor refuses any change to a boundary that reaches a contract point
    /// (its touching-boundary rule). On rough meshes (holes, fins, a fan apex) with random
    /// contract points, every collapse the check accepts keeps those boundaries' edges, up
    /// to merging u into v. Without the check's touching rule, collapses that cover or drop
    /// such an edge pass the link and pinch tests.
    #[test]
    fn accepted_collapses_keep_touching_boundaries() {
        let mut accepted = 0u64;
        for seed in 0..64u64 {
            let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
            let mut next = move || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 40) as f32 / (1u64 << 24) as f32
            };
            let (positions, triangles) = rough_mesh(&mut next);
            let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
            let n = positions.len();
            let interface: Vec<bool> = (0..n).map(|_| next() < 0.2).collect();
            let p = Problem {
                positions: &positions,
                triangles: &triangles,
                class: vec![Class::Free; n],
                interface,
                segments: HashSet::default(),
                target: 0,
                limit: 0,
                min_cos: 0.,
                owns_segment: None,
                owns_face: None,
                keys: &keys,
                steiner: false,
                snap: None,
                protected: None,
                shared: None,
                work_limit: u64::MAX,
                cancel: None,
            };
            let mut st = State::new(&p);
            let mut nb = Vec::new();
            for _ in 0..3 * n {
                let u = (next() * n as f32) as u32 % n as u32;
                if st.dead[u as usize] || st.vf[u as usize].is_empty() {
                    continue;
                }
                st.neighbours(u, &mut nb);
                let Some(&v) = nb.clone().iter().find(|&&v| st.check(u, v) == 0) else {
                    continue;
                };
                let before = touching_edges(&st, Some((u, v)));
                st.apply(u, v);
                assert_eq!(
                    touching_edges(&st, None),
                    before,
                    "seed {seed}: collapse {u} onto {v}"
                );
                accepted += 1;
            }
        }
        assert!(accepted > 1000, "{accepted}");
    }

    /// A random mesh for the differential test: a jittered grid (boundary on its rim) with
    /// random diagonals and heights, fins on some interior edges (three faces on an edge),
    /// and an open or closed fan whose apex has valence 65 to 120.
    fn rough_mesh(next: &mut impl FnMut() -> f32) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let n = 6 + (next() * 7.) as u32;
        let mut positions = Vec::new();
        for i in 0..=n {
            for j in 0..=n {
                let (x, y) = (
                    i as f32 + 0.4 * (next() - 0.5),
                    j as f32 + 0.4 * (next() - 0.5),
                );
                positions.push([x, y, 2. * next()]);
            }
        }
        let at = |i: u32, j: u32| i * (n + 1) + j;
        let mut triangles = Vec::new();
        for i in 0..n {
            for j in 0..n {
                let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
                if next() < 0.5 {
                    triangles.extend([[a, b, c], [a, c, d]]);
                } else {
                    triangles.extend([[a, b, d], [b, c, d]]);
                }
            }
        }
        // Holes: open boundaries inside the grid, so a common neighbour often holds a
        // single face on its edge to the target.
        triangles.retain(|_| next() >= 0.12);
        let grid_faces = triangles.len();
        // Fins: mostly source books (a second face using a grid edge in the same
        // direction), else a reversed third face.
        for _ in 0..3 + (next() * 6.) as usize {
            let t = triangles[(next() * grid_faces as f32) as usize % grid_faces];
            let (a, b) = (t[0], t[1]);
            let c = positions.len() as u32;
            let (pa, pb) = (positions[a as usize], positions[b as usize]);
            positions.push([(pa[0] + pb[0]) / 2., (pa[1] + pb[1]) / 2., 3. + next()]);
            triangles.push(if next() < 0.75 { [a, b, c] } else { [b, a, c] });
        }
        let k = 65 + (next() * 56.) as u32;
        let apex = positions.len() as u32;
        let (ox, oy) = (n as f32 + 10., n as f32 / 2.);
        positions.push([ox, oy, 1. + next()]);
        for i in 0..k {
            let a = std::f32::consts::TAU * i as f32 / k as f32;
            let r = 4. + next();
            positions.push([ox + r * a.cos(), oy + r * a.sin(), next()]);
        }
        let closed = next() < 0.5;
        for i in 0..k {
            if i + 1 == k && !closed {
                break;
            }
            triangles.push([apex, apex + 1 + i, apex + 1 + (i + 1) % k]);
        }
        (positions, triangles)
    }

    /// The star cache, the high-valence probe and the target-side shortcuts change only
    /// what a check costs: on random meshes with boundaries, nonmanifold edges and stars
    /// of valence above `RETRY_VALENCE`, every check accepts exactly when the reference
    /// (the check as it was) accepts, with collapses and flips interleaved so cached stars
    /// go stale. Refusal reasons may differ (a probe refusal reports the sampled face's).
    #[test]
    fn checks_decide_as_the_reference_check() {
        let mut compared = [0u64; 3];
        for seed in 0..48u64 {
            let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
            let mut next = move || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 40) as f32 / (1u64 << 24) as f32
            };
            let (positions, triangles) = rough_mesh(&mut next);
            let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
            let n = positions.len();
            let mut und: HashMap<(u32, u32), u32> = HashMap::default();
            for t in &triangles {
                for j in 0..3 {
                    let (a, b) = (t[j], t[(j + 1) % 3]);
                    *und.entry((a.min(b), a.max(b))).or_default() += 1;
                }
            }
            let mut boundary = vec![false; n];
            for (&(a, b), &c) in &und {
                if c == 1 {
                    boundary[a as usize] = true;
                    boundary[b as usize] = true;
                }
            }
            let interface: Vec<bool> = (0..n)
                .map(|v| next() < if boundary[v] { 0.5 } else { 0.05 })
                .collect();
            let mut edges: Vec<(u32, u32)> = und.keys().copied().collect();
            edges.sort_unstable();
            let segments: HashSet<(u32, u32)> = edges
                .into_iter()
                .filter(|&(a, b)| interface[a as usize] && interface[b as usize] && next() < 0.5)
                .collect();
            let owns_segment = |a: u32, b: u32| (a ^ b) & 1 == 0;
            let owns_face = |f: [u32; 3]| (f[0] ^ f[1] ^ f[2]) & 1 == 0;
            let seam_owner = (seed / 3) % 2 == 0;
            let p = Problem {
                positions: &positions,
                triangles: &triangles,
                class: vec![Class::Free; n],
                interface,
                segments,
                target: 0,
                limit: 0,
                min_cos: [0., 0.3, 0.9][(seed % 3) as usize],
                owns_segment: seam_owner.then_some(&owns_segment as &dyn Fn(u32, u32) -> bool),
                owns_face: seam_owner.then_some(&owns_face as &dyn Fn([u32; 3]) -> bool),
                keys: &keys,
                steiner: false,
                snap: None,
                protected: None,
                shared: None,
                work_limit: u64::MAX,
                cancel: None,
            };
            let mut st = State::new(&p);
            let mut nb = Vec::new();
            for _ in 0..3 * n {
                let u = (next() * n as f32) as u32 % n as u32;
                if st.dead[u as usize] || st.vf[u as usize].is_empty() {
                    continue;
                }
                st.neighbours(u, &mut nb);
                let targets = nb.clone();
                let mut accepted = Vec::new();
                for &v in &targets {
                    let want = st.reference_check(u, v) == 0;
                    let got = st.check(u, v) == 0;
                    assert_eq!(got, want, "seed {seed}: check({u}, {v})");
                    let probed = st.live_faces(u).count() > RETRY_VALENCE;
                    compared[usize::from(got)] += 1;
                    compared[2] += u64::from(probed);
                    if got {
                        accepted.push(v);
                    }
                }
                // Change the mesh while u's star is cached: a collapse of a neighbour of u
                // (its target chosen by the reference, so the cache still holds u) or a
                // flip of an edge at u, then check u again; or a collapse of u itself.
                let roll = next();
                let x = targets
                    .get((next() * targets.len() as f32) as usize % targets.len().max(1))
                    .copied();
                if roll < 0.3 {
                    if let Some(x) = x {
                        let mut nx = Vec::new();
                        st.neighbours(x, &mut nx);
                        if let Some(&y) = nx.iter().find(|&&y| st.reference_check(x, y) == 0) {
                            st.apply(x, y);
                        }
                    }
                } else if roll < 0.6 {
                    if let Some(x) = x {
                        st.flip(u, x);
                    }
                } else if roll < 0.8 && !accepted.is_empty() {
                    let v = accepted[(next() * accepted.len() as f32) as usize % accepted.len()];
                    st.apply(u, v);
                    continue;
                }
                if st.dead[u as usize] {
                    continue;
                }
                st.neighbours(u, &mut nb);
                for &v in &nb.clone() {
                    let want = st.reference_check(u, v) == 0;
                    let got = st.check(u, v) == 0;
                    assert_eq!(got, want, "seed {seed}: check({u}, {v}) after a change");
                    compared[usize::from(got)] += 1;
                }
            }
        }
        // Both outcomes, and the probe's path, are exercised.
        assert!(compared.iter().all(|&c| c > 1000), "{compared:?}");
    }

    /// The cap apexes themselves (valence k) are popped and refused against most of their
    /// targets, one turned-over face deciding each. A check collected the apex's star and
    /// neighbours for every target (about 856 units per face at k = 2000); the star is now
    /// collected once per pop and a spread sample of the moved faces decides most refusals.
    #[test]
    fn a_high_valence_vertex_pays_its_star_once_per_pop() {
        let (positions, triangles) = prism(2000);
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let n = positions.len();
        let faces = triangles.len() as u64;
        let stats = free(
            &positions,
            &triangles,
            vec![Class::Free; n],
            &keys,
            triangles.len() / 8,
            u64::MAX,
        );
        assert!(stats.why[5] > 0, "{stats:?}");
        assert!(stats.work < 200 * faces, "{} for {faces} faces", stats.work);
    }

    /// A rim vertex is tried against its cap's apex, of valence k, over and over when most
    /// collapses are refused (here a strict normal floor). A check used to scan the target's
    /// whole star and neighbours, so the work grew with k per refusal: 132,972 units per
    /// face at k = 2000. A check now costs the moving vertex's star.
    #[test]
    fn refused_checks_do_not_pay_the_targets_valence() {
        let (positions, triangles) = prism(2000);
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let n = positions.len();
        let p = Problem {
            positions: &positions,
            triangles: &triangles,
            class: vec![Class::Free; n],
            interface: vec![false; n],
            segments: HashSet::default(),
            target: triangles.len() / 8,
            limit: triangles.len() / 8,
            min_cos: 0.95,
            owns_segment: None,
            owns_face: None,
            keys: &keys,
            steiner: false,
            snap: None,
            protected: None,
            shared: None,
            work_limit: u64::MAX,
            cancel: None,
        };
        let (_, stats) = simplify(&p);
        let faces = triangles.len() as u64;
        assert!(
            stats.site.refused_checks > stats.site.checks / 2,
            "{stats:?}"
        );
        assert!(
            stats.work < 8_000 * faces,
            "{} for {faces} faces",
            stats.work
        );
    }

    /// The work count grows linearly on an ordinary mesh and on a high-valence fan whose
    /// apex has no valid collapse, and the limit stops a pass with no result, so the ladder
    /// continues instead.
    #[test]
    fn work_is_linear_on_a_grid_and_bounded_on_a_fan() {
        let (positions, triangles, class) = grid(128);
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let faces = triangles.len() as u64;
        let stats = free(
            &positions,
            &triangles,
            class,
            &keys,
            triangles.len() / 8,
            u64::MAX,
        );
        assert!(!stats.exhausted);
        assert!(stats.work < 64 * faces, "{} for {faces} faces", stats.work);
        let (positions, triangles, class) = fan(4000);
        let keys: Vec<Key> = positions.iter().map(|&p| key(p)).collect();
        let faces = triangles.len() as u64;
        // The stuck apex is retried only after a share of its rim changed, and tries its
        // cheapest targets: work stays linear (it exceeded 1,000 units per face before).
        let unbounded = free(&positions, &triangles, class.clone(), &keys, 16, u64::MAX);
        assert!(!unbounded.exhausted, "{unbounded:?}");
        assert!(unbounded.work < 256 * faces, "{unbounded:?}");
        // A smaller budget still stops the pass, with no result.
        let bounded = free(&positions, &triangles, class, &keys, 16, faces);
        assert!(bounded.exhausted, "{bounded:?}");
        assert!(bounded.work <= faces + 4 * 4001, "{bounded:?}");
    }
}
