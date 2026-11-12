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
#[cfg_attr(not(test), allow(dead_code))] // Retained for bounded unit-test fixtures; production uses the streamed facade.
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
/// A referenced source position after its accumulated node transform, in f64.
pub(super) fn source_position(
    p: &ScenePrimitive<'_>,
    positions: &mut ElementReader<'_>,
    vertex: u64,
) -> Result<[f64; 3]> {
    let vertex = usize::try_from(vertex).map_err(|_| overflow())?;
    let position = positions.vec3(vertex)?;
    if position.iter().any(|v| !v.is_finite()) {
        return Err(invalid("nonfinite accessor data"));
    }
    source::check_source_position(position)?;
    source::world_position(p.world, position)
}
/// Positive zero for every zero component, so equal positions have equal bits.
pub(super) fn canonical(mut position: [f32; 3]) -> [f32; 3] {
    for x in &mut position {
        if *x == 0. {
            *x = 0.;
        }
    }
    position
}
#[cfg_attr(not(test), allow(dead_code))] // Retained for bounded unit-test fixtures; production uses the streamed facade.
pub(super) fn bake(
    p: &ScenePrimitive<'_>,
    positions: &mut ElementReader<'_>,
    vertex: u64,
) -> Result<[f32; 3]> {
    Ok(canonical(
        source_position(p, positions, vertex)?.map(|x| x as f32),
    ))
}

