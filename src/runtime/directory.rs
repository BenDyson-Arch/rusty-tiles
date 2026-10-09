//! D1 completed-tree CreateNew publication. Producers own contents and close all
//! writers before sealing; directory replacement is a separate contract.
use super::{Attempt, CleanupDiagnostic, JobError, JobErrorKind, JobFailure, Published};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

#[path = "directory_platform.rs"]
mod platform;

/// Read-only destination preparation, before any private output is created.
pub(crate) struct DirectoryTarget {
    output: PathBuf,
}
impl DirectoryTarget {
    pub(crate) fn prepare(output: &Path) -> Result<Self, JobError> {
        let name = output.file_name().ok_or_else(|| {
            JobError::new(
                JobErrorKind::InvalidRequest,
                "directory output requires a final name",
            )
        })?;
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = fs::canonicalize(parent)
            .map_err(|e| JobError::io("resolve existing directory output parent", parent, e))?;
        if !parent.is_dir() {
            return Err(JobError::new(
                JobErrorKind::InvalidRequest,
                "directory output parent must be an existing directory",
            ));
        }
        let output = parent.join(name);
        match fs::symlink_metadata(&output) {
            Ok(_) => {
                return Err(JobError::conflict(
                    &output,
                    io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "directory output already exists",
                    ),
                ))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(JobError::io("inspect directory output", &output, e)),
        }
        platform::preflight(&parent)?;
        Ok(Self { output })
    }

    pub(crate) fn stage(self, attempt: &Attempt) -> Result<DirectoryStaging<'_>, JobFailure> {
        attempt.check().map_err(|e| attempt.fail(e))?;
        let tree = tempfile::Builder::new()
            .prefix(".tiles-dir-")
            .tempdir_in(self.output.parent().expect("prepared directory parent"))
            .map_err(|e| attempt.fail(JobError::io("create private directory", &self.output, e)))?;
        Ok(DirectoryStaging {
            tree,
            target: self,
            attempt,
        })
    }
}

pub(crate) struct DirectoryStaging<'a> {
    tree: TempDir,
    target: DirectoryTarget,
    attempt: &'a Attempt,
}
impl<'a> DirectoryStaging<'a> {
    pub(crate) fn path(&self) -> &Path {
        self.tree.path()
    }

    pub(crate) fn fail(self, error: JobError) -> JobFailure {
        self.fail_with(error, &SystemDirectories)
    }
    fn fail_with(self, error: JobError, operations: &dyn DirectoryOperations) -> JobFailure {
        let mut failure = self.attempt.fail(error);
        cleanup(self.tree.keep(), operations, &mut failure);
        failure
    }

    pub(crate) fn seal(self) -> Result<SealedDirectory<'a>, JobFailure> {
        if let Err(e) = self.attempt.check_sealable() {
            return Err(self.fail(e));
        }
        Ok(SealedDirectory {
            tree: self.tree,
            target: self.target,
            attempt: self.attempt,
        })
    }
}

pub(crate) struct SealedDirectory<'a> {
    tree: TempDir,
    target: DirectoryTarget,
    attempt: &'a Attempt,
}
impl SealedDirectory<'_> {
    pub(crate) fn publish(self) -> Result<Published, JobFailure> {
        self.publish_with(&SystemDirectories)
    }
    fn publish_with(self, operations: &dyn DirectoryOperations) -> Result<Published, JobFailure> {
        // Disable implicit Drop cleanup before the namespace can change. After
        // success the old private name may belong to someone else.
        let source = self.tree.keep();
        if let Err(e) = self.attempt.permission() {
            let mut failure = self.attempt.fail(e);
            cleanup(source, operations, &mut failure);
            return Err(failure);
        }
        match operations.install(&source, &self.target.output) {
            Ok(()) => {
                self.attempt.finish();
                Ok(Published {
                    output: self.target.output,
                    cleanup_diagnostics: Vec::new(),
                })
            }
            Err(e) => {
                let error = if e.kind() == io::ErrorKind::AlreadyExists {
                    JobError::conflict(&self.target.output, e)
                } else {
                    JobError::io("install completed directory", &self.target.output, e)
                };
                let mut failure = self.attempt.fail(error);
                cleanup(source, operations, &mut failure);
                Err(failure)
            }
        }
    }
}

trait DirectoryOperations {
    fn install(&self, source: &Path, destination: &Path) -> io::Result<()> {
        platform::install(source, destination)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        fs::remove_dir_all(path)
    }
}
struct SystemDirectories;
impl DirectoryOperations for SystemDirectories {}

