//! Bounded, tile-local boundary reconstruction and topology cleanup. The source graph
//! supplies the boundary; this module never chooses or moves a contracted endpoint.
use super::{missing, oriented, Merged};
use crate::{
    mesh_large::hlod::interfaces::{InterfaceRow, Key},
    JobError,
};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub contracted_vertices: u64,
    pub reconstructed_faces: u64,
    pub duplicate_faces: u64,
    pub dropped_components: u64,
    pub closed_holes: u64,
    pub opened_holes: u64,
    pub remeshed_vertices: u64,
    pub remesh_rounds: u64,
    pub constrained_remeshes: u64,
    pub rejected_remesh_rounds: u64,
    pub rejected_hole_closures: u64,
    pub max_dropped_component_diameter: f64,
    pub hole_patch_to_source: f64,
    pub hole_boundary_to_patch: f64,
    pub dropped_component_to_proxy: f64,
    pub hole_g_increase: f64,
    pub dropped_component_g_increase: f64,
}
impl Receipt {
    pub(crate) fn add(&mut self, r: Self) {
        self.contracted_vertices += r.contracted_vertices;
        self.reconstructed_faces += r.reconstructed_faces;
        self.duplicate_faces += r.duplicate_faces;
        self.dropped_components += r.dropped_components;
        self.closed_holes += r.closed_holes;
        self.opened_holes += r.opened_holes;
        self.remeshed_vertices += r.remeshed_vertices;
        self.remesh_rounds += r.remesh_rounds;
        self.constrained_remeshes += r.constrained_remeshes;
        self.rejected_remesh_rounds += r.rejected_remesh_rounds;
        self.rejected_hole_closures += r.rejected_hole_closures;
        self.max_dropped_component_diameter = self
            .max_dropped_component_diameter
            .max(r.max_dropped_component_diameter);
        self.hole_patch_to_source = self.hole_patch_to_source.max(r.hole_patch_to_source);
        self.hole_boundary_to_patch = self.hole_boundary_to_patch.max(r.hole_boundary_to_patch);
        self.dropped_component_to_proxy = self
            .dropped_component_to_proxy
            .max(r.dropped_component_to_proxy);
        self.hole_g_increase = self.hole_g_increase.max(r.hole_g_increase);
        self.dropped_component_g_increase = self
            .dropped_component_g_increase
            .max(r.dropped_component_g_increase);
    }
}

/// Bounded witnesses of an actual topology proposal. Faces index the proposed output;
/// boundary and removed-component positions belong to the input before cleanup.
#[derive(Debug, Default)]
pub(crate) struct Change {
    pub receipt: Receipt,
    pub patch_faces: Vec<usize>,
    pub boundary: Vec<[[f32; 3]; 2]>,
    pub dropped: Vec<[[f32; 3]; 3]>,
}

