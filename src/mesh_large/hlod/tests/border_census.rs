//! Independent D11 census. Producer subarc IDs are only lookup hints. Derive exact
//! incidence, live chains and source parameters again from independently decoded leaves;
//! check actual delivered edges, every source breakpoint, every owner and every depth mix.
use super::*;
#[path = "border_census_exact.rs"]
mod exact;
type Edge = (Point, Point);
type Q = [f64; 3];
fn q(p: Point) -> Q {
    p.map(|b| f64::from(f32::from_bits(b)))
}
fn distance(a: Q, b: Q) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}
fn point(v: &Value) -> Point {
    std::array::from_fn(|i| v[i].as_u64().unwrap() as u32)
}
fn owners(leaves: &BTreeSet<usize>, rows: &[Value], depth: u32) -> BTreeSet<(u64, u64, u64, u64)> {
    leaves
        .iter()
        .map(|&i| {
            let row = &rows[i];
            let d = row["depth"].as_u64().unwrap() as u32;
            let cd = row["cellDepth"].as_u64().unwrap() as u32;
            let cell = row["cell"].as_u64().unwrap();
            let path = row["path"].as_u64().unwrap();
            if d <= depth {
                (1, i as u64, 0, 0)
            } else if depth <= cd {
                (0, u64::from(depth), cell >> (3 * (cd - depth)), 0)
            } else {
                (0, u64::from(cd), cell, path >> (3 * (d - depth)))
            }
        })
        .collect()
}
/// The native D11 fold rule, re-derived here from decoded faces and independently
/// counted leaf incidences. Coincident observations are separate from refusals:
/// leaf encoding may already have collapsed a thin plate onto its opposite sheet.
#[derive(Debug, Default)]
struct FoldCensus {
    unsupported_edges: usize,
    coincident_groups: usize,
    duplicate_faces: usize,
    reversed_twin_groups: usize,
    unsupported_coincident_groups: usize,
}
fn corners(face: [Point; 3]) -> [Point; 3] {
    let mut c = face;
    c.sort_unstable();
    c
}
fn face_directions(faces: &[[Point; 3]]) -> HashMap<[Point; 3], [usize; 2]> {
    let mut groups = HashMap::<_, [usize; 2]>::new();
    for &f in faces {
        if f[0] == f[1] || f[1] == f[2] || f[0] == f[2] {
            continue;
        }
        let k = (0..3).min_by_key(|&k| f[k]).unwrap();
        let oriented = [f[k], f[(k + 1) % 3], f[(k + 2) % 3]];
        let key = corners(f);
        groups.entry(key).or_default()[usize::from(oriented != key)] += 1;
    }
    groups
}
/// Source groups are derived only from decoded leaves. Keep each corner set's
/// counts by leaf so a coarse tile can sum exactly its own descendants, including
/// groups split across leaves. Orientation does not affect coincidence licensing.
#[derive(Default)]
struct SourceCoincidence {
    by_face: HashMap<[Point; 3], BTreeMap<usize, usize>>,
    at_point: HashMap<Point, Vec<[Point; 3]>>,
}
impl SourceCoincidence {
    fn add_leaf(&mut self, leaf: usize, faces: &[[Point; 3]]) {
        for (face, directions) in face_directions(faces) {
            let counts = self.by_face.entry(face).or_default();
            if counts.is_empty() {
                for p in face {
                    self.at_point.entry(p).or_default().push(face);
                }
            }
            *counts.entry(leaf).or_default() += directions[0] + directions[1];
        }
    }
    fn licenses(&self, face: [Point; 3], n: usize, descendants: std::ops::Range<usize>) -> bool {
        face.into_iter().all(|p| {
            self.at_point.get(&p).is_some_and(|groups| {
                groups.iter().any(|source| {
                    self.by_face[source]
                        .range(descendants.clone())
                        .map(|(_, &count)| count)
                        .sum::<usize>()
                        >= n
                })
            })
        })
    }
}
/// Check only the unsupported direction: source-supported faces need no geometric
/// fallback. Nonzero wings live in a 2D normal plane, where at most four vectors
/// can be pairwise nonacute; reject larger sets before doing any pair work.
fn nonacute_side(edge: Edge, side: &[Point]) -> bool {
    use std::cmp::Ordering::Greater;
    if side.len() > 4 {
        return false;
    }
    side.iter().enumerate().all(|(i, &p)| {
        exact::wedge_sign(edge, p, p) == Greater
            && side[i + 1..]
                .iter()
                .all(|&other| exact::wedge_sign(edge, p, other) != Greater)
    })
}
fn fold_census(
    faces: &[[Point; 3]],
    actual: &HashMap<Edge, [u32; 2]>,
    support: &HashMap<Edge, [u32; 2]>,
    source_coincidence: &SourceCoincidence,
    descendants: std::ops::Range<usize>,
) -> FoldCensus {
    let mut c = FoldCensus::default();
    let mut incident = HashMap::<Edge, [Vec<Point>; 2]>::new();
    for f in faces {
        if f[0] == f[1] || f[1] == f[2] || f[0] == f[2] {
            continue;
        }
        for i in 0..3 {
            let (a, b) = (f[i], f[(i + 1) % 3]);
            incident.entry(seg(a, b)).or_default()[usize::from(a > b)].push(f[(i + 2) % 3]);
        }
    }
    for (&edge, &[forward, reverse]) in actual {
        if forward < 2 && reverse < 2 {
            continue;
        }
        let allowed = support.get(&edge).copied().unwrap_or([0, 0]);
        // Directional minima were taken over EVERY represented source subedge.
        // Balanced 2/2 needs the same check as 2/1, and a supported side must not
        // be subjected to the other side's geometric fallback.
        if [(0, forward, reverse), (1, reverse, forward)]
            .into_iter()
            .any(|(direction, n, other)| {
                n >= 2
                    && allowed[direction] < n
                    && (other == 0 || !nonacute_side(edge, &incident[&edge][direction]))
            })
        {
            c.unsupported_edges += 1;
        }
    }
    for (face, directions) in face_directions(faces) {
        let n = directions[0] + directions[1];
        c.duplicate_faces += directions
            .iter()
            .map(|n| n.saturating_sub(1))
            .sum::<usize>();
        c.reversed_twin_groups += usize::from(directions[0] > 0 && directions[1] > 0);
        if n < 2 {
            continue;
        }
        c.coincident_groups += 1;
        // Every coarse corner needs a source corner group with at least n
        // faces in this tile's descendant leaves. Corners may use different
        // groups; unrelated twins and edge incidence alone do not license them.
        if !source_coincidence.licenses(face, n, descendants.clone()) {
            c.unsupported_coincident_groups += 1;
        }
    }
    c
}

