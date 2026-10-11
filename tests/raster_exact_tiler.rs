//! Exact integer nearest pyramid sampling. Its own test binary: fixture GDAL
//! registration must not race in-process converter registration.
#![cfg(feature = "native-geospatial")]
use rusty_tiles::{raster_to_pyramid, RasterRequest, RunControl};
use std::fs;
/// Pixel-addressed EPSG:3857 source aligned to the max-zoom grid: R/G hold the
/// low column/row byte and B their high bits, so each tile pixel names the
/// finest source pixel it sampled. Coarse zoom global pixel X with
/// k = 2^(zmax-z) must sample finest kX + k/2; outside is zero RGBA.
#[test]
fn coarse_zooms_sample_exact_greater_tie_finest_pixels() {
    use std::{ffi::CString, ptr::null_mut};
    const H: f64 = 20_037_508.342789244;
    static REGISTER: std::sync::Once = std::sync::Once::new();
    // SAFETY: registration finishes once before any fixture uses GDAL.
    REGISTER.call_once(|| unsafe { gdal_sys::GDALAllRegister() });
    for (zmax, x0, y0, width, height) in [
        (24u8, 14_739_536u64, 5_600_000u64, 512usize, 512usize),
        (16, 36_741, 21_001, 768, 512),
    ] {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("source.tif");
        let res = H / 2f64.powi(7 + i32::from(zmax));
        let mut bands = [
            vec![0u8; width * height],
            vec![0u8; width * height],
            vec![0u8; width * height],
        ];
        for row in 0..height {
            for col in 0..width {
                let i = row * width + col;
                bands[0][i] = (col & 255) as u8;
                bands[1][i] = (row & 255) as u8;
                bands[2][i] = ((col >> 8) | ((row >> 8) << 2)) as u8;
            }
        }
        let filename = CString::new(input.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: Owned dataset/SRS handles closed here; buffers match the
        // dataset dimensions and Byte type; strings are terminated and live.
        unsafe {
            let ds = gdal_sys::GDALCreate(
                gdal_sys::GDALGetDriverByName(c"GTiff".as_ptr()),
                filename.as_ptr(),
                width as i32,
                height as i32,
                3,
                1,
                null_mut(),
            );
            assert!(!ds.is_null());
            let mut gt = [
                -H + (x0 * 256) as f64 * res,
                res,
                0.,
                H - (y0 * 256) as f64 * res,
                0.,
                -res,
            ];
            assert_eq!(gdal_sys::GDALSetGeoTransform(ds, gt.as_mut_ptr()), 0);
            let srs = gdal_sys::OSRNewSpatialReference(std::ptr::null());
            assert_eq!(gdal_sys::OSRImportFromEPSG(srs, 3857), 0);
            assert_eq!(gdal_sys::GDALSetSpatialRef(ds, srs), 0);
            gdal_sys::OSRDestroySpatialReference(srs);
            for (index, pixels) in bands.iter_mut().enumerate() {
                let band = gdal_sys::GDALGetRasterBand(ds, index as i32 + 1);
                assert_eq!(
                    gdal_sys::GDALSetRasterColorInterpretation(band, 3 + index as u32),
                    0
                );
                assert_eq!(
                    gdal_sys::GDALRasterIO(
                        band,
                        1,
                        0,
                        0,
                        width as i32,
                        height as i32,
                        pixels.as_mut_ptr().cast(),
                        width as i32,
                        height as i32,
                        1,
                        0,
                        0
                    ),
                    0
                );
            }
            assert_eq!(gdal_sys::GDALClose(ds), 0);
        }
        let output = root.path().join("result");
        let result = raster_to_pyramid(
            RasterRequest::image(&input, &output, zmax - 2, zmax),
            &RunControl::default(),
        )
        .unwrap();
        assert_eq!(
            result.report.resampling,
            "nearest-direct-finest-east-south-ties"
        );
        // The outward cover may add neighbours; it must contain the source.
        let cover = result.report.grid.finest_tiles.map(u64::from);
        assert!(cover[0] <= x0 && cover[1] <= y0);
        assert!(cover[2] >= x0 + width as u64 / 256 - 1);
        assert!(cover[3] >= y0 + height as u64 / 256 - 1);
        let mut tiles = 0;
        let (mut checked, mut padding) = (0u64, 0u64);
        for dz in 0..=2u8 {
            let (z, k) = (zmax - dz, 1u64 << dz);
            let xs = (cover[0] >> dz)..=(cover[2] >> dz);
            let ys = (cover[1] >> dz)..=(cover[3] >> dz);
            for tx in xs.clone() {
                for ty in ys.clone() {
                    tiles += 1;
                    let path = output.join(format!("tiles/{z}/{tx}/{ty}.png"));
                    let tile = image::open(&path).unwrap().into_rgba8();
                    assert_eq!(tile.dimensions(), (256, 256));
                    for (px, py, pixel) in tile.enumerate_pixels() {
                        let ex = (k * (tx * 256 + u64::from(px)) + k / 2).checked_sub(x0 * 256);
                        let ey = (k * (ty * 256 + u64::from(py)) + k / 2).checked_sub(y0 * 256);
                        let expected = match (ex, ey) {
                            (Some(ex), Some(ey)) if ex < width as u64 && ey < height as u64 => {
                                checked += 1;
                                [
                                    (ex & 255) as u8,
                                    (ey & 255) as u8,
                                    ((ex >> 8) | ((ey >> 8) << 2)) as u8,
                                    255,
                                ]
                            }
                            _ => {
                                padding += 1;
                                [0; 4]
                            }
                        };
                        assert_eq!(
                            pixel.0, expected,
                            "z{z} tile {tx}/{ty} pixel {px},{py} (k={k})"
                        );
                    }
                }
            }
            assert_eq!(
                fs::read_dir(output.join(format!("tiles/{z}")))
                    .unwrap()
                    .map(|c| fs::read_dir(c.unwrap().path()).unwrap().count())
                    .sum::<usize>(),
                xs.count() * ys.count()
            );
        }
        assert_eq!(result.report.tiles, tiles);
        let pixels = (width * height) as u64;
        assert_eq!(checked, pixels + pixels / 4 + pixels / 16);
        assert!(padding > 0, "coarse tiles extend beyond the finest raster");
    }
}
