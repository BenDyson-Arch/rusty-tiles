//! Vector layers → draft glTF vector content (`3DTILES_content_gltf_vector`).
//!
//! The prototype pins a tested draft; the encoding spec is still drafting ([3d-tiles#825](https://github.com/CesiumGS/3d-tiles/issues/825),
//! [PR #838](https://github.com/CesiumGS/3d-tiles/pull/838)). Do not invent a private format.

use std::{
    fs,
    path::{Path, PathBuf},
};

mod model;
mod pipeline;
mod source_fields;

#[cfg(feature = "native-geospatial")]
pub(crate) fn native_available() -> Result<(), Error> {
    pipeline::available()
}

use crate::{
    archive3tz::{self, CodecError, WriteFailure},
    error::Error,
    runtime::{Attempt, Staging},
    CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl, RunEvent,
};

pub const SPEC_ISSUE: &str = "https://github.com/CesiumGS/3d-tiles/issues/825";
pub const SPEC_PR: &str = "https://github.com/CesiumGS/3d-tiles/pull/838";
pub const SPEC_BLOG: &str =
    "https://cesium.com/blog/2026/06/29/help-shape-vector-data-support-in-3d-tiles/";

/// Draft extension identifiers; see README for the tested runtime and schema revision.
pub const TILESET_EXTENSION: &str = "3DTILES_content_gltf_vector";
pub const GLTF_RESTART: &str = "KHR_mesh_primitive_restart";
pub const GLTF_POLYGON: &str = "EXT_mesh_polygon";
pub const GLTF_FEATURES: &str = "EXT_mesh_features";
pub const GLTF_METADATA: &str = "EXT_structural_metadata";

pub const INPUT_ORDER: &[&str] = &["GeoJSON", "GPKG layers", "shapefile"];

#[derive(Clone, Debug)]
pub struct VectorLodOptions {
    pub tolerance_metres: f64,
    pub levels: u8,
}

impl Default for VectorLodOptions {
    fn default() -> Self {
        Self {
            tolerance_metres: 0.1,
            levels: 3,
        }
    }
}

/// Vector input selection and hard content budgets. Heights are explicit for 3D
/// horizontal-CRS sources; `local` uses metre XYZ without geospatial placement.
#[derive(Clone, Debug)]
pub struct VectorOptions {
    /// Emit an explicit hierarchy.
    pub explicit: bool,
    pub reproducible: bool,
    pub jobs: usize,
    pub quantize: bool,
    pub meshopt: bool,
    pub parent_repair: bool,
    /// Explicit count aggregates in point-only parents; leaves retain source metadata.
    pub aggregate_points: bool,
    pub max_parent_features: usize,
    pub where_clause: Option<String>,
    pub max_features: usize,
    pub repair: bool,
    pub ambiguous_outlines: bool,
    pub list_fields: String,
    pub fields: Vec<String>,
    pub drop_fields: Vec<String>,
    pub skip_invalid: bool,
    pub lod: VectorLodOptions,
    pub reuse_tileset: Option<std::path::PathBuf>,
    pub layers: Vec<String>,
    pub all_layers: bool,
    pub source_crs: Option<String>,
    pub height_offset: Option<f64>,
    pub max_vertices: usize,
    pub max_bytes: usize,
    pub max_tiles: usize,
    pub max_source_vertices: usize,
}

impl Default for VectorOptions {
    fn default() -> Self {
        Self {
            explicit: false,
            reproducible: false,
            jobs: std::thread::available_parallelism().map_or(1, usize::from),
            quantize: false,
            meshopt: false,
            parent_repair: false,
            aggregate_points: false,
            max_parent_features: 4096,
            where_clause: None,
            max_features: 64,
            repair: false,
            ambiguous_outlines: false,
            list_fields: "error".into(),
            fields: Vec::new(),
            drop_fields: Vec::new(),
            skip_invalid: false,
            lod: VectorLodOptions::default(),
            reuse_tileset: None,
            layers: Vec::new(),
            all_layers: false,
            source_crs: None,
            height_offset: None,
            max_vertices: 65536,
            max_bytes: 4194304,
            max_tiles: 100000,
            max_source_vertices: 1000000,
        }
    }
}

