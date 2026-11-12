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

/// Isolated tiny triangles reach the HLOD incidence pass's worst case (six interface items
/// each), past every capture stage. That pass declares and admits its phases as it starts,
/// so the whole-run spill check holds the run to that bound instead of refusing it against
/// capture's need alone (#309 dense output: 20,000 tiny primitives were refused).
#[test]
fn isolated_triangles_spill_within_the_declared_bound() {
    use crate::{mesh_large::TilerJobs, spill::SortConfig, RunControl};
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("src");
    fs::create_dir_all(&source).unwrap();
    let count = 80_000usize;
    let (mut bytes, mut max) = (Vec::new(), [0f32; 3]);
    for i in 0..count {
        let (x, y) = ((i % 200) as f32, (i / 200) as f32);
        for p in [[x, y, 0.], [x + 0.01, y, 0.], [x, y + 0.01, 0.]] {
            p.iter().for_each(|v: &f32| bytes.extend(v.to_le_bytes()));
            max = std::array::from_fn(|a| max[a].max(p[a]));
        }
    }
    let max = max.map(f64::from);
    fs::write(source.join("tiny.bin"), &bytes).unwrap();
    let doc = serde_json::json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"buffers":[{"uri":"tiny.bin","byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":bytes.len()}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":count*3,"type":"VEC3","min":[0,0,0],"max":max}]});
    let input = source.join("tiny.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let sort = SortConfig {
        run_bytes: 16 << 10,
        fan_in: 4,
        buffer_bytes: 4096,
        jobs: 1,
        writers: 0,
    };
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let request = super::TilerRequest::new(&input, work.path().join("out.3tz"))
        .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(2).unwrap()))
        .with_scratch_dir(&scratch)
        .with_test_spill(sort, 4096);
    let result = super::mesh_to_tileset(request, &RunControl::default()).unwrap();
    let (capture, hlod) = (&result.diagnostics.capture, &result.diagnostics.hlod);
    assert!(
        capture.run_scratch_peak_bytes > capture.scratch_logical_need_bytes,
        "the incidence pass outgrew every capture stage: {} vs {}",
        capture.run_scratch_peak_bytes,
        capture.scratch_logical_need_bytes
    );
    assert!(capture.run_scratch_peak_bytes <= hlod.stage_scratch_bound_bytes);
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
        writers: 0,
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
                let filter = Filter::parse(e["filter"].as_str().unwrap_or("NONE")).unwrap();
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

/// HLOD slice S6: H17 certified LOD and the H27 proxy bake are gone from production, with
/// no export, feature, hidden switch or compatibility record left behind. A static scan of
/// the crate, CLI and Python binding production sources (test-only `*tests.rs` files
/// excepted: they assert the banned records are absent from published output).
#[test]
fn no_h17_or_h27_production_symbols_remain() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for removed in [
        "src/mesh_large/lod",
        "src/mesh_large/bake",
        "src/mesh_large/bake_pass.rs",
        "src/mesh_large/leaves/witness.rs",
        "docs/architecture/h17-lod-contract.md",
    ] {
        assert!(!root.join(removed).exists(), "{removed} still exists");
    }
    // Module paths, types, request/CLI switches and the H17 coarse records.
    let banned = [
        "mesh_large::lod",
        "super::lod",
        "super::super::lod",
        "bake_pass",
        "ProxyBaker",
        "ProxyTextureBaker",
        "PreencodeStore",
        "PreencodePage",
        "HandoffSink",
        "TilerLod",
        "with_lod(",
        "Coarse::H17",
        "\"h17\"",
        "proxy_source",
        "lod_vanished",
        "lod_filled",
    ];
    let mut stack = vec![root.join("src"), root.join("bindings/python/src")];
    let mut scanned = 0;
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy();
            if !name.ends_with(".rs") || name.ends_with("tests.rs") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            for token in banned {
                assert!(!text.contains(token), "{} names {token}", path.display());
            }
            scanned += 1;
        }
    }
    assert!(scanned > 100, "scanned only {scanned} files");
    let module = fs::read_to_string(root.join("src/mesh_large/mod.rs")).unwrap();
    for declaration in ["mod lod;", "mod bake;"] {
        assert!(
            !module.contains(declaration),
            "mesh_large declares {declaration}"
        );
    }
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let features = manifest.split("[features]").nth(1).unwrap();
    let features = features.split("\n[").next().unwrap().to_ascii_lowercase();
    for name in ["h17", "h27", "lod", "proxy"] {
        assert!(!features.contains(name), "Cargo feature names {name}");
    }
}

