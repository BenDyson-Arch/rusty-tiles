//! GDAL-backed raster derivatives. GDAL owns CRS, masks, resampling and codecs.
use crate::Error;
use std::{path::Path, process::Command};

pub fn raster_to_directory(
    input: &Path,
    output: &Path,
    min_zoom: u8,
    max_zoom: u8,
) -> Result<(), Error> {
    if !input.is_file() {
        return Err(Error::InputNotFound(input.into()));
    }
    if output.exists() {
        return Err(Error::OutputExists(output.into()));
    }
    if min_zoom > max_zoom || max_zoom > 24 {
        return Err(Error::msg("require 0 <= minZoom <= maxZoom <= 24"));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let work = tempfile::tempdir_in(parent)?;
    let cog = work.path().join("source.cog.tif");
    let status = Command::new("gdal_translate")
        .args([
            "-of",
            "COG",
            "-co",
            "COMPRESS=ZSTD",
            "-co",
            "NUM_THREADS=ALL_CPUS",
        ])
        .arg(input)
        .arg(&cog)
        .status()?;
    if !status.success() {
        return Err(Error::msg("GDAL COG generation failed"));
    }
    let info = Command::new("gdalinfo").arg("-json").arg(&cog).output()?;
    if !info.status.success() {
        return Err(Error::msg("GDAL could not inspect the generated COG"));
    }
    let info: serde_json::Value = serde_json::from_slice(&info.stdout)?;
    let ring = info["wgs84Extent"]["coordinates"][0]
        .as_array()
        .ok_or_else(|| Error::msg("raster needs a valid georeferenced WGS84 extent"))?;
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for point in ring {
        let x = point[0]
            .as_f64()
            .ok_or_else(|| Error::msg("invalid raster longitude"))?;
        let y = point[1]
            .as_f64()
            .ok_or_else(|| Error::msg("invalid raster latitude"))?;
        bounds[0] = bounds[0].min(x);
        bounds[1] = bounds[1].min(y);
        bounds[2] = bounds[2].max(x);
        bounds[3] = bounds[3].max(y);
    }
    let status = Command::new("gdal")
        .args([
            "raster",
            "tile",
            "--webviewer=none",
            "--tiling-scheme=WebMercatorQuad",
            "--convention=xyz",
            "--output-format=PNG",
        ])
        .arg(format!("--min-zoom={min_zoom}"))
        .arg(format!("--max-zoom={max_zoom}"))
        .arg(&cog)
        .arg(work.path().join("tiles"))
        .status()?;
    if !status.success() {
        return Err(Error::msg("GDAL imagery tiling failed; display tiles require byte imagery. Numeric rasters should retain their COG and use an explicitly styled derivative."));
    }
    std::fs::write(
        work.path().join("tilejson.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "tilejson":"3.0.0", "scheme":"xyz", "tiles":["tiles/{z}/{x}/{y}.png"],
            "minzoom":min_zoom,"maxzoom":max_zoom,"bounds":bounds
        }))?,
    )?;
    // Publish only a complete directory; never replace an existing output.
    if output.exists() {
        return Err(Error::OutputExists(output.into()));
    }
    std::fs::rename(work.path(), output)?;
    Ok(())
}
