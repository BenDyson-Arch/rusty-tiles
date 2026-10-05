//! rusty-tiles: transform geospatial sources into 3D Tiles packages.
//!
//! v0 matches a subset of Cesium `3d-tiles-tools` (createTilesetJson + convert).
//! Raster, quantized-mesh terrain and draft glTF vector paths are GDAL-backed lab tools.

pub mod bbox;
pub mod compress;
pub mod error;
pub mod fixtures;
pub mod georef;
pub mod glb_write;
mod gpu_texture;
pub mod grid;
pub mod hlod;
mod jpeg;
mod lossless;
pub mod mesh;
pub mod pack;
pub mod point_cloud;
mod python;
pub mod split;
pub mod terrain;
pub mod texture;
pub mod tile;
pub mod tileset;
pub mod vector;

pub use compress::write_glb_compressed;
pub use error::Error;
pub use georef::{parse_metashape_offset, Cartographic, RotationDegrees, SourceCrs, SourceOffset};
pub use pack::{convert_to_3tz, pack_named_files, validate_3tz, TZ_INDEX_NAME};
pub use tile::{
    mesh_to_3tz, MeshTo3tzOptions, DEFAULT_MAX_BYTES, DEFAULT_MAX_TEXEL_DENSITY,
    DEFAULT_MAX_TRIANGLES, DEFAULT_TILE_SIZE,
};
pub use tileset::{create_tileset_json, glb_to_3tz, CreateTilesetOptions};

/// npm package pin used as the v0 oracle. Golden tests call this via `npx`.
pub const ORACLE_NPM: &str = "3d-tiles-tools@0.5.4";

pub mod raster;
