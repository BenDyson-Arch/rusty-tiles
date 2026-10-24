//! Independent public-facade lifecycle/alias controls; no diagnostic-stage API.
use rusty_tiles::{
    mesh_to_tileset, CancellationHandle, JobError, JobErrorKind, Observer, OutputPolicy,
    RunControl, RunEvent, TilerJobs, TilerRequest,
};
use serde_json::json;
use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

fn fixture(root: &Path) -> (PathBuf, PathBuf) {
    let input = root.join("mesh.gltf");
    let dependency = root.join("positions.bin");
    let bytes = [0f32, 0., 0., 1., 0., 0., 0., 1., 0.]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    fs::write(&dependency, bytes).unwrap();
    fs::write(&input, serde_json::to_vec(&json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
        "buffers":[{"uri":"positions.bin","byteLength":36}],
        "bufferViews":[{"buffer":0,"byteLength":36}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}]
    })).unwrap()).unwrap();
    (input, dependency)
}

fn request(input: &Path, output: &Path, scratch: &Path) -> TilerRequest {
    TilerRequest::new(input, output)
        .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(1).unwrap()))
        .with_scratch_dir(scratch)
}

fn assert_empty(scratch: &Path) {
    assert_eq!(
        scratch.read_dir().unwrap().count(),
        0,
        "owned scratch must be cleaned"
    );
}

