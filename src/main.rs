use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use rusty_tiles::error::Error;
use rusty_tiles::georef::{
    parse_metashape_offset, Cartographic, RotationDegrees, SourceCrs, SourceOffset,
};
use rusty_tiles::pack::{convert_to_3tz, PackOptions};
use rusty_tiles::tile::{mesh_to_3tz, MeshTo3tzOptions};
use rusty_tiles::tileset::{create_tileset_json, glb_to_3tz, CreateTilesetOptions};
use rusty_tiles::{terrain, vector};

#[derive(Parser)]
#[command(
    name = "rusty-tiles",
    about = "Transform geospatial sources into 3D Tiles (.3tz). createTilesetJson and convert match 3d-tiles-tools@0.5.4 argv.",
    version,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(hide = true)]
    EncodeVectorContent {
        #[arg(short, long)]
        input: PathBuf,
    },
    /// GLB/glTF file or directory → tileset.json (3d-tiles-tools createTilesetJson)
    #[command(name = "createTilesetJson", alias = "create-tileset-json")]
    CreateTilesetJson(IoArgs),
    /// Tileset directory or tileset.json → .3tz (3d-tiles-tools convert)
    Convert(ConvertArgs),
    /// GLB/glTF → .3tz (createTilesetJson + convert)
    #[command(name = "glb-to-3tz", alias = "glbTo3tz")]
    GlbTo3tz(IoArgs),
    /// GLB/glTF → spatially split .3tz (split only when over leaf budget)
    #[command(name = "mesh-to-3tz", alias = "meshTo3tz")]
    MeshTo3tz(MeshArgs),
    /// GeoJSON → glTF vector .3tz prototype (requires Python GDAL/GEOS and NumPy)
    Vector(VectorArgs),
    /// LAS/LAZ → point-cloud 3D Tiles with spatial LOD (Python laspy and pyproj)
    PointCloud(PointCloudArgs),
    /// DEM → quantized-mesh directory (requires Python GDAL and NumPy)
    Terrain(TerrainArgs),
    /// GeoTIFF imagery → lossless COG and PNG XYZ pyramid (requires GDAL)
    Raster(RasterArgs),
}

#[derive(Args)]
struct TerrainArgs {
    /// Replace an existing output after successful conversion
    #[arg(short = 'f', long)]
    force: bool,
    #[arg(short = 'i', long)]
    input: PathBuf,
    #[arg(short = 'o', long)]
    output: PathBuf,
    #[arg(long = "maxZoom")]
    max_zoom: u8,
    #[arg(long, default_value_t = 65)]
    grid: u16,
    /// Add to DEM metre heights to obtain ellipsoidal heights; never inferred
    #[arg(long = "heightOffset", allow_hyphen_values = true)]
    height_offset: f64,
    /// Ellipsoidal height used outside coverage and for NoData
    #[arg(long = "fillHeight", allow_hyphen_values = true)]
    fill_height: f64,
}

#[derive(Args)]
struct RasterArgs {
    /// Replace an existing output after successful conversion
    #[arg(short = 'f', long)]
    force: bool,
    #[arg(short = 'i', long)]
    input: PathBuf,
    #[arg(short = 'o', long)]
    output: PathBuf,
    #[arg(long = "minZoom", default_value_t = 0)]
    min_zoom: u8,
    #[arg(long = "maxZoom")]
    max_zoom: u8,
    #[arg(long, default_value = "image")]
    display: String,
    #[arg(long, default_value_t = 1)]
    band: u16,
    #[arg(long = "alphaBand", default_value_t = 0)]
    alpha_band: u16,
    #[arg(long = "displayMin", allow_hyphen_values = true)]
    display_min: Option<f64>,
    #[arg(long = "displayMax", allow_hyphen_values = true)]
    display_max: Option<f64>,
}

#[derive(Args)]
struct PointCloudArgs {
    /// Replace an existing output after successful conversion
    #[arg(short = 'f', long)]
    force: bool,
    #[arg(short = 'i', long)]
    input: PathBuf,
    #[arg(short = 'o', long)]
    output: PathBuf,
    /// local XYZ metres, header CRS, or explicit 2D horizontal CRS (e.g. EPSG:32632)
    #[arg(long = "sourceCrs")]
    source_crs: String,
    /// Metre offset to ellipsoidal height; required for geospatial input
    #[arg(long = "heightOffset", allow_hyphen_values = true)]
    height_offset: Option<f64>,
    #[arg(long = "maxPoints", default_value_t = 50000)]
    max_points: usize,
    #[arg(long = "chunkPoints", default_value_t = 100000)]
    chunk_points: usize,
}

