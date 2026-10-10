//! Untrusted original-position proposals and independent complete-face bounds.
//! No format, filesystem, publication or observer ownership.
use super::source::{Geometry, SourceIdentity, Triangle};
use crate::{JobError, JobErrorKind};
use meshopt::{simplify::SimplifyOptions, VertexDataAdapter};
use std::collections::{BTreeMap, BTreeSet};

const MAX_COMPARISON_PAIRS: u64 = 16_777_216;
const DENOMINATOR: u32 = 1 << 24;
const WEIGHT_SCALE: f64 = 1.0 / DENOMINATOR as f64;
type Face = [[f32; 3]; 3];
type Result<T> = std::result::Result<T, JobError>;

pub(super) struct ProxyTriangle {
    pub positions: Face,
    pub region: usize,
}
pub(super) struct ProxyRegion {
    pub members: Vec<SourceIdentity>,
    pub material: Option<usize>,
}
pub(super) struct ProxyGeometry {
    pub triangles: Vec<ProxyTriangle>,
    pub regions: Vec<ProxyRegion>,
    pub certified_error_metres: f64,
    pub comparison_pairs: u64,
}

fn unsupported(message: &str) -> JobError {
    JobError::new(JobErrorKind::Unsupported, message)
}
fn invalid(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}

pub(super) fn prepare(
    geometry: &Geometry,
    triangle_limit: usize,
    max_error_metres: f64,
    mut check: impl FnMut() -> Result<()>,
) -> Result<ProxyGeometry> {
    check()?;
    // Request policy was resolved at the core boundary.
    debug_assert!(triangle_limit > 0 && max_error_metres.is_finite() && max_error_metres > 0.);
    if geometry.triangles.is_empty() {
        return Err(invalid("prepared root proxy has empty source support"));
    }
    let mut groups = BTreeMap::new();
    for (index, triangle) in geometry.triangles.iter().enumerate() {
        if index.is_multiple_of(1024) {
            check()?;
        }
        if triangle.normals.is_some()
            || triangle.tangents.is_some()
            || triangle.texcoords.iter().any(Option::is_some)
            || triangle.colors.is_some()
        {
            return Err(unsupported(
                "root proxy requires selected positions-only geometry",
            ));
        }
        if triangle
            .positions
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
        {
            return Err(invalid(
                "prepared proxy position is outside the admitted finite domain",
            ));
        }
        groups
            .entry(triangle.source.primitive())
            .or_insert_with(Vec::new)
            .push(triangle);
    }

    let mut regions = Vec::with_capacity(groups.len());
    let mut components = Vec::new();
    let mut original_faces = Vec::with_capacity(groups.len());
    let mut checked_materials = BTreeSet::new();
    for triangles in groups.into_values() {
        check()?;
        let region = regions.len();
        let material = triangles[0].material;
        if triangles.iter().any(|t| t.material != material) {
            return Err(invalid(
                "prepared primitive has inconsistent material identity",
            ));
        }
        if let Some(material) = material.filter(|id| checked_materials.insert(*id)) {
            let material = geometry
                .materials
                .get(material)
                .ok_or_else(|| invalid("prepared proxy material index is out of range"))?;
            if !material.proxy_eligible() {
                return Err(unsupported(
                    "root proxy requires untextured opaque core PBR materials",
                ));
            }
        }
        let mut members: Vec<_> = triangles.iter().map(|t| t.source).collect();
        members.sort_unstable();
        if members.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("prepared source triangle identity is duplicated"));
        }
        for faces in connected_components(&triangles, &mut check)? {
            components.push((region, faces));
        }
        original_faces.push(triangles.iter().map(|t| t.positions).collect::<Vec<_>>());
        regions.push(ProxyRegion { members, material });
    }
    let total = geometry.triangles.len();
    let target = triangle_limit.min(total.saturating_sub(1));
    if target < components.len() {
        return Err(unsupported(
            "root proxy limit cannot represent every authored component",
        ));
    }
    // Each component receives one face, then a deterministic proportional share
    // of the remaining slots. Cumulative division assigns every available slot.
    let capacity = total - components.len();
    let slots = target - components.len();
    let mut prefix = 0;
    let mut assigned = 0;
    let mut triangles = Vec::with_capacity(target);
    for (region, faces) in components {
        check()?;
        prefix += faces.len() - 1;
        let cumulative = if capacity == 0 {
            0
        } else {
            ((prefix as u64) * (slots as u64) / (capacity as u64)) as usize
        };
        let budget = 1 + cumulative - assigned;
        assigned = cumulative;
        let candidate = propose(&faces, budget, max_error_metres, &mut check)?;
        check()?;
        triangles.extend(
            candidate
                .into_iter()
                .map(|positions| ProxyTriangle { positions, region }),
        );
    }
    if triangles.is_empty() || triangles.len() >= total || triangles.len() > triangle_limit {
        return Err(unsupported(
            "root proxy proposal did not achieve the requested actual reduction",
        ));
    }
    let mut proxy_faces = vec![Vec::new(); regions.len()];
    for triangle in &triangles {
        proxy_faces[triangle.region].push(triangle.positions);
    }
    let comparison_pairs = admit_pairs(&original_faces, &proxy_faces)?;
    let mut performed = 0;
    let mut squared = 0.0_f64;
    for (original, proxy) in original_faces.iter().zip(&proxy_faces) {
        squared = squared.max(directed(original, proxy, &mut performed, &mut check)?);
        squared = squared.max(directed(proxy, original, &mut performed, &mut check)?);
    }
    check()?;
    let certified_error_metres = if squared == 0. {
        0.
    } else {
        squared.sqrt().next_up()
    };
    if !certified_error_metres.is_finite() {
        return Err(invalid("proxy certificate is nonfinite"));
    }
    if certified_error_metres > max_error_metres {
        return Err(unsupported(
            "root proxy whole-face certificate exceeds the requested maximum error",
        ));
    }
    debug_assert_eq!(performed, comparison_pairs);
    Ok(ProxyGeometry {
        triangles,
        regions,
        certified_error_metres,
        comparison_pairs,
    })
}

