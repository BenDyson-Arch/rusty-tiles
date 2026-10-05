//! Read-only readiness checks for explicitly selected converter groups.
use crate::error::Error;
use serde_json::{json, Value};

pub fn report(selected: &[String]) -> Result<Value, Error> {
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
            let native = ["mesh-to-3tz", "glb-to-3tz", "createTilesetJson", "convert"];
            let mut commands = serde_json::Map::new();
            for name in native {
                commands.insert(name.into(), json!({"ready":true,"requires":[]}));
            }
            for name in ["vector", "raster", "terrain", "point-cloud"] {
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
