//! Native GDAL utility APIs, with datasets confined to the calling thread.
use super::RasterOptions;
use crate::{
    geospatial::{
        self,
        native::{
            self, c_path, c_str, Arguments, Dataset, DemOptions, InfoOptions, TranslateOptions,
            WarpOptions,
        },
        QuietErrors,
    },
    Error,
};
use serde_json::{json, Value};
use std::{
    ffi::{c_char, c_void},
    path::Path,
    ptr::{null_mut, NonNull},
};

// GDAL renamed GDT_Byte to GDT_UInt8 in 3.13; its stable C enum value is 1.
const BYTE: gdal_sys::GDALDataType::Type = 1;
const FINISH: &str = "cannot finish raster derivative";

pub(super) fn convert(input: &Path, output: &Path, options: &RasterOptions) -> Result<(), Error> {
    let source = Dataset::open_raster(input, "cannot open raster")?;
    let count = source.band_count();
    let projection = source.projection();
    if projection.is_empty() || count == 0 {
        return Err(Error::Data(
            "raster requires a declared CRS and at least one band".into(),
        ));
    }
    let source_crs = geospatial::Crs::from_definition(&projection)?;
    let mut bands = if count >= 3 { vec![1, 2, 3] } else { vec![1] };
    if options.display == "gray" {
        let band = source.band(i32::from(options.band))?;
        let mut known = 0;
        // SAFETY: Band and its parent dataset remain live during metadata queries.
        unsafe {
            let scale = gdal_sys::GDALGetRasterScale(band, &mut known);
            if known != 0 && scale != 1. {
                return Err(Error::Data(
                    "scaled bands must be converted to actual values before styling".into(),
                ));
            }
            let offset = gdal_sys::GDALGetRasterOffset(band, &mut known);
            if known != 0 && offset != 0. {
                return Err(Error::Data(
                    "scaled bands must be converted to actual values before styling".into(),
                ));
            }
        }
    } else {
        if options.alpha_band != 0 {
            bands.push(i32::from(options.alpha_band));
        }
        for &index in &bands {
            let band = source.band(index)?;
            // SAFETY: A validated band belonging to the live source dataset.
            if unsafe { gdal_sys::GDALGetRasterDataType(band) } != BYTE {
                return Err(Error::Data("image display requires byte imagery; use gray with an explicit range for numeric bands".into()));
            }
        }
    }
    let bounds = coverage_bounds(&source, &source_crs)?;
    preflight(bounds, options.min_zoom, options.max_zoom)?;
    // Instantiate before producing derivatives so stripped GDAL builds report an
    // environment error without starting expensive COG generation.
    let tile = TileAlgorithm::new()?;
    // Keep native parallelism bounded, including on large CI/server machines.
    // Compression and tiling run in separate stages, so their pools do not overlap.
    let workers = std::thread::available_parallelism().map_or(1, |count| count.get().min(4));
    let compression_workers = format!("NUM_THREADS={workers}");
    let cog = translate(
        &source,
        &output.join("source.cog.tif"),
        &[
            "-of",
            "COG",
            "-co",
            "COMPRESS=DEFLATE",
            "-co",
            &compression_workers,
        ],
    )?;
    cog.finish(FINISH)?;
    let display_path = output.join("display.tif");
    if options.display == "gray" {
        let band = options.band.to_string();
        let selected = translate(&source, Path::new(""), &["-of", "VRT", "-b", &band])?;
        let colors = output.join("colors.txt");
        std::fs::write(
            &colors,
            format!(
                "{} 0 0 0 255\n{} 255 255 255 255\nnv 0 0 0 0\n",
                options.display_min.unwrap(),
                options.display_max.unwrap()
            ),
        )?;
        let display = color_relief(&selected, &display_path, &colors)?;
        apply_coverage(&display, &selected, 4)?;
        display.finish(FINISH)?;
        std::fs::remove_file(colors)?;
    } else {
        let mut arguments = vec![
            "-of".into(),
            "VRT".into(),
            "-ot".into(),
            "Byte".into(),
            "-scale".into(),
            "0".into(),
            "255".into(),
            "0".into(),
            "255".into(),
            "-a_nodata".into(),
            "none".into(),
            "-mask".into(),
            "mask,1".into(),
        ];
        for &band in &bands {
            arguments.extend(["-b".into(), band.to_string()]);
        }
        let rgb = count >= 3;
        arguments.extend([
            "-colorinterp".into(),
            match (rgb, options.alpha_band != 0) {
                (true, true) => "red,green,blue,alpha",
                (true, false) => "red,green,blue",
                (false, true) => "gray,alpha",
                (false, false) => "gray",
            }
            .into(),
        ]);
        let selected = translate(&source, Path::new(""), &arguments)?;
        if options.alpha_band != 0 {
            let combined_path = output.join("image-alpha.tif");
            let combined = translate(
                &selected,
                &combined_path,
                &[
                    "-of",
                    "GTiff",
                    "-mask",
                    "none",
                    "-co",
                    "TILED=YES",
                    "-co",
                    // Intermediate files are deleted before publication. Avoid
                    // repeatedly compressing pixels that will become PNGs.
                    "COMPRESS=NONE",
                ],
            )?;
            apply_coverage(&combined, &selected, bands.len() as i32)?;
            let warped = display_warp(&combined, &display_path, true)?;
            warped.finish(FINISH)?;
            combined.finish(FINISH)?;
            std::fs::remove_file(combined_path)?;
        } else {
            let warped = display_warp(&selected, &display_path, false)?;
            warped.finish(FINISH)?;
        }
    }
    tile.run(&display_path, &output.join("tiles"), options, workers)?;
    std::fs::remove_file(display_path)?;
    std::fs::write(
        output.join("tilejson.json"),
        serde_json::to_vec(&json!({
            "tilejson":"3.0.0", "scheme":"xyz", "tiles":["tiles/{z}/{x}/{y}.png"],
            "minzoom":options.min_zoom, "maxzoom":options.max_zoom, "bounds":bounds,
        }))?,
    )?;
    // Keep the existing recipe schema and GDAL's numeric VERSION_NUM identity.
    // SAFETY: GDAL owns the static null-terminated version string.
    let version = unsafe { geospatial::string(gdal_sys::GDALVersionInfo(c"VERSION_NUM".as_ptr())) };
    std::fs::write(
        output.join("conversion.json"),
        serde_json::to_vec(&json!({
            "display":options.display, "band":options.band, "displayMin":options.display_min,
            "displayMax":options.display_max, "alphaBand":options.alpha_band,
            "sourceBands":count, "gdalVersion":version,
        }))?,
    )?;
    Ok(())
}

