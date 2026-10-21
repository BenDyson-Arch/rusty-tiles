//! Shared GDAL ownership for the native converters.
//!
//! One process initialisation registers drivers and bounds the raster block
//! cache. Datasets are owned, worker-local handles (neither Send nor Sync);
//! Handles are independent datasets; virtual/derived source borrowing is not
//! part of this shared owner.
//! Domain policy (DEM inspection, imagery display recipes, OGR ingestion)
//! stays with each converter.
use super::{diagnostic, offline, QuietErrors};
use crate::Error;
use std::{
    ffi::{c_void, CStr, CString},
    path::Path,
    ptr::{null, NonNull},
    sync::Once,
};

/// `GDAL_OF_VECTOR`, a C macro excluded by bindgen.
const VECTOR: u32 = 0x04;
/// Process-wide raster block cache limit shared by every native converter.
const CACHE_BYTES: i64 = 64 * 1024 * 1024;

/// Offline policy, driver registration and the bounded block cache. Cheap after
/// the first call; every native dataset is opened after this returns.
pub(crate) fn init() -> Result<(), Error> {
    offline()?;
    static INIT: Once = Once::new();
    // SAFETY: Driver registration and the process-wide cache limit run once,
    // before this module opens any dataset; GDAL synchronises both internally.
    INIT.call_once(|| unsafe {
        gdal_sys::GDALAllRegister();
        gdal_sys::GDALSetCacheMax64(CACHE_BYTES);
    });
    Ok(())
}

/// A filesystem path as GDAL's byte filename; NUL bytes are a data error.
pub(crate) fn c_path(path: &Path) -> Result<CString, Error> {
    CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| Error::Data(format!("filename contains a NUL byte: {path:?}")))
}

/// A user/recipe string for a C API; NUL bytes are a data error.
pub(crate) fn c_str(text: &str) -> Result<CString, Error> {
    CString::new(text).map_err(|_| Error::Data(format!("native argument contains NUL: {text:?}")))
}

pub(crate) struct Dataset {
    handle: NonNull<c_void>,
}

/// Vector drivers that open other datasets, query services, download schemas, run
/// algorithms or start processes named inside a source file. A source must not
/// make a conversion read beyond it.
const DEREFERENCING_VECTOR_DRIVERS: [&CStr; 7] = [
    c"OGR_VRT",
    c"GDALG",
    c"GPSBabel",
    c"WFS",
    c"OAPIF",
    c"CSW",
    c"GMLAS",
];

/// Short names of the registered vector drivers a source file may select.
fn vector_drivers() -> Vec<CString> {
    // SAFETY: Drivers are registered by `init`; the driver handles and their
    // metadata strings are process-lifetime and only read here.
    unsafe {
        (0..gdal_sys::GDALGetDriverCount())
            .map(|i| gdal_sys::GDALGetDriver(i))
            .filter(|d| {
                let vector = gdal_sys::GDALGetMetadataItem(*d, c"DCAP_VECTOR".as_ptr(), null());
                !vector.is_null() && CStr::from_ptr(vector) == c"YES"
            })
            .map(|d| CStr::from_ptr(gdal_sys::GDALGetDriverShortName(d)).to_owned())
            .filter(|name| !DEREFERENCING_VECTOR_DRIVERS.contains(&name.as_c_str()))
            .collect()
    }
}

impl Dataset {
    /// Open a vector read-only with the given driver open options, by any
    /// registered vector driver that does not dereference other sources.
    pub(crate) fn open_vector(
        path: &Path,
        open_options: &[&CStr],
        context: &str,
    ) -> Result<Self, Error> {
        init()?;
        let drivers = vector_drivers();
        if drivers.is_empty() {
            return Err(Error::Environment(
                "GDAL has no admitted vector drivers".into(),
            ));
        }
        Self::open(path, VECTOR, &drivers, open_options, context)
    }

