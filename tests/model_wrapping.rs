//! External W1 consumers: exact member associations, original bounds and F0.
use rusty_tiles::{
    model_to_archive, model_to_manifest, JobError, JobErrorKind, MeshPlacement,
    ModelManifestRequest, ModelWrapRequest, Observer, OutputPolicy, RunControl, RunEvent,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
};

fn source() -> (Value, Vec<u8>) {
    let bytes = rusty_tiles::fixtures::triangle_glb();
    let glb = gltf::binary::Glb::from_slice(&bytes).unwrap();
    (
        serde_json::from_slice(&glb.json).unwrap(),
        glb.bin.unwrap().into_owned(),
    )
}
fn external(root: &Path, name: &str, uris: &[&str]) -> (PathBuf, Vec<u8>, Vec<u8>) {
    let (mut document, bin) = source();
    document["buffers"] = Value::Array(
        uris.iter()
            .map(|uri| json!({"byteLength":36,"uri":uri}))
            .collect(),
    );
    let bytes = serde_json::to_vec_pretty(&document).unwrap();
    let input = root.join(name);
    fs::write(&input, &bytes).unwrap();
    (input, bytes, bin)
}
fn member(output: &Path, name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}
fn manifest(output: &Path) -> Value {
    serde_json::from_slice(&member(output, "tileset.json")).unwrap()
}

#[test]
fn percent_names_and_distinct_file_aliases_keep_all_original_uri_associations() {
    let work = tempfile::tempdir().unwrap();
    let (input, root, bin) = external(
        work.path(),
        "original.gltf",
        &[
            "space%20geometry.bin",
            "other.bin",
            "./folder/../space%20geometry.bin",
        ],
    );
    fs::write(work.path().join("space geometry.bin"), &bin).unwrap();
    fs::hard_link(
        work.path().join("space geometry.bin"),
        work.path().join("other.bin"),
    )
    .unwrap();
    let output = work.path().join("model.3tz");
    let result = model_to_archive(
        ModelWrapRequest::local_gltf(&input, &output),
        &RunControl::default(),
    )
    .unwrap();
    assert_eq!(member(&output, "model/source.gltf"), root);
    assert_eq!(member(&output, "model/space geometry.bin"), bin);
    assert_eq!(member(&output, "model/other.bin"), bin);
    assert_eq!(result.report.external_files, 1);
    assert_eq!(result.report.external_bytes, 36);
    assert_eq!(result.report.model_payload_files, 3);
    assert_eq!(result.report.model_payload_bytes, root.len() as u64 + 72);
    rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(output)).unwrap();
}

#[test]
fn sibling_manifest_escapes_its_original_filename_and_keeps_resources_in_place() {
    let work = tempfile::tempdir().unwrap();
    let (input, root, bin) = external(work.path(), "m%C3%A9 # plus+.gltf", &["data.bin"]);
    fs::write(work.path().join("data.bin"), &bin).unwrap();
    let result = model_to_manifest(
        ModelManifestRequest::local_gltf(&input),
        &RunControl::default(),
    )
    .unwrap();
    let manifest: Value = serde_json::from_slice(&fs::read(&result.output).unwrap()).unwrap();
    assert_eq!(
        manifest["root"]["content"]["uri"],
        "m%25C3%25A9%20%23%20plus%2B.gltf"
    );
    assert_eq!(fs::read(input).unwrap(), root);
    assert_eq!(fs::read(work.path().join("data.bin")).unwrap(), bin);
    assert_eq!(result.report.product, "manifest");
    assert_eq!(result.report.model_payload_files, 0);
    assert!(!work.path().join("conversion.json").exists());
}

#[test]
fn original_unrounded_node_translation_is_inside_wrapped_bounds() {
    let work = tempfile::tempdir().unwrap();
    let (mut doc, bin) = source();
    doc["nodes"][0]["translation"] = json!([0.1, 0., 0.]);
    let input = work.path().join("original.glb");
    let original = gltf::binary::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: 0,
        },
        json: serde_json::to_vec(&doc).unwrap().into(),
        bin: Some(bin.into()),
    }
    .to_vec()
    .unwrap();
    fs::write(&input, &original).unwrap();
    let output = work.path().join("model.3tz");
    let result = model_to_archive(
        ModelWrapRequest::local_gltf(input, &output),
        &RunControl::default(),
    )
    .unwrap();
    let b = result.report.bounding_box;
    assert!(b[0] - b[3] <= 0.1);
    assert!(b[0] + b[3] >= 1.1);
    // Casting the first authored transformed coordinate to f32 shifts it inward.
    assert!(f64::from(0.1_f32) > 0.1);
    assert_eq!(member(&output, "model/source.glb"), original);
}

