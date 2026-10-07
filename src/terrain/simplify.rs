//! Reuse the mesh simplification kernel with metre tolerances and fixed edges.
//! Validate the decoded surfaces after quantization; reject candidates with
//! flipped faces, missing coverage or altered boundary segments.
use super::quantized::Statistics;
use crate::Error;
use meshopt::{simplify::SimplifyOptions, utilities::VertexDataAdapter};
use std::collections::HashMap;

pub(super) fn reduce(
    xyz: &[[f64; 3]],
    uv: &[i32],
    heights: &[i32],
    height_range: f64,
    original: &[usize],
    max_error: f64,
) -> Result<(Vec<usize>, Statistics), Error> {
    let mut statistics = Statistics {
        input_vertices: xyz.len() as u64,
        input_triangles: (original.len() / 3) as u64,
        output_triangles: (original.len() / 3) as u64,
        ..Default::default()
    };
    if max_error == 0. {
        return Ok((original.to_vec(), statistics));
    }
    // Float32 is only the simplifier's working representation. Subtract a
    // local origin and reserve its rounding error; output retains the original
    // quantized coordinates/height codes without moving any retained vertex.
    let count = uv.len();
    let distance = |a: usize, b: usize| {
        (0..3)
            .map(|axis| (xyz[a][axis] - xyz[b][axis]).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    let width = distance(count * (count / 2), count * (count / 2) + count - 1);
    let height = distance(count / 2, count * (count - 1) + count / 2);
    // Keep geometric topology in the terrain's 2D parameter domain. A curved
    // ECEF border can otherwise produce 3D-valid triangles with zero UV area.
    // Metric ECEF/height attributes still preserve curvature and relief.
    let positions: Vec<[f32; 3]> = (0..xyz.len())
        .map(|i| {
            [
                ((f64::from(uv[i % count]) / 32767. - 0.5) * width) as f32,
                ((f64::from(uv[i / count]) / 32767. - 0.5) * height) as f32,
                0.,
            ]
        })
        .collect();
    // Remove the affine part of ECEF/height attributes. It is interpolated
    // exactly by either triangulation, so it contributes no surface error.
    // Keeping only relief/curvature avoids cancellation in float32 quadrics.
    let residuals: Vec<[f64; 4]> = xyz
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let u = f64::from(uv[i % count]) / 32767.;
            let v = f64::from(uv[i / count]) / 32767.;
            let mut values = [0.; 4];
            for axis in 0..3 {
                values[axis] = p[axis]
                    - xyz[0][axis]
                    - u * (xyz[count - 1][axis] - xyz[0][axis])
                    - v * (xyz[count * (count - 1)][axis] - xyz[0][axis]);
            }
            values[3] = (f64::from(heights[i] - heights[0])
                - u * f64::from(heights[count - 1] - heights[0])
                - v * f64::from(heights[count * (count - 1)] - heights[0]))
                / 32767.
                * height_range;
            values
        })
        .collect();
    let attributes: Vec<[f32; 4]> = residuals.iter().map(|p| p.map(|v| v as f32)).collect();
    let locks: Vec<_> = (0..xyz.len())
        .map(|i| i < count || i >= count * (count - 1) || i % count == 0 || i % count == count - 1)
        .collect();
    let rounding = residuals
        .iter()
        .zip(&attributes)
        .map(|(p, q)| {
            let xyz_error = (0..3)
                .map(|axis| (p[axis] - f64::from(q[axis])).powi(2))
                .sum::<f64>()
                .sqrt();
            xyz_error.max((p[3] - f64::from(q[3])).abs())
        })
        .fold(0., f64::max);
    let budget = max_error - 2. * rounding;
    if budget <= 0.
        || (!positions.iter().flatten().all(|p| p.is_finite())
            || !attributes.iter().flatten().all(|p| p.is_finite()))
    {
        return Ok((original.to_vec(), statistics));
    }
    let adapter = VertexDataAdapter::new(bytemuck::cast_slice(&positions), 12, 0)
        .map_err(|e| Error::msg(format!("terrain meshopt adapter: {e}")))?;
    // meshopt normalizes geometric positions by their extent, but applies
    // attribute weights directly. Match that normalization so one metre of
    // ECEF/height attribute error has the same cost at every zoom level.
    let scale = meshopt::simplify::simplify_scale(&adapter);
    if !scale.is_finite() || scale <= 0. {
        return Ok((original.to_vec(), statistics));
    }
    let weights = [1. / scale; 4];
    let indices: Vec<_> = original.iter().map(|&i| i as u32).collect();
    // A quadric estimate can understate the decoded surface error. If reducing
    // all the way to the error threshold fails validation, also try less
    // aggressive triangle targets before retaining the complete source grid.
    let mut unchanged_below = -1_f32;
    for target in [
        0,
        indices.len() / 2,
        indices.len() * 3 / 4,
        indices.len() * 7 / 8,
        indices.len() * 15 / 16,
    ] {
        let mut reached_error = None;
        for divisor in [1., 2., 4., 8., 16., 32.] {
            let tolerance = (budget / divisor) as f32;
            // No collapse was possible at this tighter tolerance in the
            // unconstrained triangle-count search. Raising the count target
            // cannot enable one, so subsequent searches need not repeat it.
            if tolerance <= unchanged_below {
                break;
            }
            // Once meshopt reached this count target, a tolerance above its
            // maximum committed collapse error reproduces the same candidate.
            // Allow a rounding margin around the float32 error estimate.
            if reached_error.is_some_and(|error| tolerance > error * 1.00001) {
                continue;
            }
            let (candidate, estimate) = crate::hlod::simplify_border_locked(
                &indices,
                &adapter,
                target,
                tolerance,
                SimplifyOptions::ErrorAbsolute,
                Some(crate::hlod::SimplificationAttributes {
                    values: bytemuck::cast_slice(&attributes),
                    weights: &weights,
                    stride: 16,
                    locks: &locks,
                }),
            );
            if candidate.len() >= indices.len() {
                unchanged_below = tolerance;
                break;
            }
            reached_error = (candidate.len() <= target && estimate.is_finite()).then_some(estimate);
            let Some(candidate) = repair_collinear_faces(&candidate, uv) else {
                continue;
            };
            if candidate.len() >= indices.len() {
                break;
            }
            if candidate.len() < 3
                || !estimate.is_finite()
                || f64::from(estimate) + 2. * rounding > max_error
            {
                continue;
            }
            if let Some((height_error, surface_error)) =
                surface_errors(&candidate, uv, heights, xyz, height_range, max_error)
            {
                statistics.output_triangles = (candidate.len() / 3) as u64;
                statistics.max_added_height_error = height_error;
                statistics.max_added_surface_error = surface_error;
                statistics.max_meshopt_estimate = f64::from(estimate) + 2. * rounding;
                return Ok((
                    candidate.into_iter().map(|i| i as usize).collect(),
                    statistics,
                ));
            }
        }
    }
    Ok((original.to_vec(), statistics))
}

