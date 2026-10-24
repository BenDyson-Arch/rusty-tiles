//! Actual CLI Replace policy and machine result parity for the raster pilot.
use std::{fs, process::Command};
#[test]
fn explicit_force_routes_same_directory_consumer_policy() {
    #[cfg(target_os = "linux")]
    let root = tempfile::tempdir_in("/dev/shm").unwrap();
    #[cfg(not(target_os = "linux"))]
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("source.tif");
    fs::write(&input, include_bytes!("fixtures/d1-rgb.tif")).unwrap();
    let output = root.path().join("output");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("old"), b"original").unwrap();
    let run = |force: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
        command
            .args(["--json", "raster-tile-to-directory", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .args(["--zoom", "3", "--x", "5", "--y", "2"]);
        if force {
            command.arg("--force");
        }
        command.output().unwrap()
    };
    let initial = run(false);
    assert!(!initial.status.success());
    let failed: serde_json::Value = serde_json::from_slice(&initial.stdout).unwrap();
    assert!(failed["error"]["recovery"].is_null());
    assert_eq!(fs::read(output.join("old")).unwrap(), b"original");
    let forced = run(true);
    let summary: serde_json::Value = serde_json::from_slice(&forced.stdout).unwrap();
    #[cfg(feature = "native-geospatial")]
    {
        assert_eq!(failed["error"]["kind"], "output_conflict");
        assert!(
            forced.status.success(),
            "{}",
            String::from_utf8_lossy(&forced.stderr)
        );
        assert_eq!(summary["ok"], true);
        assert_eq!(summary["rasterReport"]["profile"], "d1-web-mercator-rgb");
        assert_eq!(summary["cleanupDiagnostics"], serde_json::json!([]));
        assert!(!output.join("old").exists());
        assert_eq!(
            image::open(output.join("tiles/3/5/2.png"))
                .unwrap()
                .to_rgb8()
                .get_pixel(5, 2)
                .0,
            [5, 2, 25]
        );
    }
    #[cfg(not(feature = "native-geospatial"))]
    {
        assert!(!forced.status.success());
        assert_eq!(failed["error"]["kind"], "unsupported");
        assert_eq!(summary["error"]["kind"], "unsupported");
        assert!(summary["error"]["recovery"].is_null());
        assert_eq!(fs::read(output.join("old")).unwrap(), b"original");
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}
