//! Contraction support comes from complete previous-child interface paths, never
//! from the contracted proposal. All storage is bounded by the immediate children.
use super::{key, Edge, HashMap, HashSet, Key, Source};
use crate::mesh_large::hlod::interfaces::InterfaceRow;

type Mapping = Option<(Key, Key, f64, u8)>;

/// A nontrivial mapping names one parent chord and its source-ordinal parameter.
/// Retained endpoints map to themselves and are treated as literal endpoints.
fn label(mapping: Mapping) -> Option<(Edge, f64)> {
    let (a, b, t, _) = mapping?;
    if a == b || !t.is_finite() || !(0. ..=1.).contains(&t) {
        return None;
    }
    Some(if a < b { ((a, b), t) } else { ((b, a), 1. - t) })
}

fn parameter(point: Key, mapping: Mapping, chord: Edge) -> Option<f64> {
    if point == chord.0 {
        Some(0.)
    } else if point == chord.1 {
        Some(1.)
    } else {
        label(mapping)
            .filter(|&(edge, _)| edge == chord)
            .map(|(_, t)| t)
    }
}

struct Segment {
    chord: Edge,
    interval: [f64; 2],
    endpoints: [Key; 2],
    counts: [u32; 2],
}

impl Source {
    /// Expected linear hash-table work plus one O(E log E) segment sort. The
    /// temporary tables and segment array hold O(V + E) entries from at most
    /// eight capped children; required new arcs are at most the parent's 3C
    /// delivered edges. No source-sized graph or owner-pair table is retained.
    pub(crate) fn for_contraction(
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        parent_interface: &[InterfaceRow],
        previous_interface: &[InterfaceRow],
        mapping_by_vertex: &[Mapping],
    ) -> Self {
        let mut source = Self::new(positions, triangles).with_boundary(parent_interface);
        if positions.len() != mapping_by_vertex.len() {
            return source;
        }
        let mut required: HashMap<_, _> = parent_interface
            .iter()
            .filter(|r| !r.is_vertex())
            .map(|r| (r.a.min(r.b), r.a.max(r.b)))
            // A literal input edge already has trusted directional counts.
            .filter(|edge| !source.edges.contains_key(edge))
            .map(|edge| (edge, true))
            .collect();
        if required.is_empty() {
            return source;
        }
        // Store only indices here: the cached mapping tuples stay in the caller's
        // vertex array instead of being copied into every hash-table bucket.
        let vertices: HashMap<_, _> = positions
            .iter()
            .enumerate()
            .map(|(v, &p)| (key(p), v))
            .collect();
        let mut seen = HashSet::default();
        let mut segments = Vec::new();
        for row in previous_interface.iter().filter(|r| !r.is_vertex()) {
            let edge = (row.a.min(row.b), row.a.max(row.b));
            let (Some(&a), Some(&b)) = (vertices.get(&edge.0), vertices.get(&edge.1)) else {
                continue;
            };
            let (a, b) = (mapping_by_vertex[a], mapping_by_vertex[b]);
            // At least one interior endpoint must name this chord. Literal
            // endpoint-to-endpoint edges were handled by the input edge above.
            let labels = [label(a), label(b)];
            let mut candidates = [None, None];
            for (i, label) in labels.into_iter().enumerate() {
                candidates[i] = label
                    .map(|(edge, _)| edge)
                    .filter(|e| required.contains_key(e));
            }
            // Unrelated old interfaces need no deduplication table. Only edges
            // with a candidate required chord can contribute path coverage.
            if candidates.iter().all(Option::is_none) || !seen.insert(edge) {
                continue;
            }
            for (i, chord) in candidates.into_iter().enumerate() {
                let Some(chord) = chord else { continue };
                if i == 1 && candidates[0] == Some(chord) {
                    continue;
                }
                let Some((ta, tb)) = parameter(edge.0, a, chord).zip(parameter(edge.1, b, chord))
                else {
                    // An old child interface may become interior after owners
                    // merge. Its edge into another parent arc/interior point
                    // is not a branch of this required parent's source path.
                    continue;
                };
                let Some(mut counts) = source.edges.get(&edge).copied() else {
                    // Both labels claim this path, but only an actual previous
                    // shared mesh edge can establish its coverage.
                    required.insert(chord, false);
                    continue;
                };
                if ta == tb {
                    required.insert(chord, false);
                    continue;
                }
                let (interval, endpoints) = if ta < tb {
                    ([ta, tb], [edge.0, edge.1])
                } else {
                    counts.swap(0, 1);
                    ([tb, ta], [edge.1, edge.0])
                };
                segments.push(Segment {
                    chord,
                    interval,
                    endpoints,
                    counts,
                });
            }
        }
        segments.sort_unstable_by(|a, b| {
            a.chord
                .cmp(&b.chord)
                .then(a.interval[0].total_cmp(&b.interval[0]))
                .then(a.interval[1].total_cmp(&b.interval[1]))
        });
        let mut at = 0;
        while at < segments.len() {
            let chord = segments[at].chord;
            let end = at + segments[at..].partition_point(|r| r.chord == chord);
            let mut parameter = 0.;
            let mut endpoint = chord.0;
            let mut minimum = [u32::MAX; 2];
            let mut complete = required[&chord];
            for segment in &segments[at..end] {
                // Equality in both parameter and vertex identity rejects gaps,
                // branches, overlapping coverage and disconnected equal-t paths.
                complete &= segment.interval[0] == parameter && segment.endpoints[0] == endpoint;
                parameter = segment.interval[1];
                endpoint = segment.endpoints[1];
                for (min, count) in minimum.iter_mut().zip(segment.counts) {
                    *min = (*min).min(count);
                }
            }
            if complete && parameter == 1. && endpoint == chord.1 {
                source.edges.insert(chord, minimum);
            }
            at = end;
        }
        source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Chain = (
        Vec<[f32; 3]>,
        Vec<[u32; 3]>,
        Vec<InterfaceRow>,
        Vec<Mapping>,
    );

    fn chain(counts: &[[u32; 2]]) -> Chain {
        // Decreasing coordinates and reversed mapping labels exercise both
        // canonical chord direction and source-edge direction normalization.
        let mut p: Vec<_> = (0..=counts.len())
            .map(|i| [(counts.len() - i) as f32, 0., 0.])
            .collect();
        let (a, b) = (key(p[0]), key(p[counts.len()]));
        let mut mapping: Vec<_> = (0..=counts.len())
            .map(|i| Some((a, b, i as f64 / counts.len() as f64, 0)))
            .collect();
        mapping[0] = Some((a, a, 0., 0));
        mapping[counts.len()] = Some((b, b, 0., 0));
        let mut triangles = Vec::new();
        let mut previous = Vec::new();
        for (i, count) in counts.iter().enumerate() {
            previous.push(InterfaceRow::new(0, key(p[i]), key(p[i + 1]), 0, 1));
            for (d, &count) in count.iter().enumerate() {
                for wing in 0..count {
                    let v = p.len() as u32;
                    // All same-direction wings are acute, with distinct faces.
                    p.push([0., 1. + wing as f32, if d == 0 { 1. } else { -1. }]);
                    mapping.push(None);
                    triangles.push(if d == 0 {
                        [i as u32, i as u32 + 1, v]
                    } else {
                        [i as u32 + 1, i as u32, v]
                    });
                }
            }
        }
        (p, triangles, previous, mapping)
    }

    #[test]
    fn contraction_chord_uses_directional_minimum_of_every_subedge() {
        let counts = [[3, 2], [2, 1], [4, 3]];
        let (p, triangles, previous, mapping) = chain(&counts);
        let chord = (key(p[0]).min(key(p[3])), key(p[0]).max(key(p[3])));
        let parent = [InterfaceRow::new(0, chord.0, chord.1, 0, 1)];
        let source = Source::for_contraction(&p, &triangles, &parent, &previous, &mapping);
        // The source path runs opposite to the canonical chord direction.
        assert_eq!(source.edges[&chord], [1, 2]);
        for subedge in 0..counts.len() {
            let mut lower = counts;
            lower[subedge] = [1, 0];
            let (p, triangles, previous, mapping) = chain(&lower);
            let source = Source::for_contraction(&p, &triangles, &parent, &previous, &mapping);
            assert_eq!(source.edges[&chord], [0, 1], "subedge {subedge}");
        }
    }

    #[test]
    fn contraction_path_rejects_gap_branch_and_duplicate_coverage() {
        let (mut p, mut triangles, previous, mut mapping) = chain(&[[2, 1]; 3]);
        let chord = (key(p[0]).min(key(p[3])), key(p[0]).max(key(p[3])));
        let parent = [InterfaceRow::new(0, chord.0, chord.1, 0, 1)];
        let licensed = |p: &[[f32; 3]],
                        triangles: &[[u32; 3]],
                        previous: &[InterfaceRow],
                        mapping: &[Mapping]| {
            Source::for_contraction(p, triangles, &parent, previous, mapping)
                .edges
                .contains_key(&chord)
        };
        assert!(licensed(&p, &triangles, &previous, &mapping));
        // Exact owner duplicates are deduplicated, rather than counted as paths.
        let mut duplicate_owner = previous.clone();
        duplicate_owner.extend(previous.iter().map(|r| InterfaceRow { leaf: 1, ..*r }));
        assert!(licensed(&p, &triangles, &duplicate_owner, &mapping));
        let mut gap = previous.clone();
        gap.remove(1);
        assert!(!licensed(&p, &triangles, &gap, &mapping));
        // Interior triangle edges cannot fill a missing shared-interface segment.
        assert!(!licensed(&p, &triangles, &[], &mapping));
        let v = p.len() as u32;
        p.push([1.5, 0., 0.]);
        mapping.push(Some((key(p[0]), key(p[3]), 0.5, 0)));
        triangles.push([1, v, 4]);
        let mut branch = previous.clone();
        branch.push(InterfaceRow::new(0, key(p[1]), key(p[v as usize]), 0, 1));
        assert!(!licensed(&p, &triangles, &branch, &mapping));
        // A second source route is duplicate coverage even when its vertex
        // identities and parameters differ from the first route.
        triangles.push([v, 3, 4]);
        branch.push(InterfaceRow::new(0, key(p[v as usize]), key(p[3]), 0, 1));
        assert!(!licensed(&p, &triangles, &branch, &mapping));
    }

    #[test]
    fn acute_shared_chord_is_licensed_only_up_to_complete_path_minimum() {
        let (mut p, triangles, previous, mut mapping) = chain(&[[2, 1]; 2]);
        let chord = (key(p[0]).min(key(p[2])), key(p[0]).max(key(p[2])));
        let parent = [InterfaceRow::new(0, chord.0, chord.1, 0, 1)];
        p.extend([[0., 1., 2.], [0., 2., 2.], [0., -1., -2.]]);
        mapping.extend([None; 3]);
        let n = p.len() as u32;
        let output = [[0, 2, n - 3], [0, 2, n - 2], [2, 0, n - 1]];
        let source = Source::for_contraction(&p, &triangles, &parent, &previous, &mapping);
        assert!(!source.violations(&p, &output).iter().any(|v| *v));
        for subedge in 0..2 {
            let mut lower = [[2, 1]; 2];
            lower[subedge] = [1, 1];
            let (mut p, triangles, previous, mut mapping) = chain(&lower);
            p.extend([[0., 1., 2.], [0., 2., 2.], [0., -1., -2.]]);
            mapping.extend([None; 3]);
            let n = p.len() as u32;
            let output = [[0, 2, n - 3], [0, 2, n - 2], [2, 0, n - 1]];
            let source = Source::for_contraction(&p, &triangles, &parent, &previous, &mapping);
            assert!(
                source.violations(&p, &output).iter().any(|v| *v),
                "subedge {subedge}"
            );
        }
    }

    #[test]
    fn previous_child_edges_that_become_parent_interior_do_not_branch_the_arc() {
        let (p, triangles, mut previous, mapping) = chain(&[[2, 1]; 2]);
        let chord = (key(p[0]).min(key(p[2])), key(p[0]).max(key(p[2])));
        let parent = [InterfaceRow::new(0, chord.0, chord.1, 0, 1)];
        // Include every child triangle edge, including old interfaces from the
        // mapped middle vertex to unmapped interior wings. Only the two path
        // subedges have both endpoints belonging to this parent chord.
        for t in &triangles {
            for i in 0..3 {
                previous.push(InterfaceRow::new(
                    0,
                    key(p[t[i] as usize]),
                    key(p[t[(i + 1) % 3] as usize]),
                    0,
                    1,
                ));
            }
        }
        let source = Source::for_contraction(&p, &triangles, &parent, &previous, &mapping);
        assert_eq!(source.edges[&chord], [1, 2]);
    }

    #[test]
    fn contraction_guard_incremental_workspace_fits_parent_charge() {
        const TRIANGLES: usize = 32_768;
        let arcs = TRIANGLES / 2;
        const { assert!(TRIANGLES <= 8 * crate::mesh_large::hlod::TRIANGLE_CAP) };
        assert!(arcs <= 3 * crate::mesh_large::hlod::TRIANGLE_CAP);
        let mut p = Vec::with_capacity(5 * arcs);
        let mut triangles = Vec::with_capacity(TRIANGLES);
        let mut previous = Vec::with_capacity(11 * arcs);
        let mut parent = Vec::with_capacity(arcs);
        let mut mapping = Vec::with_capacity(5 * arcs);
        for arc in 0..arcs {
            let v = p.len() as u32;
            let x = (4 * arc) as f32;
            p.extend([
                [x, 0., 0.],
                [x + 0.5, 0., 0.],
                [x + 1., 0., 0.],
                [x, 1., 0.],
                [x + 1., 1., 0.],
            ]);
            let (a, b) = (key(p[v as usize]), key(p[v as usize + 2]));
            mapping.extend([None, Some((a, b, 0.5, 0)), None, None, None]);
            parent.push(InterfaceRow::new(0, a, b, 0, 1));
            for point in &p[v as usize..] {
                previous.push(InterfaceRow::new(0, key(*point), key(*point), 0, 1));
            }
            for t in [[v, v + 1, v + 3], [v + 1, v + 2, v + 4]] {
                triangles.push(t);
                for i in 0..3 {
                    previous.push(InterfaceRow::new(
                        0,
                        key(p[t[i] as usize]),
                        key(p[t[(i + 1) % 3] as usize]),
                        0,
                        1,
                    ));
                }
            }
        }
        assert_eq!(p.len(), 5 * arcs); // 2.5 distinct keys per input triangle.
        assert_eq!(previous.len(), 11 * arcs); // Every old point and edge.

        // Inputs deliberately predate the measurement. This controls additional
        // helper/violations allocations, not the complete parent's live bytes.
        let input_bytes = p.capacity() * std::mem::size_of::<[f32; 3]>()
            + triangles.capacity() * std::mem::size_of::<[u32; 3]>()
            + previous.capacity() * std::mem::size_of::<InterfaceRow>()
            + parent.capacity() * std::mem::size_of::<InterfaceRow>()
            + mapping.capacity() * std::mem::size_of::<Mapping>();
        let ((), peak) = crate::test_alloc::peak(|| {
            let source = Source::for_contraction(&p, &triangles, &parent, &previous, &mapping);
            for row in &parent {
                assert_eq!(source.edges[&(row.a, row.b)], [1, 0]);
            }
            let invalid = source.violations(&p, &triangles);
            assert!(!invalid.iter().any(|v| *v));
            std::hint::black_box((&source, &invalid));
        });
        let charge = TRIANGLES as u64 * crate::mesh_large::hlod::JOB_BYTES_PER_TRIANGLE;
        eprintln!("CONTRACTION guard incremental workspace {peak} bytes / {TRIANGLES} faces, charge {charge}; prebuilt input vectors {input_bytes} bytes excluded");
        assert!(peak as u64 <= charge);
    }
}
