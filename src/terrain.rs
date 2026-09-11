//! GDAL-backed quantized-mesh lab encoder. See the embedded script for limits.
use crate::Error;
use std::{path::Path, process::Command};

pub struct TerrainOptions {
    pub max_zoom: u8,
    pub grid: u16,
    pub height_offset: f64,
    pub fill_height: f64,
}

pub fn dem_to_terrain(input: &Path, output: &Path, options: &TerrainOptions) -> Result<(), Error> {
    if !input.is_file() {
        return Err(Error::InputNotFound(input.into()));
    }
    if output.exists() {
        return Err(Error::OutputExists(output.into()));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let work = tempfile::tempdir_in(parent)?;
    let status = Command::new("python3")
        .arg("-c")
        .arg(include_str!("../scripts/terrain.py"))
        .arg(input)
        .arg(work.path())
        .arg("--max-zoom")
        .arg(options.max_zoom.to_string())
        .arg("--grid")
        .arg(options.grid.to_string())
        .arg("--height-offset")
        .arg(options.height_offset.to_string())
        .arg("--fill-height")
        .arg(options.fill_height.to_string())
        .status()?;
    if !status.success() {
        return Err(Error::msg(
            "DEM conversion failed; Python GDAL and NumPy are required. No output published.",
        ));
    }
    if output.exists() {
        return Err(Error::OutputExists(output.into()));
    }
    std::fs::rename(work.path(), output)?;
    Ok(())
}
