use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::builder::{PossibleValue, PossibleValuesParser};
use clap::{Args, Parser, Subcommand};
use serde_json::{json, Value};

use rusty_tiles::error::Error;
use rusty_tiles::georef::{
    parse_metashape_offset, Cartographic, RotationDegrees, SourceAxes, SourceCrs, SourceOffset,
};
use rusty_tiles::package::{package, PackageRequest, PackageResult};
use rusty_tiles::tile::{mesh_to_3tz_reported, MeshTo3tzOptions};
use rusty_tiles::tileset::{create_tileset_json, glb_to_3tz_reported, CreateTilesetOptions};
use rusty_tiles::{
    doctor, terrain, vector, ConversionResult, JobError, JobErrorKind, Observer, OutputPolicy,
    Reporter, RunControl, RunEvent,
};

// Option spelling: multi-word options keep their camelCase name as the primary
// spelling (3d-tiles-tools compatibility, scripts and machine contracts) and
// always carry a `visible_alias` kebab-case spelling, e.g.
// `#[arg(long = "maxZoom", visible_alias = "max-zoom")]`. Single-word options
// have one spelling. A unit test below enforces this for every subcommand.

#[derive(Parser)]
#[command(
    name = "rusty-tiles",
    about = "Transform geospatial sources into 3D Tiles (.3tz). createTilesetJson and convert match 3d-tiles-tools@0.5.4 argv.",
    version,
    arg_required_else_help = true
)]
struct Cli {
    /// Emit one machine-readable result on stdout (diagnostics stay on stderr)
    #[arg(long, global = true, display_order = 900)]
    json: bool,
    /// Emit newline-delimited phase events on stderr
    #[arg(long, global = true, display_order = 901, value_parser = ["json"])]
    progress: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a self-contained, explicit 3TZ archive before publishing
    Validate {
        /// .3tz archive to check (raster/terrain directories are not validated yet)
        input: PathBuf,
        /// Run a locally installed official 3d-tiles-validator executable
        #[arg(long)]
        external_validator: Option<PathBuf>,
    },
    /// Check converter capabilities and local CRS resources
    Doctor(DoctorArgs),
    /// Serve selected output directories with an installed Cesium IIFE runtime
    Preview(PreviewArgs),
    #[command(hide = true)]
    EncodeVectorContent {
        #[arg(short, long)]
        input: PathBuf,
    },
    /// GLB/glTF file or directory → tileset.json (3d-tiles-tools createTilesetJson)
    #[command(name = "createTilesetJson", visible_alias = "create-tileset-json")]
    CreateTilesetJson(TilesetArgs),
    /// Tileset directory or tileset.json → .3tz (3d-tiles-tools convert)
    Convert(IoArgs),
    /// Rewrite an eligible rusty-tiles explicit point/vector .3tz as implicit tiling
    ConvertToImplicit(IoArgs),
    /// GLB/glTF → .3tz (createTilesetJson + convert)
    #[command(name = "glb-to-3tz", visible_alias = "glbTo3tz")]
    GlbTo3tz(TilesetArgs),
    /// GLB/glTF → spatially split .3tz (split only when over leaf budget)
    #[command(name = "mesh-to-3tz", visible_alias = "meshTo3tz")]
    MeshTo3tz(MeshArgs),
    /// GeoJSON/GeoPackage → glTF .3tz; other OGR inputs require native-geospatial
    Vector(VectorArgs),
    /// LAS/LAZ → point-cloud 3D Tiles with native disk-backed spatial LOD
    PointCloud(PointCloudArgs),
    /// DEM → native quantized-mesh directory (requires native-geospatial)
    Terrain(TerrainArgs),
    /// GeoTIFF imagery → lossless COG and PNG XYZ pyramid (requires GDAL)
    Raster(RasterArgs),
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Validate { .. } => "validate",
            Self::Doctor(_) => "doctor",
            Self::Preview(_) => "preview",
            Self::EncodeVectorContent { .. } => "encode-vector-content",
            Self::CreateTilesetJson(_) => "createTilesetJson",
            Self::Convert(_) => "convert",
            Self::ConvertToImplicit(_) => "convert-to-implicit",
            Self::GlbTo3tz(_) => "glb-to-3tz",
            Self::MeshTo3tz(_) => "mesh-to-3tz",
            Self::Vector(_) => "vector",
            Self::PointCloud(_) => "point-cloud",
            Self::Terrain(_) => "terrain",
            Self::Raster(_) => "raster",
        }
    }
}

/// Input, output and replacement flags shared by every converter; flattened
/// first so they lead each subcommand's help.
#[derive(Args)]
struct IoArgs {
    /// Source file or directory
    #[arg(short = 'i', long)]
    input: PathBuf,
    /// Output path
    #[arg(short = 'o', long)]
    output: PathBuf,
    /// Replace an existing output after successful conversion
    #[arg(short = 'f', long)]
    force: bool,
}

