//! GDAL-backed source COG and explicitly styled imagery derivatives.
use crate::Error;
use std::{path::Path, process::Command};

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
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let work = tempfile::tempdir_in(parent)?;
    let mut cmd = Command::new("python3");
    cmd.arg("-c")
        .arg(crate::python::script(
            include_str!("../scripts/raster.py"),
            "raster",
            "Python GDAL and NumPy",
        )?)
        .arg(input)
        .arg(work.path())
        .arg("--min-zoom")
        .arg(options.min_zoom.to_string())
        .arg("--max-zoom")
        .arg(options.max_zoom.to_string())
        .arg("--display")
        .arg(&options.display)
        .arg("--band")
        .arg(options.band.to_string())
        .arg("--alpha-band")
        .arg(options.alpha_band.to_string());
    if let Some(v) = options.display_min {
        cmd.arg("--display-min").arg(v.to_string());
    }
    if let Some(v) = options.display_max {
        cmd.arg("--display-max").arg(v.to_string());
    }
    crate::python::run(&mut cmd, "raster")?;
    crate::output::publish_directory(work.path(), output, options.force)?;
    Ok(())
}
