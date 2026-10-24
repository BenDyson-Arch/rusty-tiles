//! Path-based bindings for the portable default build.
use pyo3::{
    create_exception,
    exceptions::{PyBaseException, PyException, PyTypeError},
    prelude::*,
    types::{PyBool, PyDict, PyInt, PyList},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tiles_core::{
    package::{package, PackageRequest},
    point_cloud::{PointCloudCoordinates, PointCloudCrs, PointCloudOptions, PointCloudRequest},
    vector::{VectorLodOptions, VectorOptions},
    Error, JobError, JobErrorKind, JobFailure, MeshPlacement, MeshSourceCrs, ModelManifestRequest,
    ModelWrapRequest, Observer, OutputPolicy, RunControl, RunEvent, SourceAxes, TilerJobs,
    TilerLod, TilerProfile, TilerRequest,
};

create_exception!(rusty_tiles, TilesError, PyException);
create_exception!(rusty_tiles, DataError, TilesError);
create_exception!(rusty_tiles, ResourceLimitError, TilesError);
create_exception!(rusty_tiles, EnvironmentError, TilesError);
create_exception!(rusty_tiles, OutputExistsError, TilesError);
create_exception!(rusty_tiles, TilesIOError, TilesError);
create_exception!(rusty_tiles, UnsupportedError, TilesError);
create_exception!(rusty_tiles, InvalidRequestError, TilesError);
create_exception!(rusty_tiles, CancelledError, TilesError);
create_exception!(rusty_tiles, ObserverError, TilesError);

fn scratch_error(refusal: &tiles_core::validate::ScratchRefusal, message: String) -> PyErr {
    match refusal {
        tiles_core::validate::ScratchRefusal::MemoryBackedOnly { .. } => {
            EnvironmentError::new_err(message)
        }
        _ => TilesIOError::new_err(message),
    }
}

fn error_to_python(error: Error) -> PyErr {
    use tiles_core::validate::ValidationFailure;
    let message = error.to_string();
    match error {
        Error::Validation(failure) => match failure {
            ValidationFailure::InvalidInput(_) => DataError::new_err(message),
            ValidationFailure::Unsupported(_) => UnsupportedError::new_err(message),
            ValidationFailure::ResourceLimit(refusal) => {
                // VS-C-5: `limit` is the structured cause of a configurable-limit refusal.
                let error = ResourceLimitError::new_err(message);
                Python::attach(|py| {
                    let limit = match refusal.limit {
                        Some(limit) => serde_json::to_value(limit)
                            .ok()
                            .and_then(|value| json_to_python(py, &value).ok())
                            .unwrap_or_else(|| py.None()),
                        None => py.None(),
                    };
                    let _ = error.value(py).setattr("limit", limit);
                });
                error
            }
            ValidationFailure::Io(_) => TilesIOError::new_err(message),
            ValidationFailure::Scratch(refusal) => scratch_error(&refusal, message),
            ValidationFailure::InvalidRequest(_) => InvalidRequestError::new_err(message),
            ValidationFailure::Cancelled => CancelledError::new_err(message),
            ValidationFailure::Observer(_) => ObserverError::new_err(message),
            ValidationFailure::InvalidState(_) => TilesError::new_err(message),
        },
        Error::Scratch(refusal) => scratch_error(&refusal, message),
        Error::Job(failure) => Python::attach(|py| job_failure_to_python(py, failure)),
        Error::Environment(_) => EnvironmentError::new_err(message),
        Error::OutputExists(_) => OutputExistsError::new_err(message),
        Error::Io(_) => TilesIOError::new_err(message),
        Error::NotImplemented { .. } => UnsupportedError::new_err(message),
        _ => DataError::new_err(message),
    }
}

fn job_kind(kind: JobErrorKind) -> &'static str {
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

fn job_failure_to_python(py: Python<'_>, failure: JobFailure) -> PyErr {
    let message = failure.to_string();
    let error = match failure.error.kind() {
        JobErrorKind::InvalidRequest => InvalidRequestError::new_err(message),
        JobErrorKind::InvalidInput => DataError::new_err(message),
        JobErrorKind::Unsupported => UnsupportedError::new_err(message),
        JobErrorKind::ResourceLimit => ResourceLimitError::new_err(message),
        JobErrorKind::Io => TilesIOError::new_err(message),
        JobErrorKind::Conflict => OutputExistsError::new_err(message),
        JobErrorKind::Cancelled => CancelledError::new_err(message),
        JobErrorKind::ObserverFailure => ObserverError::new_err(message),
        JobErrorKind::InvalidState => TilesError::new_err(message),
    };
    // Newly created domain exceptions carry transport metadata. Original
    // callback/signal exceptions are returned untouched by the job adapter.
    let value = error.value(py);
    let _ = value.setattr("kind", job_kind(failure.error.kind()));
    let secondary: Vec<_> = failure
        .secondary
        .iter()
        .map(|cause| cause.to_string())
        .collect();
    let _ = value.setattr("secondary_diagnostics", secondary);
    let _ = value.setattr("retained_paths", failure.retained_paths);
    let _ = set_recovery(py, value, failure.recovery.as_ref());
    error
}

fn set_recovery(
    py: Python<'_>,
    exception: &Bound<'_, PyBaseException>,
    recovery: Option<&tiles_core::DirectoryRecovery>,
) -> PyResult<()> {
    if let Some(recovery) = recovery {
        let paths = PyDict::new(py);
        paths.set_item("output", &recovery.output)?;
        paths.set_item("previous_output", &recovery.previous_output)?;
        exception.setattr("recovery", paths)
    } else {
        exception.setattr("recovery", py.None())
    }
}

/// Packaging summary, separate from selected source file contents.
#[pyclass(frozen, module = "rusty_tiles")]
struct PackageReceipt {
    #[pyo3(get)]
    member_count: u64,
    #[pyo3(get)]
    source_bytes: u64,
    #[pyo3(get)]
    archive_bytes: u64,
}

#[derive(Clone)]
#[pyclass(frozen, skip_from_py_object, module = "rusty_tiles")]
struct CleanupDiagnostic {
    #[pyo3(get)]
    path: PathBuf,
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    message: String,
}

/// Successfully installed package and any postcommit cleanup diagnostics.
#[pyclass(frozen, module = "rusty_tiles")]
struct PackageResult {
    #[pyo3(get)]
    output: PathBuf,
    #[pyo3(get)]
    archive: bool,
    receipt: PackageReceipt,
    cleanup_diagnostics: Vec<CleanupDiagnostic>,
}

#[pymethods]
impl PackageResult {
    #[getter]
    fn receipt(&self) -> PackageReceipt {
        PackageReceipt {
            member_count: self.receipt.member_count,
            source_bytes: self.receipt.source_bytes,
            archive_bytes: self.receipt.archive_bytes,
        }
    }

    #[getter]
    fn cleanup_diagnostics(&self) -> Vec<CleanupDiagnostic> {
        self.cleanup_diagnostics
            .iter()
            .map(|diagnostic| CleanupDiagnostic {
                path: diagnostic.path.clone(),
                kind: diagnostic.kind.clone(),
                message: diagnostic.message.clone(),
            })
            .collect()
    }
}

struct RunObserver {
    callback: Option<Py<PyAny>>,
    error: Mutex<Option<PyErr>>,
}

fn run_event(event: &RunEvent<'_>) -> Value {
    match event {
        RunEvent::Progress { phase, done, total } => {
            serde_json::json!({"event":"progress", "phase":phase,"done":done,"total":total})
        }
        RunEvent::Warning { code, message } => {
            serde_json::json!({"event":"warning", "code":code,"message":message})
        }
        RunEvent::Note { message } => serde_json::json!({"event":"log", "message":message}),
    }
}

impl Observer for RunObserver {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        Python::attach(|py| {
            let result = py.check_signals().and_then(|()| {
                if let Some(callback) = &self.callback {
                    let value = json_to_python(py, &run_event(event))?;
                    callback.call1(py, (value,))?;
                }
                Ok(())
            });
            if let Err(error) = result {
                let message = error.to_string();
                {
                    let mut stored = self.error.lock().unwrap();
                    if stored.is_none() {
                        *stored = Some(error);
                    }
                }
                return Err(JobError::new(JobErrorKind::ObserverFailure, message));
            }
            Ok(())
        })
    }
}