/// 3d-tiles-tools placement of the generated root tile.
#[derive(Args)]
struct PlacementArgs {
    /// lon lat [height_m] — 3d-tiles-tools --cartographicPositionDegrees
    #[arg(
        long = "cartographicPositionDegrees",
        visible_alias = "cartographic-position-degrees",
        num_args = 2..=3,
        allow_hyphen_values = true
    )]
    cartographic_position_degrees: Vec<f64>,
    /// heading pitch roll (degrees) — 3d-tiles-tools --rotationDegrees
    #[arg(
        long = "rotationDegrees",
        visible_alias = "rotation-degrees",
        num_args = 3,
        allow_hyphen_values = true
    )]
    rotation_degrees: Vec<f64>,
}

#[derive(Args)]
struct TilesetArgs {
    #[command(flatten)]
    io: IoArgs,
    #[command(flatten)]
    placement: PlacementArgs,
}

fn doctor_commands() -> PossibleValuesParser {
    PossibleValuesParser::new(
        doctor::COMMANDS
            .iter()
            .map(|(name, aliases)| PossibleValue::new(*name).aliases(aliases.iter().copied())),
    )
}

#[derive(Args)]
struct DoctorArgs {
    /// Check only these converters; repeat to select several
    #[arg(long = "command", value_parser = doctor_commands())]
    commands: Vec<String>,
    /// Cesium Build/Cesium directory to look for (informational preview check;
    /// defaults to the README install location)
    #[arg(long)]
    cesium: Option<PathBuf>,
}

#[derive(Args)]
struct PreviewArgs {
    /// Cesium 1.146.0 Build/Cesium directory containing Cesium.js (IIFE)
    #[arg(long)]
    cesium: PathBuf,
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 9227)]
    port: u16,
    #[arg(long)]
    point_cloud: Option<PathBuf>,
    #[arg(long)]
    mesh: Option<PathBuf>,
    #[arg(long)]
    annotations: Option<PathBuf>,
    #[arg(long)]
    imagery: Option<PathBuf>,
    #[arg(long)]
    terrain: Option<PathBuf>,
}

#[derive(Args)]
struct TerrainArgs {
    #[command(flatten)]
    io: IoArgs,
    /// Finest zoom level (0..24)
    #[arg(long = "maxZoom", visible_alias = "max-zoom")]
    max_zoom: u8,
    /// Samples per tile edge: 17, 33, 65 or 129
    #[arg(long, default_value_t = 65)]
    grid: u16,
    /// Add to DEM metre heights to obtain ellipsoidal heights; never inferred
    #[arg(
        long = "heightOffset",
        visible_alias = "height-offset",
        allow_hyphen_values = true
    )]
    height_offset: f64,
    /// Ellipsoidal height used outside coverage and for NoData
    #[arg(
        long = "fillHeight",
        visible_alias = "fill-height",
        allow_hyphen_values = true
    )]
    fill_height: f64,
    /// Maximum added terrain simplification error in metres; 0 keeps the full grid
    #[arg(
        long = "maxError",
        visible_alias = "max-error",
        default_value_t = 1.,
        allow_hyphen_values = true
    )]
    max_error: f64,
}

#[derive(Args)]
struct RasterArgs {
    #[command(flatten)]
    io: IoArgs,
    /// Coarsest zoom level (0..maxZoom)
    #[arg(long = "minZoom", visible_alias = "min-zoom", default_value_t = 0)]
    min_zoom: u8,
    /// Finest zoom level (0..24)
    #[arg(long = "maxZoom", visible_alias = "max-zoom")]
    max_zoom: u8,
    /// image (RGB/RGBA bands) or gray (one band stretched by displayMin/displayMax)
    #[arg(long, default_value = "image")]
    display: String,
    #[arg(long, default_value_t = 1)]
    band: u16,
    /// Alpha band for image or gray display (0..255 opacity); 0 uses mask/NoData
    #[arg(long = "alphaBand", visible_alias = "alpha-band", default_value_t = 0)]
    alpha_band: u16,
    /// Value mapped to black for gray display
    #[arg(
        long = "displayMin",
        visible_alias = "display-min",
        allow_hyphen_values = true
    )]
    display_min: Option<f64>,
    /// Value mapped to white for gray display
    #[arg(
        long = "displayMax",
        visible_alias = "display-max",
        allow_hyphen_values = true
    )]
    display_max: Option<f64>,
}

#[derive(Args)]
struct PointCloudArgs {
    #[command(flatten)]
    io: IoArgs,
    /// Keep the legacy explicit tileset hierarchy.
    #[arg(long)]
    explicit: bool,
    /// Expose classification, intensity and return number as vertex property attributes
    #[arg(long = "metadataAttributes", visible_alias = "metadata-attributes")]
    metadata_attributes: bool,
    /// local XYZ metres, header CRS, or explicit 2D horizontal CRS (e.g. EPSG:32632)
    #[arg(long = "sourceCrs", visible_alias = "source-crs")]
    source_crs: String,
    /// Metre offset to ellipsoidal height; required for geospatial input
    #[arg(
        long = "heightOffset",
        visible_alias = "height-offset",
        allow_hyphen_values = true
    )]
    height_offset: Option<f64>,
    /// Maximum points per tile
    #[arg(
        long = "maxPoints",
        visible_alias = "max-points",
        default_value_t = 50000
    )]
    max_points: usize,
    /// Points read per source chunk
    #[arg(
        long = "chunkPoints",
        visible_alias = "chunk-points",
        default_value_t = 100000
    )]
    chunk_points: usize,
}

