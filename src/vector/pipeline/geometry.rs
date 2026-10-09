//! Shared best-fit-plane, retained-vertex triangulation and 3D RDP policies.
use super::*;

#[cfg(feature = "native-geospatial")]
mod native;
#[cfg(any(not(feature = "native-geospatial"), test))]
mod portable;
#[cfg(feature = "native-geospatial")]
pub(super) use native::{quiet_unless_diagnostics, GeometryHandle};
#[cfg(not(feature = "native-geospatial"))]
pub(super) use portable::GeometryHandle;

fn data(message: impl Into<String>) -> FeatureFailure {
    FeatureFailure::reject(message)
}
fn outline(message: &str) -> FeatureFailure {
    FeatureFailure::outline(message)
}
fn is_outline(error: &FeatureFailure) -> bool {
    error.is_outline()
}
fn opened(ring: &[Point]) -> &[Point] {
    if ring.len() > 1 && ring.first() == ring.last() {
        &ring[..ring.len() - 1]
    } else {
        ring
    }
}
struct Plane {
    origin: Point,
    axes: [Point; 3],
}
impl Plane {
    fn fit(points: &[Point]) -> Result<Self, FeatureFailure> {
        if points.len() < 3 {
            return Err(data("polygon rings need three distinct vertices"));
        }
        let mut origin = [0.; 3];
        for p in points {
            origin = add(origin, mul(*p, 1. / points.len() as f64));
        }
        let mut cov = [[0.; 3]; 3];
        for p in points {
            let d = sub(*p, origin);
            for i in 0..3 {
                for j in 0..3 {
                    cov[i][j] += d[i] * d[j];
                }
            }
        }
        let scale = cov
            .iter()
            .flatten()
            .map(|v| v.abs())
            .fold(0., f64::max)
            .max(f64::MIN_POSITIVE);
        if !scale.is_finite() {
            return Err(data("nonfinite polygon plane"));
        }
        for row in &mut cov {
            for v in row {
                *v /= scale;
            }
        }
        Ok(Self {
            origin,
            axes: crate::bbox::jacobi_eigenvectors(cov),
        })
    }
    fn project(&self, p: Point) -> Point {
        let d = sub(p, self.origin);
        [dot(d, self.axes[0]), dot(d, self.axes[1]), 0.]
    }
    fn deviation(&self, rings: &[Vec<Point>]) -> f64 {
        rings
            .iter()
            .flatten()
            .map(|p| dot(sub(*p, self.origin), self.axes[2]).abs())
            .fold(0., f64::max)
    }
}

fn intrinsic_chart(rings: &[Vec<Point>], geographic: bool) -> Vec<Vec<Point>> {
    let scale = if geographic {
        6378137. * std::f64::consts::PI / 180.
    } else {
        1.
    };
    let origin = rings[0][0];
    rings
        .iter()
        .map(|r| {
            r.iter()
                .map(|p| [(p[0] - origin[0]) * scale, (p[1] - origin[1]) * scale, 0.])
                .collect()
        })
        .collect()
}

pub(super) struct Polygon {
    pub positions: Vec<Point>,
    pub source_positions: Option<Vec<Point>>,
    pub indices: Vec<u32>,
    pub loops: Vec<u32>,
    pub triangle_offsets: Vec<u32>,
    pub loop_offsets: Vec<u32>,
    pub report: Value,
}
#[cfg(test)]
pub(super) fn polygon(rings: &[Vec<Point>], repair: bool) -> Result<Polygon, FeatureFailure> {
    polygon_in_chart(rings, None, repair)
}

pub(super) fn polygon_for(
    feature: &Feature,
    rings: &[Vec<Point>],
    index: usize,
    center: Point,
    repair: bool,
) -> Result<Polygon, FeatureFailure> {
    let chart = feature
        .intrinsic
        .as_ref()
        .map(|intrinsic| {
            intrinsic
                .rings(index)
                .map(|rings| {
                    (
                        rings,
                        intrinsic.geographic,
                        sub(intrinsic.earth_center, center),
                    )
                })
                .ok_or_else(|| data("intrinsic polygon correspondence was lost"))
        })
        .transpose()?;
    polygon_in_chart(rings, chart, repair)
}