/// Vector source and completed archive destination, with publication policy.
#[derive(Clone, Debug)]
pub struct VectorRequest {
    input: PathBuf,
    output: PathBuf,
    options: VectorOptions,
    policy: OutputPolicy,
}
impl VectorRequest {
    pub fn new(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        options: VectorOptions,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            options,
            policy: OutputPolicy::CreateNew,
        }
    }
    pub fn with_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
}
#[derive(Debug)]
pub struct VectorResult {
    pub output: PathBuf,
    pub report: serde_json::Value,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
fn validate(options: &VectorOptions) -> Result<(), Error> {
    let max_features = options.max_features;
    let lod = &options.lod;
    if options
        .where_clause
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(Error::msg(
            "--where must not be empty; omit it to convert every feature",
        ));
    }
    if !matches!(options.list_fields.as_str(), "error" | "json") {
        return Err(Error::msg(format!(
            "--listFields must be error or json, got {:?}",
            options.list_fields
        )));
    }
    if !options.fields.is_empty() && !options.drop_fields.is_empty() {
        return Err(Error::msg(
            "--fields and --dropFields cannot be combined; choose one field selection",
        ));
    }
    if options.all_layers && !options.layers.is_empty() {
        return Err(Error::msg(
            "--allLayers cannot be combined with --layer; choose one layer selection",
        ));
    }
    if options
        .height_offset
        .is_some_and(|value| !value.is_finite())
    {
        return Err(Error::msg(
            "--heightOffset must be a finite number of metres",
        ));
    }
    for (flag, value, minimum) in [
        ("--jobs", options.jobs, 1),
        ("--maxFeatures", max_features, 1),
        ("--maxParentFeatures", options.max_parent_features, 1),
        ("--maxVertices", options.max_vertices, 4),
        ("--maxBytes", options.max_bytes, 4096),
        ("--maxTiles", options.max_tiles, 1),
        ("--maxSourceVertices", options.max_source_vertices, 1),
    ] {
        if value < minimum {
            return Err(Error::msg(format!(
                "{flag} must be at least {minimum}, got {value}"
            )));
        }
    }
    if !lod.tolerance_metres.is_finite() || lod.tolerance_metres <= 0.0 {
        return Err(Error::msg(format!(
            "--lodTolerance must be a positive finite number of metres, got {}",
            lod.tolerance_metres
        )));
    }
    if !(1..=16).contains(&lod.levels) {
        return Err(Error::msg(format!(
            "--lodLevels must be between 1 and 16, got {}",
            lod.levels
        )));
    }
    Ok(())
}

