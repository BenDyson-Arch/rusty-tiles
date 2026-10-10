//! External facade consumer: source admission and lifecycle, without private imports.
use rusty_tiles::{
    mesh_to_archive, JobError, JobErrorKind, MeshPlacement, MeshRequest, Observer, OutputPolicy,
    RunControl, RunEvent,
};
use serde_json::{json, Value};
use std::{fs, path::Path, sync::Arc};

fn source() -> (Value, Vec<u8>) {
    let mut bin = Vec::new();
    for p in [
        [0_f32, 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [2., 0., 0.],
        [3., 0., 0.],
        [2., 1., 0.],
    ] {
        for x in p {
            bin.extend(x.to_le_bytes());
        }
    }
    (
        json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
        "buffers":[{"byteLength":bin.len()}],"bufferViews":[{"buffer":0,"byteLength":bin.len()}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":6,"type":"VEC3","min":[0,0,0],"max":[3,1,0]}]}),
        bin,
    )
}
fn encode(doc: &Value, bin: Vec<u8>) -> Vec<u8> {
    encode_json(serde_json::to_vec(doc).unwrap(), bin)
}
fn encode_json(mut json: Vec<u8>, mut bin: Vec<u8>) -> Vec<u8> {
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut bytes = b"glTF".to_vec();
    for n in [
        2,
        (28 + json.len() + bin.len()) as u32,
        json.len() as u32,
        0x4e4f534a,
    ] {
        bytes.extend(n.to_le_bytes());
    }
    bytes.extend(json);
    bytes.extend((bin.len() as u32).to_le_bytes());
    bytes.extend(0x004e4942_u32.to_le_bytes());
    bytes.extend(bin);
    bytes
}

#[test]
fn json_ambiguity_and_float_component_bounds_have_one_interpretation() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.glb");
    let output = work.path().join("out.3tz");
    let (doc, bin) = source();
    let text = serde_json::to_string(&doc)
        .unwrap()
        .replace("\"scene\":0", "\"scene\":0,\"scene\":0");
    fs::write(&input, encode_json(text.into_bytes(), bin.clone())).unwrap();
    let error = mesh_to_archive(
        MeshRequest::local_gltf(&input, &output, 1),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.error.kind(), JobErrorKind::InvalidInput);
    assert!(!output.exists());
    let mut text = serde_json::to_vec(&doc).unwrap();
    text.extend(b"\n\n\n\n");
    fs::write(&input, encode_json(text, bin)).unwrap();
    let error = mesh_to_archive(
        MeshRequest::local_gltf(&input, &output, 1),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.error.kind(), JobErrorKind::InvalidInput);
    assert!(!output.exists());
    let (mut doc, mut bin) = source();
    for i in 0..6 {
        bin[i * 12..i * 12 + 4].copy_from_slice(&0.1_f32.to_le_bytes());
    }
    doc["accessors"][0]["min"][0] = json!(0.1);
    doc["accessors"][0]["max"][0] = json!(0.1);
    fs::write(&input, encode(&doc, bin)).unwrap();
    assert_eq!(
        mesh_to_archive(
            MeshRequest::local_gltf(&input, &output, 1),
            &RunControl::default()
        )
        .unwrap()
        .report
        .triangles,
        2
    );
}
fn write_source(path: &Path) {
    let (doc, bin) = source();
    fs::write(path, encode(&doc, bin)).unwrap();
}

#[test]
fn admission_ceilings_and_extra_chunks_are_checked_before_output_work() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.glb");
    let absent = work.path().join("absent");
    let output = absent.join("output.3tz");
    let check = |bytes: Vec<u8>| {
        fs::write(&input, bytes).unwrap();
        let e = mesh_to_archive(
            MeshRequest::local_gltf(&input, &output, 100_000),
            &RunControl::default(),
        )
        .unwrap_err();
        assert_eq!(e.error.kind(), JobErrorKind::Unsupported, "{e}");
        assert!(!absent.exists());
    };
    let (mut doc, bin) = source();
    doc["asset"]["generator"] = json!("x".repeat(1024 * 1024));
    check(encode(&doc, bin));
    let (mut doc, bin) = source();
    doc["accessors"][0]["count"] = json!(1_000_001);
    check(encode(&doc, bin));
    let (mut doc, bin) = source();
    let bin = bin.repeat(2048);
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    doc["bufferViews"][0]["byteLength"] = json!(bin.len());
    doc["accessors"][0]["count"] = json!(6 * 2048);
    doc["nodes"] = json!((0..25).map(|_| json!({"mesh":0})).collect::<Vec<_>>());
    doc["scenes"][0]["nodes"] = json!((0..25).collect::<Vec<_>>());
    check(encode(&doc, bin)); // declared4096, expanded102400 triangles
    let (doc, bin) = source();
    let mut extra = encode(&doc, bin);
    extra.extend(4u32.to_le_bytes());
    extra.extend(*b"TEST");
    extra.extend([0; 4]);
    let size = extra.len() as u32;
    extra[8..12].copy_from_slice(&size.to_le_bytes());
    check(extra);
    let file = fs::File::create(&input).unwrap();
    file.set_len(32 * 1024 * 1024 + 1).unwrap();
    drop(file);
    let error = mesh_to_archive(
        MeshRequest::local_gltf(&input, &output, 1),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.error.kind(), JobErrorKind::Unsupported);
    assert!(!absent.exists());
}

