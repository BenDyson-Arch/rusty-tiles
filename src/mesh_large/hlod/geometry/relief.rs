//! Border relief (D11 amendment A1): contraction by collapse for relieved chords. A
//! relieved chord spans many source edges, so snapping every removed vertex to its nearer
//! endpoint at once folds the border strip of noisy surfaces. Here each removed vertex
//! instead collapses onto a neighbour along its own chord, one at a time. A collapse is
//! taken only when every surviving incident face keeps its orientation and area, the edge
//! collapse keeps local topology (link condition), no face is left on border points only
//! and no edge between contract points gains a wing (the guard's seam rules; transients
//! among one chord's points are allowed), no face is duplicated and no corner loses its
//! last face. A vertex every such collapse refuses may first move a blocking interior
//! corner onto a neighbour. What remains is `Stuck`: the ordinary contraction tries it,
//! and when the whole contraction fails the cell's `Residue` goes to the last resort.
use super::{topology, Merged};
use crate::mesh_large::hlod::interfaces::{InterfaceRow, Key};
use rustc_hash::FxHashMap as HashMap;

/// Rounds of interior relief around vertices every chord collapse refuses.
const RELIEF_ROUNDS: usize = 8;

type Mapping = Option<(Key, Key, f64, u8)>;
/// A collapse's rewritten faces (index, new corners) and removed faces.
type Plan = (Vec<(u32, [u32; 3])>, Vec<u32>);

/// A relieved border vertex the chord collapse could not remove under the strict guard.
pub(super) struct Stuck {
    /// Vertex index in `Residue::merged`.
    pub vertex: u32,
    /// Its canonical key (`merged.keys[vertex]`).
    pub key: Key,
    /// Retained endpoints of its relieved chord, in source-arc order, and its source
    /// parameter along the chord (`BorderMap`'s t; t <= 0.5 is nearer `chord[0]`).
    pub chord: [Key; 2],
    pub t: f64,
}

/// The contraction of a relieved contract that the strict guard did not complete: the
/// merged children after every collapse accepted here (nothing snapped or folded), and
/// the vertices it could not remove. A pure function of children, contract and mapping.
pub(super) struct Residue {
    /// Keys, positions and ells are the merged children's; triangles are collapsed;
    /// representatives are placeholders.
    pub merged: Merged,
    /// Sorted by vertex; empty when the collapse finished but the result was refused.
    pub stuck: Vec<Stuck>,
    /// Relieved vertices removed by collapse.
    pub collapsed: u64,
}

