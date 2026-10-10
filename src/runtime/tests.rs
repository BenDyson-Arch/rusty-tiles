use super::*;
use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Barrier, OnceLock,
    },
    thread,
};

struct Watch<F>(F);
impl<F> Observer for Watch<F>
where
    F: Fn(&RunEvent<'_>) -> Result<(), JobError> + Send + Sync,
{
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        (self.0)(event)
    }
}

fn failed<T>(result: Result<T, JobFailure>) -> JobFailure {
    match result {
        Err(failure) => failure,
        Ok(_) => panic!("expected failure"),
    }
}

fn candidate<'a>(output: &Path, attempt: &'a Attempt) -> Staging<'a> {
    let mut staging = Staging::create(output, attempt).unwrap();
    staging.writer().write_all(b"complete candidate").unwrap();
    staging
}

fn sealed<'a>(output: &Path, attempt: &'a Attempt) -> SealedArtifact<'a> {
    let staging = candidate(output, attempt);
    attempt.close_events().unwrap();
    staging.seal().unwrap()
}

#[test]
fn create_new_preserves_a_competing_destination_created_after_seal() {
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let artifact = sealed(&output, &attempt);
    fs::write(&output, b"competing bytes").unwrap();
    let failure = failed(artifact.publish(&output, OutputPolicy::CreateNew));
    assert_eq!(failure.error.kind(), JobErrorKind::Conflict);
    assert_eq!(fs::read(&output).unwrap(), b"competing bytes");
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 1);
}

#[test]
fn replace_installs_complete_bytes() {
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    fs::write(&output, b"old").unwrap();
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let published = sealed(&output, &attempt)
        .publish(&output, OutputPolicy::Replace)
        .unwrap();
    assert_eq!(published.output, output);
    assert!(published.cleanup_diagnostics.is_empty());
    assert_eq!(fs::read(&output).unwrap(), b"complete candidate");
    assert!(!control.cancellation_handle().cancel());
}

#[test]
fn producer_failure_before_permission_blocks_publication_and_stays_primary() {
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let artifact = sealed(&output, &attempt);
    let error = JobError::new(JobErrorKind::InvalidInput, "encoder refused source");
    let first = attempt.fail(error.clone());
    assert!(first.error.same_cause(&error));
    let failure = failed(artifact.publish(&output, OutputPolicy::CreateNew));
    assert!(failure.error.same_cause(&error));
    assert!(!output.exists());
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
}

#[test]
fn cancellation_before_staging_is_idempotent_and_creates_no_parent() {
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("missing-parent/result");
    let control = RunControl::default();
    let handle = control.cancellation_handle();
    assert!(handle.cancel());
    for _ in 0..100 {
        assert!(!handle.cancel());
    }
    let attempt = control.begin().unwrap();
    let failure = failed(Staging::create(&output, &attempt));
    assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
    assert!(failure.secondary.is_empty());
    assert!(!output.parent().unwrap().exists());
}

#[test]
fn cancellation_after_seal_prevents_installation() {
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let artifact = sealed(&output, &attempt);
    assert!(control.cancellation_handle().cancel());
    let failure = failed(artifact.publish(&output, OutputPolicy::CreateNew));
    assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
    assert!(!output.exists());
}

#[test]
fn publication_permission_wins_later_cancellation() {
    struct CancelDuringInstall {
        handle: CancellationHandle,
        accepted: AtomicBool,
    }
    impl FileOperations for CancelDuringInstall {
        fn install(
            &self,
            path: TempPath,
            output: &Path,
            policy: OutputPolicy,
        ) -> Result<(), PathPersistError> {
            self.accepted.store(self.handle.cancel(), Ordering::SeqCst);
            SystemFiles.install(path, output, policy)
        }
    }
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let files = CancelDuringInstall {
        handle: control.cancellation_handle(),
        accepted: AtomicBool::new(true),
    };
    let published = sealed(&output, &attempt)
        .publish_with(&output, OutputPolicy::CreateNew, &files)
        .unwrap();
    assert!(!files.accepted.load(Ordering::SeqCst));
    assert_eq!(published.output, output);
    assert_eq!(fs::read(&output).unwrap(), b"complete candidate");
}

