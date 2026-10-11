//! GDAL admission and copying; no native handles leave this module.
use super::{error, JobError, JobErrorKind};
use crate::{
    geospatial::{native::Dataset, QuietErrors},
    runtime::Attempt,
};
use std::{
    ffi::CStr,
    path::{Path, PathBuf},
};
fn native_error(e: crate::Error) -> JobError {
    let kind = match &e {
        crate::Error::Environment(_) => JobErrorKind::Unsupported,
        crate::Error::Io(_) => JobErrorKind::Io,
        crate::Error::Data(_) => last_native_kind(),
        _ => JobErrorKind::InvalidInput,
    };
    error(kind, e.to_string())
}
fn last_native_kind() -> JobErrorKind {
    // SAFETY: CPL diagnostics are thread-local and this query has no pointer arguments.
    match unsafe { gdal_sys::CPLGetLastErrorNo() } {
        2 | 3 => JobErrorKind::Io,
        6 => JobErrorKind::Unsupported,
        _ => JobErrorKind::InvalidInput,
    }
}
fn unsupported(message: &str) -> JobError {
    error(JobErrorKind::Unsupported, message)
}
pub(super) fn read(
    path: &Path,
    z: u8,
    x: u32,
    y: u32,
    attempt: &Attempt,
) -> Result<Vec<u8>, crate::JobFailure> {
    attempt.check().map_err(|e| attempt.fail(e))?;
    crate::geospatial::native::init().map_err(|e| attempt.fail(native_error(e)))?;
    let filename =
        crate::geospatial::native::c_path(path).map_err(|e| attempt.fail(native_error(e)))?;
    let sibling = std::ffi::CString::new(path.file_name().unwrap().as_encoded_bytes())
        .map_err(|_| attempt.fail(error(JobErrorKind::InvalidInput, "invalid raster filename")))?;
    let drivers = [c"GTiff".as_ptr(), std::ptr::null()];
    let options = [c"GEOREF_SOURCES=INTERNAL".as_ptr(), std::ptr::null()];
    let siblings = [sibling.as_ptr(), std::ptr::null()];
    let _quiet = QuietErrors::new();
    // SAFETY: All null-terminated arrays remain live through this read-only
    // open; GTiff is the only driver admitted and sidecars are not siblings.
    let raw = unsafe {
        gdal_sys::GDALOpenEx(
            filename.as_ptr(),
            2,
            drivers.as_ptr(),
            options.as_ptr(),
            siblings.as_ptr(),
        )
    };
    let dataset = unsafe { Dataset::adopt(raw, "cannot open D1 GTiff") }
        .map_err(|e| attempt.fail(native_error(e)))?;
    let result = copy(&dataset, path, z, x, y, attempt);
    let close = dataset
        .finish("cannot close D1 raster")
        .map_err(|e| error(JobErrorKind::Io, e.to_string()));
    finish_read(result, close, attempt)
}
fn finish_read(
    result: Result<Vec<u8>, JobError>,
    close: Result<(), JobError>,
    attempt: &Attempt,
) -> Result<Vec<u8>, crate::JobFailure> {
    match (result, close) {
        (Ok(pixels), Ok(())) => Ok(pixels),
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

fn filename_from_bytes(bytes: Vec<u8>) -> Result<PathBuf, JobError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes)
            .map(PathBuf::from)
            .map_err(|_| error(JobErrorKind::InvalidInput, "GDAL filename is not UTF-8"))
    }
}
fn copy(
    dataset: &Dataset,
    path: &Path,
    z: u8,
    x: u32,
    y: u32,
    attempt: &Attempt,
) -> Result<Vec<u8>, JobError> {
    let _quiet = QuietErrors::new();
    // SAFETY: All metadata handles and strings below borrow the live dataset;
    // file lists are released exactly once after their contents are copied.
    unsafe {
        let driver = gdal_sys::GDALGetDatasetDriver(dataset.raw());
        if driver.is_null()
            || CStr::from_ptr(gdal_sys::GDALGetDriverShortName(driver)).to_bytes() != b"GTiff"
        {
            return Err(unsupported("D1 raster supports GTiff only"));
        }
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
        if names.len() != 1 || filename_from_bytes(names.remove(0))? != path {
            return Err(unsupported(
                "D1 raster forbids external dataset dependencies",
            ));
        }
    }
    // SAFETY: GDAL metadata strings/lists borrow the live dataset and are read
    // without retaining or modifying them.
    unsafe {
        let area = gdal_sys::GDALGetMetadataItem(
            dataset.raw(),
            c"AREA_OR_POINT".as_ptr(),
            std::ptr::null(),
        );
        if !area.is_null() && CStr::from_ptr(area).to_bytes() != b"Area" {
            return Err(unsupported(
                "D1 raster requires PixelIsArea sample semantics",
            ));
        }
        let color = gdal_sys::GDALGetMetadata(dataset.raw(), c"COLOR_PROFILE".as_ptr());
        if !color.is_null() && !(*color).is_null() {
            return Err(unsupported(
                "D1 raster forbids embedded color profiles and transfer functions",
            ));
        }
    }
    if dataset.size() != [256, 256] || dataset.band_count() != 3 {
        return Err(unsupported("D1 raster requires 256x256 three-band RGB"));
    }
    let projection = dataset.projection();
    let c_projection = std::ffi::CString::new(projection)
        .map_err(|_| error(JobErrorKind::InvalidInput, "invalid raster CRS"))?;
    // SAFETY: OSR owns independent spatial references; strings live through
    // parsing/comparison, and both references are destroyed on every path.
    let mercator = unsafe {
        let source = gdal_sys::OSRNewSpatialReference(c_projection.as_ptr());
        let expected = gdal_sys::OSRNewSpatialReference(std::ptr::null());
        if source.is_null() || expected.is_null() {
            if !source.is_null() {
                gdal_sys::OSRDestroySpatialReference(source);
            }
            if !expected.is_null() {
                gdal_sys::OSRDestroySpatialReference(expected);
            }
            return Err(error(
                JobErrorKind::InvalidInput,
                "D1 raster requires declared EPSG:3857",
            ));
        }
        let imported = gdal_sys::OSRImportFromEPSG(expected, 3857);
        let same = imported == 0 && gdal_sys::OSRIsSame(source, expected) != 0;
        gdal_sys::OSRDestroySpatialReference(source);
        gdal_sys::OSRDestroySpatialReference(expected);
        same
    };
    if !mercator {
        return Err(unsupported("D1 raster requires declared EPSG:3857"));
    }
    let h = std::f64::consts::PI * 6378137.;
    let span = 2. * h / f64::from(1u32 << z);
    let expected = [
        -h + f64::from(x) * span,
        span / 256.,
        0.,
        h - f64::from(y) * span,
        0.,
        -span / 256.,
    ];
    let affine = dataset
        .geo_transform()
        .ok_or_else(|| error(JobErrorKind::InvalidInput, "raster affine is missing"))?;
    if affine.iter().zip(expected).any(|(actual, expected)| {
        !actual.is_finite() || (actual - expected).abs() > 1e-7 + expected.abs() * 1e-12
    }) {
        return Err(unsupported(
            "D1 raster affine must match the requested XYZ footprint",
        ));
    }
    let mut pixels = vec![0; 256 * 256 * 3];
    for index in 1..=3 {
        attempt.check()?;
        let band = dataset.band(index).map_err(native_error)?;
        let mut block_width = 0;
        let mut block_height = 0;
        // SAFETY: Band borrows the live dataset; GDAL writes two stack scalars.
        unsafe {
            gdal_sys::GDALGetBlockSize(band, &mut block_width, &mut block_height);
        }
        if !(1..=256).contains(&block_width) || !(1..=256).contains(&block_height) {
            return Err(unsupported(
                "D1 raster requires native block dimensions at most 256x256",
            ));
        }
        let mut channel = vec![0u8; 256 * 256];
        // SAFETY: The band borrows the live dataset. Metadata writes only stack
        // scalars; RasterIO writes the exactly dimensioned packed byte buffer.
        unsafe {
            if gdal_sys::GDALGetRasterDataType(band) != 1
                || gdal_sys::GDALGetRasterColorInterpretation(band) != index as u32 + 2
            {
                return Err(unsupported(
                    "D1 raster requires UInt8 RGB color interpretations",
                ));
            }
            let mut known = 0;
            gdal_sys::GDALGetRasterNoDataValue(band, &mut known);
            if known != 0 || gdal_sys::GDALGetMaskFlags(band) != 1 {
                return Err(unsupported("D1 raster forbids nodata and masks"));
            }
            let scale = gdal_sys::GDALGetRasterScale(band, &mut known);
            if known != 0 && scale != 1. {
                return Err(unsupported("D1 raster forbids band scaling"));
            }
            let offset = gdal_sys::GDALGetRasterOffset(band, &mut known);
            if known != 0 && offset != 0. {
                return Err(unsupported("D1 raster forbids band offsets"));
            }
            if gdal_sys::GDALRasterIO(
                band,
                gdal_sys::GDALRWFlag::GF_Read,
                0,
                0,
                256,
                256,
                channel.as_mut_ptr().cast(),
                256,
                256,
                1,
                0,
                0,
            ) != 0
            {
                return Err(error(
                    last_native_kind(),
                    crate::geospatial::diagnostic("cannot read D1 raster RGB samples"),
                ));
            }
        }
        for (pixel, value) in pixels.as_chunks_mut::<3>().0.iter_mut().zip(channel) {
            pixel[(index - 1) as usize] = value;
        }
    }
    attempt.check()?;
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_error_remains_secondary_to_read_error() {
        let control = crate::RunControl::default();
        let attempt = control.begin().unwrap();
        let failure = finish_read(
            Err(error(JobErrorKind::InvalidInput, "bad pixels")),
            Err(error(JobErrorKind::Io, "close failed")),
            &attempt,
        )
        .unwrap_err();
        assert_eq!(failure.error.message(), "bad pixels");
        assert_eq!(failure.secondary.len(), 1);
        assert_eq!(failure.secondary[0].kind(), JobErrorKind::Io);
    }
    #[test]
    fn close_error_prevents_success() {
        let control = crate::RunControl::default();
        let attempt = control.begin().unwrap();
        let failure = finish_read(
            Ok(vec![0]),
            Err(error(JobErrorKind::Io, "close failed")),
            &attempt,
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
    }
}
