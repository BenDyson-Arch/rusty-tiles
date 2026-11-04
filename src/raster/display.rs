//! One selected opacity and bounded typed windows, independent of tile phase.
use super::{
    native::{self, Dataset},
    source::{self, SampleType, ScalarFact, SourceFacts},
    RasterDisplay,
};
use crate::{runtime::Attempt, JobError, JobFailure};
use source::{exact, invalid, unsupported};
use std::{ffi::c_void, path::Path};
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct RoleRecipe {
    pub data: [u8; 3],
    pub count: u8,
    pub alpha: u8,
    pub image: bool,
    pub palette: bool,
    pub low: f64,
    pub high: f64,
}
impl RoleRecipe {
    pub(super) fn resolve(f: &SourceFacts, d: &RasterDisplay) -> Result<Self, JobError> {
        let (data, count, image, low, high, explicit) = match *d {
            RasterDisplay::Image { alpha_band } => {
                if f.bands[0].datatype != SampleType::Byte {
                    return Err(unsupported("image display requires Byte source"));
                }
                let rgb = f.bands[0].color_interp != 2
                    && f.bands.iter().filter(|b| b.color_interp != 6).count() >= 3;
                if rgb {
                    for (band, role) in f.bands[..3].iter().zip([3, 4, 5]) {
                        if band.color_interp != 0 && band.color_interp != role {
                            return Err(unsupported("contradictory defined RGB image role"));
                        }
                    }
                } else if !matches!(f.bands[0].color_interp, 0..=2) {
                    return Err(unsupported("contradictory first gray/palette image role"));
                }
                (
                    if rgb { [1, 2, 3] } else { [1, 0, 0] },
                    if rgb { 3 } else { 1 },
                    true,
                    0.,
                    255.,
                    alpha_band,
                )
            }
            RasterDisplay::Gray {
                band,
                low,
                high,
                alpha_band,
            } => {
                if band == 0 || band as usize > f.bands.len() {
                    return Err(invalid("gray band index"));
                }
                if !low.is_finite() || !high.is_finite() || low >= high || !(high - low).is_finite()
                {
                    return Err(invalid("gray endpoints require finite increasing width"));
                }
                ([band as u8, 0, 0], 1, false, low, high, alpha_band)
            }
        };
        let declared = f
            .bands
            .iter()
            .position(|b| b.color_interp == 6)
            .map(|i| (i + 1) as u16);
        let alpha = explicit.or(declared).unwrap_or(0);
        if alpha as usize > f.bands.len() || explicit == Some(0) {
            return Err(invalid("alpha band index"));
        }
        let palette = image && f.bands[0].color_interp == 2;
        if palette && f.bands[0].palette.len == 0 {
            return Err(invalid("palette image has no palette"));
        }
        Ok(Self {
            data,
            count,
            alpha: alpha as u8,
            image,
            palette,
            low,
            high,
        })
    }
    pub(super) fn width(self, f: &SourceFacts) -> usize {
        self.data[..self.count as usize]
            .iter()
            .copied()
            .chain((self.alpha != 0).then_some(self.alpha))
            .map(|i| f.bands[i as usize - 1].datatype.bytes())
            .max()
            .unwrap_or(1)
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct Window {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl Window {
    fn pixels(self) -> usize {
        self.width as usize * self.height as usize
    }
}
pub(super) fn window_pixels(f: &SourceFacts) -> usize {
    f.width.min(256) as usize * f.height.min(256) as usize
}
pub(super) fn words(pixels: usize, width: usize) -> usize {
    (pixels * width).div_ceil(8)
}
pub(super) fn read(
    ds: &Dataset,
    index: usize,
    w: Window,
    ty: SampleType,
    target: *mut c_void,
) -> Result<(), JobError> {
    let b = ds.band(index);
    if b.is_null() {
        return Err(invalid("raster band missing"));
    }
    let result = unsafe {
        gdal_sys::GDALRasterIO(
            b,
            0,
            w.x as i32,
            w.y as i32,
            w.width as i32,
            w.height as i32,
            target,
            w.width as i32,
            w.height as i32,
            ty as u32,
            0,
            0,
        )
    };
    if result != 0 {
        Err(native::error("read raster window"))
    } else {
        Ok(())
    }
}
fn read_mask(ds: &Dataset, index: usize, w: Window, target: &mut [u8]) -> Result<(), JobError> {
    let b = unsafe { gdal_sys::GDALGetMaskBand(ds.band(index)) };
    if b.is_null() {
        return Err(invalid("raster mask missing"));
    }
    if unsafe {
        gdal_sys::GDALRasterIO(
            b,
            0,
            w.x as i32,
            w.y as i32,
            w.width as i32,
            w.height as i32,
            target.as_mut_ptr().cast(),
            w.width as i32,
            w.height as i32,
            1,
            0,
            0,
        )
    } != 0
    {
        Err(native::error("read raster mask window"))
    } else {
        Ok(())
    }
}
/// Aligned typed native storage, never reinterpreted as a Rust typed slice.
pub(super) fn value(storage: &[u64], index: usize, ty: SampleType) -> f64 {
    let p = storage.as_ptr().cast::<u8>();
    unsafe {
        match ty {
            SampleType::Byte => *p.add(index) as f64,
            SampleType::I8 => *(p.add(index).cast::<i8>()) as f64,
            SampleType::U16 => *p.add(index * 2).cast::<u16>() as f64,
            SampleType::I16 => *p.add(index * 2).cast::<i16>() as f64,
            SampleType::U32 => *p.add(index * 4).cast::<u32>() as f64,
            SampleType::I32 => *p.add(index * 4).cast::<i32>() as f64,
            SampleType::F32 => *p.add(index * 4).cast::<f32>() as f64,
            SampleType::F64 => *p.add(index * 8).cast::<f64>(),
        }
    }
}
pub(super) fn nodata(v: f64, d: f64, ty: SampleType) -> bool {
    if d.is_nan() {
        return v.is_nan();
    }
    if v == d {
        return true;
    }
    match ty {
        SampleType::F32 => {
            let a = v as f32;
            let b = d as f32;
            (a - b).abs() < f32::EPSILON * (a + b).abs() * 2.
        }
        SampleType::F64 => (v - d).abs() < f32::EPSILON as f64 * (v + d).abs() * 2.,
        _ => false,
    }
}
fn channel(v: f64, low: f64, high: f64) -> u8 {
    if v <= low {
        0
    } else if v >= high {
        255
    } else {
        (255. * ((v - low) / (high - low)) + 0.5).floor() as u8
    }
}
fn alpha(v: f64) -> u8 {
    (v.clamp(0., 255.) + 0.5).floor() as u8
}
fn windows(
    f: &SourceFacts,
    mut run: impl FnMut(Window) -> Result<(), JobError>,
) -> Result<(), JobError> {
    for y in (0..f.height).step_by(256) {
        for x in (0..f.width).step_by(256) {
            run(Window {
                x,
                y,
                width: (f.width - x).min(256),
                height: (f.height - y).min(256),
            })?;
        }
    }
    Ok(())
}
/// All bands and independent boolean validity are compared before original close.
pub(super) fn coherence(
    source: &Dataset,
    cog: &Dataset,
    f: &SourceFacts,
    attempt: &Attempt,
) -> Result<(), JobError> {
    #[cfg(test)]
    super::enter_phase(1);
    let m = window_pixels(f);
    let ty = f.bands[0].datatype;
    let mut a = exact(words(m, ty.bytes()))?;
    a.resize(a.capacity(), 0u64);
    let mut b = exact(a.len())?;
    b.resize(b.capacity(), 0u64);
    let mut ma = exact(m)?;
    ma.resize(m, 0u8);
    let mut mb = exact(m)?;
    mb.resize(m, 0u8);
    windows(f, |w| {
        attempt.check()?;
        for (index, band) in f.bands.iter().enumerate() {
            read(source, index + 1, w, ty, a.as_mut_ptr().cast())?;
            read(cog, index + 1, w, ty, b.as_mut_ptr().cast())?;
            let n = w.pixels();
            let aa = unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<u8>(), n * ty.bytes()) };
            let bb = unsafe { std::slice::from_raw_parts(b.as_ptr().cast::<u8>(), n * ty.bytes()) };
            for i in 0..n {
                let start = i * ty.bytes();
                if aa[start..start + ty.bytes()] != bb[start..start + ty.bytes()]
                    && !(value(&a, i, ty).is_nan() && value(&b, i, ty).is_nan())
                {
                    return Err(invalid("closed COG changed a logical source sample"));
                }
            }
            if boolean_mask(band.mask_class) {
                read_mask(source, index + 1, w, &mut ma)?;
                read_mask(cog, index + 1, w, &mut mb)?;
                if (0..n).any(|i| (ma[i] != 0) != (mb[i] != 0)) {
                    return Err(invalid("closed COG changed boolean source validity"));
                }
            }
        }
        Ok(())
    })
}
fn boolean_mask(class: u8) -> bool {
    matches!(class, 2 | 8 | 10)
}
fn mark_mask(flags: &mut u8, mask: u8) {
    if mask == 0 {
        *flags &= !1;
    }
}
fn mark_sample(flags: &mut u8, v: f64, ty: SampleType, nd: ScalarFact) {
    if nd.present && nodata(v, nd.value(), ty) {
        *flags &= !1;
    }
    if !v.is_finite() {
        *flags |= 2;
    }
}
fn finish_pixel(flags: u8, rgba: &mut [u8]) -> Result<(), JobError> {
    if flags & 1 == 0 {
        rgba.fill(0);
        return Ok(());
    }
    if flags & 2 != 0 {
        return Err(invalid("unmasked selected source sample is nonfinite"));
    }
    if flags & 4 != 0 {
        return Err(invalid("source palette index exceeds defined palette"));
    }
    if rgba[3] == 0 {
        rgba.fill(0);
    }
    Ok(())
}

/// Buffers die before Warp/tiling. Source pixels live on disk, not in full arrays.
pub(super) fn render(
    cog: &Dataset,
    f: &SourceFacts,
    roles: RoleRecipe,
    output: &Path,
    attempt: &Attempt,
) -> Result<(), JobFailure> {
    #[cfg(test)]
    super::enter_phase(2);
    let m = window_pixels(f);
    let mut samples = exact(words(m, roles.width(f))).map_err(native::failure)?;
    samples.resize(samples.capacity(), 0u64);
    let mut mask = exact(m).map_err(native::failure)?;
    mask.resize(m, 0u8);
    let mut flags = exact(m).map_err(native::failure)?;
    flags.resize(m, 0u8);
    let mut rgba = exact(4 * m).map_err(native::failure)?;
    rgba.resize(4 * m, 0u8);
    let out = native::create_rgba(output, f.width, f.height, f.affine, Some(cog))?;
    let result = windows(f, |w| {
        attempt.check()?;
        let n = w.pixels();
        flags[..n].fill(1);
        rgba[..4 * n].fill(0);
        for pixel in rgba[..4 * n].as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        for selected in 0..roles.count as usize + usize::from(roles.alpha != 0) {
            let is_alpha = selected == roles.count as usize;
            let index = if is_alpha {
                roles.alpha
            } else {
                roles.data[selected]
            } as usize;
            let fact = f.bands[index - 1];
            read(cog, index, w, fact.datatype, samples.as_mut_ptr().cast())?;
            if boolean_mask(fact.mask_class) {
                read_mask(cog, index, w, &mut mask)?;
                for i in 0..n {
                    mark_mask(&mut flags[i], mask[i]);
                }
            }
            for i in 0..n {
                let v = value(&samples, i, fact.datatype);
                mark_sample(&mut flags[i], v, fact.datatype, fact.nodata);
                if !v.is_finite() {
                    continue;
                }
                if is_alpha {
                    rgba[4 * i + 3] = alpha(v);
                } else if roles.palette {
                    let palette = f.palette(fact.palette);
                    let Some(color) = palette.get(v as usize) else {
                        flags[i] |= 4;
                        continue;
                    };
                    rgba[4 * i..4 * i + 3].copy_from_slice(&color[..3]);
                } else {
                    let c = if roles.image {
                        v as u8
                    } else {
                        channel(v, roles.low, roles.high)
                    };
                    if roles.count == 1 {
                        rgba[4 * i..4 * i + 3].fill(c);
                    } else {
                        rgba[4 * i + selected] = c;
                    }
                }
            }
        }
        for i in 0..n {
            finish_pixel(flags[i], &mut rgba[4 * i..4 * i + 4])?;
        }
        for channel in 0..4 {
            let code = unsafe {
                gdal_sys::GDALRasterIO(
                    out.band(channel + 1),
                    1,
                    w.x as i32,
                    w.y as i32,
                    w.width as i32,
                    w.height as i32,
                    rgba.as_mut_ptr().add(channel).cast(),
                    w.width as i32,
                    w.height as i32,
                    1,
                    4,
                    (w.width * 4) as i32,
                )
            };
            if code != 0 {
                return Err(native::error("write RGBA display window"));
            }
        }
        Ok(())
    });
    let close = out.close();
    match (result, close) {
        (Err(e), c) => {
            let mut f = native::failure(e);
            native::secondary(&mut f, c);
            Err(f)
        }
        (Ok(()), c) => c.map_err(native::failure),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boolean_and_float_validity() {
        assert!(nodata(1. + f32::EPSILON as f64, 1., SampleType::F32));
        assert!(nodata(f64::NAN, f64::NAN, SampleType::F64));
        assert!(!nodata(2., 1., SampleType::U16));
        assert_eq!(channel(-f64::MAX, 0., 1.), 0);
        assert_eq!(channel(f64::MAX, 0., 1.), 255);
        assert_eq!(channel(0.5, 0., 1.), 128);
        assert_eq!(alpha(0.49), 0);
        assert_eq!(alpha(0.5), 1);
    }
    #[test]
    fn literal_validity_and_transparency() {
        let mut f = 1;
        mark_mask(&mut f, 1);
        assert_eq!(f, 1);
        mark_mask(&mut f, 0);
        assert_eq!(f, 0);
        assert!(!boolean_mask(6));
        let mut flags = 1;
        mark_sample(&mut flags, 1., SampleType::F32, ScalarFact::new(1., true));
        assert_eq!(flags, 0);
        let mut p = [12, 34, 56, 128];
        finish_pixel(flags, &mut p).unwrap();
        assert_eq!(p, [0; 4]);
        let mut p = [12, 34, 56, 0];
        assert!(finish_pixel(3, &mut p).is_err());
        finish_pixel(2, &mut p).unwrap();
        assert_eq!(p, [0; 4]);
        let mut p = [12, 34, 56, 0];
        finish_pixel(1, &mut p).unwrap();
        assert_eq!(p, [0; 4]);
    }
    #[test]
    fn actual_window_words() {
        assert_eq!(words(1, 1), 1);
        assert_eq!(words(257, 2), 65);
        assert_eq!(words(65536, 8), 65536);
    }
}