/// Run a new fallible-observer job; postcommit Python checks do not rewrite its outcome.
fn run_job<T: Send>(
    py: Python<'_>,
    callback: Option<Py<PyAny>>,
    work: impl FnOnce(&RunControl) -> Result<T, JobFailure> + Send,
) -> PyResult<T> {
    if callback
        .as_ref()
        .is_some_and(|callback| !callback.bind(py).is_callable())
    {
        return Err(PyTypeError::new_err("callback must be callable"));
    }
    py.check_signals()?;
    let observer = Arc::new(RunObserver {
        callback,
        error: Mutex::new(None),
    });
    let run = RunControl::new(Some(observer.clone()));
    match py.detach(|| work(&run)) {
        Ok(result) => Ok(result),
        Err(failure) => {
            if failure.error.kind() == JobErrorKind::ObserverFailure {
                if let Some(error) = observer.error.lock().unwrap().take() {
                    return Err(error);
                }
            }
            Err(job_failure_to_python(py, failure))
        }
    }
}

fn cleanup_diagnostics(diagnostics: Vec<tiles_core::CleanupDiagnostic>) -> Vec<CleanupDiagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| CleanupDiagnostic {
            path: diagnostic.path,
            kind: job_kind(diagnostic.error.kind()).to_owned(),
            message: diagnostic.error.to_string(),
        })
        .collect()
}

/// One installed tiler archive; runtime diagnostics stay outside its semantic report.
#[pyclass(frozen, module = "rusty_tiles")]
struct TilerResult {
    #[pyo3(get)]
    output: PathBuf,
    #[pyo3(get)]
    archive: bool,
    report: Value,
    diagnostics: Value,
    cleanup_diagnostics: Vec<CleanupDiagnostic>,
}

#[pymethods]
impl TilerResult {
    #[getter]
    fn report(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_python(py, &self.report)
    }

    #[getter]
    fn diagnostics(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_python(py, &self.diagnostics)
    }

    #[getter]
    fn cleanup_diagnostics(&self) -> Vec<CleanupDiagnostic> {
        self.cleanup_diagnostics.clone()
    }
}

