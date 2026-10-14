use super::*;
use crate::spill::{Record, Sequence};
use config::{Jobs, Profile};
use std::{
    fs,
    io::{BufWriter, Write},
    num::NonZeroUsize,
    path::PathBuf,
};

fn fixture(dir: &Path, count: usize) -> PathBuf {
    let mut bytes = Vec::new();
    for i in 0..count {
        let x = (i % 11) as f32;
        for point in [[x, 0., 0.], [x + 0.25, 1., 0.], [x, 0., 0.25]] {
            for value in point {
                bytes.extend(value.to_le_bytes());
            }
        }
    }
    fs::write(dir.join("positions.bin"), &bytes).unwrap();
    let doc = serde_json::json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
        "buffers":[{"uri":"positions.bin","byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":bytes.len()}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":count*3,"type":"VEC3",
            "min":[0.,0.,0.],"max":[((count-1).min(10)) as f32+0.25,1.,0.25]}]
    });
    let input = dir.join("scene.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    input
}

fn small_config(scratch: &Path) -> TreeConfig {
    let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(3);
    config.jobs = Jobs::Fixed(NonZeroUsize::new(1).unwrap());
    config.scratch_dir = Some(scratch.to_owned());
    config.sort.run_bytes = 512;
    config.sort.fan_in = 2;
    config.sort.buffer_bytes = 53;
    config
}

#[test]
fn cancellation_during_spill_removes_private_workspace() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("scratch");
    fs::create_dir(&parent).unwrap();
    let input = fixture(root.path(), 41);
    let mut saw_workspace = false;
    let failure = match build(&input, &small_config(&parent), &mut || {
        if fs::read_dir(&parent).unwrap().next().is_some() {
            saw_workspace = true;
            return Err(JobError::new(
                JobErrorKind::Cancelled,
                "sensitive cancellation",
            ));
        }
        Ok(())
    }) {
        Ok(_) => panic!("cancellation control did not reach its checkpoint"),
        Err(error) => error,
    };
    assert!(saw_workspace);
    assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
    assert!(failure.secondary.is_empty());
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
}

#[test]
fn exact_member_gate_and_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let input = fixture(root.path(), 41);
    let config = small_config(root.path());
    let output = build(&input, &config, &mut || Ok(())).unwrap();
    let exact = output.tree.stats.nodes + MEMBERS_BESIDES_TILES;
    output.close().unwrap();
    let output = build(
        &input,
        &config.clone().with_member_ceiling(exact),
        &mut || Ok(()),
    )
    .unwrap();
    output.close().unwrap();
    let result = build(&input, &config.with_member_ceiling(exact - 1), &mut || {
        Ok(())
    });
    let Err(failed) = result else {
        panic!("one fewer member must refuse");
    };
    assert_eq!(failed.error.kind(), JobErrorKind::Unsupported);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn auto_jobs_accounts_for_base_and_memory_backed_scratch() {
    let plan = config::plan_jobs(Jobs::Auto, 8, Some(1_000), 200, 500, 100);
    assert_eq!(plan.jobs, 3);
    assert_eq!(plan.limited_by, "memory");
    let floor = config::plan_jobs(Jobs::Auto, 8, Some(50), 200, 500, 100);
    assert_eq!(floor.jobs, 1);
    assert_eq!(floor.limited_by, "floor");
}

fn dump<T: Record>(sequence: &Sequence<T>, path: &Path) -> Result<(), JobError> {
    let file = fs::File::create(path)
        .map_err(|error| JobError::io("create H1 diagnostic", path, error))?;
    let mut writer = BufWriter::new(file);
    let mut bytes = vec![0; T::SIZE];
    for record in sequence.reader()? {
        record?.encode(&mut bytes);
        writer
            .write_all(&bytes)
            .map_err(|error| JobError::io("write H1 diagnostic", path, error))?;
    }
    writer
        .flush()
        .map_err(|error| JobError::io("flush H1 diagnostic", path, error))
}

fn env_number<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).map_or(default, |value| {
        value
            .parse()
            .ok()
            .expect("invalid diagnostic driver number")
    })
}

