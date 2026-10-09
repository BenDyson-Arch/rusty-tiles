//! External D1 facade tests: no access to private runtime or native handles.
use rusty_tiles::{raster_to_directory, JobErrorKind, RasterDirectoryRequest, RunControl};
use std::{fs, path::Path};
const SOURCE: &[u8] = include_bytes!("fixtures/d1-rgb.tif");
fn request(root: &Path, output: &str) -> RasterDirectoryRequest {
    RasterDirectoryRequest::web_mercator_rgb(root.join("source.tif"), root.join(output), 3, 5, 2)
}
fn fixture() -> tempfile::TempDir {
    // Linux container tests need admitted local storage, not overlayfs.
    #[cfg(target_os = "linux")]
    let root = tempfile::tempdir_in("/dev/shm").unwrap();
    #[cfg(not(target_os = "linux"))]
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source.tif"), SOURCE).unwrap();
    root
}
#[test]
fn requests_conflicts_and_precancel_create_no_work() {
    let root = fixture();
    fs::create_dir(root.path().join("occupied")).unwrap();
    fs::write(root.path().join("occupied/old"), b"original").unwrap();
    #[cfg(feature = "native-geospatial")]
    {
        let failure = raster_to_directory(request(root.path(), "occupied"), &RunControl::default())
            .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::Conflict);
        assert_eq!(
            fs::read(root.path().join("occupied/old")).unwrap(),
            b"original"
        );
    }
    let invalid = RasterDirectoryRequest::web_mercator_rgb(
        root.path().join("source.tif"),
        root.path().join("invalid"),
        25,
        0,
        0,
    );
    assert_eq!(
        raster_to_directory(invalid, &RunControl::default())
            .unwrap_err()
            .error
            .kind(),
        JobErrorKind::InvalidRequest
    );
    let run = RunControl::default();
    assert!(run.cancellation_handle().cancel());
    assert_eq!(
        raster_to_directory(request(root.path(), "cancelled"), &run)
            .unwrap_err()
            .error
            .kind(),
        JobErrorKind::Cancelled
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}
#[cfg(not(feature = "native-geospatial"))]
#[test]
fn portable_build_refuses_native_capability_without_staging() {
    let root = fixture();
    let failure =
        raster_to_directory(request(root.path(), "output"), &RunControl::default()).unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::Unsupported);
    assert!(failure.error.message().contains("native-geospatial"));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}
#[cfg(feature = "native-geospatial")]
mod native {
    use super::*;
    use rusty_tiles::{JobError, Observer, RunEvent};
    use std::sync::{Arc, Barrier};
    #[test]
    fn exact_samples_report_inventory_and_repeat_isolation() {
        let root = fixture();
        for name in ["first", "second"] {
            let result =
                raster_to_directory(request(root.path(), name), &RunControl::default()).unwrap();
            assert_eq!(result.report.width, 256);
            assert_eq!(result.report.source_bytes, SOURCE.len() as u64);
            let report: serde_json::Value =
                serde_json::from_slice(&fs::read(result.output.join("report.json")).unwrap())
                    .unwrap();
            assert_eq!(report, serde_json::to_value(result.report).unwrap());
            let pixels = image::open(result.output.join("tiles/3/5/2.png"))
                .unwrap()
                .to_rgb8();
            for (x, y, pixel) in pixels.enumerate_pixels() {
                assert_eq!(pixel.0, [x as u8, y as u8, ((3 * x + 5 * y) % 256) as u8]);
            }
            assert_eq!(
                walkdir::WalkDir::new(&result.output)
                    .into_iter()
                    .map(Result::unwrap)
                    .filter(|e| e.file_type().is_file())
                    .count(),
                3
            );
        }
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    }
    #[cfg(unix)]
    #[test]
    fn native_source_paths_preserve_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let root = fixture();
        let source = root
            .path()
            .join(std::ffi::OsString::from_vec(b"rgb-\xff.tif".to_vec()));
        fs::write(&source, SOURCE).unwrap();
        let result = raster_to_directory(
            RasterDirectoryRequest::web_mercator_rgb(source, root.path().join("output"), 3, 5, 2),
            &RunControl::default(),
        )
        .unwrap();
        assert_eq!(result.report.source_bytes, SOURCE.len() as u64);
        assert_eq!(
            image::open(result.output.join("tiles/3/5/2.png"))
                .unwrap()
                .to_rgb8()
                .get_pixel(5, 2)
                .0,
            [5, 2, 25]
        );
    }

    struct Refuse(&'static str);
    impl Observer for Refuse {
        fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
            if matches!(event,RunEvent::Progress{phase,..} if *phase==self.0) {
                return Err(JobError::new(
                    JobErrorKind::ObserverFailure,
                    "selected callback failure",
                ));
            }
            Ok(())
        }
    }
    #[test]
    fn early_and_final_observer_failures_leave_no_output_or_scratch() {
        for phase in ["raster_read", "raster_directory"] {
            let root = fixture();
            let run = RunControl::new(Some(Arc::new(Refuse(phase))));
            let failure = raster_to_directory(request(root.path(), "output"), &run).unwrap_err();
            assert_eq!(failure.error.kind(), JobErrorKind::ObserverFailure);
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        }
    }
    struct Rendezvous(Arc<Barrier>);
    impl Observer for Rendezvous {
        fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
            if matches!(
                event,
                RunEvent::Progress {
                    phase: "raster_directory",
                    ..
                }
            ) {
                self.0.wait();
            }
            Ok(())
        }
    }
    #[test]
    fn concurrent_create_new_has_one_winner() {
        let root = fixture();
        let barrier = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..2)
                .map(|_| {
                    let req = request(root.path(), "output");
                    let run = RunControl::new(Some(Arc::new(Rendezvous(barrier.clone()))));
                    scope.spawn(move || raster_to_directory(req, &run))
                })
                .collect();
            jobs.into_iter()
                .map(|j| j.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .into_iter()
                .filter_map(Result::err)
                .next()
                .unwrap()
                .error
                .kind(),
            JobErrorKind::Conflict
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[test]
    fn callback_cwd_cannot_redirect_source_or_destination() {
        const CHILD: &str = "RUSTY_TILES_D1_CWD_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "native::callback_cwd_cannot_redirect_source_or_destination",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        struct Change(std::path::PathBuf);
        impl Observer for Change {
            fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
                std::env::set_current_dir(&self.0).unwrap();
                Ok(())
            }
        }
        let root = fixture();
        fs::create_dir(root.path().join("elsewhere")).unwrap();
        std::env::set_current_dir(root.path()).unwrap();
        let run = RunControl::new(Some(Arc::new(Change(root.path().join("elsewhere")))));
        let result = raster_to_directory(
            RasterDirectoryRequest::web_mercator_rgb("source.tif", "output", 3, 5, 2),
            &run,
        )
        .unwrap();
        assert_eq!(
            result.output,
            fs::canonicalize(root.path().join("output")).unwrap()
        );
        assert!(!root.path().join("elsewhere/output").exists());
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
    }
}
