//! Later: DEM / raster → 3D terrain tiles.
//!
//! COLMAP is out of scope (hub `CONTEXT.md`). COG/PMTiles stay in Go+GDAL until
//! this crate has a clear win.

use std::path::Path;

use crate::error::Error;

pub fn dem_to_terrain(_input: &Path, _output: &Path) -> Result<(), Error> {
    Err(Error::NotImplemented {
        feature: "terrain",
        hint: "quantized-mesh or 3D Tiles terrain from DEM; not scheduled in v0. Raster COG/PMTiles remain tinyowl-server + GDAL.".into(),
    })
}
