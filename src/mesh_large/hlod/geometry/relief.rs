//! Border relief (D11 amendment A1): contraction by collapse for relieved chords. A
//! relieved chord spans many source edges, so snapping every removed vertex to its nearer
//! endpoint at once folds the border strip of noisy surfaces. Here each removed vertex
//! instead collapses onto a neighbour along its own chord, one at a time. A collapse is
//! taken only when every surviving incident face keeps its orientation and area, the edge
//! collapse keeps local topology (link condition), no new face or edge lies on border
//! points only (transients among one chord's points are allowed; otherwise only in its seam
//! owner under the build's `folds::Policy`, none at stage 0), no edge between contract
//! points gains a wing, no face is duplicated and no corner loses its last face. A vertex
//! every such collapse refuses may first move a blocking interior corner onto a neighbour.
//! What remains is stuck: the ordinary contraction tries it, and when the whole contraction
//! fails the cell's `Contracted` goes to the last resort's N3, which first resumes the
//! collapse under its own policy (`resume`), then snaps what is left.
use super::{folds::Policy, topology, Contracted, Merged, Stuck};
use crate::mesh_large::hlod::interfaces::{InterfaceRow, Key};
use rustc_hash::FxHashMap as HashMap;

/// Rounds of interior relief around vertices every chord collapse refuses.
const RELIEF_ROUNDS: usize = 8;
/// Collapse passes per round (a face moves at most once per pass).
const PASSES: usize = 4;

type Mapping = Option<(Key, Key, f64, u8)>;
/// A collapse's rewritten faces (index, new corners) and removed faces.
type Plan = (Vec<(u32, [u32; 3])>, Vec<u32>);

/// The hand-over of a contraction the guard did not complete (`Outcome::relief`): the
/// merged children after every collapse (nothing snapped or folded) and the relieved
/// vertices left, each with its chord. A pure function of children, contract and mapping.
/// Also returns how many relieved vertices the collapse removed.
pub(super) fn handover(
    m: &Merged,
    faces: Vec<[u32; 3]>,
    left: &[u32],
    mapping: &[Mapping],
) -> (Contracted, u64) {
    let mut merged = m.clone();
    merged.triangles = faces;
    let first = merged.representative[0];
    merged.representative.clear();
    merged.representative.resize(merged.triangles.len(), first);
    let moved = mapping
        .iter()
        .enumerate()
        .filter(|(v, mapped)| mapped.is_some_and(|(a, b, _, _)| a != m.keys[*v] && b != m.keys[*v]))
        .count() as u64;
    let mut stuck: Vec<Stuck> = left
        .iter()
        .filter_map(|&v| {
            let (a, b, t, _) = mapping[v as usize]?;
            Some(Stuck {
                vertex: v,
                key: m.keys[v as usize],
                chord: [a, b],
                t,
            })
        })
        .collect();
    stuck.sort_unstable_by_key(|s| s.vertex);
    (Contracted { merged, stuck }, moved - left.len() as u64)
}

/// N3 on a hand-over: resume the chord collapse of its stuck vertices under N3's policy,
/// where a new seam face or edge is admitted in its owner. What still remains is stuck.
pub(super) fn resume(seed: &Contracted, interface: &[InterfaceRow], policy: &Policy) -> Contracted {
    let m = &seed.merged;
    let mut mapping: Vec<Mapping> = vec![None; m.keys.len()];
    for s in &seed.stuck {
        mapping[s.vertex as usize] = Some((s.chord[0], s.chord[1], s.t, m.ells[s.vertex as usize]));
    }
    match along_border(m, interface, &mapping, policy) {
        Some((faces, left)) => handover(m, faces, &left, &mapping).0,
        None => Contracted {
            merged: m.clone(),
            stuck: seed
                .stuck
                .iter()
                .map(|s| Stuck {
                    vertex: s.vertex,
                    key: s.key,
                    chord: s.chord,
                    t: s.t,
                })
                .collect(),
        },
    }
}

#[derive(Clone, Copy)]
enum Refusal {
    Flip,
    BorderFace,
    BorderEdge,
    Duplicate,
    Orphan,
    Link,
    Wings,
}

fn edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

fn corners(mut t: [u32; 3]) -> [u32; 3] {
    t.sort_unstable();
    t
}