    fn open(
        path: &Path,
        flags: u32,
        drivers: &[CString],
        open_options: &[&CStr],
        context: &str,
    ) -> Result<Self, Error> {
        init()?;
        let _errors = QuietErrors::new();
        let path = c_path(path)?;
        let options: Vec<_> = open_options
            .iter()
            .map(|o| o.as_ptr())
            .chain([null()])
            .collect();
        let drivers: Vec<_> = drivers.iter().map(|d| d.as_ptr()).chain([null()]).collect();
        // SAFETY: The filename and the null-terminated driver and option lists
        // stay live for the call; a null sibling list lets GDAL probe siblings.
        // Flags never include GDAL_OF_UPDATE, so the dataset is read-only.
        let raw = unsafe {
            gdal_sys::GDALOpenEx(
                path.as_ptr(),
                flags,
                drivers.as_ptr(),
                if open_options.is_empty() {
                    null()
                } else {
                    options.as_ptr()
                },
                null(),
            )
        };
        // SAFETY: GDALOpenEx returned either null or a new owned handle.
        unsafe { Self::adopt(raw, context) }
    }
}

impl Dataset {
    /// Take ownership of a handle returned by a GDAL open/create/utility call.
    ///
    /// # Safety
    /// `handle` is null or a uniquely owned dataset not closed elsewhere, and
    /// it is independent of any borrowed source dataset. Call directly after
    /// the native operation so `context` reports its thread-local diagnostic.
    pub(crate) unsafe fn adopt(handle: *mut c_void, context: &str) -> Result<Self, Error> {
        NonNull::new(handle)
            .map(|handle| Self { handle })
            .ok_or_else(|| Error::Data(diagnostic(context)))
    }
}

impl Dataset {
    pub(crate) fn raw(&self) -> gdal_sys::GDALDatasetH {
        self.handle.as_ptr()
    }

    /// Close now, reporting deferred write/flush failures (Drop cannot).
    pub(crate) fn finish(self, context: &str) -> Result<(), Error> {
        let _errors = QuietErrors::new();
        let owned = std::mem::ManuallyDrop::new(self);
        // SAFETY: Consumes the uniquely owned handle; ManuallyDrop prevents a
        // second close. This independent handle has no borrowed source owner.
        if unsafe { gdal_sys::GDALClose(owned.raw()) } != 0 {
            return Err(Error::Data(diagnostic(context)));
        }
        Ok(())
    }

    pub(crate) fn band_count(&self) -> i32 {
        // SAFETY: This dataset is live.
        unsafe { gdal_sys::GDALGetRasterCount(self.raw()) }
    }

    /// Raster `[width, height]` in pixels.
    pub(crate) fn size(&self) -> [i32; 2] {
        // SAFETY: Read-only dimension queries on a live dataset.
        unsafe {
            [
                gdal_sys::GDALGetRasterXSize(self.raw()),
                gdal_sys::GDALGetRasterYSize(self.raw()),
            ]
        }
    }

    /// A 1-based band, owned by (and valid while) this dataset is live.
    pub(crate) fn band(&self, index: i32) -> Result<gdal_sys::GDALRasterBandH, Error> {
        if index < 1 || index > self.band_count() {
            return Err(Error::Data("selected band does not exist".into()));
        }
        // SAFETY: Validated index into a live dataset; the band stays owned by it.
        Ok(unsafe { gdal_sys::GDALGetRasterBand(self.raw(), index) })
    }

    /// The dataset's WKT projection, empty when undeclared.
    pub(crate) fn projection(&self) -> String {
        // SAFETY: Copies the projection string borrowed from this live dataset.
        unsafe { super::string(gdal_sys::GDALGetProjectionRef(self.raw())) }
    }

    /// The six-term geotransform, if the dataset declares one.
    pub(crate) fn geo_transform(&self) -> Option<[f64; 6]> {
        let mut gt = [0.; 6];
        // SAFETY: A live dataset and a six-element output buffer.
        (unsafe { gdal_sys::GDALGetGeoTransform(self.raw(), gt.as_mut_ptr()) } == 0).then_some(gt)
    }
}

