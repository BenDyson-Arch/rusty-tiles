//! The mesh run report (`--report-path`, #353): a JSON summary of the run, rewritten as
//! each stage finishes, so a long run can be followed and a stopped run still leaves a
//! valid partial report.
//!
//! Each write replaces the whole file atomically: the document is written to a temporary
//! file beside the report (`.rusty-tiles-report-*.tmp`), synced, then renamed over it, and
//! on Unix the directory is synced too. A reader therefore sees either the previous
//! complete document or the new one, never a partial write. A process killed outright,
//! or a power loss, between creating the temporary file and renaming it can leave that
//! temporary file behind. The replacement keeps an existing report's permission bits; a
//! new report gets the process's usual mode for new files (0666 less the umask on Unix).
//!
//! A rename the platform refuses (on Windows, while another process holds the old report
//! open without delete sharing, as viewers and scanners do) is retried briefly; if it is
//! still refused the report is stale, and every such write says so in a warning. Nothing
//! here reaches the archive. The document's schema is documented in `docs/MESH.md` ("Run
//! report"); `SCHEMA_VERSION` changes when a field changes meaning or is removed.
use crate::{JobError, JobErrorKind, JobFailure};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

/// The report document's `schema`.
pub(crate) const SCHEMA: &str = "rusty_tiles_mesh_run_report";
/// The report document's `schema_version`.
pub(crate) const SCHEMA_VERSION: u32 = 1;
/// Pauses between attempts to rename over a report the platform will not replace yet.
const RENAME_RETRIES: [Duration; 5] = [
    Duration::from_millis(10),
    Duration::from_millis(25),
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
];

/// One finished stage, in completion order.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct StageEntry {
    pub stage: &'static str,
    /// The stage's wall seconds.
    pub wall_seconds: f64,
    /// Whole-process user+system CPU seconds since the previous stage finished (all
    /// workers, and the admission work between stages); null where the platform does
    /// not report process CPU.
    pub cpu_seconds: Option<f64>,
    /// Run elapsed seconds when the stage finished.
    pub run_elapsed_seconds: f64,
    /// Stage facts known when it finished (scratch peaks, counts, worker use).
    pub facts: Value,
}

#[derive(Default)]
struct State {
    /// Set by the first write that left a document (or a stale one) in place; until then
    /// nothing has been written.
    started: bool,
    input: String,
    output: String,
    stages: Vec<StageEntry>,
}

/// Why a write did not replace the report.
#[derive(Debug)]
enum WriteError {
    /// The new document could not be written beside the report: the directory or path is
    /// not writable.
    Unwritable(io::Error),
    /// The new document was written, but the platform refused to rename it over the
    /// report on every attempt: the report on disk still holds the previous document.
    Locked(io::Error),
}

/// The run's report writer. Disabled (every call a no-op) without a path.
pub(crate) struct RunReport {
    path: Option<PathBuf>,
    started: Instant,
    state: Mutex<State>,
}

impl RunReport {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            started: Instant::now(),
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // A panicking writer cannot leave the entries inconsistent: each is pushed whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes the first document (`status: running`, no stages). A report that cannot be
    /// written at all (an unwritable directory or path) fails the run before any source is
    /// read, not hours later. An existing report the platform will not replace yet (held
    /// open elsewhere) does not: the run goes on, and the returned warning says the report
    /// is stale.
    pub(crate) fn start(&self, input: &Path, output: &Path) -> Result<Option<String>, JobError> {
        let Some(path) = &self.path else {
            return Ok(None);
        };
        let mut state = self.state();
        state.input = input.to_string_lossy().into_owned();
        state.output = output.to_string_lossy().into_owned();
        match write(path, &self.document(&state, "running", Map::new())) {
            Ok(()) => {
                state.started = true;
                Ok(None)
            }
            Err(WriteError::Locked(error)) => {
                state.started = true;
                Ok(Some(stale(path, &error, "the next stage tries again")))
            }
            Err(WriteError::Unwritable(error)) => Err(io_error(path, error)),
        }
    }

    /// Adds a finished stage and rewrites the report. Every failed write returns a warning
    /// saying the report is stale (the run goes on; the next stage writes again).
    pub(crate) fn stage(&self, entry: StageEntry) -> Option<String> {
        let path = self.path.as_ref()?;
        let mut state = self.state();
        if !state.started {
            return None;
        }
        state.stages.push(entry);
        write(path, &self.document(&state, "running", Map::new()))
            .err()
            .map(|error| stale(path, error.io(), "the next stage tries again"))
    }

