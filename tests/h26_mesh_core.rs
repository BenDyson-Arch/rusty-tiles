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
    // Every POSITION element of the grid's one primitive is decoded.
    assert_eq!(total("capture_corners"), u64::from((side + 1) * (side + 1)));
    assert_eq!(total("capture_positions"), triangles);
    assert_eq!(total("capture_triangles"), triangles);
    assert_eq!(total("capture_sequence"), triangles);
    assert_eq!(total("capture_tree"), triangles);
    assert_eq!(total("capture_corner_depths"), 3 * triangles);
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
    assert!(at("hlod_incidence", triangles) > at("hlod_incidence_ends", incidence[2].total));
    // Every stage's own events carry a total (#353): capture and the incidence pass count
    // the triangles they read, the leaves and the archive their tiles (leaves and coarse
    // tiles with content).
    for (phase, expected) in [
        ("capture", Some(triangles)),
        ("leaf_build", Some(result.report.leaves)),
        ("hlod_incidence", Some(triangles)),
        ("hlod_geometry", None),
        (
            "archive",
            Some(result.report.leaves + result.diagnostics.hlod.tiles),
        ),
    ] {
        let totals: Vec<Option<u64>> = events
            .iter()
            .filter(|e| e.0 == phase)
            .map(|e| e.2)
            .collect();
        assert!(!totals.is_empty(), "{phase}");
        assert!(totals.iter().all(Option::is_some), "{phase}: {totals:?}");
        if let Some(expected) = expected {
            assert!(
                totals.iter().all(|t| *t == Some(expected)),
                "{phase}: {totals:?}"
            );
        }
    }
    // The capture steps sit between the capture phase's own start and end events.
    let position = |phase: &str, done| {
        events
            .iter()
            .position(|e| e.0 == phase && e.1 == done)
            .unwrap()
    };
    let first_step = position("capture_scan", 0);
    assert!(position("capture", 0) < first_step);
    assert!(
        position("capture", triangles)
            > position("capture_leaf_plans", total("capture_leaf_plans"))
    );
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
/// run) fits one floor run, so every budget plans the floor and only the worker count
/// varies. Plan dependence is covered by the capture- and incidence-level sort tests and the
/// recorded real-data matrix.
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

fn read_report(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn stage_names(report: &serde_json::Value) -> Vec<String> {
    report["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["stage"].as_str().unwrap().to_owned())
        .collect()
}

/// No temporary report file is left beside the report.
fn assert_no_report_temporaries(dir: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().starts_with(".rusty-tiles-report-"),
            "{name:?}"
        );
    }
}

