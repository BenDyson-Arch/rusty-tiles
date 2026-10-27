//! Construct a star fan using only a sheet's own source vertices and exact boundary.
//! This bounded domain excludes holes, projected folds and mandatory interior edges.
use super::{edge, topology, HashMap, HashSet, InterfaceRow, Merged};
use std::collections::{BTreeMap, BTreeSet};

type Chart = (usize, bool);

/// One fixed alternative to the CDT's interior diagonals, rooted at a retained
/// endpoint of the local contracted arc. These are auxiliary constraints only;
/// the caller still inserts every unmoved interior source sample afterwards.
pub(super) fn chord_spokes(
    m: &Merged,
    boundary: &[(u32, u32)],
    endpoints: &[bool],
    axis: usize,
    positive: bool,
) -> Option<Vec<(u32, u32)>> {
    let mut next = BTreeMap::new();
    let mut incoming = HashSet::default();
    for &(a, b) in boundary {
        if next.insert(a, b).is_some() || !incoming.insert(b) {
            return None;
        }
    }
    let start = *next.first_key_value()?.0;
    let mut at = start;
    for step in 0..boundary.len() {
        at = *next.get(&at)?;
        if at == start && step + 1 != boundary.len() {
            return None;
        }
    }
    if at != start || incoming.iter().any(|v| !next.contains_key(v)) {
        return None;
    }
    let mut center = [0.; 2];
    for &v in next.keys() {
        for (k, x) in center.iter_mut().enumerate() {
            *x += f64::from(m.positions[v as usize][(axis + 1 + k) % 3]);
        }
    }
    for x in &mut center {
        *x /= next.len() as f64;
    }
    // Prove that this oriented boundary is geometrically simple before using a
    // boundary vertex as a fan root. The centroid is only a proof point, never an
    // output vertex. Strict wedges and one winding rule out multiple covers.
    let sign = if positive { 1. } else { -1. };
    let mut turns = 0;
    for &(a, b) in boundary {
        let point = |v: u32| {
            std::array::from_fn::<_, 2, _>(|k| {
                f64::from(m.positions[v as usize][(axis + 1 + k) % 3]) - center[k]
            })
        };
        let (a, b) = (point(a), point(b));
        if (a[0] * b[1] - a[1] * b[0]) * sign <= 0. {
            return None;
        }
        if a[1] * sign <= 0. && b[1] * sign > 0. {
            turns += 1;
        }
    }
    if turns != 1 {
        return None;
    }
    let mut choices = [None::<(f64, u32)>; 2];
    for &v in next.keys().filter(|&&v| endpoints[v as usize]) {
        let distance = (0..2)
            .map(|k| (f64::from(m.positions[v as usize][(axis + 1 + k) % 3]) - center[k]).powi(2))
            .sum::<f64>();
        let mut candidate = (distance, v);
        for slot in &mut choices {
            if slot.is_none_or(|old| {
                candidate
                    .0
                    .total_cmp(&old.0)
                    .then(candidate.1.cmp(&old.1))
                    .is_lt()
            }) {
                let old = slot.replace(candidate);
                let Some(old) = old else { break };
                candidate = old;
            }
        }
    }
    for (_, root) in choices.into_iter().flatten() {
        if boundary.iter().all(|&(a, b)| {
            a == root || b == root || topology::normal(&m.positions, [a, b, root])[axis] * sign > 0.
        }) {
            return Some(
                next.keys()
                    .copied()
                    .filter(|&v| v != root)
                    .map(|v| edge(root, v))
                    .collect(),
            );
        }
    }
    None
}