#[cfg_attr(not(test), allow(dead_code))] // Retained for bounded unit-test fixtures; production uses the streamed facade.
/// Read-only preflight scan gives the tight box of used baked corners. It observes payload
/// bytes, so invalid referenced positions/indices may fail here before scratch creation.
pub(crate) fn scan(
    source: &StreamSource,
    buffer_bytes: usize,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<SourceScan> {
    Ok(scan_with_source_bounds(source, buffer_bytes, 1, &mut |_| check())?.0)
}

/// Triangles one scan task reads.
const SCAN_TASK_TRIANGLES: usize = 1 << 16;
/// The scan reads POSITION in index order, which jumps around the accessor: each refill
/// re-reads its whole window, so a small one copies far fewer bytes for a few more reads
/// (on the 10M-triangle Walem scene, 64 KiB instead of 1 MiB reads about 7× fewer bytes
/// for twice the refills). Indices are read in order and keep the full stream buffer.
const SCAN_POSITION_WINDOW: usize = 64 << 10;
/// How often the scanning thread reports progress while its tasks run.
const SCAN_POLL: std::time::Duration = std::time::Duration::from_millis(5);

/// One task's box: the f64 post-node source bounds and the baked f32 bounds.
struct TaskScan {
    source_bounds: [[f64; 3]; 2],
    bounds: Bounds,
}

fn include_source(bounds: &mut [[f64; 3]; 2], world: [f64; 3]) {
    for axis in 0..3 {
        bounds[0][axis] = bounds[0][axis].min(world[axis]);
        bounds[1][axis] = bounds[1][axis].max(world[axis]);
    }
}

/// Scans `triangles` of `p` with its own bounded readers, adding to `done` as it goes and
/// giving up (with any error; the caller reports its own) once `stop` is set.
fn scan_task(
    source: &StreamSource,
    p: &ScenePrimitive<'_>,
    triangles: std::ops::Range<usize>,
    buffer_bytes: usize,
    done: &std::sync::atomic::AtomicU64,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<TaskScan> {
    use std::sync::atomic::Ordering::Relaxed;
    let mut out = TaskScan {
        source_bounds: [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]],
        bounds: Bounds::EMPTY,
    };
    let mut positions = source.reader(&p.position, buffer_bytes.min(SCAN_POSITION_WINDOW));
    let mut indices = p.indices.map(|v| source.reader(&v, buffer_bytes));
    let mut counted = triangles.start;
    for triangle in triangles.clone() {
        if triangle.is_multiple_of(1024) {
            if stop.load(Relaxed) {
                return Err(JobError::new(JobErrorKind::Cancelled, "scan stopped"));
            }
            done.fetch_add((triangle - counted) as u64, Relaxed);
            counted = triangle;
        }
        for vertex in vertices(p, &mut indices, triangle)? {
            let world = source_position(p, &mut positions, vertex)?;
            include_source(&mut out.source_bounds, world);
            out.bounds.include(canonical(world.map(|x| x as f32)));
        }
    }
    done.fetch_add((triangles.end - counted) as u64, Relaxed);
    Ok(out)
}

/// [`scan`] plus the f64 bounds `[min, max]` of the used post-node corners, from which a
/// source CRS chooses its origin. With a source CRS the f32 box is a local-frame placeholder
/// that capture replaces with its mapped box. `progress` is the cancellation check, given the
/// triangles scanned so far; it runs on the calling thread.
///
/// Up to `workers` threads scan consecutive triangle ranges with their own bounded readers.
/// Their boxes are combined in input order, and a failure reports the first failing range in
/// input order (its first failing triangle), so counts, boxes and verdicts are those of one
/// sequential pass for any worker count.
pub(crate) fn scan_with_source_bounds(
    source: &StreamSource,
    buffer_bytes: usize,
    workers: usize,
    progress: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<(SourceScan, [[f64; 3]; 2])> {
    use std::sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed},
        Mutex,
    };
    let primitives: Vec<_> = source.document.scene_primitives().collect();
    let mut triangles = 0u64;
    let mut geometry_bytes = 0u64;
    let mut tasks = Vec::new();
    for (index, p) in primitives.iter().enumerate() {
        let count = u64::try_from(p.triangles).map_err(|_| overflow())?;
        triangles = triangles.checked_add(count).ok_or_else(overflow)?;
        geometry_bytes = geometry_bytes
            .checked_add(
                count
                    .checked_mul(u64::from(triangle_bytes(p)))
                    .ok_or_else(overflow)?,
            )
            .ok_or_else(overflow)?;
        for start in (0..p.triangles).step_by(SCAN_TASK_TRIANGLES) {
            tasks.push((index, start..(start + SCAN_TASK_TRIANGLES).min(p.triangles)));
        }
    }
    progress(0)?;
    let results: Vec<Mutex<Option<Result<TaskScan>>>> =
        tasks.iter().map(|_| Mutex::new(None)).collect();
    let (next, done, stop) = (
        AtomicUsize::new(0),
        AtomicU64::new(0),
        AtomicBool::new(false),
    );
    // Ranges after the first failed one are not needed: an earlier failure reports first.
    let first_failed = AtomicUsize::new(usize::MAX);
    let stopped = std::thread::scope(|scope| {
        let worker = || loop {
            let task = next.fetch_add(1, Relaxed);
            if task >= tasks.len() || task > first_failed.load(Relaxed) || stop.load(Relaxed) {
                break;
            }
            let (index, range) = &tasks[task];
            let scanned = scan_task(
                source,
                &primitives[*index],
                range.clone(),
                buffer_bytes,
                &done,
                &stop,
            );
            if scanned.is_err() {
                first_failed.fetch_min(task, Relaxed);
            }
            *results[task].lock().unwrap_or_else(|e| e.into_inner()) = Some(scanned);
        };
        let mut threads = Vec::new();
        let mut stopped = None;
        for _ in 0..workers.clamp(1, tasks.len().max(1)) {
            match std::thread::Builder::new()
                .name("rusty-tiles-capture-scan".into())
                .spawn_scoped(scope, worker)
            {
                Ok(thread) => threads.push(thread),
                Err(error) => {
                    // The started workers stop at their next check and are joined below.
                    stop.store(true, Relaxed);
                    stopped = Some(JobError::new(
                        JobErrorKind::ResourceLimit,
                        format!("start capture scan worker: {error}"),
                    ));
                    break;
                }
            }
        }
        while stopped.is_none() && !threads.iter().all(|thread| thread.is_finished()) {
            if let Err(error) = progress(done.load(Relaxed)) {
                stop.store(true, Relaxed);
                stopped = Some(error);
                break;
            }
            std::thread::sleep(SCAN_POLL);
        }
        for thread in threads {
            if let Err(panic) = thread.join() {
                std::panic::resume_unwind(panic);
            }
        }
        stopped
    });
    if let Some(error) = stopped {
        return Err(error);
    }
    let mut source_bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut bounds = Bounds::EMPTY;
    for result in results {
        let task = result
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
            .ok_or_else(|| JobError::new(JobErrorKind::InvalidState, "scan task did not run"))??;
        include_source(&mut source_bounds, task.source_bounds[0]);
        include_source(&mut source_bounds, task.source_bounds[1]);
        bounds.union(&task.bounds);
    }
    progress(triangles)?;
    source.verify_unchanged()?;
    Ok((
        SourceScan {
            triangles,
            bounds,
            geometry_bytes,
        },
        source_bounds,
    ))
}