/// A textured sheet for archive-level delivery checks: `side`² quads (two triangles each),
/// the left half in material 0 (LINEAR minification, no mips) and the right half in
/// material 1 (LINEAR_MIPMAP_LINEAR), each with its own smooth 256² image.
fn textured_sheet(dir: &Path, side: usize) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let (mut positions, mut uvs, mut indices) = (Vec::new(), Vec::new(), [Vec::new(), Vec::new()]);
    for y in 0..=side {
        for x in 0..=side {
            let z = ((x * 7 + y * 3) % 5) as f32 * 0.01;
            positions.extend([x as f32, y as f32, z]);
            uvs.extend([x as f32 / side as f32, y as f32 / side as f32]);
        }
    }
    let at = |x: usize, y: usize| (y * (side + 1) + x) as u32;
    for y in 0..side {
        for x in 0..side {
            let quad = [
                at(x, y),
                at(x + 1, y),
                at(x, y + 1),
                at(x + 1, y),
                at(x + 1, y + 1),
                at(x, y + 1),
            ];
            indices[usize::from(x >= side / 2)].extend(quad);
        }
    }
    let mut bytes = Vec::new();
    for v in positions.iter().chain(&uvs) {
        bytes.extend(v.to_le_bytes());
    }
    let index_offset = bytes.len();
    for i in indices.iter().flatten() {
        bytes.extend(i.to_le_bytes());
    }
    fs::write(dir.join("sheet.bin"), &bytes).unwrap();
    for (n, name) in ["a.png", "b.png"].iter().enumerate() {
        image::RgbaImage::from_fn(256, 256, |x, y| {
            image::Rgba([
                (x as u8).wrapping_add(n as u8 * 80),
                y as u8,
                ((x + y) / 2) as u8,
                255,
            ])
        })
        .save(dir.join(name))
        .unwrap();
    }
    let vertices = (side + 1) * (side + 1);
    let half = indices[0].len();
    let doc = serde_json::json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "meshes":[{"primitives":[
            {"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":2,"material":0},
            {"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":3,"material":1}]}],
        "buffers":[{"uri":"sheet.bin","byteLength":bytes.len()}],
        "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":vertices * 12},
            {"buffer":0,"byteOffset":vertices * 12,"byteLength":vertices * 8},
            {"buffer":0,"byteOffset":index_offset,"byteLength":half * 4},
            {"buffer":0,"byteOffset":index_offset + half * 4,"byteLength":indices[1].len() * 4}],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":vertices,"type":"VEC3",
             "min":[0,0,0],"max":[side as f32, side as f32, 0.04]},
            {"bufferView":1,"componentType":5126,"count":vertices,"type":"VEC2"},
            {"bufferView":2,"componentType":5125,"count":half,"type":"SCALAR"},
            {"bufferView":3,"componentType":5125,"count":indices[1].len(),"type":"SCALAR"}],
        "images":[{"uri":"a.png"},{"uri":"b.png"}],
        "textures":[{"source":0,"sampler":0},{"source":1,"sampler":1}],
        "samplers":[{"minFilter":9729,"magFilter":9729},{"minFilter":9987,"magFilter":9729}],
        "materials":[
            {"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}},
            {"pbrMetallicRoughness":{"baseColorTexture":{"index":1}}}]
    });
    let input = dir.join("sheet.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    input
}

/// A GLB's JSON document and binary chunk.
fn glb_parts(bytes: &[u8]) -> (serde_json::Value, &[u8]) {
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let doc = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
    (doc, bytes.get(28 + length..).unwrap_or(&[]))
}