pub(super) fn remesh(
    m: &mut Merged,
    interface: &[InterfaceRow],
    cap: usize,
    guard: &super::super::folds::Source,
) -> Option<topology::Receipt> {
    // Count incidences within a chart, before joining faces. Literal up/down
    // source pairs then belong to distinct sheets even at zero separation.
    let panels = panels(m).or_else(|| super::rejected("fan", "source panel extraction refused"))?;
    let ids: HashMap<_, _> = m
        .keys
        .iter()
        .enumerate()
        .map(|(v, &k)| (k, v as u32))
        .collect();
    let mut required_edges = HashSet::default();
    let mut required_points = HashSet::default();
    for r in interface {
        let (a, b) = match (ids.get(&r.a), ids.get(&r.b)) {
            (Some(&a), Some(&b)) => (a, b),
            _ => return super::rejected("fan", "required endpoint missing from source"),
        };
        required_points.extend([a, b]);
        if a != b {
            required_edges.insert(edge(a, b));
        }
    }
    let mut output = Vec::new();
    let mut representatives = Vec::new();
    let mut used_centers = HashSet::default();
    let count = panels.len() as u64;
    for (chart, faces) in panels {
        let mut edges = BTreeMap::<(u32, u32), (usize, i32)>::new();
        let mut vertices = BTreeSet::new();
        let sign = if chart.1 { 1. } else { -1. };
        for &i in &faces {
            let t = m.triangles[i];
            if topology::normal(&m.positions, t)[chart.0] * sign <= 0. {
                return super::rejected("fan", "inconsistent source panel projection");
            }
            vertices.extend(t);
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let e = edges.entry(edge(a, b)).or_default();
                e.0 += 1;
                e.1 += if a < b { 1 } else { -1 };
            }
        }
        if edges.values().any(|&(n, d)| n > 2 || (n == 2 && d != 0)) {
            return super::rejected("fan", "nonmanifold or same-direction panel edge");
        }
        if edges
            .iter()
            .any(|(e, &(n, _))| n != 1 && required_edges.contains(e))
        {
            return super::rejected("fan", "required segment lies in panel interior");
        }
        let boundary: Vec<_> = edges
            .iter()
            .filter_map(|(&(a, b), &(n, d))| {
                (n == 1).then_some(if d > 0 { (a, b) } else { (b, a) })
            })
            .collect();
        if boundary.len() < 3 || output.len().checked_add(boundary.len())? > cap {
            return super::rejected("fan", "boundary too small or whole output cap exceeded");
        }
        #[cfg(test)]
        if std::env::var_os("RT_HLOD_FOLD_REPLAY").is_some() {
            eprintln!("CONSTRAINED fan panel chart={chart:?} faces={} vertices={} boundary={} output={} cap={cap}",
                faces.len(), vertices.len(), boundary.len(), output.len());
        }
        let mut next = BTreeMap::new();
        let mut incoming = HashSet::default();
        for &(a, b) in &boundary {
            if next.insert(a, b).is_some() || !incoming.insert(b) {
                return super::rejected("fan", "branching boundary");
            }
        }
        // Exactly one oriented cycle, with in/out degree one at every boundary
        // vertex. Geometric simplicity is established by the star proof below.
        let start = *next.first_key_value()?.0;
        let mut at = start;
        for step in 0..boundary.len() {
            at = *next.get(&at)?;
            if at == start && step + 1 != boundary.len() {
                return super::rejected(
                    "fan",
                    "multiple boundary cycles (hole or disconnected loop)",
                );
            }
        }
        if at != start || incoming.iter().any(|v| !next.contains_key(v)) {
            return super::rejected("fan", "boundary is not one closed oriented cycle");
        }
        let mut center = [0.; 2];
        for &v in next.keys() {
            for (k, value) in center.iter_mut().enumerate() {
                *value += f64::from(m.positions[v as usize][(chart.0 + 1 + k) % 3]);
            }
        }
        for value in &mut center {
            *value /= next.len() as f64;
        }
        // Keep only two candidates during one scan. Trying these two fixed local
        // source vertices costs at most 2B fan checks, never candidates x boundary.
        let mut candidates = [None::<(f64, u32)>; 2];
        let mut mandatory = None;
        for v in vertices.into_iter().filter(|v| !next.contains_key(v)) {
            if required_points.contains(&v) && mandatory.replace(v).is_some() {
                return super::rejected("fan", "multiple required interior points");
            }
            if used_centers.contains(&m.keys[v as usize]) {
                continue;
            }
            let p = m.positions[v as usize];
            let distance = (0..2)
                .map(|k| (f64::from(p[(chart.0 + 1 + k) % 3]) - center[k]).powi(2))
                .sum::<f64>();
            let mut candidate = (distance, v);
            for slot in &mut candidates {
                if slot.is_none_or(|old| {
                    candidate
                        .0
                        .total_cmp(&old.0)
                        .then(candidate.1.cmp(&old.1))
                        .is_lt()
                }) {
                    let previous = slot.replace(candidate);
                    let Some(previous) = previous else { break };
                    candidate = previous;
                }
            }
        }
        let choices = if let Some(v) = mandatory {
            if used_centers.contains(&m.keys[v as usize]) {
                return super::rejected("fan", "required center already used by another sheet");
            }
            [Some((0., v)), None]
        } else {
            candidates
        };
        let mut accepted = None;
        for (_, v) in choices.into_iter().flatten() {
            if star(m, &boundary, v, chart) {
                accepted = Some(v);
                break;
            }
        }
        let v = accepted
            .or_else(|| super::rejected("fan", "no eligible fixed center passed star proof"))?;
        used_centers.insert(m.keys[v as usize]);
        output.extend(boundary.iter().map(|&(a, b)| [a, b, v]));
        representatives.extend(std::iter::repeat_n(
            m.representative[faces[0]],
            boundary.len(),
        ));
    }
    // Release construction-only indexes before the topology and Source workspaces.
    drop((ids, required_edges, required_points, used_centers));
    // The local star proof does not compare wings from different panels. Keep
    // original boundary incidence limits and require changed shared-border wings
    // to remain nonacute, using the original source rather than the proposal.
    if guard
        .remesh_violations(&m.positions, &output)
        .iter()
        .any(|v| *v)
    {
        return super::rejected(
            "fan",
            "changed border fan violates original source sides or counts",
        );
    }
    super::finish(m, interface, output, representatives, guard, count, true)
}

