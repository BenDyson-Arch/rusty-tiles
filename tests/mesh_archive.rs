//! External facade consumer: source admission and lifecycle, without private imports.
use rusty_tiles::{
    mesh_to_archive, JobError, JobErrorKind, MeshRequest, Observer, OutputPolicy, RunControl,
    RunEvent,
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
        ("texture", JobErrorKind::Unsupported, |d, _| {
            d["textures"] = json!([{}])
        }),
        ("extras", JobErrorKind::Unsupported, |d, _| {
            d["nodes"][0]["extras"] = json!({"identity":42})
        }),
        ("unknown-extension", JobErrorKind::Unsupported, |d, _| {
            d["extensionsUsed"] = json!(["EXT_unknown"])
        }),
        ("uv", JobErrorKind::Unsupported, |d, _| {
            d["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = json!(0)
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