// Generic 3D simplification can leave collinear UV faces that preserve curved
// ECEF border strips. Remove those faces and split the surviving long edges at
// their original middle vertices. This restores a valid terrain triangulation
// and retains every original boundary segment without moving any vertex.
fn repair_collinear_faces(indices: &[u32], uv: &[i32]) -> Option<Vec<u32>> {
    let count = uv.len();
    let point = |id: u32| {
        [
            i64::from(uv[id as usize % count]),
            i64::from(uv[id as usize / count]),
        ]
    };
    let mut faces = Vec::new();
    let mut constraints = Vec::new();
    for &t in indices.as_chunks::<3>().0 {
        let area = cross(point(t[0]), point(t[1]), point(t[2]));
        if area < 0 {
            return None;
        }
        if area > 0 {
            faces.push(t);
            continue;
        }
        let mut order = t;
        order.sort_by_key(|&id| point(id));
        constraints.push((edge(order[0], order[2]), order[1]));
    }
    if constraints.is_empty() {
        return Some(indices.to_vec());
    }
    let mut adjacency: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (id, face) in faces.iter().enumerate() {
        for i in 0..3 {
            adjacency
                .entry(edge(face[i], face[(i + 1) % 3]))
                .or_default()
                .push(id);
        }
    }
    // Longest first handles chains of collinear faces from repeated collapses.
    constraints.sort_by_key(|&(e, _)| {
        std::cmp::Reverse({
            let a = point(e.0);
            let b = point(e.1);
            (a[0] - b[0]).abs() + (a[1] - b[1]).abs()
        })
    });
    constraints.dedup();
    for (segment, middle) in constraints {
        let p = point(middle);
        let segments = if adjacency.contains_key(&segment) {
            vec![segment]
        } else {
            let mut found: Vec<_> = adjacency
                .keys()
                .copied()
                .filter(|&(a, b)| {
                    let a = point(a);
                    let b = point(b);
                    p != a
                        && p != b
                        && cross(a, b, p) == 0
                        && (0..2).all(|i| p[i] >= a[i].min(b[i]) && p[i] <= a[i].max(b[i]))
                })
                .collect();
            found.sort_unstable();
            found
        };
        for segment in segments {
            let face_ids = adjacency.get(&segment)?.clone();
            for id in face_ids {
                let face = faces[id];
                let i = (0..3).find(|&i| edge(face[i], face[(i + 1) % 3]) == segment)?;
                let (a, b, c) = (face[i], face[(i + 1) % 3], face[(i + 2) % 3]);
                for i in 0..3 {
                    let key = edge(face[i], face[(i + 1) % 3]);
                    let neighbours = adjacency.get_mut(&key)?;
                    neighbours.retain(|&other| other != id);
                    if neighbours.is_empty() {
                        adjacency.remove(&key);
                    }
                }
                faces[id] = [a, middle, c];
                let added = faces.len();
                faces.push([middle, b, c]);
                for id in [id, added] {
                    for i in 0..3 {
                        adjacency
                            .entry(edge(faces[id][i], faces[id][(i + 1) % 3]))
                            .or_default()
                            .push(id);
                    }
                }
            }
        }
    }
    Some(faces.into_iter().flatten().collect())
}

