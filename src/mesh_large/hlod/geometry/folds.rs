//! Reject introduced face coincidences and overlapping edge fans before publication.
//! This is a proposal check: it never removes, reverses or moves a face.
use super::{key, Key};
use rustc_hash::FxHashMap as HashMap;
use rustc_hash::FxHashSet as HashSet;
#[path = "folds_contract.rs"]
mod contract;

type Face = [Key; 3];
type Edge = (Key, Key);
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

pub(crate) struct Source {
    faces: HashMap<Face, [u32; 2]>,
    edges: HashMap<Edge, [u32; 2]>,
    boundary: HashSet<Key>,
}

impl Source {
    pub(crate) fn new(positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Self {
        let _cpu = super::super::cpu_profile::start(
            super::super::cpu_profile::Stage::Source,
            triangles.len(),
        );
        let mut result = Self {
            // Reserve the face bound and the common surface-edge count. This is
            // only a capacity hint: disconnected/nonmanifold inputs still grow.
            faces: HashMap::with_capacity_and_hasher(triangles.len(), Default::default()),
            edges: HashMap::with_capacity_and_hasher(triangles.len() * 3 / 2, Default::default()),
            boundary: HashSet::default(),
        };
        for t in triangles {
            let keys = t.map(|v| key(positions[v as usize]));
            let (f, d) = face(keys);
            result.faces.entry(f).or_default()[d] += 1;
            for j in 0..3 {
                let (a, b) = (keys[j], keys[(j + 1) % 3]);
                result.edges.entry((a.min(b), a.max(b))).or_default()[usize::from(a > b)] += 1;
            }
        }
        result
    }

    pub(crate) fn with_boundary(
        mut self,
        rows: &[crate::mesh_large::hlod::interfaces::InterfaceRow],
    ) -> Self {
        let _cpu = super::super::cpu_profile::start(
            super::super::cpu_profile::Stage::Boundary,
            rows.len(),
        );
        // Grow with distinct keys rather than the row iterator's (often duplicate)
        // size hint. A parent source's checked boundary has at most 3m keys.
        for r in rows {
            self.boundary.insert(r.a);
            self.boundary.insert(r.b);
        }
        self
    }

    fn bad_faces(&self, positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<bool> {
        let _cpu = super::super::cpu_profile::start(
            super::super::cpu_profile::Stage::Faces,
            triangles.len(),
        );
        let mut invalid = vec![false; triangles.len()];
        let mut faces = Vec::with_capacity(triangles.len());
        for (i, t) in triangles.iter().enumerate() {
            let (f, d) = face(t.map(|v| key(positions[v as usize])));
            let source = self.faces.get(&f).copied().unwrap_or_default();
            // Reject the old orient() repair rather than flipping or removing a face.
            invalid[i] = (source[d] == 0 && source[1 - d] != 0)
                // Two disjoint children can independently close the same three
                // shared points into opposite faces. Their union would license
                // a new twin as "input". Retain the local source strip instead.
                || (source == [0, 0] && f.iter().all(|p| self.boundary.contains(p)))
                || (source == [0, 0]
                    && super::topology::normal(positions, *t)
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
                let source = self.faces.get(&faces[at].0).copied().unwrap_or_default();
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

    /// Corners of rejected faces, in vertex order. One flat sorted incidence array
    /// avoids an allocation for every edge. More than four wings cannot be pairwise
    /// nonacute in their plane, so the fallback never enumerates high-valence pairs.
    pub(crate) fn violations(&self, positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<bool> {
        self.check(positions, triangles, false)
    }

    /// A whole-panel fan has no certified contraction license for moved wings.
    /// Retain the boundary count ceiling and also check changed boundary fans.
    pub(super) fn remesh_violations(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
    ) -> Vec<bool> {
        self.check(positions, triangles, true)
    }

    fn check(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        changed_boundary: bool,
    ) -> Vec<bool> {
        let mut invalid = self.bad_faces(positions, triangles);
        let incidence_cpu = super::super::cpu_profile::start(
            super::super::cpu_profile::Stage::Incidence,
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
        let _cpu =
            super::super::cpu_profile::start(super::super::cpu_profile::Stage::Fans, edges.len());
        let mut at = 0;
        while at < edges.len() {
            let edge = edges[at].0;
            let end = at + edges[at..].partition_point(|r| r.0 == edge);
            let mut counts = [0; 2];
            for &(_, (_, d, _)) in &edges[at..end] {
                counts[d] += 1;
            }
            let source = self.edges.get(&edge).copied().unwrap_or_default();
            let boundary = self.boundary.contains(&edge.0) && self.boundary.contains(&edge.1);
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
                                self.faces.get(&f).is_some_and(|n| n[direction] != 0)
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

/// Retain a local carrier before reduction can join two shared contacts. Pinning
/// the free common neighbour is smaller than later freezing both contact stars.
/// The proposal guard still checks multi-step collapses and every changed fan.
pub(super) fn contact_pins(
    keys: &[Key],
    triangles: &[[u32; 3]],
    interface: &[crate::mesh_large::hlod::interfaces::InterfaceRow],
) -> Vec<bool> {
    let _cpu = super::super::cpu_profile::start(
        super::super::cpu_profile::Stage::Contacts,
        triangles.len(),
    );
    let boundary = super::interface_points(interface);
    let mut first = vec![u32::MAX; keys.len()];
    let mut pins = vec![false; keys.len()];
    for t in triangles {
        for i in 0..3 {
            let v = t[i] as usize;
            if boundary.contains(&keys[v]) {
                continue;
            }
            for j in [1, 2] {
                let other = t[(i + j) % 3];
                if boundary.contains(&keys[other as usize]) {
                    if first[v] == u32::MAX {
                        first[v] = other;
                    } else if first[v] != other {
                        pins[v] = true;
                    }
                }
            }
        }
    }
    pins
}

fn distinct_sides(edge: Edge, thirds: impl Iterator<Item = Key>) -> bool {
    let _cpu = super::super::cpu_profile::start(super::super::cpu_profile::Stage::ExactFans, 1);
    let mut normals = Vec::<[Vec<f64>; 3]>::with_capacity(4);
    for c in thirds {
        if normals.len() == 4 {
            return false;
        }
        let n = exact_normal(edge, c);
        if n.iter().all(Vec::is_empty) || normals.iter().any(|u| acute(u, &n)) {
            return false;
        }
        normals.push(n);
    }
    true
}

/// Grow an exact nonoverlapping expansion in place (TwoSum, zero elimination).
/// Degree-four terms over finite f32 coordinates are multiples of 2^-596 with
/// magnitudes below 2^522, safely inside f64's normal range, including residuals.
fn add(expansion: &mut Vec<f64>, mut q: f64) {
    let mut written = 0;
    for i in 0..expansion.len() {
        let e = expansion[i];
        let sum = q + e;
        let b = sum - q;
        let residual = (q - (sum - b)) + (e - b);
        if residual != 0. {
            expansion[written] = residual;
            written += 1;
        }
        q = sum;
    }
    expansion.truncate(written);
    if q != 0. {
        expansion.push(q);
    }
}

/// (b-a) cross (c-a), with differences expanded before multiplication. A pair
/// of f32 coordinates multiplies exactly in f64, even across its entire exponent
/// range. Keeping the cancellation terms avoids a translation-dependent epsilon.
fn exact_normal(edge: Edge, c: Key) -> [Vec<f64>; 3] {
    let p = |k: Key| k.map(|x| f64::from(f32::from_bits(x)));
    let (a, b, c) = (p(edge.0), p(edge.1), p(c));
    std::array::from_fn(|i| {
        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
        let mut terms = Vec::with_capacity(8);
        for (j, k, sign) in [(j, k, 1.), (k, j, -1.)] {
            for x in [b[j], -a[j]] {
                for y in [c[k], -a[k]] {
                    add(&mut terms, sign * x * y);
                }
            }
        }
        terms
    })
}

/// Cross-product dot sign equals the perpendicular wings' dot sign. Split every
/// product with its exact fused remainder; no angular tolerance or normalization.
fn acute(a: &[Vec<f64>; 3], b: &[Vec<f64>; 3]) -> bool {
    let mut sum = Vec::new();
    for (a, b) in a.iter().zip(b) {
        for &a in a {
            for &b in b {
                let p = a * b;
                add(&mut sum, a.mul_add(b, -p));
                add(&mut sum, p);
            }
        }
    }
    sum.last().is_some_and(|&x| x > 0.)
}

/// Encoding keeps face correspondence, so test every normal as well as multiplicity.
/// A reversed ordinary 1/1 edge is otherwise invisible to incidence counts.
pub(crate) fn decode_valid(
    before: &[[f32; 3]],
    after: &[[f32; 3]],
    triangles: &[[u32; 3]],
    boundary: &[crate::mesh_large::hlod::interfaces::InterfaceRow],
) -> bool {
    let _cpu =
        super::super::cpu_profile::start(super::super::cpu_profile::Stage::Decode, triangles.len());
    // An identity decode has exactly the source face/edge multiplicities and
    // oriented-face licenses. Finite f32 normals have positive finite f64 self-dot
    // whenever nonzero. Compare bits because signed zero is part of a canonical key.
    if before.len() == after.len()
        && before.iter().zip(after).all(|(a, b)| {
            a.iter()
                .zip(b)
                .all(|(a, b)| a.is_finite() && a.to_bits() == b.to_bits())
        })
    {
        return true;
    }
    decode_changed(before, after, triangles, boundary)
}

fn decode_changed(
    before: &[[f32; 3]],
    after: &[[f32; 3]],
    triangles: &[[u32; 3]],
    boundary: &[crate::mesh_large::hlod::interfaces::InterfaceRow],
) -> bool {
    // New decoded keys can acquire an owner in another tile, outside the source
    // incidence graph. Retain canonical source keys; merging onto an existing key
    // is still subject to the face, boundary-edge and orientation checks below.
    let keys: HashSet<_> = before.iter().copied().map(key).collect();
    if after.iter().any(|&p| !keys.contains(&key(p))) {
        return false;
    }
    for &t in triangles {
        let a = super::topology::normal(before, t);
        let b = super::topology::normal(after, t);
        let old_area = a.iter().any(|&x| x != 0.);
        let new_area = b.iter().any(|&x| x != 0.);
        if old_area != new_area
            || (old_area && a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>() <= 0.)
        {
            return false;
        }
    }
    !Source::new(before, triangles)
        .with_boundary(boundary)
        .violations(after, triangles)
        .iter()
        .any(|v| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_decode_agrees_with_full_guard_across_finite_f32_domain() {
        // Exercise the unchanged full guard, including degenerate faces, twins,
        // multiplicity, acute books, signed zero and extreme/subnormal exponents.
        for exponent in [-149, -126, -80, -1, 0, 60, 100, 125] {
            let s = 2f64.powi(exponent) as f32;
            let p = [
                [0., -0., 0.],
                [s, 0., 0.],
                [0., s, 0.],
                [0., s, s],
                [-s, -s, -s],
                [f32::MAX, f32::MIN_POSITIVE, -f32::MAX],
            ];
            let faces = [
                [0, 1, 2],
                [0, 1, 3],
                [1, 0, 2],
                [1, 0, 4],
                [0, 1, 2],
                [0, 0, 0],
                [2, 5, 4],
            ];
            for n in 0..=faces.len() {
                for boundary in [Vec::new(), boundary(&p)] {
                    assert!(decode_changed(&p, &p, &faces[..n], &boundary));
                    assert!(decode_valid(&p, &p, &faces[..n], &boundary));
                }
            }
            for v in 0..p.len() {
                let mut changed = p;
                changed[v] = p[(v + 1) % p.len()];
                assert_eq!(
                    decode_valid(&p, &changed, &faces, &boundary(&p)),
                    decode_changed(&p, &changed, &faces, &boundary(&p)),
                );
            }
        }
    }

    fn boundary(points: &[[f32; 3]]) -> Vec<crate::mesh_large::hlod::interfaces::InterfaceRow> {
        points
            .iter()
            .map(|&p| {
                crate::mesh_large::hlod::interfaces::InterfaceRow::new(0, key(p), key(p), 0, 1)
            })
            .collect()
    }

    #[test]
    fn neighbouring_children_cannot_create_a_common_new_edge_or_face() {
        let p = [
            [0., 0., 0.],
            [0., 1., 0.],
            [2., 0., -1.],
            [3., 0., -1.],
            [0., 0.5, -0.125],
            [2., 0., 1.],
            [3., 0., 1.],
            [0., 0.5, 0.125],
        ];
        let contacts = boundary(&p[..2]);
        for (c, d, m) in [(2, 3, 4), (5, 6, 7)] {
            let input = [
                [0, m, c],
                [m, 1, c],
                [1, m, d],
                [m, 0, d],
                [0, c, d],
                [1, d, c],
            ];
            let output = [[0, 1, c], [1, 0, d], [0, c, d], [1, d, c]];
            // Each child alone looks like an ordinary 1/1 tetrahedral sheet.
            assert!(!Source::new(&p, &input)
                .violations(&p, &output)
                .iter()
                .any(|v| *v));
            let source = Source::new(&p, &input).with_boundary(&contacts);
            assert!(source.violations(&p, &output).iter().any(|v| *v));
            assert!(!source.violations(&p, &input).iter().any(|v| *v));
        }
        let p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0.25, 0.25, 0.001],
        ];
        let contacts = boundary(&p[..3]);
        let fan = [[0, 1, 3], [1, 2, 3], [2, 0, 3]];
        // All three old boundary edges exist, so the face guard is needed too.
        assert!(Source::new(&p, &fan)
            .with_boundary(&contacts)
            .violations(&p, &[[0, 1, 2]])
            .iter()
            .any(|v| *v));
        assert!(!Source::new(&p, &[[0, 1, 2]])
            .with_boundary(&contacts)
            .violations(&p, &[[0, 1, 2]])
            .iter()
            .any(|v| *v));
    }

    #[test]
    fn contact_carriers_pin_the_free_neighbour_not_whole_shared_stars() {
        let p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0.25, 0.25, 0.01],
            [2., 0., 0.],
            [2., 1., 0.],
        ];
        let keys: Vec<_> = p.into_iter().map(key).collect();
        // Vertex 3 joins contacts 0 and 1. Vertex 4 sees contact 0 repeatedly,
        // but has only that one contact and must remain free to simplify.
        let triangles = [[0, 3, 2], [1, 2, 3], [0, 4, 5], [0, 5, 4]];
        assert_eq!(
            contact_pins(&keys, &triangles, &boundary(&p[..2])),
            [false, false, true, true, false, false]
        );
    }

    #[test]
    fn decoding_cannot_invent_owner_keys_or_close_a_shared_face() {
        let before = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0.001, 0.001, 0.]];
        let triangles = [[3, 1, 2]];
        let mut after = before;
        after[3] = [0.002, 0.002, 0.];
        assert!(!decode_valid(&before, &after, &triangles, &[]));
        after[3] = before[0];
        assert!(decode_valid(&before, &after, &triangles, &[]));
        assert!(!decode_valid(
            &before,
            &after,
            &triangles,
            &boundary(&before[..3])
        ));
    }

    #[test]
    fn balanced_fans_check_both_sides_and_cannot_inherit_a_new_acute_wing() {
        let mut p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
            [0., -1., 0.],
            [0., 0., -1.],
        ];
        let source = Source::new(&p, &[[0, 1, 2], [1, 0, 4]]);
        let fan = [[0, 1, 2], [0, 1, 3], [1, 0, 4], [1, 0, 5]];
        assert!(!source.violations(&p, &fan).iter().any(|v| *v));
        let child = Source::new(&p, &fan);
        for (corner, side) in [(3, 1.), (5, -1.)] {
            let before = p[corner];
            // Positive but far below the old 1e-6 angular allowance.
            p[corner][1] = side * f32::from_bits(1);
            assert!(source.violations(&p, &fan).iter().any(|v| *v));
            assert!(child.violations(&p, &fan).iter().any(|v| *v));
            p[corner] = before;
        }
        // Original acute nonmanifold sheets remain legal when kept exactly.
        p[3][1] = 0.5;
        assert!(!Source::new(&p, &fan)
            .violations(&p, &fan)
            .iter()
            .any(|v| *v));
    }

    #[test]
    fn wing_sign_handles_full_f32_range_and_exact_right_angles() {
        for (large, small) in [(f32::MAX, f32::from_bits(1)), (1., 1.), (1e-30, 1e30)] {
            let edge = (key([large, small, 0.]), key([-large, small, 0.]));
            let a = exact_normal(edge, key([0., small, small]));
            for (z, expected) in [(-small, false), (0., false), (small, true)] {
                let b = exact_normal(edge, key([0., 2. * small, z]));
                assert_eq!(acute(&a, &b), expected);
                assert!(a.iter().any(|v| !v.is_empty()));
                assert!(b.iter().any(|v| !v.is_empty()));
            }
        }
    }

    #[test]
    fn expansion_wing_sign_matches_integer_reference() {
        let mut seed = 117u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((seed >> 32) as i32 % (1 << 20)) as i128
        };
        for _ in 0..128 {
            let p: [[i128; 3]; 4] = std::array::from_fn(|_| std::array::from_fn(|_| next()));
            let normal = |c: usize| {
                let d: [i128; 3] = std::array::from_fn(|k| p[1][k] - p[0][k]);
                let u: [i128; 3] = std::array::from_fn(|k| p[c][k] - p[0][k]);
                std::array::from_fn::<_, 3, _>(|i| {
                    let (j, k) = ((i + 1) % 3, (i + 2) % 3);
                    d[j] * u[k] - d[k] * u[j]
                })
            };
            let expected = normal(2)
                .iter()
                .zip(normal(3))
                .map(|(a, b)| a * b)
                .sum::<i128>();
            let keys = p.map(|p| key(p.map(|v| v as f32)));
            let a = exact_normal((keys[0], keys[1]), keys[2]);
            let b = exact_normal((keys[0], keys[1]), keys[3]);
            assert_eq!(acute(&a, &b), expected > 0);
        }
    }

    #[test]
    fn new_twins_and_acute_flaps_are_rejected_but_source_sheets_survive() {
        let p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., 1., 0.015],
            [0., -1., 0.],
            [0., 0., 1.],
        ];
        let source = Source::new(&p, &[[0, 1, 2]]);
        for output in [
            vec![[0, 1, 2], [0, 2, 1]],
            vec![[0, 1, 2], [0, 1, 2]],
            vec![[0, 1, 2], [0, 1, 3], [1, 0, 4]],
        ] {
            assert!(source.violations(&p, &output).iter().any(|v| *v));
        }
        let book = vec![[0, 1, 2], [0, 1, 5], [1, 0, 4]];
        assert!(!source.violations(&p, &book).iter().any(|v| *v));
        let twins = vec![[0, 1, 2], [0, 2, 1]];
        assert!(!Source::new(&p, &twins)
            .violations(&p, &twins)
            .iter()
            .any(|v| *v));
    }

    #[test]
    fn final_decode_coincidence_is_detected_even_with_distinct_vertex_indices() {
        let p = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 1., 0.001]];
        let triangles = [[0, 1, 2], [1, 0, 3]];
        let source = Source::new(&p, &triangles);
        assert!(!source.violations(&p, &triangles).iter().any(|v| *v));
        let mut decoded = p;
        decoded[3] = decoded[2];
        assert!(source.violations(&decoded, &triangles).iter().any(|v| *v));
    }
    #[test]
    fn final_decode_cannot_invert_an_ordinary_manifold_face() {
        let p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., -1., 0.],
            [0., 0.9, 0.01],
        ];
        let triangles = [[0, 1, 2], [1, 0, 3]];
        let mut folded = p;
        folded[3] = p[4];
        assert!(decode_valid(&p, &p, &triangles, &[]));
        assert!(!decode_valid(&p, &folded, &triangles, &[]));
    }
}
