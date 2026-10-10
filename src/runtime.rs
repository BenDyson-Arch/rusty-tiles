//! File and completed-directory staging with one-attempt publication control. Format producers own their
//! workers, encoding, receipts, and finalization; this module owns none of them.
use std::{
    error::Error as StdError,
    fmt,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};
use tempfile::{NamedTempFile, PathPersistError, TempPath};

/// Domain failure classifications; adapters choose their presentation/status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobErrorKind {
    InvalidRequest,
    InvalidInput,
    Unsupported,
    ResourceLimit,
    Io,
    Conflict,
    Cancelled,
    ObserverFailure,
    InvalidState,
}

#[derive(Debug)]
struct ErrorDetail {
    kind: JobErrorKind,
    message: String,
    path: Option<PathBuf>,
    source: Option<Arc<dyn StdError + Send + Sync>>,
}

/// A causal domain error, cheaply cloned when choosing the first abort cause.
#[derive(Clone, Debug)]
pub struct JobError(Arc<ErrorDetail>);

impl JobError {
    pub fn new(kind: JobErrorKind, message: impl Into<String>) -> Self {
        Self(Arc::new(ErrorDetail {
            kind,
            message: message.into(),
            path: None,
            source: None,
        }))
    }

    /// Retain a typed non-I/O cause in the existing causal error owner.
    pub(crate) fn with_cause(
        kind: JobErrorKind,
        message: impl Into<String>,
        cause: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(ErrorDetail {
            kind,
            message: message.into(),
            path: None,
            source: Some(Arc::new(cause)),
        }))
    }

    pub fn io(operation: &str, path: &Path, error: io::Error) -> Self {
        Self(Arc::new(ErrorDetail {
            kind: JobErrorKind::Io,
            message: format!("{operation}: {error}"),
            path: Some(path.to_owned()),
            source: Some(Arc::new(error)),
        }))
    }

    pub(crate) fn conflict(path: &Path, error: io::Error) -> Self {
        Self(Arc::new(ErrorDetail {
            kind: JobErrorKind::Conflict,
            message: "output already exists".into(),
            path: Some(path.to_owned()),
            source: Some(Arc::new(error)),
        }))
    }

    pub fn kind(&self) -> JobErrorKind {
        self.0.kind
    }

    pub fn message(&self) -> &str {
        &self.0.message
    }

    pub fn path(&self) -> Option<&Path> {
        self.0.path.as_deref()
    }

    fn observer(error: JobError) -> Self {
        if error.kind() == JobErrorKind::ObserverFailure {
            return error;
        }
        Self(Arc::new(ErrorDetail {
            kind: JobErrorKind::ObserverFailure,
            message: error.to_string(),
            path: error.0.path.clone(),
            source: Some(Arc::new(error)),
        }))
    }

    pub(crate) fn same_cause(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())?;
        if let Some(path) = self.path() {
            write!(f, " ({})", path.display())?;
        }
        Ok(())
    }
}

impl StdError for JobError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source.as_deref().map(|error| error as &dyn StdError)
    }
}

/// Failure before successful installation, including actionable cleanup state.
/// Failed directory restoration reports the preserved original separately from scratch.
#[derive(Clone, Debug)]
pub struct JobFailure {
    pub error: JobError,
    pub secondary: Vec<JobError>,
    pub retained_paths: Vec<PathBuf>,
    pub recovery: Option<DirectoryRecovery>,
}

/// An original final-path entry retained after exclusive restoration failed.
/// Restore it to `output` only after resolving the entry currently at that path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryRecovery {
    pub output: PathBuf,
    pub previous_output: PathBuf,
}

impl fmt::Display for JobFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)?;
        for path in &self.retained_paths {
            write!(f, "; retained work at {}", path.display())?;
        }
        if let Some(recovery) = &self.recovery {
            write!(f, "; previous output retained at {}; restore to {} after resolving any current output", recovery.previous_output.display(), recovery.output.display())?;
        }
        Ok(())
    }
}

