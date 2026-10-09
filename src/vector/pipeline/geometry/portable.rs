//! Portable polygon operations. CDT uses exact input coordinates and no snapping.
//! Repair uses geo's even-odd MakeValid, then checks every source boundary in
//! both directions. Unlike GEOS, overlapping/retraced linework is conservatively
//! refused: discarding a collapsed component must require the outline policy.
use super::{data, outline, FeatureFailure, Point};
use geo::{
    algorithm::kernels::{Kernel, Orientation, RobustKernel},
    coordinate_position::CoordPos,
    dimensions::Dimensions,
    line_intersection::{line_intersection, LineIntersection},
    Area, Coord, Line, LineString, MakeValid, Polygon, PreparedGeometry, Relate, Validation,
};
use rstar::{
    primitives::{GeomWithData, Rectangle},
    RTree, RTreeObject, AABB,
};
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

type XY = [f64; 2];
type XYKey = [u64; 2];
type Cdt = ConstrainedDelaunayTriangulation<Point2<f64>>;

fn key(p: Coord<f64>) -> XYKey {
    [p.x, p.y].map(|v| if v == 0. { 0 } else { v.to_bits() })
}

// A dependency assertion indicates an unproven kernel invariant, not a
// semantic feature rejection. Catch it only to abort through the job boundary.
fn guarded<T>(operation: impl FnOnce() -> Result<T, FeatureFailure>) -> Result<T, FeatureFailure> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|payload| {
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("non-string panic payload");
        FeatureFailure::Fatal(crate::Error::Job(crate::JobFailure {
            error: crate::JobError::new(
                crate::JobErrorKind::InvalidState,
                format!("portable polygon kernel panicked: {message}"),
            ),
            secondary: Vec::new(),
            retained_paths: Vec::new(),
            recovery: None,
        }))
    })?
}

#[derive(Clone)]
pub(in super::super) struct GeometryHandle {
    polygons: Vec<Polygon<f64>>,
    valid: bool,
    collapsed: bool,
}

impl GeometryHandle {
    pub(in super::super) fn polygon(rings: &[Vec<Point>]) -> Result<Self, FeatureFailure> {
        if rings.is_empty() || rings.iter().any(|ring| ring.is_empty()) {
            return Err(data("polygon rings need three distinct vertices"));
        }
        if rings
            .iter()
            .flatten()
            .any(|p| !p.iter().all(|v| v.is_finite()))
        {
            return Err(data("polygon coordinates must be finite XYZ"));
        }
        guarded(|| {
            let mut rings = rings.iter().map(|ring| {
                let mut coords: Vec<Coord<f64>> = Vec::new();
                for p in ring {
                    let coord = Coord { x: p[0], y: p[1] };
                    if coords.last() != Some(&coord) {
                        coords.push(coord);
                    }
                }
                if coords.first() != coords.last() {
                    coords.push(coords[0]);
                }
                LineString::new(coords)
            });
            let polygon = Polygon::new(rings.next().unwrap(), rings.collect());
            let (valid, collapsed) = topology(&polygon)?;
            Ok(Self {
                polygons: vec![polygon],
                valid,
                collapsed,
            })
        })
    }

    pub(super) fn valid(&self) -> Result<bool, FeatureFailure> {
        Ok(self.valid)
    }

    pub(super) fn repair(&self) -> Result<Self, FeatureFailure> {
        guarded(|| {
            if self.collapsed {
                return Err(outline("portable repair would discard collapsed or overlapping source edges; source needs review"));
            }
            let mut polygons = Vec::new();
            for original in &self.polygons {
                let repaired = original
                    .make_valid()
                    .map_err(|e| data(format!("portable polygon repair failed: {e}")))?;
                if repaired.0.is_empty() {
                    return Err(outline("polygon has no filled area after portable repair"));
                }
                for part in &repaired.0 {
                    let (valid, collapsed) = topology(part)?;
                    if !valid || collapsed || part.unsigned_area() <= 0. {
                        return Err(outline("portable repair produced degenerate or ambiguous polygon topology; source needs review"));
                    }
                }
                // MakeValid has a 0.1 mm internal snap radius. This is not a
                // licence to move source vertices: reject repairs whose result
                // changes edges, loses linework, or invents an off-edge vertex.
                let before = edges(original);
                let after: Vec<_> = repaired.0.iter().flat_map(edges).collect();
                let retained: BTreeSet<_> = after
                    .iter()
                    .flat_map(|e| [key(e.line.start), key(e.line.end)])
                    .collect();
                if before
                    .iter()
                    .any(|e| !retained.contains(&key(e.line.start)))
                {
                    return Err(outline("portable repair would snap or discard source vertices; source needs review"));
                }
                preserve_boundaries(&before, &after)?;
                polygons.extend(repaired.0);
            }
            canonicalize(&mut polygons);
            Ok(Self {
                polygons,
                valid: true,
                collapsed: false,
            })
        })
    }