/// S4 through the public operation. Every leaf and coarse texture of the published archive
/// is checked independently of the producer: the KTX2 container header and level index are
/// parsed here, every level is transcoded and sized, and its level count must be what the
/// published sampler reads (the complete chain for a mipmapping minification filter, level
/// 0 alone otherwise, S3 review P3-2). The archive declares HLOD in `tileset.json`, so plain
/// validation runs the HLOD checks, and the same archive stripped of its identity labels is
/// refused without any expectation flag.
#[test]
fn published_textures_match_their_samplers_and_the_archive_declares_hlod() {
    use std::io::Read;
    let work = tempfile::tempdir().unwrap();
    let input = textured_sheet(&work.path().join("src"), 24);
    let output = work.path().join("sheet.3tz");
    let result = mesh_to_tileset(
        TilerRequest::new(&input, &output).with_test_leaf_triangles(256),
        &crate::RunControl::default(),
    )
    .unwrap();
    assert!(result.diagnostics.hlod.tiles >= 3);
    // D8: storage on memory-backed filesystems is charged, and measured within its charge.
    let memory = &result.diagnostics.memory;
    let storage = &memory.memory_backed_storage;
    let tmpfs = crate::spill::inspect_volume(work.path())
        .unwrap()
        .memory_backed;
    assert_eq!(storage.work_charge_bytes.is_some(), tmpfs);
    assert_eq!(storage.archive_charge_bytes.is_some(), tmpfs);
    assert_eq!(
        memory.budget.memory_backed_storage_bytes,
        storage.work_charge_bytes.unwrap_or(0) + storage.archive_charge_bytes.unwrap_or(0)
    );
    if tmpfs {
        assert!(storage.work_bytes.unwrap() <= storage.work_charge_bytes.unwrap());
        assert_eq!(
            storage.archive_bytes,
            Some(result.diagnostics.archive_bytes)
        );
        assert!(storage.archive_bytes <= storage.archive_charge_bytes);
    }
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    let mut members = Vec::new();
    for i in 0..zip.len() {
        let mut member = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        members.push((member.name().to_owned(), bytes));
    }
    let tileset: serde_json::Value = serde_json::from_slice(&members[0].1).unwrap();
    assert_eq!(members[0].0, "tileset.json");
    assert_eq!(tileset["asset"]["extras"]["rustyTiles"]["lod"], "hlod");
    // (leaf level-0-only, leaf complete chain, coarse complete chain) KTX2 textures.
    let mut seen = [0u32; 3];
    for (name, bytes) in members.iter().filter(|(n, _)| n.ends_with(".glb")) {
        let coarse = bytes.windows(25).any(|w| w == b"rusty_tiles_large_hlod_v1");
        let (doc, bin) = glb_parts(bytes);
        for texture in doc["textures"].as_array().into_iter().flatten() {
            let sampler = &doc["samplers"][texture["sampler"].as_u64().unwrap() as usize];
            let mipmapped = matches!(sampler["minFilter"].as_u64(), Some(9984..=9987) | None);
            let image = texture["extensions"]["KHR_texture_basisu"]["source"]
                .as_u64()
                .or(texture["source"].as_u64())
                .unwrap() as usize;
            let image = &doc["images"][image];
            let view = &doc["bufferViews"][image["bufferView"].as_u64().unwrap() as usize];
            let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
            let data = &bin[start..start + view["byteLength"].as_u64().unwrap() as usize];
            if image["mimeType"] != "image/ktx2" {
                assert_eq!(image["mimeType"], "image/png", "{name}");
                image::load_from_memory_with_format(data, image::ImageFormat::Png).unwrap();
                continue;
            }
            // KTX2 header (KTX 2.0 spec section 3): identifier, then little-endian fields.
            assert_eq!(&data[..12], b"\xABKTX 20\xBB\r\n\x1A\n", "{name}");
            let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
            let u64_at = |o: usize| u64::from_le_bytes(data[o..o + 8].try_into().unwrap());
            let (vk_format, width, height) = (u32_at(12), u32_at(20), u32_at(24));
            let (depth, layers, faces, levels) = (u32_at(28), u32_at(32), u32_at(36), u32_at(40));
            assert_eq!(
                (vk_format, depth, layers, faces),
                (0, 0, 0, 1),
                "{name}: Basis 2D"
            );
            assert!(matches!(u32_at(44), 1 | 2), "{name}: BasisLZ or Zstandard");
            let complete = 32 - width.max(height).leading_zeros();
            assert_eq!(
                levels,
                if mipmapped { complete } else { 1 },
                "{name}: levels for minFilter {}",
                sampler["minFilter"]
            );
            let transcoder = basisu::Transcoder::new(data).unwrap();
            for level in 0..levels {
                let index = 80 + 24 * level as usize;
                let (offset, length) = (u64_at(index), u64_at(index + 8));
                assert!(
                    length > 0 && offset + length <= data.len() as u64,
                    "{name} L{level}"
                );
                let rgba = transcoder
                    .transcode(
                        level,
                        basisu::TargetFormat::Rgba32,
                        basisu::DecodeFlags::NONE,
                    )
                    .unwrap();
                let (w, h) = ((width >> level).max(1), (height >> level).max(1));
                assert_eq!(
                    rgba.len() as u64,
                    u64::from(w) * u64::from(h) * 4,
                    "{name} L{level}"
                );
            }
            match (coarse, levels == complete && levels > 1) {
                (false, false) => seen[0] += 1,
                (false, true) => seen[1] += 1,
                (true, true) => seen[2] += 1,
                (true, false) => panic!("{name}: a coarse KTX2 page without its chain"),
            }
        }
    }
    assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
    // Declared: plain validation holds the archive to every HLOD check.
    let checked =
        crate::validate::inspect(crate::validate::ValidationRequest::new(&output)).unwrap();
    assert!(checked.checks.iter().any(|c| c == "hlodFeatureIds"));
    assert!(checked.interface_census.is_some());
    // Stripped of every identity label (as the S5 omission controls strip), the declared
    // archive is refused without a flag.
    let stripped = work.path().join("stripped");
    for (name, bytes) in &members {
        let path = stripped.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if !name.ends_with(".glb") {
            fs::write(&path, bytes).unwrap();
            continue;
        }
        let (mut doc, bin) = glb_parts(bytes);
        let doc = doc.as_object_mut().unwrap();
        doc.remove("extensions");
        if let Some(used) = doc.get_mut("extensionsUsed").and_then(|u| u.as_array_mut()) {
            used.retain(|e| e != "EXT_mesh_features" && e != "EXT_structural_metadata");
        }
        for mesh in doc["meshes"].as_array_mut().unwrap() {
            for primitive in mesh["primitives"].as_array_mut().unwrap() {
                let primitive = primitive.as_object_mut().unwrap();
                primitive.remove("extensions");
                primitive["attributes"]
                    .as_object_mut()
                    .unwrap()
                    .retain(|k, _| !k.starts_with("_FEATURE_ID_"));
            }
        }
        let mut text = serde_json::to_vec(&doc).unwrap();
        text.resize(text.len().next_multiple_of(4), b' ');
        let mut glb = b"glTF".to_vec();
        for word in [2, (28 + text.len() + bin.len()) as u32, text.len() as u32] {
            glb.extend(word.to_le_bytes());
        }
        glb.extend(b"JSON");
        glb.extend(&text);
        glb.extend((bin.len() as u32).to_le_bytes());
        glb.extend(b"BIN\0");
        glb.extend(bin);
        fs::write(&path, glb).unwrap();
    }
    fs::remove_file(stripped.join(crate::TZ_INDEX_NAME)).ok();
    let archive = work.path().join("stripped.3tz");
    crate::package::package(
        crate::package::PackageRequest::directory(&stripped, &archive),
        &crate::RunControl::default(),
    )
    .unwrap();
    let error = crate::validate::inspect(crate::validate::ValidationRequest::new(&archive))
        .unwrap_err()
        .to_string();
    assert!(error.contains("hlodFeatureIds"), "{error}");
}