fn polygon_in_chart(
    rings: &[Vec<Point>],
    chart: Option<(&[Vec<Point>], bool, Point)>,
    repair: bool,
) -> Result<Polygon, FeatureFailure> {
    if rings.is_empty() || rings.iter().any(|r| opened(r).len() < 3) {
        return Err(data("polygon rings need three distinct vertices"));
    }
    let rings: Vec<Vec<_>> = rings.iter().map(|r| opened(r).to_vec()).collect();
    let plane = Plane::fit(&rings[0])?;
    let deviation = plane.deviation(&rings);
    let source_rings =
        chart.map(|(rings, _, _)| rings.iter().map(|r| opened(r).to_vec()).collect::<Vec<_>>());
    if source_rings.as_ref().is_some_and(|source| {
        source.len() != rings.len() || source.iter().zip(&rings).any(|(a, b)| a.len() != b.len())
    }) {
        return Err(data("intrinsic polygon vertex correspondence was lost"));
    }
    let projected: Vec<Vec<_>> = if let Some(source) = &source_rings {
        // Uniform metre scale preserves source XY topology. In particular,
        // retain the source longitude chart verbatim: shortest-edge unwrapping
        // would change the declared Cartesian interpolation of its edges.
        intrinsic_chart(source, chart.unwrap().1)
    } else {
        rings
            .iter()
            .map(|r| r.iter().map(|p| plane.project(*p)).collect())
            .collect()
    };
    let mut positions = Vec::new();
    let mut source_positions = source_rings.as_ref().map(|_| Vec::new());
    let mut lookup = BTreeMap::new();
    let mut segments = Vec::new();
    let mut duplicates = 0;
    for (ring_index, (ring, xy)) in rings.iter().zip(&projected).enumerate() {
        for i in 0..ring.len() {
            let q = xy[i];
            let k = key(q);
            if let std::collections::btree_map::Entry::Vacant(e) = lookup.entry(k) {
                e.insert(positions.len() as u32);
                positions.push(ring[i]);
                if let Some(points) = &mut source_positions {
                    points.push(source_rings.as_ref().unwrap()[ring_index][i]);
                }
            } else {
                duplicates += 1;
                if !repair {
                    return Err(data(
                        "duplicate/shared polygon ring vertices require --repair",
                    ));
                }
            }
            segments.push((
                q,
                xy[(i + 1) % xy.len()],
                ring[i],
                ring[(i + 1) % ring.len()],
                source_rings.as_ref().map_or(ring[i], |r| r[ring_index][i]),
                source_rings
                    .as_ref()
                    .map_or(ring[(i + 1) % ring.len()], |r| {
                        r[ring_index][(i + 1) % ring.len()]
                    }),
            ));
        }
    }
    let shape = GeometryHandle::polygon(&projected)?;
    let valid = shape.valid()?;
    if !valid && !repair {
        return Err(data(
            "invalid polygon topology; inspect source or explicitly use --repair",
        ));
    }
    let repaired = if valid { shape } else { shape.repair()? };
    let parts = repaired.parts()?;
    if parts.is_empty() {
        return Err(outline("polygon has no filled area"));
    }
    let mut added = 0;
    let mut spread: f64 = 0.;
    let mut position = |q: [f64; 2]| -> Result<u32, FeatureFailure> {
        let q = [q[0], q[1], 0.];
        let k = key(q);
        if let Some(i) = lookup.get(&k) {
            return Ok(*i);
        }
        let candidates: Vec<_> = segments
            .iter()
            .map(|(a, b, p, r, source_a, source_b)| {
                let ab = sub(*b, *a);
                let t = (dot(sub(q, *a), ab) / dot(ab, ab).max(1e-30)).clamp(0., 1.);
                (
                    norm(sub(add(*a, mul(ab, t)), q)),
                    add(*p, mul(sub(*r, *p), t)),
                    add(*source_a, mul(sub(*source_b, *source_a), t)),
                )
            })
            .collect();
        let closest = candidates
            .iter()
            .map(|(d, _, _)| *d)
            .fold(f64::INFINITY, f64::min);
        if closest > 1e-6 {
            return Err(data("triangulator added a point away from source edges"));
        }
        let hits: Vec<_> = candidates
            .iter()
            .filter(|(d, _, _)| *d < 1e-8f64.max(closest + 1e-9))
            .map(|(_, p, source)| (*p, *source))
            .collect();
        let point = hits.iter().fold([0.; 3], |sum, (p, _)| {
            add(sum, mul(*p, 1. / hits.len() as f64))
        });
        let adjustment = hits
            .iter()
            .map(|(p, _)| norm(sub(*p, point)))
            .fold(0., f64::max);
        spread = spread.max(adjustment);
        if adjustment > 0.02 {
            return Err(outline(
                "projected intersection differs by more than 2 cm in 3D; source needs review",
            ));
        }
        let index = positions.len() as u32;
        lookup.insert(k, index);
        positions.push(point);
        if let Some(points) = &mut source_positions {
            points.push(hits.iter().fold([0.; 3], |sum, (_, p)| {
                add(sum, mul(*p, 1. / hits.len() as f64))
            }));
        }
        added += 1;
        Ok(index)
    };
    let (mut indices, mut loops, mut triangle_offsets, mut loop_offsets) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for part in &parts {
        triangle_offsets.push(indices.len() as u32);
        loop_offsets.push(loops.len() as u32);
        for (r, ring) in part.rings()?.into_iter().enumerate() {
            let mut clean = Vec::new();
            for q in ring {
                if clean.last() != Some(&q) {
                    clean.push(q);
                }
            }
            let area: f64 = (0..clean.len())
                .map(|i| {
                    clean[i][0] * clean[(i + 1) % clean.len()][1]
                        - clean[i][1] * clean[(i + 1) % clean.len()][0]
                })
                .sum();
            if (area > 0.) != (r == 0) {
                clean.reverse();
            }
            for q in clean {
                loops.push(position(q)?);
            }
            loops.push(u32::MAX);
        }
        for triangle in part.triangulate()?.parts()? {
            let rings = triangle.rings()?;
            let xy = &rings[0];
            if xy.len() != 3 {
                return Err(data("triangulator produced a non-triangle"));
            }
            let mut tri = xy
                .iter()
                .map(|q| position(*q))
                .collect::<Result<Vec<_>, _>>()?;
            if (xy[1][0] - xy[0][0]) * (xy[2][1] - xy[0][1])
                - (xy[1][1] - xy[0][1]) * (xy[2][0] - xy[0][0])
                < 0.
            {
                tri.reverse();
            }
            indices.extend(tri);
        }
    }
    let mut collapsed_pole_triangles = 0;
    if let Some((_, true, earth_center)) = chart {
        let mut filled = Vec::new();
        let mut offset = 0;
        for (index, source_tri) in indices.as_chunks::<3>().0.iter().enumerate() {
            while offset < triangle_offsets.len() && triangle_offsets[offset] == (index * 3) as u32
            {
                triangle_offsets[offset] = filled.len() as u32;
                offset += 1;
            }
            let mut tri = *source_tri;
            let [a, b, c] = [
                positions[tri[0] as usize],
                positions[tri[1] as usize],
                positions[tri[2] as usize],
            ];
            let source = source_positions.as_ref().unwrap();
            let pole_or_seam = (0..3).any(|i| {
                let p = source[tri[i] as usize];
                let q = source[tri[(i + 1) % 3] as usize];
                let pole = (p[1].abs() - 90.).abs() < 1e-10 && (p[1] - q[1]).abs() < 1e-10;
                let seam = (p[0] - q[0]).abs() > 359.9999999999
                    && ((p[0] - q[0]).abs() - 360.).abs() < 1e-10
                    && p[1] == q[1];
                p[2] == q[2]
                    && (pole || seam)
                    && norm(sub(
                        positions[tri[i] as usize],
                        positions[tri[(i + 1) % 3] as usize],
                    )) < 1e-6
            });
            if pole_or_seam {
                // A chart face with equivalent pole/seam vertices has no
                // physical filled area. Retain every source vertex in boundary
                // loops, but do not turn this zero-area face into a fragment.
                collapsed_pole_triangles += 1;
                continue;
            }
            let ab = sub(b, a);
            let ac = sub(c, a);
            let normal = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            if dot(normal, sub(mul(add(add(a, b), c), 1. / 3.), earth_center)) < 0. {
                tri.reverse();
            }
            filled.extend(tri);
        }
        indices = filled;
    }

    loops.pop();
    if indices.is_empty() {
        return Err(data("polygon produced no triangles"));
    }
    Ok(Polygon {
        positions,
        source_positions,
        indices,
        loops,
        triangle_offsets,
        loop_offsets,
        report: json!({"planarityDeviationMetres":deviation,"topologyRepaired":!valid,
        "triangulationSpace":if chart.is_some(){"intrinsic source XY"}else{"best-fit 3D"},"collapsedPoleTriangles":collapsed_pole_triangles,
        "duplicateVertices":duplicates,"addedIntersectionVertices":added,"maximumIntersectionAdjustmentMetres":spread,"polygonParts":parts.len()}),
    })
}

