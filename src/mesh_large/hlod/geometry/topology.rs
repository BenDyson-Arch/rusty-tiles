//! Bounded, tile-local boundary reconstruction and topology cleanup. The source graph
//! supplies the boundary; this module never chooses or moves a contracted endpoint.
#[cfg(test)]
use super::missing;
use super::{oriented, Merged};
use crate::{
    mesh_large::hlod::interfaces::{InterfaceRow, Key},
    JobError,
};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

fn is_zero(n: &u64) -> bool {
    *n == 0
}

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
    /// Coarse drops (D11 amendment 2026-10-28): dropped components at or above the
    /// tile's G floor, licensed only once no other strategy fits the cap.
    #[serde(skip_serializing_if = "is_zero")]
    pub coarse_dropped_components: u64,
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
        self.coarse_dropped_components += r.coarse_dropped_components;
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
    /// Changed faces on the closing clusters' boundary vertices (the oracle's domain).
    pub patch_faces: Vec<usize>,
    /// The rest of the closure's component. A branching boundary can include retained
    /// loops; their source points must still see their existing surface.
    pub closure_carriers: Vec<usize>,
    /// Vertex centroids of a fundamental cycle basis of each closing cluster's old
    /// boundary; measured where they project onto the proxy, as the oracle does.
    pub closure_centroids: Vec<[f64; 3]>,
    pub boundary: Vec<[[f32; 3]; 2]>,
    pub dropped: Vec<[[f32; 3]; 3]>,
    pub opening_reference: Vec<([[f32; 3]; 3], bool)>,
    pub opening_boundary: Vec<[[f32; 3]; 2]>,
    pub opening_faces: Vec<bool>,
    pub opening_border_contact: bool,
    pub strip: Strip,
}

/// Witnesses of a moved open boundary: input points on the changed surface behind it
/// (measured to the proxy), and added boundary edges with the output faces (indices)
/// along them (measured to the source).
#[derive(Debug, Default)]
pub(crate) struct Strip {
    pub lost_points: Vec<[f64; 3]>,
    pub added_edges: Vec<[[f32; 3]; 2]>,
    pub added_faces: Vec<usize>,
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
    super::interface_points(interface)
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
    previous_interface: &[InterfaceRow],
    mapping: &super::BorderMap<'_>,
    receipt: &mut Receipt,
) -> Result<bool, JobError> {
    contract_from(m, interface, previous_interface, mapping, receipt, None)
}

/// A removed border vertex goes to its nearer chord endpoint, unless more of its faces
/// already hold the farther one with an interface or removed third corner. Such a face
/// keeps only interface points after the move, a face the guard refuses unless it
/// collapses; moving to the endpoint it holds collapses it. The shared contract is
/// unchanged: each side remaps only its own faces, and both keep the same chord.
fn choose_targets(
    m: &Merged,
    interface: &[InterfaceRow],
    ids: &HashMap<Key, u32>,
    mapping: &[Option<(Key, Key, f64, u8)>],
    map: &mut [u32],
) {
    let required = points(interface);
    let held = |v: u32| map[v as usize] != v || required.contains(&m.keys[v as usize]);
    // Per moved vertex: votes for its nearer and farther endpoint.
    let mut votes: HashMap<u32, [u32; 2]> = HashMap::default();
    for t in &m.triangles {
        for c in 0..3 {
            let v = t[c];
            if map[v as usize] == v {
                continue;
            }
            let Some((a, b, _, _)) = mapping[v as usize] else {
                continue;
            };
            let (Some(&a), Some(&b)) = (ids.get(&a), ids.get(&b)) else {
                continue;
            };
            let near = map[v as usize];
            let far = if near == a { b } else { a };
            let (p, q) = (t[(c + 1) % 3], t[(c + 2) % 3]);
            for (e, side) in [(near, 0), (far, 1)] {
                let other = if p == e {
                    q
                } else if q == e {
                    p
                } else {
                    continue;
                };
                if other != a && other != b && held(other) {
                    votes.entry(v).or_default()[side] += 1;
                }
            }
        }
    }
    for (v, [near, far]) in votes {
        if far > near {
            let Some((a, b, _, _)) = mapping[v as usize] else {
                continue;
            };
            let (Some(&a), Some(&b)) = (ids.get(&a), ids.get(&b)) else {
                continue;
            };
            map[v as usize] = if map[v as usize] == a { b } else { a };
        }
    }
}

