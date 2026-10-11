//! Exact integer nearest tiler over the closed finest RGBA raster. Requested
//! zoom z global pixel X with k = 2^(zmax-z) samples finest global pixel
//! kX + k/2 on each axis: an east/south tie selects the greater fine index.
//! Samples outside the finest raster are zero RGBA. Tiles are sequential.
use super::{
    grid::GridPlan,
    native::{self, Dataset},
    source::{exact, invalid, limit},
};
use crate::{runtime::Attempt, JobError, JobErrorKind, JobFailure, RunEvent};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ImageEncoder,
};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};
const SIDE: usize = 256;
const TILE_BYTES: usize = SIDE * SIDE * 4;
/// Fast PNG falls back to stored deflate blocks when compression expands:
/// 262431 zlib bytes plus signature/IHDR/IDAT/IEND framing fit this bound.
const PNG_BYTES: usize = TILE_BYTES + 4096;
/// "/{z}/{x}/{y}.png" below the tiles directory, z <= 24 and x, y < 2^24.
const NAME_BYTES: usize = 3 + 2 + 8 + 8 + 4;
type Encoder<'a> = PngEncoder<&'a mut Cursor<&'a mut [u8]>>;
/// Fixed tiler owners beside the dataset/path/Vec owners charged by the caller.
pub(super) fn owner_bytes() -> usize {
    std::mem::size_of::<Cursor<&mut [u8]>>()
        + std::mem::size_of::<Encoder<'_>>()
        + std::mem::size_of::<[u8; 16]>()
        + std::mem::size_of::<Option<Span>>()
}
/// One finest row, one RGBA tile, one bounded PNG and one tile path capacity.
pub(super) fn scratch_bytes(width: u32, tiles: usize) -> Result<usize, JobError> {
    (width as usize)
        .checked_mul(4)
        .and_then(|n| n.checked_add(TILE_BYTES + PNG_BYTES + NAME_BYTES))
        .and_then(|n| n.checked_add(tiles))
        .ok_or_else(|| limit("raster tile scratch overflow"))
}
/// Local finest index sampled by requested-zoom global pixel `global`.
fn sample(k: u64, global: u64, origin: u64, extent: u64) -> Result<Option<u64>, JobError> {
    let fine = k
        .checked_mul(global)
        .and_then(|n| n.checked_add(k / 2))
        .ok_or_else(|| limit("raster tile sample index overflow"))?;
    Ok(fine.checked_sub(origin).filter(|&n| n < extent))
}
/// Contiguous in-range samples of one tile axis: first tile pixel, count and
/// first local finest index. Later samples step by k finest pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    first: usize,
    count: usize,
    fine: u64,
}
fn span(k: u64, tile: u32, origin: u64, extent: u64) -> Result<Option<Span>, JobError> {
    let base = u64::from(tile) * SIDE as u64;
    let mut found: Option<Span> = None;
    for i in 0..SIDE {
        if let Some(fine) = sample(k, base + i as u64, origin, extent)? {
            match &mut found {
                Some(s) => s.count += 1,
                None => {
                    found = Some(Span {
                        first: i,
                        count: 1,
                        fine,
                    })
                }
            }
        }
    }
    Ok(found)
}
fn decimal<'a>(buffer: &'a mut [u8; 16], value: u32, suffix: &str) -> &'a str {
    let mut cursor = Cursor::new(&mut buffer[..]);
    write!(cursor, "{value}{suffix}").expect("u32 decimal fits");
    let n = cursor.position() as usize;
    std::str::from_utf8(&buffer[..n]).expect("ASCII decimal")
}
fn progress(attempt: &Attempt, done: u64) -> Result<(), JobError> {
    let event = RunEvent::Progress {
        phase: "tiling",
        done,
        total: Some(1_000_000),
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| attempt.emit(&event))) {
        Ok(observed) => observed,
        Err(_) => Err(attempt.check().err().unwrap_or_else(|| {
            JobError::new(
                JobErrorKind::ObserverFailure,
                "panic during raster tiling progress",
            )
        })),
    }
}
fn read_row(ds: &Dataset, x: u64, y: u64, width: usize, row: &mut [u8]) -> Result<(), JobError> {
    let row = &mut row[..width * 4];
    for channel in 0..4 {
        let code = unsafe {
            gdal_sys::GDALRasterIO(
                ds.band(channel + 1),
                0,
                x as i32,
                y as i32,
                width as i32,
                1,
                row.as_mut_ptr().add(channel).cast(),
                width as i32,
                1,
                1,
                4,
                (width * 4) as i32,
            )
        };
        if code != 0 {
            return Err(native::error("read finest RGBA row"));
        }
    }
    Ok(())
}
fn encode(tile: &[u8], png: &mut [u8]) -> Result<usize, JobError> {
    let capacity = png.len();
    let mut cursor = Cursor::new(png);
    let encoded =
        PngEncoder::new_with_quality(&mut cursor, CompressionType::Fast, FilterType::Adaptive)
            .write_image(tile, 256, 256, image::ExtendedColorType::Rgba8);
    let n = cursor.position() as usize;
    match encoded {
        Ok(()) => Ok(n),
        // A fixed cursor refuses growth only once every byte is written.
        Err(_) if n == capacity => Err(limit("encoded raster tile exceeds bounded PNG buffer")),
        Err(e) => Err(JobError::with_cause(
            JobErrorKind::InvalidState,
            "encode raster PNG tile",
            e,
        )),
    }
}
pub(super) fn run(
    input: &Path,
    output: &Path,
    grid: &GridPlan,
    attempt: &Attempt,
    stage: &Path,
    working_cap: u64,
) -> Result<(), JobFailure> {
    let ds = Dataset::open(input).map_err(native::failure)?;
    let result = tiles(&ds, output, grid, attempt, stage, working_cap);
    let close = ds.close();
    match result {
        Ok(()) => close.map_err(native::failure),
        Err(e) => {
            let mut f = native::failure(e);
            native::secondary(&mut f, close);
            Err(f)
        }
    }
}
fn tiles(
    ds: &Dataset,
    output: &Path,
    grid: &GridPlan,
    attempt: &Attempt,
    stage: &Path,
    working_cap: u64,
) -> Result<(), JobError> {
    let target = grid.target();
    if unsafe { gdal_sys::GDALGetRasterXSize(ds.raw()) } != target.width as i32
        || unsafe { gdal_sys::GDALGetRasterYSize(ds.raw()) } != target.height as i32
        || unsafe { gdal_sys::GDALGetRasterCount(ds.raw()) } != 4
        || (1..=4).any(|i| unsafe { gdal_sys::GDALGetRasterDataType(ds.band(i)) } != 1)
    {
        return Err(invalid(
            "closed finest raster differs from exact RGBA GridPlan",
        ));
    }
    let (width, height) = (u64::from(target.width), u64::from(target.height));
    let finest = grid.finest_rect();
    let origin = (
        u64::from(finest.x0) * SIDE as u64,
        u64::from(finest.y0) * SIDE as u64,
    );
    let zmax = finest.z;
    let row_bytes = (target.width as usize)
        .checked_mul(4)
        .ok_or_else(|| limit("finest row overflow"))?;
    let mut row = exact::<u8>(row_bytes)?;
    row.resize(row_bytes, 0);
    let mut tile = exact::<u8>(TILE_BYTES)?;
    tile.resize(TILE_BYTES, 0);
    let mut png = exact::<u8>(PNG_BYTES)?;
    png.resize(PNG_BYTES, 0);
    let capacity = output
        .as_os_str()
        .len()
        .checked_add(NAME_BYTES)
        .ok_or_else(|| limit("tile path overflow"))?;
    let mut path = PathBuf::new();
    path.try_reserve_exact(capacity)
        .map_err(|_| limit("tile path allocation failed"))?;
    if path.capacity() != capacity {
        return Err(limit("tile path requested capacity mismatch"));
    }
    path.push(output);
    fs::create_dir(&path).map_err(|e| JobError::io("create raster tile directory", &path, e))?;
    let mut name = [0u8; 16];
    for r in grid.rectangles() {
        let k = 1u64 << (zmax - r.z);
        let total = (u64::from(r.x1 - r.x0) + 1) * (u64::from(r.y1 - r.y0) + 1);
        let (mut done, mut reported) = (0u64, 0u64);
        path.push(decimal(&mut name, u32::from(r.z), ""));
        fs::create_dir(&path).map_err(|e| JobError::io("create raster zoom", &path, e))?;
        for x in r.x0..=r.x1 {
            path.push(decimal(&mut name, x, ""));
            fs::create_dir(&path).map_err(|e| JobError::io("create raster column", &path, e))?;
            let columns = span(k, x, origin.0, width)?;
            for y in r.y0..=r.y1 {
                attempt.check()?;
                tile.fill(0);
                if let Some(c) = columns {
                    // In-range samples lie inside the finest row: read <= width.
                    let read = (c.count as u64 - 1) * k + 1;
                    let step = k as usize * 4;
                    for i in 0..SIDE {
                        let global = u64::from(y) * SIDE as u64 + i as u64;
                        let Some(fine) = sample(k, global, origin.1, height)? else {
                            continue;
                        };
                        read_row(ds, c.fine, fine, read as usize, &mut row)?;
                        let out = &mut tile[(i * SIDE + c.first) * 4..][..c.count * 4];
                        for (j, pixel) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                            pixel.copy_from_slice(&row[j * step..][..4]);
                        }
                    }
                }
                let n = encode(&tile, &mut png)?;
                path.push(decimal(&mut name, y, ".png"));
                fs::write(&path, &png[..n])
                    .map_err(|e| JobError::io("write raster tile", &path, e))?;
                path.pop();
                done += 1;
                let normalized = done * 1_000_000 / total;
                if normalized > reported {
                    progress(attempt, normalized)?;
                    reported = normalized;
                }
            }
            path.pop();
        }
        path.pop();
        attempt.check()?;
        super::check_closed_workspace(stage, grid.total_tiles(), working_cap)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_integer_sample_selects_greater_tie_and_zero_outside() {
        // Finest zoom copies the window; coarse ties select k*X + k/2.
        assert_eq!(sample(1, 7, 0, 8).unwrap(), Some(7));
        assert_eq!(sample(1, 8, 0, 8).unwrap(), None);
        assert_eq!(sample(2, 0, 0, 8).unwrap(), Some(1));
        assert_eq!(sample(4, 0, 0, 8).unwrap(), Some(2));
        assert_eq!(sample(4, 1, 0, 8).unwrap(), Some(6));
        assert_eq!(sample(4, 2, 0, 8).unwrap(), None);
        // zmax 24 global indices near 2^32 with dz=2.
        let origin = 14_739_536u64 * 256;
        let coarse = 14_739_536u64 / 4 * 256;
        assert_eq!(sample(4, coarse, origin, 1024).unwrap(), Some(2));
        assert_eq!(sample(4, coarse + 255, origin, 1024).unwrap(), Some(1022));
        assert_eq!(sample(4, coarse + 256, origin, 1024).unwrap(), None);
        let last = (1u64 << 32) - 1;
        assert_eq!(sample(1, last, last, 1).unwrap(), Some(0));
        assert_eq!(
            sample(1 << 24, 255, 0, u64::MAX).unwrap(),
            Some((255 << 24) + (1 << 23))
        );
        assert!(sample(u64::MAX, 2, 0, 1).is_err());
        // An origin west of a coarse tile clips its leading samples.
        assert_eq!(
            span(2, 0, 3, 600).unwrap(),
            Some(Span {
                first: 1,
                count: 255,
                fine: 0
            })
        );
        assert_eq!(
            span(2, 1, 0, 600).unwrap(),
            Some(Span {
                first: 0,
                count: 44,
                fine: 513
            })
        );
        assert_eq!(span(2, 2, 0, 600).unwrap(), None);
        assert_eq!(
            span(1, 3, 768, 256).unwrap(),
            Some(Span {
                first: 0,
                count: 256,
                fine: 0
            })
        );
    }
    #[test]
    fn bounded_png_buffer_holds_incompressible_tile_and_decodes() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let tile: Vec<u8> = (0..TILE_BYTES)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect();
        let mut png = vec![0; PNG_BYTES];
        let n = encode(&tile, &mut png).unwrap();
        assert!(n > TILE_BYTES && n <= PNG_BYTES, "{n}");
        let mut again = vec![0; PNG_BYTES];
        assert_eq!(encode(&tile, &mut again).unwrap(), n);
        assert_eq!(png[..n], again[..n]);
        let decoded = image::load_from_memory(&png[..n]).unwrap().into_rgba8();
        assert_eq!(decoded.as_raw(), &tile);
        let mut short = vec![0; 1024];
        assert_eq!(
            encode(&tile, &mut short).unwrap_err().kind(),
            JobErrorKind::ResourceLimit
        );
        let mut name = [0u8; 16];
        assert_eq!(decimal(&mut name, u32::MAX, ".png"), "4294967295.png");
    }
}