#[derive(Args)]
struct VectorArgs {
    #[command(flatten)]
    io: IoArgs,
    /// Keep the legacy explicit tileset hierarchy.
    #[arg(long)]
    explicit: bool,
    /// Omit volatile performance diagnostics for byte-identical archives
    #[arg(long)]
    reproducible: bool,
    /// Maximum encoding worker threads (defaults to available cores)
    #[arg(long, default_value_t = std::thread::available_parallelism().map_or(1, usize::from))]
    jobs: usize,
    /// Attribute filter applied to every selected layer (SQLite in the portable build)
    #[arg(long = "where")]
    where_clause: Option<String>,
    /// Encode list-valued properties as JSON strings, or reject them
    #[arg(
        long = "listFields",
        visible_alias = "list-fields",
        default_value = "error",
        value_parser = ["error", "json"]
    )]
    list_fields: String,
    /// Include only these source fields (comma-separated)
    #[arg(long, value_delimiter = ',', conflicts_with = "drop_fields")]
    fields: Vec<String>,
    /// Exclude these source fields (comma-separated)
    #[arg(
        long = "dropFields",
        visible_alias = "drop-fields",
        value_delimiter = ',',
        conflicts_with = "fields"
    )]
    drop_fields: Vec<String>,
    /// Skip unconvertible features and report every omitted source identity
    #[arg(long = "skipInvalid", visible_alias = "skip-invalid")]
    skip_invalid: bool,
    /// Reuse unchanged subtrees from a compatible prior vector archive
    #[arg(long = "reuseTileset", visible_alias = "reuse-tileset")]
    reuse_tileset: Option<PathBuf>,
    /// Select a spatial layer; repeat to include several layers
    #[arg(long = "layer")]
    layers: Vec<String>,
    /// Include every spatial layer (otherwise multi-layer inputs require selection)
    #[arg(
        long = "allLayers",
        visible_alias = "all-layers",
        conflicts_with = "layers"
    )]
    all_layers: bool,
    /// Override input CRS, or use local for metre XYZ
    #[arg(long = "sourceCrs", visible_alias = "source-crs")]
    source_crs: Option<String>,
    /// Explicit additive offset from source heights to ellipsoidal metres
    #[arg(
        long = "heightOffset",
        visible_alias = "height-offset",
        allow_hyphen_values = true
    )]
    height_offset: Option<f64>,
    /// Maximum encoded POSITION vertices per content tile (at least 4)
    #[arg(
        long = "maxVertices",
        visible_alias = "max-vertices",
        default_value_t = 65536
    )]
    max_vertices: usize,
    /// Maximum encoded GLB bytes per content tile (at least 4096)
    #[arg(
        long = "maxBytes",
        visible_alias = "max-bytes",
        default_value_t = 4194304
    )]
    max_bytes: usize,
    /// Maximum tiles in the hierarchy
    #[arg(
        long = "maxTiles",
        visible_alias = "max-tiles",
        default_value_t = 100000
    )]
    max_tiles: usize,
    /// Maximum coordinates in one source feature, bounding reader memory
    #[arg(
        long = "maxSourceVertices",
        visible_alias = "max-source-vertices",
        default_value_t = 1000000
    )]
    max_source_vertices: usize,
    /// Base simplification tolerance in metres; doubles for each coarse level
    #[arg(
        long = "lodTolerance",
        visible_alias = "lod-tolerance",
        default_value_t = 0.1
    )]
    lod_tolerance: f64,
    /// Coarse levels above each full-detail leaf (1..16)
    #[arg(long = "lodLevels", visible_alias = "lod-levels", default_value_t = 3)]
    lod_levels: u8,
    /// Repair invalid polygon outlines with an explicit per-feature conversion report
    #[arg(long)]
    repair: bool,
    /// Preserve geometrically ambiguous filled polygons as their source 3D outlines
    #[arg(long = "ambiguousOutlines", visible_alias = "ambiguous-outlines")]
    ambiguous_outlines: bool,
    /// Quantize vector positions to normalized 16-bit integers (lossy, reported)
    #[arg(long)]
    quantize: bool,
    /// Losslessly compress vector accessor buffers with EXT_meshopt_compression
    #[arg(long)]
    meshopt: bool,
    /// Allow explicitly reported outline stand-ins for unsimplifiable parent polygons
    #[arg(long = "parentRepair", visible_alias = "parent-repair")]
    parent_repair: bool,
    /// Aggregate point-only parents into per-layer voxel counts; keep original leaves
    #[arg(long = "aggregatePoints", visible_alias = "aggregate-points")]
    aggregate_points: bool,
    /// Maximum feature fragments in parent content; leaves use maxFeatures
    #[arg(
        long = "maxParentFeatures",
        visible_alias = "max-parent-features",
        default_value_t = 4096
    )]
    max_parent_features: usize,
    /// Maximum feature fragments in leaf content
    #[arg(
        long = "maxFeatures",
        visible_alias = "max-features",
        default_value_t = 64
    )]
    max_features: usize,
}

