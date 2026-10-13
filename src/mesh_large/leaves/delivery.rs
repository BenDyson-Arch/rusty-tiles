//! Explicit profile delivery. PNG is raw level0; KTX owns a complete class chain.
use super::{
    mips::{self, Class},
    sparse_mips,
};
use crate::{
    basisu_ktx2 as codec, gltf_source::source::TextureSlot, mesh_large::config::Profile, JobError,
    JobErrorKind,
};
use image::ImageEncoder;
use serde_json::{json, Value};

#[derive(Debug)]
pub(super) struct Delivery {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub mip_count: u32,
    pub receipts: Vec<Value>,
}
#[derive(Clone, Copy)]
pub(super) struct MaskDelivery<'a> {
    pub samples: &'a [super::mask::SurfaceSample],
    pub factor: f64,
    pub cutoff: f64,
    pub linear: bool,
    /// Explicit source minification texel filter; base keeps magnification.
    pub min_linear: bool,
}
fn error(kind: JobErrorKind, message: impl Into<String>) -> JobError {
    JobError::new(kind, message.into())
}
fn resource() -> JobError {
    error(
        JobErrorKind::ResourceLimit,
        "texture delivery workspace/size budget",
    )
}
fn fidelity(message: impl Into<String>) -> JobError {
    error(JobErrorKind::Unsupported, message)
}
fn class(slot: TextureSlot) -> Class {
    match slot {
        TextureSlot::BaseColor | TextureSlot::Emissive => Class::Colour,
        TextureSlot::Normal => Class::Normal,
        _ => Class::Data,
    }
}
fn metric(expected: &[u8], decoded: &[u8], support: &[u32], channel: usize) -> (f64, u8) {
    let mut sse = 0.;
    let mut count = 0.;
    let mut max = 0;
    for (i, &weight) in support.iter().enumerate() {
        if weight == 0 {
            continue;
        }
        let e = expected[i * 4 + channel].abs_diff(decoded[i * 4 + channel]);
        sse += f64::from(e).powi(2) * f64::from(weight);
        count += f64::from(weight);
        max = max.max(e);
    }
    (
        if sse == 0. {
            f64::INFINITY
        } else {
            10. * (65025. * count / sse).log10()
        },
        max,
    )
}
fn check_level(
    expected: &[u8],
    decoded: &[u8],
    support: &[u32],
    slot: TextureSlot,
    profile: Profile,
    mip: usize,
) -> Result<Value, JobError> {
    if expected.len() != decoded.len() || expected.len() != support.len() * 4 {
        return Err(error(JobErrorKind::InvalidState, "delivery decoded shape"));
    }
    let quality = profile == Profile::Quality;
    let fast = profile == Profile::Fast;
    let mut metrics = Vec::new();
    let channels: &[usize] = match slot {
        TextureSlot::Normal => &[0, 1, 2, 3],
        TextureSlot::Occlusion => &[0, 3],
        TextureSlot::MetallicRoughness => &[1, 2, 3],
        _ => &[0, 1, 2, 3],
    };
    for &channel in channels {
        let (psnr, max) = metric(expected, decoded, support, channel);
        let (floor, cap) = if channel == 3 {
            (
                if quality { 34. } else { 30. },
                if fast {
                    80
                } else if quality {
                    32
                } else {
                    64
                },
            )
        } else if class(slot) == Class::Colour {
            (
                if quality { 34. } else { 22. },
                if fast {
                    56
                } else if quality {
                    24
                } else {
                    32
                },
            )
        } else if slot == TextureSlot::Normal {
            (30., 112)
        } else {
            (
                30.,
                if slot == TextureSlot::Occlusion {
                    if quality {
                        8
                    } else {
                        80
                    }
                } else {
                    32
                },
            )
        };
        metrics.push(json!({"channel":channel,"psnr":if psnr.is_finite(){Some(psnr)}else{None},"exact":psnr.is_infinite(),"max":max,"floor":floor,"cap":cap}));
        if psnr < floor || max > cap {
            return Err(fidelity(format!(
                "texture {slot:?} {profile:?} mip{mip} channel{channel} PSNR{psnr} max{max}"
            )));
        }
    }
    let mut normals = Value::Null;
    if slot == TextureSlot::Normal {
        let mut sum = 0.;
        let mut weight_sum = 0.;
        let mut max: f64 = 0.;
        let mut angle_sum = 0.;
        let mut angles = Vec::new();
        angles
            .try_reserve_exact(support.len())
            .map_err(|_| resource())?;
        for (i, &weight) in support.iter().enumerate() {
            if weight == 0 {
                continue;
            }
            let vector = |p: &[u8]| {
                [
                    f64::from(p[0]) / 127.5 - 1.,
                    f64::from(p[1]) / 127.5 - 1.,
                    f64::from(p[2]) / 127.5 - 1.,
                ]
            };
            let v = vector(&decoded[i * 4..]);
            let q = vector(&expected[i * 4..]);
            let length = |v: [f64; 3]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
            let vl = length(v);
            let ql = length(q);
            if vl <= 1e-12 || ql <= 1e-12 || !vl.is_finite() {
                return Err(fidelity("decoded zero/nonfinite normal"));
            }
            let e = (vl - 1.).abs();
            let angle = (v.iter().zip(q).map(|(a, b)| a * b).sum::<f64>() / (vl * ql))
                .clamp(-1., 1.)
                .acos()
                .to_degrees();
            let w = f64::from(weight);
            sum += e * w;
            weight_sum += w;
            max = max.max(e);
            angle_sum += angle * w;
            angles.push((angle, weight));
        }
        if weight_sum == 0. {
            return Err(error(
                JobErrorKind::InvalidRequest,
                "normal page has no signal",
            ));
        }
        angles.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut cumulative = 0.;
        let mut p99 = 0.;
        for (angle, weight) in angles {
            cumulative += f64::from(weight);
            p99 = angle;
            if cumulative >= weight_sum * 0.99 {
                break;
            }
        }
        let mean = sum / weight_sum;
        let angular = angle_sum / weight_sum;
        let (mean_cap, max_cap) = if fast {
            (0.08, 0.20)
        } else if quality {
            (0.03, 0.10)
        } else {
            (0.05, 0.15)
        };
        normals = json!({"length_mean":mean,"length_max":max,"angle_mean":angular,"angle_p99":p99});
        if mean > mean_cap || max > max_cap || angular > 3. || p99 > 12. {
            return Err(fidelity(format!(
                "normal mip{mip} length{mean}/{max} angle{angular}/{p99}"
            )));
        }
    }
    Ok(json!({"mip":mip,"channels":metrics,"normal":normals}))
}

