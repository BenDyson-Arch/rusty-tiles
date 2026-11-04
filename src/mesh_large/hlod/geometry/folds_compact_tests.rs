//! Independent literal preoptimization guard from 429a716a folds.rs, SHA-256
//! c3f47571f5c86f379723334c966756ea0bcc5324bce30d0ae762d694fcb6f951.
//! Only module paths / Source field access change below. Exact arithmetic is
//! intentionally shared: distinct_sides and its helpers are outside this change.
use super::{distinct_sides, key, Edge, Face, Key, Source};
use crate::mesh_large::hlod::interfaces::InterfaceRow;

type Incidence = (usize, usize, Key);

fn face(mut f: Face) -> (Face, usize) {
    let k = (0..3).min_by_key(|&i| f[i]).unwrap();
    f.rotate_left(k);
    let direction = usize::from(f[1] > f[2]);
    if direction != 0 {
        f.swap(1, 2);
    }
    (f, direction)
}

struct Original<'a> {
    source: &'a Source,
}

impl Original<'_> {
    fn bad_faces(&self, positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<bool> {
        let _cpu = super::super::super::cpu_profile::start(
            super::super::super::cpu_profile::Stage::Faces,
            triangles.len(),
        );
        let mut invalid = vec![false; triangles.len()];
        let mut faces = Vec::with_capacity(triangles.len());
        for (i, t) in triangles.iter().enumerate() {
            let (f, d) = face(t.map(|v| key(positions[v as usize])));
            let source = self.source.faces.get(&f).copied().unwrap_or_default();
            // Reject the old orient() repair rather than flipping or removing a face.
            invalid[i] = (source[d] == 0 && source[1 - d] != 0)
                // Two disjoint children can independently close the same three
                // shared points into opposite faces. Their union would license
                // a new twin as "input". Retain the local source strip instead.
                || (source == [0, 0] && f.iter().all(|p| self.source.boundary.contains(p)))
                || (source == [0, 0]
                    && super::super::topology::normal(positions, *t)
                        .iter()
                        .all(|&n| n == 0.));
            faces.push((f, d, i));
        }
        faces.sort_unstable();
        let mut at = 0;
        while at < faces.len() {
            let end = at + faces[at..].partition_point(|r| r.0 == faces[at].0);
            if end - at > 1 {
                let mut counts = [0; 2];
                for &(_, d, _) in &faces[at..end] {
                    counts[d] += 1;
                }
                let source = self
                    .source
                    .faces
                    .get(&faces[at].0)
                    .copied()
                    .unwrap_or_default();
                if counts[0] > source[0] || counts[1] > source[1] {
                    for &(_, _, i) in &faces[at..end] {
                        invalid[i] = true;
                    }
                }
            }
            at = end;
        }
        invalid
    }

    fn check(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        changed_boundary: bool,
    ) -> Vec<bool> {
        let mut invalid = self.bad_faces(positions, triangles);
        let incidence_cpu = super::super::super::cpu_profile::start(
            super::super::super::cpu_profile::Stage::Incidence,
            triangles.len(),
        );
        let mut edges = Vec::<(Edge, Incidence)>::with_capacity(triangles.len() * 3);
        for (i, t) in triangles.iter().enumerate() {
            let keys = t.map(|v| key(positions[v as usize]));
            for j in 0..3 {
                let (a, b, c) = (keys[j], keys[(j + 1) % 3], keys[(j + 2) % 3]);
                edges.push(((a.min(b), a.max(b)), (i, usize::from(a > b), c)));
            }
        }
        edges.sort_unstable();
        drop(incidence_cpu);
        let _cpu = super::super::super::cpu_profile::start(
            super::super::super::cpu_profile::Stage::Fans,
            edges.len(),
        );
        let mut at = 0;
        while at < edges.len() {
            let edge = edges[at].0;
            let end = at + edges[at..].partition_point(|r| r.0 == edge);
            let mut counts = [0; 2];
            for &(_, (_, d, _)) in &edges[at..end] {
                counts[d] += 1;
            }
            let source = self.source.edges.get(&edge).copied().unwrap_or_default();
            let boundary =
                self.source.boundary.contains(&edge.0) && self.source.boundary.contains(&edge.1);
            let unsupported = if boundary && (0..2).any(|d| counts[d] > source[d]) {
                // A new edge between shared contacts could be created by either
                // neighbour independently. Only an existing source edge or a
                // chord certified over its entire previous arc supplies support.
                true
            } else if boundary && !changed_boundary {
                false
            } else {
                (0..2).any(|d| {
                    if counts[d] < 2 {
                        return false;
                    }
                    // Child incidence is not automatically source multiplicity: a
                    // child may have admitted a new, nonacute book spine. Only carry
                    // its license unchanged; moving a wing must pass the side test
                    // again, even when balanced 2/2 counts are unchanged.
                    let unchanged = counts[d] <= source[d]
                        && edges[at..end]
                            .iter()
                            .filter(|(_, (_, direction, _))| *direction == d)
                            .all(|&(_, (i, _, _))| {
                                let (f, direction) =
                                    face(triangles[i].map(|v| key(positions[v as usize])));
                                self.source.faces.get(&f).is_some_and(|n| n[direction] != 0)
                            });
                    !unchanged
                        && (counts[1 - d] == 0
                            || !distinct_sides(
                                edge,
                                edges[at..end]
                                    .iter()
                                    .filter(|(_, (_, direction, _))| *direction == d)
                                    .map(|(_, (_, _, third))| *third),
                            ))
                })
            };
            if unsupported {
                for &(_, (i, _, _)) in &edges[at..end] {
                    invalid[i] = true;
                }
            }
            at = end;
        }
        let mut vertices = vec![false; positions.len()];
        for (t, invalid) in triangles.iter().zip(invalid) {
            if invalid {
                for &v in t {
                    vertices[v as usize] = true;
                }
            }
        }
        vertices
    }
}