#[derive(Args)]
struct MeshArgs {
    #[command(flatten)]
    io: IoArgs,
    /// Keep the legacy explicit tileset hierarchy and median partitioning.
    #[arg(long)]
    explicit: bool,
    #[command(flatten)]
    placement: PlacementArgs,
    /// Texture output: fast full-chroma JPEG, smaller WebP, GPU UASTC, or exact PNG.
    #[arg(
        long = "textureFormat",
        visible_alias = "texture-format",
        value_enum,
        default_value = "jpeg"
    )]
    texture_format: rusty_tiles::tile::TextureFormat,
    /// Basis Universal encoder executable (required for UASTC output).
    #[arg(long = "basisu", default_value = "basisu")]
    basisu: PathBuf,
    /// Stop splitting a node at this many triangles (default 20000).
    #[arg(
        long = "maxTriangles",
        visible_alias = "max-triangles",
        default_value_t = rusty_tiles::DEFAULT_MAX_TRIANGLES
    )]
    max_triangles: usize,
    /// Source-file wrap threshold in bytes (default 204800). Split stop is
    /// `--maxTriangles`; written tiles are meshopt-compressed unless `--noMeshopt`.
    #[arg(
        long = "maxBytes",
        visible_alias = "max-bytes",
        default_value_t = rusty_tiles::DEFAULT_MAX_BYTES
    )]
    max_bytes: u64,
    /// Leaf atlas edge (default 2048). Oversized individual triangles retain
    /// their texels in larger atlases. Parent atlases are capped at 1024.
    #[arg(
        long = "tileSize",
        visible_alias = "tile-size",
        default_value_t = rusty_tiles::DEFAULT_TILE_SIZE
    )]
    tile_size: u32,
    /// Must be 0: leaf texels are preserved at source resolution.
    #[arg(
        long = "maxTexelDensity",
        visible_alias = "max-texel-density",
        default_value_t = rusty_tiles::DEFAULT_MAX_TEXEL_DENSITY
    )]
    max_texel_density: f64,
    /// Source CRS: auto/legacy adapters, or an EPSG, WKT or PROJ horizontal CRS.
    #[arg(
        long = "sourceCrs",
        visible_alias = "source-crs",
        default_value = "auto"
    )]
    source_crs: String,
    /// General CRS axes after node transforms: xyz (E,N,height), y-up (E,height,-N).
    #[arg(long = "sourceAxes", visible_alias = "source-axes", value_enum)]
    source_axes: Option<SourceAxes>,
    /// Metres added to source height plus A to give ellipsoidal height; required for general CRS.
    #[arg(
        long = "heightOffset",
        visible_alias = "height-offset",
        allow_hyphen_values = true
    )]
    height_offset: Option<f64>,
    /// Source shift E N [A]: E/N in horizontal CRS units, A metres; added in f64.
    #[arg(
        long = "sourceOffset",
        visible_alias = "source-offset",
        num_args = 2..=3,
        allow_hyphen_values = true
    )]
    source_offset: Vec<f64>,
    /// Offset file: E/N in source horizontal units and optional A in metres.
    #[arg(long = "sourceOffsetFile", visible_alias = "source-offset-file")]
    source_offset_file: Option<PathBuf>,
    /// Disable lossless meshopt compression. Both modes retain float32 geometry.
    #[arg(long = "noMeshopt", visible_alias = "no-meshopt")]
    no_meshopt: bool,
    /// Preserve source glTF nodes as pickable features with a name property
    #[arg(long = "nodeFeatures", visible_alias = "node-features")]
    node_features: bool,
}

