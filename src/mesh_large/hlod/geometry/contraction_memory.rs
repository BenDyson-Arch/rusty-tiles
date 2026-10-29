//! Complete-parent controls for the r2 conditional 1,965 B/face capacity model.
//!
//! Retain the real parent builder and both guards. Replace the old no-op mapping
//! control with a constructible shared soup plus a direct or folded strip.
//! Only held children and the separately admitted disk source-owner graph predate
//! measurement: contract rows, merge/reference, previous rows, lookups, contraction
//! Source, topology checks, recovery and the returned output are all measured.
//! The combined peak includes contract metadata; only retained certificate and
//! terminal-key capacities are subtracted for the 1,700 B/face geometry check.
//! Interface rows and their construction transients remain in that check.
//! These are allocation controls, not RSS measurements or general CDT work bounds.
use super::super::borders::{contract_workspace_bytes, Certificate};
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
const DIRECT_FACES: [[u32; 3]; 2] = [[0, 1, 3], [1, 2, 3]];
// The same four-face witness as the constrained strip/frozen-ring control.
const FOLDED_STRIP: [[f32; 3]; 6] = [
    [0., 0., 0.],
    [1., 0., 0.],
    [3., 0., 0.],
    [0.5, 0.1, 0.],
    [2., 1., 0.],
    [1., 2., 3.],
];
const FOLDED_FACES: [[u32; 3]; 4] = [[0, 1, 3], [1, 4, 3], [1, 2, 4], [3, 4, 5]];

#[derive(Clone, Copy)]
enum Fixture {
    Direct { third_contact: bool },
    Constrained,
}

impl Fixture {
    fn positions(self) -> &'static [[f32; 3]] {
        match self {
            Self::Direct { .. } => &STRIP,
            Self::Constrained => &FOLDED_STRIP,
        }
    }

    fn faces(self) -> &'static [[u32; 3]] {
        match self {
            Self::Direct { .. } => &DIRECT_FACES,
            Self::Constrained => &FOLDED_FACES,
        }
    }

    fn third_contact(self) -> bool {
        matches!(
            self,
            Self::Direct {
                third_contact: true
            }
        )
    }
}

fn row(leaf: u32, a: Key, b: Key) -> InterfaceRow {
    InterfaceRow::new(leaf, a.min(b), a.max(b), leaf, OUTSIDE_LEAF)
}

fn strip_row(leaf: u32, a: Key, b: Key) -> InterfaceRow {
    InterfaceRow::new(leaf, a.min(b), a.max(b), leaf, OUTSIDE_LEAF + 1)
}

/// A constructive source-domain witness, independent of the producer's guard.
/// An external owner holds the lower two faces. Thus every source edge incident
/// to the midpoint has two faces: source-open endpoint pinning cannot forbid its
/// zero-deviation, degree-two shared arc. Both arc edges have the same two owners.
/// For the rejection control an additional external triangle meets c only at a
/// point; it adds no branch to the midpoint's shared arc.
///
/// Each soup triangle has an oppositely oriented copy in a separate terminal
/// exterior leaf at depth one. It has three distinct shared points and three
/// shared edges, exactly six rows, with no source-open edges. Disconnected components and
/// existing opposite source faces are supported; no fresh face license is added.
/// The strip's exterior owner has a depth-two descendant (leaf OUTSIDE_LEAF+1),
/// so it is not terminal at the tested depth one. The soup's terminal exterior
/// leaf is OUTSIDE_LEAF. Source graphs are stage-owned; materialized contract
/// certificates and terminal contacts below belong to the measured parent job.
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