fn validate_bounds(bounds: [f64; 4]) -> Result<(), Error> {
    if !bounds.iter().all(|v| v.is_finite())
        || bounds[0] < -180.
        || bounds[2] > 180.
        || bounds[0] >= bounds[2]
        || bounds[1] < -85.05112878
        || bounds[3] > 85.05112878
        || bounds[1] >= bounds[3]
    {
        return Err(Error::Data(
            "split antimeridian rasters or reproject polar coverage before imagery tiling".into(),
        ));
    }
    Ok(())
}

fn preflight(bounds: [f64; 4], min_zoom: u8, max_zoom: u8) -> Result<(), Error> {
    validate_bounds(bounds)?;
    let mut total = 0u64;
    for z in min_zoom..=max_zoom {
        let n = f64::from(1u32 << z);
        let x = |lon: f64| ((lon + 180.) / 360. * n).floor().clamp(0., n - 1.) as u64;
        let y = |lat: f64| {
            ((1. - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2. * n)
                .floor()
                .clamp(0., n - 1.) as u64
        };
        total += (x(bounds[2]) - x(bounds[0]) + 1) * (y(bounds[1]) - y(bounds[3]) + 1);
        if total > 100_000 {
            return Err(Error::Data(
                "display would exceed 100000 tiles; reduce maximum zoom".into(),
            ));
        }
    }
    Ok(())
}

fn translate<'a, S: AsRef<str>>(
    source: &'a Dataset<'_>,
    path: &Path,
    arguments: &[S],
) -> Result<Dataset<'a>, Error> {
    let options = TranslateOptions::new(
        &mut Arguments::new(arguments)?,
        "invalid raster translate options",
    )?;
    source.translate(path, &options, "raster translation failed")
}
fn coverage_bounds(source: &Dataset<'_>, source_crs: &geospatial::Crs) -> Result<[f64; 4], Error> {
    let _errors = QuietErrors::new();
    let options = InfoOptions::new(
        &mut Arguments::new(&["-json", "-nomd", "-noct"])?,
        "invalid raster inspection options",
    )?;
    // SAFETY: Live dataset and options; the returned text is a GDAL allocation.
    let text = unsafe { gdal_sys::GDALInfo(source.raw(), options.as_ptr()) };
    if text.is_null() {
        return Err(Error::Data(geospatial::diagnostic(
            "cannot inspect raster extent",
        )));
    }
    // SAFETY: GDALInfo returned a live null-terminated string, copied here.
    let result = serde_json::from_str::<Value>(&unsafe { geospatial::string(text) });
    // SAFETY: Exactly one matching free of GDALInfo's allocated string.
    unsafe { gdal_sys::VSIFree(text.cast()) };
    let info = result?;
    let ring = info["wgs84Extent"]["coordinates"][0]
        .as_array()
        .ok_or_else(|| Error::Data("raster requires a finite geographic extent".into()))?;
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for point in ring {
        let x = point[0].as_f64().filter(|v| v.is_finite());
        let y = point[1].as_f64().filter(|v| v.is_finite());
        let (Some(x), Some(y)) = (x, y) else {
            return Err(Error::Data(
                "raster requires a finite geographic extent".into(),
            ));
        };
        bounds[0] = bounds[0].min(x);
        bounds[1] = bounds[1].min(y);
        bounds[2] = bounds[2].max(x);
        bounds[3] = bounds[3].max(y);
    }
    validate_bounds(bounds)?;
    // A min/max envelope alone cannot distinguish a legitimate wide raster
    // from projected coverage wrapping across the antimeridian. Sample each
    // source edge in order, using the shared offline, only-best transform.
    let target = geospatial::Crs::from_definition("EPSG:4326")?;
    let mut transform = geospatial::StrictTransform::new(source_crs, &target)?;
    let gt = source
        .geo_transform()
        .ok_or_else(|| Error::Data("raster requires a geotransform for imagery tiling".into()))?;
    let [width, height] = source.size().map(f64::from);
    for (start, end) in [
        ([0., 0.], [width, 0.]),
        ([width, 0.], [width, height]),
        ([width, height], [0., height]),
        ([0., height], [0., 0.]),
    ] {
        let points: Vec<_> = (0..=32)
            .map(|i| {
                let t = i as f64 / 32.;
                let x = start[0] + t * (end[0] - start[0]);
                let y = start[1] + t * (end[1] - start[1]);
                [
                    gt[0] + x * gt[1] + y * gt[2],
                    gt[3] + x * gt[4] + y * gt[5],
                    0.,
                ]
            })
            .collect();
        let geographic = transform.transform(&points)?;
        if geographic
            .windows(2)
            .any(|p| (p[1][0] - p[0][0]).abs() > 180.)
        {
            return Err(Error::Data(
                "split antimeridian rasters before imagery tiling".into(),
            ));
        }
    }
    Ok(bounds)
}
fn color_relief<'a>(
    source: &'a Dataset<'_>,
    path: &Path,
    colors: &Path,
) -> Result<Dataset<'a>, Error> {
    let options = DemOptions::new(
        &mut Arguments::new(&[
            "-of",
            "GTiff",
            "-alpha",
            "-co",
            "TILED=YES",
            "-co",
            "COMPRESS=NONE",
        ])?,
        "invalid color relief options",
    )?;
    let _errors = QuietErrors::new();
    let path = c_path(path)?;
    let colors = c_path(colors)?;
    let mut usage = 0;
    // SAFETY: Filenames, source and options stay live through processing; the
    // result is a new owned dataset adopted with the source's lifetime.
    unsafe {
        let raw = gdal_sys::GDALDEMProcessing(
            path.as_ptr(),
            source.raw(),
            c"color-relief".as_ptr(),
            colors.as_ptr(),
            options.as_ptr(),
            &mut usage,
        );
        Dataset::adopt(raw, "raster color relief failed")
    }
}
fn display_warp<'a>(
    source: &'a Dataset<'_>,
    path: &Path,
    alpha: bool,
) -> Result<Dataset<'a>, Error> {
    let mut argv = vec![
        "-of",
        "GTiff",
        "-t_srs",
        "EPSG:3857",
        "-dstalpha",
        "-wm",
        "64",
        "-novshift",
        "-to",
        "ALLOW_BALLPARK=NO",
        "-to",
        "ONLY_BEST=YES",
        "-co",
        "TILED=YES",
        "-co",
        "COMPRESS=NONE",
    ];
    if alpha {
        argv.push("-srcalpha");
    }
    let options = WarpOptions::new(&mut Arguments::new(&argv)?, "invalid imagery warp options")?;
    source.warp(path, &options, "imagery warp failed")
}
fn apply_coverage(
    display: &Dataset<'_>,
    source: &Dataset<'_>,
    alpha_index: i32,
) -> Result<(), Error> {
    let _errors = QuietErrors::new();
    let alpha = display.band(alpha_index)?;
    // SAFETY: The band and its mask are borrowed from the live source.
    let mask = unsafe { gdal_sys::GDALGetMaskBand(source.band(1)?) };
    // Both datasets have identical dimensions because this precedes any display
    // warp/resampling; checked so every window below is in bounds for both.
    let [width, height] = display.size();
    if [width, height] != source.size() {
        return Err(Error::Data(
            "display coverage dimensions differ from source".into(),
        ));
    }
    let mut values = vec![0u8; 256 * 256];
    let mut coverage = vec![0u8; 256 * 256];
    for y in (0..height).step_by(256) {
        for x in (0..width).step_by(256) {
            let w = (width - x).min(256);
            let h = (height - y).min(256);
            raster_io(alpha, false, x, y, w, h, &mut values)?;
            raster_io(mask, false, x, y, w, h, &mut coverage)?;
            for (v, c) in values.iter_mut().zip(&coverage).take((w * h) as usize) {
                *v = (*v).min(*c);
            }
            raster_io(alpha, true, x, y, w, h, &mut values)?;
        }
    }
    // SAFETY: Flush a live writable display dataset before downstream reads.
    if unsafe { gdal_sys::GDALFlushCache(display.raw()) } != 0 {
        return Err(Error::Data(geospatial::diagnostic(
            "cannot flush display coverage",
        )));
    }
    Ok(())
}
fn raster_io(
    band: gdal_sys::GDALRasterBandH,
    write: bool,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    buffer: &mut [u8],
) -> Result<(), Error> {
    // SAFETY: Caller supplies a live band, valid in-bounds window and a byte buffer
    // with room for w*h samples. Packed spacing and no resampling are requested.
    let code = unsafe {
        gdal_sys::GDALRasterIO(
            band,
            if write {
                gdal_sys::GDALRWFlag::GF_Write
            } else {
                gdal_sys::GDALRWFlag::GF_Read
            },
            x,
            y,
            w,
            h,
            buffer.as_mut_ptr().cast(),
            w,
            h,
            BYTE,
            0,
            0,
        )
    };
    if code != 0 {
        return Err(Error::Data(geospatial::diagnostic(
            "cannot read/write display coverage",
        )));
    }
    Ok(())
}
// gdal-sys 0.12 does not include gdalalgorithm.h in its bindgen wrapper. These
// declarations match the stable GDAL 3.12 C API (opaque handles, C bool), and use
// the same GDAL library already linked by gdal-sys. Avoid the 3.12-only FromPath API.
unsafe extern "C" {
    fn GDALGetGlobalAlgorithmRegistry() -> *mut c_void;
    fn GDALAlgorithmRegistryRelease(registry: *mut c_void);
    fn GDALAlgorithmRegistryInstantiateAlg(
        registry: *mut c_void,
        name: *const c_char,
    ) -> *mut c_void;
    fn GDALAlgorithmInstantiateSubAlgorithm(
        algorithm: *mut c_void,
        name: *const c_char,
    ) -> *mut c_void;
    fn GDALAlgorithmRelease(algorithm: *mut c_void);
    fn GDALAlgorithmParseCommandLineArguments(
        algorithm: *mut c_void,
        arguments: *const *const c_char,
    ) -> bool;
    fn GDALAlgorithmRun(
        algorithm: *mut c_void,
        progress: gdal_sys::GDALProgressFunc,
        data: *mut c_void,
    ) -> bool;
    fn GDALAlgorithmFinalize(algorithm: *mut c_void) -> bool;
}
struct TileAlgorithm(NonNull<c_void>);
impl TileAlgorithm {
    fn new() -> Result<Self, Error> {
        native::init()?;
        let _errors = QuietErrors::new();
        // SAFETY: Registry, parent and child are separate owned C API handles;
        // releasing the registry/parent leaves the instantiated child valid.
        unsafe {
            for name in [c"COG", c"GTiff", c"PNG"] {
                if gdal_sys::GDALGetDriverByName(name.as_ptr()).is_null() {
                    return Err(Error::Environment(format!(
                        "native GDAL {} driver unavailable",
                        name.to_string_lossy()
                    )));
                }
            }
            let registry = GDALGetGlobalAlgorithmRegistry();
            if registry.is_null() {
                return Err(Error::Environment(geospatial::diagnostic(
                    "native GDAL algorithm registry unavailable",
                )));
            }
            let raster = GDALAlgorithmRegistryInstantiateAlg(registry, c"raster".as_ptr());
            GDALAlgorithmRegistryRelease(registry);
            if raster.is_null() {
                return Err(Error::Environment(geospatial::diagnostic(
                    "native GDAL raster algorithms unavailable",
                )));
            }
            let tile = GDALAlgorithmInstantiateSubAlgorithm(raster, c"tile".as_ptr());
            GDALAlgorithmRelease(raster);
            NonNull::new(tile).map(Self).ok_or_else(|| {
                Error::Environment(geospatial::diagnostic(
                    "native GDAL raster tile algorithm unavailable",
                ))
            })
        }
    }
    fn run(
        self,
        input: &Path,
        output: &Path,
        options: &RasterOptions,
        workers: usize,
    ) -> Result<(), Error> {
        let _errors = QuietErrors::new();
        let mut arguments = vec![
            "--webviewer=none".into(),
            "--tiling-scheme=WebMercatorQuad".into(),
            "--convention=xyz".into(),
            "--output-format=PNG".into(),
            format!("--num-threads={workers}"),
            format!("--min-zoom={}", options.min_zoom),
            format!("--max-zoom={}", options.max_zoom),
        ];
        // Explicitly select threads on the supported GDAL 3.12+ runtime
        // so library conversion never starts a GDAL executable or forks a host
        // that may already have other threads running.
        arguments.push("--parallel-method=thread".into());
        let mut strings = arguments
            .iter()
            .map(|a| c_str(a))
            .collect::<Result<Vec<_>, _>>()?;
        strings.extend([c_path(input)?, c_path(output)?]);
        let args = Arguments::owned(strings);
        // SAFETY: Owned null-terminated argv lives through parsing; the owned
        // algorithm runs exactly once. No terminal callback or native stdout.
        unsafe {
            if !GDALAlgorithmParseCommandLineArguments(self.0.as_ptr(), args.as_ptr()) {
                return Err(Error::Data(geospatial::diagnostic(
                    "invalid native raster tiling arguments",
                )));
            }
            if !GDALAlgorithmRun(self.0.as_ptr(), None, null_mut()) {
                return Err(Error::Data(geospatial::diagnostic(
                    "native raster tiling failed",
                )));
            }
            if !GDALAlgorithmFinalize(self.0.as_ptr()) {
                return Err(Error::Data(geospatial::diagnostic(
                    "native raster tiling finalization failed",
                )));
            }
        }
        Ok(())
    }
}
impl Drop for TileAlgorithm {
    fn drop(&mut self) {
        // SAFETY: One matching release of the uniquely owned handle.
        unsafe { GDALAlgorithmRelease(self.0.as_ptr()) };
    }
}
pub(crate) fn tile_available() -> Result<(), Error> {
    TileAlgorithm::new().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrap_poles_and_unbounded_tile_counts() {
        for bounds in [[170., -1., 190., 1.], [0., 85., 1., 86.]] {
            assert!(preflight(bounds, 0, 0).is_err());
        }
        assert!(preflight([12., 40., 14., 42.], 0, 24).is_err());
        assert!(preflight([12., 40., 14., 42.], 0, 8).is_ok());
        assert!(preflight([-170., -1., 170., 1.], 0, 0).is_ok());
    }
    #[test]
    fn native_tile_algorithm_is_installed() {
        tile_available().unwrap();
    }
}