/// What a successful command produced.
enum Outcome {
    /// A command with its own single machine result (validate, doctor).
    Report(Value),
    /// A published conversion.
    Converted(ConversionResult),
    /// Installed package with a typed receipt separate from source reports.
    Pack(PackageResult),
    /// A plain output file without a conversion report (createTilesetJson).
    Wrote(PathBuf),
    Done,
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            if code != 0 && std::env::args().any(|arg| arg == "--json") {
                println!(
                    "{}",
                    json!({"ok":false,"error":{"code":"usage","message":error.to_string()},"exitCode":code})
                );
            } else {
                let _ = error.print();
            }
            return ExitCode::from(code as u8);
        }
    };
    // The internal compression worker has a separate JSON protocol.
    let internal = matches!(cli.command, Command::EncodeVectorContent { .. });
    let json = cli.json && !internal;
    let events = cli.progress.is_some() && !internal;
    let name = cli.command.name();
    // NDJSON progress mode keeps stderr machine-readable.
    let reporter = if events {
        Reporter::ndjson_stderr()
    } else {
        Reporter::human_stderr()
    };
    reporter.progress("conversion", 0, 1);
    match run(cli, &reporter) {
        Ok(Outcome::Report(report)) if report["ok"] == false => {
            println!("{report}");
            ExitCode::from(4)
        }
        Ok(outcome) => {
            reporter.progress("conversion", 1, 1);
            let summary = match outcome {
                Outcome::Report(report) => {
                    if json {
                        println!("{report}");
                    }
                    None
                }
                Outcome::Converted(result) => Some((
                    output_summary(&result.output, result.report.as_ref(), result.archive),
                    result.output,
                )),
                Outcome::Pack(result) => Some((package_summary(&result), result.output)),
                Outcome::Wrote(output) => Some((output_summary(&output, None, false), output)),
                Outcome::Done => None,
            };
            if let Some((summary, output)) = summary {
                if json {
                    println!("{summary}");
                } else if !events {
                    for line in human_summary(name, &output, &summary) {
                        eprintln!("{line}");
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            let (category, code) = match &error {
                Error::Job(failure) => job_category(failure.error.kind()),
                _ => error.category(),
            };
            if json {
                let mut failure = json!({"ok":false,"error":{"code":category,"message":error.to_string()},"exitCode":code});
                if let Error::Job(job) = &error {
                    failure["error"]["kind"] = json!(category);
                    failure["error"]["secondaryDiagnostics"] = json!(job
                        .secondary
                        .iter()
                        .map(|cause| cause.to_string())
                        .collect::<Vec<_>>());
                    failure["error"]["retainedPaths"] = json!(job
                        .retained_paths
                        .iter()
                        .map(|path| path.to_string_lossy())
                        .collect::<Vec<_>>());
                }
                println!("{failure}");
            } else {
                eprintln!("{error}");
            }
            if events {
                eprintln!(
                    "{}",
                    json!({"event":"failed","phase":"conversion","code":category})
                );
            }
            ExitCode::from(code)
        }
    }
}

/// Transport categories and process statuses belong to this CLI adapter.
fn job_category(kind: JobErrorKind) -> (&'static str, u8) {
    match kind {
        JobErrorKind::InvalidRequest => ("invalid_request", 2),
        JobErrorKind::InvalidInput => ("invalid_input", 3),
        JobErrorKind::Unsupported => ("unsupported", 2),
        JobErrorKind::Io => ("io", 1),
        JobErrorKind::Conflict => ("output_conflict", 5),
        JobErrorKind::Cancelled => ("cancelled", 1),
        JobErrorKind::ObserverFailure => ("observer_failure", 1),
        JobErrorKind::InvalidState => ("invalid_state", 1),
    }
}

struct CliPackageObserver;

impl Observer for CliPackageObserver {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        let value = match event {
            RunEvent::Progress { phase, done, total } => {
                json!({"event":"progress","phase":phase,"done":done,"total":total})
            }
            RunEvent::Warning { code, message } => {
                json!({"event":"warning","code":code,"message":message})
            }
            RunEvent::Note { message } => json!({"event":"log","message":message}),
        };
        writeln!(io::stderr().lock(), "{value}")
            .map_err(|error| JobError::new(JobErrorKind::ObserverFailure, error.to_string()))
    }
}

fn package_summary(result: &PackageResult) -> Value {
    let mut summary = output_summary(&result.output, None, true);
    summary["packageReceipt"] = json!({
        "memberCount": result.receipt.member_count,
        "sourceBytes": result.receipt.source_bytes,
        "archiveBytes": result.receipt.archive_bytes,
    });
    summary["cleanupDiagnostics"] = json!(result.cleanup_diagnostics.iter().map(|diagnostic| {
        json!({"path":diagnostic.path.to_string_lossy(),"kind":job_category(diagnostic.error.kind()).0,"message":diagnostic.error.to_string()})
    }).collect::<Vec<_>>());
    summary
}

/// Top-level numeric conversion.json fields that count produced or skipped
/// things. Every other numeric field is a setting or a derived measurement and
/// is reported under `settings`.
const COUNT_KEYS: &[&str] = &[
    // point-cloud, terrain and vector
    "points",
    "tiles",
    // vector
    "features",
    "fragments",
    "leafTiles",
    "routingTiles",
    "maximumTileVertices",
    "maximumTileBytes",
    "fragmentedPolygons",
    "skippedFeatures",
    "featuresWithoutGeometry",
    "geometryReportCount",
    "lockedSharedVertices",
    // raster
    "sourceBands",
];

/// Machine summary of a successful conversion, built from the report the
/// converter published (no output is reread).
fn output_summary(output: &Path, report: Option<&Value>, archive: bool) -> Value {
    let location = match report {
        None => Value::Null,
        Some(_) if archive => json!({"archive":output.to_string_lossy(),"entry":"conversion.json"}),
        Some(_) => json!({"path":output.join("conversion.json").to_string_lossy()}),
    };
    let report = report.unwrap_or(&Value::Null);
    let mut counts = serde_json::Map::new();
    let mut settings = serde_json::Map::new();
    for (key, value) in report.as_object().into_iter().flatten() {
        if value.is_number() {
            let target = if COUNT_KEYS.contains(&key.as_str()) {
                &mut counts
            } else {
                &mut settings
            };
            target.insert(key.clone(), value.clone());
        }
    }
    json!({"ok":true,"output":output.to_string_lossy(),"counts":counts,"settings":settings,
        "skippedFeatures":report["skippedFeatures"],"reuse":report["reuse"],"conversionReport":location})
}

/// Quote a path for a copy-pasteable shell suggestion when needed.
fn shell_path(path: &Path) -> String {
    let text = path.display().to_string();
    if !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+:=@,".contains(c))
    {
        text
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

fn plural(count: u64, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

/// One to three stderr lines after a successful conversion: what was written,
/// anything omitted or reported, and the command to run next.
fn human_summary(command: &str, output: &Path, summary: &Value) -> Vec<String> {
    let counts = &summary["counts"];
    let mut parts: Vec<String> = [
        ("features", "feature"),
        ("points", "point"),
        ("tiles", "tile"),
    ]
    .into_iter()
    .filter_map(|(key, noun)| counts[key].as_u64().map(|n| plural(n, noun)))
    .collect();
    if parts.is_empty() && output.is_file() {
        if let Ok(metadata) = output.metadata() {
            parts.push(format!("{:.1} MiB", metadata.len() as f64 / 1048576.));
        }
    }
    let detail = if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    };
    let mut lines = vec![format!("{command}: wrote {}{detail}", output.display())];
    let count = |key: &str| counts[key].as_u64().filter(|n| *n > 0);
    let mut warnings = Vec::new();
    if let Some(n) = count("skippedFeatures") {
        warnings.push(plural(n, "skipped feature"));
    }
    if let Some(n) = count("featuresWithoutGeometry") {
        warnings.push(format!("{} without geometry", plural(n, "feature")));
    }
    let reports = count("geometryReportCount").map(|n| plural(n, "geometry report"));
    match (warnings.is_empty(), reports) {
        (true, None) => {}
        (true, Some(reports)) => {
            lines.push(format!("reported: {reports} in geometry-reports.jsonl"))
        }
        (false, reports) => {
            warnings.extend(reports);
            lines.push(format!(
                "warnings: {}; see conversion.json and geometry-reports.jsonl",
                warnings.join(", ")
            ));
        }
    }
    for diagnostic in summary["cleanupDiagnostics"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let (Some(path), Some(message)) =
            (diagnostic["path"].as_str(), diagnostic["message"].as_str())
        {
            lines.push(format!(
                "cleanup warning: {message}; retained work at {path}"
            ));
        }
    }
    let path = shell_path(output);
    let next = match command {
        "raster" => format!("rusty-tiles preview --cesium <Build/Cesium> --imagery {path}"),
        "terrain" => format!("rusty-tiles preview --cesium <Build/Cesium> --terrain {path}"),
        "createTilesetJson" => format!("rusty-tiles convert -i {path} -o <archive>.3tz"),
        _ => format!("rusty-tiles validate {path}"),
    };
    lines.push(format!("next: {next}"));
    lines
}

fn run(cli: Cli, reporter: &Reporter) -> Result<Outcome, Error> {
    let pack_events = cli.progress.is_some();
    let json = cli.json;
    Ok(match cli.command {
        Command::Validate {
            input,
            external_validator,
        } => {
            let report = rusty_tiles::validate::archive(&input, external_validator.as_deref())?;
            if !json {
                println!(
                    "Validated {}: {} tiles, {} content references",
                    input.display(),
                    report["tiles"],
                    report["contentReferences"]
                );
            }
            Outcome::Report(report)
        }
        Command::Doctor(a) => {
            let report = doctor::report(&a.commands, a.cesium.as_deref())?;
            if json {
                let mut report = report;
                report["ok"] = report["ready"].clone();
                if report["ready"] != true {
                    // Include the inventory in the single error result.
                    report["error"] = json!({"code":"environment","message":"selected converters have missing dependencies"});
                    report["exitCode"] = 4.into();
                }
                return Ok(Outcome::Report(report));
            }
            doctor::display(&report, false);
            if report["ready"] != true {
                return Err(Error::Environment(
                    "selected converters have missing dependencies; see doctor report".into(),
                ));
            }
            Outcome::Done
        }
        Command::Preview(a) => {
            let layers: Vec<_> = [
                ("point-cloud", a.point_cloud),
                ("mesh", a.mesh),
                ("annotations", a.annotations),
                ("imagery", a.imagery),
                ("terrain", a.terrain),
            ]
            .into_iter()
            .filter_map(|(name, path)| path.map(|path| (name.to_owned(), path)))
            .collect();
            rusty_tiles::preview::Preview::new(&a.cesium, &layers)?.serve(&a.host, a.port, json)?;
            Outcome::Done
        }
        Command::EncodeVectorContent { input } => {
            println!("{}", rusty_tiles::vector_encoding::compress_file(&input)?);
            Outcome::Done
        }
        Command::CreateTilesetJson(a) => {
            let opts = tileset_opts(&a.io, &a.placement)?;
            create_tileset_json(&a.io.input, &a.io.output, &opts)?;
            Outcome::Wrote(a.io.output)
        }
        Command::PointCloud(a) => {
            Outcome::Converted(rusty_tiles::point_cloud::point_cloud_to_3tz_reported(
                &a.io.input,
                &a.io.output,
                &rusty_tiles::point_cloud::PointCloudOptions {
                    explicit: a.explicit,
                    metadata_attributes: a.metadata_attributes,
                    force: a.io.force,
                    source_crs: a.source_crs,
                    height_offset: a.height_offset,
                    max_points: a.max_points,
                    chunk_points: a.chunk_points,
                },
                reporter,
            )?)
        }
        Command::Convert(a) => {
            let observer: Option<Arc<dyn Observer>> =
                pack_events.then(|| Arc::new(CliPackageObserver) as Arc<dyn Observer>);
            let run = RunControl::new(observer);
            let policy = if a.force {
                OutputPolicy::Replace
            } else {
                OutputPolicy::CreateNew
            };
            let request = PackageRequest::directory(a.input, a.output).with_policy(policy);
            Outcome::Pack(package(request, &run)?)
        }
        Command::ConvertToImplicit(a) => {
            Outcome::Converted(rusty_tiles::convert_to_implicit_reported(
                &a.input,
                &a.output,
                &rusty_tiles::ConvertToImplicitOptions { force: a.force },
                reporter,
            )?)
        }
        Command::GlbTo3tz(a) => {
            let opts = tileset_opts(&a.io, &a.placement)?;
            Outcome::Converted(glb_to_3tz_reported(&a.io.input, &a.io.output, &opts)?)
        }
        Command::MeshTo3tz(a) => {
            let opts = mesh_opts(&a)?;
            Outcome::Converted(mesh_to_3tz_reported(
                &a.io.input,
                &a.io.output,
                &opts,
                reporter,
            )?)
        }
        Command::Vector(a) => Outcome::Converted(vector::vector_to_3tz_reported(
            &a.io.input,
            &a.io.output,
            a.max_features,
            a.repair,
            a.ambiguous_outlines,
            &vector::VectorOptions {
                explicit: a.explicit,
                reproducible: a.reproducible,
                jobs: a.jobs,
                quantize: a.quantize,
                meshopt: a.meshopt,
                meshopt_encoder: None,
                parent_repair: a.parent_repair,
                aggregate_points: a.aggregate_points,
                max_parent_features: a.max_parent_features,
                where_clause: a.where_clause,
                force: a.io.force,
                list_fields: a.list_fields,
                fields: a.fields,
                drop_fields: a.drop_fields,
                skip_invalid: a.skip_invalid,
                reuse_tileset: a.reuse_tileset,
                lod: vector::VectorLodOptions {
                    tolerance_metres: a.lod_tolerance,
                    levels: a.lod_levels,
                },
                layers: a.layers,
                all_layers: a.all_layers,
                source_crs: a.source_crs,
                height_offset: a.height_offset,
                max_vertices: a.max_vertices,
                max_bytes: a.max_bytes,
                max_tiles: a.max_tiles,
                max_source_vertices: a.max_source_vertices,
            },
            reporter,
        )?),
        Command::Terrain(a) => Outcome::Converted(terrain::dem_to_terrain_reported(
            &a.io.input,
            &a.io.output,
            &terrain::TerrainOptions {
                force: a.io.force,
                max_zoom: a.max_zoom,
                grid: a.grid,
                height_offset: a.height_offset,
                fill_height: a.fill_height,
                max_error: a.max_error,
            },
            reporter,
        )?),
        Command::Raster(a) => Outcome::Converted(rusty_tiles::raster::raster_reported(
            &a.io.input,
            &a.io.output,
            &rusty_tiles::raster::RasterOptions {
                force: a.io.force,
                min_zoom: a.min_zoom,
                max_zoom: a.max_zoom,
                display: a.display,
                band: a.band,
                alpha_band: a.alpha_band,
                display_min: a.display_min,
                display_max: a.display_max,
            },
            reporter,
        )?),
    })
}

fn tileset_opts(io: &IoArgs, a: &PlacementArgs) -> Result<CreateTilesetOptions, Error> {
    let cartographic = if a.cartographic_position_degrees.is_empty() {
        None
    } else {
        let v = &a.cartographic_position_degrees;
        Some(Cartographic::new(
            v[0],
            v[1],
            v.get(2).copied().unwrap_or(0.0),
        ))
    };
    let rotation = if a.rotation_degrees.is_empty() {
        None
    } else {
        Some(RotationDegrees {
            heading: a.rotation_degrees[0],
            pitch: a.rotation_degrees[1],
            roll: a.rotation_degrees[2],
        })
    };
    if rotation.is_some() && cartographic.is_none() {
        return Err(Error::msg(
            "--rotationDegrees requires --cartographicPositionDegrees",
        ));
    }
    Ok(CreateTilesetOptions {
        cartographic,
        rotation,
        force: io.force,
    })
}

fn mesh_opts(a: &MeshArgs) -> Result<MeshTo3tzOptions, Error> {
    let ts = tileset_opts(&a.io, &a.placement)?;
    if !a.source_offset.is_empty() && a.source_offset_file.is_some() {
        return Err(Error::msg(
            "pass only one of --sourceOffset and --sourceOffsetFile",
        ));
    }
    let source_offset = if !a.source_offset.is_empty() {
        let v = &a.source_offset;
        Some(SourceOffset {
            easting: v[0],
            northing: v[1],
            height: v.get(2).copied().unwrap_or(0.0),
        })
    } else if let Some(path) = &a.source_offset_file {
        let text = std::fs::read_to_string(path)?;
        Some(parse_metashape_offset(&text)?)
    } else {
        None
    };
    let mut options = MeshTo3tzOptions {
        explicit: a.explicit,
        texture_format: a.texture_format,
        basisu: a.basisu.clone(),
        cartographic: ts.cartographic,
        rotation: ts.rotation,
        force: a.io.force,
        max_triangles: a.max_triangles,
        max_bytes: a.max_bytes,
        tile_size: a.tile_size,
        max_texel_density: a.max_texel_density,
        source_offset,
        source_axes: a.source_axes,
        height_offset: a.height_offset,
        meshopt: !a.no_meshopt,
        node_features: a.node_features,
        ..MeshTo3tzOptions::default()
    };
    options.set_source_crs(&a.source_crs)?;
    if source_offset.is_some() && options.source_crs == SourceCrs::Geographic {
        return Err(Error::msg(
            "--sourceOffset requires --sourceCrs auto or epsg:3857",
        ));
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn kebab(name: &str) -> String {
        name.chars()
            .flat_map(|c| {
                let lower = c.to_ascii_lowercase();
                (c.is_ascii_uppercase())
                    .then_some('-')
                    .into_iter()
                    .chain([lower])
            })
            .collect()
    }

    #[test]
    fn camel_case_options_and_subcommands_have_visible_kebab_aliases() {
        let cli = Cli::command();
        for command in cli.get_subcommands().filter(|c| !c.is_hide_set()) {
            let name = command.get_name();
            if name.chars().any(|c| c.is_ascii_uppercase()) {
                let aliases: Vec<_> = command.get_visible_aliases().collect();
                assert!(aliases.contains(&kebab(name).as_str()), "{name}");
            }
            for arg in command.get_arguments() {
                let Some(long) = arg.get_long() else { continue };
                if long.chars().any(|c| c.is_ascii_uppercase()) {
                    let aliases = arg.get_visible_aliases().unwrap_or_default();
                    assert!(aliases.contains(&kebab(long).as_str()), "{name} --{long}");
                }
            }
            if command.get_arguments().any(|arg| arg.get_id() == "output") {
                let first: Vec<_> = command
                    .get_arguments()
                    .filter(|arg| !arg.is_global_set() && arg.get_long().is_some())
                    .take(3)
                    .map(|arg| arg.get_id().as_str().to_owned())
                    .collect();
                assert_eq!(first, ["input", "output", "force"], "{name}");
            }
        }
    }

    #[test]
    fn doctor_commands_match_cli_subcommands_and_aliases() {
        let cli = Cli::command();
        let mut seen = Vec::new();
        for command in cli.get_subcommands().filter(|c| !c.is_hide_set()) {
            let name = command.get_name();
            if name == "doctor" {
                continue;
            }
            let (_, aliases) = doctor::COMMANDS
                .iter()
                .find(|(canonical, _)| *canonical == name)
                .unwrap_or_else(|| panic!("doctor::COMMANDS lacks {name}"));
            let mut expected: Vec<_> = command.get_all_aliases().collect();
            let mut actual = aliases.to_vec();
            expected.sort_unstable();
            actual.sort_unstable();
            assert_eq!(actual, expected, "{name}");
            for alias in aliases.iter() {
                assert_eq!(doctor::canonical(alias), Some(name));
            }
            seen.push(name);
        }
        assert_eq!(seen.len(), doctor::COMMANDS.len());
        let parsed = Cli::try_parse_from([
            "rusty-tiles",
            "doctor",
            "--command",
            "meshTo3tz",
            "--command",
            "create-tileset-json",
        ])
        .unwrap();
        let Command::Doctor(args) = parsed.command else {
            panic!("doctor")
        };
        assert_eq!(args.commands, ["meshTo3tz", "create-tileset-json"]);
    }

    #[test]
    fn kebab_aliases_parse_like_camel_case() {
        let parse = |flag: &str| {
            let cli = Cli::try_parse_from([
                "rusty-tiles",
                "point-cloud",
                "-i",
                "in.las",
                "-o",
                "out.3tz",
                flag,
                "local",
                "--max-points",
                "7",
            ])
            .unwrap();
            let Command::PointCloud(args) = cli.command else {
                panic!("point-cloud")
            };
            (args.source_crs, args.max_points)
        };
        assert_eq!(parse("--sourceCrs"), parse("--source-crs"));
    }

    #[test]
    fn summary_separates_counts_from_settings() {
        let root = tempfile::tempdir().unwrap();
        let report = json!({"tiles":5,"heightOffset":2.5,"fillHeight":0,"grid":65,"heightQuantizationStep":0.1,
               "lodLevels":3,"lodToleranceMetres":0.1,"skippedFeatures":2,"features":9,"sourceCrs":"x"});
        let summary = output_summary(root.path(), Some(&report), false);
        assert_eq!(
            summary["conversionReport"],
            json!({"path":root.path().join("conversion.json")})
        );
        assert_eq!(
            summary["counts"],
            json!({"tiles":5,"skippedFeatures":2,"features":9})
        );
        assert_eq!(
            summary["settings"],
            json!({"heightOffset":2.5,"fillHeight":0,"grid":65,"heightQuantizationStep":0.1,
                "lodLevels":3,"lodToleranceMetres":0.1})
        );
        assert_eq!(summary["skippedFeatures"], 2);
        let lines = human_summary("vector", root.path(), &summary);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].contains("9 features, 5 tiles"), "{lines:?}");
        assert!(lines[1].contains("2 skipped features"), "{lines:?}");
        assert!(lines[2].starts_with("next: rusty-tiles validate "));
    }
}
