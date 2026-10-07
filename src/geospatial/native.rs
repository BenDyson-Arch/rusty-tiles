//! Shared GDAL ownership for the native converters.
//!
//! One process initialisation registers drivers and bounds the raster block
//! cache. Datasets are owned, worker-local handles (neither Send nor Sync);
//! derived/virtual datasets borrow their source so they always close first.
//! Domain policy (DEM inspection, imagery display recipes, OGR ingestion)
//! stays with each converter.
use super::{diagnostic, offline, QuietErrors};
use crate::Error;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    marker::PhantomData,
    path::Path,
    ptr::{null, null_mut, NonNull},
    sync::Once,
};

/// `GDAL_OF_RASTER`, a C macro excluded by bindgen.
const RASTER: u32 = 0x02;
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

/// Owned, null-terminated C argv for GDAL's utility option parsers.
pub(crate) struct Arguments {
    _strings: Vec<CString>,
    pointers: Vec<*mut c_char>,
}

impl Arguments {
    pub(crate) fn new<S: AsRef<str>>(arguments: &[S]) -> Result<Self, Error> {
        let strings = arguments
            .iter()
            .map(|a| c_str(a.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::owned(strings))
    }

    pub(crate) fn owned(strings: Vec<CString>) -> Self {
        let mut pointers = strings
            .iter()
            .map(|s| s.as_ptr().cast_mut())
            .collect::<Vec<_>>();
        pointers.push(null_mut());
        Self {
            _strings: strings,
            pointers,
        }
    }

    /// GDAL's parsers take `char**` but only read the list and its strings.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut *mut c_char {
        self.pointers.as_mut_ptr()
    }

    pub(crate) fn as_ptr(&self) -> *const *const c_char {
        self.pointers.as_ptr().cast()
    }
}

macro_rules! utility_options {
    ($(#[$doc:meta])* $name:ident, $ty:ty, $new:path, $free:path) => {
        $(#[$doc])*
        pub(crate) struct $name(NonNull<$ty>);
        impl $name {
            pub(crate) fn new(arguments: &mut Arguments, context: &str) -> Result<Self, Error> {
                let _errors = QuietErrors::new();
                // SAFETY: The owned null-terminated argv stays live during
                // parsing; GDAL copies what it keeps. No binary-options output.
                NonNull::new(unsafe { $new(arguments.as_mut_ptr(), null_mut()) })
                    .map(Self)
                    .ok_or_else(|| Error::Data(diagnostic(context)))
            }
            pub(crate) fn as_ptr(&self) -> *mut $ty {
                self.0.as_ptr()
            }
        }
        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY: One matching free for the uniquely owned allocation.
                unsafe { $free(self.0.as_ptr()) };
            }
        }
    };
}

utility_options!(
    /// Parsed `gdal_translate` options.
    TranslateOptions,
    gdal_sys::GDALTranslateOptions,
    gdal_sys::GDALTranslateOptionsNew,
    gdal_sys::GDALTranslateOptionsFree
);
utility_options!(
    /// Parsed `gdalinfo` options.
    InfoOptions,
    gdal_sys::GDALInfoOptions,
    gdal_sys::GDALInfoOptionsNew,
    gdal_sys::GDALInfoOptionsFree
);
utility_options!(
    /// Parsed `gdaldem` options.
    DemOptions,
    gdal_sys::GDALDEMProcessingOptions,
    gdal_sys::GDALDEMProcessingOptionsNew,
    gdal_sys::GDALDEMProcessingOptionsFree
);
utility_options!(
    /// Parsed `gdalwarp` options.
    WarpOptions,
    gdal_sys::GDALWarpAppOptions,
    gdal_sys::GDALWarpAppOptionsNew,
    gdal_sys::GDALWarpAppOptionsFree
);

/// An owned GDAL dataset. `'a` is the lifetime of the source a derived or
/// virtual dataset references, so it cannot outlive (or close after) it.
pub(crate) struct Dataset<'a> {
    handle: NonNull<c_void>,
    source: PhantomData<&'a Dataset<'a>>,
}