/// External oracle/scale driver. Runs the production private pipeline, writes only diagnostic
/// records, and stays out of ordinary CI unless explicitly selected with independent inputs.
#[test]
#[ignore = "external independent oracle supplies H1_INPUT and H1_OUTPUT"]
fn external_driver() {
    let input = PathBuf::from(std::env::var_os("H1_INPUT").expect("H1_INPUT"));
    let output_path = PathBuf::from(std::env::var_os("H1_OUTPUT").expect("H1_OUTPUT"));
    let profile = match std::env::var("H1_PROFILE").as_deref().unwrap_or("balanced") {
        "fast" => Profile::Fast,
        "balanced" => Profile::Balanced,
        "quality" => Profile::Quality,
        "lossless" => Profile::Lossless,
        _ => panic!("invalid H1_PROFILE"),
    };
    let mut config = TreeConfig::new(profile);
    config.scratch_dir = std::env::var_os("H1_SCRATCH").map(PathBuf::from);
    config.sort.run_bytes = env_number("H1_RUN_BYTES", 2 << 20);
    config.sort.fan_in = env_number("H1_FAN_IN", 8);
    config.sort.buffer_bytes = env_number("H1_BUFFER_BYTES", 4096);
    config.leaf_triangles = env_number("H1_LEAF_TRIANGLES", config.leaf_triangles);
    config.tile_bytes = env_number("H1_TILE_BYTES", config.tile_bytes);
    config.height_cap = env_number("H1_HEIGHT_CAP", config.height_cap);
    config.member_ceiling = env_number("H1_MEMBER_CEILING", config.member_ceiling);
    config.jobs = match std::env::var("H1_JOBS").as_deref().unwrap_or("auto") {
        "auto" => Jobs::Auto,
        value => Jobs::Fixed(
            NonZeroUsize::new(value.parse().expect("invalid H1_JOBS")).expect("zero H1_JOBS"),
        ),
    };
    let start = std::time::Instant::now();
    let result = build(&input, &config, &mut || Ok(()));
    let output = match result {
        Ok(output) => output,
        Err(failure) => {
            fs::create_dir_all(&output_path).unwrap();
            fs::write(output_path.join("error.json"), serde_json::to_vec_pretty(&serde_json::json!({
                "kind": format!("{:?}", failure.error.kind()), "message":failure.error.to_string(),
                "secondary":failure.secondary.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "retained_paths":failure.retained_paths,
            })).unwrap()).unwrap();
            panic!("{failure}");
        }
    };
    assert!(
        !output_path.exists(),
        "diagnostic success output must be new"
    );
    fs::create_dir_all(&output_path).unwrap();
    dump(&output.tree.triangles, &output_path.join("triangles.bin")).unwrap();
    dump(&output.tree.nodes, &output_path.join("nodes.bin")).unwrap();
    dump(
        &output.tree.weld_depths,
        &output_path.join("vertex_depth.bin"),
    )
    .unwrap();
    let info = serde_json::json!({
        "schema":1,"tree":output.tree.stats,"source_bounds":output.report.source_bounds,
        "cube":output.report.cube,"geometry_bytes":output.report.geometry_bytes,
    });
    fs::write(
        output_path.join("info.json"),
        serde_json::to_vec_pretty(&info).unwrap(),
    )
    .unwrap();
    let report = serde_json::json!({"capture":output.report,"tree":output.tree.stats,"elapsed_seconds":start.elapsed().as_secs_f64()});
    fs::write(
        output_path.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    output.close().unwrap();
}

/// Chunked sort runs change when scratch is freed, never what is published: the same source
/// tiled with spilling sorts gives byte-identical archives whether runs are one chunk each or
/// split into many small chunk files, and the same as with default (non-spilling) sorts.
#[test]
fn chunked_sort_runs_publish_identical_archives() {
    use crate::{mesh_large::TilerJobs, spill::SortConfig, RunControl};
    let work = tempfile::tempdir().unwrap();
    let input = super::full::tests::fixture(&work.path().join("src"), 2000);
    let sort = SortConfig {
        run_bytes: 64 << 10,
        fan_in: 4,
        buffer_bytes: 4096,
        jobs: 1,
    };
    let mut archives = Vec::new();
    for (tag, spill) in [
        ("default", None),
        ("unchunked", Some((sort, crate::spill::CHUNK_BYTES))),
        ("chunked", Some((sort, 4096))),
    ] {
        let scratch = work.path().join(format!("scratch-{tag}"));
        fs::create_dir(&scratch).unwrap();
        let output = work.path().join(format!("{tag}.3tz"));
        let mut request = super::TilerRequest::new(&input, &output)
            .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(2).unwrap()))
            .with_scratch_dir(&scratch);
        if let Some((sort, chunk)) = spill {
            request = request.with_test_spill(sort, chunk);
        }
        let result = super::mesh_to_tileset(request, &RunControl::default()).unwrap();
        assert_eq!(result.report.triangles, 2000);
        if tag == "chunked" {
            assert!(
                result.diagnostics.capture.run_scratch_peak_bytes > 0,
                "the sorts spilled"
            );
        }
        archives.push(fs::read(&output).unwrap());
    }
    assert_eq!(
        archives[0], archives[1],
        "spilling sorts publish the same archive"
    );
    assert_eq!(
        archives[1], archives[2],
        "chunk size publishes the same archive"
    );
}