fn job_error(error: Error) -> JobError {
    match error {
        Error::Job(failure) => failure.error,
        Error::Io(error) => JobError::io("vector producer", Path::new("."), error),
        Error::Json(error) if error.is_io() => {
            let kind = error.io_error_kind().unwrap_or(std::io::ErrorKind::Other);
            JobError::io(
                "vector JSON transport",
                Path::new("."),
                std::io::Error::new(kind, error),
            )
        }
        Error::Environment(message) => JobError::new(JobErrorKind::Unsupported, message),
        Error::OutputExists(path) => JobError::new(
            JobErrorKind::Conflict,
            format!("output exists: {}", path.display()),
        ),
        Error::Message(message) => JobError::new(JobErrorKind::InvalidRequest, message),
        Error::NotImplemented { feature, hint } => {
            JobError::new(JobErrorKind::Unsupported, format!("{feature}: {hint}"))
        }
        other => JobError::new(JobErrorKind::InvalidInput, other.to_string()),
    }
}
fn prepare(mut request: VectorRequest) -> Result<VectorRequest, JobError> {
    validate(&request.options).map_err(job_error)?;
    if request.output.extension().is_none_or(|e| e != "3tz")
        && !request
            .output
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".3dtiles.zip"))
    {
        return Err(JobError::new(
            JobErrorKind::InvalidRequest,
            "vector output must use .3tz or .3dtiles.zip",
        ));
    }
    let output = crate::output_path::resolve(&request.output)?;
    let existing = match fs::symlink_metadata(&output) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(JobError::io("inspect vector output", &output, error)),
    };
    if existing.as_ref().is_some_and(|m| !m.is_file()) {
        return Err(JobError::new(
            JobErrorKind::InvalidRequest,
            "vector output must be a regular file without a symlink leaf",
        ));
    }
    fn source(path: &Path, output: &Path, exists: bool) -> Result<PathBuf, JobError> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|e| JobError::io("inspect vector source", path, e))?;
        if !metadata.is_file() {
            return Err(JobError::new(
                JobErrorKind::InvalidInput,
                "vector source must be a regular file without a symlink leaf",
            ));
        }
        let resolved =
            fs::canonicalize(path).map_err(|e| JobError::io("resolve vector source", path, e))?;
        if resolved == output
            || (exists
                && same_file::is_same_file(&resolved, output).map_err(|e| {
                    JobError::io("compare vector source/output identity", output, e)
                })?)
        {
            return Err(JobError::new(
                JobErrorKind::InvalidRequest,
                "vector output aliases source or reused archive",
            ));
        }
        Ok(resolved)
    }
    request.input = source(&request.input, &output, existing.is_some())?;
    if let Some(previous) = &request.options.reuse_tileset {
        request.options.reuse_tileset = Some(source(previous, &output, existing.is_some())?);
    }
    if existing.is_some() && request.policy == OutputPolicy::CreateNew {
        return Err(JobError::new(
            JobErrorKind::Conflict,
            "vector output already exists",
        ));
    }
    request.output = output;
    Ok(request)
}
struct Workspace(tempfile::TempDir);
impl Workspace {
    fn create(output: &Path) -> Result<Self, JobError> {
        let parent = output.parent().expect("resolved output parent");
        fs::create_dir_all(parent)
            .map_err(|e| JobError::io("create vector output parent", parent, e))?;
        tempfile::Builder::new()
            .prefix(".vector-work-")
            .tempdir_in(parent)
            .map(Self)
            .map_err(|e| JobError::io("create vector workspace", parent, e))
    }
    fn cleanup(self) -> Result<(), CleanupDiagnostic> {
        let path = self.0.keep();
        fs::remove_dir_all(&path).map_err(|error| CleanupDiagnostic {
            error: JobError::io("remove vector workspace", &path, error),
            path,
        })
    }
}
fn normalize_failure(attempt: &Attempt, failure: JobFailure) -> JobFailure {
    let mut normalized = attempt.fail(failure.error.clone());
    // A reader may construct a failure without entering the run gate. Retain
    // its causes after arbitration, including when cancellation was first.
    for cause in std::iter::once(failure.error).chain(failure.secondary) {
        if !normalized.error.same_cause(&cause)
            && !normalized
                .secondary
                .iter()
                .any(|prior| prior.same_cause(&cause))
        {
            normalized.secondary.push(cause);
        }
    }
    normalized.retained_paths = failure.retained_paths;
    normalized.recovery = failure.recovery;
    normalized
}

