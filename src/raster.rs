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
    if options.max_zoom > 24 {
        return Err(Error::Data(format!(
            "--maxZoom must be between 0 and 24, got {}",
            options.max_zoom
        )));
    }
    if options.min_zoom > options.max_zoom {
        return Err(Error::Data(format!(
            "--minZoom ({}) must not exceed --maxZoom ({})",
            options.min_zoom, options.max_zoom
        )));
    }
    match options.display.as_str() {
        "gray" => {
            if options.alpha_band != 0 {
                return Err(Error::Data(
                    "--alphaBand applies only to --display image; gray display uses the selected band's mask/NoData".into(),
                ));
            }
            let (Some(low), Some(high)) = (options.display_min, options.display_max) else {
                return Err(Error::Data(
                    "--display gray requires both --displayMin and --displayMax".into(),
                ));
            };
            if !low.is_finite() || !high.is_finite() {
                return Err(Error::Data(
                    "--displayMin and --displayMax must be finite numbers".into(),
                ));
            }
            if low >= high {
                return Err(Error::Data(format!(
                    "--displayMin ({low}) must be less than --displayMax ({high})"
                )));
            }
        }
        "image" => {
            if options.display_min.is_some() || options.display_max.is_some() {
                return Err(Error::Data(
                    "--displayMin/--displayMax require --display gray".into(),
                ));
            }
        }
        other => {
            return Err(Error::Data(format!(
                "--display must be image or gray, got {other:?}"
            )))
        }
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
