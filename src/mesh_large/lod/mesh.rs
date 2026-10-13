//! A node's geometry in KEY space: vertices are welded source positions (normalized f32
//! bits), faces carry their source primitive label. Simplification only ever selects
//! existing source vertices (half-edge collapse), so a proxy vertex is a source vertex and
//! quantizes on the frozen lattice exactly as every neighbour does.
use super::incidence::VertexInfo;
use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BTreeSet, BinaryHeap},
};

pub(crate) type Key = [u32; 3];

pub(crate) fn position(key: Key) -> [f64; 3] {
    key.map(|b| f64::from(f32::from_bits(b)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Label {
    pub key: (u32, u32, u32),
    pub expanded_ordinal: u64,
    pub material: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Vertex {
    pub key: Key,
    pub info: VertexInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Face {
    pub v: [u32; 3],
    pub label: u32,
    /// Index of the input face this one descends from (collapses keep it). A fill face
    /// carries `u32::MAX - k` for its fill record `k` (never a valid input index).
    pub id: u32,
}

/// The source triangle a face descends from (`triangle == u64::MAX` for a fill).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) struct Origin {
    pub node: u32,
    pub mesh: u32,
    pub primitive: u32,
    pub triangle: u64,
}

/// Indexed key-space geometry; `vertices` sorted by key, `labels` sorted.
#[derive(Clone, Debug, Default)]
pub(crate) struct KeyMesh {
    pub vertices: Vec<Vertex>,
    pub labels: Vec<Label>,
    pub faces: Vec<Face>,
}

/// One face as assembled from a child: welded keys, facts, label.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeyFace {
    pub keys: [Key; 3],
    pub info: [VertexInfo; 3],
    pub label: Label,
    pub origin: Origin,
}

impl KeyMesh {
    /// Weld faces by key; faces keep their order.
    pub(crate) fn build(faces: &[KeyFace]) -> Self {
        let mut keys: Vec<(Key, VertexInfo)> = faces
            .iter()
            .flat_map(|f| (0..3).map(move |i| (f.keys[i], f.info[i])))
            .collect();
        keys.sort_unstable_by_key(|a| a.0);
        keys.dedup_by(|a, b| a.0 == b.0);
        let mut labels: Vec<Label> = faces.iter().map(|f| f.label).collect();
        labels.sort_unstable();
        labels.dedup();
        let vertex = |k: &Key| keys.binary_search_by(|x| x.0.cmp(k)).unwrap() as u32;
        let label = |l: &Label| labels.binary_search(l).unwrap() as u32;
        let faces = faces
            .iter()
            .enumerate()
            .map(|(i, f)| Face {
                v: [vertex(&f.keys[0]), vertex(&f.keys[1]), vertex(&f.keys[2])],
                label: label(&f.label),
                id: i as u32,
            })
            .collect();
        Self {
            vertices: keys
                .into_iter()
                .map(|(key, info)| Vertex { key, info })
                .collect(),
            labels,
            faces,
        }
    }

    pub(crate) fn face_area(&self, f: &Face) -> f64 {
        let p = f.v.map(|i| position(self.vertices[i as usize].key));
        let n = normal(p);
        0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
    }
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
pub(crate) fn normal(p: [[f64; 3]; 3]) -> [f64; 3] {
    cross(sub(p[1], p[0]), sub(p[2], p[0]))
}

/// A connected component (welded-vertex connectivity over all labels).
#[derive(Clone, Debug)]
pub(crate) struct Component {
    pub faces: Vec<u32>,
    pub area: f64,
    pub cut: bool,
    pub min_key: Key,
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        parent[x as usize] = parent[parent[x as usize] as usize];
        x = parent[x as usize];
    }
    x
}

/// Components in canonical order (by minimum key).
pub(crate) fn components(mesh: &KeyMesh, depth: u32) -> Vec<Component> {
    let mut parent: Vec<u32> = (0..mesh.vertices.len() as u32).collect();
    for f in &mesh.faces {
        for i in 1..3 {
            let (a, b) = (find(&mut parent, f.v[0]), find(&mut parent, f.v[i]));
            if a != b {
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                parent[hi as usize] = lo;
            }
        }
    }
    let mut by_root: BTreeMap<u32, Component> = BTreeMap::new();
    for (index, f) in mesh.faces.iter().enumerate() {
        let root = find(&mut parent, f.v[0]);
        let c = by_root.entry(root).or_insert_with(|| Component {
            faces: Vec::new(),
            area: 0.,
            cut: false,
            min_key: [u32::MAX; 3],
        });
        c.faces.push(index as u32);
        c.area += mesh.face_area(f);
        for &v in &f.v {
            let vertex = mesh.vertices[v as usize];
            c.cut |= vertex.info.cut(depth);
            c.min_key = c.min_key.min(vertex.key);
        }
    }
    // Roots are the least vertex index, i.e. the least key: map order is key order.
    by_root.into_values().collect()
}

/// Quadric error metric (symmetric 4x4 as 10 values).
#[derive(Clone, Copy, Debug, Default)]
struct Quadric([f64; 10]);

impl Quadric {
    fn plane(n: [f64; 3], d: f64, w: f64) -> Self {
        let [a, b, c] = n;
        Self([
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
        ])
    }
    fn add(&mut self, o: &Self) {
        for i in 0..10 {
            self.0[i] += o.0[i];
        }
    }
    fn eval(&self, p: [f64; 3]) -> f64 {
        let q = &self.0;
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
}

/// Vertex role during simplification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Free,
    /// On exactly two feature (border/seam) edges, to these neighbours.
    Chain(u32, u32),
    Locked,
}

#[derive(Clone, Copy, Debug)]
struct Cost(f64);
impl PartialEq for Cost {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Cost {}
impl PartialOrd for Cost {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Cost {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Simplification statistics.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SimplifyReport {
    pub input_faces: u64,
    pub forced: u64,
    pub forced_unchecked: u64,
    pub collapses: u64,
    pub output_faces: u64,
    pub locked: u64,
    /// Rejected collapse candidates: link, orphan, label, survivors, degenerate, fold,
    /// role, seam reach.
    pub rejected: [u64; 8],
}

/// The one-ring of a vertex: `(neighbour, face)` pairs sorted, one per face per neighbour.
pub(crate) struct Ring {
    pairs: Vec<(u32, u32)>,
    faces: Vec<u32>,
}

impl Ring {
    fn runs(&self) -> impl Iterator<Item = (u32, &[u32])> + '_ {
        let mut i = 0;
        std::iter::from_fn(move || {
            if i >= self.pairs.len() {
                return None;
            }
            let w = self.pairs[i].0;
            let start = i;
            while i < self.pairs.len() && self.pairs[i].0 == w {
                i += 1;
            }
            Some((w, &self.faces[start..i]))
        })
    }
    fn get(&self, w: &u32) -> Option<&[u32]> {
        let start = self.pairs.partition_point(|p| p.0 < *w);
        let end = self.pairs.partition_point(|p| p.0 <= *w);
        (start < end).then(|| &self.faces[start..end])
    }
    fn contains_key(&self, w: &u32) -> bool {
        self.get(w).is_some()
    }
    fn keys(&self) -> impl Iterator<Item = &u32> + '_ {
        self.pairs
            .iter()
            .enumerate()
            .filter(|(i, p)| *i == 0 || self.pairs[i - 1].0 != p.0)
            .map(|(_, p)| &p.0)
    }
    fn into_keys(self) -> impl Iterator<Item = u32> {
        let mut keys: Vec<u32> = self.pairs.iter().map(|p| p.0).collect();
        keys.dedup();
        keys.into_iter()
    }
    fn values(&self) -> impl Iterator<Item = &[u32]> + '_ {
        self.runs().map(|(_, f)| f)
    }
    fn iter(&self) -> impl Iterator<Item = (u32, &[u32])> + '_ {
        self.runs()
    }
}

/// Half-edge collapse simplifier over a key mesh.
pub(crate) struct Simplifier {
    pos: Vec<[f64; 3]>,
    quadric: Vec<Quadric>,
    alive_v: Vec<bool>,
    locked: Vec<bool>,
    stamp: Vec<u32>,
    faces: Vec<Face>,
    alive_f: Vec<bool>,
    vfaces: Vec<Vec<u32>>,
    live_faces: usize,
    label_live: Vec<u32>,
    /// After the strict pass cannot reach `2P`: topology-locked (non-seam) vertices may
    /// collapse too, and a primitive may lose its last faces (recorded as removed).
    relaxed: bool,
    /// Seam vertices not on the source boundary: true-boundary decimation must not reach
    /// them (their border edges stay seam edges, which the depth audit then checks exactly).
    seam: Vec<bool>,
    /// All seam vertices: the relaxed pass may not collapse onto them, so it never kills a
    /// face holding a seam edge.
    cut: Vec<bool>,
    rejected: std::cell::Cell<[u64; 8]>,
    pub report: SimplifyReport,
}

const FEATURE_WEIGHT: f64 = 4.;
const FLIP_COS: f64 = 0.2;

impl Simplifier {
    /// `locked[v]`: vertices that must survive unmoved (kept seam vertices).
    pub(crate) fn new(mesh: &KeyMesh, faces: &[Face], locked: Vec<bool>) -> Self {
        let pos: Vec<[f64; 3]> = mesh.vertices.iter().map(|v| position(v.key)).collect();
        let mut vfaces = vec![Vec::new(); pos.len()];
        for (i, f) in faces.iter().enumerate() {
            for &v in &f.v {
                vfaces[v as usize].push(i as u32);
            }
        }
        let mut s = Self {
            quadric: vec![Quadric::default(); pos.len()],
            alive_v: vfaces.iter().map(|f| !f.is_empty()).collect(),
            stamp: vec![0; pos.len()],
            locked,
            pos,
            faces: faces.to_vec(),
            alive_f: vec![true; faces.len()],
            vfaces,
            live_faces: faces.len(),
            label_live: {
                let mut counts = vec![0u32; mesh.labels.len()];
                for f in faces {
                    counts[f.label as usize] += 1;
                }
                counts
            },
            relaxed: false,
            seam: Vec::new(),
            cut: Vec::new(),
            rejected: std::cell::Cell::new([0; 8]),
            report: SimplifyReport {
                input_faces: faces.len() as u64,
                ..Default::default()
            },
        };
        for i in 0..s.faces.len() {
            let f = s.faces[i];
            let p = f.v.map(|v| s.pos[v as usize]);
            let n = normal(p);
            let len = dot(n, n).sqrt();
            if len > 0. && len.is_finite() {
                let unit = n.map(|x| x / len);
                let q = Quadric::plane(unit, -dot(unit, p[0]), 0.5 * len);
                for &v in &f.v {
                    s.quadric[v as usize].add(&q);
                }
            }
        }
        // Feature edges: constraint planes through the edge, perpendicular to its face.
        let edges = s.edge_faces();
        for ((a, b), list) in &edges {
            let feature = list.len() != 2
                || s.faces[list[0] as usize].label != s.faces[list[1] as usize].label;
            if !feature {
                continue;
            }
            for &fi in list {
                let f = s.faces[fi as usize];
                let p = f.v.map(|v| s.pos[v as usize]);
                let n = normal(p);
                let e = sub(s.pos[*b as usize], s.pos[*a as usize]);
                let m = cross(e, n);
                let len = dot(m, m).sqrt();
                if len > 0. && len.is_finite() {
                    let unit = m.map(|x| x / len);
                    let q = Quadric::plane(
                        unit,
                        -dot(unit, s.pos[*a as usize]),
                        FEATURE_WEIGHT * dot(e, e),
                    );
                    s.quadric[*a as usize].add(&q);
                    s.quadric[*b as usize].add(&q);
                }
            }
        }
        s
    }

    fn edge_faces(&self) -> BTreeMap<(u32, u32), Vec<u32>> {
        let mut edges: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
        for (i, f) in self.faces.iter().enumerate() {
            if !self.alive_f[i] {
                continue;
            }
            for k in 0..3 {
                let (a, b) = (f.v[k], f.v[(k + 1) % 3]);
                if a != b {
                    edges
                        .entry((a.min(b), a.max(b)))
                        .or_default()
                        .push(i as u32);
                }
            }
        }
        edges
    }

    fn live(&self, v: u32) -> impl Iterator<Item = u32> + '_ {
        self.vfaces[v as usize]
            .iter()
            .copied()
            .filter(move |&f| self.alive_f[f as usize] && self.faces[f as usize].v.contains(&v))
    }

    /// Neighbours of `u` with, per neighbour, the faces on edge `(u, w)`.
    fn ring(&self, u: u32) -> Ring {
        let mut pairs: Vec<(u32, u32)> = Vec::with_capacity(16);
        for f in self.live(u) {
            for &w in &self.faces[f as usize].v {
                if w != u {
                    pairs.push((w, f));
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        let faces = pairs.iter().map(|p| p.1).collect();
        Ring { pairs, faces }
    }

    fn role(&self, u: u32) -> Role {
        if self.locked[u as usize] {
            return Role::Locked;
        }
        self.role_in(u, &self.ring(u))
    }

    fn role_in(&self, u: u32, ring: &Ring) -> Role {
        if self.locked[u as usize] {
            return Role::Locked;
        }
        let mut features = Vec::new();
        for (w, list) in ring.iter() {
            if list.len() > 2 {
                return if self.relaxed {
                    Role::Free
                } else {
                    Role::Locked
                };
            }
            let feature = list.len() == 1
                || self.faces[list[0] as usize].label != self.faces[list[1] as usize].label;
            if feature {
                features.push(w);
            }
        }
        match features.len() {
            0 => Role::Free,
            2 => Role::Chain(features[0], features[1]),
            _ if self.relaxed => Role::Free,
            _ => Role::Locked,
        }
    }

    /// Topology and geometry checks for collapsing `u` onto `v`.
    fn valid(&self, u: u32, v: u32, ring_u: &Ring) -> bool {
        let reason = self.why_not(u, v, ring_u);
        if let Some(r) = reason {
            self.rejected.set({
                let mut a = self.rejected.get();
                a[r] += 1;
                a
            });
        }
        reason.is_none()
    }

    fn why_not(&self, u: u32, v: u32, ring_u: &Ring) -> Option<usize> {
        let Some(shared) = ring_u.get(&v) else {
            return Some(0);
        };
        let ring_v = self.ring(v);
        let thirds: BTreeSet<u32> = shared
            .iter()
            .flat_map(|&f| self.faces[f as usize].v)
            .filter(|&w| w != u && w != v)
            .collect();
        let mut link = true;
        for w in ring_u.keys() {
            if *w != v && ring_v.contains_key(w) && !thirds.contains(w) {
                link = false;
            }
        }
        // The relaxed pass admits a non-manifold result away from seams (it never collapses
        // onto a seam vertex); folds and duplicates are cleaned up after the collapse.
        if !link && !self.relaxed {
            return Some(0);
        }
        // No other vertex may lose its last face (a kept seam vertex would vanish).
        for &w in &thirds {
            let dying = shared
                .iter()
                .filter(|&&f| self.faces[f as usize].v.contains(&w))
                .count();
            if (self.locked[w as usize] || !self.relaxed) && self.live(w).count() <= dying {
                return Some(1);
            }
        }
        if !self.relaxed {
            // No primitive loses its last face, and no piece of surface disappears.
            let mut dying: BTreeMap<u32, u32> = BTreeMap::new();
            for &f in shared {
                *dying.entry(self.faces[f as usize].label).or_default() += 1;
            }
            if dying
                .iter()
                .any(|(l, n)| self.label_live[*l as usize] <= *n)
            {
                return Some(2);
            }
            let survivors = self
                .live(u)
                .chain(self.live(v))
                .filter(|f| !shared.contains(f))
                .count();
            if survivors == 0 {
                return Some(3);
            }
        }
        for f in self.live(u) {
            let face = self.faces[f as usize];
            if face.v.contains(&v) {
                continue;
            }
            let old = normal(face.v.map(|x| self.pos[x as usize]));
            let new = normal(
                face.v
                    .map(|x| self.pos[if x == u { v } else { x } as usize]),
            );
            let (lo, ln) = (dot(old, old).sqrt(), dot(new, new).sqrt());
            if lo == 0. || self.relaxed && ln == 0. {
                continue;
            }
            if ln == 0. || !(ln.is_finite()) {
                return Some(4);
            }
            if lo > 0. && dot(old, new) < FLIP_COS * lo * ln {
                return Some(5);
            }
        }
        None
    }

    fn apply(&mut self, u: u32, v: u32) {
        let faces: Vec<u32> = self.live(u).collect();
        for f in faces {
            let face = &mut self.faces[f as usize];
            if face.v.contains(&v) {
                self.alive_f[f as usize] = false;
                self.live_faces -= 1;
                self.label_live[face.label as usize] -= 1;
                continue;
            }
            for x in &mut face.v {
                if *x == u {
                    *x = v;
                }
            }
            self.vfaces[v as usize].push(f);
        }
        let q = self.quadric[u as usize];
        self.quadric[v as usize].add(&q);
        self.alive_v[u as usize] = false;
        self.vfaces[u as usize].clear();
        let (faces, alive) = (&self.faces, &self.alive_f);
        let list = &mut self.vfaces[v as usize];
        list.retain(|&f| alive[f as usize] && faces[f as usize].v.contains(&v));
        list.sort_unstable();
        list.dedup();
        self.stamp[v as usize] += 1;
        for w in self.ring(v).into_keys() {
            self.stamp[w as usize] += 1;
        }
    }

    /// Remove faces that repeat a vertex; of same-winding duplicates (same vertex set,
    /// label and orientation) keep the first; remove opposite-winding coincident pairs
    /// entirely (a fold has no surface, and keeping one face would pinch a seam chain).
    fn cleanup(&mut self) {
        // (sorted vertices, label) -> (first live face with even winding, with odd winding)
        let mut seen: BTreeMap<([u32; 3], u32), [Vec<usize>; 2]> = BTreeMap::new();
        for i in 0..self.faces.len() {
            if !self.alive_f[i] {
                continue;
            }
            let f = self.faces[i];
            let mut sorted = f.v;
            sorted.sort_unstable();
            if sorted[0] == sorted[1] || sorted[1] == sorted[2] {
                self.kill(i);
                continue;
            }
            // Winding parity: rotate so the least vertex leads; even if the next is the middle.
            let lead = (0..3).min_by_key(|&k| f.v[k]).unwrap();
            let odd = usize::from(f.v[(lead + 1) % 3] != sorted[1]);
            seen.entry((sorted, f.label)).or_default()[odd].push(i);
        }
        for [even, odd] in seen.into_values() {
            for &i in even.iter().skip(1).chain(odd.iter().skip(1)) {
                self.kill(i);
            }
            if let (Some(&a), Some(&b)) = (even.first(), odd.first()) {
                self.kill(a);
                self.kill(b);
            }
        }
    }

    /// [`Simplifier::cleanup`] restricted to the faces around `v`.
    fn cleanup_around(&mut self, v: u32) {
        let faces: Vec<u32> = self.live(v).collect();
        let mut seen: BTreeMap<([u32; 3], u32), [Vec<usize>; 2]> = BTreeMap::new();
        for f in faces {
            let i = f as usize;
            let face = self.faces[i];
            let mut sorted = face.v;
            sorted.sort_unstable();
            if sorted[0] == sorted[1] || sorted[1] == sorted[2] {
                self.kill(i);
                continue;
            }
            let lead = (0..3).min_by_key(|&k| face.v[k]).unwrap();
            let odd = usize::from(face.v[(lead + 1) % 3] != sorted[1]);
            seen.entry((sorted, face.label)).or_default()[odd].push(i);
        }
        for [even, odd] in seen.into_values() {
            for &i in even.iter().skip(1).chain(odd.iter().skip(1)) {
                self.kill(i);
            }
            if let (Some(&a), Some(&b)) = (even.first(), odd.first()) {
                self.kill(a);
                self.kill(b);
            }
        }
    }

    fn kill(&mut self, i: usize) {
        if self.alive_f[i] {
            self.alive_f[i] = false;
            self.live_faces -= 1;
            self.label_live[self.faces[i].label as usize] -= 1;
        }
    }

    /// Remove every `forced` vertex (non-kept seam vertices), in key order, each by a valid
    /// collapse along one of its border edges where possible: the shared seam chain then
    /// runs through the kept vertices only, identically on both sides. A vertex with no
    /// valid collapse is kept (locked) and returned: the caller adds it to the depth's
    /// keep set and re-runs every tile that shares it, so both sides keep it.
    pub(crate) fn force(&mut self, forced: &[u32]) -> Vec<u32> {
        let mut failed = Vec::new();
        for &u in forced {
            if !self.alive_v[u as usize] {
                continue;
            }
            let ring = self.ring(u);
            let mut targets: Vec<u32> = ring
                .iter()
                .filter(|(_, l)| l.len() == 1)
                .map(|(w, _)| w)
                .collect();
            // Only a vertex inside one seam chain (exactly two border edges, the same two on
            // both sides) has a side-independent removal: either collapse gives the chain
            // edge between its neighbours. A pinch where the boundary touches itself is kept.
            if targets.is_empty() {
                targets = ring.keys().copied().collect();
            } else if targets.len() != 2 {
                targets.clear();
            }
            match targets.iter().copied().find(|&v| self.valid(u, v, &ring)) {
                Some(v) => {
                    self.apply(u, v);
                    self.report.forced += 1;
                }
                None => {
                    self.locked[u as usize] = true;
                    self.report.forced_unchecked += 1;
                    failed.push(u);
                }
            }
        }
        failed
    }

    fn cost(&self, u: u32, v: u32) -> f64 {
        let mut q = self.quadric[u as usize];
        q.add(&self.quadric[v as usize]);
        q.eval(self.pos[v as usize]).max(0.)
    }

    fn push(&self, heap: &mut BinaryHeap<Reverse<(Cost, u32, u32, u32)>>, u: u32) {
        if !self.alive_v[u as usize] {
            return;
        }
        if self.locked[u as usize] {
            return;
        }
        let ring = self.ring(u);
        let role = self.role_in(u, &ring);
        if role == Role::Locked {
            let mut a = self.rejected.get();
            a[6] += 1;
            self.rejected.set(a);
            return;
        }
        for &v in ring.keys() {
            if let Role::Chain(a, b) = role {
                if v != a && v != b {
                    continue;
                }
            }
            if self.boundary_reaches_seam(v, &ring) {
                let mut a = self.rejected.get();
                a[7] += 1;
                self.rejected.set(a);
                continue;
            }
            heap.push(Reverse((
                Cost(self.cost(u, v)),
                u,
                v,
                self.stamp[u as usize],
            )));
        }
    }

    /// Collapse in strict cost order (ties by vertex index, i.e. key) until `target` faces.
    /// Costs are recomputed when popped (a target's quadric may have grown since).
    pub(crate) fn simplify(&mut self, target: usize) {
        let mut heap = BinaryHeap::new();
        for u in 0..self.pos.len() as u32 {
            self.push(&mut heap, u);
        }
        while self.live_faces > target {
            let Some(Reverse((Cost(cost), u, v, su))) = heap.pop() else {
                break;
            };
            if !self.alive_v[u as usize]
                || !self.alive_v[v as usize]
                || self.stamp[u as usize] != su
            {
                continue;
            }
            let actual = self.cost(u, v);
            if actual > cost {
                heap.push(Reverse((Cost(actual), u, v, su)));
                continue;
            }
            let ring = self.ring(u);
            match self.role_in(u, &ring) {
                Role::Locked => continue,
                Role::Chain(a, b) if v != a && v != b => continue,
                _ => {}
            }
            if self.boundary_reaches_seam(v, &ring) || !self.valid(u, v, &ring) {
                continue;
            }
            self.apply(u, v);
            if self.relaxed {
                self.cleanup_around(v);
            }
            self.report.collapses += 1;
            self.push(&mut heap, v);
            for w in self.ring(v).into_keys() {
                self.push(&mut heap, w);
            }
        }
    }

    pub(crate) fn set_seam(&mut self, seam: Vec<bool>, cut: Vec<bool>) {
        self.seam = seam;
        self.cut = cut;
    }

    /// A border vertex may not collapse onto a non-boundary seam vertex; in the relaxed
    /// pass nothing collapses onto any seam vertex.
    fn boundary_reaches_seam(&self, v: u32, ring_u: &Ring) -> bool {
        (self.relaxed && self.cut.get(v as usize).copied().unwrap_or(false))
            || (self.seam.get(v as usize).copied().unwrap_or(false)
                && ring_u.values().any(|l| l.len() == 1))
    }

    pub(crate) fn live_faces(&self) -> usize {
        self.live_faces
    }

    /// Admit topology-locked (non-seam) vertices and the loss of a primitive's last faces.
    pub(crate) fn relax(&mut self) {
        self.relaxed = true;
        for s in &mut self.stamp {
            *s += 1;
        }
    }

    /// The surviving faces, in their original order.
    pub(crate) fn finish(mut self) -> (Vec<Face>, SimplifyReport) {
        self.cleanup();
        let faces: Vec<Face> = self
            .faces
            .iter()
            .zip(&self.alive_f)
            .filter(|(_, a)| **a)
            .map(|(f, _)| *f)
            .collect();
        self.report.output_faces = faces.len() as u64;
        self.report.locked = self.locked.iter().filter(|l| **l).count() as u64;
        self.report.rejected = self.rejected.get();
        (faces, self.report)
    }
}

/// A simple closed loop of border edges, as directed in their faces.
#[derive(Clone, Debug)]
pub(crate) struct Loop {
    pub vertices: Vec<u32>,
    pub label: u32,
}

/// Simple border loops whose vertices all satisfy `eligible`, of one label, with at most
/// `max` vertices, in canonical order (by least vertex).
pub(crate) fn border_loops(
    faces: &[Face],
    eligible: impl Fn(u32) -> bool,
    max: usize,
) -> Vec<Loop> {
    let mut count: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for f in faces {
        for k in 0..3 {
            let (a, b) = (f.v[k], f.v[(k + 1) % 3]);
            *count.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    // Directed border edges a -> b with their label.
    let mut next: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
    let mut incoming: BTreeMap<u32, u32> = BTreeMap::new();
    for f in faces {
        for k in 0..3 {
            let (a, b) = (f.v[k], f.v[(k + 1) % 3]);
            if a != b && count[&(a.min(b), a.max(b))] == 1 {
                next.entry(a).or_default().push((b, f.label));
                *incoming.entry(b).or_default() += 1;
            }
        }
    }
    let simple = |v: u32| {
        eligible(v) && next.get(&v).is_some_and(|n| n.len() == 1) && incoming.get(&v) == Some(&1)
    };
    let mut done = BTreeSet::new();
    let mut loops = Vec::new();
    for &start in next.keys() {
        if done.contains(&start) || !simple(start) {
            continue;
        }
        let label = next[&start][0].1;
        let mut vertices = vec![start];
        let mut at = start;
        let ok = loop {
            let (b, l) = next[&at][0];
            if l != label {
                break false;
            }
            if b == start {
                break true;
            }
            if !simple(b) || vertices.len() >= max || vertices.contains(&b) {
                break false;
            }
            vertices.push(b);
            at = b;
        };
        for v in &vertices {
            done.insert(*v);
        }
        if ok && vertices.len() >= 3 {
            loops.push(Loop { vertices, label });
        }
    }
    loops
}

/// The fan closing `l` from its least vertex, oriented against the loop's face edges.
pub(crate) fn fan(l: &Loop) -> Vec<[u32; 3]> {
    let k = l.vertices.len();
    let start = (0..k).min_by_key(|&i| l.vertices[i]).unwrap();
    let order: Vec<u32> = (0..k).map(|i| l.vertices[(start + k - i) % k]).collect();
    (1..k - 1)
        .map(|i| [order[0], order[i], order[i + 1]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(x: f32, y: f32, z: f32) -> Key {
        super::super::incidence::weld([x, y, z])
    }

    fn grid(n: usize, label: Label, info: impl Fn(usize, usize) -> VertexInfo) -> Vec<KeyFace> {
        let mut out = Vec::new();
        let p = |i: usize, j: usize| key(i as f32, j as f32, ((i * 7 + j * 3) % 5) as f32 * 0.1);
        for i in 0..n {
            for j in 0..n {
                let quad = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
                for t in [[0, 1, 2], [0, 2, 3]] {
                    out.push(KeyFace {
                        keys: t.map(|k| p(quad[k].0, quad[k].1)),
                        info: t.map(|k| info(quad[k].0, quad[k].1)),
                        label,
                        origin: Origin::default(),
                    });
                }
            }
        }
        out
    }

    const L: Label = Label {
        key: (0, 0, 0),
        expanded_ordinal: 0,
        material: u32::MAX,
    };

    #[test]
    fn simplification_keeps_locked_vertices_labels_and_reaches_its_target() {
        let faces = grid(16, L, |_, _| VertexInfo::default());
        let mesh = KeyMesh::build(&faces);
        let locked: Vec<bool> = mesh
            .vertices
            .iter()
            .map(|v| f32::from_bits(v.key[0]) == 0.)
            .collect();
        let mut s = Simplifier::new(&mesh, &mesh.faces, locked.clone());
        s.simplify(64);
        let (out, report) = s.finish();
        assert!(out.len() <= 64 + 2, "{}", out.len());
        assert!(report.collapses > 0);
        let used: BTreeSet<u32> = out.iter().flat_map(|f| f.v).collect();
        for (i, l) in locked.iter().enumerate() {
            if *l {
                assert!(used.contains(&(i as u32)), "locked vertex {i} removed");
            }
        }
        // Same input, same output.
        let mut again = Simplifier::new(&mesh, &mesh.faces, locked);
        again.simplify(64);
        assert_eq!(again.finish().0, out);
    }

    #[test]
    fn forced_border_vertices_leave_the_kept_chain() {
        let faces = grid(8, L, |_, _| VertexInfo::default());
        let mesh = KeyMesh::build(&faces);
        // Border x = 0: keep even y, force odd y.
        let on = |v: &Vertex| f32::from_bits(v.key[0]) == 0.;
        let locked: Vec<bool> = mesh
            .vertices
            .iter()
            .map(|v| on(v) && (f32::from_bits(v.key[1]) as u32).is_multiple_of(2))
            .collect();
        let forced: Vec<u32> = mesh
            .vertices
            .iter()
            .enumerate()
            .filter(|(_, v)| on(v) && !(f32::from_bits(v.key[1]) as u32).is_multiple_of(2))
            .map(|(i, _)| i as u32)
            .collect();
        let mut s = Simplifier::new(&mesh, &mesh.faces, locked);
        assert!(s.force(&forced).is_empty());
        let (out, _) = s.finish();
        let used: BTreeSet<u32> = out.iter().flat_map(|f| f.v).collect();
        for f in &forced {
            assert!(!used.contains(f));
        }
        // The x = 0 border is now the kept vertices in order.
        let mut border: Vec<(u32, u32)> = Vec::new();
        let mut count: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for f in &out {
            for k in 0..3 {
                let (a, b) = (f.v[k], f.v[(k + 1) % 3]);
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        for ((a, b), c) in count {
            if c == 1 && on(&mesh.vertices[a as usize]) && on(&mesh.vertices[b as usize]) {
                border.push((a, b));
            }
        }
        assert_eq!(border.len(), 4);
    }

    #[test]
    fn loops_and_fans_close_holes_against_their_faces() {
        // A grid with its centre quad removed: one interior loop of four.
        let mut faces = grid(4, L, |_, _| VertexInfo::default());
        let hole = 2 * (4 + 1); // quad (1, 1): faces 10 and 11
        faces.drain(hole..hole + 2);
        let mesh = KeyMesh::build(&faces);
        let boundary: BTreeSet<u32> = mesh
            .vertices
            .iter()
            .enumerate()
            .filter(|(_, v)| {
                let (x, y) = (f32::from_bits(v.key[0]), f32::from_bits(v.key[1]));
                x == 0. || y == 0. || x == 4. || y == 4.
            })
            .map(|(i, _)| i as u32)
            .collect();
        let loops = border_loops(&mesh.faces, |v| !boundary.contains(&v), 64);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].vertices.len(), 4);
        let mut closed = mesh.faces.clone();
        for t in fan(&loops[0]) {
            closed.push(Face {
                v: t,
                label: 0,
                id: u32::MAX,
            });
        }
        // Every interior edge now has two faces with opposite directions.
        let mut directed: BTreeMap<(u32, u32), i32> = BTreeMap::new();
        for f in &closed {
            for k in 0..3 {
                *directed.entry((f.v[k], f.v[(k + 1) % 3])).or_default() += 1;
            }
        }
        for ((a, b), n) in &directed {
            assert_eq!(*n, 1, "edge {a}->{b} repeated: orientation broken");
        }
        assert!(border_loops(&closed, |v| !boundary.contains(&v), 64).is_empty());
    }
}