fn boundary(points: &[[f32; 3]]) -> Vec<InterfaceRow> {
    points
        .iter()
        .map(|&p| InterfaceRow::new(0, key(p), key(p), 0, 1))
        .collect()
}

fn compare(source: &Source, positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> [Vec<bool>; 2] {
    std::array::from_fn(|mode| {
        let changed_boundary = mode != 0;
        let expected = Original { source }.check(positions, triangles, changed_boundary);
        let actual = source.check(positions, triangles, changed_boundary);
        assert_eq!(
            actual, expected,
            "changed_boundary={changed_boundary}, positions={positions:?}, triangles={triangles:?}"
        );
        actual
    })
}

#[test]
fn duplicate_keys_keep_multiplicity_and_mark_original_vertex_indices() {
    let p = [
        [0., 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [0., 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [-0., 0., 0.],
        [0., -0., 0.],
        [99., 99., 99.],
    ];
    let source = Source::new(&p, &[[0, 1, 2]]);
    for contacts in [Vec::new(), boundary(&p[..8])] {
        let source = Source::new(&p, &[[0, 1, 2]]).with_boundary(&contacts);
        let duplicates = compare(&source, &p, &[[0, 1, 2], [3, 4, 5]]);
        for actual in duplicates {
            assert_eq!(
                actual,
                [true, true, true, true, true, true, false, false, false]
            );
        }
        for triangles in [
            vec![[0, 1, 2], [4, 3, 5]],
            vec![[0, 1, 2], [6, 1, 2]],
            vec![[0, 1, 2], [7, 1, 2]],
            vec![[0, 3, 1], [3, 0, 2], [6, 7, 5]],
        ] {
            compare(&source, &p, &triangles);
        }
    }
    // Bitwise signed-zero identity must not turn this new face into the source
    // face. All three boundary Keys trigger the existing new-face rejection.
    let signed = Source::new(&p, &[[0, 1, 2]]).with_boundary(&boundary(&p));
    assert!(compare(&signed, &p, &[[6, 1, 2]])[0].iter().any(|v| *v));
    assert!(compare(&source, &p, &[[3, 4, 5]])[0].iter().all(|v| !*v));
    let distinct_zero = [[0, 1, 2], [6, 1, 5]];
    assert!(
        compare(&Source::new(&p, &distinct_zero), &p, &distinct_zero)[0]
            .iter()
            .all(|v| !*v)
    );
}

#[test]
fn controls_cover_source_licenses_boundary_ceiling_and_changed_boundary() {
    let mut p = [
        [0., 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [0., 0., 1.],
        [0., -1., 0.],
        [0., 0., -1.],
    ];
    let input = [[0, 1, 2], [1, 0, 4]];
    let fan = [[0, 1, 2], [0, 1, 3], [1, 0, 4], [1, 0, 5]];
    let source = Source::new(&p, &input);
    assert!(compare(&source, &p, &fan)[0].iter().all(|v| !*v));
    let child = Source::new(&p, &fan);
    for (corner, side) in [(3, 1.), (5, -1.)] {
        p[corner][1] = side * f32::from_bits(1);
        assert!(compare(&source, &p, &fan)[0].iter().any(|v| *v));
        assert!(compare(&child, &p, &fan)[0].iter().any(|v| *v));
        p[corner][1] = 0.;
    }
    p[3][1] = 0.5;
    // Original acute wings carry their exact face license unchanged.
    assert!(compare(&Source::new(&p, &fan), &p, &fan)[0]
        .iter()
        .all(|v| !*v));
    let contacts = boundary(&p[..2]);
    let mut chord = Source::new(&p, &input).with_boundary(&contacts);
    assert!(compare(&chord, &p, &fan)[0].iter().any(|v| *v));
    // Model an already-certified chord count without inventing face licenses.
    // Ordinary boundary checking carries counts; remeshing rechecks moved wings.
    chord.edges.insert((key(p[0]), key(p[1])), [2, 2]);
    let checked = compare(&chord, &p, &fan);
    assert!(checked[0].iter().all(|v| !*v));
    assert!(checked[1].iter().any(|v| *v));
    for triangles in [
        vec![[0, 1, 2], [0, 2, 1]],
        vec![[0, 1, 2], [0, 1, 2]],
        vec![[0, 0, 0]],
    ] {
        assert!(compare(&source, &p, &triangles)[0].iter().any(|v| *v));
        assert!(compare(&Source::new(&p, &triangles), &p, &triangles)[0]
            .iter()
            .all(|v| !*v));
    }
    let shared =
        Source::new(&p, &[[0, 1, 3], [1, 2, 3], [2, 0, 3]]).with_boundary(&boundary(&p[..3]));
    assert!(compare(&shared, &p, &[[0, 1, 2]])[0].iter().any(|v| *v));

    // A duplicated proposal face is invalid but still has its original face
    // license. Substituting !invalid for that license would reject other wings.
    let mut p = p.to_vec();
    p.push([0., 0.5, 0.5]);
    let source = Source::new(&p, &[[0, 1, 2], [0, 1, 3], [0, 1, 6], [1, 0, 4]]);
    let output = [[0, 1, 2], [0, 1, 2], [0, 1, 3], [1, 0, 4]];
    for rejected in compare(&source, &p, &output) {
        assert_eq!(rejected, [true, true, true, false, false, false, false]);
    }
}

#[test]
fn direction_multiplicity_and_high_valence_match_literal_guard() {
    let p = [
        [1., 0., 0.], // Numeric representative order opposes Key order.
        [0., 0., 0.],
        [0., 1., 0.],
        [0., 0., 1.],
        [0., -1., 0.],
        [0., 0., -1.],
        [0., 1., 0.001],
        [0., 1., 0.], // Duplicate wing Key at another vertex index.
    ];
    for forward in 0..=6 {
        for reverse in 0..=6 {
            let triangles: Vec<_> = (0..forward)
                .map(|j| [0, 1, j + 2])
                .chain((0..reverse).map(|j| [1, 0, j + 2]))
                .collect();
            for source_faces in [&[][..], &triangles[..triangles.len() / 2], &triangles] {
                for contacts in [Vec::new(), boundary(&p[..2]), boundary(&p)] {
                    let source = Source::new(&p, source_faces).with_boundary(&contacts);
                    let expected = compare(&source, &p, &triangles);
                    let mut reordered = triangles.clone();
                    reordered.reverse();
                    assert_eq!(compare(&source, &p, &reordered), expected);
                }
            }
        }
    }
}

#[test]
fn extreme_finite_keys_and_signed_zero_match_literal_guard() {
    for s in [
        f32::from_bits(1),
        f32::MIN_POSITIVE,
        1e-30,
        1.,
        1e30,
        f32::MAX,
    ] {
        let p = [
            [s, 0., 0.],
            [-s, -0., 0.],
            [0., s, 0.],
            [-0., 0., s],
            [0., -s, 0.],
            [0., 0., -s],
            [s, 0., 0.],
            [0., -0., 0.],
        ];
        let faces = [
            [0, 1, 2],
            [0, 1, 3],
            [1, 0, 4],
            [1, 0, 5],
            [6, 1, 2],
            [1, 6, 2],
            [7, 7, 0],
        ];
        for n in 0..=faces.len() {
            for contacts in [Vec::new(), boundary(&p[..2]), boundary(&p)] {
                let source = Source::new(&p, &faces[..n]).with_boundary(&contacts);
                compare(&source, &p, &faces);
                let mut moved = p;
                moved[3][1] = f32::from_bits(1);
                compare(&source, &moved, &faces);
            }
        }
    }
}

#[test]
fn bounded_generated_guards_and_permutations_match_literal_guard() {
    let mut seed = 0x6ac4_ef29_175b_830du64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 32) as u32
    };
    for case in 0..128 {
        let mut before: Vec<[f32; 3]> = (0..12)
            .map(|_| {
                std::array::from_fn(|_| {
                    let bits = next();
                    if case % 2 == 0 {
                        ((bits % 9) as i32 - 4) as f32
                    } else {
                        // All sign/mantissa patterns, finite exponents only.
                        f32::from_bits(if bits & 0x7f80_0000 == 0x7f80_0000 {
                            bits ^ 0x0080_0000
                        } else {
                            bits
                        })
                    }
                })
            })
            .collect();
        before[8] = before[0];
        before[9] = before[1];
        before[10] = [0., 0., 0.];
        before[11] = [-0., 0., 0.];
        let mut triangles: Vec<[u32; 3]> = (0..16)
            .map(|i| {
                if i < 10 {
                    if next() & 1 == 0 {
                        [0, 1, 2 + next() % 10]
                    } else {
                        [9, 8, 2 + next() % 10]
                    }
                } else {
                    std::array::from_fn(|_| next() % 12)
                }
            })
            .collect();
        triangles[14] = triangles[0];
        triangles[15] = [triangles[0][1], triangles[0][0], triangles[0][2]];
        let source_faces: Vec<_> = triangles
            .iter()
            .copied()
            .filter(|_| next() & 1 != 0)
            .collect();
        let mut after = before.clone();
        for _ in 0..3 {
            after[next() as usize % 12] = before[next() as usize % 12];
        }
        for contacts in [Vec::new(), boundary(&before[..2]), boundary(&before)] {
            let source = Source::new(&before, &source_faces).with_boundary(&contacts);
            for positions in [&before, &after] {
                let expected = compare(&source, positions, &triangles);
                let reordered: Vec<_> = triangles.iter().rev().copied().collect();
                assert_eq!(compare(&source, positions, &reordered), expected);
                // Renumber every vertex and the corresponding source indices.
                let remap = |t: [u32; 3]| t.map(|v| 11 - v);
                let reversed_before: Vec<_> = before.iter().rev().copied().collect();
                let reversed_positions: Vec<_> = positions.iter().rev().copied().collect();
                let reversed_source: Vec<_> = source_faces.iter().copied().map(remap).collect();
                let reversed_triangles: Vec<_> = triangles.iter().copied().map(remap).collect();
                let renumbered =
                    Source::new(&reversed_before, &reversed_source).with_boundary(&contacts);
                let mut actual = compare(&renumbered, &reversed_positions, &reversed_triangles);
                actual.iter_mut().for_each(|v| v.reverse());
                assert_eq!(actual, expected, "case {case}");
            }
        }
    }
}

#[test]
fn sequential_workspace_never_exceeds_literal_guard_on_bounded_controls() {
    if usize::BITS == 64 {
        assert_eq!(std::mem::size_of::<(Edge, Incidence)>(), 56);
        assert_eq!(std::mem::size_of::<((u32, u32), super::Incidence)>(), 24);
        assert_eq!(std::mem::size_of::<(Key, u32)>(), 16);
    }
    // Include hash reservation thresholds, disconnected all-distinct corners,
    // repeated Keys, unused positions, and no triangles. Inputs and source are
    // constructed before accounting; both outputs are measured identically.
    for n in [
        0, 1, 2, 3, 4, 5, 9, 10, 18, 19, 37, 38, 74, 75, 149, 150, 299, 300, 1024,
    ] {
        for duplicate in [false, true] {
            let mut p = Vec::new();
            let mut triangles = Vec::new();
            for i in 0..n {
                let x = if duplicate { 0. } else { (i * 4) as f32 };
                let v = p.len() as u32;
                p.extend([[x, 0., 0.], [x + 1., 0., 0.], [x, 1., 0.]]);
                triangles.push([v, v + 1, v + 2]);
            }
            for unused in [0, 4096] {
                p.resize(3 * n + unused, [99., 99., 99.]);
                let source = Source::new(&p, &triangles);
                for changed_boundary in [false, true] {
                    // Warm any process-global profiling initialization first.
                    drop(Original { source: &source }.check(&p, &triangles, changed_boundary));
                    drop(source.check(&p, &triangles, changed_boundary));
                    let (expected, old_peak) = crate::test_alloc::peak(|| {
                        Original { source: &source }.check(&p, &triangles, changed_boundary)
                    });
                    let (actual, new_peak) =
                        crate::test_alloc::peak(|| source.check(&p, &triangles, changed_boundary));
                    assert_eq!(actual, expected);
                    assert!(
                        new_peak <= old_peak,
                        "n={n}, duplicate={duplicate}, unused={unused}, changed_boundary={changed_boundary}: {new_peak} > {old_peak}"
                    );
                }
            }
        }
    }
}
