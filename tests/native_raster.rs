//! Run the real converter without Python or a GDAL executable, also in the
//! GDAL 3.12 CI job which does not install development Python bindings.
mod support;

#[cfg(feature = "native-geospatial")]
#[test]
fn raster_cli_preserves_source_and_coverage_without_executables() {
    use std::{
        ffi::CString,
        process::Command,
        ptr::{null, null_mut},
    };
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("source with spaces.tif");
    let output = root.path().join("result with spaces");
    let filename = CString::new(input.as_os_str().as_encoded_bytes()).unwrap();
    let mut pixels = vec![0u8; 256 * 256];
    let mut mask = vec![255u8; 256 * 256];
    for y in 32..96 {
        for x in 32..96 {
            mask[y * 256 + x] = 0;
        }
    }
    // SAFETY: The fixture owns its dataset/SRS handles until their matching close.
    // Dimensions and IO buffers agree; all strings are terminated and live.
    unsafe {
        gdal_sys::GDALAllRegister();
        let driver = gdal_sys::GDALGetDriverByName(c"GTiff".as_ptr());
        assert!(!driver.is_null());
        // GDT_Byte / GDT_UInt8 have the stable C enum value 1.
        let dataset = gdal_sys::GDALCreate(driver, filename.as_ptr(), 256, 256, 1, 1, null_mut());
        assert!(!dataset.is_null());
        let srs = gdal_sys::OSRNewSpatialReference(null());
        assert_eq!(gdal_sys::OSRSetFromUserInput(srs, c"EPSG:4326".as_ptr()), 0);
        let mut wkt = null_mut();
        assert_eq!(gdal_sys::OSRExportToWkt(srs, &mut wkt), 0);
        assert_eq!(gdal_sys::GDALSetProjection(dataset, wkt), 0);
        gdal_sys::VSIFree(wkt.cast());
        gdal_sys::OSRDestroySpatialReference(srs);
        let mut gt = [12.5, 0.00001, 0., 41.9, 0., -0.00001];
        assert_eq!(gdal_sys::GDALSetGeoTransform(dataset, gt.as_mut_ptr()), 0);
        let band = gdal_sys::GDALGetRasterBand(dataset, 1);
        assert_eq!(
            gdal_sys::GDALRasterIO(
                band,
                gdal_sys::GDALRWFlag::GF_Write,
                0,
                0,
                256,
                256,
                pixels.as_mut_ptr().cast(),
                256,
                256,
                1,
                0,
                0
            ),
            0
        );
        assert_eq!(
            gdal_sys::GDALCreateMaskBand(band, 2 /* GMF_PER_DATASET */),
            0
        );
        assert_eq!(
            gdal_sys::GDALRasterIO(
                gdal_sys::GDALGetMaskBand(band),
                gdal_sys::GDALRWFlag::GF_Write,
                0,
                0,
                256,
                256,
                mask.as_mut_ptr().cast(),
                256,
                256,
                1,
                0,
                0
            ),
            0
        );
        assert_eq!(gdal_sys::GDALClose(dataset), 0);
    }
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["raster", "--json", "--progress", "json", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .args(["--minZoom", "16", "--maxZoom", "16"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["ok"], true);
    let progress: Vec<serde_json::Value> = String::from_utf8(result.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(progress.first().unwrap()["done"], 0);
    assert_eq!(progress.last().unwrap()["done"], 1);
    let gray_output = root.path().join("gray output");
    let gray = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["raster", "--json", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&gray_output)
        .args([
            "--minZoom",
            "16",
            "--maxZoom",
            "16",
            "--display",
            "gray",
            "--displayMin",
            "0",
            "--displayMax",
            "1",
        ])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        gray.status.success(),
        "{}",
        String::from_utf8_lossy(&gray.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&gray.stdout).unwrap()["ok"],
        true
    );
    for directory in [&output, &gray_output] {
        let mut opaque_black = false;
        let mut transparent = false;
        for entry in walkdir::WalkDir::new(directory.join("tiles")) {
            let entry = entry.unwrap();
            if entry.path().extension().is_some_and(|ext| ext == "png") {
                let image = image::open(entry.path()).unwrap().into_rgba8();
                assert_eq!(image.dimensions(), (256, 256));
                for p in image.pixels() {
                    opaque_black |= p.0 == [0, 0, 0, 255];
                    transparent |= p.0[3] == 0;
                }
            }
        }
        assert!(opaque_black && transparent);
    }
    let cog = CString::new(output.join("source.cog.tif").as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: Read-only owned dataset; buffers hold exactly one packed byte band.
    unsafe {
        let dataset = gdal_sys::GDALOpenEx(cog.as_ptr(), 0x02, null(), null(), null());
        assert!(!dataset.is_null());
        let band = gdal_sys::GDALGetRasterBand(dataset, 1);
        let mut actual = vec![255u8; 256 * 256];
        assert_eq!(
            gdal_sys::GDALRasterIO(
                band,
                gdal_sys::GDALRWFlag::GF_Read,
                0,
                0,
                256,
                256,
                actual.as_mut_ptr().cast(),
                256,
                256,
                1,
                0,
                0
            ),
            0
        );
        assert_eq!(actual, pixels);
        assert_eq!(
            gdal_sys::GDALRasterIO(
                gdal_sys::GDALGetMaskBand(band),
                gdal_sys::GDALRWFlag::GF_Read,
                0,
                0,
                256,
                256,
                actual.as_mut_ptr().cast(),
                256,
                256,
                1,
                0,
                0
            ),
            0
        );
        assert_eq!(actual, mask);
        assert_eq!(gdal_sys::GDALClose(dataset), 0);
    }
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 4);
    let readiness = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["doctor", "--command", "raster", "--json"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        readiness.status.success(),
        "{}",
        String::from_utf8_lossy(&readiness.stderr)
    );
    let readiness: serde_json::Value = serde_json::from_slice(&readiness.stdout).unwrap();
    assert_eq!(readiness["commands"]["raster"]["backend"], "native GDAL");
    assert_eq!(readiness["commands"]["raster"]["tiling"]["ready"], true);
}

#[cfg(not(feature = "native-geospatial"))]
#[test]
fn default_build_reports_native_raster_requirement() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("fixture.tif");
    let output = root.path().join("out");
    std::fs::write(&input, b"fixture").unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["raster", "--json", "-i"])
        .arg(input)
        .arg("-o")
        .arg(&output)
        .args(["--maxZoom", "0"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(4));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["error"]["code"], "environment");
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("native-geospatial"));
    assert!(!output.exists());
}

#[cfg(feature = "native-geospatial")]
#[test]
fn force_replaces_only_successful_output() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("source.asc");
    std::fs::write(
        &input,
        "ncols 2\nnrows 2\nxllcorner 12\nyllcorner 41\ncellsize 0.1\n1 1\n1 1\n",
    )
    .unwrap();
    std::fs::write(root.path().join("source.prj"), r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#).unwrap();
    support::force_replaces_only_successful_output(
        "raster",
        &input,
        &[
            "--maxZoom",
            "0",
            "--display",
            "gray",
            "--displayMin",
            "0",
            "--displayMax",
            "1",
        ],
        true,
    );
}