    /// The run's last document before installation: `status: succeeded`, with the
    /// archive report and the full run diagnostics, under the `--json` result's keys
    /// (`tilerReport`, `diagnostics`). A failed write returns a stale-report warning.
    pub(crate) fn succeeded(
        &self,
        report: &impl Serialize,
        diagnostics: &impl Serialize,
    ) -> Option<String> {
        let path = self.path.as_ref()?;
        let state = self.state();
        if !state.started {
            return None;
        }
        let mut extra = Map::new();
        extra.insert("tilerReport".into(), json!(report));
        extra.insert("diagnostics".into(), json!(diagnostics));
        write(path, &self.document(&state, "succeeded", extra))
            .err()
            .map(|error| stale(path, error.io(), "it is not written again"))
    }

    /// A failed run (cancellation included): `status: failed` with the error, keeping
    /// every stage that finished. A write that fails is returned, for the caller to report
    /// beside the run's own error (the report would otherwise still say `running`).
    pub(crate) fn failed(&self, failure: &JobFailure) -> Option<JobError> {
        let path = self.path.as_ref()?;
        let state = self.state();
        if !state.started {
            return None;
        }
        let mut extra = Map::new();
        extra.insert(
            "error".into(),
            json!({"kind": kind(failure.error.kind()), "message": failure.error.to_string()}),
        );
        write(path, &self.document(&state, "failed", extra))
            .err()
            .map(|error| {
                JobError::new(
                    JobErrorKind::Io,
                    stale(path, error.io(), "the run's failure is not recorded in it"),
                )
            })
    }

    fn document(&self, state: &State, status: &str, extra: Map<String, Value>) -> Value {
        let mut document = json!({
            "schema": SCHEMA,
            "schema_version": SCHEMA_VERSION,
            "status": status,
            "input": state.input,
            "output": state.output,
            "elapsed_seconds": self.started.elapsed().as_secs_f64(),
            "stages": state.stages,
        });
        if let Some(object) = document.as_object_mut() {
            object.extend(extra);
        }
        document
    }
}

impl WriteError {
    fn io(&self) -> &io::Error {
        match self {
            Self::Unwritable(error) | Self::Locked(error) => error,
        }
    }
}

fn io_error(path: &Path, error: io::Error) -> JobError {
    JobError::io("write run report", path, error)
}

/// The warning for a write that left `path` holding an earlier document.
fn stale(path: &Path, error: &io::Error, then: &str) -> String {
    format!(
        "the run report {} is stale: it still holds an earlier state of the run because \
         writing its new state failed ({error}); {then}",
        path.display()
    )
}

/// The error categories the CLI and Python adapters report.
fn kind(kind: JobErrorKind) -> &'static str {
    match kind {
        JobErrorKind::InvalidRequest => "invalid_request",
        JobErrorKind::InvalidInput => "invalid_input",
        JobErrorKind::Unsupported => "unsupported",
        JobErrorKind::ResourceLimit => "resource_limit",
        JobErrorKind::Io => "io",
        JobErrorKind::Conflict => "output_conflict",
        JobErrorKind::Cancelled => "cancelled",
        JobErrorKind::ObserverFailure => "observer_failure",
        JobErrorKind::InvalidState => "invalid_state",
    }
}

