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
        let actual: HashSet<Edge> = t
            .faces
            .iter()
            .flat_map(|f| (0..3).map(move |i| seg(f[i], f[(i + 1) % 3])))
            .collect();
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
                if !actual.contains(&seg(a, b)) {
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
                for i in first..end {
                    let e = seg(chain[i], chain[i + 1]);
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
