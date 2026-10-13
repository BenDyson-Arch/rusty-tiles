//! Caller-owned whole-page semantic mips. Separate calls are required for aliased
//! images used in different classes or MASK material contexts. Uniform page
//! support only: chart interiors/gutters require a future explicit support model.
//! Normal cancellation chooses the greatest-overlap unit contributor, ties by
//! stable source row-major order; authored zero/nonfinite normals are refused.
//! PNG cannot represent these authored levels; renderer mip correctness remains
//! a separate proof obligation, never an implicit production fallback.
use crate::{JobError, JobErrorKind};
use std::mem::size_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    Colour,
    Normal,
    Data,
}
#[derive(Clone, Copy, Debug)]
/// Historical uniform-image coverage reference for tests. Production MASK
/// coverage belongs to `mask.rs`, including geometry and original vertex alpha.
pub(crate) struct Mask {
    pub factor: f64,
    pub cutoff: f64,
}
#[derive(Debug)]
pub(crate) struct Level {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Debug)]
pub(crate) struct Coverage {
    pub target: f64,
    pub actual: f64,
    pub residual: f64,
    pub scale: f64,
    pub texels: usize,
    /// Attainable passing counts under a uniform nonnegative scale, after RGBA8 rounding.
    pub attainable_counts: Vec<usize>,
}
#[derive(Debug)]
pub(crate) struct Chain {
    pub class: Class,
    pub levels: Vec<Level>,
    pub coverage: Vec<Coverage>,
    pub normal_cancellations: Vec<usize>,
    /// Charged owning allocation bound (pixels, level/report records, count lists).
    /// Caller additionally owns encoder workspace/output and its job/global memory.
    pub charged_bytes: usize,
}
fn err(kind: JobErrorKind, s: &str) -> JobError {
    JobError::new(kind, s.to_owned())
}
fn overflow() -> JobError {
    err(
        JobErrorKind::ResourceLimit,
        "semantic mip size/memory overflow",
    )
}
fn bytes(w: u32, h: u32) -> Result<usize, JobError> {
    if w == 0 || h == 0 {
        return Err(err(
            JobErrorKind::InvalidRequest,
            "semantic mip dimensions must be nonzero",
        ));
    }
    (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(overflow)
}
fn byte(x: f64) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}
fn linear(x: u8) -> f64 {
    let x = f64::from(x) / 255.0;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(x: f64) -> u8 {
    byte(if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    })
}
fn normal(p: &[u8]) -> [f64; 3] {
    std::array::from_fn(|a| f64::from(p[a]) / 127.5 - 1.0)
}
fn unit(v: [f64; 3]) -> [f64; 3] {
    let n = v.iter().map(|x| x * x).sum::<f64>();
    debug_assert!(n.is_finite() && n > 1e-24);
    v.map(|x| x / n.sqrt())
}
fn scaled(a: u8, scale: f64) -> u8 {
    byte(f64::from(a) / 255.0 * scale)
}
fn passes(a: u8, m: Mask) -> bool {
    f64::from(a) / 255.0 * m.factor >= m.cutoff
}
fn coverage(level: &mut Level, m: Mask, target: f64) -> Result<Coverage, JobError> {
    let mut histogram = [0usize; 256];
    for p in level.rgba.as_chunks::<4>().0.iter() {
        histogram[p[3] as usize] += 1;
    }
    let n = level.rgba.len() / 4;
    let threshold = (0..=255u16).find(|a| passes(*a as u8, m));
    let mut candidates = [0.0_f64; 258];
    candidates[1] = 1.0;
    let mut len = 2;
    if let Some(t) = threshold {
        if t > 0 {
            for (a, &frequency) in histogram.iter().enumerate().skip(1) {
                if frequency != 0 {
                    // RGBA8 round-to-nearest changes pass state at t-0.5.
                    let crossing = (f64::from(t) - 0.5) / a as f64;
                    candidates[len] = f64::from_bits(crossing.to_bits() + 1);
                    len += 1;
                }
            }
        }
    }
    let count = |s| {
        histogram
            .iter()
            .enumerate()
            .filter(|(a, _)| passes(scaled(*a as u8, s), m))
            .map(|(_, c)| *c)
            .sum::<usize>()
    };
    let mut counts = Vec::new();
    counts.try_reserve_exact(258).map_err(|_| overflow())?;
    let mut best = (f64::INFINITY, f64::INFINITY, f64::INFINITY, 0usize);
    for &s in &candidates[..len] {
        let c = count(s);
        counts.push(c);
        let key = ((c as f64 / n as f64 - target).abs(), (s - 1.0).abs(), s, c);
        if key < best {
            best = key;
        }
    }
    counts.sort_unstable();
    counts.dedup();
    for p in level.rgba.as_chunks_mut::<4>().0 {
        p[3] = scaled(p[3], best.2);
    }
    let actual = count(best.2) as f64 / n as f64;
    Ok(Coverage {
        target,
        actual,
        residual: (actual - target).abs(),
        scale: best.2,
        texels: n,
        attainable_counts: counts,
    })
}