struct Graph {
    adjacent: HashMap<Point, Vec<Edge>>,
    live: HashSet<Point>,
}
fn graph(
    depth: u32,
    rows: &[Value],
    edges: &HashMap<Edge, BTreeSet<usize>>,
    points: &HashMap<Point, BTreeSet<usize>>,
    faces: &HashMap<Edge, usize>,
) -> Graph {
    let coarsened: HashMap<Edge, _> = edges
        .iter()
        .map(|(e, l)| (*e, owners(l, rows, depth)))
        .filter(|(_, l)| l.len() > 1)
        .collect();
    let mut adjacent: HashMap<Point, Vec<Edge>> = HashMap::new();
    for e in coarsened.keys() {
        adjacent.entry(e.0).or_default().push(*e);
        adjacent.entry(e.1).or_default().push(*e);
    }
    let open: HashSet<Point> = faces
        .iter()
        .filter(|(_, n)| **n == 1)
        .flat_map(|(e, _)| [e.0, e.1])
        .collect();
    let mut live = HashSet::new();
    for (p, l) in points {
        let owned = owners(l, rows, depth);
        if owned.len() < 2 {
            continue;
        }
        let edges_here = adjacent.get(p).map_or(&[][..], Vec::as_slice);
        let terminal = l
            .iter()
            .any(|&i| rows[i]["depth"].as_u64().unwrap() <= u64::from(depth));
        if edges_here.len() != 2
            || terminal
            || open.contains(p)
            || edges_here.iter().any(|e| coarsened[e] != owned)
        {
            live.insert(*p);
        }
    }
    Graph { adjacent, live }
}