#[test]
fn observer_failure_prevents_installation_and_suppresses_later_calls() {
    let count = Arc::new(AtomicUsize::new(0));
    let counted = count.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |_: &RunEvent<'_>| {
        counted.fetch_add(1, Ordering::SeqCst);
        Err(JobError::new(JobErrorKind::Io, "callback failed"))
    }))));
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let attempt = control.begin().unwrap();
    let staging = candidate(&output, &attempt);
    let error = attempt
        .emit(&RunEvent::Note {
            message: "last callback",
        })
        .unwrap_err();
    assert_eq!(error.kind(), JobErrorKind::ObserverFailure);
    assert!(attempt
        .emit(&RunEvent::Note {
            message: "must not call"
        })
        .is_err());
    let failure = staging.fail(error);
    assert_eq!(failure.error.kind(), JobErrorKind::ObserverFailure);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(!output.exists());
}

#[test]
fn first_gate_error_wins_and_later_observer_failure_is_secondary() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let waiting = entered.clone();
    let leaving = release.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |_: &RunEvent<'_>| {
        waiting.wait();
        leaving.wait();
        Err(JobError::new(
            JobErrorKind::ObserverFailure,
            "later callback failure",
        ))
    }))));
    let attempt = control.begin().unwrap();
    thread::scope(|scope| {
        let observed = scope.spawn(|| {
            attempt.emit(&RunEvent::Note {
                message: "concurrent",
            })
        });
        entered.wait();
        assert!(control.cancellation_handle().cancel());
        release.wait();
        assert_eq!(
            observed.join().unwrap().unwrap_err().kind(),
            JobErrorKind::Cancelled
        );
    });
    let failure = attempt.fail(attempt.check().unwrap_err());
    assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
    assert_eq!(failure.secondary.len(), 1);
    assert_eq!(failure.secondary[0].kind(), JobErrorKind::ObserverFailure);
}

#[test]
fn observer_completion_records_failure_before_releasing_admission() {
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    control.0.state().active_events = 1;
    let barrier = Arc::new(Barrier::new(2));
    thread::scope(|scope| {
        let guard = control.0.state();
        let signal = barrier.clone();
        let control_ref = &control.0;
        let worker = scope.spawn(move || {
            signal.wait();
            EventAdmission {
                control: control_ref,
                active: true,
            }
            .complete(Err(JobError::new(
                JobErrorKind::ObserverFailure,
                "failure at completion",
            )))
        });
        barrier.wait();
        assert_eq!(guard.active_events, 1);
        assert!(guard.primary.is_none());
        drop(guard);
        assert_eq!(
            worker.join().unwrap().unwrap_err().kind(),
            JobErrorKind::ObserverFailure
        );
    });
    let state = control.0.state();
    assert_eq!(state.active_events, 0);
    assert_eq!(state.phase, Phase::Aborted);
    drop(state);
    assert!(attempt.permission().is_err());
}

#[test]
fn close_events_refuses_an_observer_still_in_flight() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let first = entered.clone();
    let second = release.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |_: &RunEvent<'_>| {
        first.wait();
        second.wait();
        Ok(())
    }))));
    let attempt = control.begin().unwrap();
    thread::scope(|scope| {
        let worker = scope.spawn(|| {
            attempt.emit(&RunEvent::Note {
                message: "in flight",
            })
        });
        entered.wait();
        assert_eq!(
            attempt.close_events().unwrap_err().kind(),
            JobErrorKind::InvalidState
        );
        assert!(attempt.permission().is_err());
        release.wait();
        assert!(worker.join().unwrap().is_err());
    });
}