/// Connectivity uses authored keys, never coordinate welding. A shared vertex
/// connects faces, including degenerate faces; coincident disjoint keys do not.
fn connected_components<'a>(
    triangles: &[&'a Triangle],
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<Vec<&'a Triangle>>> {
    let mut parents: Vec<_> = (0..triangles.len()).collect();
    let mut vertices = BTreeMap::new();
    for (index, triangle) in triangles.iter().enumerate() {
        if index.is_multiple_of(1024) {
            check()?;
        }
        for (key, position) in triangle.vertex_indices.into_iter().zip(triangle.positions) {
            if let Some(&(previous, previous_position)) = vertices.get(&key) {
                if position != previous_position {
                    return Err(invalid(
                        "authored vertex key has inconsistent decoded positions",
                    ));
                }
                let a = representative(&mut parents, index);
                let b = representative(&mut parents, previous);
                parents[a.max(b)] = a.min(b);
            } else {
                vertices.insert(key, (index, position));
            }
        }
    }
    let mut components = BTreeMap::new();
    for (index, &triangle) in triangles.iter().enumerate() {
        if index.is_multiple_of(1024) {
            check()?;
        }
        components
            .entry(representative(&mut parents, index))
            .or_insert_with(Vec::new)
            .push(triangle);
    }
    Ok(components.into_values().collect())
}
fn representative(parents: &mut [usize], mut index: usize) -> usize {
    while parents[index] != index {
        parents[index] = parents[parents[index]];
        index = parents[index];
    }
    index
}

fn propose(
    faces: &[&Triangle],
    budget: usize,
    max_error_metres: f64,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<Face>> {
    if budget >= faces.len() {
        return Ok(faces.iter().map(|t| t.positions).collect());
    }
    let mut keys = BTreeMap::new();
    let mut positions = Vec::new();
    let mut indices = Vec::with_capacity(faces.len() * 3);
    for (index, triangle) in faces.iter().enumerate() {
        if index.is_multiple_of(1024) {
            check()?;
        }
        for (key, position) in triangle.vertex_indices.into_iter().zip(triangle.positions) {
            let next = positions.len() as u32;
            let id = *keys.entry(key).or_insert_with(|| {
                positions.push(position);
                next
            });
            indices.push(id);
        }
    }
    let bytes: &[u8] = bytemuck::cast_slice(&positions);
    let adapter = VertexDataAdapter::new(bytes, 12, 0)
        .map_err(|_| invalid("could not construct original-position proposal input"))?;
    // This tolerance only guides the proposal. No optimizer score is read.
    // The admitted local domain has diameter < 4e6 metres.
    let proposed = meshopt::simplify::simplify(
        &indices,
        &adapter,
        budget * 3,
        max_error_metres.min(4_000_000.) as f32,
        SimplifyOptions::Permissive | SimplifyOptions::ErrorAbsolute,
        None,
    );
    if proposed.is_empty() {
        return Err(unsupported(
            "root proxy proposal omitted an authored component",
        ));
    }
    if !proposed.len().is_multiple_of(3) {
        return Err(invalid("optimizer proposed incomplete triangle indices"));
    }
    let mut result = Vec::with_capacity(proposed.len() / 3);
    for face in proposed.as_chunks::<3>().0 {
        let mut triangle = [[0.; 3]; 3];
        for (corner, &index) in face.iter().enumerate() {
            triangle[corner] = *positions
                .get(index as usize)
                .ok_or_else(|| invalid("optimizer proposed an invalid original-position index"))?;
        }
        result.push(triangle);
    }
    Ok(result)
}

fn admit_pairs(original: &[Vec<Face>], proxy: &[Vec<Face>]) -> Result<u64> {
    if original.len() != proxy.len() {
        return Err(invalid("certificate region count is inconsistent"));
    }
    let mut pairs = 0_u64;
    for (a, b) in original.iter().zip(proxy) {
        if a.is_empty() || b.is_empty() {
            return Err(invalid("certificate has empty region support"));
        }
        pairs = (a.len() as u64)
            .checked_mul(b.len() as u64)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| pairs.checked_add(n))
            .filter(|&n| n <= MAX_COMPARISON_PAIRS)
            .ok_or_else(|| {
                unsupported("root proxy certificate exceeds 16777216 complete face pairs")
            })?;
    }
    Ok(pairs)
}

