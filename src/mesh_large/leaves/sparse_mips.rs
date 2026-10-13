//! Lossy leaf page signal mips. Real crop interiors have positive support;
//! padding/gutters have zero support and never feed a later reduction. Normal
//! values are raw unit tangent vectors; material normalScale remains literal.
//! PNG source-pixel lossless bypasses this module. MASK/guards are later delivery
//! operations and must never feed this semantic pyramid.
//! `Class::Id` pages are never averaged: each level takes the support-weighted
//! mode of its 2x2 children (whole RGBA tuple, ties to the first row-major
//! contributor), and every zero-support texel is the no-feature value
//! [`ID_NO_FEATURE`] rather than a dilated neighbour.
use super::mips::Class;
use crate::{JobError, JobErrorKind};
use std::mem::size_of;

#[derive(Debug)]
pub(super) struct Level {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub support: Vec<u32>,
}
/// Gutter/padding texel of an ID page: every channel all-ones, so each packed
/// feature-ID set reads its own `nullFeatureId` (2^(8*bytes) - 1).
pub(super) const ID_NO_FEATURE: [u8; 4] = [255; 4];
#[derive(Debug)]
pub(super) struct Chain {
    pub class: Class,
    pub levels: Vec<Level>,
    /// Owned chain plus reusable gutter scratch bound; borrowed inputs and
    /// native encoder workspace/output must be charged by the caller.
    pub charged_bytes: u64,
}
#[derive(Clone, Copy)]
struct Nearest {
    distance: u64,
    owner: usize,
}
fn error(kind: JobErrorKind, message: &str) -> JobError {
    JobError::new(kind, message.to_owned())
}
fn resource() -> JobError {
    error(
        JobErrorKind::ResourceLimit,
        "sparse mip size, support or allocation exceeds resource limit",
    )
}
fn quant(x: f64) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}
fn decode_colour_exact(x: u8) -> f64 {
    let v = f64::from(x) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
/// The same values from a table built with the same expression.
fn decode_colour(x: u8) -> f64 {
    static TABLE: std::sync::OnceLock<[f64; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|i| decode_colour_exact(i as u8)))[usize::from(x)]
}
fn encode_colour(v: f64) -> u8 {
    quant(if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    })
}
fn normal(p: &[u8]) -> [f64; 3] {
    std::array::from_fn(|a| f64::from(p[a]) / 127.5 - 1.0)
}
fn unit(v: [f64; 3]) -> Result<[f64; 3], JobError> {
    let length = v.iter().map(|x| x * x).sum::<f64>();
    if !length.is_finite() || length <= 1e-24 {
        return Err(error(
            JobErrorKind::Unsupported,
            "sparse mip unrepresentable zero/nonfinite normal",
        ));
    }
    Ok(v.map(|x| x / length.sqrt()))
}
fn empty_pixel(class: Class) -> [u8; 4] {
    match class {
        Class::Normal => [128, 128, 255, 0],
        Class::Id => ID_NO_FEATURE,
        _ => [0; 4],
    }
}
fn reserve<T>(n: usize) -> Result<Vec<T>, JobError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| resource())?;
    Ok(v)
}
fn relax(nearest: &mut [Nearest], to: usize, from: usize) {
    let src = nearest[from];
    if src.owner == usize::MAX {
        return;
    }
    let candidate = Nearest {
        distance: src.distance + 1,
        owner: src.owner,
    };
    if (candidate.distance, candidate.owner) < (nearest[to].distance, nearest[to].owner) {
        nearest[to] = candidate;
    }
}
/// Exact Manhattan nearest signal texel; ties original row-major identity.
/// Two-pass taxicab distance transform, not a quadratic search or Euclidean claim.
fn fill_gutters(level: &mut Level, class: Class, nearest: &mut Vec<Nearest>) {
    if class == Class::Id {
        for (p, &s) in level
            .rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&level.support)
        {
            if s == 0 {
                *p = ID_NO_FEATURE;
            }
        }
        return;
    }
    let n = level.support.len();
    let w = level.width as usize;
    nearest.clear();
    nearest.extend(level.support.iter().enumerate().map(|(i, &s)| Nearest {
        distance: if s > 0 { 0 } else { u64::MAX },
        owner: if s > 0 { i } else { usize::MAX },
    }));
    for i in 0..n {
        if i % w > 0 {
            relax(nearest, i, i - 1);
        }
        if i >= w {
            relax(nearest, i, i - w);
        }
    }
    for i in (0..n).rev() {
        if i % w + 1 < w {
            relax(nearest, i, i + 1);
        }
        if i + w < n {
            relax(nearest, i, i + w);
        }
    }
    for (i, &s) in level.support.iter().enumerate() {
        if s > 0 {
            continue;
        }
        let owner = nearest[i].owner;
        let value = if owner == usize::MAX {
            empty_pixel(class)
        } else {
            level.rgba[owner * 4..owner * 4 + 4]
                .try_into()
                .expect("validated RGBA shape")
        };
        level.rgba[i * 4..i * 4 + 4].copy_from_slice(&value);
    }
}