impl StdError for JobFailure {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.error)
    }
}

/// A committed artifact remains successful when removal of a temporary name fails.
#[derive(Clone, Debug)]
pub struct CleanupDiagnostic {
    pub path: PathBuf,
    pub error: JobError,
}

/// Installation policy for a completed artifact. Neither policy promises crash durability.
/// Directory Replace holds the current entry before exclusive installation and may
/// temporarily leave the output absent; failures restore or report typed recovery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputPolicy {
    #[default]
    CreateNew,
    Replace,
}

/// Transient producer events. Receivers copy borrowed data if retaining it.
#[derive(Clone, Copy, Debug)]
pub enum RunEvent<'a> {
    Progress {
        phase: &'a str,
        done: u64,
        total: Option<u64>,
    },
    Warning {
        code: &'a str,
        message: &'a str,
    },
    Note {
        message: &'a str,
    },
}

/// Synchronous, fallible observation. Calls may come from producer threads.
/// Implementations must not panic. No runtime lock is held during a call.
pub trait Observer: Send + Sync {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Open,
    Aborted,
    Publishing,
    Finished,
}

struct State {
    claimed: bool,
    phase: Phase,
    events_closed: bool,
    active_events: usize,
    primary: Option<JobError>,
    secondary: Vec<JobError>,
}

struct Control {
    state: Mutex<State>,
    observer: Option<Arc<dyn Observer>>,
}

impl Control {
    fn state(&self) -> MutexGuard<'_, State> {
        // No caller code runs under this mutex. A poisoned lock must still
        // permit cleanup after an unexpected unwind.
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

/// One operation's event admission and abort/commit arbitration. A control
/// cannot be reused, including after validation failure. Default is silent.
pub struct RunControl(Arc<Control>);

impl fmt::Debug for RunControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunControl")
            .field("phase", &self.0.state().phase)
            .finish_non_exhaustive()
    }
}

impl Default for RunControl {
    fn default() -> Self {
        Self::new(None)
    }
}

impl RunControl {
    pub fn new(observer: Option<Arc<dyn Observer>>) -> Self {
        Self(Arc::new(Control {
            state: Mutex::new(State {
                claimed: false,
                phase: Phase::Open,
                events_closed: false,
                active_events: 0,
                primary: None,
                secondary: Vec::new(),
            }),
            observer,
        }))
    }

    pub fn cancellation_handle(&self) -> CancellationHandle {
        CancellationHandle(self.0.clone())
    }

    pub(crate) fn begin(&self) -> Result<Attempt, JobFailure> {
        let mut state = self.0.state();
        if state.claimed {
            // Reentry/reuse must not abort the attempt that already owns control.
            return Err(JobFailure {
                error: invalid_state("run control has already been used"),
                secondary: Vec::new(),
                retained_paths: Vec::new(),
                recovery: None,
            });
        }
        state.claimed = true;
        Ok(Attempt {
            control: self.0.clone(),
        })
    }
}

/// A thread-safe cancellation input; cancelling after publication permission
/// returns false and cannot rewrite the committed core outcome.
#[derive(Clone)]
pub struct CancellationHandle(Arc<Control>);

impl CancellationHandle {
    pub fn cancel(&self) -> bool {
        let mut state = self.0.state();
        if state.phase != Phase::Open {
            return false;
        }
        record_abort(
            &mut state,
            JobError::new(JobErrorKind::Cancelled, "operation cancelled"),
        )
    }
}

fn invalid_state(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}

fn record_abort(state: &mut State, error: JobError) -> bool {
    match state.phase {
        Phase::Open => {
            state.primary = Some(error.clone());
            state.phase = Phase::Aborted;
            state.events_closed = true;
            true
        }
        Phase::Aborted => {
            if !state
                .primary
                .as_ref()
                .is_some_and(|primary| primary.same_cause(&error))
                && !state.secondary.iter().any(|prior| prior.same_cause(&error))
            {
                state.secondary.push(error);
            }
            false
        }
        Phase::Publishing | Phase::Finished => false,
    }
}