struct Components(Vec<usize>);
impl Components {
    fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    fn root(&mut self, mut a: usize) -> usize {
        while self.0[a] != a {
            self.0[a] = self.0[self.0[a]];
            a = self.0[a];
        }
        a
    }
    fn join(&mut self, a: u32, b: u32) {
        let (a, b) = (self.root(a as usize), self.root(b as usize));
        self.0[a.max(b)] = a.min(b);
    }
    fn mesh(m: &Merged) -> Vec<usize> {
        let mut c = Self::new(m.keys.len());
        for t in &m.triangles {
            c.join(t[0], t[1]);
            c.join(t[0], t[2]);
        }
        (0..m.keys.len()).map(|v| c.root(v)).collect()
    }
}
pub(super) fn components(m: &Merged) -> Vec<usize> {
    Components::mesh(m)
}
fn points(interface: &[InterfaceRow]) -> HashSet<Key> {
    interface.iter().flat_map(|r| [r.a, r.b]).collect()
}
fn surface(t: &[u32; 3]) -> bool {
    t[0] != t[1] && t[1] != t[2] && t[0] != t[2]
}
pub(super) fn normal(p: &[[f32; 3]], t: [u32; 3]) -> [f64; 3] {
    let t = t.map(|v| p[v as usize].map(f64::from));
    let a: [f64; 3] = std::array::from_fn(|i| t[1][i] - t[0][i]);
    let b: [f64; 3] = std::array::from_fn(|i| t[2][i] - t[0][i]);
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn mapped(p: &[[f32; 3]], t: [u32; 3], map: &[u32]) -> [u32; 3] {
    let mut out = t.map(|v| map[v as usize]);
    let (a, b) = (normal(p, t), normal(p, out));
    if (0..3).map(|i| a[i] * b[i]).sum::<f64>() < 0. {
        out.swap(1, 2);
    }
    out
}
fn retain(m: &mut Merged, keep: &[bool]) {
    let mut i = 0;
    m.triangles.retain(|_| (keep[i], i += 1).0);
    let mut i = 0;
    m.representative.retain(|_| (keep[i], i += 1).0);
}

/// A source chain is mapped monotonically onto its canonical contracted endpoints. A
/// threshold at the source midpoint collapses successive edge strips onto one chord.
/// Endpoints are nested across levels, hence already present in the capped children.
/// Degenerate incident strips may be retriangulated; the delivered edge is never split.
pub(super) fn contract(
    m: &mut Merged,
    interface: &[InterfaceRow],
    mapping: &super::BorderMap<'_>,
    receipt: &mut Receipt,
) -> Result<(), JobError> {
    let mut ids: HashMap<Key, u32> = m
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u32))
        .collect();
    let mut map: Vec<u32> = (0..m.keys.len() as u32).collect();
    for (v, target) in map.iter_mut().enumerate() {
        if let Some((a, b, t, ell)) = mapping(m.keys[v])? {
            let k = if t <= 0.5 { a } else { b };
            let next = m.keys.len() as u32;
            let id = *ids.entry(k).or_insert(next);
            if id == next {
                m.keys.push(k);
                m.positions.push(k.map(f32::from_bits));
                m.ells.push(ell);
            }
            receipt.contracted_vertices += u64::from(id != v as u32);
            *target = id;
        }
    }
    if receipt.contracted_vertices == 0 {
        return Ok(());
    }
    let component = Components::mesh(m);
    let original = m.triangles.clone();
    for t in &mut m.triangles {
        *t = mapped(&m.positions, *t, &map);
    }
    let original_representatives = m.representative.clone();
    // Remove collapsed strips before checking the delivered arcs. A degenerate strip
    // can mention an edge which disappears when another face keeps its component alive.
    let mut alive = HashSet::default();
    for (i, t) in m.triangles.iter().enumerate() {
        if surface(t) {
            alive.insert(component[original[i][0] as usize]);
        }
    }
    let mut kept = HashSet::default();
    let keep: Vec<bool> = m
        .triangles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let c = component[original[i][0] as usize];
            surface(t) || (!alive.contains(&c) && kept.insert(c))
        })
        .collect();
    retain(m, &keep);
    let required = points(interface);
    let segments = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .map(|r| (r.a, r.b))
        .collect();
    let (lost_edges, lost_points) = missing(&m.triangles, &m.keys, &segments, &required);
    if lost_edges.is_empty() && lost_points.is_empty() {
        return Ok(());
    }
    let mut candidates = HashMap::default();
    let mut witnesses: HashMap<usize, Vec<u32>> = HashMap::default();
    for (i, t) in original.iter().enumerate() {
        let witness = witnesses.entry(component[t[0] as usize]).or_default();
        for &v in t {
            candidates.entry(map[v as usize]).or_insert(i);
            // Three distinct witnesses suffice to choose support distinct from two ends.
            if witness.len() < 3 && !witness.contains(&v) {
                witness.push(v);
            }
        }
    }
    let repaired_points: HashSet<Key> = lost_edges.iter().flat_map(|(a, b)| [*a, *b]).collect();
    for (a, b) in lost_edges.into_iter().chain(
        lost_points
            .into_iter()
            .filter(|p| !repaired_points.contains(p))
            .map(|p| (p, p)),
    ) {
        let (a, b) = (ids[&a], ids[&b]);
        let i = candidates
            .get(&a)
            .or_else(|| candidates.get(&b))
            .copied()
            .ok_or_else(|| super::state("contracted border endpoint has no source component"))?;
        let root = component[original[i][0] as usize];
        let support = &witnesses[&root];
        let c = support
            .iter()
            .copied()
            .find(|v| *v != a && *v != b)
            .unwrap_or(a);
        let b = if a == b {
            support
                .iter()
                .copied()
                .find(|v| *v != a && *v != c)
                .unwrap_or(b)
        } else {
            b
        };
        let mut face = [a, b, c];
        let (old, new) = (
            normal(&m.positions, original[i]),
            normal(&m.positions, face),
        );
        if (0..3).map(|k| old[k] * new[k]).sum::<f64>() < 0. {
            face.swap(1, 2);
        }
        m.triangles.push(face);
        m.representative.push(original_representatives[i]);
        receipt.reconstructed_faces += 1;
    }
    Ok(())
}

