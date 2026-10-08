//! Readiness must use the linked native backend, even with no executable path.
use serde_json::Value;
use std::process::Command;

fn doctor(args: &[&str], data: Option<&std::path::Path>) -> (std::process::Output, Value) {
    doctor_with(args, data, &[])
}

fn doctor_with(
    args: &[&str],
    data: Option<&std::path::Path>,
    env: &[(&str, &std::ffi::OsStr)],
) -> (std::process::Output, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
    command
        .args(["doctor", "--json"])
        .args(args)
        .env("PATH", "");
    if let Some(path) = data {
        command.env("PROJ_DATA", path).env("PROJ_LIB", path);
    }
    command.envs(env.iter().copied());
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
        &[
            "--command",
            "convert",
            "--command",
            "point-cloud",
            "--command",
            "mesh-to-3tz",
        ],
        Some(root.path()),
    );
    assert!(output.status.success(), "{report}");
    assert_eq!(report["ready"], true);
    assert!(report.get("modules").is_none());
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
    assert!(report.get("modules").is_none());
    // Without a filter every command is selected and listed.
    let mut listed: Vec<_> = report["commands"].as_object().unwrap().keys().collect();
    let mut selected: Vec<_> = report["selectedCommands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    listed.sort_unstable();
    selected.sort_unstable();
    assert_eq!(listed, selected);
    assert_eq!(listed.len(), rusty_tiles::doctor::COMMANDS.len());
    assert_eq!(report["commands"]["mesh-to-3tz"]["ready"], true);
    assert_eq!(report["commands"]["point-cloud"]["local"]["ready"], true);
    assert_eq!(report["proj"]["networkEnabled"], false);
    #[cfg(feature = "native-geospatial")]
    {
        assert!(output.status.success(), "{report}");
        assert_eq!(report["ready"], true);
        assert!(report["nativeGeospatial"]["versions"]["gdal"].is_string());
        assert_eq!(report["commands"]["vector"]["geometry"]["ready"], true);
        assert_eq!(report["proj"]["database"]["ready"], true);
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
    assert_eq!(report["ok"], false);
    assert_eq!(report["error"]["code"], "environment");
    assert!(report["nativeGeospatial"]["versions"]["gdal"].is_string());
    assert!(report["proj"]["database"]["error"]
        .as_str()
        .unwrap()
        .contains("database"));
}

// Linux permits arbitrary filename bytes; macOS filesystems reject this fixture.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_grid_path_keeps_selected_readiness_and_valid_json() {
    use std::os::unix::ffi::OsStringExt;
    let root = tempfile::tempdir().unwrap();
    let grid = root
        .path()
        .join(std::ffi::OsString::from_vec(b"grid-\xff.gtx".to_vec()));
    std::fs::write(&grid, b"inventory marker").unwrap();
    let (output, report) = doctor(&["--command", "convert"], Some(root.path()));
    assert!(output.status.success(), "{report}");
    assert_eq!(report["ready"], true);
    assert!(report["proj"]["availableGrids"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path.as_str() == Some(grid.to_string_lossy().as_ref())));
    assert!(report["proj"]["inventoryErrors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error.as_str().unwrap().contains("not UTF-8")));
    assert_eq!(std::fs::read(&grid).unwrap(), b"inventory marker");
}

#[cfg(unix)]
#[test]
fn non_utf8_search_path_keeps_selected_readiness_and_valid_json() {
    use std::os::unix::ffi::OsStringExt;
    let root = tempfile::tempdir().unwrap();
    let grid = root.path().join("fixture.gtx");
    std::fs::write(&grid, b"inventory marker").unwrap();
    // Environment variables can contain these bytes even when the filesystem
    // cannot. Also retain a valid directory to check the rest of the inventory.
    let invalid = root
        .path()
        .join(std::ffi::OsString::from_vec(b"missing-\xff".to_vec()));
    let paths = std::env::join_paths([root.path(), invalid.as_path()]).unwrap();
    let (output, report) = doctor_with(
        &["--command", "convert"],
        None,
        &[
            ("PROJ_DATA", paths.as_os_str()),
            ("PROJ_LIB", paths.as_os_str()),
        ],
    );
    assert!(output.status.success(), "{report}");
    assert_eq!(report["ready"], true);
    assert!(report["proj"]["availableGrids"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!(grid)));
    assert!(report["proj"]["dataDirectories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path.as_str() == Some(invalid.to_string_lossy().as_ref())));
    assert!(report["proj"]["inventoryErrors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error.as_str().unwrap().contains("not UTF-8")));
    assert_eq!(std::fs::read(&grid).unwrap(), b"inventory marker");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[cfg(feature = "native-geospatial")]
#[test]
fn vector_profile_reports_versions_and_capabilities() {
    let (output, report) = doctor(&["--command", "vector"], None);
    assert!(output.status.success(), "{report}");
    assert_eq!(report["ready"], true);
    let vector = &report["commands"]["vector"];
    assert_eq!(vector["geometry"]["ready"], true);
    assert_eq!(vector["geospatial"]["ready"], true);
    assert_eq!(vector["backend"], "native GDAL/GEOS");
    assert!(vector["geospatial"]["versions"]["gdal"].is_string());
    assert!(report.get("python").is_none());
}

#[test]
fn command_filter_limits_json_and_human_output() {
    let (_, report) = doctor(&["--command", "vector"], None);
    let listed: Vec<_> = report["commands"].as_object().unwrap().keys().collect();
    assert_eq!(listed, ["vector"]);
    assert_eq!(report["selectedCommands"], serde_json::json!(["vector"]));
    let (_, report) = doctor(&["--command", "preview", "--command", "meshTo3tz"], None);
    let listed: Vec<_> = report["commands"].as_object().unwrap().keys().collect();
    assert_eq!(listed, ["mesh-to-3tz", "preview"]);
    let human = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["doctor", "--command", "point-cloud"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(human.status.success());
    let stdout = String::from_utf8(human.stdout).unwrap();
    let commands: Vec<_> = stdout
        .lines()
        .filter_map(|line| line.split_once(": "))
        .map(|(name, _)| name)
        .filter(|name| rusty_tiles::doctor::canonical(name).is_some())
        .collect();
    assert_eq!(commands, ["point-cloud"], "{stdout}");
}

#[test]
fn python_modules_and_path_do_not_affect_readiness() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("numpy.py"),
        "raise ImportError('doctor fixture: numpy missing')",
    )
    .unwrap();
    let pythonpath = [("PYTHONPATH", root.path().as_os_str())];
    for command in ["mesh-to-3tz", "convert"] {
        let (output, report) = doctor_with(&["--command", command], None, &pythonpath);
        assert!(output.status.success(), "{report}");
        assert_eq!(report["ready"], true);
        assert!(report.get("modules").is_none());
        assert!(report.get("python").is_none());
    }
    let (output, report) = doctor_with(&["--command", "vector"], None, &pythonpath);
    assert_eq!(output.status.success(), cfg!(feature = "native-geospatial"));
    assert_eq!(report["ready"], cfg!(feature = "native-geospatial"));
    assert!(report.get("modules").is_none());
}

#[test]
fn aliases_and_informational_cesium_check() {
    let root = tempfile::tempdir().unwrap();
    let cesium = root.path().to_str().unwrap();
    let (output, report) = doctor(
        &[
            "--command",
            "meshTo3tz",
            "--command",
            "create-tileset-json",
            "--command",
            "preview",
            "--cesium",
            cesium,
        ],
        None,
    );
    assert!(output.status.success(), "{report}");
    assert_eq!(report["ready"], true);
    assert_eq!(
        report["selectedCommands"],
        serde_json::json!(["mesh-to-3tz", "createTilesetJson", "preview"])
    );
    assert_eq!(report["commands"]["preview"]["cesium"]["found"], false);
    std::fs::write(root.path().join("Cesium.js"), "// invented runtime").unwrap();
    let (output, report) = doctor(&["--command", "preview", "--cesium", cesium], None);
    assert!(output.status.success(), "{report}");
    assert_eq!(report["commands"]["preview"]["cesium"]["path"], cesium);
    assert_eq!(report["commands"]["preview"]["cesium"]["found"], true);
}
