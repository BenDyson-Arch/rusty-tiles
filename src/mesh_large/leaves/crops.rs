//! Checked source-aligned, translation-only crops. No class/value conversion.
use crate::{JobError, JobErrorKind};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceSampler {
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub min_filter: u32,
    pub mag_filter: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Crop {
    /// Interior source texel origin; allocation includes gutter before this.
    pub origin: [i64; 2],
    pub width: u32,
    pub height: u32,
    pub gutter: u32,
    pub interior_width: u32,
    pub interior_height: u32,
}
fn error(kind: JobErrorKind, message: &str) -> JobError {
    JobError::new(kind, message.to_owned())
}
fn invalid(message: &str) -> JobError {
    error(JobErrorKind::InvalidRequest, message)
}
fn resource() -> JobError {
    error(
        JobErrorKind::ResourceLimit,
        "crop arithmetic/allocation budget",
    )
}
fn unsupported() -> JobError {
    error(
        JobErrorKind::Unsupported,
        "source UV crop exceeds physical representation",
    )
}

pub(crate) fn resolve(value: Option<&Value>) -> Result<SourceSampler, JobError> {
    if value.is_some_and(|v| !v.is_object()) {
        return Err(invalid("sampler must be an object"));
    }
    let field = |name: &str, default: u32, allowed: &[u32]| -> Result<u32, JobError> {
        let n = match value.and_then(|v| v.get(name)) {
            None => default,
            Some(v) => v
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| invalid("invalid sampler enumerant"))?,
        };
        if !allowed.contains(&n) {
            return Err(invalid("unsupported sampler enumerant"));
        }
        Ok(n)
    };
    Ok(SourceSampler {
        wrap_s: field("wrapS", 10497, &[10497, 33648, 33071])?,
        wrap_t: field("wrapT", 10497, &[10497, 33648, 33071])?,
        min_filter: field("minFilter", 9729, &[9728, 9729, 9984, 9985, 9986, 9987])?,
        mag_filter: field("magFilter", 9729, &[9728, 9729])?,
    })
}
pub(crate) fn plan(
    uvs: &[[f32; 2]],
    image_width: u32,
    image_height: u32,
    max_edge: u32,
    gutter: u32,
) -> Result<Crop, JobError> {
    if uvs.is_empty() || image_width == 0 || image_height == 0 || max_edge == 0 {
        return Err(invalid("empty crop input or dimensions"));
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for uv in uvs {
        for axis in 0..2 {
            let v = f64::from(uv[axis]);
            if !v.is_finite() {
                return Err(invalid("nonfinite source UV"));
            }
            if v.abs() > 1e6 {
                return Err(unsupported());
            }
            lo[axis] = lo[axis].min(v);
            hi[axis] = hi[axis].max(v);
        }
    }
    let mut origin = [0; 2];
    let mut interior = [0; 2];
    let mut extent = [0; 2];
    for axis in 0..2 {
        let size = f64::from(if axis == 0 { image_width } else { image_height });
        // UV admission and u32 dimensions bound each product below 2^52.
        // Real source signal halo preserves both half-texel LINEAR taps.
        // Outer gutters remain separate zero-support dilation space.
        origin[axis] = ((lo[axis] * size).floor() as i64)
            .checked_sub(1)
            .ok_or_else(resource)?;
        let end = ((hi[axis] * size).ceil() as i64)
            .checked_add(1)
            .ok_or_else(resource)?;
        let span = end.checked_sub(origin[axis]).ok_or_else(resource)?.max(1);
        interior[axis] = u32::try_from(span).map_err(|_| unsupported())?;
        extent[axis] = interior[axis]
            .checked_add(gutter.checked_mul(2).ok_or_else(resource)?)
            .ok_or_else(resource)?;
        if extent[axis] > max_edge {
            return Err(unsupported());
        }
        origin[axis]
            .checked_sub(i64::from(gutter))
            .ok_or_else(resource)?;
        origin[axis]
            .checked_add(i64::from(interior[axis]))
            .and_then(|v| v.checked_add(i64::from(gutter)))
            .ok_or_else(resource)?;
    }
    Ok(Crop {
        origin,
        width: extent[0],
        height: extent[1],
        gutter,
        interior_width: interior[0],
        interior_height: interior[1],
    })
}
/// Dedicated mipmapped binding page. Both physical edges carry at least one
/// complete source-axis period of padding, and the page origin is aligned to
/// the next-POT source axis. For POT sources this fixes every mip's grid phase.
/// NPOT source/page correspondence still requires the finite sampler gate.
pub(crate) fn plan_mip_aligned(
    uvs: &[[f32; 2]],
    image_width: u32,
    image_height: u32,
    max_edge: u32,
) -> Result<Crop, JobError> {
    if uvs.is_empty() || image_width == 0 || image_height == 0 || max_edge < 4 {
        return Err(invalid("empty mip crop or invalid dimensions"));
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for uv in uvs {
        for axis in 0..2 {
            let v = f64::from(uv[axis]);
            if !v.is_finite() {
                return Err(invalid("nonfinite mip crop UV"));
            }
            if v.abs() > 1e6 {
                return Err(unsupported());
            }
            lo[axis] = lo[axis].min(v);
            hi[axis] = hi[axis].max(v);
        }
    }
    let mut origin = [0i64; 2];
    let mut extent = [0u32; 2];
    for (axis, size) in [image_width, image_height].into_iter().enumerate() {
        let alignment = size.checked_next_power_of_two().ok_or_else(resource)?;
        let margin = i64::from(size);
        let first = (lo[axis] * f64::from(size)).floor() as i64;
        let last = (hi[axis] * f64::from(size)).ceil() as i64;
        let padded_first = first.checked_sub(margin).ok_or_else(resource)?;
        origin[axis] = padded_first
            .div_euclid(i64::from(alignment))
            .checked_mul(i64::from(alignment))
            .ok_or_else(resource)?;
        let padded_end = last.checked_add(margin).ok_or_else(resource)?;
        let span = padded_end
            .checked_sub(origin[axis])
            .ok_or_else(resource)?
            .max(4);
        extent[axis] = u32::try_from(span)
            .map_err(|_| unsupported())?
            .checked_next_power_of_two()
            .ok_or_else(resource)?;
        if extent[axis] > max_edge {
            return Err(unsupported());
        }
    }
    Ok(Crop {
        origin,
        width: extent[0],
        height: extent[1],
        gutter: 0,
        interior_width: extent[0],
        interior_height: extent[1],
    })
}

fn validate_crop(c: &Crop) -> Result<(), JobError> {
    let g = c.gutter.checked_mul(2).ok_or_else(resource)?;
    if c.interior_width == 0
        || c.interior_height == 0
        || c.interior_width.checked_add(g) != Some(c.width)
        || c.interior_height.checked_add(g) != Some(c.height)
    {
        return Err(invalid("inconsistent crop"));
    }
    for (axis, size) in [c.width, c.height].into_iter().enumerate() {
        c.origin[axis]
            .checked_sub(i64::from(c.gutter))
            .and_then(|v| v.checked_add(i64::from(size)))
            .ok_or_else(resource)?;
    }
    Ok(())
}
fn tap(i: i64, n: u32, wrap: u32) -> Result<u32, JobError> {
    let n = i64::from(n);
    if n == 0 {
        return Err(invalid("zero source size"));
    }
    let v = match wrap {
        10497 => i.rem_euclid(n),
        33648 => {
            let p = i.rem_euclid(2 * n);
            if p < n {
                p
            } else {
                2 * n - 1 - p
            }
        }
        33071 => i.clamp(0, n - 1),
        _ => return Err(invalid("invalid wrap")),
    };
    Ok(v as u32)
}
pub(crate) fn copy_crop(
    source_rgba: &[u8],
    source_width: u32,
    source_height: u32,
    crop: &Crop,
    sampler: SourceSampler,
    memory_limit: u64,
) -> Result<Vec<u8>, JobError> {
    validate_crop(crop)?;
    if source_width == 0 || source_height == 0 {
        return Err(invalid("zero source size"));
    }
    let bytes = |w: u32, h: u32| {
        u64::from(w)
            .checked_mul(u64::from(h))
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(resource)
    };
    let source_bytes = bytes(source_width, source_height)?;
    if u64::try_from(source_rgba.len()).map_err(|_| resource())? != source_bytes {
        return Err(invalid("source RGBA shape"));
    }
    let count = bytes(crop.width, crop.height)?;
    if count > memory_limit {
        return Err(resource());
    }
    // Validate both wrap modes before allocating, including zero-use axes.
    tap(0, source_width, sampler.wrap_s)?;
    tap(0, source_height, sampler.wrap_t)?;
    let count = usize::try_from(count).map_err(|_| resource())?;
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| resource())?;
    out.resize(count, 0);
    for y in 0..crop.height {
        let sy = tap(
            crop.origin[1] + i64::from(y) - i64::from(crop.gutter),
            source_height,
            sampler.wrap_t,
        )?;
        for x in 0..crop.width {
            let sx = tap(
                crop.origin[0] + i64::from(x) - i64::from(crop.gutter),
                source_width,
                sampler.wrap_s,
            )?;
            let src =
                usize::try_from((u64::from(sy) * u64::from(source_width) + u64::from(sx)) * 4)
                    .map_err(|_| resource())?;
            let dst = usize::try_from((u64::from(y) * u64::from(crop.width) + u64::from(x)) * 4)
                .map_err(|_| resource())?;
            out[dst..dst + 4].copy_from_slice(&source_rgba[src..src + 4]);
        }
    }
    Ok(out)
}
pub(crate) fn remap_uv(
    uv: [f32; 2],
    source_width: u32,
    source_height: u32,
    crop: &Crop,
    page_offset: [u32; 2],
    page_dimensions: [u32; 2],
) -> Result<[f32; 2], JobError> {
    validate_crop(crop)?;
    if source_width == 0 || source_height == 0 {
        return Err(invalid("zero source size"));
    }
    let mut out = [0.; 2];
    for axis in 0..2 {
        let size = if axis == 0 { crop.width } else { crop.height };
        if page_dimensions[axis] == 0
            || page_offset[axis]
                .checked_add(size)
                .is_none_or(|v| v > page_dimensions[axis])
        {
            return Err(invalid("crop outside page"));
        }
        if !uv[axis].is_finite() {
            return Err(invalid("nonfinite UV"));
        }
        if uv[axis].abs() > 1e6 {
            return Err(unsupported());
        }
        let source = if axis == 0 {
            source_width
        } else {
            source_height
        };
        let v = (f64::from(uv[axis]) * f64::from(source) - crop.origin[axis] as f64
            + f64::from(crop.gutter)
            + f64::from(page_offset[axis]))
            / f64::from(page_dimensions[axis]);
        let stored = v as f32;
        if !stored.is_finite() {
            return Err(resource());
        }
        out[axis] = stored;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Independent normalized-coordinate source reference: wrap before selecting
    // taps, rather than using production integer wrapping.
    fn reference(u: f64, pixels: &[f64], wrap: u32, linear: bool) -> f64 {
        let coordinate = match wrap {
            10497 => u - u.floor(),
            33648 => {
                let cycle = u.rem_euclid(2.);
                if cycle <= 1. {
                    cycle
                } else {
                    2. - cycle
                }
            }
            33071 => u.clamp(0., 1.),
            _ => panic!(),
        };
        let fetch = |i: i64| {
            let n = pixels.len() as i64;
            let k = match wrap {
                10497 => ((i % n) + n) % n,
                _ => i.clamp(0, n - 1),
            };
            pixels[k as usize]
        };
        if !linear {
            // Reflected half-open cells reverse the exact-boundary tie.
            let x = coordinate * pixels.len() as f64;
            let i = if wrap == 33648 && u.rem_euclid(2.) > 1. {
                x.ceil() as i64 - 1
            } else {
                x.floor() as i64
            };
            return fetch(i);
        }
        let x = coordinate * pixels.len() as f64 - 0.5;
        let i = x.floor() as i64;
        let t = x - i as f64;
        fetch(i) * (1. - t) + fetch(i + 1) * t
    }
    #[test]
    fn sampler_defaults_and_literal_filters() {
        let s = resolve(None).unwrap();
        assert_eq!((s.min_filter, s.mag_filter, s.wrap_s), (9729, 9729, 10497));
        let s = resolve(Some(
            &serde_json::json!({"minFilter":9987,"magFilter":9728,"wrapS":33648}),
        ))
        .unwrap();
        assert_eq!((s.min_filter, s.mag_filter, s.wrap_s), (9987, 9728, 33648));
        assert!(resolve(Some(&serde_json::json!({"minFilter":true}))).is_err());
    }
    #[test]
    fn literal_copy_and_independent_filter_correspondence() {
        let pixels = [7., 61., 149., 233.];
        let source: Vec<u8> = pixels.iter().flat_map(|p| [*p as u8, 19, 203, 0]).collect();
        let crop = plan(&[[-1., 0.], [2., 1.]], 4, 1, 32, 2).unwrap();
        for wrap in [10497, 33648, 33071] {
            let s = SourceSampler {
                wrap_s: wrap,
                ..resolve(None).unwrap()
            };
            let out = copy_crop(&source, 4, 1, &crop, s, 4096).unwrap();
            for u in [
                -1., -0.875, -0.5, -0.125, 0., 0.125, 0.25, 0.5, 0.875, 1., 1.125, 1.5, 2.,
            ] {
                let mapped = remap_uv([u as f32, 0.5], 4, 1, &crop, [3, 2], [32, 16]).unwrap();
                let x = f64::from(mapped[0]) * 32. - 3. - 0.5;
                for linear in [false, true] {
                    let actual = if linear {
                        let i = x.floor() as usize;
                        let t = x - i as f64;
                        f64::from(out[i * 4]) * (1. - t) + f64::from(out[(i + 1) * 4]) * t
                    } else {
                        f64::from(out[((x + 0.5).floor() as usize) * 4])
                    };
                    assert!(
                        (actual - reference(u, &pixels, wrap, linear)).abs() < 1e-6,
                        "wrap {wrap} u {u} linear {linear}"
                    );
                }
            }
            // Every copied texel keeps otherwise visually unused RGBA channels.
            for rgba in out.as_chunks::<4>().0 {
                assert_eq!(&rgba[1..], &[19, 203, 0]);
            }
        }
    }
    #[test]
    fn zero_uv_and_checked_physical_refusals() {
        let crop = plan(&[[0., 0.]; 3], 4, 4, 8, 2).unwrap();
        assert_eq!((crop.interior_width, crop.interior_height), (2, 2));
        for uv in [-1e6, 1e6] {
            // Domain endpoint remains admitted; only physical span is refused.
            assert!(plan(&[[uv, uv]; 3], 4, 4, 8, 2).is_ok());
            assert_eq!(
                plan(&[[0., 0.], [uv, uv]], 4, 4, 8, 2).unwrap_err().kind(),
                JobErrorKind::Unsupported
            );
        }
        assert_eq!(
            plan(&[[f32::NAN, 0.]], 4, 4, 8, 2).unwrap_err().kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(
            copy_crop(&[0; 64], 4, 4, &crop, resolve(None).unwrap(), 99)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
        assert!(remap_uv([0., 0.], 4, 4, &crop, [4, 4], [8, 8]).is_err());
    }
    #[test]
    fn source_tap_halo_survives_sparse_gutter_fill() {
        use super::super::{mips::Class, sparse_mips};
        for class in [Class::Colour, Class::Data, Class::Normal] {
            let values = if class == Class::Normal {
                [128., 128., 128., 128.]
            } else {
                [7., 61., 149., 233.]
            };
            let source: Vec<u8> = values
                .iter()
                .flat_map(|p| {
                    if class == Class::Normal {
                        [*p as u8, 128, 255, 255]
                    } else {
                        [*p as u8, 19, 203, 255]
                    }
                })
                .collect();
            for wrap in [10497, 33648, 33071] {
                for zero_area in [false, true] {
                    let uvs = if zero_area {
                        vec![[0., 0.]; 3]
                    } else {
                        vec![[-0.5, 0.], [1., 1.]]
                    };
                    let crop = plan(&uvs, 4, 1, 32, 2).unwrap();
                    let copied = copy_crop(
                        &source,
                        4,
                        1,
                        &crop,
                        SourceSampler {
                            wrap_s: wrap,
                            ..resolve(None).unwrap()
                        },
                        4096,
                    )
                    .unwrap();
                    let mut pixels = vec![0; 32 * 16 * 4];
                    let mut support = vec![0; 32 * 16];
                    for y in 0..crop.height {
                        for x in 0..crop.width {
                            let destination = ((y + 2) * 32 + x + 3) as usize;
                            let src = (y * crop.width + x) as usize;
                            pixels[destination * 4..destination * 4 + 4]
                                .copy_from_slice(&copied[src * 4..src * 4 + 4]);
                            if x >= crop.gutter
                                && x < crop.gutter + crop.interior_width
                                && y >= crop.gutter
                                && y < crop.gutter + crop.interior_height
                            {
                                support[destination] = 1;
                            }
                        }
                    }
                    let chain =
                        sparse_mips::build(32, 16, &pixels, &support, class, 1 << 20).unwrap();
                    let points = if zero_area {
                        vec![0.]
                    } else {
                        vec![-0.5, -0.125, 0., 0.125, 0.875, 1.]
                    };
                    for u in points {
                        let uv = remap_uv([u as f32, 0.], 4, 1, &crop, [3, 2], [32, 16]).unwrap();
                        let x = f64::from(uv[0]) * 32. - 0.5;
                        let y = f64::from(uv[1]) * 16. - 0.5;
                        let (ix, iy) = (x.floor() as usize, y.floor() as usize);
                        let (fx, fy) = (x - x.floor(), y - y.floor());
                        let tap =
                            |x: usize, y: usize| f64::from(chain.levels[0].rgba[(y * 32 + x) * 4]);
                        let actual = (tap(ix, iy) * (1. - fx) + tap(ix + 1, iy) * fx) * (1. - fy)
                            + (tap(ix, iy + 1) * (1. - fx) + tap(ix + 1, iy + 1) * fx) * fy;
                        assert!(
                            (actual - reference(u, &values, wrap, true)).abs() < 1e-6,
                            "{class:?} wrap{wrap} u{u}"
                        );
                        let nearest = tap((x + 0.5).floor() as usize, (y + 0.5).floor() as usize);
                        assert!(
                            (nearest - reference(u, &values, wrap, false)).abs() < 1e-6,
                            "nearest {class:?} wrap{wrap} u{u}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn mip_aligned_page_both_edge_period_margins_and_physical_limits() {
        for (width, height) in [(4, 1), (3, 5), (8, 4)] {
            let uvs = [[-0.25, 0.0], [1.0, 1.0]];
            let crop = plan_mip_aligned(&uvs, width, height, 64).unwrap();
            assert_eq!(crop.gutter, 0);
            assert_eq!(crop.width, crop.interior_width);
            assert_eq!(crop.height, crop.interior_height);
            for (axis, size) in [width, height].into_iter().enumerate() {
                assert_eq!(
                    crop.origin[axis].rem_euclid(i64::from(size.next_power_of_two())),
                    0
                );
                let physical_end =
                    crop.origin[axis] + i64::from(if axis == 0 { crop.width } else { crop.height });
                let lo = f64::from(uvs[0][axis]) * f64::from(size);
                let hi = f64::from(uvs[1][axis]) * f64::from(size);
                assert!(lo - crop.origin[axis] as f64 >= f64::from(size));
                assert!(physical_end as f64 - hi >= f64::from(size));
            }
        }
        assert_eq!(
            plan_mip_aligned(&[[0.0, 0.0], [1.0, 1.0]], 4096, 4096, 4096)
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            plan_mip_aligned(&[[f32::NAN, 0.0]], 4, 4, 64)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
    }
}