pub(super) fn check(
    dir: &Path,
    tiles: &[Tile],
    edges: &HashMap<Edge, BTreeSet<usize>>,
    points: &HashMap<Point, BTreeSet<usize>>,
    faces: &HashMap<Edge, usize>,
    c: &mut Census,
) {
    let leaf_rows = rows(&dir.join("leaves.jsonl"));
    let report: Value =
        serde_json::from_slice(&fs::read(dir.join("resources.json")).unwrap()).unwrap();
    let path = report["hlod"]["diagnostics"]["audit"]["path"]
        .as_str()
        .expect("D11 census requires durable audit evidence");
    let audit = rows(Path::new(path));
    let records: HashMap<String, &Value> = audit
        .iter()
        .filter(|r| r["kind"] == "tile")
        .map(|r| (r["tile"]["uri"].as_str().unwrap().to_owned(), r))
        .collect();
    let mut source_directions: HashMap<Edge, BTreeMap<usize, [u32; 2]>> = HashMap::new();
    let mut source_coincidence = SourceCoincidence::default();
    for t in tiles.iter().filter(|t| t.leaf) {
        source_coincidence.add_leaf(t.begin, &t.faces);
        for f in &t.faces {
            if f[0] == f[1] || f[1] == f[2] || f[0] == f[2] {
                continue;
            }
            for i in 0..3 {
                let (a, b) = (f[i], f[(i + 1) % 3]);
                source_directions
                    .entry(seg(a, b))
                    .or_default()
                    .entry(t.begin)
                    .or_default()[usize::from(a > b)] += 1;
            }
        }
    }
    let reverse = |counts: [u32; 2]| [counts[1], counts[0]];
    let mut graphs = HashMap::new();
    let mut coverage: Vec<HashMap<Edge, [Q; 2]>> = Vec::new();
    let mut vertices: Vec<HashMap<Point, Q>> = Vec::new();
    let mut errors = Vec::new();
    c.contracted_source_segments = c.missing_segments;
    c.contracted_source_vertices = c.missing_vertices;
    c.missing_segments = 0;
    c.missing_vertices = 0;
    for t in tiles {
        let crosses = |l: &BTreeSet<usize>| {
            l.iter().any(|x| (t.begin..t.end).contains(x))
                && l.iter().any(|x| !(t.begin..t.end).contains(x))
        };
        let mut actual: HashMap<Edge, [u32; 2]> = HashMap::new();
        for f in &t.faces {
            if f[0] == f[1] || f[1] == f[2] || f[0] == f[2] {
                continue;
            }
            for i in 0..3 {
                let (a, b) = (f[i], f[(i + 1) % 3]);
                actual.entry(seg(a, b)).or_default()[usize::from(a > b)] += 1;
            }
        }
        let mut chord_support = HashMap::<Edge, [u32; 2]>::new();
        let actual_points: HashSet<Point> = t.faces.iter().flatten().copied().collect();
        let mut covered = HashMap::new();
        let mut mapped = HashMap::new();
        if t.leaf {
            for (&e, l) in edges {
                if crosses(l) {
                    covered.insert(e, [q(e.0), q(e.1)]);
                }
            }
            for (&p, l) in points {
                if crosses(l) {
                    mapped.insert(p, q(p));
                }
            }
            errors.push(0.);
        } else {
            let r = records[&t.uri];
            let tile = &r["tile"];
            let contract = &r["contract"];
            let depth = contract["depth"].as_u64().unwrap() as u32;
            let g = tile["geometricError"].as_f64().unwrap();
            let scale = contract["scale"].as_f64().unwrap();
            assert!(g >= scale && scale.is_finite() && scale >= 0.);
            errors.push(g);
            let graph = graphs
                .entry(depth)
                .or_insert_with(|| graph(depth, &leaf_rows, edges, points, faces));
            let mut chains: HashMap<(Edge, Point), Vec<Point>> = HashMap::new();
            // Re-derive each hinted chain from incidence. It stops at an independently
            // identified junction or closes a cycle. No producer geometry/mapping code.
            for cert in contract["certificates"].as_array().unwrap() {
                let first_edge = seg(point(&cert["chain"][0]), point(&cert["chain"][1]));
                let origin = point(&cert["origin"]);
                let first = cert["first"].as_u64().unwrap() as usize;
                let end = cert["end"].as_u64().unwrap() as usize;
                let chain = chains.entry((first_edge, origin)).or_insert_with(|| {
                    let (mut at, mut edge) = (origin, first_edge);
                    let mut path = vec![at];
                    let mut visited = HashSet::new();
                    loop {
                        assert!(
                            graph.adjacent.get(&at).is_some_and(|v| v.contains(&edge)),
                            "hint is not an active source edge"
                        );
                        assert!(visited.insert(edge), "hinted chain repeats an edge");
                        at = if edge.0 == at {
                            edge.1
                        } else {
                            assert_eq!(edge.1, at);
                            edge.0
                        };
                        path.push(at);
                        if at == origin || graph.live.contains(&at) {
                            break;
                        }
                        let next = &graph.adjacent[&at];
                        assert_eq!(next.len(), 2);
                        edge = *next.iter().find(|e| **e != edge).unwrap();
                    }
                    path
                });
                assert!(
                    first < end && end < chain.len(),
                    "hinted source range out of bounds"
                );
                let a = point(&cert["endpoints"][0]);
                let b = point(&cert["endpoints"][1]);
                assert_eq!(
                    (a, b),
                    (chain[first], chain[end]),
                    "wrong source endpoint IDs"
                );
                if !actual.contains_key(&seg(a, b)) {
                    c.missing_segments += 1;
                }
                let epsilon = cert["epsilon"].as_f64().unwrap();
                assert!(epsilon >= 0. && epsilon <= scale / 16.);
                let mut at_values = Vec::with_capacity(end - first + 1);
                for (i, &p) in chain.iter().enumerate().take(end + 1).skip(first) {
                    let u = (i - first) as f64 / (end - first) as f64;
                    let value = std::array::from_fn(|axis| q(a)[axis] * (1. - u) + q(b)[axis] * u);
                    if distance(value, q(p)) > epsilon + 1e-10 {
                        c.certificate_failures += 1;
                    }
                    if let Some(old) = mapped.insert(p, value) {
                        assert!(distance(old, value) < 1e-10);
                    }
                    at_values.push(value);
                }
                let delivered = actual.get(&seg(a, b)).copied().unwrap_or([0, 0]);
                let delivered = if a < b { delivered } else { reverse(delivered) };
                let mut wrong_winding = false;
                let mut support = [u32::MAX; 2];
                for i in first..end {
                    let e = seg(chain[i], chain[i + 1]);
                    let allowed = source_directions[&e]
                        .range(t.begin..t.end)
                        .fold([0, 0], |a, (_, b)| [a[0] + b[0], a[1] + b[1]]);
                    let allowed = if chain[i] < chain[i + 1] {
                        allowed
                    } else {
                        reverse(allowed)
                    };
                    wrong_winding |= delivered[0] > allowed[0] || delivered[1] > allowed[1];
                    support = [support[0].min(allowed[0]), support[1].min(allowed[1])];
                    assert!(
                        crosses(&edges[&e]),
                        "certificate claims another tile's source edge"
                    );
                    let v = [at_values[i - first], at_values[i - first + 1]];
                    let v = if e.0 == chain[i] { v } else { [v[1], v[0]] };
                    assert!(
                        covered.insert(e, v).is_none(),
                        "source subarc covered twice"
                    );
                    if edges[&e]
                        .iter()
                        .any(|&l| leaf_rows[l]["depth"].as_u64().unwrap() <= u64::from(depth))
                    {
                        assert_eq!(v, [q(e.0), q(e.1)], "terminal leaf arc moved");
                    }
                }
                c.certificate_winding_failures += usize::from(wrong_winding);
                chord_support.insert(seg(a, b), if a < b { support } else { reverse(support) });
            }
            for (&p, l) in points {
                if crosses(l) && (graph.live.contains(&p) || !graph.adjacent.contains_key(&p)) {
                    if !actual_points.contains(&p) {
                        c.missing_vertices += 1;
                    }
                    mapped.insert(p, q(p));
                }
            }
        }
        if !t.leaf {
            for &edge in actual.keys() {
                chord_support.entry(edge).or_insert_with(|| {
                    source_directions.get(&edge).map_or([0, 0], |owners| {
                        owners
                            .range(t.begin..t.end)
                            .fold([0, 0], |a, (_, b)| [a[0] + b[0], a[1] + b[1]])
                    })
                });
            }
            let folds = fold_census(
                &t.faces,
                &actual,
                &chord_support,
                &source_coincidence,
                t.begin..t.end,
            );
            // A2: a stage-2/3 last-resort tile admits its declared folds.
            if !t.folds {
                c.unsupported_same_direction_edges += folds.unsupported_edges;
                c.unsupported_coincident_face_groups += folds.unsupported_coincident_groups;
            }
            c.coincident_coarse_face_groups += folds.coincident_groups;
            c.duplicate_coarse_faces += folds.duplicate_faces;
            c.reversed_twin_face_groups += folds.reversed_twin_groups;
        }
        for (e, l) in edges {
            if crosses(l) && !covered.contains_key(e) {
                c.missing_segments += 1;
            }
        }
        for (p, l) in points {
            if crosses(l) && !mapped.contains_key(p) {
                c.missing_vertices += 1;
            }
        }
        coverage.push(covered);
        vertices.push(mapped);
    }
    // Each original source breakpoint is a combined piecewise-linear breakpoint for
    // every pair of delivered curves, so endpoint maxima bound the complete curves.
    for x in 0..tiles.len() {
        for y in x + 1..tiles.len() {
            let (a, b) = (&tiles[x], &tiles[y]);
            if a.begin < b.end && b.begin < a.end {
                continue;
            }
            let mut gap = 0f64;
            let mut met = false;
            for (e, v) in &coverage[x] {
                if let Some(w) = coverage[y].get(e) {
                    met = true;
                    gap = gap.max(distance(v[0], w[0])).max(distance(v[1], w[1]));
                }
            }
            for (p, v) in &vertices[x] {
                if let Some(w) = vertices[y].get(p) {
                    met = true;
                    gap = gap.max(distance(*v, *w));
                }
            }
            if !met {
                continue;
            }
            if a.depth == b.depth {
                c.max_same_depth_gap_m = c.max_same_depth_gap_m.max(gap);
            } else {
                let g = if a.depth < b.depth {
                    errors[x]
                } else {
                    errors[y]
                };
                c.max_mixed_depth_gap_m = c.max_mixed_depth_gap_m.max(gap);
                if gap > g / 8. + 1e-10 {
                    c.certificate_failures += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod fold_controls {
    use super::*;

    const A: [f32; 3] = [0., 0., 0.];
    const B: [f32; 3] = [0., 0., 1.];
    fn decoded(faces: &[[[f32; 3]; 3]]) -> Vec<[Point; 3]> {
        faces
            .iter()
            .map(|f| f.map(|p| p.map(f32::to_bits)))
            .collect()
    }
    fn probe(faces: &[[[f32; 3]; 3]], allowed: [u32; 2]) -> FoldCensus {
        probe_with_leaves(faces, allowed, &[], 0..0)
    }
    fn probe_with_leaves(
        faces: &[[[f32; 3]; 3]],
        allowed: [u32; 2],
        leaves: &[Vec<[[f32; 3]; 3]>],
        descendants: std::ops::Range<usize>,
    ) -> FoldCensus {
        let faces = decoded(faces);
        let mut actual = HashMap::<Edge, [u32; 2]>::new();
        for f in &faces {
            for i in 0..3 {
                let (a, b) = (f[i], f[(i + 1) % 3]);
                actual.entry(seg(a, b)).or_default()[usize::from(a > b)] += 1;
            }
        }
        let edge = seg(A.map(f32::to_bits), B.map(f32::to_bits));
        // All unrelated edges retain independently supplied source incidence, so
        // each control isolates the tested edge or coincident corner group.
        let mut support = actual.clone();
        support.insert(edge, allowed);
        let mut source = SourceCoincidence::default();
        for (leaf, faces) in leaves.iter().enumerate() {
            source.add_leaf(leaf, &decoded(faces));
        }
        fold_census(&faces, &actual, &support, &source, descendants)
    }

    #[test]
    fn directed_excess_checks_three_one_way_and_two_against_one() {
        let faces = [
            [A, B, [1., 0., 0.]],
            [A, B, [1., 1., 0.]],
            [A, B, [2., 1., 0.]],
        ];
        assert_eq!(probe(&faces, [1, 0]).unsupported_edges, 1);
        let mixed = [faces[0], faces[1], [B, A, [-1., 0., 0.]]];
        assert_eq!(probe(&mixed, [1, 1]).unsupported_edges, 1);
        // Proper nonmanifold source multiplicity admits even a one-way spine.
        assert_eq!(probe(&faces, [3, 0]).unsupported_edges, 0);
        assert_eq!(probe(&faces, [2, 0]).unsupported_edges, 1);
    }

    #[test]
    fn unwitnessed_balanced_sheets_need_nonacute_wings_in_each_direction() {
        let forward = [A, B, [1., 0., 0.]];
        let reverse = [B, A, [-1., 0., 0.]];
        for third in [[0., 1., 0.], [-1., 0., 0.]] {
            let faces = [forward, [A, B, third], reverse];
            assert_eq!(probe(&faces, [0, 0]).unsupported_edges, 0);
        }
        for third in [[1., 1., 0.], [0., 0., 0.5]] {
            let faces = [forward, [A, B, third], reverse];
            assert_eq!(probe(&faces, [0, 0]).unsupported_edges, 1);
        }
        // Perpendicular pages alone cannot license a one-way unsupported fold.
        assert_eq!(
            probe(&[forward, [A, B, [0., 1., 0.]]], [0, 0]).unsupported_edges,
            1
        );
    }

    #[test]
    fn balanced_two_two_on_ordinary_one_one_source_checks_both_directions() {
        let forward = [A, B, [1., 0., 0.]];
        let reverse = [B, A, [-1., 0., 0.]];
        for (left, right, unsupported) in [
            ([0., 1., 0.], [0., -1., 0.], 0),
            ([1., 1., 0.], [0., -1., 0.], 1),
            ([0., 1., 0.], [-1., -1., 0.], 1),
            ([1., 1., 0.], [-1., -1., 0.], 1),
        ] {
            let faces = [forward, [A, B, left], reverse, [B, A, right]];
            assert_eq!(probe(&faces, [1, 1]).unsupported_edges, unsupported);
        }
    }

    #[test]
    fn supported_direction_bypasses_only_its_own_geometric_fallback() {
        // Three acute/collinear forward wings are source-supported. Only the two
        // unsupported reverse wings need geometry, even though they are fewer.
        for (third, unsupported) in [([0., -1., 0.], 0), ([-1., -1., 0.], 1)] {
            let faces = [
                [A, B, [1., 0., 0.]],
                [A, B, [1., 1., 0.]],
                [A, B, [0., 0., 0.5]],
                [B, A, [-1., 0., 0.]],
                [B, A, third],
            ];
            assert_eq!(probe(&faces, [3, 1]).unsupported_edges, unsupported);
            let reversed = faces.map(|[a, b, c]| [b, a, c]);
            assert_eq!(probe(&reversed, [1, 3]).unsupported_edges, unsupported);
        }
    }

    #[test]
    fn near_right_wedges_have_no_angular_tolerance() {
        // The minimum subnormal also gets lost if the projected dot is evaluated
        // by ordinary f64 arithmetic: its contribution is added to 0.5 * 0.25.
        for tiny in [1e-7, f32::from_bits(1)] {
            for (x, unsupported) in [(tiny, 1), (0., 0), (-tiny, 0)] {
                let faces = [
                    [A, B, [1., 0., 0.5]],
                    [A, B, [x, 1., 0.25]],
                    [B, A, [-1., 0., 0.]],
                ];
                assert_eq!(
                    probe(&faces, [1, 1]).unsupported_edges,
                    unsupported,
                    "x={x:?}"
                );
                let reversed = faces.map(|[a, b, c]| [b, a, c]);
                assert_eq!(probe(&reversed, [1, 1]).unsupported_edges, unsupported);
            }
        }
    }

    #[test]
    fn exact_wedge_sign_survives_extreme_finite_f32_and_cancellation() {
        use std::cmp::Ordering::{Equal, Greater, Less};
        let bits = |p: [f32; 3]| p.map(|v| if v == 0. { 0 } else { v.to_bits() });
        let tiny = f32::from_bits(1);
        let huge = f32::MAX;
        // Each direction of exponent imbalance is intentional: one must retain
        // subnormal normal-plane terms beside huge along-edge products, the
        // other huge normal-plane terms beside a subnormal edge length.
        for (along, across) in [(huge, tiny), (tiny, huge)] {
            // Rotate the tested axis so all three dot-product terms matter.
            for axis in 0..3 {
                let rotate = |p: [f32; 3]| bits(std::array::from_fn(|i| p[(i + axis) % 3]));
                let a = rotate([-along, 0., 0.]);
                let b = rotate([along, 0., 0.]);
                let c = rotate([0., across, 0.]);
                for (y, expected) in [(across, Greater), (0., Equal), (-across, Less)] {
                    let e = rotate([0., y, across]);
                    for edge in [(a, b), (b, a)] {
                        assert_eq!(exact::wedge_sign(edge, c, e), expected);
                        assert_eq!(exact::wedge_sign(edge, e, c), expected);
                    }
                }
                assert_eq!(exact::wedge_sign((a, b), c, c), Greater);
                assert_eq!(exact::wedge_sign((a, b), rotate([0., 0., 0.]), c), Equal);
                // A different origin on the same line changes neither wing.
                assert_eq!(exact::wedge_sign((rotate([0., 0., 0.]), b), c, c), Greater);
            }
        }
    }

    #[test]
    fn exact_wedge_matches_bounded_integer_reference_across_exponents() {
        // i128 is an independent arithmetic reference on these small integers;
        // exact power-of-two rescaling moves their bits across the limb range.
        let mut state = 17u32;
        for _ in 0..64 {
            let p: [[i128; 3]; 4] = std::array::from_fn(|_| {
                std::array::from_fn(|_| {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    i128::from((state >> 16) % 15) - 7
                })
            });
            let diff = |i: usize| -> [i128; 3] { std::array::from_fn(|j| p[i][j] - p[0][j]) };
            let dot = |x: [i128; 3], y: [i128; 3]| (0..3).map(|i| x[i] * y[i]).sum::<i128>();
            let (d, u, w) = (diff(1), diff(2), diff(3));
            let expected = (dot(d, d) * dot(u, w) - dot(u, d) * dot(w, d)).cmp(&0);
            for scale in [f32::from_bits(1 << 9), 1., f32::from_bits(247 << 23)] {
                let [a, b, c, e] = p.map(|q| {
                    q.map(|v| {
                        let value = v as f32 * scale;
                        if value == 0. {
                            0
                        } else {
                            value.to_bits()
                        }
                    })
                });
                assert_eq!(exact::wedge_sign((a, b), c, e), expected);
            }
        }
    }

    #[test]
    fn nonacute_fallback_has_a_four_wing_bound_only_when_unsupported() {
        let wings = [[1., 0., 0.], [0., 1., 0.], [-1., 0., 0.], [0., -1., 0.]];
        let mut faces: Vec<_> = wings.into_iter().map(|p| [A, B, p]).collect();
        faces.push([B, A, [2., 2., 0.]]);
        assert_eq!(probe(&faces, [1, 1]).unsupported_edges, 0);
        faces.push([A, B, [1., 1., 0.]]);
        assert_eq!(probe(&faces, [1, 1]).unsupported_edges, 1);
        assert_eq!(probe(&faces, [5, 1]).unsupported_edges, 0);
        // The early cardinality check also bounds large unsupported incidences.
        let edge = seg(A.map(f32::to_bits), B.map(f32::to_bits));
        assert!(!nonacute_side(edge, &vec![[1f32.to_bits(), 0, 0]; 1024]));
    }

    #[test]
    fn coincidence_counts_oriented_duplicates_and_reversed_twins_separately() {
        let face = [A, B, [1., 0., 0.]];
        let duplicate = probe(&[face, [face[1], face[2], face[0]]], [1, 0]);
        assert_eq!(duplicate.coincident_groups, 1);
        assert_eq!(duplicate.duplicate_faces, 1);
        assert_eq!(duplicate.reversed_twin_groups, 0);
        assert_eq!(duplicate.unsupported_coincident_groups, 1);
        let twin = probe(&[face, [face[0], face[2], face[1]]], [1, 0]);
        assert_eq!(twin.unsupported_edges, 0); // 1/1 passes direction counts.
        assert_eq!(twin.coincident_groups, 1);
        assert_eq!(twin.duplicate_faces, 0);
        assert_eq!(twin.reversed_twin_groups, 1);
        assert_eq!(twin.unsupported_coincident_groups, 1);
        // Both source orientations contribute to the same corner-set count.
        for source in [
            vec![face, [face[0], face[2], face[1]]],
            vec![face, [face[1], face[2], face[0]]],
        ] {
            assert_eq!(
                probe_with_leaves(&[face, face], [2, 0], &[source], 0..1)
                    .unsupported_coincident_groups,
                0
            );
        }
    }

    #[test]
    fn unrelated_descendant_twins_and_edge_totals_do_not_license_coincidence() {
        let face = [A, B, [1., 0., 0.]];
        let twin = [face[0], face[2], face[1]];
        let elsewhere = [[2., 0., 0.], [2., 0., 1.], [3., 0., 0.]];
        let unrelated = vec![elsewhere, [elsewhere[0], elsewhere[2], elsewhere[1]]];
        assert_eq!(
            probe_with_leaves(&[face, twin], [1, 1], &[unrelated], 0..1)
                .unsupported_coincident_groups,
            1
        );
        // Every target edge has two actual source faces (one each way), but
        // their distinct third corners leave no coincident source corner set.
        let mut edge_sources = Vec::new();
        for i in 0..3 {
            let (a, b) = (face[i], face[(i + 1) % 3]);
            edge_sources.push([a, b, [2., 2., (2 * i) as f32]]);
            edge_sources.push([b, a, [2., 2., (2 * i + 1) as f32]]);
        }
        let census = probe_with_leaves(&[face, twin], [1, 1], &[edge_sources], 0..1);
        assert_eq!(census.unsupported_edges, 0);
        assert_eq!(census.unsupported_coincident_groups, 1);
    }

    #[test]
    fn source_groups_sum_across_descendant_leaves_only() {
        let face = [A, B, [1., 0., 0.]];
        let twin = [face[0], face[2], face[1]];
        let leaves = [vec![face], vec![twin], vec![face]];
        // No leaf is coincident on its own: the group spans decoded leaves.
        assert_eq!(
            probe_with_leaves(&[face, twin], [1, 1], &leaves, 0..2).unsupported_coincident_groups,
            0
        );
        assert_eq!(
            probe_with_leaves(&[face, twin], [1, 1], &leaves, 0..1).unsupported_coincident_groups,
            1
        );
        // Two source copies license two, but never three coarse copies.
        assert_eq!(
            probe_with_leaves(&[face, twin, face], [2, 1], &leaves, 0..2)
                .unsupported_coincident_groups,
            1
        );
        assert_eq!(
            probe_with_leaves(&[face, twin, face], [2, 1], &leaves, 0..3)
                .unsupported_coincident_groups,
            0
        );
        // Source groups outside the coarse tile cannot help any corner.
        assert_eq!(
            probe_with_leaves(&[face, twin], [1, 1], &leaves, 1..2).unsupported_coincident_groups,
            1
        );
        assert_eq!(
            probe_with_leaves(&[face, twin], [1, 1], &leaves, 3..3).unsupported_coincident_groups,
            1
        );
    }

    #[test]
    fn every_coarse_corner_needs_one_source_group_of_sufficient_multiplicity() {
        let face = [A, B, [1., 0., 0.]];
        let twin = [face[0], face[2], face[1]];
        // Each corner can use a different coincident source group; the coarse
        // face itself need not occur in the source. Split each group over leaves.
        let groups = face.map(|p| [p, [2., 2., 0.], [2., 2., 1.]]);
        let leaves = [groups.to_vec(), groups.to_vec()];
        assert_eq!(
            probe_with_leaves(&[face, twin], [1, 1], &leaves, 0..2).unsupported_coincident_groups,
            0
        );
        for missing in 0..3 {
            let mut incomplete = leaves.clone();
            incomplete[1].remove(missing);
            assert_eq!(
                probe_with_leaves(&[face, twin], [1, 1], &incomplete, 0..2)
                    .unsupported_coincident_groups,
                1,
                "corner {missing} has only one matching source face"
            );
        }
        // Counts at a corner cannot be pooled across different corner sets.
        // These two distinct singleton groups touch each corner but license none.
        let singletons: Vec<_> = face
            .into_iter()
            .flat_map(|p| {
                [
                    [p, [2., 2., 0.], [2., 2., 1.]],
                    [p, [3., 3., 0.], [3., 3., 1.]],
                ]
            })
            .collect();
        assert_eq!(
            probe_with_leaves(&[face, twin], [1, 1], &[singletons], 0..1)
                .unsupported_coincident_groups,
            1
        );
    }
}