fn cross(a: [i64; 2], b: [i64; 2], p: [i64; 2]) -> i64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
fn edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

fn surface_errors(
    indices: &[u32],
    uv: &[i32],
    heights: &[i32],
    xyz: &[[f64; 3]],
    range: f64,
    limit: f64,
) -> Option<(f64, f64)> {
    let count = uv.len();
    let point = |id: usize| [i64::from(uv[id % count]), i64::from(uv[id / count])];
    let mut covered = vec![false; count * count];
    let mut edge_counts = HashMap::new();
    let mut total_area = 0_i64;
    let mut max_error: f64 = 0.;
    let mut max_surface_error: f64 = 0.;
    for triangle in indices.as_chunks::<3>().0 {
        let ids = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        if ids.iter().any(|&i| i >= heights.len()) {
            return None;
        }
        let [a, b, c] = ids.map(point);
        let area = cross(a, b, c);
        if area <= 0 {
            return None;
        }
        total_area += area;
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            *edge_counts.entry(edge(a, b)).or_insert(0_u32) += 1;
        }
        let x0 = uv.partition_point(|&u| i64::from(u) < a[0].min(b[0]).min(c[0]));
        let x1 = uv.partition_point(|&u| i64::from(u) <= a[0].max(b[0]).max(c[0]));
        let y0 = uv.partition_point(|&v| i64::from(v) < a[1].min(b[1]).min(c[1]));
        let y1 = uv.partition_point(|&v| i64::from(v) <= a[1].max(b[1]).max(c[1]));
        for y in y0..y1 {
            for x in x0..x1 {
                let id = y * count + x;
                let p = point(id);
                let weights = [cross(b, c, p), cross(c, a, p), cross(a, b, p)];
                if weights.iter().any(|&w| w < 0) {
                    continue;
                }
                covered[id] = true;
                // Offset cancels. Evaluate differences in quantized codes before
                // scaling, avoiding cancellation for large absolute heights.
                let delta: f64 = (0..3)
                    .map(|i| f64::from(heights[ids[i]] - heights[id]) * weights[i] as f64)
                    .sum();
                let error = (delta / area as f64 * range / 32767.).abs();
                if !error.is_finite() || error > limit {
                    return None;
                }
                let displacement: [f64; 3] = std::array::from_fn(|axis| {
                    (0..3)
                        .map(|i| (xyz[ids[i]][axis] - xyz[id][axis]) * weights[i] as f64)
                        .sum::<f64>()
                        / area as f64
                });
                let surface_error = displacement.iter().map(|v| v * v).sum::<f64>().sqrt();
                if !surface_error.is_finite() || surface_error > limit {
                    return None;
                }
                max_surface_error = max_surface_error.max(surface_error);
                max_error = max_error.max(error);
            }
        }
    }
    if total_area != 2 * 32767_i64.pow(2) || !covered.iter().all(|&value| value) {
        return None;
    }
    // All original boundary segments remain exactly once, and every remaining
    // interior segment is manifold. This also keeps every edge sample present.
    for i in 0..count - 1 {
        for (a, b) in [
            (i, i + 1),
            (i * count, (i + 1) * count),
            (i * count + count - 1, (i + 1) * count + count - 1),
            (i + count * (count - 1), i + 1 + count * (count - 1)),
        ] {
            if edge_counts.remove(&edge(a as u32, b as u32)) != Some(1) {
                return None;
            }
        }
    }
    if !edge_counts.values().all(|&count| count == 2) {
        return None;
    }
    // Grid vertices alone can miss an error where a simplified edge crosses
    // an original edge. The difference is affine on each triangle intersection;
    // its norm/height extrema occur at intersection vertices. Check those too.
    // The topology pass already collected every interior edge. Boundary edges
    // were removed above after checking that they match the source exactly.
    for segment in edge_counts.into_keys() {
        let (height, surface) = edge_errors(segment, uv, heights, xyz, range, limit)?;
        max_error = max_error.max(height);
        max_surface_error = max_surface_error.max(surface);
    }
    Some((max_error, max_surface_error))
}