#[cfg_attr(not(test), allow(dead_code))] // Retained for bounded unit-test fixtures; production uses the streamed facade.
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
            writers: 0,
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

    /// An indexed grid of `side × side` quads plus one extra vertex `extra`; `bad` replaces
    /// triangles' first index (by triangle number).
    fn indexed_grid(
        dir: &std::path::Path,
        side: u32,
        extra: [f32; 3],
        bad: &[(usize, u32)],
    ) -> std::path::PathBuf {
        let row = side + 1;
        let mut bytes = Vec::new();
        for y in 0..row {
            for x in 0..row {
                for v in [x as f32, ((x * 3 + y) % 7) as f32, y as f32] {
                    bytes.extend(v.to_le_bytes());
                }
            }
        }
        for v in extra {
            bytes.extend(v.to_le_bytes());
        }
        let positions = bytes.len();
        let mut indices = Vec::new();
        for y in 0..side {
            for x in 0..side {
                let a = y * row + x;
                indices.extend([a, a + row, a + 1, a + 1, a + row, a + row + 1]);
            }
        }
        for &(triangle, index) in bad {
            indices[triangle * 3] = index;
        }
        bytes.extend(indices.iter().flat_map(|i| i.to_le_bytes()));
        fs::write(dir.join("grid.bin"), &bytes).unwrap();
        let doc = json!({
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
            "buffers":[{"uri":"grid.bin","byteLength":bytes.len()}],
            "bufferViews":[{"buffer":0,"byteLength":positions},
                {"buffer":0,"byteOffset":positions,"byteLength":bytes.len()-positions}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":row*row+1,"type":"VEC3",
                    "min":[0,0,0],"max":[side as f32,6,side as f32]},
                {"bufferView":1,"componentType":5125,"count":indices.len(),"type":"SCALAR"}]
        });
        let input = dir.join("grid.gltf");
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        input
    }

    #[test]
    fn parallel_scan_matches_one_sequential_pass_and_reports_the_first_failure_in_order() {
        // 2 × 300² = 180,000 triangles: three scan tasks.
        let dir = tempfile::tempdir().unwrap();
        let input = indexed_grid(dir.path(), 300, [1.0, 2.0, 3.0], &[]);
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let scan = |workers| {
            let mut seen = 0;
            let result = scan_with_source_bounds(&source, 1 << 12, workers, &mut |done| {
                assert!(done >= seen, "progress never goes back");
                seen = done;
                Ok(())
            });
            (result, seen)
        };
        let (sequential, seen) = scan(1);
        let (sequential, bounds) = sequential.unwrap();
        assert_eq!((sequential.triangles, seen), (180_000, 180_000));
        for workers in [2, 3, 8] {
            let (parallel, seen) = scan(workers);
            let (parallel, parallel_bounds) = parallel.unwrap();
            assert_eq!(seen, 180_000);
            assert_eq!(
                (parallel.triangles, parallel.geometry_bytes),
                (sequential.triangles, sequential.geometry_bytes)
            );
            assert_eq!(parallel.bounds, sequential.bounds);
            assert_eq!(
                parallel_bounds.map(|c| c.map(f64::to_bits)),
                bounds.map(|c| c.map(f64::to_bits))
            );
        }
        // A failing progress check (here the first one taken while tasks run, or the last)
        // is returned unchanged.
        let mut calls = 0;
        let error = scan_with_source_bounds(&source, 1 << 12, 4, &mut |_| {
            calls += 1;
            if calls > 1 {
                Err(JobError::new(
                    JobErrorKind::Cancelled,
                    "operation cancelled",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::Cancelled);
        drop(source);

        // Task 0 references a nonfinite vertex at its end, task 2 an index past the accessor
        // at its start, so task 2 usually fails first in time. Every worker count reports
        // task 0's failure, as one sequential pass does.
        let dir = tempfile::tempdir().unwrap();
        let extra = 301 * 301;
        let input = indexed_grid(
            dir.path(),
            300,
            [f32::NAN, 0.0, 0.0],
            &[(131_100, 1 << 30), (65_500, extra)],
        );
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let message = |workers| {
            let error =
                scan_with_source_bounds(&source, 1 << 12, workers, &mut |_| Ok(())).unwrap_err();
            (error.kind(), error.to_string())
        };
        let sequential = message(1);
        assert_eq!(sequential.0, JobErrorKind::InvalidInput);
        assert!(sequential.1.contains("nonfinite"), "{}", sequential.1);
        for workers in [2, 3, 8] {
            assert_eq!(message(workers), sequential, "{workers} workers");
        }
    }
}
