//! rusty-tiles: transform geospatial sources into 3D Tiles packages.
//!
//! v0 matches a subset of Cesium `3d-tiles-tools` (createTilesetJson + convert).
//! Raster and elevation source decoding require the native-geospatial build.

mod archive3tz;
pub mod bbox;
pub mod convert_implicit;
mod crs;
pub mod error;
pub mod fixtures;
pub mod georef;
#[cfg(feature = "native-geospatial")]
pub mod geospatial;
mod glb;
pub mod glb_write;
mod gpu_texture;
pub mod grid;
pub mod hlod;
pub mod implicit;
mod jpeg;
mod lossless;
pub mod mesh;
mod mesh_archive;
mod mesh_crs;
pub mod metadata;
mod output;
mod output_path;
pub mod pack;
pub mod package;
pub mod point_cloud;
mod point_sampling;
mod raster_directory;
pub mod report;
mod runtime;
pub mod terrain;
pub mod texture;
pub mod tile;
pub mod tileset;
mod tileset_node;
mod vec3;
pub mod vector;

pub use convert_implicit::{
    convert_to_implicit, convert_to_implicit_reported, ConvertToImplicitOptions,
};
pub use error::Error;
pub use georef::{
    parse_metashape_offset, Cartographic, RotationDegrees, SourceAxes, SourceCrs, SourceOffset,
};
pub use mesh_archive::{mesh_to_archive, MeshReport, MeshRequest, MeshResult};
pub use pack::{convert_to_3tz, pack_named_files, validate_3tz, TZ_INDEX_NAME};
pub use raster_directory::{
    raster_to_directory, RasterDirectoryReport, RasterDirectoryRequest, RasterDirectoryResult,
};
pub use report::{ConversionResult, Event, EventSink, Reporter};
pub use runtime::{
    CancellationHandle, CleanupDiagnostic, DirectoryRecovery, JobError, JobErrorKind, JobFailure,
    Observer, OutputPolicy, RunControl, RunEvent,
};
pub use tile::{
    mesh_to_3tz, MeshTo3tzOptions, DEFAULT_MAX_BYTES, DEFAULT_MAX_TEXEL_DENSITY,
    DEFAULT_MAX_TRIANGLES, DEFAULT_TILE_SIZE,
};
pub use tileset::{create_tileset_json, glb_to_3tz, CreateTilesetOptions};

/// npm package pin used as the v0 oracle. Golden tests call this via `npx`.
pub const ORACLE_NPM: &str = "3d-tiles-tools@0.5.4";

pub mod raster;

pub mod vector_encoding;

pub mod doctor;

pub mod preview;

pub mod validate;
