//! Bounded selected-scene capture. Authored corner references are sorted for a sequential
//! POSITION join, then reassembled and Morton-sorted; no primitive-sized vertex array is held.
use super::{
    input::{ElementReader, StreamSource},
    records::{Bounds, CornerPosition, CornerRef, Cube, Provenance, TriangleRecord},
};
use crate::{
    gltf_source::source::{self, ScenePrimitive},
    spill::{ExternalSorter, Scratch, SortConfig, SortedStream},
    JobError, JobErrorKind,
};
use serde::Serialize;

type Result<T> = std::result::Result<T, JobError>;
fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidInput, message)
}
fn invariant(message: &'static str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}
fn overflow() -> JobError {
    JobError::new(
        JobErrorKind::ResourceLimit,
        "large-mesh capture count or byte sum overflow",
    )
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct SourceScan {
    pub triangles: u64,
    pub bounds: Bounds,
    /// Geometry/provenance projection only. Texture bytes and output headers are later owners.
    pub geometry_bytes: u64,
}

pub(super) fn triangle_bytes(p: &ScenePrimitive<'_>) -> u32 {
    let vertex_bytes = 12
        + 12 * u32::from(p.has_normal)
        + 16 * u32::from(p.has_tangent)
        + 8 * p.texcoord_sets as u32
        + 16 * u32::from(p.has_color);
    20 + 3 * vertex_bytes
}
pub(super) fn vertices(
    p: &ScenePrimitive<'_>,
    indices: &mut Option<ElementReader<'_>>,
    triangle: usize,
) -> Result<[u64; 3]> {
    let start = triangle.checked_mul(3).ok_or_else(overflow)?;
    let mut vertices = [0; 3];
    for (corner, vertex) in vertices.iter_mut().enumerate() {
        let index = if let Some(indices) = indices {
            let index = indices.scalar(start + corner)? as usize;
            source::check_index(
                index,
                p.position.count,
                p.indices.as_ref().unwrap().component,
            )?;
            index
        } else {
            start + corner
        };
        *vertex = u64::try_from(index).map_err(|_| overflow())?;
    }
    if p.determinant < 0. {
        vertices.swap(1, 2);
    }
    Ok(vertices)
}
pub(super) fn bake(
    p: &ScenePrimitive<'_>,
    positions: &mut ElementReader<'_>,
    vertex: u64,
) -> Result<[f32; 3]> {
    let vertex = usize::try_from(vertex).map_err(|_| overflow())?;
    let position = positions.vec3(vertex)?;
    if position.iter().any(|v| !v.is_finite()) {
        return Err(invalid("nonfinite accessor data"));
    }
    source::check_source_position(position)?;
    let mut baked = source::bake_position(p.world, position)?;
    for x in &mut baked {
        if *x == 0. {
            *x = 0.;
        }
    }
    Ok(baked)
}

/// Read-only preflight scan gives the tight box of used baked corners. It observes payload
/// bytes, so invalid referenced positions/indices may fail here before scratch creation.
pub(crate) fn scan(
    source: &StreamSource,
    buffer_bytes: usize,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<SourceScan> {
    let mut bounds = Bounds::EMPTY;
    let mut triangles = 0u64;
    let mut geometry_bytes = 0u64;
    for p in source.document.scene_primitives() {
        check()?;
        let count = u64::try_from(p.triangles).map_err(|_| overflow())?;
        triangles = triangles.checked_add(count).ok_or_else(overflow)?;
        geometry_bytes = geometry_bytes
            .checked_add(
                count
                    .checked_mul(u64::from(triangle_bytes(&p)))
                    .ok_or_else(overflow)?,
            )
            .ok_or_else(overflow)?;
        let mut positions = source.reader(&p.position, buffer_bytes);
        let mut indices = p.indices.map(|v| source.reader(&v, buffer_bytes));
        for triangle in 0..p.triangles {
            if triangle.is_multiple_of(1024) {
                check()?;
            }
            for vertex in vertices(&p, &mut indices, triangle)? {
                bounds.include(bake(&p, &mut positions, vertex)?);
            }
        }
    }
    source.verify_unchanged()?;
    Ok(SourceScan {
        triangles,
        bounds,
        geometry_bytes,
    })
}

/// Reads all source geometry before returning a sorted stream that refers only to scratch.
/// All authored triangles remain represented, including repeated welded corners.
pub(crate) fn capture<'s>(
    source: &StreamSource,
    scratch: &'s Scratch,
    sort: SortConfig,
    scan: &SourceScan,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<SortedStream<'s, TriangleRecord>> {
    source.validate_payload(sort.buffer_bytes, check)?;
    let mut corners = ExternalSorter::<CornerRef>::new(scratch, sort)?;
    for (ordinal, p) in source.document.scene_primitives().enumerate() {
        check()?;
        let ordinal = u64::try_from(ordinal).map_err(|_| overflow())?;
        let mut indices = p.indices.map(|v| source.reader(&v, sort.buffer_bytes));
        for triangle in 0..p.triangles {
            if triangle.is_multiple_of(1024) {
                check()?;
            }
            for (corner, vertex) in vertices(&p, &mut indices, triangle)?
                .into_iter()
                .enumerate()
            {
                corners.push(CornerRef {
                    primitive: ordinal,
                    vertex,
                    triangle: u64::try_from(triangle).map_err(|_| overflow())?,
                    corner: corner as u8,
                })?;
            }
        }
    }
    let mut corners = corners.finish()?;
    let mut next = corners.next().transpose()?;
    let mut positioned = ExternalSorter::<CornerPosition>::new(scratch, sort)?;
    for (ordinal, p) in source.document.scene_primitives().enumerate() {
        let ordinal = u64::try_from(ordinal).map_err(|_| overflow())?;
        let mut positions = source.reader(&p.position, sort.buffer_bytes);
        let mut cached = None;
        let mut records = 0u64;
        while let Some(corner) = next.filter(|c| c.primitive == ordinal) {
            if records.is_multiple_of(3072) {
                check()?;
            }
            let position =
                if let Some((vertex, position)) = cached.filter(|(v, _)| *v == corner.vertex) {
                    let _ = vertex;
                    position
                } else {
                    let position = bake(&p, &mut positions, corner.vertex)?;
                    cached = Some((corner.vertex, position));
                    position
                };
            positioned.push(CornerPosition {
                primitive: corner.primitive,
                triangle: corner.triangle,
                corner: corner.corner,
                vertex: corner.vertex,
                position,
            })?;
            records += 1;
            next = corners.next().transpose()?;
        }
        if records != (p.triangles as u64).checked_mul(3).ok_or_else(overflow)? {
            return Err(invariant(
                "corner reference stream disagrees with admitted primitive",
            ));
        }
    }
    if next.is_some() {
        return Err(invariant("unmatched corner reference after POSITION join"));
    }
    drop(corners);
    let mut positioned = positioned.finish()?;
    let mut triangles = ExternalSorter::<TriangleRecord>::new(scratch, sort)?;
    let cube = Cube::around(&scan.bounds);
    let mut count = 0u64;
    let mut bounds = Bounds::EMPTY;
    let mut geometry_bytes = 0u64;
    for (ordinal, p) in source.document.scene_primitives().enumerate() {
        let ordinal = u64::try_from(ordinal).map_err(|_| overflow())?;
        let bytes = triangle_bytes(&p);
        for triangle in 0..p.triangles {
            if triangle.is_multiple_of(1024) {
                check()?;
            }
            let mut positions = [[0.; 3]; 3];
            let mut vertices = [0; 3];
            for corner in 0..3 {
                let value = positioned
                    .next()
                    .transpose()?
                    .ok_or_else(|| invariant("missing positioned corner"))?;
                if (value.primitive, value.triangle, value.corner)
                    != (ordinal, triangle as u64, corner as u8)
                {
                    return Err(invariant(
                        "positioned corner stream order disagrees with provenance",
                    ));
                }
                positions[corner] = value.position;
                vertices[corner] = value.vertex;
                bounds.include(value.position);
            }
            let degenerate = positions[0] == positions[1]
                || positions[0] == positions[2]
                || positions[1] == positions[2];
            triangles.push(TriangleRecord {
                key: cube.key(&positions),
                source: Provenance {
                    node: p.node,
                    mesh: p.mesh,
                    primitive: p.primitive,
                    triangle: triangle as u64,
                },
                vertices,
                positions,
                bytes,
                degenerate,
            })?;
            count = count.checked_add(1).ok_or_else(overflow)?;
            geometry_bytes = geometry_bytes
                .checked_add(u64::from(bytes))
                .ok_or_else(overflow)?;
        }
    }
    if positioned.next().transpose()?.is_some() {
        return Err(invariant("extra positioned corner"));
    }
    drop(positioned);
    source.verify_unchanged()?;
    if count != scan.triangles || geometry_bytes != scan.geometry_bytes || bounds != scan.bounds {
        return Err(invalid(
            "mesh source geometry changed between preflight and capture",
        ));
    }
    Ok(triangles.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spill::Scratch;
    use serde_json::json;
    use std::fs;

    #[test]
    fn reflected_instances_keep_authored_identity_after_source_removal() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("scene.gltf");
        let doc = json!({
            "asset":{"version":"2.0"},"buffers":[{"uri":"data.bin","byteLength":42}],
            "bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":6}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
                {"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
            "nodes":[{"mesh":0},{"mesh":0,"scale":[-1,1,1]}],"scenes":[{"nodes":[0,1]}]
        });
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let mut bytes: Vec<_> = [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]]
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        bytes.extend([2u16, 0, 1].iter().flat_map(|v| v.to_le_bytes()));
        fs::write(dir.path().join("data.bin"), bytes).unwrap();
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let scan = scan(&source, 16, &mut || Ok(())).unwrap();
        assert_eq!(scan.triangles, 2);
        assert_eq!(scan.geometry_bytes, 112);
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let sort = SortConfig {
            run_bytes: 128,
            fan_in: 2,
            buffer_bytes: 16,
            jobs: 1,
        };
        let captured = capture(&source, &scratch, sort, &scan, &mut || Ok(())).unwrap();
        fs::remove_file(dir.path().join("data.bin")).unwrap();
        fs::remove_file(&input).unwrap();
        let rows: Vec<_> = captured.collect::<std::result::Result<_, _>>().unwrap();
        assert_eq!(rows.len(), 2);
        let reflected = rows.iter().find(|row| row.source.node == 1).unwrap();
        assert_eq!(reflected.vertices, [2, 1, 0]);
        assert_eq!(
            reflected.positions,
            [[0., 1., 0.], [-1., 0., 0.], [0., 0., 0.]]
        );
        assert_eq!(
            reflected.source,
            Provenance {
                node: 1,
                mesh: 0,
                primitive: 0,
                triangle: 0
            }
        );
        assert!(rows.iter().all(|row| row
            .positions
            .iter()
            .flatten()
            .all(|v| *v != 0. || v.to_bits() == 0)));

        // Exercise the orchestration boundary too: once the tree's triangle sequence exists,
        // capture has completed and callbacks may remove all input files.
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let mut bytes: Vec<_> = [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]]
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        bytes.extend([2u16, 0, 1].iter().flat_map(|v| v.to_le_bytes()));
        fs::write(dir.path().join("data.bin"), bytes).unwrap();
        let parent = dir.path().join("tree-scratch");
        fs::create_dir(&parent).unwrap();
        let mut config =
            super::super::config::TreeConfig::new(super::super::config::Profile::Balanced);
        config.jobs = super::super::config::Jobs::Fixed(std::num::NonZeroUsize::new(1).unwrap());
        config.scratch_dir = Some(parent.clone());
        config.sort = sort;
        config.sort.run_bytes = 256;
        let mut removed = false;
        let output = super::super::build(&input, &config, &mut || {
            let tree_started = fs::read_dir(&parent).unwrap().flatten().any(|entry| {
                fs::read_dir(entry.path()).unwrap().flatten().any(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("triangles-")
                })
            });
            if tree_started && !removed {
                fs::remove_file(&input).unwrap();
                fs::remove_file(dir.path().join("data.bin")).unwrap();
                removed = true;
            }
            Ok(())
        })
        .unwrap();
        assert!(removed, "source removal control must reach the tree stage");
        assert_eq!(output.report.triangles, 2);
        output.close().unwrap();
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
    }
}