impl Residue {
    pub(super) fn new(m: &Merged, faces: Vec<[u32; 3]>, left: &[u32], mapping: &[Mapping]) -> Self {
        let mut merged = m.clone();
        merged.triangles = faces;
        let first = merged.representative[0];
        merged.representative.clear();
        merged.representative.resize(merged.triangles.len(), first);
        let moved = mapping
            .iter()
            .enumerate()
            .filter(|(v, mapped)| {
                mapped.is_some_and(|(a, b, _, _)| a != m.keys[*v] && b != m.keys[*v])
            })
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
        Self {
            merged,
            stuck,
            collapsed: moved - left.len() as u64,
        }
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

struct State<'a> {
    m: &'a Merged,
    faces: Vec<[u32; 3]>,
    alive: Vec<bool>,
    incident: Vec<Vec<u32>>,
    border: Vec<bool>,
    required: Vec<bool>,
    /// Chords each vertex lies on (removed vertices one, endpoints those they end).
    chords: Vec<Vec<u32>>,
}

impl State<'_> {
    fn area(&self, t: [u32; 3]) -> bool {
        topology::normal(&self.m.positions, t)
            .iter()
            .any(|&n| n != 0.)
    }
    /// The faces `v -> u` rewrites and those it removes (the faces holding both), or the
    /// reason it is refused.
    fn plan(&self, v: u32, u: u32) -> Result<Plan, Refusal> {
        let (mut moved, mut removed) = (Vec::new(), Vec::new());
        let along = self.border[v as usize];
        for &f in &self.incident[v as usize] {
            if !self.alive[f as usize] {
                continue;
            }
            let t = self.faces[f as usize];
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
            if after.iter().all(|&x| self.border[x as usize]) && !self.same_chord(&after) {
                return Err(Refusal::BorderFace);
            }
            if along || self.border[u as usize] {
                for &x in &after {
                    if x != u
                        && self.border[x as usize]
                        && !self.joined(u, x)
                        && !self.same_chord(&[u, x])
                    {
                        return Err(Refusal::BorderEdge);
                    }
                }
            }
            let mut sorted = after;
            sorted.sort_unstable();
            let duplicate = self.incident[u as usize].iter().any(|&g| {
                self.alive[g as usize] && g != f && {
                    let mut s = self.faces[g as usize];
                    s.sort_unstable();
                    s == sorted
                }
            });
            if duplicate {
                return Err(Refusal::Duplicate);
            }
            moved.push((f, after));
        }
        // Link condition: the only common neighbours of `v` and `u` are the third
        // corners of the faces holding both, so the collapse keeps local topology.
        let link = |x: u32| -> Vec<u32> {
            let mut l: Vec<u32> = self.incident[x as usize]
                .iter()
                .filter(|&&g| self.alive[g as usize])
                .flat_map(|&g| self.faces[g as usize])
                .filter(|&y| y != x)
                .collect();
            l.sort_unstable();
            l.dedup();
            l
        };
        let (lv, lu) = (link(v), link(u));
        let mut thirds: Vec<u32> = removed
            .iter()
            .flat_map(|&f| self.faces[f as usize])
            .filter(|&y| y != v && y != u)
            .collect();
        thirds.sort_unstable();
        thirds.dedup();
        if lv
            .iter()
            .filter(|y| **y != u && lu.binary_search(y).is_ok())
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
                let keeps = self.incident[x as usize]
                    .iter()
                    .any(|&g| self.alive[g as usize] && !removed.contains(&g));
                if !keeps {
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
        self.incident[a as usize]
            .iter()
            .any(|&g| self.alive[g as usize] && self.faces[g as usize].contains(&b))
    }
    fn apply(&mut self, u: u32, (moved, removed): Plan) {
        for f in removed {
            self.alive[f as usize] = false;
        }
        for (f, after) in moved {
            self.faces[f as usize] = after;
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
        for &f in &self.incident[v as usize] {
            if !self.alive[f as usize] {
                continue;
            }
            let t = self.faces[f as usize];
            if t.contains(&u) {
                continue;
            }
            let after = t.map(|x| if x == v { u } else { x });
            let before_n = topology::normal(&self.m.positions, t);
            let after_n = topology::normal(&self.m.positions, after);
            let dot: f64 = (0..3).map(|i| before_n[i] * after_n[i]).sum();
            let on_border =
                after.iter().all(|&x| self.border[x as usize]) && !self.same_chord(&after);
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
                for &g in &self.incident[x as usize] {
                    if self.alive[g as usize] {
                        for y in self.faces[g as usize] {
                            if y != x && !targets.contains(&y) {
                                targets.push(y);
                            }
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
) -> Option<(Vec<[u32; 3]>, Vec<u32>)> {
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
        return Some((m.triangles.clone(), Vec::new()));
    }
    let mut border = required.clone();
    let (mut prev, mut next) = (vec![u32::MAX; n], vec![u32::MAX; n]);
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
        for &(_, v) in &run {
            border[v as usize] = true;
            on_chords[v as usize].push(c);
            prev[v as usize] = last;
            if last != a {
                next[last as usize] = v;
            }
            last = v;
            removed.push(v);
        }
        next[last as usize] = b;
    }
    let mut incident = vec![Vec::new(); n];
    for (f, t) in m.triangles.iter().enumerate() {
        for &v in t {
            incident[v as usize].push(f as u32);
        }
    }
    let mut s = State {
        m,
        faces: m.triangles.clone(),
        alive: vec![true; m.triangles.len()],
        incident,
        border,
        required,
        chords: on_chords,
    };
    let mut left: Vec<u32> = removed;
    for round in 0..=RELIEF_ROUNDS {
        loop {
            let before = left.len();
            left.retain(|&v| {
                let (p, q) = (prev[v as usize], next[v as usize]);
                let pos = |x: u32| m.positions[x as usize].map(f64::from);
                let d = |x: u32| (0..3).map(|i| (pos(v)[i] - pos(x)[i]).powi(2)).sum::<f64>();
                let mut targets = [p, q];
                if d(q) < d(p) {
                    targets.swap(0, 1);
                }
                for u in targets {
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
    Some((
        s.faces
            .into_iter()
            .zip(s.alive)
            .filter_map(|(t, alive)| alive.then_some(t))
            .collect(),
        left,
    ))
}
