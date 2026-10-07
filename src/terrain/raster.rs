//! DEM inspection and sampling over the shared worker-local GDAL datasets.
//! Warped datasets borrow their source so virtual raster references cannot
//! outlive the underlying dataset.
use crate::{
    geospatial::{
        self,
        native::{self, Arguments, WarpOptions},
        QuietErrors,
    },
    Error,
};
use std::path::Path;

pub(super) struct Dataset<'a>(native::Dataset<'a>);

impl Dataset<'static> {
    pub fn open(path: &Path) -> Result<Self, Error> {
        native::Dataset::open_raster(path, "cannot open DEM").map(Self)
    }
}

impl Dataset<'_> {
    pub fn inspect(&self) -> Result<(String, [f64; 2]), Error> {
        let _errors = QuietErrors::new();
        let projection = self.0.projection();
        if self.0.band_count() != 1 || projection.is_empty() {
            return Err(Error::Data(
                "terrain requires a single-band DEM with a declared CRS".into(),
            ));
        }
        geospatial::Crs::from_definition(&projection)?;
        let band = self.0.band(1)?;
        // SAFETY: Band 1 remains owned by this live dataset throughout the
        // metadata/min-max queries; outputs are stack locals.
        unsafe {
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

    /// Warp with `gdalwarp` arguments into an in-memory (VRT/MEM) dataset.
    pub fn warp<'a, S: AsRef<str>>(&'a self, arguments: &[S]) -> Result<Dataset<'a>, Error> {
        let options = WarpOptions::new(
            &mut Arguments::new(arguments)?,
            "invalid terrain warp options",
        )?;
        self.0
            .warp(Path::new(""), &options, "terrain warp failed")
            .map(Dataset)
    }

    pub fn geometry(&self) -> Result<([f64; 4], [f64; 2]), Error> {
        let _errors = QuietErrors::new();
        let gt = self
            .0
            .geo_transform()
            .ok_or_else(|| Error::Data(geospatial::diagnostic("DEM geotransform unavailable")))?;
        let [width, height] = self.0.size();
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

    pub fn heights(&self, grid: u16) -> Result<Vec<f64>, Error> {
        let _errors = QuietErrors::new();
        let size = i32::from(grid);
        if self.0.size() != [size, size] {
            return Err(Error::Data("terrain sample grid dimensions differ".into()));
        }
        let band = self.0.band(1)?;
        let mut pixels = vec![0_f32; usize::from(grid).pow(2)];
        // SAFETY: The output has exactly grid*grid float32 samples; dimensions
        // were checked above and GDAL writes with its packed default spacing.
        let code = unsafe {
            gdal_sys::GDALRasterIO(
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
            )
        };
        if code != 0 {
            return Err(Error::Data(geospatial::diagnostic(
                "cannot read terrain sample grid",
            )));
        }
        Ok(pixels
            .chunks_exact(usize::from(grid))
            .rev()
            .flat_map(|row| row.iter().map(|&h| f64::from(h)))
            .collect())
    }
}