#[test]
fn tileset_omission_error_is_positive_and_separate_from_full_detail_root() {
    for shape in ["triangle", "sub-metre", "degenerate"] {
        for is_manifest in [false, true] {
            let work = tempfile::tempdir().unwrap();
            let (mut doc, mut bin) = source();
            if shape == "sub-metre" {
                doc["nodes"][0]["scale"] = json!([0.25, 0.25, 0.25]);
            } else if shape == "degenerate" {
                bin.fill(0);
                doc["accessors"][0]["min"] = json!([0, 0, 0]);
                doc["accessors"][0]["max"] = json!([0, 0, 0]);
            }
            let input = work.path().join("original.glb");
            fs::write(
                &input,
                gltf::binary::Glb {
                    header: gltf::binary::Header {
                        magic: *b"glTF",
                        version: 2,
                        length: 0,
                    },
                    json: serde_json::to_vec(&doc).unwrap().into(),
                    bin: Some(bin.into()),
                }
                .to_vec()
                .unwrap(),
            )
            .unwrap();
            let (document, report) = if is_manifest {
                let result = model_to_manifest(
                    ModelManifestRequest::local_gltf(&input),
                    &RunControl::default(),
                )
                .unwrap();
                (
                    serde_json::from_slice::<Value>(&fs::read(result.output).unwrap()).unwrap(),
                    result.report,
                )
            } else {
                let output = work.path().join("model.3tz");
                let result = model_to_archive(
                    ModelWrapRequest::local_gltf(&input, &output),
                    &RunControl::default(),
                )
                .unwrap();
                assert_eq!(
                    serde_json::from_slice::<Value>(&member(&output, "conversion.json")).unwrap(),
                    serde_json::to_value(&result.report).unwrap()
                );
                (manifest(&output), result.report)
            };
            assert_eq!(document["root"]["geometricError"], 0.0);
            assert_eq!(document["root"]["refine"], "REPLACE");
            assert_eq!(report.root_geometric_error_metres, 0.0);
            assert_eq!(
                document["geometricError"],
                report.tileset_geometric_error_metres
            );
            if shape == "triangle" {
                assert!(report.tileset_geometric_error_metres >= 2.0_f64.sqrt());
                assert!(report.tileset_geometric_error_metres < 2.0);
            } else {
                assert_eq!(report.tileset_geometric_error_metres, 1.0);
            }
        }
    }
}

#[test]
fn synthetic_root_file_cannot_be_a_dependency_directory_in_either_envelope() {
    for is_glb in [false, true] {
        let root_name = if is_glb { "source.glb" } else { "source.gltf" };
        for suffix in ["", "/data.bin", "/nested/data.bin"] {
            let work = tempfile::tempdir().unwrap();
            let uri = format!("{root_name}{suffix}");
            let (input, _, _) = external(work.path(), "original.gltf", &[&uri]);
            if is_glb {
                let doc = fs::read(&input).unwrap();
                fs::write(
                    &input,
                    gltf::binary::Glb {
                        header: gltf::binary::Header {
                            magic: *b"glTF",
                            version: 2,
                            length: 0,
                        },
                        json: doc.into(),
                        bin: None,
                    }
                    .to_vec()
                    .unwrap(),
                )
                .unwrap();
            }
            let resource = work.path().join(&uri);
            fs::create_dir_all(resource.parent().unwrap()).unwrap();
            // Invalid POSITION floats would fail report evaluation. Inventory
            // must own this collision first, before geometry or observers.
            fs::write(&resource, [0xff; 36]).unwrap();
            let absent = work.path().join("absent");
            let failure = model_to_archive(
                ModelWrapRequest::local_gltf(&input, absent.join("out.3tz")),
                &RunControl::new(Some(Arc::new(Stop))),
            )
            .unwrap_err();
            assert_eq!(failure.error.kind(), JobErrorKind::Unsupported);
            assert!(failure.error.message().contains("generated source member"));
            assert!(!absent.exists());
            assert!(failure.retained_paths.is_empty());
            assert!(fs::read_dir(work.path()).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with('.')));
        }
    }
}

