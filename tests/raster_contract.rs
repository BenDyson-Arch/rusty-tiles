//! R2 contract claims executed through the built CLI against real GDAL:
//! COG metadata preservation, the Undefined->Gray role migration, the six
//! `--max-*` caps and nonfinite gray refusal. The converter always runs in a
//! child process; this binary registers GDAL once, only for its fixtures.
#![cfg(feature = "native-geospatial")]
use serde_json::Value;
use std::{
    ffi::{CStr, CString},
    path::Path,
    process::{Command, Output},
    ptr::{null, null_mut},
};

fn register_gdal() {
    static REGISTER: std::sync::Once = std::sync::Once::new();
    // SAFETY: Driver registration finishes once before any fixture uses GDAL;
    // the test runner builds these fixtures in parallel.
    REGISTER.call_once(|| unsafe { gdal_sys::GDALAllRegister() });
}

/// GDT_Byte (GDT_UInt8 since GDAL 3.11) has the stable C enum value 1.
const BYTE: u32 = 1;

fn c_path(path: &Path) -> CString {
    CString::new(path.as_os_str().as_encoded_bytes()).unwrap()
}

/// One band of a GTiff fixture; `samples` holds native-endian typed values.
#[derive(Default)]
struct BandSpec<'a> {
    samples: Vec<u8>,
    nodata: Option<f64>,
    scale: Option<f64>,
    offset: Option<f64>,
    unit: Option<&'a CStr>,
    interp: Option<u32>,
    palette: Option<&'a [[i16; 4]]>,
}

struct TiffSpec<'a> {
    width: usize,
    height: usize,
    datatype: u32,
    epsg: i32,
    affine: [f64; 6],
    area_or_point: Option<&'a CStr>,
    bands: Vec<BandSpec<'a>>,
    /// Per-dataset Byte mask; zero means invalid.
    mask: Option<Vec<u8>>,
}

fn write_tiff(path: &Path, spec: TiffSpec<'_>) {
    register_gdal();
    let filename = c_path(path);
    let (w, h) = (spec.width as i32, spec.height as i32);
    // SAFETY: Owned dataset/SRS/color-table handles are released here; every
    // IO buffer holds exactly width*height samples of the dataset type and all
    // strings are terminated and live across their calls.
    unsafe {
        let ds = gdal_sys::GDALCreate(
            gdal_sys::GDALGetDriverByName(c"GTiff".as_ptr()),
            filename.as_ptr(),
            w,
            h,
            spec.bands.len() as i32,
            spec.datatype,
            null_mut(),
        );
        assert!(!ds.is_null());
        let srs = gdal_sys::OSRNewSpatialReference(null());
        assert_eq!(gdal_sys::OSRImportFromEPSG(srs, spec.epsg), 0);
        assert_eq!(gdal_sys::GDALSetSpatialRef(ds, srs), 0);
        gdal_sys::OSRDestroySpatialReference(srs);
        let mut affine = spec.affine;
        assert_eq!(gdal_sys::GDALSetGeoTransform(ds, affine.as_mut_ptr()), 0);
        if let Some(value) = spec.area_or_point {
            assert_eq!(
                gdal_sys::GDALSetMetadataItem(
                    ds,
                    c"AREA_OR_POINT".as_ptr(),
                    value.as_ptr(),
                    null()
                ),
                0
            );
        }
        let bytes = gdal_sys::GDALGetDataTypeSizeBytes(spec.datatype) as usize;
        for (index, mut band_spec) in spec.bands.into_iter().enumerate() {
            let band = gdal_sys::GDALGetRasterBand(ds, index as i32 + 1);
            assert_eq!(band_spec.samples.len(), spec.width * spec.height * bytes);
            if let Some(v) = band_spec.nodata {
                assert_eq!(gdal_sys::GDALSetRasterNoDataValue(band, v), 0);
            }
            if let Some(v) = band_spec.scale {
                assert_eq!(gdal_sys::GDALSetRasterScale(band, v), 0);
            }
            if let Some(v) = band_spec.offset {
                assert_eq!(gdal_sys::GDALSetRasterOffset(band, v), 0);
            }
            if let Some(v) = band_spec.unit {
                assert_eq!(gdal_sys::GDALSetRasterUnitType(band, v.as_ptr()), 0);
            }
            if let Some(v) = band_spec.interp {
                assert_eq!(gdal_sys::GDALSetRasterColorInterpretation(band, v), 0);
            }
            if let Some(entries) = band_spec.palette {
                let table = gdal_sys::GDALCreateColorTable(gdal_sys::GDALPaletteInterp::GPI_RGB);
                for (i, e) in entries.iter().enumerate() {
                    let entry = gdal_sys::GDALColorEntry {
                        c1: e[0],
                        c2: e[1],
                        c3: e[2],
                        c4: e[3],
                    };
                    gdal_sys::GDALSetColorEntry(table, i as i32, &entry);
                }
                assert_eq!(gdal_sys::GDALSetRasterColorTable(band, table), 0);
                gdal_sys::GDALDestroyColorTable(table);
            }
            assert_eq!(
                gdal_sys::GDALRasterIO(
                    band,
                    gdal_sys::GDALRWFlag::GF_Write,
                    0,
                    0,
                    w,
                    h,
                    band_spec.samples.as_mut_ptr().cast(),
                    w,
                    h,
                    spec.datatype,
                    0,
                    0
                ),
                0
            );
        }
        if let Some(mut mask) = spec.mask {
            let band = gdal_sys::GDALGetRasterBand(ds, 1);
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
                    w,
                    h,
                    mask.as_mut_ptr().cast(),
                    w,
                    h,
                    1,
                    0,
                    0
                ),
                0
            );
        }
        assert_eq!(gdal_sys::GDALClose(ds), 0);
    }
}