struct State<'a> {
    m: &'a Merged,
    /// The build's seam rule: a new face or edge on border points across chords only in
    /// its owner (none at stage 0).
    policy: &'a Policy,
    faces: Vec<[u32; 3]>,
    alive: Vec<bool>,
    /// Faces each vertex has held (dead ones are skipped).
    incident: Vec<Vec<u32>>,
    /// Alive faces per vertex, per edge and per corner set: every check reads these in
    /// O(1), so a collapse costs the star of the vertex it removes, never the star of
    /// its target (a chord endpoint gathers a fan as long as its chord).
    degree: Vec<u32>,
    edges: HashMap<(u32, u32), u32>,
    sets: HashMap<[u32; 3], u32>,
    border: Vec<bool>,
    required: Vec<bool>,
    /// Chords each vertex lies on (removed vertices one, endpoints those they end).
    chords: Vec<Vec<u32>>,
    /// Incident entries visited (the scaling control's work count).
    work: std::cell::Cell<u64>,
}

impl State<'_> {
    fn area(&self, t: [u32; 3]) -> bool {
        topology::normal(&self.m.positions, t)
            .iter()
            .any(|&n| n != 0.)
    }
    fn count(&mut self, t: [u32; 3], add: bool) {
        fn step<K: std::hash::Hash + Eq>(map: &mut HashMap<K, u32>, key: K, add: bool) {
            if add {
                *map.entry(key).or_default() += 1;
            } else if let std::collections::hash_map::Entry::Occupied(mut e) = map.entry(key) {
                *e.get_mut() -= 1;
                if *e.get() == 0 {
                    e.remove();
                }
            }
        }
        for i in 0..3 {
            let d = &mut self.degree[t[i] as usize];
            *d = if add { *d + 1 } else { *d - 1 };
            step(&mut self.edges, edge(t[i], t[(i + 1) % 3]), add);
        }
        step(&mut self.sets, corners(t), add);
    }
    /// The alive faces of `v`.
    fn star(&self, v: u32) -> impl Iterator<Item = u32> + '_ {
        self.incident[v as usize].iter().copied().filter(|&f| {
            self.work.set(self.work.get() + 1);
            self.alive[f as usize]
        })
    }
    /// The faces `v -> u` rewrites and those it removes (the faces holding both), or the
    /// reason it is refused.
    fn plan(&self, v: u32, u: u32) -> Result<Plan, Refusal> {
        let (mut moved, mut removed) = (Vec::new(), Vec::new());
        let along = self.border[v as usize];
        let mut lv = Vec::new();
        for f in self.star(v) {
            let t = self.faces[f as usize];
            lv.extend(t.into_iter().filter(|&y| y != v));
            if t.contains(&u) {
                removed.push(f);
                continue;
            }
            let after = t.map(|x| if x == v { u } else { x });
            let before_n = topology::normal(&self.m.positions, t);
            let after_n = topology::normal(&self.m.positions, after);
            let dot: f64 = (0..3).map(|i| before_n[i] * after_n[i]).sum();
            if !self.area(after) || dot <= 0. {
                return Err(Refusal::Flip);
            }
            // A face or new edge on border points only is the guard's seam rule, except
            // among one chord's points: those are transients of its own contraction.
            if after.iter().all(|&x| self.border[x as usize])
                && !self.same_chord(&after)
                && !self
                    .policy
                    .owns_face(after.map(|x| self.m.keys[x as usize]))
            {
                return Err(Refusal::BorderFace);
            }
            if along || self.border[u as usize] {
                for &x in &after {
                    if x != u
                        && self.border[x as usize]
                        && !self.joined(u, x)
                        && !self.same_chord(&[u, x])
                        && !self
                            .policy
                            .owns_segment(self.m.keys[u as usize], self.m.keys[x as usize])
                    {
                        return Err(Refusal::BorderEdge);
                    }
                }
            }
            if self.sets.contains_key(&corners(after)) {
                return Err(Refusal::Duplicate);
            }
            moved.push((f, after));
        }
        // Link condition: the only common neighbours of `v` and `u` are the third
        // corners of the faces holding both, so the collapse keeps local topology.
        lv.sort_unstable();
        lv.dedup();
        let mut thirds: Vec<u32> = removed
            .iter()
            .flat_map(|&f| self.faces[f as usize])
            .filter(|&y| y != v && y != u)
            .collect();
        thirds.sort_unstable();
        thirds.dedup();
        if lv
            .iter()
            .filter(|&&y| y != u && self.joined(u, y))
            .ne(thirds.iter())
        {
            return Err(Refusal::Link);
        }
        // An edge between two contract points keeps its wing count.
        for &(_, after) in &moved {
            for i in 0..3 {
                let (a, b) = (after[i], after[(i + 1) % 3]);
                if (a == u || b == u)
                    && self.required[a as usize]
                    && self.required[b as usize]
                    && self.joined(a, b)
                {
                    let x = if a == u { b } else { a };
                    let lost = removed
                        .iter()
                        .filter(|&&f| self.faces[f as usize].contains(&x))
                        .count();
                    if lost == 0 {
                        return Err(Refusal::Wings);
                    }
                }
            }
        }
        // Removing the faces that held both must not strip any corner, the target
        // included, of its last face (a required point would be lost; an interior one
        // would dangle). The target also keeps every face it gains from `v`.
        for &f in &removed {
            for &x in &self.faces[f as usize] {
                if x == v || (x == u && !moved.is_empty()) {
                    continue;
                }
                let lost = removed
                    .iter()
                    .filter(|&&g| self.faces[g as usize].contains(&x))
                    .count() as u32;
                if self.degree[x as usize] <= lost {
                    return Err(Refusal::Orphan);
                }
            }
        }
        Ok((moved, removed))
    }
    fn same_chord(&self, vertices: &[u32]) -> bool {
        self.chords[vertices[0] as usize].iter().any(|c| {
            vertices[1..]
                .iter()
                .all(|&x| self.chords[x as usize].contains(c))
        })
    }
    /// Whether an alive face holds both `a` and `b`.
    fn joined(&self, a: u32, b: u32) -> bool {
        self.edges.contains_key(&edge(a, b))
    }
    fn apply(&mut self, u: u32, (moved, removed): Plan) {
        for f in removed {
            self.alive[f as usize] = false;
            self.count(self.faces[f as usize], false);
        }
        for (f, after) in moved {
            self.count(self.faces[f as usize], false);
            self.faces[f as usize] = after;
            self.count(after, true);
            self.incident[u as usize].push(f);
        }
    }
    /// Move one interior corner of a face blocking `v -> u` onto a neighbour, preferring
    /// interior ones.
    fn relieve(&mut self, v: u32, u: u32) -> bool {
        if u == u32::MAX {
            return false;
        }
        // Only the faces `v -> u` would flip or leave on border points block it.
        let mut blocking = Vec::new();
        for f in self.star(v) {
            let t = self.faces[f as usize];
            if t.contains(&u) {
                continue;
            }
            let after = t.map(|x| if x == v { u } else { x });
            let before_n = topology::normal(&self.m.positions, t);
            let after_n = topology::normal(&self.m.positions, after);
            let dot: f64 = (0..3).map(|i| before_n[i] * after_n[i]).sum();
            let on_border = after.iter().all(|&x| self.border[x as usize])
                && !self.same_chord(&after)
                && !self
                    .policy
                    .owns_face(after.map(|x| self.m.keys[x as usize]));
            if !self.area(after) || dot <= 0. || on_border {
                blocking.push(f);
            }
        }
        for f in blocking {
            for x in self.faces[f as usize] {
                if self.border[x as usize] || self.required[x as usize] {
                    continue;
                }
                let mut targets: Vec<u32> = Vec::new();
                for g in self.star(x) {
                    for y in self.faces[g as usize] {
                        if y != x && !targets.contains(&y) {
                            targets.push(y);
                        }
                    }
                }
                let p = self.m.positions[x as usize];
                let d = |y: &u32| {
                    let q = self.m.positions[*y as usize];
                    (0..3)
                        .map(|i| (f64::from(p[i]) - f64::from(q[i])).powi(2))
                        .sum::<f64>()
                };
                // Interior targets first; a border target passes the same seam checks.
                targets.sort_by(|a, b| {
                    self.border[*a as usize]
                        .cmp(&self.border[*b as usize])
                        .then(d(a).total_cmp(&d(b)))
                        .then(a.cmp(b))
                });
                for y in targets {
                    if let Ok(plan) = self.plan(x, y) {
                        self.apply(y, plan);
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Contract every vertex `mapping` moves onto its chord by collapses along that chord.
/// Returns the faces and the moved vertices no collapse could remove, or `None` when a
/// chord endpoint is absent from the merged children.
pub(super) fn along_border(
    m: &Merged,
    interface: &[InterfaceRow],
    mapping: &[Mapping],
    policy: &Policy,
) -> Option<(Vec<[u32; 3]>, Vec<u32>)> {
    collapse(m, interface, mapping, policy).map(|(faces, left, _)| (faces, left))
}

/// `along_border`, with the incident entries it visited. Each removed vertex collapses
/// onto its surviving chord neighbour toward its nearer endpoint (the half of a chord
/// with t <= 0.5 in ascending t, then the other half in descending t), or else onto its
/// other neighbour. A target is never a vertex this pass has still to visit, so a face
/// moves at most once per pass and every pass costs the stars of the vertices it
/// removes: no fan travels along a chord. At most `PASSES` passes per round.
fn collapse(
    m: &Merged,
    interface: &[InterfaceRow],
    mapping: &[Mapping],
    policy: &Policy,
) -> Option<(Vec<[u32; 3]>, Vec<u32>, u64)> {
    let ids: HashMap<Key, u32> = m
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u32))
        .collect();
    let n = m.keys.len();
    let mut required = vec![false; n];
    for r in interface.iter().filter(|r| r.is_vertex()) {
        if let Some(&v) = ids.get(&r.a) {
            required[v as usize] = true;
        }
    }
    // Chords: removed vertices in source order between their two retained endpoints.
    let mut chords: HashMap<(u32, u32), Vec<(f64, u32)>> = HashMap::default();
    for (v, mapped) in mapping.iter().enumerate() {
        let Some((a, b, t, _)) = *mapped else {
            continue;
        };
        if a == m.keys[v] || b == m.keys[v] {
            continue;
        }
        let (&a, &b) = (ids.get(&a)?, ids.get(&b)?);
        chords.entry((a, b)).or_default().push((t, v as u32));
    }
    if chords.is_empty() {
        return Some((m.triangles.clone(), Vec::new(), 0));
    }
    let mut border = required.clone();
    let (mut prev, mut next) = (vec![u32::MAX; n], vec![u32::MAX; n]);
    let mut toward_b = vec![false; n];
    let mut order: Vec<_> = chords.into_iter().collect();
    order.sort_unstable_by_key(|(k, _)| *k);
    let mut removed = Vec::new();
    let mut on_chords: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (c, ((a, b), mut run)) in order.into_iter().enumerate() {
        let c = c as u32;
        on_chords[a as usize].push(c);
        on_chords[b as usize].push(c);
        run.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        let mut last = a;
        for &(t, v) in &run {
            border[v as usize] = true;
            on_chords[v as usize].push(c);
            toward_b[v as usize] = t > 0.5;
            prev[v as usize] = last;
            if last != a {
                next[last as usize] = v;
            }
            last = v;
        }
        next[last as usize] = b;
        // Outside in: each half is visited from its endpoint.
        let half = run.partition_point(|&(t, _)| t <= 0.5);
        removed.extend(run[..half].iter().map(|&(_, v)| v));
        removed.extend(run[half..].iter().rev().map(|&(_, v)| v));
    }
    let mut incident = vec![Vec::new(); n];
    for (f, t) in m.triangles.iter().enumerate() {
        for &v in t {
            incident[v as usize].push(f as u32);
        }
    }
    let mut s = State {
        m,
        policy,
        faces: m.triangles.clone(),
        alive: vec![true; m.triangles.len()],
        incident,
        degree: vec![0; n],
        edges: HashMap::default(),
        sets: HashMap::default(),
        border,
        required,
        chords: on_chords,
        work: std::cell::Cell::new(0),
    };
    for &t in &m.triangles {
        s.count(t, true);
    }
    let mut left: Vec<u32> = removed;
    let mut pending = vec![false; n];
    for round in 0..=RELIEF_ROUNDS {
        for _ in 0..PASSES {
            let before = left.len();
            for &v in &left {
                pending[v as usize] = true;
            }
            left.retain(|&v| {
                pending[v as usize] = false;
                let (p, q) = (prev[v as usize], next[v as usize]);
                let targets = if toward_b[v as usize] { [q, p] } else { [p, q] };
                for u in targets {
                    if u == u32::MAX || pending[u as usize] {
                        continue;
                    }
                    if let Ok(plan) = s.plan(v, u) {
                        s.apply(u, plan);
                        s.border[v as usize] = false;
                        // Unlink v from its chord.
                        if p != u32::MAX && next[p as usize] == v {
                            next[p as usize] = q;
                        }
                        if q != u32::MAX && prev[q as usize] == v {
                            prev[q as usize] = p;
                        }
                        return false;
                    }
                }
                true
            });
            if left.is_empty() || left.len() == before {
                break;
            }
        }
        if left.is_empty() || round == RELIEF_ROUNDS {
            break;
        }
        let mut relieved = false;
        for &v in &left {
            let (p, q) = (prev[v as usize], next[v as usize]);
            relieved |= s.relieve(v, p) || s.relieve(v, q);
        }
        if !relieved {
            break;
        }
    }
    let work = s.work.get();
    Some((
        s.faces
            .into_iter()
            .zip(s.alive)
            .filter_map(|(t, alive)| alive.then_some(t))
            .collect(),
        left,
        work,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::hlod::store::SourcePrimitive;

    /// A planar strip of k quads and two rows: its bottom row is one chord P0..Pk (both
    /// retained) whose k - 1 interior vertices are removed.
    fn strip(k: u32) -> (Merged, Vec<InterfaceRow>, Vec<Mapping>) {
        let w = k + 1;
        let positions: Vec<[f32; 3]> = (0..3)
            .flat_map(|y| (0..=k).map(move |x| [x as f32, y as f32, 0.]))
            .collect();
        let mut triangles = Vec::new();
        for y in 0..2 {
            for x in 0..k {
                let a = y * w + x;
                triangles.push([a, a + 1, a + w]);
                triangles.push([a + 1, a + w + 1, a + w]);
            }
        }
        let keys: Vec<Key> = positions.iter().map(|p| p.map(f32::to_bits)).collect();
        let (a, b) = (keys[0], keys[k as usize]);
        let interface = vec![
            InterfaceRow::new(0, a, a, 0, 2),
            InterfaceRow::new(0, b, b, 0, 2),
            InterfaceRow::new(0, a.min(b), a.max(b), 0, 2),
        ];
        let mut mapping: Vec<Mapping> = vec![None; positions.len()];
        for x in 1..k {
            mapping[x as usize] = Some((a, b, f64::from(x) / f64::from(k), 0));
        }
        let first = SourcePrimitive {
            node: 0,
            mesh: 0,
            primitive: 0,
            expanded_ordinal: 0,
            material: u32::MAX,
        };
        let merged = Merged {
            keys,
            ells: vec![0; positions.len()],
            positions,
            representative: vec![first; triangles.len()],
            triangles,
        };
        (merged, interface, mapping)
    }

    /// Review 1 (P2): a long chord's collapses once marched toward one endpoint, each
    /// rewriting the growing fan (k = 8,000 took 2.6 s). Now the work per removed vertex
    /// (incident entries visited) stays flat from k = 1,000 to 32,000, and the chord still
    /// collapses completely: one face per removed vertex, a fan at each endpoint.
    #[test]
    fn chord_collapse_work_grows_linearly_with_chord_length() {
        let mut first: Option<f64> = None;
        for k in [1_000u32, 2_000, 8_000, 32_000] {
            let (m, interface, mapping) = strip(k);
            let started = std::time::Instant::now();
            let (faces, left, work) =
                collapse(&m, &interface, &mapping, &Policy::default()).unwrap();
            eprintln!(
                "RELIEF COLLAPSE k={k} work={work} secs={:.4}",
                started.elapsed().as_secs_f64()
            );
            assert!(left.is_empty(), "k={k}: {} left", left.len());
            assert_eq!(faces.len(), m.triangles.len() - (k as usize - 1));
            let per = work as f64 / f64::from(k);
            let first = *first.get_or_insert(per);
            assert!(
                per <= 1.05 * first,
                "k={k}: {per} visits per vertex, {first} at 1,000"
            );
        }
    }

    /// The children's interface along the strip's border row (every vertex and edge of
    /// row 0), and the guard N3 checks a hand-over with: the strip's own contraction
    /// licences under N3's policy from the contract's holders.
    fn n3_guard(
        m: &Merged,
        interface: &[InterfaceRow],
        previous: &[InterfaceRow],
        mapping: &[Mapping],
    ) -> super::super::folds::Source {
        let holders = crate::mesh_large::hlod::borders::holders_from_rows(interface);
        super::super::folds::Source::for_contraction(
            &m.positions,
            &m.triangles,
            interface,
            previous,
            mapping,
        )
        .with_policy(Policy::last_resort(3, 0, 1, &holders))
    }

    fn row_interface(m: &Merged, row: &[u32]) -> Vec<InterfaceRow> {
        let mut rows: Vec<InterfaceRow> = row
            .iter()
            .map(|&v| InterfaceRow::new(0, m.keys[v as usize], m.keys[v as usize], 0, 2))
            .collect();
        for w in row.windows(2) {
            let (a, b) = (m.keys[w[0] as usize], m.keys[w[1] as usize]);
            rows.push(InterfaceRow::new(0, a.min(b), a.max(b), 0, 2));
        }
        crate::mesh_large::hlod::interfaces::merge_rows(&mut rows);
        rows
    }

    /// A chord collapsed completely hands N3 an empty stuck list (the guard refused the
    /// result, not a vertex): `resume` leaves it as it is, and N3's snap publishes the
    /// collapsed faces unchanged.
    #[test]
    fn an_empty_stuck_list_hands_over_the_collapsed_faces() {
        let k = 8;
        let (m, interface, mapping) = strip(k);
        let (faces, left) = along_border(&m, &interface, &mapping, &Policy::default()).unwrap();
        assert!(left.is_empty());
        let (seed, collapsed) = handover(&m, faces.clone(), &left, &mapping);
        assert!(seed.stuck.is_empty());
        assert_eq!(collapsed, u64::from(k) - 1);
        let previous = row_interface(&m, &(0..=k).collect::<Vec<_>>());
        let guard = n3_guard(&m, &interface, &previous, &mapping);
        let holders = crate::mesh_large::hlod::borders::holders_from_rows(&interface);
        let resumed = resume(&seed, &interface, &Policy::last_resort(3, 0, 1, &holders));
        assert!(resumed.stuck.is_empty());
        assert_eq!(resumed.merged.triangles, faces);
        let snapped = super::super::snap_stuck(&m, &resumed, &interface, &guard).unwrap();
        assert_eq!(snapped.triangles, faces);
    }

    /// A stuck vertex v on chord a-b nearer a, where the faces holding b are only
    /// (a, b, v): snapping v to a degenerates the chord's only face, so N3 refuses (the
    /// cell is omitted and reported). Resuming the collapse first moves v onto b instead,
    /// which keeps the chord, and N3 then publishes it.
    #[test]
    fn a_snap_that_removes_a_chords_only_face_is_refused() {
        let positions = vec![
            [0., 0., 0.],  // a
            [2., 0., 0.],  // b
            [1., 0.3, 0.], // v
            [0.5, 1., 0.], // x
            [1.5, 1., 0.], // y
        ];
        let keys: Vec<Key> = positions
            .iter()
            .map(|p: &[f32; 3]| p.map(f32::to_bits))
            .collect();
        let triangles = vec![[0, 1, 2], [0, 2, 3], [2, 4, 3]];
        let first = crate::mesh_large::hlod::store::SourcePrimitive {
            node: 0,
            mesh: 0,
            primitive: 0,
            expanded_ordinal: 0,
            material: u32::MAX,
        };
        let m = Merged {
            ells: vec![0; keys.len()],
            keys: keys.clone(),
            positions,
            representative: vec![first; triangles.len()],
            triangles,
        };
        let (a, b) = (keys[0], keys[1]);
        let interface = vec![
            InterfaceRow::new(0, a, a, 0, 2),
            InterfaceRow::new(0, b, b, 0, 2),
            InterfaceRow::new(0, a.min(b), a.max(b), 0, 2),
        ];
        let previous = row_interface(&m, &[0, 2, 1]);
        let mut mapping: Vec<Mapping> = vec![None; keys.len()];
        mapping[2] = Some((a, b, 0.4, 0));
        let seed = Contracted {
            merged: m.clone(),
            stuck: vec![Stuck {
                vertex: 2,
                key: keys[2],
                chord: [a, b],
                t: 0.4,
            }],
        };
        let guard = n3_guard(&m, &interface, &previous, &mapping);
        assert!(super::super::snap_stuck(&m, &seed, &interface, &guard).is_none());
        let resumed = resume(&seed, &interface, &Policy::default());
        assert!(resumed.stuck.is_empty());
        let snapped = super::super::snap_stuck(&m, &resumed, &interface, &guard).unwrap();
        assert!(snapped
            .triangles
            .iter()
            .any(|t| t.contains(&0) && t.contains(&1)));
    }
}
