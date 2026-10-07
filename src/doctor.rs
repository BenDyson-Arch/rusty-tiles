//! Read-only readiness checks for explicitly selected converter groups.
use crate::error::Error;
use serde_json::{json, Value};

pub fn report(selected: &[String]) -> Result<Value, Error> {
    let mut report = if !selected.is_empty()
        && selected.iter().all(|name| {
            matches!(
                name.as_str(),
                "point-cloud" | "terrain" | "raster" | "vector"
            )
        }) {
        json!({"commands":{}})
    } else {
        python_report(selected)?
    };
    report["commands"]["point-cloud"] = point_cloud_readiness();
    report["commands"]["terrain"] = terrain_readiness();
    report["commands"]["raster"] = raster_readiness();
    report["commands"]["vector"] = vector_readiness();
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
    report["ready"] = names
        .iter()
        .all(|name| report["commands"][name]["ready"] == true)
        .into();
    report["selectedCommands"] = json!(names);
    Ok(report)
}

fn terrain_readiness() -> Value {
    let geospatial = geospatial_readiness();
    json!({"ready":geospatial["ready"],"requires":["native GDAL >= 3.12", "PROJ >= 9.2", "local PROJ database/grids"],
        "geospatial":geospatial,"note":"Native terrain sampling and encoding; conversion validates the source-specific CRS operation."})
}

fn raster_readiness() -> Value {
    let geospatial = geospatial_readiness();
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

fn vector_readiness() -> Value {
    let geospatial = geospatial_readiness();
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
    let geospatial = match crate::geospatial::versions().and_then(|versions| {
        crate::geospatial::Crs::from_definition("EPSG:4326")?;
        Ok(versions)
    }) {
        Ok(versions) => json!({"ready":true,"versions":versions}),
        Err(error) => json!({"ready":false,"error":error.to_string()}),
    };
    #[cfg(not(feature = "native-geospatial"))]
    let geospatial = json!({"ready":false,"error":"rebuild with --features native-geospatial for native GDAL/PROJ operations"});
    geospatial
}

fn point_cloud_readiness() -> Value {
    let geospatial = geospatial_readiness();
    json!({"ready":true,"requires":[],"reader":"native LAS/LAZ","local":{"ready":true},
        "geospatial":geospatial,"note":"Local XYZ needs no Python or GDAL. Geospatial placement needs native GDAL/PROJ and a source-specific strict operation; conversion validates it."})
}

fn python_report(selected: &[String]) -> Result<Value, Error> {
    let result = std::process::Command::new("python3")
        .arg("-c")
        .arg(include_str!("../scripts/doctor.py"))
        .args(selected)
        .output();
    match result {
        Ok(output) if output.status.success() => Ok(serde_json::from_slice(&output.stdout)?),
        Ok(output) => Err(Error::msg(format!(
            "dependency check failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let native = [
                "mesh-to-3tz",
                "glb-to-3tz",
                "createTilesetJson",
                "convert",
                "point-cloud",
            ];
            let mut commands = serde_json::Map::new();
            for name in native {
                commands.insert(name.into(), json!({"ready":true,"requires":[]}));
            }
            for name in ["vector", "raster", "terrain"] {
                commands.insert(name.into(), json!({"ready":false,"missing":["python3"]}));
            }
            let ready =
                !selected.is_empty() && selected.iter().all(|name| native.contains(&name.as_str()));
            Ok(
                json!({"ready":ready,"python":{"available":false,"error":"python3 interpreter not found"},
                "commands":commands,"selectedCommands":selected}),
            )
        }
        Err(error) => Err(error.into()),
    }
}

pub fn display(report: &Value, json_output: bool) {
    if json_output {
        println!("{report}");
        return;
    }
    if !report["python"].is_object() {
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
                let geospatial = &info["geospatial"];
                if geospatial["ready"] == false {
                    println!(
                        "  native geospatial: {}",
                        geospatial["error"].as_str().unwrap_or("unavailable")
                    );
                }
            }
        }
        return;
    }
    println!(
        "Python: {}",
        report["python"]["executable"]
            .as_str()
            .unwrap_or("not found")
    );
    if let Some(modules) = report["modules"].as_object() {
        for (name, info) in modules {
            println!(
                "{name}: {}",
                info["version"]
                    .as_str()
                    .unwrap_or_else(|| info["error"].as_str().unwrap_or("unavailable"))
            );
        }
    }
    if let Some(commands) = report["commands"].as_object() {
        for (name, info) in commands {
            println!(
                "{name}: {}",
                if info["ready"] == true {
                    "ready"
                } else {
                    "missing dependencies"
                }
            );
        }
    }
    println!(
        "GEOS: {}",
        report["geos"]["version"].as_str().unwrap_or("unavailable")
    );
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
}
