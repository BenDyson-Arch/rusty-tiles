//! tinyowl-tiles: transform geospatial sources into 3D Tiles packages.
//!
//! v0 matches a subset of Cesium `3d-tiles-tools` (createTilesetJson + convert).
//! Vector (3D Tiles 2.0) and terrain are CLI-shaped stubs until those specs/pipelines land.

pub mod bbox;
pub mod error;
pub mod fixtures;
pub mod georef;
pub mod pack;
pub mod terrain;
pub mod tileset;
pub mod vector;

pub use error::Error;
pub use georef::{Cartographic, RotationDegrees};
pub use pack::{convert_to_3tz, pack_named_files, validate_3tz, TZ_INDEX_NAME};
pub use tileset::{create_tileset_json, glb_to_3tz, CreateTilesetOptions};

/// npm package pin used as the v0 oracle. Golden tests call this via `npx`.
pub const ORACLE_NPM: &str = "3d-tiles-tools@0.5.4";
