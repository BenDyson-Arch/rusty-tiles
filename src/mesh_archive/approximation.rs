//! Untrusted original-position proposals and independently certified complete surfaces.
//! No format, filesystem, publication or observer ownership.
use super::{
    source::{Geometry, SourceIdentity, Triangle},
    RootProxyLimits,
};

mod certificate;
use crate::{JobError, JobErrorKind};
pub(super) use certificate::Certificate;
use certificate::Face;
use meshopt::{simplify::SimplifyOptions, VertexDataAdapter};
use std::collections::{BTreeMap, BTreeSet};

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
    pub certificate: Certificate,
}

fn unsupported(message: &str) -> JobError {
    JobError::new(JobErrorKind::Unsupported, message)
}
fn invalid(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}

pub(super) fn prepare(
    geometry: &Geometry,
    limits: RootProxyLimits,
    mut check: impl FnMut() -> Result<()>,
) -> Result<ProxyGeometry> {
    check()?;
    let triangle_limit = limits.triangle_limit();
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
        let candidate = propose(&faces, budget, limits, &mut check)?;
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
    let certificate = certificate::certify(
        &original_faces,
        &proxy_faces,
        limits.max_error_metres,
        check,
    )?;
    Ok(ProxyGeometry {
        triangles,
        regions,
        certificate,
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
    limits: RootProxyLimits,
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
        limits.max_error_metres.min(4_000_000.) as f32,
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
        let proxy = prepare(&geometry, RootProxyLimits::new(8, 3.).unwrap(), || Ok(())).unwrap();
        assert!(!proxy.triangles.is_empty() && proxy.triangles.len() <= 8);
        assert!(proxy.certificate.error_metres <= 3.);
        assert_eq!(proxy.regions.len(), 1);
        assert_eq!(
            proxy.regions[0].members,
            geometry
                .triangles
                .iter()
                .map(|t| t.source)
                .collect::<Vec<_>>()
        );
        assert!(proxy.certificate.patch_face_tests >= 2 * 32 * proxy.triangles.len() as u64);
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
        let proxy = prepare(&geometry, RootProxyLimits::new(9, 3.).unwrap(), || Ok(())).unwrap();
        assert!(proxy
            .triangles
            .iter()
            .any(|t| t.positions.iter().all(|p| p[0] >= 100.)));
        assert!(proxy.triangles.iter().all(|t| {
            t.positions.iter().all(|p| p[0] >= 100.) || t.positions.iter().all(|p| p[0] <= 4.)
        }));
        assert_eq!(
            prepare(&geometry, RootProxyLimits::new(1, 3.).unwrap(), || Ok(()))
                .err()
                .unwrap()
                .kind(),
            JobErrorKind::Unsupported
        );
        geometry.triangles[0].normals = Some([[0., 0., 1.]; 3]);
        assert_eq!(
            prepare(&geometry, RootProxyLimits::new(9, 3.).unwrap(), || Ok(()))
                .err()
                .unwrap()
                .kind(),
            JobErrorKind::Unsupported
        );
    }
}
