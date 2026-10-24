//! Dedicated binding page mips from ORIGINAL source class levels. Never reduce
//! padded/composite page pixels. POT phase alignment is exact; NPOT resampling
//! requires the finite source correspondence gate and independent acceptance.
//! MASK alpha has its own postcoverage semantic/decoded gate: this sampler gate
//! covers colour RGB, meaningful data channels and normal RGB/frame vectors;
//! non-MASK alpha retains its numeric source floors. Correspondence samples use
//! caller-derived decoded geometry texel-Jacobian LOD bias (finite isotropic
//! surface footprints only, not all perspective/anisotropic camera footprints).
use super::{
    crops::{Crop, SourceSampler},
    mips::{self, Class, Level},
    sparse_mips,
};
use crate::{
    gltf_source::source::TextureSlot, mesh_large::config::Profile, JobError, JobErrorKind,
};
use std::mem::size_of;

#[derive(Debug)]
pub(super) struct Chain {
    pub original: mips::Chain,
    pub page: sparse_mips::Chain,
    /// Includes caller-borrowed source RGBA still live during owned source copy,
    /// original full chain, and page RGBA/support chain; native excluded.
    pub charged_bytes: u64,
}
#[derive(Clone, Copy)]
pub(super) struct SamplerSample {
    pub source_uv: [f64; 2],
    pub page_uv: [f64; 2],
    pub weight: f64,
    /// Caller derives this from original/page texel Jacobian rho for the pinned
    /// geometry/footprint. POT source has bias D; NPOT cannot assume integer D.
    pub source_lod_bias: f64,
}
pub(super) struct SamplerCheck<'a> {
    pub samples: &'a [SamplerSample],
    pub sampler: SourceSampler,
    pub slot: TextureSlot,
    pub profile: Profile,
    pub density_drop: u32,
    pub mask: bool,
}
#[derive(Debug, serde::Serialize)]
pub(super) struct SamplerReceipt {
    pub evaluations: u64,
    pub worst_byte_error: f64,
    pub normal_max_angle: f64,
}
fn error(kind: JobErrorKind, s: &str) -> JobError {
    JobError::new(kind, s.to_owned())
}
fn resource() -> JobError {
    error(
        JobErrorKind::ResourceLimit,
        "source sampler mip ownership/size overflow",
    )
}
fn invalid(s: &str) -> JobError {
    error(JobErrorKind::InvalidRequest, s)
}
fn quant(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}
fn linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(v: f64) -> f64 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}
fn unit(v: [f64; 3]) -> Result<[f64; 3], JobError> {
    let n = v.iter().map(|x| x * x).sum::<f64>();
    if !n.is_finite() || n <= 1e-24 {
        return Err(error(
            JobErrorKind::Unsupported,
            "source sampler zero/nonfinite normal",
        ));
    }
    Ok(v.map(|x| x / n.sqrt()))
}
fn wrap(i: i64, n: u32, mode: u32) -> usize {
    let n = i64::from(n);
    match mode {
        10497 => i.rem_euclid(n) as usize,
        33648 => {
            let p = i.rem_euclid(2 * n);
            (if p < n { p } else { 2 * n - 1 - p }) as usize
        }
        _ => i.clamp(0, n - 1) as usize,
    }
}
trait ImageLevel {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn rgba(&self) -> &[u8];
}
impl ImageLevel for Level {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}
impl ImageLevel for sparse_mips::Level {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

fn sample<L: ImageLevel>(
    level: &L,
    uv: [f64; 2],
    sampler: SourceSampler,
    filter_linear: bool,
    class: Class,
) -> [f64; 4] {
    let fetch = |x: i64, y: i64| {
        let i = (wrap(y, level.height(), sampler.wrap_t) * level.width() as usize
            + wrap(x, level.width(), sampler.wrap_s))
            * 4;
        std::array::from_fn::<_, 4, _>(|a| {
            let v = f64::from(level.rgba()[i + a]) / 255.0;
            if class == Class::Colour && a < 3 {
                linear(v)
            } else {
                v
            }
        })
    };
    let x = uv[0] * f64::from(level.width());
    let y = uv[1] * f64::from(level.height());
    if !filter_linear {
        return fetch(x.floor() as i64, y.floor() as i64);
    }
    let x = x - 0.5;
    let y = y - 0.5;
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let fx = x - x.floor();
    let fy = y - y.floor();
    let taps = [
        fetch(ix, iy),
        fetch(ix + 1, iy),
        fetch(ix, iy + 1),
        fetch(ix + 1, iy + 1),
    ];
    std::array::from_fn(|a| {
        (taps[0][a] * (1.0 - fx) + taps[1][a] * fx) * (1.0 - fy)
            + (taps[2][a] * (1.0 - fx) + taps[3][a] * fx) * fy
    })
}
fn validate_sampler(s: SourceSampler) -> Result<(), JobError> {
    if ![10497, 33648, 33071].contains(&s.wrap_s)
        || ![10497, 33648, 33071].contains(&s.wrap_t)
        || ![9728, 9729, 9984, 9985, 9986, 9987].contains(&s.min_filter)
        || ![9728, 9729].contains(&s.mag_filter)
    {
        return Err(invalid("source sampler enumerant invalid"));
    }
    Ok(())
}

pub(super) fn build(
    source: (&[u8], [u32; 2]),
    crop: &Crop,
    drop: u32,
    class: Class,
    sampler: SourceSampler,
    memory_limit: u64,
) -> Result<Chain, JobError> {
    let (source, source_size) = source;
    if class == Class::Id {
        // Source material pages resample with filters; IDs never do.
        return Err(JobError::new(
            JobErrorKind::InvalidRequest,
            "ID pages are not source-sampler pages",
        ));
    }
    validate_sampler(sampler)?;
    if drop > 12
        || crop.gutter != 0
        || crop.width != crop.interior_width
        || crop.height != crop.interior_height
        || crop.width < 4
        || crop.height < 4
        || !crop.width.is_power_of_two()
        || !crop.height.is_power_of_two()
    {
        return Err(invalid(
            "source sampler page requires whole POT page with zero gutter and drop<=12",
        ));
    }
    let source_bytes = u64::from(source_size[0])
        .checked_mul(u64::from(source_size[1]))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(resource)?;
    if source_size.contains(&0) || source_bytes != source.len() as u64 {
        return Err(invalid("original source RGBA shape invalid"));
    }
    if source_bytes.checked_mul(2).is_none_or(|n| n > memory_limit) {
        return Err(resource());
    }
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(source.len())
        .map_err(|_| resource())?;
    owned.extend_from_slice(source);
    let original = mips::build(
        source_size[0],
        source_size[1],
        owned,
        class,
        None,
        usize::try_from(
            memory_limit
                .checked_sub(source_bytes)
                .ok_or_else(resource)?,
        )
        .map_err(|_| resource())?,
    )?;
    let index = (drop as usize).min(original.levels.len() - 1);
    let reduced = [original.levels[index].width, original.levels[index].height];
    let exact = source_size.iter().all(|x| x.is_power_of_two());
    if exact
        && (crop.origin[0].rem_euclid(i64::from(reduced[0])) != 0
            || crop.origin[1].rem_euclid(i64::from(reduced[1])) != 0)
    {
        return Err(invalid("POT source sampler page phase is unaligned"));
    }
    let mut charged = (original.charged_bytes as u64)
        .checked_add(source_bytes)
        .ok_or_else(resource)?;
    let mut count = 0usize;
    let (mut w, mut h) = (crop.width, crop.height);
    loop {
        charged = charged
            .checked_add(
                u64::from(w)
                    .checked_mul(u64::from(h))
                    .and_then(|x| x.checked_mul(8))
                    .ok_or_else(resource)?,
            )
            .ok_or_else(resource)?;
        count += 1;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    charged = charged
        .checked_add(
            (count as u64)
                .checked_mul(size_of::<sparse_mips::Level>() as u64)
                .ok_or_else(resource)?,
        )
        .ok_or_else(resource)?;
    if charged > memory_limit {
        return Err(resource());
    }
    let mut levels = Vec::new();
    levels.try_reserve_exact(count).map_err(|_| resource())?;
    let (mut w, mut h) = (crop.width, crop.height);
    for level_index in 0..count {
        let src = &original.levels[(index + level_index).min(original.levels.len() - 1)];
        let n = (w as usize)
            .checked_mul(h as usize)
            .and_then(|x| x.checked_mul(4))
            .ok_or_else(resource)?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(n).map_err(|_| resource())?;
        rgba.resize(n, 0);
        for y in 0..h {
            for x in 0..w {
                // Inverse fixed positive source->page affine. Rectangular axes that
                // already reached1 keep their original base/page extent in mapping.
                let uv = [
                    ((f64::from(x) + 0.5) * f64::from(crop.width) / f64::from(w)
                        + crop.origin[0] as f64)
                        / f64::from(reduced[0]),
                    ((f64::from(y) + 0.5) * f64::from(crop.height) / f64::from(h)
                        + crop.origin[1] as f64)
                        / f64::from(reduced[1]),
                ];
                let mut p = sample(src, uv, sampler, !exact, Class::Data);
                // NPOT interpolation is class-correct, rather than sRGB-byte
                // interpolation. POT samples are exact texel-centre copies.
                if !exact && class == Class::Colour {
                    let xx = uv[0] * f64::from(src.width) - 0.5;
                    let yy = uv[1] * f64::from(src.height) - 0.5;
                    let ix = xx.floor() as i64;
                    let iy = yy.floor() as i64;
                    let fx = xx - xx.floor();
                    let fy = yy - yy.floor();
                    for (a, value) in p[..3].iter_mut().enumerate() {
                        let get = |dx: i64, dy: i64| {
                            linear(
                                f64::from(
                                    src.rgba[(wrap(iy + dy, src.height, sampler.wrap_t)
                                        * src.width as usize
                                        + wrap(ix + dx, src.width, sampler.wrap_s))
                                        * 4
                                        + a],
                                ) / 255.0,
                            )
                        };
                        *value = srgb(
                            (get(0, 0) * (1.0 - fx) + get(1, 0) * fx) * (1.0 - fy)
                                + (get(0, 1) * (1.0 - fx) + get(1, 1) * fx) * fy,
                        );
                    }
                }
                if class == Class::Normal && !exact {
                    let raw = [p[0] * 2.0 - 1.0, p[1] * 2.0 - 1.0, p[2] * 2.0 - 1.0];
                    let v = if raw.iter().map(|x| x * x).sum::<f64>() <= 1e-24 {
                        // NPOT cancellation: greatest tap weight, tie source row ID.
                        let xx = uv[0] * f64::from(src.width) - 0.5;
                        let yy = uv[1] * f64::from(src.height) - 0.5;
                        let ix = xx.floor() as i64;
                        let iy = yy.floor() as i64;
                        let fx = xx - xx.floor();
                        let fy = yy - yy.floor();
                        let mut best = (-1.0, usize::MAX, [0.0, 0.0, 1.0]);
                        for (dx, dy, weight) in [
                            (0, 0, (1.0 - fx) * (1.0 - fy)),
                            (1, 0, fx * (1.0 - fy)),
                            (0, 1, (1.0 - fx) * fy),
                            (1, 1, fx * fy),
                        ] {
                            let id = wrap(iy + dy, src.height, sampler.wrap_t) * src.width as usize
                                + wrap(ix + dx, src.width, sampler.wrap_s);
                            if weight > best.0 || (weight == best.0 && id < best.1) {
                                best = (
                                    weight,
                                    id,
                                    unit(std::array::from_fn(|a| {
                                        f64::from(src.rgba[id * 4 + a]) / 127.5 - 1.0
                                    }))?,
                                );
                            }
                        }
                        best.2
                    } else {
                        unit(raw)?
                    };
                    for (a, &component) in v.iter().enumerate() {
                        p[a] = (component + 1.0) * 0.5;
                    }
                }
                let i = (y as usize * w as usize + x as usize) * 4;
                for (a, &v) in p.iter().enumerate() {
                    rgba[i + a] = quant(v);
                }
            }
        }
        let mut support = Vec::new();
        support.try_reserve_exact(n / 4).map_err(|_| resource())?;
        support.resize(n / 4, 1);
        levels.push(sparse_mips::Level {
            width: w,
            height: h,
            rgba,
            support,
        });
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    // Keep source-level allocation ownership explicitly in returned chain.
    let page_charge = charged - original.charged_bytes as u64 - source_bytes;
    Ok(Chain {
        original,
        page: sparse_mips::Chain {
            class,
            levels,
            charged_bytes: page_charge,
        },
        charged_bytes: charged,
    })
}
fn at_lod<L: ImageLevel>(
    levels: &[L],
    uv: [f64; 2],
    lod: f64,
    sampler: SourceSampler,
    class: Class,
) -> [f64; 4] {
    if lod <= 0.0 {
        return sample(&levels[0], uv, sampler, sampler.mag_filter == 9729, class);
    }
    if sampler.min_filter == 9728 || sampler.min_filter == 9729 {
        return sample(&levels[0], uv, sampler, sampler.min_filter == 9729, class);
    }
    let lod = lod.clamp(0.0, (levels.len() - 1) as f64);
    let linear = matches!(sampler.min_filter, 9985 | 9987);
    if matches!(sampler.min_filter, 9984 | 9985) {
        return sample(
            &levels[(lod + 0.5).floor() as usize],
            uv,
            sampler,
            linear,
            class,
        );
    }
    let lo = lod.floor() as usize;
    let hi = (lo + 1).min(levels.len() - 1);
    let f = lod - lo as f64;
    let a = sample(&levels[lo], uv, sampler, linear, class);
    let b = sample(&levels[hi], uv, sampler, linear, class);
    std::array::from_fn(|i| a[i] * (1.0 - f) + b[i] * f)
}

pub(super) fn check_samples(
    chain: &Chain,
    delivered: &[Level],
    context: &SamplerCheck<'_>,
) -> Result<SamplerReceipt, JobError> {
    check_levels(chain, delivered, context, true)
}
/// Source sampler gate before codec, borrowing authored levels without a clone.
pub(super) fn check_authored(
    chain: &Chain,
    context: &SamplerCheck<'_>,
) -> Result<SamplerReceipt, JobError> {
    check_levels(chain, &chain.page.levels, context, false)
}
/// The sampler has no support mask: its band uses the sampled level's area,
/// unlike the leaf texel gate's count of positively supported texels.
fn sampled_level_area<L: ImageLevel>(levels: &[L], min_filter: u32, lod: f64) -> u64 {
    let index = if lod <= 0. || matches!(min_filter, 9728 | 9729) {
        0
    } else if matches!(min_filter, 9984 | 9985) {
        (lod + 0.5).floor() as usize
    } else {
        lod.ceil() as usize
    };
    let level = &levels[index.min(levels.len() - 1)];
    u64::from(level.width()) * u64::from(level.height())
}

fn check_levels<L: ImageLevel>(
    chain: &Chain,
    delivered: &[L],
    context: &SamplerCheck<'_>,
    decoded: bool,
) -> Result<SamplerReceipt, JobError> {
    let samples = context.samples;
    let sampler = context.sampler;
    let slot = context.slot;
    let profile = context.profile;
    let drop = context.density_drop;
    let mask = context.mask;

    validate_sampler(sampler)?;
    if chain.original.levels.is_empty() || delivered.len() != chain.page.levels.len() || drop > 12 {
        return Err(invalid("source sampler chain shape/drop invalid"));
    }
    for (reference, actual) in chain.page.levels.iter().zip(delivered) {
        if actual.width() != reference.width
            || actual.height() != reference.height
            || (actual.width() as usize)
                .checked_mul(actual.height() as usize)
                .and_then(|x| x.checked_mul(4))
                != Some(actual.rgba().len())
        {
            return Err(invalid("source sampler delivered level shape invalid"));
        }
    }
    for s in samples {
        if s.source_uv
            .iter()
            .chain(s.page_uv.iter())
            .any(|x| !x.is_finite() || x.abs() > 1e6)
            || !s.weight.is_finite()
            || s.weight < 0.0
            || !s.source_lod_bias.is_finite()
        {
            return Err(invalid("source sampler sample invalid"));
        }
    }
    // Raw decoded normal-length floors apply to stored texels BEFORE any
    // normalization, not legitimate shrinkage of LINEAR-filtered unit vectors.
    if slot == TextureSlot::Normal {
        let (mean_cap, max_cap) = if profile == Profile::Fast {
            (0.08, 0.20)
        } else if profile == Profile::Quality {
            (0.03, 0.10)
        } else {
            (0.05, 0.15)
        };
        for level in delivered {
            let (mut sum, mut max) = (0.0, 0.0_f64);
            for p in level.rgba().as_chunks::<4>().0 {
                let length = p[..3]
                    .iter()
                    .map(|&b| {
                        let v = f64::from(b) / 127.5 - 1.0;
                        v * v
                    })
                    .sum::<f64>()
                    .sqrt();
                if length <= 1e-12 || !length.is_finite() {
                    return Err(error(
                        JobErrorKind::Unsupported,
                        "source sampler decoded zero/nonfinite normal",
                    ));
                }
                let e = (length - 1.0).abs();
                sum += e;
                max = max.max(e);
            }
            if sum / (level.rgba().len() / 4) as f64 > mean_cap || max > max_cap {
                return Err(error(
                    JobErrorKind::Unsupported,
                    "source sampler stored normal length floor",
                ));
            }
        }
    }
    let page_sampler = SourceSampler {
        wrap_s: 33071,
        wrap_t: 33071,
        ..sampler
    };
    let channels: &[usize] = match (slot, mask) {
        (TextureSlot::Occlusion, false) => &[0, 3],
        (TextureSlot::MetallicRoughness, false) => &[1, 2, 3],
        (_, false) => &[0, 1, 2, 3],
        (TextureSlot::Occlusion, true) => &[0],
        (TextureSlot::MetallicRoughness, true) => &[1, 2],
        (_, true) => &[0, 1, 2],
    };
    let quality = profile == Profile::Quality;
    let fast = profile == Profile::Fast;
    let mut receipt = SamplerReceipt {
        evaluations: 0,
        worst_byte_error: 0.0,
        normal_max_angle: 0.0,
    };
    // Negative magnification witness and explicit quarter-LOD grid. The finite
    // GLES3.1 section8.13.1 fixes lambda<=0 magnification for every filter pair;
    // a pinned viewer independently checks footprint/nearest-mip tie behavior.
    for lod in std::iter::once(-1.0).chain((0..=4 * (delivered.len() - 1)).map(|i| i as f64 / 4.0))
    {
        let mut squared = [0.0; 4];
        let mut total = 0.0;
        let mut maximum = [0.0_f64; 4];
        let mut angle_sum = 0.0;
        let mut angles = Vec::new();
        angles
            .try_reserve_exact(samples.len())
            .map_err(|_| resource())?;
        for s in samples {
            if s.weight == 0.0 {
                continue;
            }
            let expected = at_lod(
                &chain.original.levels,
                s.source_uv,
                lod + s.source_lod_bias,
                sampler,
                chain.original.class,
            );
            let actual = at_lod(
                delivered,
                s.page_uv,
                lod,
                page_sampler,
                chain.original.class,
            );
            total += s.weight;
            receipt.evaluations = receipt.evaluations.checked_add(1).ok_or_else(resource)?;
            for &a in channels {
                let e = if chain.original.class == Class::Colour && a < 3 {
                    (srgb(expected[a]) - srgb(actual[a])).abs() * 255.0
                } else {
                    (expected[a] - actual[a]).abs() * 255.0
                };
                squared[a] += e * e * s.weight;
                maximum[a] = maximum[a].max(e);
                receipt.worst_byte_error = receipt.worst_byte_error.max(e);
            }
            if slot == TextureSlot::Normal {
                let v = [
                    actual[0] * 2.0 - 1.0,
                    actual[1] * 2.0 - 1.0,
                    actual[2] * 2.0 - 1.0,
                ];
                let q = [
                    expected[0] * 2.0 - 1.0,
                    expected[1] * 2.0 - 1.0,
                    expected[2] * 2.0 - 1.0,
                ];
                let v = unit(v)?;
                let q = unit(q)?;
                let angle = v
                    .iter()
                    .zip(q)
                    .map(|(a, b)| a * b)
                    .sum::<f64>()
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees();
                angle_sum += angle * s.weight;
                receipt.normal_max_angle = receipt.normal_max_angle.max(angle);
                angles.push((angle, s.weight));
            }
        }
        if total == 0.0 {
            continue;
        }
        if !total.is_finite() {
            return Err(invalid("source sampler sample total weight invalid"));
        }
        if decoded && matches!(slot, TextureSlot::BaseColor | TextureSlot::Emissive) {
            if let Some(floor) = super::delivery::tiny_colour_rgb_floor(
                profile,
                sampled_level_area(delivered, page_sampler.min_filter, lod),
            ) {
                let rgb_sse: f64 = squared[..3].iter().sum();
                let psnr = if rgb_sse == 0. {
                    f64::INFINITY
                } else {
                    10. * (65025. * total * 3. / rgb_sse).log10()
                };
                if psnr < floor {
                    return Err(error(
                        JobErrorKind::Unsupported,
                        &format!("source sampler combined RGB LOD{lod} PSNR{psnr} below {floor}"),
                    ));
                }
            }
        }
        for &a in channels {
            let psnr = if squared[a] == 0.0 {
                f64::INFINITY
            } else {
                10.0 * (65025.0 * total / squared[a]).log10()
            };
            let (floor, cap) = if a == 3 {
                (
                    if quality { 34.0 } else { 30.0 },
                    if fast {
                        80.0
                    } else if quality {
                        32.0
                    } else {
                        64.0
                    },
                )
            } else if matches!(slot, TextureSlot::BaseColor | TextureSlot::Emissive) {
                if decoded && matches!(profile, Profile::Fast | Profile::Balanced) {
                    // At a fractional LOD use the coarser participating level's
                    // band. Pre-encode correspondence retains its stricter floors.
                    (
                        super::delivery::colour_psnr(
                            profile,
                            sampled_level_area(delivered, page_sampler.min_filter, lod),
                        ),
                        if fast { 160. } else { 144. },
                    )
                } else {
                    (
                        if quality { 34.0 } else { 22.0 },
                        if fast {
                            56.0
                        } else if quality {
                            24.0
                        } else {
                            32.0
                        },
                    )
                }
            } else if slot == TextureSlot::Normal {
                (30.0, 112.0)
            } else {
                (
                    30.0,
                    if slot == TextureSlot::Occlusion {
                        if quality {
                            8.0
                        } else {
                            80.0
                        }
                    } else {
                        32.0
                    },
                )
            };
            if psnr < floor || maximum[a] > cap {
                return Err(error(
                    JobErrorKind::Unsupported,
                    &format!(
                        "source sampler {slot:?} LOD{lod} channel{a} PSNR{psnr} max{}",
                        maximum[a]
                    ),
                ));
            }
        }
        if slot == TextureSlot::Normal {
            angles.sort_by(|a, b| a.0.total_cmp(&b.0));
            let (mut accumulated, mut p99) = (0.0, 0.0);
            for (angle, weight) in angles {
                accumulated += weight;
                p99 = angle;
                if accumulated >= total * 0.99 {
                    break;
                }
            }
            if angle_sum / total > 3.0 || p99 > 12.0 {
                return Err(error(
                    JobErrorKind::Unsupported,
                    "source sampler normal angle/length floor",
                ));
            }
        }
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::super::crops;
    use super::*;
    fn decoded(chain: &Chain) -> Vec<Level> {
        chain
            .page
            .levels
            .iter()
            .map(|l| Level {
                width: l.width,
                height: l.height,
                rgba: l.rgba.clone(),
            })
            .collect()
    }
    fn sampler(min: u32, wrap: u32) -> SourceSampler {
        SourceSampler {
            wrap_s: wrap,
            wrap_t: 33071,
            min_filter: min,
            mag_filter: 9729,
        }
    }
    fn original() -> Vec<u8> {
        [0u8, 0, 0, 255]
            .iter()
            .flat_map(|&v| [v, v, v, 255])
            .collect()
    }
    fn flat_colour_chain() -> Chain {
        let rgba = [100, 100, 100, 255].repeat(32 * 32);
        // Only one supported leaf texel, while sampler level0 area is1024.
        let mut support = vec![0; 32 * 32];
        support[0] = 1;
        Chain {
            original: mips::build(32, 32, rgba.clone(), Class::Colour, None, usize::MAX).unwrap(),
            page: sparse_mips::build(32, 32, &rgba, &support, Class::Colour, u64::MAX).unwrap(),
            charged_bytes: 0,
        }
    }
    #[test]
    fn decoded_colour_sampler_bands_caps_and_authored_correspondence() {
        let chain = flat_colour_chain();
        let samples = [SamplerSample {
            source_uv: [0.5; 2],
            page_uv: [0.5; 2],
            weight: 1.,
            source_lod_bias: 0.,
        }];
        for min in [9728, 9729, 9984, 9985, 9986, 9987] {
            let mode = sampler(min, 33071);
            for profile in [Profile::Fast, Profile::Balanced] {
                let context = SamplerCheck {
                    samples: &samples,
                    sampler: mode,
                    slot: TextureSlot::BaseColor,
                    profile,
                    density_drop: 0,
                    mask: false,
                };
                let mut delivered = decoded(&chain);
                for level in &mut delivered {
                    let area = level.width * level.height;
                    // Red errors24/38/48 have20.53/16.54/14.51dB;
                    // tiny G/B errors24 keep joint RGB above16dB.
                    // Fractional blends cross the finer band's floor, making
                    // the coarser participating band a sensitive requirement.
                    let offsets = if area >= 1024 {
                        [24; 3]
                    } else if area >= 64 {
                        [38; 3]
                    } else {
                        [48, 24, 24]
                    };
                    for pixel in level.rgba.as_chunks_mut::<4>().0 {
                        for (v, offset) in pixel[..3].iter_mut().zip(offsets) {
                            *v += offset;
                        }
                    }
                }
                check_samples(&chain, &delivered, &context).unwrap();
                check_authored(&chain, &context).unwrap();
                // A leaf gate would use one supported texel and admit error30
                // under13dB/channel and16dB RGB. The sampler uses area1024
                // and must reject it at20dB/channel.
                for pixel in delivered[0].rgba.as_chunks_mut::<4>().0 {
                    pixel[..3].fill(130);
                }
                assert!(check_samples(&chain, &delivered, &context).is_err());
                let mut wrong = flat_colour_chain();
                for level in &mut wrong.page.levels {
                    for pixel in level.rgba.as_chunks_mut::<4>().0 {
                        pixel[..3].fill(124);
                    }
                }
                assert!(
                    check_authored(&wrong, &context).is_err(),
                    "authored floor remains22dB"
                );
                if matches!(min, 9984..=9987) {
                    let mut bad_tail = decoded(&chain);
                    bad_tail.last_mut().unwrap().rgba[..3].fill(152);
                    let error = check_samples(&chain, &bad_tail, &context).unwrap_err();
                    assert!(error.message().contains("combined RGB"), "{error:?}");
                    bad_tail.last_mut().unwrap().rgba[..3].copy_from_slice(&[160, 100, 100]);
                    assert!(
                        check_samples(&chain, &bad_tail, &context).is_err(),
                        "tiny channel floor"
                    );
                }
            }
        }
        let mut delivered = decoded(&chain);
        delivered[0].rgba[0] = 255;
        let weighted = [
            samples[0],
            SamplerSample {
                source_uv: [1. / 64.; 2],
                page_uv: [1. / 64.; 2],
                weight: 1e-6,
                source_lod_bias: 0.,
            },
        ];
        for (profile, accepts) in [(Profile::Fast, true), (Profile::Balanced, false)] {
            let context = SamplerCheck {
                samples: &weighted,
                sampler: sampler(9987, 33071),
                slot: TextureSlot::BaseColor,
                profile,
                density_drop: 0,
                mask: false,
            };
            assert_eq!(
                check_samples(&chain, &delivered, &context).is_ok(),
                accepts,
                "155-byte outlier vs160/144 caps"
            );
        }
    }

    #[test]
    fn sampled_level_selector_uses_literal_gles_mip_rules() {
        let levels = decoded(&flat_colour_chain());
        for min in [9728, 9729, 9984, 9985, 9986, 9987] {
            assert_eq!(sampled_level_area(&levels, min, -1.), 1024);
            assert_eq!(sampled_level_area(&levels, min, 0.), 1024);
        }
        for min in [9728, 9729] {
            assert_eq!(sampled_level_area(&levels, min, 100.), 1024);
        }
        for min in [9984, 9985] {
            for (lod, area) in [
                (0.25, 1024),
                (0.5, 256),
                (0.75, 256),
                (2.25, 64),
                (2.5, 16),
                (100., 1),
            ] {
                assert_eq!(
                    sampled_level_area(&levels, min, lod),
                    area,
                    "nearest {min}/{lod}"
                );
            }
        }
        for min in [9986, 9987] {
            for (lod, area) in [
                (0.25, 256),
                (0.5, 256),
                (0.75, 256),
                (2., 64),
                (2.25, 16),
                (100., 1),
            ] {
                assert_eq!(
                    sampled_level_area(&levels, min, lod),
                    area,
                    "blend {min}/{lod}"
                );
            }
        }
    }
    // Separate literal source levels and integer wrapping, no production class
    // chain, filtering, wrap, LOD or planning helper used as expected truth.
    fn reference(data: &[f64], u: f64, mode: u32, linear_filter: bool) -> f64 {
        let fetch = |i: i64| {
            let n = data.len() as i64;
            let k = match mode {
                10497 => ((i % n) + n) % n,
                33648 => {
                    let cycle = ((i % (2 * n)) + 2 * n) % (2 * n);
                    if cycle < n {
                        cycle
                    } else {
                        2 * n - cycle - 1
                    }
                }
                _ => i.max(0).min(n - 1),
            };
            data[k as usize]
        };
        if !linear_filter {
            return fetch((u * data.len() as f64).floor() as i64);
        }
        let x = u * data.len() as f64 - 0.5;
        let left = x.floor() as i64;
        let fraction = x - x.floor();
        fetch(left) * (1.0 - fraction) + fetch(left + 1) * fraction
    }
    #[test]
    fn original_mips_phase_terminal_and_fractional_sampler_reference() {
        let crop = crops::plan_mip_aligned(&[[0.0, 0.0], [1.0, 1.0]], 4, 1, 32).unwrap();
        assert_eq!(crop.origin, [-4, -1]);
        assert_eq!((crop.width, crop.height), (16, 4));
        let literal = [vec![0.0, 0.0, 0.0, 255.0], vec![0.0, 128.0], vec![64.0]];
        for mode in [10497, 33648, 33071] {
            for min in [9984, 9985, 9986, 9987] {
                let sampler = sampler(min, mode);
                let chain = build(
                    (&original(), [4, 1]),
                    &crop,
                    0,
                    Class::Data,
                    sampler,
                    1 << 20,
                )
                .unwrap();
                assert_eq!(chain.page.levels.last().unwrap().rgba[0], 64);
                let delivered = decoded(&chain);
                let mut samples = Vec::new();
                for u in [0.0, 0.125, 0.375, 0.625, 0.875, 1.0] {
                    let uv = crops::remap_uv(
                        [u as f32, 0.5],
                        4,
                        1,
                        &crop,
                        [0, 0],
                        [crop.width, crop.height],
                    )
                    .unwrap();
                    samples.push(SamplerSample {
                        source_uv: [u, 0.5],
                        page_uv: uv.map(f64::from),
                        weight: 1.0,
                        source_lod_bias: 0.0,
                    });
                    for step in 0..=16 {
                        let lod = step as f64 * 0.25;
                        let slod = lod.min(2.0);
                        let filter = if lod == 0.0 {
                            true
                        } else {
                            matches!(min, 9985 | 9987)
                        };
                        let expected = if matches!(min, 9986 | 9987) && lod > 0.0 {
                            let low = slod.floor() as usize;
                            let high = (low + 1).min(2);
                            let f = slod - low as f64;
                            reference(&literal[low], u, mode, filter) * (1.0 - f)
                                + reference(&literal[high], u, mode, filter) * f
                        } else {
                            reference(&literal[(slod + 0.5).floor() as usize], u, mode, filter)
                        };
                        let value = at_lod(
                            &delivered,
                            uv.map(f64::from),
                            lod,
                            SourceSampler {
                                wrap_s: 33071,
                                ..sampler
                            },
                            Class::Data,
                        )[0] * 255.0;
                        assert!(
                            (value - expected).abs() < 1e-10,
                            "wrap{mode} filter{min} lod{lod} u{u}"
                        );
                    }
                }
                let receipt = check_samples(
                    &chain,
                    &delivered,
                    &SamplerCheck {
                        samples: &samples,
                        sampler,
                        slot: TextureSlot::Occlusion,
                        profile: Profile::Quality,
                        density_drop: 0,
                        mask: false,
                    },
                )
                .unwrap();
                assert!(receipt.worst_byte_error < 1e-10);
                let authored = check_authored(
                    &chain,
                    &SamplerCheck {
                        samples: &samples,
                        sampler,
                        slot: TextureSlot::Occlusion,
                        profile: Profile::Quality,
                        density_drop: 0,
                        mask: false,
                    },
                )
                .unwrap();
                assert!(authored.worst_byte_error < 1e-10);
            }
        }
    }
    #[test]
    fn wrong_phase_shared_terminal_and_source_alpha_controls() {
        let crop = crops::plan_mip_aligned(&[[0.0, 0.0], [1.0, 1.0]], 4, 1, 32).unwrap();
        let sampler = sampler(9987, 10497);
        let mut wrong = crop.clone();
        wrong.origin[0] += 1;
        assert_eq!(
            build(
                (&original(), [4, 1]),
                &wrong,
                0,
                Class::Data,
                sampler,
                1 << 20
            )
            .unwrap_err()
            .kind(),
            JobErrorKind::InvalidRequest
        );
        let chain = build(
            (&original(), [4, 1]),
            &crop,
            0,
            Class::Data,
            sampler,
            1 << 20,
        )
        .unwrap();
        let uv =
            crops::remap_uv([0.625, 0.5], 4, 1, &crop, [0, 0], [crop.width, crop.height]).unwrap();
        let samples = [SamplerSample {
            source_uv: [0.625, 0.5],
            page_uv: uv.map(f64::from),
            weight: 1.0,
            source_lod_bias: 0.0,
        }];
        let context = SamplerCheck {
            samples: &samples,
            sampler,
            slot: TextureSlot::Occlusion,
            profile: Profile::Quality,
            density_drop: 0,
            mask: false,
        };
        let mut corrupt = decoded(&chain);
        corrupt.last_mut().unwrap().rgba[0] = 128;
        assert_eq!(
            check_samples(&chain, &corrupt, &context)
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported
        );
        let mut corrupt = decoded(&chain);
        for l in &mut corrupt {
            for p in l.rgba.as_chunks_mut::<4>().0 {
                p[3] = 0;
            }
        }
        assert_eq!(
            check_samples(&chain, &corrupt, &context)
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported
        );
        assert!(check_samples(
            &chain,
            &corrupt,
            &SamplerCheck {
                mask: true,
                ..context
            }
        )
        .is_ok());
        assert_eq!(
            build((&original(), [4, 1]), &crop, 0, Class::Data, sampler, 1)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
    }
    #[test]
    fn srgb_sampler_is_linear_light_and_npot_has_explicit_finite_gate() {
        let level = Level {
            width: 2,
            height: 1,
            rgba: vec![0, 0, 0, 255, 255, 255, 255, 255],
        };
        let sampler = sampler(9987, 33071);
        let filtered = at_lod(&[level], [0.5, 0.5], 0.0, sampler, Class::Colour);
        assert!((srgb(filtered[0]) * 255.0 - 187.516).abs() < 0.01); // wrong numeric byte average is127.5
        let source = [0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255];
        let crop = crops::plan_mip_aligned(&[[0.0, 0.0], [1.0, 1.0]], 3, 1, 32).unwrap();
        let chain = build((&source, [3, 1]), &crop, 0, Class::Colour, sampler, 1 << 20).unwrap();
        let uv =
            crops::remap_uv([0.5, 0.5], 3, 1, &crop, [0, 0], [crop.width, crop.height]).unwrap();
        let samples = [SamplerSample {
            source_uv: [0.5, 0.5],
            page_uv: uv.map(f64::from),
            weight: 1.0,
            source_lod_bias: 0.0,
        }];
        // This point may pass; it is NOT an all-NPOT exactness theorem. Caller
        // must derive rho bias/finite footprint grid and accept only actual gates.
        let result = check_samples(
            &chain,
            &decoded(&chain),
            &SamplerCheck {
                samples: &samples,
                sampler,
                slot: TextureSlot::BaseColor,
                profile: Profile::Fast,
                density_drop: 0,
                mask: false,
            },
        );
        assert!(result.is_ok() || result.err().unwrap().kind() == JobErrorKind::Unsupported);
        assert_eq!(chain.page.levels.last().unwrap().support, vec![1]);
    }
    #[test]
    fn gles31_zero_mag_min_threshold_quarter_and_separate_bias_control() {
        // GLES3.1 section8.13.1: lambda<=0 magnification, otherwise
        // minification, including magLINEAR/minNEAREST_MIPMAP_NEAREST.
        // The old desktop c=.5 exception would incorrectly return127.5 at.25.
        let levels = [
            Level {
                width: 2,
                height: 1,
                rgba: vec![0, 0, 0, 255, 255, 255, 255, 255],
            },
            Level {
                width: 1,
                height: 1,
                rgba: vec![128, 128, 128, 255],
            },
        ];
        let mode = SourceSampler {
            wrap_s: 33071,
            wrap_t: 33071,
            min_filter: 9984,
            mag_filter: 9729,
        };
        let at = |lambda| at_lod(&levels, [0.5, 0.5], lambda, mode, Class::Data)[0] * 255.0;
        assert_eq!(at(-1.0), 127.5);
        assert_eq!(at(0.0), 127.5);
        assert_eq!(at(0.25), 255.0);
        assert_eq!(at(0.75), 128.0);
        // Original/page independently biased lambdas can straddlezero: a single
        // shared mag/min decision would conceal this128-byte-class deviation.
        let page_lambda = 0.0;
        let actual_source_bias = 0.25;
        assert!((at(page_lambda + actual_source_bias) - at(page_lambda)).abs() > 112.0);
        let other = SourceSampler {
            min_filter: 9987,
            ..mode
        };
        let trilinear = at_lod(&levels, [0.5, 0.5], 0.25, other, Class::Data)[0] * 255.0;
        assert!((trilinear - 127.625).abs() < 1e-12);
    }
}
