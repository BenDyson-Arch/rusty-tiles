//! Constructive cap fit for consistently projected manifold patches. Keep boundary loops
//! and mandatory interface items, discard source interior sampling, and retriangulate.
//! Outside this domain return to the bounded general remesher without changing the mesh.
use super::{topology, Merged};
use crate::mesh_large::hlod::interfaces::InterfaceRow;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use spade::{ConstrainedDelaunayTriangulation, Point2, PositionInTriangulation, Triangulation};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

pub(super) fn remesh(
    m: &mut Merged,
    interface: &[InterfaceRow],
    cap: usize,
) -> Option<topology::Receipt> {
    triangulate(m, interface, cap, None, &[])
}

/// Reconstruct contracted patches with all unaffected interior samples. In particular,
/// no moved face is reoriented to conceal an inversion, and no distant support vertex is
/// chosen to invent a replacement face. Nonmanifold edges separate their surface sheets.
pub(super) fn contract(
    m: &mut Merged,
    interface: &[InterfaceRow],
    map: &[u32],
    endpoints: &[u32],
) -> Option<topology::Receipt> {
    triangulate(m, interface, usize::MAX, Some(map), endpoints)
}

fn triangulate(
    m: &mut Merged,
    interface: &[InterfaceRow],
    cap: usize,
    mapping: Option<&[u32]>,
    endpoints: &[u32],
) -> Option<topology::Receipt> {
    let components = topology::components(m);
    let mut patches: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    // Freeze the surface beyond the first interior ring. Include both removed vertices
    // and retained chord endpoints: an interior sample on the chord may need its two
    // endpoint faces rebuilt as well. Their unmoved edges form the constrained ring.
    let mut touched = vec![false; m.positions.len()];
    for &v in endpoints {
        touched[v as usize] = true;
    }
    if let Some(map) = mapping {
        for (v, &to) in map.iter().enumerate() {
            if v != to as usize {
                touched[v] = true;
                touched[to as usize] = true;
            }
        }
    }
    let affected: Vec<bool> = m
        .triangles
        .iter()
        .map(|t| t.iter().any(|&v| touched[v as usize]))
        .collect();
    let face_patches = if mapping.is_some() {
        manifold_patches(m, &affected)
    } else {
        Vec::new()
    };
    for (i, t) in m.triangles.iter().enumerate() {
        patches
            .entry(if mapping.is_some() {
                face_patches[i]
            } else {
                components[t[0] as usize]
            })
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
    let mut required = Vec::new();
    for r in interface {
        let (a, b) = (*ids.get(&r.a)?, *ids.get(&r.b)?);
        if mapping.is_none() && components[a as usize] != components[b as usize] {
            return None;
        }
        required.push((a, b));
    }
    let mut output = Vec::new();
    let mut representatives = Vec::new();
    let mut count = 0;
    for (_, faces) in patches {
        if mapping.is_some() && faces.iter().all(|&i| !affected[i]) {
            output.extend(faces.iter().map(|&i| m.triangles[i]));
            representatives.extend(faces.iter().map(|&i| m.representative[i]));
            continue;
        }
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
        let mut boundary: BTreeSet<_> = edges
            .iter()
            .filter(|(_, v)| v.0 == 1)
            .map(|(e, _)| *e)
            .collect();
        if let Some(map) = mapping {
            let mut mapped = BTreeMap::<(u32, u32), usize>::new();
            for &(a, b) in &boundary {
                let (a, b) = (map[a as usize], map[b as usize]);
                if a != b {
                    *mapped.entry(edge(a, b)).or_default() += 1;
                }
            }
            // A multiply covered boundary is not a simple projected patch.
            if mapped.values().any(|&n| n != 1) {
                return None;
            }
            boundary = mapped.into_keys().collect();
        }
        if boundary.is_empty() {
            return None;
        }
        let mut adjacent: HashMap<u32, Vec<u32>> = HashMap::default();
        for &(a, b) in &boundary {
            adjacent.entry(a).or_default().push(b);
            adjacent.entry(b).or_default().push(a);
        }
        if adjacent.values().any(|v| v.len() != 2) {
            return None;
        }
        let mut visited = HashSet::default();
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
        let present: HashSet<u32> = faces
            .iter()
            .flat_map(|&i| m.triangles[i])
            .map(|v| mapping.map_or(v, |map| map[v as usize]))
            .collect();
        for &(a, b) in required
            .iter()
            .filter(|&&(a, b)| present.contains(&a) && present.contains(&b))
        {
            vertices.extend([a, b]);
            if a != b {
                constraints.insert(edge(a, b));
            }
        }
        let mut predicted = 2 * (vertices.len() - adjacent.len()) + adjacent.len() + 2 * loops - 4;
        if output.len() + predicted > cap {
            return None;
        }
        let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
        let mut handles = HashMap::default();
        let mut reverse = HashMap::default();
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
        if let Some(map) = mapping {
            let interior: BTreeSet<u32> = faces
                .iter()
                .flat_map(|&i| m.triangles[i])
                .filter(|&v| map[v as usize] == v && !handles.contains_key(&v))
                .collect();
            for v in interior {
                let p = m.positions[v as usize];
                let p = Point2::new(f64::from(p[(axis + 1) % 3]), f64::from(p[(axis + 2) % 3]));
                // A collinear interior sample must not split a delivered border chord.
                if let PositionInTriangulation::OnEdge(e) = cdt.locate(p) {
                    if cdt.directed_edge(e).is_constraint_edge() {
                        continue;
                    }
                }
                let handle = cdt.insert(p).ok()?;
                if reverse.insert(handle, v).is_some() {
                    return None;
                }
                predicted += 2;
            }
        }
        let mut flags = HashMap::default();
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
        if (mapping.is_none() && output.len() - start != predicted)
            || (mapping.is_none() && (area - sum[axis].abs()).abs() > 1e-10 * area.max(1.))
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
    if change.receipt.closed_holes != 0 || change.receipt.opened_holes != 0 {
        return None;
    }
    let mut receipt = change.receipt;
    receipt.constrained_remeshes = count;
    m.triangles = output;
    m.representative = representatives;
    Some(receipt)
}

fn manifold_patches(m: &Merged, affected: &[bool]) -> Vec<usize> {
    let faces = &m.triangles;
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut edges = HashMap::<(u32, u32), (usize, usize, usize, i32)>::default();
    for (i, t) in faces.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let e = edges.entry(edge(a, b)).or_insert((i, i, 0, 0));
            e.1 = i;
            e.2 += 1;
            e.3 += if a < b { 1 } else { -1 };
        }
    }
    let mut parent: Vec<_> = (0..faces.len()).collect();
    for &(a, b, count, direction) in edges.values() {
        // A folded book can be manifold along a two-page spine, yet cannot be one
        // projected patch. Dominant-axis/sign charts split such folds deterministically.
        let chart = |i: usize| {
            let n = topology::normal(&m.positions, faces[i]);
            let axis = (0..3)
                .max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs()))
                .unwrap();
            (axis, n[axis] > 0.)
        };
        if count == 2 && direction == 0 && affected[a] && affected[b] && chart(a) == chart(b) {
            let (a, b) = (root(&mut parent, a), root(&mut parent, b));
            parent[a.max(b)] = a.min(b);
        }
    }
    (0..faces.len()).map(|i| root(&mut parent, i)).collect()
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
    fn contracted_strip_workspace_fits_the_parent_charge() {
        let mut m = sheet(181, false); // 65,522 faces: almost eight capped children.
        let input = m.triangles.len();
        let mut map: Vec<_> = (0..m.positions.len() as u32).collect();
        for (v, target) in map.iter_mut().enumerate().take(181).skip(1) {
            *target = if v <= 90 { 0 } else { 181 };
        }
        let interface = [InterfaceRow::new(0, m.keys[0], m.keys[181], 0, 1)];
        let (receipt, peak) =
            crate::test_alloc::peak(|| contract(&mut m, &interface, &map, &[0, 181]));
        assert!(receipt.is_some());
        let charge = input as u64 * super::super::super::JOB_BYTES_PER_TRIANGLE;
        eprintln!("CONTRACTION workspace {peak} bytes / {input} faces, charge {charge}");
        assert!(peak as u64 <= charge);
    }

    #[test]
    fn interior_point_on_new_chord_does_not_make_a_sliver_or_split_the_chord() {
        let positions = vec![
            [0., 0., 0.],
            [1., 0.1, 0.],
            [2., 0., 0.],
            [1., 0., 0.],
            [1., -1., 0.],
        ];
        let triangles = vec![[0, 1, 3], [1, 2, 3], [0, 3, 4], [3, 2, 4]];
        let mut m = Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            representative: vec![sheet(1, false).representative[0]; triangles.len()],
            positions,
            triangles,
        };
        let interface = [InterfaceRow::new(0, m.keys[0], m.keys[2], 0, 1)];
        contract(&mut m, &interface, &[0, 0, 2, 3, 4], &[0, 2]).unwrap();
        assert_eq!(m.triangles.len(), 1);
        let t = m.triangles[0];
        assert!(t.contains(&0) && t.contains(&2) && t.contains(&4));
        assert!(topology::normal(&m.positions, t)[2] < 0.);
    }

    #[test]
    fn contracted_strip_retriangulates_a_fold_and_freezes_the_interior_ring() {
        // Snapping m to a in (m,q,p) reverses its sign. Flipping that face would
        // double-cover the same side of a-p. The constrained strip has area 1.35.
        let positions = vec![
            [0., 0., 0.],   // a
            [1., 0., 0.],   // m
            [3., 0., 0.],   // b
            [0.5, 0.1, 0.], // p
            [2., 1., 0.],   // q
            [1., 2., 3.],   // unchanged curved interior beyond p-q
        ];
        let triangles = vec![[0, 1, 3], [1, 4, 3], [1, 2, 4], [3, 4, 5]];
        let mut m = Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            representative: vec![sheet(1, false).representative[0]; triangles.len()],
            positions,
            triangles,
        };
        let area: f64 = m.triangles[..3]
            .iter()
            .map(|&t| topology::normal(&m.positions, t)[2])
            .sum();
        let interface = [InterfaceRow::new(0, m.keys[0], m.keys[2], 0, 1)];
        let receipt = contract(&mut m, &interface, &[0, 0, 2, 3, 4, 5], &[0, 2]).unwrap();
        assert!(receipt.constrained_remeshes > 0);
        assert!(m.triangles.contains(&[3, 4, 5]), "curved interior moved");
        let strip: Vec<_> = m
            .triangles
            .iter()
            .copied()
            .filter(|t| !t.contains(&5))
            .collect();
        assert_eq!(strip.len(), 2);
        assert!(strip
            .iter()
            .all(|&t| topology::normal(&m.positions, t)[2] > 0.));
        let after: f64 = strip
            .iter()
            .map(|&t| topology::normal(&m.positions, t)[2])
            .sum();
        assert!((area - after).abs() < 1e-12);
        assert!(strip.iter().any(|t| t.contains(&0) && t.contains(&2)));
        let mut counts = BTreeMap::<(u32, u32), [u32; 2]>::new();
        for t in &m.triangles {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                counts.entry(edge(a, b)).or_default()[usize::from(a > b)] += 1;
            }
        }
        assert!(counts.values().all(|c| c[0] <= 1 && c[1] <= 1));
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