#[test]
fn public_facade_multileaf_report_and_repeat_are_isolated() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("mesh.glb");
    write_source(&input);
    for limit in [1, 2] {
        let output = work.path().join(format!("{limit}.3tz"));
        let run = RunControl::default();
        let result =
            mesh_to_archive(MeshRequest::local_gltf(&input, &output, limit), &run).unwrap();
        assert_eq!(result.report.triangles, 2);
        assert_eq!(result.report.leaf_tiles, if limit == 1 { 2 } else { 1 });
        assert!(result.cleanup_diagnostics.is_empty());
        let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
        let report: Value =
            serde_json::from_reader(zip.by_name("conversion.json").unwrap()).unwrap();
        assert_eq!(report, serde_json::to_value(&result.report).unwrap());
        assert_eq!(zip.len(), result.report.leaf_tiles as usize + 3);
        assert_eq!(
            mesh_to_archive(MeshRequest::local_gltf(&input, &output, limit), &run)
                .unwrap_err()
                .error
                .kind(),
            JobErrorKind::InvalidState
        );
    }
}

#[test]
fn malformed_and_unsupported_sources_fail_before_output_work_for_every_leaf_limit() {
    type Mutation = fn(&mut Value, &mut Vec<u8>);
    let cases: &[(&str, JobErrorKind, Mutation)] = &[
        ("unused-u32", JobErrorKind::InvalidInput, |d, _| {
            d["accessors"].as_array_mut().unwrap().push(json!({
                "bufferView": 0, "componentType": 5125, "count": 1, "type": "SCALAR"
            }));
        }),
        (
            "shared-vertex-without-stride",
            JobErrorKind::InvalidInput,
            |d, b| {
                let offset = b.len();
                for _ in 0..6 {
                    for v in [0f32, 0., 1.] {
                        b.extend_from_slice(&v.to_le_bytes());
                    }
                }
                d["buffers"][0]["byteLength"] = json!(b.len());
                d["bufferViews"][0]["byteLength"] = json!(b.len());
                d["accessors"].as_array_mut().unwrap().push(json!({
                    "bufferView": 0, "byteOffset": offset, "componentType": 5126,
                    "count": 6, "type": "VEC3"
                }));
                d["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] = json!(1);
            },
        ),
        ("mixed-buffer-view", JobErrorKind::InvalidInput, |d, b| {
            let offset = b.len();
            for index in 0..6u16 {
                b.extend_from_slice(&index.to_le_bytes());
            }
            d["buffers"][0]["byteLength"] = json!(b.len());
            d["bufferViews"][0]["byteLength"] = json!(b.len());
            d["accessors"].as_array_mut().unwrap().push(json!({
                "bufferView": 0, "byteOffset": offset, "componentType": 5123,
                "count": 6, "type": "SCALAR"
            }));
            d["meshes"][0]["primitives"][0]["indices"] = json!(1);
        }),
        (
            "quaternion-component",
            JobErrorKind::InvalidInput,
            |d, _| d["nodes"][0]["rotation"] = json!([0, 0, 0, 1.0000005]),
        ),
        ("missing-version", JobErrorKind::InvalidInput, |d, _| {
            d["asset"].as_object_mut().unwrap().remove("version");
        }),
        ("missing-type", JobErrorKind::InvalidInput, |d, _| {
            d["accessors"][0].as_object_mut().unwrap().remove("type");
        }),
        ("unused-float-scalar", JobErrorKind::Unsupported, |d, _| {
            d["accessors"]
                .as_array_mut()
                .unwrap()
                .push(json!({"bufferView":0,"componentType":5126,"count":1,"type":"SCALAR"}));
        }),
        ("empty-materials", JobErrorKind::InvalidInput, |d, _| {
            d["materials"] = json!([])
        }),
        ("empty-children", JobErrorKind::InvalidInput, |d, _| {
            d["nodes"][0]["children"] = json!([])
        }),
        ("missing-bin", JobErrorKind::InvalidInput, |_, bin| {
            bin.clear()
        }),
        ("truncated-view", JobErrorKind::InvalidInput, |d, _| {
            d["bufferViews"][0]["byteLength"] = json!(1000)
        }),
        ("bad-count", JobErrorKind::InvalidInput, |d, _| {
            d["accessors"][0]["count"] = json!(7)
        }),
        ("bounds", JobErrorKind::InvalidInput, |d, _| {
            d["accessors"][0]["max"] = json!([1, 1, 0])
        }),
        ("nan", JobErrorKind::InvalidInput, |_, b| {
            b[0..4].copy_from_slice(&f32::NAN.to_le_bytes())
        }),
        ("normal-count", JobErrorKind::InvalidInput, |d, _| {
            let mut a = d["accessors"][0].clone();
            a["count"] = json!(3);
            d["accessors"].as_array_mut().unwrap().push(a);
            d["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] = json!(1);
        }),
        ("cycle", JobErrorKind::InvalidInput, |d, _| {
            d["nodes"][0]["children"] = json!([0])
        }),
        ("duplicate-root", JobErrorKind::InvalidInput, |d, _| {
            d["scenes"][0]["nodes"] = json!([0, 0])
        }),
        ("both-transforms", JobErrorKind::InvalidInput, |d, _| {
            d["nodes"][0]["matrix"] = json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
            d["nodes"][0]["scale"] = json!([1, 1, 1]);
        }),
        (
            "missing-texture-reference",
            JobErrorKind::InvalidInput,
            |d, _| d["materials"] = json!([{"normalTexture":{"index":0}}]),
        ),
        ("extras", JobErrorKind::Unsupported, |d, _| {
            d["nodes"][0]["extras"] = json!({"identity":42})
        }),
        ("unknown-extension", JobErrorKind::Unsupported, |d, _| {
            d["extensionsUsed"] = json!(["EXT_unknown"])
        }),
        ("uv1-non-vec2", JobErrorKind::InvalidInput, |d, _| {
            d["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_1"] = json!(0)
        }),
        ("blend", JobErrorKind::Unsupported, |d, _| {
            d["materials"] = json!([{"alphaMode":"BLEND"}])
        }),
        ("sparse", JobErrorKind::Unsupported, |d, _| {
            d["accessors"][0]["sparse"] = json!({})
        }),
        ("singular", JobErrorKind::Unsupported, |d, _| {
            d["nodes"][0]["scale"] = json!([0, 1, 1])
        }),
        ("domain", JobErrorKind::Unsupported, |d, _| {
            d["nodes"][0]["translation"] = json!([1e7, 0, 0])
        }),
        ("ambiguous", JobErrorKind::Unsupported, |d, _| {
            d.as_object_mut().unwrap().remove("scene");
            d["scenes"]
                .as_array_mut()
                .unwrap()
                .push(json!({"nodes":[0]}));
        }),
    ];
    let work = tempfile::tempdir().unwrap();
    for (name, kind, mutate) in cases {
        for limit in [1, 1000] {
            let (mut doc, mut bin) = source();
            mutate(&mut doc, &mut bin);
            let input = work.path().join("in.glb");
            fs::write(&input, encode(&doc, bin)).unwrap();
            let absent = work.path().join("absent");
            let output = absent.join("out.3tz");
            let error = mesh_to_archive(
                MeshRequest::local_gltf(&input, output, limit),
                &RunControl::default(),
            )
            .unwrap_err();
            assert_eq!(
                error.error.kind(),
                *kind,
                "{name}, leaf limit {limit}: {error}"
            );
            assert!(!absent.exists(), "{name} created output parent");
            assert!(error.retained_paths.is_empty());
        }
    }
}

struct Fail;
impl Observer for Fail {
    fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
        Err(JobError::new(JobErrorKind::ObserverFailure, "stop"))
    }
}

fn external_source(root: &Path) -> (std::path::PathBuf, std::path::PathBuf, usize) {
    let (mut document, bytes) = source();
    let resource = root.join("geometry.bin");
    fs::write(&resource, &bytes).unwrap();
    document["buffers"][0]["uri"] = json!("geometry.bin");
    let input = root.join("source.gltf");
    fs::write(&input, serde_json::to_vec(&document).unwrap()).unwrap();
    (input, resource, bytes.len())
}

#[test]
fn external_source_is_owned_before_callbacks_and_reentrant_attempts() {
    struct DeleteAndReenter {
        source: std::path::PathBuf,
        dependency: std::path::PathBuf,
        nested_source: std::path::PathBuf,
        nested_output: std::path::PathBuf,
        entered: std::sync::atomic::AtomicBool,
    }
    impl Observer for DeleteAndReenter {
        fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
            if !self.entered.swap(true, std::sync::atomic::Ordering::SeqCst) {
                fs::remove_file(&self.source).unwrap();
                fs::remove_file(&self.dependency).unwrap();
                let nested = mesh_to_archive(
                    MeshRequest::local_gltf(&self.nested_source, &self.nested_output, 2),
                    &RunControl::default(),
                )
                .unwrap();
                assert_eq!(nested.report.external_files, 1);
                assert_eq!(nested.report.leaf_tiles, 1);
            }
            Ok(())
        }
    }
    let work = tempfile::tempdir().unwrap();
    let first = work.path().join("first");
    let second = work.path().join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    let (input, dependency, bytes) = external_source(&first);
    let root_bytes = fs::metadata(&input).unwrap().len();
    let (nested_source, _, _) = external_source(&second);
    let nested_output = second.join("nested.3tz");
    let output = first.join("out.3tz");
    let run = RunControl::new(Some(Arc::new(DeleteAndReenter {
        source: input.clone(),
        dependency: dependency.clone(),
        nested_source,
        nested_output: nested_output.clone(),
        entered: std::sync::atomic::AtomicBool::new(false),
    })));
    let result = mesh_to_archive(MeshRequest::local_gltf(&input, &output, 1), &run).unwrap();
    assert_eq!(result.report.source_bytes, root_bytes);
    assert_eq!(result.report.external_files, 1);
    assert_eq!(result.report.external_bytes, bytes as u64);
    assert_eq!(result.report.triangles, 2);
    assert_eq!(result.report.leaf_tiles, 2);
    assert!(!input.exists());
    assert!(!dependency.exists());
    assert!(nested_output.exists());
    assert!(output.exists());
    assert_eq!(fs::read_dir(first).unwrap().count(), 1);
}

