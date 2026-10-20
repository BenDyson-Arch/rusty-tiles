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

/// A non-indexed source with varied unit NORMALs (both hemispheres) through the public
/// facade: every profile publishes an archive the validator accepts; `lossless` leaves keep
/// the f32 source bits and `balanced` leaves decode (meshopt OCTAHEDRAL filter) to within the
/// 12-bit octahedral angle of the source normal named by each corner's provenance row.
#[test]
fn leaf_normals_follow_the_profile_and_provenance_tables_decode() {
    use crate::content_integrity::meshopt::{check_header, decode_filtered, Filter, Mode};
    use serde_json::Value;
    use std::io::Read;
    let work = tempfile::tempdir().unwrap();
    let triangles = 240usize;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for t in 0..triangles {
        let (x, y) = ((t % 16) as f32, (t / 16) as f32);
        for (k, p) in [[x, 0., y], [x + 1., 0., y], [x, 0.5, y + 1.]]
            .iter()
            .enumerate()
        {
            positions.extend_from_slice(p);
            let s = (3 * t + k) as f32;
            let n = [s.sin(), (1.7 * s).cos(), (0.3 * s).sin() - 0.2];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            normals.extend(n.map(|v| v / l));
        }
    }
    let bytes: Vec<u8> = positions
        .iter()
        .chain(&normals)
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let half = positions.len() * 4;
    fs::write(work.path().join("data.bin"), &bytes).unwrap();
    let doc = serde_json::json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1}}]}],
        "buffers":[{"uri":"data.bin","byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":half},{"buffer":0,"byteOffset":half,"byteLength":half}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":triangles*3,"type":"VEC3",
            "min":[0.,0.,0.],"max":(0..3).map(|a| positions.iter().skip(a).step_by(3).fold(0f32, |m, v| m.max(*v))).collect::<Vec<_>>()},
            {"bufferView":1,"componentType":5126,"count":triangles*3,"type":"VEC3"}]
    });
    let input = work.path().join("scene.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let view = |doc: &Value, bin: &[u8], index: usize| -> Vec<u8> {
        let v = &doc["bufferViews"][index];
        let at = |v: &Value, key: &str| v[key].as_u64().unwrap_or(0) as usize;
        match v.get("extensions").map(|e| &e["EXT_meshopt_compression"]) {
            Some(e) => {
                let stream = &bin[at(e, "byteOffset")..at(e, "byteOffset") + at(e, "byteLength")];
                check_header(stream, Mode::Attributes).unwrap();
                let filter = Filter::parse(e["filter"].as_str().unwrap()).unwrap();
                let out = decode_filtered(
                    stream,
                    at(e, "count"),
                    at(e, "byteStride"),
                    Mode::Attributes,
                    filter,
                )
                .unwrap();
                assert_eq!(out.len(), at(v, "byteLength"));
                out
            }
            None => bin[at(v, "byteOffset")..at(v, "byteOffset") + at(v, "byteLength")].to_vec(),
        }
    };
    for profile in [TilerProfile::Balanced, TilerProfile::Lossless] {
        let output = work.path().join(format!("{profile:?}.3tz"));
        mesh_to_tileset(
            TilerRequest::new(&input, &output)
                .with_profile(profile)
                .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(2).unwrap())),
            &crate::RunControl::default(),
        )
        .unwrap();
        crate::validate::inspect(crate::validate::ValidationRequest::new(&output)).unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
        let (mut corners, mut worst) = (0usize, 0f64);
        for i in 0..zip.len() {
            let mut glb = Vec::new();
            zip.by_index(i).unwrap().read_to_end(&mut glb).unwrap();
            if !glb.starts_with(b"glTF") {
                continue;
            }
            let length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
            let doc: Value = serde_json::from_slice(&glb[20..20 + length]).unwrap();
            let bin = &glb[28 + length..];
            let metadata = &doc["extensions"]["EXT_structural_metadata"];
            if metadata["schema"]["id"] != "rusty_tiles_large_source_v1" {
                continue;
            }
            let table = &metadata["propertyTables"][1];
            let column = |name: &str| -> Vec<u64> {
                let index = table["properties"][name]["values"].as_u64().unwrap() as usize;
                view(&doc, bin, index)
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|c| u64::from_le_bytes(*c))
                    .collect()
            };
            let vertices = [column("vertex_0"), column("vertex_1"), column("vertex_2")];
            for primitive in doc["meshes"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|m| m["primitives"].as_array().unwrap())
            {
                let attributes = &primitive["attributes"];
                let accessor =
                    |name: &str| &doc["accessors"][attributes[name].as_u64().unwrap() as usize];
                let normal = accessor("NORMAL");
                let decoded = view(&doc, bin, normal["bufferView"].as_u64().unwrap() as usize);
                let ids = view(
                    &doc,
                    bin,
                    accessor("_FEATURE_ID_1")["bufferView"].as_u64().unwrap() as usize,
                );
                for (corner, id) in ids.as_chunks::<4>().0.iter().enumerate() {
                    let row = f32::from_le_bytes(*id) as usize;
                    let source = vertices[corner % 3][row] as usize;
                    // Capture renormalizes each source normal in f64 (attributes.rs).
                    let stored = &normals[3 * source..3 * source + 3];
                    let norm = stored
                        .iter()
                        .map(|v| f64::from(*v).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    let truth: Vec<f32> = stored
                        .iter()
                        .map(|v| (f64::from(*v) / norm) as f32)
                        .collect();
                    if profile == TilerProfile::Lossless {
                        assert_eq!(normal["componentType"], 5126);
                        let got: Vec<f32> = decoded[12 * corner..12 * corner + 12]
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|b| f32::from_le_bytes(*b))
                            .collect();
                        assert_eq!(
                            got.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                            truth.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                        );
                    } else {
                        assert_eq!(
                            (&normal["componentType"], &normal["normalized"]),
                            (&serde_json::json!(5122), &serde_json::json!(true))
                        );
                        let q: Vec<f64> = decoded[8 * corner..8 * corner + 6]
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|b| f64::from(i16::from_le_bytes(*b)) / 32767.)
                            .collect();
                        let ql = q.iter().map(|v| v * v).sum::<f64>().sqrt();
                        let dot = (0..3).map(|a| q[a] * f64::from(truth[a])).sum::<f64>() / ql;
                        worst = worst.max(dot.clamp(-1., 1.).acos().to_degrees());
                    }
                    corners += 1;
                }
            }
        }
        assert_eq!(
            corners,
            triangles * 3,
            "{profile:?}: every source corner in one leaf"
        );
        assert!(
            worst < 0.07,
            "{profile:?}: octahedral normal error {worst} degrees"
        );
    }
}

/// External H8 leaf-oracle driver (`bench/architecture_audit/mesh_large_h8/leaf_oracle.py
/// verify`): writes the private leaf-payload directory for an oracle-generated source.
#[test]
#[ignore = "external independent oracle supplies H8_INPUT and H8_OUTPUT"]
fn external_leaf_payload_driver() {
    use leaves::producer::{mesh_to_leaf_payloads, LeafOutputRequest, LeafProfile};
    let input = PathBuf::from(std::env::var_os("H8_INPUT").expect("H8_INPUT"));
    let output = PathBuf::from(std::env::var_os("H8_OUTPUT").expect("H8_OUTPUT"));
    let profile = match std::env::var("H8_PROFILE").as_deref().unwrap_or("balanced") {
        "fast" => LeafProfile::Fast,
        "balanced" => LeafProfile::Balanced,
        "quality" => LeafProfile::Quality,
        "lossless" => LeafProfile::Lossless,
        _ => panic!("invalid H8_PROFILE"),
    };
    mesh_to_leaf_payloads(
        LeafOutputRequest::new(&input, &output, profile),
        &crate::RunControl::default(),
    )
    .unwrap();
}
