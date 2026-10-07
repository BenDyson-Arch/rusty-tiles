//! Read-only readiness checks for explicitly selected converter groups.
use crate::error::Error;
use serde_json::{json, Value};

use std::path::Path;

/// Subcommands with a readiness entry: canonical name, then accepted aliases.
/// `doctor --command` takes its values from this list, and the CLI tests check
/// it against the clap subcommand definitions so the two cannot drift.
pub const COMMANDS: &[(&str, &[&str])] = &[
    ("vector", &[]),
    ("raster", &[]),
    ("terrain", &[]),
    ("point-cloud", &[]),
    ("mesh-to-3tz", &["meshTo3tz"]),
    ("glb-to-3tz", &["glbTo3tz"]),
    ("createTilesetJson", &["create-tileset-json"]),
    ("convert", &[]),
    ("validate", &[]),
    ("preview", &[]),
];

/// Cesium runtime location used by the README install instructions.
pub const DEFAULT_CESIUM: &str = "target/preview-runtime/node_modules/cesium/Build/Cesium";

/// Canonical readiness name for a subcommand name or alias.
pub fn canonical(name: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .find(|(canonical, aliases)| *canonical == name || aliases.contains(&name))
        .map(|(canonical, _)| *canonical)
}

/// Readiness of `selected` commands (all when empty). `cesium` is the preview
/// runtime directory to look for; it defaults to [`DEFAULT_CESIUM`].
pub fn report(selected: &[String], cesium: Option<&Path>) -> Result<Value, Error> {
    let geospatial = geospatial_readiness();
    let mut report =
        json!({"commands":{},"nativeGeospatial":geospatial,"proj":proj_inventory(&geospatial)});
    for name in [
        "mesh-to-3tz",
        "glb-to-3tz",
        "createTilesetJson",
        "convert",
        "validate",
    ] {
        report["commands"][name] = json!({"ready":true,"requires":[],"backend":"native Rust"});
    }
    report["commands"]["preview"] = preview_readiness(cesium.unwrap_or(Path::new(DEFAULT_CESIUM)));
    report["commands"]["point-cloud"] = point_cloud_readiness(&geospatial);
    report["commands"]["terrain"] = terrain_readiness(&geospatial);
    report["commands"]["raster"] = raster_readiness(&geospatial);
    report["commands"]["vector"] = vector_readiness(&geospatial);
    let mut names: Vec<String> = Vec::new();
    if selected.is_empty() {
        names.extend(report["commands"].as_object().unwrap().keys().cloned());
    }
    for name in selected {
        let name = canonical(name)
            .ok_or_else(|| Error::Data(format!("unknown readiness command: {name}")))?;
        if !names.iter().any(|selected| selected == name) {
            names.push(name.to_owned());
        }
    }
    report["ready"] = names
        .iter()
        .all(|name| report["commands"][name]["ready"] == true)
        .into();
    report["selectedCommands"] = json!(names);
    Ok(report)
}

/// Informational only: preview always takes an explicit `--cesium`, so a
/// missing runtime at the checked path never makes the command unavailable.
fn preview_readiness(cesium: &Path) -> Value {
    let found = cesium.join("Cesium.js").is_file();
    json!({"ready":true,"requires":[],"backend":"native Rust",
        "cesium":{"path":cesium.to_string_lossy(),"found":found,
            "note":if found {"Cesium IIFE runtime found; pass this directory to preview --cesium."}
                else {"No Cesium.js here. Install cesium@1.143.0 as described in the README and pass its Build/Cesium directory to preview --cesium."}}})
}

fn terrain_readiness(geospatial: &Value) -> Value {
    json!({"ready":geospatial["ready"],"backend":"native GDAL/Rust","requires":["native GDAL >= 3.12", "PROJ >= 9.2", "local PROJ database/grids"],
        "geospatial":geospatial,"note":"Native terrain sampling and encoding; conversion validates the source-specific CRS operation."})
}

fn raster_readiness(geospatial: &Value) -> Value {
    #[cfg(feature = "native-geospatial")]
    let tiling = match crate::raster::tile_available() {
        Ok(()) => json!({"ready":true}),
        Err(error) => json!({"ready":false,"error":error.to_string()}),
    };
    #[cfg(not(feature = "native-geospatial"))]
    let tiling = json!({"ready":false});
    json!({"ready":geospatial["ready"] == true && tiling["ready"] == true,
        "requires":["native GDAL >= 3.12 with raster tile algorithm", "PROJ >= 9.2", "local PROJ database/grids"],
        "backend":"native GDAL", "geospatial":geospatial, "tiling":tiling,
        "note":"Native COG, display and tiling APIs; no Python or GDAL executable required."})
}

fn vector_readiness(geospatial: &Value) -> Value {
    #[cfg(feature = "native-geospatial")]
    let geometry = match crate::vector::native_available() {
        Ok(()) => json!({"ready":true}),
        Err(error) => json!({"ready":false,"error":error.to_string()}),
    };
    #[cfg(not(feature = "native-geospatial"))]
    let geometry = json!({"ready":false});
    json!({"ready":geospatial["ready"] == true && geometry["ready"] == true,
        "requires":["native GDAL >= 3.12", "GEOS >= 3.10", "SQLite", "PROJ >= 9.2"],
        "backend":"native GDAL/GEOS", "geospatial":geospatial, "geometry":geometry,
        "note":"Native OGR ingestion, constrained triangulation, LOD, meshopt and archive reuse; no Python required."})
}

