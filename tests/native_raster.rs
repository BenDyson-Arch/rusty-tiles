//! Run the real converter without Python or a GDAL executable, also in the
//! GDAL 3.12 CI job which does not install development Python bindings.
mod support;

#[cfg(feature = "native-geospatial")]
fn register_gdal() {
    static REGISTER: std::sync::Once = std::sync::Once::new();
    // SAFETY: Driver registration must finish once before either fixture starts
    // using GDAL; the Rust test runner constructs these fixtures in parallel.
    REGISTER.call_once(|| unsafe { gdal_sys::GDALAllRegister() });
}

#[cfg(feature = "native-geospatial")]
#[test]
fn gray_int16_alpha_intersects_mask_and_nodata_without_executables() {
    use std::{
        ffi::CString,
        process::Command,
        ptr::{null, null_mut},
    };
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("rendered survey.tif");
    // Cross a 256-pixel window boundary in both dimensions.
    let (width, height) = (257usize, 259usize);
    let mut gray = vec![128i16; width * height];
    let mut alpha = vec![255i16; width * height];
    let mut mask = vec![255u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            if x < 64 && y < 64 {
                gray[i] = 0;
                alpha[i] = 0;
            }
            if (64..128).contains(&x) && y < 64 {
                alpha[i] = 128;
            }
            if (128..192).contains(&x) && y < 64 {
                gray[i] = 0;
            }
            if x < 64 && (128..192).contains(&y) {
                gray[i] = -32768;
            }
            if (128..192).contains(&x) && (128..192).contains(&y) {
                mask[i] = 0;
            }
            if y >= 256 {
                gray[i] = 220;
                alpha[i] = 64;
            }
            if x == 256 {
                alpha[i] = 0;
            }
        }
    }
    let filename = CString::new(input.as_os_str().as_encoded_bytes()).unwrap();
    register_gdal();
    // SAFETY: Owned dataset/SRS handles and live terminated strings; all IO
    // buffers match the dataset dimensions and requested Int16/Byte types.
    unsafe {
        let dataset = gdal_sys::GDALCreate(
            gdal_sys::GDALGetDriverByName(c"GTiff".as_ptr()),
            filename.as_ptr(),
            width as i32,
            height as i32,
            4,
            gdal_sys::GDALDataType::GDT_Int16,
            null_mut(),
        );
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
        for index in 1..=4 {
            let band = gdal_sys::GDALGetRasterBand(dataset, index);
            let pixels = if index == 4 { &mut alpha } else { &mut gray };
            assert_eq!(gdal_sys::GDALSetRasterNoDataValue(band, -32768.), 0);
            assert_eq!(
                gdal_sys::GDALRasterIO(
                    band,
                    gdal_sys::GDALRWFlag::GF_Write,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    pixels.as_mut_ptr().cast(),
                    width as i32,
                    height as i32,
                    gdal_sys::GDALDataType::GDT_Int16,
                    0,
                    0
                ),
                0
            );
        }
        assert_eq!(
            gdal_sys::GDALSetRasterColorInterpretation(
                gdal_sys::GDALGetRasterBand(dataset, 4),
                gdal_sys::GDALColorInterp::GCI_AlphaBand
            ),
            0
        );
        let band = gdal_sys::GDALGetRasterBand(dataset, 1);
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
                width as i32,
                height as i32,
                mask.as_mut_ptr().cast(),
                width as i32,
                height as i32,
                1,
                0,
                0
            ),
            0
        );
        assert_eq!(gdal_sys::GDALClose(dataset), 0);
    }
    let original = std::fs::read(&input).unwrap();
    let output = work.path().join("gray with alpha");
    let run = |output: &std::path::Path, alpha_band: &str, force: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
        command
            .args(["raster", "--json", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(output)
            .args([
                "--minZoom",
                "16",
                "--maxZoom",
                "16",
                "--display",
                "gray",
                "--band",
                "1",
                "--displayMin",
                "0",
                "--displayMax",
                "255",
                "--alphaBand",
                alpha_band,
            ])
            .env("PATH", "");
        if force {
            command.arg("--force");
        }
        command.output().unwrap()
    };
    let result = run(&output, "4", false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["ok"], true);
    let recipe: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("report.json")).unwrap()).unwrap();
    assert_eq!(recipe["display"]["alpha_band"], 4);
    assert_eq!(recipe, report["rasterReport"]);
    assert_eq!(recipe["profile"], "r2-source-cog-direct-nearest-pyramid");
    let sample = |x: usize, y: usize| {
        let lon = 12.5 + (x as f64 + 0.5) * 0.00001;
        let lat = 41.9 - (y as f64 + 0.5) * 0.00001;
        let tx = (lon + 180.) / 360. * 65536.;
        let ty = (1. - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2. * 65536.;
        let path = output.join(format!(
            "tiles/16/{}/{}.png",
            tx.floor() as u32,
            ty.floor() as u32
        ));
        let image = image::open(path).unwrap().into_rgba8();
        image
            .get_pixel((tx.fract() * 256.) as u32, (ty.fract() * 256.) as u32)
            .0
    };
    assert_eq!(sample(32, 32)[3], 0, "explicit alpha-0 surround");
    assert_eq!(sample(96, 32)[3], 128, "partial opacity is not scaled");
    assert_eq!(
        sample(160, 32),
        [0, 0, 0, 255],
        "valid black remains opaque"
    );
    assert_eq!(sample(32, 160)[3], 0, "NoData with opaque alpha");
    assert_eq!(sample(160, 160)[3], 0, "masked data with opaque alpha");
    assert_eq!(sample(224, 224), [128, 128, 128, 255]);
    assert_eq!(
        sample(224, 257),
        [220, 220, 220, 64],
        "display reaches the final source window"
    );
    assert_eq!(std::fs::read(&input).unwrap(), original);
    // COG keeps every Int16 band and the dataset mask unchanged.
    let cog = CString::new(output.join("source.cog.tif").as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: Read-only owned dataset and precisely sized typed buffers.
    unsafe {
        let dataset = gdal_sys::GDALOpenEx(cog.as_ptr(), 0x02, null(), null(), null());
        assert!(!dataset.is_null());
        assert_eq!(gdal_sys::GDALGetRasterCount(dataset), 4);
        for index in 1..=4 {
            let band = gdal_sys::GDALGetRasterBand(dataset, index);
            assert_eq!(
                gdal_sys::GDALGetRasterDataType(band),
                gdal_sys::GDALDataType::GDT_Int16
            );
            let mut actual = vec![0i16; width * height];
            assert_eq!(
                gdal_sys::GDALRasterIO(
                    band,
                    gdal_sys::GDALRWFlag::GF_Read,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    actual.as_mut_ptr().cast(),
                    width as i32,
                    height as i32,
                    gdal_sys::GDALDataType::GDT_Int16,
                    0,
                    0
                ),
                0
            );
            assert_eq!(actual, if index == 4 { &alpha } else { &gray }.as_slice());
        }
        let mut actual = vec![0u8; width * height];
        assert_eq!(
            gdal_sys::GDALRasterIO(
                gdal_sys::GDALGetMaskBand(gdal_sys::GDALGetRasterBand(dataset, 1)),
                gdal_sys::GDALRWFlag::GF_Read,
                0,
                0,
                width as i32,
                height as i32,
                actual.as_mut_ptr().cast(),
                width as i32,
                height as i32,
                1,
                0,
                0
            ),
            0
        );
        assert_eq!(actual, mask);
        assert_eq!(gdal_sys::GDALClose(dataset), 0);
    }
    let before = std::fs::read(output.join("tilejson.json")).unwrap();
    let result = run(&output, "5", true);
    assert_eq!(result.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&result.stdout).contains("alpha band index"));
    assert_eq!(std::fs::read(output.join("tilejson.json")).unwrap(), before);
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 4);
}

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
    register_gdal();
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
    let completed: Vec<_> = progress
        .iter()
        .filter(|event| event["phase"] == "raster_complete")
        .collect();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0]["done"], report["rasterReport"]["tiles"]);
    assert_eq!(completed[0]["total"], report["rasterReport"]["tiles"]);
    assert_eq!(progress.last().unwrap()["phase"], "conversion");
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
    assert_eq!(result.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["error"]["code"], "unsupported");
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