    pub(super) fn area(&self) -> Result<f64, FeatureFailure> {
        Ok(self.polygons.iter().map(Area::unsigned_area).sum())
    }

    pub(super) fn parts(&self) -> Result<Vec<Self>, FeatureFailure> {
        if self.collapsed {
            return Err(outline(
                "polygon contains collapsed source geometry; source needs review",
            ));
        }
        Ok(self
            .polygons
            .iter()
            .cloned()
            .map(|polygon| Self {
                polygons: vec![polygon],
                valid: self.valid,
                collapsed: false,
            })
            .collect())
    }

    pub(super) fn rings(&self) -> Result<Vec<Vec<XY>>, FeatureFailure> {
        Ok(self
            .polygons
            .first()
            .map(|polygon| {
                std::iter::once(polygon.exterior())
                    .chain(polygon.interiors())
                    .map(|ring| {
                        ring.0[..ring.0.len().saturating_sub(1)]
                            .iter()
                            .map(|c| [c.x, c.y])
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub(in super::super) fn triangulate(&self) -> Result<Self, FeatureFailure> {
        guarded(|| {
            if !self.valid || self.collapsed {
                return Err(data(
                    "portable triangulation requires valid filled polygon topology",
                ));
            }
            let mut triangles = Vec::new();
            for polygon in &self.polygons {
                triangles.extend(triangulate(polygon)?);
            }
            Ok(Self {
                polygons: triangles,
                valid: true,
                collapsed: false,
            })
        })
    }
}

fn ring_cmp(a: &LineString<f64>, b: &LineString<f64>) -> std::cmp::Ordering {
    for (a, b) in a.0.iter().zip(&b.0) {
        let order = a.x.total_cmp(&b.x).then_with(|| a.y.total_cmp(&b.y));
        if !order.is_eq() {
            return order;
        }
    }
    a.0.len().cmp(&b.0.len())
}

// MakeValid traces HashSet-held boundary edges. Canonicalize only repaired
// geometry so process-local hash seeds cannot change leaf bytes or reuse keys.
fn canonicalize(polygons: &mut [Polygon<f64>]) {
    fn ring(mut ring: LineString<f64>, exterior: bool) -> LineString<f64> {
        let ccw = Polygon::new(ring.clone(), vec![]).signed_area() > 0.;
        ring.0.pop();
        if ccw != exterior {
            ring.0.reverse();
        }
        let start = (0..ring.0.len())
            .min_by(|a, b| {
                ring.0[*a]
                    .x
                    .total_cmp(&ring.0[*b].x)
                    .then_with(|| ring.0[*a].y.total_cmp(&ring.0[*b].y))
            })
            .unwrap();
        ring.0.rotate_left(start);
        ring.0.push(ring.0[0]);
        ring
    }
    for polygon in polygons.iter_mut() {
        let (exterior, interiors) = polygon.clone().into_inner();
        let mut interiors: Vec<_> = interiors.into_iter().map(|r| ring(r, false)).collect();
        interiors.sort_by(ring_cmp);
        *polygon = Polygon::new(ring(exterior, true), interiors);
    }
    polygons.sort_by(|a, b| ring_cmp(a.exterior(), b.exterior()));
}

#[derive(Clone)]
struct Edge {
    line: Line<f64>,
    ring: usize,
    offset: usize,
    ring_len: usize,
    index: usize,
}
impl RTreeObject for Edge {
    type Envelope = AABB<XY>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            [
                self.line.start.x.min(self.line.end.x),
                self.line.start.y.min(self.line.end.y),
            ],
            [
                self.line.start.x.max(self.line.end.x),
                self.line.start.y.max(self.line.end.y),
            ],
        )
    }
}

fn edges(polygon: &Polygon<f64>) -> Vec<Edge> {
    let mut result = Vec::new();
    for (ring, path) in std::iter::once(polygon.exterior())
        .chain(polygon.interiors())
        .enumerate()
    {
        for (offset, line) in path.lines().enumerate() {
            result.push(Edge {
                line,
                ring,
                offset,
                ring_len: path.0.len() - 1,
                index: result.len(),
            });
        }
    }
    result
}

fn root(parents: &mut [usize], mut i: usize) -> usize {
    while parents[i] != i {
        parents[i] = parents[parents[i]];
        i = parents[i];
    }
    i
}

/// Geo's Validation currently omits the connected-interior rule and its ring
/// intersection check is quadratic. Use indexed exact predicates for large
/// rings and a bipartite ring/contact graph to detect disconnected interiors.
fn topology(polygon: &Polygon<f64>) -> Result<(bool, bool), FeatureFailure> {
    let rings: Vec<_> = std::iter::once(polygon.exterior())
        .chain(polygon.interiors())
        .collect();
    let mut valid = true;
    for ring in &rings {
        if ring.0.len() < 4 {
            return Ok((false, true));
        }
        let a = ring.0[0];
        let Some(b) = ring.0.iter().copied().find(|b| *b != a) else {
            return Ok((false, true));
        };
        if ring
            .0
            .iter()
            .all(|c| RobustKernel::orient2d(a, b, *c) == Orientation::Collinear)
        {
            return Ok((false, true));
        }
        let mut seen = BTreeSet::new();
        valid &= ring.0[..ring.0.len() - 1]
            .iter()
            .all(|c| seen.insert(key(*c)));
    }
    let lines = edges(polygon);
    let tree = RTree::bulk_load(lines.clone());
    let mut parents: Vec<_> = (0..rings.len()).collect();
    let mut contacts = BTreeMap::new();
    let mut contact_edges = BTreeSet::new();
    let mut pairs = 0usize;
    for a in &lines {
        for b in tree
            .locate_in_envelope_intersecting(&a.envelope())
            .filter(|b| b.index > a.index)
        {
            pairs += 1;
            if pairs > 10_000_000 {
                return Err(data(
                    "portable polygon topology exceeds intersection complexity limit",
                ));
            }
            match line_intersection(a.line, b.line) {
                Some(LineIntersection::Collinear { intersection })
                    if intersection.start != intersection.end =>
                {
                    return Ok((false, true));
                }
                Some(LineIntersection::SinglePoint {
                    intersection,
                    is_proper,
                }) => {
                    if a.ring == b.ring {
                        let adjacent = a.offset.abs_diff(b.offset) == 1
                            || a.offset.abs_diff(b.offset) == a.ring_len - 1;
                        valid &= adjacent && !is_proper;
                    } else if is_proper {
                        valid = false;
                    } else {
                        let node = *contacts.entry(key(intersection)).or_insert_with(|| {
                            let node = parents.len();
                            parents.push(node);
                            node
                        });
                        for ring in [a.ring, b.ring] {
                            if contact_edges.insert((ring, node)) {
                                let x = root(&mut parents, ring);
                                let y = root(&mut parents, node);
                                if x == y {
                                    valid = false;
                                } else {
                                    parents[x] = y;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if !valid {
        return Ok((false, false));
    }
    let area = polygon.unsigned_area();
    if !area.is_finite() {
        return Err(data("nonfinite portable polygon area"));
    }
    if area <= 0. {
        return Ok((false, false));
    }
    // Cross-check geo's full validation on small polygons. For larger rings,
    // the indexed simple-ring checks above and prepared relationships below
    // implement the same validity rules without an all-pairs edge scan.
    if lines.len() <= 256 {
        return Ok((polygon.is_valid(), false));
    }
    let exterior = Polygon::new(polygon.exterior().clone(), vec![]);
    let exterior = PreparedGeometry::from(&exterior);
    let hole_bounds: Vec<_> = polygon
        .interiors()
        .iter()
        .enumerate()
        .map(|(i, ring)| {
            let mut lo = [f64::INFINITY; 2];
            let mut hi = [f64::NEG_INFINITY; 2];
            for c in &ring.0 {
                lo[0] = lo[0].min(c.x);
                lo[1] = lo[1].min(c.y);
                hi[0] = hi[0].max(c.x);
                hi[1] = hi[1].max(c.y);
            }
            GeomWithData::new(Rectangle::from_corners(lo, hi), i)
        })
        .collect();
    let holes = RTree::bulk_load(hole_bounds.clone());
    for (i, ring) in polygon.interiors().iter().enumerate() {
        let hole = Polygon::new(ring.clone(), vec![]);
        if !exterior.relate(&hole).is_contains() {
            return Ok((false, false));
        }
        let hole = PreparedGeometry::from(&hole);
        for candidate in holes
            .locate_in_envelope_intersecting(&hole_bounds[i].envelope())
            .filter(|c| c.data > i)
        {
            pairs += 1;
            if pairs > 10_000_000 {
                return Err(data(
                    "portable polygon topology exceeds intersection complexity limit",
                ));
            }
            let other = Polygon::new(polygon.interiors()[candidate.data].clone(), vec![]);
            if hole.relate(&other).get(CoordPos::Inside, CoordPos::Inside)
                == Dimensions::TwoDimensional
            {
                return Ok((false, false));
            }
        }
    }
    Ok((true, false))
}

fn preserve_boundaries(before: &[Edge], after: &[Edge]) -> Result<(), FeatureFailure> {
    // Boundary coverage is checked in both directions: this detects a repaired
    // polygon plus a silently discarded spike/hole, as well as a new chord.
    // The threshold is one hundred times tighter than the shared XYZ edge guard.
    const EPS: f64 = 1e-8;
    for (source, target) in [(before, after), (after, before)] {
        let tree = RTree::bulk_load(target.to_vec());
        for edge in source {
            let [start, end] = [edge.line.start, edge.line.end];
            let delta = [end.x - start.x, end.y - start.y];
            let length = delta[0].hypot(delta[1]);
            if !length.is_finite() || length == 0. {
                return Err(outline("portable repair encountered collapsed source edge"));
            }
            let unit = [delta[0] / length, delta[1] / length];
            let bounds = edge.envelope();
            let lo = bounds.lower();
            let hi = bounds.upper();
            let query = AABB::from_corners([lo[0] - EPS, lo[1] - EPS], [hi[0] + EPS, hi[1] + EPS]);
            let mut intervals = Vec::new();
            for candidate in tree.locate_in_envelope_intersecting(&query) {
                let coords = [candidate.line.start, candidate.line.end];
                let mut distances = [0.; 2];
                let mut along = true;
                for (i, c) in coords.into_iter().enumerate() {
                    let d = [c.x - start.x, c.y - start.y];
                    along &= (d[0] * unit[1] - d[1] * unit[0]).abs() <= EPS;
                    distances[i] = d[0] * unit[0] + d[1] * unit[1];
                }
                if along {
                    let a = distances[0].min(distances[1]).max(0.);
                    let b = distances[0].max(distances[1]).min(length);
                    if b > a {
                        intervals.push((a, b));
                    }
                }
            }
            intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
            let mut covered = 0.;
            let mut any = false;
            for (a, b) in intervals {
                if a > covered + EPS {
                    break;
                }
                covered = covered.max(b);
                any = true;
            }
            if !any || covered < length - EPS {
                return Err(outline(
                    "portable repair would move or discard source boundary; source needs review",
                ));
            }
        }
    }
    Ok(())
}

fn triangulate(polygon: &Polygon<f64>) -> Result<Vec<Polygon<f64>>, FeatureFailure> {
    let mut cdt = Cdt::new();
    let mut vertices = BTreeMap::new();
    let lines = edges(polygon);
    // Insert each exact coordinate before adding constraints, so even collinear
    // source boundary vertices split the corresponding constraint edge.
    for edge in &lines {
        for p in [edge.line.start, edge.line.end] {
            if let std::collections::btree_map::Entry::Vacant(entry) = vertices.entry(key(p)) {
                let handle = cdt.insert(Point2::new(p.x, p.y)).map_err(|e| {
                    data(format!(
                        "portable triangulation coordinate unsupported: {e}"
                    ))
                })?;
                entry.insert(handle);
            }
        }
    }
    for edge in &lines {
        let a = vertices[&key(edge.line.start)];
        let b = vertices[&key(edge.line.end)];
        if !cdt.can_add_constraint(a, b) {
            return Err(data(
                "portable triangulation cannot preserve polygon constraint edges",
            ));
        }
        cdt.add_constraint(a, b);
    }
    // Flood from the outer face. Crossing a ring constraint flips even-odd
    // membership; this excludes holes without per-triangle polygon scans.
    let mut flags = HashMap::new();
    let mut queue = VecDeque::new();
    for edge in cdt.directed_edges().filter(|e| e.face().is_outer()) {
        if let Some(face) = edge.rev().face().as_inner() {
            let handle = face.fix();
            let inside = edge.is_constraint_edge();
            if let Some(existing) = flags.get(&handle) {
                if *existing != inside {
                    return Err(data("portable triangulation has inconsistent hull parity"));
                }
            } else {
                flags.insert(handle, inside);
                queue.push_back(handle);
            }
        }
    }
    while let Some(handle) = queue.pop_front() {
        let inside = flags[&handle];
        for edge in cdt.face(handle).adjacent_edges() {
            if let Some(face) = edge.rev().face().as_inner() {
                let next = face.fix();
                let value = inside ^ edge.is_constraint_edge();
                if let Some(existing) = flags.get(&next) {
                    if *existing != value {
                        return Err(data(
                            "portable triangulation has inconsistent boundary parity",
                        ));
                    }
                } else {
                    flags.insert(next, value);
                    queue.push_back(next);
                }
            }
        }
    }
    let mut triangles = Vec::new();
    let mut used = BTreeSet::new();
    for face in cdt
        .inner_faces()
        .filter(|f| flags.get(&f.fix()) == Some(&true))
    {
        let coords: Vec<_> = face
            .positions()
            .into_iter()
            .map(|p| Coord { x: p.x, y: p.y })
            .collect();
        for p in &coords {
            used.insert(key(*p));
        }
        let triangle = Polygon::new(LineString::new(coords), vec![]);
        let area = triangle.unsigned_area();
        if !area.is_finite() || area <= 0. {
            return Err(data("portable triangulation produced degenerate triangle"));
        }
        triangles.push(triangle);
    }
    if used.len() != vertices.len() || triangles.is_empty() {
        return Err(data(
            "portable triangulation lost source boundary vertices or filled area",
        ));
    }
    let actual: f64 = triangles.iter().map(Area::unsigned_area).sum();
    let expected = polygon.unsigned_area();
    if !actual.is_finite() || (actual - expected).abs() > expected * 1e-9 {
        return Err(data(
            "portable triangulation does not preserve polygon area",
        ));
    }
    Ok(triangles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_panic_is_fatal_and_never_outline_eligible() {
        for owned in [false, true] {
            let result: Result<(), FeatureFailure> = guarded(|| {
                if owned {
                    std::panic::panic_any(String::from("fixture kernel invariant"));
                }
                panic!("fixture kernel invariant");
            });
            let failure = result.unwrap_err();
            assert!(!failure.is_outline());
            let FeatureFailure::Fatal(crate::Error::Job(failure)) = failure else {
                panic!("kernel panic must abort rather than reject a feature")
            };
            assert_eq!(failure.error.kind(), crate::JobErrorKind::InvalidState);
            assert!(failure
                .error
                .to_string()
                .contains("fixture kernel invariant"));
        }
    }

    fn polygon(rings: &[&[XY]]) -> GeometryHandle {
        GeometryHandle::polygon(
            &rings
                .iter()
                .map(|r| r.iter().map(|p| [p[0], p[1], 0.]).collect())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn assert_area(shape: &GeometryHandle, expected: f64) {
        assert!(shape.valid().unwrap());
        assert!((shape.area().unwrap() - expected).abs() < 1e-10);
        let triangles = shape.triangulate().unwrap();
        assert!((triangles.area().unwrap() - expected).abs() < 1e-10);
        assert!(triangles
            .parts()
            .unwrap()
            .iter()
            .all(|t| t.rings().unwrap()[0].len() == 3));
    }

    #[test]
    fn concave_polygon_hole_and_collinear_boundary_vertices_are_retained() {
        let outer = [
            [0., 0.],
            [5., 0.],
            [10., 0.],
            [10., 10.],
            [6., 10.],
            [6., 4.],
            [4., 4.],
            [4., 10.],
            [0., 10.],
        ];
        let hole = [[1., 1.], [1., 3.], [3., 3.], [3., 1.]];
        let shape = polygon(&[&outer, &hole]);
        assert_area(&shape, 84.);
        let triangles = shape.triangulate().unwrap();
        let vertices: BTreeSet<_> = triangles
            .polygons
            .iter()
            .flat_map(|p| p.exterior().0.iter().map(|c| key(*c)))
            .collect();
        for p in outer.into_iter().chain(hole) {
            assert!(vertices.contains(&key(Coord { x: p[0], y: p[1] })));
        }
        assert!(triangles.polygons.iter().all(|t| {
            let x = t.exterior().0.iter().take(3).map(|c| c.x).sum::<f64>() / 3.;
            let y = t.exterior().0.iter().take(3).map(|c| c.y).sum::<f64>() / 3.;
            !(x > 1. && x < 3. && y > 1. && y < 3.)
        }));
    }

    #[test]
    fn bowtie_repair_preserves_source_edges_and_adds_only_the_exact_crossing() {
        let shape = polygon(&[&[[0., 0.], [2., 2.], [2., 0.], [0., 2.]]]);
        assert!(!shape.valid().unwrap());
        let repaired = shape.repair().unwrap();
        assert_eq!(repaired.parts().unwrap().len(), 2);
        assert_area(&repaired, 2.);
        assert!(repaired
            .polygons
            .iter()
            .flat_map(|p| &p.exterior().0)
            .all(|c| { [[0., 0.], [2., 2.], [2., 0.], [0., 2.], [1., 1.]].contains(&[c.x, c.y]) }));
    }

    #[test]
    fn duplicate_adjacent_vertices_and_hole_point_tangency_do_not_break_cdt() {
        let shape = polygon(&[
            &[[0., 0.], [0., 0.], [10., 0.], [10., 10.], [0., 10.]],
            &[[0., 5.], [2., 3.], [2., 7.]],
        ]);
        assert_area(&shape, 96.);
    }

    #[test]
    fn repair_cannot_discard_collapsed_holes_spikes_or_canceling_rings() {
        let outer = [[0., 0.], [10., 0.], [10., 10.], [0., 10.]];
        for rings in [
            vec![outer.to_vec(), vec![[2., 2.], [3., 2.], [4., 2.]]],
            vec![outer.to_vec(), outer.to_vec()],
            vec![vec![
                [0., 0.],
                [10., 0.],
                [15., 5.],
                [10., 0.],
                [10., 10.],
                [0., 10.],
            ]],
            vec![vec![[0., 0.], [1., 0.], [2., 0.], [1., 0.]]],
        ] {
            let shape = polygon(&rings.iter().map(Vec::as_slice).collect::<Vec<_>>());
            assert!(!shape.valid().unwrap());
            let error = shape.repair().err().unwrap();
            assert!(super::super::is_outline(&error), "{error}");
        }
    }

    #[test]
    fn nested_and_outside_holes_repair_to_even_odd_filled_parts() {
        let outer = [[0., 0.], [10., 0.], [10., 10.], [0., 10.]];
        let nested = polygon(&[
            &outer,
            &[[2., 2.], [8., 2.], [8., 8.], [2., 8.]],
            &[[3., 3.], [4., 3.], [4., 4.], [3., 4.]],
        ]);
        assert!(!nested.valid().unwrap());
        assert_area(&nested.repair().unwrap(), 65.);
        let outside = polygon(&[&outer, &[[12., 0.], [14., 0.], [14., 2.], [12., 2.]]]);
        assert!(!outside.valid().unwrap());
        assert_area(&outside.repair().unwrap(), 104.);
    }

    #[test]
    fn hole_touching_shell_twice_has_disconnected_interior_and_requires_repair() {
        let shape = polygon(&[
            &[[0., 0.], [10., 0.], [10., 10.], [0., 10.]],
            &[[0., 5.], [5., 3.], [10., 5.], [5., 7.]],
        ]);
        assert!(!shape.valid().unwrap());
        let repaired = shape.repair().unwrap();
        assert_eq!(repaired.parts().unwrap().len(), 2);
        assert_area(&repaired, 80.);
    }

    #[test]
    fn finite_precision_failures_are_errors_and_do_not_invent_or_drop_geometry() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                GeometryHandle::polygon(&[vec![[0., 0., 0.], [value, 1., 0.], [1., 0., 0.]]])
                    .is_err()
            );
        }
        // Valid submillimetre polygons need no repair and retain exact vertices.
        let tiny = polygon(&[&[[0., 0.], [1e-6, 0.], [1e-6, 1e-6], [0., 1e-6]]]);
        assert!(tiny.valid().unwrap());
        assert!((tiny.triangulate().unwrap().area().unwrap() - 1e-12).abs() < 1e-24);
        // geo MakeValid's internal endpoint snapping cannot silently turn this
        // hole into nothing while retaining the surrounding square.
        let hole = polygon(&[
            &[[0., 0.], [10., 0.], [10., 10.], [0., 10.]],
            &[
                [5., 5.],
                [5.000001, 5.000001],
                [5.000001, 5.],
                [5., 5.000001],
            ],
        ]);
        assert!(!hole.valid().unwrap());
        assert!(super::super::is_outline(&hole.repair().err().unwrap()));
    }

    #[test]
    fn large_valid_ring_uses_indexed_topology_and_keeps_all_constraint_vertices() {
        let ring: Vec<_> = (0..1024)
            .map(|i| {
                let phi = std::f64::consts::TAU * i as f64 / 1024.;
                [20. * phi.cos(), 20. * phi.sin()]
            })
            .collect();
        let shape = polygon(&[&ring]);
        assert!(shape.valid().unwrap());
        let triangles = shape.triangulate().unwrap();
        assert_eq!(triangles.parts().unwrap().len(), 1022);
        assert!((triangles.area().unwrap() - shape.area().unwrap()).abs() < 1e-8);
    }

    #[test]
    fn many_disjoint_holes_use_indexed_containment_checks() {
        let mut rings = vec![vec![[-1., -1.], [151., -1.], [151., 61.], [-1., 61.]]];
        for x in 0..50 {
            for y in 0..20 {
                let (x, y) = (3. * x as f64 + 1., 3. * y as f64 + 1.);
                rings.push(vec![[x, y], [x + 1., y], [x + 1., y + 1.], [x, y + 1.]]);
            }
        }
        let shape = polygon(&rings.iter().map(Vec::as_slice).collect::<Vec<_>>());
        assert!(shape.valid().unwrap());
        assert_eq!(shape.area().unwrap(), 8424.);
        assert!((shape.triangulate().unwrap().area().unwrap() - 8424.).abs() < 1e-8);
    }

    #[test]
    fn repaired_component_ring_and_triangle_order_is_independent_of_hash_seeds() {
        let corners = [[0., 0.], [2., 2.], [2., 0.], [0., 2.], [0., 0.]];
        let mut ring = Vec::new();
        for pair in corners.windows(2) {
            for i in 0..25 {
                let t = i as f64 / 25.;
                ring.push([
                    pair[0][0] + t * (pair[1][0] - pair[0][0]),
                    pair[0][1] + t * (pair[1][1] - pair[0][1]),
                ]);
            }
        }
        let source = polygon(&[&ring]);
        let expected = source.repair().unwrap();
        let triangles = expected.triangulate().unwrap();
        for _ in 0..32 {
            let actual = source.repair().unwrap();
            assert_eq!(actual.polygons, expected.polygons);
            assert_eq!(actual.triangulate().unwrap().polygons, triangles.polygons);
        }
    }
}
