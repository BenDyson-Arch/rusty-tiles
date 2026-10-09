//! Finite standalone DEM decoding. All native ownership ends before returning.
use crate::{
    geospatial::{
        native::{self, Dataset},
        QuietErrors,
    },
    runtime::Attempt,
    JobError, JobErrorKind, JobFailure,
};
use std::{
    ffi::{CStr, CString},
    path::{Path, PathBuf},
};
const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PIXELS: usize = 8_388_608;
fn error(kind: JobErrorKind, message: impl Into<String>) -> JobError {
    JobError::new(kind, message)
}
fn unsupported(message: &str) -> JobError {
    error(JobErrorKind::Unsupported, message)
}
fn native_error(e: crate::Error) -> JobError {
    error(
        match e {
            crate::Error::Environment(_) => JobErrorKind::Unsupported,
            crate::Error::Io(_) => JobErrorKind::Io,
            _ => JobErrorKind::InvalidInput,
        },
        e.to_string(),
    )
}
pub(super) struct PreparedRaster {
    pub source_crs: String,
    pub bounds: [f64; 4],
    pub pixel: [f64; 2],
    pub width: usize,
    pub height: usize,
    pub source_bytes: u64,
    values: Vec<f64>,
    validity: Vec<bool>,
}
impl PreparedRaster {
    /// Closed pixel-area footprint; extend edge centres only inside that footprint.
    /// Invalid neighbours with zero interpolation weight do not affect a sample.
    pub fn sample(&self, lon: f64, lat: f64) -> Option<f64> {
        if !lon.is_finite()
            || !lat.is_finite()
            || lon < self.bounds[0]
            || lon > self.bounds[2]
            || lat < self.bounds[1]
            || lat > self.bounds[3]
        {
            return None;
        }
        let x = ((lon - self.bounds[0]) / self.pixel[0] - 0.5).clamp(0., (self.width - 1) as f64);
        let y = ((self.bounds[3] - lat) / self.pixel[1] - 0.5).clamp(0., (self.height - 1) as f64);
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fx = x - x0 as f64;
        let fy = y - y0 as f64;
        // Same-sign endpoints use a bounded difference; opposite-sign
        // endpoints use products whose sum cannot overflow. Exact endpoints
        // bypass unused values, including invalid/NaN neighbours.
        let lerp = |a: f64, b: f64, t: f64| {
            let value = if a.is_sign_negative() == b.is_sign_negative() {
                a + (b - a) * t
            } else {
                a * (1. - t) + b * t
            };
            value.clamp(a.min(b), a.max(b))
        };
        let row = |py: usize| -> Option<f64> {
            let a = py * self.width + x0;
            if !self.validity[a] {
                return None;
            }
            if fx == 0. {
                return Some(self.values[a]);
            }
            let b = py * self.width + x1;
            if !self.validity[b] {
                return None;
            }
            Some(lerp(self.values[a], self.values[b], fx))
        };
        let a = row(y0)?;
        if fy == 0. {
            return Some(a);
        }
        Some(lerp(a, row(y1)?, fy))
    }
}
fn standalone(path: &Path) -> Result<(), JobError> {
    for suffix in [".aux.xml", ".msk", ".ovr"] {
        let mut companion = path.as_os_str().to_os_string();
        companion.push(suffix);
        let companion = PathBuf::from(companion);
        match std::fs::symlink_metadata(&companion) {
            Ok(_) => return Err(unsupported("T1 DEM forbids external sidecars")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(JobError::io("inspect DEM sidecar", &companion, e)),
        }
    }
    Ok(())
}
pub(super) fn prepare(path: &Path, attempt: &Attempt) -> Result<PreparedRaster, JobFailure> {
    prepare_with_close(path, attempt, |dataset| {
        dataset
            .finish("cannot close T1 DEM")
            .map_err(|e| error(JobErrorKind::Io, e.to_string()))
    })
}
fn prepare_with_close(
    path: &Path,
    attempt: &Attempt,
    close: impl FnOnce(Dataset<'static>) -> Result<(), JobError>,
) -> Result<PreparedRaster, JobFailure> {
    let open = || -> Result<(Dataset<'static>, std::fs::Metadata), JobError> {
        attempt.check()?;
        let metadata =
            std::fs::symlink_metadata(path).map_err(|e| JobError::io("inspect DEM", path, e))?;
        if !metadata.is_file() {
            return Err(unsupported(
                "T1 DEM must be a regular file without a symlink leaf",
            ));
        }
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err(unsupported("T1 DEM exceeds 128 MiB source limit"));
        }
        standalone(path)?;
        native::init().map_err(native_error)?;
        let filename = native::c_path(path).map_err(native_error)?;
        let sibling = CString::new(
            path.file_name()
                .ok_or_else(|| unsupported("DEM filename missing"))?
                .as_encoded_bytes(),
        )
        .map_err(|_| error(JobErrorKind::InvalidInput, "DEM filename contains NUL"))?;
        let drivers = [c"GTiff".as_ptr(), std::ptr::null()];
        let options = [c"GEOREF_SOURCES=INTERNAL".as_ptr(), std::ptr::null()];
        let siblings = [sibling.as_ptr(), std::ptr::null()];
        let _quiet = QuietErrors::new();
        // SAFETY: Read-only open with owned null-terminated lists; only GTiff can open.
        let raw = unsafe {
            gdal_sys::GDALOpenEx(
                filename.as_ptr(),
                2,
                drivers.as_ptr(),
                options.as_ptr(),
                siblings.as_ptr(),
            )
        };
        unsafe { Dataset::adopt(raw, "cannot open T1 GTiff DEM") }
            .map(|dataset| (dataset, metadata))
            .map_err(native_error)
    };
    let (dataset, before) = open().map_err(|e| attempt.fail(e))?;
    let result = decode(&dataset, path, attempt);
    let close = close(dataset);
    let result = result.and_then(|mut source| {
        let after = std::fs::symlink_metadata(path)
            .map_err(|e| JobError::io("reinspect closed DEM", path, e))?;
        if !same_source(&before, &after) {
            return Err(error(
                JobErrorKind::InvalidInput,
                "DEM source changed during decoding or checked close",
            ));
        }
        source.source_bytes = before.len();
        Ok(source)
    });
    finish_prepared(result, close, attempt)
}
fn same_source(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    if !b.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if a.dev() != b.dev()
            || a.ino() != b.ino()
            || a.ctime() != b.ctime()
            || a.ctime_nsec() != b.ctime_nsec()
        {
            return false;
        }
    }
    a.len() == b.len() && a.modified().ok() == b.modified().ok()
}
fn finish_prepared(
    result: Result<PreparedRaster, JobError>,
    close: Result<(), JobError>,
    attempt: &Attempt,
) -> Result<PreparedRaster, JobFailure> {
    match (result, close) {
        (Ok(source), Ok(())) => Ok(source),
        (Err(e), close) => {
            let mut failure = attempt.fail(e);
            if let Err(e) = close {
                failure.secondary.push(e);
            }
            Err(failure)
        }
        (Ok(_), Err(e)) => Err(attempt.fail(e)),
    }
}
fn file_path(bytes: Vec<u8>) -> Result<PathBuf, JobError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(std::ffi::OsString::from_vec(bytes).into())
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes)
            .map(PathBuf::from)
            .map_err(|_| unsupported("DEM dependency filename is not UTF-8"))
    }
}
fn admit_block(width: i32, height: i32, sample_bytes: u64) -> Result<(), JobError> {
    if width <= 0 || height <= 0 {
        return Err(error(
            JobErrorKind::InvalidInput,
            "DEM storage block dimensions must be positive",
        ));
    }
    let bytes = (width as u64)
        .checked_mul(height as u64)
        .and_then(|n| n.checked_mul(sample_bytes));
    if bytes.is_none_or(|n| n > 64 * 1024 * 1024) {
        return Err(unsupported(
            "T1 DEM storage block exceeds 64 MiB decoded-block limit",
        ));
    }
    Ok(())
}