/// D8 through the public operation with a metered work directory (simulated memory-backed
/// storage, so the control runs on any filesystem): a charge that holds the run admits it
/// with every stored file inside what was admitted, and the same archive results; a tight
/// charge is refused as `resource_limit` at the write that would pass it, publishing nothing
/// and leaving no work files.
// Memory-backed work storage is detected only on Linux, and the stored-model cross-check
// counts hard links by link count, which stable Rust exposes only on Unix.
#[cfg(unix)]
#[test]
fn tight_memory_backed_work_charge_is_refused_before_it_is_exceeded() {
    let work = tempfile::tempdir().unwrap();
    let input = textured_sheet(&work.path().join("src"), 24);
    let run = |name: &str, charge: Option<u64>| {
        let scratch = work.path().join(format!("scratch-{name}"));
        fs::create_dir_all(&scratch).unwrap();
        let mut request = TilerRequest::new(&input, work.path().join(name))
            .with_test_leaf_triangles(256)
            .with_scratch_dir(&scratch);
        if let Some(charge) = charge {
            request = request.with_test_work_charge(charge);
        }
        let result = mesh_to_tileset(request, &crate::RunControl::default());
        assert_eq!(
            fs::read_dir(&scratch).unwrap().count(),
            0,
            "{name}: work files left"
        );
        result
    };
    let plain = run("plain.3tz", None).unwrap();
    let metered = run("metered.3tz", Some(64 << 20)).unwrap();
    let storage = &metered.diagnostics.memory.memory_backed_storage;
    let (admitted, stored) = (
        storage.work_admitted_bytes.unwrap(),
        storage.work_bytes.unwrap(),
    );
    assert!(
        stored <= admitted && admitted <= 64 << 20,
        "{stored} {admitted}"
    );
    assert_eq!(
        fs::read(work.path().join("plain.3tz")).unwrap(),
        fs::read(work.path().join("metered.3tz")).unwrap()
    );
    assert_eq!(plain.report.leaves, metered.report.leaves);
    for charge in [admitted - 1, admitted / 2, 64 << 10] {
        let failure = run("tight.3tz", Some(charge)).unwrap_err();
        assert_eq!(
            failure.error.kind(),
            JobErrorKind::ResourceLimit,
            "{}",
            failure.error
        );
        let message = failure.error.to_string();
        assert!(
            message.contains("work directory on memory-backed storage")
                && message.contains(&format!("the {charge} bytes charged")),
            "{message}"
        );
        assert!(!work.path().join("tight.3tz").exists());
    }
}

