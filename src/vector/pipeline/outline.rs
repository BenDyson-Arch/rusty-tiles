//! Outlines of partitioned polygons as contiguous ring runs.
//!
//! An oversized polygon is split into fill fragments (one `EXT_mesh_polygon`
//! polygon per triangle) plus one locked line feature holding its source
//! rings. Each ring is closed and starts at the midpoint of its longest edge,
//! so the seam of a closed strip is collinear. The line splitter cuts an
//! oversized ring into contiguous runs that are placed where they lie. In each
//! tile, the outline pieces of one source feature are chained back into
//! maximal polylines: a strip ends only where its ring leaves the tile (or
//! meets another ring at a shared vertex), and a ring wholly inside the tile
//! is one closed strip. CesiumJS draws every strip end butt-ended and joins
//! nothing across strips, so the earlier one-strip-per-edge outline rendered
//! as a notched comb at wide line widths.
//!
//! A polygon that is not partitioned gets no outline line: its loops already
//! carry its source boundary in `EXT_mesh_polygon`, and CesiumJS 1.146 draws
//! those fills without an outline. A partitioned fill's loops are its
//! triangles, so its source boundary is kept as this separate line instead.

use super::{key, Feature, Geometry, Point, PointKey};
use crate::vec3::{add, dot, mul, norm, sub};
use std::collections::{HashMap, HashSet};

/// A partitioned polygon's outline piece, as opposed to its fill fragments.
pub(super) fn is_outline(feature: &Feature) -> bool {
    feature.surface_fragment
        && matches!(
            feature.geometry,
            Geometry::LineString(_) | Geometry::MultiLineString(_)
        )
}

/// The closed source rings of a partitioned polygon, from its loops (vertex
/// indices into `positions`, `u32::MAX` between rings). Each ring starts and
/// ends at the midpoint of its longest edge: one extra vertex makes the seam
/// of a closed strip collinear, so its two butt ends meet without a notch.
pub(super) fn source_rings(positions: &[Point], loops: &[u32]) -> Vec<Vec<Point>> {
    loops
        .split(|i| *i == u32::MAX)
        .filter(|ring| ring.len() >= 2)
        .map(|ring| {
            let ring: Vec<Point> = ring.iter().map(|i| positions[*i as usize]).collect();
            let n = ring.len();
            let edge = |k: usize| norm(sub(ring[(k + 1) % n], ring[k]));
            let longest = (0..n).fold(0, |best, k| if edge(k) > edge(best) { k } else { best });
            let seam = mul(add(ring[longest], ring[(longest + 1) % n]), 0.5);
            std::iter::once(seam)
                .chain((1..=n).map(|k| ring[(longest + k) % n]))
                .chain([seam])
                .collect()
        })
        .collect()
}

/// One outline line feature per partitioned source feature, from its outline
/// pieces in one tile, in first-piece order. Pieces merge only when all
/// properties (including `_source_id`) agree, so outline metadata and picking
/// are those of the source feature.
pub(super) fn fragment_outlines<'a>(pieces: impl IntoIterator<Item = &'a Feature>) -> Vec<Feature> {
    let mut groups: Vec<(&Feature, Vec<&[Point]>)> = Vec::new();
    let mut index = HashMap::new();
    for feature in pieces {
        let lines: Vec<&[Point]> = match &feature.geometry {
            Geometry::LineString(line) => vec![line],
            Geometry::MultiLineString(lines) => lines.iter().map(Vec::as_slice).collect(),
            _ => continue,
        };
        let identity =
            serde_json::to_string(&feature.properties).expect("JSON properties serialize");
        let group = *index.entry(identity).or_insert_with(|| {
            groups.push((feature, Vec::new()));
            groups.len() - 1
        });
        groups[group].1.extend(lines);
    }
    groups
        .into_iter()
        .filter_map(|(first, lines)| {
            let runs = ring_runs(lines);
            (!runs.is_empty()).then(|| {
                let mut outline = first.clone();
                outline.geometry = Geometry::MultiLineString(runs);
                outline
            })
        })
        .collect()
}