/// The private producer owns this guard until its operation is terminal.
pub(crate) struct Attempt {
    control: Arc<Control>,
}

impl Attempt {
    #[cfg(feature = "native-geospatial")]
    pub(crate) fn failure_record_bytes() -> usize {
        std::mem::size_of::<ErrorDetail>() + 2 * std::mem::size_of::<usize>()
    }

    #[cfg(feature = "native-geospatial")]
    pub(crate) fn fixed_owned_bytes(&self) -> usize {
        // The caller owns the observer. Empty error/diagnostic storage has no
        // backing allocation at successful producer admission.
        std::mem::size_of::<Self>()
            + std::mem::size_of::<Control>()
            + 2 * std::mem::size_of::<usize>()
    }

    pub(crate) fn check(&self) -> Result<(), JobError> {
        let state = self.control.state();
        match state.phase {
            Phase::Open => Ok(()),
            Phase::Aborted => Err(state.primary.as_ref().unwrap().clone()),
            Phase::Publishing | Phase::Finished => Err(state
                .primary
                .as_ref()
                .cloned()
                .unwrap_or_else(|| invalid_state("operation is no longer producing"))),
        }
    }

    pub(crate) fn emit(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        {
            let mut state = self.control.state();
            if state.phase == Phase::Aborted {
                return Err(state.primary.as_ref().unwrap().clone());
            }
            if state.phase != Phase::Open || state.events_closed {
                let error = invalid_state("event admission is closed");
                record_abort(&mut state, error.clone());
                return Err(error);
            }
            state.active_events += 1;
        }
        // RAII also decrements the active count if a user observer unwinds.
        let admission = EventAdmission {
            control: &self.control,
            active: true,
        };
        let observed = self
            .control
            .observer
            .as_ref()
            .map_or(Ok(()), |observer| observer.observe(event));
        admission.complete(observed)
    }

    pub(crate) fn close_events(&self) -> Result<(), JobError> {
        let mut state = self.control.state();
        state.events_closed = true;
        if state.phase == Phase::Aborted {
            return Err(state.primary.as_ref().unwrap().clone());
        }
        if state.phase != Phase::Open {
            return Err(invalid_state("operation is no longer producing"));
        }
        if state.active_events != 0 {
            let error = invalid_state("producer must finish observer calls before sealing");
            record_abort(&mut state, error.clone());
            return Err(error);
        }
        Ok(())
    }

    fn check_sealable(&self) -> Result<(), JobError> {
        self.check()?;
        let state = self.control.state();
        if !state.events_closed || state.active_events != 0 {
            return Err(invalid_state(
                "producer must close event admission before sealing",
            ));
        }
        Ok(())
    }

    fn permission(&self) -> Result<(), JobError> {
        let mut state = self.control.state();
        if state.phase == Phase::Aborted {
            return Err(state.primary.as_ref().unwrap().clone());
        }
        if state.phase != Phase::Open || !state.events_closed || state.active_events != 0 {
            return Err(invalid_state("operation is not ready for publication"));
        }
        state.phase = Phase::Publishing;
        Ok(())
    }

    pub(crate) fn fail(&self, error: JobError) -> JobFailure {
        let mut state = self.control.state();
        if state.phase == Phase::Publishing {
            // Publication owns the result after permission was granted.
            state.primary = Some(error.clone());
        } else {
            record_abort(&mut state, error.clone());
        }
        let failure = JobFailure {
            error: state.primary.as_ref().cloned().unwrap_or(error),
            secondary: state.secondary.clone(),
            retained_paths: Vec::new(),
            recovery: None,
        };
        state.phase = Phase::Finished;
        state.events_closed = true;
        failure
    }

    fn finish(&self) {
        let mut state = self.control.state();
        state.phase = Phase::Finished;
        state.events_closed = true;
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.finish();
    }
}