/// Critical facts as GDAL reads them; floats are compared by exact bits.
#[derive(Debug, PartialEq)]
struct BandFacts {
    datatype: u32,
    nodata: Option<u64>,
    scale: Option<u64>,
    offset: Option<u64>,
    unit: String,
    interp: u32,
    palette: Vec<[i16; 4]>,
    mask_flags: i32,
    samples: Samples,
}
/// Exact typed sample bytes; failures print a length and FNV-1a digest.
#[derive(PartialEq)]
struct Samples(Vec<u8>);
impl std::fmt::Debug for Samples {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let digest = self.0.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
        });
        write!(f, "{} bytes fnv1a {digest:016x}", self.0.len())
    }
}
#[derive(Debug, PartialEq)]
struct Facts {
    size: [i32; 2],
    affine: [u64; 6],
    wkt: String,
    epsg: Option<String>,
    area_or_point: Option<String>,
    bands: Vec<BandFacts>,
}

fn text(p: *const std::ffi::c_char) -> Option<String> {
    // SAFETY: GDAL returns null or a terminated string valid until the next
    // call on the same object; it is copied immediately.
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_owned())
}

fn read_facts(path: &Path) -> Facts {
    register_gdal();
    let filename = c_path(path);
    // SAFETY: Read-only owned dataset closed at the end; buffers hold exactly
    // width*height samples of each band's actual type.
    unsafe {
        let ds = gdal_sys::GDALOpenEx(filename.as_ptr(), 0x02, null(), null(), null());
        assert!(!ds.is_null(), "{}", path.display());
        let size = [
            gdal_sys::GDALGetRasterXSize(ds),
            gdal_sys::GDALGetRasterYSize(ds),
        ];
        let mut affine = [0.; 6];
        assert_eq!(gdal_sys::GDALGetGeoTransform(ds, affine.as_mut_ptr()), 0);
        let srs = gdal_sys::GDALGetSpatialRef(ds);
        assert!(!srs.is_null());
        let epsg = text(gdal_sys::OSRGetAuthorityCode(srs, null()));
        let mut bands = Vec::new();
        for index in 1..=gdal_sys::GDALGetRasterCount(ds) {
            let band = gdal_sys::GDALGetRasterBand(ds, index);
            let scalar = |get: unsafe extern "C" fn(gdal_sys::GDALRasterBandH, *mut i32) -> f64| {
                let mut present = 0;
                let value = get(band, &mut present);
                (present != 0).then_some(value.to_bits())
            };
            let datatype = gdal_sys::GDALGetRasterDataType(band);
            let table = gdal_sys::GDALGetRasterColorTable(band);
            let palette = if table.is_null() {
                Vec::new()
            } else {
                (0..gdal_sys::GDALGetColorEntryCount(table))
                    .map(|i| {
                        let e = &*gdal_sys::GDALGetColorEntry(table, i);
                        [e.c1, e.c2, e.c3, e.c4]
                    })
                    .collect()
            };
            let bytes = gdal_sys::GDALGetDataTypeSizeBytes(datatype) as usize;
            let mut samples = vec![0u8; size[0] as usize * size[1] as usize * bytes];
            assert_eq!(
                gdal_sys::GDALRasterIO(
                    band,
                    gdal_sys::GDALRWFlag::GF_Read,
                    0,
                    0,
                    size[0],
                    size[1],
                    samples.as_mut_ptr().cast(),
                    size[0],
                    size[1],
                    datatype,
                    0,
                    0
                ),
                0
            );
            bands.push(BandFacts {
                datatype,
                nodata: scalar(gdal_sys::GDALGetRasterNoDataValue),
                scale: scalar(gdal_sys::GDALGetRasterScale),
                offset: scalar(gdal_sys::GDALGetRasterOffset),
                unit: text(gdal_sys::GDALGetRasterUnitType(band)).unwrap_or_default(),
                interp: gdal_sys::GDALGetRasterColorInterpretation(band),
                palette,
                mask_flags: gdal_sys::GDALGetMaskFlags(band),
                samples: Samples(samples),
            });
        }
        let facts = Facts {
            size,
            affine: affine.map(f64::to_bits),
            wkt: text(gdal_sys::GDALGetProjectionRef(ds)).unwrap(),
            epsg,
            area_or_point: text(gdal_sys::GDALGetMetadataItem(
                ds,
                c"AREA_OR_POINT".as_ptr(),
                null(),
            )),
            bands,
        };
        assert_eq!(gdal_sys::GDALClose(ds), 0);
        facts
    }
}

