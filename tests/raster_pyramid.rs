//! The full raster facade remains separate from the accepted single-tile API.
use rusty_tiles::{raster_to_pyramid, JobErrorKind, RasterLimits, RasterRequest, RunControl};
use std::{fs, path::Path};
fn image(root: &Path) -> RasterRequest {
    RasterRequest::image(root.join("source.tif"), root.join("result"), 2, 4)
}
#[test]
fn request_domain_and_control_are_checked_before_filesystem_work() {
    let root = tempfile::tempdir().unwrap();
    let invalid = [
        RasterRequest::image(
            root.path().join("source.tif"),
            root.path().join("result"),
            5,
            4,
        ),
        RasterRequest::image(
            root.path().join("source.tif"),
            root.path().join("result"),
            0,
            25,
        ),
        image(root.path()).with_alpha_band(0),
        RasterRequest::gray(
            root.path().join("source.tif"),
            root.path().join("result"),
            2,
            4,
            0,
            0.,
            1.,
        ),
        RasterRequest::gray(
            root.path().join("source.tif"),
            root.path().join("result"),
            2,
            4,
            1,
            -f64::MAX,
            f64::MAX,
        ),
        image(root.path()).with_limits(RasterLimits {
            workers: 5,
            ..RasterLimits::default()
        }),
        image(root.path()).with_limits(RasterLimits {
            max_source_bytes: 0,
            ..RasterLimits::default()
        }),
    ];
    for request in invalid {
        let control = RunControl::default();
        assert_eq!(
            raster_to_pyramid(request, &control)
                .unwrap_err()
                .error
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(
            raster_to_pyramid(image(root.path()), &control)
                .unwrap_err()
                .error
                .kind(),
            JobErrorKind::InvalidState
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    let control = RunControl::default();
    assert!(control.cancellation_handle().cancel());
    assert_eq!(
        raster_to_pyramid(image(root.path()), &control)
            .unwrap_err()
            .error
            .kind(),
        JobErrorKind::Cancelled
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
#[cfg(not(feature = "native-geospatial"))]
#[test]
fn portable_capability_refusal_never_opens_missing_source_or_destination() {
    let root = tempfile::tempdir().unwrap();
    let request = RasterRequest::image(
        root.path().join("missing/source.tif"),
        root.path().join("missing/output"),
        0,
        1,
    );
    let failure = raster_to_pyramid(request, &RunControl::default()).unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::Unsupported);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
#[cfg(feature = "native-geospatial")]
#[test]
fn completion_observer_panic_preserves_previous_directory() {
    use rusty_tiles::{JobError, Observer, OutputPolicy, RunEvent};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    struct RejectCompletion(Arc<AtomicBool>);
    impl Observer for RejectCompletion {
        fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
            if matches!(
                event,
                RunEvent::Progress {
                    phase: "raster_complete",
                    ..
                }
            ) {
                self.0.store(true, Ordering::SeqCst);
                panic!("reached completion rejection");
            }
            Ok(())
        }
    }
    #[cfg(target_os = "linux")]
    let root = tempfile::tempdir_in("/dev/shm").unwrap();
    #[cfg(not(target_os = "linux"))]
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("source.tif"),
        include_bytes!("fixtures/d1-rgb.tif"),
    )
    .unwrap();
    fs::create_dir(root.path().join("result")).unwrap();
    fs::write(root.path().join("result/previous"), b"preserved output").unwrap();
    let reached = Arc::new(AtomicBool::new(false));
    let control = RunControl::new(Some(Arc::new(RejectCompletion(reached.clone()))));
    let failure = raster_to_pyramid(
        image(root.path()).with_policy(OutputPolicy::Replace),
        &control,
    )
    .unwrap_err();
    assert!(
        reached.load(Ordering::SeqCst),
        "completion observer must actually be reached"
    );
    assert_eq!(failure.error.kind(), JobErrorKind::ObserverFailure);
    assert_eq!(
        fs::read(root.path().join("result/previous")).unwrap(),
        b"preserved output"
    );
    assert_eq!(fs::read_dir(root.path().join("result")).unwrap().count(), 1);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[cfg(feature = "native-geospatial")]
#[test]
fn interior_native_observer_panic_keeps_cause_and_restores_owners() {
    use rusty_tiles::{JobError, Observer, OutputPolicy, RunEvent};
    use std::{
        ffi::CStr,
        ptr,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };
    struct RejectInterior(Arc<AtomicBool>);
    impl Observer for RejectInterior {
        fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
            if let RunEvent::Progress {
                phase: "source_cog" | "aligned_warp" | "tiling",
                done,
                total: Some(total),
            } = event
            {
                if *done > 0 && done < total {
                    self.0.store(true, Ordering::SeqCst);
                    panic!("reached interior native observer rejection");
                }
            }
            Ok(())
        }
    }
    let keys = [c"CPL_TMPDIR", c"GDAL_PAM_ENABLED", c"GDAL_NUM_THREADS"];
    let snapshot = || {
        keys.map(|key| {
            let value =
                unsafe { gdal_sys::CPLGetThreadLocalConfigOption(key.as_ptr(), ptr::null()) };
            (!value.is_null()).then(|| unsafe { CStr::from_ptr(value).to_bytes().to_vec() })
        })
    };
    let before = snapshot();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.tif");
    fs::write(&source, include_bytes!("fixtures/d1-rgb.tif")).unwrap();
    fs::create_dir(root.path().join("result")).unwrap();
    fs::write(root.path().join("result/previous"), b"preserved output").unwrap();
    let reached = Arc::new(AtomicBool::new(false));
    let control = RunControl::new(Some(Arc::new(RejectInterior(reached.clone()))));
    let failure = raster_to_pyramid(
        image(root.path()).with_policy(OutputPolicy::Replace),
        &control,
    )
    .unwrap_err();
    assert!(reached.load(Ordering::SeqCst), "interior callback must run");
    assert_eq!(failure.error.kind(), JobErrorKind::ObserverFailure);
    assert!(
        failure.secondary.is_empty(),
        "unexpected secondary failure: {failure:?}"
    );
    assert_eq!(snapshot(), before, "thread-local native settings restored");
    assert_eq!(
        fs::read(root.path().join("result/previous")).unwrap(),
        b"preserved output"
    );
    assert_eq!(fs::read_dir(root.path().join("result")).unwrap().count(), 1);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    // Renaming and removing the admitted source also exercises release of the
    // original handle on platforms that prohibit deleting an open raster.
    let moved = root.path().join("closed-source.tif");
    fs::rename(&source, &moved).unwrap();
    fs::remove_file(&moved).unwrap();
}

#[cfg(feature = "native-geospatial")]
#[test]
fn source_mutation_at_reached_cog_completion_refuses_publication() {
    use rusty_tiles::{JobError, Observer, OutputPolicy, RunEvent};
    use std::{
        io::Write,
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };
    struct ChangeSource {
        path: PathBuf,
        reached: Arc<AtomicBool>,
    }
    impl Observer for ChangeSource {
        fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
            if matches!(
                event,
                RunEvent::Progress {
                    phase: "source_cog",
                    done: 1_000_000,
                    ..
                }
            ) && !self.reached.swap(true, Ordering::SeqCst)
            {
                fs::OpenOptions::new()
                    .append(true)
                    .open(&self.path)
                    .unwrap()
                    .write_all(b"source mutation")
                    .unwrap();
            }
            Ok(())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.tif");
    fs::write(&source, include_bytes!("fixtures/d1-rgb.tif")).unwrap();
    fs::create_dir(root.path().join("result")).unwrap();
    fs::write(root.path().join("result/previous"), b"preserved output").unwrap();
    let reached = Arc::new(AtomicBool::new(false));
    let control = RunControl::new(Some(Arc::new(ChangeSource {
        path: source,
        reached: reached.clone(),
    })));
    let failure = raster_to_pyramid(
        image(root.path()).with_policy(OutputPolicy::Replace),
        &control,
    )
    .unwrap_err();
    assert!(reached.load(Ordering::SeqCst), "COG completion must run");
    assert_eq!(failure.error.kind(), JobErrorKind::Conflict);
    assert!(failure.error.to_string().contains("dependency changed"));
    assert!(failure.secondary.is_empty());
    assert_eq!(
        fs::read(root.path().join("result/previous")).unwrap(),
        b"preserved output"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}