#[test]
fn unused_external_dependency_output_alias_is_rejected_before_observation() {
    struct Unexpected;
    impl Observer for Unexpected {
        fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
            panic!("source overlap reached observer")
        }
    }
    let work = tempfile::tempdir().unwrap();
    let (input, _, _) = external_source(work.path());
    let mut document: Value = serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
    document["buffers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"uri":"unused.3tz","byteLength":4}));
    fs::write(&input, serde_json::to_vec(&document).unwrap()).unwrap();
    let unused = work.path().join("unused.3tz");
    fs::write(&unused, b"keep").unwrap();
    let alias = work.path().join("alias.3tz");
    fs::hard_link(&unused, &alias).unwrap();
    for output in [&unused, &alias] {
        let failure = mesh_to_archive(
            MeshRequest::local_gltf(&input, output, 1).with_policy(OutputPolicy::Replace),
            &RunControl::new(Some(Arc::new(Unexpected))),
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
        assert!(failure.retained_paths.is_empty());
        assert_eq!(fs::read(output).unwrap(), b"keep");
    }
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 4);
}

#[test]
fn contradictory_accessor_bounds_are_rejected_before_missing_dependency_io() {
    let work = tempfile::tempdir().unwrap();
    for unused in [false, true] {
        let (input, dependency, _) = external_source(work.path());
        fs::remove_file(dependency).unwrap();
        let mut doc: Value = serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
        if unused {
            doc["accessors"].as_array_mut().unwrap().push(json!({
                "bufferView":0, "componentType":5126, "count":1, "type":"VEC2",
                "min":[2,0], "max":[1,1]
            }));
        } else {
            doc["accessors"][0]["min"] = json!([4, 0, 0]);
        }
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let output = work.path().join("absent/out.3tz");
        let failure = mesh_to_archive(
            MeshRequest::local_gltf(&input, &output, 1),
            &RunControl::default(),
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidInput);
        assert!(!output.parent().unwrap().exists());
        assert!(failure.retained_paths.is_empty());
    }
}