fn raster(input: &Path, output: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["raster", "--json", "-i"])
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(args)
        .env("PATH", "")
        .output()
        .unwrap()
}

/// Asserts success and returns the published report, equal to the summary's.
fn published(result: &Output, output: &Path) -> Value {
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
    let published: Value =
        serde_json::from_slice(&std::fs::read(output.join("report.json")).unwrap()).unwrap();
    assert_eq!(summary["rasterReport"], published);
    published
}

/// Asserts a refusal with the given category/status and no output; returns
/// the error message.
fn refusal(result: &Output, output: &Path, code: &str, status: i32) -> String {
    let summary: Value = serde_json::from_slice(&result.stdout).unwrap_or_else(|_| {
        panic!(
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )
    });
    assert_eq!(result.status.code(), Some(status), "{summary}");
    assert_eq!(summary["error"]["code"], code, "{summary}");
    assert!(!output.exists(), "refused raster must not publish output");
    summary["error"]["message"].as_str().unwrap().to_owned()
}

fn f32_samples(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

/// Typed Int16 bands with distinct scale/offset/unit, shared NoData, Point
/// registration and EPSG:3857 survive into the COG exactly as GDAL reads them
/// from the source. Multiband/defined roles receive no migration.
#[test]
fn cog_preserves_typed_metadata_point_affine_and_crs() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("typed.tif");
    let (width, height) = (64usize, 48usize);
    let affine = [1_234_567.25, 30.5, 0., 6_543_210.75, 0., -30.5];
    let band = |seed: i16| -> Vec<u8> {
        (0..width * height)
            .map(|i| {
                if i % 97 == 5 {
                    -9999
                } else {
                    seed + (i % 61) as i16
                }
            })
            .flat_map(i16::to_ne_bytes)
            .collect()
    };
    write_tiff(
        &input,
        TiffSpec {
            width,
            height,
            datatype: gdal_sys::GDALDataType::GDT_Int16,
            epsg: 3857,
            affine,
            area_or_point: Some(c"Point"),
            bands: vec![
                BandSpec {
                    samples: band(-30),
                    nodata: Some(-9999.),
                    scale: Some(0.25),
                    offset: Some(-10.5),
                    unit: Some(c"m"),
                    ..BandSpec::default()
                },
                BandSpec {
                    samples: band(7),
                    nodata: Some(-9999.),
                    scale: Some(2.),
                    offset: Some(0.125),
                    unit: Some(c"ft"),
                    ..BandSpec::default()
                },
            ],
            mask: None,
        },
    );
    // Every source fact is internal to the TIFF: no PAM sidecar.
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    let source = read_facts(&input);
    // Independent literal reference for what GDAL reads from the source.
    assert_eq!(source.affine, affine.map(f64::to_bits));
    assert_eq!(source.area_or_point.as_deref(), Some("Point"));
    assert_eq!(source.epsg.as_deref(), Some("3857"));
    let literal = [(0.25, -10.5, "m"), (2., 0.125, "ft")];
    for (band, (scale, offset, unit)) in source.bands.iter().zip(literal) {
        assert_eq!(band.datatype, gdal_sys::GDALDataType::GDT_Int16);
        assert_eq!(band.nodata, Some((-9999f64).to_bits()));
        assert_eq!(band.scale, Some(f64::to_bits(scale)));
        assert_eq!(band.offset, Some(f64::to_bits(offset)));
        assert_eq!(band.unit, unit);
    }
    assert_eq!(
        source.bands.iter().map(|b| b.interp).collect::<Vec<_>>(),
        [1, 0],
        "fixture roles: Gray then Undefined"
    );
    let output = root.path().join("out");
    let result = raster(
        &input,
        &output,
        &[
            "--minZoom",
            "12",
            "--maxZoom",
            "12",
            "--display",
            "gray",
            "--displayMin",
            "-30",
            "--displayMax",
            "70",
        ],
    );
    let report = published(&result, &output);
    assert_eq!(read_facts(&output.join("source.cog.tif")), source);
    assert_eq!(report["source"]["area_point"], "Point");
    assert_eq!(report["source"]["epsg"], 3857);
    assert_eq!(
        report["grid"]["source_affine"],
        serde_json::json!(affine),
        "report names the exact source affine"
    );
    // Multiband with a defined first role: no migration, even though band 2
    // is Undefined.
    assert_eq!(report["color_role_migration"], "exact");
    assert_eq!(report["source_color_interp"], 1);
    assert_eq!(report["target_color_interp"], 1);
}