fn tiler_request_error(py: Python<'_>, error: JobError) -> PyErr {
    job_failure_to_python(
        py,
        JobFailure {
            error,
            secondary: Vec::new(),
            retained_paths: Vec::new(),
            recovery: None,
        },
    )
}

fn tiler_jobs(py: Python<'_>, jobs: Option<Bound<'_, PyAny>>) -> PyResult<TilerJobs> {
    let Some(jobs) = jobs else {
        return Ok(TilerJobs::Auto);
    };
    if jobs.is_instance_of::<PyBool>() || !jobs.is_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err(
            "jobs must be an integer maximum or None",
        ));
    }
    let count = jobs
        .extract::<usize>()
        .ok()
        .and_then(std::num::NonZeroUsize::new)
        .ok_or_else(|| {
            tiler_request_error(
                py,
                JobError::new(
                    JobErrorKind::InvalidRequest,
                    "jobs must be a positive integer within the platform worker-count range",
                ),
            )
        })?;
    Ok(TilerJobs::Maximum(count))
}

/// A declared memory budget: an integer byte count (not a bool), refused as an invalid request
/// when zero, negative or beyond 2^64 - 1. The conversion to decimal text lets the shared
/// core parser own every rule, so the CLI and Python refuse identically.
fn tiler_memory_budget(py: Python<'_>, budget: Option<Bound<'_, PyAny>>) -> PyResult<Option<u64>> {
    let Some(budget) = budget else {
        return Ok(None);
    };
    if budget.is_instance_of::<PyBool>() || !budget.is_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err(
            "memory_budget_bytes must be an integer byte count or None",
        ));
    }
    let text = budget
        .str()
        .and_then(|text| text.extract::<String>())
        .unwrap_or_else(|_| "9".repeat(64));
    tiles_core::parse_memory_budget(&text)
        .map(Some)
        .map_err(|error| tiler_request_error(py, error))
}

/// Stream an admitted static mesh into one Tiles 1.1 .3tz: local metre/Y-up, or a declared
/// source CRS mapped at capture. Profiles are provisional; jobs is a resource-capped
/// maximum, and None means auto. memory_budget_bytes is a positive integer ceiling for the
/// run (default: the lesser of 4 GiB and half the detected headroom, 2 GiB if undetected).
#[pyfunction]
#[pyo3(signature = (input, output, *, profile="balanced", jobs=None, memory_budget_bytes=None, scratch_dir=None, anchor=None, orientation_xyzw=None, scene_offset=None, source_crs=None, source_axes=None, source_offset=None, source_offset_file=None, height_offset=None, force=false, callback=None, _lod=None))]
#[allow(clippy::too_many_arguments)]
fn mesh_to_tileset(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    profile: &str,
    jobs: Option<Bound<'_, PyAny>>,
    memory_budget_bytes: Option<Bound<'_, PyAny>>,
    scratch_dir: Option<PathBuf>,
    anchor: Option<(f64, f64, f64)>,
    orientation_xyzw: Option<(f64, f64, f64, f64)>,
    scene_offset: Option<(f64, f64, f64)>,
    source_crs: Option<&str>,
    source_axes: Option<&str>,
    source_offset: Option<Vec<f64>>,
    source_offset_file: Option<PathBuf>,
    height_offset: Option<f64>,
    force: bool,
    callback: Option<Py<PyAny>>,
    // Private coarse-tile selector ("h17" or "hlod"), the CLI's hidden `--lod`. Undocumented
    // and unsupported; it goes when HLOD becomes the only method.
    _lod: Option<&str>,
) -> PyResult<TilerResult> {
    let profile = profile
        .parse::<TilerProfile>()
        .map_err(|error| tiler_request_error(py, error))?;
    let jobs = tiler_jobs(py, jobs)?;
    let memory_budget = tiler_memory_budget(py, memory_budget_bytes)?;
    let lod = _lod
        .map(str::parse::<TilerLod>)
        .transpose()
        .map_err(|error| tiler_request_error(py, error))?;
    let source_axes = source_axes
        .map(|axes| match axes {
            "xyz" => Ok(SourceAxes::Xyz),
            "y-up" => Ok(SourceAxes::YUp),
            _ => Err(tiler_request_error(
                py,
                JobError::new(
                    JobErrorKind::InvalidRequest,
                    "source_axes must be xyz or y-up",
                ),
            )),
        })
        .transpose()?;
    let source_crs = MeshSourceCrs::from_parameters(
        source_crs,
        source_axes,
        source_offset.as_deref(),
        source_offset_file.as_deref(),
        height_offset,
    )
    .map_err(|error| tiler_request_error(py, error))?;
    let mut request = TilerRequest::new(input, output)
        .with_profile(profile)
        .with_jobs(jobs)
        .with_placement(rigid_placement(anchor, orientation_xyzw, scene_offset)?)
        .with_output_policy(if force {
            OutputPolicy::Replace
        } else {
            OutputPolicy::CreateNew
        });
    if let Some(source_crs) = source_crs {
        request = request.with_source_crs(source_crs);
    }
    if let Some(path) = scratch_dir {
        request = request.with_scratch_dir(path);
    }
    if let Some(bytes) = memory_budget {
        request = request.with_memory_budget(bytes);
    }
    if let Some(lod) = lod {
        request = request.with_lod(lod);
    }
    let result = run_job(py, callback, |run| {
        tiles_core::mesh_to_tileset(request, run)
    })?;
    Ok(TilerResult {
        output: result.output,
        archive: true,
        report: serde_json::json!(result.report),
        diagnostics: serde_json::json!(result.diagnostics),
        cleanup_diagnostics: cleanup_diagnostics(result.cleanup_diagnostics),
    })
}