/// Replaces `path` with `document`: a synced temporary file beside it, renamed over it
/// (retried while the platform refuses), then the directory synced where that is cheap.
fn write(path: &Path, document: &Value) -> Result<(), WriteError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut builder = tempfile::Builder::new();
    builder.prefix(".rusty-tiles-report-").suffix(".tmp");
    // A new report gets the usual mode for new files (open's 0666 less the umask), not the
    // temporary file's owner-only default.
    #[cfg(unix)]
    let new_file = <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o666);
    #[cfg(unix)]
    builder.permissions(new_file);
    let mut file = builder
        .tempfile_in(parent)
        .map_err(WriteError::Unwritable)?;
    let mut bytes = serde_json::to_vec_pretty(document)
        .map_err(|e| WriteError::Unwritable(io::Error::other(e)))?;
    bytes.push(b'\n');
    file.write_all(&bytes).map_err(WriteError::Unwritable)?;
    // A replaced report keeps its permission bits (a report others could read stays so).
    #[cfg(unix)]
    if let Ok(existing) = std::fs::metadata(path) {
        if existing.is_file() {
            file.as_file()
                .set_permissions(existing.permissions())
                .map_err(WriteError::Unwritable)?;
        }
    }
    file.as_file().sync_all().map_err(WriteError::Unwritable)?;
    // The temporary file is removed by its own guard if every rename fails.
    let mut pending = file;
    let mut retries = RENAME_RETRIES.iter();
    loop {
        match pending.persist(path) {
            Ok(_) => break,
            Err(refused) => match retries.next() {
                Some(pause) => {
                    std::thread::sleep(*pause);
                    pending = refused.file;
                }
                None => return Err(WriteError::Locked(refused.error)),
            },
        }
    }
    // The rename is durable once its directory is synced. Unix only: Windows cannot open
    // a directory as a file. A filesystem that refuses to sync a directory still has the
    // new document in place, so that failure is not reported as a stale report.
    #[cfg(unix)]
    if let Ok(directory) = std::fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn entry(stage: &'static str) -> StageEntry {
        StageEntry {
            stage,
            wall_seconds: 1.5,
            cpu_seconds: Some(2.),
            run_elapsed_seconds: 3.,
            facts: json!({"scratch_peak_bytes": 7}),
        }
    }

    fn cancelled() -> JobFailure {
        JobFailure {
            error: JobError::new(JobErrorKind::Cancelled, "operation cancelled"),
            secondary: Vec::new(),
            retained_paths: Vec::new(),
            recovery: None,
        }
    }

    /// Every write leaves one complete document: running with each finished stage, then
    /// the final status; no temporary file is left beside it.
    #[test]
    fn each_stage_rewrites_a_complete_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.json");
        let report = RunReport::new(Some(path.clone()));
        // Nothing is written before the run starts.
        assert!(report.stage(entry("capture")).is_none());
        assert!(!path.exists());
        let start = report.start(Path::new("in.glb"), Path::new("out.3tz"));
        assert!(start.unwrap().is_none());
        let first = read(&path);
        assert_eq!(first["schema"], SCHEMA);
        assert_eq!(first["schema_version"], SCHEMA_VERSION);
        assert_eq!(first["status"], "running");
        assert_eq!(first["stages"], json!([]));
        assert!(report.stage(entry("capture")).is_none());
        assert!(report.stage(entry("leaf_build")).is_none());
        let partial = read(&path);
        assert_eq!(partial["status"], "running");
        assert_eq!(partial["stages"][1]["stage"], "leaf_build");
        assert_eq!(partial["stages"][0]["facts"]["scratch_peak_bytes"], 7);
        assert!(report.failed(&cancelled()).is_none());
        let failed = read(&path);
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["error"]["kind"], "cancelled");
        assert_eq!(failed["stages"].as_array().unwrap().len(), 2);
        assert!(report.succeeded(&json!({"tiles": 3}), &json!({})).is_none());
        let done = read(&path);
        assert_eq!(done["status"], "succeeded");
        assert_eq!(done["tilerReport"]["tiles"], 3);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["run.json"]);
    }

    /// An unwritable directory refuses the start. A report that cannot be replaced (here a
    /// directory stands where it is renamed to; on Windows, a file held open elsewhere)
    /// does not: the start and every later write warn that the report is stale, and the
    /// final failed write is returned.
    #[test]
    fn unwritable_paths_refuse_the_start_and_unreplaceable_reports_warn_every_write() {
        let dir = tempfile::tempdir().unwrap();
        let missing = RunReport::new(Some(dir.path().join("missing").join("run.json")));
        let refused = missing.start(Path::new("in.glb"), Path::new("out.3tz"));
        assert_eq!(refused.unwrap_err().kind(), JobErrorKind::Io);
        let path = dir.path().join("run.json");
        std::fs::create_dir(&path).unwrap();
        let report = RunReport::new(Some(path.clone()));
        let warning = report
            .start(Path::new("in.glb"), Path::new("out.3tz"))
            .unwrap()
            .unwrap();
        assert!(warning.contains("is stale"), "{warning}");
        for stage in ["capture", "leaf_build"] {
            let warning = report.stage(entry(stage)).unwrap();
            assert!(warning.contains("is stale"), "{warning}");
        }
        let failed = report.failed(&cancelled()).unwrap();
        assert_eq!(failed.kind(), JobErrorKind::Io);
        assert!(failed.to_string().contains("is stale"), "{failed}");
        // Every refused rename removed its temporary file.
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["run.json"]);
        // Disabled reports write nothing and never warn.
        let disabled = RunReport::new(None);
        assert!(disabled
            .start(Path::new("a"), Path::new("b"))
            .unwrap()
            .is_none());
        assert!(disabled.stage(entry("capture")).is_none());
        assert!(disabled.failed(&cancelled()).is_none());
    }

    /// A replaced report keeps its mode; a new one gets the mode a plain new file gets.
    #[cfg(unix)]
    #[test]
    fn replacing_keeps_the_mode_and_new_reports_follow_the_umask() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o7777;
        let probe = dir.path().join("probe");
        std::fs::File::create(&probe).unwrap();
        let fresh = dir.path().join("fresh.json");
        let report = RunReport::new(Some(fresh.clone()));
        report.start(Path::new("a"), Path::new("b")).unwrap();
        assert_eq!(mode(&fresh), mode(&probe));
        for kept in [0o644, 0o640, 0o600] {
            let path = dir.path().join(format!("kept-{kept:o}.json"));
            std::fs::write(&path, b"{}").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(kept)).unwrap();
            let report = RunReport::new(Some(path.clone()));
            report.start(Path::new("a"), Path::new("b")).unwrap();
            assert!(report.stage(entry("capture")).is_none());
            assert_eq!(mode(&path), kept);
            assert_eq!(read(&path)["stages"][0]["stage"], "capture");
        }
    }
}
