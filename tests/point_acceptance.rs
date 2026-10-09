//! Public point-cloud lifecycle controls. Fidelity has a separate independent oracle.
use rusty_tiles::{
    point_cloud::{
        point_cloud_to_archive, PointCloudCoordinates, PointCloudOptions, PointCloudRequest,
    },
    JobError, JobErrorKind, Observer, OutputPolicy, RunControl, RunEvent,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

fn fixture(path: &Path) {
    let mut writer = las::Writer::from_path(path, las::Header::default()).unwrap();
    for i in 0..24 {
        writer
            .write_point(las::Point {
                x: i as f64 * 3.,
                y: (i % 5) as f64,
                z: (i % 7) as f64,
                ..Default::default()
            })
            .unwrap();
    }
    writer.close().unwrap();
}
fn request(input: &Path, output: &Path) -> PointCloudRequest {
    PointCloudRequest::new(
        input,
        output,
        PointCloudCoordinates::LocalMetres,
        PointCloudOptions {
            explicit: true,
            max_points: 3,
            chunk_points: 2,
            ..Default::default()
        },
    )
}
struct Calls(AtomicUsize);
impl Observer for Calls {
    fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[test]
fn unsafe_aliases_and_invalid_profiles_fail_before_observation_or_output_work() {
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("source.3tz");
    // LAS parsing is independent of the file extension.
    let source_las = work.path().join("source.las");
    fixture(&source_las);
    fs::rename(&source_las, &source).unwrap();
    let before = fs::read(&source).unwrap();
    let alias = work.path().join("alias.3tz");
    fs::hard_link(&source, &alias).unwrap();
    for destination in [&source, &alias] {
        let calls = Arc::new(Calls(AtomicUsize::new(0)));
        let run = RunControl::new(Some(calls.clone()));
        let failure = point_cloud_to_archive(
            request(&source, destination).with_policy(OutputPolicy::Replace),
            &run,
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
        assert_eq!(calls.0.load(Ordering::SeqCst), 0);
        assert_eq!(fs::read(&source).unwrap(), before);
    }
    let malformed = work.path().join("bad.las");
    fs::write(&malformed, b"bad source").unwrap();
    let parent = work.path().join("not-created");
    let output = parent.join("out.3tz");
    let calls = Arc::new(Calls(AtomicUsize::new(0)));
    let run = RunControl::new(Some(calls.clone()));
    assert!(point_cloud_to_archive(request(&malformed, &output), &run).is_err());
    assert_eq!(calls.0.load(Ordering::SeqCst), 0);
    assert!(!parent.exists());
}

struct FaultAt {
    phase: &'static str,
    calls: AtomicUsize,
}
impl Observer for FaultAt {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        if matches!(event, RunEvent::Progress { phase, .. } if *phase == self.phase) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            return Err(JobError::new(
                JobErrorKind::ObserverFailure,
                format!("fault at {}", self.phase),
            ));
        }
        Ok(())
    }
}
#[test]
fn observer_failure_before_commit_preserves_previous_output_and_removes_scratch() {
    for phase in ["ingestion", "tiling", "point_archive", "ready_to_publish"] {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("source.las");
        fixture(&source);
        let output = work.path().join("output.3tz");
        fs::write(&output, b"previous archive").unwrap();
        let observer = Arc::new(FaultAt {
            phase,
            calls: AtomicUsize::new(0),
        });
        let run = RunControl::new(Some(observer.clone()));
        let failure = point_cloud_to_archive(
            request(&source, &output).with_policy(OutputPolicy::Replace),
            &run,
        )
        .unwrap_err();
        assert_eq!(
            observer.calls.load(Ordering::SeqCst),
            1,
            "phase not exercised: {phase}"
        );
        assert_eq!(failure.error.kind(), JobErrorKind::ObserverFailure);
        assert!(failure.error.message().contains(phase));
        assert_eq!(fs::read(&output).unwrap(), b"previous archive");
        assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
        assert!(failure.retained_paths.is_empty());
    }
}
struct CancelAtReady(Mutex<Option<rusty_tiles::CancellationHandle>>);
impl Observer for CancelAtReady {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        if matches!(
            event,
            RunEvent::Progress {
                phase: "ready_to_publish",
                ..
            }
        ) {
            assert!(self.0.lock().unwrap().take().unwrap().cancel());
        }
        Ok(())
    }
}
#[test]
fn cancellation_at_commit_boundary_preserves_old_output() {
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("source.las");
    fixture(&source);
    let output = work.path().join("output.3tz");
    fs::write(&output, b"previous").unwrap();
    let observer = Arc::new(CancelAtReady(Mutex::new(None)));
    let run = RunControl::new(Some(observer.clone()));
    *observer.0.lock().unwrap() = Some(run.cancellation_handle());
    let failure = point_cloud_to_archive(
        request(&source, &output).with_policy(OutputPolicy::Replace),
        &run,
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
    assert_eq!(fs::read(output).unwrap(), b"previous");
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
}
#[test]
fn concurrent_runs_are_isolated_and_control_reuse_is_rejected() {
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("source.las");
    fixture(&source);
    let handles: Vec<_> = (0..3)
        .map(|i| {
            let source = source.clone();
            let output = work.path().join(format!("out-{i}.3tz"));
            std::thread::spawn(move || {
                point_cloud_to_archive(request(&source, &output), &RunControl::default())
                    .unwrap()
                    .output
            })
        })
        .collect();
    let outputs: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    for output in &outputs {
        rusty_tiles::validate_3tz(output).unwrap();
    }
    let output = work.path().join("once.3tz");
    let run = RunControl::default();
    point_cloud_to_archive(request(&source, &output), &run).unwrap();
    let before = fs::read(&output).unwrap();
    let failure = point_cloud_to_archive(
        request(&source, &output).with_policy(OutputPolicy::Replace),
        &run,
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidState);
    assert_eq!(fs::read(output).unwrap(), before);
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 5);
}