/// Duplicated oriented soup adds no surface. Only components with NO border contact and
/// strict diameter bound below the known G floor can be dropped. Keep one component if
/// every component is eligible, so the parent still has geometry to measure against.
pub(super) fn cleanup(m: &mut Merged, interface: &[InterfaceRow], g: f64, r: &mut Receipt) {
    let mut seen = HashSet::default();
    let keep: Vec<bool> = m
        .triangles
        .iter()
        .map(|t| seen.insert(oriented(t)))
        .collect();
    r.duplicate_faces += keep.iter().filter(|k| !**k).count() as u64;
    retain(m, &keep);
    let component = Components::mesh(m);
    let border = points(interface);
    let mut boxes: HashMap<usize, ([f64; 3], [f64; 3], bool)> = HashMap::default();
    for t in &m.triangles {
        for &v in t {
            let p = m.positions[v as usize].map(f64::from);
            let b = boxes.entry(component[v as usize]).or_insert((p, p, false));
            for (a, x) in p.into_iter().enumerate() {
                b.0[a] = b.0[a].min(x);
                b.1[a] = b.1[a].max(x);
            }
            b.2 |= border.contains(&m.keys[v as usize]);
        }
    }
    let mut dropped: HashSet<usize> = boxes
        .iter()
        .filter(|(_, (lo, hi, touch))| {
            !touch && (0..3).map(|a| (hi[a] - lo[a]).powi(2)).sum::<f64>().sqrt() < g
        })
        .map(|(c, _)| *c)
        .collect();
    if dropped.len() == boxes.len() {
        if let Some(c) = dropped.iter().min().copied() {
            dropped.remove(&c);
        }
    }
    r.dropped_components += dropped.len() as u64;
    let keep = m
        .triangles
        .iter()
        .map(|t| !dropped.contains(&component[t[0] as usize]))
        .collect::<Vec<_>>();
    retain(m, &keep);
}