fn workspace_charge(dimensions: [u32; 2], profile: Profile) -> Result<u64, JobError> {
    let [width, height] = dimensions;
    let base = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(resource)?;
    let mut texels = 0u64;
    let (mut w, mut h) = (width, height);
    loop {
        texels = texels
            .checked_add(u64::from(w) * u64::from(h))
            .ok_or_else(resource)?;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    let borrowed = base.checked_mul(8).ok_or_else(resource)?;
    let charge = if profile == Profile::Lossless {
        base.checked_mul(32)
            .and_then(|n| n.checked_add(64 * 1024 * 1024))
            .ok_or_else(resource)?
    } else {
        texels
            .checked_mul(132)
            .and_then(|n| n.checked_add(64 * 1024 * 1024))
            .and_then(|n| n.checked_add(borrowed))
            .ok_or_else(resource)?
    };
    Ok(charge)
}

pub(super) fn encode(
    dimensions: [u32; 2],
    rgba: &[u8],
    support: &[u32],
    slot: TextureSlot,
    profile: Profile,
    memory_limit: u64,
    mask: Option<MaskDelivery<'_>>,
) -> Result<Delivery, JobError> {
    let [width, height] = dimensions;
    let base = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(resource)?;
    if base == 0
        || u64::try_from(rgba.len()).map_err(|_| resource())?
            != base.checked_mul(4).ok_or_else(resource)?
        || u64::try_from(support.len()).map_err(|_| resource())? != base
    {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "delivery RGBA/support shape",
        ));
    }
    let charge = workspace_charge(dimensions, profile)?;
    if charge > memory_limit {
        return Err(resource());
    }
    if profile == Profile::Lossless {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(
                usize::try_from(base.checked_mul(8).ok_or_else(resource)?)
                    .map_err(|_| resource())?,
            )
            .map_err(|_| resource())?;
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
            .map_err(|e| error(JobErrorKind::InvalidState, format!("PNG encode {e}")))?;
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|e| error(JobErrorKind::InvalidState, format!("PNG verify {e}")))?
            .into_rgba8();
        if decoded.as_raw() != rgba {
            return Err(error(
                JobErrorKind::InvalidState,
                "PNG changed literal RGBA8",
            ));
        }
        return Ok(Delivery {
            bytes,
            mime: "image/png",
            mip_count: 1,
            receipts: vec![
                json!({"profile":"lossless","level0_rgba_exact":true,"normal_normalized":false,"alpha_guard":false,"mips":"viewer generated outside guarantee","charged_bytes":charge}),
            ],
        });
    }
    let chain = sparse_mips::build(width, height, rgba, support, class(slot), memory_limit)?;
    encode_authored(&chain, slot, profile, memory_limit, mask, |_| {
        Ok(Value::Null)
    })
}