fn directed(
    source: &[Face],
    target: &[Face],
    performed: &mut u64,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<f64> {
    if source.is_empty() || target.is_empty() {
        return Err(invalid("empty directed certificate support"));
    }
    let mut bound = 0.0_f64;
    for s in source {
        let mut best = f64::INFINITY;
        for t in target {
            if performed.is_multiple_of(64) {
                check()?;
            }
            *performed += 1;
            best = best.min(face_squared_bound(s, t)?);
        }
        bound = bound.max(best);
    }
    Ok(bound)
}

fn face_squared_bound(source: &Face, target: &Face) -> Result<f64> {
    // Equality of corner sets establishes equal convex hulls, including points
    // and segments; winding and repeated-corner multiplicity do not affect it.
    if source.iter().all(|p| target.contains(p)) && target.iter().all(|p| source.contains(p)) {
        return Ok(0.);
    }
    let mut bound = 0.0_f64;
    for &point in source {
        bound = bound.max(point_squared_bound(point, target)?);
    }
    Ok(bound)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

/// Projection is only a witness proposal. Integer membership and interval
/// reconstruction establish validity even if the projection itself is wrong.
fn dyadic(u: f64, v: f64) -> Option<[u32; 3]> {
    if !u.is_finite() || !v.is_finite() {
        return None;
    }
    let u = (u.clamp(0., 1.) * f64::from(DENOMINATOR)).round() as u32;
    let v = ((v.clamp(0., 1.) * f64::from(DENOMINATOR)).round() as u32).min(DENOMINATOR - u);
    Some([DENOMINATOR - u - v, u, v])
}

fn point_squared_bound(point: [f32; 3], target: &Face) -> Result<f64> {
    if target.contains(&point) {
        return Ok(0.);
    }
    let p = point.map(f64::from);
    let [a, b, c] = target.map(|v| v.map(f64::from));
    let mut best = f64::INFINITY;
    // These explicit vertices always provide finite candidates, including for
    // a fully degenerate target. No unproved distance-query result is used.
    for weights in [
        [DENOMINATOR, 0, 0],
        [0, DENOMINATOR, 0],
        [0, 0, DENOMINATOR],
    ] {
        best = best.min(witness_squared_bound(p, target, weights)?);
    }
    for (start, end) in [(0, 1), (1, 2), (2, 0)] {
        let vertices = [a, b, c];
        let edge = sub(vertices[end], vertices[start]);
        let length_squared = dot(edge, edge);
        if length_squared > 0. {
            let t = dot(sub(p, vertices[start]), edge) / length_squared;
            if t.is_finite() {
                let amount = (t.clamp(0., 1.) * f64::from(DENOMINATOR)).round() as u32;
                let mut weights = [0; 3];
                weights[start] = DENOMINATOR - amount;
                weights[end] = amount;
                best = best.min(witness_squared_bound(p, target, weights)?);
            }
        }
    }
    let e = sub(b, a);
    let f = sub(c, a);
    let r = sub(p, a);
    let (ee, ef, ff) = (dot(e, e), dot(e, f), dot(f, f));
    let determinant = ee * ff - ef * ef;
    if determinant > 0. {
        let (re, rf) = (dot(r, e), dot(r, f));
        if let Some(weights) = dyadic(
            (re * ff - rf * ef) / determinant,
            (rf * ee - re * ef) / determinant,
        ) {
            best = best.min(witness_squared_bound(p, target, weights)?);
        }
    }
    if !best.is_finite() {
        return Err(invalid("no finite explicit certificate witness"));
    }
    Ok(best)
}

fn witness_squared_bound(point: [f64; 3], target: &Face, weights: [u32; 3]) -> Result<f64> {
    if weights.iter().map(|&v| u64::from(v)).sum::<u64>() != u64::from(DENOMINATOR) {
        return Err(invalid("certificate witness is outside the target simplex"));
    }
    let mut sum = 0.0_f64;
    for (axis, coordinate) in point.into_iter().enumerate() {
        let mut low = 0.0;
        let mut high = 0.0;
        for (vertex, weight) in target.iter().zip(weights) {
            if weight == 0 {
                continue;
            }
            let product = f64::from(vertex[axis]) * (f64::from(weight) * WEIGHT_SCALE);
            low = (low + product.next_down()).next_down();
            high = (high + product.next_up()).next_up();
        }
        let residual = (coordinate - high)
            .next_down()
            .abs()
            .max((coordinate - low).next_up().abs());
        let squared = (residual * residual).next_up();
        sum = (sum + squared).next_up();
    }
    if !sum.is_finite() {
        return Err(invalid("certificate interval arithmetic is nonfinite"));
    }
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn triangle(index: u32, keys: [u32; 3], positions: Face) -> Triangle {
        Triangle {
            source: SourceIdentity {
                triangle_index: index,
                ..Default::default()
            },
            vertex_indices: keys,
            positions,
            normals: None,
            tangents: None,
            texcoords: [None; 2],
            colors: None,
            material: None,
        }
    }
    fn bound(a: &[Face], b: &[Face]) -> f64 {
        let mut work = 0;
        directed(a, b, &mut work, &mut || Ok(()))
            .unwrap()
            .sqrt()
            .max(directed(b, a, &mut work, &mut || Ok(())).unwrap().sqrt())
    }

    #[test]
    fn face_certificate_covers_spikes_interiors_and_degenerate_targets() {
        let flat = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let spike = [[0., 0., 0.], [1., 0., 100.], [0., 1., 100.]];
        let mut faces = vec![flat; 3000];
        faces[1] = spike;
        let first = bound(&faces, &[flat]);
        faces.reverse();
        assert_eq!(first, bound(&faces, &[flat]));
        assert!((100. ..100.00001).contains(&first));
        let large = [[0., 0., 0.], [8., 0., 0.], [0., 8., 0.]];
        let corner_faces = [
            flat,
            [[8., 0., 0.], [7., 0., 0.], [7., 1., 0.]],
            [[0., 8., 0.], [0., 7., 0.], [1., 7., 0.]],
        ];
        assert!(bound(&[large], &corner_faces) >= 7.);
        assert!(bound(&[flat], &[[[0.; 3]; 3]]) >= 1.);
    }

    #[test]
    fn identical_support_and_planar_looseness_are_explicit() {
        let face = [[0., 0., 0.], [8., 0., 0.], [0., 8., 0.]];
        let children = [
            [[0., 0., 0.], [4., 0., 0.], [0., 4., 0.]],
            [[4., 0., 0.], [8., 0., 0.], [4., 4., 0.]],
            [[0., 4., 0.], [4., 4., 0.], [0., 8., 0.]],
            [[4., 0., 0.], [4., 4., 0.], [0., 4., 0.]],
        ];
        assert_eq!(bound(&[face], &[[face[2], face[1], face[0]]]), 0.);
        let error = bound(&[face], &children);
        assert!((4. ..4.00001).contains(&error));
        let mixed = [[-f32::from_bits(1), 0., 0.], [1e6, 0., 0.], [1., 0., 0.]];
        assert_eq!(bound(&[mixed], &[mixed]), 0.);
        assert!(bound(&[mixed], &[[[0.; 3]; 3]]) >= 1e6);
    }

    #[test]
    fn authored_keys_separate_coincident_components_and_preserve_points() {
        let face = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let a = triangle(0, [0, 1, 2], face);
        let b = triangle(1, [3, 4, 5], face);
        let c = triangle(2, [2, 6, 7], [face[2], [0., 2., 0.], [1., 2., 0.]]);
        let d = triangle(3, [8, 8, 8], [[0.; 3]; 3]);
        let components = connected_components(&[&a, &b, &c, &d], &mut || Ok(())).unwrap();
        assert_eq!(
            components.iter().map(Vec::len).collect::<Vec<_>>(),
            [2, 1, 1]
        );
        let bad = triangle(4, [0, 10, 11], [[9.; 3]; 3]);
        assert_eq!(
            connected_components(&[&a, &bad], &mut || Ok(()))
                .err()
                .unwrap()
                .kind(),
            JobErrorKind::InvalidState
        );
    }

    #[test]
    fn dyadic_membership_and_invalid_support_fail_closed() {
        for (u, v) in [(-3., 4.), (1., 1.), (0.333333333, 0.777777777), (0., 0.)] {
            let weights = dyadic(u, v).unwrap();
            assert_eq!(weights.into_iter().sum::<u32>(), DENOMINATOR);
        }
        assert!(dyadic(f64::NAN, 0.).is_none());
        assert!(directed(&[], &[[[0.; 3]; 3]], &mut 0, &mut || Ok(())).is_err());
        assert!(witness_squared_bound([0.; 3], &[[0.; 3]; 3], [1, 2, 3]).is_err());
        let a = vec![vec![[[0.; 3]; 3]; 4096]];
        let b = vec![vec![[[0.; 3]; 3]; 2048]];
        assert_eq!(admit_pairs(&a, &b).unwrap(), MAX_COMPARISON_PAIRS);
        let b = vec![vec![[[0.; 3]; 3]; 2049]];
        assert_eq!(
            admit_pairs(&a, &b).unwrap_err().kind(),
            JobErrorKind::Unsupported
        );
    }

    #[test]
    fn complete_comparison_honors_bounded_checkpoints() {
        let face = [[0.; 3]; 3];
        let mut calls = 0;
        let error = directed(&[face; 100], &[face], &mut 0, &mut || {
            calls += 1;
            if calls == 2 {
                Err(invalid("injected checkpoint"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::InvalidState);
        assert_eq!(calls, 2);
    }

    fn grid() -> Geometry {
        let mut triangles = Vec::new();
        for y in 0..4_u32 {
            for x in 0..4_u32 {
                let keys = [
                    y * 5 + x,
                    y * 5 + x + 1,
                    (y + 1) * 5 + x,
                    (y + 1) * 5 + x + 1,
                ];
                let positions = [
                    [x as f32, y as f32, 0.],
                    [x as f32 + 1., y as f32, 0.],
                    [x as f32, y as f32 + 1., 0.],
                    [x as f32 + 1., y as f32 + 1., 0.],
                ];
                for corners in [[0, 1, 2], [1, 3, 2]] {
                    triangles.push(triangle(
                        triangles.len() as u32,
                        corners.map(|c| keys[c]),
                        corners.map(|c| positions[c]),
                    ));
                }
            }
        }
        Geometry {
            node_names: vec![None],
            triangles,
            materials: Vec::new(),
            images: Vec::new(),
            textures: Vec::new(),
            samplers: Vec::new(),
        }
    }

    #[test]
    fn real_proposal_reduces_grid_and_preserves_complete_region_membership() {
        let geometry = grid();
        let proxy = prepare(&geometry, 8, 3., || Ok(())).unwrap();
        assert!(!proxy.triangles.is_empty() && proxy.triangles.len() <= 8);
        assert!(proxy.certified_error_metres <= 3.);
        assert_eq!(proxy.regions.len(), 1);
        assert_eq!(
            proxy.regions[0].members,
            geometry
                .triangles
                .iter()
                .map(|t| t.source)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            proxy.comparison_pairs,
            2 * 32 * proxy.triangles.len() as u64
        );
        assert!(proxy
            .triangles
            .iter()
            .flat_map(|t| t.positions)
            .all(|p| geometry.triangles.iter().any(|t| t.positions.contains(&p))));
    }

    #[test]
    fn actual_proposal_retains_separate_component_and_respects_profile_refusal() {
        let mut geometry = grid();
        geometry.triangles.push(triangle(
            32,
            [100, 101, 102],
            [[100., 0., 0.], [101., 0., 0.], [100., 1., 0.]],
        ));
        let proxy = prepare(&geometry, 9, 3., || Ok(())).unwrap();
        assert!(proxy
            .triangles
            .iter()
            .any(|t| t.positions.iter().all(|p| p[0] >= 100.)));
        assert!(proxy.triangles.iter().all(|t| {
            t.positions.iter().all(|p| p[0] >= 100.) || t.positions.iter().all(|p| p[0] <= 4.)
        }));
        assert_eq!(
            prepare(&geometry, 1, 3., || Ok(())).err().unwrap().kind(),
            JobErrorKind::Unsupported
        );
        geometry.triangles[0].normals = Some([[0., 0., 1.]; 3]);
        assert_eq!(
            prepare(&geometry, 9, 3., || Ok(())).err().unwrap().kind(),
            JobErrorKind::Unsupported
        );
    }
}
