//! Reject introduced face coincidences and overlapping edge fans before publication.
//! This is a proposal check: it never removes, reverses or moves a face.
use super::{key, Key};
use rustc_hash::FxHashMap as HashMap;

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
}

impl Source {
    pub(crate) fn new(positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Self {
        let mut result = Self {
            faces: HashMap::default(),
            edges: HashMap::default(),
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

    /// Face-only check used before accepting a contraction. Directed edge support
    /// for a new chord belongs to the common source-subarc contract, not its literal
    /// absence from the uncontracted input. Coincident faces still need input faces.
    pub(crate) fn preserves_faces(&self, positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> bool {
        !self.bad_faces(positions, triangles).iter().any(|v| *v)
    }

    fn bad_faces(&self, positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<bool> {
        let mut invalid = vec![false; triangles.len()];
        let mut faces = Vec::with_capacity(triangles.len());
        for (i, t) in triangles.iter().enumerate() {
            let (f, d) = face(t.map(|v| key(positions[v as usize])));
            let source = self.faces.get(&f).copied().unwrap_or_default();
            // Reject the old orient() repair rather than flipping or removing a face.
            invalid[i] = (source[d] == 0 && source[1 - d] != 0)
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
        let mut invalid = self.bad_faces(positions, triangles);
        let mut edges = Vec::<(Edge, Incidence)>::with_capacity(triangles.len() * 3);
        for (i, t) in triangles.iter().enumerate() {
            let keys = t.map(|v| key(positions[v as usize]));
            for j in 0..3 {
                let (a, b, c) = (keys[j], keys[(j + 1) % 3], keys[(j + 2) % 3]);
                edges.push(((a.min(b), a.max(b)), (i, usize::from(a > b), c)));
            }
        }
        edges.sort_unstable();
        let mut at = 0;
        while at < edges.len() {
            let edge = edges[at].0;
            let end = at + edges[at..].partition_point(|r| r.0 == edge);
            let mut counts = [0; 2];
            for &(_, (_, d, _)) in &edges[at..end] {
                counts[d] += 1;
            }
            let source = self.edges.get(&edge).copied().unwrap_or_default();
            let unsupported = (0..2)
                .any(|d| counts[d] >= 2 && counts[d] > counts[1 - d] && counts[d] > source[d]);
            if unsupported
                && !(counts[0] > 0
                    && counts[1] > 0
                    && two_sided(edge, edges[at..end].iter().map(|r| r.1)))
            {
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

fn two_sided(edge: Edge, incidence: impl Iterator<Item = Incidence> + Clone) -> bool {
    let p = |k: Key| k.map(|x| f64::from(f32::from_bits(x)));
    let (a, b) = (p(edge.0), p(edge.1));
    let d: [f64; 3] = std::array::from_fn(|k| b[k] - a[k]);
    let dd = d.iter().map(|x| x * x).sum::<f64>();
    for direction in 0..2 {
        let mut wings = Vec::<[f64; 3]>::new();
        for (_, dir, c) in incidence.clone() {
            if dir != direction {
                continue;
            }
            if wings.len() == 4 {
                return false;
            }
            let c = p(c);
            let w: [f64; 3] = std::array::from_fn(|k| c[k] - a[k]);
            let t = w.iter().zip(d).map(|(w, d)| w * d).sum::<f64>() / dd;
            let w: [f64; 3] = std::array::from_fn(|k| w[k] - t * d[k]);
            let norm = w.iter().map(|x| x * x).sum::<f64>().sqrt();
            if !norm.is_finite() || norm <= 0. {
                return false;
            }
            let w = w.map(|x| x / norm);
            if wings
                .iter()
                .any(|u| u.iter().zip(w).map(|(u, w)| u * w).sum::<f64>() > 1e-6)
            {
                return false;
            }
            wings.push(w);
        }
    }
    true
}

/// Encoding keeps face correspondence, so test every normal as well as multiplicity.
/// A reversed ordinary 1/1 edge is otherwise invisible to incidence counts.
pub(crate) fn decode_valid(
    before: &[[f32; 3]],
    after: &[[f32; 3]],
    triangles: &[[u32; 3]],
) -> bool {
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
        .violations(after, triangles)
        .iter()
        .any(|v| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let p = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., -1., 0.]];
        let triangles = [[0, 1, 2], [1, 0, 3]];
        let mut folded = p;
        folded[3] = [0., 0.9, 0.01];
        assert!(decode_valid(&p, &p, &triangles));
        assert!(!decode_valid(&p, &folded, &triangles));
    }
}
