//! Complete-parent controls for the r2 conditional 1,965 B/face capacity model.
//!
//! Retain the real parent builder and both guards. Replace the old no-op mapping
//! control with a constructible shared soup plus one contracted two-face strip.
//! Only held children and the separately admitted disk source-owner graph predate
//! measurement: contract rows, merge/reference, previous rows, lookups, contraction
//! Source, topology checks, recovery and the returned output are all measured.
//! These are allocation controls, not RSS measurements or general CDT work bounds.
use super::*;
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

const FACES: usize = 5_000;
const CHILDREN: usize = 8;
const OUTSIDE_LEAF: u32 = CHILDREN as u32;
const STRIP: [[f32; 3]; 4] = [
    [0., 0., 0.],  // a
    [0.5, 0., 0.], // midpoint, contracted to a
    [1., 0., 0.],  // b
    [0., 1., 0.],  // free third vertex (contact only in the rejection control)
];

fn row(leaf: u32, a: Key, b: Key) -> InterfaceRow {
    InterfaceRow::new(leaf, a.min(b), a.max(b), leaf, OUTSIDE_LEAF)
}

/// A constructive source-domain witness, independent of the producer's guard.
/// An external owner holds the lower two faces. Thus every source edge incident
/// to the midpoint has two faces: source-open endpoint pinning cannot forbid its
/// zero-deviation, degree-two shared arc. Both arc edges have the same two owners.
/// For the rejection control an additional external triangle meets c only at a
/// point; it adds no branch to the midpoint's shared arc.
///
/// Each soup triangle can likewise have an oppositely oriented copy in that
/// external owner. It has three distinct shared points and three shared edges,
/// exactly six rows, with no source-open edges. Disconnected components and
/// existing opposite source faces are supported; no fresh face license is added.
/// The external owner has a depth-two descendant, so it is not terminal at the
/// tested depth one. Its source graph is stage-owned, not parent-job workspace.
fn assert_source_domain(third_contact: bool) {
    let positions = [
        STRIP[0],
        STRIP[1],
        STRIP[2],
        STRIP[3],
        [0., -1., 0.],
        [0., 2., 1.],
        [1., 2., 1.],
    ];
    let local = [[0usize, 1, 3], [1, 2, 3]];
    let mut external = vec![[1usize, 0, 4], [2, 1, 4]];
    if third_contact {
        external.push([3, 5, 6]);
    }
    let edges = |faces: &[[usize; 3]]| {
        let mut counts = HashMap::default();
        for t in faces {
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                *counts.entry((a.min(b), a.max(b))).or_insert(0usize) += 1;
            }
        }
        counts
    };
    let local_edges = edges(&local);
    let external_edges = edges(&external);
    for (&(a, b), &count) in &local_edges {
        if a == 1 || b == 1 {
            assert_eq!(count + external_edges.get(&(a, b)).copied().unwrap_or(0), 2);
        }
    }
    let mut shared: Vec<_> = local_edges
        .keys()
        .filter(|e| external_edges.contains_key(*e))
        .copied()
        .collect();
    shared.sort_unstable();
    assert_eq!(shared, [(0, 1), (1, 2)]);
    for (v, p) in positions[..4].iter().enumerate() {
        let contact = external
            .iter()
            .flatten()
            .any(|&other| positions[other] == *p);
        assert_eq!(contact, v != 3 || third_contact);
    }
    assert_eq!(STRIP[1][0], (STRIP[0][0] + STRIP[2][0]) / 2.);
}