fn decode(
    dataset: &Dataset<'_>,
    path: &Path,
    attempt: &Attempt,
) -> Result<PreparedRaster, JobError> {
    let _quiet = QuietErrors::new();
    // SAFETY: Metadata pointers borrow the live dataset; file list is released once.
    unsafe {
        let files = gdal_sys::GDALGetFileList(dataset.raw());
        let mut names = Vec::new();
        if !files.is_null() {
            let mut i = 0;
            while !(*files.add(i)).is_null() {
                names.push(CStr::from_ptr(*files.add(i)).to_bytes().to_vec());
                i += 1;
            }
            gdal_sys::CSLDestroy(files);
        }
        if names.len() != 1 || file_path(names.remove(0))? != path {
            return Err(unsupported("T1 DEM forbids external dataset dependencies"));
        }
        if gdal_sys::GDALGetGCPCount(dataset.raw()) != 0 {
            return Err(unsupported("T1 DEM forbids GCP georeferencing"));
        }
        for domain in [c"RPC", c"GEOLOCATION"] {
            let metadata = gdal_sys::GDALGetMetadata(dataset.raw(), domain.as_ptr());
            if !metadata.is_null() && !(*metadata).is_null() {
                return Err(unsupported("T1 DEM forbids RPC/geolocation metadata"));
            }
        }
        let area = gdal_sys::GDALGetMetadataItem(
            dataset.raw(),
            c"AREA_OR_POINT".as_ptr(),
            std::ptr::null(),
        );
        if !area.is_null() && CStr::from_ptr(area).to_bytes() != b"Area" {
            return Err(unsupported("T1 DEM requires PixelIsArea semantics"));
        }
    }
    if dataset.band_count() != 1 {
        return Err(unsupported("T1 DEM requires one elevation band"));
    }
    let [width, height] = dataset.size();
    if width <= 0 || height <= 0 {
        return Err(error(
            JobErrorKind::InvalidInput,
            "DEM dimensions must be positive",
        ));
    }
    let (width, height) = (width as usize, height as usize);
    if width > 32768 || height > 32768 || width.checked_mul(height).is_none_or(|n| n > MAX_PIXELS) {
        return Err(unsupported(
            "T1 DEM exceeds dimension/64 MiB decoded-value limit",
        ));
    }
    let source_crs = dataset.projection();
    if source_crs.is_empty() {
        return Err(error(JobErrorKind::InvalidInput, "DEM CRS is missing"));
    }
    let projection = CString::new(source_crs.as_str())
        .map_err(|_| error(JobErrorKind::InvalidInput, "DEM CRS contains NUL"))?;
    // SAFETY: Independent OSR objects are destroyed on every branch.
    let same = unsafe {
        let source = gdal_sys::OSRNewSpatialReference(projection.as_ptr());
        let expected = gdal_sys::OSRNewSpatialReference(std::ptr::null());
        let same = !source.is_null()
            && !expected.is_null()
            && gdal_sys::OSRImportFromEPSG(expected, 4326) == 0
            && gdal_sys::OSRIsSame(source, expected) != 0;
        if !source.is_null() {
            gdal_sys::OSRDestroySpatialReference(source);
        }
        if !expected.is_null() {
            gdal_sys::OSRDestroySpatialReference(expected);
        }
        same
    };
    if !same {
        return Err(unsupported("T1 DEM requires horizontal WGS84 EPSG:4326"));
    }
    let gt = dataset
        .geo_transform()
        .ok_or_else(|| error(JobErrorKind::InvalidInput, "DEM affine is missing"))?;
    if !gt.iter().all(|v| v.is_finite()) || gt[1] <= 0. || gt[5] >= 0. || gt[2] != 0. || gt[4] != 0.
    {
        return Err(unsupported("T1 DEM requires finite north-up affine"));
    }
    let bounds = [
        gt[0],
        gt[3] + gt[5] * height as f64,
        gt[0] + gt[1] * width as f64,
        gt[3],
    ];
    if !bounds.iter().all(|v| v.is_finite())
        || bounds[0] <= -180.
        || bounds[2] >= 180.
        || bounds[1] <= -90.
        || bounds[3] >= 90.
        || bounds[0] >= bounds[2]
        || bounds[1] >= bounds[3]
        || bounds[2] - bounds[0] >= 180.
    {
        return Err(unsupported("T1 DEM footprint must lie strictly inside geographic limits and span less than 180 degrees"));
    }
    let band = dataset.band(1).map_err(native_error)?;
    // SAFETY: All band queries and reads borrow this live dataset.
    let nodata = unsafe {
        let kind = gdal_sys::GDALGetRasterDataType(band);
        if kind != gdal_sys::GDALDataType::GDT_Float32
            && kind != gdal_sys::GDALDataType::GDT_Float64
        {
            return Err(unsupported("T1 DEM requires Float32 or Float64 samples"));
        }
        let mut block_width = 0;
        let mut block_height = 0;
        gdal_sys::GDALGetBlockSize(band, &mut block_width, &mut block_height);
        admit_block(
            block_width,
            block_height,
            if kind == gdal_sys::GDALDataType::GDT_Float64 {
                8
            } else {
                4
            },
        )?;
        let unit = CStr::from_ptr(gdal_sys::GDALGetRasterUnitType(band))
            .to_string_lossy()
            .to_lowercase();
        if !["", "m", "metre", "meter", "metres", "meters"].contains(&unit.as_str()) {
            return Err(unsupported("T1 DEM declared units must be metres"));
        }
        let mut known = 0;
        let scale = gdal_sys::GDALGetRasterScale(band, &mut known);
        if known != 0 && scale != 1. {
            return Err(unsupported("T1 DEM forbids nonidentity band scale"));
        }
        let offset = gdal_sys::GDALGetRasterOffset(band, &mut known);
        if known != 0 && offset != 0. {
            return Err(unsupported("T1 DEM forbids nonidentity band offset"));
        }
        if gdal_sys::GDALGetOverviewCount(band) != 0 {
            return Err(unsupported("T1 DEM forbids overviews"));
        }
        let mask = gdal_sys::GDALGetMaskFlags(band);
        if mask != 1 && mask != 8 {
            return Err(unsupported(
                "T1 DEM supports only all-valid or scalar-NoData masks",
            ));
        }
        let value = gdal_sys::GDALGetRasterNoDataValue(band, &mut known);
        (known != 0).then_some(value)
    };
    let mut values = vec![0_f64; width * height];
    let mut validity = vec![false; width * height];
    let mut range = [f64::INFINITY, f64::NEG_INFINITY];
    for row in (0..height).step_by(64) {
        attempt.check()?;
        let rows = (height - row).min(64);
        // SAFETY: Contiguous slice has exactly width*rows f64 elements.
        let code = unsafe {
            gdal_sys::GDALRasterIO(
                band,
                gdal_sys::GDALRWFlag::GF_Read,
                0,
                row as i32,
                width as i32,
                rows as i32,
                values[row * width..].as_mut_ptr().cast(),
                width as i32,
                rows as i32,
                gdal_sys::GDALDataType::GDT_Float64,
                0,
                0,
            )
        };
        if code != 0 {
            return Err(error(
                JobErrorKind::InvalidInput,
                crate::geospatial::diagnostic("cannot read T1 DEM values"),
            ));
        }
        for i in row * width..(row + rows) * width {
            if i % 4096 == 0 {
                attempt.check()?;
            }
            let value = values[i];
            let missing = nodata.is_some_and(|n| {
                if n.is_nan() {
                    value.is_nan()
                } else {
                    value == n
                }
            });
            if missing {
                continue;
            }
            if !value.is_finite() {
                return Err(error(
                    JobErrorKind::InvalidInput,
                    "DEM contains nonfinite valid elevation",
                ));
            }
            validity[i] = true;
            range[0] = range[0].min(value);
            range[1] = range[1].max(value);
        }
        attempt.emit(&crate::RunEvent::Progress {
            phase: "terrain_source",
            done: (row + rows) as u64,
            total: Some(height as u64),
        })?;
    }
    if !range[0].is_finite() {
        return Err(error(
            JobErrorKind::InvalidInput,
            "DEM has no valid elevation samples",
        ));
    }
    Ok(PreparedRaster {
        source_crs,
        bounds,
        pixel: [gt[1], -gt[5]],
        width,
        height,
        source_bytes: 0,
        values,
        validity,
    })
}

