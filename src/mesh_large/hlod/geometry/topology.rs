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
    pub rejected_hole_openings: u64,
    pub max_dropped_component_diameter: f64,
    pub hole_patch_to_source: f64,
    pub hole_boundary_to_patch: f64,
    pub dropped_component_to_proxy: f64,
    pub opened_hole_to_proxy: f64,
    pub opened_hole_boundary_to_source: f64,
    pub hole_g_increase: f64,
    pub dropped_component_g_increase: f64,
    pub opened_hole_g_increase: f64,
    pub closed_hole_final_g: f64,
    pub opened_hole_final_g: f64,
    pub dropped_component_final_g: f64,
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
        self.rejected_hole_openings += r.rejected_hole_openings;
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
        self.opened_hole_to_proxy = self.opened_hole_to_proxy.max(r.opened_hole_to_proxy);
        self.opened_hole_boundary_to_source = self
            .opened_hole_boundary_to_source
            .max(r.opened_hole_boundary_to_source);
        self.opened_hole_g_increase = self.opened_hole_g_increase.max(r.opened_hole_g_increase);
        self.closed_hole_final_g = self.closed_hole_final_g.max(r.closed_hole_final_g);
        self.opened_hole_final_g = self.opened_hole_final_g.max(r.opened_hole_final_g);
        self.dropped_component_final_g = self
            .dropped_component_final_g
            .max(r.dropped_component_final_g);
    }

    pub(crate) fn published(&mut self, g: f64) {
        if self.closed_holes > 0 {
            self.closed_hole_final_g = g;
        }
        if self.opened_holes > 0 {
            self.opened_hole_final_g = g;
        }
        if self.dropped_components > 0 {
            self.dropped_component_final_g = g;
        }
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
    pub opening_reference: Vec<([[f32; 3]; 3], bool)>,
    pub opening_boundary: Vec<[[f32; 3]; 2]>,
    pub opening_faces: Vec<bool>,
    pub opening_border_contact: bool,
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
fn preserves_orientation(p: &[[f32; 3]], before: [u32; 3], after: [u32; 3]) -> bool {
    let (a, b) = (normal(p, before), normal(p, after));
    // A collapsed face contributes no surface; a zero-area input cannot grow surface.
    b.iter().all(|&x| x == 0.) || (0..3).map(|i| a[i] * b[i]).sum::<f64>() > 0.
}

/// Two same-direction faces are supported only when they already shared their source
/// edge. Coalescing unrelated edges cannot create an overlapping manifold pair.
fn mapped_incidence(m: &Merged, map: &[u32]) -> bool {
    let mut edges = HashMap::<(u32, u32), ([u32; 2], (u32, u32), bool)>::default();
    for &t in &m.triangles {
        let out = t.map(|v| map[v as usize]);
        if normal(&m.positions, out).iter().all(|&x| x == 0.) {
            continue;
        }
        for i in 0..3 {
            let (a, b) = (out[i], out[(i + 1) % 3]);
            let old = (t[i].min(t[(i + 1) % 3]), t[i].max(t[(i + 1) % 3]));
            let row = edges
                .entry((a.min(b), a.max(b)))
                .or_insert(([0, 0], old, true));
            row.0[usize::from(a > b)] += 1;
            row.2 &= row.1 == old;
        }
    }
    edges.values().all(|(counts, _, same)| {
        *same || !((counts[0] == 2 && counts[1] == 0) || (counts[1] == 2 && counts[0] == 0))
    })
}
fn retain(m: &mut Merged, keep: &[bool]) {
    let mut i = 0;
    m.triangles.retain(|_| (keep[i], i += 1).0);
    let mut i = 0;
    m.representative.retain(|_| (keep[i], i += 1).0);
}

/// Consume the shared chord without folding incident faces. A sign-preserving collapse
/// is cheap; otherwise rebuild the affected manifold patch against the exact boundary.
/// Failure leaves the source mesh intact for the caller's reported frontier policy.
pub(super) fn contract(
    m: &mut Merged,
    interface: &[InterfaceRow],
    mapping: &super::BorderMap<'_>,
    receipt: &mut Receipt,
) -> Result<bool, JobError> {
    let ids: HashMap<Key, u32> = m
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u32))
        .collect();
    let mut map: Vec<u32> = (0..m.keys.len() as u32).collect();
    let mut endpoints = Vec::new();
    for (v, target) in map.iter_mut().enumerate() {
        if let Some((a, b, t, _)) = mapping(m.keys[v])? {
            let k = if t <= 0.5 { a } else { b };
            let Some(&id) = ids.get(&k) else {
                return Ok(false);
            };
            if id != v as u32 {
                let (Some(&a), Some(&b)) = (ids.get(&a), ids.get(&b)) else {
                    return Ok(false);
                };
                endpoints.extend([a, b]);
                receipt.contracted_vertices += 1;
            }
            *target = id;
        }
    }
    if receipt.contracted_vertices == 0 {
        return Ok(true);
    }
    let mut output = Vec::with_capacity(m.triangles.len());
    let mut representatives = Vec::with_capacity(m.triangles.len());
    let mut folded = false;
    for (i, &t) in m.triangles.iter().enumerate() {
        let out = t.map(|v| map[v as usize]);
        let (a, b) = (normal(&m.positions, t), normal(&m.positions, out));
        let old_area = a.iter().any(|&x| x != 0.);
        let new_area = b.iter().any(|&x| x != 0.);
        if !preserves_orientation(&m.positions, t, out) {
            folded = true;
        }
        // Preserve an already degenerate source support, but never create a new sliver.
        if new_area || !old_area {
            output.push(out);
            representatives.push(m.representative[i]);
        }
    }
    let required = points(interface);
    let segments = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .map(|r| (r.a, r.b))
        .collect();
    let lost = missing(&output, &m.keys, &segments, &required);
    if !folded
        && !output.is_empty()
        && lost.0.is_empty()
        && lost.1.is_empty()
        && mapped_incidence(m, &map)
        && super::folds::Source::new(&m.positions, &m.triangles)
            .preserves_faces(&m.positions, &output)
    {
        m.triangles = output;
        m.representative = representatives;
        return Ok(true);
    }
    if let Some(r) = super::constrained::contract(m, interface, &map, &endpoints) {
        receipt.reconstructed_faces += m.triangles.len() as u64;
        receipt.add(r);
        Ok(true)
    } else {
        receipt.rejected_remesh_rounds += 1;
        Ok(false)
    }
}