#[test]
fn callback_reentry_rejects_same_run_without_aborting_outer_attempt() {
    let holder: Arc<OnceLock<Arc<RunControl>>> = Arc::new(OnceLock::new());
    let shared = holder.clone();
    let control = Arc::new(RunControl::new(Some(Arc::new(Watch(
        move |_: &RunEvent<'_>| {
            let failure = failed(shared.get().unwrap().begin());
            assert_eq!(failure.error.kind(), JobErrorKind::InvalidState);
            let nested = RunControl::default();
            nested.begin().unwrap().emit(&RunEvent::Note {
                message: "other run",
            })?;
            Ok(())
        },
    )))));
    holder.set(control.clone()).unwrap();
    let attempt = control.begin().unwrap();
    attempt
        .emit(&RunEvent::Note { message: "reenter" })
        .unwrap();
    attempt.check().unwrap();
    attempt.close_events().unwrap();
}

#[test]
fn a_run_cannot_be_reused_and_no_callbacks_run_after_publication() {
    let calls = Arc::new(AtomicUsize::new(0));
    let watched = calls.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |_: &RunEvent<'_>| {
        watched.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }))));
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let attempt = control.begin().unwrap();
    attempt
        .emit(&RunEvent::Note {
            message: "only event",
        })
        .unwrap();
    sealed(&output, &attempt)
        .publish(&output, OutputPolicy::CreateNew)
        .unwrap();
    assert!(attempt.emit(&RunEvent::Note { message: "late" }).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        failed(control.begin()).error.kind(),
        JobErrorKind::InvalidState
    );
    assert_eq!(fs::read(&output).unwrap(), b"complete candidate");
}

#[test]
fn a_second_candidate_cannot_publish_with_the_same_attempt() {
    let work = tempfile::tempdir().unwrap();
    let first = work.path().join("first");
    let second = work.path().join("second");
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let candidate1 = candidate(&first, &attempt);
    let candidate2 = candidate(&second, &attempt);
    attempt.close_events().unwrap();
    let sealed1 = candidate1.seal().unwrap();
    let sealed2 = candidate2.seal().unwrap();
    sealed1.publish(&first, OutputPolicy::CreateNew).unwrap();
    assert_eq!(
        failed(sealed2.publish(&second, OutputPolicy::CreateNew))
            .error
            .kind(),
        JobErrorKind::InvalidState
    );
    assert!(!second.exists());
    assert_eq!(fs::read(&first).unwrap(), b"complete candidate");
}

#[test]
fn synchronization_failure_is_fatal_and_does_not_replace_destination() {
    struct SyncFailure;
    impl FileOperations for SyncFailure {
        fn sync(&self, _: &File) -> io::Result<()> {
            Err(io::Error::other("injected sync failure"))
        }
    }
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    fs::write(&output, b"previous").unwrap();
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let staging = candidate(&output, &attempt);
    attempt.close_events().unwrap();
    let failure = failed(staging.seal_with(&SyncFailure));
    assert_eq!(failure.error.kind(), JobErrorKind::Io);
    assert_eq!(fs::read(&output).unwrap(), b"previous");
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 1);
}

#[test]
fn publisher_failure_after_permission_is_primary_and_preserves_previous_file() {
    struct InstallFailure;
    impl FileOperations for InstallFailure {
        fn install(
            &self,
            path: TempPath,
            _: &Path,
            _: OutputPolicy,
        ) -> Result<(), PathPersistError> {
            Err(PathPersistError {
                error: io::Error::other("injected install failure"),
                path,
            })
        }
    }
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    fs::write(&output, b"previous").unwrap();
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let failure = failed(sealed(&output, &attempt).publish_with(
        &output,
        OutputPolicy::Replace,
        &InstallFailure,
    ));
    assert_eq!(failure.error.kind(), JobErrorKind::Io);
    assert!(failure.error.message().contains("injected install failure"));
    assert_eq!(fs::read(&output).unwrap(), b"previous");
}

#[test]
fn cleanup_failure_retains_work_and_does_not_replace_primary_cause() {
    struct CleanupFailure;
    impl FileOperations for CleanupFailure {
        fn remove(&self, _: &Path) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected removal failure",
            ))
        }
    }
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let producer = JobError::new(JobErrorKind::InvalidInput, "producer failed");
    let failure = candidate(&output, &attempt).fail_with(producer.clone(), &CleanupFailure);
    assert!(failure.error.same_cause(&producer));
    assert_eq!(failure.secondary.len(), 1);
    assert_eq!(failure.retained_paths.len(), 1);
    assert_eq!(
        fs::read(&failure.retained_paths[0]).unwrap(),
        b"complete candidate"
    );
    assert!(!output.exists());
}