/// The budget plans of capture and of the coarse stage's incidence pass grow their sorts
/// (single pass, run writers) from small floors, and publish the floor's archive byte for
/// byte, each stage's measured scratch within its declaration.
#[test]
fn planned_capture_and_incidence_sorts_publish_the_floor_archive() {
    use crate::{mesh_large::TilerJobs, spill::SortConfig, RunControl};
    let work = tempfile::tempdir().unwrap();
    // The plan needs disk-backed scratch and the system temporary directory may be tmpfs.
    let disk = tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
    let input = super::full::tests::shuffled_grid(&work.path().join("src"), 30, 0x7e57_5eed);
    // Production's floor fan-in with small runs: a larger fan-in would declare a larger file
    // inventory than the floor's, which the plans never do, so they grow runs and writers.
    let sort = SortConfig {
        run_bytes: 8 << 10,
        fan_in: 64,
        buffer_bytes: 4096,
        jobs: 1,
        writers: 0,
    };
    let mut archives = Vec::new();
    for plan in [false, true] {
        let scratch = disk.path().join(format!("scratch-{plan}"));
        fs::create_dir(&scratch).unwrap();
        let output = work.path().join(format!("plan-{plan}.3tz"));
        let request = super::TilerRequest::new(&input, &output)
            .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(4).unwrap()))
            .with_scratch_dir(&scratch)
            .with_memory_budget(2 << 30)
            .with_test_leaf_triangles(64)
            .with_test_spill(sort, 4096)
            .with_test_sort_plan(plan);
        let result = super::mesh_to_tileset(request, &RunControl::default()).unwrap();
        let diagnostics = &result.diagnostics;
        let (capture, hlod) = (&diagnostics.capture, &diagnostics.hlod);
        let incidence = diagnostics.memory.hlod.incidence_sort;
        assert!(!capture.scratch_memory_backed, "planned scratch on disk");
        assert!(hlod.tiles > 0, "the coarse stage ran");
        if plan {
            assert_eq!(
                (capture.sort.limited_by, capture.sort.single_pass),
                ("single_pass", true)
            );
            assert!(capture.sort.writers > 0, "{:?}", capture.sort);
            assert_eq!(
                (incidence.limited_by, incidence.single_pass),
                ("single_pass", true)
            );
            assert!(incidence.writers > 0, "{incidence:?}");
            assert_eq!(
                incidence.fan_in, 64,
                "the incidence plan keeps the floor's fan-in"
            );
            assert!(incidence.run_bytes > 4 << 10, "{incidence:?}");
        } else {
            assert_eq!(capture.sort.limited_by, "floor");
            assert_eq!(incidence.limited_by, "floor");
        }
        // Measured scratch within every declaration: capture's stages, the incidence pass's
        // phases and the whole run.
        let stages = [
            (
                capture.scratch_stage_peak_bytes.capture,
                capture.scratch_stage_bytes.capture,
            ),
            (
                capture.scratch_stage_peak_bytes.tree,
                capture.scratch_stage_bytes.tree,
            ),
            (
                capture.scratch_stage_peak_bytes.corner_depths,
                capture.scratch_stage_bytes.corner_depths,
            ),
        ];
        for (peak, bound) in stages {
            assert!(peak <= bound, "plan {plan}: {peak} > {bound}");
        }
        assert!(hlod.incidence_scratch_peak_bytes <= hlod.incidence_scratch.declared());
        assert!(capture.run_scratch_peak_bytes <= capture.run_scratch_bound_bytes);
        archives.push(fs::read(&output).unwrap());
        assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    }
    assert!(
        archives[0] == archives[1],
        "the planned run publishes the floor's archive"
    );
}

/// Rewrites the PNG at `path` to exactly `length` bytes by inserting one private ancillary
/// chunk of zeros after the image data. The chunk data is a sparse hole, so a gigabyte-scale
/// source costs no disk until something reads it; decoders skip the chunk, so the pixels
/// are unchanged.
fn pad_png(path: &Path, length: u64) {
    use std::io::{Seek, SeekFrom};
    let png = fs::read(path).unwrap();
    let body = &png[..png.len() - 12];
    assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    let data = length - body.len() as u64 - 24;
    let mut crc = crc32fast::Hasher::new();
    crc.update(b"ruTi");
    let zeros = vec![0u8; 1 << 20];
    let mut left = data;
    while left > 0 {
        let n = left.min(zeros.len() as u64) as usize;
        crc.update(&zeros[..n]);
        left -= n as u64;
    }
    let mut file = fs::File::create(path).unwrap();
    file.write_all(body).unwrap();
    file.write_all(&u32::try_from(data).unwrap().to_be_bytes())
        .unwrap();
    file.write_all(b"ruTi").unwrap();
    file.seek(SeekFrom::Current(data as i64)).unwrap();
    file.write_all(&crc.finalize().to_be_bytes()).unwrap();
    file.write_all(&png[png.len() - 12..]).unwrap();
    drop(file);
    assert_eq!(fs::metadata(path).unwrap().len(), length);
}