fn cleanup(path: PathBuf, operations: &dyn DirectoryOperations, failure: &mut JobFailure) {
    if let Err(e) = operations.remove(&path) {
        if e.kind() != io::ErrorKind::NotFound {
            let diagnostic = CleanupDiagnostic {
                error: JobError::io("remove private directory", &path, e),
                path,
            };
            failure.retained_paths.push(diagnostic.path);
            failure.secondary.push(diagnostic.error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RunControl, RunEvent};

    fn target(root: &Path) -> DirectoryTarget {
        DirectoryTarget::prepare(&root.join("output")).unwrap()
    }

    #[test]
    fn tree_is_invisible_until_single_commit_and_control_is_terminal() {
        let root = platform::test_directory();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = target(root.path()).stage(&attempt).unwrap();
        fs::create_dir(stage.path().join("nested")).unwrap();
        fs::write(stage.path().join("nested/tile"), b"complete").unwrap();
        assert!(!root.path().join("output").exists());
        attempt.close_events().unwrap();
        let published = stage.seal().unwrap().publish().unwrap();
        assert_eq!(
            fs::read(published.output.join("nested/tile")).unwrap(),
            b"complete"
        );
        assert!(!run.cancellation_handle().cancel());
        assert!(run.begin().is_err());
    }

    struct CancelDuringInstall(crate::runtime::CancellationHandle);
    impl DirectoryOperations for CancelDuringInstall {
        fn install(&self, source: &Path, destination: &Path) -> io::Result<()> {
            assert!(!self.0.cancel(), "publication permission owns the outcome");
            platform::install(source, destination)
        }
    }
    #[test]
    fn cancellation_during_install_cannot_abort_committed_publication() {
        let root = platform::test_directory();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = target(root.path()).stage(&attempt).unwrap();
        fs::write(stage.path().join("tile"), b"complete").unwrap();
        attempt.close_events().unwrap();
        let result = stage
            .seal()
            .unwrap()
            .publish_with(&CancelDuringInstall(run.cancellation_handle()))
            .unwrap();
        assert_eq!(fs::read(result.output.join("tile")).unwrap(), b"complete");
        assert!(!run.cancellation_handle().cancel());
    }

    struct ReusePrivateName {
        directory: bool,
    }
    impl DirectoryOperations for ReusePrivateName {
        fn install(&self, source: &Path, destination: &Path) -> io::Result<()> {
            platform::install(source, destination)?;
            if self.directory {
                fs::create_dir(source)?;
                fs::write(source.join("other-writer"), b"preserve")?;
            } else {
                fs::write(source, b"preserve")?;
            }
            Ok(())
        }
    }
    #[test]
    fn success_preserves_competitor_reusing_the_private_name() {
        for directory in [false, true] {
            let root = platform::test_directory();
            let run = RunControl::default();
            let attempt = run.begin().unwrap();
            let stage = target(root.path()).stage(&attempt).unwrap();
            let private = stage.path().to_owned();
            fs::write(stage.path().join("tile"), b"complete").unwrap();
            attempt.close_events().unwrap();
            let result = stage
                .seal()
                .unwrap()
                .publish_with(&ReusePrivateName { directory })
                .unwrap();
            assert_eq!(fs::read(result.output.join("tile")).unwrap(), b"complete");
            let competitor = if directory {
                private.join("other-writer")
            } else {
                private
            };
            assert_eq!(fs::read(competitor).unwrap(), b"preserve");
        }
    }

    struct Competitor {
        nonempty: bool,
    }
    impl DirectoryOperations for Competitor {
        fn install(&self, source: &Path, destination: &Path) -> io::Result<()> {
            fs::create_dir(destination)?;
            if self.nonempty {
                fs::write(destination.join("original"), b"competitor")?;
            }
            platform::install(source, destination)
        }
    }
    #[test]
    fn empty_and_nonempty_competitors_at_install_are_untouched() {
        for nonempty in [false, true] {
            let root = platform::test_directory();
            let run = RunControl::default();
            let attempt = run.begin().unwrap();
            let stage = target(root.path()).stage(&attempt).unwrap();
            let private = stage.path().to_owned();
            fs::write(stage.path().join("candidate"), b"new").unwrap();
            attempt.close_events().unwrap();
            let failure = stage
                .seal()
                .unwrap()
                .publish_with(&Competitor { nonempty })
                .err()
                .unwrap();
            assert_eq!(failure.error.kind(), JobErrorKind::Conflict);
            assert_eq!(
                failure.error.path(),
                Some(
                    fs::canonicalize(root.path())
                        .unwrap()
                        .join("output")
                        .as_path()
                )
            );
            assert!(std::error::Error::source(&failure.error).is_some());
            assert!(!private.exists());
            assert!(!root.path().join("output/candidate").exists());
            if nonempty {
                assert_eq!(
                    fs::read(root.path().join("output/original")).unwrap(),
                    b"competitor"
                );
            } else {
                assert_eq!(fs::read_dir(root.path().join("output")).unwrap().count(), 0);
            }
        }
    }

    struct FailedInstall;
    impl DirectoryOperations for FailedInstall {
        fn install(&self, _: &Path, _: &Path) -> io::Result<()> {
            Err(io::Error::other("injected install failure"))
        }
        fn remove(&self, _: &Path) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected cleanup failure",
            ))
        }
    }
    #[test]
    fn failed_cleanup_is_retained_without_drop_retry() {
        let root = platform::test_directory();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = target(root.path()).stage(&attempt).unwrap();
        let private = stage.path().to_owned();
        fs::write(private.join("candidate"), b"new").unwrap();
        attempt.close_events().unwrap();
        let failure = stage
            .seal()
            .unwrap()
            .publish_with(&FailedInstall)
            .err()
            .unwrap();
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
        assert_eq!(failure.retained_paths, vec![private.clone()]);
        assert_eq!(failure.secondary.len(), 1);
        assert_eq!(fs::read(private.join("candidate")).unwrap(), b"new");
        assert!(!root.path().join("output").exists());
    }

    #[test]
    fn cancellation_after_seal_prevents_install_and_cleans_tree() {
        let root = platform::test_directory();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = target(root.path()).stage(&attempt).unwrap();
        let private = stage.path().to_owned();
        attempt.close_events().unwrap();
        let sealed = stage.seal().unwrap();
        assert!(run.cancellation_handle().cancel());
        let failure = sealed.publish().err().unwrap();
        assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
        assert!(!private.exists());
        assert!(!root.path().join("output").exists());
    }

    #[test]
    fn unclosed_events_prevent_sealing() {
        let root = platform::test_directory();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = target(root.path()).stage(&attempt).unwrap();
        let private = stage.path().to_owned();
        let failure = stage.seal().err().unwrap();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidState);
        assert!(!private.exists());
        assert!(attempt.emit(&RunEvent::Note { message: "late" }).is_err());
    }
}
