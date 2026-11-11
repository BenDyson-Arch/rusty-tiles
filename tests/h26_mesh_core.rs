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

/// A `side × side` grid of indexed quads (two triangles each) in one primitive.
fn grid_fixture(root: &Path, side: u32) -> PathBuf {
    let mut bytes = Vec::new();
    for y in 0..=side {
        for x in 0..=side {
            for v in [x as f32, ((x * 7 + y * 3) % 5) as f32 * 0.1, y as f32] {
                bytes.extend(v.to_le_bytes());
            }
        }
    }
    let positions = bytes.len();
    let row = side + 1;
    for y in 0..side {
        for x in 0..side {
            let a = y * row + x;
            for i in [a, a + row, a + 1, a + 1, a + row, a + row + 1] {
                bytes.extend(i.to_le_bytes());
            }
        }
    }
    fs::write(root.join("grid.bin"), &bytes).unwrap();
    let vertices = row * row;
    let input = root.join("grid.gltf");
    fs::write(&input, serde_json::to_vec(&json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
        "buffers":[{"uri":"grid.bin","byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":positions},
            {"buffer":0,"byteOffset":positions,"byteLength":bytes.len()-positions}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":vertices,"type":"VEC3",
                "min":[0,0,0],"max":[side as f32,0.4,side as f32]},
            {"bufferView":1,"componentType":5125,"count":side*side*6,"type":"SCALAR"}]
    })).unwrap()).unwrap();
    input
}

#[derive(Debug, PartialEq)]
struct Step {
    phase: String,
    first: u64,
    last: u64,
    total: u64,
    events: usize,
}

/// Capture steps in order (consecutive events of one phase), with their first and last counts.
fn capture_steps(events: &[(String, u64, Option<u64>)]) -> Vec<Step> {
    steps_with(events, "capture_")
}

/// Steps whose phase starts with `prefix`, as [`capture_steps`].
fn steps_with(events: &[(String, u64, Option<u64>)], prefix: &str) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    for (phase, done, total) in events {
        if !phase.starts_with(prefix) {
            continue;
        }
        let total = total.expect("capture steps declare totals");
        assert!(*done <= total, "{phase} {done}/{total}");
        match steps.last_mut() {
            Some(step) if step.phase == *phase => {
                assert!(*done >= step.last, "{phase} counts never go back");
                step.last = *done;
                step.events += 1;
            }
            _ => steps.push(Step {
                phase: phase.clone(),
                first: *done,
                last: *done,
                total,
                events: 1,
            }),
        }
    }
    steps
}