/// The lower external fan closes a-m and m-b. The other midpoint edges m-p
/// and m-q already have two local faces. The new external edge m-d also has
/// two faces, so every midpoint incidence is closed, with opposite directions.
/// The external owner shares precisely a, m, b and the two arc edges; p, q and
/// the curved frozen-ring vertex remain free. No source-open pin or shared
/// junction prevents the degree-two a-m-b arc from becoming its zero-error chord.
fn assert_folded_source_domain() {
    let mut positions = FOLDED_STRIP.to_vec();
    positions.push([1., -1., 0.]); // d, owned only by the external tile
    let external = [[1u32, 0, 6], [2, 1, 6]];
    let counts = |faces: &[[u32; 3]]| {
        let mut edges = HashMap::default();
        for t in faces {
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                let count = edges.entry((a.min(b), a.max(b))).or_insert([0usize; 2]);
                count[usize::from(a > b)] += 1;
            }
        }
        edges
    };
    let local_edges = counts(&FOLDED_FACES);
    let external_edges = counts(&external);
    let mut shared: Vec<_> = local_edges
        .keys()
        .filter(|e| external_edges.contains_key(*e))
        .copied()
        .collect();
    shared.sort_unstable();
    assert_eq!(shared, [(0, 1), (1, 2)]);
    for &(a, b) in local_edges.keys().chain(external_edges.keys()) {
        if a == 1 || b == 1 {
            let local = local_edges.get(&(a, b)).copied().unwrap_or_default();
            let outside = external_edges.get(&(a, b)).copied().unwrap_or_default();
            assert_eq!([local[0] + outside[0], local[1] + outside[1]], [1, 1]);
        }
    }
    for (v, p) in FOLDED_STRIP.iter().enumerate() {
        assert_eq!(
            external
                .iter()
                .flatten()
                .any(|&i| positions[i as usize] == *p),
            v < 3
        );
    }
    // A direct snap really inverts (m,q,p); an unchanged/direct path cannot
    // satisfy the CDT receipt assertions below. All three arc points are collinear.
    let [a, mid, b, p, q, _] = FOLDED_STRIP.map(|p| p.map(f64::from));
    let area = |origin: [f64; 3]| {
        (q[0] - origin[0]) * (p[1] - origin[1]) - (q[1] - origin[1]) * (p[0] - origin[0])
    };
    assert!(area(mid) > 0. && area(a) < 0.);
    assert_eq!(a[1], mid[1]);
    assert_eq!(mid[1], b[1]);
}

