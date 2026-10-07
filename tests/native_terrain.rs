use serde_json::Value;
use std::{path::Path, process::Command};

fn source(root: &Path) -> std::path::PathBuf {
    let path = root.join("dem.asc");
    std::fs::write(&path,"ncols 2\nnrows 2\nxllcorner 12\nyllcorner 41\ncellsize 0.1\nNODATA_value -9999\n100 110\n120 -9999\n").unwrap();
    std::fs::write(root.join("dem.prj"),r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#).unwrap();
    path
}
fn convert(input: &Path, output: &Path, extra: &[&str]) -> (std::process::Output, Value) {
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["--json", "terrain", "-i"])
        .arg(input)
        .arg("-o")
        .arg(output)
        .args([
            "--maxZoom",
            "9",
            "--grid",
            "17",
            "--heightOffset",
            "10",
            "--fillHeight",
            "-999",
        ])
        .args(extra)
        .env("PATH", "")
        .output()
        .unwrap();
    let report = serde_json::from_slice(&result.stdout).unwrap();
    (result, report)
}

#[test]
fn terrain_validation_and_readiness_without_python() {
    let root = tempfile::tempdir().unwrap();
    let input = source(root.path());
    let output = root.path().join("tiles");
    let (result, report) = convert(&input, &output, &["--grid", "18"]);
    // Clap rejects repeated options as usage before the converter can run.
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(report["error"]["code"], "usage");
    let (result, report) = convert(&input, &output, &[]);
    #[cfg(not(feature = "native-geospatial"))]
    {
        assert_eq!(result.status.code(), Some(4));
        assert_eq!(report["error"]["code"], "environment");
        assert!(report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("native-geospatial"));
        assert!(!output.exists());
    }
    #[cfg(feature = "native-geospatial")]
    {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(report["ok"], true);
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(output.join("layer.json")).unwrap()).unwrap();
        assert_eq!(manifest["heightOverlay"]["rowOrder"], "south-to-north");
        assert_eq!(manifest["available"][0][0]["endX"], 1);
        let bytes = std::fs::read(output.join("0/0/0.terrain")).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[88..92].try_into().unwrap()), 289);
        assert_eq!(f32::from_le_bytes(bytes[24..28].try_into().unwrap()), -999.);
        assert_eq!(f32::from_le_bytes(bytes[28..32].try_into().unwrap()), 130.);
        let (conflict, report) = convert(&input, &output, &[]);
        assert_eq!(conflict.status.code(), Some(5));
        assert_eq!(report["error"]["code"], "output_conflict");
    }
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["doctor", "--json", "--command", "terrain"])
        .env("PATH", "")
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        report["commands"]["terrain"]["ready"],
        cfg!(feature = "native-geospatial")
    );
}

#[cfg(feature = "native-geospatial")]
#[test]
fn resource_limit_and_bad_source_preserve_existing_directory() {
    let root = tempfile::tempdir().unwrap();
    let input = source(root.path());
    let output = root.path().join("tiles");
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("sentinel"), "keep").unwrap();
    let options = rusty_tiles::terrain::TerrainOptions {
        force: true,
        max_zoom: 24,
        grid: 129,
        height_offset: 0.,
        fill_height: 0.,
        max_error: 1.,
    };
    let error = rusty_tiles::terrain::dem_to_terrain(&input, &output, &options).unwrap_err();
    assert_eq!(error.category(), ("data", 3));
    assert!(error.to_string().contains("tile count"));
    std::fs::write(&input, "invalid DEM").unwrap();
    let error = rusty_tiles::terrain::dem_to_terrain(&input, &output, &options).unwrap_err();
    assert_eq!(error.category(), ("data", 3));
    assert_eq!(
        std::fs::read_to_string(output.join("sentinel")).unwrap(),
        "keep"
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 3);
}
