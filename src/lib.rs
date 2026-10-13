//! rusty-tiles: transform geospatial sources into 3D Tiles packages.
//!
//! Typed foundation producers have explicit bounded source and output contracts.
//! Raster and elevation source decoding require the native-geospatial build.

mod archive3tz;
mod basisu_ktx2;
pub mod bbox;
mod content_integrity;
mod crs;
mod dir_sync;
pub mod error;
pub mod fixtures;
pub mod georef;
#[cfg(feature = "native-geospatial")]
pub mod geospatial;
mod glb;
pub mod glb_write;
mod gltf_source;
mod gpu_texture;
pub mod grid;
pub mod hlod;
pub mod implicit;
mod jpeg;
mod lossless;
pub mod mesh;
mod mesh_archive;
mod mesh_crs;
mod mesh_large;
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
mod spill;
mod stage_timing;
pub mod terrain;
pub mod texture;
pub mod tile;
mod tileset;
mod tileset_node;
mod vec3;
pub mod vector;

pub use error::Error;
pub use georef::{
    parse_metashape_offset, Cartographic, RotationDegrees, SourceAxes, SourceCrs, SourceOffset,
};
pub use mesh_archive::{
    mesh_to_archive, model_to_archive, model_to_manifest, MeshApproximation,
    MeshApproximationReport, MeshCertificateReport, MeshPlacement, MeshPlacementReport, MeshReport,
    MeshRequest, MeshResult, ModelManifestRequest, ModelManifestResult, ModelReport,
    ModelWrapRequest, ModelWrapResult,
};
pub use mesh_large::{
    mesh_to_leaf_payloads, LeafOutputReport, LeafOutputRequest, LeafOutputResult, LeafProfile,
};
pub use pack::{validate_3tz, TZ_INDEX_NAME};
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

/// npm package pin used as the v0 oracle. Golden tests call this via `npx`.
pub const ORACLE_NPM: &str = "3d-tiles-tools@0.5.4";

mod raster;
pub use raster::{
    raster_to_pyramid, RasterBandReport, RasterBands, RasterDisplay, RasterGridReport,
    RasterLimits, RasterReport, RasterRequest, RasterResult, RasterSourceReport,
};

mod vector_compression;
mod vector_encoding;
pub use vector_compression::{
    compress_vector_file, CompressionLimits, VectorCompressionReport, VectorCompressionRequest,
    VectorCompressionResult,
};

pub mod doctor;

pub mod preview;

pub mod validate;

#[cfg(test)]
#[path = "../bench/architecture_audit/c1_payload_integrity/production_controls/compiled_private_controls_clippy.rs"]
mod payload_integrity_acceptance;

#[cfg(test)]
#[path = "../bench/architecture_audit/c1_payload_integrity/production_controls/context_controls/2026-10-10/compiled_context_controls.rs"]
mod payload_context_acceptance;

#[cfg(test)]
#[path = "../bench/architecture_audit/c1_payload_integrity/production_controls/stride_controls/2026-10-10/compiled_stride_controls_clippy.rs"]
mod payload_stride_acceptance;

#[cfg(test)]
#[path = "../bench/architecture_audit/c2_decoded_containment/acceptance/oracle_parity.rs"]
mod c2_oracle_parity;

#[cfg(test)]
#[path = "../bench/architecture_audit/implicit_read/acceptance/oracle_parity.rs"]
mod i1_oracle_parity;

#[cfg(test)]
#[path = "../bench/architecture_audit/mesh_large_h8/decode_ktx.rs"]
mod h8_ktx_artifact_decoder;