/// A paletted Byte source keeps its opaque palette, rotated affine, Area and
/// CRS in the COG; a palette role is not migrated.
#[test]
fn cog_preserves_palette_rotated_affine_and_area() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("palette.tif");
    let (width, height) = (40usize, 24usize);
    let affine = [12.5, 1e-4, 2e-6, 41.9, 3e-6, -1e-4];
    let palette = [
        [10, 20, 30, 255],
        [200, 0, 5, 255],
        [0, 255, 0, 255],
        [7, 8, 9, 255],
        [255, 255, 255, 255],
    ];
    write_tiff(
        &input,
        TiffSpec {
            width,
            height,
            datatype: BYTE,
            epsg: 4326,
            affine,
            area_or_point: Some(c"Area"),
            bands: vec![BandSpec {
                samples: (0..width * height).map(|i| (i % 5) as u8).collect(),
                interp: Some(gdal_sys::GDALColorInterp::GCI_PaletteIndex),
                palette: Some(&palette),
                ..BandSpec::default()
            }],
            mask: None,
        },
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    let source = read_facts(&input);
    assert_eq!(source.affine, affine.map(f64::to_bits));
    assert_eq!(source.area_or_point.as_deref(), Some("Area"));
    assert_eq!(source.epsg.as_deref(), Some("4326"));
    let band = &source.bands[0];
    assert_eq!(band.interp, gdal_sys::GDALColorInterp::GCI_PaletteIndex);
    assert_eq!(band.palette[..5], palette);
    assert_eq!(band.nodata, None);
    let output = root.path().join("out");
    let report = published(
        &raster(&input, &output, &["--minZoom", "15", "--maxZoom", "15"]),
        &output,
    );
    let cog = read_facts(&output.join("source.cog.tif"));
    assert_eq!(cog.bands[0].palette[..5], palette);
    assert_eq!(cog, source);
    assert_eq!(report["source"]["area_point"], "Area");
    assert_eq!(report["color_role_migration"], "exact");
    assert_eq!(report["source_color_interp"], 2);
    assert_eq!(report["target_color_interp"], 2);
}

