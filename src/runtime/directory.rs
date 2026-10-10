//! Completed-tree publication. Replace holds the current entry, then installs
//! exclusively: output may be absent between these moves. A failed install
//! restores exclusively or retains typed recovery. Concurrent attempts are not
//! serialized; Replace captures the entry at hold time, not a preflight snapshot.
//! Producers close all writers before sealing. No crash durability is promised.
use super::{
    Attempt, CleanupDiagnostic, DirectoryRecovery, JobError, JobErrorKind, JobFailure,
    OutputPolicy, Published,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

#[path = "directory_platform.rs"]
mod platform;

#[cfg(all(test, feature = "native-geospatial"))]
pub(crate) use platform::test_directory;

/// Read-only destination preparation, before any private output is created.
pub(crate) struct DirectoryTarget {
    output: PathBuf,
    policy: OutputPolicy,
}
impl DirectoryTarget {
    #[cfg(feature = "native-geospatial")]
    pub(crate) fn output(&self) -> &Path {
        &self.output
    }

    pub(crate) fn prepare(output: &Path, policy: OutputPolicy) -> Result<Self, JobError> {
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
            Ok(_) if policy == OutputPolicy::CreateNew => {
                return Err(JobError::conflict(
                    &output,
                    io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "directory output already exists",
                    ),
                ))
            }
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(JobError::io("inspect directory output", &output, e)),
        }
        platform::preflight(&parent)?;
        Ok(Self { output, policy })
    }

    pub(crate) fn stage(self, attempt: &Attempt) -> Result<DirectoryStaging<'_>, JobFailure> {
        self.stage_pending(attempt).map_err(|e| attempt.fail(e))
    }

    /// A producer with admitted live owners must close them before finalizing
    /// a failed attempt. This preparation leaves that finalization to its caller.
    pub(crate) fn stage_pending(self, attempt: &Attempt) -> Result<DirectoryStaging<'_>, JobError> {
        attempt.check()?;
        let tree = tempfile::Builder::new()
            .prefix(".tiles-dir-")
            .tempdir_in(self.output.parent().expect("prepared directory parent"))
            .map_err(|e| JobError::io("create private directory", &self.output, e))?;
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
    #[cfg(feature = "native-geospatial")]
    pub(crate) fn owned_bytes(&self) -> usize {
        // TempDir retains a Box<Path>, whose backing length is exact.
        std::mem::size_of::<Self>()
            + self.target.output.capacity()
            + self.tree.path().as_os_str().len()
    }

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
        let output = &self.target.output;
        let mut diagnostics = Vec::new();
        // Only an explicitly held entry becomes a recovery asset. TempDir Drop
        // is disabled before hold, so no implicit cleanup can delete that asset.
        let previous = if self.target.policy == OutputPolicy::Replace {
            let holder = match tempfile::Builder::new()
                .prefix(".tiles-previous-")
                .tempdir_in(output.parent().expect("prepared directory parent"))
            {
                Ok(holder) => holder.keep(),
                Err(error) => {
                    let mut failure = self.attempt.fail(JobError::io(
                        "create previous-output holder",
                        output,
                        error,
                    ));
                    cleanup(source, operations, &mut failure);
                    return Err(failure);
                }
            };
            let previous_output = holder.join("previous");
            match operations.hold(output, &previous_output) {
                Ok(()) => Some(PreviousOutput {
                    holder,
                    path: previous_output,
                }),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if let Some(diagnostic) = cleanup_empty_diagnostic(holder, operations) {
                        diagnostics.push(diagnostic);
                    }
                    None
                }
                Err(error) => {
                    let mut failure =
                        self.attempt
                            .fail(JobError::io("hold previous output", output, error));
                    cleanup_empty(holder, operations, &mut failure);
                    cleanup(source, operations, &mut failure);
                    return Err(failure);
                }
            }
        } else {
            None
        };
        match operations.install(&source, output) {
            Ok(()) => {
                self.attempt.finish();
                // This rename is the commit point. Failure to retire the previous
                // entry cannot change success. Recursive cleanup is confined to
                // our holder and never follows a held symlink's referent.
                if let Some(previous) = previous {
                    if let Some(diagnostic) = cleanup_diagnostic(previous.holder, operations) {
                        diagnostics.push(diagnostic);
                    }
                }
                Ok(Published {
                    output: self.target.output,
                    cleanup_diagnostics: diagnostics,
                })
            }
            Err(error) => {
                let cause = if error.kind() == io::ErrorKind::AlreadyExists {
                    JobError::conflict(output, error)
                } else {
                    JobError::io("install completed directory", output, error)
                };
                let mut failure = self.attempt.fail(cause);
                for diagnostic in diagnostics {
                    failure.retained_paths.push(diagnostic.path);
                    failure.secondary.push(diagnostic.error);
                }
                if let Some(previous) = previous {
                    match operations.restore(&previous.path, output) {
                        Ok(()) => cleanup_empty(previous.holder, operations, &mut failure),
                        Err(error) => {
                            failure.secondary.push(JobError::io(
                                "restore previous output",
                                output,
                                error,
                            ));
                            failure.recovery = Some(DirectoryRecovery {
                                output: output.clone(),
                                previous_output: previous.path,
                            });
                            // The holder remains intact. No Drop retry or scratch
                            // cleanup may erase the only retained previous entry.
                        }
                    }
                }
                cleanup(source, operations, &mut failure);
                Err(failure)
            }
        }
    }
}

