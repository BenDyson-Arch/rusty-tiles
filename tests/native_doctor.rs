//! Readiness must use the linked native backend, even with no executable path.
use serde_json::{json, Value};
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
            "mesh",
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
    assert_eq!(listed.len(), rusty_tiles::doctor::Command::ALL.len());
    for readiness in report["commands"].as_object().unwrap().values() {
        let status = readiness["status"].as_str().unwrap();
        assert!(["ready", "unavailable", "unsupported"].contains(&status));
        assert_eq!(readiness["ready"], status == "ready");
    }
    assert_eq!(report["commands"]["mesh"]["ready"], true);
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
        // Nothing compiled in is missing, so the build alone decides.
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(report["error"]["code"], "unsupported");
        assert_eq!(report["status"], "unsupported");
        assert_eq!(report["commands"]["terrain"]["status"], "unsupported");
        assert_eq!(report["commands"]["vector"]["ready"], true);
        assert_eq!(report["nativeGeospatial"]["status"], "unsupported");
        assert!(report["nativeGeospatial"]["reason"]
            .as_str()
            .unwrap()
            .contains("native-geospatial"));
    }
}

#[cfg(not(feature = "native-geospatial"))]
#[test]
fn portable_vector_readiness_needs_no_native_database_or_executables() {
    let root = tempfile::tempdir().unwrap();
    let (output, report) = doctor(&["--command", "vector"], Some(root.path()));
    assert!(output.status.success(), "{report}");
    assert_eq!(report["ready"], true);
    assert_eq!(report["nativeGeospatial"]["ready"], false);
    let vector = &report["commands"]["vector"];
    assert_eq!(vector["requires"], serde_json::json!([]));
    assert_eq!(
        vector["inputs"],
        serde_json::json!(["GeoJSON", "GeoPackage"])
    );
    assert_eq!(vector["reader"]["ready"], true);
    assert_eq!(vector["geometry"]["ready"], true);
    assert_eq!(vector["geospatial"]["ready"], true);
    assert_eq!(vector["geospatial"]["nativeFallback"]["ready"], false);
    assert!(vector["limitations"]
        .as_str()
        .unwrap()
        .contains("Shapefile"));
    assert!(vector["geospatial"]["limitations"]
        .as_str()
        .unwrap()
        .contains("height offset"));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[cfg(feature = "native-geospatial")]
#[test]
fn missing_database_keeps_versions_and_execution_category() {
    let root = tempfile::tempdir().unwrap();
    let (output, report) = doctor(&["--command", "terrain"], Some(root.path()));
    // Terrain itself refuses its source CRS as unsupported without the database.
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(report["ok"], false);
    assert_eq!(report["error"]["code"], "unsupported");
    assert!(report["nativeGeospatial"]["versions"]["gdal"].is_string());
    assert_eq!(report["status"], "unavailable");
    assert_eq!(report["commands"]["terrain"]["status"], "unavailable");
    assert_eq!(report["proj"]["database"]["status"], "unavailable");
    assert!(report["proj"]["database"]["reason"]
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
    let (_, report) = doctor(&["--command", "preview", "--command", "glbTo3tz"], None);
    let listed: Vec<_> = report["commands"].as_object().unwrap().keys().collect();
    assert_eq!(listed, ["glb-to-3tz", "preview"]);
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
        .filter(|name| rusty_tiles::doctor::Command::from_name(name).is_some())
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
    for command in ["mesh", "convert"] {
        let (output, report) = doctor_with(&["--command", command], None, &pythonpath);
        assert!(output.status.success(), "{report}");
        assert_eq!(report["ready"], true);
        assert!(report.get("modules").is_none());
        assert!(report.get("python").is_none());
    }
    let (output, report) = doctor_with(&["--command", "vector"], None, &pythonpath);
    assert!(output.status.success(), "{report}");
    assert_eq!(report["ready"], true);
    assert!(report.get("modules").is_none());
}

#[test]
fn aliases_and_informational_cesium_check() {
    let root = tempfile::tempdir().unwrap();
    let cesium = root.path().to_str().unwrap();
    let (output, report) = doctor(
        &[
            "--command",
            "glbTo3tz",
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
        serde_json::json!(["glb-to-3tz", "createTilesetJson", "preview"])
    );
    assert_eq!(report["commands"]["preview"]["cesium"]["found"], false);
    std::fs::write(root.path().join("Cesium.js"), "// invented runtime").unwrap();
    let (output, report) = doctor(&["--command", "preview", "--cesium", cesium], None);
    assert!(output.status.success(), "{report}");
    assert_eq!(report["commands"]["preview"]["cesium"]["path"], cesium);
    assert_eq!(report["commands"]["preview"]["cesium"]["found"], true);
}

/// Run a converter with `--json` and return its exit code and error category.
fn execute(args: &[&std::ffi::OsStr], env: &[(&str, &str)]) -> (Option<i32>, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .arg("--json")
        .args(args)
        .env("PATH", "")
        .envs(env.iter().copied())
        .output()
        .unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    (output.status.code(), result)
}

/// The GTiff-only native commands, each with a real fixture invocation.
fn gtiff_commands(root: &std::path::Path) -> Vec<(&'static str, Vec<std::ffi::OsString>)> {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let arg = |text: &str| std::ffi::OsString::from(text);
    let path = |path: std::path::PathBuf| path.into_os_string();
    vec![
        (
            "raster",
            vec![
                arg("raster"),
                arg("-i"),
                path(fixtures.join("d1-rgb.tif")),
                arg("-o"),
                path(root.join("raster")),
                arg("--maxZoom"),
                arg("0"),
            ],
        ),
        (
            "raster-tile-to-directory",
            vec![
                arg("raster-tile-to-directory"),
                arg("-i"),
                path(fixtures.join("d1-rgb.tif")),
                arg("-o"),
                path(root.join("d1")),
                arg("--zoom"),
                arg("3"),
                arg("--x"),
                arg("5"),
                arg("--y"),
                arg("2"),
            ],
        ),
        (
            "terrain",
            vec![
                arg("terrain"),
                arg("-i"),
                path(fixtures.join("t1-plane.tif")),
                arg("-o"),
                path(root.join("terrain")),
                arg("--height-offset"),
                arg("0"),
                arg("--fill-height"),
                arg("0"),
            ],
        ),
    ]
}

#[cfg(not(feature = "native-geospatial"))]
#[test]
fn portable_native_commands_are_unsupported_like_their_execution() {
    let root = tempfile::tempdir().unwrap();
    for (name, args) in gtiff_commands(root.path()) {
        let (output, report) = doctor(&["--command", name], None);
        assert_eq!(output.status.code(), Some(2), "{report}");
        assert_eq!(report["ok"], false);
        assert_eq!(report["error"]["code"], "unsupported");
        assert_eq!(report["status"], "unsupported");
        let readiness = &report["commands"][name];
        assert_eq!(readiness["status"], "unsupported", "{name}");
        assert_eq!(readiness["ready"], false);
        assert!(readiness["reason"]
            .as_str()
            .unwrap()
            .contains("--features native-geospatial"));
        assert_eq!(
            readiness["refusal"],
            json!({"code":"unsupported","exitCode":2})
        );
        // Execution in the same build refuses with the same category.
        let args: Vec<_> = args.iter().map(|a| a.as_os_str()).collect();
        let (code, result) = execute(&args, &[]);
        assert_eq!(code, Some(2), "{name}: {result}");
        assert_eq!(result["error"]["code"], "unsupported", "{name}");
    }
    // The human report uses the same status and category.
    let human = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["doctor", "--command", "terrain", "--command", "convert"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(2));
    let stdout = String::from_utf8(human.stdout).unwrap();
    assert!(stdout.contains("terrain: unsupported"), "{stdout}");
    assert!(stdout.contains("convert: ready"), "{stdout}");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

/// A vector conversion of the repository GeoJSON fixture.
#[cfg(feature = "native-geospatial")]
fn vector_command(root: &std::path::Path) -> Vec<std::ffi::OsString> {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vector.geojson");
    ["vector", "-i"]
        .map(std::ffi::OsString::from)
        .into_iter()
        .chain([
            fixture.into_os_string(),
            "-o".into(),
            root.join("vector.3tz").into_os_string(),
        ])
        .collect()
}

/// Each not-ready command reports the category and exit status that the
/// command's own run returns under the same missing dependency. Terrain and D1
/// are the recorded exception: their producers report a missing GTiff driver
/// as invalid_input (see the T1 contract), while doctor refuses it as
/// unsupported. This test pins both sides so either change is visible.
#[cfg(feature = "native-geospatial")]
#[test]
fn native_refusals_report_the_category_and_exit_of_execution() {
    let root = tempfile::tempdir().unwrap();
    let empty = root.path().join("empty-proj");
    std::fs::create_dir(&empty).unwrap();
    let empty = empty.to_str().unwrap();
    let no_proj = [("PROJ_DATA", empty), ("PROJ_LIB", empty)];
    type Case<'a> = (&'a [(&'a str, &'a str)], &'a [&'a str]);
    let cases: [Case; 5] = [
        (
            &[("GDAL_SKIP", "GTiff")],
            &["raster", "raster-tile-to-directory", "terrain"],
        ),
        (&[("GDAL_SKIP", "COG")], &["raster"]),
        (&[("GDAL_SKIP", "GPKG")], &["vector"]),
        (&[("GDAL_SKIP", "GeoJSON")], &["vector"]),
        (&no_proj, &["raster", "raster-tile-to-directory", "terrain"]),
    ];
    for (index, (env, names)) in cases.into_iter().enumerate() {
        let work = root.path().join(index.to_string());
        std::fs::create_dir(&work).unwrap();
        let mut commands = gtiff_commands(&work);
        commands.push(("vector", vector_command(&work)));
        for name in names {
            let os_env: Vec<_> = env
                .iter()
                .map(|(k, v)| (*k, std::ffi::OsStr::new(*v)))
                .collect();
            let (output, report) = doctor_with(&["--command", name], None, &os_env);
            let readiness = &report["commands"][*name];
            assert_eq!(
                readiness["status"], "unavailable",
                "{env:?} {name}: {report}"
            );
            assert_eq!(readiness["ready"], false);
            assert_eq!(report["error"]["code"], readiness["refusal"]["code"]);
            assert_eq!(
                output.status.code().map(Value::from),
                Some(readiness["refusal"]["exitCode"].clone())
            );
            let args = &commands
                .iter()
                .find(|(command, _)| command == name)
                .unwrap()
                .1;
            let args: Vec<_> = args.iter().map(|a| a.as_os_str()).collect();
            let (code, result) = execute(&args, env);
            if env == [("GDAL_SKIP", "GTiff")]
                && ["raster-tile-to-directory", "terrain"].contains(name)
            {
                // Recorded producer divergence: a failed GTiff open is invalid_input.
                assert_eq!(
                    (output.status.code(), &report["error"]["code"]),
                    (Some(2), &json!("unsupported"))
                );
                assert_eq!(
                    (code, &result["error"]["code"]),
                    (Some(3), &json!("invalid_input")),
                    "{name}: {result}"
                );
            } else {
                assert_eq!(code, output.status.code(), "{env:?} {name}: {result}");
                assert_eq!(
                    result["error"]["code"], report["error"]["code"],
                    "{env:?} {name}: {result}"
                );
            }
        }
    }
    // Native vector's own check needs no PROJ database; neither does a
    // WGS84 GeoJSON conversion. Doctor previously reported it not ready.
    let os_env: Vec<_> = no_proj
        .iter()
        .map(|(k, v)| (*k, std::ffi::OsStr::new(*v)))
        .collect();
    let (output, report) = doctor_with(&["--command", "vector"], None, &os_env);
    assert!(output.status.success(), "{report}");
    assert_eq!(report["commands"]["vector"]["status"], "ready");
    let args = vector_command(root.path());
    let args: Vec<_> = args.iter().map(|a| a.as_os_str()).collect();
    let (code, result) = execute(&args, &no_proj);
    assert_eq!(code, Some(0), "{result}");
}

#[test]
fn probes_are_named_and_limited_to_the_selection() {
    let names = |report: &Value| -> Vec<String> {
        report["probes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|probe| probe.as_str().unwrap().to_owned())
            .collect()
    };
    let top_level = [
        "linked-versions",
        "proj-offline-policy",
        "proj-database",
        "proj-directory-listing",
    ];
    let (output, report) = doctor(&["--command", "convert", "--command", "validate"], None);
    assert!(output.status.success(), "{report}");
    let probes = names(&report);
    assert!(
        probes.iter().all(|p| top_level.contains(&p.as_str())),
        "{probes:?}"
    );
    // The CLI always passes its documented --cesium default to preview.
    let root = tempfile::tempdir().unwrap();
    let cesium = root.path().to_str().unwrap();
    let (_, report) = doctor(&["--command", "preview", "--cesium", cesium], None);
    assert!(names(&report).contains(&"cesium-runtime".to_owned()));
    let (_, report) = doctor(&["--command", "vector"], None);
    let probes = names(&report);
    #[cfg(feature = "native-geospatial")]
    {
        assert!(
            probes.contains(&"gdal-driver-registry".to_owned()),
            "{probes:?}"
        );
        assert!(
            probes.contains(&"geos-triangulation".to_owned()),
            "{probes:?}"
        );
        assert!(probes.contains(&"proj-database".to_owned()), "{probes:?}");
    }
    #[cfg(not(feature = "native-geospatial"))]
    assert!(
        probes.iter().all(|p| p == "proj-directory-listing"),
        "{probes:?}"
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn doctor_writes_nothing_and_keeps_proj_networking_off() {
    let root = tempfile::tempdir().unwrap();
    let [home, cwd, writable] = ["home", "cwd", "proj-user"].map(|name| {
        let path = root.path().join(name);
        std::fs::create_dir(&path).unwrap();
        path
    });
    let env = [
        ("HOME", home.as_os_str()),
        ("XDG_DATA_HOME", home.as_os_str()),
        ("XDG_CACHE_HOME", home.as_os_str()),
        ("XDG_CONFIG_HOME", home.as_os_str()),
        ("PROJ_USER_WRITABLE_DIRECTORY", writable.as_os_str()),
        ("PROJ_NETWORK", std::ffi::OsStr::new("ON")),
    ];
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["doctor", "--json"])
        .current_dir(&cwd)
        .env("PATH", "")
        .envs(env)
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["proj"]["networkEnabled"], false);
    for directory in [&home, &cwd, &writable] {
        let entries: Vec<_> = std::fs::read_dir(directory).unwrap().collect();
        assert!(entries.is_empty(), "{directory:?}: {entries:?}");
    }
}