#[derive(Args)]
struct VectorArgs {
    /// OGR attribute filter applied to every selected layer
    #[arg(long = "where")]
    where_clause: Option<String>,
    /// Replace an existing output after successful conversion
    #[arg(short = 'f', long)]
    force: bool,
    /// Encode list-valued properties as JSON strings, or reject them
    #[arg(long = "listFields", default_value = "error", value_parser = ["error", "json"])]
    list_fields: String,
    /// Include only these source fields (comma-separated)
    #[arg(long, value_delimiter = ',', conflicts_with = "drop_fields")]
    fields: Vec<String>,
    /// Exclude these source fields (comma-separated)
    #[arg(long = "dropFields", value_delimiter = ',', conflicts_with = "fields")]
    drop_fields: Vec<String>,
    /// Skip unconvertible features and report every omitted source identity
    #[arg(long = "skipInvalid")]
    skip_invalid: bool,
    /// Reuse unchanged subtrees from a compatible prior vector archive
    #[arg(long = "reuseTileset")]
    reuse_tileset: Option<PathBuf>,
    /// Select a spatial layer; repeat to include several layers
    #[arg(long = "layer")]
    layers: Vec<String>,
    /// Include every spatial layer (otherwise multi-layer inputs require selection)
    #[arg(long = "allLayers", conflicts_with = "layers")]
    all_layers: bool,
    /// Override input CRS, or use local for metre XYZ
    #[arg(long = "sourceCrs")]
    source_crs: Option<String>,
    /// Explicit additive offset from source heights to ellipsoidal metres
    #[arg(long = "heightOffset", allow_hyphen_values = true)]
    height_offset: Option<f64>,
    /// Maximum encoded POSITION vertices per content tile
    #[arg(long = "maxVertices", default_value_t = 65536)]
    max_vertices: usize,
    /// Maximum encoded GLB bytes per content tile
    #[arg(long = "maxBytes", default_value_t = 4194304)]
    max_bytes: usize,
    #[arg(long = "maxTiles", default_value_t = 100000)]
    max_tiles: usize,
    /// Maximum coordinates in one source feature, bounding reader memory
    #[arg(long = "maxSourceVertices", default_value_t = 1000000)]
    max_source_vertices: usize,

    /// Base simplification tolerance in metres; doubles for each coarse level
    #[arg(long = "lodTolerance", default_value_t = 0.1)]
    lod_tolerance: f64,
    /// Coarse levels above each full-detail leaf (1..16)
    #[arg(long = "lodLevels", default_value_t = 3)]
    lod_levels: u8,
    /// Repair invalid polygon outlines with an explicit per-feature conversion report
    #[arg(long)]
    repair: bool,
    /// Preserve geometrically ambiguous filled polygons as their source 3D outlines
    #[arg(long = "ambiguousOutlines")]
    ambiguous_outlines: bool,
    /// Quantize vector positions to normalized 16-bit integers (lossy, reported)
    #[arg(long)]
    quantize: bool,
    /// Losslessly compress vector accessor buffers with EXT_meshopt_compression
    #[arg(long)]
    meshopt: bool,
    /// Allow explicitly reported outline stand-ins for unsimplifiable parent polygons
    #[arg(long = "parentRepair")]
    parent_repair: bool,
    /// Maximum feature fragments in parent content; leaves use maxFeatures
    #[arg(long = "maxParentFeatures", default_value_t = 4096)]
    max_parent_features: usize,
    #[arg(long = "maxFeatures", default_value_t = 64)]
    max_features: usize,
    #[arg(short = 'i', long = "input")]
    input: PathBuf,
    #[arg(short = 'o', long = "output")]
    output: PathBuf,
}

#[derive(Args)]
struct ConvertArgs {
    #[arg(short = 'i', long = "input")]
    input: PathBuf,
    #[arg(short = 'o', long = "output")]
    output: PathBuf,
    #[arg(short = 'f', long = "force")]
    force: bool,
}

#[derive(Args)]
struct IoArgs {
    #[arg(short = 'i', long = "input")]
    input: PathBuf,
    #[arg(short = 'o', long = "output")]
    output: PathBuf,
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
    #[arg(short = 'f', long = "force")]
    force: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            if matches!(e, Error::NotImplemented { .. }) {
                ExitCode::from(2)
            } else {
                ExitCode::from(1)
            }
        }
    }
}