/// Compare the actual output topology, including changes made by meshopt itself. A
/// boundary cycle touching the interface must survive as a distinct boundary cycle with
/// its border contacts. Interior component loss is eligible only below the existing G floor.
/// This is checked after simplification and before S2; it never substitutes for S2.
pub(super) fn changes(
    m: &Merged,
    output: &[[u32; 3]],
    interface: &[InterfaceRow],
    g: f64,
) -> Option<Change> {
    if output == m.triangles {
        return Some(Change::default());
    }
    let rejected = |reason: &str| {
        #[cfg(test)]
        if std::env::var_os("RT_HLOD_REPLAY_DIR").is_some() {
            eprintln!("TOPOLOGY: {reason}");
        }
        let _ = reason;
        None
    };
    let component = Components::mesh(m);
    let border = points(interface);
    let mut bounds: HashMap<usize, ([f64; 3], [f64; 3], bool)> = HashMap::default();
    for t in &m.triangles {
        for &v in t {
            let p = m.positions[v as usize].map(f64::from);
            let b = bounds.entry(component[v as usize]).or_insert((p, p, false));
            for (a, coordinate) in p.into_iter().enumerate() {
                b.0[a] = b.0[a].min(coordinate);
                b.1[a] = b.1[a].max(coordinate);
            }
            b.2 |= border.contains(&m.keys[v as usize]);
        }
    }
    let surviving: HashSet<usize> = output
        .iter()
        .flatten()
        .map(|v| component[*v as usize])
        .collect();
    let mut receipt = Receipt::default();
    let mut dropped = HashSet::default();
    for (c, (lo, hi, touch)) in &bounds {
        if !surviving.contains(c) {
            if *touch {
                return rejected("border component disappeared");
            }
            let diameter = (0..3).map(|a| (hi[a] - lo[a]).powi(2)).sum::<f64>().sqrt();
            if diameter >= g {
                return rejected("interior component is not below the existing G floor");
            }
            dropped.insert(*c);
            receipt.dropped_components += 1;
            receipt.max_dropped_component_diameter =
                receipt.max_dropped_component_diameter.max(diameter);
        }
    }
    let boundary = |triangles: &[[u32; 3]]| {
        let mut edges: HashMap<(u32, u32), u32> = HashMap::default();
        for t in triangles {
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                if a != b {
                    *edges.entry((a.min(b), a.max(b))).or_default() += 1;
                }
            }
        }
        let mut groups = Components::new(m.keys.len());
        let mut degree = vec![0u32; m.keys.len()];
        for (&(a, b), &n) in &edges {
            if n == 1 {
                groups.join(a, b);
                degree[a as usize] += 1;
                degree[b as usize] += 1;
            }
        }
        let root: Vec<usize> = (0..m.keys.len()).map(|v| groups.root(v)).collect();
        (edges, degree, root)
    };
    let (old_edges, old_degree, old_root) = boundary(&m.triangles);
    let (new_edges, new_degree, new_root) = boundary(output);
    let mut touching = HashSet::default();
    for v in 0..m.keys.len() {
        if old_degree[v] > 0 && border.contains(&m.keys[v]) {
            touching.insert(old_root[v]);
            if new_degree[v] == 0 {
                return rejected("border boundary vertex became interior");
            }
        }
    }
    // A boundary component can branch at a nonmanifold pinch. Preserve its graph up
    // to subdivision, not every source-density edge: degree-two vertices may disappear.
    // Every branch and every border contact is an anchor. Paths between anchors (including
    // loops returning to one anchor) are a multigraph; equality preserves touching holes.
    let anchors: HashSet<u32> = (0..m.keys.len())
        .filter(|&v| {
            old_degree[v] > 0
                && touching.contains(&old_root[v])
                && (old_degree[v] != 2 || border.contains(&m.keys[v]))
        })
        .map(|v| v as u32)
        .collect();
    let signature = |edges: &HashMap<(u32, u32), u32>| -> Option<HashMap<(u32, u32), usize>> {
        let mut adjacent = vec![Vec::new(); m.keys.len()];
        for (&(a, b), &n) in edges {
            if n == 1 {
                adjacent[a as usize].push(b);
                adjacent[b as usize].push(a);
            }
        }
        let mut visited = HashSet::default();
        let mut paths = HashMap::default();
        for &a in &anchors {
            if adjacent[a as usize].is_empty() {
                return None;
            }
            for &next in &adjacent[a as usize] {
                if !visited.insert((a.min(next), a.max(next))) {
                    continue;
                }
                let (mut previous, mut at) = (a, next);
                while !anchors.contains(&at) {
                    let neighbors = &adjacent[at as usize];
                    if neighbors.len() != 2 {
                        return None;
                    }
                    let next = if neighbors[0] == previous {
                        neighbors[1]
                    } else {
                        neighbors[0]
                    };
                    if !visited.insert((at.min(next), at.max(next))) {
                        return None;
                    }
                    previous = at;
                    at = next;
                }
                *paths.entry((a.min(at), a.max(at))).or_default() += 1;
            }
        }
        Some(paths)
    };
    let old_signature = signature(&old_edges)?;
    if signature(&new_edges).as_ref() != Some(&old_signature) {
        return rejected("touching boundary graph changed after suppressing degree-two vertices");
    }
    let cycles = |edges: &HashMap<(u32, u32), u32>, degree: &[u32], roots: &[usize]| {
        let mut graph_counts: HashMap<usize, (usize, usize)> = HashMap::default();
        for (&(a, _), &n) in edges {
            if n == 1 {
                graph_counts.entry(roots[a as usize]).or_default().0 += 1;
            }
        }
        for (v, &n) in degree.iter().enumerate() {
            if n > 0 {
                graph_counts.entry(roots[v]).or_default().1 += 1;
            }
        }
        let mut per_component: HashMap<usize, usize> = HashMap::default();
        for (root, (edges, vertices)) in graph_counts {
            if surviving.contains(&component[root]) {
                *per_component.entry(component[root]).or_default() += edges + 1 - vertices;
            }
        }
        per_component
    };
    let before_counts = cycles(&old_edges, &old_degree, &old_root);
    let after_counts = cycles(&new_edges, &new_degree, &new_root);
    let (closed, closed_count, opened_count) = changed_cycles(
        &old_edges,
        &new_edges,
        &old_degree,
        &new_degree,
        &old_root,
        &new_root,
        &component,
        &surviving,
    );
    receipt.closed_holes = closed_count as u64;
    receipt.opened_holes = opened_count as u64;
    debug_assert_eq!(
        before_counts.values().sum::<usize>() + opened_count,
        after_counts.values().sum::<usize>() + closed_count
    );
    let affected: HashSet<usize> = closed.iter().map(|&r| component[r]).collect();
    let old_faces: HashSet<_> = m.triangles.iter().map(oriented).collect();
    let patch_faces = if receipt.closed_holes > 0 {
        output
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                affected.contains(&component[t[0] as usize]) && !old_faces.contains(&oriented(t))
            })
            .map(|(i, _)| i)
            .collect()
    } else {
        Vec::new()
    };
    let boundary = if receipt.closed_holes > 0 {
        let mut edges: Vec<_> = old_edges
            .iter()
            .filter(|((a, _), n)| **n == 1 && closed.contains(&old_root[*a as usize]))
            .map(|(&(a, b), _)| (a, b))
            .collect();
        edges.sort_unstable();
        edges
            .into_iter()
            .map(|(a, b)| [m.positions[a as usize], m.positions[b as usize]])
            .collect()
    } else {
        Vec::new()
    };
    let dropped = m
        .triangles
        .iter()
        .filter(|t| dropped.contains(&component[t[0] as usize]))
        .map(|t| t.map(|v| m.positions[v as usize]))
        .collect();
    Some(Change {
        receipt,
        patch_faces,
        boundary,
        dropped,
    })
}