#[test]
fn successful_install_does_not_delete_a_reoccupied_temporary_name() {
    struct Reoccupy;
    impl FileOperations for Reoccupy {
        fn install(
            &self,
            path: TempPath,
            output: &Path,
            policy: OutputPolicy,
        ) -> Result<(), PathPersistError> {
            let old_name = path.to_path_buf();
            SystemFiles.install(path, output, policy)?;
            fs::write(old_name, b"unrelated owner").unwrap();
            Ok(())
        }
    }
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let published = sealed(&output, &attempt)
        .publish_with(&output, OutputPolicy::CreateNew, &Reoccupy)
        .unwrap();
    assert_eq!(published.cleanup_diagnostics.len(), 1);
    assert_eq!(
        fs::read(&published.cleanup_diagnostics[0].path).unwrap(),
        b"unrelated owner"
    );
    assert_eq!(fs::read(&output).unwrap(), b"complete candidate");
}

#[cfg(unix)]
#[test]
fn staged_and_published_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let staging = candidate(&output, &attempt);
    assert_eq!(
        staging
            .file
            .as_file()
            .metadata()
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    attempt.close_events().unwrap();
    staging
        .seal()
        .unwrap()
        .publish(&output, OutputPolicy::CreateNew)
        .unwrap();
    assert_eq!(
        fs::metadata(&output).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn concurrent_create_new_publishers_install_exactly_one_candidate() {
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let ready = Barrier::new(2);
    let outcomes = thread::scope(|scope| {
        let handles: Vec<_> = [b"first".as_slice(), b"second".as_slice()]
            .into_iter()
            .map(|bytes| {
                let output = &output;
                let ready = &ready;
                scope.spawn(move || {
                    let control = RunControl::default();
                    let attempt = control.begin().unwrap();
                    let mut staging = Staging::create(output, &attempt).unwrap();
                    staging.writer().write_all(bytes).unwrap();
                    attempt.close_events().unwrap();
                    let artifact = staging.seal().unwrap();
                    ready.wait();
                    (bytes, artifact.publish(output, OutputPolicy::CreateNew))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let winners: Vec<_> = outcomes
        .iter()
        .filter(|(_, result)| result.is_ok())
        .collect();
    assert_eq!(winners.len(), 1);
    assert_eq!(fs::read(&output).unwrap(), winners[0].0);
    for (_, outcome) in outcomes {
        if let Err(failure) = outcome {
            assert_eq!(failure.error.kind(), JobErrorKind::Conflict);
            assert!(failure.retained_paths.is_empty());
        }
    }
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 1);
}

#[test]
fn real_replace_failure_preserves_competing_directory_and_cleans_candidate() {
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("result");
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let artifact = sealed(&output, &attempt);
    fs::create_dir(&output).unwrap();
    fs::write(output.join("competitor"), b"retained directory content").unwrap();
    let failure = failed(artifact.publish(&output, OutputPolicy::Replace));
    assert!(matches!(
        failure.error.kind(),
        JobErrorKind::Io | JobErrorKind::Conflict
    ));
    assert_eq!(
        fs::read(output.join("competitor")).unwrap(),
        b"retained directory content"
    );
    assert!(failure.retained_paths.is_empty());
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 1);
}

#[test]
fn resource_refusal_retains_typed_cause_and_shared_first_abort_identity() {
    let cause = crate::content_integrity::FormatError::ResourceLimit("requested storage".into());
    let error = JobError::with_cause(JobErrorKind::ResourceLimit, "bounded codec refused", cause);
    assert!(std::error::Error::source(&error)
        .unwrap()
        .is::<crate::content_integrity::FormatError>());
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let failure = attempt.fail(error.clone());
    assert!(failure.error.same_cause(&error));
    assert_eq!(failure.error.kind(), JobErrorKind::ResourceLimit);
    assert!(!control.cancellation_handle().cancel());
}