#[test]
fn capture_reports_every_step_with_its_total_and_honours_a_cancel_inside_capture() {
    let work = tempfile::tempdir().unwrap();
    // Above one 8,192-triangle leaf, so the coarse stage (and its incidence pass) runs.
    let side = 96;
    let triangles = u64::from(2 * side * side);
    let input = grid_fixture(work.path(), side);
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let output = work.path().join("output.3tz");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let run = control(move |event| {
        if let RunEvent::Progress { phase, done, total } = event {
            sink.lock()
                .unwrap()
                .push((phase.to_string(), *done, *total));
        }
        Ok(())
    });
    let result = mesh_to_tileset(request(&input, &output, &scratch), &run).unwrap();
    assert_eq!(result.report.triangles, triangles);
    let events = seen.lock().unwrap();
    let steps = capture_steps(&events);
    let names: Vec<&str> = steps.iter().map(|s| s.phase.as_str()).collect();
    assert_eq!(
        names,
        [
            "capture_scan",
            "capture_validate",
            "capture_corners",
            "capture_positions",
            "capture_triangles",
            "capture_sequence",
            "capture_images",
            "capture_tree",
            "capture_corner_depths",
            "capture_leaf_plans",
        ]
    );
    let total = |name: &str| steps.iter().find(|s| s.phase == name).unwrap().total;
    assert_eq!(total("capture_scan"), triangles);
    assert_eq!(total("capture_corners"), 3 * triangles);
    assert_eq!(total("capture_positions"), 3 * triangles);
    assert_eq!(total("capture_triangles"), triangles);
    assert_eq!(total("capture_sequence"), triangles);
    assert_eq!(total("capture_tree"), triangles);
    assert_eq!(total("capture_corner_depths"), 7 * triangles);
    assert_eq!(total("capture_images"), 0);
    for step in &steps {
        assert_eq!((step.first, step.last), (0, step.total), "{step:?}");
    }
    // The coarse stage's incidence pass reports its steps the same way, inside its phase.
    let incidence = steps_with(&events, "hlod_incidence_");
    let names: Vec<&str> = incidence.iter().map(|s| s.phase.as_str()).collect();
    assert_eq!(
        names,
        [
            "hlod_incidence_items",
            "hlod_incidence_reduce",
            "hlod_incidence_ends"
        ]
    );
    assert_eq!(incidence[0].total, triangles);
    assert!(incidence[1].total >= triangles, "{incidence:?}");
    for step in &incidence {
        assert_eq!((step.first, step.last), (0, step.total), "{step:?}");
    }
    let at = |phase: &str, done| {
        events
            .iter()
            .position(|e| e.0 == phase && e.1 == done)
            .unwrap()
    };
    assert!(at("hlod_incidence", 0) < at("hlod_incidence_items", 0));
    assert!(at("hlod_incidence", 1) > at("hlod_incidence_ends", incidence[2].total));
    // The capture steps sit between the capture phase's own start and end events.
    let position = |phase: &str, done| {
        events
            .iter()
            .position(|e| e.0 == phase && e.1 == done)
            .unwrap()
    };
    let first_step = position("capture_scan", 0);
    assert!(position("capture", 0) < first_step);
    assert!(position("capture", 1) > position("capture_leaf_plans", total("capture_leaf_plans")));
    drop(events);

    // A cancel issued from an observer during capture (as the Python callback does on an
    // interrupt) lands at capture's next check: nothing after the current step runs.
    let output = work.path().join("cancelled.3tz");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let handle = Arc::new(Mutex::new(None::<CancellationHandle>));
    let observer_handle = handle.clone();
    let run = control(move |event| {
        if let RunEvent::Progress { phase, done, total } = event {
            sink.lock()
                .unwrap()
                .push((phase.to_string(), *done, *total));
            if *phase == "capture_positions" && *done == 0 {
                assert!(observer_handle.lock().unwrap().as_ref().unwrap().cancel());
            }
        }
        Ok(())
    });
    *handle.lock().unwrap() = Some(run.cancellation_handle());
    let failure = mesh_to_tileset(request(&input, &output, &scratch), &run).unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
    let events = seen.lock().unwrap();
    let last = capture_steps(&events).pop().unwrap();
    assert_eq!(last.phase, "capture_positions");
    assert!(last.last < last.total, "{last:?}");
    assert!(!events.iter().any(|e| e.0 == "leaf_build"));
    assert!(!output.exists());
    assert_empty(&scratch);
}

/// Plumbing control: this grid (three leaves, so the coarse stage and its incidence pass
/// run) fits one floor run, so every budget plans floor-sized runs and only the worker and
/// writer counts vary. Plan dependence is covered by the capture- and incidence-level sort
/// tests and the recorded real-data matrix.
#[test]
fn archives_are_identical_across_workers_and_budgets() {
    let work = tempfile::tempdir().unwrap();
    let input = grid_fixture(work.path(), 96);
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let mut archives = Vec::new();
    for (jobs, budget) in [(1, 2u64 << 30), (4, 2 << 30), (1, 6 << 30), (4, 6 << 30)] {
        let output = work.path().join(format!("grid-{jobs}-{budget}.3tz"));
        let request = TilerRequest::new(&input, &output)
            .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(jobs).unwrap()))
            .with_scratch_dir(&scratch)
            .with_memory_budget(budget);
        mesh_to_tileset(request, &RunControl::default()).unwrap();
        archives.push(fs::read(&output).unwrap());
        assert_empty(&scratch);
    }
    assert!(archives.windows(2).all(|pair| pair[0] == pair[1]));
}