/// Common boundary paths are compared after suppressing unshared degree-two vertices.
/// Cycle rank of their intersection distinguishes closure from simultaneous opening;
/// subtracting only whole-component ranks would silently cancel those two operations.
fn changed_cycles(
    old: &HashMap<(u32, u32), u32>,
    new: &HashMap<(u32, u32), u32>,
    old_degree: &[u32],
    new_degree: &[u32],
    old_root: &[usize],
    new_root: &[usize],
    component: &[usize],
    surviving: &HashSet<usize>,
) -> (HashSet<usize>, usize, usize) {
    let n = old_degree.len();
    let mut anchors = vec![false; n];
    let mut contacts = HashMap::<(usize, usize), usize>::default();
    for v in 0..n {
        let shared = old_degree[v] > 0 && new_degree[v] > 0;
        anchors[v] = shared
            || (old_degree[v] > 0 && old_degree[v] != 2)
            || (new_degree[v] > 0 && new_degree[v] != 2);
        if shared {
            *contacts.entry((old_root[v], new_root[v])).or_default() += 1;
        }
    }
    // Anchor isolated cycles too. Distinct entirely replaced cycles have no shared key.
    let mut old_anchored = HashSet::default();
    let mut new_anchored = HashSet::default();
    for (v, &anchor) in anchors.iter().enumerate() {
        if anchor && old_degree[v] > 0 {
            old_anchored.insert(old_root[v]);
        }
        if anchor && new_degree[v] > 0 {
            new_anchored.insert(new_root[v]);
        }
    }
    for v in 0..n {
        if old_degree[v] > 0 && !old_anchored.contains(&old_root[v]) {
            anchors[old_root[v]] = true;
        }
        if new_degree[v] > 0 && !new_anchored.contains(&new_root[v]) {
            anchors[new_root[v]] = true;
        }
    }
    let paths = |edges: &HashMap<(u32, u32), u32>| {
        let mut adjacent = vec![Vec::new(); n];
        for (&(a, b), &count) in edges {
            if count == 1 {
                adjacent[a as usize].push(b);
                adjacent[b as usize].push(a);
            }
        }
        let mut visited = HashSet::default();
        let mut paths = HashMap::<(u32, u32), usize>::default();
        for (v, &anchor) in anchors.iter().enumerate() {
            if !anchor {
                continue;
            }
            let a = v as u32;
            for &b in &adjacent[v] {
                if !visited.insert((a.min(b), a.max(b))) {
                    continue;
                }
                let (mut previous, mut at) = (a, b);
                while !anchors[at as usize] {
                    let next = adjacent[at as usize]
                        .iter()
                        .copied()
                        .find(|&v| v != previous)
                        .unwrap();
                    visited.insert((at.min(next), at.max(next)));
                    previous = at;
                    at = next;
                }
                *paths.entry((a.min(at), a.max(at))).or_default() += 1;
            }
        }
        paths
    };
    let (old_paths, new_paths) = (paths(old), paths(new));
    let common: HashMap<_, _> = old_paths
        .iter()
        .filter_map(|(&(a, b), &count)| {
            let next = new_paths.get(&(a, b)).copied().unwrap_or(0);
            // Fewer than three retained contacts cannot establish a surviving closed loop.
            let contacts = contacts
                .get(&(old_root[a as usize], new_root[a as usize]))
                .copied()
                .unwrap_or(0);
            (next > 0 && contacts >= 3).then_some(((a, b), count.min(next)))
        })
        .collect();
    let ranks = |paths: &HashMap<(u32, u32), usize>, roots: &[usize]| {
        let mut groups = Components::new(n);
        for &(a, b) in paths.keys() {
            groups.join(a, b);
        }
        let mut vertices = HashSet::default();
        let mut counts = HashMap::<usize, (usize, usize)>::default();
        for (&(a, b), &count) in paths {
            let r = groups.root(a as usize);
            counts.entry(r).or_default().0 += count;
            for v in [a, b] {
                if vertices.insert(v) {
                    counts.entry(r).or_default().1 += 1;
                }
            }
        }
        let mut result = HashMap::<usize, usize>::default();
        for (r, (edges, vertices)) in counts {
            if surviving.contains(&component[r]) {
                *result.entry(roots[r]).or_default() += edges + 1 - vertices;
            }
        }
        result
    };
    let old_ranks = ranks(&old_paths, old_root);
    let new_ranks = ranks(&new_paths, new_root);
    let common_old = ranks(&common, old_root);
    let common_new = ranks(&common, new_root);
    let mut closed = HashSet::default();
    let mut closed_count = 0;
    for (root, count) in old_ranks {
        let lost = count - common_old.get(&root).copied().unwrap_or(0);
        if lost > 0 {
            closed.insert(root);
            closed_count += lost;
        }
    }
    let opened_count = new_ranks
        .into_iter()
        .map(|(r, count)| count - common_new.get(&r).copied().unwrap_or(0))
        .sum();
    (closed, closed_count, opened_count)
}