/// #338: an encoded image is charged at its actual length instead of being refused at a
/// fixed 32 MiB. One byte over the old ceiling builds, and every archive member is
/// byte-identical to the same sheet with its unpadded image: admission and accounting do not
/// change output. The one exception is the report's source fact `external_bytes`, which
/// counts the padding.
#[test]
fn encoded_image_past_the_old_32_mib_ceiling_is_admitted_and_builds_identically() {
    use std::io::Read;
    let work = tempfile::tempdir().unwrap();
    let run = |dir: &str, pad: Option<u64>| {
        let input = textured_sheet(&work.path().join(dir), 4);
        if let Some(length) = pad {
            pad_png(&input.with_file_name("a.png"), length);
        }
        let output = work.path().join(format!("{dir}.3tz"));
        let result = super::mesh_to_tileset(
            super::TilerRequest::new(&input, &output)
                .with_jobs(super::TilerJobs::Maximum(NonZeroUsize::new(2).unwrap()))
                .with_memory_budget(2 << 30),
            &crate::RunControl::default(),
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
        let members = (0..zip.len())
            .map(|i| {
                let mut member = zip.by_index(i).unwrap();
                let mut bytes = Vec::new();
                member.read_to_end(&mut bytes).unwrap();
                (member.name().to_owned(), bytes)
            })
            .collect::<Vec<_>>();
        (
            members,
            result.diagnostics.memory.hlod.texture_workspace_bytes,
        )
    };
    let (padded, padded_texture) = run("padded", Some((32 << 20) + 1));
    let (plain, plain_texture) = run("plain", None);
    // The coarse stage charges the largest encoded image at its actual length.
    let largest_plain = ["a.png", "b.png"]
        .map(|n| {
            fs::metadata(work.path().join("plain").join(n))
                .unwrap()
                .len()
        })
        .into_iter()
        .max()
        .unwrap();
    assert!(largest_plain < 1 << 20);
    assert_eq!(
        padded_texture - plain_texture,
        (32 << 20) + 1 - largest_plain
    );
    assert_eq!(padded.len(), plain.len());
    assert!(padded.iter().any(|(name, _)| name.ends_with(".glb")));
    for ((name, a), (other, b)) in padded.iter().zip(&plain) {
        assert_eq!(name, other);
        if name != "conversion.json" {
            assert!(a == b, "{name} differs");
            continue;
        }
        let [mut a, mut b] =
            [a, b].map(|v| serde_json::from_slice::<serde_json::Value>(v).unwrap());
        let padding = a["external_bytes"].as_u64().unwrap() - b["external_bytes"].as_u64().unwrap();
        assert_eq!(
            padding,
            (32 << 20) + 1 - fs::metadata(work.path().join("plain/a.png")).unwrap().len()
        );
        a["external_bytes"] = 0.into();
        b["external_bytes"] = 0.into();
        assert_eq!(a, b);
    }
}

/// Rewrites the PNG IHDR at `path` to declare `width` x `height` (CRC kept valid). The pixel
/// data no longer matches, so only header admission can read the result.
fn declare_png_size(path: &Path, width: u32, height: u32) {
    let mut png = fs::read(path).unwrap();
    assert_eq!(&png[12..16], b"IHDR");
    png[16..20].copy_from_slice(&width.to_be_bytes());
    png[20..24].copy_from_slice(&height.to_be_bytes());
    let crc = crc32fast::hash(&png[12..29]).to_be_bytes();
    png[29..33].copy_from_slice(&crc);
    fs::write(path, png).unwrap();
}

/// A temporary directory on disk-backed storage, so capture scratch is not charged to the
/// memory budget (which would refuse large sources earlier, at capture's resident bound).
fn disk_backed_workdir() -> tempfile::TempDir {
    let parent = [
        std::env::temp_dir(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    ]
    .into_iter()
    .find(|dir| crate::spill::inspect_volume(dir).is_ok_and(|volume| !volume.memory_backed))
    .expect("a disk-backed temporary or manifest directory");
    tempfile::Builder::new()
        .prefix(".image-admission-")
        .tempdir_in(parent)
        .unwrap()
}

/// A large encoded image the budget cannot hold is a typed resource refusal naming the
/// image stage, raised before any scratch or output exists; over the 1 GiB sanity ceiling
/// it stays `unsupported`. Both sources are sparse, so neither run reads gigabytes. The
/// work directory must be disk-backed: memory-backed scratch would charge the 1 GiB
/// snapshot to the budget and refuse earlier, at capture's resident bound.
#[test]
fn large_encoded_images_are_refused_by_budget_or_sanity_ceiling() {
    let work = disk_backed_workdir();
    let request = |dir: &str, budget: u64| {
        let input = textured_sheet(&work.path().join(dir), 4);
        let scratch = work.path().join(format!("{dir}-scratch"));
        fs::create_dir(&scratch).unwrap();
        let output = work.path().join(format!("{dir}.3tz"));
        let request = super::TilerRequest::new(&input, &output)
            .with_scratch_dir(&scratch)
            .with_jobs(super::TilerJobs::Maximum(NonZeroUsize::new(1).unwrap()))
            .with_memory_budget(budget);
        (input, scratch, output, request)
    };
    // Capture's own resident bound for this sheet, from an admitted run of its plain image.
    let (_, _, _, plain) = request("plain", 2 << 30);
    let bound = super::mesh_to_tileset(plain, &crate::RunControl::default())
        .unwrap()
        .diagnostics
        .memory
        .capture_bound_bytes;
    // Capture's bound fits with 512 MiB to spare, so image decoding has those 512 MiB plus
    // the 192 MiB decoder allowance: less than the 1 GiB image, and less than the decode
    // of a 16384² image even though its 110 MB encoding would fit. Only capture's header
    // preflight names "image capture:"; the later decode batches refuse differently.
    for (dir, length, budget, kind, needle) in [
        (
            // Review of #378: 110 MB fits the budget but its 16384² decode does not; the
            // header alone refuses it before the snapshot is written.
            "texels",
            110_000_000,
            bound + (512 << 20),
            JobErrorKind::ResourceLimit,
            "image capture: decoding a source image needs 3331225472 bytes (110000000 \
             encoded plus 12 per texel of 16384x16384)",
        ),
        (
            "budget",
            1u64 << 30,
            bound + (512 << 20),
            JobErrorKind::ResourceLimit,
            "image capture: decoding a source image needs 1074528256 bytes (1073741824 encoded",
        ),
        (
            "ceiling",
            (1 << 30) + 1,
            64 << 30,
            JobErrorKind::Unsupported,
            "encoded image exceeds 1 GiB admission ceiling",
        ),
    ] {
        let (input, scratch, output, request) = request(dir, budget);
        if dir == "texels" {
            declare_png_size(&input.with_file_name("a.png"), 16384, 16384);
        }
        pad_png(&input.with_file_name("a.png"), length);
        let failure = match super::mesh_to_tileset(request, &crate::RunControl::default()) {
            Ok(_) => panic!("{dir}: a {length}-byte image must be refused"),
            Err(failure) => failure,
        };
        assert_eq!(failure.error.kind(), kind, "{dir}: {}", failure.error);
        let message = failure.error.to_string();
        assert!(message.contains(needle), "{dir}: {message}");
        assert!(!output.exists(), "{dir}");
        assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0, "{dir}");
    }
}

/// #338: capture's pre-scratch refusal sits exactly at the budget. A 256 MiB image (above
/// the 192 MiB decoder allowance) is refused under a budget that leaves its whole decode
/// charge less one byte, naming both figures; one byte more budget admits it, and the
/// run's later decode batches admit the same charge, so the two checks agree.
#[test]
fn capture_admits_the_largest_decode_charge_exactly_at_the_budget() {
    let work = disk_backed_workdir();
    let input = textured_sheet(&work.path().join("src"), 4);
    let plain = textured_sheet(&work.path().join("plain"), 4);
    pad_png(&input.with_file_name("a.png"), 256 << 20);
    let run = |name: &str, source: &Path, budget: u64| {
        let scratch = work.path().join(format!("{name}-scratch"));
        fs::create_dir(&scratch).unwrap();
        let output = work.path().join(format!("{name}.3tz"));
        let result = super::mesh_to_tileset(
            super::TilerRequest::new(source, &output)
                .with_scratch_dir(&scratch)
                .with_jobs(super::TilerJobs::Maximum(NonZeroUsize::new(1).unwrap()))
                .with_memory_budget(budget),
            &crate::RunControl::default(),
        );
        (result, output, scratch)
    };
    let figure = |message: &str, before: &str, after: &str| -> u64 {
        let start = message.find(before).unwrap() + before.len();
        let end = start + message[start..].find(after).unwrap();
        message[start..end].parse().unwrap()
    };
    let bound = run("plain", &plain, 2 << 30)
        .0
        .unwrap()
        .diagnostics
        .memory
        .capture_bound_bytes;
    // Capture's bound fits with 1 MiB to spare: decoding gets about 193 MiB.
    let (result, _, _) = run("probe", &input, bound + (1 << 20));
    let message = result.unwrap_err().error.to_string();
    let charge = figure(&message, "needs ", " bytes");
    let leaves = figure(&message, "leaves ", " for");
    assert_eq!(charge, (256 << 20) + 12 * 256 * 256, "{message}");
    assert!(leaves < charge, "{message}");
    let exact = bound + (1 << 20) + (charge - leaves);
    // Run names keep one length: the bound charges the scratch and output path lengths.
    let (result, output, scratch) = run("below", &input, exact - 1);
    let failure = result.unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::ResourceLimit);
    let message = failure.error.to_string();
    assert!(message.starts_with("image capture:"), "{message}");
    assert_eq!(figure(&message, "leaves ", " for"), charge - 1, "{message}");
    assert!(!output.exists());
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    // At exactly the charge, capture runs on. A later stage may still refuse this
    // budget, but not image admission or its decode batches.
    if let Err(failure) = run("exact", &input, exact).0 {
        let message = failure.error.to_string();
        assert!(
            !message.starts_with("image capture:")
                && !message.contains("decoding an admitted image does not fit"),
            "{message}"
        );
    }
}