/// Only one ordinary band with Undefined interpretation becomes Gray; the
/// report keeps the original role and names the migration. A single band
/// with a defined Gray role is reported as exact.
#[test]
fn single_undefined_band_migrates_to_gray_and_defined_role_does_not() {
    let root = tempfile::tempdir().unwrap();
    // AAIGrid bands have no defined color role.
    let input = root.path().join("undefined.asc");
    std::fs::write(
        &input,
        "ncols 4\nnrows 3\nxllcorner 12\nyllcorner 41\ncellsize 0.001\nNODATA_value -1\n\
         0 1 2 3\n4 -1 6 7\n8 9 10 11\n",
    )
    .unwrap();
    std::fs::write(root.path().join("undefined.prj"), r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#).unwrap();
    let gray = [
        "--minZoom",
        "10",
        "--maxZoom",
        "10",
        "--display",
        "gray",
        "--displayMin",
        "0",
        "--displayMax",
        "11",
    ];
    let source = read_facts(&input);
    assert_eq!(source.bands.len(), 1);
    assert_eq!(
        source.bands[0].interp,
        gdal_sys::GDALColorInterp::GCI_Undefined
    );
    let output = root.path().join("migrated");
    let report = published(&raster(&input, &output, &gray), &output);
    assert_eq!(report["color_role_migration"], "single_undefined_gray");
    assert_eq!(report["source_color_interp"], 0);
    assert_eq!(report["target_color_interp"], 1);
    assert_eq!(report["source"]["bands"][0]["color_interp"], 0);
    let cog = read_facts(&output.join("source.cog.tif"));
    assert_eq!(
        cog.bands[0].interp,
        gdal_sys::GDALColorInterp::GCI_GrayIndex
    );
    // Everything else about the band is unchanged.
    let mut expected = source.bands;
    expected[0].interp = gdal_sys::GDALColorInterp::GCI_GrayIndex;
    assert_eq!(cog.bands, expected);
    assert_eq!(cog.affine, source.affine);
    assert_eq!(cog.epsg.as_deref(), Some("4326"));

    // Control: one real band with a defined Gray role.
    let input = root.path().join("defined.tif");
    write_tiff(
        &input,
        TiffSpec {
            width: 4,
            height: 3,
            datatype: gdal_sys::GDALDataType::GDT_Float32,
            epsg: 4326,
            affine: [12., 0.001, 0., 41.003, 0., -0.001],
            area_or_point: None,
            bands: vec![BandSpec {
                samples: f32_samples(&[0.5; 12]),
                ..BandSpec::default()
            }],
            mask: None,
        },
    );
    assert_eq!(
        read_facts(&input).bands[0].interp,
        gdal_sys::GDALColorInterp::GCI_GrayIndex
    );
    let output = root.path().join("defined");
    let report = published(&raster(&input, &output, &gray), &output);
    assert_eq!(report["color_role_migration"], "exact");
    assert_eq!(report["source_color_interp"], 1);
    assert_eq!(report["target_color_interp"], 1);
}

fn files_bytes(directory: &Path) -> u64 {
    walkdir::WalkDir::new(directory)
        .into_iter()
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().unwrap().len())
        .sum()
}