#[test]
fn enclosing_accessor_bounds_must_equal_actual_extrema_even_when_unused() {
    let work = tempfile::tempdir().unwrap();
    for unused in [false, true] {
        let (input, _, _) = external_source(work.path());
        let mut doc: Value = serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
        if unused {
            doc["accessors"].as_array_mut().unwrap().push(json!({
                "bufferView":0, "componentType":5126, "count":1, "type":"VEC2",
                "min":[-1,-1], "max":[1,1]
            }));
        } else {
            doc["accessors"][0]["max"] = json!([4, 1, 0]);
        }
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let output = work.path().join("absent/out.3tz");
        let failure = mesh_to_archive(
            MeshRequest::local_gltf(&input, &output, 1),
            &RunControl::default(),
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidInput);
        assert!(!output.parent().unwrap().exists());
        assert!(failure.retained_paths.is_empty());
    }
}
#[test]
fn observer_cancel_conflict_and_alias_preserve_source_and_destination() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("in.glb");
    write_source(&input);
    let output = work.path().join("out.3tz");
    fs::write(&output, b"previous").unwrap();
    let request = || MeshRequest::local_gltf(&input, &output, 1).with_policy(OutputPolicy::Replace);
    let run = RunControl::new(Some(Arc::new(Fail)));
    assert_eq!(
        mesh_to_archive(request(), &run).unwrap_err().error.kind(),
        JobErrorKind::ObserverFailure
    );
    let run = RunControl::default();
    assert!(run.cancellation_handle().cancel());
    assert_eq!(
        mesh_to_archive(request(), &run).unwrap_err().error.kind(),
        JobErrorKind::Cancelled
    );
    assert_eq!(
        mesh_to_archive(
            MeshRequest::local_gltf(&input, &output, 1),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::Conflict
    );
    assert_eq!(fs::read(&output).unwrap(), b"previous");
    let alias = work.path().join("alias.3tz");
    fs::hard_link(&input, &alias).unwrap();
    let before = fs::read(&input).unwrap();
    let failure = mesh_to_archive(
        MeshRequest::local_gltf(&input, &alias, 1).with_policy(OutputPolicy::Replace),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
    assert_eq!(fs::read(input).unwrap(), before);
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 3);
}

