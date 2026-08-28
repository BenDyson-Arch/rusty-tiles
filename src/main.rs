use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use tinyowl_tiles::error::Error;
use tinyowl_tiles::georef::{
    parse_metashape_offset, Cartographic, RotationDegrees, SourceCrs, SourceOffset,
};
use tinyowl_tiles::pack::{convert_to_3tz, PackOptions};
use tinyowl_tiles::tile::{mesh_to_3tz, MeshTo3tzOptions};
use tinyowl_tiles::tileset::{create_tileset_json, glb_to_3tz, CreateTilesetOptions};
use tinyowl_tiles::{terrain, vector};

#[derive(Parser)]
#[command(
    name = "tinyowl-tiles",
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
    /// GeoJSON/GPKG → 3D Tiles 2.0 vector tiles (not implemented until spec pin)
    Vector(BasicIo),
    /// DEM → 3D terrain tiles (not scheduled in v0)
    Terrain(BasicIo),
}

#[derive(Args)]
struct BasicIo {
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
        Command::CreateTilesetJson(a) => {
            let opts = tileset_opts(&a)?;
            create_tileset_json(&a.input, &a.output, &opts)?;
        }
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
        Command::Vector(a) => vector::vector_to_3tz(&a.input, &a.output)?,
        Command::Terrain(a) => terrain::dem_to_terrain(&a.input, &a.output)?,
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
    #[command(flatten)]
    io: IoArgs,
    /// Stop splitting a node at this many triangles (default 20000).
    #[arg(
        long = "maxTriangles",
        visible_alias = "max-triangles",
        default_value_t = tinyowl_tiles::DEFAULT_MAX_TRIANGLES
    )]
    max_triangles: usize,
    /// Source-file wrap threshold in bytes (default 204800). Split stop is
    /// `--maxTriangles`; written tiles are meshopt-compressed unless `--noMeshopt`.
    #[arg(
        long = "maxBytes",
        visible_alias = "max-bytes",
        default_value_t = tinyowl_tiles::DEFAULT_MAX_BYTES
    )]
    max_bytes: u64,
    /// Max edge length in pixels of a cropped leaf texture (default 256).
    #[arg(
        long = "tileSize",
        visible_alias = "tile-size",
        default_value_t = tinyowl_tiles::DEFAULT_TILE_SIZE
    )]
    tile_size: u32,
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
    /// Write float32 GLBs (skip KHR_mesh_quantization + EXT_meshopt_compression).
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
        cartographic: ts.cartographic,
        rotation: ts.rotation,
        force: ts.force,
        max_triangles: a.max_triangles,
        max_bytes: a.max_bytes,
        tile_size: a.tile_size,
        source_crs,
        source_offset,
        meshopt: !a.no_meshopt,
    })
}
