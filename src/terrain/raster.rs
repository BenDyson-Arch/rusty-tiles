//! Owned, worker-local GDAL datasets. Warped datasets borrow their source so
//! virtual raster references cannot outlive the underlying dataset.
use crate::{
    geospatial::{self, QuietErrors},
    Error,
};
use std::{
    ffi::{c_void, CString},
    marker::PhantomData,
    path::Path,
    ptr::{null, null_mut, NonNull},
    sync::Once,
};

pub(super) struct Dataset<'a> {
    handle: NonNull<c_void>,
    source: PhantomData<&'a Dataset<'a>>,
}

impl Dataset<'static> {
    pub fn open(path: &Path) -> Result<Self, Error> {
        geospatial::offline()?;
        static REGISTER: Once = Once::new();
        // SAFETY: Driver registration and the process-wide 64 MiB raster block
        // cache limit are initialized once before opening any dataset here.
        REGISTER.call_once(|| unsafe {
            gdal_sys::GDALAllRegister();
            gdal_sys::GDALSetCacheMax64(64 * 1024 * 1024);
        });
        let _errors = QuietErrors::new();
        let path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| Error::Data("DEM filename contains a NUL byte".into()))?;
        // SAFETY: The filename is live and terminated; flags request read-only
        // raster access, and null lists select installed drivers/default options.
        let handle = NonNull::new(unsafe {
            gdal_sys::GDALOpenEx(
                path.as_ptr(),
                0x02, /* GDAL_OF_RASTER, a C macro excluded by bindgen */
                null(),
                null(),
                null(),
            )
        })
        .ok_or_else(|| Error::Data(geospatial::diagnostic("cannot open DEM")))?;
        Ok(Self {
            handle,
            source: PhantomData,
        })
    }
}

