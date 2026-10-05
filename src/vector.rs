//! GeoJSON → draft glTF vector content (`3DTILES_content_gltf_vector`).
//!
//! The prototype pins a tested draft; the encoding spec is still drafting ([3d-tiles#825](https://github.com/CesiumGS/3d-tiles/issues/825),
//! [PR #838](https://github.com/CesiumGS/3d-tiles/pull/838)). Do not invent a private format.

use std::path::Path;

use crate::error::Error;

pub const SPEC_ISSUE: &str = "https://github.com/CesiumGS/3d-tiles/issues/825";
pub const SPEC_PR: &str = "https://github.com/CesiumGS/3d-tiles/pull/838";
pub const SPEC_BLOG: &str =
    "https://cesium.com/blog/2026/06/29/help-shape-vector-data-support-in-3d-tiles/";

/// Draft extension identifiers; see README for the tested runtime and schema revision.
pub const TILESET_EXTENSION: &str = "3DTILES_content_gltf_vector";
pub const GLTF_RESTART: &str = "KHR_mesh_primitive_restart";
pub const GLTF_POLYGON: &str = "EXT_mesh_polygon";
pub const GLTF_FEATURES: &str = "EXT_mesh_features";
pub const GLTF_METADATA: &str = "EXT_structural_metadata";

pub const INPUT_ORDER: &[&str] = &["GeoJSON", "GPKG layers", "shapefile"];

#[derive(Clone, Debug)]
pub struct VectorLodOptions {
    pub tolerance_metres: f64,
    pub levels: u8,
}

impl Default for VectorLodOptions {
    fn default() -> Self {
        Self {
            tolerance_metres: 0.1,
            levels: 3,
        }
    }
}

/// Compatibility entry point with default vector LOD settings.
pub fn vector_to_3tz(
    input: &Path,
    output: &Path,
    max_features: usize,
    repair: bool,
    ambiguous_outlines: bool,
) -> Result<(), Error> {
    vector_to_3tz_with_lod(
        input,
        output,
        max_features,
        repair,
        ambiguous_outlines,
        &VectorLodOptions::default(),
    )
}

pub fn vector_to_3tz_with_lod(
    input: &Path,
    output: &Path,
    max_features: usize,
    repair: bool,
    ambiguous_outlines: bool,
    lod: &VectorLodOptions,
) -> Result<(), Error> {
    if max_features == 0
        || !lod.tolerance_metres.is_finite()
        || lod.tolerance_metres <= 0.0
        || !(1..=16).contains(&lod.levels)
    {
        return Err(Error::msg("invalid vector LOD budgets: positive finite tolerance, positive feature budget and 1..16 levels required"));
    }
    if !input.is_file() {
        return Err(Error::InputNotFound(input.into()));
    }
    if output.exists() {
        return Err(Error::OutputExists(output.into()));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let work = tempfile::tempdir_in(parent)?;
    let mut command = std::process::Command::new("python3");
    command
        .arg("-c")
        .arg(include_str!("../scripts/vector.py"))
        .arg(input)
        .arg(work.path())
        .arg("--max-features")
        .arg(max_features.to_string())
        .arg("--lod-tolerance")
        .arg(lod.tolerance_metres.to_string())
        .arg("--lod-levels")
        .arg(lod.levels.to_string());
    if repair {
        command.arg("--repair");
    }
    if ambiguous_outlines {
        command.arg("--ambiguous-outlines");
    }
    let status = command.status()?;
    if !status.success() {
        return Err(Error::msg("glTF vector prototype failed; Python GDAL/GEOS and NumPy are required. No archive published."));
    }
    crate::pack::convert_to_3tz(work.path(), output, &crate::pack::PackOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_lod_settings_do_not_publish_output() {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("input.geojson");
        let output = work.path().join("output.3tz");
        std::fs::write(&input, b"{}").unwrap();
        for lod in [
            VectorLodOptions {
                tolerance_metres: f64::NAN,
                levels: 3,
            },
            VectorLodOptions {
                tolerance_metres: 0.0,
                levels: 3,
            },
            VectorLodOptions {
                tolerance_metres: 0.1,
                levels: 0,
            },
            VectorLodOptions {
                tolerance_metres: 0.1,
                levels: 17,
            },
        ] {
            assert!(vector_to_3tz_with_lod(&input, &output, 64, false, false, &lod).is_err());
            assert!(!output.exists());
        }
    }
}