/// Every `--max-*` flag reaches its own `RasterLimits` field: each run passes
/// with all six caps at the actual need and refuses with exactly one cap
/// below it. The logical working ledger is not published, so that cap uses a
/// value below the one 256x256 RGBA tile the ledger charges against one far
/// above the run; every other need is exact.
#[test]
fn six_cli_caps_pass_at_need_and_refuse_one_below() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("rgb.tif");
    let (width, height) = (40usize, 30usize);
    write_tiff(
        &input,
        TiffSpec {
            width,
            height,
            datatype: BYTE,
            epsg: 4326,
            affine: [12.5, 1e-4, 0., 41.9, 0., -1e-4],
            area_or_point: None,
            bands: (0..3u8)
                .map(|band| BandSpec {
                    samples: (0..width * height)
                        .map(|i| (i as u8).wrapping_mul(band + 1))
                        .collect(),
                    interp: Some(3 + u32::from(band)),
                    ..BandSpec::default()
                })
                .collect(),
            mask: None,
        },
    );
    let run = |name: &str, caps: &[(&str, u64)]| -> (Output, std::path::PathBuf) {
        let output = root.path().join(name);
        let mut args = vec![
            "--minZoom".to_owned(),
            "15".into(),
            "--maxZoom".into(),
            "16".into(),
        ];
        for (flag, value) in caps {
            args.push((*flag).to_owned());
            args.push(value.to_string());
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        (raster(&input, &output, &args), output)
    };
    let (result, output) = run("defaults", &[]);
    let baseline = published(&result, &output);
    let source_bytes = std::fs::metadata(&input).unwrap().len();
    assert_eq!(baseline["source"]["bytes"], source_bytes);
    let pixels = (width * height) as u64;
    // Three Byte bands, each with its independent Boolean validity byte.
    let decoded = pixels * 3 * (1 + 1);
    let tiles = baseline["tiles"].as_u64().unwrap();
    assert!(tiles >= 2, "a one-below tile cap must stay positive");
    let mut caps = [
        ("--max-source-bytes", source_bytes),
        ("--max-source-pixels", pixels),
        ("--max-decoded-bytes", decoded),
        ("--max-tiles", tiles),
        ("--max-output-bytes", 8 << 30),
        ("--max-working-bytes", 64 << 20),
    ];
    // report.json echoes every cap, so the closed output need depends on the
    // output cap's digits: iterate to the fixed point where cap == need.
    let mut at_need = None;
    for attempt in 0..4 {
        let (result, output) = run(&format!("output-{attempt}"), &caps);
        let report = published(&result, &output);
        let total = files_bytes(&output);
        if total == caps[4].1 {
            at_need = Some(report);
            break;
        }
        caps[4].1 = total;
    }
    let at_need = at_need.expect("closed output size reaches a fixed point");
    let output_need = caps[4].1;
    assert_eq!(
        output_need.to_string().len(),
        (output_need - 1).to_string().len(),
        "one below must not shorten report.json"
    );
    let values: std::collections::BTreeSet<_> = caps.iter().map(|c| c.1).collect();
    assert_eq!(
        values.len(),
        6,
        "distinct caps expose a field swap: {caps:?}"
    );
    assert_eq!(
        at_need["limits"],
        serde_json::json!({
            "max_source_bytes": caps[0].1,
            "max_source_pixels": caps[1].1,
            "max_decoded_bytes": caps[2].1,
            "max_tiles": caps[3].1,
            "max_output_bytes": caps[4].1,
            "max_working_bytes": caps[5].1,
            "workers": 2,
        })
    );
    let expected = [
        "source byte limit",
        "pixel/decoded byte limit",
        "pixel/decoded byte limit",
        // The CLI message omits the grid cause; this tiny pyramid is far
        // from the other grid refusal (2 GiB finest RGBA).
        "raster grid admission failed",
        "completed output byte limit",
        "logical working allowance",
    ];
    for (index, needle) in expected.into_iter().enumerate() {
        let mut below = caps;
        below[index].1 = if index == 5 {
            256 * 256 * 4 - 1
        } else {
            caps[index].1 - 1
        };
        let (result, output) = run(&format!("below-{index}"), &below);
        let message = refusal(&result, &output, "resource_limit", 1);
        assert!(message.contains(needle), "{}: {message}", caps[index].0);
    }
    // The contract requires positive caps and one to four workers.
    for (flag, value) in caps
        .iter()
        .map(|c| (c.0, 0))
        .chain([("--jobs", 0), ("--jobs", 5)])
    {
        let (result, output) = run("zero", &[(flag, value)]);
        refusal(&result, &output, "invalid_request", 2);
    }
}