impl Dataset<'_> {
    pub fn inspect(&self) -> Result<(String, [f64; 2]), Error> {
        let _errors = QuietErrors::new();
        // SAFETY: All dataset queries borrow this live handle. Band 1 remains
        // owned by the dataset throughout the metadata/min-max queries.
        unsafe {
            let projection =
                geospatial::string(gdal_sys::GDALGetProjectionRef(self.handle.as_ptr()));
            if gdal_sys::GDALGetRasterCount(self.handle.as_ptr()) != 1 || projection.is_empty() {
                return Err(Error::Data(
                    "terrain requires a single-band DEM with a declared CRS".into(),
                ));
            }
            geospatial::Crs::from_definition(&projection)?;
            let band = gdal_sys::GDALGetRasterBand(self.handle.as_ptr(), 1);
            let unit = geospatial::string(gdal_sys::GDALGetRasterUnitType(band)).to_lowercase();
            if !["", "m", "metre", "meter", "metres", "meters"].contains(&unit.as_str()) {
                return Err(Error::Data(format!(
                    "height units must be metres, found {unit:?}"
                )));
            }
            let mut known = 0;
            let scale = gdal_sys::GDALGetRasterScale(band, &mut known);
            if known != 0 && scale != 1. {
                return Err(Error::Data(
                    "scaled DEM bands must be converted to actual metre values first".into(),
                ));
            }
            let offset = gdal_sys::GDALGetRasterOffset(band, &mut known);
            if known != 0 && offset != 0. {
                return Err(Error::Data(
                    "scaled DEM bands must be converted to actual metre values first".into(),
                ));
            }
            let mut range = [0.; 2];
            if gdal_sys::GDALComputeRasterMinMax(band, 0, range.as_mut_ptr()) != 0
                || !range.iter().all(|h| h.is_finite())
            {
                return Err(Error::Data(geospatial::diagnostic(
                    "DEM has no finite height range",
                )));
            }
            Ok((projection, range))
        }
    }

    pub fn warp<'a>(&'a self, arguments: &[String]) -> Result<Dataset<'a>, Error> {
        let _errors = QuietErrors::new();
        let strings: Vec<_> = arguments
            .iter()
            .map(|arg| CString::new(arg.as_str()))
            .collect::<Result<_, _>>()
            .map_err(|_| Error::Data("warp option contains a NUL byte".into()))?;
        let mut pointers: Vec<_> = strings.iter().map(|s| s.as_ptr().cast_mut()).collect();
        pointers.push(null_mut());
        // SAFETY: The null-terminated argv and its strings stay live through
        // parsing; GDAL owns the returned options until their RAII guard drops.
        let options = WarpOptions(
            NonNull::new(unsafe {
                gdal_sys::GDALWarpAppOptionsNew(pointers.as_mut_ptr(), null_mut())
            })
            .ok_or_else(|| Error::Data(geospatial::diagnostic("invalid terrain warp options")))?,
        );
        let mut source = self.handle.as_ptr();
        let mut usage_error = 0;
        // SAFETY: Source/options are live, one source is passed, and empty path
        // creates an in-memory output. Quiet mode prevents native stdout from
        // contaminating the CLI's JSON result. The returned dataset is owned.
        let handle = unsafe {
            gdal_sys::GDALWarpAppOptionsSetQuiet(options.0.as_ptr(), 1);
            gdal_sys::GDALWarp(
                c"".as_ptr(),
                null_mut(),
                1,
                &mut source,
                options.0.as_ptr(),
                &mut usage_error,
            )
        };
        NonNull::new(handle)
            .map(|handle| Dataset {
                handle,
                source: PhantomData,
            })
            .ok_or_else(|| Error::Data(geospatial::diagnostic("terrain warp failed")))
    }

    pub fn geometry(&self) -> Result<([f64; 4], [f64; 2]), Error> {
        let _errors = QuietErrors::new();
        let mut gt = [0.; 6];
        // SAFETY: Queries use a live dataset and a six-element output buffer.
        unsafe {
            if gdal_sys::GDALGetGeoTransform(self.handle.as_ptr(), gt.as_mut_ptr()) != 0 {
                return Err(Error::Data(geospatial::diagnostic(
                    "DEM geotransform unavailable",
                )));
            }
            let width = gdal_sys::GDALGetRasterXSize(self.handle.as_ptr());
            let height = gdal_sys::GDALGetRasterYSize(self.handle.as_ptr());
            let bounds = [
                gt[0],
                gt[3] + gt[5] * height as f64,
                gt[0] + gt[1] * width as f64,
                gt[3],
            ];
            if !bounds.iter().all(|v| v.is_finite())
                || gt[1] <= 0.
                || gt[5] >= 0.
                || gt[2] != 0.
                || gt[4] != 0.
            {
                return Err(Error::Data(
                    "warped DEM must have finite north-up bounds".into(),
                ));
            }
            Ok((bounds, [gt[1], gt[5].abs()]))
        }
    }

    pub fn heights(&self, grid: u16) -> Result<Vec<f64>, Error> {
        let _errors = QuietErrors::new();
        let mut pixels = vec![0_f32; usize::from(grid).pow(2)];
        // SAFETY: The output has exactly grid*grid float32 samples; dimensions
        // are checked before GDAL writes with its packed default spacing.
        unsafe {
            let size = i32::from(grid);
            if gdal_sys::GDALGetRasterXSize(self.handle.as_ptr()) != size
                || gdal_sys::GDALGetRasterYSize(self.handle.as_ptr()) != size
            {
                return Err(Error::Data("terrain sample grid dimensions differ".into()));
            }
            let band = gdal_sys::GDALGetRasterBand(self.handle.as_ptr(), 1);
            if gdal_sys::GDALRasterIO(
                band,
                gdal_sys::GDALRWFlag::GF_Read,
                0,
                0,
                size,
                size,
                pixels.as_mut_ptr().cast(),
                size,
                size,
                gdal_sys::GDALDataType::GDT_Float32,
                0,
                0,
            ) != 0
            {
                return Err(Error::Data(geospatial::diagnostic(
                    "cannot read terrain sample grid",
                )));
            }
        }
        Ok(pixels
            .chunks_exact(usize::from(grid))
            .rev()
            .flat_map(|row| row.iter().map(|&h| f64::from(h)))
            .collect())
    }
}

impl Drop for Dataset<'_> {
    fn drop(&mut self) {
        // SAFETY: This handle is uniquely owned; borrowed derived datasets drop
        // before their source. GDAL closes/reclaims all owned bands and buffers.
        unsafe {
            gdal_sys::GDALClose(self.handle.as_ptr());
        }
    }
}
struct WarpOptions(NonNull<gdal_sys::GDALWarpAppOptions>);
impl Drop for WarpOptions {
    fn drop(&mut self) {
        // SAFETY: Exactly one matching free for the owned options allocation.
        unsafe { gdal_sys::GDALWarpAppOptionsFree(self.0.as_ptr()) };
    }
}