impl Drop for Dataset {
    fn drop(&mut self) {
        // SAFETY: This independent dataset handle is uniquely owned. GDAL
        // closes and reclaims every band and buffer the dataset owns.
        unsafe {
            gdal_sys::GDALClose(self.raw());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_bytes_are_data_errors_not_panics() {
        assert!(matches!(c_str("a\0b"), Err(Error::Data(_))));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let path = Path::new(std::ffi::OsStr::from_bytes(b"dem\0.tif"));
            assert!(matches!(c_path(path), Err(Error::Data(_))));
            assert!(matches!(
                Dataset::open_vector(path, &[], "cannot open"),
                Err(Error::Data(_))
            ));
        }
    }

    #[test]
    fn vector_sources_cannot_select_dereferencing_drivers() {
        init().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.geojson");
        std::fs::write(
            &secret,
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[1,2]}}]}"#,
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let vrt = root.path().join("source.vrt");
        std::fs::write(
            &vrt,
            format!(
                "<OGRVRTDataSource><OGRVRTLayer name=\"secret\"><SrcDataSource>{}</SrcDataSource></OGRVRTLayer></OGRVRTDataSource>",
                secret.display()
            ),
        )
        .unwrap();
        // Control: with every installed driver, the VRT opens the outside file.
        let filename = c_path(&vrt).unwrap();
        // SAFETY: Live terminated filename; read-only vector flags, null lists.
        let handle =
            unsafe { gdal_sys::GDALOpenEx(filename.as_ptr(), VECTOR, null(), null(), null()) };
        // SAFETY: This new independent handle has no other owner.
        let control = unsafe { Dataset::adopt(handle, "cannot open") }.unwrap();
        // SAFETY: The dataset is live; the driver name is process-lifetime.
        let driver = unsafe {
            CStr::from_ptr(gdal_sys::GDALGetDriverShortName(
                gdal_sys::GDALGetDatasetDriver(control.raw()),
            ))
        };
        assert_eq!(driver, c"OGR_VRT");
        // SAFETY: The dataset is live; its first layer is borrowed while it is.
        let features = unsafe {
            gdal_sys::OGR_L_GetFeatureCount(gdal_sys::GDALDatasetGetLayer(control.raw(), 0), 1)
        };
        assert_eq!(features, 1, "the control reads the outside file's feature");
        control.finish("cannot finish").unwrap();
        assert!(matches!(
            Dataset::open_vector(&vrt, &[], "cannot open"),
            Err(Error::Data(_))
        ));
        assert!(Dataset::open_vector(&secret, &[], "cannot open").is_ok());
        let drivers = vector_drivers();
        assert!(drivers.iter().any(|d| d.as_c_str() == c"GeoJSON"));
        assert!(!drivers
            .iter()
            .any(|d| DEREFERENCING_VECTOR_DRIVERS.contains(&d.as_c_str())));
    }

    #[test]
    fn owned_datasets_report_shape_and_close_explicitly() {
        init().unwrap();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("grid.asc");
        std::fs::write(
            &path,
            "ncols 2\nnrows 2\nxllcorner 12\nyllcorner 41\ncellsize 0.1\n1 2\n3 4\n",
        )
        .unwrap();
        let filename = c_path(&path).unwrap();
        // SAFETY: Live terminated filename; read-only raster flags, null optional
        // lists. GDAL returns an independent uniquely owned source dataset.
        let handle =
            unsafe { gdal_sys::GDALOpenEx(filename.as_ptr(), 0x02, null(), null(), null()) };
        // SAFETY: This new independent handle has no other owner.
        let source = unsafe { Dataset::adopt(handle, "cannot open") }.unwrap();
        assert_eq!((source.band_count(), source.size()), (1, [2, 2]));
        source.finish("cannot finish").unwrap();
    }
}