#[test]
fn shared_synthetic_root_name_prefix_without_path_boundary_is_admitted() {
    let work = tempfile::tempdir().unwrap();
    let uri = "source.gltf-extra/data.bin";
    let (input, original, bin) = external(work.path(), "original.gltf", &[uri]);
    let resource = work.path().join(uri);
    fs::create_dir_all(resource.parent().unwrap()).unwrap();
    fs::write(resource, &bin).unwrap();
    let output = work.path().join("model.3tz");
    model_to_archive(
        ModelWrapRequest::local_gltf(input, &output),
        &RunControl::default(),
    )
    .unwrap();
    assert_eq!(member(&output, "model/source.gltf"), original);
    assert_eq!(member(&output, &format!("model/{uri}")), bin);
}

#[test]
fn synthetic_source_name_collision_and_data_uri_are_explicit_refusals() {
    for (uri, create) in [
        ("source.gltf", true),
        ("data:application/octet-stream;base64,AAAA", false),
    ] {
        let work = tempfile::tempdir().unwrap();
        let (input, _, bin) = external(work.path(), "original.gltf", &[uri]);
        if create {
            fs::write(work.path().join(uri), bin).unwrap();
        }
        let absent = work.path().join("absent");
        let error = model_to_archive(
            ModelWrapRequest::local_gltf(input, absent.join("out.3tz")),
            &RunControl::default(),
        )
        .unwrap_err();
        assert_eq!(error.error.kind(), JobErrorKind::Unsupported);
        assert!(!absent.exists());
    }
}

#[test]
fn conservative_bounds_outside_local_component_profile_are_refused_before_output_work() {
    let work = tempfile::tempdir().unwrap();
    let (mut doc, bin) = source();
    // Raw near-unit quaternion and accumulated interval uncertainty push the
    // unchanged envelope beyond a cap that a rounded/normalized bake may admit.
    doc["nodes"][0]["translation"] = json!([999_999., 0., 0.]);
    let input = work.path().join("boundary.glb");
    let source = gltf::binary::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: 0,
        },
        json: serde_json::to_vec(&doc).unwrap().into(),
        bin: Some(bin.into()),
    }
    .to_vec()
    .unwrap();
    fs::write(&input, source).unwrap();
    let absent = work.path().join("absent");
    let failure = model_to_archive(
        ModelWrapRequest::local_gltf(input, absent.join("out.3tz")),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::Unsupported);
    assert!(!absent.exists());
}

struct MutateAfterCapture {
    input: PathBuf,
    resource: PathBuf,
}
impl Observer for MutateAfterCapture {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        if matches!(
            event,
            RunEvent::Progress {
                phase: "model_capture",
                ..
            }
        ) {
            fs::write(&self.input, b"changed document").unwrap();
            fs::write(&self.resource, b"changed resource").unwrap();
        }
        Ok(())
    }
}
#[test]
fn archive_uses_immutable_capture_after_observer_mutates_live_sources() {
    let work = tempfile::tempdir().unwrap();
    let (input, root, bin) = external(work.path(), "model.gltf", &["data.bin"]);
    let resource = work.path().join("data.bin");
    fs::write(&resource, &bin).unwrap();
    let output = work.path().join("output.3tz");
    let control = RunControl::new(Some(Arc::new(MutateAfterCapture {
        input: input.clone(),
        resource,
    })));
    model_to_archive(ModelWrapRequest::local_gltf(input, &output), &control).unwrap();
    assert_eq!(member(&output, "model/source.gltf"), root);
    assert_eq!(member(&output, "model/data.bin"), bin);
}

struct Stop;
impl Observer for Stop {
    fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
        Err(JobError::new(
            JobErrorKind::ObserverFailure,
            "intentional observer control",
        ))
    }
}
#[test]
fn observation_cancellation_and_control_reuse_preserve_previous_outputs() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.glb");
    fs::write(&input, rusty_tiles::fixtures::triangle_glb()).unwrap();
    for is_manifest in [false, true] {
        let output = work.path().join(if is_manifest {
            "tileset.json"
        } else {
            "out.3tz"
        });
        fs::write(&output, b"KEEP").unwrap();
        let control = RunControl::new(Some(Arc::new(Stop)));
        let failure = if is_manifest {
            model_to_manifest(
                ModelManifestRequest::local_gltf(&input).with_policy(OutputPolicy::Replace),
                &control,
            )
            .unwrap_err()
        } else {
            model_to_archive(
                ModelWrapRequest::local_gltf(&input, &output).with_policy(OutputPolicy::Replace),
                &control,
            )
            .unwrap_err()
        };
        assert_eq!(failure.error.kind(), JobErrorKind::ObserverFailure);
        assert_eq!(fs::read(&output).unwrap(), b"KEEP");
        let reused = if is_manifest {
            model_to_manifest(ModelManifestRequest::local_gltf(&input), &control).unwrap_err()
        } else {
            model_to_archive(ModelWrapRequest::local_gltf(&input, &output), &control).unwrap_err()
        };
        assert_eq!(reused.error.kind(), JobErrorKind::InvalidState);
        let cancelled = RunControl::default();
        cancelled.cancellation_handle().cancel();
        let failure = if is_manifest {
            model_to_manifest(ModelManifestRequest::local_gltf(&input), &cancelled).unwrap_err()
        } else {
            model_to_archive(ModelWrapRequest::local_gltf(&input, &output), &cancelled).unwrap_err()
        };
        assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
        assert_eq!(fs::read(output).unwrap(), b"KEEP");
    }
    assert!(fs::read_dir(work.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with('.')));
}