/// Keep the licenses of the original merged children across recovery proposals.
pub(super) fn contract_from(
    m: &mut Merged,
    interface: &[InterfaceRow],
    previous_interface: &[InterfaceRow],
    mapping: &super::BorderMap<'_>,
    receipt: &mut Receipt,
    original_faces: Option<&[[u32; 3]]>,
) -> Result<bool, JobError> {
    let _cpu = super::super::cpu_profile::start(
        super::super::cpu_profile::Stage::Contract,
        m.triangles.len(),
    );
    let ids: HashMap<Key, u32> = m
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u32))
        .collect();
    let mut map: Vec<u32> = (0..m.keys.len() as u32).collect();
    let mut endpoints = Vec::new();
    let mut mapping_by_vertex = Vec::with_capacity(m.keys.len());
    for (v, target) in map.iter_mut().enumerate() {
        let mapped = mapping(m.keys[v])?;
        mapping_by_vertex.push(mapped);
        if let Some((a, b, t, _)) = mapped {
            let k = if t <= 0.5 { a } else { b };
            let Some(&id) = ids.get(&k) else {
                return Err(super::state(
                    "a nested border chord endpoint is absent from its merged children",
                ));
            };
            if id != v as u32 {
                let (Some(&a), Some(&b)) = (ids.get(&a), ids.get(&b)) else {
                    return Err(super::state(
                        "a nested border chord endpoint is absent from its merged children",
                    ));
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
    choose_targets(m, interface, &ids, &mapping_by_vertex, &mut map);
    drop(ids);
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
    let lost = super::missing_owned(&output, &m.keys, segments, required);
    let direct = !folded
        && !output.is_empty()
        && lost.0.is_empty()
        && lost.1.is_empty()
        && changes(m, &output, interface, 0.).is_some();
    drop(lost);
    let guard = super::folds::Source::for_contraction(
        &m.positions,
        original_faces.unwrap_or(&m.triangles),
        interface,
        previous_interface,
        &mapping_by_vertex,
    );
    drop(mapping_by_vertex);
    if direct && !guard.violations(&m.positions, &output).iter().any(|v| *v) {
        m.triangles = output;
        m.representative = representatives;
        return Ok(true);
    }
    drop(output);
    drop(representatives);
    if let Some(r) = super::constrained::contract(
        m,
        interface,
        &map,
        &endpoints,
        original_faces.map_or(m.triangles.len(), <[_]>::len),
        &guard,
    ) {
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
    let (component, parts) = parts(m, interface);
    let mut dropped: HashSet<usize> = parts
        .iter()
        .filter(|(_, p)| !p.touch && p.diagonal < g)
        .map(|(c, _)| *c)
        .collect();
    if dropped.len() == parts.len() {
        // Keep the largest (drops go smallest first, as validators check).
        if let Some(c) = parts.iter().max_by(|a, b| a.1.order(b.1)).map(|(c, _)| *c) {
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

/// One component of a merged mesh's faces.
pub(super) struct Part {
    pub diagonal: f64,
    pub triangles: u64,
    pub least: Key,
    /// A key on the parent's interface.
    pub touch: bool,
}
impl Part {
    /// Drop order: bounding-box diagonal, then triangle count, then least key.
    pub(super) fn order(&self, other: &Self) -> std::cmp::Ordering {
        self.diagonal
            .total_cmp(&other.diagonal)
            .then(self.triangles.cmp(&other.triangles))
            .then(self.least.cmp(&other.least))
    }
}

/// Each vertex's component and every component's part, over `m`'s faces.
pub(super) fn parts(m: &Merged, interface: &[InterfaceRow]) -> (Vec<usize>, HashMap<usize, Part>) {
    let component = Components::mesh(m);
    let border = points(interface);
    let mut boxes: HashMap<usize, ([f64; 3], [f64; 3], Part)> = HashMap::default();
    for t in &m.triangles {
        let b = boxes.entry(component[t[0] as usize]).or_insert((
            [f64::MAX; 3],
            [f64::MIN; 3],
            Part {
                diagonal: 0.,
                triangles: 0,
                least: m.keys[t[0] as usize],
                touch: false,
            },
        ));
        b.2.triangles += 1;
        for &v in t {
            let p = m.positions[v as usize].map(f64::from);
            for (a, x) in p.into_iter().enumerate() {
                b.0[a] = b.0[a].min(x);
                b.1[a] = b.1[a].max(x);
            }
            let k = m.keys[v as usize];
            b.2.least = b.2.least.min(k);
            b.2.touch |= border.contains(&k);
        }
    }
    let parts = boxes
        .into_iter()
        .map(|(c, (lo, hi, mut part))| {
            part.diagonal = (0..3).map(|a| (hi[a] - lo[a]).powi(2)).sum::<f64>().sqrt();
            (c, part)
        })
        .collect();
    (component, parts)
}

/// Drops go smallest first, as validators check from the archive: once `output` drops a
/// component of `m` with no border contact and a diagonal of at least `from` (the
/// children's G), every kept border-free component smaller than the largest drop goes
/// too. Each lies below that drop's diagonal, hence below its licence, and only shrinks
/// the output; the caller fails the attempt if nothing is left. Returns whether `output`
/// changed.
pub(super) fn order_drops(
    m: &Merged,
    output: &mut Vec<[u32; 3]>,
    interface: &[InterfaceRow],
    from: f64,
) -> bool {
    let (component, parts) = parts(m, interface);
    let kept: HashSet<usize> = output.iter().map(|t| component[t[0] as usize]).collect();
    let largest = parts
        .iter()
        .filter(|(c, p)| !p.touch && !kept.contains(c))
        .map(|(_, p)| p.diagonal)
        .fold(f64::MIN, f64::max);
    if largest < from {
        return false;
    }
    let smaller: HashSet<usize> = parts
        .iter()
        .filter(|(c, p)| !p.touch && kept.contains(c) && p.diagonal < largest)
        .map(|(c, _)| *c)
        .collect();
    if smaller.is_empty() {
        return false;
    }
    output.retain(|t| !smaller.contains(&component[t[0] as usize]));
    true
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
    let _cpu =
        super::super::cpu_profile::start(super::super::cpu_profile::Stage::Topology, output.len());
    if output == m.triangles {
        return Some(Change::default());
    }
    let rejected = |reason: &str| {
        #[cfg(test)]
        if std::env::var_os("RT_HLOD_REPLAY_DIR").is_some()
            || std::env::var_os("RT_HLOD_FOLD_REPLAY").is_some()
        {
            eprintln!("TOPOLOGY: {reason}");
        }
        let _ = reason;
        None
    };
    let component = Components::mesh(m);
    let border = points(interface);
    let mut surviving = HashSet::default();
    for t in output {
        for &v in t {
            surviving.insert(component[v as usize]);
        }
    }
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
    drop(bounds);
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
        edges.retain(|_, n| *n == 1);
        edges.shrink_to_fit();
        edges
    };
    let mut old_edges = boundary(&m.triangles);
    let mut new_edges = boundary(output);
    // A connected component of the UNION boundary graph whose edge sets are
    // identical has identical cycles, contacts and path multiplicities. Remove
    // only these exact unchanged components before correspondence work. Joining
    // both graphs first retains every old/new component involved in a change.
    {
        let mut union = Components::new(m.keys.len());
        for &(a, b) in old_edges.keys().chain(new_edges.keys()) {
            union.join(a, b);
        }
        let mut changed = vec![false; m.keys.len()];
        for &(a, _) in old_edges.keys().filter(|e| !new_edges.contains_key(e)) {
            changed[union.root(a as usize)] = true;
        }
        for &(a, _) in new_edges.keys().filter(|e| !old_edges.contains_key(e)) {
            changed[union.root(a as usize)] = true;
        }
        old_edges.retain(|&(a, _), _| changed[union.root(a as usize)]);
        new_edges.retain(|&(a, _), _| changed[union.root(a as usize)]);
    }
    old_edges.shrink_to_fit();
    new_edges.shrink_to_fit();
    if dropped.is_empty() && old_edges.is_empty() && new_edges.is_empty() {
        return Some(Change::default());
    }
    let graph = |edges: &HashMap<(u32, u32), u32>| {
        let mut groups = Components::new(m.keys.len());
        let mut degree = vec![0u32; m.keys.len()];
        for &(a, b) in edges.keys() {
            groups.join(a, b);
            degree[a as usize] += 1;
            degree[b as usize] += 1;
        }
        let root: Vec<usize> = (0..m.keys.len()).map(|v| groups.root(v)).collect();
        (degree, root)
    };
    let (old_degree, old_root) = graph(&old_edges);
    let (new_degree, new_root) = graph(&new_edges);
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
        let (offset, adjacent) = boundary_adjacency(m.keys.len(), edges);
        let neighbors = |v: u32| &adjacent[offset[v as usize]..offset[v as usize + 1]];
        let mut paths = HashMap::with_capacity_and_hasher(edges.len(), Default::default());
        for &a in &anchors {
            if neighbors(a).is_empty() {
                return None;
            }
            for &first in neighbors(a) {
                let (mut previous, mut at) = (a, first);
                while !anchors.contains(&at) {
                    let next = neighbors(at);
                    if next.len() != 2 {
                        return None;
                    }
                    let next = if next[0] == previous {
                        next[1]
                    } else {
                        next[0]
                    };
                    previous = at;
                    at = next;
                }
                // Walk each anchored path in both directions; retain its canonical
                // endpoint/first-edge direction. This includes loops at one anchor.
                // Degree-two interiors cannot branch into an unanchored cycle.
                if (a, first) < (at, previous) {
                    *paths.entry((a.min(at), a.max(at))).or_default() += 1;
                }
            }
        }
        Some(paths)
    };
    let old_signature = signature(&old_edges)?;
    if signature(&new_edges).as_ref() != Some(&old_signature) {
        return rejected("touching boundary graph changed after suppressing degree-two vertices");
    }
    drop((old_signature, anchors, touching));
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
    let before_count = cycles(&old_edges, &old_degree, &old_root)
        .values()
        .sum::<usize>();
    let after_count = cycles(&new_edges, &new_degree, &new_root)
        .values()
        .sum::<usize>();
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
    debug_assert_eq!(before_count + opened_count, after_count + closed_count);
    let affected: HashSet<usize> = closed.iter().map(|&r| component[r]).collect();
    let mut patch_faces = Vec::new();
    let mut closure_carriers = Vec::new();
    let mut closure_centroids = Vec::new();
    let mut boundary = Vec::new();
    if receipt.closed_holes > 0 {
        // The S5 oracle's closure domain: changed output faces touching a boundary
        // vertex of a cluster (component of old and new boundary edges) that lost a
        // cycle. Output vertices are input vertices: no Steiner point to pass through.
        let mut cluster = Components::new(m.keys.len());
        for &(a, b) in old_edges.keys().chain(new_edges.keys()) {
            cluster.join(a, b);
        }
        let mut closing = vec![false; m.keys.len()];
        for &(a, _) in old_edges.keys() {
            if closed.contains(&old_root[a as usize]) {
                closing[cluster.root(a as usize)] = true;
            }
        }
        let mut seed = vec![false; m.keys.len()];
        for &(a, b) in old_edges.keys().chain(new_edges.keys()) {
            if closing[cluster.root(a as usize)] {
                seed[a as usize] = true;
                seed[b as usize] = true;
            }
        }
        drop((cluster, closing));
        // As the oracle: a cluster that lost a cycle may not touch a delivered border
        // anywhere, including where its moved boundary reaches a formerly interior point.
        if (0..m.keys.len()).any(|v| seed[v] && border.contains(&m.keys[v])) {
            return rejected("a closing boundary cluster touches a delivered border");
        }
        closure_centroids = cycle_centroids(m, &old_edges, &seed);
        // Changed by multiset, as the oracle counts: a repeated face is changed once
        // its input copies are used up. The patch is selected over the whole output, as
        // the oracle does; carriers are per affected component, which is sound because no
        // output face spans two input components (simplification collapses edges, and
        // remesh cells and repairs are keyed by component).
        debug_assert!(output.iter().all(|t| {
            t.iter()
                .all(|&v| component[v as usize] == component[t[0] as usize])
        }));
        let mut old_faces: HashMap<[u32; 3], u32> = HashMap::default();
        for t in &m.triangles {
            *old_faces.entry(oriented(t)).or_default() += 1;
        }
        for (i, t) in output.iter().enumerate() {
            let kept = old_faces
                .get_mut(&oriented(t))
                .is_some_and(|n| std::mem::replace(n, n.saturating_sub(1)) > 0);
            if !kept && t.iter().any(|&v| seed[v as usize]) {
                patch_faces.push(i);
            } else if affected.contains(&component[t[0] as usize]) {
                closure_carriers.push(i);
            }
        }
        let mut edges: Vec<_> = old_edges
            .keys()
            .filter(|(a, _)| closed.contains(&old_root[*a as usize]))
            .copied()
            .collect();
        edges.sort_unstable();
        boundary = edges
            .into_iter()
            .map(|(a, b)| [m.positions[a as usize], m.positions[b as usize]])
            .collect();
    }
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
    let strip = strip(
        m, output, &old_edges, &new_edges, &border, &component, &surviving,
    );
    Some(Change {
        receipt,
        patch_faces,
        closure_carriers,
        closure_centroids,
        boundary,
        dropped,
        opening_reference,
        opening_boundary,
        opening_faces,
        opening_border_contact,
        strip,
    })
}

/// A boundary simplification moves an open boundary without necessarily opening or
/// closing a cycle, and simplifies the surface behind it. Measure that surface the way
/// the S5 source oracle does (`hlod-s1b-borders.md`): changed input faces reachable from
/// each changed boundary cluster, flooding only through cluster and removed vertices.
/// S2's area strata can miss a local peak there; these witnesses join S2's fixed
/// controls and hence G. Clusters changed only along shared borders (certified
/// contraction) and dropped components have their own controls.
fn strip(
    m: &Merged,
    output: &[[u32; 3]],
    old_edges: &HashMap<(u32, u32), u32>,
    new_edges: &HashMap<(u32, u32), u32>,
    border: &HashSet<Key>,
    component: &[usize],
    surviving: &HashSet<usize>,
) -> Strip {
    let n = m.keys.len();
    let on_border = |a: u32, b: u32| {
        border.contains(&m.keys[a as usize]) && border.contains(&m.keys[b as usize])
    };
    let live = |a: u32| surviving.contains(&component[a as usize]);
    let mut cluster = Components::new(n);
    for &(a, b) in old_edges.keys().chain(new_edges.keys()) {
        cluster.join(a, b);
    }
    let mut moved_cluster = vec![false; n];
    let mut added: Vec<(u32, u32)> = Vec::new();
    for (from, to, collect) in [(old_edges, new_edges, false), (new_edges, old_edges, true)] {
        for &(a, b) in from.keys() {
            if !to.contains_key(&(a, b)) && live(a) && !on_border(a, b) {
                moved_cluster[cluster.root(a as usize)] = true;
                if collect {
                    added.push((a, b));
                }
            }
        }
    }
    let mut seed = vec![false; n];
    for &(a, b) in old_edges.keys().chain(new_edges.keys()) {
        if moved_cluster[cluster.root(a as usize)] {
            seed[a as usize] = true;
            seed[b as usize] = true;
        }
    }
    drop((cluster, moved_cluster));
    if !seed.iter().any(|s| *s) {
        return Strip::default();
    }
    added.sort_unstable();
    let mut in_output = vec![false; n];
    for t in output {
        for &v in t {
            in_output[v as usize] = true;
        }
    }
    let kept: HashSet<_> = output.iter().map(oriented).collect();
    let changed: Vec<u32> = (0..m.triangles.len() as u32)
        .filter(|&i| {
            let t = &m.triangles[i as usize];
            live(t[0]) && !kept.contains(&oriented(t))
        })
        .collect();
    drop(kept);
    // Vertex -> changed-face incidence, flat, so the flood is linear.
    let mut offsets = vec![0usize; n + 1];
    for &i in &changed {
        for &v in &m.triangles[i as usize] {
            offsets[v as usize + 1] += 1;
        }
    }
    for v in 0..n {
        offsets[v + 1] += offsets[v];
    }
    let mut faces = vec![0u32; offsets[n]];
    let mut cursor = offsets[..n].to_vec();
    for (j, &i) in changed.iter().enumerate() {
        for &v in &m.triangles[i as usize] {
            faces[cursor[v as usize]] = j as u32;
            cursor[v as usize] += 1;
        }
    }
    drop(cursor);
    let passable = |v: usize| seed[v] || !in_output[v];
    let mut expanded = vec![false; n];
    let mut reached = vec![false; changed.len()];
    let mut todo = Vec::new();
    let mut expand = |v: usize, todo: &mut Vec<u32>, reached: &mut [bool]| {
        if !std::mem::replace(&mut expanded[v], true) {
            for &j in &faces[offsets[v]..offsets[v + 1]] {
                if !std::mem::replace(&mut reached[j as usize], true) {
                    todo.push(j);
                }
            }
        }
    };
    for (v, _) in seed.iter().enumerate().filter(|(_, s)| **s) {
        expand(v, &mut todo, &mut reached);
    }
    while let Some(j) = todo.pop() {
        for &v in &m.triangles[changed[j as usize] as usize] {
            if passable(v as usize) {
                expand(v as usize, &mut todo, &mut reached);
            }
        }
    }
    drop((faces, offsets, expanded, todo));
    // Each reached face: its distinct corners, edge midpoints and centroid.
    let position = |v: u32| m.positions[v as usize].map(f64::from);
    let mut corner = vec![false; n];
    let mut edges = HashSet::default();
    let mut lost_points = Vec::new();
    for (j, &i) in changed.iter().enumerate() {
        if !reached[j] {
            continue;
        }
        let t = m.triangles[i as usize];
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if !std::mem::replace(&mut corner[a as usize], true) {
                lost_points.push(position(a));
            }
            if edges.insert((a.min(b), a.max(b))) {
                let (p, q) = (position(a), position(b));
                lost_points.push(std::array::from_fn(|c| (p[c] + q[c]) / 2.));
            }
        }
        let p = t.map(position);
        lost_points.push(std::array::from_fn(|c| (p[0][c] + p[1][c] + p[2][c]) / 3.));
    }
    let added_set: HashSet<_> = added.iter().copied().collect();
    let added_faces = output
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            (0..3).any(|i| {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                added_set.contains(&(a.min(b), a.max(b)))
            })
        })
        .map(|(i, _)| i)
        .collect();
    Strip {
        lost_points,
        added_edges: added
            .iter()
            .map(|&(a, b)| [m.positions[a as usize], m.positions[b as usize]])
            .collect(),
        added_faces,
    }
}

/// The oracle's closure centroids: a depth-first spanning forest of the boundary edges
/// on `seed` vertices, iterating start points and adjacency in sorted position order as
/// the oracle does, so both build the same basis; each non-tree edge closes one cycle.
/// A cycle's vertex sum comes from root-relative prefix sums and a binary-lifting lowest
/// common ancestor, so B boundary edges cost O(B log B), not one walk per cycle.
fn cycle_centroids(m: &Merged, edges: &HashMap<(u32, u32), u32>, seed: &[bool]) -> Vec<[f64; 3]> {
    let position = |v: u32| m.positions[v as usize].map(f64::from);
    // -0.0 and +0.0 compare Equal, so two distinct vertices differing only in a zero's
    // sign tie and `sort_unstable` may order them either way, where the oracle's stable
    // sort keeps input order. A tie can at worst leave a duplicate start point or a
    // different basis, not an unsound witness; welded vertices should never differ so.
    let order = |a: u32, b: u32| {
        position(a)
            .partial_cmp(&position(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    let mut list: Vec<(u32, u32)> = edges
        .keys()
        .filter(|&&(a, _)| seed[a as usize])
        .map(|&(a, b)| if order(b, a).is_lt() { (b, a) } else { (a, b) })
        .collect();
    list.sort_unstable_by(|x, y| order(x.0, y.0).then(order(x.1, y.1)));
    let mut points: Vec<u32> = list.iter().flat_map(|&(a, b)| [a, b]).collect();
    points.sort_unstable_by(|&a, &b| order(a, b));
    points.dedup();
    let n = points.len();
    let mut local = vec![u32::MAX; m.keys.len()];
    for (i, &v) in points.iter().enumerate() {
        local[v as usize] = i as u32;
    }
    let list: Vec<(usize, usize)> = list
        .into_iter()
        .map(|(a, b)| (local[a as usize] as usize, local[b as usize] as usize))
        .collect();
    drop(local);
    // Adjacency in sorted edge order.
    let mut offset = vec![0usize; n + 1];
    for &(a, b) in &list {
        offset[a + 1] += 1;
        offset[b + 1] += 1;
    }
    for v in 0..n {
        offset[v + 1] += offset[v];
    }
    let mut adjacent = vec![0usize; offset[n]];
    let mut cursor = offset[..n].to_vec();
    for &(a, b) in &list {
        adjacent[cursor[a]] = b;
        cursor[a] += 1;
        adjacent[cursor[b]] = a;
        cursor[b] += 1;
    }
    drop(cursor);
    let mut parent = vec![usize::MAX; n];
    let mut depth = vec![0u32; n];
    let mut root = vec![0usize; n];
    let mut sum = vec![[0f64; 3]; n];
    let mut stack = Vec::new();
    for start in 0..n {
        if parent[start] != usize::MAX {
            continue;
        }
        (parent[start], root[start]) = (start, start);
        stack.push(start);
        while let Some(v) = stack.pop() {
            for &w in &adjacent[offset[v]..offset[v + 1]] {
                if parent[w] == usize::MAX {
                    let (p, r) = (position(points[w]), position(points[root[v]]));
                    (parent[w], depth[w], root[w]) = (v, depth[v] + 1, root[v]);
                    sum[w] = std::array::from_fn(|i| sum[v][i] + (p[i] - r[i]));
                    stack.push(w);
                }
            }
        }
    }
    drop((offset, adjacent, stack));
    let levels = (32 - depth.iter().max().map_or(0, |d| d.leading_zeros())).max(1) as usize;
    // Local indices are u32 (`local`), so the n x levels table holds u32, allocated once.
    let mut up: Vec<u32> = Vec::with_capacity(n * levels);
    up.extend(parent.iter().map(|&p| p as u32));
    for j in 1..levels {
        for v in 0..n {
            let half = up[(j - 1) * n + v] as usize;
            up.push(up[(j - 1) * n + half]);
        }
    }
    let ancestor = |mut a: usize, mut b: usize| {
        if depth[a] < depth[b] {
            std::mem::swap(&mut a, &mut b);
        }
        let lift = depth[a] - depth[b];
        for j in (0..levels).filter(|&j| lift >> j & 1 == 1) {
            a = up[j * n + a] as usize;
        }
        if a == b {
            return a;
        }
        for j in (0..levels).rev() {
            if up[j * n + a] != up[j * n + b] {
                (a, b) = (up[j * n + a] as usize, up[j * n + b] as usize);
            }
        }
        parent[a]
    };
    let mut out = Vec::new();
    for &(a, b) in &list {
        if parent[a] == b || parent[b] == a {
            continue;
        }
        let c = ancestor(a, b);
        let count = (depth[a] + depth[b] - 2 * depth[c] + 1) as f64;
        let (r, l) = (position(points[root[a]]), position(points[c]));
        out.push(std::array::from_fn(|i| {
            r[i] + (sum[a][i] + sum[b][i] - 2. * sum[c][i] + (l[i] - r[i])) / count
        }));
    }
    out
}

/// Flat adjacency keeps graph traversal linear without a Vec allocation per vertex.
fn boundary_adjacency(n: usize, edges: &HashMap<(u32, u32), u32>) -> (Vec<usize>, Vec<u32>) {
    let mut offsets = vec![0usize; n + 1];
    for (&(a, b), &count) in edges {
        if count == 1 {
            offsets[a as usize + 1] += 1;
            offsets[b as usize + 1] += 1;
        }
    }
    for v in 0..n {
        offsets[v + 1] += offsets[v];
    }
    let mut adjacent = vec![0; offsets[n]];
    let mut cursor = offsets[..n].to_vec();
    for (&(a, b), &count) in edges {
        if count == 1 {
            adjacent[cursor[a as usize]] = b;
            adjacent[cursor[b as usize]] = a;
            cursor[a as usize] += 1;
            cursor[b as usize] += 1;
        }
    }
    (offsets, adjacent)
}

/// Common boundary paths are compared after suppressing unshared degree-two vertices.
/// Cycle rank of their intersection distinguishes closure from simultaneous opening;
/// subtracting only whole-component ranks would silently cancel those two operations.
#[allow(clippy::too_many_arguments)]
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
        let (offset, adjacent) = boundary_adjacency(n, edges);
        let neighbors = |v: u32| &adjacent[offset[v as usize]..offset[v as usize + 1]];
        let mut paths =
            HashMap::<(u32, u32), usize>::with_capacity_and_hasher(edges.len(), Default::default());
        for (v, &anchor) in anchors.iter().enumerate() {
            if !anchor {
                continue;
            }
            let a = v as u32;
            for &first in neighbors(a) {
                let (mut previous, mut at) = (a, first);
                while !anchors[at as usize] {
                    let next = neighbors(at)
                        .iter()
                        .copied()
                        .find(|&v| v != previous)
                        .unwrap();
                    previous = at;
                    at = next;
                }
                if (a, first) < (at, previous) {
                    *paths.entry((a.min(at), a.max(at))).or_default() += 1;
                }
            }
        }
        paths
    };
    let (old_paths, new_paths) = (paths(old), paths(new));
    // Identical suppressed graphs with the same three-contact cycle witness make
    // their common graph identical too. Every rank subtraction below is then zero.
    // This avoids constructing three rank tables for dense retained boundaries.
    if old_paths == new_paths
        && old_paths.keys().all(|&(a, _)| {
            contacts
                .get(&(old_root[a as usize], new_root[a as usize]))
                .copied()
                .unwrap_or(0)
                >= 3
        })
    {
        return (HashSet::default(), HashSet::default(), 0, 0);
    }
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
    drop((anchors, contacts, old_anchored, new_anchored));
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
    drop(old_paths);
    let new_ranks = ranks(&new_paths, new_root);
    drop(new_paths);
    let common_old = ranks(&common, old_root);
    let common_new = ranks(&common, new_root);
    drop(common);
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

/// Exact carriers for protected boundary edges and one face of every component.
/// Merely pinning both edge endpoints does not preserve its incident face: its third
/// vertex can collapse onto an endpoint while the endpoints survive elsewhere.
pub(super) fn support_faces(m: &Merged, interface: &[InterfaceRow], interior: bool) -> Vec<bool> {
    let (locked, _) = protected(m, interface, interior);
    let mut edges: HashMap<(u32, u32), u32> = HashMap::default();
    for t in &m.triangles {
        for i in 0..3 {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let component = Components::mesh(m);
    let mut supported = HashSet::default();
    m.triangles
        .iter()
        .map(|t| {
            let support = supported.insert(component[t[0] as usize]);
            support
                || (0..3).any(|i| {
                    let (a, b) = (t[i], t[(i + 1) % 3]);
                    locked[a as usize] && locked[b as usize] && edges[&(a.min(b), a.max(b))] == 1
                })
        })
        .collect()
}

/// Cluster-splitting passes per remesh round.
const REMESH_SPLITS: u32 = 16;

/// Topology-changing vertex clustering, only after ordinary simplification has stalled.
/// All allocations and visits are bounded by the eight-child job. Components remain
/// separate; protected boundary loops and one support face per component survive.
pub(super) fn remesh(
    m: &mut Merged,
    interface: &[InterfaceRow],
    round: u32,
    r: &mut Receipt,
) -> bool {
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
        return false;
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
    r.remesh_rounds += 1;
    let Some(Split {
        output,
        keep,
        change,
        passes,
    }) = split_clusters(m, interface, &mut map, REMESH_SPLITS)
    else {
        r.rejected_remesh_rounds += 1;
        return false;
    };
    r.add(change.receipt);
    let changed = m.triangles != output;
    // Splits that dissolved every cluster leave nothing: the round failed.
    r.rejected_remesh_rounds += u64::from(passes > 0 && !changed);
    r.remeshed_vertices += map
        .iter()
        .enumerate()
        .filter(|(v, to)| *v != **to as usize)
        .count() as u64;
    retain(m, &keep);
    m.triangles = output;
    changed
}

/// An accepted clustering: its output, the kept input faces, its topology change and
/// the number of splits it took.
struct Split {
    output: Vec<[u32; 3]>,
    keep: Vec<bool>,
    change: Change,
    passes: u32,
}

/// A clustered face that flips, or a fold, condemns its clusters, not the round: split
/// those clusters (their vertices keep their own positions) and check again. Splitting
/// only moves towards the unchanged input, so at most `limit` splits either find a
/// clustering every face of which passes, or reject (None).
fn split_clusters(
    m: &Merged,
    interface: &[InterfaceRow],
    map: &mut [u32],
    limit: u32,
) -> Option<Split> {
    // Built per pass that reaches the fold check and dropped before `changes()`, so
    // the two never overlap; only a topology split pays to rebuild it.
    let mut guard = None;
    let mut passes = 0;
    loop {
        let mut output: Vec<[u32; 3]> = m
            .triangles
            .iter()
            .map(|t| t.map(|v| map[v as usize]))
            .collect();
        let mut split: HashSet<u32> = HashSet::default();
        for (&before, after) in m.triangles.iter().zip(&output) {
            if !preserves_orientation(&m.positions, before, *after) {
                split.extend(after.iter().copied());
            }
        }
        if split.is_empty() {
            let mut seen = HashSet::default();
            let keep: Vec<bool> = output
                .iter()
                .map(|t| surface(t) && seen.insert(oriented(t)))
                .collect();
            let mut i = 0;
            output.retain(|_| (keep[i], i += 1).0);
            let invalid = guard
                .get_or_insert_with(|| {
                    super::folds::Source::new(&m.positions, &m.triangles).with_boundary(interface)
                })
                .violations(&m.positions, &output);
            split.extend(
                invalid
                    .iter()
                    .enumerate()
                    .filter(|(_, bad)| **bad)
                    .map(|(v, _)| v as u32),
            );
            if split.is_empty() {
                drop((invalid, guard.take()));
                let change = changes(m, &output, interface, 0.)
                    .filter(|c| c.receipt.closed_holes == 0 && c.receipt.opened_holes == 0);
                if let Some(change) = change {
                    return Some(Split {
                        output,
                        keep,
                        change,
                        passes,
                    });
                }
                // A boundary cycle closed or opened: split every cluster on an input
                // boundary edge that no longer maps to an output boundary edge, and on
                // every new output boundary edge.
                let boundary = |faces: &[[u32; 3]]| {
                    let mut n: HashMap<(u32, u32), u32> = HashMap::default();
                    for t in faces.iter().filter(|t| surface(t)) {
                        for i in 0..3 {
                            let (a, b) = (t[i], t[(i + 1) % 3]);
                            *n.entry((a.min(b), a.max(b))).or_default() += 1;
                        }
                    }
                    n.retain(|_, c| *c == 1);
                    n
                };
                let (before, after) = (boundary(&m.triangles), boundary(&output));
                let mut images = HashSet::default();
                for &(a, b) in before.keys() {
                    let (x, y) = (map[a as usize], map[b as usize]);
                    images.insert((x.min(y), x.max(y)));
                    if x == y || !after.contains_key(&(x.min(y), x.max(y))) {
                        split.extend([x, y]);
                    }
                }
                for &(x, y) in after.keys() {
                    if !images.contains(&(x, y)) {
                        split.extend([x, y]);
                    }
                }
                let mut size = vec![0u32; map.len()];
                for &to in map.iter() {
                    size[to as usize] += 1;
                }
                split.retain(|&x| size[x as usize] > 1);
                if split.is_empty() {
                    return None;
                }
            }
        }
        if passes == limit {
            return None;
        }
        passes += 1;
        for (v, to) in map.iter_mut().enumerate() {
            if split.contains(to) {
                *to = v as u32;
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn a_missing_nested_chord_endpoint_is_an_invariant_error() {
        let mut m = mesh(
            vec![[0., 0., 0.], [0.5, 0., 0.], [0., 1., 0.]],
            vec![[0, 1, 2]],
        );
        let (a, b, middle) = (m.keys[0], super::super::key([1., 0., 0.]), m.keys[1]);
        let error = contract(
            &mut m,
            &[],
            &[],
            &|k| Ok((k == middle).then_some((a, b, 0.75, 0))),
            &mut Receipt::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), crate::JobErrorKind::InvalidState);
        assert_eq!(m.triangles, [[0, 1, 2]]);
    }

    /// Separate triangles at x = 20i, each `size` across.
    fn islands(sizes: &[f32]) -> Merged {
        let mut positions = Vec::new();
        for (i, &s) in sizes.iter().enumerate() {
            let x = 20. * i as f32;
            positions.extend([[x, 0., 0.], [x + s, 0., 0.], [x, s, 0.]]);
        }
        let triangles = (0..sizes.len() as u32)
            .map(|i| [3 * i, 3 * i + 1, 3 * i + 2])
            .collect();
        mesh(positions, triangles)
    }

    /// Review r2 P1-1: cleanup that licenses every component keeps the largest.
    #[test]
    fn cleanup_keeps_the_largest_component_when_every_one_is_eligible() {
        let mut m = islands(&[1., 3., 2.]);
        cleanup(&mut m, &[], 100., &mut Receipt::default());
        assert_eq!(m.triangles, [[3, 4, 5]]);
    }

    /// Review r2 P1-1: once a drop reaches the children's G, every kept border-free
    /// component smaller than it goes too; below that G nothing changes.
    #[test]
    fn drops_at_or_above_child_error_go_smallest_first() {
        let m = islands(&[8., 5., 3.]);
        let mut output = vec![m.triangles[0], m.triangles[2]];
        assert!(!order_drops(&m, &mut output, &[], 7.1));
        assert!(order_drops(&m, &mut output, &[], 0.));
        assert_eq!(output, [m.triangles[0]]);
    }

    fn mesh(positions: Vec<[f32; 3]>, triangles: Vec<[u32; 3]>) -> Merged {
        Merged {
            keys: positions.iter().copied().map(super::super::key).collect(),
            ells: vec![0; positions.len()],
            positions,
            representative: vec![ring().representative[0]; triangles.len()],
            triangles,
        }
    }

    #[test]
    fn contraction_review_p1_cannot_publish_an_acute_balanced_fan() {
        let mut m = mesh(
            vec![
                [0., 0., 0.],
                [0., 1., 0.],
                [1., 0., 1.],
                [1., 0., -1.],
                [0.125, 0., 0.],
                [1., 0., 2.],
                [1., 0., -2.],
                [1., 0., 0.],
                [0.5, -1., 0.],
            ],
            vec![
                [0, 1, 2],
                [1, 0, 3],
                [0, 2, 3],
                [1, 3, 2],
                [4, 1, 5],
                [1, 4, 6],
                [4, 5, 6],
                [1, 6, 5],
                [0, 4, 8],
                [4, 7, 8],
            ],
        );
        let (a, middle, q, r) = (m.keys[0], m.keys[4], m.keys[7], m.keys[8]);
        let rows = |edges: &[(Key, Key)]| {
            edges
                .iter()
                .map(|&(a, b)| InterfaceRow::new(0, a.min(b), a.max(b), 0, 1))
                .collect::<Vec<_>>()
        };
        let interface = rows(&[(a, q), (a, r), (q, r)]);
        let previous = rows(&[(a, middle), (middle, q), (a, r), (q, r)]);
        let before = m.clone();
        let mut mapping = vec![None; m.keys.len()];
        mapping[4] = Some((a, q, 0.5, 0));
        // Isolate the fan failure from the separate all-three-boundary-face
        // rule: only a and q are boundary contacts for this guard control.
        let guard = super::super::folds::Source::for_contraction(
            &m.positions,
            &m.triangles,
            &rows(&[(a, q)]),
            &previous,
            &mapping,
        );
        let mapped: Vec<_> = m
            .triangles
            .iter()
            .map(|t| t.map(|v| if v == 4 { 0 } else { v }))
            .filter(|&t| normal(&m.positions, t).iter().any(|&n| n != 0.))
            .collect();
        assert_eq!(mapped.len(), 9);
        let invalid = guard.violations(&m.positions, &mapped);
        assert!(invalid[1] && invalid[5] && invalid[6]);
        let mut receipt = Receipt::default();
        let accepted = contract(
            &mut m,
            &interface,
            &previous,
            &|p| Ok((p == middle).then_some((a, q, 0.5, 0))),
            &mut receipt,
        )
        .unwrap();
        if accepted {
            // A reconstructed result is allowed only if the original fan guard
            // accepts it; accepting the mapped nine-face witness is forbidden.
            let guard = super::super::folds::Source::new(&before.positions, &before.triangles);
            assert!(!guard
                .violations(&m.positions, &m.triangles)
                .iter()
                .any(|v| *v));
            let counts = m.triangles.iter().fold([0; 2], |mut counts, t| {
                for i in 0..3 {
                    match (t[i], t[(i + 1) % 3]) {
                        (0, 1) => counts[0] += 1,
                        (1, 0) => counts[1] += 1,
                        _ => {}
                    }
                }
                counts
            });
            assert_ne!(counts, [2, 2]);
        } else {
            assert_eq!(m.keys, before.keys);
            assert_eq!(m.ells, before.ells);
            assert_eq!(m.positions, before.positions);
            assert_eq!(m.triangles, before.triangles);
            assert_eq!(m.representative, before.representative);
        }
    }

    #[test]
    fn contraction_preserves_a_certified_acute_nonmanifold_chain() {
        let mut m = mesh(
            vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [2., 0., 0.],
                [0., 1., 0.],
                [0., 2., 1.],
                [0., -1., 0.],
            ],
            vec![
                [0, 1, 3],
                [1, 2, 3],
                [0, 1, 4],
                [1, 2, 4],
                [1, 0, 5],
                [2, 1, 5],
            ],
        );
        let (a, middle, b) = (m.keys[0], m.keys[1], m.keys[2]);
        let interface = [InterfaceRow::new(0, a, b, 0, 1)];
        let previous = [
            InterfaceRow::new(0, a, middle, 0, 1),
            InterfaceRow::new(0, middle, b, 0, 1),
        ];
        let mut receipt = Receipt::default();
        assert!(contract(
            &mut m,
            &interface,
            &previous,
            &|p| Ok((p == middle).then_some((a, b, 0.5, 0))),
            &mut receipt
        )
        .unwrap());
        assert_eq!(m.triangles, [[0, 2, 3], [0, 2, 4], [2, 0, 5]]);
        assert_eq!(receipt.contracted_vertices, 1);
        assert_eq!(receipt.reconstructed_faces, 0);
    }

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
    fn moved_boundary_witnesses_reach_simplified_surface_behind_the_strip() {
        // A 9x9 grid loses its bottom row (a plain boundary retreat: no cycle opens or
        // closes) and an interior vertex two rows in collapses onto the new boundary.
        // The oracle-equivalent domain floods past the lost strip to that vertex.
        let n = 8u32;
        let mut m = ring();
        m.positions = (0..=n)
            .flat_map(|y| (0..=n).map(move |x| [x as f32, y as f32, 0.]))
            .collect();
        m.keys = m.positions.iter().map(|p| p.map(f32::to_bits)).collect();
        m.ells = vec![0; m.positions.len()];
        m.triangles.clear();
        for y in 0..n {
            for x in 0..n {
                let a = y * (n + 1) + x;
                m.triangles
                    .extend([[a, a + 1, a + n + 2], [a, a + n + 2, a + n + 1]]);
            }
        }
        let at = |x: u32, y: u32| y * (n + 1) + x;
        let (v, u) = (at(4, 2), at(4, 1));
        let output: Vec<[u32; 3]> = m
            .triangles
            .iter()
            .filter(|t| t.iter().all(|&w| w > n))
            .map(|t| t.map(|w| if w == v { u } else { w }))
            .filter(super::surface)
            .collect();
        let change = changes(&m, &output, &[], 0.5).unwrap();
        assert_eq!(
            (change.receipt.opened_holes, change.receipt.closed_holes),
            (0, 0)
        );
        let has = |p: [f32; 3]| change.strip.lost_points.contains(&p.map(f64::from));
        assert!(has(m.positions[at(4, 0) as usize]), "lost strip vertex");
        assert!(
            has(m.positions[v as usize]),
            "collapsed vertex behind the strip"
        );
        assert!(!has(m.positions[at(4, 6) as usize]), "unchanged interior");
        assert!(!change.strip.added_edges.is_empty() && !change.strip.added_faces.is_empty());
        // The same interior collapse without a boundary change adds no witnesses.
        let interior: Vec<[u32; 3]> = m
            .triangles
            .iter()
            .map(|t| t.map(|w| if w == at(4, 5) { at(4, 4) } else { w }))
            .filter(super::surface)
            .collect();
        let change = changes(&m, &interior, &[], 0.5).unwrap();
        assert!(change.strip.lost_points.is_empty() && change.strip.added_edges.is_empty());
    }
    #[test]
    fn boundary_carriers_keep_the_free_third_vertex_without_freezing_the_whole_ring() {
        let m = ring();
        let p = m.keys[0];
        let border = [InterfaceRow::new(0, p, p, 0, 1)];
        let carriers = support_faces(&m, &border, false);
        assert_eq!(
            carriers,
            [true, false, true, false, true, false, true, false]
        );
        assert!(support_faces(&m, &border, true).iter().all(|v| *v));
        let (pins, _) = protected(&m, &border, false);
        // Both endpoints of the boundary edge are fixed, while its third corner
        // is free. Keeping the carrier itself prevents a collapse onto that edge.
        assert!(pins[2] && pins[3]);
        assert!(!pins[6]);
        assert!(carriers[4]);
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

    /// `n`×`n` unit quads in z=0, counter-clockwise, without the quads in `holes`.
    fn grid(n: u32, holes: &[(u32, u32)]) -> Merged {
        let mut m = ring();
        m.positions = (0..=n)
            .flat_map(|y| (0..=n).map(move |x| [x as f32, y as f32, 0.]))
            .collect();
        m.keys = m.positions.iter().map(|p| p.map(f32::to_bits)).collect();
        m.triangles = (0..n * n)
            .filter(|&q| !holes.contains(&(q % n, q / n)))
            .flat_map(|q| {
                let a = q / n * (n + 1) + q % n;
                [[a, a + 1, a + n + 2], [a, a + n + 2, a + n + 1]]
            })
            .collect();
        m.representative = vec![m.representative[0]; m.triangles.len()];
        m
    }

    #[test]
    fn a_closure_is_measured_on_its_own_cluster_not_the_whole_changed_component() {
        // One hole closes; a far quad is retriangulated in the same component.
        let m = grid(6, &[(1, 1)]);
        let mut output = grid(6, &[]).triangles;
        let a = 4 * 7 + 4;
        let far = [[a, a + 1, a + 8], [a, a + 8, a + 7]];
        output.retain(|t| !far.contains(t));
        output.extend([[a, a + 1, a + 7], [a + 1, a + 8, a + 7]]);
        let change = changes(&m, &output, &[], 0.).unwrap();
        assert_eq!(change.receipt.closed_holes, 1);
        let patch: Vec<_> = change.patch_faces.iter().map(|&i| output[i]).collect();
        assert_eq!(patch, [[8, 9, 16], [8, 16, 15]]);
        assert_eq!(change.closure_carriers.len(), output.len() - 2);
        assert_eq!(change.closure_centroids, [[1.5, 1.5, 0.]]);
    }

    #[test]
    fn a_closing_cluster_whose_moved_boundary_reaches_a_border_point_is_rejected() {
        // Review S1c r1 P2-1: holes (2,2) and (3,3) pinch at vertex (3,3). The output fills
        // (3,3) and removes quad (1,2), so the other hole's boundary moves onto vertex
        // (1,2): interior before, a border point. The cluster lost a cycle and touches the
        // border, which the S5 oracle refuses.
        let m = grid(8, &[(2, 2), (3, 3)]);
        let output = grid(8, &[(1, 2), (2, 2)]).triangles;
        let k = m.keys[2 * 9 + 1];
        let border = [InterfaceRow::new(0, k, k, 0, 1)];
        assert!(changes(&m, &output, &border, 0.).is_none());
        let free = changes(&m, &output, &[], 0.).unwrap();
        assert_eq!(
            (free.receipt.closed_holes, free.receipt.opened_holes),
            (1, 0)
        );
    }

    #[test]
    fn a_rank_three_closing_cluster_has_the_oracles_centroid_basis() {
        // Review S1c r2 P3-A: an asymmetric layout, so index order (y-major) and sorted
        // position order (x-major) pick different bases; a mirror-symmetric one cannot
        // tell them apart. The pre-fix order gives (2.0, 2.227..), (4, 4), (3.5, 3.5).
        // It touches the grid edge, so measure the whole boundary directly.
        // Expected: the S5 oracle's `_cycle_centroids` on the same edges.
        let m = grid(5, &[(3, 0), (4, 0), (0, 1), (4, 2), (3, 3), (2, 4)]);
        let mut edges: HashMap<(u32, u32), u32> = HashMap::default();
        for t in &m.triangles {
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        edges.retain(|_, n| *n == 1);
        let centroids = cycle_centroids(&m, &edges, &vec![true; m.keys.len()]);
        let oracle = [
            [2.423076923076923, 2.6153846153846154, 0.],
            [4., 4., 0.],
            [4.125, 4.125, 0.],
        ];
        assert_eq!(centroids.len(), oracle.len());
        for (c, o) in centroids.iter().zip(oracle) {
            assert!(
                (0..3).all(|i| (c[i] - o[i]).abs() < 1e-12),
                "{c:?} vs {o:?}"
            );
        }
    }

    #[test]
    fn a_flipping_cluster_is_split_while_a_valid_one_survives() {
        let m = grid(6, &[]);
        let id = |x: u32, y: u32| y * 7 + x;
        let mut map: Vec<u32> = (0..m.keys.len() as u32).collect();
        // A valid collapse across a diagonal, and a vertex thrown outside its star.
        map[id(1, 1) as usize] = id(2, 2);
        map[id(4, 4) as usize] = id(5, 2);
        assert!(m
            .triangles
            .iter()
            .any(|&t| { !preserves_orientation(&m.positions, t, t.map(|v| map[v as usize])) }));
        assert!(split_clusters(&m, &[], &mut map.clone(), 0).is_none());
        let split = split_clusters(&m, &[], &mut map, 1).unwrap();
        assert_eq!((split.passes, split.change.receipt.closed_holes), (1, 0));
        assert_eq!(map[id(1, 1) as usize], id(2, 2));
        assert_eq!(map[id(4, 4) as usize], id(4, 4));
        assert_eq!(split.output.len(), m.triangles.len() - 2);
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
            &[
                InterfaceRow::new(0, a, middle, 0, 1),
                InterfaceRow::new(0, middle, b, 0, 1),
            ],
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
