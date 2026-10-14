//! Explicit profile delivery. PNG is raw level0; KTX owns a complete class chain.
//! ID pages ([`Class::Id`]) are PNG-only in every profile: one exact PNG per
//! authored level ([`encode_id_chain`]); Basis ETC1S/UASTC are refused.
use super::{
    mips::{self, Class},
    sparse_mips,
};
use crate::stage_timing::{self as timing, Counter, Stage};
use crate::{
    basisu_ktx2 as codec, gltf_source::source::TextureSlot, mesh_large::config::Profile, JobError,
    JobErrorKind,
};
use image::ImageEncoder;
use serde_json::{json, Value};

#[derive(Debug)]
pub(in crate::mesh_large) struct Delivery {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub mip_count: u32,
    pub receipts: Vec<Value>,
}
#[derive(Clone, Copy)]
pub(in crate::mesh_large) struct MaskDelivery<'a> {
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
/// Support-weighted error statistics of one channel of one decoded level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ChannelStats {
    /// Weighted PSNR in dB; infinite when exact.
    pub psnr: f64,
    /// Weighted 99.9th-percentile absolute error.
    pub p999: u8,
    pub max: u8,
    /// Largest RMSE of any aligned 4x4 codec block (supported texels only).
    pub block_rmse: f64,
    /// Texels with positive support.
    pub texels: u64,
}
pub(super) fn channel_stats(
    expected: &[u8],
    decoded: &[u8],
    support: &[u32],
    width: usize,
    channel: usize,
) -> ChannelStats {
    let mut histogram = [0f64; 256];
    let mut sse = 0.;
    let mut count = 0.;
    let mut max = 0;
    let mut texels = 0u64;
    let height = support.len() / width.max(1);
    let mut block_rmse = 0f64;
    for by in (0..height).step_by(4) {
        for bx in (0..width).step_by(4) {
            let (mut block_sse, mut block_weight) = (0., 0.);
            for y in by..(by + 4).min(height) {
                for x in bx..(bx + 4).min(width) {
                    let i = y * width + x;
                    let weight = support[i];
                    if weight == 0 {
                        continue;
                    }
                    let e = expected[i * 4 + channel].abs_diff(decoded[i * 4 + channel]);
                    let w = f64::from(weight);
                    let squared = f64::from(e).powi(2) * w;
                    histogram[usize::from(e)] += w;
                    sse += squared;
                    count += w;
                    block_sse += squared;
                    block_weight += w;
                    max = max.max(e);
                    texels += 1;
                }
            }
            if block_weight > 0. {
                block_rmse = block_rmse.max((block_sse / block_weight).sqrt());
            }
        }
    }
    let mut p999 = 0;
    let mut cumulative = 0.;
    for (e, w) in histogram.iter().enumerate() {
        cumulative += w;
        if cumulative >= count * 0.999 {
            p999 = e as u8;
            break;
        }
    }
    ChannelStats {
        psnr: if sse == 0. {
            f64::INFINITY
        } else {
            10. * (65025. * count / sse).log10()
        },
        p999,
        max,
        block_rmse,
        texels,
    }
}

