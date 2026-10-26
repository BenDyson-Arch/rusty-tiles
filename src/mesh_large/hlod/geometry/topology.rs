//! Bounded, tile-local boundary reconstruction and topology cleanup. The source graph
//! supplies the boundary; this module never chooses or moves a contracted endpoint.
use super::{missing, oriented, Merged};
use crate::{
    mesh_large::hlod::interfaces::{InterfaceRow, Key},
    JobError,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub contracted_vertices: u64,
    pub reconstructed_faces: u64,
    pub duplicate_faces: u64,
    pub dropped_components: u64,
    pub closed_holes: u64,
    pub remeshed_vertices: u64,
    pub remesh_rounds: u64,
}
impl Receipt {
    pub(crate) fn add(&mut self, r: Self) {
        self.contracted_vertices += r.contracted_vertices;
        self.reconstructed_faces += r.reconstructed_faces;
        self.duplicate_faces += r.duplicate_faces;
        self.dropped_components += r.dropped_components;
        self.closed_holes += r.closed_holes;
        self.remeshed_vertices += r.remeshed_vertices;
        self.remesh_rounds += r.remesh_rounds;
    }
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
fn points(interface: &[InterfaceRow]) -> HashSet<Key> {
    interface.iter().flat_map(|r| [r.a, r.b]).collect()
}
fn surface(t: &[u32; 3]) -> bool {
    t[0] != t[1] && t[1] != t[2] && t[0] != t[2]
}
fn normal(p: &[[f32; 3]], t: [u32; 3]) -> [f64; 3] {
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
    mapping: &dyn Fn(Key) -> Result<Option<(Key, Key, f64, u8)>, JobError>,
    receipt: &mut Receipt,
) -> Result<(), JobError> {
    let mut ids: HashMap<Key, u32> = m
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u32))
        .collect();
    let mut map: Vec<u32> = (0..m.keys.len() as u32).collect();
    for v in 0..map.len() {
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
            map[v] = id;
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
    // Preserve the surface support of each original border-connected component even
    // when all its triangles lie in strips collapsed by a boundary contraction.
    let required = points(interface);
    let segments = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .map(|r| (r.a, r.b))
        .collect();
    let (lost_edges, lost_points) = missing(&m.triangles, &m.keys, &segments, &required);
    let mut candidates: HashMap<u32, (usize, u32)> = HashMap::new();
    for (i, t) in original.iter().enumerate() {
        for &v in t {
            candidates.entry(map[v as usize]).or_insert((i, v));
        }
    }
    let mut witness = HashMap::new();
    for (i, t) in original.iter().enumerate() {
        witness.entry(component[t[0] as usize]).or_insert(i);
    }
    for (a, b) in lost_edges
        .into_iter()
        .chain(lost_points.into_iter().map(|p| (p, p)))
    {
        let (a, b) = (ids[&a], ids[&b]);
        let i = candidates
            .get(&a)
            .or_else(|| candidates.get(&b))
            .map(|v| v.0)
            .ok_or_else(|| super::state("contracted border endpoint has no source component"))?;
        // Pick a source vertex of this component distinct from the chord endpoints.
        // Keep the original canonical position as interior support, never as a new arc vertex.
        let root = component[original[i][0] as usize];
        let support = original[witness[&root]]
            .iter()
            .copied()
            .find(|v| *v != a && *v != b)
            .unwrap_or(original[i][0]);
        m.triangles.push([a, b, support]);
        m.representative.push(m.representative[i]);
        receipt.reconstructed_faces += 1;
    }
    // Strip faces with repeated corners are redundant unless they are the sole remaining
    // support for a component. Retain one such face for a genuinely zero-area component.
    let mut alive = HashSet::new();
    for (i, t) in m.triangles.iter().take(original.len()).enumerate() {
        if surface(t) {
            alive.insert(component[original[i][0] as usize]);
        }
    }
    let mut kept = HashSet::new();
    let keep: Vec<bool> = m
        .triangles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i >= original.len() || surface(t) {
                return true;
            }
            let c = component[original[i][0] as usize];
            !alive.contains(&c) && kept.insert(c)
        })
        .collect();
    retain(m, &keep);
    Ok(())
}

/// Duplicated oriented soup adds no surface. Only components with NO border contact and
/// strict diameter bound below the known G floor can be dropped. Keep one component if
/// every component is eligible, so the parent still has geometry to measure against.
pub(super) fn cleanup(m: &mut Merged, interface: &[InterfaceRow], g: f64, r: &mut Receipt) {
    let mut seen = HashSet::new();
    let keep: Vec<bool> = m
        .triangles
        .iter()
        .map(|t| seen.insert(oriented(t)))
        .collect();
    r.duplicate_faces += keep.iter().filter(|k| !**k).count() as u64;
    retain(m, &keep);
    let component = Components::mesh(m);
    let border = points(interface);
    let mut boxes: HashMap<usize, ([f64; 3], [f64; 3], bool)> = HashMap::new();
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

/// Boundary edge components touching a delivered arc or live junction are protected in
/// full. A free vertex never maps onto a protected one, so each such boundary edge keeps
/// its one incident face. No touching hole can close during interior clustering.
fn protected(m: &Merged, interface: &[InterfaceRow]) -> (Vec<bool>, usize) {
    let border = points(interface);
    let mut locked: Vec<bool> = m.keys.iter().map(|k| border.contains(k)).collect();
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
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
    let mut touched = HashSet::new();
    for (v, &lock) in locked.iter().enumerate() {
        if lock {
            touched.insert(boundary.root(v));
        }
    }
    let mut loops = HashSet::new();
    for (&(a, b), &n) in &edges {
        if n == 1 {
            let root = boundary.root(a as usize);
            loops.insert(root);
            if touched.contains(&root) {
                locked[a as usize] = true;
                locked[b as usize] = true;
            }
        }
    }
    // One fixed support triangle per component prevents deletion of any surface component.
    let component = Components::mesh(m);
    let mut supported = HashSet::new();
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
    let (locked, before) = protected(m, interface);
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
    let mut cells = HashMap::new();
    let mut map = Vec::with_capacity(m.keys.len());
    for (v, p) in m.positions.iter().enumerate() {
        if locked[v] {
            map.push(v as u32);
            continue;
        }
        let cell: [i64; 3] =
            std::array::from_fn(|a| ((f64::from(p[a]) - lo[a]) / width).floor() as i64);
        let id = *cells.entry((component[v], cell)).or_insert(v as u32);
        r.remeshed_vertices += u64::from(id != v as u32);
        map.push(id);
    }
    for t in &mut m.triangles {
        *t = mapped(&m.positions, *t, &map);
    }
    let mut seen = HashSet::new();
    let keep = m
        .triangles
        .iter()
        .map(|t| surface(t) && seen.insert(oriented(t)))
        .collect::<Vec<_>>();
    retain(m, &keep);
    let (_, after) = protected(m, interface);
    r.closed_holes += before.saturating_sub(after) as u64;
    r.remesh_rounds += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut r = Receipt::default();
        cleanup(&mut m, &interface, 1., &mut r);
        assert_eq!(m.triangles, vec![[0, 1, 2]]);
        assert_eq!(r.dropped_components, 1);
    }
}
