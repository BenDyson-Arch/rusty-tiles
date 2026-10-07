//! Native GDAL sampling and regular-grid quantized-mesh terrain prototype.
use crate::{
    report::{ConversionResult, Reporter},
    Error,
};
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
    dem_to_terrain_reported(input, output, options, &Reporter::default()).map(drop)
}

/// [`dem_to_terrain`] with `terrain` tile progress sent to `reporter`,
/// returning the published directory and its report.
pub fn dem_to_terrain_reported(
    input: &Path,
    output: &Path,
    options: &TerrainOptions,
    reporter: &Reporter,
) -> Result<ConversionResult, Error> {
    crate::output::require_file(input)?;
    crate::output::check_output(output, options.force)?;
    if options.max_zoom > 24 {
        return Err(Error::Data(format!(
            "--maxZoom must be between 0 and 24, got {}",
            options.max_zoom
        )));
    }
    if ![17, 33, 65, 129].contains(&options.grid) {
        return Err(Error::Data(format!(
            "--grid must be 17, 33, 65 or 129, got {}",
            options.grid
        )));
    }
    if !options.height_offset.is_finite() {
        return Err(Error::Data(
            "--heightOffset must be a finite number of metres".into(),
        ));
    }
    if !options.fill_height.is_finite() {
        return Err(Error::Data(
            "--fillHeight must be a finite number of metres".into(),
        ));
    }
    if !options.max_error.is_finite()
        || options.max_error < 0.
        || options.max_error > f64::from(f32::MAX)
    {
        return Err(Error::Data(
            "--maxError must be a finite, non-negative number of metres within float32 range (0 keeps the full grid)".into(),
        ));
    }
    #[cfg(not(feature = "native-geospatial"))]
    {
        let _ = reporter;
        Err(Error::Environment(
            "terrain requires native GDAL/PROJ; rebuild with --features native-geospatial".into(),
        ))
    }
    #[cfg(feature = "native-geospatial")]
    {
        let job = crate::output::Job::begin(output, options.force)?;
        let staging = job.staging("terrain")?;
        let report = convert(input, &staging, options, reporter)?;
        job.publish_dir(&staging, Some(report))
    }
}

#[cfg(feature = "native-geospatial")]
fn convert(
    input: &Path,
    output: &Path,
    options: &TerrainOptions,
    reporter: &Reporter,
) -> Result<serde_json::Value, Error> {
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
    reporter.progress("terrain", tiles, total);
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
                        reporter,
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
            reporter,
        )?;
    }
    std::fs::write(
        output.join("layer.json"),
        serde_json::to_vec_pretty(&json!({
        "tilejson":"2.1.0","format":"quantized-mesh-1.0","version":"1.0.0","scheme":"tms","projection":"EPSG:4326",
        "minzoom":0,"maxzoom":options.max_zoom,"bounds":bounds,"tiles":["{z}/{x}/{y}.terrain"],"available":available,
        "heightOverlay":{"version":1,"tiles":["{z}/{x}/{y}.heights.json"],"grid":options.grid,"rowOrder":"south-to-north"}}))?,
    )?;
    crate::output::write_report(
        output,
        json!({
        "sourceCrs":source_crs,"heightOffset":options.height_offset,"fillHeight":options.fill_height,"grid":options.grid,"tiles":tiles,
        "heightRange":[low,high],"heightQuantizationStep":(high-low)/32767.,"sourcePixelDegrees":pixel,
        "finestGridDegrees":180./f64::from(1_u32<<options.max_zoom)/f64::from(options.grid-1),
        "simplification":optimization.report(options.max_error),
        "limitations":"Regular-grid sampling prototype with border-locked simplification. NoData/outside filled explicitly. Height datum supplied by caller. Simplification errors are measured against the quantized grid, not a certified bound on the source DEM surface."}),
        true,
    )
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
    reporter: &Reporter,
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
        reporter.progress("terrain", *tiles, total);
    }
    batch.clear();
    Ok(())
}