fn star(m: &Merged, boundary: &[(u32, u32)], center: u32, chart: Chart) -> bool {
    let sign = if chart.1 { 1. } else { -1. };
    let mut turns = 0;
    for &(a, b) in boundary {
        if topology::normal(&m.positions, [a, b, center])[chart.0] * sign <= 0. {
            return false;
        }
        let coordinate = (chart.0 + 2) % 3;
        let a = (f64::from(m.positions[a as usize][coordinate])
            - f64::from(m.positions[center as usize][coordinate]))
            * sign;
        let b = (f64::from(m.positions[b as usize][coordinate])
            - f64::from(m.positions[center as usize][coordinate]))
            * sign;
        if a <= 0. && b > 0. {
            turns += 1;
        }
    }
    // Positive projected fan faces give strictly increasing polar angles with
    // each step < pi. One positive-ray crossing establishes exactly one turn:
    // their angular intervals partition the circle, proving no fan overlap or
    // boundary self-intersection. Multiple windings must not pass as star-shaped.
    turns == 1
}

fn panels(m: &Merged) -> Option<Vec<(Chart, Vec<usize>)>> {
    fn root(parent: &mut [usize], mut v: usize) -> usize {
        while parent[v] != v {
            parent[v] = parent[parent[v]];
            v = parent[v];
        }
        v
    }
    let mut charts = Vec::with_capacity(m.triangles.len());
    let mut edges = BTreeMap::<((u32, u32), Chart), (usize, usize, usize, i32)>::new();
    for (i, &t) in m.triangles.iter().enumerate() {
        let n = topology::normal(&m.positions, t);
        let axis = (0..3).max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs()))?;
        if n[axis] == 0. {
            return None;
        }
        let chart = (axis, n[axis] > 0.);
        charts.push(chart);
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let e = edges.entry((edge(a, b), chart)).or_insert((i, i, 0, 0));
            e.1 = i;
            e.2 += 1;
            e.3 += if a < b { 1 } else { -1 };
        }
    }
    let mut parent: Vec<_> = (0..m.triangles.len()).collect();
    for &(a, b, n, d) in edges.values() {
        if n == 2 && d == 0 {
            let (a, b) = (root(&mut parent, a), root(&mut parent, b));
            parent[a.max(b)] = a.min(b);
        }
    }
    let mut panels = BTreeMap::<usize, Vec<usize>>::new();
    for i in 0..m.triangles.len() {
        panels.entry(root(&mut parent, i)).or_default().push(i);
    }
    Some(
        panels
            .into_values()
            .map(|faces| (charts[faces[0]], faces))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::hlod::{interfaces::key, store::SourcePrimitive};

    fn coincident_grid() -> (Merged, Vec<InterfaceRow>) {
        const N: u32 = 4;
        let positions: Vec<_> = (0..=N)
            .flat_map(|y| (0..=N).map(move |x| [x as f32, y as f32, 0.]))
            .collect();
        let mut triangles = Vec::new();
        for y in 0..N {
            for x in 0..N {
                let a = y * (N + 1) + x;
                for t in [[a, a + 1, a + N + 2], [a, a + N + 2, a + N + 1]] {
                    triangles.extend([t, [t[0], t[2], t[1]]]);
                }
            }
        }
        let representative = vec![
            SourcePrimitive {
                node: 0,
                mesh: 0,
                primitive: 0,
                expanded_ordinal: 0,
                material: u32::MAX,
            };
            triangles.len()
        ];
        let m = Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            positions,
            triangles,
            representative,
        };
        let mut interface = Vec::new();
        for i in 0..N {
            for (a, b) in [
                (i, i + 1),
                (N * (N + 1) + i, N * (N + 1) + i + 1),
                (i * (N + 1), (i + 1) * (N + 1)),
                (i * (N + 1) + N, (i + 1) * (N + 1) + N),
            ] {
                interface.push(InterfaceRow::new(
                    0,
                    m.keys[a as usize],
                    m.keys[b as usize],
                    0,
                    1,
                ));
            }
        }
        (m, interface)
    }

    #[test]
    fn coincident_source_sheets_fit_cap_with_distinct_local_fan_centers() {
        let (mut m, interface) = coincident_grid();
        let source = m.triangles.clone();
        let guard = super::super::super::folds::Source::new(&m.positions, &source)
            .with_boundary(&interface);
        // CDT cannot project the combined up/down component. Its final fallback
        // needs exactly the two 16-edge source boundaries, no extra border split.
        assert!(super::super::remesh(&mut m, &interface, 31).is_none());
        assert_eq!(m.triangles, source);
        let receipt = super::super::remesh(&mut m, &interface, 32).unwrap();
        assert_eq!(m.triangles.len(), 32);
        assert_eq!(receipt.constrained_remeshes, 2);
        assert_eq!(
            receipt.closed_holes + receipt.opened_holes + receipt.dropped_components,
            0
        );
        assert!(!guard
            .violations(&m.positions, &m.triangles)
            .iter()
            .any(|v| *v));
        let required = interface.iter().map(|r| (r.a, r.b)).collect();
        let points = interface.iter().flat_map(|r| [r.a, r.b]).collect();
        let lost = super::super::super::missing(&m.triangles, &m.keys, &required, &points);
        assert!(lost.0.is_empty() && lost.1.is_empty());
        let canonical = |mut t: [u32; 3]| {
            let k = (0..3).min_by_key(|&i| t[i]).unwrap();
            t.rotate_left(k);
            t
        };
        let old: HashSet<_> = source.iter().copied().map(canonical).collect();
        assert_eq!(
            receipt.reconstructed_faces as usize,
            m.triangles
                .iter()
                .filter(|&&t| !old.contains(&canonical(t)))
                .count()
        );
        let centers: BTreeSet<_> = m.triangles.iter().map(|t| t[2]).collect();
        assert_eq!(centers.len(), 2);
        assert!(centers.into_iter().all(|v| m.positions[v as usize][0] > 0.
            && m.positions[v as usize][0] < 4.
            && m.positions[v as usize][1] > 0.
            && m.positions[v as usize][1] < 4.));
    }

    #[test]
    fn mandatory_interior_items_refuse_the_fan_without_mutation() {
        for segment in [false, true] {
            let (mut m, mut interface) = coincident_grid();
            interface.push(InterfaceRow::new(
                0,
                m.keys[12],
                m.keys[if segment { 13 } else { 12 }],
                0,
                1,
            ));
            let before = m.triangles.clone();
            let representatives = m.representative.clone();
            let guard = super::super::super::folds::Source::new(&m.positions, &m.triangles)
                .with_boundary(&interface);
            // An interior edge is outside the fan domain. A shared mandatory
            // center cannot supply two distinct coincident-sheet centers either.
            assert!(remesh(&mut m, &interface, 64, &guard).is_none());
            assert_eq!(m.triangles, before);
            assert_eq!(m.representative, representatives);
        }
    }

    #[test]
    fn remesh_does_not_turn_nonacute_shared_source_wings_acute() {
        // Independent eight-vertex preaudit witness. Both panels have a valid
        // local star, but their nearest local centers create acute shared wings.
        let positions = vec![
            [0., 0., 0.],
            [6., 0., 0.],
            [0., 6., -5.25],
            [2., 2., -1.75],
            [2., 0.125, 0.015625],
            [0., 5.25, -6.],
            [2., 1.75, -2.],
            [2., -0.015625, -0.125],
        ];
        let triangles = vec![
            [0, 1, 4],
            [1, 3, 4],
            [3, 0, 4],
            [1, 2, 3],
            [2, 0, 3],
            [0, 1, 7],
            [1, 6, 7],
            [6, 0, 7],
            [1, 5, 6],
            [5, 0, 6],
        ];
        let mut m = Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            representative: vec![coincident_grid().0.representative[0]; triangles.len()],
            positions,
            triangles,
        };
        let interface = [
            InterfaceRow::new(0, m.keys[0], m.keys[0], 0, 1),
            InterfaceRow::new(0, m.keys[1], m.keys[1], 0, 1),
            InterfaceRow::new(0, m.keys[0], m.keys[1], 0, 1),
        ];
        // For this fixed edge (0,0,0)->(6,0,0), compute its wing normal directly,
        // without using the producer's fold predicate or remesh guard as oracle.
        let wing = |v: u32| {
            [
                0.,
                -6. * f64::from(m.positions[v as usize][2]),
                6. * f64::from(m.positions[v as usize][1]),
            ]
        };
        let dot = |a: [f64; 3], b: [f64; 3]| (0..3).map(|i| a[i] * b[i]).sum::<f64>();
        assert_eq!(dot(wing(4), wing(7)), -0.140625);
        assert_eq!(dot(wing(3), wing(6)), 252.);
        let before = m.triangles.clone();
        let representatives = m.representative.clone();
        let result = super::super::remesh(&mut m, &interface, 6);
        if result.is_none() {
            assert_eq!(m.triangles, before);
            assert_eq!(m.representative, representatives);
        } else {
            let wings: Vec<_> = m
                .triangles
                .iter()
                .filter_map(|t| {
                    (0..3)
                        .find_map(|k| (t[k] == 0 && t[(k + 1) % 3] == 1).then_some(t[(k + 2) % 3]))
                })
                .map(|v| {
                    [
                        0.,
                        -6. * f64::from(m.positions[v as usize][2]),
                        6. * f64::from(m.positions[v as usize][1]),
                    ]
                })
                .collect();
            assert_eq!(wings.len(), 2, "shared source incidence changed");
            assert!(
                dot(wings[0], wings[1]) <= 0.,
                "remesh introduced acute shared wings"
            );
        }
    }

    #[test]
    fn positive_fan_faces_do_not_license_a_multiply_wound_boundary() {
        let (mut m, _) = coincident_grid();
        m.positions = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [-1., 1., 0.],
            [-1., -1., 0.],
            [1., 0., 1.],
            [-1., 1., 1.],
            [-1., -1., 1.],
        ];
        assert!(star(&m, &[(1, 2), (2, 3), (3, 1)], 0, (2, true)));
        let twice = [(1, 2), (2, 3), (3, 4), (4, 5), (5, 6), (6, 1)];
        assert!(twice
            .iter()
            .all(|&(a, b)| topology::normal(&m.positions, [a, b, 0])[2] > 0.));
        assert!(!star(&m, &twice, 0, (2, true)));
    }
}