/// Published D1 raster directory; the standard wheel reports Unsupported.
#[pyclass(frozen, module = "rusty_tiles")]
struct RasterDirectoryResult {
    #[pyo3(get)]
    output: PathBuf,
    report: Value,
    cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
#[pymethods]
impl RasterDirectoryResult {
    #[getter]
    fn report(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_python(py, &self.report)
    }
    #[getter]
    fn cleanup_diagnostics(&self) -> Vec<CleanupDiagnostic> {
        self.cleanup_diagnostics.clone()
    }
}
#[pyfunction]
#[pyo3(signature = (input, output, *, zoom, x, y, force=false, callback=None))]
#[allow(clippy::too_many_arguments)]
fn raster_tile_to_directory(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    zoom: u8,
    x: u32,
    y: u32,
    force: bool,
    callback: Option<Py<PyAny>>,
) -> PyResult<RasterDirectoryResult> {
    let request = tiles_core::RasterDirectoryRequest::web_mercator_rgb(input, output, zoom, x, y)
        .with_policy(if force {
            tiles_core::OutputPolicy::Replace
        } else {
            tiles_core::OutputPolicy::CreateNew
        });
    let result = run_job(py, callback, |run| {
        tiles_core::raster_to_directory(request, run)
    })?;
    Ok(RasterDirectoryResult {
        output: result.output,
        report: serde_json::json!(result.report),
        cleanup_diagnostics: cleanup_diagnostics(result.cleanup_diagnostics),
    })
}

fn json_to_python(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    Ok(match value {
        Value::Null => py.None(),
        Value::Bool(v) => v.into_pyobject(py)?.to_owned().into_any().unbind(),
        Value::Number(v) => {
            if let Some(n) = v.as_i64() {
                n.into_pyobject(py)?.into_any().unbind()
            } else if let Some(n) = v.as_u64() {
                n.into_pyobject(py)?.into_any().unbind()
            } else {
                v.as_f64().unwrap().into_pyobject(py)?.into_any().unbind()
            }
        }
        Value::String(v) => v.into_pyobject(py)?.into_any().unbind(),
        Value::Array(values) => {
            let values = values
                .iter()
                .map(|v| json_to_python(py, v))
                .collect::<PyResult<Vec<_>>>()?;
            PyList::new(py, values)?.into_any().unbind()
        }
        Value::Object(values) => {
            let dict = PyDict::new(py);
            for (key, value) in values {
                dict.set_item(key, json_to_python(py, value)?)?;
            }
            dict.into_any().unbind()
        }
    })
}

#[pyclass(frozen, module = "rusty_tiles")]
struct ModelWrapResult {
    #[pyo3(get)]
    output: PathBuf,
    report: Value,
    cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
#[pymethods]
impl ModelWrapResult {
    #[getter]
    fn report(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_python(py, &self.report)
    }
    #[getter]
    fn cleanup_diagnostics(&self) -> Vec<CleanupDiagnostic> {
        self.cleanup_diagnostics.clone()
    }
}

/// Published sibling manifest; source resources remain caller owned.
#[pyclass(frozen, module = "rusty_tiles")]
struct ModelManifestResult {
    #[pyo3(get)]
    output: PathBuf,
    report: Value,
    cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
#[pymethods]
impl ModelManifestResult {
    #[getter]
    fn report(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_python(py, &self.report)
    }
    #[getter]
    fn cleanup_diagnostics(&self) -> Vec<CleanupDiagnostic> {
        self.cleanup_diagnostics.clone()
    }
}

fn rigid_placement(
    anchor: Option<(f64, f64, f64)>,
    orientation_xyzw: Option<(f64, f64, f64, f64)>,
    scene_offset: Option<(f64, f64, f64)>,
) -> PyResult<MeshPlacement> {
    MeshPlacement::from_parameters(
        anchor.map(|(x, y, z)| [x, y, z]),
        orientation_xyzw.map(|(x, y, z, w)| [x, y, z, w]),
        scene_offset.map(|(x, y, z)| [x, y, z]),
    )
    .map_err(|error| {
        Python::attach(|py| {
            job_failure_to_python(
                py,
                JobFailure {
                    error,
                    secondary: Vec::new(),
                    retained_paths: Vec::new(),
                    recovery: None,
                },
            )
        })
    })
}

/// Preserve admitted static metre/Y-up glTF source and resource bytes in a 3TZ.
#[pyfunction]
#[pyo3(signature = (input, output, *, anchor=None, orientation_xyzw=None, scene_offset=None, force=false, callback=None))]
#[allow(clippy::too_many_arguments)]
fn glb_to_3tz(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    anchor: Option<(f64, f64, f64)>,
    orientation_xyzw: Option<(f64, f64, f64, f64)>,
    scene_offset: Option<(f64, f64, f64)>,
    force: bool,
    callback: Option<Py<PyAny>>,
) -> PyResult<ModelWrapResult> {
    let request = ModelWrapRequest::local_gltf(input, output)
        .with_policy(if force {
            OutputPolicy::Replace
        } else {
            OutputPolicy::CreateNew
        })
        .with_placement(rigid_placement(anchor, orientation_xyzw, scene_offset)?);
    let result = run_job(py, callback, |run| {
        tiles_core::model_to_archive(request, run)
    })?;
    Ok(ModelWrapResult {
        output: result.output,
        report: serde_json::json!(result.report),
        cleanup_diagnostics: cleanup_diagnostics(result.cleanup_diagnostics),
    })
}

/// Write tileset.json beside one admitted static source model using F0 publication.
#[pyfunction]
#[pyo3(signature = (input, *, anchor=None, orientation_xyzw=None, scene_offset=None, force=false, callback=None))]
fn model_to_manifest(
    py: Python<'_>,
    input: PathBuf,
    anchor: Option<(f64, f64, f64)>,
    orientation_xyzw: Option<(f64, f64, f64, f64)>,
    scene_offset: Option<(f64, f64, f64)>,
    force: bool,
    callback: Option<Py<PyAny>>,
) -> PyResult<ModelManifestResult> {
    let request = ModelManifestRequest::local_gltf(input)
        .with_policy(if force {
            OutputPolicy::Replace
        } else {
            OutputPolicy::CreateNew
        })
        .with_placement(rigid_placement(anchor, orientation_xyzw, scene_offset)?);
    let result = run_job(py, callback, |run| {
        tiles_core::model_to_manifest(request, run)
    })?;
    Ok(ModelManifestResult {
        output: result.output,
        report: serde_json::json!(result.report),
        cleanup_diagnostics: cleanup_diagnostics(result.cleanup_diagnostics),
    })
}

/// Tile local or grid-free georeferenced LAS/LAZ point clouds.
#[pyfunction]
#[pyo3(signature = (input, output, *, source_crs, force=false, height_offset=None, max_points=8192, chunk_points=100_000, metadata_attributes=false, callback=None))]
#[allow(clippy::too_many_arguments)]
fn point_cloud_to_3tz(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    source_crs: &str,
    force: bool,
    height_offset: Option<f64>,
    max_points: usize,
    chunk_points: usize,
    metadata_attributes: bool,
    callback: Option<Py<PyAny>>,
) -> PyResult<PointCloudResult> {
    let coordinates = if source_crs == "local" {
        if height_offset.is_some() {
            return Err(job_failure_to_python(
                py,
                JobFailure {
                    error: JobError::new(
                        JobErrorKind::InvalidRequest,
                        "heightOffset applies only to geospatial CRS; local XYZ is in metres",
                    ),
                    secondary: vec![],
                    retained_paths: vec![],
                    recovery: None,
                },
            ));
        }
        PointCloudCoordinates::LocalMetres
    } else {
        PointCloudCoordinates::Horizontal {
            source: if source_crs == "header" { PointCloudCrs::Header } else { PointCloudCrs::Definition(source_crs.into()) },
            height_offset_metres: height_offset.filter(|value| value.is_finite()).ok_or_else(|| job_failure_to_python(py, JobFailure { error: JobError::new(JobErrorKind::InvalidRequest, "geospatial input requires explicit finite heightOffset to ellipsoidal metres"), secondary: vec![], retained_paths: vec![], recovery: None }))?,
        }
    };
    let options = PointCloudOptions {
        max_points,
        chunk_points,
        metadata_attributes,
    };
    let policy = if force {
        OutputPolicy::Replace
    } else {
        OutputPolicy::CreateNew
    };
    let request = PointCloudRequest::new(input, output, coordinates, options).with_policy(policy);
    let result = run_job(py, callback, |run| {
        tiles_core::point_cloud::point_cloud_to_archive(request, run)
    })?;
    Ok(PointCloudResult {
        output: result.output,
        report: serde_json::to_value(result.report)
            .expect("point report contains JSON-compatible values"),
        cleanup_diagnostics: cleanup_diagnostics(result.cleanup_diagnostics),
    })
}

/// Published point-cloud archive and finalized acceptance report.
#[pyclass(frozen, module = "rusty_tiles")]
struct PointCloudResult {
    #[pyo3(get)]
    output: PathBuf,
    report: Value,
    cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
#[pymethods]
impl PointCloudResult {
    #[getter]
    fn archive(&self) -> bool {
        true
    }
    #[getter]
    fn report(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_python(py, &self.report)
    }
    #[getter]
    fn cleanup_diagnostics(&self) -> Vec<CleanupDiagnostic> {
        self.cleanup_diagnostics.clone()
    }
}

/// Published vector archive and finalized acceptance report.
#[pyclass(frozen, module = "rusty_tiles")]
struct VectorResult {
    #[pyo3(get)]
    output: PathBuf,
    report: Value,
    cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
#[pymethods]
impl VectorResult {
    #[getter]
    fn archive(&self) -> bool {
        true
    }
    #[getter]
    fn report(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_python(py, &self.report)
    }
    #[getter]
    fn cleanup_diagnostics(&self) -> Vec<CleanupDiagnostic> {
        self.cleanup_diagnostics.clone()
    }
}

/// Tile GeoJSON or GeoPackage vector features using the portable pipeline.
#[pyfunction]
#[pyo3(signature = (input, output, *, force=false, source_crs=None, height_offset=None,
    layers=None, all_layers=false, fields=None, drop_fields=None, list_fields="error",
    where_clause=None, repair=false, ambiguous_outlines=false, skip_invalid=false,
    max_features=64, max_vertices=65_536, max_bytes=4_194_304, max_tiles=100_000,
    max_source_vertices=1_000_000, lod_tolerance=0.1, lod_levels=3, jobs=None,
    reproducible=false, quantize=false, meshopt=false, parent_repair=false,
    aggregate_points=false, max_parent_features=4096, reuse_tileset=None, callback=None))]
#[allow(clippy::too_many_arguments)]
fn vector_to_3tz(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    force: bool,
    source_crs: Option<String>,
    height_offset: Option<f64>,
    layers: Option<Vec<String>>,
    all_layers: bool,
    fields: Option<Vec<String>>,
    drop_fields: Option<Vec<String>>,
    list_fields: &str,
    where_clause: Option<String>,
    repair: bool,
    ambiguous_outlines: bool,
    skip_invalid: bool,
    max_features: usize,
    max_vertices: usize,
    max_bytes: usize,
    max_tiles: usize,
    max_source_vertices: usize,
    lod_tolerance: f64,
    lod_levels: u8,
    jobs: Option<usize>,
    reproducible: bool,
    quantize: bool,
    meshopt: bool,
    parent_repair: bool,
    aggregate_points: bool,
    max_parent_features: usize,
    reuse_tileset: Option<PathBuf>,
    callback: Option<Py<PyAny>>,
) -> PyResult<VectorResult> {
    let defaults = VectorOptions::default();
    let options = VectorOptions {
        max_features,
        repair,
        ambiguous_outlines,
        source_crs,
        height_offset,
        layers: layers.unwrap_or_default(),
        all_layers,
        fields: fields.unwrap_or_default(),
        drop_fields: drop_fields.unwrap_or_default(),
        list_fields: list_fields.into(),
        where_clause,
        skip_invalid,
        max_vertices,
        max_bytes,
        max_tiles,
        max_source_vertices,
        lod: VectorLodOptions {
            tolerance_metres: lod_tolerance,
            levels: lod_levels,
        },
        jobs: jobs.unwrap_or(defaults.jobs),
        reproducible,
        quantize,
        meshopt,
        parent_repair,
        aggregate_points,
        max_parent_features,
        reuse_tileset,
    };
    let policy = if force {
        OutputPolicy::Replace
    } else {
        OutputPolicy::CreateNew
    };
    let request =
        tiles_core::vector::VectorRequest::new(input, output, options).with_policy(policy);
    let result = run_job(py, callback, |run| {
        tiles_core::vector::vector_to_archive(request, run)
    })?;
    Ok(VectorResult {
        output: result.output,
        report: result.report,
        cleanup_diagnostics: cleanup_diagnostics(result.cleanup_diagnostics),
    })
}

/// Pack a tileset directory or tileset.json path as a 3TZ archive.
#[pyfunction]
#[pyo3(signature = (input, output, *, force=false, callback=None))]
fn convert_to_3tz(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    force: bool,
    callback: Option<Py<PyAny>>,
) -> PyResult<PackageResult> {
    let policy = if force {
        OutputPolicy::Replace
    } else {
        OutputPolicy::CreateNew
    };
    let request = PackageRequest::directory(input, output).with_policy(policy);
    let result = run_job(py, callback, |run| package(request, run))?;
    Ok(PackageResult {
        output: result.output,
        archive: true,
        receipt: PackageReceipt {
            member_count: result.receipt.member_count,
            source_bytes: result.receipt.source_bytes,
            archive_bytes: result.receipt.archive_bytes,
        },
        cleanup_diagnostics: cleanup_diagnostics(result.cleanup_diagnostics),
    })
}

/// A validation report's payload rows: a read-only sequence of dicts in URI order, read on
/// demand so a large report is never materialized.
#[pyclass(frozen, sequence, module = "rusty_tiles", name = "ValidationPayloads")]
struct ValidationPayloads {
    rows: tiles_core::validate::PayloadRows,
}

impl ValidationPayloads {
    fn row(&self, py: Python<'_>, index: usize) -> PyResult<Py<PyAny>> {
        let row = self
            .rows
            .get(index)
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("payload index out of range"))?
            .map_err(|failure| error_to_python(Error::Validation(failure)))?;
        let row = serde_json::to_value(row).map_err(|e| DataError::new_err(e.to_string()))?;
        json_to_python(py, &row)
    }
}

#[pymethods]
impl ValidationPayloads {
    fn __len__(&self) -> usize {
        self.rows.len()
    }