fn children(n: usize, third_contact: bool) -> (TileKey, Vec<TileGeometry>) {
    assert_source_domain(third_contact);
    assert!(n >= CHILDREN + 2);
    assert!(n <= CHILDREN * super::super::TRIANGLE_CAP);
    let mut children = Vec::with_capacity(CHILDREN);
    let mut first = 0;
    let mut soup = 0;
    for slot in 0..CHILDREN {
        let soup_faces = (n - 2) / CHILDREN + usize::from(slot < (n - 2) % CHILDREN);
        let count = soup_faces + if slot == 0 { 2 } else { 0 };
        assert!(count <= super::super::TRIANGLE_CAP);
        let leaf = slot as u32;
        let mut child = TileGeometry {
            key: TileKey {
                first,
                end: first + count as u64,
                depth: 2,
                leaf_begin: leaf,
                leaf_end: leaf + 1,
            },
            ells: Vec::new(),
            positions: Vec::new(),
            triangles: Vec::new(),
            representative: Vec::new(),
            primitives: Vec::new(),
            interface: Vec::new(),
            geometric_error: 0.,
            lease: None,
        };
        if slot == 0 {
            child.positions.extend(STRIP);
            child.triangles.extend([[0, 1, 3], [1, 2, 3]]);
            let [a, mid, b, c] = STRIP.map(key);
            child.interface.extend([
                row(leaf, a, a),
                row(leaf, mid, mid),
                row(leaf, b, b),
                row(leaf, a, mid),
                row(leaf, mid, b),
            ]);
            if third_contact {
                child.interface.push(row(leaf, c, c));
            }
        }
        for _ in 0..soup_faces {
            let x = 4. + 4. * soup as f32;
            let p = [[x, 0., 0.], [x + 1., 0., 0.], [x, 1., 0.]];
            let base = child.positions.len() as u32;
            child.positions.extend(p);
            child.triangles.push([base, base + 1, base + 2]);
            for i in 0..3 {
                child.interface.push(row(leaf, key(p[i]), key(p[i])));
                child
                    .interface
                    .push(row(leaf, key(p[i]), key(p[(i + 1) % 3])));
            }
            soup += 1;
        }
        child.ells.resize(child.positions.len(), 2);
        child.representative = (0..count as u32).collect();
        child.primitives = (0..count)
            .map(|i| SourcePrimitive {
                node: 0,
                mesh: 0,
                primitive: (first + i as u64) as u32,
                expanded_ordinal: first + i as u64,
                material: u32::MAX,
            })
            .collect();
        child.interface.sort_unstable_by_key(|r| (r.a, r.b));
        child.ells.shrink_to_fit();
        child.positions.shrink_to_fit();
        child.triangles.shrink_to_fit();
        child.representative.shrink_to_fit();
        child.primitives.shrink_to_fit();
        child.interface.shrink_to_fit();
        child.check_charge().unwrap();
        first += count as u64;
        children.push(child);
    }
    assert_eq!(first, n as u64);
    assert_eq!(soup, n - 2);
    assert_eq!(
        children.iter().map(|c| c.positions.len()).sum::<usize>(),
        3 * n - 2
    );
    assert_eq!(
        children.iter().map(|c| c.interface.len()).sum::<usize>(),
        6 * (n - 2) + 5 + usize::from(third_contact)
    );
    (
        TileKey {
            first: 0,
            end: n as u64,
            depth: 1,
            leaf_begin: 0,
            leaf_end: OUTSIDE_LEAF,
        },
        children,
    )
}