/// Duplicated oriented soup adds no surface. Only components with NO border contact and
/// strict diameter bound below the known G floor can be dropped. Keep one component if
/// every component is eligible, so the parent still has geometry to measure against.
pub(super) fn cleanup(
    m: &mut Merged,
    interface: &[InterfaceRow],
    g: f64,
    r: &mut Receipt,
) -> Vec<[[f32; 3]; 3]> {
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
    let removed = m
        .triangles
        .iter()
        .zip(&keep)
        .filter(|(_, keep)| !**keep)
        .map(|(t, _)| t.map(|v| m.positions[v as usize]))
        .collect();
    retain(m, &keep);
    removed
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
    let surviving: HashSet<usize> = output
        .iter()
        .flatten()
        .map(|v| component[*v as usize])
        .collect();
    let mut bounds: HashMap<usize, ([f64; 3], [f64; 3], bool)> = HashMap::default();
    for t in &m.triangles {
        // Only a removed component needs a diameter/contact eligibility decision.
        // Boundary preservation below still checks every surviving component.
        if surviving.contains(&component[t[0] as usize]) {
            continue;
        }
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
        // All subsequent graph comparisons consume only one-sided boundary edges.
        // Compact once instead of repeatedly walking the full surface-edge table.
        edges.retain(|_, n| *n == 1);
        edges.shrink_to_fit();
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
    let (closed, opened, closed_count, opened_count) = changed_cycles(
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
    let patch_faces = if receipt.closed_holes > 0 {
        let old_faces: HashSet<_> = m.triangles.iter().map(oriented).collect();
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
    let mut opening_reference = Vec::new();
    let mut opening_boundary = Vec::new();
    let mut opening_faces = Vec::new();
    let mut opening_border_contact = false;
    if opened_count > 0 {
        // Include junction-only contacts, even when the vertex was interior before the
        // simplification and therefore absent from the old touching-boundary signature.
        opening_border_contact = (0..m.keys.len()).any(|v| {
            new_degree[v] > 0 && opened.contains(&new_root[v]) && border.contains(&m.keys[v])
        });
        let affected: HashSet<_> = opened.iter().map(|&r| component[r]).collect();
        let new_faces: HashSet<_> = output.iter().map(oriented).collect();
        opening_reference = m
            .triangles
            .iter()
            .map(|t| {
                (
                    t.map(|v| m.positions[v as usize]),
                    affected.contains(&component[t[0] as usize])
                        && !new_faces.contains(&oriented(t)),
                )
            })
            .collect();
        let mut edges: Vec<_> = new_edges
            .iter()
            .filter(|((a, _), n)| **n == 1 && opened.contains(&new_root[*a as usize]))
            .map(|(&e, _)| e)
            .collect();
        edges.sort_unstable();
        opening_boundary = edges
            .iter()
            .map(|&(a, b)| [m.positions[a as usize], m.positions[b as usize]])
            .collect();
        let edges: HashSet<_> = edges.into_iter().collect();
        opening_faces = output
            .iter()
            .map(|t| {
                (0..3).any(|i| {
                    let (a, b) = (t[i], t[(i + 1) % 3]);
                    edges.contains(&(a.min(b), a.max(b)))
                })
            })
            .collect();
    }
    Some(Change {
        receipt,
        patch_faces,
        boundary,
        dropped,
        opening_reference,
        opening_boundary,
        opening_faces,
        opening_border_contact,
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
) -> (HashSet<usize>, HashSet<usize>, usize, usize) {
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
    let mut opened = HashSet::default();
    let mut opened_count = 0;
    for (r, count) in new_ranks {
        let added = count - common_new.get(&r).copied().unwrap_or(0);
        if added > 0 {
            opened.insert(r);
            opened_count += added;
        }
    }
    (closed, opened, closed_count, opened_count)
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
        .map(|t| t.map(|v| map[v as usize]))
        .collect();
    r.remesh_rounds += 1;
    if m.triangles
        .iter()
        .zip(&output)
        .any(|(&a, &b)| !preserves_orientation(&m.positions, a, b))
        || !mapped_incidence(m, &map)
    {
        r.rejected_remesh_rounds += 1;
        return;
    }
    let mut seen = HashSet::default();
    let keep: Vec<_> = output
        .iter()
        .map(|t| surface(t) && seen.insert(oriented(t)))
        .collect();
    let mut i = 0;
    output.retain(|_| (keep[i], i += 1).0);
    let Some(change) = changes(m, &output, interface, 0.)
        .filter(|c| c.receipt.closed_holes == 0 && c.receipt.opened_holes == 0)
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
        assert_eq!(receipt.reconstructed_faces, 0);
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