/// The run report (#353): one complete document after every stage, its stage facts and the
/// coarse walk's worker use (#343), the final status, and an unchanged archive.
#[test]
fn the_run_report_is_rewritten_per_stage_and_leaves_the_archive_unchanged() {
    let work = tempfile::tempdir().unwrap();
    let input = grid_fixture(work.path(), 96);
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let plain = work.path().join("plain.3tz");
    let plain_result =
        mesh_to_tileset(request(&input, &plain, &scratch), &RunControl::default()).unwrap();
    // Stage timing is reported only when asked for.
    assert!(json!(plain_result.diagnostics)
        .get("stage_timing")
        .is_none());
    let output = work.path().join("reported.3tz");
    let report_path = work.path().join("run.json");
    // Whenever the run reports progress, the report on disk is one complete document.
    let seen = Arc::new(Mutex::new(Vec::<(String, usize)>::new()));
    let (sink, watched) = (seen.clone(), report_path.clone());
    let run = control(move |event| {
        if matches!(event, RunEvent::Progress { .. }) && watched.exists() {
            let report = read_report(&watched);
            let stages = report["stages"].as_array().unwrap().len();
            let status = report["status"].as_str().unwrap().to_owned();
            sink.lock().unwrap().push((status, stages));
        }
        Ok(())
    });
    let result = mesh_to_tileset(
        request(&input, &output, &scratch)
            .with_report_path(&report_path)
            .with_stage_timing(true),
        &run,
    )
    .unwrap();
    assert_eq!(fs::read(&output).unwrap(), fs::read(&plain).unwrap());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.first().unwrap(), &("running".to_owned(), 0));
    assert!(seen.windows(2).all(|w| w[0].1 <= w[1].1), "{seen:?}");
    assert!(seen
        .iter()
        .any(|(status, stages)| status == "running" && *stages == 3));
    let report = read_report(&report_path);
    assert_eq!(report["schema"], "rusty_tiles_mesh_run_report");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["status"], "succeeded");
    assert_eq!(
        stage_names(&report),
        [
            "capture",
            "leaf_build",
            "hlod_incidence",
            "hlod_geometry",
            "archive"
        ]
    );
    for stage in report["stages"].as_array().unwrap() {
        assert!(stage["wall_seconds"].as_f64().unwrap() >= 0., "{stage}");
        assert!(
            stage["run_elapsed_seconds"].as_f64().unwrap() >= 0.,
            "{stage}"
        );
        assert!(stage["facts"].is_object(), "{stage}");
    }
    let facts = |i: usize| &report["stages"][i]["facts"];
    assert_eq!(facts(0)["triangles"], result.report.triangles);
    assert!(facts(0)["scratch_logical_peak_bytes"].as_u64().unwrap() > 0);
    assert_eq!(facts(1)["leaves"], result.report.leaves);
    assert!(facts(2)["scratch_peak_bytes"].is_u64());
    let geometry = facts(3);
    assert_eq!(geometry["tiles"], result.diagnostics.hlod.tiles);
    assert_eq!(
        geometry["omitted_by_depth"],
        json!(result.diagnostics.hlod.omitted_by_depth)
    );
    assert_eq!(
        geometry["last_resort_tiles"],
        result.diagnostics.hlod.last_resort_tiles
    );
    assert_eq!(
        geometry["last_resort_records"].as_array().unwrap().len() as u64,
        result.diagnostics.hlod.last_resort_tiles
    );
    assert!(geometry["scratch_peak_bytes"].is_u64());
    // Worker use: busy time by depth adds up, and the depths' idle time with the serial
    // idle time is the walk's whole idle time.
    let walk = &geometry["walk_utilisation"];
    assert_eq!(walk["workers"], 1);
    let depths = walk["by_depth"].as_array().unwrap();
    assert!(!depths.is_empty());
    let sum = |key: &str| depths.iter().map(|d| d[key].as_f64().unwrap()).sum::<f64>();
    let near = |a: f64, b: f64| (a - b).abs() <= 1e-6 * (1. + a.abs().max(b.abs()));
    assert!(near(
        sum("busy_worker_seconds"),
        walk["busy_worker_seconds"].as_f64().unwrap()
    ));
    assert!(near(
        sum("idle_worker_seconds") + walk["serial_idle_worker_seconds"].as_f64().unwrap(),
        walk["idle_worker_seconds"].as_f64().unwrap()
    ));
    for depth in depths {
        let share = depth["slowest_cell_share"].as_f64().unwrap();
        assert!(share > 0. && share <= 1., "{depth}");
        assert!(depth["cells"].as_u64().unwrap() > 0);
    }
    assert_eq!(
        facts(4)["archive_bytes"],
        fs::metadata(&output).unwrap().len()
    );
    // The final document carries the run's report and diagnostics, as `--json` does.
    assert_eq!(report["tilerReport"]["tiles"], result.report.tiles);
    assert_eq!(report["tilerReport"]["triangles"], result.report.triangles);
    assert_eq!(
        report["diagnostics"]["archive_bytes"],
        result.diagnostics.archive_bytes
    );
    assert_eq!(
        report["diagnostics"]["hlod"]["walk_utilisation"]["by_depth"]
            .as_array()
            .unwrap()
            .len(),
        depths.len()
    );
    assert!(report["diagnostics"]["stage_timing"]["stages"].is_object());
    assert_no_report_temporaries(work.path());
    assert_empty(&scratch);
}

/// A cancelled run leaves a valid report naming the stages that finished; a report path
/// that aliases the run's files or cannot be written refuses the run before capture.
#[test]
fn a_cancelled_run_leaves_a_valid_partial_report_and_bad_report_paths_are_refused() {
    let work = tempfile::tempdir().unwrap();
    let input = grid_fixture(work.path(), 96);
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let output = work.path().join("cancelled.3tz");
    let report_path = work.path().join("run.json");
    let handle = Arc::new(Mutex::new(None::<CancellationHandle>));
    let observer_handle = handle.clone();
    let run = control(move |event| {
        if let RunEvent::Progress { phase, .. } = event {
            if *phase == "hlod_geometry" {
                observer_handle.lock().unwrap().as_ref().unwrap().cancel();
            }
        }
        Ok(())
    });
    *handle.lock().unwrap() = Some(run.cancellation_handle());
    let failure = mesh_to_tileset(
        request(&input, &output, &scratch).with_report_path(&report_path),
        &run,
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
    assert!(!output.exists());
    let report = read_report(&report_path);
    assert_eq!(report["status"], "failed");
    assert_eq!(report["error"]["kind"], "cancelled");
    assert_eq!(
        stage_names(&report),
        ["capture", "leaf_build", "hlod_incidence"]
    );
    assert!(report.get("diagnostics").is_none());
    assert_no_report_temporaries(work.path());
    assert_empty(&scratch);

    let refused = |report: PathBuf| {
        let output = work.path().join("refused.3tz");
        let failure = mesh_to_tileset(
            request(&input, &output, &scratch).with_report_path(report),
            &RunControl::default(),
        )
        .unwrap_err();
        assert!(!output.exists());
        assert_empty(&scratch);
        failure.error.kind()
    };
    assert_eq!(
        refused(work.path().join("refused.3tz")),
        JobErrorKind::InvalidRequest
    );
    assert_eq!(refused(input.clone()), JobErrorKind::InvalidRequest);
    assert_eq!(refused(scratch.clone()), JobErrorKind::InvalidRequest);
    assert_eq!(
        refused(work.path().join("missing").join("run.json")),
        JobErrorKind::Io
    );
}
