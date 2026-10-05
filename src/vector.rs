//! OGR vector layers → draft glTF vector content (`3DTILES_content_gltf_vector`).
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

/// OGR input selection and hard content budgets. Heights are explicit for 3D
/// horizontal-CRS sources; `local` uses metre XYZ without geospatial placement.
#[derive(Clone, Debug)]
pub struct VectorOptions {
    pub quantize: bool,
    pub meshopt: bool,
    /// Library callers supply the rusty-tiles executable used for native compression.
    pub meshopt_encoder: Option<std::path::PathBuf>,
    pub parent_repair: bool,
    pub max_parent_features: usize,
    pub where_clause: Option<String>,
    pub force: bool,
    pub list_fields: String,
    pub fields: Vec<String>,
    pub drop_fields: Vec<String>,
    pub skip_invalid: bool,
    pub lod: VectorLodOptions,
    pub reuse_tileset: Option<std::path::PathBuf>,
    pub layers: Vec<String>,
    pub all_layers: bool,
    pub source_crs: Option<String>,
    pub height_offset: Option<f64>,
    pub max_vertices: usize,
    pub max_bytes: usize,
    pub max_tiles: usize,
    pub max_source_vertices: usize,
}

impl Default for VectorOptions {
    fn default() -> Self {
        Self {
            quantize: false,
            meshopt: false,
            meshopt_encoder: None,
            parent_repair: false,
            max_parent_features: 4096,
            where_clause: None,
            force: false,
            list_fields: "error".into(),
            fields: Vec::new(),
            drop_fields: Vec::new(),
            skip_invalid: false,
            lod: VectorLodOptions::default(),
            reuse_tileset: None,
            layers: Vec::new(),
            all_layers: false,
            source_crs: None,
            height_offset: None,
            max_vertices: 65536,
            max_bytes: 4194304,
            max_tiles: 100000,
            max_source_vertices: 1000000,
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
    vector_to_3tz_with_options(
        input,
        output,
        max_features,
        repair,
        ambiguous_outlines,
        &VectorOptions {
            lod: lod.clone(),
            ..VectorOptions::default()
        },
    )
}

pub fn vector_to_3tz_with_options(
    input: &Path,
    output: &Path,
    max_features: usize,
    repair: bool,
    ambiguous_outlines: bool,
    options: &VectorOptions,
) -> Result<(), Error> {
    let lod = &options.lod;
    if options
        .where_clause
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(Error::msg("where filter must not be empty"));
    }
    if !matches!(options.list_fields.as_str(), "error" | "json")
        || (!options.fields.is_empty() && !options.drop_fields.is_empty())
    {
        return Err(Error::msg(
            "invalid vector field selection or listFields setting",
        ));
    }
    if options.max_parent_features == 0
        || options.max_vertices < 4
        || options.max_bytes < 4096
        || options.max_tiles == 0
        || options.max_source_vertices == 0
        || options.height_offset.is_some_and(|v| !v.is_finite())
        || (options.all_layers && !options.layers.is_empty())
    {
        return Err(Error::msg(
            "invalid vector input selection or content budgets",
        ));
    }
    if max_features == 0
        || !lod.tolerance_metres.is_finite()
        || lod.tolerance_metres <= 0.0
        || !(1..=16).contains(&lod.levels)
    {
        return Err(Error::msg("invalid vector LOD budgets: positive finite tolerance, positive feature budget and 1..16 levels required"));
    }
    if let Some(previous) = &options.reuse_tileset {
        if !previous.is_file() {
            return Err(Error::InputNotFound(previous.clone()));
        }
    }
    if !input.is_file() {
        return Err(Error::InputNotFound(input.into()));
    }
    if output.exists() && !options.force {
        return Err(Error::OutputExists(output.into()));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let work = tempfile::tempdir_in(parent)?;
    let mut script = String::from("import sys,types\n");
    for (name, source) in [
        ("vector_source", include_str!("../scripts/vector_source.py")),
        ("vector_reuse", include_str!("../scripts/vector_reuse.py")),
        (
            "vector_pipeline",
            include_str!("../scripts/vector_pipeline.py"),
        ),
    ] {
        script.push_str(&format!(
            "m=types.ModuleType({name:?});sys.modules[{name:?}]=m;m.__source__={}\nexec(compile(m.__source__, '<rusty-tiles/{name}.py>', 'exec'),m.__dict__)\n",
            serde_json::to_string(source)?
        ));
    }
    script.push_str(&format!(
        "__source__={}\nexec(compile(__source__, '<rusty-tiles/vector.py>', 'exec'),globals())",
        serde_json::to_string(include_str!("../scripts/vector.py"))?
    ));
    let mut command = std::process::Command::new("python3");
    command
        .arg("-c")
        .arg(crate::python::script(
            &script,
            "vector",
            "Python GDAL/GEOS and NumPy",
        )?)
        .arg(input)
        .arg(work.path())
        .arg("--max-parent-features")
        .arg(options.max_parent_features.to_string())
        .arg("--max-features")
        .arg(max_features.to_string())
        .arg("--lod-tolerance")
        .arg(lod.tolerance_metres.to_string())
        .arg("--lod-levels")
        .arg(lod.levels.to_string())
        .arg("--max-vertices")
        .arg(options.max_vertices.to_string())
        .arg("--max-bytes")
        .arg(options.max_bytes.to_string())
        .arg("--max-tiles")
        .arg(options.max_tiles.to_string())
        .arg("--max-source-vertices")
        .arg(options.max_source_vertices.to_string());
    if let Some(previous) = &options.reuse_tileset {
        command.arg("--reuse-tileset").arg(previous);
    }
    for layer in &options.layers {
        command.arg("--layer").arg(layer);
    }
    if options.all_layers {
        command.arg("--all-layers");
    }
    if let Some(crs) = &options.source_crs {
        command.arg("--source-crs").arg(crs);
    }
    if let Some(offset) = options.height_offset {
        command.arg("--height-offset").arg(offset.to_string());
    }
    if let Some(expression) = &options.where_clause {
        command.arg("--where").arg(expression);
    }
    command.arg("--list-fields").arg(&options.list_fields);
    for field in &options.fields {
        command.arg("--field").arg(field);
    }
    for field in &options.drop_fields {
        command.arg("--drop-field").arg(field);
    }
    if options.skip_invalid {
        command.arg("--skip-invalid");
    }
    if options.quantize {
        command.arg("--quantize");
    }
    if options.meshopt {
        command
            .arg("--meshopt-helper")
            .arg(std::env::current_exe()?);
    }
    if options.parent_repair {
        command.arg("--parent-repair");
    }
    if repair {
        command.arg("--repair");
    }
    if ambiguous_outlines {
        command.arg("--ambiguous-outlines");
    }
    crate::python::run(&mut command, "vector")?;
    crate::pack::convert_to_3tz(
        work.path(),
        output,
        &crate::pack::PackOptions {
            force: options.force,
        },
    )
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
