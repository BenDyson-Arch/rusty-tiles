//! Independent D11 census. Producer subarc IDs are only lookup hints. Derive exact
//! incidence, live chains and source parameters again from independently decoded leaves;
//! check actual delivered edges, every source breakpoint, every owner and every depth mix.
use super::*;
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
fn leaf_has_coincident_faces(faces: &[[Point; 3]]) -> bool {
    face_directions(faces).values().any(|n| n[0] + n[1] > 1)
}
/// Same-direction wings may be perpendicular or opposite (book pages or a collapsed
/// plate), but never acute. A collinear wing has no side and cannot justify excess.
/// Index third corners once per tile rather than rescanning all faces for each edge.
fn two_sided(edge: Edge, incident: &[Vec<Point>; 2]) -> bool {
    let (a, b) = (q(edge.0), q(edge.1));
    let d: Q = std::array::from_fn(|i| b[i] - a[i]);
    let dd: f64 = d.iter().map(|v| v * v).sum();
    for side in incident {
        let mut wings: Vec<Q> = Vec::with_capacity(side.len());
        for &point in side {
            let p = q(point);
            let w: Q = std::array::from_fn(|i| p[i] - a[i]);
            let t = w.iter().zip(d).map(|(w, d)| w * d).sum::<f64>() / dd;
            let w: Q = std::array::from_fn(|i| w[i] - t * d[i]);
            let norm = w.iter().map(|v| v * v).sum::<f64>().sqrt();
            if !norm.is_finite() || norm <= 0. {
                return false;
            }
            let unit = w.map(|v| v / norm);
            if wings
                .iter()
                .any(|other| unit.iter().zip(other).map(|(a, b)| a * b).sum::<f64>() > 1e-6)
            {
                return false;
            }
            wings.push(unit);
        }
    }
    true
}
fn fold_census(
    faces: &[[Point; 3]],
    actual: &HashMap<Edge, [u32; 2]>,
    support: &HashMap<Edge, [u32; 2]>,
    total_support: &HashMap<Edge, u32>,
    descendant_coincident: bool,
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
    let excess = |n, other| n >= 2 && n > other;
    for (&edge, &[forward, reverse]) in actual {
        if !excess(forward, reverse) && !excess(reverse, forward) {
            continue;
        }
        let allowed = support.get(&edge).copied().unwrap_or([0, 0]);
        let witnessed = (!excess(forward, reverse) || allowed[0] >= forward)
            && (!excess(reverse, forward) || allowed[1] >= reverse);
        if !witnessed && !(forward > 0 && reverse > 0 && two_sided(edge, &incident[&edge])) {
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
        // Match native licensing: any coincident group within a descendant leaf,
        // or enough source incidence on EVERY used edge/subedge. Twins split across
        // different leaves do not supply the leaf-local coincidence license.
        if !descendant_coincident
            && (0..3).any(|i| {
                let edge = seg(face[i], face[(i + 1) % 3]);
                let used = actual[&edge];
                total_support.get(&edge).copied().unwrap_or(0) < used[0] + used[1]
            })
        {
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
    let mut source_coincident = BTreeSet::new();
    for t in tiles.iter().filter(|t| t.leaf) {
        if leaf_has_coincident_faces(&t.faces) {
            source_coincident.insert(t.begin);
        }
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
        // Minima of directional and total source counts are distinct: summing two
        // independent directional minima could understate the per-subedge total.
        let mut chord_total_support = HashMap::<Edge, u32>::new();
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
                let mut total_support = u32::MAX;
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
                    total_support = total_support.min(allowed[0] + allowed[1]);
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
                chord_total_support.insert(seg(a, b), total_support);
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
                let source = chord_support.entry(edge).or_insert_with(|| {
                    source_directions.get(&edge).map_or([0, 0], |owners| {
                        owners
                            .range(t.begin..t.end)
                            .fold([0, 0], |a, (_, b)| [a[0] + b[0], a[1] + b[1]])
                    })
                });
                chord_total_support
                    .entry(edge)
                    .or_insert(source[0] + source[1]);
            }
            let folds = fold_census(
                &t.faces,
                &actual,
                &chord_support,
                &chord_total_support,
                source_coincident.range(t.begin..t.end).next().is_some(),
            );
            c.unsupported_same_direction_edges += folds.unsupported_edges;
            c.coincident_coarse_face_groups += folds.coincident_groups;
            c.duplicate_coarse_faces += folds.duplicate_faces;
            c.reversed_twin_face_groups += folds.reversed_twin_groups;
            c.unsupported_coincident_face_groups += folds.unsupported_coincident_groups;
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
    fn probe(faces: &[[[f32; 3]; 3]], allowed: [u32; 2], total: u32, licensed: bool) -> FoldCensus {
        let faces: Vec<_> = faces
            .iter()
            .map(|f| f.map(|p| p.map(f32::to_bits)))
            .collect();
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
        let mut totals: HashMap<_, _> = actual.iter().map(|(e, n)| (*e, n[0] + n[1])).collect();
        totals.insert(edge, total);
        fold_census(&faces, &actual, &support, &totals, licensed)
    }

    #[test]
    fn directed_excess_checks_three_one_way_and_two_against_one() {
        let faces = [
            [A, B, [1., 0., 0.]],
            [A, B, [1., 1., 0.]],
            [A, B, [2., 1., 0.]],
        ];
        assert_eq!(probe(&faces, [1, 0], 1, false).unsupported_edges, 1);
        let mixed = [faces[0], faces[1], [B, A, [-1., 0., 0.]]];
        assert_eq!(probe(&mixed, [1, 1], 2, false).unsupported_edges, 1);
        // Proper nonmanifold source multiplicity admits even a one-way spine.
        assert_eq!(probe(&faces, [3, 0], 3, false).unsupported_edges, 0);
        assert_eq!(probe(&faces, [2, 0], 2, false).unsupported_edges, 1);
    }

    #[test]
    fn unwitnessed_balanced_sheets_need_nonacute_wings_in_each_direction() {
        let forward = [A, B, [1., 0., 0.]];
        let reverse = [B, A, [-1., 0., 0.]];
        for third in [[0., 1., 0.], [-1., 0., 0.]] {
            let faces = [forward, [A, B, third], reverse];
            assert_eq!(probe(&faces, [0, 0], 0, false).unsupported_edges, 0);
        }
        for third in [[1., 1., 0.], [0., 0., 0.5]] {
            let faces = [forward, [A, B, third], reverse];
            assert_eq!(probe(&faces, [0, 0], 0, false).unsupported_edges, 1);
        }
        // Perpendicular pages alone cannot license a one-way unsupported fold.
        assert_eq!(
            probe(&[forward, [A, B, [0., 1., 0.]]], [0, 0], 0, false).unsupported_edges,
            1
        );
    }

    #[test]
    fn coincidence_counts_oriented_duplicates_and_reversed_twins_separately() {
        let face = [A, B, [1., 0., 0.]];
        let duplicate = probe(&[face, [face[1], face[2], face[0]]], [1, 0], 1, false);
        assert_eq!(duplicate.coincident_groups, 1);
        assert_eq!(duplicate.duplicate_faces, 1);
        assert_eq!(duplicate.reversed_twin_groups, 0);
        assert_eq!(duplicate.unsupported_coincident_groups, 1);
        let twin = probe(&[face, [face[0], face[2], face[1]]], [1, 0], 1, false);
        assert_eq!(twin.unsupported_edges, 0); // 1/1 passes direction counts.
        assert_eq!(twin.coincident_groups, 1);
        assert_eq!(twin.duplicate_faces, 0);
        assert_eq!(twin.reversed_twin_groups, 1);
        assert_eq!(twin.unsupported_coincident_groups, 1);
        // Any leaf-local descendant coincidence licenses the twin, as in native
        // D11. Otherwise every source edge/subedge must carry the full total.
        assert_eq!(
            probe(&[face, [face[0], face[2], face[1]]], [1, 0], 1, true)
                .unsupported_coincident_groups,
            0
        );
        assert_eq!(
            probe(&[face, [face[0], face[2], face[1]]], [1, 1], 2, false)
                .unsupported_coincident_groups,
            0
        );
    }

    #[test]
    fn coincident_source_license_is_leaf_local() {
        let face = [A, B, [1., 0., 0.]].map(|p| p.map(f32::to_bits));
        let reverse = [face[0], face[2], face[1]];
        assert!(!leaf_has_coincident_faces(&[face]));
        assert!(!leaf_has_coincident_faces(&[reverse]));
        assert!(leaf_has_coincident_faces(&[face, reverse]));
        assert!(leaf_has_coincident_faces(&[
            face,
            [face[1], face[2], face[0]]
        ]));
    }

    #[test]
    fn total_arc_support_is_not_the_sum_of_directional_minima() {
        let face = [A, B, [1., 0., 0.]];
        // Two source subedges with counts 2/0 and 0/2 have directional minima
        // 0/0, yet both supply total 2. A reversed twin's total guard uses that
        // separate minimum, while the direction guard has no excess at 1/1.
        let twin = probe(&[face, [face[0], face[2], face[1]]], [0, 0], 2, false);
        assert_eq!(twin.unsupported_edges, 0);
        assert_eq!(twin.unsupported_coincident_groups, 0);
        let missing = probe(&[face, [face[0], face[2], face[1]]], [0, 0], 1, false);
        assert_eq!(missing.unsupported_coincident_groups, 1);
    }
}