#[test]
fn concurrent_create_new_has_one_winner_and_independent_reports() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("in.glb");
    write_source(&input);
    let output = work.path().join("out.3tz");
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    mesh_to_archive(
                        MeshRequest::local_gltf(&input, &output, 1),
                        &RunControl::default(),
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    for result in results {
        match result {
            Ok(r) => assert_eq!(r.report.triangles, 2),
            Err(e) => assert_eq!(e.error.kind(), JobErrorKind::Conflict),
        }
    }
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
}

#[test]
fn callback_chdir_cannot_redirect_bound_output() {
    const CHILD: &str = "RUSTY_TILES_F1A_CWD_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "callback_chdir_cannot_redirect_bound_output",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    struct Chdir(std::path::PathBuf);
    impl Observer for Chdir {
        fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
            std::env::set_current_dir(&self.0).unwrap();
            Ok(())
        }
    }
    let work = tempfile::tempdir().unwrap();
    let original = fs::canonicalize(work.path()).unwrap();
    let other = original.join("other");
    fs::create_dir(&other).unwrap();
    let input = original.join("in.glb");
    write_source(&input);
    fs::write(other.join("out.3tz"), b"protected").unwrap();
    std::env::set_current_dir(&original).unwrap();
    let run = RunControl::new(Some(Arc::new(Chdir(other.clone()))));
    let result = mesh_to_archive(
        MeshRequest::local_gltf("in.glb", "out.3tz", 1).with_policy(OutputPolicy::Replace),
        &run,
    )
    .unwrap();
    assert_eq!(result.output, original.join("out.3tz"));
    assert_eq!(fs::read(other.join("out.3tz")).unwrap(), b"protected");
    std::env::set_current_dir(std::env::temp_dir()).unwrap();
}

