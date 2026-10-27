//! Small producer control for retaining fold repairs across topology escalation.
use super::*;

#[test]
fn fold_repairs_and_component_carriers_share_one_protected_partition() {
    let mut positions = vec![
        [0., 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [0.25, 0.25, 0.01],
        [1. / 3., 1. / 3., 0.],
        // This degenerate component has diameter two, above the 0.1 G floor.
        // Ordinary meshopt filtering removes it; topology must retain its support.
        [100., 0., 0.],
        [101., 0., 0.],
        [102., 0., 0.],
    ];
    let mut triangles = vec![
        [0, 1, 4],
        [1, 2, 4],
        [2, 0, 4],
        [0, 3, 1],
        [1, 3, 2],
        [2, 3, 0],
        [5, 6, 7],
    ];
    // A gently curved open sheet supplies more interior reduction room than a
    // closed octahedron. Its curvature makes reductions more expensive than the
    // thin-tetrahedron collapse, while its consistently upward faces avoid an
    // incidental closed-shell collapse into another pair of opposite twins.
    let sheet = positions.len() as u32;
    const N: u32 = 4;
    const W: u32 = N + 1;
    for y in 0..=N {
        for x in 0..=N {
            positions.push([
                20. + 10. * x as f32,
                10. * y as f32,
                0.2 * (x * x) as f32 + 0.15 * (y * y) as f32,
            ]);
        }
    }
    for y in 0..N {
        for x in 0..N {
            let a = sheet + y * W + x;
            triangles.extend([[a, a + 1, a + W + 1], [a, a + W + 1, a + W]]);
        }
    }
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
    let cap = 24;
    // Use interior pins (as from a prior hole repair), so proactive shared-contact
    // carriers do not prevent the fold this control must exercise.
    let interface = Vec::new();
    let opening_pins = merged.keys[..3].iter().copied().collect();
    let source = folds::Source::new(&merged.positions, &merged.triangles);
    let (raw, _) = simplify(&merged, &merged.triangles, &|v| v < 3, cap);
    assert!(
        raw.iter().enumerate().any(|(i, t)| {
            t[0] != t[1]
                && t[1] != t[2]
                && t[0] != t[2]
                && raw[i + 1..]
                    .iter()
                    .any(|other| oriented(other) == oriented(&[t[0], t[2], t[1]]))
        }),
        "raw reduction must expose opposite twins: {raw:?}"
    );
    let invalid = source.violations(&merged.positions, &raw);
    assert!(invalid.iter().any(|v| *v));

    // Independently reconstruct just the first fold repair. It must solve the
    // coincidence while still failing the component policy, so an implementation
    // that never combines the two protection sets cannot pass this control.
    let protected: Vec<_> = merged
        .triangles
        .iter()
        .map(|t| t.iter().any(|&v| invalid[v as usize]))
        .collect();
    let keep: Vec<_> = merged
        .triangles
        .iter()
        .zip(&protected)
        .filter(|(_, protected)| **protected)
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(keep.len(), 6, "the thin tetrahedron is the local fold ring");
    let rest: Vec<_> = merged
        .triangles
        .iter()
        .zip(&protected)
        .filter(|(_, protected)| !**protected)
        .map(|(t, _)| *t)
        .collect();
    let kept_vertices: HashSet<_> = keep.iter().flatten().copied().collect();
    let (mut fold_only, _) = simplify(
        &merged,
        &rest,
        &|v| v < 3 || kept_vertices.contains(&v),
        cap - keep.len(),
    );
    fold_only.extend(keep);
    assert!(fold_only.len() <= cap);
    assert!(!source
        .violations(&merged.positions, &fold_only)
        .iter()
        .any(|v| *v));
    assert!(!fold_only.iter().flatten().any(|&v| (5..=7).contains(&v)));
    assert!(topology::changes(&merged, &fold_only, &interface, 0.1).is_none());

    let built = attempt(
        TileKey {
            first: 0,
            end: merged.triangles.len() as u64,
            depth: 0,
            leaf_begin: 0,
            leaf_end: 1,
        },
        &merged,
        &interface,
        cap,
        0.,
        &merged,
        &merged.triangles,
        0.1,
        false,
        &opening_pins,
    )
    .expect("combined fold and topology protection must fit the cap");
    assert!(built.retries >= 2, "both protection stages must run");
    assert!(
        built.protected_faces > 6,
        "component support must join the fold ring"
    );
    assert!(built.geometry.triangles.len() <= cap);
    assert_eq!(built.topology.dropped_components, 0);
    assert_eq!(built.reoriented, 0);

    let ids: HashMap<_, _> = merged
        .keys
        .iter()
        .enumerate()
        .map(|(i, &k)| (k, i as u32))
        .collect();
    let output: Vec<_> = built
        .geometry
        .triangles
        .iter()
        .map(|t| t.map(|v| ids[&key(built.geometry.positions[v as usize])]))
        .collect();
    for t in &merged.triangles[..7] {
        assert!(output.iter().any(|out| oriented(out) == oriented(t)));
    }
    assert!(!source
        .violations(&merged.positions, &output)
        .iter()
        .any(|v| *v));
    assert!(topology::changes(&merged, &output, &interface, 0.1).is_some());
}