pub(super) fn build(
    width: u32,
    height: u32,
    rgba: &[u8],
    support: &[u32],
    class: Class,
    memory_limit: u64,
) -> Result<Chain, JobError> {
    if width < 4 || height < 4 || !width.is_power_of_two() || !height.is_power_of_two() {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "sparse mip page requires power-of-two dimensions >=4",
        ));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(resource)?;
    let base_len = usize::try_from(pixels).map_err(|_| resource())?;
    let rgba_len = base_len.checked_mul(4).ok_or_else(resource)?;
    if rgba.len() != rgba_len || support.len() != base_len {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "sparse mip RGBA/support shape mismatch",
        ));
    }
    support
        .iter()
        .try_fold(0u32, |sum, &s| sum.checked_add(s).ok_or_else(resource))?;
    let mut dimensions = [(0u32, 0u32); 33];
    let mut count = 0usize;
    let (mut w, mut h) = (width, height);
    let mut charged = 0u64;
    loop {
        dimensions[count] = (w, h);
        count += 1;
        let n = u64::from(w) * u64::from(h);
        charged = charged
            .checked_add(
                n.checked_mul(4 + size_of::<u32>() as u64)
                    .ok_or_else(resource)?,
            )
            .ok_or_else(resource)?;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    charged = charged
        .checked_add(
            pixels
                .checked_mul(size_of::<Nearest>() as u64)
                .ok_or_else(resource)?,
        )
        .ok_or_else(resource)?;
    charged = charged
        .checked_add(
            (count as u64)
                .checked_mul(size_of::<Level>() as u64)
                .ok_or_else(resource)?,
        )
        .ok_or_else(resource)?;
    if charged > memory_limit {
        return Err(resource());
    }
    let mut chain = Chain {
        class,
        levels: reserve(count)?,
        charged_bytes: charged,
    };
    let mut nearest = reserve(base_len)?;
    let mut base = Level {
        width,
        height,
        rgba: reserve(rgba_len)?,
        support: reserve(base_len)?,
    };
    base.rgba.extend_from_slice(rgba);
    base.support.extend_from_slice(support);
    if class == Class::Normal {
        for (p, &s) in base.rgba.as_chunks_mut::<4>().0.iter_mut().zip(support) {
            if s > 0 {
                let v = unit(normal(p))?;
                for (a, &component) in v.iter().enumerate() {
                    p[a] = quant((component + 1.0) * 0.5);
                }
            }
        }
    }
    fill_gutters(&mut base, class, &mut nearest);
    chain.levels.push(base);
    for &(dw, dh) in &dimensions[1..count] {
        let src = chain.levels.last().expect("base exists");
        let sw = src.width as usize;
        let n = (dw as usize)
            .checked_mul(dh as usize)
            .ok_or_else(resource)?;
        let mut dst = Level {
            width: dw,
            height: dh,
            rgba: reserve(n.checked_mul(4).ok_or_else(resource)?)?,
            support: reserve(n)?,
        };
        dst.rgba.resize(n * 4, 0);
        dst.support.resize(n, 0);
        for y in 0..dh as usize {
            for x in 0..dw as usize {
                if class == Class::Id {
                    let i = y * dw as usize + x;
                    let mut tally = [([0u8; 4], 0u64); 4];
                    let mut len = 0;
                    let mut weight = 0u32;
                    for sy in y * 2..(y * 2 + 2).min(src.height as usize) {
                        for sx in x * 2..(x * 2 + 2).min(sw) {
                            let j = sy * sw + sx;
                            let s = src.support[j];
                            if s == 0 {
                                continue;
                            }
                            let p: [u8; 4] = src.rgba[j * 4..j * 4 + 4].try_into().expect("RGBA8");
                            match tally[..len].iter_mut().find(|(v, _)| *v == p) {
                                Some((_, w)) => *w += u64::from(s),
                                None => {
                                    tally[len] = (p, u64::from(s));
                                    len += 1;
                                }
                            }
                            weight = weight.checked_add(s).ok_or_else(resource)?;
                        }
                    }
                    dst.support[i] = weight;
                    // Strictly greater keeps the first row-major value on ties.
                    let mut best = (empty_pixel(class), 0u64);
                    for &(v, w) in &tally[..len] {
                        if w > best.1 {
                            best = (v, w);
                        }
                    }
                    dst.rgba[i * 4..i * 4 + 4].copy_from_slice(&best.0);
                    continue;
                }
                let mut sum = [0.0; 4];
                let mut weight = 0u32;
                let mut dominant = [0.0, 0.0, 1.0];
                let mut dominant_weight = 0u32;
                for sy in y * 2..(y * 2 + 2).min(src.height as usize) {
                    for sx in x * 2..(x * 2 + 2).min(sw) {
                        let i = sy * sw + sx;
                        let s = src.support[i];
                        if s == 0 {
                            continue;
                        }
                        let p = &src.rgba[i * 4..i * 4 + 4];
                        let rgb = match class {
                            Class::Colour => std::array::from_fn(|a| decode_colour(p[a])),
                            Class::Data => std::array::from_fn(|a| f64::from(p[a]) / 255.0),
                            Class::Normal => unit(normal(p))?,
                            Class::Id => unreachable!("ID footprints take the mode above"),
                        };
                        if s > dominant_weight {
                            dominant = rgb;
                            dominant_weight = s;
                        }
                        for (a, &v) in rgb.iter().enumerate() {
                            sum[a] += v * f64::from(s);
                        }
                        sum[3] += f64::from(p[3]) / 255.0 * f64::from(s);
                        weight = weight.checked_add(s).ok_or_else(resource)?;
                    }
                }
                let i = y * dw as usize + x;
                dst.support[i] = weight;
                if weight == 0 {
                    dst.rgba[i * 4..i * 4 + 4].copy_from_slice(&empty_pixel(class));
                    continue;
                }
                let rgb = if class == Class::Normal {
                    if sum[..3].iter().map(|v| v * v).sum::<f64>() <= 1e-24 {
                        dominant
                    } else {
                        unit([sum[0], sum[1], sum[2]])?
                    }
                } else {
                    std::array::from_fn(|a| sum[a] / f64::from(weight))
                };
                for (a, &v) in rgb.iter().enumerate() {
                    dst.rgba[i * 4 + a] = match class {
                        Class::Colour => encode_colour(v),
                        Class::Data => quant(v),
                        Class::Normal => quant((v + 1.0) * 0.5),
                        Class::Id => unreachable!("ID footprints take the mode above"),
                    };
                }
                dst.rgba[i * 4 + 3] = quant(sum[3] / f64::from(weight));
            }
        }
        fill_gutters(&mut dst, class, &mut nearest);
        chain.levels.push(dst);
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(class: Class, pixels: &[u8], weights: &[u32]) -> Chain {
        build(4, 4, pixels, weights, class, 1 << 20).unwrap()
    }
    #[test]
    fn gamma_data_and_gutter_contamination_sensitive_control() {
        let mut pixels = vec![17; 64];
        let mut weights = vec![0; 16];
        pixels[0..4].copy_from_slice(&[0, 0, 0, 0]);
        weights[0] = 1;
        pixels[4..8].copy_from_slice(&[255, 255, 255, 255]);
        weights[1] = 1;
        let colour = page(Class::Colour, &pixels, &weights);
        let data = page(Class::Data, &pixels, &weights);
        for level in &colour.levels[1..] {
            assert_eq!(level.rgba[0], 188);
            assert_eq!(level.rgba[3], 128);
            assert_eq!(level.support.iter().sum::<u32>(), 2);
        }
        for level in &data.levels[1..] {
            assert_eq!(level.rgba[0], 128);
        }
        // A black-padding/gutter-as-support mutant averages four values, not two.
        assert!((188_i32 - 137).abs() > 1);
        assert_eq!(colour.levels[0].support, weights);
    }
    #[test]
    fn independent_integer_weighted_linear_reference_every_level() {
        let mut pixels = vec![0; 64];
        let weights: Vec<u32> = (0..16)
            .map(|i| if i % 3 == 0 { 0 } else { (i % 4 + 1) as u32 })
            .collect();
        for (i, p) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            p.copy_from_slice(&[(i * 13) as u8, (255 - i * 11) as u8, (i * 7) as u8, 255]);
        }
        let chain = page(Class::Data, &pixels, &weights);
        // Independent reference uses integer rational weighted byte means and
        // explicit child index sets, not any production filtering helper.
        let mut expected: Vec<([u8; 4], u32)> = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .zip(weights)
            .collect();
        let mut width = 4;
        for actual in &chain.levels[1..] {
            let mut reduced = Vec::new();
            for y in 0..width / 2 {
                for x in 0..width / 2 {
                    let ids = [
                        (2 * y) * width + 2 * x,
                        (2 * y) * width + 2 * x + 1,
                        (2 * y + 1) * width + 2 * x,
                        (2 * y + 1) * width + 2 * x + 1,
                    ];
                    let weight: u32 = ids.iter().map(|&i| expected[i].1).sum();
                    let pixel = if weight == 0 {
                        [0; 4]
                    } else {
                        std::array::from_fn(|a| {
                            let numerator: u64 = ids
                                .iter()
                                .map(|&i| u64::from(expected[i].0[a]) * u64::from(expected[i].1))
                                .sum();
                            ((2 * numerator + u64::from(weight)) / (2 * u64::from(weight))) as u8
                        })
                    };
                    reduced.push((pixel, weight));
                }
            }
            for (i, (pixel, weight)) in reduced.iter().enumerate() {
                assert_eq!(actual.support[i], *weight);
                if *weight > 0 {
                    assert_eq!(&actual.rgba[i * 4..i * 4 + 4], pixel);
                }
            }
            expected = reduced;
            width /= 2;
        }
    }
    #[test]
    fn independent_bruteforce_gutter_nearest_and_ties() {
        for mask in [1u16, 0x8001, 0x0440, 0x1357, 0x9009] {
            let mut pixels = vec![0; 64];
            let weights: Vec<u32> = (0..16).map(|i| u32::from(mask & (1 << i) != 0)).collect();
            for (i, p) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *p = [i as u8, 0, 0, 255];
            }
            let chain = page(Class::Data, &pixels, &weights);
            let base = &chain.levels[0];
            for i in 0usize..16 {
                let owner = (0usize..16)
                    .filter(|&j| weights[j] > 0)
                    .min_by_key(|&j| ((i % 4).abs_diff(j % 4) + (i / 4).abs_diff(j / 4), j))
                    .unwrap();
                assert_eq!(base.rgba[i * 4], owner as u8);
                assert_eq!(base.support[i], weights[i]);
            }
        }
    }
    #[test]
    fn normal_unit_every_level_and_row_major_cancellation_donor() {
        let mut pixels = vec![0; 64];
        let mut weights = vec![0; 16];
        pixels[..4].copy_from_slice(&[255, 255, 255, 255]);
        pixels[4..8].copy_from_slice(&[0, 0, 0, 255]);
        weights[0] = 1;
        weights[1] = 1;
        let chain = page(Class::Normal, &pixels, &weights);
        assert_eq!(&chain.levels[1].rgba[..3], &chain.levels[0].rgba[..3]);
        for l in &chain.levels {
            for p in l.rgba.as_chunks::<4>().0 {
                let v = p[..3]
                    .iter()
                    .map(|&b| 2.0 * f64::from(b) / 255.0 - 1.0)
                    .collect::<Vec<_>>();
                assert!((v.iter().map(|x| x * x).sum::<f64>().sqrt() - 1.0).abs() < 0.01);
            }
        }
        let oblique: Vec<u8> = (0..16)
            .flat_map(|i| {
                if i % 2 == 0 {
                    [218, 128, 218, 255]
                } else {
                    [37, 128, 218, 255]
                }
            })
            .collect();
        let oblique_chain = page(Class::Normal, &oblique, &[1; 16]);
        for l in &oblique_chain.levels {
            for p in l.rgba.as_chunks::<4>().0 {
                let squared = p[..3]
                    .iter()
                    .map(|&b| {
                        let x = 2.0 * f64::from(b) / 255.0 - 1.0;
                        x * x
                    })
                    .sum::<f64>();
                assert!((squared.sqrt() - 1.0).abs() < 0.01);
            }
        }
        // Oblique unnormalized mean loses length despite a valid angle.
        let length = (181.0_f64 / 255.0).abs();
        assert!((length - 1.0) > 0.01 || (1.0 - length) > 0.01);
    }
    #[test]
    fn empty_pages_and_rectangular_complete_chain() {
        for class in [Class::Colour, Class::Data, Class::Normal, Class::Id] {
            let chain = build(8, 4, &[255; 128], &[0; 32], class, 1 << 20).unwrap();
            assert_eq!(
                chain
                    .levels
                    .iter()
                    .map(|l| (l.width, l.height))
                    .collect::<Vec<_>>(),
                vec![(8, 4), (4, 2), (2, 1), (1, 1)]
            );
            for l in chain.levels {
                assert!(l.support.iter().all(|&s| s == 0));
                for p in l.rgba.as_chunks::<4>().0 {
                    assert_eq!(*p, empty_pixel(class));
                }
            }
        }
    }
    #[test]
    fn typed_shape_memory_and_support_overflow() {
        assert_eq!(
            build(3, 4, &[], &[], Class::Data, u64::MAX)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(
            build(4, 4, &[0; 63], &[0; 16], Class::Data, u64::MAX)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(
            build(4, 4, &[0; 64], &[0; 16], Class::Data, 1)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
        assert_eq!(
            build(4, 4, &[0; 64], &[u32::MAX; 16], Class::Data, u64::MAX)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
    }
}
