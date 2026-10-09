//! Audit observations, not regression assertions for the baseline's behavior.
//! Built by probe.py against the repository's locked portable library.
use rusty_tiles::{
    tile::{mesh_to_3tz_reported, MeshTo3tzOptions},
    tileset::{glb_to_3tz_reported, CreateTilesetOptions},
    Reporter, RotationDegrees, SourceCrs, SourceOffset,
};
use serde_json::{json, Value};
use std::{env, path::Path};

fn record(result: Result<rusty_tiles::ConversionResult, rusty_tiles::Error>) -> Value {
    match result {
        Ok(result) => json!({"status":"returned_success", "archive":result.archive}),
        Err(error) => json!({"status":"returned_error", "message":error.to_string()}),
    }
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let fixtures = Path::new(&args[1]);
    let outputs = Path::new(&args[2]);
    let mut records = serde_json::Map::new();
    for (name, offset) in [
        ("geographic_control", None),
        (
            "geographic_offset",
            Some(SourceOffset {
                easting: 1000.,
                northing: 2000.,
                height: 300.,
            }),
        ),
    ] {
        let options = MeshTo3tzOptions {
            source_crs: SourceCrs::Geographic,
            source_offset: offset,
            explicit: true,
            meshopt: false,
            ..Default::default()
        };
        records.insert(
            name.into(),
            record(mesh_to_3tz_reported(
                &fixtures.join("geographic.glb"),
                &outputs.join(format!("core_{name}.3tz")),
                &options,
                &Reporter::silent(),
            )),
        );
    }
    for (name, rotation) in [
        ("rotation_control", None),
        (
            "rotation_only",
            Some(RotationDegrees {
                heading: 37.,
                pitch: 11.,
                roll: 5.,
            }),
        ),
    ] {
        let options = MeshTo3tzOptions {
            rotation,
            explicit: true,
            ..Default::default()
        };
        records.insert(
            format!("mesh_{name}"),
            record(mesh_to_3tz_reported(
                &fixtures.join("local.glb"),
                &outputs.join(format!("core_mesh_{name}.3tz")),
                &options,
                &Reporter::silent(),
            )),
        );
        let options = CreateTilesetOptions {
            rotation,
            ..Default::default()
        };
        records.insert(
            format!("glb_{name}"),
            record(glb_to_3tz_reported(
                &fixtures.join("local.glb"),
                &outputs.join(format!("core_glb_{name}.3tz")),
                &options,
            )),
        );
    }
    let options = MeshTo3tzOptions {
        max_bytes: 1,
        tile_size: 64,
        explicit: true,
        meshopt: false,
        ..Default::default()
    };
    records.insert(
        "default_codec".into(),
        record(mesh_to_3tz_reported(
            &fixtures.join("textured.glb"),
            &outputs.join("core_default_codec.3tz"),
            &options,
            &Reporter::silent(),
        )),
    );
    println!("{}", Value::Object(records));
}