#[cfg(test)]
pub(super) fn test_raster() -> PreparedRaster {
    PreparedRaster {
        source_crs: "EPSG:4326".into(),
        bounds: [0., 0., 2., 2.],
        pixel: [1., 1.],
        width: 2,
        height: 2,
        source_bytes: 0,
        values: vec![6., 8., 3., 5.],
        validity: vec![true; 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plane() -> PreparedRaster {
        test_raster()
    }
    struct ChangingSource {
        path: PathBuf,
        replace: bool,
        changed: std::sync::atomic::AtomicBool,
    }
    impl crate::Observer for ChangingSource {
        fn observe(&self, event: &crate::RunEvent<'_>) -> Result<(), JobError> {
            if matches!(
                event,
                crate::RunEvent::Progress {
                    phase: "terrain_source",
                    ..
                }
            ) && !self.changed.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                if self.replace {
                    let replacement = self.path.with_extension("replacement");
                    std::fs::write(
                        &replacement,
                        include_bytes!("../../tests/fixtures/t1-plane.tif"),
                    )
                    .unwrap();
                    std::fs::rename(&replacement, &self.path).unwrap();
                } else {
                    use std::io::Write;
                    std::fs::OpenOptions::new()
                        .append(true)
                        .open(&self.path)
                        .unwrap()
                        .write_all(b"changed")
                        .unwrap();
                }
            }
            Ok(())
        }
    }
    #[cfg(unix)]
    #[test]
    fn real_native_source_replacement_and_mutation_are_rejected_before_staging() {
        for replace in [true, false] {
            let parent = crate::runtime::directory::test_directory();
            let input = parent.path().join("input.tif");
            std::fs::write(&input, include_bytes!("../../tests/fixtures/t1-plane.tif")).unwrap();
            let output = parent.path().join("published");
            std::fs::create_dir(&output).unwrap();
            std::fs::write(output.join("old"), b"old inventory").unwrap();
            let observer = std::sync::Arc::new(ChangingSource {
                path: input.clone(),
                replace,
                changed: std::sync::atomic::AtomicBool::new(false),
            });
            let control = crate::RunControl::new(Some(observer.clone()));
            let request = crate::terrain::TerrainRequest::new(
                &input,
                &output,
                crate::terrain::TerrainHeights::RawMetres {
                    height_offset_metres: 0.,
                    fill_height_metres: 0.,
                },
                crate::terrain::TerrainOptions::new(16),
            )
            .with_policy(crate::OutputPolicy::Replace);
            let failure = crate::terrain::terrain_to_directory(request, &control).unwrap_err();
            assert!(observer.changed.load(std::sync::atomic::Ordering::SeqCst));
            assert_eq!(failure.error.kind(), JobErrorKind::InvalidInput);
            assert!(failure.error.message().contains("source changed"));
            assert_eq!(std::fs::read(output.join("old")).unwrap(), b"old inventory");
            assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 2);
        }
    }
    struct CancelSource {
        handle: std::sync::Mutex<Option<crate::CancellationHandle>>,
    }
    impl crate::Observer for CancelSource {
        fn observe(&self, event: &crate::RunEvent<'_>) -> Result<(), JobError> {
            if matches!(
                event,
                crate::RunEvent::Progress {
                    phase: "terrain_source",
                    ..
                }
            ) {
                self.handle.lock().unwrap().take().unwrap().cancel();
            }
            Ok(())
        }
    }
    #[test]
    fn real_native_source_close_finishes_before_return_and_is_secondary_to_cancellation() {
        let parent = crate::runtime::directory::test_directory();
        let path = parent.path().join("input.tif");
        std::fs::write(&path, include_bytes!("../../tests/fixtures/t1-plane.tif")).unwrap();
        for cancel in [false, true] {
            let observer = std::sync::Arc::new(CancelSource {
                handle: std::sync::Mutex::new(None),
            });
            let control = if cancel {
                crate::RunControl::new(Some(observer.clone()))
            } else {
                crate::RunControl::default()
            };
            if cancel {
                *observer.handle.lock().unwrap() = Some(control.cancellation_handle());
            }
            let attempt = control.begin().unwrap();
            let mut closed = false;
            let failure = prepare_with_close(&path, &attempt, |dataset| {
                dataset.finish("native test close").unwrap();
                closed = true;
                Err(error(JobErrorKind::Io, "injected checked-close fault"))
            })
            .err()
            .unwrap();
            assert!(closed);
            assert_eq!(
                failure.error.kind(),
                if cancel {
                    JobErrorKind::Cancelled
                } else {
                    JobErrorKind::Io
                }
            );
            if cancel {
                assert_eq!(failure.secondary.len(), 1);
                assert!(failure.secondary[0]
                    .message()
                    .contains("checked-close fault"));
            } else {
                assert!(failure.secondary.is_empty());
            }
        }
    }
    #[test]
    fn decoded_storage_blocks_have_an_independent_byte_ceiling() {
        assert!(admit_block(4096, 2048, 8).is_ok());
        assert_eq!(
            admit_block(4096, 2049, 8).unwrap_err().kind(),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            admit_block(i32::MAX, i32::MAX, 8).unwrap_err().kind(),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            admit_block(0, 16, 8).unwrap_err().kind(),
            JobErrorKind::InvalidInput
        );
    }
    #[test]
    fn checked_close_failure_prevents_success_and_remains_secondary_to_decode_failure() {
        let control = crate::RunControl::default();
        let attempt = control.begin().unwrap();
        let failure = finish_prepared(
            Err(error(JobErrorKind::InvalidInput, "decode failed")),
            Err(error(JobErrorKind::Io, "close failed")),
            &attempt,
        )
        .err()
        .unwrap();
        assert_eq!(failure.error.kind(), JobErrorKind::InvalidInput);
        assert_eq!(failure.secondary.len(), 1);
        assert_eq!(failure.secondary[0].kind(), JobErrorKind::Io);
        let control = crate::RunControl::default();
        let attempt = control.begin().unwrap();
        let failure = finish_prepared(
            Ok(plane()),
            Err(error(JobErrorKind::Io, "close failed")),
            &attempt,
        )
        .err()
        .unwrap();
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
    }
    #[test]
    fn finite_extreme_values_remain_finite_and_bounded() {
        let mut p = plane();
        p.values = vec![f64::MAX; 4];
        for x in [0.5, 0.6, 0.9, 1., 1.4, 1.5] {
            assert_eq!(p.sample(x, 1.2), Some(f64::MAX));
        }
        p.values = vec![-f64::MAX, f64::MAX, -f64::MAX, f64::MAX];
        assert_eq!(p.sample(1., 1.), Some(0.));
        assert!(p.sample(0.6, 1.1).unwrap().is_finite());
    }
    #[test]
    fn analytic_plane_centres_interpolation_and_closed_footprint() {
        let p = plane();
        for (x, y, expected) in [
            (0.5, 1.5, 6.),
            (1.5, 0.5, 5.),
            (1., 1., 5.5),
            (0., 2., 6.),
            (2., 0., 5.),
        ] {
            assert_eq!(p.sample(x, y), Some(expected));
        }
        assert_eq!(p.sample(-f64::EPSILON, 1.), None);
        assert_eq!(p.sample(1., f64::NAN), None);
    }
    #[test]
    fn nodata_requires_only_positive_weight_neighbours() {
        let mut p = plane();
        p.validity[1] = false;
        p.values[1] = f64::NAN;
        assert_eq!(p.sample(0.5, 1.5), Some(6.));
        assert_eq!(p.sample(1., 1.5), None);
        assert_eq!(p.sample(0.5, 1.), Some(4.5));
        assert_eq!(p.sample(2., 2.), None);
    }
    #[test]
    fn single_pixel_is_constant_only_inside_footprint() {
        let mut p = plane();
        p.width = 1;
        p.height = 1;
        p.bounds = [0., 0., 1., 1.];
        p.values = vec![42.];
        p.validity = vec![true];
        assert_eq!(p.sample(0., 0.), Some(42.));
        assert_eq!(p.sample(1., 1.), Some(42.));
        assert_eq!(p.sample(1.1, 0.5), None);
    }
}
