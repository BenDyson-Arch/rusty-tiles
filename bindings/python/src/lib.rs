//! Path-based bindings for the portable default build.
use pyo3::{
    create_exception,
    exceptions::{PyException, PyTypeError, PyValueError},
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
    pack::PackOptions,
    point_cloud::PointCloudOptions,
    report::ndjson,
    tile::TextureFormat,
    vector::{VectorLodOptions, VectorOptions},
    Cartographic, CreateTilesetOptions, Error, Event, EventSink, MeshTo3tzOptions, Reporter,
    SourceAxes,
};

create_exception!(rusty_tiles, TilesError, PyException);
create_exception!(rusty_tiles, DataError, TilesError);
create_exception!(rusty_tiles, EnvironmentError, TilesError);
create_exception!(rusty_tiles, OutputExistsError, TilesError);
create_exception!(rusty_tiles, TilesIOError, TilesError);
create_exception!(rusty_tiles, UnsupportedError, TilesError);

fn error_to_python(error: Error) -> PyErr {
    let message = error.to_string();
    match error {
        Error::Environment(_) => EnvironmentError::new_err(message),
        Error::OutputExists(_) => OutputExistsError::new_err(message),
        Error::Io(_) | Error::InputNotFound(_) => TilesIOError::new_err(message),
        Error::NotImplemented { .. } => UnsupportedError::new_err(message),
        _ => DataError::new_err(message),
    }
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
#[pyo3(signature = (input, output, *, force=false))]
fn convert_to_3tz(
    py: Python<'_>,
    input: PathBuf,
    output: PathBuf,
    force: bool,
) -> PyResult<ConversionResult> {
    run_conversion(py, None, |_| {
        tiles_core::pack::convert_to_3tz_reported(&input, &output, &PackOptions { force })
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
    module.add("TilesError", module.py().get_type::<TilesError>())?;
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
    module.add_function(wrap_pyfunction!(glb_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(point_cloud_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(vector_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(convert_to_3tz, module)?)?;
    module.add_function(wrap_pyfunction!(convert_to_implicit, module)?)?;
    module.add_function(wrap_pyfunction!(validate, module)?)?;
    Ok(())
}