#[test]
fn source_output_aliases_are_rejected_even_when_replacement_requested() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.glb");
    let original = rusty_tiles::fixtures::triangle_glb();
    fs::write(&input, &original).unwrap();
    let output = work.path().join("output.3tz");
    fs::hard_link(&input, &output).unwrap();
    let failure = model_to_archive(
        ModelWrapRequest::local_gltf(&input, &output).with_policy(OutputPolicy::Replace),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
    fs::hard_link(&input, work.path().join("tileset.json")).unwrap();
    let failure = model_to_manifest(
        ModelManifestRequest::local_gltf(&input).with_policy(OutputPolicy::Replace),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
    assert_eq!(fs::read(input).unwrap(), original);
}

struct ReadyTogether(Arc<Barrier>);
impl Observer for ReadyTogether {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        if matches!(
            event,
            RunEvent::Progress {
                phase: "ready_to_publish",
                ..
            }
        ) {
            self.0.wait();
        }
        Ok(())
    }
}
#[test]
fn concurrent_create_new_has_one_complete_winner_for_each_product() {
    for is_manifest in [false, true] {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("source.glb");
        fs::write(&input, rusty_tiles::fixtures::triangle_glb()).unwrap();
        let output = work.path().join(if is_manifest {
            "tileset.json"
        } else {
            "out.3tz"
        });
        let barrier = Arc::new(Barrier::new(2));
        let jobs = (0..2)
            .map(|_| {
                let input = input.clone();
                let output = output.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let control = RunControl::new(Some(Arc::new(ReadyTogether(barrier))));
                    if is_manifest {
                        model_to_manifest(ModelManifestRequest::local_gltf(input), &control)
                            .map(|_| ())
                    } else {
                        model_to_archive(ModelWrapRequest::local_gltf(input, output), &control)
                            .map(|_| ())
                    }
                })
            })
            .collect::<Vec<_>>();
        let results = jobs
            .into_iter()
            .map(|job| job.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .into_iter()
                .find_map(Result::err)
                .unwrap()
                .error
                .kind(),
            JobErrorKind::Conflict
        );
        let root = if is_manifest {
            serde_json::from_slice::<Value>(&fs::read(output).unwrap()).unwrap()
        } else {
            manifest(&output)
        };
        assert!(root["geometricError"].as_f64().unwrap() >= 1.0);
        assert_eq!(root["root"]["geometricError"], 0.0);
        assert_eq!(root["root"]["refine"], "REPLACE");
    }
}

#[test]
fn pure_placement_errors_precede_missing_source_io_for_both_products() {
    let bad = MeshPlacement::Wgs84 {
        anchor_degrees_metres: [0., 91., 0.],
        orientation_xyzw: [0., 0., 0., 1.],
        scene_offset_metres: [0.; 3],
    };
    let failure = model_to_archive(
        ModelWrapRequest::local_gltf("missing.glb", "absent/out.3tz").with_placement(bad.clone()),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
    let failure = model_to_manifest(
        ModelManifestRequest::local_gltf("missing.glb").with_placement(bad),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
}

#[test]
fn emitted_alias_ceiling_counts_distinct_names_instead_of_unique_captured_bytes() {
    let work = tempfile::tempdir().unwrap();
    let (input, _, bin) = external(work.path(), "model.gltf", &["a.bin", "b.bin", "c.bin"]);
    fs::write(work.path().join("a.bin"), bin).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(work.path().join("a.bin"))
        .unwrap()
        .set_len(22 * 1024 * 1024)
        .unwrap();
    for name in ["b.bin", "c.bin"] {
        fs::hard_link(work.path().join("a.bin"), work.path().join(name)).unwrap();
    }
    let absent = work.path().join("absent");
    let failure = model_to_archive(
        ModelWrapRequest::local_gltf(input, absent.join("out.3tz")),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::Unsupported);
    assert!(failure.error.message().contains("emitted alias payload"));
    assert!(!absent.exists());
}