#[test]
fn source_and_buffer_identity_aliases_are_refused_before_capture() {
    let work = tempfile::tempdir().unwrap();
    let (input, dependency) = fixture(work.path());
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    for (name, original) in [("source.3tz", &input), ("buffer.3tz", &dependency)] {
        let output = work.path().join(name);
        fs::hard_link(original, &output).unwrap();
        let before = fs::read(original).unwrap();
        let failure = mesh_to_tileset(
            request(&input, &output, &scratch).with_output_policy(OutputPolicy::Replace),
            &RunControl::default(),
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
        assert_eq!(fs::read(original).unwrap(), before);
        assert_eq!(fs::read(&output).unwrap(), before);
        assert!(failure.retained_paths.is_empty());
        assert_empty(&scratch);
        fs::remove_file(output).unwrap();
    }
}

#[test]
fn image_dependency_alias_is_refused_even_when_not_used_by_a_material() {
    let work = tempfile::tempdir().unwrap();
    let (input, _) = fixture(work.path());
    // A valid 1x1 PNG, unused but declared, is part of the confined source snapshot.
    let image = work.path().join("unused.png");
    let png = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5,
        1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    fs::write(&image, png).unwrap();
    let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
    doc["images"] = json!([{"uri":"unused.png"}]);
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let output = work.path().join("image.3tz");
    fs::hard_link(&image, &output).unwrap();
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let failure = mesh_to_tileset(
        request(&input, &output, &scratch).with_output_policy(OutputPolicy::Replace),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
    assert_eq!(fs::read(image).unwrap(), png);
    assert_eq!(fs::read(output).unwrap(), png);
    assert_empty(&scratch);
}

#[test]
fn precancel_and_run_reuse_precede_validation_io_and_publication() {
    let work = tempfile::tempdir().unwrap();
    let missing = work.path().join("missing.glb");
    let output = work.path().join("unsupported.zip");
    fs::write(&output, b"previous").unwrap();
    let run = RunControl::default();
    assert!(run.cancellation_handle().cancel());
    let first = mesh_to_tileset(TilerRequest::new(&missing, &output), &run).unwrap_err();
    assert_eq!(first.error.kind(), JobErrorKind::Cancelled);
    let second = mesh_to_tileset(TilerRequest::new(&missing, &output), &run).unwrap_err();
    assert_eq!(second.error.kind(), JobErrorKind::InvalidState);
    assert_eq!(fs::read(&output).unwrap(), b"previous");
    assert_eq!(work.path().read_dir().unwrap().count(), 1);

    let run = RunControl::default();
    assert_eq!(
        mesh_to_tileset(TilerRequest::new(&missing, &output), &run)
            .unwrap_err()
            .error
            .kind(),
        JobErrorKind::InvalidRequest
    );
    assert_eq!(
        mesh_to_tileset(
            TilerRequest::new(&missing, work.path().join("valid.3tz")),
            &run
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidState
    );
}

struct ObserverFn<F>(F);
impl<F: Fn(&RunEvent<'_>) -> Result<(), JobError> + Send + Sync> Observer for ObserverFn<F> {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        (self.0)(event)
    }
}

fn control<F>(observer: F) -> RunControl
where
    F: Fn(&RunEvent<'_>) -> Result<(), JobError> + Send + Sync + 'static,
{
    RunControl::new(Some(Arc::new(ObserverFn(observer))))
}

#[test]
fn final_observer_failure_preserves_destination_and_cleans_owned_scratch() {
    let work = tempfile::tempdir().unwrap();
    let (input, dependency) = fixture(work.path());
    let source_before = fs::read(&input).unwrap();
    let buffer_before = fs::read(&dependency).unwrap();
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let output = work.path().join("output.3tz");
    fs::write(&output, b"old output").unwrap();
    let raced_output = output.clone();
    let fired = Arc::new(AtomicUsize::new(0));
    let counted = fired.clone();
    let run = control(move |event| {
        if matches!(
            event,
            RunEvent::Progress {
                phase: "ready_to_publish",
                ..
            }
        ) {
            assert_eq!(fs::read(&raced_output).unwrap(), b"old output");
            counted.fetch_add(1, Ordering::SeqCst);
            return Err(JobError::new(
                JobErrorKind::Io,
                "independent final observer cause",
            ));
        }
        Ok(())
    });
    let failure = mesh_to_tileset(
        request(&input, &output, &scratch).with_output_policy(OutputPolicy::Replace),
        &run,
    )
    .unwrap_err();
    assert_eq!(fired.load(Ordering::SeqCst), 1);
    assert_eq!(failure.error.kind(), JobErrorKind::ObserverFailure);
    assert!(failure
        .error
        .to_string()
        .contains("independent final observer cause"));
    assert!(failure.retained_paths.is_empty());
    assert_eq!(fs::read(&output).unwrap(), b"old output");
    assert_eq!(fs::read(input).unwrap(), source_before);
    assert_eq!(fs::read(dependency).unwrap(), buffer_before);
    assert_empty(&scratch);
    assert_eq!(work.path().read_dir().unwrap().count(), 4);
}

#[test]
fn competing_destination_at_final_callback_is_preserved_by_create_new() {
    let work = tempfile::tempdir().unwrap();
    let (input, _) = fixture(work.path());
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let output = work.path().join("output.3tz");
    let competitor = output.clone();
    let run = control(move |event| {
        if matches!(
            event,
            RunEvent::Progress {
                phase: "ready_to_publish",
                ..
            }
        ) {
            fs::write(&competitor, b"competitor").unwrap();
        }
        Ok(())
    });
    let failure = mesh_to_tileset(request(&input, &output, &scratch), &run).unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::Conflict);
    assert_eq!(fs::read(output).unwrap(), b"competitor");
    assert!(failure.retained_paths.is_empty());
    assert_empty(&scratch);
}

#[test]
fn successful_run_cannot_be_reused_or_cancelled_after_installation() {
    let work = tempfile::tempdir().unwrap();
    let (input, _) = fixture(work.path());
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let output = work.path().join("output.3tz");
    let run = RunControl::default();
    let cancel = run.cancellation_handle();
    let result = mesh_to_tileset(request(&input, &output, &scratch), &run).unwrap();
    assert_eq!(result.report.triangles, 1);
    assert!(!cancel.cancel());
    let installed = fs::read(&output).unwrap();
    let failure = mesh_to_tileset(
        request(&input, &output, &scratch).with_output_policy(OutputPolicy::Replace),
        &run,
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidState);
    assert_eq!(fs::read(output).unwrap(), installed);
    assert_empty(&scratch);
}

#[test]
fn cancellation_wins_when_an_observer_cancels_then_returns_a_second_failure() {
    let work = tempfile::tempdir().unwrap();
    let (input, _) = fixture(work.path());
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let output = work.path().join("output.3tz");
    fs::write(&output, b"old output").unwrap();
    let handle = Arc::new(Mutex::new(None::<CancellationHandle>));
    let observer_handle = handle.clone();
    let run = control(move |event| {
        if matches!(
            event,
            RunEvent::Progress {
                phase: "ready_to_publish",
                ..
            }
        ) {
            assert!(observer_handle.lock().unwrap().as_ref().unwrap().cancel());
            return Err(JobError::new(JobErrorKind::Io, "later observer cause"));
        }
        Ok(())
    });
    *handle.lock().unwrap() = Some(run.cancellation_handle());
    let failure = mesh_to_tileset(
        request(&input, &output, &scratch).with_output_policy(OutputPolicy::Replace),
        &run,
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
    assert!(failure
        .secondary
        .iter()
        .any(|cause| cause.kind() == JobErrorKind::ObserverFailure));
    assert_eq!(fs::read(output).unwrap(), b"old output");
    assert_empty(&scratch);
}