struct ChangeDirectory {
    destination: PathBuf,
    calls: AtomicUsize,
}
impl Observer for ChangeDirectory {
    fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            std::env::set_current_dir(&self.destination).unwrap();
        }
        Ok(())
    }
}
#[test]
fn relative_paths_survive_callback_cwd_changes() {
    if std::env::var_os("RT_POINT_CWD_CHILD").is_none() {
        let work = tempfile::tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "relative_paths_survive_callback_cwd_changes",
                "--nocapture",
            ])
            .env("RT_POINT_CWD_CHILD", "1")
            .current_dir(work.path())
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let original = std::env::current_dir().unwrap();
    fs::create_dir("elsewhere").unwrap();
    fixture(Path::new("source.las"));
    let source_before = fs::read("source.las").unwrap();
    let observer = Arc::new(ChangeDirectory {
        destination: original.join("elsewhere"),
        calls: AtomicUsize::new(0),
    });
    let run = RunControl::new(Some(observer.clone()));
    let result =
        point_cloud_to_archive(request(Path::new("source.las"), Path::new("out.3tz")), &run)
            .unwrap();
    assert_eq!(
        result.output,
        fs::canonicalize(original.join("out.3tz")).unwrap()
    );
    assert_eq!(
        fs::read(original.join("source.las")).unwrap(),
        source_before
    );
    assert!(!original.join("elsewhere/out.3tz").exists());
    let mut archive = zip::ZipArchive::new(fs::File::open(&result.output).unwrap()).unwrap();
    let report: serde_json::Value =
        serde_json::from_reader(archive.by_name("conversion.json").unwrap()).unwrap();
    assert_eq!(report, serde_json::to_value(result.report).unwrap());
    assert!(observer.calls.load(Ordering::SeqCst) > 0);
}
