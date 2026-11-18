//! The mesh run report (`--report-path`, #353): a JSON summary of the run, rewritten as
//! each stage finishes, so a long run can be followed and a stopped run still leaves a
//! valid partial report.
//!
//! Each write replaces the whole file atomically: the document is written to a temporary
//! file beside the report, synced, then renamed over it. A reader therefore sees either
//! the previous complete document or the new one, never a partial write. Nothing here
//! reaches the archive. The document's schema is documented in `docs/MESH.md` ("Run
//! report"); `SCHEMA_VERSION` changes when a field changes meaning or is removed.
use crate::{JobError, JobErrorKind, JobFailure};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
    time::Instant,
};

/// The report document's `schema`.
pub(crate) const SCHEMA: &str = "rusty_tiles_mesh_run_report";
/// The report document's `schema_version`.
pub(crate) const SCHEMA_VERSION: u32 = 1;

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
    /// Set by the first successful write; until then nothing has been written.
    started: bool,
    input: String,
    output: String,
    stages: Vec<StageEntry>,
    /// A write failed and was reported; later failures are not reported again.
    warned: bool,
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

    /// Writes the first document (`status: running`, no stages). Its failure fails the
    /// run before any source is read: a report that was asked for and cannot be written
    /// is refused up front, not discovered hours later.
    pub(crate) fn start(&self, input: &Path, output: &Path) -> Result<(), JobError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut state = self.state();
        state.input = input.to_string_lossy().into_owned();
        state.output = output.to_string_lossy().into_owned();
        write(path, &self.document(&state, "running", Map::new()))?;
        state.started = true;
        Ok(())
    }

    /// Adds a finished stage and rewrites the report. A failed write is returned once as
    /// a warning message (the run goes on; the next stage writes again) and then `None`.
    pub(crate) fn stage(&self, entry: StageEntry) -> Option<String> {
        let path = self.path.as_ref()?;
        let mut state = self.state();
        if !state.started {
            return None;
        }
        state.stages.push(entry);
        let written = write(path, &self.document(&state, "running", Map::new()));
        Self::warning(&mut state, written)
    }

    /// The run's last document before installation: `status: succeeded`, with the
    /// archive report and the full run diagnostics (the `--json` envelope's fields).
    pub(crate) fn succeeded(
        &self,
        report: &impl Serialize,
        diagnostics: &impl Serialize,
    ) -> Option<String> {
        let path = self.path.as_ref()?;
        let mut state = self.state();
        if !state.started {
            return None;
        }
        let mut extra = Map::new();
        extra.insert("tiler_report".into(), json!(report));
        extra.insert("diagnostics".into(), json!(diagnostics));
        let written = write(path, &self.document(&state, "succeeded", extra));
        Self::warning(&mut state, written)
    }

    /// A failed run (cancellation included): `status: failed` with the error, keeping
    /// every stage that finished. Best effort: the run's own error is what it returns.
    pub(crate) fn failed(&self, failure: &JobFailure) {
        let Some(path) = &self.path else {
            return;
        };
        let state = self.state();
        if !state.started {
            return;
        }
        let mut extra = Map::new();
        extra.insert(
            "error".into(),
            json!({"kind": kind(failure.error.kind()), "message": failure.error.to_string()}),
        );
        let _ = write(path, &self.document(&state, "failed", extra));
    }

    fn warning(state: &mut State, written: Result<(), JobError>) -> Option<String> {
        match written {
            Err(error) if !state.warned => {
                state.warned = true;
                Some(format!("the run report could not be written: {error}"))
            }
            _ => None,
        }
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

/// Replaces `path` with `document`: a synced temporary file beside it, renamed over it.
fn write(path: &Path, document: &Value) -> Result<(), JobError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let io = |error| JobError::io("write run report", path, error);
    let mut file = tempfile::Builder::new()
        .prefix(".rusty-tiles-report-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(io)?;
    let mut bytes = serde_json::to_vec_pretty(document)
        .map_err(|e| JobError::new(JobErrorKind::InvalidState, e.to_string()))?;
    bytes.push(b'\n');
    file.write_all(&bytes).map_err(io)?;
    file.as_file().sync_all().map_err(io)?;
    // The temporary file is removed by its own guard if the rename fails.
    file.persist(path).map_err(|e| io(e.error))?;
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
        report
            .start(Path::new("in.glb"), Path::new("out.3tz"))
            .unwrap();
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
        report.failed(&JobFailure {
            error: JobError::new(JobErrorKind::Cancelled, "operation cancelled"),
            secondary: Vec::new(),
            retained_paths: Vec::new(),
            recovery: None,
        });
        let failed = read(&path);
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["error"]["kind"], "cancelled");
        assert_eq!(failed["stages"].as_array().unwrap().len(), 2);
        assert!(report.succeeded(&json!({"tiles": 3}), &json!({})).is_none());
        let done = read(&path);
        assert_eq!(done["status"], "succeeded");
        assert_eq!(done["tiler_report"]["tiles"], 3);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["run.json"]);
    }

    /// A start that cannot write fails; a later failed write is a single warning.
    #[test]
    fn write_failures_refuse_the_start_and_warn_once_later() {
        let dir = tempfile::tempdir().unwrap();
        let missing = RunReport::new(Some(dir.path().join("missing").join("run.json")));
        assert_eq!(
            missing
                .start(Path::new("in.glb"), Path::new("out.3tz"))
                .unwrap_err()
                .kind(),
            JobErrorKind::Io
        );
        let path = dir.path().join("run.json");
        let report = RunReport::new(Some(path.clone()));
        report
            .start(Path::new("in.glb"), Path::new("out.3tz"))
            .unwrap();
        // A directory now stands where the report is renamed to.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(report.stage(entry("capture")).is_some());
        assert!(report.stage(entry("leaf_build")).is_none());
        // Disabled reports write nothing and never warn.
        let disabled = RunReport::new(None);
        disabled.start(Path::new("a"), Path::new("b")).unwrap();
        assert!(disabled.stage(entry("capture")).is_none());
    }
}
