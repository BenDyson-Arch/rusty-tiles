//! GDAL-backed source COG and explicitly styled imagery derivatives.
use crate::Error;
use std::path::Path;
#[cfg(feature = "native-geospatial")]
mod native;

pub struct RasterOptions {
    pub force: bool,
    pub min_zoom: u8,
    pub max_zoom: u8,
    pub display: String,
    pub band: u16,
    pub alpha_band: u16,
    pub display_min: Option<f64>,
    pub display_max: Option<f64>,
}

#[cfg(feature = "native-geospatial")]
pub(crate) fn tile_available() -> Result<(), Error> {
    native::tile_available()
}

pub fn raster_to_directory(
    input: &Path,
    output: &Path,
    min_zoom: u8,
    max_zoom: u8,
) -> Result<(), Error> {
    raster_with_options(
        input,
        output,
        &RasterOptions {
            force: false,
            min_zoom,
            max_zoom,
            display: "image".into(),
            band: 1,
            alpha_band: 0,
            display_min: None,
            display_max: None,
        },
    )
}

pub fn raster_with_options(
    input: &Path,
    output: &Path,
    options: &RasterOptions,
) -> Result<(), Error> {
    if !input.is_file() {
        return Err(Error::InputNotFound(input.into()));
    }
    if output.exists() && !options.force {
        return Err(Error::OutputExists(output.into()));
    }
    if options.min_zoom > options.max_zoom || options.max_zoom > 24 {
        return Err(Error::Data("require 0 <= minZoom <= maxZoom <= 24".into()));
    }
    match options.display.as_str() {
        "gray" => {
            if options.alpha_band != 0 {
                return Err(Error::Data("gray display uses the selected band mask/NoData; alphaBand is for image display".into()));
            }
            match (options.display_min, options.display_max) {
                (Some(low), Some(high)) if low.is_finite() && high.is_finite() && low < high => {}
                _ => {
                    return Err(Error::Data(
                        "numeric display requires a finite increasing range".into(),
                    ))
                }
            }
        }
        "image" => {
            if options.display_min.is_some() || options.display_max.is_some() {
                return Err(Error::Data(
                    "displayMin/displayMax require gray display".into(),
                ));
            }
        }
        _ => return Err(Error::Data("display must be image or gray".into())),
    }
    #[cfg(not(feature = "native-geospatial"))]
    {
        Err(Error::Environment(
            "raster requires native GDAL/PROJ; rebuild with --features native-geospatial".into(),
        ))
    }
    #[cfg(feature = "native-geospatial")]
    {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let work = tempfile::tempdir_in(parent)?;
        native::convert(input, work.path(), options)?;
        crate::output::publish_directory(work.path(), output, options.force)?;
        Ok(())
    }
}
