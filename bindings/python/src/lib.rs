//! Path-based bindings for the portable default build.
use pyo3::{
    create_exception,
    exceptions::{PyBaseException, PyException, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tiles_core::{
    georef::{RotationDegrees, SourceOffset},
    package::{package, PackageRequest},
    point_cloud::PointCloudOptions,
    report::ndjson,
    tile::TextureFormat,
    vector::{VectorLodOptions, VectorOptions},
    Cartographic, CreateTilesetOptions, Error, Event, EventSink, JobError, JobErrorKind,
    JobFailure, MeshRequest, MeshTo3tzOptions, Observer, OutputPolicy, Reporter, RunControl,
    RunEvent, SourceAxes,
};

create_exception!(rusty_tiles, TilesError, PyException);
create_exception!(rusty_tiles, DataError, TilesError);
create_exception!(rusty_tiles, EnvironmentError, TilesError);
create_exception!(rusty_tiles, OutputExistsError, TilesError);
create_exception!(rusty_tiles, TilesIOError, TilesError);
create_exception!(rusty_tiles, UnsupportedError, TilesError);
create_exception!(rusty_tiles, InvalidRequestError, TilesError);
create_exception!(rusty_tiles, CancelledError, TilesError);
create_exception!(rusty_tiles, ObserverError, TilesError);

fn error_to_python(error: Error) -> PyErr {
    let message = error.to_string();
    match error {
        Error::Job(failure) => Python::attach(|py| job_failure_to_python(py, failure)),
        Error::Environment(_) => EnvironmentError::new_err(message),
        Error::OutputExists(_) => OutputExistsError::new_err(message),
        Error::Io(_) | Error::InputNotFound(_) => TilesIOError::new_err(message),
        Error::NotImplemented { .. } => UnsupportedError::new_err(message),
        _ => DataError::new_err(message),
    }
}

fn job_kind(kind: JobErrorKind) -> &'static str {
    match kind {
        JobErrorKind::InvalidRequest => "invalid_request",
        JobErrorKind::InvalidInput => "invalid_input",
        JobErrorKind::Unsupported => "unsupported",
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

/// Published F1a local mesh with its finalized report and cleanup diagnostics.
#[pyclass(frozen, module = "rusty_tiles")]
struct MeshResult {
    #[pyo3(get)]
    output: PathBuf,
    report: Value,
    cleanup_diagnostics: Vec<CleanupDiagnostic>,
}

#[pymethods]
impl MeshResult {
    #[getter]
    fn report(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_python(py, &self.report)
    }

    #[getter]
    fn cleanup_diagnostics(&self) -> Vec<CleanupDiagnostic> {
        self.cleanup_diagnostics.clone()
    }
}

/// Convert the F1a embedded static untextured GLB profile in local metre/Y-up coordinates.
#[pyfunction]
#[pyo3(signature = (input, output, *, leaf_triangles, force=false, callback=None))]
fn mesh_local_to_3tz(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    leaf_triangles: usize,
    force: bool,
    callback: Option<Py<PyAny>>,
) -> PyResult<MeshResult> {
    let policy = if force {
        OutputPolicy::Replace
    } else {
        OutputPolicy::CreateNew
    };
    let request = MeshRequest::local_gltf(input, output, leaf_triangles).with_policy(policy);
    let result = run_job(py, callback, |run| {
        tiles_core::mesh_to_archive(request, run)
    })?;
    Ok(MeshResult {
        output: result.output,
        report: serde_json::json!(result.report),
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

/// Published output and optional conversion report.
#[pyclass(frozen, module = "rusty_tiles")]
struct ConversionResult {
    #[pyo3(get)]
    output: PathBuf,
    #[pyo3(get)]
    archive: bool,
    report: Option<Value>,
}

#[pymethods]
impl ConversionResult {
    #[getter]
    fn report(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.report {
            Some(value) => json_to_python(py, value),
            None => Ok(py.None()),
        }
    }
}

struct CallbackSink {
    callback: Py<PyAny>,
    error: Mutex<Option<PyErr>>,
}

impl EventSink for CallbackSink {
    fn event(&self, event: &Event<'_>) {
        Python::attach(|py| {
            // Never hold the mutex while calling Python: callbacks may re-enter us.
            if self.error.lock().unwrap().is_some() {
                return;
            }
            let result = json_to_python(py, &ndjson(event))
                .and_then(|value| self.callback.call1(py, (value,)));
            if let Err(error) = result {
                let mut first = self.error.lock().unwrap();
                if first.is_none() {
                    *first = Some(error);
                }
            }
        });
    }
}

fn run_conversion(
    py: Python<'_>,
    callback: Option<Py<PyAny>>,
    work: impl FnOnce(&Reporter) -> Result<tiles_core::ConversionResult, Error> + Send,
) -> PyResult<ConversionResult> {
    let sink = callback
        .map(|callback| {
            if !callback.bind(py).is_callable() {
                return Err(PyTypeError::new_err("callback must be callable"));
            }
            Ok(Arc::new(CallbackSink {
                callback,
                error: Mutex::new(None),
            }))
        })
        .transpose()?;
    let reporter = sink
        .as_ref()
        .map_or_else(Reporter::silent, |sink| Reporter::custom(sink.clone()));
    let result = py.detach(|| work(&reporter));
    if let Some(sink) = sink {
        if let Some(error) = sink.error.lock().unwrap().take() {
            return Err(error);
        }
    }
    py.check_signals()?;
    let result = result.map_err(error_to_python)?;
    Ok(ConversionResult {
        output: result.output,
        archive: result.archive,
        report: result.report,
    })
}

fn placement(
    cartographic: Option<[f64; 3]>,
    rotation: Option<[f64; 3]>,
) -> PyResult<(Option<Cartographic>, Option<RotationDegrees>)> {
    if let Some([lon, lat, height]) = cartographic {
        if !lon.is_finite()
            || !(-180.0..=180.0).contains(&lon)
            || !lat.is_finite()
            || !(-90.0..=90.0).contains(&lat)
            || !height.is_finite()
        {
            return Err(PyValueError::new_err("cartographic must be finite (longitude degrees, latitude degrees, height metres) within longitude/latitude bounds"));
        }
    }
    if rotation.is_some_and(|v| v.iter().any(|n| !n.is_finite())) {
        return Err(PyValueError::new_err(
            "rotation must contain finite heading, pitch and roll degrees",
        ));
    }
    Ok((
        cartographic.map(|[lon, lat, height]| Cartographic::new(lon, lat, height)),
        rotation.map(|[heading, pitch, roll]| RotationDegrees {
            heading,
            pitch,
            roll,
        }),
    ))
}

/// Tile a mesh using the portable Rust texture encoders.
#[pyfunction]
#[pyo3(signature = (input, output, *, cartographic=None, rotation=None, force=false,
    max_triangles=20_000, max_bytes=204_800, tile_size=2048, texture_format="lossless",
    source_crs="auto", source_offset=None, meshopt=true, explicit=false, node_features=false,
    source_axes=None, height_offset=None, callback=None))]
#[allow(clippy::too_many_arguments)]
fn mesh_to_3tz(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    cartographic: Option<[f64; 3]>,
    rotation: Option<[f64; 3]>,
    force: bool,
    max_triangles: usize,
    max_bytes: u64,
    tile_size: u32,
    texture_format: &str,
    source_crs: &str,
    source_offset: Option<[f64; 3]>,
    meshopt: bool,
    explicit: bool,
    node_features: bool,
    source_axes: Option<&str>,
    height_offset: Option<f64>,
    callback: Option<Py<PyAny>>,
) -> PyResult<ConversionResult> {
    let (cartographic, rotation) = placement(cartographic, rotation)?;
    let texture_format = match texture_format {
        "lossless" => TextureFormat::Lossless,
        "jpeg" => TextureFormat::Jpeg,
        "webp" => TextureFormat::Webp,
        _ => {
            return Err(PyValueError::new_err(
                "texture_format must be lossless, jpeg or webp",
            ))
        }
    };
    let source_axes = source_axes
        .map(|axes| match axes {
            "xyz" => Ok(SourceAxes::Xyz),
            "y-up" => Ok(SourceAxes::YUp),
            _ => Err(PyValueError::new_err("source_axes must be xyz or y-up")),
        })
        .transpose()?;
    if source_offset.is_some_and(|v| v.iter().any(|n| !n.is_finite())) {
        return Err(PyValueError::new_err(
            "source_offset must contain finite offsets (E/N in horizontal units, A in metres)",
        ));
    }
    let mut options = MeshTo3tzOptions {
        cartographic,
        rotation,
        force,
        max_triangles,
        max_bytes,
        tile_size,
        texture_format,
        source_axes,
        height_offset,
        source_offset: source_offset.map(|[easting, northing, height]| SourceOffset {
            easting,
            northing,
            height,
        }),
        meshopt,
        explicit,
        node_features,
        ..MeshTo3tzOptions::default()
    };
    options
        .set_source_crs(source_crs)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    run_conversion(py, callback, |reporter| {
        tiles_core::tile::mesh_to_3tz_reported(&input, &output, &options, reporter)
    })
}

/// Pack a glTF/GLB model without making new levels of detail.
#[pyfunction]
#[pyo3(signature = (input, output, *, cartographic=None, rotation=None, force=false))]
fn glb_to_3tz(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    cartographic: Option<[f64; 3]>,
    rotation: Option<[f64; 3]>,
    force: bool,
) -> PyResult<ConversionResult> {
    let (cartographic, rotation) = placement(cartographic, rotation)?;
    let options = CreateTilesetOptions {
        cartographic,
        rotation,
        force,
    };
    run_conversion(py, None, |_| {
        tiles_core::tileset::glb_to_3tz_reported(&input, &output, &options)
    })
}

/// Tile local or grid-free georeferenced LAS/LAZ point clouds.
#[pyfunction]
#[pyo3(signature = (input, output, *, force=false, source_crs="local", height_offset=None, max_points=50_000, chunk_points=100_000, explicit=false, metadata_attributes=false, callback=None))]
#[allow(clippy::too_many_arguments)]
fn point_cloud_to_3tz(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    force: bool,
    source_crs: &str,
    height_offset: Option<f64>,
    max_points: usize,
    chunk_points: usize,
    explicit: bool,
    metadata_attributes: bool,
    callback: Option<Py<PyAny>>,
) -> PyResult<ConversionResult> {
    let options = PointCloudOptions {
        force,
        source_crs: source_crs.into(),
        height_offset,
        max_points,
        chunk_points,
        explicit,
        metadata_attributes,
    };
    run_conversion(py, callback, |reporter| {
        tiles_core::point_cloud::point_cloud_to_3tz_reported(&input, &output, &options, reporter)
    })
}

/// Tile GeoJSON or GeoPackage vector features using the portable pipeline.
#[pyfunction]
#[pyo3(signature = (input, output, *, force=false, source_crs=None, height_offset=None,
    layers=None, all_layers=false, fields=None, drop_fields=None, list_fields="error",
    where_clause=None, repair=false, ambiguous_outlines=false, skip_invalid=false,
    max_features=64, max_vertices=65_536, max_bytes=4_194_304, max_tiles=100_000,
    max_source_vertices=1_000_000, lod_tolerance=0.1, lod_levels=3, jobs=None,
    explicit=false, reproducible=false, quantize=false, meshopt=false, parent_repair=false,
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
    explicit: bool,
    reproducible: bool,
    quantize: bool,
    meshopt: bool,
    parent_repair: bool,
    aggregate_points: bool,
    max_parent_features: usize,
    reuse_tileset: Option<PathBuf>,
    callback: Option<Py<PyAny>>,
) -> PyResult<ConversionResult> {
    let defaults = VectorOptions::default();
    let options = VectorOptions {
        force,
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
        explicit,
        reproducible,
        quantize,
        meshopt,
        parent_repair,
        aggregate_points,
        max_parent_features,
        reuse_tileset,
        ..defaults
    };
    run_conversion(py, callback, |reporter| {
        tiles_core::vector::vector_to_3tz_reported(
            &input,
            &output,
            max_features,
            repair,
            ambiguous_outlines,
            &options,
            reporter,
        )
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

/// Rewrite an eligible rusty-tiles explicit point/vector archive as implicit tiling.
#[pyfunction]
#[pyo3(signature = (input, output, *, force=false))]
fn convert_to_implicit(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    force: bool,
) -> PyResult<ConversionResult> {
    run_conversion(py, None, |reporter| {
        tiles_core::convert_to_implicit_reported(
            &input,
            &output,
            &tiles_core::ConvertToImplicitOptions { force },
            reporter,
        )
    })
}

/// Check an archive's index, schema, bounds, references, hashes and budgets.
#[pyfunction]
fn validate(py: Python<'_>, input: PathBuf) -> PyResult<Py<PyAny>> {
    let result = py
        .detach(|| tiles_core::validate::archive(&input, None))
        .map_err(error_to_python)?;
    py.check_signals()?;
    json_to_python(py, &result)
}

#[pymodule]
fn rusty_tiles(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add_class::<ConversionResult>()?;
    module.add_class::<PackageResult>()?;
    module.add_class::<PackageReceipt>()?;
    module.add_class::<MeshResult>()?;
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
    module.add_function(wrap_pyfunction!(mesh_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(mesh_local_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(raster_tile_to_directory, module)?)?;
    module.add_function(wrap_pyfunction!(glb_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(point_cloud_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(vector_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(convert_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(convert_to_implicit, module)?)?;
    module.add_function(wrap_pyfunction!(validate, module)?)?;
    Ok(())
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
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