#[test]
fn placement_admission_precedes_source_io_and_cannot_replace_prior_output() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("missing.glb");
    let existing = work.path().join("prior.3tz");
    fs::write(&existing, b"previous artifact").unwrap();
    for (anchor, orientation, offset, expected) in [
        (
            [181., 0., 0.],
            [0., 0., 0., 1.],
            [0.; 3],
            JobErrorKind::InvalidRequest,
        ),
        ([0.; 3], [0.; 4], [0.; 3], JobErrorKind::InvalidRequest),
        (
            [0., 0., f64::NAN],
            [0., 0., 0., 1.],
            [0.; 3],
            JobErrorKind::InvalidRequest,
        ),
        (
            [0., 0., 60_000_000.],
            [0., 0., 0., 1.],
            [0.; 3],
            JobErrorKind::Unsupported,
        ),
    ] {
        for output in [&existing, &work.path().join("absent/out.3tz")] {
            let failure = mesh_to_archive(
                MeshRequest::local_gltf(&input, output, 1)
                    .with_policy(OutputPolicy::Replace)
                    .with_placement(MeshPlacement::Wgs84 {
                        anchor_degrees_metres: anchor,
                        orientation_xyzw: orientation,
                        scene_offset_metres: offset,
                    }),
                &RunControl::default(),
            )
            .unwrap_err();
            assert_eq!(failure.error.kind(), expected);
            assert!(failure.secondary.is_empty());
            assert!(failure.retained_paths.is_empty());
            assert!(failure.recovery.is_none());
            assert_eq!(fs::read(&existing).unwrap(), b"previous artifact");
            assert!(!work.path().join("absent").exists());
        }
    }
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 1);
}