/// #338 evidence lane: an 8192² RGB noise PNG of about 100 MB (three times the old ceiling)
/// admits under a 4 GiB budget and builds. The noise is drawn from 16 levels per channel and
/// left unfiltered, so deflate cannot shrink it much below 4 bits per sample. Generated
/// here, never committed.
/// Run with `cargo test --release --lib -- --ignored eight_k_noise_png`.
#[test]
#[ignore = "slow: generates and tiles a ~100 MB 8192² PNG"]
fn eight_k_noise_png_admits_under_a_few_gib_budget_and_builds() {
    let work = tempfile::tempdir().unwrap();
    let input = textured_sheet(&work.path().join("src"), 4);
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut raw = vec![0u8; 8192 * 8192 * 3];
    for chunk in raw.chunks_mut(16) {
        let bits = next();
        for (i, byte) in chunk.iter_mut().enumerate() {
            *byte = ((bits >> (4 * i)) & 15) as u8 * 17;
        }
    }
    {
        use image::{codecs::png, ImageEncoder};
        let file = BufWriter::new(fs::File::create(input.with_file_name("a.png")).unwrap());
        png::PngEncoder::new_with_quality(
            file,
            png::CompressionType::Level(1),
            png::FilterType::NoFilter,
        )
        .write_image(&raw, 8192, 8192, image::ExtendedColorType::Rgb8)
        .unwrap();
    }
    let encoded = fs::metadata(input.with_file_name("a.png")).unwrap().len();
    assert!(encoded > 2 * (32 << 20), "{encoded} bytes");
    let output = work.path().join("noise.3tz");
    let result = super::mesh_to_tileset(
        super::TilerRequest::new(&input, &output)
            .with_jobs(super::TilerJobs::Maximum(NonZeroUsize::new(2).unwrap()))
            .with_memory_budget(4 << 30),
        &crate::RunControl::default(),
    )
    .unwrap();
    assert!(output.exists());
    eprintln!(
        "encoded {encoded} bytes; minimum budget {} bytes",
        result.diagnostics.memory.minimum_bytes
    );
}

/// #338 evidence lane: an image exactly at the 1 GiB sanity ceiling (a sparse 256² PNG
/// padded with an ancillary chunk) is admitted and builds under a budget that holds it.
/// Capture writes the whole gigabyte to scratch and each stage rebuilds it to decode.
/// Run with `cargo test --release --lib -- --ignored one_gib_encoded_image`.
#[test]
#[ignore = "slow: captures and decodes a 1 GiB encoded image"]
fn one_gib_encoded_image_at_the_ceiling_builds() {
    let work = disk_backed_workdir();
    let input = textured_sheet(&work.path().join("src"), 4);
    pad_png(&input.with_file_name("a.png"), 1 << 30);
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let output = work.path().join("gib.3tz");
    let result = super::mesh_to_tileset(
        super::TilerRequest::new(&input, &output)
            .with_scratch_dir(&scratch)
            .with_jobs(super::TilerJobs::Maximum(NonZeroUsize::new(1).unwrap()))
            .with_memory_budget(6 << 30),
        &crate::RunControl::default(),
    )
    .unwrap();
    assert!(output.exists());
    eprintln!(
        "minimum budget {} bytes",
        result.diagnostics.memory.minimum_bytes
    );
}