fn run() -> Result<(), Error> {
    let cli = Cli::parse();
    match cli.command {
        Command::EncodeVectorContent { input } => {
            println!("{}", rusty_tiles::vector_encoding::compress_file(&input)?);
        }
        Command::CreateTilesetJson(a) => {
            let opts = tileset_opts(&a)?;
            create_tileset_json(&a.input, &a.output, &opts)?;
        }
        Command::PointCloud(a) => rusty_tiles::point_cloud::point_cloud_to_3tz(
            &a.input,
            &a.output,
            &rusty_tiles::point_cloud::PointCloudOptions {
                force: a.force,
                source_crs: a.source_crs,
                height_offset: a.height_offset,
                max_points: a.max_points,
                chunk_points: a.chunk_points,
            },
        )?,
        Command::Convert(a) => {
            convert_to_3tz(&a.input, &a.output, &PackOptions { force: a.force })?;
        }
        Command::GlbTo3tz(a) => {
            let opts = tileset_opts(&a)?;
            glb_to_3tz(&a.input, &a.output, &opts)?;
        }
        Command::MeshTo3tz(a) => {
            let opts = mesh_opts(&a)?;
            mesh_to_3tz(&a.io.input, &a.io.output, &opts)?;
        }
        Command::Vector(a) => vector::vector_to_3tz_with_options(
            &a.input,
            &a.output,
            a.max_features,
            a.repair,
            a.ambiguous_outlines,
            &vector::VectorOptions {
                quantize: a.quantize,
                meshopt: a.meshopt,
                meshopt_encoder: if a.meshopt {
                    Some(std::env::current_exe()?)
                } else {
                    None
                },
                parent_repair: a.parent_repair,
                max_parent_features: a.max_parent_features,
                where_clause: a.where_clause,
                force: a.force,
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
        )?,
        Command::Terrain(a) => terrain::dem_to_terrain(
            &a.input,
            &a.output,
            &terrain::TerrainOptions {
                force: a.force,
                max_zoom: a.max_zoom,
                grid: a.grid,
                height_offset: a.height_offset,
                fill_height: a.fill_height,
            },
        )?,
        Command::Raster(a) => rusty_tiles::raster::raster_with_options(
            &a.input,
            &a.output,
            &rusty_tiles::raster::RasterOptions {
                force: a.force,
                min_zoom: a.min_zoom,
                max_zoom: a.max_zoom,
                display: a.display,
                band: a.band,
                alpha_band: a.alpha_band,
                display_min: a.display_min,
                display_max: a.display_max,
            },
        )?,
    }
    Ok(())
}

fn tileset_opts(a: &IoArgs) -> Result<CreateTilesetOptions, Error> {
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
        force: a.force,
    })
}

#[derive(Args)]
struct MeshArgs {
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
    #[command(flatten)]
    io: IoArgs,
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
    /// POSITION CRS: auto (detect), geographic (lon°/height/−lat°), or epsg:3857.
    #[arg(
        long = "sourceCrs",
        visible_alias = "source-crs",
        default_value = "auto"
    )]
    source_crs: String,
    /// Metashape Shift E N [A] in metres (Pseudo-Mercator). Added in f64, not f32.
    #[arg(
        long = "sourceOffset",
        visible_alias = "source-offset",
        num_args = 2..=3,
        allow_hyphen_values = true
    )]
    source_offset: Vec<f64>,
    /// Metashape offset.txt (`E: …` / `N: …` / `A: …`).
    #[arg(long = "sourceOffsetFile", visible_alias = "source-offset-file")]
    source_offset_file: Option<PathBuf>,
    /// Disable lossless meshopt compression. Both modes retain float32 geometry.
    #[arg(long = "noMeshopt", visible_alias = "no-meshopt")]
    no_meshopt: bool,
}

fn mesh_opts(a: &MeshArgs) -> Result<MeshTo3tzOptions, Error> {
    let ts = tileset_opts(&a.io)?;
    let source_crs = SourceCrs::parse_cli(&a.source_crs)?;
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
    if source_offset.is_some() && source_crs == SourceCrs::Geographic {
        return Err(Error::msg(
            "--sourceOffset requires --sourceCrs auto or epsg:3857",
        ));
    }
    Ok(MeshTo3tzOptions {
        texture_format: a.texture_format,
        basisu: a.basisu.clone(),
        cartographic: ts.cartographic,
        rotation: ts.rotation,
        force: ts.force,
        max_triangles: a.max_triangles,
        max_bytes: a.max_bytes,
        tile_size: a.tile_size,
        max_texel_density: a.max_texel_density,
        source_crs,
        source_offset,
        meshopt: !a.no_meshopt,
    })
}
