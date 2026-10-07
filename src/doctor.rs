//! Read-only readiness checks for explicitly selected converter groups.
use crate::error::Error;
use serde_json::{json, Value};

pub fn report(selected: &[String]) -> Result<Value, Error> {
    let geospatial = geospatial_readiness();
    let mut report =
        json!({"commands":{},"nativeGeospatial":geospatial,"proj":proj_inventory(&geospatial)});
    for name in [
        "mesh-to-3tz",
        "glb-to-3tz",
        "createTilesetJson",
        "convert",
        "validate",
        "preview",
    ] {
        report["commands"][name] = json!({"ready":true,"requires":[],"backend":"native Rust"});
    }
    report["commands"]["point-cloud"] = point_cloud_readiness(&geospatial);
    report["commands"]["terrain"] = terrain_readiness(&geospatial);
    report["commands"]["raster"] = raster_readiness(&geospatial);
    report["commands"]["vector"] = vector_readiness(&geospatial);
    let names: Vec<_> = if selected.is_empty() {
        report["commands"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    } else {
        selected.to_vec()
    };
    for name in &names {
        if !report["commands"][name].is_object() {
            return Err(Error::Data(format!("unknown readiness command: {name}")));
        }
    }
    report["ready"] = names
        .iter()
        .all(|name| report["commands"][name]["ready"] == true)
        .into();
    report["selectedCommands"] = json!(names);
    Ok(report)
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