struct EventAdmission<'a> {
    control: &'a Control,
    active: bool,
}

impl EventAdmission<'_> {
    fn complete(mut self, observed: Result<(), JobError>) -> Result<(), JobError> {
        let mut state = self.control.state();
        if let Err(error) = observed {
            record_abort(&mut state, JobError::observer(error));
        }
        // Record the observer result and release its admission in one lock.
        state.active_events -= 1;
        self.active = false;
        match state.phase {
            Phase::Open => Ok(()),
            Phase::Aborted => Err(state.primary.as_ref().unwrap().clone()),
            Phase::Publishing | Phase::Finished => Err(state
                .primary
                .as_ref()
                .cloned()
                .unwrap_or_else(|| invalid_state("observer completed outside its attempt"))),
        }
    }
}

impl Drop for EventAdmission<'_> {
    fn drop(&mut self) {
        if self.active {
            let mut state = self.control.state();
            record_abort(
                &mut state,
                JobError::new(
                    JobErrorKind::ObserverFailure,
                    "observer unwound before returning",
                ),
            );
            state.active_events -= 1;
        }
    }
}

/// Pure platform admission, available before a producer reads or stages data.
pub(crate) fn file_publication_supported() -> Result<(), JobError> {
    if cfg!(any(unix, windows)) {
        Ok(())
    } else {
        Err(JobError::new(
            JobErrorKind::Unsupported,
            "file publication is supported only on Unix and Windows",
        ))
    }
}

/// Writable storage accessible only to the private producer via scoped borrows.
pub(crate) struct Staging<'a> {
    file: NamedTempFile,
    attempt: &'a Attempt,
}

impl<'a> Staging<'a> {
    pub(crate) fn create(output: &Path, attempt: &'a Attempt) -> Result<Self, JobFailure> {
        attempt.check().map_err(|error| attempt.fail(error))?;
        file_publication_supported().map_err(|error| attempt.fail(error))?;
        let parent = output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|error| attempt.fail(JobError::io("create output parent", parent, error)))?;
        let file = tempfile::Builder::new()
            .prefix(".tiles-stage-")
            .tempfile_in(parent)
            .map_err(|error| attempt.fail(JobError::io("create staging file", parent, error)))?;
        Ok(Self { file, attempt })
    }

    pub(crate) fn path(&self) -> &Path {
        self.file.path()
    }

    pub(crate) fn writer(&mut self) -> &mut File {
        self.file.as_file_mut()
    }

    pub(crate) fn fail(self, error: JobError) -> JobFailure {
        self.fail_with(error, &SystemFiles)
    }

    fn fail_with(self, error: JobError, files: &dyn FileOperations) -> JobFailure {
        let mut failure = self.attempt.fail(error);
        if let Some(diagnostic) = cleanup_owned(self.file.into_temp_path(), files) {
            failure.retained_paths.push(diagnostic.path);
            failure.secondary.push(diagnostic.error);
        }
        failure
    }

    pub(crate) fn seal(self) -> Result<SealedArtifact<'a>, JobFailure> {
        self.seal_with(&SystemFiles)
    }

    fn seal_with(self, files: &dyn FileOperations) -> Result<SealedArtifact<'a>, JobFailure> {
        let checked = self.attempt.check_sealable().and_then(|()| {
            files
                .sync(self.file.as_file())
                .map_err(|error| JobError::io("synchronize candidate", self.file.path(), error))
        });
        match checked {
            Ok(()) => Ok(SealedArtifact {
                path: self.file.into_temp_path(),
                attempt: self.attempt,
            }),
            Err(error) => Err(self.fail_with(error, files)),
        }
    }
}

/// Closed candidate; no writer API, cloning, or repeat publication exists.
pub(crate) struct SealedArtifact<'a> {
    path: TempPath,
    attempt: &'a Attempt,
}

pub(crate) struct Published {
    pub output: PathBuf,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}