/// Owned recovery container after the output-to-backup move succeeds.
struct PreviousOutput {
    holder: PathBuf,
    path: PathBuf,
}

trait DirectoryOperations {
    fn hold(&self, output: &Path, previous: &Path) -> io::Result<()> {
        platform::install(output, previous)
    }
    fn restore(&self, previous: &Path, output: &Path) -> io::Result<()> {
        platform::install(previous, output)
    }
    fn install(&self, source: &Path, destination: &Path) -> io::Result<()> {
        platform::install(source, destination)
    }
    fn remove_empty(&self, path: &Path) -> io::Result<()> {
        fs::remove_dir(path)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        fs::remove_dir_all(path)
    }
}
struct SystemDirectories;
impl DirectoryOperations for SystemDirectories {}

fn cleanup_empty_diagnostic(
    path: PathBuf,
    operations: &dyn DirectoryOperations,
) -> Option<CleanupDiagnostic> {
    match operations.remove_empty(&path) {
        Ok(()) => None,
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => Some(CleanupDiagnostic {
            error: JobError::io("remove empty previous-output holder", &path, error),
            path,
        }),
    }
}
fn cleanup_empty(path: PathBuf, operations: &dyn DirectoryOperations, failure: &mut JobFailure) {
    if let Some(diagnostic) = cleanup_empty_diagnostic(path, operations) {
        failure.retained_paths.push(diagnostic.path);
        failure.secondary.push(diagnostic.error);
    }
}
fn cleanup_diagnostic(
    path: PathBuf,
    operations: &dyn DirectoryOperations,
) -> Option<CleanupDiagnostic> {
    match operations.remove(&path) {
        Ok(()) => None,
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => Some(CleanupDiagnostic {
            error: JobError::io("remove private directory", &path, error),
            path,
        }),
    }
}
fn cleanup(path: PathBuf, operations: &dyn DirectoryOperations, failure: &mut JobFailure) {
    if let Some(diagnostic) = cleanup_diagnostic(path, operations) {
        failure.retained_paths.push(diagnostic.path);
        failure.secondary.push(diagnostic.error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RunControl, RunEvent};

    fn target(root: &Path) -> DirectoryTarget {
        DirectoryTarget::prepare(&root.join("output"), OutputPolicy::CreateNew).unwrap()
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

    fn replacement(root: &Path) -> DirectoryTarget {
        DirectoryTarget::prepare(&root.join("output"), OutputPolicy::Replace).unwrap()
    }
    fn old_tree(root: &Path) {
        fs::create_dir_all(root.join("output/nested")).unwrap();
        fs::write(root.join("output/nested/original"), b"original bytes").unwrap();
        fs::write(root.join("output/manifest"), b"old manifest").unwrap();
    }
    fn assert_old(root: &Path) {
        assert_eq!(
            fs::read(root.join("output/nested/original")).unwrap(),
            b"original bytes"
        );
        assert_eq!(
            fs::read(root.join("output/manifest")).unwrap(),
            b"old manifest"
        );
        assert_eq!(fs::read_dir(root.join("output")).unwrap().count(), 2);
        assert_eq!(fs::read_dir(root.join("output/nested")).unwrap().count(), 1);
    }
    #[test]
    fn replace_commits_complete_tree_and_retires_old_inventory() {
        let root = platform::test_directory();
        old_tree(root.path());
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        fs::write(stage.path().join("new"), b"new bytes").unwrap();
        attempt.close_events().unwrap();
        let result = stage.seal().unwrap().publish().unwrap();
        assert_eq!(fs::read(result.output.join("new")).unwrap(), b"new bytes");
        assert!(!result.output.join("manifest").exists());
        assert!(result.cleanup_diagnostics.is_empty());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn replace_absent_output_installs_without_recovery_asset() {
        let root = platform::test_directory();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        assert!(stage
            .seal()
            .unwrap()
            .publish()
            .unwrap()
            .cleanup_diagnostics
            .is_empty());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    struct HoldFailure {
        collision: bool,
    }
    impl DirectoryOperations for HoldFailure {
        fn hold(&self, output: &Path, previous: &Path) -> io::Result<()> {
            if self.collision {
                fs::write(previous, b"other writer")?;
                platform::install(output, previous)
            } else {
                Err(io::Error::other("injected hold failure"))
            }
        }
        fn install(&self, _: &Path, _: &Path) -> io::Result<()> {
            panic!("hold failure must prevent install")
        }
    }
    #[test]
    fn hold_failure_preserves_output_and_backup_collision() {
        for collision in [false, true] {
            let root = platform::test_directory();
            old_tree(root.path());
            let run = RunControl::default();
            let attempt = run.begin().unwrap();
            let stage = replacement(root.path()).stage(&attempt).unwrap();
            let private = stage.path().to_owned();
            attempt.close_events().unwrap();
            let failure = stage
                .seal()
                .unwrap()
                .publish_with(&HoldFailure { collision })
                .err()
                .unwrap();
            assert_eq!(failure.error.kind(), JobErrorKind::Io);
            assert!(failure.recovery.is_none());
            assert_old(root.path());
            assert!(!private.exists());
            if collision {
                assert_eq!(failure.retained_paths.len(), 1);
                assert_eq!(
                    fs::read(failure.retained_paths[0].join("previous")).unwrap(),
                    b"other writer"
                );
            }
        }
    }
    struct InstallFailure {
        restore_fails: bool,
        competitor: bool,
    }
    impl DirectoryOperations for InstallFailure {
        fn install(&self, _: &Path, output: &Path) -> io::Result<()> {
            assert!(!output.exists(), "hold creates a visible absence interval");
            if self.competitor {
                fs::create_dir(output)?;
                fs::write(output.join("competitor"), b"preserve")?;
            }
            Err(io::Error::other("injected install failure"))
        }
        fn restore(&self, previous: &Path, output: &Path) -> io::Result<()> {
            if self.restore_fails {
                Err(io::Error::other("injected restore failure"))
            } else {
                platform::install(previous, output)
            }
        }
    }
    #[test]
    fn install_failure_restores_original_inventory_and_bytes() {
        let root = platform::test_directory();
        old_tree(root.path());
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        let failure = stage
            .seal()
            .unwrap()
            .publish_with(&InstallFailure {
                restore_fails: false,
                competitor: false,
            })
            .err()
            .unwrap();
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
        assert!(failure.error.message().contains("injected install failure"));
        assert!(failure.recovery.is_none());
        assert!(failure.secondary.is_empty());
        assert_old(root.path());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn failed_install_restores_held_file_bytes() {
        let root = platform::test_directory();
        let output = root.path().join("output");
        fs::write(&output, b"exact original file bytes").unwrap();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        let failure = stage
            .seal()
            .unwrap()
            .publish_with(&InstallFailure {
                restore_fails: false,
                competitor: false,
            })
            .err()
            .unwrap();
        assert!(failure.recovery.is_none());
        assert!(failure.secondary.is_empty());
        assert!(fs::symlink_metadata(&output).unwrap().is_file());
        assert_eq!(fs::read(output).unwrap(), b"exact original file bytes");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn failed_install_restores_held_symlink_identity_without_following_referent() {
        let root = platform::test_directory();
        let referent = root.path().join("referent");
        fs::create_dir(&referent).unwrap();
        fs::write(referent.join("safe"), b"preserved referent").unwrap();
        let output = root.path().join("output");
        // Relative link text must survive unchanged, rather than being resolved.
        std::os::unix::fs::symlink("referent", &output).unwrap();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        let failure = stage
            .seal()
            .unwrap()
            .publish_with(&InstallFailure {
                restore_fails: false,
                competitor: false,
            })
            .err()
            .unwrap();
        assert!(failure.recovery.is_none());
        assert!(failure.secondary.is_empty());
        assert!(fs::symlink_metadata(&output)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_link(output).unwrap(), Path::new("referent"));
        assert_eq!(
            fs::read(referent.join("safe")).unwrap(),
            b"preserved referent"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[cfg(windows)]
    #[test]
    fn failed_install_restores_held_windows_directory_symlink() {
        let root = platform::test_directory();
        let referent = root.path().join("referent");
        fs::create_dir(&referent).unwrap();
        fs::write(referent.join("safe"), b"preserved referent").unwrap();
        let output = root.path().join("output");
        std::os::windows::fs::symlink_dir(&referent, &output)
            .expect("Windows symlink fixture requires symbolic-link privilege or Developer Mode");
        let original_link = fs::read_link(&output).unwrap();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        let failure = stage
            .seal()
            .unwrap()
            .publish_with(&InstallFailure {
                restore_fails: false,
                competitor: false,
            })
            .err()
            .unwrap();
        assert!(failure.recovery.is_none());
        assert!(failure.secondary.is_empty());
        assert!(fs::symlink_metadata(&output)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_link(output).unwrap(), original_link);
        assert_eq!(
            fs::read(referent.join("safe")).unwrap(),
            b"preserved referent"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[cfg(windows)]
    #[test]
    fn replacement_retires_windows_directory_symlink_without_following_referent() {
        let root = platform::test_directory();
        let referent = root.path().join("referent");
        fs::create_dir(&referent).unwrap();
        fs::write(referent.join("safe"), b"preserved referent").unwrap();
        let output = root.path().join("output");
        std::os::windows::fs::symlink_dir(&referent, &output)
            .expect("Windows symlink fixture requires symbolic-link privilege or Developer Mode");
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        fs::write(stage.path().join("new"), b"committed").unwrap();
        attempt.close_events().unwrap();
        let result = stage.seal().unwrap().publish().unwrap();
        assert!(result.cleanup_diagnostics.is_empty());
        assert!(!fs::symlink_metadata(&output)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(output.join("new")).unwrap(), b"committed");
        assert_eq!(
            fs::read(referent.join("safe")).unwrap(),
            b"preserved referent"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn successful_restore_preserves_original_directory_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = platform::test_directory();
        old_tree(root.path());
        let output = root.path().join("output");
        fs::set_permissions(&output, fs::Permissions::from_mode(0o750)).unwrap();
        fs::set_permissions(output.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        let failure = stage
            .seal()
            .unwrap()
            .publish_with(&InstallFailure {
                restore_fails: false,
                competitor: false,
            })
            .err()
            .unwrap();
        assert!(failure.recovery.is_none());
        assert_old(root.path());
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o7777,
            0o750
        );
        assert_eq!(
            fs::metadata(output.join("nested"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
    }

    #[test]
    fn failed_restore_retains_typed_original_and_preserves_competitor() {
        for competitor in [false, true] {
            let root = platform::test_directory();
            old_tree(root.path());
            let run = RunControl::default();
            let attempt = run.begin().unwrap();
            let stage = replacement(root.path()).stage(&attempt).unwrap();
            attempt.close_events().unwrap();
            let failure = stage
                .seal()
                .unwrap()
                .publish_with(&InstallFailure {
                    restore_fails: !competitor,
                    competitor,
                })
                .err()
                .unwrap();
            assert!(failure.error.message().contains("injected install failure"));
            assert_eq!(failure.secondary.len(), 1);
            let recovery = failure.recovery.as_ref().unwrap();
            assert_eq!(
                recovery.output,
                fs::canonicalize(root.path()).unwrap().join("output")
            );
            assert_eq!(
                fs::read(recovery.previous_output.join("nested/original")).unwrap(),
                b"original bytes"
            );
            assert!(failure.retained_paths.is_empty());
            if competitor {
                assert_eq!(
                    fs::read(root.path().join("output/competitor")).unwrap(),
                    b"preserve"
                );
            } else {
                assert!(!root.path().join("output").exists());
            }
            assert!(failure.to_string().contains("restore to"));
        }
    }
    #[derive(Clone, Copy)]
    enum RestoreCompetitor {
        EmptyDirectory,
        File,
        #[cfg(unix)]
        Symlink,
    }
    struct OccupiedRestore(RestoreCompetitor);
    impl DirectoryOperations for OccupiedRestore {
        fn install(&self, _: &Path, _: &Path) -> io::Result<()> {
            Err(io::Error::other("injected install failure"))
        }
        fn restore(&self, previous: &Path, output: &Path) -> io::Result<()> {
            match self.0 {
                RestoreCompetitor::EmptyDirectory => fs::create_dir(output)?,
                RestoreCompetitor::File => fs::write(output, b"preserve file")?,
                #[cfg(unix)]
                RestoreCompetitor::Symlink => {
                    std::os::unix::fs::symlink("missing referent", output)?
                }
            }
            platform::install(previous, output)
        }
    }
    #[test]
    fn restore_preserves_empty_directory_file_and_symlink_competitors() {
        let competitors = [
            RestoreCompetitor::EmptyDirectory,
            RestoreCompetitor::File,
            #[cfg(unix)]
            RestoreCompetitor::Symlink,
        ];
        for competitor in competitors {
            let root = platform::test_directory();
            old_tree(root.path());
            let run = RunControl::default();
            let attempt = run.begin().unwrap();
            let stage = replacement(root.path()).stage(&attempt).unwrap();
            attempt.close_events().unwrap();
            let failure = stage
                .seal()
                .unwrap()
                .publish_with(&OccupiedRestore(competitor))
                .err()
                .unwrap();
            assert!(failure.error.message().contains("injected install failure"));
            assert_eq!(failure.secondary.len(), 1);
            let recovery = failure.recovery.unwrap();
            assert_eq!(
                fs::read(recovery.previous_output.join("nested/original")).unwrap(),
                b"original bytes"
            );
            let output = root.path().join("output");
            match competitor {
                RestoreCompetitor::EmptyDirectory => {
                    assert_eq!(fs::read_dir(output).unwrap().count(), 0)
                }
                RestoreCompetitor::File => assert_eq!(fs::read(output).unwrap(), b"preserve file"),
                #[cfg(unix)]
                RestoreCompetitor::Symlink => assert_eq!(
                    fs::read_link(output).unwrap(),
                    Path::new("missing referent")
                ),
            }
        }
    }
    struct CancelDuringRecovery(crate::CancellationHandle);
    impl DirectoryOperations for CancelDuringRecovery {
        fn install(&self, _: &Path, _: &Path) -> io::Result<()> {
            assert!(
                !self.0.cancel(),
                "failed install remains publication's outcome"
            );
            Err(io::Error::other("injected install failure"))
        }
        fn restore(&self, previous: &Path, output: &Path) -> io::Result<()> {
            assert!(
                !self.0.cancel(),
                "restoration remains publication's outcome"
            );
            platform::install(previous, output)
        }
    }
    #[test]
    fn cancellation_during_failed_install_and_restore_cannot_replace_primary_cause() {
        let root = platform::test_directory();
        old_tree(root.path());
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        let failure = stage
            .seal()
            .unwrap()
            .publish_with(&CancelDuringRecovery(run.cancellation_handle()))
            .err()
            .unwrap();
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
        assert!(failure.error.message().contains("injected install failure"));
        assert!(failure.recovery.is_none());
        assert!(failure.secondary.is_empty());
        assert_old(root.path());
    }

    struct BackupCleanupFailure;
    impl DirectoryOperations for BackupCleanupFailure {
        fn remove(&self, path: &Path) -> io::Result<()> {
            if path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".tiles-previous-")
            {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected backup cleanup failure",
                ))
            } else {
                fs::remove_dir_all(path)
            }
        }
    }
    #[test]
    fn backup_cleanup_failure_preserves_committed_success_and_old_copy() {
        let root = platform::test_directory();
        old_tree(root.path());
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        fs::write(stage.path().join("new"), b"committed").unwrap();
        attempt.close_events().unwrap();
        let result = stage
            .seal()
            .unwrap()
            .publish_with(&BackupCleanupFailure)
            .unwrap();
        assert_eq!(result.cleanup_diagnostics.len(), 1);
        assert_eq!(fs::read(result.output.join("new")).unwrap(), b"committed");
        assert_eq!(
            fs::read(
                result.cleanup_diagnostics[0]
                    .path
                    .join("previous/nested/original")
            )
            .unwrap(),
            b"original bytes"
        );
        assert!(!run.cancellation_handle().cancel());
    }
    struct ReuseRestoredName;
    impl DirectoryOperations for ReuseRestoredName {
        fn install(&self, _: &Path, _: &Path) -> io::Result<()> {
            Err(io::Error::other("injected install failure"))
        }
        fn restore(&self, previous: &Path, output: &Path) -> io::Result<()> {
            platform::install(previous, output)?;
            fs::create_dir(previous)?;
            fs::write(previous.join("competitor"), b"preserve")
        }
    }
    #[test]
    fn successful_restore_preserves_reused_previous_name() {
        let root = platform::test_directory();
        old_tree(root.path());
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        let failure = stage
            .seal()
            .unwrap()
            .publish_with(&ReuseRestoredName)
            .err()
            .unwrap();
        assert_old(root.path());
        assert!(failure.recovery.is_none());
        assert_eq!(failure.retained_paths.len(), 1);
        assert_eq!(
            fs::read(failure.retained_paths[0].join("previous/competitor")).unwrap(),
            b"preserve"
        );
    }
    struct PanicAfterHold;
    impl DirectoryOperations for PanicAfterHold {
        fn install(&self, _: &Path, _: &Path) -> io::Result<()> {
            panic!("injected unwind after hold")
        }
    }
    #[test]
    fn unwind_after_hold_does_not_drop_the_original_recovery_asset() {
        let root = platform::test_directory();
        old_tree(root.path());
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stage
                .seal()
                .unwrap()
                .publish_with(&PanicAfterHold)))
            .is_err()
        );
        let holder = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".tiles-previous-")
            })
            .unwrap();
        assert_eq!(
            fs::read(holder.join("previous/nested/original")).unwrap(),
            b"original bytes"
        );
        assert!(!root.path().join("output").exists());
    }
    #[test]
    fn replacement_cancelled_before_permission_never_holds_original() {
        let root = platform::test_directory();
        old_tree(root.path());
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        let sealed = stage.seal().unwrap();
        assert!(run.cancellation_handle().cancel());
        let failure = sealed.publish().err().unwrap();
        assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
        assert!(failure.recovery.is_none());
        assert_old(root.path());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    struct CancelAfterPermission(crate::CancellationHandle);
    impl DirectoryOperations for CancelAfterPermission {
        fn hold(&self, output: &Path, previous: &Path) -> io::Result<()> {
            assert!(!self.0.cancel());
            platform::install(output, previous)
        }
        fn install(&self, source: &Path, output: &Path) -> io::Result<()> {
            assert!(!self.0.cancel());
            platform::install(source, output)
        }
    }
    #[test]
    fn replacement_owns_outcome_after_permission_through_hold_and_install() {
        let root = platform::test_directory();
        old_tree(root.path());
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        fs::write(stage.path().join("new"), b"new bytes").unwrap();
        attempt.close_events().unwrap();
        let result = stage
            .seal()
            .unwrap()
            .publish_with(&CancelAfterPermission(run.cancellation_handle()))
            .unwrap();
        assert_eq!(fs::read(result.output.join("new")).unwrap(), b"new bytes");
        assert!(!run.cancellation_handle().cancel());
    }

    #[test]
    fn replacement_retires_file_entry() {
        let root = platform::test_directory();
        fs::write(root.path().join("output"), b"old file").unwrap();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        stage.seal().unwrap().publish().unwrap();
        assert!(root.path().join("output").is_dir());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn replacement_retires_symlink_without_following_referent() {
        let root = platform::test_directory();
        let referent = root.path().join("referent");
        fs::create_dir(&referent).unwrap();
        fs::write(referent.join("safe"), b"preserve").unwrap();
        std::os::unix::fs::symlink(&referent, root.path().join("output")).unwrap();
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        attempt.close_events().unwrap();
        stage.seal().unwrap().publish().unwrap();
        assert!(!fs::symlink_metadata(root.path().join("output"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(referent.join("safe")).unwrap(), b"preserve");
    }

    fn wait_for_marker(path: &Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    // A subprocess entry point, not an evidence test on its own. Its caller
    // supplies a supported fixture and checks the subprocess result.
    #[test]
    fn concurrent_replacement_child() {
        let Some(root) = std::env::var_os("TILES_D2_CHILD_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        wait_for_marker(&root.join("a-held"));
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(&root).stage(&attempt).unwrap();
        fs::write(stage.path().join("child"), b"child committed").unwrap();
        attempt.close_events().unwrap();
        stage.seal().unwrap().publish().unwrap();
        fs::write(root.join("b-committed"), b"ready").unwrap();
    }
    fn replacement_child(root: &Path) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::directory::tests::concurrent_replacement_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TILES_D2_CHILD_ROOT", root)
            .spawn()
            .unwrap()
    }
    struct WaitForChild {
        root: PathBuf,
    }
    impl DirectoryOperations for WaitForChild {
        fn hold(&self, output: &Path, previous: &Path) -> io::Result<()> {
            platform::install(output, previous)?;
            fs::write(self.root.join("a-held"), b"ready")
        }
        fn install(&self, source: &Path, output: &Path) -> io::Result<()> {
            wait_for_marker(&self.root.join("b-committed"));
            platform::install(source, output)
        }
    }
    #[test]
    fn separate_process_commits_during_absence_and_failed_restore_preserves_it() {
        let root = platform::test_directory();
        old_tree(root.path());
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        fs::write(stage.path().join("parent"), b"parent candidate").unwrap();
        attempt.close_events().unwrap();
        let mut child = replacement_child(root.path());
        let failure = stage
            .seal()
            .unwrap()
            .publish_with(&WaitForChild {
                root: root.path().to_owned(),
            })
            .err()
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(failure.error.kind(), JobErrorKind::Conflict);
        assert_eq!(failure.secondary.len(), 1);
        let recovery = failure.recovery.unwrap();
        assert_eq!(
            fs::read(recovery.previous_output.join("nested/original")).unwrap(),
            b"original bytes"
        );
        assert_eq!(
            fs::read(root.path().join("output/child")).unwrap(),
            b"child committed"
        );
        assert!(!root.path().join("output/parent").exists());
    }
    #[test]
    fn separate_process_may_replace_an_already_committed_output() {
        let root = platform::test_directory();
        old_tree(root.path());
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        let stage = replacement(root.path()).stage(&attempt).unwrap();
        fs::write(stage.path().join("parent"), b"parent committed").unwrap();
        attempt.close_events().unwrap();
        stage.seal().unwrap().publish().unwrap();
        assert_eq!(
            fs::read(root.path().join("output/parent")).unwrap(),
            b"parent committed"
        );
        fs::write(root.path().join("a-held"), b"ready").unwrap();
        let mut child = replacement_child(root.path());
        assert!(child.wait().unwrap().success());
        assert_eq!(
            fs::read(root.path().join("output/child")).unwrap(),
            b"child committed"
        );
        assert!(!root.path().join("output/parent").exists());
        assert!(!fs::read_dir(root.path()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tiles-previous-")));
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