impl Dataset<'static> {
    /// Open a raster read-only with default drivers and options.
    pub(crate) fn open_raster(path: &Path, context: &str) -> Result<Self, Error> {
        Self::open(path, RASTER, &[], context)
    }

    /// Open a vector read-only with the given driver open options.
    pub(crate) fn open_vector(
        path: &Path,
        open_options: &[&CStr],
        context: &str,
    ) -> Result<Self, Error> {
        Self::open(path, VECTOR, open_options, context)
    }

    fn open(path: &Path, flags: u32, open_options: &[&CStr], context: &str) -> Result<Self, Error> {
        init()?;
        let _errors = QuietErrors::new();
        let path = c_path(path)?;
        let options: Vec<_> = open_options
            .iter()
            .map(|o| o.as_ptr())
            .chain([null()])
            .collect();
        // SAFETY: The filename and null-terminated option list stay live for
        // the call; null driver/sibling lists select installed drivers. Flags
        // never include GDAL_OF_UPDATE, so the dataset is read-only.
        let raw = unsafe {
            gdal_sys::GDALOpenEx(
                path.as_ptr(),
                flags,
                null(),
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

impl<'a> Dataset<'a> {
    /// Take ownership of a handle returned by a GDAL open/create/utility call.
    ///
    /// # Safety
    /// `handle` is null or a uniquely owned dataset not closed elsewhere, and
    /// anything it references lives for `'a`. Call directly after the native
    /// operation so `context` reports its thread-local diagnostic.
    pub(crate) unsafe fn adopt(handle: *mut c_void, context: &str) -> Result<Self, Error> {
        NonNull::new(handle)
            .map(|handle| Self {
                handle,
                source: PhantomData,
            })
            .ok_or_else(|| Error::Data(diagnostic(context)))
    }
}

impl Dataset<'_> {
    pub(crate) fn raw(&self) -> gdal_sys::GDALDatasetH {
        self.handle.as_ptr()
    }

    /// Close now, reporting deferred write/flush failures (Drop cannot).
    pub(crate) fn finish(self, context: &str) -> Result<(), Error> {
        let _errors = QuietErrors::new();
        let owned = std::mem::ManuallyDrop::new(self);
        // SAFETY: Consumes the uniquely owned handle; ManuallyDrop prevents a
        // second close. Borrowing datasets cannot exist (self is moved).
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

    /// `gdal_translate` to `destination` ("" with VRT/MEM stays in memory).
    pub(crate) fn translate<'s>(
        &'s self,
        destination: &Path,
        options: &TranslateOptions,
        context: &str,
    ) -> Result<Dataset<'s>, Error> {
        let _errors = QuietErrors::new();
        let destination = c_path(destination)?;
        let mut usage = 0;
        // SAFETY: Source, filename and options stay live through translation;
        // the result is a new owned dataset that may reference this source.
        unsafe {
            let raw = gdal_sys::GDALTranslate(
                destination.as_ptr(),
                self.raw(),
                options.as_ptr(),
                &mut usage,
            );
            Dataset::adopt(raw, context)
        }
    }

    /// Quiet single-source `gdalwarp` to `destination` ("" with VRT/MEM stays in
    /// memory). Quiet mode keeps native progress off the CLI's JSON stdout.
    pub(crate) fn warp<'s>(
        &'s self,
        destination: &Path,
        options: &WarpOptions,
        context: &str,
    ) -> Result<Dataset<'s>, Error> {
        let _errors = QuietErrors::new();
        let destination = c_path(destination)?;
        let mut source = self.raw();
        let mut usage = 0;
        // SAFETY: Source/options/filename are live and exactly one source is
        // passed. The result is a new owned dataset that may reference self.
        unsafe {
            gdal_sys::GDALWarpAppOptionsSetQuiet(options.as_ptr(), 1);
            let raw = gdal_sys::GDALWarp(
                destination.as_ptr(),
                null_mut(),
                1,
                &mut source,
                options.as_ptr(),
                &mut usage,
            );
            Dataset::adopt(raw, context)
        }
    }
}

impl Drop for Dataset<'_> {
    fn drop(&mut self) {
        // SAFETY: Unique ownership; borrowing derived datasets drop first. GDAL
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
        assert!(matches!(
            Arguments::new(&["-of", "x\0"]),
            Err(Error::Data(_))
        ));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let path = Path::new(std::ffi::OsStr::from_bytes(b"dem\0.tif"));
            assert!(matches!(c_path(path), Err(Error::Data(_))));
            assert!(matches!(
                Dataset::open_raster(path, "cannot open"),
                Err(Error::Data(_))
            ));
        }
    }

    #[test]
    fn derived_datasets_close_before_their_source_and_report_close() {
        let source = Dataset::open_raster(Path::new("/nonexistent/rusty-tiles.tif"), "missing");
        assert!(source.is_err());
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("grid.asc");
        std::fs::write(
            &path,
            "ncols 2\nnrows 2\nxllcorner 12\nyllcorner 41\ncellsize 0.1\n1 2\n3 4\n",
        )
        .unwrap();
        let source = Dataset::open_raster(&path, "cannot open").unwrap();
        assert_eq!((source.band_count(), source.size()), (1, [2, 2]));
        let options =
            WarpOptions::new(&mut Arguments::new(&["-of", "VRT"]).unwrap(), "bad").unwrap();
        let derived = source.warp(Path::new(""), &options, "warp failed").unwrap();
        assert_eq!(derived.size(), [2, 2]);
        derived.finish("cannot finish").unwrap();
        source.finish("cannot finish").unwrap();
    }
}
