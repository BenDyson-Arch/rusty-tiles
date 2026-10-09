//! External D1 facade tests: no access to private runtime or native handles.
#[cfg(feature = "native-geospatial")]
use rusty_tiles::OutputPolicy;
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
    #[test]
    fn replace_commits_complete_raster_for_present_and_absent_entries() {
        for old_kind in ["absent", "directory", "file"] {
            let root = fixture();
            let output = root.path().join("output");
            match old_kind {
                "directory" => {
                    fs::create_dir(&output).unwrap();
                    fs::write(output.join("old.bin"), b"original inventory").unwrap();
                }
                "file" => fs::write(&output, b"original file").unwrap(),
                _ => (),
            }
            let result = raster_to_directory(
                request(root.path(), "output").with_policy(OutputPolicy::Replace),
                &RunControl::default(),
            )
            .unwrap();
            assert_eq!(result.output, output);
            assert!(result.cleanup_diagnostics.is_empty());
            assert!(!output.join("old.bin").exists());
            assert_eq!(
                image::open(output.join("tiles/3/5/2.png"))
                    .unwrap()
                    .to_rgb8()
                    .get_pixel(5, 2)
                    .0,
                [5, 2, 25]
            );
            assert_eq!(
                walkdir::WalkDir::new(&output)
                    .into_iter()
                    .map(Result::unwrap)
                    .filter(|e| e.file_type().is_file())
                    .count(),
                3
            );
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        }
    }
    #[cfg(unix)]
    #[test]
    fn replace_symlink_moves_link_without_touching_its_referent() {
        let root = fixture();
        let referent = root.path().join("referent");
        fs::create_dir(&referent).unwrap();
        fs::write(referent.join("original"), b"referent bytes").unwrap();
        std::os::unix::fs::symlink(&referent, root.path().join("output")).unwrap();
        let result = raster_to_directory(
            request(root.path(), "output").with_policy(OutputPolicy::Replace),
            &RunControl::default(),
        )
        .unwrap();
        assert!(!fs::symlink_metadata(&result.output)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read(referent.join("original")).unwrap(),
            b"referent bytes"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    }
    struct CountEvents(std::sync::atomic::AtomicUsize);
    impl Observer for CountEvents {
        fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }
    fn assert_overlap_refused(source: &Path, output: &Path, root: &Path) {
        let before: Vec<_> = walkdir::WalkDir::new(root)
            .sort_by_file_name()
            .into_iter()
            .map(Result::unwrap)
            .map(|entry| {
                (
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    if entry.file_type().is_file() {
                        Some(fs::read(entry.path()).unwrap())
                    } else {
                        None
                    },
                )
            })
            .collect();
        let observer = Arc::new(CountEvents(std::sync::atomic::AtomicUsize::new(0)));
        let run = RunControl::new(Some(observer.clone()));
        let failure = raster_to_directory(
            RasterDirectoryRequest::web_mercator_rgb(source, output, 3, 5, 2)
                .with_policy(OutputPolicy::Replace),
            &run,
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
        assert!(failure.recovery.is_none());
        assert!(failure.retained_paths.is_empty());
        assert_eq!(observer.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        let after: Vec<_> = walkdir::WalkDir::new(root)
            .sort_by_file_name()
            .into_iter()
            .map(Result::unwrap)
            .map(|entry| {
                (
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    if entry.file_type().is_file() {
                        Some(fs::read(entry.path()).unwrap())
                    } else {
                        None
                    },
                )
            })
            .collect();
        assert_eq!(after, before);
        assert_eq!(fs::read(source).unwrap(), SOURCE);
    }
    #[test]
    fn replace_rejects_source_file_hardlink_and_containing_directory_before_work() {
        let root = fixture();
        let source = root.path().join("source.tif");
        assert_overlap_refused(&source, &source, root.path());
        let alias = root.path().join("hardlink.tif");
        fs::hard_link(&source, &alias).unwrap();
        assert_overlap_refused(&source, &alias, root.path());
        let tree = root.path().join("tree");
        fs::create_dir(&tree).unwrap();
        fs::create_dir(tree.join("nested")).unwrap();
        fs::write(tree.join("source.tif"), SOURCE).unwrap();
        fs::write(tree.join("nested/other"), b"other inventory").unwrap();
        assert_overlap_refused(&tree.join("nested/../source.tif"), &tree, root.path());
        let nested = tree.join("nested/source.tif");
        fs::write(&nested, SOURCE).unwrap();
        assert_overlap_refused(&nested, &tree, root.path());
    }
    #[cfg(unix)]
    #[test]
    fn replace_rejects_source_under_aliased_output_parent() {
        let root = fixture();
        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::create_dir(real.join("tree")).unwrap();
        fs::write(real.join("tree/source.tif"), SOURCE).unwrap();
        let alias = root.path().join("parent-alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        assert_overlap_refused(
            &real.join("tree/source.tif"),
            &alias.join("tree"),
            root.path(),
        );
        assert_overlap_refused(
            &alias.join("tree/source.tif"),
            &real.join("tree"),
            root.path(),
        );
    }
    #[cfg(windows)]
    #[test]
    fn replace_rejects_case_aliased_containing_directory() {
        let root = fixture();
        let tree = root.path().join("MixedCase");
        fs::create_dir(&tree).unwrap();
        fs::write(tree.join("source.tif"), SOURCE).unwrap();
        let alias = root.path().join("mixedcase");
        // Case-sensitive Windows directories do not provide this alias.
        if alias.exists() {
            assert_overlap_refused(&tree.join("source.tif"), &alias, root.path());
        }
    }

    #[cfg(unix)]
    #[test]
    fn replace_source_referent_symlink_entry_is_allowed() {
        for directory_referent in [false, true] {
            let root = fixture();
            let output = root.path().join("output");
            std::os::unix::fs::symlink(
                if directory_referent {
                    root.path().to_owned()
                } else {
                    root.path().join("source.tif")
                },
                &output,
            )
            .unwrap();
            let result = raster_to_directory(
                request(root.path(), "output").with_policy(OutputPolicy::Replace),
                &RunControl::default(),
            )
            .unwrap();
            assert!(!fs::symlink_metadata(&result.output)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(fs::read(root.path().join("source.tif")).unwrap(), SOURCE);
            assert_eq!(
                image::open(result.output.join("tiles/3/5/2.png"))
                    .unwrap()
                    .to_rgb8()
                    .get_pixel(5, 2)
                    .0,
                [5, 2, 25]
            );
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        }
    }

    #[test]
    fn replace_precancel_and_observer_failures_preserve_original_tree() {
        struct CancelFinal(std::sync::Mutex<Option<rusty_tiles::CancellationHandle>>);
        impl Observer for CancelFinal {
            fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
                if matches!(
                    event,
                    RunEvent::Progress {
                        phase: "raster_directory",
                        ..
                    }
                ) {
                    self.0.lock().unwrap().as_ref().unwrap().cancel();
                }
                Ok(())
            }
        }
        for phase in [
            "precancel",
            "raster_read",
            "raster_directory",
            "cancel_final",
        ] {
            let root = fixture();
            let output = root.path().join("output");
            fs::create_dir(&output).unwrap();
            fs::create_dir(output.join("nested")).unwrap();
            fs::write(output.join("nested/original"), b"old output").unwrap();
            let run = if phase == "precancel" {
                let run = RunControl::default();
                run.cancellation_handle().cancel();
                run
            } else if phase == "cancel_final" {
                let observer = Arc::new(CancelFinal(std::sync::Mutex::new(None)));
                let run = RunControl::new(Some(observer.clone()));
                *observer.0.lock().unwrap() = Some(run.cancellation_handle());
                run
            } else {
                RunControl::new(Some(Arc::new(Refuse(phase))))
            };
            let failure = raster_to_directory(
                request(root.path(), "output").with_policy(OutputPolicy::Replace),
                &run,
            )
            .unwrap_err();
            assert_eq!(
                failure.error.kind(),
                if phase == "precancel" || phase == "cancel_final" {
                    JobErrorKind::Cancelled
                } else {
                    JobErrorKind::ObserverFailure
                }
            );
            assert!(failure.recovery.is_none());
            assert!(failure.retained_paths.is_empty());
            assert_eq!(
                fs::read(output.join("nested/original")).unwrap(),
                b"old output"
            );
            assert_eq!(
                walkdir::WalkDir::new(&output)
                    .into_iter()
                    .map(Result::unwrap)
                    .filter(|e| e.file_type().is_file())
                    .count(),
                1
            );
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        }
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

    #[test]
    fn malformed_bigtiff_offsets_are_input_errors_before_seek() {
        for little in [false, true] {
            for offset in [16u64, 17, 23, 24, 1 << 40, 1 << 63, u64::MAX] {
                let root = fixture();
                let mut source = vec![0u8; 24];
                source[..2].copy_from_slice(if little { b"II" } else { b"MM" });
                source[2..4].copy_from_slice(&if little {
                    43u16.to_le_bytes()
                } else {
                    43u16.to_be_bytes()
                });
                source[4..6].copy_from_slice(&if little {
                    8u16.to_le_bytes()
                } else {
                    8u16.to_be_bytes()
                });
                source[8..16].copy_from_slice(&if little {
                    offset.to_le_bytes()
                } else {
                    offset.to_be_bytes()
                });
                fs::write(root.path().join("source.tif"), source).unwrap();
                let failure =
                    raster_to_directory(request(root.path(), "output"), &RunControl::default())
                        .unwrap_err();
                assert_eq!(
                    failure.error.kind(),
                    JobErrorKind::InvalidInput,
                    "offset={offset}, little={little}: {failure}"
                );
                assert!(failure.retained_paths.is_empty());
                assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
            }
        }
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
