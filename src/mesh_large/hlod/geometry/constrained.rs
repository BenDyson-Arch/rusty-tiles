//! Constructive cap fit for consistently projected manifold patches. Keep boundary loops
//! and mandatory interface items, discard source interior sampling, and retriangulate.
//! Outside this domain return to the bounded general remesher without changing the mesh.
use super::{topology, Merged};
use crate::mesh_large::hlod::interfaces::InterfaceRow;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

fn edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

pub(super) fn remesh(
    m: &mut Merged,
    interface: &[InterfaceRow],
    cap: usize,
) -> Option<topology::Receipt> {
    let components = topology::components(m);
    let mut patches: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, t) in m.triangles.iter().enumerate() {
        patches
            .entry(components[t[0] as usize])
            .or_default()
            .push(i);
    }
    let ids: HashMap<_, _> = m
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u32))
        .collect();
    if patches.len() > cap {
        return None;
    }
    let mut required_by_component: HashMap<usize, Vec<(u32, u32)>> = HashMap::new();
    for r in interface {
        let (a, b) = (*ids.get(&r.a)?, *ids.get(&r.b)?);
        if components[a as usize] != components[b as usize] {
            return None;
        }
        required_by_component
            .entry(components[a as usize])
            .or_default()
            .push((a, b));
    }
    let mut output = Vec::new();
    let mut representatives = Vec::new();
    let mut count = 0;
    for (component, faces) in patches {
        let mut edges: BTreeMap<(u32, u32), (u32, i32)> = BTreeMap::new();
        let mut sum = [0f64; 3];
        for &i in &faces {
            let t = m.triangles[i];
            let normal = topology::normal(&m.positions, t);
            for a in 0..3 {
                sum[a] += normal[a];
            }
            for a in 0..3 {
                let (v, w) = (t[a], t[(a + 1) % 3]);
                if v == w {
                    return None;
                }
                let (n, sign) = edges.entry(edge(v, w)).or_default();
                *n += 1;
                *sign += if v < w { 1 } else { -1 };
            }
        }
        if edges
            .values()
            .any(|&(n, sign)| n > 2 || (n == 2 && sign != 0))
        {
            return None;
        }
        let axis = (0..3).max_by(|&a, &b| sum[a].abs().total_cmp(&sum[b].abs()))?;
        if sum[axis] == 0. {
            return None;
        }
        if faces
            .iter()
            .any(|&i| topology::normal(&m.positions, m.triangles[i])[axis] * sum[axis] <= 0.)
        {
            return None;
        }
        let boundary: BTreeSet<_> = edges
            .iter()
            .filter(|(_, v)| v.0 == 1)
            .map(|(e, _)| *e)
            .collect();
        if boundary.is_empty() {
            return None;
        }
        let mut adjacent: HashMap<u32, Vec<u32>> = HashMap::new();
        for &(a, b) in &boundary {
            adjacent.entry(a).or_default().push(b);
            adjacent.entry(b).or_default().push(a);
        }
        if adjacent.values().any(|v| v.len() != 2) {
            return None;
        }
        let mut visited = HashSet::new();
        let mut loops = 0;
        for &start in adjacent.keys() {
            if visited.contains(&start) {
                continue;
            }
            loops += 1;
            let (mut previous, mut at) = (u32::MAX, start);
            loop {
                if !visited.insert(at) {
                    if at != start {
                        return None;
                    }
                    break;
                }
                let n = &adjacent[&at];
                let next = if n[0] == previous { n[1] } else { n[0] };
                previous = at;
                at = next;
            }
        }
        let mut vertices: BTreeSet<u32> = adjacent.keys().copied().collect();
        let mut constraints = boundary.clone();
        for &(a, b) in required_by_component.get(&component).into_iter().flatten() {
            vertices.extend([a, b]);
            if a != b {
                constraints.insert(edge(a, b));
            }
        }
        let predicted = 2 * (vertices.len() - adjacent.len()) + adjacent.len() + 2 * loops - 4;
        if output.len() + predicted > cap {
            return None;
        }
        let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
        let mut handles = HashMap::new();
        let mut reverse = HashMap::new();
        for v in vertices {
            let p = m.positions[v as usize];
            let handle = cdt
                .insert(Point2::new(
                    f64::from(p[(axis + 1) % 3]),
                    f64::from(p[(axis + 2) % 3]),
                ))
                .ok()?;
            if reverse.insert(handle, v).is_some() {
                return None;
            }
            handles.insert(v, handle);
        }
        for &(a, b) in &constraints {
            if !cdt.can_add_constraint(handles[&a], handles[&b]) {
                return None;
            }
            cdt.add_constraint(handles[&a], handles[&b]);
        }
        let mut flags = HashMap::new();
        let mut queue = VecDeque::new();
        for e in cdt.directed_edges().filter(|e| e.face().is_outer()) {
            if let Some(face) = e.rev().face().as_inner() {
                let h = face.fix();
                let inside =
                    boundary.contains(&edge(reverse[&e.from().fix()], reverse[&e.to().fix()]));
                if let Some(old) = flags.insert(h, inside) {
                    if old != inside {
                        return None;
                    }
                } else {
                    queue.push_back(h);
                }
            }
        }
        while let Some(h) = queue.pop_front() {
            let inside = flags[&h];
            for e in cdt.face(h).adjacent_edges() {
                if let Some(f) = e.rev().face().as_inner() {
                    let next = f.fix();
                    let value = inside
                        ^ boundary
                            .contains(&edge(reverse[&e.from().fix()], reverse[&e.to().fix()]));
                    if let Some(old) = flags.insert(next, value) {
                        if old != value {
                            return None;
                        }
                    } else {
                        queue.push_back(next);
                    }
                }
            }
        }
        let start = output.len();
        let mut area = 0.;
        for f in cdt
            .inner_faces()
            .filter(|f| flags.get(&f.fix()) == Some(&true))
        {
            let mut t = f.vertices().map(|v| reverse[&v.fix()]);
            let normal = topology::normal(&m.positions, t)[axis];
            if normal * sum[axis] < 0. {
                t.swap(1, 2);
            }
            area += normal.abs();
            output.push(t);
            representatives.push(m.representative[faces[0]]);
        }
        // Positive input Jacobians plus a simple boundary of degree one establish the
        // height-field domain. The area identity detects inconsistent ring orientation.
        if output.len() - start != predicted
            || (area - sum[axis].abs()).abs() > 1e-10 * area.max(1.)
        {
            return None;
        }
        count += 1;
    }
    let required: HashSet<_> = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .map(|r| (r.a, r.b))
        .collect();
    let points = interface.iter().flat_map(|r| [r.a, r.b]).collect();
    let lost = super::missing(&output, &m.keys, &required, &points);
    if !lost.0.is_empty() || !lost.1.is_empty() {
        return None;
    }
    let change = topology::changes(m, &output, interface, 0.)?;
    if change.receipt.closed_holes != 0 {
        return None;
    }
    let mut receipt = change.receipt;
    receipt.constrained_remeshes = count;
    m.triangles = output;
    m.representative = representatives;
    Some(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::hlod::{interfaces::key, store::SourcePrimitive};
    fn sheet(n: u32, hole: bool) -> Merged {
        let positions: Vec<_> = (0..=n)
            .flat_map(|y| (0..=n).map(move |x| [x as f32, y as f32, 0.]))
            .collect();
        let mut triangles = Vec::new();
        for y in 0..n {
            for x in 0..n {
                if hole && x == n / 2 && y == n / 2 {
                    continue;
                }
                let a = y * (n + 1) + x;
                triangles.extend([[a, a + 1, a + n + 2], [a, a + n + 2, a + n + 1]]);
            }
        }
        Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            positions,
            representative: vec![
                SourcePrimitive {
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
    fn planar_cap_fit_depends_on_boundary_and_holes_not_interior_density() {
        for hole in [false, true] {
            let mut m = sheet(32, hole);
            let p = m.keys[(16 * 33 + 16) as usize];
            let interface = vec![InterfaceRow::new(0, p, p, 0, 1)];
            let result = remesh(&mut m, &interface, 140).unwrap();
            assert_eq!(result.constrained_remeshes, 1);
            assert_eq!(result.closed_holes, 0);
            assert_eq!(m.triangles.len(), if hole { 132 } else { 128 });
            assert!(m
                .triangles
                .iter()
                .flatten()
                .any(|v| m.keys[*v as usize] == p));
        }
    }
    #[test]
    fn insufficient_cap_and_folded_projection_leave_input_unchanged() {
        let mut m = sheet(8, false);
        let before = m.triangles.clone();
        assert!(remesh(&mut m, &[], 2).is_none());
        assert_eq!(m.triangles, before);
        m.triangles[0].swap(0, 1);
        let before = m.triangles.clone();
        assert!(remesh(&mut m, &[], 8192).is_none());
        assert_eq!(m.triangles, before);
    }
}