#[test]
fn placement_is_per_request_and_keeps_encoded_geometry_local() {
    use std::io::Read;
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.glb");
    write_source(&input);
    let placements = [
        MeshPlacement::Local,
        MeshPlacement::Wgs84 {
            anchor_degrees_metres: [153., -27., 42.],
            orientation_xyzw: [0., 0., 0., 1.],
            scene_offset_metres: [5., 7., 11.],
        },
        MeshPlacement::Wgs84 {
            anchor_degrees_metres: [73., 90., 42.],
            orientation_xyzw: [1., 0., 0., 0.],
            scene_offset_metres: [0.; 3],
        },
    ];
    let results = std::thread::scope(|scope| {
        placements
            .into_iter()
            .enumerate()
            .map(|(index, placement)| {
                let input = &input;
                let output = work.path().join(format!("placed-{index}.3tz"));
                scope.spawn(move || {
                    mesh_to_archive(
                        MeshRequest::local_gltf(input, output, 1).with_placement(placement),
                        &RunControl::default(),
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let mut geometry = Vec::new();
    for result in &results {
        assert_eq!(result.report.schema_version, 4);
        assert_eq!(result.report.profile, "f1c1-placed-gltf-v1");
        assert_eq!(result.report.source_coordinates, "local-gltf");
        let mut archive = zip::ZipArchive::new(fs::File::open(&result.output).unwrap()).unwrap();
        let manifest: Value =
            serde_json::from_reader(archive.by_name("tileset.json").unwrap()).unwrap();
        assert_eq!(
            manifest["root"]["transform"],
            json!(result.report.root_transform)
        );
        let report: Value =
            serde_json::from_reader(archive.by_name("conversion.json").unwrap()).unwrap();
        assert_eq!(report, json!(result.report));
        let mut leaves = std::collections::BTreeMap::new();
        for index in 0..archive.len() {
            let mut member = archive.by_index(index).unwrap();
            if member.name().ends_with(".glb") {
                let mut bytes = Vec::new();
                member.read_to_end(&mut bytes).unwrap();
                leaves.insert(member.name().to_owned(), bytes);
            }
        }
        geometry.push(leaves);
    }
    assert_eq!(geometry[0], geometry[1]);
    assert_eq!(geometry[0], geometry[2]);
    assert_eq!(results[0].report.coordinates, "local-gltf");
    assert!(results[1..]
        .iter()
        .all(|result| result.report.coordinates == "wgs84-ecef"));
    assert_ne!(
        results[0].report.root_transform,
        results[1].report.root_transform
    );
    assert_ne!(
        results[1].report.root_transform,
        results[2].report.root_transform
    );
}

#[test]
fn frozen_independent_cartographic_and_rational_frames_hold_on_this_target() {
    // Authored independently before candidate execution (Decimal80/cardinals
    // and exact Hamilton action); no production coordinate helper supplies truth.
    let references: Value =
        serde_json::from_str(include_str!("fixtures/f1c1/analytic-references.json")).unwrap();
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.glb");
    write_source(&input);
    let rotations = [
        ([0., 0., 0., 1.], [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]),
        (
            [1., 2., 3., 4.].map(|x| x / 30_f64.sqrt()),
            [
                [2. / 15., -2. / 3., 11. / 15.],
                [14. / 15., 1. / 3., 2. / 15.],
                [-1. / 3., 2. / 3., 2. / 3.],
            ],
        ),
    ];
    for (index, anchor) in references["anchors"].as_array().unwrap().iter().enumerate() {
        let parse = |value: &Value| value.as_str().unwrap().parse::<f64>().unwrap();
        let parameters = std::array::from_fn(|i| parse(&anchor["anchor_strings"][i]));
        let origin: [f64; 3] = std::array::from_fn(|i| parse(&anchor["origin_ecef_decimal"][i]));
        let columns: [[f64; 3]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| parse(&anchor["enu_columns_decimal"][i][j]))
        });
        for (orientation_index, (quaternion, rotation)) in rotations.into_iter().enumerate() {
            let expected: [[f64; 3]; 3] = std::array::from_fn(|row| {
                std::array::from_fn(|column| {
                    (0..3)
                        .map(|axis| columns[axis][row] * rotation[axis][column])
                        .sum()
                })
            });
            let result = mesh_to_archive(
                MeshRequest::local_gltf(
                    &input,
                    work.path().join(format!("{index}-{orientation_index}.3tz")),
                    1,
                )
                .with_placement(MeshPlacement::Wgs84 {
                    anchor_degrees_metres: parameters,
                    orientation_xyzw: quaternion,
                    scene_offset_metres: [5., 7., 11.],
                }),
                &RunControl::default(),
            )
            .unwrap();
            let matrix = result.report.root_transform;
            for row in 0..3 {
                for column in 0..3 {
                    assert!(
                        (matrix[4 * column + row] - expected[row][column]).abs() <= 1e-14,
                        "{} orientation {orientation_index}",
                        anchor["name"]
                    );
                }
                let translation = origin[row] + expected[row][0] * 5. - expected[row][1] * 11.
                    + expected[row][2] * 7.;
                assert!(
                    (matrix[12 + row] - translation).abs() <= 1e-6,
                    "{} orientation {orientation_index}",
                    anchor["name"]
                );
            }
            assert_eq!(
                [matrix[3], matrix[7], matrix[11], matrix[15]],
                [0., 0., 0., 1.]
            );
        }
    }
}