    fn __getitem__(&self, py: Python<'_>, index: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let len = self.rows.len();
        if let Ok(slice) = index.cast::<pyo3::types::PySlice>() {
            let range = slice.indices(len as isize)?;
            let mut rows = Vec::with_capacity(range.slicelength);
            let mut at = range.start;
            for _ in 0..range.slicelength {
                rows.push(self.row(py, at as usize)?);
                at += range.step;
            }
            return Ok(PyList::new(py, rows)?.into_any().unbind());
        }
        let index: isize = index.extract()?;
        let resolved = if index < 0 {
            index + len as isize
        } else {
            index
        };
        if resolved < 0 || resolved as usize >= len {
            return Err(pyo3::exceptions::PyIndexError::new_err(
                "payload index out of range",
            ));
        }
        self.row(py, resolved as usize)
    }

    fn __iter__(slf: PyRef<'_, Self>) -> ValidationPayloadsIter {
        ValidationPayloadsIter {
            rows: slf.rows.clone(),
            next: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn __repr__(&self) -> String {
        format!("<ValidationPayloads of {} rows>", self.rows.len())
    }
}

#[pyclass(frozen, module = "rusty_tiles")]
struct ValidationPayloadsIter {
    rows: tiles_core::validate::PayloadRows,
    next: std::sync::atomic::AtomicUsize,
}

#[pymethods]
impl ValidationPayloadsIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let index = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if index >= self.rows.len() {
            return Ok(None);
        }
        ValidationPayloads {
            rows: self.rows.clone(),
        }
        .row(py, index)
        .map(Some)
    }
}

/// Inspect the bounded C1 archive/payload profile; see checks and notInspected.
/// `report["payloads"]` is a list of dicts, so the report is plain JSON data. With
/// `lazy_payloads=True` it is a read-only `ValidationPayloads` sequence that reads rows on
/// demand instead, for reports too large to hold (it is not JSON-serializable as is;
/// `list(...)` materializes it).
///
/// `max_volume` maps configurable limit keys (`sourceArchiveBytes`, `archiveEntries`,
/// `centralDirectoryBytes`, `archiveStoredBytes`, `hierarchyVisits`, `references`,
/// `totalBytesRead`, `totalPayloadElements`, `containmentWorkUnits`, `containmentBytesRead`,
/// `containmentDecodedBytes`) to non-negative ints below 2**64: a volume cap is set, a budget is
/// replaced (values below the defaults are allowed). `scratch_dir` places validation scratch
/// in that existing directory. `callback` receives progress events; raising from it (or
/// Ctrl-C) stops the validation and the exception propagates.
#[pyfunction]
#[pyo3(signature = (input, *, lazy_payloads = false, max_volume = None, scratch_dir = None, callback = None))]
fn validate(
    py: Python<'_>,
    input: PathBuf,
    lazy_payloads: bool,
    max_volume: Option<Bound<'_, PyAny>>,
    scratch_dir: Option<PathBuf>,
    callback: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    use tiles_core::validate::{LimitKey, ValidationFailure, ValidationRequest};
    if callback
        .as_ref()
        .is_some_and(|callback| !callback.bind(py).is_callable())
    {
        return Err(PyTypeError::new_err("callback must be callable"));
    }
    let mut request = ValidationRequest::new(input);
    if let Some(mapping) = max_volume {
        let mapping = mapping
            .cast::<pyo3::types::PyMapping>()
            .map_err(|_| InvalidRequestError::new_err("max_volume must be a mapping"))?;
        for item in mapping.items()?.iter() {
            let (key, value): (Bound<'_, PyAny>, Bound<'_, PyAny>) = item.extract()?;
            let key: String = key
                .extract()
                .map_err(|_| InvalidRequestError::new_err("max_volume keys must be strings"))?;
            let key = key
                .parse::<LimitKey>()
                .map_err(|error| InvalidRequestError::new_err(error.to_string()))?;
            if value.is_instance_of::<pyo3::types::PyBool>()
                || !value.is_instance_of::<pyo3::types::PyInt>()
            {
                return Err(InvalidRequestError::new_err(format!(
                    "max_volume[{key:?}] must be an int",
                    key = key.as_str()
                )));
            }
            let value: u64 = value.extract().map_err(|_| {
                InvalidRequestError::new_err(format!(
                    "max_volume[{:?}] must be in 0..2**64",
                    key.as_str()
                ))
            })?;
            request = request.max_volume(key, value);
        }
    }
    if let Some(dir) = scratch_dir {
        request = request.scratch_dir(dir);
    }
    py.check_signals()?;
    let observer = Arc::new(RunObserver {
        callback,
        error: Mutex::new(None),
    });
    let run = RunControl::new(Some(observer.clone()));
    let result = match py.detach(|| tiles_core::validate::inspect_with(request, &run)) {
        Ok(result) => result,
        Err(failure) => {
            if matches!(failure, ValidationFailure::Observer(_)) {
                if let Some(error) = observer.error.lock().unwrap().take() {
                    return Err(error);
                }
            }
            return Err(error_to_python(Error::Validation(failure)));
        }
    };
    py.check_signals()?;
    let mut summary = result
        .summary_json()
        .map_err(|e| DataError::new_err(e.to_string()))?;
    // Insert the key here so the dict keeps the CLI's sorted key order.
    summary["payloads"] = Value::Null;
    let report = json_to_python(py, &summary)?;
    let lazy = ValidationPayloads {
        rows: result.payloads,
    };
    let payloads = if lazy_payloads {
        Py::new(py, lazy)?.into_any()
    } else {
        let rows = (0..lazy.rows.len())
            .map(|index| lazy.row(py, index))
            .collect::<PyResult<Vec<_>>>()?;
        PyList::new(py, rows)?.into_any().unbind()
    };
    report.bind(py).set_item("payloads", payloads)?;
    Ok(report)
}

#[pymodule]
fn rusty_tiles(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add_class::<ValidationPayloads>()?;
    module.add_class::<VectorResult>()?;
    module.add_class::<PointCloudResult>()?;
    module.add_class::<PackageResult>()?;
    module.add_class::<PackageReceipt>()?;
    module.add_class::<TilerResult>()?;
    module.add_class::<ModelWrapResult>()?;
    module.add_class::<ModelManifestResult>()?;
    module.add_class::<RasterDirectoryResult>()?;
    module.add_class::<CleanupDiagnostic>()?;
    module.add("TilesError", module.py().get_type::<TilesError>())?;
    module.add(
        "InvalidRequestError",
        module.py().get_type::<InvalidRequestError>(),
    )?;
    module.add("CancelledError", module.py().get_type::<CancelledError>())?;
    module.add("ObserverError", module.py().get_type::<ObserverError>())?;
    module.add("DataError", module.py().get_type::<DataError>())?;
    module.add(
        "ResourceLimitError",
        module.py().get_type::<ResourceLimitError>(),
    )?;
    module.add(
        "EnvironmentError",
        module.py().get_type::<EnvironmentError>(),
    )?;
    module.add(
        "OutputExistsError",
        module.py().get_type::<OutputExistsError>(),
    )?;
    module.add("TilesIOError", module.py().get_type::<TilesIOError>())?;
    module.add(
        "UnsupportedError",
        module.py().get_type::<UnsupportedError>(),
    )?;
    module.add_function(wrap_pyfunction!(mesh_to_tileset, module)?)?;
    module.add_function(wrap_pyfunction!(raster_tile_to_directory, module)?)?;
    module.add_function(wrap_pyfunction!(glb_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(model_to_manifest, module)?)?;
    module.add_function(wrap_pyfunction!(point_cloud_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(vector_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(convert_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(validate, module)?)?;
    Ok(())
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn producer_resource_refusal_uses_existing_exception_and_explicit_kind() {
        Python::initialize();
        Python::attach(|py| {
            let failure = JobFailure {
                error: tiles_core::JobError::new(JobErrorKind::ResourceLimit, "codec work refused"),
                secondary: Vec::new(),
                retained_paths: Vec::new(),
                recovery: None,
            };
            let error = job_failure_to_python(py, failure);
            assert!(error.is_instance_of::<ResourceLimitError>(py));
            assert_eq!(
                error
                    .value(py)
                    .getattr("kind")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "resource_limit"
            );
        });
    }

    #[test]
    fn failed_restore_maps_primary_secondary_and_distinct_recovery_paths() {
        Python::initialize();
        Python::attach(|py| {
            #[cfg(unix)]
            let old = {
                use std::os::unix::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/previous-\xff".to_vec()))
            };
            #[cfg(not(unix))]
            let old = PathBuf::from("previous-output");
            let output = PathBuf::from("destination");
            let failure = JobFailure {
                error: tiles_core::JobError::new(JobErrorKind::Io, "install failed"),
                secondary: vec![tiles_core::JobError::new(
                    JobErrorKind::Conflict,
                    "restore occupied",
                )],
                retained_paths: vec![PathBuf::from("new-staging")],
                recovery: Some(tiles_core::DirectoryRecovery {
                    output: output.clone(),
                    previous_output: old.clone(),
                }),
            };
            let error = job_failure_to_python(py, failure);
            assert!(error.is_instance_of::<TilesIOError>(py));
            let value = error.value(py);
            assert_eq!(
                value.getattr("kind").unwrap().extract::<String>().unwrap(),
                "io"
            );
            assert_eq!(
                value
                    .getattr("secondary_diagnostics")
                    .unwrap()
                    .extract::<Vec<String>>()
                    .unwrap(),
                ["restore occupied"]
            );
            assert_eq!(
                value
                    .getattr("retained_paths")
                    .unwrap()
                    .extract::<Vec<PathBuf>>()
                    .unwrap(),
                [PathBuf::from("new-staging")]
            );
            let recovery = value.getattr("recovery").unwrap();
            let paths = recovery.cast::<PyDict>().unwrap();
            assert_eq!(paths.len(), 2);
            assert_eq!(
                paths
                    .get_item("output")
                    .unwrap()
                    .unwrap()
                    .extract::<PathBuf>()
                    .unwrap(),
                output
            );
            assert_eq!(
                paths
                    .get_item("previous_output")
                    .unwrap()
                    .unwrap()
                    .extract::<PathBuf>()
                    .unwrap(),
                old
            );
        });
    }
    #[test]
    fn committed_cleanup_diagnostic_remains_a_result_value() {
        let diagnostics = cleanup_diagnostics(vec![tiles_core::CleanupDiagnostic {
            path: PathBuf::from("previous-output"),
            error: tiles_core::JobError::new(JobErrorKind::Io, "backup cleanup failed"),
        }]);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path, PathBuf::from("previous-output"));
        assert_eq!(diagnostics[0].kind, "io");
        assert_eq!(diagnostics[0].message, "backup cleanup failed");
    }
}