pub(super) fn validate(
    feature: &mut Feature,
    repair: bool,
    ambiguous: bool,
) -> Result<Vec<Value>, FeatureFailure> {
    let polygons = match &feature.geometry {
        Geometry::Polygon(r) => vec![r],
        Geometry::MultiPolygon(p) => p.iter().collect(),
        _ => Vec::new(),
    };
    let mut reports = Vec::new();
    for (index, rings) in polygons.iter().enumerate() {
        if let Err(error) = polygon_for(feature, rings, index, [0.; 3], repair) {
            if !ambiguous || !is_outline(&error) {
                return Err(error);
            }
            reports.push(json!({"sourceId":feature.source_id(),"sourceLayer":feature.layer(),"topologyRepaired":false,
            "outputGeometry":"outline","reason":error.to_string()}));
        }
    }
    if !reports.is_empty() {
        feature.geometry =
            Geometry::MultiLineString(polygons.iter().flat_map(|r| r.iter().cloned()).collect());
        feature.intrinsic = None;
    }
    if feature.geometry.paths().iter().any(|p| {
        p.is_empty()
            || (matches!(
                feature.geometry,
                Geometry::LineString(_) | Geometry::MultiLineString(_)
            ) && p.len() < 2)
    }) {
        return Err(data("empty/degenerate feature"));
    }
    Ok(reports)
}