/// Chain boundary segments into maximal polylines. Each distinct undirected,
/// non-degenerate segment is emitted exactly once. Open runs start where the
/// boundary leaves the segment set; closed rings repeat their first vertex,
/// with the seam placed at the straightest vertex so the strip's two butt ends
/// meet almost collinearly.
pub(super) fn ring_runs<'a>(lines: impl IntoIterator<Item = &'a [Point]>) -> Vec<Vec<Point>> {
    let mut vertices: Vec<Point> = Vec::new();
    let mut ids: HashMap<PointKey, usize> = HashMap::new();
    let mut seen = HashSet::new();
    let mut segments: Vec<[usize; 2]> = Vec::new();
    for line in lines {
        for pair in line.windows(2) {
            let [a, b] = [pair[0], pair[1]].map(|p| {
                *ids.entry(key(p)).or_insert_with(|| {
                    vertices.push(p);
                    vertices.len() - 1
                })
            });
            if a != b && seen.insert([a.min(b), a.max(b)]) {
                segments.push([a, b]);
            }
        }
    }
    let mut incident = vec![Vec::new(); vertices.len()];
    for (s, [a, b]) in segments.iter().enumerate() {
        incident[*a].push(s);
        incident[*b].push(s);
    }
    let mut used = vec![false; segments.len()];
    let walk = |start: usize, first: usize, used: &mut Vec<bool>| {
        let (mut v, mut s, mut run) = (start, first, vec![start]);
        loop {
            used[s] = true;
            let [a, b] = segments[s];
            v = if a == v { b } else { a };
            run.push(v);
            if incident[v].len() != 2 {
                break run;
            }
            match incident[v].iter().find(|n| !used[**n]) {
                Some(next) => s = *next,
                None => break run,
            }
        }
    };
    let mut runs = Vec::new();
    // Open runs, preferring the stored (ring) direction.
    for reverse in [false, true] {
        for s in 0..segments.len() {
            let start = segments[s][usize::from(reverse)];
            if !used[s] && incident[start].len() != 2 {
                runs.push(walk(start, s, &mut used));
            }
        }
    }
    // Every remaining segment lies on a ring wholly inside the set.
    for s in 0..segments.len() {
        if !used[s] {
            let ring = walk(segments[s][0], s, &mut used);
            runs.push(straightest_seam(&ring, &vertices));
        }
    }
    runs.into_iter()
        .map(|run| run.into_iter().map(|v| vertices[v]).collect())
        .collect()
}