/// Gray display: an unmasked selected NaN or infinity refuses as
/// `invalid_input`, including beside an unrelated finite NoData. NaN NoData,
/// an invalid mask or an unselected band make it a zero-RGBA or ignored pixel.
#[test]
fn gray_refuses_unmasked_selected_nonfinite_samples() {
    let root = tempfile::tempdir().unwrap();
    let (width, height, bad_at) = (16usize, 16usize, 5 * 16 + 5);
    let affine = [12.5, 1e-4, 0., 41.9, 0., -1e-4];
    let samples = |bad: f32| {
        let mut values = vec![0.5f32; width * height];
        values[bad_at] = bad;
        f32_samples(&values)
    };
    let fixture = |name: &str, bands: Vec<BandSpec<'static>>, mask: Option<Vec<u8>>| {
        let input = root.path().join(format!("{name}.tif"));
        write_tiff(
            &input,
            TiffSpec {
                width,
                height,
                datatype: gdal_sys::GDALDataType::GDT_Float32,
                epsg: 4326,
                affine,
                area_or_point: None,
                bands,
                mask,
            },
        );
        let output = root.path().join(name);
        let result = raster(
            &input,
            &output,
            &[
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
            ],
        );
        (input, output, result)
    };
    for (name, bad, nodata) in [
        ("nan", f32::NAN, None),
        ("nan-beside-finite-nodata", f32::NAN, Some(-9999.)),
        ("infinity", f32::INFINITY, None),
        ("negative-infinity", f32::NEG_INFINITY, Some(-9999.)),
    ] {
        let band = BandSpec {
            samples: samples(bad),
            nodata,
            ..BandSpec::default()
        };
        let (_, output, result) = fixture(name, vec![band], None);
        let message = refusal(&result, &output, "invalid_input", 3);
        assert!(message.contains("nonfinite"), "{name}: {message}");
    }
    // Source pixel (x, y) center, sampled from its z16 tile.
    let pixel = |output: &Path, x: usize, y: usize| {
        let lon = affine[0] + (x as f64 + 0.5) * affine[1];
        let lat = affine[3] + (y as f64 + 0.5) * affine[5];
        let tx = (lon + 180.) / 360. * 65536.;
        let ty = (1. - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2. * 65536.;
        let tile = output.join(format!("tiles/16/{}/{}.png", tx.floor(), ty.floor()));
        image::open(tile)
            .unwrap()
            .into_rgba8()
            .get_pixel((tx.fract() * 256.) as u32, (ty.fract() * 256.) as u32)
            .0
    };
    let mut mask = vec![255u8; width * height];
    mask[bad_at] = 0;
    let accepted = [
        (
            "nan-nodata",
            vec![BandSpec {
                samples: samples(f32::NAN),
                nodata: Some(f64::NAN),
                ..BandSpec::default()
            }],
            None,
            [0; 4],
        ),
        (
            "masked-nan",
            vec![BandSpec {
                samples: samples(f32::NAN),
                ..BandSpec::default()
            }],
            Some(mask),
            [0; 4],
        ),
        (
            "unselected-nan",
            vec![
                BandSpec {
                    samples: samples(0.5),
                    ..BandSpec::default()
                },
                BandSpec {
                    samples: samples(f32::NAN),
                    ..BandSpec::default()
                },
            ],
            None,
            [128, 128, 128, 255],
        ),
    ];
    for (name, bands, mask, at_bad) in accepted {
        let (input, output, result) = fixture(name, bands, mask);
        published(&result, &output);
        assert_eq!(pixel(&output, 5, 5), at_bad, "{name}");
        assert_eq!(pixel(&output, 10, 10), [128, 128, 128, 255], "{name}");
        // The COG keeps the nonfinite sample bits and validity.
        assert_eq!(
            read_facts(&output.join("source.cog.tif")),
            read_facts(&input),
            "{name}"
        );
    }
}