/// Per-channel decoded-image floors of one slot class and profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ChannelGate {
    pub psnr: f64,
    /// 99.9th-percentile cap (calibrated colour channels of large levels).
    pub p999: Option<u8>,
    /// Worst 4x4-block RMSE cap (colour channels).
    pub block_rmse: Option<f64>,
    /// Maximum absolute error cap (alpha, data and normal channels).
    pub max: Option<u8>,
}
impl ChannelGate {
    pub(super) fn accepts(&self, stats: &ChannelStats) -> bool {
        stats.psnr >= self.psnr
            && self.p999.is_none_or(|cap| stats.p999 <= cap)
            && self.block_rmse.is_none_or(|cap| stats.block_rmse <= cap)
            && self.max.is_none_or(|cap| stats.max <= cap)
    }
}
/// Below this many supported texels (a 32x32 level) a 99.9th percentile is
/// the maximum of a few hundred samples, and mips this small are sampled only
/// at extreme minification. Small colour levels of packed island pages mix
/// neighbouring islands inside codec blocks; they keep the block corruption
/// guard and a PSNR floor 4 dB lower, without a percentile cap.
pub(super) const SMALL_LEVEL_TEXELS: u64 = 1024;
/// Colour channels: PSNR bounds mean quality, the 99.9th percentile bounds
/// the error tail without being decided by one outlier texel, and the worst
/// 4x4 codec-block RMSE catches one localized corrupted block that neither
/// can see on a large page. They are calibrated on every level of the real
/// 10M-triangle photogrammetry mesh (bench/architecture_audit/mesh_large_h8/
/// throughput.md). Alpha, data and normal channels have no real-data
/// calibration yet and keep their previous PSNR floors and maximum caps.
pub(super) fn channel_gate(
    slot: TextureSlot,
    profile: Profile,
    channel: usize,
    texels: u64,
) -> ChannelGate {
    let quality = profile == Profile::Quality;
    let fast = profile == Profile::Fast;
    let legacy = |psnr: f64, max: u8| ChannelGate {
        psnr,
        p999: None,
        block_rmse: None,
        max: Some(max),
    };
    if channel == 3 {
        return legacy(
            if quality { 34. } else { 30. },
            if fast {
                80
            } else if quality {
                32
            } else {
                64
            },
        );
    }
    match class(slot) {
        Class::Colour => {
            // Worst levels of >=1024 texels on the real mesh: fast and balanced
            // (UASTC pack level 0) PSNR 27.2 dB, p99.9 67, block RMSE 63.4;
            // quality (pack level 1, smaller leaves) PSNR 30.3 dB, p99.9 45,
            // block RMSE 26.6. Floors keep about 3 dB and 13 levels of margin.
            let psnr = if quality { 27. } else { 24. };
            let small = texels < SMALL_LEVEL_TEXELS;
            ChannelGate {
                psnr: if small { psnr - 4. } else { psnr },
                p999: (!small).then_some(if quality { 64 } else { 80 }),
                block_rmse: Some(if quality { 48. } else { 80. }),
                max: None,
            }
        }
        Class::Normal => legacy(30., 112),
        Class::Data if slot == TextureSlot::Occlusion => legacy(30., if quality { 8 } else { 80 }),
        Class::Data => legacy(30., 32),
        // Defensive: IDs admit no error at all.
        Class::Id => ChannelGate {
            psnr: f64::INFINITY,
            p999: None,
            block_rmse: None,
            max: Some(0),
        },
    }
}
pub(in crate::mesh_large) fn check_level(
    expected: &[u8],
    decoded: &[u8],
    support: &[u32],
    width: u32,
    slot: TextureSlot,
    profile: Profile,
    mip: usize,
) -> Result<Value, JobError> {
    if expected.len() != decoded.len()
        || expected.len() != support.len() * 4
        || width == 0
        || !support.len().is_multiple_of(width as usize)
    {
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
        let stats = channel_stats(expected, decoded, support, width as usize, channel);
        let gate = channel_gate(slot, profile, channel, stats.texels);
        metrics.push(json!({"channel":channel,"psnr":if stats.psnr.is_finite(){Some(stats.psnr)}else{None},"exact":stats.psnr.is_infinite(),"p999":stats.p999,"max":stats.max,"blockRmse":stats.block_rmse,"texels":stats.texels,"floor":gate.psnr,"p999Cap":gate.p999,"blockRmseCap":gate.block_rmse,"maxCap":gate.max}));
        if !gate.accepts(&stats) {
            return Err(fidelity(format!(
                "texture {slot:?} {profile:?} mip{mip} channel{channel} PSNR{} p99.9 {} blockRMSE{} max{} over {} texels (gate {gate:?})",
                stats.psnr, stats.p999, stats.block_rmse, stats.max, stats.texels
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

/// Whether a slot/profile stores occlusion as scalar ETC1S (R replicated).
/// Quality is excluded on purpose: its occlusion gate (p99.9 <= 8, against 80
/// elsewhere) is far tighter than ETC1S can meet, so Quality keeps UASTC.
pub(super) fn scalar_occlusion(slot: TextureSlot, profile: Profile) -> bool {
    slot == TextureSlot::Occlusion && profile != Profile::Quality
}
/// Encoder mode and parameters of one slot and lossy profile.
pub(super) fn codec_settings(slot: TextureSlot, profile: Profile) -> (codec::Mode, codec::Params) {
    let scalar = scalar_occlusion(slot, profile);
    // Quality's effort of 2 is deliberately not pinned by a test: lowering it
    // only changes encoder time and size, while every output is still held to
    // the per-profile decoded-image floors in `check_level`, which are the
    // contract. Mutating 2 to 0 therefore survives on purpose.
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
    (
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
    )
}

pub(in crate::mesh_large) fn workspace_charge(
    dimensions: [u32; 2],
    profile: Profile,
) -> Result<u64, JobError> {
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
        timing::time(Stage::PngEncode, || {
            image::codecs::png::PngEncoder::new(&mut bytes).write_image(
                rgba,
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
        })
        .map_err(|e| error(JobErrorKind::InvalidState, format!("PNG encode {e}")))?;
        let decoded = timing::time(Stage::PngVerify, || {
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        })
        .map_err(|e| error(JobErrorKind::InvalidState, format!("PNG verify {e}")))?
        .into_rgba8();
        timing::count(Counter::EncodedChainTexels, base);
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
    let chain = timing::time(Stage::SparseMips, || {
        sparse_mips::build(width, height, rgba, support, class(slot), memory_limit)
    })?;
    encode_authored(&chain, slot, profile, memory_limit, mask, |_| {
        Ok(Value::Null)
    })
}

/// The atlas owner supplies the complete chain and the independent source-sampler check.
pub(in crate::mesh_large) fn encode_authored(
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
    if chain.class == Class::Id {
        // Basis block codecs (ETC1S, UASTC) are lossy and blend IDs.
        return Err(error(
            JobErrorKind::InvalidRequest,
            "ID texture pages are lossless PNG only; Basis KTX2 refused",
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
    let scalar = scalar_occlusion(slot, profile);
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
    let (mode, params) = codec_settings(slot, profile);
    let effort = params.effort;
    let (bytes, mut decoded_levels) = codec::encode_ktx2_mips_decoded(&levels, mode, params)?;
    let mut receipts = Vec::new();
    receipts
        .try_reserve_exact(chain.levels.len() * 3 + 1)
        .map_err(|_| resource())?;
    let mut decoded_chain = Vec::new();
    decoded_chain
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    for (index, level) in chain.levels.iter().enumerate() {
        timing::count(
            Counter::EncodedChainTexels,
            u64::from(level.width) * u64::from(level.height),
        );
        // The codec's mandatory verification decoded every level with the
        // vendored transcoder; reuse those bytes rather than decode again.
        let decoded = std::mem::take(
            decoded_levels
                .get_mut(index)
                .ok_or_else(|| error(JobErrorKind::InvalidState, "missing decoded level"))?,
        );
        if decoded.len() != level.rgba.len() {
            return Err(error(JobErrorKind::InvalidState, "decoded level shape"));
        }
        receipts.push(timing::time(Stage::FidelityCheck, || {
            check_level(
                &expected[index],
                &decoded,
                &level.support,
                level.width,
                slot,
                profile,
                index,
            )
        })?);
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
    let sampler_receipt = timing::time(Stage::SamplerCheckDecoded, || validate(&decoded_chain))?;
    if !sampler_receipt.is_null() {
        receipts.push(sampler_receipt);
    }
    receipts.push(json!({"charged_bytes":charge,"native_workspace_provisional":true,"scalar_rrr":scalar,"effort":effort,"caller_levels":chain.levels.len()}));
    Ok(Delivery {
        bytes,
        mime: "image/ktx2",
        mip_count: chain.levels.len() as u32,
        receipts,
    })
}

/// One exact PNG per authored ID level. Level 0 is the delivered texture
/// (sampled NEAREST, no runtime mips); coarser levels are ID-safe pages for
/// coarse proxies. Every level is decoded independently and must equal the
/// authored bytes; `validate` then sees each decoded level.
#[derive(Debug)]
pub(super) struct IdDelivery {
    /// PNG bytes per level, level 0 first.
    pub levels: Vec<Vec<u8>>,
    pub receipts: Vec<Value>,
}
pub(super) fn encode_id_chain(
    chain: &sparse_mips::Chain,
    memory_limit: u64,
    mut validate: impl FnMut(usize, &mips::Level) -> Result<Value, JobError>,
) -> Result<IdDelivery, JobError> {
    if chain.class != Class::Id {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "ID delivery requires an ID-class chain",
        ));
    }
    let base = chain.levels.first().ok_or_else(resource)?;
    let charge = workspace_charge([base.width, base.height], Profile::Lossless)?;
    if charge > memory_limit {
        return Err(resource());
    }
    let mut levels = Vec::new();
    levels
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    let mut receipts = Vec::new();
    receipts
        .try_reserve_exact(chain.levels.len() + 1)
        .map_err(|_| resource())?;
    for (index, level) in chain.levels.iter().enumerate() {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(level.rgba.len().checked_mul(2).ok_or_else(resource)?)
            .map_err(|_| resource())?;
        timing::time(Stage::PngEncode, || {
            image::codecs::png::PngEncoder::new(&mut bytes).write_image(
                &level.rgba,
                level.width,
                level.height,
                image::ExtendedColorType::Rgba8,
            )
        })
        .map_err(|e| error(JobErrorKind::InvalidState, format!("ID PNG encode {e}")))?;
        let decoded = timing::time(Stage::PngVerify, || {
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        })
        .map_err(|e| error(JobErrorKind::InvalidState, format!("ID PNG verify {e}")))?
        .into_rgba8();
        timing::count(
            Counter::EncodedChainTexels,
            u64::from(level.width) * u64::from(level.height),
        );
        let decoded = mips::Level {
            width: decoded.width(),
            height: decoded.height(),
            rgba: decoded.into_raw(),
        };
        if decoded.width != level.width
            || decoded.height != level.height
            || decoded.rgba != level.rgba
        {
            return Err(error(
                JobErrorKind::InvalidState,
                format!("ID PNG level {index} changed authored IDs"),
            ));
        }
        receipts.push(validate(index, &decoded)?);
        levels.push(bytes);
    }
    receipts.push(json!({"profile":"id","encoding":"png","levels_exact":true,"charged_bytes":charge,"caller_levels":chain.levels.len()}));
    Ok(IdDelivery { levels, receipts })
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
                        w as u32,
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
    /// Occlusion is scalar ETC1S in fast/balanced and UASTC in quality; every
    /// other slot is UASTC. Read from the KTX2 DFD colour model of the output
    /// itself (basic block word 2, byte 0): 163 is ETC1S, 166 is UASTC.
    #[test]
    fn occlusion_is_scalar_etc1s_except_in_quality() {
        const ETC1S: u8 = 163;
        const UASTC: u8 = 166;
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            for slot in [TextureSlot::Occlusion, TextureSlot::MetallicRoughness] {
                let out = encode(
                    [8, 8],
                    &[90, 90, 90, 255].repeat(64),
                    &[1; 64],
                    slot,
                    profile,
                    128 * 1024 * 1024,
                    None,
                )
                .unwrap();
                let dfd = u32::from_le_bytes(out.bytes[48..52].try_into().unwrap()) as usize;
                let scalar = matches!(
                    (slot, profile),
                    (TextureSlot::Occlusion, Profile::Fast | Profile::Balanced)
                );
                assert_eq!(
                    out.bytes[dfd + 12],
                    if scalar { ETC1S } else { UASTC },
                    "{slot:?}/{profile:?} codec"
                );
                assert_eq!(
                    out.receipts.last().unwrap()["scalar_rrr"],
                    scalar,
                    "{slot:?}/{profile:?} receipt"
                );
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
                        w as u32,
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
    /// Deterministic photograph-like page: smooth value noise plus texel
    /// detail and a few hard edges, opaque.
    fn photo_page(edge: usize) -> Vec<u8> {
        let hash = |x: usize, y: usize, s: usize| {
            let mut h = (x as u64).wrapping_mul(0x9E37_79B9)
                ^ (y as u64).wrapping_mul(0x85EB_CA6B)
                ^ s as u64;
            h ^= h >> 13;
            h = h.wrapping_mul(0xC2B2_AE35);
            (h >> 7) % 256
        };
        let smooth = |x: usize, y: usize, s: usize| {
            let (cx, cy) = (x / 16, y / 16);
            let (fx, fy) = ((x % 16) as f64 / 16., (y % 16) as f64 / 16.);
            let v = |i, j| hash(cx + i, cy + j, s) as f64;
            (v(0, 0) * (1. - fx) + v(1, 0) * fx) * (1. - fy)
                + (v(0, 1) * (1. - fx) + v(1, 1) * fx) * fy
        };
        let mut out = Vec::with_capacity(edge * edge * 4);
        for y in 0..edge {
            for x in 0..edge {
                let edge_shift = if (x / 37 + y / 53) % 3 == 0 { 40. } else { 0. };
                for c in 0..3 {
                    let v = smooth(x, y, c) * 0.7
                        + (hash(x, y, c + 7) as f64 - 128.) * 0.12
                        + edge_shift;
                    out.push(v.round().clamp(0., 255.) as u8);
                }
                out.push(255);
            }
        }
        out
    }
    fn decoded_levels(out: &Delivery, edge: usize) -> Vec<Vec<u8>> {
        let independent = basisu::Transcoder::new(&out.bytes).unwrap();
        (0..independent.level_count())
            .map(|mip| {
                let decoded = independent
                    .transcode(mip, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
                    .unwrap();
                assert_eq!(decoded.len(), (edge >> mip).max(1).pow(2) * 4);
                decoded
            })
            .collect()
    }
    #[test]
    fn robust_gate_accepts_codec_outliers_and_refuses_corruption_controls() {
        let edge = 128;
        let rgba = photo_page(edge);
        let support = vec![1; edge * edge];
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            let out = encode(
                [edge as u32, edge as u32],
                &rgba,
                &support,
                TextureSlot::BaseColor,
                profile,
                128 * 1024 * 1024,
                None,
            )
            .unwrap();
            let decoded = decoded_levels(&out, edge);
            let check = |level: &[u8]| {
                check_level(
                    &rgba,
                    level,
                    &support,
                    edge as u32,
                    TextureSlot::BaseColor,
                    profile,
                    0,
                )
            };
            assert!(check(&decoded[0]).is_ok());
            // One hot texel at full-range error: the old maximum cap refused
            // this; the percentile, PSNR and block guard accept it.
            let mut outlier = decoded[0].clone();
            outlier[(40 * edge + 40) * 4] ^= 0xff;
            assert!(check(&outlier).is_ok(), "{profile:?} single outlier");
            // A corrupted 4x4 codec block is refused by the block guard alone.
            let mut corrupt = decoded[0].clone();
            for y in 64..68 {
                for x in 64..68 {
                    for c in 0..3 {
                        corrupt[(y * edge + x) * 4 + c] ^= 0x80;
                    }
                }
            }
            let stats = channel_stats(&rgba, &corrupt, &support, edge, 0);
            let gate = channel_gate(TextureSlot::BaseColor, profile, 0, stats.texels);
            assert!(stats.psnr >= gate.psnr && stats.p999 <= gate.p999.unwrap());
            assert!(check(&corrupt).is_err(), "{profile:?} corrupted block");
            // Swapped red/blue channels are refused.
            let mut swapped = decoded[0].clone();
            for px in swapped.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            assert!(check(&swapped).is_err(), "{profile:?} swapped channels");
            // Ten percent of texels uniformly damaged: the percentile refuses
            // even when the damage stays below the block guard.
            let mut damaged = decoded[0].clone();
            for (i, px) in damaged.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                if i % 10 == 0 {
                    px[1] = px[1].wrapping_add(96);
                }
            }
            assert!(check(&damaged).is_err(), "{profile:?} spread damage");
        }
    }
    #[test]
    fn shifted_levels_and_flipped_normal_green_are_refused_every_profile() {
        let edge = 64usize;
        let support = vec![1; edge * edge];
        // One-texel saturated checker: a level shifted by one (each texel
        // replaced by the next coarser level) loses all its contrast.
        let checker: Vec<u8> = (0..edge * edge)
            .flat_map(|i| {
                if (i % edge + i / edge).is_multiple_of(2) {
                    [0, 255, 0, 255]
                } else {
                    [255, 0, 255, 255]
                }
            })
            .collect();
        // Smooth tangent-space bumps, unit length.
        let normals: Vec<u8> = (0..edge * edge)
            .flat_map(|i| {
                let (x, y) = ((i % edge) as f64, (i / edge) as f64);
                let n = [0.25 * (x / 40.).sin(), 0.25 * (y / 48.).cos(), 1.];
                let l = n.iter().map(|v| v * v).sum::<f64>().sqrt();
                let q = |v: f64| ((v / l + 1.) * 127.5).round() as u8;
                [q(n[0]), q(n[1]), q(n[2]), 255]
            })
            .collect();
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            let out = encode(
                [edge as u32, edge as u32],
                &checker,
                &support,
                TextureSlot::BaseColor,
                profile,
                128 * 1024 * 1024,
                None,
            )
            .unwrap();
            let decoded = decoded_levels(&out, edge);
            let half = edge / 2;
            let shifted: Vec<u8> = (0..edge * edge)
                .flat_map(|i| {
                    let (x, y) = (i % edge / 2, i / edge / 2);
                    decoded[1][(y * half + x) * 4..(y * half + x) * 4 + 4].to_vec()
                })
                .collect();
            let check = |slot, expected: &[u8], level: &[u8]| {
                check_level(expected, level, &support, edge as u32, slot, profile, 0)
            };
            assert!(check(TextureSlot::BaseColor, &checker, &decoded[0]).is_ok());
            assert!(check(TextureSlot::BaseColor, &checker, &shifted).is_err());
            let out = encode(
                [edge as u32, edge as u32],
                &normals,
                &support,
                TextureSlot::Normal,
                profile,
                128 * 1024 * 1024,
                None,
            )
            .unwrap();
            let mut flipped = decoded_levels(&out, edge).swap_remove(0);
            for px in flipped.as_chunks_mut::<4>().0 {
                px[1] = 255 - px[1];
            }
            assert!(
                check(TextureSlot::Normal, &normals, &flipped).is_err(),
                "{profile:?} flipped green"
            );
        }
    }
    #[test]
    fn small_level_relaxation_is_colour_only_and_keeps_the_block_guard() {
        // 16x16 = 256 supported texels: a small level.
        let expected = [100u8, 120, 140, 255].repeat(256);
        let mut offset = expected.clone();
        for px in offset.as_chunks_mut::<4>().0 {
            px[2] += 17;
        }
        // Uniform 17-level error: PSNR 23.5 dB, above the relaxed colour floor
        // (24 - 4) but below every full-size floor.
        assert!(check_level(
            &expected,
            &offset,
            &[1; 256],
            16,
            TextureSlot::BaseColor,
            Profile::Balanced,
            4
        )
        .is_ok());
        let big = [100u8, 120, 140, 255].repeat(1024);
        let mut big_offset = big.clone();
        for px in big_offset.as_chunks_mut::<4>().0 {
            px[2] += 17;
        }
        assert!(check_level(
            &big,
            &big_offset,
            &[1; 1024],
            32,
            TextureSlot::BaseColor,
            Profile::Balanced,
            3
        )
        .is_err());
        let normal = [128u8, 128, 255, 255].repeat(256);
        let mut normal_offset = normal.clone();
        for px in normal_offset.as_chunks_mut::<4>().0 {
            px[2] -= 17;
        }
        assert!(check_level(
            &normal,
            &normal_offset,
            &[1; 256],
            16,
            TextureSlot::Normal,
            Profile::Fast,
            4
        )
        .is_err());
        // Garbage on a small colour level is still refused.
        let garbage: Vec<u8> = expected
            .iter()
            .enumerate()
            .map(|(i, v)| if i % 4 == 3 { *v } else { v ^ 0x80 })
            .collect();
        assert!(check_level(
            &expected,
            &garbage,
            &[1; 256],
            16,
            TextureSlot::BaseColor,
            Profile::Balanced,
            4
        )
        .is_err());
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
            1,
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