fn edge_errors(
    segment: (u32, u32),
    uv: &[i32],
    heights: &[i32],
    xyz: &[[f64; 3]],
    range: f64,
    limit: f64,
) -> Option<(f64, f64)> {
    let count = uv.len();
    let (a, b) = (segment.0 as usize, segment.1 as usize);
    let (ax, ay, bx, by) = (a % count, a / count, b % count, b / count);
    // A retained source edge interpolates the same two original vertices in
    // both meshes. Its displacement is identically zero, even on curved ECEF
    // geometry. The opposite cell diagonal still needs an intersection check.
    if (ax == bx && ay.abs_diff(by) == 1)
        || (ay == by && ax.abs_diff(bx) == 1)
        || (ax.abs_diff(bx) == 1 && ay.abs_diff(by) == 1 && (ax < bx) != (ay < by))
    {
        return Some((0., 0.));
    }
    let start = [f64::from(uv[a % count]), f64::from(uv[a / count])];
    let end = [f64::from(uv[b % count]), f64::from(uv[b / count])];
    let delta = [end[0] - start[0], end[1] - start[1]];
    let point = |t: f64| [start[0] + delta[0] * t, start[1] + delta[1] * t];
    let cell = |p: [f64; 2]| {
        std::array::from_fn::<_, 2, _>(|axis| {
            uv.partition_point(|&v| f64::from(v) <= p[axis])
                .saturating_sub(1)
                .min(count - 2)
        })
    };
    let evaluate = |t: f64| {
        let p = point(t);
        let [x, y] = cell(p);
        let u = ((p[0] - f64::from(uv[x])) / f64::from(uv[x + 1] - uv[x])).clamp(0., 1.);
        let v = ((p[1] - f64::from(uv[y])) / f64::from(uv[y + 1] - uv[y])).clamp(0., 1.);
        let i = y * count + x;
        let (ids, weights) = if u + v <= 1. {
            ([i, i + 1, i + count], [1. - u - v, u, v])
        } else {
            (
                [i + 1, i + count + 1, i + count],
                [1. - v, u + v - 1., 1. - u],
            )
        };
        let height = ((0..3)
            .map(|i| f64::from(heights[ids[i]] - heights[a]) * weights[i])
            .sum::<f64>()
            - t * f64::from(heights[b] - heights[a]))
        .abs()
            * range
            / 32767.;
        let displacement: [f64; 3] = std::array::from_fn(|axis| {
            (0..3)
                .map(|i| (xyz[ids[i]][axis] - xyz[a][axis]) * weights[i])
                .sum::<f64>()
                - t * (xyz[b][axis] - xyz[a][axis])
        });
        let surface = displacement.iter().map(|v| v * v).sum::<f64>().sqrt();
        (height.is_finite() && surface.is_finite() && height <= limit && surface <= limit)
            .then_some((height, surface))
    };
    let mut cuts = vec![0., 1.];
    for axis in 0..2 {
        if delta[axis] == 0. {
            continue;
        }
        let (first, last) = if axis == 0 { (ax, bx) } else { (ay, by) };
        for &v in &uv[first.min(last) + 1..first.max(last)] {
            cuts.push((f64::from(v) - start[axis]) / delta[axis]);
        }
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut errors: [f64; 2] = [0., 0.];
    // Endpoints are unchanged source vertices, so their errors are zero.
    for &t in &cuts[1..cuts.len() - 1] {
        let (h, s) = evaluate(t)?;
        errors[0] = errors[0].max(h);
        errors[1] = errors[1].max(s);
    }
    for pair in cuts.windows(2) {
        let [x, y] = cell(point((pair[0] + pair[1]) / 2.));
        let diagonal = |t: f64| {
            let p = point(t);
            (p[0] - f64::from(uv[x])) / f64::from(uv[x + 1] - uv[x])
                + (p[1] - f64::from(uv[y])) / f64::from(uv[y + 1] - uv[y])
                - 1.
        };
        let (first, last) = (diagonal(pair[0]), diagonal(pair[1]));
        if first * last < 0. {
            let t = pair[0] + (pair[1] - pair[0]) * first / (first - last);
            let (h, s) = evaluate(t)?;
            errors[0] = errors[0].max(h);
            errors[1] = errors[1].max(s);
        }
    }
    Some((errors[0], errors[1]))
}

#[cfg(test)]
mod tests {
    use super::surface_errors;

    #[test]
    fn triangle_intersections_catch_error_with_unchanged_grid_vertices() {
        // Both diagonals preserve all four vertices and boundary segments,
        // but their surfaces differ by half a metre at the cell centre.
        let uv = [0, 32767];
        let heights = [0, 0, 0, 32767];
        let xyz = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 1.]];
        let alternate_diagonal = [0, 1, 3, 0, 3, 2];
        assert!(surface_errors(&alternate_diagonal, &uv, &heights, &xyz, 1., 0.25).is_none());
        let (height, surface) =
            surface_errors(&alternate_diagonal, &uv, &heights, &xyz, 1., 0.6).unwrap();
        assert_eq!(height, 0.5);
        assert_eq!(surface, 0.5);
    }
}
