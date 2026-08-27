use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use tinyowl_tiles::error::Error;
use tinyowl_tiles::georef::{Cartographic, RotationDegrees};
use tinyowl_tiles::pack::{convert_to_3tz, PackOptions};
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