/// The atlas owner supplies the complete chain and the independent source-sampler check.
pub(super) fn encode_authored(
    chain: &sparse_mips::Chain,
    slot: TextureSlot,
    profile: Profile,
    memory_limit: u64,
    mask: Option<MaskDelivery<'_>>,
    mut validate: impl FnMut(&[mips::Level]) -> Result<Value, JobError>,
) -> Result<Delivery, JobError> {
    if profile == Profile::Lossless {
        return Err(error(
            JobErrorKind::InvalidState,
            "PNG cannot carry an authored mip chain",
        ));
    }
    let base = chain.levels.first().ok_or_else(resource)?;
    let charge = workspace_charge([base.width, base.height], profile)?;
    if charge > memory_limit {
        return Err(resource());
    }
    let mut delivered = Vec::new();
    delivered
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    for level in &chain.levels {
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(level.rgba.len())
            .map_err(|_| resource())?;
        pixels.extend_from_slice(&level.rgba);
        delivered.push(pixels);
    }
    let mut mask_receipts = Vec::new();
    mask_receipts
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    for (index, (level, pixels)) in chain.levels.iter().zip(&mut delivered).enumerate() {
        let receipt = if let Some(context) = mask {
            Some(super::mask::adjust(
                pixels,
                level.width,
                level.height,
                context.samples,
                context.factor,
                context.cutoff,
                if index == 0 {
                    context.linear
                } else {
                    context.min_linear
                },
            )?)
        } else {
            None
        };
        mask_receipts.push(receipt);
    }
    // Fixed postscale, PRE-GUARD expected bytes. Guard never changes the
    // semantic gap or the alpha quality reference, nor feeds another mip.
    let mut expected = Vec::new();
    expected
        .try_reserve_exact(delivered.len())
        .map_err(|_| resource())?;
    let mut guard_receipts = Vec::new();
    guard_receipts
        .try_reserve_exact(delivered.len())
        .map_err(|_| resource())?;
    for (index, (level, pixels)) in chain.levels.iter().zip(&mut delivered).enumerate() {
        let mut truth = Vec::new();
        truth
            .try_reserve_exact(pixels.len())
            .map_err(|_| resource())?;
        truth.extend_from_slice(pixels);
        if let Some(context) = mask {
            let mut candidate = Vec::new();
            candidate
                .try_reserve_exact(pixels.len())
                .map_err(|_| resource())?;
            candidate.extend_from_slice(pixels);
            let mut guarded = 0usize;
            for pixel in candidate.as_chunks_mut::<4>().0 {
                if pixel[3] < 255 && f64::from(pixel[3]) / 255. * context.factor >= context.cutoff {
                    pixel[3] += 1;
                    guarded += 1;
                }
            }
            let safe = super::mask::preserves_classifications(
                &truth,
                &candidate,
                [level.width, level.height],
                context.samples,
                context.factor,
                context.cutoff,
                if index == 0 {
                    context.linear
                } else {
                    context.min_linear
                },
            )?;
            let enabled = safe && guarded > 0;
            if enabled {
                *pixels = candidate;
            }
            guard_receipts.push(json!({"mip":index,"finite_sample_guard_enabled":enabled,"classifications_preserved":safe,"candidate_texels":guarded,"max_alpha_delta":if enabled {1}else{0},"scope":"positive-weight declared geometry samples only"}));
        } else {
            guard_receipts.push(Value::Null);
        }
        expected.push(truth);
    }
    let scalar = slot == TextureSlot::Occlusion && profile != Profile::Quality;
    if scalar {
        for pixels in &mut delivered {
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel[1] = pixel[0];
                pixel[2] = pixel[0];
            }
        }
    }
    let mut levels = Vec::new();
    levels
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    for (level, pixels) in chain.levels.iter().zip(&delivered) {
        levels.push(codec::MipLevel {
            rgba: pixels,
            width: level.width,
            height: level.height,
        });
    }
    let effort = match profile {
        Profile::Fast => 0,
        Profile::Balanced => 1,
        _ => 2,
    };
    let mode = if scalar {
        codec::Mode::Etc1s { quality: 90 }
    } else {
        codec::Mode::Uastc { zstd: true }
    };
    let bytes = codec::encode_ktx2_mips(
        &levels,
        mode,
        codec::Params {
            effort: if scalar { 0 } else { effort },
            transfer: if class(slot) == Class::Colour {
                codec::Transfer::Srgb
            } else {
                codec::Transfer::Linear
            },
            mipmaps: false,
        },
    )?;
    let mut receipts = Vec::new();
    receipts
        .try_reserve_exact(chain.levels.len() * 3 + 1)
        .map_err(|_| resource())?;
    let mut decoded_chain = Vec::new();
    decoded_chain
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    for (index, level) in chain.levels.iter().enumerate() {
        let decoded = codec::decode_level_rgba(&bytes, index as u32, level.width, level.height)?;
        receipts.push(check_level(
            &expected[index],
            &decoded,
            &level.support,
            slot,
            profile,
            index,
        )?);
        if mask.is_some() {
            receipts.push(guard_receipts[index].clone());
        }
        if let (Some(context), Some(reference)) = (mask, mask_receipts[index].as_ref()) {
            let coverage = super::mask::coverage(
                &decoded,
                level.width,
                level.height,
                context.samples,
                context.factor,
                context.cutoff,
                if index == 0 {
                    context.linear
                } else {
                    context.min_linear
                },
            )?;
            let floor = match profile {
                Profile::Fast => 0.02,
                Profile::Balanced => 0.01,
                _ => 0.005,
            };
            let residual = (coverage - reference.target).abs();
            let allowed = (reference.optimum_gap + floor).min(1.);
            receipts.push(json!({"mip":index,"mask_target":reference.target,"fixed_semantic_gap":reference.optimum_gap,"scale":reference.scale,"scans":reference.scans,"decoded_coverage":coverage,"residual":residual,"allowed":allowed}));
            if residual > allowed {
                return Err(fidelity(format!(
                    "MASK mip{index} residual{residual} allowed{allowed}"
                )));
            }
        }
        decoded_chain.push(mips::Level {
            width: level.width,
            height: level.height,
            rgba: decoded,
        });
    }
    let sampler_receipt = validate(&decoded_chain)?;
    if !sampler_receipt.is_null() {
        receipts.push(sampler_receipt);
    }
    receipts.push(json!({"charged_bytes":charge,"native_workspace_provisional":true,"scalar_rrr":scalar,"effort":if scalar{0}else{effort},"caller_levels":chain.levels.len()}));
    Ok(Delivery {
        bytes,
        mime: "image/ktx2",
        mip_count: chain.levels.len() as u32,
        receipts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_preserves_nonunit_normals_transparent_rgb_and_alpha() {
        let rgba = [
            255, 128, 128, 0, 128, 128, 200, 203, 170, 190, 220, 204, 1, 2, 3, 255,
        ];
        let out = encode(
            [4, 1],
            &rgba,
            &[1; 4],
            TextureSlot::Normal,
            Profile::Lossless,
            128 * 1024 * 1024,
            None,
        )
        .unwrap();
        assert_eq!(out.mime, "image/png");
        assert_eq!(out.mip_count, 1);
        let decoded = image::load_from_memory_with_format(&out.bytes, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        assert_eq!(decoded.as_raw(), &rgba);
        for channel in 0..4 {
            let mut mutant = decoded.as_raw().clone();
            mutant[channel] ^= 1;
            assert_ne!(mutant.as_slice(), &rgba);
        }
        assert_eq!(
            encode(
                [4, 1],
                &rgba,
                &[1; 4],
                TextureSlot::Normal,
                Profile::Lossless,
                1,
                None
            )
            .unwrap_err()
            .kind(),
            JobErrorKind::ResourceLimit
        );
    }
    #[test]
    fn all_profiles_and_slots_complete_independent_decode() {
        for profile in [
            Profile::Fast,
            Profile::Balanced,
            Profile::Quality,
            Profile::Lossless,
        ] {
            for slot in [
                TextureSlot::BaseColor,
                TextureSlot::Emissive,
                TextureSlot::MetallicRoughness,
                TextureSlot::Occlusion,
                TextureSlot::Normal,
            ] {
                let pixel = if slot == TextureSlot::Normal {
                    [128, 128, 255, 255]
                } else {
                    [90, 90, 90, 255]
                };
                let rgba = pixel.repeat(64);
                let support = vec![1; 64];
                let out = encode(
                    [8, 8],
                    &rgba,
                    &support,
                    slot,
                    profile,
                    128 * 1024 * 1024,
                    None,
                )
                .unwrap();
                if profile == Profile::Lossless {
                    assert_eq!(out.mip_count, 1);
                    continue;
                }
                // KTX2 DFD starts at the little-endian header offset 48.
                // The transfer byte is format metadata, independent of pixels.
                let dfd = u32::from_le_bytes(out.bytes[48..52].try_into().unwrap()) as usize;
                let transfer = out.bytes[dfd + 14];
                assert_eq!(
                    transfer,
                    if matches!(slot, TextureSlot::BaseColor | TextureSlot::Emissive) {
                        2
                    } else {
                        1
                    },
                    "wrong KTX transfer for {slot:?}/{profile:?}"
                );
                let independent = basisu::Transcoder::new(&out.bytes).unwrap();
                assert_eq!(independent.level_count(), 4);
                assert_eq!(out.mip_count, 4);
                let (mut w, mut h) = (8, 8);
                for mip in 0..4 {
                    let decoded = independent
                        .transcode(mip, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
                        .unwrap();
                    // Constant authored signal: independent expected values, no
                    // production mip/filter values used as expected truth.
                    let expected = pixel.repeat(w * h);
                    check_level(
                        &expected,
                        &decoded,
                        &vec![1; w * h],
                        slot,
                        profile,
                        mip as usize,
                    )
                    .unwrap();
                    w = (w / 2).max(1);
                    h = (h / 2).max(1);
                }
            }
        }
    }
    // Independent centre LINEAR coverage for this constant-image fixture. The
    // source target/gap are literal mathematical fixture facts, not mask output.
    fn centre_coverage(
        pixels: &[u8],
        w: usize,
        h: usize,
        factor: f64,
        cutoff: f64,
        weights: [f64; 2],
    ) -> f64 {
        let x = w as f64 / 2. - 0.5;
        let y = h as f64 / 2. - 0.5;
        let fetch = |x: i64, y: i64| {
            f64::from(
                pixels[((y.clamp(0, h as i64 - 1) as usize) * w
                    + x.clamp(0, w as i64 - 1) as usize)
                    * 4
                    + 3],
            ) / 255.
        };
        let (ix, iy) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let alpha = (fetch(ix, iy) * (1. - fx) + fetch(ix + 1, iy) * fx) * (1. - fy)
            + (fetch(ix, iy + 1) * (1. - fx) + fetch(ix + 1, iy + 1) * fx) * fy;
        let total = weights.iter().sum::<f64>();
        if total == 0. {
            return 0.;
        }
        [1., 0.5]
            .iter()
            .zip(weights)
            .filter(|(c0, _)| alpha * factor * **c0 >= cutoff)
            .map(|(_, weight)| weight)
            .sum::<f64>()
            / total
    }
    #[test]
    fn mask_variable_colour_source_target_every_profile_and_mip() {
        use super::super::mask::SurfaceSample;
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            for (cutoff, weights) in [(0.5, [3., 1.]), (1.5, [3., 1.]), (0.5, [0., 0.])] {
                let factor = 0.75;
                let surface = [
                    SurfaceSample {
                        uv: [0.5, 0.5],
                        alpha: 1.,
                        weight: weights[0],
                        source_pass: cutoff < 1.,
                    },
                    SurfaceSample {
                        uv: [0.5, 0.5],
                        alpha: 0.5,
                        weight: weights[1],
                        source_pass: false,
                    },
                ];
                let target = if cutoff > 1. || weights == [0., 0.] {
                    0.
                } else {
                    0.75
                };
                // Alpha255: context1 passes .75>=.5; context.5 fails
                // .375<.5. This fixed semantic outcome is attainable at all
                // levels including1x1; independently established gap is zero.
                let rgba = [90, 90, 90, 255].repeat(64);
                let out = encode(
                    [8, 8],
                    &rgba,
                    &[1; 64],
                    TextureSlot::BaseColor,
                    profile,
                    128 * 1024 * 1024,
                    Some(MaskDelivery {
                        samples: &surface,
                        factor,
                        cutoff,
                        linear: true,
                        min_linear: true,
                    }),
                )
                .unwrap();
                let independent = basisu::Transcoder::new(&out.bytes).unwrap();
                assert_eq!(independent.level_count(), 4);
                let (mut w, mut h) = (8, 8);
                for mip in 0..4 {
                    let decoded = independent
                        .transcode(mip, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
                        .unwrap();
                    let actual = centre_coverage(&decoded, w, h, factor, cutoff, weights);
                    let floor = match profile {
                        Profile::Fast => 0.02,
                        Profile::Balanced => 0.01,
                        _ => 0.005,
                    };
                    println!("delivery-mask-independent profile={profile:?} mip={mip} target={target} fixed_gap=0 coverage={actual} floor={floor}");
                    assert!((actual - target).abs() <= floor);
                    check_level(
                        &[90, 90, 90, 255].repeat(w * h),
                        &decoded,
                        &vec![1; w * h],
                        TextureSlot::BaseColor,
                        profile,
                        mip as usize,
                    )
                    .unwrap();
                    w = (w / 2).max(1);
                    h = (h / 2).max(1);
                }
            }
        }
    }
    #[test]
    fn near_cutoff_safe_guard_positive_and_original_source_truth() {
        use super::super::mask::SurfaceSample;
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            for alpha in [203, 204] {
                let surface = [
                    SurfaceSample {
                        uv: [0.5, 0.5],
                        alpha: 1.,
                        weight: 1.,
                        source_pass: alpha >= 204,
                    },
                    SurfaceSample {
                        uv: [0.5, 0.5],
                        alpha: 0.5,
                        weight: 1.,
                        source_pass: false,
                    },
                ];
                let target = if alpha >= 204 { 0.5 } else { 0. };
                let rgba = [90, 90, 90, alpha].repeat(64);
                match encode(
                    [8, 8],
                    &rgba,
                    &[1; 64],
                    TextureSlot::BaseColor,
                    profile,
                    128 * 1024 * 1024,
                    Some(MaskDelivery {
                        samples: &surface,
                        factor: 1.,
                        cutoff: 0.8,
                        linear: true,
                        min_linear: true,
                    }),
                ) {
                    Err(error) => {
                        assert_ne!(
                            alpha, 204,
                            "safe 1/.5 guard must produce accepted fixture: {error:?}"
                        );
                        assert_eq!(error.kind(), JobErrorKind::Unsupported);
                        println!("delivery-mask-nearcutoff typed-refusal profile={profile:?} source_alpha={alpha} source_target={target} error={error:?}");
                    }
                    Ok(out) => {
                        if alpha == 204 {
                            assert_eq!(
                                out.receipts
                                    .iter()
                                    .filter(|r| r.get("finite_sample_guard_enabled")
                                        == Some(&Value::Bool(true)))
                                    .count(),
                                4
                            );
                        }
                        let independent = basisu::Transcoder::new(&out.bytes).unwrap();
                        let (mut w, mut h) = (8, 8);
                        for mip in 0..4 {
                            let decoded = independent
                                .transcode(
                                    mip,
                                    basisu::TargetFormat::Rgba32,
                                    basisu::DecodeFlags::NONE,
                                )
                                .unwrap();
                            let actual = centre_coverage(&decoded, w, h, 1., 0.8, [1., 1.]);
                            let floor = match profile {
                                Profile::Fast => 0.02,
                                Profile::Balanced => 0.01,
                                _ => 0.005,
                            };
                            assert!((actual - target).abs() <= floor);
                            println!("delivery-mask-nearcutoff decoded profile={profile:?} mip={mip} source_alpha={alpha} source_target={target} fixed_gap=0 coverage={actual}");
                            w = (w / 2).max(1);
                            h = (h / 2).max(1);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn normal_rgb_gate_detects_radial_error_below_fast_length_cap() {
        // Radial Z perturbation passes fast meanlength.08/max.20 and
        // angular3/12, but stored Z PSNR<30. RGB floor adds protection.
        let expected = [128, 128, 255, 255];
        let mutant = [128, 128, 246, 255];
        let error = check_level(
            &expected,
            &mutant,
            &[1],
            TextureSlot::Normal,
            Profile::Fast,
            0,
        )
        .unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
    }
    #[test]
    fn coupled_colour_guard_disabled_native_outcome_honest() {
        use super::super::mask::SurfaceSample;
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            let samples = [
                SurfaceSample {
                    uv: [0.5, 0.5],
                    alpha: 1.,
                    weight: 1.,
                    source_pass: true,
                },
                SurfaceSample {
                    uv: [0.5, 0.5],
                    alpha: 0.999,
                    weight: 1.,
                    source_pass: false,
                },
            ];
            // Original204/255 passes1 but fails.999 at cutoff.8; candidate205
            // passes both. Independent pre-code classification counterexample.
            let rgba = [90, 90, 90, 204].repeat(64);
            match encode(
                [8, 8],
                &rgba,
                &[1; 64],
                TextureSlot::BaseColor,
                profile,
                128 * 1024 * 1024,
                Some(MaskDelivery {
                    samples: &samples,
                    factor: 1.,
                    cutoff: 0.8,
                    linear: true,
                    min_linear: true,
                }),
            ) {
                Err(error) => {
                    assert_eq!(error.kind(), JobErrorKind::Unsupported);
                    println!("unsafe-coupled-guard typedrefusal {profile:?} {error:?}");
                }
                Ok(out) => {
                    let guards: Vec<_> = out
                        .receipts
                        .iter()
                        .filter(|r| r.get("finite_sample_guard_enabled").is_some())
                        .collect();
                    assert_eq!(guards.len(), 4);
                    for receipt in guards {
                        assert_eq!(receipt["finite_sample_guard_enabled"], false);
                        assert_eq!(receipt["classifications_preserved"], false);
                    }
                    let independent = basisu::Transcoder::new(&out.bytes).unwrap();
                    let (mut w, mut h) = (8, 8);
                    for mip in 0..4 {
                        let decoded = independent
                            .transcode(mip, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
                            .unwrap();
                        let first = centre_coverage(&decoded, w, h, 1., 0.8, [1., 0.]);
                        let second = centre_coverage(&decoded, w, h, 0.999, 0.8, [1., 0.]);
                        let actual = (first + second) / 2.;
                        let floor = match profile {
                            Profile::Fast => 0.02,
                            Profile::Balanced => 0.01,
                            _ => 0.005,
                        };
                        assert!((actual - 0.5).abs() <= floor);
                        println!("unsafe-coupled-guard honestnative {profile:?} mip{mip} target.5 gap0 actual{actual}");
                        w = (w / 2).max(1);
                        h = (h / 2).max(1);
                    }
                }
            }
        }
    }
}