impl SealedArtifact<'_> {
    pub(crate) fn publish(
        self,
        output: &Path,
        policy: OutputPolicy,
    ) -> Result<Published, JobFailure> {
        self.publish_with(output, policy, &SystemFiles)
    }

    fn publish_with(
        self,
        output: &Path,
        policy: OutputPolicy,
        files: &dyn FileOperations,
    ) -> Result<Published, JobFailure> {
        let attempt = self.attempt;
        if let Err(error) = attempt.permission() {
            let mut failure = attempt.fail(error);
            if let Some(diagnostic) = cleanup_owned(self.path, files) {
                failure.retained_paths.push(diagnostic.path);
                failure.secondary.push(diagnostic.error);
            }
            return Err(failure);
        }
        let temporary_name = self.path.to_path_buf();
        match files.install(self.path, output, policy) {
            Ok(()) => {
                // tempfile's no-clobber fallback may install with hard_link and
                // leave the old name. Success does not prove we still own that
                // pathname: after a rename another writer may have occupied it.
                let cleanup_diagnostics = inspect_postcommit_name(&temporary_name, files)
                    .into_iter()
                    .collect();
                attempt.finish();
                Ok(Published {
                    output: output.to_owned(),
                    cleanup_diagnostics,
                })
            }
            Err(error) => {
                let cause = if error.error.kind() == io::ErrorKind::AlreadyExists {
                    JobError::conflict(output, error.error)
                } else {
                    JobError::io("install completed file", output, error.error)
                };
                let mut failure = attempt.fail(cause);
                if let Some(diagnostic) = cleanup_owned(error.path, files) {
                    failure.retained_paths.push(diagnostic.path);
                    failure.secondary.push(diagnostic.error);
                }
                Err(failure)
            }
        }
    }
}

// Narrow filesystem seam for deterministic fault/interleaving tests. Not an
// adapter/plugin API and not carried in RunControl or producer configuration.
trait FileOperations {
    fn sync(&self, file: &File) -> io::Result<()> {
        file.sync_all()
    }
    fn install(
        &self,
        path: TempPath,
        output: &Path,
        policy: OutputPolicy,
    ) -> Result<(), PathPersistError> {
        match policy {
            OutputPolicy::CreateNew => path.persist_noclobber(output),
            OutputPolicy::Replace => path.persist(output),
        }
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
    fn leftover(&self, path: &Path) -> io::Result<bool> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}
struct SystemFiles;
impl FileOperations for SystemFiles {}

fn cleanup_owned(mut path: TempPath, files: &dyn FileOperations) -> Option<CleanupDiagnostic> {
    // An explicit failed cleanup must retain the path rather than hide a
    // second deletion attempt in Drop after returning recovery information.
    path.disable_cleanup(true);
    cleanup_name(&path, files)
}

fn cleanup_name(path: &Path, files: &dyn FileOperations) -> Option<CleanupDiagnostic> {
    match files.remove(path) {
        Ok(()) => None,
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => Some(CleanupDiagnostic {
            path: path.to_owned(),
            error: JobError::io("remove temporary file", path, error),
        }),
    }
}

fn inspect_postcommit_name(path: &Path, files: &dyn FileOperations) -> Option<CleanupDiagnostic> {
    let error = match files.leftover(path) {
        Ok(false) => return None,
        Ok(true) => JobError(Arc::new(ErrorDetail {
            kind: JobErrorKind::Io,
            message: "temporary name remains after installation; retained for inspection because ownership is uncertain".into(),
            path: Some(path.to_owned()),
            source: None,
        })),
        Err(error) => JobError::io("inspect temporary name after installation", path, error),
    };
    // Never unlink a postcommit pathname without an ownership token.
    Some(CleanupDiagnostic {
        path: path.to_owned(),
        error,
    })
}

#[cfg(test)]
mod tests;

#[cfg(any(feature = "native-geospatial", test))]
pub(crate) mod directory;