pub(super) fn simplify_path(
    source: &[Point],
    tolerance: f64,
    locked: &BTreeSet<PointKey>,
    closed: bool,
) -> (Vec<Point>, f64) {
    let source = if closed { opened(source) } else { source };
    if source.len() < (if closed { 4 } else { 3 }) || tolerance <= 0. {
        return (source.to_vec(), 0.);
    }
    let mut points = source.to_vec();
    let mut anchors = BTreeSet::from([0, points.len() - 1]);
    if closed {
        let split = (0..points.len())
            .max_by(|a, b| {
                norm(sub(points[*a], points[0]))
                    .total_cmp(&norm(sub(points[*b], points[0])))
                    .then_with(|| b.cmp(a))
            })
            .unwrap();
        if split == 0 {
            return (points, 0.);
        }
        points.push(points[0]);
        anchors = BTreeSet::from([0, split, points.len() - 1]);
    }
    anchors.extend(
        points
            .iter()
            .enumerate()
            .filter(|(_, p)| locked.contains(&key(**p)))
            .map(|(i, _)| i),
    );
    let mut keep = anchors.clone();
    let a: Vec<_> = anchors.into_iter().collect();
    let mut pending: Vec<_> = a.windows(2).map(|p| (p[0], p[1])).collect();
    let mut error: f64 = 0.;
    while let Some((a, b)) = pending.pop() {
        if b <= a + 1 {
            continue;
        }
        let ab = sub(points[b], points[a]);
        let (index, distance) = (a + 1..b)
            .map(|i| {
                let t = (dot(sub(points[i], points[a]), ab) / dot(ab, ab).max(1e-30)).clamp(0., 1.);
                (i, norm(sub(sub(points[i], points[a]), mul(ab, t))))
            })
            .max_by(|(i, a), (j, b)| a.total_cmp(b).then_with(|| j.cmp(i)))
            .unwrap();
        if distance > tolerance {
            keep.insert(index);
            pending.extend([(a, index), (index, b)]);
        } else {
            error = error.max(distance);
        }
    }
    let mut result: Vec<_> = keep.into_iter().map(|i| points[i]).collect();
    if closed {
        result.pop();
        if result.len() < 3 {
            return (source.to_vec(), 0.);
        }
    }
    (result, error)
}