fn geospatial_readiness() -> Value {
    #[cfg(feature = "native-geospatial")]
    let geospatial = match crate::geospatial::versions() {
        Ok(versions) => match crate::geospatial::Crs::from_definition("EPSG:4326") {
            Ok(_) => json!({"ready":true,"versions":versions}),
            Err(error) => json!({"ready":false,"versions":versions,"error":error.to_string()}),
        },
        Err(error) => json!({"ready":false,"error":error.to_string()}),
    };
    #[cfg(not(feature = "native-geospatial"))]
    let geospatial = json!({"ready":false,"error":"rebuild with --features native-geospatial for native GDAL/PROJ operations"});
    geospatial
}

fn point_cloud_readiness(geospatial: &Value) -> Value {
    json!({"ready":true,"requires":[],"reader":"native LAS/LAZ","local":{"ready":true},
        "geospatial":geospatial,"note":"Local XYZ needs no Python or GDAL. Geospatial placement needs native GDAL/PROJ and a source-specific strict operation; conversion validates it."})
}

fn proj_inventory(database: &Value) -> Value {
    let mut paths = std::collections::BTreeSet::new();
    for name in ["PROJ_DATA", "PROJ_LIB"] {
        if let Some(value) = std::env::var_os(name) {
            paths.extend(std::env::split_paths(&value).filter(|path| !path.as_os_str().is_empty()));
        }
    }
    #[cfg(feature = "native-geospatial")]
    paths.extend(crate::geospatial::proj_search_paths());
    let mut grids = std::collections::BTreeSet::new();
    let mut inventory_errors = Vec::new();
    for path in &paths {
        match std::fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => {
                            let file = entry.path();
                            if file.is_file()
                                && file.extension().is_some_and(|suffix| {
                                    matches!(suffix.to_str(), Some("gtx" | "gsb" | "tif" | "bin"))
                                })
                            {
                                grids.insert(file);
                            }
                        }
                        Err(error) => inventory_errors.push(format!("{}: {error}", path.display())),
                    }
                }
            }
            Err(error) => inventory_errors.push(format!("{}: {error}", path.display())),
        }
    }
    // JSON paths are text; retain the inventory instead of panicking on Unix
    // filenames that are not UTF-8, and report any lossy representation.
    let mut text_paths = |paths: std::collections::BTreeSet<std::path::PathBuf>| {
        paths
            .into_iter()
            .map(|path| {
                if path.to_str().is_none() {
                    inventory_errors.push(format!(
                        "{}: path is not UTF-8; displayed with replacement characters",
                        path.display()
                    ));
                }
                path.to_string_lossy().into_owned()
            })
            .collect::<Vec<_>>()
    };
    let paths = text_paths(paths);
    let grids = text_paths(grids);
    json!({"database":database,"dataDirectories":paths,"availableGrids":grids,
        "inventoryErrors":inventory_errors,"networkEnabled":false,
        "note":"Read-only top-level local grid inventory is not proof that a source-specific height operation is available; conversion validates that operation offline."})
}

pub fn display(report: &Value, json_output: bool) {
    if json_output {
        println!("{report}");
        return;
    }
    if let Some(versions) = report["nativeGeospatial"]["versions"].as_object() {
        println!(
            "GDAL: {}",
            versions["gdal"].as_str().unwrap_or("unavailable")
        );
        println!("PROJ: {}", versions["proj"]);
        println!("GEOS: {}", versions["geos"]);
    }
    if report["nativeGeospatial"]["ready"] == false {
        println!(
            "Native geospatial: {}",
            report["nativeGeospatial"]["error"]
                .as_str()
                .unwrap_or("unavailable")
        );
    }
    if let Some(commands) = report["commands"].as_object() {
        for (name, info) in commands {
            println!(
                "{name}: {}",
                if info["ready"] == true {
                    "ready"
                } else {
                    "unavailable"
                }
            );
            for capability in ["geospatial", "geometry", "tiling"] {
                if let Some(error) = info[capability]["error"].as_str() {
                    println!("  {capability}: {error}");
                }
            }
            if let Some(path) = info["cesium"]["path"].as_str() {
                if info["cesium"]["found"] == true {
                    println!("  cesium: found at {path}");
                } else {
                    println!("  cesium: not found at {path} (informational; pass preview --cesium <Build/Cesium>)");
                }
            }
        }
    }
    if let Some(paths) = report["proj"]["dataDirectories"].as_array() {
        for path in paths {
            println!("PROJ data: {}", path.as_str().unwrap_or(""));
        }
    }
    if let Some(grids) = report["proj"]["availableGrids"].as_array() {
        println!("PROJ grids: {}", grids.len());
        for grid in grids {
            println!("  {}", grid.as_str().unwrap_or(""));
        }
    }
    println!("{}", report["proj"]["note"].as_str().unwrap_or(""));
}
