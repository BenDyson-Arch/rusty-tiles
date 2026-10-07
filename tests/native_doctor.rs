//! Readiness must use the linked native backend, even with no executable path.
use serde_json::Value;
use std::process::Command;

fn doctor(args: &[&str], data: Option<&std::path::Path>) -> (std::process::Output, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
    command
        .args(["doctor", "--json"])
        .args(args)
        .env("PATH", "");
    if let Some(path) = data {
        command.env("PROJ_DATA", path).env("PROJ_LIB", path);
    }
    let output = command.output().unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap();
    (output, report)
}

#[test]
fn selected_native_commands_ignore_missing_geospatial_database() {
    let root = tempfile::tempdir().unwrap();
    let grid = root.path().join("fixture.gtx");
    std::fs::write(&grid, b"inventory marker").unwrap();
    let (output, report) = doctor(
        &["--command", "convert", "--command", "point-cloud"],
        Some(root.path()),
    );
    assert!(output.status.success(), "{report}");
    assert_eq!(report["ready"], true);
    assert!(report.get("python").is_none());
    assert_eq!(report["proj"]["database"]["ready"], false);
    assert!(report["proj"]["availableGrids"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!(grid)));
    assert_eq!(std::fs::read(&grid).unwrap(), b"inventory marker");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn all_command_inventory_matches_enabled_capabilities() {
    let (output, report) = doctor(&[], None);
    assert!(report.get("python").is_none());
    assert_eq!(report["commands"]["mesh-to-3tz"]["ready"], true);
    assert_eq!(report["commands"]["point-cloud"]["local"]["ready"], true);
    assert_eq!(report["proj"]["networkEnabled"], false);
    #[cfg(feature = "native-geospatial")]
    {
        assert!(output.status.success(), "{report}");
        assert_eq!(report["ready"], true);
        assert!(report["nativeGeospatial"]["versions"]["gdal"].is_string());
        assert_eq!(report["commands"]["vector"]["geometry"]["ready"], true);
    }
    #[cfg(not(feature = "native-geospatial"))]
    {
        assert_eq!(output.status.code(), Some(4));
        assert_eq!(report["error"]["code"], "environment");
        assert_eq!(report["commands"]["terrain"]["ready"], false);
        assert!(report["nativeGeospatial"]["error"]
            .as_str()
            .unwrap()
            .contains("native-geospatial"));
    }
}

#[cfg(feature = "native-geospatial")]
#[test]
fn missing_database_keeps_versions_and_environment_category() {
    let root = tempfile::tempdir().unwrap();
    let (output, report) = doctor(&["--command", "terrain"], Some(root.path()));
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(report["error"]["code"], "environment");
    assert!(report["nativeGeospatial"]["versions"]["gdal"].is_string());
    assert!(report["proj"]["database"]["error"]
        .as_str()
        .unwrap()
        .contains("database"));
}