/// Rotate a closed vertex cycle (first == last) to start at its straightest
/// vertex.
fn straightest_seam(ring: &[usize], vertices: &[Point]) -> Vec<usize> {
    let open = &ring[..ring.len() - 1];
    let n = open.len();
    let turn = |k: usize| {
        let d0 = sub(vertices[open[k]], vertices[open[(k + n - 1) % n]]);
        let d1 = sub(vertices[open[(k + 1) % n]], vertices[open[k]]);
        dot(d0, d1) / (norm(d0) * norm(d1))
    };
    let seam = (0..n).fold(0, |best, k| if turn(k) > turn(best) { k } else { best });
    open[seam..].iter().chain(&open[..=seam]).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn edge(a: Point, b: Point) -> [PointKey; 2] {
        let mut pair = [key(a), key(b)];
        pair.sort();
        pair
    }

    /// Every run is a contiguous source polyline; return its undirected edges.
    fn run_edges(runs: &[Vec<Point>]) -> Vec<[PointKey; 2]> {
        runs.iter()
            .inspect(|run| assert!(run.len() >= 2, "run {run:?}"))
            .flat_map(|run| run.windows(2).map(|p| edge(p[0], p[1])))
            .collect()
    }

    fn circle(n: usize, radius: f64) -> Vec<Point> {
        (0..n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                [radius * a.cos(), radius * a.sin(), 0.]
            })
            .collect()
    }

    #[test]
    fn closed_ring_is_one_closed_strip_without_gap_or_overlap() {
        let ring = circle(9, 10.);
        // Edges arrive per triangle, in arbitrary order and orientation.
        let mut segments: Vec<Vec<Point>> = (0..ring.len())
            .map(|i| vec![ring[i], ring[(i + 1) % ring.len()]])
            .collect();
        segments.reverse();
        segments[2].reverse();
        segments.push(segments[0].clone()); // duplicate edge
        segments.push(vec![ring[3], ring[3]]); // degenerate edge
        let runs = ring_runs(segments.iter().map(Vec::as_slice));
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.len(), ring.len() + 1);
        assert_eq!(run.first(), run.last(), "closed ring stays closed");
        let edges = run_edges(&runs);
        let unique: BTreeSet<_> = edges.iter().copied().collect();
        assert_eq!(unique.len(), edges.len(), "no overlapping segment");
        let expected: BTreeSet<_> = (0..ring.len())
            .map(|i| edge(ring[i], ring[(i + 1) % ring.len()]))
            .collect();
        assert_eq!(unique, expected, "no gap");
    }

    #[test]
    fn closed_ring_seam_sits_at_the_straightest_vertex() {
        // A square with one extra collinear vertex at (5, 0).
        let ring = [
            [0., 0., 0.],
            [5., 0., 0.],
            [10., 0., 0.],
            [10., 10., 0.],
            [0., 10., 0.],
        ];
        let segments: Vec<Vec<Point>> = (0..ring.len())
            .map(|i| vec![ring[i], ring[(i + 1) % ring.len()]])
            .collect();
        let runs = ring_runs(segments.iter().map(Vec::as_slice));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0][0], [5., 0., 0.]);
        assert_eq!(runs[0].last(), Some(&[5., 0., 0.]));
    }

    #[test]
    fn open_runs_split_only_where_the_ring_leaves_the_set() {
        let ring = circle(12, 10.);
        // Edges 0..=3 and 7..=9 owned; 10 and 11 not; 4..=6 not.
        let owned = [9, 0, 2, 8, 1, 3, 7];
        let segments: Vec<Vec<Point>> = owned
            .iter()
            .map(|i| vec![ring[*i], ring[(i + 1) % ring.len()]])
            .collect();
        let runs = ring_runs(segments.iter().map(Vec::as_slice));
        assert_eq!(runs.len(), 2);
        let mut lengths: Vec<_> = runs.iter().map(Vec::len).collect();
        lengths.sort();
        assert_eq!(lengths, [4, 5]);
        // Stored (ring) direction is kept.
        assert!(runs.contains(&ring[0..5].to_vec()));
        assert!(runs.contains(&ring[7..11].to_vec()));
    }

    #[test]
    fn rings_meeting_at_a_vertex_split_there() {
        // Two triangles sharing the apex (0, 0): a pinch vertex of degree 4.
        let a = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.]];
        let b = [[0., 0., 0.], [-1., 0., 0.], [-1., -1., 0.]];
        let segments: Vec<Vec<Point>> = [a, b]
            .iter()
            .flat_map(|t| (0..3).map(|i| vec![t[i], t[(i + 1) % 3]]))
            .collect();
        let runs = ring_runs(segments.iter().map(Vec::as_slice));
        assert_eq!(runs.len(), 2);
        assert!(runs
            .iter()
            .all(|r| r.len() == 4 && r[0] == [0., 0., 0.] && r[3] == r[0]));
    }

    #[test]
    fn source_rings_close_at_the_midpoint_of_the_longest_edge() {
        let positions = [
            [0., 0., 0.],
            [10., 0., 0.],
            [10., 4., 0.],
            [0., 4., 0.],
            [2., 1., 0.],
            [3., 1., 0.],
            [3., 2., 0.],
        ];
        let rings = source_rings(&positions, &[0, 1, 2, 3, u32::MAX, 4, 6, 5]);
        assert_eq!(
            rings,
            [
                vec![
                    [5., 0., 0.],
                    [10., 0., 0.],
                    [10., 4., 0.],
                    [0., 4., 0.],
                    [0., 0., 0.],
                    [5., 0., 0.],
                ],
                vec![
                    [2.5, 1.5, 0.],
                    [3., 2., 0.],
                    [3., 1., 0.],
                    [2., 1., 0.],
                    [2.5, 1.5, 0.],
                ],
            ]
        );
    }

    /// Split like the spool: anything over `limit` vertices is cut again.
    fn admit(feature: Feature, limit: usize, admitted: &mut Vec<Feature>) {
        if feature.geometry.size() <= limit {
            admitted.push(feature);
            return;
        }
        let parts =
            super::super::store::split(&feature, false, &mut Default::default(), &mut Vec::new())
                .unwrap();
        for part in parts {
            admit(part, limit, admitted);
        }
    }

    /// A big polygon partitioned by the producer: the line splitter cuts its
    /// outline, and the pieces land in two tiles. Each tile draws one strip
    /// that follows the ring, the two strips meet end to end and together draw
    /// every source edge once. A tile holding both pieces draws one closed
    /// ring whose seam is the inserted collinear midpoint.
    #[test]
    fn partitioned_polygon_outline_is_contiguous_ring_runs_per_tile() {
        let mut ring = circle(48, 1000.);
        ring.push(ring[0]);
        let feature: Feature = serde_json::from_value(serde_json::json!({
            "properties": {"_source_id": "big", "_source_layer": "test"},
            "geometry": {"type": "Polygon", "coordinates": [ring]}
        }))
        .unwrap();
        ring.pop();
        let n = ring.len();
        let mut admitted = Vec::new();
        admit(feature, 30, &mut admitted);
        let fills = admitted.iter().filter(|f| !is_outline(f)).count();
        assert!(fills >= 4, "{fills} fill fragments");
        let pieces: Vec<&Feature> = admitted.iter().filter(|f| is_outline(f)).collect();
        assert_eq!(pieces.len(), 2, "the line splitter halves the ring once");
        assert!(pieces
            .iter()
            .all(|f| f.surface_fragment && f.intrinsic.is_none()));

        // Independent oracle: the source edges, with one edge bisected by the
        // single vertex that is not a source vertex.
        let drawn: Vec<Point> = pieces
            .iter()
            .flat_map(|f| f.geometry.points())
            .copied()
            .collect();
        let source: HashSet<PointKey> = ring.iter().map(|p| key(*p)).collect();
        let extra: HashSet<PointKey> = drawn
            .iter()
            .map(|p| key(*p))
            .filter(|k| !source.contains(k))
            .collect();
        assert_eq!(extra.len(), 1, "one seam vertex");
        let bisected = (0..n)
            .find(|i| extra.contains(&key(mul(add(ring[*i], ring[(i + 1) % n]), 0.5))))
            .expect("the seam vertex bisects a source edge");
        let seam = mul(add(ring[bisected], ring[(bisected + 1) % n]), 0.5);
        let mut expected: BTreeSet<_> = (0..n)
            .filter(|i| *i != bisected)
            .map(|i| edge(ring[i], ring[(i + 1) % n]))
            .collect();
        expected.insert(edge(ring[bisected], seam));
        expected.insert(edge(seam, ring[(bisected + 1) % n]));

        let strips = |tile: &[&Feature]| {
            let outlines = fragment_outlines(tile.iter().copied());
            assert_eq!(outlines.len(), 1, "pieces of one feature share an outline");
            assert_eq!(outlines[0].properties["_source_id"], "big");
            let Geometry::MultiLineString(runs) = &outlines[0].geometry else {
                panic!("outline geometry")
            };
            for run in runs {
                for pair in run.windows(2) {
                    assert!(expected.contains(&edge(pair[0], pair[1])), "continuity");
                }
            }
            runs.clone()
        };
        let (a, b) = (strips(&pieces[..1]), strips(&pieces[1..]));
        assert_eq!((a.len(), b.len()), (1, 1), "one strip per tile");
        assert_eq!(a[0].len() + b[0].len(), n + 3, "split vertex in both");
        assert!(
            a[0].last() == b[0].first() || b[0].last() == a[0].first(),
            "the strips meet end to end"
        );
        let mut edges = run_edges(&a);
        edges.extend(run_edges(&b));
        assert_eq!(edges.len(), expected.len(), "no overlap across tiles");
        assert_eq!(
            edges.into_iter().collect::<BTreeSet<_>>(),
            expected,
            "no gap"
        );

        let whole = strips(&pieces);
        assert_eq!(whole.len(), 1);
        assert_eq!(whole[0].len(), n + 2);
        assert_eq!(
            whole[0].first(),
            Some(&seam),
            "seam at the collinear midpoint"
        );
        assert_eq!(whole[0].last(), Some(&seam));
        assert_eq!(run_edges(&whole).len(), expected.len());
    }
}