fn cleanup_failure(mut failure: JobFailure, workspace: Workspace) -> JobFailure {
    if let Err(diagnostic) = workspace.cleanup() {
        failure.retained_paths.push(diagnostic.path);
        failure.secondary.push(diagnostic.error);
    }
    failure
}
fn codec_error(error: CodecError, output: &Path) -> JobError {
    match error {
        CodecError::Io(e) => JobError::io("serialize vector archive", output, e),
        CodecError::SourceIo { path, source } => {
            JobError::io("read accepted vector member", &path, source)
        }
        CodecError::Zip(e) => {
            JobError::io("serialize vector archive", output, std::io::Error::other(e))
        }
        other => JobError::new(
            JobErrorKind::InvalidState,
            format!("encode accepted vector inventory: {other}"),
        ),
    }
}
/// Execute one vector producer and archive publication under the F0 lifecycle.
pub fn vector_to_archive(
    request: VectorRequest,
    control: &RunControl,
) -> Result<VectorResult, JobFailure> {
    let attempt = control.begin()?;
    let request = prepare(request).map_err(|e| attempt.fail(e))?;
    attempt.check().map_err(|e| attempt.fail(e))?;
    let workspace = Workspace::create(&request.output).map_err(|e| attempt.fail(e))?;
    let completed = match pipeline::convert(
        &request.input,
        workspace.0.path(),
        &request.options,
        &attempt,
    ) {
        Ok(value) => value,
        Err(Error::Job(failure)) => {
            return Err(cleanup_failure(
                normalize_failure(&attempt, failure),
                workspace,
            ));
        }
        Err(error) => return Err(cleanup_failure(attempt.fail(job_error(error)), workspace)),
    };
    let selected = match completed
        .members
        .iter()
        .map(|member| {
            let metadata = fs::symlink_metadata(&member.path)
                .map_err(|e| JobError::io("inspect accepted vector member", &member.path, e))?;
            if !metadata.is_file() || !member.path.starts_with(workspace.0.path()) {
                return Err(JobError::new(
                    JobErrorKind::InvalidState,
                    "accepted vector member must be a regular file in its workspace",
                ));
            }
            Ok(archive3tz::Member {
                name: &member.name,
                source: &member.path,
                size: metadata.len(),
                modified: metadata.modified().ok(),
            })
        })
        .collect::<Result<Vec<_>, JobError>>()
    {
        Ok(value) => value,
        Err(error) => return Err(cleanup_failure(attempt.fail(error), workspace)),
    };
    let mut staging = match Staging::create(&request.output, &attempt) {
        Ok(value) => value,
        Err(failure) => return Err(cleanup_failure(failure, workspace)),
    };
    let total = selected.iter().map(|m| m.size).sum();
    if let Err(error) = archive3tz::serialize(&selected, staging.writer(), &mut |done| {
        attempt.check()?;
        attempt.emit(&RunEvent::Progress {
            phase: "vector_archive",
            done,
            total: Some(total),
        })
    }) {
        let error = match error {
            WriteFailure::Checkpoint(error) => error,
            WriteFailure::Codec(error) => codec_error(error, &request.output),
        };
        return Err(cleanup_failure(staging.fail(error), workspace));
    }
    if let Err(diagnostic) = workspace.cleanup() {
        let mut failure = staging.fail(diagnostic.error);
        failure.retained_paths.push(diagnostic.path);
        return Err(failure);
    }
    if let Err(error) = attempt
        .emit(&RunEvent::Progress {
            phase: "ready_to_publish",
            done: total,
            total: Some(total),
        })
        .and_then(|_| attempt.close_events())
    {
        return Err(staging.fail(error));
    }
    let published = staging.seal()?.publish(&request.output, request.policy)?;
    Ok(VectorResult {
        output: published.output,
        report: completed.report,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}

#[cfg(test)]
mod foundation_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    struct Count(AtomicUsize);
    impl crate::Observer for Count {
        fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    #[test]
    fn aliases_are_rejected_before_observation() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("input.3tz");
        let destination = work.path().join("alias.3tz");
        fs::write(&source, b"source bytes").unwrap();
        fs::hard_link(&source, &destination).unwrap();
        let observer = Arc::new(Count(AtomicUsize::new(0)));
        let run = RunControl::new(Some(observer.clone()));
        let request = VectorRequest::new(&source, &destination, VectorOptions::default())
            .with_policy(OutputPolicy::Replace);
        let failure = vector_to_archive(request, &run).unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
        assert_eq!(observer.0.load(Ordering::SeqCst), 0);
        assert_eq!(fs::read(&source).unwrap(), b"source bytes");
    }
    #[test]
    fn reuse_output_alias_is_rejected_before_observation() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("input.geojson");
        let previous = work.path().join("previous.3tz");
        fs::write(&source, b"{}").unwrap();
        fs::write(&previous, b"previous bytes").unwrap();
        let observer = Arc::new(Count(AtomicUsize::new(0)));
        let run = RunControl::new(Some(observer.clone()));
        let options = VectorOptions {
            reuse_tileset: Some(previous.clone()),
            ..VectorOptions::default()
        };
        let failure = vector_to_archive(
            VectorRequest::new(&source, &previous, options).with_policy(OutputPolicy::Replace),
            &run,
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
        assert_eq!(observer.0.load(Ordering::SeqCst), 0);
        assert_eq!(fs::read(&previous).unwrap(), b"previous bytes");
    }
    #[test]
    fn json_transport_fault_preserves_io_cause() {
        struct Fault;
        impl std::io::Read for Fault {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::StorageFull,
                    "JSON transport fault",
                ))
            }
        }
        let json_error = serde_json::from_reader::<_, serde_json::Value>(Fault).unwrap_err();
        let error = job_error(Error::Json(json_error));
        assert_eq!(error.kind(), JobErrorKind::Io);
        let cause = std::error::Error::source(&error)
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(cause.kind(), std::io::ErrorKind::StorageFull);
        assert!(cause.to_string().contains("JSON transport fault"));
        let syntax = serde_json::from_slice::<serde_json::Value>(b"{").unwrap_err();
        assert_eq!(
            job_error(Error::Json(syntax)).kind(),
            JobErrorKind::InvalidInput
        );
    }
    #[test]
    fn cancellation_at_publication_boundary_aborts_skip_invalid() {
        struct Cancel(std::sync::Mutex<Option<crate::CancellationHandle>>);
        impl crate::Observer for Cancel {
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
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("source.geojson");
        let output = work.path().join("output.3tz");
        fs::write(&source,br#"{"type":"FeatureCollection","features":[{"type":"Feature","id":"accepted","properties":{},"geometry":{"type":"Point","coordinates":[1,2,3]}}]}"#).unwrap();
        fs::write(&output, b"existing archive").unwrap();
        let observer = Arc::new(Cancel(std::sync::Mutex::new(None)));
        let run = RunControl::new(Some(observer.clone()));
        *observer.0.lock().unwrap() = Some(run.cancellation_handle());
        let options = VectorOptions {
            source_crs: Some("local".into()),
            skip_invalid: true,
            lod: VectorLodOptions {
                levels: 1,
                ..VectorLodOptions::default()
            },
            ..VectorOptions::default()
        };
        let result = vector_to_archive(
            VectorRequest::new(&source, &output, options).with_policy(OutputPolicy::Replace),
            &run,
        );
        assert_eq!(result.unwrap_err().error.kind(), JobErrorKind::Cancelled);
        assert_eq!(fs::read(&output).unwrap(), b"existing archive");
        let entries = fs::read_dir(work.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(
            entries.len(),
            2,
            "all producer and staging scratch must be removed"
        );
    }
    #[test]
    fn cancellation_wins_over_reader_failure_without_duplicating_causes() {
        let run = RunControl::default();
        let attempt = run.begin().unwrap();
        assert!(run.cancellation_handle().cancel());
        let read = JobError::io(
            "read source",
            Path::new("source.geojson"),
            std::io::Error::new(std::io::ErrorKind::StorageFull, "read fault"),
        );
        let close = JobError::io(
            "close source",
            Path::new("source.geojson"),
            std::io::Error::other("close fault"),
        );
        let retained = PathBuf::from("retained-source-spool");
        let normalized = normalize_failure(
            &attempt,
            JobFailure {
                error: read.clone(),
                secondary: vec![read.clone(), close.clone(), close.clone()],
                retained_paths: vec![retained.clone()],
                recovery: None,
            },
        );
        assert_eq!(normalized.error.kind(), JobErrorKind::Cancelled);
        assert_eq!(normalized.secondary.len(), 2);
        assert!(normalized.secondary[0].same_cause(&read));
        assert!(normalized.secondary[1].same_cause(&close));
        assert_eq!(normalized.retained_paths, vec![retained]);
        let again = normalize_failure(&attempt, normalized);
        assert_eq!(again.error.kind(), JobErrorKind::Cancelled);
        assert_eq!(again.secondary.len(), 2);
    }
    #[test]
    fn observer_io_faults_abort_every_real_vector_stage_under_skip_invalid() {
        struct Fault {
            stage: &'static str,
            cause: JobError,
            calls: AtomicUsize,
        }
        impl crate::Observer for Fault {
            fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
                let selected = match event {
                    RunEvent::Progress { phase, .. } => *phase == self.stage,
                    RunEvent::Warning { code, .. } => *code == self.stage,
                    _ => false,
                };
                if selected {
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    Err(self.cause.clone())
                } else {
                    Ok(())
                }
            }
        }
        for stage in [
            "ingestion",
            "encoding",
            "vector_archive",
            "ready_to_publish",
            "vector-feature-rejected",
        ] {
            let work = tempfile::tempdir().unwrap();
            let source = work.path().join("source.geojson");
            let output = work.path().join("output.3tz");
            let features = if stage == "vector-feature-rejected" {
                serde_json::json!([
                    {"type":"Feature","id":"rejected","properties":{"invalid":{}},"geometry":{"type":"Point","coordinates":[100,200,300]}},
                    {"type":"Feature","id":"accepted","properties":{},"geometry":{"type":"Point","coordinates":[1,2,3]}}
                ])
            } else {
                serde_json::json!([{ "type":"Feature","id":"accepted","properties":{},"geometry":{"type":"Point","coordinates":[1,2,3]} }])
            };
            let source_bytes = serde_json::to_vec(
                &serde_json::json!({"type":"FeatureCollection","features":features}),
            )
            .unwrap();
            fs::write(&source, &source_bytes).unwrap();
            let previous = b"existing archive with deliberately non-ZIP bytes";
            fs::write(&output, previous).unwrap();
            let cause = JobError::io(
                "injected vector observer",
                &source,
                std::io::Error::new(std::io::ErrorKind::StorageFull, format!("{stage} fault")),
            );
            let observer = Arc::new(Fault {
                stage,
                cause: cause.clone(),
                calls: AtomicUsize::new(0),
            });
            let run = RunControl::new(Some(observer.clone()));
            let options = VectorOptions {
                source_crs: Some("local".into()),
                skip_invalid: true,
                lod: VectorLodOptions {
                    levels: 1,
                    ..VectorLodOptions::default()
                },
                ..VectorOptions::default()
            };
            let failure = vector_to_archive(
                VectorRequest::new(&source, &output, options).with_policy(OutputPolicy::Replace),
                &run,
            )
            .unwrap_err();
            assert_eq!(
                failure.error.kind(),
                JobErrorKind::ObserverFailure,
                "{stage}"
            );
            let observed = std::error::Error::source(&failure.error)
                .unwrap()
                .downcast_ref::<JobError>()
                .unwrap();
            assert!(
                observed.same_cause(&cause),
                "{stage}: original cause identity must survive"
            );
            let io = std::error::Error::source(observed)
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap();
            assert_eq!(io.kind(), std::io::ErrorKind::StorageFull, "{stage}");
            assert_eq!(observer.calls.load(Ordering::SeqCst), 1, "{stage}");
            assert_eq!(fs::read(&output).unwrap(), previous, "{stage}");
            assert_eq!(fs::read(&source).unwrap(), source_bytes, "{stage}");
            assert!(failure.retained_paths.is_empty(), "{stage}");
            assert_eq!(
                fs::read_dir(work.path()).unwrap().count(),
                2,
                "{stage}: producer/staging scratch must be removed"
            );
        }
    }
}