/// Consumes the base allocation; never invokes native encoder mip generation.
/// Budget covers owned chain allocations, not caller encoder/native workspace.
pub(crate) fn build(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    class: Class,
    mask: Option<Mask>,
    memory_limit: usize,
) -> Result<Chain, JobError> {
    if rgba.len() != bytes(width, height)? {
        return Err(err(
            JobErrorKind::InvalidRequest,
            "semantic mip RGBA8 shape mismatch",
        ));
    }
    if let Some(m) = mask {
        if class != Class::Colour
            || !m.factor.is_finite()
            || !m.cutoff.is_finite()
            || !(0.0..=1.0).contains(&m.factor)
            || !(0.0..=1.0).contains(&m.cutoff)
        {
            return Err(err(
                JobErrorKind::InvalidRequest,
                "semantic mip invalid colour MASK context",
            ));
        }
    }
    let mut dims = [(0u32, 0u32); 33];
    let mut count = 0;
    let (mut w, mut h) = (width, height);
    let mut charged = rgba.capacity();
    loop {
        dims[count] = (w, h);
        count += 1;
        if count > 1 {
            charged = charged.checked_add(bytes(w, h)?).ok_or_else(overflow)?;
        }
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    let records = size_of::<Level>()
        + size_of::<usize>()
        + if mask.is_some() {
            size_of::<Coverage>() + 258 * size_of::<usize>()
        } else {
            0
        };
    charged = charged
        .checked_add(count.checked_mul(records).ok_or_else(overflow)?)
        .ok_or_else(overflow)?;
    if charged > memory_limit {
        return Err(err(
            JobErrorKind::ResourceLimit,
            "semantic mip owning memory exceeds budget",
        ));
    }
    let mut chain = Chain {
        class,
        levels: Vec::new(),
        coverage: Vec::new(),
        normal_cancellations: Vec::new(),
        charged_bytes: charged,
    };
    chain
        .levels
        .try_reserve_exact(count)
        .map_err(|_| overflow())?;
    chain
        .normal_cancellations
        .try_reserve_exact(count)
        .map_err(|_| overflow())?;
    if mask.is_some() {
        chain
            .coverage
            .try_reserve_exact(count)
            .map_err(|_| overflow())?;
    }
    let target = mask.map(|m| {
        rgba.as_chunks::<4>()
            .0
            .iter()
            .filter(|p| passes(p[3], m))
            .count() as f64
            / (rgba.len() / 4) as f64
    });
    let mut base = Level {
        width,
        height,
        rgba,
    };
    let cancellations = 0;
    if class == Class::Normal {
        for p in base.rgba.as_chunks_mut::<4>().0 {
            let authored = normal(p);
            if authored.iter().any(|v| !v.is_finite())
                || authored.iter().map(|v| v * v).sum::<f64>() <= 1e-24
            {
                return Err(err(
                    JobErrorKind::Unsupported,
                    "semantic mip zero/nonfinite authored normal",
                ));
            }
            let v = unit(authored);
            for a in 0..3 {
                p[a] = byte((v[a] + 1.0) * 0.5);
            }
        }
    }
    if let Some(m) = mask {
        chain
            .coverage
            .push(coverage(&mut base, m, target.unwrap())?);
    }
    chain.levels.push(base);
    chain.normal_cancellations.push(cancellations);
    for &(dw, dh) in &dims[1..count] {
        let src = chain.levels.last().unwrap();
        let (sw, sh) = (src.width as u64, src.height as u64);
        let mut out = Level {
            width: dw,
            height: dh,
            rgba: Vec::new(),
        };
        let size = bytes(dw, dh)?;
        out.rgba.try_reserve_exact(size).map_err(|_| overflow())?;
        out.rgba.resize(size, 0);
        let mut cancelled = 0;
        for y in 0..dh as u64 {
            for x in 0..dw as u64 {
                // Integer overlap intervals in units of 1/destination dimension.
                // Every edge source texel contributes, including odd and 1D pages.
                let (xl, xr, yl, yr) = (x * sw, (x + 1) * sw, y * sh, (y + 1) * sh);
                let mut sum = [0.0; 4];
                let mut total = 0.0;
                let mut dominant = [0.0, 0.0, 1.0];
                let mut dominant_weight = -1.0;
                for sy in yl / dh as u64..yr.div_ceil(dh as u64) {
                    for sx in xl / dw as u64..xr.div_ceil(dw as u64) {
                        let wx = xr.min((sx + 1) * dw as u64) - xl.max(sx * dw as u64);
                        let wy = yr.min((sy + 1) * dh as u64) - yl.max(sy * dh as u64);
                        let weight = wx as f64 * wy as f64;
                        let i = ((sy * sw + sx) * 4) as usize;
                        let p = &src.rgba[i..i + 4];
                        let rgb = match class {
                            Class::Colour => std::array::from_fn(|a| linear(p[a])),
                            Class::Normal => unit(normal(p)),
                            Class::Data => std::array::from_fn(|a| f64::from(p[a]) / 255.0),
                        };
                        if class == Class::Normal && weight > dominant_weight {
                            dominant = rgb;
                            dominant_weight = weight;
                        }
                        for a in 0..3 {
                            sum[a] += rgb[a] * weight;
                        }
                        sum[3] += f64::from(p[3]) / 255.0 * weight;
                        total += weight;
                    }
                }
                let i = ((y * dw as u64 + x) * 4) as usize;
                let rgb = if class == Class::Normal {
                    if sum[..3].iter().map(|v| v * v).sum::<f64>() <= 1e-24 {
                        cancelled += 1;
                        dominant
                    } else {
                        unit([sum[0], sum[1], sum[2]])
                    }
                } else {
                    [sum[0] / total, sum[1] / total, sum[2] / total]
                };
                for (a, &component) in rgb.iter().enumerate() {
                    out.rgba[i + a] = match class {
                        Class::Colour => srgb(component),
                        Class::Normal => byte((component + 1.0) * 0.5),
                        Class::Data => byte(component),
                    };
                }
                out.rgba[i + 3] = byte(sum[3] / total);
            }
        }
        if let Some(m) = mask {
            chain.coverage.push(coverage(&mut out, m, target.unwrap())?);
        }
        chain.levels.push(out);
        chain.normal_cancellations.push(cancelled);
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make(w: u32, h: u32, p: Vec<u8>, c: Class, m: Option<Mask>) -> Chain {
        build(w, h, p, c, m, 1 << 20).unwrap()
    }
    #[test]
    fn gamma_and_linear_data_are_distinct() {
        let p = vec![0, 0, 0, 255, 255, 255, 255, 255];
        assert_eq!(
            make(2, 1, p.clone(), Class::Colour, None).levels[1].rgba[0],
            188
        );
        assert_eq!(make(2, 1, p, Class::Data, None).levels[1].rgba[0], 128);
        // The wrong-gamma control (byte averaging) differs by sixty bytes.
        assert!((188_i32 - 128).abs() > 1);
    }
    #[test]
    fn odd_rectangular_edges_and_one_dimensional_chain() {
        let p = (0..15)
            .flat_map(|i| [if i == 14 { 255 } else { 0 }, 0, 0, 255])
            .collect();
        let c = make(5, 3, p, Class::Data, None);
        assert_eq!(
            c.levels
                .iter()
                .map(|l| (l.width, l.height))
                .collect::<Vec<_>>(),
            vec![(5, 3), (2, 1), (1, 1)]
        );
        // Last corner is 1/7.5 of the right footprint, then 1/2 terminal.
        assert_eq!(c.levels[1].rgba[4], 34);
        assert_eq!(c.levels[2].rgba[0], 17);
        assert_eq!(make(1, 7, vec![255; 28], Class::Data, None).levels.len(), 3);
    }
    #[test]
    fn oblique_normals_have_unit_raw_length_including_base() {
        let p = (0..64)
            .flat_map(|i| {
                if i % 2 == 0 {
                    [218, 128, 218, 255]
                } else {
                    [37, 128, 218, 255]
                }
            })
            .collect();
        let c = make(8, 8, p, Class::Normal, None);
        for l in &c.levels {
            for p in l.rgba.as_chunks::<4>().0.iter() {
                let n = normal(p).iter().map(|x| x * x).sum::<f64>().sqrt();
                assert!((n - 1.0).abs() < 0.01);
            }
        }
        // Averaging these raw oblique vectors without normalization loses length.
        let v = [0.0, 1.0 / 255.0, 181.0 / 255.0];
        assert!((v.iter().map(|x| x * x).sum::<f64>().sqrt() - 1.0).abs() > 0.2);
    }
    #[test]
    fn cancellation_uses_greatest_weight_row_major_unit_contributor() {
        let c = make(
            2,
            1,
            vec![255, 255, 255, 255, 0, 0, 0, 255],
            Class::Normal,
            None,
        );
        assert_eq!(c.normal_cancellations[1], 1);
        assert_eq!(&c.levels[1].rgba[..3], &c.levels[0].rgba[..3]);
    }
    #[test]
    fn mask_preserves_attainable_larger_level_coverage() {
        // Half the footprints have three surviving alpha texels; half only one.
        // Plain averaging makes all sixteen footprints fail cutoff 0.8.
        let mut p = vec![0; 8 * 8 * 4];
        for y in 0..8 {
            for x in 0..8 {
                let i = (y * 8 + x) * 4;
                p[i + 3] = if (x / 2 < 2 && (y % 2 == 0 || x % 2 == 0))
                    || (x / 2 >= 2 && y % 2 == 0 && x % 2 == 0)
                {
                    255
                } else {
                    0
                };
            }
        }
        let m = Mask {
            factor: 1.0,
            cutoff: 0.8,
        };
        let plain = make(8, 8, p.clone(), Class::Colour, None);
        let c = make(8, 8, p, Class::Colour, Some(m));
        let r = &c.coverage[1];
        let bad = plain.levels[1]
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| passes(p[3], m))
            .count() as f64
            / 16.0;
        assert!((bad - r.target).abs() > 0.2);
        assert_eq!(r.residual, 0.0);
        assert_eq!(r.actual, 0.5);
        assert!(r.attainable_counts.contains(&8));
        assert!(c.coverage.last().unwrap().residual > 0.0); // honest 1x1 discreteness
    }
    #[test]
    fn invalid_shape_overflow_and_memory_are_refused() {
        assert_eq!(
            build(0, 1, vec![], Class::Data, None, usize::MAX)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(
            build(u32::MAX, u32::MAX, vec![], Class::Data, None, usize::MAX)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
        assert_eq!(
            build(2, 2, vec![0; 15], Class::Data, None, usize::MAX)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(
            build(2, 2, vec![0; 16], Class::Data, None, 1)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
        assert_eq!(
            build(
                1,
                1,
                vec![0; 4],
                Class::Normal,
                Some(Mask {
                    factor: 1.0,
                    cutoff: 0.5
                }),
                usize::MAX
            )
            .unwrap_err()
            .kind(),
            JobErrorKind::InvalidRequest
        );
        for mask in [
            Mask {
                factor: f64::NAN,
                cutoff: 0.5,
            },
            Mask {
                factor: 2.0,
                cutoff: 0.5,
            },
            Mask {
                factor: 1.0,
                cutoff: -0.1,
            },
        ] {
            assert_eq!(
                build(1, 1, vec![255; 4], Class::Colour, Some(mask), usize::MAX)
                    .unwrap_err()
                    .kind(),
                JobErrorKind::InvalidRequest
            );
        }
    }
}
