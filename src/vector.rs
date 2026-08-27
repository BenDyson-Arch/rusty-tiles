//! v1: GeoJSON / GPKG → 3D Tiles vector content (`3DTILES_content_gltf_vector`).
//!
//! The encoding spec is still drafting ([3d-tiles#825](https://github.com/CesiumGS/3d-tiles/issues/825),
//! [PR #838](https://github.com/CesiumGS/3d-tiles/pull/838)). Do not invent a private format.

use std::path::Path;

use crate::error::Error;

pub const SPEC_ISSUE: &str = "https://github.com/CesiumGS/3d-tiles/issues/825";
pub const SPEC_PR: &str = "https://github.com/CesiumGS/3d-tiles/pull/838";
pub const SPEC_BLOG: &str =
    "https://cesium.com/blog/2026/06/29/help-shape-vector-data-support-in-3d-tiles/";

/// Extensions the encoder will target once #838 is pin-able.
pub const TILESET_EXTENSION: &str = "3DTILES_content_gltf_vector";
pub const GLTF_RESTART: &str = "KHR_mesh_primitive_restart";
pub const GLTF_POLYGON: &str = "EXT_mesh_polygon";
pub const GLTF_FEATURES: &str = "EXT_mesh_features";
pub const GLTF_METADATA: &str = "EXT_structural_metadata";

pub const INPUT_ORDER: &[&str] = &["GeoJSON", "GPKG layers", "shapefile"];

pub fn vector_to_3tz(_input: &Path, _output: &Path) -> Result<(), Error> {
    Err(Error::NotImplemented {
        feature: "vector",
        hint: format!(
            "wait for a pin of {SPEC_PR}; then GeoJSON → {TILESET_EXTENSION} + implicit quadtree + .3tz. See {SPEC_ISSUE}"
        ),
    })
}
