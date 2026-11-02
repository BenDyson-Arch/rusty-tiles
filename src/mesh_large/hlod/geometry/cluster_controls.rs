//! N3 grid feasibility must include the unchanged source/fold guard.
use super::*;

#[test]
fn clustering_can_coarsen_past_a_settled_shared_edge_conflict() {
    // A tetrahedron's free apices are close. Three free flaps on their edge
    // put the coarsest representative elsewhere, without touching the seam.
    let positions = vec![
        [-1., 0., 0.],
        [1., 0., 0.],
        [0., 0.1, 1.],
        [0.1, 0., 1.],
        [10., 0., 1.],
        [11., 0., 1.],
        [12., 0., 1.],
    ];
    let triangles = vec![
        [0, 1, 2],
        [1, 0, 3],
        [0, 2, 3],
        [1, 3, 2],
        [2, 3, 4],
        [2, 3, 5],
        [2, 3, 6],
    ];
    let merged = Merged {
        keys: positions.iter().copied().map(key).collect(),
        ells: vec![0; positions.len()],
        positions,
        representative: vec![
            SourcePrimitive {
                node: 0,
                mesh: 0,
                primitive: 0,
                expanded_ordinal: 0,
                material: u32::MAX,
            };
            triangles.len()
        ],
        triangles,
    };
    let (a, b) = (merged.keys[0], merged.keys[1]);
    let interface = [
        InterfaceRow::new(0, a, a, 0, 1),
        InterfaceRow::new(0, b, b, 0, 1),
        InterfaceRow::new(0, a, b, 0, 1),
    ];
    let policy = folds::Policy {
        folds: true,
        ..Default::default()
    };
    let guard = folds::Source::new(&merged.positions, &merged.triangles)
        .with_boundary(&interface)
        .with_policy(policy.clone());
    // Sensitive control: the first grid merges 3 onto 2. Its opposite twins
    // fit, but restoring the source orientation overloads shared edge 0--1.
    let mut first = vec![[0, 1, 2], [1, 0, 2]];
    assert!(!guard
        .violations(&merged.positions, &first)
        .iter()
        .any(|v| *v));
    guard.settle(&merged.positions, &mut first, false);
    assert!(guard
        .violations(&merged.positions, &first)
        .iter()
        .any(|v| *v));

    let locked = [true, true, false, false, false, false, false];
    let (ordinary, _) = cluster_reduce(
        &merged,
        &merged.triangles,
        &interface,
        &locked,
        2,
        &policy,
        false,
    );
    assert_eq!(ordinary, first, "the ordinary search stays unchanged");
    let (output, error) = cluster_reduce(
        &merged,
        &merged.triangles,
        &interface,
        &locked,
        2,
        &policy,
        true,
    );
    assert_eq!(output.len(), 2);
    assert!(error.is_finite() && error > 0.);
    assert!(!guard
        .violations(&merged.positions, &output)
        .iter()
        .any(|v| *v));
    let (edges, points) = missing(
        &output,
        &merged.keys,
        &HashSet::from_iter([edge(a, b)]),
        &HashSet::from_iter([a, b]),
    );
    assert!(edges.is_empty() && points.is_empty());
    assert!(topology::changes(&merged, &output, &interface, f64::INFINITY).is_some());
}