/// Boundary edge components touching a delivered arc or live junction are protected in
/// full. A free vertex never maps onto a protected one, so each such boundary edge keeps
/// its one incident face. No touching hole can close during interior clustering.
pub(super) fn protected(
    m: &Merged,
    interface: &[InterfaceRow],
    interior: bool,
) -> (Vec<bool>, usize) {
    let border = points(interface);
    let mut locked: Vec<bool> = m.keys.iter().map(|k| border.contains(k)).collect();
    let mut edges: HashMap<(u32, u32), u32> = HashMap::default();
    for t in &m.triangles {
        for i in 0..3 {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut boundary = Components::new(m.keys.len());
    for (&(a, b), &n) in &edges {
        if n == 1 {
            boundary.join(a, b);
        }
    }
    let mut touched = HashSet::default();
    for (v, &lock) in locked.iter().enumerate() {
        if lock {
            touched.insert(boundary.root(v));
        }
    }
    let mut loops = HashSet::default();
    for (&(a, b), &n) in &edges {
        if n == 1 {
            let root = boundary.root(a as usize);
            loops.insert(root);
            if interior || touched.contains(&root) {
                locked[a as usize] = true;
                locked[b as usize] = true;
            }
        }
    }
    // One fixed support triangle per component prevents deletion of any surface component.
    let component = Components::mesh(m);
    let mut supported = HashSet::default();
    for t in &m.triangles {
        if supported.insert(component[t[0] as usize]) {
            for &v in t {
                locked[v as usize] = true;
            }
        }
    }
    (locked, loops.len())
}

/// Topology-changing vertex clustering, only after ordinary simplification has stalled.
/// All allocations and visits are bounded by the eight-child job. Components remain
/// separate; protected boundary loops and one support face per component survive.
pub(super) fn remesh(m: &mut Merged, interface: &[InterfaceRow], round: u32, r: &mut Receipt) {
    let (locked, _) = protected(m, interface, false);
    let component = Components::mesh(m);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for t in &m.triangles {
        for &v in t {
            for a in 0..3 {
                let x = f64::from(m.positions[v as usize][a]);
                lo[a] = lo[a].min(x);
                hi[a] = hi[a].max(x);
            }
        }
    }
    let extent = (0..3).map(|a| hi[a] - lo[a]).fold(0f64, f64::max);
    if extent == 0. {
        return;
    }
    let width = extent / (1u32 << (6 - round.min(6))) as f64;
    let mut cells = HashMap::default();
    let mut map = Vec::with_capacity(m.keys.len());
    for (v, p) in m.positions.iter().enumerate() {
        if locked[v] {
            map.push(v as u32);
            continue;
        }
        let cell: [i64; 3] =
            std::array::from_fn(|a| ((f64::from(p[a]) - lo[a]) / width).floor() as i64);
        let id = *cells.entry((component[v], cell)).or_insert(v as u32);

        map.push(id);
    }
    let mut output: Vec<_> = m
        .triangles
        .iter()
        .map(|t| mapped(&m.positions, *t, &map))
        .collect();
    let mut seen = HashSet::default();
    let keep: Vec<_> = output
        .iter()
        .map(|t| surface(t) && seen.insert(oriented(t)))
        .collect();
    let mut i = 0;
    output.retain(|_| (keep[i], i += 1).0);
    r.remesh_rounds += 1;
    let Some(change) = changes(m, &output, interface, 0.).filter(|c| c.receipt.closed_holes == 0)
    else {
        r.rejected_remesh_rounds += 1;
        return;
    };
    r.add(change.receipt);
    r.remeshed_vertices += map
        .iter()
        .enumerate()
        .filter(|(v, to)| *v != **to as usize)
        .count() as u64;
    retain(m, &keep);
    m.triangles = output;
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(crate) fn ring() -> Merged {
        let positions = vec![
            [-2., -2., 0.],
            [2., -2., 0.],
            [2., 2., 0.],
            [-2., 2., 0.],
            [-0.1, -0.1, 0.],
            [0.1, -0.1, 0.],
            [0.1, 0.1, 0.],
            [-0.1, 0.1, 0.],
        ];
        let mut triangles = Vec::new();
        for i in 0..4u32 {
            let j = (i + 1) % 4;
            triangles.extend([[i, j, i + 4], [j, j + 4, i + 4]]);
        }
        Merged {
            keys: positions.iter().map(|p| p.map(f32::to_bits)).collect(),
            ells: vec![0; 8],
            positions,
            representative: vec![
                crate::mesh_large::hlod::store::SourcePrimitive {
                    node: 0,
                    mesh: 0,
                    primitive: 0,
                    expanded_ordinal: 0,
                    material: u32::MAX
                };
                triangles.len()
            ],
            triangles,
        }
    }
    #[test]
    fn many_touching_sheet_loops_can_retriangulate_but_none_can_close() {
        let mut m = ring();
        m.positions = vec![[0., 0., 0.]];
        m.triangles.clear();
        let mut reduced = Vec::new();
        for i in 0..64 {
            let a = m.positions.len() as u32;
            m.positions.extend([
                [1., i as f32 + 1., 0.],
                [2., i as f32 + 1., 0.],
                [2., i as f32 + 1., 1.],
            ]);
            m.triangles.extend([[0, a, a + 1], [0, a + 1, a + 2]]);
            reduced.push([0, a, a + 2]);
        }
        m.keys = m.positions.iter().map(|p| p.map(f32::to_bits)).collect();
        let p = m.keys[0];
        let border = vec![InterfaceRow::new(0, p, p, 0, 1)];
        assert_eq!(
            changes(&m, &reduced, &border, 100.)
                .unwrap()
                .receipt
                .closed_holes,
            0
        );
        reduced.pop();
        assert!(changes(&m, &reduced, &border, 100.).is_none());
        let loss = changes(&m, &reduced, &[], 100.).unwrap();
        assert_eq!(loss.receipt.closed_holes, 1);
        assert!(!loss.patch_faces.is_empty());
    }
    #[test]
    fn closing_one_hole_and_opening_another_do_not_cancel_in_the_receipt() {
        let mut m = ring();
        m.positions = (0..=3)
            .flat_map(|y| (0..=5).map(move |x| [x as f32, y as f32, 0.]))
            .collect();
        m.keys = m.positions.iter().map(|p| p.map(f32::to_bits)).collect();
        m.triangles.clear();
        let mut output = Vec::new();
        for y in 0..3u32 {
            for x in 0..5 {
                let a = y * 6 + x;
                let faces = [[a, a + 1, a + 7], [a, a + 7, a + 6]];
                if (x, y) != (1, 1) {
                    m.triangles.extend(faces);
                }
                if (x, y) != (3, 1) {
                    output.extend(faces);
                }
            }
        }
        let change = changes(&m, &output, &[], 1.).unwrap();
        assert_eq!(change.receipt.closed_holes, 1);
        assert_eq!(change.receipt.opened_holes, 1);
        assert_eq!(change.patch_faces.len(), 2);
        assert_eq!(change.boundary.len(), 4);
    }

    #[test]
    fn degenerate_strip_removal_cannot_remove_the_certified_edge() {
        let mut m = ring();
        m.positions = vec![
            [0., 0., 0.],
            [0.5, 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [1., 1., 0.],
        ];
        m.keys = m.positions.iter().map(|p| p.map(f32::to_bits)).collect();
        m.ells = vec![0; 5];
        m.triangles = vec![[0, 1, 2], [0, 3, 4]];
        m.representative.truncate(2);
        let [a, middle, b] = [m.keys[0], m.keys[1], m.keys[2]];
        let interface = vec![InterfaceRow::new(0, a, b, 0, 1)];
        let mut receipt = Receipt::default();
        contract(
            &mut m,
            &interface,
            &|p| Ok((p == middle).then_some((a, b, 0.5, 0))),
            &mut receipt,
        )
        .unwrap();
        let segments = [(a, b)].into_iter().collect();
        assert!(
            missing(&m.triangles, &m.keys, &segments, &HashSet::default())
                .0
                .is_empty()
        );
        assert_eq!(receipt.reconstructed_faces, 1);
    }
    #[test]
    fn closing_a_hole_with_border_contact_is_rejected_and_interior_closure_is_measured() {
        let m = ring();
        let mut filled = m.triangles.clone();
        filled.extend([[4, 5, 6], [4, 6, 7]]);
        let p = m.keys[4];
        let border = vec![InterfaceRow::new(0, p, p, 0, 1)];
        assert!(
            changes(&m, &filled, &border, 1.).is_none(),
            "even one junction contact protects a hole"
        );
        let change = changes(&m, &filled, &[], 1.).unwrap();
        let receipt = change.receipt;
        assert_eq!(receipt.closed_holes, 1);
        assert_eq!(receipt.dropped_components, 0);
        assert_eq!(change.patch_faces, vec![8, 9]);
        assert_eq!(change.boundary.len(), 4);
    }
    #[test]
    fn interior_remeshing_preserves_a_hole_touching_a_border_junction() {
        let mut m = ring();
        let original = m.triangles.clone();
        let p = m.keys[4];
        let border = vec![InterfaceRow::new(0, p, p, 0, 1)];
        let mut receipt = Receipt::default();
        for round in 1..=6 {
            remesh(&mut m, &border, round, &mut receipt);
        }
        assert_eq!(receipt.closed_holes, 0);
        let mut before = ring();
        before.triangles = original;
        assert!(changes(&before, &m.triangles, &border, 1.).is_some());
    }
    #[test]
    fn sub_g_border_component_survives_while_identical_interior_component_is_dropped() {
        let positions = vec![
            [0., 0., 0.],
            [0.1, 0., 0.],
            [0., 0.1, 0.],
            [1., 0., 0.],
            [1.1, 0., 0.],
            [1., 0.1, 0.],
        ];
        let keys: Vec<Key> = positions.iter().map(|p| p.map(f32::to_bits)).collect();
        let interface = vec![InterfaceRow::new(0, keys[0], keys[0], 0, 1)];
        let mut m = Merged {
            keys,
            positions,
            ells: vec![0; 6],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
            representative: vec![
                crate::mesh_large::hlod::store::SourcePrimitive {
                    node: 0,
                    mesh: 0,
                    primitive: 0,
                    expanded_ordinal: 0,
                    material: u32::MAX
                };
                2
            ],
        };
        assert!(changes(&m, &[[3, 4, 5]], &interface, 1.).is_none());
        assert_eq!(
            changes(&m, &[[0, 1, 2]], &interface, 1.)
                .unwrap()
                .receipt
                .dropped_components,
            1
        );
        assert!(
            changes(&m, &[[0, 1, 2]], &interface, 0.1).is_none(),
            "implicit deletion must obey the same size rule"
        );
        let mut large = m.clone();
        large.positions[4][0] = 3.;
        large.keys = large
            .positions
            .iter()
            .map(|p| p.map(f32::to_bits))
            .collect();
        assert!(changes(&large, &[[0, 1, 2]], &interface, 1.).is_none());
        let mut receipt = Receipt::default();
        cleanup(&mut large, &interface, 1., &mut receipt);
        assert_eq!(large.triangles.len(), 2);
        let mut r = Receipt::default();
        cleanup(&mut m, &interface, 1., &mut r);
        assert_eq!(m.triangles, vec![[0, 1, 2]]);
        assert_eq!(r.dropped_components, 1);
    }
}