fn children(n: usize, fixture: Fixture) -> (TileKey, Vec<TileGeometry>) {
    match fixture {
        Fixture::Direct { third_contact } => assert_source_domain(third_contact),
        Fixture::Constrained => assert_folded_source_domain(),
    }
    let strip_faces = fixture.faces().len();
    let third_contact = fixture.third_contact();
    assert!(n >= CHILDREN + strip_faces);
    assert!(n <= CHILDREN * super::super::TRIANGLE_CAP);
    let mut children = Vec::with_capacity(CHILDREN);
    let mut first = 0;
    let mut soup = 0;
    for slot in 0..CHILDREN {
        let soup_faces =
            (n - strip_faces) / CHILDREN + usize::from(slot < (n - strip_faces) % CHILDREN);
        let count = soup_faces + if slot == 0 { strip_faces } else { 0 };
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
            child.positions.extend_from_slice(fixture.positions());
            child.triangles.extend_from_slice(fixture.faces());
            let [a, mid, b, c] = std::array::from_fn(|i| key(fixture.positions()[i]));
            child.interface.extend([
                strip_row(leaf, a, a),
                strip_row(leaf, mid, mid),
                strip_row(leaf, b, b),
                strip_row(leaf, a, mid),
                strip_row(leaf, mid, b),
            ]);
            if third_contact {
                child.interface.push(strip_row(leaf, c, c));
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
    assert_eq!(soup, n - strip_faces);
    assert_eq!(
        children.iter().map(|c| c.positions.len()).sum::<usize>(),
        3 * (n - strip_faces) + fixture.positions().len()
    );
    assert_eq!(
        children.iter().map(|c| c.interface.len()).sum::<usize>(),
        6 * (n - strip_faces) + 5 + usize::from(third_contact)
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

#[derive(Clone, Copy, Debug)]
struct Metadata {
    certificates: usize,
    certificate_capacity: usize,
    terminal_vertices: usize,
    terminal_capacity: usize,
}

impl Metadata {
    fn of(contract: &Contract) -> Self {
        Self {
            certificates: contract.certificates.len(),
            certificate_capacity: contract.certificates.capacity(),
            terminal_vertices: contract.terminal_vertices.len(),
            terminal_capacity: contract.terminal_vertices.capacity(),
        }
    }

    fn bytes(self) -> usize {
        self.certificate_capacity * std::mem::size_of::<Certificate>()
            + self.terminal_capacity * std::mem::size_of::<Key>()
    }
}

/// Model the exact reservations and emission of Borders::contract, including
/// duplicate endpoint rows/terminal keys before merge/dedup. Soup endpoints are
/// terminal junctions, so each literal edge is its own one-edge source chain.
/// The strip is the nonterminal ordered arc [a,mid,b], and an isolated third
/// contact contributes a linked point item but no certificate or terminal key.
fn measured_contract(parent: TileKey, children: &[TileGeometry], fixture: Fixture) -> Contract {
    let soup_faces: usize =
        children.iter().map(|c| c.triangles.len()).sum::<usize>() - fixture.faces().len();
    let expected_certificates = 3 * soup_faces + 1;
    let count = expected_certificates + usize::from(fixture.third_contact());
    let limit = 6 * super::super::TRIANGLE_CAP;
    assert!(count <= limit);
    // Allocate all retained metadata before rows, and never resize it. Thus its
    // full capacity is live throughout row construction and the geometry job.
    let mut certificates = Vec::with_capacity(count);
    let mut terminal_vertices = Vec::with_capacity(2 * count);
    let mut interface = Vec::with_capacity(3 * count);
    for (slot, child) in children.iter().enumerate() {
        let skip = if slot == 0 { fixture.faces().len() } else { 0 };
        for t in &child.triangles[skip..] {
            for i in 0..3 {
                let (a, b) = edge(
                    key(child.positions[t[i] as usize]),
                    key(child.positions[t[(i + 1) % 3] as usize]),
                );
                let certificate = Certificate {
                    endpoints: [a, b],
                    chain: [a, b],
                    origin: a,
                    first: 0,
                    end: 1,
                    epsilon: 0.,
                    terminal_lock: true,
                };
                assert_eq!(certificate.endpoints, certificate.chain);
                assert_eq!(certificate.origin, certificate.chain[0]);
                assert_eq!(certificate.end - certificate.first, 1);
                assert_eq!(certificate.epsilon.to_bits(), 0f64.to_bits());
                certificates.push(certificate);
                terminal_vertices.extend([a, b]);
                for point in [a, b] {
                    interface.push(row(child.key.leaf_begin, point, point));
                }
                interface.push(row(child.key.leaf_begin, a, b));
            }
        }
    }
    let positions = fixture.positions();
    let (a, mid, b) = (key(positions[0]), key(positions[1]), key(positions[2]));
    assert!(a < mid && mid < b);
    // Source ordinals are 0,1,2, so the chord's t=1/2 need not be a spatial
    // midpoint. Match the outward certificate arithmetic of curve::shortcuts:
    // literal edges are exactly zero; a genuine shortcut includes rounding.
    let distance = (0..3)
        .map(|axis| {
            let interpolated =
                f64::from(positions[0][axis]) * 0.5 + f64::from(positions[2][axis]) * 0.5;
            (interpolated - f64::from(positions[1][axis])).powi(2)
        })
        .sum::<f64>()
        .sqrt();
    let rounding = positions[..3]
        .iter()
        .flatten()
        .map(|&x| f64::from(x).abs())
        .fold(1., f64::max)
        * (128. * f64::EPSILON);
    let epsilon = (distance + rounding).next_up();
    assert!(epsilon > distance && epsilon.is_finite());
    match fixture {
        Fixture::Direct { .. } => assert_eq!(distance, 0.),
        Fixture::Constrained => assert_eq!(distance, 0.5),
    }
    // These are existing child arc edges, while a-b is genuinely new.
    assert!(!fixture
        .faces()
        .iter()
        .any(|t| t.contains(&0) && t.contains(&2)));
    for arc in [(a, mid), (mid, b)] {
        assert!(children[0].interface.iter().any(|r| r.item() == arc));
    }
    certificates.push(Certificate {
        endpoints: [a, b],
        chain: [a, mid],
        origin: a,
        first: 0,
        end: 2,
        epsilon,
        terminal_lock: false,
    });
    for point in [a, b] {
        interface.push(strip_row(parent.leaf_begin, point, point));
    }
    interface.push(strip_row(parent.leaf_begin, a, b));
    if fixture.third_contact() {
        let c = key(positions[3]);
        // An isolated point segment emits its endpoint row twice in production.
        interface.extend([strip_row(parent.leaf_begin, c, c); 2]);
    }
    assert_eq!(certificates.len(), expected_certificates);
    assert_eq!(terminal_vertices.len(), 6 * soup_faces);
    assert_eq!(
        interface.len(),
        3 * expected_certificates + 2 * usize::from(fixture.third_contact())
    );
    assert_eq!(interface.capacity(), 3 * count);
    super::super::interfaces::merge_rows(&mut interface);
    certificates.sort_by_key(|c| (c.endpoints, c.chain, c.first));
    terminal_vertices.sort_unstable();
    terminal_vertices.dedup();
    let contract = Contract {
        depth: parent.depth,
        interface,
        certificates,
        terminal_vertices,
        // A depth-one ladder for an extent containing the generated source.
        scale: (4. + 4. * soup_faces as f64) / 2048.,
    };
    let metadata = Metadata::of(&contract);
    assert_eq!(metadata.certificates, expected_certificates);
    assert_eq!(metadata.certificate_capacity, count);
    assert_eq!(metadata.terminal_vertices, 3 * soup_faces);
    assert_eq!(metadata.terminal_capacity, 2 * count);
    assert!(metadata.certificate_capacity <= limit);
    assert!(metadata.terminal_capacity <= 2 * limit);
    assert_eq!(
        contract.interface.len(),
        6 * soup_faces + 3 + usize::from(fixture.third_contact())
    );
    assert_eq!(contract.interface.capacity(), contract.interface.len());
    assert_eq!(
        contract.interface.iter().filter(|r| !r.is_vertex()).count(),
        expected_certificates
    );
    assert_eq!(
        contract
            .certificates
            .iter()
            .filter(|c| c.end - c.first == 2)
            .count(),
        1
    );
    assert_eq!(
        contract
            .certificates
            .iter()
            .filter(|c| c.terminal_lock)
            .count(),
        3 * soup_faces
    );
    assert!(contract
        .certificates
        .iter()
        .all(|c| c.epsilon <= contract.scale / 16.));
    assert!(contract
        .terminal_vertices
        .iter()
        .all(|k| positions.iter().all(|p| key(*p) != *k)));
    assert!(metadata.bytes() as u64 <= contract_workspace_bytes(limit as u64));
    contract
}

fn assert_peaks(n: usize, combined_peak: usize, metadata: Metadata) {
    let metadata_bytes = metadata.bytes();
    let geometry_peak = combined_peak
        .checked_sub(metadata_bytes)
        .expect("retained metadata must be present in the combined peak");
    let geometry_charge = n as u64 * super::super::JOB_BYTES_PER_TRIANGLE;
    let contract_charge = contract_workspace_bytes((6 * super::super::TRIANGLE_CAP) as u64);
    eprintln!(
        "CONTRACT WORKSPACE: combined_peak={combined_peak} bytes, \
        certificates={} capacity={}, terminal_vertices={} capacity={}, \
        retained_metadata={metadata_bytes} bytes, geometry_peak={geometry_peak} bytes \
        ({:.2} B/input-face), geometry_charge={geometry_charge}, \
        contract_charge={contract_charge}, combined_charge={}",
        metadata.certificates,
        metadata.certificate_capacity,
        metadata.terminal_vertices,
        metadata.terminal_capacity,
        geometry_peak as f64 / n as f64,
        geometry_charge + contract_charge
    );
    assert_eq!(super::super::JOB_BYTES_PER_TRIANGLE, 1700);
    // Subtract only certificate/terminal capacities; interface rows and every
    // construction transient remain in this conservative geometry residual.
    assert!(
        geometry_peak as u64 <= geometry_charge,
        "geometry {geometry_peak} > {geometry_charge}"
    );
    assert!(
        combined_peak as u64 <= geometry_charge + contract_charge,
        "combined {combined_peak} > {}",
        geometry_charge + contract_charge
    );
}

fn complete_parent(n: usize, third_contact: bool) {
    // Held inputs are separately leased, as in parent_jobs_allocate_within_their_charge.
    // Nothing built here is moved into or freed within the counting closure.
    let (parent, children) = children(n, Fixture::Direct { third_contact });
    let [a, mid, b, c] = STRIP.map(key);
    let calls = Cell::new(0usize);
    let mapped_mid = Cell::new(0usize);
    let assessed = Cell::new(0usize);
    let checks = AtomicUsize::new(0);
    let expected_rows = 6 * (n - 2) + 3 + usize::from(third_contact);
    let expected_contacts = 3 * (n - 2) + 3 + usize::from(third_contact);
    let ((result, metadata), peak) = crate::test_alloc::peak(|| {
        let contract = measured_contract(parent, &children, Fixture::Direct { third_contact });
        assert_eq!(contract.interface.len(), expected_rows);
        assert_eq!(
            contract.interface.iter().any(|r| r.a == c || r.b == c),
            third_contact
        );
        let result = build_parent(
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
                    Ok(Some((k, k, 0., if [a, b, c].contains(&k) { 2 } else { 1 })))
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
            true,
        );
        // Read and retain both metadata vectors after the complete builder.
        let metadata = Metadata::of(std::hint::black_box(&contract));
        (result, metadata)
    });
    let charge = n as u64 * super::super::JOB_BYTES_PER_TRIANGLE;
    eprintln!(
        "COMPLETE CONTRACTION third_contact={third_contact}: input={n}, \
        keys={}, previous_rows={}, parent_rows={expected_rows}, combined_peak={peak} bytes, \
        {:.2} combined B/input-face, geometry_charge={charge} ({} B/input-face), \
        mapping_calls={}, mapped_mid={}",
        3 * n - 2,
        6 * (n - 2) + 5 + usize::from(third_contact),
        peak as f64 / n as f64,
        super::super::JOB_BYTES_PER_TRIANGLE,
        calls.get(),
        mapped_mid.get()
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
    assert_peaks(n, peak, metadata);
}

#[test]
fn complete_parent_actual_contraction_dense_rows_fit_charge() {
    complete_parent(FACES, false);
}

#[test]
fn complete_parent_rejected_contraction_and_recovery_dense_rows_fit_charge() {
    complete_parent(FACES, true);
}

#[test]
fn complete_parent_constrained_contraction_dense_rows_fit_charge() {
    let n = FACES;
    // Only held children and the source-owner domain witness predate counting.
    // In particular, no merged mesh, reference or fold Source is preallocated.
    let (parent, children) = children(n, Fixture::Constrained);
    let [a, mid, b, p, q, frozen] = FOLDED_STRIP.map(key);
    let calls = Cell::new(0usize);
    let mapped_mid = Cell::new(0usize);
    let assessed = Cell::new(0usize);
    let soup_faces = n - FOLDED_FACES.len();
    let expected_rows = 6 * soup_faces + 3;
    let expected_contacts = 3 * soup_faces + 3;
    let ((result, metadata), peak) = crate::test_alloc::peak(|| {
        let contract = measured_contract(parent, &children, Fixture::Constrained);
        assert_eq!(contract.interface.len(), expected_rows);
        assert!(contract
            .interface
            .iter()
            .all(|r| ![mid, p, q, frozen].contains(&r.a) && ![mid, p, q, frozen].contains(&r.b)));
        // This calls the complete producer, including its original-source
        // contraction guard and the successful constrained triangulator. There
        // is no helper-only measurement or substitute prelicensed output.
        let result = build_parent(
            parent,
            &children,
            &[super::super::TRIANGLE_CAP],
            &contract,
            &|k| {
                calls.set(calls.get() + 1);
                assert!(![p, q, frozen].contains(&k), "interior source lookup");
                if k == mid {
                    mapped_mid.set(mapped_mid.get() + 1);
                    Ok(Some((a, b, 0.5, 2)))
                } else {
                    Ok(Some((k, k, 0., if [a, b].contains(&k) { 2 } else { 1 })))
                }
            },
            &mut |_| {
                assessed.set(assessed.get() + 1);
                Ok(true)
            },
            &|| Ok(()),
            true,
        );
        let metadata = Metadata::of(std::hint::black_box(&contract));
        (result, metadata)
    });
    let charge = n as u64 * super::super::JOB_BYTES_PER_TRIANGLE;
    eprintln!(
        "COMPLETE CONTRACTION constrained: input={n}, keys={}, \
        previous_rows={}, parent_rows={expected_rows}, combined_peak={peak} bytes, \
        {:.2} combined B/input-face, geometry_charge={charge} ({} B/input-face), \
        mapping_calls={}, mapped_mid={}",
        3 * soup_faces + FOLDED_STRIP.len(),
        6 * soup_faces + 5,
        peak as f64 / n as f64,
        super::super::JOB_BYTES_PER_TRIANGLE,
        calls.get(),
        mapped_mid.get()
    );
    assert_eq!(super::super::JOB_BYTES_PER_TRIANGLE, 1700);
    let outcome = result.expect("complete constrained parent must report an outcome");
    assert!(!outcome.contraction_failed);
    assert!(outcome.attempted_topology.contracted_vertices > 0);
    let built = outcome.built.expect("constrained strip must publish");
    assert_eq!(calls.get(), expected_contacts);
    assert_eq!(
        mapped_mid.get(),
        1,
        "the initial CDT contraction must succeed"
    );
    assert_eq!(assessed.get(), 1);
    assert_eq!(built.topology.contracted_vertices, 1);
    assert!(
        built.topology.constrained_remeshes > 0,
        "a direct pass is not this control"
    );
    assert_eq!(built.topology.reconstructed_faces, 2);
    assert_eq!(built.topology.rejected_remesh_rounds, 0);
    let geometry = built.geometry;
    assert_eq!(geometry.triangles.len(), n - 1);
    assert_eq!(geometry.interface.len(), expected_rows);
    let keys: Vec<_> = geometry.positions.iter().copied().map(key).collect();
    assert!(!keys.contains(&mid));
    let strip: Vec<_> = geometry
        .triangles
        .iter()
        .copied()
        .filter(|t| t.iter().all(|&v| geometry.positions[v as usize][0] < 4.))
        .collect();
    assert_eq!(strip.len(), 3);
    assert!(
        strip.iter().any(|t| {
            let face = t.map(|v| keys[v as usize]);
            face == [p, q, frozen] || face == [q, frozen, p] || face == [frozen, p, q]
        }),
        "the oriented frozen [3,4,5] face must survive exactly"
    );
    let rebuilt: Vec<_> = strip
        .iter()
        .copied()
        .filter(|t| !t.iter().any(|&v| keys[v as usize] == frozen))
        .collect();
    assert_eq!(rebuilt.len(), 2);
    assert!(rebuilt
        .iter()
        .all(|&t| topology::normal(&geometry.positions, t)[2] > 0.));
    assert!(rebuilt.iter().any(|t| {
        let face = t.map(|v| keys[v as usize]);
        face.contains(&a) && face.contains(&b)
    }));
    let before: f64 = FOLDED_FACES[..3]
        .iter()
        .map(|&t| topology::normal(&FOLDED_STRIP, t)[2])
        .sum();
    let after: f64 = rebuilt
        .iter()
        .map(|&t| topology::normal(&geometry.positions, t)[2])
        .sum();
    assert!(
        (before - after).abs() < 1e-12,
        "strip area changed: {before} -> {after}"
    );
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
    // Verify the actual successful CDT/frozen-ring result before the resource
    // assertion, so an overcharge cannot hide an unrelated branch failure.
    assert_peaks(n, peak, metadata);
}
