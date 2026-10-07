//! Native GDAL sampling and regular-grid quantized-mesh terrain prototype.
use crate::Error;
use std::path::Path;
#[cfg(feature = "native-geospatial")]
mod quantized;
#[cfg(feature = "native-geospatial")]
mod raster;
#[cfg(feature = "native-geospatial")]
mod simplify;

pub struct TerrainOptions {
    pub force: bool,
    pub max_zoom: u8,
    pub grid: u16,
    pub height_offset: f64,
    pub fill_height: f64,
    /// Maximum added simplification error in metres; zero retains the full grid.
    pub max_error: f64,
}

pub fn dem_to_terrain(input: &Path, output: &Path, options: &TerrainOptions) -> Result<(), Error> {
    if !input.is_file() {
        return Err(Error::InputNotFound(input.into()));
    }
    if output.exists() && !options.force {
        return Err(Error::OutputExists(output.into()));
    }
    if options.max_zoom > 24 || ![17, 33, 65, 129].contains(&options.grid) {
        return Err(Error::Data(
            "maxZoom must be 0..24; grid must be 17, 33, 65 or 129".into(),
        ));
    }
    if !options.height_offset.is_finite() || !options.fill_height.is_finite() {
        return Err(Error::Data("heights must be finite".into()));
    }
    if !options.max_error.is_finite()
        || options.max_error < 0.
        || options.max_error > f64::from(f32::MAX)
    {
        return Err(Error::Data(
            "maxError must be finite, non-negative and fit float32 metres".into(),
        ));
    }
    #[cfg(not(feature = "native-geospatial"))]
    {
        Err(Error::Environment(
            "terrain requires native GDAL/PROJ; rebuild with --features native-geospatial".into(),
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
        convert(input, work.path(), options)?;
        crate::output::publish_directory(work.path(), output, options.force)?;
        Ok(())
    }
}

#[cfg(feature = "native-geospatial")]
fn convert(input: &Path, output: &Path, options: &TerrainOptions) -> Result<(), Error> {
    use serde_json::json;
    let source = raster::Dataset::open(input)?;
    let (source_crs, range) = source.inspect()?;
    let low = ((range[0] + options.height_offset).min(options.fill_height) as f32) as f64;
    let high = ((range[1] + options.height_offset).max(options.fill_height) as f32) as f64;
    if !low.is_finite() || !high.is_finite() || low > high {
        return Err(Error::Data(
            "terrain height range must fit finite float32 endpoints".into(),
        ));
    }
    let common = [
        "-ot",
        "Float32",
        "-dstnodata",
        "nan",
        "-wm",
        "64",
        "-novshift",
        "-to",
        "ALLOW_BALLPARK=NO",
        "-to",
        "ONLY_BEST=YES",
    ];
    let vrt_args: Vec<_> = ["-of", "VRT", "-t_srs", "EPSG:4326"]
        .into_iter()
        .chain(common)
        .map(str::to_owned)
        .collect();
    let vrt = source.warp(&vrt_args)?;
    let (bounds, pixel) = vrt.geometry()?;
    if bounds[2] - bounds[0] >= 180.
        || bounds[0] < -180.
        || bounds[2] > 180.
        || bounds[0] >= bounds[2]
    {
        return Err(Error::Data(
            "antimeridian/global DEMs require a split before this lab conversion".into(),
        ));
    }
    if bounds[1] < -90. || bounds[3] > 90. || bounds[1] >= bounds[3] {
        return Err(Error::Data(
            "DEM latitude bounds must be within -90..90".into(),
        ));
    }
    // Count every level before emitting any tile. This cap also bounds disk
    // output; sampling and encoding hold just one <=129*129 grid at a time.
    let mut levels = Vec::new();
    let mut total = 0_u64;
    for z in 0..=options.max_zoom {
        let n = 1_u32 << z;
        let size = 180. / f64::from(n);
        let (x0, x1, y0, y1) = if z == 0 {
            (0, 1, 0, 0)
        } else {
            (
                ((bounds[0] + 180.) / size).floor().max(0.) as u32,
                (((bounds[2] + 180.) / size).floor() as u32).min(n * 2 - 1),
                ((bounds[1] + 90.) / size).floor().max(0.) as u32,
                (((bounds[3] + 90.) / size).floor() as u32).min(n - 1),
            )
        };
        total += u64::from(x1 - x0 + 1) * u64::from(y1 - y0 + 1);
        if total > 100_000 {
            return Err(Error::Data("tile count exceeds --maxTiles=100000".into()));
        }
        levels.push((size, x0, x1, y0, y1));
    }
    let mut available = Vec::new();
    let mut tiles = 0_u64;
    let mut optimization = quantized::Statistics::default();
    // GDAL sampling stays on this thread. Only owned Rust grids enter Rayon;
    // cap queued encoders so larger DEMs do not multiply GDAL warp/cache memory.
    let batch_size = if options.max_error == 0. {
        1
    } else {
        rayon::current_num_threads().clamp(1, 4)
    };
    let mut batch = Vec::with_capacity(batch_size);
    progress(tiles, total);
    for (z, &(size, x0, x1, y0, y1)) in levels.iter().enumerate() {
        available.push(json!([{"startX":x0,"startY":y0,"endX":x1,"endY":y1}]));
        for x in x0..=x1 {
            let folder = output.join(z.to_string()).join(x.to_string());
            std::fs::create_dir_all(&folder)?;
            for y in y0..=y1 {
                let west = -180. + f64::from(x) * size;
                let south = -90. + f64::from(y) * size;
                let step = size / f64::from(options.grid - 1);
                let mut arguments: Vec<_> = ["-of", "MEM", "-r", "bilinear"]
                    .into_iter()
                    .chain(common)
                    .map(str::to_owned)
                    .collect();
                arguments.push("-te".into());
                arguments.extend(
                    [
                        west - step / 2.,
                        south - step / 2.,
                        west + size + step / 2.,
                        south + size + step / 2.,
                    ]
                    .map(|v| v.to_string()),
                );
                arguments.extend([
                    "-ts".into(),
                    options.grid.to_string(),
                    options.grid.to_string(),
                ]);
                let tile = vrt.warp(&arguments)?;
                let samples = tile.heights(options.grid)?;
                let overlay: Vec<_> = samples
                    .iter()
                    .map(|&h| h.is_finite().then_some(h + options.height_offset))
                    .collect();
                let heights: Vec<_> = overlay
                    .iter()
                    .map(|h| h.unwrap_or(options.fill_height))
                    .collect();
                std::fs::write(
                    folder.join(format!("{y}.heights.json")),
                    serde_json::to_vec(
                        &json!({"width":options.grid,"height":options.grid,"heights":overlay}),
                    )?,
                )?;
                batch.push(SampledTile {
                    path: folder.join(format!("{y}.terrain")),
                    west,
                    south,
                    size,
                    heights,
                });
                if batch.len() == batch_size {
                    flush_batch(
                        &mut batch,
                        options,
                        [low, high],
                        &mut optimization,
                        &mut tiles,
                        total,
                    )?;
                }
            }
        }
        flush_batch(
            &mut batch,
            options,
            [low, high],
            &mut optimization,
            &mut tiles,
            total,
        )?;
        if std::env::var_os("RUSTY_TILES_PROGRESS_JSON").is_none() {
            eprintln!(
                "terrain level {z}: {} tiles",
                u64::from(x1 - x0 + 1) * u64::from(y1 - y0 + 1)
            );
        }
    }
    std::fs::write(
        output.join("layer.json"),
        serde_json::to_vec_pretty(&json!({
        "tilejson":"2.1.0","format":"quantized-mesh-1.0","version":"1.0.0","scheme":"tms","projection":"EPSG:4326",
        "minzoom":0,"maxzoom":options.max_zoom,"bounds":bounds,"tiles":["{z}/{x}/{y}.terrain"],"available":available,
        "heightOverlay":{"version":1,"tiles":["{z}/{x}/{y}.heights.json"],"grid":options.grid,"rowOrder":"south-to-north"}}))?,
    )?;
    std::fs::write(
        output.join("conversion.json"),
        serde_json::to_vec_pretty(&json!({
        "sourceCrs":source_crs,"heightOffset":options.height_offset,"fillHeight":options.fill_height,"grid":options.grid,"tiles":tiles,
        "heightRange":[low,high],"heightQuantizationStep":(high-low)/32767.,"sourcePixelDegrees":pixel,
        "finestGridDegrees":180./f64::from(1_u32<<options.max_zoom)/f64::from(options.grid-1),
        "simplification":optimization.report(options.max_error),
        "limitations":"Regular-grid sampling prototype with border-locked simplification. NoData/outside filled explicitly. Height datum supplied by caller. Simplification errors are measured against the quantized grid, not a certified bound on the source DEM surface."}))?,
    )?;
    Ok(())
}

#[cfg(feature = "native-geospatial")]
struct SampledTile {
    path: std::path::PathBuf,
    west: f64,
    south: f64,
    size: f64,
    heights: Vec<f64>,
}

#[cfg(feature = "native-geospatial")]
fn flush_batch(
    batch: &mut Vec<SampledTile>,
    options: &TerrainOptions,
    range: [f64; 2],
    optimization: &mut quantized::Statistics,
    tiles: &mut u64,
    total: u64,
) -> Result<(), Error> {
    use rayon::prelude::*;
    let encode = |tile: &SampledTile| {
        quantized::encode(
            tile.west,
            tile.south,
            tile.size,
            &tile.heights,
            options.grid,
            range[0],
            range[1],
            options.max_error,
        )
    };
    let encoded: Result<Vec<_>, Error> = if batch.len() <= 1 {
        batch.iter().map(encode).collect()
    } else {
        batch.par_iter().map(encode).collect()
    };
    // Publication, reports and progress stay in input order regardless of the
    // encoding schedule. Each batch holds at most four sampled/encoded grids.
    for (tile, encoded) in batch.iter().zip(encoded?) {
        std::fs::write(&tile.path, encoded.bytes)?;
        optimization.add(&encoded.statistics);
        *tiles += 1;
        progress(*tiles, total);
    }
    batch.clear();
    Ok(())
}

#[cfg(feature = "native-geospatial")]
fn progress(done: u64, total: u64) {
    if std::env::var_os("RUSTY_TILES_PROGRESS_JSON").is_some() {
        eprintln!(
            "{}",
            serde_json::json!({"event":"progress","phase":"terrain","done":done,"total":total})
        );
    }
}