/// Simplified rings, the measured error, and why simplification was skipped.
type SimplifiedPolygon = (Vec<Vec<Point>>, f64, Option<&'static str>);

fn simplify_polygon(
    rings: &[Vec<Point>],
    tolerance: f64,
    locked: &BTreeSet<PointKey>,
) -> Result<SimplifiedPolygon, FeatureFailure> {
    let original = rings.to_vec();
    let opened: Vec<_> = rings.iter().map(|r| opened(r).to_vec()).collect();
    let plane = Plane::fit(&opened[0])?;
    let deviation = plane.deviation(&opened);
    if 2. * deviation >= tolerance {
        return Ok((original, 0., Some("nonplanar polygon retained")));
    }
    let shape = |rings: &[Vec<Point>]| {
        GeometryHandle::polygon(
            &rings
                .iter()
                .map(|r| r.iter().map(|p| plane.project(*p)).collect())
                .collect::<Vec<_>>(),
        )
    };
    if !shape(&opened)?.valid()? {
        return Ok((
            original,
            0.,
            Some("invalid source topology retained for existing repair policy"),
        ));
    }
    let mut candidates = Vec::new();
    let mut error: f64 = 0.;
    for ring in &opened {
        let (p, e) = simplify_path(ring, tolerance - 2. * deviation, locked, true);
        candidates.push(p);
        error = error.max(e);
    }
    let candidate = shape(&candidates)?;
    if !candidate.valid()? || candidate.area()? <= 0. {
        return Ok((
            original,
            0.,
            Some("simplification would change polygon topology"),
        ));
    }
    if candidates.iter().map(Vec::len).sum::<usize>() == opened.iter().map(Vec::len).sum::<usize>()
    {
        return Ok((original, 0., None));
    }
    for ring in &mut candidates {
        ring.push(ring[0]);
    }
    Ok((candidates, error + 2. * deviation, None))
}
/// RDP retains original vertices in ring order. Carry the matching source
/// positions with them so validation and later encoding use the same chart.
fn retain_intrinsic(
    original: &[Vec<Point>],
    source: &[Vec<Point>],
    candidate: &[Vec<Point>],
) -> Result<Vec<Vec<Point>>, FeatureFailure> {
    if original.len() != source.len() || original.len() != candidate.len() {
        return Err(data("intrinsic ring correspondence was lost"));
    }
    original
        .iter()
        .zip(source)
        .zip(candidate)
        .map(|((original, source), candidate)| {
            let original = opened(original);
            let source = opened(source);
            if original.len() != source.len() {
                return Err(data("intrinsic vertex correspondence was lost"));
            }
            let mut from = 0;
            let mut result = Vec::new();
            for point in opened(candidate) {
                let index = original[from..]
                    .iter()
                    .position(|p| p == point)
                    .map(|i| from + i)
                    .ok_or_else(|| data("simplification introduced an unmatched source vertex"))?;
                result.push(source[index]);
                from = index + 1;
            }
            if candidate.first() == candidate.last() {
                result.push(result[0]);
            }
            Ok(result)
        })
        .collect()
}

pub(super) fn simplify(
    feature: &Feature,
    tolerance: f64,
    locked: &BTreeSet<PointKey>,
    reports: &mut Vec<Value>,
    parent_repair: bool,
) -> Result<(Feature, f64), FeatureFailure> {
    let mut result = feature.clone();
    let mut error: f64 = 0.;
    let mut fallback = false;
    match &mut result.geometry {
        Geometry::Point(_) | Geometry::MultiPoint(_) => {}
        Geometry::LineString(p) => {
            let (s, e) = simplify_path(p, tolerance, locked, false);
            *p = s;
            error = e;
        }
        Geometry::MultiLineString(parts) => {
            for p in parts {
                let (s, e) = simplify_path(p, tolerance, locked, false);
                *p = s;
                error = error.max(e);
            }
        }
        geometry => {
            let polygons = match geometry {
                Geometry::Polygon(r) => vec![r],
                Geometry::MultiPolygon(p) => p.iter_mut().collect(),
                _ => unreachable!(),
            };
            for (index, rings) in polygons.into_iter().enumerate() {
                let (candidate, e, mut reason) = simplify_polygon(rings, tolerance, locked)?;
                let mut source_candidate = None;
                if candidate != *rings {
                    if let Some(intrinsic) = &result.intrinsic {
                        let source = intrinsic
                            .rings(index)
                            .ok_or_else(|| data("intrinsic polygon correspondence was lost"))?;
                        let retained = retain_intrinsic(rings, source, &candidate)?;
                        let chart = intrinsic_chart(&retained, intrinsic.geographic);
                        let shape = GeometryHandle::polygon(&chart)?;
                        if !shape.valid()? || shape.area()? <= 0. {
                            reason = Some("simplification would change intrinsic source topology");
                        } else {
                            source_candidate = Some(retained);
                        }
                    }
                }
                if reason != Some("simplification would change intrinsic source topology") {
                    *rings = candidate;
                    error = error.max(e);
                    if let Some(source_candidate) = source_candidate {
                        let intrinsic = result.intrinsic.as_mut().unwrap();
                        match &mut intrinsic.geometry {
                            Geometry::Polygon(rings) => *rings = source_candidate,
                            Geometry::MultiPolygon(polygons) => polygons[index] = source_candidate,
                            _ => return Err(data("intrinsic polygon correspondence was lost")),
                        }
                    }
                }
                if let Some(reason) = reason {
                    fallback |= reason != "nonplanar polygon retained";
                    reports.push(json!({"sourceId":feature.source_id(),"reason":reason}));
                }
            }
        }
    }
    if parent_repair
        && !feature.surface_fragment
        && (result.geometry == feature.geometry || fallback)
    {
        let polygons = match &feature.geometry {
            Geometry::Polygon(r) => vec![r],
            Geometry::MultiPolygon(p) => p.iter().collect(),
            _ => return Ok((result, error)),
        };
        let mut bound: f64 = 0.;
        for rings in &polygons {
            let (lo, hi) = bounds(rings.iter().flatten())?;
            bound = bound.max(norm(sub(hi, lo)));
        }
        if bound <= tolerance {
            let mut outlines = Vec::new();
            for rings in polygons {
                for ring in rings {
                    let p = opened(ring);
                    let farthest = (0..p.len())
                        .max_by(|a, b| {
                            norm(sub(p[*a], p[0]))
                                .total_cmp(&norm(sub(p[*b], p[0])))
                                .then_with(|| b.cmp(a))
                        })
                        .unwrap();
                    let mut keep = BTreeSet::from([0, farthest]);
                    keep.extend(
                        p.iter()
                            .enumerate()
                            .filter(|(_, p)| locked.contains(&key(**p)))
                            .map(|(i, _)| i),
                    );
                    if keep.len() < 2 {
                        return Ok((result, error));
                    }
                    let mut values: Vec<_> = keep.into_iter().map(|i| p[i]).collect();
                    values.push(values[0]);
                    outlines.push(values);
                }
            }
            result = feature.clone();
            result.geometry = Geometry::MultiLineString(outlines);
            result.intrinsic = None;
            reports.push(json!({"sourceId":feature.source_id(),"sourceLayer":feature.layer(),"reason":"parent polygon replaced by bounded source outline",
                "substitution":"parentOutline","sourceGeometry":if matches!(feature.geometry,Geometry::Polygon(_)){"Polygon"}else{"MultiPolygon"},
                "geometryErrorMetres":bound,"toleranceMetres":tolerance,"retainedSharedVertices":locked.len()}));
            return Ok((result, bound));
        }
    }
    Ok((result, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_polygon_hole_retains_original_xyz_and_area() {
        let rings = vec![
            vec![
                [0., 0., 0.],
                [20., 0., 0.],
                [20., 0., 20.],
                [0., 0., 20.],
                [0., 0., 0.],
            ],
            vec![
                [5., 0., 5.],
                [5., 0., 10.],
                [10., 0., 10.],
                [10., 0., 5.],
                [5., 0., 5.],
            ],
        ];
        let mesh = polygon(&rings, false).unwrap();
        assert_eq!(mesh.positions.len(), 8);
        assert!(mesh
            .positions
            .iter()
            .all(|p| rings.iter().flatten().any(|s| s == p)));
        let area: f64 = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let a = mesh.positions[t[0] as usize];
                let b = mesh.positions[t[1] as usize];
                let c = mesh.positions[t[2] as usize];
                ((b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0])).abs() / 2.
            })
            .sum();
        assert!((area - 375.).abs() < 1e-10);
        assert_eq!(mesh.loops.iter().filter(|i| **i == u32::MAX).count(), 1);
    }

    #[test]
    fn rdp_bounds_three_dimensional_distance_and_pins_shared_vertices() {
        let points: Vec<_> = (0..101)
            .map(|i| {
                [
                    i as f64,
                    (i as f64 * 0.1).sin() * 0.1,
                    (i as f64 * 0.2).cos() * 0.1,
                ]
            })
            .collect();
        let locked = BTreeSet::from([key(points[31]), key(points[67])]);
        let (path, error) = simplify_path(&points, 0.15, &locked, false);
        assert!(path.len() < points.len() / 2);
        assert!(path.contains(&points[31]) && path.contains(&points[67]));
        for p in points {
            let distance = path
                .windows(2)
                .map(|ab| {
                    let d = sub(ab[1], ab[0]);
                    let t = (dot(sub(p, ab[0]), d) / dot(d, d)).clamp(0., 1.);
                    norm(sub(p, add(ab[0], mul(d, t))))
                })
                .fold(f64::INFINITY, f64::min);
            assert!(distance <= error + 1e-12);
        }
        assert!(error <= 0.15);
    }

    #[test]
    fn repair_requires_opt_in_and_collapsed_surfaces_require_outline_policy() {
        let bow = vec![vec![
            [0., 0., 0.],
            [2., 2., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [0., 0., 0.],
        ]];
        assert!(polygon(&bow, false).is_err());
        assert!(polygon(&bow, true).unwrap().report["topologyRepaired"] == true);
        let collapsed = vec![vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [2., 0., 0.],
            [1., 0., 0.],
            [0., 0., 0.],
        ]];
        assert!(is_outline(&polygon(&collapsed, true).err().unwrap()));
    }

    #[test]
    fn repaired_crossings_interpolate_source_xyz_and_enforce_two_centimetre_adjustment() {
        let bow = |separation| {
            vec![vec![
                [0., 0., 0.],
                [2., 2., 0.],
                [2., 0., separation],
                [0., 2., separation],
            ]]
        };
        let rings = bow(0.02);
        let mesh = polygon(&rings, true).unwrap();
        assert_eq!(mesh.positions.len(), 5);
        assert_eq!(mesh.report["addedIntersectionVertices"], 1);
        for p in rings.iter().flatten() {
            assert!(mesh.positions.contains(p));
        }
        let intersection = mesh
            .positions
            .iter()
            .find(|p| !rings[0].contains(p))
            .unwrap();
        assert!(norm(sub(*intersection, [1., 1., 0.01])) < 1e-10);
        assert!(
            (mesh.report["maximumIntersectionAdjustmentMetres"]
                .as_f64()
                .unwrap()
                - 0.01)
                .abs()
                < 1e-10
        );
        let error = polygon(&bow(0.08), true).err().unwrap();
        assert!(is_outline(&error), "{error}");
        assert!(error.to_string().contains("more than 2 cm"));
    }

    #[test]
    fn shared_vertices_require_repair_and_keep_exact_source_positions() {
        let rings = vec![vec![
            [0., 0., 2.],
            [1., 0., 2.],
            [1., 0., 2.],
            [2., 0., 2.],
            [2., 2., 2.],
            [0., 2., 2.],
        ]];
        assert!(polygon(&rings, false)
            .err()
            .unwrap()
            .to_string()
            .contains("require --repair"));
        let mesh = polygon(&rings, true).unwrap();
        assert_eq!(mesh.report["duplicateVertices"], 1);
        assert_eq!(mesh.report["addedIntersectionVertices"], 0);
        assert_eq!(mesh.positions.len(), 5);
        assert!(mesh.positions.iter().all(|p| rings[0].contains(p)));
        assert!(mesh.indices.contains(
            &mesh
                .positions
                .iter()
                .position(|p| *p == [1., 0., 2.])
                .unwrap()
                .try_into()
                .unwrap()
        ));
    }
    #[test]
    fn native_polygon_lod_retains_holes_and_bounds_both_boundary_directions() {
        fn ring(radius: f64) -> Vec<Point> {
            let mut points: Vec<_> = (0..200)
                .map(|i| {
                    let a = i as f64 * std::f64::consts::TAU / 200.;
                    [radius * a.cos(), radius * a.sin(), 0.005 * (5. * a).sin()]
                })
                .collect();
            points.push(points[0]);
            points
        }
        let original = vec![ring(0.15), ring(0.05)];
        let (coarse, error, reason) = simplify_polygon(&original, 0.03, &BTreeSet::new()).unwrap();
        assert!(reason.is_none());
        assert_eq!(coarse.len(), 2);
        assert!(
            coarse.iter().map(Vec::len).sum::<usize>()
                < original.iter().map(Vec::len).sum::<usize>() / 4
        );
        assert!(error <= 0.03);
        assert!(!polygon(&coarse, false).unwrap().indices.is_empty());
        let distance = |p: Point, path: &[Point]| {
            path.windows(2)
                .map(|ab| {
                    let d = sub(ab[1], ab[0]);
                    let t = (dot(sub(p, ab[0]), d) / dot(d, d).max(1e-30)).clamp(0., 1.);
                    norm(sub(p, add(ab[0], mul(d, t))))
                })
                .fold(f64::INFINITY, f64::min)
        };
        for (before, after) in original.iter().zip(&coarse) {
            for p in before {
                assert!(distance(*p, after) <= error + 1e-12);
            }
            for edge in after.windows(2) {
                for i in 0..100 {
                    let p = add(edge[0], mul(sub(edge[1], edge[0]), i as f64 / 100.));
                    assert!(distance(p, before) <= error + 1e-12);
                }
            }
        }
        let nonplanar = vec![vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [1., 1., 1.],
            [0., 1., 0.],
            [0., 0., 0.],
        ]];
        let (coarse, error, reason) = simplify_polygon(&nonplanar, 0.01, &BTreeSet::new()).unwrap();
        assert_eq!(coarse, nonplanar);
        assert_eq!(error, 0.);
        assert!(reason.unwrap().contains("nonplanar"));
    }

    #[test]
    fn bounded_native_parent_outlines_preserve_locked_vertices_and_properties() {
        let corners = [
            [0., 0., 0.],
            [2., 2., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [0., 0., 0.],
        ];
        let mut ring = Vec::new();
        for pair in corners.windows(2) {
            for i in 0..25 {
                ring.push(add(pair[0], mul(sub(pair[1], pair[0]), i as f64 / 25.)));
            }
        }
        ring.push(ring[0]);
        let feature: Feature=serde_json::from_value(json!({"properties":{"_source_id":"1","_source_layer":"bow","name":"source"},"geometry":{"type":"Polygon","coordinates":[ring]}})).unwrap();
        let locked = BTreeSet::from([key(ring[20])]);
        let (baseline, error) = simplify(&feature, 3., &locked, &mut Vec::new(), false).unwrap();
        assert_eq!(baseline.geometry, feature.geometry);
        assert_eq!(error, 0.);
        let mut reports = Vec::new();
        let (coarse, error) = simplify(&feature, 3., &locked, &mut reports, true).unwrap();
        let Geometry::MultiLineString(lines) = coarse.geometry else {
            panic!("expected parent outline")
        };
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains(&ring[20]));
        assert!(lines[0].len() < ring.len());
        assert!(error <= 3.);
        assert_eq!(coarse.properties, feature.properties);
        assert_eq!(reports.last().unwrap()["substitution"], "parentOutline");
        let (near, error) = simplify(&feature, 0.1, &locked, &mut Vec::new(), true).unwrap();
        assert_eq!(near.geometry, feature.geometry);
        assert_eq!(error, 0.);
        let mut holes = feature.clone();
        let circles = [2., 0.5].map(|radius| {
            let mut ring: Vec<_> = (0..100)
                .map(|i| {
                    let a = i as f64 * std::f64::consts::TAU / 100.;
                    [radius * a.cos(), radius * a.sin(), 0.]
                })
                .collect();
            ring.push(ring[0]);
            ring
        });
        holes.geometry = Geometry::Polygon(circles.to_vec());
        let (coarse, error) =
            simplify(&holes, 6., &BTreeSet::new(), &mut Vec::new(), true).unwrap();
        let Geometry::MultiLineString(lines) = coarse.geometry else {
            panic!("expected bounded hole outlines")
        };
        assert_eq!(lines.len(), 2);
        assert!(error <= 6.);
    }
}

#[cfg(test)]
mod countries_tests;