fn complete_parent(n: usize, third_contact: bool) {
    crate::test_alloc::route_meshopt();
    // Held inputs are separately leased, as in parent_jobs_allocate_within_their_charge.
    // Nothing built here is moved into or freed within the counting closure.
    let (parent, children) = children(n, third_contact);
    let [a, mid, b, c] = STRIP.map(key);
    let calls = Cell::new(0usize);
    let mapped_mid = Cell::new(0usize);
    let assessed = Cell::new(0usize);
    let checks = AtomicUsize::new(0);
    let expected_rows = 6 * (n - 2) + 3 + usize::from(third_contact);
    let expected_contacts = 3 * (n - 2) + 3 + usize::from(third_contact);
    let (result, peak) = crate::test_alloc::peak(|| {
        // This allocation belongs to the parent job. In particular, do not
        // prebuild the dense parent rows alongside the held child fixtures.
        let mut interface: Vec<_> = children
            .iter()
            .flat_map(|child| child.interface.iter())
            .filter(|r| r.outer(parent.leaf_begin, parent.leaf_end))
            .filter(|r| r.a != mid && r.b != mid)
            .copied()
            .collect();
        interface.push(row(0, a, b));
        super::super::interfaces::merge_rows(&mut interface);
        assert_eq!(interface.len(), expected_rows);
        assert!(interface.len() <= 6 * super::super::TRIANGLE_CAP);
        assert_eq!(
            interface.iter().any(|r| r.a == c || r.b == c),
            third_contact
        );
        let contract = Contract {
            depth: parent.depth,
            interface,
            certificates: Vec::new(),
            terminal_vertices: Vec::new(),
            scale: 0.,
        };
        build_parent(
            parent,
            &children,
            &[super::super::TRIANGLE_CAP],
            &contract,
            &|k| {
                calls.set(calls.get() + 1);
                if k == mid {
                    mapped_mid.set(mapped_mid.get() + 1);
                    Ok(Some((a, b, 0.5, 2)))
                } else {
                    // The geometry owner must not query an unshared interior key.
                    assert!(k != c || third_contact);
                    Ok(Some((k, k, 0., 2)))
                }
            },
            &mut |_| {
                assessed.set(assessed.get() + 1);
                Ok(true)
            },
            &|| {
                checks.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
        )
    });
    let charge = n as u64 * super::super::JOB_BYTES_PER_TRIANGLE;
    eprintln!(
        "COMPLETE CONTRACTION third_contact={third_contact}: input={n}, \
        keys={}, previous_rows={}, parent_rows={expected_rows}, peak={peak} bytes, \
        {:.2} B/input-face, charge={charge} ({} B/input-face), \
        mapping_calls={}, mapped_mid={}",
        3 * n - 2,
        6 * (n - 2) + 5 + usize::from(third_contact),
        peak as f64 / n as f64,
        super::super::JOB_BYTES_PER_TRIANGLE,
        calls.get(),
        mapped_mid.get()
    );
    assert_eq!(super::super::JOB_BYTES_PER_TRIANGLE, 1700);
    assert!(
        peak as u64 <= charge,
        "{peak} > {charge}; third_contact={third_contact}"
    );
    let outcome = result.expect("complete parent must report an outcome");
    assert!(calls.get() >= expected_contacts);
    assert!(mapped_mid.get() > 0, "a no-op mapping is not this control");
    assert!(outcome.attempted_topology.contracted_vertices > 0);
    if third_contact {
        // The only possible upper strip face is a-b-c, a fresh all-contact face.
        // No recovery proposal can self-license it against the original source.
        assert!(outcome.built.is_none());
        assert!(outcome.contraction_failed);
        assert!(outcome.attempted_topology.rejected_remesh_rounds >= 2);
        assert!(
            mapped_mid.get() > 1,
            "a recovery proposal must retry contraction against the original source"
        );
        assert!(
            checks.load(Ordering::Relaxed) > 1,
            "recovery must run after initial contraction rejection"
        );
        assert_eq!(assessed.get(), 0, "an illegal proposal reached assessment");
    } else {
        assert!(!outcome.contraction_failed);
        let built = outcome
            .built
            .expect("real contraction must publish the strip");
        assert_eq!(calls.get(), expected_contacts);
        assert_eq!(mapped_mid.get(), 1);
        assert_eq!(built.topology.contracted_vertices, 1);
        assert_eq!(built.topology.reconstructed_faces, 0);
        assert_eq!(built.topology.rejected_remesh_rounds, 0);
        assert_eq!(assessed.get(), 1);
        let geometry = built.geometry;
        assert_eq!(geometry.triangles.len(), n - 1);
        assert_eq!(geometry.interface.len(), expected_rows);
        let keys: Vec<_> = geometry.positions.iter().copied().map(key).collect();
        assert!(!keys.contains(&mid));
        assert!(geometry.triangles.iter().any(|t| {
            let face = t.map(|v| keys[v as usize]);
            face == [a, b, c] || face == [b, c, a] || face == [c, a, b]
        }));
        let segments = geometry
            .interface
            .iter()
            .filter(|r| !r.is_vertex())
            .map(|r| (r.a, r.b))
            .collect();
        let points = geometry
            .interface
            .iter()
            .filter(|r| r.is_vertex())
            .map(|r| r.a)
            .collect();
        let lost = missing(&geometry.triangles, &keys, &segments, &points);
        assert!(lost.0.is_empty() && lost.1.is_empty());
    }
}

#[test]
fn complete_parent_actual_contraction_dense_rows_fit_charge() {
    complete_parent(FACES, false);
}

#[test]
fn complete_parent_rejected_contraction_and_recovery_dense_rows_fit_charge() {
    complete_parent(FACES, true);
}
